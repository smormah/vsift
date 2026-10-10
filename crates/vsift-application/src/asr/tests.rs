//! Local ASR use cases over fake audio and recognizer ports (T-03, T-05).

use std::{
    future::Future,
    num::{NonZeroU16, NonZeroU32},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use vsift_domain::{
    AsrChunkOutcome, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider, AsrProviderBuild,
    ChunkPlan, ChunkTime, ConfidenceOrigin, CueSource, CueText, CueTiming, ImportedCue,
    LanguageTag, MediaTime, ParsedTranscript, PlannedChunk, ProviderChunkOutput,
    ProviderOutputError, ProviderSegment, ProviderToken, ProviderTokenKind, SearchCoverage,
    SegmentOrigin, SessionId, Sha256Hex, SidecarIdentity, SourceId, SourceSegment, SourceSegmentId,
    TimeRange, TranscriptFormat, TranscriptOffset, TranscriptProvenance, TranscriptRevision,
    TranscriptRevisionError, TranscriptWarningKind, TranscriptWarnings,
};

use super::{
    AsrCancellation, AsrFailure, AsrFailureReason, AsrRevisionRequest, AsrStage, AsrTranscription,
    RecognizerIdentity, RevisionSplice, SpeechAudioError, SpeechAudioSource, SpeechPcm,
    SpeechRecognitionError, SpeechRecognizer, TranscribeRangeRequest, UnusableChunk,
    UnusableChunks, build_asr_revision, transcribe_range,
};
use crate::{
    ImportedRevisionRequest, SuppliedTranscript, TranscriptBuildError, build_imported_revision,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const OTHER_DIGEST: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const SECOND: u64 = 1_000_000;

fn identity(model: &str) -> Built<RecognizerIdentity> {
    Ok(RecognizerIdentity {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(DIGEST)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(model)?),
        decoding: AsrDecodingProfile::R0V1,
        threads: NonZeroU16::new(4).ok_or("zero")?,
    })
}

fn source() -> Built<SourceSegment> {
    Ok(SourceSegment::whole_file(
        SourceSegmentId::parse("sgm_0123456789abcdef")?,
        MediaTime::from_micros(70 * SECOND),
    )?)
}

fn range(start: u64, end: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(start),
        MediaTime::from_micros(end),
    )?)
}

/// How the fake audio port answers for one chunk index.
#[derive(Clone, Copy)]
enum Audio {
    Speech,
    Silence,
    Nothing,
    /// This many loud samples from the window's start, whatever its length.
    Loud(usize),
    Fails(SpeechAudioError),
}

struct FakeAudio {
    answers: Vec<Audio>,
    calls: AtomicUsize,
}

impl FakeAudio {
    fn new(answers: Vec<Audio>) -> Self {
        Self {
            answers,
            calls: AtomicUsize::new(0),
        }
    }
}

impl SpeechAudioSource for FakeAudio {
    fn speech_pcm(
        &self,
        chunk: &PlannedChunk,
    ) -> impl Future<Output = Result<SpeechPcm, SpeechAudioError>> + Send {
        std::future::ready(self.answer(chunk))
    }
}

impl FakeAudio {
    fn answer(&self, chunk: &PlannedChunk) -> Result<SpeechPcm, SpeechAudioError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let answer = usize::try_from(chunk.index())
            .ok()
            .and_then(|index| self.answers.get(index).copied())
            .unwrap_or(Audio::Speech);
        let samples = usize::try_from(chunk.window().duration_micros() / 1_000 * 16)
            .map_err(|_| SpeechAudioError::ResourceLimit)?;
        match answer {
            Audio::Speech => Ok(SpeechPcm {
                actual_start: chunk.window().start(),
                samples: vec![3_000; samples],
            }),
            Audio::Silence => Ok(SpeechPcm {
                actual_start: chunk.window().start(),
                samples: vec![0; samples],
            }),
            Audio::Nothing => Err(SpeechAudioError::NoAudio),
            Audio::Loud(count) => Ok(SpeechPcm {
                actual_start: chunk.window().start(),
                samples: vec![3_000; count],
            }),
            Audio::Fails(error) => Err(error),
        }
    }
}

/// Answers every chunk with one segment two seconds into its core.
struct FakeRecognizer {
    identities: Mutex<Vec<RecognizerIdentity>>,
    output: fn(&PlannedChunk) -> Result<ProviderChunkOutput, SpeechRecognitionError>,
    calls: AtomicUsize,
    /// Every chunk it was given, in order: its index and its sample count.
    heard: Mutex<Vec<(u32, usize)>>,
    cancel_after_first: Option<&'static AtomicBool>,
}

impl FakeRecognizer {
    fn new(identity: RecognizerIdentity) -> Self {
        Self {
            identities: Mutex::new(vec![identity]),
            output: one_segment,
            calls: AtomicUsize::new(0),
            heard: Mutex::new(Vec::new()),
            cancel_after_first: None,
        }
    }

    /// The chunks the recognizer was given.
    fn heard(&self) -> Built<Vec<(u32, usize)>> {
        Ok(self
            .heard
            .lock()
            .map_err(|_| "the fake recognizer's record is poisoned")?
            .clone())
    }
}

fn one_segment(chunk: &PlannedChunk) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    let words = format!("words of chunk {}", chunk.index());
    let text = CueText::new(words.clone(), words).map_err(|_| SpeechRecognitionError::Io)?;
    // Start one second after the window's left overlap (or at 10 s in chunk 0).
    let start = if chunk.index() == 0 { 10_000 } else { 6_000 };
    Ok(ProviderChunkOutput {
        language: LanguageTag::parse("en").ok(),
        segments: vec![ProviderSegment {
            start: ChunkTime::from_millis(start).ok_or(SpeechRecognitionError::Io)?,
            end: ChunkTime::from_millis(start + 2_000).ok_or(SpeechRecognitionError::Io)?,
            text: Some(text),
            tokens: vec![
                ProviderToken {
                    kind: ProviderTokenKind::Special,
                    probability: 0.4,
                },
                ProviderToken {
                    kind: ProviderTokenKind::Text,
                    probability: 0.75,
                },
            ],
        }],
    })
}

impl SpeechRecognizer for FakeRecognizer {
    fn identity(
        &self,
    ) -> impl Future<Output = Result<RecognizerIdentity, SpeechRecognitionError>> + Send {
        std::future::ready(self.next_identity())
    }

    fn recognize(
        &self,
        chunk: &PlannedChunk,
        pcm: &SpeechPcm,
    ) -> impl Future<Output = Result<ProviderChunkOutput, SpeechRecognitionError>> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut heard) = self.heard.lock() {
            heard.push((chunk.index(), pcm.samples.len()));
        }
        if let Some(flag) = self.cancel_after_first {
            flag.store(true, Ordering::SeqCst);
        }
        std::future::ready((self.output)(chunk))
    }
}

impl FakeRecognizer {
    fn next_identity(&self) -> Result<RecognizerIdentity, SpeechRecognitionError> {
        let mut identities = self
            .identities
            .lock()
            .map_err(|_| SpeechRecognitionError::Io)?;
        // Each call consumes one answer until the last, which repeats.
        if identities.len() > 1 {
            Ok(identities.remove(0))
        } else {
            identities
                .first()
                .cloned()
                .ok_or(SpeechRecognitionError::Io)
        }
    }
}

struct Flag<'a>(&'a AtomicBool);

impl AsrCancellation for Flag<'_> {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

static NEVER: AtomicBool = AtomicBool::new(false);

async fn run(
    audio: &FakeAudio,
    recognizer: &FakeRecognizer,
    cancellation: &Flag<'_>,
) -> Built<Result<super::AsrTranscription, AsrFailure>> {
    let source = source()?;
    let expected = identity(DIGEST)?;
    Ok(transcribe_range(
        TranscribeRangeRequest {
            source_segment: &source,
            range: source.range(),
            plan: ChunkPlan::R0,
            audio_stream: 1,
            expected: &expected,
        },
        audio,
        recognizer,
        cancellation,
    )
    .await)
}

#[tokio::test]
async fn a_run_transcribes_every_chunk_and_builds_a_valid_revision() -> TestResult {
    let audio = FakeAudio::new(Vec::new());
    let recognizer = FakeRecognizer::new(identity(DIGEST)?);
    let transcription = run(&audio, &recognizer, &Flag(&NEVER)).await??;
    assert_eq!(recognizer.calls.load(Ordering::SeqCst), 3);
    let starts: Vec<u64> = transcription
        .segments
        .iter()
        .map(|merged| merged.segment.range().start().as_micros())
        .collect();
    assert_eq!(starts, [10 * SECOND, 31 * SECOND, 56 * SECOND]);
    assert_eq!(
        transcription.language.as_ref().map(LanguageTag::as_str),
        Some("en")
    );
    assert!(transcription.warnings.as_slice().is_empty());

    let source = source()?;
    let session = SessionId::parse("ses_0123456789abcdef")?;
    let source_id = SourceId::from_sha256(DIGEST)?;
    let build = |transcription| {
        build_asr_revision(AsrRevisionRequest {
            session_id: &session,
            source_id: &source_id,
            source_segment: &source,
            number: NonZeroU32::MIN,
            transcription,
            splice: None,
        })
    };
    let revision = build(transcription.clone())?;
    assert_eq!(revision, build(transcription)?);
    assert!(revision.id().as_str().starts_with("trv_"));
    assert_eq!(revision.origin().identifier(), "local_asr");
    let segment = &revision.segments()[1];
    assert_eq!(
        segment.origin(),
        SegmentOrigin::Asr {
            chunk: 1,
            provider_start: ChunkTime::from_micros(6 * SECOND),
            provider_end: ChunkTime::from_micros(8 * SECOND),
            trimmed: vsift_domain::ProviderEndTrim::Unchanged,
        }
    );
    assert_eq!(segment.confidence().basis_points(), Some(7_500));
    assert_eq!(
        segment.confidence().origin(),
        ConfidenceOrigin::ProviderUncalibrated
    );
    Ok(())
}

/// T-05: each port failure surfaces as a typed failure naming its stage.
#[tokio::test]
async fn port_failures_are_typed_by_stage() -> TestResult {
    let audio = FakeAudio::new(vec![
        Audio::Speech,
        Audio::Fails(SpeechAudioError::Deadline),
    ]);
    let recognizer = FakeRecognizer::new(identity(DIGEST)?);
    assert_eq!(
        run(&audio, &recognizer, &Flag(&NEVER)).await?,
        Err(AsrFailure {
            stage: AsrStage::AudioExtraction,
            reason: AsrFailureReason::Deadline,
        })
    );

    let audio = FakeAudio::new(Vec::new());
    let mut failing = FakeRecognizer::new(identity(DIGEST)?);
    failing.output = |_| Err(SpeechRecognitionError::ProviderFailed);
    assert_eq!(
        run(&audio, &failing, &Flag(&NEVER)).await?,
        Err(AsrFailure {
            stage: AsrStage::Recognition,
            reason: AsrFailureReason::ProviderFailed,
        })
    );

    let mut malformed = FakeRecognizer::new(identity(DIGEST)?);
    malformed.output = |chunk| {
        let mut output = one_segment(chunk)?;
        let mut earlier = output.segments[0].clone();
        earlier.start = ChunkTime::from_micros(0);
        output.segments.push(earlier);
        Ok(output)
    };
    // Every chunk's answer is unusable, so the run fails; it is known to fail
    // as soon as the second of the three chunks is judged (two unusable of at
    // most three answered), and the third is never asked.
    assert_eq!(
        run(&audio, &malformed, &Flag(&NEVER)).await?,
        Err(AsrFailure {
            stage: AsrStage::OutputValidation,
            reason: AsrFailureReason::MalformedOutput(UnusableChunks {
                unusable: 2,
                answered: 2,
                planned: 3,
                first: UnusableChunk {
                    index: 0,
                    window: range(0, 30 * SECOND)?,
                    error: ProviderOutputError::OutOfOrderSegments,
                },
            }),
        })
    );
    assert_eq!(malformed.calls.load(Ordering::SeqCst), 2);
    Ok(())
}

/// Cancellation mid-run returns nothing and runs no further chunk.
#[tokio::test]
async fn cancellation_mid_run_produces_nothing() -> TestResult {
    static CANCELLED: AtomicBool = AtomicBool::new(false);
    let audio = FakeAudio::new(Vec::new());
    let mut recognizer = FakeRecognizer::new(identity(DIGEST)?);
    recognizer.cancel_after_first = Some(&CANCELLED);
    let result = run(&audio, &recognizer, &Flag(&CANCELLED)).await?;
    assert_eq!(
        result,
        Err(AsrFailure {
            stage: AsrStage::AudioExtraction,
            reason: AsrFailureReason::Cancelled,
        })
    );
    assert_eq!(recognizer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(audio.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

/// T-05: a model other than the selected one, or one that changes during the
/// run, fails the run instead of mixing outputs.
#[tokio::test]
async fn model_changes_before_or_during_the_run_fail_it() -> TestResult {
    let audio = FakeAudio::new(Vec::new());
    let other = FakeRecognizer::new(identity(OTHER_DIGEST)?);
    assert_eq!(
        run(&audio, &other, &Flag(&NEVER)).await?,
        Err(AsrFailure {
            stage: AsrStage::RecognizerIdentity,
            reason: AsrFailureReason::ModelChanged,
        })
    );
    assert_eq!(other.calls.load(Ordering::SeqCst), 0);

    let swapped = FakeRecognizer::new(identity(DIGEST)?);
    if let Ok(mut identities) = swapped.identities.lock() {
        identities.push(identity(OTHER_DIGEST)?);
    }
    assert_eq!(
        run(&audio, &swapped, &Flag(&NEVER)).await?,
        Err(AsrFailure {
            stage: AsrStage::RecognizerIdentity,
            reason: AsrFailureReason::ModelChanged,
        })
    );
    assert_eq!(swapped.calls.load(Ordering::SeqCst), 3);

    let mut missing = FakeRecognizer::new(identity(DIGEST)?);
    missing.output = |_| Err(SpeechRecognitionError::ModelUnavailable);
    assert_eq!(
        run(&audio, &missing, &Flag(&NEVER)).await?,
        Err(AsrFailure {
            stage: AsrStage::Recognition,
            reason: AsrFailureReason::ModelUnavailable,
        })
    );
    Ok(())
}

/// T-05 / T-03 silence: silent and audio-less chunks are skipped and recorded.
#[tokio::test]
async fn silent_chunks_are_skipped_and_recorded_as_gaps() -> TestResult {
    let audio = FakeAudio::new(vec![Audio::Speech, Audio::Silence, Audio::Nothing]);
    let recognizer = FakeRecognizer::new(identity(DIGEST)?);
    let transcription = run(&audio, &recognizer, &Flag(&NEVER)).await??;
    assert_eq!(recognizer.calls.load(Ordering::SeqCst), 1);
    let outcomes: Vec<AsrChunkOutcome> = transcription
        .run
        .chunks()
        .iter()
        .map(vsift_domain::AsrChunkRecord::outcome)
        .collect();
    assert_eq!(
        outcomes,
        [
            AsrChunkOutcome::Transcribed {
                audio: range(0, 30 * SECOND)?,
            },
            AsrChunkOutcome::Silent {
                audio: range(25 * SECOND, 55 * SECOND)?,
            },
            AsrChunkOutcome::NoAudio,
        ]
    );
    let warnings: Vec<_> = transcription
        .warnings
        .as_slice()
        .iter()
        .map(|warning| (warning.kind(), warning.count(), warning.first_cue()))
        .collect();
    assert_eq!(
        warnings,
        [(TranscriptWarningKind::SilentChunksSkipped, 2, 2)]
    );
    assert_eq!(transcription.segments.len(), 1);
    Ok(())
}

// ------------------------------------------------ #353: unusable chunks

/// A source of ten R0 chunks (25 s apart, the last 30 s long).
fn ten_chunk_source() -> Built<SourceSegment> {
    Ok(SourceSegment::whole_file(
        SourceSegmentId::parse("sgm_0123456789abcdef")?,
        MediaTime::from_micros(255 * SECOND),
    )?)
}

/// What a recognizer answers when it did not describe the audio: its one
/// segment starts after the chunk's audio ends, so nothing in it can be placed
/// and every segment of the chunk is rejected.
fn after_the_audio(chunk: &PlannedChunk) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    let mut output = one_segment(chunk)?;
    let past = chunk.window().duration_micros() / 1_000 + 1_000;
    for segment in &mut output.segments {
        segment.start = ChunkTime::from_millis(past).ok_or(SpeechRecognitionError::Io)?;
        segment.end = ChunkTime::from_millis(past + 2_000).ok_or(SpeechRecognitionError::Io)?;
    }
    Ok(output)
}

fn unusable_in_chunk_four(
    chunk: &PlannedChunk,
) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    if chunk.index() == 4 {
        after_the_audio(chunk)
    } else {
        one_segment(chunk)
    }
}

fn unusable_before_chunk_five(
    chunk: &PlannedChunk,
) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    if chunk.index() < 5 {
        after_the_audio(chunk)
    } else {
        one_segment(chunk)
    }
}

fn unusable_before_chunk_six(
    chunk: &PlannedChunk,
) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    if chunk.index() < 6 {
        after_the_audio(chunk)
    } else {
        one_segment(chunk)
    }
}

fn unusable_in_the_first_chunk(
    chunk: &PlannedChunk,
) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    if chunk.index() == 0 {
        after_the_audio(chunk)
    } else {
        one_segment(chunk)
    }
}

fn unusable_in_chunk_seven(
    chunk: &PlannedChunk,
) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    if chunk.index() == 7 {
        after_the_audio(chunk)
    } else {
        one_segment(chunk)
    }
}

/// Transcribes `requested` of `source` with `recognizer`'s answers.
async fn transcribe_with(
    source: &SourceSegment,
    requested: TimeRange,
    audio: &FakeAudio,
    recognizer: &FakeRecognizer,
) -> Built<Result<AsrTranscription, AsrFailure>> {
    let expected = identity(DIGEST)?;
    Ok(transcribe_range(
        TranscribeRangeRequest {
            source_segment: source,
            range: requested,
            plan: ChunkPlan::R0,
            audio_stream: 1,
            expected: &expected,
        },
        audio,
        recognizer,
        &Flag(&NEVER),
    )
    .await)
}

fn outcomes_of(transcription: &AsrTranscription) -> Vec<AsrChunkOutcome> {
    transcription
        .run
        .chunks()
        .iter()
        .map(vsift_domain::AsrChunkRecord::outcome)
        .collect()
}

fn warning_tuples(transcription: &AsrTranscription) -> Vec<(TranscriptWarningKind, u32, u32)> {
    transcription
        .warnings
        .as_slice()
        .iter()
        .map(|warning| (warning.kind(), warning.count(), warning.first_cue()))
        .collect()
}

/// #353: one chunk of ten whose answer cannot be placed in its audio is a
/// recorded gap, not a failed run. The nine others are kept, the unusable
/// chunk's window is untranscribed (not silent) and is counted under its own
/// warning, and nothing about the nine others differs from a run without it.
#[tokio::test]
async fn one_unusable_chunk_of_ten_is_a_recorded_gap_and_the_run_is_kept() -> TestResult {
    let source = ten_chunk_source()?;
    let audio = FakeAudio::new(Vec::new());
    let mut recognizer = FakeRecognizer::new(identity(DIGEST)?);
    recognizer.output = unusable_in_chunk_four;
    let transcription = transcribe_with(&source, source.range(), &audio, &recognizer).await??;

    // Every chunk was given to the recognizer, and none was asked twice.
    assert_eq!(recognizer.calls.load(Ordering::SeqCst), 10);
    let outcomes = outcomes_of(&transcription);
    assert_eq!(outcomes.len(), 10);
    for (index, outcome) in outcomes.iter().enumerate() {
        let start = u64::try_from(index)? * 25 * SECOND;
        let window = range(start, (start + 30 * SECOND).min(255 * SECOND))?;
        if index == 4 {
            assert_eq!(*outcome, AsrChunkOutcome::Unusable { audio: window });
        } else {
            assert_eq!(*outcome, AsrChunkOutcome::Transcribed { audio: window });
        }
    }
    // The other nine chunks' segments are all there.
    assert_eq!(transcription.segments.len(), 9);
    assert!(
        transcription
            .segments
            .iter()
            .all(|merged| merged.chunk != 4)
    );
    // Counted by chunk, first at chunk ordinal 5; no silence is claimed.
    assert_eq!(
        warning_tuples(&transcription),
        [(TranscriptWarningKind::ProviderChunksRejected, 1, 5)]
    );

    // As a revision, the window is untranscribed, which a search reports; the
    // parts of it the neighbours' overlaps transcribed are covered.
    let session = SessionId::parse("ses_0123456789abcdef")?;
    let source_id = SourceId::from_sha256(DIGEST)?;
    let revision = build_asr_revision(AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::MIN,
        transcription,
        splice: None,
    })?;
    let coverage = vsift_domain::SearchCoverage::of(&revision, None);
    assert_eq!(
        coverage.untranscribed(),
        [range(105 * SECOND, 125 * SECOND)?]
    );
    assert!(coverage.no_speech().is_empty());
    Ok(())
}

/// #353: the run fails only when MORE than half of the chunks the recognizer
/// answered are unusable. Five of ten is a partial run; six of ten is not, and
/// the run stops as soon as that is certain instead of recognising the rest.
#[tokio::test]
async fn exactly_half_unusable_is_a_partial_run_and_more_than_half_fails_early() -> TestResult {
    let source = ten_chunk_source()?;
    let audio = FakeAudio::new(Vec::new());

    let mut half = FakeRecognizer::new(identity(DIGEST)?);
    half.output = unusable_before_chunk_five;
    let kept = transcribe_with(&source, source.range(), &audio, &half).await??;
    assert_eq!(half.calls.load(Ordering::SeqCst), 10);
    assert_eq!(
        warning_tuples(&kept),
        [(TranscriptWarningKind::ProviderChunksRejected, 5, 1)]
    );
    assert_eq!(kept.segments.len(), 5);

    let mut most = FakeRecognizer::new(identity(DIGEST)?);
    most.output = unusable_before_chunk_six;
    let failed = transcribe_with(&source, source.range(), &audio, &most).await?;
    assert_eq!(
        failed,
        Err(AsrFailure {
            stage: AsrStage::OutputValidation,
            reason: AsrFailureReason::MalformedOutput(UnusableChunks {
                // Judged after the sixth chunk: six unusable of at most ten,
                // and the four still to come could not bring them to a half.
                unusable: 6,
                answered: 6,
                planned: 10,
                first: UnusableChunk {
                    index: 0,
                    window: range(0, 30 * SECOND)?,
                    error: ProviderOutputError::TooManyRejectedSegments,
                },
            }),
        })
    );
    // The four chunks after the sixth were never given to the recognizer.
    assert_eq!(most.calls.load(Ordering::SeqCst), 6);
    Ok(())
}

/// #353: only chunks the recognizer answered count in the share. Silence and
/// missing audio say nothing about it, so a recording that is mostly quiet is
/// judged by the few chunks that had speech, as the failure it is for.
#[tokio::test]
async fn chunks_never_given_to_the_recognizer_are_not_answers() -> TestResult {
    let source = ten_chunk_source()?;
    // Six quiet chunks and four with speech; one of the four is unusable. One
    // of four answered is a quarter: a partial run, though it is a tenth of
    // all the chunks and would be under any share of them.
    let mut answers = vec![Audio::Silence; 6];
    answers.extend([Audio::Speech; 4]);
    let audio = FakeAudio::new(answers);
    let mut recognizer = FakeRecognizer::new(identity(DIGEST)?);
    recognizer.output = unusable_in_chunk_seven;
    let kept = transcribe_with(&source, source.range(), &audio, &recognizer).await??;
    assert_eq!(recognizer.calls.load(Ordering::SeqCst), 4);
    assert_eq!(
        warning_tuples(&kept),
        [
            (TranscriptWarningKind::SilentChunksSkipped, 6, 1),
            (TranscriptWarningKind::ProviderChunksRejected, 1, 8),
        ]
    );

    // Nine quiet chunks and one with speech whose answer is unusable: the only
    // chunk the recognizer answered is unusable, so the run fails, though one
    // chunk in ten is nothing like most of them.
    let mut answers = vec![Audio::Silence; 9];
    answers.push(Audio::Speech);
    let audio = FakeAudio::new(answers);
    let mut garbage = FakeRecognizer::new(identity(DIGEST)?);
    garbage.output = after_the_audio;
    let failed = transcribe_with(&source, source.range(), &audio, &garbage).await?;
    assert_eq!(
        failed.map_err(|failure| failure.reason),
        Err(AsrFailureReason::MalformedOutput(UnusableChunks {
            unusable: 1,
            answered: 1,
            planned: 10,
            first: UnusableChunk {
                index: 9,
                window: range(225 * SECOND, 255 * SECOND)?,
                error: ProviderOutputError::TooManyRejectedSegments,
            },
        }))
    );
    Ok(())
}

/// #353: with few chunks the other chunk is the evidence. One unusable chunk of
/// two is a partial run, since the other was answered by the same recognizer;
/// a lone chunk that cannot be placed fails the run, as it always did.
#[tokio::test]
async fn one_unusable_chunk_of_two_is_partial_and_a_lone_one_fails() -> TestResult {
    let source = source()?;
    let audio = FakeAudio::new(Vec::new());
    let mut recognizer = FakeRecognizer::new(identity(DIGEST)?);
    recognizer.output = unusable_in_the_first_chunk;
    // 0 s to 55 s is two chunks.
    let two = transcribe_with(&source, range(0, 55 * SECOND)?, &audio, &recognizer).await??;
    assert_eq!(
        outcomes_of(&two),
        [
            AsrChunkOutcome::Unusable {
                audio: range(0, 30 * SECOND)?,
            },
            AsrChunkOutcome::Transcribed {
                audio: range(25 * SECOND, 55 * SECOND)?,
            },
        ]
    );
    assert_eq!(
        warning_tuples(&two),
        [(TranscriptWarningKind::ProviderChunksRejected, 1, 1)]
    );
    // 0 s to 20 s is one chunk.
    let lone = transcribe_with(&source, range(0, 20 * SECOND)?, &audio, &recognizer).await?;
    assert!(
        matches!(
            lone,
            Err(AsrFailure {
                stage: AsrStage::OutputValidation,
                reason: AsrFailureReason::MalformedOutput(UnusableChunks {
                    unusable: 1,
                    answered: 1,
                    planned: 1,
                    ..
                }),
            })
        ),
        "{lone:?}"
    );
    Ok(())
}

/// A run in which no chunk is unusable is what it was: no unusable outcome and
/// no new warning, whatever else the run holds.
#[tokio::test]
async fn a_run_without_an_unusable_chunk_records_nothing_about_them() -> TestResult {
    let source = ten_chunk_source()?;
    let mut answers = vec![Audio::Speech; 10];
    answers[3] = Audio::Silence;
    answers[6] = Audio::Nothing;
    let audio = FakeAudio::new(answers);
    let recognizer = FakeRecognizer::new(identity(DIGEST)?);
    let transcription = transcribe_with(&source, source.range(), &audio, &recognizer).await??;
    assert!(
        outcomes_of(&transcription)
            .iter()
            .all(|outcome| !matches!(outcome, AsrChunkOutcome::Unusable { .. }))
    );
    assert_eq!(
        warning_tuples(&transcription),
        [(TranscriptWarningKind::SilentChunksSkipped, 2, 4)]
    );
    Ok(())
}

/// What the recognizer answers when the audio holds nothing but a marker.
fn only_a_marker(chunk: &PlannedChunk) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    let mut output = one_segment(chunk)?;
    for segment in &mut output.segments {
        segment.text = Some(
            CueText::new("[BLANK_AUDIO]".to_owned(), "[BLANK_AUDIO]".to_owned())
                .map_err(|_| SpeechRecognitionError::Io)?,
        );
    }
    Ok(output)
}

/// A chunk that could not be read and one that held only a marker.
fn unusable_then_a_marker(
    chunk: &PlannedChunk,
) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    if chunk.index() == 0 {
        after_the_audio(chunk)
    } else {
        only_a_marker(chunk)
    }
}

/// #353: "no speech was recognised" is a claim about the whole range, so a run
/// that could not use its answer for some of the audio does not make it: it
/// has not heard that there was none. A run that read all of it still does.
#[tokio::test]
async fn a_run_that_could_not_read_a_chunk_does_not_claim_there_was_no_speech() -> TestResult {
    /// The warnings of the revision a run over two chunks (0-55 s) yields.
    async fn warning_kinds(recognizer: FakeRecognizer) -> Built<Vec<TranscriptWarningKind>> {
        let source = source()?;
        let audio = FakeAudio::new(Vec::new());
        let transcription =
            transcribe_with(&source, range(0, 55 * SECOND)?, &audio, &recognizer).await??;
        assert!(transcription.segments.is_empty());
        let revision = build_asr_revision(AsrRevisionRequest {
            session_id: &SessionId::parse("ses_0123456789abcdef")?,
            source_id: &SourceId::from_sha256(DIGEST)?,
            source_segment: &source,
            number: NonZeroU32::MIN,
            transcription,
            splice: None,
        })?;
        Ok(revision
            .warnings()
            .as_slice()
            .iter()
            .map(|warning| warning.kind())
            .collect())
    }

    let mut mixed = FakeRecognizer::new(identity(DIGEST)?);
    mixed.output = unusable_then_a_marker;
    let kinds = warning_kinds(mixed).await?;
    assert!(kinds.contains(&TranscriptWarningKind::ProviderChunksRejected));
    assert!(
        !kinds.contains(&TranscriptWarningKind::NoSpeechRecognised),
        "{kinds:?}"
    );

    let mut heard_nothing = FakeRecognizer::new(identity(DIGEST)?);
    heard_nothing.output = only_a_marker;
    let kinds = warning_kinds(heard_nothing).await?;
    assert!(!kinds.contains(&TranscriptWarningKind::ProviderChunksRejected));
    assert!(kinds.contains(&TranscriptWarningKind::NoSpeechRecognised));
    Ok(())
}

/// #353: the verdict on the answers is not reached before the model has been
/// checked. A recognizer that was swapped while the run was in progress made
/// the answers meaningless, so the run reports the swap, which the user can act
/// on, and not that the answers could not be used.
#[tokio::test]
async fn a_model_swapped_during_a_run_is_reported_before_unusable_answers() -> TestResult {
    let audio = FakeAudio::new(Vec::new());
    let mut recognizer = FakeRecognizer::new(identity(DIGEST)?);
    recognizer.output = after_the_audio;
    if let Ok(mut identities) = recognizer.identities.lock() {
        identities.push(identity(OTHER_DIGEST)?);
    }
    assert_eq!(
        run(&audio, &recognizer, &Flag(&NEVER)).await?,
        Err(AsrFailure {
            stage: AsrStage::RecognizerIdentity,
            reason: AsrFailureReason::ModelChanged,
        })
    );
    Ok(())
}

/// A run that heard no speech is recorded as a revision with no segment, its
/// silent chunks and a typed warning, so the attempt is not lost (ADR 0017).
#[tokio::test]
async fn a_run_without_speech_is_recorded_without_segments() -> TestResult {
    let audio = FakeAudio::new(vec![Audio::Silence, Audio::Silence, Audio::Silence]);
    let recognizer = FakeRecognizer::new(identity(DIGEST)?);
    let transcription = run(&audio, &recognizer, &Flag(&NEVER)).await??;
    assert_eq!(recognizer.calls.load(Ordering::SeqCst), 0);
    let source = source()?;
    let revision = build_asr_revision(AsrRevisionRequest {
        session_id: &SessionId::parse("ses_0123456789abcdef")?,
        source_id: &SourceId::from_sha256(DIGEST)?,
        source_segment: &source,
        number: NonZeroU32::MIN,
        transcription,
        splice: None,
    })?;
    assert!(revision.segments().is_empty());
    let warnings: Vec<_> = revision
        .warnings()
        .as_slice()
        .iter()
        .map(|warning| (warning.kind(), warning.count(), warning.first_cue()))
        .collect();
    assert_eq!(
        warnings,
        [
            (TranscriptWarningKind::SilentChunksSkipped, 3, 1),
            (TranscriptWarningKind::NoSpeechRecognised, 1, 1),
        ]
    );
    Ok(())
}

/// #322, L-137: decoded audio under 100 ms (1,600 samples) is recorded as a
/// gap, with its decoded range, and is never given to the recognizer, however
/// loud it is; audio of exactly 100 ms is given to it. Before the floor the
/// recognizer was called for the 1,599 loud samples too (whisper.cpp v1.9.2
/// reads past its buffer for fewer than 201).
#[tokio::test]
async fn audio_under_the_floor_is_a_recorded_gap_and_never_reaches_the_recognizer() -> TestResult {
    let audio = FakeAudio::new(vec![Audio::Speech, Audio::Loud(1_599), Audio::Loud(1_600)]);
    let mut recognizer = FakeRecognizer::new(identity(DIGEST)?);
    // It hears nothing: a tenth of a second has no room for the usual segment.
    recognizer.output = |_| {
        Ok(ProviderChunkOutput {
            language: LanguageTag::parse("en").ok(),
            segments: Vec::new(),
        })
    };
    let transcription = run(&audio, &recognizer, &Flag(&NEVER)).await??;

    // Chunk 1 was decoded and was not sent; chunks 0 and 2 were sent whole.
    assert_eq!(audio.calls.load(Ordering::SeqCst), 3);
    assert_eq!(recognizer.heard()?, [(0, 480_000), (2, 1_600)]);
    let outcomes: Vec<AsrChunkOutcome> = transcription
        .run
        .chunks()
        .iter()
        .map(vsift_domain::AsrChunkRecord::outcome)
        .collect();
    assert_eq!(
        outcomes,
        [
            AsrChunkOutcome::Transcribed {
                audio: range(0, 30 * SECOND)?,
            },
            // 1,599 samples from 25 s: 99.9375 ms, kept to the microsecond.
            AsrChunkOutcome::Silent {
                audio: range(25 * SECOND, 25 * SECOND + 99_937)?,
            },
            AsrChunkOutcome::Transcribed {
                audio: range(50 * SECOND, 50 * SECOND + 100_000)?,
            },
        ]
    );
    let warnings: Vec<_> = transcription
        .warnings
        .as_slice()
        .iter()
        .map(|warning| (warning.kind(), warning.count(), warning.first_cue()))
        .collect();
    assert_eq!(
        warnings,
        [(TranscriptWarningKind::SilentChunksSkipped, 1, 2)]
    );
    Ok(())
}

/// Every size the reviewed recognizer mishandles stays away from it: one
/// sample, the 40 its command line fails on, the 200 it reads past, and the
/// sample before the floor.
#[tokio::test]
async fn no_chunk_of_fewer_than_1600_samples_is_ever_recognised() -> TestResult {
    for samples in [1_usize, 40, 41, 200, 201, 1_599] {
        let audio = FakeAudio::new(vec![Audio::Loud(samples); 3]);
        let recognizer = FakeRecognizer::new(identity(DIGEST)?);
        let transcription = run(&audio, &recognizer, &Flag(&NEVER)).await??;
        assert_eq!(recognizer.calls.load(Ordering::SeqCst), 0, "{samples}");
        assert!(
            transcription
                .run
                .chunks()
                .iter()
                .all(|record| matches!(record.outcome(), AsrChunkOutcome::Silent { .. })),
            "{samples}"
        );
    }
    Ok(())
}

/// Transcribes `requested` of the 70 s source with a recognizer that hears
/// nothing, and returns the run with how often audio was decoded and how often
/// the recognizer was called.
async fn run_range(requested: TimeRange) -> Built<(AsrTranscription, usize, usize)> {
    let source = source()?;
    let expected = identity(DIGEST)?;
    let audio = FakeAudio::new(Vec::new());
    let mut recognizer = FakeRecognizer::new(identity(DIGEST)?);
    recognizer.output = |_| {
        Ok(ProviderChunkOutput {
            language: LanguageTag::parse("en").ok(),
            segments: Vec::new(),
        })
    };
    let transcription = transcribe_range(
        TranscribeRangeRequest {
            source_segment: &source,
            range: requested,
            plan: ChunkPlan::R0,
            audio_stream: 1,
            expected: &expected,
        },
        &audio,
        &recognizer,
        &Flag(&NEVER),
    )
    .await?;
    Ok((
        transcription,
        audio.calls.load(Ordering::SeqCst),
        recognizer.calls.load(Ordering::SeqCst),
    ))
}

/// #332: a requested range shorter than 100 ms is one chunk whose window is
/// under the floor. It is recorded as a gap with no audio and is **neither
/// decoded nor recognised**, and the run is not a failure: it commits a
/// revision with no segment and the two warnings of a run that heard nothing
/// (ADR 0017 section 7). The lengths are the ones the media tool mishandled (a
/// window of 31 microseconds or less was answered with a block of about four
/// seconds and recognised as seconds of speech), its first good ones, and the floor's two
/// sides. Before the rule every one of them was handed to the decoder.
#[tokio::test]
async fn a_range_under_the_floor_is_recorded_without_decoding_and_is_not_a_failure() -> TestResult {
    let start = 10 * SECOND;
    for micros in [1_u64, 31, 32, 62, 63, 12_500, 50_000, 99_999] {
        let requested = range(start, start + micros)?;
        let (transcription, decodes, recognitions) = run_range(requested).await?;
        assert_eq!((decodes, recognitions), (0, 0), "{micros}");
        assert!(transcription.segments.is_empty(), "{micros}");
        assert_eq!(transcription.language, None, "{micros}");

        let source = source()?;
        let revision = build_asr_revision(AsrRevisionRequest {
            session_id: &SessionId::parse("ses_0123456789abcdef")?,
            source_id: &SourceId::from_sha256(DIGEST)?,
            source_segment: &source,
            number: NonZeroU32::MIN,
            transcription,
            splice: None,
        })?;
        assert!(revision.segments().is_empty(), "{micros}");
        let TranscriptProvenance::LocalAsr(run) = revision.provenance() else {
            return Err("the revision is not a local-ASR one".into());
        };
        assert_eq!(run.covered_range(), Some(requested), "{micros}");
        assert_eq!(
            run.chunks()
                .iter()
                .map(vsift_domain::AsrChunkRecord::outcome)
                .collect::<Vec<_>>(),
            [AsrChunkOutcome::NoAudio],
            "{micros}"
        );
        let warnings: Vec<_> = revision
            .warnings()
            .as_slice()
            .iter()
            .map(|warning| (warning.kind(), warning.count(), warning.first_cue()))
            .collect();
        assert_eq!(
            warnings,
            [
                (TranscriptWarningKind::SilentChunksSkipped, 1, 1),
                (TranscriptWarningKind::NoSpeechRecognised, 1, 1),
            ],
            "{micros}"
        );
    }
    Ok(())
}

/// The other side of the floor: a range of exactly 100 ms, and anything
/// longer, is decoded and given to the recognizer as before.
#[tokio::test]
async fn a_range_of_a_tenth_of_a_second_or_more_is_decoded_and_recognised() -> TestResult {
    let start = 10 * SECOND;
    for micros in [100_000_u64, 100_001, 150_000, SECOND] {
        let requested = range(start, start + micros)?;
        let (transcription, decodes, recognitions) = run_range(requested).await?;
        assert_eq!((decodes, recognitions), (1, 1), "{micros}");
        assert!(
            matches!(
                transcription
                    .run
                    .chunks()
                    .first()
                    .map(vsift_domain::AsrChunkRecord::outcome),
                Some(AsrChunkOutcome::Transcribed { .. })
            ),
            "{micros}"
        );
    }
    Ok(())
}

/// D5: a model that is not a reviewed pinned profile never runs.
#[tokio::test]
async fn unpinned_models_are_refused_before_any_work() -> TestResult {
    let audio = FakeAudio::new(Vec::new());
    let mut unpinned = identity(DIGEST)?;
    unpinned.model = AsrModel::new(AsrModelProfile::Unreviewed, Sha256Hex::parse(DIGEST)?);
    let recognizer = FakeRecognizer::new(unpinned.clone());
    let source = source()?;
    let result = transcribe_range(
        TranscribeRangeRequest {
            source_segment: &source,
            range: source.range(),
            plan: ChunkPlan::R0,
            audio_stream: 1,
            expected: &unpinned,
        },
        &audio,
        &recognizer,
        &Flag(&NEVER),
    )
    .await;
    assert_eq!(
        result,
        Err(AsrFailure {
            stage: AsrStage::RecognizerIdentity,
            reason: AsrFailureReason::UnpinnedModel,
        })
    );
    assert_eq!(audio.calls.load(Ordering::SeqCst), 0);
    assert_eq!(recognizer.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

/// An imported base revision over the 70 s source: cues at 5-7 s, 12-14 s,
/// 40-42 s and 60-62 s, aligned with a zero offset.
fn imported_base(session: &SessionId, source_id: &SourceId) -> Built<TranscriptRevision> {
    let mut cues = Vec::new();
    for (ordinal, start) in (1_u32..).zip([5, 12, 40, 60]) {
        let words = format!("imported cue {ordinal}");
        cues.push(ImportedCue {
            source: CueSource::new(
                NonZeroU32::new(ordinal).ok_or("zero")?,
                NonZeroU32::new(ordinal * 4).ok_or("zero")?,
            ),
            timing: CueTiming::new(start * SECOND, (start + 2) * SECOND)?,
            text: CueText::new(words.clone(), words)?,
            speaker: None,
        });
    }
    let supplied = SuppliedTranscript {
        transcript: ParsedTranscript::new(
            TranscriptFormat::Srt,
            LanguageTag::parse("en").ok(),
            cues,
            TranscriptWarnings::default(),
        )?,
        sidecar: SidecarIdentity::new(DIGEST, 100)?,
    };
    Ok(build_imported_revision(ImportedRevisionRequest {
        session_id: session,
        source_id,
        source_duration: MediaTime::from_micros(70 * SECOND),
        supplied: &supplied,
        offset: TranscriptOffset::ZERO,
        number: NonZeroU32::MIN,
    })?)
}

/// One segment half a second into each chunk, two seconds long.
fn early_segment(chunk: &PlannedChunk) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    let words = format!("recognised in chunk {}", chunk.index());
    let text = CueText::new(words.clone(), words).map_err(|_| SpeechRecognitionError::Io)?;
    Ok(ProviderChunkOutput {
        language: LanguageTag::parse("es").ok(),
        segments: vec![ProviderSegment {
            start: ChunkTime::from_millis(500).ok_or(SpeechRecognitionError::Io)?,
            end: ChunkTime::from_millis(2_500).ok_or(SpeechRecognitionError::Io)?,
            text: Some(text),
            tokens: Vec::new(),
        }],
    })
}

async fn transcribe(source: &SourceSegment, requested: TimeRange) -> Built<AsrTranscription> {
    let audio = FakeAudio::new(Vec::new());
    let mut recognizer = FakeRecognizer::new(identity(DIGEST)?);
    recognizer.output = early_segment;
    let expected = identity(DIGEST)?;
    Ok(transcribe_range(
        TranscribeRangeRequest {
            source_segment: source,
            range: requested,
            plan: ChunkPlan::R0,
            audio_stream: 1,
            expected: &expected,
        },
        &audio,
        &recognizer,
        &Flag(&NEVER),
    )
    .await?)
}

fn texts(revision: &TranscriptRevision) -> Vec<(u64, String)> {
    revision
        .segments()
        .iter()
        .map(|segment| {
            (
                segment.range().start().as_micros() / SECOND,
                segment.text().text().to_owned(),
            )
        })
        .collect()
}

/// D3/T-06: a bounded retranscription is a complete spliced revision whose
/// carried segments keep their provenance, and a second one carries text from
/// both earlier revisions, always naming the revision that first produced it.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "One chain of three revisions, checked as it grows"
)]
async fn bounded_retranscriptions_splice_complete_revisions() -> TestResult {
    let session = SessionId::parse("ses_0123456789abcdef")?;
    let source_id = SourceId::from_sha256(DIGEST)?;
    let base = imported_base(&session, &source_id)?;
    let source = base.source_segment().clone();
    // 11-13 s cuts the 12-14 s cue, so the range widens to 11-14 s.
    let replaced = base.snap_to_segments(range(11 * SECOND, 13 * SECOND)?);
    assert_eq!(replaced, range(11 * SECOND, 14 * SECOND)?);
    let second = build_asr_revision(AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::new(2).ok_or("zero")?,
        transcription: transcribe(&source, replaced).await?,
        splice: Some(RevisionSplice {
            base: &base,
            replaced_range: replaced,
        }),
    })?;
    assert_eq!(
        texts(&second),
        [
            (5, "imported cue 1".to_owned()),
            (11, "recognised in chunk 0".to_owned()),
            (40, "imported cue 3".to_owned()),
            (60, "imported cue 4".to_owned()),
        ]
    );
    assert_eq!(second.supersedes(), Some(base.id()));
    assert_eq!(second.replaced_range(), Some(replaced));
    assert_eq!(second.inherited().len(), 1);
    let carried = &second.segments()[0];
    let origin = carried.carried_from().ok_or("not carried")?;
    assert_eq!(origin.revision(), base.id());
    assert_eq!(origin.segment(), base.segments()[0].id());
    assert_ne!(carried.id(), base.segments()[0].id());
    assert_eq!(carried.origin(), base.segments()[0].origin());
    assert!(matches!(
        second.segment_provenance(carried),
        TranscriptProvenance::Imported { .. }
    ));
    assert_eq!(
        second.segment_language(carried).map(LanguageTag::as_str),
        Some("en")
    );
    assert_eq!(
        second
            .segment_language(&second.segments()[1])
            .map(LanguageTag::as_str),
        Some("es")
    );

    // A second bounded run over 40-42 s carries from both earlier revisions.
    let replaced = second.snap_to_segments(range(40_500_000, 41 * SECOND)?);
    let third = build_asr_revision(AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::new(3).ok_or("zero")?,
        transcription: transcribe(&source, replaced).await?,
        splice: Some(RevisionSplice {
            base: &second,
            replaced_range: replaced,
        }),
    })?;
    assert_eq!(
        texts(&third),
        [
            (5, "imported cue 1".to_owned()),
            (11, "recognised in chunk 0".to_owned()),
            (40, "recognised in chunk 0".to_owned()),
            (60, "imported cue 4".to_owned()),
        ]
    );
    let origins: Vec<_> = third
        .segments()
        .iter()
        .map(|segment| segment.carried_from().map(|from| from.revision().clone()))
        .collect();
    assert_eq!(
        origins,
        [
            Some(base.id().clone()),
            Some(second.id().clone()),
            None,
            Some(base.id().clone()),
        ]
    );
    assert_eq!(third.inherited().len(), 2);

    // The splice must match the run exactly.
    for (number, replaced_range) in [(4, range(40 * SECOND, 43 * SECOND)?), (2, replaced)] {
        let mismatched = build_asr_revision(AsrRevisionRequest {
            session_id: &session,
            source_id: &source_id,
            source_segment: &source,
            number: NonZeroU32::new(number).ok_or("zero")?,
            transcription: transcribe(&source, replaced).await?,
            splice: Some(RevisionSplice {
                base: &third,
                replaced_range,
            }),
        });
        assert_eq!(
            mismatched,
            Err(TranscriptBuildError::Invalid(
                TranscriptRevisionError::InvalidSupersession
            ))
        );
    }
    Ok(())
}

/// A whole-source retranscription supersedes the base and carries nothing.
#[tokio::test]
async fn whole_source_retranscription_replaces_everything() -> TestResult {
    let session = SessionId::parse("ses_0123456789abcdef")?;
    let source_id = SourceId::from_sha256(DIGEST)?;
    let base = imported_base(&session, &source_id)?;
    let source = base.source_segment().clone();
    let revision = build_asr_revision(AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::new(2).ok_or("zero")?,
        transcription: transcribe(&source, source.range()).await?,
        splice: Some(RevisionSplice {
            base: &base,
            replaced_range: source.range(),
        }),
    })?;
    assert!(
        revision
            .segments()
            .iter()
            .all(|segment| segment.carried_from().is_none())
    );
    assert!(revision.inherited().is_empty());
    assert_eq!(revision.supersedes(), Some(base.id()));
    Ok(())
}

/// #353: a range run that could not read one of its chunks must not delete the
/// text the session already had in that part of the range. An imported
/// transcript has cues at 5, 12, 40 and 60 s; re-transcribing 10-65 s plans
/// the chunks 10-40 s and 35-65 s, and the first is unusable. The part of the
/// range only that chunk covers (10-35 s) keeps the cue at 12 s, as imported,
/// with its original provenance; the cues at 40 and 60 s are in the part the
/// second chunk transcribed, so what the run heard there replaces them, as it
/// always did; the cue at 5 s was never in the range.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "One scenario: the revision, its provenance, its warning and its coverage"
)]
async fn a_range_run_keeps_the_earlier_text_of_a_chunk_it_could_not_read() -> TestResult {
    let session = SessionId::parse("ses_0123456789abcdef")?;
    let source_id = SourceId::from_sha256(DIGEST)?;
    let base = imported_base(&session, &source_id)?;
    let source = base.source_segment().clone();
    let replaced = base.snap_to_segments(range(10 * SECOND, 65 * SECOND)?);
    assert_eq!(replaced, range(10 * SECOND, 65 * SECOND)?);

    let audio = FakeAudio::new(Vec::new());
    let mut recognizer = FakeRecognizer::new(identity(DIGEST)?);
    recognizer.output = unusable_in_the_first_chunk;
    let transcription = transcribe_with(&source, replaced, &audio, &recognizer).await??;
    assert_eq!(
        outcomes_of(&transcription),
        [
            AsrChunkOutcome::Unusable {
                audio: range(10 * SECOND, 40 * SECOND)?,
            },
            AsrChunkOutcome::Transcribed {
                audio: range(35 * SECOND, 65 * SECOND)?,
            },
        ]
    );
    let revision = build_asr_revision(AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::new(2).ok_or("zero")?,
        transcription,
        splice: Some(RevisionSplice {
            base: &base,
            replaced_range: replaced,
        }),
    })?;

    // The cue at 12 s is kept; the cues at 40 and 60 s were replaced by what
    // the second chunk heard.
    assert_eq!(
        texts(&revision),
        [
            (5, "imported cue 1".to_owned()),
            (12, "imported cue 2".to_owned()),
            (41, "words of chunk 1".to_owned()),
        ]
    );
    let kept = &revision.segments()[1];
    assert_eq!(kept.range(), base.segments()[1].range());
    let origin = kept.carried_from().ok_or("not carried")?;
    assert_eq!(origin.revision(), base.id());
    assert_eq!(origin.segment(), base.segments()[1].id());
    assert!(matches!(
        revision.segment_provenance(kept),
        TranscriptProvenance::Imported { .. }
    ));
    assert_eq!(revision.inherited().len(), 1);
    assert_eq!(revision.replaced_range(), Some(replaced));

    // The revision says what it did not do: one chunk could not be used, and
    // the run does not claim to have heard that there was no speech.
    let warnings: Vec<_> = revision
        .warnings()
        .as_slice()
        .iter()
        .map(|warning| (warning.kind(), warning.count(), warning.first_cue()))
        .collect();
    assert_eq!(
        warnings,
        [(TranscriptWarningKind::ProviderChunksRejected, 1, 1)]
    );

    // The kept cue is covered, as supplied text; the rest of the part of the
    // range that no chunk transcribed is not.
    let coverage = SearchCoverage::of(&revision, None);
    assert_eq!(
        coverage.transcribed(),
        [
            range(0, 10 * SECOND)?,
            range(12 * SECOND, 14 * SECOND)?,
            range(35 * SECOND, 70 * SECOND)?,
        ]
    );
    assert_eq!(
        coverage.untranscribed(),
        [
            range(10 * SECOND, 12 * SECOND)?,
            range(14 * SECOND, 35 * SECOND)?,
        ]
    );
    assert!(coverage.no_speech().is_empty());
    // What the run itself did not transcribe is the same ranges, from the run.
    assert_eq!(revision_gaps(&revision), [range(10 * SECOND, 35 * SECOND)?]);
    Ok(())
}

/// The part of the revision's range that its own run left unread.
fn revision_gaps(revision: &TranscriptRevision) -> Vec<TimeRange> {
    match revision.provenance() {
        TranscriptProvenance::LocalAsr(run) => run.unusable_gaps(),
        TranscriptProvenance::Imported { .. } => Vec::new(),
    }
}

/// #353: when every chunk of the range is read, nothing of the earlier text
/// is kept inside it, as before: the gap rule changes nothing for a run that
/// has no gap.
#[tokio::test]
async fn a_range_run_without_a_gap_replaces_the_earlier_text_as_before() -> TestResult {
    let session = SessionId::parse("ses_0123456789abcdef")?;
    let source_id = SourceId::from_sha256(DIGEST)?;
    let base = imported_base(&session, &source_id)?;
    let source = base.source_segment().clone();
    let replaced = base.snap_to_segments(range(10 * SECOND, 65 * SECOND)?);
    let revision = build_asr_revision(AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::new(2).ok_or("zero")?,
        transcription: transcribe(&source, replaced).await?,
        splice: Some(RevisionSplice {
            base: &base,
            replaced_range: replaced,
        }),
    })?;
    let kept: Vec<_> = texts(&revision)
        .into_iter()
        .filter(|(_, text)| text.starts_with("imported"))
        .collect();
    assert_eq!(kept, [(5, "imported cue 1".to_owned())]);
    assert!(revision_gaps(&revision).is_empty());
    Ok(())
}

#[tokio::test]
async fn ranges_outside_the_source_are_refused_before_any_work() -> TestResult {
    let audio = FakeAudio::new(Vec::new());
    let recognizer = FakeRecognizer::new(identity(DIGEST)?);
    let source = source()?;
    let expected = identity(DIGEST)?;
    let result = transcribe_range(
        TranscribeRangeRequest {
            source_segment: &source,
            range: range(60 * SECOND, 80 * SECOND)?,
            plan: ChunkPlan::R0,
            audio_stream: 1,
            expected: &expected,
        },
        &audio,
        &recognizer,
        &Flag(&NEVER),
    )
    .await;
    assert_eq!(
        result,
        Err(AsrFailure {
            stage: AsrStage::Planning,
            reason: AsrFailureReason::InvalidRange,
        })
    );
    assert_eq!(audio.calls.load(Ordering::SeqCst), 0);
    Ok(())
}
