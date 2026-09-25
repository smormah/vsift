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
    ProviderOutputError, ProviderSegment, ProviderToken, ProviderTokenKind, SegmentOrigin,
    SessionId, Sha256Hex, SidecarIdentity, SourceId, SourceSegment, SourceSegmentId, TimeRange,
    TranscriptFormat, TranscriptOffset, TranscriptProvenance, TranscriptRevision,
    TranscriptRevisionError, TranscriptWarningKind, TranscriptWarnings,
};

use super::{
    AsrCancellation, AsrFailure, AsrFailureReason, AsrRevisionRequest, AsrStage, AsrTranscription,
    RecognizerIdentity, RevisionSplice, SpeechAudioError, SpeechAudioSource, SpeechPcm,
    SpeechRecognitionError, SpeechRecognizer, TranscribeRangeRequest, build_asr_revision,
    transcribe_range,
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
            Audio::Fails(error) => Err(error),
        }
    }
}

/// Answers every chunk with one segment two seconds into its core.
struct FakeRecognizer {
    identities: Mutex<Vec<RecognizerIdentity>>,
    output: fn(&PlannedChunk) -> Result<ProviderChunkOutput, SpeechRecognitionError>,
    calls: AtomicUsize,
    cancel_after_first: Option<&'static AtomicBool>,
}

impl FakeRecognizer {
    fn new(identity: RecognizerIdentity) -> Self {
        Self {
            identities: Mutex::new(vec![identity]),
            output: one_segment,
            calls: AtomicUsize::new(0),
            cancel_after_first: None,
        }
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
        _pcm: &SpeechPcm,
    ) -> impl Future<Output = Result<ProviderChunkOutput, SpeechRecognitionError>> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
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
    assert_eq!(
        run(&audio, &malformed, &Flag(&NEVER)).await?,
        Err(AsrFailure {
            stage: AsrStage::OutputValidation,
            reason: AsrFailureReason::MalformedOutput(ProviderOutputError::OutOfOrderSegments),
        })
    );
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
