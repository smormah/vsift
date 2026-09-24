//! The automatic media-tool preflight through the engine library.
//!
//! A supplied-transcript ingest runs `FFprobe` on the source, so it must first
//! verify the resolved `FFmpeg`/`FFprobe` pair. These tests replace the reviewed
//! fixture verifier with a counting test double at the application port, so they
//! are fast and deterministic and need no real tools. The configured "tools" are
//! plain files: once the preflight passes, the real probe of the source fails on
//! them, which proves the preflight ran first and let the operation continue.
//!
//! The opt-in tests at the end use the real reviewed fixture verifier and need
//! `ffmpeg` and `ffprobe` on `PATH`:
//!
//! `cargo test -p vsift --locked --test engine_media_tool_preflight -- --ignored`

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use tokio::sync::Barrier;
use vsift::{
    Clock, ClockError, Engine, EngineConfig, EngineError, EnginePorts, FailureCode, HostIsolation,
    IdentifierGenerationError, IdentifierSource, IngestRequest, MediaToolCheck, MediaToolFailure,
    MediaToolPreflightFailure, MediaToolVerification, MediaToolVerifier, OpenSessionError,
    OperationId, RuntimeDependency, SessionId, SessionRootLocation, SuppliedTranscriptRequest,
    UserConfigurationLocation,
};

type TestResult = Result<(), Box<dyn Error>>;
/// Damages the verification record at the given path.
type Corruption = Box<dyn Fn(&Path) -> std::io::Result<()>>;

/// 2027-01-15T08:00:00Z: an arbitrary fixed start for deterministic ageing.
const T0: u64 = 1_800_000_000;
const SEVEN_DAYS: u64 = 7 * 24 * 60 * 60;
const OWNED_PREFIX: &str = "vsift-engine-preflight-test-";
const PLACEHOLDER_SOURCE: &[u8] = b"\0\0\0\x18ftypisomengine-preflight-source";
const SIDECAR: &[u8] = b"1\n00:00:00,500 --> 00:00:01,000\nx\n";
const STATE: &str = "media-tool-verification";
const RECORD: &str = "verified-v1.json";

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

/// A verifier double that returns a scripted result and counts its runs.
#[derive(Clone)]
struct CountingVerifier {
    result: MediaToolVerification,
    /// Holds every run until all expected runs are in flight, so concurrent
    /// preflights provably overlap.
    rendezvous: Option<Arc<Barrier>>,
    calls: Arc<AtomicUsize>,
}

impl CountingVerifier {
    fn passing() -> Self {
        Self::returning(MediaToolVerification::Verified)
    }

    fn returning(result: MediaToolVerification) -> Self {
        Self {
            result,
            rendezvous: None,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl MediaToolVerifier for CountingVerifier {
    async fn verify(&self) -> MediaToolVerification {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(rendezvous) = &self.rendezvous {
            rendezvous.wait().await;
        }
        self.result
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
}

impl Harness {
    fn new() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            root: OwnedRoot::new()?,
            clock: ControlledClock::at(T0),
            identifiers: SequentialIdentifiers::new(),
        })
    }

    fn engine(&self, verifier: Option<&CountingVerifier>) -> Engine {
        let ports = EnginePorts::new(self.clock.clone(), self.identifiers.clone());
        let ports = match verifier {
            Some(verifier) => ports.with_media_tool_verifier(verifier.clone()),
            None => ports,
        };
        Engine::new(
            EngineConfig {
                session_root: SessionRootLocation::Explicit(self.sessions()),
                user_configuration: UserConfigurationLocation::Explicit(self.root.path("config")),
                host_isolation: HostIsolation::ProcessOnly,
            },
            ports,
        )
    }

    fn sessions(&self) -> PathBuf {
        self.root.path("sessions")
    }

    fn state(&self) -> PathBuf {
        self.root.path("config").join(STATE)
    }

    fn record(&self) -> PathBuf {
        self.state().join(RECORD)
    }

    fn write(&self, name: &str, content: &[u8]) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.root.path(name);
        fs::write(&path, content)?;
        Ok(path)
    }

    /// Configures plain files as `FFmpeg` and `FFprobe`; returns the `FFprobe` path.
    fn configure_stand_in_tools(&self, engine: &Engine) -> Result<PathBuf, Box<dyn Error>> {
        let ffmpeg = self.write("ffmpeg-stand-in.exe", b"not a program: ffmpeg")?;
        let ffprobe = self.write("ffprobe-stand-in.exe", b"not a program: ffprobe")?;
        engine.configure_executable(RuntimeDependency::Ffmpeg, &ffmpeg)?;
        engine.configure_executable(RuntimeDependency::Ffprobe, &ffprobe)?;
        Ok(ffprobe)
    }

    fn transcript_request(&self) -> Result<IngestRequest, Box<dyn Error>> {
        Ok(IngestRequest {
            source: self.write("placeholder.mp4", PLACEHOLDER_SOURCE)?,
            transcript: Some(SuppliedTranscriptRequest {
                path: self.write("captions.srt", SIDECAR)?,
                offset_micros: 0,
            }),
        })
    }
}

/// After a passing preflight the operation continues to the real probe, which
/// fails on the stand-in tools; a verification failure would stop earlier.
fn assert_preflight_passed(result: &Result<vsift::IngestOutcome, EngineError>) {
    assert!(
        matches!(
            result,
            Err(EngineError::OpenSession(OpenSessionError::SourceProbe(_)))
        ),
        "expected the post-preflight probe failure, got {result:?}"
    );
}

#[tokio::test]
async fn a_pass_is_recorded_once_and_reused_until_the_tools_change_or_age_out() -> TestResult {
    let harness = Harness::new()?;
    let verifier = CountingVerifier::passing();
    let engine = harness.engine(Some(&verifier));
    let ffprobe = harness.configure_stand_in_tools(&engine)?;

    assert_preflight_passed(&engine.ingest(harness.transcript_request()?).await);
    assert_eq!(verifier.calls(), 1, "cache miss verifies");
    assert!(harness.record().is_file());

    assert_preflight_passed(&engine.ingest(harness.transcript_request()?).await);
    assert_eq!(verifier.calls(), 1, "cache hit does not verify");

    // A new engine reads the same per-user record.
    assert_preflight_passed(
        &harness
            .engine(Some(&verifier))
            .ingest(harness.transcript_request()?)
            .await,
    );
    assert_eq!(verifier.calls(), 1);

    // A replaced FFprobe is a different identity.
    fs::write(&ffprobe, b"a different ffprobe build, larger")?;
    assert_preflight_passed(&engine.ingest(harness.transcript_request()?).await);
    assert_eq!(verifier.calls(), 2, "changed executable invalidates");
    assert_preflight_passed(&engine.ingest(harness.transcript_request()?).await);
    assert_eq!(verifier.calls(), 2);

    // Selecting a different executable is a different identity too.
    let other = harness.write("other-ffprobe.exe", b"another ffprobe")?;
    engine.configure_executable(RuntimeDependency::Ffprobe, &other)?;
    assert_preflight_passed(&engine.ingest(harness.transcript_request()?).await);
    assert_eq!(verifier.calls(), 3);

    // A pass ages out after seven days.
    harness.clock.set(T0 + SEVEN_DAYS);
    assert_preflight_passed(&engine.ingest(harness.transcript_request()?).await);
    assert_eq!(verifier.calls(), 4, "an aged-out pass is re-verified");

    // The record holds digests and times only, never a path.
    let record = fs::read_to_string(harness.record())?;
    assert!(!record.contains("stand-in"), "{record}");
    assert!(!record.contains("config"), "{record}");
    Ok(())
}

#[tokio::test]
async fn a_failed_verification_stops_before_any_session_write_and_is_not_recorded() -> TestResult {
    let harness = Harness::new()?;
    let failure = MediaToolPreflightFailure {
        check: MediaToolCheck::Probe,
        failure: MediaToolFailure::UnexpectedResult,
    };
    let verifier = CountingVerifier::returning(MediaToolVerification::Failed {
        check: failure.check,
        failure: failure.failure,
    });
    let engine = harness.engine(Some(&verifier));
    harness.configure_stand_in_tools(&engine)?;

    for attempt in 1..=2 {
        let result = engine.ingest(harness.transcript_request()?).await;

        assert_eq!(
            result.as_ref().map(|_| ()),
            Err(&EngineError::MediaToolVerificationFailed(failure))
        );
        let error = result.err().ok_or("expected a failure")?;
        assert_eq!(error.failure_code(), FailureCode::MissingCapability);
        assert_eq!(error.media_tool_verification_failure(), Some(failure));
        assert_eq!(verifier.calls(), attempt, "failures are never cached");
    }
    assert!(!harness.sessions().exists(), "session root was touched");
    assert_eq!(harness.identifiers.issued(), 0, "identifiers were issued");
    assert!(!harness.record().exists(), "a failure was recorded");
    Ok(())
}

#[tokio::test]
async fn defective_or_linked_records_are_ignored_and_rewritten() -> TestResult {
    let harness = Harness::new()?;
    let verifier = CountingVerifier::passing();
    let engine = harness.engine(Some(&verifier));
    harness.configure_stand_in_tools(&engine)?;
    assert_preflight_passed(&engine.ingest(harness.transcript_request()?).await);
    let good = fs::read(harness.record())?;
    let outside = harness.write("outside-record.json", &good)?;

    let mut oversized = good.clone();
    oversized.extend(std::iter::repeat_n(b' ', 5000));
    let corruptions: [(&str, Corruption); 4] = [
        ("garbage", Box::new(|path| fs::write(path, b"{not json"))),
        (
            "oversized",
            Box::new(move |path| fs::write(path, &oversized)),
        ),
        (
            "unknown field",
            Box::new(|path| {
                fs::write(path, br#"{"schema_version":1,"entries":[],"trusted":true}"#)
            }),
        ),
        (
            "hard link",
            Box::new(move |path| {
                fs::remove_file(path)?;
                fs::hard_link(&outside, path)
            }),
        ),
    ];
    let mut expected_calls = 1;
    for (label, corrupt) in corruptions {
        corrupt(&harness.record())?;

        assert_preflight_passed(&engine.ingest(harness.transcript_request()?).await);
        expected_calls += 1;
        assert_eq!(verifier.calls(), expected_calls, "{label} was trusted");

        // The pass was recorded again in a fresh, valid record.
        let rewritten: serde_json::Value = serde_json::from_slice(&fs::read(harness.record())?)?;
        assert_eq!(rewritten["schema_version"], 1, "{label}");
        assert_eq!(
            rewritten["entries"].as_array().map(Vec::len),
            Some(1),
            "{label}"
        );
        assert_preflight_passed(&engine.ingest(harness.transcript_request()?).await);
        assert_eq!(verifier.calls(), expected_calls, "{label} not rewritten");
    }
    // The hard-linked file outside the state directory was never written through.
    assert_eq!(fs::read(harness.root.path("outside-record.json"))?, good);
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn a_symbolic_link_record_is_not_followed() -> TestResult {
    let harness = Harness::new()?;
    let verifier = CountingVerifier::passing();
    let engine = harness.engine(Some(&verifier));
    harness.configure_stand_in_tools(&engine)?;
    assert_preflight_passed(&engine.ingest(harness.transcript_request()?).await);
    let good = fs::read(harness.record())?;
    let outside = harness.write("outside-record.json", &good)?;
    fs::remove_file(harness.record())?;
    std::os::unix::fs::symlink(&outside, harness.record())?;

    assert_preflight_passed(&engine.ingest(harness.transcript_request()?).await);

    assert_eq!(verifier.calls(), 2);
    assert!(
        !fs::symlink_metadata(harness.record())?
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&outside)?, good);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_preflights_all_proceed_and_leave_one_valid_record() -> TestResult {
    const ENGINES: usize = 6;
    let harness = Harness::new()?;
    let mut verifier = CountingVerifier::passing();
    verifier.rendezvous = Some(Arc::new(Barrier::new(ENGINES)));
    let setup = harness.engine(None);
    harness.configure_stand_in_tools(&setup)?;
    // Provision the session root first: this test is about the preflight, and
    // first-use root provisioning is not designed for concurrent creators.
    setup
        .ingest(IngestRequest {
            source: harness.write("provision.mp4", PLACEHOLDER_SOURCE)?,
            transcript: None,
        })
        .await?;

    let mut tasks = Vec::with_capacity(ENGINES);
    for _ in 0..ENGINES {
        let engine = harness.engine(Some(&verifier));
        let request = harness.transcript_request_unique(tasks.len())?;
        tasks.push(tokio::spawn(async move { engine.ingest(request).await }));
    }
    for task in tasks {
        // Past the preflight, the session store's own admission may refuse a
        // concurrent open as busy; either way the preflight let it through.
        let result = task.await?;
        assert!(
            matches!(
                result,
                Err(EngineError::OpenSession(
                    OpenSessionError::SourceProbe(_)
                        | OpenSessionError::Storage(vsift::SessionStorageError::Busy)
                ))
            ),
            "expected a post-preflight failure, got {result:?}"
        );
    }

    // Every engine verified (none waited on another) and the record is valid.
    assert_eq!(verifier.calls(), ENGINES);
    let record: serde_json::Value = serde_json::from_slice(&fs::read(harness.record())?)?;
    assert_eq!(record["entries"].as_array().map(Vec::len), Some(1));
    let leftovers = fs::read_dir(harness.state())?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(leftovers.len(), 2, "{leftovers:?}");

    assert_preflight_passed(
        &harness
            .engine(Some(&verifier))
            .ingest(harness.transcript_request()?)
            .await,
    );
    assert_eq!(verifier.calls(), ENGINES, "the recorded pass is reused");
    Ok(())
}

impl Harness {
    /// A transcript request with its own files, so concurrent requests never
    /// rewrite a file another request is reading.
    fn transcript_request_unique(&self, index: usize) -> Result<IngestRequest, Box<dyn Error>> {
        Ok(IngestRequest {
            source: self.write(&format!("placeholder-{index}.mp4"), PLACEHOLDER_SOURCE)?,
            transcript: Some(SuppliedTranscriptRequest {
                path: self.write(&format!("captions-{index}.srt"), SIDECAR)?,
                offset_micros: 0,
            }),
        })
    }
}

#[tokio::test]
async fn plain_ingest_setup_and_rejected_transcripts_never_run_the_preflight() -> TestResult {
    let harness = Harness::new()?;
    let verifier = CountingVerifier::passing();
    let engine = harness.engine(Some(&verifier));
    harness.configure_stand_in_tools(&engine)?;

    engine
        .ingest(IngestRequest {
            source: harness.write("plain.mp4", PLACEHOLDER_SOURCE)?,
            transcript: None,
        })
        .await?;
    let rejected = engine
        .ingest(IngestRequest {
            source: harness.write("rejected.mp4", PLACEHOLDER_SOURCE)?,
            transcript: Some(SuppliedTranscriptRequest {
                path: harness.write("bad.srt", b"not a transcript")?,
                offset_micros: 0,
            }),
        })
        .await;

    assert!(rejected.is_err());
    assert_eq!(verifier.calls(), 0);
    assert!(!harness.state().exists(), "preflight state was created");
    Ok(())
}

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn find_on_path(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    let file_name = format!("{name}{}", env::consts::EXE_SUFFIX);
    env::split_paths(&env::var_os("PATH").ok_or("PATH is not set")?)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join(&file_name))
        .find(|candidate| Path::is_file(candidate))
        .ok_or_else(|| format!("{name} is not on PATH").into())
}

fn f10_request() -> IngestRequest {
    IngestRequest {
        source: repository("fixtures/corpus/generated/F10.mp4"),
        transcript: Some(SuppliedTranscriptRequest {
            path: repository("fixtures/corpus/transcripts/F10.srt"),
            offset_micros: 500_000,
        }),
    }
}

/// Opt-in: the reviewed fixture verifier runs once for real tools, and the
/// second import reuses the recorded pass.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn real_tools_are_verified_once_and_the_pass_is_reused() -> TestResult {
    let harness = Harness::new()?;
    let engine = harness.engine(None);
    engine.configure_executable(RuntimeDependency::Ffmpeg, &find_on_path("ffmpeg")?)?;
    engine.configure_executable(RuntimeDependency::Ffprobe, &find_on_path("ffprobe")?)?;

    let first_started = Instant::now();
    engine.ingest(f10_request()).await?;
    let first = first_started.elapsed();
    assert!(harness.record().is_file());
    let second_started = Instant::now();
    engine.ingest(f10_request()).await?;
    let second = second_started.elapsed();

    println!(
        "first import with verification: {first:?}; second import with cached pass: {second:?}"
    );
    let workspaces = fs::read_dir(harness.state())?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        workspaces.len(),
        2,
        "verification left a workspace: {workspaces:?}"
    );
    Ok(())
}

/// Opt-in: `FFmpeg` in `FFprobe`'s place passes a version probe but fails the
/// fixture at the probe check, before any session is written.
#[tokio::test]
#[ignore = "requires FFmpeg on PATH"]
async fn ffmpeg_selected_as_ffprobe_fails_at_the_probe_check() -> TestResult {
    let harness = Harness::new()?;
    let engine = harness.engine(None);
    let ffmpeg = find_on_path("ffmpeg")?;
    engine.configure_executable(RuntimeDependency::Ffmpeg, &ffmpeg)?;
    engine.configure_executable(RuntimeDependency::Ffprobe, &ffmpeg)?;

    let result = engine.ingest(f10_request()).await;

    let failure = result
        .as_ref()
        .err()
        .and_then(EngineError::media_tool_verification_failure)
        .ok_or_else(|| format!("expected a verification failure, got {result:?}"))?;
    assert_eq!(failure.check, MediaToolCheck::Probe);
    assert_eq!(
        result.map_err(|error| error.failure_code()).map(|_| ()),
        Err(FailureCode::MissingCapability)
    );
    assert!(!harness.sessions().exists());
    assert!(!harness.record().exists());
    Ok(())
}
