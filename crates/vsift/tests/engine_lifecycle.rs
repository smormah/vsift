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
    ClockError, DEFAULT_LOCAL_ASR_CHECK_BUDGET, DependencyState, Engine, EngineConfig, EngineError,
    EnginePorts, ExecutableRejection, ExecutableSelections, FailureCode, HostIsolation,
    IdentifierGenerationError, IdentifierSource, IngestRequest, LocalAsrCheckOutcome,
    LocalAsrModelStatus, LocalAsrNotRunReason, LocalAsrSetupStatus, MediaToolSelection,
    MediaToolVerification, MediaToolVerificationRequest, ModelSelection, ModelVerification,
    OperationId, RuntimeDependency, RuntimeReadiness, SessionId, SessionLifetime, SessionListEntry,
    SessionPhase, SessionRootError, SessionRootLocation, SetupCheckRequest, SourceRetention,
    SuppliedTranscriptRequest, TranscriptSourceError, UserConfigurationLocation,
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
                cancellation: Cancellation::new(),
                durability: vsift::DurabilityRequirement::Ephemeral,
            })
            .await?;
        Ok(opened.session.session_id)
    }
}

fn session(number: u64) -> Result<SessionId, Box<dyn Error>> {
    Ok(SessionId::parse(format!("ses_{number:032x}"))?)
}

/// A durable ingest (engine level only, ADR 0020 D-3) is honoured only on
/// the qualified profile (Ubuntu 24.04, local ext4 with write barriers);
/// anywhere else it fails with `MISSING_CAPABILITY` before any session is
/// registered, and it is never downgraded to an ephemeral session.
#[tokio::test]
async fn a_durable_ingest_is_durable_or_fails_closed() -> TestResult {
    let harness = Harness::new()?;
    let opened = harness
        .engine
        .ingest(IngestRequest {
            source: harness.root.source()?,
            transcript: None,
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Durable,
        })
        .await;
    let sessions = harness.root.path("sessions").join("sessions");
    match opened {
        Ok(outcome) => {
            assert_eq!(std::env::consts::OS, "linux", "only Linux can be qualified");
            assert_eq!(
                outcome.session.publication,
                vsift::PublicationGuarantee::OsCrashDurable
            );
            let manifest = fs::read_to_string(
                sessions
                    .join(outcome.session.session_id.as_str())
                    .join("generations")
                    .join("0.json"),
            )?;
            assert!(manifest.contains(r#""durability":"durable""#), "{manifest}");
        }
        Err(error) => {
            assert_eq!(error.failure_code(), FailureCode::MissingCapability);
            assert!(
                !sessions.exists() || fs::read_dir(&sessions)?.next().is_none(),
                "a session was created"
            );
        }
    }
    // An ephemeral session reports its own guarantee, never the root's
    // strongest one, on every host.
    let opened = harness
        .engine
        .ingest(IngestRequest {
            source: harness.root.source()?,
            transcript: None,
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Ephemeral,
        })
        .await?;
    assert_eq!(
        opened.session.publication,
        vsift::PublicationGuarantee::ProcessCrashConsistent
    );
    Ok(())
}

#[tokio::test]
async fn open_status_renew_close_and_clean_use_the_injected_clock_and_identifiers() -> TestResult {
    let harness = Harness::new()?;
    let engine = &harness.engine;

    let opened = engine
        .ingest(IngestRequest {
            source: harness.root.source()?,
            transcript: None,
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Ephemeral,
        })
        .await?;
    // One session identity, then initialize, stage and activate operations.
    assert_eq!(opened.session.session_id, session(1)?);
    assert_eq!(harness.identifiers.issued(), 4);
    assert_eq!(
        opened.session.source_bytes,
        u64::try_from(SOURCE_BYTES.len())?
    );
    assert_eq!(opened.session.lifetime, SessionLifetime::open(T0)?);
    assert_eq!(
        opened.session.lifetime.expires_at_unix_seconds(),
        T0 + SessionLifetime::IDLE_SECONDS
    );

    harness.clock.set(T0 + 100);
    let status = engine.session_status(&opened.session.session_id)?;
    assert_eq!(status.session_id(), &opened.session.session_id);
    assert_eq!(status.source_id(), &opened.session.source_id);
    assert_eq!(status.phase(), SessionPhase::Open);
    assert_eq!(status.generation(), opened.session.generation);
    assert_eq!(status.observed_at_unix_seconds(), T0 + 100);
    assert_eq!(status.artifact_count(), 0);

    harness.clock.set(T0 + 3_600);
    let renewed = engine.renew_session(&opened.session.session_id)?;
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

    let closed = engine.close_session(&opened.session.session_id)?;
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
            session_id: opened.session.session_id.clone(),
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
            session_id: opened.session.session_id.clone(),
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

    // A supplied transcript is read and validated before the root is touched.
    let transcript = harness
        .engine
        .ingest(IngestRequest {
            source: harness.root.source()?,
            transcript: Some(SuppliedTranscriptRequest {
                path: harness.root.path("captions.srt"),
                offset_micros: 0,
            }),
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Ephemeral,
        })
        .await;
    assert_eq!(
        transcript,
        Err(EngineError::TranscriptSource(TranscriptSourceError::Io))
    );
    assert_eq!(
        transcript.map_err(|error| error.failure_code()),
        Err(FailureCode::StorageIo)
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
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Ephemeral,
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
            local_asr_budget: DEFAULT_LOCAL_ASR_CHECK_BUDGET,
        })
        .await?;

    assert_eq!(report.diagnosis().readiness, RuntimeReadiness::Blocked);
    assert_eq!(
        *report.local_asr(),
        LocalAsrSetupStatus {
            model: LocalAsrModelStatus::NotSelected,
            verification: LocalAsrCheckOutcome::NotRun(LocalAsrNotRunReason::MediaToolsUnavailable),
        }
    );
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

/// A worker workspace's policy (ADR 0021 D1, D2) for the harness root.
fn workspace_policy(
    durability: vsift::DurabilityRequirement,
    hours: u64,
) -> Result<vsift::WorkspacePolicy, Box<dyn Error>> {
    Ok(vsift::WorkspacePolicy::new(
        durability,
        std::num::NonZeroU16::new(2).ok_or("zero")?,
        vsift::WorkspaceRetention::from_hours(hours)?,
    )?)
}

/// ADR 0021 D2: a session of a worker workspace lives the workspace's
/// retention after it opens or is renewed, never beyond 720 hours, and
/// `session clean --expired` removes it once that has passed.
#[tokio::test]
async fn a_workspace_session_lives_the_workspace_retention() -> TestResult {
    let harness = Harness::new()?;
    let policy = workspace_policy(vsift::DurabilityRequirement::Ephemeral, 48)?;
    let created = harness
        .engine
        .init_workspace(vsift::WorkspaceInitRequest { policy })?;
    assert_eq!(created.outcome(), vsift::WorkspaceInitOutcome::Created);
    assert_eq!(
        created.publication(),
        vsift::PublicationGuarantee::ProcessCrashConsistent
    );
    let retention = 48 * 3_600;

    let session_id = harness.open().await?;
    let opened = harness.engine.session_status(&session_id)?.lifetime();
    assert_eq!(
        opened.policy(),
        vsift::SessionLifetimePolicy::Workspace(policy.retention())
    );
    assert_eq!(opened.expires_at_unix_seconds(), T0 + retention);

    harness.clock.set(T0 + retention - 1);
    let renewed = harness.engine.renew_session(&session_id)?.lifetime();
    assert_eq!(renewed.expires_at_unix_seconds(), T0 + 2 * retention - 1);
    // Fifteen renewals of 48 hours reach the 720-hour limit; one more
    // cannot pass it.
    for _ in 0..16 {
        let expires = harness
            .engine
            .session_status(&session_id)?
            .lifetime()
            .expires_at_unix_seconds();
        harness.clock.set(expires - 1);
        harness.engine.renew_session(&session_id)?;
    }
    let capped = harness.engine.session_status(&session_id)?.lifetime();
    assert_eq!(capped.expires_at_unix_seconds(), T0 + 720 * 3_600);

    let request = CleanRequest {
        scope: CleanScope::Expired,
        mode: CleanMode::Remove,
        cursor: None,
    };
    harness.clock.set(T0 + 720 * 3_600 - 1);
    assert_eq!(
        harness.engine.clean_sessions(request)?.entries(),
        [CleanEntry::Examined {
            session_id: session_id.clone(),
            decision: CleanDecision::Ineligible,
        }]
    );
    harness.clock.set(T0 + 720 * 3_600);
    assert_eq!(
        harness.engine.clean_sessions(request)?.entries(),
        [CleanEntry::Examined {
            session_id,
            decision: CleanDecision::Removed,
        }]
    );
    Ok(())
}

/// A request never raises a workspace's policy: a durable session in an
/// ephemeral workspace is refused before any session is registered, and a
/// durable workspace makes every session durable (ADR 0020 D-3).
#[tokio::test]
async fn a_workspace_decides_the_durability_of_its_sessions() -> TestResult {
    let harness = Harness::new()?;
    harness.engine.init_workspace(vsift::WorkspaceInitRequest {
        policy: workspace_policy(vsift::DurabilityRequirement::Ephemeral, 168)?,
    })?;
    let refused = harness
        .engine
        .ingest(IngestRequest {
            source: harness.root.source()?,
            transcript: None,
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Durable,
        })
        .await;
    match refused {
        Err(EngineError::WorkspaceNotDurable) => {}
        other => return Err(format!("expected WorkspaceNotDurable, got {other:?}").into()),
    }
    assert!(harness.engine.list_sessions(None)?.entries().is_empty());
    // The workspace's free-space reserve is checked before the copy on Unix.
    let opened = harness
        .engine
        .ingest(IngestRequest {
            source: harness.root.source()?,
            transcript: None,
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Ephemeral,
        })
        .await?;
    assert_eq!(
        opened.free_space,
        if cfg!(unix) {
            vsift::FreeSpaceReserveCheck::Enforced
        } else {
            vsift::FreeSpaceReserveCheck::NotEnforced
        }
    );

    let durable = Harness::new()?;
    let initialised = durable.engine.init_workspace(vsift::WorkspaceInitRequest {
        policy: workspace_policy(vsift::DurabilityRequirement::Durable, 168)?,
    });
    let qualified =
        vsift_infrastructure::directory_offers_os_crash_durability(&durable.root.path(""));
    match initialised {
        Ok(created) => {
            assert!(qualified, "a durable workspace on an unqualified host");
            assert_eq!(
                created.publication(),
                vsift::PublicationGuarantee::OsCrashDurable
            );
            let opened = durable
                .engine
                .ingest(IngestRequest {
                    source: durable.root.source()?,
                    transcript: None,
                    cancellation: Cancellation::new(),
                    durability: vsift::DurabilityRequirement::Ephemeral,
                })
                .await?;
            assert_eq!(
                opened.session.publication,
                vsift::PublicationGuarantee::OsCrashDurable
            );
        }
        Err(EngineError::SessionRoot(SessionRootError::DurabilityUnavailable)) => {
            assert!(!qualified, "a qualified host refused a durable workspace");
            assert!(!durable.root.path("sessions").exists());
        }
        Err(other) => return Err(format!("unexpected failure: {other}").into()),
    }
    Ok(())
}
