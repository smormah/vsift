//! Presentation of `search` results through the v1 contract.
//!
//! The engine searches; this module maps its typed page into the
//! `vsift-contract` result or evidence stream and formats the session expiry.

use vsift::{Engine, FailureCode, SearchRange, SearchRequest, SearchResults};
use vsift_contract::{
    LifecycleResponse, OperationResponse, ResultView, SearchEvidenceStream, SearchPresentation,
    search_response_with_speakers,
};

use crate::{CommandFailure, command::SearchArguments, session::session_lifecycle};

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
        speaker: arguments.speaker,
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
    session_lifecycle(results.session().lifetime())
}

/// Searches a session's transcript and presents one page as one result, in
/// `view`.
pub(crate) fn search(
    engine: &Engine,
    arguments: SearchArguments,
    view: ResultView,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let results = run(engine, arguments)?;
    search_response_with_speakers(
        &presentation(&results),
        results.speakers(),
        results.speaker_filter(),
        lifecycle(&results)?,
        view,
    )
    .map_err(|_| CommandFailure::from(FailureCode::Internal))
}

/// Searches a session's transcript and presents one page as an evidence
/// stream: the matching segments, then the terminal event with the hits.
///
/// A stream is always the full records; the brief view is refused before the
/// session is read ([`crate::read_view::refuse_brief_stream`]).
pub(crate) fn search_stream(
    engine: &Engine,
    arguments: SearchArguments,
) -> Result<SearchEvidenceStream, CommandFailure> {
    crate::read_view::refuse_brief_stream(arguments.view)?;
    let results = run(engine, arguments)?;
    SearchEvidenceStream::new(&presentation(&results), lifecycle(&results)?)
        .map_err(|_| CommandFailure::from(FailureCode::Internal))
}
