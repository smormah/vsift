//! Capability-scoped session storage for the qualified ephemeral desktop profile.

use std::{
    error::Error,
    fmt, fs,
    io::{self, Read, Write},
    path::Path,
};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, File, Metadata, OpenOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vsift_application::{
    AuthorizedSessionStorageInitialization, SessionStorageError, SessionStore, StorageCapabilities,
};
use vsift_domain::{OperationId, PublicationGuarantee, SessionId, StorageGeneration};

const MAX_METADATA_BYTES: u64 = 64 * 1024;
const OWNERSHIP_FILE: &str = "ownership.json";
const COORDINATION_DIRECTORY: &str = "coordination";
const SESSIONS_DIRECTORY: &str = "sessions";
const INITIALIZATION_LOCK: &str = "session-initialize.lock";
const CURRENT_FILE: &str = "current.json";
const GENERATIONS_DIRECTORY: &str = "generations";
const RECORDS_DIRECTORY: &str = "records";
const ARTIFACTS_DIRECTORY: &str = "artifacts";
const ATTEMPTS_DIRECTORY: &str = "attempts";
const INITIAL_GENERATION_FILE: &str = "0.json";
const STORAGE_SCHEMA_VERSION: u16 = 1;
const STORAGE_LAYOUT_VERSION: u16 = 1;

/// Failure to open and validate an explicitly selected private storage root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionStoreOpenError {
    /// Storage roots must be explicit absolute paths.
    RootMustBeAbsolute,
    /// The selected root is unavailable or cannot be opened safely.
    RootUnavailable,
    /// The selected root is not a directory.
    RootNotDirectory,
    /// Unix permission bits allow access outside the owning user.
    RootNotPrivate,
    /// The ownership marker is absent, malformed, or not a supported `VSift` marker.
    InvalidOwnership,
    /// A required contained directory or stable lock anchor is invalid.
    InvalidLayout,
}

impl fmt::Display for SessionStoreOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::RootMustBeAbsolute => "session storage root must be an absolute path",
            Self::RootUnavailable => "session storage root is unavailable",
            Self::RootNotDirectory => "session storage root is not a directory",
            Self::RootNotPrivate => "session storage root permissions are not private",
            Self::InvalidOwnership => "session storage ownership marker is invalid",
            Self::InvalidLayout => "session storage layout is invalid",
        };
        formatter.write_str(message)
    }
}

impl Error for SessionStoreOpenError {}

/// Existing filesystem root for process-crash-consistent session publication.
///
/// Opening is read-only. Root provisioning and user-facing session lifecycle remain
/// separate operations so a durability preflight cannot create state accidentally.
/// Unix owner-only mode is verified here. Windows ACL qualification remains a P03
/// integration gate, so no CLI or worker composition may expose this adapter yet.
pub struct FilesystemSessionStore {
    root: Dir,
}

impl FilesystemSessionStore {
    /// Opens an existing explicitly selected VSift-owned root without modifying it.
    ///
    /// This is the ambient filesystem authority boundary. Every later operation is
    /// relative to the held directory capability.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the path, ownership marker, permissions, or required
    /// layout cannot be validated.
    pub fn open_existing(root_path: impl AsRef<Path>) -> Result<Self, SessionStoreOpenError> {
        let root_path = root_path.as_ref();
        if !root_path.is_absolute() {
            return Err(SessionStoreOpenError::RootMustBeAbsolute);
        }

        let metadata =
            fs::symlink_metadata(root_path).map_err(|_| SessionStoreOpenError::RootUnavailable)?;
        if metadata.file_type().is_symlink() {
            return Err(SessionStoreOpenError::RootUnavailable);
        }
        if !metadata.is_dir() {
            return Err(SessionStoreOpenError::RootNotDirectory);
        }
        #[cfg(unix)]
        validate_root_permissions(&metadata)?;

        let canonical =
            fs::canonicalize(root_path).map_err(|_| SessionStoreOpenError::RootUnavailable)?;
        let root = Dir::open_ambient_dir(canonical, cap_std::ambient_authority())
            .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
        validate_root_layout(&root)?;

        Ok(Self { root })
    }
}

impl SessionStore for FilesystemSessionStore {
    fn capabilities(&self) -> StorageCapabilities {
        StorageCapabilities::new(PublicationGuarantee::ProcessCrashConsistent)
    }

    fn initialize(
        &self,
        request: AuthorizedSessionStorageInitialization,
    ) -> impl Future<Output = Result<StorageGeneration, SessionStorageError>> + Send {
        let root = self.root.try_clone().map_err(map_storage_io);
        async move {
            let root = root?;
            tokio::task::spawn_blocking(move || initialize_session(&root, &request))
                .await
                .map_err(|_| SessionStorageError::Io)?
        }
    }
}

fn validate_root_layout(root: &Dir) -> Result<(), SessionStoreOpenError> {
    let ownership = open_regular_file(root, OWNERSHIP_FILE, false)
        .map_err(|_| SessionStoreOpenError::InvalidOwnership)?;
    let bytes = read_bounded(ownership).map_err(|_| SessionStoreOpenError::InvalidOwnership)?;
    let marker: OwnershipMarker =
        serde_json::from_slice(&bytes).map_err(|_| SessionStoreOpenError::InvalidOwnership)?;
    if marker
        != (OwnershipMarker {
            schema_version: STORAGE_SCHEMA_VERSION,
            application: String::from("vsift"),
            layout_version: STORAGE_LAYOUT_VERSION,
        })
    {
        return Err(SessionStoreOpenError::InvalidOwnership);
    }

    root.open_dir_nofollow(SESSIONS_DIRECTORY)
        .map_err(|_| SessionStoreOpenError::InvalidLayout)?;
    let coordination = root
        .open_dir_nofollow(COORDINATION_DIRECTORY)
        .map_err(|_| SessionStoreOpenError::InvalidLayout)?;
    open_regular_file(&coordination, INITIALIZATION_LOCK, true)
        .map_err(|_| SessionStoreOpenError::InvalidLayout)?;
    Ok(())
}

fn initialize_session(
    root: &Dir,
    request: &AuthorizedSessionStorageInitialization,
) -> Result<StorageGeneration, SessionStorageError> {
    let coordination = root
        .open_dir_nofollow(COORDINATION_DIRECTORY)
        .map_err(map_storage_io)?;
    let initialization_lock = open_regular_file(&coordination, INITIALIZATION_LOCK, true)
        .map_err(map_storage_io)?
        .into_std();
    initialization_lock.try_lock().map_err(map_lock_error)?;

    let result = initialize_session_while_locked(root, &coordination, request);
    drop(initialization_lock);
    result
}

fn initialize_session_while_locked(
    root: &Dir,
    coordination: &Dir,
    request: &AuthorizedSessionStorageInitialization,
) -> Result<StorageGeneration, SessionStorageError> {
    ensure_session_lock_anchor(coordination, request.session_id(), "lifetime")?;
    ensure_session_lock_anchor(coordination, request.session_id(), "writer")?;

    let sessions = root
        .open_dir_nofollow(SESSIONS_DIRECTORY)
        .map_err(map_storage_io)?;
    if sessions
        .try_exists(request.session_id().as_str())
        .map_err(map_storage_io)?
    {
        return validate_initial_session(&sessions, request.session_id(), request.operation_id());
    }

    let attempt_name = initialization_attempt_name(request.session_id(), request.operation_id());
    if sessions.try_exists(&attempt_name).map_err(map_storage_io)? {
        let generation = validate_attempt(&sessions, &attempt_name, request)?;
        sessions
            .rename(&attempt_name, &sessions, request.session_id().as_str())
            .map_err(map_storage_io)?;
        return Ok(generation);
    }

    create_initial_attempt(&sessions, &attempt_name, request)?;
    sessions
        .rename(&attempt_name, &sessions, request.session_id().as_str())
        .map_err(map_storage_io)?;
    validate_initial_session(&sessions, request.session_id(), request.operation_id())
}

fn ensure_session_lock_anchor(
    coordination: &Dir,
    session_id: &SessionId,
    role: &str,
) -> Result<(), SessionStorageError> {
    let name = format!("{}.{}.lock", session_id.as_str(), role);
    match create_regular_file(coordination, &name, b"vsift stable lock anchor\n") {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            open_regular_file(coordination, &name, true)
                .map(drop)
                .map_err(map_storage_io)
        }
        Err(error) => Err(map_storage_io(error)),
    }
}

fn create_initial_attempt(
    sessions: &Dir,
    attempt_name: &str,
    request: &AuthorizedSessionStorageInitialization,
) -> Result<(), SessionStorageError> {
    sessions.create_dir(attempt_name).map_err(map_storage_io)?;
    let attempt = sessions
        .open_dir_nofollow(attempt_name)
        .map_err(map_storage_io)?;
    for directory in [
        GENERATIONS_DIRECTORY,
        RECORDS_DIRECTORY,
        ARTIFACTS_DIRECTORY,
        ATTEMPTS_DIRECTORY,
    ] {
        attempt.create_dir(directory).map_err(map_storage_io)?;
    }

    let manifest = InitialManifest {
        schema_version: STORAGE_SCHEMA_VERSION,
        session_id: request.session_id().as_str().to_owned(),
        operation_id: request.operation_id().as_str().to_owned(),
        generation: StorageGeneration::INITIAL.value(),
    };
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|_| SessionStorageError::Io)?;
    let generations = attempt
        .open_dir_nofollow(GENERATIONS_DIRECTORY)
        .map_err(map_storage_io)?;
    create_regular_file(&generations, INITIAL_GENERATION_FILE, &manifest_bytes)
        .map_err(map_storage_io)?;

    let pointer = CommitPointer {
        schema_version: STORAGE_SCHEMA_VERSION,
        generation: StorageGeneration::INITIAL.value(),
        manifest_sha256: sha256_hex(&manifest_bytes),
    };
    let pointer_bytes = serde_json::to_vec(&pointer).map_err(|_| SessionStorageError::Io)?;
    create_regular_file(&attempt, CURRENT_FILE, &pointer_bytes).map_err(map_storage_io)
}

fn validate_attempt(
    sessions: &Dir,
    attempt_name: &str,
    request: &AuthorizedSessionStorageInitialization,
) -> Result<StorageGeneration, SessionStorageError> {
    validate_session_directory(
        sessions,
        attempt_name,
        request.session_id(),
        request.operation_id(),
    )
}

fn validate_initial_session(
    sessions: &Dir,
    session_id: &SessionId,
    operation_id: &OperationId,
) -> Result<StorageGeneration, SessionStorageError> {
    validate_session_directory(sessions, session_id.as_str(), session_id, operation_id)
}

fn validate_session_directory(
    sessions: &Dir,
    directory_name: &str,
    session_id: &SessionId,
    operation_id: &OperationId,
) -> Result<StorageGeneration, SessionStorageError> {
    let session = sessions
        .open_dir_nofollow(directory_name)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    for directory in [
        GENERATIONS_DIRECTORY,
        RECORDS_DIRECTORY,
        ARTIFACTS_DIRECTORY,
        ATTEMPTS_DIRECTORY,
    ] {
        session
            .open_dir_nofollow(directory)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
    }

    let pointer = read_json_file::<CommitPointer>(&session, CURRENT_FILE)?;
    if pointer.schema_version != STORAGE_SCHEMA_VERSION
        || pointer.generation != StorageGeneration::INITIAL.value()
        || !is_canonical_sha256(&pointer.manifest_sha256)
    {
        return Err(SessionStorageError::IntegrityFailure);
    }

    let generations = session
        .open_dir_nofollow(GENERATIONS_DIRECTORY)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let manifest_file = open_regular_file(&generations, INITIAL_GENERATION_FILE, false)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let manifest_bytes =
        read_bounded(manifest_file).map_err(|_| SessionStorageError::IntegrityFailure)?;
    if sha256_hex(&manifest_bytes) != pointer.manifest_sha256 {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let manifest: InitialManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    if manifest.schema_version != STORAGE_SCHEMA_VERSION
        || manifest.generation != StorageGeneration::INITIAL.value()
        || manifest.session_id != session_id.as_str()
    {
        return Err(SessionStorageError::IntegrityFailure);
    }
    if manifest.operation_id != operation_id.as_str() {
        return Err(SessionStorageError::StateConflict);
    }
    Ok(StorageGeneration::INITIAL)
}

fn read_json_file<T>(directory: &Dir, name: &str) -> Result<T, SessionStorageError>
where
    T: for<'de> Deserialize<'de>,
{
    let file = open_regular_file(directory, name, false)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let bytes = read_bounded(file).map_err(|_| SessionStorageError::IntegrityFailure)?;
    serde_json::from_slice(&bytes).map_err(|_| SessionStorageError::IntegrityFailure)
}

fn create_regular_file(directory: &Dir, name: &str, bytes: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let mut file = directory.open_with(name, &options)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn open_regular_file(directory: &Dir, name: &str, write: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(write).follow(FollowSymlinks::No);
    let file = directory.open_with(name, &options)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || !has_one_link(&metadata) {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(file)
}

fn read_bounded(file: File) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(MAX_METADATA_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}

fn has_one_link(metadata: &Metadata) -> bool {
    metadata.nlink() == 1
}

#[cfg(unix)]
fn validate_root_permissions(metadata: &fs::Metadata) -> Result<(), SessionStoreOpenError> {
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(SessionStoreOpenError::RootNotPrivate);
    }
    Ok(())
}

fn map_lock_error(error: fs::TryLockError) -> SessionStorageError {
    match error {
        fs::TryLockError::WouldBlock => SessionStorageError::Busy,
        fs::TryLockError::Error(error) => map_storage_io(error),
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Result::map_err requires ownership of the source error"
)]
fn map_storage_io(error: io::Error) -> SessionStorageError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => SessionStorageError::AccessDenied,
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => {
            SessionStorageError::CapacityExhausted
        }
        _ => SessionStorageError::Io,
    }
}

fn initialization_attempt_name(session_id: &SessionId, operation_id: &OperationId) -> String {
    format!(
        ".initialize-{}-{}",
        session_id.as_str(),
        operation_id.as_str()
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn is_canonical_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct OwnershipMarker {
    schema_version: u16,
    application: String,
    layout_version: u16,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InitialManifest {
    schema_version: u16,
    session_id: String,
    operation_id: String,
    generation: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommitPointer {
    schema_version: u16,
    generation: u64,
    manifest_sha256: String,
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs::{self, File as StdFile},
        io::Write,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        COORDINATION_DIRECTORY, CURRENT_FILE, FilesystemSessionStore, GENERATIONS_DIRECTORY,
        INITIALIZATION_LOCK, OWNERSHIP_FILE, SESSIONS_DIRECTORY, SessionStoreOpenError,
    };
    use vsift_application::{
        InitializeSessionStorage, InitializeSessionStorageRequest, SessionStorageError,
    };
    use vsift_domain::{DurabilityRequirement, OperationId, SessionId, StorageGeneration};

    type TestResult = Result<(), Box<dyn Error>>;

    struct Fixture {
        path: PathBuf,
    }

    impl Fixture {
        fn new() -> Result<Self, Box<dyn Error>> {
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let path = std::env::temp_dir()
                .join(format!("vsift-p03-store-{}-{stamp}", std::process::id()));
            create_private_directory(&path)?;
            create_private_directory(&path.join(COORDINATION_DIRECTORY))?;
            create_private_directory(&path.join(SESSIONS_DIRECTORY))?;
            write_new(
                &path.join(OWNERSHIP_FILE),
                br#"{"schema_version":1,"application":"vsift","layout_version":1}"#,
            )?;
            write_new(
                &path.join(COORDINATION_DIRECTORY).join(INITIALIZATION_LOCK),
                b"vsift stable lock anchor\n",
            )?;
            Ok(Self { path })
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let session_name = "ses_0123456789abcdef";
            let operation_name = "op_0123456789abcdef";
            let attempt_name = format!(".initialize-{session_name}-{operation_name}");
            for directory_name in [session_name, attempt_name.as_str()] {
                let session = self.path.join(SESSIONS_DIRECTORY).join(directory_name);
                let generations = session.join(GENERATIONS_DIRECTORY);
                let _ = fs::remove_file(generations.join("0.json"));
                let _ = fs::remove_file(session.join(CURRENT_FILE));
                let _ = fs::remove_dir(generations);
                let _ = fs::remove_dir(session.join("records"));
                let _ = fs::remove_dir(session.join("artifacts"));
                let _ = fs::remove_dir(session.join("attempts"));
                let _ = fs::remove_dir(session);
            }
            let coordination = self.path.join(COORDINATION_DIRECTORY);
            for name in [
                INITIALIZATION_LOCK.to_owned(),
                format!("{session_name}.lifetime.lock"),
                format!("{session_name}.writer.lock"),
            ] {
                let _ = fs::remove_file(coordination.join(name));
            }
            let _ = fs::remove_dir(self.path.join(SESSIONS_DIRECTORY));
            let _ = fs::remove_dir(coordination);
            let _ = fs::remove_file(self.path.join(OWNERSHIP_FILE));
            let _ = fs::remove_dir(&self.path);
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
            1
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
}
