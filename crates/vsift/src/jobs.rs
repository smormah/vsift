//! Recoverable jobs through the engine (P10, ADR 0020): status, resume and
//! cancellation by job id, and the jobs a session holds.
//!
//! These are the operations the public `job status`, `job resume` and
//! `job cancel` commands and the `jobs` member of `session status` present
//! (P10 PR 3). A job is found through the root's job index, so a caller needs
//! only the job id a retranscription reported.

use vsift_application::{
    CancelOutcome, JobLiveness, JobProgressLog as _, JobRecord, JobRequest, JobStore, JobView,
    Resumability, StreamPhase, cancel_job, job_status as job_status_query, observed_state,
    resumable_request,
};
use vsift_domain::{
    AdmissionWait, AttemptFailure, JobId, JobKind, JobMode, JobState, MediaProgress, OperationId,
    PrepareStream, ProgressStage, RevisionState, SessionId, SessionPhase, StorageGeneration,
    TimeRange, TranscriptRevisionId,
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
    mode: Option<JobMode>,
    first: Option<TimeRange>,
    streams: Vec<StreamStatus>,
    queue_position: Option<usize>,
}

/// Where one stream of a `prepare` job is (ADR 0033 section 5), as
/// `job status` reports it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamStatus {
    stream: PrepareStream,
    job_id: Option<JobId>,
    state: Option<JobState>,
    stage: Option<ProgressStage>,
    chunks_done: Option<u64>,
    chunks_total: Option<u64>,
    media: Option<MediaProgress>,
    published_until_us: Option<u64>,
    live: bool,
    phase: StreamPhase,
}

impl StreamStatus {
    /// The stream.
    #[must_use]
    pub const fn stream(&self) -> PrepareStream {
        self.stream
    }

    /// The stream's own job, once it has one.
    #[must_use]
    pub const fn job_id(&self) -> Option<&JobId> {
        self.job_id.as_ref()
    }

    /// The state of the stream's job as a reader observes it; `None` before
    /// the stream has a job.
    #[must_use]
    pub const fn state(&self) -> Option<JobState> {
        self.state
    }

    /// The stage the stream last reported.
    #[must_use]
    pub const fn stage(&self) -> Option<ProgressStage> {
        self.stage
    }

    /// Chunks done, reused ones included, as last reported.
    #[must_use]
    pub const fn chunks_done(&self) -> Option<u64> {
        self.chunks_done
    }

    /// Chunks planned, when known.
    #[must_use]
    pub const fn chunks_total(&self) -> Option<u64> {
        self.chunks_total
    }

    /// How far the stream has come in media time, as last reported.
    #[must_use]
    pub const fn media(&self) -> Option<MediaProgress> {
        self.media
    }

    /// Up to where its revision is published (an open revision of the
    /// stream's job), or its whole range once sealed.
    #[must_use]
    pub const fn published_until_us(&self) -> Option<u64> {
        self.published_until_us
    }

    /// Whether a process runs the stream's job right now: only then is an
    /// estimate of the time left meaningful.
    #[must_use]
    pub const fn live(&self) -> bool {
        self.live
    }

    /// Where the runner recorded the stream to be; [`StreamPhase::Skipped`]
    /// for a stream that had nothing to do (a source without video).
    #[must_use]
    pub const fn phase(&self) -> StreamPhase {
        self.phase
    }
}

impl JobStatusReport {
    /// Builds the report of a job whose record `view` holds, in a session
    /// that is or is not open. `view` must already say what a reader should
    /// see (reconciled, or its state replaced by [`observed_state`]).
    fn from_view(view: JobView, state: JobState, session_open: bool) -> Self {
        let live = view.liveness == JobLiveness::Owned;
        let resumability = Resumability::of(state, view.liveness, session_open);
        let record = view.record;
        let range = record.request.range();
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
            mode: record.request.mode(),
            first: record.request.first(),
            streams: Vec::new(),
            queue_position: None,
        }
    }

    /// How a `prepare` job runs (ADR 0033 section 6); `None` for every other
    /// kind.
    #[must_use]
    pub const fn mode(&self) -> Option<JobMode> {
        self.mode
    }

    /// The window a `prepare` job transcribes first, when one was named.
    #[must_use]
    pub const fn first(&self) -> Option<TimeRange> {
        self.first
    }

    /// Where a queued `prepare` job that has not started stands in the root's
    /// queue: 0 for the next to run (ADR 0033 section 6).
    #[must_use]
    pub const fn queue_position(&self) -> Option<usize> {
        self.queue_position
    }

    /// Where each started stream of a `prepare` job is; empty for every
    /// other kind.
    #[must_use]
    pub fn streams(&self) -> &[StreamStatus] {
        &self.streams
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
    /// When the resumed run publishes, as
    /// [`RetranscribeRequest::publication`] says.
    pub publication: crate::RetranscribePublication,
}

/// A resumed job's result and the job as it stands afterwards.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobResumeReport {
    outcome: RetranscribeOutcome,
    status: JobStatusReport,
}

impl JobResumeReport {
    pub(crate) const fn new(outcome: RetranscribeOutcome, status: JobStatusReport) -> Self {
        Self { outcome, status }
    }

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
    seal: StoppedSeal,
}

/// What a cancellation did with the revision its job published as it went
/// (ADR 0033, D1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StoppedSeal {
    /// Nothing to seal here: the job published no open revision, or a live
    /// owner seals its own when it sees the request.
    NotHere,
    /// The open revision is sealed as stopped.
    Sealed,
    /// The stopped seal did not land (a busy session, for example), so the
    /// revision is still open; cancelling again seals it.
    LeftOpen,
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

    /// Whether the job is cancelled but the revision it published as it went
    /// is still open, because its stopped seal did not land (RV-1 finding 7).
    /// Until it is sealed, every other transcription and retention of the
    /// session is refused ([`EngineError::RevisionLeftOpen`]); cancelling the
    /// job again seals it.
    #[must_use]
    pub const fn revision_left_open(&self) -> bool {
        matches!(self.seal, StoppedSeal::LeftOpen)
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
        let kind = view.record.request.kind();
        let mut report = JobStatusReport::from_view(view, state, open);
        if kind == JobKind::Prepare {
            let prepare = (report.state(), report.live());
            report.streams = stream_statuses(&store, &session, job, prepare, now)?;
            // A queued job that has not started says where it stands (ADR 0033
            // section 6).
            if report.mode == Some(JobMode::Queued)
                && matches!(report.state, JobState::Queued | JobState::Interrupted)
            {
                report.queue_position = crate::prepare::queue_position(&store, job);
            }
        }
        Ok(report)
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
        if let JobRequest::Prepare { mode, .. } = record.request {
            // Boxed: the prepare path nests a whole retranscription, and an
            // unboxed future that size overflows a host's main-thread stack.
            return Box::pin(self.resume_prepare(
                store,
                session,
                request.job.clone(),
                mode,
                request,
            ))
            .await;
        }
        drop(store);
        let outcome = self
            .retranscribe(resumed_retranscription(session, &record, request))
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
        // A `prepare` job's streams are jobs of their own: each is cancelled
        // with it (best effort; its runner cancels a stream it sees stop).
        if let Ok(Some(view)) = store.job(&session, job)
            && view.record.request.kind() == JobKind::Prepare
            && let Ok(Some(progress)) = store.read_progress(&session, job)
        {
            for stream in progress
                .streams
                .iter()
                .filter_map(|stream| stream.job.as_ref())
            {
                if stream != job {
                    let _ = self.job_cancel(stream);
                }
            }
        }
        let outcome = cancel_job(&store, &session, job, now)?;
        // A cancelled queued job leaves the root's queue.
        if matches!(
            outcome,
            CancelOutcome::Cancelled | CancelOutcome::AlreadyEnded(JobState::Cancelled)
        ) {
            store.dequeue_job(job);
        }
        // A job no process runs is cancelled here, so its open revision, if it
        // published one, is sealed as stopped here too (ADR 0033, D1); a live
        // owner seals its own when it sees the request. A session busy now
        // keeps the revision open: the report says so, and cancelling again
        // seals it.
        let seal = if matches!(
            outcome,
            CancelOutcome::Cancelled | CancelOutcome::AlreadyEnded(JobState::Cancelled)
        ) {
            let view = job_status_query(&store, &session, job, now)?;
            match vsift_application::seal_stopped_now(
                &store,
                &session,
                job,
                view.record.epoch,
                view.record.attempt,
                now,
            ) {
                Ok(None) => StoppedSeal::NotHere,
                Ok(Some(_)) => StoppedSeal::Sealed,
                Err(_) => StoppedSeal::LeftOpen,
            }
        } else {
            StoppedSeal::NotHere
        };
        drop(store);
        let status = self.job_status(job)?;
        Ok(JobCancelReport {
            outcome,
            status,
            seal,
        })
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

    /// #370: when an evidence or `candidates` call of `session` met
    /// contention, names the transcription of the same session that is
    /// running, if one is: its recognition holds its recognizer threads of
    /// the root's capacity for the whole run, so the call cannot be admitted
    /// until it ends, and a blind retry would only become a reported gap.
    ///
    /// Every other error, and contention while no job of the session is
    /// live (another session's work, or a lock held for a moment), is
    /// returned as it was. Looking the job up is best effort: a job listing
    /// that cannot be read leaves the original answer, which is already the
    /// published `BUSY`.
    pub(crate) fn name_running_transcription(
        &self,
        session: &SessionId,
        error: EngineError,
    ) -> EngineError {
        if !error.is_media_contention() {
            return error;
        }
        let Ok((Some(store), _)) = self.existing_store() else {
            return error;
        };
        let Ok(views) = store.session_jobs(session) else {
            return error;
        };
        views
            .into_iter()
            .filter(|view| view.liveness == JobLiveness::Owned)
            .max_by_key(|view| view.record.updated_at_unix_seconds)
            .map_or(error, |view| EngineError::TranscriptionRunning {
                session: session.clone(),
                job: view.record.job_id,
            })
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

/// Where each started stream of the `prepare` job `job` is: its progress
/// record for the figures, the stream's own job for its state, and the
/// session's open revision for what is published.
fn stream_statuses(
    store: &FilesystemSessionStore,
    session: &SessionId,
    job: &JobId,
    (prepare_state, prepare_live): (JobState, bool),
    now: u64,
) -> Result<Vec<StreamStatus>, EngineError> {
    let Some(progress) = store.read_progress(session, job)? else {
        return Ok(Vec::new());
    };
    let open = store.read_open_transcript(session, now).ok().flatten();
    let mut streams = Vec::with_capacity(progress.streams.len());
    for stream in progress.streams {
        let view = match &stream.job {
            Some(id) => store.job(session, id)?,
            None => None,
        };
        let job_state = view
            .as_ref()
            .map(|view| observed_state(view, store))
            .transpose()?;
        // A stream with no job of its own (the visual stream) is in the state
        // its runner recorded, while the `prepare` job runs, and in the
        // `prepare` job's otherwise.
        let (state, live) = match (job_state, stream.phase) {
            (Some(state), _) => (
                Some(state),
                view.as_ref()
                    .is_some_and(|view| view.liveness == JobLiveness::Owned),
            ),
            (None, StreamPhase::Done) => (Some(JobState::Succeeded), false),
            (None, StreamPhase::Running) if prepare_live => (Some(JobState::Running), true),
            // A stream that reported nothing before its runner stopped never
            // got going: it is shown as not started.
            (None, StreamPhase::Running) if stream.update.is_some() => (Some(prepare_state), false),
            // Skipped, or a stream that never got going.
            (None, StreamPhase::Skipped | StreamPhase::Running) => (None, false),
        };
        let media = stream.update.and_then(|update| update.media);
        let published_until_us = match (&stream.job, state) {
            (_, Some(JobState::Succeeded)) => media.and_then(|media| media.to_us),
            // The visual stream commits every batch it analysed.
            (None, _) if stream.stream == PrepareStream::Visual => {
                media.map(|media| media.done_until_us)
            }
            (Some(id), _) => open
                .as_ref()
                .filter(|open| open.job_id == *id && open.revision.state() == RevisionState::Open)
                .and_then(|open| open.revision.published_until())
                .map(vsift_domain::MediaTime::as_micros),
            (None, _) => None,
        };
        streams.push(StreamStatus {
            stream: stream.stream,
            job_id: stream.job,
            state,
            stage: stream.update.map(|update| update.stage),
            chunks_done: stream.update.map(|update| update.completed),
            chunks_total: stream.update.and_then(|update| update.total),
            media,
            published_until_us,
            live,
            phase: stream.phase,
        });
    }
    Ok(streams)
}

/// Whether `session` is open and unexpired at `now`.
pub(crate) fn session_open(
    store: &FilesystemSessionStore,
    session: &SessionId,
    now: u64,
) -> Result<bool, EngineError> {
    let status = crate::sessions::published_status(store, session)?;
    Ok(status.phase() == SessionPhase::Open && !status.lifetime().expired(now))
}

/// The refusal for a write to `session` while its open revision, published by
/// `job`, is open (ADR 0033): the job runs, or can still be resumed, so the
/// caller follows it ([`EngineError::TranscriptionRunning`]); or the job has
/// ended without sealing it, so cancelling it again seals it
/// ([`EngineError::RevisionLeftOpen`], RV-1 finding 7). A job whose record
/// cannot be read is reported as running, which names it all the same.
pub(crate) fn open_revision_refusal(
    store: &FilesystemSessionStore,
    session: &SessionId,
    job: JobId,
    now: u64,
) -> EngineError {
    let ended = job_status_query(store, session, &job, now)
        .ok()
        .and_then(|view| observed_state(&view, store).ok())
        .is_some_and(JobState::is_terminal);
    if ended {
        EngineError::RevisionLeftOpen {
            session: session.clone(),
            job,
        }
    } else {
        EngineError::TranscriptionRunning {
            session: session.clone(),
            job,
        }
    }
}

/// The retranscription a resume of `record` runs: its recorded range and
/// share, under the operation id the caller first bound to it.
///
/// The share is the one the job was recorded with: a watched run's child
/// takes one unit less, and the recognizer's threads are part of the job's
/// key, so a resume with any other share would compute another job, answer
/// `BUSY` naming this one or start afresh beside it (RV-1 finding 2).
fn resumed_retranscription(
    session: SessionId,
    record: &JobRecord,
    request: JobResumeRequest,
) -> RetranscribeRequest {
    RetranscribeRequest {
        session,
        range: record.request.range().map(|range| RetranscribeRange {
            from_micros: range.start().as_micros(),
            to_micros: range.end().as_micros(),
        }),
        operation_id: record.operation_ids.first().cloned(),
        cancellation: request.cancellation,
        progress: request.progress,
        admission: request.admission,
        publication: request.publication,
        share: record.request.share().unwrap_or_default(),
        seed: None,
    }
}

#[cfg(test)]
mod tests {
    use vsift_application::{
        JobRecord, JobRequest, JobSpec, RequestDigest, job_id, retranscribe_operation_key,
    };
    use vsift_domain::{AdmissionWait, RecognitionKey, RecognitionShare, SessionId, Sha256Hex};

    use super::{JobResumeRequest, resumed_retranscription};
    use crate::{ProgressObserver, RetranscribePublication, verification::Cancellation};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn record(share: RecognitionShare) -> Result<JobRecord, Box<dyn std::error::Error>> {
        let session = SessionId::parse("ses_0123456789abcdef0123456789abcdef")?;
        let key = RecognitionKey::new(Sha256Hex::parse(DIGEST)?);
        let operation_key = retranscribe_operation_key(&key, None)?;
        Ok(JobRecord::queued(
            &JobSpec {
                job_id: job_id(&session, &operation_key)?,
                session_id: session,
                request_digest: RequestDigest::new(Sha256Hex::parse(DIGEST)?),
                operation_key,
                recognition_key: key,
                request: JobRequest::Retranscribe { range: None, share },
                planned_chunks: None,
            },
            1_000,
        ))
    }

    /// RV-1 finding 2: `job resume` of a watched run's child (a job recorded
    /// with one unit less of the root) recognises with that share again, so
    /// it computes the same job; an ordinary job still takes the whole.
    #[test]
    fn a_resume_recognises_with_the_share_its_job_was_recorded_with() -> TestResult {
        for share in RecognitionShare::ALL {
            let record = record(share)?;
            let resumed = resumed_retranscription(
                record.session_id.clone(),
                &record,
                JobResumeRequest {
                    job: record.job_id.clone(),
                    cancellation: Cancellation::new(),
                    progress: ProgressObserver::none(),
                    admission: AdmissionWait::Immediate,
                    publication: RetranscribePublication::default(),
                },
            );
            assert_eq!(resumed.share, share);
            assert_eq!(resumed.range, None);
            assert_eq!(resumed.session, record.session_id);
        }
        Ok(())
    }
}
