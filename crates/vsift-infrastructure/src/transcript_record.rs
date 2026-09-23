//! Versioned storage record of one transcript revision.
//!
//! A committed revision is stored as one immutable, content-addressed session
//! artifact of kind `transcript_record` and travels unchanged into retained
//! bundles. Decoding is strict (unknown fields, versions and values are
//! rejected) and rebuilds the revision through the domain constructors, so a
//! modified record cannot bypass the import rules. Imported revisions carry no
//! stored confidence: the domain fixes it to unknown.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};
use vsift_application::SessionStorageError;
use vsift_domain::{
    AlignmentOrigin, Confidence, CueSource, CueText, CueTiming, LanguageTag, MediaTime,
    SidecarIdentity, SourceId, SourceSegment, SourceSegmentId, SourceSegmentState, SpeakerLabel,
    TimeRange, TranscriptOffset, TranscriptRevision, TranscriptRevisionId, TranscriptRevisionParts,
    TranscriptSegment, TranscriptSegmentId, TranscriptSegmentParts, TranscriptWarning,
    TranscriptWarningKind, TranscriptWarnings,
};

/// Largest encoded transcript record a session will store or read.
///
/// Three times the supplied-file limit leaves room for the original text kept
/// beside marked-up cues; an import whose record would be larger is rejected
/// with a resource limit rather than stored partially.
pub const MAX_TRANSCRIPT_RECORD_BYTES: usize = 24 * 1024 * 1024;
const RECORD_VERSION: u16 = 1;
const RECORD_FORMAT: &str = "vsift.transcript_record";

/// Encodes a revision as its bounded storage record.
///
/// # Errors
///
/// Returns [`SessionStorageError::CapacityExhausted`] when the record would
/// exceed [`MAX_TRANSCRIPT_RECORD_BYTES`].
pub fn encode_transcript_record(
    revision: &TranscriptRevision,
) -> Result<Vec<u8>, SessionStorageError> {
    let record = StoredTranscript::from_revision(revision);
    let bytes = serde_json::to_vec(&record).map_err(|_| SessionStorageError::Io)?;
    if bytes.len() > MAX_TRANSCRIPT_RECORD_BYTES {
        return Err(SessionStorageError::CapacityExhausted);
    }
    Ok(bytes)
}

/// Decodes and revalidates a stored revision.
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
    if probe.schema_version > RECORD_VERSION {
        return Err(SessionStorageError::UnsupportedVersion);
    }
    let record: StoredTranscript =
        serde_json::from_slice(bytes).map_err(|_| SessionStorageError::IntegrityFailure)?;
    record
        .into_revision()
        .ok_or(SessionStorageError::IntegrityFailure)
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
    fn from_revision(revision: &TranscriptRevision) -> Self {
        let segment = revision.source_segment();
        Self {
            schema_version: RECORD_VERSION,
            format: RECORD_FORMAT.to_owned(),
            revision_id: revision.id().as_str().to_owned(),
            revision: revision.number(),
            source_id: revision.source_id().as_str().to_owned(),
            source_segment: StoredSourceSegment {
                id: segment.id().as_str().to_owned(),
                index: segment.index(),
                start_us: segment.range().start().as_micros(),
                end_us: segment.range().end().as_micros(),
                state: match segment.state() {
                    SourceSegmentState::Closed => StoredSegmentState::Closed,
                },
            },
            origin: match revision.origin() {
                AlignmentOrigin::ImportedSrt => StoredOrigin::ImportedSrt,
                AlignmentOrigin::ImportedWebVtt => StoredOrigin::ImportedWebvtt,
            },
            offset_us: revision.offset().as_micros(),
            sidecar: StoredSidecar {
                sha256: revision.sidecar().sha256().to_owned(),
                bytes: revision.sidecar().bytes(),
            },
            language: revision.language().map(|tag| tag.as_str().to_owned()),
            warnings: revision
                .warnings()
                .as_slice()
                .iter()
                .map(|warning| StoredWarning {
                    kind: StoredWarningKind::from_domain(warning.kind()),
                    count: warning.count(),
                    first_cue: warning.first_cue(),
                })
                .collect(),
            segments: revision
                .segments()
                .iter()
                .map(|segment| StoredSegment {
                    id: segment.id().as_str().to_owned(),
                    ordinal: segment.ordinal(),
                    start_us: segment.range().start().as_micros(),
                    end_us: segment.range().end().as_micros(),
                    text: segment.text().text().to_owned(),
                    original_text: segment.text().original().map(str::to_owned),
                    speaker: segment.speaker().map(|label| label.as_str().to_owned()),
                    cue_ordinal: segment.cue().ordinal(),
                    cue_line: segment.cue().line(),
                    cue_start_us: segment.cue_timing().start_micros(),
                    cue_end_us: segment.cue_timing().end_micros(),
                })
                .collect(),
        }
    }

    fn into_revision(self) -> Option<TranscriptRevision> {
        if self.schema_version != RECORD_VERSION || self.format != RECORD_FORMAT {
            return None;
        }
        let source_segment = self.source_segment.into_domain()?;
        let revision_id = TranscriptRevisionId::parse(self.revision_id).ok()?;
        let mut segments = Vec::with_capacity(self.segments.len());
        for segment in self.segments {
            segments.push(segment.into_domain()?);
        }
        let warnings = TranscriptWarnings::from_stored(
            self.warnings
                .into_iter()
                .map(|warning| {
                    TranscriptWarning::new(
                        warning.kind.into_domain(),
                        warning.count,
                        warning.first_cue,
                    )
                })
                .collect::<Result<Vec<_>, _>>()
                .ok()?,
        )
        .ok()?;
        let language = match self.language {
            Some(tag) => Some(LanguageTag::parse(tag).ok()?),
            None => None,
        };
        TranscriptRevision::new(TranscriptRevisionParts {
            id: revision_id,
            number: NonZeroU32::new(self.revision)?,
            source_id: SourceId::parse(self.source_id).ok()?,
            source_segment,
            origin: match self.origin {
                StoredOrigin::ImportedSrt => AlignmentOrigin::ImportedSrt,
                StoredOrigin::ImportedWebvtt => AlignmentOrigin::ImportedWebVtt,
            },
            offset: TranscriptOffset::from_micros(self.offset_us).ok()?,
            sidecar: SidecarIdentity::new(self.sidecar.sha256, self.sidecar.bytes).ok()?,
            language,
            segments,
            warnings,
        })
        .ok()
    }
}

impl StoredSourceSegment {
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
        let text = match self.original_text {
            Some(original) if original != self.text => CueText::new(self.text, original).ok()?,
            Some(_) => return None,
            None => CueText::new(self.text.clone(), self.text).ok()?,
        };
        let speaker = match self.speaker {
            Some(label) => Some(SpeakerLabel::parse(label).ok()?),
            None => None,
        };
        Some(TranscriptSegment::new(TranscriptSegmentParts {
            id: TranscriptSegmentId::parse(self.id).ok()?,
            ordinal: NonZeroU32::new(self.ordinal)?,
            range: TimeRange::new(
                MediaTime::from_micros(self.start_us),
                MediaTime::from_micros(self.end_us),
            )
            .ok()?,
            text,
            speaker,
            confidence: Confidence::unknown(),
            cue: CueSource::new(
                NonZeroU32::new(self.cue_ordinal)?,
                NonZeroU32::new(self.cue_line)?,
            ),
            cue_timing: CueTiming::new(self.cue_start_us, self.cue_end_us).ok()?,
        }))
    }
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
        }
    }
}
