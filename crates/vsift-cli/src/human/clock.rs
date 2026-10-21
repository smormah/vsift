//! Clock times in human output (time anchoring, ADR 0029): the session's
//! recording time and each transcript segment's clock times.
//!
//! The values are the result's own strings (`utc`, `local`, `utc_offset`),
//! which the contract writes in fixed forms; this module only arranges them.
//! A reading names its zone honestly: an instant with its local time and
//! offset, an instant whose local zone is unknown, or a local time whose zone
//! is unknown.

use super::{
    text::TerminalText,
    view::{Clock, ClockTime, TimeAnchor},
};

/// Writes one reading.
fn push_clock(text: &mut TerminalText, clock: &Clock) {
    match (&clock.local, &clock.utc_offset, &clock.utc) {
        (Some(local), Some(offset), utc) => {
            text.push_value(local).push_value(offset);
            if let Some(utc) = utc {
                text.push_fixed(" (").push_value(utc).push_fixed(")");
            }
        }
        (Some(local), None, _) => {
            text.push_value(local)
                .push_fixed(" local time, zone unknown");
        }
        (None, _, Some(utc)) => {
            text.push_value(utc).push_fixed(", local zone unknown");
        }
        (None, _, None) => {
            text.push_fixed("unknown");
        }
    }
}

/// Writes the source and confidence of a clock time.
fn push_authority(text: &mut TerminalText, source: &str, confidence: &str) {
    text.push_fixed("from ")
        .push_value(source)
        .push_fixed(" (")
        .push_value(confidence)
        .push_fixed(")");
}

/// The `Recorded at:` line of `ingest` and the session commands, and a
/// second line when the anchors disagree.
pub(super) fn push_recording_time(text: &mut TerminalText, anchor: Option<&TimeAnchor>) {
    let Some(anchor) = anchor else {
        return;
    };
    text.push_fixed("Recorded at: ");
    push_clock(text, &anchor.recording_start);
    text.push_fixed(", ");
    push_authority(text, &anchor.source, &anchor.confidence);
    text.end_line();
    if anchor.consistency == "conflicting" {
        text.push_fixed(
            "Clock times: the recording's time sources disagree; clock times use the first. See the anchors with --json.",
        )
        .end_line();
    }
}

/// A segment's `Clock:` line, when the session has an anchor.
pub(super) fn push_segment_clock(text: &mut TerminalText, clock: Option<&ClockTime>) {
    let Some(clock) = clock else {
        return;
    };
    let (Some(start), Some(end)) = (&clock.start, &clock.end) else {
        return;
    };
    text.push_fixed("  Clock: ");
    push_clock(text, start);
    text.push_fixed(" to ");
    push_clock(text, end);
    text.push_fixed(", ");
    push_authority(text, &clock.source, &clock.confidence);
    text.end_line();
}
