//! `job status`, `job resume`, `job cancel` and the worker hosts `job run`
//! and `job batch` (P13 PR 2b).
//!
//! Their results carry identifiers, enum values and numbers only: no path
//! and no evidence text (the worker contract forbids both). A worker host
//! renders its final result only. Its `progress`, `lifecycle` and `result`
//! events stay the JSON Lines stream of `--events jsonl`, a supervisor's
//! interface; human mode never streamed them (`docs/contracts/cli-v1.md`,
//! "Human-readable text").

use super::{
    push_outcome,
    text::{RenderedText, TerminalText, TooLarge},
    time::push_range,
    transcript::{count, push_retranscription},
    view::{
        BatchSummary, Envelope, JobResume, JobStart, JobStream, RecoverableJob, WorkResult,
        WorkStep,
    },
};

/// The coverage reason that says a step's gap list was cut at 100.
const GAPS_TRUNCATED_REASON: &str = "gap_list_truncated";

/// `job status` and `job cancel`: the job as it is now.
pub(super) fn status(
    heading: &'static str,
    envelope: &Envelope<RecoverableJob>,
) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    push_job(&mut text, heading, &envelope.data);
    push_outcome(&mut text, envelope);
    text.finish()
}

/// `job resume`: the job afterwards, then the retranscription it finished.
pub(super) fn resume(envelope: &Envelope<JobResume>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    push_job(&mut text, "Resumed job ", &data.job);
    text.blank_line();
    push_retranscription(&mut text, &data.outcome);
    push_outcome(&mut text, envelope);
    text.finish()
}

/// `job start`: what it did, the job, and the retranscription a foreground
/// run committed.
pub(super) fn start(envelope: &Envelope<JobStart>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("Started: ")
        .push_value(&data.start.state)
        .push_fixed(", mode ")
        .push_value(&data.start.mode)
        .push_fixed(" (")
        .push_value(&data.start.mode_basis)
        .push_fixed("), runner ")
        .push_value(&data.start.runner)
        .end_line();
    push_job(&mut text, "Job ", &data.job);
    if data.start.runner == "background" {
        text.push_fixed("Follow it: vsift job status ")
            .push_value(&data.job.job_id)
            .end_line();
        text.push_fixed(
            "Read the finished part while it runs with search and transcript get; it is marked partial.",
        )
        .end_line();
    }
    if let Some(outcome) = &data.outcome {
        text.blank_line();
        push_retranscription(&mut text, outcome);
    }
    push_outcome(&mut text, envelope);
    text.finish()
}

/// Writes one stream of a `prepare` job: its state and how far it has come.
fn push_stream(text: &mut TerminalText, stream: &JobStream) {
    text.push_fixed("Stream ")
        .push_value(&stream.stream)
        .push_fixed(": ")
        .push_value(&stream.state);
    if let Some(job) = &stream.job_id {
        text.push_fixed(" (job ").push_value(job).push_fixed(")");
    }
    text.end_line();
    if let Some(media) = &stream.media {
        text.push_fixed("  Done: ");
        super::time::push_clock(text, media.done_us);
        if let Some(total) = media.total_us {
            text.push_fixed(" of ");
            super::time::push_clock(text, total);
        }
        if let Some(percent) = media.percent {
            text.push_fixed(" (")
                .push_unsigned(percent)
                .push_fixed("%)");
        }
        text.end_line();
        if let Some(speed) = media.speed {
            text.push_fixed("  Speed: ")
                .push_value(&format!("{speed:.1}"))
                .push_fixed("x real time")
                .end_line();
        }
        if let Some(estimate) = &media.estimate {
            text.push_fixed("  Time left: about ")
                .push_unsigned(estimate.remaining_ms.div_ceil(60_000))
                .push_fixed(" min (an estimate from the measured speed)")
                .end_line();
        }
    } else if let Some(done) = stream.chunks_done {
        text.push_fixed("  Chunks done: ").push_unsigned(done);
        if let Some(total) = stream.chunks_total {
            text.push_fixed(" of ").push_unsigned(total);
        }
        text.end_line();
    }
    if let Some(published) = stream.published_until_us {
        text.push_fixed("  Published until: ");
        super::time::push_clock(text, published);
        text.end_line();
    }
}

/// Writes a job: identity, state, what it was asked, how far it came and
/// how it ended, then what to run next when it can be resumed.
fn push_job(text: &mut TerminalText, heading: &'static str, job: &RecoverableJob) {
    text.push_fixed(heading)
        .push_value(&job.job_id)
        .push_fixed(" (")
        .push_value(&job.kind)
        .push_fixed(")")
        .end_line();
    text.push_fixed("Session: ")
        .push_value(&job.session_id)
        .end_line();
    text.push_fixed("State: ")
        .push_value(&job.state)
        .push_fixed(if job.resumable {
            ", resumable ("
        } else {
            ", not resumable ("
        })
        .push_value(&job.resumable_reason)
        .push_fixed(if job.live_owner {
            "), running in a live process"
        } else {
            ")"
        })
        .end_line();
    if let Some(operation) = &job.operation_id {
        text.push_fixed("Bound to operation: ")
            .push_value(operation)
            .end_line();
    }
    if let Some(mode) = &job.request.mode {
        text.push_fixed("Mode: ").push_value(mode).end_line();
    }
    if let Some(first) = job.request.first {
        text.push_fixed("First window: ");
        push_range(text, first);
        text.end_line();
    }
    text.push_fixed("Range: ");
    match job.request.range {
        Some(range) => push_range(text, range),
        None => {
            text.push_fixed("the whole source");
        }
    }
    text.end_line();
    text.push_fixed("Chunks checkpointed: ")
        .push_unsigned(job.progress.chunks_checkpointed);
    match job.progress.chunks_total {
        Some(total) => text.push_fixed(" of ").push_unsigned(total),
        None => text.push_fixed(" (total not recorded)"),
    };
    text.end_line();
    text.push_fixed("Attempts in this epoch: ")
        .push_unsigned(job.attempts)
        .end_line();
    for stream in job.streams.iter().flatten() {
        push_stream(text, stream);
    }
    if let Some(result) = &job.result {
        text.push_fixed("Result: revision ")
            .push_value(&result.revision_id)
            .push_fixed(", generation ")
            .push_unsigned(result.generation)
            .end_line();
    }
    if let Some(failure) = &job.failure {
        text.push_fixed("Last failure: ")
            .push_value(&failure.code)
            .push_fixed(if failure.retryable {
                " (retryable)"
            } else {
                " (not retryable)"
            })
            .end_line();
    }
    if job.resumable {
        text.push_fixed("Resume it: vsift job resume ")
            .push_value(&job.job_id)
            .end_line();
    }
}

/// `job run`: the worker request's result, step by step. A failed or
/// cancelled request's error follows on stderr, as every failure does.
pub(super) fn run(envelope: &Envelope<WorkResult>) -> Result<RenderedText, TooLarge> {
    let result = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("Worker request: ")
        .push_value(&result.status);
    if result.replayed {
        text.push_fixed(" (replayed: the recorded result of an earlier delivery; nothing ran)");
    }
    text.end_line();
    text.push_fixed("Attempt: ")
        .push_unsigned(result.attempt)
        .end_line();
    text.push_fixed("Request digest: ")
        .push_value(&result.request_digest)
        .end_line();
    match &result.session_id {
        Some(session) => text.push_fixed("Session: ").push_value(session),
        None => text.push_fixed("Session: none opened"),
    };
    text.end_line();
    if let Some(source) = &result.source_id {
        text.push_fixed("Source: ").push_value(source).end_line();
    }
    if let Some(publication) = &result.publication {
        text.push_fixed("Publication: ")
            .push_value(publication)
            .end_line();
    }
    let controls = &result.controls;
    text.push_fixed("Controls: isolation ")
        .push_value(&controls.isolation)
        .push_fixed(", admission capacity ")
        .push_unsigned(controls.admission_capacity)
        .push_fixed(", concurrency ")
        .push_unsigned(controls.concurrency)
        .push_fixed(", resource limits ")
        .push_value(&controls.resource_limits)
        .push_fixed(", free-space reserve ")
        .push_value(&controls.free_space_reserve)
        .end_line();
    if result.steps.is_empty() {
        text.push_fixed("Steps: none ran").end_line();
    } else {
        text.push_fixed("Steps:").end_line();
    }
    for (index, step) in result.steps.iter().enumerate() {
        push_step(&mut text, count(index), step);
    }
    if let Some(failure) = &result.failure {
        text.push_fixed("Ended by: ").push_value(&failure.code);
        if let Some(step) = failure.step {
            text.push_fixed(" at step ").push_unsigned(step);
        }
        if let Some(rejection) = &failure.rejection {
            text.push_fixed(", rejection ").push_value(rejection);
        }
        text.push_fixed(if failure.retryable {
            " (retryable"
        } else {
            " (not retryable"
        });
        if let Some(retry_after_ms) = failure.retry_after_ms {
            text.push_fixed(", after ")
                .push_unsigned(retry_after_ms)
                .push_fixed(" ms");
        }
        text.push_fixed(")").end_line();
    }
    push_outcome(&mut text, envelope);
    text.finish()
}

/// Writes one step: kind, status and times, then its outputs, coverage and
/// failure.
fn push_step(text: &mut TerminalText, index: u64, step: &WorkStep) {
    text.push_fixed("  ")
        .push_unsigned(index)
        .push_fixed(". ")
        .push_value(&step.kind)
        .push_fixed(": ")
        .push_value(&step.status);
    if step.status != "not_started" {
        text.push_fixed(" (")
            .push_unsigned(step.elapsed_ms)
            .push_fixed(" ms, ")
            .push_unsigned(step.admission_wait_ms)
            .push_fixed(" ms waiting for admission)");
    }
    if let Some(job) = &step.job_id {
        text.push_fixed(", job ").push_value(job);
    }
    text.end_line();
    if let Some(outputs) = &step.outputs {
        let mut facts: Vec<String> = Vec::new();
        if let Some(index) = &outputs.visual_index_id {
            facts.push(format!("visual index {index}"));
        }
        if let Some(candidates) = outputs.candidate_count {
            facts.push(format!("{candidates} candidates"));
        }
        if let Some(bundle) = &outputs.bundle_name {
            facts.push(format!("bundle {bundle}"));
        }
        if let Some(digest) = &outputs.bundle_sha256 {
            facts.push(format!("manifest sha256 {digest}"));
        }
        if let Some(artifacts) = outputs.artifact_count {
            facts.push(format!("{artifacts} artifacts"));
        }
        match &outputs.revision_id {
            Some(revision) => facts.push(format!("revision {revision}")),
            None if step.kind == "ingest" => facts.push(String::from("no transcript imported")),
            None => {}
        }
        if let Some(reused) = outputs.chunks_reused {
            facts.push(format!("{reused} chunks reused"));
        }
        if let Some(generation) = outputs.generation {
            facts.push(format!("generation {generation}"));
        }
        if !facts.is_empty() {
            text.push_fixed("     ")
                .push_value(&facts.join(", "))
                .end_line();
        }
    }
    if let Some(coverage) = &step.coverage
        && !coverage.gaps.is_empty()
    {
        // `truncated` says that part of the step's range is not covered,
        // which every listed gap already shows; only the reason
        // `gap_list_truncated` says the list itself was cut (#362).
        let cut = coverage
            .reasons
            .iter()
            .any(|reason| reason == GAPS_TRUNCATED_REASON);
        let reasons: Vec<&str> = coverage
            .reasons
            .iter()
            .map(String::as_str)
            .filter(|reason| *reason != GAPS_TRUNCATED_REASON)
            .collect();
        text.push_fixed("     Not covered (us): ")
            .push_value(&coverage.gaps.join(", "))
            .push_fixed(" (")
            .push_value(&reasons.join(", "))
            .push_fixed(if cut {
                "; the list is cut at 100, use --json for the full coverage)"
            } else {
                ")"
            })
            .end_line();
    }
    if let Some(failure) = &step.failure {
        text.push_fixed("     Failed: ")
            .push_value(&failure.code)
            .push_fixed(if failure.retryable {
                " (retryable"
            } else {
                " (not retryable"
            });
        if let Some(retry_after_ms) = failure.retry_after_ms {
            text.push_fixed(", after ")
                .push_unsigned(retry_after_ms)
                .push_fixed(" ms");
        }
        text.push_fixed(")").end_line();
    }
}

/// `job batch`: how the batch ended and each processed line.
pub(super) fn batch(envelope: &Envelope<BatchSummary>) -> Result<RenderedText, TooLarge> {
    let summary = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("Worker batch ended: ")
        .push_value(&summary.termination_reason)
        .end_line();
    let counts = &summary.counts;
    text.push_fixed("Requests: ")
        .push_unsigned(counts.complete)
        .push_fixed(" complete, ")
        .push_unsigned(counts.partial)
        .push_fixed(" partial, ")
        .push_unsigned(counts.failed)
        .push_fixed(" failed, ")
        .push_unsigned(counts.cancelled)
        .push_fixed(" cancelled, ")
        .push_unsigned(counts.rejected)
        .push_fixed(" rejected")
        .end_line();
    if let Some(line) = summary.not_started_from_line {
        text.push_fixed("Not started from line: ")
            .push_unsigned(line)
            .push_fixed(" (deliver the file again to continue)")
            .end_line();
    }
    if !summary.items.is_empty() {
        text.push_fixed("Lines, in the order they finished:")
            .end_line();
    }
    for item in &summary.items {
        text.push_fixed("  line ")
            .push_unsigned(item.line)
            .push_fixed("  ");
        match &item.operation_id {
            Some(operation) => text.push_value(operation),
            None => text.push_fixed("(no operation id)"),
        };
        text.push_fixed("  ").push_value(&item.status);
        if let Some(code) = &item.code {
            text.push_fixed("  ").push_value(code);
        }
        if let Some(rejection) = &item.rejection {
            text.push_fixed("  ").push_value(rejection);
        }
        text.end_line();
    }
    text.push_fixed("Each request's full result is its result event in --events jsonl.")
        .end_line();
    push_outcome(&mut text, envelope);
    text.finish()
}
