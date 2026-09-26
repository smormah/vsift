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
    /// The audio stream local speech recognition transcribes: the first
    /// audio stream, in original order, whose codec this adapter decodes.
    ///
    /// R0 never mixes or chooses between several spoken tracks; the first is
    /// the one players default to. `None` means the source has no usable
    /// audio, which a caller reports rather than transcribing silence.
    #[must_use]
    pub fn speech_audio_stream(&self) -> Option<u32> {
        self.streams
            .iter()
            .find(|stream| {
                stream.kind == MediaStreamKind::Audio
                    && stream.decode_support == MediaDecodeSupport::Supported
            })
            .map(|stream| stream.index)
    }

    /// The video stream visual analysis samples: the first video stream, in
    /// original order, whose codec this adapter decodes, with its
    /// orientation-correct displayed dimensions.
    ///
    /// As for speech, R0 never chooses between several video tracks; the
    /// first decodable one is the one players show.
    ///
    /// # Errors
    ///
    /// Returns [`VisualStreamError::Absent`] when the source has no video
    /// stream at all (an audio-only file: the caller asked for something the
    /// source cannot have), and [`VisualStreamError::Unsupported`] when every
    /// video stream uses a codec the adapter does not decode, or reports no
    /// dimensions (the source is the problem).
    pub fn visual_video_stream(&self) -> Result<(u32, FrameDimensions), VisualStreamError> {
        let mut video = self
            .streams
            .iter()
            .filter(|stream| stream.kind == MediaStreamKind::Video)
            .peekable();
        if video.peek().is_none() {
            return Err(VisualStreamError::Absent);
        }
        video
            .find(|stream| stream.decode_support == MediaDecodeSupport::Supported)
            .and_then(|stream| {
                stream
                    .encoded_dimensions
                    .map(|encoded| (stream.index, stream.rotation.displayed_dimensions(encoded)))
            })
            .ok_or(VisualStreamError::Unsupported)
    }

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

/// Why a source has no video stream visual analysis can sample.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualStreamError {
    /// The source has no video stream.
    Absent,
    /// No video stream has a codec and dimensions the adapter decodes.
    Unsupported,
}

impl fmt::Display for VisualStreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Absent => "the source has no video stream",
            Self::Unsupported => "the source has no video stream the adapter decodes",
        })
    }
}

impl Error for VisualStreamError {}

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

#[cfg(test)]
mod tests {
    use super::{
        DisplayRotation, MediaDecodeSupport, MediaDescription, MediaStream, MediaStreamKind,
        VisualStreamError,
    };
    use crate::{FrameDimensions, MediaTime};

    type Built<T> = Result<T, Box<dyn std::error::Error>>;

    fn stream(
        index: u32,
        kind: MediaStreamKind,
        support: MediaDecodeSupport,
        rotation: DisplayRotation,
    ) -> Built<MediaStream> {
        Ok(MediaStream {
            index,
            kind,
            codec: String::from("h264"),
            decode_support: support,
            time_base_numerator: 1,
            time_base_denominator: 1_000,
            start_pts: None,
            encoded_dimensions: (kind == MediaStreamKind::Video)
                .then(|| FrameDimensions::new(1280, 720))
                .transpose()?,
            rotation,
            language: None,
        })
    }

    fn description(streams: Vec<MediaStream>) -> MediaDescription {
        MediaDescription {
            duration: MediaTime::from_micros(1_000_000),
            origin_micros: 0,
            streams,
        }
    }

    #[test]
    fn the_first_decodable_video_stream_is_sampled_with_displayed_dimensions() -> Built<()> {
        let source = description(vec![
            stream(
                0,
                MediaStreamKind::Audio,
                MediaDecodeSupport::Supported,
                DisplayRotation::Zero,
            )?,
            stream(
                1,
                MediaStreamKind::Video,
                MediaDecodeSupport::Unsupported,
                DisplayRotation::Zero,
            )?,
            stream(
                2,
                MediaStreamKind::Video,
                MediaDecodeSupport::Supported,
                DisplayRotation::Clockwise90,
            )?,
        ]);
        assert_eq!(
            source.visual_video_stream(),
            Ok((2, FrameDimensions::new(720, 1280)?))
        );
        Ok(())
    }

    #[test]
    fn audio_only_and_undecodable_video_are_told_apart() -> Built<()> {
        let audio_only = description(vec![stream(
            0,
            MediaStreamKind::Audio,
            MediaDecodeSupport::Supported,
            DisplayRotation::Zero,
        )?]);
        assert_eq!(
            audio_only.visual_video_stream(),
            Err(VisualStreamError::Absent)
        );
        let undecodable = description(vec![stream(
            0,
            MediaStreamKind::Video,
            MediaDecodeSupport::Unsupported,
            DisplayRotation::Zero,
        )?]);
        assert_eq!(
            undecodable.visual_video_stream(),
            Err(VisualStreamError::Unsupported)
        );
        Ok(())
    }
}
