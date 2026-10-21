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
    reads: AtomicUsize,
    images: AtomicUsize,
    odd: Mutex<Option<(String, EngineReading)>>,
    fail_batch: Option<(usize, ScreenTextReadError)>,
}

impl Scripted {
    fn new(runner: char) -> Self {
        Self {
            runner,
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
        images: &[ScreenTextImage<'_>],
    ) -> Result<Vec<EngineReading>, ScreenTextReadError> {
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
        ready(self.provider())
    }

    fn max_images_per_run(&self) -> usize {
        4
    }

    fn read(
        &self,
        images: &[ScreenTextImage<'_>],
    ) -> impl Future<Output = Result<Vec<EngineReading>, ScreenTextReadError>> + Send {
        ready(self.reading(images))
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
        },
        video,
        engine,
        &running(),
    )
    .await
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
            pending: 0
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
