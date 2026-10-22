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
    ForegroundSessionPort, NeverCancelled, OpenSessionError, SessionStorageError, SourcePathFault,
    SpecialFileKind, StageCancellation, StagedSessionSource,
};
use vsift_domain::{
    DirectLinkOrigin, DownloadLimits, OperationId, SessionId, SourceFetchReason, SourceId,
    SourceLink, SourceOrigin,
};

use crate::{
    FilesystemSessionStore, SessionReadHold, SessionRegistration,
    link_acquisition::{
        LinkFetchError, LinkRoute, LinkSink, LinkSinkError, MAGIC_BYTES, fetch_link,
    },
    source_binding::FileIdentity,
};

/// Maximum source size in the accepted desktop profile.
pub const MAX_SOURCE_BYTES: u64 = 20 * 1024 * 1024 * 1024;
/// Maximum time for an opened source stream to be copied or rehashed: ten
/// minutes, for the whole read and not per block.
///
/// The source is untrusted input, so something has to bound a file that never
/// ends or a device that never answers. The bound is fixed: no option raises
/// it, and a 20 GiB source needs about 34 MiB/s sustained to be copied inside
/// it, which a network share, a removable drive or a folder that downloads on
/// first read may not give (known limit L-140, #325).
pub const MAX_SOURCE_READ_DURATION: Duration = Duration::from_secs(600);
const HEX: &[u8; 16] = b"0123456789abcdef";

/// How long a staging copies before it may say that its copy will not fit
/// [`MAX_SOURCE_READ_DURATION`] (#325): a minute, so a slow start (a disk
/// spinning up, a share's first answer) does not decide it.
const FORECAST_AFTER: Duration = Duration::from_secs(60);
/// The span over which the copy's speed is measured; the forecast uses the
/// fastest such span seen so far, which is the optimistic reading.
const FORECAST_WINDOW: Duration = Duration::from_secs(10);
/// The blocks a span must hold for its speed to count. A span that one long
/// read fills (a cloud-synced file that downloads on its first read and is
/// fast after it) says nothing about the speed of the rest, so it is passed
/// over.
const FORECAST_MIN_BLOCKS: u32 = 16;
/// A copy is stopped early only when, at its fastest measured speed, it would
/// end after half as long again as the limit (fifteen minutes): a forecast
/// near the limit is left to the limit itself. Numerator and denominator.
const FORECAST_MARGIN: (u128, u128) = (3, 2);

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
    /// Where the bytes were fetched from, for a source fetched from a link
    /// (ADR 0028); `None` for a local file's copy and a reopened copy.
    origin: Option<SourceOrigin>,
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
        Self::stage_cancellable(
            store,
            session_id,
            operation_id,
            source_path,
            &NeverCancelled,
        )
    }

    /// [`SourceSnapshot::stage`] that checks `cancellation` before every
    /// 64 KiB block of the copy, so a first Ctrl-C stops a copy of a large
    /// video within one block instead of after the whole file (P10 PR 3).
    /// A cancelled copy removes its partial private file.
    ///
    /// # Errors
    ///
    /// As [`SourceSnapshot::stage`], and [`SourceError::Cancelled`].
    pub fn stage_cancellable(
        store: &FilesystemSessionStore,
        session_id: &SessionId,
        operation_id: &OperationId,
        source_path: &Path,
        cancellation: &dyn StageCancellation,
    ) -> Result<Self, SourceError> {
        let (source, initial) = open_source(source_path)?;
        Self::stage_opened(
            store,
            session_id,
            operation_id,
            (source, &initial),
            None,
            (cancellation, &mut copy_clock(initial.len())),
        )
    }

    /// Stages a source a worker request named inside the operator's input
    /// root (P11, ADR 0021 section 9): the file was opened by
    /// [`crate::InputRoot::open_file`], component by component and following
    /// no link, so nothing outside the root is ever read. The copy, its
    /// identity and its cancellation are those of [`Self::stage_cancellable`].
    ///
    /// # Errors
    ///
    /// As [`Self::stage_cancellable`].
    pub fn stage_contained(
        store: &FilesystemSessionStore,
        session_id: &SessionId,
        operation_id: &OperationId,
        source: crate::ContainedFile,
        cancellation: &dyn StageCancellation,
    ) -> Result<Self, SourceError> {
        let crate::ContainedFile { file, metadata } = source;
        let expected = metadata.len();
        Self::stage_opened(
            store,
            session_id,
            operation_id,
            (file, &metadata),
            None,
            (cancellation, &mut copy_clock(expected)),
        )
    }

    /// [`Self::stage_contained`] under the copy's admission (weight 1) the
    /// caller already holds. A worker takes the admission before it
    /// registers a session, so a busy root makes it wait without leaving a
    /// registration behind for each try (P11 PR 3).
    ///
    /// # Errors
    ///
    /// As [`Self::stage_contained`].
    pub fn stage_contained_admitted(
        store: &FilesystemSessionStore,
        session_id: &SessionId,
        operation_id: &OperationId,
        source: crate::ContainedFile,
        admission: crate::FilesystemAdmissionPermit,
        cancellation: &dyn StageCancellation,
    ) -> Result<Self, SourceError> {
        let crate::ContainedFile { file, metadata } = source;
        let expected = metadata.len();
        Self::stage_opened(
            store,
            session_id,
            operation_id,
            (file, &metadata),
            Some(admission),
            (cancellation, &mut copy_clock(expected)),
        )
    }

    /// Copies an opened source whose metadata is `initial` into the session,
    /// under `admission` when the caller holds it, else under a weight-1
    /// admission taken here.
    ///
    /// The copy stops at the caller's cancellation or when `clock` says so
    /// ([`copy_until`]); either way its partial private file is removed.
    /// Every caller passes [`copy_clock`] for the source's size; the tests
    /// pass a count of blocks so that the time is up without ten minutes going
    /// by.
    fn stage_opened(
        store: &FilesystemSessionStore,
        session_id: &SessionId,
        operation_id: &OperationId,
        (mut source, initial): (File, &cap_std::fs::Metadata),
        admission: Option<crate::FilesystemAdmissionPermit>,
        (cancellation, clock): (&dyn StageCancellation, &mut dyn FnMut(u64) -> CopyTime),
    ) -> Result<Self, SourceError> {
        if initial.len() > MAX_SOURCE_BYTES {
            return Err(SourceError::TooLarge);
        }
        let _admission = match admission {
            Some(held) => held,
            None => store.try_admit(1).map_err(SourceError::Storage)?,
        };
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
        let result = copy_until(&mut source, &mut output, cancellation, clock).and_then(
            |(id, bytes, container)| {
                output.sync_all().map_err(SourceError::Io)?;
                let final_meta = source.metadata().map_err(SourceError::Io)?;
                if initial.len() != final_meta.len()
                    || initial.modified().ok() != final_meta.modified().ok()
                {
                    return Err(SourceError::ChangedDuringStage);
                }
                Ok((id, bytes, container))
            },
        );
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
            origin: None,
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
                origin: None,
                #[cfg(test)]
                full_hashes: std::sync::atomic::AtomicU32::new(1),
            },
            identity,
        ))
    }

    /// Reopens an open session's committed source copy for a read-only
    /// evidence call (ADR 0019 D1).
    ///
    /// When the session records a verified on-disk identity and the copy
    /// still has exactly that identity, the bytes are not hashed again: the
    /// snapshot is built from the committed identity and the file's header,
    /// and the third value is `false`. Otherwise the copy is hashed in full
    /// like [`Self::open_committed`] and the third value is `true`.
    ///
    /// # Errors
    ///
    /// As [`Self::open_committed`].
    pub(crate) fn open_committed_for_evidence(
        store: &FilesystemSessionStore,
        session_id: &SessionId,
        now_unix_seconds: u64,
    ) -> Result<(Self, FileIdentity, bool), SourceError> {
        let committed = store
            .committed_source(session_id, now_unix_seconds)
            .map_err(SourceError::Storage)?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = committed
            .directory
            .open_with(&committed.file_name, &options)
            .map_err(|_| SourceError::SnapshotChanged)?;
        let metadata = file.metadata().map_err(SourceError::Io)?;
        let current = FileIdentity::of(&metadata);
        let unchanged = metadata.is_file()
            && metadata.len() == committed.bytes
            && committed
                .verified_identity
                .as_ref()
                .is_some_and(|recorded| {
                    crate::VerifiedSourceIdentity::of(&current).as_ref() == Some(recorded)
                });
        let (container, identity, hashed) = if unchanged {
            let mut header = [0_u8; 12];
            let read = read_header(&mut file, &mut header)?;
            let container = container_of(header.get(..read).unwrap_or_default())
                .ok_or(SourceError::SnapshotChanged)?;
            (container, current, false)
        } else {
            let (id, container, identity) = read_identified(&mut file, committed.bytes)?;
            if id != committed.source_id {
                return Err(SourceError::SnapshotChanged);
            }
            (container, identity, true)
        };
        Ok((
            Self {
                session_id: session_id.clone(),
                id: committed.source_id,
                bytes: committed.bytes,
                container,
                file_name: committed.file_name,
                directory: committed.directory,
                directory_path: committed.directory_path,
                _hold: committed.hold,
                origin: None,
                #[cfg(test)]
                full_hashes: std::sync::atomic::AtomicU32::new(u32::from(hashed)),
            },
            identity,
            hashed,
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

    /// Where the bytes were fetched from (ADR 0028); `None` for a local
    /// file's copy and for a committed copy reopened later (the session's
    /// status holds the origin).
    #[must_use]
    pub const fn origin(&self) -> Option<&SourceOrigin> {
        self.origin.as_ref()
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
            Err(SourceError::Path(SourcePathFault::NotFound)) => {
                return Ok(true);
            }
            Err(error) => return Err(error),
        };
        match copy_bounded(&mut file, &mut std::io::sink(), &NeverCancelled) {
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

    fn origin(&self) -> Option<&SourceOrigin> {
        self.origin.as_ref()
    }
}

/// The session's source slot while a link's body arrives: the private file
/// `source-<operation>.media`, created new in the session's artifact
/// directory, hashed as it is written. A slot that is dropped before
/// [`SourceSlot::finish`] removes its file, so a failed, refused or
/// cancelled fetch leaves nothing.
struct SourceSlot<'store> {
    store: &'store FilesystemSessionStore,
    session_id: SessionId,
    file_name: String,
    directory: Dir,
    directory_path: PathBuf,
    hold: Option<SessionReadHold>,
    output: Option<File>,
    hash: Sha256,
    header: [u8; MAGIC_BYTES],
    header_count: usize,
    total: u64,
    /// The free space the link test route simulates, if any.
    #[cfg(any(test, feature = "install-test-hooks"))]
    simulated_free_space: Option<u64>,
    _admission: crate::FilesystemAdmissionPermit,
}

impl<'store> SourceSlot<'store> {
    /// Creates the slot under a weight-1 admission, as a copy takes one.
    fn create(
        store: &'store FilesystemSessionStore,
        session_id: &SessionId,
        operation_id: &OperationId,
    ) -> Result<Self, SourceError> {
        let admission = store.try_admit(1).map_err(SourceError::Storage)?;
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
        let output = directory
            .open_with(&file_name, &options)
            .map_err(SourceError::Io)?;
        Ok(Self {
            store,
            session_id: session_id.clone(),
            file_name,
            directory,
            directory_path,
            hold: Some(hold),
            output: Some(output),
            hash: Sha256::new(),
            header: [0; MAGIC_BYTES],
            header_count: 0,
            total: 0,
            #[cfg(any(test, feature = "install-test-hooks"))]
            simulated_free_space: None,
            _admission: admission,
        })
    }

    /// Syncs the file and makes it the session's snapshot, with `origin`.
    fn finish(mut self, origin: SourceOrigin) -> Result<SourceSnapshot, SourceError> {
        let output = self.output.take().ok_or(SourceError::IdentityFailure)?;
        output.sync_all().map_err(SourceError::Io)?;
        drop(output);
        let container = container_of(self.header.get(..self.header_count).unwrap_or_default())
            .ok_or(SourceError::UnsupportedContainer)?;
        let hash = std::mem::take(&mut self.hash);
        let mut digest = String::with_capacity(64);
        for byte in hash.finalize() {
            digest.push(char::from(HEX[usize::from(byte >> 4)]));
            digest.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        let id = SourceId::from_sha256(&digest).map_err(|_| SourceError::IdentityFailure)?;
        let hold = self.hold.take().ok_or(SourceError::IdentityFailure)?;
        let directory = self.directory.try_clone().map_err(SourceError::Io)?;
        Ok(SourceSnapshot {
            session_id: self.session_id.clone(),
            id,
            bytes: self.total,
            container,
            file_name: self.file_name.clone(),
            directory,
            directory_path: self.directory_path.clone(),
            _hold: hold,
            origin: Some(origin),
            #[cfg(test)]
            full_hashes: std::sync::atomic::AtomicU32::new(0),
        })
    }
}

impl Drop for SourceSlot<'_> {
    fn drop(&mut self) {
        if let Some(output) = self.output.take() {
            drop(output);
            let _ = self.directory.remove_file(&self.file_name);
        }
    }
}

impl LinkSink for SourceSlot<'_> {
    fn append(&mut self, chunk: &[u8]) -> Result<(), LinkSinkError> {
        let output = self
            .output
            .as_mut()
            .ok_or_else(|| LinkSinkError::Write(std::io::Error::other("the slot is closed")))?;
        output.write_all(chunk).map_err(|error| {
            if error.kind() == std::io::ErrorKind::StorageFull {
                LinkSinkError::NoRoom
            } else {
                LinkSinkError::Write(error)
            }
        })?;
        self.hash.update(chunk);
        let take = (MAGIC_BYTES - self.header_count).min(chunk.len());
        self.header[self.header_count..self.header_count + take].copy_from_slice(&chunk[..take]);
        self.header_count += take;
        self.total = self
            .total
            .saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
        Ok(())
    }

    fn header(&self) -> Option<&[u8]> {
        (self.header_count == MAGIC_BYTES).then_some(&self.header[..])
    }

    fn ensure_room(&self, bytes: u64) -> Result<(), LinkSinkError> {
        // Best effort, as for a copy: only a read that shows the space is
        // short refuses; anything else is found by a failed write.
        #[cfg(any(test, feature = "install-test-hooks"))]
        if let Some(available) = self.simulated_free_space {
            let left = available.saturating_sub(self.total);
            return match crate::filesystem_session_store::decide_copy_room(Some(left), bytes) {
                Err(SessionStorageError::CapacityExhausted) => Err(LinkSinkError::NoRoom),
                _ => Ok(()),
            };
        }
        match self.store.ensure_room_for_copy(bytes) {
            Err(SessionStorageError::CapacityExhausted) => Err(LinkSinkError::NoRoom),
            _ => Ok(()),
        }
    }
}

impl SourceSnapshot {
    /// Fetches one video from a direct HTTPS link into the session's source
    /// slot (ADR 0028, #407) and makes it the session's snapshot, recording
    /// its origin. A fetch that fails, is refused or is cancelled removes its
    /// partial file.
    ///
    /// # Errors
    ///
    /// [`SourceError::Fetch`] with the typed reason, [`SourceError::Cancelled`],
    /// [`SourceError::NoRoom`], or the slot's storage or I/O failure.
    pub async fn stage_link(
        store: &FilesystemSessionStore,
        session_id: &SessionId,
        operation_id: &OperationId,
        (link, limits): (&SourceLink, DownloadLimits),
        cancellation: &dyn StageCancellation,
    ) -> Result<Self, SourceError> {
        Self::stage_link_through(
            store,
            session_id,
            operation_id,
            (link, limits),
            &LinkRoute::production(),
            cancellation,
        )
        .await
    }

    /// [`Self::stage_link`] through a local HTTPS test server
    /// (`install-test-hooks`, development builds only).
    ///
    /// # Errors
    ///
    /// As [`Self::stage_link`].
    #[cfg(any(test, feature = "install-test-hooks"))]
    pub async fn stage_link_for_tests(
        store: &FilesystemSessionStore,
        session_id: &SessionId,
        operation_id: &OperationId,
        (link, limits): (&SourceLink, DownloadLimits),
        route: crate::LinkTestRoute,
        cancellation: &dyn StageCancellation,
    ) -> Result<Self, SourceError> {
        Self::stage_link_through(
            store,
            session_id,
            operation_id,
            (link, limits),
            &LinkRoute::test(route),
            cancellation,
        )
        .await
    }

    async fn stage_link_through(
        store: &FilesystemSessionStore,
        session_id: &SessionId,
        operation_id: &OperationId,
        (link, limits): (&SourceLink, DownloadLimits),
        route: &LinkRoute,
        cancellation: &dyn StageCancellation,
    ) -> Result<Self, SourceError> {
        let mut slot = SourceSlot::create(store, session_id, operation_id)?;
        #[cfg(any(test, feature = "install-test-hooks"))]
        {
            slot.simulated_free_space = route.free_space();
        }
        let fetched = fetch_link(link, limits, route, &mut slot, cancellation)
            .await
            .map_err(|error| match error {
                LinkFetchError::Refused(reason) => SourceError::Fetch(reason),
                LinkFetchError::Cancelled => SourceError::Cancelled,
                LinkFetchError::Sink(LinkSinkError::NoRoom) => SourceError::NoRoom,
                LinkFetchError::Sink(LinkSinkError::Write(error)) => SourceError::Io(error),
            })?;
        let retrieved_at = vsift_application::Clock::now_unix_seconds(&crate::SystemClock)
            .map_err(|_| SourceError::IdentityFailure)?;
        let origin = DirectLinkOrigin::new(
            fetched.link_sha256,
            (fetched.requested_host, fetched.final_host),
            fetched.redirects,
            fetched.address_check,
            retrieved_at,
            fetched.bytes,
        )
        .map_err(|_| SourceError::Fetch(SourceFetchReason::Size))?;
        slot.finish(SourceOrigin::DirectHttps(origin))
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

    fn lifetime_policy(&self) -> vsift_domain::SessionLifetimePolicy {
        Self::lifetime_policy(self)
    }

    fn stage_source(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        source: &Path,
        cancellation: &dyn StageCancellation,
    ) -> Result<Self::Snapshot, OpenSessionError> {
        SourceSnapshot::stage_cancellable(self, session_id, operation_id, source, cancellation)
            .map_err(open_session_error)
    }

    async fn stage_link(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        link: &SourceLink,
        limits: DownloadLimits,
        cancellation: &dyn StageCancellation,
    ) -> Result<Self::Snapshot, OpenSessionError> {
        SourceSnapshot::stage_link(self, session_id, operation_id, (link, limits), cancellation)
            .await
            .map_err(open_session_error)
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

/// How a failed staging is reported to the open-session use case.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Result::map_err requires ownership of the source error"
)]
pub(crate) fn open_session_error(error: SourceError) -> OpenSessionError {
    match error {
        SourceError::Storage(storage) => OpenSessionError::Storage(storage),
        SourceError::Cancelled => OpenSessionError::Cancelled,
        // A write that ran out of room is the root's filesystem being too
        // small, not a source that could not be read (#266).
        SourceError::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded
            ) =>
        {
            OpenSessionError::SourceNoRoom
        }
        SourceError::Io(_) => OpenSessionError::SourceIo,
        SourceError::NoRoom => OpenSessionError::SourceNoRoom,
        SourceError::Fetch(reason) => OpenSessionError::SourceFetch(reason),
        SourceError::Path(fault) => OpenSessionError::SourcePath(fault),
        SourceError::SymbolicLink => OpenSessionError::SourceIsLink,
        SourceError::SpecialFile(kind) => OpenSessionError::SourceSpecialFile(kind),
        // A copy that ran out of time says nothing about the source: it was
        // slow to read, or the root slow to write (#325). The code a host
        // gives it stays the one it always was (L-127).
        SourceError::Deadline => OpenSessionError::SourceCopyTooSlow,
        SourceError::CopyWillRunOut => OpenSessionError::SourceCopyWouldBeTooSlow,
        SourceError::InvalidPath
        | SourceError::NotRegularFile
        | SourceError::TooLarge
        | SourceError::UnsupportedContainer
        | SourceError::ChangedDuringStage
        | SourceError::SnapshotChanged
        | SourceError::IdentityFailure => OpenSessionError::InvalidSource,
    }
}

/// Opens one directly named, local regular file without following a final link.
///
/// Shared by source staging and supplied-transcript import so every
/// caller-selected input passes the same path policy.
///
/// A failure to reach the file is classified here, where the path is still in
/// hand, and not by the callers: an `Io` failure later in a staging (the
/// private copy, a read) can also say "not found" about a file that is `VSift`'s
/// own, and that must never be told to a person as "no file is at the path you
/// gave" (#369).
pub(crate) fn open_source(path: &Path) -> Result<(File, cap_std::fs::Metadata), SourceError> {
    if !path.is_absolute() || !is_local_path(path) {
        return Err(SourceError::InvalidPath);
    }
    let name = path.file_name().ok_or(SourceError::InvalidPath)?;
    if invalid_source_name(name) {
        return Err(SourceError::InvalidPath);
    }
    let parent = path.parent().ok_or(SourceError::InvalidPath)?;
    let canonical_parent = std::fs::canonicalize(parent).map_err(source_path_failure)?;
    if !is_local_path(&canonical_parent) {
        return Err(SourceError::InvalidPath);
    }
    let directory = Dir::open_ambient_dir(canonical_parent, cap_std::ambient_authority())
        .map_err(source_path_failure)?;
    // A link is classified from the directory entry before anything is opened,
    // as the worker's input root does: opening it without following fails with
    // a platform-specific error that would otherwise reach the caller as a
    // storage failure (#265). The open below still follows nothing, so a name
    // swapped for a link in between is not followed either.
    //
    // A named pipe, a device or a socket is refused from the same entry, before
    // anything opens it (#264): opening a pipe waits for a writer and opening a
    // device can act on it. Reading an entry does neither. The handle's own
    // type is still checked after the open below, which a swap in between
    // reaches, and that open does not wait either.
    if let Ok(entry) = directory.symlink_metadata(Path::new(name)) {
        if entry.file_type().is_symlink() {
            return Err(SourceError::SymbolicLink);
        }
        if let Some(refusal) = refuse_special_file(&directory, name, entry.file_type()) {
            return Err(refusal);
        }
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    never_wait_for_a_writer(&mut options)?;
    let file = directory
        .open_with(Path::new(name), &options)
        .map_err(|error| {
            // Looked at again, as the entry is now: the swap case (whatever
            // made the open fail, a link is a link) and a folder, which
            // Windows refuses to open as a file and reports as a denied
            // access, so the kind of the failure alone would call it a
            // permission problem.
            match directory.symlink_metadata(Path::new(name)) {
                Ok(entry) if entry.file_type().is_symlink() => SourceError::SymbolicLink,
                Ok(entry) if entry.is_dir() => SourceError::Path(SourcePathFault::IsFolder),
                _ => source_path_failure(error),
            }
        })?;
    let metadata = file.metadata().map_err(SourceError::Io)?;
    if !metadata.is_file() {
        return Err(SourceError::NotRegularFile);
    }
    Ok((file, metadata))
}

/// The refusal of a directory entry that is neither a regular file, a folder
/// nor a link, read from its type without opening it (#264), or `None` for
/// any other entry.
///
/// An entry the account may not read keeps the answer its refused open gave
/// before (`STORAGE_IO`, access denied, #369): a published failure code does
/// not change within v1 (known limit L-127), and asking for read access opens
/// nothing either.
#[cfg(unix)]
fn refuse_special_file(
    directory: &Dir,
    name: &std::ffi::OsStr,
    file_type: cap_std::fs::FileType,
) -> Option<SourceError> {
    use cap_std::fs::FileTypeExt as _;
    let kind = if file_type.is_fifo() {
        SpecialFileKind::NamedPipe
    } else if file_type.is_char_device() || file_type.is_block_device() {
        SpecialFileKind::Device
    } else if file_type.is_socket() {
        SpecialFileKind::Socket
    } else {
        return None;
    };
    let readable = rustix::fs::accessat(
        directory,
        name,
        rustix::fs::Access::READ_OK,
        rustix::fs::AtFlags::EACCESS,
    );
    Some(match readable {
        Err(rustix::io::Errno::ACCESS) => SourceError::Path(SourcePathFault::AccessDenied),
        _ => SourceError::SpecialFile(kind),
    })
}

/// Windows names none of these kinds in a folder: a named pipe or a device is
/// a `\\.\` path, which [`is_local_path`] refuses before any entry is read.
#[cfg(not(unix))]
const fn refuse_special_file(
    _directory: &Dir,
    _name: &std::ffi::OsStr,
    _file_type: cap_std::fs::FileType,
) -> Option<SourceError> {
    None
}

/// What the operating system said about a path it could not reach, when it
/// said it plainly; `None` for every other kind, which stays an I/O failure.
///
/// "Not a directory" is "not found" for a person: a part of the path that
/// should be a folder is a file, so no file is at that path.
const fn source_path_fault(kind: std::io::ErrorKind) -> Option<SourcePathFault> {
    match kind {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
            Some(SourcePathFault::NotFound)
        }
        std::io::ErrorKind::PermissionDenied => Some(SourcePathFault::AccessDenied),
        _ => None,
    }
}

/// Reports a failure to reach the selected path as the fault the system named,
/// or as an untyped I/O failure when it named none (see [`source_path_fault`]).
fn source_path_failure(error: std::io::Error) -> SourceError {
    match source_path_fault(error.kind()) {
        Some(fault) => SourceError::Path(fault),
        None => SourceError::Io(error),
    }
}

/// Opens without waiting, so that naming a named pipe is a refusal and not a hang.
///
/// The file type can only be checked on the opened handle (checking the path
/// first would leave a window in which the file is swapped for a pipe), and
/// opening a named pipe for reading blocks until something opens it for
/// writing, which nothing may ever do (#264). With `O_NONBLOCK` the open of a
/// pipe returns at once, the handle is then found not to be a regular file
/// and is dropped; a regular file ignores the flag, so every later read of an
/// accepted source is unchanged. Windows has no such open.
///
/// # Errors
///
/// An I/O failure, never a silent no-op: if the platform's flag cannot be
/// expressed the open would wait for a writer, which is the hang this exists to
/// prevent, so it is refused instead.
#[cfg(unix)]
fn never_wait_for_a_writer(options: &mut OpenOptions) -> Result<(), SourceError> {
    use cap_std::fs::OpenOptionsExt as _;
    let non_blocking = i32::try_from(rustix::fs::OFlags::NONBLOCK.bits()).map_err(|_| {
        SourceError::Io(std::io::Error::other(
            "the non-blocking open flag does not fit the platform's flag type",
        ))
    })?;
    options.custom_flags(non_blocking);
    Ok(())
}

#[cfg(not(unix))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "the Unix implementation can fail to express the flag"
)]
fn never_wait_for_a_writer(_options: &mut OpenOptions) -> Result<(), SourceError> {
    Ok(())
}

/// Whether a path is on a local drive (Windows) or not a network path
/// (`//host`, elsewhere).
#[cfg(windows)]
pub(crate) fn is_local_path(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    matches!(path.components().next(), Some(Component::Prefix(prefix))
        if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
}

/// Whether a path is on a local drive (Windows) or not a network path
/// (`//host`, elsewhere).
#[cfg(not(windows))]
pub(crate) fn is_local_path(path: &Path) -> bool {
    !path.as_os_str().as_encoded_bytes().starts_with(b"//")
}

/// Whether a file name is a Windows device name (`CON`, `nul.txt`, `COM1`,
/// `LPT¹`, `CONIN$`, ...), whatever its case and extension. Checked on
/// every platform for request paths, so one request means the same file
/// everywhere.
pub(crate) fn is_reserved_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix).is_some_and(|number| {
            matches!(
                number,
                "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    };
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || numbered("COM")
        || numbered("LPT")
}

#[cfg(windows)]
fn invalid_source_name(name: &std::ffi::OsStr) -> bool {
    let text = name.to_string_lossy();
    text.contains(':') || text.ends_with([' ', '.']) || is_reserved_device_name(&text)
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
    let (id, bytes, container) = copy_bounded(file, &mut std::io::sink(), &NeverCancelled)?;
    let after = file.metadata().map_err(SourceError::Io)?;
    let identity = FileIdentity::of(&after);
    if bytes != expected_bytes || FileIdentity::of(&before) != identity {
        return Err(SourceError::SnapshotChanged);
    }
    Ok((id, container, identity))
}

/// The admitted container family a file's first bytes announce.
fn container_of(header: &[u8]) -> Option<SourceContainer> {
    if header.get(4..8) == Some(b"ftyp".as_slice()) {
        Some(SourceContainer::IsoMedia)
    } else if header.get(..4) == Some([0x1a, 0x45, 0xdf, 0xa3].as_slice()) {
        Some(SourceContainer::Matroska)
    } else {
        None
    }
}

/// Reads up to `header.len()` leading bytes of an opened copy.
fn read_header(file: &mut File, header: &mut [u8]) -> Result<usize, SourceError> {
    let mut read = 0;
    while let Some(rest) = header.get_mut(read..)
        && !rest.is_empty()
    {
        let count = file.read(rest).map_err(SourceError::Io)?;
        if count == 0 {
            break;
        }
        read += count;
    }
    Ok(read)
}

/// What a copy's clock answers before the next block (#325).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CopyTime {
    /// Go on.
    Left,
    /// [`MAX_SOURCE_READ_DURATION`] has passed.
    Spent,
    /// At the fastest speed measured so far the copy would end well after
    /// [`MAX_SOURCE_READ_DURATION`] ([`CopyForecast`]).
    WillRunOut,
}

/// Whether a copy of `expected` bytes will fit [`MAX_SOURCE_READ_DURATION`],
/// told early from the speed it is copied at (#325, the first step of its
/// design; ADR 0012 note of 2026-10-17).
///
/// Pure arithmetic on the time and the bytes the copy reports before each
/// block, so the decision is tested with any speeds and no clock. It is
/// cautious on purpose, because stopping a copy that would have finished is
/// worse than the minutes it saves: it says nothing in the first
/// [`FORECAST_AFTER`], measures the speed over spans of at least
/// [`FORECAST_WINDOW`] that hold more than [`FORECAST_MIN_BLOCKS`] blocks,
/// keeps the fastest, and says [`CopyTime::WillRunOut`] only when even that
/// speed ends the copy after [`FORECAST_MARGIN`] times the limit. Anything
/// closer runs until the limit, as before.
#[derive(Debug)]
struct CopyForecast {
    expected: u64,
    window_started: Duration,
    window_bytes: u64,
    window_blocks: u32,
    /// The fastest span: its bytes and its length.
    fastest: Option<(u64, Duration)>,
}

impl CopyForecast {
    const fn new(expected: u64) -> Self {
        Self {
            expected,
            window_started: Duration::ZERO,
            window_bytes: 0,
            window_blocks: 0,
            fastest: None,
        }
    }

    /// The answer before the next block, `elapsed` after the copy started,
    /// with `copied` bytes done.
    fn ask(&mut self, elapsed: Duration, copied: u64) -> CopyTime {
        if elapsed > MAX_SOURCE_READ_DURATION {
            return CopyTime::Spent;
        }
        let span = elapsed.saturating_sub(self.window_started);
        if span >= FORECAST_WINDOW {
            let bytes = copied.saturating_sub(self.window_bytes);
            if self.window_blocks > FORECAST_MIN_BLOCKS && faster(bytes, span, self.fastest) {
                self.fastest = Some((bytes, span));
            }
            self.window_started = elapsed;
            self.window_bytes = copied;
            self.window_blocks = 0;
        }
        self.window_blocks = self.window_blocks.saturating_add(1);
        if elapsed < FORECAST_AFTER {
            return CopyTime::Left;
        }
        let Some((bytes, span)) = self.fastest else {
            return CopyTime::Left;
        };
        if bytes == 0 {
            return CopyTime::Left;
        }
        // The end at the fastest speed, elapsed + remaining * span / bytes in
        // nanoseconds, against the limit with its margin.
        let remaining = u128::from(self.expected.saturating_sub(copied));
        let projected = elapsed
            .as_nanos()
            .saturating_add(remaining.saturating_mul(span.as_nanos()) / u128::from(bytes));
        let (numerator, denominator) = FORECAST_MARGIN;
        let allowed = MAX_SOURCE_READ_DURATION.as_nanos() * numerator / denominator;
        if projected > allowed {
            CopyTime::WillRunOut
        } else {
            CopyTime::Left
        }
    }
}

/// Whether `bytes` in `span` is faster than the fastest span so far.
fn faster(bytes: u64, span: Duration, fastest: Option<(u64, Duration)>) -> bool {
    fastest.is_none_or(|(best_bytes, best_span)| {
        // bytes / span > best_bytes / best_span, without a division.
        u128::from(bytes).saturating_mul(best_span.as_nanos())
            > u128::from(best_bytes).saturating_mul(span.as_nanos())
    })
}

/// The wall-clock answer to [`copy_until`]'s question for a staging of
/// `expected` bytes: [`MAX_SOURCE_READ_DURATION`] from now, and the early
/// answer of [`CopyForecast`].
fn copy_clock(expected: u64) -> impl FnMut(u64) -> CopyTime {
    let started = Instant::now();
    let mut forecast = CopyForecast::new(expected);
    move |copied| forecast.ask(started.elapsed(), copied)
}

/// The wall-clock answer for a read that is not a staging (the re-hash of the
/// original): [`MAX_SOURCE_READ_DURATION`] from now, and nothing early.
fn read_deadline() -> impl FnMut(u64) -> CopyTime {
    let started = Instant::now();
    move |_| {
        if started.elapsed() > MAX_SOURCE_READ_DURATION {
            CopyTime::Spent
        } else {
            CopyTime::Left
        }
    }
}

/// Copies and hashes a whole source within [`MAX_SOURCE_READ_DURATION`].
fn copy_bounded(
    source: &mut impl Read,
    destination: &mut impl Write,
    cancellation: &dyn StageCancellation,
) -> Result<(SourceId, u64, SourceContainer), SourceError> {
    copy_until(source, destination, cancellation, &mut read_deadline())
}

/// Copies `source` to `destination` in 64 KiB blocks, hashing what it copies,
/// until the source ends, `clock` says the time is spent
/// ([`SourceError::Deadline`]) or will run out
/// ([`SourceError::CopyWillRunOut`]), or the caller cancels
/// ([`SourceError::Cancelled`]). Each is asked before every block, so none
/// waits for more than one read.
///
/// The limit is on the whole copy; a copy that will clearly not fit it is
/// stopped early ([`CopyForecast`]). #325 holds the rest of the design (a
/// budget a caller can raise, a copy that can be continued, a stall told
/// apart). The clock is a parameter given the bytes copied so far, and not a
/// clock read here, so that a test can make the time run out after a chosen
/// block instead of waiting ten minutes.
fn copy_until(
    source: &mut impl Read,
    destination: &mut impl Write,
    cancellation: &dyn StageCancellation,
    clock: &mut dyn FnMut(u64) -> CopyTime,
) -> Result<(SourceId, u64, SourceContainer), SourceError> {
    let mut hash = Sha256::new();
    let mut header = [0_u8; 12];
    let mut header_count = 0_usize;
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        match clock(total) {
            CopyTime::Left => {}
            CopyTime::Spent => return Err(SourceError::Deadline),
            CopyTime::WillRunOut => return Err(SourceError::CopyWillRunOut),
        }
        if cancellation.is_cancelled() {
            return Err(SourceError::Cancelled);
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
    let container = container_of(header.get(..header_count).unwrap_or_default())
        .ok_or(SourceError::UnsupportedContainer)?;
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
    /// The selected path's own final component is a link. The link is not
    /// followed (the folders on the way are resolved as the operating system
    /// resolves them, so a link there is), so the source is refused as what it
    /// is and not as a failed disk (#265). It is reported as the published
    /// `STORAGE_IO` code with a remediation (known limit L-127).
    SymbolicLink,
    /// The selected path's own directory entry is a named pipe, a device or a
    /// socket, refused before anything opened it (#264). A handle that turns
    /// out not to be a regular file after the open (a swap in between) is
    /// [`Self::NotRegularFile`].
    SpecialFile(SpecialFileKind),
    /// Selected source exceeds the profile maximum.
    TooLarge,
    /// Reading the source took longer than [`MAX_SOURCE_READ_DURATION`] and
    /// was stopped. It says how long the read took, not what the source is: a
    /// staging reports it as a slow copy
    /// ([`OpenSessionError::SourceCopyTooSlow`], #325).
    Deadline,
    /// The copy was stopped early because, at the fastest speed measured, it
    /// would end well after [`MAX_SOURCE_READ_DURATION`] (#325). Like
    /// [`Self::Deadline`] it says nothing about the source; a staging reports
    /// it as [`OpenSessionError::SourceCopyWouldBeTooSlow`].
    CopyWillRunOut,
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
    /// The selected path could not be opened, and the operating system named
    /// why: no file is there, it is a folder, or reading it is not allowed
    /// (#369). Only the open of the selected path makes this; every later
    /// filesystem failure of a staging is [`Self::Io`]. It is reported as the
    /// published `STORAGE_IO` code with a remediation (known limit L-127).
    Path(SourcePathFault),
    /// The caller cancelled the copy; its partial private file was removed.
    Cancelled,
    /// A link could not be fetched (ADR 0028), for the typed reason; the
    /// partial private file was removed.
    Fetch(SourceFetchReason),
    /// The session root's filesystem has no room for the fetched body.
    NoRoom,
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath => formatter.write_str("source path is invalid"),
            Self::NotRegularFile => formatter.write_str("source is not a regular file"),
            Self::SymbolicLink => formatter.write_str("source is a link, which is not followed"),
            Self::SpecialFile(kind) => kind.fmt(formatter),
            Self::TooLarge => formatter.write_str("source exceeds the byte limit"),
            Self::Deadline => formatter.write_str("source read exceeded the deadline"),
            Self::CopyWillRunOut => {
                formatter.write_str("source copy would not finish within the deadline")
            }
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
            Self::Path(fault) => fault.fmt(formatter),
            Self::Cancelled => formatter.write_str("source staging was cancelled"),
            Self::Fetch(reason) => reason.fmt(formatter),
            Self::NoRoom => formatter.write_str("the session root has no room for the source"),
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

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        ffi::OsStr,
        fs,
        io::{Cursor, Error as IoError, ErrorKind},
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use vsift_application::{
        InitializeSessionStorage, InitializeSessionStorageRequest, NeverCancelled,
        OpenSessionError, SourcePathFault,
    };
    use vsift_domain::{DurabilityRequirement, OperationId, SessionId};

    use super::{
        CopyForecast, CopyTime, MAX_SOURCE_BYTES, MAX_SOURCE_READ_DURATION, SourceContainer,
        SourceError, SourceSnapshot, copy_clock, copy_until, open_session_error, open_source,
        read_deadline, source_path_failure, source_path_fault,
    };
    use crate::FilesystemSessionStore;

    type TestResult = Result<(), Box<dyn Error>>;

    /// The size of one block of the copy.
    const BLOCK: usize = 64 * 1024;
    /// [`BLOCK`] as the copy counts bytes.
    const BLOCK_BYTES: u64 = 64 * 1024;
    const MIB: u64 = 1024 * 1024;
    const OWNED_PREFIX: &str = "vsift-source-snapshot-unit-";
    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    /// A scratch folder of this test file, removed on drop.
    struct OwnedRoot(PathBuf);

    impl OwnedRoot {
        fn new() -> Result<Self, Box<dyn Error>> {
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
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

    /// A stand-in video of `blocks` blocks: an ISO media header, then filler.
    fn stand_in(blocks: usize) -> Vec<u8> {
        let mut bytes = vec![0x5a_u8; blocks * BLOCK];
        for (target, byte) in bytes.iter_mut().zip(b"\0\0\0\x18ftypisom") {
            *target = *byte;
        }
        bytes
    }

    /// The time is up from the `ask`-th time the copy asks (it asks before
    /// every block), so the copy stops after `ask - 1` blocks without a clock.
    fn up_from(ask: usize) -> impl FnMut(u64) -> CopyTime {
        stop_from(ask, CopyTime::Spent)
    }

    /// The clock answers `answer` from the `ask`-th time the copy asks.
    fn stop_from(ask: usize, answer: CopyTime) -> impl FnMut(u64) -> CopyTime {
        let mut asked = 0_usize;
        move |_| {
            asked += 1;
            if asked >= ask { answer } else { CopyTime::Left }
        }
    }

    /// Asks `forecast` before each block of a copy of `blocks` blocks, the
    /// `n`-th block (from 0) being asked at `at(n)`, until it answers anything
    /// but [`CopyTime::Left`]; returns that answer and when it came, or
    /// `None` when the copy ends first.
    fn run(
        forecast: &mut CopyForecast,
        blocks: u64,
        at: impl Fn(u64) -> Duration,
    ) -> Option<(CopyTime, Duration)> {
        for block in 0..blocks {
            let elapsed = at(block);
            let answer = forecast.ask(elapsed, block * BLOCK_BYTES);
            if answer != CopyTime::Left {
                return Some((answer, elapsed));
            }
        }
        None
    }

    /// A steady copy of `rate` bytes per second: block `n` is asked when `n`
    /// blocks are done.
    fn steady(rate: u64) -> impl Fn(u64) -> Duration {
        move |block| {
            let nanos =
                u128::from(block) * u128::from(BLOCK_BYTES) * 1_000_000_000 / u128::from(rate);
            Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
        }
    }

    /// #325: the limit on a source copy is asked before every block, so a copy
    /// that runs out of time stops there, as a deadline and as nothing else,
    /// whatever the bytes are. The clock is a parameter of the copy, so the
    /// time runs out after two blocks here and not after ten minutes.
    #[test]
    fn a_copy_whose_time_runs_out_stops_before_its_next_block() -> TestResult {
        let source = stand_in(4);

        // With time to spare the whole source is copied and identified.
        let mut copied = Vec::new();
        let (_, bytes, container) = copy_until(
            &mut Cursor::new(&source),
            &mut copied,
            &NeverCancelled,
            &mut |_| CopyTime::Left,
        )?;
        assert_eq!(bytes, u64::try_from(source.len())?);
        assert_eq!(container, SourceContainer::IsoMedia);
        assert_eq!(copied, source);

        // Out of time at the third ask: two blocks are copied, the third is
        // never read.
        let mut reader = Cursor::new(&source);
        let mut copied = Vec::new();
        let stopped = copy_until(&mut reader, &mut copied, &NeverCancelled, &mut up_from(3));
        assert!(matches!(stopped, Err(SourceError::Deadline)), "{stopped:?}");
        assert_eq!(copied.len(), 2 * BLOCK);
        assert_eq!(reader.position(), u64::try_from(2 * BLOCK)?);

        // Out of time before the first block: nothing is read at all.
        let mut reader = Cursor::new(&source);
        let mut copied = Vec::new();
        let stopped = copy_until(&mut reader, &mut copied, &NeverCancelled, &mut up_from(1));
        assert!(matches!(stopped, Err(SourceError::Deadline)), "{stopped:?}");
        assert_eq!(copied, [] as [u8; 0]);
        assert_eq!(reader.position(), 0);

        // The bytes are not judged before the copy ends: a file that is no
        // video and ran out of time is a slow copy, not an unsupported one.
        let not_a_video = vec![0x41_u8; 4 * BLOCK];
        let stopped = copy_until(
            &mut Cursor::new(&not_a_video),
            &mut Vec::new(),
            &NeverCancelled,
            &mut up_from(3),
        );
        assert!(matches!(stopped, Err(SourceError::Deadline)), "{stopped:?}");
        let judged = copy_until(
            &mut Cursor::new(&not_a_video),
            &mut Vec::new(),
            &NeverCancelled,
            &mut |_| CopyTime::Left,
        );
        assert!(
            matches!(judged, Err(SourceError::UnsupportedContainer)),
            "{judged:?}"
        );
        Ok(())
    }

    /// The wall-clock answer: not up when the copy starts, with the published
    /// ten minutes behind it (20 GiB in ten minutes is about 34 MiB/s, the
    /// figure known limit L-140 gives).
    #[test]
    fn the_real_limit_is_ten_minutes_from_the_start_of_the_copy() {
        assert_eq!(MAX_SOURCE_READ_DURATION, Duration::from_mins(10));
        let mut time_is_up = read_deadline();
        assert_eq!(time_is_up(0), CopyTime::Left);
        assert_eq!(time_is_up(u64::MAX), CopyTime::Left);
        // A staging's clock is not early either at its start.
        let mut staging = copy_clock(MAX_SOURCE_BYTES);
        assert_eq!(staging(0), CopyTime::Left);
        let needed = MAX_SOURCE_BYTES / MAX_SOURCE_READ_DURATION.as_secs() / (1024 * 1024);
        assert_eq!(needed, 34);
    }

    /// #325, the first step: a copy that will clearly not fit the ten minutes is
    /// stopped soon after its first minute and not at ten. 20 GiB at 2 MiB/s
    /// would take 170 minutes.
    #[test]
    fn a_copy_far_too_slow_for_the_limit_is_stopped_after_its_first_minute() -> TestResult {
        let mut forecast = CopyForecast::new(MAX_SOURCE_BYTES);
        let stopped = run(
            &mut forecast,
            MAX_SOURCE_BYTES / BLOCK_BYTES,
            steady(2 * MIB),
        );
        let Some((answer, at)) = stopped else {
            return Err("a copy of 170 minutes was never stopped".into());
        };
        assert_eq!(answer, CopyTime::WillRunOut);
        assert!(at >= Duration::from_mins(1), "{at:?}");
        assert!(at < Duration::from_secs(61), "{at:?}");
        Ok(())
    }

    /// The forecast is cautious: a copy that ends at its fastest speed
    /// within half as long again as the limit is left to the limit (here 12
    /// minutes: stopped at ten, as before), and a copy that fits is never
    /// stopped.
    #[test]
    fn a_copy_near_the_limit_or_inside_it_is_not_stopped_early() {
        // 12 minutes: 20 GiB at 20480 / 720 MiB/s.
        let rate = MAX_SOURCE_BYTES / 720;
        let mut forecast = CopyForecast::new(MAX_SOURCE_BYTES);
        let stopped = run(&mut forecast, MAX_SOURCE_BYTES / BLOCK_BYTES, steady(rate));
        assert!(
            matches!(stopped, Some((CopyTime::Spent, at)) if at > MAX_SOURCE_READ_DURATION),
            "{stopped:?}"
        );

        // Nine minutes: never stopped.
        let rate = MAX_SOURCE_BYTES / 540;
        let mut forecast = CopyForecast::new(MAX_SOURCE_BYTES);
        let stopped = run(&mut forecast, MAX_SOURCE_BYTES / BLOCK_BYTES, steady(rate));
        assert_eq!(stopped, None);
    }

    /// Just over fifteen minutes at a steady speed is stopped early, just
    /// under is not: the margin is the limit and half of it again.
    #[test]
    fn the_margin_is_half_as_long_again_as_the_limit() {
        let blocks = MAX_SOURCE_BYTES / BLOCK_BYTES;
        let over = MAX_SOURCE_BYTES / 920;
        let mut forecast = CopyForecast::new(MAX_SOURCE_BYTES);
        assert!(matches!(
            run(&mut forecast, blocks, steady(over)),
            Some((CopyTime::WillRunOut, _))
        ));
        let under = MAX_SOURCE_BYTES / 880;
        let mut forecast = CopyForecast::new(MAX_SOURCE_BYTES);
        assert!(matches!(
            run(&mut forecast, blocks, steady(under)),
            Some((CopyTime::Spent, _))
        ));
    }

    /// A cloud-synced file that downloads on its first read: the first block
    /// takes five minutes, the rest are fast. The span that one read filled
    /// says nothing about the rest and is passed over, so the copy is not
    /// stopped early and ends inside the limit.
    #[test]
    fn a_first_read_that_waits_long_and_a_fast_rest_is_not_stopped_early() {
        let wait = Duration::from_mins(5);
        let rest = steady(200 * MIB);
        let blocks = 3 * 1024 * MIB / BLOCK_BYTES;
        let mut forecast = CopyForecast::new(blocks * BLOCK_BYTES);
        let stopped = run(&mut forecast, blocks, |block| {
            if block == 0 {
                Duration::ZERO
            } else {
                wait + rest(block)
            }
        });
        assert_eq!(stopped, None);
    }

    /// A copy whose every read waits long (a device that hardly answers) has
    /// no span to measure: nothing is forecast and the limit stops it.
    #[test]
    fn a_copy_with_no_measurable_span_is_left_to_the_limit() {
        let mut forecast = CopyForecast::new(MAX_SOURCE_BYTES);
        let stopped = run(&mut forecast, 1_000, |block| {
            Duration::from_secs(20 * block)
        });
        assert!(
            matches!(stopped, Some((CopyTime::Spent, at)) if at > MAX_SOURCE_READ_DURATION),
            "{stopped:?}"
        );
    }

    /// The fastest span decides, not the average: a copy at 100 MiB/s for its
    /// first half minute that then slows to 1 MiB/s is not stopped early
    /// (at its fastest it would fit), so the limit stops it, as before. The
    /// other way round is the forecast's known cost (L-140): a copy that stays
    /// slow for more than its first minute and then speeds up is judged by the
    /// speed it showed.
    #[test]
    fn the_fastest_span_decides_and_not_the_average() {
        let fast_blocks = 30 * 100 * MIB / BLOCK_BYTES;
        let fast = steady(100 * MIB);
        let slow = steady(MIB);
        let mut forecast = CopyForecast::new(MAX_SOURCE_BYTES);
        let stopped = run(&mut forecast, MAX_SOURCE_BYTES / BLOCK_BYTES, |block| {
            if block <= fast_blocks {
                fast(block)
            } else {
                Duration::from_secs(30) + slow(block - fast_blocks)
            }
        });
        assert!(
            matches!(stopped, Some((CopyTime::Spent, at)) if at > MAX_SOURCE_READ_DURATION),
            "{stopped:?}"
        );
    }

    /// #325: a staging stopped early by its forecast leaves nothing and is a
    /// slow copy for the host, with its own cause, not an invalid source.
    #[test]
    fn a_copy_stopped_early_is_its_own_cause_and_not_an_invalid_source() {
        let source = stand_in(4);
        let mut reader = Cursor::new(&source);
        let mut copied = Vec::new();
        let stopped = copy_until(
            &mut reader,
            &mut copied,
            &NeverCancelled,
            &mut stop_from(2, CopyTime::WillRunOut),
        );
        assert!(
            matches!(stopped, Err(SourceError::CopyWillRunOut)),
            "{stopped:?}"
        );
        assert_eq!(copied.len(), BLOCK);
        assert_eq!(
            open_session_error(SourceError::CopyWillRunOut),
            OpenSessionError::SourceCopyWouldBeTooSlow
        );
    }

    /// #325: a staging whose copy runs out of time fails as a deadline, leaves
    /// no partial private copy behind, and is reported to the open-session use
    /// case as a slow copy, not as an invalid source. The same file is staged
    /// when it has the time: the source was never what was wrong.
    #[tokio::test]
    async fn a_staging_that_runs_out_of_time_is_a_slow_copy_and_leaves_nothing() -> TestResult {
        let root = OwnedRoot::new()?;
        let workspace = root.0.join("workspace");
        let store = FilesystemSessionStore::provision_default(&workspace)?;
        let session_id = SessionId::parse("ses_0123456789abcdef")?;
        let registration = store.register_session(
            &session_id,
            &OperationId::parse("op_0123456789abcdef")?,
            1_000,
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
        let source_path = root.0.join("recording.mp4");
        fs::write(&source_path, stand_in(4))?;

        let (source, initial) = open_source(&source_path)?;
        let stopped = SourceSnapshot::stage_opened(
            &store,
            &session_id,
            &OperationId::parse("op_1111111111111111")?,
            (source, &initial),
            None,
            (&NeverCancelled, &mut up_from(3)),
        );
        let Err(error) = stopped else {
            return Err("a copy that ran out of time was staged".into());
        };
        assert!(matches!(error, SourceError::Deadline), "{error:?}");
        {
            let (directory, _, _hold) = store.source_artifact_directory(&session_id)?;
            let mut left = Vec::new();
            for entry in directory.entries()? {
                left.push(entry?.file_name());
            }
            assert!(left.is_empty(), "the partial copy was left: {left:?}");
        }
        assert_eq!(
            open_session_error(error),
            OpenSessionError::SourceCopyTooSlow
        );

        let (source, initial) = open_source(&source_path)?;
        let staged = SourceSnapshot::stage_opened(
            &store,
            &session_id,
            &OperationId::parse("op_2222222222222222")?,
            (source, &initial),
            None,
            (&NeverCancelled, &mut read_deadline()),
        )?;
        assert_eq!(staged.bytes(), u64::try_from(4 * BLOCK)?);
        assert_eq!(staged.container(), SourceContainer::IsoMedia);
        Ok(())
    }

    /// #325: a deadline is its own cause for the host, and every other refusal
    /// of a source is what it was. Before, a deadline was folded into
    /// `InvalidSource`, and a slow copy told the user the video was at fault.
    #[test]
    fn a_deadline_is_a_slow_copy_and_every_other_refusal_is_unchanged() {
        assert_eq!(
            open_session_error(SourceError::Deadline),
            OpenSessionError::SourceCopyTooSlow
        );
        for invalid in [
            SourceError::InvalidPath,
            SourceError::NotRegularFile,
            SourceError::TooLarge,
            SourceError::UnsupportedContainer,
            SourceError::ChangedDuringStage,
            SourceError::SnapshotChanged,
            SourceError::IdentityFailure,
        ] {
            let described = invalid.to_string();
            assert_eq!(
                open_session_error(invalid),
                OpenSessionError::InvalidSource,
                "{described}"
            );
        }
        assert_eq!(
            open_session_error(SourceError::Cancelled),
            OpenSessionError::Cancelled
        );
        assert_eq!(
            open_session_error(SourceError::SymbolicLink),
            OpenSessionError::SourceIsLink
        );
    }

    /// #266: a write that ran out of room is the session root's filesystem
    /// being too small (`RESOURCE_LIMIT`), not a source that could not be read
    /// (`STORAGE_IO`, with no hint what to do).
    #[test]
    fn a_write_that_ran_out_of_room_is_not_a_source_that_could_not_be_read() {
        for kind in [ErrorKind::StorageFull, ErrorKind::QuotaExceeded] {
            assert_eq!(
                open_session_error(SourceError::Io(IoError::from(kind))),
                OpenSessionError::SourceNoRoom,
                "{kind:?}"
            );
        }
        for kind in [
            ErrorKind::PermissionDenied,
            ErrorKind::UnexpectedEof,
            ErrorKind::Other,
        ] {
            assert_eq!(
                open_session_error(SourceError::Io(IoError::from(kind))),
                OpenSessionError::SourceIo,
                "{kind:?}"
            );
        }
        assert_eq!(
            open_session_error(SourceError::NotRegularFile),
            OpenSessionError::InvalidSource
        );
    }

    /// #310: a source over the size limit is refused as an invalid source,
    /// which is the answer 0.1.0 gave it whatever the free disk, and not as a
    /// root with no room: the room check steps aside for it (the engine's
    /// `ensure_room_for_desktop_copy`), so this is the answer that remains.
    #[test]
    fn a_source_over_the_size_limit_is_an_invalid_source_and_not_a_root_with_no_room() {
        assert_eq!(
            open_session_error(SourceError::TooLarge),
            OpenSessionError::InvalidSource
        );
    }

    /// #369: only what the operating system names plainly is told apart. A
    /// missing name and a folder on the way that is not a folder are both "no
    /// file is at that path"; a refusal is a refusal; every other kind stays
    /// the untyped I/O failure it was, because a cause the system did not name
    /// would be a remediation that is not true.
    #[test]
    fn only_the_kinds_the_system_names_plainly_are_told_apart() {
        for kind in [ErrorKind::NotFound, ErrorKind::NotADirectory] {
            assert_eq!(
                source_path_fault(kind),
                Some(SourcePathFault::NotFound),
                "{kind:?}"
            );
        }
        assert_eq!(
            source_path_fault(ErrorKind::PermissionDenied),
            Some(SourcePathFault::AccessDenied)
        );
        for kind in [
            ErrorKind::Other,
            ErrorKind::UnexpectedEof,
            ErrorKind::StorageFull,
            ErrorKind::AlreadyExists,
            ErrorKind::InvalidInput,
            ErrorKind::TimedOut,
        ] {
            assert_eq!(source_path_fault(kind), None, "{kind:?}");
            assert!(
                matches!(
                    source_path_failure(IoError::from(kind)),
                    SourceError::Io(error) if error.kind() == kind
                ),
                "{kind:?}"
            );
        }
        assert!(matches!(
            source_path_failure(IoError::from(ErrorKind::PermissionDenied)),
            SourceError::Path(SourcePathFault::AccessDenied)
        ));
    }

    /// #369: a path fault reaches the open-session use case as itself, and an
    /// I/O failure later in a staging stays untyped even when it says "not
    /// found": a private file of `VSift`'s own that vanished is not "no file is
    /// at the path you gave".
    #[test]
    fn a_path_fault_is_reported_as_itself_and_a_later_not_found_is_not_one() {
        for fault in SourcePathFault::ALL {
            assert_eq!(
                open_session_error(SourceError::Path(fault)),
                OpenSessionError::SourcePath(fault),
                "{fault}"
            );
        }
        assert_eq!(
            open_session_error(SourceError::Io(IoError::from(ErrorKind::NotFound))),
            OpenSessionError::SourceIo
        );
    }

    /// #369: a missing file, a missing folder on the way and a folder on the
    /// way that is a file are all "no file is at that path", found where the
    /// path is opened.
    #[test]
    fn a_path_that_names_no_file_is_not_found_whatever_is_missing() -> TestResult {
        let root = OwnedRoot::new()?;
        let present = root.0.join("present.mp4");
        fs::write(&present, stand_in(1))?;
        for path in [
            root.0.join("not-there.mp4"),
            root.0.join("no-folder").join("not-there.mp4"),
            present.join("not-there.mp4"),
        ] {
            let Err(error) = open_source(&path) else {
                return Err("a path that names no file was opened".into());
            };
            assert!(
                matches!(error, SourceError::Path(SourcePathFault::NotFound)),
                "{error:?}"
            );
        }
        // The one that is there is opened.
        assert!(open_source(&present).is_ok());
        Ok(())
    }

    /// #369: a folder is told apart from a refusal where the system refuses to
    /// open it (Windows, which reports a denied access) and is no regular file
    /// where the system opens it (Linux and macOS: `INVALID_SOURCE`, the answer
    /// each platform has always given, and neither changes).
    #[test]
    fn a_folder_is_a_folder_where_it_cannot_be_opened_and_no_regular_file_where_it_can()
    -> TestResult {
        let root = OwnedRoot::new()?;
        let folder = root.0.join("a-folder");
        fs::create_dir(&folder)?;
        let Err(error) = open_source(&folder) else {
            return Err("a folder was opened as a source".into());
        };
        #[cfg(windows)]
        assert!(
            matches!(error, SourceError::Path(SourcePathFault::IsFolder)),
            "{error:?}"
        );
        #[cfg(not(windows))]
        assert!(matches!(error, SourceError::NotRegularFile), "{error:?}");
        Ok(())
    }

    /// #369: a file the account may not read is told apart from a missing one.
    /// Unix only: a Windows account cannot be denied a file it just made
    /// without changing its access list, which a test does not do; the
    /// classification of the kind is checked above for every platform. An
    /// account that reads anything (root) cannot be refused, so the test says
    /// it checked nothing there.
    #[cfg(unix)]
    #[test]
    fn a_file_that_may_not_be_read_is_access_denied() -> TestResult {
        use std::os::unix::fs::PermissionsExt as _;

        let root = OwnedRoot::new()?;
        let locked = root.0.join("locked.mp4");
        fs::write(&locked, stand_in(1))?;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))?;
        if fs::File::open(&locked).is_ok() {
            eprintln!(
                "SKIPPED: this account can read a file with no permissions; the test checks nothing here"
            );
            return Ok(());
        }
        let Err(error) = open_source(&locked) else {
            return Err("a file that may not be read was opened".into());
        };
        assert!(
            matches!(error, SourceError::Path(SourcePathFault::AccessDenied)),
            "{error:?}"
        );
        Ok(())
    }
}
