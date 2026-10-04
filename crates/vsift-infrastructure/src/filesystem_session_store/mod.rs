//! Capability-scoped session storage for the qualified ephemeral desktop profile.
//!
//! This module holds the adapter's public types, stored metadata shapes and the
//! small handle-relative I/O helpers every part shares. The operations live in
//! submodules by concern: the root and admission (`root`), session
//! initialization (`initialization`), how one commit touches the filesystem
//! (`commit`: durability, fault points, the test trace), generation
//! publication (`publication`),
//! reading and validating the committed chain (`chain`), stored-metadata
//! validation (`stored`), lifecycle generations (`lifecycle`), verified record
//! reads (`reads`), evidence (`evidence`), work directories (`work`), the
//! session index (`index`), cleanup (`cleanup`), retained bundles (`bundle`)
//! recoverable jobs with their chunk checkpoints (`jobs`, `job_records`) and
//! worker request records (`worker_requests`).

#[cfg(test)]
mod abandon_tests;
mod bundle;
mod chain;
mod cleanup;
mod commit;
mod evidence;
mod index;
#[cfg(test)]
mod index_tests;
mod initialization;
mod job_records;
#[cfg(test)]
mod job_tests;
mod jobs;
mod lifecycle;
#[cfg(test)]
mod p10_tests;
mod publication;
mod reads;
mod root;
mod stored;
#[cfg(test)]
mod tests;
mod work;
#[cfg(test)]
mod worker_request_tests;
mod worker_requests;
#[cfg(test)]
mod workspace_tests;

use std::{
    collections::BTreeSet,
    error::Error,
    fmt, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, DirBuilder, File, Metadata, OpenOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vsift_application::{SessionStorageError, StorageCapabilities};
use vsift_domain::{
    DurabilityRequirement, EvidenceMediaKind, EvidenceRecord, OperationId, SessionArtifactKind,
    SessionId, SessionLifetime, SessionLifetimePolicy, SessionPhase, SourceId, StorageGeneration,
    WorkspacePolicy, WorkspaceRetention,
};

use crate::{VerifiedSourceIdentity, file_lock::HeldFileLock};

pub use job_records::{
    decode_chunk_checkpoint, decode_job_record, encode_chunk_checkpoint, encode_job_record,
};
pub use jobs::{FilesystemJobOwner, JOB_CANCEL_POLL};
pub use root::{FREE_SPACE_RESERVE_BYTES, FreeSpaceCheck};
pub(crate) use root::{RootProvisioningState, root_provisioning_state};
pub use worker_requests::{
    MAX_RECORDED_DOCUMENT_BYTES, MAX_RECORDED_STEPS, MAX_REQUEST_RECORD_BYTES, MAX_REQUEST_RECORDS,
    RecordedRequestResult, RequestRecordWrite, WorkerRequestClaim, WorkerRequestOwner,
    WorkerRequestRecord, decode_request_record, encode_request_record,
};

const MAX_METADATA_BYTES: u64 = 64 * 1024;
/// Largest generation manifest (and retained bundle manifest), which lists
/// every artifact: 512 entries of about 200 bytes need about 100 KiB (ADR
/// 0020 D-2). Every other metadata file keeps [`MAX_METADATA_BYTES`].
const MAX_MANIFEST_BYTES: u64 = 128 * 1024;
const OWNERSHIP_FILE: &str = "ownership.json";
const COORDINATION_DIRECTORY: &str = "coordination";
const SESSIONS_DIRECTORY: &str = "sessions";
const SESSION_INDEX_DIRECTORY: &str = "session-index";
const INITIALIZATION_LOCK: &str = "session-initialize.lock";
/// Held exclusively by the one process provisioning a new root, from just after
/// the root and its coordination directory exist until the ownership marker is
/// complete, then removed. Concurrent openers read a held lock as "a creator is
/// still working" and wait, instead of rejecting the unmarked root (issue #131).
const PROVISIONING_LOCK: &str = "root-provisioning.lock";
const CURRENT_FILE: &str = "current.json";
const GENERATIONS_DIRECTORY: &str = "generations";
const RECORDS_DIRECTORY: &str = "records";
const ARTIFACTS_DIRECTORY: &str = "artifacts";
const ATTEMPTS_DIRECTORY: &str = "attempts";
/// Private scratch space for work in progress on one session, such as the
/// speech chunks a retranscription hands to the recognizer.
const WORK_DIRECTORY: &str = "work";
/// Every work directory is named this prefix and 16 lowercase hex digits.
const WORK_PREFIX: &str = "asr-";
const WORK_RANDOM_BYTES: usize = 8;
/// Held exclusively by the live run that owns a work directory.
const WORK_LOCK_FILE: &str = "work.lock";
/// Most leftover work directories one new run removes.
const MAX_REMOVED_WORK_DIRECTORIES: usize = 8;
const INITIAL_GENERATION_FILE: &str = "0.json";
/// The newest generation whose chain the session's writer has verified
/// (issue #164): readers walk the chain down only to it.
const CHAIN_CHECKPOINT_FILE: &str = "chain-verified.json";
const STORAGE_SCHEMA_VERSION: u16 = 1;
const STORAGE_LAYOUT_VERSION: u16 = 1;
const MAX_ADMISSION_CAPACITY: u16 = 64;
/// The admission capacity of a desktop root created on first use, in weight
/// units: a whisper.cpp run of four threads, or two visual windows, at once.
pub const DEFAULT_ADMISSION_CAPACITY: u16 = 4;
const MAX_GENERATIONS_PER_SESSION: u64 = 4_096;
/// Most artifacts one session holds (ADR 0020 D-2; 256 before P10).
const MAX_SESSION_ARTIFACTS: usize = 512;
/// Most artifact bytes one session holds: 10 GiB.
const MAX_SESSION_ARTIFACT_BYTES: u64 = 10 * 1024 * 1024 * 1024;
const HEX: &[u8; 16] = b"0123456789abcdef";

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
    /// The directory holds no ownership marker at all. `VSift` writes the marker
    /// last when it creates a root, so a directory without one was made by
    /// someone else (or is a creator's first step) and is never adopted. It is
    /// kept apart from [`Self::InvalidOwnership`] so a person who pointed
    /// `--session-root` at a folder of their own is told so, not that stored
    /// data is damaged.
    OwnershipMarkerMissing,
    /// The ownership marker is present but malformed, linked, unreadable, or not a
    /// supported `VSift` marker.
    InvalidOwnership,
    /// A required contained directory or stable lock anchor is invalid.
    InvalidLayout,
    /// The selected root already exists; provisioning never adopts or overwrites it.
    RootAlreadyExists,
    /// The immutable root admission capacity is outside the supported bound.
    InvalidAdmissionCapacity,
    /// A durable workspace was requested where OS-crash durability is not
    /// qualified (anything but Ubuntu 24.04 on local ext4, ADR 0010);
    /// nothing was created.
    DurabilityUnavailable,
}

impl fmt::Display for SessionStoreOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::RootMustBeAbsolute => "session storage root must be an absolute path",
            Self::RootUnavailable => "session storage root is unavailable",
            Self::RootNotDirectory => "session storage root is not a directory",
            Self::RootNotPrivate => "session storage root permissions are not private",
            Self::OwnershipMarkerMissing => "session storage root has no ownership marker",
            Self::InvalidOwnership => "session storage ownership marker is invalid",
            Self::InvalidLayout => "session storage layout is invalid",
            Self::RootAlreadyExists => "session storage root already exists",
            Self::InvalidAdmissionCapacity => "session storage admission capacity is invalid",
            Self::DurabilityUnavailable => {
                "durable publication is not qualified for this session storage root"
            }
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
    /// What this root may acknowledge on this host, decided once when the
    /// store is opened (`durable_profile`, ADR 0010).
    capabilities: StorageCapabilities,
    /// The operator policy of a worker workspace (ADR 0021 D1, D2), read
    /// from the root's marker; `None` for an ordinary desktop root. It is
    /// immutable: every revalidation requires the marker to still say the
    /// same.
    workspace: Option<WorkspacePolicy>,
    /// The last head this store instance verified, so several reads in one
    /// command walk the chain once (#164). Adapter state, never shared
    /// between instances or processes.
    verified_heads: VerifiedHeadCache,
}

/// Root-wide weighted permit backed by stable OS-locked slot files.
///
/// Dropping the value releases every reservation. The slot files themselves are
/// immutable anchors and are never replaced during normal operation.
pub struct FilesystemAdmissionPermit {
    _slots: Vec<HeldFileLock>,
}

/// A committed metadata snapshot protected by a shared session lifetime hold.
pub struct SessionReadHold {
    _lifetime_lock: HeldFileLock,
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
    _lifetime_lock: HeldFileLock,
}

/// A private, uniquely named scratch directory inside one open session.
///
/// It lives at `sessions/<id>/work/asr-<16 hex>` under the owner-private
/// root, so it is as private as the session's evidence, never shared between
/// sessions, and removed with the session by `session clean`. While it exists
/// it holds a shared hold on the session's lifetime, so closing or cleaning
/// the session returns busy instead of removing files a run is using, and an
/// exclusive lock on its own `work.lock` so a later run can tell a live
/// directory from one left by a killed process. Dropping it removes it; a
/// directory left by a killed process is removed by the next run that
/// creates one in the same session (at most eight at a time).
pub struct SessionWorkDirectory {
    path: PathBuf,
    owner: Option<HeldFileLock>,
    _hold: SessionReadHold,
}

impl SessionWorkDirectory {
    /// Absolute path of the directory, for providers that need one.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SessionWorkDirectory {
    fn drop(&mut self) {
        // Windows cannot remove a directory holding an open file, so the lock
        // is released and closed first. Nothing adopts an existing work
        // directory, so no other run can start using it in between.
        if let Some(owner) = self.owner.take() {
            let _ = owner.release();
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// The committed private source copy of one open session, under a shared
/// lifetime hold, with the identity its lifecycle record gives it.
pub(crate) struct CommittedSource {
    pub(crate) directory: Dir,
    pub(crate) directory_path: PathBuf,
    pub(crate) hold: SessionReadHold,
    pub(crate) source_id: SourceId,
    pub(crate) file_name: String,
    pub(crate) bytes: u64,
    /// The copy's on-disk identity recorded after its last full
    /// verification by an evidence call (ADR 0019 D1), if any.
    pub(crate) verified_identity: Option<VerifiedSourceIdentity>,
}

/// One media file an evidence call commits.
#[derive(Clone, Copy, Debug)]
pub struct EvidenceMediaFile<'a> {
    /// What the file is.
    pub kind: EvidenceMediaKind,
    /// The file's bytes.
    pub bytes: &'a [u8],
}

/// A session's committed evidence and what it may still take (ADR 0019 D4).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceInventory {
    records: Vec<EvidenceRecord>,
    status: SessionStatus,
    evidence_artifacts: usize,
    known_media: BTreeSet<String>,
}

impl EvidenceInventory {
    /// Every committed evidence record, oldest first, each decoded strictly.
    #[must_use]
    pub fn records(&self) -> &[EvidenceRecord] {
        &self.records
    }

    /// The committed session state the records were read from.
    #[must_use]
    pub const fn status(&self) -> &SessionStatus {
        &self.status
    }

    /// SHA-256 digests of the committed frame images and audio clips.
    #[must_use]
    pub const fn known_media(&self) -> &BTreeSet<String> {
        &self.known_media
    }

    /// Evidence artifacts the session can still take: the smaller of what is
    /// left of the evidence sub-budget and of the session's artifact slots.
    #[must_use]
    pub const fn evidence_slots_left(&self) -> usize {
        let evidence = crate::MAX_EVIDENCE_ARTIFACTS.saturating_sub(self.evidence_artifacts);
        let total = MAX_SESSION_ARTIFACTS.saturating_sub(self.status.artifact_count);
        if evidence < total { evidence } else { total }
    }

    /// Artifact bytes the session can still take.
    #[must_use]
    pub const fn bytes_left(&self) -> u64 {
        MAX_SESSION_ARTIFACT_BYTES.saturating_sub(self.status.artifact_bytes)
    }
}

/// Verified committed lifecycle state for one disposable session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionStatus {
    session_id: SessionId,
    source_id: SourceId,
    source_bytes: u64,
    phase: SessionPhase,
    lifetime: SessionLifetime,
    generation: StorageGeneration,
    artifact_count: usize,
    artifact_bytes: u64,
}

/// Explicit portability requested for a user-selected retained bundle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BundleSourcePolicy {
    /// Export evidence metadata only; matching original media is needed later.
    EvidenceOnly,
    /// Copy and verify the private source snapshot into the bundle.
    IncludeSource,
}

/// A validated, explicitly retained data-only bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleStatus {
    session_id: SessionId,
    source_id: SourceId,
    source_bytes: u64,
    source_policy: BundleSourcePolicy,
    artifact_count: usize,
    artifact_bytes: u64,
    manifest_sha256: String,
}

/// Result of examining one positively identified disposable session for cleanup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanOutcome {
    /// The session remains open and within its expiry.
    Ineligible,
    /// A dry run found a closed, expired, or abandoned session.
    Eligible,
    /// A claimed owned session was quarantined and removed.
    Removed,
    /// The registration was gone by the time cleanup claimed it: another
    /// cleanup, or a failed open removing its own registration (#277), got
    /// there first. There is nothing left to examine and nothing went wrong.
    Gone,
}

/// Stable held registration while a source is staged and activated.
///
/// A cleaner cannot classify a suspended opener as abandoned merely from its
/// registration timestamp; this OS lock remains authoritative until drop/crash.
pub struct SessionRegistration {
    _marker_lock: HeldFileLock,
}

/// One bounded page from a fixed index bucket, with no media or transcript data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionIndexPage {
    session_ids: Vec<SessionId>,
    next_bucket: Option<u16>,
}

impl SessionIndexPage {
    /// Positively registered session IDs in this bucket.
    #[must_use]
    pub fn session_ids(&self) -> &[SessionId] {
        &self.session_ids
    }

    /// Cursor to the next bounded bucket, if any.
    #[must_use]
    pub const fn next_bucket(&self) -> Option<u16> {
        self.next_bucket
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SessionIndexMarker {
    schema_version: u16,
    session_id: String,
    operation_id: String,
    registered_at_unix_seconds: u64,
}

impl MetadataVersion for SessionIndexMarker {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }
}

impl BundleStatus {
    /// Session from which the bundle originated.
    #[must_use]
    pub const fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Hash of the source required for later re-extraction.
    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    /// Size of the required source.
    #[must_use]
    pub const fn source_bytes(&self) -> u64 {
        self.source_bytes
    }

    /// Whether source bytes were included.
    #[must_use]
    pub const fn source_policy(&self) -> BundleSourcePolicy {
        self.source_policy
    }

    /// Number of validated evidence artifacts.
    #[must_use]
    pub const fn artifact_count(&self) -> usize {
        self.artifact_count
    }

    /// Total validated evidence bytes.
    #[must_use]
    pub const fn artifact_bytes(&self) -> u64 {
        self.artifact_bytes
    }

    /// SHA-256 of the validated `bundle.json`, as 64 lowercase hexadecimal
    /// digits: the manifest names every file and its digest, so this one
    /// digest identifies the whole bundle (a worker records it, P11).
    #[must_use]
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BundleManifest {
    schema_version: u16,
    format: String,
    session_id: String,
    source_id: String,
    source_bytes: u64,
    source_included: bool,
    publication: String,
    artifacts: Vec<StoredArtifact>,
}

impl MetadataVersion for BundleManifest {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }
}

impl SessionStatus {
    /// Session described by this committed generation.
    #[must_use]
    pub const fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Hash identity of the private source copy.
    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    /// Original source byte count; no content is returned.
    #[must_use]
    pub const fn source_bytes(&self) -> u64 {
        self.source_bytes
    }

    /// Committed open or closed phase.
    #[must_use]
    pub const fn phase(&self) -> SessionPhase {
        self.phase
    }

    /// Validated expiry policy for this session.
    #[must_use]
    pub const fn lifetime(&self) -> SessionLifetime {
        self.lifetime
    }

    /// Generation used for optimistic later mutations.
    #[must_use]
    pub const fn generation(&self) -> StorageGeneration {
        self.generation
    }

    /// Number of committed evidence artifacts.
    #[must_use]
    pub const fn artifact_count(&self) -> usize {
        self.artifact_count
    }

    /// Total committed evidence bytes, excluding the source.
    #[must_use]
    pub const fn artifact_bytes(&self) -> u64 {
        self.artifact_bytes
    }
}

enum LifecycleUpdate {
    Keep,
    Activate {
        source_id: String,
        source_name: String,
        source_bytes: u64,
        now: u64,
        artifacts: Vec<StoredArtifact>,
        /// The lifetime rules of the root the session opens in.
        lifetime: SessionLifetimePolicy,
    },
    Renew {
        now: u64,
    },
    Close,
    AddArtifact {
        artifact: StoredArtifact,
        now: u64,
    },
    /// An evidence call's media and record, all in one generation, and the
    /// source identity it verified, if it hashed the copy in full.
    AddEvidence {
        artifacts: Vec<StoredArtifact>,
        verified_identity: Option<String>,
        now: u64,
    },
}

fn lowercase_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    text
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

fn map_open_error(error: SessionStoreOpenError) -> SessionStorageError {
    match error {
        SessionStoreOpenError::RootNotPrivate => SessionStorageError::AccessDenied,
        // A root that opened and then lost its marker was damaged underneath
        // a running operation, which is integrity damage like any other.
        SessionStoreOpenError::OwnershipMarkerMissing
        | SessionStoreOpenError::InvalidOwnership
        | SessionStoreOpenError::InvalidLayout => SessionStorageError::IntegrityFailure,
        _ => SessionStorageError::Io,
    }
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

struct CommittedManifest {
    manifest: GenerationManifest,
    digest: String,
}

/// How far a read validates the manifest chain below the committed head.
#[derive(Clone, Copy)]
enum ChainCheck<'a> {
    /// Every generation down to 0: retained exports and cleanup.
    Full,
    /// Down to the newest verified anchor: the session's checkpoint or this
    /// store instance's last verified head (#164).
    Incremental(Option<&'a VerifiedHeadCache>),
}

trait MetadataVersion {
    fn schema_version(&self) -> u16;
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

/// Opens `name` without following a link and requires a regular file with
/// exactly one link.
///
/// A file with more links is rejected as [`io::ErrorKind::InvalidData`]: it
/// could be a hard link to data outside the root. A file with no link left
/// was renamed over or removed between the open and the metadata read, so the
/// name no longer names it; that is [`io::ErrorKind::NotFound`], never an
/// integrity failure, and [`open_replaced_file`] tries the name again.
fn open_regular_file(directory: &Dir, name: &str, write: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(write).follow(FollowSymlinks::No);
    checked_regular_file(directory.open_with(name, &options)?)
}

/// The single-link regular-file check of [`open_regular_file`] on an opened file.
fn checked_regular_file(file: File) -> io::Result<File> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    match metadata.nlink() {
        1 => Ok(file),
        0 => Err(io::ErrorKind::NotFound.into()),
        _ => Err(io::ErrorKind::InvalidData.into()),
    }
}

/// How long a reader keeps trying a file a writer is replacing by rename.
///
/// The window is a rename: microseconds when the machine is idle, a few
/// milliseconds under heavy load. A file still missing after this is missing.
const REPLACED_FILE_RETRY: std::time::Duration = std::time::Duration::from_millis(500);
/// Attempts that only yield the thread before the retry starts sleeping.
const REPLACED_FILE_SPINS: u32 = 8;

/// Opens a metadata file that a correct writer replaces by staging it and
/// renaming it over the old one: the commit pointer, the chain checkpoint,
/// job records, bindings and index entries.
///
/// A reader can meet that replacement. On every platform it can open the old
/// file just before the rename and read its metadata just after, when the
/// old file has no link left; on Windows the name can also be absent for a
/// moment during the rename, and a file being deleted can refuse to open.
/// Those states are reported as `NotFound` (or, on Windows, `PermissionDenied`)
/// and are retried, a few times at once and then a millisecond apart, for
/// at most [`REPLACED_FILE_RETRY`]. Whatever the reader then opens is a file
/// the writer committed, old or new; the old one is a consistent earlier
/// snapshot. A hard link, a non-regular file and every other error are
/// returned at once, and a file still absent after the budget is absent, so
/// real damage or tampering is still reported.
fn open_replaced_file(directory: &Dir, name: &str, write: bool) -> io::Result<File> {
    open_replaced_file_with(|| open_regular_file(directory, name, write))
}

/// [`open_replaced_file`] over any opener; the retry policy on its own, so a
/// test can drive the exact interleavings a concurrent writer causes.
fn open_replaced_file_with(mut open: impl FnMut() -> io::Result<File>) -> io::Result<File> {
    let started = std::time::Instant::now();
    let mut attempts = 0_u32;
    loop {
        match open() {
            Err(error)
                if in_replacement_window(&error) && started.elapsed() < REPLACED_FILE_RETRY =>
            {
                attempts = attempts.saturating_add(1);
                if attempts <= REPLACED_FILE_SPINS {
                    std::thread::yield_now();
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
            result => return result,
        }
    }
}

/// Whether an open failed in a way a concurrent rename-replace can cause.
fn in_replacement_window(error: &io::Error) -> bool {
    match error.kind() {
        io::ErrorKind::NotFound => true,
        // A file being deleted (the replaced one) refuses a new open with
        // access denied on Windows only.
        io::ErrorKind::PermissionDenied => cfg!(windows),
        _ => false,
    }
}

fn read_bounded(file: File) -> io::Result<Vec<u8>> {
    read_bounded_to(file, MAX_METADATA_BYTES)
}

/// Reads a generation or bundle manifest, bounded by [`MAX_MANIFEST_BYTES`].
fn read_bounded_manifest(file: File) -> io::Result<Vec<u8>> {
    read_bounded_to(file, MAX_MANIFEST_BYTES)
}

fn read_bounded_to(file: File, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
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

/// Maps a failure to open or read committed state.
///
/// The committed layout is known, so a missing, mistyped, oversized, linked
/// or otherwise unexpected entry is damage: an integrity failure. A failure
/// of the storage itself says nothing about the bytes and is a storage
/// failure instead: an I/O error (`EIO`, for example from a disk that fails
/// or from ext4 after it shut itself down on a write error), a read-only or
/// full filesystem, a stale network handle, a busy resource or an interrupted
/// call. Before P10 PR 4 every such failure was reported as an integrity
/// failure, so a failing disk looked like tampered evidence; the crash
/// campaign's write-error layer found it.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Result::map_err requires ownership of the source error"
)]
fn map_committed_io(error: io::Error) -> SessionStorageError {
    if is_storage_failure(&error) {
        map_storage_io(error)
    } else {
        SessionStorageError::IntegrityFailure
    }
}

/// Whether `error` is a failure of the storage rather than of the stored
/// layout (see [`map_committed_io`]).
fn is_storage_failure(error: &io::Error) -> bool {
    #[cfg(unix)]
    if error.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) {
        return true;
    }
    matches!(
        error.kind(),
        io::ErrorKind::ReadOnlyFilesystem
            | io::ErrorKind::StorageFull
            | io::ErrorKind::QuotaExceeded
            | io::ErrorKind::StaleNetworkFileHandle
            | io::ErrorKind::ResourceBusy
            | io::ErrorKind::Interrupted
            | io::ErrorKind::TimedOut
            | io::ErrorKind::OutOfMemory
    )
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Result::map_err requires ownership of the source error"
)]
fn map_storage_io(error: io::Error) -> SessionStorageError {
    #[cfg(test)]
    report_erased_io_error(&error);
    match error.kind() {
        io::ErrorKind::PermissionDenied => SessionStorageError::AccessDenied,
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => {
            SessionStorageError::CapacityExhausted
        }
        _ => SessionStorageError::Io,
    }
}

/// Under test, writes the OS error that [`SessionStorageError::Io`] erases,
/// with the descriptor load and the call site, to the test's captured output.
///
/// `Io` is deliberately a unit variant: the public contract never carries OS
/// detail. A rare `Io` in a concurrent test on one platform is then
/// undiagnosable from the failure alone, so the unit tests keep the errno,
/// the open-descriptor count against the soft limit (a process near its
/// descriptor limit fails any open with `EMFILE`) and a backtrace. The test
/// harness shows captured output only for a failing test, and worker threads
/// inherit the capture of the test that spawned them.
#[cfg(test)]
fn report_erased_io_error(error: &io::Error) {
    let kind = error.kind();
    if matches!(
        kind,
        io::ErrorKind::PermissionDenied | io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded
    ) {
        return;
    }
    eprintln!(
        "session storage I/O failed: {kind:?}, OS error {:?}, {}\n{}",
        error.raw_os_error(),
        descriptor_load(),
        std::backtrace::Backtrace::force_capture()
    );
}

/// This process's open descriptors and its soft descriptor limit.
#[cfg(all(test, unix))]
fn descriptor_load() -> String {
    let open = fs::read_dir("/dev/fd").map_or_else(
        |error| format!("unknown ({error})"),
        |entries| entries.count().to_string(),
    );
    let limit = rustix::process::getrlimit(rustix::process::Resource::Nofile)
        .current
        .map_or_else(|| "unlimited".to_owned(), |limit| limit.to_string());
    format!("{open} descriptors open, soft limit {limit}")
}

/// Windows has no small per-process handle limit to report.
#[cfg(all(test, not(unix)))]
fn descriptor_load() -> String {
    "no descriptor limit on this platform".to_owned()
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

fn session_bucket(session_id: &SessionId) -> String {
    let digest = Sha256::digest(session_id.as_str().as_bytes());
    format!("{:02x}", digest[0])
}

fn copy_and_hash_bounded(
    mut source: File,
    destination: &mut impl Write,
    expected_bytes: u64,
) -> Result<String, SessionStorageError> {
    let started = std::time::Instant::now();
    let metadata = source.metadata().map_err(map_storage_io)?;
    if !metadata.is_file() || metadata.len() != expected_bytes || !has_one_link(&metadata) {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        if started.elapsed() > crate::MAX_SOURCE_READ_DURATION {
            return Err(SessionStorageError::CapacityExhausted);
        }
        let count = source.read(&mut buffer).map_err(map_storage_io)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(count).map_err(|_| SessionStorageError::CapacityExhausted)?)
            .ok_or(SessionStorageError::CapacityExhausted)?;
        if total > expected_bytes {
            return Err(SessionStorageError::IntegrityFailure);
        }
        destination
            .write_all(&buffer[..count])
            .map_err(map_storage_io)?;
        hasher.update(&buffer[..count]);
    }
    if total != expected_bytes {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let mut digest = String::with_capacity(64);
    for byte in hasher.finalize() {
        digest.push(char::from(HEX[usize::from(byte >> 4)]));
        digest.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(digest)
}

fn hash_bounded(source: File, expected_bytes: u64) -> Result<String, SessionStorageError> {
    copy_and_hash_bounded(source, &mut io::sink(), expected_bytes)
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
    /// The policy of a worker workspace (P11, ADR 0021 D1). Absent for an
    /// ordinary desktop root, whose marker is unchanged since P03. A build
    /// before P11 refuses a marker that has it (unknown fields are refused),
    /// so an older build never opens a workspace as a desktop root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace: Option<StoredWorkspacePolicy>,
}

/// A workspace's policy as its marker records it; the admission capacity is
/// the marker's own `admission_capacity`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredWorkspacePolicy {
    profile: StoredWorkspaceProfile,
    durability: StoredDurability,
    session_retention_seconds: u64,
}

/// The one workspace profile.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredWorkspaceProfile {
    /// A worker workspace created by `session init-workspace`.
    DurableWorkspace,
}

impl StoredWorkspacePolicy {
    fn from_policy(policy: WorkspacePolicy) -> Self {
        Self {
            profile: StoredWorkspaceProfile::DurableWorkspace,
            durability: StoredDurability::from_requirement(policy.durability()),
            session_retention_seconds: policy.retention().seconds(),
        }
    }

    /// The validated policy, with the marker's admission capacity.
    fn policy(self, admission_capacity: u16) -> Option<WorkspacePolicy> {
        let StoredWorkspaceProfile::DurableWorkspace = self.profile;
        let retention = WorkspaceRetention::from_seconds(self.session_retention_seconds).ok()?;
        let capacity = std::num::NonZeroU16::new(admission_capacity)?;
        WorkspacePolicy::new(self.durability.requirement(), capacity, retention).ok()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GenerationManifest {
    schema_version: u16,
    session_id: String,
    operation_id: String,
    generation: u64,
    previous_manifest_sha256: Option<String>,
    /// How the session publishes, fixed at generation 0 and carried
    /// unchanged by every later generation (ADR 0020). Absent, as in every
    /// session written before P10, means ephemeral; an ephemeral manifest
    /// still omits it, so older builds keep reading ephemeral sessions.
    #[serde(default, skip_serializing_if = "StoredDurability::is_ephemeral")]
    durability: StoredDurability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lifecycle: Option<StoredLifecycle>,
}

/// A session's publication mode as its manifests record it.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredDurability {
    /// Process-crash-consistent publication (ADR 0010's desktop profile).
    #[default]
    Ephemeral,
    /// The ordered, synchronised protocol of ADR 0020, acknowledged only on a
    /// qualified OS-crash-durable profile.
    Durable,
}

impl StoredDurability {
    const fn from_requirement(requirement: DurabilityRequirement) -> Self {
        match requirement {
            DurabilityRequirement::Ephemeral => Self::Ephemeral,
            DurabilityRequirement::Durable => Self::Durable,
        }
    }

    const fn requirement(self) -> DurabilityRequirement {
        match self {
            Self::Ephemeral => DurabilityRequirement::Ephemeral,
            Self::Durable => DurabilityRequirement::Durable,
        }
    }

    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    const fn is_ephemeral(&self) -> bool {
        matches!(self, Self::Ephemeral)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredLifecycle {
    phase: StoredSessionPhase,
    opened_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
    source_id: String,
    source_name: String,
    source_bytes: u64,
    #[serde(default)]
    artifacts: Vec<StoredArtifact>,
    /// Digest of the source copy's on-disk identity after its last full
    /// verification by an evidence call (ADR 0019 D1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    verified_source_identity: Option<String>,
    /// The retention of the worker workspace the session was opened in
    /// (ADR 0021 D2), which bounds its expiry; absent for a desktop
    /// session, whose record is unchanged since P05.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace_retention_seconds: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredArtifact {
    kind: StoredArtifactKind,
    name: String,
    sha256: String,
    bytes: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredArtifactKind {
    FramePng,
    AudioPcm,
    TranscriptRecord,
    VisualIndexRecord,
    AudioWav,
    EvidenceRecord,
}

impl StoredArtifactKind {
    fn from_domain(kind: SessionArtifactKind) -> Self {
        match kind {
            SessionArtifactKind::FramePng => Self::FramePng,
            SessionArtifactKind::AudioPcm => Self::AudioPcm,
            SessionArtifactKind::TranscriptRecord => Self::TranscriptRecord,
            SessionArtifactKind::VisualIndexRecord => Self::VisualIndexRecord,
            SessionArtifactKind::AudioWav => Self::AudioWav,
            SessionArtifactKind::EvidenceRecord => Self::EvidenceRecord,
        }
    }

    const fn from_media(kind: EvidenceMediaKind) -> Self {
        match kind {
            EvidenceMediaKind::FramePng => Self::FramePng,
            EvidenceMediaKind::AudioWav => Self::AudioWav,
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::FramePng => "png",
            Self::AudioPcm => "pcm",
            Self::AudioWav => "wav",
            Self::TranscriptRecord | Self::VisualIndexRecord | Self::EvidenceRecord => "json",
        }
    }

    /// Whether the kind counts against the evidence sub-budget: frame and
    /// crop images, audio and evidence records (ADR 0019 D4).
    const fn is_evidence(self) -> bool {
        match self {
            Self::FramePng | Self::AudioPcm | Self::AudioWav | Self::EvidenceRecord => true,
            Self::TranscriptRecord | Self::VisualIndexRecord => false,
        }
    }

    /// Largest committed artifact of this kind, enforced on write and on read.
    const fn max_bytes(self) -> usize {
        match self {
            Self::FramePng => crate::MAX_FRAME_BYTES,
            Self::AudioPcm => crate::MAX_AUDIO_BYTES,
            Self::TranscriptRecord => crate::MAX_TRANSCRIPT_RECORD_BYTES,
            Self::VisualIndexRecord => crate::MAX_VISUAL_INDEX_RECORD_BYTES,
            Self::AudioWav => crate::MAX_AUDIO_WAV_BYTES,
            Self::EvidenceRecord => crate::MAX_EVIDENCE_RECORD_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredSessionPhase {
    Open,
    Closed,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommitPointer {
    schema_version: u16,
    generation: u64,
    manifest_sha256: String,
}

/// `chain-verified.json`: the newest generation whose whole chain the
/// session's writer has verified, and that generation's manifest digest
/// (#164). Written only by the writer, under the writer lock, after a
/// successful publication; never ahead of the head it was written for.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ChainCheckpoint {
    schema_version: u16,
    generation: u64,
    manifest_sha256: String,
}

impl MetadataVersion for ChainCheckpoint {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }
}

/// One verified head: the session, its generation and manifest digest.
#[derive(Clone, Debug, Eq, PartialEq)]
struct VerifiedHead {
    session_id: SessionId,
    generation: u64,
    manifest_sha256: String,
}

/// The last head one store instance verified (#164).
///
/// One entry is enough for its purpose, several reads of one session in one
/// command; a read of another session replaces it.
#[derive(Debug, Default)]
struct VerifiedHeadCache(Mutex<Option<VerifiedHead>>);

impl VerifiedHeadCache {
    /// The verified head of `session_id`, if this instance holds one.
    fn get(&self, session_id: &SessionId) -> Option<VerifiedHead> {
        // A poisoned lock still holds a value that was set only after a
        // successful verification, so it stays usable.
        let guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        guard
            .as_ref()
            .filter(|head| &head.session_id == session_id)
            .cloned()
    }

    fn set(&self, head: VerifiedHead) {
        let mut guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *guard = Some(head);
    }
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
