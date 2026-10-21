//! `job start`: the `prepare` job (ADR 0033 sections 5 and 6).
//!
//! A `prepare` job is the preparation of one session, run step by step, in
//! the foreground (quick) or by a runner in the background (watched). It runs
//! each of its streams as a job of its own: the transcript is an ordinary
//! whole-source retranscribe job, so its checkpoints, its resume and its
//! commit do not change. The `prepare` job holds no checkpoints and commits
//! nothing itself: it records the commit its last stream made, so the
//! manifest chain stays the only authority on what was published and a
//! `prepare` job whose runner died while committing is reconciled like any
//! other ([`super::reconcile`]).
//!
//! **Identity.** A `prepare` job derives from the session, the mode and the
//! sealed revision it starts from. `job start` asked again while the job runs
//! finds the same job; once its transcript is sealed, the base has changed and
//! a new request is new work.
//!
//! **Progress.** The runner records where each stream is, at most once a
//! second, in a small record of the job ([`JobProgressLog`]); `job status`
//! reads it. It is advisory, like every progress report: the job record and
//! the manifest chain decide what happened.

use std::future::Future;

use vsift_domain::{
    AttemptFailure, JobId, JobMode, JobState, OperationKey, PrepareStream, ProgressUpdate,
    RecognitionKey, SessionId, TranscriptRevisionId,
};

use super::{
    CommitLedger, JobChange, JobCommit, JobKeyError, JobOwner, JobRequest, JobSpec, JobStore,
    JobStoreError, RequestDigest, digest_lines, job_id, reconcile,
};
use crate::{Clock, SessionStorageError};

/// Command name bound into a `prepare` job's request digest.
const PREPARE_COMMAND: &str = "job.start";

/// The request digest of `job start` (`vsift.prepare-request.v1`): the
/// session and the mode.
///
/// # Errors
///
/// [`JobKeyError::NotCanonical`] only through an internal fault.
pub fn prepare_request_digest(
    session_id: &SessionId,
    mode: JobMode,
) -> Result<RequestDigest, JobKeyError> {
    digest_lines(
        "vsift.prepare-request.v1",
        &[
            session_id.as_str(),
            PREPARE_COMMAND,
            &format!("mode={}", mode.identifier()),
        ],
    )
    .map(RequestDigest)
}

/// The operation key of a `prepare` job (`vsift.prepare-op.v1`): its request
/// and the sealed revision it starts from.
///
/// # Errors
///
/// [`JobKeyError::NotCanonical`] only through an internal fault.
pub fn prepare_operation_key(
    request: &RequestDigest,
    base: Option<&TranscriptRevisionId>,
) -> Result<OperationKey, JobKeyError> {
    let digest = digest_lines(
        "vsift.prepare-op.v1",
        &[
            request.as_str(),
            base.map_or("none", TranscriptRevisionId::as_str),
        ],
    )?;
    OperationKey::from_sha256(digest.as_str()).map_err(|_| JobKeyError::NotCanonical)
}

/// Everything that identifies the `prepare` job of `session_id` in `mode`,
/// starting from the sealed revision `base`.
///
/// The record's recognition key derives from the operation key
/// (`vsift.prepare-recognition.v1`): a `prepare` job recognises nothing
/// itself and holds no checkpoints, so the key only has to be its own.
///
/// # Errors
///
/// [`JobKeyError::NotCanonical`] only through an internal fault.
pub fn prepare_spec(
    session_id: &SessionId,
    mode: JobMode,
    base: Option<&TranscriptRevisionId>,
) -> Result<JobSpec, JobKeyError> {
    let request_digest = prepare_request_digest(session_id, mode)?;
    let operation_key = prepare_operation_key(&request_digest, base)?;
    let recognition_key = RecognitionKey::new(digest_lines(
        "vsift.prepare-recognition.v1",
        &[operation_key.as_str()],
    )?);
    Ok(JobSpec {
        session_id: session_id.clone(),
        job_id: job_id(session_id, &operation_key)?,
        request_digest,
        operation_key,
        recognition_key,
        request: JobRequest::Prepare { mode },
        planned_chunks: None,
    })
}

/// Where one stream of a `prepare` job is, as its runner last recorded it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamProgress {
    /// The stream.
    pub stream: PrepareStream,
    /// The stream's own job, once it is known.
    pub job: Option<JobId>,
    /// The stream's last progress update, if it reported one.
    pub update: Option<ProgressUpdate>,
}

/// Where a `prepare` job's streams are, as its runner last recorded it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareProgress {
    /// One entry a stream that has started, in the order they run.
    pub streams: Vec<StreamProgress>,
    /// When the runner recorded it, in Unix seconds.
    pub recorded_at_unix_seconds: u64,
}

/// Port that keeps the progress record of a `prepare` job: written by its
/// runner, read by `job status`.
pub trait JobProgressLog: Send + Sync {
    /// Replaces the job's progress record, best effort: a record that could
    /// not be written leaves the previous one.
    ///
    /// # Errors
    ///
    /// Storage failures, which the runner ignores.
    fn record_progress(
        &self,
        session_id: &SessionId,
        job_id: &JobId,
        progress: &PrepareProgress,
    ) -> Result<(), SessionStorageError>;

    /// The job's progress record, if its runner wrote one.
    ///
    /// # Errors
    ///
    /// Storage failures; a damaged record is read as none.
    fn read_progress(
        &self,
        session_id: &SessionId,
        job_id: &JobId,
    ) -> Result<Option<PrepareProgress>, SessionStorageError>;
}

/// How one stream's run ended without a commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamStop {
    /// It stopped and its job can be resumed (a Ctrl-C, a crash of a tool,
    /// a busy root): the `prepare` job is interrupted too.
    Interrupted {
        /// The stream's job, when it had one.
        job: Option<JobId>,
        /// What stopped it, when it was a failure.
        failure: Option<AttemptFailure>,
    },
    /// Its job was cancelled.
    Cancelled,
    /// It cannot continue; a resume would only repeat it.
    Failed {
        /// What ended it.
        failure: Option<AttemptFailure>,
    },
}

/// The streams a `prepare` job runs, supplied by the engine.
pub trait PrepareStreams: Send + Sync {
    /// Runs `stream` to its end: the commit its job made, or how it stopped.
    fn run(
        &self,
        stream: PrepareStream,
    ) -> impl Future<Output = Result<JobCommit, StreamStop>> + Send;

    /// Cancels a stream's job that stopped while the `prepare` job was asked
    /// to stop, so that what it published is sealed as stopped (ADR 0033,
    /// D1) rather than left open. Best effort.
    fn cancel(&self, job: &JobId) -> impl Future<Output = ()> + Send;
}

/// How running a `prepare` job ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrepareOutcome {
    /// Every stream committed; the job records the last commit.
    Succeeded(JobCommit),
    /// The job had already succeeded: its commit answers again.
    AlreadySucceeded(JobCommit),
    /// A stream stopped, and the job with it.
    Stopped(StreamStop),
}

/// Why a `prepare` job could not run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrepareError {
    /// Another process runs the job.
    Busy {
        /// The job.
        job: JobId,
    },
    /// The job ended as failed or cancelled; `job start` starts it again.
    NotResumable {
        /// The job.
        job: JobId,
        /// Its state.
        state: JobState,
    },
    /// The job store failed.
    Job(JobStoreError),
}

impl From<JobStoreError> for PrepareError {
    fn from(error: JobStoreError) -> Self {
        Self::Job(error)
    }
}

impl std::fmt::Display for PrepareError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy { .. } => formatter.write_str("another process runs the job"),
            Self::NotResumable { state, .. } => {
                write!(formatter, "the job is {}", state.identifier())
            }
            Self::Job(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PrepareError {}

/// Runs (or resumes) the `prepare` job `job_id` of `session_id`: takes it
/// over, reconciles it with the manifest chain, then runs its streams in
/// order and records how they ended.
///
/// The caller watches the job's cancellation request while it runs and stops
/// the streams when it is made; a stream that stops while the request stands
/// is cancelled, never left interrupted, so a cancelled `prepare` job leaves
/// no open revision behind.
///
/// # Errors
///
/// [`PrepareError::Busy`] for a job another process runs,
/// [`PrepareError::NotResumable`] for one that failed or was cancelled, and
/// store failures.
pub async fn run_prepare<S, P>(
    store: &S,
    session_id: &SessionId,
    job: &JobId,
    clock: &dyn Clock,
    streams: &P,
) -> Result<PrepareOutcome, PrepareError>
where
    S: JobStore + CommitLedger,
    P: PrepareStreams,
{
    let started = clock.now_unix_seconds().unwrap_or_default();
    let now = || clock.now_unix_seconds().unwrap_or(started);
    let mut owner = match store.acquire(session_id, job) {
        Ok(owner) => owner,
        Err(JobStoreError::Storage(SessionStorageError::Busy)) => {
            return Err(PrepareError::Busy { job: job.clone() });
        }
        Err(error) => return Err(error.into()),
    };
    reconcile(&mut owner, store, started)?;
    match owner.record().state {
        JobState::Succeeded => {
            let commit = owner.record().commit.clone().ok_or(JobStoreError::Storage(
                SessionStorageError::IntegrityFailure,
            ))?;
            return Ok(PrepareOutcome::AlreadySucceeded(commit));
        }
        state @ (JobState::Failed | JobState::Cancelled) => {
            return Err(PrepareError::NotResumable {
                job: job.clone(),
                state,
            });
        }
        JobState::Queued | JobState::Interrupted => owner.apply(&JobChange::Start, started)?,
        // Reconciliation moved every owner-claiming state on.
        JobState::Running | JobState::Committing | JobState::Cancelling => {
            return Err(PrepareError::Job(JobStoreError::Storage(
                SessionStorageError::StateConflict,
            )));
        }
    }
    let mut last = None;
    for stream in PrepareStream::ALL {
        match streams.run(stream).await {
            Ok(commit) => last = Some(commit),
            Err(stop) => return settle_stop(&mut owner, streams, stop, now()).await,
        }
    }
    let commit = last.ok_or(JobStoreError::Storage(SessionStorageError::StateConflict))?;
    if owner.cancel_requested() {
        // The streams committed before the request was seen: their result
        // stands, and the job, asked to stop, ends cancelled.
        owner.apply(&JobChange::Cancel, now())?;
        return Ok(PrepareOutcome::Stopped(StreamStop::Cancelled));
    }
    owner.apply(&JobChange::Commit(commit.clone()), now())?;
    owner.apply(&JobChange::Succeed, now())?;
    Ok(PrepareOutcome::Succeeded(commit))
}

/// Records how a stream stopped on the `prepare` job.
async fn settle_stop<O, P>(
    owner: &mut O,
    streams: &P,
    stop: StreamStop,
    now: u64,
) -> Result<PrepareOutcome, PrepareError>
where
    O: JobOwner,
    P: PrepareStreams,
{
    let asked_to_stop = owner.cancel_requested();
    let stop = match stop {
        StreamStop::Interrupted { job, .. } if asked_to_stop => {
            if let Some(job) = &job {
                streams.cancel(job).await;
            }
            StreamStop::Cancelled
        }
        other => other,
    };
    match &stop {
        StreamStop::Interrupted { failure, .. } => {
            owner.apply(&JobChange::Interrupt(*failure), now)?;
        }
        StreamStop::Cancelled => {
            if !asked_to_stop {
                owner.apply(&JobChange::RequestCancel, now)?;
            }
            owner.apply(&JobChange::Cancel, now)?;
        }
        StreamStop::Failed { failure } => owner.apply(&JobChange::Fail(*failure), now)?,
    }
    Ok(PrepareOutcome::Stopped(stop))
}
