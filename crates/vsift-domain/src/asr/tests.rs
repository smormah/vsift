//! Chunk planning, provider-output validation, silence and seam-merge rules.

use std::num::{NonZeroU16, NonZeroU32};

use proptest::prelude::{prop_assert, prop_assert_eq, proptest};

use super::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, ChunkPlan, ChunkPlanError, ChunkSegments, ChunkTime,
    MIN_RECOGNITION_MICROS, MIN_RECOGNITION_SAMPLES, MIN_SEGMENTS_FOR_REJECTION_RATIO,
    PlannedChunk, ProviderChunkOutput, ProviderOutputError, ProviderSegment, ProviderToken,
    ProviderTokenKind, ReviewedAsrModel, SPEECH_SAMPLE_RATE, Sha256Hex,
    UNUSABLE_CHUNK_SHARE_DENOMINATOR, decoded_audio_range, is_below_recognition_floor,
    is_silent_pcm, merge_chunks, plan_chunks, rounds_to_no_pcm_sample, unusable_chunks_end_the_run,
    validate_chunk_output,
};
use crate::{
    CarriedFrom, Confidence, ConfidenceOrigin, CueSource, CueText, CueTiming, InheritedRevision,
    LanguageTag, MediaTime, ProviderEndTrim, SegmentOrigin, SidecarIdentity, SourceId,
    SourceSegment, SourceSegmentId, SpeakerLabel, TimeRange, TranscriptFormat, TranscriptOffset,
    TranscriptProvenance, TranscriptRevision, TranscriptRevisionError, TranscriptRevisionId,
    TranscriptRevisionParts, TranscriptSegment, TranscriptSegmentId, TranscriptSegmentParts,
    TranscriptWarningKind, TranscriptWarnings,
};

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
    })
}

fn texts(chunks: &[ChunkSegments]) -> (Vec<String>, u32) {
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

/// The quarter rule applies from `MIN_SEGMENTS_FOR_REJECTION_RATIO` text
/// segments, and below it the rejected ones are dropped and counted. At the
/// minimum the rule is exactly what it always was.
#[test]
fn the_quarter_rule_applies_from_four_text_segments() -> TestResult {
    assert_eq!(MIN_SEGMENTS_FOR_REJECTION_RATIO, 4);
    // Below the minimum: kept, whatever the share, unless nothing is kept.
    assert_eq!(judged(&[true, false])?, Ok((1, 1)));
    assert_eq!(judged(&[false, true])?, Ok((1, 1)));
    assert_eq!(judged(&[true, true, false])?, Ok((2, 1)));
    assert_eq!(judged(&[false, true, false])?, Ok((1, 2)));
    assert_eq!(judged(&[false, false, true])?, Ok((1, 2)));
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

/// Why the minimum is 4: it is the smallest count in which one rejection is no
/// more than the share the rule allows, so it is where the rule starts to
/// weigh a share and one below it is where a single rejection already exceeds
/// it. Moving either constant without the other breaks this.
#[test]
fn the_minimum_is_the_smallest_count_that_tolerates_one_rejection() {
    let shares_one_rejection = |segments: u32| segments >= MIN_SEGMENTS_FOR_REJECTION_RATIO;
    for segments in 1..=64_u32 {
        // Under the old rule alone, one rejection beside kept segments fails
        // exactly when it is more than a quarter of them.
        let more_than_a_quarter = 4 > segments;
        assert_eq!(
            shares_one_rejection(segments),
            !more_than_a_quarter,
            "{segments} segments"
        );
    }
}

/// Empty or marker-only text is removed and counted as a marker, never as a
/// rejection, and does not count as a text segment: it cannot lift a chunk to
/// the minimum.
#[test]
fn a_text_that_is_a_marker_does_not_count_as_a_segment() -> TestResult {
    let planned = chunk(0, 0, 10 * SECOND)?;
    let mut blank = provider_segment(1_500, 2_500, "ignored")?;
    blank.text = None;
    let marker = provider_segment(2_600, 2_900, "[BLANK_AUDIO]")?;
    // One kept and two rejected text segments with two markers among them:
    // three text segments, so below the minimum, and the kept one stays. Had
    // the markers counted, four segments with two rejected would fail it.
    let segments = vec![
        provider_segment(1_000, 1_400, "kept")?,
        blank,
        marker,
        provider_segment(3_000, 3_000, "empty range")?,
        provider_segment(4_000, 3_000, "reversed")?,
    ];
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
            (TranscriptWarningKind::ProviderSegmentsRejected, 2),
            (TranscriptWarningKind::NonSpeechMarkersRemoved, 2),
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

/// T-03: identical and time-shifted copies of seam text are kept once.
#[test]
fn seam_duplicates_are_removed() -> TestResult {
    // Both chunks own a copy by midpoint (26.4 s and 27.8 s straddle 27.5 s).
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
    assert!(revision.segments().is_empty());
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
