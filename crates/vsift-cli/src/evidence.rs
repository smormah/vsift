//! Presentation of the evidence navigation commands through the v1 contract
//! (P09, ADR 0019).
//!
//! The engine validates, reuses, extracts and commits; this module maps the
//! parsed command to its engine request, the engine's typed result to the
//! `vsift-contract` result or evidence stream, and an engine failure to its
//! code with the fixed-prose remediation an agent needs to recover.

use vsift::{
    Cancellation, Engine, EngineError, EvidenceResults, FailureCode, FrameBurstRequest,
    FrameGetRequest, FrameNeighboursRequest, FrameSelection, FrameTarget,
};
use vsift_contract::{
    BURST_RANGE_REMEDIATION, CROP_OUTSIDE_REMEDIATION, DeliveredEvidenceFile,
    EVIDENCE_BUDGET_REMEDIATION, EVIDENCE_KIND_REMEDIATION, EVIDENCE_PATH_REMEDIATION,
    EVIDENCE_TOOLS_REMEDIATION, EvidencePresentation, EvidencePresentationError,
    FrameEvidenceStream, LifecycleResponse, NO_FRAMES_REMEDIATION, OperationResponse,
    UNKNOWN_CANDIDATE_REMEDIATION, UNKNOWN_EVIDENCE_REMEDIATION, frame_response,
    frame_selection_summary,
};

use crate::{CommandFailure, command::FrameCommand, session::rfc3339};

/// Maps an evidence failure to its code and remediation.
///
/// Evidence-specific causes are matched first: the generic mapping would
/// describe a missing media tool in terms of transcript import.
fn evidence_failure(error: EngineError) -> CommandFailure {
    let remediation = match &error {
        EngineError::MediaToolUnavailable(_) => Some(EVIDENCE_TOOLS_REMEDIATION),
        EngineError::EvidenceBudgetExhausted => Some(EVIDENCE_BUDGET_REMEDIATION),
        EngineError::NavigationRangeTooLong => Some(BURST_RANGE_REMEDIATION),
        EngineError::CandidateNotFound => Some(UNKNOWN_CANDIDATE_REMEDIATION),
        EngineError::EvidenceNotFound => Some(UNKNOWN_EVIDENCE_REMEDIATION),
        EngineError::EvidenceKindMismatch => Some(EVIDENCE_KIND_REMEDIATION),
        EngineError::CropOutsideParent => Some(CROP_OUTSIDE_REMEDIATION),
        EngineError::NoVideoStream => Some(NO_FRAMES_REMEDIATION),
        EngineError::FrameNotSelected(selection) => Some(frame_selection_summary(*selection)),
        _ => None,
    };
    match remediation {
        Some(summary) => CommandFailure {
            code: error.failure_code(),
            remediation: Some(summary.to_owned()),
        },
        None => CommandFailure::from(error),
    }
}

/// A presentation failure: a path that JSON cannot name is the output's
/// fault, anything else an internal one.
fn presentation_failure(error: &EvidencePresentationError) -> CommandFailure {
    match error {
        EvidencePresentationError::NonUtf8Path => CommandFailure {
            code: FailureCode::StorageIo,
            remediation: Some(EVIDENCE_PATH_REMEDIATION.to_owned()),
        },
        EvidencePresentationError::OperationMismatch
        | EvidencePresentationError::FileMismatch
        | EvidencePresentationError::Serialization(_) => {
            CommandFailure::from(FailureCode::Internal)
        }
    }
}

/// Runs one frame command in the engine.
async fn run_frame(
    engine: &Engine,
    command: FrameCommand,
) -> Result<EvidenceResults, CommandFailure> {
    let cancellation = Cancellation::new();
    let result = match command {
        FrameCommand::Get(arguments) => {
            let target = match (arguments.at, arguments.candidate) {
                (Some(at_micros), None) => FrameTarget::At {
                    at_micros,
                    selection: arguments
                        .select
                        .map_or(FrameSelection::AtOrAfter, FrameSelection::from),
                    tolerance_micros: arguments.tolerance_us,
                },
                (None, Some(candidate)) => FrameTarget::Candidate(candidate),
                // The grammar requires exactly one of the two.
                (Some(_), Some(_)) | (None, None) => {
                    return Err(CommandFailure::from(FailureCode::InvalidArgument));
                }
            };
            engine
                .frame_get(FrameGetRequest {
                    session: arguments.session,
                    target,
                    cancellation,
                })
                .await
        }
        FrameCommand::Neighbours(arguments) => {
            engine
                .frame_neighbours(FrameNeighboursRequest {
                    session: arguments.session,
                    anchor: arguments.evidence,
                    count: arguments.count,
                    cancellation,
                })
                .await
        }
        FrameCommand::Burst(arguments) => {
            engine
                .frame_burst(FrameBurstRequest {
                    session: arguments.session,
                    from_micros: arguments.from,
                    to_micros: arguments.to,
                    max_frames: arguments.max_frames,
                    cancellation,
                })
                .await
        }
    };
    result.map_err(evidence_failure)
}

fn lifecycle(results: &EvidenceResults) -> Result<LifecycleResponse, CommandFailure> {
    Ok(LifecycleResponse::ephemeral(rfc3339(
        results.session().lifetime().expires_at_unix_seconds(),
    )?))
}

fn delivered(results: &EvidenceResults) -> Vec<DeliveredEvidenceFile<'_>> {
    results
        .files()
        .iter()
        .map(|file| DeliveredEvidenceFile {
            evidence_id: file.evidence_id(),
            kind: file.kind(),
            path: file.path(),
        })
        .collect()
}

/// Runs a frame command and presents its result.
pub(crate) async fn frame(
    engine: &Engine,
    command: FrameCommand,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let results = run_frame(engine, command).await?;
    let files = delivered(&results);
    frame_response(
        &EvidencePresentation {
            record: results.record(),
            reused: results.reused(),
            files: &files,
        },
        lifecycle(&results)?,
    )
    .map_err(|error| presentation_failure(&error))
}

/// Runs a frame command and presents its result as an evidence stream: the
/// items, then the terminal event with everything else.
pub(crate) async fn frame_stream(
    engine: &Engine,
    command: FrameCommand,
) -> Result<FrameEvidenceStream, CommandFailure> {
    let results = run_frame(engine, command).await?;
    let files = delivered(&results);
    FrameEvidenceStream::new(
        &EvidencePresentation {
            record: results.record(),
            reused: results.reused(),
            files: &files,
        },
        lifecycle(&results)?,
    )
    .map_err(|error| presentation_failure(&error))
}
