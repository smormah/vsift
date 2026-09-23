//! Verification that selected specialist tools work, not merely that they exist.
//!
//! `setup check` only proves an executable responds to a version probe. The
//! types here describe a stronger result: the selected tools processed a
//! reviewed fixture through `VSift`'s real media pipeline and produced its
//! recorded truth. Hosts call the port directly; a preflight will call it
//! before the first media stage (ADR 0015, P06).

use std::future::Future;

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
    /// Probe, frame and audio results all matched the fixture's recorded truth.
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

/// Outcome of checking a registered speech model file.
///
/// Only identity is checked. Whether the model transcribes with the selected
/// `whisper.cpp` build is proven once P07 supplies the transcription adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelVerification {
    /// Exact size and SHA-256 match a reviewed pinned model.
    KnownPinned,
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
            Self::KnownPinned => "known_pinned",
            Self::Unrecognised => "unrecognised",
            Self::Unreadable => "unreadable",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MediaToolCheck, MediaToolFailure, MediaToolVerification, ModelVerification};

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
        assert_eq!(ModelVerification::KnownPinned.identifier(), "known_pinned");
    }
}
