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
    io::{Read, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
#[cfg(unix)]
use cap_std::fs::OpenOptionsExt;
use cap_std::fs::{Dir, DirBuilder, OpenOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vsift_application::{
    CachedMediaToolVerification, MediaToolFingerprint, MediaToolVerificationCache,
    ReviewedCompatibilityPolicy, VerificationRecord, VerificationRecordSkip,
};

use crate::{
    HostIsolation, MediaProviderConformance,
    file_lock::HeldFileLock,
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

const STATE_DIRECTORY: &str = "media-tool-verification";
const RECORD_FILE: &str = "verified-v1.json";
const LOCK_FILE: &str = "verified.lock";
const PENDING_PREFIX: &str = "verified-";
const PENDING_SUFFIX: &str = ".pending";
const PENDING_RANDOM_BYTES: usize = 16;
/// Bounds the stale-pending sweep so a hostile directory cannot stall a write.
const MAX_SWEPT_ENTRIES: usize = 256;
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
    const fn tag(self) -> &'static [u8] {
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
fn field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn number(hasher: &mut Sha256, value: u64) {
    field(hasher, &value.to_le_bytes());
}

fn list(hasher: &mut Sha256, values: &[&str]) {
    number(hasher, values.len() as u64);
    for value in values {
        field(hasher, value.as_bytes());
    }
}

const fn isolation_tag(host_isolation: HostIsolation) -> &'static [u8] {
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

fn executable_identity(hasher: &mut Sha256, path: &Path) -> Option<()> {
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
            Err(skip) => VerificationRecord::Skipped(skip),
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
    Missing,
    Unsafe,
    Unreadable,
    Invalid,
}

fn read_record(state: &Dir) -> Result<StoredRecord, RecordDefect> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = match state.open_with(RECORD_FILE, &options) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(RecordDefect::Missing);
        }
        Err(_) => return Err(RecordDefect::Unsafe),
    };
    let metadata = file.metadata().map_err(|_| RecordDefect::Unreadable)?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(RecordDefect::Unsafe);
    }
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

fn record_pass(
    state: &Dir,
    fingerprint: &str,
    now_unix_seconds: u64,
) -> Result<(), VerificationRecordSkip> {
    let lock = open_lock(state)?;
    let held = HeldFileLock::try_exclusive(lock).map_err(|error| match error {
        fs::TryLockError::WouldBlock => VerificationRecordSkip::Busy,
        fs::TryLockError::Error(_) => VerificationRecordSkip::Unavailable,
    })?;
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
    );
    // The record is complete either way; a failed unlock is released on close.
    let _ = held.release();
    written
}

fn open_lock(state: &Dir) -> Result<fs::File, VerificationRecordSkip> {
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
        .map_err(|_| VerificationRecordSkip::Unavailable)?;
    let metadata = lock
        .metadata()
        .map_err(|_| VerificationRecordSkip::Unavailable)?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(VerificationRecordSkip::Unavailable);
    }
    Ok(lock.into_std())
}

fn write_record(state: &Dir, record: &StoredRecord) -> Result<(), VerificationRecordSkip> {
    let unavailable = VerificationRecordSkip::Unavailable;
    let bytes = serde_json::to_vec(record).map_err(|_| unavailable)?;
    if bytes.len() as u64 > MAX_MEDIA_TOOL_VERIFICATION_RECORD_BYTES {
        return Err(unavailable);
    }
    let mut random = [0_u8; PENDING_RANDOM_BYTES];
    getrandom::fill(&mut random).map_err(|_| unavailable)?;
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
        .map_err(|_| unavailable)?;
    if file
        .write_all(&bytes)
        .and_then(|()| file.sync_all())
        .is_err()
    {
        drop(file);
        let _ = state.remove_file(&pending);
        return Err(unavailable);
    }
    drop(file);
    // Rename replaces the directory entry itself, so a planted link or a
    // hard-linked record is swapped out rather than written through.
    if state.rename(&pending, state, RECORD_FILE).is_err() {
        let _ = state.remove_file(&pending);
        return Err(unavailable);
    }
    Ok(())
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
        path::PathBuf,
        sync::{Arc, Barrier},
        thread,
        time::{Duration, SystemTime},
    };

    use vsift_application::{
        CachedMediaToolVerification, MediaToolFingerprint, MediaToolVerificationCache,
        VerificationRecord, VerificationRecordSkip,
    };

    use super::{
        FilesystemMediaToolVerificationCache, LOCK_FILE, MAX_MEDIA_TOOL_VERIFICATION_ENTRIES,
        MAX_MEDIA_TOOL_VERIFICATION_RECORD_BYTES, MEDIA_TOOL_VERIFICATION_MAX_AGE_SECONDS,
        MediaToolVerificationAuthority, RECORD_FILE, RecordDefect, STATE_DIRECTORY, hex,
        is_pending_name, media_tool_fingerprint, read_record,
    };
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
                    barrier.wait();
                    let mut recorded = 0;
                    for round in 0..ROUNDS {
                        match cache.record_verified(&digest(writer * ROUNDS + round), NOW) {
                            VerificationRecord::Recorded => recorded += 1,
                            VerificationRecord::Skipped(VerificationRecordSkip::Busy) => {}
                            other @ VerificationRecord::Skipped(_) => {
                                return Err(format!("unexpected outcome {other:?}"));
                            }
                        }
                        let state = cache.state.as_ref().ok_or("state unavailable")?;
                        match read_record(state) {
                            Ok(_) | Err(RecordDefect::Missing) => {}
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
}
