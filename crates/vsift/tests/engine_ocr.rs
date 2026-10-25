//! On-screen text through the engine library (ADR 0030; the analysed-windows
//! bound is ADR 0033's background `prepare` job's).
//!
//! The session is opened with a plain `ingest` of a stand-in source, which runs
//! no provider. A call bounded to analysed windows must never start visual
//! analysis: with no window analysed it fails with its own typed error before
//! any tool runs, and leaves the session without a visual index. How such a
//! call plans a partly analysed range is pinned in `vsift-application`
//! (`a_plan_of_analysed_windows_waits_for_the_others`) and in the domain
//! (`a_plan_in_analysed_windows_leaves_out_and_counts_the_reads_awaiting_analysis`).

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use vsift::{
    Cancellation, EngineConfig, EngineError, EnginePorts, FailureCode, HostIsolation,
    IngestRequest, ManagedRootLocation, OcrError, ScreenTextRequest, ScreenTextWindows,
    SessionRootLocation, UserConfigurationLocation,
};
use vsift_infrastructure::FilesystemSessionStore;

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-engine-ocr-test-";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

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

#[tokio::test]
async fn a_call_bounded_to_analysed_windows_never_starts_visual_analysis() -> TestResult {
    let root = OwnedRoot::new()?;
    let engine = vsift::Engine::new(
        EngineConfig {
            session_root: SessionRootLocation::Explicit(root.path("sessions")),
            user_configuration: UserConfigurationLocation::Explicit(root.path("config")),
            managed_root: ManagedRootLocation::Explicit(root.path("managed")),
            host_isolation: HostIsolation::ProcessOnly,
        },
        EnginePorts::system(),
    );
    let stand_in = root.path("stand-in.mp4");
    fs::write(&stand_in, b"\0\0\0\x18ftypisomengine-ocr")?;
    let opened = engine
        .ingest(IngestRequest {
            source: stand_in,
            transcript: None,
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Ephemeral,
        })
        .await?;
    // A runner file is named so that resolution passes; it is never run here.
    let runner = root.path("vsift-ocr-runner.exe");
    fs::write(&runner, b"never run")?;
    let session = opened.session.session_id;
    let result = engine
        .read_screen_text(ScreenTextRequest {
            session: session.clone(),
            range: None,
            period_micros: None,
            max_frames: None,
            runner: Some(runner),
            threads: None,
            windows: ScreenTextWindows::AnalysedOnly,
            cancellation: Cancellation::new(),
        })
        .await;
    let Err(error) = result else {
        return Err("a call with no analysed window answered".into());
    };
    assert!(
        matches!(error, EngineError::Ocr(OcrError::WindowsNotAnalysed)),
        "{error:?}"
    );
    assert_eq!(error.failure_code(), FailureCode::InvalidArgument);
    let store = FilesystemSessionStore::open_existing(root.path("sessions"))?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    assert!(
        store.read_visual_index(&session, now)?.is_none(),
        "nothing was analysed"
    );
    assert_eq!(
        ScreenTextWindows::default(),
        ScreenTextWindows::AnalyseMissing
    );
    Ok(())
}
