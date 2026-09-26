//! Visual-candidate analysis and the session visual index (P08).
//!
//! An agent cannot look at every frame of a video, so `VSift` proposes
//! *candidates*: moments at which the screen changed, plus a periodic sample
//! so that a static screen is still represented. This module holds the whole
//! rule, pure and integer-only, so the same samples always give the same
//! candidates on every platform:
//!
//! - **Samples.** The media adapter decodes actual frames at most every 0.5 s
//!   (2 Hz), downscaled to 128x72 grey. A [`VisualSample`] keeps the rounded
//!   mean of each 8x8 block (a 16x9 grid of 144 blocks) and a 64-bit
//!   difference hash. Nothing else of the image is kept.
//! - **Windows.** Source time is cut into fixed 60 s windows
//!   (`[60k s, min(60(k+1) s, duration))`) that never merge. Each window is
//!   analysed on its own, from its own samples and one *lead-in* sample taken
//!   up to 0.5 s before it, so a change exactly at a window boundary is still
//!   seen, and so candidate identities (derived from the window ordinal and
//!   the candidate's time) never change when later windows are analysed
//!   (V-05 by construction).
//! - **Changes.** [`VisualChangePolicy::R0`] compares each sample with the
//!   first sample of the current screen state (its anchor, so slow drift is
//!   caught) and with the previous sample.
//! - **Merging with time preserved.** Consecutive unchanged samples collapse
//!   into one candidate whose span runs to the next candidate or the window
//!   end, with its sample count. A screen that reappears later (A-B-A) is a
//!   separate candidate with the same visual hash, never merged across time.
//! - **Motion.** Three or more consecutive samples that each differ from the
//!   one before are one `motion_start` candidate (`in_motion`) followed by a
//!   `settled_after_motion` candidate once the screen holds still.
//! - **Coverage.** Every 10 s cell of source time that holds a decoded frame
//!   has at least one candidate; if no change falls in it, its first sample
//!   becomes a `periodic_coverage` candidate. A cell with no decoded frame is
//!   recorded as a gap.
//! - **Budget.** At most 32 candidates per window. The window's first
//!   candidate and the coverage candidates are always kept; beyond that the
//!   highest-scoring changes are kept (ties to the earlier time) and the
//!   window records how many were dropped.

use std::{collections::HashSet, error::Error, fmt, num::NonZeroU32};

use crate::{FrameDimensions, MediaTime, SourceId, TimeRange, VisualCandidateId, VisualIndexId};

/// Length of one analysis window: 60 s.
pub const VISUAL_WINDOW_MICROS: u64 = 60_000_000;
/// Length of one coverage cell: 10 s. Every cell with a decoded frame has a candidate.
pub const VISUAL_CELL_MICROS: u64 = 10_000_000;
/// Minimum spacing between two samples: 0.5 s, so at most 2 Hz.
pub const VISUAL_SAMPLE_INTERVAL_MICROS: u64 = 500_000;
/// How far before a window its lead-in sample may lie.
pub const VISUAL_LEAD_IN_MICROS: u64 = 500_000;
/// Width of a decoded sample frame in pixels.
pub const VISUAL_FRAME_WIDTH: usize = 128;
/// Height of a decoded sample frame in pixels.
pub const VISUAL_FRAME_HEIGHT: usize = 72;
/// Bytes of one decoded 8-bit grey sample frame.
pub const VISUAL_FRAME_BYTES: usize = VISUAL_FRAME_WIDTH * VISUAL_FRAME_HEIGHT;
/// Columns of the 8x8 block grid.
pub const VISUAL_BLOCK_COLUMNS: usize = 16;
/// Rows of the 8x8 block grid.
pub const VISUAL_BLOCK_ROWS: usize = 9;
/// Blocks per sample.
pub const VISUAL_BLOCKS: usize = VISUAL_BLOCK_COLUMNS * VISUAL_BLOCK_ROWS;
/// Most samples one window's decode may return: 121 at exact 0.5 s spacing
/// over the 60.5 s from the lead-in to the window end, plus one of slack.
pub const MAX_WINDOW_SAMPLES: usize = 122;
/// Most candidates one window keeps.
pub const MAX_WINDOW_CANDIDATES: usize = 32;
/// Longest source a visual index covers: the four-hour source bound.
pub const MAX_VISUAL_INDEX_DURATION_MICROS: u64 = 4 * 60 * 60 * 1_000_000;

const BLOCK_EDGE: usize = 8;
const BLOCK_PIXELS: u32 = 64;
const HASH_COLUMNS: usize = 9;
const HASH_ROWS: usize = 8;

/// The reviewed visual-analysis profile; part of every candidate identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum VisualIndexProfile {
    /// 2 Hz grey 128x72 samples, 16x9 block means, [`VisualChangePolicy::R0`].
    R0,
}

impl VisualIndexProfile {
    /// Stable identifier stored in records and hashed into identities.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::R0 => "r0-visual-v1",
        }
    }

    /// The change policy this profile applies.
    #[must_use]
    pub const fn policy(self) -> VisualChangePolicy {
        match self {
            Self::R0 => VisualChangePolicy::R0,
        }
    }
}

/// A 64-bit difference hash of one sample.
///
/// Two samples of the same screen have the same hash in practice, which lets
/// an agent see that a later candidate repeats an earlier screen (A-B-A)
/// without the index merging them across time. It is a similarity key, not a
/// security digest.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VisualHash(u64);

impl VisualHash {
    /// Wraps hash bits.
    #[must_use]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// Returns the hash bits.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Returns the canonical 16-digit lowercase hexadecimal form.
    #[must_use]
    pub fn to_hex(self) -> String {
        format!("{:016x}", self.0)
    }

    /// Parses the canonical 16-digit lowercase hexadecimal form.
    ///
    /// # Errors
    ///
    /// Returns [`VisualIndexError::InvalidHash`] for any other text.
    pub fn parse_hex(text: &str) -> Result<Self, VisualIndexError> {
        if text.len() != 16
            || !text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(VisualIndexError::InvalidHash);
        }
        u64::from_str_radix(text, 16)
            .map(Self)
            .map_err(|_| VisualIndexError::InvalidHash)
    }
}

/// One decoded frame reduced to what change detection needs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualSample {
    time: MediaTime,
    blocks: [u8; VISUAL_BLOCKS],
    hash: VisualHash,
}

impl VisualSample {
    /// Reduces one 128x72 8-bit grey frame decoded at `time`.
    ///
    /// Block means are rounded half up over each 8x8 block. The hash is a
    /// difference hash over a 9x8 grid of area means (column edges at
    /// `128 * i / 9`, row edges every 9 pixels): bit `8 * row + column`,
    /// most significant first, is set when a cell is brighter than its right
    /// neighbour. Everything is integer arithmetic, so results are identical
    /// on every platform.
    #[must_use]
    pub fn from_gray(time: MediaTime, pixels: &[u8; VISUAL_FRAME_BYTES]) -> Self {
        let mut blocks = [0_u8; VISUAL_BLOCKS];
        for (index, block) in blocks.iter_mut().enumerate() {
            let left = (index % VISUAL_BLOCK_COLUMNS) * BLOCK_EDGE;
            let top = (index / VISUAL_BLOCK_COLUMNS) * BLOCK_EDGE;
            let sum = region_sum(pixels, left, left + BLOCK_EDGE, top, top + BLOCK_EDGE);
            *block = u8::try_from((sum + BLOCK_PIXELS / 2) / BLOCK_PIXELS).unwrap_or(u8::MAX);
        }
        Self {
            time,
            blocks,
            hash: difference_hash(pixels),
        }
    }

    /// Rebuilds a sample from recorded block means and hash (tests and
    /// recorded provider output); nothing here can be checked against pixels.
    #[must_use]
    pub const fn from_parts(
        time: MediaTime,
        blocks: [u8; VISUAL_BLOCKS],
        hash: VisualHash,
    ) -> Self {
        Self { time, blocks, hash }
    }

    /// Normalized source time of the decoded frame.
    #[must_use]
    pub const fn time(&self) -> MediaTime {
        self.time
    }

    /// The 16x9 block means, row by row.
    #[must_use]
    pub const fn blocks(&self) -> &[u8; VISUAL_BLOCKS] {
        &self.blocks
    }

    /// The difference hash.
    #[must_use]
    pub const fn hash(&self) -> VisualHash {
        self.hash
    }
}

fn region_sum(
    pixels: &[u8; VISUAL_FRAME_BYTES],
    left: usize,
    right: usize,
    top: usize,
    bottom: usize,
) -> u32 {
    let mut sum = 0_u32;
    for row in top..bottom {
        let start = row * VISUAL_FRAME_WIDTH;
        if let Some(values) = pixels.get(start + left..start + right) {
            sum += values.iter().map(|value| u32::from(*value)).sum::<u32>();
        }
    }
    sum
}

fn difference_hash(pixels: &[u8; VISUAL_FRAME_BYTES]) -> VisualHash {
    let mut bits = 0_u64;
    for row in 0..HASH_ROWS {
        let top = row * VISUAL_FRAME_HEIGHT / HASH_ROWS;
        let bottom = (row + 1) * VISUAL_FRAME_HEIGHT / HASH_ROWS;
        let mut means = [0_u32; HASH_COLUMNS];
        for (column, mean) in means.iter_mut().enumerate() {
            let left = column * VISUAL_FRAME_WIDTH / HASH_COLUMNS;
            let right = (column + 1) * VISUAL_FRAME_WIDTH / HASH_COLUMNS;
            let area = u32::try_from((right - left) * (bottom - top)).unwrap_or(u32::MAX);
            *mean = region_sum(pixels, left, right, top, bottom) / area.max(1);
        }
        for pair in means.windows(2) {
            bits = (bits << 1) | u64::from(pair.first() > pair.get(1));
        }
    }
    VisualHash(bits)
}

/// How far two samples differ, block by block.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VisualDelta {
    changed_blocks: u8,
    max_block_delta: u8,
}

impl VisualDelta {
    /// Creates a recorded delta.
    ///
    /// # Errors
    ///
    /// Returns [`VisualIndexError::InvalidChange`] when more blocks changed
    /// than a sample has.
    pub fn new(changed_blocks: u8, max_block_delta: u8) -> Result<Self, VisualIndexError> {
        if usize::from(changed_blocks) > VISUAL_BLOCKS {
            return Err(VisualIndexError::InvalidChange);
        }
        Ok(Self {
            changed_blocks,
            max_block_delta,
        })
    }

    /// Blocks whose mean moved by at least the policy's block threshold (0..=144).
    #[must_use]
    pub const fn changed_blocks(self) -> u8 {
        self.changed_blocks
    }

    /// Largest change of any block mean (0..=255).
    #[must_use]
    pub const fn max_block_delta(self) -> u8 {
        self.max_block_delta
    }
}

/// The reviewed rule that decides whether two samples show different screens.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum VisualChangePolicy {
    /// A block counts as changed when its mean moves by at least 4 levels;
    /// two such blocks, or any single block moving by at least 6, is a change.
    ///
    /// Measured on the synthetic corpus (`FFmpeg` 9.0, 2026-09-26), unchanged
    /// consecutive samples differ by at most 1 level in any block, and the
    /// smallest real edit (F04's order 1001 becoming 1017) moves one block
    /// by 6-7. The first proposal (two blocks at 6, or one at 12) missed that
    /// edit, so the single-block threshold is 6 and the per-block count
    /// threshold 4: still four to six times the observed noise. Real screen
    /// recordings with heavier compression noise are unmeasured.
    R0,
}

impl VisualChangePolicy {
    const fn block_threshold(self) -> u8 {
        match self {
            Self::R0 => 4,
        }
    }

    const fn min_changed_blocks(self) -> u8 {
        match self {
            Self::R0 => 2,
        }
    }

    const fn single_block_threshold(self) -> u8 {
        match self {
            Self::R0 => 6,
        }
    }

    /// Compares two samples block by block.
    #[must_use]
    pub fn delta(self, before: &VisualSample, after: &VisualSample) -> VisualDelta {
        let mut changed = 0_u8;
        let mut largest = 0_u8;
        for (left, right) in before.blocks.iter().zip(after.blocks.iter()) {
            let difference = left.abs_diff(*right);
            largest = largest.max(difference);
            if difference >= self.block_threshold() {
                changed = changed.saturating_add(1);
            }
        }
        VisualDelta {
            changed_blocks: changed,
            max_block_delta: largest,
        }
    }

    /// Reports whether `delta` is a change of screen under this policy.
    #[must_use]
    pub const fn is_change(self, delta: VisualDelta) -> bool {
        delta.changed_blocks >= self.min_changed_blocks()
            || delta.max_block_delta >= self.single_block_threshold()
    }
}

/// Number of windows a source of `duration` is cut into.
#[must_use]
pub fn visual_window_count(duration: MediaTime) -> u32 {
    u32::try_from(duration.as_micros().div_ceil(VISUAL_WINDOW_MICROS)).unwrap_or(u32::MAX)
}

/// One fixed analysis window of a source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisualWindow {
    ordinal: u32,
    range: TimeRange,
}

impl VisualWindow {
    /// Returns window `ordinal` of a source of `duration`:
    /// `[60 * ordinal s, min(60 * (ordinal + 1) s, duration))`.
    ///
    /// # Errors
    ///
    /// Returns [`VisualIndexError::InvalidWindow`] when the window lies at or
    /// beyond the end of the source or the duration exceeds the index bound.
    pub fn new(ordinal: u32, duration: MediaTime) -> Result<Self, VisualIndexError> {
        if duration.as_micros() == 0 || duration.as_micros() > MAX_VISUAL_INDEX_DURATION_MICROS {
            return Err(VisualIndexError::InvalidDuration);
        }
        let start = u64::from(ordinal)
            .checked_mul(VISUAL_WINDOW_MICROS)
            .ok_or(VisualIndexError::InvalidWindow)?;
        let end = start
            .saturating_add(VISUAL_WINDOW_MICROS)
            .min(duration.as_micros());
        let range = TimeRange::new(MediaTime::from_micros(start), MediaTime::from_micros(end))
            .map_err(|_| VisualIndexError::InvalidWindow)?;
        Ok(Self { ordinal, range })
    }

    /// Zero-based position of the window on the source timeline.
    #[must_use]
    pub const fn ordinal(self) -> u32 {
        self.ordinal
    }

    /// The window's half-open source range.
    #[must_use]
    pub const fn range(self) -> TimeRange {
        self.range
    }

    /// Earliest time of the window's lead-in sample.
    #[must_use]
    pub const fn lead_in_start(self) -> MediaTime {
        MediaTime::from_micros(
            self.range
                .start()
                .as_micros()
                .saturating_sub(VISUAL_LEAD_IN_MICROS),
        )
    }

    /// The window's 10 s coverage cells, in order; the last may be shorter.
    #[must_use]
    pub fn cells(self) -> Vec<TimeRange> {
        let mut cells = Vec::new();
        let mut start = self.range.start().as_micros();
        let end = self.range.end().as_micros();
        while start < end {
            let cell_end = start.saturating_add(VISUAL_CELL_MICROS).min(end);
            if let Ok(cell) = TimeRange::new(
                MediaTime::from_micros(start),
                MediaTime::from_micros(cell_end),
            ) {
                cells.push(cell);
            }
            start = cell_end;
        }
        cells
    }
}

/// Why a candidate was proposed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CandidateReason {
    /// The first decoded frame of a window with no lead-in sample.
    FirstFrame,
    /// The screen differs from the state before it.
    VisualChange,
    /// The first of three or more consecutive changing samples.
    MotionStart,
    /// The first still sample after motion.
    SettledAfterMotion,
    /// No change fell in this 10 s cell (or the window continues the screen
    /// its lead-in sample showed), so its first sample represents it.
    PeriodicCoverage,
}

impl CandidateReason {
    /// Every reason, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::FirstFrame,
        Self::VisualChange,
        Self::MotionStart,
        Self::SettledAfterMotion,
        Self::PeriodicCoverage,
    ];

    /// Stable identifier used in records and contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::FirstFrame => "first_frame",
            Self::VisualChange => "visual_change",
            Self::MotionStart => "motion_start",
            Self::SettledAfterMotion => "settled_after_motion",
            Self::PeriodicCoverage => "periodic_coverage",
        }
    }

    const fn has_change(self) -> bool {
        matches!(
            self,
            Self::VisualChange | Self::MotionStart | Self::SettledAfterMotion
        )
    }
}

/// What the samples showed about how long a candidate's screen held.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CandidateStability {
    /// The screen held for at least two consecutive samples.
    Settled,
    /// The screen was seen in exactly one sample and then changed.
    Transient,
    /// The screen was changing from sample to sample.
    InMotion,
    /// Seen only in the window's last sample; whether it held is unknown.
    OpenAtWindowEnd,
}

impl CandidateStability {
    /// Every stability, in declaration order.
    pub const ALL: [Self; 4] = [
        Self::Settled,
        Self::Transient,
        Self::InMotion,
        Self::OpenAtWindowEnd,
    ];

    /// Stable identifier used in records and contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Settled => "settled",
            Self::Transient => "transient",
            Self::InMotion => "in_motion",
            Self::OpenAtWindowEnd => "open_at_window_end",
        }
    }
}

/// The observed change that opened a candidate: it happened after
/// `previous_sample` and at or before the candidate's representative time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandidateChange {
    previous_sample: MediaTime,
    delta: VisualDelta,
}

impl CandidateChange {
    /// Creates a recorded change.
    #[must_use]
    pub const fn new(previous_sample: MediaTime, delta: VisualDelta) -> Self {
        Self {
            previous_sample,
            delta,
        }
    }

    /// Time of the sample before the change.
    #[must_use]
    pub const fn previous_sample(self) -> MediaTime {
        self.previous_sample
    }

    /// Size of the change.
    #[must_use]
    pub const fn delta(self) -> VisualDelta {
        self.delta
    }
}

/// A candidate before its identity is derived.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandidateDraft {
    /// Why it was proposed.
    pub reason: CandidateReason,
    /// Time of the decoded frame that represents it.
    pub representative: MediaTime,
    /// From the representative to the next candidate or the window end.
    pub span: TimeRange,
    /// Samples inside the span.
    pub sample_count: u16,
    /// How long the screen held.
    pub stability: CandidateStability,
    /// The change that opened it; absent for first frames and coverage.
    pub change: Option<CandidateChange>,
    /// Difference hash of the representative frame.
    pub hash: VisualHash,
}

/// An identified candidate of a committed index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualCandidate {
    id: VisualCandidateId,
    draft: CandidateDraft,
}

impl VisualCandidate {
    /// Attaches a derived identity to a draft.
    #[must_use]
    pub const fn new(id: VisualCandidateId, draft: CandidateDraft) -> Self {
        Self { id, draft }
    }

    /// Stable identity.
    #[must_use]
    pub const fn id(&self) -> &VisualCandidateId {
        &self.id
    }

    /// Why it was proposed.
    #[must_use]
    pub const fn reason(&self) -> CandidateReason {
        self.draft.reason
    }

    /// Time of the representative decoded frame.
    #[must_use]
    pub const fn representative(&self) -> MediaTime {
        self.draft.representative
    }

    /// Time span the candidate stands for.
    #[must_use]
    pub const fn span(&self) -> TimeRange {
        self.draft.span
    }

    /// Samples inside the span.
    #[must_use]
    pub const fn sample_count(&self) -> u16 {
        self.draft.sample_count
    }

    /// How long the screen held.
    #[must_use]
    pub const fn stability(&self) -> CandidateStability {
        self.draft.stability
    }

    /// The change that opened it, if any.
    #[must_use]
    pub const fn change(&self) -> Option<CandidateChange> {
        self.draft.change
    }

    /// Difference hash of the representative frame.
    #[must_use]
    pub const fn hash(&self) -> VisualHash {
        self.draft.hash
    }

    /// The draft this candidate was built from.
    #[must_use]
    pub const fn draft(&self) -> &CandidateDraft {
        &self.draft
    }
}

/// Why a window's samples were rejected before analysis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualAnalysisError {
    /// More samples than one window's decode may return.
    TooManySamples,
    /// A sample lies before the lead-in start or at or after the window end.
    SampleOutsideWindow,
    /// Sample times are not strictly increasing.
    UnorderedSamples,
}

impl fmt::Display for VisualAnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManySamples => "window decode returned too many samples",
            Self::SampleOutsideWindow => "a sample lies outside its window",
            Self::UnorderedSamples => "sample times are not strictly increasing",
        })
    }
}

impl Error for VisualAnalysisError {}

/// The candidates one window's samples produced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowAnalysis {
    /// The analysed window.
    pub window: VisualWindow,
    /// Samples inside the window (the lead-in is not counted).
    pub sample_count: u16,
    /// Candidates in time order.
    pub candidates: Vec<CandidateDraft>,
    /// Change candidates dropped by the per-window budget.
    pub dropped_candidates: u16,
    /// Coverage cells in which no frame was decoded.
    pub frameless_cells: Vec<TimeRange>,
}

struct State {
    start: usize,
    reason: CandidateReason,
    change: Option<CandidateChange>,
    motion: bool,
}

/// Analyses one window's decoded samples under `policy`.
///
/// `samples` is everything the window's decode returned, in time order: at
/// most one lead-in sample before the window (the last one before the window
/// start is used if a provider returns more) and the window's own samples.
///
/// # Errors
///
/// Returns [`VisualAnalysisError`] when the samples are unordered, outside
/// the window or too many.
pub fn analyse_window(
    window: VisualWindow,
    samples: &[VisualSample],
    policy: VisualChangePolicy,
) -> Result<WindowAnalysis, VisualAnalysisError> {
    validate_samples(window, samples)?;
    let split = samples.partition_point(|sample| sample.time < window.range.start());
    let lead = split.checked_sub(1).and_then(|index| samples.get(index));
    let body = samples.get(split..).unwrap_or_default();
    let cells = window.cells();
    let sample_count =
        u16::try_from(body.len()).map_err(|_| VisualAnalysisError::TooManySamples)?;
    if body.is_empty() {
        return Ok(WindowAnalysis {
            window,
            sample_count,
            candidates: Vec::new(),
            dropped_candidates: 0,
            frameless_cells: cells,
        });
    }
    let states = find_states(lead, body, policy);
    let first_in_cell: Vec<Option<usize>> = cells
        .iter()
        .map(|cell| body.iter().position(|sample| contains(*cell, sample.time)))
        .collect();
    let (kept, dropped) = apply_budget(&states, body, &cells, &first_in_cell);
    let candidates = build_candidates(&states, &kept, body, window, &cells, &first_in_cell);
    let frameless_cells = cells
        .iter()
        .zip(&first_in_cell)
        .filter(|(_, first)| first.is_none())
        .map(|(cell, _)| *cell)
        .collect();
    Ok(WindowAnalysis {
        window,
        sample_count,
        candidates,
        dropped_candidates: u16::try_from(dropped).unwrap_or(u16::MAX),
        frameless_cells,
    })
}

fn validate_samples(
    window: VisualWindow,
    samples: &[VisualSample],
) -> Result<(), VisualAnalysisError> {
    if samples.len() > MAX_WINDOW_SAMPLES {
        return Err(VisualAnalysisError::TooManySamples);
    }
    let lead_start = window.lead_in_start();
    let mut previous: Option<MediaTime> = None;
    for sample in samples {
        if sample.time < lead_start || sample.time >= window.range.end() {
            return Err(VisualAnalysisError::SampleOutsideWindow);
        }
        if previous.is_some_and(|time| time >= sample.time) {
            return Err(VisualAnalysisError::UnorderedSamples);
        }
        previous = Some(sample.time);
    }
    Ok(())
}

const fn contains(range: TimeRange, time: MediaTime) -> bool {
    range.start().as_micros() <= time.as_micros() && time.as_micros() < range.end().as_micros()
}

/// Splits the window's samples into screen states and marks motion.
fn find_states(
    lead: Option<&VisualSample>,
    body: &[VisualSample],
    policy: VisualChangePolicy,
) -> Vec<State> {
    let mut boundaries: Vec<Option<(CandidateReason, Option<CandidateChange>)>> =
        Vec::with_capacity(body.len());
    let mut moving = Vec::with_capacity(body.len());
    let mut anchor: Option<&VisualSample> = None;
    for (index, sample) in body.iter().enumerate() {
        let previous = if index == 0 {
            lead
        } else {
            body.get(index - 1)
        };
        let previous_delta = previous.map(|before| (before, policy.delta(before, sample)));
        let is_moving = previous_delta.is_some_and(|(_, delta)| policy.is_change(delta));
        moving.push(is_moving);
        let boundary = match (index, previous_delta, anchor) {
            (0, None, _) => Some((CandidateReason::FirstFrame, None)),
            (0, Some((before, delta)), _) => Some(if policy.is_change(delta) {
                (
                    CandidateReason::VisualChange,
                    Some(CandidateChange::new(before.time, delta)),
                )
            } else {
                (CandidateReason::PeriodicCoverage, None)
            }),
            (_, Some((before, delta)), Some(anchor_sample)) => {
                let anchor_delta = policy.delta(anchor_sample, sample);
                if policy.is_change(delta) {
                    Some((
                        CandidateReason::VisualChange,
                        Some(CandidateChange::new(before.time, delta)),
                    ))
                } else if policy.is_change(anchor_delta) {
                    Some((
                        CandidateReason::VisualChange,
                        Some(CandidateChange::new(before.time, anchor_delta)),
                    ))
                } else {
                    None
                }
            }
            _ => None,
        };
        if boundary.is_some() {
            anchor = Some(sample);
        }
        boundaries.push(boundary);
    }
    let mut motion_starts = Vec::new();
    let mut run_start: Option<usize> = None;
    for index in 0..=body.len() {
        let is_moving = moving.get(index).copied().unwrap_or(false);
        match (is_moving, run_start) {
            (true, None) => run_start = Some(index),
            (false, Some(start)) => {
                let end = index - 1;
                if end - start + 1 >= 3 {
                    motion_starts.push(start);
                    let settles = end + 1 < body.len();
                    let last_motion = if settles { end } else { end + 1 };
                    for boundary in boundaries.iter_mut().take(last_motion).skip(start + 1) {
                        *boundary = None;
                    }
                    if let Some(Some(first)) = boundaries.get_mut(start) {
                        first.0 = CandidateReason::MotionStart;
                    }
                    if settles && let Some(Some(settled)) = boundaries.get_mut(end) {
                        settled.0 = CandidateReason::SettledAfterMotion;
                    }
                }
                run_start = None;
            }
            _ => {}
        }
    }
    boundaries
        .into_iter()
        .enumerate()
        .filter_map(|(start, boundary)| {
            boundary.map(|(reason, change)| State {
                start,
                reason,
                change,
                motion: motion_starts.contains(&start),
            })
        })
        .collect()
}

fn stability_of(states: &[State], position: usize, body_len: usize) -> CandidateStability {
    let Some(state) = states.get(position) else {
        return CandidateStability::Settled;
    };
    if state.motion {
        return CandidateStability::InMotion;
    }
    let next = states.get(position + 1).map_or(body_len, |next| next.start);
    match (next - state.start, position + 1 == states.len()) {
        (1, true) => CandidateStability::OpenAtWindowEnd,
        (1, false) => CandidateStability::Transient,
        _ => CandidateStability::Settled,
    }
}

/// Cells holding a decoded frame but none of the kept candidates.
fn uncovered_cells(
    kept: &[usize],
    states: &[State],
    body: &[VisualSample],
    cells: &[TimeRange],
    first_in_cell: &[Option<usize>],
) -> usize {
    cells
        .iter()
        .zip(first_in_cell)
        .filter(|(cell, first)| {
            first.is_some()
                && !kept.iter().any(|position| {
                    states
                        .get(*position)
                        .and_then(|state| body.get(state.start))
                        .is_some_and(|sample| contains(**cell, sample.time))
                })
        })
        .count()
}

/// Chooses which states stay candidates within the per-window budget.
///
/// The first state always stays. Coverage candidates are added afterwards for
/// every cell the kept states leave uncovered, so a state is accepted only
/// while the kept states plus the coverage they still need fit the budget.
/// Adding a state never increases the coverage needed, so the greedy pass in
/// score order (ties to the earlier time) is deterministic and always leaves
/// room for coverage: a window has at most six cells.
fn apply_budget(
    states: &[State],
    body: &[VisualSample],
    cells: &[TimeRange],
    first_in_cell: &[Option<usize>],
) -> (Vec<usize>, usize) {
    let all: Vec<usize> = (0..states.len()).collect();
    if all.len() + uncovered_cells(&all, states, body, cells, first_in_cell)
        <= MAX_WINDOW_CANDIDATES
    {
        return (all, 0);
    }
    let mut optional: Vec<usize> = (1..states.len()).collect();
    optional.sort_by(|left, right| {
        let score = |position: &usize| {
            states
                .get(*position)
                .and_then(|state| state.change)
                .map(|change| change.delta)
        };
        score(right).cmp(&score(left)).then(left.cmp(right))
    });
    let mut kept = vec![0];
    for position in optional {
        kept.push(position);
        if kept.len() + uncovered_cells(&kept, states, body, cells, first_in_cell)
            > MAX_WINDOW_CANDIDATES
        {
            kept.pop();
        }
    }
    kept.sort_unstable();
    let dropped = states.len() - kept.len();
    (kept, dropped)
}

fn build_candidates(
    states: &[State],
    kept: &[usize],
    body: &[VisualSample],
    window: VisualWindow,
    cells: &[TimeRange],
    first_in_cell: &[Option<usize>],
) -> Vec<CandidateDraft> {
    // (sample index, reason, change, stability)
    let mut starts: Vec<(
        usize,
        CandidateReason,
        Option<CandidateChange>,
        CandidateStability,
    )> = kept
        .iter()
        .filter_map(|position| {
            states.get(*position).map(|state| {
                (
                    state.start,
                    state.reason,
                    state.change,
                    stability_of(states, *position, body.len()),
                )
            })
        })
        .collect();
    for (cell, first) in cells.iter().zip(first_in_cell) {
        let Some(first) = *first else {
            continue;
        };
        let covered = starts.iter().any(|(start, ..)| {
            body.get(*start)
                .is_some_and(|sample| contains(*cell, sample.time))
        });
        if !covered {
            let containing = states.iter().rposition(|state| state.start <= first);
            let stability = containing.map_or(CandidateStability::Settled, |position| {
                stability_of(states, position, body.len())
            });
            starts.push((first, CandidateReason::PeriodicCoverage, None, stability));
        }
    }
    starts.sort_by_key(|(start, ..)| *start);
    let mut drafts = Vec::with_capacity(starts.len());
    for (position, (start, reason, change, stability)) in starts.iter().enumerate() {
        let Some(sample) = body.get(*start) else {
            continue;
        };
        let (end, next_start) =
            starts
                .get(position + 1)
                .map_or((window.range.end(), body.len()), |(next, ..)| {
                    (
                        body.get(*next).map_or(window.range.end(), |next| next.time),
                        *next,
                    )
                });
        let Ok(span) = TimeRange::new(sample.time, end) else {
            continue;
        };
        drafts.push(CandidateDraft {
            reason: *reason,
            representative: sample.time,
            span,
            sample_count: u16::try_from(next_start - start).unwrap_or(u16::MAX),
            stability: *stability,
            change: *change,
            hash: sample.hash,
        });
    }
    drafts
}

/// What a window of a committed index holds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VisualWindowOutcome {
    /// The window was decoded and analysed.
    Analysed {
        /// Samples inside the window.
        sample_count: u16,
        /// Identified candidates in time order.
        candidates: Vec<VisualCandidate>,
        /// Change candidates the budget dropped.
        dropped_candidates: u16,
        /// Cells in which no frame was decoded.
        frameless_cells: Vec<TimeRange>,
    },
    /// The provider rejected the window; recorded so it is not retried.
    Undecodable,
}

/// One window of a committed index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualIndexWindow {
    window: VisualWindow,
    outcome: VisualWindowOutcome,
}

impl VisualIndexWindow {
    /// Validates a window's outcome against the analysis rules.
    ///
    /// Every rule the analysis guarantees is checked again, so a stored
    /// window cannot claim coverage or changes the rules would not produce:
    /// candidates lie in the window in time order and tile it from the first
    /// one to the window end; sample counts add up; each change satisfies the
    /// policy; every 10 s cell holds a candidate or is recorded as frameless;
    /// the budget holds.
    ///
    /// # Errors
    ///
    /// Returns [`VisualIndexError`] naming the first violated rule.
    pub fn new(
        window: VisualWindow,
        outcome: VisualWindowOutcome,
        policy: VisualChangePolicy,
    ) -> Result<Self, VisualIndexError> {
        if let VisualWindowOutcome::Analysed {
            sample_count,
            candidates,
            dropped_candidates,
            frameless_cells,
        } = &outcome
        {
            validate_analysed(
                window,
                *sample_count,
                candidates,
                *dropped_candidates,
                frameless_cells,
                policy,
            )?;
        }
        Ok(Self { window, outcome })
    }

    /// The window.
    #[must_use]
    pub const fn window(&self) -> VisualWindow {
        self.window
    }

    /// What the window holds.
    #[must_use]
    pub const fn outcome(&self) -> &VisualWindowOutcome {
        &self.outcome
    }

    /// Candidates of an analysed window; none for an undecodable one.
    #[must_use]
    pub fn candidates(&self) -> &[VisualCandidate] {
        match &self.outcome {
            VisualWindowOutcome::Analysed { candidates, .. } => candidates,
            VisualWindowOutcome::Undecodable => &[],
        }
    }
}

#[allow(clippy::too_many_lines)] // One auditable list of the analysis guarantees.
fn validate_analysed(
    window: VisualWindow,
    sample_count: u16,
    candidates: &[VisualCandidate],
    dropped: u16,
    frameless: &[TimeRange],
    policy: VisualChangePolicy,
) -> Result<(), VisualIndexError> {
    let cells = window.cells();
    if usize::from(sample_count) > MAX_WINDOW_SAMPLES
        || candidates.len() > MAX_WINDOW_CANDIDATES
        || usize::from(dropped) + candidates.len() > usize::from(sample_count)
    {
        return Err(VisualIndexError::TooManyCandidates);
    }
    if sample_count == 0 {
        if !candidates.is_empty() || dropped != 0 || frameless != cells.as_slice() {
            return Err(VisualIndexError::InvalidCoverage);
        }
        return Ok(());
    }
    let first = candidates
        .first()
        .ok_or(VisualIndexError::InvalidCandidate)?;
    if first.draft.representative != first.draft.span.start() {
        return Err(VisualIndexError::InvalidCandidate);
    }
    let mut total_samples = 0_usize;
    for (position, candidate) in candidates.iter().enumerate() {
        let draft = candidate.draft;
        let expected_end = candidates
            .get(position + 1)
            .map_or(window.range.end(), |next| next.draft.representative);
        if !contains(window.range, draft.representative)
            || draft.span.start() != draft.representative
            || draft.span.end() != expected_end
            || draft.sample_count == 0
        {
            return Err(VisualIndexError::InvalidCandidate);
        }
        total_samples += usize::from(draft.sample_count);
        if (draft.reason == CandidateReason::FirstFrame && position != 0)
            || (draft.stability == CandidateStability::OpenAtWindowEnd
                && position + 1 != candidates.len())
        {
            return Err(VisualIndexError::InvalidCandidate);
        }
        match (draft.reason.has_change(), draft.change) {
            (false, None) => {}
            (true, Some(change)) => {
                let earliest = if position == 0 {
                    window.lead_in_start()
                } else {
                    candidates
                        .get(position - 1)
                        .map_or(window.range.start(), |previous| {
                            previous.draft.representative
                        })
                };
                if change.previous_sample < earliest
                    || change.previous_sample >= draft.representative
                    || !policy.is_change(change.delta)
                {
                    return Err(VisualIndexError::InvalidChange);
                }
            }
            _ => return Err(VisualIndexError::InvalidChange),
        }
    }
    if total_samples != usize::from(sample_count) {
        return Err(VisualIndexError::InvalidCandidate);
    }
    let mut expected_frameless = Vec::new();
    for cell in &cells {
        if !candidates
            .iter()
            .any(|candidate| contains(*cell, candidate.draft.representative))
        {
            expected_frameless.push(*cell);
        }
    }
    if frameless != expected_frameless.as_slice() {
        return Err(VisualIndexError::InvalidCoverage);
    }
    Ok(())
}

/// Why a gap in visual coverage exists.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CoverageGapReason {
    /// The window has not been analysed yet; a later call can analyse it.
    NotAnalyzed,
    /// The window's decode exceeded its deadline in this call; retryable.
    DeadlineExceeded,
    /// The provider could not decode the window; not retried.
    Undecodable,
    /// No frame was decoded in this 10 s cell.
    NoDecodedFrame,
    /// The window had more changes than its candidate budget.
    CandidateBudgetExhausted,
}

impl CoverageGapReason {
    /// Every reason, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::NotAnalyzed,
        Self::DeadlineExceeded,
        Self::Undecodable,
        Self::NoDecodedFrame,
        Self::CandidateBudgetExhausted,
    ];

    /// Stable identifier used in contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::NotAnalyzed => "not_analyzed",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Undecodable => "undecodable",
            Self::NoDecodedFrame => "no_decoded_frame",
            Self::CandidateBudgetExhausted => "candidate_budget_exhausted",
        }
    }
}

/// One honest gap in the visual coverage of a range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisualCoverageGap {
    /// The affected part of the requested range.
    pub range: TimeRange,
    /// Why it is a gap.
    pub reason: CoverageGapReason,
    /// For [`CoverageGapReason::CandidateBudgetExhausted`], how many change
    /// candidates were dropped; zero otherwise.
    pub dropped_candidates: u16,
}

/// Why a visual index or one of its parts is invalid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualIndexError {
    /// The source duration is zero or beyond the four-hour bound.
    InvalidDuration,
    /// A window ordinal or range does not match the fixed grid.
    InvalidWindow,
    /// Windows are not in strictly ascending ordinal order.
    UnorderedWindows,
    /// A candidate's time, span, count, reason or stability breaks the rules.
    InvalidCandidate,
    /// A recorded change does not satisfy the policy or lies outside its window.
    InvalidChange,
    /// A cell is neither covered by a candidate nor recorded as frameless.
    InvalidCoverage,
    /// A window exceeds its sample or candidate budget.
    TooManyCandidates,
    /// Two candidates share an identity.
    DuplicateCandidate,
    /// A visual hash is not 16 lowercase hexadecimal digits.
    InvalidHash,
}

impl fmt::Display for VisualIndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDuration => "visual index source duration is invalid",
            Self::InvalidWindow => "visual index window does not match the grid",
            Self::UnorderedWindows => "visual index windows are not in ascending order",
            Self::InvalidCandidate => "visual candidate breaks the analysis rules",
            Self::InvalidChange => "visual candidate change breaks the policy",
            Self::InvalidCoverage => "visual index cell coverage is inconsistent",
            Self::TooManyCandidates => "visual index window exceeds its budget",
            Self::DuplicateCandidate => "visual candidate identity is repeated",
            Self::InvalidHash => "visual hash is not canonical",
        })
    }
}

impl Error for VisualIndexError {}

/// Everything needed to assemble a committed index revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualIndexParts {
    /// Revision identity.
    pub id: VisualIndexId,
    /// Revision number within the session, starting at 1.
    pub number: NonZeroU32,
    /// Source the index describes.
    pub source_id: SourceId,
    /// Selected video stream index.
    pub stream_index: u32,
    /// The stream's orientation-correct displayed dimensions.
    ///
    /// Kept with the index so a warm read can state what a candidate's
    /// representative frame shows (and P09 can crop it) without probing the
    /// source again.
    pub displayed_dimensions: FrameDimensions,
    /// Normalized source duration the window grid is cut from.
    pub duration: MediaTime,
    /// Analysis profile.
    pub profile: VisualIndexProfile,
    /// Analysed or undecodable windows in ascending ordinal order.
    pub windows: Vec<VisualIndexWindow>,
}

/// One immutable revision of a session's visual-candidate index.
///
/// Windows that are absent have not been analysed. A later revision holds
/// every window of the one before it unchanged plus newly analysed ones, so
/// candidate identities and pages over already analysed time never change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualIndex {
    parts: VisualIndexParts,
}

impl VisualIndex {
    /// Validates and assembles an index revision.
    ///
    /// # Errors
    ///
    /// Returns [`VisualIndexError`] when the duration, window grid or order,
    /// or candidate identities are invalid.
    pub fn new(parts: VisualIndexParts) -> Result<Self, VisualIndexError> {
        let duration = parts.duration;
        if duration.as_micros() == 0 || duration.as_micros() > MAX_VISUAL_INDEX_DURATION_MICROS {
            return Err(VisualIndexError::InvalidDuration);
        }
        let mut previous: Option<u32> = None;
        let mut identities = HashSet::new();
        for window in &parts.windows {
            let ordinal = window.window.ordinal();
            if VisualWindow::new(ordinal, duration)? != window.window {
                return Err(VisualIndexError::InvalidWindow);
            }
            if previous.is_some_and(|before| before >= ordinal) {
                return Err(VisualIndexError::UnorderedWindows);
            }
            previous = Some(ordinal);
            for candidate in window.candidates() {
                if !identities.insert(candidate.id.as_str()) {
                    return Err(VisualIndexError::DuplicateCandidate);
                }
            }
        }
        Ok(Self { parts })
    }

    /// Revision identity.
    #[must_use]
    pub const fn id(&self) -> &VisualIndexId {
        &self.parts.id
    }

    /// Revision number.
    #[must_use]
    pub const fn number(&self) -> NonZeroU32 {
        self.parts.number
    }

    /// Source the index describes.
    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.parts.source_id
    }

    /// Selected video stream index.
    #[must_use]
    pub const fn stream_index(&self) -> u32 {
        self.parts.stream_index
    }

    /// Displayed dimensions of the selected video stream.
    #[must_use]
    pub const fn displayed_dimensions(&self) -> FrameDimensions {
        self.parts.displayed_dimensions
    }

    /// Normalized source duration.
    #[must_use]
    pub const fn duration(&self) -> MediaTime {
        self.parts.duration
    }

    /// Analysis profile.
    #[must_use]
    pub const fn profile(&self) -> VisualIndexProfile {
        self.parts.profile
    }

    /// Recorded windows in ascending ordinal order.
    #[must_use]
    pub fn windows(&self) -> &[VisualIndexWindow] {
        &self.parts.windows
    }

    /// The recorded window with `ordinal`, if analysed or undecodable.
    #[must_use]
    pub fn window(&self, ordinal: u32) -> Option<&VisualIndexWindow> {
        self.parts
            .windows
            .binary_search_by_key(&ordinal, |window| window.window.ordinal())
            .ok()
            .and_then(|position| self.parts.windows.get(position))
    }

    /// Ordinals of the windows that intersect `range`, clipped to the source.
    #[must_use]
    pub fn window_ordinals(&self, range: TimeRange) -> std::ops::Range<u32> {
        let count = visual_window_count(self.parts.duration);
        let first = range.start().as_micros() / VISUAL_WINDOW_MICROS;
        let last = range.end().as_micros().div_ceil(VISUAL_WINDOW_MICROS);
        let first = u32::try_from(first).unwrap_or(u32::MAX).min(count);
        let last = u32::try_from(last).unwrap_or(u32::MAX).min(count);
        first..last
    }

    /// Candidates whose representative time lies in `range`, in time order.
    pub fn candidates_in(&self, range: TimeRange) -> impl Iterator<Item = &VisualCandidate> {
        self.parts
            .windows
            .iter()
            .flat_map(VisualIndexWindow::candidates)
            .filter(move |candidate| contains(range, candidate.draft.representative))
    }

    /// The gaps in visual coverage of `range`, in time order.
    ///
    /// Unanalysed windows are `not_analyzed`, undecodable ones `undecodable`,
    /// frameless cells `no_decoded_frame`, and a window whose budget dropped
    /// candidates is `candidate_budget_exhausted` over its whole range, each
    /// clipped to `range`.
    #[must_use]
    pub fn coverage_gaps(&self, range: TimeRange) -> Vec<VisualCoverageGap> {
        let mut gaps = Vec::new();
        for ordinal in self.window_ordinals(range) {
            let Ok(grid) = VisualWindow::new(ordinal, self.parts.duration) else {
                continue;
            };
            let mut push = |part: TimeRange, reason, dropped_candidates| {
                if let Some(clipped) = intersect(part, range) {
                    gaps.push(VisualCoverageGap {
                        range: clipped,
                        reason,
                        dropped_candidates,
                    });
                }
            };
            match self.window(ordinal).map(VisualIndexWindow::outcome) {
                None => push(grid.range(), CoverageGapReason::NotAnalyzed, 0),
                Some(VisualWindowOutcome::Undecodable) => {
                    push(grid.range(), CoverageGapReason::Undecodable, 0);
                }
                Some(VisualWindowOutcome::Analysed {
                    dropped_candidates,
                    frameless_cells,
                    ..
                }) => {
                    if *dropped_candidates > 0 {
                        push(
                            grid.range(),
                            CoverageGapReason::CandidateBudgetExhausted,
                            *dropped_candidates,
                        );
                    }
                    for cell in frameless_cells {
                        push(*cell, CoverageGapReason::NoDecodedFrame, 0);
                    }
                }
            }
        }
        gaps
    }

    /// Consumes the index, returning its parts.
    #[must_use]
    pub fn into_parts(self) -> VisualIndexParts {
        self.parts
    }
}

fn intersect(left: TimeRange, right: TimeRange) -> Option<TimeRange> {
    TimeRange::new(left.start().max(right.start()), left.end().min(right.end())).ok()
}

#[cfg(test)]
mod tests;
