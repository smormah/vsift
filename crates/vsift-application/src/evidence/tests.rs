//! Evidence use cases over fake extractors: selection, identities, reuse,
//! partial results and budgets.

use std::{
    collections::BTreeSet,
    future::{Future, ready},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use vsift_domain::{
    AudioRange, BurstCount, BurstRange, EvidenceDetail, EvidenceId, EvidenceProfile,
    EvidenceRecord, EvidenceRecordParts, EvidenceSubject, FrameDimensions, FrameListing,
    FrameSelection, FrameSelectionError, FrameTolerance, ListedFrame, ListingTail, MediaTime,
    NeighbourCount, NeighbourStop, PartialReason, SelectionRole, SessionId, Sha256Hex, SourceCheck,
    SourceId, TimeBase, TimeRange, VisualCandidateId,
};

use super::{
    AudioExtractor, CropRequest, EvidenceBudget, EvidenceCall, EvidenceControl, EvidenceError,
    EvidenceExtraction, EvidenceIdentityError, EvidenceMediaError, EvidenceScope, EvidenceStop,
    ExtractedClip, ExtractedFrame, FrameAtRequest, FrameExtractor, VideoStreamFacts,
    evidence_request_key, extract_audio, extract_burst, extract_crop, extract_frame_at,
    extract_neighbours, find_evidence_item, find_reusable_record, verify_evidence_record,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const FRAME_MICROS: u64 = 50_000;
const FRAMES: i64 = 120;
const FINGERPRINT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_FINGERPRINT: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const SOURCE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn micros(value: u64) -> MediaTime {
    MediaTime::from_micros(value)
}

fn range(start: u64, end: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(micros(start), micros(end))?)
}

/// A 20 fps, 6 s stream whose frame `n` has timestamp `n` in a 1/20 time
/// base. Frames within one `static_run` share their pixels.
struct FakeVideo {
    static_run: i64,
    runs: AtomicUsize,
    listings: AtomicUsize,
    /// Fails the extraction run with this ordinal (1-based) with the error.
    fail_run: Option<(usize, EvidenceMediaError)>,
    per_run: usize,
    crops: Mutex<Vec<vsift_domain::CropRect>>,
    facts: VideoStreamFacts,
}

impl FakeVideo {
    fn new() -> Built<Self> {
        Ok(Self {
            facts: VideoStreamFacts {
                stream_index: 0,
                time_base: TimeBase::new(1, 20)?,
                displayed: FrameDimensions::new(64, 36)?,
                duration: micros(6_000_000),
            },
            static_run: 1,
            runs: AtomicUsize::new(0),
            listings: AtomicUsize::new(0),
            fail_run: None,
            per_run: 8,
            crops: Mutex::new(Vec::new()),
        })
    }

    fn provider_runs(&self) -> usize {
        self.runs.load(Ordering::SeqCst) + self.listings.load(Ordering::SeqCst)
    }

    fn image(&self, pts: i64) -> ExtractedFrame {
        let content = pts / self.static_run;
        ExtractedFrame {
            pts,
            time: frame_time(pts),
            png: format!("png of frame group {content}").into_bytes(),
        }
    }
}

fn frame_time(pts: i64) -> MediaTime {
    micros(u64::try_from(pts).unwrap_or(0) * FRAME_MICROS)
}

impl FrameExtractor for FakeVideo {
    fn stream(&self) -> VideoStreamFacts {
        self.facts
    }

    fn max_frames_per_run(&self) -> usize {
        self.per_run
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
        ready(self.extracted(pts))
    }

    fn crop(
        &self,
        pts: i64,
        rect: vsift_domain::CropRect,
    ) -> impl Future<Output = Result<ExtractedFrame, EvidenceMediaError>> + Send {
        ready(self.cropped(pts, rect))
    }
}

impl FakeVideo {
    fn listing(&self, requested: TimeRange) -> Result<FrameListing, EvidenceMediaError> {
        self.listings.fetch_add(1, Ordering::SeqCst);
        let duration = self.stream().duration;
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

    fn extracted(&self, pts: &[i64]) -> Result<Vec<ExtractedFrame>, EvidenceMediaError> {
        let run = self.runs.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some((failing, error)) = self.fail_run
            && failing == run
        {
            return Err(error);
        }
        if pts.len() > self.per_run || !pts.windows(2).all(|pair| matches!(pair, [a, b] if a < b)) {
            return Err(EvidenceMediaError::Invalid);
        }
        Ok(pts.iter().map(|value| self.image(*value)).collect())
    }

    fn cropped(
        &self,
        pts: i64,
        rect: vsift_domain::CropRect,
    ) -> Result<ExtractedFrame, EvidenceMediaError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.crops
            .lock()
            .map_err(|_| EvidenceMediaError::Invalid)?
            .push(rect);
        Ok(ExtractedFrame {
            pts,
            time: frame_time(pts),
            png: format!("crop {pts} {},{}", rect.x(), rect.y()).into_bytes(),
        })
    }
}

struct FakeAudio;

impl AudioExtractor for FakeAudio {
    fn stream_index(&self) -> u32 {
        1
    }

    fn duration(&self) -> MediaTime {
        micros(6_000_000)
    }

    fn clip(
        &self,
        clipped: TimeRange,
    ) -> impl Future<Output = Result<ExtractedClip, EvidenceMediaError>> + Send {
        ready(Ok(ExtractedClip {
            actual_start: micros(clipped.start().as_micros() + 64_000),
            wav: format!("wav {}", clipped.duration_micros()).into_bytes(),
        }))
    }
}

/// Stops the call after it was asked `continue_for` times.
struct Control {
    asked: AtomicUsize,
    continue_for: usize,
    stop: EvidenceStop,
}

impl Control {
    fn never() -> Self {
        Self {
            asked: AtomicUsize::new(0),
            continue_for: usize::MAX,
            stop: EvidenceStop::Cancelled,
        }
    }

    fn after(continue_for: usize, stop: EvidenceStop) -> Self {
        Self {
            asked: AtomicUsize::new(0),
            continue_for,
            stop,
        }
    }
}

impl EvidenceControl for Control {
    fn stop(&self) -> Option<EvidenceStop> {
        let asked = self.asked.fetch_add(1, Ordering::SeqCst);
        (asked >= self.continue_for).then_some(self.stop)
    }
}

struct Fixture {
    session: SessionId,
    source: SourceId,
    fingerprint: Sha256Hex,
    known: BTreeSet<String>,
}

impl Fixture {
    fn new() -> Built<Self> {
        Ok(Self {
            session: SessionId::parse("ses_0123456789abcdef")?,
            source: SourceId::from_sha256(SOURCE)?,
            fingerprint: Sha256Hex::parse(FINGERPRINT)?,
            known: BTreeSet::new(),
        })
    }

    fn scope<'a>(&'a self, fingerprint: Option<&'a Sha256Hex>) -> EvidenceScope<'a> {
        EvidenceScope {
            session_id: &self.session,
            source_id: &self.source,
            profile: EvidenceProfile::P09R0,
            tool_fingerprint: fingerprint,
        }
    }

    fn call<'a, C>(&'a self, control: &'a C, budget: EvidenceBudget) -> EvidenceCall<'a, C> {
        EvidenceCall {
            scope: self.scope(Some(&self.fingerprint)),
            source_check: SourceCheck::FullHash,
            budget,
            known_media: &self.known,
            control,
        }
    }
}

fn budget() -> EvidenceBudget {
    EvidenceBudget::per_call(160, u64::MAX)
}

fn at(time: u64) -> FrameAtRequest {
    FrameAtRequest {
        at: micros(time),
        selection: FrameSelection::AtOrAfter,
        tolerance: FrameTolerance::MAX,
        candidate: None,
    }
}

fn only_selection(extraction: &EvidenceExtraction) -> Built<&vsift_domain::EvidenceSelection> {
    match extraction.record.selections() {
        [selection] => Ok(selection),
        _ => Err("expected one selection".into()),
    }
}

#[tokio::test]
async fn a_frame_is_selected_by_policy_and_keeps_requested_and_actual_time() -> TestResult {
    let fixture = Fixture::new()?;
    let control = Control::never();
    let video = FakeVideo::new()?;
    let after = extract_frame_at(&fixture.call(&control, budget()), &video, at(1_025_000)).await?;
    let selection = only_selection(&after)?;
    assert_eq!(selection.timing().actual(), micros(1_050_000));
    assert_eq!(selection.timing().delta_micros(), 25_000);
    assert_eq!(selection.role(), SelectionRole::Requested);
    assert_eq!(after.media.len(), 1);
    verify_evidence_record(&after.record)?;

    let displayed = extract_frame_at(
        &fixture.call(&control, budget()),
        &video,
        FrameAtRequest {
            selection: FrameSelection::DisplayedAt,
            ..at(1_025_000)
        },
    )
    .await?;
    assert_eq!(only_selection(&displayed)?.timing().delta_micros(), -25_000);
    assert_eq!(
        extract_frame_at(
            &fixture.call(&control, budget()),
            &video,
            FrameAtRequest {
                tolerance: FrameTolerance::new(10_000)?,
                ..at(1_025_000)
            },
        )
        .await,
        Err(EvidenceError::Selection(
            FrameSelectionError::NoFrameWithinTolerance
        ))
    );
    assert_eq!(
        extract_frame_at(&fixture.call(&control, budget()), &video, at(6_000_000)).await,
        Err(EvidenceError::Selection(FrameSelectionError::AtOrAfterEnd))
    );
    Ok(())
}

#[tokio::test]
async fn two_requests_resolving_to_one_frame_share_one_item_and_one_file() -> TestResult {
    let mut fixture = Fixture::new()?;
    let control = Control::never();
    let video = FakeVideo::new()?;
    let first = extract_frame_at(&fixture.call(&control, budget()), &video, at(1_025_000)).await?;
    for media in &first.media {
        fixture.known.insert(media.sha256.as_str().to_owned());
    }
    let second = extract_frame_at(&fixture.call(&control, budget()), &video, at(1_050_000)).await?;
    assert_ne!(first.record.request_key(), second.record.request_key());
    assert_eq!(
        first
            .record
            .items()
            .first()
            .map(vsift_domain::EvidenceItem::id),
        second
            .record
            .items()
            .first()
            .map(vsift_domain::EvidenceItem::id)
    );
    assert!(
        second.media.is_empty(),
        "the file the session holds is not extracted again"
    );
    Ok(())
}

#[tokio::test]
async fn reuse_needs_the_same_key_a_known_provider_and_a_complete_record() -> TestResult {
    let fixture = Fixture::new()?;
    let control = Control::never();
    let video = FakeVideo::new()?;
    let extraction =
        extract_frame_at(&fixture.call(&control, budget()), &video, at(2_000_000)).await?;
    let records = vec![extraction.record.clone()];
    let key = extraction.record.request_key().clone();
    assert_eq!(
        find_reusable_record(&records, &key, Some(&fixture.fingerprint)),
        Some(&extraction.record)
    );
    assert_eq!(find_reusable_record(&records, &key, None), None);

    // Another provider is another key and other item identities.
    let other = Sha256Hex::parse(OTHER_FINGERPRINT)?;
    let request = extraction.record.request().clone();
    let other_key = evidence_request_key(&fixture.scope(Some(&other)), &request)?;
    assert_ne!(other_key, key);
    assert_eq!(
        find_reusable_record(&records, &other_key, Some(&other)),
        None
    );
    let call = EvidenceCall {
        scope: fixture.scope(Some(&other)),
        ..fixture.call(&control, budget())
    };
    let again = extract_frame_at(&call, &video, at(2_000_000)).await?;
    assert_ne!(
        again
            .record
            .items()
            .first()
            .map(vsift_domain::EvidenceItem::id),
        extraction
            .record
            .items()
            .first()
            .map(vsift_domain::EvidenceItem::id)
    );

    // Without a fingerprint the content is part of the identity.
    let unidentified = EvidenceCall {
        scope: fixture.scope(None),
        ..fixture.call(&control, budget())
    };
    let without = extract_frame_at(&unidentified, &video, at(2_000_000)).await?;
    verify_evidence_record(&without.record)?;
    assert_eq!(without.record.tool_fingerprint(), None);

    // A partial record is never reused.
    let mut parts = record_parts(&extraction.record);
    parts.partial = Some(PartialReason::DeadlineExceeded);
    let partial = EvidenceRecord::new(parts)?;
    assert_eq!(
        find_reusable_record(&[partial], &key, Some(&fixture.fingerprint)),
        None
    );
    Ok(())
}

fn record_parts(record: &EvidenceRecord) -> EvidenceRecordParts {
    EvidenceRecordParts {
        request_key: record.request_key().clone(),
        session_id: record.session_id().clone(),
        source_id: record.source_id().clone(),
        profile: record.profile(),
        tool_fingerprint: record.tool_fingerprint().cloned(),
        request: record.request().clone(),
        selections: record.selections().to_vec(),
        detail: record.detail(),
        items: record.items().to_vec(),
        partial: record.partial(),
    }
}

#[tokio::test]
async fn an_edited_identity_does_not_verify() -> TestResult {
    let fixture = Fixture::new()?;
    let control = Control::never();
    let extraction =
        extract_frame_at(&fixture.call(&control, budget()), &FakeVideo::new()?, at(0)).await?;
    let mut parts = record_parts(&extraction.record);
    parts.request_key = vsift_domain::OperationKey::from_sha256(FINGERPRINT)?;
    assert_eq!(
        verify_evidence_record(&EvidenceRecord::new(parts)?),
        Err(EvidenceIdentityError::Mismatch)
    );
    Ok(())
}

#[tokio::test]
async fn a_candidate_frame_must_lie_exactly_at_its_time() -> TestResult {
    let fixture = Fixture::new()?;
    let control = Control::never();
    let candidate = FrameAtRequest {
        at: micros(1_050_000),
        selection: FrameSelection::AtOrAfter,
        tolerance: FrameTolerance::EXACT,
        candidate: Some(VisualCandidateId::parse("vcd_0123456789abcdef")?),
    };
    let video = FakeVideo::new()?;
    let exact =
        extract_frame_at(&fixture.call(&control, budget()), &video, candidate.clone()).await?;
    assert_eq!(only_selection(&exact)?.timing().delta_micros(), 0);
    assert_eq!(
        extract_frame_at(
            &fixture.call(&control, budget()),
            &video,
            FrameAtRequest {
                at: micros(1_060_000),
                ..candidate
            },
        )
        .await,
        Err(EvidenceError::CandidateMismatch)
    );
    Ok(())
}

#[tokio::test]
async fn neighbours_are_consecutive_nearest_first_and_say_why_a_side_is_short() -> TestResult {
    let fixture = Fixture::new()?;
    let control = Control::never();
    let video = FakeVideo::new()?;
    let anchor = extract_frame_at(&fixture.call(&control, budget()), &video, at(3_000_000))
        .await?
        .record;
    let anchor = anchor.items().first().ok_or("no anchor")?;
    let around = extract_neighbours(
        &fixture.call(&control, budget()),
        &video,
        anchor,
        NeighbourCount::new(3)?,
    )
    .await?;
    let times: Vec<(SelectionRole, u64)> = around
        .record
        .selections()
        .iter()
        .map(|selection| (selection.role(), selection.timing().actual().as_micros()))
        .collect();
    assert_eq!(
        times,
        [
            (SelectionRole::Before, 2_850_000),
            (SelectionRole::Before, 2_900_000),
            (SelectionRole::Before, 2_950_000),
            (SelectionRole::After, 3_050_000),
            (SelectionRole::After, 3_100_000),
            (SelectionRole::After, 3_150_000),
        ]
    );
    assert_eq!(
        around.record.detail(),
        EvidenceDetail::Neighbours {
            before_stop: None,
            after_stop: None
        }
    );
    // Nearest first: the first run holds the closest frames.
    assert_eq!(
        around
            .record
            .items()
            .first()
            .and_then(|item| item.subject().frame())
            .map(|frame| frame.pts),
        Some(61)
    );

    let first = extract_frame_at(&fixture.call(&control, budget()), &video, at(50_000))
        .await?
        .record;
    let first = first.items().first().ok_or("no anchor")?;
    let start = extract_neighbours(
        &fixture.call(&control, budget()),
        &video,
        first,
        NeighbourCount::new(3)?,
    )
    .await?;
    assert_eq!(
        start.record.detail(),
        EvidenceDetail::Neighbours {
            before_stop: Some(NeighbourStop::StartOfStream),
            after_stop: None
        }
    );
    Ok(())
}

#[tokio::test]
async fn a_burst_is_deduplicated_and_partial_on_a_later_deadline() -> TestResult {
    let fixture = Fixture::new()?;
    let control = Control::never();
    let video = FakeVideo {
        static_run: 10,
        per_run: 4,
        fail_run: Some((2, EvidenceMediaError::Deadline)),
        ..FakeVideo::new()?
    };
    let burst = extract_burst(
        &fixture.call(&control, budget()),
        &video,
        BurstRange::new(range(0, 2_000_000)?)?,
        BurstCount::new(10)?,
    )
    .await?;
    assert_eq!(
        burst.record.partial(),
        Some(PartialReason::DeadlineExceeded)
    );
    assert_eq!(burst.record.items().len(), 4);
    assert_eq!(
        burst.record.detail(),
        EvidenceDetail::Burst {
            planned: range(0, 2_000_000)?,
            extent: vsift_domain::BurstExtent::Requested,
            targets: 10,
            distinct: 10,
        }
    );
    // Frames 0..=39 fall into four static groups of ten; the first run
    // extracted frames 0, 4, 8 and 12 (groups 0, 0, 0, 1): two files.
    assert_eq!(burst.media.len(), 2);
    assert!(burst.record.selections().len() <= 4);
    verify_evidence_record(&burst.record)?;

    // A deadline before anything was extracted fails the call.
    let failing = FakeVideo {
        fail_run: Some((1, EvidenceMediaError::Deadline)),
        ..FakeVideo::new()?
    };
    assert_eq!(
        extract_burst(
            &fixture.call(&control, budget()),
            &failing,
            BurstRange::new(range(0, 2_000_000)?)?,
            BurstCount::new(10)?,
        )
        .await,
        Err(EvidenceError::Media(EvidenceMediaError::Deadline))
    );
    Ok(())
}

#[tokio::test]
async fn budgets_stop_a_call_early_and_an_exhausted_session_runs_nothing() -> TestResult {
    let fixture = Fixture::new()?;
    let control = Control::never();
    let video = FakeVideo::new()?;
    let exhausted = EvidenceBudget::per_call(1, u64::MAX);
    assert_eq!(
        extract_frame_at(&fixture.call(&control, exhausted), &video, at(0)).await,
        Err(EvidenceError::BudgetExhausted)
    );
    assert_eq!(video.provider_runs(), 0, "no provider runs without room");

    let burst = |slots, pixels| {
        let budget = EvidenceBudget {
            max_pixels: pixels,
            ..EvidenceBudget::per_call(slots, u64::MAX)
        };
        let fixture = &fixture;
        let control = &control;
        async move {
            let video = FakeVideo::new()?;
            extract_burst(
                &fixture.call(control, budget),
                &video,
                BurstRange::new(range(0, 1_000_000)?)?,
                BurstCount::new(5)?,
            )
            .await
            .map_err(Box::<dyn std::error::Error>::from)
        }
    };
    let slots = burst(3, u64::MAX).await?;
    assert_eq!(
        slots.record.partial(),
        Some(PartialReason::SessionEvidenceBudget)
    );
    assert_eq!(slots.media.len(), 2);
    let pixels = burst(160, 64 * 36 * 2).await?;
    assert_eq!(pixels.record.partial(), Some(PartialReason::PixelBudget));
    assert_eq!(pixels.record.items().len(), 2);

    let stopped = extract_burst(
        &fixture.call(&Control::after(0, EvidenceStop::Cancelled), budget()),
        &video,
        BurstRange::new(range(0, 1_000_000)?)?,
        BurstCount::new(5)?,
    )
    .await;
    assert_eq!(
        stopped,
        Err(EvidenceError::Stopped(EvidenceStop::Cancelled))
    );
    let cancelled_later = extract_burst(
        &fixture.call(&Control::after(2, EvidenceStop::Cancelled), budget()),
        &FakeVideo {
            per_run: 2,
            ..FakeVideo::new()?
        },
        BurstRange::new(range(0, 1_000_000)?)?,
        BurstCount::new(5)?,
    )
    .await?;
    assert_eq!(
        cancelled_later.record.partial(),
        Some(PartialReason::Cancelled)
    );
    assert_eq!(cancelled_later.record.items().len(), 2);
    Ok(())
}

#[tokio::test]
async fn a_crop_of_a_crop_names_source_pixels() -> TestResult {
    let fixture = Fixture::new()?;
    let control = Control::never();
    let video = FakeVideo::new()?;
    let frame = extract_frame_at(&fixture.call(&control, budget()), &video, at(1_000_000))
        .await?
        .record;
    let frame = frame.items().first().ok_or("no frame")?;
    let crop = |parent, x, y, width, height| CropRequest {
        parent,
        x,
        y,
        width,
        height,
    };
    let outer = extract_crop(
        &fixture.call(&control, budget()),
        &video,
        crop(frame, 10, 6, 40, 20),
    )
    .await?;
    let outer_item = outer.record.items().first().ok_or("no crop")?;
    let inner = extract_crop(
        &fixture.call(&control, budget()),
        &video,
        crop(outer_item, 5, 4, 30, 16),
    )
    .await?;
    let Some(EvidenceSubject::Crop { region, .. }) = inner
        .record
        .items()
        .first()
        .map(vsift_domain::EvidenceItem::subject)
    else {
        return Err("expected a crop".into());
    };
    assert_eq!(region.parent, *outer_item.id());
    assert_eq!(
        (region.frame_rect.x(), region.frame_rect.y()),
        (15, 10),
        "composed to frame coordinates"
    );
    assert_eq!(
        extract_crop(
            &fixture.call(&control, budget()),
            &video,
            crop(outer_item, 11, 0, 30, 1),
        )
        .await,
        Err(EvidenceError::CropOutsideParent)
    );
    let audio = extract_audio(
        &fixture.call(&control, budget()),
        &FakeAudio,
        AudioRange::new(range(0, 1_000_000)?)?,
    )
    .await?
    .record;
    let audio = audio.items().first().ok_or("no clip")?;
    assert_eq!(
        extract_crop(
            &fixture.call(&control, budget()),
            &video,
            crop(audio, 0, 0, 1, 1),
        )
        .await,
        Err(EvidenceError::KindMismatch)
    );
    let records = vec![outer.record.clone(), inner.record.clone()];
    assert_eq!(
        find_evidence_item(&records, outer_item.id()).map(vsift_domain::EvidenceItem::id),
        Some(outer_item.id())
    );
    assert_eq!(
        find_evidence_item(&records, &EvidenceId::parse("evd_ffffffffffffffff")?),
        None
    );
    Ok(())
}

#[tokio::test]
async fn an_audio_clip_is_clipped_to_the_source_and_reports_its_first_sample() -> TestResult {
    let fixture = Fixture::new()?;
    let control = Control::never();
    let clip = extract_audio(
        &fixture.call(&control, budget()),
        &FakeAudio,
        AudioRange::new(range(5_000_000, 8_000_000)?)?,
    )
    .await?;
    assert_eq!(
        clip.record.detail(),
        EvidenceDetail::Audio {
            range_clipped: true
        }
    );
    assert_eq!(only_selection(&clip)?.timing().delta_micros(), 64_000);
    let Some(EvidenceSubject::Audio { range: clipped, .. }) = clip
        .record
        .items()
        .first()
        .map(vsift_domain::EvidenceItem::subject)
    else {
        return Err("expected a clip".into());
    };
    assert_eq!(*clipped, range(5_000_000, 6_000_000)?);
    assert_eq!(
        extract_audio(
            &fixture.call(&control, budget()),
            &FakeAudio,
            AudioRange::new(range(6_000_000, 7_000_000)?)?,
        )
        .await,
        Err(EvidenceError::RangeOutsideSource)
    );
    Ok(())
}
