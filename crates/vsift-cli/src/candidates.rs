//! Presentation of `candidates` results through the v1 contract.
//!
//! The engine analyses and pages; this module maps its typed page into the
//! `vsift-contract` result or evidence stream and formats the session expiry.

use vsift::{
    Cancellation, CandidatesRange, CandidatesRequest, CandidatesResults, Engine, FailureCode,
};
use vsift_contract::{
    CandidatesEvidenceStream, CandidatesPresentation, LifecycleResponse, OperationResponse,
    candidates_response,
};

use crate::{CommandFailure, command::CandidatesArguments, session::rfc3339};

async fn run(
    engine: &Engine,
    arguments: CandidatesArguments,
) -> Result<CandidatesResults, CommandFailure> {
    Ok(engine
        .candidates(CandidatesRequest {
            session: arguments.session,
            range: CandidatesRange {
                from_micros: arguments.from,
                to_micros: arguments.to,
            },
            limit: arguments.limit,
            cursor: arguments.cursor,
            cancellation: Cancellation::new(),
        })
        .await?)
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
    Ok(LifecycleResponse::ephemeral(rfc3339(
        results.session().lifetime().expires_at_unix_seconds(),
    )?))
}

/// Pages a session's visual candidates, analysing missing windows first,
/// and presents one page as one result.
pub(crate) async fn candidates(
    engine: &Engine,
    arguments: CandidatesArguments,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let results = run(engine, arguments).await?;
    candidates_response(&presentation(&results), lifecycle(&results)?)
        .map_err(|_| CommandFailure::from(FailureCode::Internal))
}

/// Pages a session's visual candidates and presents one page as an evidence
/// stream: the candidates, then the terminal event with the coverage.
pub(crate) async fn candidates_stream(
    engine: &Engine,
    arguments: CandidatesArguments,
) -> Result<CandidatesEvidenceStream, CommandFailure> {
    let results = run(engine, arguments).await?;
    CandidatesEvidenceStream::new(&presentation(&results), lifecycle(&results)?)
        .map_err(|_| CommandFailure::from(FailureCode::Internal))
}
