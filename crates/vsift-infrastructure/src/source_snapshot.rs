//! Bounded source copying into a capability-scoped private session directory.

use std::{
    error::Error,
    fmt,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, File, OpenOptions};
use sha2::{Digest, Sha256};
use vsift_application::{
    ForegroundSessionPort, OpenSessionError, SessionStorageError, StagedSessionSource,
};
use vsift_domain::{OperationId, SessionId, SourceId};

use crate::{
    FilesystemSessionStore, SessionReadHold, SessionRegistration, source_binding::FileIdentity,
};

/// Maximum source size in the accepted desktop profile.
pub const MAX_SOURCE_BYTES: u64 = 20 * 1024 * 1024 * 1024;
/// Maximum time for an opened source stream to be copied or rehashed.
pub const MAX_SOURCE_READ_DURATION: Duration = Duration::from_secs(600);
const HEX: &[u8; 16] = b"0123456789abcdef";

/// Container families allowed by the R0 media adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceContainer {
    /// ISO base media (MP4/MOV); external data references remain disabled by provider policy.
    IsoMedia,
    /// Matroska/WebM with embedded tracks only.
    Matroska,
}

impl SourceContainer {
    /// Fixed demuxer name supplied to `FFmpeg` and `FFprobe`.
    #[must_use]
    pub const fn demuxer(self) -> &'static str {
        match self {
            Self::IsoMedia => "mov",
            Self::Matroska => "matroska",
        }
    }
}

/// An immutable-by-contract private copy tied to a held session lifetime.
pub struct SourceSnapshot {
    session_id: SessionId,
    id: SourceId,
    bytes: u64,
    container: SourceContainer,
    file_name: String,
    directory: Dir,
    directory_path: PathBuf,
    _hold: SessionReadHold,
    /// Test seam: how many times this copy's bytes were read whole to verify
    /// them (issue #148 requires exactly two per bound operation).
    #[cfg(test)]
    full_hashes: std::sync::atomic::AtomicU32,
}

impl SourceSnapshot {
    /// Stages bytes from one directly selected, local regular file into an initialized session.
    /// The file is opened no-follow relative to a held parent. A SHA-256 of the copied bytes,
    /// not path metadata, is the source identity. The original is never written or deleted.
    ///
    /// # Errors
    /// Returns a typed rejection for unsafe input, size, container, source mutation, or I/O.
    pub fn stage(
        store: &FilesystemSessionStore,
        session_id: &SessionId,
        operation_id: &OperationId,
        source_path: &Path,
    ) -> Result<Self, SourceError> {
        let (mut source, initial) = open_source(source_path)?;
        if initial.len() > MAX_SOURCE_BYTES {
            return Err(SourceError::TooLarge);
        }
        let _admission = store.try_admit(1).map_err(SourceError::Storage)?;
        let (directory, directory_path, hold) = store
            .source_artifact_directory(session_id)
            .map_err(SourceError::Storage)?;
        let file_name = format!("source-{}.media", operation_id.as_str());
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let mut output = directory
            .open_with(&file_name, &options)
            .map_err(SourceError::Io)?;
        let result = copy_bounded(&mut source, &mut output).and_then(|(id, bytes, container)| {
            output.sync_all().map_err(SourceError::Io)?;
            let final_meta = source.metadata().map_err(SourceError::Io)?;
            if initial.len() != final_meta.len()
                || initial.modified().ok() != final_meta.modified().ok()
            {
                return Err(SourceError::ChangedDuringStage);
            }
            Ok((id, bytes, container))
        });
        let (id, bytes, container) = match result {
            Ok(value) => value,
            Err(error) => {
                drop(output);
                let _ = directory.remove_file(&file_name);
                return Err(error);
            }
        };
        drop(output);
        Ok(Self {
            session_id: session_id.clone(),
            id,
            bytes,
            container,
            file_name,
            directory,
            directory_path,
            _hold: hold,
            #[cfg(test)]
            full_hashes: std::sync::atomic::AtomicU32::new(0),
        })
    }

    /// Reopens the committed private source copy of an open session.
    ///
    /// Later operations on a session (local speech recognition today, frames
    /// and audio later) work on the copy `ingest` committed, never on the
    /// user's original. The copy is opened no-follow inside the held artifact
    /// directory and rehashed: its bytes must still be the identity and size
    /// the session committed, and its container is detected again from them.
    /// The returned snapshot keeps the session's shared lifetime hold, so the
    /// session cannot be closed or cleaned while it is in use.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Storage`] for a missing, closed, expired or busy
    /// session, and [`SourceError::SnapshotChanged`] when the copy no longer
    /// matches its committed identity.
    pub fn open_committed(
        store: &FilesystemSessionStore,
        session_id: &SessionId,
        now_unix_seconds: u64,
    ) -> Result<Self, SourceError> {
        Self::open_committed_identified(store, session_id, now_unix_seconds)
            .map(|(snapshot, _)| snapshot)
    }

    /// [`Self::open_committed`], also returning the on-disk identity of the
    /// file that was hashed, for a [`BoundSource`](crate::BoundSource).
    pub(crate) fn open_committed_identified(
        store: &FilesystemSessionStore,
        session_id: &SessionId,
        now_unix_seconds: u64,
    ) -> Result<(Self, FileIdentity), SourceError> {
        let committed = store
            .committed_source(session_id, now_unix_seconds)
            .map_err(SourceError::Storage)?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = committed
            .directory
            .open_with(&committed.file_name, &options)
            .map_err(|_| SourceError::SnapshotChanged)?;
        let (id, container, identity) = read_identified(&mut file, committed.bytes)?;
        if id != committed.source_id {
            return Err(SourceError::SnapshotChanged);
        }
        Ok((
            Self {
                session_id: session_id.clone(),
                id,
                bytes: committed.bytes,
                container,
                file_name: committed.file_name,
                directory: committed.directory,
                directory_path: committed.directory_path,
                _hold: committed.hold,
                #[cfg(test)]
                full_hashes: std::sync::atomic::AtomicU32::new(1),
            },
            identity,
        ))
    }

    /// Cryptographic identity of the staged bytes.
    #[must_use]
    pub fn id(&self) -> &SourceId {
        &self.id
    }

    /// Session whose shared lifetime hold protects this private copy.
    #[must_use]
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// The held artifact directory the snapshot lives in, for installing
    /// evidence published together with activation.
    pub(crate) const fn artifact_directory(&self) -> &Dir {
        &self.directory
    }

    pub(crate) fn file_name(&self) -> &str {
        &self.file_name
    }

    /// Size of the staged source.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Validated container family.
    #[must_use]
    pub const fn container(&self) -> SourceContainer {
        self.container
    }

    /// Canonical private path for providers that cannot consume the held handle.
    #[must_use]
    pub fn provider_path(&self) -> PathBuf {
        self.directory_path.join(&self.file_name)
    }

    /// Rehashes the private copy immediately before a provider operation.
    ///
    /// This is the per-call binding of ADR 0012. An operation that calls a
    /// provider many times binds the copy once with a
    /// [`BoundSource`](crate::BoundSource) instead (issue #148).
    ///
    /// # Errors
    /// Rejects a removed, substituted, or changed snapshot.
    pub fn verify(&self) -> Result<(), SourceError> {
        let file = self.open_copy().map_err(SourceError::Io)?;
        self.rehash(file).map(|_| ())
    }

    /// Opens the private copy no-follow inside the held artifact directory.
    pub(crate) fn open_copy(&self) -> std::io::Result<File> {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        self.directory.open_with(&self.file_name, &options)
    }

    /// Reads the whole opened copy, requires its SHA-256 and size to be the
    /// snapshot's, and returns the identity of the file that was read.
    pub(crate) fn rehash(&self, mut file: File) -> Result<FileIdentity, SourceError> {
        #[cfg(test)]
        self.full_hashes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (id, _, identity) = read_identified(&mut file, self.bytes)?;
        if id != self.id {
            return Err(SourceError::SnapshotChanged);
        }
        Ok(identity)
    }

    /// How many times this copy was read whole to verify it.
    #[cfg(test)]
    pub(crate) fn full_hashes(&self) -> u32 {
        self.full_hashes.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Rehashes the original selected path to detect a changed or replaced source.
    ///
    /// # Errors
    /// Returns a typed input/I/O failure if the selected path cannot be safely reopened.
    pub fn original_changed(&self, source_path: &Path) -> Result<bool, SourceError> {
        let (mut file, _) = match open_source(source_path) {
            Ok(opened) => opened,
            Err(SourceError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(true);
            }
            Err(error) => return Err(error),
        };
        match copy_bounded(&mut file, &mut std::io::sink()) {
            Ok((id, bytes, _)) => Ok(id != self.id || bytes != self.bytes),
            Err(SourceError::UnsupportedContainer) => Ok(true),
            Err(error) => Err(error),
        }
    }
}

impl StagedSessionSource for SourceSnapshot {
    fn source_id(&self) -> &SourceId {
        self.id()
    }

    fn source_bytes(&self) -> u64 {
        self.bytes()
    }
}

impl ForegroundSessionPort for FilesystemSessionStore {
    type Registration = SessionRegistration;
    type Snapshot = SourceSnapshot;

    fn register(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        now_unix_seconds: u64,
    ) -> Result<Self::Registration, OpenSessionError> {
        self.register_session(session_id, operation_id, now_unix_seconds)
            .map_err(OpenSessionError::Storage)
    }

    fn stage_source(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        source: &Path,
    ) -> Result<Self::Snapshot, OpenSessionError> {
        SourceSnapshot::stage(self, session_id, operation_id, source).map_err(|error| match error {
            SourceError::Storage(storage) => OpenSessionError::Storage(storage),
            SourceError::Io(_) => OpenSessionError::SourceIo,
            SourceError::InvalidPath
            | SourceError::NotRegularFile
            | SourceError::TooLarge
            | SourceError::Deadline
            | SourceError::UnsupportedContainer
            | SourceError::ChangedDuringStage
            | SourceError::SnapshotChanged
            | SourceError::IdentityFailure => OpenSessionError::InvalidSource,
        })
    }

    fn activate(
        &self,
        snapshot: &Self::Snapshot,
        operation_id: &OperationId,
        expected_generation: vsift_domain::StorageGeneration,
        now_unix_seconds: u64,
    ) -> Result<vsift_domain::StorageGeneration, OpenSessionError> {
        self.activate_source(
            snapshot,
            operation_id,
            expected_generation,
            now_unix_seconds,
        )
        .map_err(OpenSessionError::Storage)
    }

    fn activate_with_transcript(
        &self,
        snapshot: &Self::Snapshot,
        operation_id: &OperationId,
        expected_generation: vsift_domain::StorageGeneration,
        now_unix_seconds: u64,
        transcript: &vsift_domain::TranscriptRevision,
    ) -> Result<vsift_domain::StorageGeneration, OpenSessionError> {
        let record =
            crate::encode_transcript_record(transcript).map_err(OpenSessionError::Storage)?;
        self.activate_source_with_artifact(
            snapshot,
            operation_id,
            expected_generation,
            now_unix_seconds,
            vsift_domain::SessionArtifactKind::TranscriptRecord,
            &record,
        )
        .map_err(OpenSessionError::Storage)
    }
}

/// Opens one directly named, local regular file without following a final link.
///
/// Shared by source staging and supplied-transcript import so every
/// caller-selected input passes the same path policy.
pub(crate) fn open_source(path: &Path) -> Result<(File, cap_std::fs::Metadata), SourceError> {
    if !path.is_absolute() || !local_path(path) {
        return Err(SourceError::InvalidPath);
    }
    let name = path.file_name().ok_or(SourceError::InvalidPath)?;
    if invalid_source_name(name) {
        return Err(SourceError::InvalidPath);
    }
    let parent = path.parent().ok_or(SourceError::InvalidPath)?;
    let canonical_parent = std::fs::canonicalize(parent).map_err(SourceError::Io)?;
    if !local_path(&canonical_parent) {
        return Err(SourceError::InvalidPath);
    }
    let directory = Dir::open_ambient_dir(canonical_parent, cap_std::ambient_authority())
        .map_err(SourceError::Io)?;
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = directory
        .open_with(Path::new(name), &options)
        .map_err(SourceError::Io)?;
    let metadata = file.metadata().map_err(SourceError::Io)?;
    if !metadata.is_file() {
        return Err(SourceError::NotRegularFile);
    }
    Ok((file, metadata))
}

#[cfg(windows)]
fn local_path(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    matches!(path.components().next(), Some(Component::Prefix(prefix))
        if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
}

#[cfg(not(windows))]
fn local_path(path: &Path) -> bool {
    !path.as_os_str().as_encoded_bytes().starts_with(b"//")
}

#[cfg(windows)]
fn invalid_source_name(name: &std::ffi::OsStr) -> bool {
    let text = name.to_string_lossy();
    let stem = text
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    text.contains(':')
        || text.ends_with([' ', '.'])
        || matches!(
            stem.as_str(),
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

#[cfg(not(windows))]
fn invalid_source_name(_name: &std::ffi::OsStr) -> bool {
    false
}

/// Hashes one opened private copy of `expected_bytes` and returns its SHA-256
/// source identity, its container and its on-disk identity.
///
/// The identity is read from the same handle before and after the bytes, and
/// must not change in between, so it is the identity of exactly the file whose
/// bytes were hashed: a write during the read, or a file of another size, is a
/// changed snapshot.
fn read_identified(
    file: &mut File,
    expected_bytes: u64,
) -> Result<(SourceId, SourceContainer, FileIdentity), SourceError> {
    let before = file.metadata().map_err(SourceError::Io)?;
    if !before.is_file() || before.len() != expected_bytes {
        return Err(SourceError::SnapshotChanged);
    }
    let (id, bytes, container) = copy_bounded(file, &mut std::io::sink())?;
    let after = file.metadata().map_err(SourceError::Io)?;
    let identity = FileIdentity::of(&after);
    if bytes != expected_bytes || FileIdentity::of(&before) != identity {
        return Err(SourceError::SnapshotChanged);
    }
    Ok((id, container, identity))
}

fn copy_bounded(
    source: &mut impl Read,
    destination: &mut impl Write,
) -> Result<(SourceId, u64, SourceContainer), SourceError> {
    let started = Instant::now();
    let mut hash = Sha256::new();
    let mut header = [0_u8; 12];
    let mut header_count = 0_usize;
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        if started.elapsed() > MAX_SOURCE_READ_DURATION {
            return Err(SourceError::Deadline);
        }
        let read = source.read(&mut buffer).map_err(SourceError::Io)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(read).map_err(|_| SourceError::TooLarge)?)
            .ok_or(SourceError::TooLarge)?;
        if total > MAX_SOURCE_BYTES {
            return Err(SourceError::TooLarge);
        }
        let take = (header.len() - header_count).min(read);
        header[header_count..header_count + take].copy_from_slice(&buffer[..take]);
        header_count += take;
        destination
            .write_all(&buffer[..read])
            .map_err(SourceError::Io)?;
        hash.update(&buffer[..read]);
    }
    let container = if header_count >= 8 && &header[4..8] == b"ftyp" {
        SourceContainer::IsoMedia
    } else if header_count >= 4 && header[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
        SourceContainer::Matroska
    } else {
        return Err(SourceError::UnsupportedContainer);
    };
    let mut digest = String::with_capacity(64);
    for byte in hash.finalize() {
        digest.push(char::from(HEX[usize::from(byte >> 4)]));
        digest.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    let id = SourceId::from_sha256(&digest).map_err(|_| SourceError::IdentityFailure)?;
    Ok((id, total, container))
}

/// Expected source-binding and staging failures.
#[derive(Debug)]
pub enum SourceError {
    /// Selection was relative, ambiguous, or otherwise unsafe.
    InvalidPath,
    /// Selection did not resolve to a regular file.
    NotRegularFile,
    /// Selected source exceeds the profile maximum.
    TooLarge,
    /// Source read exceeded the stage deadline.
    Deadline,
    /// Only embedded-track ISO media and Matroska containers are admitted.
    UnsupportedContainer,
    /// The source changed while the private copy was made.
    ChangedDuringStage,
    /// The private snapshot no longer matches its cryptographic identity.
    SnapshotChanged,
    /// Private workspace coordination or admission failed.
    Storage(SessionStorageError),
    /// A validated digest could not be represented by the domain identity.
    IdentityFailure,
    /// Filesystem access failed.
    Io(std::io::Error),
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath => formatter.write_str("source path is invalid"),
            Self::NotRegularFile => formatter.write_str("source is not a regular file"),
            Self::TooLarge => formatter.write_str("source exceeds the byte limit"),
            Self::Deadline => formatter.write_str("source read exceeded the deadline"),
            Self::UnsupportedContainer => {
                formatter.write_str("source container is unsupported or unsafe")
            }
            Self::ChangedDuringStage => formatter.write_str("source changed during staging"),
            Self::SnapshotChanged => formatter.write_str("private source snapshot changed"),
            Self::Storage(_) => formatter.write_str("private source workspace is unavailable"),
            Self::IdentityFailure => {
                formatter.write_str("source identity could not be represented")
            }
            Self::Io(_) => formatter.write_str("source filesystem operation failed"),
        }
    }
}

impl Error for SourceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Storage(error) => Some(error),
            _ => None,
        }
    }
}
