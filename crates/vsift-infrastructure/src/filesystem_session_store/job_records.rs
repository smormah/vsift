//! Stored shapes of a job: its record (`job.json`), an operation binding
//! (`jobs/by-operation/<op>.json`), its root index entry and its chunk
//! checkpoints (`chunks/<ordinal>.json`), each strictly versioned (ADR 0020
//! section 4).
//!
//! Every shape is decoded with unknown fields rejected and every value
//! validated; a newer schema version is reported as such. A job record whose
//! identity does not derive from its own keys is an integrity failure, so a
//! record copied from another job cannot answer for it. A checkpoint that
//! fails any check is unusable and is redone, never repaired (S-08).

use serde::{Deserialize, Serialize};
use vsift_application::{
    JobCommit, JobRecord, JobRequest, RequestDigest, SessionStorageError, job_id as derive_job_id,
};
use vsift_domain::{
    AttemptFailure, CheckpointOutcome, ChunkCheckpoint, ChunkTime, CueText, FailureCode, JobId,
    JobKind, JobState, LanguageTag, MAX_JOB_ATTEMPTS, MAX_PLANNED_CHUNKS, MAX_PROVIDER_SEGMENTS,
    MAX_PROVIDER_TOKENS, MediaTime, OperationId, OperationKey, ProviderChunkOutput,
    ProviderOutputError, ProviderSegment, ProviderToken, ProviderTokenKind, RecognitionKey,
    SessionId, Sha256Hex, StorageGeneration, TimeRange, TranscriptRevisionId,
};

use super::{MetadataVersion, STORAGE_SCHEMA_VERSION, sha256_hex, stored::parse_versioned_json};

/// Largest chunk checkpoint file.
pub(super) const MAX_CHECKPOINT_BYTES: u64 = 256 * 1024;
/// Most operation ids one job record lists.
const MAX_OPERATION_IDS: usize = vsift_application::MAX_JOB_OPERATION_IDS;

/// A half-open microsecond range as stored.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredRange {
    from_us: u64,
    to_us: u64,
}

impl StoredRange {
    const fn of(range: TimeRange) -> Self {
        Self {
            from_us: range.start().as_micros(),
            to_us: range.end().as_micros(),
        }
    }

    fn range(self) -> Option<TimeRange> {
        TimeRange::new(
            MediaTime::from_micros(self.from_us),
            MediaTime::from_micros(self.to_us),
        )
        .ok()
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredJobRequest {
    range: Option<StoredRange>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredFailure {
    chunk: Option<u32>,
    code: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredCommit {
    operation_id: String,
    observed_generation: u64,
    revision_id: String,
}

/// `job.json` v1.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredJob {
    schema_version: u16,
    job_id: String,
    session_id: String,
    kind: String,
    request_digest: String,
    operation_key: String,
    recognition_key: String,
    request: StoredJobRequest,
    /// Added in P10 PR 3; absent from records written before it (P10 PR 2
    /// builds reject records that have it, and sessions are disposable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    planned_chunks: Option<u32>,
    operation_ids: Vec<String>,
    state: String,
    epoch: u32,
    attempt: u32,
    failures: Vec<StoredFailure>,
    commit: Option<StoredCommit>,
    created_at_unix_seconds: u64,
    updated_at_unix_seconds: u64,
}

impl MetadataVersion for StoredJob {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }
}

/// Encodes a job record.
///
/// # Errors
///
/// [`SessionStorageError::Io`] only if serialisation failed.
pub(super) fn encode_job(record: &JobRecord) -> Result<Vec<u8>, SessionStorageError> {
    let JobRequest::Retranscribe { range } = record.request;
    let stored = StoredJob {
        schema_version: STORAGE_SCHEMA_VERSION,
        job_id: record.job_id.as_str().to_owned(),
        session_id: record.session_id.as_str().to_owned(),
        kind: record.request.kind().identifier().to_owned(),
        request_digest: record.request_digest.as_str().to_owned(),
        operation_key: record.operation_key.as_str().to_owned(),
        recognition_key: record.recognition_key.as_str().to_owned(),
        request: StoredJobRequest {
            range: range.map(StoredRange::of),
        },
        planned_chunks: record.planned_chunks,
        operation_ids: record
            .operation_ids
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect(),
        state: record.state.identifier().to_owned(),
        epoch: record.epoch,
        attempt: record.attempt,
        failures: record
            .failures
            .iter()
            .map(|failure| StoredFailure {
                chunk: failure.chunk,
                code: failure.code.identifier().to_owned(),
            })
            .collect(),
        commit: record.commit.as_ref().map(|commit| StoredCommit {
            operation_id: commit.operation_id.as_str().to_owned(),
            observed_generation: commit.observed_generation.value(),
            revision_id: commit.revision_id.as_str().to_owned(),
        }),
        created_at_unix_seconds: record.created_at_unix_seconds,
        updated_at_unix_seconds: record.updated_at_unix_seconds,
    };
    serde_json::to_vec(&stored).map_err(|_| SessionStorageError::Io)
}

fn parse_code(identifier: &str) -> Option<FailureCode> {
    FailureCode::ALL
        .into_iter()
        .find(|code| code.identifier() == identifier)
}

/// Decodes the record of `job_id` in `session_id`.
///
/// # Errors
///
/// [`SessionStorageError::UnsupportedVersion`] for a newer record and
/// [`SessionStorageError::IntegrityFailure`] for anything else that is not a
/// valid record of exactly this job.
pub(super) fn decode_job(
    bytes: &[u8],
    job_id: &JobId,
    session_id: &SessionId,
) -> Result<JobRecord, SessionStorageError> {
    let stored: StoredJob = parse_versioned_json(bytes)?;
    validate_job(stored, job_id, session_id).ok_or(SessionStorageError::IntegrityFailure)
}

fn validate_job(stored: StoredJob, job_id: &JobId, session_id: &SessionId) -> Option<JobRecord> {
    if stored.job_id != job_id.as_str() || stored.session_id != session_id.as_str() {
        return None;
    }
    let kind = JobKind::parse(&stored.kind)?;
    let request = match kind {
        JobKind::Retranscribe => JobRequest::Retranscribe {
            range: match stored.request.range {
                Some(range) => Some(range.range()?),
                None => None,
            },
        },
    };
    let operation_key = OperationKey::parse(stored.operation_key).ok()?;
    // The identity must derive from the record's own keys.
    if derive_job_id(session_id, &operation_key).ok()? != *job_id {
        return None;
    }
    if stored.operation_ids.len() > MAX_OPERATION_IDS
        || stored.attempt > MAX_JOB_ATTEMPTS
        || stored.failures.len() > vsift_application::MAX_RECORDED_FAILURES
        || stored.created_at_unix_seconds > stored.updated_at_unix_seconds
        || stored.planned_chunks.is_some_and(|planned| {
            planned == 0
                || usize::try_from(planned).map_or(true, |planned| planned > MAX_PLANNED_CHUNKS)
        })
    {
        return None;
    }
    let mut operation_ids = Vec::with_capacity(stored.operation_ids.len());
    for id in stored.operation_ids {
        let id = OperationId::parse(id).ok()?;
        if operation_ids.contains(&id) {
            return None;
        }
        operation_ids.push(id);
    }
    let state = JobState::parse(&stored.state)?;
    let mut failures = Vec::with_capacity(stored.failures.len());
    for failure in stored.failures {
        failures.push(AttemptFailure {
            chunk: failure.chunk,
            code: parse_code(&failure.code)?,
        });
    }
    let commit = match stored.commit {
        Some(commit) => Some(JobCommit {
            operation_id: OperationId::parse(commit.operation_id).ok()?,
            observed_generation: StorageGeneration::from_value(commit.observed_generation),
            revision_id: TranscriptRevisionId::parse(commit.revision_id).ok()?,
        }),
        None => None,
    };
    // Only a committing or succeeded job records the publication it made.
    if commit.is_some() != matches!(state, JobState::Committing | JobState::Succeeded) {
        return None;
    }
    Some(JobRecord {
        job_id: job_id.clone(),
        session_id: session_id.clone(),
        request_digest: RequestDigest::new(Sha256Hex::parse(stored.request_digest).ok()?),
        operation_key,
        recognition_key: RecognitionKey::new(Sha256Hex::parse(stored.recognition_key).ok()?),
        request,
        planned_chunks: stored.planned_chunks,
        operation_ids,
        state,
        epoch: stored.epoch,
        attempt: stored.attempt,
        failures,
        commit,
        created_at_unix_seconds: stored.created_at_unix_seconds,
        updated_at_unix_seconds: stored.updated_at_unix_seconds,
    })
}

/// `jobs/by-operation/<op>.json` v1: the job an operation id is bound to.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredBinding {
    schema_version: u16,
    operation_id: String,
    job_id: String,
}

impl MetadataVersion for StoredBinding {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }
}

impl StoredBinding {
    pub(super) fn new(operation_id: &OperationId, job_id: &JobId) -> Self {
        Self {
            schema_version: STORAGE_SCHEMA_VERSION,
            operation_id: operation_id.as_str().to_owned(),
            job_id: job_id.as_str().to_owned(),
        }
    }

    /// The bound job, if the binding is exactly for `operation_id`.
    pub(super) fn job(&self, operation_id: &OperationId) -> Option<JobId> {
        (self.operation_id == operation_id.as_str())
            .then(|| JobId::parse(self.job_id.as_str()).ok())
            .flatten()
    }
}

/// `job-index/<bucket>/<job_id>.json` v1: the session a job belongs to.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredJobIndex {
    schema_version: u16,
    session_id: String,
}

impl MetadataVersion for StoredJobIndex {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }
}

impl StoredJobIndex {
    pub(super) fn new(session_id: &SessionId) -> Self {
        Self {
            schema_version: STORAGE_SCHEMA_VERSION,
            session_id: session_id.as_str().to_owned(),
        }
    }

    pub(super) fn session(&self) -> Option<SessionId> {
        SessionId::parse(self.session_id.as_str()).ok()
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredToken {
    kind: String,
    /// The probability's IEEE 754 bits: a decimal round trip could change
    /// the last digit, and with it the resumed revision.
    probability_bits: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredSegment {
    start_us: u64,
    end_us: u64,
    text: Option<String>,
    /// The provider's original text, only when it differs from `text`.
    original: Option<String>,
    tokens: Vec<StoredToken>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredOutput {
    language: Option<String>,
    segments: Vec<StoredSegment>,
}

/// `chunks/<ordinal>.json` v1.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredCheckpoint {
    schema_version: u16,
    recognition_key: String,
    ordinal: u32,
    planned_range: StoredRange,
    outcome: String,
    audio: Option<StoredRange>,
    payload_sha256: Option<String>,
    payload: Option<String>,
}

impl MetadataVersion for StoredCheckpoint {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }
}

/// The 1-based ordinal a chunk's checkpoint is stored under.
pub(super) fn checkpoint_ordinal(index: u32) -> Option<u32> {
    index.checked_add(1)
}

/// The checkpoint file name of `ordinal`.
pub(super) fn checkpoint_name(ordinal: u32) -> String {
    format!("{ordinal:05}.json")
}

fn token_kind(kind: ProviderTokenKind) -> &'static str {
    match kind {
        ProviderTokenKind::Text => "text",
        ProviderTokenKind::Special => "special",
    }
}

/// Encodes a checkpoint; `None` only if serialisation failed.
pub(super) fn encode_checkpoint(checkpoint: &ChunkCheckpoint) -> Option<Vec<u8>> {
    let (outcome, audio, payload) = match checkpoint.outcome() {
        CheckpointOutcome::NoAudio => ("no_audio", None, None),
        CheckpointOutcome::Silent { audio } => ("silent", Some(StoredRange::of(*audio)), None),
        // The reason travels as the payload, under its own digest like any
        // other, so a damaged one is an unusable checkpoint and is redone.
        CheckpointOutcome::Unusable { audio, error } => (
            "unusable",
            Some(StoredRange::of(*audio)),
            Some(error.identifier().to_owned()),
        ),
        CheckpointOutcome::Recognised { audio, output } => {
            let stored = StoredOutput {
                language: output.language.as_ref().map(|tag| tag.as_str().to_owned()),
                segments: output
                    .segments
                    .iter()
                    .map(|segment| StoredSegment {
                        start_us: segment.start.as_micros(),
                        end_us: segment.end.as_micros(),
                        text: segment.text.as_ref().map(|text| text.text().to_owned()),
                        original: segment
                            .text
                            .as_ref()
                            .and_then(|text| text.original().map(str::to_owned)),
                        tokens: segment
                            .tokens
                            .iter()
                            .map(|token| StoredToken {
                                kind: token_kind(token.kind).to_owned(),
                                probability_bits: token.probability.to_bits(),
                            })
                            .collect(),
                    })
                    .collect(),
            };
            let payload = serde_json::to_string(&stored).ok()?;
            ("recognised", Some(StoredRange::of(*audio)), Some(payload))
        }
    };
    let stored = StoredCheckpoint {
        schema_version: STORAGE_SCHEMA_VERSION,
        recognition_key: checkpoint.key().as_str().to_owned(),
        ordinal: checkpoint_ordinal(checkpoint.index())?,
        planned_range: StoredRange::of(checkpoint.window()),
        outcome: outcome.to_owned(),
        audio,
        payload_sha256: payload.as_deref().map(|text| sha256_hex(text.as_bytes())),
        payload,
    };
    serde_json::to_vec(&stored).ok()
}

/// Decodes the checkpoint stored as `ordinal`, or `None` when it is not a
/// valid checkpoint v1 of that ordinal: truncated, forged, a payload that
/// does not match its digest, out of bounds, or a newer version.
pub(super) fn decode_checkpoint(bytes: &[u8], ordinal: u32) -> Option<ChunkCheckpoint> {
    let stored: StoredCheckpoint = parse_versioned_json(bytes).ok()?;
    if stored.ordinal != ordinal || ordinal == 0 {
        return None;
    }
    let key = RecognitionKey::new(Sha256Hex::parse(stored.recognition_key).ok()?);
    let window = stored.planned_range.range()?;
    let outcome = match (stored.outcome.as_str(), stored.audio, stored.payload) {
        ("no_audio", None, None) if stored.payload_sha256.is_none() => CheckpointOutcome::NoAudio,
        ("silent", Some(audio), None) if stored.payload_sha256.is_none() => {
            CheckpointOutcome::Silent {
                audio: audio.range()?,
            }
        }
        ("recognised", Some(audio), Some(payload)) => {
            if stored.payload_sha256.as_deref() != Some(sha256_hex(payload.as_bytes()).as_str()) {
                return None;
            }
            CheckpointOutcome::Recognised {
                audio: audio.range()?,
                output: decode_output(&payload)?,
            }
        }
        ("unusable", Some(audio), Some(payload)) => {
            if stored.payload_sha256.as_deref() != Some(sha256_hex(payload.as_bytes()).as_str()) {
                return None;
            }
            CheckpointOutcome::Unusable {
                audio: audio.range()?,
                error: ProviderOutputError::parse(&payload)?,
            }
        }
        _ => return None,
    };
    Some(ChunkCheckpoint::from_parts(
        key,
        ordinal - 1,
        window,
        outcome,
    ))
}

fn decode_output(payload: &str) -> Option<ProviderChunkOutput> {
    let stored: StoredOutput = serde_json::from_str(payload).ok()?;
    if stored.segments.len() > MAX_PROVIDER_SEGMENTS {
        return None;
    }
    let language = match stored.language {
        Some(tag) => Some(LanguageTag::parse(tag).ok()?),
        None => None,
    };
    let mut segments = Vec::with_capacity(stored.segments.len());
    for segment in stored.segments {
        if segment.tokens.len() > MAX_PROVIDER_TOKENS {
            return None;
        }
        let text = match (segment.text, segment.original) {
            (Some(text), original) => {
                let original = original.unwrap_or_else(|| text.clone());
                Some(CueText::new(text, original).ok()?)
            }
            (None, None) => None,
            (None, Some(_)) => return None,
        };
        let mut tokens = Vec::with_capacity(segment.tokens.len());
        for token in segment.tokens {
            tokens.push(ProviderToken {
                kind: match token.kind.as_str() {
                    "text" => ProviderTokenKind::Text,
                    "special" => ProviderTokenKind::Special,
                    _ => return None,
                },
                probability: f64::from_bits(token.probability_bits),
            });
        }
        segments.push(ProviderSegment {
            start: ChunkTime::from_micros(segment.start_us),
            end: ChunkTime::from_micros(segment.end_us),
            text,
            tokens,
        });
    }
    Some(ProviderChunkOutput { language, segments })
}

/// Decodes a job record (`job.json` v1) of `job_id` in `session_id`, exactly
/// as the store reads one back.
///
/// Public for the `job_record` fuzz target (ADR 0016 decision 6, issue
/// #180): the record is private storage, read only after ownership and link
/// checks, but it is still untrusted input to its decoder.
///
/// # Errors
///
/// [`SessionStorageError::UnsupportedVersion`] for a newer record and
/// [`SessionStorageError::IntegrityFailure`] for anything else that is not a
/// valid record of exactly this job.
pub fn decode_job_record(
    bytes: &[u8],
    job_id: &JobId,
    session_id: &SessionId,
) -> Result<JobRecord, SessionStorageError> {
    decode_job(bytes, job_id, session_id)
}

/// Encodes a job record exactly as the store writes one.
///
/// # Errors
///
/// [`SessionStorageError::Io`] only if serialisation failed.
pub fn encode_job_record(record: &JobRecord) -> Result<Vec<u8>, SessionStorageError> {
    encode_job(record)
}

/// Decodes the chunk checkpoint stored as the 1-based `ordinal`
/// (`chunks/<ordinal>.json` v1), exactly as a resumed run reads one; `None`
/// is a checkpoint the run discards and redoes.
///
/// Public for the `chunk_checkpoint` fuzz target (issue #180).
#[must_use]
pub fn decode_chunk_checkpoint(bytes: &[u8], ordinal: u32) -> Option<ChunkCheckpoint> {
    decode_checkpoint(bytes, ordinal)
}

/// Encodes a chunk checkpoint exactly as a run stores one; `None` only if
/// serialisation failed.
#[must_use]
pub fn encode_chunk_checkpoint(checkpoint: &ChunkCheckpoint) -> Option<Vec<u8>> {
    encode_checkpoint(checkpoint)
}

#[cfg(test)]
mod tests {
    use vsift_domain::{
        CheckpointOutcome, ChunkCheckpoint, ChunkTime, CueText, LanguageTag, MediaTime,
        PlannedChunk, ProviderChunkOutput, ProviderOutputError, ProviderSegment, ProviderToken,
        ProviderTokenKind, RecognitionKey, Sha256Hex, SourceSegmentId, TimeRange,
    };

    use super::{decode_checkpoint, encode_checkpoint};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn recognised() -> Result<ChunkCheckpoint, Box<dyn std::error::Error>> {
        let window = TimeRange::new(
            MediaTime::from_micros(0),
            MediaTime::from_micros(30_000_000),
        )?;
        let chunk = PlannedChunk::new(SourceSegmentId::parse("sgm_0123456789abcdef")?, 0, window);
        Ok(ChunkCheckpoint::new(
            RecognitionKey::new(Sha256Hex::parse("a".repeat(64))?),
            &chunk,
            CheckpointOutcome::Recognised {
                audio: window,
                output: ProviderChunkOutput {
                    language: LanguageTag::parse("en").ok(),
                    segments: vec![
                        ProviderSegment {
                            start: ChunkTime::from_micros(1_000_000),
                            end: ChunkTime::from_micros(2_000_000),
                            text: Some(CueText::new(
                                "plain words".to_owned(),
                                "<i>plain</i> words".to_owned(),
                            )?),
                            tokens: vec![
                                ProviderToken {
                                    kind: ProviderTokenKind::Text,
                                    probability: 0.1 + 0.2,
                                },
                                ProviderToken {
                                    kind: ProviderTokenKind::Special,
                                    probability: f64::MIN_POSITIVE,
                                },
                            ],
                        },
                        ProviderSegment {
                            start: ChunkTime::from_micros(2_000_000),
                            end: ChunkTime::from_micros(2_500_000),
                            text: None,
                            tokens: Vec::new(),
                        },
                    ],
                },
            },
        ))
    }

    /// The raw output survives exactly, probabilities bit for bit.
    #[test]
    fn a_checkpoint_round_trips_exactly() -> TestResult {
        let checkpoint = recognised()?;
        let bytes = encode_checkpoint(&checkpoint).ok_or("not encoded")?;
        assert_eq!(decode_checkpoint(&bytes, 1), Some(checkpoint));
        assert_eq!(decode_checkpoint(&bytes, 2), None);
        Ok(())
    }

    /// #353: the verdict that a chunk's answer was unusable, with its reason,
    /// round-trips for every reason, and a damaged one is an unusable
    /// checkpoint (discarded and redone), never taken for a verdict.
    #[test]
    fn an_unusable_verdict_round_trips_and_a_damaged_one_is_refused() -> TestResult {
        let window = TimeRange::new(
            MediaTime::from_micros(25_000_000),
            MediaTime::from_micros(55_000_000),
        )?;
        let chunk = PlannedChunk::new(SourceSegmentId::parse("sgm_0123456789abcdef")?, 1, window);
        for error in ProviderOutputError::ALL {
            let checkpoint = ChunkCheckpoint::new(
                RecognitionKey::new(Sha256Hex::parse("a".repeat(64))?),
                &chunk,
                CheckpointOutcome::Unusable {
                    audio: window,
                    error,
                },
            );
            let bytes = encode_checkpoint(&checkpoint).ok_or("not encoded")?;
            assert_eq!(decode_checkpoint(&bytes, 2), Some(checkpoint), "{error:?}");
        }
        let bytes = encode_checkpoint(&ChunkCheckpoint::new(
            RecognitionKey::new(Sha256Hex::parse("a".repeat(64))?),
            &chunk,
            CheckpointOutcome::Unusable {
                audio: window,
                error: ProviderOutputError::TooManyRejectedSegments,
            },
        ))
        .ok_or("not encoded")?;
        let text = String::from_utf8(bytes)?;
        // A reason that was changed after it was stored, or that names no
        // reason; a payload that does not match its digest; a verdict stored
        // as another kind.
        for (label, damaged) in [
            (
                "another reason",
                text.replace("too_many_rejected_segments", "too_many_tokens"),
            ),
            (
                "no reason",
                text.replace("too_many_rejected_segments", "malformed_output"),
            ),
            (
                "wrong digest",
                text.replace("\"payload_sha256\":\"", "\"payload_sha256\":\"0"),
            ),
            (
                "wrong kind",
                text.replace("\"outcome\":\"unusable\"", "\"outcome\":\"silent\""),
            ),
        ] {
            assert_eq!(decode_checkpoint(damaged.as_bytes(), 2), None, "{label}");
        }
        Ok(())
    }

    /// S-08: truncated, forged (payload without its digest), unknown
    /// fields and future versions are unusable.
    #[test]
    fn damaged_forged_or_future_checkpoints_are_unusable() -> TestResult {
        let bytes = encode_checkpoint(&recognised()?).ok_or("not encoded")?;
        let text = String::from_utf8(bytes.clone())?;
        let truncated = &bytes[..bytes.len() / 2];
        let forged = text.replace("plain words", "other words");
        let future = text.replace("\"schema_version\":1", "\"schema_version\":2");
        let unknown = text.replacen('{', "{\"extra\":1,", 1);
        let wrong_kind = text.replace("\"outcome\":\"recognised\"", "\"outcome\":\"silent\"");
        for (label, damaged) in [
            ("truncated", truncated.to_vec()),
            ("forged", forged.into_bytes()),
            ("future", future.into_bytes()),
            ("unknown field", unknown.into_bytes()),
            ("wrong kind", wrong_kind.into_bytes()),
            ("empty", Vec::new()),
        ] {
            assert_eq!(decode_checkpoint(&damaged, 1), None, "{label}");
        }
        Ok(())
    }
}
