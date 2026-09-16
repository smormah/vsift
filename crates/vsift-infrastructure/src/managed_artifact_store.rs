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
const MAX_RUNTIME_FILES: usize = 128;
const MAX_RUNTIME_BYTES: u64 = 1_073_741_824;
const ROOT_IDENTITY: &[u8] = b"VSIFT-MANAGED-ROOT-v1\n";
const STAGE_IDENTITY: &[u8] = b"VSIFT-MANAGED-STAGE-v1\n";

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
        Ok(ManagedInstallGuard { _lock: lock })
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

#[derive(Clone, Copy)]
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
            runtime,
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
    runtime: Dir,
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
        check_marker(&artifact.root, ROOT_MARKER, ROOT_IDENTITY)?;
        check_marker(&artifact.stage, STAGE_MARKER, STAGE_IDENTITY)?;
        artifact.validate_stage_at_name()?;
        let at_name = artifact
            .stage
            .open_dir_nofollow(RUNTIME)
            .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
        validate_same_held_directory(&at_name, &self.runtime).map_err(map_private_error)?;
        validate_private_root(&artifact.stage_path.join(RUNTIME), &self.runtime)
            .map_err(map_private_error)?;
        let reviewed = self
            .files
            .iter()
            .find(|file| file.name == name)
            .ok_or(ManagedArtifactError::UnsafeStorage)?;
        self.validate_exact_contents()?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = self
            .runtime
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
        let mut observed = HashSet::with_capacity(self.files.len());
        for entry in self
            .runtime
            .entries()
            .map_err(|_| ManagedArtifactError::Io)?
        {
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
            let metadata = self
                .runtime
                .symlink_metadata(name)
                .map_err(|_| ManagedArtifactError::UnsafeStorage)?;
            validate_runtime_file(&metadata, reviewed.mode)?;
        }
        if observed.len() != self.files.len() {
            return Err(ManagedArtifactError::UnsafeStorage);
        }
        Ok(())
    }

    /// Discards only reviewed runtime copies; original payload remains available.
    ///
    /// # Errors
    ///
    /// Unexpected or linked entries prevent deletion.
    pub fn discard(self) -> Result<(), ManagedArtifactError> {
        let reviewed_names = self
            .files
            .iter()
            .map(|file| file.name.clone())
            .collect::<Vec<_>>();
        self.payload
            .artifact
            .remove_reviewed_runtime(self.runtime, &reviewed_names)
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
        ArtifactTransferError, INSTALL_LOCK, ManagedArtifactError, ManagedArtifactStore,
        ManagedPayloadError, ManagedRuntimeLayoutError, PAYLOAD, RUNTIME, ReviewedArchiveFile,
        ReviewedPayloadArchive, ReviewedRuntimeAlias, ReviewedRuntimeLayout, TarInventoryError,
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
