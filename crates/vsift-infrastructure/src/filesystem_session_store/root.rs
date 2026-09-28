//! Opening, provisioning and validating the owner-private storage root, and root-wide admission.

use std::{
    fs, io,
    path::{Component, Path, Prefix},
};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use vsift_application::SessionStorageError;
use vsift_domain::{
    DurabilityRequirement, SessionId, SessionLifetimePolicy, StorageGeneration, WorkspacePolicy,
};

use super::{
    COORDINATION_DIRECTORY, ChainCheck, DEFAULT_ADMISSION_CAPACITY, ExclusiveSessionLifetimeHold,
    FilesystemAdmissionPermit, FilesystemSessionStore, INITIALIZATION_LOCK, MAX_ADMISSION_CAPACITY,
    OWNERSHIP_FILE, OwnershipMarker, PROVISIONING_LOCK, SESSIONS_DIRECTORY, STORAGE_LAYOUT_VERSION,
    STORAGE_SCHEMA_VERSION, SessionReadHold, SessionStoreOpenError, StoredWorkspacePolicy,
    VerifiedHeadCache, chain::read_committed_manifest, create_private_child_directory,
    create_regular_file, map_lock_error, map_open_error, map_storage_io, open_regular_file,
    open_session_lock, read_bounded,
};
use super::{is_storage_failure, map_committed_io};
use crate::{
    durable_profile::storage_capabilities, file_lock::HeldFileLock,
    private_user_root::restrict_new_directory,
};

impl FilesystemSessionStore {
    /// How an ordinary read validates the manifest chain: down to the
    /// session's checkpoint or this instance's last verified head (#164).
    pub(super) const fn chain_check(&self) -> ChainCheck<'_> {
        ChainCheck::Incremental(Some(&self.verified_heads))
    }

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
    /// Existing paths are never adopted or overwritten. The new root is made
    /// private before anything is written into it: owner-only mode on Unix,
    /// and on Windows its own protected DACL, so entries the parent would pass
    /// on are never inherited. The finished root is then validated: on Windows
    /// every allow ACE must belong to the current user, `LocalSystem`, or the
    /// local Administrators group.
    ///
    /// The exclusive creation of the final component elects exactly one creator
    /// among concurrent callers; the others receive
    /// [`SessionStoreOpenError::RootAlreadyExists`]. Until its ownership marker is
    /// complete the creator holds a provisioning lock inside the root, which lets
    /// [`crate::open_session_root`] wait for it instead of rejecting a root that
    /// is merely unfinished. The marker is written last, so an unmarked root is
    /// never valid.
    ///
    /// # Errors
    ///
    /// Returns a typed error without adopting an existing or non-private root.
    pub fn provision(
        root_path: impl AsRef<Path>,
        admission_capacity: u16,
    ) -> Result<Self, SessionStoreOpenError> {
        Self::provision_root(root_path.as_ref(), admission_capacity, None)
    }

    /// Provisions a new worker workspace (P11, ADR 0021 D1): a root like
    /// [`Self::provision`] whose marker also records the operator's
    /// immutable `policy`, so its sessions publish with the policy's
    /// durability and live its retention, and its admission capacity is the
    /// policy's.
    ///
    /// A durable policy is accepted only where OS-crash durability is
    /// qualified (Ubuntu 24.04 on local ext4, ADR 0010): the parent
    /// directory's filesystem is checked before anything is created, and
    /// the new root's again before its marker is written, so an unqualified
    /// host gets [`SessionStoreOpenError::DurabilityUnavailable`] and no
    /// directory.
    ///
    /// # Errors
    ///
    /// As [`Self::provision`], and [`SessionStoreOpenError::DurabilityUnavailable`].
    pub fn provision_workspace(
        root_path: impl AsRef<Path>,
        policy: WorkspacePolicy,
    ) -> Result<Self, SessionStoreOpenError> {
        Self::provision_root(
            root_path.as_ref(),
            policy.admission_capacity().get(),
            Some(policy),
        )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "The creation, privacy, qualification and rollback order is the contract"
    )]
    fn provision_root(
        root_path: &Path,
        admission_capacity: u16,
        workspace: Option<WorkspacePolicy>,
    ) -> Result<Self, SessionStoreOpenError> {
        validate_root_selection(root_path)?;
        if !(1..=MAX_ADMISSION_CAPACITY).contains(&admission_capacity) {
            return Err(SessionStoreOpenError::InvalidAdmissionCapacity);
        }
        if fs::symlink_metadata(root_path).is_ok() {
            return Err(SessionStoreOpenError::RootAlreadyExists);
        }
        let durable =
            workspace.is_some_and(|policy| policy.durability() == DurabilityRequirement::Durable);

        let parent_path = root_path
            .parent()
            .ok_or(SessionStoreOpenError::RootUnavailable)?;
        let name = root_path
            .file_name()
            .ok_or(SessionStoreOpenError::RootUnavailable)?;
        let canonical_parent =
            fs::canonicalize(parent_path).map_err(|_| SessionStoreOpenError::RootUnavailable)?;
        let parent = Dir::open_ambient_dir(&canonical_parent, cap_std::ambient_authority())
            .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
        // Checked before anything exists: the new root is a directory on
        // the parent's filesystem.
        if durable && !dir_offers_os_crash_durability(&parent) {
            return Err(SessionStoreOpenError::DurabilityUnavailable);
        }
        create_private_child_directory(&parent, Path::new(name))
            .map_err(map_provision_create_error)?;
        let root = parent
            .open_dir_nofollow(Path::new(name))
            .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
        // And again on the root itself, which could only differ if the
        // parent's filesystem changed underneath; the empty root is removed.
        if durable && !dir_offers_os_crash_durability(&root) {
            drop(root);
            let _ = parent.remove_dir(Path::new(name));
            return Err(SessionStoreOpenError::DurabilityUnavailable);
        }
        // Made private before anything is written into it, whatever the
        // parent's permissions would have passed on. The held handle pins it.
        if restrict_new_directory(&canonical_parent.join(name)).is_err() {
            drop(root);
            let _ = parent.remove_dir(Path::new(name));
            return Err(SessionStoreOpenError::RootUnavailable);
        }

        // The exclusive directory creation above makes this process the one
        // creator. Everything else it writes is covered by the provisioning lock,
        // so a concurrent opener can tell this root from an abandoned one.
        let provision_result = begin_provisioning(&root).and_then(|provisioning| {
            let layout = provision_layout(&root, admission_capacity, workspace).and_then(|()| {
                let canonical = fs::canonicalize(root_path)
                    .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
                validate_platform_root_permissions(&canonical, &root)?;
                Ok(canonical)
            });
            match layout {
                Ok(canonical) => Ok((canonical, provisioning)),
                Err(error) => {
                    // Released before rollback so the anchor can be removed.
                    drop(provisioning);
                    Err(error)
                }
            }
        });
        let (canonical, provisioning) = match provision_result {
            Ok(provisioned) => provisioned,
            Err(error) => {
                rollback_unpublished_root(&root, &parent, Path::new(name), admission_capacity);
                return Err(error);
            }
        };
        validate_root_layout(&root)?;
        finish_provisioning(&root, provisioning);
        Ok(Self {
            capabilities: storage_capabilities(&root),
            root,
            root_path: canonical,
            admission_capacity,
            workspace,
            verified_heads: VerifiedHeadCache::default(),
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
            capabilities: storage_capabilities(&root),
            root,
            root_path: canonical,
            admission_capacity: marker.admission_capacity,
            workspace: marker_workspace(&marker)?,
            verified_heads: VerifiedHeadCache::default(),
        })
    }

    /// This store as a host that cannot claim OS-crash durability would open
    /// it, so the fail-closed tests hold on every host, including a qualified
    /// Ubuntu 24.04 / ext4 one.
    #[cfg(test)]
    pub(super) fn without_durable_profile(mut self) -> Self {
        self.capabilities = vsift_application::StorageCapabilities::new(
            vsift_domain::PublicationGuarantee::ProcessCrashConsistent,
        );
        self
    }

    /// Returns the immutable, root-wide weighted admission capacity.
    #[must_use]
    pub const fn admission_capacity(&self) -> u16 {
        self.admission_capacity
    }

    /// The worker workspace policy of this root, or `None` for an ordinary
    /// desktop root.
    #[must_use]
    pub const fn workspace_policy(&self) -> Option<WorkspacePolicy> {
        self.workspace
    }

    /// Whether this root may acknowledge OS-crash-durable publication on
    /// this host (the qualified profile of ADR 0010), decided when it was
    /// opened.
    #[must_use]
    pub const fn offers_os_crash_durability(&self) -> bool {
        self.capabilities.supports(DurabilityRequirement::Durable)
    }

    /// The lifetime rules of sessions opened in this root.
    #[must_use]
    pub const fn lifetime_policy(&self) -> SessionLifetimePolicy {
        match self.workspace {
            Some(policy) => policy.lifetime_policy(),
            None => SessionLifetimePolicy::Desktop,
        }
    }

    /// Requires the root's marker to still hold the policy this store was
    /// opened with: the capacity and any workspace policy are immutable, so
    /// a marker changed underneath (raised, lowered or converted) is
    /// damage, never a new policy to adopt.
    pub(super) fn revalidate_root(&self) -> Result<(), SessionStorageError> {
        validate_platform_root_permissions(&self.root_path, &self.root).map_err(map_open_error)?;
        let marker = validate_root_layout(&self.root).map_err(map_open_error)?;
        if marker.admission_capacity != self.admission_capacity
            || marker_workspace(&marker).map_err(map_open_error)? != self.workspace
        {
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
        let lifetime =
            HeldFileLock::try_shared(open_session_lock(&coordination, session_id, "lifetime")?)
                .map_err(map_lock_error)?;
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?;
        let session = sessions
            .open_dir_nofollow(session_id.as_str())
            .map_err(map_committed_io)?;
        let committed = read_committed_manifest(&session, session_id, self.chain_check())?;
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
        let lifetime =
            HeldFileLock::try_exclusive(open_session_lock(&coordination, session_id, "lifetime")?)
                .map_err(map_lock_error)?;
        Ok(ExclusiveSessionLifetimeHold {
            _lifetime_lock: lifetime,
        })
    }
}

pub(super) fn validate_root_selection(root_path: &Path) -> Result<(), SessionStoreOpenError> {
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

pub(super) fn is_reserved_windows_name(name: &str) -> bool {
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
pub(super) fn map_provision_create_error(error: io::Error) -> SessionStoreOpenError {
    if error.kind() == io::ErrorKind::AlreadyExists {
        SessionStoreOpenError::RootAlreadyExists
    } else {
        SessionStoreOpenError::RootUnavailable
    }
}

/// Creates the coordination directory and takes the provisioning lock in it.
///
/// These are the creator's first two steps, so a concurrent opener that finds
/// the root holding nothing else knows a creator may be about to take the lock.
pub(super) fn begin_provisioning(root: &Dir) -> Result<HeldFileLock, SessionStoreOpenError> {
    create_private_child_directory(root, Path::new(COORDINATION_DIRECTORY))
        .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
    let coordination = root
        .open_dir_nofollow(COORDINATION_DIRECTORY)
        .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let anchor = coordination
        .open_with(PROVISIONING_LOCK, &options)
        .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
    HeldFileLock::try_exclusive(anchor.into_std())
        .map_err(|_| SessionStoreOpenError::RootUnavailable)
}

/// Releases the provisioning lock of a complete, validated root and removes it.
///
/// Neither step can make the root invalid: the marker is already complete and
/// validation never reads the anchor. A failed release is still followed by the
/// file closing, and an anchor that cannot be removed stays as an inert empty
/// file, so both failures are tolerated rather than failing a usable root.
pub(super) fn finish_provisioning(root: &Dir, provisioning: HeldFileLock) {
    let _ = provisioning.release();
    if let Ok(coordination) = root.open_dir_nofollow(COORDINATION_DIRECTORY) {
        let _ = coordination.remove_file(PROVISIONING_LOCK);
    }
}

/// What a root that failed ownership validation shows about a concurrent creator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RootProvisioningState {
    /// Recently created and holding at most the creator's first two steps: the
    /// creator may not have taken the provisioning lock yet.
    Starting,
    /// A creator holds the provisioning lock.
    InProgress,
    /// No creator is at work, so the root's validation result is final.
    Settled,
}

/// Classifies a root that failed ownership or layout validation.
///
/// The answer only decides whether an opener waits and validates again; it never
/// authorizes use of the root, which still requires [`FilesystemSessionStore::open_existing`]
/// to pass in full. The recent-activity check reads the root's modification time,
/// which each of the creator's root-level entries refreshes; an old, empty
/// directory is therefore settled at once rather than waited on.
pub(crate) fn root_provisioning_state(
    root_path: &Path,
    now: std::time::SystemTime,
    recent: std::time::Duration,
) -> RootProvisioningState {
    let (Some(parent), Some(name)) = (root_path.parent(), root_path.file_name()) else {
        return RootProvisioningState::Settled;
    };
    let Ok(parent) = Dir::open_ambient_dir(parent, cap_std::ambient_authority()) else {
        return RootProvisioningState::Settled;
    };
    let Ok(root) = parent.open_dir_nofollow(Path::new(name)) else {
        return RootProvisioningState::Settled;
    };
    // The first steps are checked before the lock: a creator takes the lock
    // before it adds anything beyond them, so a root seen past them with a free
    // lock has a finished (or failed) creator, never one about to start.
    let recently_modified = fs::symlink_metadata(root_path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| {
            now.duration_since(modified)
                .map_or(true, |elapsed| elapsed <= recent)
        });
    if recently_modified && holds_only_first_provisioning_steps(&root) {
        return RootProvisioningState::Starting;
    }
    if provisioning_lock_is_held(&root) {
        RootProvisioningState::InProgress
    } else {
        RootProvisioningState::Settled
    }
}

/// Whether the root is empty, or holds only an empty coordination directory or
/// one containing just the provisioning anchor. At most two entries are read.
pub(super) fn holds_only_first_provisioning_steps(root: &Dir) -> bool {
    let Some(names) = bounded_entry_names(root) else {
        return false;
    };
    match names.as_slice() {
        [] => true,
        [only] if only == COORDINATION_DIRECTORY => root
            .open_dir_nofollow(COORDINATION_DIRECTORY)
            .ok()
            .and_then(|coordination| bounded_entry_names(&coordination))
            .is_some_and(|names| names.iter().all(|name| name == PROVISIONING_LOCK)),
        _ => false,
    }
}

/// Names of a directory holding at most one entry; `None` when it holds more
/// or cannot be read.
pub(super) fn bounded_entry_names(directory: &Dir) -> Option<Vec<String>> {
    let mut names = Vec::with_capacity(1);
    for entry in directory.entries().ok()? {
        if names.len() == 1 {
            return None;
        }
        names.push(entry.ok()?.file_name().into_string().ok()?);
    }
    Some(names)
}

/// Whether another holder has the provisioning lock. Probing takes a shared
/// lock for an instant and releases it explicitly; it never blocks.
pub(super) fn provisioning_lock_is_held(root: &Dir) -> bool {
    let Ok(coordination) = root.open_dir_nofollow(COORDINATION_DIRECTORY) else {
        return false;
    };
    let Ok(anchor) = open_regular_file(&coordination, PROVISIONING_LOCK, false) else {
        return false;
    };
    match HeldFileLock::try_shared(anchor.into_std()) {
        Ok(probe) => {
            let _ = probe.release();
            false
        }
        Err(fs::TryLockError::WouldBlock) => true,
        Err(fs::TryLockError::Error(_)) => false,
    }
}

pub(super) fn provision_layout(
    root: &Dir,
    admission_capacity: u16,
    workspace: Option<WorkspacePolicy>,
) -> Result<(), SessionStoreOpenError> {
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
        workspace: workspace.map(StoredWorkspacePolicy::from_policy),
    };
    let marker_bytes =
        serde_json::to_vec(&marker).map_err(|_| SessionStoreOpenError::RootUnavailable)?;
    create_regular_file(root, OWNERSHIP_FILE, &marker_bytes)
        .map_err(|_| SessionStoreOpenError::RootUnavailable)
}

pub(super) fn rollback_unpublished_root(root: &Dir, parent: &Dir, name: &Path, capacity: u16) {
    let _ = root.remove_file(OWNERSHIP_FILE);
    if let Ok(coordination) = root.open_dir_nofollow(COORDINATION_DIRECTORY) {
        let _ = coordination.remove_file(PROVISIONING_LOCK);
        let _ = coordination.remove_file(INITIALIZATION_LOCK);
        for index in 0..capacity.min(MAX_ADMISSION_CAPACITY) {
            let _ = coordination.remove_file(admission_slot_name(index));
        }
    }
    let _ = root.remove_dir(SESSIONS_DIRECTORY);
    let _ = root.remove_dir(COORDINATION_DIRECTORY);
    let _ = parent.remove_dir(name);
}

/// The validated workspace policy a marker records, if any. A recorded
/// policy that is out of range is damage: the marker is refused whole.
fn marker_workspace(
    marker: &OwnershipMarker,
) -> Result<Option<WorkspacePolicy>, SessionStoreOpenError> {
    marker
        .workspace
        .map(|stored| {
            stored
                .policy(marker.admission_capacity)
                .ok_or(SessionStoreOpenError::InvalidOwnership)
        })
        .transpose()
}

/// Whether a directory's filesystem offers OS-crash durability on this host.
fn dir_offers_os_crash_durability(directory: &Dir) -> bool {
    storage_capabilities(directory).supports(DurabilityRequirement::Durable)
}

pub(super) fn admission_slot_name(index: u16) -> String {
    format!("admission-{index:03}.lock")
}

#[cfg(unix)]
pub(super) fn validate_same_object(
    path_metadata: &fs::Metadata,
    root: &Dir,
) -> Result<(), SessionStoreOpenError> {
    use cap_fs_ext::MetadataExt;
    let held_metadata = root
        .dir_metadata()
        .map_err(|_| SessionStoreOpenError::RootUnavailable)?;
    if path_metadata.dev() != held_metadata.dev() || path_metadata.ino() != held_metadata.ino() {
        return Err(SessionStoreOpenError::RootUnavailable);
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn validate_same_object(
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
pub(super) fn validate_platform_root_permissions(
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
pub(super) fn validate_platform_root_permissions(
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

pub(super) fn acquire_admission(
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
        match HeldFileLock::try_exclusive(slot) {
            Ok(held) => slots.push(held),
            Err(fs::TryLockError::WouldBlock) => {}
            Err(fs::TryLockError::Error(error)) => return Err(map_storage_io(error)),
        }
        if slots.len() == usize::from(weight) {
            return Ok(FilesystemAdmissionPermit { _slots: slots });
        }
    }
    Err(SessionStorageError::Busy)
}

/// A failure to open or read the root's layout: `damage` for an unexpected
/// entry, but [`SessionStoreOpenError::RootUnavailable`] (`STORAGE_IO`) when the
/// storage itself failed (see [`super::map_committed_io`]), so a failing or
/// shut-down filesystem is never reported as a tampered root.
fn layout_error(error: &io::Error, damage: SessionStoreOpenError) -> SessionStoreOpenError {
    if is_storage_failure(error) {
        SessionStoreOpenError::RootUnavailable
    } else {
        damage
    }
}

pub(super) fn validate_root_layout(root: &Dir) -> Result<OwnershipMarker, SessionStoreOpenError> {
    let ownership = open_regular_file(root, OWNERSHIP_FILE, false)
        .map_err(|error| layout_error(&error, SessionStoreOpenError::InvalidOwnership))?;
    let bytes = read_bounded(ownership)
        .map_err(|error| layout_error(&error, SessionStoreOpenError::InvalidOwnership))?;
    let marker: OwnershipMarker =
        serde_json::from_slice(&bytes).map_err(|_| SessionStoreOpenError::InvalidOwnership)?;
    if marker.schema_version != STORAGE_SCHEMA_VERSION
        || marker.application != "vsift"
        || marker.layout_version != STORAGE_LAYOUT_VERSION
        || !(1..=MAX_ADMISSION_CAPACITY).contains(&marker.admission_capacity)
    {
        return Err(SessionStoreOpenError::InvalidOwnership);
    }
    marker_workspace(&marker)?;

    root.open_dir_nofollow(SESSIONS_DIRECTORY)
        .map_err(|error| layout_error(&error, SessionStoreOpenError::InvalidLayout))?;
    let coordination = root
        .open_dir_nofollow(COORDINATION_DIRECTORY)
        .map_err(|error| layout_error(&error, SessionStoreOpenError::InvalidLayout))?;
    open_regular_file(&coordination, INITIALIZATION_LOCK, true)
        .map_err(|error| layout_error(&error, SessionStoreOpenError::InvalidLayout))?;
    for index in 0..marker.admission_capacity {
        open_regular_file(&coordination, &admission_slot_name(index), true)
            .map_err(|error| layout_error(&error, SessionStoreOpenError::InvalidLayout))?;
    }
    Ok(marker)
}
