//! Presentation of the public job commands (`job status`, `job resume`,
//! `job cancel`, `job start`) through the v1 contract (P10 PR 3, ADR 0033).
//!
//! The engine finds the job through the root's job index, reconciles it and
//! runs or cancels it; this module maps its typed reports into
//! `vsift-contract` data. A job result names no path and no transcript text.
//!
//! `job start` in the watched mode starts the background runner: the same
//! executable, by its own path, as `job resume <job> --json` with the
//! caller's operator options, detached from this process
//! ([`vsift::launch_detached`]). It answers once the runner
//! holds the job, within [`RUNNER_HANDSHAKE`]; a runner that does not is
//! ended, and the job is left waiting for `job resume` in the foreground.

use std::{ffi::OsString, path::PathBuf, time::Duration};

use vsift::{
    Cancellation, Engine, JobCancelReport, JobMode, JobResumeRequest, JobStartPlan,
    JobStartRequest, JobState, JobStatusReport, ProgressObserver,
};
use vsift_contract::{
    CANCELLATION_TOO_LATE_WARNING, CommandName, JobData, JobPresentation, JobResumeData, JobRunner,
    JobStartData, OperationResponse, REVISION_LEFT_OPEN_WARNING, RUNNER_NOT_STARTED_REMEDIATION,
    StreamPresentation, finish_retranscription,
};

use crate::{
    CommandFailure,
    command::{HostIsolationArgument, JobIdentityArguments, JobStartArguments},
    session::retranscription,
};

/// How long `job start` waits for its background runner to hold the job.
pub(crate) const RUNNER_HANDSHAKE: Duration = Duration::from_secs(10);

/// How often it looks.
const RUNNER_POLL: Duration = Duration::from_millis(100);

/// The operator options a background runner is started with: the caller's,
/// so that it runs against the same session root and isolation.
#[derive(Clone, Debug)]
pub(crate) struct RunnerOptions {
    /// `--session-root`, when the caller gave one.
    pub(crate) session_root: Option<PathBuf>,
    /// `--host-isolation`.
    pub(crate) host_isolation: HostIsolationArgument,
}

type Response = OperationResponse<serde_json::Value>;

/// The contract's view of one job report.
pub(crate) fn job_data(report: &JobStatusReport) -> JobData {
    let streams: Vec<StreamPresentation<'_>> = report
        .streams()
        .iter()
        .map(|stream| StreamPresentation {
            stream: stream.stream(),
            job_id: stream.job_id(),
            state: stream.state(),
            stage: stream.stage(),
            chunks_done: stream.chunks_done(),
            chunks_total: stream.chunks_total(),
            published_until_us: stream.published_until_us(),
            media: stream.media(),
            live: stream.live(),
        })
        .collect();
    JobData::new(&JobPresentation {
        job_id: report.job_id(),
        session_id: report.session_id(),
        kind: report.kind(),
        state: report.state(),
        live_owner: report.live(),
        resumability: report.resumability(),
        operation_id: report.operation_id(),
        requested: report.requested(),
        planned_chunks: report.planned_chunks(),
        checkpoints: report.checkpoints(),
        attempts: report.attempt(),
        result: report.revision_id().zip(report.committed_generation()),
        failure: report.last_failure(),
        mode: report.mode(),
        streams: &streams,
    })
}

fn complete(
    command: CommandName,
    data: &impl serde::Serialize,
) -> Result<Response, CommandFailure> {
    OperationResponse::complete(command.identifier(), data)
        .map_err(|_| CommandFailure::from(vsift::FailureCode::Internal))
}

/// Reports one job: its state, whether it can be resumed, its progress
/// and its result or last failure.
pub(crate) fn status(
    engine: &Engine,
    arguments: &JobIdentityArguments,
) -> Result<Response, CommandFailure> {
    let report = engine.job_status(&arguments.job)?;
    complete(CommandName::JobStatus, &job_data(&report))
}

/// Cancels one job, or asks its running owner to stop, and reports the job
/// afterwards. A job that had committed (or was committing) keeps its
/// result, with the warning `cancellation_too_late`. A cancelled job whose
/// published revision could not be sealed now has the warning
/// `revision_left_open` (RV-1 finding 7).
pub(crate) fn cancel(
    engine: &Engine,
    arguments: &JobIdentityArguments,
) -> Result<Response, CommandFailure> {
    let report: JobCancelReport = engine.job_cancel(&arguments.job)?;
    let response = complete(CommandName::JobCancel, &job_data(report.status()))?;
    Ok(if report.too_late() {
        response.with_warnings(&[CANCELLATION_TOO_LATE_WARNING])
    } else if report.revision_left_open() {
        response.with_warnings(&[REVISION_LEFT_OPEN_WARNING])
    } else {
        response
    })
}

/// Continues one interrupted job from its checkpoints and reports the job
/// and the revision it committed, under the operation id the job's result
/// is recorded under.
pub(crate) async fn resume(
    engine: &Engine,
    arguments: &JobIdentityArguments,
    cancellation: &Cancellation,
    progress: ProgressObserver,
) -> Result<Response, CommandFailure> {
    let report = engine
        .job_resume(JobResumeRequest {
            job: arguments.job.clone(),
            cancellation: cancellation.clone(),
            progress,
            // Interactive: contention keeps P10's two bounded retries.
            admission: vsift::AdmissionWait::Immediate,
            // A resumed run continues what it published as it goes (ADR 0033).
            publication: vsift::RetranscribePublication::AsItGoes {
                interval: vsift::PUBLICATION_INTERVAL,
            },
        })
        .await?;
    let presented = retranscription(report.outcome())?;
    let data = JobResumeData::new(job_data(report.status()), presented.data);
    Ok(finish_retranscription(
        complete(CommandName::JobResume, &data)?
            .with_operation_id(report.outcome().job().operation_id())
            .with_lifecycle(presented.lifecycle)
            .with_warnings(&presented.warnings),
        report.outcome().revision(),
    ))
}

/// Starts preparing a session (ADR 0033): records its `prepare` job, then
/// runs it here (quick) or starts its background runner (watched).
pub(crate) async fn start(
    engine: &Engine,
    arguments: &JobStartArguments,
    runner: &RunnerOptions,
    cancellation: &Cancellation,
    progress: ProgressObserver,
) -> Result<Response, CommandFailure> {
    let plan = engine.job_start(&JobStartRequest {
        session: arguments.session.clone(),
        mode: arguments.mode.map(Into::into),
    })?;
    if !plan.start().needs_runner() {
        return started(&plan, plan.status(), JobRunner::None);
    }
    match plan.mode() {
        JobMode::Quick => {
            let report = engine
                .job_resume(JobResumeRequest {
                    job: plan.job_id().clone(),
                    cancellation: cancellation.clone(),
                    progress,
                    admission: vsift::AdmissionWait::Immediate,
                    // A quick run is sealed in one generation (ADR 0033).
                    publication: vsift::RetranscribePublication::AtTheSeal,
                })
                .await?;
            let presented = retranscription(report.outcome())?;
            let data = JobStartData::new(
                job_data(report.status()),
                plan.start().identifier(),
                plan.mode(),
                plan.basis().identifier(),
                JobRunner::Foreground,
                Some(presented.data),
            );
            Ok(finish_retranscription(
                complete(CommandName::JobStart, &data)?
                    .with_operation_id(report.outcome().job().operation_id())
                    .with_lifecycle(presented.lifecycle)
                    .with_warnings(&presented.warnings),
                report.outcome().revision(),
            ))
        }
        JobMode::Watched => {
            let detachment = start_runner(engine, plan.job_id(), runner).await?;
            let status = engine.job_status(plan.job_id())?;
            started(&plan, &status, JobRunner::Background(detachment))
        }
    }
}

fn started(
    plan: &JobStartPlan,
    status: &JobStatusReport,
    runner: JobRunner,
) -> Result<Response, CommandFailure> {
    let data = JobStartData::new(
        job_data(status),
        plan.start().identifier(),
        plan.mode(),
        plan.basis().identifier(),
        runner,
        None,
    );
    complete(CommandName::JobStart, &data)
}

/// Starts the background runner of `job` and waits until it holds the job.
async fn start_runner(
    engine: &Engine,
    job: &vsift::JobId,
    options: &RunnerOptions,
) -> Result<&'static str, CommandFailure> {
    let not_started = |code| {
        let mut failure = CommandFailure::with_suggested_command(
            code,
            RUNNER_NOT_STARTED_REMEDIATION.to_owned(),
            vec![
                "job".to_owned(),
                "resume".to_owned(),
                job.as_str().to_owned(),
            ],
        );
        failure.affected_ids = vec![job.as_str().to_owned()];
        failure
    };
    let executable =
        std::env::current_exe().map_err(|_| not_started(vsift::FailureCode::MissingCapability))?;
    let folder = executable
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| not_started(vsift::FailureCode::MissingCapability))?;
    let mut runner = vsift::launch_detached(&executable, &runner_arguments(job, options), &folder)
        .map_err(|_| not_started(vsift::FailureCode::MissingCapability))?;
    let detachment = runner.detachment().identifier();
    let waited = std::time::Instant::now();
    loop {
        // The runner holds the job once it is live, or has moved it on (it
        // may have finished, or failed, already).
        let status = engine.job_status(job)?;
        if status.live() || status.state() != JobState::Queued {
            return Ok(detachment);
        }
        let ended = runner.try_exited().ok().flatten().is_some();
        if ended || waited.elapsed() >= RUNNER_HANDSHAKE {
            let _ = runner.kill();
            return Err(not_started(if ended {
                vsift::FailureCode::MissingCapability
            } else {
                vsift::FailureCode::DeadlineExceeded
            }));
        }
        tokio::time::sleep(RUNNER_POLL).await;
    }
}

/// `job resume <job> --json` and the caller's operator options. Nothing here
/// is shell text: each is one argument.
fn runner_arguments(job: &vsift::JobId, options: &RunnerOptions) -> Vec<OsString> {
    let mut arguments: Vec<OsString> = vec![
        "job".into(),
        "resume".into(),
        job.as_str().into(),
        "--json".into(),
    ];
    if let Some(root) = &options.session_root {
        arguments.push("--session-root".into());
        arguments.push(root.clone().into_os_string());
    }
    if options.host_isolation == HostIsolationArgument::StrictLinux {
        arguments.push("--host-isolation".into());
        arguments.push("strict-linux".into());
    }
    arguments
}
