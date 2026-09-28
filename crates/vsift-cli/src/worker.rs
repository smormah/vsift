//! `job run` (P11 PR 3, ADR 0021): one versioned worker request, presented
//! through the v1 contract.
//!
//! The request file is read bounded and decoded by the contract's strict
//! decoder before anything else; a refusal names its fixed rejection and
//! never echoes the input. The engine then runs the request in the worker
//! workspace (`--session-root`) with the operator's roots and controls. The
//! result is the `job-result` as the command's `data`: `complete` and
//! `partial` succeed (exit 0); `failed` and `cancelled` carry the error of
//! the failure that ended the request, remediation from its typed cause, and
//! exit with its class (D5: the failing step's class; a shutdown is 6).
//!
//! In `--events jsonl` mode the stream is: `lifecycle started` (readiness),
//! `lifecycle request_admitted`, the request's `progress` (steps, and chunks
//! of a recognition) and `lifecycle admission_waiting` events, `lifecycle
//! draining` when a shutdown arrives, the `result` event, `lifecycle
//! request_finished`, `lifecycle stopped` (`shutdown` or `end_of_input`) and
//! the terminal event, which carries the same response `--json` prints.
//! Progress is gated and may be dropped (counted); nothing else is.

use std::{
    fs::File,
    io::{Read, Write},
    num::NonZeroU16,
    path::Path,
    pin::pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::{
    CommandFailure,
    command::JobRequestArguments,
    failure_response,
    output::{JsonLinesWriter, OutputError, OutputMode, OutputWriter, ProcessExit},
    progress::ProgressGate,
    signal::Shutdown,
    write_command_failure,
};
use tokio::sync::mpsc;
use vsift::{
    AdmissionBudget, AdmissionWait, Engine, EngineError, FailureCode, JobProgress, OperationId,
    OperationStatus, ProgressObserver, WorkOutcome, WorkRequestRun, WorkerFailure,
};
use vsift_contract::{
    BUNDLE_MISMATCH_REMEDIATION, BUNDLE_ROOT_REQUIRED_REMEDIATION, BatchItemStatus, CommandName,
    INPUT_NOT_FOUND_REMEDIATION, INPUT_NOT_REGULAR_FILE_REMEDIATION, INPUT_ROOT_REMEDIATION,
    INPUT_UNREADABLE_REMEDIATION, LifecycleEventResponse, LifecycleReason, OperationResponse,
    PARTIAL_REQUEST_WARNING, ProgressEventResponse, ProgressReport, REQUEST_BUSY_REMEDIATION,
    REQUEST_CONFLICT_REMEDIATION, REQUEST_DEADLINE_REMEDIATION, REQUEST_FILE_REMEDIATION,
    REQUEST_SESSION_REMEDIATION, REQUEST_STOPPED_REMEDIATION, Readiness, RequestEnd, RequestRef,
    ResultEventResponse, WORK_REQUEST_LIMITS, WORKER_WORKSPACE_REQUIRED_REMEDIATION, WorkRequest,
    WorkResult, decode_work_request,
};

type Response = OperationResponse<serde_json::Value>;

/// Observations queued for the writer before new ones are dropped.
const PROGRESS_QUEUE: usize = 16;

/// Runs `job run` in `mode`: the request is read and decoded first, then
/// run, then presented (see the module documentation).
pub(crate) async fn execute<StandardOutput, StandardError>(
    engine: &Engine,
    arguments: &JobRequestArguments,
    shutdown: &Shutdown,
    mode: OutputMode,
    writer: &mut OutputWriter<StandardOutput, StandardError>,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
{
    let request = match read_request(&arguments.request) {
        Ok(request) => request,
        Err(failure) => return write_command_failure(writer, mode, CommandName::JobRun, failure),
    };
    if mode == OutputMode::JsonLines {
        let readiness = match engine.worker_readiness(NonZeroU16::MIN) {
            Ok(readiness) => readiness,
            Err(error) => {
                return write_command_failure(
                    writer,
                    mode,
                    CommandName::JobRun,
                    CommandFailure::from(error),
                );
            }
        };
        let operation = request.operation_id().clone();
        return stream(writer, readiness, &operation, shutdown, |observer| {
            engine.run_work_request(work_run(request, arguments, shutdown, observer))
        })
        .await;
    }
    let outcome = engine
        .run_work_request(work_run(
            request,
            arguments,
            shutdown,
            ProgressObserver::none(),
        ))
        .await;
    let (response, exit) = present(outcome);
    let written = match mode {
        OutputMode::Json | OutputMode::JsonLines => writer.write_json(&response),
        OutputMode::Human => match serde_json::to_string_pretty(&response) {
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

/// Reads and decodes the request file: at most the request bound plus one
/// byte is read, so a larger file is refused as too large without being
/// held whole.
pub(crate) fn read_request(path: &Path) -> Result<WorkRequest, CommandFailure> {
    let unreadable = || {
        CommandFailure::with_remediation(
            FailureCode::InvalidArgument,
            REQUEST_FILE_REMEDIATION.to_owned(),
        )
    };
    let file = File::open(path).map_err(|_| unreadable())?;
    let budget = u64::try_from(WORK_REQUEST_LIMITS.max_bytes)
        .map_or(u64::MAX, |bytes| bytes.saturating_add(1));
    let mut bytes = Vec::with_capacity(WORK_REQUEST_LIMITS.max_bytes.min(64 * 1024));
    file.take(budget)
        .read_to_end(&mut bytes)
        .map_err(|_| unreadable())?;
    decode_work_request(&bytes).map_err(|rejection| {
        CommandFailure::with_remediation(
            rejection.failure_code(),
            rejection.remediation().to_owned(),
        )
    })
}

/// The engine run of `request` with the operator's roots and controls.
pub(crate) fn work_run(
    request: WorkRequest,
    arguments: &JobRequestArguments,
    shutdown: &Shutdown,
    progress: ProgressObserver,
) -> WorkRequestRun {
    let admission = AdmissionBudget::new(Duration::from_millis(arguments.admission_wait_ms))
        .map_or(AdmissionWait::Immediate, AdmissionWait::Bounded);
    WorkRequestRun {
        request,
        input_root: arguments.input_root.clone(),
        bundle_root: arguments.bundle_root.clone(),
        admission,
        concurrency: NonZeroU16::MIN,
        stop: shutdown.stop.clone(),
        cancellation: shutdown.work.clone(),
        progress,
    }
}

/// The `job.run` response for an outcome, and the process exit it earns.
pub(crate) fn present(outcome: WorkOutcome) -> (Response, ProcessExit) {
    let (result, cause) = outcome.into_parts();
    let command = CommandName::JobRun.identifier();
    let operation = OperationId::parse(result.operation_id()).ok();
    let lifecycle = result.lifecycle().cloned();
    let presented = match (result.status(), result.failure_code()) {
        (OperationStatus::Partial, _) => {
            OperationResponse::partial(command, &result, PARTIAL_REQUEST_WARNING)
                .map(|response| (response, ProcessExit::Success))
        }
        (OperationStatus::Complete, _) | (_, None) => OperationResponse::complete(command, &result)
            .map(|response| (response, ProcessExit::Success)),
        (OperationStatus::Failed | OperationStatus::Cancelled, Some(code)) => {
            let failure = failure_of(&result, cause, code);
            failure_response(CommandName::JobRun, failure)
                .with_failure_data(&result)
                .map(|response| (response, ProcessExit::from(code.class())))
        }
    };
    let Ok((mut response, exit)) = presented else {
        return (
            OperationResponse::failure(command, FailureCode::Internal),
            ProcessExit::Internal,
        );
    };
    if let Some(operation) = &operation {
        response = response.with_operation_id(operation);
    }
    if let Some(lifecycle) = lifecycle {
        response = response.with_lifecycle(lifecycle);
    }
    (response, exit)
}

/// The failure of a result: its code and hint, the session in
/// `affected_ids`, and remediation from the typed cause or rejection.
fn failure_of(
    result: &WorkResult,
    cause: Option<EngineError>,
    code: FailureCode,
) -> CommandFailure {
    let mut failure = match cause {
        Some(cause) if cause.failure_code() == code => CommandFailure::from(cause),
        _ => CommandFailure::from(code),
    };
    if failure.remediation.is_none() {
        failure.remediation = result
            .rejection()
            .map(|rejection| rejection.remediation().to_owned());
    }
    failure.retry_after_ms = failure.retry_after_ms.or_else(|| result.retry_after_ms());
    if let Some(session) = result.session_id()
        && !failure.affected_ids.iter().any(|id| id == session)
    {
        failure.affected_ids.insert(0, session.to_owned());
    }
    failure
}

/// Fixed-prose remediation for the worker host's own failures.
pub(crate) const fn worker_failure_remediation(failure: WorkerFailure) -> &'static str {
    match failure {
        WorkerFailure::WorkspaceRequired => WORKER_WORKSPACE_REQUIRED_REMEDIATION,
        WorkerFailure::BundleRootRequired => BUNDLE_ROOT_REQUIRED_REMEDIATION,
        WorkerFailure::InputRootUnavailable => INPUT_ROOT_REMEDIATION,
        WorkerFailure::InputNotFound => INPUT_NOT_FOUND_REMEDIATION,
        WorkerFailure::InputNotRegularFile => INPUT_NOT_REGULAR_FILE_REMEDIATION,
        WorkerFailure::InputUnreadable => INPUT_UNREADABLE_REMEDIATION,
        WorkerFailure::SessionNotFound => REQUEST_SESSION_REMEDIATION,
        WorkerFailure::BundleMismatch => BUNDLE_MISMATCH_REMEDIATION,
        WorkerFailure::Busy => REQUEST_BUSY_REMEDIATION,
        WorkerFailure::Conflict => REQUEST_CONFLICT_REMEDIATION,
        WorkerFailure::DeadlineExceeded => REQUEST_DEADLINE_REMEDIATION,
        WorkerFailure::Stopped => REQUEST_STOPPED_REMEDIATION,
    }
}

/// What the observer queues for the stream writer.
enum Observed {
    Progress(JobProgress, u64),
    AdmissionWaiting,
}

/// Runs the request in `--events jsonl` mode (see the module
/// documentation).
#[allow(
    clippy::too_many_lines,
    reason = "The stream's event order is the contract; keep it visible in one place"
)]
pub(crate) async fn stream<StandardOutput, StandardError, Run, Work>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    readiness: Readiness,
    request: &OperationId,
    shutdown: &Shutdown,
    run: Run,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
    Run: FnOnce(ProgressObserver) -> Work,
    Work: Future<Output = WorkOutcome>,
{
    let command = CommandName::JobRun;
    let (sender, mut receiver) = mpsc::channel::<Observed>(PROGRESS_QUEUE);
    // Lifecycle notices are never dropped; there is at most one per step.
    let (waiting_sender, mut waiting) = mpsc::unbounded_channel::<Observed>();
    let gate = Arc::new(Mutex::new(ProgressGate::new()));
    let observer = gated_observer(Arc::clone(&gate), sender, waiting_sender);

    let mut stream = JsonLinesWriter::new(writer);
    let reference = RequestRef {
        line: None,
        operation_id: Some(request),
    };
    let mut failure: Option<OutputError> = None;
    write(&mut stream, &mut failure, |sequence| {
        Event::Lifecycle(LifecycleEventResponse::started(
            sequence, command, readiness,
        ))
    });
    write(&mut stream, &mut failure, |sequence| {
        Event::Lifecycle(LifecycleEventResponse::request_admitted(
            sequence, command, &reference,
        ))
    });
    let mut draining = false;
    let mut work = pin!(run(observer));
    let outcome = loop {
        tokio::select! {
            biased;
            outcome = &mut work => break outcome,
            Some(notice) = receiver.recv() => {
                observe(&mut stream, &mut failure, request, notice);
            }
            Some(notice) = waiting.recv() => {
                observe(&mut stream, &mut failure, request, notice);
            }
            () = shutdown.stop.cancelled(), if !draining => {
                draining = true;
                write(&mut stream, &mut failure, |sequence| {
                    Event::Lifecycle(LifecycleEventResponse::draining(
                        sequence,
                        command,
                        LifecycleReason::Shutdown,
                    ))
                });
            }
        }
    };
    while let Ok(observed) = receiver.try_recv() {
        observe(&mut stream, &mut failure, request, observed);
    }
    while let Ok(observed) = waiting.try_recv() {
        observe(&mut stream, &mut failure, request, observed);
    }
    let (held, dropped) = gate.lock().map_or((None, 0), |mut gate| {
        let held = gate.finish();
        (held, gate.dropped())
    });
    if let Some((progress, dropped)) = held {
        observe(
            &mut stream,
            &mut failure,
            request,
            Observed::Progress(progress, dropped),
        );
    }
    let stopped = outcome.stopped_by_shutdown() || shutdown.stop.is_cancelled();
    // A shutdown the request ended on before the loop saw it is still
    // announced, before the result it caused.
    if shutdown.stop.is_cancelled() && !draining {
        write(&mut stream, &mut failure, |sequence| {
            Event::Lifecycle(LifecycleEventResponse::draining(
                sequence,
                command,
                LifecycleReason::Shutdown,
            ))
        });
    }
    let end = RequestEnd {
        status: item_status(outcome.result()),
        code: outcome.result().failure_code(),
        rejection: outcome.result().rejection(),
        progress_dropped: dropped,
    };
    let result = outcome.result().clone();
    let (response, exit) = present(outcome);
    write(&mut stream, &mut failure, |sequence| {
        Event::Result(ResultEventResponse::new(sequence, command, None, result))
    });
    write(&mut stream, &mut failure, |sequence| {
        Event::Lifecycle(LifecycleEventResponse::request_finished(
            sequence, command, &reference, end,
        ))
    });
    write(&mut stream, &mut failure, |sequence| {
        Event::Lifecycle(LifecycleEventResponse::stopped(
            sequence,
            command,
            if stopped {
                LifecycleReason::Shutdown
            } else {
                LifecycleReason::EndOfInput
            },
        ))
    });
    if let Some(error) = failure {
        stream.output().write_safe_diagnostic(&error.to_string());
        return ProcessExit::StorageOrIo;
    }
    match stream.write_terminal(response) {
        Ok(()) => exit,
        Err(error) => {
            writer.write_safe_diagnostic(&error.to_string());
            ProcessExit::StorageOrIo
        }
    }
}

/// The observer the engine reports to: progress through the gate into the
/// bounded queue (dropped and counted when it is full), admission waits into
/// the lifecycle queue.
fn gated_observer(
    gate: Arc<Mutex<ProgressGate>>,
    sender: mpsc::Sender<Observed>,
    waiting: mpsc::UnboundedSender<Observed>,
) -> ProgressObserver {
    ProgressObserver::new(move |progress| {
        // A poisoned gate only loses advisory progress.
        let Ok(mut gate) = gate.lock() else {
            return;
        };
        if let Some((progress, dropped)) = gate.offer(progress.clone(), Instant::now())
            && sender
                .try_send(Observed::Progress(progress, dropped))
                .is_err()
        {
            gate.queue_full();
        }
    })
    .with_admission_waiting(move |_| {
        let _ = waiting.send(Observed::AdmissionWaiting);
    })
}

/// The batch item status a request's result gives its `request_finished`.
fn item_status(result: &WorkResult) -> BatchItemStatus {
    if result.rejection().is_some() {
        return BatchItemStatus::Rejected;
    }
    match result.status() {
        OperationStatus::Complete => BatchItemStatus::Complete,
        OperationStatus::Partial => BatchItemStatus::Partial,
        OperationStatus::Failed => BatchItemStatus::Failed,
        OperationStatus::Cancelled => BatchItemStatus::Cancelled,
    }
}

/// One event of the stream.
#[derive(serde::Serialize)]
#[serde(untagged)]
enum Event {
    Lifecycle(LifecycleEventResponse),
    Progress(ProgressEventResponse),
    Result(ResultEventResponse),
}

/// Writes one event unless an earlier write failed; a failed write ends
/// the stream with exit 7 once the request has ended.
fn write<StandardOutput, StandardError>(
    stream: &mut JsonLinesWriter<'_, StandardOutput, StandardError>,
    failure: &mut Option<OutputError>,
    build: impl FnOnce(u64) -> Event,
) where
    StandardOutput: Write,
    StandardError: Write,
{
    if failure.is_some() {
        return;
    }
    if let Err(error) = stream.write_event(build) {
        *failure = Some(error);
    }
}

fn observe<StandardOutput, StandardError>(
    stream: &mut JsonLinesWriter<'_, StandardOutput, StandardError>,
    failure: &mut Option<OutputError>,
    request: &OperationId,
    observed: Observed,
) where
    StandardOutput: Write,
    StandardError: Write,
{
    let command = CommandName::JobRun;
    match observed {
        Observed::Progress(progress, dropped) => write(stream, failure, |sequence| {
            Event::Progress(ProgressEventResponse::new(
                sequence,
                command,
                &ProgressReport {
                    update: progress.update,
                    job: progress.job.as_ref(),
                    request: Some(request),
                    dropped,
                },
            ))
        }),
        Observed::AdmissionWaiting => write(stream, failure, |sequence| {
            Event::Lifecycle(LifecycleEventResponse::admission_waiting(
                sequence,
                command,
                &RequestRef {
                    line: None,
                    operation_id: Some(request),
                },
            ))
        }),
    }
}
