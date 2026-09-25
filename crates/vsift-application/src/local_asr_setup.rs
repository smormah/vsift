//! What `setup check` reports about local speech recognition (maintainer
//! decision D4, P07 increment 3c).
//!
//! The executable probes of `setup check` only show that whisper.cpp responds.
//! These values describe the stronger facts a host needs before it asks for
//! `transcript retranscribe`: whether the registered model is a reviewed pinned
//! profile, and whether the selected recognizer, model and media tools passed
//! the local-ASR functional verification (ADR 0017), from a recorded pass or a
//! run made by the check itself. Every value is typed; presentation lives in
//! the contract crate.

use vsift_domain::ReviewedAsrModel;

use crate::{LocalAsrVerificationFailure, ModelVerification};

/// What the registered speech model is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalAsrModelStatus {
    /// No model is registered with `setup configure-model`.
    NotSelected,
    /// The registered path cannot be read as a regular file.
    Unreadable,
    /// The file is readable but matches no reviewed pinned model.
    Unrecognised,
    /// The file is this reviewed pinned profile.
    KnownPinned(ReviewedAsrModel),
}

impl LocalAsrModelStatus {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::NotSelected => "not_selected",
            Self::Unreadable => "unreadable",
            Self::Unrecognised => "unrecognised",
            Self::KnownPinned(_) => "known_pinned",
        }
    }

    /// The pinned profile, when the model is one.
    #[must_use]
    pub const fn profile(self) -> Option<ReviewedAsrModel> {
        match self {
            Self::KnownPinned(profile) => Some(profile),
            Self::NotSelected | Self::Unreadable | Self::Unrecognised => None,
        }
    }
}

impl From<ModelVerification> for LocalAsrModelStatus {
    fn from(verification: ModelVerification) -> Self {
        match verification {
            ModelVerification::KnownPinned(profile) => Self::KnownPinned(profile),
            ModelVerification::Unrecognised => Self::Unrecognised,
            ModelVerification::Unreadable => Self::Unreadable,
        }
    }
}

/// Why the local-ASR verification was not attempted.
///
/// Reasons are checked in the order a retranscription resolves its
/// dependencies, and the first that applies is reported: media tools, then
/// whisper.cpp, then the model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalAsrNotRunReason {
    /// `FFmpeg` or `FFprobe` is missing, rejected, or failed its own
    /// media-tool verification; the speech clip cannot be decoded.
    MediaToolsUnavailable,
    /// No whisper.cpp CLI is selected or found, or the selection is rejected.
    WhisperUnavailable,
    /// No model is registered.
    ModelNotSelected,
    /// The registered model is unreadable or not a reviewed pinned profile,
    /// so no retranscription would run it.
    ModelNotPinned,
}

impl LocalAsrNotRunReason {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::MediaToolsUnavailable => "media_tools_unavailable",
            Self::WhisperUnavailable => "whisper_unavailable",
            Self::ModelNotSelected => "model_not_selected",
            Self::ModelNotPinned => "model_not_pinned",
        }
    }
}

/// Where a passing local-ASR verification came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalAsrVerificationSource {
    /// A still-valid pass for exactly this setup was already recorded; nothing
    /// ran.
    Recorded,
    /// The check ran the verification now, and it passed.
    RanNow,
}

impl LocalAsrVerificationSource {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Recorded => "recorded",
            Self::RanNow => "ran_now",
        }
    }
}

/// Why a verification run by `setup check` did not pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalAsrCheckFailure {
    /// The verification ran to a typed failure.
    Verification(LocalAsrVerificationFailure),
    /// The check's own time budget ended before the verification did; it was
    /// cancelled and nothing was recorded.
    BudgetExceeded,
}

impl LocalAsrCheckFailure {
    /// Stable identifier of the step that did not pass: `preparation`,
    /// `fixture_probe`, a transcription stage (`planning`,
    /// `recognizer_identity`, `audio_extraction`, `recognition`,
    /// `output_validation`, `assembly`), `transcript` or `budget`.
    #[must_use]
    pub const fn check(self) -> &'static str {
        match self {
            Self::Verification(
                LocalAsrVerificationFailure::FixtureIntegrity
                | LocalAsrVerificationFailure::Workspace,
            ) => "preparation",
            Self::Verification(LocalAsrVerificationFailure::FixtureMedia) => "fixture_probe",
            Self::Verification(LocalAsrVerificationFailure::Transcription(failure)) => {
                failure.stage.identifier()
            }
            Self::Verification(LocalAsrVerificationFailure::UnexpectedTranscript) => "transcript",
            Self::BudgetExceeded => "budget",
        }
    }

    /// Stable identifier of why it did not pass: the verification failure
    /// kind, the transcription failure reason, or `budget_exceeded`.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Verification(LocalAsrVerificationFailure::Transcription(failure)) => {
                failure.reason.identifier()
            }
            Self::Verification(failure) => failure.identifier(),
            Self::BudgetExceeded => "budget_exceeded",
        }
    }
}

/// The outcome of the local-ASR verification in `setup check`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalAsrCheckOutcome {
    /// The selected recognizer, model and media tools transcribe the reviewed
    /// speech clip on this machine.
    Verified(LocalAsrVerificationSource),
    /// The verification ran and did not pass.
    Failed(LocalAsrCheckFailure),
    /// The verification could not be attempted.
    NotRun(LocalAsrNotRunReason),
}

impl LocalAsrCheckOutcome {
    /// Stable machine-readable status: `verified`, `failed` or `not_run`.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Verified(_) => "verified",
            Self::Failed(_) => "failed",
            Self::NotRun(_) => "not_run",
        }
    }
}

/// Everything `setup check` reports about local speech recognition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalAsrSetupStatus {
    /// What the registered model is.
    pub model: LocalAsrModelStatus,
    /// Whether local ASR passed its functional verification.
    pub verification: LocalAsrCheckOutcome,
}

#[cfg(test)]
mod tests {
    use vsift_domain::{ProviderOutputError, ReviewedAsrModel};

    use super::{
        LocalAsrCheckFailure, LocalAsrCheckOutcome, LocalAsrModelStatus, LocalAsrNotRunReason,
        LocalAsrVerificationSource,
    };
    use crate::{
        AsrFailure, AsrFailureReason, AsrStage, LocalAsrVerificationFailure, ModelVerification,
    };

    #[test]
    fn model_status_follows_identification_and_names_only_pinned_profiles() {
        for (verification, status, profile) in [
            (
                ModelVerification::KnownPinned(ReviewedAsrModel::Base),
                "known_pinned",
                Some(ReviewedAsrModel::Base),
            ),
            (
                ModelVerification::KnownPinned(ReviewedAsrModel::BaseQ5_1),
                "known_pinned",
                Some(ReviewedAsrModel::BaseQ5_1),
            ),
            (ModelVerification::Unrecognised, "unrecognised", None),
            (ModelVerification::Unreadable, "unreadable", None),
        ] {
            let model = LocalAsrModelStatus::from(verification);
            assert_eq!(model.identifier(), status);
            assert_eq!(model.profile(), profile);
        }
        assert_eq!(
            LocalAsrModelStatus::NotSelected.identifier(),
            "not_selected"
        );
        assert_eq!(LocalAsrModelStatus::NotSelected.profile(), None);
    }

    #[test]
    fn every_failure_names_a_check_and_a_reason() {
        let transcription = LocalAsrCheckFailure::Verification(
            LocalAsrVerificationFailure::Transcription(AsrFailure {
                stage: AsrStage::OutputValidation,
                reason: AsrFailureReason::MalformedOutput(ProviderOutputError::TooManySegments),
            }),
        );
        for (failure, check, reason) in [
            (
                LocalAsrCheckFailure::Verification(LocalAsrVerificationFailure::FixtureIntegrity),
                "preparation",
                "fixture_integrity",
            ),
            (
                LocalAsrCheckFailure::Verification(LocalAsrVerificationFailure::Workspace),
                "preparation",
                "workspace",
            ),
            (
                LocalAsrCheckFailure::Verification(LocalAsrVerificationFailure::FixtureMedia),
                "fixture_probe",
                "fixture_media",
            ),
            (transcription, "output_validation", "malformed_output"),
            (
                LocalAsrCheckFailure::Verification(
                    LocalAsrVerificationFailure::UnexpectedTranscript,
                ),
                "transcript",
                "unexpected_transcript",
            ),
            (
                LocalAsrCheckFailure::BudgetExceeded,
                "budget",
                "budget_exceeded",
            ),
        ] {
            assert_eq!((failure.check(), failure.reason()), (check, reason));
            assert_eq!(LocalAsrCheckOutcome::Failed(failure).identifier(), "failed");
        }
    }

    #[test]
    fn outcomes_and_reasons_have_stable_identifiers() {
        assert_eq!(
            LocalAsrCheckOutcome::Verified(LocalAsrVerificationSource::Recorded).identifier(),
            "verified"
        );
        assert_eq!(LocalAsrVerificationSource::RanNow.identifier(), "ran_now");
        for (reason, identifier) in [
            (
                LocalAsrNotRunReason::MediaToolsUnavailable,
                "media_tools_unavailable",
            ),
            (
                LocalAsrNotRunReason::WhisperUnavailable,
                "whisper_unavailable",
            ),
            (LocalAsrNotRunReason::ModelNotSelected, "model_not_selected"),
            (LocalAsrNotRunReason::ModelNotPinned, "model_not_pinned"),
        ] {
            assert_eq!(reason.identifier(), identifier);
            assert_eq!(LocalAsrCheckOutcome::NotRun(reason).identifier(), "not_run");
        }
    }
}
