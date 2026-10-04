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
    Cancellation, Clock, ClockError, CueMarkup, Engine, EngineConfig, EngineError, EnginePorts,
    FailureCode, HostIsolation, IdentifierGenerationError, IdentifierSource, IngestRequest,
    ManagedRootLocation, OperationId, RuntimeDependency, SegmentOrigin, SessionId,
    SessionListEntry, SessionRootError, SessionRootLocation, SessionStorageError,
    SuppliedTranscriptRequest, TranscriptImportError, TranscriptProvenance, TranscriptQuery,
    TranscriptRejection, TranscriptSegment, UserConfigurationLocation,
};
use vsift_application::{InitializeSessionStorage, InitializeSessionStorageRequest};
use vsift_infrastructure::FilesystemSessionStore;

type TestResult = Result<(), Box<dyn Error>>;

/// Ordinal of the supplied cue an imported segment was read from.
fn cue_ordinal(segment: &TranscriptSegment) -> Option<u32> {
    match segment.origin() {
        SegmentOrigin::ImportedCue { cue, .. } => Some(cue.ordinal()),
        SegmentOrigin::Asr { .. } => None,
    }
}

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
        Self::with_ports(|ports| ports)
    }

    /// A harness whose engine waits only `wait` for a busy root to let a failed
    /// open remove its registration, so a test of the path where it cannot does
    /// not take the production five seconds.
    fn with_removal_wait(wait: std::time::Duration) -> Result<Self, Box<dyn Error>> {
        Self::with_ports(|ports| ports.with_failed_open_removal_wait(wait))
    }

    fn with_ports(
        configure: impl FnOnce(EnginePorts) -> EnginePorts,
    ) -> Result<Self, Box<dyn Error>> {
        let root = OwnedRoot::new()?;
        let clock = ControlledClock::at(T0);
        let identifiers = SequentialIdentifiers::new();
        let engine = Engine::new(
            EngineConfig {
                session_root: SessionRootLocation::Explicit(root.path("sessions")),
                user_configuration: UserConfigurationLocation::Explicit(root.path("config")),
                managed_root: ManagedRootLocation::Explicit(root.path("managed")),
                host_isolation: HostIsolation::ProcessOnly,
            },
            configure(EnginePorts::new(clock.clone(), identifiers.clone())),
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
                cancellation: Cancellation::new(),
                durability: vsift::DurabilityRequirement::Ephemeral,
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
    // The session it began is gone too (#277), not left initializing.
    let listed = harness.engine.list_sessions(None)?;
    assert!(listed.entries().is_empty(), "{listed:?}");
    Ok(())
}

/// #277: a registration whose opening never finished (a crash leaves one; a
/// failed open used to) is listed as initializing, and every command that
/// names it answers `SessionNotPublished`, as does an id that was never
/// registered. **The code is the one the lookup always gave, `STORAGE_IO`**
/// (changing a published failure code is not additive within v1, known limit
/// L-127); what is new is the typed error that carries the remediation saying
/// it is a missing session. A real I/O failure is not touched by this.
#[tokio::test]
async fn a_session_that_never_finished_opening_is_not_published_but_keeps_its_code() -> TestResult {
    let harness = Harness::new()?;
    let registered = register_foreign_session(&harness, FOREIGN_SESSION)?;

    let listed = harness.engine.list_sessions(None)?;
    let [SessionListEntry::Initializing(initializing)] = listed.entries() else {
        return Err(format!("expected one initializing session: {listed:?}").into());
    };
    assert_eq!(initializing, &registered);
    let never_registered = SessionId::parse("ses_00000000000000000000000000000001")?;
    let not_published = EngineError::SessionNotPublished(SessionStorageError::Io);
    for session in [initializing, &never_registered] {
        let status = harness.engine.session_status(session);
        assert_eq!(status, Err(not_published.clone()), "{session:?}");
        assert_eq!(
            status.map_err(|error| error.failure_code()).map(|_| ()),
            Err(FailureCode::StorageIo)
        );
        assert_eq!(
            harness.engine.renew_session(session).map(|_| ()),
            Err(not_published.clone())
        );
        assert_eq!(
            harness.engine.close_session(session).map(|_| ()),
            Err(not_published.clone())
        );
    }
    Ok(())
}

/// #277 review: a session that was closed and cleaned leaves its lock files
/// behind, so the lookup used to find its folder missing as **damage**: the
/// status of a cleaned id answered `INTEGRITY_FAILURE`, which says stored data
/// is corrupt. It is still that code (v1 keeps it), but it is now the typed
/// not-published error with its remediation, which says it is a missing session.
/// A session that still exists is untouched by this.
#[tokio::test]
async fn a_closed_and_cleaned_session_is_not_published_and_not_damaged() -> TestResult {
    let harness = Harness::new()?;
    let opened = harness
        .engine
        .ingest(IngestRequest {
            source: harness.placeholder_source()?,
            transcript: None,
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Ephemeral,
        })
        .await?;
    let session = opened.session.session_id;
    harness.engine.close_session(&session)?;
    let cleaned = harness.engine.clean_sessions(vsift::CleanRequest {
        scope: vsift::CleanScope::Expired,
        mode: vsift::CleanMode::Remove,
        cursor: None,
    })?;
    assert_eq!(
        cleaned.entries(),
        [vsift::CleanEntry::Examined {
            session_id: session.clone(),
            decision: vsift::CleanDecision::Removed,
        }]
    );
    let after = harness.engine.session_status(&session);

    assert_eq!(
        after,
        Err(EngineError::SessionNotPublished(
            SessionStorageError::IntegrityFailure
        ))
    );
    assert_eq!(
        after.map_err(|error| error.failure_code()).map(|_| ()),
        Err(FailureCode::IntegrityFailure)
    );
    Ok(())
}

/// A session some other operation registered and has not finished opening.
const FOREIGN_SESSION: &str = "ses_000000000000000000000000000000ff";

/// Registers `session` as another opener would, in the harness's root, and
/// lets go of the registration's marker hold as a crashed opener's would be.
fn register_foreign_session(harness: &Harness, session: &str) -> Result<SessionId, Box<dyn Error>> {
    let store = FilesystemSessionStore::provision_default(harness.root.path("sessions"))?;
    let session = SessionId::parse(session)?;
    let foreign_opener = OperationId::parse("op_000000000000000000000000000000ff")?;
    drop(store.register_session(&session, &foreign_opener, T0)?);
    Ok(session)
}

/// The registration markers under the harness's session root.
fn registration_markers(harness: &Harness) -> Result<usize, Box<dyn Error>> {
    let index = harness.root.path("sessions").join("session-index");
    let mut markers = 0;
    for bucket in fs::read_dir(index)? {
        let bucket = bucket?;
        if bucket.file_type()?.is_dir() {
            markers += fs::read_dir(bucket.path())?.count();
        }
    }
    Ok(markers)
}

/// #277: a source that is not media is refused after its session was
/// registered, and the open that made the registration removes it at once:
/// nothing is listed (not even as initializing), no registration or session
/// folder remains, and the next `session clean` has nothing to collect. Before
/// the fix each failed ingest left one registration for a day.
#[tokio::test]
async fn a_failed_ingest_leaves_nothing_behind() -> TestResult {
    let harness = Harness::new()?;
    let not_media = harness.sidecar("not-media.mp4", b"plain text, not a video")?;
    for _ in 0..3 {
        let failed = harness
            .engine
            .ingest(IngestRequest {
                source: not_media.clone(),
                transcript: None,
                cancellation: Cancellation::new(),
                durability: vsift::DurabilityRequirement::Ephemeral,
            })
            .await;
        assert_eq!(
            failed.map_err(|error| error.failure_code()).map(|_| ()),
            Err(FailureCode::InvalidSource)
        );
    }

    let listed = harness.engine.list_sessions(None)?;
    assert!(listed.entries().is_empty(), "{listed:?}");
    assert_eq!(registration_markers(&harness)?, 0);
    assert_eq!(
        fs::read_dir(harness.root.path("sessions").join("sessions"))?.count(),
        0
    );
    Ok(())
}

/// #277: the removal is the failed open's own. Another opener's registration
/// (a different operation, in the same root, not finished) survives a failed
/// ingest untouched and is still listed as initializing.
#[tokio::test]
async fn a_failed_ingest_never_removes_another_openers_registration() -> TestResult {
    let harness = Harness::new()?;
    let foreign = register_foreign_session(&harness, FOREIGN_SESSION)?;
    let not_media = harness.sidecar("not-media.mp4", b"plain text, not a video")?;

    let failed = harness
        .engine
        .ingest(IngestRequest {
            source: not_media,
            transcript: None,
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Ephemeral,
        })
        .await;
    assert_eq!(
        failed.map_err(|error| error.failure_code()).map(|_| ()),
        Err(FailureCode::InvalidSource)
    );

    let listed = harness.engine.list_sessions(None)?;
    assert_eq!(
        listed.entries(),
        [SessionListEntry::Initializing(foreign)].as_slice()
    );
    assert_eq!(registration_markers(&harness)?, 1);
    Ok(())
}

/// One ingest of a source that is not media, reduced to the failure it ended
/// with.
async fn refused(harness: &Harness, source: &Path) -> Result<FailureCode, String> {
    let outcome = harness
        .engine
        .ingest(IngestRequest {
            source: source.to_owned(),
            transcript: None,
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Ephemeral,
        })
        .await;
    match outcome {
        Err(error) => Ok(error.failure_code()),
        Ok(_) => Err("an ingest of a text file succeeded".to_owned()),
    }
}

/// #277 review: when the removal of a failed open's registration cannot proceed
/// (the root stays busy for the whole wait), the request still ends with **its
/// own failure** and not with anything about the removal, and the registration
/// is left alone and removable. Here the registration is held by another opener
/// (its marker lock) under the very id the engine issues first, so the open
/// fails at registration with a conflict, the removal finds the marker held, and
/// gives up after the (shortened) wait.
#[tokio::test]
async fn a_removal_that_cannot_proceed_keeps_the_original_failure_and_the_registration()
-> TestResult {
    let harness = Harness::with_removal_wait(std::time::Duration::from_millis(120))?;
    let store = FilesystemSessionStore::provision_default(harness.root.path("sessions"))?;
    let session = SessionId::parse("ses_00000000000000000000000000000001")?;
    let foreign_opener = OperationId::parse("op_000000000000000000000000000000ff")?;
    let held = store.register_session(&session, &foreign_opener, T0)?;
    let started = std::time::Instant::now();

    let failed = harness
        .engine
        .ingest(IngestRequest {
            source: harness.placeholder_source()?,
            transcript: None,
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Ephemeral,
        })
        .await;

    assert_eq!(
        failed.map_err(|error| error.failure_code()).map(|_| ()),
        Err(FailureCode::InvalidArgument),
        "the open's own failure, a conflict with the held registration"
    );
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(120),
        "the removal waited for the busy root: {:?}",
        started.elapsed()
    );
    assert_eq!(store.is_registered(&session), Ok(true));
    assert_eq!(registration_markers(&harness)?, 1);
    drop(held);
    Ok(())
}

/// #277 review: a cancellation after the registration is a failed open like any
/// other and removes its own registration and folder (the CHANGELOG and
/// `cli-v1.md` name it as one of the ways an open fails).
#[tokio::test]
async fn a_cancelled_ingest_leaves_nothing_behind() -> TestResult {
    let harness = Harness::new()?;
    let cancellation = Cancellation::new();
    cancellation.cancel();

    let failed = harness
        .engine
        .ingest(IngestRequest {
            source: harness.placeholder_source()?,
            transcript: None,
            cancellation,
            durability: vsift::DurabilityRequirement::Ephemeral,
        })
        .await;

    assert_eq!(
        failed.map_err(|error| error.failure_code()).map(|_| ()),
        Err(FailureCode::Cancelled)
    );
    assert!(harness.engine.list_sessions(None)?.entries().is_empty());
    assert_eq!(registration_markers(&harness)?, 0);
    assert_eq!(
        fs::read_dir(harness.root.path("sessions").join("sessions"))?.count(),
        0
    );
    Ok(())
}

/// #277 review: a session folder that exists but whose first generation was
/// never published (an opening interrupted after initialization) is not
/// published either. It keeps the code it always had, `INVALID_ARGUMENT` (the
/// lookup found a state conflict), now as the typed not-published answer.
#[tokio::test]
async fn a_folder_without_its_first_generation_is_not_published_and_stays_invalid_argument()
-> TestResult {
    let harness = Harness::new()?;
    let store = FilesystemSessionStore::provision_default(harness.root.path("sessions"))?;
    let session = SessionId::parse(FOREIGN_SESSION)?;
    let opener = OperationId::parse("op_000000000000000000000000000000ff")?;
    drop(store.register_session(&session, &opener, T0)?);
    InitializeSessionStorage::new(&store)
        .execute(InitializeSessionStorageRequest::new(
            session.clone(),
            opener,
            vsift::DurabilityRequirement::Ephemeral,
        ))
        .await?;

    let status = harness.engine.session_status(&session);

    assert_eq!(
        status,
        Err(EngineError::SessionNotPublished(
            SessionStorageError::StateConflict
        ))
    );
    assert_eq!(
        status.map_err(|error| error.failure_code()).map(|_| ()),
        Err(FailureCode::InvalidArgument)
    );
    Ok(())
}

/// Scans every bucket of the root, over and over, until told to stop, leaving
/// the root's lock free part of the time so listings and registrations also get
/// through. Returns how many buckets it scanned; any answer but `Busy` ends it.
fn spawn_scanner(
    store: FilesystemSessionStore,
    stop: Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<Result<u32, SessionStorageError>> {
    std::thread::spawn(move || {
        let mut scans = 0_u32;
        while !stop.load(Ordering::Acquire) {
            for bucket in 0..=255_u16 {
                match store.scan_session_bucket(bucket) {
                    Ok(_) => scans += 1,
                    Err(SessionStorageError::Busy) => std::thread::yield_now(),
                    Err(error) => return Err(error),
                }
                std::thread::sleep(std::time::Duration::from_micros(300));
            }
        }
        Ok(scans)
    })
}

/// One listing and one dry-run clean, each of which may end `Busy` and nothing
/// else. A registration being removed under them is shown by the listing as
/// initializing or, when its session is locked by the remover at that moment, as
/// unavailable because it is busy (documented); a clean skips it as busy or does
/// not report it at all once it is gone. Nothing may read as damage.
fn probe_listing_and_clean(engine: &Engine) -> TestResult {
    let busy = EngineError::Storage(SessionStorageError::Busy);
    match engine.list_sessions(None) {
        Ok(page) => {
            for entry in page.entries() {
                if let SessionListEntry::Unavailable { error, .. } = entry {
                    assert_eq!(error, &busy, "a listed session was unavailable");
                }
            }
        }
        Err(error) if error == busy => {}
        Err(error) => return Err(format!("a listing failed: {error:?}").into()),
    }
    let cleaning = vsift::CleanRequest {
        scope: vsift::CleanScope::Expired,
        mode: vsift::CleanMode::DryRun,
        cursor: None,
    };
    match engine.clean_sessions(cleaning) {
        Ok(page) => {
            for entry in page.entries() {
                if let vsift::CleanEntry::Skipped { error, .. } = entry {
                    assert_eq!(error, &busy, "a cleaned session was skipped");
                }
            }
        }
        Err(error) if error == busy => {}
        Err(error) => return Err(format!("a clean failed: {error:?}").into()),
    }
    Ok(())
}

/// #277 review: failed opens that remove their own registrations, listings, and
/// a scan of every bucket all run at the same time. A listing never fails
/// because a registration was removed under it (a refusal because the root's
/// lock was busy is the one answer it may give), and a failed open ends as the
/// source's `INVALID_SOURCE` or, when the root's lock was busy when it asked to
/// register (nothing was registered, so nothing is removed), as `BUSY`.
///
/// **What is left is only what `session clean` collects.** Under this much
/// contention a removal can meet a busy root on every one of its bounded tries
/// (seen in about one run in eight on Windows, where the timer is coarse); the
/// request then still reports its own failure and its registration (and the
/// folder it began, if it got that far) waits for `session clean`, as before the
/// fix. So the test does not demand an empty root at once: it demands that no
/// published session is left, and that a `session clean` after the idle interval
/// leaves nothing at all. (The uncontended case, where the root is empty at
/// once, is `a_failed_ingest_leaves_nothing_behind`.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_opens_listings_and_scans_side_by_side_leave_nothing_and_never_fail_a_listing()
-> TestResult {
    const OPENERS: usize = 3;
    const ROUNDS: usize = 10;
    let harness = Arc::new(Harness::new()?);
    let not_media = harness.sidecar("not-media.mp4", b"plain text, not a video")?;
    // The first refused ingest makes the root the scanner opens.
    assert_eq!(
        refused(&harness, &not_media).await,
        Ok(FailureCode::InvalidSource)
    );
    let scanner_store = FilesystemSessionStore::open_existing(harness.root.path("sessions"))?;
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let scanner = spawn_scanner(scanner_store, Arc::clone(&stop));

    let mut openers = Vec::new();
    for _ in 0..OPENERS {
        let harness = Arc::clone(&harness);
        let source = not_media.clone();
        openers.push(tokio::spawn(async move {
            let mut codes = Vec::new();
            for _ in 0..ROUNDS {
                codes.push(refused(&harness, &source).await);
            }
            codes
        }));
    }
    // A listing and a clean (dry-run, so it examines and removes nothing) visit
    // every bucket and each visit only tries the root's lock, so under this much
    // contention most end Busy; any other answer is a failure. A registration
    // being removed under them is not an error (see `probe_listing_and_clean`).
    let mut attempts = 0_u32;
    while !openers.iter().all(tokio::task::JoinHandle::is_finished) {
        probe_listing_and_clean(&harness.engine)?;
        attempts += 1;
        tokio::task::yield_now().await;
    }
    for opener in openers {
        for code in opener.await? {
            let code = code?;
            assert!(
                matches!(code, FailureCode::InvalidSource | FailureCode::Busy),
                "{code:?}"
            );
        }
    }
    stop.store(true, Ordering::Release);
    let scans = scanner
        .join()
        .map_err(|_| "the scanning thread panicked")?
        .map_err(|error| format!("a scan failed: {error:?}"))?;

    assert!(
        scans > 0 && attempts > 0,
        "{scans} scans, {attempts} listing attempts"
    );
    // Whatever a busy root made a removal leave is listed as initializing, never
    // as a published session.
    let listed = harness.engine.list_sessions(None)?;
    assert!(
        listed
            .entries()
            .iter()
            .all(|entry| matches!(entry, SessionListEntry::Initializing(_))),
        "{listed:?}"
    );
    // And `session clean` collects all of it once the idle interval has passed.
    harness.clock.set(T0 + vsift::SessionLifetime::IDLE_SECONDS);
    let mut cursor = None;
    loop {
        let page = harness.engine.clean_sessions(vsift::CleanRequest {
            scope: vsift::CleanScope::Expired,
            mode: vsift::CleanMode::Remove,
            cursor,
        })?;
        match page.next_cursor() {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert!(harness.engine.list_sessions(None)?.entries().is_empty());
    assert_eq!(registration_markers(&harness)?, 0);
    assert_eq!(
        fs::read_dir(harness.root.path("sessions").join("sessions"))?.count(),
        0
    );
    Ok(())
}

#[tokio::test]
async fn transcript_reads_are_typed_for_bad_ranges_and_sessions_without_one() -> TestResult {
    let harness = Harness::new()?;
    let query = |session: SessionId, from: u64, to: u64, limit: Option<u16>| TranscriptQuery {
        session,
        revision: None,
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
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Ephemeral,
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
    let TranscriptProvenance::Imported { offset, .. } = revision.provenance() else {
        return Err("an import produced another provenance".into());
    };
    assert_eq!(offset.as_micros(), 500_000);
    assert_eq!(revision.supersedes(), None);
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
            revision: None,
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
        revision: None,
        from_micros: 5_000_000,
        to_micros: 9_000_000,
        limit: None,
        cursor: None,
    })?;
    assert_eq!(cited.segments().len(), 1);
    assert_eq!(cited.segments()[0].text().markup(), CueMarkup::None);
    assert_eq!(cue_ordinal(&cited.segments()[0]), Some(2));

    // A cursor survives renewal (it is bound to the revision, not the
    // storage generation) and is refused once the session has expired.
    let first = harness.engine.transcript(TranscriptQuery {
        session: session.clone(),
        revision: None,
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
        revision: None,
        from_micros: 0,
        to_micros: 12_000_000,
        limit: Some(1),
        cursor: Some(continuation),
    })?;
    assert_eq!(cue_ordinal(&resumed.segments()[0]), Some(2));
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
                revision: None,
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
