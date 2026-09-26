use std::{future::Future, sync::Mutex};

use proptest::prelude::{ProptestConfig, prop_assert, prop_assert_eq, proptest};
use vsift_domain::{
    CoverageGapReason, CursorError, MediaTime, PageLimit, SessionId, SourceId, TimeRange,
    VISUAL_BLOCKS, VisualCandidate, VisualHash, VisualIndex, VisualIndexProfile, VisualSample,
    VisualWindow, VisualWindowOutcome,
};

use super::{
    CandidatePageRequest, CandidateQueryError, ExtendVisualIndexRequest, MAX_WINDOWS_PER_EXTENSION,
    VisualExtensionStop, VisualIndexBuildError, VisualIndexExtension, VisualIndexScope,
    VisualSampler, VisualSamplingError, extend_visual_index, page_candidates,
    verify_visual_index_identities, visual_coverage_gaps,
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
}

impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            session: SessionId::parse("ses_0123456789abcdef")?,
            source: SourceId::from_sha256(DIGEST)?,
        })
    }

    fn scope(&self, duration_seconds: u64) -> VisualIndexScope<'_> {
        VisualIndexScope {
            session_id: &self.session,
            source_id: &self.source,
            stream_index: 0,
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
