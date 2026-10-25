//! Versioned storage record of one on-screen text index revision (OCR, ADR 0030).
//!
//! Each committed revision of a session's screen-text index is one immutable,
//! content-addressed session artifact of kind `screen_text_index_record`, and
//! travels unchanged into retained bundles. A revision holds every frame of the
//! revision before it, so only the newest record is read.
//!
//! Decoding is strict, like the other records: unknown fields, versions and
//! values are rejected, every value is rebuilt through the domain constructors
//! (text, regions inside their frames, frame order, bounds), and every identity
//! is derived again from the session, source, stream, profile, engine build and
//! frame, so a record whose identities were edited is refused. Its bytes are
//! checked against the manifest's size and digest before any is read, which is
//! what protects the text itself. The record never holds pixels, only what was
//! read and where.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};
use vsift_application::{SessionStorageError, verify_screen_text_identities};
use vsift_domain::{
    Confidence, ConfidenceOrigin, FrameDimensions, FrameRef, MediaTime, ProviderLabel, ReadTarget,
    ScreenText, ScreenTextFailure, ScreenTextFrame, ScreenTextIndex, ScreenTextIndexId,
    ScreenTextIndexParts, ScreenTextLine, ScreenTextLineId, ScreenTextOutcome, ScreenTextProfile,
    ScreenTextProvider, ScreenTextScope, SessionId, Sha256Hex, SourceId, TextRegion, TimeBase,
    VisualCandidateId,
};

/// Largest encoded screen-text record a session will store or read.
///
/// The index holds at most 200,000 lines (`MAX_SCREEN_TEXT_LINES`); a line of
/// ordinary interface text encodes in about 150 bytes, so 48 MiB holds a full
/// index of long lines without allowing an unbounded record.
pub const MAX_SCREEN_TEXT_INDEX_RECORD_BYTES: usize = 48 * 1024 * 1024;
/// Most screen-text records one session holds: each `ocr` call that reads
/// anything commits one, and a call reads up to 200 frames.
pub const MAX_SCREEN_TEXT_INDEX_RECORDS: usize = 64;
const RECORD_VERSION: u16 = 1;
const RECORD_FORMAT: &str = "vsift.screen_text_index_record";

/// Encodes an index revision as its bounded storage record.
///
/// # Errors
///
/// [`SessionStorageError::CapacityExhausted`] when the record would exceed
/// [`MAX_SCREEN_TEXT_INDEX_RECORD_BYTES`].
pub fn encode_screen_text_index_record(
    index: &ScreenTextIndex,
) -> Result<Vec<u8>, SessionStorageError> {
    let bytes =
        serde_json::to_vec(&StoredIndex::from_index(index)).map_err(|_| SessionStorageError::Io)?;
    if bytes.len() > MAX_SCREEN_TEXT_INDEX_RECORD_BYTES {
        return Err(SessionStorageError::CapacityExhausted);
    }
    Ok(bytes)
}

/// Decodes and revalidates a stored index revision of `session_id`.
///
/// # Errors
///
/// [`SessionStorageError::UnsupportedVersion`] for a newer record and
/// [`SessionStorageError::IntegrityFailure`] for anything malformed, oversized,
/// of another session, or violating a domain or identity rule.
pub fn decode_screen_text_index_record(
    bytes: &[u8],
    session_id: &SessionId,
) -> Result<ScreenTextIndex, SessionStorageError> {
    #[derive(Deserialize)]
    struct VersionProbe {
        schema_version: u16,
    }
    if bytes.len() > MAX_SCREEN_TEXT_INDEX_RECORD_BYTES {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let probe: VersionProbe =
        serde_json::from_slice(bytes).map_err(|_| SessionStorageError::IntegrityFailure)?;
    match probe.schema_version {
        RECORD_VERSION => {}
        newer if newer > RECORD_VERSION => return Err(SessionStorageError::UnsupportedVersion),
        _ => return Err(SessionStorageError::IntegrityFailure),
    }
    let stored: StoredIndex =
        serde_json::from_slice(bytes).map_err(|_| SessionStorageError::IntegrityFailure)?;
    stored
        .into_index(session_id)
        .ok_or(SessionStorageError::IntegrityFailure)
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredIndex {
    schema_version: u16,
    format: String,
    index_id: String,
    revision: u32,
    session_id: String,
    source_id: String,
    stream_index: u32,
    profile: String,
    engine: StoredEngine,
    period_us: u64,
    frames: Vec<StoredFrame>,
    unresolved: Vec<StoredTarget>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredEngine {
    runner_sha256: String,
    runner_version: String,
    runtime_version: String,
    detector_sha256: String,
    recognizer_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredFrame {
    pts: i64,
    time_base: [u32; 2],
    time_us: u64,
    width: u32,
    height: u32,
    targets: Vec<StoredTarget>,
    outcome: StoredOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lines: Option<Vec<StoredLine>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredOutcome {
    Read,
    ImageRejected,
    EngineFailed,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredTarget {
    time_us: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    candidate_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredLine {
    id: String,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    text: String,
    confidence_bp: u16,
}

impl StoredIndex {
    fn from_index(index: &ScreenTextIndex) -> Self {
        let scope = index.scope();
        Self {
            schema_version: RECORD_VERSION,
            format: RECORD_FORMAT.to_owned(),
            index_id: index.id().as_str().to_owned(),
            revision: index.revision().get(),
            session_id: scope.session_id.as_str().to_owned(),
            source_id: scope.source_id.as_str().to_owned(),
            stream_index: scope.stream_index,
            profile: scope.profile.identifier().to_owned(),
            engine: StoredEngine {
                runner_sha256: scope.provider.runner_sha256.as_str().to_owned(),
                runner_version: scope.provider.runner_version.as_str().to_owned(),
                runtime_version: scope.provider.runtime_version.as_str().to_owned(),
                detector_sha256: scope.provider.detector_sha256.as_str().to_owned(),
                recognizer_sha256: scope.provider.recognizer_sha256.as_str().to_owned(),
            },
            period_us: index.period_micros(),
            frames: index.frames().iter().map(StoredFrame::from_frame).collect(),
            unresolved: index
                .unresolved()
                .iter()
                .map(StoredTarget::from_target)
                .collect(),
        }
    }

    fn into_index(self, session_id: &SessionId) -> Option<ScreenTextIndex> {
        if self.format != RECORD_FORMAT || self.session_id != session_id.as_str() {
            return None;
        }
        let scope = ScreenTextScope {
            session_id: session_id.clone(),
            source_id: SourceId::parse(self.source_id).ok()?,
            stream_index: self.stream_index,
            profile: ScreenTextProfile::from_identifier(&self.profile)?,
            provider: ScreenTextProvider {
                runner_sha256: Sha256Hex::parse(self.engine.runner_sha256).ok()?,
                runner_version: ProviderLabel::parse(self.engine.runner_version).ok()?,
                runtime_version: ProviderLabel::parse(self.engine.runtime_version).ok()?,
                detector_sha256: Sha256Hex::parse(self.engine.detector_sha256).ok()?,
                recognizer_sha256: Sha256Hex::parse(self.engine.recognizer_sha256).ok()?,
            },
        };
        let frames = self
            .frames
            .into_iter()
            .map(|frame| frame.into_frame(self.stream_index))
            .collect::<Option<Vec<_>>>()?;
        let unresolved = self
            .unresolved
            .into_iter()
            .map(StoredTarget::into_target)
            .collect::<Option<Vec<_>>>()?;
        let index = ScreenTextIndex::new(ScreenTextIndexParts {
            id: ScreenTextIndexId::parse(self.index_id).ok()?,
            revision: NonZeroU32::new(self.revision)?,
            scope,
            period_micros: self.period_us,
            frames,
            unresolved,
        })
        .ok()?;
        verify_screen_text_identities(&index).ok()?;
        Some(index)
    }
}

impl StoredFrame {
    fn from_frame(frame: &ScreenTextFrame) -> Self {
        let (outcome, truncated, lines) = match &frame.outcome {
            ScreenTextOutcome::Read { lines, truncated } => (
                StoredOutcome::Read,
                Some(*truncated),
                Some(lines.iter().map(StoredLine::from_line).collect()),
            ),
            ScreenTextOutcome::Failed(ScreenTextFailure::ImageRejected) => {
                (StoredOutcome::ImageRejected, None, None)
            }
            ScreenTextOutcome::Failed(ScreenTextFailure::EngineFailed) => {
                (StoredOutcome::EngineFailed, None, None)
            }
        };
        Self {
            pts: frame.frame.pts,
            time_base: [
                frame.frame.time_base.numerator(),
                frame.frame.time_base.denominator(),
            ],
            time_us: frame.frame.time.as_micros(),
            width: frame.frame.dimensions.width(),
            height: frame.frame.dimensions.height(),
            targets: frame
                .targets
                .iter()
                .map(StoredTarget::from_target)
                .collect(),
            outcome,
            truncated,
            lines,
        }
    }

    fn into_frame(self, stream_index: u32) -> Option<ScreenTextFrame> {
        let dimensions = FrameDimensions::new(self.width, self.height).ok()?;
        let [numerator, denominator] = self.time_base;
        let frame = FrameRef {
            stream_index,
            time_base: TimeBase::new(numerator, denominator).ok()?,
            pts: self.pts,
            time: MediaTime::from_micros(self.time_us),
            dimensions,
        };
        let outcome = match (self.outcome, self.truncated, self.lines) {
            (StoredOutcome::Read, Some(truncated), Some(lines)) => ScreenTextOutcome::Read {
                lines: lines
                    .into_iter()
                    .map(|line| line.into_line(dimensions))
                    .collect::<Option<Vec<_>>>()?,
                truncated,
            },
            (StoredOutcome::ImageRejected, None, None) => {
                ScreenTextOutcome::Failed(ScreenTextFailure::ImageRejected)
            }
            (StoredOutcome::EngineFailed, None, None) => {
                ScreenTextOutcome::Failed(ScreenTextFailure::EngineFailed)
            }
            _ => return None,
        };
        let targets = self
            .targets
            .into_iter()
            .map(StoredTarget::into_target)
            .collect::<Option<Vec<_>>>()?;
        Some(ScreenTextFrame {
            frame,
            targets,
            outcome,
        })
    }
}

impl StoredTarget {
    fn from_target(target: &ReadTarget) -> Self {
        Self {
            time_us: target.time.as_micros(),
            candidate_id: target.candidate.as_ref().map(|id| id.as_str().to_owned()),
        }
    }

    fn into_target(self) -> Option<ReadTarget> {
        let candidate = match self.candidate_id {
            Some(id) => Some(VisualCandidateId::parse(id).ok()?),
            None => None,
        };
        Some(ReadTarget {
            time: MediaTime::from_micros(self.time_us),
            candidate,
        })
    }
}

impl StoredLine {
    fn from_line(line: &ScreenTextLine) -> Self {
        Self {
            id: line.id.as_str().to_owned(),
            x: line.region.x(),
            y: line.region.y(),
            width: line.region.width(),
            height: line.region.height(),
            text: line.text.as_str().to_owned(),
            confidence_bp: line.confidence.basis_points().unwrap_or(0),
        }
    }

    fn into_line(self, dimensions: FrameDimensions) -> Option<ScreenTextLine> {
        Some(ScreenTextLine {
            id: ScreenTextLineId::parse(self.id).ok()?,
            region: TextRegion::new(self.x, self.y, self.width, self.height, dimensions).ok()?,
            text: ScreenText::parse(self.text).ok()?,
            confidence: Confidence::provider_score(
                self.confidence_bp,
                ConfidenceOrigin::ProviderUncalibrated,
            )
            .ok()?,
        })
    }
}

#[cfg(test)]
mod tests;
