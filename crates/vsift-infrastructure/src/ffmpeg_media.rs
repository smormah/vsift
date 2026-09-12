//! Restricted FFprobe/FFmpeg adapter over the trusted process supervisor.

use std::{error::Error, fmt, num::NonZeroUsize, time::Duration};

use serde::Deserialize;
use vsift_domain::{
    DisplayRotation, FrameDimensions, FrameTiming, MediaDecodeSupport, MediaDescription,
    MediaSelection, MediaStream, MediaStreamKind, MediaTime, StreamTime, TimeRange,
};

use crate::{
    FilesystemSessionStore, HostIsolation, ProcessCancellation, ProcessError, ProcessOutcome,
    ProcessRequest, ProcessRequestError, ProcessSupervisor, ProcessWorkingDirectory, SourceError,
    SourceSnapshot, SupervisorPolicy, TerminationReason, TrustedExecutable,
};

/// Hard bound for structured probe output.
pub const MAX_PROBE_BYTES: usize = 4 * 1024 * 1024;
/// Hard bound for provider diagnostics.
pub const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;
/// Hard bound for one in-memory image result.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
/// Hard bound for one extracted PCM chunk.
pub const MAX_AUDIO_BYTES: usize = 4 * 1024 * 1024;
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
    /// # Errors
    /// Fails on changed bytes, rejected media, oversized metadata, invalid timeline, or process failure.
    pub async fn probe(
        &self,
        source: &SourceSnapshot,
        cancellation: ProcessCancellation,
    ) -> Result<MediaDescription, MediaError> {
        let _admission = self
            .store
            .try_admit(1)
            .map_err(|_| MediaError::CapacityUnavailable)?;
        source.verify().map_err(MediaError::Source)?;
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
        parse_probe(&output.stdout.bytes, source.container())
    }

    /// Returns the first displayed frame at or after the requested normalized time.
    /// The reported PTS is parsed from `FFmpeg`'s selected decoded frame, never inferred
    /// from the requested timestamp or a constant frame rate.
    ///
    /// # Errors
    /// Rejects an absent stream, unsafe timestamp, timeout, oversized frame, or unmet tolerance.
    #[allow(clippy::too_many_lines)] // Keep validation, supervised run, and provenance checks in one auditable path.
    pub async fn frame(
        &self,
        source: &SourceSnapshot,
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
        source.verify().map_err(MediaError::Source)?;
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
        let raw_micros = i128::from(description.origin_micros) + i128::from(requested.as_micros());
        let raw = i64::try_from(raw_micros).map_err(|_| MediaError::InvalidTimeline)?;
        let seek = i64::try_from(requested.as_micros().saturating_sub(5_000_000))
            .map_err(|_| MediaError::InvalidTimeline)?;
        let filter = format!("select=gte(t\\,{}),showinfo", seconds_arg(raw));
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
        let (pts, numerator, denominator) = parse_showinfo(&output.stderr.bytes)?;
        let offset = description
            .origin_micros
            .checked_neg()
            .ok_or(MediaError::InvalidTimeline)?;
        let actual = StreamTime {
            stream_index: index,
            presentation_timestamp: pts,
            time_base_numerator: numerator,
            time_base_denominator: denominator,
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
    #[allow(clippy::too_many_lines)] // The bounded chunk and timestamp checks form one operation contract.
    pub async fn audio(
        &self,
        source: &SourceSnapshot,
        description: &MediaDescription,
        selection: MediaSelection,
        range: TimeRange,
        cancellation: ProcessCancellation,
    ) -> Result<ExtractedAudio, MediaError> {
        let _admission = self
            .store
            .try_admit(1)
            .map_err(|_| MediaError::CapacityUnavailable)?;
        source.verify().map_err(MediaError::Source)?;
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
        if range.end() > description.duration || range.duration_micros() > 10_000_000 {
            return Err(MediaError::InvalidAudioRange);
        }
        let seek =
            i64::try_from(range.start().as_micros()).map_err(|_| MediaError::InvalidAudioRange)?;
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
                "ashowinfo",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-f",
                "s16le",
                "pipe:1",
            ]);
        let output = self
            .supervisor(MAX_AUDIO_BYTES)
            .run(request, cancellation)
            .await
            .map_err(MediaError::Process)?;
        validate_outcome(&output)?;
        if output.stdout.bytes.is_empty()
            || output.stdout.bytes.len() % 2 != 0
            || output.stdout.bytes.len() > 320_000
        {
            return Err(MediaError::InvalidDecodedOutput);
        }
        let first_pts = parse_ashowinfo_time(&output.stderr.bytes)?;
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
        let stdout = NonZeroUsize::new(stdout_limit).unwrap_or(NonZeroUsize::MIN);
        let stderr = NonZeroUsize::new(MAX_DIAGNOSTIC_BYTES).unwrap_or(NonZeroUsize::MIN);
        let policy = SupervisorPolicy::default().with_stream_limits(stdout, stderr);
        ProcessSupervisor::new(policy, self.host_isolation)
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

fn parse_showinfo(bytes: &[u8]) -> Result<(i64, u32, u32), MediaError> {
    parse_filter_pts(bytes, "Parsed_showinfo_")
}

fn parse_ashowinfo_time(bytes: &[u8]) -> Result<i64, MediaError> {
    let text = std::str::from_utf8(bytes).map_err(|_| MediaError::InvalidDecodedOutput)?;
    for line in text
        .lines()
        .filter(|line| line.contains("Parsed_ashowinfo_") && line.contains("n:"))
    {
        for word in line.split_ascii_whitespace() {
            if let Some(value) = word.strip_prefix("pts_time:")
                && !value.is_empty()
            {
                return parse_seconds_micros(value).ok_or(MediaError::InvalidDecodedOutput);
            }
        }
    }
    Err(MediaError::InvalidDecodedOutput)
}

fn parse_filter_pts(bytes: &[u8], marker: &str) -> Result<(i64, u32, u32), MediaError> {
    let text = std::str::from_utf8(bytes).map_err(|_| MediaError::InvalidDecodedOutput)?;
    let mut base = None;
    let mut pts = None;
    for line in text.lines().filter(|line| line.contains(marker)) {
        if let Some((_, tail)) = line.split_once("time_base:")
            && let Some(parsed) = tail.split([',', ' ']).find_map(parse_time_base)
        {
            base = Some(parsed);
        }
        if line.contains("n:") && pts.is_none() {
            let words: Vec<_> = line.split_ascii_whitespace().collect();
            pts = words.iter().enumerate().find_map(|(position, word)| {
                let suffix = word.strip_prefix("pts:")?;
                if suffix.is_empty() {
                    words.get(position + 1)?.parse::<i64>().ok()
                } else {
                    suffix.parse::<i64>().ok()
                }
            });
        }
    }
    let (numerator, denominator) = base.ok_or(MediaError::InvalidDecodedOutput)?;
    Ok((
        pts.ok_or(MediaError::InvalidDecodedOutput)?,
        numerator,
        denominator,
    ))
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

#[allow(clippy::too_many_lines)] // Parse and validate the provider document before exposing any stream metadata.
fn parse_probe(
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
    use super::{MediaError, parse_probe, parse_showinfo};
    use crate::SourceContainer;
    use vsift_domain::{DisplayRotation, MediaDecodeSupport, MediaSelection, MediaStreamKind};

    const VALID: &str = r#"{"format":{"duration":"2.000000","start_time":"1.250000"},"streams":[{"index":0,"codec_type":"video","codec_name":"h264","time_base":"1/1000","start_pts":1250,"width":320,"height":240,"side_data_list":[{"rotation":-90}]},{"index":2,"codec_type":"audio","codec_name":"aac","time_base":"1/16000","start_pts":32000,"tags":{"language":"eng"}}]}"#;

    #[test]
    fn parses_nonzero_origin_rotation_and_explicit_tracks() -> Result<(), Box<dyn std::error::Error>>
    {
        let parsed = parse_probe(VALID.as_bytes(), SourceContainer::IsoMedia)?;
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
            parse_probe(huge.as_bytes(), SourceContainer::IsoMedia),
            Err(MediaError::InvalidDimensions)
        ));
        let missing = VALID.replace("\"duration\":\"2.000000\"", "\"duration\":\"N/A\"");
        assert!(matches!(
            parse_probe(missing.as_bytes(), SourceContainer::IsoMedia),
            Err(MediaError::InvalidDuration)
        ));
    }

    #[test]
    fn extracts_observed_pts_and_filter_clock() -> Result<(), Box<dyn std::error::Error>> {
        let diagnostic = b"[Parsed_showinfo_1] config in time_base: 1/16384, frame_rate: 2/1\n[Parsed_showinfo_1] n:   0 pts:  16384 pts_time:1\n";
        assert_eq!(parse_showinfo(diagnostic)?, (16_384, 1, 16_384));
        Ok(())
    }

    #[test]
    fn malformed_family_rejects_excessive_streams_and_dimensions() {
        let many = include_bytes!("../../../fixtures/corpus/generated/F11-excessive-streams.json");
        let huge =
            include_bytes!("../../../fixtures/corpus/generated/F11-oversized-dimensions.json");
        assert!(matches!(
            parse_probe(many, SourceContainer::IsoMedia),
            Err(MediaError::TooManyStreams)
        ));
        assert!(matches!(
            parse_probe(huge, SourceContainer::IsoMedia),
            Err(MediaError::InvalidDimensions)
        ));
    }

    #[test]
    fn preserves_missing_capabilities_and_marks_unknown_codec_unsupported()
    -> Result<(), Box<dyn std::error::Error>> {
        let no_video = br#"{"format":{"duration":"1.000000"},"streams":[{"index":3,"codec_type":"audio","codec_name":"future-codec","time_base":"1/1000"}]}"#;
        let description = parse_probe(no_video, SourceContainer::IsoMedia)?;
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
}
