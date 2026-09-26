//! Transcript search through the engine library (P08, ADR 0018).
//!
//! Sessions are committed directly through the session store with a
//! controlled clock, so every test runs everywhere without `FFprobe`. The
//! S-11 measurement pages a 20,000-segment revision (the import bound)
//! completely and records the time of each page, including reading and
//! verifying the stored record, beside the time of a `transcript get` page of
//! the same record. An unoptimised build reads the record too slowly for the
//! default suite, so it is opt-in, and it asserts the 250 ms p95 page target
//! only in an optimised build (the application crate's always-on S-11 test
//! checks the same paging in memory):
//!
//! `cargo test --release -p vsift --locked --test engine_search -- --ignored --nocapture`

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
    Clock, ClockError, CoverageBasis, CursorError, DurabilityRequirement, Engine, EngineConfig,
    EngineError, EnginePorts, FailureCode, HostIsolation, IdentifierGenerationError,
    IdentifierSource, MediaTime, OperationId, SearchMatch, SearchQueryRejection, SearchRange,
    SearchRequest, SessionId, SessionRootLocation, StorageGeneration, TranscriptOffset,
    TranscriptQuery, TranscriptQueryError, TranscriptRevision, UserConfigurationLocation,
};
use vsift_application::{
    ForegroundSessionPort, ImportedRevisionRequest, InitializeSessionStorage,
    InitializeSessionStorageRequest, SuppliedTranscript, build_imported_revision,
};
use vsift_domain::{
    CueSource, CueText, CueTiming, ImportedCue, ParsedTranscript, SidecarIdentity, SourceId,
    TranscriptFormat, TranscriptWarnings,
};
use vsift_infrastructure::{FilesystemSessionStore, SourceSnapshot};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

/// 2027-01-15T08:00:00Z: an arbitrary fixed start for deterministic expiry.
const T0: u64 = 1_800_000_000;
const HOUR: u64 = 3_600;
const OWNED_PREFIX: &str = "vsift-engine-search-test-";
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
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

/// An engine over a private root holding one open session whose transcript
/// is an import of `texts`, cue `n` (from 1) at `n` seconds for 0.9 s.
struct Harness {
    root: OwnedRoot,
    clock: ControlledClock,
    engine: Engine,
    session: SessionId,
}

impl Harness {
    async fn with_transcript(texts: &[String]) -> Built<Self> {
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
        let session = seed(&root, texts).await?;
        Ok(Self {
            root,
            clock,
            engine,
            session,
        })
    }

    fn search(&self, query: &str) -> SearchRequest {
        SearchRequest {
            session: self.session.clone(),
            query: query.to_owned(),
            revision: None,
            range: None,
            limit: None,
            cursor: None,
        }
    }
}

fn revision_of(
    session_id: &SessionId,
    source_id: &SourceId,
    texts: &[String],
) -> Built<TranscriptRevision> {
    let mut cues = Vec::with_capacity(texts.len());
    for (ordinal, text) in (1_u32..).zip(texts) {
        let start = u64::from(ordinal) * SECOND;
        cues.push(ImportedCue {
            source: CueSource::new(
                NonZeroU32::new(ordinal).ok_or("zero")?,
                NonZeroU32::new(ordinal.saturating_mul(4)).ok_or("zero")?,
            ),
            timing: CueTiming::new(start, start + 900_000)?,
            text: CueText::new(text.clone(), text.clone())?,
            speaker: None,
        });
    }
    let supplied = SuppliedTranscript {
        transcript: ParsedTranscript::new(
            TranscriptFormat::Srt,
            None,
            cues,
            TranscriptWarnings::default(),
        )?,
        sidecar: SidecarIdentity::new(DIGEST, 1)?,
    };
    Ok(build_imported_revision(ImportedRevisionRequest {
        session_id,
        source_id,
        source_duration: MediaTime::from_micros((u64::try_from(texts.len())? + 2) * SECOND),
        supplied: &supplied,
        offset: TranscriptOffset::ZERO,
        number: NonZeroU32::MIN,
    })?)
}

/// Commits one open session holding the import of `texts` at `T0`.
async fn seed(root: &OwnedRoot, texts: &[String]) -> Built<SessionId> {
    let source = root.path("stand-in.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomengine-search")?;
    let session_id = SessionId::parse("ses_0123456789abcdef0123456789abcdef")?;
    let opener = OperationId::parse("op_0123456789abcdef")?;
    let store = FilesystemSessionStore::provision_default(root.path("sessions"))?;
    let registration = store.register_session(&session_id, &opener, T0)?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            opener,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    drop(registration);
    let store = FilesystemSessionStore::open_existing(root.path("sessions"))?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_1111111111111111")?,
        &source,
    )?;
    let revision = revision_of(&session_id, snapshot.id(), texts)?;
    store.activate_with_transcript(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        T0,
        &revision,
    )?;
    Ok(session_id)
}

fn texts(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

/// A rejected query fails before anything is read: here there is no session
/// root at all, and the typed reason still wins.
#[tokio::test]
async fn queries_are_rejected_before_any_read() -> TestResult {
    let root = OwnedRoot::new()?;
    let engine = Engine::new(
        EngineConfig {
            session_root: SessionRootLocation::Explicit(root.path("absent")),
            user_configuration: UserConfigurationLocation::Explicit(root.path("config")),
            host_isolation: HostIsolation::ProcessOnly,
        },
        EnginePorts::new(
            ControlledClock(Arc::new(AtomicU64::new(T0))),
            SequentialIdentifiers(Arc::new(AtomicU64::new(0))),
        ),
    );
    for (query, rejection) in [
        (String::new(), SearchQueryRejection::Empty),
        ("x".repeat(257), SearchQueryRejection::TooLong),
        (vec!["w"; 17].join(" "), SearchQueryRejection::TooManyTerms),
        (
            "a\u{1b}b".to_owned(),
            SearchQueryRejection::ControlCharacter,
        ),
    ] {
        let error = engine
            .search(SearchRequest {
                session: SessionId::parse("ses_0123456789abcdef")?,
                query,
                revision: None,
                range: None,
                limit: None,
                cursor: None,
            })
            .err()
            .ok_or("a rejected query was accepted")?;
        assert_eq!(error, EngineError::SearchQueryRejected(rejection));
        assert_eq!(error.failure_code(), FailureCode::InvalidArgument);
    }
    assert!(
        !root.path("absent").exists(),
        "a rejected search created state"
    );
    Ok(())
}

#[tokio::test]
async fn search_ranks_restricts_and_states_coverage() -> TestResult {
    let harness = Harness::with_transcript(&texts(&[
        "Dialog R-17 is displayed now.",
        "17 and r are here",
        "nothing",
        "dialog R17 again",
    ]))
    .await?;
    let results = harness.engine.search(harness.search("r 17"))?;
    let ranked: Vec<(SearchMatch, u32)> = results
        .hits()
        .iter()
        .map(|hit| (hit.tier(), hit.segment().ordinal()))
        .collect();
    assert_eq!(
        ranked,
        [
            (SearchMatch::Phrase, 1),
            (SearchMatch::Phrase, 4),
            (SearchMatch::AllTerms, 2)
        ]
    );
    assert_eq!(results.query().terms(), ["r", "17"]);
    assert_eq!(
        results.coverage().basis(),
        CoverageBasis::SuppliedTranscript
    );
    assert!(results.coverage().is_complete());
    assert!(results.next_cursor().is_none());

    let mut restricted = harness.search("r 17");
    restricted.range = Some(SearchRange {
        from_micros: 2 * SECOND,
        to_micros: 4 * SECOND,
    });
    let results = harness.engine.search(restricted)?;
    assert_eq!(results.hits().len(), 1);
    assert_eq!(results.hits()[0].segment().ordinal(), 2);
    Ok(())
}

#[tokio::test]
async fn bad_ranges_limits_revisions_and_sessions_are_typed() -> TestResult {
    let harness = Harness::with_transcript(&texts(&["alpha", "beta"])).await?;
    let mut empty = harness.search("alpha");
    empty.range = Some(SearchRange {
        from_micros: 5,
        to_micros: 5,
    });
    assert_eq!(
        harness.engine.search(empty).err(),
        Some(EngineError::InvalidTimeRange)
    );
    for limit in [0, 101] {
        let mut request = harness.search("alpha");
        request.limit = Some(limit);
        assert_eq!(
            harness.engine.search(request).err(),
            Some(EngineError::InvalidPageLimit)
        );
    }
    let mut unknown = harness.search("alpha");
    unknown.revision = Some(vsift::TranscriptRevisionId::parse("trv_2222222222222222")?);
    assert_eq!(
        harness.engine.search(unknown).err(),
        Some(EngineError::TranscriptRevisionNotFound)
    );
    let mut other = harness.search("alpha");
    other.session = SessionId::parse("ses_ffffffffffffffffffffffffffffffff")?;
    assert!(harness.engine.search(other).is_err());

    harness.engine.close_session(&harness.session)?;
    let closed = harness
        .engine
        .search(harness.search("alpha"))
        .err()
        .ok_or("a closed session was searched")?;
    assert_eq!(closed.failure_code(), FailureCode::InvalidArgument);
    Ok(())
}

/// C-03: a cursor is bound to the session's expiry when it was issued; after
/// a renewal moves the expiry, an old cursor past its own expiry is rejected
/// rather than silently restarting.
#[tokio::test]
async fn cursors_expire_with_the_session_expiry_they_were_issued_under() -> TestResult {
    let harness = Harness::with_transcript(&texts(&["alpha", "alpha", "alpha"])).await?;
    let mut first = harness.search("alpha");
    first.limit = Some(1);
    let cursor = harness
        .engine
        .search(first.clone())?
        .next_cursor()
        .ok_or("no cursor")?
        .to_owned();
    harness.clock.set(T0 + 23 * HOUR);
    harness.engine.renew_session(&harness.session)?;
    let mut continued = first;
    continued.cursor = Some(cursor.clone());
    assert_eq!(harness.engine.search(continued.clone())?.hits().len(), 1);
    harness.clock.set(T0 + 25 * HOUR);
    assert_eq!(
        harness.engine.search(continued).err(),
        Some(EngineError::TranscriptQuery(TranscriptQueryError::Cursor(
            CursorError::Expired
        )))
    );
    drop(harness.root);
    Ok(())
}

/// The p95 of `durations`, by nearest rank.
fn p95(durations: &[Duration]) -> Duration {
    let mut sorted = durations.to_vec();
    sorted.sort_unstable();
    let rank = (sorted.len() * 95).div_ceil(100).saturating_sub(1);
    sorted.get(rank).copied().unwrap_or_default()
}

/// S-11: a revision at the 20,000-segment import bound is paged completely at
/// limits 1, 20 and 100 with no gap or duplicate, and each page's time,
/// including reading and verifying the stored record, is recorded.
#[tokio::test]
#[ignore = "opt-in S-11 measurement; run with --release --ignored --nocapture"]
async fn s11_a_20000_segment_revision_pages_completely() -> TestResult {
    let texts: Vec<String> = (1_u32..=20_000)
        .map(|ordinal| {
            if ordinal % 100 == 0 {
                format!("Dialog R-17 marker {ordinal}")
            } else if ordinal % 250 == 0 {
                format!("17 then r reversed {ordinal}")
            } else {
                format!("segment {ordinal} of the long synthetic transcript")
            }
        })
        .collect();
    let harness = Harness::with_transcript(&texts).await?;
    let mut expected: Vec<(SearchMatch, u32)> = (1_u32..=20_000)
        .filter(|ordinal| ordinal % 100 == 0)
        .map(|ordinal| (SearchMatch::Phrase, ordinal))
        .collect();
    expected.extend(
        (1_u32..=20_000)
            .filter(|ordinal| ordinal % 250 == 0 && ordinal % 100 != 0)
            .map(|ordinal| (SearchMatch::AllTerms, ordinal)),
    );
    let mut reads = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        harness.engine.transcript(TranscriptQuery {
            session: harness.session.clone(),
            revision: None,
            from_micros: 0,
            to_micros: 30_000 * SECOND,
            limit: Some(100),
            cursor: None,
        })?;
        reads.push(started.elapsed());
    }
    println!(
        "S-11 baseline: transcript get page of the same record, p95 {} ms",
        p95(&reads).as_millis()
    );
    for limit in [1_u16, 20, 100] {
        let mut request = harness.search("r 17");
        request.limit = Some(limit);
        let mut seen = Vec::new();
        let mut durations = Vec::new();
        loop {
            let started = Instant::now();
            let results = harness.engine.search(request.clone())?;
            durations.push(started.elapsed());
            assert!(results.hits().len() <= usize::from(limit));
            seen.extend(
                results
                    .hits()
                    .iter()
                    .map(|hit| (hit.tier(), hit.segment().ordinal())),
            );
            match results.next_cursor() {
                Some(cursor) => request.cursor = Some(cursor.to_owned()),
                None => break,
            }
        }
        assert_eq!(seen, expected, "limit {limit}");
        let distinct: BTreeSet<u32> = seen.iter().map(|(_, ordinal)| *ordinal).collect();
        assert_eq!(distinct.len(), seen.len(), "limit {limit}");
        let p95 = p95(&durations);
        let slowest = durations.iter().max().copied().unwrap_or_default();
        println!(
            "S-11 search, 20000 segments, limit {limit}: {} pages, p95 {} ms, max {} ms ({})",
            durations.len(),
            p95.as_millis(),
            slowest.as_millis(),
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
