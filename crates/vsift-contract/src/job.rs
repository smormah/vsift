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
    AttemptFailure, JobId, JobKind, JobMode, JobState, MediaProgress, OperationId, PrepareStream,
    ProgressStage, SessionId, StorageGeneration, TimeRange, TranscriptRevisionId,
};

use crate::{TranscriptRetranscribeData, session::StatusData, transcript::RangeData};

/// What an estimate of the time left is based on (ADR 0033, D8): the media
/// still to do over the cumulative measured speed of the chunks recognised
/// in this attempt.
pub const ESTIMATE_BASIS: &str = "cumulative_measured_speed";

/// The state of a stream whose job has not been created yet.
pub const STREAM_NOT_STARTED: &str = "not_started";

/// A speed in media seconds a wall second, held in thousandths so that the
/// data keeps exact equality, and written as a JSON number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Speed(u64);

impl Serialize for Speed {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Thousandths of a ratio, at most a few thousand: exact in an f64.
        #[allow(
            clippy::cast_precision_loss,
            reason = "a speed in thousandths is far below 2^52 and converts exactly"
        )]
        let speed = self.0 as f64 / 1000.0;
        serializer.serialize_f64(speed)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct EstimateData {
    remaining_ms: u64,
    basis: &'static str,
    fresh_chunks: u32,
}

/// How far a recognition has come in media time (ADR 0033 section 5), as a
/// progress event and a stream of `job status` show it.
///
/// `done_us` and `total_us` are media microseconds (`total_us` null when the
/// end is not known); `elapsed_ms` is wall time in this attempt; `speed` is
/// media seconds a wall second over the chunks recognised in this attempt
/// (null before the first); `percent` is whole percent done (null without an
/// end); `estimate` is the time left, null until two fresh chunks and 20 s
/// of work, and absent from a stream no process runs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MediaProgressData {
    done_us: u64,
    total_us: Option<u64>,
    elapsed_ms: u64,
    speed: Option<Speed>,
    percent: Option<u8>,
    estimate: Option<EstimateData>,
}

impl MediaProgressData {
    /// Presents `media`, with its estimate when `estimating` (a live run).
    #[must_use]
    pub fn new(media: &MediaProgress, estimating: bool) -> Self {
        Self {
            done_us: media.done_us(),
            total_us: media.total_us(),
            elapsed_ms: media.elapsed_us / 1000,
            speed: media.speed_millis().map(Speed),
            percent: media.percent(),
            estimate: media
                .estimate_remaining_us()
                .filter(|_| estimating)
                .map(|remaining| EstimateData {
                    remaining_ms: remaining / 1000,
                    basis: ESTIMATE_BASIS,
                    fresh_chunks: media.fresh_chunks,
                }),
        }
    }
}

/// One stream of a `prepare` job as the host read it, in plain values.
#[derive(Clone, Copy, Debug)]
pub struct StreamPresentation<'a> {
    /// The stream.
    pub stream: PrepareStream,
    /// Its own job, once it has one.
    pub job_id: Option<&'a JobId>,
    /// That job's state as a reader observes it.
    pub state: Option<JobState>,
    /// The stage it last reported.
    pub stage: Option<ProgressStage>,
    /// Chunks done, as last reported.
    pub chunks_done: Option<u64>,
    /// Chunks planned, when known.
    pub chunks_total: Option<u64>,
    /// Up to where its revision is published.
    pub published_until_us: Option<u64>,
    /// How far it has come in media time, as last reported.
    pub media: Option<MediaProgress>,
    /// Whether a process runs it now, so that an estimate means something.
    pub live: bool,
}

/// One stream in a `prepare` job's data (ADR 0033 section 5).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct StreamData {
    stream: &'static str,
    job_id: Option<String>,
    state: &'static str,
    stage: Option<&'static str>,
    chunks_done: Option<u64>,
    chunks_total: Option<u64>,
    published_until_us: Option<u64>,
    media: Option<MediaProgressData>,
}

impl StreamData {
    fn new(stream: &StreamPresentation<'_>) -> Self {
        Self {
            stream: stream.stream.identifier(),
            job_id: stream.job_id.map(|id| id.as_str().to_owned()),
            state: stream
                .state
                .map_or(STREAM_NOT_STARTED, JobState::identifier),
            stage: stream.stage.map(ProgressStage::identifier),
            chunks_done: stream.chunks_done,
            chunks_total: stream.chunks_total,
            published_until_us: stream.published_until_us,
            media: stream
                .media
                .map(|media| MediaProgressData::new(&media, stream.live)),
        }
    }
}

/// Remediation when a run was cancelled by its caller (Ctrl-C or
/// `SIGTERM`) before it committed: the job is interrupted and resumable.
pub const JOB_INTERRUPTED_REMEDIATION: &str = "The run was cancelled before it committed, so nothing was committed; affected_ids names its session and job. The job is interrupted and keeps the chunks it finished: resume it with job resume <job>, or run the same command again.";

/// Remediation when a job's session is closed or expired.
///
/// It must not advise `session renew`: a renewal only extends a session that
/// is still open (the domain's `SessionLifetime::renew`), so the only way on is
/// a new session.
pub const JOB_SESSION_NOT_OPEN_REMEDIATION: &str = "The job's session is closed or expired, so the job cannot continue; affected_ids names the session and job. Nothing was run or changed. A closed or expired session cannot be renewed or reopened: open a new session with ingest and run the request there.";

/// Remediation when `job start` could not hand its job to a background
/// runner (ADR 0033).
pub const RUNNER_NOT_STARTED_REMEDIATION: &str = "The background runner did not take the job, so nothing ran; affected_ids names the job, which waits to be run. Run it in the foreground with job resume <job>, which answers when it is done; job status from another terminal reports how far it has come.";

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
    /// How a `prepare` job runs; `None` for every other kind.
    pub mode: Option<JobMode>,
    /// Where a `prepare` job's started streams are; empty otherwise.
    pub streams: &'a [StreamPresentation<'a>],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct JobRequestData {
    range: Option<RangeData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<&'static str>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    streams: Option<Vec<StreamData>>,
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
                mode: job.mode.map(JobMode::identifier),
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
            // A `prepare` job always lists its streams, none before its
            // runner starts the first (ADR 0033).
            streams: (job.kind == JobKind::Prepare)
                .then(|| job.streams.iter().map(StreamData::new).collect()),
        }
    }
}

/// How `job start` runs the job it recorded (ADR 0033 section 5).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobRunner {
    /// It ran in this process, and the call answered when it ended (quick).
    Foreground,
    /// A runner was started in the background and took the job (watched);
    /// the identifier says how far it is detached from its caller.
    Background(&'static str),
    /// Nothing was started: a runner runs it already, or it is done.
    None,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct JobStartedData {
    state: &'static str,
    mode: &'static str,
    mode_basis: &'static str,
    runner: &'static str,
    detachment: Option<&'static str>,
}

/// Data of a `job start` result (ADR 0033): the `prepare` job, what the start
/// did, and, for a job that ran in the foreground, the retranscription it
/// committed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JobStartData {
    job: JobData,
    start: JobStartedData,
    outcome: Option<TranscriptRetranscribeData>,
}

impl JobStartData {
    /// Presents a started job. `state`, `mode` and `basis` are the stable
    /// identifiers the engine gave.
    #[must_use]
    pub fn new(
        job: JobData,
        state: &'static str,
        mode: JobMode,
        basis: &'static str,
        runner: JobRunner,
        outcome: Option<TranscriptRetranscribeData>,
    ) -> Self {
        let (runner, detachment) = match runner {
            JobRunner::Foreground => ("foreground", None),
            JobRunner::Background(detachment) => ("background", Some(detachment)),
            JobRunner::None => ("none", None),
        };
        Self {
            job,
            start: JobStartedData {
                state,
                mode: mode.identifier(),
                mode_basis: basis,
                runner,
                detachment,
            },
            outcome,
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
