//! The P11 event kinds of a JSON Lines stream: `progress`, `lifecycle` and
//! `result` (ADR 0021, superseding ADR 0017 decision D7's "no progress").
//!
//! They share the identity fields of every event (`schema_version`, `event`,
//! a contiguous `sequence`, `command`, `operation_id`) and precede the one
//! terminal event, so a v1 reader that skips unknown kinds still sees a
//! gap-free sequence ending in the terminal event. Every string member is an
//! enum or a bounded identifier; no event carries a path, evidence text or
//! provider output, and every event line stays within
//! [`MAX_EVENT_LINE_BYTES`] (the terminal event keeps the 1 MiB result
//! budget).
//!
//! Progress is advisory and bounded: a host writes at most one progress event
//! per second per request and at most [`MAX_PROGRESS_EVENTS`] per request,
//! and drops progress (counting it in `progress_dropped`) rather than block
//! on a slow reader. Lifecycle, result and terminal events are never dropped.

use std::num::NonZeroU16;

use serde::Serialize;
use vsift_domain::{FailureCode, JobId, OperationId, ProgressUpdate, PublicationGuarantee};

use crate::{
    BatchItemStatus, BatchTermination, CONTRACT_VERSION, CommandName, EventKind, RequestRejection,
    WorkResult, WorkerIsolation,
};

/// Longest line of any event but the terminal one.
pub const MAX_EVENT_LINE_BYTES: usize = 65_536;
/// Most progress events one request writes.
pub const MAX_PROGRESS_EVENTS: u32 = 4_096;
/// Shortest interval between two progress events of one request.
pub const PROGRESS_INTERVAL_MS: u64 = 1_000;

/// One progress event.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProgressEventResponse {
    schema_version: &'static str,
    event: &'static str,
    sequence: u64,
    command: &'static str,
    operation_id: Option<String>,
    request_operation_id: Option<String>,
    job_id: Option<String>,
    stage: &'static str,
    completed: u64,
    total: Option<u64>,
    unit: &'static str,
    progress_dropped: u64,
}

/// What one progress event reports, besides its position in the stream.
#[derive(Clone, Copy, Debug)]
pub struct ProgressReport<'a> {
    /// The observation.
    pub update: ProgressUpdate,
    /// The job doing the work, when it runs as one.
    pub job: Option<&'a JobId>,
    /// The worker request it belongs to (`job run` / `job batch` only).
    pub request: Option<&'a OperationId>,
    /// Progress updates of this request dropped before this event.
    pub dropped: u64,
}

impl ProgressEventResponse {
    /// A progress event at `sequence`.
    #[must_use]
    pub fn new(sequence: u64, command: CommandName, report: &ProgressReport<'_>) -> Self {
        Self {
            schema_version: CONTRACT_VERSION,
            event: EventKind::Progress.identifier(),
            sequence,
            command: command.identifier(),
            operation_id: None,
            request_operation_id: report.request.map(|id| id.as_str().to_owned()),
            job_id: report.job.map(|id| id.as_str().to_owned()),
            stage: report.update.stage.identifier(),
            completed: report.update.completed,
            total: report.update.total,
            unit: report.update.stage.unit().identifier(),
            progress_dropped: report.dropped,
        }
    }
}

/// What a lifecycle event announces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleKind {
    /// The host is ready and states its readiness.
    Started,
    /// A request was admitted and started.
    RequestAdmitted,
    /// A request waits for admission capacity.
    AdmissionWaiting,
    /// A request finished (or was refused).
    RequestFinished,
    /// A shutdown stopped admission; running requests drain or stop.
    Draining,
    /// The host stopped reading and every admitted request ended.
    Stopped,
}

impl LifecycleKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::Started,
        Self::RequestAdmitted,
        Self::AdmissionWaiting,
        Self::RequestFinished,
        Self::Draining,
        Self::Stopped,
    ];

    /// The stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::RequestAdmitted => "request_admitted",
            Self::AdmissionWaiting => "admission_waiting",
            Self::RequestFinished => "request_finished",
            Self::Draining => "draining",
            Self::Stopped => "stopped",
        }
    }
}

/// Why a host drains, stops or makes a request wait.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleReason {
    /// Every line was read.
    EndOfInput,
    /// A shutdown signal arrived.
    Shutdown,
    /// The drain deadline (`--drain-timeout-ms`) passed.
    DrainTimeout,
    /// The batch file had too many lines.
    LineLimit,
    /// The batch file could not be read further.
    InputError,
    /// The admission capacity is in use.
    AdmissionCapacity,
}

impl LifecycleReason {
    /// Every reason, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::EndOfInput,
        Self::Shutdown,
        Self::DrainTimeout,
        Self::LineLimit,
        Self::InputError,
        Self::AdmissionCapacity,
    ];

    /// The stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::EndOfInput => "end_of_input",
            Self::Shutdown => "shutdown",
            Self::DrainTimeout => "drain_timeout",
            Self::LineLimit => "line_limit",
            Self::InputError => "input_error",
            Self::AdmissionCapacity => "admission_capacity",
        }
    }
}

impl From<BatchTermination> for LifecycleReason {
    fn from(termination: BatchTermination) -> Self {
        match termination {
            BatchTermination::EndOfInput => Self::EndOfInput,
            BatchTermination::Shutdown => Self::Shutdown,
            BatchTermination::LineLimit => Self::LineLimit,
            BatchTermination::InputError => Self::InputError,
        }
    }
}

/// What a host states when it starts (O-03): the guarantee its sessions get,
/// its isolation, its admission capacity and its request concurrency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Readiness {
    /// The publication guarantee of the workspace's sessions.
    pub publication: PublicationGuarantee,
    /// The isolation requests run under.
    pub isolation: WorkerIsolation,
    /// The workspace's admission capacity.
    pub admission_capacity: NonZeroU16,
    /// Requests run at once.
    pub concurrency: NonZeroU16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct ReadinessData {
    publication: &'static str,
    isolation: &'static str,
    admission_capacity: u16,
    concurrency: u16,
}

/// Which request a lifecycle event concerns.
#[derive(Clone, Copy, Debug)]
pub struct RequestRef<'a> {
    /// Its 1-based line in a batch file; `None` for `job run`.
    pub line: Option<u32>,
    /// Its operation id, when it had a valid one.
    pub operation_id: Option<&'a OperationId>,
}

/// How a request finished, for its `request_finished` event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestEnd {
    /// The item status.
    pub status: BatchItemStatus,
    /// The failure code, if it did not succeed.
    pub code: Option<FailureCode>,
    /// The rejection, if it was refused.
    pub rejection: Option<RequestRejection>,
    /// Progress updates of the request that were dropped.
    pub progress_dropped: u64,
}

/// One lifecycle event. Members that do not apply to its kind are `null`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LifecycleEventResponse {
    schema_version: &'static str,
    event: &'static str,
    sequence: u64,
    command: &'static str,
    operation_id: Option<String>,
    kind: &'static str,
    line: Option<u32>,
    request_operation_id: Option<String>,
    status: Option<&'static str>,
    code: Option<&'static str>,
    rejection: Option<&'static str>,
    progress_dropped: Option<u64>,
    readiness: Option<ReadinessData>,
    reason: Option<&'static str>,
}

impl LifecycleEventResponse {
    fn bare(sequence: u64, command: CommandName, kind: LifecycleKind) -> Self {
        Self {
            schema_version: CONTRACT_VERSION,
            event: EventKind::Lifecycle.identifier(),
            sequence,
            command: command.identifier(),
            operation_id: None,
            kind: kind.identifier(),
            line: None,
            request_operation_id: None,
            status: None,
            code: None,
            rejection: None,
            progress_dropped: None,
            readiness: None,
            reason: None,
        }
    }

    fn for_request(mut self, request: &RequestRef<'_>) -> Self {
        self.line = request.line;
        self.request_operation_id = request.operation_id.map(|id| id.as_str().to_owned());
        self
    }

    /// The host started, with its readiness.
    #[must_use]
    pub fn started(sequence: u64, command: CommandName, readiness: Readiness) -> Self {
        let mut event = Self::bare(sequence, command, LifecycleKind::Started);
        event.readiness = Some(ReadinessData {
            publication: readiness.publication.identifier(),
            isolation: readiness.isolation.identifier(),
            admission_capacity: readiness.admission_capacity.get(),
            concurrency: readiness.concurrency.get(),
        });
        event
    }

    /// A request was admitted.
    #[must_use]
    pub fn request_admitted(sequence: u64, command: CommandName, request: &RequestRef<'_>) -> Self {
        Self::bare(sequence, command, LifecycleKind::RequestAdmitted).for_request(request)
    }

    /// A request waits for admission capacity.
    #[must_use]
    pub fn admission_waiting(
        sequence: u64,
        command: CommandName,
        request: &RequestRef<'_>,
    ) -> Self {
        let mut event =
            Self::bare(sequence, command, LifecycleKind::AdmissionWaiting).for_request(request);
        event.reason = Some(LifecycleReason::AdmissionCapacity.identifier());
        event
    }

    /// A request finished or was refused.
    #[must_use]
    pub fn request_finished(
        sequence: u64,
        command: CommandName,
        request: &RequestRef<'_>,
        end: RequestEnd,
    ) -> Self {
        let mut event =
            Self::bare(sequence, command, LifecycleKind::RequestFinished).for_request(request);
        event.status = Some(end.status.identifier());
        event.code = end.code.map(FailureCode::identifier);
        event.rejection = end.rejection.map(RequestRejection::identifier);
        event.progress_dropped = Some(end.progress_dropped);
        event
    }

    /// Admission stopped (a shutdown).
    #[must_use]
    pub fn draining(sequence: u64, command: CommandName, reason: LifecycleReason) -> Self {
        let mut event = Self::bare(sequence, command, LifecycleKind::Draining);
        event.reason = Some(reason.identifier());
        event
    }

    /// The host stopped.
    #[must_use]
    pub fn stopped(sequence: u64, command: CommandName, reason: LifecycleReason) -> Self {
        let mut event = Self::bare(sequence, command, LifecycleKind::Stopped);
        event.reason = Some(reason.identifier());
        event
    }
}

/// One request's result in a stream.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResultEventResponse {
    schema_version: &'static str,
    event: &'static str,
    sequence: u64,
    command: &'static str,
    operation_id: Option<String>,
    line: Option<u32>,
    result: WorkResult,
}

impl ResultEventResponse {
    /// The result of the request on `line` (`None` for `job run`).
    #[must_use]
    pub fn new(sequence: u64, command: CommandName, line: Option<u32>, result: WorkResult) -> Self {
        Self {
            schema_version: CONTRACT_VERSION,
            event: EventKind::Result.identifier(),
            sequence,
            command: command.identifier(),
            operation_id: None,
            line,
            result,
        }
    }
}
