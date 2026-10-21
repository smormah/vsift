//! `job start`: preparing a session as one job, in the foreground or in the
//! background (ADR 0033 sections 5 and 6).
//!
//! [`Engine::job_start`] records the session's `prepare` job and chooses its
//! mode; the host then runs it, in its own process for the quick mode or by a
//! runner it starts in the background for the watched mode, through
//! [`Engine::job_resume`]. The `prepare` job runs its transcript as an
//! ordinary whole-source retranscription, publishing as it goes and leaving
//! one admission unit free in the watched mode, and its runner records where
//! the stream is, at most once a second, for `job status`.

use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use vsift_application::{
    JobCommit, JobProgressLog, JobStore, PrepareError, PrepareOutcome, PrepareProgress,
    PrepareStreams, StreamProgress, StreamStop, prepare_spec, run_prepare,
};
use vsift_domain::{
    AdmissionWait, AttemptFailure, JobId, JobMode, JobState, PrepareStream, RevisionState,
    SessionId,
};
use vsift_infrastructure::FilesystemSessionStore;

use crate::{
    Cancellation, JobResumeReport, JobStatusReport, ProgressObserver, RetranscribeOutcome,
    RetranscribePublication, RetranscribeRequest,
    engine::Engine,
    error::{EngineError, SessionRootError},
    jobs::session_open,
    progress::JobProgress,
};

/// How often a runner records where its streams are (ADR 0033 section 5).
pub const PROGRESS_RECORD_INTERVAL: Duration = Duration::from_secs(1);

/// The share of the root's admission capacity a recognition takes (ADR 0033,
/// D4).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RecognitionShare {
    /// All of it, as before 0.3.0: interactive and quick runs.
    #[default]
    Whole,
    /// One unit less, when that leaves at least one, so that evidence calls
    /// are admitted while it runs: a watched (background) run. The recognizer
    /// runs with fewer threads, which its provenance records.
    LeavingOneFree,
}

/// A request to start preparing a session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobStartRequest {
    /// The session to prepare.
    pub session: SessionId,
    /// The mode asked for; `None` chooses it by the recording's length.
    pub mode: Option<JobMode>,
}

/// Why a `prepare` job runs in its mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModeBasis {
    /// The caller asked for it.
    Requested,
    /// The recording is shorter than [`JobMode::QUICK_LIMIT_MICROS`].
    ShortRecording,
    /// The recording is at least that long.
    LongRecording,
    /// The session holds no transcript yet, so its length is not known: a
    /// long recording must never keep its caller waiting, so it is watched.
    LengthUnknown,
}

impl ModeBasis {
    /// Stable lowercase identifier used in contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::ShortRecording => "short_recording",
            Self::LongRecording => "long_recording",
            Self::LengthUnknown => "length_unknown",
        }
    }
}

/// What `job start` found or made.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobStartState {
    /// A new job, queued for its runner.
    Created,
    /// The same request's job existed, interrupted or never started: it is
    /// queued again for a runner that continues it.
    Waiting,
    /// The job had failed or was cancelled: it starts again from nothing.
    Restarted,
    /// A runner runs the job already; nothing was started.
    Running,
    /// The job already succeeded; nothing was started.
    Done,
}

impl JobStartState {
    /// Whether the job needs a runner now.
    #[must_use]
    pub const fn needs_runner(self) -> bool {
        matches!(self, Self::Created | Self::Waiting | Self::Restarted)
    }

    /// Stable lowercase identifier used in contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Waiting => "waiting",
            Self::Restarted => "restarted",
            Self::Running => "running",
            Self::Done => "done",
        }
    }
}

/// The `prepare` job `job start` recorded, and how it is to run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobStartPlan {
    status: JobStatusReport,
    mode: JobMode,
    basis: ModeBasis,
    start: JobStartState,
}

impl JobStartPlan {
    /// The job as it stands.
    #[must_use]
    pub const fn status(&self) -> &JobStatusReport {
        &self.status
    }

    /// The job.
    #[must_use]
    pub const fn job_id(&self) -> &JobId {
        self.status.job_id()
    }

    /// How it runs.
    #[must_use]
    pub const fn mode(&self) -> JobMode {
        self.mode
    }

    /// Why it runs in that mode.
    #[must_use]
    pub const fn basis(&self) -> ModeBasis {
        self.basis
    }

    /// What `job start` found or made.
    #[must_use]
    pub const fn start(&self) -> JobStartState {
        self.start
    }
}

impl Engine {
    /// Records the `prepare` job of a session and chooses its mode (the
    /// `StartJob` use case, ADR 0033): asked for, or quick for a recording
    /// shorter than ten minutes and watched otherwise. Nothing runs here: the
    /// host runs the job through [`Engine::job_resume`], in its own process
    /// for the quick mode and by a background runner for the watched mode,
    /// when [`JobStartState::needs_runner`] says so.
    ///
    /// The job derives from the session, the mode and the sealed revision it
    /// starts from, so asking again while it runs finds it.
    ///
    /// # Errors
    ///
    /// A missing session root or session, [`EngineError::JobSessionNotOpen`]
    /// for a closed or expired session, and storage failures.
    pub fn job_start(&self, request: &JobStartRequest) -> Result<JobStartPlan, EngineError> {
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        let session = &request.session;
        let (newest, _) = store.read_transcript_head(session, now)?;
        // An open revision is not a base: the revision it supersedes is, as for
        // a retranscription. Its source segment gives the recording's length.
        let base = match &newest {
            Some(open) if open.state() == RevisionState::Open => open.supersedes().cloned(),
            Some(newest) => Some(newest.id().clone()),
            None => None,
        };
        let length = newest
            .as_ref()
            .map(|revision| revision.source_segment().range().end().as_micros());
        let (mode, basis) = match (request.mode, length) {
            (Some(mode), _) => (mode, ModeBasis::Requested),
            (None, Some(length)) => match JobMode::for_duration(Some(length)) {
                JobMode::Quick => (JobMode::Quick, ModeBasis::ShortRecording),
                JobMode::Watched => (JobMode::Watched, ModeBasis::LongRecording),
            },
            (None, None) => (JobMode::for_duration(None), ModeBasis::LengthUnknown),
        };
        // The newest revision is the commit of a `prepare` job that succeeded:
        // the session is prepared, and asking again answers with that job
        // instead of transcribing it all again.
        if let Some(newest) = newest
            .as_ref()
            .filter(|newest| newest.state() != RevisionState::Open)
            && let Some(done) = store.session_jobs(session)?.into_iter().find(|view| {
                view.record.request.kind() == vsift_domain::JobKind::Prepare
                    && view.record.state == JobState::Succeeded
                    && view
                        .record
                        .commit
                        .as_ref()
                        .is_some_and(|commit| commit.revision_id == *newest.id())
            })
        {
            let mode = done.record.request.mode().unwrap_or(mode);
            drop(store);
            return Ok(JobStartPlan {
                status: self.job_status(&done.record.job_id)?,
                mode,
                basis,
                start: JobStartState::Done,
            });
        }
        let spec =
            prepare_spec(session, mode, base.as_ref()).map_err(|_| EngineError::JobInvariant)?;
        if !session_open(&store, session, now)? {
            return Err(EngineError::JobSessionNotOpen {
                session: session.clone(),
                job: spec.job_id,
            });
        }
        let start = match store.open_or_create(&spec, now) {
            Ok((mut owner, created)) => {
                use vsift_application::JobOwner as _;
                match owner.record().state {
                    _ if created => JobStartState::Created,
                    JobState::Failed | JobState::Cancelled => {
                        owner.apply(&vsift_application::JobChange::Restart, now)?;
                        JobStartState::Restarted
                    }
                    JobState::Succeeded => JobStartState::Done,
                    _ => JobStartState::Waiting,
                }
            }
            Err(vsift_application::JobStoreError::Storage(
                vsift_application::SessionStorageError::Busy,
            )) => JobStartState::Running,
            Err(error) => return Err(error.into()),
        };
        drop(store);
        Ok(JobStartPlan {
            status: self.job_status(&spec.job_id)?,
            mode,
            basis,
            start,
        })
    }

    /// Runs a `prepare` job resumed by [`Engine::job_resume`].
    pub(crate) async fn resume_prepare(
        &self,
        store: FilesystemSessionStore,
        session: SessionId,
        job: JobId,
        mode: JobMode,
        request: crate::JobResumeRequest,
    ) -> Result<JobResumeReport, EngineError> {
        let streams = EngineStreams {
            engine: self,
            store: &store,
            session: session.clone(),
            prepare: job.clone(),
            mode,
            cancellation: request.cancellation,
            progress: request.progress,
            admission: request.admission,
            recorded: Arc::new(Mutex::new(Vec::new())),
            outcome: Mutex::new(None),
            error: Mutex::new(None),
        };
        let ran = run_prepare(&store, &session, &job, self.clock(), &streams).await;
        match ran {
            Ok(PrepareOutcome::Succeeded(_)) => {
                let outcome = streams
                    .outcome
                    .into_inner()
                    .unwrap_or_else(PoisonError::into_inner)
                    .ok_or(EngineError::JobInvariant)?;
                let status = self.job_status(&job)?;
                Ok(JobResumeReport::new(outcome, status))
            }
            Ok(PrepareOutcome::AlreadySucceeded(_)) => Err(EngineError::JobNotResumable {
                job,
                state: JobState::Succeeded,
            }),
            Ok(PrepareOutcome::Stopped(stop)) => Err(streams
                .error
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner)
                .unwrap_or(match stop {
                    StreamStop::Cancelled => EngineError::JobCancelled { job },
                    StreamStop::Interrupted { .. } => EngineError::JobInterrupted { session, job },
                    StreamStop::Failed { .. } => EngineError::JobInvariant,
                })),
            Err(PrepareError::Busy { job }) => Err(EngineError::JobBusy { job }),
            Err(PrepareError::NotResumable { job, state }) => {
                Err(EngineError::JobNotResumable { job, state })
            }
            Err(PrepareError::Job(error)) => Err(error.into()),
        }
    }
}

/// The streams of one `prepare` job, run through the engine.
struct EngineStreams<'e> {
    engine: &'e Engine,
    store: &'e FilesystemSessionStore,
    session: SessionId,
    prepare: JobId,
    mode: JobMode,
    cancellation: Cancellation,
    progress: ProgressObserver,
    admission: AdmissionWait,
    /// Where each started stream is, as the progress record holds it.
    recorded: Arc<Mutex<Vec<StreamProgress>>>,
    /// The transcript's retranscription, once it committed.
    outcome: Mutex<Option<RetranscribeOutcome>>,
    /// The error that stopped a stream, answered for the whole job.
    error: Mutex<Option<EngineError>>,
}

impl EngineStreams<'_> {
    /// Records where the streams are, best effort: progress is advisory.
    fn record(&self) {
        let streams = self
            .recorded
            .lock()
            .map(|streams| streams.clone())
            .unwrap_or_default();
        let progress = PrepareProgress {
            streams,
            recorded_at_unix_seconds: self.engine.now_unix_seconds().unwrap_or_default(),
        };
        let _ = self
            .store
            .record_progress(&self.session, &self.prepare, &progress);
    }

    /// The transcript stream: a whole-source retranscription, watched for the
    /// `prepare` job's own cancellation, while its progress is recorded.
    async fn transcript(&self) -> Result<RetranscribeOutcome, EngineError> {
        if let Ok(mut streams) = self.recorded.lock() {
            streams.retain(|stream| stream.stream != PrepareStream::Transcript);
            streams.push(StreamProgress {
                stream: PrepareStream::Transcript,
                job: None,
                update: None,
            });
        }
        self.record();
        let recorded = Arc::clone(&self.recorded);
        let host = self.progress.clone();
        // The callback only keeps the latest update (it must not block); the
        // loop below writes it out.
        let progress = ProgressObserver::new(move |seen: &JobProgress| {
            host.report(seen);
            if let Ok(mut streams) = recorded.lock()
                && let Some(stream) = streams
                    .iter_mut()
                    .find(|stream| stream.stream == PrepareStream::Transcript)
            {
                stream.job.clone_from(&seen.job);
                stream.update = Some(seen.update);
            }
        });
        let watched = self.mode == JobMode::Watched;
        let run = Box::pin(self.engine.retranscribe(RetranscribeRequest {
            session: self.session.clone(),
            range: None,
            operation_id: None,
            cancellation: self.cancellation.clone(),
            progress,
            admission: self.admission,
            publication: if watched {
                RetranscribePublication::AsItGoes {
                    interval: crate::PUBLICATION_INTERVAL,
                }
            } else {
                RetranscribePublication::AtTheSeal
            },
            share: if watched {
                RecognitionShare::LeavingOneFree
            } else {
                RecognitionShare::Whole
            },
        }));
        // `job cancel` of the `prepare` job stops its stream within the poll
        // interval, as it does a retranscription's.
        let run = self.store.watch_for_job_cancel(
            &self.session,
            &self.prepare,
            &self.cancellation.0,
            run,
        );
        let mut run = std::pin::pin!(run);
        let mut tick = tokio::time::interval(PROGRESS_RECORD_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let result = loop {
            tokio::select! {
                biased;
                result = &mut run => break result,
                _ = tick.tick() => self.record(),
            }
        };
        self.record();
        result
    }

    /// The transcript stream's job, once it reported one.
    fn transcript_job(&self) -> Option<JobId> {
        self.recorded.lock().ok().and_then(|streams| {
            streams
                .iter()
                .find(|stream| stream.stream == PrepareStream::Transcript)
                .and_then(|stream| stream.job.clone())
        })
    }

    /// How a stream that failed with `error` stopped: as its job records it,
    /// when it had one.
    fn stop_of(&self, error: &EngineError) -> StreamStop {
        let failure = AttemptFailure {
            chunk: None,
            code: error.failure_code(),
        };
        match error {
            EngineError::JobInterrupted { job, .. } => StreamStop::Interrupted {
                job: Some(job.clone()),
                failure: None,
            },
            EngineError::JobCancelled { .. } => StreamStop::Cancelled,
            _ => {
                let job = self.transcript_job();
                let state = job.as_ref().and_then(|job| {
                    self.store
                        .job(&self.session, job)
                        .ok()
                        .flatten()
                        .map(|view| view.record.state)
                });
                match state {
                    Some(JobState::Interrupted | JobState::Queued) => StreamStop::Interrupted {
                        job,
                        failure: Some(failure),
                    },
                    Some(JobState::Cancelled) => StreamStop::Cancelled,
                    // Failed for good, or before the stream had a job (a
                    // missing tool, an unusable source): `job start` again
                    // starts it afresh.
                    _ => StreamStop::Failed {
                        failure: Some(failure),
                    },
                }
            }
        }
    }
}

impl PrepareStreams for EngineStreams<'_> {
    async fn run(&self, stream: PrepareStream) -> Result<JobCommit, StreamStop> {
        match stream {
            PrepareStream::Transcript => match self.transcript().await {
                Ok(outcome) => {
                    let commit = self
                        .store
                        .job(&self.session, outcome.job().job_id())
                        .ok()
                        .flatten()
                        .and_then(|view| view.record.commit);
                    if let Ok(mut slot) = self.outcome.lock() {
                        *slot = Some(outcome);
                    }
                    commit.ok_or(StreamStop::Failed { failure: None })
                }
                Err(error) => {
                    let stop = self.stop_of(&error);
                    if let Ok(mut slot) = self.error.lock() {
                        *slot = Some(error);
                    }
                    Err(stop)
                }
            },
        }
    }

    async fn cancel(&self, job: &JobId) {
        let _ = self.engine.job_cancel(job);
    }
}
