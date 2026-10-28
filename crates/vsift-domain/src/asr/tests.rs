//! Chunk planning, provider-output validation, silence and seam-merge rules.

use std::{
    collections::BTreeSet,
    num::{NonZeroU16, NonZeroU32},
};

use proptest::prelude::{
    Just, ProptestConfig, Strategy, prop, prop_assert, prop_assert_eq, prop_oneof, proptest,
};

use super::previous_merge::previous_merge_chunks;
use super::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, ChunkPlan, ChunkPlanError, ChunkSegments, ChunkTime,
    EarlierTextRule, MIN_RECOGNITION_MICROS, MIN_RECOGNITION_SAMPLES,
    MIN_SEGMENTS_FOR_REJECTION_RATIO, PlannedChunk, ProviderChunkOutput, ProviderOutputError,
    ProviderSegment, ProviderToken, ProviderTokenKind, ReviewedAsrModel,
    SEAM_COVERAGE_TOLERANCE_MICROS, SPEECH_SAMPLE_RATE, Sha256Hex,
    UNUSABLE_CHUNK_SHARE_DENOMINATOR, decoded_audio_range, is_below_recognition_floor,
    is_silent_pcm, merge_chunks, plan_chunks, rounds_to_no_pcm_sample, unusable_chunks_end_the_run,
    validate_chunk_output,
};
use crate::{
    CarriedFrom, Confidence, ConfidenceOrigin, CueSource, CueText, CueTiming, InheritedRevision,
    LanguageTag, MAX_UNPLACED_STRETCHES, MediaTime, ProviderEndTrim, SegmentOrigin,
    SidecarIdentity, SourceId, SourceSegment, SourceSegmentId, SpeakerLabel, TimeRange,
    TranscriptFormat, TranscriptOffset, TranscriptProvenance, TranscriptRevision,
    TranscriptRevisionError, TranscriptRevisionId, TranscriptRevisionParts, TranscriptSegment,
    TranscriptSegmentId, TranscriptSegmentParts, TranscriptWarningKind, TranscriptWarnings,
    spans::{SpanSet, merged, span},
};

mod settled;

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const SECOND: u64 = 1_000_000;

fn range(start: u64, end: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(start),
        MediaTime::from_micros(end),
    )?)
}

fn segment_id() -> Built<SourceSegmentId> {
    Ok(SourceSegmentId::parse("sgm_0123456789abcdef")?)
}

fn chunk(index: u32, start: u64, end: u64) -> Built<PlannedChunk> {
    Ok(PlannedChunk::new(segment_id()?, index, range(start, end)?))
}

fn text(value: &str) -> Built<CueText> {
    Ok(CueText::new(value.to_owned(), value.to_owned())?)
}

fn token(kind: ProviderTokenKind, probability: f64) -> ProviderToken {
    ProviderToken { kind, probability }
}

fn provider_segment(start_ms: u64, end_ms: u64, words: &str) -> Built<ProviderSegment> {
    Ok(ProviderSegment {
        start: ChunkTime::from_millis(start_ms).ok_or("time")?,
        end: ChunkTime::from_millis(end_ms).ok_or("time")?,
        text: Some(text(words)?),
        tokens: vec![
            token(ProviderTokenKind::Special, 0.5),
            token(ProviderTokenKind::Text, 0.9),
            token(ProviderTokenKind::Text, 0.7),
            token(ProviderTokenKind::Special, 0.01),
        ],
    })
}

fn output(segments: Vec<ProviderSegment>) -> ProviderChunkOutput {
    ProviderChunkOutput {
        language: None,
        segments,
    }
}

/// A validated segment of `chunk_index` over `[start, end)` source seconds.
fn draft(
    chunk_index: u32,
    window: (u64, u64),
    start: u64,
    end: u64,
    words: &str,
) -> Built<ChunkSegments> {
    let planned = chunk(chunk_index, window.0, window.1)?;
    let audio = planned.window();
    let relative_start = (start - window.0) / 1_000;
    let relative_end = (end - window.0) / 1_000;
    let validated = validate_chunk_output(
        &planned,
        audio,
        range(0, 600 * SECOND)?,
        output(vec![provider_segment(relative_start, relative_end, words)?]),
    )?;
    Ok(validated.into_parts().0)
}

fn with(mut base: ChunkSegments, extra: ChunkSegments) -> ChunkSegments {
    base.segments.extend(extra.segments);
    base
}

fn empty(chunk_index: u32, window: (u64, u64)) -> Built<ChunkSegments> {
    Ok(ChunkSegments {
        chunk: chunk(chunk_index, window.0, window.1)?,
        segments: Vec::new(),
        unplaced: Vec::new(),
    })
}

fn texts(chunks: &[ChunkSegments]) -> (Vec<String>, u32) {
    // ADR 0033: every merge these tests make is also cut after each of its
    // chunks, and each cut must settle a prefix of this very merge.
    assert_eq!(
        settled::every_cut_settles_a_prefix(chunks).map(|_| ()),
        Ok(())
    );
    let merged = merge_chunks(chunks);
    let duplicates = merged
        .warnings
        .as_slice()
        .iter()
        .find(|warning| warning.kind() == TranscriptWarningKind::SeamDuplicatesRemoved)
        .map_or(0, |warning| warning.count());
    (
        merged
            .segments
            .iter()
            .map(|merged| merged.segment.text().text().to_owned())
            .collect(),
        duplicates,
    )
}

#[test]
fn chunk_plans_reject_windows_without_a_core() {
    assert_eq!(ChunkPlan::new(0, 0), Err(ChunkPlanError::InvalidWindow));
    assert_eq!(
        ChunkPlan::new(31 * SECOND, SECOND),
        Err(ChunkPlanError::InvalidWindow)
    );
    assert_eq!(
        ChunkPlan::new(10 * SECOND, 5 * SECOND),
        Err(ChunkPlanError::InvalidOverlap)
    );
    assert_eq!(ChunkPlan::new(30 * SECOND, 5 * SECOND), Ok(ChunkPlan::R0));
}

#[test]
fn r0_plans_thirty_second_windows_five_seconds_apart_in_overlap() -> TestResult {
    let chunks = plan_chunks(&segment_id()?, range(0, 70 * SECOND)?, ChunkPlan::R0)?;
    let windows: Vec<(u64, u64)> = chunks
        .iter()
        .map(|chunk| {
            (
                chunk.window().start().as_micros() / SECOND,
                chunk.window().end().as_micros() / SECOND,
            )
        })
        .collect();
    assert_eq!(windows, [(0, 30), (25, 55), (50, 70)]);
    assert_eq!(
        chunks.iter().map(PlannedChunk::index).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    let short = plan_chunks(
        &segment_id()?,
        range(4 * SECOND, 9 * SECOND)?,
        ChunkPlan::R0,
    )?;
    assert_eq!(short.len(), 1);
    assert_eq!(short[0].window(), range(4 * SECOND, 9 * SECOND)?);
    Ok(())
}

proptest! {
    /// Chunks cover the range exactly, respect the window and overlap, and
    /// the same inputs always give the same plan.
    #[test]
    fn chunk_plans_cover_ranges_exactly_and_deterministically(
        start in 0_u64..10_000_000_000,
        length in 1_u64..3_600_000_000,
        window in 1_000_u64..=30_000_000,
        overlap_share in 0_u64..499,
    ) {
        let overlap = window * overlap_share / 1_000;
        let plan = ChunkPlan::new(window, overlap);
        prop_assert!(plan.is_ok());
        let (Ok(plan), Ok(requested), Ok(id)) =
            (plan, range(start, start + length), segment_id())
        else {
            return Ok(());
        };
        match plan_chunks(&id, requested, plan) {
            Ok(chunks) => {
                prop_assert_eq!(chunks.first().map(|chunk| chunk.window().start()), Some(requested.start()));
                prop_assert_eq!(chunks.last().map(|chunk| chunk.window().end()), Some(requested.end()));
                for (index, chunk) in chunks.iter().enumerate() {
                    prop_assert_eq!(usize::try_from(chunk.index()).ok(), Some(index));
                    prop_assert!(chunk.window().duration_micros() <= window);
                    prop_assert!(chunk.window().duration_micros() > 0);
                }
                for pair in chunks.windows(2) {
                    // No gap, and exactly the planned overlap between neighbours.
                    prop_assert!(pair[1].window().start() <= pair[0].window().end());
                    prop_assert_eq!(
                        pair[0].window().end().as_micros() - pair[1].window().start().as_micros(),
                        overlap
                    );
                    prop_assert_eq!(pair[0].window().duration_micros(), window);
                }
                // A window after the first is longer than the overlap: the
                // window before it did not reach the end of the range.
                for later in chunks.iter().skip(1) {
                    prop_assert!(later.window().duration_micros() > overlap);
                }
                prop_assert_eq!(plan_chunks(&id, requested, plan).ok(), Some(chunks));
            }
            Err(error) => prop_assert_eq!(error, ChunkPlanError::TooManyChunks),
        }
    }
}

#[test]
fn decoded_ranges_follow_the_sample_count() -> TestResult {
    assert_eq!(
        decoded_audio_range(MediaTime::from_micros(750_000), 16_000),
        Some(range(750_000, 1_750_000)?)
    );
    assert_eq!(decoded_audio_range(MediaTime::from_micros(0), 0), None);
    Ok(())
}

/// T-06: provider times are validated against the chunk's decoded audio.
#[test]
fn provider_segments_outside_their_audio_are_rejected_or_trimmed() -> TestResult {
    let planned = chunk(2, 50 * SECOND, 70 * SECOND)?;
    // Audio decoded from 50.1 s for ten seconds.
    let audio = range(50_100_000, 60_100_000)?;
    let source = range(0, 70 * SECOND)?;
    let segments = vec![
        provider_segment(0, 2_000, "kept as reported")?,
        provider_segment(2_500, 2_500, "empty range")?,
        provider_segment(3_000, 9_000, "also kept")?,
        provider_segment(9_000, 10_600, "ends just past the audio")?,
        provider_segment(9_500, 11_200, "ends further past the audio")?,
        provider_segment(9_600, 10_000, "ends at the audio end")?,
        provider_segment(9_700, 10_000, "one more kept")?,
        provider_segment(9_800, 10_000, "and another")?,
        provider_segment(9_900, 10_000, "and a last one")?,
    ];
    let validated = validate_chunk_output(&planned, audio, source, output(segments))?;
    let ranges: Vec<(u64, u64, ProviderEndTrim)> = validated
        .segments()
        .iter()
        .map(|segment| {
            (
                segment.range().start().as_micros(),
                segment.range().end().as_micros(),
                segment.trimmed(),
            )
        })
        .collect();
    assert_eq!(
        ranges[..5],
        [
            (50_100_000, 52_100_000, ProviderEndTrim::Unchanged),
            (53_100_000, 59_100_000, ProviderEndTrim::Unchanged),
            (59_100_000, 60_100_000, ProviderEndTrim::TrimmedToAudioEnd),
            (59_600_000, 60_100_000, ProviderEndTrim::TrimmedToAudioEnd),
            (59_700_000, 60_100_000, ProviderEndTrim::Unchanged),
        ]
    );
    // The raw provider end is kept beside the trimmed range, however far past
    // the audio it lay.
    assert_eq!(
        validated.segments()[2].provider_end().as_micros(),
        10_600_000
    );
    assert_eq!(
        validated.segments()[3].provider_end().as_micros(),
        11_200_000
    );
    let warnings: Vec<_> = validated
        .warnings()
        .as_slice()
        .iter()
        .map(|warning| (warning.kind(), warning.count(), warning.first_cue()))
        .collect();
    assert_eq!(
        warnings,
        [
            (TranscriptWarningKind::ProviderSegmentsRejected, 1, 3),
            (TranscriptWarningKind::ProviderEndTrimmed, 2, 3),
        ]
    );
    Ok(())
}

/// #274: whisper.cpp ends the last segment of a range cut mid-speech past the
/// audio (a 5 s cut ended at 7 s). The segment starts inside the audio, so its
/// end is cut at the audio end, as far as the padded 30 s window the recogniser
/// works in, and the chunk is not failed for it.
#[test]
fn a_range_cut_mid_speech_keeps_its_last_segment_as_far_as_the_padded_window() -> TestResult {
    let planned = chunk(0, 0, 5 * SECOND)?;
    // F02's first five seconds decode to 5.001 s.
    let audio = range(0, 5_001_000)?;
    let source = range(0, 12 * SECOND)?;
    for end_ms in [5_500, 6_100, 7_000, 29_000, 30_000] {
        let segments = vec![provider_segment(
            0,
            end_ms,
            "First the queue is empty. Here it rises to 12.",
        )?];
        let validated = validate_chunk_output(&planned, audio, source, output(segments))?;
        let [only] = validated.segments() else {
            return Err("the segment was not kept".into());
        };
        assert_eq!(only.range().start().as_micros(), 0, "{end_ms}");
        assert_eq!(only.range().end().as_micros(), 5_001_000, "{end_ms}");
        assert_eq!(only.trimmed(), ProviderEndTrim::TrimmedToAudioEnd);
        assert_eq!(only.provider_end().as_micros(), end_ms * 1_000);
        let warnings: Vec<_> = validated
            .warnings()
            .as_slice()
            .iter()
            .map(|warning| (warning.kind(), warning.count()))
            .collect();
        assert_eq!(
            warnings,
            [(TranscriptWarningKind::ProviderEndTrimmed, 1)],
            "{end_ms}"
        );
    }
    Ok(())
}

/// What is still rejected after #274: a segment that does not start inside the
/// audio, a reversed one and an empty one, and the quarter rule still fails a
/// chunk that has too many of them.
#[test]
fn segments_that_do_not_start_inside_the_audio_are_still_rejected() -> TestResult {
    let planned = chunk(0, 0, 5 * SECOND)?;
    let audio = range(0, 5_001_000)?;
    let source = range(0, 12 * SECOND)?;
    for (start_ms, end_ms) in [(5_001, 7_000), (5_500, 7_000), (30_000, 31_000)] {
        let segments = vec![provider_segment(start_ms, end_ms, "after the audio")?];
        assert_eq!(
            validate_chunk_output(&planned, audio, source, output(segments)),
            Err(ProviderOutputError::TooManyRejectedSegments),
            "{start_ms}"
        );
    }
    // Reversed and empty ranges, beside a kept segment: rejected, counted.
    let segments = vec![
        provider_segment(0, 1_000, "kept")?,
        provider_segment(2_000, 2_000, "empty")?,
        provider_segment(3_000, 2_500, "reversed")?,
        provider_segment(3_100, 3_200, "kept too")?,
        provider_segment(3_300, 3_400, "and this one")?,
        provider_segment(3_500, 3_600, "and another")?,
        provider_segment(3_700, 3_800, "and one more")?,
        provider_segment(3_900, 4_000, "and the last")?,
    ];
    let validated = validate_chunk_output(&planned, audio, source, output(segments))?;
    assert_eq!(validated.segments().len(), 6);
    // A segment that starts just inside the audio is kept and cut, never empty.
    let segments = vec![provider_segment(5_000, 7_000, "right at the end")?];
    let validated = validate_chunk_output(&planned, audio, source, output(segments))?;
    let [only] = validated.segments() else {
        return Err("the segment was not kept".into());
    };
    assert_eq!(only.range().start().as_micros(), 5_000_000);
    assert_eq!(only.range().end().as_micros(), 5_001_000);
    Ok(())
}

/// The sanity bound on the overrun (review of #274): a recogniser's end stays
/// inside the padded 30 s window it works in, so an end beyond the window is not
/// a time of this audio and rejects the segment, however it starts. A short
/// chunk is bounded by the window (30 s), not by its audio.
#[test]
fn an_end_beyond_the_padded_window_rejects_the_segment() -> TestResult {
    let planned = chunk(0, 0, 5 * SECOND)?;
    let audio = range(0, 5_001_000)?;
    let source = range(0, 60 * SECOND)?;
    // The window's own end is kept and cut; one millisecond past it is not.
    let at_the_window = vec![provider_segment(0, 30_000, "to the window")?];
    let validated = validate_chunk_output(&planned, audio, source, output(at_the_window))?;
    assert_eq!(validated.segments().len(), 1);
    for end_ms in [30_001, 31_000, 59_000] {
        let beyond = vec![provider_segment(0, end_ms, "beyond the window")?];
        assert_eq!(
            validate_chunk_output(&planned, audio, source, output(beyond)),
            Err(ProviderOutputError::TooManyRejectedSegments),
            "{end_ms}"
        );
    }
    // Beside kept segments it is counted and left out, not a failure.
    let segments = vec![
        provider_segment(0, 1_000, "kept")?,
        provider_segment(1_100, 2_000, "kept too")?,
        provider_segment(2_100, 3_000, "and this one")?,
        provider_segment(3_100, 4_000, "and another")?,
        provider_segment(4_100, 7_000, "ran on, cut")?,
        provider_segment(4_200, 31_000, "beyond the window")?,
    ];
    let validated = validate_chunk_output(&planned, audio, source, output(segments))?;
    assert_eq!(validated.segments().len(), 5);
    let warnings: Vec<_> = validated
        .warnings()
        .as_slice()
        .iter()
        .map(|warning| (warning.kind(), warning.count()))
        .collect();
    assert_eq!(
        warnings,
        [
            (TranscriptWarningKind::ProviderSegmentsRejected, 1),
            (TranscriptWarningKind::ProviderEndTrimmed, 1),
        ]
    );
    Ok(())
}

/// The bound never falls below the second past the audio that 0.1.0 applied to
/// every chunk, so a chunk that fills the window keeps a segment ending up to a
/// second past it, and a revision stored by 0.1.0 still rebuilds.
#[test]
fn a_chunk_filling_the_window_keeps_the_second_of_slack_it_always_had() -> TestResult {
    let planned = chunk(0, 0, 30 * SECOND)?;
    let audio = range(0, 30 * SECOND)?;
    let source = range(0, 60 * SECOND)?;
    for end_ms in [30_100, 30_900, 31_000] {
        let segments = vec![provider_segment(29_000, end_ms, "runs a little on")?];
        let validated = validate_chunk_output(&planned, audio, source, output(segments))?;
        let [only] = validated.segments() else {
            return Err("the segment was not kept".into());
        };
        assert_eq!(only.range().end().as_micros(), 30 * SECOND, "{end_ms}");
        assert_eq!(only.trimmed(), ProviderEndTrim::TrimmedToAudioEnd);
    }
    let beyond = vec![provider_segment(29_000, 31_001, "a millisecond too far")?];
    assert_eq!(
        validate_chunk_output(&planned, audio, source, output(beyond)),
        Err(ProviderOutputError::TooManyRejectedSegments)
    );
    Ok(())
}

/// One text segment per second of a 10 s chunk, in start order: `true` is kept
/// as reported, `false` has an empty range (it starts and ends together), so it
/// is rejected.
fn pattern(kept: &[bool]) -> Built<ProviderChunkOutput> {
    let mut segments = Vec::new();
    for (second, kept) in (0_u64..).zip(kept) {
        let start = second * 1_000;
        let end = if *kept { start + 500 } else { start };
        segments.push(provider_segment(start, end, "words")?);
    }
    Ok(output(segments))
}

/// What a pattern of kept and rejected segments becomes in a 10 s chunk: the
/// kept count and the counted rejections, or the error that makes it unusable.
fn judged(kept: &[bool]) -> Built<Result<(usize, u32), ProviderOutputError>> {
    let planned = chunk(0, 0, 10 * SECOND)?;
    let judged = validate_chunk_output(
        &planned,
        planned.window(),
        range(0, 10 * SECOND)?,
        pattern(kept)?,
    );
    Ok(judged.map(|validated| {
        let rejected = validated
            .warnings()
            .as_slice()
            .iter()
            .find(|warning| warning.kind() == TranscriptWarningKind::ProviderSegmentsRejected)
            .map_or(0, |warning| warning.count());
        (validated.segments().len(), rejected)
    }))
}

/// #353: the real recording had chunks of a few segments, one of which did not
/// fit its audio. One rejected segment beside kept ones is dropped and counted,
/// whatever the reason it was rejected, and the others are kept.
#[test]
fn one_rejected_segment_beside_kept_ones_does_not_fail_a_sparse_chunk() -> TestResult {
    let planned = chunk(65, 0, 10 * SECOND)?;
    let audio = planned.window();
    let source = range(0, 10 * SECOND)?;
    // Three text segments, the middle one's range empty, reversed, or at or
    // after the audio's end, and a text that is only whitespace besides.
    for (label, rejected) in [
        ("empty range", provider_segment(4_000, 4_000, "no time")?),
        ("reversed", provider_segment(4_000, 3_500, "backwards")?),
        (
            "at the audio end",
            provider_segment(10_000, 11_000, "late")?,
        ),
        (
            "after the audio end",
            provider_segment(12_000, 13_000, "later")?,
        ),
        (
            "beyond the window",
            provider_segment(4_000, 31_000, "too long")?,
        ),
    ] {
        let mut segments = vec![provider_segment(1_000, 2_000, "first words")?];
        // Placed so that the start order holds whatever the rejected one is.
        let late = rejected.start.as_micros() >= 10_000_000;
        if !late {
            segments.push(rejected.clone());
        }
        segments.push(provider_segment(6_000, 7_000, "last words")?);
        if late {
            segments.push(rejected);
        }
        let validated = validate_chunk_output(&planned, audio, source, output(segments))?;
        let kept: Vec<&str> = validated
            .segments()
            .iter()
            .map(|segment| segment.text().text())
            .collect();
        assert_eq!(kept, ["first words", "last words"], "{label}");
        let warnings: Vec<_> = validated
            .warnings()
            .as_slice()
            .iter()
            .map(|warning| (warning.kind(), warning.count(), warning.first_cue()))
            .collect();
        assert_eq!(
            warnings,
            [(TranscriptWarningKind::ProviderSegmentsRejected, 1, 66)],
            "{label}"
        );
    }
    Ok(())
}

/// A chunk whose every text segment is rejected is unusable whatever its size:
/// nothing the recogniser said about it can be placed.
#[test]
fn a_chunk_whose_every_segment_is_rejected_is_unusable() -> TestResult {
    for count in [1_usize, 2, 3, 4, 5, 8, 20] {
        assert_eq!(
            judged(&vec![false; count])?,
            Err(ProviderOutputError::TooManyRejectedSegments),
            "{count} rejected"
        );
    }
    // No text segment at all is not a rejection: a chunk with nothing said.
    assert_eq!(judged(&[])?, Ok((0, 0)));
    Ok(())
}

/// Below `MIN_SEGMENTS_FOR_REJECTION_RATIO` text segments one rejected segment
/// is dropped and counted and two are not tolerated; from the minimum on the
/// rule is the quarter rule it always was.
#[test]
fn the_quarter_rule_applies_from_four_text_segments() -> TestResult {
    // Below the minimum: one rejection is dropped and counted, whatever the
    // share, and two or more are not tolerated.
    assert_eq!(judged(&[true, false])?, Ok((1, 1)));
    assert_eq!(judged(&[false, true])?, Ok((1, 1)));
    assert_eq!(judged(&[true, true, false])?, Ok((2, 1)));
    for two_rejected in [[false, true, false], [false, false, true]] {
        assert_eq!(
            judged(&two_rejected)?,
            Err(ProviderOutputError::TooManyRejectedSegments),
            "{two_rejected:?}"
        );
    }
    // A valid segment added to a chunk that is not usable does not make it
    // usable, and one added to a usable chunk does not make it unusable (the
    // rule was not monotonic once: `[rejected, ok, rejected]` was kept and
    // `[rejected, ok, rejected, ok]` failed it).
    assert_eq!(
        judged(&[false, true, false, true])?,
        Err(ProviderOutputError::TooManyRejectedSegments)
    );
    assert_eq!(
        judged(&[false, true, false, true, true])?,
        Err(ProviderOutputError::TooManyRejectedSegments)
    );
    assert_eq!(judged(&[false, true, true])?, Ok((2, 1)));
    assert_eq!(judged(&[false, true, true, true])?, Ok((3, 1)));
    assert_eq!(judged(&[false, true, true, true, true])?, Ok((4, 1)));
    // At the minimum one rejection is exactly a quarter and is tolerated; two
    // are not.
    assert_eq!(judged(&[true, true, true, false])?, Ok((3, 1)));
    assert_eq!(judged(&[false, true, true, true])?, Ok((3, 1)));
    for two_rejected in [
        [true, true, false, false],
        [false, false, true, true],
        [false, true, false, true],
    ] {
        assert_eq!(
            judged(&two_rejected)?,
            Err(ProviderOutputError::TooManyRejectedSegments),
            "{two_rejected:?}"
        );
    }
    // Above it the share stays a quarter: 1 of 5 passes, 2 of 5 fails; 2 of 8
    // is a quarter and passes, 3 of 8 fails.
    assert_eq!(judged(&[true, true, true, true, false])?, Ok((4, 1)));
    assert_eq!(
        judged(&[true, true, true, false, false])?,
        Err(ProviderOutputError::TooManyRejectedSegments)
    );
    let mut eight = [true; 8];
    eight[2] = false;
    eight[5] = false;
    assert_eq!(judged(&eight)?, Ok((6, 2)));
    eight[7] = false;
    assert_eq!(
        judged(&eight)?,
        Err(ProviderOutputError::TooManyRejectedSegments)
    );
    Ok(())
}

/// The rule says: every segment rejected is unusable; otherwise more than one
/// rejected is unusable below the minimum and more than a quarter from it on.
/// The same rule in closed form, to compare it with.
const fn closed_form(segments: u32, rejected: u32) -> bool {
    rejected == segments || (rejected >= 2 && rejected * 4 > segments)
}

/// The quarter rule alone, as it was before #353.
const fn quarter_rule_alone(segments: u32, rejected: u32) -> bool {
    rejected == segments || rejected * 4 > segments
}

/// #353, monotonic: for every segment count and rejected count up to sixteen,
/// another valid segment never turns a usable chunk unusable, another
/// rejection never turns an unusable chunk usable, and neither does a rejected
/// segment that is added (it raises both counts). The rule was once not
/// monotonic below the minimum: two rejections were kept among three segments
/// and failed among four.
#[test]
fn the_rejection_rule_is_monotonic_for_every_count_up_to_sixteen() {
    let rule = super::rejections_make_chunk_unusable;
    for segments in 1..=16_u32 {
        for rejected in 0..=segments {
            let unusable = rule(segments, rejected);
            // One more valid segment.
            if segments < 16 {
                assert!(
                    unusable || !rule(segments + 1, rejected),
                    "{rejected} of {segments} is usable but {rejected} of {} is not",
                    segments + 1
                );
            }
            // One valid segment turned into a rejected one.
            if rejected < segments {
                assert!(
                    !unusable || rule(segments, rejected + 1),
                    "{rejected} of {segments} is unusable but {} of {segments} is not",
                    rejected + 1
                );
            }
            // One more rejected segment.
            if segments < 16 {
                assert!(
                    !unusable || rule(segments + 1, rejected + 1),
                    "{rejected} of {segments} is unusable but {} of {} is not",
                    rejected + 1,
                    segments + 1
                );
            }
            // The closed form is the rule.
            assert_eq!(unusable, closed_form(segments, rejected));
        }
    }
    // Nothing said is not a rejection.
    assert!(!rule(0, 0));
}

/// What the rule tolerates, by segment count: the most rejected segments a
/// chunk can have and still be usable. A single rejection never costs a chunk
/// that has another segment; two cost it below eight segments; the share
/// grows by one for every four segments after that.
#[test]
fn the_rule_tolerates_the_documented_number_of_rejections() {
    let rule = super::rejections_make_chunk_unusable;
    let tolerated = |segments: u32| {
        (0..segments)
            .rev()
            .find(|rejected| !rule(segments, *rejected))
            .unwrap_or(0)
    };
    let expected = [0, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4];
    for (segments, expected) in (1..=16_u32).zip(expected) {
        assert_eq!(tolerated(segments), expected, "{segments} segments");
    }
}

/// Why the minimum is 4, found by search and not assumed: it is the smallest
/// count from which the quarter rule alone (what #353 found failing sparse
/// chunks) and the rule agree for every count above it. Below it the quarter
/// rule alone tolerates no rejection at all, and the rule tolerates one. If the
/// rule's boundary moves, this fails.
#[test]
fn the_minimum_is_where_the_quarter_rule_alone_starts_to_agree() {
    let rule = super::rejections_make_chunk_unusable;
    let agrees = |segments: u32| {
        (0..=segments)
            .all(|rejected| rule(segments, rejected) == quarter_rule_alone(segments, rejected))
    };
    let first_always_agreeing = (1..=64_u32)
        .find(|from| (*from..=64).all(agrees))
        .unwrap_or(u32::MAX);
    assert_eq!(first_always_agreeing, MIN_SEGMENTS_FOR_REJECTION_RATIO);
    // And what they differ in below it: one rejection.
    for segments in 2..MIN_SEGMENTS_FOR_REJECTION_RATIO {
        assert!(quarter_rule_alone(segments, 1), "{segments}");
        assert!(!rule(segments, 1), "{segments}");
        assert!(rule(segments, 2), "{segments}");
    }
}

/// Empty or marker-only text is removed and counted as a marker, never as a
/// rejection, and does not count as a text segment: it can neither lift a
/// chunk to the minimum nor dilute the share of rejections.
#[test]
fn a_text_that_is_a_marker_does_not_count_as_a_segment() -> TestResult {
    let planned = chunk(0, 0, 10 * SECOND)?;
    let mut blank = provider_segment(1_500, 2_500, "ignored")?;
    blank.text = None;
    // One kept and two rejected text segments, with five markers among them.
    // Three text segments: two rejected are not tolerated below the minimum.
    // Had the five markers counted, two of eight would be a quarter, which is
    // tolerated, and the chunk would be kept.
    let mut segments = vec![
        provider_segment(1_000, 1_400, "kept")?,
        blank,
        provider_segment(2_600, 2_900, "[BLANK_AUDIO]")?,
        provider_segment(2_950, 3_000, "(music)")?,
        provider_segment(3_000, 3_000, "empty range")?,
        provider_segment(3_100, 3_200, "[Applause]")?,
        provider_segment(3_300, 3_400, "[Music]")?,
        provider_segment(4_000, 3_000, "reversed")?,
    ];
    assert_eq!(
        validate_chunk_output(
            &planned,
            planned.window(),
            planned.window(),
            output(segments.clone())
        ),
        Err(ProviderOutputError::TooManyRejectedSegments)
    );
    // With only one rejection among them the chunk is kept, and the markers are
    // counted as markers.
    segments.remove(4);
    let validated = validate_chunk_output(
        &planned,
        planned.window(),
        planned.window(),
        output(segments),
    )?;
    assert_eq!(validated.segments().len(), 1);
    let warnings: Vec<_> = validated
        .warnings()
        .as_slice()
        .iter()
        .map(|warning| (warning.kind(), warning.count()))
        .collect();
    assert_eq!(
        warnings,
        [
            (TranscriptWarningKind::ProviderSegmentsRejected, 1),
            (TranscriptWarningKind::NonSpeechMarkersRemoved, 5),
        ]
    );
    Ok(())
}

/// #353: a run fails only when more than half of the chunks the recogniser
/// answered are unusable. The threshold's edges, for small and large runs.
#[test]
fn a_run_fails_only_when_more_than_half_of_its_answered_chunks_are_unusable() {
    assert_eq!(UNUSABLE_CHUNK_SHARE_DENOMINATOR, 2);
    for (unusable, answered, fails) in [
        // Nothing answered, nothing unusable.
        (0, 0, false),
        (0, 5, false),
        // A single chunk the recogniser cannot describe still fails the run.
        (1, 1, true),
        // One bad chunk of two does not: the other was answered.
        (1, 2, false),
        (2, 2, true),
        (1, 3, false),
        (2, 3, true),
        (2, 4, false),
        (3, 4, true),
        // The recording that found #353: 1 unusable chunk of 83, and the
        // edge of a majority of 83.
        (1, 83, false),
        (41, 83, false),
        (42, 83, true),
        (83, 83, true),
    ] {
        assert_eq!(
            unusable_chunks_end_the_run(unusable, answered),
            fails,
            "{unusable} of {answered}"
        );
    }
}

proptest! {
    /// The verdict is a strict majority for every size of run, never fails a
    /// run with no unusable chunk, and always fails a run in which every
    /// answered chunk is unusable.
    #[test]
    fn the_run_threshold_is_a_strict_majority(answered in 1_u32..=1_024, share in 0_u32..=100) {
        let unusable = answered * share / 100;
        prop_assert_eq!(
            unusable_chunks_end_the_run(unusable, answered),
            u64::from(unusable) * 2 > u64::from(answered)
        );
        prop_assert!(!unusable_chunks_end_the_run(0, answered));
        prop_assert!(unusable_chunks_end_the_run(answered, answered));
        // Failing is monotonic: one more unusable chunk of the same answered
        // ones can only keep a failing run failing.
        if unusable_chunks_end_the_run(unusable, answered) && unusable < answered {
            prop_assert!(unusable_chunks_end_the_run(unusable + 1, answered));
        }
    }
}

/// T-05: a chunk whose provider evidently described other audio fails.
#[test]
fn chunks_with_too_many_rejected_segments_fail() -> TestResult {
    let planned = chunk(0, 0, 10 * SECOND)?;
    let audio = planned.window();
    let source = range(0, 10 * SECOND)?;
    // Four of eight text segments do not fit their audio, which is more than a
    // quarter of a chunk of dense speech: the provider evidently described
    // other audio, so the chunk is unusable. (Below four segments the same
    // share would be dropped and counted, as the tests above show.)
    let mostly_wrong = vec![
        provider_segment(0, 1_000, "fine")?,
        provider_segment(1_000, 1_000, "empty")?,
        provider_segment(2_000, 2_500, "kept")?,
        provider_segment(3_000, 3_500, "kept")?,
        provider_segment(4_000, 4_500, "kept")?,
        provider_segment(12_000, 13_000, "after the audio")?,
        provider_segment(12_500, 13_000, "after the audio")?,
        provider_segment(13_000, 14_000, "after the audio")?,
    ];
    assert_eq!(
        validate_chunk_output(&planned, audio, source, output(mostly_wrong)),
        Err(ProviderOutputError::TooManyRejectedSegments)
    );
    let all_wrong = vec![provider_segment(20_000, 21_000, "after the audio")?];
    assert_eq!(
        validate_chunk_output(&planned, audio, source, output(all_wrong)),
        Err(ProviderOutputError::TooManyRejectedSegments)
    );
    // A segment beyond the source is rejected even inside the decoded audio.
    let short_source = range(0, 5 * SECOND)?;
    let beyond = vec![
        provider_segment(0, 1_000, "one")?,
        provider_segment(1_000, 2_000, "two")?,
        provider_segment(2_000, 3_000, "three")?,
        provider_segment(3_000, 4_000, "four")?,
        provider_segment(4_500, 5_500, "crosses the source end")?,
    ];
    let validated = validate_chunk_output(&planned, audio, short_source, output(beyond))?;
    assert_eq!(validated.segments().len(), 4);
    Ok(())
}

/// T-05: malformed provider output fails the chunk rather than being repaired.
#[test]
fn malformed_provider_output_fails_the_chunk() -> TestResult {
    let planned = chunk(0, 0, 10 * SECOND)?;
    let audio = planned.window();
    let source = planned.window();
    let reordered = vec![
        provider_segment(3_000, 4_000, "second")?,
        provider_segment(1_000, 2_000, "first")?,
    ];
    assert_eq!(
        validate_chunk_output(&planned, audio, source, output(reordered)),
        Err(ProviderOutputError::OutOfOrderSegments)
    );
    for probability in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
        let mut segment = provider_segment(0, 1_000, "score")?;
        segment
            .tokens
            .push(token(ProviderTokenKind::Special, probability));
        assert_eq!(
            validate_chunk_output(&planned, audio, source, output(vec![segment])),
            Err(ProviderOutputError::InvalidTokenProbability),
            "{probability}"
        );
    }
    Ok(())
}

#[test]
fn confidence_is_the_uncalibrated_mean_of_text_tokens() -> TestResult {
    let planned = chunk(0, 0, 10 * SECOND)?;
    let validated = validate_chunk_output(
        &planned,
        planned.window(),
        planned.window(),
        output(vec![provider_segment(0, 1_000, "scored")?]),
    )?;
    let confidence = validated.segments()[0].confidence();
    // Special tokens (0.5 and 0.01) are excluded: mean of 0.9 and 0.7.
    assert_eq!(confidence.basis_points(), Some(8_000));
    assert_eq!(confidence.origin(), ConfidenceOrigin::ProviderUncalibrated);
    assert_eq!(
        ProviderTokenKind::classify("[_TT_263]"),
        ProviderTokenKind::Special
    );
    assert_eq!(
        ProviderTokenKind::classify("<|en|>"),
        ProviderTokenKind::Special
    );
    assert_eq!(
        ProviderTokenKind::classify(" service"),
        ProviderTokenKind::Text
    );
    Ok(())
}

/// T-03 silence: non-speech markers are removed and counted.
#[test]
fn non_speech_markers_are_removed() -> TestResult {
    let planned = chunk(1, 25 * SECOND, 55 * SECOND)?;
    let mut blank = provider_segment(0, 30_000, "[BLANK_AUDIO]")?;
    let music = provider_segment(0, 30_000, "(music) [Applause]")?;
    let speech = provider_segment(1_000, 3_000, "[inaudible] words remain")?;
    let validated = validate_chunk_output(
        &planned,
        planned.window(),
        range(0, 60 * SECOND)?,
        output(vec![
            {
                blank.text = None;
                blank
            },
            provider_segment(0, 30_000, "[BLANK_AUDIO]")?,
            music,
            speech,
        ]),
    )?;
    assert_eq!(validated.segments().len(), 1);
    assert_eq!(
        validated.segments()[0].text().text(),
        "[inaudible] words remain"
    );
    let warnings: Vec<_> = validated
        .warnings()
        .as_slice()
        .iter()
        .map(|warning| (warning.kind(), warning.count(), warning.first_cue()))
        .collect();
    assert_eq!(
        warnings,
        [(TranscriptWarningKind::NonSpeechMarkersRemoved, 3, 2)]
    );
    Ok(())
}

/// T-03 silence: a chunk is silent only when every 20 ms frame is below -50 dBFS.
#[test]
fn silence_is_every_frame_below_minus_fifty_dbfs() {
    assert!(is_silent_pcm(&[]));
    assert!(is_silent_pcm(&vec![0; 16_000]));
    // Amplitude 100 is about -50.3 dBFS: silent. 104 is about -49.97 dBFS.
    assert!(is_silent_pcm(&vec![100; 16_000]));
    assert!(!is_silent_pcm(&vec![104; 16_000]));
    let mut one_loud_frame = vec![0_i16; 16_000];
    for sample in &mut one_loud_frame[640..960] {
        *sample = 2_000;
    }
    assert!(!is_silent_pcm(&one_loud_frame));
    // A short click inside one frame still makes that frame audible.
    let mut click = vec![0_i16; 16_000];
    click[5] = i16::MIN;
    click[6] = i16::MAX;
    assert!(!is_silent_pcm(&click));
}

/// #322: decoded audio under 100 ms (1,600 samples at 16 kHz) is below the
/// floor whatever it holds; 100 ms is not. The floor is a count of samples, so
/// full-scale audio one sample short is under it and silence at the floor is
/// not (silence has its own rule).
#[test]
fn the_recognition_floor_is_one_hundred_milliseconds_of_samples() -> TestResult {
    assert_eq!(MIN_RECOGNITION_SAMPLES, 1_600);
    assert_eq!(
        u64::try_from(MIN_RECOGNITION_SAMPLES)? * 1_000_000 / u64::from(SPEECH_SAMPLE_RATE),
        100_000,
        "the floor is 100 ms of the speech sample rate"
    );
    for level in [0_i16, 100, 3_000, i16::MAX, i16::MIN] {
        assert!(is_below_recognition_floor(&vec![level; 1_599]), "{level}");
        assert!(!is_below_recognition_floor(&vec![level; 1_600]), "{level}");
    }
    // The sizes whisper.cpp v1.9.2 mishandles are all under it: 40 samples
    // (its command line fails), 200 (it reads past the buffer) and one.
    for samples in [0_usize, 1, 40, 41, 200, 201] {
        assert!(
            is_below_recognition_floor(&vec![i16::MAX; samples]),
            "{samples}"
        );
    }
    assert!(!is_below_recognition_floor(&vec![3_000; 16_000]));
    assert!(!is_below_recognition_floor(&vec![3_000; 480_000]));
    // The two rules are independent: loud and short is under the floor and
    // not silent; long and quiet is silent and not under the floor.
    assert!(!is_silent_pcm(&vec![3_000; 1_599]));
    assert!(is_silent_pcm(&vec![0; 1_600]));
    Ok(())
}

/// #332, the decode boundary: a range whose length rounds to no 16 kHz sample
/// (under 31.25 microseconds) is not a range to ask a decoder for; the media
/// tool took 1 and 31 microseconds for no limit and returned a block of about
/// four seconds. 32 and 62 microseconds are shorter than a sample too (62.5), but
/// they round to one and were cut to one: they stay decode requests, as 63 is.
#[test]
fn only_a_range_that_rounds_to_no_sample_is_not_a_decode_request() -> TestResult {
    let start = 2 * SECOND;
    for micros in [1_u64, 2, 30, 31] {
        assert!(
            rounds_to_no_pcm_sample(range(start, start + micros)?),
            "{micros}"
        );
    }
    for micros in [32_u64, 33, 62, 63, 64, 125, 99_999, 100_000, 30 * SECOND] {
        assert!(
            !rounds_to_no_pcm_sample(range(start, start + micros)?),
            "{micros}"
        );
    }
    // The rule is the rounded count of samples and nothing else: for every
    // length up to two samples it agrees with rounding to the nearest.
    for micros in 1_u64..=125 {
        let doubled = micros * 2 * u64::from(SPEECH_SAMPLE_RATE);
        let nearest = (doubled + 1_000_000) / 2_000_000;
        assert_eq!(
            rounds_to_no_pcm_sample(range(start, start + micros)?),
            nearest == 0,
            "{micros}"
        );
    }
    // The longest range there is does not overflow the rule.
    assert!(!rounds_to_no_pcm_sample(range(0, u64::MAX)?));
    Ok(())
}

/// #332: a window shorter than 100 ms is below the floor before anything is
/// decoded, down to one microsecond; a window of exactly 100 ms is not. The
/// lengths are those the media tool was seen to mishandle (it answered a
/// window of 31 microseconds or less with a block of about four seconds) and
/// the two sides of the floor.
#[test]
fn a_window_shorter_than_the_floor_is_below_it_whatever_a_decoder_would_return() -> TestResult {
    assert_eq!(MIN_RECOGNITION_MICROS, 100_000);
    assert_eq!(
        u64::try_from(MIN_RECOGNITION_SAMPLES)? * 1_000_000 / u64::from(SPEECH_SAMPLE_RATE),
        MIN_RECOGNITION_MICROS,
        "the two floors are one length"
    );
    let start = 2 * SECOND;
    for micros in [1_u64, 31, 32, 62, 63, 1_000, 12_500, 50_000, 99_999] {
        assert!(
            chunk(0, start, start + micros)?.is_below_recognition_floor(),
            "{micros}"
        );
    }
    for micros in [100_000_u64, 100_001, SECOND, 30 * SECOND] {
        assert!(
            !chunk(0, start, start + micros)?.is_below_recognition_floor(),
            "{micros}"
        );
    }
    Ok(())
}

proptest! {
    /// With the R0 plan a window is below the floor only when the whole
    /// requested range is, and then it is the only chunk: a longer range never
    /// leaves a tail too short to recognise (its last window is longer than
    /// the 5 s overlap).
    #[test]
    fn only_a_requested_range_under_the_floor_gives_an_r0_chunk_under_it(
        start in 0_u64..14_000_000_000,
        length in 1_u64..200_000_000,
    ) {
        let (Ok(requested), Ok(id)) = (range(start, start + length), segment_id()) else {
            return Ok(());
        };
        let Ok(chunks) = plan_chunks(&id, requested, ChunkPlan::R0) else {
            return Ok(());
        };
        let below = chunks
            .iter()
            .filter(|chunk| chunk.is_below_recognition_floor())
            .count();
        if length < MIN_RECOGNITION_MICROS {
            prop_assert_eq!((chunks.len(), below), (1, 1));
        } else {
            prop_assert_eq!(below, 0);
        }
    }

    /// The floor depends on the number of samples alone, and it is the same
    /// line as "the decoded range is shorter than 100 ms": a record that keeps
    /// only the range still says which side of the floor the chunk was on.
    #[test]
    fn the_floor_follows_the_sample_count_and_the_decoded_range(
        samples in 0_usize..4_000,
        level in proptest::num::i16::ANY,
        start in 0_u64..14_400_000_000,
    ) {
        let pcm = vec![level; samples];
        prop_assert_eq!(is_below_recognition_floor(&pcm), samples < 1_600);
        let decoded = decoded_audio_range(MediaTime::from_micros(start), samples);
        prop_assert_eq!(decoded.is_none(), samples == 0);
        if let Some(decoded) = decoded {
            prop_assert_eq!(
                decoded.duration_micros() < 100_000,
                is_below_recognition_floor(&pcm)
            );
        }
    }
}

/// T-03: a sentence crossing a seam is heard whole by one chunk and kept once.
#[test]
fn a_sentence_across_a_seam_is_kept_once() -> TestResult {
    // Chunk 0 [0, 30) cuts the sentence at its edge; chunk 1 [25, 55) hears it whole.
    let first = with(
        draft(
            0,
            (0, 30 * SECOND),
            20 * SECOND,
            24 * SECOND,
            "before the seam",
        )?,
        draft(
            0,
            (0, 30 * SECOND),
            26 * SECOND,
            29_800_000,
            "the sentence crosses",
        )?,
    );
    let second = draft(
        1,
        (25 * SECOND, 55 * SECOND),
        26 * SECOND,
        32 * SECOND,
        "the sentence crosses the seam cleanly",
    )?;
    let (kept, duplicates) = texts(&[first, second]);
    assert_eq!(
        kept,
        ["before the seam", "the sentence crosses the seam cleanly"]
    );
    assert_eq!(duplicates, 0);
    Ok(())
}

/// #274: the last segment of a middle chunk, cut by the chunk's edge, may be
/// ended by the recogniser well past the audio (seconds, not a second). It is
/// kept and cut at the chunk end, then stitched like any cut segment: the
/// neighbour that heard the sentence whole supplies it, and the transcript has
/// no repeat, no gap and no step backwards. The windows are 20 s, as the
/// shorter chunks of a range are: a full 30 s window can be overrun by a second
/// at most, because the recogniser's ends stay inside the padded 30 s window.
#[test]
fn an_overrunning_final_segment_of_a_chunk_stitches_without_a_gap_or_a_repeat() -> TestResult {
    let (window_0, window_1, window_2) = (
        (0, 20 * SECOND),
        (15 * SECOND, 35 * SECOND),
        (30 * SECOND, 50 * SECOND),
    );
    // Chunks 0 and 1 end mid-sentence, and the recogniser runs those segments'
    // ends 3 s and 2.5 s past their audio.
    let first = with(
        draft(
            0,
            window_0,
            10 * SECOND,
            14 * SECOND,
            "before the first seam",
        )?,
        draft(
            0,
            window_0,
            16 * SECOND,
            23 * SECOND,
            "the first sentence crosses",
        )?,
    );
    let second = with(
        draft(
            1,
            window_1,
            16 * SECOND,
            22 * SECOND,
            "the first sentence crosses the seam cleanly",
        )?,
        draft(
            1,
            window_1,
            31 * SECOND,
            37_500_000,
            "and the second sentence crosses",
        )?,
    );
    let third = draft(
        2,
        window_2,
        31 * SECOND,
        38 * SECOND,
        "and the second sentence crosses the seam too",
    )?;
    let merged = merge_chunks(&[first, second, third]);
    let kept: Vec<&str> = merged
        .segments
        .iter()
        .map(|merged| merged.segment.text().text())
        .collect();
    assert_eq!(
        kept,
        [
            "before the first seam",
            "the first sentence crosses the seam cleanly",
            "and the second sentence crosses the seam too",
        ]
    );
    let starts: Vec<u64> = merged
        .segments
        .iter()
        .map(|merged| merged.segment.range().start().as_micros())
        .collect();
    assert!(
        starts.windows(2).all(|pair| pair[0] < pair[1]),
        "the transcript steps backwards: {starts:?}"
    );
    assert!(
        merged
            .warnings
            .as_slice()
            .iter()
            .all(|warning| warning.kind() != TranscriptWarningKind::SeamDuplicatesRemoved),
        "a repeat was found where the neighbour supplied the sentence"
    );
    Ok(())
}

/// T-03: identical and time-shifted copies of seam text are kept once, when
/// what is kept covers the dropped copy's time to within the tolerance (#382).
#[test]
fn seam_duplicates_are_removed() -> TestResult {
    // Both chunks own a copy by midpoint (26.4 s and 27.5 s, the boundary of
    // the cores); the later copy ends 0.5 s after the earlier one, the tolerance.
    let first = draft(
        0,
        (0, 30 * SECOND),
        25_300_000,
        27_500_000,
        "Service is healthy.",
    )?;
    let second = draft(
        1,
        (25 * SECOND, 55 * SECOND),
        27_000_000,
        28_000_000,
        "service is healthy",
    )?;
    let (kept, duplicates) = texts(&[first, second]);
    assert_eq!(kept, ["Service is healthy."]);
    assert_eq!(duplicates, 1);

    // A later copy that continues the sentence loses only the repeated words.
    let first = draft(0, (0, 30 * SECOND), 25_300_000, 27_500_000, "the build is")?;
    let second = draft(
        1,
        (25 * SECOND, 55 * SECOND),
        26_600_000,
        29_000_000,
        "build is 2048.",
    )?;
    let (kept, duplicates) = texts(&[first, second]);
    assert_eq!(kept, ["the build is", "2048."]);
    assert_eq!(duplicates, 1);
    Ok(())
}

/// #382: a later copy whose words are all in the earlier segment is kept whole
/// when dropping it would leave more than the tolerance of its time in no kept
/// segment. Here the two chunks end the same sentence 1.5 s apart (27.5 s and
/// 29.0 s): until #382 the later copy was dropped and 27.5 to 29.0 s was in no
/// segment, called transcribed; now the words stand twice, which can be seen.
#[test]
fn a_contained_copy_that_carries_time_beyond_the_tolerance_is_kept() -> TestResult {
    let first = draft(
        0,
        (0, 30 * SECOND),
        25_300_000,
        27_500_000,
        "Service is healthy.",
    )?;
    let second = draft(
        1,
        (25 * SECOND, 55 * SECOND),
        26_600_000,
        29_000_000,
        "service is healthy",
    )?;
    let (kept, duplicates) = texts(&[first, second]);
    assert_eq!(kept, ["Service is healthy.", "service is healthy"]);
    assert_eq!(duplicates, 0);
    Ok(())
}

/// #380: a repeat that differs by one word inside a run of four or more is
/// found; one that differs at the first or last compared word, or in a run of
/// three, is not, because that is more likely a different word than the same
/// word heard differently.
#[test]
fn a_repeat_that_differs_by_one_inner_word_is_trimmed() -> TestResult {
    let seam = |earlier: &str, later: &str| -> Built<(Vec<String>, u32)> {
        let first = draft(0, (0, 30 * SECOND), 22 * SECOND, 27_500_000, earlier)?;
        let second = draft(
            1,
            (25 * SECOND, 55 * SECOND),
            26 * SECOND,
            31 * SECOND,
            later,
        )?;
        Ok(texts(&[first, second]))
    };
    // "settings" and "setting": five words, the third different.
    assert_eq!(
        seam(
            "so we open the settings page and",
            "open the setting page and click save"
        )?,
        (
            vec![
                "so we open the settings page and".to_owned(),
                "click save".to_owned()
            ],
            1
        )
    );
    // Different at the last compared word: not a repeat.
    let (kept, duplicates) = seam("so we open the file", "we open the door and press save")?;
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[1], "we open the door and press save");
    assert_eq!(duplicates, 0);
    // Three words, one different: not a repeat.
    let (kept, duplicates) = seam("press the red", "press a red button")?;
    assert_eq!(kept[1], "press a red button");
    assert_eq!(duplicates, 0);
    // A whole copy with one inner word different is dropped when its time is
    // covered (here it lies inside the earlier segment).
    let first = draft(
        0,
        (0, 30 * SECOND),
        22 * SECOND,
        29 * SECOND,
        "then press the blue save button",
    )?;
    let second = draft(
        1,
        (25 * SECOND, 55 * SECOND),
        27_200_000,
        28_900_000,
        "press the new save button",
    )?;
    assert_eq!(
        texts(&[first, second]),
        (vec!["then press the blue save button".to_owned()], 1)
    );
    Ok(())
}

/// #380: every overlapping pair of kept segments of consecutive chunks is
/// compared, not only the last of the earlier chunk and the first of the later
/// one. Here the later chunk's first kept segment ("um") shares nothing with the
/// earlier chunk's last, so the old rule stopped there, and its second one
/// repeats the earlier segment's last three words.
#[test]
fn a_repeat_beyond_the_first_pair_of_a_seam_is_trimmed() -> TestResult {
    let window_0 = (0, 30 * SECOND);
    let window_1 = (25 * SECOND, 55 * SECOND);
    let first = draft(
        0,
        window_0,
        22 * SECOND,
        29_500_000,
        "we open the file and press save now",
    )?;
    let second = with(
        draft(1, window_1, 25 * SECOND, 26 * SECOND, "um")?,
        draft(
            1,
            window_1,
            27 * SECOND,
            31 * SECOND,
            "press save now and wait",
        )?,
    );
    let chunks = [first, second];
    let every: BTreeSet<(usize, usize)> = [(0, 0), (1, 0), (1, 1)].into_iter().collect();
    let merged_transcript = super::assemble(&chunks, every);
    let words: Vec<&str> = merged_transcript
        .segments
        .iter()
        .map(|kept| kept.segment.text().text())
        .collect();
    assert_eq!(
        words,
        ["we open the file and press save now", "um", "and wait"]
    );
    Ok(())
}

/// T-03: repeated words at different times are genuine speech and survive.
#[test]
fn repeated_words_at_different_times_survive() -> TestResult {
    let first = draft(
        0,
        (0, 30 * SECOND),
        20 * SECOND,
        23 * SECOND,
        "again and again",
    )?;
    let second = draft(
        1,
        (25 * SECOND, 55 * SECOND),
        31 * SECOND,
        34 * SECOND,
        "again and again",
    )?;
    let (kept, duplicates) = texts(&[first, second]);
    assert_eq!(kept, ["again and again", "again and again"]);
    assert_eq!(duplicates, 0);
    // One shared word at an overlapping time is not enough to call a duplicate.
    let first = draft(0, (0, 30 * SECOND), 25_300_000, 27_400_000, "yes")?;
    let second = draft(1, (25 * SECOND, 55 * SECOND), 26_600_000, 29_000_000, "yes")?;
    assert_eq!(texts(&[first, second]).0, ["yes", "yes"]);
    Ok(())
}

/// T-03 silence: a silent chunk contributes nothing and does not break its neighbours.
#[test]
fn silent_chunks_take_part_without_segments() -> TestResult {
    let first = draft(0, (0, 30 * SECOND), 2 * SECOND, 4 * SECOND, "opening words")?;
    let silent = empty(1, (25 * SECOND, 55 * SECOND))?;
    let third = draft(
        2,
        (50 * SECOND, 70 * SECOND),
        60 * SECOND,
        62 * SECOND,
        "closing words",
    )?;
    let (kept, duplicates) = texts(&[first, silent, third]);
    assert_eq!(kept, ["opening words", "closing words"]);
    assert_eq!(duplicates, 0);
    Ok(())
}

/// A cut segment with no uncut neighbour copy is kept: speech is never lost.
#[test]
fn a_cut_segment_without_a_neighbour_copy_is_kept() -> TestResult {
    let first = draft(
        0,
        (0, 30 * SECOND),
        26 * SECOND,
        29_800_000,
        "cut but alone",
    )?;
    let second = empty(1, (25 * SECOND, 55 * SECOND))?;
    assert_eq!(texts(&[first, second]).0, ["cut but alone"]);
    Ok(())
}

/// T-03 regression (P07 increment 3c, `base_q5_1` on the real seam clip): the
/// later chunk hears the whole sentence from its first sample, and the earlier
/// chunk's previous sentence ends 0.24 s after that. The previous sentence is
/// not a copy of the cut one, so the whole sentence is kept exactly once
/// instead of being dropped from both chunks.
#[test]
fn a_sentence_starting_at_a_window_is_not_covered_by_the_previous_sentence() -> TestResult {
    let window_0 = (0, 30 * SECOND);
    let window_1 = (25 * SECOND, 55 * SECOND);
    let first = with(
        draft(
            0,
            window_0,
            22_600_000,
            25_240_000,
            "and leaves submit enabled",
        )?,
        draft(
            0,
            window_0,
            25_240_000,
            29_780_000,
            "the orange line spikes at ten",
        )?,
    );
    let second = with(
        draft(
            1,
            window_1,
            25 * SECOND,
            34 * SECOND,
            "the orange line spikes at ten thirty two while the median stays",
        )?,
        draft(
            1,
            window_1,
            34 * SECOND,
            40 * SECOND,
            "first the queue is empty",
        )?,
    );
    let (kept, _) = texts(&[first, second]);
    assert_eq!(
        kept,
        [
            "and leaves submit enabled",
            "the orange line spikes at ten thirty two while the median stays",
            "first the queue is empty"
        ]
    );
    Ok(())
}

/// #359, the first row of the report, with its own timings. The earlier chunk
/// (window 0 to 30 s) ends with a segment 22.32 to 29.50 s, which ends 0.5 s
/// before its window does and so is not cut. The later chunk (25 to 55 s)
/// starts with 25.00 to 32.44 s and 32.44 to 39.72 s. The first of those is cut
/// by its window's edge, and the earlier segment spans its midpoint (28.72 s),
/// so the merge used to keep the earlier segment instead and drop the later one
/// whole, although it runs 2.94 s past the end of the earlier one, into audio
/// the earlier chunk never heard. Everything after 29.50 s was in neither chunk's
/// output. Now the later segment is kept and loses only the words the earlier
/// one already has.
#[test]
fn a_later_segment_that_runs_past_the_earlier_window_keeps_its_new_words() -> TestResult {
    let window_0 = (0, 30 * SECOND);
    let window_1 = (25 * SECOND, 55 * SECOND);
    let first = with(
        draft(0, window_0, 10 * SECOND, 14 * SECOND, "intro words here")?,
        draft(
            0,
            window_0,
            22_320_000,
            29_500_000,
            "and then we open the settings page and click",
        )?,
    );
    let second = with(
        draft(
            1,
            window_1,
            25 * SECOND,
            32_440_000,
            "settings page and click the save button twice",
        )?,
        draft(
            1,
            window_1,
            32_440_000,
            39_720_000,
            "then the banner appears",
        )?,
    );
    let merged = merge_chunks(&[first, second]);
    let kept: Vec<&str> = merged
        .segments
        .iter()
        .map(|merged| merged.segment.text().text())
        .collect();
    assert_eq!(
        kept,
        [
            "intro words here",
            "and then we open the settings page and click",
            "the save button twice",
            "then the banner appears",
        ]
    );
    // The repeated words were trimmed from the later copy and counted.
    assert_eq!(
        merged
            .warnings
            .as_slice()
            .iter()
            .find(|warning| warning.kind() == TranscriptWarningKind::SeamDuplicatesRemoved)
            .map(|warning| warning.count()),
        Some(1)
    );
    // The kept segment keeps its own range, 2.94 s of it past the earlier one.
    assert_eq!(
        merged.segments[2].segment.range(),
        range(25 * SECOND, 32_440_000)?
    );
    Ok(())
}

/// #359, the mirror at a right edge. The earlier chunk's last segment
/// (22.0 to 29.9 s) is cut by its window's end (30 s). The later chunk (25 to
/// 55 s) hears the same sentence from 25.6 s, 0.6 s after its own window
/// starts, so its segment is not cut, and it spans the earlier segment's
/// midpoint (25.95 s). The merge used to keep only the later one, which cannot
/// have the words said before 25.6 s: the first 3.6 s of the earlier segment,
/// heard by no other chunk, were lost.
#[test]
fn an_earlier_segment_that_starts_before_the_later_window_keeps_its_first_words() -> TestResult {
    let window_0 = (0, 30 * SECOND);
    let window_1 = (25 * SECOND, 55 * SECOND);
    let first = with(
        draft(0, window_0, 10 * SECOND, 14 * SECOND, "before the seam")?,
        draft(
            0,
            window_0,
            22 * SECOND,
            29_900_000,
            "so first we log in and then we open",
        )?,
    );
    let second = with(
        draft(
            1,
            window_1,
            25_600_000,
            40 * SECOND,
            "and then we open the settings page",
        )?,
        draft(1, window_1, 41 * SECOND, 44 * SECOND, "after the seam")?,
    );
    let (kept, duplicates) = texts(&[first, second]);
    assert_eq!(
        kept,
        [
            "before the seam",
            "so first we log in and then we open",
            "the settings page",
            "after the seam",
        ]
    );
    assert_eq!(duplicates, 1);
    Ok(())
}

/// #359 in a chain of three chunks, windows 0 to 30, 25 to 55 and 50 to 80 s:
/// both seams used to lose the end of a segment that the later chunk hears
/// from its first sample. Chunk 1's first segment runs 2.94 s past chunk 0's
/// last, and chunk 2's first (50.0 to 57.0 s) runs 2.6 s past the 54.4 s at
/// which chunk 1's long segment ends (0.6 s before that window's end, so it is
/// not cut, and it spans the later segment's midpoint, 53.5 s).
#[test]
fn a_chain_of_chunks_loses_nothing_at_either_seam() -> TestResult {
    let window_0 = (0, 30 * SECOND);
    let window_1 = (25 * SECOND, 55 * SECOND);
    let window_2 = (50 * SECOND, 80 * SECOND);
    let first = draft(
        0,
        window_0,
        22_320_000,
        29_500_000,
        "one two three four five six",
    )?;
    let second = with(
        draft(
            1,
            window_1,
            25 * SECOND,
            32_440_000,
            "five six seven eight nine",
        )?,
        draft(
            1,
            window_1,
            32_440_000,
            54_400_000,
            "ten eleven twelve thirteen fourteen fifteen sixteen",
        )?,
    );
    let third = with(
        draft(
            2,
            window_2,
            50 * SECOND,
            57 * SECOND,
            "fifteen sixteen seventeen eighteen nineteen",
        )?,
        draft(
            2,
            window_2,
            58 * SECOND,
            62 * SECOND,
            "twenty twenty-one twenty-two",
        )?,
    );
    let (kept, duplicates) = texts(&[first, second, third]);
    assert_eq!(
        kept,
        [
            "one two three four five six",
            "seven eight nine",
            "ten eleven twelve thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen",
            "twenty twenty-one twenty-two",
        ]
    );
    assert_eq!(duplicates, 2);
    Ok(())
}

/// A segment that spans the whole overlap and more: 25.0 to 31.0 s is longer
/// than the 5 s overlap. The earlier chunk's segment 20.0 to 29.0 s spans its
/// midpoint (28.0 s) and is not cut, so the later one used to be dropped and
/// the 2 s after 29.0 s lost.
#[test]
fn a_segment_longer_than_the_overlap_that_starts_at_the_window_edge_is_kept() -> TestResult {
    let first = draft(
        0,
        (0, 30 * SECOND),
        20 * SECOND,
        29 * SECOND,
        "the first half of a long thought",
    )?;
    let second = draft(
        1,
        (25 * SECOND, 55 * SECOND),
        25 * SECOND,
        31 * SECOND,
        "long thought that goes on and on",
    )?;
    let (kept, duplicates) = texts(&[first, second]);
    assert_eq!(
        kept,
        ["the first half of a long thought", "that goes on and on"]
    );
    assert_eq!(duplicates, 1);
    // The same segment 0.4 s after the window's start is still cut by it, and
    // 0.6 s after it is not; both keep the words after the earlier one's end.
    for start in [25_400_000, 25_600_000] {
        let first = draft(
            0,
            (0, 30 * SECOND),
            20 * SECOND,
            29 * SECOND,
            "the first half of a long thought",
        )?;
        let second = draft(
            1,
            (25 * SECOND, 55 * SECOND),
            start,
            31 * SECOND,
            "long thought that goes on and on",
        )?;
        assert_eq!(
            texts(&[first, second]).0,
            ["the first half of a long thought", "that goes on and on"],
            "starting at {start}"
        );
    }
    Ok(())
}

/// The mirror's margin: a segment ending 0.4 s before the window's end is cut
/// by it, and one ending 0.6 s before it is not. Either way, the words said
/// before the later chunk's window starts are kept.
#[test]
fn the_edge_margin_does_not_decide_whether_speech_is_lost() -> TestResult {
    for end in [29_600_000, 29_400_000] {
        let first = draft(
            0,
            (0, 30 * SECOND),
            22 * SECOND,
            end,
            "so first we log in and then we open",
        )?;
        let second = draft(
            1,
            (25 * SECOND, 55 * SECOND),
            25_600_000,
            40 * SECOND,
            "and then we open the settings page",
        )?;
        assert_eq!(
            texts(&[first, second]).0,
            ["so first we log in and then we open", "the settings page"],
            "ending at {end}"
        );
    }
    Ok(())
}

/// What the guard tolerates is a derived bound, found here by search and not
/// assumed (#359). Over the R0 plan (windows 0 to 30 s and 25 to 55 s), the
/// earlier chunk's last segment is as close to its window's end as a segment can
/// be and still not be cut (it ends one edge margin before 30 s), and the later
/// chunk's first segment runs past that window's end by as little as one
/// microsecond. Rules 1 and 2 drop the later segment. The tolerance must restore
/// it, because what it carries past 30 s is audio the earlier chunk never heard;
/// the largest tolerance that does is the edge margin. The mirror at the right
/// edge (the earlier chunk's segment starts one microsecond before the later
/// window does, the later chunk's segment starts one margin after it) gives the
/// same bound.
#[test]
fn the_tolerance_is_the_largest_that_never_loses_speech_beyond_a_window_edge() -> TestResult {
    let window_0 = chunk(0, 0, 30 * SECOND)?;
    let window_1 = chunk(1, 25 * SECOND, 55 * SECOND)?;
    let edge = super::SEAM_EDGE_MICROS;
    // One chunk's segment over absolute microseconds, built through the
    // validation every real segment goes through.
    let provider =
        |planned: &PlannedChunk, from: u64, to: u64, words: &str| -> Built<ChunkSegments> {
            let origin = planned.window().start().as_micros();
            let segment = ProviderSegment {
                start: ChunkTime::from_micros(from - origin),
                end: ChunkTime::from_micros(to - origin),
                text: Some(text(words)?),
                tokens: Vec::new(),
            };
            Ok(validate_chunk_output(
                planned,
                planned.window(),
                range(0, 600 * SECOND)?,
                output(vec![segment]),
            )?
            .into_parts()
            .0)
        };
    let pairs = |loss_in_audio_only_the_later_chunk_heard: u64| -> Built<Vec<Vec<ChunkSegments>>> {
        let beyond = loss_in_audio_only_the_later_chunk_heard;
        Ok(vec![
            // Right edge of the earlier window: the later segment runs past it.
            vec![
                provider(&window_0, 22 * SECOND, 30 * SECOND - edge, "earlier")?,
                provider(&window_1, 25 * SECOND, 30 * SECOND + beyond, "later")?,
            ],
            // Left edge of the later window: the earlier segment starts before it.
            vec![
                provider(
                    &window_0,
                    25 * SECOND - beyond,
                    30 * SECOND - 100_000,
                    "earlier",
                )?,
                provider(&window_1, 25 * SECOND + edge, 40 * SECOND, "later")?,
            ],
        ])
    };
    // Whether the guard, with this tolerance, keeps every second of the audio
    // that only one chunk heard.
    let loses_nothing = |tolerance: u64| -> Built<bool> {
        for beyond in [1, 1_000, 250_000, 3_000_000] {
            for layout in pairs(beyond)? {
                // Each layout is two chunks, one segment each, merged by rules
                // 1 and 2 first.
                let mut kept = super::keep_by_rules_one_and_two(&layout);
                let before = kept.clone();
                super::restore_lost_speech_within(&layout, &mut kept, tolerance);
                // A segment of one chunk is the other's loss only when rules 1
                // and 2 dropped it, which they must have for the test to say
                // anything.
                assert_ne!(before.len(), 2, "rules 1 and 2 kept both: nothing to test");
                if kept.len() != 2 {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    };
    // Monotonic in the tolerance: a larger one restores no more than a smaller.
    let (mut low, mut high) = (0_u64, 4 * edge);
    assert!(loses_nothing(low)?);
    assert!(!loses_nothing(high)?);
    while high - low > 1 {
        let middle = low + (high - low) / 2;
        if loses_nothing(middle)? {
            low = middle;
        } else {
            high = middle;
        }
    }
    assert_eq!(low, edge);
    assert_eq!(low, SEAM_COVERAGE_TOLERANCE_MICROS);
    Ok(())
}

/// The boundary of the tolerance itself: a dropped segment whose uncovered end
/// is exactly the tolerance long is left (it is the recognisers' noise), and one
/// microsecond longer is restored.
#[test]
fn what_a_dropped_segment_carries_beyond_the_tolerance_is_restored_and_no_more() -> TestResult {
    let window_0 = chunk(0, 0, 30 * SECOND)?;
    let window_1 = chunk(1, 25 * SECOND, 55 * SECOND)?;
    let segment = |planned: &PlannedChunk, from: u64, to: u64| -> Built<ChunkSegments> {
        let origin = planned.window().start().as_micros();
        Ok(validate_chunk_output(
            planned,
            planned.window(),
            range(0, 600 * SECOND)?,
            output(vec![ProviderSegment {
                start: ChunkTime::from_micros(from - origin),
                end: ChunkTime::from_micros(to - origin),
                text: Some(text("words")?),
                tokens: Vec::new(),
            }]),
        )?
        .into_parts()
        .0)
    };
    let end_of_neighbour = 29_500_000;
    for (extra, restored) in [
        (0, false),
        (1, false),
        (SEAM_COVERAGE_TOLERANCE_MICROS - 1, false),
        (SEAM_COVERAGE_TOLERANCE_MICROS, false),
        (SEAM_COVERAGE_TOLERANCE_MICROS + 1, true),
        (2 * SECOND, true),
    ] {
        let layout = [
            segment(&window_0, 22 * SECOND, end_of_neighbour)?,
            segment(&window_1, 25 * SECOND, end_of_neighbour + extra)?,
        ];
        let merged = merge_chunks(&layout);
        assert_eq!(
            merged.segments.len(),
            if restored { 2 } else { 1 },
            "{extra} microseconds past the neighbour's end"
        );
    }
    Ok(())
}

/// The merge input of one generated layout.
struct Generated {
    chunks: Vec<ChunkSegments>,
    /// Each planned chunk with the audio it was decoded over, as a run records it.
    records: Vec<AsrChunkRecord>,
}

/// What the segments of a generated layout say.
#[derive(Clone, Copy)]
enum Words<'a> {
    /// Every segment has words of its own, so the repeat rule never acts.
    Unique,
    /// Two or three words from a vocabulary of two, chosen by these seeds (each
    /// below 16), so that neighbouring chunks repeat each other and the repeat
    /// rule acts.
    Repeated(&'a [u8]),
    /// Words of their own, except that the segments these seeds choose (about
    /// half) loop: one word said eight times, so that the loop rule (#539) acts.
    Looped(&'a [u8]),
}

/// The words of segment `number` of chunk `chunk`.
fn words_of(words: Words<'_>, chunk: u32, number: usize) -> String {
    const VOCABULARY: [&str; 2] = ["alpha", "beta"];
    let seed_of = |seeds: &[u8]| {
        let place = usize::try_from(chunk).unwrap_or(0) * 8 + number;
        seeds.get(place % seeds.len().max(1)).copied().unwrap_or(0)
    };
    let seed = match words {
        Words::Unique => return format!("c{chunk}s{number}"),
        Words::Looped(seeds) if seed_of(seeds) & 4 == 0 => return format!("c{chunk}s{number}"),
        Words::Looped(_) => return vec!["alpha"; 8].join(" "),
        Words::Repeated(seeds) => seed_of(seeds),
    };
    let bit = |position: u32| usize::from((seed >> position) & 1);
    (0..2 + bit(3))
        .map(|position| {
            let index = bit(u32::try_from(position).unwrap_or(0));
            VOCABULARY.get(index).copied().unwrap_or("alpha")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One chunk's generated answer: the delay before its first segment, then
/// (length, gap before the next) for each segment, and the start of each
/// rejected segment with words among them. The kind of a rejected segment is
/// decided by its start: an empty range, a reversed one, or one that ends beyond
/// the padded window, a third each.
type ChunkAnswer = (u64, Vec<(u64, u64)>, Vec<u64>);

fn chunk_answer() -> impl Strategy<Value = ChunkAnswer> {
    (
        // A first segment right at the window's edge, just inside the edge
        // margin, just outside it, or anywhere.
        prop_oneof![
            Just(0_u64),
            Just(300),
            Just(450),
            Just(500),
            Just(600),
            0_u64..4_000
        ],
        prop::collection::vec((200_u64..14_000, 0_u64..3_000), 0..7),
        // A rejected segment right at the start of the audio (a reversed one at
        // 1 ms, one that ends beyond the window at 2 and 5 ms), where an
        // earlier first sample puts its stretch before the window, or anywhere.
        prop::collection::vec(
            prop_oneof![Just(1_u64), Just(2), Just(5), 0_u64..30_000],
            0..3,
        ),
    )
}

/// The rejected segment with words that starts at `start` milliseconds, of the
/// kind that `start` selects.
fn rejected_by_kind(start: u64) -> Built<ProviderSegment> {
    match start % 3 {
        0 => provider_segment(start, start, "junk words"),
        1 => provider_segment(start, start - 1, "junk words"),
        _ => provider_segment(start, 40_000, "junk words"),
    }
}

/// Builds the merge input for an R0 plan over `total_ms` from generated
/// answers. `None` when a chunk's answer is unusable under the output rules,
/// which no case is about.
fn generated(total_ms: u64, answers: &[ChunkAnswer]) -> Built<Option<Generated>> {
    generated_with(0, total_ms, answers, 0, Words::Unique)
}

/// [`generated`] for a run that starts at `from_ms`, with the audio of every chunk
/// decoded from `early_ms` milliseconds before its window starts (the decoder's
/// first sample may be at an earlier frame boundary than the window asked for;
/// the first chunk of a run from 0 cannot start before 0), and with the given
/// words.
fn generated_with(
    from_ms: u64,
    total_ms: u64,
    answers: &[ChunkAnswer],
    early_ms: u64,
    words: Words<'_>,
) -> Built<Option<Generated>> {
    generated_with_plan(ChunkPlan::R0, from_ms, total_ms, answers, early_ms, words)
}

/// [`generated_with`] for any chunk plan (ADR 0033 cuts runs of smaller chunks
/// too).
fn generated_with_plan(
    plan: ChunkPlan,
    from_ms: u64,
    total_ms: u64,
    answers: &[ChunkAnswer],
    early_ms: u64,
    words: Words<'_>,
) -> Built<Option<Generated>> {
    let planned = plan_chunks(
        &segment_id()?,
        range(from_ms * 1_000, total_ms * 1_000)?,
        plan,
    )?;
    let (mut chunks, mut records) = (Vec::new(), Vec::new());
    for (planned, (delay, shape, junk)) in planned.iter().zip(answers) {
        let window = planned.window();
        let window_ms = window.duration_micros() / 1_000;
        let start = window.start().as_micros().saturating_sub(early_ms * 1_000);
        let audio = range(start, start + window.duration_micros())?;
        let mut provider = Vec::new();
        let mut cursor = *delay;
        for (number, (length, gap)) in shape.iter().enumerate() {
            if cursor >= window_ms {
                break;
            }
            let end = (cursor + length).min(window_ms);
            provider.push((
                cursor,
                provider_segment(cursor, end, &words_of(words, planned.index(), number))?,
            ));
            cursor = end + gap;
        }
        for start in junk.iter().filter(|start| **start < window_ms) {
            provider.push((*start, rejected_by_kind(*start)?));
        }
        provider.sort_by_key(|(start, _)| *start);
        let validated = validate_chunk_output(
            planned,
            audio,
            range(0, 600 * SECOND)?,
            output(provider.into_iter().map(|(_, segment)| segment).collect()),
        );
        let Ok(validated) = validated else {
            return Ok(None);
        };
        chunks.push(validated.into_parts().0);
        records.push(AsrChunkRecord::new(
            planned.clone(),
            AsrChunkOutcome::Transcribed { audio },
        ));
    }
    Ok(Some(Generated { chunks, records }))
}

/// The parts of the revision a run over `generated` would store if it kept no
/// segment and listed `unplaced`, over a source `total_ms` long: what a stored
/// revision makes of the stretches a merge reports.
fn parts_listing(
    generated: &Generated,
    total_ms: u64,
    unplaced: &[TimeRange],
) -> Built<TranscriptRevisionParts> {
    let run = AsrRun::new(AsrRunParts {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(DIGEST)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(DIGEST)?),
        decoding: AsrDecodingProfile::R0V1,
        plan: ChunkPlan::R0,
        threads: NonZeroU16::MIN,
        audio_stream: 1,
        chunks: generated.records.clone(),
    })?;
    Ok(TranscriptRevisionParts {
        id: TranscriptRevisionId::parse("trv_0123456789abcdef")?,
        number: NonZeroU32::MIN,
        source_id: SourceId::from_sha256(DIGEST)?,
        source_segment: SourceSegment::whole_file(
            segment_id()?,
            MediaTime::from_micros(total_ms * 1_000),
        )?,
        provenance: TranscriptProvenance::LocalAsr(run),
        supersedes: None,
        replaced_range: None,
        inherited: Vec::new(),
        carried_untranscribed: Vec::new(),
        unplaced: unplaced.to_vec(),
        language: None,
        segments: Vec::new(),
        warnings: TranscriptWarnings::default(),
    })
}

/// The longest stretch of `segment` that no segment of `kept` covers.
fn longest_uncovered(kept: &SpanSet, segment: &super::AsrSegmentDraft) -> u64 {
    kept.gaps_within(span(segment.range()))
        .iter()
        .map(|(start, end)| end - start)
        .max()
        .unwrap_or(0)
}

/// The span set of the segments a merge kept: used to ask what is covered.
fn kept_spans(segments: &[super::MergedSegment]) -> SpanSet {
    let mut kept = SpanSet::default();
    for segment in segments {
        kept.insert(span(segment.segment.range()));
    }
    kept
}

/// The holes of the transcript the guard is about: the parts of the union of
/// every validated segment of every chunk that no kept segment covers.
fn holes_of_the_union(chunks: &[ChunkSegments], kept: &SpanSet) -> Vec<(u64, u64)> {
    let union = merged(
        chunks
            .iter()
            .flat_map(|chunk| chunk.segments.iter().map(|segment| span(segment.range())))
            .collect(),
    );
    union
        .into_iter()
        .flat_map(|stretch| kept.gaps_within(stretch))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1_500,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// #359: over generated layouts of up to five chunks of the R0 plan, with
    /// segments hugging the window edges, the audio of a chunk decoded from up
    /// to 40 ms before its window (an earlier frame boundary; the run may start
    /// anywhere in its first 20 s, so that the first chunk is early too), and rejected
    /// segments of every kind among them (words and no words are not generated:
    /// every rejected segment has words). **Every segment has words of its own
    /// here, so the repeat rule (rule 3) never acts in this property**; the
    /// next one lets it act.
    ///
    /// - **no stretch longer than the tolerance of the union of every validated
    ///   segment is covered by no kept segment** (and so none of any one
    ///   segment), which the merge before the guard did not hold;
    /// - the merge invents nothing and keeps start order;
    /// - against a frozen copy of the merge as it was (`previous_merge`): where
    ///   that merge left no hole longer than the tolerance, this one gives the
    ///   very same segments and warnings, and where it did, this one keeps
    ///   every segment that one kept, in the same order, and adds only
    ///   segments (it restores, it never removes; **that holds where the repeat
    ///   rule does not act**, and
    ///   `a_repeat_beyond_the_first_pair_of_a_seam_is_trimmed` (a trim the old rule did not make)
    ///   is a case where it does);
    /// - whatever the merge reports as unplaced is uncovered, long enough to
    ///   report, and inside the (joined) stretches the chunks gave, **and a
    ///   stored revision accepts it** (the producer and the check of a stored
    ///   revision share one definition of where a stretch may lie).
    #[test]
    fn no_layout_of_chunks_loses_speech_at_a_seam(
        total_ms in 31_000_u64..=130_000,
        answers in prop::collection::vec(chunk_answer(), 5),
        from_ms in 0_u64..=20_000,
        early_ms in 0_u64..=40,
    ) {
        let Ok(Some(layout)) = generated_with(from_ms, total_ms, &answers, early_ms, Words::Unique) else {
            return Ok(());
        };
        let chunks = &layout.chunks;
        let merged_transcript = merge_chunks(chunks);
        let kept = kept_spans(&merged_transcript.segments);
        // The union has no hole beyond the tolerance, and so no segment has.
        for (start, end) in holes_of_the_union(chunks, &kept) {
            prop_assert!(
                end - start <= SEAM_COVERAGE_TOLERANCE_MICROS,
                "a hole of {} us at {} us",
                end - start,
                start
            );
        }
        for chunk in chunks {
            for segment in &chunk.segments {
                prop_assert!(longest_uncovered(&kept, segment) <= SEAM_COVERAGE_TOLERANCE_MICROS);
            }
        }
        // The merge invents nothing, and keeps order.
        for merged_segment in &merged_transcript.segments {
            let source = chunks
                .get(usize::try_from(merged_segment.chunk).unwrap_or(usize::MAX))
                .map(|chunk| chunk.segments.iter().any(|segment| {
                    segment.range() == merged_segment.segment.range()
                }));
            prop_assert_eq!(source, Some(true));
        }
        let in_order = merged_transcript
            .segments
            .windows(2)
            .all(|pair| pair[0].segment.range().start() <= pair[1].segment.range().start());
        prop_assert!(in_order);
        // Against the merge as it was.
        let before = previous_merge_chunks(chunks);
        let before_kept = kept_spans(&before.segments);
        let before_lost = holes_of_the_union(chunks, &before_kept)
            .iter()
            .any(|(start, end)| end - start > SEAM_COVERAGE_TOLERANCE_MICROS);
        if before_lost {
            // Everything the earlier merge kept is kept, in the same order.
            let mut now = merged_transcript.segments.iter();
            for old in &before.segments {
                prop_assert!(
                    now.any(|new| new.chunk == old.chunk && new.segment == old.segment),
                    "the earlier merge's segment {:?} is gone",
                    old.segment.range()
                );
            }
        } else {
            prop_assert_eq!(&merged_transcript.segments, &before.segments);
            prop_assert_eq!(&merged_transcript.warnings, &before.warnings);
        }
        // What is reported unplaced. Touching stretches of the chunks are joined
        // by the merge, so a reported one lies inside a joined one.
        let candidates = merged(
            chunks
                .iter()
                .flat_map(|chunk| chunk.unplaced.iter().copied().map(span))
                .collect(),
        );
        for stretch in &merged_transcript.unplaced {
            let (start, end) = span(*stretch);
            prop_assert!(end - start > SEAM_COVERAGE_TOLERANCE_MICROS);
            prop_assert!(kept.gaps_within((start, end)) == vec![(start, end)]);
            prop_assert!(candidates.iter().any(|(from, until)| *from <= start && end <= *until));
        }
        prop_assert!(merged_transcript.unplaced.windows(2).all(|pair| pair[0].end() < pair[1].start()));
        // A stored revision accepts what the merge reports, whatever the audio's
        // first sample was.
        let accepted = parts_listing(&layout, total_ms, &merged_transcript.unplaced)
            .is_ok_and(|parts| TranscriptRevision::new(parts).is_ok());
        prop_assert!(
            accepted,
            "a stored revision refuses the stretches {:?}",
            merged_transcript.unplaced
        );
    }

    /// #359, #380 and #382: the same layouts with a small vocabulary, so that the
    /// repeat rule (rule 3) acts. **Since #382 the guard's guarantee holds after
    /// that rule too**: the rule drops a copy only where what else is kept covers
    /// its time to within the tolerance, so
    ///
    /// - no stretch of the union longer than the tolerance is covered by no kept
    ///   segment, before the rule and after it (before #382 a stretch could be
    ///   left after it where no word was lost: L-157);
    /// - what the rule takes away is only a segment whose words a kept segment
    ///   repeats (they occur, in order, inside it, as rule 3 compares words).
    #[test]
    fn with_repeated_words_no_hole_is_left_and_only_repeats_are_dropped(
        total_ms in 31_000_u64..=130_000,
        answers in prop::collection::vec(chunk_answer(), 5),
        seeds in prop::collection::vec(0_u8..16, 40),
    ) {
        let Ok(Some(layout)) = generated_with(0, total_ms, &answers, 0, Words::Repeated(&seeds)) else {
            return Ok(());
        };
        let chunks = &layout.chunks;
        let before_the_rule = guarded_keep(chunks);
        let mut before_spans = SpanSet::default();
        for &(index, position) in &before_the_rule {
            if let Some(segment) = chunks.get(index).and_then(|chunk| chunk.segments.get(position)) {
                before_spans.insert(span(segment.range()));
            }
        }
        for (start, end) in holes_of_the_union(chunks, &before_spans) {
            prop_assert!(end - start <= SEAM_COVERAGE_TOLERANCE_MICROS);
        }
        let merged_transcript = merge_chunks(chunks);
        let kept = kept_spans(&merged_transcript.segments);
        for (start, end) in holes_of_the_union(chunks, &kept) {
            prop_assert!(
                end - start <= SEAM_COVERAGE_TOLERANCE_MICROS,
                "after the repeat rule a hole of {} us at {} us",
                end - start,
                start
            );
        }
        let repeats = dropped_by_the_repeat_rule(chunks, &before_the_rule, &merged_transcript);
        let kept_words: Vec<Vec<String>> = merged_transcript
            .segments
            .iter()
            .map(|kept| super::normalized_words(kept.segment.text().text()))
            .collect();
        for dropped in &repeats {
            let words = super::normalized_words(dropped.text().text());
            let repeated = words.len() >= 2
                && kept_words.iter().any(|kept| {
                    kept.windows(words.len())
                        .any(|window| super::same_seam_words(window, &words))
                });
            prop_assert!(
                repeated,
                "the repeat rule dropped {:?}, whose words no kept segment repeats",
                words
            );
        }
    }
}

/// What rules 1 and 2 and the guard keep, before the repeat rule: the
/// `(chunk, position)` of each segment.
fn guarded_keep(chunks: &[ChunkSegments]) -> BTreeSet<(usize, usize)> {
    let mut kept = super::keep_by_rules_one_and_two(chunks);
    super::restore_lost_speech(chunks, &mut kept);
    kept
}

/// The segments that were kept before the repeat rule and are not in `merged`:
/// the ones that rule dropped.
fn dropped_by_the_repeat_rule<'a>(
    chunks: &'a [ChunkSegments],
    before_the_rule: &BTreeSet<(usize, usize)>,
    merged_transcript: &super::MergedTranscript,
) -> Vec<&'a super::AsrSegmentDraft> {
    before_the_rule
        .iter()
        .filter_map(|&(index, position)| {
            chunks
                .get(index)?
                .segments
                .get(position)
                .map(|s| (index, s))
        })
        .filter(|(index, segment)| {
            !merged_transcript.segments.iter().any(|kept| {
                usize::try_from(kept.chunk) == Ok(*index) && kept.segment.range() == segment.range()
            })
        })
        .map(|(_, segment)| segment)
        .collect()
}

/// The property above is not vacuous: over a fixed pseudo-random sweep of the
/// same layouts, the guard acts in a real share of them (the earlier merge left
/// a hole beyond the tolerance there) and leaves a real share alone, and the
/// unplaced stretches it reports are non-empty in some. If a change to the
/// generator or the rules made the guard never act, the property would still
/// pass; this would not.
#[test]
fn the_generated_layouts_reach_the_guard() -> TestResult {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = |bound: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    let (mut acted, mut left_alone, mut unplaced) = (0_u32, 0_u32, 0_u32);
    for _ in 0..3_000 {
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
        let Some(layout) = generated(total_ms, &answers)? else {
            continue;
        };
        let by_rules = super::keep_by_rules_one_and_two(&layout.chunks);
        let mut guarded = by_rules.clone();
        super::restore_lost_speech(&layout.chunks, &mut guarded);
        if guarded == by_rules {
            left_alone += 1;
        } else {
            acted += 1;
        }
        unplaced += u32::from(!merge_chunks(&layout.chunks).unplaced.is_empty());
    }
    assert!(acted > 100, "{acted}");
    assert!(left_alone > 100, "{left_alone}");
    assert!(unplaced > 20, "{unplaced}");
    Ok(())
}

/// The repeated-vocabulary property is not vacuous either: over a fixed
/// pseudo-random sweep of its layouts the repeat rule trims or drops segments
/// that the guard had kept in a real share of them, and **since #382 it never leaves a
/// stretch longer than the tolerance in no kept segment** (before, it did in
/// more than 50 of these layouts: the case of L-157).
#[test]
fn the_repeated_vocabulary_layouts_reach_the_repeat_rule() -> TestResult {
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = |bound: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    let (mut dropped, mut acted, mut left_a_hole) = (0_u32, 0_u32, 0_u32);
    for _ in 0..3_000 {
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
        let Some(layout) = generated_with(0, total_ms, &answers, 0, Words::Repeated(&seeds))?
        else {
            continue;
        };
        let before_the_rule = guarded_keep(&layout.chunks);
        let merged_transcript = merge_chunks(&layout.chunks);
        let repeats =
            dropped_by_the_repeat_rule(&layout.chunks, &before_the_rule, &merged_transcript);
        dropped += u32::from(!repeats.is_empty());
        acted += u32::from(
            merged_transcript
                .warnings
                .as_slice()
                .iter()
                .any(|warning| warning.kind() == TranscriptWarningKind::SeamDuplicatesRemoved),
        );
        let kept = kept_spans(&merged_transcript.segments);
        left_a_hole += u32::from(
            holes_of_the_union(&layout.chunks, &kept)
                .iter()
                .any(|(start, end)| end - start > SEAM_COVERAGE_TOLERANCE_MICROS),
        );
    }
    // 13 and 86 of these layouts at the change of #380 and #382 (dropping a
    // copy that carries time is no longer allowed, so drops are rarer).
    assert!(dropped > 5, "{dropped}");
    assert!(acted > 50, "{acted}");
    assert_eq!(left_a_hole, 0);
    Ok(())
}

/// A stretch never leaves its chunk's window, even when the audio was decoded
/// from before it (#359, review): the decoder's first sample may be at an
/// earlier frame boundary than the window asked for, and a kept segment from
/// such audio is not checked against the window. A run over 10 to 40 s whose
/// audio began 20 ms early, with a rejected segment at 0 ms that ends beyond the
/// padded window and a kept one at 3 s, would otherwise list 9.98 to 12.98 s,
/// which a stored revision refuses, after every chunk had been transcribed.
#[test]
fn a_stretch_never_starts_before_its_chunks_window_when_the_audio_does() -> TestResult {
    let planned = chunk(0, 10 * SECOND, 40 * SECOND)?;
    let audio = range(10 * SECOND - 20_000, 40 * SECOND - 20_000)?;
    let validated = validate_chunk_output(
        &planned,
        audio,
        range(0, 70 * SECOND)?,
        output(vec![
            provider_segment(0, 40_000, "words that run on past the padded window")?,
            provider_segment(3_000, 3_500, "kept words")?,
        ]),
    )?;
    assert_eq!(validated.unplaced(), [range(10 * SECOND, 12_980_000)?]);
    // Audio that starts with its window is as it was.
    let validated = validate_chunk_output(
        &planned,
        planned.window(),
        range(0, 70 * SECOND)?,
        output(vec![
            provider_segment(0, 40_000, "words that run on past the padded window")?,
            provider_segment(3_000, 3_500, "kept words")?,
        ]),
    )?;
    assert_eq!(validated.unplaced(), [range(10 * SECOND, 13 * SECOND)?]);
    Ok(())
}

/// A smoke and performance test, and not a test of the guard (it passes with
/// the guard disabled): a long run of dense speech, 128 chunks of 250 segments
/// of 120 ms, so that every overlap holds a neighbour's segments at another
/// phase, merges, keeps what it has in order and invents nothing. The guard
/// keeps what is covered in a tree, not in a list it rebuilt for each segment,
/// so its work does not grow with the square of the segments (this run, about
/// 32,000 segments, took about 0.7 s in an unoptimised build when it was last
/// measured; nothing larger was measured for the guard that judges the union).
#[test]
fn a_long_run_of_dense_speech_merges_in_order_and_invents_nothing() -> TestResult {
    let total_ms = 25_000 * 127 + 30_000;
    let planned = plan_chunks(&segment_id()?, range(0, total_ms * 1_000)?, ChunkPlan::R0)?;
    assert_eq!(planned.len(), 128);
    let mut chunks = Vec::new();
    for planned in &planned {
        let window_ms = planned.window().duration_micros() / 1_000;
        let mut provider = Vec::new();
        // 250 segments of 120 ms, dense enough that every overlap holds copies
        // of its neighbour's segments, offset by the chunk's own start.
        for number in 0..250_u64 {
            let start = number * 120;
            if start + 100 < window_ms {
                provider.push(provider_segment(
                    start,
                    start + 100,
                    &format!("c{}s{number}", planned.index()),
                )?);
            }
        }
        chunks.push(
            validate_chunk_output(
                planned,
                planned.window(),
                range(0, 40_000 * SECOND)?,
                output(provider),
            )?
            .into_parts()
            .0,
        );
    }
    let merged_transcript = merge_chunks(&chunks);
    assert!(
        merged_transcript.segments.len() <= chunks.iter().map(|chunk| chunk.segments.len()).sum()
    );
    assert!(
        merged_transcript
            .segments
            .windows(2)
            .all(|pair| { pair[0].segment.range().start() <= pair[1].segment.range().start() })
    );
    // (The union has no hole beyond the tolerance either, though the guard is not
    // what this test is about.)
    assert!(
        holes_of_the_union(&chunks, &kept_spans(&merged_transcript.segments))
            .iter()
            .all(|(start, end)| end - start <= SEAM_COVERAGE_TOLERANCE_MICROS)
    );
    Ok(())
}

/// The guard alone, over segments that are given as `(chunk, start, end)` in
/// microseconds (chunk 0 has the window 0 to 30 s and chunk 1 25 to 55 s) and
/// the positions `(chunk, place in the chunk)` that rules 1 and 2 are taken to
/// have kept: what it keeps after it has run. A segment's place in its chunk is
/// the order in which the chunk's segments are given.
fn after_the_guard(
    segments: &[(u32, u64, u64)],
    kept: &[(usize, usize)],
) -> Built<Vec<(usize, usize)>> {
    let windows = [(0, 30 * SECOND), (25 * SECOND, 55 * SECOND)];
    let mut chunks: Vec<ChunkSegments> = vec![empty(0, windows[0])?, empty(1, windows[1])?];
    for (number, (chunk, start, end)) in segments.iter().enumerate() {
        let index = usize::try_from(*chunk)?;
        let draft = draft(
            *chunk,
            *windows.get(index).ok_or("no such chunk")?,
            *start,
            *end,
            &format!("s{number}"),
        )?;
        chunks[index].segments.extend(draft.segments);
    }
    let mut kept: BTreeSet<(usize, usize)> = kept.iter().copied().collect();
    super::restore_lost_speech(&chunks, &mut kept);
    Ok(kept.into_iter().collect())
}

/// The guard closes a hole with the fewest dropped segments that its order
/// needs, and not one more. The kept segment 20 to 24 s leaves a hole of 5 s
/// (24 to 29 s) in the union, and two dropped segments lie over it: 24 to 29 s,
/// which covers all of it, and 25 to 28 s, which covers most. The one that
/// overlaps the hole most is restored first, and then the other has nothing left
/// to cover. It fails when a restoration is not counted as covering (the other
/// is restored too), and when the order is reversed (the smaller is restored
/// first, and then the larger is needed as well).
#[test]
fn the_guard_restores_the_segment_that_covers_most_and_then_stops() -> TestResult {
    let kept = after_the_guard(
        &[
            (0, 20 * SECOND, 24 * SECOND),
            (0, 24 * SECOND, 29 * SECOND),
            (1, 25 * SECOND, 28 * SECOND),
        ],
        &[(0, 0)],
    )?;
    assert_eq!(kept, [(0, 0), (0, 1)]);
    // Three dropped segments over a hole of 6 s, none of which covers it alone:
    // each is needed, whatever the order they are listed in.
    let kept = after_the_guard(
        &[
            (0, 20 * SECOND, 24 * SECOND),
            (0, 26 * SECOND, 29 * SECOND),
            (1, 25 * SECOND, 27_500_000),
            (1, 27_500_000, 31 * SECOND),
        ],
        &[(0, 0)],
    )?;
    assert_eq!(kept, [(0, 0), (0, 1), (1, 0), (1, 1)]);
    // Two copies of the same range tie, and the earlier chunk's is restored.
    let kept = after_the_guard(
        &[
            (0, 20 * SECOND, 25 * SECOND),
            (0, 25 * SECOND, 29 * SECOND),
            (1, 25 * SECOND, 29 * SECOND),
        ],
        &[(0, 0)],
    )?;
    assert_eq!(kept, [(0, 0), (0, 1)]);
    Ok(())
}

/// A dropped segment that lies inside one the guard has already restored is not
/// restored too (#359, review): the kept segment 10 to 20 s leaves a hole of
/// 10 s (20 to 30 s), with four dropped segments over it, X 20 to 24 s, W 21 to
/// 23.5 s, V 24 to 26 s and U 26 to 30 s. X and U overlap the hole most (4 s)
/// and X starts first, so X is restored, then U; W lies wholly inside X, so the
/// piece of the hole that is left (24 to 26 s) does not overlap it and it is
/// skipped; V closes that piece. It fails when a candidate is restored without
/// asking whether it overlaps what is still a hole (W is restored too).
#[test]
fn a_dropped_segment_inside_one_already_restored_is_skipped() -> TestResult {
    let kept = after_the_guard(
        &[
            (0, 10 * SECOND, 20 * SECOND),
            (0, 20 * SECOND, 24 * SECOND),
            (0, 21 * SECOND, 23_500_000),
            (0, 24 * SECOND, 26 * SECOND),
            (0, 26 * SECOND, 30 * SECOND),
        ],
        &[(0, 0)],
    )?;
    assert_eq!(kept, [(0, 0), (0, 1), (0, 3), (0, 4)]);
    Ok(())
}

/// What the order pins (#359, review): the segment that overlaps a hole **most**
/// is restored first, and not the one that starts first. Over the hole 20 to
/// 30 s (after the kept 10 to 20 s) the dropped A, 20 to 21 s, starts first and
/// B, 20.5 to 30 s, overlaps most. Most-overlapping-first restores B alone, and
/// what is left of the hole (20 to 20.5 s) is within the tolerance; start order
/// would restore A and then B too, which writes the words of 20.5 to 21 s twice.
/// The reversed order is pinned by the test above; this one pins the choice
/// against plain start order. (In a reviewer's synthetic simulation, plain start
/// order gave 21,031 duplicated words against 18,339: the simulation is not in
/// the repository, so these figures are not reproducible from it.)
#[test]
fn the_guard_takes_the_most_overlapping_segment_first_and_not_the_first_to_start() -> TestResult {
    let kept = after_the_guard(
        &[
            (0, 10 * SECOND, 20 * SECOND),
            (0, 20 * SECOND, 21 * SECOND),
            (0, 20_500_000, 30 * SECOND),
        ],
        &[(0, 0)],
    )?;
    assert_eq!(kept, [(0, 0), (0, 2)]);
    Ok(())
}

/// The guard no longer exposes a segment to the repeat rule (#382). Chunk 0
/// (window 0 to 30 s) has 20.0 to 26.9 s "one two three" and 27.0 to 29.5 s
/// "alpha beta gamma"; chunk 1 (window 25 to 55 s) has 28.0 to 33.0 s "beta
/// gamma". The old merge keeps 20.0 to 26.9 s and 28.0 to 33.0 s and leaves 27
/// to 28 s (1 s) in no segment. The guard restores "alpha beta gamma" (it covers
/// that hole). Until #382 the repeat rule then dropped 28.0 to 33.0 s because its
/// words are inside the restored segment, and 29.5 to 33.0 s (3.5 s) was in no
/// segment; now that copy carries time no other segment covers, so it is kept,
/// and nothing longer than the tolerance is left.
#[test]
fn the_repeat_rule_no_longer_undoes_what_the_guard_restored() -> TestResult {
    let window_0 = (0, 30 * SECOND);
    let window_1 = (25 * SECOND, 55 * SECOND);
    let first = with(
        with(
            empty(0, window_0)?,
            draft(0, window_0, 20 * SECOND, 26_900_000, "one two three")?,
        ),
        draft(0, window_0, 27 * SECOND, 29_500_000, "alpha beta gamma")?,
    );
    let second = draft(1, window_1, 28 * SECOND, 33 * SECOND, "beta gamma")?;
    let chunks = [first, second];
    let ranges = |merged_transcript: &super::MergedTranscript| -> Vec<(u64, u64)> {
        merged_transcript
            .segments
            .iter()
            .map(|kept| {
                (
                    kept.segment.range().start().as_micros(),
                    kept.segment.range().end().as_micros(),
                )
            })
            .collect()
    };
    let old = previous_merge_as_transcript(&chunks);
    assert_eq!(
        ranges(&old),
        [(20 * SECOND, 26_900_000), (28 * SECOND, 33 * SECOND)]
    );
    let new = merge_chunks(&chunks);
    assert_eq!(
        ranges(&new),
        [
            (20 * SECOND, 26_900_000),
            (27 * SECOND, 29_500_000),
            (28 * SECOND, 33 * SECOND)
        ]
    );
    let kept = kept_spans(&new.segments);
    assert_eq!(
        kept.gaps_within((20 * SECOND, 33 * SECOND)),
        [(26_900_000, 27 * SECOND)]
    );
    Ok(())
}

/// The tolerance does not stack (#359, review). Chunk 0's last segments are
/// 20 to 27 s (kept) and 27.5 to 29 s (dropped); chunk 1's are 25 to 27.5 s
/// (dropped) and 28 to 33 s (kept). Each dropped copy, taken alone, leaves
/// exactly the tolerance uncovered, at the ends that face each other, and the
/// two together leave a hole of 1 s from 27 to 28 s in speech that both chunks
/// wrote words for. The guard is on the union, so it restores one of them, and
/// the hole that is left is no longer than the tolerance.
#[test]
fn the_tolerance_does_not_stack_into_a_longer_hole() -> TestResult {
    let window_0 = (0, 30 * SECOND);
    let window_1 = (25 * SECOND, 55 * SECOND);
    let first = with(
        draft(0, window_0, 20 * SECOND, 27 * SECOND, "k zero words")?,
        draft(0, window_0, 27_500_000, 29 * SECOND, "d zero words")?,
    );
    let second = with(
        draft(1, window_1, 25 * SECOND, 27_500_000, "d one words")?,
        draft(1, window_1, 28 * SECOND, 33 * SECOND, "k one words")?,
    );
    let chunks = [first, second];
    // Before the guard, the union had the hole of 1 s that is the point.
    let by_rules = super::keep_by_rules_one_and_two(&chunks);
    assert_eq!(by_rules.len(), 2, "rules 1 and 2 kept {by_rules:?}");
    let merged_transcript = merge_chunks(&chunks);
    let kept = kept_spans(&merged_transcript.segments);
    let holes = holes_of_the_union(&chunks, &kept);
    assert!(
        holes
            .iter()
            .all(|(start, end)| end - start <= SEAM_COVERAGE_TOLERANCE_MICROS),
        "{holes:?}"
    );
    // One copy was restored, not both.
    assert_eq!(merged_transcript.segments.len(), 3);
    Ok(())
}

/// A copy the guard restored is removed by the repeat rule when what is kept
/// covers its time (#380). The guard judges by time and not by words, so it can
/// restore a copy whose words the kept neighbour also wrote. Here the two chunks
/// disagree by 1 s on where the same last words end ("the file and save", 27.5
/// to 29.4 s in chunk 0 and 27.4 to 28.4 s in chunk 1): the merge before the
/// guard kept one copy, and the guard restores the other because 28.4 to 29.4 s
/// is covered by nothing kept. Until #380 the repeat rule compared only the last
/// kept segment of chunk 0 with the first of chunk 1, which are not this pair,
/// so the words stood twice; now every overlapping pair is compared, and chunk
/// 1's copy, whose time the other kept segments cover to within 0.1 s, goes.
#[test]
fn a_copy_the_guard_restored_is_removed_when_what_is_kept_covers_its_time() -> TestResult {
    let window_0 = (0, 30 * SECOND);
    let window_1 = (25 * SECOND, 55 * SECOND);
    let first = with(
        draft(
            0,
            window_0,
            20 * SECOND,
            24 * SECOND,
            "earlier sentence here",
        )?,
        draft(0, window_0, 27_500_000, 29_400_000, "the file and save")?,
    );
    let second = with(
        with(
            draft(1, window_1, 25_200_000, 27_400_000, "so first we open")?,
            draft(1, window_1, 27_400_000, 28_400_000, "the file and save")?,
        ),
        draft(1, window_1, 30 * SECOND, 34 * SECOND, "then we continue on")?,
    );
    let copies = |merged_transcript: &super::MergedTranscript| {
        merged_transcript
            .segments
            .iter()
            .filter(|merged_segment| merged_segment.segment.text().text() == "the file and save")
            .count()
    };
    let chunks = [first, second];
    assert_eq!(copies(&previous_merge_as_transcript(&chunks)), 1);
    let merged_transcript = merge_chunks(&chunks);
    assert_eq!(copies(&merged_transcript), 1);
    let kept = kept_spans(&merged_transcript.segments);
    for (start, end) in holes_of_the_union(&chunks, &kept) {
        assert!(end - start <= SEAM_COVERAGE_TOLERANCE_MICROS);
    }
    Ok(())
}

/// #485 (L-201), fixed by the maintainer's decision of 2026-10-20: a chunk that
/// writes one sentence twice in a row (a recogniser repetition) no longer hides
/// what the next chunk heard there. The layout is RM09's seam at 925 to 930 s
/// (hosted run 39466921335), shifted to start at 0: chunk 0 writes "i am sure
/// all will be well" over 0 to 22.5 s and again over 22.5 to 29.5 s; chunk 1
/// writes new words over 25.0 to 27.96 s, then goes on. That segment's midpoint
/// lies in chunk 0's core, and until the fix chunk 0's repeated copy counted as
/// hearing something there, so rule 1 dropped it, and the copy covered its time,
/// so the guard had no hole to close: its words were in no segment and a search
/// called the time transcribed. Now the repeated copy claims no time, so the
/// next chunk's words are kept; the repeated copy stays, a visible repeat.
#[test]
fn a_repeated_copy_in_one_chunk_no_longer_hides_the_next_chunks_words() -> TestResult {
    let window_0 = (0, 30 * SECOND);
    let window_1 = (25 * SECOND, 55 * SECOND);
    let first = with(
        draft(0, window_0, 0, 22_500_000, "i am sure all will be well")?,
        draft(
            0,
            window_0,
            22_500_000,
            29_500_000,
            "i am sure all will be well",
        )?,
    );
    let second = with(
        with(
            draft(
                1,
                window_1,
                25 * SECOND,
                27_960_000,
                "we shall continue human spaceflight",
            )?,
            draft(
                1,
                window_1,
                27_960_000,
                29_380_000,
                "yeah besides we will introduce you",
            )?,
        ),
        draft(
            1,
            window_1,
            29_380_000,
            31_060_000,
            "and we will see each other again",
        )?,
    );
    let chunks = [first, second];
    let words = |merged_transcript: &super::MergedTranscript| -> Vec<String> {
        merged_transcript
            .segments
            .iter()
            .map(|kept| kept.segment.text().text().to_owned())
            .collect()
    };
    // The merge before the fix (the earlier merge lost them too) dropped them.
    assert!(
        !words(&previous_merge_as_transcript(&chunks))
            .contains(&"we shall continue human spaceflight".to_owned())
    );
    let merged_transcript = merge_chunks(&chunks);
    let kept_words = words(&merged_transcript);
    assert!(
        kept_words.contains(&"we shall continue human spaceflight".to_owned()),
        "{kept_words:?}"
    );
    assert_eq!(
        kept_words
            .iter()
            .filter(|text| text.as_str() == "i am sure all will be well")
            .count(),
        2,
        "the repeated copy is kept: nothing is removed by the rule"
    );
    let kept = kept_spans(&merged_transcript.segments);
    for (start, end) in holes_of_the_union(&chunks, &kept) {
        assert!(end - start <= SEAM_COVERAGE_TOLERANCE_MICROS);
    }
    Ok(())
}

/// #485: a speaker who says the same thing twice, in two segments, keeps both,
/// and where the next chunk also heard the second saying the words are never
/// lost: the second saying claims no time, so the next chunk's copy is kept by
/// rule 1 and then trimmed by rule 3 as a repeat whose time is covered. Only
/// words that both chunks wrote are removed, never one of the two sayings.
#[test]
fn a_sentence_said_twice_is_kept_twice() -> TestResult {
    let window_0 = (0, 30 * SECOND);
    let window_1 = (25 * SECOND, 55 * SECOND);
    // Alone in one chunk: both sayings stay.
    let alone = with(
        draft(
            0,
            window_0,
            10 * SECOND,
            12 * SECOND,
            "we will go there now",
        )?,
        draft(
            0,
            window_0,
            12 * SECOND,
            14 * SECOND,
            "we will go there now",
        )?,
    );
    let (kept, _) = texts(&[alone, empty(1, window_1)?]);
    assert_eq!(kept, ["we will go there now", "we will go there now"]);
    // At a seam, with the next chunk's copy of the second saying.
    let first = with(
        draft(0, window_0, 23 * SECOND, 25_500_000, "we will go there now")?,
        draft(0, window_0, 25_500_000, 28 * SECOND, "we will go there now")?,
    );
    let second = with(
        draft(1, window_1, 25_600_000, 28_100_000, "we will go there now")?,
        draft(1, window_1, 28_100_000, 30 * SECOND, "and then we stop")?,
    );
    let (kept, _) = texts(&[first, second]);
    assert_eq!(
        kept.iter()
            .filter(|text| text.as_str() == "we will go there now")
            .count(),
        2,
        "{kept:?}"
    );
    assert!(kept.contains(&"and then we stop".to_owned()), "{kept:?}");
    Ok(())
}

/// #485: what counts as a repeat of the segment before. The same words (two or
/// more, one inner word allowed to differ among four or more) are; segments that
/// only share their first words, a one-word "no" said twice, a difference at the
/// first or last word, and a first segment are not. A segment that only shares a
/// prefix still claims its time, so the merge of such a layout is the earlier
/// merge's.
#[test]
fn only_a_segment_with_the_same_words_as_the_one_before_is_a_repeat() -> TestResult {
    let window_0 = (0, 30 * SECOND);
    let chunk_of = |pairs: &[(&str, &str)]| -> Built<Vec<bool>> {
        let mut answers = Vec::new();
        for (before, after) in pairs {
            let chunk = with(
                draft(0, window_0, 2 * SECOND, 4 * SECOND, before)?,
                draft(0, window_0, 4 * SECOND, 6 * SECOND, after)?,
            );
            answers.push(super::repeats_the_segment_before(&chunk, 1));
            assert!(
                !super::repeats_the_segment_before(&chunk, 0),
                "a first segment repeats nothing"
            );
        }
        Ok(answers)
    };
    assert_eq!(
        chunk_of(&[
            ("i am sure all will be well", "I am sure, all will be well."),
            ("we open the settings page", "we open the settings window"),
            ("we open the settings page", "we open the"),
            ("no", "no"),
            ("so we open the file now", "so we open a file now"),
            ("we open the file now", "they open the file now"),
            ("we open the file now", "we open the file again"),
        ])?,
        [true, false, false, false, true, false, false]
    );
    // A neighbour copy over a segment that only shares a prefix with the one
    // before it is dropped as before: the merge is the earlier merge's.
    let window_1 = (25 * SECOND, 55 * SECOND);
    let first = with(
        draft(0, window_0, 20 * SECOND, 24 * SECOND, "we open the")?,
        draft(0, window_0, 24 * SECOND, 29_500_000, "we open the file now")?,
    );
    let second = draft(1, window_1, 25 * SECOND, 28 * SECOND, "the file now")?;
    let chunks = [first, second];
    assert_eq!(
        merge_chunks(&chunks).segments,
        previous_merge_as_transcript(&chunks).segments
    );
    Ok(())
}

/// #539: a chunk heard in another language that loops one word over its whole
/// window no longer hides the words the chunk before it heard at their seam.
///
/// Modelled on RM03 at 75 to 80 s: chunk 3 wrote one segment over its whole
/// window, one word repeated fifty times over English speech, and rule 1
/// dropped chunk 2's last words in the overlap because the loop owned their
/// time; the guard saw no hole because the loop covered it. Now a segment that
/// loops on its own words claims no time, so the earlier chunk's words are
/// kept, and the loop stays, as any segment of its own chunk does.
#[test]
fn a_segment_that_loops_on_its_own_words_no_longer_hides_the_chunk_before() -> TestResult {
    let window_0 = (0, 30 * SECOND);
    let window_1 = (25 * SECOND, 55 * SECOND);
    let looped = vec!["okay"; 50].join(" ");
    let first = with(
        draft(
            0,
            window_0,
            20 * SECOND,
            25_500_000,
            "then we open the settings page",
        )?,
        draft(
            0,
            window_0,
            25_500_000,
            29_600_000,
            "and we choose the second option there",
        )?,
    );
    let second = draft(1, window_1, 25 * SECOND, 55 * SECOND, &looped)?;
    let chunks = [first, second];
    let words = |merged_transcript: &super::MergedTranscript| -> Vec<String> {
        merged_transcript
            .segments
            .iter()
            .map(|kept| kept.segment.text().text().to_owned())
            .collect()
    };
    // ADR 0033: each cut of this run still settles a prefix of its merge.
    assert_eq!(
        settled::every_cut_settles_a_prefix(&chunks).map(|_| ()),
        Ok(())
    );
    let hidden = "and we choose the second option there".to_owned();
    // The merge before the fix (the earlier merge lost them too) dropped them.
    assert!(!words(&previous_merge_as_transcript(&chunks)).contains(&hidden));
    let merged_transcript = merge_chunks(&chunks);
    let kept_words = words(&merged_transcript);
    assert!(kept_words.contains(&hidden), "{kept_words:?}");
    assert!(
        kept_words.contains(&looped),
        "the loop is kept: nothing is removed by the rule"
    );
    let kept = kept_spans(&merged_transcript.segments);
    for (start, end) in holes_of_the_union(&chunks, &kept) {
        assert!(end - start <= SEAM_COVERAGE_TOLERANCE_MICROS);
    }
    Ok(())
}

/// #539: what counts as a segment that loops on its own words. A unit of one to
/// four words said at least eight times in a row, making up at least half of
/// the segment's words, is; a word said a few times for emphasis, a loop that
/// is a small part of a longer segment, and ordinary speech are not.
#[test]
fn only_a_segment_mostly_made_of_one_repeated_unit_loops() -> TestResult {
    let window_0 = (0, 30 * SECOND);
    let answer = |text: &str| -> Built<bool> {
        let chunk = draft(0, window_0, 2 * SECOND, 10 * SECOND, text)?;
        Ok(chunk
            .segments
            .first()
            .is_some_and(super::loops_on_its_own_words))
    };
    let cases = [
        (vec!["okay"; 50].join(" "), true),
        (vec!["thank you"; 10].join(" "), true),
        (vec!["we will go there"; 8].join(" "), true),
        (
            format!("we open the file and {}", vec!["now"; 8].join(" ")),
            true,
        ),
        (vec!["okay"; 7].join(" "), false),
        ("no no no no".to_owned(), false),
        (vec!["we will go there now"; 9].join(" "), false),
        (
            format!(
                "we open the settings page and then we choose the second option {}",
                vec!["now"; 8].join(" ")
            ),
            false,
        ),
        ("then we open the settings page".to_owned(), false),
    ];
    let mut answers = Vec::new();
    for (text, _) in &cases {
        answers.push(answer(text)?);
    }
    let expected: Vec<bool> = cases.iter().map(|(_, loops)| *loops).collect();
    assert_eq!(answers, expected);
    Ok(())
}

/// The frozen merge as a [`super::MergedTranscript`], for comparisons that only
/// look at its segments.
fn previous_merge_as_transcript(chunks: &[ChunkSegments]) -> super::MergedTranscript {
    let before = previous_merge_chunks(chunks);
    super::MergedTranscript {
        segments: before.segments,
        warnings: before.warnings,
        unplaced: Vec::new(),
    }
}

/// The repeat rule keeps a restored copy that carries new time (#382). The
/// later chunk's 25.00 to 32.44 s ("yes yes") is restored by the guard because
/// 29.5 to 32.44 s is in no kept segment. Its words occur in the earlier segment
/// ("well yes yes okay"), and until #382 the repeat rule then dropped it whole,
/// so 29.5 to 32.44 s was in no segment and a search called it transcribed
/// (L-157); the merge before the guard lost that time too. Now it stays: "yes
/// yes" may be the same words or said again, and either way nothing is lost.
#[test]
fn the_repeat_rule_keeps_a_restored_copy_that_carries_new_time() -> TestResult {
    let window_0 = (0, 30 * SECOND);
    let window_1 = (25 * SECOND, 55 * SECOND);
    let first = draft(0, window_0, 22_320_000, 29_500_000, "well yes yes okay")?;
    let second = draft(1, window_1, 25 * SECOND, 32_440_000, "yes yes")?;
    let merged_transcript = merge_chunks(&[first, second]);
    assert_eq!(merged_transcript.segments.len(), 2);
    let kept = kept_spans(&merged_transcript.segments);
    assert_eq!(kept.gaps_within((22_320_000, 32_440_000)), []);
    Ok(())
}

/// The stretches of source time that validating `segments` (milliseconds into
/// the chunk, `(start, end, words)`) over a chunk whose window is `window`, whose
/// audio is `audio_ms` long from its start and inside `source` (seconds of
/// source time, `(start, end)`), leaves as possibly holding rejected words,
/// in milliseconds.
fn unplaced_in(
    window: (u64, u64),
    audio_ms: u64,
    source: (u64, u64),
    segments: &[(u64, u64, &str)],
) -> Built<Vec<(u64, u64)>> {
    let planned = chunk(0, window.0, window.1)?;
    let audio = range(window.0, window.0 + audio_ms * 1_000)?;
    let mut provider = Vec::new();
    for (start, end, words) in segments {
        provider.push(provider_segment(*start, *end, words)?);
    }
    let validated = validate_chunk_output(
        &planned,
        audio,
        range(source.0, source.1)?,
        output(provider),
    )?;
    Ok(validated
        .unplaced()
        .iter()
        .map(|stretch| {
            (
                stretch.start().as_micros() / 1_000,
                stretch.end().as_micros() / 1_000,
            )
        })
        .collect())
}

fn unplaced_by(
    window: (u64, u64),
    audio_ms: u64,
    segments: &[(u64, u64, &str)],
) -> Built<Vec<(u64, u64)>> {
    unplaced_in(window, audio_ms, (0, 600 * SECOND), segments)
}

/// Kept segments around a rejected `x`: one before it and three after, so that
/// one rejection among five text segments is tolerated.
fn around(x: (u64, u64, &str)) -> Vec<(u64, u64, &str)> {
    vec![
        (0, 1_500, "kept one"),
        x,
        (6_000, 8_000, "kept two"),
        (9_000, 10_000, "kept three"),
        (11_000, 12_000, "kept four"),
    ]
}

/// #359 and #383, every kind of rejected segment and what it leaves, in a chunk
/// of 30 s whose audio fills its window. A stretch runs from the segment's own
/// start to the next segment's start, whatever its range says; **since #383 an
/// empty range is placed too** (it was only counted, so its words could be
/// missing with no sign). Only text with no letter or digit leaves none.
#[test]
fn each_kind_of_rejected_segment_leaves_the_stretch_it_can_or_none() -> TestResult {
    type Kind<'a> = (&'a str, (u64, u64, &'a str), Vec<(u64, u64)>);
    let window = (0, 30 * SECOND);
    let table: [Kind; 7] = [
        // An empty range gives a start, and its words lie after it, before the
        // next thing the recogniser wrote.
        (
            "empty",
            (2_000, 2_000, "words nobody placed"),
            vec![(2_000, 6_000)],
        ),
        // A reversed range keeps its start: from there to the next segment.
        (
            "reversed",
            (3_000, 2_500, "reversed words"),
            vec![(3_000, 6_000)],
        ),
        // And never before it, whatever its end says.
        (
            "reversed to nothing",
            (2_000, 0, "an end of zero"),
            vec![(2_000, 6_000)],
        ),
        // An end beyond the padded window leaves the range positive, and the
        // start is the start.
        (
            "ends beyond the window",
            (2_000, 40_000, "ran on"),
            vec![(2_000, 6_000)],
        ),
        // Punctuation is not a word, in any kind.
        ("reversed without a word", (3_000, 2_500, "..."), vec![]),
        (
            "beyond the window without a word",
            (2_000, 40_000, "- ,"),
            vec![],
        ),
        ("empty without a word", (2_000, 2_000, "."), vec![]),
    ];
    for (label, x, expected) in table {
        assert_eq!(
            unplaced_by(window, 30_000, &around(x))?,
            expected,
            "{label}"
        );
    }
    Ok(())
}

/// #359 and #383, what follows a rejected segment decides where its words end: a
/// marker (it says that what follows is not speech) or another rejected segment
/// bound it as a kept segment does, and each rejected one has its own stretch.
/// **A rejected segment that is last in its chunk is bounded by the audio's
/// end** (#383; it was only counted, and a gap on a real recording matched it).
#[test]
fn what_follows_a_rejected_segment_bounds_its_words() -> TestResult {
    let window = (0, 30 * SECOND);
    // Followed by a marker.
    assert_eq!(
        unplaced_by(
            window,
            30_000,
            &[
                (0, 1_500, "kept one"),
                (2_000, 1_900, "reversed words"),
                (3_000, 4_000, "[BLANK_AUDIO]"),
                (9_000, 10_000, "kept three"),
                (11_000, 12_000, "kept four"),
            ],
        )?,
        [(2_000, 3_000)]
    );
    // Followed by another rejected segment: each has its own stretch.
    assert_eq!(
        unplaced_by(
            window,
            30_000,
            &[
                (0, 1_500, "kept one"),
                (2_000, 1_900, "first words"),
                (3_000, 2_900, "second words"),
                (6_000, 8_000, "kept two"),
                (9_000, 10_000, "kept three"),
                (11_000, 12_000, "kept four"),
                (13_000, 14_000, "kept five"),
                (15_000, 16_000, "kept six"),
            ],
        )?,
        [(2_000, 3_000), (3_000, 6_000)]
    );
    // Another segment at the same start leaves no stretch: the words lie at
    // that start, which the next segment answers for.
    assert_eq!(
        unplaced_by(
            window,
            30_000,
            &[
                (0, 1_500, "kept one"),
                (2_000, 1_900, "same start"),
                (2_000, 4_000, "kept two"),
                (9_000, 10_000, "kept three"),
                (11_000, 12_000, "kept four"),
            ],
        )?,
        []
    );
    // Last in the chunk: its words lie, at most, up to the audio's end.
    assert_eq!(
        unplaced_by(
            window,
            30_000,
            &[
                (0, 1_500, "kept one"),
                (6_000, 8_000, "kept two"),
                (9_000, 10_000, "kept three"),
                (11_000, 12_000, "kept four"),
                (20_000, 19_900, "a stray last word"),
            ],
        )?,
        [(20_000, 30_000)]
    );
    // Last with an empty range: the same.
    assert_eq!(
        unplaced_by(
            window,
            30_000,
            &[
                (0, 1_500, "kept one"),
                (6_000, 8_000, "kept two"),
                (9_000, 10_000, "kept three"),
                (11_000, 12_000, "kept four"),
                (24_880, 24_880, "the end of a sentence"),
            ],
        )?,
        [(24_880, 30_000)]
    );
    Ok(())
}

/// #359 and #383, the same rule at the end of a chunk's audio. A rejected
/// segment that starts at or after the audio's end names a time in the padding
/// of the 30 s window, so its words lie after the start of the latest segment
/// before it that starts inside the audio and before the audio's end; one
/// followed only by such segments is bounded by the audio's end. **The cliff
/// the earlier rule had is gone** (a next segment 0.1 s before the audio's end
/// gave a stretch of almost the whole rest of the chunk, one at its end gave
/// none): both now reach the audio's end, or that next segment's start.
#[test]
fn a_rejected_segment_near_the_audios_end_is_bounded_by_the_audios_end() -> TestResult {
    let window = (0, 30 * SECOND);
    // The audio is 5.001 s here. Kept segments first, then a tail.
    let kept = [
        (0, 1_000, "kept one"),
        (1_200, 2_000, "kept two"),
        (2_100, 3_000, "kept three"),
        (3_100, 4_000, "kept four"),
        (4_100, 4_200, "kept five"),
        (4_300, 4_400, "kept six"),
        (4_500, 4_600, "kept seven"),
        (4_700, 4_800, "kept eight"),
    ];
    let with = |tail: &[(u64, u64, &'static str)]| -> Vec<(u64, u64, &'static str)> {
        kept.iter().chain(tail).copied().collect()
    };
    let short = (0, 5 * SECOND);
    // Starting at or after the audio's end: after the last start inside the
    // audio (4.7 s), up to the audio's end, cut to the window's end (5 s); both
    // segments in the padding say the same.
    assert_eq!(
        unplaced_by(
            short,
            5_001,
            &with(&[
                (5_001, 4_900, "in the padding"),
                (7_100, 8_000, "more padding")
            ]),
        )?,
        [(4_700, 5_000), (4_700, 5_000)]
    );
    // Followed only by a segment that starts after the audio's end: up to the
    // audio's end (cut to the window), and that segment's words after the 4.9 s
    // start before it.
    assert_eq!(
        unplaced_by(
            short,
            5_001,
            &with(&[(4_900, 4_800, "a stray word"), (5_200, 6_000, "padding")]),
        )?,
        [(4_900, 5_000), (4_900, 5_000)]
    );
    // In a chunk whose audio fills its window of 30 s, a rejected segment at
    // 10 s followed by one that starts at 29.9 s, 0.1 s before the audio's end,
    // reaches 29.9 s; one followed by a segment that starts at the audio's end
    // reaches the audio's end, and that segment's own words lie after the start
    // before it.
    let padding = [
        (0, 500, "kept one"),
        (600, 1_100, "kept two"),
        (1_200, 1_700, "kept three"),
        (1_800, 2_300, "kept four"),
        (2_400, 2_900, "kept five"),
        (3_000, 3_500, "kept six"),
        (3_600, 4_100, "kept seven"),
        (4_200, 4_700, "kept eight"),
    ];
    let after_padding = |tail: &[(u64, u64, &'static str)]| -> Vec<(u64, u64, &'static str)> {
        padding.iter().chain(tail).copied().collect()
    };
    assert_eq!(
        unplaced_by(
            window,
            30_000,
            &after_padding(&[(10_000, 9_900, "words"), (29_900, 30_000, "last words")]),
        )?,
        [(10_000, 29_900)]
    );
    assert_eq!(
        unplaced_by(
            window,
            30_000,
            &after_padding(&[(10_000, 9_900, "words"), (30_000, 31_000, "in the padding")]),
        )?,
        [(10_000, 30_000), (10_000, 30_000)]
    );
    Ok(())
}

/// #359: the stretch is clipped to the source, at either end: a recogniser's
/// segment can be placed in audio that reaches beyond the source the run is
/// about, and a stretch beyond the source is not a stretch of this transcript.
#[test]
fn a_stretch_never_leaves_the_source() -> TestResult {
    let window = (0, 30 * SECOND);
    // The source ends at 10 s; the next segment starts at 14 s, outside it (and
    // is rejected itself), so the stretch is cut at 10 s.
    let ends_early = [
        (0, 500, "kept one"),
        (600, 1_100, "kept two"),
        (1_200, 1_700, "kept three"),
        (1_800, 2_300, "kept four"),
        (2_400, 2_900, "kept five"),
        (3_000, 3_500, "kept six"),
        (3_600, 4_100, "kept seven"),
        (4_200, 4_700, "kept eight"),
        (8_000, 7_900, "words past the clip"),
        (14_000, 15_000, "outside the source"),
    ];
    assert_eq!(
        unplaced_in(window, 30_000, (0, 10 * SECOND), &ends_early)?,
        [(8_000, 10_000)]
    );
    // The source starts at 5 s; the rejected segment starts before it, at 2 s
    // (so it is rejected whatever its range), and its stretch starts at 5 s.
    let starts_late = [
        (2_000, 1_900, "words before the source"),
        (8_000, 9_000, "kept one"),
        (9_500, 10_500, "kept two"),
        (11_000, 12_000, "kept three"),
        (12_500, 13_500, "kept four"),
    ];
    assert_eq!(
        unplaced_in(window, 30_000, (5 * SECOND, 600 * SECOND), &starts_late)?,
        [(5_000, 8_000)]
    );
    // And a range that ends past the source, though it starts inside it, is
    // rejected and keeps its stretch.
    let ends_past = [
        (0, 500, "kept one"),
        (600, 1_100, "kept two"),
        (1_200, 1_700, "kept three"),
        (2_000, 12_000, "runs past the source"),
        (6_000, 7_000, "kept four"),
    ];
    assert_eq!(
        unplaced_in(window, 30_000, (0, 10 * SECOND), &ends_past)?,
        [(2_000, 6_000)]
    );
    Ok(())
}

/// What the merge reports of the stretches the chunks mark: only what no kept
/// segment of any chunk covers, and only where that is longer than the
/// tolerance.
#[test]
fn the_merge_reports_a_marked_stretch_only_where_nothing_kept_covers_it() -> TestResult {
    let window_0 = (0, 30 * SECOND);
    let window_1 = (25 * SECOND, 55 * SECOND);
    let validated = |planned_index: u32,
                     window: (u64, u64),
                     segments: &[(u64, u64, &str)]|
     -> Built<ChunkSegments> {
        let planned = chunk(planned_index, window.0, window.1)?;
        let mut provider = Vec::new();
        for (start, end, words) in segments {
            provider.push(provider_segment(*start, *end, words)?);
        }
        Ok(validate_chunk_output(
            &planned,
            planned.window(),
            range(0, 600 * SECOND)?,
            output(provider),
        )?
        .into_parts()
        .0)
    };
    // Chunk 0 marks 26 s to 29 s (a rejected segment at 26 s, the next at 29 s),
    // and chunk 1, which starts at 25 s, has a kept segment from 26.5 s to
    // 28.0 s: what is left is 26.0 to 26.5 s, as long as the tolerance and so
    // not reported, and 28.0 to 29.0 s, which is.
    let first = validated(
        0,
        window_0,
        &[
            (1_000, 4_000, "kept one"),
            (5_000, 8_000, "kept two"),
            (26_000, 25_900, "unplaced words"),
            (29_000, 29_500, "kept three"),
        ],
    )?;
    let second = validated(
        1,
        window_1,
        &[(1_500, 3_000, "kept four"), (14_000, 16_000, "kept five")],
    )?;
    assert_eq!(first.unplaced.len(), 1);
    let merged_transcript = merge_chunks(&[first.clone(), second.clone()]);
    assert_eq!(
        merged_transcript
            .unplaced
            .iter()
            .map(|stretch| (stretch.start().as_micros(), stretch.end().as_micros()))
            .collect::<Vec<_>>(),
        [(28_000_000, 29_000_000)]
    );
    // With nothing from the second chunk over it, the whole stretch is.
    let alone = merge_chunks(&[first.clone(), empty(1, window_1)?]);
    assert_eq!(
        alone
            .unplaced
            .iter()
            .map(|stretch| (stretch.start().as_micros(), stretch.end().as_micros()))
            .collect::<Vec<_>>(),
        [(26_000_000, 29_000_000)]
    );
    // A kept segment over the whole stretch leaves none.
    let covering = validated(
        1,
        window_1,
        &[(500, 5_000, "kept four"), (14_000, 16_000, "kept five")],
    )?;
    assert_eq!(
        merge_chunks(&[first, covering]).unplaced,
        [] as [crate::timeline::TimeRange; 0]
    );
    Ok(())
}

/// #359, a run that rejects tens of thousands of segments (the bound of a run is
/// 1,024 chunks of up to 256 segments, a quarter of them rejected): the stretches
/// are bounded by joining the nearest, which marks more and never less, and the
/// join is not quadratic (it took minutes at this size when each join searched
/// the whole list). 80 chunks of 25 groups of a rejected segment and three kept
/// ones give about two thousand stretches.
#[test]
fn many_unplaced_stretches_are_bounded_by_joining_the_nearest() -> TestResult {
    let total_ms = 25_000 * 79 + 30_000;
    let planned = plan_chunks(&segment_id()?, range(0, total_ms * 1_000)?, ChunkPlan::R0)?;
    let mut chunks = Vec::new();
    for planned in &planned {
        let window_ms = planned.window().duration_micros() / 1_000;
        let mut provider = Vec::new();
        let mut at = 100_u64;
        while at + 1_200 < window_ms {
            provider.push(provider_segment(at, at - 1, "stray words")?);
            provider.push(provider_segment(at + 600, at + 700, "kept a")?);
            provider.push(provider_segment(at + 800, at + 900, "kept b")?);
            provider.push(provider_segment(at + 1_000, at + 1_100, "kept c")?);
            at += 1_200;
        }
        chunks.push(
            validate_chunk_output(
                planned,
                planned.window(),
                range(0, 40_000 * SECOND)?,
                output(provider),
            )?
            .into_parts()
            .0,
        );
    }
    let raw: usize = chunks.iter().map(|chunk| chunk.unplaced.len()).sum();
    let started = std::time::Instant::now();
    let merged_transcript = merge_chunks(&chunks);
    let elapsed = started.elapsed();
    assert!(raw > MAX_UNPLACED_STRETCHES, "{raw} raw stretches");
    assert_eq!(merged_transcript.unplaced.len(), MAX_UNPLACED_STRETCHES);
    // Joining only adds: everything that was uncovered in a stretch the chunks
    // marked is inside a reported stretch.
    let kept = kept_spans(&merged_transcript.segments);
    let mut reported = SpanSet::default();
    for stretch in &merged_transcript.unplaced {
        reported.insert(span(*stretch));
    }
    for chunk in &chunks {
        for candidate in &chunk.unplaced {
            for gap in kept.gaps_within(span(*candidate)) {
                if gap.1 - gap.0 > SEAM_COVERAGE_TOLERANCE_MICROS {
                    assert!(
                        reported.gaps_within(gap).is_empty(),
                        "{gap:?} is not reported"
                    );
                }
            }
        }
    }
    assert!(
        elapsed < std::time::Duration::from_secs(60),
        "merging {raw} stretches took {elapsed:?}"
    );
    Ok(())
}

fn run(outcomes: &[AsrChunkOutcome]) -> Built<AsrRun> {
    let planned = plan_chunks(&segment_id()?, range(0, 70 * SECOND)?, ChunkPlan::R0)?;
    Ok(AsrRun::new(AsrRunParts {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(DIGEST)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(DIGEST)?),
        decoding: AsrDecodingProfile::R0V1,
        plan: ChunkPlan::R0,
        threads: NonZeroU16::new(4).ok_or("zero")?,
        audio_stream: 1,
        chunks: planned
            .into_iter()
            .zip(outcomes)
            .map(|(chunk, outcome)| AsrChunkRecord::new(chunk, *outcome))
            .collect(),
    })?)
}

#[test]
fn runs_must_record_exactly_their_plan() -> TestResult {
    let transcribed = AsrChunkOutcome::Transcribed {
        audio: range(0, 30 * SECOND)?,
    };
    assert!(
        run(&[
            transcribed,
            AsrChunkOutcome::NoAudio,
            AsrChunkOutcome::NoAudio
        ])
        .is_ok()
    );
    // Two chunks are the whole plan of the shorter range they cover.
    assert!(run(&[transcribed, AsrChunkOutcome::NoAudio]).is_ok());
    let planned = plan_chunks(&segment_id()?, range(0, 70 * SECOND)?, ChunkPlan::R0)?;
    let mut parts = AsrRunParts {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(DIGEST)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(DIGEST)?),
        decoding: AsrDecodingProfile::R0V1,
        plan: ChunkPlan::R0,
        threads: NonZeroU16::MIN,
        audio_stream: 0,
        chunks: vec![AsrChunkRecord::new(chunk(1, 0, 30 * SECOND)?, transcribed)],
    };
    assert_eq!(
        AsrRun::new(parts.clone()),
        Err(TranscriptRevisionError::InvalidAsrRun)
    );
    // A missing middle chunk leaves a gap the plan does not have.
    parts.chunks = vec![
        AsrChunkRecord::new(planned[0].clone(), transcribed),
        AsrChunkRecord::new(planned[2].clone(), AsrChunkOutcome::NoAudio),
    ];
    assert_eq!(
        AsrRun::new(parts.clone()),
        Err(TranscriptRevisionError::InvalidAsrRun)
    );
    parts.chunks.clear();
    assert_eq!(
        AsrRun::new(parts),
        Err(TranscriptRevisionError::InvalidAsrRun)
    );
    assert!(Sha256Hex::parse(DIGEST.to_ascii_uppercase()).is_err());
    Ok(())
}

fn asr_segment(
    ordinal: u32,
    source: TimeRange,
    chunk: u32,
    provider: (u64, u64),
    trimmed: ProviderEndTrim,
    confidence: Confidence,
) -> Built<TranscriptSegment> {
    Ok(TranscriptSegment::new(TranscriptSegmentParts {
        id: TranscriptSegmentId::parse(format!("tsg_{ordinal:016x}"))?,
        ordinal: NonZeroU32::new(ordinal).ok_or("zero")?,
        range: source,
        text: text("recognised words")?,
        speaker: None,
        confidence,
        origin: SegmentOrigin::Asr {
            chunk,
            provider_start: ChunkTime::from_micros(provider.0),
            provider_end: ChunkTime::from_micros(provider.1),
            trimmed,
        },
    }))
}

fn asr_revision(segments: Vec<TranscriptSegment>) -> Built<TranscriptRevisionParts> {
    asr_revision_over(segments, range(25_750_000, 55 * SECOND)?)
}

/// [`asr_revision`] with chunk 1 decoded over `audio`.
fn asr_revision_over(
    segments: Vec<TranscriptSegment>,
    audio: TimeRange,
) -> Built<TranscriptRevisionParts> {
    let transcribed = AsrChunkOutcome::Transcribed { audio };
    Ok(TranscriptRevisionParts {
        id: TranscriptRevisionId::parse("trv_0123456789abcdef")?,
        number: NonZeroU32::MIN,
        source_id: SourceId::from_sha256(DIGEST)?,
        source_segment: SourceSegment::whole_file(
            segment_id()?,
            MediaTime::from_micros(70 * SECOND),
        )?,
        provenance: TranscriptProvenance::LocalAsr(run(&[
            AsrChunkOutcome::Silent {
                audio: range(0, 30 * SECOND)?,
            },
            transcribed,
            AsrChunkOutcome::NoAudio,
        ])?),
        supersedes: None,
        replaced_range: None,
        inherited: Vec::new(),
        carried_untranscribed: Vec::new(),
        unplaced: Vec::new(),
        language: None,
        segments,
        warnings: TranscriptWarnings::default(),
    })
}

/// A run that heard no speech is still a revision: its chunk outcomes are
/// the record of the attempt. An import with nothing in it is never one.
#[test]
fn a_local_asr_revision_may_hold_no_segment() -> TestResult {
    let revision = TranscriptRevision::new(asr_revision(Vec::new())?)?;
    assert_eq!(revision.segments(), []);
    assert_eq!(
        revision.provenance().alignment_origin().identifier(),
        "local_asr"
    );
    let mut import = asr_revision(Vec::new())?;
    import.provenance = TranscriptProvenance::Imported {
        format: TranscriptFormat::Srt,
        sidecar: SidecarIdentity::new(DIGEST, 10)?,
        offset: TranscriptOffset::ZERO,
    };
    assert_eq!(
        TranscriptRevision::new(import),
        Err(TranscriptRevisionError::Empty)
    );
    Ok(())
}

const BASE_REVISION: &str = "trv_1111111111111111";

/// A cue of an imported base revision, offset by +0.5 s, carried into a
/// spliced revision at `[start, end)`.
fn carried_cue(ordinal: u32, start: u64, end: u64, original: u32) -> Built<TranscriptSegment> {
    Ok(TranscriptSegment::new(TranscriptSegmentParts {
        id: TranscriptSegmentId::parse(format!("tsg_{ordinal:016x}"))?,
        ordinal: NonZeroU32::new(ordinal).ok_or("zero")?,
        range: range(start, end)?,
        text: text("carried words")?,
        speaker: None,
        confidence: Confidence::unknown(),
        origin: SegmentOrigin::ImportedCue {
            cue: CueSource::new(
                NonZeroU32::new(original).ok_or("zero")?,
                NonZeroU32::new(original * 4).ok_or("zero")?,
            ),
            timing: CueTiming::new(start - 500_000, end - 500_000)?,
        },
    })
    .with_carried_from(CarriedFrom::new(
        TranscriptRevisionId::parse(BASE_REVISION)?,
        TranscriptSegmentId::parse(format!("tsg_{:016x}", 0xb00_u64 + u64::from(original)))?,
    )))
}

fn inherited_import() -> Built<InheritedRevision> {
    Ok(InheritedRevision::new(
        TranscriptRevisionId::parse(BASE_REVISION)?,
        TranscriptProvenance::Imported {
            format: TranscriptFormat::Srt,
            sidecar: SidecarIdentity::new(DIGEST, 10)?,
            offset: TranscriptOffset::from_micros(500_000)?,
        },
        Some(LanguageTag::parse("en")?),
    ))
}

/// A revision replacing [25 s, 55 s) of the base with chunk 1's speech and
/// carrying one base cue from before the range and one after it.
fn spliced(before: TranscriptSegment, after: TranscriptSegment) -> Built<TranscriptRevisionParts> {
    let own = asr_segment(
        2,
        range(26_750_000, 28_750_000)?,
        1,
        (SECOND, 3 * SECOND),
        ProviderEndTrim::Unchanged,
        Confidence::unknown(),
    )?;
    let mut parts = asr_revision(vec![before, own, after])?;
    parts.supersedes = Some(TranscriptRevisionId::parse(BASE_REVISION)?);
    parts.replaced_range = Some(range(25 * SECOND, 55 * SECOND)?);
    parts.inherited = vec![inherited_import()?];
    Ok(parts)
}

/// D3/T-06: carried segments keep their original provenance, are checked
/// against it, and never lie in the replaced range.
#[test]
fn spliced_revisions_check_carried_segments_against_their_origin() -> TestResult {
    let before = carried_cue(1, SECOND, 3 * SECOND, 1)?;
    let after = carried_cue(3, 60 * SECOND, 62 * SECOND, 7)?;
    let revision = TranscriptRevision::new(spliced(before.clone(), after.clone())?)?;
    let carried = &revision.segments()[0];
    assert!(matches!(
        revision.segment_provenance(carried),
        TranscriptProvenance::Imported { .. }
    ));
    assert_eq!(
        revision.segment_language(carried).map(LanguageTag::as_str),
        Some("en")
    );
    let own = &revision.segments()[1];
    assert!(own.carried_from().is_none());
    assert!(matches!(
        revision.segment_provenance(own),
        TranscriptProvenance::LocalAsr(_)
    ));
    assert_eq!(revision.segment_language(own), None);

    let invalid = |parts: TranscriptRevisionParts| TranscriptRevision::new(parts);
    // A carried segment inside the replaced range.
    let inside = carried_cue(3, 30 * SECOND, 31 * SECOND, 7)?;
    assert_eq!(
        invalid(spliced(before.clone(), inside)?),
        Err(TranscriptRevisionError::InvalidCarriedSegment)
    );
    // A carried segment whose timing is not its cue plus the inherited offset.
    let shifted = TranscriptSegment::new(TranscriptSegmentParts {
        id: after.id().clone(),
        ordinal: NonZeroU32::new(3).ok_or("zero")?,
        range: range(60 * SECOND + 1, 62 * SECOND)?,
        text: after.text().clone(),
        speaker: None,
        confidence: Confidence::unknown(),
        origin: after.origin(),
    })
    .with_carried_from(after.carried_from().ok_or("carried")?.clone());
    assert_eq!(
        invalid(spliced(before.clone(), shifted)?),
        Err(TranscriptRevisionError::AlignmentMismatch)
    );
    // A speaker label on text carried from SubRip, which has no voice syntax.
    let labelled = TranscriptSegment::new(TranscriptSegmentParts {
        id: after.id().clone(),
        ordinal: NonZeroU32::new(3).ok_or("zero")?,
        range: after.range(),
        text: after.text().clone(),
        speaker: Some(SpeakerLabel::parse("Ana")?),
        confidence: Confidence::unknown(),
        origin: after.origin(),
    })
    .with_carried_from(after.carried_from().ok_or("carried")?.clone());
    assert_eq!(
        invalid(spliced(before.clone(), labelled)?),
        Err(TranscriptRevisionError::UnsupportedSpeaker)
    );
    // The same original segment carried twice.
    let twice = carried_cue(3, 60 * SECOND, 62 * SECOND, 1)?;
    assert_eq!(
        invalid(spliced(before.clone(), twice)?),
        Err(TranscriptRevisionError::InvalidCarriedSegment)
    );
    // A carried segment naming a revision with no inherited provenance.
    let mut unknown = spliced(before.clone(), after.clone())?;
    unknown.inherited = vec![InheritedRevision::new(
        TranscriptRevisionId::parse("trv_2222222222222222")?,
        inherited_import()?.provenance().clone(),
        None,
    )];
    assert_eq!(
        invalid(unknown),
        Err(TranscriptRevisionError::InvalidCarriedSegment)
    );
    // Inherited provenance nobody refers to.
    let mut dangling = spliced(before.clone(), after.clone())?;
    dangling.inherited.push(InheritedRevision::new(
        TranscriptRevisionId::parse("trv_2222222222222222")?,
        inherited_import()?.provenance().clone(),
        None,
    ));
    assert_eq!(
        invalid(dangling),
        Err(TranscriptRevisionError::InvalidCarriedSegment)
    );
    // Carrying needs a superseded revision and a replaced range.
    let mut unanchored = spliced(before.clone(), after.clone())?;
    unanchored.replaced_range = None;
    assert_eq!(
        invalid(unanchored),
        Err(TranscriptRevisionError::InvalidCarriedSegment)
    );
    let mut orphaned = spliced(before, after)?;
    orphaned.supersedes = None;
    orphaned.replaced_range = None;
    assert_eq!(
        invalid(orphaned),
        Err(TranscriptRevisionError::InvalidCarriedSegment)
    );
    Ok(())
}

/// #274: a stored segment is checked with the same rule that made it. An end
/// past the audio is valid when it is marked as cut at the audio's end and lies
/// inside the sanity bound (the padded 30 s window, or a second past the audio
/// where that is later: every end 0.1.0 accepted is still valid); an end beyond
/// the bound, an unmarked overrun, a cut mark on an end inside the audio and a
/// start at or after the audio's end stay refused.
#[test]
fn a_stored_segment_ending_past_its_audio_is_valid_only_when_marked_cut() -> TestResult {
    // Chunk 1 decoded from 25.75 s to 55 s (29.25 s of audio): the bound is
    // 30.25 s, a second past the audio, because the window is shorter.
    for end in [29_300_000, 30 * SECOND, 30_250_000] {
        let ran_on = asr_segment(
            1,
            range(54 * SECOND, 55 * SECOND)?,
            1,
            (28_250_000, end),
            ProviderEndTrim::TrimmedToAudioEnd,
            Confidence::unknown(),
        )?;
        TranscriptRevision::new(asr_revision(vec![ran_on])?)?;
    }
    for end in [30_250_001, 33 * SECOND, 59 * SECOND] {
        let beyond = asr_segment(
            1,
            range(54 * SECOND, 55 * SECOND)?,
            1,
            (28_250_000, end),
            ProviderEndTrim::TrimmedToAudioEnd,
            Confidence::unknown(),
        )?;
        assert_eq!(
            TranscriptRevision::new(asr_revision(vec![beyond])?),
            Err(TranscriptRevisionError::AlignmentMismatch),
            "{end}"
        );
    }
    // A range cut mid-speech: chunk 1 decoded 5 s (25.75 s to 30.75 s). Its
    // last segment's provider end ran on to 7 s, which 0.1.0 refused; it is
    // valid up to the 30 s window and refused beyond it.
    let short = range(25_750_000, 30_750_000)?;
    for end in [5_500_000, 7 * SECOND, 30 * SECOND] {
        let ran_on = asr_segment(
            1,
            range(29_750_000, 30_750_000)?,
            1,
            (4 * SECOND, end),
            ProviderEndTrim::TrimmedToAudioEnd,
            Confidence::unknown(),
        )?;
        TranscriptRevision::new(asr_revision_over(vec![ran_on], short)?)?;
    }
    for end in [30_000_001, 45 * SECOND] {
        let beyond = asr_segment(
            1,
            range(29_750_000, 30_750_000)?,
            1,
            (4 * SECOND, end),
            ProviderEndTrim::TrimmedToAudioEnd,
            Confidence::unknown(),
        )?;
        assert_eq!(
            TranscriptRevision::new(asr_revision_over(vec![beyond], short)?),
            Err(TranscriptRevisionError::AlignmentMismatch),
            "{end}"
        );
    }
    let unmarked = asr_segment(
        1,
        range(54 * SECOND, 55 * SECOND)?,
        1,
        (28_250_000, 33 * SECOND),
        ProviderEndTrim::Unchanged,
        Confidence::unknown(),
    )?;
    let marked_inside = asr_segment(
        1,
        range(54 * SECOND, 55 * SECOND)?,
        1,
        (28_250_000, 29_000_000),
        ProviderEndTrim::TrimmedToAudioEnd,
        Confidence::unknown(),
    )?;
    let starts_after = asr_segment(
        1,
        range(54_500_000, 55 * SECOND)?,
        1,
        (29_250_000, 31 * SECOND),
        ProviderEndTrim::TrimmedToAudioEnd,
        Confidence::unknown(),
    )?;
    for refused in [unmarked, marked_inside, starts_after] {
        assert_eq!(
            TranscriptRevision::new(asr_revision(vec![refused])?),
            Err(TranscriptRevisionError::AlignmentMismatch)
        );
    }
    Ok(())
}

/// T-06: an ASR segment's range is re-derived from its chunk and provider times.
#[test]
fn asr_segments_are_validated_against_their_own_chunk() -> TestResult {
    let uncalibrated = Confidence::provider_score(8_000, ConfidenceOrigin::ProviderUncalibrated)?;
    // Chunk 1 decoded from 25.75 s to 55 s (29.25 s of audio).
    let valid = asr_segment(
        1,
        range(26_750_000, 28_750_000)?,
        1,
        (SECOND, 3 * SECOND),
        ProviderEndTrim::Unchanged,
        uncalibrated,
    )?;
    let trimmed = asr_segment(
        2,
        range(54 * SECOND, 55 * SECOND)?,
        1,
        (28_250_000, 29_900_000),
        ProviderEndTrim::TrimmedToAudioEnd,
        Confidence::unknown(),
    )?;
    let revision = TranscriptRevision::new(asr_revision(vec![valid.clone(), trimmed])?)?;
    assert_eq!(revision.origin().identifier(), "local_asr");

    let shifted = asr_segment(
        1,
        range(26_750_001, 28_750_000)?,
        1,
        (SECOND, 3 * SECOND),
        ProviderEndTrim::Unchanged,
        uncalibrated,
    )?;
    assert_eq!(
        TranscriptRevision::new(asr_revision(vec![shifted])?),
        Err(TranscriptRevisionError::AlignmentMismatch)
    );
    for silent_chunk in [0, 2, 3] {
        let misplaced = asr_segment(
            1,
            range(26_750_000, 28_750_000)?,
            silent_chunk,
            (SECOND, 3 * SECOND),
            ProviderEndTrim::Unchanged,
            uncalibrated,
        )?;
        assert_eq!(
            TranscriptRevision::new(asr_revision(vec![misplaced])?),
            Err(TranscriptRevisionError::OriginMismatch)
        );
    }
    let calibrated = asr_segment(
        1,
        range(26_750_000, 28_750_000)?,
        1,
        (SECOND, 3 * SECOND),
        ProviderEndTrim::Unchanged,
        Confidence::provider_score(8_000, ConfidenceOrigin::ProviderCalibrated)?,
    )?;
    assert_eq!(
        TranscriptRevision::new(asr_revision(vec![calibrated])?),
        Err(TranscriptRevisionError::ManufacturedConfidence)
    );

    let mut superseding = asr_revision(vec![valid])?;
    superseding.replaced_range = Some(range(25 * SECOND, 55 * SECOND)?);
    assert_eq!(
        TranscriptRevision::new(superseding.clone()),
        Err(TranscriptRevisionError::InvalidSupersession)
    );
    superseding.supersedes = Some(TranscriptRevisionId::parse("trv_fedcba9876543210")?);
    let revision = TranscriptRevision::new(superseding.clone())?;
    assert_eq!(
        revision.replaced_range(),
        Some(range(25 * SECOND, 55 * SECOND)?)
    );
    superseding.supersedes = Some(superseding.id.clone());
    assert_eq!(
        TranscriptRevision::new(superseding),
        Err(TranscriptRevisionError::InvalidSupersession)
    );
    Ok(())
}

/// #353: an unusable chunk is recorded in the run and holds no segment. A
/// revision may carry one beside chunks that were transcribed, and a segment
/// that names it is refused, as one that names a silent chunk is.
#[test]
fn an_unusable_chunk_is_recorded_and_holds_no_segment() -> TestResult {
    let uncalibrated = Confidence::provider_score(8_000, ConfidenceOrigin::ProviderUncalibrated)?;
    let chunks = run(&[
        AsrChunkOutcome::Transcribed {
            audio: range(0, 30 * SECOND)?,
        },
        AsrChunkOutcome::Unusable {
            audio: range(25_750_000, 55 * SECOND)?,
        },
        AsrChunkOutcome::NoAudio,
    ])?;
    let in_transcribed = asr_segment(
        1,
        range(SECOND, 3 * SECOND)?,
        0,
        (SECOND, 3 * SECOND),
        ProviderEndTrim::Unchanged,
        uncalibrated,
    )?;
    let mut parts = asr_revision(vec![in_transcribed])?;
    parts.provenance = TranscriptProvenance::LocalAsr(chunks.clone());
    let revision = TranscriptRevision::new(parts)?;
    assert_eq!(revision.segments().len(), 1);

    let in_unusable = asr_segment(
        1,
        range(26_750_000, 28_750_000)?,
        1,
        (SECOND, 3 * SECOND),
        ProviderEndTrim::Unchanged,
        uncalibrated,
    )?;
    let mut parts = asr_revision(vec![in_unusable])?;
    parts.provenance = TranscriptProvenance::LocalAsr(chunks);
    assert_eq!(
        TranscriptRevision::new(parts),
        Err(TranscriptRevisionError::OriginMismatch)
    );
    Ok(())
}

/// #353: the gaps of a run are what its unusable chunks leave unread: the
/// windows of the unusable chunks less the windows of every chunk that was
/// answered (transcribed, silent or without audio), because the five seconds
/// two neighbours share are read by whichever of them was answered.
#[test]
fn the_gaps_of_a_run_are_its_unusable_windows_less_the_answered_ones() -> TestResult {
    let transcribed = |start: u64, end: u64| -> Built<AsrChunkOutcome> {
        Ok(AsrChunkOutcome::Transcribed {
            audio: range(start * SECOND, end * SECOND)?,
        })
    };
    let unusable = |start: u64, end: u64| -> Built<AsrChunkOutcome> {
        Ok(AsrChunkOutcome::Unusable {
            audio: range(start * SECOND, end * SECOND)?,
        })
    };
    // The plan of the test run is the windows 0-30, 25-55 and 50-70 s.
    let none = run(&[
        transcribed(0, 30)?,
        transcribed(25, 55)?,
        transcribed(50, 70)?,
    ])?;
    assert_eq!(none.unusable_gaps(), []);
    // The middle chunk alone: both neighbours read their own five seconds of it.
    let middle = run(&[transcribed(0, 30)?, unusable(25, 55)?, transcribed(50, 70)?])?;
    assert_eq!(middle.unusable_gaps(), [range(30 * SECOND, 50 * SECOND)?]);
    // The first chunk alone, its neighbour without audio still an answer.
    let first = run(&[
        unusable(0, 30)?,
        AsrChunkOutcome::NoAudio,
        AsrChunkOutcome::NoAudio,
    ])?;
    assert_eq!(first.unusable_gaps(), [range(0, 25 * SECOND)?]);
    // A silent neighbour is an answer too: the shared seconds are not a gap.
    let beside_silence = run(&[
        unusable(0, 30)?,
        AsrChunkOutcome::Silent {
            audio: range(25 * SECOND, 55 * SECOND)?,
        },
        transcribed(50, 70)?,
    ])?;
    assert_eq!(beside_silence.unusable_gaps(), [range(0, 25 * SECOND)?]);
    // Two unusable neighbours leave one gap, merged, with nothing inside it.
    let both = run(&[unusable(0, 30)?, unusable(25, 55)?, transcribed(50, 70)?])?;
    assert_eq!(both.unusable_gaps(), [range(0, 50 * SECOND)?]);
    // Nothing answered: the whole range.
    let all = run(&[unusable(0, 30)?, unusable(25, 55)?, unusable(50, 70)?])?;
    assert_eq!(all.unusable_gaps(), [range(0, 70 * SECOND)?]);
    Ok(())
}

/// #353, the rule for the text a run that could not read all of its range keeps
/// of the earlier revision: kept, whole, when it reaches into a gap and none of
/// the run's own text overlaps it; every other segment in the range is
/// replaced; a segment outside the range is carried.
#[test]
fn the_rule_keeps_text_that_reaches_into_a_gap_unless_the_runs_own_text_overlaps_it() -> TestResult
{
    let outcomes = [
        AsrChunkOutcome::Unusable {
            audio: range(0, 30 * SECOND)?,
        },
        AsrChunkOutcome::Transcribed {
            audio: range(25 * SECOND, 55 * SECOND)?,
        },
        AsrChunkOutcome::Transcribed {
            audio: range(50 * SECOND, 70 * SECOND)?,
        },
    ];
    let planned = run(&outcomes)?;
    // The gap is 0-25 s. The run replaced 0-65 s and wrote 26-28 s and 40-41 s.
    assert_eq!(planned.unusable_gaps(), [range(0, 25 * SECOND)?]);
    let rule = EarlierTextRule::new(
        range(0, 65 * SECOND)?,
        &planned,
        [
            range(26 * SECOND, 28 * SECOND)?,
            range(40 * SECOND, 41 * SECOND)?,
        ],
    );
    for (label, earlier, carried) in [
        ("wholly in the gap", range(3 * SECOND, 5 * SECOND)?, true),
        ("the whole gap", range(0, 25 * SECOND)?, true),
        (
            "past the gap's edge by a microsecond",
            range(24 * SECOND, 25 * SECOND + 1)?,
            true,
        ),
        (
            "across the gap into what was read, over no text",
            range(24 * SECOND, 25 * SECOND + 500_000)?,
            true,
        ),
        (
            "ending where the run's text starts",
            range(20 * SECOND, 26 * SECOND)?,
            true,
        ),
        (
            "overlapped by the run's text by a microsecond",
            range(20 * SECOND, 26 * SECOND + 1)?,
            false,
        ),
        (
            "across the gap and over the run's text",
            range(20 * SECOND, 30 * SECOND)?,
            false,
        ),
        (
            "touching the gap's edge only",
            range(25 * SECOND, 26 * SECOND)?,
            false,
        ),
        (
            "wholly in what was read",
            range(30 * SECOND, 32 * SECOND)?,
            false,
        ),
        (
            "inside the replaced range, beyond any gap",
            range(60 * SECOND, 62 * SECOND)?,
            false,
        ),
        (
            "outside the replaced range",
            range(66 * SECOND, 68 * SECOND)?,
            true,
        ),
        (
            "across the replaced range's edge",
            range(60 * SECOND, 70 * SECOND)?,
            false,
        ),
    ] {
        assert_eq!(rule.carries(earlier), carried, "{label}");
    }

    // A run that read everything keeps nothing inside its range.
    let read = run(&[
        AsrChunkOutcome::Transcribed {
            audio: range(0, 30 * SECOND)?,
        },
        AsrChunkOutcome::Silent {
            audio: range(25 * SECOND, 55 * SECOND)?,
        },
        AsrChunkOutcome::NoAudio,
    ])?;
    let nothing = EarlierTextRule::new(range(0, 70 * SECOND)?, &read, []);
    assert!(!nothing.carries(range(3 * SECOND, 5 * SECOND)?));
    Ok(())
}

/// D6: exactly the reviewed profiles are pinned, and each keeps one stable
/// identifier on the wire and in records.
#[test]
fn only_reviewed_model_profiles_are_pinned() {
    assert_eq!(AsrModelProfile::Base.identifier(), "base");
    assert_eq!(AsrModelProfile::BaseQ5_1.identifier(), "base_q5_1");
    assert_eq!(AsrModelProfile::Unreviewed.reviewed(), None);
    for reviewed in ReviewedAsrModel::ALL {
        assert_eq!(reviewed.profile().reviewed(), Some(reviewed));
        assert_eq!(reviewed.identifier(), reviewed.profile().identifier());
    }
    assert_eq!(ReviewedAsrModel::ALL[0], ReviewedAsrModel::Base);
}
