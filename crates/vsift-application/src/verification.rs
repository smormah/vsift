//! Verification that selected specialist tools work, not merely that they exist.
//!
//! `setup check` only proves an executable responds to a version probe. The
//! types here describe a stronger result: the selected tools processed a
//! reviewed fixture through `VSift`'s real media pipeline and produced its
//! recorded truth. Hosts call the port directly, and [`preflight_media_tools`]
//! calls it before the first media stage of an operation, reusing a cached pass
//! for an unchanged tool identity (ADR 0015).

use std::future::Future;

use vsift_domain::ReviewedAsrModel;

use crate::AsrFailure;

/// The media operation a verification exercised when it stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaToolCheck {
    /// Private workspace and fixture preparation, before any tool ran.
    Preparation,
    /// `FFprobe` metadata inspection.
    Probe,
    /// `FFmpeg` frame extraction.
    Frame,
    /// `FFmpeg` audio extraction.
    Audio,
    /// `FFmpeg` visual sampling for the candidate index (P08).
    VisualSampling,
}

impl MediaToolCheck {
    /// Returns the stable identifier used in machine-readable contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Preparation => "preparation",
            Self::Probe => "probe",
            Self::Frame => "frame",
            Self::Audio => "audio",
            Self::VisualSampling => "visual_sampling",
        }
    }
}

/// Why a media tool verification did not pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaToolFailure {
    /// The embedded fixture or reviewed policy failed its own integrity check.
    FixtureIntegrity,
    /// The private verification workspace could not be prepared.
    Workspace,
    /// The tool could not be started or exited abnormally.
    ProcessFailure,
    /// The tool rejected or could not decode the fixture.
    ProviderRejected,
    /// The tool exceeded its deadline.
    Deadline,
    /// Tool output exceeded a reviewed byte limit.
    OutputLimit,
    /// The tool ran but its results differ from the fixture's recorded truth.
    UnexpectedResult,
    /// The caller cancelled verification.
    Cancelled,
}

impl MediaToolFailure {
    /// Returns the stable identifier used in machine-readable contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::FixtureIntegrity => "fixture_integrity",
            Self::Workspace => "workspace",
            Self::ProcessFailure => "process_failure",
            Self::ProviderRejected => "provider_rejected",
            Self::Deadline => "deadline",
            Self::OutputLimit => "output_limit",
            Self::UnexpectedResult => "unexpected_result",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Outcome of running the selected `FFmpeg` and `FFprobe` against the fixture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaToolVerification {
    /// Probe, frame, audio and visual-sampling results all matched the
    /// fixture's recorded truth.
    Verified,
    /// Verification stopped at `check` for `failure`; later checks did not run.
    Failed {
        /// Operation that did not pass.
        check: MediaToolCheck,
        /// Reason it did not pass.
        failure: MediaToolFailure,
    },
}

impl MediaToolVerification {
    /// Reports whether every media check passed.
    #[must_use]
    pub const fn is_verified(self) -> bool {
        matches!(self, Self::Verified)
    }
}

/// Port that proves the selected media tools work on this machine.
pub trait MediaToolVerifier: Send + Sync {
    /// Runs the bounded fixture verification, leaving no workspace behind.
    fn verify(&self) -> impl Future<Output = MediaToolVerification> + Send;
}

/// Identity of everything that makes one media-tool verification valid.
///
/// Infrastructure derives it from the executables' on-disk identity, the
/// reviewed compatibility policy, the verification profile and the `VSift`
/// version. The application only compares fingerprints, so it never needs to
/// know which inputs were bound, and a cache that stores fingerprints stores
/// digests rather than paths.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MediaToolFingerprint([u8; 32]);

impl MediaToolFingerprint {
    /// Wraps a SHA-256 digest computed over the bound inputs.
    #[must_use]
    pub const fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    /// Returns the digest.
    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Whether a cache holds a still-valid pass for one fingerprint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CachedMediaToolVerification {
    /// A pass for exactly this fingerprint is on record and has not aged out.
    Verified,
    /// No trustworthy pass is on record; the tools must be verified.
    Unverified,
}

/// Why a passing verification was not recorded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationRecordSkip {
    /// The tools' identity could not be read, so a pass could not be keyed.
    IdentityUnavailable,
    /// Another process held the cache lock; recording is never waited for.
    Busy,
    /// The cache storage could not be used safely.
    Unavailable,
}

/// What happened when a passing verification was offered to the cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationRecord {
    /// The pass was recorded; the next preflight for the same identity is free.
    Recorded,
    /// The pass was not recorded; the next preflight verifies again.
    Skipped(VerificationRecordSkip),
}

/// Port for the per-user record of media-tool setups that already passed.
///
/// Implementations fail closed: anything unreadable, corrupt, oversized,
/// linked or otherwise untrustworthy is reported as
/// [`CachedMediaToolVerification::Unverified`], and a pass that cannot be
/// recorded is [`VerificationRecord::Skipped`]. Neither method returns an
/// error because no cache failure may stop an operation; it only costs a later
/// re-verification. Failures are never recorded.
pub trait MediaToolVerificationCache: Send + Sync {
    /// Reports whether a pass for `fingerprint` is on record and still valid at
    /// `now_unix_seconds`.
    fn lookup(
        &self,
        fingerprint: &MediaToolFingerprint,
        now_unix_seconds: u64,
    ) -> CachedMediaToolVerification;

    /// Records that `fingerprint` passed verification at `now_unix_seconds`.
    fn record_verified(
        &self,
        fingerprint: &MediaToolFingerprint,
        now_unix_seconds: u64,
    ) -> VerificationRecord;
}

/// How a preflight established that the selected media tools work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaToolPreflightOutcome {
    /// A still-valid pass for the same identity was on record; no tool ran.
    AlreadyVerified,
    /// Verification ran and passed; the record says whether it was cached.
    VerifiedNow(VerificationRecord),
}

/// A preflight whose verification did not pass.
///
/// It names the check that stopped verification and the reason, so a caller
/// can tell an unusable tool from an unusable workspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaToolPreflightFailure {
    /// Operation that did not pass.
    pub check: MediaToolCheck,
    /// Reason it did not pass.
    pub failure: MediaToolFailure,
}

/// Ensures the selected media tools are verified before a media stage runs.
///
/// A still-valid cached pass for `fingerprint` skips verification. Otherwise
/// the verifier runs; a pass is offered to the cache and a failure is returned
/// without being recorded. Without a fingerprint (the tools' identity could not
/// be read) verification always runs and nothing is recorded, because a pass
/// that cannot be keyed to an identity must not be reused.
///
/// # Errors
///
/// Returns the failed check and its reason when verification does not pass.
pub async fn preflight_media_tools<V, C>(
    verifier: &V,
    cache: &C,
    fingerprint: Option<&MediaToolFingerprint>,
    now_unix_seconds: u64,
) -> Result<MediaToolPreflightOutcome, MediaToolPreflightFailure>
where
    V: MediaToolVerifier,
    C: MediaToolVerificationCache,
{
    if let Some(fingerprint) = fingerprint
        && cache.lookup(fingerprint, now_unix_seconds) == CachedMediaToolVerification::Verified
    {
        return Ok(MediaToolPreflightOutcome::AlreadyVerified);
    }
    match verifier.verify().await {
        MediaToolVerification::Verified => {
            Ok(MediaToolPreflightOutcome::VerifiedNow(fingerprint.map_or(
                VerificationRecord::Skipped(VerificationRecordSkip::IdentityUnavailable),
                |fingerprint| cache.record_verified(fingerprint, now_unix_seconds),
            )))
        }
        MediaToolVerification::Failed { check, failure } => {
            Err(MediaToolPreflightFailure { check, failure })
        }
    }
}

/// Why local speech recognition failed its functional verification.
///
/// The verification runs a reviewed speech fixture through the same chain a
/// retranscription uses (speech PCM, the recognizer, output parsing and
/// validation) and compares the transcript with the fixture's recorded words
/// and speech window (ADR 0017).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalAsrVerificationFailure {
    /// The embedded speech fixture failed its own integrity check.
    FixtureIntegrity,
    /// The private verification workspace could not be prepared.
    Workspace,
    /// The fixture could not be probed or its speech stream is missing, with
    /// media tools that already passed their own verification.
    FixtureMedia,
    /// Transcribing the fixture failed at a typed stage.
    Transcription(AsrFailure),
    /// Transcription completed but missed the fixture's recorded words or
    /// placed them outside its recorded speech window.
    UnexpectedTranscript,
}

impl LocalAsrVerificationFailure {
    /// Stable machine-readable identifier of the failure kind.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::FixtureIntegrity => "fixture_integrity",
            Self::Workspace => "workspace",
            Self::FixtureMedia => "fixture_media",
            Self::Transcription(_) => "transcription",
            Self::UnexpectedTranscript => "unexpected_transcript",
        }
    }
}

/// Outcome of running local speech recognition on the reviewed fixture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalAsrVerification {
    /// The fixture's words were recognised inside its speech window.
    Verified,
    /// Verification did not pass.
    Failed(LocalAsrVerificationFailure),
}

/// Port that proves the selected recognizer and model transcribe speech here.
pub trait LocalAsrVerifier: Send + Sync {
    /// Runs the bounded fixture verification, leaving no workspace behind.
    fn verify(&self) -> impl Future<Output = LocalAsrVerification> + Send;
}

/// Ensures local speech recognition is verified before it touches user media.
///
/// It mirrors [`preflight_media_tools`] and shares its per-user record: the
/// record stores only digests, and a local-ASR fingerprint is derived in its
/// own domain (it binds the recognizer, the model, the media tools, the
/// profile and the fixture), so it never matches a media-tool pass. A
/// still-valid recorded pass skips verification; otherwise the verifier runs,
/// a pass is offered to the cache and a failure is returned unrecorded.
///
/// # Errors
///
/// Returns the verification failure when it does not pass.
pub async fn preflight_local_asr<V, C>(
    verifier: &V,
    cache: &C,
    fingerprint: Option<&MediaToolFingerprint>,
    now_unix_seconds: u64,
) -> Result<MediaToolPreflightOutcome, LocalAsrVerificationFailure>
where
    V: LocalAsrVerifier,
    C: MediaToolVerificationCache,
{
    if let Some(fingerprint) = fingerprint
        && cache.lookup(fingerprint, now_unix_seconds) == CachedMediaToolVerification::Verified
    {
        return Ok(MediaToolPreflightOutcome::AlreadyVerified);
    }
    match verifier.verify().await {
        LocalAsrVerification::Verified => {
            Ok(MediaToolPreflightOutcome::VerifiedNow(fingerprint.map_or(
                VerificationRecord::Skipped(VerificationRecordSkip::IdentityUnavailable),
                |fingerprint| cache.record_verified(fingerprint, now_unix_seconds),
            )))
        }
        LocalAsrVerification::Failed(failure) => Err(failure),
    }
}

/// Outcome of checking a registered speech model file.
///
/// Only identity is checked. Whether the model transcribes with the selected
/// `whisper.cpp` build is proven by the local-ASR verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelVerification {
    /// Exact size and SHA-256 match this reviewed pinned model profile.
    KnownPinned(ReviewedAsrModel),
    /// The file is readable but is not a reviewed pinned model.
    Unrecognised,
    /// The file could not be read as a regular file.
    Unreadable,
}

impl ModelVerification {
    /// Returns the stable identifier used in machine-readable contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::KnownPinned(_) => "known_pinned",
            Self::Unrecognised => "unrecognised",
            Self::Unreadable => "unreadable",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use super::{
        CachedMediaToolVerification, MediaToolCheck, MediaToolFailure, MediaToolFingerprint,
        MediaToolPreflightFailure, MediaToolPreflightOutcome, MediaToolVerification,
        MediaToolVerificationCache, MediaToolVerifier, ModelVerification, VerificationRecord,
        VerificationRecordSkip, preflight_media_tools,
    };
    use vsift_domain::ReviewedAsrModel;

    struct ScriptedVerifier {
        result: MediaToolVerification,
        calls: AtomicUsize,
    }

    impl ScriptedVerifier {
        const fn new(result: MediaToolVerification) -> Self {
            Self {
                result,
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl MediaToolVerifier for ScriptedVerifier {
        fn verify(&self) -> impl Future<Output = MediaToolVerification> + Send {
            self.calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(self.result)
        }
    }

    /// An in-memory cache that honours a fixed record outcome.
    struct MemoryCache {
        passes: Mutex<Vec<(MediaToolFingerprint, u64)>>,
        record_outcome: VerificationRecord,
    }

    impl MemoryCache {
        const fn new(record_outcome: VerificationRecord) -> Self {
            Self {
                passes: Mutex::new(Vec::new()),
                record_outcome,
            }
        }

        fn recorded(&self) -> usize {
            self.passes.lock().map_or(usize::MAX, |passes| passes.len())
        }
    }

    impl MediaToolVerificationCache for MemoryCache {
        fn lookup(
            &self,
            fingerprint: &MediaToolFingerprint,
            _now_unix_seconds: u64,
        ) -> CachedMediaToolVerification {
            let known = self
                .passes
                .lock()
                .is_ok_and(|passes| passes.iter().any(|(known, _)| known == fingerprint));
            if known {
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
            if self.record_outcome == VerificationRecord::Recorded
                && let Ok(mut passes) = self.passes.lock()
            {
                passes.push((*fingerprint, now_unix_seconds));
            }
            self.record_outcome
        }
    }

    const FIRST: MediaToolFingerprint = MediaToolFingerprint::from_digest([1; 32]);
    const SECOND: MediaToolFingerprint = MediaToolFingerprint::from_digest([2; 32]);
    const PROBE_FAILURE: MediaToolVerification = MediaToolVerification::Failed {
        check: MediaToolCheck::Probe,
        failure: MediaToolFailure::UnexpectedResult,
    };

    #[tokio::test]
    async fn a_recorded_pass_is_reused_only_for_the_same_identity() {
        let verifier = ScriptedVerifier::new(MediaToolVerification::Verified);
        let cache = MemoryCache::new(VerificationRecord::Recorded);

        let miss = preflight_media_tools(&verifier, &cache, Some(&FIRST), 10).await;
        let hit = preflight_media_tools(&verifier, &cache, Some(&FIRST), 11).await;
        let other = preflight_media_tools(&verifier, &cache, Some(&SECOND), 12).await;

        assert_eq!(
            miss,
            Ok(MediaToolPreflightOutcome::VerifiedNow(
                VerificationRecord::Recorded
            ))
        );
        assert_eq!(hit, Ok(MediaToolPreflightOutcome::AlreadyVerified));
        assert_eq!(
            other,
            Ok(MediaToolPreflightOutcome::VerifiedNow(
                VerificationRecord::Recorded
            ))
        );
        assert_eq!(verifier.calls(), 2);
    }

    #[tokio::test]
    async fn a_failed_verification_is_returned_and_never_recorded() {
        let verifier = ScriptedVerifier::new(PROBE_FAILURE);
        let cache = MemoryCache::new(VerificationRecord::Recorded);

        for _ in 0..2 {
            assert_eq!(
                preflight_media_tools(&verifier, &cache, Some(&FIRST), 10).await,
                Err(MediaToolPreflightFailure {
                    check: MediaToolCheck::Probe,
                    failure: MediaToolFailure::UnexpectedResult,
                })
            );
        }
        assert_eq!(verifier.calls(), 2);
        assert_eq!(cache.recorded(), 0);
    }

    #[tokio::test]
    async fn an_unkeyed_or_unrecorded_pass_verifies_every_time() {
        let verifier = ScriptedVerifier::new(MediaToolVerification::Verified);
        let recording = MemoryCache::new(VerificationRecord::Recorded);
        let busy = MemoryCache::new(VerificationRecord::Skipped(VerificationRecordSkip::Busy));

        for _ in 0..2 {
            assert_eq!(
                preflight_media_tools(&verifier, &recording, None, 10).await,
                Ok(MediaToolPreflightOutcome::VerifiedNow(
                    VerificationRecord::Skipped(VerificationRecordSkip::IdentityUnavailable)
                ))
            );
            assert_eq!(
                preflight_media_tools(&verifier, &busy, Some(&FIRST), 10).await,
                Ok(MediaToolPreflightOutcome::VerifiedNow(
                    VerificationRecord::Skipped(VerificationRecordSkip::Busy)
                ))
            );
        }
        assert_eq!(verifier.calls(), 4);
        assert_eq!(recording.recorded(), 0);
    }

    #[test]
    fn only_a_complete_pass_counts_as_verified() {
        assert!(MediaToolVerification::Verified.is_verified());
        assert!(
            !MediaToolVerification::Failed {
                check: MediaToolCheck::Audio,
                failure: MediaToolFailure::UnexpectedResult,
            }
            .is_verified()
        );
    }

    #[test]
    fn identifiers_are_stable_snake_case() {
        assert_eq!(MediaToolCheck::Preparation.identifier(), "preparation");
        assert_eq!(
            MediaToolFailure::UnexpectedResult.identifier(),
            "unexpected_result"
        );
        assert_eq!(
            ModelVerification::KnownPinned(ReviewedAsrModel::BaseQ5_1).identifier(),
            "known_pinned"
        );
    }
}
