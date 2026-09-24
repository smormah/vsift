//! The [`SpeechAudioSource`] port over the restricted `FFmpeg` adapter.
//!
//! Each planned chunk is decoded by [`FfmpegMedia::speech_pcm`] with its fixed
//! arguments, admission, deadline and bounds; this adapter only converts the
//! little-endian bytes to samples and maps failures to the port's typed errors.

use vsift_application::{SpeechAudioError, SpeechAudioSource, SpeechPcm};
use vsift_domain::{MediaDescription, MediaSelection, PlannedChunk};

use crate::{FfmpegMedia, MediaError, ProcessCancellation, SourceSnapshot};

/// Decodes speech chunks of one held source snapshot's selected audio stream.
pub struct FfmpegSpeechAudio<'a> {
    media: &'a FfmpegMedia<'a>,
    source: &'a SourceSnapshot,
    description: &'a MediaDescription,
    selection: MediaSelection,
    cancellation: ProcessCancellation,
}

impl<'a> FfmpegSpeechAudio<'a> {
    /// Decodes from `source`, whose probed `description` validates `selection`.
    #[must_use]
    pub const fn new(
        media: &'a FfmpegMedia<'a>,
        source: &'a SourceSnapshot,
        description: &'a MediaDescription,
        selection: MediaSelection,
        cancellation: ProcessCancellation,
    ) -> Self {
        Self {
            media,
            source,
            description,
            selection,
            cancellation,
        }
    }
}

impl SpeechAudioSource for FfmpegSpeechAudio<'_> {
    async fn speech_pcm(&self, chunk: &PlannedChunk) -> Result<SpeechPcm, SpeechAudioError> {
        let audio = self
            .media
            .speech_pcm(
                self.source,
                self.description,
                self.selection,
                chunk.window(),
                self.cancellation.clone(),
            )
            .await
            .map_err(|error| speech_audio_error(&error))?;
        Ok(SpeechPcm {
            actual_start: audio.actual_start,
            samples: audio
                .pcm_s16le
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| i16::from_le_bytes(*pair))
                .collect(),
        })
    }
}

/// Maps every media failure to the speech-audio port; exhaustive by design.
const fn speech_audio_error(error: &MediaError) -> SpeechAudioError {
    match error {
        MediaError::NoDecodedAudio => SpeechAudioError::NoAudio,
        MediaError::CapacityUnavailable => SpeechAudioError::Busy,
        MediaError::Deadline => SpeechAudioError::Deadline,
        MediaError::Cancelled => SpeechAudioError::Cancelled,
        MediaError::OutputLimit => SpeechAudioError::ResourceLimit,
        MediaError::StreamUnavailable
        | MediaError::UnsupportedCodec
        | MediaError::InvalidAudioRange
        | MediaError::ProviderRejected => SpeechAudioError::Unavailable,
        MediaError::Source(_)
        | MediaError::Request(_)
        | MediaError::Process(_)
        | MediaError::InvalidSourcePath
        | MediaError::InvalidMetadata
        | MediaError::InvalidDuration
        | MediaError::TooManyStreams
        | MediaError::InvalidDimensions
        | MediaError::InvalidOrientation
        | MediaError::InvalidTimeline
        | MediaError::NoFrameWithinTolerance
        | MediaError::InvalidDecodedOutput => SpeechAudioError::Io,
    }
}
