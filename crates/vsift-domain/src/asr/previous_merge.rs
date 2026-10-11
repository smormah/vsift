//! A frozen copy of the seam merge as it was before the guard of #359: rules 1 to
//! 3 of `merge_chunks` at `origin/main` `52bc325`, kept in the tests as the
//! definition of what `merge_chunks` must still do where the guard has nothing
//! to restore, and as the baseline of what it restores where it has.
//!
//! Do not change it. A change to rules 1 to 3 is a change to what the property
//! tests compare with, and is made by changing the merge and saying so.

use std::collections::BTreeSet;

use super::{
    AsrSegmentDraft, ChunkSegments, CueText, MIN_SEAM_DUPLICATE_WORDS, MergedSegment,
    SEAM_EDGE_MICROS, TranscriptWarningKind, TranscriptWarnings,
};

/// What the merge gave before the guard: the segments and the repeats it counted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PreviousMerge {
    pub(super) segments: Vec<MergedSegment>,
    pub(super) warnings: TranscriptWarnings,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Side {
    Left,
    Right,
}

pub(super) fn previous_merge_chunks(chunks: &[ChunkSegments]) -> PreviousMerge {
    let cores = chunk_cores(chunks);
    let mut kept = BTreeSet::new();
    for (index, chunk) in chunks.iter().enumerate() {
        for (position, segment) in chunk.segments.iter().enumerate() {
            let middle = segment.midpoint();
            let owner = cores
                .iter()
                .position(|(start, end)| middle >= *start && middle < *end)
                .unwrap_or(index);
            if owner != index {
                // The owning chunk decides, unless it heard nothing at this
                // time; then this copy is the only one and is kept.
                if !overlapping(&chunks[owner].segments, segment) {
                    kept.insert((index, position));
                }
                continue;
            }
            let mut cut_sides = Vec::new();
            if cut_at(chunks, index, segment, Side::Left) {
                cut_sides.push((index - 1, Side::Right));
            }
            if cut_at(chunks, index, segment, Side::Right) {
                cut_sides.push((index + 1, Side::Left));
            }
            let mut replacements = Vec::new();
            let covered = !cut_sides.is_empty()
                && cut_sides.iter().all(|(neighbour, facing)| {
                    let found = covering(chunks, *neighbour, segment, *facing);
                    let any = !found.is_empty();
                    replacements.extend(found);
                    any
                });
            if covered {
                kept.extend(replacements);
            } else {
                kept.insert((index, position));
            }
        }
    }
    let mut ordered: Vec<(usize, AsrSegmentDraft)> = kept
        .into_iter()
        .filter_map(|(index, position)| {
            chunks
                .get(index)
                .and_then(|chunk| chunk.segments.get(position))
                .map(|segment| (index, segment.clone()))
        })
        .collect();
    ordered.sort_by_key(|(index, segment)| (segment.range.start(), segment.range.end(), *index));
    let warnings = remove_seam_duplicates(&mut ordered, chunks);
    PreviousMerge {
        segments: ordered
            .into_iter()
            .map(|(index, segment)| MergedSegment {
                chunk: u32::try_from(index).unwrap_or(u32::MAX),
                segment,
            })
            .collect(),
        warnings,
    }
}

fn overlapping(segments: &[AsrSegmentDraft], segment: &AsrSegmentDraft) -> bool {
    segments.iter().any(|candidate| {
        candidate.range.start() < segment.range.end()
            && candidate.range.end() > segment.range.start()
    })
}

/// Each chunk's `[start, end)` core: overlap midpoints to overlap midpoints.
fn chunk_cores(chunks: &[ChunkSegments]) -> Vec<(u64, u64)> {
    let boundaries: Vec<u64> = chunks
        .windows(2)
        .map(|pair| {
            u64::midpoint(
                pair[1].chunk.window.start().as_micros(),
                pair[0].chunk.window.end().as_micros(),
            )
        })
        .collect();
    (0..chunks.len())
        .map(|index| {
            let start = if index == 0 { 0 } else { boundaries[index - 1] };
            let end = boundaries.get(index).copied().unwrap_or(u64::MAX);
            (start, end)
        })
        .collect()
}

/// Whether `segment` of chunk `index` lies within the edge margin of an inner
/// window edge on `side`.
fn cut_at(chunks: &[ChunkSegments], index: usize, segment: &AsrSegmentDraft, side: Side) -> bool {
    let window = chunks[index].chunk.window;
    match side {
        Side::Left => {
            index > 0
                && segment.range.start().as_micros()
                    < window.start().as_micros().saturating_add(SEAM_EDGE_MICROS)
        }
        Side::Right => {
            index + 1 < chunks.len()
                && segment
                    .range
                    .end()
                    .as_micros()
                    .saturating_add(SEAM_EDGE_MICROS)
                    > window.end().as_micros()
        }
    }
}

/// Segments of chunk `neighbour` spanning the midpoint of `segment` and not
/// cut at the neighbour's edge that faces it.
///
/// Spanning the midpoint, not merely overlapping, is what makes a neighbour
/// segment a copy of the same speech: a segment that ends just inside the
/// cut one is the sentence before it, and treating it as a copy would drop
/// the cut sentence from both chunks (found with the `base_q5_1` profile in
/// P07 increment 3c).
fn covering(
    chunks: &[ChunkSegments],
    neighbour: usize,
    segment: &AsrSegmentDraft,
    facing: Side,
) -> Vec<(usize, usize)> {
    let middle = segment.midpoint();
    chunks[neighbour]
        .segments
        .iter()
        .enumerate()
        .filter(|(_, candidate)| {
            candidate.range.start().as_micros() <= middle
                && candidate.range.end().as_micros() > middle
                && !cut_at(chunks, neighbour, candidate, facing)
        })
        .map(|(position, _)| (neighbour, position))
        .collect()
}

fn remove_seam_duplicates(
    ordered: &mut Vec<(usize, AsrSegmentDraft)>,
    chunks: &[ChunkSegments],
) -> TranscriptWarnings {
    let mut warnings = TranscriptWarnings::default();
    for (seam, later_chunk) in chunks.iter().enumerate().skip(1) {
        let earlier = ordered.iter().rposition(|(index, _)| *index == seam - 1);
        let later = ordered.iter().position(|(index, _)| *index == seam);
        let (Some(earlier), Some(later)) = (earlier, later) else {
            continue;
        };
        let (first, second) = (&ordered[earlier].1, &ordered[later].1);
        if first.range.start() >= second.range.end() || second.range.start() >= first.range.end() {
            continue;
        }
        let earlier_words = normalized_words(first.text.text());
        let later_words = normalized_words(second.text.text());
        let shared = shared_seam_words(&earlier_words, &later_words);
        if shared == 0 {
            continue;
        }
        let remaining = (shared < later_words.len())
            .then(|| without_leading_words(second.text.text(), shared))
            .flatten();
        match remaining {
            Some(text) => ordered[later].1.text = text,
            None => {
                ordered.remove(later);
            }
        }
        warnings.add(
            TranscriptWarningKind::SeamDuplicatesRemoved,
            1,
            later_chunk.chunk.ordinal(),
        );
    }
    warnings
}

/// Number of leading words of `later` repeated at the end of `earlier`, or
/// all of `later` when it appears whole inside `earlier`; zero when fewer than
/// [`MIN_SEAM_DUPLICATE_WORDS`] words are shared.
fn shared_seam_words(earlier: &[String], later: &[String]) -> usize {
    if later.len() >= MIN_SEAM_DUPLICATE_WORDS
        && earlier.windows(later.len()).any(|window| window == later)
    {
        return later.len();
    }
    (MIN_SEAM_DUPLICATE_WORDS..=earlier.len().min(later.len()))
        .rev()
        .find(|length| earlier[earlier.len() - length..] == later[..*length])
        .unwrap_or(0)
}

/// Lowercase words with punctuation removed, for comparing seam text.
fn normalized_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|character| character.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

/// `text` without its first `count` normalised words, or `None` when no word remains.
fn without_leading_words(text: &str, count: usize) -> Option<CueText> {
    let mut skipped = 0;
    let mut rest = Vec::new();
    for word in text.split_whitespace() {
        let counts = word.chars().any(char::is_alphanumeric);
        if skipped < count {
            if counts {
                skipped += 1;
            }
            continue;
        }
        rest.push(word);
    }
    if !rest
        .iter()
        .any(|word| word.chars().any(char::is_alphanumeric))
    {
        return None;
    }
    let joined = rest.join(" ");
    CueText::new(joined.clone(), joined).ok()
}
