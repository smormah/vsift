//! Local speech recognition over a range of one source segment.
//!
//! [`transcribe_range`] drives two ports chunk by chunk: a
//! [`SpeechAudioSource`] that decodes bounded PCM for a planned chunk and a
//! [`SpeechRecognizer`] that turns it into bounded provider output. Every
//! decision about that output (validation, silence, seams) is the domain's;
//! this module sequences the stages, checks the recognizer's identity before
//! and after the run so output from two different models is never mixed, and
//! stops at the first typed failure without producing anything. Only a model
//! identified as a reviewed pinned profile may run (maintainer decision D5).
//! [`build_asr_revision`] then identifies the result as a transcript revision,
//! splicing it into the revision it supersedes when the run covered only part
//! of the source (decision D3, ADR 0017).

use std::{error::Error, fmt, future::Future, num::NonZeroU16, num::NonZeroU32};

use vsift_domain::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile,
    AsrProviderBuild, AsrRun, AsrRunParts, CarriedFrom, CheckpointOutcome, ChunkCheckpoint,
    ChunkPlan, ChunkPlanError, InheritedRevision, LanguageTag, MediaTime, MergedSegment,
    PlannedChunk, ProgressStage, ProgressUpdate, ProviderChunkOutput, ProviderOutputError,
    RecognitionKey, SegmentOrigin, SessionId, SourceId, SourceSegment, TimeRange,
    TranscriptProvenance, TranscriptRevision, TranscriptRevisionError, TranscriptRevisionId,
    TranscriptRevisionParts, TranscriptSegment, TranscriptSegmentParts, TranscriptWarningKind,
    TranscriptWarnings, ValidatedChunk, decoded_audio_range, is_silent_pcm, merge_chunks,
    plan_chunks, validate_chunk_output,
};

use crate::{
    TranscriptBuildError,
    job::{CheckpointRead, ChunkCheckpoints, NoCheckpoints},
    transcript::{derived_identity, transcript_segment_id},
};

/// Decoded mono 16 kHz signed 16-bit speech for one planned chunk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpeechPcm {
    /// Observed source time of the first decoded sample.
    ///
    /// This, not the requested window start, anchors provider times: a codec
    /// may start a stream late or decode from an earlier frame boundary.
    pub actual_start: MediaTime,
    /// Samples in time order.
    pub samples: Vec<i16>,
}

/// Why speech audio for a chunk could not be decoded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechAudioError {
    /// No sample was decoded in the window; the chunk is a gap with no audio.
    NoAudio,
    /// The selected audio stream is absent or cannot be decoded.
    Unavailable,
    /// Admission capacity was unavailable.
    Busy,
    /// Decoding exceeded its deadline.
    Deadline,
    /// Decoding was cancelled.
    Cancelled,
    /// Decoded output exceeded a bound.
    ResourceLimit,
    /// The decoder could not run or its output could not be read.
    Io,
}

impl fmt::Display for SpeechAudioError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoAudio => "no speech audio was decoded in the window",
            Self::Unavailable => "speech audio stream is unavailable",
            Self::Busy => "speech audio admission is busy",
            Self::Deadline => "speech audio decoding exceeded its deadline",
            Self::Cancelled => "speech audio decoding was cancelled",
            Self::ResourceLimit => "speech audio exceeded its bound",
            Self::Io => "speech audio could not be decoded",
        })
    }
}

impl Error for SpeechAudioError {}

/// Port that decodes bounded speech PCM for one planned chunk.
pub trait SpeechAudioSource: Send + Sync {
    /// Decodes at most the chunk's window of the selected audio stream.
    fn speech_pcm(
        &self,
        chunk: &PlannedChunk,
    ) -> impl Future<Output = Result<SpeechPcm, SpeechAudioError>> + Send;
}

/// Why a recognizer could not describe a chunk or itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechRecognitionError {
    /// The model file is missing or unreadable.
    ModelUnavailable,
    /// Admission capacity was unavailable.
    Busy,
    /// Recognition exceeded its deadline.
    Deadline,
    /// Recognition was cancelled.
    Cancelled,
    /// Provider output exceeded a bound.
    ResourceLimit,
    /// The provider exited unsuccessfully.
    ProviderFailed,
    /// The provider's output could not be parsed as its documented format.
    UnparseableOutput,
    /// The provider ended abnormally (a signal, or an operating-system crash
    /// status) rather than exiting; exhausted memory is the usual cause.
    AbnormalTermination,
    /// The private work directory, or a chunk file in it, could not be used.
    Workspace,
    /// The provider could not be started or identified.
    Io,
}

impl fmt::Display for SpeechRecognitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ModelUnavailable => "speech model is unavailable",
            Self::Busy => "speech recognition admission is busy",
            Self::Deadline => "speech recognition exceeded its deadline",
            Self::Cancelled => "speech recognition was cancelled",
            Self::ResourceLimit => "speech recognition output exceeded its bound",
            Self::ProviderFailed => "speech recognizer failed",
            Self::UnparseableOutput => "speech recognizer output is unparseable",
            Self::AbnormalTermination => "speech recognizer terminated abnormally",
            Self::Workspace => "speech recognition work directory could not be used",
            Self::Io => "speech recognizer could not run",
        })
    }
}

impl Error for SpeechRecognitionError {}

/// Exactly what would recognize speech: provider build, model and settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecognizerIdentity {
    /// Provider build.
    pub provider: AsrProviderBuild,
    /// Model file identity.
    pub model: AsrModel,
    /// Decoding settings.
    pub decoding: AsrDecodingProfile,
    /// Recognizer threads.
    pub threads: NonZeroU16,
}

/// Port that recognizes speech in one chunk's PCM.
pub trait SpeechRecognizer: Send + Sync {
    /// Identifies the provider build and model as they are now, from their bytes.
    fn identity(
        &self,
    ) -> impl Future<Output = Result<RecognizerIdentity, SpeechRecognitionError>> + Send;

    /// Recognizes `pcm` and returns the provider's bounded, parsed output.
    ///
    /// The output is structurally valid (bounded sizes, strict UTF-8 text) but
    /// not yet checked against the chunk; the use case applies the domain's
    /// output rules.
    fn recognize(
        &self,
        chunk: &PlannedChunk,
        pcm: &SpeechPcm,
    ) -> impl Future<Output = Result<ProviderChunkOutput, SpeechRecognitionError>> + Send;
}

/// Caller-controlled cancellation observed between stages.
pub trait AsrCancellation: Send + Sync {
    /// Whether the caller has asked the run to stop.
    fn is_cancelled(&self) -> bool;
}

/// The stage a local ASR run was in when it failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsrStage {
    /// Checking the range and cutting it into chunks.
    Planning,
    /// Identifying the recognizer before or after the run.
    RecognizerIdentity,
    /// Decoding a chunk's speech audio.
    AudioExtraction,
    /// Running the recognizer on a chunk.
    Recognition,
    /// Checking a chunk's provider output against its audio.
    OutputValidation,
    /// Assembling the run's provenance.
    Assembly,
}

impl AsrStage {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::RecognizerIdentity => "recognizer_identity",
            Self::AudioExtraction => "audio_extraction",
            Self::Recognition => "recognition",
            Self::OutputValidation => "output_validation",
            Self::Assembly => "assembly",
        }
    }
}

/// Why a local ASR run failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsrFailureReason {
    /// The range lies outside the source segment.
    InvalidRange,
    /// The range needs more chunks than one run may plan.
    TooManyChunks,
    /// The provider build, model or settings differ from those expected, or
    /// changed during the run; output from two models is never mixed.
    ModelChanged,
    /// The model file is missing or unreadable.
    ModelUnavailable,
    /// The model is not a reviewed pinned profile, so it is not run (D5).
    UnpinnedModel,
    /// The caller cancelled the run.
    Cancelled,
    /// A stage exceeded its deadline.
    Deadline,
    /// Admission capacity was unavailable.
    Busy,
    /// A stage's output exceeded a bound.
    ResourceLimit,
    /// The selected audio stream is absent or cannot be decoded.
    AudioUnavailable,
    /// The recognizer exited unsuccessfully.
    ProviderFailed,
    /// The recognizer's output could not be parsed.
    UnparseableOutput,
    /// The recognizer's output violated the domain's output rules.
    MalformedOutput(ProviderOutputError),
    /// The recognizer ended abnormally, usually because memory ran out.
    AbnormalTermination,
    /// The private work directory could not be used.
    Workspace,
    /// A provider process could not run, or its output or the source copy
    /// could not be read.
    Io,
    /// The assembled run violated an invariant; an internal fault.
    InvalidRun(TranscriptRevisionError),
}

impl AsrFailureReason {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::InvalidRange => "invalid_range",
            Self::TooManyChunks => "too_many_chunks",
            Self::ModelChanged => "model_changed",
            Self::ModelUnavailable => "model_unavailable",
            Self::UnpinnedModel => "unpinned_model",
            Self::Cancelled => "cancelled",
            Self::Deadline => "deadline",
            Self::Busy => "busy",
            Self::ResourceLimit => "resource_limit",
            Self::AudioUnavailable => "audio_unavailable",
            Self::ProviderFailed => "provider_failed",
            Self::UnparseableOutput => "unparseable_output",
            Self::MalformedOutput(_) => "malformed_output",
            Self::AbnormalTermination => "abnormal_termination",
            Self::Workspace => "workspace",
            Self::Io => "io",
            Self::InvalidRun(_) => "invalid_run",
        }
    }
}

/// A typed local ASR failure: where it happened and why.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AsrFailure {
    /// Stage that failed.
    pub stage: AsrStage,
    /// Why it failed.
    pub reason: AsrFailureReason,
}

impl AsrFailure {
    const fn at(stage: AsrStage, reason: AsrFailureReason) -> Self {
        Self { stage, reason }
    }
}

impl fmt::Display for AsrFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "local ASR failed at {} ({})",
            self.stage.identifier(),
            self.reason.identifier()
        )
    }
}

impl Error for AsrFailure {}

impl From<SpeechAudioError> for AsrFailureReason {
    fn from(error: SpeechAudioError) -> Self {
        match error {
            SpeechAudioError::NoAudio | SpeechAudioError::Unavailable => Self::AudioUnavailable,
            SpeechAudioError::Busy => Self::Busy,
            SpeechAudioError::Deadline => Self::Deadline,
            SpeechAudioError::Cancelled => Self::Cancelled,
            SpeechAudioError::ResourceLimit => Self::ResourceLimit,
            SpeechAudioError::Io => Self::Io,
        }
    }
}

impl From<SpeechRecognitionError> for AsrFailureReason {
    fn from(error: SpeechRecognitionError) -> Self {
        match error {
            SpeechRecognitionError::ModelUnavailable => Self::ModelUnavailable,
            SpeechRecognitionError::Busy => Self::Busy,
            SpeechRecognitionError::Deadline => Self::Deadline,
            SpeechRecognitionError::Cancelled => Self::Cancelled,
            SpeechRecognitionError::ResourceLimit => Self::ResourceLimit,
            SpeechRecognitionError::ProviderFailed => Self::ProviderFailed,
            SpeechRecognitionError::UnparseableOutput => Self::UnparseableOutput,
            SpeechRecognitionError::AbnormalTermination => Self::AbnormalTermination,
            SpeechRecognitionError::Workspace => Self::Workspace,
            SpeechRecognitionError::Io => Self::Io,
        }
    }
}

/// What to transcribe and with which recognizer.
#[derive(Clone, Copy, Debug)]
pub struct TranscribeRangeRequest<'a> {
    /// Source segment the range belongs to.
    pub source_segment: &'a SourceSegment,
    /// Half-open source range to transcribe.
    pub range: TimeRange,
    /// How the range is chunked.
    pub plan: ChunkPlan,
    /// Original index of the audio stream the audio source decodes.
    pub audio_stream: u32,
    /// The recognizer identity the caller selected and verified.
    pub expected: &'a RecognizerIdentity,
}

/// A completed local ASR run: its provenance and merged segments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsrTranscription {
    /// Run provenance with every chunk's outcome.
    pub run: AsrRun,
    /// Language every transcribed chunk agreed on, if any.
    pub language: Option<LanguageTag>,
    /// Merged segments in source start order.
    pub segments: Vec<MergedSegment>,
    /// Rejections, trims, removed markers, silent chunks and seam duplicates.
    pub warnings: TranscriptWarnings,
}

/// Transcribes `range` chunk by chunk and merges the result (T-03, T-05, T-06).
///
/// Only a model identified as a reviewed pinned profile runs: an
/// [`AsrModelProfile::Unreviewed`] `expected` model is refused before any
/// work (maintainer decision D5, ADR 0017), because nothing about an
/// unreviewed model's accuracy, licence or resource use is known.
///
/// The recognizer's identity is read before the first chunk and after the
/// last and must equal `expected` both times, so a model or binary swapped
/// during the run fails it rather than mixing outputs. A chunk whose decoded
/// audio is silent is recorded as a silent gap without running the
/// recognizer; a chunk with no decoded audio is recorded as a gap. Nothing is
/// returned unless every chunk succeeded: a failure or cancellation discards
/// all work, so a caller never commits part of a run.
///
/// # Errors
///
/// Returns the first [`AsrFailure`], naming its stage.
pub async fn transcribe_range<A, R, C>(
    request: TranscribeRangeRequest<'_>,
    audio: &A,
    recognizer: &R,
    cancellation: &C,
) -> Result<AsrTranscription, AsrFailure>
where
    A: SpeechAudioSource,
    R: SpeechRecognizer,
    C: AsrCancellation,
{
    run_chunks(
        request,
        None::<CheckpointScope<'_, NoCheckpoints>>,
        audio,
        recognizer,
        cancellation,
    )
    .await
    .map(|(transcription, _)| transcription)
    .map_err(|failure| failure.failure)
}

/// Where a long run reports how far it has come (P11, ADR 0021).
///
/// A report is advisory: a host may coalesce or drop it to keep its output
/// bounded, and it must return at once, because the run calls it between
/// chunks. `Send + Sync` so a run holding it stays a `Send` future.
pub trait ProgressSink: Send + Sync {
    /// Receives one observation.
    fn report(&self, update: ProgressUpdate);
}

/// A sink that reports nowhere, for runs nobody watches.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoProgress;

impl ProgressSink for NoProgress {
    fn report(&self, _update: ProgressUpdate) {}
}

/// The chunk checkpoints a run reads and writes, the run they belong to,
/// and where the job reports its progress.
pub struct CheckpointScope<'a, K> {
    /// Where the job keeps its checkpoints.
    pub checkpoints: &'a K,
    /// The recognition every usable checkpoint must belong to.
    pub key: &'a RecognitionKey,
    /// Receives `recognising_speech` progress: 0 of the planned chunks once
    /// the plan is made, then one more after every chunk, reused or fresh.
    pub progress: &'a dyn ProgressSink,
}

// Two references: copyable whatever `K` is, which a derive would not allow.
impl<K> Clone for CheckpointScope<'_, K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for CheckpointScope<'_, K> {}

/// How a run used its chunk checkpoints.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CheckpointUse {
    /// Chunks taken from a stored checkpoint instead of being decoded and
    /// recognised again.
    pub reused: u32,
    /// Checkpoints found unusable (damaged, another run's, or holding output
    /// the domain rules reject) and done again.
    pub discarded: u32,
}

/// A failed run and the chunk it failed at, if it was working on one.
///
/// The chunk lets the job apply the poison rule: three identical failures at
/// the same chunk make the job non-resumable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AsrRunFailure {
    /// Stage and reason.
    pub failure: AsrFailure,
    /// Zero-based index of the chunk.
    pub chunk: Option<u32>,
}

impl fmt::Display for AsrRunFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.failure.fmt(formatter)
    }
}

impl Error for AsrRunFailure {}

impl From<AsrFailure> for AsrRunFailure {
    fn from(failure: AsrFailure) -> Self {
        Self {
            failure,
            chunk: None,
        }
    }
}

/// [`transcribe_range`] that continues from the chunk checkpoints of an
/// earlier, interrupted run (P10, ADR 0020).
///
/// Before a chunk is decoded its checkpoint is read. A checkpoint of this
/// run (the same recognition key, index and window) is used in place of
/// decoding and recognising: a silent or empty chunk as recorded, and stored
/// recognizer output through exactly the validation and merge a fresh run
/// applies, so an interrupted and resumed run yields the same revision as an
/// uninterrupted one. A checkpoint that is unreadable, of another run, or
/// whose output the rules reject is removed and the chunk done again; it is
/// counted in [`CheckpointUse::discarded`], never guessed at (S-08). After a
/// chunk is done fresh its outcome (the raw output, once it has passed
/// validation) is stored before the next chunk starts; a checkpoint that
/// cannot be stored costs only the redo.
///
/// # Errors
///
/// As [`transcribe_range`], with the chunk the run failed at.
pub async fn transcribe_range_checkpointed<A, R, C, K>(
    request: TranscribeRangeRequest<'_>,
    scope: CheckpointScope<'_, K>,
    audio: &A,
    recognizer: &R,
    cancellation: &C,
) -> Result<(AsrTranscription, CheckpointUse), AsrRunFailure>
where
    A: SpeechAudioSource,
    R: SpeechRecognizer,
    C: AsrCancellation,
    K: ChunkCheckpoints,
{
    run_chunks(request, Some(scope), audio, recognizer, cancellation).await
}

/// What one chunk became.
enum ChunkResult {
    /// No audio, or silent audio: a gap.
    Gap(AsrChunkOutcome),
    /// Recognised output that passed validation.
    Transcribed {
        audio: TimeRange,
        validated: ValidatedChunk,
    },
}

async fn run_chunks<A, R, C, K>(
    request: TranscribeRangeRequest<'_>,
    scope: Option<CheckpointScope<'_, K>>,
    audio: &A,
    recognizer: &R,
    cancellation: &C,
) -> Result<(AsrTranscription, CheckpointUse), AsrRunFailure>
where
    A: SpeechAudioSource,
    R: SpeechRecognizer,
    C: AsrCancellation,
    K: ChunkCheckpoints,
{
    let bounds = request.source_segment.range();
    let chunks = plan_run(&request)?;
    ensure_identity(recognizer, request.expected).await?;
    let total = u64::try_from(chunks.len()).ok();
    let report = |completed: u64| {
        if let Some(scope) = &scope {
            scope.progress.report(ProgressUpdate {
                stage: ProgressStage::RecognisingSpeech,
                completed,
                total,
            });
        }
    };
    report(0);
    let mut completed = 0_u64;

    let mut usage = CheckpointUse::default();
    let mut records = Vec::with_capacity(chunks.len());
    let mut merge_input = Vec::with_capacity(chunks.len());
    let mut warnings = TranscriptWarnings::default();
    let mut languages = Vec::new();
    let mut gaps = 0_u32;
    let mut first_gap = None;
    for chunk in chunks {
        let index = chunk.index();
        let at_chunk = |failure: AsrFailure| AsrRunFailure {
            failure,
            chunk: Some(index),
        };
        stop_if_cancelled(cancellation, AsrStage::AudioExtraction).map_err(at_chunk)?;
        let reused = scope
            .as_ref()
            .and_then(|scope| reuse_checkpoint(scope, &chunk, bounds, &mut usage));
        let result = if let Some(result) = reused {
            result
        } else {
            let (result, checkpoint) = fresh_chunk(&chunk, bounds, audio, recognizer, cancellation)
                .await
                .map_err(at_chunk)?;
            if let Some(scope) = &scope {
                // Best effort by design: see the port's documentation.
                let _ = scope.checkpoints.store(&ChunkCheckpoint::new(
                    scope.key.clone(),
                    &chunk,
                    checkpoint,
                ));
            }
            result
        };
        match result {
            ChunkResult::Gap(outcome) => {
                gaps = gaps.saturating_add(1);
                first_gap.get_or_insert(chunk.ordinal());
                records.push(AsrChunkRecord::new(chunk.clone(), outcome));
                merge_input.push(empty_chunk(chunk));
            }
            ChunkResult::Transcribed { audio, validated } => {
                records.push(AsrChunkRecord::new(
                    chunk,
                    AsrChunkOutcome::Transcribed { audio },
                ));
                let (segments, language, chunk_warnings) = validated.into_parts();
                warnings.extend(&chunk_warnings);
                languages.push(language);
                merge_input.push(segments);
            }
        }
        completed = completed.saturating_add(1);
        report(completed);
    }
    if let Some(first) = first_gap {
        warnings.add(TranscriptWarningKind::SilentChunksSkipped, gaps, first);
    }
    stop_if_cancelled(cancellation, AsrStage::RecognizerIdentity)?;
    ensure_identity(recognizer, request.expected).await?;

    let merged = merge_chunks(&merge_input);
    warnings.extend(&merged.warnings);
    let run = AsrRun::new(AsrRunParts {
        provider: request.expected.provider.clone(),
        model: request.expected.model.clone(),
        decoding: request.expected.decoding,
        plan: request.plan,
        threads: request.expected.threads,
        audio_stream: request.audio_stream,
        chunks: records,
    })
    .map_err(|error| AsrFailure::at(AsrStage::Assembly, AsrFailureReason::InvalidRun(error)))?;
    Ok((
        AsrTranscription {
            run,
            language: agreed_language(languages),
            segments: merged.segments,
            warnings,
        },
        usage,
    ))
}

/// The chunk's stored result, if a usable checkpoint of this run holds it.
///
/// Anything else found is removed and counted as discarded, so the chunk is
/// done again from the audio.
fn reuse_checkpoint<K: ChunkCheckpoints>(
    scope: &CheckpointScope<'_, K>,
    chunk: &PlannedChunk,
    bounds: TimeRange,
    usage: &mut CheckpointUse,
) -> Option<ChunkResult> {
    let checkpoint = match scope.checkpoints.load(chunk.index()) {
        CheckpointRead::Absent => return None,
        CheckpointRead::Unusable => {
            usage.discarded = usage.discarded.saturating_add(1);
            return None;
        }
        CheckpointRead::Found(checkpoint) => checkpoint,
    };
    let result = if checkpoint.belongs_to(scope.key, chunk) {
        match checkpoint.into_outcome() {
            CheckpointOutcome::NoAudio => Some(ChunkResult::Gap(AsrChunkOutcome::NoAudio)),
            CheckpointOutcome::Silent { audio } => {
                Some(ChunkResult::Gap(AsrChunkOutcome::Silent { audio }))
            }
            CheckpointOutcome::Recognised { audio, output } => {
                validate_chunk_output(chunk, audio, bounds, output)
                    .ok()
                    .map(|validated| ChunkResult::Transcribed { audio, validated })
            }
        }
    } else {
        None
    };
    if result.is_some() {
        usage.reused = usage.reused.saturating_add(1);
    } else {
        scope.checkpoints.discard(chunk.index());
        usage.discarded = usage.discarded.saturating_add(1);
    }
    result
}

/// Decodes and, unless it is silent, recognises one chunk; returns what it
/// became and the checkpoint outcome that records it.
async fn fresh_chunk<A, R, C>(
    chunk: &PlannedChunk,
    bounds: TimeRange,
    audio: &A,
    recognizer: &R,
    cancellation: &C,
) -> Result<(ChunkResult, CheckpointOutcome), AsrFailure>
where
    A: SpeechAudioSource,
    R: SpeechRecognizer,
    C: AsrCancellation,
{
    let decoded = match audio.speech_pcm(chunk).await {
        Ok(pcm) => {
            decoded_audio_range(pcm.actual_start, pcm.samples.len()).map(|range| (pcm, range))
        }
        Err(SpeechAudioError::NoAudio) => None,
        Err(error) => {
            return Err(AsrFailure::at(AsrStage::AudioExtraction, error.into()));
        }
    };
    let Some((pcm, decoded)) = decoded else {
        return Ok((
            ChunkResult::Gap(AsrChunkOutcome::NoAudio),
            CheckpointOutcome::NoAudio,
        ));
    };
    if is_silent_pcm(&pcm.samples) {
        return Ok((
            ChunkResult::Gap(AsrChunkOutcome::Silent { audio: decoded }),
            CheckpointOutcome::Silent { audio: decoded },
        ));
    }
    stop_if_cancelled(cancellation, AsrStage::Recognition)?;
    let output = recognizer
        .recognize(chunk, &pcm)
        .await
        .map_err(|error| AsrFailure::at(AsrStage::Recognition, error.into()))?;
    let raw = output.clone();
    let validated = validate_chunk_output(chunk, decoded, bounds, output).map_err(|error| {
        AsrFailure::at(
            AsrStage::OutputValidation,
            AsrFailureReason::MalformedOutput(error),
        )
    })?;
    Ok((
        ChunkResult::Transcribed {
            audio: decoded,
            validated,
        },
        CheckpointOutcome::Recognised {
            audio: decoded,
            output: raw,
        },
    ))
}

/// Refuses a range outside the source and an unpinned model, then cuts the
/// range into chunks; nothing has run yet.
fn plan_run(request: &TranscribeRangeRequest<'_>) -> Result<Vec<PlannedChunk>, AsrFailure> {
    let bounds = request.source_segment.range();
    if request.range.start() < bounds.start() || request.range.end() > bounds.end() {
        return Err(AsrFailure::at(
            AsrStage::Planning,
            AsrFailureReason::InvalidRange,
        ));
    }
    if request.expected.model.profile() == AsrModelProfile::Unreviewed {
        return Err(AsrFailure::at(
            AsrStage::RecognizerIdentity,
            AsrFailureReason::UnpinnedModel,
        ));
    }
    plan_chunks(request.source_segment.id(), request.range, request.plan).map_err(|error| {
        AsrFailure::at(
            AsrStage::Planning,
            match error {
                ChunkPlanError::TooManyChunks => AsrFailureReason::TooManyChunks,
                ChunkPlanError::InvalidWindow | ChunkPlanError::InvalidOverlap => {
                    AsrFailureReason::InvalidRange
                }
            },
        )
    })
}

async fn ensure_identity<R: SpeechRecognizer>(
    recognizer: &R,
    expected: &RecognizerIdentity,
) -> Result<(), AsrFailure> {
    let actual = recognizer
        .identity()
        .await
        .map_err(|error| AsrFailure::at(AsrStage::RecognizerIdentity, error.into()))?;
    if actual != *expected {
        return Err(AsrFailure::at(
            AsrStage::RecognizerIdentity,
            AsrFailureReason::ModelChanged,
        ));
    }
    Ok(())
}

fn stop_if_cancelled<C: AsrCancellation>(
    cancellation: &C,
    stage: AsrStage,
) -> Result<(), AsrFailure> {
    if cancellation.is_cancelled() {
        return Err(AsrFailure::at(stage, AsrFailureReason::Cancelled));
    }
    Ok(())
}

const fn empty_chunk(chunk: PlannedChunk) -> vsift_domain::ChunkSegments {
    vsift_domain::ChunkSegments {
        chunk,
        segments: Vec::new(),
    }
}

/// The language every transcribed chunk reported, or `None` when any chunk
/// reported none or they disagree.
fn agreed_language(languages: Vec<Option<LanguageTag>>) -> Option<LanguageTag> {
    let mut agreed: Option<LanguageTag> = None;
    for language in languages {
        let language = language?;
        match &agreed {
            Some(existing) if *existing != language => return None,
            Some(_) => {}
            None => agreed = Some(language),
        }
    }
    agreed
}

/// The revision a retranscription supersedes and the range it replaces there.
#[derive(Clone, Copy, Debug)]
pub struct RevisionSplice<'a> {
    /// The superseded revision: the session's newest when the run started.
    pub base: &'a TranscriptRevision,
    /// The replaced range. It is the requested range widened to whole base
    /// segments ([`TranscriptRevision::snap_to_segments`]) and must be exactly
    /// the range the run covered.
    pub replaced_range: TimeRange,
}

/// Inputs that identify and place one local-ASR revision.
#[derive(Clone, Debug)]
pub struct AsrRevisionRequest<'a> {
    /// Session that will hold the revision.
    pub session_id: &'a SessionId,
    /// Source the revision describes.
    pub source_id: &'a SourceId,
    /// Source segment the run covered.
    pub source_segment: &'a SourceSegment,
    /// Revision number within the session.
    pub number: NonZeroU32,
    /// The completed run.
    pub transcription: AsrTranscription,
    /// The revision this one supersedes and splices into, if the session
    /// already had one.
    pub splice: Option<RevisionSplice<'a>>,
}

/// The range a retranscription replaces: the whole source without a
/// request, the request itself when there is no revision yet, and otherwise
/// the request widened to whole segments of the newest revision
/// ([`TranscriptRevision::snap_to_segments`]).
///
/// A job commits only if this range is unchanged when it commits: widening
/// over a newer revision that another run committed meanwhile may cut
/// different segments.
#[must_use]
pub fn retranscription_range(
    newest: Option<&TranscriptRevision>,
    requested: Option<TimeRange>,
    source: TimeRange,
) -> TimeRange {
    match (newest, requested) {
        (_, None) => source,
        (Some(newest), Some(range)) => newest.snap_to_segments(range),
        (None, Some(range)) => range,
    }
}

/// One segment of a revision being assembled, before ordinals are assigned.
enum Assembled<'a> {
    /// Recognised by this run.
    Own(MergedSegment),
    /// Carried unchanged from the superseded revision.
    Carried(&'a TranscriptSegment, CarriedFrom),
}

impl Assembled<'_> {
    fn order_key(&self) -> (MediaTime, MediaTime, u8) {
        match self {
            Self::Carried(segment, _) => (segment.range().start(), segment.range().end(), 0),
            Self::Own(merged) => (
                merged.segment.range().start(),
                merged.segment.range().end(),
                1,
            ),
        }
    }
}

/// Identifies a completed run as a transcript revision.
///
/// The revision identity is derived from the session, revision number, source
/// segment, run provenance (provider, model, settings, plan, stream and covered
/// range) and what it supersedes, so the same run over the same audio always
/// names itself the same way. Segment identities follow from the revision and
/// ordinal, as for imports.
///
/// With a [`RevisionSplice`] the result is a complete revision (D3): every
/// segment of the superseded revision outside the replaced range is carried
/// with its original text, timing and provenance, and the run's segments fill
/// the range. Carried segments get new identities in this revision, so the
/// superseded revision's records stay valid and are never overwritten, and
/// each names the revision and segment that first produced it
/// ([`CarriedFrom`]). A run that recognised no speech still yields a revision
/// (with no new segment and a [`TranscriptWarningKind::NoSpeechRecognised`]
/// warning), so the attempt and its chunk outcomes are recorded.
///
/// # Errors
///
/// Returns [`TranscriptBuildError::Invalid`] when the splice does not match
/// the run (another source, a different range, or a revision number that does
/// not follow the superseded one) or the assembly violates an invariant.
pub fn build_asr_revision(
    request: AsrRevisionRequest<'_>,
) -> Result<TranscriptRevision, TranscriptBuildError> {
    let transcription = request.transcription;
    let run = &transcription.run;
    let covered = run.covered_range().ok_or(TranscriptBuildError::Invalid(
        TranscriptRevisionError::InvalidAsrRun,
    ))?;
    if let Some(splice) = &request.splice
        && (splice.replaced_range != covered
            || splice.base.source_id() != request.source_id
            || splice.base.source_segment() != request.source_segment
            || request.number.get() <= splice.base.number())
    {
        return Err(TranscriptBuildError::Invalid(
            TranscriptRevisionError::InvalidSupersession,
        ));
    }
    let supersedes = request.splice.map(|splice| splice.base.id().clone());
    let revision_id = TranscriptRevisionId::parse(derived_identity(
        "trv_",
        "vsift.transcript-revision.local-asr.v1",
        &[
            request.session_id.as_str(),
            &request.number.get().to_string(),
            request.source_segment.id().as_str(),
            run.provider().provider().identifier(),
            run.provider().executable_sha256().as_str(),
            run.model().profile().identifier(),
            run.model().sha256().as_str(),
            run.decoding().identifier(),
            &run.plan().window_us().to_string(),
            &run.plan().overlap_us().to_string(),
            &run.threads().get().to_string(),
            &run.audio_stream().to_string(),
            &covered.start().as_micros().to_string(),
            &covered.end().as_micros().to_string(),
            supersedes.as_ref().map_or("", |id| id.as_str()),
        ],
    ))
    .map_err(|_| TranscriptBuildError::Invalid(TranscriptRevisionError::InvalidIdentity))?;
    let mut warnings = transcription.warnings;
    if transcription.segments.is_empty() {
        warnings.add(
            TranscriptWarningKind::NoSpeechRecognised,
            1,
            NonZeroU32::MIN,
        );
    }
    let (mut assembled, inherited) = match request.splice {
        Some(splice) => carried_segments(splice)?,
        None => (Vec::new(), Vec::new()),
    };
    assembled.extend(transcription.segments.into_iter().map(Assembled::Own));
    // Stable: ties keep carried segments first, then provider order.
    assembled.sort_by_key(Assembled::order_key);
    let segments = identified_segments(&revision_id, assembled)?;
    TranscriptRevision::new(TranscriptRevisionParts {
        id: revision_id,
        number: request.number,
        source_id: request.source_id.clone(),
        source_segment: request.source_segment.clone(),
        provenance: TranscriptProvenance::LocalAsr(transcription.run),
        supersedes,
        replaced_range: request.splice.map(|splice| splice.replaced_range),
        inherited,
        language: transcription.language,
        segments,
        warnings,
    })
    .map_err(TranscriptBuildError::Invalid)
}

/// Gives assembled segments their ordinals and identities in `revision_id`.
fn identified_segments(
    revision_id: &TranscriptRevisionId,
    assembled: Vec<Assembled<'_>>,
) -> Result<Vec<TranscriptSegment>, TranscriptBuildError> {
    let mut segments = Vec::with_capacity(assembled.len());
    for (index, entry) in assembled.into_iter().enumerate() {
        let ordinal = u32::try_from(index + 1)
            .ok()
            .and_then(NonZeroU32::new)
            .ok_or(TranscriptBuildError::Invalid(
                TranscriptRevisionError::TooManySegments,
            ))?;
        let id = transcript_segment_id(revision_id, ordinal.get())
            .map_err(TranscriptBuildError::Invalid)?;
        segments.push(match entry {
            Assembled::Own(merged) => {
                let segment = merged.segment;
                TranscriptSegment::new(TranscriptSegmentParts {
                    id,
                    ordinal,
                    range: segment.range(),
                    text: segment.text().clone(),
                    speaker: None,
                    confidence: segment.confidence(),
                    origin: SegmentOrigin::Asr {
                        chunk: merged.chunk,
                        provider_start: segment.provider_start(),
                        provider_end: segment.provider_end(),
                        trimmed: segment.trimmed(),
                    },
                })
            }
            Assembled::Carried(segment, carried_from) => {
                TranscriptSegment::new(TranscriptSegmentParts {
                    id,
                    ordinal,
                    range: segment.range(),
                    text: segment.text().clone(),
                    speaker: segment.speaker().cloned(),
                    confidence: segment.confidence(),
                    origin: segment.origin(),
                })
                .with_carried_from(carried_from)
            }
        });
    }
    Ok(segments)
}

/// The superseded revision's segments outside the replaced range, each naming
/// its originating revision and segment, and the provenance of every revision
/// they originate in, in order of first use.
fn carried_segments(
    splice: RevisionSplice<'_>,
) -> Result<(Vec<Assembled<'_>>, Vec<InheritedRevision>), TranscriptBuildError> {
    let base = splice.base;
    let mut carried = Vec::new();
    let mut inherited: Vec<InheritedRevision> = Vec::new();
    for segment in base.segments() {
        if segment.intersects(splice.replaced_range) {
            continue;
        }
        let (origin, source) = match segment.carried_from() {
            Some(carried_from) => (
                carried_from.clone(),
                base.inherited()
                    .iter()
                    .find(|entry| entry.id() == carried_from.revision())
                    .cloned()
                    .ok_or(TranscriptBuildError::Invalid(
                        TranscriptRevisionError::InvalidCarriedSegment,
                    ))?,
            ),
            None => (
                CarriedFrom::new(base.id().clone(), segment.id().clone()),
                InheritedRevision::new(
                    base.id().clone(),
                    base.provenance().clone(),
                    base.language().cloned(),
                ),
            ),
        };
        if !inherited.iter().any(|entry| entry.id() == source.id()) {
            inherited.push(source);
        }
        carried.push(Assembled::Carried(segment, origin));
    }
    Ok((carried, inherited))
}

#[cfg(test)]
mod tests;
