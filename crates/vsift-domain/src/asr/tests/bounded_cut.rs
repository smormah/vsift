//! L-256: the bounded cut settles part of a long stretch of continuous speech,
//! and what it settles is still a prefix of the whole run's merge (ADR 0033
//! section 2, the amendment of 2026-11-11).
//!
//! The runs here are continuous speech: segments of one to six seconds that
//! touch or leave a gap shorter than the guard's half second, across every
//! chunk, so that the finished chunks' union is one long stretch and the
//! stretch cut alone would settle nothing inside it. Their segment edges are
//! snapped, often, onto the points the bounded cut compares against: the
//! reach (for the recorded file's bound and the tight one), half a second
//! before a window's end, and half a second before another edge.

use proptest::prelude::{
    Just, ProptestConfig, Strategy, prop, prop_assert_eq, prop_oneof, proptest,
};

use super::settled::{
    Cut, CutCounts, every_cut_settles_a_prefix, every_cut_settles_a_prefix_by,
    every_cut_settles_a_prefix_within, plans,
};
use super::{
    Built, ChunkAnswer, SECOND, TestResult, Words, chunk, generated_with_plan, output,
    provider_segment, range, segment_id, with,
};
use crate::{
    MediaTime,
    asr::{
        ChunkPlan, ChunkSegments, MAX_EARLY_AUDIO_MICROS, PlannedChunk,
        SEAM_COVERAGE_TOLERANCE_MICROS, SEAM_EDGE_MICROS, SettledTranscript, plan_chunks,
        settled::{CutMargins, settled_merge_within},
        settled_merge, validate_chunk_output,
    },
};

/// The longest segment the continuous runs make, in milliseconds.
const LONGEST_SEGMENT_MS: u64 = 6_000;

/// The tight early-audio bound the matrix also cuts with (ADR 0033 section 8),
/// in milliseconds.
const TIGHT_BOUND_MS: u64 = 40;

/// How many snap codes [`continuous_answer`] draws from: six kinds (none, the
/// reach, the tight reach, the window's edge margin, the tolerance, none) by
/// four nudges (exact, exact, a millisecond early, a millisecond late).
const SNAP_CODES: u8 = 24;

/// One chunk's answer in a run of continuous speech: the delay before its
/// first segment, then for each segment its length, the gap after it and a
/// snap code (see [`snapped`]), all in milliseconds; and the start of each
/// rejected segment with words, as [`ChunkAnswer`] has them.
type ContinuousAnswer = (u64, Vec<(u64, u64, u8)>, Vec<u64>);

fn continuous_answer() -> impl Strategy<Value = ContinuousAnswer> {
    (
        prop_oneof![Just(0_u64), 0_u64..=300],
        prop::collection::vec(
            (
                1_000_u64..=LONGEST_SEGMENT_MS,
                prop_oneof![Just(0_u64), 1_u64..500],
                0_u8..SNAP_CODES,
            ),
            40,
        ),
        prop::collection::vec(
            prop_oneof![Just(1_u64), Just(2), Just(5), 0_u64..30_000],
            0..2,
        ),
    )
}

/// The plain answers that place the continuous answers' segments, with each
/// segment whose snap code names a point it can reach (no longer than
/// [`LONGEST_SEGMENT_MS`]) ending exactly there, or a millisecond either side:
/// the next window's start less the recorded file's early-audio bound or less
/// [`TIGHT_BOUND_MS`] (the reach the cut of this chunk compares against), its
/// own window's end less the edge margin, or (code 4) the tolerance after its
/// start, so that the next edge lies exactly the tolerance after this one.
fn snapped(
    plan: ChunkPlan,
    from_ms: u64,
    total_ms: u64,
    answers: &[ContinuousAnswer],
    early_ms: u64,
) -> Built<Vec<ChunkAnswer>> {
    let chunks = plan_chunks(
        &segment_id()?,
        range(from_ms * 1_000, total_ms * 1_000)?,
        plan,
    )?;
    let ms = |time: MediaTime| time.as_micros() / 1_000;
    let mut placed = Vec::with_capacity(answers.len());
    for (index, (planned_chunk, (delay, shape, junk))) in chunks.iter().zip(answers).enumerate() {
        let window = planned_chunk.window();
        let heard_from = ms(window.start()).saturating_sub(early_ms);
        let next_start = chunks.get(index + 1).map(|next| ms(next.window().start()));
        let mut cursor = *delay;
        let mut segments = Vec::with_capacity(shape.len());
        for &(length, gap, code) in shape {
            let point = match code % 6 {
                1 => next_start.map(|start| start.saturating_sub(MAX_EARLY_AUDIO_MICROS / 1_000)),
                2 => next_start.map(|start| start.saturating_sub(TIGHT_BOUND_MS)),
                3 => Some(ms(window.end()).saturating_sub(SEAM_EDGE_MICROS / 1_000)),
                _ => None,
            };
            let wanted = if code % 6 == 4 {
                Some(SEAM_COVERAGE_TOLERANCE_MICROS / 1_000)
            } else {
                point.and_then(|point| point.checked_sub(heard_from + cursor))
            };
            let nudged = wanted.and_then(|wanted| match code / 6 {
                2 => wanted.checked_sub(1),
                3 => Some(wanted + 1),
                _ => Some(wanted),
            });
            let length = nudged
                .filter(|wanted| (1..=LONGEST_SEGMENT_MS).contains(wanted))
                .unwrap_or(length);
            segments.push((length, gap));
            cursor += length + gap;
        }
        placed.push((*delay, segments, junk.clone()));
    }
    Ok(placed)
}

/// The merge input of a continuous run, or `None` when a chunk's answer is
/// unusable under the output rules.
fn continuous_run(
    plan: ChunkPlan,
    from_ms: u64,
    total_ms: u64,
    answers: &[ContinuousAnswer],
    early_ms: u64,
    words: Words<'_>,
) -> Built<Option<Vec<ChunkSegments>>> {
    let answers = snapped(plan, from_ms, total_ms, answers, early_ms)?;
    Ok(
        generated_with_plan(plan, from_ms, total_ms, &answers, early_ms, words)?
            .map(|generated| generated.chunks),
    )
}

/// L-256's regression, for the R0 plan and the recorded file's bound: from the
/// second finished chunk on, the settled part is not empty, and the settled
/// time trails the end of the finished audio by at most the reach's distance
/// from it (10 s with R0) plus 12.5 s, two of the generator's longest segments
/// and the tolerance.
///
/// Why, for these runs only (segments up to [`LONGEST_SEGMENT_MS`], gaps under
/// the tolerance): between the end of the previous window and the reach, only
/// the last finished chunk heard anything, its segments there are its own and
/// away from its edges, so rules 1 and 2 keep them. Where they leave no gap,
/// the latest edge before the reach that one of them ends at is at most one
/// segment back, and the latest edge more than the tolerance before that at
/// most one segment and the tolerance further; where they leave a gap, the
/// stretch cut is after it. A single segment as long as a window would hold its
/// stretch back, so this is no bound of the contract.
fn trails_by_at_most_two_segments(chunks: &[ChunkSegments]) -> Result<(), String> {
    let slack = 2 * LONGEST_SEGMENT_MS * 1_000 + SEAM_COVERAGE_TOLERANCE_MICROS;
    for index in 2..chunks.len() {
        let (Some(finished), Some(next)) = (chunks.get(..index), chunks.get(index)) else {
            continue;
        };
        let settled = settled_merge(finished, Some(&next.chunk), MAX_EARLY_AUDIO_MICROS);
        let finished_until = finished
            .last()
            .map_or(0, |last| last.chunk.window().end().as_micros());
        let reach = next
            .chunk
            .window()
            .start()
            .as_micros()
            .saturating_sub(MAX_EARLY_AUDIO_MICROS);
        let trail = finished_until.saturating_sub(settled.settled_until.as_micros());
        if settled.segments.is_empty() {
            return Err(format!("cut {index}: nothing settled"));
        }
        if trail > finished_until.saturating_sub(reach) + slack {
            return Err(format!(
                "cut {index}: settled until {}, {trail} us behind the finished audio",
                settled.settled_until.as_micros()
            ));
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 2_000,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// ADR 0033 section 2, amended for L-256: over generated runs of
    /// continuous speech, up to 24 chunks of R0 or a smaller plan, with
    /// segment edges snapped onto the points the cut compares against, the
    /// audio decoded early (by a frame boundary, by as much as the bound
    /// allows, or by exactly the bound), rejected segments of every kind, and
    /// words unique, repeated or looping, **every cut of the run settles
    /// exactly a prefix of the whole run's merge**, with the recorded file's
    /// bound and, where the audio starts at most 40 ms early, with that bound
    /// too. With R0, the settled part also keeps up with the speech
    /// ([`trails_by_at_most_two_segments`]).
    #[test]
    fn every_cut_of_a_continuous_run_settles_a_prefix_of_its_merge(
        plan in 0_usize..4,
        total_ms in 31_000_u64..=205_000,
        answers in prop::collection::vec(continuous_answer(), 24),
        from_ms in 0_u64..=20_000,
        early_ms in prop_oneof![0_u64..=40, 4_900_u64..=5_000, Just(5_000)],
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
        let Ok(Some(chunks)) = continuous_run(plan, from_ms, total_ms, &answers, early_ms, words)
        else {
            return Ok(());
        };
        prop_assert_eq!(every_cut_settles_a_prefix(&chunks).map(|_| ()), Ok(()));
        if early_ms <= TIGHT_BOUND_MS {
            prop_assert_eq!(
                every_cut_settles_a_prefix_within(&chunks, TIGHT_BOUND_MS * 1_000).map(|_| ()),
                Ok(())
            );
        }
        if plan == ChunkPlan::R0 {
            prop_assert_eq!(trails_by_at_most_two_segments(&chunks), Ok(()));
        }
    }
}

/// A fixed pseudo-random sweep of continuous runs of `plan`, each cut with the
/// early-audio `bound` (audio decoded early by up to the bound) and with `cut`:
/// the counts over the runs, or the first counterexample.
fn continuous_sweep(plan: ChunkPlan, bound: u64, cut: Cut<'_>) -> Built<Result<CutCounts, String>> {
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = |bound: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    let mut total = CutCounts::default();
    for round in 0..300_u32 {
        let total_ms = 31_000 + next(174_000);
        let answers: Vec<ContinuousAnswer> = (0..24)
            .map(|_| {
                let delay = if next(2) == 0 { 0 } else { next(301) };
                let shape = (0..40)
                    .map(|_| {
                        let gap = if next(2) == 0 { 0 } else { 1 + next(499) };
                        let code = u8::try_from(next(u64::from(SNAP_CODES))).unwrap_or(0);
                        (1_000 + next(LONGEST_SEGMENT_MS - 999), gap, code)
                    })
                    .collect();
                let junk = (0..next(2)).map(|_| next(30_000)).collect();
                (delay, shape, junk)
            })
            .collect();
        let seeds: Vec<u8> = (0..64)
            .map(|_| u8::try_from(next(16)).unwrap_or(0))
            .collect();
        let words = match round % 3 {
            0 => Words::Unique,
            1 => Words::Repeated(&seeds),
            _ => Words::Looped(&seeds),
        };
        let early_ms = match round % 3 {
            0 => bound / 1_000,
            1 => next(bound / 1_000 + 1),
            _ => next(41).min(bound / 1_000),
        };
        let Some(chunks) = continuous_run(plan, 0, total_ms, &answers, early_ms, words)? else {
            continue;
        };
        match every_cut_settles_a_prefix_by(&chunks, bound, cut) {
            Ok(counts) => {
                total.cuts += counts.cuts;
                total.partial += counts.partial;
                total.held_back += counts.held_back;
                total.inside_stretch += counts.inside_stretch;
            }
            Err(found) => return Ok(Err(format!("round {round}: {found}"))),
        }
    }
    Ok(Ok(total))
}

/// The bounded cut is not vacuous: over the continuous sweep, for every plan
/// and both bounds, many cuts settle inside a stretch that runs to the reach,
/// where the stretch cut alone settles nothing.
#[test]
fn the_continuous_matrix_settles_inside_stretches_for_every_plan() -> TestResult {
    for plan in plans()? {
        for bound in [MAX_EARLY_AUDIO_MICROS, TIGHT_BOUND_MS * 1_000] {
            let counts = continuous_sweep(plan, bound, &settled_merge)??;
            assert!(counts.cuts > 1_000, "{plan:?}, {bound}: {counts:?}");
            assert!(counts.partial > 500, "{plan:?}, {bound}: {counts:?}");
            assert!(counts.inside_stretch > 200, "{plan:?}, {bound}: {counts:?}");
        }
    }
    Ok(())
}

/// The cut with `margins`, for the sweep.
fn with_margins(
    margins: CutMargins,
) -> impl Fn(&[ChunkSegments], Option<&PlannedChunk>, u64) -> SettledTranscript {
    move |finished, next, bound| settled_merge_within(finished, next, bound, margins)
}

/// The matrix catches a wrong cut: a bounded cut allowed to look a second
/// past the reach, where a later chunk's segments may start, settles what the
/// whole run's merge changes, and the continuous sweep finds it.
///
/// The cut's other conditions are kept although no sweep breaks them. Two are
/// redundant safety margins, kept on purpose (ADR 0033, amendment of
/// 2026-11-11): **the edge term** of `λ`, because every segment a later chunk
/// can change ends after the reach itself (rule 1 needs a midpoint past the
/// next window's start, rule 2's cover a neighbour that starts half a second
/// after it, a replacement a midpoint after the reach); and **rule 3's margin**,
/// because the kept segment that ends at `T` starts at or after the cut, so it
/// is still there when rule 3 judges a settled segment, whose hole test then
/// stops before `T`. The two structural conditions (no finished segment
/// crosses `T`; a segment kept by rules 1 and 2 ends there) are what the proof
/// uses; the same sweep with either dropped found no counterexample, and they
/// stay.
#[test]
fn a_bounded_cut_allowed_past_the_reach_settles_what_the_seal_changes() -> TestResult {
    let past_the_reach = with_margins(CutMargins {
        past_the_reach: SECOND,
        ..CutMargins::MERGE
    });
    let mut found = Vec::new();
    for plan in plans()? {
        for bound in [MAX_EARLY_AUDIO_MICROS, TIGHT_BOUND_MS * 1_000] {
            if continuous_sweep(plan, bound, &past_the_reach)?.is_err() {
                found.push((plan, bound));
            }
        }
    }
    assert!(found.len() >= 4, "too few counterexamples: {found:?}");
    Ok(())
}

// ------------------------------------------------- boundaries, by hand

/// A chunk `index` over `window` (seconds as microseconds) whose audio was
/// decoded from `heard_from`, with one segment for each `(start, end, words)`.
fn heard(
    index: u32,
    window: (u64, u64),
    heard_from: u64,
    segments: &[(u64, u64, &str)],
) -> Built<ChunkSegments> {
    let planned = chunk(index, window.0, window.1)?;
    let audio = range(heard_from, heard_from + (window.1 - window.0))?;
    let provider = segments
        .iter()
        .map(|&(start, end, words)| {
            provider_segment(
                (start - heard_from) / 1_000,
                (end - heard_from) / 1_000,
                words,
            )
        })
        .collect::<Built<Vec<_>>>()?;
    let validated =
        validate_chunk_output(&planned, audio, range(0, 600 * SECOND)?, output(provider))?;
    Ok(validated.into_parts().0)
}

/// Milliseconds as microseconds.
const fn ms(value: u64) -> u64 {
    value * 1_000
}

/// The settled time of the first cut of `chunks` (after their first chunk),
/// after checking every cut of them.
fn first_cut(chunks: &[ChunkSegments], bound: u64) -> Built<SettledTranscript> {
    assert_eq!(
        every_cut_settles_a_prefix_within(chunks, bound).map(|_| ()),
        Ok(())
    );
    let (Some(first), Some(second)) = (chunks.first(), chunks.get(1)) else {
        return Err("two chunks are needed".into());
    };
    Ok(settled_merge(
        std::slice::from_ref(first),
        Some(&second.chunk),
        bound,
    ))
}

/// Touching segments of continuous speech, `(start, end)` in milliseconds.
fn speech(edges: &[(u64, u64)]) -> Vec<(u64, u64, String)> {
    edges
        .iter()
        .enumerate()
        .map(|(number, &(start, end))| (ms(start), ms(end), format!("words {number}")))
        .collect()
}

fn as_borrowed(segments: &[(u64, u64, String)]) -> Vec<(u64, u64, &str)> {
    segments
        .iter()
        .map(|(start, end, words)| (*start, *end, words.as_str()))
        .collect()
}

/// A run of two R0 chunks, the first speaking continuously over `edges`, the
/// second from its window's start to past the first's end.
fn two_chunks(edges: &[(u64, u64)]) -> Built<[ChunkSegments; 2]> {
    let first = speech(edges);
    Ok([
        heard(0, (0, 30 * SECOND), 0, &as_borrowed(&first))?,
        heard(
            1,
            (25 * SECOND, 55 * SECOND),
            25 * SECOND,
            &[(ms(25_000), ms(33_000), "later words")],
        )?,
    ])
}

/// The bounded cut settles inside a stretch: a talk from 1 s to 29 s, one
/// stretch past the reach (20 s), is settled to an edge before it, where the
/// stretch cut alone settled nothing.
#[test]
fn a_continuous_talk_is_settled_inside_its_stretch() -> TestResult {
    let chunks = two_chunks(&[
        (1_000, 6_000),
        (6_000, 12_000),
        (12_000, 18_000),
        (18_000, 24_000),
        (24_000, 29_000),
    ])?;
    let settled = first_cut(&chunks, MAX_EARLY_AUDIO_MICROS)?;
    // T is 18 s, the last edge before the reach; the cut is the last edge
    // more than the tolerance before it.
    assert_eq!(settled.settled_until, MediaTime::from_micros(12 * SECOND));
    assert_eq!(settled.segments.len(), 2);
    Ok(())
}

/// Boundary: a kept segment that ends **exactly at the reach** (here also
/// `λ`) is not taken as T, although rules 1 and 2 could not change it; the
/// edge before it is.
#[test]
fn a_segment_ending_exactly_at_the_reach_is_held_back() -> TestResult {
    let chunks = two_chunks(&[
        (1_000, 5_000),
        (5_000, 10_000),
        (10_000, 15_000),
        (15_000, 20_000),
        (20_000, 29_000),
    ])?;
    let settled = first_cut(&chunks, MAX_EARLY_AUDIO_MICROS)?;
    // T is 15 s, not 20 s; the cut is 10 s, not 15 s.
    assert_eq!(settled.settled_until, MediaTime::from_micros(10 * SECOND));
    Ok(())
}

/// Boundary: with a bound tighter than the edge margin, `λ` is the window's
/// end less the edge margin, and a segment that ends **exactly there** is not
/// taken as T (rule 2's test is `end + margin > window end`, so it is not
/// cut, but the cut does not rely on the equality).
#[test]
fn a_segment_ending_exactly_at_the_edge_margin_is_held_back() -> TestResult {
    let first = speech(&[
        (1_000, 10_000),
        (10_000, 20_000),
        (20_000, 25_000),
        (25_000, 29_500),
        (29_500, 30_000),
    ]);
    let chunks = [
        heard(0, (0, 30 * SECOND), 0, &as_borrowed(&first))?,
        heard(
            1,
            (29_600_000, 59_600_000),
            29_600_000,
            &[(ms(29_600), ms(35_000), "later words")],
        )?,
    ];
    // The reach is 29.56 s, λ 29.5 s: T is 25 s, not 29.5 s, and the cut 20 s.
    let settled = first_cut(&chunks, TIGHT_BOUND_MS * 1_000)?;
    assert_eq!(settled.settled_until, MediaTime::from_micros(20 * SECOND));
    Ok(())
}

/// Boundary: a later chunk's segment that starts **exactly at the reach**
/// (its audio decoded the whole bound early) touches nothing the cut settled.
#[test]
fn a_later_segment_starting_exactly_at_the_reach_reaches_nothing_settled() -> TestResult {
    let first = speech(&[
        (1_000, 8_000),
        (8_000, 14_000),
        (14_000, 20_000),
        (20_000, 29_800),
    ]);
    let chunks = [
        heard(0, (0, 30 * SECOND), 0, &as_borrowed(&first))?,
        heard(
            1,
            (25 * SECOND, 55 * SECOND),
            20 * SECOND,
            &[
                (ms(20_000), ms(26_000), "the same words"),
                (ms(26_000), ms(32_000), "later words"),
            ],
        )?,
    ];
    let settled = first_cut(&chunks, MAX_EARLY_AUDIO_MICROS)?;
    // T is 14 s (20 s is the reach itself), the cut 8 s.
    assert_eq!(settled.settled_until, MediaTime::from_micros(8 * SECOND));
    Ok(())
}

/// Boundary: an edge **exactly the tolerance before T** is not the cut; the
/// cut must be more than the tolerance before T, so it is the edge before.
#[test]
fn an_edge_exactly_the_tolerance_before_t_is_not_the_cut() -> TestResult {
    let chunks = two_chunks(&[
        (1_000, 5_000),
        (5_000, 14_500),
        (14_500, 15_000),
        (15_000, 22_000),
        (22_000, 29_000),
    ])?;
    let settled = first_cut(&chunks, MAX_EARLY_AUDIO_MICROS)?;
    // T is 15 s; 14.5 s is exactly the tolerance before it, so the cut is 5 s.
    assert_eq!(settled.settled_until, MediaTime::from_micros(5 * SECOND));
    Ok(())
}

/// Boundary: segments that **touch** at a point leave it a possible T, and a
/// segment that **crosses** it does not.
#[test]
fn a_touched_point_can_be_t_and_a_crossed_one_cannot() -> TestResult {
    let edges = [
        (1_000, 5_000),
        (5_000, 10_000),
        (10_000, 15_000),
        (15_000, 22_000),
        (22_000, 29_000),
    ];
    let touching = two_chunks(&edges)?;
    assert_eq!(
        first_cut(&touching, MAX_EARLY_AUDIO_MICROS)?.settled_until,
        MediaTime::from_micros(10 * SECOND),
        "T is 15 s, where two segments touch"
    );
    let [first, second] = two_chunks(&edges)?;
    let crossed = [
        with(
            first,
            heard(
                0,
                (0, 30 * SECOND),
                0,
                &[(ms(14_000), ms(16_000), "crossing words")],
            )?,
        ),
        second,
    ];
    assert_eq!(
        first_cut(&crossed, MAX_EARLY_AUDIO_MICROS)?.settled_until,
        MediaTime::from_micros(5 * SECOND),
        "a segment crosses 15 s, so T is 10 s"
    );
    Ok(())
}
