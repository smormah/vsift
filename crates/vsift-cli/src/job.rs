//! Presentation of the public job commands (`job status`, `job resume`,
//! `job cancel`) through the v1 contract (P10 PR 3).
//!
//! The engine finds the job through the root's job index, reconciles it and
//! runs or cancels it; this module maps its typed reports into
//! `vsift-contract` data. A job result names no path and no transcript text.

use vsift::{Cancellation, Engine, JobCancelReport, JobResumeRequest, JobStatusReport};
use vsift_contract::{
    CANCELLATION_TOO_LATE_WARNING, CommandName, JobData, JobPresentation, JobResumeData,
    OperationResponse,
};

use crate::{CommandFailure, command::JobIdentityArguments, session::retranscription};

type Response = OperationResponse<serde_json::Value>;

/// The contract's view of one job report.
pub(crate) fn job_data(report: &JobStatusReport) -> JobData {
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
/// result, with the warning `cancellation_too_late`.
pub(crate) fn cancel(
    engine: &Engine,
    arguments: &JobIdentityArguments,
) -> Result<Response, CommandFailure> {
    let report: JobCancelReport = engine.job_cancel(&arguments.job)?;
    let response = complete(CommandName::JobCancel, &job_data(report.status()))?;
    Ok(if report.too_late() {
        response.with_warnings(&[CANCELLATION_TOO_LATE_WARNING])
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
) -> Result<Response, CommandFailure> {
    let report = engine
        .job_resume(JobResumeRequest {
            job: arguments.job.clone(),
            cancellation: cancellation.clone(),
        })
        .await?;
    let presented = retranscription(report.outcome())?;
    let data = JobResumeData::new(job_data(report.status()), presented.data);
    Ok(complete(CommandName::JobResume, &data)?
        .with_operation_id(report.outcome().job().operation_id())
        .with_lifecycle(presented.lifecycle)
        .with_warnings(&presented.warnings))
}
