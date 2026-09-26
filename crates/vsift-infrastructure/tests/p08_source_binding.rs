//! Bracketed source binding (issue #148): a multi-call operation hashes the
//! session's private source copy when it binds it and again before it
//! commits, and compares only the copy's on-disk identity before each
//! provider call. These regressions change the copy between calls the way a
//! same-user actor could and need no media provider.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use vsift_application::{InitializeSessionStorage, InitializeSessionStorageRequest};
use vsift_domain::{DurabilityRequirement, OperationId, SessionId, StorageGeneration};
use vsift_infrastructure::{BoundSource, FilesystemSessionStore, SourceError, SourceSnapshot};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-p08-source-binding-";
const OPENED_AT: u64 = 1_000;
const SOURCE: &[u8] = b"\0\0\0\x18ftypisomp08-source-binding-committed-bytes";

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

/// An open session whose committed source is [`SOURCE`].
struct Committed {
    _root: OwnedRoot,
    store: FilesystemSessionStore,
    session_id: SessionId,
}

impl Committed {
    async fn new() -> Built<Self> {
        let root = OwnedRoot::new()?;
        let source = root.0.join("source.mp4");
        fs::write(&source, SOURCE)?;
        let workspace = root.0.join("workspace");
        let store = FilesystemSessionStore::provision_default(&workspace)?;
        let session_id = SessionId::parse("ses_0123456789abcdef")?;
        let registration = store.register_session(
            &session_id,
            &OperationId::parse("op_0123456789abcdef")?,
            OPENED_AT,
        )?;
        InitializeSessionStorage::new(store)
            .execute(InitializeSessionStorageRequest::new(
                session_id.clone(),
                OperationId::parse("op_0123456789abcdef")?,
                DurabilityRequirement::Ephemeral,
            ))
            .await?;
        drop(registration);
        let store = FilesystemSessionStore::open_existing(&workspace)?;
        let snapshot = SourceSnapshot::stage(
            &store,
            &session_id,
            &OperationId::parse("op_1111111111111111")?,
            &source,
        )?;
        store.activate_source(
            &snapshot,
            &OperationId::parse("op_2222222222222222")?,
            StorageGeneration::INITIAL,
            OPENED_AT,
        )?;
        drop(snapshot);
        Ok(Self {
            _root: root,
            store,
            session_id,
        })
    }

    fn bind(&self) -> Result<BoundSource, SourceError> {
        BoundSource::open_committed(&self.store, &self.session_id, OPENED_AT)
    }

    fn artifact_count(&self) -> Built<usize> {
        Ok(self
            .store
            .session_status(&self.session_id)?
            .artifact_count())
    }
}

fn modified(path: &Path) -> Built<SystemTime> {
    Ok(fs::metadata(path)?.modified()?)
}

/// Sets a file's modification time through a separate handle, after any
/// write handle is closed, so no pending write can move it again.
fn restore_modified(path: &Path, time: SystemTime) -> TestResult {
    OpenOptions::new()
        .write(true)
        .open(path)?
        .set_modified(time)?;
    assert_eq!(
        modified(path)?,
        time,
        "the modification time was not restored"
    );
    Ok(())
}

/// Overwrites the copy's bytes in place with others of the same length.
fn rewrite_in_place(path: &Path) -> TestResult {
    let mut file = OpenOptions::new().write(true).open(path)?;
    file.seek(SeekFrom::Start(20))?;
    file.write_all(b"TAMPERED")?;
    file.sync_all()?;
    Ok(())
}

/// Issue #148 (a): bytes changed between two provider calls, with size and
/// modification time put back, cannot reach a commit. The closing full
/// verification fails on every platform. On Unix the next identity check
/// already fails, because the kernel's status-change time cannot be set back;
/// Windows has no such time, so there only the closing hash sees it, which
/// is the instant-in-time guarantee per-call hashing gave (ADR 0012).
#[tokio::test]
async fn rewritten_bytes_with_restored_size_and_time_fail_the_closing_verification() -> TestResult {
    let session = Committed::new().await?;
    let artifacts = session.artifact_count()?;
    let bound = session.bind()?;
    let copy = bound.snapshot().provider_path();
    let original_time = modified(&copy)?;
    bound.check_identity()?;

    // File times have the filesystem clock's granularity (a scheduler tick
    // on Linux); let it move on so the Unix change time observably changes.
    std::thread::sleep(Duration::from_millis(50));
    rewrite_in_place(&copy)?;
    restore_modified(&copy, original_time)?;
    assert_eq!(fs::metadata(&copy)?.len(), u64::try_from(SOURCE.len())?);

    let between_calls = bound.check_identity();
    if cfg!(unix) {
        assert!(matches!(between_calls, Err(SourceError::SnapshotChanged)));
    } else {
        assert!(
            between_calls.is_ok(),
            "identity changed without a change time"
        );
    }
    assert!(matches!(
        bound.release_verified(),
        Err(SourceError::SnapshotChanged)
    ));
    assert_eq!(
        session.artifact_count()?,
        artifacts,
        "something was committed"
    );
    Ok(())
}

/// Issue #148 (b): a changed size or modification time fails the next
/// identity check, before a provider reads the file.
#[tokio::test]
async fn a_changed_size_or_modification_time_fails_the_next_identity_check() -> TestResult {
    let session = Committed::new().await?;
    let bound = session.bind()?;
    let copy = bound.snapshot().provider_path();
    let original_time = modified(&copy)?;

    let mut grown = SOURCE.to_vec();
    grown.push(0);
    fs::write(&copy, &grown)?;
    restore_modified(&copy, original_time)?;
    assert!(matches!(
        bound.check_identity(),
        Err(SourceError::SnapshotChanged)
    ));

    // The same bytes again, but a different modification time.
    fs::write(&copy, SOURCE)?;
    restore_modified(&copy, original_time + Duration::from_secs(1))?;
    assert!(matches!(
        bound.check_identity(),
        Err(SourceError::SnapshotChanged)
    ));
    assert!(matches!(
        bound.release_verified(),
        Err(SourceError::SnapshotChanged)
    ));
    Ok(())
}

/// A file renamed into the copy's place is another file even when its bytes,
/// size and modification time are the committed ones: the copy is reopened
/// by name, so the file identity is compared too.
#[tokio::test]
async fn a_substituted_file_fails_the_identity_check_even_with_identical_bytes() -> TestResult {
    let session = Committed::new().await?;
    let bound = session.bind()?;
    let copy = bound.snapshot().provider_path();
    let original_time = modified(&copy)?;
    let substitute = copy.with_extension("substitute");
    fs::write(&substitute, SOURCE)?;
    restore_modified(&substitute, original_time)?;
    fs::rename(&substitute, &copy)?;
    assert!(matches!(
        bound.check_identity(),
        Err(SourceError::SnapshotChanged)
    ));
    Ok(())
}

/// A removed copy fails the identity check and the closing verification.
#[tokio::test]
async fn a_removed_copy_fails_the_bound_checks() -> TestResult {
    let session = Committed::new().await?;
    let bound = session.bind()?;
    fs::remove_file(bound.snapshot().provider_path())?;
    assert!(matches!(
        bound.check_identity(),
        Err(SourceError::SnapshotChanged)
    ));
    assert!(matches!(
        bound.release_verified(),
        Err(SourceError::SnapshotChanged)
    ));
    Ok(())
}

/// An untouched copy passes every check and is handed back for commit.
#[tokio::test]
async fn an_untouched_copy_passes_and_is_released_for_commit() -> TestResult {
    let session = Committed::new().await?;
    let bound = session.bind()?;
    for _ in 0..4 {
        bound.check_identity()?;
    }
    let snapshot = bound.release_verified()?;
    snapshot.verify()?;
    Ok(())
}

/// A copy changed before the operation binds it is refused at binding, as
/// `open_committed` always did.
#[tokio::test]
async fn a_copy_changed_before_binding_is_refused() -> TestResult {
    let session = Committed::new().await?;
    let copy = session.bind()?.snapshot().provider_path();
    let original_time = modified(&copy)?;
    rewrite_in_place(&copy)?;
    restore_modified(&copy, original_time)?;
    assert!(matches!(session.bind(), Err(SourceError::SnapshotChanged)));
    Ok(())
}
