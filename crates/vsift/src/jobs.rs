//! Recoverable jobs through the engine (P10, ADR 0020): status, resume and
//! cancellation by job id, and the jobs a session holds.
//!
//! These are the operations the public `job status`, `job resume` and
//! `job cancel` commands and the `jobs` member of `session status` present
//! (P10 PR 3). A job is found through the root's job index, so a caller needs
//! only the job id a retranscription reported.

use vsift_application::{
    CancelOutcome, JobLiveness, JobRecord, JobRequest, JobStore, JobView, Resumability, cancel_job,
    job_status as job_status_query, observed_state, resumable_request,
};
use vsift_domain::{
    AdmissionWait, AttemptFailure, JobId, JobKind, JobState, OperationId, SessionId, SessionPhase,
    StorageGeneration, TimeRange, TranscriptRevisionId,
};
use vsift_infrastructure::FilesystemSessionStore;

use crate::{
    asr::{RetranscribeOutcome, RetranscribeRange, RetranscribeRequest},
    engine::Engine,
    error::EngineError,
    progress::ProgressObserver,
    verification::Cancellation,
};

/// What cancelling a job achieved.
pub type JobCancelOutcome = CancelOutcome;

/// Most jobs `session status` lists; the newest first.
pub const MAX_LISTED_SESSION_JOBS: usize = 16;

/// A job as `job status` reports it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobStatusReport {
    job_id: JobId,
    session_id: SessionId,
    kind: JobKind,
    state: JobState,
    live: bool,
    epoch: u32,
    attempt: u32,
    checkpoints: usize,
    planned_chunks: Option<u32>,
    requested: Option<TimeRange>,
    revision_id: Option<TranscriptRevisionId>,
    committed_generation: Option<StorageGeneration>,
    operation_ids: Vec<OperationId>,
    operation_id: Option<OperationId>,
    resumability: Resumability,
    last_failure: Option<AttemptFailure>,
}

impl JobStatusReport {
    /// Builds the report of a job whose record `view` holds, in a session
    /// that is or is not open. `view` must already say what a reader should
    /// see (reconciled, or its state replaced by [`observed_state`]).
    fn from_view(view: JobView, state: JobState, session_open: bool) -> Self {
        let live = view.liveness == JobLiveness::Owned;
        let resumability = Resumability::of(state, view.liveness, session_open);
        let record = view.record;
        let JobRequest::Retranscribe { range } = record.request;
        let succeeded = record
            .commit
            .as_ref()
            .filter(|_| state == JobState::Succeeded);
        let last_failure = record
            .failures
            .last()
            .copied()
            .filter(|_| matches!(state, JobState::Interrupted | JobState::Failed));
        Self {
            operation_id: reported_operation(&record, state),
            job_id: record.job_id,
            session_id: record.session_id,
            kind: record.request.kind(),
            state,
            live,
            epoch: record.epoch,
            attempt: record.attempt,
            checkpoints: view.checkpoints,
            planned_chunks: record.planned_chunks,
            requested: range,
            revision_id: succeeded.map(|commit| commit.revision_id.clone()),
            // A commit publishes exactly the generation after the one it
            // recorded as expected (the publication is fenced on it).
            committed_generation: succeeded.map(|commit| {
                StorageGeneration::from_value(commit.observed_generation.value().saturating_add(1))
            }),
            operation_ids: record.operation_ids,
            resumability,
            last_failure,
        }
    }

    /// The job.
    #[must_use]
    pub const fn job_id(&self) -> &JobId {
        &self.job_id
    }

    /// The session the job belongs to.
    #[must_use]
    pub const fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// What kind of operation the job runs.
    #[must_use]
    pub const fn kind(&self) -> JobKind {
        self.kind
    }

    /// The job's state; a job whose process ended without recording it is
    /// reported `interrupted`, never `running`.
    #[must_use]
    pub const fn state(&self) -> JobState {
        self.state
    }

    /// Whether a process owns the job right now.
    #[must_use]
    pub const fn live(&self) -> bool {
        self.live
    }

    /// How often the job was restarted after it failed or was cancelled.
    #[must_use]
    pub const fn epoch(&self) -> u32 {
        self.epoch
    }

    /// Attempts started in this epoch.
    #[must_use]
    pub const fn attempt(&self) -> u32 {
        self.attempt
    }

    /// Chunk checkpoints the job holds.
    #[must_use]
    pub const fn checkpoints(&self) -> usize {
        self.checkpoints
    }

    /// How many chunks the recognised range is cut into, when known (jobs
    /// created before P10 PR 3 did not record it).
    #[must_use]
    pub const fn planned_chunks(&self) -> Option<u32> {
        self.planned_chunks
    }

    /// The range the job was asked to retranscribe, or `None` for the whole
    /// source.
    #[must_use]
    pub const fn requested(&self) -> Option<TimeRange> {
        self.requested
    }

    /// The revision a succeeded job committed.
    #[must_use]
    pub const fn revision_id(&self) -> Option<&TranscriptRevisionId> {
        self.revision_id.as_ref()
    }

    /// The session generation a succeeded job committed.
    #[must_use]
    pub const fn committed_generation(&self) -> Option<StorageGeneration> {
        self.committed_generation
    }

    /// Operation ids callers bound to the job.
    #[must_use]
    pub fn operation_ids(&self) -> &[OperationId] {
        &self.operation_ids
    }

    /// The operation id a retry should carry: the first one a caller bound
    /// to the job, or else, for a succeeded job, the one its commit is
    /// recorded under (a retry with it returns the committed result).
    #[must_use]
    pub const fn operation_id(&self) -> Option<&OperationId> {
        self.operation_id.as_ref()
    }

    /// Whether `job resume` can continue the job, and why not.
    #[must_use]
    pub const fn resumability(&self) -> Resumability {
        self.resumability
    }

    /// The failure that ended the last attempt of an interrupted or failed
    /// job, when one was recorded.
    #[must_use]
    pub const fn last_failure(&self) -> Option<AttemptFailure> {
        self.last_failure
    }
}

/// The operation id a caller should retry with, if the job has one.
fn reported_operation(record: &JobRecord, state: JobState) -> Option<OperationId> {
    record.operation_ids.first().cloned().or_else(|| {
        record
            .commit
            .as_ref()
            .filter(|_| state == JobState::Succeeded)
            .map(|commit| commit.operation_id.clone())
    })
}

/// A request to resume an interrupted job.
#[derive(Clone, Debug)]
pub struct JobResumeRequest {
    /// The job to resume.
    pub job: JobId,
    /// Signal that stops the run at its next provider boundary.
    pub cancellation: Cancellation,
    /// Receives the resumed run's chunk progress, as
    /// [`RetranscribeRequest::progress`] does (P11).
    pub progress: ProgressObserver,
    /// How the resumed run waits for admission, as
    /// [`RetranscribeRequest::admission`] does.
    pub admission: AdmissionWait,
}

/// A resumed job's result and the job as it stands afterwards.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobResumeReport {
    outcome: RetranscribeOutcome,
    status: JobStatusReport,
}

impl JobResumeReport {
    /// The retranscription the job committed (or had committed).
    #[must_use]
    pub const fn outcome(&self) -> &RetranscribeOutcome {
        &self.outcome
    }

    /// The job after the run.
    #[must_use]
    pub const fn status(&self) -> &JobStatusReport {
        &self.status
    }
}

/// The result of a cancellation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobCancelReport {
    outcome: JobCancelOutcome,
    status: JobStatusReport,
}

impl JobCancelReport {
    /// The job.
    #[must_use]
    pub const fn job_id(&self) -> &JobId {
        self.status.job_id()
    }

    /// What the cancellation achieved: cancelled now, requested of a live
    /// owner, too late (the job is committing or committed; its result
    /// stands, warning `cancellation_too_late`) or already ended.
    #[must_use]
    pub const fn outcome(&self) -> JobCancelOutcome {
        self.outcome
    }

    /// The job after the cancellation.
    #[must_use]
    pub const fn status(&self) -> &JobStatusReport {
        &self.status
    }

    /// Whether the job had committed, or was committing, so the cancellation
    /// changed nothing.
    #[must_use]
    pub const fn too_late(&self) -> bool {
        matches!(self.outcome, CancelOutcome::TooLate(_))
    }
}

/// One job of a session, as `session status` lists it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionJobEntry {
    job_id: JobId,
    kind: JobKind,
    state: JobState,
    live: bool,
    resumability: Resumability,
}

impl SessionJobEntry {
    /// The job.
    #[must_use]
    pub const fn job_id(&self) -> &JobId {
        &self.job_id
    }

    /// What kind of operation the job runs.
    #[must_use]
    pub const fn kind(&self) -> JobKind {
        self.kind
    }

    /// The state a reader observes, without the listing changing the job.
    #[must_use]
    pub const fn state(&self) -> JobState {
        self.state
    }

    /// Whether a process owns the job right now.
    #[must_use]
    pub const fn live(&self) -> bool {
        self.live
    }

    /// Whether `job resume` can continue the job, and why not.
    #[must_use]
    pub const fn resumability(&self) -> Resumability {
        self.resumability
    }
}

/// The newest jobs of a session.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionJobs {
    entries: Vec<SessionJobEntry>,
    truncated: bool,
}

impl SessionJobs {
    /// At most [`MAX_LISTED_SESSION_JOBS`] jobs, the most recently changed
    /// first.
    #[must_use]
    pub fn entries(&self) -> &[SessionJobEntry] {
        &self.entries
    }

    /// Whether the session holds more jobs than are listed.
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}

impl Engine {
    /// Reports a job's state (the `JobStatusQuery` use case).
    ///
    /// A job whose owner has gone is reconciled with its session's manifest
    /// chain first: a commit that landed makes it `succeeded`, one that did
    /// not makes it `interrupted`.
    ///
    /// # Errors
    ///
    /// [`EngineError::JobNotFound`] for an unknown job, and storage failures.
    pub fn job_status(&self, job: &JobId) -> Result<JobStatusReport, EngineError> {
        let (store, session, now) = self.job_session(job)?;
        let view = job_status_query(&store, &session, job, now)?;
        let open = session_open(&store, &session, now)?;
        let state = view.effective_state();
        Ok(JobStatusReport::from_view(view, state, open))
    }

    /// Resumes an interrupted job (the `ResumeJob` use case) by running its
    /// recorded request again: the request finds the same job through its
    /// keys and continues from its checkpoints, under the operation id the
    /// caller first bound to it, if any.
    ///
    /// A job whose session was renewed or received another revision since
    /// is continued as far as its checkpoints still apply (see
    /// [`Engine::retranscribe`]). Jobs never renew their session: after it
    /// closed or expired, resuming fails before anything runs.
    ///
    /// # Errors
    ///
    /// [`EngineError::JobNotFound`], [`EngineError::JobBusy`] for a live job,
    /// [`EngineError::JobNotResumable`] for a succeeded, failed or cancelled
    /// one, [`EngineError::JobSessionNotOpen`] for a closed or expired
    /// session, and every error of [`Engine::retranscribe`].
    pub async fn job_resume(
        &self,
        request: JobResumeRequest,
    ) -> Result<JobResumeReport, EngineError> {
        let (store, session, now) = self.job_session(&request.job)?;
        let record = resumable_request(&store, &session, &request.job, now)?;
        if !session_open(&store, &session, now)? {
            return Err(EngineError::JobSessionNotOpen {
                session,
                job: request.job,
            });
        }
        drop(store);
        let JobRequest::Retranscribe { range } = record.request;
        let outcome = self
            .retranscribe(RetranscribeRequest {
                session,
                range: range.map(|range| RetranscribeRange {
                    from_micros: range.start().as_micros(),
                    to_micros: range.end().as_micros(),
                }),
                operation_id: record.operation_ids.first().cloned(),
                cancellation: request.cancellation,
                progress: request.progress,
                admission: request.admission,
            })
            .await?;
        let status = self.job_status(outcome.job().job_id())?;
        Ok(JobResumeReport { outcome, status })
    }

    /// Cancels a job (the `CancelJob` use case).
    ///
    /// Cancellation is serialized with the commit under the job's state
    /// lock: one that wins leaves no generation and removes the job's
    /// checkpoints; one that loses reports the committed result; a repeated
    /// cancel is idempotent. A live owner is asked to stop: it notices within
    /// [`vsift_infrastructure::JOB_CANCEL_POLL`], stops its provider and
    /// cancels the job before its commit.
    ///
    /// # Errors
    ///
    /// [`EngineError::JobNotFound`], busy when an owner appeared meanwhile,
    /// and storage failures.
    pub fn job_cancel(&self, job: &JobId) -> Result<JobCancelReport, EngineError> {
        let (store, session, now) = self.job_session(job)?;
        let outcome = cancel_job(&store, &session, job, now)?;
        drop(store);
        let status = self.job_status(job)?;
        Ok(JobCancelReport { outcome, status })
    }

    /// The newest jobs of a session, as `session status` lists them.
    ///
    /// Read-only: a job whose owner has gone is reported as reconciliation
    /// would record it ([`observed_state`]) without taking it over.
    ///
    /// # Errors
    ///
    /// Storage failures, including a damaged job record.
    pub fn session_jobs(&self, session: &SessionId) -> Result<SessionJobs, EngineError> {
        let (store, now) = self.existing_store()?;
        let Some(store) = store else {
            return Ok(SessionJobs::default());
        };
        let open = session_open(&store, session, now)?;
        let mut views = store.session_jobs(session)?;
        views.sort_by(|left, right| {
            right
                .record
                .updated_at_unix_seconds
                .cmp(&left.record.updated_at_unix_seconds)
                .then_with(|| left.record.job_id.cmp(&right.record.job_id))
        });
        let truncated = views.len() > MAX_LISTED_SESSION_JOBS;
        let entries = views
            .into_iter()
            .take(MAX_LISTED_SESSION_JOBS)
            .map(|view| {
                let state = observed_state(&view, &store)?;
                Ok(SessionJobEntry {
                    job_id: view.record.job_id.clone(),
                    kind: view.record.request.kind(),
                    state,
                    live: view.liveness == JobLiveness::Owned,
                    resumability: Resumability::of(state, view.liveness, open),
                })
            })
            .collect::<Result<Vec<_>, EngineError>>()?;
        Ok(SessionJobs { entries, truncated })
    }

    /// The store, the job's session from the root's job index, and the time.
    fn job_session(
        &self,
        job: &JobId,
    ) -> Result<(FilesystemSessionStore, SessionId, u64), EngineError> {
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::JobNotFound)?;
        let session = store.job_session(job)?.ok_or(EngineError::JobNotFound)?;
        Ok((store, session, now))
    }
}

/// Whether `session` is open and unexpired at `now`.
fn session_open(
    store: &FilesystemSessionStore,
    session: &SessionId,
    now: u64,
) -> Result<bool, EngineError> {
    let status = crate::sessions::published_status(store, session)?;
    Ok(status.phase() == SessionPhase::Open && !status.lifetime().expired(now))
}
