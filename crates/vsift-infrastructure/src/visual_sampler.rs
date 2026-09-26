//! The [`VisualSampler`] port over the restricted `FFmpeg` adapter.
//!
//! Each window is decoded by [`FfmpegMedia::visual_samples`] with its fixed
//! arguments, admission, deadline and bounds; this adapter only reduces the
//! decoded grey frames to domain samples and maps failures to the port's
//! typed errors. A visual index decodes one window after another from the
//! same copy, so the adapter reads a [`BoundSource`] (issue #148).

use vsift_application::{VisualSampler, VisualSamplingError};
use vsift_domain::{MediaDescription, MediaSelection, VisualSample, VisualWindow};

use crate::{BoundSource, FfmpegMedia, MediaError, ProcessCancellation};

/// Decodes visual windows of one bound source's selected video stream.
pub struct FfmpegVisualSampler<'a> {
    media: &'a FfmpegMedia<'a>,
    source: &'a BoundSource,
    description: &'a MediaDescription,
    selection: MediaSelection,
    cancellation: ProcessCancellation,
}

impl<'a> FfmpegVisualSampler<'a> {
    /// Decodes from `source`, whose probed `description` validates `selection`.
    #[must_use]
    pub const fn new(
        media: &'a FfmpegMedia<'a>,
        source: &'a BoundSource,
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

impl VisualSampler for FfmpegVisualSampler<'_> {
    async fn window_samples(
        &self,
        window: VisualWindow,
    ) -> Result<Vec<VisualSample>, VisualSamplingError> {
        let frames = self
            .media
            .visual_samples(
                self.source,
                self.description,
                self.selection,
                window,
                self.cancellation.clone(),
            )
            .await
            .map_err(|error| visual_sampling_error(&error))?;
        Ok(frames
            .iter()
            .map(|frame| VisualSample::from_gray(frame.time, &frame.pixels))
            .collect())
    }
}

/// Maps every media failure to the visual-sampling port; exhaustive by design.
///
/// Rejections, unsupported streams, output beyond its bounds and malformed
/// provider output are recorded as an undecodable window: the same tools
/// would fail the same way again. A changed source, a provider that cannot
/// run and adapter faults fail the whole call, so nothing is committed.
const fn visual_sampling_error(error: &MediaError) -> VisualSamplingError {
    match error {
        MediaError::CapacityUnavailable => VisualSamplingError::Busy,
        MediaError::Deadline => VisualSamplingError::Deadline,
        MediaError::Cancelled => VisualSamplingError::Cancelled,
        MediaError::ProviderRejected
        | MediaError::StreamUnavailable
        | MediaError::UnsupportedCodec
        | MediaError::OutputLimit
        | MediaError::InvalidDecodedOutput
        | MediaError::InvalidTimeline => VisualSamplingError::Undecodable,
        MediaError::Source(_)
        | MediaError::Request(_)
        | MediaError::Process(_)
        | MediaError::InvalidSourcePath
        | MediaError::InvalidMetadata
        | MediaError::InvalidDuration
        | MediaError::InvalidAudioRange
        | MediaError::TooManyStreams
        | MediaError::InvalidDimensions
        | MediaError::InvalidOrientation
        | MediaError::NoFrameWithinTolerance
        | MediaError::NoDecodedAudio
        | MediaError::InvalidVisualWindow => VisualSamplingError::Io,
    }
}
