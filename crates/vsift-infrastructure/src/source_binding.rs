//! Bracketed source binding for operations that call a media provider many
//! times over one session's private source copy (issue #148, ADR 0012 note of
//! 2026-09-26).
//!
//! ADR 0012's check-before-use rule rehashes the whole copy before every
//! provider call. That is right for one call, but a local speech-recognition
//! run makes one `FFmpeg` call per 30 s chunk, so a one-hour source would be
//! hashed about 150 times. A [`BoundSource`] brackets the whole operation
//! instead: one full SHA-256 verification when it is opened, a cheap on-disk
//! identity comparison immediately before each provider call, and a second
//! full verification (after a last identity comparison) before the caller
//! commits anything derived from the source.
//!
//! # Why the guarantee is unchanged
//!
//! ADR 0012 already records that a same-user actor can modify the private
//! copy after a verification and before the provider reads it: per-call
//! hashing only proves the bytes at the instant they were hashed. The
//! bracketed binding keeps exactly that residual and nothing wider:
//!
//! - Any change that is still present when the operation ends fails the
//!   closing full verification, so nothing derived from different bytes is
//!   committed.
//! - A replacement of the copy by another file (a rename or delete and
//!   recreate), a change of size or modification time, and on Unix any write
//!   or metadata change at all (the kernel sets the status-change time, which
//!   an unprivileged process cannot set back) fail the next identity check,
//!   before the provider reads the changed file.
//! - What remains is a transient in-place change that is undone, bytes and
//!   visible metadata alike, before the closing verification. Per-call
//!   hashing has the same blind spot: an actor who can write the file between
//!   a hash and the provider's read can also restore it afterwards. On
//!   Windows, which has no user-immutable change time, an actor who also
//!   restores the modification time is caught only by the closing hash, which
//!   is the same instant-in-time guarantee per-call hashing gave. File times
//!   also have the filesystem clock's granularity, so a write within the same
//!   tick as the binding is likewise left to the closing hash.
//!
//! Only a same-user actor can write the copy at all: it lives in a private
//! session directory (SEC-18) under the session's lifetime hold.

use cap_std::fs::{File, Metadata};
use sha2::{Digest, Sha256};
use vsift_domain::SourceCheck;

use crate::{FilesystemSessionStore, SourceError, SourceSnapshot};

/// A digest of the on-disk identity a session's source copy had right after
/// a full SHA-256 verification (ADR 0019 D1).
///
/// An evidence call that hashes the copy in full commits this digest with
/// its evidence; later read-only evidence calls compare the copy's current
/// identity with it instead of hashing the bytes again. It is a digest, so
/// the session manifest holds no device or file numbers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedSourceIdentity(String);

impl VerifiedSourceIdentity {
    /// Digests an identity, or returns `None` when it has no modification
    /// time after the Unix epoch; such a copy is always hashed in full.
    pub(crate) fn of(identity: &FileIdentity) -> Option<Self> {
        let modified = identity
            .modified?
            .into_std()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        let mut hasher = Sha256::new();
        let mut field = |bytes: &[u8]| {
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        };
        field(b"vsift.source-identity.v1");
        field(&identity.bytes.to_le_bytes());
        field(&modified.as_secs().to_le_bytes());
        field(&modified.subsec_nanos().to_le_bytes());
        field(&identity.device.to_le_bytes());
        field(&identity.file_index.to_le_bytes());
        identity.platform.digest_into(&mut field);
        let digest = hasher.finalize();
        let mut text = String::with_capacity(64);
        for byte in digest {
            text.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
            text.push(char::from(b"0123456789abcdef"[usize::from(byte & 0x0f)]));
        }
        Some(Self(text))
    }

    /// Accepts a stored digest: exactly 64 lowercase hexadecimal digits.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        (text.len() == 64
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        .then(|| Self(text.to_owned()))
    }

    /// The digest's canonical text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How an evidence call bound the session's source copy (ADR 0019 D1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvidenceSourceCheck {
    /// The copy's identity matched the one recorded after an earlier full
    /// verification; its bytes were not read.
    Identity,
    /// The copy was hashed in full and matched the committed source. The
    /// identity it had then is committed with the call's evidence, if it can
    /// be recorded.
    FullHash(Option<VerifiedSourceIdentity>),
}

impl EvidenceSourceCheck {
    /// The check as recorded in evidence items.
    #[must_use]
    pub const fn check(&self) -> SourceCheck {
        match self {
            Self::Identity => SourceCheck::Identity,
            Self::FullHash(_) => SourceCheck::FullHash,
        }
    }

    /// The identity to commit with the call's evidence, if any.
    #[must_use]
    pub const fn verified_identity(&self) -> Option<&VerifiedSourceIdentity> {
        match self {
            Self::Identity => None,
            Self::FullHash(identity) => identity.as_ref(),
        }
    }
}

/// How a media provider call proves that it reads the session's committed
/// source bytes.
///
/// The media adapter accepts any binding: a plain [`SourceSnapshot`] rehashes
/// the whole copy before every call (ADR 0012, right for single-call
/// operations such as `ingest`'s probe), and a [`BoundSource`] compares the
/// copy's on-disk identity (for operations that call the provider many
/// times). The trait is sealed so that no binding outside this crate can
/// weaken the check.
pub trait SourceBinding: sealed::Sealed + Sync {
    /// The held private copy the provider reads.
    fn snapshot(&self) -> &SourceSnapshot;

    /// The check made immediately before one provider call.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::SnapshotChanged`] (or an I/O failure) when the
    /// copy is no longer the committed one.
    fn check_before_provider_call(&self) -> Result<(), SourceError>;
}

mod sealed {
    /// Restricts [`super::SourceBinding`] to the bindings defined here.
    pub trait Sealed {}

    impl Sealed for crate::SourceSnapshot {}
    impl Sealed for super::BoundSource {}
}

impl SourceBinding for SourceSnapshot {
    fn snapshot(&self) -> &SourceSnapshot {
        self
    }

    fn check_before_provider_call(&self) -> Result<(), SourceError> {
        self.verify()
    }
}

/// A session's private source copy bound for one multi-call operation.
///
/// Opening it verifies the copy's SHA-256 once and records the identity of
/// the file that was hashed. Every provider call through the media adapter
/// then compares only that identity, and [`Self::release_verified`] verifies
/// the full SHA-256 again before the caller commits. The type is the state:
/// a provider cannot be given a bound source without its opening
/// verification, and the snapshot is handed back for commit only after the
/// closing one.
pub struct BoundSource {
    snapshot: SourceSnapshot,
    identity: FileIdentity,
}

impl BoundSource {
    /// Reopens an open session's committed source copy for a multi-call
    /// operation, with one full verification.
    ///
    /// # Errors
    ///
    /// As [`SourceSnapshot::open_committed`]: [`SourceError::Storage`] for a
    /// missing, closed, expired or busy session and
    /// [`SourceError::SnapshotChanged`] for a copy that no longer matches its
    /// committed identity or changed while it was hashed.
    pub fn open_committed(
        store: &FilesystemSessionStore,
        session_id: &vsift_domain::SessionId,
        now_unix_seconds: u64,
    ) -> Result<Self, SourceError> {
        let (snapshot, identity) =
            SourceSnapshot::open_committed_identified(store, session_id, now_unix_seconds)?;
        Ok(Self { snapshot, identity })
    }

    /// Reopens an open session's committed source copy for a read-only
    /// evidence call (ADR 0019 D1): an identity comparison when the session
    /// records a verified identity the copy still has, a full verification
    /// otherwise.
    ///
    /// The residual is ADR 0012's, extended across calls: on Windows a
    /// same-user rewrite that restores the modification time is caught only
    /// by a later full hash. Mutating and committing operations elsewhere
    /// keep [`Self::open_committed`] and [`Self::release_verified`].
    ///
    /// # Errors
    ///
    /// As [`Self::open_committed`]; a copy whose identity changed and whose
    /// bytes differ from the committed source is
    /// [`SourceError::SnapshotChanged`].
    pub fn open_for_evidence(
        store: &FilesystemSessionStore,
        session_id: &vsift_domain::SessionId,
        now_unix_seconds: u64,
    ) -> Result<(Self, EvidenceSourceCheck), SourceError> {
        let (snapshot, identity, hashed) =
            SourceSnapshot::open_committed_for_evidence(store, session_id, now_unix_seconds)?;
        let check = if hashed {
            EvidenceSourceCheck::FullHash(VerifiedSourceIdentity::of(&identity))
        } else {
            EvidenceSourceCheck::Identity
        };
        Ok((Self { snapshot, identity }, check))
    }

    /// Ends a read-only evidence call's provider calls with a last identity
    /// comparison (ADR 0019 D1) and hands the snapshot back so the caller
    /// keeps the session's hold until it has committed.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::SnapshotChanged`] when the copy's identity
    /// changed; the caller must then commit nothing.
    pub fn release_identity_checked(self) -> Result<SourceSnapshot, SourceError> {
        self.check_identity()?;
        Ok(self.snapshot)
    }

    /// Binds an already staged snapshot, with one full verification.
    ///
    /// For operations that stage their own private copy (the local-ASR
    /// fixture check) rather than reopening a session's committed one.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::SnapshotChanged`] when the copy no longer
    /// matches the staged identity or changed while it was hashed.
    pub fn bind(snapshot: SourceSnapshot) -> Result<Self, SourceError> {
        let identity = snapshot.rehash(open_bound_copy(&snapshot)?)?;
        Ok(Self { snapshot, identity })
    }

    /// The held private copy.
    #[must_use]
    pub const fn snapshot(&self) -> &SourceSnapshot {
        &self.snapshot
    }

    /// Compares the copy's current on-disk identity with the one recorded
    /// when it was hashed, without reading its contents.
    ///
    /// The copy is reopened by name, no-follow, inside the held directory, so
    /// a file renamed into its place is a different file even when its bytes,
    /// size and times match.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::SnapshotChanged`] when the copy is missing, is
    /// not a regular file, or differs in size, modification time, device,
    /// file identity or platform change fields.
    pub fn check_identity(&self) -> Result<(), SourceError> {
        let file = open_bound_copy(&self.snapshot)?;
        let metadata = file.metadata().map_err(SourceError::Io)?;
        if !metadata.is_file() || FileIdentity::of(&metadata) != self.identity {
            return Err(SourceError::SnapshotChanged);
        }
        Ok(())
    }

    /// Ends the operation's provider calls: compares the identity a last time,
    /// verifies the full SHA-256 again and hands the snapshot back so the
    /// caller keeps the session's hold until it has committed.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::SnapshotChanged`] when the copy's identity or
    /// bytes changed at any point the checks can observe; the caller must
    /// then commit nothing.
    pub fn release_verified(self) -> Result<SourceSnapshot, SourceError> {
        self.check_identity()?;
        let identity = self.snapshot.rehash(open_bound_copy(&self.snapshot)?)?;
        if identity != self.identity {
            return Err(SourceError::SnapshotChanged);
        }
        Ok(self.snapshot)
    }
}

impl SourceBinding for BoundSource {
    fn snapshot(&self) -> &SourceSnapshot {
        &self.snapshot
    }

    fn check_before_provider_call(&self) -> Result<(), SourceError> {
        self.check_identity()
    }
}

/// Reopens a bound copy. Within a bound operation a copy that cannot be
/// reopened has been removed or replaced, which is a changed snapshot, as
/// when [`SourceSnapshot::open_committed`] cannot open it.
fn open_bound_copy(snapshot: &SourceSnapshot) -> Result<File, SourceError> {
    snapshot
        .open_copy()
        .map_err(|_| SourceError::SnapshotChanged)
}

/// The on-disk identity of the private copy, read from an open handle.
///
/// The fields mirror the media-tool fingerprint's file identity (size,
/// modification time and platform identity), plus the device and file index
/// on every platform, because the copy is reopened by name and a substituted
/// file must not pass as the original. Contents are never part of it; that is
/// what the two full verifications are for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileIdentity {
    bytes: u64,
    modified: Option<cap_std::time::SystemTime>,
    device: u64,
    file_index: u64,
    platform: PlatformIdentity,
}

impl FileIdentity {
    /// Reads the identity from metadata of an open handle.
    ///
    /// On Windows `cap_fs_ext` reports the volume serial number and file
    /// index that cap-std reads from the handle itself; metadata from
    /// [`File::metadata`] always carries both, which is why identities are
    /// only ever read from an open handle and never from a path.
    pub(crate) fn of(metadata: &Metadata) -> Self {
        Self {
            bytes: metadata.len(),
            modified: metadata.modified().ok(),
            device: cap_fs_ext::MetadataExt::dev(metadata),
            file_index: cap_fs_ext::MetadataExt::ino(metadata),
            platform: PlatformIdentity::of(metadata),
        }
    }
}

/// Unix change fields: permission bits, owner and the status-change time,
/// which the kernel sets on every write or metadata change and an
/// unprivileged process cannot set back.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PlatformIdentity {
    mode: u32,
    owner: u32,
    changed_seconds: i64,
    changed_nanos: i64,
}

#[cfg(unix)]
impl PlatformIdentity {
    fn digest_into(&self, field: &mut impl FnMut(&[u8])) {
        field(&self.mode.to_le_bytes());
        field(&self.owner.to_le_bytes());
        field(&self.changed_seconds.to_le_bytes());
        field(&self.changed_nanos.to_le_bytes());
    }

    fn of(metadata: &Metadata) -> Self {
        // `cap_fs_ext::MetadataExt` also defines `dev` and `ino`; name the
        // cap-std Unix trait explicitly.
        Self {
            mode: cap_std::fs::MetadataExt::mode(metadata),
            owner: cap_std::fs::MetadataExt::uid(metadata),
            changed_seconds: cap_std::fs::MetadataExt::ctime(metadata),
            changed_nanos: cap_std::fs::MetadataExt::ctime_nsec(metadata),
        }
    }
}

/// Windows change fields: creation time and file attributes. Windows keeps
/// no change time that a same-user process cannot set, so an in-place
/// rewrite that restores the modification time is caught by the closing
/// full verification rather than by an identity check.
#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PlatformIdentity {
    created: u64,
    attributes: u32,
}

#[cfg(windows)]
impl PlatformIdentity {
    fn digest_into(&self, field: &mut impl FnMut(&[u8])) {
        field(&self.created.to_le_bytes());
        field(&self.attributes.to_le_bytes());
    }

    fn of(metadata: &Metadata) -> Self {
        Self {
            created: cap_std::fs::MetadataExt::creation_time(metadata),
            attributes: cap_std::fs::MetadataExt::file_attributes(metadata),
        }
    }
}

#[cfg(test)]
mod tests;
