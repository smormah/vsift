//! Versioned storage record of one transcript revision.
//!
//! A committed revision is stored as one immutable, content-addressed session
//! artifact of kind `transcript_record` and travels unchanged into retained
//! bundles. Decoding is strict (unknown fields, versions and values are
//! rejected) and rebuilds the revision through the domain constructors, so a
//! modified record cannot bypass the rules that produced it.
//!
//! Two versions exist. Version 1 records an imported revision and is still
//! the only version an import writes, byte for byte as before, so published
//! bundles and the v1 bundle schema stay valid. Version 2 records a revision
//! produced by local speech recognition: its run provenance, chunk outcomes
//! and each segment's provider times. Readers accept both. Imported revisions
//! carry no stored confidence: the domain fixes it to unknown.

use std::num::{NonZeroU16, NonZeroU32};

use serde::{Deserialize, Serialize};
use vsift_application::SessionStorageError;
use vsift_domain::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, ChunkPlan, ChunkTime, Confidence, ConfidenceOrigin,
    CueSource, CueText, CueTiming, LanguageTag, LanguageTagError, MediaTime, PlannedChunk,
    ProviderEndTrim, SegmentOrigin, Sha256Hex, SidecarIdentity, SourceId, SourceSegment,
    SourceSegmentId, SourceSegmentState, SpeakerLabel, TimeRange, TranscriptFormat,
    TranscriptOffset, TranscriptProvenance, TranscriptRevision, TranscriptRevisionId,
    TranscriptRevisionParts, TranscriptSegment, TranscriptSegmentId, TranscriptSegmentParts,
    TranscriptWarning, TranscriptWarningKind, TranscriptWarnings,
};

/// Largest encoded transcript record a session will store or read.
///
/// Three times the supplied-file limit leaves room for the original text kept
/// beside marked-up cues; an import whose record would be larger is rejected
/// with a resource limit rather than stored partially.
pub const MAX_TRANSCRIPT_RECORD_BYTES: usize = 24 * 1024 * 1024;
const IMPORTED_RECORD_VERSION: u16 = 1;
const LOCAL_ASR_RECORD_VERSION: u16 = 2;
const NEWEST_RECORD_VERSION: u16 = LOCAL_ASR_RECORD_VERSION;
const RECORD_FORMAT: &str = "vsift.transcript_record";

/// Encodes a revision as its bounded storage record.
///
/// An imported revision is written as version 1, exactly as before local ASR
/// existed; a local-ASR revision is written as version 2.
///
/// # Errors
///
/// Returns [`SessionStorageError::CapacityExhausted`] when the record would
/// exceed [`MAX_TRANSCRIPT_RECORD_BYTES`].
pub fn encode_transcript_record(
    revision: &TranscriptRevision,
) -> Result<Vec<u8>, SessionStorageError> {
    let bytes = match revision.provenance() {
        TranscriptProvenance::Imported {
            format,
            sidecar,
            offset,
        } => serde_json::to_vec(&StoredTranscript::from_revision(
            revision, *format, sidecar, *offset,
        )),
        TranscriptProvenance::LocalAsr(run) => {
            serde_json::to_vec(&StoredAsrTranscript::from_revision(revision, run))
        }
    }
    .map_err(|_| SessionStorageError::Io)?;
    if bytes.len() > MAX_TRANSCRIPT_RECORD_BYTES {
        return Err(SessionStorageError::CapacityExhausted);
    }
    Ok(bytes)
}

/// Decodes and revalidates a stored revision of either version.
///
/// # Errors
///
/// Returns [`SessionStorageError::UnsupportedVersion`] for a newer record and
/// [`SessionStorageError::IntegrityFailure`] for anything malformed, oversized
/// or violating a domain invariant.
pub fn decode_transcript_record(bytes: &[u8]) -> Result<TranscriptRevision, SessionStorageError> {
    #[derive(Deserialize)]
    struct VersionProbe {
        schema_version: u16,
    }
    if bytes.len() > MAX_TRANSCRIPT_RECORD_BYTES {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let probe: VersionProbe =
        serde_json::from_slice(bytes).map_err(|_| SessionStorageError::IntegrityFailure)?;
    let revision = match probe.schema_version {
        IMPORTED_RECORD_VERSION => serde_json::from_slice::<StoredTranscript>(bytes)
            .ok()
            .and_then(StoredTranscript::into_revision),
        LOCAL_ASR_RECORD_VERSION => serde_json::from_slice::<StoredAsrTranscript>(bytes)
            .ok()
            .and_then(StoredAsrTranscript::into_revision),
        newer if newer > NEWEST_RECORD_VERSION => {
            return Err(SessionStorageError::UnsupportedVersion);
        }
        _ => None,
    };
    revision.ok_or(SessionStorageError::IntegrityFailure)
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredTranscript {
    schema_version: u16,
    format: String,
    revision_id: String,
    revision: u32,
    source_id: String,
    source_segment: StoredSourceSegment,
    origin: StoredOrigin,
    offset_us: i64,
    sidecar: StoredSidecar,
    language: Option<String>,
    warnings: Vec<StoredWarning>,
    segments: Vec<StoredSegment>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredSourceSegment {
    id: String,
    index: u32,
    start_us: u64,
    end_us: u64,
    state: StoredSegmentState,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredSegmentState {
    Closed,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredOrigin {
    ImportedSrt,
    ImportedWebvtt,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredSidecar {
    sha256: String,
    bytes: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredWarning {
    kind: StoredWarningKind,
    count: u32,
    first_cue: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredWarningKind {
    EmptyCuesSkipped,
    MarkupRemoved,
    SpeakerLabelDiscarded,
    OverlappingCues,
    CuesOutsideSource,
    CuesCrossingSourceBoundary,
    ProviderSegmentsRejected,
    ProviderEndTrimmed,
    NonSpeechMarkersRemoved,
    SeamDuplicatesRemoved,
    SilentChunksSkipped,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredSegment {
    id: String,
    ordinal: u32,
    start_us: u64,
    end_us: u64,
    text: String,
    original_text: Option<String>,
    speaker: Option<String>,
    cue_ordinal: u32,
    cue_line: u32,
    cue_start_us: u64,
    cue_end_us: u64,
}

impl StoredTranscript {
    fn from_revision(
        revision: &TranscriptRevision,
        format: TranscriptFormat,
        sidecar: &SidecarIdentity,
        offset: TranscriptOffset,
    ) -> Self {
        Self {
            schema_version: IMPORTED_RECORD_VERSION,
            format: RECORD_FORMAT.to_owned(),
            revision_id: revision.id().as_str().to_owned(),
            revision: revision.number(),
            source_id: revision.source_id().as_str().to_owned(),
            source_segment: StoredSourceSegment::from_domain(revision.source_segment()),
            origin: match format {
                TranscriptFormat::Srt => StoredOrigin::ImportedSrt,
                TranscriptFormat::WebVtt => StoredOrigin::ImportedWebvtt,
            },
            offset_us: offset.as_micros(),
            sidecar: StoredSidecar {
                sha256: sidecar.sha256().to_owned(),
                bytes: sidecar.bytes(),
            },
            language: revision.language().map(|tag| tag.as_str().to_owned()),
            warnings: stored_warnings(revision.warnings()),
            segments: revision
                .segments()
                .iter()
                .filter_map(|segment| {
                    let SegmentOrigin::ImportedCue { cue, timing } = segment.origin() else {
                        // The domain guarantees an imported revision holds only cues.
                        return None;
                    };
                    Some(StoredSegment {
                        id: segment.id().as_str().to_owned(),
                        ordinal: segment.ordinal(),
                        start_us: segment.range().start().as_micros(),
                        end_us: segment.range().end().as_micros(),
                        text: segment.text().text().to_owned(),
                        original_text: segment.text().original().map(str::to_owned),
                        speaker: segment.speaker().map(|label| label.as_str().to_owned()),
                        cue_ordinal: cue.ordinal(),
                        cue_line: cue.line(),
                        cue_start_us: timing.start_micros(),
                        cue_end_us: timing.end_micros(),
                    })
                })
                .collect(),
        }
    }

    fn into_revision(self) -> Option<TranscriptRevision> {
        if self.schema_version != IMPORTED_RECORD_VERSION || self.format != RECORD_FORMAT {
            return None;
        }
        let source_segment = self.source_segment.into_domain()?;
        let revision_id = TranscriptRevisionId::parse(self.revision_id).ok()?;
        let mut segments = Vec::with_capacity(self.segments.len());
        for segment in self.segments {
            segments.push(segment.into_domain()?);
        }
        let warnings = domain_warnings(self.warnings)?;
        // Version 1 is the import record: local-ASR warning kinds cannot occur in it.
        if warnings
            .as_slice()
            .iter()
            .any(|warning| warning.kind().is_local_asr())
        {
            return None;
        }
        TranscriptRevision::new(TranscriptRevisionParts {
            id: revision_id,
            number: NonZeroU32::new(self.revision)?,
            source_id: SourceId::parse(self.source_id).ok()?,
            source_segment,
            provenance: TranscriptProvenance::Imported {
                format: match self.origin {
                    StoredOrigin::ImportedSrt => TranscriptFormat::Srt,
                    StoredOrigin::ImportedWebvtt => TranscriptFormat::WebVtt,
                },
                sidecar: SidecarIdentity::new(self.sidecar.sha256, self.sidecar.bytes).ok()?,
                offset: TranscriptOffset::from_micros(self.offset_us).ok()?,
            },
            supersedes: None,
            replaced_range: None,
            language: optional_language(self.language).ok()?,
            segments,
            warnings,
        })
        .ok()
    }
}

impl StoredSourceSegment {
    fn from_domain(segment: &SourceSegment) -> Self {
        Self {
            id: segment.id().as_str().to_owned(),
            index: segment.index(),
            start_us: segment.range().start().as_micros(),
            end_us: segment.range().end().as_micros(),
            state: match segment.state() {
                SourceSegmentState::Closed => StoredSegmentState::Closed,
            },
        }
    }

    fn into_domain(self) -> Option<SourceSegment> {
        let StoredSegmentState::Closed = self.state;
        if self.index != 0 || self.start_us != 0 {
            return None;
        }
        let id = SourceSegmentId::parse(self.id).ok()?;
        SourceSegment::whole_file(id, MediaTime::from_micros(self.end_us)).ok()
    }
}

impl StoredSegment {
    fn into_domain(self) -> Option<TranscriptSegment> {
        let text = stored_text(self.text, self.original_text)?;
        let speaker = match self.speaker {
            Some(label) => Some(SpeakerLabel::parse(label).ok()?),
            None => None,
        };
        Some(TranscriptSegment::new(TranscriptSegmentParts {
            id: TranscriptSegmentId::parse(self.id).ok()?,
            ordinal: NonZeroU32::new(self.ordinal)?,
            range: stored_range(self.start_us, self.end_us)?,
            text,
            speaker,
            confidence: Confidence::unknown(),
            origin: SegmentOrigin::ImportedCue {
                cue: CueSource::new(
                    NonZeroU32::new(self.cue_ordinal)?,
                    NonZeroU32::new(self.cue_line)?,
                ),
                timing: CueTiming::new(self.cue_start_us, self.cue_end_us).ok()?,
            },
        }))
    }
}

/// Version 2: a revision recognised locally from the source's own audio.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredAsrTranscript {
    schema_version: u16,
    format: String,
    revision_id: String,
    revision: u32,
    source_id: String,
    source_segment: StoredSourceSegment,
    origin: StoredAsrOrigin,
    run: StoredAsrRun,
    supersedes: Option<String>,
    replaced_range: Option<StoredRange>,
    language: Option<String>,
    warnings: Vec<StoredWarning>,
    segments: Vec<StoredAsrSegment>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredAsrOrigin {
    LocalAsr,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredRange {
    start_us: u64,
    end_us: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredAsrRun {
    provider: StoredAsrProvider,
    executable_sha256: String,
    model_profile: StoredModelProfile,
    model_sha256: String,
    decoding_profile: StoredDecodingProfile,
    window_us: u64,
    overlap_us: u64,
    threads: u16,
    audio_stream: u32,
    chunks: Vec<StoredChunk>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredAsrProvider {
    WhisperCpp,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredModelProfile {
    Base,
    Unreviewed,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum StoredDecodingProfile {
    #[serde(rename = "r0-v1")]
    R0V1,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredChunk {
    index: u32,
    start_us: u64,
    end_us: u64,
    outcome: StoredChunkOutcome,
    audio: Option<StoredRange>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredChunkOutcome {
    Transcribed,
    Silent,
    NoAudio,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredAsrSegment {
    id: String,
    ordinal: u32,
    start_us: u64,
    end_us: u64,
    text: String,
    confidence_basis_points: Option<u16>,
    chunk: u32,
    provider_start_us: u64,
    provider_end_us: u64,
    end_trim: StoredEndTrim,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredEndTrim {
    Unchanged,
    TrimmedToAudioEnd,
}

impl StoredAsrTranscript {
    fn from_revision(revision: &TranscriptRevision, run: &AsrRun) -> Self {
        Self {
            schema_version: LOCAL_ASR_RECORD_VERSION,
            format: RECORD_FORMAT.to_owned(),
            revision_id: revision.id().as_str().to_owned(),
            revision: revision.number(),
            source_id: revision.source_id().as_str().to_owned(),
            source_segment: StoredSourceSegment::from_domain(revision.source_segment()),
            origin: StoredAsrOrigin::LocalAsr,
            run: StoredAsrRun::from_domain(run),
            supersedes: revision.supersedes().map(|id| id.as_str().to_owned()),
            replaced_range: revision.replaced_range().map(StoredRange::from_domain),
            language: revision.language().map(|tag| tag.as_str().to_owned()),
            warnings: stored_warnings(revision.warnings()),
            segments: revision
                .segments()
                .iter()
                .filter_map(|segment| {
                    let SegmentOrigin::Asr {
                        chunk,
                        provider_start,
                        provider_end,
                        trimmed,
                    } = segment.origin()
                    else {
                        // The domain guarantees a local-ASR revision holds only ASR segments.
                        return None;
                    };
                    Some(StoredAsrSegment {
                        id: segment.id().as_str().to_owned(),
                        ordinal: segment.ordinal(),
                        start_us: segment.range().start().as_micros(),
                        end_us: segment.range().end().as_micros(),
                        text: segment.text().text().to_owned(),
                        confidence_basis_points: segment.confidence().basis_points(),
                        chunk,
                        provider_start_us: provider_start.as_micros(),
                        provider_end_us: provider_end.as_micros(),
                        end_trim: match trimmed {
                            ProviderEndTrim::Unchanged => StoredEndTrim::Unchanged,
                            ProviderEndTrim::TrimmedToAudioEnd => StoredEndTrim::TrimmedToAudioEnd,
                        },
                    })
                })
                .collect(),
        }
    }

    fn into_revision(self) -> Option<TranscriptRevision> {
        if self.schema_version != LOCAL_ASR_RECORD_VERSION || self.format != RECORD_FORMAT {
            return None;
        }
        let StoredAsrOrigin::LocalAsr = self.origin;
        let source_segment = self.source_segment.into_domain()?;
        let run = self.run.into_domain(source_segment.id())?;
        let mut segments = Vec::with_capacity(self.segments.len());
        for segment in self.segments {
            segments.push(segment.into_domain()?);
        }
        let supersedes = match self.supersedes {
            Some(id) => Some(TranscriptRevisionId::parse(id).ok()?),
            None => None,
        };
        let replaced_range = match self.replaced_range {
            Some(range) => Some(stored_range(range.start_us, range.end_us)?),
            None => None,
        };
        TranscriptRevision::new(TranscriptRevisionParts {
            id: TranscriptRevisionId::parse(self.revision_id).ok()?,
            number: NonZeroU32::new(self.revision)?,
            source_id: SourceId::parse(self.source_id).ok()?,
            source_segment,
            provenance: TranscriptProvenance::LocalAsr(run),
            supersedes,
            replaced_range,
            language: optional_language(self.language).ok()?,
            segments,
            warnings: domain_warnings(self.warnings)?,
        })
        .ok()
    }
}

impl StoredRange {
    const fn from_domain(range: TimeRange) -> Self {
        Self {
            start_us: range.start().as_micros(),
            end_us: range.end().as_micros(),
        }
    }
}

impl StoredAsrRun {
    fn from_domain(run: &AsrRun) -> Self {
        Self {
            provider: match run.provider().provider() {
                AsrProvider::WhisperCpp => StoredAsrProvider::WhisperCpp,
            },
            executable_sha256: run.provider().executable_sha256().as_str().to_owned(),
            model_profile: match run.model().profile() {
                AsrModelProfile::Base => StoredModelProfile::Base,
                AsrModelProfile::Unreviewed => StoredModelProfile::Unreviewed,
            },
            model_sha256: run.model().sha256().as_str().to_owned(),
            decoding_profile: match run.decoding() {
                AsrDecodingProfile::R0V1 => StoredDecodingProfile::R0V1,
            },
            window_us: run.plan().window_us(),
            overlap_us: run.plan().overlap_us(),
            threads: run.threads().get(),
            audio_stream: run.audio_stream(),
            chunks: run
                .chunks()
                .iter()
                .map(|record| {
                    let (outcome, audio) = match record.outcome() {
                        AsrChunkOutcome::Transcribed { audio } => {
                            (StoredChunkOutcome::Transcribed, Some(audio))
                        }
                        AsrChunkOutcome::Silent { audio } => {
                            (StoredChunkOutcome::Silent, Some(audio))
                        }
                        AsrChunkOutcome::NoAudio => (StoredChunkOutcome::NoAudio, None),
                    };
                    StoredChunk {
                        index: record.chunk().index(),
                        start_us: record.chunk().window().start().as_micros(),
                        end_us: record.chunk().window().end().as_micros(),
                        outcome,
                        audio: audio.map(StoredRange::from_domain),
                    }
                })
                .collect(),
        }
    }

    fn into_domain(self, source_segment: &SourceSegmentId) -> Option<AsrRun> {
        let mut chunks = Vec::with_capacity(self.chunks.len());
        for chunk in self.chunks {
            let audio = match chunk.audio {
                Some(range) => Some(stored_range(range.start_us, range.end_us)?),
                None => None,
            };
            let outcome = match (chunk.outcome, audio) {
                (StoredChunkOutcome::Transcribed, Some(audio)) => {
                    AsrChunkOutcome::Transcribed { audio }
                }
                (StoredChunkOutcome::Silent, Some(audio)) => AsrChunkOutcome::Silent { audio },
                (StoredChunkOutcome::NoAudio, None) => AsrChunkOutcome::NoAudio,
                _ => return None,
            };
            chunks.push(AsrChunkRecord::new(
                PlannedChunk::new(
                    source_segment.clone(),
                    chunk.index,
                    stored_range(chunk.start_us, chunk.end_us)?,
                ),
                outcome,
            ));
        }
        AsrRun::new(AsrRunParts {
            provider: AsrProviderBuild::new(
                match self.provider {
                    StoredAsrProvider::WhisperCpp => AsrProvider::WhisperCpp,
                },
                Sha256Hex::parse(self.executable_sha256).ok()?,
            ),
            model: AsrModel::new(
                match self.model_profile {
                    StoredModelProfile::Base => AsrModelProfile::Base,
                    StoredModelProfile::Unreviewed => AsrModelProfile::Unreviewed,
                },
                Sha256Hex::parse(self.model_sha256).ok()?,
            ),
            decoding: match self.decoding_profile {
                StoredDecodingProfile::R0V1 => AsrDecodingProfile::R0V1,
            },
            plan: ChunkPlan::new(self.window_us, self.overlap_us).ok()?,
            threads: NonZeroU16::new(self.threads)?,
            audio_stream: self.audio_stream,
            chunks,
        })
        .ok()
    }
}

impl StoredAsrSegment {
    fn into_domain(self) -> Option<TranscriptSegment> {
        let confidence = match self.confidence_basis_points {
            Some(points) => {
                Confidence::provider_score(points, ConfidenceOrigin::ProviderUncalibrated).ok()?
            }
            None => Confidence::unknown(),
        };
        Some(TranscriptSegment::new(TranscriptSegmentParts {
            id: TranscriptSegmentId::parse(self.id).ok()?,
            ordinal: NonZeroU32::new(self.ordinal)?,
            range: stored_range(self.start_us, self.end_us)?,
            text: CueText::new(self.text.clone(), self.text).ok()?,
            speaker: None,
            confidence,
            origin: SegmentOrigin::Asr {
                chunk: self.chunk,
                provider_start: ChunkTime::from_micros(self.provider_start_us),
                provider_end: ChunkTime::from_micros(self.provider_end_us),
                trimmed: match self.end_trim {
                    StoredEndTrim::Unchanged => ProviderEndTrim::Unchanged,
                    StoredEndTrim::TrimmedToAudioEnd => ProviderEndTrim::TrimmedToAudioEnd,
                },
            },
        }))
    }
}

fn stored_text(text: String, original: Option<String>) -> Option<CueText> {
    match original {
        Some(original) if original != text => CueText::new(text, original).ok(),
        Some(_) => None,
        None => CueText::new(text.clone(), text).ok(),
    }
}

fn stored_range(start_us: u64, end_us: u64) -> Option<TimeRange> {
    TimeRange::new(
        MediaTime::from_micros(start_us),
        MediaTime::from_micros(end_us),
    )
    .ok()
}

fn optional_language(language: Option<String>) -> Result<Option<LanguageTag>, LanguageTagError> {
    language.map(LanguageTag::parse).transpose()
}

fn stored_warnings(warnings: &TranscriptWarnings) -> Vec<StoredWarning> {
    warnings
        .as_slice()
        .iter()
        .map(|warning| StoredWarning {
            kind: StoredWarningKind::from_domain(warning.kind()),
            count: warning.count(),
            first_cue: warning.first_cue(),
        })
        .collect()
}

fn domain_warnings(warnings: Vec<StoredWarning>) -> Option<TranscriptWarnings> {
    TranscriptWarnings::from_stored(
        warnings
            .into_iter()
            .map(|warning| {
                TranscriptWarning::new(warning.kind.into_domain(), warning.count, warning.first_cue)
            })
            .collect::<Result<Vec<_>, _>>()
            .ok()?,
    )
    .ok()
}

impl StoredWarningKind {
    const fn from_domain(kind: TranscriptWarningKind) -> Self {
        match kind {
            TranscriptWarningKind::EmptyCuesSkipped => Self::EmptyCuesSkipped,
            TranscriptWarningKind::MarkupRemoved => Self::MarkupRemoved,
            TranscriptWarningKind::SpeakerLabelDiscarded => Self::SpeakerLabelDiscarded,
            TranscriptWarningKind::OverlappingCues => Self::OverlappingCues,
            TranscriptWarningKind::CuesOutsideSource => Self::CuesOutsideSource,
            TranscriptWarningKind::CuesCrossingSourceBoundary => Self::CuesCrossingSourceBoundary,
            TranscriptWarningKind::ProviderSegmentsRejected => Self::ProviderSegmentsRejected,
            TranscriptWarningKind::ProviderEndTrimmed => Self::ProviderEndTrimmed,
            TranscriptWarningKind::NonSpeechMarkersRemoved => Self::NonSpeechMarkersRemoved,
            TranscriptWarningKind::SeamDuplicatesRemoved => Self::SeamDuplicatesRemoved,
            TranscriptWarningKind::SilentChunksSkipped => Self::SilentChunksSkipped,
        }
    }

    const fn into_domain(self) -> TranscriptWarningKind {
        match self {
            Self::EmptyCuesSkipped => TranscriptWarningKind::EmptyCuesSkipped,
            Self::MarkupRemoved => TranscriptWarningKind::MarkupRemoved,
            Self::SpeakerLabelDiscarded => TranscriptWarningKind::SpeakerLabelDiscarded,
            Self::OverlappingCues => TranscriptWarningKind::OverlappingCues,
            Self::CuesOutsideSource => TranscriptWarningKind::CuesOutsideSource,
            Self::CuesCrossingSourceBoundary => TranscriptWarningKind::CuesCrossingSourceBoundary,
            Self::ProviderSegmentsRejected => TranscriptWarningKind::ProviderSegmentsRejected,
            Self::ProviderEndTrimmed => TranscriptWarningKind::ProviderEndTrimmed,
            Self::NonSpeechMarkersRemoved => TranscriptWarningKind::NonSpeechMarkersRemoved,
            Self::SeamDuplicatesRemoved => TranscriptWarningKind::SeamDuplicatesRemoved,
            Self::SilentChunksSkipped => TranscriptWarningKind::SilentChunksSkipped,
        }
    }
}
