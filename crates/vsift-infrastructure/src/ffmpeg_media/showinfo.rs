//! Strict readers of `FFmpeg`'s `showinfo` and `ashowinfo` diagnostics.
//!
//! The diagnostics are the only place `FFmpeg` reports which frames and
//! samples it actually decoded, so every reported time comes from here. They
//! share the stream with everything else `FFmpeg` logs at `-loglevel info`,
//! including the input's and the output's metadata (a `title`, a chapter
//! name), which the source author controls. So a reader:
//!
//! - reads only lines that **begin** with the filter's own `[Parsed_showinfo_`
//!   or `[Parsed_ashowinfo_` prefix. `FFmpeg` indents every metadata value it
//!   echoes, and turns a line break inside a value into another indented
//!   line, so echoed metadata can never start such a line (SEC-17);
//! - requires the frame lines to be numbered `0, 1, 2, ...` without a gap or
//!   repeat, and their timestamps to strictly increase. A line that did begin
//!   with the prefix but was not the filter's own (for example one forged
//!   through a log message that embeds an untrusted string with a line
//!   break) adds a frame the real numbering does not have, and the whole
//!   output is rejected;
//! - checks the count against what the caller asked for or what the other
//!   output stream holds, and the time base against the probed stream.
//!
//! Before 2026-09-26 the single-frame and audio readers accepted any line
//! that *contained* the prefix, so a crafted `title` could set the reported
//! frame time and time base, or the first audio sample time.

use vsift_domain::{
    FrameListing, ListedFrame, ListingTail, MAX_LISTED_FRAMES, MediaTime, StreamTime, TimeRange,
};

use super::{MediaError, parse_seconds_micros, parse_time_base};

/// Hard bound for one frame listing's diagnostics: 1,201 frame lines of
/// about 250 bytes, with configuration, colour and side-data lines.
pub const MAX_LISTING_DIAGNOSTIC_BYTES: usize = 1024 * 1024;
/// Most `ashowinfo` frame lines accepted. The speech and evidence audio
/// profiles regroup audio into 65,536-sample frames and the ten-second clip
/// profile logs one line per codec frame, so real output stays far below.
const MAX_AUDIO_FRAME_LINES: usize = 8_192;

const SHOWINFO_PREFIX: &[u8] = b"[Parsed_showinfo_";
const ASHOWINFO_PREFIX: &[u8] = b"[Parsed_ashowinfo_";

/// A `showinfo` frame line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ShowinfoFrame {
    /// Presentation timestamp in the filter's input time base.
    pub(super) pts: i64,
    /// The frame's size, `s:<width>x<height>`, when logged.
    pub(super) size: Option<(u32, u32)>,
}

/// Everything a reader takes from `showinfo` diagnostics.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ShowinfoLog {
    /// The filter's input time base, when a configuration line was logged.
    pub(super) time_base: Option<(u32, u32)>,
    /// Frame lines `0..n-1`, in order.
    pub(super) frames: Vec<ShowinfoFrame>,
}

/// Reads `showinfo` frame lines and the filter time base, accepting at most
/// `max_frames` frame lines.
///
/// Every `config in time_base` line must agree (a filter graph reconfigured
/// mid-stream, for example on a resolution change, repeats it), frame lines
/// must be numbered from zero without gaps and their timestamps must strictly
/// increase.
pub(super) fn scan_showinfo(stderr: &[u8], max_frames: usize) -> Result<ShowinfoLog, MediaError> {
    let mut log = ShowinfoLog::default();
    for rest in filter_lines(stderr, SHOWINFO_PREFIX)? {
        if let Some(config) = rest.strip_prefix("config in time_base:") {
            let parsed = config
                .split([',', ' '])
                .find_map(parse_time_base)
                .ok_or(MediaError::InvalidDecodedOutput)?;
            if log.time_base.is_some_and(|known| known != parsed) {
                return Err(MediaError::InvalidDecodedOutput);
            }
            log.time_base = Some(parsed);
        } else if rest.starts_with("n:") {
            if log.frames.len() >= max_frames {
                return Err(MediaError::InvalidDecodedOutput);
            }
            let words: Vec<&str> = rest.split_ascii_whitespace().collect();
            let number = labelled_integer(&words, "n:").ok_or(MediaError::InvalidDecodedOutput)?;
            let pts = labelled_integer(&words, "pts:").ok_or(MediaError::InvalidDecodedOutput)?;
            if usize::try_from(number).ok() != Some(log.frames.len())
                || log
                    .frames
                    .last()
                    .is_some_and(|previous| previous.pts >= pts)
            {
                return Err(MediaError::InvalidDecodedOutput);
            }
            let size = labelled_word(&words, "s:").map(parse_size).transpose()?;
            log.frames.push(ShowinfoFrame { pts, size });
        }
    }
    Ok(log)
}

/// One frame's observed presentation timestamp and the filter time base it
/// is expressed in.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ObservedFrameTime {
    /// Presentation timestamp, in `time_base_numerator / time_base_denominator` seconds.
    pub pts: i64,
    /// Positive time-base numerator.
    pub time_base_numerator: u32,
    /// Positive time-base denominator.
    pub time_base_denominator: u32,
}

/// Parses the diagnostics of a single-frame extraction: exactly one
/// `showinfo` frame line, numbered zero, and its filter time base.
///
/// The diagnostics are untrusted provider output, so this parser is
/// published beside [`super::parse_visual_samples`] for fuzzing (ADR 0016,
/// decision 6); see the module documentation for what it accepts.
///
/// # Errors
///
/// Returns [`MediaError::OutputLimit`] beyond [`super::MAX_DIAGNOSTIC_BYTES`]
/// and [`MediaError::InvalidDecodedOutput`] unless exactly one well-formed
/// frame line and a time base were logged.
pub fn parse_frame_showinfo(stderr: &[u8]) -> Result<ObservedFrameTime, MediaError> {
    if stderr.len() > super::MAX_DIAGNOSTIC_BYTES {
        return Err(MediaError::OutputLimit);
    }
    let log = scan_showinfo(stderr, 1)?;
    let (time_base_numerator, time_base_denominator) =
        log.time_base.ok_or(MediaError::InvalidDecodedOutput)?;
    let [frame] = log.frames.as_slice() else {
        return Err(MediaError::InvalidDecodedOutput);
    };
    Ok(ObservedFrameTime {
        pts: frame.pts,
        time_base_numerator,
        time_base_denominator,
    })
}

/// Parses the time of the first decoded audio sample, in microseconds after
/// the decode's own zero, from `ashowinfo` diagnostics.
///
/// Only lines that begin with `[Parsed_ashowinfo_` are read; their frames
/// must be numbered `0, 1, 2, ...` with strictly increasing timestamps, and
/// the result is frame zero's `pts_time`. Published for fuzzing like
/// [`parse_frame_showinfo`].
///
/// # Errors
///
/// Returns [`MediaError::OutputLimit`] beyond [`super::MAX_DIAGNOSTIC_BYTES`]
/// and [`MediaError::InvalidDecodedOutput`] when no frame was logged or the
/// lines are inconsistent.
pub fn parse_ashowinfo_start(stderr: &[u8]) -> Result<i64, MediaError> {
    if stderr.len() > super::MAX_DIAGNOSTIC_BYTES {
        return Err(MediaError::OutputLimit);
    }
    let mut first = None;
    let mut count: usize = 0;
    let mut previous: Option<i64> = None;
    for rest in filter_lines(stderr, ASHOWINFO_PREFIX)? {
        if !rest.starts_with("n:") {
            continue;
        }
        if count >= MAX_AUDIO_FRAME_LINES {
            return Err(MediaError::InvalidDecodedOutput);
        }
        let words: Vec<&str> = rest.split_ascii_whitespace().collect();
        let number = labelled_integer(&words, "n:").ok_or(MediaError::InvalidDecodedOutput)?;
        let pts = labelled_integer(&words, "pts:").ok_or(MediaError::InvalidDecodedOutput)?;
        if usize::try_from(number).ok() != Some(count) || previous.is_some_and(|last| last >= pts) {
            return Err(MediaError::InvalidDecodedOutput);
        }
        if count == 0 {
            let time =
                labelled_word(&words, "pts_time:").ok_or(MediaError::InvalidDecodedOutput)?;
            first = Some(parse_seconds_micros(time).ok_or(MediaError::InvalidDecodedOutput)?);
        }
        previous = Some(pts);
        count += 1;
    }
    first.ok_or(MediaError::InvalidDecodedOutput)
}

/// The expected shape of one frame listing: which stream, how its
/// timestamps map to source time, and which range was listed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameListingWindow {
    stream_index: u32,
    time_base: (u32, u32),
    origin_micros: i64,
    covered: TimeRange,
    tail: ListingTail,
    first_pts: i64,
    end_pts: i64,
}

impl FrameListingWindow {
    /// Describes a listing of `stream_index` (time base
    /// `numerator/denominator`) over the normalized range `covered`, for a
    /// source whose earliest stream starts at `origin_micros` of raw stream
    /// time; `tail` says whether `covered` ends where the source ends.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::InvalidTimeline`] when the range's timestamp
    /// bounds cannot be expressed in the stream's time base.
    pub fn new(
        stream_index: u32,
        time_base_numerator: u32,
        time_base_denominator: u32,
        origin_micros: i64,
        covered: TimeRange,
        tail: ListingTail,
    ) -> Result<Self, MediaError> {
        let offset = origin_micros
            .checked_neg()
            .ok_or(MediaError::InvalidTimeline)?;
        let bound = |time| {
            StreamTime::first_at_or_after(
                stream_index,
                time_base_numerator,
                time_base_denominator,
                offset,
                time,
            )
            .map(|stream| stream.presentation_timestamp)
            .map_err(|_| MediaError::InvalidTimeline)
        };
        Ok(Self {
            stream_index,
            time_base: (time_base_numerator, time_base_denominator),
            origin_micros,
            covered,
            tail,
            first_pts: bound(covered.start())?,
            end_pts: bound(covered.end())?,
        })
    }

    /// The first stream timestamp inside the range.
    #[must_use]
    pub const fn first_pts(&self) -> i64 {
        self.first_pts
    }

    /// The first stream timestamp after the range.
    #[must_use]
    pub const fn end_pts(&self) -> i64 {
        self.end_pts
    }

    /// Normalizes one timestamp of this stream.
    pub(super) fn time_of(&self, pts: i64) -> Result<MediaTime, MediaError> {
        normalize(self.stream_index, pts, self.time_base, self.origin_micros)
    }
}

/// Normalizes a stream timestamp through the source origin.
pub(super) fn normalize(
    stream_index: u32,
    pts: i64,
    (numerator, denominator): (u32, u32),
    origin_micros: i64,
) -> Result<MediaTime, MediaError> {
    StreamTime {
        stream_index,
        presentation_timestamp: pts,
        time_base_numerator: numerator,
        time_base_denominator: denominator,
        timeline_offset_micros: origin_micros
            .checked_neg()
            .ok_or(MediaError::InvalidTimeline)?,
    }
    .to_media_time()
    .map_err(|_| MediaError::InvalidTimeline)
}

/// Parses a frame listing's `showinfo` diagnostics into a [`FrameListing`].
///
/// The listing run selects the frames whose timestamps lie in
/// `[first_pts, end_pts)` and stops after [`MAX_LISTED_FRAMES`] + 1 of them.
/// Frame lines must be numbered from zero, strictly increasing, inside those
/// bounds, and expressed in the probed stream's time base
/// ([`MediaError::TimeBaseMismatch`] otherwise). When the extra frame was
/// reached the listing is cut just before it: it then covers the window's
/// start up to that frame's time, completely, and more frames may follow.
/// Published for fuzzing like [`parse_frame_showinfo`].
///
/// # Errors
///
/// Returns [`MediaError::OutputLimit`] beyond
/// [`MAX_LISTING_DIAGNOSTIC_BYTES`], [`MediaError::TimeBaseMismatch`] for a
/// filter time base other than the stream's, and
/// [`MediaError::InvalidDecodedOutput`] or [`MediaError::InvalidTimeline`]
/// for anything inconsistent; parsing never partially succeeds.
pub fn parse_frame_listing(
    stderr: &[u8],
    window: &FrameListingWindow,
) -> Result<FrameListing, MediaError> {
    if stderr.len() > MAX_LISTING_DIAGNOSTIC_BYTES {
        return Err(MediaError::OutputLimit);
    }
    let log = scan_showinfo(stderr, MAX_LISTED_FRAMES + 1)?;
    if !log.frames.is_empty() && log.time_base != Some(window.time_base) {
        return Err(if log.time_base.is_some() {
            MediaError::TimeBaseMismatch
        } else {
            MediaError::InvalidDecodedOutput
        });
    }
    let mut frames = Vec::with_capacity(log.frames.len());
    for frame in &log.frames {
        if frame.pts < window.first_pts || frame.pts >= window.end_pts {
            return Err(MediaError::InvalidDecodedOutput);
        }
        frames.push(ListedFrame {
            pts: frame.pts,
            time: window.time_of(frame.pts)?,
        });
    }
    let (covered, tail) = if frames.len() > MAX_LISTED_FRAMES {
        let cut = frames
            .get(MAX_LISTED_FRAMES)
            .map(|frame| frame.time)
            .ok_or(MediaError::InvalidDecodedOutput)?;
        frames.retain(|frame| frame.time < cut);
        let covered = TimeRange::new(window.covered.start(), cut)
            .map_err(|_| MediaError::InvalidDecodedOutput)?;
        (covered, ListingTail::MoreMayFollow)
    } else {
        (window.covered, window.tail)
    };
    FrameListing::new(covered, frames, tail).map_err(|_| MediaError::InvalidDecodedOutput)
}

/// The text after `[<prefix>... @ <address>] ` on every line that begins
/// with `prefix`. Other lines are never decoded as text.
fn filter_lines<'a>(stderr: &'a [u8], prefix: &[u8]) -> Result<Vec<&'a str>, MediaError> {
    let mut lines = Vec::new();
    for line in stderr.split(|byte| *byte == b'\n') {
        if !line.starts_with(prefix) {
            continue;
        }
        let line = std::str::from_utf8(line).map_err(|_| MediaError::InvalidDecodedOutput)?;
        let (_, rest) = line
            .split_once("] ")
            .ok_or(MediaError::InvalidDecodedOutput)?;
        lines.push(rest.trim_end_matches('\r'));
    }
    Ok(lines)
}

/// Reads the integer after `label`, written as `label123` or `label 123`.
fn labelled_integer(words: &[&str], label: &str) -> Option<i64> {
    labelled_word(words, label)?.parse::<i64>().ok()
}

/// Reads the value after `label`, written as `labelvalue` or `label value`.
fn labelled_word<'a>(words: &[&'a str], label: &str) -> Option<&'a str> {
    words.iter().enumerate().find_map(|(position, word)| {
        let suffix = word.strip_prefix(label)?;
        if suffix.is_empty() {
            words.get(position + 1).copied()
        } else {
            Some(suffix)
        }
    })
}

/// Reads `<width>x<height>`.
fn parse_size(text: &str) -> Result<(u32, u32), MediaError> {
    let (width, height) = text
        .split_once('x')
        .ok_or(MediaError::InvalidDecodedOutput)?;
    let parse = |value: &str| {
        if value.bytes().all(|byte| byte.is_ascii_digit()) {
            value
                .parse::<u32>()
                .map_err(|_| MediaError::InvalidDecodedOutput)
        } else {
            Err(MediaError::InvalidDecodedOutput)
        }
    };
    Ok((parse(width)?, parse(height)?))
}

#[cfg(test)]
mod tests;
