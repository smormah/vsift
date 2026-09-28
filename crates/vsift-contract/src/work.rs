//! The worker result: `job-result` v1 (P11, ADR 0021).
//!
//! One [`WorkResult`] answers one [`crate::WorkRequest`]: as the data of a
//! `job run` result, and in a `result` event of a `job batch` stream. It
//! reports what each step did (its status, timing, the job behind it, typed
//! outputs such as a revision or bundle digest, coverage and failure) and
//! the controls the host ran it under, so a supervisor can tell busy from
//! unhealthy from missing capability (O-03) without scraping text.
//!
//! A result never names a path (bundles are named by their `bundle_name`
//! under the operator's bundle root) and never carries evidence text; its
//! members are identities, counts, digests and enums, so it stays within
//! [`MAX_WORK_RESULT_BYTES`] by construction.

mod recorded;

use std::{error::Error, fmt, num::NonZeroU16, num::NonZeroU32};

pub use recorded::RecordedResultError;

use serde::Serialize;
use vsift_domain::{
    FailureCode, JobId, OperationId, OperationStatus, PublicationGuarantee, SessionId, Sha256Hex,
    SourceId, StorageGeneration, TranscriptRevisionId, VisualCoverageGap, VisualIndexId,
};

use crate::{
    CoverageResponse, LifecycleResponse, MAX_REQUEST_STEPS, RequestRejection, WorkRequestDigest,
    WorkStepKind, candidates::envelope_coverage, request::BundleName,
};

/// Most bytes one serialized result may take: the same bound as a request.
pub const MAX_WORK_RESULT_BYTES: usize = 65_536;
/// The envelope warning of a `partial` `job run` result.
pub const PARTIAL_REQUEST_WARNING: &str =
    "The request completed with a stated gap; see the coverage of each partial step.";
/// Most steps one result reports: the ingest of its target and every
/// requested step.
pub const MAX_RESULT_STEPS: usize = MAX_REQUEST_STEPS + 1;

/// What happened to one step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepStatus {
    /// The step did all it was asked.
    Complete,
    /// The step committed useful output with a stated gap.
    Partial,
    /// The step failed; nothing after it ran.
    Failed,
    /// The step was cancelled; nothing after it ran.
    Cancelled,
    /// The step never started because an earlier one failed or was cancelled.
    NotStarted,
}

impl StepStatus {
    /// Every status, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::Complete,
        Self::Partial,
        Self::Failed,
        Self::Cancelled,
        Self::NotStarted,
    ];

    /// The stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::NotStarted => "not_started",
        }
    }

    const fn ended_the_request(self) -> bool {
        matches!(self, Self::Failed | Self::Cancelled)
    }
}

/// The isolation a host ran a request under.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerIsolation {
    /// Process lifecycle containment only (Job Object or process group).
    ProcessOnly,
    /// The attested strict Linux worker profile: cgroup v2 CPU, memory and
    /// PID limits, a read-only root and no network interface but loopback.
    StrictLinux,
}

impl WorkerIsolation {
    /// Every isolation, in declaration order.
    pub const ALL: [Self; 2] = [Self::ProcessOnly, Self::StrictLinux];

    /// The stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::ProcessOnly => "process_only",
            Self::StrictLinux => "strict_linux",
        }
    }
}

/// Where the memory and process limits a request ran under come from (P11
/// PR 2). `VSift` never sets or claims to enforce them itself: they are the
/// host's cgroup limits, attested with the strict Linux profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceLimits {
    /// The attested host cgroup's finite CPU, memory and PID limits.
    HostCgroup,
    /// No host limit was attested.
    NotEnforced,
}

impl ResourceLimits {
    /// Every value, in declaration order.
    pub const ALL: [Self; 2] = [Self::HostCgroup, Self::NotEnforced];

    /// The stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::HostCgroup => "host_cgroup",
            Self::NotEnforced => "not_enforced",
        }
    }
}

/// Whether the workspace's free-space reserve was checked before a source
/// was copied into it (P11 PR 2): on Unix the filesystem's available space
/// is compared with the source and the reserve; on Windows it is not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FreeSpaceReserve {
    /// Checked before the copy.
    Enforced,
    /// Not checked on this platform.
    NotEnforced,
}

impl FreeSpaceReserve {
    /// Every value, in declaration order.
    pub const ALL: [Self; 2] = [Self::Enforced, Self::NotEnforced];

    /// The stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Enforced => "enforced",
            Self::NotEnforced => "not_enforced",
        }
    }
}

/// Whether a result was computed now or returned from an earlier commit of
/// the same operation id and request digest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResultOrigin {
    /// Computed by this call.
    Fresh,
    /// The recorded result of an earlier call, returned without new work.
    Replayed,
}

/// The controls a request ran under.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkControls {
    /// The isolation profile.
    pub isolation: WorkerIsolation,
    /// The workspace's admission capacity, in weight units.
    pub admission_capacity: NonZeroU16,
    /// The host's request concurrency.
    pub concurrency: NonZeroU16,
    /// Where its memory and process limits come from.
    pub resource_limits: ResourceLimits,
    /// Whether the free-space reserve was checked before its copy.
    pub free_space_reserve: FreeSpaceReserve,
}

/// How long a step took.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StepTiming {
    /// Wall time from the step's start to its end, including admission.
    pub elapsed_ms: u64,
    /// Time spent waiting for admission.
    pub admission_wait_ms: u64,
}

/// A step's or a request's failure: a public code, whether retrying can
/// help, and how long to wait first.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkFailure {
    code: FailureCode,
    retry_after_ms: Option<u64>,
}

impl WorkFailure {
    /// A failure with `code`; a retry hint is kept only for a retryable code.
    #[must_use]
    pub fn new(code: FailureCode, retry_after_ms: Option<u64>) -> Self {
        Self {
            code,
            retry_after_ms: retry_after_ms
                .filter(|_| code.retryable())
                .map(|delay| delay.clamp(1, 86_400_000)),
        }
    }

    /// The public code.
    #[must_use]
    pub const fn code(self) -> FailureCode {
        self.code
    }
}

/// What a step produced, typed per kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StepOutputs {
    /// The ingest of the target: the new session's generation and the
    /// imported transcript revision, if a transcript was supplied.
    Ingest {
        /// The generation the ingest committed.
        generation: StorageGeneration,
        /// The imported revision.
        revision_id: Option<TranscriptRevisionId>,
    },
    /// A committed local-ASR revision.
    Retranscribe {
        /// The new revision.
        revision_id: TranscriptRevisionId,
        /// The generation it was committed in.
        generation: StorageGeneration,
        /// Chunks taken from checkpoints of an earlier attempt.
        chunks_reused: u32,
    },
    /// The visual index after the range was analysed as far as it can be.
    Candidates {
        /// The index revision.
        visual_index_id: VisualIndexId,
        /// The generation it was committed in.
        generation: StorageGeneration,
        /// Candidates in the range.
        candidate_count: u32,
        /// Parts of the range that stay uncovered (undecodable, no decoded
        /// frame, candidate budget): the step is `partial` when any remain.
        gaps: Vec<VisualCoverageGap>,
    },
    /// A retained bundle.
    Retain {
        /// Its directory name below the bundle root.
        bundle_name: BundleName,
        /// SHA-256 of its manifest, checked again on a replay.
        bundle_sha256: Sha256Hex,
        /// Artifacts it holds.
        artifact_count: u32,
    },
    /// The closed session.
    Close {
        /// The generation that recorded the close.
        generation: StorageGeneration,
    },
}

impl StepOutputs {
    const fn kind(&self) -> WorkStepKind {
        match self {
            Self::Ingest { .. } => WorkStepKind::Ingest,
            Self::Retranscribe { .. } => WorkStepKind::Retranscribe,
            Self::Candidates { .. } => WorkStepKind::Candidates,
            Self::Retain { .. } => WorkStepKind::Retain,
            Self::Close { .. } => WorkStepKind::Close,
        }
    }
}

/// One step of a result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StepResult {
    kind: &'static str,
    status: &'static str,
    elapsed_ms: u64,
    admission_wait_ms: u64,
    job_id: Option<String>,
    outputs: Option<OutputsData>,
    coverage: Option<CoverageResponse>,
    failure: Option<StepFailureData>,
    #[serde(skip)]
    step_kind: WorkStepKind,
    #[serde(skip)]
    state: StepStatus,
    #[serde(skip)]
    cause: Option<WorkFailure>,
}

impl StepResult {
    /// A step that finished: `partial` when its outputs leave a gap.
    #[must_use]
    pub fn finished(outputs: &StepOutputs, timing: StepTiming, job: Option<&JobId>) -> Self {
        let coverage = match outputs {
            StepOutputs::Candidates { gaps, .. } => Some(envelope_coverage(gaps)),
            _ => None,
        };
        let state = if coverage.as_ref().is_some_and(CoverageResponse::truncated) {
            StepStatus::Partial
        } else {
            StepStatus::Complete
        };
        Self {
            kind: outputs.kind().identifier(),
            step_kind: outputs.kind(),
            status: state.identifier(),
            elapsed_ms: timing.elapsed_ms,
            admission_wait_ms: timing.admission_wait_ms,
            job_id: job.map(|job| job.as_str().to_owned()),
            outputs: Some(OutputsData::of(outputs)),
            coverage,
            failure: None,
            state,
            cause: None,
        }
    }

    /// A step that failed, or was cancelled when `failure` is `CANCELLED`.
    #[must_use]
    pub fn failed(
        kind: WorkStepKind,
        timing: StepTiming,
        job: Option<&JobId>,
        failure: WorkFailure,
    ) -> Self {
        let state = if failure.code == FailureCode::Cancelled {
            StepStatus::Cancelled
        } else {
            StepStatus::Failed
        };
        Self {
            kind: kind.identifier(),
            step_kind: kind,
            status: state.identifier(),
            elapsed_ms: timing.elapsed_ms,
            admission_wait_ms: timing.admission_wait_ms,
            job_id: job.map(|job| job.as_str().to_owned()),
            outputs: None,
            coverage: None,
            failure: Some(StepFailureData::of(failure)),
            state,
            cause: Some(failure),
        }
    }

    /// A step that never started.
    #[must_use]
    pub fn not_started(kind: WorkStepKind) -> Self {
        Self {
            kind: kind.identifier(),
            step_kind: kind,
            status: StepStatus::NotStarted.identifier(),
            elapsed_ms: 0,
            admission_wait_ms: 0,
            job_id: None,
            outputs: None,
            coverage: None,
            failure: None,
            state: StepStatus::NotStarted,
            cause: None,
        }
    }

    /// The step's status.
    #[must_use]
    pub const fn status(&self) -> StepStatus {
        self.state
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
enum OutputsData {
    Ingest {
        generation: u64,
        revision_id: Option<String>,
    },
    Retranscribe {
        revision_id: String,
        generation: u64,
        chunks_reused: u32,
    },
    Candidates {
        visual_index_id: String,
        generation: u64,
        candidate_count: u32,
    },
    Retain {
        bundle_name: String,
        bundle_sha256: String,
        artifact_count: u32,
    },
    Close {
        generation: u64,
    },
}

impl OutputsData {
    fn of(outputs: &StepOutputs) -> Self {
        match outputs {
            StepOutputs::Ingest {
                generation,
                revision_id,
            } => Self::Ingest {
                generation: generation.value(),
                revision_id: revision_id.as_ref().map(|id| id.as_str().to_owned()),
            },
            StepOutputs::Retranscribe {
                revision_id,
                generation,
                chunks_reused,
            } => Self::Retranscribe {
                revision_id: revision_id.as_str().to_owned(),
                generation: generation.value(),
                chunks_reused: *chunks_reused,
            },
            StepOutputs::Candidates {
                visual_index_id,
                generation,
                candidate_count,
                gaps: _,
            } => Self::Candidates {
                visual_index_id: visual_index_id.as_str().to_owned(),
                generation: generation.value(),
                candidate_count: *candidate_count,
            },
            StepOutputs::Retain {
                bundle_name,
                bundle_sha256,
                artifact_count,
            } => Self::Retain {
                bundle_name: bundle_name.as_str().to_owned(),
                bundle_sha256: bundle_sha256.as_str().to_owned(),
                artifact_count: *artifact_count,
            },
            StepOutputs::Close { generation } => Self::Close {
                generation: generation.value(),
            },
        }
    }
}

/// A step's failure as published.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct StepFailureData {
    code: &'static str,
    retryable: bool,
    retry_after_ms: Option<u64>,
}

impl StepFailureData {
    const fn of(failure: WorkFailure) -> Self {
        Self {
            code: failure.code.identifier(),
            retryable: failure.code.retryable(),
            retry_after_ms: failure.retry_after_ms,
        }
    }
}

/// A request's failure as published: the index of the step that ended it,
/// or the rejection that refused it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct RequestFailureData {
    code: &'static str,
    retryable: bool,
    retry_after_ms: Option<u64>,
    step: Option<usize>,
    rejection: Option<&'static str>,
}

impl RequestFailureData {
    const fn of(
        failure: WorkFailure,
        step: Option<usize>,
        rejection: Option<RequestRejection>,
    ) -> Self {
        Self {
            code: failure.code.identifier(),
            retryable: failure.code.retryable(),
            retry_after_ms: failure.retry_after_ms,
            step,
            rejection: match rejection {
                Some(rejection) => Some(rejection.identifier()),
                None => None,
            },
        }
    }
}

/// Why a request failed before its steps, if it did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestFailure {
    /// The request was refused: invalid, a conflict with its workspace or
    /// input root, or a duplicate operation id in a batch.
    Rejected(RequestRejection),
    /// The request was valid but could not start (admission, deadline,
    /// capability, isolation, shutdown).
    NotStarted(WorkFailure),
}

/// Everything a host knows when it presents a request's outcome.
#[derive(Clone, Debug)]
pub struct WorkResultParts<'a> {
    /// The request's operation id.
    pub operation_id: &'a OperationId,
    /// The request's digest.
    pub request_digest: &'a WorkRequestDigest,
    /// Computed now or replayed.
    pub origin: ResultOrigin,
    /// The attempt of this operation id that produced the result (1 for
    /// the first).
    pub attempt: NonZeroU32,
    /// The session the request created or used, once known.
    pub session_id: Option<&'a SessionId>,
    /// Its source, once known.
    pub source_id: Option<&'a SourceId>,
    /// The session's publication guarantee, once known.
    pub publication: Option<PublicationGuarantee>,
    /// The session's lifecycle, once known.
    pub lifecycle: Option<LifecycleResponse>,
    /// The steps, in order: the ingest of an ingest target, then every
    /// requested step.
    pub steps: Vec<StepResult>,
    /// A failure before any step ran.
    pub failure: Option<RequestFailure>,
    /// The controls the request ran under.
    pub controls: WorkControls,
}

/// Why a result could not be presented; only a host defect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkResultError {
    /// More than [`MAX_RESULT_STEPS`] steps.
    TooManySteps,
    /// A step ran after one that failed or was cancelled, or a request that
    /// failed before its steps reports a started step.
    Inconsistent,
}

impl fmt::Display for WorkResultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManySteps => "a job result reports too many steps",
            Self::Inconsistent => "a job result reports steps after a failure",
        })
    }
}

impl Error for WorkResultError {}

/// `job-result` v1.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkResult {
    operation_id: String,
    request_digest: String,
    status: &'static str,
    replayed: bool,
    attempt: u32,
    session_id: Option<String>,
    source_id: Option<String>,
    publication: Option<&'static str>,
    lifecycle: Option<LifecycleResponse>,
    steps: Vec<StepResult>,
    failure: Option<RequestFailureData>,
    controls: ControlsData,
    #[serde(skip)]
    outcome: OperationStatus,
    #[serde(skip)]
    failure_code: Option<FailureCode>,
    #[serde(skip)]
    rejection: Option<RequestRejection>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct ControlsData {
    isolation: &'static str,
    admission_capacity: u16,
    concurrency: u16,
    resource_limits: &'static str,
    free_space_reserve: &'static str,
}

impl WorkResult {
    /// Presents a request's outcome. The status is derived, never supplied:
    /// `cancelled` when the request or a step was cancelled, `failed` when
    /// it or a step failed, `partial` when a step left a gap, else
    /// `complete`. The result's `failure` is the request's own, or the
    /// failure of the step that ended it (with that step's index).
    ///
    /// # Errors
    ///
    /// [`WorkResultError`] for a result no run can produce.
    pub fn new(parts: WorkResultParts<'_>) -> Result<Self, WorkResultError> {
        if parts.steps.len() > MAX_RESULT_STEPS {
            return Err(WorkResultError::TooManySteps);
        }
        let ending = parts
            .steps
            .iter()
            .position(|step| step.state.ended_the_request());
        let started_after_end = |from: usize| {
            parts
                .steps
                .iter()
                .skip(from)
                .any(|step| step.state != StepStatus::NotStarted)
        };
        let consistent = match (parts.failure, ending) {
            (Some(_), _) => !started_after_end(0),
            (None, Some(index)) => !started_after_end(index + 1),
            (None, None) => true,
        };
        if !consistent {
            return Err(WorkResultError::Inconsistent);
        }
        let (failure, step) = match (parts.failure, ending) {
            (Some(RequestFailure::Rejected(rejection)), _) => (
                Some((
                    WorkFailure::new(rejection.failure_code(), None),
                    Some(rejection),
                )),
                None,
            ),
            (Some(RequestFailure::NotStarted(failure)), _) => (Some((failure, None)), None),
            (None, Some(index)) => (
                parts
                    .steps
                    .get(index)
                    .and_then(|step| step.cause)
                    .map(|failure| (failure, None)),
                Some(index),
            ),
            (None, None) => (None, None),
        };
        let failure_code = failure.map(|(failure, _)| failure.code);
        let rejection = failure.and_then(|(_, rejection)| rejection);
        let failure =
            failure.map(|(failure, rejection)| RequestFailureData::of(failure, step, rejection));
        let outcome = match failure_code {
            Some(FailureCode::Cancelled) => OperationStatus::Cancelled,
            Some(_) => OperationStatus::Failed,
            None if parts
                .steps
                .iter()
                .any(|step| step.state == StepStatus::Partial) =>
            {
                OperationStatus::Partial
            }
            None => OperationStatus::Complete,
        };
        Ok(Self {
            operation_id: parts.operation_id.as_str().to_owned(),
            request_digest: parts.request_digest.as_str().to_owned(),
            status: outcome.identifier(),
            replayed: parts.origin == ResultOrigin::Replayed,
            attempt: parts.attempt.get(),
            session_id: parts.session_id.map(|id| id.as_str().to_owned()),
            source_id: parts.source_id.map(|id| id.as_str().to_owned()),
            publication: parts.publication.map(PublicationGuarantee::identifier),
            lifecycle: parts.lifecycle,
            steps: parts.steps,
            failure,
            controls: ControlsData {
                isolation: parts.controls.isolation.identifier(),
                admission_capacity: parts.controls.admission_capacity.get(),
                concurrency: parts.controls.concurrency.get(),
                resource_limits: parts.controls.resource_limits.identifier(),
                free_space_reserve: parts.controls.free_space_reserve.identifier(),
            },
            outcome,
            failure_code,
            rejection,
        })
    }

    /// A request that ran no step because `failure` refused or stopped it
    /// first: its first attempt, no session, no step. Unlike
    /// [`Self::new`] it cannot be inconsistent, so it cannot fail.
    #[must_use]
    pub fn refused(
        operation_id: &OperationId,
        request_digest: &WorkRequestDigest,
        failure: RequestFailure,
        controls: WorkControls,
    ) -> Self {
        let (cause, rejection) = match failure {
            RequestFailure::Rejected(rejection) => (
                WorkFailure::new(rejection.failure_code(), None),
                Some(rejection),
            ),
            RequestFailure::NotStarted(failure) => (failure, None),
        };
        let outcome = if cause.code == FailureCode::Cancelled {
            OperationStatus::Cancelled
        } else {
            OperationStatus::Failed
        };
        Self {
            operation_id: operation_id.as_str().to_owned(),
            request_digest: request_digest.as_str().to_owned(),
            status: outcome.identifier(),
            replayed: false,
            attempt: 1,
            session_id: None,
            source_id: None,
            publication: None,
            lifecycle: None,
            steps: Vec::new(),
            failure: Some(RequestFailureData::of(cause, None, rejection)),
            controls: ControlsData {
                isolation: controls.isolation.identifier(),
                admission_capacity: controls.admission_capacity.get(),
                concurrency: controls.concurrency.get(),
                resource_limits: controls.resource_limits.identifier(),
                free_space_reserve: controls.free_space_reserve.identifier(),
            },
            outcome,
            failure_code: Some(cause.code),
            rejection,
        }
    }

    /// The derived status.
    #[must_use]
    pub const fn status(&self) -> OperationStatus {
        self.outcome
    }

    /// The failure that ended the request, if any.
    #[must_use]
    pub const fn failure_code(&self) -> Option<FailureCode> {
        self.failure_code
    }

    /// The operation id.
    #[must_use]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
}
