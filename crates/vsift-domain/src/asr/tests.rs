//! Chunk planning, provider-output validation, silence and seam-merge rules.

use std::num::{NonZeroU16, NonZeroU32};

use proptest::prelude::{prop_assert, prop_assert_eq, proptest};

use super::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, ChunkPlan, ChunkPlanError, ChunkSegments, ChunkTime,
    PlannedChunk, ProviderChunkOutput, ProviderOutputError, ProviderSegment, ProviderToken,
    ProviderTokenKind, ReviewedAsrModel, Sha256Hex, decoded_audio_range, is_silent_pcm,
    merge_chunks, plan_chunks, validate_chunk_output,
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
        provider_segment(9_500, 11_200, "ends too far past the audio")?,
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
        ranges[..4],
        [
            (50_100_000, 52_100_000, ProviderEndTrim::Unchanged),
            (53_100_000, 59_100_000, ProviderEndTrim::Unchanged),
            (59_100_000, 60_100_000, ProviderEndTrim::TrimmedToAudioEnd),
            (59_700_000, 60_100_000, ProviderEndTrim::Unchanged),
        ]
    );
    // The raw provider end is kept beside the trimmed range.
    assert_eq!(
        validated.segments()[2].provider_end().as_micros(),
        10_600_000
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
            (TranscriptWarningKind::ProviderSegmentsRejected, 2, 3),
            (TranscriptWarningKind::ProviderEndTrimmed, 1, 3),
        ]
    );
    Ok(())
}

/// T-05: a chunk whose provider evidently described other audio fails.
#[test]
fn chunks_with_too_many_rejected_segments_fail() -> TestResult {
    let planned = chunk(0, 0, 10 * SECOND)?;
    let audio = planned.window();
    let source = range(0, 10 * SECOND)?;
    let mostly_wrong = vec![
        provider_segment(0, 1_000, "fine")?,
        provider_segment(12_000, 13_000, "after the audio")?,
        provider_segment(12_500, 13_000, "after the audio")?,
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
    let transcribed = AsrChunkOutcome::Transcribed {
        audio: range(25_750_000, 55 * SECOND)?,
    };
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
    let too_far = asr_segment(
        1,
        range(54 * SECOND, 55 * SECOND)?,
        1,
        (28_250_000, 30_300_000),
        ProviderEndTrim::TrimmedToAudioEnd,
        Confidence::unknown(),
    )?;
    assert_eq!(
        TranscriptRevision::new(asr_revision(vec![too_far])?),
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
