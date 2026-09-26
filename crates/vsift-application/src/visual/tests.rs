use std::{future::Future, num::NonZeroUsize, sync::Mutex};

use proptest::prelude::{ProptestConfig, prop_assert, prop_assert_eq, proptest};
use vsift_domain::{
    CoverageGapReason, CursorError, FrameDimensions, MediaTime, PageLimit, SessionId, SourceId,
    TimeRange, VISUAL_BLOCKS, VisualCandidate, VisualHash, VisualIndex, VisualIndexProfile,
    VisualSample, VisualWindow, VisualWindowOutcome,
};

use super::{
    CandidatePageRequest, CandidateQueryError, ExtendVisualIndexRequest, MAX_WINDOWS_PER_EXTENSION,
    VisualExtensionStop, VisualIndexBuildError, VisualIndexExtension, VisualIndexScope,
    VisualSampler, VisualSamplingError, analysed_ranges, extend_visual_index,
    extend_visual_index_within, indexes_any_of, merge_visual_extension, missing_windows,
    page_candidates, verify_visual_index_identities, visual_coverage_gaps,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const EXPIRES: u64 = 2_000_000_000_000_000;
const NOW: u64 = 1_000_000_000_000_000;
const SECOND: u64 = 1_000_000;

/// Draws a screen level for a time in microseconds.
type Screen = fn(u64) -> u8;

/// A sampler that draws samples at 0.5 s from each window's lead-in, and
/// fails chosen windows.
struct FakeSampler {
    screen: Screen,
    failures: Vec<(u32, VisualSamplingError)>,
    calls: Mutex<Vec<u32>>,
}

impl FakeSampler {
    fn new(screen: Screen) -> Self {
        Self {
            screen,
            failures: Vec::new(),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing(screen: Screen, failures: Vec<(u32, VisualSamplingError)>) -> Self {
        Self {
            screen,
            failures,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<u32> {
        self.calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }
}

impl FakeSampler {
    fn answer(&self, window: VisualWindow) -> Result<Vec<VisualSample>, VisualSamplingError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(window.ordinal());
        }
        if let Some((_, error)) = self
            .failures
            .iter()
            .find(|(ordinal, _)| *ordinal == window.ordinal())
        {
            return Err(*error);
        }
        let mut samples = Vec::new();
        let mut time = window.lead_in_start().as_micros();
        while time < window.range().end().as_micros() {
            let level = (self.screen)(time);
            samples.push(VisualSample::from_parts(
                MediaTime::from_micros(time),
                [level; VISUAL_BLOCKS],
                VisualHash::from_bits(u64::from(level)),
            ));
            time += SECOND / 2;
        }
        Ok(samples)
    }
}

impl VisualSampler for FakeSampler {
    fn window_samples(
        &self,
        window: VisualWindow,
    ) -> impl Future<Output = Result<Vec<VisualSample>, VisualSamplingError>> + Send {
        std::future::ready(self.answer(window))
    }
}

fn still(_: u64) -> u8 {
    40
}

/// WAITING, a dialog from 5 s to 9 s of every minute, WAITING again.
fn dialog(time: u64) -> u8 {
    if (5 * SECOND..9 * SECOND).contains(&(time % (60 * SECOND))) {
        90
    } else {
        40
    }
}

/// A different screen every second.
fn busy(time: u64) -> u8 {
    let second = time / SECOND;
    if second.is_multiple_of(2) {
        40
    } else {
        u8::try_from(60 + (second % 60) * 3).unwrap_or(u8::MAX)
    }
}

struct Fixture {
    session: SessionId,
    source: SourceId,
    dimensions: FrameDimensions,
}

impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            session: SessionId::parse("ses_0123456789abcdef")?,
            source: SourceId::from_sha256(DIGEST)?,
            dimensions: FrameDimensions::new(1280, 720)?,
        })
    }

    fn scope(&self, duration_seconds: u64) -> VisualIndexScope<'_> {
        VisualIndexScope {
            session_id: &self.session,
            source_id: &self.source,
            stream_index: 0,
            displayed_dimensions: self.dimensions,
            duration: MediaTime::from_micros(duration_seconds * SECOND),
            profile: VisualIndexProfile::R0,
        }
    }
}

fn range(from_seconds: u64, to_seconds: u64) -> Result<TimeRange, Box<dyn std::error::Error>> {
    Ok(TimeRange::new(
        MediaTime::from_micros(from_seconds * SECOND),
        MediaTime::from_micros(to_seconds * SECOND),
    )?)
}

async fn extend(
    scope: VisualIndexScope<'_>,
    previous: Option<&VisualIndex>,
    requested: TimeRange,
    sampler: &FakeSampler,
) -> Result<VisualIndexExtension, VisualIndexBuildError> {
    extend_visual_index(
        ExtendVisualIndexRequest {
            scope,
            previous,
            range: requested,
        },
        sampler,
    )
    .await
}

fn ids(index: &VisualIndex, ordinal: u32) -> Vec<String> {
    index
        .window(ordinal)
        .map(|window| {
            window
                .candidates()
                .iter()
                .map(|candidate| candidate.id().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn a_long_source_is_indexed_thirty_windows_per_call_with_stable_identities() -> TestResult {
    let fixture = Fixture::new()?;
    let scope = fixture.scope(45 * 60);
    let sampler = FakeSampler::new(dialog);
    let first = extend(scope, None, range(0, 45 * 60)?, &sampler).await?;
    assert_eq!(first.recorded.len(), MAX_WINDOWS_PER_EXTENSION);
    assert_eq!(first.stop, Some(VisualExtensionStop::WindowLimit));
    let first_index = first.revision.ok_or("no revision")?;
    assert_eq!(first_index.number().get(), 1);
    verify_visual_index_identities(&scope, &first_index)?;
    let gaps = visual_coverage_gaps(Some(&first_index), scope.duration, range(0, 45 * 60)?, None);
    assert_eq!(gaps.len(), 15);
    assert!(
        gaps.iter()
            .all(|gap| gap.reason == CoverageGapReason::NotAnalyzed)
    );
    assert_eq!(
        gaps.first().map(|gap| gap.range),
        Some(range(30 * 60, 31 * 60)?)
    );

    let second = extend(scope, Some(&first_index), range(0, 45 * 60)?, &sampler).await?;
    assert_eq!(second.recorded, (30..45).collect::<Vec<u32>>());
    assert_eq!(second.stop, None);
    let second_index = second.revision.ok_or("no revision")?;
    assert_eq!(second_index.number().get(), 2);
    assert_ne!(second_index.id(), first_index.id());
    verify_visual_index_identities(&scope, &second_index)?;
    for ordinal in 0..30 {
        assert_eq!(ids(&first_index, ordinal), ids(&second_index, ordinal));
    }
    assert!(
        visual_coverage_gaps(
            Some(&second_index),
            scope.duration,
            range(0, 45 * 60)?,
            None
        )
        .is_empty()
    );
    let third = extend(scope, Some(&second_index), range(0, 45 * 60)?, &sampler).await?;
    assert_eq!(third.revision, None);
    assert_eq!(sampler.calls().len(), 45);
    Ok(())
}

#[tokio::test]
async fn a_deadline_stops_the_call_and_leaves_the_rest_retryable() -> TestResult {
    let fixture = Fixture::new()?;
    let scope = fixture.scope(5 * 60);
    let failing = FakeSampler::failing(still, vec![(2, VisualSamplingError::Deadline)]);
    let outcome = extend(scope, None, range(0, 5 * 60)?, &failing).await?;
    assert_eq!(outcome.recorded, vec![0, 1]);
    assert_eq!(
        outcome.stop,
        Some(VisualExtensionStop::Deadline { ordinal: 2 })
    );
    let index = outcome.revision.ok_or("no revision")?;
    let gaps = visual_coverage_gaps(
        Some(&index),
        scope.duration,
        range(0, 5 * 60)?,
        outcome.stop,
    );
    let reasons: Vec<(CoverageGapReason, TimeRange)> =
        gaps.iter().map(|gap| (gap.reason, gap.range)).collect();
    assert_eq!(
        reasons,
        vec![
            (CoverageGapReason::DeadlineExceeded, range(120, 180)?),
            (CoverageGapReason::NotAnalyzed, range(180, 240)?),
            (CoverageGapReason::NotAnalyzed, range(240, 300)?),
        ]
    );

    let retry = FakeSampler::new(still);
    let retried = extend(scope, Some(&index), range(0, 5 * 60)?, &retry).await?;
    assert_eq!(retried.recorded, vec![2, 3, 4]);
    assert_eq!(retry.calls(), vec![2, 3, 4]);

    for (error, stop) in [
        (
            VisualSamplingError::Busy,
            VisualExtensionStop::Busy { ordinal: 0 },
        ),
        (
            VisualSamplingError::Cancelled,
            VisualExtensionStop::Cancelled { ordinal: 0 },
        ),
    ] {
        let sampler = FakeSampler::failing(still, vec![(0, error)]);
        let outcome = extend(scope, None, range(0, 5 * 60)?, &sampler).await?;
        assert_eq!(outcome.revision, None);
        assert_eq!(outcome.stop, Some(stop));
        assert_eq!(
            visual_coverage_gaps(None, scope.duration, range(30, 5 * 60)?, outcome.stop)
                .first()
                .map(|gap| (gap.reason, gap.range)),
            Some((CoverageGapReason::NotAnalyzed, range(30, 60)?))
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_rejected_window_is_recorded_once_and_never_decoded_again() -> TestResult {
    let fixture = Fixture::new()?;
    let scope = fixture.scope(3 * 60);
    let sampler = FakeSampler::failing(still, vec![(1, VisualSamplingError::Undecodable)]);
    let outcome = extend(scope, None, range(0, 3 * 60)?, &sampler).await?;
    assert_eq!(outcome.recorded, vec![0, 1, 2]);
    let index = outcome.revision.ok_or("no revision")?;
    assert_eq!(
        index
            .window(1)
            .map(vsift_domain::VisualIndexWindow::outcome),
        Some(&VisualWindowOutcome::Undecodable)
    );
    let gaps = visual_coverage_gaps(Some(&index), scope.duration, range(0, 3 * 60)?, None);
    assert_eq!(
        gaps.iter().map(|gap| gap.reason).collect::<Vec<_>>(),
        vec![CoverageGapReason::Undecodable]
    );
    let again = FakeSampler::new(still);
    let repeated = extend(scope, Some(&index), range(0, 3 * 60)?, &again).await?;
    assert_eq!(repeated.revision, None);
    assert!(again.calls().is_empty());

    let broken = FakeSampler::failing(still, vec![(0, VisualSamplingError::Io)]);
    assert_eq!(
        extend(scope, None, range(0, 60)?, &broken).await,
        Err(VisualIndexBuildError::Sampling(VisualSamplingError::Io))
    );
    Ok(())
}

#[tokio::test]
async fn repeated_screens_stay_separate_and_budgets_are_reported() -> TestResult {
    let fixture = Fixture::new()?;
    let scope = fixture.scope(60);
    let outcome = extend(scope, None, range(0, 60)?, &FakeSampler::new(dialog)).await?;
    let index = outcome.revision.ok_or("no revision")?;
    let window = index.window(0).ok_or("window missing")?;
    let candidates: Vec<&VisualCandidate> = window.candidates().iter().collect();
    let waiting: Vec<&&VisualCandidate> = candidates
        .iter()
        .filter(|candidate| candidate.hash() == VisualHash::from_bits(40))
        .collect();
    assert!(waiting.len() >= 2);
    assert_ne!(
        waiting.first().map(|candidate| candidate.id()),
        waiting.get(1).map(|candidate| candidate.id())
    );

    let busy_outcome = extend(scope, None, range(0, 60)?, &FakeSampler::new(busy)).await?;
    let busy_index = busy_outcome.revision.ok_or("no revision")?;
    let gaps = visual_coverage_gaps(Some(&busy_index), scope.duration, range(0, 60)?, None);
    assert_eq!(gaps.len(), 1);
    assert_eq!(
        gaps.first().map(|gap| (gap.reason, gap.dropped_candidates)),
        Some((CoverageGapReason::CandidateBudgetExhausted, 28))
    );
    Ok(())
}

#[tokio::test]
async fn scope_and_range_are_checked_before_sampling() -> TestResult {
    let fixture = Fixture::new()?;
    let sampler = FakeSampler::new(still);
    let outcome = extend(fixture.scope(60), None, range(0, 60)?, &sampler).await?;
    let index = outcome.revision.ok_or("no revision")?;
    assert_eq!(
        extend(fixture.scope(120), Some(&index), range(0, 120)?, &sampler).await,
        Err(VisualIndexBuildError::ScopeMismatch)
    );
    assert_eq!(
        extend(fixture.scope(60), None, range(60, 120)?, &sampler).await,
        Err(VisualIndexBuildError::RangeOutsideSource)
    );
    let other = SessionId::parse("ses_fedcba9876543210")?;
    let foreign = VisualIndexScope {
        session_id: &other,
        ..fixture.scope(60)
    };
    assert!(verify_visual_index_identities(&foreign, &index).is_err());
    Ok(())
}

#[tokio::test]
async fn cursors_survive_unrelated_extensions_and_reject_changes_in_their_range() -> TestResult {
    let fixture = Fixture::new()?;
    let scope = fixture.scope(3 * 60);
    let sampler = FakeSampler::new(dialog);
    let index = extend(scope, None, range(0, 60)?, &sampler)
        .await?
        .revision
        .ok_or("no revision")?;
    let window_zero = range(0, 60)?;
    let limit = PageLimit::new(2)?;
    let first = page_candidates(
        &fixture.session,
        &index,
        &CandidatePageRequest {
            range: window_zero,
            limit,
            cursor: None,
        },
        EXPIRES,
        NOW,
    )?;
    assert_eq!(first.candidates.len(), 2);
    let cursor = first.next_cursor.ok_or("no cursor")?.encode();

    let extended = extend(scope, Some(&index), range(60, 120)?, &sampler)
        .await?
        .revision
        .ok_or("no revision")?;
    let mut seen: Vec<String> = first
        .candidates
        .iter()
        .map(|candidate| candidate.id().to_string())
        .collect();
    let mut next = Some(cursor.clone());
    while let Some(token) = next {
        let page = page_candidates(
            &fixture.session,
            &extended,
            &CandidatePageRequest {
                range: window_zero,
                limit,
                cursor: Some(token),
            },
            EXPIRES,
            NOW,
        )?;
        seen.extend(
            page.candidates
                .iter()
                .map(|candidate| candidate.id().to_string()),
        );
        next = page.next_cursor.map(|token| token.encode());
    }
    assert_eq!(seen, ids(&index, 0));

    let wide = range(0, 180)?;
    let wide_first = page_candidates(
        &fixture.session,
        &extended,
        &CandidatePageRequest {
            range: wide,
            limit,
            cursor: None,
        },
        EXPIRES,
        NOW,
    )?;
    let wide_cursor = wide_first.next_cursor.ok_or("no cursor")?.encode();
    let complete = extend(scope, Some(&extended), wide, &sampler)
        .await?
        .revision
        .ok_or("no revision")?;
    let rejected = |index: &VisualIndex, cursor: &str, session: &SessionId, now: u64| {
        page_candidates(
            session,
            index,
            &CandidatePageRequest {
                range: wide,
                limit,
                cursor: Some(cursor.to_owned()),
            },
            EXPIRES,
            now,
        )
        .err()
    };
    assert_eq!(
        rejected(&complete, &wide_cursor, &fixture.session, NOW),
        Some(CandidateQueryError::Cursor(CursorError::WrongQuery))
    );
    let other = SessionId::parse("ses_fedcba9876543210")?;
    assert_eq!(
        rejected(&extended, &wide_cursor, &other, NOW),
        Some(CandidateQueryError::Cursor(CursorError::WrongSession))
    );
    assert_eq!(
        rejected(&extended, &wide_cursor, &fixture.session, EXPIRES),
        Some(CandidateQueryError::Cursor(CursorError::Expired))
    );
    let forged = wide_cursor.replacen("|0-", "|0-1", 1);
    assert!(rejected(&extended, &forged, &fixture.session, NOW).is_some());
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn identities_do_not_depend_on_how_the_index_was_extended(
        minutes in 1_u64..8,
        split in 0_u64..8,
    ) {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread().build() else {
            return Ok(());
        };
        let Ok(fixture) = Fixture::new() else {
            return Ok(());
        };
        let scope = fixture.scope(minutes * 60);
        let whole = range(0, minutes * 60).ok();
        let first_part = range(0, split.clamp(1, minutes) * 60).ok();
        let (Some(whole), Some(first_part)) = (whole, first_part) else {
            return Ok(());
        };
        let sampler = FakeSampler::new(dialog);
        let at_once = runtime.block_on(extend(scope, None, whole, &sampler));
        let stepwise = runtime.block_on(async {
            let first = extend(scope, None, first_part, &sampler).await?;
            let first = first.revision;
            let second = extend(scope, first.as_ref(), whole, &sampler).await?;
            Ok::<_, VisualIndexBuildError>(second.revision.or(first))
        });
        let (Ok(at_once), Ok(Some(stepwise))) = (at_once, stepwise) else {
            prop_assert!(false, "extension failed");
            return Ok(());
        };
        let Some(at_once) = at_once.revision else {
            prop_assert!(false, "no revision");
            return Ok(());
        };
        prop_assert_eq!(at_once.windows(), stepwise.windows());
        prop_assert!(verify_visual_index_identities(&scope, &stepwise).is_ok());
    }
}

// C-03 and V-04 for `candidates` (P08 PR 4): page bounds, empty pages,
// cursor reuse and scope, complete paging, the per-call window budget, the
// union a lost commit race merges, coverage helpers, and a search hit's
// time leading to the candidate that shows what was said.

fn page(
    fixture: &Fixture,
    index: &VisualIndex,
    requested: TimeRange,
    limit: u16,
    cursor: Option<String>,
) -> Result<(Vec<String>, Option<String>), Box<dyn std::error::Error>> {
    let page = page_candidates(
        &fixture.session,
        index,
        &CandidatePageRequest {
            range: requested,
            limit: PageLimit::new(limit)?,
            cursor,
        },
        EXPIRES,
        NOW,
    )?;
    Ok((
        page.candidates
            .iter()
            .map(|candidate| candidate.id().to_string())
            .collect(),
        page.next_cursor.map(|cursor| cursor.encode()),
    ))
}

/// Every page of `requested` at `limit`, concatenated.
fn all_pages(
    fixture: &Fixture,
    index: &VisualIndex,
    requested: TimeRange,
    limit: u16,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let (ids, next) = page(fixture, index, requested, limit, cursor)?;
        seen.extend(ids);
        match next {
            Some(next) => cursor = Some(next),
            None => return Ok(seen),
        }
    }
}

fn in_range(index: &VisualIndex, requested: TimeRange) -> Vec<String> {
    index
        .candidates_in(requested)
        .map(|candidate| candidate.id().to_string())
        .collect()
}

#[tokio::test]
async fn page_limits_one_and_one_hundred_cover_the_range_and_zero_and_101_are_rejected()
-> TestResult {
    assert!(PageLimit::new(0).is_err());
    assert!(PageLimit::new(101).is_err());
    let fixture = Fixture::new()?;
    let scope = fixture.scope(5 * 60);
    let index = extend(scope, None, range(0, 5 * 60)?, &FakeSampler::new(busy))
        .await?
        .revision
        .ok_or("no revision")?;
    let whole = range(0, 5 * 60)?;
    let expected = in_range(&index, whole);
    assert!(expected.len() > 100, "the busy screen needs several pages");
    assert_eq!(all_pages(&fixture, &index, whole, 1)?, expected);
    assert_eq!(all_pages(&fixture, &index, whole, 100)?, expected);
    let (first, next) = page(&fixture, &index, whole, 100, None)?;
    assert_eq!(first.len(), 100);
    assert!(next.is_some());
    Ok(())
}

#[tokio::test]
async fn an_empty_range_is_one_final_empty_page() -> TestResult {
    let fixture = Fixture::new()?;
    let scope = fixture.scope(60);
    let index = extend(scope, None, range(0, 60)?, &FakeSampler::new(dialog))
        .await?
        .revision
        .ok_or("no revision")?;
    // Between the dialog's close at 9 s and the next cell at 10 s.
    let quiet = TimeRange::new(
        MediaTime::from_micros(9_100_000),
        MediaTime::from_micros(9_900_000),
    )?;
    assert_eq!(page(&fixture, &index, quiet, 20, None)?, (Vec::new(), None));
    Ok(())
}

/// A cursor may be used again (a retried request gets the same page); it is
/// rejected for another range, and pages never skip or repeat.
#[tokio::test]
async fn a_cursor_is_reusable_and_bound_to_its_range() -> TestResult {
    let fixture = Fixture::new()?;
    let scope = fixture.scope(2 * 60);
    let index = extend(scope, None, range(0, 2 * 60)?, &FakeSampler::new(busy))
        .await?
        .revision
        .ok_or("no revision")?;
    let whole = range(0, 2 * 60)?;
    let (_, cursor) = page(&fixture, &index, whole, 5, None)?;
    let cursor = cursor.ok_or("no cursor")?;
    let first = page(&fixture, &index, whole, 5, Some(cursor.clone()))?;
    let again = page(&fixture, &index, whole, 5, Some(cursor.clone()))?;
    assert_eq!(first, again);
    // The page size may change between pages, as for transcripts.
    let (wider, _) = page(&fixture, &index, whole, 7, Some(cursor.clone()))?;
    assert_eq!(wider.get(..5), Some(first.0.as_slice()));
    let other = page_candidates(
        &fixture.session,
        &index,
        &CandidatePageRequest {
            range: range(0, 60)?,
            limit: PageLimit::new(5)?,
            cursor: Some(cursor),
        },
        EXPIRES,
        NOW,
    );
    assert_eq!(
        other.err(),
        Some(CandidateQueryError::Cursor(CursorError::WrongQuery))
    );
    Ok(())
}

#[tokio::test]
async fn the_window_budget_bounds_a_call_and_is_clamped_to_thirty() -> TestResult {
    let fixture = Fixture::new()?;
    let scope = fixture.scope(40 * 60);
    let sampler = FakeSampler::new(still);
    let whole = range(0, 40 * 60)?;
    let request = |previous| ExtendVisualIndexRequest {
        scope,
        previous,
        range: whole,
    };
    let one = extend_visual_index_within(request(None), &sampler, NonZeroUsize::MIN).await?;
    assert_eq!(one.recorded, vec![0]);
    assert_eq!(one.stop, Some(VisualExtensionStop::WindowLimit));
    let first = one.revision.ok_or("no revision")?;
    let clamped = extend_visual_index_within(
        request(Some(&first)),
        &sampler,
        NonZeroUsize::new(1_000).ok_or("zero")?,
    )
    .await?;
    assert_eq!(clamped.recorded, (1..31).collect::<Vec<u32>>());
    assert_eq!(clamped.stop, Some(VisualExtensionStop::WindowLimit));
    Ok(())
}

/// A commit that lost a race carries only the windows the newest revision
/// lacks, keeps every window of the newest unchanged, and commits nothing
/// when the other call already recorded everything.
#[tokio::test]
async fn a_lost_commit_race_merges_onto_the_newest_revision() -> TestResult {
    let fixture = Fixture::new()?;
    let scope = fixture.scope(4 * 60);
    let sampler = FakeSampler::new(dialog);
    let base = extend(scope, None, range(0, 60)?, &sampler)
        .await?
        .revision
        .ok_or("no revision")?;
    // Two calls start from `base`: one indexes 60-180 s and commits first,
    // the other 120-240 s.
    let winner = extend(scope, Some(&base), range(60, 180)?, &sampler)
        .await?
        .revision
        .ok_or("no revision")?;
    let loser = extend(scope, Some(&base), range(120, 240)?, &sampler).await?;
    let merged = merge_visual_extension(&scope, Some(&winner), &loser.recorded_windows())?
        .ok_or("nothing merged")?;
    assert_eq!(merged.number().get(), 3);
    assert_eq!(
        merged
            .windows()
            .iter()
            .map(|window| window.window().ordinal())
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    for window in winner.windows() {
        assert_eq!(merged.window(window.window().ordinal()), Some(window));
    }
    verify_visual_index_identities(&scope, &merged)?;
    assert_eq!(
        merge_visual_extension(&scope, Some(&merged), &loser.recorded_windows())?,
        None
    );
    let other = SourceId::from_sha256(&"f".repeat(64))?;
    let foreign = VisualIndexScope {
        source_id: &other,
        ..scope
    };
    assert_eq!(
        merge_visual_extension(&foreign, Some(&winner), &loser.recorded_windows()),
        Err(VisualIndexBuildError::ScopeMismatch)
    );
    Ok(())
}

#[tokio::test]
async fn coverage_helpers_tell_analysed_missing_and_undecodable_windows_apart() -> TestResult {
    let fixture = Fixture::new()?;
    let scope = fixture.scope(4 * 60);
    let failing = FakeSampler::failing(still, vec![(1, VisualSamplingError::Undecodable)]);
    let index = extend(scope, None, range(0, 3 * 60)?, &failing)
        .await?
        .revision
        .ok_or("no revision")?;
    let whole = range(0, 10 * 60)?;
    assert_eq!(
        analysed_ranges(&index, whole),
        vec![range(0, 60)?, range(120, 180)?]
    );
    assert_eq!(
        missing_windows(Some(&index), scope.duration, whole),
        vec![3]
    );
    assert_eq!(
        missing_windows(None, scope.duration, whole),
        vec![0, 1, 2, 3]
    );
    assert!(indexes_any_of(Some(&index), range(60, 70)?));
    assert!(!indexes_any_of(Some(&index), range(200, 210)?));
    assert!(!indexes_any_of(None, whole));
    Ok(())
}

/// V-04: a search hit's time leads to the candidate that shows what was
/// said. The dialog is on screen from 5 s to 9 s; the transcript cue saying
/// "Dialog R-17 is displayed now." starts at 5.5 s. Candidates within 10 s
/// of the hit include one inside the dialog's window.
#[tokio::test]
async fn a_search_hit_time_finds_the_candidate_that_shows_it() -> TestResult {
    use std::num::NonZeroU32;

    use vsift_domain::{
        CueSource, CueText, CueTiming, ImportedCue, ParsedTranscript, SearchQuery, SidecarIdentity,
        TranscriptFormat, TranscriptOffset, TranscriptWarnings,
    };

    use crate::{
        ImportedRevisionRequest, SearchPageRequest, SuppliedTranscript, build_imported_revision,
        page_search,
    };

    let fixture = Fixture::new()?;
    let scope = fixture.scope(60);
    let index = extend(scope, None, range(0, 60)?, &FakeSampler::new(dialog))
        .await?
        .revision
        .ok_or("no revision")?;
    let text = "Dialog R-17 is displayed now.";
    let supplied = SuppliedTranscript {
        transcript: ParsedTranscript::new(
            TranscriptFormat::Srt,
            None,
            vec![ImportedCue {
                source: CueSource::new(NonZeroU32::MIN, NonZeroU32::MIN),
                timing: CueTiming::new(5_500_000, 8_000_000)?,
                text: CueText::new(text.to_owned(), text.to_owned())?,
                speaker: None,
            }],
            TranscriptWarnings::default(),
        )?,
        sidecar: SidecarIdentity::new(DIGEST, 64)?,
    };
    let revision = build_imported_revision(ImportedRevisionRequest {
        session_id: &fixture.session,
        source_id: &fixture.source,
        source_duration: scope.duration,
        supplied: &supplied,
        offset: TranscriptOffset::ZERO,
        number: NonZeroU32::MIN,
    })?;
    let hits = page_search(
        &fixture.session,
        &revision,
        &SearchPageRequest {
            query: SearchQuery::parse("R-17")?,
            range: None,
            limit: PageLimit::DEFAULT,
            cursor: None,
        },
        EXPIRES,
        NOW,
    )?;
    let hit = hits.hits.first().ok_or("no hit")?.segment().range().start();
    let lead_lag = TimeRange::new(
        MediaTime::from_micros(hit.as_micros().saturating_sub(10 * SECOND)),
        MediaTime::from_micros(hit.as_micros() + 10 * SECOND),
    )?;
    let page = page_candidates(
        &fixture.session,
        &index,
        &CandidatePageRequest {
            range: lead_lag,
            limit: PageLimit::DEFAULT,
            cursor: None,
        },
        EXPIRES,
        NOW,
    )?;
    let dialog_window = range(5, 9)?;
    assert!(page.candidates.iter().any(|candidate| {
        dialog_window.start() <= candidate.representative()
            && candidate.representative() < dialog_window.end()
    }));
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// C-03: any range and page size pages every candidate of the range
    /// exactly once, in time order.
    #[test]
    fn any_range_and_limit_page_without_gaps_or_duplicates(
        from in 0_u64..240,
        length in 1_u64..240,
        limit in 1_u16..=100,
    ) {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread().build() else {
            return Ok(());
        };
        let Ok(fixture) = Fixture::new() else {
            return Ok(());
        };
        let scope = fixture.scope(4 * 60);
        let Ok(Ok(extension)) = range(0, 4 * 60)
            .map(|whole| runtime.block_on(extend(scope, None, whole, &FakeSampler::new(busy))))
        else {
            prop_assert!(false, "extension failed");
            return Ok(());
        };
        let Some(index) = extension.revision else {
            prop_assert!(false, "no revision");
            return Ok(());
        };
        let Ok(requested) = range(from, from + length) else {
            return Ok(());
        };
        let Ok(paged) = all_pages(&fixture, &index, requested, limit) else {
            prop_assert!(false, "paging failed");
            return Ok(());
        };
        let unique: std::collections::BTreeSet<&String> = paged.iter().collect();
        prop_assert_eq!(unique.len(), paged.len());
        prop_assert_eq!(paged, in_range(&index, requested));
    }
}
