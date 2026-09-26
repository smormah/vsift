//! Visual candidates through the engine library (P08, ADR 0018).
//!
//! Sessions are opened with a plain `ingest` of a stand-in source (which
//! runs no provider) and their visual index is committed directly through
//! the session store with a controlled clock, so the default tests run
//! everywhere without `FFmpeg`: a range whose windows are all indexed is a
//! warm read. Two opt-in tests need real tools:
//!
//! - `concurrent_calls_over_one_window_commit_it_once` analyses the F05
//!   fixture from two concurrent calls; the second commit loses the race and
//!   merges onto the first (`FFmpeg` and `FFprobe` on `PATH`);
//! - `s11_a_four_hour_index_pages_warm` pages a synthetic index of the
//!   four-hour source bound and records each warm page's time, asserting the
//!   250 ms p95 target in an optimised build:
//!
//! `cargo test --release -p vsift --locked --test engine_candidates -- --ignored --nocapture`

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    ffi::OsStr,
    fs,
    num::NonZeroU32,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use vsift::{
    Cancellation, CandidateQueryError, CandidatesRange, CandidatesRequest, Clock, ClockError,
    CoverageGapReason, CursorError, Engine, EngineConfig, EngineError, EnginePorts, FailureCode,
    HostIsolation, IdentifierGenerationError, IdentifierSource, IngestRequest, MediaTime,
    OperationId, SessionId, SessionRootLocation, SourceId, UserConfigurationLocation,
};
use vsift_application::{VisualIndexScope, visual_candidate_id, visual_index_id};
use vsift_domain::{
    FrameDimensions, SessionArtifactKind, VISUAL_BLOCKS, VisualCandidate, VisualChangePolicy,
    VisualHash, VisualIndex, VisualIndexParts, VisualIndexProfile, VisualIndexWindow, VisualSample,
    VisualWindow, VisualWindowOutcome, analyse_window, visual_window_count,
};
use vsift_infrastructure::{
    ExecutableResolver, FilesystemSessionStore, encode_visual_index_record,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

/// 2027-01-15T08:00:00Z: an arbitrary fixed start for deterministic expiry.
const T0: u64 = 1_800_000_000;
const HOUR: u64 = 3_600;
const OWNED_PREFIX: &str = "vsift-engine-candidates-test-";
const SECOND: u64 = 1_000_000;
/// The warm p95 page target of the verification plan's reference profile.
const P95_TARGET: Duration = Duration::from_millis(250);

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
struct ControlledClock(Arc<AtomicU64>);

impl ControlledClock {
    fn set(&self, seconds: u64) {
        self.0.store(seconds, Ordering::SeqCst);
    }
}

impl Clock for ControlledClock {
    fn now_unix_seconds(&self) -> Result<u64, ClockError> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}

#[derive(Clone)]
struct SequentialIdentifiers(Arc<AtomicU64>);

impl IdentifierSource for SequentialIdentifiers {
    fn session_id(&self) -> Result<SessionId, IdentifierGenerationError> {
        SessionId::parse(format!(
            "ses_{:032x}",
            self.0.fetch_add(1, Ordering::SeqCst) + 1
        ))
        .map_err(|_| IdentifierGenerationError::NonCanonical)
    }

    fn operation_id(&self) -> Result<OperationId, IdentifierGenerationError> {
        OperationId::parse(format!(
            "op_{:032x}",
            self.0.fetch_add(1, Ordering::SeqCst) + 1
        ))
        .map_err(|_| IdentifierGenerationError::NonCanonical)
    }
}

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Built<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self, child: &str) -> PathBuf {
        self.0.join(child)
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// What a seeded window holds.
#[derive(Clone, Copy)]
enum Seeded {
    /// Analysed from a screen that changes every 20 s.
    Analysed,
    /// Recorded as undecodable.
    Undecodable,
}

struct Harness {
    root: OwnedRoot,
    clock: ControlledClock,
    engine: Engine,
    session: SessionId,
    source: SourceId,
}

impl Harness {
    /// An engine over a private root holding one open session over a
    /// stand-in source (or `source`), with no visual index.
    async fn open(source: Option<PathBuf>) -> Built<Self> {
        let root = OwnedRoot::new()?;
        let clock = ControlledClock(Arc::new(AtomicU64::new(T0)));
        let engine = Engine::new(
            EngineConfig {
                session_root: SessionRootLocation::Explicit(root.path("sessions")),
                user_configuration: UserConfigurationLocation::Explicit(root.path("config")),
                host_isolation: HostIsolation::ProcessOnly,
            },
            EnginePorts::new(
                clock.clone(),
                SequentialIdentifiers(Arc::new(AtomicU64::new(0))),
            ),
        );
        let source = if let Some(source) = source {
            source
        } else {
            let stand_in = root.path("stand-in.mp4");
            fs::write(&stand_in, b"\0\0\0\x18ftypisomengine-candidates")?;
            stand_in
        };
        let opened = engine
            .ingest(IngestRequest {
                source,
                transcript: None,
            })
            .await?;
        Ok(Self {
            root,
            clock,
            engine,
            session: opened.session.session_id,
            source: opened.session.source_id,
        })
    }

    /// Commits a visual index of a `duration_seconds` source holding `windows`.
    fn seed(&self, duration_seconds: u64, windows: &[(u32, Seeded)]) -> Built<VisualIndex> {
        let store = FilesystemSessionStore::open_existing(self.root.path("sessions"))?;
        let scope = VisualIndexScope {
            session_id: &self.session,
            source_id: &self.source,
            stream_index: 0,
            displayed_dimensions: FrameDimensions::new(1440, 900)?,
            duration: MediaTime::from_micros(duration_seconds * SECOND),
            profile: VisualIndexProfile::R0,
        };
        let mut recorded = Vec::new();
        for (ordinal, seeded) in windows {
            recorded.push(seeded_window(&scope, *ordinal, *seeded)?);
        }
        let status = store.session_status(&self.session)?;
        let number = NonZeroU32::MIN;
        let index = VisualIndex::new(VisualIndexParts {
            id: visual_index_id(&scope, number)?,
            number,
            source_id: self.source.clone(),
            stream_index: 0,
            displayed_dimensions: scope.displayed_dimensions,
            duration: scope.duration,
            profile: VisualIndexProfile::R0,
            windows: recorded,
        })?;
        store.publish_artifact(
            &self.session,
            &OperationId::parse("op_7777777777777777")?,
            status.generation(),
            SessionArtifactKind::VisualIndexRecord,
            &encode_visual_index_record(&self.session, &index)?,
            T0,
        )?;
        Ok(index)
    }

    fn request(&self, from_seconds: u64, to_seconds: u64) -> CandidatesRequest {
        CandidatesRequest {
            session: self.session.clone(),
            range: CandidatesRange {
                from_micros: from_seconds * SECOND,
                to_micros: to_seconds * SECOND,
            },
            limit: None,
            cursor: None,
            cancellation: Cancellation::new(),
        }
    }
}

/// A screen that changes every 20 s of its window (or every second when
/// `busy`), sampled every 0.5 s from the window's lead-in.
fn samples(window: VisualWindow, busy: bool) -> Vec<VisualSample> {
    let mut samples = Vec::new();
    let mut time = window.lead_in_start().as_micros();
    while time < window.range().end().as_micros() {
        let level = if busy {
            u8::try_from(40 + (time / SECOND % 2) * 60 + (time / SECOND % 60)).unwrap_or(0)
        } else {
            u8::try_from(40 + ((time % (60 * SECOND)) / (20 * SECOND)) * 50).unwrap_or(0)
        };
        samples.push(VisualSample::from_parts(
            MediaTime::from_micros(time),
            [level; VISUAL_BLOCKS],
            VisualHash::from_bits(u64::from(level)),
        ));
        time += SECOND / 2;
    }
    samples
}

fn analysed_window(
    scope: &VisualIndexScope<'_>,
    ordinal: u32,
    busy: bool,
) -> Built<VisualIndexWindow> {
    let window = VisualWindow::new(ordinal, scope.duration)?;
    let analysis = analyse_window(window, &samples(window, busy), VisualChangePolicy::R0)?;
    let mut candidates = Vec::new();
    for draft in analysis.candidates {
        candidates.push(VisualCandidate::new(
            visual_candidate_id(scope, ordinal, draft.representative)?,
            draft,
        ));
    }
    Ok(VisualIndexWindow::new(
        window,
        VisualWindowOutcome::Analysed {
            sample_count: analysis.sample_count,
            candidates,
            dropped_candidates: analysis.dropped_candidates,
            frameless_cells: analysis.frameless_cells,
        },
        VisualChangePolicy::R0,
    )?)
}

fn seeded_window(
    scope: &VisualIndexScope<'_>,
    ordinal: u32,
    seeded: Seeded,
) -> Built<VisualIndexWindow> {
    match seeded {
        Seeded::Analysed => analysed_window(scope, ordinal, false),
        Seeded::Undecodable => Ok(VisualIndexWindow::new(
            VisualWindow::new(ordinal, scope.duration)?,
            VisualWindowOutcome::Undecodable,
            VisualChangePolicy::R0,
        )?),
    }
}

/// An indexed range is read warm: nothing analysed, the candidates of the
/// committed index in time order, and each typed gap reported.
#[tokio::test]
async fn an_indexed_range_is_read_warm_with_typed_gaps() -> TestResult {
    let harness = Harness::open(None).await?;
    let index = harness.seed(150, &[(0, Seeded::Analysed), (1, Seeded::Undecodable)])?;
    let complete = harness.engine.candidates(harness.request(0, 60)).await?;
    assert_eq!(complete.analysed_now(), 0);
    assert_eq!(complete.index().id(), index.id());
    assert!(complete.gaps().is_empty());
    let expected: Vec<&str> = index
        .windows()
        .iter()
        .flat_map(VisualIndexWindow::candidates)
        .map(|candidate| candidate.id().as_str())
        .collect();
    let returned: Vec<&str> = complete
        .candidates()
        .iter()
        .map(|candidate| candidate.id().as_str())
        .collect();
    assert_eq!(returned, expected);
    assert_eq!(complete.analysed(), [complete.searched()]);

    let with_gap = harness.engine.candidates(harness.request(30, 120)).await?;
    assert_eq!(with_gap.gaps().len(), 1);
    assert_eq!(
        with_gap.gaps().first().map(|gap| gap.reason),
        Some(CoverageGapReason::Undecodable)
    );
    // A range past the end of the video is clipped to it.
    let clipped = harness.engine.candidates(harness.request(0, 100)).await?;
    assert_eq!(clipped.searched().end().as_micros(), 100 * SECOND);
    drop(harness.root);
    Ok(())
}

/// Bad requests fail before anything is read or run; a cursor for a
/// session without an index cannot page anything.
#[tokio::test]
async fn bad_ranges_limits_cursors_and_sessions_are_typed() -> TestResult {
    let harness = Harness::open(None).await?;
    let mut cursor = harness.request(0, 60);
    cursor.cursor = Some("v1.anything".to_owned());
    let error = harness.engine.candidates(cursor.clone()).await.err();
    assert_eq!(error, Some(EngineError::CandidateCursorWithoutIndex));
    assert_eq!(
        error.map(|error| error.failure_code()),
        Some(FailureCode::InvalidArgument)
    );
    harness.seed(150, &[(0, Seeded::Analysed), (1, Seeded::Analysed)])?;
    for (from, to) in [(5, 5), (10, 5)] {
        assert_eq!(
            harness
                .engine
                .candidates(harness.request(from, to))
                .await
                .err(),
            Some(EngineError::InvalidTimeRange)
        );
    }
    assert_eq!(
        harness
            .engine
            .candidates(harness.request(150, 160))
            .await
            .err(),
        Some(EngineError::RangeOutsideSource)
    );
    for limit in [0, 101] {
        let mut request = harness.request(0, 60);
        request.limit = Some(limit);
        assert_eq!(
            harness.engine.candidates(request).await.err(),
            Some(EngineError::InvalidPageLimit)
        );
    }
    assert!(matches!(
        harness.engine.candidates(cursor).await.err(),
        Some(EngineError::CandidateQuery(CandidateQueryError::Cursor(_)))
    ));
    harness.engine.close_session(&harness.session)?;
    assert_eq!(
        harness
            .engine
            .candidates(harness.request(0, 60))
            .await
            .err()
            .map(|error| error.failure_code()),
        Some(FailureCode::InvalidArgument)
    );
    Ok(())
}

/// A cursor survives a renewal and expires with the session expiry it was
/// issued under, as transcript and search cursors do.
#[tokio::test]
async fn cursors_expire_with_the_session_expiry_they_were_issued_under() -> TestResult {
    let harness = Harness::open(None).await?;
    harness.seed(60, &[(0, Seeded::Analysed)])?;
    let mut first = harness.request(0, 60);
    first.limit = Some(1);
    let cursor = harness
        .engine
        .candidates(first.clone())
        .await?
        .next_cursor()
        .ok_or("no cursor")?
        .to_owned();
    harness.clock.set(T0 + 23 * HOUR);
    harness.engine.renew_session(&harness.session)?;
    let mut continued = first;
    continued.cursor = Some(cursor);
    assert_eq!(
        harness
            .engine
            .candidates(continued.clone())
            .await?
            .candidates()
            .len(),
        1
    );
    harness.clock.set(T0 + 25 * HOUR);
    assert_eq!(
        harness.engine.candidates(continued).await.err(),
        Some(EngineError::CandidateQuery(CandidateQueryError::Cursor(
            CursorError::Expired
        )))
    );
    Ok(())
}

fn fixture(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/corpus/generated")
        .join(relative)
}

fn media_tools_on_path() -> bool {
    let resolver = ExecutableResolver::from_current_path();
    resolver.resolve(OsStr::new("ffmpeg")).is_ok()
        && resolver.resolve(OsStr::new("ffprobe")).is_ok()
}

/// Two concurrent calls over the same unindexed window both analyse it;
/// the one that commits second finds a newer revision, re-reads it, and
/// merges nothing, so the window is committed once and both calls page the
/// same candidates.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn concurrent_calls_over_one_window_commit_it_once() -> TestResult {
    if !media_tools_on_path() {
        return Err("FFmpeg and FFprobe must be on PATH".into());
    }
    let harness = Harness::open(Some(fixture("F05.mp4"))).await?;
    // The first call verifies the tools, so the two calls below race on
    // analysis and commit rather than on the first preflight.
    harness.engine.candidates(harness.request(0, 1)).await?;
    let second = harness
        .engine
        .ingest(IngestRequest {
            source: fixture("F05.mp4"),
            transcript: None,
        })
        .await?
        .session
        .session_id;
    let request = || {
        let mut request = harness.request(0, 20);
        request.session = second.clone();
        request
    };
    let (left, right) = tokio::join!(
        harness.engine.candidates(request()),
        harness.engine.candidates(request()),
    );
    let (left, right) = (left?, right?);
    assert_eq!(left.index().windows(), right.index().windows());
    assert_eq!(left.candidates(), right.candidates());
    assert_eq!(left.index().number().get(), 1);
    assert_eq!(right.index().number().get(), 1);
    let warm = harness.engine.candidates(request()).await?;
    assert_eq!(warm.analysed_now(), 0);
    assert_eq!(warm.index().number().get(), 1);
    Ok(())
}

/// The p95 of `durations`, by nearest rank.
fn p95(durations: &[Duration]) -> Duration {
    let mut sorted = durations.to_vec();
    sorted.sort_unstable();
    let rank = (sorted.len() * 95).div_ceil(100).saturating_sub(1);
    sorted.get(rank).copied().unwrap_or_default()
}

/// S-11: an index of the four-hour source bound whose every window is at
/// its 32-candidate budget (the largest record a session reads) pages
/// completely at limits 20 and 100 without a gap or a duplicate; each warm
/// page reads and verifies the newest record only.
#[tokio::test]
#[ignore = "opt-in S-11 measurement; run with --release --ignored --nocapture"]
#[allow(
    clippy::too_many_lines,
    reason = "One measurement: seed, page at two sizes, report"
)]
async fn s11_a_four_hour_index_pages_warm() -> TestResult {
    let harness = Harness::open(None).await?;
    let duration = 4 * 60 * 60;
    let store = FilesystemSessionStore::open_existing(harness.root.path("sessions"))?;
    let scope = VisualIndexScope {
        session_id: &harness.session,
        source_id: &harness.source,
        stream_index: 0,
        displayed_dimensions: FrameDimensions::new(1920, 1080)?,
        duration: MediaTime::from_micros(duration * SECOND),
        profile: VisualIndexProfile::R0,
    };
    let mut windows = Vec::new();
    for ordinal in 0..visual_window_count(scope.duration) {
        windows.push(analysed_window(&scope, ordinal, true)?);
    }
    let number = NonZeroU32::MIN;
    let index = VisualIndex::new(VisualIndexParts {
        id: visual_index_id(&scope, number)?,
        number,
        source_id: harness.source.clone(),
        stream_index: 0,
        displayed_dimensions: scope.displayed_dimensions,
        duration: scope.duration,
        profile: VisualIndexProfile::R0,
        windows,
    })?;
    let record = encode_visual_index_record(&harness.session, &index)?;
    let status = store.session_status(&harness.session)?;
    store.publish_artifact(
        &harness.session,
        &OperationId::parse("op_8888888888888888")?,
        status.generation(),
        SessionArtifactKind::VisualIndexRecord,
        &record,
        T0,
    )?;
    let expected: Vec<String> = index
        .windows()
        .iter()
        .flat_map(VisualIndexWindow::candidates)
        .map(|candidate| candidate.id().as_str().to_owned())
        .collect();
    println!(
        "S-11 candidates: {} windows, {} candidates, record {} bytes",
        index.windows().len(),
        expected.len(),
        record.len()
    );
    for limit in [20_u16, 100] {
        let mut request = harness.request(0, duration);
        request.limit = Some(limit);
        let mut seen = Vec::new();
        let mut durations = Vec::new();
        loop {
            let started = Instant::now();
            let results = harness.engine.candidates(request.clone()).await?;
            durations.push(started.elapsed());
            assert_eq!(results.analysed_now(), 0);
            seen.extend(
                results
                    .candidates()
                    .iter()
                    .map(|candidate| candidate.id().as_str().to_owned()),
            );
            match results.next_cursor() {
                Some(cursor) => request.cursor = Some(cursor.to_owned()),
                None => break,
            }
            // Bound the measurement: 60 pages suffice for a p95.
            if durations.len() == 60 {
                break;
            }
        }
        let distinct: BTreeSet<&String> = seen.iter().collect();
        assert_eq!(distinct.len(), seen.len(), "limit {limit}");
        assert_eq!(
            expected.get(..seen.len()),
            Some(seen.as_slice()),
            "limit {limit}"
        );
        let p95 = p95(&durations);
        println!(
            "S-11 candidates, four-hour index, limit {limit}: {} pages, p95 {} ms, max {} ms ({})",
            durations.len(),
            p95.as_millis(),
            durations
                .iter()
                .max()
                .copied()
                .unwrap_or_default()
                .as_millis(),
            if cfg!(debug_assertions) {
                "unoptimised build, target not asserted"
            } else {
                "optimised build"
            }
        );
        if !cfg!(debug_assertions) {
            assert!(
                p95 <= P95_TARGET,
                "limit {limit}: p95 page time {} ms exceeds {} ms",
                p95.as_millis(),
                P95_TARGET.as_millis()
            );
        }
    }
    Ok(())
}
