//! Local speech recognition over a range of one source segment.
//!
//! [`transcribe_range`] drives two ports chunk by chunk: a
//! [`SpeechAudioSource`] that decodes bounded PCM for a planned chunk and a
//! [`SpeechRecognizer`] that turns it into bounded provider output. Every
//! decision about that output (validation, silence, seams) is the domain's;
//! this module sequences the stages, checks the recognizer's identity before
//! and after the run so output from two different models is never mixed, and
//! stops at the first typed failure without producing anything.
//! [`build_asr_revision`] then identifies the result as a transcript revision.

use std::{error::Error, fmt, future::Future, num::NonZeroU16, num::NonZeroU32};

use vsift_domain::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrProviderBuild, AsrRun,
    AsrRunParts, ChunkPlan, ChunkPlanError, LanguageTag, MediaTime, MergedSegment, PlannedChunk,
    ProviderChunkOutput, ProviderOutputError, SegmentOrigin, SessionId, SourceId, SourceSegment,
    TimeRange, TranscriptProvenance, TranscriptRevision, TranscriptRevisionError,
    TranscriptRevisionId, TranscriptRevisionParts, TranscriptSegment, TranscriptSegmentParts,
    TranscriptWarningKind, TranscriptWarnings, decoded_audio_range, is_silent_pcm, merge_chunks,
    plan_chunks, validate_chunk_output,
};

use crate::{
    TranscriptBuildError,
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
    /// The provider could not run or its output could not be read.
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
    /// A process or file could not be used.
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
            Self::Cancelled => "cancelled",
            Self::Deadline => "deadline",
            Self::Busy => "busy",
            Self::ResourceLimit => "resource_limit",
            Self::AudioUnavailable => "audio_unavailable",
            Self::ProviderFailed => "provider_failed",
            Self::UnparseableOutput => "unparseable_output",
            Self::MalformedOutput(_) => "malformed_output",
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
    let bounds = request.source_segment.range();
    if request.range.start() < bounds.start() || request.range.end() > bounds.end() {
        return Err(AsrFailure::at(
            AsrStage::Planning,
            AsrFailureReason::InvalidRange,
        ));
    }
    let chunks =
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
        })?;
    ensure_identity(recognizer, request.expected).await?;

    let mut records = Vec::with_capacity(chunks.len());
    let mut merge_input = Vec::with_capacity(chunks.len());
    let mut warnings = TranscriptWarnings::default();
    let mut languages = Vec::new();
    let mut gaps = 0_u32;
    let mut first_gap = None;
    for chunk in chunks {
        stop_if_cancelled(cancellation, AsrStage::AudioExtraction)?;
        let outcome = match audio.speech_pcm(&chunk).await {
            Ok(pcm) => {
                decoded_audio_range(pcm.actual_start, pcm.samples.len()).map(|range| (pcm, range))
            }
            Err(SpeechAudioError::NoAudio) => None,
            Err(error) => {
                return Err(AsrFailure::at(AsrStage::AudioExtraction, error.into()));
            }
        };
        let Some((pcm, decoded)) = outcome else {
            gaps = gaps.saturating_add(1);
            first_gap.get_or_insert(chunk.ordinal());
            records.push(AsrChunkRecord::new(chunk.clone(), AsrChunkOutcome::NoAudio));
            merge_input.push(empty_chunk(chunk));
            continue;
        };
        if is_silent_pcm(&pcm.samples) {
            gaps = gaps.saturating_add(1);
            first_gap.get_or_insert(chunk.ordinal());
            records.push(AsrChunkRecord::new(
                chunk.clone(),
                AsrChunkOutcome::Silent { audio: decoded },
            ));
            merge_input.push(empty_chunk(chunk));
            continue;
        }
        stop_if_cancelled(cancellation, AsrStage::Recognition)?;
        let output = recognizer
            .recognize(&chunk, &pcm)
            .await
            .map_err(|error| AsrFailure::at(AsrStage::Recognition, error.into()))?;
        let validated =
            validate_chunk_output(&chunk, decoded, bounds, output).map_err(|error| {
                AsrFailure::at(
                    AsrStage::OutputValidation,
                    AsrFailureReason::MalformedOutput(error),
                )
            })?;
        records.push(AsrChunkRecord::new(
            chunk,
            AsrChunkOutcome::Transcribed { audio: decoded },
        ));
        let (segments, language, chunk_warnings) = validated.into_parts();
        warnings.extend(&chunk_warnings);
        languages.push(language);
        merge_input.push(segments);
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
    Ok(AsrTranscription {
        run,
        language: agreed_language(languages),
        segments: merged.segments,
        warnings,
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
    /// The revision this one replaces, if any.
    pub supersedes: Option<TranscriptRevisionId>,
    /// The source range this revision replaced in the superseded one.
    pub replaced_range: Option<TimeRange>,
}

/// Identifies a completed run as a transcript revision.
///
/// The revision identity is derived from the session, revision number, source
/// segment, run provenance (provider, model, settings, plan, stream and covered
/// range) and what it supersedes, so the same run over the same audio always
/// names itself the same way. Segment identities follow from the revision and
/// ordinal, as for imports.
///
/// # Errors
///
/// Returns [`TranscriptBuildError::Invalid`] when the run found no speech
/// ([`TranscriptRevisionError::Empty`]) or the assembly violates an invariant.
pub fn build_asr_revision(
    request: AsrRevisionRequest<'_>,
) -> Result<TranscriptRevision, TranscriptBuildError> {
    let transcription = request.transcription;
    let run = &transcription.run;
    let covered = run.covered_range().ok_or(TranscriptBuildError::Invalid(
        TranscriptRevisionError::InvalidAsrRun,
    ))?;
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
            request.supersedes.as_ref().map_or("", |id| id.as_str()),
        ],
    ))
    .map_err(|_| TranscriptBuildError::Invalid(TranscriptRevisionError::InvalidIdentity))?;
    let mut segments = Vec::with_capacity(transcription.segments.len());
    for (index, merged) in transcription.segments.into_iter().enumerate() {
        let ordinal = u32::try_from(index + 1)
            .ok()
            .and_then(NonZeroU32::new)
            .ok_or(TranscriptBuildError::Invalid(
                TranscriptRevisionError::TooManySegments,
            ))?;
        let segment = merged.segment;
        segments.push(TranscriptSegment::new(TranscriptSegmentParts {
            id: transcript_segment_id(&revision_id, ordinal.get())
                .map_err(TranscriptBuildError::Invalid)?,
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
        }));
    }
    TranscriptRevision::new(TranscriptRevisionParts {
        id: revision_id,
        number: request.number,
        source_id: request.source_id.clone(),
        source_segment: request.source_segment.clone(),
        provenance: TranscriptProvenance::LocalAsr(transcription.run),
        supersedes: request.supersedes,
        replaced_range: request.replaced_range,
        language: transcription.language,
        segments,
        warnings: transcription.warnings,
    })
    .map_err(TranscriptBuildError::Invalid)
}

#[cfg(test)]
mod tests;
