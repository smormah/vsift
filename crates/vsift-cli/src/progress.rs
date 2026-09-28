//! Progress events of a long command in `--events jsonl` mode (P11, ADR 0021).
//!
//! `transcript retranscribe` and `job resume` report how many chunks they
//! have recognised. The engine calls the observer on the command's own task,
//! so the observer only decides and queues: [`ProgressGate`] lets through at
//! most one observation per second (a newer one inside the second replaces
//! the held one, which is written before the terminal event if nothing newer
//! came) and at most [`MAX_PROGRESS_EVENTS`], and a bounded queue drops
//! rather than waits when the reader is slow, counting every drop in the next
//! event's `progress_dropped`. The command's future and the queue are served
//! on one task, so the output writer (which may hold a locked, non-`Send`
//! stdout) never crosses threads. The terminal event is never dropped.

use std::{
    io::Write,
    pin::pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tokio::sync::mpsc;
use vsift::{JobProgress, ProgressObserver};
use vsift_contract::{
    CommandName, MAX_PROGRESS_EVENTS, OperationResponse, PROGRESS_INTERVAL_MS,
    ProgressEventResponse, ProgressReport,
};

use crate::{
    CommandFailure, failure_response,
    output::{JsonLinesWriter, OutputError, OutputWriter, ProcessExit},
};

/// Observations queued for the writer before new ones are dropped.
const PROGRESS_QUEUE: usize = 16;

/// An observation admitted for writing, with the drops before it.
type Admitted = (JobProgress, u64);

/// Which progress observations of one request become events.
#[derive(Debug)]
pub(crate) struct ProgressGate {
    interval: Duration,
    last_admitted: Option<Instant>,
    held: Option<JobProgress>,
    admitted: u32,
    dropped: u64,
}

impl ProgressGate {
    /// A gate with the contract's interval and cap.
    pub(crate) const fn new() -> Self {
        Self {
            interval: Duration::from_millis(PROGRESS_INTERVAL_MS),
            last_admitted: None,
            held: None,
            admitted: 0,
            dropped: 0,
        }
    }

    /// Admits `progress` for writing now, or holds it (inside the interval)
    /// or drops it (past the cap).
    pub(crate) fn offer(&mut self, progress: JobProgress, now: Instant) -> Option<Admitted> {
        if self.admitted >= MAX_PROGRESS_EVENTS {
            self.held = None;
            self.dropped = self.dropped.saturating_add(1);
            return None;
        }
        match self.last_admitted {
            Some(last) if now.saturating_duration_since(last) < self.interval => {
                self.held = Some(progress);
                None
            }
            _ => {
                self.last_admitted = Some(now);
                self.held = None;
                self.admitted += 1;
                Some((progress, self.dropped))
            }
        }
    }

    /// The admitted observation could not be queued: it is dropped.
    pub(crate) const fn queue_full(&mut self) {
        self.admitted = self.admitted.saturating_sub(1);
        self.dropped = self.dropped.saturating_add(1);
    }

    /// Observations dropped so far (the cap, a full queue).
    pub(crate) const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// At the end of the command: the held observation, if the cap allows.
    pub(crate) fn finish(&mut self) -> Option<Admitted> {
        let held = self.held.take()?;
        if self.admitted >= MAX_PROGRESS_EVENTS {
            self.dropped = self.dropped.saturating_add(1);
            return None;
        }
        self.admitted += 1;
        Some((held, self.dropped))
    }
}

/// Runs one long command in `--events jsonl` mode: its progress events as
/// they come, then its one terminal event.
pub(crate) async fn stream_with_progress<StandardOutput, StandardError, Run, Work>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    command: CommandName,
    run: Run,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
    Run: FnOnce(ProgressObserver) -> Work,
    Work: Future<Output = Result<OperationResponse<serde_json::Value>, CommandFailure>>,
{
    let (sender, mut receiver) = mpsc::channel::<Admitted>(PROGRESS_QUEUE);
    let gate = Arc::new(Mutex::new(ProgressGate::new()));
    let observer_gate = Arc::clone(&gate);
    let observer = ProgressObserver::new(move |progress| {
        // A poisoned gate only loses advisory progress.
        let Ok(mut gate) = observer_gate.lock() else {
            return;
        };
        if let Some(admitted) = gate.offer(progress.clone(), Instant::now())
            && sender.try_send(admitted).is_err()
        {
            gate.queue_full();
        }
    });

    let mut stream = JsonLinesWriter::new(writer);
    let mut output_failure = None;
    let mut work = pin!(run(observer));
    let result = loop {
        tokio::select! {
            biased;
            result = &mut work => break result,
            Some(admitted) = receiver.recv() => {
                write_progress(&mut stream, command, &admitted, &mut output_failure);
            }
        }
    };
    while let Ok(admitted) = receiver.try_recv() {
        write_progress(&mut stream, command, &admitted, &mut output_failure);
    }
    let held = gate.lock().ok().and_then(|mut gate| gate.finish());
    if let Some(admitted) = held {
        write_progress(&mut stream, command, &admitted, &mut output_failure);
    }
    if let Some(error) = output_failure {
        stream.output().write_safe_diagnostic(&error.to_string());
        return ProcessExit::StorageOrIo;
    }
    let (response, exit) = match result {
        Ok(response) => (response, ProcessExit::Success),
        Err(failure) => {
            let exit = ProcessExit::from(failure.code.class());
            (failure_response(command, failure), exit)
        }
    };
    match stream.write_terminal(response) {
        Ok(()) => exit,
        Err(error) => {
            writer.write_safe_diagnostic(&error.to_string());
            ProcessExit::StorageOrIo
        }
    }
}

/// Writes one progress event unless an earlier write failed; a failed write
/// stops further progress and fails the command with exit 7 at its end.
fn write_progress<StandardOutput, StandardError>(
    stream: &mut JsonLinesWriter<'_, StandardOutput, StandardError>,
    command: CommandName,
    (progress, dropped): &Admitted,
    output_failure: &mut Option<OutputError>,
) where
    StandardOutput: Write,
    StandardError: Write,
{
    if output_failure.is_some() {
        return;
    }
    let written = stream.write_event(|sequence| {
        ProgressEventResponse::new(
            sequence,
            command,
            &ProgressReport {
                update: progress.update,
                job: progress.job.as_ref(),
                request: None,
                dropped: *dropped,
            },
        )
    });
    if let Err(error) = written {
        *output_failure = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use vsift::{FailureCode, JobId, JobProgress, ProgressObserver, ProgressStage, ProgressUpdate};
    use vsift_contract::{CommandName, MAX_PROGRESS_EVENTS, OperationResponse};

    use super::{ProgressGate, stream_with_progress};
    use crate::{
        CommandFailure,
        output::{OutputWriter, ProcessExit},
    };

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn chunk(completed: u64) -> Result<JobProgress, vsift::IdentifierError> {
        Ok(JobProgress {
            job: Some(JobId::parse("job_0123456789abcdef0123456789abcdef")?),
            update: ProgressUpdate {
                stage: ProgressStage::RecognisingSpeech,
                completed,
                total: Some(4),
            },
        })
    }

    fn completed(admitted: Option<(JobProgress, u64)>) -> Option<(u64, u64)> {
        admitted.map(|(progress, dropped)| (progress.update.completed, dropped))
    }

    #[test]
    fn at_most_one_observation_per_second_passes_and_the_newest_is_held() -> TestResult {
        let start = Instant::now();
        let mut gate = ProgressGate::new();
        assert_eq!(completed(gate.offer(chunk(0)?, start)), Some((0, 0)));
        assert_eq!(
            completed(gate.offer(chunk(1)?, start + Duration::from_millis(400))),
            None
        );
        assert_eq!(
            completed(gate.offer(chunk(2)?, start + Duration::from_millis(900))),
            None
        );
        // After the second, the next passes and the held one is superseded.
        assert_eq!(
            completed(gate.offer(chunk(3)?, start + Duration::from_millis(1_000))),
            Some((3, 0))
        );
        assert_eq!(completed(gate.finish()), None);
        // Held at the end: written before the terminal event.
        assert_eq!(
            completed(gate.offer(chunk(4)?, start + Duration::from_millis(1_100))),
            None
        );
        assert_eq!(completed(gate.finish()), Some((4, 0)));
        Ok(())
    }

    #[test]
    fn a_full_queue_and_the_cap_drop_and_count() -> TestResult {
        let start = Instant::now();
        let mut gate = ProgressGate::new();
        assert!(gate.offer(chunk(0)?, start).is_some());
        gate.queue_full();
        assert_eq!(
            completed(gate.offer(chunk(1)?, start + Duration::from_secs(1))),
            Some((1, 1))
        );
        let mut at = start + Duration::from_secs(1);
        for _ in 1..MAX_PROGRESS_EVENTS {
            at += Duration::from_secs(1);
            assert!(gate.offer(chunk(2)?, at).is_some());
        }
        at += Duration::from_secs(1);
        assert!(gate.offer(chunk(3)?, at).is_none());
        assert!(gate.finish().is_none());
        Ok(())
    }

    fn lines(stdout: &[u8]) -> Result<Vec<serde_json::Value>, serde_json::Error> {
        stdout
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(serde_json::from_slice)
            .collect()
    }

    /// Four chunks reported at once: the first passes, the last is held and
    /// written before the terminal event, which follows at their count.
    #[tokio::test]
    async fn progress_precedes_the_terminal_event() -> TestResult {
        let mut stdout = Vec::new();
        let mut output = OutputWriter::new(&mut stdout, Vec::<u8>::new());
        let exit = stream_with_progress(
            &mut output,
            CommandName::TranscriptRetranscribe,
            |observer: ProgressObserver| async move {
                for completed in 0..=4 {
                    let progress = chunk(completed).map_err(|_| FailureCode::Internal)?;
                    observer_report(&observer, &progress);
                    tokio::task::yield_now().await;
                }
                OperationResponse::complete("transcript.retranscribe", &serde_json::json!({}))
                    .map_err(|_| CommandFailure::from(FailureCode::Internal))
            },
        )
        .await;
        assert_eq!(exit, ProcessExit::Success);
        let lines = lines(&stdout)?;
        let events: Vec<(&str, u64)> = lines
            .iter()
            .filter_map(|line| Some((line["event"].as_str()?, line["sequence"].as_u64()?)))
            .collect();
        assert_eq!(events, [("progress", 0), ("progress", 1), ("terminal", 2)]);
        assert_eq!(lines[0]["completed"], 0);
        assert_eq!(lines[1]["completed"], 4);
        assert_eq!(lines[1]["job_id"], "job_0123456789abcdef0123456789abcdef");
        assert_eq!(lines[1]["unit"], "chunks");
        assert_eq!(lines[2]["result"]["status"], "complete");
        Ok(())
    }

    /// A command that fails after reporting progress ends with its failure
    /// as the terminal event, after the progress, and its failure's exit.
    #[tokio::test]
    async fn a_failure_after_progress_is_the_terminal_event() -> TestResult {
        let mut stdout = Vec::new();
        let mut output = OutputWriter::new(&mut stdout, Vec::<u8>::new());
        let exit = stream_with_progress(
            &mut output,
            CommandName::JobResume,
            |observer: ProgressObserver| async move {
                let progress = chunk(0).map_err(|_| FailureCode::Internal)?;
                observer_report(&observer, &progress);
                Err(CommandFailure::from(FailureCode::Cancelled))
            },
        )
        .await;
        assert_eq!(exit, ProcessExit::Cancelled);
        let lines = lines(&stdout)?;
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1]["event"], "terminal");
        assert_eq!(lines[1]["sequence"], 1);
        assert_eq!(lines[1]["result"]["error"]["code"], "CANCELLED");
        Ok(())
    }

    /// Stands in for the engine calling its observer.
    fn observer_report(observer: &ProgressObserver, progress: &JobProgress) {
        observer.report(progress);
    }
}
