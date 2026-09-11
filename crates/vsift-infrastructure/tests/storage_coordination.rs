//! Native cross-process evidence for P03 stable coordination anchors.

use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use vsift_application::{InitializeSessionStorage, InitializeSessionStorageRequest};
use vsift_domain::{DurabilityRequirement, OperationId, SessionId};
use vsift_infrastructure::FilesystemSessionStore;

type TestResult = Result<(), Box<dyn Error>>;

const CHILD_ROOT: &str = "VSIFT_P03_COORDINATION_CHILD_ROOT";
const CHILD_READY: &str = "VSIFT_P03_COORDINATION_CHILD_READY";
const CHILD_MODE: &str = "VSIFT_P03_COORDINATION_CHILD_MODE";
const SESSION: &str = "ses_0123456789abcdef";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot {
    path: PathBuf,
}

impl OwnedRoot {
    fn provision() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "vsift-p03-coordination-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        FilesystemSessionStore::provision(&path, 2)?;
        Ok(Self { path })
    }

    fn ready_path(&self) -> PathBuf {
        self.path.with_extension("ready")
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.ready_path());
        if self
            .path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|name| name.starts_with("vsift-p03-coordination-"))
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn spawn_child(root: &Path, ready: &Path, mode: &str) -> Result<Child, Box<dyn Error>> {
    let executable = env::current_exe()?;
    Ok(Command::new(executable)
        .args([
            "--exact",
            "coordination_child_holds_os_lock",
            "--ignored",
            "--nocapture",
        ])
        .env(CHILD_ROOT, root)
        .env(CHILD_READY, ready)
        .env(CHILD_MODE, mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?)
}

fn wait_for_ready(child: &mut Child, ready: &Path) -> TestResult {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if ready.is_file() {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            return Err(format!("coordination child exited before ready: {status}").into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err("coordination child did not become ready".into())
}

fn terminate(child: &mut Child) -> TestResult {
    child.kill()?;
    let _ = child.wait()?;
    Ok(())
}

async fn initialize(root: &Path) -> TestResult {
    let store = FilesystemSessionStore::open_existing(root)?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            SessionId::parse(SESSION)?,
            OperationId::parse("op_0123456789abcdef")?,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    Ok(())
}

#[test]
#[ignore = "internal child entry launched by watchdog-bounded parent tests"]
fn coordination_child_holds_os_lock() -> TestResult {
    let root = PathBuf::from(env::var_os(CHILD_ROOT).ok_or("missing child root")?);
    let ready = PathBuf::from(env::var_os(CHILD_READY).ok_or("missing child ready path")?);
    let mode = env::var(CHILD_MODE)?;
    let store = FilesystemSessionStore::open_existing(root)?;
    if mode == "admission" {
        let _permit = store.try_admit(2)?;
        fs::write(ready, b"ready")?;
        thread::sleep(Duration::from_secs(30));
    } else if mode == "read" {
        let session = SessionId::parse(SESSION)?;
        let _hold = store.acquire_read(&session)?;
        fs::write(ready, b"ready")?;
        thread::sleep(Duration::from_secs(30));
    } else {
        return Err("unknown child mode".into());
    }
    Ok(())
}

#[test]
fn admission_is_root_wide_across_processes_and_releases_after_kill() -> TestResult {
    let owned = OwnedRoot::provision()?;
    let ready = owned.ready_path();
    let mut child = spawn_child(&owned.path, &ready, "admission")?;
    wait_for_ready(&mut child, &ready)?;
    let store = FilesystemSessionStore::open_existing(&owned.path)?;
    assert!(store.try_admit(1).is_err());
    terminate(&mut child)?;
    let _permit = store.try_admit(2)?;
    Ok(())
}

#[tokio::test]
async fn shared_lifetime_hold_blocks_cleanup_across_processes_and_releases_after_kill() -> TestResult
{
    let owned = OwnedRoot::provision()?;
    initialize(&owned.path).await?;
    let ready = owned.ready_path();
    let mut child = spawn_child(&owned.path, &ready, "read")?;
    wait_for_ready(&mut child, &ready)?;
    let store = FilesystemSessionStore::open_existing(&owned.path)?;
    let session = SessionId::parse(SESSION)?;
    assert!(store.try_acquire_exclusive_lifetime(&session).is_err());
    terminate(&mut child)?;
    let _exclusive = store.try_acquire_exclusive_lifetime(&session)?;
    Ok(())
}
