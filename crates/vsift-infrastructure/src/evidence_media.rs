//! The evidence-navigation ports over `FFmpeg` (P09 PR 2, ADR 0019).
//!
//! [`FfmpegFrameExtractor`] and [`FfmpegAudioExtractor`] implement the
//! application's [`FrameExtractor`] and [`AudioExtractor`] over one probed
//! source, bound for the call, with the stream the engine selected. It adds no media
//! behaviour of its own: listing, exact extraction, crops and WAV clips are
//! the P09 PR 1 adapter calls, and every failure is mapped to the ports'
//! typed errors here.

use vsift_application::{
    AudioExtractor, EvidenceMediaError, ExtractedClip, ExtractedFrame, FrameExtractor,
    VideoStreamFacts,
};
use vsift_domain::{
    CropRect, FrameDimensions, FrameListing, MediaDescription, MediaSelection, MediaTime, TimeBase,
    TimeRange,
};

use crate::{
    FfmpegMedia, MediaError, ProcessCancellation, SourceBinding, SourceError, max_frames_per_run,
};

/// The frame port over one probed video stream of a bound source.
pub struct FfmpegFrameExtractor<'a, B> {
    media: &'a FfmpegMedia<'a>,
    source: &'a B,
    description: &'a MediaDescription,
    video: VideoStreamFacts,
    cancellation: ProcessCancellation,
}

impl<'a, B: SourceBinding> FfmpegFrameExtractor<'a, B> {
    /// Adapts `media` over `source` for the video stream `stream_index`,
    /// whose orientation-correct size is `displayed`.
    ///
    /// # Errors
    ///
    /// Returns [`EvidenceMediaError::Undecodable`] when the probe holds no
    /// such stream or it has no usable time base.
    pub fn new(
        media: &'a FfmpegMedia<'a>,
        source: &'a B,
        description: &'a MediaDescription,
        stream_index: u32,
        displayed: FrameDimensions,
        cancellation: ProcessCancellation,
    ) -> Result<Self, EvidenceMediaError> {
        let stream = description
            .streams
            .iter()
            .find(|stream| stream.index == stream_index)
            .ok_or(EvidenceMediaError::Undecodable)?;
        let time_base = TimeBase::new(stream.time_base_numerator, stream.time_base_denominator)
            .map_err(|_| EvidenceMediaError::Undecodable)?;
        Ok(Self {
            media,
            source,
            description,
            video: VideoStreamFacts {
                stream_index,
                time_base,
                displayed,
                duration: description.duration,
            },
            cancellation,
        })
    }

    const fn selection(&self) -> MediaSelection {
        MediaSelection {
            video: Some(self.video.stream_index),
            audio: None,
        }
    }
}

impl<B: SourceBinding> FrameExtractor for FfmpegFrameExtractor<'_, B> {
    fn stream(&self) -> VideoStreamFacts {
        self.video
    }

    fn max_frames_per_run(&self) -> usize {
        max_frames_per_run(self.video.displayed)
    }

    async fn list_frames(&self, range: TimeRange) -> Result<FrameListing, EvidenceMediaError> {
        self.media
            .list_frame_times(
                self.source,
                self.description,
                self.selection(),
                range,
                self.cancellation.clone(),
            )
            .await
            .map_err(|error| evidence_media_error(&error))
    }

    async fn frames(&self, pts: &[i64]) -> Result<Vec<ExtractedFrame>, EvidenceMediaError> {
        let images = self
            .media
            .frames_at(
                self.source,
                self.description,
                self.selection(),
                pts,
                self.cancellation.clone(),
            )
            .await
            .map_err(|error| evidence_media_error(&error))?;
        Ok(images
            .into_iter()
            .map(|image| ExtractedFrame {
                pts: image.frame.pts,
                time: image.frame.time,
                png: image.png,
            })
            .collect())
    }

    async fn crop(&self, pts: i64, rect: CropRect) -> Result<ExtractedFrame, EvidenceMediaError> {
        let image = self
            .media
            .crop_at(
                self.source,
                self.description,
                self.selection(),
                pts,
                rect,
                self.cancellation.clone(),
            )
            .await
            .map_err(|error| evidence_media_error(&error))?;
        Ok(ExtractedFrame {
            pts: image.frame.pts,
            time: image.frame.time,
            png: image.png,
        })
    }
}

/// The audio port over one probed audio stream of a bound source.
pub struct FfmpegAudioExtractor<'a, B> {
    media: &'a FfmpegMedia<'a>,
    source: &'a B,
    description: &'a MediaDescription,
    stream_index: u32,
    cancellation: ProcessCancellation,
}

impl<'a, B: SourceBinding> FfmpegAudioExtractor<'a, B> {
    /// Adapts `media` over `source` for the audio stream `stream_index`.
    #[must_use]
    pub const fn new(
        media: &'a FfmpegMedia<'a>,
        source: &'a B,
        description: &'a MediaDescription,
        stream_index: u32,
        cancellation: ProcessCancellation,
    ) -> Self {
        Self {
            media,
            source,
            description,
            stream_index,
            cancellation,
        }
    }
}

impl<B: SourceBinding> AudioExtractor for FfmpegAudioExtractor<'_, B> {
    fn stream_index(&self) -> u32 {
        self.stream_index
    }

    fn duration(&self) -> MediaTime {
        self.description.duration
    }

    async fn clip(&self, range: TimeRange) -> Result<ExtractedClip, EvidenceMediaError> {
        let clip = self
            .media
            .wav_clip(
                self.source,
                self.description,
                MediaSelection {
                    video: None,
                    audio: Some(self.stream_index),
                },
                range,
                self.cancellation.clone(),
            )
            .await
            .map_err(|error| evidence_media_error(&error))?;
        Ok(ExtractedClip {
            actual_start: clip.actual_start,
            wav: clip.wav,
        })
    }
}

/// Maps every media failure to the evidence ports; exhaustive by design.
///
/// A changed copy is the source's integrity; a provider that could not run
/// is a missing capability; rejections, unsupported streams and malformed
/// provider output are the source's; exceeded bounds are resource limits;
/// requests the adapter rejects before any I/O are internal faults, because
/// the use cases only ask for what the probe and the listing allow.
const fn evidence_media_error(error: &MediaError) -> EvidenceMediaError {
    match error {
        MediaError::CapacityUnavailable => EvidenceMediaError::Busy,
        MediaError::Deadline => EvidenceMediaError::Deadline,
        MediaError::Cancelled => EvidenceMediaError::Cancelled,
        MediaError::Source(SourceError::SnapshotChanged) => EvidenceMediaError::SourceChanged,
        MediaError::FrameNotFound => EvidenceMediaError::FrameNotFound,
        MediaError::NoDecodedAudio => EvidenceMediaError::NoDecodedAudio,
        MediaError::OutputLimit => EvidenceMediaError::ResourceLimit,
        MediaError::ProviderRejected
        | MediaError::StreamUnavailable
        | MediaError::UnsupportedCodec
        | MediaError::InvalidDecodedOutput
        | MediaError::InvalidTimeline
        | MediaError::InvalidDimensions
        | MediaError::InvalidOrientation
        | MediaError::TimeBaseMismatch => EvidenceMediaError::Undecodable,
        MediaError::Source(_) | MediaError::Process(_) | MediaError::InvalidSourcePath => {
            EvidenceMediaError::Unavailable
        }
        MediaError::Request(_)
        | MediaError::InvalidMetadata
        | MediaError::InvalidDuration
        | MediaError::InvalidAudioRange
        | MediaError::TooManyStreams
        | MediaError::NoFrameWithinTolerance
        | MediaError::InvalidVisualWindow
        | MediaError::InvalidFrameRequest
        | MediaError::CropOutsideFrame => EvidenceMediaError::Invalid,
    }
}
