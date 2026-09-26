//! Evidence metadata that preserves uncertainty and provider provenance, the
//! pure navigation rules that choose which frames a request names, and the
//! evidence items and lineage records of P09.

use std::{error::Error, fmt};

mod navigation;
mod record;

pub use navigation::{
    BurstCount, BurstExtent, BurstPlan, BurstRange, FrameListing, FrameSelection,
    FrameSelectionError, FrameTolerance, ListedFrame, ListingTail, MAX_BURST_FRAMES,
    MAX_BURST_RANGE_MICROS, MAX_FRAME_TOLERANCE_MICROS, MAX_LISTED_FRAMES, MAX_NEIGHBOUR_COUNT,
    NavigationError, NeighbourCount, NeighbourPlan, NeighbourStop, SelectedFrame, plan_burst,
    plan_neighbours, select_frame,
};
pub use record::{
    AUDIO_CLIP_SAMPLE_RATE, AudioRange, CropRegion, EvidenceDetail, EvidenceItem,
    EvidenceItemParts, EvidenceMedia, EvidenceMediaKind, EvidenceOperation, EvidenceProfile,
    EvidenceRecord, EvidenceRecordError, EvidenceRecordParts, EvidenceRequest, EvidenceSelection,
    EvidenceSubject, FrameRef, MAX_AUDIO_CLIP_MICROS, MAX_RECORD_ITEMS, MAX_RECORD_SELECTIONS,
    PartialReason, SelectionRole, SourceCheck, TimeBase,
};

const MAX_SPEAKER_LABEL_BYTES: usize = 128;

/// Origin and interpretation of a reported confidence value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfidenceOrigin {
    /// The provider did not supply a confidence value.
    Unavailable,
    /// The provider supplied a score that is not a calibrated probability.
    ProviderUncalibrated,
    /// A named provider contract defines the score as calibrated.
    ProviderCalibrated,
}

/// A provider confidence value represented in basis points when known.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Confidence {
    basis_points: Option<u16>,
    origin: ConfidenceOrigin,
}

impl Confidence {
    /// Records that no numeric confidence was available.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            basis_points: None,
            origin: ConfidenceOrigin::Unavailable,
        }
    }

    /// Records a provider score without manufacturing probability semantics.
    ///
    /// # Errors
    ///
    /// Returns [`ConfidenceError`] for an out-of-range score or missing semantics.
    pub fn provider_score(
        basis_points: u16,
        origin: ConfidenceOrigin,
    ) -> Result<Self, ConfidenceError> {
        if basis_points > 10_000 {
            return Err(ConfidenceError::OutsideRange);
        }
        if origin == ConfidenceOrigin::Unavailable {
            return Err(ConfidenceError::MissingOrigin);
        }
        Ok(Self {
            basis_points: Some(basis_points),
            origin,
        })
    }

    /// Returns the score in basis points, or `None` when the provider supplied none.
    #[must_use]
    pub const fn basis_points(self) -> Option<u16> {
        self.basis_points
    }

    /// Returns the score's declared semantics.
    #[must_use]
    pub const fn origin(self) -> ConfidenceOrigin {
        self.origin
    }
}

/// Why confidence metadata was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfidenceError {
    /// Basis points must be between zero and ten thousand.
    OutsideRange,
    /// A numeric score requires an explicit provider interpretation.
    MissingOrigin,
}

impl fmt::Display for ConfidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::OutsideRange => "confidence score is outside the basis-point range",
            Self::MissingOrigin => "numeric confidence requires an explicit origin",
        };
        formatter.write_str(message)
    }
}

impl Error for ConfidenceError {}

/// Provider-issued speaker label that is not a verified human identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpeakerLabel(String);

impl SpeakerLabel {
    /// Validates a bounded display label without control characters.
    ///
    /// # Errors
    ///
    /// Returns [`SpeakerLabelError`] when length or characters violate the contract.
    pub fn parse(value: impl Into<String>) -> Result<Self, SpeakerLabelError> {
        let value = value.into();
        if value.trim().is_empty() || value.len() > MAX_SPEAKER_LABEL_BYTES {
            return Err(SpeakerLabelError::InvalidLength);
        }
        if value.chars().any(char::is_control) {
            return Err(SpeakerLabelError::ControlCharacter);
        }
        Ok(Self(value))
    }

    /// Returns the provider label exactly as validated.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why a provider speaker label was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeakerLabelError {
    /// The label is empty or exceeds the display budget.
    InvalidLength,
    /// The label contains a terminal or line control character.
    ControlCharacter,
}

impl fmt::Display for SpeakerLabelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidLength => "speaker label length is invalid",
            Self::ControlCharacter => "speaker label contains a control character",
        };
        formatter.write_str(message)
    }
}

impl Error for SpeakerLabelError {}

#[cfg(test)]
mod tests {
    use super::{Confidence, ConfidenceOrigin, SpeakerLabel};

    #[test]
    fn unknown_confidence_remains_null_with_explicit_origin() {
        let confidence = Confidence::unknown();

        assert_eq!(confidence.basis_points(), None);
        assert_eq!(confidence.origin(), ConfidenceOrigin::Unavailable);
    }

    #[test]
    fn provider_score_cannot_claim_missing_semantics() {
        assert!(Confidence::provider_score(5_000, ConfidenceOrigin::Unavailable).is_err());
        assert!(Confidence::provider_score(10_001, ConfidenceOrigin::ProviderCalibrated).is_err());
        assert!(Confidence::provider_score(8_250, ConfidenceOrigin::ProviderUncalibrated).is_ok());
    }

    #[test]
    fn speaker_label_is_bounded_provider_metadata() {
        assert!(SpeakerLabel::parse("speaker-1").is_ok());
        assert!(SpeakerLabel::parse("speaker\u{1b}[31m").is_err());
        assert!(SpeakerLabel::parse(" ").is_err());
    }
}
