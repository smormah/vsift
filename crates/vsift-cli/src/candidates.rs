//! Presentation of `candidates` results through the v1 contract.
//!
//! The engine analyses and pages; this module maps its typed page into the
//! `vsift-contract` result or evidence stream and formats the session expiry.

use vsift::{
    Cancellation, CandidatesRange, CandidatesRequest, CandidatesResults, Engine, EngineError,
    FailureCode, SessionStorageError, SourceProbeError, VisualExtensionStop, VisualIndexBuildError,
    VisualSamplingError,
};
use vsift_contract::{
    CandidatesEvidenceStream, CandidatesPresentation, LifecycleResponse, MEDIA_BUSY_REMEDIATION,
    OperationResponse, candidates_response,
};

use crate::{CommandFailure, command::CandidatesArguments, session::session_lifecycle};

/// Whether the failure is another request holding what `candidates` needs
/// (#342), as for the evidence commands: the capacity a window of analysis
/// reserves (a stop before the first window, or a sampling that was refused),
/// the probe's, or a session lock another request holds. `candidates` fails
/// with `BUSY` only when it analysed nothing, so nothing was committed.
const fn is_contention(error: &EngineError) -> bool {
    matches!(
        error,
        EngineError::VisualAnalysisStopped(VisualExtensionStop::Busy { .. })
            | EngineError::VisualIndexBuild(VisualIndexBuildError::Sampling(
                VisualSamplingError::Busy
            ))
            | EngineError::SourceProbe(SourceProbeError::Busy)
            | EngineError::Storage(SessionStorageError::Busy)
    )
}

/// Maps a `candidates` failure to its code, with the contention answer for
/// the causes above and the engine's generic one for every other.
fn candidates_failure(error: EngineError) -> CommandFailure {
    if is_contention(&error) {
        CommandFailure::contention(error.failure_code(), MEDIA_BUSY_REMEDIATION.to_owned())
    } else {
        CommandFailure::from(error)
    }
}

async fn run(
    engine: &Engine,
    arguments: CandidatesArguments,
    cancellation: &Cancellation,
) -> Result<CandidatesResults, CommandFailure> {
    engine
        .candidates(CandidatesRequest {
            session: arguments.session,
            range: CandidatesRange {
                from_micros: arguments.from,
                to_micros: arguments.to,
            },
            limit: arguments.limit,
            cursor: arguments.cursor,
            cancellation: cancellation.clone(),
        })
        .await
        .map_err(candidates_failure)
}

fn presentation(results: &CandidatesResults) -> CandidatesPresentation<'_> {
    CandidatesPresentation {
        session_id: results.session().session_id(),
        index: results.index(),
        source_segment_id: results.source_segment_id(),
        range: results.range(),
        searched: results.searched(),
        candidates: results.candidates(),
        next_cursor: results.next_cursor(),
        analysed: results.analysed(),
        gaps: results.gaps(),
    }
}

fn lifecycle(results: &CandidatesResults) -> Result<LifecycleResponse, FailureCode> {
    session_lifecycle(results.session().lifetime())
}

/// Pages a session's visual candidates, analysing missing windows first,
/// and presents one page as one result.
pub(crate) async fn candidates(
    engine: &Engine,
    arguments: CandidatesArguments,
    cancellation: &Cancellation,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let results = run(engine, arguments, cancellation).await?;
    candidates_response(&presentation(&results), lifecycle(&results)?)
        .map_err(|_| CommandFailure::from(FailureCode::Internal))
}

/// Pages a session's visual candidates and presents one page as an evidence
/// stream: the candidates, then the terminal event with the coverage.
pub(crate) async fn candidates_stream(
    engine: &Engine,
    arguments: CandidatesArguments,
    cancellation: &Cancellation,
) -> Result<CandidatesEvidenceStream, CommandFailure> {
    let results = run(engine, arguments, cancellation).await?;
    CandidatesEvidenceStream::new(&presentation(&results), lifecycle(&results)?)
        .map_err(|_| CommandFailure::from(FailureCode::Internal))
}

#[cfg(test)]
mod tests {
    use vsift::{
        EngineError, FailureCode, SessionStorageError, SourceProbeError, VisualExtensionStop,
        VisualIndexBuildError, VisualSamplingError,
    };
    use vsift_contract::MEDIA_BUSY_REMEDIATION;

    use super::candidates_failure;

    /// #342: `candidates` met the same contention the evidence commands do,
    /// and answers it the same way: `BUSY`, the remediation and the admission
    /// retry hint. A stop for any other reason (a deadline, a cancellation)
    /// and any other failure keep what they had.
    #[test]
    fn contention_on_candidates_is_busy_with_a_remediation_and_a_retry_hint() {
        for error in [
            EngineError::VisualAnalysisStopped(VisualExtensionStop::Busy { ordinal: 0 }),
            EngineError::VisualIndexBuild(VisualIndexBuildError::Sampling(
                VisualSamplingError::Busy,
            )),
            EngineError::SourceProbe(SourceProbeError::Busy),
            EngineError::Storage(SessionStorageError::Busy),
        ] {
            let failure = candidates_failure(error.clone());
            assert_eq!(failure.code, FailureCode::Busy, "{error}");
            assert_eq!(failure.summary(), Some(MEDIA_BUSY_REMEDIATION), "{error}");
            assert_eq!(failure.retry_after_ms, Some(2_000), "{error}");
        }

        for (error, code) in [
            (
                EngineError::VisualAnalysisStopped(VisualExtensionStop::Deadline { ordinal: 0 }),
                FailureCode::DeadlineExceeded,
            ),
            (
                EngineError::VisualAnalysisStopped(VisualExtensionStop::Cancelled { ordinal: 0 }),
                FailureCode::Cancelled,
            ),
            (
                EngineError::Storage(SessionStorageError::Io),
                FailureCode::StorageIo,
            ),
        ] {
            let failure = candidates_failure(error.clone());
            assert_eq!(failure.code, code, "{error}");
            assert_eq!(failure.summary(), None, "{error}");
            assert_eq!(failure.retry_after_ms, None, "{error}");
        }
    }
}
