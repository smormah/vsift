//! On-screen text read off a session's video frames (OCR, ADR 0030).
//!
//! Text read off the screen is derived evidence: it helps find a moment, and it
//! is never promoted to what the video shows ([R1 invariant 2], SEC-28). Each
//! line keeps the exact frame it was read from (stream, timestamp and time), its
//! region in that frame's displayed pixels, the engine's own uncalibrated
//! confidence, and, through its index, the engine and models that read it.
//!
//! **Which frames are read.** A call plans *read targets* over a range: the
//! representative frame of every visual candidate in it, and one frame every
//! period (ten seconds unless the caller chooses). Each target resolves to the
//! first displayed frame at or after its time. A frame read once is never read
//! again by the same engine; a later call reads the targets still pending, so a
//! long recording is read over several bounded calls.
//!
//! **What an index holds.** The index is the session's set of frame readings by
//! one engine build and one profile, revised as calls add frames: every
//! revision holds every frame of the one before. A different engine build
//! starts a new index rather than mixing readings.
//!
//! [R1 invariant 2]: ../../docs/planning/r1-industrial-capability-expansion.md

use std::{collections::BTreeSet, error::Error, fmt, num::NonZeroU32};

use crate::{
    Confidence, FrameDimensions, FrameRef, MediaTime, ScreenTextIndexId, ScreenTextLineId,
    SessionId, Sha256Hex, SourceId, TimeRange, VisualCandidateId,
};

/// Most frames one index holds.
pub const MAX_SCREEN_TEXT_FRAMES: usize = 4096;
/// Most lines kept of one frame; a frame with more says it was truncated.
pub const MAX_SCREEN_TEXT_LINES_PER_FRAME: usize = 256;
/// Most lines one index holds over all its frames.
pub const MAX_SCREEN_TEXT_LINES: usize = 200_000;
/// Most characters of one line of screen text.
pub const MAX_SCREEN_TEXT_CHARS: usize = 512;
/// Most read targets that resolved to no frame one index remembers.
pub const MAX_UNRESOLVED_TARGETS: usize = 4096;
/// The default period of the frames read between candidates: one every ten seconds.
pub const DEFAULT_READ_PERIOD_MICROS: u64 = 10_000_000;
/// The shortest period a caller may choose: one frame a second.
pub const MIN_READ_PERIOD_MICROS: u64 = 1_000_000;
/// The longest period a caller may choose: one frame every ten minutes.
pub const MAX_READ_PERIOD_MICROS: u64 = 600_000_000;
/// Most bytes of an engine's version label.
pub const MAX_PROVIDER_LABEL_BYTES: usize = 64;

/// Why screen-text data was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenTextError {
    /// A line's text is empty, too long or holds a control character.
    InvalidText,
    /// A region is empty or reaches outside its frame.
    InvalidRegion,
    /// An engine label is empty, too long or not in its alphabet.
    InvalidLabel,
    /// A period is outside its bounds.
    InvalidPeriod,
    /// Frames are out of order, repeated, of another stream, or without a target.
    InvalidFrames,
    /// A bound on frames, lines or unresolved targets is exceeded.
    TooLarge,
    /// A line identity repeats within the index.
    DuplicateLine,
}

impl fmt::Display for ScreenTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidText => "screen text is empty, too long or holds a control character",
            Self::InvalidRegion => "a screen-text region is empty or outside its frame",
            Self::InvalidLabel => "an OCR engine label is invalid",
            Self::InvalidPeriod => "the read period is outside its bounds",
            Self::InvalidFrames => {
                "screen-text frames are out of order, repeated or of another stream"
            }
            Self::TooLarge => "a screen-text index exceeds its bounds",
            Self::DuplicateLine => "a screen-text line identity repeats",
        })
    }
}

impl Error for ScreenTextError {}

/// The reading profile: which models, settings and pipeline produced the text.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ScreenTextProfile {
    /// `PP-OCRv5` mobile detection and its Latin recogniser, upright lines, lines of
    /// confidence 0.5 or more (ADR 0030; `docs/planning/ocr-engine-evaluation.md`).
    PpOcrV5LatinV1,
}

impl ScreenTextProfile {
    /// Every profile.
    pub const ALL: [Self; 1] = [Self::PpOcrV5LatinV1];

    /// Stable identifier written to records and results.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::PpOcrV5LatinV1 => "ppocrv5-latin-v1",
        }
    }

    /// The profile with this identifier.
    #[must_use]
    pub fn from_identifier(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|profile| profile.identifier() == value)
    }
}

/// A version label an engine reports about itself (`0.1.0`, `1.31.0`).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProviderLabel(String);

impl ProviderLabel {
    /// Validates a label of 1 to [`MAX_PROVIDER_LABEL_BYTES`] ASCII letters, digits and `.`, `_`, `+`, `-`.
    ///
    /// # Errors
    ///
    /// [`ScreenTextError::InvalidLabel`] otherwise.
    pub fn parse(value: impl Into<String>) -> Result<Self, ScreenTextError> {
        let value = value.into();
        let allowed = |character: char| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '+' | '-')
        };
        if value.is_empty() || value.len() > MAX_PROVIDER_LABEL_BYTES || !value.chars().all(allowed)
        {
            return Err(ScreenTextError::InvalidLabel);
        }
        Ok(Self(value))
    }

    /// The label as validated.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The engine build that read an index's frames: the runner, its runtime and its two models.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTextProvider {
    /// SHA-256 of the runner executable.
    pub runner_sha256: Sha256Hex,
    /// The runner's own version.
    pub runner_version: ProviderLabel,
    /// The ONNX Runtime version the runner loaded.
    pub runtime_version: ProviderLabel,
    /// SHA-256 of the detection model.
    pub detector_sha256: Sha256Hex,
    /// SHA-256 of the recognition model.
    pub recognizer_sha256: Sha256Hex,
}

/// One line of text as read: non-empty, at most [`MAX_SCREEN_TEXT_CHARS`], no control characters.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ScreenText(String);

impl ScreenText {
    /// Validates engine text.
    ///
    /// # Errors
    ///
    /// [`ScreenTextError::InvalidText`] for empty, too long or control-bearing text.
    pub fn parse(value: impl Into<String>) -> Result<Self, ScreenTextError> {
        let value = value.into();
        if value.trim().is_empty()
            || value.chars().count() > MAX_SCREEN_TEXT_CHARS
            || value.chars().any(char::is_control)
        {
            return Err(ScreenTextError::InvalidText);
        }
        Ok(Self(value))
    }

    /// The text as read.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An upright rectangle of a frame's displayed pixels.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl TextRegion {
    /// A region that lies inside a frame of `frame` dimensions.
    ///
    /// # Errors
    ///
    /// [`ScreenTextError::InvalidRegion`] for an empty region or one reaching outside the frame.
    pub fn new(
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        frame: FrameDimensions,
    ) -> Result<Self, ScreenTextError> {
        let fits = |start: u32, length: u32, limit: u32| {
            start.checked_add(length).is_some_and(|end| end <= limit)
        };
        if width == 0
            || height == 0
            || !fits(x, width, frame.width())
            || !fits(y, height, frame.height())
        {
            return Err(ScreenTextError::InvalidRegion);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    /// Left edge.
    #[must_use]
    pub const fn x(self) -> u32 {
        self.x
    }

    /// Top edge.
    #[must_use]
    pub const fn y(self) -> u32 {
        self.y
    }

    /// Width.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Height.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    /// Whether the two regions overlap by at least half of the smaller one: the
    /// same place on two frames of one screen, give or take a few pixels.
    #[must_use]
    pub fn same_place(self, other: Self) -> bool {
        let overlap = |a: u32, a_len: u32, b: u32, b_len: u32| {
            let start = a.max(b);
            let end = (a + a_len).min(b + b_len);
            u64::from(end.saturating_sub(start))
        };
        let shared = overlap(self.x, self.width, other.x, other.width)
            * overlap(self.y, self.height, other.y, other.height);
        let smaller = (u64::from(self.width) * u64::from(self.height))
            .min(u64::from(other.width) * u64::from(other.height));
        shared * 2 >= smaller
    }
}

/// One line read off a frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTextLine {
    /// Stable identity of the line.
    pub id: ScreenTextLineId,
    /// Where it lies on the frame.
    pub region: TextRegion,
    /// What was read.
    pub text: ScreenText,
    /// The engine's own score (uncalibrated).
    pub confidence: Confidence,
}

/// A time the plan asked to read, and the candidate it stands for, if any.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ReadTarget {
    /// The requested time; the frame read is the first displayed at or after it.
    pub time: MediaTime,
    /// The candidate whose representative frame this is; `None` for a periodic target.
    pub candidate: Option<VisualCandidateId>,
}

/// Why a frame was planned but gave no text reading.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ScreenTextFailure {
    /// The engine refused the frame's image.
    ImageRejected,
    /// The engine ran but its reading of the frame could not be used.
    EngineFailed,
}

impl ScreenTextFailure {
    /// Every failure.
    pub const ALL: [Self; 2] = [Self::ImageRejected, Self::EngineFailed];

    /// Stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::ImageRejected => "image_rejected",
            Self::EngineFailed => "engine_failed",
        }
    }
}

/// What reading a frame gave.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScreenTextOutcome {
    /// The frame was read: its lines in reading order.
    Read {
        /// The lines, at most [`MAX_SCREEN_TEXT_LINES_PER_FRAME`].
        lines: Vec<ScreenTextLine>,
        /// True when the frame held more lines than were kept.
        truncated: bool,
    },
    /// The frame could not be read.
    Failed(ScreenTextFailure),
}

/// One frame of an index: which frame, why it was read, and what reading gave.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTextFrame {
    /// The exact frame.
    pub frame: FrameRef,
    /// The targets that resolved to it, in order; at least one.
    pub targets: Vec<ReadTarget>,
    /// The reading.
    pub outcome: ScreenTextOutcome,
}

impl ScreenTextFrame {
    /// The lines, empty for a failed frame.
    #[must_use]
    pub fn lines(&self) -> &[ScreenTextLine] {
        match &self.outcome {
            ScreenTextOutcome::Read { lines, .. } => lines,
            ScreenTextOutcome::Failed(_) => &[],
        }
    }
}

/// What an index is of and by whom: the identity every revision shares.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTextScope {
    /// The session.
    pub session_id: SessionId,
    /// The session's source.
    pub source_id: SourceId,
    /// The video stream read.
    pub stream_index: u32,
    /// How the text was read.
    pub profile: ScreenTextProfile,
    /// Which engine build read it.
    pub provider: ScreenTextProvider,
}

/// The parts of a new index revision, checked by [`ScreenTextIndex::new`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTextIndexParts {
    /// The revision's identity.
    pub id: ScreenTextIndexId,
    /// The revision number, from 1.
    pub revision: NonZeroU32,
    /// What the index is of and by whom.
    pub scope: ScreenTextScope,
    /// The period of the latest call's plan, in microseconds.
    pub period_micros: u64,
    /// The frames read, in timestamp order.
    pub frames: Vec<ScreenTextFrame>,
    /// Targets that resolved to no frame (none displayed at or after them before the end).
    pub unresolved: Vec<ReadTarget>,
}

/// One immutable revision of a session's on-screen text index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTextIndex {
    parts: ScreenTextIndexParts,
}

impl ScreenTextIndex {
    /// Checks and builds a revision.
    ///
    /// # Errors
    ///
    /// [`ScreenTextError`] for a period outside its bounds, frames out of
    /// timestamp order, repeated or of another stream, a frame without a
    /// target, a bound exceeded, or a repeated line identity.
    pub fn new(mut parts: ScreenTextIndexParts) -> Result<Self, ScreenTextError> {
        if !(MIN_READ_PERIOD_MICROS..=MAX_READ_PERIOD_MICROS).contains(&parts.period_micros) {
            return Err(ScreenTextError::InvalidPeriod);
        }
        if parts.frames.len() > MAX_SCREEN_TEXT_FRAMES
            || parts.unresolved.len() > MAX_UNRESOLVED_TARGETS
        {
            return Err(ScreenTextError::TooLarge);
        }
        let mut lines = BTreeSet::new();
        let mut previous_pts = None;
        for frame in &mut parts.frames {
            if frame.frame.stream_index != parts.scope.stream_index
                || previous_pts.is_some_and(|pts| frame.frame.pts <= pts)
                || frame.targets.is_empty()
            {
                return Err(ScreenTextError::InvalidFrames);
            }
            previous_pts = Some(frame.frame.pts);
            frame.targets.sort();
            frame.targets.dedup();
            if frame.lines().len() > MAX_SCREEN_TEXT_LINES_PER_FRAME {
                return Err(ScreenTextError::TooLarge);
            }
            for line in frame.lines() {
                if !lines.insert(line.id.clone()) {
                    return Err(ScreenTextError::DuplicateLine);
                }
                TextRegion::new(
                    line.region.x,
                    line.region.y,
                    line.region.width,
                    line.region.height,
                    frame.frame.dimensions,
                )?;
            }
            if lines.len() > MAX_SCREEN_TEXT_LINES {
                return Err(ScreenTextError::TooLarge);
            }
        }
        parts.unresolved.sort();
        parts.unresolved.dedup();
        Ok(Self { parts })
    }

    /// The revision's identity.
    #[must_use]
    pub const fn id(&self) -> &ScreenTextIndexId {
        &self.parts.id
    }

    /// The revision number.
    #[must_use]
    pub const fn revision(&self) -> NonZeroU32 {
        self.parts.revision
    }

    /// What the index is of and by whom.
    #[must_use]
    pub const fn scope(&self) -> &ScreenTextScope {
        &self.parts.scope
    }

    /// The period of the latest call's plan.
    #[must_use]
    pub const fn period_micros(&self) -> u64 {
        self.parts.period_micros
    }

    /// The frames, in timestamp order.
    #[must_use]
    pub fn frames(&self) -> &[ScreenTextFrame] {
        &self.parts.frames
    }

    /// Targets that resolved to no frame.
    #[must_use]
    pub fn unresolved(&self) -> &[ReadTarget] {
        &self.parts.unresolved
    }

    /// The frame with this timestamp, if read.
    #[must_use]
    pub fn frame_at_pts(&self, pts: i64) -> Option<&ScreenTextFrame> {
        self.parts
            .frames
            .binary_search_by_key(&pts, |frame| frame.frame.pts)
            .ok()
            .and_then(|index| self.parts.frames.get(index))
    }

    /// Whether `target` was attempted: it resolved to a frame of the index, or to none.
    #[must_use]
    pub fn attempted(&self, target: &ReadTarget) -> bool {
        self.parts.unresolved.binary_search(target).is_ok()
            || self
                .parts
                .frames
                .iter()
                .any(|frame| frame.targets.binary_search(target).is_ok())
    }

    /// The parts, for a new revision built on this one.
    #[must_use]
    pub fn into_parts(self) -> ScreenTextIndexParts {
        self.parts
    }
}

/// The plan of a range: the representative frame of every candidate in it, and
/// one frame every `period_micros` from the period's first multiple at or after the
/// range's start, in time order.
///
/// # Errors
///
/// [`ScreenTextError::InvalidPeriod`] for a period outside its bounds.
pub fn plan_reads(
    range: TimeRange,
    candidates: &[(VisualCandidateId, MediaTime)],
    period_micros: u64,
) -> Result<Vec<ReadTarget>, ScreenTextError> {
    if !(MIN_READ_PERIOD_MICROS..=MAX_READ_PERIOD_MICROS).contains(&period_micros) {
        return Err(ScreenTextError::InvalidPeriod);
    }
    let start = range.start().as_micros();
    let end = range.end().as_micros();
    let mut targets: Vec<ReadTarget> = candidates
        .iter()
        .filter(|(_, time)| (start..end).contains(&time.as_micros()))
        .map(|(candidate, time)| ReadTarget {
            time: *time,
            candidate: Some(candidate.clone()),
        })
        .collect();
    let mut at = start.div_ceil(period_micros).saturating_mul(period_micros);
    while at < end {
        targets.push(ReadTarget {
            time: MediaTime::from_micros(at),
            candidate: None,
        });
        at = at.saturating_add(period_micros);
    }
    targets.sort();
    targets.dedup();
    Ok(targets)
}

/// The plan of `range` restricted to analysed windows: the candidates there and
/// the periodic reads that fall in a window the visual index has recorded
/// (analysed or undecodable), and how many periodic reads were left out because
/// their window is not analysed yet. A caller that must never start visual
/// analysis itself (a background job that reads text after the windows are
/// analysed) plans with this; the reads left out join the plan once their
/// windows are analysed.
///
/// # Errors
///
/// [`ScreenTextError::InvalidPeriod`] for a period outside its bounds.
pub fn plan_reads_in_analysed_windows(
    range: TimeRange,
    candidates: &[(VisualCandidateId, MediaTime)],
    period_micros: u64,
    analysed: impl Fn(MediaTime) -> bool,
) -> Result<(Vec<ReadTarget>, usize), ScreenTextError> {
    let whole = plan_reads(range, candidates, period_micros)?;
    let planned = whole.len();
    let kept: Vec<ReadTarget> = whole
        .into_iter()
        .filter(|target| target.candidate.is_some() || analysed(target.time))
        .collect();
    let awaiting = planned - kept.len();
    Ok((kept, awaiting))
}

/// How much of a plan an index has read.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScreenTextCoverage {
    /// Targets planned.
    pub planned: usize,
    /// Targets whose frame was read.
    pub read: usize,
    /// Targets whose frame could not be read.
    pub failed: usize,
    /// Targets that resolved to no frame.
    pub unresolved: usize,
    /// Targets not yet attempted.
    pub pending: usize,
    /// Periodic reads of the range left out of the plan because their visual
    /// window is not analysed yet ([`plan_reads_in_analysed_windows`]); always 0
    /// for a plan of the whole range. Not part of `planned`.
    pub awaiting_analysis: usize,
}

impl ScreenTextCoverage {
    /// The coverage of `plan` by `index` (none: nothing attempted).
    #[must_use]
    pub fn of(plan: &[ReadTarget], index: Option<&ScreenTextIndex>) -> Self {
        let mut coverage = Self {
            planned: plan.len(),
            ..Self::default()
        };
        for target in plan {
            let Some(index) = index else {
                coverage.pending += 1;
                continue;
            };
            if index.unresolved().binary_search(target).is_ok() {
                coverage.unresolved += 1;
            } else if let Some(frame) = index
                .frames()
                .iter()
                .find(|frame| frame.targets.binary_search(target).is_ok())
            {
                match frame.outcome {
                    ScreenTextOutcome::Read { .. } => coverage.read += 1,
                    ScreenTextOutcome::Failed(_) => coverage.failed += 1,
                }
            } else {
                coverage.pending += 1;
            }
        }
        coverage
    }

    /// Every planned target was attempted and read or found to have no frame,
    /// and no read waits for its window to be analysed.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        self.pending == 0 && self.failed == 0 && self.awaiting_analysis == 0
    }
}

#[cfg(test)]
mod tests;
