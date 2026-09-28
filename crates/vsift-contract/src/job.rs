//! Data carried by the public job commands (`job status`, `job resume`,
//! `job cancel`) and by the `jobs` member of `session status` (P10 PR 3,
//! ADR 0020 section 6).
//!
//! A job is described by identifiers, states, counts and codes only: never a
//! path, never transcript text and never provider output, so a job listing
//! is safe to hand to any agent. The host reads the job through the engine
//! and passes plain values in a [`JobPresentation`]; this crate owns every
//! field name and identifier.

use serde::Serialize;
use vsift_application::Resumability;
use vsift_domain::{
    AttemptFailure, JobId, JobKind, JobState, OperationId, SessionId, StorageGeneration, TimeRange,
    TranscriptRevisionId,
};

use crate::{TranscriptRetranscribeData, session::StatusData, transcript::RangeData};

/// Remediation when a run was cancelled by its caller (Ctrl-C or
/// `SIGTERM`) before it committed: the job is interrupted and resumable.
pub const JOB_INTERRUPTED_REMEDIATION: &str = "The run was cancelled before it committed, so nothing was committed; affected_ids names its session and job. The job is interrupted and keeps the chunks it finished: resume it with job resume <job>, or run the same command again.";

/// Remediation when a job's session is closed or expired.
///
/// It must not advise `session renew`: a renewal only extends a session that
/// is still open (the domain's `SessionLifetime::renew`), so the only way on is
/// a new session.
pub const JOB_SESSION_NOT_OPEN_REMEDIATION: &str = "The job's session is closed or expired, so the job cannot continue; affected_ids names the session and job. Nothing was run or changed. A closed or expired session cannot be renewed or reopened: open a new session with ingest and run the request there.";

/// Remediation when no session of the root holds the job.
pub const UNKNOWN_JOB_REMEDIATION: &str = "No session of this session root holds a job with that identity. Use a job_id returned by transcript retranscribe or session status for a session that still exists; jobs are removed with their session.";

/// Remediation when a job cannot be resumed because it already ended.
pub const JOB_NOT_RESUMABLE_REMEDIATION: &str = "The job has ended (succeeded, failed or cancelled), so job resume has nothing to continue; affected_ids names it. Read its result with job status, or run the original command again to start it afresh.";

/// Remediation when a job was cancelled through `job cancel` while it ran.
pub const JOB_CANCELLED_REMEDIATION: &str = "The job was cancelled with job cancel before it committed; affected_ids names it. Nothing was committed and its checkpoints were removed. Run the original command again to start it afresh.";

/// A job as the host read it, in plain values.
#[derive(Clone, Copy, Debug)]
pub struct JobPresentation<'a> {
    /// The job.
    pub job_id: &'a JobId,
    /// The session it belongs to.
    pub session_id: &'a SessionId,
    /// What it runs.
    pub kind: JobKind,
    /// Its state as a reader observes it.
    pub state: JobState,
    /// Whether a process owns it right now.
    pub live_owner: bool,
    /// Whether and why `job resume` can continue it.
    pub resumability: Resumability,
    /// The operation id a retry should carry, if any.
    pub operation_id: Option<&'a OperationId>,
    /// The range it was asked for (`None`: the whole source).
    pub requested: Option<TimeRange>,
    /// Chunks its range is cut into, when recorded.
    pub planned_chunks: Option<u32>,
    /// Checkpoint files it keeps now.
    pub checkpoints: usize,
    /// Attempts started in its current epoch.
    pub attempts: u32,
    /// The revision and generation it committed, when it succeeded.
    pub result: Option<(&'a TranscriptRevisionId, StorageGeneration)>,
    /// What ended its last attempt, when it is interrupted or failed.
    pub failure: Option<AttemptFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct JobRequestData {
    range: Option<RangeData>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct JobProgressData {
    chunks_total: Option<u32>,
    chunks_checkpointed: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct JobResultData {
    revision_id: String,
    generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct JobFailureData {
    code: &'static str,
    retryable: bool,
}

/// Data of a `job status` or `job cancel` result: one job.
///
/// `resumable` and `resumable_reason` say whether `job resume` would run it
/// now and why (`interrupted`, `not_started`) or why not (`live_owner`,
/// `succeeded`, `failed`, `cancelled`, `session_not_open`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JobData {
    job_id: String,
    session_id: String,
    kind: &'static str,
    state: &'static str,
    live_owner: bool,
    resumable: bool,
    resumable_reason: &'static str,
    operation_id: Option<String>,
    request: JobRequestData,
    progress: JobProgressData,
    attempts: u32,
    result: Option<JobResultData>,
    failure: Option<JobFailureData>,
}

impl JobData {
    /// Presents one job.
    #[must_use]
    pub fn new(job: &JobPresentation<'_>) -> Self {
        Self {
            job_id: job.job_id.as_str().to_owned(),
            session_id: job.session_id.as_str().to_owned(),
            kind: job.kind.identifier(),
            state: job.state.identifier(),
            live_owner: job.live_owner,
            resumable: job.resumability.resumable(),
            resumable_reason: job.resumability.identifier(),
            operation_id: job.operation_id.map(|id| id.as_str().to_owned()),
            request: JobRequestData {
                range: job.requested.map(RangeData::new),
            },
            progress: JobProgressData {
                chunks_total: job.planned_chunks,
                chunks_checkpointed: job.checkpoints,
            },
            attempts: job.attempts,
            result: job.result.map(|(revision_id, generation)| JobResultData {
                revision_id: revision_id.as_str().to_owned(),
                generation: generation.value(),
            }),
            failure: job.failure.map(|failure| JobFailureData {
                code: failure.code.identifier(),
                retryable: failure.code.retryable(),
            }),
        }
    }
}

/// Data of a `job resume` result: the job after the run and the
/// retranscription it committed (or had committed, when replayed).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JobResumeData {
    job: JobData,
    outcome: TranscriptRetranscribeData,
}

impl JobResumeData {
    /// Presents a resumed job and its result.
    #[must_use]
    pub const fn new(job: JobData, outcome: TranscriptRetranscribeData) -> Self {
        Self { job, outcome }
    }
}

/// One job in `session status`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SessionJobData {
    job_id: String,
    kind: &'static str,
    state: &'static str,
    live_owner: bool,
    resumable: bool,
    resumable_reason: &'static str,
}

impl SessionJobData {
    /// Presents one listed job.
    #[must_use]
    pub fn new(
        job_id: &JobId,
        kind: JobKind,
        state: JobState,
        live_owner: bool,
        resumability: Resumability,
    ) -> Self {
        Self {
            job_id: job_id.as_str().to_owned(),
            kind: kind.identifier(),
            state: state.identifier(),
            live_owner,
            resumable: resumability.resumable(),
            resumable_reason: resumability.identifier(),
        }
    }
}

/// Data of a `session status` result: the session's committed status and,
/// since P10 PR 3, its newest jobs (at most 16; `jobs_truncated` says
/// whether it holds more). Additive: every [`StatusData`] member is
/// unchanged.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SessionStatusData {
    #[serde(flatten)]
    status: StatusData,
    jobs: Vec<SessionJobData>,
    jobs_truncated: bool,
}

impl SessionStatusData {
    /// Adds the listed jobs to a session's status.
    #[must_use]
    pub const fn new(status: StatusData, jobs: Vec<SessionJobData>, jobs_truncated: bool) -> Self {
        Self {
            status,
            jobs,
            jobs_truncated,
        }
    }

    /// The session's committed status.
    #[must_use]
    pub const fn status(&self) -> &StatusData {
        &self.status
    }
}
