//! The visual change rule measured on real recordings (#175, register L-028).
//!
//! The rule of profile `r0-visual-v1` (a block counts as changed at 4 grey
//! levels; two such blocks, or one at 6, is a change) was calibrated on the
//! synthetic corpus only. This measurement answers, on a real recording, how
//! far the rule's view of each pair of consecutive samples agrees with what
//! changed in the full-resolution frames. Nothing here changes the rule.
//!
//! For every window the real adapter's samples are decoded as an index
//! extension decodes them (`visual_decode::decode_file`), and the same frames
//! are decoded a second time at native resolution with the same `select`
//! expression (no scaling). Each pair of consecutive samples is then measured
//! twice:
//!
//! - **the rule's view:** the largest block-mean difference, how many blocks
//!   moved by at least each of `BLOCK_LEVELS`, whether `R0` calls it a change,
//!   and the distance of the two difference hashes;
//! - **the reference:** how many native pixels moved by at least 16, 32 and 64
//!   grey levels, the bounding box of those that moved by 64 and the most of
//!   them in any 32x32 tile. A pair whose native frames differ nowhere by 64
//!   levels is a *still* pair: a person sees no change between them. A
//!   changed pair is put in a size class by the share of the frame that
//!   moved (`SizeClass`). The reference level is 64, not 32, because the
//!   synthetic corpus's own static screens move by 32 to 63 levels at every
//!   keyframe (an encoder re-drawing text edges: up to 507 pixels spread over
//!   a box of 609x362, invisible to a person and to the block means), and
//!   never by 64.
//!
//! From those come the noise floor (the rule's view of still pairs), the
//! false changes a minute (still pairs the rule calls a change), the share of
//! each size class the rule sees, the same figures for a grid of other
//! thresholds (pairs only: the anchor comparison and the motion merging of the
//! real analysis are not replayed, so the grid is a comparison, not a
//! prediction of candidate counts), and the index's own candidates with the
//! reference behind each change candidate.
//!
//! The reference is a measurement, not a label: "changed" means pixels moved
//! by 64 levels, which includes a moving cursor or a blinking caret, so the
//! report keeps every size class apart and a person decides which ones should
//! be candidates. The labels of `docs/planning/real-media-set.md` section 6
//! remain the truth when they exist.
//!
//! Opt-in and hosted: the `P14 real media` workflow runs it with
//! `mode: visual` on the items it fetched (never on a development machine; it
//! decodes each recording twice). With `ffmpeg` and `ffprobe` on `PATH`:
//!
//! ```console
//! VSIFT_REAL_MEDIA_DIR=<the fetched items> VSIFT_VISUAL_MEASUREMENT_OUT=<file.json> \
//!   [VSIFT_REAL_MEDIA_ITEMS="RM10 RM01"] \
//!   cargo test --release -p vsift-infrastructure --locked --test p08_real_media_visual \
//!   -- --ignored --exact measures_the_change_rule_on_real_recordings --nocapture
//! ```
//!
//! The file holds measurements only: item ids, the pinned digests, numbers.
//! No pixel, block mean or path is written. The always-run tests below check
//! every measuring function on hand-made frames and samples.

mod candidate_recall;
mod visual_decode;

use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fs,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

use candidate_recall::{Built, RECORDED_FIXTURES, RecordedWindow, repository};
use serde_json::{Value, json};
use visual_decode::{Decoded, decode_file, ffmpeg_version, today, tools};
use vsift_domain::{
    CandidateReason, MediaTime, VISUAL_BLOCK_COLUMNS, VISUAL_BLOCK_ROWS, VISUAL_BLOCKS,
    VISUAL_SAMPLE_INTERVAL_MICROS, VisualChangePolicy, VisualIndex, VisualSample, VisualWindow,
    VisualWindowOutcome,
};
use vsift_infrastructure::{MediaProviderConformance, TrustedExecutable};

type TestResult = Result<(), Box<dyn Error>>;

/// Block-mean differences at which changed blocks are counted for each pair.
const BLOCK_LEVELS: [u8; 9] = [2, 3, 4, 5, 6, 8, 10, 12, 16];
/// Native grey-level differences at which changed pixels are counted.
const NATIVE_LEVELS: [u8; 3] = [16, 32, 64];
/// The native level (an index of `NATIVE_LEVELS`) that defines a still pair,
/// the bounding box and the tile count.
const REFERENCE_LEVEL: usize = 2;
/// Edge of the square tiles in which a change's concentration is counted.
const TILE_EDGE: usize = 32;
/// Largest mean block difference between the adapter's sample and the
/// native frame's own block means at which the two are taken to be the same
/// frame. Area scaling to 128x72 and a mean over the same region differ
/// only by rounding where the frame size is a multiple of the grid.
const PAIRING_TOLERANCE: u32 = 3;
/// The grid of alternative thresholds compared with `R0` on pairs.
const GRID_BLOCK_LEVELS: [u8; 5] = [3, 4, 5, 6, 8];
const GRID_MIN_BLOCKS: [u16; 4] = [1, 2, 3, 4];
const GRID_SINGLE_LEVELS: [u8; 6] = [6, 8, 10, 12, 16, 24];
/// `R0`'s own thresholds, the reference row of the grid.
const R0_THRESHOLDS: Thresholds = Thresholds {
    block_level: 4,
    min_blocks: 2,
    single_level: 6,
};
/// How far from a change a candidate may lie to count in the loose figure
/// (the campaign's check 7 uses one second).
const LOOSE_MICROS: u64 = 1_000_000;

/// How one pair of consecutive samples looks to the rule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RuleView {
    max_block_delta: u8,
    /// Blocks that moved by at least each of `BLOCK_LEVELS`.
    blocks_at: [u16; BLOCK_LEVELS.len()],
    r0_change: bool,
    hash_distance: u32,
}

fn rule_view(before: &VisualSample, after: &VisualSample) -> RuleView {
    let mut blocks_at = [0_u16; BLOCK_LEVELS.len()];
    let mut largest = 0_u8;
    for (left, right) in before.blocks().iter().zip(after.blocks()) {
        let difference = left.abs_diff(*right);
        largest = largest.max(difference);
        for (count, level) in blocks_at.iter_mut().zip(BLOCK_LEVELS) {
            if difference >= level {
                *count += 1;
            }
        }
    }
    let policy = VisualChangePolicy::R0;
    RuleView {
        max_block_delta: largest,
        blocks_at,
        r0_change: policy.is_change(policy.delta(before, after)),
        hash_distance: (before.hash().bits() ^ after.hash().bits()).count_ones(),
    }
}

/// How one pair of consecutive native frames differs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NativeDiff {
    /// Pixels that moved by at least each of `NATIVE_LEVELS`.
    changed: [u32; NATIVE_LEVELS.len()],
    /// Width and height of the box around the pixels that moved by the
    /// reference level, when any did.
    bbox: Option<(u32, u32)>,
    /// The most pixels that moved by the reference level in any one
    /// `TILE_EDGE` square: a real edit is dense, encoder noise is spread.
    densest_tile: u32,
}

fn native_diff(before: &[u8], after: &[u8], width: usize) -> NativeDiff {
    let mut changed = [0_u32; NATIVE_LEVELS.len()];
    let mut bounds: Option<(usize, usize, usize, usize)> = None;
    let columns = width.div_ceil(TILE_EDGE);
    let mut tiles = vec![0_u32; columns * (before.len() / width.max(1)).div_ceil(TILE_EDGE)];
    for (position, (left, right)) in before.iter().zip(after).enumerate() {
        let difference = left.abs_diff(*right);
        for (count, level) in changed.iter_mut().zip(NATIVE_LEVELS) {
            if difference >= level {
                *count += 1;
            }
        }
        if difference >= NATIVE_LEVELS[REFERENCE_LEVEL] {
            let (x, y) = (position % width, position / width);
            if let Some(tile) = tiles.get_mut(y / TILE_EDGE * columns + x / TILE_EDGE) {
                *tile += 1;
            }
            bounds = Some(bounds.map_or((x, y, x, y), |(x0, y0, x1, y1)| {
                (x0.min(x), y0.min(y), x1.max(x), y1.max(y))
            }));
        }
    }
    NativeDiff {
        changed,
        bbox: bounds.and_then(|(x0, y0, x1, y1)| {
            Some((
                u32::try_from(x1 - x0 + 1).ok()?,
                u32::try_from(y1 - y0 + 1).ok()?,
            ))
        }),
        densest_tile: tiles.iter().copied().max().unwrap_or(0),
    }
}

/// The 16x9 block means of a native frame, each over its share of the frame
/// (`[w * c / 16, w * (c + 1) / 16)` by `[h * r / 9, h * (r + 1) / 9)`):
/// what the adapter's area scaling and 8x8 means give, up to rounding.
fn native_block_means(frame: &[u8], width: usize, height: usize) -> [u8; VISUAL_BLOCKS] {
    let mut means = [0_u8; VISUAL_BLOCKS];
    for (index, mean) in means.iter_mut().enumerate() {
        let (column, row) = (index % VISUAL_BLOCK_COLUMNS, index / VISUAL_BLOCK_COLUMNS);
        let (x0, x1) = (
            width * column / VISUAL_BLOCK_COLUMNS,
            width * (column + 1) / VISUAL_BLOCK_COLUMNS,
        );
        let (y0, y1) = (
            height * row / VISUAL_BLOCK_ROWS,
            height * (row + 1) / VISUAL_BLOCK_ROWS,
        );
        let mut sum = 0_u64;
        for y in y0..y1 {
            if let Some(line) = frame.get(y * width + x0..y * width + x1) {
                sum += line.iter().map(|value| u64::from(*value)).sum::<u64>();
            }
        }
        let count = u64::try_from((x1 - x0) * (y1 - y0))
            .unwrap_or(u64::MAX)
            .max(1);
        *mean = u8::try_from((sum + count / 2) / count).unwrap_or(u8::MAX);
    }
    means
}

/// The mean absolute difference of two block-mean grids, rounded down.
fn mean_block_difference(left: &[u8; VISUAL_BLOCKS], right: &[u8; VISUAL_BLOCKS]) -> u32 {
    let total: u32 = left
        .iter()
        .zip(right)
        .map(|(a, b)| u32::from(a.abs_diff(*b)))
        .sum();
    total / u32::try_from(VISUAL_BLOCKS).unwrap_or(u32::MAX)
}

/// The share of the frame a change covered, in classes a person can name.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SizeClass {
    /// No native pixel moved by the reference level.
    Still,
    /// Under 20 parts per million of the frame (18 pixels at 1280x720): a
    /// caret, a speck of compression.
    Speck,
    /// 20 to 199 ppm (up to about 180 pixels at 1280x720): a cursor, one
    /// character.
    Tiny,
    /// 200 to 1,999 ppm (up to about 1,800 pixels): a word, a small icon.
    Small,
    /// 2,000 to 19,999 ppm (up to about 18,000 pixels): a line of text, a menu.
    Medium,
    /// 2% of the frame or more: a panel, a page, a scroll.
    Large,
}

impl SizeClass {
    const ALL: [Self; 6] = [
        Self::Still,
        Self::Speck,
        Self::Tiny,
        Self::Small,
        Self::Medium,
        Self::Large,
    ];

    fn of(diff: &NativeDiff, frame_pixels: u64) -> Self {
        let moved = u64::from(diff.changed[REFERENCE_LEVEL]);
        if moved == 0 {
            return Self::Still;
        }
        let ppm = moved.saturating_mul(1_000_000) / frame_pixels.max(1);
        match ppm {
            0..20 => Self::Speck,
            20..200 => Self::Tiny,
            200..2_000 => Self::Small,
            2_000..20_000 => Self::Medium,
            _ => Self::Large,
        }
    }

    const fn identifier(self) -> &'static str {
        match self {
            Self::Still => "still",
            Self::Speck => "speck",
            Self::Tiny => "tiny",
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }
}

/// A threshold set of the grid: the same shape as `R0`'s rule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Thresholds {
    block_level: u8,
    min_blocks: u16,
    single_level: u8,
}

impl Thresholds {
    fn fires(self, view: &RuleView) -> bool {
        let at = BLOCK_LEVELS
            .iter()
            .position(|level| *level == self.block_level)
            .and_then(|position| view.blocks_at.get(position).copied())
            .unwrap_or(0);
        at >= self.min_blocks || view.max_block_delta >= self.single_level
    }

    fn grid() -> Vec<Self> {
        let mut grid = Vec::new();
        for block_level in GRID_BLOCK_LEVELS {
            for min_blocks in GRID_MIN_BLOCKS {
                if min_blocks == 1 {
                    // Any one block at the block level fires: a separate
                    // single-block level adds nothing.
                    grid.push(Self {
                        block_level,
                        min_blocks,
                        single_level: block_level,
                    });
                    continue;
                }
                for single_level in GRID_SINGLE_LEVELS {
                    if single_level < block_level {
                        continue;
                    }
                    grid.push(Self {
                        block_level,
                        min_blocks,
                        single_level,
                    });
                }
            }
        }
        grid
    }
}

/// One measured pair of consecutive samples.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Pair {
    earlier_us: u64,
    later_us: u64,
    rule: RuleView,
    /// Absent when the window's native frames could not be paired with its
    /// samples.
    native: Option<NativeDiff>,
}

/// Every consecutive pair of each window's samples, the lead-in included,
/// with the native difference where the window was paired. A window's pairs
/// end at its own samples, so no pair is counted twice across windows.
fn pairs_of(windows: &[RecordedWindow], native: &BTreeMap<u32, Vec<NativeDiff>>) -> Vec<Pair> {
    let mut pairs = Vec::new();
    for window in windows {
        let diffs = native
            .get(&window.ordinal)
            .filter(|diffs| diffs.len() + 1 == window.samples.len());
        for (position, couple) in window.samples.windows(2).enumerate() {
            let [before, after] = couple else { continue };
            pairs.push(Pair {
                earlier_us: before.time().as_micros(),
                later_us: after.time().as_micros(),
                rule: rule_view(before, after),
                native: diffs.and_then(|diffs| diffs.get(position).copied()),
            });
        }
    }
    pairs
}

fn share(part: usize, whole: usize) -> Value {
    if whole == 0 {
        return Value::Null;
    }
    let (part, whole) = (
        u32::try_from(part).unwrap_or(u32::MAX),
        u32::try_from(whole).unwrap_or(u32::MAX),
    );
    json!((f64::from(part) / f64::from(whole) * 1_000.0).round() / 1_000.0)
}

fn per_minute(count: usize, duration_us: u64) -> Value {
    let minutes = f64::from(u32::try_from(duration_us / 1_000).unwrap_or(u32::MAX)) / 60_000.0;
    if minutes <= 0.0 {
        return Value::Null;
    }
    let count = f64::from(u32::try_from(count).unwrap_or(u32::MAX));
    json!((count / minutes * 100.0).round() / 100.0)
}

/// Quantiles of a list of levels: median, 95th and 99th percentile, maximum
/// (nearest rank).
fn quantiles(values: &mut [u8]) -> Value {
    if values.is_empty() {
        return Value::Null;
    }
    values.sort_unstable();
    let rank = |percent: usize| {
        let position = (values.len() * percent).div_ceil(100).max(1) - 1;
        values.get(position).copied().unwrap_or(0)
    };
    json!({ "p50": rank(50), "p95": rank(95), "p99": rank(99), "max": rank(100) })
}

/// The rule's view of still pairs: the noise floor the thresholds stand on.
fn noise_floor(pairs: &[Pair], frame_pixels: u64, duration_us: u64) -> Value {
    let still: Vec<&Pair> = pairs
        .iter()
        .filter(|pair| {
            pair.native
                .is_some_and(|diff| SizeClass::of(&diff, frame_pixels) == SizeClass::Still)
        })
        .collect();
    let mut levels: Vec<u8> = still.iter().map(|pair| pair.rule.max_block_delta).collect();
    let mut histogram = [0_usize; 18];
    for level in &levels {
        let slot = usize::from(*level).min(17);
        if let Some(count) = histogram.get_mut(slot) {
            *count += 1;
        }
    }
    let false_changes = still.iter().filter(|pair| pair.rule.r0_change).count();
    json!({
        "still_pairs": still.len(),
        "max_block_delta": quantiles(&mut levels),
        "max_block_delta_histogram_0_to_16_then_17_or_more": histogram,
        "r0_false_changes": false_changes,
        "r0_false_changes_per_minute": per_minute(false_changes, duration_us),
    })
}

/// Whether a change candidate of the index accounts for the change between
/// `pair`'s samples: a change candidate whose change ends at the pair's later
/// sample, or a motion candidate whose span holds it.
fn covered_exactly(index: &VisualIndex, pair: &Pair) -> bool {
    candidates_of(index).any(|candidate| {
        let at = candidate.representative().as_micros();
        match candidate.reason() {
            CandidateReason::VisualChange
            | CandidateReason::SettledAfterMotion
            | CandidateReason::FirstFrame => at == pair.later_us,
            CandidateReason::MotionStart => {
                at <= pair.later_us && pair.later_us < candidate.span().end().as_micros()
            }
            CandidateReason::PeriodicCoverage => false,
        }
    })
}

fn covered_loosely(index: &VisualIndex, pair: &Pair) -> bool {
    candidates_of(index).any(|candidate| {
        candidate.reason() != CandidateReason::PeriodicCoverage
            && candidate
                .representative()
                .as_micros()
                .abs_diff(pair.later_us)
                <= LOOSE_MICROS
    })
}

fn candidates_of(index: &VisualIndex) -> impl Iterator<Item = &vsift_domain::VisualCandidate> {
    index
        .windows()
        .iter()
        .flat_map(vsift_domain::VisualIndexWindow::candidates)
}

/// Per size class: how many pairs, how many the rule calls a change, and how
/// many a candidate of the index accounts for.
fn by_size_class(index: &VisualIndex, pairs: &[Pair], frame_pixels: u64) -> Value {
    let mut classes = serde_json::Map::new();
    for class in SizeClass::ALL {
        let members: Vec<&Pair> = pairs
            .iter()
            .filter(|pair| {
                pair.native
                    .is_some_and(|diff| SizeClass::of(&diff, frame_pixels) == class)
            })
            .collect();
        let seen = members.iter().filter(|pair| pair.rule.r0_change).count();
        let exact = members
            .iter()
            .filter(|pair| covered_exactly(index, pair))
            .count();
        let loose = members
            .iter()
            .filter(|pair| covered_loosely(index, pair))
            .count();
        classes.insert(
            class.identifier().to_owned(),
            json!({
                "pairs": members.len(),
                "r0_change": seen,
                "r0_change_share": share(seen, members.len()),
                "candidate_exact": exact,
                "candidate_exact_share": share(exact, members.len()),
                "candidate_within_1s": loose,
                "candidate_within_1s_share": share(loose, members.len()),
            }),
        );
    }
    Value::Object(classes)
}

/// The grid of other thresholds on the paired pairs: false changes a minute
/// on still pairs and the share of each changed size class each would see.
fn threshold_grid(pairs: &[Pair], frame_pixels: u64, duration_us: u64) -> Value {
    let classified: Vec<(SizeClass, &RuleView)> = pairs
        .iter()
        .filter_map(|pair| {
            pair.native
                .map(|diff| (SizeClass::of(&diff, frame_pixels), &pair.rule))
        })
        .collect();
    let rows: Vec<Value> = Thresholds::grid()
        .into_iter()
        .map(|thresholds| {
            let mut row = serde_json::Map::new();
            row.insert("block_level".to_owned(), json!(thresholds.block_level));
            row.insert("min_blocks".to_owned(), json!(thresholds.min_blocks));
            row.insert("single_level".to_owned(), json!(thresholds.single_level));
            row.insert("is_r0".to_owned(), json!(thresholds == R0_THRESHOLDS));
            for class in SizeClass::ALL {
                let members: Vec<&&RuleView> = classified
                    .iter()
                    .filter(|(member, _)| *member == class)
                    .map(|(_, view)| view)
                    .collect();
                let fired = members.iter().filter(|view| thresholds.fires(view)).count();
                if class == SizeClass::Still {
                    row.insert(
                        "false_per_minute".to_owned(),
                        per_minute(fired, duration_us),
                    );
                } else {
                    row.insert(
                        format!("{}_share", class.identifier()),
                        share(fired, members.len()),
                    );
                }
            }
            Value::Object(row)
        })
        .collect();
    Value::Array(rows)
}

/// The index's own candidates: counts by reason and stability, the budget's
/// drops, the sizes of the changes that opened them, and how many change
/// candidates have no native change in their own pair.
fn candidate_summary(index: &VisualIndex, pairs: &[Pair], frame_pixels: u64) -> Value {
    let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
    let mut stabilities: BTreeMap<&str, usize> = BTreeMap::new();
    let mut opened_by: BTreeMap<&str, usize> = BTreeMap::new();
    let mut deltas = Vec::new();
    let mut listed = Vec::new();
    for candidate in candidates_of(index) {
        *reasons.entry(candidate.reason().identifier()).or_default() += 1;
        *stabilities
            .entry(candidate.stability().identifier())
            .or_default() += 1;
        let at = candidate.representative().as_micros();
        let own_pair = pairs.iter().find(|pair| pair.later_us == at);
        let class = own_pair
            .and_then(|pair| pair.native)
            .map(|diff| SizeClass::of(&diff, frame_pixels));
        if let Some(change) = candidate.change() {
            deltas.push(change.delta().max_block_delta());
            *opened_by
                .entry(class.map_or("unpaired", SizeClass::identifier))
                .or_default() += 1;
        }
        listed.push(json!([
            at,
            candidate.reason().identifier(),
            candidate.stability().identifier(),
            candidate
                .change()
                .map(|change| change.delta().changed_blocks()),
            candidate
                .change()
                .map(|change| change.delta().max_block_delta()),
            class.map(SizeClass::identifier),
        ]));
    }
    let dropped: u32 = index
        .windows()
        .iter()
        .map(|window| match window.outcome() {
            VisualWindowOutcome::Analysed {
                dropped_candidates, ..
            } => u32::from(*dropped_candidates),
            VisualWindowOutcome::Undecodable => 0,
        })
        .sum();
    let over_budget = index
        .windows()
        .iter()
        .filter(|window| {
            matches!(window.outcome(), VisualWindowOutcome::Analysed { dropped_candidates, .. } if *dropped_candidates > 0)
        })
        .count();
    json!({
        "count": listed.len(),
        "per_minute": per_minute(listed.len(), index.duration().as_micros()),
        "by_reason": reasons,
        "by_stability": stabilities,
        "windows_over_budget": over_budget,
        "dropped": dropped,
        "change_candidates_by_native_class_of_their_pair": opened_by,
        "change_max_block_delta": quantiles(&mut deltas),
        "columns": ["representative_us", "reason", "stability", "changed_blocks", "max_block_delta", "native_class_of_its_pair"],
        "rows": listed,
    })
}

/// The microsecond count as `FFmpeg` reads seconds, with the sign kept.
fn seconds_arg(micros: i64) -> String {
    let sign = if micros < 0 { "-" } else { "" };
    let magnitude = micros.unsigned_abs();
    format!(
        "{sign}{}.{:06}",
        magnitude / 1_000_000,
        magnitude % 1_000_000
    )
}

/// The adapter's `select` expression for a window, without its scaling: the
/// frames at least 0.5 s apart from the lead-in start to the window end, on
/// the raw timeline.
fn select_filter(window: VisualWindow, origin_micros: i64) -> Built<String> {
    let raw = |time: MediaTime| -> Built<i64> {
        Ok(i64::try_from(time.as_micros())?
            .checked_add(origin_micros)
            .ok_or("time overflow")?)
    };
    Ok(format!(
        "select='gte(t\\,{lead})*lt(t\\,{end})*(isnan(prev_selected_t)+gte(t-prev_selected_t\\,{interval}))',format=gray",
        lead = seconds_arg(raw(window.lead_in_start())?),
        end = seconds_arg(raw(window.range().end())?),
        interval = seconds_arg(i64::try_from(VISUAL_SAMPLE_INTERVAL_MICROS)?),
    ))
}

/// Reads one frame of `buffer.len()` bytes; `false` at a clean end of stream.
fn read_frame(reader: &mut impl Read, buffer: &mut [u8]) -> Built<bool> {
    let mut filled = 0;
    while filled < buffer.len() {
        let Some(rest) = buffer.get_mut(filled..) else {
            break;
        };
        match reader.read(rest) {
            Ok(0) if filled == 0 => return Ok(false),
            Ok(0) => return Err("the native decode ended inside a frame".into()),
            Ok(read) => filled += read,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(true)
}

/// The native differences of one window's consecutive frames, when its
/// native decode returned exactly the adapter's frames; `None` otherwise.
fn native_window(
    ffmpeg: &TrustedExecutable,
    file: &Path,
    decoded: &Decoded,
    recorded: &RecordedWindow,
) -> Built<Option<(Vec<NativeDiff>, u32)>> {
    let window = VisualWindow::new(recorded.ordinal, decoded.recorded.duration)?;
    let width = usize::try_from(decoded.dimensions.width())?;
    let height = usize::try_from(decoded.dimensions.height())?;
    let seek = window.lead_in_start().as_micros().saturating_sub(2_000_000);
    let read = window.range().end().as_micros() - seek + 2_000_000;
    let mut child = Command::new(ffmpeg.path())
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-nostats"])
        .args(["-threads", "2", "-copyts", "-ss"])
        .arg(seconds_arg(i64::try_from(seek)?))
        .arg("-t")
        .arg(seconds_arg(i64::try_from(read)?))
        .arg("-i")
        .arg(file)
        .arg("-map")
        .arg(format!("0:{}", decoded.video_index))
        .args(["-an", "-sn", "-dn", "-vf"])
        .arg(select_filter(window, decoded.origin_micros)?)
        .args(["-fps_mode", "passthrough", "-frames:v", "122"])
        .args(["-f", "rawvideo", "-pix_fmt", "gray", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdout = child.stdout.take().ok_or("no native decode output")?;
    let mut previous = vec![0_u8; width * height];
    let mut current = vec![0_u8; width * height];
    let mut diffs = Vec::new();
    let mut frames = 0_usize;
    let mut worst = 0_u32;
    let mut paired = true;
    while read_frame(&mut stdout, &mut current)? {
        match recorded.samples.get(frames) {
            Some(sample) => {
                let difference = mean_block_difference(
                    &native_block_means(&current, width, height),
                    sample.blocks(),
                );
                worst = worst.max(difference);
                paired &= difference <= PAIRING_TOLERANCE;
            }
            None => paired = false,
        }
        if frames > 0 {
            diffs.push(native_diff(&previous, &current, width));
        }
        std::mem::swap(&mut previous, &mut current);
        frames += 1;
    }
    let status = child.wait()?;
    if !status.success() || frames != recorded.samples.len() || !paired {
        return Ok(None);
    }
    Ok(Some((diffs, worst)))
}

/// One approved, pinned item of the committed real-media manifest whose file
/// was fetched.
struct Item {
    id: String,
    file_name: String,
    /// The pinned digest the file must have; `None` for a corpus fixture,
    /// whose digests `generated/provenance.json` and its verifier hold.
    sha256: Option<String>,
    file: PathBuf,
}

fn fetched_items(media: &Path, wanted: &[String]) -> Built<(Vec<Item>, Vec<String>)> {
    let manifest: Value = serde_json::from_slice(&fs::read(repository(
        "tools/p14-campaigns/real-media/manifest.json",
    ))?)?;
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    for item in manifest["items"]
        .as_array()
        .ok_or("the manifest has no items")?
    {
        let id = item["id"].as_str().ok_or("an item has no id")?.to_owned();
        if !wanted.is_empty() && !wanted.contains(&id) {
            continue;
        }
        let (Some("approved"), Some(file_name), Some(sha256)) = (
            item["status"].as_str(),
            item["file_name"].as_str(),
            item["sha256"].as_str(),
        ) else {
            skipped.push(format!("{id}: not approved and pinned"));
            continue;
        };
        let file = media.join(&id).join(file_name);
        if !file.is_file() {
            skipped.push(format!("{id}: not fetched"));
            continue;
        }
        items.push(Item {
            id,
            file_name: file_name.to_owned(),
            sha256: Some(sha256.to_owned()),
            file,
        });
    }
    Ok((items, skipped))
}

async fn measure_item(
    item: &Item,
    ffmpeg: &TrustedExecutable,
    conformance: &MediaProviderConformance,
) -> Built<Value> {
    let decoded = decode_file(&item.file, &item.id, conformance).await?;
    if item
        .sha256
        .as_ref()
        .is_some_and(|pinned| *pinned != decoded.recorded.source_sha256)
    {
        return Err(format!("{}: the file is not the pinned one", item.id).into());
    }
    let started = Instant::now();
    let mut native = BTreeMap::new();
    let mut unpaired = Vec::new();
    let mut worst = 0_u32;
    for recorded in &decoded.recorded.windows {
        match native_window(ffmpeg, &item.file, &decoded, recorded)? {
            Some((diffs, difference)) => {
                worst = worst.max(difference);
                native.insert(recorded.ordinal, diffs);
            }
            None => unpaired.push(recorded.ordinal),
        }
    }
    let native_seconds = started.elapsed().as_secs_f64();
    let pairs = pairs_of(&decoded.recorded.windows, &native);
    let frame_pixels =
        u64::from(decoded.dimensions.width()) * u64::from(decoded.dimensions.height());
    let duration_us = decoded.recorded.duration.as_micros();
    let index = &decoded.index;
    let rows: Vec<Value> = pairs
        .iter()
        .map(|pair| {
            json!([
                pair.earlier_us,
                pair.later_us,
                pair.rule.max_block_delta,
                pair.rule.blocks_at,
                pair.rule.r0_change,
                pair.rule.hash_distance,
                pair.native.map(|diff| diff.changed),
                pair.native.and_then(|diff| diff.bbox),
                pair.native.map(|diff| diff.densest_tile),
                pair.native
                    .map(|diff| SizeClass::of(&diff, frame_pixels).identifier()),
            ])
        })
        .collect();
    Ok(json!({
        "id": item.id,
        "file_name": item.file_name,
        "sha256": decoded.recorded.source_sha256,
        "duration_us": duration_us,
        "displayed_dimensions": [decoded.dimensions.width(), decoded.dimensions.height()],
        "samples": decoded.recorded.windows.iter().map(|window| window.samples.len()).sum::<usize>(),
        "index_seconds": decoded.elapsed.as_secs_f64(),
        "native_decode_seconds": native_seconds,
        "pairing": {
            "windows": decoded.recorded.windows.len(),
            "unpaired_windows": unpaired,
            "worst_mean_block_difference": worst,
            "tolerance": PAIRING_TOLERANCE,
        },
        "noise_floor": noise_floor(&pairs, frame_pixels, duration_us),
        "by_size_class": by_size_class(index, &pairs, frame_pixels),
        "candidates": candidate_summary(index, &pairs, frame_pixels),
        "threshold_grid": threshold_grid(&pairs, frame_pixels, duration_us),
        "pairs": {
            "columns": ["earlier_us", "later_us", "max_block_delta", "blocks_at_levels", "r0_change", "hash_distance", "native_changed_at_levels", "native_bbox_at_64", "native_densest_32px_tile_at_64", "native_class"],
            "block_levels": BLOCK_LEVELS,
            "native_levels": NATIVE_LEVELS,
            "rows": rows,
        },
    }))
}

#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe on PATH and fetched real-media items; hosted only"]
async fn measures_the_change_rule_on_real_recordings() -> TestResult {
    let media = PathBuf::from(env::var("VSIFT_REAL_MEDIA_DIR")?);
    let output = PathBuf::from(env::var("VSIFT_VISUAL_MEASUREMENT_OUT")?);
    if !media.is_absolute() || !output.is_absolute() {
        return Err(
            "VSIFT_REAL_MEDIA_DIR and VSIFT_VISUAL_MEASUREMENT_OUT must be absolute".into(),
        );
    }
    let wanted: Vec<String> = env::var("VSIFT_REAL_MEDIA_ITEMS")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let (ffmpeg, ffprobe) = tools()?;
    let version = ffmpeg_version(&ffmpeg)?;
    let conformance = MediaProviderConformance::r0(ffmpeg.clone(), ffprobe);
    let (items, skipped) = fetched_items(&media, &wanted)?;
    if items.is_empty() {
        return Err(format!("no fetched item to measure ({skipped:?})").into());
    }
    let measured = measure_all(&items, &ffmpeg, &conformance).await?;
    let report = report(&version, &skipped, &measured)?;
    fs::write(
        &output,
        format!(
            "{}
",
            serde_json::to_string(&report)?
        ),
    )?;
    print_summary(&measured)
}

/// The same measurement over the synthetic corpus's visual fixtures: the
/// control. Where the thresholds were calibrated, still pairs should show a
/// noise floor of at most 1 level and no false change. Local runs are fine:
/// the eleven clips are short. With `VSIFT_VISUAL_MEASUREMENT_OUT` set the
/// whole report is written there.
#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
async fn measures_the_change_rule_on_the_synthetic_corpus() -> TestResult {
    let (ffmpeg, ffprobe) = tools()?;
    let version = ffmpeg_version(&ffmpeg)?;
    let conformance = MediaProviderConformance::r0(ffmpeg.clone(), ffprobe);
    let items: Vec<Item> = RECORDED_FIXTURES
        .iter()
        .map(|fixture| {
            let file_name = format!(
                "{fixture}.{}",
                if *fixture == "F09" { "mkv" } else { "mp4" }
            );
            Item {
                id: (*fixture).to_owned(),
                file: repository(&format!("fixtures/corpus/generated/{file_name}")),
                file_name,
                sha256: None,
            }
        })
        .collect();
    let measured = measure_all(&items, &ffmpeg, &conformance).await?;
    if let Ok(output) = env::var("VSIFT_VISUAL_MEASUREMENT_OUT") {
        let report = report(&version, &[], &measured)?;
        fs::write(
            output,
            format!(
                "{}
",
                serde_json::to_string(&report)?
            ),
        )?;
    }
    print_summary(&measured)
}

async fn measure_all(
    items: &[Item],
    ffmpeg: &TrustedExecutable,
    conformance: &MediaProviderConformance,
) -> Built<Vec<Value>> {
    let mut measured = Vec::new();
    for item in items {
        println!("measuring {}", item.id);
        measured.push(measure_item(item, ffmpeg, conformance).await?);
    }
    Ok(measured)
}

fn report(version: &str, skipped: &[String], measured: &[Value]) -> Built<Value> {
    Ok(json!({
        "measurement": "p08_real_media_visual (#175, L-028)",
        "profile": "r0-visual-v1",
        "r0_thresholds": { "block_level": R0_THRESHOLDS.block_level, "min_blocks": R0_THRESHOLDS.min_blocks, "single_level": R0_THRESHOLDS.single_level },
        "reference": "native frames at the samples' times; a pair is still when no pixel moved by 64 grey levels; size classes by the share of the frame that moved",
        "ffmpeg": version,
        "recorded": today()?,
        "skipped": skipped,
        "items": measured,
    }))
}

fn print_summary(measured: &[Value]) -> TestResult {
    for item in measured {
        println!(
            "{}: {}",
            item["id"],
            serde_json::to_string_pretty(&json!({
                "pairing": item["pairing"],
                "noise_floor": item["noise_floor"],
                "by_size_class": item["by_size_class"],
                "candidates": {
                    "per_minute": item["candidates"]["per_minute"],
                    "by_reason": item["candidates"]["by_reason"],
                    "dropped": item["candidates"]["dropped"],
                    "change_candidates_by_native_class_of_their_pair": item["candidates"]["change_candidates_by_native_class_of_their_pair"],
                },
            }))?
        );
    }
    Ok(())
}

#[cfg(test)]
mod measuring {
    use super::*;
    use vsift_domain::VisualHash;

    fn sample(time_us: u64, blocks: [u8; VISUAL_BLOCKS], hash: u64) -> VisualSample {
        VisualSample::from_parts(
            MediaTime::from_micros(time_us),
            blocks,
            VisualHash::from_bits(hash),
        )
    }

    fn view(max: u8, blocks_at: [u16; BLOCK_LEVELS.len()]) -> RuleView {
        RuleView {
            max_block_delta: max,
            blocks_at,
            r0_change: false,
            hash_distance: 0,
        }
    }

    #[test]
    fn the_rule_view_counts_blocks_at_each_level_and_agrees_with_r0() {
        let before = sample(0, [100; VISUAL_BLOCKS], 0b1010);
        let mut moved = [100_u8; VISUAL_BLOCKS];
        moved[0] = 105; // 5 levels: counted at 2, 3, 4 and 5
        moved[1] = 97; // 3 levels: counted at 2 and 3
        let after = sample(500_000, moved, 0b0110);
        let seen = rule_view(&before, &after);
        assert_eq!(seen.max_block_delta, 5);
        assert_eq!(seen.blocks_at, [2, 2, 1, 1, 0, 0, 0, 0, 0]);
        // One block at 4 or more, none at 6: not a change under R0.
        assert!(!seen.r0_change);
        assert_eq!(seen.hash_distance, 2);
        moved[1] = 104;
        let after = sample(500_000, moved, 0b1010);
        let seen = rule_view(&before, &after);
        // Two blocks at 4 or more: a change.
        assert!(seen.r0_change);
        assert_eq!(seen.hash_distance, 0);
        // The grid's R0 row decides as R0 does.
        assert!(R0_THRESHOLDS.fires(&seen));
    }

    #[test]
    fn the_grid_fires_on_either_arm_of_the_rule() {
        let one_strong = view(6, [1, 1, 1, 1, 1, 0, 0, 0, 0]);
        let two_weak = view(4, [2, 2, 2, 0, 0, 0, 0, 0, 0]);
        let noise = view(3, [5, 5, 0, 0, 0, 0, 0, 0, 0]);
        assert!(R0_THRESHOLDS.fires(&one_strong));
        assert!(R0_THRESHOLDS.fires(&two_weak));
        assert!(!R0_THRESHOLDS.fires(&noise));
        let stricter = Thresholds {
            block_level: 5,
            min_blocks: 3,
            single_level: 8,
        };
        assert!(!stricter.fires(&one_strong));
        assert!(!stricter.fires(&two_weak));
        // The grid holds R0 once, and no row repeats.
        let grid = Thresholds::grid();
        assert_eq!(grid.iter().filter(|row| **row == R0_THRESHOLDS).count(), 1);
        for (position, row) in grid.iter().enumerate() {
            assert!(!grid.iter().skip(position + 1).any(|other| other == row));
        }
    }

    #[test]
    fn the_native_difference_counts_pixels_at_each_level_and_boxes_them() {
        let width = 8;
        let before = vec![50_u8; width * 4];
        let mut after = before.clone();
        after[width + 2] = 70; // 20 levels: counted at 16 only
        after[2 * width + 5] = 90; // 40 levels: at 16 and 32
        after[3 * width + 6] = 130; // 80 levels: at every level
        after[3 * width + 1] = 200; // 150 levels: at every level
        let diff = native_diff(&before, &after, width);
        assert_eq!(diff.changed, [4, 3, 2]);
        // The box holds the two pixels that moved by 64: columns 1 to 6, row 3.
        assert_eq!(diff.bbox, Some((6, 1)));
        assert_eq!(diff.densest_tile, 2);
        let none = native_diff(&before, &before, width);
        assert_eq!((none.bbox, none.densest_tile), (None, 0));
    }

    #[test]
    fn size_classes_follow_the_share_of_the_frame() {
        let frame = 1_000_000_u64;
        let of = |moved: u32| {
            SizeClass::of(
                &NativeDiff {
                    changed: [moved, moved, moved],
                    bbox: None,
                    densest_tile: 0,
                },
                frame,
            )
        };
        assert_eq!(of(0), SizeClass::Still);
        assert_eq!(of(19), SizeClass::Speck);
        assert_eq!(of(20), SizeClass::Tiny);
        assert_eq!(of(199), SizeClass::Tiny);
        assert_eq!(of(200), SizeClass::Small);
        assert_eq!(of(2_000), SizeClass::Medium);
        assert_eq!(of(19_999), SizeClass::Medium);
        assert_eq!(of(20_000), SizeClass::Large);
    }

    #[test]
    fn native_block_means_match_the_adapter_on_a_grid_multiple() {
        // 128x72 is the adapter's own size: its 8x8 means are exactly these.
        let (width, height) = (128, 72);
        let frame: Vec<u8> = (0..width * height)
            .map(|position| u8::try_from((position % width) * 2).unwrap_or(0))
            .collect();
        let pixels: [u8; vsift_domain::VISUAL_FRAME_BYTES] = frame
            .clone()
            .try_into()
            .unwrap_or([0; vsift_domain::VISUAL_FRAME_BYTES]);
        let adapter = VisualSample::from_gray(MediaTime::from_micros(0), &pixels);
        let means = native_block_means(&frame, width, height);
        assert_eq!(&means, adapter.blocks());
        assert_eq!(mean_block_difference(&means, adapter.blocks()), 0);
        let shifted = means.map(|mean| mean.saturating_add(5));
        assert_eq!(mean_block_difference(&means, &shifted), 5);
    }

    #[test]
    fn pairs_stay_inside_their_window_and_take_native_differences_in_order() {
        let still = NativeDiff {
            changed: [0, 0, 0],
            bbox: None,
            densest_tile: 0,
        };
        let moved = NativeDiff {
            changed: [900, 800, 100],
            bbox: Some((40, 20)),
            densest_tile: 60,
        };
        let windows = vec![
            RecordedWindow {
                ordinal: 0,
                samples: vec![
                    sample(0, [10; VISUAL_BLOCKS], 0),
                    sample(500_000, [10; VISUAL_BLOCKS], 0),
                    sample(1_000_000, [30; VISUAL_BLOCKS], 0),
                ],
            },
            RecordedWindow {
                ordinal: 1,
                samples: vec![
                    sample(59_800_000, [30; VISUAL_BLOCKS], 0),
                    sample(60_300_000, [30; VISUAL_BLOCKS], 0),
                ],
            },
        ];
        // Window 1's native decode was not paired: its pair has no reference.
        let native = BTreeMap::from([(0, vec![still, moved]), (1, vec![still, still])]);
        let pairs = pairs_of(&windows, &native);
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0].native, Some(still));
        assert_eq!(pairs[1].native, Some(moved));
        assert!(pairs[1].rule.r0_change);
        assert_eq!(
            (pairs[2].earlier_us, pairs[2].later_us),
            (59_800_000, 60_300_000)
        );
        assert_eq!(pairs[2].native, None);
    }

    #[test]
    fn the_noise_floor_reads_still_pairs_only() {
        let still = Some(NativeDiff {
            changed: [3, 40, 0],
            bbox: None,
            densest_tile: 0,
        });
        let moved = Some(NativeDiff {
            changed: [500, 400, 10],
            bbox: Some((10, 10)),
            densest_tile: 10,
        });
        let pair = |max: u8, r0_change: bool, native| Pair {
            earlier_us: 0,
            later_us: 500_000,
            rule: RuleView {
                max_block_delta: max,
                blocks_at: [0; BLOCK_LEVELS.len()],
                r0_change,
                hash_distance: 0,
            },
            native,
        };
        let pairs = [
            pair(1, false, still),
            pair(2, false, still),
            pair(7, true, still),
            pair(40, true, moved),
            pair(9, true, None),
        ];
        let floor = noise_floor(&pairs, 1_000_000, 60_000_000);
        assert_eq!(floor["still_pairs"], 3);
        assert_eq!(floor["max_block_delta"]["max"], 7);
        assert_eq!(floor["max_block_delta"]["p50"], 2);
        assert_eq!(floor["r0_false_changes"], 1);
        assert_eq!(floor["r0_false_changes_per_minute"], 1.0);
    }

    #[test]
    fn quantiles_use_the_nearest_rank() {
        let mut levels: Vec<u8> = (1..=100).collect();
        assert_eq!(
            quantiles(&mut levels),
            json!({ "p50": 50, "p95": 95, "p99": 99, "max": 100 })
        );
        assert_eq!(quantiles(&mut []), Value::Null);
    }

    #[test]
    fn the_native_select_is_the_adapters_without_scaling() -> TestResult {
        let window = VisualWindow::new(1, MediaTime::from_micros(150_000_000))?;
        assert_eq!(
            select_filter(window, 0)?,
            "select='gte(t\\,59.500000)*lt(t\\,120.000000)*(isnan(prev_selected_t)+gte(t-prev_selected_t\\,0.500000))',format=gray"
        );
        // A source whose presentation starts late is selected on its raw timeline.
        assert!(
            select_filter(window, 1_250_000)?
                .starts_with("select='gte(t\\,60.750000)*lt(t\\,121.250000)")
        );
        assert_eq!(seconds_arg(-1_500_000), "-1.500000");
        Ok(())
    }

    #[test]
    fn a_frame_is_read_whole_or_the_stream_is_refused() -> TestResult {
        let mut buffer = [0_u8; 4];
        let mut whole: &[u8] = &[1, 2, 3, 4];
        assert!(read_frame(&mut whole, &mut buffer)?);
        assert_eq!(buffer, [1, 2, 3, 4]);
        assert!(!read_frame(&mut whole, &mut buffer)?);
        let mut cut: &[u8] = &[1, 2];
        assert!(read_frame(&mut cut, &mut buffer).is_err());
        Ok(())
    }
}
