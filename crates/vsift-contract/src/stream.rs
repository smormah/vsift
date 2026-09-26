//! The JSON Lines event stream: evidence record events and their terminal event.
//!
//! ADR 0016 decision 5 makes evidence records available as a JSON Lines stream,
//! so a pipeline or indexer can consume them one line at a time instead of
//! reading a page or a bundle. A stream is a finite sequence of self-describing
//! events. Every line carries the contract version, its event kind, a
//! contiguous `sequence` starting at 0, the command and the operation
//! identity. Evidence events come first, one per record; exactly one terminal
//! event ends the stream and carries the operation result, including the
//! paging cursor and a count of the records that preceded it.
//!
//! The sequencing lives here rather than in a host so every host emits the
//! same stream for the same page.

use serde::Serialize;
use vsift_domain::{SessionId, TimeRange, TranscriptRevision, TranscriptSegment};

use crate::{
    CONTRACT_VERSION, CommandName, LifecycleResponse, OperationResponse, TerminalEventResponse,
    TranscriptRevisionData, TranscriptSegmentData, VisualCandidateData, transcript::RangeData,
};

/// One published JSON Lines event kind, written to the event's `event` field.
///
/// A reader dispatches on this value. Within major v1 new kinds (for example
/// progress) may be added before the terminal event, so a reader skips a kind
/// it does not know but still counts its `sequence`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    /// One evidence record (`evidence-event.schema.json`).
    Evidence,
    /// The single event that ends a stream (`terminal-event.schema.json`).
    Terminal,
}

impl EventKind {
    /// Every published event kind, in declaration order.
    ///
    /// Contract tests compare this list with the `event` constants of the
    /// published event schemas, so a kind cannot be added on one side only.
    pub const ALL: [Self; 2] = [Self::Evidence, Self::Terminal];

    /// Returns the stable identifier written to the `event` field.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Evidence => "evidence",
            Self::Terminal => "terminal",
        }
    }

    /// Returns the variant's position in [`EventKind::ALL`]; the exhaustive
    /// match and the constant assertion below keep the list complete.
    const fn ordinal(self) -> usize {
        match self {
            Self::Evidence => 0,
            Self::Terminal => 1,
        }
    }
}

const _: () = {
    let mut index = 0;
    while index < EventKind::ALL.len() {
        assert!(EventKind::ALL[index].ordinal() == index);
        index += 1;
    }
};

/// One published evidence record type, written to an evidence event's
/// `record_type` field.
///
/// The record type names the schema of the event's `record` member and the
/// meaning of its upsert `key`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceRecordType {
    /// A transcript segment (`transcript-segment.schema.json`); its key is the
    /// segment's `segment_id`.
    TranscriptSegment,
    /// A visual candidate (`visual-candidate.schema.json`, P08); its key is
    /// the candidate's `candidate_id`.
    VisualCandidate,
}

impl EvidenceRecordType {
    /// Every published record type, in declaration order.
    ///
    /// Contract tests compare this list with the `record_type` enum of
    /// `evidence-event.schema.json`.
    pub const ALL: [Self; 2] = [Self::TranscriptSegment, Self::VisualCandidate];

    /// Returns the stable identifier written to the `record_type` field.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::TranscriptSegment => "transcript_segment",
            Self::VisualCandidate => "visual_candidate",
        }
    }

    /// Returns the variant's position in [`EvidenceRecordType::ALL`]; the
    /// exhaustive match and the constant assertion below keep the list complete.
    const fn ordinal(self) -> usize {
        match self {
            Self::TranscriptSegment => 0,
            Self::VisualCandidate => 1,
        }
    }
}

const _: () = {
    let mut index = 0;
    while index < EvidenceRecordType::ALL.len() {
        assert!(EvidenceRecordType::ALL[index].ordinal() == index);
        index += 1;
    }
};

/// The record carried by an evidence event, serialized as the record itself.
///
/// A private enum keeps the event type closed: only published record schemas
/// can be streamed, and a new record type is added here and to
/// [`EvidenceRecordType`] together.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
enum EvidenceRecord {
    TranscriptSegment(Box<TranscriptSegmentData>),
    VisualCandidate(Box<VisualCandidateData>),
}

/// One evidence record event: a single self-describing JSON Lines record.
///
/// `key` is the record's upsert key. Evidence records are immutable, so the
/// same key always carries the same record and an indexer may upsert by
/// (`record_type`, `key`) idempotently, however often a page is re-read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvidenceEventResponse {
    schema_version: &'static str,
    event: &'static str,
    sequence: u64,
    command: &'static str,
    operation_id: Option<String>,
    record_type: &'static str,
    key: String,
    record: EvidenceRecord,
}

impl EvidenceEventResponse {
    pub(crate) fn transcript_segment(
        sequence: u64,
        command: CommandName,
        revision: &TranscriptRevision,
        segment: &TranscriptSegment,
    ) -> Self {
        Self {
            schema_version: CONTRACT_VERSION,
            event: EventKind::Evidence.identifier(),
            sequence,
            command: command.identifier(),
            operation_id: None,
            record_type: EvidenceRecordType::TranscriptSegment.identifier(),
            key: segment.id().as_str().to_owned(),
            record: EvidenceRecord::TranscriptSegment(Box::new(TranscriptSegmentData::new(
                revision, segment,
            ))),
        }
    }
}

impl EvidenceEventResponse {
    pub(crate) fn visual_candidate(
        sequence: u64,
        command: CommandName,
        candidate: VisualCandidateData,
    ) -> Self {
        Self {
            schema_version: CONTRACT_VERSION,
            event: EventKind::Evidence.identifier(),
            sequence,
            command: command.identifier(),
            operation_id: None,
            record_type: EvidenceRecordType::VisualCandidate.identifier(),
            key: candidate.candidate_id().to_owned(),
            record: EvidenceRecord::VisualCandidate(Box::new(candidate)),
        }
    }
}

/// A finite JSON Lines evidence stream: its evidence events, then the one
/// terminal event that ends it.
///
/// Hosts write any stream through this one view, so every command that
/// streams evidence (`transcript get`, `search`, `candidates`) is written
/// identically.
pub trait EvidenceStream {
    /// The evidence events, in stream order.
    fn records(&self) -> &[EvidenceEventResponse];

    /// The terminal event that ends the stream.
    fn terminal(&self) -> &TerminalEventResponse;
}

/// Data of the terminal event that ends a `transcript.get` evidence stream.
///
/// It is the page's metadata without its items: the items were the preceding
/// evidence events, and `record_count` says how many there were so a reader
/// can prove it received all of them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranscriptStreamData {
    session_id: String,
    revision: TranscriptRevisionData,
    range: RangeData,
    record_count: usize,
    next_cursor: Option<String>,
}

/// One `transcript.get` page as a JSON Lines evidence stream.
///
/// The records are the page's segments in start order, numbered from 0; the
/// terminal event follows with the next sequence number. The stream is
/// bounded by the page limit: it holds at most that many evidence events and
/// exactly one terminal event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptEvidenceStream {
    records: Vec<EvidenceEventResponse>,
    terminal: TerminalEventResponse,
}

impl TranscriptEvidenceStream {
    /// Presents one page read from `revision` as an evidence stream.
    ///
    /// # Errors
    ///
    /// Returns the serialization error when the terminal data cannot be
    /// represented as JSON.
    pub fn new(
        session_id: &SessionId,
        revision: &TranscriptRevision,
        range: TimeRange,
        segments: &[TranscriptSegment],
        next_cursor: Option<&str>,
        lifecycle: LifecycleResponse,
    ) -> Result<Self, serde_json::Error> {
        let command = CommandName::TranscriptGet;
        let records: Vec<_> = (0_u64..)
            .zip(segments)
            .map(|(sequence, segment)| {
                EvidenceEventResponse::transcript_segment(sequence, command, revision, segment)
            })
            .collect();
        let data = TranscriptStreamData {
            session_id: session_id.as_str().to_owned(),
            revision: TranscriptRevisionData::new(revision),
            range: RangeData::new(range),
            record_count: records.len(),
            next_cursor: next_cursor.map(str::to_owned),
        };
        let result =
            OperationResponse::complete(command.identifier(), &data)?.with_lifecycle(lifecycle);
        let terminal_sequence = u64::try_from(records.len()).unwrap_or(u64::MAX);
        Ok(Self {
            records,
            terminal: TerminalEventResponse::at_sequence(result, terminal_sequence),
        })
    }

    /// The evidence events, in stream order.
    #[must_use]
    pub fn records(&self) -> &[EvidenceEventResponse] {
        &self.records
    }

    /// The terminal event that ends the stream.
    #[must_use]
    pub const fn terminal(&self) -> &TerminalEventResponse {
        &self.terminal
    }
}

impl EvidenceStream for TranscriptEvidenceStream {
    fn records(&self) -> &[EvidenceEventResponse] {
        &self.records
    }

    fn terminal(&self) -> &TerminalEventResponse {
        &self.terminal
    }
}

#[cfg(test)]
mod tests {
    use super::{EventKind, EvidenceRecordType};

    #[test]
    fn event_kind_identifiers_are_distinct() {
        assert_ne!(
            EventKind::Evidence.identifier(),
            EventKind::Terminal.identifier()
        );
    }

    #[test]
    fn record_type_identifiers_are_snake_case() {
        for record_type in EvidenceRecordType::ALL {
            assert!(
                record_type
                    .identifier()
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
            );
        }
    }
}
