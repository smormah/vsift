//! Capability-scoped session storage for the qualified ephemeral desktop profile.

use std::{
    error::Error,
    fmt, fs,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf, Prefix},
};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, DirBuilder, File, Metadata, OpenOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vsift_application::{
    AuthorizedSessionGenerationPublication, AuthorizedSessionStorageInitialization,
    PublishSessionGenerationRequest, SessionStorageError, SessionStore, StorageCapabilities,
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
const MAX_ADMISSION_CAPACITY: u16 = 64;
const DEFAULT_ADMISSION_CAPACITY: u16 = 4;
const MAX_GENERATIONS_PER_SESSION: u64 = 4_096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationBoundary {
    ManifestWrite,
    ManifestFlush,
    ManifestRename,
    PointerWrite,
    PointerFlush,
    PointerRename,
}

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
    /// The selected root already exists; provisioning never adopts or overwrites it.
    RootAlreadyExists,
    /// The immutable root admission capacity is outside the supported bound.
    InvalidAdmissionCapacity,
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
            Self::RootAlreadyExists => "session storage root already exists",
            Self::InvalidAdmissionCapacity => "session storage admission capacity is invalid",
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
    root_path: PathBuf,
    admission_capacity: u16,
}

/// Root-wide weighted permit backed by stable OS-locked slot files.
///
/// Dropping the value releases every reservation. The slot files themselves are
/// immutable anchors and are never replaced during normal operation.
pub struct FilesystemAdmissionPermit {
    _slots: Vec<fs::File>,
}

/// A committed metadata snapshot protected by a shared session lifetime hold.
pub struct SessionReadHold {
    _lifetime_lock: fs::File,
    generation: StorageGeneration,
    manifest_sha256: String,
}

impl SessionReadHold {
    /// Returns the generation that remains selected for this read operation.
    #[must_use]
    pub const fn generation(&self) -> StorageGeneration {
        self.generation
    }

    /// Returns the verified immutable manifest digest selected by the pointer.
    #[must_use]
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }
}

/// Exclusive session lifetime ownership for a future close or cleanup operation.
///
/// P03 exposes only coordination. P05 owns lifecycle state changes and deletion.
pub struct ExclusiveSessionLifetimeHold {
    _lifetime_lock: fs::File,
}

impl FilesystemSessionStore {
    /// Provisions a root with the reviewed desktop admission default.
    ///
    /// # Errors
    ///
    /// Preserves the same fail-closed behavior as [`Self::provision`].
    pub fn provision_default(root_path: impl AsRef<Path>) -> Result<Self, SessionStoreOpenError> {
        Self::provision(root_path, DEFAULT_ADMISSION_CAPACITY)
    }

    /// Provisions a new private, explicitly selected VSift-owned root.
    ///
    /// The final path component is created relative to a held canonical parent.
    /// Existing paths are never adopted or overwritten. On Windows the inherited
    /// DACL is inspected and provisioning fails unless every allow ACE belongs to
    /// the current user, `LocalSystem`, or the local Administrators group.
    ///
    /// # Errors
    ///
    /// Returns a typed error without adopting an existing or non-private root.
    pub fn provision(
        root_path: impl AsRef<Path>,
        admission_capacity: u16,
    ) -> Result<Self, SessionStoreOpenError> {
        let root_path = root_path.as_ref();
        validate_root_selection(root_path)?;
        if !(1..=MAX_ADMISSION_CAPACITY).contains(&admission_capacity) {
            return Err(SessionStoreOpenError::InvalidAdmissionCapacity);
        }
        if fs::symlink_metadata(root_path).is_ok() {
            return Err(SessionStoreOpenError::RootAlreadyExists);
        }

        let parent_path = root_path
            .parent()
            .ok_or(SessionStoreOpenError::RootUnavailable)?;
        let name = root_path
            .file_name()
            .ok_or(SessionStoreOpenError::RootUnavailable)?;
        let canonical_parent =
            fs::canonicalize(parent_path).map_err(|_| SessionStoreOpenError::RootUnavailable)?;
        let parent = Dir::open_ambient_dir(canonical_parent, cap_std::ambient_authority())
            .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
        create_private_child_directory(&parent, Path::new(name))
            .map_err(map_provision_create_error)?;
        let root = parent
            .open_dir_nofollow(Path::new(name))
            .map_err(|_| SessionStoreOpenError::RootUnavailable)?;

        let provision_result = provision_layout(&root, admission_capacity).and_then(|()| {
            let canonical =
                fs::canonicalize(root_path).map_err(|_| SessionStoreOpenError::RootUnavailable)?;
            validate_platform_root_permissions(&canonical, &root)?;
            Ok(canonical)
        });
        let canonical = match provision_result {
            Ok(canonical) => canonical,
            Err(error) => {
                rollback_unpublished_root(&root, &parent, Path::new(name), admission_capacity);
                return Err(error);
            }
        };
        validate_root_layout(&root)?;
        Ok(Self {
            root,
            root_path: canonical,
            admission_capacity,
        })
    }

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
        validate_root_selection(root_path)?;

        let metadata =
            fs::symlink_metadata(root_path).map_err(|_| SessionStoreOpenError::RootUnavailable)?;
        if metadata.file_type().is_symlink() {
            return Err(SessionStoreOpenError::RootUnavailable);
        }
        if !metadata.is_dir() {
            return Err(SessionStoreOpenError::RootNotDirectory);
        }
        let opened_path =
            fs::canonicalize(root_path).map_err(|_| SessionStoreOpenError::RootUnavailable)?;
        let root = Dir::open_ambient_dir(&opened_path, cap_std::ambient_authority())
            .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
        validate_same_object(&metadata, &root)?;
        let canonical =
            fs::canonicalize(root_path).map_err(|_| SessionStoreOpenError::RootUnavailable)?;
        if canonical != opened_path {
            return Err(SessionStoreOpenError::RootUnavailable);
        }
        validate_platform_root_permissions(&canonical, &root)?;
        let marker = validate_root_layout(&root)?;

        Ok(Self {
            root,
            root_path: canonical,
            admission_capacity: marker.admission_capacity,
        })
    }

    /// Returns the immutable, root-wide weighted admission capacity.
    #[must_use]
    pub const fn admission_capacity(&self) -> u16 {
        self.admission_capacity
    }

    fn revalidate_root(&self) -> Result<(), SessionStorageError> {
        validate_platform_root_permissions(&self.root_path, &self.root).map_err(map_open_error)?;
        let marker = validate_root_layout(&self.root).map_err(map_open_error)?;
        if marker.admission_capacity != self.admission_capacity {
            return Err(SessionStorageError::IntegrityFailure);
        }
        Ok(())
    }

    /// Attempts a root-wide weighted reservation without waiting.
    ///
    /// A request can consume but cannot alter the immutable root policy. Requests
    /// larger than the configured capacity fail as capacity errors; contention is
    /// reported as busy.
    ///
    /// # Errors
    ///
    /// Returns a typed capacity, contention, permission, integrity, or I/O error.
    pub fn try_admit(&self, weight: u16) -> Result<FilesystemAdmissionPermit, SessionStorageError> {
        if weight == 0 || weight > self.admission_capacity {
            return Err(SessionStorageError::CapacityExhausted);
        }
        self.revalidate_root()?;
        acquire_admission(&self.root, self.admission_capacity, weight)
    }

    /// Acquires a shared lifetime hold and returns one verified committed snapshot.
    ///
    /// # Errors
    ///
    /// Returns busy while cleanup owns the lifetime anchor, and fails closed on
    /// malformed, future-version, missing, or checksum-invalid metadata.
    pub fn acquire_read(
        &self,
        session_id: &SessionId,
    ) -> Result<SessionReadHold, SessionStorageError> {
        self.revalidate_root()?;
        let coordination = self
            .root
            .open_dir_nofollow(COORDINATION_DIRECTORY)
            .map_err(map_storage_io)?;
        let lifetime = open_session_lock(&coordination, session_id, "lifetime")?;
        lifetime.try_lock_shared().map_err(map_lock_error)?;
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?;
        let session = sessions
            .open_dir_nofollow(session_id.as_str())
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let committed = read_committed_manifest(&session, session_id)?;
        Ok(SessionReadHold {
            _lifetime_lock: lifetime,
            generation: StorageGeneration::from_value(committed.manifest.generation),
            manifest_sha256: committed.digest,
        })
    }

    /// Attempts exclusive lifetime ownership for a future close/cleanup transaction.
    ///
    /// # Errors
    ///
    /// Returns busy while any reader or writer retains a shared hold.
    pub fn try_acquire_exclusive_lifetime(
        &self,
        session_id: &SessionId,
    ) -> Result<ExclusiveSessionLifetimeHold, SessionStorageError> {
        self.revalidate_root()?;
        let coordination = self
            .root
            .open_dir_nofollow(COORDINATION_DIRECTORY)
            .map_err(map_storage_io)?;
        let lifetime = open_session_lock(&coordination, session_id, "lifetime")?;
        lifetime.try_lock().map_err(map_lock_error)?;
        Ok(ExclusiveSessionLifetimeHold {
            _lifetime_lock: lifetime,
        })
    }

    /// Publishes the next immutable manifest with optimistic generation fencing.
    ///
    /// Lock ordering is admission, shared lifetime, then metadata writer. No method
    /// waits while holding the writer lock. A repeated compatible operation returns
    /// its already committed generation; a stale token or conflicting operation is
    /// rejected.
    ///
    /// # Errors
    ///
    /// Durable requests fail before admission or filesystem mutation. Other typed
    /// failures preserve the previously committed generation.
    #[cfg(test)]
    async fn publish_generation(
        &self,
        request: PublishSessionGenerationRequest,
    ) -> Result<StorageGeneration, SessionStorageError> {
        if request.durability() == vsift_domain::DurabilityRequirement::Durable {
            return Err(SessionStorageError::UnsupportedGuarantee {
                requested: request.durability(),
                available: PublicationGuarantee::ProcessCrashConsistent,
            });
        }
        let root = self.root.try_clone().map_err(map_storage_io)?;
        let root_path = self.root_path.clone();
        let capacity = self.admission_capacity;
        tokio::task::spawn_blocking(move || {
            validate_platform_root_permissions(&root_path, &root).map_err(map_open_error)?;
            publish_generation(&root, capacity, &request, None)
        })
        .await
        .map_err(|_| SessionStorageError::Io)?
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
        let root_path = self.root_path.clone();
        async move {
            let root = root?;
            tokio::task::spawn_blocking(move || {
                validate_platform_root_permissions(&root_path, &root).map_err(map_open_error)?;
                initialize_session(&root, &request)
            })
            .await
            .map_err(|_| SessionStorageError::Io)?
        }
    }

    fn publish(
        &self,
        request: AuthorizedSessionGenerationPublication,
    ) -> impl Future<Output = Result<StorageGeneration, SessionStorageError>> + Send {
        let root = self.root.try_clone().map_err(map_storage_io);
        let root_path = self.root_path.clone();
        let capacity = self.admission_capacity;
        let request = PublishSessionGenerationRequest::new(
            request.session_id().clone(),
            request.operation_id().clone(),
            request.expected_generation(),
            request.durability(),
        );
        async move {
            let root = root?;
            tokio::task::spawn_blocking(move || {
                validate_platform_root_permissions(&root_path, &root).map_err(map_open_error)?;
                publish_generation(&root, capacity, &request, None)
            })
            .await
            .map_err(|_| SessionStorageError::Io)?
        }
    }
}

fn validate_root_selection(root_path: &Path) -> Result<(), SessionStoreOpenError> {
    if !root_path.is_absolute() {
        return Err(SessionStoreOpenError::RootMustBeAbsolute);
    }
    let mut components = root_path.components();
    let first = components
        .next()
        .ok_or(SessionStoreOpenError::RootUnavailable)?;
    if matches!(
        first,
        Component::Prefix(prefix)
            if !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    ) || components.any(|part| matches!(part, Component::ParentDir))
    {
        return Err(SessionStoreOpenError::RootUnavailable);
    }
    let name = root_path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or(SessionStoreOpenError::RootUnavailable)?;
    if name.is_empty()
        || name.contains(':')
        || name.ends_with(' ')
        || name.ends_with('.')
        || is_reserved_windows_name(name)
    {
        return Err(SessionStoreOpenError::RootUnavailable);
    }
    Ok(())
}

fn is_reserved_windows_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default();
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Result::map_err requires ownership of the source error"
)]
fn map_provision_create_error(error: io::Error) -> SessionStoreOpenError {
    if error.kind() == io::ErrorKind::AlreadyExists {
        SessionStoreOpenError::RootAlreadyExists
    } else {
        SessionStoreOpenError::RootUnavailable
    }
}

fn create_private_child_directory(parent: &Dir, name: &Path) -> io::Result<()> {
    #[allow(
        unused_mut,
        reason = "Unix configures the creation mode on this builder"
    )]
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use cap_std::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    parent.create_dir_with(name, &builder)
}

fn provision_layout(root: &Dir, admission_capacity: u16) -> Result<(), SessionStoreOpenError> {
    create_private_child_directory(root, Path::new(COORDINATION_DIRECTORY))
        .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
    create_private_child_directory(root, Path::new(SESSIONS_DIRECTORY))
        .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
    let coordination = root
        .open_dir_nofollow(COORDINATION_DIRECTORY)
        .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
    create_regular_file(
        &coordination,
        INITIALIZATION_LOCK,
        b"vsift stable lock anchor\n",
    )
    .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
    for index in 0..admission_capacity {
        create_regular_file(
            &coordination,
            &admission_slot_name(index),
            b"vsift stable admission slot\n",
        )
        .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
    }
    let marker = OwnershipMarker {
        schema_version: STORAGE_SCHEMA_VERSION,
        application: String::from("vsift"),
        layout_version: STORAGE_LAYOUT_VERSION,
        admission_capacity,
    };
    let marker_bytes =
        serde_json::to_vec(&marker).map_err(|_| SessionStoreOpenError::RootUnavailable)?;
    create_regular_file(root, OWNERSHIP_FILE, &marker_bytes)
        .map_err(|_| SessionStoreOpenError::RootUnavailable)
}

fn rollback_unpublished_root(root: &Dir, parent: &Dir, name: &Path, capacity: u16) {
    let _ = root.remove_file(OWNERSHIP_FILE);
    if let Ok(coordination) = root.open_dir_nofollow(COORDINATION_DIRECTORY) {
        let _ = coordination.remove_file(INITIALIZATION_LOCK);
        for index in 0..capacity.min(MAX_ADMISSION_CAPACITY) {
            let _ = coordination.remove_file(admission_slot_name(index));
        }
    }
    let _ = root.remove_dir(SESSIONS_DIRECTORY);
    let _ = root.remove_dir(COORDINATION_DIRECTORY);
    let _ = parent.remove_dir(name);
}

fn admission_slot_name(index: u16) -> String {
    format!("admission-{index:03}.lock")
}

#[cfg(unix)]
fn validate_same_object(
    path_metadata: &fs::Metadata,
    root: &Dir,
) -> Result<(), SessionStoreOpenError> {
    let held_metadata = root
        .dir_metadata()
        .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
    if path_metadata.dev() != held_metadata.dev() || path_metadata.ino() != held_metadata.ino() {
        return Err(SessionStoreOpenError::RootUnavailable);
    }
    Ok(())
}

#[cfg(windows)]
fn validate_same_object(
    _path_metadata: &fs::Metadata,
    root: &Dir,
) -> Result<(), SessionStoreOpenError> {
    let held_metadata = root
        .dir_metadata()
        .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
    if !held_metadata.is_dir() {
        return Err(SessionStoreOpenError::RootUnavailable);
    }
    Ok(())
}

#[cfg(unix)]
fn validate_platform_root_permissions(
    _root_path: &Path,
    root: &Dir,
) -> Result<(), SessionStoreOpenError> {
    use cap_std::fs::PermissionsExt;
    let metadata = root
        .dir_metadata()
        .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
    if metadata.permissions().mode() & 0o077 != 0
        || cap_std::fs::MetadataExt::uid(&metadata) != rustix::process::getuid().as_raw()
    {
        return Err(SessionStoreOpenError::RootNotPrivate);
    }
    Ok(())
}

#[cfg(windows)]
fn validate_platform_root_permissions(
    root_path: &Path,
    _root: &Dir,
) -> Result<(), SessionStoreOpenError> {
    use windows_acl::{
        acl::{ACL, AceType},
        helper::{current_user, name_to_sid, sid_to_string},
    };

    let username = current_user().ok_or(SessionStoreOpenError::RootNotPrivate)?;
    let user_sid = name_to_sid(&username, None)
        .ok()
        .and_then(|mut sid| sid_to_string(sid.as_mut_ptr().cast()).ok())
        .ok_or(SessionStoreOpenError::RootNotPrivate)?;
    let path = root_path
        .to_str()
        .ok_or(SessionStoreOpenError::RootNotPrivate)?;
    let acl =
        ACL::from_file_path(path, false).map_err(|_| SessionStoreOpenError::RootNotPrivate)?;
    let entries = acl
        .all()
        .map_err(|_| SessionStoreOpenError::RootNotPrivate)?;
    let trusted = [user_sid.as_str(), "S-1-5-18", "S-1-5-32-544"];
    let current_user_allowed = entries.iter().any(|entry| {
        matches!(
            entry.entry_type,
            AceType::AccessAllow
                | AceType::AccessAllowCallback
                | AceType::AccessAllowObject
                | AceType::AccessAllowCallbackObject
        ) && entry.string_sid == user_sid
    });
    let unsafe_entry = entries.iter().any(|entry| {
        entry.entry_type == AceType::Unknown
            || (matches!(
                entry.entry_type,
                AceType::AccessAllow
                    | AceType::AccessAllowCallback
                    | AceType::AccessAllowObject
                    | AceType::AccessAllowCallbackObject
            ) && !trusted.contains(&entry.string_sid.as_str()))
    });
    if !current_user_allowed || unsafe_entry {
        return Err(SessionStoreOpenError::RootNotPrivate);
    }
    Ok(())
}

fn map_open_error(error: SessionStoreOpenError) -> SessionStorageError {
    match error {
        SessionStoreOpenError::RootNotPrivate => SessionStorageError::AccessDenied,
        SessionStoreOpenError::InvalidOwnership | SessionStoreOpenError::InvalidLayout => {
            SessionStorageError::IntegrityFailure
        }
        _ => SessionStorageError::Io,
    }
}

fn acquire_admission(
    root: &Dir,
    capacity: u16,
    weight: u16,
) -> Result<FilesystemAdmissionPermit, SessionStorageError> {
    let coordination = root
        .open_dir_nofollow(COORDINATION_DIRECTORY)
        .map_err(map_storage_io)?;
    let mut slots = Vec::with_capacity(usize::from(weight));
    for index in 0..capacity {
        let slot = open_regular_file(&coordination, &admission_slot_name(index), true)
            .map_err(map_storage_io)?
            .into_std();
        match slot.try_lock() {
            Ok(()) => slots.push(slot),
            Err(fs::TryLockError::WouldBlock) => {}
            Err(fs::TryLockError::Error(error)) => return Err(map_storage_io(error)),
        }
        if slots.len() == usize::from(weight) {
            return Ok(FilesystemAdmissionPermit { _slots: slots });
        }
    }
    Err(SessionStorageError::Busy)
}

fn open_session_lock(
    coordination: &Dir,
    session_id: &SessionId,
    role: &str,
) -> Result<fs::File, SessionStorageError> {
    open_regular_file(
        coordination,
        &format!("{}.{}.lock", session_id.as_str(), role),
        true,
    )
    .map(File::into_std)
    .map_err(map_storage_io)
}

fn publish_generation(
    root: &Dir,
    capacity: u16,
    request: &PublishSessionGenerationRequest,
    fault: Option<PublicationBoundary>,
) -> Result<StorageGeneration, SessionStorageError> {
    let _admission = acquire_admission(root, capacity, 1)?;
    let coordination = root
        .open_dir_nofollow(COORDINATION_DIRECTORY)
        .map_err(map_storage_io)?;
    let lifetime = open_session_lock(&coordination, request.session_id(), "lifetime")?;
    lifetime.try_lock_shared().map_err(map_lock_error)?;
    let writer = open_session_lock(&coordination, request.session_id(), "writer")?;
    writer.try_lock().map_err(map_lock_error)?;

    let result = publish_generation_while_locked(root, request, fault);
    drop(writer);
    drop(lifetime);
    result
}

fn publish_generation_while_locked(
    root: &Dir,
    request: &PublishSessionGenerationRequest,
    fault: Option<PublicationBoundary>,
) -> Result<StorageGeneration, SessionStorageError> {
    let sessions = root
        .open_dir_nofollow(SESSIONS_DIRECTORY)
        .map_err(map_storage_io)?;
    let session = sessions
        .open_dir_nofollow(request.session_id().as_str())
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let current = read_committed_manifest(&session, request.session_id())?;
    let expected = request.expected_generation().value();

    if current.manifest.generation != expected {
        if current.manifest.generation == expected.saturating_add(1)
            && current.manifest.operation_id == request.operation_id().as_str()
        {
            return Ok(StorageGeneration::from_value(current.manifest.generation));
        }
        return Err(SessionStorageError::StateConflict);
    }
    if current.manifest.operation_id == request.operation_id().as_str() {
        return Err(SessionStorageError::StateConflict);
    }

    let next = request
        .expected_generation()
        .successor()
        .map_err(|_| SessionStorageError::CapacityExhausted)?;
    if next.value() >= MAX_GENERATIONS_PER_SESSION {
        return Err(SessionStorageError::CapacityExhausted);
    }
    let manifest = GenerationManifest {
        schema_version: STORAGE_SCHEMA_VERSION,
        session_id: request.session_id().as_str().to_owned(),
        operation_id: request.operation_id().as_str().to_owned(),
        generation: next.value(),
        previous_manifest_sha256: Some(current.digest),
    };
    let bytes = serde_json::to_vec(&manifest).map_err(|_| SessionStorageError::Io)?;
    let generations = session
        .open_dir_nofollow(GENERATIONS_DIRECTORY)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let attempts = session
        .open_dir_nofollow(ATTEMPTS_DIRECTORY)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let generation_name = format!("{}.json", next.value());
    let staged_manifest = format!(
        "{}.{}.manifest.tmp",
        request.operation_id().as_str(),
        next.value()
    );
    install_immutable_file(
        &attempts,
        &staged_manifest,
        &generations,
        &generation_name,
        &bytes,
        fault,
    )?;

    let pointer = CommitPointer {
        schema_version: STORAGE_SCHEMA_VERSION,
        generation: next.value(),
        manifest_sha256: sha256_hex(&bytes),
    };
    let pointer_bytes = serde_json::to_vec(&pointer).map_err(|_| SessionStorageError::Io)?;
    let staged_pointer = format!(
        "{}.{}.pointer.tmp",
        request.operation_id().as_str(),
        next.value()
    );
    prepare_staged_file(
        &attempts,
        &staged_pointer,
        &pointer_bytes,
        fault,
        PublicationBoundary::PointerWrite,
        PublicationBoundary::PointerFlush,
    )?;
    attempts
        .rename(&staged_pointer, &session, CURRENT_FILE)
        .map_err(map_storage_io)?;
    inject_fault(fault, PublicationBoundary::PointerRename)?;

    let committed = read_committed_manifest(&session, request.session_id())?;
    if committed.manifest.generation != next.value()
        || committed.manifest.operation_id != request.operation_id().as_str()
    {
        return Err(SessionStorageError::IntegrityFailure);
    }
    Ok(next)
}

fn install_immutable_file(
    staging_directory: &Dir,
    staged_name: &str,
    destination_directory: &Dir,
    destination_name: &str,
    bytes: &[u8],
    fault: Option<PublicationBoundary>,
) -> Result<(), SessionStorageError> {
    if destination_directory
        .try_exists(destination_name)
        .map_err(map_storage_io)?
    {
        let existing = open_regular_file(destination_directory, destination_name, false)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        if read_bounded(existing).map_err(|_| SessionStorageError::IntegrityFailure)? == bytes {
            return Ok(());
        }
        return Err(SessionStorageError::StateConflict);
    }
    prepare_staged_file(
        staging_directory,
        staged_name,
        bytes,
        fault,
        PublicationBoundary::ManifestWrite,
        PublicationBoundary::ManifestFlush,
    )?;
    staging_directory
        .rename(staged_name, destination_directory, destination_name)
        .map_err(map_storage_io)?;
    inject_fault(fault, PublicationBoundary::ManifestRename)
}

fn prepare_staged_file(
    directory: &Dir,
    name: &str,
    bytes: &[u8],
    fault: Option<PublicationBoundary>,
    write_boundary: PublicationBoundary,
    flush_boundary: PublicationBoundary,
) -> Result<(), SessionStorageError> {
    if directory.try_exists(name).map_err(map_storage_io)? {
        let existing_file = open_regular_file(directory, name, true);
        let existing = existing_file
            .as_ref()
            .ok()
            .and_then(|file| file.try_clone().ok())
            .and_then(|file| read_bounded(file).ok())
            .unwrap_or_default();
        if existing == bytes {
            existing_file
                .map_err(map_storage_io)?
                .sync_all()
                .map_err(map_storage_io)?;
            return Ok(());
        }
        directory.remove_file(name).map_err(map_storage_io)?;
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let mut file = directory
        .open_with(name, &options)
        .map_err(map_storage_io)?;
    file.write_all(bytes).map_err(map_storage_io)?;
    inject_fault(fault, write_boundary)?;
    file.sync_all().map_err(map_storage_io)?;
    inject_fault(fault, flush_boundary)
}

fn inject_fault(
    configured: Option<PublicationBoundary>,
    reached: PublicationBoundary,
) -> Result<(), SessionStorageError> {
    #[cfg(test)]
    if std::env::var("VSIFT_P03_CRASH_BOUNDARY").ok().as_deref()
        == Some(publication_boundary_name(reached))
    {
        std::process::exit(91);
    }
    if configured == Some(reached) {
        Err(SessionStorageError::Io)
    } else {
        Ok(())
    }
}

#[cfg(test)]
const fn publication_boundary_name(boundary: PublicationBoundary) -> &'static str {
    match boundary {
        PublicationBoundary::ManifestWrite => "manifest-write",
        PublicationBoundary::ManifestFlush => "manifest-flush",
        PublicationBoundary::ManifestRename => "manifest-rename",
        PublicationBoundary::PointerWrite => "pointer-write",
        PublicationBoundary::PointerFlush => "pointer-flush",
        PublicationBoundary::PointerRename => "pointer-rename",
    }
}

struct CommittedManifest {
    manifest: GenerationManifest,
    digest: String,
}

fn read_committed_manifest(
    session: &Dir,
    session_id: &SessionId,
) -> Result<CommittedManifest, SessionStorageError> {
    let pointer = read_versioned_json_file::<CommitPointer>(session, CURRENT_FILE)?;
    if !is_canonical_sha256(&pointer.manifest_sha256) {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let generations = session
        .open_dir_nofollow(GENERATIONS_DIRECTORY)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let name = format!("{}.json", pointer.generation);
    let file = open_regular_file(&generations, &name, false)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let bytes = read_bounded(file).map_err(|_| SessionStorageError::IntegrityFailure)?;
    if sha256_hex(&bytes) != pointer.manifest_sha256 {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let manifest: GenerationManifest = parse_versioned_json(&bytes)?;
    if manifest.generation != pointer.generation || manifest.session_id != session_id.as_str() {
        return Err(SessionStorageError::IntegrityFailure);
    }
    if manifest.generation == 0 {
        if manifest.previous_manifest_sha256.is_some() {
            return Err(SessionStorageError::IntegrityFailure);
        }
    } else if !manifest
        .previous_manifest_sha256
        .as_deref()
        .is_some_and(is_canonical_sha256)
    {
        return Err(SessionStorageError::IntegrityFailure);
    }
    validate_manifest_chain(&generations, session_id, &manifest)?;
    Ok(CommittedManifest {
        manifest,
        digest: pointer.manifest_sha256,
    })
}

fn validate_manifest_chain(
    generations: &Dir,
    session_id: &SessionId,
    current: &GenerationManifest,
) -> Result<(), SessionStorageError> {
    if current.generation >= MAX_GENERATIONS_PER_SESSION {
        return Err(SessionStorageError::CapacityExhausted);
    }
    let mut generation = current.generation;
    let mut expected_digest = current.previous_manifest_sha256.clone();
    while generation > 0 {
        generation -= 1;
        let expected = expected_digest.ok_or(SessionStorageError::IntegrityFailure)?;
        let file = open_regular_file(generations, &format!("{generation}.json"), false)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let bytes = read_bounded(file).map_err(|_| SessionStorageError::IntegrityFailure)?;
        if sha256_hex(&bytes) != expected {
            return Err(SessionStorageError::IntegrityFailure);
        }
        let manifest: GenerationManifest = parse_versioned_json(&bytes)?;
        if manifest.session_id != session_id.as_str() || manifest.generation != generation {
            return Err(SessionStorageError::IntegrityFailure);
        }
        expected_digest = manifest.previous_manifest_sha256;
    }
    if expected_digest.is_some() {
        return Err(SessionStorageError::IntegrityFailure);
    }
    Ok(())
}

fn read_versioned_json_file<T>(directory: &Dir, name: &str) -> Result<T, SessionStorageError>
where
    T: for<'de> Deserialize<'de> + MetadataVersion,
{
    let file = open_regular_file(directory, name, false)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let bytes = read_bounded(file).map_err(|_| SessionStorageError::IntegrityFailure)?;
    parse_versioned_json(&bytes)
}

trait MetadataVersion {
    fn schema_version(&self) -> u16;
}

fn parse_versioned_json<T>(bytes: &[u8]) -> Result<T, SessionStorageError>
where
    T: for<'de> Deserialize<'de> + MetadataVersion,
{
    #[derive(Deserialize)]
    struct VersionProbe {
        schema_version: u16,
    }
    let probe: VersionProbe =
        serde_json::from_slice(bytes).map_err(|_| SessionStorageError::IntegrityFailure)?;
    if probe.schema_version > STORAGE_SCHEMA_VERSION {
        return Err(SessionStorageError::UnsupportedVersion);
    }
    let value: T =
        serde_json::from_slice(bytes).map_err(|_| SessionStorageError::IntegrityFailure)?;
    if value.schema_version() != STORAGE_SCHEMA_VERSION {
        return Err(SessionStorageError::IntegrityFailure);
    }
    Ok(value)
}

fn validate_root_layout(root: &Dir) -> Result<OwnershipMarker, SessionStoreOpenError> {
    let ownership = open_regular_file(root, OWNERSHIP_FILE, false)
        .map_err(|_| SessionStoreOpenError::InvalidOwnership)?;
    let bytes = read_bounded(ownership).map_err(|_| SessionStoreOpenError::InvalidOwnership)?;
    let marker: OwnershipMarker =
        serde_json::from_slice(&bytes).map_err(|_| SessionStoreOpenError::InvalidOwnership)?;
    if marker.schema_version != STORAGE_SCHEMA_VERSION
        || marker.application != "vsift"
        || marker.layout_version != STORAGE_LAYOUT_VERSION
        || !(1..=MAX_ADMISSION_CAPACITY).contains(&marker.admission_capacity)
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
    for index in 0..marker.admission_capacity {
        open_regular_file(&coordination, &admission_slot_name(index), true)
            .map_err(|_| SessionStoreOpenError::InvalidLayout)?;
    }
    Ok(marker)
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
        match validate_attempt(&sessions, &attempt_name, request) {
            Ok(generation) => {
                sessions
                    .rename(&attempt_name, &sessions, request.session_id().as_str())
                    .map_err(map_storage_io)?;
                return Ok(generation);
            }
            Err(SessionStorageError::IntegrityFailure) => {
                discard_incomplete_initial_attempt(&sessions, &attempt_name)?;
            }
            Err(error) => return Err(error),
        }
    }

    create_initial_attempt(&sessions, &attempt_name, request)?;
    sessions
        .rename(&attempt_name, &sessions, request.session_id().as_str())
        .map_err(map_storage_io)?;
    validate_initial_session(&sessions, request.session_id(), request.operation_id())
}

fn discard_incomplete_initial_attempt(
    sessions: &Dir,
    attempt_name: &str,
) -> Result<(), SessionStorageError> {
    let attempt = sessions
        .open_dir_nofollow(attempt_name)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let _ = attempt.remove_file(CURRENT_FILE);
    for directory_name in [
        GENERATIONS_DIRECTORY,
        RECORDS_DIRECTORY,
        ARTIFACTS_DIRECTORY,
        ATTEMPTS_DIRECTORY,
    ] {
        if let Ok(directory) = attempt.open_dir_nofollow(directory_name) {
            if directory_name == GENERATIONS_DIRECTORY {
                let _ = directory.remove_file(INITIAL_GENERATION_FILE);
            }
            drop(directory);
            attempt
                .remove_dir(directory_name)
                .map_err(|_| SessionStorageError::IntegrityFailure)?;
        }
    }
    drop(attempt);
    sessions
        .remove_dir(attempt_name)
        .map_err(|_| SessionStorageError::IntegrityFailure)
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

    let manifest = GenerationManifest {
        schema_version: STORAGE_SCHEMA_VERSION,
        session_id: request.session_id().as_str().to_owned(),
        operation_id: request.operation_id().as_str().to_owned(),
        generation: StorageGeneration::INITIAL.value(),
        previous_manifest_sha256: None,
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

    let committed = read_committed_manifest(&session, session_id)?;
    let manifest = committed.manifest;
    if manifest.generation != StorageGeneration::INITIAL.value() {
        return Err(SessionStorageError::IntegrityFailure);
    }
    if manifest.operation_id != operation_id.as_str() {
        return Err(SessionStorageError::StateConflict);
    }
    Ok(StorageGeneration::INITIAL)
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

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct OwnershipMarker {
    schema_version: u16,
    application: String,
    layout_version: u16,
    admission_capacity: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GenerationManifest {
    schema_version: u16,
    session_id: String,
    operation_id: String,
    generation: u64,
    previous_manifest_sha256: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommitPointer {
    schema_version: u16,
    generation: u64,
    manifest_sha256: String,
}

impl MetadataVersion for GenerationManifest {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }
}

impl MetadataVersion for CommitPointer {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs::{self, File as StdFile},
        io::{self, Write},
        path::{Path, PathBuf},
        process::{Command, Stdio},
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        ATTEMPTS_DIRECTORY, COORDINATION_DIRECTORY, CURRENT_FILE, DEFAULT_ADMISSION_CAPACITY,
        FilesystemSessionStore, GENERATIONS_DIRECTORY, INITIALIZATION_LOCK, OWNERSHIP_FILE,
        PublicationBoundary, SESSIONS_DIRECTORY, SessionStoreOpenError, admission_slot_name,
        map_storage_io, publication_boundary_name, publish_generation,
    };
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

        let publish =
            publication_request("op_1111111111111111", 0, DurabilityRequirement::Ephemeral)?;
        assert_eq!(
            use_case.execute(publish.clone()).await?,
            StorageGeneration::from_value(1)
        );
        assert_eq!(
            use_case.execute(publish).await?,
            StorageGeneration::from_value(1)
        );
        let stale =
            publication_request("op_2222222222222222", 0, DurabilityRequirement::Ephemeral)?;
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
        let request =
            publication_request("op_3333333333333333", 0, DurabilityRequirement::Durable)?;
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
        let root =
            PathBuf::from(std::env::var_os("VSIFT_P03_CRASH_ROOT").ok_or("missing crash root")?);
        let store = FilesystemSessionStore::open_existing(root)?;
        let request =
            publication_request("op_5555555555555555", 0, DurabilityRequirement::Ephemeral)?;
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
                let store = FilesystemSessionStore::open_existing(path)
                    .map_err(|_| SessionStorageError::Io)?;
                let operation = format!("op_{:016x}", index + 100);
                let request = publication_request(&operation, 0, DurabilityRequirement::Ephemeral)
                    .map_err(|_| SessionStorageError::Io)?;
                for _ in 0..100 {
                    match store.publish_generation(request.clone()).await {
                        Err(SessionStorageError::Busy) => tokio::task::yield_now().await,
                        result => return result,
                    }
                }
                Err(SessionStorageError::Busy)
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
                let store = FilesystemSessionStore::open_existing(path)
                    .map_err(|_| SessionStorageError::Io)?;
                let use_case = InitializeSessionStorage::new(store);
                for _ in 0..100 {
                    match use_case
                        .execute(
                            request(DurabilityRequirement::Ephemeral)
                                .map_err(|_| SessionStorageError::Io)?,
                        )
                        .await
                    {
                        Err(SessionStorageError::Busy) => tokio::task::yield_now().await,
                        result => return result.map(|initialized| initialized.generation()),
                    }
                }
                Err(SessionStorageError::Busy)
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
        let publish =
            publication_request("op_6666666666666666", 0, DurabilityRequirement::Ephemeral)?;
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
        let publish =
            publication_request("op_8888888888888888", 0, DurabilityRequirement::Ephemeral)?;
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
}
