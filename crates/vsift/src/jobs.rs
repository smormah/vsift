//! Recoverable jobs through the engine (P10, ADR 0020): status, resume and
//! cancellation by job id.
//!
//! These are the operations the public `job status`, `job resume` and
//! `job cancel` commands present (P10 PR 3); until then they are library
//! APIs. A job is found through the root's job index, so a caller needs only
//! the job id a retranscription reported.

use vsift_application::{
    CancelOutcome, JobRequest, JobView, cancel_job, job_status as job_status_query,
    resumable_request,
};
use vsift_domain::{
    JobId, JobKind, JobState, OperationId, SessionId, TimeRange, TranscriptRevisionId,
};
use vsift_infrastructure::FilesystemSessionStore;

use crate::{
    asr::{RetranscribeOutcome, RetranscribeRange, RetranscribeRequest},
    engine::Engine,
    error::EngineError,
    verification::Cancellation,
};

/// What cancelling a job achieved.
pub type JobCancelOutcome = CancelOutcome;

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
    requested: Option<TimeRange>,
    revision_id: Option<TranscriptRevisionId>,
    operation_ids: Vec<OperationId>,
}

impl JobStatusReport {
    fn from_view(view: JobView) -> Self {
        let live = view.liveness == vsift_application::JobLiveness::Owned;
        let state = view.effective_state();
        let record = view.record;
        let JobRequest::Retranscribe { range } = record.request;
        Self {
            job_id: record.job_id,
            session_id: record.session_id,
            kind: record.request.kind(),
            state,
            live,
            epoch: record.epoch,
            attempt: record.attempt,
            checkpoints: view.checkpoints,
            requested: range,
            revision_id: record
                .commit
                .filter(|_| state == JobState::Succeeded)
                .map(|commit| commit.revision_id),
            operation_ids: record.operation_ids,
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

    /// Operation ids callers bound to the job.
    #[must_use]
    pub fn operation_ids(&self) -> &[OperationId] {
        &self.operation_ids
    }
}

/// A request to resume an interrupted job.
#[derive(Clone, Debug)]
pub struct JobResumeRequest {
    /// The job to resume.
    pub job: JobId,
    /// Signal that stops the run at its next provider boundary.
    pub cancellation: Cancellation,
}

/// The result of a cancellation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobCancelReport {
    job_id: JobId,
    outcome: JobCancelOutcome,
}

impl JobCancelReport {
    /// The job.
    #[must_use]
    pub const fn job_id(&self) -> &JobId {
        &self.job_id
    }

    /// What the cancellation achieved: cancelled now, requested of a live
    /// owner, too late (the job is committing or committed; its result
    /// stands, warning `cancellation_too_late`) or already ended.
    #[must_use]
    pub const fn outcome(&self) -> JobCancelOutcome {
        self.outcome
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
        Ok(JobStatusReport::from_view(view))
    }

    /// Resumes an interrupted job (the `ResumeJob` use case) by running its
    /// recorded request again: the request finds the same job through its
    /// keys and continues from its checkpoints, under the operation id the
    /// caller first bound to it, if any.
    ///
    /// A job whose session was renewed or received another revision since
    /// is continued as far as its checkpoints still apply (see
    /// [`Engine::retranscribe`]). Jobs never renew their session: after it
    /// closed or expired, resuming fails like any request to it.
    ///
    /// # Errors
    ///
    /// [`EngineError::JobNotFound`], [`EngineError::JobBusy`] for a live job,
    /// [`EngineError::JobNotResumable`] for a succeeded, failed or cancelled
    /// one, and every error of [`Engine::retranscribe`].
    pub async fn job_resume(
        &self,
        request: JobResumeRequest,
    ) -> Result<RetranscribeOutcome, EngineError> {
        let (store, session, now) = self.job_session(&request.job)?;
        let record = resumable_request(&store, &session, &request.job, now)?;
        drop(store);
        let JobRequest::Retranscribe { range } = record.request;
        self.retranscribe(RetranscribeRequest {
            session,
            range: range.map(|range| RetranscribeRange {
                from_micros: range.start().as_micros(),
                to_micros: range.end().as_micros(),
            }),
            operation_id: record.operation_ids.first().cloned(),
            cancellation: request.cancellation,
        })
        .await
    }

    /// Cancels a job (the `CancelJob` use case).
    ///
    /// Cancellation is serialized with the commit under the job's state
    /// lock: one that wins leaves no generation and removes the job's
    /// checkpoints; one that loses reports the committed result; a repeated
    /// cancel is idempotent. A live owner is asked to stop and cancels at its
    /// next chunk or before its commit.
    ///
    /// # Errors
    ///
    /// [`EngineError::JobNotFound`], busy when an owner appeared meanwhile,
    /// and storage failures.
    pub fn job_cancel(&self, job: &JobId) -> Result<JobCancelReport, EngineError> {
        let (store, session, now) = self.job_session(job)?;
        let outcome = cancel_job(&store, &session, job, now)?;
        Ok(JobCancelReport {
            job_id: job.clone(),
            outcome,
        })
    }

    /// The store, the job's session from the root's job index, and the time.
    fn job_session(
        &self,
        job: &JobId,
    ) -> Result<(FilesystemSessionStore, SessionId, u64), EngineError> {
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::JobNotFound)?;
        let session = vsift_application::JobStore::job_session(&store, job)?
            .ok_or(EngineError::JobNotFound)?;
        Ok((store, session, now))
    }
}
