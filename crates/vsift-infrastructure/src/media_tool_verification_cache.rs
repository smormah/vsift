//! Private per-user record of media-tool setups that already passed verification,
//! and the fingerprint that keys it.
//!
//! The automatic media-tool preflight (ADR 0015) runs the reviewed fixture once
//! per tool identity rather than before every operation. This module derives
//! that identity and stores passes in a small, strict, bounded JSON record
//! inside the private per-user `VSift` directory.
//!
//! # Trust model
//!
//! The record is an optimisation, never an authority. It is read with no-follow
//! opens, rejected when linked, oversized, malformed or from another schema, and
//! any such defect is treated as "unverified": the tools are verified again and
//! the record is rewritten. Writes are serialised by a non-blocking lock; a
//! writer that finds the lock held skips recording rather than waiting. Only
//! passes are recorded, and each entry holds a digest and a time, never a path,
//! so the record reveals nothing about the user's files or media.
//!
//! # Concurrent readers
//!
//! Readers take no lock. A writer replaces the record by renaming a complete
//! new file over it, so a reader sees either the old or the new record, never a
//! mixture. A reader can still catch the replacement itself: the file it opened
//! is unlinked before it checks the link count (the count is then zero), or, on
//! Windows, the name briefly refuses opens while the replaced file is pending
//! deletion. Neither is a defect of the record, so the read is retried a few
//! times, and if a replacement is still in the way it counts as "unverified"
//! like any other unreadable record (issue #136). A record with more than one
//! link is still rejected as unsafe.
//!
//! # Leftover verification workspaces
//!
//! Each verification runs in its own `vsift-tool-verification-<16 hex>`
//! directory inside the state directory and removes it when done. A process
//! killed mid-verification leaves it behind (issue #132), so each preflight,
//! holding the record lock, removes stale ones: see
//! [`FilesystemMediaToolVerificationCache::remove_stale_workspaces`].
//!
//! # What the fingerprint binds
//!
//! A pass is valid only for the exact inputs it was produced with: the
//! canonical paths of both executables and their on-disk identity (size and
//! modification time everywhere; device, inode, mode, owner and status-change
//! time on Unix; creation time and attributes on Windows), the media adapter
//! profile, every field of the reviewed compatibility policy (including the
//! fixture digest), the host isolation, which verifier produced the pass, the
//! verification profile version and the `VSift` version. Executable contents
//! are deliberately not hashed: see [`media_tool_fingerprint`].

use std::{
    ffi::OsStr,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
#[cfg(unix)]
use cap_std::fs::OpenOptionsExt;
use cap_std::fs::{Dir, DirBuilder, File, OpenOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vsift_application::{
    CachedMediaToolVerification, MediaToolFingerprint, MediaToolVerificationCache,
    ReviewedCompatibilityPolicy, VerificationRecord, VerificationRecordSkip,
};

use crate::{
    HostIsolation, MediaProviderConformance,
    file_lock::HeldFileLock,
    media_tool_verification::{WORKSPACE_LOCK_FILE, is_verification_workspace_name},
    private_user_root::{validate_private_root, validate_same_directory_object},
};

/// Version of what a media-tool verification checks.
///
/// Bump it whenever the fixture checks change meaning, so every recorded pass
/// from the previous checks is re-verified.
pub const MEDIA_TOOL_VERIFICATION_PROFILE: u32 = 1;

/// How long a recorded pass is trusted before the tools are verified again.
///
/// The fingerprint cannot see everything a tool depends on (shared libraries,
/// codecs loaded at run time), so a pass also ages out after seven days.
pub const MEDIA_TOOL_VERIFICATION_MAX_AGE_SECONDS: u64 = 7 * 24 * 60 * 60;

/// Largest verification record read or written.
pub const MAX_MEDIA_TOOL_VERIFICATION_RECORD_BYTES: u64 = 4096;

/// Most recorded passes kept; older passes are evicted first.
pub const MAX_MEDIA_TOOL_VERIFICATION_ENTRIES: usize = 8;

/// How long a verification workspace must have been unchanged before a sweep
/// may remove it.
///
/// A verification is bounded by three media deadlines of at most 60 seconds
/// each, so a workspace this old that nobody holds can only be a leftover.
/// The age alone never removes a workspace whose verification still holds its
/// lock (for example one suspended with the machine); it covers the instant
/// between creating a workspace and locking it, and workspaces made before
/// workspaces were locked.
pub const STALE_VERIFICATION_WORKSPACE_AGE_SECONDS: u64 = 60 * 60;

/// Most leftover workspaces one sweep removes, so a preflight stays quick.
pub const MAX_REMOVED_VERIFICATION_WORKSPACES: usize = 8;

const STATE_DIRECTORY: &str = "media-tool-verification";
const RECORD_FILE: &str = "verified-v1.json";
const LOCK_FILE: &str = "verified.lock";
const PENDING_PREFIX: &str = "verified-";
const PENDING_SUFFIX: &str = ".pending";
const PENDING_RANDOM_BYTES: usize = 16;
/// Bounds the stale-pending sweep so a hostile directory cannot stall a write.
const MAX_SWEPT_ENTRIES: usize = 256;
/// Reads attempted when a concurrent writer replaces the record mid-read. Each
/// replacement is complete, so the next attempt normally succeeds at once.
const MAX_RECORD_READ_ATTEMPTS: usize = 4;
const SCHEMA_VERSION: u8 = 1;
const FINGERPRINT_DOMAIN: &[u8] = b"vsift-media-tool-verification";

/// Which verifier produced a pass.
///
/// Binding it into the fingerprint keeps a pass from a host-supplied verifier
/// (for example a test double) from ever satisfying the reviewed fixture check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaToolVerificationAuthority {
    /// The embedded reviewed fixture verifier.
    ReviewedFixture,
    /// A verifier supplied by the embedding host.
    HostSupplied,
}

impl MediaToolVerificationAuthority {
    pub(crate) const fn tag(self) -> &'static [u8] {
        match self {
            Self::ReviewedFixture => b"reviewed_fixture",
            Self::HostSupplied => b"host_supplied",
        }
    }
}

/// Derives the identity a media-tool verification pass is valid for.
///
/// Returns `None` when either executable's identity cannot be read; the
/// preflight then verifies without recording.
///
/// Executable contents are not hashed. A static `FFmpeg` build is roughly
/// 100 MiB, so hashing both tools would add a noticeable cost to every media
/// operation, which is the cost the cache exists to remove. It would also be
/// incomplete, because shared builds load libraries the digest would not cover.
/// Package managers and installers replace files, which changes size,
/// modification time and (on Unix) inode and status-change time; an in-place
/// rewrite that preserves all of them needs write access to the user's chosen
/// executable, which is outside what this functional check defends against.
/// The seven-day age limit bounds any change the fingerprint cannot see.
#[must_use]
pub fn media_tool_fingerprint(
    conformance: &MediaProviderConformance,
    host_isolation: HostIsolation,
    policy: &ReviewedCompatibilityPolicy,
    authority: MediaToolVerificationAuthority,
) -> Option<MediaToolFingerprint> {
    let mut hasher = Sha256::new();
    field(&mut hasher, FINGERPRINT_DOMAIN);
    field(&mut hasher, &MEDIA_TOOL_VERIFICATION_PROFILE.to_le_bytes());
    field(&mut hasher, env!("CARGO_PKG_VERSION").as_bytes());
    field(&mut hasher, authority.tag());
    field(&mut hasher, isolation_tag(host_isolation));
    field(&mut hasher, conformance.profile_version.as_bytes());
    list(&mut hasher, conformance.demuxers);
    list(&mut hasher, conformance.protocols);
    policy_fields(&mut hasher, policy);
    for (role, executable) in [
        (b"ffmpeg".as_slice(), conformance.ffmpeg.path()),
        (b"ffprobe".as_slice(), conformance.ffprobe.path()),
    ] {
        field(&mut hasher, role);
        executable_identity(&mut hasher, executable)?;
    }
    Some(MediaToolFingerprint::from_digest(hasher.finalize().into()))
}

/// Length-prefixes every field so no two input sequences hash alike.
pub(crate) fn field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

pub(crate) fn number(hasher: &mut Sha256, value: u64) {
    field(hasher, &value.to_le_bytes());
}

fn list(hasher: &mut Sha256, values: &[&str]) {
    number(hasher, values.len() as u64);
    for value in values {
        field(hasher, value.as_bytes());
    }
}

pub(crate) const fn isolation_tag(host_isolation: HostIsolation) -> &'static [u8] {
    match host_isolation {
        HostIsolation::ProcessOnly => b"process_only",
        #[cfg(target_os = "linux")]
        HostIsolation::StrictLinux => b"strict_linux",
    }
}

fn policy_fields(hasher: &mut Sha256, policy: &ReviewedCompatibilityPolicy) {
    let ReviewedCompatibilityPolicy {
        fixture,
        expected_ffmpeg_version,
        expected_ffprobe_version,
        stream_limit_bytes,
        audio_file_limit_bytes,
        transcript_file_limit_bytes,
        media_deadline_seconds,
        inference_deadline_seconds,
        audio_sample_rate_hz,
        audio_channels,
    } = policy;
    number(hasher, fixture.bytes());
    field(hasher, &fixture.sha256());
    field(hasher, expected_ffmpeg_version.as_bytes());
    field(hasher, expected_ffprobe_version.as_bytes());
    number(
        hasher,
        u64::try_from(*stream_limit_bytes).unwrap_or(u64::MAX),
    );
    number(hasher, *audio_file_limit_bytes);
    number(hasher, *transcript_file_limit_bytes);
    number(hasher, *media_deadline_seconds);
    number(hasher, *inference_deadline_seconds);
    number(hasher, u64::from(*audio_sample_rate_hz));
    number(hasher, u64::from(*audio_channels));
}

/// Binds a regular file's canonical path and on-disk identity (size,
/// modification time and platform identity), never its contents.
pub(crate) fn executable_identity(hasher: &mut Sha256, path: &Path) -> Option<()> {
    field(hasher, &path_bytes(path.as_os_str()));
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    number(hasher, metadata.len());
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    number(hasher, modified.as_secs());
    number(hasher, u64::from(modified.subsec_nanos()));
    platform_identity(hasher, &metadata);
    Some(())
}

#[cfg(unix)]
fn path_bytes(path: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_bytes().to_vec()
}

#[cfg(windows)]
fn path_bytes(path: &OsStr) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.encode_wide().flat_map(u16::to_le_bytes).collect()
}

#[cfg(unix)]
fn platform_identity(hasher: &mut Sha256, metadata: &fs::Metadata) {
    use std::os::unix::fs::MetadataExt as _;
    // `cap_fs_ext::MetadataExt` also defines `dev` and `ino`; name the std trait.
    number(hasher, std::os::unix::fs::MetadataExt::dev(metadata));
    number(hasher, std::os::unix::fs::MetadataExt::ino(metadata));
    number(hasher, u64::from(metadata.mode()));
    number(hasher, u64::from(metadata.uid()));
    field(hasher, &metadata.ctime().to_le_bytes());
    field(hasher, &metadata.ctime_nsec().to_le_bytes());
}

#[cfg(windows)]
fn platform_identity(hasher: &mut Sha256, metadata: &fs::Metadata) {
    use std::os::windows::fs::MetadataExt as _;
    number(hasher, metadata.creation_time());
    number(hasher, metadata.last_write_time());
    number(hasher, u64::from(metadata.file_attributes()));
}

/// The private media-tool verification state inside the per-user `VSift`
/// directory: the pass record and the parent for verification workspaces.
///
/// A state directory that cannot be opened safely disables recording (every
/// preflight verifies) and verification falls back to the private root itself
/// as its workspace parent; it never makes an operation fail.
#[derive(Debug)]
pub struct FilesystemMediaToolVerificationCache {
    state: Option<Dir>,
    workspace_parent: PathBuf,
}

impl FilesystemMediaToolVerificationCache {
    /// Opens, creating when missing, the state directory inside an already
    /// validated private root.
    pub(crate) fn open(root: &Dir, root_path: &Path) -> Self {
        let state_path = root_path.join(STATE_DIRECTORY);
        match open_state_directory(root, &state_path) {
            Some(state) => Self {
                state: Some(state),
                workspace_parent: state_path,
            },
            None => Self {
                state: None,
                workspace_parent: root_path.to_path_buf(),
            },
        }
    }

    /// The private directory in which a verification creates, uses and removes
    /// its own uniquely named workspace.
    #[must_use]
    pub fn workspace_parent(&self) -> &Path {
        &self.workspace_parent
    }

    /// Reports whether passes can be recorded at all.
    #[must_use]
    pub const fn is_recording(&self) -> bool {
        self.state.is_some()
    }

    /// Removes verification workspaces left behind by verifications that were
    /// killed before they could clean up (issue #132).
    ///
    /// A child of the state directory is removed only when all of these hold:
    ///
    /// - its name is exactly `vsift-tool-verification-` and 16 lowercase hex
    ///   digits, the only name a verification creates;
    /// - it is a real directory, not a symbolic link, junction or file;
    /// - it has been unchanged for at least
    ///   [`STALE_VERIFICATION_WORKSPACE_AGE_SECONDS`] at `now_unix_seconds`
    ///   (a modification time in the future counts as recent);
    /// - no verification holds its lock: the lock file can be locked right now,
    ///   because the process that held it has ended, or the directory has no
    ///   lock file at all.
    ///
    /// A workspace whose lock is held, or whose lock cannot be probed safely, is
    /// kept whatever its age. Removal never follows links, inside or out.
    /// Nothing else in the state directory is examined or touched, and the
    /// private root used when the state directory is unusable is never swept.
    ///
    /// The sweep holds the record lock so two preflights never sweep at once.
    /// It never waits: when the lock is held or the state directory is
    /// unavailable, it is skipped and the next preflight tries again. At most
    /// [`MAX_REMOVED_VERIFICATION_WORKSPACES`] workspaces are removed per call.
    #[must_use]
    pub fn remove_stale_workspaces(&self, now_unix_seconds: u64) -> StaleWorkspaceSweep {
        let Some(state) = self.state.as_ref() else {
            return StaleWorkspaceSweep::Skipped;
        };
        let Ok(held) = acquire_record_lock(state) else {
            return StaleWorkspaceSweep::Skipped;
        };
        let removed = sweep_stale_workspaces(state, now_unix_seconds);
        // The sweep is complete either way; a failed unlock is released on close.
        let _ = held.release();
        StaleWorkspaceSweep::Completed { removed }
    }
}

/// What one sweep for leftover verification workspaces did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StaleWorkspaceSweep {
    /// The state directory was examined and this many workspaces were removed.
    Completed {
        /// Stale workspaces removed.
        removed: usize,
    },
    /// Nothing was examined: the state directory is unavailable or another
    /// process holds its lock. The next preflight sweeps instead.
    Skipped,
}

impl MediaToolVerificationCache for FilesystemMediaToolVerificationCache {
    fn lookup(
        &self,
        fingerprint: &MediaToolFingerprint,
        now_unix_seconds: u64,
    ) -> CachedMediaToolVerification {
        let wanted = hex(fingerprint.digest());
        let verified =
            self.state
                .as_ref()
                .and_then(|state| read_record(state).ok())
                .is_some_and(|record| {
                    record.entries.iter().any(|entry| {
                        entry.fingerprint == wanted && entry.is_fresh_at(now_unix_seconds)
                    })
                });
        if verified {
            CachedMediaToolVerification::Verified
        } else {
            CachedMediaToolVerification::Unverified
        }
    }

    fn record_verified(
        &self,
        fingerprint: &MediaToolFingerprint,
        now_unix_seconds: u64,
    ) -> VerificationRecord {
        let Some(state) = self.state.as_ref() else {
            return VerificationRecord::Skipped(VerificationRecordSkip::Unavailable);
        };
        match record_pass(state, &hex(fingerprint.digest()), now_unix_seconds) {
            Ok(()) => VerificationRecord::Recorded,
            Err(failure) => VerificationRecord::Skipped(failure.skip()),
        }
    }
}

fn open_state_directory(root: &Dir, state_path: &Path) -> Option<Dir> {
    #[allow(unused_mut, reason = "Unix configures the creation mode")]
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use cap_std::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match root.create_dir_with(STATE_DIRECTORY, &builder) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return None,
    }
    let state = root.open_dir_nofollow(STATE_DIRECTORY).ok()?;
    let path_metadata = fs::symlink_metadata(state_path).ok()?;
    validate_same_directory_object(&path_metadata, &state).ok()?;
    validate_private_root(state_path, &state).ok()?;
    Some(state)
}

/// The stored record. Strict: unknown or missing fields reject the whole file.
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredRecord {
    schema_version: u8,
    entries: Vec<StoredPass>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredPass {
    fingerprint: String,
    verified_at_unix_seconds: u64,
}

impl StoredPass {
    /// A pass from the future (the clock moved back) is not trusted either.
    fn is_fresh_at(&self, now_unix_seconds: u64) -> bool {
        now_unix_seconds
            .checked_sub(self.verified_at_unix_seconds)
            .is_some_and(|age| age < MEDIA_TOOL_VERIFICATION_MAX_AGE_SECONDS)
    }
}

impl StoredRecord {
    fn is_valid(&self) -> bool {
        self.schema_version == SCHEMA_VERSION
            && self.entries.len() <= MAX_MEDIA_TOOL_VERIFICATION_ENTRIES
            && self
                .entries
                .iter()
                .all(|entry| is_digest_hex(&entry.fingerprint))
            && self.entries.iter().enumerate().all(|(index, entry)| {
                self.entries
                    .iter()
                    .skip(index + 1)
                    .all(|later| later.fingerprint != entry.fingerprint)
            })
    }
}

/// Why the record could not be read; every variant means "unverified".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordDefect {
    /// There is no record.
    Missing,
    /// A concurrent writer was replacing the record on every attempt. The
    /// replacement is complete, so this is transient, never a torn record.
    Replaced,
    /// The name is a link, not a regular file, or a file with other links.
    Unsafe,
    /// The record could not be read.
    Unreadable,
    /// The content is oversized, malformed or from another schema.
    Invalid,
}

/// Win32 `ERROR_ACCESS_DENIED`: how opening a name whose previous file is still
/// pending deletion (`STATUS_DELETE_PENDING`) surfaces during a replacement.
#[cfg(windows)]
const ERROR_ACCESS_DENIED: i32 = 5;
/// Win32 `ERROR_SHARING_VIOLATION`.
#[cfg(windows)]
const ERROR_SHARING_VIOLATION: i32 = 32;
/// Win32 `ERROR_DELETE_PENDING`.
#[cfg(windows)]
const ERROR_DELETE_PENDING: i32 = 303;

fn read_record(state: &Dir) -> Result<StoredRecord, RecordDefect> {
    read_record_with(|| {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        state.open_with(RECORD_FILE, &options)
    })
}

/// Reads through `open`, retrying while a concurrent replacement is in the way.
fn read_record_with(
    mut open: impl FnMut() -> io::Result<File>,
) -> Result<StoredRecord, RecordDefect> {
    let mut outcome = Err(RecordDefect::Replaced);
    for attempt in 0..MAX_RECORD_READ_ATTEMPTS {
        if attempt > 0 {
            // Let the writer finish its rename; no sleep, which could stall a
            // preflight for no benefit.
            std::thread::yield_now();
        }
        outcome = open()
            .map_err(|error| classify_open_error(&error))
            .and_then(read_opened_record);
        if !matches!(outcome, Err(RecordDefect::Replaced)) {
            break;
        }
    }
    outcome
}

/// Classifies a failure to open the record without following links.
///
/// Anything but a missing file or a known transient replacement stays unsafe,
/// as before: a link, a directory or an unexpected error all mean "unverified".
fn classify_open_error(error: &io::Error) -> RecordDefect {
    if error.kind() == io::ErrorKind::NotFound {
        RecordDefect::Missing
    } else if is_transient_replacement(error) {
        RecordDefect::Replaced
    } else {
        RecordDefect::Unsafe
    }
}

/// Windows refuses opens of a name while the file it replaced is pending
/// deletion (issue #136); the refusal ends when the replacement completes.
#[cfg(windows)]
fn is_transient_replacement(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_DELETE_PENDING)
    )
}

/// A POSIX rename swaps the name atomically, so an open never sees it midway.
#[cfg(not(windows))]
const fn is_transient_replacement(_error: &io::Error) -> bool {
    false
}

/// Classifies the link count of an opened regular file.
///
/// Zero means the file was replaced (unlinked) after it was opened: its
/// content is a complete earlier record, but a newer one exists, so the read is
/// retried. More than one link means another name reaches the same file, which
/// a writer never creates, so it is unsafe.
const fn check_link_count(links: u64) -> Result<(), RecordDefect> {
    match links {
        0 => Err(RecordDefect::Replaced),
        1 => Ok(()),
        _ => Err(RecordDefect::Unsafe),
    }
}

fn read_opened_record(file: File) -> Result<StoredRecord, RecordDefect> {
    let metadata = file.metadata().map_err(|_| RecordDefect::Unreadable)?;
    if !metadata.is_file() {
        return Err(RecordDefect::Unsafe);
    }
    check_link_count(metadata.nlink())?;
    if metadata.len() > MAX_MEDIA_TOOL_VERIFICATION_RECORD_BYTES {
        return Err(RecordDefect::Invalid);
    }
    let mut bytes = Vec::new();
    file.take(MAX_MEDIA_TOOL_VERIFICATION_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| RecordDefect::Unreadable)?;
    if bytes.len() as u64 > MAX_MEDIA_TOOL_VERIFICATION_RECORD_BYTES {
        return Err(RecordDefect::Invalid);
    }
    let record: StoredRecord = serde_json::from_slice(&bytes).map_err(|_| RecordDefect::Invalid)?;
    if record.is_valid() {
        Ok(record)
    } else {
        Err(RecordDefect::Invalid)
    }
}

/// Which step of recording a pass failed.
///
/// Only [`Self::Busy`] is a designed outcome of concurrent writers; every other
/// variant means the storage could not be used, and all of them only cost a
/// later re-verification. Tests name the step; the port reports the skip.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordWriteFailure {
    /// Another writer holds the record lock.
    Busy,
    /// The record lock could not be taken for a reason other than another
    /// holder; the cause names the sub-step and the operating-system error.
    Lock(LockFailure),
    /// The record could not be encoded within its size bound.
    Encode,
    /// No randomness for the pending file name.
    Randomness,
    /// The pending file could not be created or written.
    Pending,
    /// The pending file could not replace the record.
    Replace,
}

impl RecordWriteFailure {
    const fn skip(self) -> VerificationRecordSkip {
        match self {
            Self::Busy => VerificationRecordSkip::Busy,
            Self::Lock(_) | Self::Encode | Self::Randomness | Self::Pending | Self::Replace => {
                VerificationRecordSkip::Unavailable
            }
        }
    }
}

/// An operating-system error reduced to what is safe to report: its kind and
/// raw code, never a path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IoCause {
    kind: io::ErrorKind,
    os_code: Option<i32>,
}

impl IoCause {
    fn of(error: &io::Error) -> Self {
        Self {
            kind: error.kind(),
            os_code: error.raw_os_error(),
        }
    }
}

/// Why the record lock could not be taken (issue #136).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LockFailure {
    /// Opening or creating the lock file without following links failed.
    Open(IoCause),
    /// Reading the opened lock file's metadata failed.
    Inspect(IoCause),
    /// The lock file is not a regular file with exactly one link. Linked or
    /// foreign lock files are never used and never reported as busy.
    Shape {
        /// Whether it is a regular file.
        regular_file: bool,
        /// Its link count.
        links: u64,
    },
    /// The operating system refused the lock for a reason other than another
    /// holder.
    Acquire(IoCause),
}

impl LockFailure {
    /// Whether a bounded retry may resolve the failure.
    ///
    /// Retried: an interrupted open, or the lock file being created by another
    /// writer at the same moment (not found, already exists); a just-created
    /// regular file that does not yet report its link; and any failure to
    /// inspect an opened lock file or to lock one proven to be a single-link
    /// regular file (an interrupted call, or a kernel short of lock records).
    /// A retry reopens and rechecks everything, so it can never make an unsafe
    /// lock file usable. Every other open failure (a link, a directory, no
    /// permission) and every other shape is final.
    const fn is_transient(self) -> bool {
        match self {
            Self::Open(cause) => matches!(
                cause.kind,
                io::ErrorKind::Interrupted | io::ErrorKind::NotFound | io::ErrorKind::AlreadyExists
            ),
            Self::Inspect(_) | Self::Acquire(_) => true,
            Self::Shape {
                regular_file,
                links,
            } => regular_file && links == 0,
        }
    }
}

/// Attempts to take the record lock before a transient failure is final.
const MAX_LOCK_ATTEMPTS: usize = 4;

/// Takes the record lock without waiting.
fn acquire_record_lock(state: &Dir) -> Result<HeldFileLock, RecordWriteFailure> {
    acquire_record_lock_with(|| open_lock(state), HeldFileLock::try_exclusive)
}

/// Takes the record lock through `open` and `lock`, retrying transient
/// failures a bounded number of times without sleeping. A holder is reported
/// as [`RecordWriteFailure::Busy`] at once and never retried.
fn acquire_record_lock_with(
    mut open: impl FnMut() -> Result<fs::File, LockFailure>,
    mut lock: impl FnMut(fs::File) -> Result<HeldFileLock, fs::TryLockError>,
) -> Result<HeldFileLock, RecordWriteFailure> {
    let mut attempt = 1;
    loop {
        // `None` is another holder; `Some` is why the lock could not be taken.
        let attempted = match open() {
            Err(cause) => Err(Some(cause)),
            Ok(file) => lock(file).map_err(|error| match error {
                fs::TryLockError::WouldBlock => None,
                // `WouldBlock` is how std reports a holder; the same kind in
                // an error still means a holder, not a storage problem.
                fs::TryLockError::Error(error) if error.kind() == io::ErrorKind::WouldBlock => None,
                fs::TryLockError::Error(error) => Some(LockFailure::Acquire(IoCause::of(&error))),
            }),
        };
        match attempted {
            Ok(held) => return Ok(held),
            Err(None) => return Err(RecordWriteFailure::Busy),
            Err(Some(cause)) if cause.is_transient() && attempt < MAX_LOCK_ATTEMPTS => {
                attempt += 1;
                std::thread::yield_now();
            }
            Err(Some(cause)) => return Err(RecordWriteFailure::Lock(cause)),
        }
    }
}

fn record_pass(
    state: &Dir,
    fingerprint: &str,
    now_unix_seconds: u64,
) -> Result<(), RecordWriteFailure> {
    let held = acquire_record_lock(state)?;
    // Only a lock holder creates pending files, so any found now are debris
    // from a writer that was killed mid-write.
    remove_stale_pending(state);
    // A defective record is replaced, not repaired: its passes are not trusted.
    let mut entries = read_record(state).map_or_else(|_| Vec::new(), |record| record.entries);
    entries.retain(|entry| entry.fingerprint != fingerprint && entry.is_fresh_at(now_unix_seconds));
    entries.insert(
        0,
        StoredPass {
            fingerprint: fingerprint.to_owned(),
            verified_at_unix_seconds: now_unix_seconds,
        },
    );
    entries.truncate(MAX_MEDIA_TOOL_VERIFICATION_ENTRIES);
    let written = write_record(
        state,
        &StoredRecord {
            schema_version: SCHEMA_VERSION,
            entries,
        },
        File::sync_all,
    );
    // The record is complete either way; a failed unlock is released on close.
    let _ = held.release();
    written
}

fn open_lock(state: &Dir) -> Result<fs::File, LockFailure> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .follow(FollowSymlinks::No);
    #[cfg(unix)]
    options.mode(0o600);
    let lock = state
        .open_with(LOCK_FILE, &options)
        .map_err(|error| LockFailure::Open(IoCause::of(&error)))?;
    let metadata = lock
        .metadata()
        .map_err(|error| LockFailure::Inspect(IoCause::of(&error)))?;
    check_lock_shape(metadata.is_file(), metadata.nlink())?;
    Ok(lock.into_std())
}

/// Accepts only a regular file with exactly one link as the record lock.
const fn check_lock_shape(regular_file: bool, links: u64) -> Result<(), LockFailure> {
    if regular_file && links == 1 {
        Ok(())
    } else {
        Err(LockFailure::Shape {
            regular_file,
            links,
        })
    }
}

/// Writes `record` to a fresh pending file and renames it over the record.
///
/// `flush` asks the file system to make the pending file durable before the
/// rename. It is best effort: a failed flush does not abandon the write. The
/// record is an optimisation whose every defect reads as "unverified", so a
/// crash that loses or tears an unflushed record only costs one re-verification,
/// whereas giving up guarantees one. A flush can fail where the write did not:
/// on macOS it is `F_FULLFSYNC`, which asks the disk itself to flush and which
/// not every volume or virtual disk honours (issue #136).
fn write_record(
    state: &Dir,
    record: &StoredRecord,
    flush: impl FnOnce(&File) -> io::Result<()>,
) -> Result<(), RecordWriteFailure> {
    let bytes = serde_json::to_vec(record).map_err(|_| RecordWriteFailure::Encode)?;
    if bytes.len() as u64 > MAX_MEDIA_TOOL_VERIFICATION_RECORD_BYTES {
        return Err(RecordWriteFailure::Encode);
    }
    let mut random = [0_u8; PENDING_RANDOM_BYTES];
    getrandom::fill(&mut random).map_err(|_| RecordWriteFailure::Randomness)?;
    let pending = format!("{PENDING_PREFIX}{}{PENDING_SUFFIX}", hex(&random));
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = state
        .open_with(&pending, &options)
        .map_err(|_| RecordWriteFailure::Pending)?;
    if file.write_all(&bytes).is_err() {
        drop(file);
        let _ = state.remove_file(&pending);
        return Err(RecordWriteFailure::Pending);
    }
    let _ = flush(&file);
    drop(file);
    // Rename replaces the directory entry itself, so a planted link or a
    // hard-linked record is swapped out rather than written through.
    if state.rename(&pending, state, RECORD_FILE).is_err() {
        let _ = state.remove_file(&pending);
        return Err(RecordWriteFailure::Replace);
    }
    Ok(())
}

/// Removes stale verification workspaces from the locked state directory and
/// returns how many were removed; see
/// [`FilesystemMediaToolVerificationCache::remove_stale_workspaces`].
fn sweep_stale_workspaces(state: &Dir, now_unix_seconds: u64) -> usize {
    let Ok(entries) = state.entries() else {
        return 0;
    };
    // Names are collected first so removal never disturbs the listing.
    let candidates = entries
        .take(MAX_SWEPT_ENTRIES)
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_verification_workspace_name(name))
        .collect::<Vec<_>>();
    let mut removed = 0;
    for name in candidates {
        if removed == MAX_REMOVED_VERIFICATION_WORKSPACES {
            break;
        }
        if workspace_disposition(state, &name, now_unix_seconds) == WorkspaceDisposition::Stale
            && state.remove_dir_all(&name).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// Whether a positively named workspace may be removed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceDisposition {
    /// Unchanged for the stale age and held by no verification.
    Stale,
    /// Changed too recently, or its modification time is unknown or ahead.
    Recent,
    /// A verification holds its lock.
    InUse,
    /// Not a real directory, or its lock cannot be probed safely; left alone.
    Untouchable,
}

fn workspace_disposition(state: &Dir, name: &str, now_unix_seconds: u64) -> WorkspaceDisposition {
    // Not following links: a symbolic link or junction is not a directory here.
    let Ok(metadata) = state.symlink_metadata(name) else {
        return WorkspaceDisposition::Untouchable;
    };
    if !metadata.is_dir() {
        return WorkspaceDisposition::Untouchable;
    }
    let old_enough = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.into_std().duration_since(UNIX_EPOCH).ok())
        .and_then(|modified| now_unix_seconds.checked_sub(modified.as_secs()))
        .is_some_and(|age| age >= STALE_VERIFICATION_WORKSPACE_AGE_SECONDS);
    if !old_enough {
        return WorkspaceDisposition::Recent;
    }
    let Ok(workspace) = state.open_dir_nofollow(name) else {
        return WorkspaceDisposition::Untouchable;
    };
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let lock = match workspace.open_with(WORKSPACE_LOCK_FILE, &options) {
        Ok(lock) => lock,
        // Made before workspaces were locked, or its verification was killed
        // between creating it and locking it.
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return WorkspaceDisposition::Stale;
        }
        Err(_) => return WorkspaceDisposition::Untouchable,
    };
    if !lock.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return WorkspaceDisposition::Untouchable;
    }
    // The probe is released and every handle closed before removal, because
    // Windows cannot remove a directory with open handles. No verification
    // adopts an existing workspace, so none can claim it in between.
    match HeldFileLock::try_exclusive(lock.into_std()) {
        Ok(probe) => {
            let _ = probe.release();
            WorkspaceDisposition::Stale
        }
        Err(fs::TryLockError::WouldBlock) => WorkspaceDisposition::InUse,
        Err(fs::TryLockError::Error(_)) => WorkspaceDisposition::Untouchable,
    }
}

fn remove_stale_pending(state: &Dir) {
    let Ok(entries) = state.entries() else {
        return;
    };
    for entry in entries.take(MAX_SWEPT_ENTRIES).flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let is_file_or_link = entry.file_type().is_ok_and(|file_type| !file_type.is_dir());
        if is_file_or_link && is_pending_name(name) {
            // `remove_file` removes a link itself and never follows it.
            let _ = state.remove_file(name);
        }
    }
}

fn is_pending_name(name: &str) -> bool {
    name.strip_prefix(PENDING_PREFIX)
        .and_then(|rest| rest.strip_suffix(PENDING_SUFFIX))
        .is_some_and(|random| random.len() == PENDING_RANDOM_BYTES * 2 && is_lowercase_hex(random))
}

fn is_digest_hex(value: &str) -> bool {
    value.len() == 64 && is_lowercase_hex(value)
}

fn is_lowercase_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs,
        path::{Path, PathBuf},
        sync::{Arc, Barrier},
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{Dir, File, OpenOptions};

    use vsift_application::{
        CachedMediaToolVerification, MediaToolFingerprint, MediaToolVerificationCache,
        VerificationRecord, VerificationRecordSkip,
    };

    use super::{
        FilesystemMediaToolVerificationCache, IoCause, LOCK_FILE, LockFailure, MAX_LOCK_ATTEMPTS,
        MAX_MEDIA_TOOL_VERIFICATION_ENTRIES, MAX_MEDIA_TOOL_VERIFICATION_RECORD_BYTES,
        MAX_RECORD_READ_ATTEMPTS, MAX_REMOVED_VERIFICATION_WORKSPACES,
        MEDIA_TOOL_VERIFICATION_MAX_AGE_SECONDS, MediaToolVerificationAuthority, RECORD_FILE,
        RecordDefect, RecordWriteFailure, SCHEMA_VERSION, STALE_VERIFICATION_WORKSPACE_AGE_SECONDS,
        STATE_DIRECTORY, StaleWorkspaceSweep, StoredPass, StoredRecord, acquire_record_lock_with,
        check_link_count, check_lock_shape, classify_open_error, hex, is_pending_name,
        media_tool_fingerprint, read_record, read_record_with, record_pass, write_record,
    };
    use crate::media_tool_verification::{VerificationWorkspace, WORKSPACE_LOCK_FILE};
    use crate::{
        HostIsolation, MediaProviderConformance, TrustedExecutable, UserDependencyConfigStore,
        file_lock::HeldFileLock, reviewed_compatibility_policy,
    };

    type TestResult = Result<(), Box<dyn Error>>;

    const NOW: u64 = 1_800_000_000;

    /// A uniquely named temporary parent removed on drop.
    struct Parent(PathBuf);

    impl Parent {
        fn new() -> Result<Self, Box<dyn Error>> {
            let mut random = [0_u8; 16];
            getrandom::fill(&mut random).map_err(|_| std::io::Error::other("no randomness"))?;
            let path = std::env::temp_dir().join(format!("vsift-tool-cache-test-{}", hex(&random)));
            fs::create_dir(&path)?;
            Ok(Self(path))
        }

        fn config(&self) -> PathBuf {
            self.0.join("config")
        }

        fn state(&self) -> PathBuf {
            self.config().join(STATE_DIRECTORY)
        }

        fn record(&self) -> PathBuf {
            self.state().join(RECORD_FILE)
        }

        fn cache(&self) -> Result<FilesystemMediaToolVerificationCache, Box<dyn Error>> {
            Ok(UserDependencyConfigStore::at(self.config())?.media_tool_verification_state()?)
        }

        fn tools(&self) -> Result<MediaProviderConformance, Box<dyn Error>> {
            let ffmpeg = self.0.join("ffmpeg-tool");
            let ffprobe = self.0.join("ffprobe-tool");
            fs::write(&ffmpeg, b"ffmpeg stand-in")?;
            fs::write(&ffprobe, b"ffprobe stand-in")?;
            Ok(MediaProviderConformance::r0(
                TrustedExecutable::explicit(&ffmpeg)?,
                TrustedExecutable::explicit(&ffprobe)?,
            ))
        }
    }

    impl Drop for Parent {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fingerprint(
        tools: &MediaProviderConformance,
        authority: MediaToolVerificationAuthority,
    ) -> Result<MediaToolFingerprint, Box<dyn Error>> {
        media_tool_fingerprint(
            tools,
            HostIsolation::ProcessOnly,
            &reviewed_compatibility_policy()?,
            authority,
        )
        .ok_or_else(|| "fingerprint unavailable".into())
    }

    const fn digest(byte: u8) -> MediaToolFingerprint {
        MediaToolFingerprint::from_digest([byte; 32])
    }

    fn valid_record_json(fingerprint: &MediaToolFingerprint, at: u64) -> String {
        format!(
            r#"{{"schema_version":1,"entries":[{{"fingerprint":"{}","verified_at_unix_seconds":{at}}}]}}"#,
            hex(fingerprint.digest())
        )
    }

    fn state_entries(parent: &Parent) -> Result<Vec<String>, Box<dyn Error>> {
        let mut names = fs::read_dir(parent.state())?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<Result<Vec<_>, _>>()?;
        names.sort();
        Ok(names)
    }

    fn cache_state(
        cache: &FilesystemMediaToolVerificationCache,
    ) -> Result<cap_std::fs::Dir, Box<dyn Error>> {
        Ok(cache
            .state
            .as_ref()
            .ok_or("state unavailable")?
            .try_clone()?)
    }

    #[test]
    fn fingerprint_is_stable_and_bound_to_identity_policy_and_authority() -> TestResult {
        let parent = Parent::new()?;
        let tools = parent.tools()?;
        let fixture = MediaToolVerificationAuthority::ReviewedFixture;
        let original = fingerprint(&tools, fixture)?;

        assert_eq!(fingerprint(&tools, fixture)?, original);
        assert_ne!(
            fingerprint(&tools, MediaToolVerificationAuthority::HostSupplied)?,
            original
        );
        let mut policy = reviewed_compatibility_policy()?;
        policy.audio_channels += 1;
        assert_ne!(
            media_tool_fingerprint(&tools, HostIsolation::ProcessOnly, &policy, fixture),
            Some(original)
        );
        let swapped = MediaProviderConformance::r0(tools.ffprobe.clone(), tools.ffmpeg.clone());
        assert_ne!(fingerprint(&swapped, fixture)?, original);

        // A replaced executable of a different size is a different identity.
        fs::write(tools.ffprobe.path(), b"a different ffprobe build")?;
        let resized = fingerprint(&tools, fixture)?;
        assert_ne!(resized, original);

        // So is one with the same size but another modification time.
        fs::File::options()
            .write(true)
            .open(tools.ffprobe.path())?
            .set_modified(SystemTime::now() - Duration::from_secs(3600))?;
        assert_ne!(fingerprint(&tools, fixture)?, resized);

        fs::remove_file(tools.ffmpeg.path())?;
        assert_eq!(
            media_tool_fingerprint(
                &tools,
                HostIsolation::ProcessOnly,
                &reviewed_compatibility_policy()?,
                fixture
            ),
            None
        );
        Ok(())
    }

    #[test]
    fn a_recorded_pass_is_found_until_it_ages_out() -> TestResult {
        let parent = Parent::new()?;
        let cache = parent.cache()?;
        assert!(cache.is_recording());
        assert_eq!(cache.workspace_parent(), parent.state());
        assert_eq!(
            cache.lookup(&digest(1), NOW),
            CachedMediaToolVerification::Unverified
        );

        assert_eq!(
            cache.record_verified(&digest(1), NOW),
            VerificationRecord::Recorded
        );

        let reopened = parent.cache()?;
        let limit = NOW + MEDIA_TOOL_VERIFICATION_MAX_AGE_SECONDS;
        for (fingerprint, now, expected) in [
            (digest(1), NOW, CachedMediaToolVerification::Verified),
            (digest(1), limit - 1, CachedMediaToolVerification::Verified),
            (digest(1), limit, CachedMediaToolVerification::Unverified),
            (digest(1), NOW - 1, CachedMediaToolVerification::Unverified),
            (digest(2), NOW, CachedMediaToolVerification::Unverified),
        ] {
            assert_eq!(reopened.lookup(&fingerprint, now), expected, "{now}");
        }
        // Only a digest and a time are stored: never a path.
        let record = fs::read_to_string(parent.record())?;
        assert!(!record.contains("config"), "{record}");
        assert_eq!(
            state_entries(&parent)?,
            [RECORD_FILE.to_owned(), LOCK_FILE.to_owned()]
        );
        Ok(())
    }

    #[test]
    fn defective_records_are_unverified_and_replaced_by_the_next_pass() -> TestResult {
        let parent = Parent::new()?;
        let cache = parent.cache()?;
        let known = digest(0xab);
        let known_hex = hex(known.digest());
        let valid = valid_record_json(&known, NOW);
        let oversized = format!(
            "{valid}{}",
            " ".repeat(usize::try_from(MAX_MEDIA_TOOL_VERIFICATION_RECORD_BYTES)?)
        );
        for defective in [
            String::from("not json"),
            oversized,
            valid.replace("\"schema_version\":1", "\"schema_version\":2"),
            valid.replace("}]}", ",\"path\":\"elsewhere\"}]}"),
            valid.replace(&known_hex, &known_hex.to_uppercase()),
            valid.replace(&known_hex, "00"),
            format!(
                r#"{{"schema_version":1,"entries":[{{"fingerprint":"{known_hex}","verified_at_unix_seconds":{NOW}}},{{"fingerprint":"{known_hex}","verified_at_unix_seconds":{NOW}}}]}}"#
            ),
        ] {
            fs::write(parent.record(), &defective)?;
            assert_eq!(
                cache.lookup(&known, NOW),
                CachedMediaToolVerification::Unverified,
                "{defective}"
            );
            assert_eq!(
                cache.record_verified(&digest(8), NOW),
                VerificationRecord::Recorded
            );
            // The defective record's passes were discarded, not carried over.
            assert_eq!(
                cache.lookup(&known, NOW),
                CachedMediaToolVerification::Unverified
            );
            assert_eq!(
                cache.lookup(&digest(8), NOW),
                CachedMediaToolVerification::Verified
            );
        }
        Ok(())
    }

    #[test]
    fn a_hard_linked_record_is_not_trusted_and_is_replaced_not_written_through() -> TestResult {
        let parent = Parent::new()?;
        let cache = parent.cache()?;
        let outside = parent.0.join("outside.json");
        let planted = valid_record_json(&digest(3), NOW);
        fs::write(&outside, &planted)?;
        fs::hard_link(&outside, parent.record())?;

        assert_eq!(
            read_record(&cache_state(&cache)?).err(),
            Some(RecordDefect::Unsafe)
        );
        assert_eq!(
            cache.lookup(&digest(3), NOW),
            CachedMediaToolVerification::Unverified
        );
        assert_eq!(
            cache.record_verified(&digest(4), NOW),
            VerificationRecord::Recorded
        );

        assert_eq!(fs::read_to_string(&outside)?, planted);
        assert_eq!(
            cache.lookup(&digest(4), NOW),
            CachedMediaToolVerification::Verified
        );
        assert_eq!(
            cache.lookup(&digest(3), NOW),
            CachedMediaToolVerification::Unverified
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_record_is_not_followed_and_is_replaced() -> TestResult {
        let parent = Parent::new()?;
        let cache = parent.cache()?;
        let outside = parent.0.join("outside.json");
        let planted = valid_record_json(&digest(5), NOW);
        fs::write(&outside, &planted)?;
        std::os::unix::fs::symlink(&outside, parent.record())?;

        assert_eq!(
            cache.lookup(&digest(5), NOW),
            CachedMediaToolVerification::Unverified
        );
        assert_eq!(
            cache.record_verified(&digest(6), NOW),
            VerificationRecord::Recorded
        );

        assert!(
            !fs::symlink_metadata(parent.record())?
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&outside)?, planted);
        assert_eq!(
            cache.lookup(&digest(6), NOW),
            CachedMediaToolVerification::Verified
        );
        Ok(())
    }

    #[test]
    fn a_held_lock_skips_recording_immediately() -> TestResult {
        let parent = Parent::new()?;
        let cache = parent.cache()?;
        assert_eq!(
            cache.record_verified(&digest(1), NOW),
            VerificationRecord::Recorded
        );
        let lock_file = fs::File::options()
            .read(true)
            .write(true)
            .open(parent.state().join(LOCK_FILE))?;
        let held = HeldFileLock::try_exclusive(lock_file)
            .map_err(|_| std::io::Error::other("lock unavailable"))?;

        assert_eq!(
            cache.record_verified(&digest(2), NOW),
            VerificationRecord::Skipped(VerificationRecordSkip::Busy)
        );
        // Reads never wait for the lock.
        assert_eq!(
            cache.lookup(&digest(1), NOW),
            CachedMediaToolVerification::Verified
        );
        held.release()?;
        assert_eq!(
            cache.lookup(&digest(2), NOW),
            CachedMediaToolVerification::Unverified
        );
        Ok(())
    }

    /// Readers take no lock, so they race writers' renames. The contract is
    /// that a reader never sees a torn record: every read is a complete valid
    /// record, no record, or a replacement still in the way after the bounded
    /// retries, which reads as "unverified" (issue #136). Content defects and
    /// unsafe files never appear. Writers only ever skip because another
    /// writer holds the lock; the failing step is named if one does not.
    #[test]
    fn concurrent_writers_and_readers_never_see_a_torn_record() -> TestResult {
        const WRITERS: u8 = 8;
        const ROUNDS: u8 = 20;
        let parent = Parent::new()?;
        let _ = parent.cache()?;
        let barrier = Arc::new(Barrier::new(usize::from(WRITERS)));
        let handles = (0..WRITERS)
            .map(|writer| {
                let config = parent.config();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || -> Result<usize, String> {
                    let cache = UserDependencyConfigStore::at(config)
                        .and_then(|store| store.media_tool_verification_state())
                        .map_err(|error| error.to_string())?;
                    let state = cache.state.as_ref().ok_or("state unavailable")?;
                    barrier.wait();
                    let mut recorded = 0;
                    for round in 0..ROUNDS {
                        let fingerprint = hex(digest(writer * ROUNDS + round).digest());
                        match record_pass(state, &fingerprint, NOW) {
                            Ok(()) => recorded += 1,
                            Err(RecordWriteFailure::Busy) => {}
                            Err(failure) => {
                                return Err(format!("writer failed at step {failure:?}"));
                            }
                        }
                        match read_record(state) {
                            Ok(_) | Err(RecordDefect::Missing | RecordDefect::Replaced) => {}
                            Err(defect) => return Err(format!("torn record: {defect:?}")),
                        }
                    }
                    Ok(recorded)
                })
            })
            .collect::<Vec<_>>();
        let mut recorded = 0;
        for handle in handles {
            recorded += handle
                .join()
                .map_err(|_| std::io::Error::other("writer panicked"))??;
        }

        assert!(recorded > 0);
        let final_record = read_record(&cache_state(&parent.cache()?)?)
            .map_err(|defect| std::io::Error::other(format!("{defect:?}")))?;
        assert!(!final_record.entries.is_empty());
        assert!(final_record.entries.len() <= MAX_MEDIA_TOOL_VERIFICATION_ENTRIES);
        assert!(
            state_entries(&parent)?
                .iter()
                .all(|name| !is_pending_name(name))
        );
        Ok(())
    }

    #[test]
    fn the_newest_passes_are_kept_and_stale_pending_debris_is_removed() -> TestResult {
        let parent = Parent::new()?;
        let cache = parent.cache()?;
        let debris = parent
            .state()
            .join(format!("verified-{}.pending", "ab".repeat(16)));
        let lookalike = parent.state().join("verified-not-ours.pending");
        fs::write(&debris, b"half written")?;
        fs::write(&lookalike, b"not a VSift pending name")?;

        let total = u8::try_from(MAX_MEDIA_TOOL_VERIFICATION_ENTRIES)? + 2;
        for index in 0..total {
            assert_eq!(
                cache.record_verified(&digest(index), NOW + u64::from(index)),
                VerificationRecord::Recorded
            );
        }

        let now = NOW + u64::from(total);
        for index in 0..total {
            let expected = if index < 2 {
                CachedMediaToolVerification::Unverified
            } else {
                CachedMediaToolVerification::Verified
            };
            assert_eq!(cache.lookup(&digest(index), now), expected, "{index}");
        }
        assert!(!debris.exists());
        assert!(lookalike.exists());
        Ok(())
    }

    #[test]
    fn an_unusable_state_directory_disables_recording_only() -> TestResult {
        let parent = Parent::new()?;
        let _ = parent.cache()?;
        fs::remove_dir_all(parent.state())?;
        fs::write(parent.state(), b"a file where the state directory belongs")?;

        let cache = parent.cache()?;

        assert!(!cache.is_recording());
        assert_eq!(cache.workspace_parent(), parent.config());
        assert_eq!(
            cache.record_verified(&digest(1), NOW),
            VerificationRecord::Skipped(VerificationRecordSkip::Unavailable)
        );
        assert_eq!(
            cache.lookup(&digest(1), NOW),
            CachedMediaToolVerification::Unverified
        );
        // The fallback workspace parent is the private root itself, which is
        // never swept.
        assert_eq!(
            cache.remove_stale_workspaces(NOW),
            StaleWorkspaceSweep::Skipped
        );
        Ok(())
    }

    #[test]
    fn pending_names_are_recognised_exactly() {
        assert!(is_pending_name(&format!(
            "verified-{}.pending",
            "0f".repeat(16)
        )));
        for name in [
            String::from("verified-v1.json"),
            String::from("verified.lock"),
            String::from("verified-0f.pending"),
            format!("verified-{}.pending", "0F".repeat(16)),
            format!("other-{}.pending", "0f".repeat(16)),
        ] {
            assert!(!is_pending_name(&name), "{name}");
        }
    }

    fn open_record(state: &Dir) -> std::io::Result<File> {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        state.open_with(RECORD_FILE, &options)
    }

    fn fingerprints(record: &StoredRecord) -> Vec<String> {
        record
            .entries
            .iter()
            .map(|entry| entry.fingerprint.clone())
            .collect()
    }

    /// Issue #136, deterministically: a reader that opened the record before a
    /// writer renamed a new one over it holds an unlinked file. That is a
    /// replacement to retry, not an unsafe record.
    #[test]
    fn a_record_replaced_after_it_was_opened_is_reread_not_rejected() -> TestResult {
        let parent = Parent::new()?;
        let cache = parent.cache()?;
        let state = cache_state(&cache)?;
        assert_eq!(
            cache.record_verified(&digest(1), NOW),
            VerificationRecord::Recorded
        );
        let replaced = open_record(&state)?;
        let reread_later = open_record(&state)?;

        assert_eq!(
            cache.record_verified(&digest(2), NOW),
            VerificationRecord::Recorded
        );

        // The old handle alone reports the replacement.
        assert_eq!(
            read_record_with(|| reread_later.try_clone()).err(),
            Some(RecordDefect::Replaced)
        );
        // With the retry, the next open reads the complete new record.
        let mut handles = vec![replaced].into_iter();
        let mut opens = 0;
        let record = read_record_with(|| {
            opens += 1;
            handles.next().map_or_else(|| open_record(&state), Ok)
        })
        .map_err(|defect| std::io::Error::other(format!("{defect:?}")))?;
        assert_eq!(opens, 2);
        assert_eq!(
            fingerprints(&record),
            [hex(digest(2).digest()), hex(digest(1).digest())]
        );
        assert_eq!(
            cache.lookup(&digest(2), NOW),
            CachedMediaToolVerification::Verified
        );
        Ok(())
    }

    #[test]
    fn a_replacement_in_the_way_of_every_attempt_reads_as_unverified() -> TestResult {
        let parent = Parent::new()?;
        let cache = parent.cache()?;
        let state = cache_state(&cache)?;
        assert_eq!(
            cache.record_verified(&digest(1), NOW),
            VerificationRecord::Recorded
        );
        let mut replaced_handles = (0..=MAX_RECORD_READ_ATTEMPTS)
            .map(|_| open_record(&state))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter();
        assert_eq!(
            cache.record_verified(&digest(2), NOW),
            VerificationRecord::Recorded
        );

        let mut opens = 0;
        let outcome = read_record_with(|| {
            opens += 1;
            replaced_handles
                .next()
                .ok_or_else(|| std::io::Error::other("no replaced handle left"))
        });

        assert_eq!(outcome.err(), Some(RecordDefect::Replaced));
        assert_eq!(opens, MAX_RECORD_READ_ATTEMPTS, "retries are bounded");
        Ok(())
    }

    #[test]
    fn open_failures_and_link_counts_are_classified() {
        assert_eq!(
            classify_open_error(&std::io::Error::from(std::io::ErrorKind::NotFound)),
            RecordDefect::Missing
        );
        for other in [
            std::io::Error::other("unexpected"),
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        ] {
            assert_eq!(classify_open_error(&other), RecordDefect::Unsafe);
        }
        // Windows refuses opens of a name whose previous file is pending
        // deletion: access denied, sharing violation or delete pending.
        #[cfg(windows)]
        for code in [5, 32, 303] {
            assert_eq!(
                classify_open_error(&std::io::Error::from_raw_os_error(code)),
                RecordDefect::Replaced,
                "{code}"
            );
        }
        assert_eq!(check_link_count(0), Err(RecordDefect::Replaced));
        assert_eq!(check_link_count(1), Ok(()));
        assert_eq!(check_link_count(2), Err(RecordDefect::Unsafe));
    }

    /// A flush that fails where the write succeeded (macOS `F_FULLFSYNC` on
    /// some volumes) must not skip the pass: the record stays strict either way.
    #[test]
    fn a_failed_flush_still_records_the_pass() -> TestResult {
        let parent = Parent::new()?;
        let cache = parent.cache()?;
        let state = cache_state(&cache)?;
        let record = StoredRecord {
            schema_version: SCHEMA_VERSION,
            entries: vec![StoredPass {
                fingerprint: hex(digest(7).digest()),
                verified_at_unix_seconds: NOW,
            }],
        };

        assert_eq!(
            write_record(&state, &record, |_| Err(std::io::Error::other(
                "flush unsupported"
            ))),
            Ok(())
        );

        assert_eq!(
            cache.lookup(&digest(7), NOW),
            CachedMediaToolVerification::Verified
        );
        assert!(
            state_entries(&parent)?
                .iter()
                .all(|name| !is_pending_name(name))
        );
        Ok(())
    }

    #[test]
    fn only_a_held_lock_is_reported_busy() {
        assert_eq!(
            RecordWriteFailure::Busy.skip(),
            VerificationRecordSkip::Busy
        );
        for failure in [
            RecordWriteFailure::Lock(LockFailure::Shape {
                regular_file: true,
                links: 2,
            }),
            RecordWriteFailure::Encode,
            RecordWriteFailure::Randomness,
            RecordWriteFailure::Pending,
            RecordWriteFailure::Replace,
        ] {
            assert_eq!(failure.skip(), VerificationRecordSkip::Unavailable);
        }
    }

    fn cause(kind: std::io::ErrorKind) -> IoCause {
        IoCause::of(&std::io::Error::from(kind))
    }

    /// Opens the real lock file of `parent`, as the writer does.
    fn lock_file(parent: &Parent) -> Result<fs::File, LockFailure> {
        fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(parent.state().join(LOCK_FILE))
            .map_err(|error| LockFailure::Open(IoCause::of(&error)))
    }

    fn outcome_name(outcome: &Result<HeldFileLock, RecordWriteFailure>) -> String {
        match outcome {
            Ok(_) => String::from("acquired"),
            Err(failure) => format!("{failure:?}"),
        }
    }

    /// Issue #136 on macOS: a writer reported a non-busy failure while taking
    /// the record lock. Failures a concurrent writer causes for an instant are
    /// retried a bounded number of times; everything else keeps its precise
    /// cause, and nothing but a holder is ever reported as busy.
    #[test]
    fn transient_lock_failures_are_retried_and_others_keep_their_cause() -> TestResult {
        let parent = Parent::new()?;
        let _ = parent.cache()?;

        // The lock file being created by another writer at the same moment.
        for transient in [
            LockFailure::Open(cause(std::io::ErrorKind::AlreadyExists)),
            LockFailure::Open(cause(std::io::ErrorKind::NotFound)),
            LockFailure::Open(cause(std::io::ErrorKind::Interrupted)),
            LockFailure::Inspect(cause(std::io::ErrorKind::Interrupted)),
            LockFailure::Shape {
                regular_file: true,
                links: 0,
            },
        ] {
            let mut opens = 0;
            let outcome = acquire_record_lock_with(
                || {
                    opens += 1;
                    if opens == 1 {
                        Err(transient)
                    } else {
                        lock_file(&parent)
                    }
                },
                HeldFileLock::try_exclusive,
            );
            assert_eq!(outcome_name(&outcome), "acquired", "{transient:?}");
            assert_eq!(opens, 2, "{transient:?}");
        }

        // A transient failure that persists is reported with its cause.
        let mut opens = 0;
        let persistent = LockFailure::Open(cause(std::io::ErrorKind::AlreadyExists));
        let outcome = acquire_record_lock_with(
            || {
                opens += 1;
                Err(persistent)
            },
            HeldFileLock::try_exclusive,
        );
        assert_eq!(outcome.err(), Some(RecordWriteFailure::Lock(persistent)));
        assert_eq!(opens, MAX_LOCK_ATTEMPTS, "retries are bounded");

        // Final failures are reported at once, never as busy.
        for final_failure in [
            LockFailure::Open(IoCause::of(&std::io::Error::from_raw_os_error(24))),
            LockFailure::Open(cause(std::io::ErrorKind::PermissionDenied)),
            LockFailure::Shape {
                regular_file: true,
                links: 2,
            },
            LockFailure::Shape {
                regular_file: false,
                links: 1,
            },
        ] {
            let mut opens = 0;
            let outcome = acquire_record_lock_with(
                || {
                    opens += 1;
                    Err(final_failure)
                },
                HeldFileLock::try_exclusive,
            );
            assert_eq!(outcome.err(), Some(RecordWriteFailure::Lock(final_failure)));
            assert_eq!(opens, 1, "{final_failure:?}");
        }
        Ok(())
    }

    #[test]
    fn refused_lock_calls_are_retried_and_only_holders_are_busy() -> TestResult {
        let parent = Parent::new()?;
        let _ = parent.cache()?;

        // An interrupted lock call is retried; a refused one keeps its code.
        let mut locks = 0;
        let outcome = acquire_record_lock_with(
            || lock_file(&parent),
            |file| {
                locks += 1;
                if locks == 1 {
                    Err(fs::TryLockError::Error(std::io::Error::from(
                        std::io::ErrorKind::Interrupted,
                    )))
                } else {
                    HeldFileLock::try_exclusive(file)
                }
            },
        );
        assert_eq!(outcome_name(&outcome), "acquired");
        drop(outcome);
        // A lock that keeps being refused (for example ENOLCK) is retried a
        // bounded number of times, then reported with its raw code.
        let refused = std::io::Error::from_raw_os_error(77);
        let expected = LockFailure::Acquire(IoCause::of(&refused));
        let mut locks = 0;
        let outcome = acquire_record_lock_with(
            || lock_file(&parent),
            |_| {
                locks += 1;
                Err(fs::TryLockError::Error(std::io::Error::from_raw_os_error(
                    77,
                )))
            },
        );
        assert_eq!(outcome.err(), Some(RecordWriteFailure::Lock(expected)));
        assert_eq!(locks, MAX_LOCK_ATTEMPTS);

        // A holder is busy at once, however std reports it.
        for held in [
            fs::TryLockError::WouldBlock,
            fs::TryLockError::Error(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
        ] {
            let mut slot = Some(held);
            let outcome = acquire_record_lock_with(
                || lock_file(&parent),
                |_| Err(slot.take().unwrap_or(fs::TryLockError::WouldBlock)),
            );
            assert_eq!(outcome.err(), Some(RecordWriteFailure::Busy));
        }
        Ok(())
    }

    #[test]
    fn a_hard_linked_lock_file_is_refused_with_its_shape_never_as_busy() -> TestResult {
        let parent = Parent::new()?;
        let cache = parent.cache()?;
        let state = cache_state(&cache)?;
        let outside = parent.0.join("outside.lock");
        fs::write(&outside, b"")?;
        fs::hard_link(&outside, parent.state().join(LOCK_FILE))?;

        assert_eq!(
            record_pass(&state, &hex(digest(1).digest()), NOW).err(),
            Some(RecordWriteFailure::Lock(LockFailure::Shape {
                regular_file: true,
                links: 2,
            }))
        );
        assert_eq!(
            cache.record_verified(&digest(1), NOW),
            VerificationRecord::Skipped(VerificationRecordSkip::Unavailable)
        );
        assert_eq!(
            cache.remove_stale_workspaces(NOW),
            StaleWorkspaceSweep::Skipped
        );
        assert_eq!(check_lock_shape(true, 1), Ok(()));
        Ok(())
    }

    const LEFTOVER: &str = "vsift-tool-verification-0123456789abcdef";
    const UNLOCKED: &str = "vsift-tool-verification-1111111111111111";
    const LIVE: &str = "vsift-tool-verification-2222222222222222";

    fn real_now() -> Result<u64, Box<dyn Error>> {
        Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
    }

    /// A leftover workspace as a killed verification leaves it.
    fn leftover(parent: &Parent, name: &str) -> Result<PathBuf, Box<dyn Error>> {
        let workspace = parent.state().join(name);
        fs::create_dir(&workspace)?;
        fs::write(workspace.join("F01.mp4"), b"fixture")?;
        fs::create_dir(workspace.join("store"))?;
        fs::write(workspace.join("store").join("marker"), b"session")?;
        Ok(workspace)
    }

    fn with_lock_file(workspace: &Path) -> Result<fs::File, Box<dyn Error>> {
        Ok(fs::File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(workspace.join(WORKSPACE_LOCK_FILE))?)
    }

    /// Issue #132: leftovers are removed; live workspaces, look-alikes and
    /// everything else in the state directory are kept.
    #[test]
    fn stale_leftover_workspaces_are_removed_and_nothing_else() -> TestResult {
        let parent = Parent::new()?;
        let cache = parent.cache()?;
        assert_eq!(
            cache.record_verified(&digest(1), NOW),
            VerificationRecord::Recorded
        );
        let lockless = leftover(&parent, LEFTOVER)?;
        let unlocked = leftover(&parent, UNLOCKED)?;
        drop(with_lock_file(&unlocked)?);
        let live = leftover(&parent, LIVE)?;
        let held = HeldFileLock::try_exclusive(with_lock_file(&live)?)
            .map_err(|_| std::io::Error::other("lock unavailable"))?;
        let lookalikes = [
            "vsift-tool-verification-ABCDEF0123456789",
            "vsift-tool-verification-0123",
            "vsift-tool-verification-0123456789abcdef0",
            "vsift-tool-verification-test",
            "other",
        ];
        for name in lookalikes {
            leftover(&parent, name)?;
        }
        let file_named_like_a_workspace = parent
            .state()
            .join("vsift-tool-verification-3333333333333333");
        fs::write(&file_named_like_a_workspace, b"not a directory")?;
        // A hard link inside a leftover is removed as a link; its target stays.
        let outside = parent.0.join("outside-evidence");
        fs::write(&outside, b"keep me")?;
        fs::hard_link(&outside, lockless.join("linked"))?;

        let later = real_now()? + STALE_VERIFICATION_WORKSPACE_AGE_SECONDS + 60;
        assert_eq!(
            cache.remove_stale_workspaces(later),
            StaleWorkspaceSweep::Completed { removed: 2 }
        );

        assert!(!lockless.exists());
        assert!(!unlocked.exists());
        assert!(
            live.join("store").join("marker").is_file(),
            "live workspace removed"
        );
        for name in lookalikes {
            assert!(
                parent.state().join(name).join("F01.mp4").is_file(),
                "{name}"
            );
        }
        assert!(file_named_like_a_workspace.is_file());
        assert_eq!(fs::read(&outside)?, b"keep me");
        assert_eq!(
            cache.lookup(&digest(1), NOW),
            CachedMediaToolVerification::Verified,
            "the record was disturbed"
        );

        // Once its verification has ended, the workspace is a leftover too.
        held.release()?;
        assert_eq!(
            cache.remove_stale_workspaces(later),
            StaleWorkspaceSweep::Completed { removed: 1 }
        );
        assert!(!live.exists());
        Ok(())
    }

    #[test]
    fn recent_workspaces_are_kept_until_they_age() -> TestResult {
        let parent = Parent::new()?;
        let cache = parent.cache()?;
        let created = real_now()?;
        let workspace = leftover(&parent, LEFTOVER)?;

        for now in [
            created,
            created + STALE_VERIFICATION_WORKSPACE_AGE_SECONDS - 1,
            // A modification time ahead of the clock is never old.
            created - STALE_VERIFICATION_WORKSPACE_AGE_SECONDS,
        ] {
            assert_eq!(
                cache.remove_stale_workspaces(now),
                StaleWorkspaceSweep::Completed { removed: 0 },
                "{now}"
            );
            assert!(workspace.is_dir());
        }

        assert_eq!(
            cache.remove_stale_workspaces(created + STALE_VERIFICATION_WORKSPACE_AGE_SECONDS + 5),
            StaleWorkspaceSweep::Completed { removed: 1 }
        );
        assert!(!workspace.exists());
        Ok(())
    }

    #[test]
    fn a_sweep_never_waits_for_the_record_lock_and_is_bounded() -> TestResult {
        let parent = Parent::new()?;
        let cache = parent.cache()?;
        let total = MAX_REMOVED_VERIFICATION_WORKSPACES + 2;
        for index in 0..total {
            leftover(&parent, &format!("vsift-tool-verification-{index:016x}"))?;
        }
        let later = real_now()? + STALE_VERIFICATION_WORKSPACE_AGE_SECONDS + 60;
        let lock_file = fs::File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(parent.state().join(LOCK_FILE))?;
        let held = HeldFileLock::try_exclusive(lock_file)
            .map_err(|_| std::io::Error::other("lock unavailable"))?;

        assert_eq!(
            cache.remove_stale_workspaces(later),
            StaleWorkspaceSweep::Skipped
        );
        held.release()?;
        assert_eq!(
            cache.remove_stale_workspaces(later),
            StaleWorkspaceSweep::Completed {
                removed: MAX_REMOVED_VERIFICATION_WORKSPACES
            }
        );
        assert_eq!(
            cache.remove_stale_workspaces(later),
            StaleWorkspaceSweep::Completed { removed: 2 }
        );
        assert_eq!(state_entries(&parent)?, [LOCK_FILE.to_owned()]);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn links_named_or_nested_like_workspaces_are_never_followed() -> TestResult {
        let parent = Parent::new()?;
        let cache = parent.cache()?;
        let outside = parent.0.join("outside");
        fs::create_dir(&outside)?;
        fs::write(outside.join("evidence"), b"keep me")?;
        std::os::unix::fs::symlink(&outside, parent.state().join(LEFTOVER))?;
        let workspace = leftover(&parent, UNLOCKED)?;
        std::os::unix::fs::symlink(&outside, workspace.join("escape"))?;

        let later = real_now()? + STALE_VERIFICATION_WORKSPACE_AGE_SECONDS + 60;
        assert_eq!(
            cache.remove_stale_workspaces(later),
            StaleWorkspaceSweep::Completed { removed: 1 }
        );

        assert!(!workspace.exists());
        assert!(
            fs::symlink_metadata(parent.state().join(LEFTOVER))?
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(outside.join("evidence"))?, b"keep me");
        Ok(())
    }

    /// A workspace made by a running verification is never swept, however old
    /// the clock says it is.
    #[test]
    fn a_live_verification_workspace_is_never_swept() -> TestResult {
        let parent = Parent::new()?;
        let cache = parent.cache()?;
        let workspace = VerificationWorkspace::create(cache.workspace_parent())?;
        let path = workspace.path().to_path_buf();
        let later = real_now()? + 10 * STALE_VERIFICATION_WORKSPACE_AGE_SECONDS;

        assert_eq!(
            cache.remove_stale_workspaces(later),
            StaleWorkspaceSweep::Completed { removed: 0 }
        );
        assert!(path.join(WORKSPACE_LOCK_FILE).is_file());

        drop(workspace);
        assert!(!path.exists());
        Ok(())
    }
}
