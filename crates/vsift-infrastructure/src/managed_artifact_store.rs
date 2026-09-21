//! Positively owned private staging for reviewed, unactivated managed artifacts.

use std::{
    collections::HashSet,
    env,
    error::Error,
    fmt, fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, DirBuilder, OpenOptions};
#[cfg(unix)]
use cap_std::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use vsift_domain::ArtifactIntegrity;

use crate::{
    ArchiveInventoryBounds, ArtifactTransferError, GzipTarInventoryError, ReviewedArchiveAlias,
    ReviewedArchiveFile, TarInventoryError, XzTarInventoryError,
    archive_inventory::safe_archive_path,
    bounded_tar_inventory::safe_staging_name,
    private_user_root::{
        PrivateRootError, open_private_root_with_creation, validate_private_root,
        validate_same_held_directory,
    },
    stage_gzip_tar_selected_files, stage_tar_selected_files, stage_xz_tar_selected_files,
    transfer_verified,
    verified_artifact_transfer::StreamingArtifactVerifier,
};

const ROOT_MARKER: &str = "owner-v1";
const STAGE_MARKER: &str = "stage-v1";
const ARTIFACT: &str = "artifact.pending";
const PAYLOAD: &str = "payload.pending";
const RUNTIME: &str = "runtime.pending";
const INSTALL_LOCK: &str = "install.lock";
const VERSIONS: &str = "versions-v1";
const CURRENT: &str = "current-v1";
const DIRECTORY_MARKER: &str = "owner-v1";
const VERSION_MANIFEST: &str = "version-v1";
const VERSION_USE_LOCK: &str = "use.lock";
const VERSION_REMOVING: &str = "removing-v1";
const MAX_MANAGED_KEY_BYTES: usize = 64;
const MAX_RUNTIME_FILES: usize = 128;
const MAX_RUNTIME_BYTES: u64 = 1_073_741_824;
const MAX_VERSION_METADATA_BYTES: u64 = 32_768;
const ROOT_IDENTITY: &[u8] = b"VSIFT-MANAGED-ROOT-v1\n";
const STAGE_IDENTITY: &[u8] = b"VSIFT-MANAGED-STAGE-v1\n";
const VERSIONS_IDENTITY: &[u8] = b"VSIFT-MANAGED-VERSIONS-v1\n";
const CURRENT_IDENTITY: &[u8] = b"VSIFT-MANAGED-CURRENT-v1\n";
const USE_LOCK_IDENTITY: &[u8] = b"VSIFT-MANAGED-USE-LOCK-v1\n";
const REMOVING_IDENTITY: &[u8] = b"VSIFT-MANAGED-REMOVING-v1\n";

/// A typed failure to own, stage or discard an unactivated artifact.
#[derive(Debug)]
pub enum ManagedArtifactError {
    /// No absolute per-user storage location is available.
    Unavailable,
    /// The root, marker or staging directory is not positively owned and private.
    UnsafeStorage,
    /// A concurrent creator or installation transaction holds the managed root.
    Busy,
    /// Private storage failed to create, read, write or remove files.
    Io,
    /// Source bytes failed exact reviewed size or SHA-256 verification.
    Transfer(ArtifactTransferError),
}

/// Reviewed archive encoding and limits, without a floating decoder choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewedPayloadArchive {
    /// Uncompressed tar stream with a reviewed expanded-stream limit.
    Tar {
        /// Highest reviewed uncompressed tar-stream byte count.
        max_tar_bytes: u64,
    },
    /// Gzip/tar with independent compressed and expanded limits.
    GzipTar {
        /// Highest reviewed gzip input byte count.
        max_compressed_bytes: u64,
        /// Highest reviewed uncompressed tar-stream byte count.
        max_tar_bytes: u64,
    },
    /// XZ/tar with independent compressed and expanded limits.
    XzTar {
        /// Highest reviewed XZ input byte count.
        max_compressed_bytes: u64,
        /// Highest reviewed uncompressed tar-stream byte count.
        max_tar_bytes: u64,
    },
}

/// Typed reason reviewed archive bytes cannot become an unactivated payload.
#[derive(Debug)]
pub enum ManagedPayloadError {
    /// The artifact or owned private payload boundary failed.
    Storage(ManagedArtifactError),
    /// Raw tar inventory, selection or staging failed.
    Tar(TarInventoryError),
    /// Gzip/tar inventory, selection or staging failed.
    GzipTar(GzipTarInventoryError),
    /// XZ/tar inventory, selection or staging failed.
    XzTar(XzTarInventoryError),
}

/// One reviewed regular-file alias, flattened to a selected file's exact bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReviewedRuntimeAlias<'a> {
    /// Portable flat runtime filename; never an archive-provided link target.
    pub name: &'a str,
    /// Portable flat name of a previously hash-verified selected regular file.
    pub source_selected: &'a str,
}

/// Reviewed output policy for a private, unactivated runtime copy.
#[derive(Clone, Copy, Debug)]
pub struct ReviewedRuntimeLayout<'a> {
    /// Exact upper byte budget for all selected and alias copies.
    pub max_bytes: u64,
    /// Only aliases separately approved for the eventual runtime layout.
    pub aliases: &'a [ReviewedRuntimeAlias<'a>],
    /// Selected regular files to grant private owner execution on Unix.
    pub executables: &'a [&'a str],
}

/// Typed reason a reviewed runtime layout cannot be prepared.
#[derive(Debug)]
pub enum ManagedRuntimeLayoutError {
    /// The trusted layout is invalid, colliding or exceeds its byte budget.
    InvalidReview,
    /// The owned directory or selected source could not be trusted or written.
    Storage(ManagedArtifactError),
    /// A source copy differed from its selected-file digest or size.
    Transfer(ArtifactTransferError),
}

/// Typed reason a prepared runtime cannot be published and selected.
#[derive(Debug)]
pub enum ManagedRuntimePublicationError {
    /// A component or version key is not a bounded canonical storage key.
    InvalidIdentity,
    /// The managed storage boundary is unavailable, unsafe or failed I/O.
    Storage(ManagedArtifactError),
    /// The requested immutable identity already names different reviewed bytes.
    VersionConflict,
}

/// Outcome of a requested managed-version removal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedVersionRemovalOutcome {
    /// The managed version is absent after removal or an idempotent retry.
    Removed,
    /// The version remains the component's selected version.
    Selected,
    /// A live published-runtime capability holds the version for use.
    InUse,
}

impl fmt::Display for ManagedRuntimePublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentity => "managed runtime identity is invalid",
            Self::Storage(_) => "managed runtime publication storage failed",
            Self::VersionConflict => "managed runtime identity already names different bytes",
        })
    }
}

impl Error for ManagedRuntimePublicationError {}

/// Canonical provider-neutral identity for one immutable managed runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedRuntimeIdentity {
    component: String,
    version: String,
}

impl ManagedRuntimeIdentity {
    /// Validates bounded lowercase ASCII keys used only beneath the managed root.
    ///
    /// # Errors
    ///
    /// Rejects empty, oversized, noncanonical or path-like keys.
    pub fn new(
        component: impl Into<String>,
        version: impl Into<String>,
    ) -> Result<Self, ManagedRuntimePublicationError> {
        let component = component.into();
        let version = version.into();
        if !canonical_managed_key(&component) || !canonical_managed_key(&version) {
            return Err(ManagedRuntimePublicationError::InvalidIdentity);
        }
        Ok(Self { component, version })
    }

    /// Stable component key selected by reviewed application policy.
    #[must_use]
    pub fn component(&self) -> &str {
        &self.component
    }

    /// Stable version key selected by reviewed application policy.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    fn version_directory_name(&self) -> String {
        format!("{}--{}", self.component, self.version)
    }

    fn current_name(&self) -> String {
        format!("{}.current", self.component)
    }

    fn pending_name(&self) -> String {
        format!("{}.pending", self.component)
    }
}

impl fmt::Display for ManagedRuntimeLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidReview => "reviewed runtime layout is invalid",
            Self::Storage(_) => "private runtime layout storage failed",
            Self::Transfer(_) => "selected runtime copy failed verification or I/O",
        })
    }
}

impl Error for ManagedRuntimeLayoutError {}

impl fmt::Display for ManagedPayloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Storage(_) => "private managed payload boundary failed",
            Self::Tar(_) => "reviewed tar payload could not be staged",
            Self::GzipTar(_) => "reviewed gzip/tar payload could not be staged",
            Self::XzTar(_) => "reviewed XZ/tar payload could not be staged",
        })
    }
}

impl Error for ManagedPayloadError {}

impl fmt::Display for ManagedArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "per-user managed storage location is unavailable",
            Self::UnsafeStorage => "managed staging storage is not positively owned and private",
            Self::Busy => "managed root or installation transaction is busy",
            Self::Io => "managed staging I/O failed",
            Self::Transfer(_) => "managed artifact differs from reviewed bytes",
        })
    }
}

impl Error for ManagedArtifactError {}

/// A per-user root which can only hold unactivated staging in this P06 increment.
#[derive(Clone, Debug)]
pub struct ManagedArtifactStore {
    root_path: PathBuf,
}

/// Exclusive root-wide authority for one managed installation transaction.
///
/// Dropping the guard releases the OS lock. It grants serialization only; it
/// does not represent plan acceptance, compatibility success or install authority.
#[derive(Debug)]
pub struct ManagedInstallGuard {
    _lock: fs::File,
    root: Dir,
    root_path: PathBuf,
}

impl ManagedArtifactStore {
    /// Finds the platform's per-user managed-data location.
    ///
    /// # Errors
    ///
    /// Returns `Unavailable` when the host cannot identify an absolute per-user base.
    pub fn default_location() -> Result<Self, ManagedArtifactError> {
        #[cfg(windows)]
        let base = env::var_os("LOCALAPPDATA").map(PathBuf::from);
        #[cfg(target_os = "macos")]
        let base = env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support"));
        #[cfg(all(unix, not(target_os = "macos")))]
        let base = env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
        let base = base.ok_or(ManagedArtifactError::Unavailable)?;
        Self::at(base.join("vsift/managed-v1"))
    }

    /// Selects an absolute root supplied by a trusted host or isolated test.
    ///
    /// # Errors
    ///
    /// Rejects relative, parent-traversing, root and network paths.
    pub fn at(root_path: PathBuf) -> Result<Self, ManagedArtifactError> {
        if !root_path.is_absolute()
            || root_path.file_name().is_none()
            || root_path
                .components()
                .any(|part| part == Component::ParentDir)
        {
            return Err(ManagedArtifactError::Unavailable);
        }
        #[cfg(windows)]
        if matches!(
            root_path.components().next(),
            Some(Component::Prefix(prefix))
                if !matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_))
        ) {
            return Err(ManagedArtifactError::Unavailable);
        }
        Ok(Self { root_path })
    }

    /// Tries to serialize one managed installation transaction for this root.
    ///
    /// The lock file lives inside the positively owned private root and must be
    /// a single-link regular file with private Unix permissions. The operation
    /// never waits or retries, so headless callers receive a typed `Busy` result.
    ///
    /// # Errors
    ///
    /// Returns `Busy` when another process holds the installation lock, and
    /// rejects linked, replaced or incorrectly permissioned lock files.
    pub fn try_install_guard(&self) -> Result<ManagedInstallGuard, ManagedArtifactError> {
        let root = self.open_root()?;
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create(true)
            .follow(FollowSymlinks::No);
        #[cfg(unix)]
        options.mode(0o600);
        let lock = root
            .open_with(INSTALL_LOCK, &options)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        let metadata = lock.metadata().map_err(|_| ManagedArtifactError::Io)?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(ManagedArtifactError::UnsafeStorage);
        }
        #[cfg(unix)]
        if metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(ManagedArtifactError::UnsafeStorage);
        }
        let lock = lock.into_std();
        lock.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => ManagedArtifactError::Busy,
            fs::TryLockError::Error(_) => ManagedArtifactError::Io,
        })?;
        Ok(ManagedInstallGuard {
            _lock: lock,
            root,
            root_path: self.root_path.clone(),
        })
    }

    /// Opens the immutable runtime currently selected for a component.
    ///
    /// # Errors
    ///
    /// Rejects an invalid component key, corrupt pointer, changed manifest or
    /// unsafe published directory. Returns `None` when no version is selected.
    pub fn open_selected_runtime(
        &self,
        component: &str,
    ) -> Result<Option<PublishedManagedRuntime>, ManagedRuntimePublicationError> {
        if !canonical_managed_key(component) {
            return Err(ManagedRuntimePublicationError::InvalidIdentity);
        }
        let Some(root) = self
            .open_existing_root()
            .map_err(ManagedRuntimePublicationError::Storage)?
        else {
            return Ok(None);
        };
        let versions_exists = root
            .try_exists(VERSIONS)
            .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?;
        let current_exists = root
            .try_exists(CURRENT)
            .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?;
        if !versions_exists && !current_exists {
            return Ok(None);
        }
        if !versions_exists || !current_exists {
            return Err(ManagedRuntimePublicationError::Storage(
                ManagedArtifactError::UnsafeStorage,
            ));
        }
        let versions = open_managed_directory(&root, &self.root_path, VERSIONS, VERSIONS_IDENTITY)
            .map_err(ManagedRuntimePublicationError::Storage)?;
        let current = open_managed_directory(&root, &self.root_path, CURRENT, CURRENT_IDENTITY)
            .map_err(ManagedRuntimePublicationError::Storage)?;
        let current_name = format!("{component}.current");
        if !current
            .try_exists(&current_name)
            .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?
        {
            return Ok(None);
        }
        let pointer = read_current_pointer(&current, &current_name)?;
        if pointer.identity.component() != component {
            return Err(ManagedRuntimePublicationError::Storage(
                ManagedArtifactError::UnsafeStorage,
            ));
        }
        open_published_runtime_from_manifest(
            &self.root_path,
            &versions,
            pointer.identity,
            Some(&pointer.manifest_sha256),
        )
        .map(Some)
    }

    /// Opens one immutable published runtime without changing selection.
    ///
    /// # Errors
    ///
    /// Rejects a missing, corrupt, changed or unsafe version directory.
    pub fn open_published_runtime(
        &self,
        identity: &ManagedRuntimeIdentity,
    ) -> Result<PublishedManagedRuntime, ManagedRuntimePublicationError> {
        let root = self
            .open_existing_root()
            .map_err(ManagedRuntimePublicationError::Storage)?
            .ok_or(ManagedRuntimePublicationError::Storage(
                ManagedArtifactError::UnsafeStorage,
            ))?;
        let versions = open_managed_directory(&root, &self.root_path, VERSIONS, VERSIONS_IDENTITY)
            .map_err(ManagedRuntimePublicationError::Storage)?;
        open_published_runtime_from_manifest(&self.root_path, &versions, identity.clone(), None)
    }

    /// Atomically selects an already published immutable version for rollback.
    ///
    /// # Errors
    ///
    /// Requires the installation guard for this root and rejects a missing,
    /// removing, changed or unsafe published version.
    pub fn select_published_runtime(
        &self,
        guard: &ManagedInstallGuard,
        identity: &ManagedRuntimeIdentity,
    ) -> Result<PublishedManagedRuntime, ManagedRuntimePublicationError> {
        let root = self
            .open_existing_root()
            .map_err(ManagedRuntimePublicationError::Storage)?
            .ok_or(ManagedRuntimePublicationError::Storage(
                ManagedArtifactError::UnsafeStorage,
            ))?;
        validate_install_guard(guard, &root, &self.root_path)?;
        let versions = open_managed_directory(&root, &self.root_path, VERSIONS, VERSIONS_IDENTITY)
            .map_err(ManagedRuntimePublicationError::Storage)?;
        let current = open_managed_directory(&root, &self.root_path, CURRENT, CURRENT_IDENTITY)
            .map_err(ManagedRuntimePublicationError::Storage)?;
        let published = open_published_runtime_from_manifest(
            &self.root_path,
            &versions,
            identity.clone(),
            None,
        )?;
        let manifest = read_private_regular_file(
            &published.directory,
            VERSION_MANIFEST,
            MAX_VERSION_METADATA_BYTES,
        )
        .map_err(ManagedRuntimePublicationError::Storage)?;
        write_current_pointer(&current, identity, &manifest, None)?;
        Ok(published)
    }

    /// Removes one unselected managed version when no live capability uses it.
    ///
    /// A tombstone written while holding the exclusive use lock prevents a new
    /// opener from racing deletion. Only the manifest's exact managed files are
    /// removed; unexpected content fails closed.
    ///
    /// # Errors
    ///
    /// Requires the installation guard for this root and rejects corrupt,
    /// linked or otherwise unsafe managed storage.
    pub fn remove_published_runtime(
        &self,
        guard: &ManagedInstallGuard,
        identity: &ManagedRuntimeIdentity,
    ) -> Result<ManagedVersionRemovalOutcome, ManagedRuntimePublicationError> {
        self.remove_published_runtime_at_boundary(guard, identity, None)
    }

    fn remove_published_runtime_at_boundary(
        &self,
        guard: &ManagedInstallGuard,
        identity: &ManagedRuntimeIdentity,
        fault: Option<ManagedRemovalBoundary>,
    ) -> Result<ManagedVersionRemovalOutcome, ManagedRuntimePublicationError> {
        let root = self
            .open_existing_root()
            .map_err(ManagedRuntimePublicationError::Storage)?
            .ok_or(ManagedRuntimePublicationError::Storage(
                ManagedArtifactError::UnsafeStorage,
            ))?;
        validate_install_guard(guard, &root, &self.root_path)?;
        let versions = open_managed_directory(&root, &self.root_path, VERSIONS, VERSIONS_IDENTITY)
            .map_err(ManagedRuntimePublicationError::Storage)?;
        let current = open_managed_directory(&root, &self.root_path, CURRENT, CURRENT_IDENTITY)
            .map_err(ManagedRuntimePublicationError::Storage)?;
        let current_name = identity.current_name();
        if current
            .try_exists(&current_name)
            .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?
            && read_current_pointer(&current, &current_name)?.identity == *identity
        {
            return Ok(ManagedVersionRemovalOutcome::Selected);
        }

        let version_name = identity.version_directory_name();
        match versions.symlink_metadata(&version_name) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ManagedVersionRemovalOutcome::Removed);
            }
            Err(_) => {
                return Err(ManagedRuntimePublicationError::Storage(
                    ManagedArtifactError::Io,
                ));
            }
        }
        let directory = versions.open_dir_nofollow(&version_name).map_err(|_| {
            ManagedRuntimePublicationError::Storage(ManagedArtifactError::UnsafeStorage)
        })?;
        validate_private_root(
            &self.root_path.join(VERSIONS).join(&version_name),
            &directory,
        )
        .map_err(map_private_error)
        .map_err(ManagedRuntimePublicationError::Storage)?;
        if finish_manifestless_removal(&directory)
            .map_err(ManagedRuntimePublicationError::Storage)?
        {
            drop(directory);
            versions
                .remove_dir(&version_name)
                .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?;
            return Ok(ManagedVersionRemovalOutcome::Removed);
        }
        let manifest =
            read_private_regular_file(&directory, VERSION_MANIFEST, MAX_VERSION_METADATA_BYTES)
                .map_err(ManagedRuntimePublicationError::Storage)?;
        let (observed_identity, files) = parse_version_manifest(&manifest)?;
        if observed_identity != *identity {
            return Err(ManagedRuntimePublicationError::Storage(
                ManagedArtifactError::UnsafeStorage,
            ));
        }
        let removing = directory
            .try_exists(VERSION_REMOVING)
            .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?;
        validate_version_contents(&directory, &files, removing)
            .map_err(ManagedRuntimePublicationError::Storage)?;
        let use_lock = match try_exclusive_version_use(&directory, removing)? {
            ExclusiveVersionUse::Acquired(lock) => lock,
            ExclusiveVersionUse::InUse => return Ok(ManagedVersionRemovalOutcome::InUse),
        };
        if removing {
            check_marker(&directory, VERSION_REMOVING, REMOVING_IDENTITY)
                .map_err(ManagedRuntimePublicationError::Storage)?;
        } else {
            write_marker(&directory, VERSION_REMOVING, REMOVING_IDENTITY)
                .map_err(ManagedRuntimePublicationError::Storage)?;
        }
        drop(use_lock);
        validate_version_contents(&directory, &files, true)
            .map_err(ManagedRuntimePublicationError::Storage)?;
        remove_managed_version_files(&directory, &files, fault)
            .map_err(ManagedRuntimePublicationError::Storage)?;
        drop(directory);
        versions
            .remove_dir(&version_name)
            .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?;
        Ok(ManagedVersionRemovalOutcome::Removed)
    }

    /// Imports exact reviewed bytes into a new, private, unactivated directory.
    ///
    /// The caller must supply integrity from reviewed source, not a URL, response,
    /// media file or user-entered digest. On transfer failure, the owned artifact
    /// and staging directory are removed; an unexpected entry is left untouched.
    ///
    /// # Errors
    ///
    /// Returns typed ownership, I/O, short/excess source or digest failures.
    pub fn import_verified<Source: Read>(
        &self,
        source: Source,
        integrity: ArtifactIntegrity,
    ) -> Result<StagedManagedArtifact, ManagedArtifactError> {
        let staged = self.create_stage(integrity)?;
        let result = staged
            .stage
            .open_with(ARTIFACT, &artifact_write_options())
            .map_err(|_| ManagedArtifactError::Io)
            .and_then(|mut file| {
                transfer_verified(source, &mut file, integrity)
                    .map_err(ManagedArtifactError::Transfer)?;
                file.sync_all().map_err(|_| ManagedArtifactError::Io)
            });
        if let Err(error) = result {
            staged.discard()?;
            return Err(error);
        }
        Ok(staged)
    }

    pub(crate) fn begin_stream(
        &self,
        integrity: ArtifactIntegrity,
    ) -> Result<StreamingManagedArtifact, ManagedArtifactError> {
        let staged = self.create_stage(integrity)?;
        let Ok(file) = staged.stage.open_with(ARTIFACT, &artifact_write_options()) else {
            staged.discard()?;
            return Err(ManagedArtifactError::Io);
        };
        Ok(StreamingManagedArtifact {
            staged,
            file: tokio::fs::File::from_std(file.into_std()),
            verifier: StreamingArtifactVerifier::new(integrity),
        })
    }

    fn create_stage(
        &self,
        integrity: ArtifactIntegrity,
    ) -> Result<StagedManagedArtifact, ManagedArtifactError> {
        let root = self.open_root()?;
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| ManagedArtifactError::Io)?;
        let stage_name = format!("stage-{}", hex(&random));
        #[allow(unused_mut, reason = "Unix configures the creation mode")]
        let mut builder = DirBuilder::new();
        #[cfg(unix)]
        builder.mode(0o700);
        root.create_dir_with(Path::new(&stage_name), &builder)
            .map_err(|_| ManagedArtifactError::Io)?;
        let stage = root
            .open_dir_nofollow(&stage_name)
            .map_err(|_| ManagedArtifactError::Io)?;
        if let Err(error) = validate_private_root(&self.root_path.join(&stage_name), &stage) {
            drop(stage);
            root.remove_dir(&stage_name)
                .map_err(|_| ManagedArtifactError::Io)?;
            return Err(map_private_error(error));
        }
        if let Err(error) = write_marker(&stage, STAGE_MARKER, STAGE_IDENTITY) {
            drop(stage);
            // A partial marker is an ambiguous directory; leave it for explicit repair.
            return Err(error);
        }
        let stage_path = self.root_path.join(&stage_name);
        let staged = StagedManagedArtifact {
            root,
            stage,
            stage_name,
            stage_path,
            integrity,
        };
        Ok(staged)
    }

    fn open_root(&self) -> Result<Dir, ManagedArtifactError> {
        let (root, created) = open_private_root_with_creation(&self.root_path, true)
            .map_err(map_private_error)?
            .ok_or(ManagedArtifactError::Unavailable)?;
        if created {
            write_marker(&root, ROOT_MARKER, ROOT_IDENTITY)?;
        } else {
            check_marker(&root, ROOT_MARKER, ROOT_IDENTITY)?;
        }
        Ok(root)
    }

    fn open_existing_root(&self) -> Result<Option<Dir>, ManagedArtifactError> {
        let Some((root, created)) =
            open_private_root_with_creation(&self.root_path, false).map_err(map_private_error)?
        else {
            return Ok(None);
        };
        if created {
            return Err(ManagedArtifactError::UnsafeStorage);
        }
        check_marker(&root, ROOT_MARKER, ROOT_IDENTITY)?;
        Ok(Some(root))
    }
}

pub(crate) struct StreamingManagedArtifact {
    staged: StagedManagedArtifact,
    file: tokio::fs::File,
    verifier: StreamingArtifactVerifier,
}

impl StreamingManagedArtifact {
    pub(crate) async fn append(&mut self, chunk: &[u8]) -> Result<(), ManagedArtifactError> {
        self.verifier
            .accept(chunk)
            .map_err(ManagedArtifactError::Transfer)?;
        self.file
            .write_all(chunk)
            .await
            .map_err(|_| ManagedArtifactError::Io)
    }

    pub(crate) async fn finish(&mut self) -> Result<(), ManagedArtifactError> {
        self.verifier
            .finish()
            .map_err(ManagedArtifactError::Transfer)?;
        self.file
            .sync_all()
            .await
            .map_err(|_| ManagedArtifactError::Io)
    }

    pub(crate) fn complete(self) -> StagedManagedArtifact {
        drop(self.file);
        self.staged
    }

    pub(crate) fn abort(self) -> Result<(), ManagedArtifactError> {
        drop(self.file);
        self.staged.discard()
    }
}

fn artifact_write_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    #[cfg(unix)]
    options.mode(0o600);
    options
}

/// Verified artifact bytes awaiting further archive checks and smoke tests.
pub struct StagedManagedArtifact {
    root: Dir,
    stage: Dir,
    stage_name: String,
    stage_path: PathBuf,
    integrity: ArtifactIntegrity,
}

impl StagedManagedArtifact {
    /// Rechecks exact bytes and opens the candidate without following links.
    ///
    /// # Errors
    ///
    /// Refuses a missing, replaced or linked artifact.
    pub fn open_artifact(&self) -> Result<fs::File, ManagedArtifactError> {
        check_marker(&self.root, ROOT_MARKER, ROOT_IDENTITY)?;
        check_marker(&self.stage, STAGE_MARKER, STAGE_IDENTITY)?;
        self.validate_stage_at_name()?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = self
            .stage
            .open_with(ARTIFACT, &options)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        let metadata = file.metadata().map_err(|_| ManagedArtifactError::Io)?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(ManagedArtifactError::UnsafeStorage);
        }
        let mut file = file.into_std();
        transfer_verified(&mut file, std::io::sink(), self.integrity)
            .map_err(ManagedArtifactError::Transfer)?;
        file.seek(SeekFrom::Start(0))
            .map_err(|_| ManagedArtifactError::Io)?;
        Ok(file)
    }

    /// Stages an exact reviewed selection beneath this owned unactivated artifact.
    ///
    /// The archive reader receives a newly created empty private payload directory.
    /// It ignores archive links and modes, while this boundary retains the original
    /// artifact for a later whole-byte recheck. A failed archive read leaves no
    /// selected files; an unexpected payload entry blocks directory cleanup.
    ///
    /// # Errors
    ///
    /// Returns typed storage, decoder, inventory, selection or digest failures.
    pub fn stage_reviewed_payload(
        &self,
        archive: ReviewedPayloadArchive,
        bounds: ArchiveInventoryBounds,
        reviewed_aliases: &[ReviewedArchiveAlias<'_>],
        selected_files: &[ReviewedArchiveFile<'_>],
    ) -> Result<StagedManagedPayload<'_>, ManagedPayloadError> {
        let artifact = self.open_artifact().map_err(ManagedPayloadError::Storage)?;
        let payload = self
            .create_payload_directory()
            .map_err(ManagedPayloadError::Storage)?;
        let result = match archive {
            ReviewedPayloadArchive::Tar { max_tar_bytes } => stage_tar_selected_files(
                artifact,
                max_tar_bytes,
                bounds,
                reviewed_aliases,
                selected_files,
                &payload,
            )
            .map_err(ManagedPayloadError::Tar),
            ReviewedPayloadArchive::GzipTar {
                max_compressed_bytes,
                max_tar_bytes,
            } => stage_gzip_tar_selected_files(
                artifact,
                max_compressed_bytes,
                max_tar_bytes,
                bounds,
                reviewed_aliases,
                selected_files,
                &payload,
            )
            .map_err(ManagedPayloadError::GzipTar),
            ReviewedPayloadArchive::XzTar {
                max_compressed_bytes,
                max_tar_bytes,
            } => stage_xz_tar_selected_files(
                artifact,
                max_compressed_bytes,
                max_tar_bytes,
                bounds,
                reviewed_aliases,
                selected_files,
                &payload,
            )
            .map_err(ManagedPayloadError::XzTar),
        };
        if let Err(error) = result {
            self.remove_empty_payload(payload)
                .map_err(ManagedPayloadError::Storage)?;
            return Err(error);
        }
        let mut selected = Vec::with_capacity(selected_files.len());
        for file in selected_files {
            let Some(name) = file.path.rsplit('/').next() else {
                self.remove_empty_payload(payload)
                    .map_err(ManagedPayloadError::Storage)?;
                return Err(ManagedPayloadError::Tar(
                    TarInventoryError::InvalidSelection,
                ));
            };
            selected.push(SelectedPayloadFile {
                name: name.to_owned(),
                integrity: file.integrity,
            });
        }
        Ok(StagedManagedPayload {
            artifact: self,
            payload,
            selected,
        })
    }

    fn create_payload_directory(&self) -> Result<Dir, ManagedArtifactError> {
        self.create_private_child(PAYLOAD)
    }

    fn create_runtime_directory(&self) -> Result<Dir, ManagedArtifactError> {
        self.create_private_child(RUNTIME)
    }

    fn create_private_child(&self, name: &str) -> Result<Dir, ManagedArtifactError> {
        check_marker(&self.root, ROOT_MARKER, ROOT_IDENTITY)?;
        check_marker(&self.stage, STAGE_MARKER, STAGE_IDENTITY)?;
        self.validate_stage_at_name()?;
        #[allow(unused_mut, reason = "Unix configures the creation mode")]
        let mut builder = DirBuilder::new();
        #[cfg(unix)]
        builder.mode(0o700);
        self.stage
            .create_dir_with(name, &builder)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        let payload = self
            .stage
            .open_dir_nofollow(name)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        if let Err(error) = validate_private_root(&self.stage_path.join(name), &payload) {
            self.remove_empty_child(name, payload)?;
            return Err(map_private_error(error));
        }
        Ok(payload)
    }

    fn remove_empty_payload(&self, payload: Dir) -> Result<(), ManagedArtifactError> {
        self.remove_empty_child(PAYLOAD, payload)
    }

    fn remove_empty_child(&self, name: &str, payload: Dir) -> Result<(), ManagedArtifactError> {
        check_marker(&self.root, ROOT_MARKER, ROOT_IDENTITY)?;
        check_marker(&self.stage, STAGE_MARKER, STAGE_IDENTITY)?;
        self.validate_stage_at_name()?;
        let at_name = self
            .stage
            .open_dir_nofollow(name)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        validate_same_held_directory(&at_name, &payload).map_err(map_private_error)?;
        drop(at_name);
        if payload
            .entries()
            .map_err(|_| ManagedArtifactError::Io)?
            .next()
            .is_some()
        {
            return Err(ManagedArtifactError::UnsafeStorage);
        }
        drop(payload);
        self.stage
            .remove_dir(name)
            .map_err(|_| ManagedArtifactError::Io)
    }

    fn remove_reviewed_runtime(
        &self,
        runtime: Dir,
        reviewed_names: &[String],
    ) -> Result<(), ManagedArtifactError> {
        check_marker(&self.root, ROOT_MARKER, ROOT_IDENTITY)?;
        check_marker(&self.stage, STAGE_MARKER, STAGE_IDENTITY)?;
        self.validate_stage_at_name()?;
        let at_name = self
            .stage
            .open_dir_nofollow(RUNTIME)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        validate_same_held_directory(&at_name, &runtime).map_err(map_private_error)?;
        drop(at_name);
        for entry in runtime.entries().map_err(|_| ManagedArtifactError::Io)? {
            let entry = entry.map_err(|_| ManagedArtifactError::Io)?;
            let name = entry.file_name();
            if !reviewed_names
                .iter()
                .any(|reviewed| name == reviewed.as_str())
            {
                return Err(ManagedArtifactError::UnsafeStorage);
            }
            let metadata = runtime
                .symlink_metadata(&name)
                .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
            if !metadata.is_file() || metadata.nlink() != 1 {
                return Err(ManagedArtifactError::UnsafeStorage);
            }
        }
        for name in reviewed_names {
            match runtime.symlink_metadata(name) {
                Ok(metadata) if metadata.is_file() && metadata.nlink() == 1 => runtime
                    .remove_file(name)
                    .map_err(|_| ManagedArtifactError::Io)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err(ManagedArtifactError::UnsafeStorage),
            }
        }
        self.remove_empty_child(RUNTIME, runtime)
    }

    fn validate_stage_at_name(&self) -> Result<(), ManagedArtifactError> {
        let at_name = self
            .root
            .open_dir_nofollow(&self.stage_name)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        validate_same_held_directory(&at_name, &self.stage).map_err(map_private_error)?;
        validate_private_root(&self.stage_path, &self.stage).map_err(map_private_error)
    }

    /// Discards only the two positively identified files and their owned directory.
    ///
    /// Unexpected entries stop cleanup so a future installer cannot accidentally
    /// erase user material or an active version.
    ///
    /// # Errors
    ///
    /// Fails closed on a replaced marker, unexpected entry or filesystem error.
    pub fn discard(self) -> Result<(), ManagedArtifactError> {
        let Self {
            root,
            stage,
            stage_name,
            stage_path: _,
            integrity: _,
        } = self;
        check_marker(&root, ROOT_MARKER, ROOT_IDENTITY)?;
        check_marker(&stage, STAGE_MARKER, STAGE_IDENTITY)?;
        let stage_at_name = root
            .open_dir_nofollow(&stage_name)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        validate_same_held_directory(&stage_at_name, &stage).map_err(map_private_error)?;
        drop(stage_at_name);
        let mut entries = stage.entries().map_err(|_| ManagedArtifactError::Io)?;
        for entry in entries.by_ref() {
            let entry = entry.map_err(|_| ManagedArtifactError::Io)?;
            let name = entry.file_name();
            if name != STAGE_MARKER && name != ARTIFACT {
                return Err(ManagedArtifactError::UnsafeStorage);
            }
        }
        drop(entries);
        match stage.symlink_metadata(ARTIFACT) {
            Ok(metadata) if metadata.is_file() && metadata.nlink() == 1 => {
                stage
                    .remove_file(ARTIFACT)
                    .map_err(|_| ManagedArtifactError::Io)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(ManagedArtifactError::UnsafeStorage),
        }
        stage
            .remove_file(STAGE_MARKER)
            .map_err(|_| ManagedArtifactError::Io)?;
        drop(stage);
        root.remove_dir(&stage_name)
            .map_err(|_| ManagedArtifactError::Io)
    }
}

struct SelectedPayloadFile {
    name: String,
    integrity: ArtifactIntegrity,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum RuntimeFileMode {
    PrivateData,
    OwnerExecutable,
}

struct PlannedRuntimeFile {
    name: String,
    source_selected: String,
    integrity: ArtifactIntegrity,
    mode: RuntimeFileMode,
}

fn portable_runtime_name(name: &str) -> bool {
    safe_archive_path(name, false) && !name.contains('/') && safe_staging_name(name)
}

fn review_runtime_layout(
    selected: &[SelectedPayloadFile],
    layout: ReviewedRuntimeLayout<'_>,
) -> Result<Vec<PlannedRuntimeFile>, ManagedRuntimeLayoutError> {
    if layout.max_bytes == 0 || layout.max_bytes > MAX_RUNTIME_BYTES {
        return Err(ManagedRuntimeLayoutError::InvalidReview);
    }
    let mut executable_names = HashSet::new();
    for executable in layout.executables {
        if !portable_runtime_name(executable)
            || !selected.iter().any(|file| file.name == *executable)
            || !executable_names.insert(executable.to_ascii_lowercase())
        {
            return Err(ManagedRuntimeLayoutError::InvalidReview);
        }
    }
    let mut names = HashSet::new();
    let mut bytes = 0_u64;
    let mut planned = Vec::with_capacity(selected.len() + layout.aliases.len());
    for file in selected {
        if !portable_runtime_name(&file.name) || !names.insert(file.name.to_ascii_lowercase()) {
            return Err(ManagedRuntimeLayoutError::InvalidReview);
        }
        bytes = bytes
            .checked_add(file.integrity.bytes())
            .ok_or(ManagedRuntimeLayoutError::InvalidReview)?;
        planned.push(PlannedRuntimeFile {
            name: file.name.clone(),
            source_selected: file.name.clone(),
            integrity: file.integrity,
            mode: if executable_names.contains(&file.name.to_ascii_lowercase()) {
                RuntimeFileMode::OwnerExecutable
            } else {
                RuntimeFileMode::PrivateData
            },
        });
    }
    for alias in layout.aliases {
        let source = selected
            .iter()
            .find(|file| file.name == alias.source_selected)
            .ok_or(ManagedRuntimeLayoutError::InvalidReview)?;
        if !portable_runtime_name(alias.name) || !names.insert(alias.name.to_ascii_lowercase()) {
            return Err(ManagedRuntimeLayoutError::InvalidReview);
        }
        bytes = bytes
            .checked_add(source.integrity.bytes())
            .ok_or(ManagedRuntimeLayoutError::InvalidReview)?;
        planned.push(PlannedRuntimeFile {
            name: alias.name.to_owned(),
            source_selected: source.name.clone(),
            integrity: source.integrity,
            mode: RuntimeFileMode::PrivateData,
        });
    }
    if planned.len() > MAX_RUNTIME_FILES || bytes > layout.max_bytes {
        return Err(ManagedRuntimeLayoutError::InvalidReview);
    }
    Ok(planned)
}

/// An exact reviewed selection in a private unactivated payload directory.
pub struct StagedManagedPayload<'a> {
    artifact: &'a StagedManagedArtifact,
    payload: Dir,
    selected: Vec<SelectedPayloadFile>,
}

impl StagedManagedPayload<'_> {
    /// The flat names recorded by the selected-file archive review.
    #[must_use]
    pub fn selected_names(&self) -> Vec<&str> {
        self.selected
            .iter()
            .map(|file| file.name.as_str())
            .collect()
    }

    /// Copies verified selected files and reviewed regular-file aliases into a
    /// fresh private runtime directory; the original payload remains untouched.
    ///
    /// The result remains unactivated. The caller must recheck every runtime
    /// file and perform bounded compatibility smoke before any later promotion.
    ///
    /// # Errors
    ///
    /// Rejects invalid/colliding reviews, changed selected bytes and unsafe
    /// storage. On failure, only positively identified new runtime files are
    /// removed; unexpected entries block cleanup.
    pub fn prepare_reviewed_runtime(
        &self,
        layout: ReviewedRuntimeLayout<'_>,
    ) -> Result<PreparedManagedRuntime<'_, '_>, ManagedRuntimeLayoutError> {
        let planned = review_runtime_layout(&self.selected, layout)?;
        let sources = planned
            .iter()
            .map(|file| {
                self.open_selected_file(&file.source_selected)
                    .map_err(ManagedRuntimeLayoutError::Storage)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let runtime = self
            .artifact
            .create_runtime_directory()
            .map_err(ManagedRuntimeLayoutError::Storage)?;
        let mut created = Vec::with_capacity(planned.len());
        let result = planned
            .iter()
            .zip(sources)
            .try_for_each(|(file, mut source)| {
                let mut output = runtime
                    .open_with(&file.name, &artifact_write_options())
                    .map_err(|_| ManagedRuntimeLayoutError::Storage(ManagedArtifactError::Io))?;
                created.push(file.name.clone());
                transfer_verified(&mut source, &mut output, file.integrity)
                    .map_err(ManagedRuntimeLayoutError::Transfer)?;
                output
                    .sync_all()
                    .map_err(|_| ManagedRuntimeLayoutError::Storage(ManagedArtifactError::Io))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = match file.mode {
                        RuntimeFileMode::PrivateData => 0o600,
                        RuntimeFileMode::OwnerExecutable => 0o700,
                    };
                    output
                        .into_std()
                        .set_permissions(fs::Permissions::from_mode(mode))
                        .map_err(|_| {
                            ManagedRuntimeLayoutError::Storage(ManagedArtifactError::Io)
                        })?;
                }
                #[cfg(windows)]
                let _ = file.mode;
                Ok::<(), ManagedRuntimeLayoutError>(())
            });
        if let Err(error) = result {
            self.artifact
                .remove_reviewed_runtime(runtime, &created)
                .map_err(ManagedRuntimeLayoutError::Storage)?;
            return Err(error);
        }
        Ok(PreparedManagedRuntime {
            payload: self,
            runtime: Some(runtime),
            files: planned,
        })
    }

    /// Rechecks one selected regular file by its reviewed SHA-256 before smoke.
    ///
    /// # Errors
    ///
    /// Rejects an unselected name, replaced directory, linked file or changed bytes.
    pub fn open_selected_file(&self, name: &str) -> Result<fs::File, ManagedArtifactError> {
        check_marker(&self.artifact.root, ROOT_MARKER, ROOT_IDENTITY)?;
        check_marker(&self.artifact.stage, STAGE_MARKER, STAGE_IDENTITY)?;
        self.artifact.validate_stage_at_name()?;
        let selected = self
            .selected
            .iter()
            .find(|file| file.name == name)
            .ok_or(ManagedArtifactError::UnsafeStorage)?;
        let at_name = self
            .artifact
            .stage
            .open_dir_nofollow(PAYLOAD)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        validate_same_held_directory(&at_name, &self.payload).map_err(map_private_error)?;
        self.validate_exact_contents()?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = self
            .payload
            .open_with(name, &options)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        let metadata = file.metadata().map_err(|_| ManagedArtifactError::Io)?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(ManagedArtifactError::UnsafeStorage);
        }
        let mut file = file.into_std();
        transfer_verified(&mut file, std::io::sink(), selected.integrity)
            .map_err(ManagedArtifactError::Transfer)?;
        file.seek(SeekFrom::Start(0))
            .map_err(|_| ManagedArtifactError::Io)?;
        Ok(file)
    }

    fn validate_exact_contents(&self) -> Result<(), ManagedArtifactError> {
        let mut observed = HashSet::with_capacity(self.selected.len());
        for entry in self
            .payload
            .entries()
            .map_err(|_| ManagedArtifactError::Io)?
        {
            let entry = entry.map_err(|_| ManagedArtifactError::Io)?;
            let name = entry.file_name();
            let name = name.to_str().ok_or(ManagedArtifactError::UnsafeStorage)?;
            if !self.selected.iter().any(|file| file.name == name)
                || !observed.insert(name.to_owned())
            {
                return Err(ManagedArtifactError::UnsafeStorage);
            }
            let metadata = self
                .payload
                .symlink_metadata(name)
                .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
            if !metadata.is_file() || metadata.nlink() != 1 {
                return Err(ManagedArtifactError::UnsafeStorage);
            }
        }
        if observed.len() != self.selected.len() {
            return Err(ManagedArtifactError::UnsafeStorage);
        }
        Ok(())
    }

    /// Removes only reviewed selected files and the positively held payload.
    ///
    /// The artifact stage remains available for explicit discard or retry.
    /// Unexpected entries and linked files block deletion rather than expanding
    /// the set of material this operation is allowed to remove.
    ///
    /// # Errors
    ///
    /// Returns typed ownership or I/O failure when cleanup cannot be proven safe.
    pub fn discard(self) -> Result<(), ManagedArtifactError> {
        let Self {
            artifact,
            payload,
            selected,
        } = self;
        check_marker(&artifact.root, ROOT_MARKER, ROOT_IDENTITY)?;
        check_marker(&artifact.stage, STAGE_MARKER, STAGE_IDENTITY)?;
        artifact.validate_stage_at_name()?;
        let at_name = artifact
            .stage
            .open_dir_nofollow(PAYLOAD)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        validate_same_held_directory(&at_name, &payload).map_err(map_private_error)?;
        drop(at_name);
        for entry in payload.entries().map_err(|_| ManagedArtifactError::Io)? {
            let entry = entry.map_err(|_| ManagedArtifactError::Io)?;
            let name = entry.file_name();
            if !selected.iter().any(|file| name == file.name.as_str()) {
                return Err(ManagedArtifactError::UnsafeStorage);
            }
            let metadata = payload
                .symlink_metadata(&name)
                .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
            if !metadata.is_file() || metadata.nlink() != 1 {
                return Err(ManagedArtifactError::UnsafeStorage);
            }
        }
        for file in &selected {
            match payload.symlink_metadata(&file.name) {
                Ok(metadata) if metadata.is_file() && metadata.nlink() == 1 => payload
                    .remove_file(&file.name)
                    .map_err(|_| ManagedArtifactError::Io)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err(ManagedArtifactError::UnsafeStorage),
            }
        }
        artifact.remove_empty_payload(payload)
    }
}

/// Private, byte-verified runtime copies and aliases awaiting compatibility smoke.
pub struct PreparedManagedRuntime<'a, 'b> {
    payload: &'b StagedManagedPayload<'a>,
    runtime: Option<Dir>,
    files: Vec<PlannedRuntimeFile>,
}

impl PreparedManagedRuntime<'_, '_> {
    /// Exact flat filenames in the unactivated runtime directory.
    #[must_use]
    pub fn reviewed_names(&self) -> Vec<&str> {
        self.files.iter().map(|file| file.name.as_str()).collect()
    }

    /// Rechecks every regular runtime file and permission before later smoke.
    ///
    /// # Errors
    ///
    /// Rejects substituted directories, linked or extra files, changed bytes
    /// and unexpected Unix modes.
    pub fn recheck_all(&self) -> Result<(), ManagedArtifactError> {
        for file in &self.files {
            self.open_reviewed_file(&file.name)?;
        }
        Ok(())
    }

    /// Opens one reviewed runtime file as a held, rehashed regular-file handle.
    ///
    /// # Errors
    ///
    /// Rejects names outside the review and any changed ownership, content or
    /// permissions before handing the file to a later smoke boundary.
    pub fn open_reviewed_file(&self, name: &str) -> Result<fs::File, ManagedArtifactError> {
        let artifact = self.payload.artifact;
        let runtime = self
            .runtime
            .as_ref()
            .ok_or(ManagedArtifactError::UnsafeStorage)?;
        check_marker(&artifact.root, ROOT_MARKER, ROOT_IDENTITY)?;
        check_marker(&artifact.stage, STAGE_MARKER, STAGE_IDENTITY)?;
        artifact.validate_stage_at_name()?;
        let at_name = artifact
            .stage
            .open_dir_nofollow(RUNTIME)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        validate_same_held_directory(&at_name, runtime).map_err(map_private_error)?;
        validate_private_root(&artifact.stage_path.join(RUNTIME), runtime)
            .map_err(map_private_error)?;
        let reviewed = self
            .files
            .iter()
            .find(|file| file.name == name)
            .ok_or(ManagedArtifactError::UnsafeStorage)?;
        self.validate_exact_contents()?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = runtime
            .open_with(name, &options)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        let metadata = file.metadata().map_err(|_| ManagedArtifactError::Io)?;
        validate_runtime_file(&metadata, reviewed.mode)?;
        let mut file = file.into_std();
        transfer_verified(&mut file, std::io::sink(), reviewed.integrity)
            .map_err(ManagedArtifactError::Transfer)?;
        file.seek(SeekFrom::Start(0))
            .map_err(|_| ManagedArtifactError::Io)?;
        Ok(file)
    }

    fn validate_exact_contents(&self) -> Result<(), ManagedArtifactError> {
        let runtime = self
            .runtime
            .as_ref()
            .ok_or(ManagedArtifactError::UnsafeStorage)?;
        let mut observed = HashSet::with_capacity(self.files.len());
        for entry in runtime.entries().map_err(|_| ManagedArtifactError::Io)? {
            let entry = entry.map_err(|_| ManagedArtifactError::Io)?;
            let name = entry.file_name();
            let name = name.to_str().ok_or(ManagedArtifactError::UnsafeStorage)?;
            let reviewed = self
                .files
                .iter()
                .find(|file| file.name == name)
                .ok_or(ManagedArtifactError::UnsafeStorage)?;
            if !observed.insert(name.to_owned()) {
                return Err(ManagedArtifactError::UnsafeStorage);
            }
            let metadata = runtime
                .symlink_metadata(name)
                .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
            validate_runtime_file(&metadata, reviewed.mode)?;
        }
        if observed.len() != self.files.len() {
            return Err(ManagedArtifactError::UnsafeStorage);
        }
        Ok(())
    }

    /// Publishes this rechecked runtime as an immutable version and atomically
    /// selects it for its component.
    ///
    /// The caller must hold the installation guard for the same managed root.
    /// A retry with the same identity and exact manifest is idempotent. A
    /// different manifest under an existing identity fails without changing
    /// the selected version. This operation carries no plan or compatibility
    /// authority; the application layer must establish both before calling it.
    ///
    /// # Errors
    ///
    /// Rejects a guard from another root, changed runtime bytes, conflicting
    /// immutable identity, unsafe storage or pointer publication failure.
    pub fn publish_and_select(
        &mut self,
        guard: &ManagedInstallGuard,
        identity: &ManagedRuntimeIdentity,
    ) -> Result<PublishedManagedRuntime, ManagedRuntimePublicationError> {
        self.publish_and_select_at_boundary(guard, identity, None)
    }

    fn publish_and_select_at_boundary(
        &mut self,
        guard: &ManagedInstallGuard,
        identity: &ManagedRuntimeIdentity,
        fault: Option<ManagedPublicationBoundary>,
    ) -> Result<PublishedManagedRuntime, ManagedRuntimePublicationError> {
        self.recheck_all()
            .map_err(ManagedRuntimePublicationError::Storage)?;
        let artifact = self.payload.artifact;
        validate_same_held_directory(&guard.root, &artifact.root)
            .map_err(map_private_error)
            .map_err(ManagedRuntimePublicationError::Storage)?;
        let root_path =
            artifact
                .stage_path
                .parent()
                .ok_or(ManagedRuntimePublicationError::Storage(
                    ManagedArtifactError::UnsafeStorage,
                ))?;
        if guard.root_path != root_path {
            return Err(ManagedRuntimePublicationError::Storage(
                ManagedArtifactError::UnsafeStorage,
            ));
        }
        let versions = open_or_create_managed_directory(
            &artifact.root,
            root_path,
            VERSIONS,
            VERSIONS_IDENTITY,
        )
        .map_err(ManagedRuntimePublicationError::Storage)?;
        let current =
            open_or_create_managed_directory(&artifact.root, root_path, CURRENT, CURRENT_IDENTITY)
                .map_err(ManagedRuntimePublicationError::Storage)?;
        let manifest = version_manifest(identity, &self.files);
        let runtime = self
            .runtime
            .as_ref()
            .ok_or(ManagedRuntimePublicationError::Storage(
                ManagedArtifactError::UnsafeStorage,
            ))?;
        prepare_version_metadata(runtime, &manifest)?;

        let version_name = identity.version_directory_name();
        let version_exists = versions
            .try_exists(&version_name)
            .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?;
        if version_exists {
            let existing =
                open_published_runtime(root_path, &versions, identity, &manifest, &self.files);
            remove_version_metadata(runtime)?;
            let existing = existing?;
            let candidate = self
                .runtime
                .take()
                .ok_or(ManagedRuntimePublicationError::Storage(
                    ManagedArtifactError::UnsafeStorage,
                ))?;
            let reviewed_names = self
                .files
                .iter()
                .map(|file| file.name.clone())
                .collect::<Vec<_>>();
            artifact
                .remove_reviewed_runtime(candidate, &reviewed_names)
                .map_err(ManagedRuntimePublicationError::Storage)?;
            inject_publication_fault(fault, ManagedPublicationBoundary::VersionPublished)?;
            write_current_pointer(&current, identity, &manifest, fault)?;
            return Ok(existing);
        }

        let runtime = self
            .runtime
            .take()
            .ok_or(ManagedRuntimePublicationError::Storage(
                ManagedArtifactError::UnsafeStorage,
            ))?;
        drop(runtime);
        if artifact
            .stage
            .rename(RUNTIME, &versions, &version_name)
            .is_err()
        {
            self.runtime = artifact.stage.open_dir_nofollow(RUNTIME).ok();
            return Err(ManagedRuntimePublicationError::Storage(
                ManagedArtifactError::Io,
            ));
        }
        let at_name = versions.open_dir_nofollow(&version_name).map_err(|_| {
            ManagedRuntimePublicationError::Storage(ManagedArtifactError::UnsafeStorage)
        })?;
        validate_private_root(&root_path.join(VERSIONS).join(&version_name), &at_name)
            .map_err(map_private_error)
            .map_err(ManagedRuntimePublicationError::Storage)?;
        let published =
            open_published_runtime(root_path, &versions, identity, &manifest, &self.files)?;
        inject_publication_fault(fault, ManagedPublicationBoundary::VersionPublished)?;
        write_current_pointer(&current, identity, &manifest, fault)?;
        Ok(published)
    }

    /// Discards only reviewed runtime copies; original payload remains available.
    ///
    /// # Errors
    ///
    /// Unexpected or linked entries prevent deletion.
    pub fn discard(mut self) -> Result<(), ManagedArtifactError> {
        let Some(runtime) = self.runtime.take() else {
            return Ok(());
        };
        let reviewed_names = self
            .files
            .iter()
            .map(|file| file.name.clone())
            .collect::<Vec<_>>();
        self.payload
            .artifact
            .remove_reviewed_runtime(runtime, &reviewed_names)
    }
}

/// Held, revalidated capability for one immutable published runtime version.
pub struct PublishedManagedRuntime {
    identity: ManagedRuntimeIdentity,
    directory: Dir,
    files: Vec<PublishedRuntimeFile>,
    _use_lock: fs::File,
}

impl PublishedManagedRuntime {
    /// Immutable component/version identity selected by reviewed policy.
    #[must_use]
    pub fn identity(&self) -> &ManagedRuntimeIdentity {
        &self.identity
    }

    /// Exact flat filenames in this immutable runtime version.
    #[must_use]
    pub fn reviewed_names(&self) -> Vec<&str> {
        self.files.iter().map(|file| file.name.as_str()).collect()
    }

    /// Opens and rehashes one reviewed runtime file without following links.
    ///
    /// # Errors
    ///
    /// Rejects unknown, changed, linked or permission-altered files.
    pub fn open_reviewed_file(&self, name: &str) -> Result<fs::File, ManagedArtifactError> {
        let reviewed = self
            .files
            .iter()
            .find(|file| file.name == name)
            .ok_or(ManagedArtifactError::UnsafeStorage)?;
        validate_published_contents(&self.directory, &self.files)?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = self
            .directory
            .open_with(name, &options)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        let metadata = file.metadata().map_err(|_| ManagedArtifactError::Io)?;
        validate_runtime_file(&metadata, reviewed.mode)?;
        let mut file = file.into_std();
        transfer_verified(&mut file, std::io::sink(), reviewed.integrity)
            .map_err(ManagedArtifactError::Transfer)?;
        file.seek(SeekFrom::Start(0))
            .map_err(|_| ManagedArtifactError::Io)?;
        Ok(file)
    }
}

struct PublishedRuntimeFile {
    name: String,
    integrity: ArtifactIntegrity,
    mode: RuntimeFileMode,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ManagedPublicationBoundary {
    VersionPublished,
    PointerPrepared,
    PointerReplaced,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ManagedRemovalBoundary {
    PayloadFiles,
    UseLock,
    VersionManifest,
}

enum ExclusiveVersionUse {
    Acquired(Option<fs::File>),
    InUse,
}

struct CurrentPointer {
    identity: ManagedRuntimeIdentity,
    manifest_sha256: String,
}

fn canonical_managed_key(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_MANAGED_KEY_BYTES
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && (bytes[bytes.len() - 1].is_ascii_lowercase() || bytes[bytes.len() - 1].is_ascii_digit())
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn validate_install_guard(
    guard: &ManagedInstallGuard,
    root: &Dir,
    root_path: &Path,
) -> Result<(), ManagedRuntimePublicationError> {
    if guard.root_path != root_path {
        return Err(ManagedRuntimePublicationError::Storage(
            ManagedArtifactError::UnsafeStorage,
        ));
    }
    validate_same_held_directory(&guard.root, root)
        .map_err(map_private_error)
        .map_err(ManagedRuntimePublicationError::Storage)
}

fn open_or_create_managed_directory(
    root: &Dir,
    root_path: &Path,
    name: &str,
    identity: &[u8],
) -> Result<Dir, ManagedArtifactError> {
    let exists = root
        .try_exists(name)
        .map_err(|_| ManagedArtifactError::Io)?;
    if !exists {
        #[allow(unused_mut, reason = "Unix configures the creation mode")]
        let mut builder = DirBuilder::new();
        #[cfg(unix)]
        builder.mode(0o700);
        root.create_dir_with(name, &builder)
            .map_err(|_| ManagedArtifactError::Io)?;
    }
    if !exists {
        let directory = root
            .open_dir_nofollow(name)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        validate_private_root(&root_path.join(name), &directory).map_err(map_private_error)?;
        write_marker(&directory, DIRECTORY_MARKER, identity)?;
    }
    open_managed_directory(root, root_path, name, identity)
}

fn open_managed_directory(
    root: &Dir,
    root_path: &Path,
    name: &str,
    identity: &[u8],
) -> Result<Dir, ManagedArtifactError> {
    let directory = root
        .open_dir_nofollow(name)
        .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
    validate_private_root(&root_path.join(name), &directory).map_err(map_private_error)?;
    check_marker(&directory, DIRECTORY_MARKER, identity)?;
    Ok(directory)
}

fn version_manifest(identity: &ManagedRuntimeIdentity, files: &[PlannedRuntimeFile]) -> Vec<u8> {
    let mut manifest = format!(
        "VSIFT-MANAGED-VERSION-v1\ncomponent={}\nversion={}\nfiles={}\n",
        identity.component(),
        identity.version(),
        files.len()
    );
    for file in files {
        let mode = match file.mode {
            RuntimeFileMode::PrivateData => "data",
            RuntimeFileMode::OwnerExecutable => "executable",
        };
        manifest.push_str("file=");
        manifest.push_str(&file.name);
        manifest.push('\t');
        manifest.push_str(&file.integrity.bytes().to_string());
        manifest.push('\t');
        manifest.push_str(&hex(&file.integrity.sha256()));
        manifest.push('\t');
        manifest.push_str(mode);
        manifest.push('\n');
    }
    manifest.into_bytes()
}

fn prepare_version_metadata(
    runtime: &Dir,
    manifest: &[u8],
) -> Result<(), ManagedRuntimePublicationError> {
    write_marker(runtime, VERSION_MANIFEST, manifest)
        .map_err(ManagedRuntimePublicationError::Storage)?;
    if let Err(error) = write_marker(runtime, VERSION_USE_LOCK, USE_LOCK_IDENTITY) {
        runtime
            .remove_file(VERSION_MANIFEST)
            .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?;
        return Err(ManagedRuntimePublicationError::Storage(error));
    }
    Ok(())
}

fn remove_version_metadata(runtime: &Dir) -> Result<(), ManagedRuntimePublicationError> {
    for name in [VERSION_MANIFEST, VERSION_USE_LOCK] {
        runtime
            .remove_file(name)
            .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?;
    }
    Ok(())
}

fn parse_version_manifest(
    bytes: &[u8],
) -> Result<(ManagedRuntimeIdentity, Vec<PublishedRuntimeFile>), ManagedRuntimePublicationError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ManagedRuntimePublicationError::Storage(ManagedArtifactError::UnsafeStorage)
    })?;
    let mut lines = text.lines();
    if lines.next() != Some("VSIFT-MANAGED-VERSION-v1") {
        return Err(ManagedRuntimePublicationError::Storage(
            ManagedArtifactError::UnsafeStorage,
        ));
    }
    let component = required_metadata_value(lines.next(), "component=")?;
    let version = required_metadata_value(lines.next(), "version=")?;
    let identity = ManagedRuntimeIdentity::new(component, version).map_err(|_| {
        ManagedRuntimePublicationError::Storage(ManagedArtifactError::UnsafeStorage)
    })?;
    let count = required_metadata_value(lines.next(), "files=")?
        .parse::<usize>()
        .map_err(|_| {
            ManagedRuntimePublicationError::Storage(ManagedArtifactError::UnsafeStorage)
        })?;
    if count == 0 || count > MAX_RUNTIME_FILES {
        return Err(ManagedRuntimePublicationError::Storage(
            ManagedArtifactError::UnsafeStorage,
        ));
    }
    let mut names = HashSet::with_capacity(count);
    let mut files = Vec::with_capacity(count);
    for line in lines {
        let Some(value) = line.strip_prefix("file=") else {
            return Err(ManagedRuntimePublicationError::Storage(
                ManagedArtifactError::UnsafeStorage,
            ));
        };
        let mut fields = value.split('\t');
        let (Some(name), Some(bytes), Some(sha256), Some(mode), None) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            return Err(ManagedRuntimePublicationError::Storage(
                ManagedArtifactError::UnsafeStorage,
            ));
        };
        if !portable_runtime_name(name) || !names.insert(name.to_ascii_lowercase()) {
            return Err(ManagedRuntimePublicationError::Storage(
                ManagedArtifactError::UnsafeStorage,
            ));
        }
        let bytes = bytes.parse::<u64>().map_err(|_| {
            ManagedRuntimePublicationError::Storage(ManagedArtifactError::UnsafeStorage)
        })?;
        let integrity = ArtifactIntegrity::from_sha256_hex(bytes, sha256).map_err(|_| {
            ManagedRuntimePublicationError::Storage(ManagedArtifactError::UnsafeStorage)
        })?;
        let mode = match mode {
            "data" => RuntimeFileMode::PrivateData,
            "executable" => RuntimeFileMode::OwnerExecutable,
            _ => {
                return Err(ManagedRuntimePublicationError::Storage(
                    ManagedArtifactError::UnsafeStorage,
                ));
            }
        };
        files.push(PublishedRuntimeFile {
            name: name.to_owned(),
            integrity,
            mode,
        });
    }
    if files.len() != count || !text.ends_with('\n') {
        return Err(ManagedRuntimePublicationError::Storage(
            ManagedArtifactError::UnsafeStorage,
        ));
    }
    Ok((identity, files))
}

fn required_metadata_value<'a>(
    line: Option<&'a str>,
    prefix: &str,
) -> Result<&'a str, ManagedRuntimePublicationError> {
    line.and_then(|value| value.strip_prefix(prefix)).ok_or(
        ManagedRuntimePublicationError::Storage(ManagedArtifactError::UnsafeStorage),
    )
}

fn open_published_runtime(
    root_path: &Path,
    versions: &Dir,
    identity: &ManagedRuntimeIdentity,
    expected_manifest: &[u8],
    planned: &[PlannedRuntimeFile],
) -> Result<PublishedManagedRuntime, ManagedRuntimePublicationError> {
    let published =
        open_published_runtime_from_manifest(root_path, versions, identity.clone(), None)?;
    let observed = read_private_regular_file(
        &published.directory,
        VERSION_MANIFEST,
        MAX_VERSION_METADATA_BYTES,
    )
    .map_err(ManagedRuntimePublicationError::Storage)?;
    if observed != expected_manifest {
        return Err(ManagedRuntimePublicationError::VersionConflict);
    }
    if published.files.len() != planned.len()
        || !published.files.iter().zip(planned).all(|(left, right)| {
            left.name == right.name && left.integrity == right.integrity && left.mode == right.mode
        })
    {
        return Err(ManagedRuntimePublicationError::VersionConflict);
    }
    Ok(published)
}

fn open_published_runtime_from_manifest(
    root_path: &Path,
    versions: &Dir,
    identity: ManagedRuntimeIdentity,
    expected_manifest_sha256: Option<&str>,
) -> Result<PublishedManagedRuntime, ManagedRuntimePublicationError> {
    let version_name = identity.version_directory_name();
    let directory = versions.open_dir_nofollow(&version_name).map_err(|_| {
        ManagedRuntimePublicationError::Storage(ManagedArtifactError::UnsafeStorage)
    })?;
    validate_private_root(&root_path.join(VERSIONS).join(&version_name), &directory)
        .map_err(map_private_error)
        .map_err(ManagedRuntimePublicationError::Storage)?;
    let manifest =
        read_private_regular_file(&directory, VERSION_MANIFEST, MAX_VERSION_METADATA_BYTES)
            .map_err(ManagedRuntimePublicationError::Storage)?;
    if expected_manifest_sha256.is_some_and(|expected| sha256_hex(&manifest) != expected) {
        return Err(ManagedRuntimePublicationError::Storage(
            ManagedArtifactError::UnsafeStorage,
        ));
    }
    let (observed_identity, files) = parse_version_manifest(&manifest)?;
    if observed_identity != identity {
        return Err(ManagedRuntimePublicationError::Storage(
            ManagedArtifactError::UnsafeStorage,
        ));
    }
    validate_published_contents(&directory, &files)
        .map_err(ManagedRuntimePublicationError::Storage)?;
    let use_lock =
        open_version_use_lock(&directory).map_err(ManagedRuntimePublicationError::Storage)?;
    use_lock.try_lock_shared().map_err(|error| match error {
        fs::TryLockError::WouldBlock => {
            ManagedRuntimePublicationError::Storage(ManagedArtifactError::Busy)
        }
        fs::TryLockError::Error(_) => {
            ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io)
        }
    })?;
    validate_published_contents(&directory, &files)
        .map_err(ManagedRuntimePublicationError::Storage)?;
    Ok(PublishedManagedRuntime {
        identity,
        directory,
        files,
        _use_lock: use_lock,
    })
}

fn validate_published_contents(
    directory: &Dir,
    files: &[PublishedRuntimeFile],
) -> Result<(), ManagedArtifactError> {
    validate_version_contents(directory, files, false)
}

fn validate_version_contents(
    directory: &Dir,
    files: &[PublishedRuntimeFile],
    allow_removing: bool,
) -> Result<(), ManagedArtifactError> {
    let mut observed = HashSet::with_capacity(files.len() + 3);
    for entry in directory.entries().map_err(|_| ManagedArtifactError::Io)? {
        let entry = entry.map_err(|_| ManagedArtifactError::Io)?;
        let name = entry.file_name();
        let name = name.to_str().ok_or(ManagedArtifactError::UnsafeStorage)?;
        if name != VERSION_MANIFEST
            && name != VERSION_USE_LOCK
            && !(allow_removing && name == VERSION_REMOVING)
            && !files.iter().any(|file| file.name == name)
        {
            return Err(ManagedArtifactError::UnsafeStorage);
        }
        if !observed.insert(name.to_owned()) {
            return Err(ManagedArtifactError::UnsafeStorage);
        }
        let metadata = directory
            .symlink_metadata(name)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        if name == VERSION_MANIFEST || name == VERSION_USE_LOCK || name == VERSION_REMOVING {
            validate_private_regular_metadata(&metadata)?;
        } else {
            let reviewed = files
                .iter()
                .find(|file| file.name == name)
                .ok_or(ManagedArtifactError::UnsafeStorage)?;
            validate_runtime_file(&metadata, reviewed.mode)?;
        }
    }
    if (!allow_removing && observed.len() != files.len() + 2)
        || (allow_removing && observed.len() < 2)
        || !observed.contains(VERSION_MANIFEST)
        || (!allow_removing && !observed.contains(VERSION_USE_LOCK))
        || (allow_removing && !observed.contains(VERSION_REMOVING))
    {
        return Err(ManagedArtifactError::UnsafeStorage);
    }
    if observed.contains(VERSION_USE_LOCK) {
        check_marker(directory, VERSION_USE_LOCK, USE_LOCK_IDENTITY)?;
    }
    if allow_removing {
        check_marker(directory, VERSION_REMOVING, REMOVING_IDENTITY)?;
    }
    for reviewed in files {
        if allow_removing
            && !directory
                .try_exists(&reviewed.name)
                .map_err(|_| ManagedArtifactError::Io)?
        {
            continue;
        }
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = directory
            .open_with(&reviewed.name, &options)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        let metadata = file.metadata().map_err(|_| ManagedArtifactError::Io)?;
        validate_runtime_file(&metadata, reviewed.mode)?;
        transfer_verified(file.into_std(), std::io::sink(), reviewed.integrity)
            .map_err(ManagedArtifactError::Transfer)?;
    }
    Ok(())
}

fn remove_managed_version_files(
    directory: &Dir,
    files: &[PublishedRuntimeFile],
    fault: Option<ManagedRemovalBoundary>,
) -> Result<(), ManagedArtifactError> {
    for file in files {
        match directory.symlink_metadata(&file.name) {
            Ok(metadata) => {
                validate_runtime_file(&metadata, file.mode)?;
                directory
                    .remove_file(&file.name)
                    .map_err(|_| ManagedArtifactError::Io)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(ManagedArtifactError::UnsafeStorage),
        }
    }
    if fault == Some(ManagedRemovalBoundary::PayloadFiles) {
        return Err(ManagedArtifactError::Io);
    }
    remove_known_regular_file_if_present(directory, VERSION_USE_LOCK)?;
    if fault == Some(ManagedRemovalBoundary::UseLock) {
        return Err(ManagedArtifactError::Io);
    }
    remove_known_regular_file_if_present(directory, VERSION_MANIFEST)?;
    if fault == Some(ManagedRemovalBoundary::VersionManifest) {
        return Err(ManagedArtifactError::Io);
    }
    remove_known_regular_file_if_present(directory, VERSION_REMOVING)?;
    if directory
        .entries()
        .map_err(|_| ManagedArtifactError::Io)?
        .next()
        .is_some()
    {
        return Err(ManagedArtifactError::UnsafeStorage);
    }
    Ok(())
}

fn remove_known_regular_file_if_present(
    directory: &Dir,
    name: &str,
) -> Result<(), ManagedArtifactError> {
    match directory.symlink_metadata(name) {
        Ok(metadata) => {
            validate_private_regular_metadata(&metadata)?;
            directory
                .remove_file(name)
                .map_err(|_| ManagedArtifactError::Io)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(ManagedArtifactError::UnsafeStorage),
    }
}

fn finish_manifestless_removal(directory: &Dir) -> Result<bool, ManagedArtifactError> {
    if directory
        .try_exists(VERSION_MANIFEST)
        .map_err(|_| ManagedArtifactError::Io)?
    {
        return Ok(false);
    }
    let mut entries = directory.entries().map_err(|_| ManagedArtifactError::Io)?;
    let Some(entry) = entries.next() else {
        return Ok(true);
    };
    let entry = entry.map_err(|_| ManagedArtifactError::Io)?;
    let name = entry
        .file_name()
        .to_str()
        .ok_or(ManagedArtifactError::UnsafeStorage)?
        .to_owned();
    if name != VERSION_REMOVING || entries.next().is_some() {
        return Err(ManagedArtifactError::UnsafeStorage);
    }
    check_marker(directory, VERSION_REMOVING, REMOVING_IDENTITY)?;
    directory
        .remove_file(VERSION_REMOVING)
        .map_err(|_| ManagedArtifactError::Io)?;
    Ok(true)
}

fn try_exclusive_version_use(
    directory: &Dir,
    removing: bool,
) -> Result<ExclusiveVersionUse, ManagedRuntimePublicationError> {
    if removing
        && !directory
            .try_exists(VERSION_USE_LOCK)
            .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?
    {
        return Ok(ExclusiveVersionUse::Acquired(None));
    }
    let lock = open_version_use_lock(directory).map_err(ManagedRuntimePublicationError::Storage)?;
    match lock.try_lock() {
        Ok(()) => Ok(ExclusiveVersionUse::Acquired(Some(lock))),
        Err(fs::TryLockError::WouldBlock) => Ok(ExclusiveVersionUse::InUse),
        Err(fs::TryLockError::Error(_)) => Err(ManagedRuntimePublicationError::Storage(
            ManagedArtifactError::Io,
        )),
    }
}

fn open_version_use_lock(directory: &Dir) -> Result<fs::File, ManagedArtifactError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).follow(FollowSymlinks::No);
    let lock = directory
        .open_with(VERSION_USE_LOCK, &options)
        .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
    let metadata = lock.metadata().map_err(|_| ManagedArtifactError::Io)?;
    validate_private_regular_metadata(&metadata)?;
    check_marker(directory, VERSION_USE_LOCK, USE_LOCK_IDENTITY)?;
    Ok(lock.into_std())
}

fn write_current_pointer(
    current: &Dir,
    identity: &ManagedRuntimeIdentity,
    manifest: &[u8],
    fault: Option<ManagedPublicationBoundary>,
) -> Result<(), ManagedRuntimePublicationError> {
    let pending_name = identity.pending_name();
    if current
        .try_exists(&pending_name)
        .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?
    {
        let metadata = current.symlink_metadata(&pending_name).map_err(|_| {
            ManagedRuntimePublicationError::Storage(ManagedArtifactError::UnsafeStorage)
        })?;
        validate_private_regular_metadata(&metadata)
            .map_err(ManagedRuntimePublicationError::Storage)?;
        current
            .remove_file(&pending_name)
            .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?;
    }
    let pointer = format!(
        "VSIFT-MANAGED-POINTER-v1\ncomponent={}\nversion={}\nmanifest_sha256={}\n",
        identity.component(),
        identity.version(),
        sha256_hex(manifest)
    );
    write_marker(current, &pending_name, pointer.as_bytes())
        .map_err(ManagedRuntimePublicationError::Storage)?;
    inject_publication_fault(fault, ManagedPublicationBoundary::PointerPrepared)?;
    current
        .rename(&pending_name, current, identity.current_name())
        .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?;
    inject_publication_fault(fault, ManagedPublicationBoundary::PointerReplaced)
}

fn read_current_pointer(
    current: &Dir,
    name: &str,
) -> Result<CurrentPointer, ManagedRuntimePublicationError> {
    let bytes = read_private_regular_file(current, name, MAX_VERSION_METADATA_BYTES)
        .map_err(ManagedRuntimePublicationError::Storage)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        ManagedRuntimePublicationError::Storage(ManagedArtifactError::UnsafeStorage)
    })?;
    let mut lines = text.lines();
    if lines.next() != Some("VSIFT-MANAGED-POINTER-v1") {
        return Err(ManagedRuntimePublicationError::Storage(
            ManagedArtifactError::UnsafeStorage,
        ));
    }
    let component = required_metadata_value(lines.next(), "component=")?;
    let version = required_metadata_value(lines.next(), "version=")?;
    let manifest_sha256 = required_metadata_value(lines.next(), "manifest_sha256=")?;
    if lines.next().is_some()
        || !text.ends_with('\n')
        || manifest_sha256.len() != 64
        || !manifest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ManagedRuntimePublicationError::Storage(
            ManagedArtifactError::UnsafeStorage,
        ));
    }
    Ok(CurrentPointer {
        identity: ManagedRuntimeIdentity::new(component, version).map_err(|_| {
            ManagedRuntimePublicationError::Storage(ManagedArtifactError::UnsafeStorage)
        })?,
        manifest_sha256: manifest_sha256.to_owned(),
    })
}

fn read_private_regular_file(
    directory: &Dir,
    name: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, ManagedArtifactError> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = directory
        .open_with(name, &options)
        .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
    let metadata = file.metadata().map_err(|_| ManagedArtifactError::Io)?;
    validate_private_regular_metadata(&metadata)?;
    if metadata.len() == 0 || metadata.len() > max_bytes {
        return Err(ManagedArtifactError::UnsafeStorage);
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| ManagedArtifactError::Io)?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ManagedArtifactError::Io)?;
    if bytes.len() as u64 != metadata.len() {
        return Err(ManagedArtifactError::UnsafeStorage);
    }
    Ok(bytes)
}

fn validate_private_regular_metadata(
    metadata: &cap_std::fs::Metadata,
) -> Result<(), ManagedArtifactError> {
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(ManagedArtifactError::UnsafeStorage);
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(ManagedArtifactError::UnsafeStorage);
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn inject_publication_fault(
    configured: Option<ManagedPublicationBoundary>,
    reached: ManagedPublicationBoundary,
) -> Result<(), ManagedRuntimePublicationError> {
    if configured == Some(reached) {
        Err(ManagedRuntimePublicationError::Storage(
            ManagedArtifactError::Io,
        ))
    } else {
        Ok(())
    }
}

fn validate_runtime_file(
    metadata: &cap_std::fs::Metadata,
    mode: RuntimeFileMode,
) -> Result<(), ManagedArtifactError> {
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(ManagedArtifactError::UnsafeStorage);
    }
    #[cfg(unix)]
    {
        let expected = match mode {
            RuntimeFileMode::PrivateData => 0o600,
            RuntimeFileMode::OwnerExecutable => 0o700,
        };
        if metadata.permissions().mode() & 0o777 != expected {
            return Err(ManagedArtifactError::UnsafeStorage);
        }
    }
    #[cfg(windows)]
    let _ = mode;
    Ok(())
}

fn write_marker(directory: &Dir, name: &str, expected: &[u8]) -> Result<(), ManagedArtifactError> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = directory
        .open_with(name, &options)
        .map_err(|_| ManagedArtifactError::Io)?;
    file.write_all(expected)
        .and_then(|()| file.sync_all())
        .map_err(|_| ManagedArtifactError::Io)
}

fn check_marker(directory: &Dir, name: &str, expected: &[u8]) -> Result<(), ManagedArtifactError> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = directory
        .open_with(name, &options)
        .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
    let metadata = file.metadata().map_err(|_| ManagedArtifactError::Io)?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() != expected.len() as u64 {
        return Err(ManagedArtifactError::UnsafeStorage);
    }
    let mut bytes = vec![0_u8; expected.len()];
    file.read_exact(&mut bytes)
        .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
    if bytes != expected {
        return Err(ManagedArtifactError::UnsafeStorage);
    }
    Ok(())
}

fn map_private_error(error: PrivateRootError) -> ManagedArtifactError {
    match error {
        PrivateRootError::Unavailable => ManagedArtifactError::Unavailable,
        PrivateRootError::UnsafeStorage => ManagedArtifactError::UnsafeStorage,
        PrivateRootError::Busy => ManagedArtifactError::Busy,
        PrivateRootError::Io => ManagedArtifactError::Io,
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

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs,
        io::{Cursor, Read},
        path::PathBuf,
    };

    use sha2::{Digest, Sha256};
    use tar::{Builder, Header};
    use vsift_domain::ArtifactIntegrity;

    use super::{
        ArtifactTransferError, INSTALL_LOCK, MAX_MANAGED_KEY_BYTES, ManagedArtifactError,
        ManagedArtifactStore, ManagedInstallGuard, ManagedPayloadError, ManagedPublicationBoundary,
        ManagedRemovalBoundary, ManagedRuntimeIdentity, ManagedRuntimeLayoutError,
        ManagedRuntimePublicationError, ManagedVersionRemovalOutcome, PAYLOAD,
        PublishedManagedRuntime, RUNTIME, ReviewedArchiveFile, ReviewedPayloadArchive,
        ReviewedRuntimeAlias, ReviewedRuntimeLayout, TarInventoryError, VERSION_MANIFEST, VERSIONS,
        hex,
    };
    use crate::ArchiveInventoryBounds;

    fn fixture_root() -> Result<PathBuf, Box<dyn Error>> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| std::io::Error::other("random source failed"))?;
        let parent = std::env::temp_dir().join(format!("vsift-managed-test-{}", hex(&random)));
        fs::create_dir(&parent)?;
        Ok(parent)
    }

    fn abc_integrity() -> Result<ArtifactIntegrity, Box<dyn Error>> {
        Ok(ArtifactIntegrity::from_sha256_hex(
            3,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )?)
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

    fn selected_tool() -> Result<ReviewedArchiveFile<'static>, Box<dyn Error>> {
        Ok(ReviewedArchiveFile {
            path: "root/tool",
            integrity: abc_integrity()?,
        })
    }

    fn reviewed_bounds() -> Result<ArchiveInventoryBounds, Box<dyn Error>> {
        Ok(ArchiveInventoryBounds::new(2, 100)?)
    }

    #[test]
    fn installation_guard_is_exclusive_and_releases_without_retry() -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let root = parent.join("managed");
            let first = ManagedArtifactStore::at(root.clone())?;
            let second = ManagedArtifactStore::at(root)?;
            let guard = first.try_install_guard()?;
            assert!(matches!(
                second.try_install_guard(),
                Err(ManagedArtifactError::Busy)
            ));
            drop(guard);
            let next = second.try_install_guard()?;
            drop(next);
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn linked_installation_lock_is_rejected_and_external_file_is_preserved()
    -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let root = parent.join("managed");
            let store = ManagedArtifactStore::at(root.clone())?;
            drop(store.try_install_guard()?);
            let external = parent.join("external-lock-link");
            fs::hard_link(root.join(INSTALL_LOCK), &external)?;
            assert!(matches!(
                store.try_install_guard(),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            assert!(external.is_file());
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[cfg(unix)]
    #[test]
    fn non_private_installation_lock_mode_is_rejected() -> Result<(), Box<dyn Error>> {
        use std::os::unix::fs::PermissionsExt;

        let parent = fixture_root()?;
        let result = (|| {
            let root = parent.join("managed");
            let store = ManagedArtifactStore::at(root.clone())?;
            drop(store.try_install_guard()?);
            fs::set_permissions(root.join(INSTALL_LOCK), fs::Permissions::from_mode(0o644))?;
            assert!(matches!(
                store.try_install_guard(),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn owned_payload_assembly_rechecks_selected_bytes_and_discards_only_selection()
    -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let root = parent.join("managed");
            let store = ManagedArtifactStore::at(root.clone())?;
            let (bytes, integrity) = reviewed_tar()?;
            let artifact = store.import_verified(&bytes[..], integrity)?;
            let selected = [selected_tool()?];
            let payload = artifact.stage_reviewed_payload(
                ReviewedPayloadArchive::Tar {
                    max_tar_bytes: 10_000,
                },
                reviewed_bounds()?,
                &[],
                &selected,
            )?;
            assert_eq!(payload.selected_names(), ["tool"]);
            let mut observed = Vec::new();
            payload
                .open_selected_file("tool")?
                .read_to_end(&mut observed)?;
            assert_eq!(observed, b"abc");
            assert!(matches!(
                payload.open_selected_file("other"),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            fs::write(artifact.stage_path.join(PAYLOAD).join("tool"), b"abd")?;
            assert!(matches!(
                payload.open_selected_file("tool"),
                Err(ManagedArtifactError::Transfer(_))
            ));
            payload.discard()?;
            artifact.discard()?;
            assert_eq!(fs::read_dir(root)?.count(), 1);
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn reviewed_runtime_copies_selected_bytes_and_regular_alias_without_activation()
    -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            let (archive, integrity) = reviewed_tar()?;
            let artifact = store.import_verified(&archive[..], integrity)?;
            let selected = [selected_tool()?];
            let payload = artifact.stage_reviewed_payload(
                ReviewedPayloadArchive::Tar {
                    max_tar_bytes: 10_000,
                },
                reviewed_bounds()?,
                &[],
                &selected,
            )?;
            let aliases = [ReviewedRuntimeAlias {
                name: "tool-alias",
                source_selected: "tool",
            }];
            let runtime = payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
                max_bytes: 6,
                aliases: &aliases,
                executables: &["tool"],
            })?;
            assert_eq!(runtime.reviewed_names(), ["tool", "tool-alias"]);
            runtime.recheck_all()?;
            let mut observed = Vec::new();
            runtime
                .open_reviewed_file("tool-alias")?
                .read_to_end(&mut observed)?;
            assert_eq!(observed, b"abc");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(artifact.stage_path.join(RUNTIME).join("tool"))?
                        .permissions()
                        .mode()
                        & 0o777,
                    0o700
                );
                assert_eq!(
                    fs::metadata(artifact.stage_path.join(RUNTIME).join("tool-alias"))?
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
                fs::set_permissions(
                    artifact.stage_path.join(RUNTIME).join("tool"),
                    fs::Permissions::from_mode(0o600),
                )?;
                assert!(matches!(
                    runtime.open_reviewed_file("tool"),
                    Err(ManagedArtifactError::UnsafeStorage)
                ));
                fs::set_permissions(
                    artifact.stage_path.join(RUNTIME).join("tool"),
                    fs::Permissions::from_mode(0o700),
                )?;
            }
            fs::write(artifact.stage_path.join(RUNTIME).join("tool-alias"), b"abd")?;
            assert!(matches!(
                runtime.open_reviewed_file("tool-alias"),
                Err(ManagedArtifactError::Transfer(_))
            ));
            runtime.discard()?;
            assert!(!artifact.stage_path.join(RUNTIME).exists());
            payload.open_selected_file("tool")?;
            payload.discard()?;
            artifact.discard()?;
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn publication_selects_immutable_version_and_preserves_previous_version()
    -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            let guard = store.try_install_guard()?;
            let (archive, integrity) = reviewed_tar()?;
            let first_identity = ManagedRuntimeIdentity::new("whisper-cli", "1.9.2-linux-x64")?;
            let first_artifact = store.import_verified(&archive[..], integrity)?;
            let selected = [selected_tool()?];
            let first_payload = first_artifact.stage_reviewed_payload(
                ReviewedPayloadArchive::Tar {
                    max_tar_bytes: 10_000,
                },
                reviewed_bounds()?,
                &[],
                &selected,
            )?;
            let mut first_runtime =
                first_payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
                    max_bytes: 3,
                    aliases: &[],
                    executables: &["tool"],
                })?;
            let published = first_runtime
                .publish_and_select(&guard, &first_identity)
                .map_err(|error| std::io::Error::other(format!("publish first: {error:?}")))?;
            assert_eq!(published.identity(), &first_identity);
            assert_eq!(published.reviewed_names(), ["tool"]);
            let mut observed = Vec::new();
            published
                .open_reviewed_file("tool")?
                .read_to_end(&mut observed)?;
            assert_eq!(observed, b"abc");
            first_runtime.discard()?;
            first_payload.discard()?;
            first_artifact.discard()?;

            let second_identity = ManagedRuntimeIdentity::new("whisper-cli", "1.9.3-linux-x64")?;
            let second_artifact = store.import_verified(&archive[..], integrity)?;
            let second_payload = second_artifact.stage_reviewed_payload(
                ReviewedPayloadArchive::Tar {
                    max_tar_bytes: 10_000,
                },
                reviewed_bounds()?,
                &[],
                &selected,
            )?;
            let mut second_runtime =
                second_payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
                    max_bytes: 3,
                    aliases: &[],
                    executables: &["tool"],
                })?;
            second_runtime
                .publish_and_select(&guard, &second_identity)
                .map_err(|error| std::io::Error::other(format!("publish second: {error:?}")))?;
            second_runtime.discard()?;
            second_payload.discard()?;
            second_artifact.discard()?;

            let current = store
                .open_selected_runtime("whisper-cli")?
                .ok_or("selected runtime missing")?;
            assert_eq!(current.identity(), &second_identity);
            published.open_reviewed_file("tool")?;
            let previous = store.open_published_runtime(&first_identity)?;
            assert_eq!(previous.identity(), &first_identity);
            previous.open_reviewed_file("tool")?;
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn interrupted_pointer_publication_preserves_selection_and_retry_is_idempotent()
    -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            let guard = store.try_install_guard()?;
            let (archive, integrity) = reviewed_tar()?;
            let selected = [selected_tool()?];

            let publish = |identity: &ManagedRuntimeIdentity,
                           fault: Option<ManagedPublicationBoundary>|
             -> Result<(), Box<dyn Error>> {
                let artifact = store.import_verified(&archive[..], integrity)?;
                let payload = artifact.stage_reviewed_payload(
                    ReviewedPayloadArchive::Tar {
                        max_tar_bytes: 10_000,
                    },
                    reviewed_bounds()?,
                    &[],
                    &selected,
                )?;
                let mut runtime = payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
                    max_bytes: 3,
                    aliases: &[],
                    executables: &["tool"],
                })?;
                let outcome = runtime.publish_and_select_at_boundary(&guard, identity, fault);
                runtime.discard()?;
                payload.discard()?;
                artifact.discard()?;
                outcome.map(drop).map_err(Into::into)
            };

            let first = ManagedRuntimeIdentity::new("whisper-cli", "1.9.2-linux-x64")?;
            publish(&first, None)?;
            let second = ManagedRuntimeIdentity::new("whisper-cli", "1.9.3-linux-x64")?;
            let interrupted = publish(&second, Some(ManagedPublicationBoundary::PointerPrepared));
            assert!(matches!(
                interrupted,
                Err(error)
                    if error.downcast_ref::<ManagedRuntimePublicationError>().is_some()
            ));
            assert_eq!(
                store
                    .open_selected_runtime("whisper-cli")?
                    .ok_or("selected runtime missing")?
                    .identity(),
                &first
            );
            store.open_published_runtime(&second)?;

            publish(&second, None)?;
            assert_eq!(
                store
                    .open_selected_runtime("whisper-cli")?
                    .ok_or("selected runtime missing")?
                    .identity(),
                &second
            );

            let third = ManagedRuntimeIdentity::new("whisper-cli", "1.9.4-linux-x64")?;
            let committed_but_unreported =
                publish(&third, Some(ManagedPublicationBoundary::PointerReplaced));
            assert!(committed_but_unreported.is_err());
            assert_eq!(
                store
                    .open_selected_runtime("whisper-cli")?
                    .ok_or("selected runtime missing")?
                    .identity(),
                &third
            );
            publish(&third, None)?;
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn publication_rejects_guard_from_another_root_and_linked_version_manifest()
    -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let root = parent.join("managed");
            let store = ManagedArtifactStore::at(root.clone())?;
            let other = ManagedArtifactStore::at(parent.join("other-managed"))?;
            let wrong_guard = other.try_install_guard()?;
            let (archive, integrity) = reviewed_tar()?;
            let selected = [selected_tool()?];
            let artifact = store.import_verified(&archive[..], integrity)?;
            let payload = artifact.stage_reviewed_payload(
                ReviewedPayloadArchive::Tar {
                    max_tar_bytes: 10_000,
                },
                reviewed_bounds()?,
                &[],
                &selected,
            )?;
            let mut runtime = payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
                max_bytes: 3,
                aliases: &[],
                executables: &["tool"],
            })?;
            let identity = ManagedRuntimeIdentity::new("whisper-cli", "1.9.2-linux-x64")?;
            assert!(matches!(
                runtime.publish_and_select(&wrong_guard, &identity),
                Err(ManagedRuntimePublicationError::Storage(
                    ManagedArtifactError::UnsafeStorage
                ))
            ));
            drop(wrong_guard);
            let guard = store.try_install_guard()?;
            runtime.publish_and_select(&guard, &identity)?;
            runtime.discard()?;
            payload.discard()?;
            artifact.discard()?;

            let manifest = root
                .join("versions-v1")
                .join("whisper-cli--1.9.2-linux-x64")
                .join(VERSION_MANIFEST);
            let external = parent.join("external-version-marker");
            fs::hard_link(&manifest, &external)?;
            assert!(matches!(
                store.open_published_runtime(&identity),
                Err(ManagedRuntimePublicationError::Storage(
                    ManagedArtifactError::UnsafeStorage
                ))
            ));
            assert!(external.is_file());
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn conflicting_version_identity_preserves_selected_runtime_and_candidate_cleanup()
    -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            let guard = store.try_install_guard()?;
            let identity = ManagedRuntimeIdentity::new("whisper-cli", "1.9.2-linux-x64")?;
            let (archive, integrity) = reviewed_tar()?;
            let selected = [selected_tool()?];

            let first_artifact = store.import_verified(&archive[..], integrity)?;
            let first_payload = first_artifact.stage_reviewed_payload(
                ReviewedPayloadArchive::Tar {
                    max_tar_bytes: 10_000,
                },
                reviewed_bounds()?,
                &[],
                &selected,
            )?;
            let mut first_runtime =
                first_payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
                    max_bytes: 3,
                    aliases: &[],
                    executables: &["tool"],
                })?;
            first_runtime.publish_and_select(&guard, &identity)?;
            first_runtime.discard()?;
            first_payload.discard()?;
            first_artifact.discard()?;

            let candidate_artifact = store.import_verified(&archive[..], integrity)?;
            let candidate_payload = candidate_artifact.stage_reviewed_payload(
                ReviewedPayloadArchive::Tar {
                    max_tar_bytes: 10_000,
                },
                reviewed_bounds()?,
                &[],
                &selected,
            )?;
            let aliases = [ReviewedRuntimeAlias {
                name: "tool-copy",
                source_selected: "tool",
            }];
            let mut candidate_runtime =
                candidate_payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
                    max_bytes: 6,
                    aliases: &aliases,
                    executables: &["tool"],
                })?;
            assert!(matches!(
                candidate_runtime.publish_and_select(&guard, &identity),
                Err(ManagedRuntimePublicationError::VersionConflict)
            ));
            candidate_runtime.discard()?;
            candidate_payload.discard()?;
            candidate_artifact.discard()?;
            let current = store
                .open_selected_runtime("whisper-cli")?
                .ok_or("selected runtime missing")?;
            assert_eq!(current.identity(), &identity);
            assert_eq!(current.reviewed_names(), ["tool"]);
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn managed_runtime_identity_rejects_path_and_noncanonical_keys() {
        for (component, version) in [
            ("", "1"),
            ("Whisper", "1"),
            ("whisper/cli", "1"),
            ("whisper", "../1"),
            ("whisper", "1."),
        ] {
            assert!(matches!(
                ManagedRuntimeIdentity::new(component, version),
                Err(ManagedRuntimePublicationError::InvalidIdentity)
            ));
        }
        assert!(matches!(
            ManagedRuntimeIdentity::new("whisper", "a".repeat(MAX_MANAGED_KEY_BYTES + 1)),
            Err(ManagedRuntimePublicationError::InvalidIdentity)
        ));
    }

    #[test]
    fn selected_runtime_lookup_does_not_create_managed_storage() -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let root = parent.join("managed");
        let store = ManagedArtifactStore::at(root.clone())?;
        assert!(store.open_selected_runtime("whisper-cli")?.is_none());
        assert!(!root.exists());
        fs::remove_dir_all(parent)?;
        Ok(())
    }

    fn assert_removal_fault(
        store: &ManagedArtifactStore,
        guard: &ManagedInstallGuard,
        identity: &ManagedRuntimeIdentity,
        boundary: ManagedRemovalBoundary,
    ) {
        assert!(matches!(
            store.remove_published_runtime_at_boundary(guard, identity, Some(boundary)),
            Err(ManagedRuntimePublicationError::Storage(
                ManagedArtifactError::Io
            ))
        ));
    }

    #[test]
    fn rollback_and_removal_respect_selection_and_live_version_holds() -> Result<(), Box<dyn Error>>
    {
        let parent = fixture_root()?;
        let result = (|| {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            let guard = store.try_install_guard()?;
            let (archive, integrity) = reviewed_tar()?;
            let selected = [selected_tool()?];
            let publish = |identity: &ManagedRuntimeIdentity|
             -> Result<PublishedManagedRuntime, Box<dyn Error>> {
                let artifact = store.import_verified(&archive[..], integrity)?;
                let payload = artifact.stage_reviewed_payload(
                    ReviewedPayloadArchive::Tar {
                        max_tar_bytes: 10_000,
                    },
                    reviewed_bounds()?,
                    &[],
                    &selected,
                )?;
                let mut runtime = payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
                    max_bytes: 3,
                    aliases: &[],
                    executables: &["tool"],
                })?;
                let published = runtime.publish_and_select(&guard, identity)?;
                runtime.discard()?;
                payload.discard()?;
                artifact.discard()?;
                Ok(published)
            };

            let first = ManagedRuntimeIdentity::new("whisper-cli", "1.9.2-linux-x64")?;
            let active_first = publish(&first)?;
            let second = ManagedRuntimeIdentity::new("whisper-cli", "1.9.3-linux-x64")?;
            let active_second = publish(&second)?;
            assert_eq!(
                store.remove_published_runtime(&guard, &second)?,
                ManagedVersionRemovalOutcome::Selected
            );
            assert_eq!(
                store.remove_published_runtime(&guard, &first)?,
                ManagedVersionRemovalOutcome::InUse
            );

            drop(active_first);
            let rollback = store.select_published_runtime(&guard, &first)?;
            assert_eq!(
                store
                    .open_selected_runtime("whisper-cli")?
                    .ok_or("selected runtime missing")?
                    .identity(),
                &first
            );
            assert_eq!(
                store.remove_published_runtime(&guard, &second)?,
                ManagedVersionRemovalOutcome::InUse
            );
            drop(active_second);
            assert_removal_fault(
                &store,
                &guard,
                &second,
                ManagedRemovalBoundary::PayloadFiles,
            );
            assert_removal_fault(&store, &guard, &second, ManagedRemovalBoundary::UseLock);
            assert_removal_fault(
                &store,
                &guard,
                &second,
                ManagedRemovalBoundary::VersionManifest,
            );
            assert_eq!(
                store.remove_published_runtime(&guard, &second)?,
                ManagedVersionRemovalOutcome::Removed
            );
            assert_eq!(
                store.remove_published_runtime(&guard, &second)?,
                ManagedVersionRemovalOutcome::Removed
            );
            let versions = parent.join("managed").join(VERSIONS);
            let substituted = versions.join(second.version_directory_name());
            fs::write(&substituted, b"keep")?;
            assert!(matches!(
                store.remove_published_runtime(&guard, &second),
                Err(ManagedRuntimePublicationError::Storage(
                    ManagedArtifactError::UnsafeStorage
                ))
            ));
            assert_eq!(fs::read(substituted)?, b"keep");
            assert!(matches!(
                store.open_published_runtime(&second),
                Err(ManagedRuntimePublicationError::Storage(
                    ManagedArtifactError::UnsafeStorage
                ))
            ));
            drop(rollback);
            assert_eq!(
                store.remove_published_runtime(&guard, &first)?,
                ManagedVersionRemovalOutcome::Selected
            );
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn invalid_runtime_review_has_no_filesystem_effect() -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            let (archive, integrity) = reviewed_tar()?;
            let artifact = store.import_verified(&archive[..], integrity)?;
            let selected = [selected_tool()?];
            let payload = artifact.stage_reviewed_payload(
                ReviewedPayloadArchive::Tar {
                    max_tar_bytes: 10_000,
                },
                reviewed_bounds()?,
                &[],
                &selected,
            )?;
            for (max_bytes, alias) in [
                (
                    2,
                    ReviewedRuntimeAlias {
                        name: "alias",
                        source_selected: "tool",
                    },
                ),
                (
                    6,
                    ReviewedRuntimeAlias {
                        name: "../escape",
                        source_selected: "tool",
                    },
                ),
                (
                    6,
                    ReviewedRuntimeAlias {
                        name: "alias",
                        source_selected: "missing",
                    },
                ),
                (
                    6,
                    ReviewedRuntimeAlias {
                        name: "TOOL",
                        source_selected: "tool",
                    },
                ),
            ] {
                assert!(matches!(
                    payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
                        max_bytes,
                        aliases: &[alias],
                        executables: &["tool"],
                    }),
                    Err(ManagedRuntimeLayoutError::InvalidReview)
                ));
                assert!(!artifact.stage_path.join(RUNTIME).exists());
            }
            assert!(matches!(
                payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
                    max_bytes: 3,
                    aliases: &[],
                    executables: &["missing"],
                }),
                Err(ManagedRuntimeLayoutError::InvalidReview)
            ));
            assert!(!artifact.stage_path.join(RUNTIME).exists());
            payload.open_selected_file("tool")?;
            payload.discard()?;
            artifact.discard()?;
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn unexpected_runtime_entry_blocks_open_and_cleanup() -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            let (archive, integrity) = reviewed_tar()?;
            let artifact = store.import_verified(&archive[..], integrity)?;
            let selected = [selected_tool()?];
            let payload = artifact.stage_reviewed_payload(
                ReviewedPayloadArchive::Tar {
                    max_tar_bytes: 10_000,
                },
                reviewed_bounds()?,
                &[],
                &selected,
            )?;
            let runtime = payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
                max_bytes: 3,
                aliases: &[],
                executables: &["tool"],
            })?;
            let unexpected = artifact.stage_path.join(RUNTIME).join("unexpected");
            fs::write(&unexpected, b"keep")?;
            assert!(matches!(
                runtime.open_reviewed_file("tool"),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            assert!(matches!(
                runtime.discard(),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            assert_eq!(fs::read(unexpected)?, b"keep");
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[cfg(unix)]
    #[test]
    fn substituted_runtime_directory_cannot_redirect_open_or_cleanup() -> Result<(), Box<dyn Error>>
    {
        let parent = fixture_root()?;
        let result = (|| {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            let (archive, integrity) = reviewed_tar()?;
            let artifact = store.import_verified(&archive[..], integrity)?;
            let selected = [selected_tool()?];
            let payload = artifact.stage_reviewed_payload(
                ReviewedPayloadArchive::Tar {
                    max_tar_bytes: 10_000,
                },
                reviewed_bounds()?,
                &[],
                &selected,
            )?;
            let runtime = payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
                max_bytes: 3,
                aliases: &[],
                executables: &["tool"],
            })?;
            let at_name = artifact.stage_path.join(RUNTIME);
            let held_name = artifact.stage_path.join("runtime-held");
            fs::rename(&at_name, &held_name)?;
            fs::create_dir(&at_name)?;
            fs::write(at_name.join("attacker"), b"keep")?;
            assert!(matches!(
                runtime.open_reviewed_file("tool"),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            assert!(matches!(
                runtime.discard(),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            assert_eq!(fs::read(at_name.join("attacker"))?, b"keep");
            assert_eq!(fs::read(held_name.join("tool"))?, b"abc");
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[cfg(unix)]
    #[test]
    fn hard_linked_runtime_file_is_neither_opened_nor_deleted() -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            let (archive, integrity) = reviewed_tar()?;
            let artifact = store.import_verified(&archive[..], integrity)?;
            let selected = [selected_tool()?];
            let payload = artifact.stage_reviewed_payload(
                ReviewedPayloadArchive::Tar {
                    max_tar_bytes: 10_000,
                },
                reviewed_bounds()?,
                &[],
                &selected,
            )?;
            let runtime = payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
                max_bytes: 3,
                aliases: &[],
                executables: &["tool"],
            })?;
            let outside = parent.join("outside");
            fs::hard_link(artifact.stage_path.join(RUNTIME).join("tool"), &outside)?;
            assert!(matches!(
                runtime.open_reviewed_file("tool"),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            assert!(matches!(
                runtime.discard(),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            assert_eq!(fs::read(outside)?, b"abc");
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn failed_owned_payload_assembly_removes_only_created_directory() -> Result<(), Box<dyn Error>>
    {
        let parent = fixture_root()?;
        let result = (|| {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            let (bytes, integrity) = reviewed_tar()?;
            let artifact = store.import_verified(&bytes[..], integrity)?;
            let wrong = [ReviewedArchiveFile {
                path: "root/tool",
                integrity: ArtifactIntegrity::from_sha256_hex(
                    3,
                    "a52d159f262b2c6ddb724a61840befc36eb30c88877a4030b65cbe86298449c9",
                )?,
            }];
            assert!(matches!(
                artifact.stage_reviewed_payload(
                    ReviewedPayloadArchive::Tar {
                        max_tar_bytes: 10_000
                    },
                    reviewed_bounds()?,
                    &[],
                    &wrong,
                ),
                Err(ManagedPayloadError::Tar(
                    TarInventoryError::SelectedFileMismatch
                ))
            ));
            assert!(!artifact.stage_path.join(PAYLOAD).exists());
            artifact.discard()?;
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn unexpected_payload_content_blocks_discard_and_is_preserved() -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            let (bytes, integrity) = reviewed_tar()?;
            let artifact = store.import_verified(&bytes[..], integrity)?;
            let selected = [selected_tool()?];
            let payload = artifact.stage_reviewed_payload(
                ReviewedPayloadArchive::Tar {
                    max_tar_bytes: 10_000,
                },
                reviewed_bounds()?,
                &[],
                &selected,
            )?;
            let unexpected = artifact.stage_path.join(PAYLOAD).join("unexpected");
            fs::write(&unexpected, b"keep")?;
            assert!(matches!(
                payload.open_selected_file("tool"),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            assert!(matches!(
                payload.discard(),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            assert_eq!(fs::read(unexpected)?, b"keep");
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[cfg(unix)]
    #[test]
    fn replaced_payload_name_cannot_redirect_cleanup() -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            let (bytes, integrity) = reviewed_tar()?;
            let artifact = store.import_verified(&bytes[..], integrity)?;
            let selected = [selected_tool()?];
            let payload = artifact.stage_reviewed_payload(
                ReviewedPayloadArchive::Tar {
                    max_tar_bytes: 10_000,
                },
                reviewed_bounds()?,
                &[],
                &selected,
            )?;
            let at_name = artifact.stage_path.join(PAYLOAD);
            fs::rename(&at_name, artifact.stage_path.join("payload-held"))?;
            fs::create_dir(&at_name)?;
            fs::write(at_name.join("attacker"), b"keep")?;
            assert!(matches!(
                payload.discard(),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            assert_eq!(fs::read(at_name.join("attacker"))?, b"keep");
            assert_eq!(
                fs::read(artifact.stage_path.join("payload-held/tool"))?,
                b"abc"
            );
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[cfg(unix)]
    #[test]
    fn linked_selected_payload_is_not_opened_or_deleted() -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            let (bytes, integrity) = reviewed_tar()?;
            let artifact = store.import_verified(&bytes[..], integrity)?;
            let selected = [selected_tool()?];
            let payload = artifact.stage_reviewed_payload(
                ReviewedPayloadArchive::Tar {
                    max_tar_bytes: 10_000,
                },
                reviewed_bounds()?,
                &[],
                &selected,
            )?;
            let outside = parent.join("outside");
            fs::hard_link(artifact.stage_path.join(PAYLOAD).join("tool"), &outside)?;
            assert!(matches!(
                payload.open_selected_file("tool"),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            assert!(matches!(
                payload.discard(),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            assert_eq!(fs::read(outside)?, b"abc");
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn imports_rechecks_and_discards_only_owned_staging() -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let root = parent.join("managed");
            let store = ManagedArtifactStore::at(root.clone())?;
            let staged = store.import_verified(&b"abc"[..], abc_integrity()?)?;
            let mut bytes = Vec::new();
            staged.open_artifact()?.read_to_end(&mut bytes)?;
            assert_eq!(bytes, b"abc");
            staged.discard()?;
            assert_eq!(fs::read(root.join("owner-v1"))?, b"VSIFT-MANAGED-ROOT-v1\n");
            assert_eq!(fs::read_dir(&root)?.count(), 1);
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn changed_source_cleans_artifact_and_stage() -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let root = parent.join("managed");
            let store = ManagedArtifactStore::at(root.clone())?;
            assert!(matches!(
                store.import_verified(&b"abd"[..], abc_integrity()?),
                Err(ManagedArtifactError::Transfer(_))
            ));
            assert_eq!(fs::read_dir(root)?.count(), 1);
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[tokio::test]
    async fn streaming_import_enforces_exact_bytes_and_discards_failed_stages()
    -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = async {
            let root = parent.join("managed");
            let store = ManagedArtifactStore::at(root.clone())?;
            for (bytes, expected) in [
                (&b"ab"[..], ArtifactTransferError::Truncated),
                (&b"abd"[..], ArtifactTransferError::DigestMismatch),
                (&b"abcd"[..], ArtifactTransferError::Oversized),
            ] {
                let mut stream = store.begin_stream(abc_integrity()?)?;
                let failure = match stream.append(bytes).await {
                    Ok(()) => stream.finish().await.err(),
                    Err(error) => Some(error),
                };
                assert!(matches!(failure, Some(ManagedArtifactError::Transfer(error)) if error == expected));
                stream.abort()?;
                assert_eq!(fs::read_dir(&root)?.count(), 1);
            }
            let mut stream = store.begin_stream(abc_integrity()?)?;
            stream.append(b"a").await?;
            stream.append(b"bc").await?;
            stream.finish().await?;
            let staged = stream.complete();
            assert_eq!(staged.open_artifact()?.metadata()?.len(), 3);
            staged.discard()?;
            assert_eq!(fs::read_dir(root)?.count(), 1);
            Ok::<(), Box<dyn Error>>(())
        }.await;
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn existing_unowned_root_is_not_claimed_or_cleaned() -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let root = parent.join("managed");
            fs::create_dir(&root)?;
            fs::write(root.join("user-file"), b"keep")?;
            let store = ManagedArtifactStore::at(root.clone())?;
            assert!(matches!(
                store.import_verified(&b"abc"[..], abc_integrity()?),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            assert_eq!(fs::read(root.join("user-file"))?, b"keep");
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn unexpected_staging_content_blocks_discard() -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let root = parent.join("managed");
            let store = ManagedArtifactStore::at(root.clone())?;
            let staged = store.import_verified(&b"abc"[..], abc_integrity()?)?;
            let stage_path = fs::read_dir(&root)?
                .filter_map(Result::ok)
                .find(|entry| entry.file_name().to_string_lossy().starts_with("stage-"))
                .ok_or("owned staging directory missing")?
                .path();
            fs::write(stage_path.join("unexpected"), b"keep")?;
            assert!(matches!(
                staged.discard(),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            assert_eq!(fs::read(stage_path.join("unexpected"))?, b"keep");
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[test]
    fn changed_staged_file_fails_recheck_before_archive_use() -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let root = parent.join("managed");
            let store = ManagedArtifactStore::at(root.clone())?;
            let staged = store.import_verified(&b"abc"[..], abc_integrity()?)?;
            let stage_path = fs::read_dir(&root)?
                .filter_map(Result::ok)
                .find(|entry| entry.file_name().to_string_lossy().starts_with("stage-"))
                .ok_or("owned staging directory missing")?
                .path();
            fs::write(stage_path.join("artifact.pending"), b"abd")?;
            assert!(matches!(
                staged.open_artifact(),
                Err(ManagedArtifactError::Transfer(_))
            ));
            staged.discard()?;
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }

    #[cfg(unix)]
    #[test]
    fn replaced_stage_name_cannot_redirect_discard() -> Result<(), Box<dyn Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let root = parent.join("managed");
            let store = ManagedArtifactStore::at(root.clone())?;
            let staged = store.import_verified(&b"abc"[..], abc_integrity()?)?;
            let stage_path = fs::read_dir(&root)?
                .filter_map(Result::ok)
                .find(|entry| entry.file_name().to_string_lossy().starts_with("stage-"))
                .ok_or("owned staging directory missing")?
                .path();
            let moved = root.join("moved-stage");
            fs::rename(&stage_path, &moved)?;
            fs::create_dir(&stage_path)?;
            assert!(matches!(
                staged.discard(),
                Err(ManagedArtifactError::UnsafeStorage)
            ));
            assert_eq!(fs::read(moved.join("artifact.pending"))?, b"abc");
            assert!(stage_path.is_dir());
            Ok::<(), Box<dyn Error>>(())
        })();
        fs::remove_dir_all(parent)?;
        result
    }
}
