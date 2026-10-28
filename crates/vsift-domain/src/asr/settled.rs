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
//! **The cut.** Let `next` be the first planned chunk that is not finished.
//! None of its segments, nor any later chunk's, starts before
//! `next.window().start()` less the early-audio bound (the audio a decoder
//! returns may begin before the window it was asked for, at an earlier frame
//! boundary; [`MAX_EARLY_AUDIO_MICROS`] for a recorded file). A run stops
//! publishing at a chunk whose audio began earlier than its bound, and the
//! revision it seals is checked against what it published, so a source that
//! breaks the bound stops the publication rather than changing what was
//! published. Call
//! that time the reach. Every stretch of the finished chunks' union that ends
//! before the reach is final: no later segment can join it, overlap it or touch
//! it, and the last finished chunk's right edge and core lie past the reach (its
//! window ends after `next` starts, and its core after the middle of their
//! overlap), so no segment in such a stretch is decided by them. The **settled
//! time** is the start of the first stretch that ends at or after the reach, or
//! the reach itself when there is none. The settled part is every merged
//! segment that ends at or before it, which is exactly the segments of the
//! final stretches. They are a prefix of the merged order of the whole run:
//! every other segment of the run starts at or after the settled time, and
//! each settled one ends at or before it.
//!
//! **What holds it.** The claim is pinned by a test, not by this argument:
//! `every_cut_of_a_run_settles_a_prefix_of_its_merge` (and every merge test of
//! this module, through its helper) cuts each run after every chunk and checks
//! that the settled part is exactly the first segments of the whole run's merge,
//! byte for byte, and that each cut settles at least what the one before it did.
//! A counterexample makes the cut more conservative; the merge rules are never
//! changed to suit it.

use crate::{MediaTime, TimeRange, TranscriptWarnings, spans::merged, spans::span};

use super::{ChunkSegments, MergedSegment, PlannedChunk, merge_chunks};

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
    let union = merged(
        finished
            .iter()
            .flat_map(|chunk| chunk.segments.iter().map(|segment| span(segment.range())))
            .collect(),
    );
    let cut = union
        .iter()
        .find(|&&(_, end)| end >= reach)
        .map_or(reach, |&(start, _)| start.min(reach));
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
