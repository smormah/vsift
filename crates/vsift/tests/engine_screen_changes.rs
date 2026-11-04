//! What changed on screen through the engine library (ADR 0034).
//!
//! The default tests run everywhere without `FFmpeg`: a session over a
//! stand-in source answers the request checks before any tool. One opt-in
//! test needs `FFmpeg` and `FFprobe` on `PATH` and reads the synthetic F04
//! recording, whose order 1017 changes status at 10 s while every other cell
//! stays as it was (`fixtures/corpus/manifest.json`, F04-E04):
//!
//! `cargo test -p vsift --locked --test engine_screen_changes -- --ignored`

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use vsift::{
    Cancellation, CandidatesRange, Clock, ClockError, Engine, EngineConfig, EngineError,
    EnginePorts, FrameComparison, HostIsolation, IdentifierGenerationError, IdentifierSource,
    IngestRequest, ManagedRootLocation, OperationId, ScreenChangeKind, ScreenChangesRequest,
    SessionId, SessionRootLocation, UserConfigurationLocation,
};
use vsift_infrastructure::ExecutableResolver;

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

/// 2027-01-15T08:00:00Z: an arbitrary fixed start for deterministic expiry.
const T0: u64 = 1_800_000_000;
const OWNED_PREFIX: &str = "vsift-engine-screen-changes-test-";
const SECOND: u64 = 1_000_000;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct FixedClock;

impl Clock for FixedClock {
    fn now_unix_seconds(&self) -> Result<u64, ClockError> {
        Ok(T0)
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

struct Harness {
    engine: Engine,
    session: SessionId,
    // Last: on Windows a folder something still holds open cannot be removed.
    _root: OwnedRoot,
}

impl Harness {
    async fn open(source: Option<PathBuf>) -> Built<Self> {
        let root = OwnedRoot::new()?;
        let engine = Engine::new(
            EngineConfig {
                session_root: SessionRootLocation::Explicit(root.path("sessions")),
                user_configuration: UserConfigurationLocation::Explicit(root.path("config")),
                managed_root: ManagedRootLocation::Explicit(root.path("managed")),
                host_isolation: HostIsolation::ProcessOnly,
            },
            EnginePorts::new(
                FixedClock,
                SequentialIdentifiers(Arc::new(AtomicU64::new(0))),
            ),
        );
        let source = if let Some(source) = source {
            source
        } else {
            let stand_in = root.path("stand-in.mp4");
            fs::write(&stand_in, b"\0\0\0\x18ftypisomengine-changes")?;
            stand_in
        };
        let opened = engine
            .ingest(IngestRequest {
                source,
                transcript: None,
                cancellation: Cancellation::new(),
                durability: vsift::DurabilityRequirement::Ephemeral,
            })
            .await?;
        Ok(Self {
            engine,
            session: opened.session.session_id,
            _root: root,
        })
    }

    fn request(&self, from_seconds: u64, to_seconds: u64) -> ScreenChangesRequest {
        ScreenChangesRequest {
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

#[tokio::test]
async fn request_checks_answer_before_any_tool() -> TestResult {
    let harness = Harness::open(None).await?;
    let mut reversed = harness.request(10, 10);
    reversed.range.to_micros = 5 * SECOND;
    assert_eq!(
        harness.engine.screen_changes(reversed).await.err(),
        Some(EngineError::InvalidTimeRange)
    );
    let mut too_many = harness.request(0, 10);
    too_many.limit = Some(21);
    assert_eq!(
        harness.engine.screen_changes(too_many).await.err(),
        Some(EngineError::InvalidPageLimit)
    );
    let mut none = harness.request(0, 10);
    none.limit = Some(0);
    assert_eq!(
        harness.engine.screen_changes(none).await.err(),
        Some(EngineError::InvalidPageLimit)
    );
    // A cursor for a session with no visual index cannot continue anything,
    // and must not analyse.
    let mut continued = harness.request(0, 10);
    continued.cursor = Some("not-a-cursor".to_owned());
    assert_eq!(
        harness.engine.screen_changes(continued).await.err(),
        Some(EngineError::CandidateCursorWithoutIndex)
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

/// F04: order 1017's status change at 10 s is one small region, compared
/// between two exact frames that `frame get` stored. (F04's scroll is drawn
/// with the pixels of the state before it, #159, so it is no change here.) A repeat names the same changes, frames
/// and regions without storing anything new.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn f04_a_status_change_is_one_small_region_between_two_exact_frames() -> TestResult {
    if !media_tools_on_path() {
        return Err("FFmpeg and FFprobe must be on PATH".into());
    }
    let harness = Harness::open(Some(fixture("F04.mp4"))).await?;
    let mut request = harness.request(0, 14);
    request.limit = Some(20);
    let first = harness.engine.screen_changes(request.clone()).await?;
    assert_eq!(first.next_cursor(), None);
    assert_eq!(first.resume(), None);
    assert_eq!(first.stop(), None);
    let dimensions = first.index().displayed_dimensions();
    let frame_area = u64::from(dimensions.width()) * u64::from(dimensions.height());

    let status = first
        .changes()
        .iter()
        .find(|item| {
            let change = &item.compared.change;
            change.before.as_micros() < 10 * SECOND && 10 * SECOND <= change.after.as_micros()
        })
        .ok_or("no change at the status edit")?;
    assert_eq!(status.compared.change.kind, ScreenChangeKind::Change);
    let FrameComparison::Compared(regions) = &status.compared.comparison else {
        return Err("the status change was not compared".into());
    };
    // The largest region is the status row: a small part of the frame. The
    // re-encoded text around it may add regions of a few pixels each, which
    // come after it (ADR 0034 section 3; the hosted run measures them on real
    // recordings).
    let largest = regions
        .regions
        .first()
        .ok_or("the status change has no region")?;
    let changed = u64::from(largest.rect.width()) * u64::from(largest.rect.height());
    assert!(
        changed * 20 < frame_area,
        "the status edit should change under 5% of the frame, not {changed} of {frame_area} pixels"
    );
    assert!(
        regions
            .regions
            .iter()
            .skip(1)
            .all(|region| region.changed_pixels * 10 < largest.changed_pixels)
    );
    for item in first.changes() {
        assert!(item.compared.before.time < item.compared.after.time);
        assert!(item.id.as_str().starts_with("chg_"));
    }
    assert_eq!(first.frames().len(), first.files().len());

    let again = harness.engine.screen_changes(request).await?;
    assert_eq!(again.changes(), first.changes());
    assert_eq!(again.frames(), first.frames());
    Ok(())
}
