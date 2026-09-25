//! Presentation of `search` results through the v1 contract.
//!
//! The engine searches; this module maps its typed page into the
//! `vsift-contract` result or evidence stream and formats the session expiry.

use vsift::{Engine, FailureCode, SearchRange, SearchRequest, SearchResults};
use vsift_contract::{
    LifecycleResponse, OperationResponse, SearchEvidenceStream, SearchPresentation, search_response,
};

use crate::{CommandFailure, command::SearchArguments, session::rfc3339};

fn run(engine: &Engine, arguments: SearchArguments) -> Result<SearchResults, CommandFailure> {
    let range = arguments
        .from
        .zip(arguments.to)
        .map(|(from_micros, to_micros)| SearchRange {
            from_micros,
            to_micros,
        });
    Ok(engine.search(SearchRequest {
        session: arguments.session,
        query: arguments.query,
        revision: arguments.revision,
        range,
        limit: arguments.limit,
        cursor: arguments.cursor,
    })?)
}

fn presentation(results: &SearchResults) -> SearchPresentation<'_> {
    SearchPresentation {
        session_id: results.session().session_id(),
        revision: results.revision(),
        query: results.query(),
        range: results.range(),
        hits: results
            .hits()
            .iter()
            .map(|hit| (hit.segment(), hit.tier()))
            .collect(),
        next_cursor: results.next_cursor(),
        coverage: results.coverage(),
    }
}

fn lifecycle(results: &SearchResults) -> Result<LifecycleResponse, FailureCode> {
    Ok(LifecycleResponse::ephemeral(rfc3339(
        results.session().lifetime().expires_at_unix_seconds(),
    )?))
}

/// Searches a session's transcript and presents one page as one result.
pub(crate) fn search(
    engine: &Engine,
    arguments: SearchArguments,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let results = run(engine, arguments)?;
    search_response(&presentation(&results), lifecycle(&results)?)
        .map_err(|_| CommandFailure::from(FailureCode::Internal))
}

/// Searches a session's transcript and presents one page as an evidence
/// stream: the matching segments, then the terminal event with the hits.
pub(crate) fn search_stream(
    engine: &Engine,
    arguments: SearchArguments,
) -> Result<SearchEvidenceStream, CommandFailure> {
    let results = run(engine, arguments)?;
    SearchEvidenceStream::new(&presentation(&results), lifecycle(&results)?)
        .map_err(|_| CommandFailure::from(FailureCode::Internal))
}
