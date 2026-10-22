//! Reading screen text over a fake stream and a scripted engine: resolution,
//! budgets, failures, merges and identities.

use std::{
    future::{Future, ready},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use vsift_domain::{
    DEFAULT_READ_PERIOD_MICROS, FrameDimensions, FrameListing, ListedFrame, ListingTail, MediaTime,
    ProviderLabel, ReadTarget, ScreenTextCoverage, ScreenTextFailure, ScreenTextIndex,
    ScreenTextOutcome, ScreenTextProfile, ScreenTextProvider, SessionId, Sha256Hex, SourceId,
    TimeBase, TimeRange, VisualCandidateId, plan_reads,
};

use super::{
    EngineLine, EngineReading, MAX_SCREEN_TEXT_FRAMES_PER_CALL, ReadScreenTextRequest,
    ScreenTextBuildError, ScreenTextExtension, ScreenTextImage, ScreenTextReadError,
    ScreenTextReader, ScreenTextStop, merge_screen_text, read_screen_text_within,
    verify_screen_text_identities,
};
use crate::{
    EvidenceControl, EvidenceMediaError, EvidenceStop, ExtractedFrame, FrameExtractor,
    VideoStreamFacts,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const SECOND: u64 = 1_000_000;
/// The fake stream: 10 frames a second for 60 seconds, frame `n` at `n / 10` s.
const FRAMES: i64 = 600;

fn frame_time(pts: i64) -> MediaTime {
    MediaTime::from_micros(u64::try_from(pts).unwrap_or(0) * SECOND / 10)
}

struct FakeVideo {
    facts: VideoStreamFacts,
    listings: AtomicUsize,
    extractions: AtomicUsize,
}

impl FakeVideo {
    fn new() -> Built<Self> {
        Ok(Self {
            facts: VideoStreamFacts {
                stream_index: 0,
                time_base: TimeBase::new(1, 10)?,
                displayed: FrameDimensions::new(640, 360)?,
                duration: MediaTime::from_micros(60 * SECOND),
            },
            listings: AtomicUsize::new(0),
            extractions: AtomicUsize::new(0),
        })
    }

    fn listing(&self, requested: TimeRange) -> Result<FrameListing, EvidenceMediaError> {
        self.listings.fetch_add(1, Ordering::SeqCst);
        let duration = self.facts.duration;
        let (end, tail) = if requested.end() >= duration {
            (duration, ListingTail::EndOfStream)
        } else {
            (requested.end(), ListingTail::MoreMayFollow)
        };
        let covered =
            TimeRange::new(requested.start(), end).map_err(|_| EvidenceMediaError::Invalid)?;
        let frames = (0..FRAMES)
            .map(|pts| ListedFrame {
                pts,
                time: frame_time(pts),
            })
            .filter(|frame| frame.time >= covered.start() && frame.time < covered.end())
            .collect();
        FrameListing::new(covered, frames, tail).map_err(|_| EvidenceMediaError::Invalid)
    }
}

impl FrameExtractor for FakeVideo {
    fn stream(&self) -> VideoStreamFacts {
        self.facts
    }

    fn max_frames_per_run(&self) -> usize {
        8
    }

    fn list_frames(
        &self,
        requested: TimeRange,
    ) -> impl Future<Output = Result<FrameListing, EvidenceMediaError>> + Send {
        ready(self.listing(requested))
    }

    fn frames(
        &self,
        pts: &[i64],
    ) -> impl Future<Output = Result<Vec<ExtractedFrame>, EvidenceMediaError>> + Send {
        self.extractions.fetch_add(1, Ordering::SeqCst);
        ready(Ok(pts
            .iter()
            .map(|pts| ExtractedFrame {
                pts: *pts,
                time: frame_time(*pts),
                png: pts.to_string().into_bytes(),
            })
            .collect()))
    }

    fn crop(
        &self,
        _pts: i64,
        _rect: vsift_domain::CropRect,
    ) -> impl Future<Output = Result<ExtractedFrame, EvidenceMediaError>> + Send {
        ready(Err(EvidenceMediaError::Invalid))
    }
}

/// An engine that reads "frame N" off frame N, and can be told to misbehave on one frame.
struct Scripted {
    runner: char,
    identities: AtomicUsize,
    reads: AtomicUsize,
    images: AtomicUsize,
    odd: Mutex<Option<(String, EngineReading)>>,
    fail_batch: Option<(usize, ScreenTextReadError)>,
}

impl Scripted {
    fn new(runner: char) -> Self {
        Self {
            runner,
            identities: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            images: AtomicUsize::new(0),
            odd: Mutex::new(None),
            fail_batch: None,
        }
    }

    fn provider(&self) -> Result<ScreenTextProvider, ScreenTextReadError> {
        let digest = |fill: char| {
            Sha256Hex::parse(fill.to_string().repeat(64))
                .map_err(|_| ScreenTextReadError::ProviderFailed)
        };
        Ok(ScreenTextProvider {
            runner_sha256: digest(self.runner)?,
            runner_version: ProviderLabel::parse("0.1.0")
                .map_err(|_| ScreenTextReadError::ProviderFailed)?,
            runtime_version: ProviderLabel::parse("1.31.0")
                .map_err(|_| ScreenTextReadError::ProviderFailed)?,
            detector_sha256: digest('d')?,
            recognizer_sha256: digest('e')?,
        })
    }

    fn reading(
        &self,
        build: &ScreenTextProvider,
        images: &[ScreenTextImage<'_>],
    ) -> Result<Vec<EngineReading>, ScreenTextReadError> {
        // As the runner adapter does: a batch for another build is refused.
        if *build != self.provider()? {
            return Err(ScreenTextReadError::BuildChanged);
        }
        let batch = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some((failing, error)) = self.fail_batch
            && failing == batch
        {
            return Err(error);
        }
        self.images.fetch_add(images.len(), Ordering::SeqCst);
        let odd = self
            .odd
            .lock()
            .map_err(|_| ScreenTextReadError::ProviderFailed)?
            .clone();
        Ok(images
            .iter()
            .map(|image| {
                let name = String::from_utf8_lossy(image.png).into_owned();
                match &odd {
                    Some((frame, reading)) if *frame == name => reading.clone(),
                    _ => EngineReading::Lines {
                        lines: vec![EngineLine {
                            x: 10,
                            y: 10,
                            width: 200,
                            height: 20,
                            text: format!("frame {name}"),
                            confidence_basis_points: 9_900,
                        }],
                        truncated: false,
                    },
                }
            })
            .collect())
    }
}

impl ScreenTextReader for Scripted {
    fn identity(
        &self,
    ) -> impl Future<Output = Result<ScreenTextProvider, ScreenTextReadError>> + Send {
        self.identities.fetch_add(1, Ordering::SeqCst);
        ready(self.provider())
    }

    fn max_images_per_run(&self) -> usize {
        4
    }

    fn read(
        &self,
        build: &ScreenTextProvider,
        images: &[ScreenTextImage<'_>],
    ) -> impl Future<Output = Result<Vec<EngineReading>, ScreenTextReadError>> + Send {
        ready(self.reading(build, images))
    }
}

struct Control {
    cancelled: AtomicBool,
}

impl EvidenceControl for Control {
    fn stop(&self) -> Option<EvidenceStop> {
        self.cancelled
            .load(Ordering::SeqCst)
            .then_some(EvidenceStop::Cancelled)
    }
}

fn running() -> Control {
    Control {
        cancelled: AtomicBool::new(false),
    }
}

fn ids() -> Built<(SessionId, SourceId)> {
    Ok((
        SessionId::parse("ses_0123456789abcdef")?,
        SourceId::parse(format!("src_sha256_{}", "a".repeat(64)))?,
    ))
}

fn whole_plan(candidates: &[(VisualCandidateId, MediaTime)]) -> Built<Vec<ReadTarget>> {
    let range = TimeRange::new(
        MediaTime::from_micros(0),
        MediaTime::from_micros(70 * SECOND),
    )?;
    Ok(plan_reads(range, candidates, DEFAULT_READ_PERIOD_MICROS)?)
}

async fn read(
    plan: &[ReadTarget],
    previous: Option<&ScreenTextIndex>,
    video: &FakeVideo,
    engine: &Scripted,
    max_frames: u16,
) -> Result<ScreenTextExtension, ScreenTextBuildError> {
    let (session, source) = ids().map_err(|_| ScreenTextBuildError::ScopeMismatch)?;
    read_screen_text_within(
        ReadScreenTextRequest {
            session_id: &session,
            source_id: &source,
            profile: ScreenTextProfile::PpOcrV5LatinV1,
            previous,
            plan,
            max_frames,
            revisions_left: 64,
        },
        video,
        engine,
        &running(),
    )
    .await
}

/// An index of `engine`'s build holding `frames` frames past the fake video's
/// end (so no target of a plan resolves to one of them), each with `lines` lines.
fn held_index(engine: &Scripted, frames: usize, lines: usize) -> Built<ScreenTextIndex> {
    let (session_id, source_id) = ids()?;
    let scope = vsift_domain::ScreenTextScope {
        session_id,
        source_id,
        stream_index: 0,
        profile: ScreenTextProfile::PpOcrV5LatinV1,
        provider: engine.provider()?,
    };
    let dimensions = FrameDimensions::new(640, 360)?;
    let mut held = Vec::with_capacity(frames);
    for ordinal in 0..frames {
        let pts = FRAMES + i64::try_from(ordinal)?;
        let mut read = Vec::with_capacity(lines);
        for line in 0..lines {
            read.push(vsift_domain::ScreenTextLine {
                id: super::screen_text_line_id(&scope, pts, line + 1)?,
                region: vsift_domain::TextRegion::new(0, 0, 10, 10, dimensions)?,
                text: vsift_domain::ScreenText::parse("held")?,
                confidence: vsift_domain::Confidence::provider_score(
                    9_000,
                    vsift_domain::ConfidenceOrigin::ProviderUncalibrated,
                )?,
            });
        }
        held.push(vsift_domain::ScreenTextFrame {
            frame: vsift_domain::FrameRef {
                stream_index: 0,
                time_base: TimeBase::new(1, 10)?,
                pts,
                time: frame_time(pts),
                dimensions,
            },
            targets: vec![ReadTarget {
                time: frame_time(pts),
                candidate: None,
            }],
            outcome: ScreenTextOutcome::Read {
                lines: read,
                truncated: false,
            },
        });
    }
    let revision = std::num::NonZeroU32::MIN;
    Ok(ScreenTextIndex::new(vsift_domain::ScreenTextIndexParts {
        id: super::screen_text_index_id(&scope, revision)?,
        revision,
        scope,
        period_micros: DEFAULT_READ_PERIOD_MICROS,
        frames: held,
        unresolved: Vec::new(),
    })?)
}

/// RV-2 finding 5: the budget is clipped to the room the index has left before
/// anything is read. An index two frames short of its bound reads two frames
/// and stops as `index_full`, and the commit then fits; a full one reads
/// nothing and runs no batch. Before, the call read its whole budget and the
/// merge failed, every time.
#[tokio::test]
async fn a_call_reads_only_what_the_index_has_room_for() -> TestResult {
    let plan = whole_plan(&[])?;
    let video = FakeVideo::new()?;
    let engine = Scripted::new('1');
    let nearly = held_index(&engine, vsift_domain::MAX_SCREEN_TEXT_FRAMES - 2, 0)?;
    assert_eq!(super::frames_with_room(Some(&nearly)), 2);
    let extension = read(&plan, Some(&nearly), &video, &engine, 40).await?;
    assert_eq!(extension.stop, Some(ScreenTextStop::IndexFull));
    assert_eq!(extension.frames.len(), 2);
    let merged = merge_screen_text(Some(&nearly), &extension, DEFAULT_READ_PERIOD_MICROS)?
        .ok_or("no revision")?;
    assert_eq!(merged.frames().len(), vsift_domain::MAX_SCREEN_TEXT_FRAMES);

    let reads = engine.reads.load(Ordering::SeqCst);
    let extension = read(&plan, Some(&merged), &video, &engine, 40).await?;
    assert_eq!(extension.stop, Some(ScreenTextStop::IndexFull));
    assert!(extension.frames.is_empty() && extension.unresolved.is_empty());
    assert_eq!(
        engine.reads.load(Ordering::SeqCst),
        reads,
        "a full index runs no batch"
    );
    assert_eq!(video.extractions.load(Ordering::SeqCst), 1);
    Ok(())
}

/// The line bound counts too: room is kept for the most lines a frame may hold.
#[test]
fn room_counts_lines_as_if_every_new_frame_were_full() -> TestResult {
    let engine = Scripted::new('1');
    // A new index: 200,000 lines hold 781 full frames, more than any call reads.
    assert_eq!(super::frames_with_room(None), 781);
    // 100 frames of 256 lines leave 174,400 lines: room for 681 full frames,
    // fewer than the 3,996 the frame bound alone would allow.
    let lined = held_index(&engine, 100, vsift_domain::MAX_SCREEN_TEXT_LINES_PER_FRAME)?;
    assert_eq!(super::frames_with_room(Some(&lined)), 681);
    Ok(())
}

/// A session that holds as many revisions as it may: the call stops before the
/// engine identifies itself or reads anything.
#[tokio::test]
async fn no_revision_left_runs_nothing() -> TestResult {
    let plan = whole_plan(&[])?;
    let video = FakeVideo::new()?;
    let engine = Scripted::new('1');
    let (session, source) = ids()?;
    let outcome = read_screen_text_within(
        ReadScreenTextRequest {
            session_id: &session,
            source_id: &source,
            profile: ScreenTextProfile::PpOcrV5LatinV1,
            previous: None,
            plan: &plan,
            max_frames: 40,
            revisions_left: 0,
        },
        &video,
        &engine,
        &running(),
    )
    .await;
    assert_eq!(outcome, Err(ScreenTextBuildError::IndexFull));
    assert_eq!(engine.identities.load(Ordering::SeqCst), 0);
    assert_eq!(video.listings.load(Ordering::SeqCst), 0);
    Ok(())
}

/// RV-2 finding 3: each batch is read with the build the engine identified
/// itself as, and a batch by another build stops the call like any engine
/// failure, keeping only the batches before it.
#[tokio::test]
async fn a_batch_by_another_build_is_refused_and_not_kept() -> TestResult {
    let plan = whole_plan(&[])?;
    let video = FakeVideo::new()?;
    let mut engine = Scripted::new('1');
    engine.fail_batch = Some((2, ScreenTextReadError::BuildChanged));
    let extension = read(&plan, None, &video, &engine, 40).await?;
    assert_eq!(
        extension.stop,
        Some(ScreenTextStop::Engine(ScreenTextReadError::BuildChanged))
    );
    assert_eq!(extension.frames.len(), 4, "only the first batch is kept");
    assert_eq!(engine.identities.load(Ordering::SeqCst), 1);
    Ok(())
}

fn candidate(suffix: char) -> Built<VisualCandidateId> {
    Ok(VisualCandidateId::parse(format!(
        "vcd_{}",
        suffix.to_string().repeat(32)
    ))?)
}

#[tokio::test]
async fn a_plan_is_read_in_bounded_calls_until_complete() -> TestResult {
    let plan = whole_plan(&[
        (candidate('a')?, MediaTime::from_micros(12_300_000)),
        (candidate('b')?, MediaTime::from_micros(30 * SECOND)),
    ])?;
    // Periodic 0, 10, ..., 60 s (60 s is past the end) and the two candidates.
    assert_eq!(plan.len(), 9);
    let video = FakeVideo::new()?;
    let engine = Scripted::new('1');
    let first = read(&plan, None, &video, &engine, 3).await?;
    assert_eq!(first.stop, Some(ScreenTextStop::FrameLimit));
    assert_eq!(first.frames.len(), 3);
    let first_index =
        merge_screen_text(None, &first, DEFAULT_READ_PERIOD_MICROS)?.ok_or("no first revision")?;
    verify_screen_text_identities(&first_index)?;
    assert_eq!(first_index.revision().get(), 1);
    let coverage = ScreenTextCoverage::of(&plan, Some(&first_index));
    assert_eq!((coverage.read, coverage.pending), (3, 6));

    let second = read(&plan, Some(&first_index), &video, &engine, 40).await?;
    assert_eq!(second.stop, None);
    let second_index = merge_screen_text(Some(&first_index), &second, DEFAULT_READ_PERIOD_MICROS)?
        .ok_or("no second revision")?;
    verify_screen_text_identities(&second_index)?;
    assert_eq!(second_index.revision().get(), 2);
    let coverage = ScreenTextCoverage::of(&plan, Some(&second_index));
    // The 30 s candidate and the 30 s period share one frame; 60 s has none.
    assert_eq!(
        coverage,
        ScreenTextCoverage {
            planned: 9,
            read: 8,
            failed: 0,
            unresolved: 1,
            pending: 0,
            awaiting_analysis: 0,
        }
    );
    assert!(coverage.is_complete());
    assert_eq!(second_index.frames().len(), 7);
    let shared = second_index.frame_at_pts(300).ok_or("no frame at 30 s")?;
    assert_eq!(shared.targets.len(), 2);
    let candidate_frame = second_index
        .frame_at_pts(123)
        .ok_or("the candidate's frame is exact")?;
    assert_eq!(
        candidate_frame
            .lines()
            .first()
            .map(|line| line.text.as_str()),
        Some("frame 123")
    );
    // Nothing is read twice, and a third call has nothing to do.
    assert_eq!(engine.images.load(Ordering::SeqCst), 7);
    let third = read(&plan, Some(&second_index), &video, &engine, 40).await?;
    assert!(third.frames.is_empty() && third.unresolved.is_empty());
    assert_eq!(
        merge_screen_text(Some(&second_index), &third, DEFAULT_READ_PERIOD_MICROS)?,
        None
    );
    Ok(())
}

#[tokio::test]
async fn targets_close_together_share_one_listing() -> TestResult {
    let plan = whole_plan(&[])?;
    let video = FakeVideo::new()?;
    read(
        &plan,
        None,
        &video,
        &Scripted::new('1'),
        MAX_SCREEN_TEXT_FRAMES_PER_CALL,
    )
    .await?;
    // 0 to 20 s and 30 to 50 s share a listing each; 60 s is the end and needs none.
    assert_eq!(video.listings.load(Ordering::SeqCst), 2);
    assert_eq!(
        video.extractions.load(Ordering::SeqCst),
        2,
        "six frames in runs of at most four"
    );
    Ok(())
}

#[tokio::test]
async fn an_invalid_or_refused_reading_fails_its_frame_and_nothing_else() -> TestResult {
    let plan = whole_plan(&[])?;
    let video = FakeVideo::new()?;
    let engine = Scripted::new('1');
    *engine.odd.lock().map_err(|_| "poisoned")? = Some((
        String::from("100"),
        EngineReading::Lines {
            lines: vec![EngineLine {
                x: 600,
                y: 10,
                width: 200,
                height: 20,
                text: String::from("outside"),
                confidence_basis_points: 9_000,
            }],
            truncated: false,
        },
    ));
    let extension = read(&plan, None, &video, &engine, 40).await?;
    let failed: Vec<i64> = extension
        .frames
        .iter()
        .filter(|frame| frame.outcome == ScreenTextOutcome::Failed(ScreenTextFailure::EngineFailed))
        .map(|frame| frame.frame.pts)
        .collect();
    assert_eq!(failed, vec![100]);
    *engine.odd.lock().map_err(|_| "poisoned")? =
        Some((String::from("200"), EngineReading::Rejected));
    let extension = read(&plan, None, &video, &engine, 40).await?;
    assert!(extension.frames.iter().any(|frame| frame.frame.pts == 200
        && frame.outcome == ScreenTextOutcome::Failed(ScreenTextFailure::ImageRejected)));
    let index =
        merge_screen_text(None, &extension, DEFAULT_READ_PERIOD_MICROS)?.ok_or("no revision")?;
    assert_eq!(ScreenTextCoverage::of(&plan, Some(&index)).failed, 1);
    Ok(())
}

#[tokio::test]
async fn an_engine_failure_keeps_the_frames_read_before_it() -> TestResult {
    let plan = whole_plan(&[])?;
    let video = FakeVideo::new()?;
    let mut engine = Scripted::new('1');
    engine.fail_batch = Some((2, ScreenTextReadError::AbnormalTermination));
    let extension = read(&plan, None, &video, &engine, 40).await?;
    assert_eq!(
        extension.stop,
        Some(ScreenTextStop::Engine(
            ScreenTextReadError::AbnormalTermination
        ))
    );
    assert_eq!(extension.frames.len(), 4);
    let index =
        merge_screen_text(None, &extension, DEFAULT_READ_PERIOD_MICROS)?.ok_or("no revision")?;
    let coverage = ScreenTextCoverage::of(&plan, Some(&index));
    assert_eq!(
        (coverage.read, coverage.pending),
        (4, 2),
        "the failed batch's targets stay pending"
    );
    Ok(())
}

#[tokio::test]
async fn another_engine_build_starts_a_new_index() -> TestResult {
    let plan = whole_plan(&[])?;
    let video = FakeVideo::new()?;
    let first = read(&plan, None, &video, &Scripted::new('1'), 40).await?;
    let first_index =
        merge_screen_text(None, &first, DEFAULT_READ_PERIOD_MICROS)?.ok_or("no revision")?;
    let rebuilt = Scripted::new('2');
    let second = read(&plan, Some(&first_index), &video, &rebuilt, 40).await?;
    assert_eq!(
        rebuilt.images.load(Ordering::SeqCst),
        6,
        "every frame is read again by the new build"
    );
    let second_index = merge_screen_text(Some(&first_index), &second, DEFAULT_READ_PERIOD_MICROS)?
        .ok_or("no revision")?;
    assert_eq!(second_index.revision().get(), 1);
    assert_ne!(second_index.id(), first_index.id());
    Ok(())
}

#[tokio::test]
async fn a_cancelled_call_reads_nothing() -> TestResult {
    let plan = whole_plan(&[])?;
    let video = FakeVideo::new()?;
    let (session, source) = ids()?;
    let control = Control {
        cancelled: AtomicBool::new(true),
    };
    let extension = read_screen_text_within(
        ReadScreenTextRequest {
            session_id: &session,
            source_id: &source,
            profile: ScreenTextProfile::PpOcrV5LatinV1,
            previous: None,
            plan: &plan,
            max_frames: 40,
            revisions_left: 64,
        },
        &video,
        &Scripted::new('1'),
        &control,
    )
    .await?;
    assert_eq!(extension.stop, Some(ScreenTextStop::Cancelled));
    assert_eq!(extension.frames.len(), 0);
    assert_eq!(
        merge_screen_text(None, &extension, DEFAULT_READ_PERIOD_MICROS)?,
        None
    );
    Ok(())
}

/// A still screen, the same every half second.
struct StillScreen;

impl crate::VisualSampler for StillScreen {
    fn window_samples(
        &self,
        window: vsift_domain::VisualWindow,
    ) -> impl Future<Output = Result<Vec<vsift_domain::VisualSample>, crate::VisualSamplingError>> + Send
    {
        let mut samples = Vec::new();
        let mut time = window.lead_in_start().as_micros();
        while time < window.range().end().as_micros() {
            samples.push(vsift_domain::VisualSample::from_parts(
                MediaTime::from_micros(time),
                [40; vsift_domain::VISUAL_BLOCKS],
                vsift_domain::VisualHash::from_bits(40),
            ));
            time += SECOND / 2;
        }
        ready(Ok(samples))
    }
}

/// ADR 0033's background job: a 150 s video whose first 60 s window alone is
/// analysed is planned only there; the rest waits for analysis, and nothing is
/// analysed by planning.
#[tokio::test]
async fn a_plan_of_analysed_windows_waits_for_the_others() -> TestResult {
    let session = SessionId::parse("ses_0123456789abcdef")?;
    let source = SourceId::parse(format!("src_sha256_{}", "b".repeat(64)))?;
    let scope = crate::VisualIndexScope {
        session_id: &session,
        source_id: &source,
        stream_index: 0,
        displayed_dimensions: FrameDimensions::new(1280, 720)?,
        duration: MediaTime::from_micros(150 * SECOND),
        profile: vsift_domain::VisualIndexProfile::R0,
    };
    let first_window = TimeRange::new(
        MediaTime::from_micros(0),
        MediaTime::from_micros(60 * SECOND),
    )?;
    let visual = crate::extend_visual_index(
        crate::ExtendVisualIndexRequest {
            scope,
            previous: None,
            range: first_window,
        },
        &StillScreen,
    )
    .await?
    .revision
    .ok_or("no revision")?;
    let whole = TimeRange::new(
        MediaTime::from_micros(0),
        MediaTime::from_micros(400 * SECOND),
    )?;
    let planned = super::plan_analysed_windows(&visual, whole, DEFAULT_READ_PERIOD_MICROS)?;
    assert_eq!(
        planned.range.end().as_micros(),
        150 * SECOND,
        "clipped to the video"
    );
    assert!(
        planned
            .plan
            .iter()
            .all(|target| target.time.as_micros() < 60 * SECOND)
    );
    assert!(planned.plan.iter().any(|target| target.candidate.is_some()));
    // The periodic reads at 60, 70, ..., 140 s wait for their two windows.
    assert_eq!(planned.awaiting_analysis, 9);
    assert!(
        planned
            .gaps
            .iter()
            .any(|gap| gap.reason == vsift_domain::CoverageGapReason::NotAnalyzed)
    );
    let coverage = ScreenTextCoverage {
        awaiting_analysis: planned.awaiting_analysis,
        ..ScreenTextCoverage::of(&planned.plan, None)
    };
    assert!(!coverage.is_complete());
    assert_eq!(coverage.pending, planned.plan.len());
    assert_eq!(visual.windows().len(), 1, "planning analysed nothing");
    Ok(())
}
