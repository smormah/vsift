//! Presentation of `ocr` results through the v1 contract (ADR 0030).
//!
//! The engine plans, reads and commits; this module maps its typed result into
//! the `vsift-contract` result and formats the session expiry.

use std::num::NonZeroU16;

use vsift::{
    Cancellation, Engine, FailureCode, ScreenTextRange, ScreenTextRequest, ScreenTextResults,
    ScreenTextSearchRequest, ScreenTextSearchResults, ScreenTextWindows, SearchRange,
};
use vsift_contract::{
    LifecycleResponse, OperationResponse, ScreenTextPresentation, ScreenTextSearchPagePresentation,
    screen_text_response, screen_text_search_response,
};

use crate::{
    CommandFailure,
    command::{OcrArguments, OcrSearchArguments},
    search::screen_text_part,
    session::session_lifecycle,
};

fn presentation(results: &ScreenTextResults) -> ScreenTextPresentation<'_> {
    ScreenTextPresentation {
        session_id: results.session().session_id(),
        index: results.index(),
        range: results.range(),
        period_micros: results.period_micros(),
        coverage: results.coverage(),
        candidates_complete: results.candidates_complete(),
        read_now: results.read_now(),
        stop: results.stop(),
        visual_gaps: results.visual_gaps(),
    }
}

fn lifecycle(results: &ScreenTextResults) -> Result<LifecycleResponse, FailureCode> {
    session_lifecycle(results.session().lifetime())
}

/// Reads on-screen text off a session's frames and presents the call as one result.
pub(crate) async fn ocr(
    engine: &Engine,
    arguments: OcrArguments,
    cancellation: &Cancellation,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    // The parser requires the session whenever no subcommand is given.
    let session = arguments
        .session
        .ok_or_else(|| CommandFailure::from(FailureCode::InvalidArgument))?;
    let range = match (arguments.from, arguments.to) {
        (Some(from_micros), Some(to_micros)) => Some(ScreenTextRange {
            from_micros,
            to_micros,
        }),
        _ => None,
    };
    let results = engine
        .read_screen_text(ScreenTextRequest {
            session,
            range,
            period_micros: arguments.period_us,
            max_frames: arguments.max_frames,
            runner: arguments.runner,
            threads: arguments.threads.and_then(NonZeroU16::new),
            windows: ScreenTextWindows::AnalyseMissing,
            cancellation: cancellation.clone(),
        })
        .await
        .map_err(CommandFailure::from)?;
    screen_text_response(&presentation(&results), lifecycle(&results)?)
        .map_err(|_| CommandFailure::from(FailureCode::Internal))
}

/// Pages a session's on-screen text and presents the page as one result.
pub(crate) fn search(
    engine: &Engine,
    arguments: OcrSearchArguments,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let range = match (arguments.from, arguments.to) {
        (Some(from_micros), Some(to_micros)) => Some(SearchRange {
            from_micros,
            to_micros,
        }),
        _ => None,
    };
    let results = engine
        .search_screen_text(ScreenTextSearchRequest {
            session: arguments.session,
            query: arguments.query,
            range,
            limit: arguments.limit,
            cursor: arguments.cursor,
        })
        .map_err(CommandFailure::from)?;
    let lifecycle = search_lifecycle(&results)?;
    screen_text_search_response(
        &ScreenTextSearchPagePresentation {
            session_id: results.session().session_id(),
            query: results.query(),
            range: results.range(),
            part: screen_text_part(results.screen_text()),
            next_cursor: results.next_cursor(),
        },
        lifecycle,
    )
    .map_err(|_| CommandFailure::from(FailureCode::Internal))
}

fn search_lifecycle(results: &ScreenTextSearchResults) -> Result<LifecycleResponse, FailureCode> {
    session_lifecycle(results.session().lifetime())
}
