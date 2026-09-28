//! Worker batches: the engine side of `job batch` (P11 PR 4, ADR 0021
//! section 5).
//!
//! [`Engine::run_work_batch`] runs the requests of one finite JSON Lines
//! file in a worker workspace, each exactly as [`Engine::run_work_request`]
//! runs one, and summarises them as `job-batch-data`:
//!
//! - **Finite and bounded.** The file is opened once and its lines counted
//!   through the same handle before anything starts: more than
//!   [`MAX_BATCH_LINES`] refuse the whole batch (`line_limit`, nothing run),
//!   and a file that cannot be read to its end is an `input_error`. Then it
//!   is read again one line at a time, each line held to 64 KiB.
//! - **Backpressure (X-08).** The next line is read only when fewer than
//!   `concurrency` requests run, and a request's events are handed to the
//!   host through its bounded channel with `send().await`: a host that stops
//!   reading stops the batch from reading further. The file, the results
//!   and the events are never all in memory; the summary holds one bounded
//!   item per line.
//! - **Isolation.** Every line is its own request, with its own child
//!   cancellation, deadline, record and result. A line that cannot be
//!   decoded, and a line whose operation id an earlier line of the batch
//!   used, is refused alone (`INVALID_ARGUMENT`); the others run. Blank lines
//!   are skipped but counted.
//! - **Runtime.** Each request runs as its own Tokio task over the shared
//!   engine (the request's future is `Send`), so requests progress in
//!   parallel on a multi-threaded runtime even while the host is busy
//!   writing, and a slow request never delays the reading of the next line
//!   once its slot frees.
//! - **Shutdown (D4).** The run's `stop` stops the reading: no further line
//!   starts, and each running request starts no further step; its
//!   `cancellation` (fired by the host after its drain time) stops their
//!   running steps at the next boundary. Every started request stays
//!   resumable, and the summary names the first line not started.

use std::{collections::HashMap, collections::HashSet, num::NonZeroU16, path::PathBuf, sync::Arc};

use tokio::{sync::mpsc, task::JoinSet};
use vsift_contract::{
    BatchLine as DecodedLine, BatchTermination, JobBatchData, MAX_BATCH_LINES, Readiness,
    RequestRejection, WORK_REQUEST_LIMITS, decode_batch_line,
};
use vsift_domain::{AdmissionWait, FailureCode, OperationId, OperationStatus};
use vsift_infrastructure::{BatchFile, BatchFileError, BatchLine};

use crate::{
    engine::Engine,
    error::{EngineError, WorkerFailure},
    progress::ProgressObserver,
    verification::Cancellation,
    worker::{WorkOutcome, WorkRequestRun},
};

/// Most requests one batch runs at once, whatever the workspace's capacity.
pub const MAX_BATCH_CONCURRENCY: u16 = 16;

/// Builds the progress observer of one request of a batch, from its line
/// and operation id; a host installs its own per-request gate here.
pub type BatchProgress = Arc<dyn Fn(u32, &OperationId) -> ProgressObserver + Send + Sync>;

/// One batch to run, with the host's controls.
pub struct WorkBatchRun {
    /// The JSON Lines request file: at most [`MAX_BATCH_LINES`] lines of at
    /// most 64 KiB, each a job-request v1 object or blank.
    pub requests: PathBuf,
    /// The operator's absolute input root, for every request.
    pub input_root: PathBuf,
    /// The operator's absolute bundle root, for `retain` steps.
    pub bundle_root: Option<PathBuf>,
    /// How each step waits for admission capacity.
    pub admission: AdmissionWait,
    /// Requests run at once: at most [`MAX_BATCH_CONCURRENCY`] and the
    /// workspace's admission capacity.
    pub concurrency: NonZeroU16,
    /// Fired by a shutdown: no further line is read and no running request
    /// starts another step.
    pub stop: Cancellation,
    /// Stops the running steps at their next boundary (a shutdown once its
    /// drain time is over, or a second signal). Each request gets a child.
    pub cancellation: Cancellation,
    /// Each request's progress observer.
    pub progress: BatchProgress,
    /// Where the batch reports its requests as they start and end. Sent
    /// with `send().await`: a full channel holds the batch back.
    pub events: mpsc::Sender<BatchEvent>,
}

/// What a batch tells its host, in the order it happens.
#[derive(Debug)]
pub enum BatchEvent {
    /// The request of `line` was admitted and started.
    Admitted {
        /// Its 1-based line.
        line: u32,
        /// Its operation id.
        operation_id: OperationId,
    },
    /// The line ended: refused, or its request finished.
    Finished {
        /// Its 1-based line.
        line: u32,
        /// How it ended.
        end: BatchLineEnd,
    },
}

/// How one line of a batch ended.
#[derive(Debug)]
pub enum BatchLineEnd {
    /// The line was refused before it ran: no result exists.
    Refused {
        /// Its operation id, when it had a valid one.
        operation_id: Option<OperationId>,
        /// Why.
        rejection: RequestRejection,
    },
    /// The request ran (or was refused by the engine against the
    /// workspace) and has a result.
    Ran(Box<WorkOutcome>),
    /// The request's task ended without an outcome (it panicked or was
    /// aborted): reported as `INTERNAL`, with no result.
    Lost {
        /// Its operation id.
        operation_id: OperationId,
    },
}

impl BatchLineEnd {
    /// The item status and failure code the line contributes to the
    /// summary: a refusal is `rejected`; an ended request's result whose
    /// record could not be written is `failed` with `STORAGE_IO`, since it
    /// is not acknowledged.
    fn record(&self, line: u32, summary: &mut JobBatchData) -> Result<(), EngineError> {
        let recorded = match self {
            Self::Refused {
                operation_id,
                rejection,
            } => summary.record_rejection(line, operation_id.as_ref(), *rejection),
            Self::Ran(outcome) => {
                let result = outcome.result();
                let operation_id = OperationId::parse(result.operation_id())
                    .map_err(|_| EngineError::JobInvariant)?;
                match (outcome.unrecorded(), result.rejection()) {
                    (Some(_), _) => summary.record_result(
                        line,
                        &operation_id,
                        OperationStatus::Failed,
                        Some(FailureCode::StorageIo),
                    ),
                    (None, Some(rejection)) => {
                        summary.record_rejection(line, Some(&operation_id), rejection)
                    }
                    (None, None) => summary.record_result(
                        line,
                        &operation_id,
                        result.status(),
                        result.failure_code(),
                    ),
                }
            }
            Self::Lost { operation_id } => summary.record_result(
                line,
                operation_id,
                OperationStatus::Failed,
                Some(FailureCode::Internal),
            ),
        };
        recorded.map_err(|_| EngineError::JobInvariant)
    }
}

/// Why the reading of a batch ended.
enum Reading {
    /// More lines may follow.
    Open,
    /// Every line was read.
    Ended,
    /// The batch stopped reading early: why, and the first line not read.
    Stopped(BatchTermination, Option<u32>),
}

impl Engine {
    /// What a batch host states when it starts (O-03), after checking that
    /// the workspace can run `concurrency` requests at once.
    ///
    /// # Errors
    ///
    /// As [`Engine::worker_readiness`]; and
    /// [`WorkerFailure::ConcurrencyExceedsCapacity`] when `concurrency` is
    /// above [`MAX_BATCH_CONCURRENCY`] or the workspace's admission capacity.
    pub fn batch_readiness(&self, concurrency: NonZeroU16) -> Result<Readiness, EngineError> {
        let readiness = self.worker_readiness(concurrency)?;
        if concurrency.get() > MAX_BATCH_CONCURRENCY || concurrency > readiness.admission_capacity {
            return Err(EngineError::Worker(
                WorkerFailure::ConcurrencyExceedsCapacity,
            ));
        }
        Ok(readiness)
    }

    /// Runs one batch of worker requests in the worker workspace this
    /// engine's session root names (see the module documentation), and
    /// returns its summary once every started request has ended.
    ///
    /// # Errors
    ///
    /// Before anything runs, as [`Engine::batch_readiness`]. A file that cannot
    /// be read, or holds too many lines, is not an error: its summary says
    /// so (`input_error`, `line_limit`). [`EngineError::JobInvariant`] if the
    /// summary could not take a line it must (never expected), after every
    /// started request has ended.
    pub async fn run_work_batch(
        self: Arc<Self>,
        run: WorkBatchRun,
    ) -> Result<JobBatchData, EngineError> {
        self.batch_readiness(run.concurrency)?;
        let summary = JobBatchData::new();
        let max_lines = u32::try_from(MAX_BATCH_LINES).unwrap_or(u32::MAX);
        let mut file =
            match BatchFile::open(&run.requests, max_lines, WORK_REQUEST_LIMITS.max_bytes) {
                Ok(file) => file,
                Err(BatchFileError::TooManyLines) => {
                    return Ok(summary.finish(BatchTermination::LineLimit, Some(1)));
                }
                Err(BatchFileError::Unreadable(_)) => {
                    return Ok(summary.finish(BatchTermination::InputError, Some(1)));
                }
            };
        let mut batch = RunningBatch {
            engine: self,
            summary,
            tasks: JoinSet::new(),
            running: HashMap::new(),
            seen: HashSet::new(),
            stopped_work: false,
            invariant: None,
        };
        let concurrency = usize::from(run.concurrency.get());
        let mut reading = Reading::Open;
        loop {
            loop {
                if !matches!(reading, Reading::Open) {
                    break;
                }
                if run.stop.is_cancelled() {
                    let next = file.lines_read().saturating_add(1);
                    let not_started = (next <= file.lines()).then_some(next);
                    reading = Reading::Stopped(BatchTermination::Shutdown, not_started);
                    break;
                }
                if batch.tasks.len() >= concurrency {
                    break;
                }
                match file.next_line() {
                    Ok(Some(line)) => batch.start(&run, line).await,
                    Ok(None) => reading = Reading::Ended,
                    Err(BatchFileError::TooManyLines) => {
                        reading = Reading::Stopped(
                            BatchTermination::LineLimit,
                            Some(max_lines.saturating_add(1)),
                        );
                    }
                    Err(BatchFileError::Unreadable(_)) => {
                        reading = Reading::Stopped(
                            BatchTermination::InputError,
                            Some(file.lines_read().saturating_add(1)),
                        );
                    }
                }
            }
            if batch.tasks.is_empty() {
                break;
            }
            if matches!(reading, Reading::Open) {
                // A shutdown while every slot is busy must stop the reading
                // now, not when a slot frees: nothing after it starts.
                tokio::select! {
                    biased;
                    joined = batch.tasks.join_next_with_id() => batch.joined(&run, joined).await,
                    () = run.stop.cancelled() => {}
                }
            } else {
                let joined = batch.tasks.join_next_with_id().await;
                batch.joined(&run, joined).await;
            }
        }
        let (termination, not_started) = match reading {
            Reading::Stopped(termination, not_started) => (termination, not_started),
            Reading::Open | Reading::Ended if batch.stopped_work => {
                (BatchTermination::Shutdown, None)
            }
            Reading::Open | Reading::Ended => (BatchTermination::EndOfInput, None),
        };
        if let Some(error) = batch.invariant {
            return Err(error);
        }
        Ok(batch.summary.finish(termination, not_started))
    }
}

/// The requests of one batch while it runs.
struct RunningBatch {
    engine: Arc<Engine>,
    summary: JobBatchData,
    tasks: JoinSet<WorkOutcome>,
    /// The line and operation id of each running task.
    running: HashMap<tokio::task::Id, (u32, OperationId)>,
    /// Every operation id a line of this batch has used.
    seen: HashSet<OperationId>,
    /// Whether the shutdown stopped a running request.
    stopped_work: bool,
    /// The first summary invariant broken, reported once everything ended.
    invariant: Option<EngineError>,
}

impl RunningBatch {
    /// Decodes one line and starts its request, or refuses it.
    async fn start(&mut self, run: &WorkBatchRun, line: BatchLine) {
        let number = line.number();
        let decoded = match line {
            BatchLine::TooLong { .. } => Err(RequestRejection::TooLarge),
            BatchLine::Line { bytes, .. } => decode_batch_line(&bytes),
        };
        let request = match decoded {
            Ok(DecodedLine::Blank) => return,
            Ok(DecodedLine::Request(request)) => *request,
            Err(rejection) => {
                self.finish(
                    run,
                    number,
                    BatchLineEnd::Refused {
                        operation_id: None,
                        rejection,
                    },
                )
                .await;
                return;
            }
        };
        let operation_id = request.operation_id().clone();
        if !self.seen.insert(operation_id.clone()) {
            self.finish(
                run,
                number,
                BatchLineEnd::Refused {
                    operation_id: Some(operation_id),
                    rejection: RequestRejection::DuplicateOperationId,
                },
            )
            .await;
            return;
        }
        let _ = run
            .events
            .send(BatchEvent::Admitted {
                line: number,
                operation_id: operation_id.clone(),
            })
            .await;
        let work = WorkRequestRun {
            request,
            input_root: run.input_root.clone(),
            bundle_root: run.bundle_root.clone(),
            admission: run.admission,
            concurrency: run.concurrency,
            stop: run.stop.clone(),
            cancellation: run.cancellation.child(),
            progress: (run.progress)(number, &operation_id),
        };
        let engine = Arc::clone(&self.engine);
        let task = self
            .tasks
            .spawn(async move { engine.run_work_request(work).await });
        self.running.insert(task.id(), (number, operation_id));
    }

    /// Takes the outcome of one ended task.
    async fn joined(
        &mut self,
        run: &WorkBatchRun,
        joined: Option<Result<(tokio::task::Id, WorkOutcome), tokio::task::JoinError>>,
    ) {
        let (id, outcome) = match joined {
            None => return,
            Some(Ok((id, outcome))) => (id, Some(outcome)),
            Some(Err(error)) => (error.id(), None),
        };
        let Some((line, operation_id)) = self.running.remove(&id) else {
            self.invariant.get_or_insert(EngineError::JobInvariant);
            return;
        };
        let end = match outcome {
            Some(outcome) => {
                if outcome.stopped_by_shutdown() {
                    self.stopped_work = true;
                }
                BatchLineEnd::Ran(Box::new(outcome))
            }
            None => BatchLineEnd::Lost { operation_id },
        };
        self.finish(run, line, end).await;
    }

    /// Records a line's end and tells the host.
    async fn finish(&mut self, run: &WorkBatchRun, line: u32, end: BatchLineEnd) {
        if let Err(error) = end.record(line, &mut self.summary) {
            self.invariant.get_or_insert(error);
        }
        // A host that has gone away no longer reads; the summary still
        // records the line.
        let _ = run.events.send(BatchEvent::Finished { line, end }).await;
    }
}
