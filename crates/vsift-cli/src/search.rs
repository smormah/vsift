//! Presentation of `search` results through the v1 contract.
//!
//! The engine searches; this module maps its typed page into the
//! `vsift-contract` result or evidence stream and formats the session expiry.

use vsift::{Engine, FailureCode, ScreenTextSearch, SearchRange, SearchRequest, SearchResults};
use vsift_contract::{
    LifecycleResponse, OperationResponse, ResultView, ScreenTextHitPresentation,
    ScreenTextSearchPresentation, SearchEvidenceStream, SearchPresentation,
    search_response_with_view,
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
        screen_text: results.screen_text().map(screen_text_part),
    }
}

/// The engine's on-screen text part as the contract presents it, for `search`'s
/// first page and for `ocr search`.
pub(crate) fn screen_text_part(part: &ScreenTextSearch) -> ScreenTextSearchPresentation<'_> {
    ScreenTextSearchPresentation {
        index: part.index(),
        hits: part
            .hits()
            .iter()
            .map(|hit| ScreenTextHitPresentation {
                line: hit.line(),
                frame: hit.frame(),
                candidates: hit.candidates(),
                last_time: hit.last_time(),
                frames: hit.frames(),
                tier: hit.tier(),
            })
            .collect(),
        total_hits: part.total_hits(),
        coverage: part.coverage(),
        candidates_complete: part.candidates_complete(),
        period_micros: part.period_micros(),
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
    search_response_with_view(&presentation(&results), lifecycle(&results)?, view)
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
