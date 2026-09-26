//! Restricted FFprobe/FFmpeg adapter over the trusted process supervisor.

use std::{error::Error, fmt, num::NonZeroUsize, time::Duration};

use serde::Deserialize;
use vsift_domain::{
    DisplayRotation, FrameDimensions, FrameTiming, MAX_WINDOW_SAMPLES, MediaDecodeSupport,
    MediaDescription, MediaSelection, MediaStream, MediaStreamKind, MediaTime, StreamTime,
    TimeRange, VISUAL_FRAME_BYTES, VISUAL_FRAME_HEIGHT, VISUAL_FRAME_WIDTH,
    VISUAL_SAMPLE_INTERVAL_MICROS, VisualWindow,
};

mod evidence;
mod png;
mod showinfo;

pub use evidence::{
    ExtractedImage, ExtractedWav, ImageRegion, MAX_LISTING_RANGE_MICROS, MAX_WAV_CLIP_MICROS,
    WAV_HEADER_BYTES, WAV_SAMPLE_RATE, max_frames_per_run, wav_from_pcm_s16le_mono,
};
pub use png::{MAX_IMAGES_PER_RUN, parse_png_sequence};
pub use showinfo::{
    FrameListingWindow, MAX_LISTING_DIAGNOSTIC_BYTES, ObservedFrameTime, parse_ashowinfo_start,
    parse_frame_listing, parse_frame_showinfo,
};

use crate::{
    BoundSource, FilesystemSessionStore, HostIsolation, ProcessCancellation, ProcessError,
    ProcessOutcome, ProcessRequest, ProcessRequestError, ProcessSupervisor,
    ProcessWorkingDirectory, SourceBinding, SourceError, SourceSnapshot, SupervisorPolicy,
    TerminationReason, TrustedExecutable,
};

/// Hard bound for structured probe output.
pub const MAX_PROBE_BYTES: usize = 4 * 1024 * 1024;
/// Hard bound for provider diagnostics.
pub const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;
/// Hard bound for one in-memory image result.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
/// Hard bound for one extracted PCM chunk.
pub const MAX_AUDIO_BYTES: usize = 4 * 1024 * 1024;
/// Hard bound for one speech-recognition PCM chunk: thirty seconds of mono
/// 16 kHz signed 16-bit audio is 960,000 bytes.
pub const MAX_SPEECH_PCM_BYTES: usize = 1024 * 1024;
/// Longest window [`FfmpegMedia::speech_pcm`] decodes.
pub const MAX_SPEECH_PCM_MICROS: u64 = 30_000_000;
/// Largest clip [`FfmpegMedia::audio`] decodes: ten seconds at 16 kHz mono.
const MAX_CLIP_PCM_BYTES: usize = 320_000;
/// Hard bound for one visual window's decoded samples: at most
/// [`MAX_WINDOW_SAMPLES`] grey 128x72 frames.
pub const MAX_VISUAL_SAMPLE_BYTES: usize = MAX_WINDOW_SAMPLES * VISUAL_FRAME_BYTES;
/// Hard bound for one visual window's diagnostics. `showinfo` logs two or
/// three lines (about 450 bytes) per sample and keyframes add side-data
/// lines, so the general 64 KiB diagnostic bound is too small for 122
/// samples; 256 KiB is still a hard, small cap.
pub const MAX_VISUAL_DIAGNOSTIC_BYTES: usize = 256 * 1024;
/// Deadline for decoding one 60 s visual window.
const VISUAL_WINDOW_DEADLINE: Duration = Duration::from_secs(120);
/// Margin of the input seek and read length around a visual window.
///
/// `FFmpeg` interprets an input `-ss` relative to the container's start time,
/// which is not guaranteed to equal the probed origin (the earliest stream
/// start), and `-t` counts from the seek point. Seeking one second early and
/// reading one second late costs little and leaves the exact bounds to the
/// `select` expression, which compares raw stream timestamps.
const VISUAL_SEEK_MARGIN_MICROS: u64 = 1_000_000;
const MAX_STREAMS: usize = 32;
const MAX_DURATION_MICROS: u64 = 4 * 60 * 60 * 1_000_000;
const MAX_FRAME_PIXELS: u64 = 16_000_000;

/// Executable identity and the fixed P04 conformance profile.
#[derive(Clone, Debug)]
pub struct MediaProviderConformance {
    /// Trusted, canonical `FFmpeg` executable.
    pub ffmpeg: TrustedExecutable,
    /// Trusted, canonical `FFprobe` executable.
    pub ffprobe: TrustedExecutable,
    /// Fixed adapter profile version, included in future operation keys.
    pub profile_version: &'static str,
    /// Accepted input demuxers.
    pub demuxers: &'static [&'static str],
    /// Input protocols allowed for a verified private snapshot.
    pub protocols: &'static [&'static str],
}

impl MediaProviderConformance {
    /// Constructs the P04 adapter registry entry from already trusted executables.
    #[must_use]
    pub const fn r0(ffmpeg: TrustedExecutable, ffprobe: TrustedExecutable) -> Self {
        Self {
            ffmpeg,
            ffprobe,
            profile_version: "p04-r0-v1",
            demuxers: &["mov", "matroska"],
            protocols: &["file"],
        }
    }
}

/// Provider-owned media operations; consumers never construct `FFmpeg` flags.
pub struct FfmpegMedia<'a> {
    registry: MediaProviderConformance,
    host_isolation: HostIsolation,
    store: &'a FilesystemSessionStore,
}

impl<'a> FfmpegMedia<'a> {
    /// Creates a media adapter with explicit executable provenance and effective host controls.
    #[must_use]
    pub const fn new(
        registry: MediaProviderConformance,
        host_isolation: HostIsolation,
        store: &'a FilesystemSessionStore,
    ) -> Self {
        Self {
            registry,
            host_isolation,
            store,
        }
    }

    /// Runs bounded `FFprobe` against a verified private snapshot and parses selected fields.
    ///
    /// Every provider call first applies `binding`'s check: a full rehash for
    /// a plain [`SourceSnapshot`], an identity comparison for a
    /// [`BoundSource`](crate::BoundSource) (see [`SourceBinding`]).
    ///
    /// # Errors
    /// Fails on changed bytes, rejected media, oversized metadata, invalid timeline, or process failure.
    pub async fn probe(
        &self,
        binding: &impl SourceBinding,
        cancellation: ProcessCancellation,
    ) -> Result<MediaDescription, MediaError> {
        let _admission = self
            .store
            .try_admit(1)
            .map_err(|_| MediaError::CapacityUnavailable)?;
        binding
            .check_before_provider_call()
            .map_err(MediaError::Source)?;
        let source = binding.snapshot();
        let request = Self::request(
            source,
            self.registry.ffprobe.clone(),
            Duration::from_secs(15),
        )?
        .with_arguments([
            "-v",
            "error",
            "-hide_banner",
            "-max_alloc",
            "67108864",
            "-protocol_whitelist",
            "file",
            "-probesize",
            "5000000",
            "-analyzeduration",
            "5000000",
        ]);
        let request = restrict_mov_references(request, source)
            .with_arguments(["-f", source.container().demuxer(), "-show_entries",
                "format=duration,start_time:stream=index,codec_type,codec_name,time_base,start_pts,width,height:stream_tags=language,rotate:stream_side_data=rotation",
                "-of", "json", "-i"])
            .with_argument(source.provider_path().as_os_str());
        let output = self
            .supervisor(MAX_PROBE_BYTES)
            .run(request, cancellation)
            .await
            .map_err(MediaError::Process)?;
        validate_outcome(&output)?;
        parse_ffprobe_metadata(&output.stdout.bytes, source.container())
    }

    /// Returns the first displayed frame at or after the requested normalized time.
    /// The reported PTS is parsed from `FFmpeg`'s selected decoded frame, never inferred
    /// from the requested timestamp or a constant frame rate.
    ///
    /// Since P09 PR 1 (2026-09-26) the frame is selected by an integer
    /// stream-timestamp bound ([`StreamTime::first_at_or_after`]) instead of
    /// a decimal time compared in floating point, only the first matching
    /// frame passes the filter, so `showinfo` must log exactly one frame, and
    /// the filter's time base must equal the probed stream's
    /// ([`MediaError::TimeBaseMismatch`] otherwise). The frame chosen is the
    /// same except where the decimal comparison rounded; the stricter
    /// diagnostics check is what closes the forged-metadata finding (SEC-17).
    ///
    /// # Errors
    /// Rejects an absent stream, unsafe timestamp, timeout, oversized frame, or unmet tolerance.
    #[allow(clippy::too_many_lines)] // Keep validation, supervised run, and provenance checks in one auditable path.
    pub async fn frame(
        &self,
        binding: &impl SourceBinding,
        description: &MediaDescription,
        selection: MediaSelection,
        requested: MediaTime,
        tolerance_micros: u64,
        cancellation: ProcessCancellation,
    ) -> Result<ExtractedFrame, MediaError> {
        let _admission = self
            .store
            .try_admit(1)
            .map_err(|_| MediaError::CapacityUnavailable)?;
        binding
            .check_before_provider_call()
            .map_err(MediaError::Source)?;
        let source = binding.snapshot();
        description
            .validate_selection(selection)
            .map_err(|_| MediaError::StreamUnavailable)?;
        let index = selection.video.ok_or(MediaError::StreamUnavailable)?;
        let stream = description
            .streams
            .iter()
            .find(|value| value.index == index && value.kind == MediaStreamKind::Video)
            .ok_or(MediaError::StreamUnavailable)?;
        if stream.decode_support == MediaDecodeSupport::Unsupported {
            return Err(MediaError::UnsupportedCodec);
        }
        if requested >= description.duration || tolerance_micros > 10_000_000 {
            return Err(MediaError::NoFrameWithinTolerance);
        }
        let offset = description
            .origin_micros
            .checked_neg()
            .ok_or(MediaError::InvalidTimeline)?;
        let bound = StreamTime::first_at_or_after(
            index,
            stream.time_base_numerator,
            stream.time_base_denominator,
            offset,
            requested,
        )
        .map_err(|_| MediaError::InvalidTimeline)?;
        let seek = i64::try_from(requested.as_micros().saturating_sub(5_000_000))
            .map_err(|_| MediaError::InvalidTimeline)?;
        let filter = format!(
            "select='gte(pts\\,{})*isnan(prev_selected_t)',showinfo",
            bound.presentation_timestamp
        );
        let request = Self::request(
            source,
            self.registry.ffmpeg.clone(),
            Duration::from_secs(30),
        )?
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
        .with_argument(seconds_arg(seek))
        .with_arguments(["-protocol_whitelist", "file"]);
        let request = restrict_mov_references(request, source)
            .with_arguments(["-f", source.container().demuxer(), "-i"])
            .with_argument(source.provider_path().as_os_str())
            .with_arguments(["-map"])
            .with_argument(format!("0:{index}"))
            .with_arguments(["-an", "-sn", "-dn", "-vf"])
            .with_argument(filter)
            .with_arguments([
                "-frames:v",
                "1",
                "-fps_mode",
                "passthrough",
                "-f",
                "image2pipe",
                "-vcodec",
                "png",
                "pipe:1",
            ]);
        let output = self
            .supervisor(MAX_FRAME_BYTES)
            .run(request, cancellation)
            .await
            .map_err(MediaError::Process)?;
        validate_outcome(&output)?;
        let observed = parse_frame_showinfo(&output.stderr.bytes)?;
        if (observed.time_base_numerator, observed.time_base_denominator)
            != (stream.time_base_numerator, stream.time_base_denominator)
        {
            return Err(MediaError::TimeBaseMismatch);
        }
        let actual = StreamTime {
            stream_index: index,
            presentation_timestamp: observed.pts,
            time_base_numerator: observed.time_base_numerator,
            time_base_denominator: observed.time_base_denominator,
            timeline_offset_micros: offset,
        }
        .to_media_time()
        .map_err(|_| MediaError::InvalidTimeline)?;
        let timing =
            FrameTiming::new(requested, actual).map_err(|_| MediaError::InvalidTimeline)?;
        if timing.delta_micros() < 0
            || u64::try_from(timing.delta_micros()).map_err(|_| MediaError::InvalidTimeline)?
                > tolerance_micros
        {
            return Err(MediaError::NoFrameWithinTolerance);
        }
        let dimensions = png_dimensions(&output.stdout.bytes)?;
        let expected = stream.rotation.displayed_dimensions(
            stream
                .encoded_dimensions
                .ok_or(MediaError::InvalidDimensions)?,
        );
        if dimensions != expected {
            return Err(MediaError::InvalidDecodedOutput);
        }
        Ok(ExtractedFrame {
            stream_index: index,
            timing,
            dimensions,
            png: output.stdout.bytes,
        })
    }

    /// Extracts at most ten seconds of mono 16 kHz signed 16-bit PCM from one selected track.
    ///
    /// # Errors
    /// Rejects unsupported ranges, missing audio, exceeded budgets, or invalid decoded bytes.
    pub async fn audio(
        &self,
        source: &impl SourceBinding,
        description: &MediaDescription,
        selection: MediaSelection,
        range: TimeRange,
        cancellation: ProcessCancellation,
    ) -> Result<ExtractedAudio, MediaError> {
        self.decode_pcm(
            PcmProfile::Clip,
            source,
            description,
            selection,
            range,
            cancellation,
        )
        .await
    }

    /// Decodes at most thirty seconds of mono 16 kHz signed 16-bit PCM for
    /// local speech recognition, reporting the first decoded sample's time.
    ///
    /// Unlike [`Self::audio`], a window in which the stream has no audio at
    /// all is reported as [`MediaError::NoDecodedAudio`] so a recognizer can
    /// record the gap, and the timestamp diagnostic is reported per 65,536
    /// samples rather than per codec frame, so thirty seconds of any source
    /// rate stays well inside the diagnostic bound.
    ///
    /// # Errors
    /// Rejects unsupported ranges, missing streams, exceeded budgets, or invalid decoded bytes.
    pub async fn speech_pcm(
        &self,
        source: &impl SourceBinding,
        description: &MediaDescription,
        selection: MediaSelection,
        range: TimeRange,
        cancellation: ProcessCancellation,
    ) -> Result<ExtractedAudio, MediaError> {
        self.decode_pcm(
            PcmProfile::Speech,
            source,
            description,
            selection,
            range,
            cancellation,
        )
        .await
    }

    /// Decodes one visual window's samples: actual frames at least 0.5 s
    /// apart from the window's lead-in time (up to 0.5 s before the window)
    /// to its end, downscaled to 128x72 8-bit grey.
    ///
    /// The window is sampled as one bounded `FFmpeg` run with a closed
    /// argument list; only numbers and the private provider path are filled
    /// in. A `select` expression on raw stream timestamps keeps frames in
    /// `[lead-in, end)` spaced at least 0.5 s apart (the first selected frame
    /// is always kept), `-fps_mode passthrough` keeps each frame's own
    /// timestamp, and `showinfo` reports it. At most 122 frames are written.
    /// The reported times are parsed and normalized by
    /// [`parse_visual_samples`], never inferred from the request. The scale
    /// to 128x72 ignores the aspect ratio: samples only detect change.
    ///
    /// A visual index decodes window after window from the same copy, so this
    /// operation takes a [`BoundSource`]: each call compares only the copy's
    /// on-disk identity, and the caller verifies the full hash again before
    /// committing (issue #148). Each call takes one root admission slot, as
    /// every other provider operation does.
    ///
    /// # Errors
    ///
    /// Rejects an absent or unsupported video stream, a window beyond the
    /// source, a changed source, a timeout, exceeded bounds, or malformed
    /// provider output.
    #[allow(clippy::too_many_lines)] // Keep validation, the closed argument list and parsing in one auditable path.
    pub async fn visual_samples(
        &self,
        binding: &BoundSource,
        description: &MediaDescription,
        selection: MediaSelection,
        window: VisualWindow,
        cancellation: ProcessCancellation,
    ) -> Result<Vec<RawGrayFrame>, MediaError> {
        let _admission = self
            .store
            .try_admit(1)
            .map_err(|_| MediaError::CapacityUnavailable)?;
        binding
            .check_before_provider_call()
            .map_err(MediaError::Source)?;
        let source = binding.snapshot();
        description
            .validate_selection(selection)
            .map_err(|_| MediaError::StreamUnavailable)?;
        let index = selection.video.ok_or(MediaError::StreamUnavailable)?;
        let stream = description
            .streams
            .iter()
            .find(|value| value.index == index && value.kind == MediaStreamKind::Video)
            .ok_or(MediaError::StreamUnavailable)?;
        if stream.decode_support == MediaDecodeSupport::Unsupported {
            return Err(MediaError::UnsupportedCodec);
        }
        let range = window.range();
        if range.end() > description.duration {
            return Err(MediaError::InvalidVisualWindow);
        }
        let sampling = VisualSamplingWindow::new(
            index,
            description.origin_micros,
            window.lead_in_start(),
            range.end(),
        )?;
        let raw_lead = sampling.raw_micros(sampling.lead_in)?;
        let raw_end = sampling.raw_micros(sampling.end)?;
        let seek = sampling
            .lead_in
            .as_micros()
            .saturating_sub(VISUAL_SEEK_MARGIN_MICROS);
        let read = range
            .end()
            .as_micros()
            .checked_sub(seek)
            .and_then(|span| span.checked_add(VISUAL_SEEK_MARGIN_MICROS))
            .ok_or(MediaError::InvalidVisualWindow)?;
        let as_argument =
            |micros: u64| i64::try_from(micros).map_err(|_| MediaError::InvalidVisualWindow);
        let filter = format!(
            "select='gte(t\\,{lead})*lt(t\\,{end})*(isnan(prev_selected_t)+gte(t-prev_selected_t\\,{interval}))',scale={VISUAL_FRAME_WIDTH}:{VISUAL_FRAME_HEIGHT}:flags=area,format=gray,showinfo",
            lead = seconds_arg(raw_lead),
            end = seconds_arg(raw_end),
            interval = seconds_arg(as_argument(VISUAL_SAMPLE_INTERVAL_MICROS)?),
        );
        let request = Self::request(source, self.registry.ffmpeg.clone(), VISUAL_WINDOW_DEADLINE)?
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
            .with_argument(seconds_arg(as_argument(seek)?))
            .with_argument("-t")
            .with_argument(seconds_arg(as_argument(read)?))
            .with_arguments(["-protocol_whitelist", "file"]);
        let request = restrict_mov_references(request, source)
            .with_arguments(["-f", source.container().demuxer(), "-i"])
            .with_argument(source.provider_path().as_os_str())
            .with_arguments(["-map"])
            .with_argument(format!("0:{index}"))
            .with_arguments(["-an", "-sn", "-dn", "-vf"])
            .with_argument(filter)
            .with_arguments(["-fps_mode", "passthrough", "-frames:v"])
            .with_argument(MAX_WINDOW_SAMPLES.to_string())
            .with_arguments(["-f", "rawvideo", "-pix_fmt", "gray", "pipe:1"]);
        let output = self
            .supervisor_with(MAX_VISUAL_SAMPLE_BYTES, MAX_VISUAL_DIAGNOSTIC_BYTES)
            .run(request, cancellation)
            .await
            .map_err(MediaError::Process)?;
        validate_outcome(&output)?;
        parse_visual_samples(&output.stdout.bytes, &output.stderr.bytes, &sampling)
    }

    #[allow(clippy::too_many_lines)] // The bounded chunk and timestamp checks form one operation contract.
    async fn decode_pcm(
        &self,
        profile: PcmProfile,
        binding: &impl SourceBinding,
        description: &MediaDescription,
        selection: MediaSelection,
        range: TimeRange,
        cancellation: ProcessCancellation,
    ) -> Result<ExtractedAudio, MediaError> {
        let _admission = self
            .store
            .try_admit(1)
            .map_err(|_| MediaError::CapacityUnavailable)?;
        binding
            .check_before_provider_call()
            .map_err(MediaError::Source)?;
        let source = binding.snapshot();
        description
            .validate_selection(selection)
            .map_err(|_| MediaError::StreamUnavailable)?;
        let index = selection.audio.ok_or(MediaError::StreamUnavailable)?;
        let stream = description
            .streams
            .iter()
            .find(|value| value.index == index && value.kind == MediaStreamKind::Audio)
            .ok_or(MediaError::StreamUnavailable)?;
        if stream.decode_support == MediaDecodeSupport::Unsupported {
            return Err(MediaError::UnsupportedCodec);
        }
        if range.end() > description.duration
            || range.duration_micros() > profile.max_range_micros()
        {
            return Err(MediaError::InvalidAudioRange);
        }
        let seek =
            i64::try_from(range.start().as_micros()).map_err(|_| MediaError::InvalidAudioRange)?;
        let request = Self::request(source, self.registry.ffmpeg.clone(), profile.deadline())?
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
                "-ss",
            ])
            .with_argument(seconds_arg(seek))
            .with_arguments(["-protocol_whitelist", "file"]);
        let request = restrict_mov_references(request, source)
            .with_arguments(["-f", source.container().demuxer(), "-i"])
            .with_argument(source.provider_path().as_os_str())
            .with_arguments(["-map"])
            .with_argument(format!("0:{index}"))
            .with_arguments(["-vn", "-sn", "-dn", "-t"])
            .with_argument(seconds_arg(
                i64::try_from(range.duration_micros())
                    .map_err(|_| MediaError::InvalidAudioRange)?,
            ))
            .with_arguments([
                "-af",
                profile.filter(),
                "-ac",
                "1",
                "-ar",
                "16000",
                "-f",
                "s16le",
                "pipe:1",
            ]);
        let output = self
            .supervisor(profile.stdout_limit())
            .run(request, cancellation)
            .await
            .map_err(MediaError::Process)?;
        validate_outcome(&output)?;
        if output.stdout.bytes.is_empty()
            && matches!(profile, PcmProfile::Speech | PcmProfile::EvidenceWav)
        {
            return Err(MediaError::NoDecodedAudio);
        }
        if output.stdout.bytes.is_empty()
            || output.stdout.bytes.len() % 2 != 0
            || output.stdout.bytes.len() > profile.max_pcm_bytes()
        {
            return Err(MediaError::InvalidDecodedOutput);
        }
        let first_pts = parse_ashowinfo_start(&output.stderr.bytes)?;
        let normalized = seek
            .checked_add(first_pts)
            .ok_or(MediaError::InvalidTimeline)?;
        let actual_start = MediaTime::from_micros(
            u64::try_from(normalized).map_err(|_| MediaError::InvalidTimeline)?,
        );
        Ok(ExtractedAudio {
            stream_index: index,
            requested: range,
            actual_start,
            sample_rate: 16_000,
            channels: 1,
            pcm_s16le: output.stdout.bytes,
        })
    }

    fn request(
        source: &SourceSnapshot,
        executable: TrustedExecutable,
        deadline: Duration,
    ) -> Result<ProcessRequest, MediaError> {
        let provider_path = source.provider_path();
        let parent = provider_path
            .parent()
            .ok_or(MediaError::InvalidSourcePath)?;
        let cwd = ProcessWorkingDirectory::new(parent).map_err(MediaError::Request)?;
        ProcessRequest::new(executable, cwd, deadline).map_err(MediaError::Request)
    }

    fn supervisor(&self, stdout_limit: usize) -> ProcessSupervisor {
        self.supervisor_with(stdout_limit, MAX_DIAGNOSTIC_BYTES)
    }

    fn supervisor_with(&self, stdout_limit: usize, stderr_limit: usize) -> ProcessSupervisor {
        let stdout = NonZeroUsize::new(stdout_limit).unwrap_or(NonZeroUsize::MIN);
        let stderr = NonZeroUsize::new(stderr_limit).unwrap_or(NonZeroUsize::MIN);
        let policy = SupervisorPolicy::default().with_stream_limits(stdout, stderr);
        ProcessSupervisor::new(policy, self.host_isolation)
    }
}

/// The bounded PCM operations: a short clip, a speech chunk and an evidence WAV clip.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PcmProfile {
    /// [`FfmpegMedia::audio`]: at most ten seconds, per-frame timestamp log.
    Clip,
    /// [`FfmpegMedia::speech_pcm`]: at most thirty seconds for recognition.
    Speech,
    /// [`FfmpegMedia::wav_clip`]: at most thirty seconds of evidence audio.
    EvidenceWav,
}

impl PcmProfile {
    const fn max_range_micros(self) -> u64 {
        match self {
            Self::Clip => 10_000_000,
            Self::Speech => MAX_SPEECH_PCM_MICROS,
            Self::EvidenceWav => evidence::MAX_WAV_CLIP_MICROS,
        }
    }

    const fn deadline(self) -> Duration {
        match self {
            Self::Clip | Self::EvidenceWav => Duration::from_secs(30),
            Self::Speech => Duration::from_secs(60),
        }
    }

    const fn stdout_limit(self) -> usize {
        match self {
            Self::Clip => MAX_AUDIO_BYTES,
            Self::Speech | Self::EvidenceWav => MAX_SPEECH_PCM_BYTES,
        }
    }

    const fn max_pcm_bytes(self) -> usize {
        match self {
            Self::Clip => MAX_CLIP_PCM_BYTES,
            Self::Speech | Self::EvidenceWav => MAX_SPEECH_PCM_BYTES,
        }
    }

    /// The timestamp diagnostic filter. `ashowinfo` logs one line per frame,
    /// which for thirty seconds of a 48 kHz source would exceed the 64 KiB
    /// diagnostic bound; regrouping into 65,536-sample frames first keeps the
    /// first frame's timestamp and logs a few dozen lines at most.
    const fn filter(self) -> &'static str {
        match self {
            Self::Clip => "ashowinfo",
            Self::Speech | Self::EvidenceWav => "asetnsamples=n=65536:p=0,ashowinfo",
        }
    }
}

fn restrict_mov_references(request: ProcessRequest, source: &SourceSnapshot) -> ProcessRequest {
    if source.container() == crate::SourceContainer::IsoMedia {
        request.with_arguments(["-enable_drefs", "0", "-use_absolute_path", "0"])
    } else {
        request
    }
}

/// One bounded PNG frame with independently observed timing and orientation.
pub struct ExtractedFrame {
    /// Original selected video stream index.
    pub stream_index: u32,
    /// Requested and actual normalized timestamps.
    pub timing: FrameTiming,
    /// Displayed image dimensions.
    pub dimensions: FrameDimensions,
    /// PNG bytes, bounded by the P04 profile.
    pub png: Vec<u8>,
}

/// One bounded source-grounded PCM chunk.
pub struct ExtractedAudio {
    /// Original selected audio stream index.
    pub stream_index: u32,
    /// Requested normalized half-open range.
    pub requested: TimeRange,
    /// Observed first decoded audio PTS on the normalized timeline.
    pub actual_start: MediaTime,
    /// Output sample rate.
    pub sample_rate: u32,
    /// Output channel count.
    pub channels: u8,
    /// Interleaved signed little-endian PCM bytes.
    pub pcm_s16le: Vec<u8>,
}

fn seconds_arg(micros: i64) -> String {
    let value = i128::from(micros);
    let sign = if value < 0 { "-" } else { "" };
    let magnitude = value.abs();
    format!(
        "{sign}{}.{:06}",
        magnitude / 1_000_000,
        magnitude % 1_000_000
    )
}

fn png_dimensions(bytes: &[u8]) -> Result<FrameDimensions, MediaError> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return Err(MediaError::InvalidDecodedOutput);
    }
    let width = u32::from_be_bytes(
        bytes[16..20]
            .try_into()
            .map_err(|_| MediaError::InvalidDecodedOutput)?,
    );
    let height = u32::from_be_bytes(
        bytes[20..24]
            .try_into()
            .map_err(|_| MediaError::InvalidDecodedOutput)?,
    );
    if u64::from(width) * u64::from(height) > MAX_FRAME_PIXELS {
        return Err(MediaError::InvalidDecodedOutput);
    }
    FrameDimensions::new(width, height).map_err(|_| MediaError::InvalidDecodedOutput)
}

fn validate_outcome(output: &ProcessOutcome) -> Result<(), MediaError> {
    match output.termination {
        TerminationReason::Exited
            if output.status.success() && output.stdout.complete && output.stderr.complete =>
        {
            Ok(())
        }
        TerminationReason::Exited => Err(MediaError::ProviderRejected),
        TerminationReason::Deadline => Err(MediaError::Deadline),
        TerminationReason::Cancelled => Err(MediaError::Cancelled),
        TerminationReason::OutputLimit(_) => Err(MediaError::OutputLimit),
    }
}

#[derive(Deserialize)]
struct RawProbe {
    format: Option<RawFormat>,
    streams: Option<Vec<RawStream>>,
}

#[derive(Deserialize)]
struct RawFormat {
    duration: Option<String>,
    start_time: Option<String>,
}

#[derive(Deserialize)]
struct RawStream {
    index: u32,
    codec_type: Option<String>,
    codec_name: Option<String>,
    time_base: Option<String>,
    start_pts: Option<i64>,
    width: Option<u32>,
    height: Option<u32>,
    tags: Option<RawTags>,
    side_data_list: Option<Vec<RawSideData>>,
}

#[derive(Deserialize)]
struct RawTags {
    language: Option<String>,
    rotate: Option<String>,
}

#[derive(Deserialize)]
struct RawSideData {
    rotation: Option<i32>,
}

/// Parses and validates the `FFprobe` JSON metadata document that
/// [`FfmpegMedia::probe`] requests.
///
/// The document is untrusted provider output, so it is published beside the
/// other pure provider parsers ([`parse_whisper_full_json`](crate::parse_whisper_full_json),
/// [`parse_supplied_transcript`](crate::parse_supplied_transcript)) where fuzz
/// targets and tests can reach it without running `FFprobe` (ADR 0016,
/// decision 6). Only the fields named in the probe's `-show_entries` are read.
/// The whole description is validated before any of it is returned: at most
/// [`MAX_PROBE_BYTES`], 32 streams with unique indexes, a positive duration of
/// at most four hours (Matroska's reported end is normalized by the earliest
/// stream start), a positive time base for every stream, dimensions of at most
/// 16 megapixels for every video stream, a quarter-turn display rotation, and
/// short ASCII codec and language names.
///
/// # Errors
///
/// Returns the first [`MediaError`] describing why the document is unusable;
/// parsing never partially succeeds.
#[allow(clippy::too_many_lines)] // Parse and validate the provider document before exposing any stream metadata.
pub fn parse_ffprobe_metadata(
    bytes: &[u8],
    container: crate::SourceContainer,
) -> Result<MediaDescription, MediaError> {
    if bytes.len() > MAX_PROBE_BYTES {
        return Err(MediaError::OutputLimit);
    }
    let raw: RawProbe = serde_json::from_slice(bytes).map_err(|_| MediaError::InvalidMetadata)?;
    let raw_format = raw.format.ok_or(MediaError::InvalidMetadata)?;
    let duration = raw_format
        .duration
        .as_deref()
        .and_then(parse_seconds_micros)
        .ok_or(MediaError::InvalidDuration)?;
    let raw_streams = raw.streams.ok_or(MediaError::InvalidMetadata)?;
    if raw_streams.len() > MAX_STREAMS {
        return Err(MediaError::TooManyStreams);
    }
    let mut streams = Vec::with_capacity(raw_streams.len());
    let mut starts = Vec::new();
    for raw in raw_streams {
        if streams
            .iter()
            .any(|stream: &MediaStream| stream.index == raw.index)
        {
            return Err(MediaError::InvalidMetadata);
        }
        let kind = match raw.codec_type.as_deref() {
            Some("video") => MediaStreamKind::Video,
            Some("audio") => MediaStreamKind::Audio,
            _ => MediaStreamKind::Other,
        };
        let codec = raw.codec_name.unwrap_or_else(|| "unknown".to_owned());
        if codec.len() > 64 || !codec.is_ascii() {
            return Err(MediaError::InvalidMetadata);
        }
        let decode_support = match kind {
            MediaStreamKind::Video
                if matches!(
                    codec.as_str(),
                    "h264" | "hevc" | "vp8" | "vp9" | "av1" | "mpeg4"
                ) =>
            {
                MediaDecodeSupport::Supported
            }
            MediaStreamKind::Audio
                if matches!(
                    codec.as_str(),
                    "aac" | "pcm_s16le" | "pcm_f32le" | "mp3" | "opus" | "vorbis" | "flac"
                ) =>
            {
                MediaDecodeSupport::Supported
            }
            _ => MediaDecodeSupport::Unsupported,
        };
        let (numerator, denominator) = raw
            .time_base
            .as_deref()
            .and_then(parse_time_base)
            .ok_or(MediaError::InvalidMetadata)?;
        let dimensions = match kind {
            MediaStreamKind::Video => {
                let width = raw.width.ok_or(MediaError::InvalidDimensions)?;
                let height = raw.height.ok_or(MediaError::InvalidDimensions)?;
                if u64::from(width) * u64::from(height) > MAX_FRAME_PIXELS {
                    return Err(MediaError::InvalidDimensions);
                }
                Some(
                    FrameDimensions::new(width, height)
                        .map_err(|_| MediaError::InvalidDimensions)?,
                )
            }
            MediaStreamKind::Audio | MediaStreamKind::Other => None,
        };
        let angle = raw
            .side_data_list
            .as_ref()
            .and_then(|items| items.iter().find_map(|item| item.rotation))
            .or_else(|| {
                raw.tags
                    .as_ref()
                    .and_then(|tags| tags.rotate.as_deref()?.parse::<i32>().ok())
            })
            .unwrap_or(0);
        // FFprobe's display-matrix angle is counter-clockwise; domain orientation is clockwise.
        let rotation = match angle.rem_euclid(360) {
            0 => DisplayRotation::Zero,
            90 => DisplayRotation::Clockwise270,
            180 => DisplayRotation::HalfTurn,
            270 => DisplayRotation::Clockwise90,
            _ => return Err(MediaError::InvalidOrientation),
        };
        let language = raw.tags.and_then(|tags| tags.language);
        if language
            .as_ref()
            .is_some_and(|value| value.len() > 32 || !value.is_ascii())
        {
            return Err(MediaError::InvalidMetadata);
        }
        if let Some(pts) = raw.start_pts {
            let micros = i128::from(pts)
                .checked_mul(i128::from(numerator))
                .and_then(|value| value.checked_mul(1_000_000))
                .ok_or(MediaError::InvalidTimeline)?
                / i128::from(denominator);
            starts.push(i64::try_from(micros).map_err(|_| MediaError::InvalidTimeline)?);
        }
        streams.push(MediaStream {
            index: raw.index,
            kind,
            codec,
            decode_support,
            time_base_numerator: numerator,
            time_base_denominator: denominator,
            start_pts: raw.start_pts,
            encoded_dimensions: dimensions,
            rotation,
            language,
        });
    }
    let fallback = raw_format
        .start_time
        .as_deref()
        .and_then(parse_seconds_micros)
        .unwrap_or(0);
    let origin_micros = starts.into_iter().min().unwrap_or(fallback);
    // Matroska's container duration is an end PTS after an output timestamp offset;
    // ISO media reports the presentation span directly. Keep the original origin.
    let normalized_duration = if container == crate::SourceContainer::Matroska {
        duration
            .checked_sub(origin_micros)
            .ok_or(MediaError::InvalidDuration)?
    } else {
        duration
    };
    let duration = u64::try_from(normalized_duration).map_err(|_| MediaError::InvalidDuration)?;
    if duration == 0 || duration > MAX_DURATION_MICROS {
        return Err(MediaError::InvalidDuration);
    }
    Ok(MediaDescription {
        duration: MediaTime::from_micros(duration),
        origin_micros,
        streams,
    })
}

/// The timeline of one visual window's decode: which stream, how its raw
/// timestamps map to source time, and where samples may lie.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisualSamplingWindow {
    stream_index: u32,
    origin_micros: i64,
    lead_in: MediaTime,
    end: MediaTime,
}

impl VisualSamplingWindow {
    /// Describes a decode of `stream_index` whose samples must lie in
    /// `[lead_in, end)` on the normalized timeline, for a source whose
    /// earliest stream starts at `origin_micros` of raw stream time.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::InvalidVisualWindow`] for an empty interval.
    pub fn new(
        stream_index: u32,
        origin_micros: i64,
        lead_in: MediaTime,
        end: MediaTime,
    ) -> Result<Self, MediaError> {
        if end <= lead_in {
            return Err(MediaError::InvalidVisualWindow);
        }
        Ok(Self {
            stream_index,
            origin_micros,
            lead_in,
            end,
        })
    }

    fn raw_micros(self, time: MediaTime) -> Result<i64, MediaError> {
        i64::try_from(time.as_micros())
            .ok()
            .and_then(|micros| micros.checked_add(self.origin_micros))
            .ok_or(MediaError::InvalidVisualWindow)
    }
}

/// One decoded 128x72 8-bit grey sample frame at its observed source time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawGrayFrame {
    /// Observed presentation time on the normalized source timeline.
    pub time: MediaTime,
    /// Row-major grey pixels.
    pub pixels: Box<[u8; VISUAL_FRAME_BYTES]>,
}

/// Parses one visual window's decoded samples from `FFmpeg`'s raw grey
/// output and its `showinfo` diagnostics.
///
/// Both streams are untrusted provider output, so the parser is published
/// beside [`parse_ffprobe_metadata`] for fuzzing (ADR 0016, decision 6). It
/// accepts the output only when the two agree exactly:
///
/// - stdout is a whole number `n` of 9,216-byte frames, `n` at most
///   [`MAX_WINDOW_SAMPLES`];
/// - stderr has `showinfo` frame lines numbered `0..n-1`, in order, and when
///   `n > 0` at least one `config in time_base` line, all of them equal (a
///   filter graph reconfigured mid-window, for example on a resolution
///   change, repeats it);
/// - timestamps strictly increase and, normalized through the source origin,
///   lie in the window's `[lead-in, end)`.
///
/// Only lines that begin with `[Parsed_showinfo_` are read, so input
/// metadata that `FFmpeg` echoes (always indented) cannot forge a frame line,
/// and other lines are never decoded as text at all.
///
/// # Errors
///
/// Returns [`MediaError::OutputLimit`] beyond the byte bounds and
/// [`MediaError::InvalidDecodedOutput`] or [`MediaError::InvalidTimeline`]
/// for anything inconsistent; parsing never partially succeeds.
pub fn parse_visual_samples(
    stdout: &[u8],
    stderr: &[u8],
    window: &VisualSamplingWindow,
) -> Result<Vec<RawGrayFrame>, MediaError> {
    if stdout.len() > MAX_VISUAL_SAMPLE_BYTES || stderr.len() > MAX_VISUAL_DIAGNOSTIC_BYTES {
        return Err(MediaError::OutputLimit);
    }
    if !stdout.len().is_multiple_of(VISUAL_FRAME_BYTES) {
        return Err(MediaError::InvalidDecodedOutput);
    }
    let count = stdout.len() / VISUAL_FRAME_BYTES;
    let log = showinfo::scan_showinfo(stderr, MAX_WINDOW_SAMPLES)?;
    let time_base = log.time_base;
    let stamps: Vec<i64> = log.frames.iter().map(|frame| frame.pts).collect();
    if stamps.len() != count {
        return Err(MediaError::InvalidDecodedOutput);
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    let (numerator, denominator) = time_base.ok_or(MediaError::InvalidDecodedOutput)?;
    let offset = window
        .origin_micros
        .checked_neg()
        .ok_or(MediaError::InvalidTimeline)?;
    let mut frames = Vec::with_capacity(count);
    let mut previous: Option<MediaTime> = None;
    for (pts, pixels) in stamps
        .into_iter()
        .zip(stdout.as_chunks::<VISUAL_FRAME_BYTES>().0)
    {
        let time = StreamTime {
            stream_index: window.stream_index,
            presentation_timestamp: pts,
            time_base_numerator: numerator,
            time_base_denominator: denominator,
            timeline_offset_micros: offset,
        }
        .to_media_time()
        .map_err(|_| MediaError::InvalidTimeline)?;
        if time < window.lead_in
            || time >= window.end
            || previous.is_some_and(|before| before >= time)
        {
            return Err(MediaError::InvalidDecodedOutput);
        }
        previous = Some(time);
        frames.push(RawGrayFrame {
            time,
            pixels: Box::new(*pixels),
        });
    }
    Ok(frames)
}

fn parse_time_base(text: &str) -> Option<(u32, u32)> {
    let (n, d) = text.split_once('/')?;
    let numerator = n.parse::<u32>().ok()?;
    let denominator = d.parse::<u32>().ok()?;
    if numerator == 0 || denominator == 0 {
        None
    } else {
        Some((numerator, denominator))
    }
}

fn parse_seconds_micros(text: &str) -> Option<i64> {
    let (negative, body) = if let Some(rest) = text.strip_prefix('-') {
        (true, rest)
    } else {
        (false, text)
    };
    let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let seconds = whole.parse::<i128>().ok()?;
    let micro_digits = fraction.get(..fraction.len().min(6))?;
    let mut fraction_micros = micro_digits.parse::<i128>().unwrap_or(0);
    for _ in micro_digits.len()..6 {
        fraction_micros *= 10;
    }
    let micros = seconds
        .checked_mul(1_000_000)?
        .checked_add(fraction_micros)?;
    i64::try_from(if negative { -micros } else { micros }).ok()
}

/// Typed media-adapter failures; provider diagnostics remain bounded and untrusted.
#[derive(Debug)]
pub enum MediaError {
    /// Source staging or identity failed.
    Source(SourceError),
    /// Provider request could not be validated.
    Request(ProcessRequestError),
    /// Trusted supervisor could not run the provider.
    Process(ProcessError),
    /// Snapshot had no canonical private parent.
    InvalidSourcePath,
    /// Provider rejected or could not decode the selected media.
    ProviderRejected,
    /// Provider operation exceeded its deadline.
    Deadline,
    /// Caller cancelled the operation.
    Cancelled,
    /// Structured output or diagnostics exceeded a hard byte limit.
    OutputLimit,
    /// Structured probe data was missing, malformed, or inconsistent.
    InvalidMetadata,
    /// Source duration was unknown, zero, or above the accepted profile.
    InvalidDuration,
    /// Root-wide heavy-stage admission was unavailable.
    CapacityUnavailable,
    /// Audio range exceeds the accepted chunk or source timeline.
    InvalidAudioRange,
    /// Source has more streams than the accepted profile.
    TooManyStreams,
    /// Encoded frame dimensions are missing, zero, or too large.
    InvalidDimensions,
    /// Display orientation is unsupported.
    InvalidOrientation,
    /// Stream timestamp cannot be normalized.
    InvalidTimeline,
    /// Requested stream is absent or has the wrong kind.
    StreamUnavailable,
    /// Selected stream codec is outside this adapter's reviewed decode set.
    UnsupportedCodec,
    /// No displayed frame met the requested tolerance.
    NoFrameWithinTolerance,
    /// Decoded output was malformed or exceeded its pixel/byte budget.
    InvalidDecodedOutput,
    /// The selected stream has no audio in the requested speech window.
    NoDecodedAudio,
    /// A visual window lies outside the source or cannot be expressed.
    InvalidVisualWindow,
    /// An evidence request was rejected before any I/O: an empty, unordered
    /// or oversized batch of timestamps, or a range outside the source or
    /// beyond its bound.
    InvalidFrameRequest,
    /// A crop rectangle is not contained by the displayed frame.
    CropOutsideFrame,
    /// A requested stream timestamp decoded no frame.
    FrameNotFound,
    /// The provider's filter time base differs from the probed stream's, so
    /// integer timestamps would not name the same instants.
    TimeBaseMismatch,
}

impl fmt::Display for MediaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Source(_) => "source binding failed",
            Self::Request(_) => "provider request is invalid",
            Self::Process(_) => "provider process failed",
            Self::InvalidSourcePath => "source snapshot path is invalid",
            Self::ProviderRejected => "provider rejected selected media",
            Self::Deadline => "media operation timed out",
            Self::Cancelled => "media operation was cancelled",
            Self::OutputLimit => "media output exceeded the byte limit",
            Self::InvalidMetadata => "media metadata is invalid",
            Self::InvalidDuration => "media duration is invalid",
            Self::CapacityUnavailable => "media admission capacity is unavailable",
            Self::InvalidAudioRange => "audio range is invalid",
            Self::TooManyStreams => "media has too many streams",
            Self::InvalidDimensions => "media dimensions are invalid",
            Self::InvalidOrientation => "media orientation is unsupported",
            Self::InvalidTimeline => "media timeline is invalid",
            Self::StreamUnavailable => "selected media stream is unavailable",
            Self::UnsupportedCodec => "selected media codec is unsupported",
            Self::NoFrameWithinTolerance => "no displayed frame met the requested tolerance",
            Self::InvalidDecodedOutput => "decoded media output is invalid",
            Self::NoDecodedAudio => "no audio was decoded in the requested window",
            Self::InvalidVisualWindow => "visual window is outside the source",
            Self::InvalidFrameRequest => "frame request is invalid",
            Self::CropOutsideFrame => "crop rectangle is outside the displayed frame",
            Self::FrameNotFound => "a requested frame was not decoded",
            Self::TimeBaseMismatch => "provider time base differs from the probed stream",
        };
        formatter.write_str(message)
    }
}

impl Error for MediaError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(error) => Some(error),
            Self::Request(error) => Some(error),
            Self::Process(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MediaError, ObservedFrameTime, VisualSamplingWindow, parse_ashowinfo_start,
        parse_ffprobe_metadata, parse_frame_showinfo, parse_visual_samples,
    };
    use crate::SourceContainer;
    use vsift_domain::{
        DisplayRotation, MediaDecodeSupport, MediaSelection, MediaStreamKind, MediaTime,
        VISUAL_FRAME_BYTES,
    };

    const VALID: &str = r#"{"format":{"duration":"2.000000","start_time":"1.250000"},"streams":[{"index":0,"codec_type":"video","codec_name":"h264","time_base":"1/1000","start_pts":1250,"width":320,"height":240,"side_data_list":[{"rotation":-90}]},{"index":2,"codec_type":"audio","codec_name":"aac","time_base":"1/16000","start_pts":32000,"tags":{"language":"eng"}}]}"#;

    #[test]
    fn parses_nonzero_origin_rotation_and_explicit_tracks() -> Result<(), Box<dyn std::error::Error>>
    {
        let parsed = parse_ffprobe_metadata(VALID.as_bytes(), SourceContainer::IsoMedia)?;
        assert_eq!(parsed.origin_micros, 1_250_000);
        assert_eq!(parsed.duration.as_micros(), 2_000_000);
        assert_eq!(parsed.streams[0].rotation, DisplayRotation::Clockwise90);
        assert_eq!(parsed.streams[1].kind, MediaStreamKind::Audio);
        assert_eq!(parsed.streams[1].language.as_deref(), Some("eng"));
        parsed.validate_selection(MediaSelection {
            video: Some(0),
            audio: Some(2),
        })?;
        assert!(
            parsed
                .validate_selection(MediaSelection {
                    video: Some(2),
                    audio: None
                })
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn rejects_bomb_dimensions_and_missing_duration() {
        let huge = VALID.replace("\"width\":320", "\"width\":100000");
        assert!(matches!(
            parse_ffprobe_metadata(huge.as_bytes(), SourceContainer::IsoMedia),
            Err(MediaError::InvalidDimensions)
        ));
        let missing = VALID.replace("\"duration\":\"2.000000\"", "\"duration\":\"N/A\"");
        assert!(matches!(
            parse_ffprobe_metadata(missing.as_bytes(), SourceContainer::IsoMedia),
            Err(MediaError::InvalidDuration)
        ));
    }

    #[test]
    fn extracts_observed_pts_and_filter_clock() -> Result<(), Box<dyn std::error::Error>> {
        let diagnostic = b"[Parsed_showinfo_1] config in time_base: 1/16384, frame_rate: 2/1\n[Parsed_showinfo_1] n:   0 pts:  16384 pts_time:1\n";
        assert_eq!(
            parse_frame_showinfo(diagnostic)?,
            ObservedFrameTime {
                pts: 16_384,
                time_base_numerator: 1,
                time_base_denominator: 16_384
            }
        );
        Ok(())
    }

    /// `FFmpeg` 9.0 echoes the input's and the output's global metadata at
    /// `-loglevel info`, indented, around the filter's own lines. A `title`
    /// written to look like a `showinfo` frame line and a time base must not
    /// be read as either (SEC-17).
    const FORGED_FRAME_DIAGNOSTIC: &str = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'source.media':\n  Metadata:\n    title           : [Parsed_showinfo_0 @ 0] n:   0 pts:  99999 time_base: 1/1\n[Parsed_showinfo_1 @ 0000014d] config in time_base: 1/10240, frame_rate: 20/1\n[Parsed_showinfo_1 @ 0000014d] config out time_base: 0/0, frame_rate: 0/0\n[Parsed_showinfo_1 @ 0000014d] n:   0 pts:  10752 pts_time:1.05    duration:    512 fmt:yuv420p s:1280x720\nOutput #0, image2pipe, to 'pipe:1':\n  Metadata:\n    title           : [Parsed_showinfo_0 @ 0] n:   0 pts:  99999 time_base: 1/1\n";

    const FORGED_AUDIO_DIAGNOSTIC: &str = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'source.media':\n  Metadata:\n    title           : [Parsed_ashowinfo_0 @ 0] n:0 pts:0 pts_time:9.5\n[Parsed_ashowinfo_1 @ 00000200] n:0 pts:1024 pts_time:0.064 fmt:fltp channels:1 chlayout:mono rate:16000 nb_samples:65536\nOutput #0, s16le, to 'pipe:1':\n  Metadata:\n    title           : [Parsed_ashowinfo_0 @ 0] n:0 pts:0 pts_time:9.5\n";

    #[test]
    fn echoed_metadata_cannot_forge_a_frame_time() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            parse_frame_showinfo(FORGED_FRAME_DIAGNOSTIC.as_bytes())?,
            ObservedFrameTime {
                pts: 10_752,
                time_base_numerator: 1,
                time_base_denominator: 10_240
            }
        );
        Ok(())
    }

    #[test]
    fn echoed_metadata_cannot_forge_an_audio_start() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            parse_ashowinfo_start(FORGED_AUDIO_DIAGNOSTIC.as_bytes())?,
            64_000
        );
        Ok(())
    }

    #[test]
    fn malformed_family_rejects_excessive_streams_and_dimensions() {
        let many = include_bytes!("../../../fixtures/corpus/generated/F11-excessive-streams.json");
        let huge =
            include_bytes!("../../../fixtures/corpus/generated/F11-oversized-dimensions.json");
        assert!(matches!(
            parse_ffprobe_metadata(many, SourceContainer::IsoMedia),
            Err(MediaError::TooManyStreams)
        ));
        assert!(matches!(
            parse_ffprobe_metadata(huge, SourceContainer::IsoMedia),
            Err(MediaError::InvalidDimensions)
        ));
    }

    #[test]
    fn preserves_missing_capabilities_and_marks_unknown_codec_unsupported()
    -> Result<(), Box<dyn std::error::Error>> {
        let no_video = br#"{"format":{"duration":"1.000000"},"streams":[{"index":3,"codec_type":"audio","codec_name":"future-codec","time_base":"1/1000"}]}"#;
        let description = parse_ffprobe_metadata(no_video, SourceContainer::IsoMedia)?;
        assert!(
            description
                .validate_selection(MediaSelection {
                    video: Some(0),
                    audio: None
                })
                .is_err()
        );
        assert_eq!(
            description.streams[0].decode_support,
            MediaDecodeSupport::Unsupported
        );
        Ok(())
    }

    /// `showinfo` diagnostics shaped like `FFmpeg` 9.0's for three samples of
    /// F01, with the echoed input header, both configuration lines and a
    /// side-data line.
    const VISUAL_DIAGNOSTIC: &str = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'source.media':\n  Metadata:\n    title           : [Parsed_showinfo_0 @ 0] n:   0 pts:  99999\n[Parsed_showinfo_3 @ 000001f8] config in time_base: 1/10240, frame_rate: 20/1\n[Parsed_showinfo_3 @ 000001f8] config out time_base: 0/0, frame_rate: 0/0\n[Parsed_showinfo_3 @ 000001f8] n:   0 pts:      0 pts_time:0       duration:    512 fmt:gray s:128x72\n[Parsed_showinfo_3 @ 000001f8]   side data - H.26[45] User Data Unregistered SEI message\n[Parsed_showinfo_3 @ 000001f8] n:   1 pts:   5120 pts_time:0.5     duration:    512 fmt:gray s:128x72\r\n[Parsed_showinfo_3 @ 000001f8] color_range:pc color_space:unknown\n[Parsed_showinfo_3 @ 000001f8] n:   2 pts:  10240 pts_time:1       duration:    512 fmt:gray s:128x72\n[out#0/rawvideo @ 000001f8] video:27KiB\n";

    fn visual_window(
        lead_micros: u64,
        end_micros: u64,
    ) -> Result<VisualSamplingWindow, MediaError> {
        VisualSamplingWindow::new(
            0,
            0,
            MediaTime::from_micros(lead_micros),
            MediaTime::from_micros(end_micros),
        )
    }

    #[test]
    fn parses_visual_samples_when_frames_and_diagnostics_agree()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut stdout = vec![10_u8; VISUAL_FRAME_BYTES * 3];
        if let Some(byte) = stdout.get_mut(VISUAL_FRAME_BYTES) {
            *byte = 99;
        }
        let frames = parse_visual_samples(
            &stdout,
            VISUAL_DIAGNOSTIC.as_bytes(),
            &visual_window(0, 6_000_000)?,
        )?;
        let times: Vec<u64> = frames.iter().map(|frame| frame.time.as_micros()).collect();
        assert_eq!(times, vec![0, 500_000, 1_000_000]);
        assert_eq!(
            frames.get(1).and_then(|frame| frame.pixels.first()),
            Some(&99)
        );
        let origin = VisualSamplingWindow::new(
            0,
            -250_000,
            MediaTime::from_micros(250_000),
            MediaTime::from_micros(2_000_000),
        )?;
        let shifted = parse_visual_samples(&stdout, VISUAL_DIAGNOSTIC.as_bytes(), &origin)?;
        assert_eq!(
            shifted.first().map(|frame| frame.time.as_micros()),
            Some(250_000)
        );
        assert!(parse_visual_samples(&[], b"", &visual_window(0, 1)?)?.is_empty());
        Ok(())
    }

    #[test]
    fn rejects_visual_output_that_disagrees_with_its_diagnostics()
    -> Result<(), Box<dyn std::error::Error>> {
        let three = vec![0_u8; VISUAL_FRAME_BYTES * 3];
        let window = visual_window(0, 6_000_000)?;
        let diagnostic = VISUAL_DIAGNOSTIC.as_bytes();
        for (stdout, stderr) in [
            (vec![0_u8; VISUAL_FRAME_BYTES * 2], diagnostic.to_vec()),
            (vec![0_u8; VISUAL_FRAME_BYTES * 3 + 1], diagnostic.to_vec()),
            (
                three.clone(),
                VISUAL_DIAGNOSTIC
                    .replace("pts:  10240", "pts:   5120")
                    .into_bytes(),
            ),
            (
                three.clone(),
                VISUAL_DIAGNOSTIC.replace("n:   2", "n:   3").into_bytes(),
            ),
            (
                three.clone(),
                VISUAL_DIAGNOSTIC
                    .replace("config in time_base: 1/10240", "config in time_base: 0/1")
                    .into_bytes(),
            ),
            (
                three.clone(),
                format!("{VISUAL_DIAGNOSTIC}[Parsed_showinfo_3 @ 0] config in time_base: 1/1000\n")
                    .into_bytes(),
            ),
            (
                three.clone(),
                VISUAL_DIAGNOSTIC
                    .replace("pts:      0", "pts:   0x10")
                    .into_bytes(),
            ),
            (
                vec![0_u8; VISUAL_FRAME_BYTES],
                b"[Parsed_showinfo_0 @ 0] n:   0 pts: 0\n".to_vec(),
            ),
        ] {
            assert!(
                parse_visual_samples(&stdout, &stderr, &window).is_err(),
                "{}",
                String::from_utf8_lossy(&stderr)
            );
        }
        assert!(matches!(
            parse_visual_samples(&three, diagnostic, &visual_window(0, 1_000_000)?),
            Err(MediaError::InvalidDecodedOutput)
        ));
        assert!(matches!(
            parse_visual_samples(&three, diagnostic, &visual_window(1, 6_000_000)?),
            Err(MediaError::InvalidDecodedOutput)
        ));
        let mut invalid_utf8 = b"[Parsed_showinfo_3 @ 0] n:   0 pts: \xff".to_vec();
        invalid_utf8.push(b'\n');
        assert!(parse_visual_samples(&[], &invalid_utf8, &window).is_err());
        assert!(matches!(
            parse_visual_samples(
                &vec![0_u8; super::MAX_VISUAL_SAMPLE_BYTES + VISUAL_FRAME_BYTES],
                b"",
                &window
            ),
            Err(MediaError::OutputLimit)
        ));
        assert!(
            VisualSamplingWindow::new(0, 0, MediaTime::from_micros(5), MediaTime::from_micros(5))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn reads_wide_frame_numbers_written_without_padding() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut stderr =
            String::from("[Parsed_showinfo_3 @ 0] config in time_base: 1/1000, frame_rate: 2/1\n");
        for (number, pts) in [("0", "0"), ("1", "500"), ("2", "1000")] {
            stderr.push_str("[Parsed_showinfo_3 @ 0] n:");
            stderr.push_str(number);
            stderr.push_str(" pts:");
            stderr.push_str(pts);
            stderr.push_str(" pts_time:x\n");
        }
        let frames = parse_visual_samples(
            &vec![0_u8; VISUAL_FRAME_BYTES * 3],
            stderr.as_bytes(),
            &visual_window(0, 2_000_000)?,
        )?;
        assert_eq!(frames.len(), 3);
        Ok(())
    }
}
