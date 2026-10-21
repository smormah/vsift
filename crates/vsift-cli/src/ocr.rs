//! Presentation of `ocr` results through the v1 contract (ADR 0030).
//!
//! The engine plans, reads and commits; this module maps its typed result into
//! the `vsift-contract` result and formats the session expiry.

use std::num::NonZeroU16;

use vsift::{
    Cancellation, Engine, FailureCode, ScreenTextRange, ScreenTextRequest, ScreenTextResults,
};
use vsift_contract::{
    LifecycleResponse, OperationResponse, ScreenTextPresentation, screen_text_response,
};

use crate::{CommandFailure, command::OcrArguments, session::session_lifecycle};

fn presentation(results: &ScreenTextResults) -> ScreenTextPresentation<'_> {
    ScreenTextPresentation {
        session_id: results.session().session_id(),
        index: results.index(),
        range: results.range(),
        period_micros: results.period_micros(),
        coverage: results.coverage(),
        candidates_complete: results.visual_gaps().is_empty(),
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
    let range = match (arguments.from, arguments.to) {
        (Some(from_micros), Some(to_micros)) => Some(ScreenTextRange {
            from_micros,
            to_micros,
        }),
        _ => None,
    };
    let results = engine
        .read_screen_text(ScreenTextRequest {
            session: arguments.session,
            range,
            period_micros: arguments.period_us,
            max_frames: arguments.max_frames,
            runner: arguments.runner,
            threads: arguments.threads.and_then(NonZeroU16::new),
            cancellation: cancellation.clone(),
        })
        .await
        .map_err(CommandFailure::from)?;
    screen_text_response(&presentation(&results), lifecycle(&results)?)
        .map_err(|_| CommandFailure::from(FailureCode::Internal))
}
