//! Recorded results (P11 PR 3, ADR 0021 section 4).
//!
//! A worker host keeps each request's finished steps and, once the request
//! has ended, its whole result in a request record, so a redelivered
//! request continues from its first unfinished step or is answered with the
//! recorded result without new work. The record's storage adapter treats
//! these documents as opaque bounded bytes; this module owns their shape,
//! because the result is a contract type and must be written and read back
//! by one set of rules.
//!
//! A recorded document is the canonical serialization of the value
//! (`serde_json`'s compact form in declaration order, `replayed: false`).
//! Reading one back is strict: every member is required and typed, every
//! identifier is checked against its grammar and every enum against its
//! identifiers, and the decoded value must serialize to exactly the bytes
//! that were read. A document any other writer produced, or one that was
//! changed, is refused rather than guessed at.

use std::{
    error::Error,
    fmt,
    num::{NonZeroU16, NonZeroU32},
};

use serde::Deserialize;
use vsift_domain::{
    CoverageGapReason, FailureCode, JobId, OperationId, OperationStatus, PublicationGuarantee,
    SessionId, Sha256Hex, SourceId, TranscriptRevisionId, VisualIndexId,
};

use super::{
    FreeSpaceReserve, MAX_RESULT_STEPS, MAX_WORK_RESULT_BYTES, OutputsData, RequestFailure,
    ResourceLimits, ResultOrigin, StepFailureData, StepResult, StepStatus, WorkControls,
    WorkFailure, WorkResult, WorkResultParts, WorkerIsolation,
};
use crate::{
    CoverageResponse, LifecycleResponse, MAX_COVERAGE_RANGES, RequestRejection, WorkRequestDigest,
    WorkStepKind, request::BundleName,
};

/// The reason a truncated gap list names (see the candidates contract).
const GAPS_TRUNCATED_REASON: &str = "gap_list_truncated";

/// Why a recorded step or result could not be read back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordedResultError {
    /// More than [`MAX_WORK_RESULT_BYTES`].
    TooLarge,
    /// Not the shape a recorded document has: a missing, unknown or
    /// mistyped member, an identifier outside its grammar, an unknown enum
    /// value, or a result the contract would refuse to present.
    Malformed,
    /// The document is not the canonical serialization of what it decodes
    /// to: it was changed or written by something else.
    NotCanonical,
}

impl fmt::Display for RecordedResultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "a recorded job result is larger than a job result may be",
            Self::Malformed => "a recorded job result is malformed",
            Self::NotCanonical => "a recorded job result is not in its canonical form",
        })
    }
}

impl Error for RecordedResultError {}

impl StepResult {
    /// The step's kind.
    #[must_use]
    pub const fn kind(&self) -> WorkStepKind {
        self.step_kind
    }

    /// The failure that ended the step, if it failed or was cancelled.
    #[must_use]
    pub const fn failure(&self) -> Option<WorkFailure> {
        self.cause
    }

    /// The canonical bytes a request record keeps for a finished step.
    ///
    /// # Errors
    ///
    /// [`RecordedResultError::TooLarge`] for a step over the result bound
    /// (not reachable from a real run).
    pub fn recorded_bytes(&self) -> Result<Vec<u8>, RecordedResultError> {
        let bytes = serde_json::to_vec(self).map_err(|_| RecordedResultError::Malformed)?;
        if bytes.len() > MAX_WORK_RESULT_BYTES {
            return Err(RecordedResultError::TooLarge);
        }
        Ok(bytes)
    }

    /// Reads a step back from its recorded bytes.
    ///
    /// # Errors
    ///
    /// [`RecordedResultError`] for anything but the canonical bytes of a
    /// step.
    pub fn decode_recorded(bytes: &[u8]) -> Result<Self, RecordedResultError> {
        if bytes.len() > MAX_WORK_RESULT_BYTES {
            return Err(RecordedResultError::TooLarge);
        }
        let wire: WireStep =
            serde_json::from_slice(bytes).map_err(|_| RecordedResultError::Malformed)?;
        let step = wire.decode()?;
        canonical(&step, bytes)?;
        Ok(step)
    }
}

impl WorkResult {
    /// The request's attempt that produced the result.
    #[must_use]
    pub const fn attempt(&self) -> u32 {
        self.attempt
    }

    /// The session the request created or used, if known.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Whether the result was returned from its record without new work.
    #[must_use]
    pub const fn replayed(&self) -> bool {
        self.replayed
    }

    /// The steps, in order.
    #[must_use]
    pub fn steps(&self) -> &[StepResult] {
        &self.steps
    }

    /// The rejection that refused the request, if one did.
    #[must_use]
    pub const fn rejection(&self) -> Option<RequestRejection> {
        self.rejection
    }

    /// The lifecycle of the request's session, once it had one.
    #[must_use]
    pub const fn lifecycle(&self) -> Option<&LifecycleResponse> {
        self.lifecycle.as_ref()
    }

    /// The retry hint of the failure that ended the request, if any.
    #[must_use]
    pub fn retry_after_ms(&self) -> Option<u64> {
        self.failure.and_then(|failure| failure.retry_after_ms)
    }

    /// The same result marked as returned from its record: the one change a
    /// replay makes (`replayed: true`).
    #[must_use]
    pub fn into_replayed(mut self) -> Self {
        self.replayed = true;
        self
    }

    /// The canonical bytes a request record keeps for an ended request:
    /// the result as it was first presented, with `replayed: false`, so a
    /// replay can prove it returns exactly what was recorded.
    ///
    /// # Errors
    ///
    /// [`RecordedResultError::TooLarge`] for a result over
    /// [`MAX_WORK_RESULT_BYTES`].
    pub fn recorded_bytes(&self) -> Result<Vec<u8>, RecordedResultError> {
        let mut fresh = self.clone();
        fresh.replayed = false;
        let bytes = serde_json::to_vec(&fresh).map_err(|_| RecordedResultError::Malformed)?;
        if bytes.len() > MAX_WORK_RESULT_BYTES {
            return Err(RecordedResultError::TooLarge);
        }
        Ok(bytes)
    }

    /// Reads a result back from its recorded bytes; it comes back as it was
    /// recorded (`replayed: false`).
    ///
    /// # Errors
    ///
    /// [`RecordedResultError`] for anything but the canonical bytes of a
    /// result the contract can present.
    pub fn decode_recorded(bytes: &[u8]) -> Result<Self, RecordedResultError> {
        if bytes.len() > MAX_WORK_RESULT_BYTES {
            return Err(RecordedResultError::TooLarge);
        }
        let wire: WireResult =
            serde_json::from_slice(bytes).map_err(|_| RecordedResultError::Malformed)?;
        let result = wire.decode()?;
        canonical(&result, bytes)?;
        Ok(result)
    }
}

/// Requires `value` to serialize to exactly `bytes`.
fn canonical<T: serde::Serialize>(value: &T, bytes: &[u8]) -> Result<(), RecordedResultError> {
    let written = serde_json::to_vec(value).map_err(|_| RecordedResultError::Malformed)?;
    if written == bytes {
        Ok(())
    } else {
        Err(RecordedResultError::NotCanonical)
    }
}

/// The first of `candidates` whose identifier is `text`.
fn identified<T: Copy>(
    candidates: &[T],
    identifier: impl Fn(T) -> &'static str,
    text: &str,
) -> Result<T, RecordedResultError> {
    candidates
        .iter()
        .copied()
        .find(|candidate| identifier(*candidate) == text)
        .ok_or(RecordedResultError::Malformed)
}

const OPERATION_STATUSES: [OperationStatus; 4] = [
    OperationStatus::Complete,
    OperationStatus::Partial,
    OperationStatus::Failed,
    OperationStatus::Cancelled,
];

const PUBLICATIONS: [PublicationGuarantee; 2] = [
    PublicationGuarantee::ProcessCrashConsistent,
    PublicationGuarantee::OsCrashDurable,
];

fn malformed<T, E>(result: Result<T, E>) -> Result<T, RecordedResultError> {
    result.map_err(|_| RecordedResultError::Malformed)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFailure {
    code: String,
    retryable: bool,
    retry_after_ms: Option<u64>,
}

impl WireFailure {
    fn decode(&self) -> Result<WorkFailure, RecordedResultError> {
        let code = identified(&FailureCode::ALL, FailureCode::identifier, &self.code)?;
        let failure = WorkFailure::new(code, self.retry_after_ms);
        if failure.retry_after_ms != self.retry_after_ms || self.retryable != code.retryable() {
            return Err(RecordedResultError::Malformed);
        }
        Ok(failure)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireCoverage {
    truncated: bool,
    gaps: Vec<String>,
    reasons: Vec<String>,
}

impl WireCoverage {
    fn decode(self) -> Result<CoverageResponse, RecordedResultError> {
        if self.gaps.len() > MAX_COVERAGE_RANGES
            || self.reasons.len() > CoverageGapReason::ALL.len() + 1
        {
            return Err(RecordedResultError::Malformed);
        }
        for gap in &self.gaps {
            let (from, to) = gap.split_once('-').ok_or(RecordedResultError::Malformed)?;
            let (from, to) = (
                malformed(from.parse::<u64>())?,
                malformed(to.parse::<u64>())?,
            );
            if to <= from {
                return Err(RecordedResultError::Malformed);
            }
        }
        if !self.reasons.iter().all(|reason| {
            reason == GAPS_TRUNCATED_REASON
                || CoverageGapReason::ALL
                    .iter()
                    .any(|known| known.identifier() == reason)
        }) {
            return Err(RecordedResultError::Malformed);
        }
        Ok(CoverageResponse::new(
            self.truncated,
            self.gaps,
            self.reasons,
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireIngestOutputs {
    generation: u64,
    revision_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRetranscribeOutputs {
    revision_id: String,
    generation: u64,
    chunks_reused: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireCandidatesOutputs {
    visual_index_id: String,
    generation: u64,
    candidate_count: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRetainOutputs {
    bundle_name: String,
    bundle_sha256: String,
    artifact_count: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireCloseOutputs {
    generation: u64,
}

fn outputs_of(
    kind: WorkStepKind,
    value: serde_json::Value,
) -> Result<OutputsData, RecordedResultError> {
    Ok(match kind {
        WorkStepKind::Ingest => {
            let outputs: WireIngestOutputs = malformed(serde_json::from_value(value))?;
            if let Some(revision) = &outputs.revision_id {
                malformed(TranscriptRevisionId::parse(revision.as_str()))?;
            }
            OutputsData::Ingest {
                generation: outputs.generation,
                revision_id: outputs.revision_id,
            }
        }
        WorkStepKind::Retranscribe => {
            let outputs: WireRetranscribeOutputs = malformed(serde_json::from_value(value))?;
            malformed(TranscriptRevisionId::parse(outputs.revision_id.as_str()))?;
            OutputsData::Retranscribe {
                revision_id: outputs.revision_id,
                generation: outputs.generation,
                chunks_reused: outputs.chunks_reused,
            }
        }
        WorkStepKind::Candidates => {
            let outputs: WireCandidatesOutputs = malformed(serde_json::from_value(value))?;
            malformed(VisualIndexId::parse(outputs.visual_index_id.as_str()))?;
            OutputsData::Candidates {
                visual_index_id: outputs.visual_index_id,
                generation: outputs.generation,
                candidate_count: outputs.candidate_count,
            }
        }
        WorkStepKind::Retain => {
            let outputs: WireRetainOutputs = malformed(serde_json::from_value(value))?;
            malformed(BundleName::parse(&outputs.bundle_name))?;
            malformed(Sha256Hex::parse(outputs.bundle_sha256.as_str()))?;
            OutputsData::Retain {
                bundle_name: outputs.bundle_name,
                bundle_sha256: outputs.bundle_sha256,
                artifact_count: outputs.artifact_count,
            }
        }
        WorkStepKind::Close => {
            let outputs: WireCloseOutputs = malformed(serde_json::from_value(value))?;
            OutputsData::Close {
                generation: outputs.generation,
            }
        }
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireStep {
    kind: String,
    status: String,
    elapsed_ms: u64,
    admission_wait_ms: u64,
    job_id: Option<String>,
    outputs: Option<serde_json::Value>,
    coverage: Option<WireCoverage>,
    failure: Option<WireFailure>,
}

impl WireStep {
    fn decode(self) -> Result<StepResult, RecordedResultError> {
        let kind = identified(&WorkStepKind::ALL, WorkStepKind::identifier, &self.kind)?;
        let state = identified(&StepStatus::ALL, StepStatus::identifier, &self.status)?;
        if let Some(job) = &self.job_id {
            malformed(JobId::parse(job.as_str()))?;
        }
        let cause = self.failure.as_ref().map(WireFailure::decode).transpose()?;
        let outputs = self
            .outputs
            .map(|outputs| outputs_of(kind, outputs))
            .transpose()?;
        let coverage = self.coverage.map(WireCoverage::decode).transpose()?;
        // The states a real step is presented in, and nothing else.
        let consistent = match state {
            StepStatus::Complete | StepStatus::Partial => {
                outputs.is_some()
                    && cause.is_none()
                    && (coverage.as_ref().is_some_and(CoverageResponse::truncated)
                        == (state == StepStatus::Partial))
                    && (coverage.is_some() == (kind == WorkStepKind::Candidates))
            }
            StepStatus::Failed => {
                outputs.is_none()
                    && coverage.is_none()
                    && cause.is_some_and(|cause| cause.code() != FailureCode::Cancelled)
            }
            StepStatus::Cancelled => {
                outputs.is_none()
                    && coverage.is_none()
                    && cause.is_some_and(|cause| cause.code() == FailureCode::Cancelled)
            }
            StepStatus::NotStarted => {
                outputs.is_none()
                    && coverage.is_none()
                    && cause.is_none()
                    && self.job_id.is_none()
                    && self.elapsed_ms == 0
                    && self.admission_wait_ms == 0
            }
        };
        if !consistent {
            return Err(RecordedResultError::Malformed);
        }
        Ok(StepResult {
            kind: kind.identifier(),
            step_kind: kind,
            status: state.identifier(),
            elapsed_ms: self.elapsed_ms,
            admission_wait_ms: self.admission_wait_ms,
            job_id: self.job_id,
            outputs,
            coverage,
            failure: cause.map(StepFailureData::of),
            state,
            cause,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireLifecycle {
    mode: String,
    expires_at: Option<String>,
}

impl WireLifecycle {
    fn decode(self) -> Result<LifecycleResponse, RecordedResultError> {
        let bounded = |text: &str| {
            text.len() <= 64
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-:.+".contains(&byte))
        };
        match (self.mode.as_str(), self.expires_at) {
            ("ephemeral", Some(expires)) if bounded(&expires) => {
                Ok(LifecycleResponse::ephemeral(expires))
            }
            ("durable_worker", Some(expires)) if bounded(&expires) => {
                Ok(LifecycleResponse::durable_worker(expires))
            }
            ("retained", None) => Ok(LifecycleResponse::retained()),
            _ => Err(RecordedResultError::Malformed),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRequestFailure {
    code: String,
    retryable: bool,
    retry_after_ms: Option<u64>,
    step: Option<usize>,
    rejection: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireControls {
    isolation: String,
    admission_capacity: u16,
    concurrency: u16,
    resource_limits: String,
    free_space_reserve: String,
}

impl WireControls {
    fn decode(&self) -> Result<WorkControls, RecordedResultError> {
        Ok(WorkControls {
            isolation: identified(
                &WorkerIsolation::ALL,
                WorkerIsolation::identifier,
                &self.isolation,
            )?,
            admission_capacity: NonZeroU16::new(self.admission_capacity)
                .ok_or(RecordedResultError::Malformed)?,
            concurrency: NonZeroU16::new(self.concurrency).ok_or(RecordedResultError::Malformed)?,
            resource_limits: identified(
                &ResourceLimits::ALL,
                ResourceLimits::identifier,
                &self.resource_limits,
            )?,
            free_space_reserve: identified(
                &FreeSpaceReserve::ALL,
                FreeSpaceReserve::identifier,
                &self.free_space_reserve,
            )?,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResult {
    operation_id: String,
    request_digest: String,
    status: String,
    replayed: bool,
    attempt: u32,
    session_id: Option<String>,
    source_id: Option<String>,
    publication: Option<String>,
    lifecycle: Option<WireLifecycle>,
    steps: Vec<WireStep>,
    failure: Option<WireRequestFailure>,
    controls: WireControls,
}

impl WireResult {
    /// Decodes every member, then presents the result again through
    /// [`WorkResult::new`], so a recorded result obeys every rule a fresh
    /// one does (its derived status, its failure, no step after an ending
    /// one); the caller then requires the same bytes.
    fn decode(self) -> Result<WorkResult, RecordedResultError> {
        if self.replayed || self.steps.len() > MAX_RESULT_STEPS {
            return Err(RecordedResultError::Malformed);
        }
        let operation_id = malformed(OperationId::parse(self.operation_id.as_str()))?;
        let request_digest = malformed(WorkRequestDigest::parse(&self.request_digest))?;
        let attempt = NonZeroU32::new(self.attempt).ok_or(RecordedResultError::Malformed)?;
        let session_id = self
            .session_id
            .map(|session| malformed(SessionId::parse(session)))
            .transpose()?;
        let source_id = self
            .source_id
            .map(|source| malformed(SourceId::parse(source)))
            .transpose()?;
        let publication = self
            .publication
            .as_deref()
            .map(|text| identified(&PUBLICATIONS, PublicationGuarantee::identifier, text))
            .transpose()?;
        let lifecycle = self.lifecycle.map(WireLifecycle::decode).transpose()?;
        let steps = self
            .steps
            .into_iter()
            .map(WireStep::decode)
            .collect::<Result<Vec<_>, _>>()?;
        identified(
            &OPERATION_STATUSES,
            OperationStatus::identifier,
            &self.status,
        )?;
        // A step's failure is derived from the step; only a failure before
        // any step is the request's own.
        let failure = match self.failure {
            Some(failure) if failure.step.is_none() => {
                let cause = WireFailure {
                    code: failure.code,
                    retryable: failure.retryable,
                    retry_after_ms: failure.retry_after_ms,
                }
                .decode()?;
                Some(match failure.rejection.as_deref() {
                    Some(text) => RequestFailure::Rejected(identified(
                        &RequestRejection::ALL,
                        RequestRejection::identifier,
                        text,
                    )?),
                    None => RequestFailure::NotStarted(cause),
                })
            }
            Some(_) | None => None,
        };
        malformed(WorkResult::new(WorkResultParts {
            operation_id: &operation_id,
            request_digest: &request_digest,
            origin: ResultOrigin::Fresh,
            attempt,
            session_id: session_id.as_ref(),
            source_id: source_id.as_ref(),
            publication,
            lifecycle,
            steps,
            failure,
            controls: self.controls.decode()?,
        }))
    }
}
