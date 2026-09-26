//! Evidence-navigation media operations (P09 PR 1, ADR 0019): listing a
//! stream's displayed frames, extracting exact frames and crops as PNG, and
//! decoding a WAV clip.
//!
//! Every operation is one bounded `FFmpeg` run with a closed argument list in
//! which only numbers and the private copy's path are filled in, the forced
//! local demuxer and `file` protocol, MOV external references disabled,
//! `-xerror`, a 64 MiB allocation cap and two threads (SEC-05). Frames are
//! chosen by **integer** stream timestamps (`select=eq(pts,P)`), never by
//! decimal seconds, so a frame named by an earlier listing (or a visual
//! candidate's representative time) is extracted exactly; the filter's time
//! base must equal the probed stream's or the run fails closed
//! ([`MediaError::TimeBaseMismatch`]). `FFmpeg` applies display rotation
//! before the filters named here, so sizes and crop rectangles are in
//! displayed orientation.
//!
//! These operations are not user-reachable yet: the evidence use cases and
//! commands arrive in later P09 pull requests.

use std::time::Duration;

use vsift_domain::{
    CropRect, FrameDimensions, FrameListing, ListedFrame, ListingTail, MediaDescription,
    MediaSelection, MediaStream, MediaStreamKind, MediaTime, TimeRange,
};

use super::{
    ExtractedAudio, FfmpegMedia, MAX_FRAME_BYTES, MAX_FRAME_PIXELS, MediaError, PcmProfile,
    png::{MAX_IMAGES_PER_RUN, parse_png_sequence},
    restrict_mov_references, seconds_arg,
    showinfo::{
        FrameListingWindow, MAX_LISTING_DIAGNOSTIC_BYTES, normalize, parse_frame_listing,
        scan_showinfo,
    },
};
use crate::{ProcessCancellation, ProcessRequest, SourceBinding, SourceSnapshot};

/// Longest range one frame listing covers.
pub const MAX_LISTING_RANGE_MICROS: u64 = 60_000_000;
/// Longest WAV clip: thirty seconds.
pub const MAX_WAV_CLIP_MICROS: u64 = 30_000_000;
/// Deadline of one evidence run.
const EVIDENCE_DEADLINE: Duration = Duration::from_secs(30);
/// How far before the first requested frame the input seek lands.
///
/// An input `-ss` is relative to the container's start time, which need not
/// equal the probed origin, and lands on the keyframe before it; the exact
/// frames are chosen by the `select` expression, so seeking early only costs
/// decoding. A frame the seek skipped is reported as missing, never replaced.
const EVIDENCE_SEEK_MARGIN_MICROS: u64 = 5_000_000;
/// Read margin after the last requested time.
const EVIDENCE_READ_MARGIN_MICROS: u64 = 1_000_000;
/// Standard output bound of a listing run, which writes nothing there.
const LISTING_STDOUT_BYTES: usize = 4 * 1024;
/// Bytes of a canonical 44-byte WAV header.
pub const WAV_HEADER_BYTES: usize = 44;
/// Sample rate of every evidence WAV clip.
pub const WAV_SAMPLE_RATE: u32 = 16_000;

/// What an extracted image shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageRegion {
    /// The whole displayed frame, at these dimensions.
    WholeFrame(FrameDimensions),
    /// This rectangle of the displayed frame, in displayed coordinates.
    Crop(CropRect),
}

impl ImageRegion {
    /// Returns the image's pixel dimensions.
    #[must_use]
    pub const fn dimensions(self) -> FrameDimensions {
        match self {
            Self::WholeFrame(dimensions) => dimensions,
            Self::Crop(rect) => rect.dimensions(),
        }
    }
}

/// One PNG image of one exactly identified displayed frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractedImage {
    /// Original selected video stream index.
    pub stream_index: u32,
    /// The frame's stream timestamp and normalized time, as decoded.
    pub frame: ListedFrame,
    /// The whole frame or the crop the image shows.
    pub region: ImageRegion,
    /// 8-bit RGB PNG bytes exactly as `FFmpeg` encoded them.
    pub png: Vec<u8>,
}

/// A WAV clip of one audio stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractedWav {
    /// Original selected audio stream index.
    pub stream_index: u32,
    /// Requested normalized half-open range.
    pub requested: TimeRange,
    /// Observed first decoded audio sample time on the normalized timeline.
    pub actual_start: MediaTime,
    /// A canonical WAV file: 16 kHz mono signed 16-bit PCM.
    pub wav: Vec<u8>,
}

/// Returns how many whole frames of `dimensions` one extraction run may
/// return: at most [`MAX_IMAGES_PER_RUN`], and no more than the 64 MiB
/// output bound holds at the worst-case PNG size (uncompressed 8-bit RGB
/// plus deflate and chunk overhead). At least one frame of up to
/// 16 megapixels always fits.
#[must_use]
pub fn max_frames_per_run(dimensions: FrameDimensions) -> usize {
    let worst = worst_case_png_bytes(dimensions);
    let fitting = u64::try_from(MAX_FRAME_BYTES).unwrap_or(u64::MAX) / worst.max(1);
    usize::try_from(fitting)
        .unwrap_or(usize::MAX)
        .min(MAX_IMAGES_PER_RUN)
}

/// An upper bound on an 8-bit RGB PNG's size: one filter byte and three
/// bytes per pixel per row, stored deflate blocks (5 bytes per 65,535),
/// chunk headers and a fixed allowance for the header and ancillary chunks.
fn worst_case_png_bytes(dimensions: FrameDimensions) -> u64 {
    let width = u64::from(dimensions.width());
    let height = u64::from(dimensions.height());
    let raw = height * (1 + 3 * width);
    raw + raw / 64 + 64 * 1024
}

/// Wraps 16 kHz mono signed 16-bit little-endian PCM in a canonical 44-byte
/// WAV header.
///
/// The header is written here rather than by `FFmpeg`'s WAV muxer, which
/// writes a streaming header with unknown sizes when its output is a pipe.
///
/// # Errors
///
/// Returns [`MediaError::InvalidDecodedOutput`] for an odd or empty sample
/// buffer or one whose size a WAV header cannot state.
pub fn wav_from_pcm_s16le_mono(pcm: &[u8]) -> Result<Vec<u8>, MediaError> {
    const CHANNELS: u16 = 1;
    const BITS: u16 = 16;
    if pcm.is_empty() || !pcm.len().is_multiple_of(2) {
        return Err(MediaError::InvalidDecodedOutput);
    }
    let data = u32::try_from(pcm.len()).map_err(|_| MediaError::InvalidDecodedOutput)?;
    let riff = data
        .checked_add(36)
        .ok_or(MediaError::InvalidDecodedOutput)?;
    let block_align = CHANNELS * BITS / 8;
    let byte_rate = WAV_SAMPLE_RATE * u32::from(block_align);
    let mut wav = Vec::with_capacity(WAV_HEADER_BYTES + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&CHANNELS.to_le_bytes());
    wav.extend_from_slice(&WAV_SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&BITS.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data.to_le_bytes());
    wav.extend_from_slice(pcm);
    Ok(wav)
}

impl FfmpegMedia<'_> {
    /// Lists every displayed frame of the selected video stream whose
    /// normalized time lies in `range` (clipped to the source), in one
    /// bounded decode.
    ///
    /// The run seeks five seconds early, selects frames by integer timestamp
    /// bounds computed from the probed time base and origin, logs them with
    /// `showinfo` and writes no image. At most [`vsift_domain::MAX_LISTED_FRAMES`]
    /// frames are listed; a denser range is cut before the first unlisted
    /// frame and says more may follow. See [`parse_frame_listing`].
    ///
    /// # Errors
    ///
    /// Rejects before any I/O an absent or unsupported stream, or a range
    /// that starts at or after the end of the source or is longer than
    /// [`MAX_LISTING_RANGE_MICROS`] ([`MediaError::InvalidFrameRequest`]);
    /// then fails on a changed source, a provider failure, a timeout,
    /// exceeded bounds, a time-base mismatch or malformed diagnostics.
    pub async fn list_frame_times(
        &self,
        binding: &impl SourceBinding,
        description: &MediaDescription,
        selection: MediaSelection,
        range: TimeRange,
        cancellation: ProcessCancellation,
    ) -> Result<FrameListing, MediaError> {
        let stream = evidence_video_stream(description, selection)?;
        if range.start() >= description.duration
            || range.duration_micros() > MAX_LISTING_RANGE_MICROS
        {
            return Err(MediaError::InvalidFrameRequest);
        }
        let (end, tail) = if range.end() >= description.duration {
            (description.duration, ListingTail::EndOfStream)
        } else {
            (range.end(), ListingTail::MoreMayFollow)
        };
        let covered =
            TimeRange::new(range.start(), end).map_err(|_| MediaError::InvalidFrameRequest)?;
        let window = FrameListingWindow::new(
            stream.index,
            stream.time_base_numerator,
            stream.time_base_denominator,
            description.origin_micros,
            covered,
            tail,
        )?;
        let filter = format!(
            "select='gte(pts\\,{first})*lt(pts\\,{end})',showinfo=checksum=0",
            first = window.first_pts(),
            end = window.end_pts(),
        );
        let _admission = self
            .store
            .try_admit(1)
            .map_err(|_| MediaError::CapacityUnavailable)?;
        binding
            .check_before_provider_call()
            .map_err(MediaError::Source)?;
        let source = binding.snapshot();
        let request =
            self.evidence_request(source, stream.index, covered.start(), covered.end())?;
        let request = request
            .with_arguments(["-an", "-sn", "-dn", "-vf"])
            .with_argument(filter)
            .with_arguments(["-fps_mode", "passthrough", "-frames:v"])
            .with_argument((vsift_domain::MAX_LISTED_FRAMES + 1).to_string())
            .with_arguments(["-f", "null", "-"]);
        let output = self
            .supervisor_with(LISTING_STDOUT_BYTES, MAX_LISTING_DIAGNOSTIC_BYTES)
            .run(request, cancellation)
            .await
            .map_err(MediaError::Process)?;
        super::validate_outcome(&output)?;
        parse_frame_listing(&output.stderr.bytes, &window)
    }

    /// Extracts the displayed frames with exactly these stream timestamps
    /// (from a listing) as 8-bit RGB PNG images, in one bounded run.
    ///
    /// `pts` must be strictly increasing and hold at most
    /// [`max_frames_per_run`] timestamps for the stream's displayed size.
    /// Each image is checked against the `showinfo` line of the frame it
    /// shows: the same timestamp, the displayed size, the stream's time base.
    ///
    /// # Errors
    ///
    /// Rejects an invalid batch before any I/O
    /// ([`MediaError::InvalidFrameRequest`]); a timestamp that decodes no
    /// frame is [`MediaError::FrameNotFound`]; otherwise as
    /// [`Self::list_frame_times`].
    pub async fn frames_at(
        &self,
        binding: &impl SourceBinding,
        description: &MediaDescription,
        selection: MediaSelection,
        pts: &[i64],
        cancellation: ProcessCancellation,
    ) -> Result<Vec<ExtractedImage>, MediaError> {
        let stream = evidence_video_stream(description, selection)?;
        let dimensions = displayed_dimensions(stream)?;
        if pts.len() > max_frames_per_run(dimensions) {
            return Err(MediaError::InvalidFrameRequest);
        }
        self.extract(
            binding,
            description,
            stream,
            pts,
            ImageRegion::WholeFrame(dimensions),
            cancellation,
        )
        .await
    }

    /// Extracts one rectangle of the displayed frame with this stream
    /// timestamp as an 8-bit RGB PNG image, decoding the source again and
    /// cropping in `FFmpeg` (`crop=w:h:x:y`, exact) after display rotation.
    ///
    /// # Errors
    ///
    /// Rejects a rectangle that is not contained by the stream's displayed
    /// frame before any I/O ([`MediaError::CropOutsideFrame`]); otherwise as
    /// [`Self::frames_at`].
    pub async fn crop_at(
        &self,
        binding: &impl SourceBinding,
        description: &MediaDescription,
        selection: MediaSelection,
        pts: i64,
        rect: CropRect,
        cancellation: ProcessCancellation,
    ) -> Result<ExtractedImage, MediaError> {
        let stream = evidence_video_stream(description, selection)?;
        let dimensions = displayed_dimensions(stream)?;
        CropRect::new(rect.x(), rect.y(), rect.width(), rect.height(), dimensions)
            .map_err(|_| MediaError::CropOutsideFrame)?;
        let mut images = self
            .extract(
                binding,
                description,
                stream,
                &[pts],
                ImageRegion::Crop(rect),
                cancellation,
            )
            .await?;
        match (images.pop(), images.is_empty()) {
            (Some(image), true) => Ok(image),
            _ => Err(MediaError::InvalidDecodedOutput),
        }
    }

    /// Decodes at most thirty seconds of one audio stream as a 16 kHz mono
    /// 16-bit WAV clip, reporting the first decoded sample's time through the
    /// hardened `ashowinfo` reader.
    ///
    /// # Errors
    ///
    /// Rejects a range past the source or longer than
    /// [`MAX_WAV_CLIP_MICROS`] ([`MediaError::InvalidAudioRange`]), a missing
    /// or unsupported stream, a window without decoded audio
    /// ([`MediaError::NoDecodedAudio`]), exceeded bounds or malformed output.
    pub async fn wav_clip(
        &self,
        binding: &impl SourceBinding,
        description: &MediaDescription,
        selection: MediaSelection,
        range: TimeRange,
        cancellation: ProcessCancellation,
    ) -> Result<ExtractedWav, MediaError> {
        let ExtractedAudio {
            stream_index,
            requested,
            actual_start,
            pcm_s16le,
            ..
        } = self
            .decode_pcm(
                PcmProfile::EvidenceWav,
                binding,
                description,
                selection,
                range,
                cancellation,
            )
            .await?;
        Ok(ExtractedWav {
            stream_index,
            requested,
            actual_start,
            wav: wav_from_pcm_s16le_mono(&pcm_s16le)?,
        })
    }

    /// The shared extraction run of [`Self::frames_at`] and [`Self::crop_at`].
    async fn extract(
        &self,
        binding: &impl SourceBinding,
        description: &MediaDescription,
        stream: &MediaStream,
        pts: &[i64],
        region: ImageRegion,
        cancellation: ProcessCancellation,
    ) -> Result<Vec<ExtractedImage>, MediaError> {
        if pts.is_empty()
            || pts.len() > MAX_IMAGES_PER_RUN
            || !pts.windows(2).all(|pair| matches!(pair, [a, b] if a < b))
        {
            return Err(MediaError::InvalidFrameRequest);
        }
        let time_base = (stream.time_base_numerator, stream.time_base_denominator);
        let times = pts
            .iter()
            .map(|value| normalize(stream.index, *value, time_base, description.origin_micros))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| MediaError::InvalidFrameRequest)?;
        let (Some(first), Some(last)) = (times.first(), times.last()) else {
            return Err(MediaError::InvalidFrameRequest);
        };
        if *last >= description.duration {
            return Err(MediaError::InvalidFrameRequest);
        }
        let terms: Vec<String> = pts
            .iter()
            .map(|value| format!("eq(pts\\,{value})"))
            .collect();
        let crop = match region {
            ImageRegion::WholeFrame(_) => String::new(),
            ImageRegion::Crop(rect) => format!(
                ",crop=w={}:h={}:x={}:y={}:exact=1",
                rect.width(),
                rect.height(),
                rect.x(),
                rect.y()
            ),
        };
        let filter = format!(
            "select='{}',format=rgb24{crop},showinfo=checksum=0",
            terms.join("+")
        );
        let _admission = self
            .store
            .try_admit(1)
            .map_err(|_| MediaError::CapacityUnavailable)?;
        binding
            .check_before_provider_call()
            .map_err(MediaError::Source)?;
        let source = binding.snapshot();
        let last_end = MediaTime::from_micros(last.as_micros().saturating_add(1));
        let request = self
            .evidence_request(source, stream.index, *first, last_end)?
            .with_arguments(["-an", "-sn", "-dn", "-vf"])
            .with_argument(filter)
            .with_arguments(["-fps_mode", "passthrough", "-frames:v"])
            .with_argument(pts.len().to_string())
            .with_arguments(["-f", "image2pipe", "-vcodec", "png", "pipe:1"]);
        let output = self
            .supervisor(MAX_FRAME_BYTES)
            .run(request, cancellation)
            .await
            .map_err(MediaError::Process)?;
        super::validate_outcome(&output)?;
        let dimensions = region.dimensions();
        let log = scan_showinfo(&output.stderr.bytes, MAX_IMAGES_PER_RUN)?;
        if !log.frames.is_empty() && log.time_base != Some(time_base) {
            return Err(if log.time_base.is_some() {
                MediaError::TimeBaseMismatch
            } else {
                MediaError::InvalidDecodedOutput
            });
        }
        let logged: Vec<i64> = log.frames.iter().map(|frame| frame.pts).collect();
        if logged != pts {
            // Both lists strictly increase, so fewer frames that were all
            // asked for means some timestamp named no decoded frame.
            let missing = logged.len() < pts.len()
                && logged.iter().all(|value| pts.binary_search(value).is_ok());
            return Err(if missing {
                MediaError::FrameNotFound
            } else {
                MediaError::InvalidDecodedOutput
            });
        }
        let expected_size = Some((dimensions.width(), dimensions.height()));
        if log.frames.iter().any(|frame| frame.size != expected_size) {
            return Err(MediaError::InvalidDecodedOutput);
        }
        let images = parse_png_sequence(&output.stdout.bytes, pts.len(), dimensions)?;
        Ok(pts
            .iter()
            .zip(times)
            .zip(images)
            .map(|((value, time), png)| ExtractedImage {
                stream_index: stream.index,
                frame: ListedFrame { pts: *value, time },
                region,
                png,
            })
            .collect())
    }

    /// The common head of an evidence run over `[from, to)`: the bounded
    /// flags, a seek five seconds before `from`, a read to one second after
    /// `to`, the restricted input and the stream mapping.
    fn evidence_request(
        &self,
        source: &SourceSnapshot,
        stream_index: u32,
        from: MediaTime,
        to: MediaTime,
    ) -> Result<ProcessRequest, MediaError> {
        let seek = from.as_micros().saturating_sub(EVIDENCE_SEEK_MARGIN_MICROS);
        let read = to
            .as_micros()
            .checked_sub(seek)
            .and_then(|span| span.checked_add(EVIDENCE_READ_MARGIN_MICROS))
            .ok_or(MediaError::InvalidFrameRequest)?;
        let as_argument = |micros: u64| {
            i64::try_from(micros)
                .map(seconds_arg)
                .map_err(|_| MediaError::InvalidFrameRequest)
        };
        let request = Self::request(source, self.registry.ffmpeg.clone(), EVIDENCE_DEADLINE)?
            .with_arguments([
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "info",
                "-nostats",
                "-xerror",
                "-max_alloc",
                "67108864",
                "-threads",
                "2",
                "-copyts",
                "-ss",
            ])
            .with_argument(as_argument(seek)?)
            .with_argument("-t")
            .with_argument(as_argument(read)?)
            .with_arguments(["-protocol_whitelist", "file"]);
        Ok(restrict_mov_references(request, source)
            .with_arguments(["-f", source.container().demuxer(), "-i"])
            .with_argument(source.provider_path().as_os_str())
            .with_arguments(["-map"])
            .with_argument(format!("0:{stream_index}")))
    }
}

/// The selected video stream, when the adapter decodes its codec.
fn evidence_video_stream(
    description: &MediaDescription,
    selection: MediaSelection,
) -> Result<&MediaStream, MediaError> {
    description
        .validate_selection(selection)
        .map_err(|_| MediaError::StreamUnavailable)?;
    let index = selection.video.ok_or(MediaError::StreamUnavailable)?;
    let stream = description
        .streams
        .iter()
        .find(|value| value.index == index && value.kind == MediaStreamKind::Video)
        .ok_or(MediaError::StreamUnavailable)?;
    if stream.decode_support == vsift_domain::MediaDecodeSupport::Unsupported {
        return Err(MediaError::UnsupportedCodec);
    }
    Ok(stream)
}

/// The stream's displayed dimensions, at most 16 megapixels.
fn displayed_dimensions(stream: &MediaStream) -> Result<FrameDimensions, MediaError> {
    let dimensions = stream.rotation.displayed_dimensions(
        stream
            .encoded_dimensions
            .ok_or(MediaError::InvalidDimensions)?,
    );
    if u64::from(dimensions.width()) * u64::from(dimensions.height()) > MAX_FRAME_PIXELS {
        return Err(MediaError::InvalidDimensions);
    }
    Ok(dimensions)
}

#[cfg(test)]
mod tests {
    use vsift_domain::FrameDimensions;

    use super::{
        MAX_FRAME_BYTES, WAV_HEADER_BYTES, max_frames_per_run, wav_from_pcm_s16le_mono,
        worst_case_png_bytes,
    };

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn the_run_budget_always_fits_one_frame_and_at_most_eight() -> TestResult {
        for (width, height, expected) in [
            (1_280, 720, 8),
            (1_920, 1_080, 8),
            (2_560, 1_440, 5),
            (3_840, 2_160, 2),
            (4_000, 4_000, 1),
        ] {
            let dimensions = FrameDimensions::new(width, height)?;
            assert_eq!(max_frames_per_run(dimensions), expected, "{width}x{height}");
            let fitting = u64::try_from(expected)?;
            assert!(worst_case_png_bytes(dimensions) * fitting <= u64::try_from(MAX_FRAME_BYTES)?);
        }
        Ok(())
    }

    #[test]
    fn wav_clips_carry_a_canonical_header() -> TestResult {
        let wav = wav_from_pcm_s16le_mono(&[1, 0, 255, 127])?;
        assert_eq!(wav.len(), WAV_HEADER_BYTES + 4);
        let mut expected = b"RIFF".to_vec();
        expected.extend_from_slice(&40_u32.to_le_bytes());
        expected.extend_from_slice(b"WAVEfmt ");
        expected.extend_from_slice(&[16, 0, 0, 0, 1, 0, 1, 0]);
        expected.extend_from_slice(&16_000_u32.to_le_bytes());
        expected.extend_from_slice(&32_000_u32.to_le_bytes());
        expected.extend_from_slice(&[2, 0, 16, 0]);
        expected.extend_from_slice(b"data");
        expected.extend_from_slice(&4_u32.to_le_bytes());
        expected.extend_from_slice(&[1, 0, 255, 127]);
        assert_eq!(wav, expected);
        assert!(wav_from_pcm_s16le_mono(&[]).is_err());
        assert!(wav_from_pcm_s16le_mono(&[1, 2, 3]).is_err());
        Ok(())
    }
}
