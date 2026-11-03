//! What changed on screen between two exact frames (ADR 0034).
//!
//! Two concerns live here, both pure and deterministic:
//!
//! - **Which changes a range holds.** The visual index (ADR 0018) already
//!   says when the screen changed; [`plan_screen_changes`] reads its
//!   candidates into changes, each with the exact frame before and after. A
//!   motion run (a `motion_start` and the `settled_after_motion` that ends
//!   it) is one change, so a scroll is compared from before it moved to where
//!   it settled, not step by step.
//! - **Where it changed.** [`compare_frames`] compares the pixels of the two
//!   frames themselves, the very images a caller cites, and reports the
//!   rectangles that differ. It never says what a change means.

use std::error::Error;
use std::fmt;

use crate::{
    CandidateReason, CropRect, FrameDimensions, MediaTime, TimeRange, VisualCandidate,
    VisualCandidateId, VisualIndex, VisualWindowOutcome,
};

/// Edge of a square comparison tile, in displayed pixels.
pub const SCREEN_CHANGE_TILE: u32 = 16;
/// Smallest difference of one colour channel, in levels, that changes a pixel.
pub const SCREEN_CHANGE_PIXEL_LEVEL: u8 = 32;
/// Fewest changed pixels that change a tile, so isolated compression noise
/// is not a change.
pub const SCREEN_CHANGE_TILE_PIXELS: u16 = 4;
/// Changed tiles at most this many tiles apart (in either direction) join one
/// region: 2 joins touching tiles and tiles one unchanged tile apart, so a
/// line of words is one region.
pub const SCREEN_CHANGE_JOIN_DISTANCE: u32 = 2;
/// Most regions reported for one change; the rest are counted.
pub const MAX_CHANGE_REGIONS: usize = 8;

/// The versioned rule that finds regions. Changing a threshold is a new
/// profile, never a silent change of this one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenChangeProfile {
    /// `screen-change-regions-v1` (ADR 0034 section 3).
    RegionsV1,
}

impl ScreenChangeProfile {
    /// Published identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::RegionsV1 => "screen-change-regions-v1",
        }
    }
}

/// Which part of a motion run a change stands for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MotionPart {
    /// The whole run: before it moved, after it settled.
    Run,
    /// A `motion_start` without the candidate that ended its run (still
    /// moving at its window's end, or not paired in a window whose budget
    /// dropped candidates): after is its first moving frame.
    Start,
    /// A `settled_after_motion` without the start of its run, in a window
    /// whose budget dropped candidates: before is the last moving frame.
    End,
}

impl MotionPart {
    /// Published identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Start => "start",
            Self::End => "end",
        }
    }

    /// Whether the after frame is a settled screen.
    #[must_use]
    pub const fn settled(self) -> bool {
        matches!(self, Self::Run | Self::End)
    }
}

/// What kind of change the index recorded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenChangeKind {
    /// One sampled change of screen.
    Change,
    /// A run of three or more changing samples, or part of one.
    Motion(MotionPart),
}

impl ScreenChangeKind {
    /// Published identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Change => "change",
            Self::Motion(_) => "motion",
        }
    }
}

/// One change of a range, before its frames are taken.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedScreenChange {
    /// What kind of change it is.
    pub kind: ScreenChangeKind,
    /// The candidates it stands for, in time order (one, or two for a run).
    pub candidates: Vec<VisualCandidateId>,
    /// Ordinal of the window the candidates lie in.
    pub window_ordinal: u32,
    /// Time of the sampled frame shown before the change.
    pub before: MediaTime,
    /// Time of the sampled frame after it.
    pub after: MediaTime,
}

/// The changes whose after frame lies in `range`, in time order.
///
/// A `visual_change` candidate is one change. A `motion_start` followed by
/// the `settled_after_motion` that ends its run is one change; periodic
/// coverage candidates between them (cells inside a long run) are skipped.
/// The pair is formed only in a window whose budget dropped nothing: once
/// the budget has dropped candidates, a kept `settled_after_motion` may end a
/// later run whose start was dropped, so no pairing could be told apart from
/// a wrong one, and each kept motion candidate is a change of its own.
/// `first_frame` and `periodic_coverage` candidates are not changes. A
/// change's before frame, and a run's start, may lie before `range`.
#[must_use]
pub fn plan_screen_changes(index: &VisualIndex, range: TimeRange) -> Vec<PlannedScreenChange> {
    let mut changes = Vec::new();
    for ordinal in index.window_ordinals(range) {
        let Some(window) = index.window(ordinal) else {
            continue;
        };
        let VisualWindowOutcome::Analysed {
            candidates,
            dropped_candidates,
            ..
        } = window.outcome()
        else {
            continue;
        };
        for change in window_changes(ordinal, candidates, *dropped_candidates == 0) {
            if contains(range, change.after) {
                changes.push(change);
            }
        }
    }
    changes
}

fn window_changes(
    ordinal: u32,
    candidates: &[VisualCandidate],
    may_pair: bool,
) -> Vec<PlannedScreenChange> {
    let mut changes = Vec::new();
    let mut position = 0;
    while let Some(candidate) = candidates.get(position) {
        position += 1;
        let Some(change) = candidate.change() else {
            continue;
        };
        let single = |kind| PlannedScreenChange {
            kind,
            candidates: vec![candidate.id().clone()],
            window_ordinal: ordinal,
            before: change.previous_sample(),
            after: candidate.representative(),
        };
        match candidate.reason() {
            CandidateReason::VisualChange => changes.push(single(ScreenChangeKind::Change)),
            CandidateReason::MotionStart => {
                let settled = may_pair
                    .then(|| next_change_candidate(candidates, position))
                    .flatten()
                    .filter(|(_, next)| next.reason() == CandidateReason::SettledAfterMotion);
                if let Some((next_position, end)) = settled {
                    position = next_position + 1;
                    changes.push(PlannedScreenChange {
                        kind: ScreenChangeKind::Motion(MotionPart::Run),
                        candidates: vec![candidate.id().clone(), end.id().clone()],
                        window_ordinal: ordinal,
                        before: change.previous_sample(),
                        after: end.representative(),
                    });
                } else {
                    changes.push(single(ScreenChangeKind::Motion(MotionPart::Start)));
                }
            }
            CandidateReason::SettledAfterMotion => {
                changes.push(single(ScreenChangeKind::Motion(MotionPart::End)));
            }
            CandidateReason::FirstFrame | CandidateReason::PeriodicCoverage => {}
        }
    }
    changes
}

/// The next candidate from `start` that records a change, with its position.
fn next_change_candidate(
    candidates: &[VisualCandidate],
    start: usize,
) -> Option<(usize, &VisualCandidate)> {
    candidates
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, candidate)| candidate.reason() != CandidateReason::PeriodicCoverage)
}

const fn contains(range: TimeRange, time: MediaTime) -> bool {
    range.start().as_micros() <= time.as_micros() && time.as_micros() < range.end().as_micros()
}

/// Why pixels could not be compared.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenChangeError {
    /// The buffer is not exactly three bytes per pixel of its dimensions.
    PixelBufferSize,
}

impl fmt::Display for ScreenChangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PixelBufferSize => "the pixel buffer does not match its dimensions",
        })
    }
}

impl Error for ScreenChangeError {}

/// The 8-bit RGB pixels of one displayed frame, row by row.
#[derive(Clone, Copy, Debug)]
pub struct RgbPixels<'a> {
    dimensions: FrameDimensions,
    bytes: &'a [u8],
}

impl<'a> RgbPixels<'a> {
    /// Wraps `bytes`, which must hold exactly `width * height * 3` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ScreenChangeError::PixelBufferSize`] for any other length.
    pub fn new(dimensions: FrameDimensions, bytes: &'a [u8]) -> Result<Self, ScreenChangeError> {
        let expected = u64::from(dimensions.width())
            .checked_mul(u64::from(dimensions.height()))
            .and_then(|pixels| pixels.checked_mul(3))
            .ok_or(ScreenChangeError::PixelBufferSize)?;
        if u64::try_from(bytes.len()).ok() != Some(expected) {
            return Err(ScreenChangeError::PixelBufferSize);
        }
        Ok(Self { dimensions, bytes })
    }

    /// The frame's displayed dimensions.
    #[must_use]
    pub const fn dimensions(&self) -> FrameDimensions {
        self.dimensions
    }
}

/// A rectangle of the frame that differs, in displayed pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChangeRegion {
    /// The rectangle, aligned to tiles and clipped to the frame: the space
    /// `crop --rect` takes.
    pub rect: CropRect,
    /// Changed pixels in the region's changed tiles.
    pub changed_pixels: u64,
}

/// Where two frames of the same size differ.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegionComparison {
    /// The largest regions by changed pixels, at most [`MAX_CHANGE_REGIONS`].
    pub regions: Vec<ChangeRegion>,
    /// Regions found beyond [`MAX_CHANGE_REGIONS`].
    pub regions_left_out: u32,
    /// The rectangle around every changed tile, with every changed pixel of
    /// them; `None` when the frames differ nowhere at the threshold.
    pub changed_area: Option<ChangeRegion>,
}

/// The outcome of comparing two frames.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrameComparison {
    /// The frames have the same size and were compared.
    Compared(RegionComparison),
    /// The frames have different displayed sizes; nothing was compared.
    DimensionsDiffer,
}

/// Compares two frames under [`ScreenChangeProfile::RegionsV1`].
///
/// A pixel changed when any of its channels differs by at least
/// [`SCREEN_CHANGE_PIXEL_LEVEL`]; a [`SCREEN_CHANGE_TILE`]-pixel tile changed
/// when it holds at least [`SCREEN_CHANGE_TILE_PIXELS`] changed pixels;
/// changed tiles at most [`SCREEN_CHANGE_JOIN_DISTANCE`] tiles apart join one
/// region, whose rectangle is its tiles' bounding box clipped to the frame.
#[must_use]
pub fn compare_frames(before: RgbPixels<'_>, after: RgbPixels<'_>) -> FrameComparison {
    if before.dimensions != after.dimensions {
        return FrameComparison::DimensionsDiffer;
    }
    let grid = TileGrid::count(before, after);
    FrameComparison::Compared(grid.regions(before.dimensions))
}

/// Changed-pixel counts per tile, row by row.
struct TileGrid {
    columns: u32,
    rows: u32,
    counts: Vec<u16>,
}

impl TileGrid {
    fn count(before: RgbPixels<'_>, after: RgbPixels<'_>) -> Self {
        let width = before.dimensions.width();
        let height = before.dimensions.height();
        let columns = width.div_ceil(SCREEN_CHANGE_TILE);
        let rows = height.div_ceil(SCREEN_CHANGE_TILE);
        let mut counts = vec![0_u16; tile_count(columns, rows)];
        let row_bytes = usize::try_from(width)
            .unwrap_or(usize::MAX)
            .saturating_mul(3);
        let lines = before
            .bytes
            .chunks_exact(row_bytes)
            .zip(after.bytes.chunks_exact(row_bytes));
        for (y, (line_before, line_after)) in (0_u32..).zip(lines) {
            let tile_row = y / SCREEN_CHANGE_TILE;
            let pixels = line_before.chunks_exact(3).zip(line_after.chunks_exact(3));
            for (x, (pixel_before, pixel_after)) in (0_u32..).zip(pixels) {
                if pixel_changed(pixel_before, pixel_after)
                    && let Some(count) =
                        counts.get_mut(tile_index(columns, x / SCREEN_CHANGE_TILE, tile_row))
                {
                    *count = count.saturating_add(1);
                }
            }
        }
        Self {
            columns,
            rows,
            counts,
        }
    }

    fn changed(&self, column: u32, row: u32) -> bool {
        self.counts
            .get(tile_index(self.columns, column, row))
            .is_some_and(|count| *count >= SCREEN_CHANGE_TILE_PIXELS)
    }

    fn changed_pixels(&self, column: u32, row: u32) -> u64 {
        self.counts
            .get(tile_index(self.columns, column, row))
            .map_or(0, |count| u64::from(*count))
    }

    /// Joins changed tiles into regions, in a deterministic row-major order.
    fn regions(&self, dimensions: FrameDimensions) -> RegionComparison {
        let mut seen = vec![false; self.counts.len()];
        let mut found: Vec<TileBox> = Vec::new();
        for row in 0..self.rows {
            for column in 0..self.columns {
                let index = tile_index(self.columns, column, row);
                if !self.changed(column, row) || seen.get(index).copied().unwrap_or(true) {
                    continue;
                }
                found.push(self.component(column, row, &mut seen));
            }
        }
        let changed_area = found
            .iter()
            .copied()
            .reduce(TileBox::union)
            .and_then(|area| area.region(dimensions));
        let mut regions: Vec<ChangeRegion> = found
            .iter()
            .filter_map(|tiles| tiles.region(dimensions))
            .collect();
        regions.sort_by(|left, right| {
            right
                .changed_pixels
                .cmp(&left.changed_pixels)
                .then(left.rect.y().cmp(&right.rect.y()))
                .then(left.rect.x().cmp(&right.rect.x()))
        });
        let left_out = regions.len().saturating_sub(MAX_CHANGE_REGIONS);
        regions.truncate(MAX_CHANGE_REGIONS);
        RegionComparison {
            regions,
            regions_left_out: u32::try_from(left_out).unwrap_or(u32::MAX),
            changed_area,
        }
    }

    /// The bounding box of the changed tiles joined to (`column`, `row`).
    fn component(&self, column: u32, row: u32, seen: &mut [bool]) -> TileBox {
        let mut tiles = TileBox::single(column, row, self.changed_pixels(column, row));
        let mut pending = vec![(column, row)];
        if let Some(mark) = seen.get_mut(tile_index(self.columns, column, row)) {
            *mark = true;
        }
        while let Some((x, y)) = pending.pop() {
            let reach = SCREEN_CHANGE_JOIN_DISTANCE;
            let rows =
                y.saturating_sub(reach)..=y.saturating_add(reach).min(self.rows.saturating_sub(1));
            for near_row in rows {
                let columns = x.saturating_sub(reach)
                    ..=x.saturating_add(reach).min(self.columns.saturating_sub(1));
                for near_column in columns {
                    let index = tile_index(self.columns, near_column, near_row);
                    if !self.changed(near_column, near_row)
                        || seen.get(index).copied().unwrap_or(true)
                    {
                        continue;
                    }
                    if let Some(mark) = seen.get_mut(index) {
                        *mark = true;
                    }
                    tiles = tiles.union(TileBox::single(
                        near_column,
                        near_row,
                        self.changed_pixels(near_column, near_row),
                    ));
                    pending.push((near_column, near_row));
                }
            }
        }
        tiles
    }
}

/// Inclusive tile bounds of a set of changed tiles, with their changed pixels.
#[derive(Clone, Copy, Debug)]
struct TileBox {
    first_column: u32,
    first_row: u32,
    last_column: u32,
    last_row: u32,
    changed_pixels: u64,
}

impl TileBox {
    const fn single(column: u32, row: u32, changed_pixels: u64) -> Self {
        Self {
            first_column: column,
            first_row: row,
            last_column: column,
            last_row: row,
            changed_pixels,
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            first_column: self.first_column.min(other.first_column),
            first_row: self.first_row.min(other.first_row),
            last_column: self.last_column.max(other.last_column),
            last_row: self.last_row.max(other.last_row),
            changed_pixels: self.changed_pixels.saturating_add(other.changed_pixels),
        }
    }

    /// The box in displayed pixels, clipped to the frame.
    fn region(self, dimensions: FrameDimensions) -> Option<ChangeRegion> {
        let x = self.first_column.checked_mul(SCREEN_CHANGE_TILE)?;
        let y = self.first_row.checked_mul(SCREEN_CHANGE_TILE)?;
        let right = self
            .last_column
            .checked_add(1)?
            .checked_mul(SCREEN_CHANGE_TILE)?
            .min(dimensions.width());
        let bottom = self
            .last_row
            .checked_add(1)?
            .checked_mul(SCREEN_CHANGE_TILE)?
            .min(dimensions.height());
        let rect = CropRect::new(
            x,
            y,
            right.checked_sub(x)?,
            bottom.checked_sub(y)?,
            dimensions,
        )
        .ok()?;
        Some(ChangeRegion {
            rect,
            changed_pixels: self.changed_pixels,
        })
    }
}

fn pixel_changed(before: &[u8], after: &[u8]) -> bool {
    before
        .iter()
        .zip(after)
        .any(|(left, right)| left.abs_diff(*right) >= SCREEN_CHANGE_PIXEL_LEVEL)
}

fn tile_count(columns: u32, rows: u32) -> usize {
    usize::try_from(u64::from(columns) * u64::from(rows)).unwrap_or(0)
}

fn tile_index(columns: u32, column: u32, row: u32) -> usize {
    usize::try_from(u64::from(row) * u64::from(columns) + u64::from(column)).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests;
