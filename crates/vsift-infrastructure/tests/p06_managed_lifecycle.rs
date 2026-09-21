//! Native cross-process evidence for P06 managed-version use fencing.

use std::{
    env,
    error::Error,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use tar::{Builder, Header};
use vsift_domain::ArtifactIntegrity;
use vsift_infrastructure::{
    ArchiveInventoryBounds, ManagedArtifactStore, ManagedInstallGuard, ManagedRuntimeIdentity,
    ManagedVersionRemovalOutcome, PublishedManagedRuntime, ReviewedArchiveFile,
    ReviewedPayloadArchive, ReviewedRuntimeLayout,
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const CHILD_ROOT: &str = "VSIFT_P06_LIFECYCLE_CHILD_ROOT";
const CHILD_READY: &str = "VSIFT_P06_LIFECYCLE_CHILD_READY";
const COMPONENT: &str = "whisper-cli";
const FIRST_VERSION: &str = "1.9.2-linux-x64";
const SECOND_VERSION: &str = "1.9.3-linux-x64";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot {
    path: PathBuf,
}

impl OwnedRoot {
    fn new() -> TestResult<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "vsift-p06-lifecycle-{}-{stamp}-{sequence}",
            std::process::id()
        ));
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
            .is_some_and(|name| name.starts_with("vsift-p06-lifecycle-"))
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(DIGITS[usize::from(byte >> 4)]));
        result.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    result
}

fn reviewed_tar() -> Result<(Vec<u8>, ArtifactIntegrity), Box<dyn Error>> {
    let mut archive = Builder::new(Vec::new());
    let mut header = Header::new_gnu();
    header.set_path("root/tool")?;
    header.set_size(3);
    header.set_mode(0o777);
    header.set_cksum();
    archive.append(&header, Cursor::new(b"abc"))?;
    let bytes = archive.into_inner()?;
    let digest = Sha256::digest(&bytes);
    let integrity = ArtifactIntegrity::from_sha256_hex(bytes.len() as u64, &hex(&digest))?;
    Ok((bytes, integrity))
}

fn publish(
    store: &ManagedArtifactStore,
    guard: &ManagedInstallGuard,
    identity: &ManagedRuntimeIdentity,
) -> Result<PublishedManagedRuntime, Box<dyn Error>> {
    let (archive, integrity) = reviewed_tar()?;
    let artifact = store.import_verified(&archive[..], integrity)?;
    let selected = [ReviewedArchiveFile {
        path: "root/tool",
        integrity: ArtifactIntegrity::from_sha256_hex(
            3,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )?,
    }];
    let payload = artifact.stage_reviewed_payload(
        ReviewedPayloadArchive::Tar {
            max_tar_bytes: 10_000,
        },
        ArchiveInventoryBounds::new(2, 100)?,
        &[],
        &selected,
    )?;
    let mut runtime = payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
        max_bytes: 3,
        aliases: &[],
        executables: &["tool"],
    })?;
    let published = runtime.publish_and_select(guard, identity)?;
    runtime.discard()?;
    payload.discard()?;
    artifact.discard()?;
    Ok(published)
}

fn spawn_child(root: &Path, ready: &Path) -> Result<Child, Box<dyn Error>> {
    Ok(Command::new(env::current_exe()?)
        .args([
            "--exact",
            "managed_runtime_child_holds_version",
            "--ignored",
            "--nocapture",
        ])
        .env(CHILD_ROOT, root)
        .env(CHILD_READY, ready)
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
            return Err(format!("managed-runtime child exited before ready: {status}").into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err("managed-runtime child did not become ready".into())
}

fn terminate(child: &mut Child) -> TestResult {
    child.kill()?;
    let _ = child.wait()?;
    Ok(())
}

#[test]
#[ignore = "internal child entry launched by the cross-process lifecycle test"]
fn managed_runtime_child_holds_version() -> TestResult {
    let root = PathBuf::from(env::var_os(CHILD_ROOT).ok_or("missing child root")?);
    let ready = PathBuf::from(env::var_os(CHILD_READY).ok_or("missing child ready path")?);
    let store = ManagedArtifactStore::at(root)?;
    let identity = ManagedRuntimeIdentity::new(COMPONENT, FIRST_VERSION)?;
    let _runtime = store.open_published_runtime(&identity)?;
    fs::write(ready, b"ready")?;
    thread::sleep(Duration::from_secs(30));
    Ok(())
}

#[test]
fn cross_process_hold_blocks_removal_and_releases_after_process_exit() -> TestResult {
    let owned = OwnedRoot::new()?;
    let store = ManagedArtifactStore::at(owned.path.clone())?;
    let guard = store.try_install_guard()?;
    let first = ManagedRuntimeIdentity::new(COMPONENT, FIRST_VERSION)?;
    let first_hold = publish(&store, &guard, &first)?;
    let second = ManagedRuntimeIdentity::new(COMPONENT, SECOND_VERSION)?;
    let _selected_hold = publish(&store, &guard, &second)?;
    drop(first_hold);

    let ready = owned.ready_path();
    let mut child = spawn_child(&owned.path, &ready)?;
    wait_for_ready(&mut child, &ready)?;
    let held_outcome = store.remove_published_runtime(&guard, &first);
    terminate(&mut child)?;
    assert_eq!(held_outcome?, ManagedVersionRemovalOutcome::InUse);
    assert_eq!(
        store.remove_published_runtime(&guard, &first)?,
        ManagedVersionRemovalOutcome::Removed
    );
    Ok(())
}
