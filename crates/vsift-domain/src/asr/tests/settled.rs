//! ADR 0033: the settled part of a run is a prefix of its whole merge, at every
//! cut.
//!
//! A run that publishes as it goes publishes [`settled_merge`] of its finished
//! chunks, and seals with [`merge_chunks`] of all of them. A published segment
//! is evidence, so the two must agree exactly on every segment the first one
//! published. These tests cut runs after every chunk (the full matrix of cut
//! points) and hold each cut to that.

use proptest::prelude::{ProptestConfig, prop, prop_assert_eq, prop_oneof, proptest};

use super::{
    Built, ChunkAnswer, SECOND, TestResult, Words, chunk, chunk_answer, draft, empty,
    generated_with_plan, with,
};
use crate::{
    MediaTime,
    asr::{
        ChunkPlan, ChunkSegments, MAX_EARLY_AUDIO_MICROS, SettledTranscript, merge_chunks,
        settled_merge,
    },
};

/// What the cuts of one run showed, for the tests that check the matrix is not
/// vacuous.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct CutCounts {
    /// Cuts made (one more than the chunks: before the first and after the last).
    pub(super) cuts: u32,
    /// Cuts that settled some of the run's segments but not all.
    pub(super) partial: u32,
    /// Cuts whose settled time is before the reach, because a stretch of the
    /// union ran into the next chunk's reach.
    pub(super) held_back: u32,
}

/// The earliest time a segment of chunk `index` or a later one starts, if any.
fn earliest_later_start(chunks: &[ChunkSegments], index: usize) -> Option<u64> {
    chunks
        .get(index..)
        .unwrap_or_default()
        .iter()
        .flat_map(|chunk| chunk.segments.iter())
        .map(|segment| segment.range().start().as_micros())
        .min()
}

/// Cuts `chunks` (one run, in plan order) after each of its chunks and checks
/// every cut against the merge of the whole run:
///
/// - the settled segments are exactly the first segments of the whole merge,
///   byte for byte (time, text, confidence, provider times, trim, chunk);
/// - the whole merge holds exactly those segments ending at or before the
///   settled time, and no other segment of it starts before that time;
/// - every settled unplaced stretch is one the whole merge reports;
/// - each cut settles at least what the cut before it did, and no earlier;
/// - the last cut settles the whole merge, its unplaced stretches included.
///
/// A cut whose later chunks start before the bound the cut relies on
/// ([`MAX_EARLY_AUDIO_MICROS`] before the next window) is a broken input, which
/// a run refuses; it is reported as an error, never skipped.
pub(super) fn every_cut_settles_a_prefix(chunks: &[ChunkSegments]) -> Result<CutCounts, String> {
    every_cut_settles_a_prefix_within(chunks, MAX_EARLY_AUDIO_MICROS)
}

/// [`every_cut_settles_a_prefix`] for a run that bounds its audio's early
/// start by `bound` microseconds (a tighter source, ADR 0033 section 8).
fn every_cut_settles_a_prefix_within(
    chunks: &[ChunkSegments],
    bound: u64,
) -> Result<CutCounts, String> {
    let whole = merge_chunks(chunks);
    let mut counts = CutCounts::default();
    let mut previous: Option<SettledTranscript> = None;
    for index in 0..=chunks.len() {
        let finished = chunks.get(..index).unwrap_or_default();
        let next = chunks.get(index).map(|chunk| &chunk.chunk);
        if let Some(next) = next {
            let reach = next.window().start().as_micros().saturating_sub(bound);
            if earliest_later_start(chunks, index).is_some_and(|start| start < reach) {
                return Err(format!(
                    "cut {index}: a later chunk starts before the reach {reach}"
                ));
            }
        }
        let settled = settled_merge(finished, next, bound);
        counts.cuts += 1;
        let count = settled.segments.len();
        if whole.segments.get(..count) != Some(settled.segments.as_slice()) {
            return Err(format!(
                "cut {index}: the {count} settled segments are not the first of the merge"
            ));
        }
        let until = settled.settled_until;
        let ending_by = whole
            .segments
            .iter()
            .filter(|segment| segment.segment.range().end() <= until)
            .count();
        if ending_by != count {
            return Err(format!(
                "cut {index}: the merge has {ending_by} segments ending by {}, {count} settled",
                until.as_micros()
            ));
        }
        if let Some(early) = whole
            .segments
            .get(count..)
            .unwrap_or_default()
            .iter()
            .find(|segment| segment.segment.range().start() < until)
        {
            return Err(format!(
                "cut {index}: an unsettled segment starts at {}, before {}",
                early.segment.range().start().as_micros(),
                until.as_micros()
            ));
        }
        if let Some(stretch) = settled
            .unplaced
            .iter()
            .find(|stretch| !whole.unplaced.contains(stretch))
        {
            return Err(format!(
                "cut {index}: unplaced {stretch:?} is not the merge's"
            ));
        }
        if let Some(before) = &previous
            && (before.settled_until > until || before.segments.len() > count)
        {
            return Err(format!("cut {index}: settles less than the cut before it"));
        }
        if next.is_none()
            && (settled.segments != whole.segments || settled.unplaced != whole.unplaced)
        {
            return Err("the last cut does not settle the whole merge".to_owned());
        }
        if count > 0 && count < whole.segments.len() {
            counts.partial += 1;
        }
        if let Some(next) = next {
            let reach = next.window().start().as_micros().saturating_sub(bound);
            if !finished.is_empty() && until.as_micros() < reach {
                counts.held_back += 1;
            }
        }
        previous = Some(settled);
    }
    Ok(counts)
}

/// The chunk plans the matrix cuts: R0, and smaller ones such as a live
/// source would use (ADR 0033 section 8 asks that the rule hold for them),
/// down to windows shorter than [`MAX_EARLY_AUDIO_MICROS`].
fn plans() -> Built<[ChunkPlan; 4]> {
    Ok([
        ChunkPlan::R0,
        ChunkPlan::new(10 * SECOND, 2 * SECOND)?,
        ChunkPlan::new(5 * SECOND, SECOND)?,
        ChunkPlan::new(2 * SECOND, 400_000)?,
    ])
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 2_000,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// ADR 0033, the claim the growing revision rests on: over generated runs
    /// of up to 24 chunks of R0 or a smaller plan, with segments hugging the
    /// window edges, the audio decoded early (by a frame boundary, or by as
    /// much as the bound allows), rejected segments of every kind, and words
    /// unique, from a vocabulary of two (so that the repeat rule acts), or
    /// looping in about half the segments (so that the loop rule of #539
    /// acts), **every cut of the run settles exactly a prefix of the whole
    /// run's merge**, with the recorded file's early-audio bound and, where the
    /// audio starts at most 40 ms early, with that tighter bound too.
    #[test]
    fn every_cut_of_a_run_settles_a_prefix_of_its_merge(
        plan in 0_usize..4,
        total_ms in 31_000_u64..=205_000,
        answers in prop::collection::vec(chunk_answer(), 24),
        from_ms in 0_u64..=20_000,
        early_ms in prop_oneof![0_u64..=40, 4_900_u64..=5_000],
        kind in 0_u8..3,
        seeds in prop::collection::vec(0_u8..16, 64),
    ) {
        let words = match kind {
            0 => Words::Unique,
            1 => Words::Repeated(&seeds),
            _ => Words::Looped(&seeds),
        };
        let Ok(plans) = plans() else {
            return Ok(());
        };
        let Some(plan) = plans.get(plan).copied() else {
            return Ok(());
        };
        let Ok(Some(layout)) =
            generated_with_plan(plan, from_ms, total_ms, &answers, early_ms, words)
        else {
            return Ok(());
        };
        prop_assert_eq!(every_cut_settles_a_prefix(&layout.chunks).map(|_| ()), Ok(()));
        // A source that starts its audio at most 40 ms early may say so, and
        // the cut then settles sooner, still a prefix.
        if early_ms <= 40 {
            prop_assert_eq!(
                every_cut_settles_a_prefix_within(&layout.chunks, 40_000).map(|_| ()),
                Ok(())
            );
        }
    }
}

/// The matrix is not vacuous: over a fixed pseudo-random sweep of the same
/// runs, for every plan, many cuts settle part of a run but not all of it,
/// and the stretch rule holds the settled time back before the reach in a real
/// share of them.
#[test]
fn the_cut_matrix_reaches_partial_and_held_back_cuts_for_every_plan() -> TestResult {
    let mut state = 0x5851_f42d_4c95_7f2d_u64;
    let mut next = |bound: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    for plan in plans()? {
        let mut total = CutCounts::default();
        for round in 0..1_500_u32 {
            let total_ms = 31_000 + next(174_000);
            let answers: Vec<ChunkAnswer> = (0..24)
                .map(|_| {
                    let delay =
                        [0, 300, 450, 500, 600, next(4_000)][usize::try_from(next(6)).unwrap_or(0)];
                    let shape = (0..next(7))
                        .map(|_| (200 + next(13_800), next(3_000)))
                        .collect();
                    let junk = (0..next(2)).map(|_| next(30_000)).collect();
                    (delay, shape, junk)
                })
                .collect();
            let seeds: Vec<u8> = (0..64)
                .map(|_| u8::try_from(next(16)).unwrap_or(0))
                .collect();
            let words = if round % 2 == 0 {
                Words::Unique
            } else {
                Words::Repeated(&seeds)
            };
            let early_ms = if round % 3 == 0 { 4_950 } else { next(41) };
            let Some(layout) = generated_with_plan(plan, 0, total_ms, &answers, early_ms, words)?
            else {
                continue;
            };
            let counts = every_cut_settles_a_prefix(&layout.chunks)?;
            total.cuts += counts.cuts;
            total.partial += counts.partial;
            total.held_back += counts.held_back;
        }
        assert!(total.cuts > 1_000, "{plan:?}: {total:?}");
        assert!(total.partial > 500, "{plan:?}: {total:?}");
        assert!(total.held_back > 100, "{plan:?}: {total:?}");
    }
    Ok(())
}

/// A sentence across a seam whose second chunk is not finished is not
/// settled: the first chunk's copy is held back until the second chunk is
/// done, because the merge may keep the second chunk's copy instead.
#[test]
fn a_sentence_across_an_unfinished_seam_is_held_back() -> TestResult {
    let first = with(
        draft(0, (0, 30 * SECOND), 2 * SECOND, 6 * SECOND, "hello there")?,
        draft(
            0,
            (0, 30 * SECOND),
            23 * SECOND,
            29_800_000,
            "the cut sentence",
        )?,
    );
    let second = draft(
        1,
        (25 * SECOND, 55 * SECOND),
        25_200_000,
        31 * SECOND,
        "the cut sentence whole",
    )?;
    let chunks = [first, second];
    let settled = settled_merge(&chunks[..1], Some(&chunks[1].chunk), MAX_EARLY_AUDIO_MICROS);
    // The reach is 20 s; the stretch of the cut sentence ends past it, so the
    // settled time is where that stretch starts, and only the first sentence
    // is settled.
    assert_eq!(settled.settled_until, MediaTime::from_micros(20 * SECOND));
    assert_eq!(
        settled
            .segments
            .iter()
            .map(|merged| merged.segment.text().text())
            .collect::<Vec<_>>(),
        ["hello there"]
    );
    let whole = merge_chunks(&chunks);
    assert_eq!(whole.segments.first(), settled.segments.first());
    assert_eq!(every_cut_settles_a_prefix(&chunks).map(|_| ()), Ok(()));
    Ok(())
}

/// A stretch that starts before the reach and runs past it holds the settled
/// time back to its start, even though it began in audio no later chunk hears.
#[test]
fn a_stretch_that_runs_past_the_reach_holds_the_settled_time_back() -> TestResult {
    let first = with(
        draft(0, (0, 30 * SECOND), SECOND, 4 * SECOND, "one")?,
        with(
            draft(0, (0, 30 * SECOND), 17 * SECOND, 19 * SECOND, "two")?,
            draft(0, (0, 30 * SECOND), 18 * SECOND, 22 * SECOND, "three")?,
        ),
    );
    let second = empty(1, (25 * SECOND, 55 * SECOND))?;
    let settled = settled_merge(
        std::slice::from_ref(&first),
        Some(&second.chunk),
        MAX_EARLY_AUDIO_MICROS,
    );
    assert_eq!(settled.settled_until, MediaTime::from_micros(17 * SECOND));
    assert_eq!(settled.segments.len(), 1);
    assert_eq!(
        every_cut_settles_a_prefix(&[first, second]).map(|_| ()),
        Ok(())
    );
    Ok(())
}

/// With nothing finished nothing is settled, at the reach of the first
/// window (before its start, clamped at zero); with everything finished the whole merge is, to the end of the last
/// window.
#[test]
fn the_first_and_last_cuts_settle_nothing_and_everything() -> Built<()> {
    let only = draft(0, (0, 12 * SECOND), SECOND, 3 * SECOND, "only")?;
    let before = settled_merge(
        &[],
        Some(&chunk(0, 0, 12 * SECOND)?),
        MAX_EARLY_AUDIO_MICROS,
    );
    assert_eq!(before.settled_until, MediaTime::from_micros(0));
    let later = settled_merge(
        &[],
        Some(&chunk(0, 60 * SECOND, 90 * SECOND)?),
        MAX_EARLY_AUDIO_MICROS,
    );
    assert_eq!(later.settled_until, MediaTime::from_micros(55 * SECOND));
    assert_eq!(later.segments, Vec::new());
    assert_eq!(before.segments, Vec::new());
    let after = settled_merge(std::slice::from_ref(&only), None, MAX_EARLY_AUDIO_MICROS);
    assert_eq!(after.settled_until, MediaTime::from_micros(12 * SECOND));
    assert_eq!(after.segments, merge_chunks(&[only]).segments);
    Ok(())
}

/// The digest of what the merge makes of a fixed sweep of runs: each kept
/// segment's chunk, times and text, and each run's unplaced stretches.
fn merge_rules_digest() -> Built<u64> {
    // FNV-1a, 64 bits: stable across Rust releases, unlike the standard
    // library's hasher.
    let mut digest = 0xcbf2_9ce4_8422_2325_u64;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            digest ^= u64::from(*byte);
            digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = |bound: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    for round in 0..400_u32 {
        let total_ms = 31_000 + next(99_000);
        let answers: Vec<ChunkAnswer> = (0..5)
            .map(|_| {
                let delay =
                    [0, 300, 450, 500, 600, next(4_000)][usize::try_from(next(6)).unwrap_or(0)];
                let shape = (0..next(7))
                    .map(|_| (200 + next(13_800), next(3_000)))
                    .collect();
                let junk = (0..next(3)).map(|_| next(30_000)).collect();
                (delay, shape, junk)
            })
            .collect();
        let seeds: Vec<u8> = (0..40)
            .map(|_| u8::try_from(next(16)).unwrap_or(0))
            .collect();
        let words = match round % 3 {
            0 => Words::Unique,
            1 => Words::Repeated(&seeds),
            _ => Words::Looped(&seeds),
        };
        let Some(layout) = generated_with_plan(ChunkPlan::R0, 0, total_ms, &answers, 0, words)?
        else {
            continue;
        };
        let merged = merge_chunks(&layout.chunks);
        for segment in &merged.segments {
            feed(&segment.chunk.to_le_bytes());
            feed(&segment.segment.range().start().as_micros().to_le_bytes());
            feed(&segment.segment.range().end().as_micros().to_le_bytes());
            feed(segment.segment.text().text().as_bytes());
            feed(&[0]);
        }
        for stretch in &merged.unplaced {
            feed(&stretch.start().as_micros().to_le_bytes());
            feed(&stretch.end().as_micros().to_le_bytes());
        }
        feed(&[0xff]);
    }
    Ok(digest)
}

/// The merge rules version of a run's open revision (ADR 0033) names what the
/// merge makes. **When this fails because the merge rules changed, bump
/// `MERGE_RULES_VERSION` and record the new digest here together**: an open
/// revision published under the old rules is then sealed as stopped by a
/// resume under the new ones instead of being extended with text the old rules
/// would not have settled.
#[test]
fn the_merge_rules_version_names_what_the_merge_makes() -> TestResult {
    const VERSION_2_DIGEST: u64 = 9_263_716_986_213_698_812;
    assert_eq!(crate::asr::MERGE_RULES_VERSION, 2);
    assert_eq!(
        merge_rules_digest()?,
        VERSION_2_DIGEST,
        "the merge rules changed: bump MERGE_RULES_VERSION and record the new digest"
    );
    Ok(())
}

/// D5: a first window on the whole run's chunk grid is planned in chunks with
/// exactly the whole run's windows, wherever the window falls, so the whole run
/// can take every one of their answers.
#[test]
fn a_window_aligned_to_the_chunk_grid_plans_the_whole_runs_own_chunks() -> TestResult {
    let segment = crate::SourceSegmentId::parse("sgm_0123456789abcdef")?;
    let at = |seconds: u64| MediaTime::from_micros(seconds * SECOND);
    for source_s in [61_u64, 75, 140, 600, 3_601] {
        let source_end = at(source_s);
        let whole = crate::asr::plan_chunks(
            &segment,
            crate::TimeRange::new(at(0), source_end)?,
            ChunkPlan::R0,
        )?;
        for from_s in (0..source_s).step_by(7) {
            for length_s in [1_u64, 4, 25, 31, 90] {
                let window =
                    crate::TimeRange::new(at(from_s), at((from_s + length_s).min(source_s)))?;
                let aligned = crate::asr::align_to_chunk_grid(window, ChunkPlan::R0, source_end)
                    .ok_or("a window inside the source has a grid range")?;
                assert!(aligned.start() <= window.start() && aligned.end() >= window.end());
                for chunk in crate::asr::plan_chunks(&segment, aligned, ChunkPlan::R0)? {
                    assert!(
                        whole.iter().any(|own| own.window() == chunk.window()),
                        "{source_s} s source, window {from_s}+{length_s} s: {:?} is not a chunk                          of the whole run",
                        chunk.window()
                    );
                }
            }
        }
    }
    // A window at or after the end of the source has none.
    let past = crate::TimeRange::new(at(61), at(62))?;
    assert_eq!(
        crate::asr::align_to_chunk_grid(past, ChunkPlan::R0, at(60)),
        None
    );
    Ok(())
}
