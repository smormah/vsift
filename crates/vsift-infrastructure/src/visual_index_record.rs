//! Versioned storage record of one visual-candidate index revision (P08).
//!
//! Each committed revision of a session's visual index is one immutable,
//! content-addressed session artifact of kind `visual_index_record`, and
//! travels unchanged into retained bundles. A revision holds every window of
//! the revision before it, so only the newest record is ever read.
//!
//! Decoding is strict, like the transcript record: unknown fields, versions
//! and values are rejected, and the index is rebuilt through the domain
//! constructors and the application's identity derivation. That re-derives
//! everything a record could claim without being true:
//!
//! - every window's range from its ordinal and the source duration (the
//!   fixed grid);
//! - every candidate's span from its representative time and the next
//!   candidate or the window end, and its sample counts;
//! - that every recorded change satisfies the change policy, and every 10 s
//!   cell holds a candidate or is recorded as frameless (the coverage rule);
//! - every candidate and revision identity from the session, source, stream,
//!   profile, window and time.
//!
//! The record never holds pixels or sample data, only the candidates.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};
use vsift_application::{SessionStorageError, VisualIndexScope, verify_visual_index_identities};
use vsift_domain::{
    CandidateChange, CandidateDraft, CandidateReason, CandidateStability, MediaTime, SessionId,
    SourceId, TimeRange, VisualCandidate, VisualCandidateId, VisualDelta, VisualHash, VisualIndex,
    VisualIndexId, VisualIndexParts, VisualIndexProfile, VisualIndexWindow, VisualWindow,
    VisualWindowOutcome,
};

/// Largest encoded visual-index record a session will store or read.
///
/// Four hours of source are 240 windows of at most 32 candidates; at about
/// 330 bytes a candidate that is 2.5 MB, so 8 MiB leaves room without
/// allowing an unbounded record.
pub const MAX_VISUAL_INDEX_RECORD_BYTES: usize = 8 * 1024 * 1024;
/// Most visual-index records one session holds. Each call that analyses
/// anything commits one; 64 calls of 30 windows cover eight times the
/// four-hour source bound, so the limit only stops a runaway caller.
pub const MAX_VISUAL_INDEX_RECORDS: usize = 64;
const RECORD_VERSION: u16 = 1;
const RECORD_FORMAT: &str = "vsift.visual_index_record";

/// Encodes an index revision of `session_id` as its bounded storage record.
///
/// # Errors
///
/// Returns [`SessionStorageError::CapacityExhausted`] when the record would
/// exceed [`MAX_VISUAL_INDEX_RECORD_BYTES`].
pub fn encode_visual_index_record(
    session_id: &SessionId,
    index: &VisualIndex,
) -> Result<Vec<u8>, SessionStorageError> {
    let bytes = serde_json::to_vec(&StoredVisualIndex::from_index(session_id, index))
        .map_err(|_| SessionStorageError::Io)?;
    if bytes.len() > MAX_VISUAL_INDEX_RECORD_BYTES {
        return Err(SessionStorageError::CapacityExhausted);
    }
    Ok(bytes)
}

/// Decodes and revalidates a stored index revision of `session_id`.
///
/// # Errors
///
/// Returns [`SessionStorageError::UnsupportedVersion`] for a newer record and
/// [`SessionStorageError::IntegrityFailure`] for anything malformed,
/// oversized, of another session, or violating an analysis or identity rule.
pub fn decode_visual_index_record(
    bytes: &[u8],
    session_id: &SessionId,
) -> Result<VisualIndex, SessionStorageError> {
    #[derive(Deserialize)]
    struct VersionProbe {
        schema_version: u16,
    }
    if bytes.len() > MAX_VISUAL_INDEX_RECORD_BYTES {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let probe: VersionProbe =
        serde_json::from_slice(bytes).map_err(|_| SessionStorageError::IntegrityFailure)?;
    match probe.schema_version {
        RECORD_VERSION => {}
        newer if newer > RECORD_VERSION => return Err(SessionStorageError::UnsupportedVersion),
        _ => return Err(SessionStorageError::IntegrityFailure),
    }
    let stored: StoredVisualIndex =
        serde_json::from_slice(bytes).map_err(|_| SessionStorageError::IntegrityFailure)?;
    stored
        .into_index(session_id)
        .ok_or(SessionStorageError::IntegrityFailure)
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredVisualIndex {
    schema_version: u16,
    format: String,
    index_id: String,
    revision: u32,
    session_id: String,
    source_id: String,
    stream_index: u32,
    duration_us: u64,
    profile: StoredProfile,
    windows: Vec<StoredWindow>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum StoredProfile {
    #[serde(rename = "r0-visual-v1")]
    R0,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredWindowState {
    Analysed,
    Undecodable,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredWindow {
    ordinal: u32,
    start_us: u64,
    end_us: u64,
    state: StoredWindowState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sample_count: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dropped_candidates: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    frameless_cells: Option<Vec<StoredRange>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    candidates: Option<Vec<StoredCandidate>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredRange {
    start_us: u64,
    end_us: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredCandidate {
    id: String,
    reason: StoredReason,
    representative_us: u64,
    span_end_us: u64,
    sample_count: u16,
    stability: StoredStability,
    visual_hash: String,
    change: Option<StoredChange>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredReason {
    FirstFrame,
    VisualChange,
    MotionStart,
    SettledAfterMotion,
    PeriodicCoverage,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredStability {
    Settled,
    Transient,
    InMotion,
    OpenAtWindowEnd,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredChange {
    previous_sample_us: u64,
    changed_blocks: u8,
    max_block_delta: u8,
}

impl StoredVisualIndex {
    fn from_index(session_id: &SessionId, index: &VisualIndex) -> Self {
        Self {
            schema_version: RECORD_VERSION,
            format: RECORD_FORMAT.to_owned(),
            index_id: index.id().as_str().to_owned(),
            revision: index.number().get(),
            session_id: session_id.as_str().to_owned(),
            source_id: index.source_id().as_str().to_owned(),
            stream_index: index.stream_index(),
            duration_us: index.duration().as_micros(),
            profile: match index.profile() {
                VisualIndexProfile::R0 => StoredProfile::R0,
            },
            windows: index
                .windows()
                .iter()
                .map(StoredWindow::from_window)
                .collect(),
        }
    }

    fn into_index(self, session_id: &SessionId) -> Option<VisualIndex> {
        if self.format != RECORD_FORMAT || self.session_id != session_id.as_str() {
            return None;
        }
        let profile = match self.profile {
            StoredProfile::R0 => VisualIndexProfile::R0,
        };
        let source_id = SourceId::parse(self.source_id).ok()?;
        let duration = MediaTime::from_micros(self.duration_us);
        let mut windows = Vec::with_capacity(self.windows.len());
        for stored in self.windows {
            windows.push(stored.into_window(duration, profile)?);
        }
        let index = VisualIndex::new(VisualIndexParts {
            id: VisualIndexId::parse(self.index_id).ok()?,
            number: NonZeroU32::new(self.revision)?,
            source_id: source_id.clone(),
            stream_index: self.stream_index,
            duration,
            profile,
            windows,
        })
        .ok()?;
        let scope = VisualIndexScope {
            session_id,
            source_id: &source_id,
            stream_index: self.stream_index,
            duration,
            profile,
        };
        verify_visual_index_identities(&scope, &index).ok()?;
        Some(index)
    }
}

impl StoredWindow {
    fn from_window(window: &VisualIndexWindow) -> Self {
        let range = window.window().range();
        let mut stored = Self {
            ordinal: window.window().ordinal(),
            start_us: range.start().as_micros(),
            end_us: range.end().as_micros(),
            state: StoredWindowState::Undecodable,
            sample_count: None,
            dropped_candidates: None,
            frameless_cells: None,
            candidates: None,
        };
        if let VisualWindowOutcome::Analysed {
            sample_count,
            candidates,
            dropped_candidates,
            frameless_cells,
        } = window.outcome()
        {
            stored.state = StoredWindowState::Analysed;
            stored.sample_count = Some(*sample_count);
            stored.dropped_candidates = Some(*dropped_candidates);
            stored.frameless_cells = Some(
                frameless_cells
                    .iter()
                    .map(|cell| StoredRange {
                        start_us: cell.start().as_micros(),
                        end_us: cell.end().as_micros(),
                    })
                    .collect(),
            );
            stored.candidates = Some(
                candidates
                    .iter()
                    .map(StoredCandidate::from_candidate)
                    .collect(),
            );
        }
        stored
    }

    fn into_window(
        self,
        duration: MediaTime,
        profile: VisualIndexProfile,
    ) -> Option<VisualIndexWindow> {
        let window = VisualWindow::new(self.ordinal, duration).ok()?;
        if window.range().start().as_micros() != self.start_us
            || window.range().end().as_micros() != self.end_us
        {
            return None;
        }
        let outcome = match (
            self.state,
            self.sample_count,
            self.dropped_candidates,
            self.frameless_cells,
            self.candidates,
        ) {
            (StoredWindowState::Undecodable, None, None, None, None) => {
                VisualWindowOutcome::Undecodable
            }
            (
                StoredWindowState::Analysed,
                Some(sample_count),
                Some(dropped_candidates),
                Some(frameless_cells),
                Some(candidates),
            ) => {
                let mut cells = Vec::with_capacity(frameless_cells.len());
                for cell in frameless_cells {
                    cells.push(range(cell.start_us, cell.end_us)?);
                }
                let mut identified = Vec::with_capacity(candidates.len());
                for candidate in candidates {
                    identified.push(candidate.into_candidate()?);
                }
                VisualWindowOutcome::Analysed {
                    sample_count,
                    candidates: identified,
                    dropped_candidates,
                    frameless_cells: cells,
                }
            }
            _ => return None,
        };
        VisualIndexWindow::new(window, outcome, profile.policy()).ok()
    }
}

impl StoredCandidate {
    fn from_candidate(candidate: &VisualCandidate) -> Self {
        Self {
            id: candidate.id().as_str().to_owned(),
            reason: match candidate.reason() {
                CandidateReason::FirstFrame => StoredReason::FirstFrame,
                CandidateReason::VisualChange => StoredReason::VisualChange,
                CandidateReason::MotionStart => StoredReason::MotionStart,
                CandidateReason::SettledAfterMotion => StoredReason::SettledAfterMotion,
                CandidateReason::PeriodicCoverage => StoredReason::PeriodicCoverage,
            },
            representative_us: candidate.representative().as_micros(),
            span_end_us: candidate.span().end().as_micros(),
            sample_count: candidate.sample_count(),
            stability: match candidate.stability() {
                CandidateStability::Settled => StoredStability::Settled,
                CandidateStability::Transient => StoredStability::Transient,
                CandidateStability::InMotion => StoredStability::InMotion,
                CandidateStability::OpenAtWindowEnd => StoredStability::OpenAtWindowEnd,
            },
            visual_hash: candidate.hash().to_hex(),
            change: candidate.change().map(|change| StoredChange {
                previous_sample_us: change.previous_sample().as_micros(),
                changed_blocks: change.delta().changed_blocks(),
                max_block_delta: change.delta().max_block_delta(),
            }),
        }
    }

    fn into_candidate(self) -> Option<VisualCandidate> {
        let change = match self.change {
            None => None,
            Some(change) => Some(CandidateChange::new(
                MediaTime::from_micros(change.previous_sample_us),
                VisualDelta::new(change.changed_blocks, change.max_block_delta).ok()?,
            )),
        };
        Some(VisualCandidate::new(
            VisualCandidateId::parse(self.id).ok()?,
            CandidateDraft {
                reason: match self.reason {
                    StoredReason::FirstFrame => CandidateReason::FirstFrame,
                    StoredReason::VisualChange => CandidateReason::VisualChange,
                    StoredReason::MotionStart => CandidateReason::MotionStart,
                    StoredReason::SettledAfterMotion => CandidateReason::SettledAfterMotion,
                    StoredReason::PeriodicCoverage => CandidateReason::PeriodicCoverage,
                },
                representative: MediaTime::from_micros(self.representative_us),
                span: range(self.representative_us, self.span_end_us)?,
                sample_count: self.sample_count,
                stability: match self.stability {
                    StoredStability::Settled => CandidateStability::Settled,
                    StoredStability::Transient => CandidateStability::Transient,
                    StoredStability::InMotion => CandidateStability::InMotion,
                    StoredStability::OpenAtWindowEnd => CandidateStability::OpenAtWindowEnd,
                },
                change,
                hash: VisualHash::parse_hex(&self.visual_hash).ok()?,
            },
        ))
    }
}

fn range(start_us: u64, end_us: u64) -> Option<TimeRange> {
    TimeRange::new(
        MediaTime::from_micros(start_us),
        MediaTime::from_micros(end_us),
    )
    .ok()
}

#[cfg(test)]
mod tests;
