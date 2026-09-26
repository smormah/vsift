use std::{
    error::Error,
    fs::{self, File as StdFile},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use super::{
    ATTEMPTS_DIRECTORY, COORDINATION_DIRECTORY, CURRENT_FILE, DEFAULT_ADMISSION_CAPACITY,
    FilesystemSessionStore, GENERATIONS_DIRECTORY, INITIALIZATION_LOCK, OWNERSHIP_FILE,
    PROVISIONING_LOCK, PublicationBoundary, RootProvisioningState, SESSIONS_DIRECTORY,
    SessionStoreOpenError, map_storage_io,
    publication::{publication_boundary_name, publish_generation},
    root::admission_slot_name,
    root_provisioning_state,
};
use crate::{SessionRootError, SessionRootProvisioning, session_root::open_session_root_within};
use vsift_application::{
    InitializeSessionStorage, InitializeSessionStorageRequest, PublishSessionGeneration,
    PublishSessionGenerationRequest, SessionStorageError,
};
use vsift_domain::{DurabilityRequirement, OperationId, SessionId, StorageGeneration};

type TestResult = Result<(), Box<dyn Error>>;

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = fixture_path(stamp, sequence);
        create_private_directory(&path)?;
        create_private_directory(&path.join(COORDINATION_DIRECTORY))?;
        create_private_directory(&path.join(SESSIONS_DIRECTORY))?;
        write_new(
            &path.join(OWNERSHIP_FILE),
            br#"{"schema_version":1,"application":"vsift","layout_version":1,"admission_capacity":4}"#,
        )?;
        write_new(
            &path.join(COORDINATION_DIRECTORY).join(INITIALIZATION_LOCK),
            b"vsift stable lock anchor\n",
        )?;
        for index in 0..DEFAULT_ADMISSION_CAPACITY {
            write_new(
                &path
                    .join(COORDINATION_DIRECTORY)
                    .join(admission_slot_name(index)),
                b"vsift stable admission slot\n",
            )?;
        }
        Ok(Self { path })
    }
}

fn fixture_path(stamp: u128, sequence: u64) -> PathBuf {
    std::env::temp_dir().join(format!(
        "vsift-p03-store-{}-{stamp}-{sequence}",
        std::process::id()
    ))
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self
            .path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|name| name.starts_with("vsift-p03-store-"))
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(windows)]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    fs::DirBuilder::new().create(path)
}

fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = StdFile::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn request(
    durability: DurabilityRequirement,
) -> Result<InitializeSessionStorageRequest, vsift_domain::IdentifierError> {
    Ok(InitializeSessionStorageRequest::new(
        SessionId::parse("ses_0123456789abcdef")?,
        OperationId::parse("op_0123456789abcdef")?,
        durability,
    ))
}

fn publication_request(
    operation: &str,
    expected: u64,
    durability: DurabilityRequirement,
) -> Result<PublishSessionGenerationRequest, vsift_domain::IdentifierError> {
    Ok(PublishSessionGenerationRequest::new(
        SessionId::parse("ses_0123456789abcdef")?,
        OperationId::parse(operation)?,
        StorageGeneration::from_value(expected),
        durability,
    ))
}

#[test]
fn fixture_paths_are_distinct_when_timestamps_match() {
    assert_ne!(fixture_path(42, 0), fixture_path(42, 1));
}

#[test]
fn opening_is_read_only_and_rejects_relative_or_unowned_roots() -> TestResult {
    assert_eq!(
        FilesystemSessionStore::open_existing(Path::new("relative-root")).err(),
        Some(SessionStoreOpenError::RootMustBeAbsolute)
    );

    let fixture = Fixture::new()?;
    fs::remove_file(fixture.path.join(OWNERSHIP_FILE))?;
    assert_eq!(
        FilesystemSessionStore::open_existing(&fixture.path).err(),
        Some(SessionStoreOpenError::InvalidOwnership)
    );
    assert!(fixture.path.join(SESSIONS_DIRECTORY).is_dir());
    Ok(())
}

#[test]
fn ownership_marker_cannot_be_an_external_hard_link() -> TestResult {
    let fixture = Fixture::new()?;
    let outside = Fixture::new()?;
    let marker = fixture.path.join(OWNERSHIP_FILE);
    let outside_marker = outside.path.join(OWNERSHIP_FILE);
    let expected = fs::read(&outside_marker)?;
    fs::remove_file(&marker)?;
    fs::hard_link(&outside_marker, &marker)?;

    let result = FilesystemSessionStore::open_existing(&fixture.path);

    assert_eq!(result.err(), Some(SessionStoreOpenError::InvalidOwnership));
    assert_eq!(fs::read(outside_marker)?, expected);
    Ok(())
}

#[tokio::test]
async fn durable_initialization_fails_without_creating_session_state() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let use_case = InitializeSessionStorage::new(store);

    let result = use_case
        .execute(request(DurabilityRequirement::Durable)?)
        .await;

    assert!(matches!(
        result,
        Err(SessionStorageError::UnsupportedGuarantee { .. })
    ));
    assert_eq!(
        fs::read_dir(fixture.path.join(SESSIONS_DIRECTORY))?.count(),
        0
    );
    assert_eq!(
        fs::read_dir(fixture.path.join(COORDINATION_DIRECTORY))?.count(),
        usize::from(DEFAULT_ADMISSION_CAPACITY) + 1
    );
    Ok(())
}

#[tokio::test]
async fn ephemeral_initialization_publishes_and_recovers_generation_zero() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let use_case = InitializeSessionStorage::new(store);

    let first = use_case
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await?;
    let second = use_case
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await?;

    assert_eq!(first.generation(), StorageGeneration::INITIAL);
    assert_eq!(second.generation(), StorageGeneration::INITIAL);
    let session = fixture
        .path
        .join(SESSIONS_DIRECTORY)
        .join("ses_0123456789abcdef");
    assert!(session.join(CURRENT_FILE).is_file());
    assert!(session.join(GENERATIONS_DIRECTORY).join("0.json").is_file());
    Ok(())
}

#[tokio::test]
async fn incomplete_unpublished_initialization_attempt_is_rebuilt() -> TestResult {
    let fixture = Fixture::new()?;
    let attempt = fixture
        .path
        .join(SESSIONS_DIRECTORY)
        .join(".initialize-ses_0123456789abcdef-op_0123456789abcdef");
    create_private_directory(&attempt)?;
    create_private_directory(&attempt.join(GENERATIONS_DIRECTORY))?;
    write_new(
        &attempt.join(GENERATIONS_DIRECTORY).join("0.json"),
        b"partial",
    )?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let initialized = InitializeSessionStorage::new(store)
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await?;
    assert_eq!(initialized.generation(), StorageGeneration::INITIAL);
    assert!(
        fixture
            .path
            .join(SESSIONS_DIRECTORY)
            .join("ses_0123456789abcdef")
            .join(CURRENT_FILE)
            .is_file()
    );
    Ok(())
}

#[tokio::test]
async fn live_initialization_lock_returns_busy_without_mutation() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let initialization_lock = StdFile::options().read(true).write(true).open(
        fixture
            .path
            .join(COORDINATION_DIRECTORY)
            .join(INITIALIZATION_LOCK),
    )?;
    initialization_lock.try_lock()?;
    let use_case = InitializeSessionStorage::new(store);

    let result = use_case
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await;

    initialization_lock.unlock()?;
    assert_eq!(result, Err(SessionStorageError::Busy));
    assert_eq!(
        fs::read_dir(fixture.path.join(SESSIONS_DIRECTORY))?.count(),
        0
    );
    Ok(())
}

#[test]
fn root_initialization_lock_is_released_despite_a_surviving_duplicate() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let root_lock = store.try_root_initialization_lock()?;
    let inherited = root_lock.duplicate_descriptor()?;
    assert_eq!(
        store.try_root_initialization_lock().err(),
        Some(SessionStorageError::Busy)
    );

    root_lock.release()?;

    drop(store.try_root_initialization_lock()?);
    drop(inherited);
    Ok(())
}

/// Regression for issue #66: a descriptor inherited by a concurrently spawned
/// child must not keep a dropped session hold's lock alive.
#[tokio::test]
#[allow(
    clippy::used_underscore_binding,
    reason = "the test reaches into a private hold to simulate an inherited descriptor"
)]
async fn dropped_session_holds_release_despite_surviving_duplicates() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    InitializeSessionStorage::new(FilesystemSessionStore::open_existing(&fixture.path)?)
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;

    let reader = store.acquire_read(&session_id)?;
    let inherited_read = reader._lifetime_lock.duplicate_descriptor()?;
    drop(reader);
    let exclusive = store.try_acquire_exclusive_lifetime(&session_id)?;

    let inherited_exclusive = exclusive._lifetime_lock.duplicate_descriptor()?;
    drop(exclusive);
    drop(store.acquire_read(&session_id)?);

    drop(inherited_read);
    drop(inherited_exclusive);
    Ok(())
}

#[tokio::test]
async fn corrupt_commit_pointer_is_never_accepted_as_a_session() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let use_case = InitializeSessionStorage::new(store);
    use_case
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await?;
    let pointer = fixture
        .path
        .join(SESSIONS_DIRECTORY)
        .join("ses_0123456789abcdef")
        .join(CURRENT_FILE);
    fs::write(pointer, b"{}")?;

    let result = use_case
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await;

    assert_eq!(result, Err(SessionStorageError::IntegrityFailure));
    Ok(())
}

#[tokio::test]
async fn an_existing_session_rejects_a_different_initialization_operation() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let use_case = InitializeSessionStorage::new(store);
    use_case
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await?;
    let conflicting = InitializeSessionStorageRequest::new(
        SessionId::parse("ses_0123456789abcdef")?,
        OperationId::parse("op_fedcba9876543210")?,
        DurabilityRequirement::Ephemeral,
    );

    let result = use_case.execute(conflicting).await;

    assert_eq!(result, Err(SessionStorageError::StateConflict));
    Ok(())
}

#[test]
fn provisioning_is_exclusive_private_and_policy_bounded() -> TestResult {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = fixture_path(stamp, sequence);
    let store = FilesystemSessionStore::provision(&path, 3)?;
    assert_eq!(store.admission_capacity(), 3);
    // The transient provisioning lock is gone once the root is complete.
    assert!(
        !path
            .join(COORDINATION_DIRECTORY)
            .join(PROVISIONING_LOCK)
            .exists()
    );
    assert_eq!(
        FilesystemSessionStore::provision(&path, 3).err(),
        Some(SessionStoreOpenError::RootAlreadyExists)
    );
    assert_eq!(
        FilesystemSessionStore::provision(path.with_file_name("CON"), 3,).err(),
        Some(SessionStoreOpenError::RootUnavailable)
    );
    drop(store);
    fs::remove_dir_all(path)?;
    Ok(())
}

/// A root caught mid-provisioning: the creator's directories and held
/// provisioning lock exist, the ownership marker does not yet.
struct CreatorInProgress {
    path: PathBuf,
    lock: StdFile,
}

impl CreatorInProgress {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = fixture_path(stamp, sequence);
        create_private_directory(&path)?;
        create_private_directory(&path.join(COORDINATION_DIRECTORY))?;
        let lock = StdFile::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path.join(COORDINATION_DIRECTORY).join(PROVISIONING_LOCK))?;
        lock.try_lock()?;
        create_private_directory(&path.join(SESSIONS_DIRECTORY))?;
        Ok(Self { path, lock })
    }

    /// Writes the rest of the layout, marker last, then lets go of the lock.
    fn finish(&self) -> Result<(), Box<dyn Error>> {
        let coordination = self.path.join(COORDINATION_DIRECTORY);
        write_new(
            &coordination.join(INITIALIZATION_LOCK),
            b"vsift stable lock anchor\n",
        )?;
        for index in 0..DEFAULT_ADMISSION_CAPACITY {
            write_new(
                &coordination.join(admission_slot_name(index)),
                b"vsift stable admission slot\n",
            )?;
        }
        write_new(
            &self.path.join(OWNERSHIP_FILE),
            br#"{"schema_version":1,"application":"vsift","layout_version":1,"admission_capacity":4}"#,
        )?;
        self.lock.unlock()?;
        fs::remove_file(coordination.join(PROVISIONING_LOCK))?;
        Ok(())
    }
}

impl Drop for CreatorInProgress {
    fn drop(&mut self) {
        let _ = self.lock.unlock();
        if self
            .path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|name| name.starts_with("vsift-p03-store-"))
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn fresh_directory() -> Result<Fixture, Box<dyn Error>> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = fixture_path(stamp, sequence);
    create_private_directory(&path)?;
    Ok(Fixture { path })
}

#[test]
fn an_opener_waits_for_an_active_creator_and_then_adopts_the_root() -> TestResult {
    let creator = CreatorInProgress::new()?;
    assert_eq!(
        FilesystemSessionStore::open_existing(&creator.path).err(),
        Some(SessionStoreOpenError::InvalidOwnership)
    );
    assert_eq!(
        root_provisioning_state(&creator.path, SystemTime::now(), Duration::from_secs(10)),
        RootProvisioningState::InProgress
    );

    let opened = std::thread::scope(|scope| {
        let opener = scope.spawn(|| {
            open_session_root_within(
                &creator.path,
                SessionRootProvisioning::ExistingOnly,
                Duration::from_secs(30),
            )
        });
        std::thread::sleep(Duration::from_millis(150));
        let finished = creator.finish().map_err(|error| error.to_string());
        (finished, opener.join())
    });

    let (finished, joined) = opened;
    finished?;
    let store = joined
        .map_err(|_| "opener thread panicked")?
        .map_err(|error| error.to_string())?
        .ok_or("an existing root was reported missing")?;
    assert_eq!(store.admission_capacity(), DEFAULT_ADMISSION_CAPACITY);
    Ok(())
}

#[test]
fn an_opener_reports_busy_when_the_creator_outlasts_the_bound() -> TestResult {
    let creator = CreatorInProgress::new()?;
    let started = Instant::now();

    let result = open_session_root_within(
        &creator.path,
        SessionRootProvisioning::CreateIfMissing,
        Duration::from_millis(100),
    );

    assert!(
        matches!(result, Err(SessionRootError::ProvisioningInProgress)),
        "expected ProvisioningInProgress"
    );
    assert!(started.elapsed() >= Duration::from_millis(100));
    // Nothing was adopted or written: the marker is still absent.
    assert!(!creator.path.join(OWNERSHIP_FILE).exists());
    Ok(())
}

#[test]
fn a_creator_that_stopped_leaves_a_root_that_is_rejected_at_once() -> TestResult {
    let creator = CreatorInProgress::new()?;
    creator.lock.unlock()?;
    let started = Instant::now();

    let result = open_session_root_within(
        &creator.path,
        SessionRootProvisioning::CreateIfMissing,
        Duration::from_secs(30),
    );

    assert!(
        matches!(
            result,
            Err(SessionRootError::Store(
                SessionStoreOpenError::InvalidOwnership
            ))
        ),
        "expected InvalidOwnership"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    Ok(())
}

#[test]
fn an_empty_unmarked_directory_is_never_adopted() -> TestResult {
    let fresh = fresh_directory()?;
    // Just created, it could be a creator's first step: waited on, then rejected.
    assert_eq!(
        root_provisioning_state(&fresh.path, SystemTime::now(), Duration::from_secs(10)),
        RootProvisioningState::Starting
    );
    let started = Instant::now();
    let result = open_session_root_within(
        &fresh.path,
        SessionRootProvisioning::CreateIfMissing,
        Duration::from_millis(100),
    );
    assert!(
        matches!(
            result,
            Err(SessionRootError::Store(
                SessionStoreOpenError::InvalidOwnership
            ))
        ),
        "expected InvalidOwnership"
    );
    assert!(started.elapsed() >= Duration::from_millis(100));
    assert_eq!(fs::read_dir(&fresh.path)?.count(), 0, "nothing was written");

    // Long unchanged, it is settled at once.
    assert_eq!(
        root_provisioning_state(
            &fresh.path,
            SystemTime::now() + Duration::from_secs(3_600),
            Duration::from_secs(10),
        ),
        RootProvisioningState::Settled
    );
    Ok(())
}

/// A creator makes a new root private just after creating it, so an opener
/// that meets a fresh, empty root which is not yet private waits for it;
/// a root with content is refused at once, and neither is ever changed.
#[cfg(windows)]
#[test]
fn a_fresh_root_that_is_not_yet_private_is_waited_on_then_refused() -> TestResult {
    let fresh = fresh_directory()?;
    let system_root = std::env::var_os("SystemRoot").ok_or("SystemRoot is not set")?;
    let granted = Command::new(PathBuf::from(system_root).join("System32/icacls.exe"))
        .arg(&fresh.path)
        .args(["/grant", "*S-1-5-32-545:(RX)", "/Q"])
        .output()?
        .status;
    assert!(granted.success());

    let started = Instant::now();
    let result = open_session_root_within(
        &fresh.path,
        SessionRootProvisioning::ExistingOnly,
        Duration::from_millis(100),
    );
    assert!(matches!(
        result,
        Err(SessionRootError::Store(
            SessionStoreOpenError::RootNotPrivate
        ))
    ));
    assert!(started.elapsed() >= Duration::from_millis(100));
    assert_eq!(fs::read_dir(&fresh.path)?.count(), 0, "nothing was written");

    write_new(&fresh.path.join("notes.txt"), b"not vsift")?;
    let started = Instant::now();
    let result = open_session_root_within(
        &fresh.path,
        SessionRootProvisioning::ExistingOnly,
        Duration::from_secs(5),
    );
    assert!(matches!(
        result,
        Err(SessionRootError::Store(
            SessionStoreOpenError::RootNotPrivate
        ))
    ));
    assert!(started.elapsed() < Duration::from_secs(5));
    Ok(())
}

#[test]
fn foreign_content_is_settled_even_when_recent() -> TestResult {
    let fresh = fresh_directory()?;
    write_new(&fresh.path.join("notes.txt"), b"not vsift")?;
    assert_eq!(
        root_provisioning_state(&fresh.path, SystemTime::now(), Duration::from_secs(10)),
        RootProvisioningState::Settled
    );
    create_private_directory(&fresh.path.join("other"))?;
    fs::remove_file(fresh.path.join("notes.txt"))?;
    assert_eq!(
        root_provisioning_state(&fresh.path, SystemTime::now(), Duration::from_secs(10)),
        RootProvisioningState::Settled
    );
    Ok(())
}

#[test]
fn provisioning_rejects_parent_traversal_ads_and_invalid_capacity_before_mutation() {
    let base = std::env::temp_dir();
    let traversal = base.join("vsift-contained").join("..").join("escape");
    let ads = base.join("vsift-root:stream");
    let zero = base.join("vsift-zero-capacity");
    assert_eq!(
        FilesystemSessionStore::provision(&traversal, 1).err(),
        Some(SessionStoreOpenError::RootUnavailable)
    );
    assert_eq!(
        FilesystemSessionStore::provision(&ads, 1).err(),
        Some(SessionStoreOpenError::RootUnavailable)
    );
    assert_eq!(
        FilesystemSessionStore::provision(&zero, 0).err(),
        Some(SessionStoreOpenError::InvalidAdmissionCapacity)
    );
    assert!(!zero.exists());
}

#[cfg(windows)]
#[test]
fn case_insensitive_root_collision_is_not_adopted() -> TestResult {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = fixture_path(stamp, sequence);
    let store = FilesystemSessionStore::provision(&path, 1)?;
    let upper = PathBuf::from(path.to_string_lossy().to_uppercase());
    assert_eq!(
        FilesystemSessionStore::provision(&upper, 1).err(),
        Some(SessionStoreOpenError::RootAlreadyExists)
    );
    drop(store);
    fs::remove_dir_all(path)?;
    Ok(())
}

#[tokio::test]
async fn root_wide_weighted_admission_cannot_be_raised_by_a_request() -> TestResult {
    let fixture = Fixture::new()?;
    let first = FilesystemSessionStore::open_existing(&fixture.path)?;
    let second = FilesystemSessionStore::open_existing(&fixture.path)?;
    let permit = first.try_admit(3)?;
    assert!(
        second
            .try_admit(2)
            .is_err_and(|error| error == SessionStorageError::Busy)
    );
    assert!(
        second
            .try_admit(DEFAULT_ADMISSION_CAPACITY + 1)
            .is_err_and(|error| error == SessionStorageError::CapacityExhausted)
    );
    let remaining = second.try_admit(1)?;
    drop(permit);
    drop(remaining);
    let expanded = first.try_admit(DEFAULT_ADMISSION_CAPACITY)?;
    drop(expanded);
    Ok(())
}

#[tokio::test]
async fn shared_read_holds_exclude_cleanup_but_not_other_readers() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    InitializeSessionStorage::new(FilesystemSessionStore::open_existing(&fixture.path)?)
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    let first = store.acquire_read(&session_id)?;
    let second = store.acquire_read(&session_id)?;
    assert_eq!(first.generation(), StorageGeneration::INITIAL);
    assert_eq!(first.manifest_sha256(), second.manifest_sha256());
    assert!(
        store
            .try_acquire_exclusive_lifetime(&session_id)
            .is_err_and(|error| error == SessionStorageError::Busy)
    );
    drop(first);
    drop(second);
    let exclusive = store.try_acquire_exclusive_lifetime(&session_id)?;
    assert!(
        store
            .acquire_read(&session_id)
            .is_err_and(|error| error == SessionStorageError::Busy)
    );
    drop(exclusive);
    Ok(())
}

#[tokio::test]
async fn later_generation_is_monotonic_idempotent_and_stale_fenced() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    InitializeSessionStorage::new(FilesystemSessionStore::open_existing(&fixture.path)?)
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await?;
    let use_case = PublishSessionGeneration::new(store);

    let publish = publication_request("op_1111111111111111", 0, DurabilityRequirement::Ephemeral)?;
    assert_eq!(
        use_case.execute(publish.clone()).await?,
        StorageGeneration::from_value(1)
    );
    assert_eq!(
        use_case.execute(publish).await?,
        StorageGeneration::from_value(1)
    );
    let stale = publication_request("op_2222222222222222", 0, DurabilityRequirement::Ephemeral)?;
    assert_eq!(
        use_case.execute(stale).await,
        Err(SessionStorageError::StateConflict)
    );
    let next = publication_request("op_2222222222222222", 1, DurabilityRequirement::Ephemeral)?;
    assert_eq!(
        use_case.execute(next).await?,
        StorageGeneration::from_value(2)
    );
    Ok(())
}

#[tokio::test]
async fn durable_publication_fails_before_admission_or_mutation() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    InitializeSessionStorage::new(FilesystemSessionStore::open_existing(&fixture.path)?)
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await?;
    let session = fixture
        .path
        .join(SESSIONS_DIRECTORY)
        .join("ses_0123456789abcdef");
    let before = fs::read(session.join(CURRENT_FILE))?;
    let request = publication_request("op_3333333333333333", 0, DurabilityRequirement::Durable)?;
    assert!(matches!(
        store.publish_generation(request).await,
        Err(SessionStorageError::UnsupportedGuarantee { .. })
    ));
    assert_eq!(fs::read(session.join(CURRENT_FILE))?, before);
    assert_eq!(fs::read_dir(session.join(ATTEMPTS_DIRECTORY))?.count(), 0);
    Ok(())
}

#[tokio::test]
async fn corrupt_and_future_metadata_fail_closed() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    InitializeSessionStorage::new(FilesystemSessionStore::open_existing(&fixture.path)?)
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    let pointer = fixture
        .path
        .join(SESSIONS_DIRECTORY)
        .join(session_id.as_str())
        .join(CURRENT_FILE);
    fs::write(
        &pointer,
        br#"{"schema_version":2,"generation":0,"manifest_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
    )?;
    assert!(
        store
            .acquire_read(&session_id)
            .is_err_and(|error| error == SessionStorageError::UnsupportedVersion)
    );
    fs::write(&pointer, b"not-json")?;
    assert!(
        store
            .acquire_read(&session_id)
            .is_err_and(|error| error == SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

#[tokio::test]
async fn every_publication_fault_preserves_or_recovers_a_committed_generation() -> TestResult {
    for boundary in [
        PublicationBoundary::ManifestWrite,
        PublicationBoundary::ManifestFlush,
        PublicationBoundary::ManifestRename,
        PublicationBoundary::PointerWrite,
        PublicationBoundary::PointerFlush,
        PublicationBoundary::PointerRename,
    ] {
        let fixture = Fixture::new()?;
        let store = FilesystemSessionStore::open_existing(&fixture.path)?;
        InitializeSessionStorage::new(FilesystemSessionStore::open_existing(&fixture.path)?)
            .execute(request(DurabilityRequirement::Ephemeral)?)
            .await?;
        let publish =
            publication_request("op_4444444444444444", 0, DurabilityRequirement::Ephemeral)?;
        assert_eq!(
            publish_generation(
                &store.root,
                store.admission_capacity,
                &publish,
                Some(boundary),
            ),
            Err(SessionStorageError::Io),
            "fault at {boundary:?}"
        );
        let session_id = SessionId::parse("ses_0123456789abcdef")?;
        let recovered = store.acquire_read(&session_id)?;
        assert!(
            recovered.generation() == StorageGeneration::INITIAL
                || recovered.generation() == StorageGeneration::from_value(1),
            "invalid recovery at {boundary:?}"
        );
        drop(recovered);
        assert_eq!(
            store.publish_generation(publish).await?,
            StorageGeneration::from_value(1),
            "retry at {boundary:?}"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "internal child entry launched by the process-crash parent test"]
async fn publication_crash_child() -> TestResult {
    let root = PathBuf::from(std::env::var_os("VSIFT_P03_CRASH_ROOT").ok_or("missing crash root")?);
    let store = FilesystemSessionStore::open_existing(root)?;
    let request = publication_request("op_5555555555555555", 0, DurabilityRequirement::Ephemeral)?;
    let _ = store.publish_generation(request).await?;
    Err("crash boundary was not reached".into())
}

#[tokio::test]
async fn process_kill_at_every_publication_boundary_recovers_and_retries() -> TestResult {
    for boundary in [
        PublicationBoundary::ManifestWrite,
        PublicationBoundary::ManifestFlush,
        PublicationBoundary::ManifestRename,
        PublicationBoundary::PointerWrite,
        PublicationBoundary::PointerFlush,
        PublicationBoundary::PointerRename,
    ] {
        let fixture = Fixture::new()?;
        InitializeSessionStorage::new(FilesystemSessionStore::open_existing(&fixture.path)?)
            .execute(request(DurabilityRequirement::Ephemeral)?)
            .await?;
        let status = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "filesystem_session_store::tests::publication_crash_child",
                "--ignored",
                "--nocapture",
            ])
            .env("VSIFT_P03_CRASH_ROOT", &fixture.path)
            .env(
                "VSIFT_P03_CRASH_BOUNDARY",
                publication_boundary_name(boundary),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        assert_eq!(status.code(), Some(91), "child at {boundary:?}");

        let store = FilesystemSessionStore::open_existing(&fixture.path)?;
        let session_id = SessionId::parse("ses_0123456789abcdef")?;
        let recovered = store.acquire_read(&session_id)?;
        assert!(
            recovered.generation() == StorageGeneration::INITIAL
                || recovered.generation() == StorageGeneration::from_value(1)
        );
        drop(recovered);
        let retry =
            publication_request("op_5555555555555555", 0, DurabilityRequirement::Ephemeral)?;
        assert_eq!(
            store.publish_generation(retry).await?,
            StorageGeneration::from_value(1)
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writers_publish_once_without_corruption() -> TestResult {
    let fixture = Fixture::new()?;
    InitializeSessionStorage::new(FilesystemSessionStore::open_existing(&fixture.path)?)
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await?;
    let mut tasks = Vec::new();
    for index in 0..8_u64 {
        let path = fixture.path.clone();
        tasks.push(tokio::spawn(async move {
            let store =
                FilesystemSessionStore::open_existing(path).map_err(|_| SessionStorageError::Io)?;
            let operation = format!("op_{:016x}", index + 100);
            let request = publication_request(&operation, 0, DurabilityRequirement::Ephemeral)
                .map_err(|_| SessionStorageError::Io)?;
            let retry_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                match store.publish_generation(request.clone()).await {
                    Err(SessionStorageError::Busy)
                        if tokio::time::Instant::now() < retry_deadline =>
                    {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    result => return result,
                }
            }
        }));
    }
    let mut successes = 0;
    for task in tasks {
        match task.await? {
            Ok(generation) => {
                assert_eq!(generation, StorageGeneration::from_value(1));
                successes += 1;
            }
            Err(SessionStorageError::StateConflict) => {}
            Err(other) => return Err(format!("unexpected writer result: {other}").into()),
        }
    }
    assert_eq!(successes, 1);
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let hold = store.acquire_read(&SessionId::parse("ses_0123456789abcdef")?)?;
    assert_eq!(hold.generation(), StorageGeneration::from_value(1));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_compatible_initialization_is_idempotent() -> TestResult {
    let fixture = Fixture::new()?;
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let path = fixture.path.clone();
        tasks.push(tokio::spawn(async move {
            let store =
                FilesystemSessionStore::open_existing(path).map_err(|_| SessionStorageError::Io)?;
            let use_case = InitializeSessionStorage::new(store);
            let retry_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                match use_case
                    .execute(
                        request(DurabilityRequirement::Ephemeral)
                            .map_err(|_| SessionStorageError::Io)?,
                    )
                    .await
                {
                    Err(SessionStorageError::Busy)
                        if tokio::time::Instant::now() < retry_deadline =>
                    {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    result => return result.map(|initialized| initialized.generation()),
                }
            }
        }));
    }
    for task in tasks {
        assert_eq!(task.await??, StorageGeneration::INITIAL);
    }
    Ok(())
}

#[tokio::test]
async fn hard_linked_pointer_is_rejected_without_touching_its_peer() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    InitializeSessionStorage::new(FilesystemSessionStore::open_existing(&fixture.path)?)
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await?;
    let session = fixture
        .path
        .join(SESSIONS_DIRECTORY)
        .join("ses_0123456789abcdef");
    let pointer = session.join(CURRENT_FILE);
    let peer = session.join("pointer-peer.json");
    fs::hard_link(&pointer, &peer)?;
    let expected = fs::read(&peer)?;
    assert!(
        store
            .acquire_read(&SessionId::parse("ses_0123456789abcdef")?)
            .is_err_and(|error| error == SessionStorageError::IntegrityFailure)
    );
    assert_eq!(fs::read(peer)?, expected);
    Ok(())
}

#[tokio::test]
async fn unpublished_attempts_are_ignored_and_old_read_snapshots_remain_stable() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    InitializeSessionStorage::new(FilesystemSessionStore::open_existing(&fixture.path)?)
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    let old = store.acquire_read(&session_id)?;
    let attempts = fixture
        .path
        .join(SESSIONS_DIRECTORY)
        .join(session_id.as_str())
        .join(ATTEMPTS_DIRECTORY);
    fs::write(attempts.join("unpublished-garbage.tmp"), b"partial")?;
    let publish = publication_request("op_6666666666666666", 0, DurabilityRequirement::Ephemeral)?;
    assert_eq!(
        store.publish_generation(publish).await?,
        StorageGeneration::from_value(1)
    );
    let current = store.acquire_read(&session_id)?;
    assert_eq!(old.generation(), StorageGeneration::INITIAL);
    assert_eq!(current.generation(), StorageGeneration::from_value(1));
    assert_ne!(old.manifest_sha256(), current.manifest_sha256());
    Ok(())
}

#[tokio::test]
async fn generation_chain_tampering_and_missing_manifest_fail_closed() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    InitializeSessionStorage::new(FilesystemSessionStore::open_existing(&fixture.path)?)
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await?;
    store
        .publish_generation(publication_request(
            "op_7777777777777777",
            0,
            DurabilityRequirement::Ephemeral,
        )?)
        .await?;
    let generations = fixture
        .path
        .join(SESSIONS_DIRECTORY)
        .join("ses_0123456789abcdef")
        .join(GENERATIONS_DIRECTORY);
    let zero = generations.join("0.json");
    let mut bytes = fs::read(&zero)?;
    if let Some(first) = bytes.first_mut() {
        *first ^= 1;
    }
    fs::write(&zero, bytes)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    assert!(
        store
            .acquire_read(&session_id)
            .is_err_and(|error| error == SessionStorageError::IntegrityFailure)
    );
    fs::remove_file(generations.join("1.json"))?;
    assert!(
        store
            .acquire_read(&session_id)
            .is_err_and(|error| error == SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

#[tokio::test]
async fn exclusive_lifetime_owner_blocks_publication_without_leaking_admission() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    InitializeSessionStorage::new(FilesystemSessionStore::open_existing(&fixture.path)?)
        .execute(request(DurabilityRequirement::Ephemeral)?)
        .await?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    let exclusive = store.try_acquire_exclusive_lifetime(&session_id)?;
    let publish = publication_request("op_8888888888888888", 0, DurabilityRequirement::Ephemeral)?;
    assert_eq!(
        store.publish_generation(publish.clone()).await,
        Err(SessionStorageError::Busy)
    );
    drop(exclusive);
    assert_eq!(
        store.publish_generation(publish).await?,
        StorageGeneration::from_value(1)
    );
    let _full_capacity = store.try_admit(DEFAULT_ADMISSION_CAPACITY)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn root_permission_change_fails_closed_before_new_admission() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    fs::set_permissions(&fixture.path, fs::Permissions::from_mode(0o755))?;
    assert!(
        store
            .try_admit(1)
            .is_err_and(|error| error == SessionStorageError::AccessDenied)
    );
    fs::set_permissions(&fixture.path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(windows)]
#[test]
fn windows_acl_change_fails_closed_before_new_admission() -> TestResult {
    use windows_acl::{acl::ACL, helper::string_to_sid};
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let path = fixture.path.to_str().ok_or("fixture path is not Unicode")?;
    let mut world = string_to_sid("S-1-1-0").map_err(|code| format!("SID error {code}"))?;
    let mut acl =
        ACL::from_file_path(path, false).map_err(|code| format!("ACL open error {code}"))?;
    let changed = acl
        .allow(world.as_mut_ptr().cast(), true, 0x8000_0000)
        .map_err(|code| format!("ACL update error {code}"))?;
    assert!(changed);
    assert!(
        store
            .try_admit(1)
            .is_err_and(|error| error == SessionStorageError::AccessDenied)
    );
    Ok(())
}

#[test]
fn capacity_and_access_failures_keep_their_typed_mapping() {
    assert_eq!(
        map_storage_io(io::Error::from(io::ErrorKind::StorageFull)),
        SessionStorageError::CapacityExhausted
    );
    assert_eq!(
        map_storage_io(io::Error::from(io::ErrorKind::PermissionDenied)),
        SessionStorageError::AccessDenied
    );
    assert_eq!(
        map_storage_io(io::Error::from(io::ErrorKind::Other)),
        SessionStorageError::Io
    );
}
