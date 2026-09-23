//! Supplied-transcript import and bounded retrieval through the engine library.
//!
//! Every test injects a controlled clock and sequential identifiers. Failures
//! that must happen before any work run everywhere. The import itself probes
//! the staged source with the real `FFprobe`, so the tests that import F10 are
//! `#[ignore]`d and opt-in:
//!
//! `cargo test -p vsift --locked --test engine_transcript -- --ignored`

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use vsift::{
    Clock, ClockError, CueMarkup, Engine, EngineConfig, EngineError, EnginePorts, FailureCode,
    HostIsolation, IdentifierGenerationError, IdentifierSource, IngestRequest, OperationId,
    RuntimeDependency, SessionId, SessionListEntry, SessionRootError, SessionRootLocation,
    SuppliedTranscriptRequest, TranscriptImportError, TranscriptQuery, TranscriptRejection,
    UserConfigurationLocation,
};

type TestResult = Result<(), Box<dyn Error>>;

/// 2027-01-15T08:00:00Z: an arbitrary fixed start for deterministic expiry.
const T0: u64 = 1_800_000_000;
const OWNED_PREFIX: &str = "vsift-engine-transcript-test-";
const PLACEHOLDER_SOURCE: &[u8] = b"\0\0\0\x18ftypisomengine-transcript-source";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
struct ControlledClock(Arc<AtomicU64>);

impl ControlledClock {
    fn at(seconds: u64) -> Self {
        Self(Arc::new(AtomicU64::new(seconds)))
    }

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

impl SequentialIdentifiers {
    fn new() -> Self {
        Self(Arc::new(AtomicU64::new(0)))
    }

    fn issued(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }

    fn next(&self) -> u64 {
        self.0.fetch_add(1, Ordering::SeqCst) + 1
    }
}

impl IdentifierSource for SequentialIdentifiers {
    fn session_id(&self) -> Result<SessionId, IdentifierGenerationError> {
        SessionId::parse(format!("ses_{:032x}", self.next()))
            .map_err(|_| IdentifierGenerationError::NonCanonical)
    }

    fn operation_id(&self) -> Result<OperationId, IdentifierGenerationError> {
        OperationId::parse(format!("op_{:032x}", self.next()))
            .map_err(|_| IdentifierGenerationError::NonCanonical)
    }
}

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Result<Self, Box<dyn Error>> {
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

struct Harness {
    root: OwnedRoot,
    clock: ControlledClock,
    identifiers: SequentialIdentifiers,
    engine: Engine,
}

impl Harness {
    fn new() -> Result<Self, Box<dyn Error>> {
        let root = OwnedRoot::new()?;
        let clock = ControlledClock::at(T0);
        let identifiers = SequentialIdentifiers::new();
        let engine = Engine::new(
            EngineConfig {
                session_root: SessionRootLocation::Explicit(root.path("sessions")),
                user_configuration: UserConfigurationLocation::Explicit(root.path("config")),
                host_isolation: HostIsolation::ProcessOnly,
            },
            EnginePorts::new(clock.clone(), identifiers.clone()),
        );
        Ok(Self {
            root,
            clock,
            identifiers,
            engine,
        })
    }

    fn placeholder_source(&self) -> Result<PathBuf, Box<dyn Error>> {
        let source = self.root.path("placeholder.mp4");
        fs::write(&source, PLACEHOLDER_SOURCE)?;
        Ok(source)
    }

    fn sidecar(&self, name: &str, content: &[u8]) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.root.path(name);
        fs::write(&path, content)?;
        Ok(path)
    }

    async fn ingest(
        &self,
        source: PathBuf,
        sidecar: PathBuf,
        offset_micros: i64,
    ) -> Result<vsift::IngestOutcome, EngineError> {
        self.engine
            .ingest(IngestRequest {
                source,
                transcript: Some(SuppliedTranscriptRequest {
                    path: sidecar,
                    offset_micros,
                }),
            })
            .await
    }
}

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let file_name = format!("{name}{}", env::consts::EXE_SUFFIX);
    env::split_paths(&env::var_os("PATH")?)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join(&file_name))
        .find(|candidate| Path::is_file(candidate))
}

/// T-01/T-02: every transcript defect that can be found without the source is
/// reported before the session root, identifiers or tools are touched.
#[tokio::test]
async fn transcript_defects_are_rejected_before_any_work() -> TestResult {
    let harness = Harness::new()?;
    let source = harness.placeholder_source()?;
    let cases = [
        (
            harness.sidecar("bad-time.srt", b"1\n00:00:01,000 --> 00:61:00,000\nx\n")?,
            0,
            EngineError::TranscriptRejected(TranscriptImportError::at_line(
                TranscriptRejection::InvalidTimestamp,
                2,
            )),
            FailureCode::InvalidSource,
        ),
        (
            harness.sidecar("untimed.srt", b"Just prose without timing.\n")?,
            0,
            EngineError::TranscriptRejected(TranscriptImportError::at_line(
                TranscriptRejection::UntimedText,
                1,
            )),
            FailureCode::InvalidSource,
        ),
        (
            harness.sidecar("valid.vtt", b"WEBVTT\n\n00:01.000 --> 00:02.000\nx\n")?,
            86_400_000_001,
            EngineError::TranscriptRejected(TranscriptImportError::new(
                TranscriptRejection::OffsetOutOfRange,
            )),
            FailureCode::InvalidArgument,
        ),
        (
            harness.sidecar(
                "long-line.srt",
                format!("1\n00:00:01,000 --> 00:00:02,000\n{}\n", "a".repeat(5_000)).as_bytes(),
            )?,
            0,
            EngineError::TranscriptRejected(TranscriptImportError::at_line(
                TranscriptRejection::LineTooLong,
                3,
            )),
            FailureCode::ResourceLimit,
        ),
    ];
    for (sidecar, offset, error, code) in cases {
        let result = harness.ingest(source.clone(), sidecar, offset).await;
        assert_eq!(result.as_ref().map(|_| ()), Err(&error));
        assert_eq!(
            result.map_err(|error| error.failure_code()).map(|_| ()),
            Err(code)
        );
        assert!(error.transcript_rejection().is_some());
    }
    assert!(!harness.root.path("sessions").exists());
    assert_eq!(harness.identifiers.issued(), 0);
    Ok(())
}

/// A media tool that cannot run stops the import with a typed missing
/// capability, and the session it began is never opened.
#[tokio::test]
async fn an_unusable_probe_leaves_no_open_session() -> TestResult {
    let harness = Harness::new()?;
    let not_a_program = harness.sidecar("not-a-program.exe", b"plain text, not a program")?;
    harness
        .engine
        .configure_executable(RuntimeDependency::Ffmpeg, &not_a_program)?;
    harness
        .engine
        .configure_executable(RuntimeDependency::Ffprobe, &not_a_program)?;
    let sidecar = harness.sidecar("valid.srt", b"1\n00:00:00,500 --> 00:00:01,000\nx\n")?;

    let result = harness
        .ingest(harness.placeholder_source()?, sidecar, 0)
        .await;

    assert_eq!(
        result.map_err(|error| error.failure_code()).map(|_| ()),
        Err(FailureCode::MissingCapability)
    );
    let listed = harness.engine.list_sessions(None)?;
    assert!(
        listed
            .entries()
            .iter()
            .all(|entry| matches!(entry, SessionListEntry::Initializing(_))),
        "{listed:?}"
    );
    Ok(())
}

#[tokio::test]
async fn transcript_reads_are_typed_for_bad_ranges_and_sessions_without_one() -> TestResult {
    let harness = Harness::new()?;
    let query = |session: SessionId, from: u64, to: u64, limit: Option<u16>| TranscriptQuery {
        session,
        from_micros: from,
        to_micros: to,
        limit,
        cursor: None,
    };
    let unknown = SessionId::parse("ses_00000000000000000000000000000001")?;
    assert_eq!(
        harness
            .engine
            .transcript(query(unknown.clone(), 0, 1, None)),
        Err(EngineError::SessionRoot(SessionRootError::Missing))
    );
    assert_eq!(
        harness
            .engine
            .transcript(query(unknown.clone(), 5, 5, None)),
        Err(EngineError::InvalidTimeRange)
    );
    assert_eq!(
        harness
            .engine
            .transcript(query(unknown.clone(), 0, 1, Some(101))),
        Err(EngineError::InvalidPageLimit)
    );

    let opened = harness
        .engine
        .ingest(IngestRequest {
            source: harness.placeholder_source()?,
            transcript: None,
        })
        .await?;
    assert!(opened.transcript.is_none());
    let without =
        harness
            .engine
            .transcript(query(opened.session.session_id.clone(), 0, 1_000_000, None));
    assert_eq!(without, Err(EngineError::TranscriptUnavailable));
    assert_eq!(
        without.map_err(|error| error.failure_code()).map(|_| ()),
        Err(FailureCode::InvalidArgument)
    );
    Ok(())
}

/// Opt-in: the A-09 supplied-transcript path on F10 through the library.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the import, paging, citation and cursor-lifetime journey in one place"
)]
async fn f10_import_pages_and_cites_the_dialog_window() -> TestResult {
    let harness = Harness::new()?;
    let ffmpeg = find_on_path("ffmpeg").ok_or("ffmpeg is not on PATH")?;
    let ffprobe = find_on_path("ffprobe").ok_or("ffprobe is not on PATH")?;
    harness
        .engine
        .configure_executable(RuntimeDependency::Ffmpeg, &ffmpeg)?;
    harness
        .engine
        .configure_executable(RuntimeDependency::Ffprobe, &ffprobe)?;

    let opened = harness
        .ingest(
            repository("fixtures/corpus/generated/F10.mp4"),
            repository("fixtures/corpus/transcripts/F10.srt"),
            500_000,
        )
        .await?;
    // One session identity, then initialize, stage and activate operations.
    assert_eq!(harness.identifiers.issued(), 4);
    let revision = opened.transcript.ok_or("no transcript revision")?;
    assert_eq!(revision.number(), 1);
    assert_eq!(revision.offset().as_micros(), 500_000);
    assert_eq!(
        revision.source_segment().range().end().as_micros(),
        12_000_000
    );
    assert!(revision.warnings().as_slice().is_empty());
    let session = opened.session.session_id;
    let status = harness.engine.session_status(&session)?;
    assert_eq!(status.artifact_count(), 1);
    assert_eq!(status.generation(), opened.session.generation);

    // Page one segment at a time across the whole video.
    let mut cursor = None;
    let mut seen = Vec::new();
    loop {
        let page = harness.engine.transcript(TranscriptQuery {
            session: session.clone(),
            from_micros: 0,
            to_micros: 12_000_000,
            limit: Some(1),
            cursor: cursor.take(),
        })?;
        seen.extend(page.segments().iter().map(|segment| {
            (
                segment.range().start().as_micros(),
                segment.range().end().as_micros(),
                segment.text().text().to_owned(),
            )
        }));
        match page.next_cursor() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }
    assert_eq!(seen.len(), 3);
    assert_eq!(
        seen[1],
        (
            5_000_000,
            9_000_000,
            "Dialog R-17 is displayed now.".to_owned()
        )
    );

    // The truth window of F10-E01 returns exactly the dialog cue.
    let cited = harness.engine.transcript(TranscriptQuery {
        session: session.clone(),
        from_micros: 5_000_000,
        to_micros: 9_000_000,
        limit: None,
        cursor: None,
    })?;
    assert_eq!(cited.segments().len(), 1);
    assert_eq!(cited.segments()[0].text().markup(), CueMarkup::None);
    assert_eq!(cited.segments()[0].cue().ordinal(), 2);

    // A cursor survives renewal (it is bound to the revision, not the
    // storage generation) and is refused once the session has expired.
    let first = harness.engine.transcript(TranscriptQuery {
        session: session.clone(),
        from_micros: 0,
        to_micros: 12_000_000,
        limit: Some(1),
        cursor: None,
    })?;
    let continuation = first.next_cursor().ok_or("expected a cursor")?.to_owned();
    harness.clock.set(T0 + 60);
    harness.engine.renew_session(&session)?;
    let resumed = harness.engine.transcript(TranscriptQuery {
        session: session.clone(),
        from_micros: 0,
        to_micros: 12_000_000,
        limit: Some(1),
        cursor: Some(continuation),
    })?;
    assert_eq!(resumed.segments()[0].cue().ordinal(), 2);
    let expiry = harness
        .engine
        .session_status(&session)?
        .lifetime()
        .expires_at_unix_seconds();
    harness.clock.set(expiry);
    assert_eq!(
        harness
            .engine
            .transcript(TranscriptQuery {
                session,
                from_micros: 0,
                to_micros: 12_000_000,
                limit: None,
                cursor: None,
            })
            .map_err(|error| error.failure_code())
            .map(|_| ()),
        Err(FailureCode::InvalidArgument)
    );
    Ok(())
}

/// Opt-in T-02: a wrong offset is rejected after probing and no session opens.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn f10_with_a_wrong_offset_opens_no_session() -> TestResult {
    let harness = Harness::new()?;
    let result = harness
        .ingest(
            repository("fixtures/corpus/generated/F10.mp4"),
            repository("fixtures/corpus/transcripts/F10.vtt"),
            -60_000_000,
        )
        .await;
    assert_eq!(
        result.as_ref().map(|_| ()),
        Err(&EngineError::OpenSession(
            vsift::OpenSessionError::TranscriptRejected(TranscriptImportError::new(
                TranscriptRejection::NoCuesWithinSource
            ))
        ))
    );
    assert_eq!(
        result.map_err(|error| error.failure_code()).map(|_| ()),
        Err(FailureCode::InvalidArgument)
    );
    let listed = harness.engine.list_sessions(None)?;
    assert!(
        listed
            .entries()
            .iter()
            .all(|entry| matches!(entry, SessionListEntry::Initializing(_)))
    );
    Ok(())
}
