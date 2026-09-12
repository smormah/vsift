//! Provider-neutral media description and explicit stream selection.

use std::{error::Error, fmt};

use crate::{FrameDimensions, MediaTime};

/// Kind of one source stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaStreamKind {
    /// Displayed video frames.
    Video,
    /// Decoded audio samples.
    Audio,
    /// A text, subtitle, attachment, or other stream not decoded by P04.
    Other,
}

/// Whether the selected adapter has admitted this stream codec for decoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaDecodeSupport {
    /// The codec is in this adapter's reviewed decode set.
    Supported,
    /// The stream remains visible in metadata but cannot be decoded by this adapter.
    Unsupported,
}

/// Clockwise orientation applied for display.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayRotation {
    /// No rotation.
    Zero,
    /// Quarter turn clockwise.
    Clockwise90,
    /// Half turn.
    HalfTurn,
    /// Quarter turn anticlockwise.
    Clockwise270,
}

impl DisplayRotation {
    /// Returns dimensions after display orientation is applied.
    #[must_use]
    pub fn displayed_dimensions(self, encoded: FrameDimensions) -> FrameDimensions {
        match self {
            Self::Zero | Self::HalfTurn => encoded,
            Self::Clockwise90 | Self::Clockwise270 => encoded.quarter_turn(),
        }
    }
}

/// One bounded provider-neutral source stream description.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaStream {
    /// Original zero-based stream index.
    pub index: u32,
    /// Media capability provided by the stream.
    pub kind: MediaStreamKind,
    /// Provider codec identifier, recorded as evidence provenance.
    pub codec: String,
    /// Explicit decode capability for this adapter profile.
    pub decode_support: MediaDecodeSupport,
    /// Original stream time-base numerator.
    pub time_base_numerator: u32,
    /// Original stream time-base denominator.
    pub time_base_denominator: u32,
    /// Original stream start PTS if reported.
    pub start_pts: Option<i64>,
    /// Encoded dimensions for a video stream.
    pub encoded_dimensions: Option<FrameDimensions>,
    /// Display orientation for a video stream.
    pub rotation: DisplayRotation,
    /// Bounded language tag if reported.
    pub language: Option<String>,
}

/// Probe result on a normalized source presentation timeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaDescription {
    /// Positive source duration.
    pub duration: MediaTime,
    /// Original presentation origin in microseconds, preserved for provenance.
    pub origin_micros: i64,
    /// Every selected and unselected stream in original order.
    pub streams: Vec<MediaStream>,
}

/// Explicit source stream selection for one operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaSelection {
    /// Selected original video index, if visual work is requested.
    pub video: Option<u32>,
    /// Selected original audio index, if audio work is requested.
    pub audio: Option<u32>,
}

impl MediaDescription {
    /// Validates explicit indexes without silently switching tracks.
    ///
    /// # Errors
    /// Returns an absent or wrong-kind selection error.
    pub fn validate_selection(&self, selection: MediaSelection) -> Result<(), MediaSelectionError> {
        for (index, kind) in [
            (selection.video, MediaStreamKind::Video),
            (selection.audio, MediaStreamKind::Audio),
        ] {
            if let Some(index) = index
                && !self
                    .streams
                    .iter()
                    .any(|stream| stream.index == index && stream.kind == kind)
            {
                return Err(MediaSelectionError::StreamUnavailable);
            }
        }
        Ok(())
    }
}

/// An explicit stream index is missing or has the wrong kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaSelectionError {
    /// No stream of the requested kind exists at that index.
    StreamUnavailable,
}

impl fmt::Display for MediaSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("selected source stream is unavailable")
    }
}

impl Error for MediaSelectionError {}
