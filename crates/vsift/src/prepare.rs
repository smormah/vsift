//! `job start`: preparing a session as one job, in the foreground or in the
//! background (ADR 0033 sections 5 and 6).
//!
//! [`Engine::job_start`] records the session's `prepare` job and chooses its
//! mode; the host then runs it, in its own process for the quick mode or by a
//! runner it starts in the background for the watched mode, through
//! [`Engine::job_resume`]. The `prepare` job runs, in order:
//!
//! 1. **the first window** (`--first`, D5), when one was named: put on the
//!    whole run's chunk grid and retranscribed as a range, sealed at once, its
//!    chunks' answers kept in a [`RecognitionSeed`];
//! 2. **the visual stream** (D6): the source's visual-candidate index, in
//!    batches of [`VISUAL_BATCH_WINDOWS`] windows, each committed as an index
//!    revision, so `candidates` is a warm read once it is done; a source
//!    without video skips it;
//! 3. **the transcript**: a whole-source retranscription, publishing as it
//!    goes and leaving one admission unit free in the watched mode, taking the
//!    first window's answers from the seed.
//!
//! Its runner records where each stream is, at most once a second, for
//! `job status`.

use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use std::num::NonZeroUsize;

use vsift_application::{
    JobProgressLog, JobStore, PrepareError, PrepareOutcome, PrepareProgress, PrepareStreams,
    Stopwatch as _, StreamEnd, StreamPhase, StreamProgress, StreamStop, VisualExtensionStop,
    prepare_spec, run_prepare,
};
use vsift_domain::{
    ADMISSION_RETRY_AFTER, AdmissionWait, AttemptFailure, ChunkPlan, JobId, JobMode, JobState,
    MAX_FIRST_WINDOW_MICROS, MediaProgress, PrepareStream, ProgressStage, ProgressUpdate,
    RevisionState, SessionId, TimeRange, VISUAL_WINDOW_MICROS, align_to_chunk_grid,
};
use vsift_infrastructure::{FilesystemSessionStore, MonotonicStopwatch};

use crate::{
    Cancellation, JobResumeReport, JobStatusReport, ProgressObserver, RecognitionSeed,
    RetranscribeOutcome, RetranscribePublication, RetranscribeRange, RetranscribeRequest,
    engine::Engine,
    error::{EngineError, SessionRootError},
    jobs::session_open,
    progress::JobProgress,
};

/// How often a runner records where its streams are (ADR 0033 section 5).
pub const PROGRESS_RECORD_INTERVAL: Duration = Duration::from_secs(1);

/// Windows one batch of the visual stream analyses and publishes (D6): ten
/// minutes of media, so a four-hour source takes 24 of the 64 index records.
pub const VISUAL_BATCH_WINDOWS: NonZeroUsize = match NonZeroUsize::new(10) {
    Some(windows) => windows,
    None => NonZeroUsize::MIN,
};

/// How often the visual stream asks again when the root is busy before it
/// stops, interrupted: about a minute at [`ADMISSION_RETRY_AFTER`].
const VISUAL_BUSY_RETRIES: u32 = 30;

/// The share of the root's admission capacity a recognition takes (ADR 0033,
/// D4); the domain owns it because a job record keeps it.
pub use vsift_domain::RecognitionShare;

/// A request to start preparing a session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobStartRequest {
    /// The session to prepare.
    pub session: SessionId,
    /// The mode asked for; `None` chooses it by the recording's length.
    pub mode: Option<JobMode>,
    /// The window to transcribe first (`--first`, D5), at most
    /// [`MAX_FIRST_WINDOW_MICROS`] long; `None` for none.
    pub first: Option<RetranscribeRange>,
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
        // A first window is checked before anything is read.
        if let Some(first) = request.first
            && (first.to_micros <= first.from_micros
                || first.to_micros - first.from_micros > MAX_FIRST_WINDOW_MICROS)
        {
            return Err(EngineError::InvalidTimeRange);
        }
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
            (None, Some(length)) if JobMode::for_duration(Some(length)) == JobMode::Quick => {
                (JobMode::Quick, ModeBasis::ShortRecording)
            }
            (None, Some(_)) => (JobMode::Watched, ModeBasis::LongRecording),
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
        let first = request
            .first
            .map(|first| {
                TimeRange::new(
                    vsift_domain::MediaTime::from_micros(first.from_micros),
                    vsift_domain::MediaTime::from_micros(first.to_micros),
                )
                .ok()
                .filter(|first| first.duration_micros() <= MAX_FIRST_WINDOW_MICROS)
                .ok_or(EngineError::InvalidTimeRange)
            })
            .transpose()?;
        let spec = prepare_spec(session, mode, first, base.as_ref())
            .map_err(|_| EngineError::JobInvariant)?;
        if !session_open(&store, session, now)? {
            return Err(EngineError::JobSessionNotOpen {
                session: session.clone(),
                job: spec.job_id,
            });
        }
        let start = match store.open_or_create(&spec, now) {
            Ok((mut owner, created)) => {
                use vsift_application::JobOwner as _;
                // This call holds the job now, so an owner its record still
                // claims is gone: reconcile it first. A job whose dead owner
                // never finished its cancellation is then cancelled and starts
                // again, instead of being handed as waiting to a runner that
                // refuses it as not resumable (RV-1 finding 3).
                if !created {
                    vsift_application::reconcile(&mut owner, &store, now)?;
                }
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
        // A queued job takes its place at the end of the root's queue (ADR
        // 0033 section 6); one already in it keeps its place.
        if mode == JobMode::Queued && start.needs_runner() {
            store.enqueue_job(session, &spec.job_id, now)?;
        }
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
            seed: RecognitionSeed::new(),
            turn: Mutex::new(None),
            outcome: Mutex::new(None),
            error: Mutex::new(None),
        };
        let ran = Box::pin(run_prepare(&store, &session, &job, self.clock(), &streams)).await;
        // A queued job that ended leaves the queue, and gives the turn to the
        // next one; an interrupted or waiting one keeps its place.
        drop(streams.turn.lock().map(|mut turn| turn.take()));
        if mode == JobMode::Queued
            && let Ok(Some(view)) = store.job(&session, &job)
            && view.record.state.is_terminal()
        {
            store.dequeue_job(&job);
        }
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
    /// The first window's chunk answers, for the whole transcript (D5).
    seed: RecognitionSeed,
    /// The root's queue turn, held while a queued job runs.
    turn: Mutex<Option<vsift_infrastructure::QueueTurn>>,
    /// The whole transcript's retranscription, once it committed.
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

    /// Changes the recorded entry of `stream`, adding it first if it has none.
    fn update(&self, stream: PrepareStream, change: impl FnOnce(&mut StreamProgress)) {
        if let Ok(mut streams) = self.recorded.lock() {
            if !streams.iter().any(|entry| entry.stream == stream) {
                streams.push(StreamProgress {
                    stream,
                    job: None,
                    phase: StreamPhase::Running,
                    update: None,
                });
            }
            if let Some(entry) = streams.iter_mut().find(|entry| entry.stream == stream) {
                change(entry);
            }
        }
    }

    /// Starts recording `stream` afresh.
    fn begin(&self, stream: PrepareStream) {
        self.update(stream, |entry| {
            entry.job = None;
            entry.phase = StreamPhase::Running;
            entry.update = None;
        });
        self.record();
    }

    /// Records how `stream` ended.
    fn finish(&self, stream: PrepareStream, phase: StreamPhase) {
        self.update(stream, |entry| entry.phase = phase);
        self.record();
    }

    /// Runs `work` watched for the `prepare` job's own cancellation (`job
    /// cancel` stops a stream within the poll interval, as it does a
    /// retranscription), recording progress once a second meanwhile.
    async fn watched<T>(&self, work: impl std::future::Future<Output = T>) -> T {
        let run = self.store.watch_for_job_cancel(
            &self.session,
            &self.prepare,
            &self.cancellation.0,
            work,
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

    /// A transcript stream: `range` retranscribed (the whole source for
    /// `None`), keeping its chunk answers in the seed and taking those already
    /// there.
    async fn transcribe(
        &self,
        stream: PrepareStream,
        range: Option<RetranscribeRange>,
        publication: RetranscribePublication,
    ) -> Result<RetranscribeOutcome, EngineError> {
        let recorded = Arc::clone(&self.recorded);
        let host = self.progress.clone();
        // The callback only keeps the latest update (it must not block); the
        // loop of `watched` writes it out.
        let progress = ProgressObserver::new(move |seen: &JobProgress| {
            host.report(seen);
            if let Ok(mut streams) = recorded.lock()
                && let Some(entry) = streams.iter_mut().find(|entry| entry.stream == stream)
            {
                entry.job.clone_from(&seen.job);
                entry.update = Some(seen.update);
            }
        });
        let run = Box::pin(self.engine.retranscribe(RetranscribeRequest {
            session: self.session.clone(),
            range,
            operation_id: None,
            cancellation: self.cancellation.clone(),
            progress,
            // Queued work waits for capacity as long as it takes, and yields
            // it at every chunk boundary to anyone who waits (section 6).
            admission: if self.mode == JobMode::Queued {
                AdmissionWait::Queued
            } else {
                self.admission
            },
            publication,
            share: if self.mode == JobMode::Watched {
                RecognitionShare::LeavingOneFree
            } else {
                RecognitionShare::Whole
            },
            seed: Some(self.seed.clone()),
        }));
        self.watched(run).await
    }

    /// The first window (D5): `first` put on the whole run's chunk grid and
    /// retranscribed as a range, sealed at once.
    async fn first_window(&self, first: TimeRange) -> Result<RetranscribeOutcome, EngineError> {
        let end = self
            .engine
            .committed_source_duration(&self.session, &self.cancellation)
            .await?;
        let aligned = align_to_chunk_grid(first, ChunkPlan::R0, end)
            .ok_or(EngineError::RangeOutsideSource)?;
        self.transcribe(
            PrepareStream::FirstWindow,
            Some(RetranscribeRange {
                from_micros: aligned.start().as_micros(),
                to_micros: aligned.end().as_micros(),
            }),
            RetranscribePublication::AtTheSeal,
        )
        .await
    }

    /// The visual stream (D6): batches of windows until the index holds every
    /// window of the source. A busy root is asked again for about a minute.
    async fn visual(&self) -> Result<StreamEnd, EngineError> {
        let stopwatch = MonotonicStopwatch::started();
        let started = stopwatch.elapsed();
        let mut fresh_windows: u64 = 0;
        let mut busy = 0_u32;
        // Time spent waiting for the root is no window's work (D8).
        let mut waited = Duration::ZERO;
        loop {
            let batch = self
                .engine
                .analyse_visual_batch(&self.session, &self.cancellation, VISUAL_BATCH_WINDOWS)
                .await;
            let batch = match batch {
                Ok(batch) => batch,
                Err(EngineError::NoVideoStream | EngineError::UnsupportedVideoStream) => {
                    return Ok(StreamEnd::Skipped);
                }
                Err(error) if error.is_media_contention() && busy < VISUAL_BUSY_RETRIES => {
                    busy += 1;
                    tokio::time::sleep(ADMISSION_RETRY_AFTER).await;
                    waited += ADMISSION_RETRY_AFTER;
                    continue;
                }
                Err(error) => return Err(error),
            };
            busy = 0;
            fresh_windows += u64::try_from(batch.analysed_now).unwrap_or(u64::MAX);
            let elapsed = micros(stopwatch.elapsed().saturating_sub(started));
            let total = u64::try_from(batch.indexed + batch.missing).unwrap_or(u64::MAX);
            let indexed = u64::try_from(batch.indexed).unwrap_or(u64::MAX);
            let duration = batch.duration.as_micros();
            let update = ProgressUpdate {
                stage: ProgressStage::AnalysingVideo,
                completed: indexed,
                total: Some(total),
                media: Some(MediaProgress {
                    from_us: 0,
                    done_until_us: indexed.saturating_mul(VISUAL_WINDOW_MICROS).min(duration),
                    to_us: Some(duration),
                    elapsed_us: elapsed,
                    fresh_media_us: fresh_windows
                        .saturating_mul(VISUAL_WINDOW_MICROS)
                        .min(duration),
                    fresh_elapsed_us: elapsed.saturating_sub(micros(waited)),
                    fresh_chunks: u32::try_from(fresh_windows).unwrap_or(u32::MAX),
                }),
            };
            self.progress.report(&JobProgress { job: None, update });
            self.update(PrepareStream::Visual, |entry| entry.update = Some(update));
            if batch.missing == 0 {
                return Ok(StreamEnd::Indexed);
            }
            if batch.analysed_now == 0 {
                // Missing windows and nothing analysed: the call stopped
                // without an error (another writer took the windows); ask again.
                tokio::time::sleep(ADMISSION_RETRY_AFTER).await;
                waited += ADMISSION_RETRY_AFTER;
            }
        }
    }

    /// The job of `stream`, once it reported one.
    fn stream_job(&self, stream: PrepareStream) -> Option<JobId> {
        self.recorded.lock().ok().and_then(|streams| {
            streams
                .iter()
                .find(|entry| entry.stream == stream)
                .and_then(|entry| entry.job.clone())
        })
    }

    /// How `stream`, which failed with `error`, stopped: as its job records
    /// it, when it had one.
    fn stop_of(&self, stream: PrepareStream, error: &EngineError) -> StreamStop {
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
            // A stop of the visual stream's analysis: the caller's interrupt,
            // or `job cancel` of the `prepare` job, which `run_prepare` turns
            // into a cancellation. Every window it committed stays.
            EngineError::VisualAnalysisStopped(VisualExtensionStop::Cancelled { .. }) => {
                StreamStop::Interrupted {
                    job: None,
                    failure: None,
                }
            }
            // The visual stream keeps every window it committed, so a stop a
            // retry can get past (a busy root, a deadline) leaves the job
            // resumable; one it cannot (a missing tool, an unusable source)
            // ends it, and `job start` starts it again.
            _ if stream == PrepareStream::Visual => {
                if failure.code.retryable() {
                    StreamStop::Interrupted {
                        job: None,
                        failure: Some(failure),
                    }
                } else {
                    StreamStop::Failed {
                        failure: Some(failure),
                    }
                }
            }
            _ => {
                let job = self.stream_job(stream);
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

    /// The commit of a transcript stream that succeeded.
    fn committed(&self, outcome: &RetranscribeOutcome) -> Result<StreamEnd, StreamStop> {
        self.store
            .job(&self.session, outcome.job().job_id())
            .ok()
            .flatten()
            .and_then(|view| view.record.commit)
            .map(StreamEnd::Committed)
            .ok_or(StreamStop::Failed { failure: None })
    }

    /// Records how `stream` failed and answers how it stopped.
    fn stopped(&self, stream: PrepareStream, error: EngineError) -> StreamStop {
        let stop = self.stop_of(stream, &error);
        if let Ok(mut slot) = self.error.lock() {
            *slot = Some(error);
        }
        stop
    }
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

impl PrepareStreams for EngineStreams<'_> {
    async fn run(
        &self,
        stream: PrepareStream,
        first: Option<TimeRange>,
    ) -> Result<StreamEnd, StreamStop> {
        self.begin(stream);
        let ended = match stream {
            PrepareStream::FirstWindow => match first {
                Some(first) => match self.first_window(first).await {
                    Ok(outcome) => self.committed(&outcome),
                    Err(error) => Err(self.stopped(stream, error)),
                },
                None => Ok(StreamEnd::Skipped),
            },
            PrepareStream::Visual => match Box::pin(self.watched(self.visual())).await {
                Ok(end) => Ok(end),
                Err(error) => Err(self.stopped(stream, error)),
            },
            PrepareStream::Transcript => {
                let publication = if self.mode == JobMode::Watched {
                    RetranscribePublication::AsItGoes {
                        interval: crate::PUBLICATION_INTERVAL,
                    }
                } else {
                    RetranscribePublication::AtTheSeal
                };
                match self.transcribe(stream, None, publication).await {
                    Ok(outcome) => {
                        let end = self.committed(&outcome);
                        if let Ok(mut slot) = self.outcome.lock() {
                            *slot = Some(outcome);
                        }
                        end
                    }
                    Err(error) => Err(self.stopped(stream, error)),
                }
            }
        };
        match &ended {
            Ok(StreamEnd::Skipped) => self.finish(stream, StreamPhase::Skipped),
            Ok(_) => self.finish(stream, StreamPhase::Done),
            Err(_) => self.record(),
        }
        ended
    }

    async fn before_start(&self, mode: JobMode) -> Result<(), StreamStop> {
        if mode != JobMode::Queued {
            return Ok(());
        }
        loop {
            if matches!(
                self.store.recorded_job_state(&self.session, &self.prepare),
                Ok(Some(JobState::Cancelling))
            ) {
                return Err(StreamStop::Cancelled);
            }
            if self.cancellation.is_cancelled() {
                return Err(StreamStop::Interrupted {
                    job: None,
                    failure: None,
                });
            }
            let position = queue_position(self.store, &self.prepare);
            if position == Some(0)
                && let Ok(Some(turn)) = self.store.try_take_queue_turn()
            {
                if let Ok(mut held) = self.turn.lock() {
                    *held = Some(turn);
                }
                return Ok(());
            }
            if position.is_none() {
                // Not in the queue (it was removed, or the queue is full when
                // it was started): it takes its place now.
                let now = self.engine.now_unix_seconds().unwrap_or_default();
                if self
                    .store
                    .enqueue_job(&self.session, &self.prepare, now)
                    .is_err()
                {
                    return Err(StreamStop::Failed { failure: None });
                }
            }
            tokio::time::sleep(QUEUE_TURN_POLL).await;
        }
    }

    async fn cancel(&self, job: &JobId) {
        let _ = self.engine.job_cancel(job);
    }
}

/// How often a queued job waiting its turn looks at the queue.
const QUEUE_TURN_POLL: Duration = Duration::from_secs(2);

/// Where `job` stands in the root's queue (0 for the next to run), among the
/// entries whose job has not ended; an ended job's entry is removed as it is
/// found. `None` when the job is not in the queue or the queue cannot be read.
pub(crate) fn queue_position(store: &FilesystemSessionStore, job: &JobId) -> Option<usize> {
    let entries = store.job_queue().ok()?;
    let mut position = 0;
    for entry in entries {
        let view = store.job(&entry.session_id, &entry.job_id).ok().flatten();
        let ended = view
            .as_ref()
            .is_none_or(|view| view.record.state.is_terminal());
        if ended {
            store.dequeue_job(&entry.job_id);
            continue;
        }
        if entry.job_id == *job {
            return Some(position);
        }
        position += 1;
    }
    None
}
