//! `job batch` (P11 PR 4, ADR 0021 section 5): a finite JSON Lines file of
//! worker requests, presented through the v1 contract.
//!
//! The engine reads the file, runs its requests (at most `--concurrency` at
//! once, each line independently) and summarises them as `job-batch-data`;
//! this module only presents. Its events reach it through a bounded
//! channel, so a reader that stops reading stdout stops the batch from
//! reading its file (X-08). Progress goes through one gate per request and
//! a bounded queue and is dropped, and counted, rather than delay anything.
//!
//! In `--events jsonl` mode the stream is: `lifecycle started` (readiness,
//! with the batch's concurrency); per line `request_admitted`, its
//! `progress` and `admission_waiting` events, its `result` event (a request
//! that has a result) and `request_finished` (every line that was not
//! blank, in the order they ended, refused lines included); `draining`
//! (`shutdown`, and `drain_timeout` if requests still run when the drain
//! time is over); `stopped` with the batch's termination; then the terminal
//! event carrying the same response `--json` prints: the summary, with the
//! outcome of maintainer decision D5 as its status and the process exit.

use std::{
    collections::HashMap,
    io::Write,
    num::NonZeroU16,
    pin::pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tokio::sync::mpsc;
use vsift::{
    AdmissionBudget, AdmissionWait, BatchEvent, BatchLineEnd, BatchProgress, Engine, FailureCode,
    JobProgress, OperationId, ProgressObserver, WorkBatchRun,
};
use vsift_contract::{
    BATCH_FILE_REMEDIATION, BATCH_LINE_LIMIT_REMEDIATION, BATCH_LINES_FAILED_REMEDIATION,
    BATCH_STOPPED_REMEDIATION, BatchItemStatus, BatchOutcome, BatchTermination, CommandName,
    JobBatchData, LifecycleEventResponse, LifecycleReason, OperationResponse,
    PARTIAL_BATCH_WARNING, ProgressEventResponse, ProgressReport, RequestEnd, RequestRef,
    ResultEventResponse,
};

use crate::{
    CommandFailure,
    command::JobBatchArguments,
    failure_response,
    output::{JsonLinesWriter, OutputError, OutputMode, OutputWriter, ProcessExit},
    progress::ProgressGate,
    signal::Shutdown,
    worker::{Event, item_status, write},
    write_command_failure,
};

type Response = OperationResponse<serde_json::Value>;

const COMMAND: CommandName = CommandName::JobBatch;
/// Engine events waiting for the writer before the batch waits for it.
const EVENT_QUEUE: usize = 16;
/// Progress observations waiting for the writer before new ones are
/// dropped.
const PROGRESS_QUEUE: usize = 16;

/// What the per-request observers queue for the writer.
enum Observed {
    Progress {
        operation_id: OperationId,
        progress: JobProgress,
        dropped: u64,
    },
    AdmissionWaiting {
        line: u32,
        operation_id: OperationId,
    },
}

/// The progress gate of each running request, by line.
type Gates = Arc<Mutex<HashMap<u32, Arc<Mutex<ProgressGate>>>>>;

/// Runs `job batch` in `mode` (see the module documentation).
#[allow(
    clippy::too_many_lines,
    reason = "The stream's event order is the contract; keep it visible in one place"
)]
pub(crate) async fn execute<StandardOutput, StandardError>(
    engine: Engine,
    arguments: &JobBatchArguments,
    shutdown: &Shutdown,
    mode: OutputMode,
    writer: &mut OutputWriter<StandardOutput, StandardError>,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
{
    let engine = Arc::new(engine);
    let concurrency = NonZeroU16::new(arguments.concurrency).unwrap_or(NonZeroU16::MIN);
    let readiness = match engine.batch_readiness(concurrency) {
        Ok(readiness) => readiness,
        Err(error) => {
            return write_command_failure(writer, mode, COMMAND, CommandFailure::from(error));
        }
    };
    let streaming = mode == OutputMode::JsonLines;
    let (events, mut engine_events) = mpsc::channel::<BatchEvent>(EVENT_QUEUE);
    let (progress_sender, mut progress) = mpsc::channel::<Observed>(PROGRESS_QUEUE);
    // Admission notices are never dropped; there is at most one per step
    // of a running request, so at most concurrency times eight are queued.
    let (waiting_sender, mut waiting) = mpsc::unbounded_channel::<Observed>();
    let gates: Gates = Arc::new(Mutex::new(HashMap::new()));
    let observers: BatchProgress = if streaming {
        let gates = Arc::clone(&gates);
        Arc::new(move |line, operation_id: &OperationId| {
            let gate = Arc::new(Mutex::new(ProgressGate::new()));
            if let Ok(mut gates) = gates.lock() {
                gates.insert(line, Arc::clone(&gate));
            }
            gated_observer(
                line,
                operation_id,
                gate,
                progress_sender.clone(),
                waiting_sender.clone(),
            )
        })
    } else {
        Arc::new(|_, _: &OperationId| ProgressObserver::none())
    };
    let admission = AdmissionBudget::new(Duration::from_millis(arguments.admission_wait_ms))
        .map_or(AdmissionWait::Immediate, AdmissionWait::Bounded);
    let run = WorkBatchRun {
        requests: arguments.requests.clone(),
        input_root: arguments.input_root.clone(),
        bundle_root: arguments.bundle_root.clone(),
        admission,
        concurrency,
        stop: shutdown.stop.clone(),
        cancellation: shutdown.work.clone(),
        progress: observers,
        events,
    };

    let mut stream = Stream {
        writer: if streaming {
            Some(JsonLinesWriter::new(&mut *writer))
        } else {
            None
        },
        failure: None,
        stop: shutdown.stop.clone(),
    };
    stream.emit(|sequence| {
        Event::Lifecycle(LifecycleEventResponse::started(
            sequence, COMMAND, readiness,
        ))
    });
    let mut running: u32 = 0;
    let mut draining = false;
    let mut drain_announced = shutdown.drain.is_zero();
    let mut batch = pin!(Arc::clone(&engine).run_work_batch(run));
    let summary = loop {
        tokio::select! {
            biased;
            Some(event) = engine_events.recv() => {
                stream.event(event, &mut running, &gates, &mut progress, &mut waiting);
            }
            Some(observed) = progress.recv() => stream.observed(observed),
            Some(observed) = waiting.recv() => stream.observed(observed),
            () = shutdown.stop.cancelled(), if !draining => {
                draining = true;
                stream.emit(|sequence| {
                    Event::Lifecycle(LifecycleEventResponse::draining(
                        sequence,
                        COMMAND,
                        LifecycleReason::Shutdown,
                    ))
                });
            }
            () = shutdown.work.cancelled(), if draining && !drain_announced => {
                drain_announced = true;
                // A second signal escalates rather than times out.
                if running > 0 && !shutdown.work.is_escalated() {
                    stream.emit(|sequence| {
                        Event::Lifecycle(LifecycleEventResponse::draining(
                            sequence,
                            COMMAND,
                            LifecycleReason::DrainTimeout,
                        ))
                    });
                }
            }
            summary = &mut batch => break summary,
        }
    };
    // Every event was sent before the batch returned.
    while let Ok(event) = engine_events.try_recv() {
        stream.event(event, &mut running, &gates, &mut progress, &mut waiting);
    }
    let summary = match summary {
        Ok(summary) => summary,
        Err(error) => {
            let failure = CommandFailure::from(error);
            let exit = ProcessExit::from(failure.code.class());
            let ending = stream.end(failure_response(COMMAND, failure), exit);
            return finish(writer, mode, ending);
        }
    };
    if shutdown.stop.is_cancelled() && !draining {
        stream.emit(|sequence| {
            Event::Lifecycle(LifecycleEventResponse::draining(
                sequence,
                COMMAND,
                LifecycleReason::Shutdown,
            ))
        });
    }
    stream.emit(|sequence| {
        Event::Lifecycle(LifecycleEventResponse::stopped(
            sequence,
            COMMAND,
            LifecycleReason::from(summary.termination()),
        ))
    });
    let (response, exit) = present(&summary);
    let ending = stream.end(response, exit);
    finish(writer, mode, ending)
}

/// The `job.batch` response for a summary, and the process exit it earns
/// (maintainer decision D5).
pub(crate) fn present(summary: &JobBatchData) -> (Response, ProcessExit) {
    let command = COMMAND.identifier();
    let presented = match summary.outcome() {
        BatchOutcome::Success if summary.count(BatchItemStatus::Partial) > 0 => {
            OperationResponse::partial(command, summary, PARTIAL_BATCH_WARNING)
                .map(|response| (response, ProcessExit::Success))
        }
        BatchOutcome::Success => OperationResponse::complete(command, summary)
            .map(|response| (response, ProcessExit::Success)),
        BatchOutcome::Stopped => failure_response(
            COMMAND,
            CommandFailure::with_remediation(
                FailureCode::Cancelled,
                BATCH_STOPPED_REMEDIATION.to_owned(),
            ),
        )
        .with_failure_data(summary)
        .map(|response| (response, ProcessExit::Cancelled)),
        BatchOutcome::Failed(code) => {
            let remediation = match (summary.termination(), code) {
                (BatchTermination::InputError, FailureCode::StorageIo) => BATCH_FILE_REMEDIATION,
                (BatchTermination::LineLimit, FailureCode::ResourceLimit) => {
                    BATCH_LINE_LIMIT_REMEDIATION
                }
                _ => BATCH_LINES_FAILED_REMEDIATION,
            };
            failure_response(
                COMMAND,
                CommandFailure::with_remediation(code, remediation.to_owned()),
            )
            .with_failure_data(summary)
            .map(|response| (response, ProcessExit::from(code.class())))
        }
    };
    presented.unwrap_or_else(|_| {
        (
            OperationResponse::failure(command, FailureCode::Internal),
            ProcessExit::Internal,
        )
    })
}

/// The per-request observer: progress through the request's gate into the
/// bounded queue (dropped and counted when it is full), admission waits
/// into the lifecycle queue.
fn gated_observer(
    line: u32,
    operation_id: &OperationId,
    gate: Arc<Mutex<ProgressGate>>,
    sender: mpsc::Sender<Observed>,
    waiting: mpsc::UnboundedSender<Observed>,
) -> ProgressObserver {
    let progress_id = operation_id.clone();
    let waiting_id = operation_id.clone();
    ProgressObserver::new(move |progress| {
        // A poisoned gate only loses advisory progress.
        let Ok(mut gate) = gate.lock() else {
            return;
        };
        if let Some((progress, dropped)) = gate.offer(progress.clone(), Instant::now())
            && sender
                .try_send(Observed::Progress {
                    operation_id: progress_id.clone(),
                    progress,
                    dropped,
                })
                .is_err()
        {
            gate.queue_full();
        }
    })
    .with_admission_waiting(move |_| {
        let _ = waiting.send(Observed::AdmissionWaiting {
            line,
            operation_id: waiting_id.clone(),
        });
    })
}

/// The output of one batch: the event stream in `--events jsonl` mode,
/// nothing until the end otherwise.
struct Stream<'writer, StandardOutput, StandardError> {
    writer: Option<JsonLinesWriter<'writer, StandardOutput, StandardError>>,
    failure: Option<OutputError>,
    /// A stream that cannot be written stops the batch from starting more
    /// requests: their results could not be reported.
    stop: vsift::Cancellation,
}

impl<StandardOutput, StandardError> Stream<'_, StandardOutput, StandardError>
where
    StandardOutput: Write,
    StandardError: Write,
{
    fn emit(&mut self, build: impl FnOnce(u64) -> Event) {
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        write(writer, &mut self.failure, build);
        if self.failure.is_some() {
            self.stop.cancel();
        }
    }

    fn observed(&mut self, observed: Observed) {
        match observed {
            Observed::Progress {
                operation_id,
                progress,
                dropped,
            } => self.emit(|sequence| {
                Event::Progress(ProgressEventResponse::new(
                    sequence,
                    COMMAND,
                    &ProgressReport {
                        update: progress.update,
                        job: progress.job.as_ref(),
                        request: Some(&operation_id),
                        dropped,
                    },
                ))
            }),
            Observed::AdmissionWaiting { line, operation_id } => self.emit(|sequence| {
                Event::Lifecycle(LifecycleEventResponse::admission_waiting(
                    sequence,
                    COMMAND,
                    &RequestRef {
                        line: Some(line),
                        operation_id: Some(&operation_id),
                    },
                ))
            }),
        }
    }

    /// Presents one engine event. Before a request's end, every observation
    /// queued so far (its own included) is written, then its held progress,
    /// so a request's progress always precedes its result.
    fn event(
        &mut self,
        event: BatchEvent,
        running: &mut u32,
        gates: &Gates,
        progress: &mut mpsc::Receiver<Observed>,
        waiting: &mut mpsc::UnboundedReceiver<Observed>,
    ) {
        let (line, end) = match event {
            BatchEvent::Admitted { line, operation_id } => {
                *running = running.saturating_add(1);
                self.emit(|sequence| {
                    Event::Lifecycle(LifecycleEventResponse::request_admitted(
                        sequence,
                        COMMAND,
                        &RequestRef {
                            line: Some(line),
                            operation_id: Some(&operation_id),
                        },
                    ))
                });
                return;
            }
            BatchEvent::Finished { line, end } => (line, end),
        };
        while let Ok(observed) = progress.try_recv() {
            self.observed(observed);
        }
        while let Ok(observed) = waiting.try_recv() {
            self.observed(observed);
        }
        let gate = gates.lock().ok().and_then(|mut gates| gates.remove(&line));
        let (held, dropped) = gate
            .and_then(|gate| {
                gate.lock().ok().map(|mut gate| {
                    let held = gate.finish();
                    (held, gate.dropped())
                })
            })
            .unwrap_or((None, 0));
        let (operation_id, request_end) = request_end(&end, dropped);
        if matches!(end, BatchLineEnd::Ran(_) | BatchLineEnd::Lost { .. }) {
            *running = running.saturating_sub(1);
        }
        if let (Some((progress, held_dropped)), Some(operation_id)) = (held, &operation_id) {
            self.emit(|sequence| {
                Event::Progress(ProgressEventResponse::new(
                    sequence,
                    COMMAND,
                    &ProgressReport {
                        update: progress.update,
                        job: progress.job.as_ref(),
                        request: Some(operation_id),
                        dropped: held_dropped,
                    },
                ))
            });
        }
        if let BatchLineEnd::Ran(outcome) = end {
            let result = outcome.result().clone();
            self.emit(|sequence| {
                Event::Result(ResultEventResponse::new(
                    sequence,
                    COMMAND,
                    Some(line),
                    result,
                ))
            });
        }
        self.emit(|sequence| {
            Event::Lifecycle(LifecycleEventResponse::request_finished(
                sequence,
                COMMAND,
                &RequestRef {
                    line: Some(line),
                    operation_id: operation_id.as_ref(),
                },
                request_end,
            ))
        });
    }

    /// Ends the output with `response`: the terminal event of a stream; in
    /// the other modes the caller writes it once the stream is gone.
    fn end(self, response: Response, exit: ProcessExit) -> Ending {
        let Self {
            writer, failure, ..
        } = self;
        let Some(mut stream) = writer else {
            return Ending::Pending(Box::new(response), exit);
        };
        if let Some(error) = failure {
            stream.output().write_safe_diagnostic(&error.to_string());
            return Ending::Written(ProcessExit::StorageOrIo);
        }
        match stream.write_terminal(response) {
            Ok(()) => Ending::Written(exit),
            Err(error) => Ending::Broken(error),
        }
    }
}

/// The operation id and `request_finished` members of a line's end: a
/// refusal is `rejected` with its code; an ended request's result whose
/// record could not be written is `failed` with `STORAGE_IO`, since it is
/// not acknowledged; a lost task is `failed` with `INTERNAL`.
fn request_end(end: &BatchLineEnd, dropped: u64) -> (Option<OperationId>, RequestEnd) {
    match end {
        BatchLineEnd::Refused {
            operation_id,
            rejection,
        } => (
            operation_id.clone(),
            RequestEnd {
                status: BatchItemStatus::Rejected,
                code: Some(rejection.failure_code()),
                rejection: Some(*rejection),
                progress_dropped: 0,
            },
        ),
        BatchLineEnd::Ran(outcome) => {
            let result = outcome.result();
            let end = if outcome.unrecorded().is_some() {
                RequestEnd {
                    status: BatchItemStatus::Failed,
                    code: Some(FailureCode::StorageIo),
                    rejection: None,
                    progress_dropped: dropped,
                }
            } else {
                RequestEnd {
                    status: item_status(result),
                    code: result.failure_code(),
                    rejection: result.rejection(),
                    progress_dropped: dropped,
                }
            };
            (OperationId::parse(result.operation_id()).ok(), end)
        }
        BatchLineEnd::Lost { operation_id } => (
            Some(operation_id.clone()),
            RequestEnd {
                status: BatchItemStatus::Failed,
                code: Some(FailureCode::Internal),
                rejection: None,
                progress_dropped: dropped,
            },
        ),
    }
}

/// How the output of a batch ended.
enum Ending {
    /// Everything was written; the process exits with this.
    Written(ProcessExit),
    /// The one response of `--json` or the human mode is still to write.
    Pending(Box<Response>, ProcessExit),
    /// The terminal event could not be written.
    Broken(OutputError),
}

/// Writes what [`Stream::end`] left, and returns the process exit.
fn finish<StandardOutput, StandardError>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    mode: OutputMode,
    ending: Ending,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
{
    let (response, exit) = match ending {
        Ending::Written(exit) => return exit,
        Ending::Broken(error) => {
            writer.write_safe_diagnostic(&error.to_string());
            return ProcessExit::StorageOrIo;
        }
        Ending::Pending(response, exit) => (response, exit),
    };
    let written = match mode {
        OutputMode::Json | OutputMode::JsonLines => writer.write_json(&*response),
        OutputMode::Human => match serde_json::to_string_pretty(&*response) {
            Ok(mut text) => {
                text.push('\n');
                writer.write_trusted_stdout(&text)
            }
            Err(_) => return ProcessExit::Internal,
        },
    };
    match written {
        Ok(()) => exit,
        Err(error) => {
            writer.write_safe_diagnostic(&error.to_string());
            ProcessExit::StorageOrIo
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use tokio::sync::mpsc;
    use vsift::{JobProgress, OperationId, ProgressStage, ProgressUpdate};

    use super::{Observed, PROGRESS_QUEUE, gated_observer};
    use crate::progress::ProgressGate;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn step() -> JobProgress {
        JobProgress {
            job: None,
            update: ProgressUpdate {
                stage: ProgressStage::RunningRequest,
                completed: 0,
                total: Some(2),
            },
        }
    }

    /// X-08 / O-02: the requests of a batch share one bounded progress
    /// queue. When nobody drains it, progress beyond it is dropped at once
    /// and counted in the gate of the request it belonged to (reported as
    /// that request's `progress_dropped`), never buffered, and the
    /// admission notices, which are never dropped, still arrive.
    #[test]
    fn progress_is_dropped_and_counted_not_buffered() -> TestResult {
        let (sender, mut progress) = mpsc::channel::<Observed>(PROGRESS_QUEUE);
        let (waiting_sender, mut waiting) = mpsc::unbounded_channel::<Observed>();
        let mut gates = HashMap::new();
        let requests = PROGRESS_QUEUE + 8;
        for line in 1..=u32::try_from(requests)? {
            let operation_id = OperationId::parse(format!("op_{line:032}"))?;
            let gate = Arc::new(Mutex::new(ProgressGate::new()));
            gates.insert(line, Arc::clone(&gate));
            let observer = gated_observer(
                line,
                &operation_id,
                gate,
                sender.clone(),
                waiting_sender.clone(),
            );
            observer.report(&step());
            observer.admission_waiting(&vsift::AdmissionWaiting {
                job: None,
                weight: std::num::NonZeroU16::MIN,
            });
        }
        let mut queued = 0;
        while progress.try_recv().is_ok() {
            queued += 1;
        }
        assert_eq!(queued, PROGRESS_QUEUE);
        let mut dropped = 0;
        for gate in gates.values() {
            dropped += gate.lock().map_err(|_| "poisoned")?.dropped();
        }
        assert_eq!(dropped, u64::try_from(requests - PROGRESS_QUEUE)?);
        let mut notices = 0;
        while waiting.try_recv().is_ok() {
            notices += 1;
        }
        assert_eq!(notices, requests);
        Ok(())
    }
}
