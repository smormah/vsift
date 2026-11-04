use std::{future::Future, sync::Mutex};

use vsift_domain::{
    CropRect, CursorError, EvidenceId, FrameComparison, FrameDimensions, MediaTime, SessionId,
    SourceId, TimeRange, VISUAL_BLOCKS, VisualCandidateId, VisualHash, VisualIndex,
    VisualIndexProfile, VisualSample, VisualWindow,
};

use super::{
    ComparedPage, CropStop, FrameRefusal, MAX_CROPS_PER_CALL, PageContinuation, ScreenChangeCrops,
    ScreenChangeError, ScreenChangeFrames, ScreenChangePageRequest, ScreenChangeQueryError,
    ScreenChangeResume, ScreenChangeResumeReason, ScreenChangeStop, TakenFrame,
    compare_screen_changes, crop_screen_changes, first_unanalysed, page_continuation,
    page_screen_changes, screen_change_id, screen_change_page_limit, screen_change_query_digest,
};
use crate::transcript::sha256_hex;
use crate::visual::candidate_query_digest;
use crate::visual::{
    ExtendVisualIndexRequest, VisualIndexScope, VisualSampler, VisualSamplingError,
    extend_visual_index, visual_coverage_gaps,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const EXPIRES: u64 = 2_000_000_000_000_000;
const NOW: u64 = 1_000_000_000_000_000;
const SECOND: u64 = 1_000_000;

/// A dialog shown from 5 s to 9 s of every minute: two changes a minute.
struct DialogSampler;

impl VisualSampler for DialogSampler {
    fn window_samples(
        &self,
        window: VisualWindow,
    ) -> impl Future<Output = Result<Vec<VisualSample>, VisualSamplingError>> + Send {
        let mut samples = Vec::new();
        let mut time = window.lead_in_start().as_micros();
        while time < window.range().end().as_micros() {
            let level = if (5 * SECOND..9 * SECOND).contains(&(time % (60 * SECOND))) {
                90
            } else {
                40
            };
            samples.push(VisualSample::from_parts(
                MediaTime::from_micros(time),
                [level; VISUAL_BLOCKS],
                VisualHash::from_bits(u64::from(level)),
            ));
            time += SECOND / 2;
        }
        std::future::ready(Ok(samples))
    }
}

struct Fixture {
    session: SessionId,
    source: SourceId,
    dimensions: FrameDimensions,
    duration: MediaTime,
}

impl Fixture {
    fn new(duration_minutes: u64) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            session: SessionId::parse("ses_0123456789abcdef")?,
            source: SourceId::from_sha256(DIGEST)?,
            dimensions: FrameDimensions::new(64, 32)?,
            duration: MediaTime::from_micros(duration_minutes * 60 * SECOND),
        })
    }

    fn scope(&self) -> VisualIndexScope<'_> {
        VisualIndexScope {
            session_id: &self.session,
            source_id: &self.source,
            stream_index: 0,
            displayed_dimensions: self.dimensions,
            duration: self.duration,
            profile: VisualIndexProfile::R0,
        }
    }

    /// A call without a cursor: analyses at most thirty windows of `range`.
    async fn extend(
        &self,
        previous: Option<&VisualIndex>,
        range: TimeRange,
    ) -> Result<VisualIndex, Box<dyn std::error::Error>> {
        self.extend_with(previous, range, &DialogSampler).await
    }

    async fn extend_with(
        &self,
        previous: Option<&VisualIndex>,
        range: TimeRange,
        sampler: &impl VisualSampler,
    ) -> Result<VisualIndex, Box<dyn std::error::Error>> {
        let extension = extend_visual_index(
            ExtendVisualIndexRequest {
                scope: self.scope(),
                previous,
                range,
            },
            sampler,
        )
        .await?;
        Ok(match (extension.revision, previous) {
            (Some(revision), _) => revision,
            (None, Some(previous)) => previous.clone(),
            (None, None) => return Err("nothing analysed".into()),
        })
    }
}

fn range(from_seconds: u64, to_seconds: u64) -> Result<TimeRange, Box<dyn std::error::Error>> {
    Ok(TimeRange::new(
        MediaTime::from_micros(from_seconds * SECOND),
        MediaTime::from_micros(to_seconds * SECOND),
    )?)
}

fn request(
    range: TimeRange,
    limit: u16,
    cursor: Option<String>,
    crops: ScreenChangeCrops,
) -> Result<ScreenChangePageRequest, Box<dyn std::error::Error>> {
    Ok(ScreenChangePageRequest {
        range,
        limit: screen_change_page_limit(Some(limit))?,
        cursor,
        crops,
    })
}

/// Pages `range` of `index` to the end of its analysed part.
fn every_page(
    fixture: &Fixture,
    index: &VisualIndex,
    range: TimeRange,
    limit: u16,
) -> Result<(Vec<MediaTime>, usize), Box<dyn std::error::Error>> {
    let mut afters = Vec::new();
    let mut pages = 0;
    let mut cursor = None;
    loop {
        let page = page_screen_changes(
            &fixture.session,
            index,
            &request(range, limit, cursor, ScreenChangeCrops::NoCrops)?,
            NOW,
        )?;
        pages += 1;
        afters.extend(page.changes.iter().map(|change| change.after));
        match (page.later, page.changes.last()) {
            (0, _) | (_, None) => return Ok((afters, pages)),
            (_, Some(last)) => {
                cursor = Some(page.cursor_after(&fixture.session, last, EXPIRES)?.encode());
            }
        }
    }
}

#[tokio::test]
async fn a_range_over_thirty_windows_is_paged_to_its_end_then_resumed() -> TestResult {
    // Forty minutes: one call analyses thirty windows; the pages reach the end
    // of what it analysed, `resume` names the first window still to analyse,
    // and a fresh call from there lists the rest. Every change appears once.
    let fixture = Fixture::new(40)?;
    let whole = range(0, 40 * 60)?;
    let index = fixture.extend(None, whole).await?;
    let (first, pages) = every_page(&fixture, &index, whole, 20)?;
    assert_eq!(first.len(), 60, "two changes in each of thirty minutes");
    assert_eq!(pages, 3);
    let gaps = visual_coverage_gaps(Some(&index), fixture.duration, whole, None);
    let resume = first_unanalysed(&gaps).ok_or("no resume point")?;
    assert_eq!(resume, MediaTime::from_micros(30 * 60 * SECOND));

    let rest = TimeRange::new(resume, whole.end())?;
    let index = fixture.extend(Some(&index), rest).await?;
    let (second, _) = every_page(&fixture, &index, rest, 20)?;
    assert_eq!(second.len(), 20);
    let gaps = visual_coverage_gaps(Some(&index), fixture.duration, whole, None);
    assert_eq!(first_unanalysed(&gaps), None);

    let mut all = first;
    all.extend(second);
    let mut unique = all.clone();
    unique.dedup();
    assert_eq!(unique.len(), 80);
    assert!(all.windows(2).all(|pair| pair.first() < pair.get(1)));
    assert_eq!(all.first(), Some(&MediaTime::from_micros(5 * SECOND)));
    Ok(())
}

#[tokio::test]
async fn the_cursor_is_bound_to_the_page_size_crops_and_window_states() -> TestResult {
    let fixture = Fixture::new(3)?;
    let first_minute = range(0, 60)?;
    let index = fixture.extend(None, range(0, 60)?).await?;
    let page = page_screen_changes(
        &fixture.session,
        &index,
        &request(range(0, 120)?, 1, None, ScreenChangeCrops::NoCrops)?,
        NOW,
    )?;
    let last = page.changes.last().ok_or("a change")?;
    let cursor = page.cursor_after(&fixture.session, last, EXPIRES)?.encode();
    let rejected = |result: Result<_, ScreenChangeQueryError>| {
        matches!(
            result,
            Err(ScreenChangeQueryError::Cursor(CursorError::WrongQuery))
        )
    };
    assert!(rejected(page_screen_changes(
        &fixture.session,
        &index,
        &request(
            range(0, 120)?,
            2,
            Some(cursor.clone()),
            ScreenChangeCrops::NoCrops
        )?,
        NOW,
    )));
    assert!(rejected(page_screen_changes(
        &fixture.session,
        &index,
        &request(
            range(0, 120)?,
            1,
            Some(cursor.clone()),
            ScreenChangeCrops::LargestRegions
        )?,
        NOW,
    )));
    assert!(rejected(page_screen_changes(
        &fixture.session,
        &index,
        &request(
            first_minute,
            1,
            Some(cursor.clone()),
            ScreenChangeCrops::NoCrops
        )?,
        NOW,
    )));
    // A window of the range analysed since rejects it.
    let extended = fixture.extend(Some(&index), range(60, 120)?).await?;
    assert!(rejected(page_screen_changes(
        &fixture.session,
        &extended,
        &request(
            range(0, 120)?,
            1,
            Some(cursor.clone()),
            ScreenChangeCrops::NoCrops
        )?,
        NOW,
    )));
    // The same query continues.
    let next = page_screen_changes(
        &fixture.session,
        &index,
        &request(range(0, 120)?, 1, Some(cursor), ScreenChangeCrops::NoCrops)?,
        NOW,
    )?;
    assert_eq!(
        next.changes.first().map(|change| change.after),
        Some(MediaTime::from_micros(9 * SECOND))
    );
    Ok(())
}

#[test]
fn a_page_holds_one_to_twenty_changes() {
    assert_eq!(
        screen_change_page_limit(None).map(vsift_domain::PageLimit::get),
        Ok(10)
    );
    assert!(screen_change_page_limit(Some(0)).is_err());
    assert!(screen_change_page_limit(Some(21)).is_err());
    assert_eq!(
        screen_change_page_limit(Some(20)).map(vsift_domain::PageLimit::get),
        Ok(20)
    );
}

#[tokio::test]
async fn a_change_id_is_stable_and_names_its_candidates() -> TestResult {
    let fixture = Fixture::new(2)?;
    let index = fixture.extend(None, range(0, 120)?).await?;
    let page = page_screen_changes(
        &fixture.session,
        &index,
        &request(range(0, 120)?, 20, None, ScreenChangeCrops::NoCrops)?,
        NOW,
    )?;
    let ids = page
        .changes
        .iter()
        .map(|change| screen_change_id(&fixture.session, &index, change))
        .collect::<Result<Vec<_>, _>>()?;
    let again = page
        .changes
        .iter()
        .map(|change| screen_change_id(&fixture.session, &index, change))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(ids, again);
    assert_eq!(ids.len(), 4);
    let mut distinct = ids.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 4);
    assert!(ids.iter().all(|id| id.as_str().starts_with("chg_")));
    Ok(())
}

#[derive(Debug, PartialEq)]
enum PortError {
    Failed,
    Pixels,
}

impl std::fmt::Display for PortError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Failed => "the stand-in failed",
            Self::Pixels => "a pixel buffer did not match its frame",
        })
    }
}

impl std::error::Error for PortError {}

impl From<ScreenChangeError> for PortError {
    fn from(_: ScreenChangeError) -> Self {
        Self::Pixels
    }
}

/// A port that has room for `room` frames and crops, stops after
/// `stop_after` of them, and draws every after frame with `regions` changed
/// blocks along the top row, four tiles apart so none join: block `i` starts
/// at x = 64 * i and has 16 - i changed pixels, so the first is the largest.
struct StandIn {
    room: usize,
    stop_after: Option<(usize, ScreenChangeStop)>,
    fail_at: Option<usize>,
    taken: Mutex<usize>,
    dimensions: FrameDimensions,
    regions: u32,
    crops: Mutex<Vec<(EvidenceId, CropRect)>>,
}

impl StandIn {
    fn new(room: usize) -> Result<Self, Box<dyn std::error::Error>> {
        Self::with_regions(room, 1)
    }

    fn with_regions(room: usize, regions: u32) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            room,
            stop_after: None,
            fail_at: None,
            taken: Mutex::new(0),
            dimensions: FrameDimensions::new(256, 32)?,
            regions,
            crops: Mutex::new(Vec::new()),
        })
    }

    /// The crops asked for, in order: each one's parent and rectangle.
    fn crops_asked(&self) -> Vec<(EvidenceId, CropRect)> {
        self.crops
            .lock()
            .map(|crops| crops.clone())
            .unwrap_or_default()
    }

    fn take(&self, time: MediaTime, after: bool) -> Result<TakenFrame, FrameRefusal<PortError>> {
        let mut taken = self
            .taken
            .lock()
            .map_err(|_| FrameRefusal::Failed(PortError::Failed))?;
        if self.fail_at == Some(*taken) {
            return Err(FrameRefusal::Failed(PortError::Failed));
        }
        if *taken >= self.room {
            return Err(FrameRefusal::NoRoom);
        }
        *taken += 1;
        let kind = u8::from(after);
        Ok(TakenFrame {
            evidence_id: EvidenceId::parse(format!("evd_{kind}{:031}", time.as_micros()))
                .map_err(|_| FrameRefusal::Failed(PortError::Failed))?,
            time,
            dimensions: self.dimensions,
        })
    }
}

impl ScreenChangeFrames for StandIn {
    type Error = PortError;

    fn stop(&self) -> Option<ScreenChangeStop> {
        let taken = self.taken.lock().map_or(0, |taken| *taken);
        self.stop_after
            .filter(|(after, _)| taken >= *after)
            .map(|(_, stop)| stop)
    }

    fn before_frame(
        &self,
        at: MediaTime,
    ) -> impl Future<Output = Result<TakenFrame, FrameRefusal<PortError>>> + Send {
        std::future::ready(self.take(at, false))
    }

    fn after_frame(
        &self,
        candidate: &VisualCandidateId,
    ) -> impl Future<Output = Result<TakenFrame, FrameRefusal<PortError>>> + Send {
        let serial = candidate
            .as_str()
            .trim_start_matches("vcd_")
            .parse::<u64>()
            .unwrap_or(0);
        std::future::ready(self.take(MediaTime::from_micros(serial), true))
    }

    fn pixels(&self, frame: &TakenFrame) -> Result<Vec<u8>, PortError> {
        let width = usize::try_from(self.dimensions.width()).map_err(|_| PortError::Pixels)?;
        let height = usize::try_from(self.dimensions.height()).map_err(|_| PortError::Pixels)?;
        let mut pixels = vec![0_u8; width * height * 3];
        if frame.evidence_id.as_str().starts_with("evd_1") {
            for block in 0..usize::try_from(self.regions).map_err(|_| PortError::Pixels)? {
                let start = block * 64 * 3;
                let end = start + (16 - block) * 3;
                pixels
                    .get_mut(start..end)
                    .ok_or(PortError::Pixels)?
                    .fill(255);
            }
        }
        Ok(pixels)
    }

    fn crop(
        &self,
        frame: &TakenFrame,
        rect: CropRect,
    ) -> impl Future<Output = Result<EvidenceId, FrameRefusal<PortError>>> + Send {
        std::future::ready(self.cut(frame, rect))
    }
}

impl StandIn {
    fn cut(
        &self,
        frame: &TakenFrame,
        rect: CropRect,
    ) -> Result<EvidenceId, FrameRefusal<PortError>> {
        let mut taken = self
            .taken
            .lock()
            .map_err(|_| FrameRefusal::Failed(PortError::Failed))?;
        if self.fail_at == Some(*taken) {
            return Err(FrameRefusal::Failed(PortError::Failed));
        }
        if *taken >= self.room {
            return Err(FrameRefusal::NoRoom);
        }
        *taken += 1;
        let mut crops = self
            .crops
            .lock()
            .map_err(|_| FrameRefusal::Failed(PortError::Failed))?;
        crops.push((frame.evidence_id.clone(), rect));
        EvidenceId::parse(format!("evd_2{:031}", crops.len()))
            .map_err(|_| FrameRefusal::Failed(PortError::Failed))
    }
}

fn planned(
    count: u64,
) -> Result<Vec<vsift_domain::PlannedScreenChange>, Box<dyn std::error::Error>> {
    (1..=count)
        .map(|serial| {
            Ok(vsift_domain::PlannedScreenChange {
                kind: vsift_domain::ScreenChangeKind::Change,
                candidates: vec![VisualCandidateId::parse(format!("vcd_{serial:032}"))?],
                window_ordinal: 0,
                before: MediaTime::from_micros(serial * 10),
                after: MediaTime::from_micros(serial * 10 + 5),
            })
        })
        .collect()
}

#[tokio::test]
async fn each_change_is_compared_from_its_two_frames() -> TestResult {
    let port = StandIn::new(100)?;
    let page = compare_screen_changes(&port, &planned(3)?).await?;
    assert_eq!(page.stop, None);
    assert_eq!(page.changes.len(), 3);
    for change in &page.changes {
        let FrameComparison::Compared(regions) = &change.comparison else {
            return Err("not compared".into());
        };
        assert_eq!(regions.regions.len(), 1);
        assert_eq!(
            regions.regions.first().map(|region| region.changed_pixels),
            Some(16)
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_full_session_stops_the_page_at_the_last_change_with_both_frames() -> TestResult {
    // Room for five frames: two changes, then the third's after frame does
    // not fit, so the third is not listed.
    let port = StandIn::new(5)?;
    let page = compare_screen_changes(&port, &planned(4)?).await?;
    assert_eq!(page.changes.len(), 2);
    assert_eq!(page.stop, Some(ScreenChangeStop::SessionEvidenceBudget));
    Ok(())
}

#[tokio::test]
async fn a_deadline_or_cancellation_stops_the_page() -> TestResult {
    for stop in [
        ScreenChangeStop::DeadlineExceeded,
        ScreenChangeStop::Cancelled,
    ] {
        let mut port = StandIn::new(100)?;
        port.stop_after = Some((2, stop));
        let page = compare_screen_changes(&port, &planned(4)?).await?;
        assert_eq!(page.changes.len(), 1);
        assert_eq!(page.stop, Some(stop));
    }
    Ok(())
}

#[tokio::test]
async fn any_other_failure_fails_the_call() -> TestResult {
    let mut port = StandIn::new(100)?;
    port.fail_at = Some(3);
    assert_eq!(
        compare_screen_changes(&port, &planned(4)?).await,
        Err(PortError::Failed)
    );
    Ok(())
}

/// Motion runs: three changing samples, then a settled screen. In the first
/// minute one run every 12 s, which the budget keeps whole; in the second one
/// every 3 s, about forty candidates, which a window's budget of 32 cannot keep.
struct RunsSampler;

impl RunsSampler {
    fn level(time: u64) -> u8 {
        let period = if time < 60 * SECOND { 12 } else { 3 } * SECOND;
        // The sample's place in its period: 0, 1, 2 and 3 move, the rest hold.
        let step = (time % period) / (SECOND / 2);
        u8::try_from(40 + 20 * step.min(3)).unwrap_or(u8::MAX)
    }
}

impl VisualSampler for RunsSampler {
    fn window_samples(
        &self,
        window: VisualWindow,
    ) -> impl Future<Output = Result<Vec<VisualSample>, VisualSamplingError>> + Send {
        let mut samples = Vec::new();
        let mut time = window.lead_in_start().as_micros();
        while time < window.range().end().as_micros() {
            let level = Self::level(time);
            samples.push(VisualSample::from_parts(
                MediaTime::from_micros(time),
                [level; VISUAL_BLOCKS],
                VisualHash::from_bits(u64::from(level)),
            ));
            time += SECOND / 2;
        }
        std::future::ready(Ok(samples))
    }
}

#[tokio::test]
async fn a_window_whose_budget_dropped_candidates_never_pairs_a_run() -> TestResult {
    use vsift_domain::{MotionPart, ScreenChangeKind, VisualWindowOutcome, plan_screen_changes};

    let fixture = Fixture::new(2)?;
    let index = fixture
        .extend_with(None, range(0, 120)?, &RunsSampler)
        .await?;
    let dropped = |ordinal| match index
        .window(ordinal)
        .map(vsift_domain::VisualIndexWindow::outcome)
    {
        Some(VisualWindowOutcome::Analysed {
            dropped_candidates, ..
        }) => Some(*dropped_candidates),
        _ => None,
    };
    assert_eq!(dropped(0), Some(0));
    assert!(dropped(1).is_some_and(|count| count > 0));
    let kinds = |from, to| -> Result<Vec<ScreenChangeKind>, Box<dyn std::error::Error>> {
        Ok(plan_screen_changes(&index, range(from, to)?)
            .iter()
            .map(|change| change.kind)
            .collect())
    };
    let whole = kinds(0, 60)?;
    assert_ne!(whole, Vec::new());
    assert!(
        whole
            .iter()
            .all(|kind| *kind == ScreenChangeKind::Motion(MotionPart::Run))
    );
    let budgeted = kinds(60, 120)?;
    assert_ne!(budgeted, Vec::new());
    assert!(
        budgeted
            .iter()
            .all(|kind| *kind != ScreenChangeKind::Motion(MotionPart::Run)),
        "{budgeted:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_stopped_page_resumes_at_its_first_change_not_listed() -> TestResult {
    let fixture = Fixture::new(3)?;
    let index = fixture.extend(None, range(0, 180)?).await?;
    let page = page_screen_changes(
        &fixture.session,
        &index,
        &request(range(0, 180)?, 4, None, ScreenChangeCrops::NoCrops)?,
        NOW,
    )?;
    let afters: Vec<MediaTime> = page.changes.iter().map(|change| change.after).collect();
    assert_eq!(afters.len(), 4);
    let port = StandIn::new(100)?;
    let full = compare_screen_changes(&port, &page.changes).await?;
    let one = ComparedPage {
        changes: full.changes.iter().take(1).cloned().collect(),
        stop: Some(ScreenChangeStop::SessionEvidenceBudget),
    };
    assert_eq!(
        page_continuation(&page, &one, &[]),
        PageContinuation::Resume(ScreenChangeResume {
            from: afters[1],
            reason: ScreenChangeResumeReason::SessionEvidenceBudget,
        })
    );
    let none = ComparedPage {
        changes: Vec::new(),
        stop: Some(ScreenChangeStop::DeadlineExceeded),
    };
    assert_eq!(
        page_continuation(&page, &none, &[]),
        PageContinuation::Resume(ScreenChangeResume {
            from: afters[0],
            reason: ScreenChangeResumeReason::DeadlineExceeded,
        })
    );
    // A whole page with more after it continues by cursor, never by resume.
    assert_eq!(
        page_continuation(&page, &full, &[]),
        PageContinuation::Next(&page.changes[3])
    );
    Ok(())
}

#[tokio::test]
async fn the_last_page_resumes_at_the_first_window_still_to_analyse() -> TestResult {
    let fixture = Fixture::new(3)?;
    let index = fixture.extend(None, range(0, 60)?).await?;
    let whole = range(0, 180)?;
    let page = page_screen_changes(
        &fixture.session,
        &index,
        &request(whole, 20, None, ScreenChangeCrops::NoCrops)?,
        NOW,
    )?;
    let port = StandIn::new(100)?;
    let compared = compare_screen_changes(&port, &page.changes).await?;
    let gaps = visual_coverage_gaps(Some(&index), fixture.duration, whole, None);
    assert_eq!(
        page_continuation(&page, &compared, &gaps),
        PageContinuation::Resume(ScreenChangeResume {
            from: MediaTime::from_micros(60 * SECOND),
            reason: ScreenChangeResumeReason::NotAnalyzed,
        })
    );
    let analysed = fixture.extend(Some(&index), whole).await?;
    let gaps = visual_coverage_gaps(Some(&analysed), fixture.duration, whole, None);
    let page = page_screen_changes(
        &fixture.session,
        &analysed,
        &request(whole, 20, None, ScreenChangeCrops::NoCrops)?,
        NOW,
    )?;
    let compared = compare_screen_changes(&port, &page.changes).await?;
    assert_eq!(
        page_continuation(&page, &compared, &gaps),
        PageContinuation::Done
    );
    Ok(())
}

/// Slice 1 bound its boolean as the words `crops` and `no-crops`; the enum
/// binds exactly the same material, so a cursor issued before crops existed
/// still continues.
#[tokio::test]
async fn the_cursor_digest_binds_the_words_slice_one_bound() -> TestResult {
    let fixture = Fixture::new(1)?;
    let whole = range(0, 60)?;
    let index = fixture.extend(None, whole).await?;
    for (crops, word) in [
        (ScreenChangeCrops::NoCrops, "no-crops"),
        (ScreenChangeCrops::LargestRegions, "crops"),
    ] {
        let candidates = candidate_query_digest(&index, whole)?;
        let slice_one = format!(
            "vsift.screen-changes.v1\n{}\nscreen-change-regions-v1\n7\n{word}",
            candidates.as_str()
        );
        let digest = screen_change_query_digest(&index, &request(whole, 7, None, crops)?)?;
        assert_eq!(digest.as_str(), sha256_hex(slice_one.as_bytes()));
    }
    Ok(())
}

#[tokio::test]
async fn each_change_crops_its_two_largest_regions_before_then_after() -> TestResult {
    let port = StandIn::with_regions(100, 3)?;
    let mut page = compare_screen_changes(&port, &planned(2)?).await?;
    let summary = crop_screen_changes(&port, &mut page.changes).await?;
    assert_eq!(summary.made, 8);
    assert_eq!(summary.regions_without_crop, 0);
    assert_eq!(summary.stop, None);
    let asked = port.crops_asked();
    let parents: Vec<&str> = asked.iter().map(|(parent, _)| parent.as_str()).collect();
    let xs: Vec<u32> = asked.iter().map(|(_, rect)| rect.x()).collect();
    // Page order; the largest region (x 0) then the next (x 64); before then
    // after; the third region is never cropped.
    assert_eq!(xs, [0, 0, 64, 64, 0, 0, 64, 64]);
    for (pair, change) in parents.chunks(4).zip(&page.changes) {
        assert_eq!(
            pair,
            [
                change.before.evidence_id.as_str(),
                change.after.evidence_id.as_str(),
                change.before.evidence_id.as_str(),
                change.after.evidence_id.as_str(),
            ]
        );
    }
    for change in &page.changes {
        let FrameComparison::Compared(found) = &change.comparison else {
            return Err("not compared".into());
        };
        assert_eq!(found.regions.len(), 3);
        assert_eq!(change.crops.len(), 2);
        assert!(change.crops.iter().all(super::RegionCrops::is_complete));
        for (index, region) in found.regions.iter().take(2).enumerate() {
            assert_eq!(
                asked.get(2 * index).map(|(_, rect)| *rect),
                Some(region.rect)
            );
        }
    }
    // The page itself is untouched.
    assert_eq!(page.stop, None);
    Ok(())
}

#[tokio::test]
async fn a_call_makes_at_most_sixteen_crops() -> TestResult {
    let port = StandIn::with_regions(100, 2)?;
    let mut page = compare_screen_changes(&port, &planned(5)?).await?;
    let summary = crop_screen_changes(&port, &mut page.changes).await?;
    assert_eq!(summary.made, MAX_CROPS_PER_CALL);
    assert_eq!(port.crops_asked().len(), MAX_CROPS_PER_CALL);
    // Four changes have both regions cropped; the fifth's two keep their
    // rectangles with no crop.
    assert_eq!(summary.regions_without_crop, 2);
    assert_eq!(summary.stop, Some(CropStop::CropLimit));
    assert!(!CropStop::CropLimit.is_interrupted());
    let last = page.changes.last().ok_or("a change")?;
    assert_eq!(last.crops.len(), 2);
    assert!(
        last.crops
            .iter()
            .all(|crops| crops.before.is_none() && crops.after.is_none())
    );
    assert_eq!(page.stop, None);
    Ok(())
}

#[tokio::test]
async fn exactly_sixteen_crops_is_no_stop() -> TestResult {
    let port = StandIn::with_regions(100, 2)?;
    let mut page = compare_screen_changes(&port, &planned(4)?).await?;
    let summary = crop_screen_changes(&port, &mut page.changes).await?;
    assert_eq!(summary.made, MAX_CROPS_PER_CALL);
    assert_eq!(summary.regions_without_crop, 0);
    assert_eq!(summary.stop, None);
    Ok(())
}

#[tokio::test]
async fn changes_without_regions_have_no_crops() -> TestResult {
    let port = StandIn::with_regions(100, 0)?;
    let mut page = compare_screen_changes(&port, &planned(3)?).await?;
    let summary = crop_screen_changes(&port, &mut page.changes).await?;
    assert_eq!(summary, super::CropSummary::default());
    assert!(port.crops_asked().is_empty());
    assert!(page.changes.iter().all(|change| change.crops.is_empty()));
    // Frames of different sizes were never compared, so they have no regions.
    let mut different = page.changes.clone();
    for change in &mut different {
        change.comparison = FrameComparison::DimensionsDiffer;
    }
    let summary = crop_screen_changes(&port, &mut different).await?;
    assert_eq!(summary.made, 0);
    assert!(port.crops_asked().is_empty());
    Ok(())
}

/// The room, the deadline and a cancellation stop the cropping and are
/// reported in the summary alone: the page's own stop, and so its status and
/// how it continues, stay what the comparison made them.
#[tokio::test]
async fn a_crop_stop_leaves_the_page_untouched() -> TestResult {
    let fixture = Fixture::new(3)?;
    let index = fixture.extend(None, range(0, 180)?).await?;
    let plan = page_screen_changes(
        &fixture.session,
        &index,
        &request(range(0, 180)?, 3, None, ScreenChangeCrops::LargestRegions)?,
        NOW,
    )?;
    let without = compare_screen_changes(&StandIn::with_regions(100, 2)?, &plan.changes).await?;
    let expected = page_continuation(&plan, &without, &[]);
    assert!(matches!(expected, PageContinuation::Next(_)));
    // Three changes take six frames; the stops come after three crops.
    for (room, stop_after, stop) in [
        (9, None, CropStop::SessionEvidenceBudget),
        (
            100,
            Some((9, ScreenChangeStop::DeadlineExceeded)),
            CropStop::DeadlineExceeded,
        ),
        (
            100,
            Some((9, ScreenChangeStop::Cancelled)),
            CropStop::Cancelled,
        ),
    ] {
        let mut port = StandIn::with_regions(room, 2)?;
        port.stop_after = stop_after;
        let mut page = compare_screen_changes(&port, &plan.changes).await?;
        assert_eq!(page.stop, None);
        let summary = crop_screen_changes(&port, &mut page.changes).await?;
        assert_eq!(summary.stop, Some(stop));
        assert!(stop.is_interrupted());
        assert_eq!(summary.made, 3);
        // The first region is whole, the second has its before crop only,
        // and the four of the next two changes have none.
        assert_eq!(summary.regions_without_crop, 5);
        let first = page.changes.first().ok_or("a change")?;
        assert!(
            first
                .crops
                .first()
                .is_some_and(super::RegionCrops::is_complete)
        );
        assert!(
            first
                .crops
                .get(1)
                .is_some_and(|crops| crops.before.is_some() && crops.after.is_none())
        );
        assert_eq!(page.stop, None);
        assert_eq!(page.changes.len(), 3);
        assert_eq!(page_continuation(&plan, &page, &[]), expected);
    }
    Ok(())
}

#[tokio::test]
async fn any_other_crop_failure_fails_the_call() -> TestResult {
    let mut port = StandIn::with_regions(100, 1)?;
    port.fail_at = Some(4);
    let mut page = compare_screen_changes(&port, &planned(2)?).await?;
    assert_eq!(
        crop_screen_changes(&port, &mut page.changes).await,
        Err(PortError::Failed)
    );
    Ok(())
}
