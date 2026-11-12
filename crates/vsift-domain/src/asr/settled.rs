//! The settled part of a run whose later chunks are not finished yet (ADR 0033).
//!
//! A transcription that publishes as it goes may only publish what no chunk
//! still to come can change: a published segment is evidence, cited at once,
//! and keeps its text, time and identity in the revision the run seals. This
//! module decides that part. It changes no merge rule: it runs
//! [`merge_chunks`] over the finished chunks and keeps the segments the
//! unfinished ones cannot reach.
//!
//! **Why the merge is local.** Every rule of [`merge_chunks`] relates a
//! segment only to segments that overlap it in time, or to the segments of its
//! own chunk:
//!
//! - rule 1 asks whether the chunk that owns a segment's midpoint has a segment
//!   overlapping it, and rule 2 whether a neighbour has one spanning its
//!   midpoint; both look at the segment's own time;
//! - the guard closes holes inside one stretch of the union of all validated
//!   segments, with dropped segments that overlap the hole;
//! - rule 3 compares kept segments of consecutive chunks whose times overlap,
//!   and drops a copy only when no hole would open in the stretches it lies in.
//!
//! A stretch of that union (overlapping or touching segments joined) is
//! therefore decided by the segments inside it, and by the whole output of the
//! chunks they come from. Two kinds of decision reach past a stretch, and both
//! are excluded below: the core and edge of the last finished chunk, which
//! [`merge_chunks`] decides as if no chunk followed (its right edge is not
//! "cut", its core runs to the end), and the segments a later chunk adds to a
//! stretch that reaches its audio.
//!
//! **The reach.** Let `next` be the first planned chunk that is not finished
//! (the chunks are recognised one after another in plan order, so the finished
//! ones are always the first of the plan, gaps included as empty chunks). None
//! of its segments, nor any later chunk's, starts before
//! `next.window().start()` less the early-audio bound (the audio a decoder
//! returns may begin before the window it was asked for, at an earlier frame
//! boundary; [`MAX_EARLY_AUDIO_MICROS`] for a recorded file). A run stops
//! publishing at a chunk whose audio began earlier than its bound, and the
//! revision it seals is checked against what it published, so a source that
//! breaks the bound stops the publication rather than changing what was
//! published. Call that time the reach.
//!
//! **The stretch cut.** Every stretch of the finished chunks' union that ends
//! before the reach is final: no later segment can join it, overlap it or
//! touch it, and the last finished chunk's right edge and core lie past the
//! reach (its window ends after `next` starts, and its core after the middle
//! of their overlap), so no segment in such a stretch is decided by them. The
//! stretch cut is the start of the first stretch that ends at or after the
//! reach, or the reach itself when there is none.
//!
//! **The bounded cut (L-256).** Continuous speech is one long stretch, which
//! the stretch cut holds back whole. Inside it, the merge reaches back only so
//! far, though not by a fixed time:
//!
//! - **Rules 1 and 2** change, once later chunks are finished, only segments
//!   that end after `λ`, the reach or half a second (the edge margin) before
//!   the last finished window's end, whichever is earlier: a segment whose
//!   midpoint lies in the last finished chunk's core past its next seam, one of
//!   that chunk's segments within the edge margin of its right window edge,
//!   and one that rule 2 takes as the replacement of a later chunk's segment
//!   (it spans that segment's midpoint, after the reach).
//! - **The guard** closes each hole of the union with dropped segments ordered
//!   by how much of the whole hole they cover, and nothing bounds how long a
//!   hole is, so it can reach back any distance. It is cut only at a point
//!   that no segment crosses and where a segment kept by rules 1 and 2 ends:
//!   no hole contains that point, holes before it are closed from segments
//!   that end by it, and holes after it from segments that start after it.
//! - **Rule 3** drops a copy only where no hole longer than the tolerance
//!   ([`super::SEAM_COVERAGE_TOLERANCE_MICROS`]) would open, and every hole is
//!   within the tolerance after the guard, so the hole it looks for runs at
//!   most the tolerance past the copy's end. The repeat and loop rules act
//!   through rules 1 and 2.
//!
//! So: `T` is the latest point before `λ` that no segment of a finished chunk
//! crosses (a segment that ends there and one that starts there may touch) and
//! where a segment kept by rules 1 and 2 ends; the bounded cut is the latest
//! point that no such segment crosses and that lies more than the tolerance
//! before `T`. Every segment that starts before `T` is decided by rules 1 and 2
//! and by the guard as the whole run's merge decides it; the bounded cut
//! separates the segments that end by it from all others in the merge's order;
//! and rule 3 decides each of those as the whole run does, because what it
//! looks at ends before `T`.
//!
//! **The settled time** is the later of the two cuts (each alone settles a
//! prefix, so the later one does). The settled part is every merged segment
//! that ends at or before it. They are a prefix of the merged order of the whole
//! run: every other segment of the run starts at or after the settled time, and
//! each settled one ends at or before it. Each cut only moves forward as chunks
//! finish, so the settled part only grows. On a long stretch of speech the
//! settled time trails the finished audio by the overlap, the early-audio bound
//! and about two segments; a single segment as long as a window still holds
//! its stretch back, so the bound is one of structure, not of time.
//!
//! **What holds it.** The claim is pinned by tests, not by this argument:
//! `every_cut_of_a_run_settles_a_prefix_of_its_merge` and
//! `every_cut_of_a_continuous_run_settles_a_prefix_of_its_merge` (and every
//! merge test of this module, through their helper) cut each run after every
//! chunk and check that the settled part is exactly the first segments of the
//! whole run's merge, byte for byte, and that each cut settles at least what
//! the one before it did; with the bounded cut allowed past the reach, they
//! find a counterexample. The cut's edge term and rule 3's margin are redundant
//! safety margins, kept on purpose (ADR 0033, amendment of 2026-11-11). A
//! counterexample makes the cut more conservative; the merge rules are never
//! changed to suit it.

use std::collections::BTreeSet;

use crate::{
    MediaTime, TimeRange, TranscriptWarnings,
    spans::{Span, merged, span},
};

use super::{
    ChunkSegments, MergedSegment, PlannedChunk, SEAM_COVERAGE_TOLERANCE_MICROS, SEAM_EDGE_MICROS,
    keep_by_rules_one_and_two, merge_chunks,
};

/// The version of the merge rules ([`merge_chunks`] and the validation of a
/// chunk's output), recorded in every part of an open revision (ADR 0033).
///
/// A run resumed by a `VSift` whose rules are another version could settle a
/// published stretch differently, so it seals the open revision as stopped at
/// what it published instead of publishing more. The version is bumped with
/// every change to what the merge makes of the same chunks; the test
/// `the_merge_rules_version_names_what_the_merge_makes` holds a digest of the
/// merge of a fixed sweep of runs and fails when the rules change without it.
/// Version 2 (#539): a segment that loops on its own words claims no time.
pub const MERGE_RULES_VERSION: u32 = 2;

/// The most that a chunk's decoded audio, and so any segment of it, may begin
/// before the window the chunk planned: five seconds.
///
/// A decoder seeks to the frame boundary at or before the window's start, which
/// for real audio is milliseconds away. The bound is the same slack a stored
/// chunk checkpoint is allowed ([`crate::ChunkCheckpoint::belongs_to`] accepts
/// audio no longer than its window plus five seconds that overlaps it). A run
/// that publishes as it goes stops publishing at a chunk whose audio began
/// earlier, because [`settled_merge`] relies on the bound: a segment that began
/// further back could reach a stretch already published (and the seal checks
/// what was published, so it is never changed).
pub const MAX_EARLY_AUDIO_MICROS: u64 = 5_000_000;

/// The part of a run's merge that no unfinished chunk can change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettledTranscript {
    /// Everything before this time is final: the run's merge has exactly
    /// [`Self::segments`] ending at or before it, and no other segment starts
    /// before it. When the run is finished, the end of its last window.
    pub settled_until: MediaTime,
    /// The first segments of the run's merge, in its order, exactly as the
    /// merge of the whole run will hold them.
    pub segments: Vec<MergedSegment>,
    /// The stretches of unplaced words ([`crate::MergedTranscript::unplaced`])
    /// that end before [`Self::settled_until`], as the merge of the whole run
    /// will report them, unless that merge has to join stretches to stay within
    /// [`crate::MAX_UNPLACED_STRETCHES`].
    pub unplaced: Vec<TimeRange>,
    /// The seam duplicates the merge of the finished chunks removed: what the
    /// run's warning counts are so far, not a settled figure (the last finished
    /// chunk's seam with the next is not judged yet).
    pub warnings: TranscriptWarnings,
}

/// The settled part of a run of which `finished` are the first chunks, in plan
/// order, and `next` the first chunk that is not finished (`None` when every
/// chunk is), as the module's documentation says.
///
/// `max_early_audio_us` is the most that the run lets a chunk's audio begin
/// before its window: [`MAX_EARLY_AUDIO_MICROS`] for a recorded file. It is a
/// parameter and not the constant because it sets the least delay between
/// audio being heard and its text being settled; a source that bounds its
/// decoder more tightly (a live source with second-level latency, ADR 0033
/// section 8) passes its own bound.
///
/// With no finished chunk nothing is settled, and the settled time is the
/// reach of `next` (its window's start less `max_early_audio_us`), which
/// can lie before the run's start: its first segment may begin that early. A
/// settled time before the run's start settles nothing either.
#[must_use]
pub fn settled_merge(
    finished: &[ChunkSegments],
    next: Option<&PlannedChunk>,
    max_early_audio_us: u64,
) -> SettledTranscript {
    settled_merge_within(finished, next, max_early_audio_us, CutMargins::MERGE)
}

/// The margins of the bounded cut. A parameter only so that a test can loosen
/// one and see the cut fail
/// (`a_bounded_cut_allowed_past_the_reach_settles_what_the_seal_changes`).
///
/// The edge term and rule 3's margin are **redundant safety margins, kept on
/// purpose**: every segment a later chunk can change ends after the reach
/// itself, and the kept segment that ends at `T` is still there when rule 3
/// judges a settled segment (ADR 0033, amendment of 2026-11-11). They cost at
/// most 60 ms and about one segment of trail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CutMargins {
    /// How far before the last finished window's end, in microseconds, rule 2
    /// may still call a segment cut at that edge once the next chunk is
    /// finished: [`SEAM_EDGE_MICROS`].
    pub(super) edge_margin: u64,
    /// How far past a segment's end, in microseconds, rule 3 may look for a
    /// hole: [`SEAM_COVERAGE_TOLERANCE_MICROS`].
    pub(super) rule_three_reach: u64,
    /// How far past the reach, in microseconds, the bounded cut may look: zero,
    /// since a later chunk's segment may start at the reach. Only a test sets
    /// more.
    pub(super) past_the_reach: u64,
}

impl CutMargins {
    /// The margins [`merge_chunks`] needs.
    pub(super) const MERGE: Self = Self {
        edge_margin: SEAM_EDGE_MICROS,
        rule_three_reach: SEAM_COVERAGE_TOLERANCE_MICROS,
        past_the_reach: 0,
    };
}

/// [`settled_merge`] with the bounded cut's margins given.
pub(super) fn settled_merge_within(
    finished: &[ChunkSegments],
    next: Option<&PlannedChunk>,
    max_early_audio_us: u64,
    margins: CutMargins,
) -> SettledTranscript {
    let merged_run = merge_chunks(finished);
    let Some(next) = next else {
        let settled_until = finished.last().map_or(MediaTime::from_micros(0), |chunk| {
            chunk.chunk.window().end()
        });
        return SettledTranscript {
            settled_until,
            segments: merged_run.segments,
            unplaced: merged_run.unplaced,
            warnings: merged_run.warnings,
        };
    };
    let reach = next
        .window()
        .start()
        .as_micros()
        .saturating_sub(max_early_audio_us);
    let spans: Vec<Span> = finished
        .iter()
        .flat_map(|chunk| chunk.segments.iter().map(|segment| span(segment.range())))
        .collect();
    let union = merged(spans.clone());
    // The stretch cut. `end >= reach`: a stretch that ends exactly at the reach
    // could be touched by a later segment that starts there, so it is held.
    let stretch_cut = union
        .iter()
        .find(|&&(_, end)| end >= reach)
        .map_or(reach, |&(start, _)| start.min(reach));
    let bounded = finished.last().and_then(|last| {
        let lambda = reach.saturating_add(margins.past_the_reach).min(
            last.chunk
                .window()
                .end()
                .as_micros()
                .saturating_sub(margins.edge_margin),
        );
        // Only a cut inside the stretch the stretch cut holds can settle more.
        (stretch_cut < lambda)
            .then(|| bounded_cut(finished, spans, lambda, margins.rule_three_reach))
            .flatten()
    });
    let cut = bounded.map_or(stretch_cut, |bounded| bounded.max(stretch_cut));
    let settled_until = MediaTime::from_micros(cut);
    let segments = merged_run
        .segments
        .into_iter()
        .take_while(|segment| segment.segment.range().end() <= settled_until)
        .collect();
    // A stretch of unplaced words that ends exactly at the cut may be joined
    // with one of the next chunk's in the whole run's merge, so only those
    // that end before it are settled.
    let unplaced = merged_run
        .unplaced
        .into_iter()
        .filter(|stretch| stretch.end() < settled_until)
        .collect();
    SettledTranscript {
        settled_until,
        segments,
        unplaced,
        warnings: merged_run.warnings,
    }
}

/// The bounded cut of the module's documentation, inside a stretch that runs
/// past `lambda`; `None` when no point qualifies. `spans` are the finished
/// chunks' validated segments.
///
/// Every boundary is held back where the rules would allow it, so that an
/// equality never settles a case the proof does not cover:
///
/// - `T` is **before** `lambda`, not at it. A segment that rules 1 and 2 may
///   still change ends strictly after `lambda` (rule 2's edge test is
///   `end + margin > window end`), so one ending exactly at `lambda` would be
///   safe, but it is not used.
/// - The same holds at `window end − edge_margin`, which `lambda` takes when it
///   is the earlier of the two (a source whose early-audio bound is under the
///   edge margin). That term is a redundant margin, kept on purpose: every
///   segment a later chunk can change ends after the reach itself.
/// - A later chunk's segment may start exactly at the reach, at or after
///   `lambda`, so it can touch `T` only if `T` were at `lambda`, which it is
///   not.
/// - `T` may be touched: a segment that ends there and one that starts there
///   are on either side of it. Only a segment that crosses it (starts before,
///   ends after) disqualifies it.
/// - The cut is **strictly** more than `rule_three_reach` before `T`. Rule 3 opens
///   no hole longer than the tolerance (`until − from > tolerance` is a hole),
///   so the hole it looks for past a segment's end can be exactly the tolerance
///   long. The margin is a redundant one, kept on purpose: the kept segment
///   that ends at `T` starts at or after the cut, so it is still there when
///   rule 3 judges a settled segment, and that segment's hole test stops before
///   `T` whatever the margin; the cut is strictly before `T` in any case.
fn bounded_cut(
    finished: &[ChunkSegments],
    mut spans: Vec<Span>,
    lambda: u64,
    rule_three_reach: u64,
) -> Option<u64> {
    let blocks = uncrossed_blocks(&mut spans);
    let kept_ends: BTreeSet<u64> = keep_by_rules_one_and_two(finished)
        .into_iter()
        .filter_map(|(index, position)| {
            finished
                .get(index)
                .and_then(|chunk| chunk.segments.get(position))
                .map(|segment| segment.range().end().as_micros())
        })
        .collect();
    // A block's end is a point no segment crosses. A kept segment that ends
    // there covers the instant before it, so no hole of the guard contains it.
    let guard_cut = blocks
        .iter()
        .map(|&(_, end)| end)
        .filter(|&end| end < lambda && kept_ends.contains(&end))
        .max()?;
    let limit = guard_cut.checked_sub(rule_three_reach)?;
    blocks
        .iter()
        .flat_map(|&(start, end)| [start, end])
        .filter(|&point| point < limit)
        .max()
}

/// The blocks of `spans` that segments **cross**: spans joined where one
/// starts strictly before another ends, so that touching spans stay apart.
/// Every start and end of a block is a point that no span crosses, and every
/// point that none crosses is outside the blocks' insides.
fn uncrossed_blocks(spans: &mut [Span]) -> Vec<Span> {
    spans.sort_unstable();
    let mut blocks: Vec<Span> = Vec::with_capacity(spans.len());
    for &(start, end) in spans.iter() {
        match blocks.last_mut() {
            Some(last) if start < last.1 => last.1 = last.1.max(end),
            _ => blocks.push((start, end)),
        }
    }
    blocks
}
