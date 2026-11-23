//! Presentation of `clocks` results through the v1 contract (ADR 0029, stage 2).
//!
//! The engine finds the clocks and records what the request states; this module
//! parses the statements and maps the typed result into the contract's.

use std::num::NonZeroU16;

use vsift::{Cancellation, ClockId, ClocksRequest, Engine, FailureCode, GapId, UtcOffset};
use vsift_contract::{ClocksPresentation, OperationResponse, clocks_response};

use crate::{CommandFailure, command::ClocksArguments, session::session_lifecycle};

/// One `--screen-zone <clock>=<±HH:MM>`: what the user stated, an offset rather
/// than an IANA name (ADR 0029 decision 16).
pub(crate) fn parse_screen_zone(value: &str) -> Result<(ClockId, UtcOffset), String> {
    let (clock, offset) = value
        .split_once('=')
        .ok_or_else(|| "expected <clock>=<+HH:MM or -HH:MM>, for example c1=-05:00".to_owned())?;
    let clock = ClockId::parse(clock).ok_or_else(|| format!("{clock} is not a clock id"))?;
    Ok((clock, parse_offset(offset)?))
}

fn parse_offset(text: &str) -> Result<UtcOffset, String> {
    let refused = || format!("{text} is not an offset of the form +HH:MM or -HH:MM");
    let bytes = text.as_bytes();
    let sign = match bytes {
        [b'+', _, _, b':', _, _] => 1,
        [b'-', _, _, b':', _, _] => -1,
        _ => return Err(refused()),
    };
    let hours: i32 = text[1..3].parse().map_err(|_| refused())?;
    let minutes: i32 = text[4..6].parse().map_err(|_| refused())?;
    if minutes > 59 {
        return Err(refused());
    }
    UtcOffset::from_seconds(sign * (hours * 3_600 + minutes * 60)).map_err(|_| refused())
}

/// One `--accept-gap <gap>`.
pub(crate) fn parse_gap(value: &str) -> Result<GapId, String> {
    GapId::parse(value).ok_or_else(|| format!("{value} is not a gap id"))
}

/// Finds a session's clocks and presents the call as one result.
pub(crate) async fn clocks(
    engine: &Engine,
    arguments: ClocksArguments,
    cancellation: &Cancellation,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let results = engine
        .read_clocks(ClocksRequest {
            session: arguments.session,
            max_frames: arguments.max_frames,
            runner: arguments.runner,
            threads: arguments.threads.and_then(NonZeroU16::new),
            screen_zones: arguments.screen_zone,
            accepted_gaps: arguments.accept_gap,
            cancellation: cancellation.clone(),
        })
        .await
        .map_err(CommandFailure::from)?;
    let lifecycle =
        session_lifecycle(results.session().lifetime()).map_err(CommandFailure::from)?;
    clocks_response(
        &ClocksPresentation {
            session_id: results.session().session_id(),
            state: results.state().identifier(),
            index: results.index(),
            analysis: results.analysis(),
            register: results.register(),
            anchors: results.session().time_anchors(),
            observations: results.observations(),
            conflicts: results.conflicts(),
            read_now: results.read_now(),
            stop: results.stop(),
            reads_pending: results.reads_pending(),
            frames_pending: results.frames_pending(),
        },
        lifecycle,
    )
    .map_err(|_| CommandFailure::from(FailureCode::Internal))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zone_is_a_clock_and_an_offset() {
        let parsed = parse_screen_zone("c1=-05:00");
        assert_eq!(parsed.as_ref().map(|(clock, _)| clock.ordinal()), Ok(1));
        assert_eq!(parsed.map(|(_, offset)| offset.as_seconds()), Ok(-18_000));
    }

    #[test]
    fn anything_else_is_refused_before_any_work() {
        for value in [
            "c1",
            "c1=",
            "c1=America/New_York",
            "c1=-5",
            "c1=-0500",
            "c1=+15:00",
            "c1=+01:60",
            "c0=+01:00",
            "1=+01:00",
        ] {
            assert!(parse_screen_zone(value).is_err(), "{value}");
        }
        assert!(parse_gap("g1").is_ok());
        assert!(parse_gap("c1").is_err());
    }
}
