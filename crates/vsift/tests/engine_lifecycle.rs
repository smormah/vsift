//! The engine used as a library, without the CLI.
//!
//! Every test injects a controlled clock and a sequential identifier source and
//! works in its own temporary directory, so identities and expiry are exact.
//! The one test that needs real `FFmpeg`/`FFprobe` is `#[ignore]`d and opt-in:
//!
//! `cargo test -p vsift --locked --test engine_lifecycle -- --ignored`

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
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use vsift::{
    Cancellation, CleanDecision, CleanEntry, CleanMode, CleanRequest, CleanScope, Clock,
    ClockError, DependencyState, Engine, EngineConfig, EngineError, EnginePorts,
    ExecutableRejection, ExecutableSelections, FailureCode, HostIsolation,
    IdentifierGenerationError, IdentifierSource, IngestRequest, MediaToolSelection,
    MediaToolVerification, MediaToolVerificationRequest, ModelSelection, ModelVerification,
    OperationId, RuntimeDependency, RuntimeReadiness, SessionId, SessionLifetime, SessionListEntry,
    SessionPhase, SessionRootError, SessionRootLocation, SetupCheckRequest, SourceRetention,
    UserConfigurationLocation,
};

use vsift_contract::DependencyLookup;

type TestResult = Result<(), Box<dyn Error>>;

/// 2027-01-15T08:00:00Z: an arbitrary fixed start for deterministic expiry.
const T0: u64 = 1_800_000_000;
const OWNED_PREFIX: &str = "vsift-engine-test-";
const SOURCE_BYTES: &[u8] = b"\0\0\0\x18ftypisomengine-lifecycle-source";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

/// A clock the test moves explicitly.
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

/// A clock that is always unreadable.
struct BrokenClock;

impl Clock for BrokenClock {
    fn now_unix_seconds(&self) -> Result<u64, ClockError> {
        Err(ClockError::BeforeUnixEpoch)
    }
}

/// Issues `ses_<n>` and `op_<n>` from one shared counter, as 32 hex digits.
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

/// A temporary directory this test created and alone may remove.
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

    fn source(&self) -> Result<PathBuf, Box<dyn Error>> {
        let source = self.path("original media.mp4");
        fs::write(&source, SOURCE_BYTES)?;
        Ok(source)
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

    async fn open(&self) -> Result<SessionId, Box<dyn Error>> {
        let opened = self
            .engine
            .ingest(IngestRequest {
                source: self.root.source()?,
                transcript: None,
            })
            .await?;
        Ok(opened.session_id)
    }
}

fn session(number: u64) -> Result<SessionId, Box<dyn Error>> {
    Ok(SessionId::parse(format!("ses_{number:032x}"))?)
}

#[tokio::test]
async fn open_status_renew_close_and_clean_use_the_injected_clock_and_identifiers() -> TestResult {
    let harness = Harness::new()?;
    let engine = &harness.engine;

    let opened = engine
        .ingest(IngestRequest {
            source: harness.root.source()?,
            transcript: None,
        })
        .await?;
    // One session identity, then initialize, stage and activate operations.
    assert_eq!(opened.session_id, session(1)?);
    assert_eq!(harness.identifiers.issued(), 4);
    assert_eq!(opened.source_bytes, u64::try_from(SOURCE_BYTES.len())?);
    assert_eq!(opened.lifetime, SessionLifetime::open(T0)?);
    assert_eq!(
        opened.lifetime.expires_at_unix_seconds(),
        T0 + SessionLifetime::IDLE_SECONDS
    );

    harness.clock.set(T0 + 100);
    let status = engine.session_status(&opened.session_id)?;
    assert_eq!(status.session_id(), &opened.session_id);
    assert_eq!(status.source_id(), &opened.source_id);
    assert_eq!(status.phase(), SessionPhase::Open);
    assert_eq!(status.generation(), opened.generation);
    assert_eq!(status.observed_at_unix_seconds(), T0 + 100);
    assert_eq!(status.artifact_count(), 0);

    harness.clock.set(T0 + 3_600);
    let renewed = engine.renew_session(&opened.session_id)?;
    assert_eq!(harness.identifiers.issued(), 5);
    assert_eq!(
        renewed.lifetime().expires_at_unix_seconds(),
        T0 + 3_600 + SessionLifetime::IDLE_SECONDS
    );
    assert_eq!(renewed.lifetime().opened_at_unix_seconds(), T0);
    assert!(renewed.generation().value() > status.generation().value());

    let listed = engine.list_sessions(None)?;
    assert!(!listed.is_partial());
    assert!(matches!(
        listed.entries(),
        [SessionListEntry::Indexed(snapshot)] if snapshot == &renewed
    ));

    let closed = engine.close_session(&opened.session_id)?;
    assert_eq!(harness.identifiers.issued(), 6);
    assert_eq!(closed.phase(), SessionPhase::Closed);

    let dry_run = engine.clean_sessions(CleanRequest {
        scope: CleanScope::Expired,
        mode: CleanMode::DryRun,
        cursor: None,
    })?;
    assert_eq!(dry_run.mode(), CleanMode::DryRun);
    assert_eq!(
        dry_run.entries(),
        [CleanEntry::Examined {
            session_id: opened.session_id.clone(),
            decision: CleanDecision::Eligible,
        }]
    );

    let removed = engine.clean_sessions(CleanRequest {
        scope: CleanScope::Expired,
        mode: CleanMode::Remove,
        cursor: None,
    })?;
    assert_eq!(
        removed.entries(),
        [CleanEntry::Examined {
            session_id: opened.session_id.clone(),
            decision: CleanDecision::Removed,
        }]
    );
    assert!(engine.list_sessions(None)?.entries().is_empty());
    // The original source is never touched by cleanup.
    assert_eq!(
        fs::read(harness.root.path("original media.mp4"))?,
        SOURCE_BYTES
    );
    Ok(())
}

#[tokio::test]
async fn an_open_session_becomes_cleanable_only_after_its_expiry() -> TestResult {
    let harness = Harness::new()?;
    let session_id = harness.open().await?;
    let request = CleanRequest {
        scope: CleanScope::Expired,
        mode: CleanMode::DryRun,
        cursor: None,
    };

    harness.clock.set(T0 + SessionLifetime::IDLE_SECONDS - 1);
    assert_eq!(
        harness.engine.clean_sessions(request)?.entries(),
        [CleanEntry::Examined {
            session_id: session_id.clone(),
            decision: CleanDecision::Ineligible,
        }]
    );

    harness.clock.set(T0 + SessionLifetime::IDLE_SECONDS);
    let snapshot = harness.engine.session_status(&session_id)?;
    assert_eq!(snapshot.phase(), SessionPhase::Open);
    assert!(
        snapshot
            .lifetime()
            .expired(snapshot.observed_at_unix_seconds())
    );
    assert_eq!(
        harness.engine.clean_sessions(request)?.entries(),
        [CleanEntry::Examined {
            session_id,
            decision: CleanDecision::Eligible,
        }]
    );
    Ok(())
}

#[tokio::test]
async fn retained_bundles_validate_independently_of_the_session_root() -> TestResult {
    let harness = Harness::new()?;
    let session_id = harness.open().await?;
    let output = harness.root.path("retained bundle");

    let retained =
        harness
            .engine
            .retain_session(&session_id, &output, SourceRetention::EvidenceOnly)?;
    let validated = harness.engine.validate_bundle(&output)?;

    assert_eq!(retained, validated);
    assert_eq!(validated.session_id(), &session_id);
    assert_eq!(validated.source_retention(), SourceRetention::EvidenceOnly);
    assert_eq!(validated.artifact_count(), 0);
    Ok(())
}

#[tokio::test]
async fn read_only_operations_never_create_a_missing_session_root() -> TestResult {
    let harness = Harness::new()?;
    let root = harness.root.path("sessions");

    assert!(harness.engine.list_sessions(None)?.entries().is_empty());
    let cleaned = harness.engine.clean_sessions(CleanRequest {
        scope: CleanScope::Expired,
        mode: CleanMode::Remove,
        cursor: None,
    })?;
    assert!(cleaned.entries().is_empty());
    let missing = harness.engine.session_status(&session(1)?);
    assert_eq!(
        missing,
        Err(EngineError::SessionRoot(SessionRootError::Missing))
    );
    assert_eq!(
        missing.map_err(|error| error.failure_code()),
        Err(FailureCode::StorageIo)
    );
    assert!(!root.exists());
    assert_eq!(harness.identifiers.issued(), 0);
    Ok(())
}

#[tokio::test]
async fn requests_the_engine_cannot_honour_fail_before_any_work() -> TestResult {
    let harness = Harness::new()?;

    let transcript = harness
        .engine
        .ingest(IngestRequest {
            source: harness.root.source()?,
            transcript: Some(harness.root.path("captions.srt")),
        })
        .await;
    assert_eq!(transcript, Err(EngineError::TranscriptImportUnavailable));
    assert_eq!(
        transcript.map_err(|error| error.failure_code()),
        Err(FailureCode::CommandNotImplemented)
    );

    let unrestricted = harness.engine.clean_sessions(CleanRequest {
        scope: CleanScope::Unrestricted,
        mode: CleanMode::DryRun,
        cursor: None,
    });
    assert_eq!(unrestricted, Err(EngineError::UnrestrictedCleanRejected));

    assert!(!harness.root.path("sessions").exists());
    assert_eq!(harness.identifiers.issued(), 0);
    Ok(())
}

#[tokio::test]
async fn relative_session_roots_and_unreadable_clocks_are_typed_failures() -> TestResult {
    let root = OwnedRoot::new()?;
    let relative = Engine::new(
        EngineConfig {
            session_root: SessionRootLocation::Explicit(PathBuf::from("relative-sessions")),
            user_configuration: UserConfigurationLocation::Explicit(root.path("config")),
            host_isolation: HostIsolation::ProcessOnly,
        },
        EnginePorts::new(ControlledClock::at(T0), SequentialIdentifiers::new()),
    );
    let listed = relative.list_sessions(None);
    assert_eq!(
        listed,
        Err(EngineError::SessionRoot(SessionRootError::NotAbsolute))
    );
    assert_eq!(
        listed.map_err(|error| error.failure_code()),
        Err(FailureCode::InvalidArgument)
    );

    let broken = Engine::new(
        EngineConfig {
            session_root: SessionRootLocation::Explicit(root.path("sessions")),
            user_configuration: UserConfigurationLocation::Explicit(root.path("config")),
            host_isolation: HostIsolation::ProcessOnly,
        },
        EnginePorts::new(BrokenClock, SequentialIdentifiers::new()),
    );
    let opened = broken
        .ingest(IngestRequest {
            source: root.source()?,
            transcript: None,
        })
        .await;
    assert_eq!(opened, Err(EngineError::Clock(ClockError::BeforeUnixEpoch)));
    assert_eq!(
        opened.map_err(|error| error.failure_code()),
        Err(FailureCode::Internal)
    );
    Ok(())
}

#[tokio::test]
async fn setup_check_reports_missing_explicit_paths_without_searching_path() -> TestResult {
    let harness = Harness::new()?;
    let configured_ffmpeg = harness.root.path("configured-ffmpeg");
    fs::write(&configured_ffmpeg, b"not an executable")?;
    harness
        .engine
        .configure_executable(RuntimeDependency::Ffmpeg, &configured_ffmpeg)?;

    let report = harness
        .engine
        .check_setup(SetupCheckRequest {
            probe_timeout: Duration::from_secs(5),
            per_call: ExecutableSelections {
                ffmpeg: None,
                ffprobe: Some(harness.root.path("missing-ffprobe")),
                whisper: Some(harness.root.path("missing-whisper")),
            },
        })
        .await?;

    assert_eq!(report.diagnosis().readiness, RuntimeReadiness::Blocked);
    assert_eq!(
        report.lookup(RuntimeDependency::Ffmpeg),
        DependencyLookup::ConfiguredUserPath
    );
    for dependency in [RuntimeDependency::Ffprobe, RuntimeDependency::Whisper] {
        assert_eq!(report.lookup(dependency), DependencyLookup::ExplicitPath);
        let status = report
            .diagnosis()
            .dependencies
            .iter()
            .find(|status| status.dependency == dependency)
            .ok_or("dependency was not probed")?;
        assert_eq!(status.state, DependencyState::Missing);
    }
    Ok(())
}

#[test]
fn model_identification_reports_identity_without_trusting_the_file() -> TestResult {
    let harness = Harness::new()?;
    let engine = &harness.engine;

    let unconfigured = engine.identify_model(ModelSelection::Configured);
    assert_eq!(unconfigured, Err(EngineError::ModelNotSelected));
    assert_eq!(
        unconfigured.map_err(|error| error.failure_code()),
        Err(FailureCode::MissingCapability)
    );

    let model = harness.root.path("ggml-model.bin");
    fs::write(&model, b"not the reviewed model")?;
    assert_eq!(
        engine.identify_model(ModelSelection::Explicit(model.clone()))?,
        ModelVerification::Unrecognised
    );
    assert_eq!(
        engine.identify_model(ModelSelection::Explicit(harness.root.path("absent.bin")))?,
        ModelVerification::Unreadable
    );

    engine.configure_model(&model)?;
    assert_eq!(
        engine.identify_model(ModelSelection::Configured)?,
        ModelVerification::Unrecognised
    );
    Ok(())
}

#[tokio::test]
async fn media_tool_verification_rejects_unusable_selections_before_running() -> TestResult {
    let harness = Harness::new()?;
    let request = |tools| MediaToolVerificationRequest {
        tools,
        workspace_parent: harness.root.0.clone(),
        cancellation: Cancellation::new(),
    };

    assert_eq!(
        harness
            .engine
            .verify_media_tools(request(MediaToolSelection::Configured))
            .await,
        Err(EngineError::DependencyNotSelected(
            RuntimeDependency::Ffmpeg
        ))
    );
    assert_eq!(
        harness
            .engine
            .verify_media_tools(request(MediaToolSelection::Explicit {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            }))
            .await,
        Err(EngineError::Executable(ExecutableRejection::NotAbsolute))
    );
    Ok(())
}

/// Real FFmpeg/FFprobe from `PATH` process the embedded reviewed fixture.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn selected_media_tools_verify_against_the_reviewed_fixture() -> TestResult {
    let harness = Harness::new()?;
    let ffmpeg = find_on_path("ffmpeg").ok_or("ffmpeg is not on PATH")?;
    let ffprobe = find_on_path("ffprobe").ok_or("ffprobe is not on PATH")?;

    let outcome = harness
        .engine
        .verify_media_tools(MediaToolVerificationRequest {
            tools: MediaToolSelection::Explicit { ffmpeg, ffprobe },
            workspace_parent: harness.root.0.clone(),
            cancellation: Cancellation::new(),
        })
        .await?;

    assert_eq!(outcome, MediaToolVerification::Verified);
    Ok(())
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let file_name = format!("{name}{}", env::consts::EXE_SUFFIX);
    env::split_paths(&env::var_os("PATH")?)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join(&file_name))
        .find(|candidate| Path::is_file(candidate))
}
