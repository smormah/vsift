//! Source times and ranges as a reader writes them.

use super::{text::TerminalText, view::Range};

/// Writes a source time in microseconds as `HH:MM:SS.ffffff`: exact to the
/// microsecond, as the JSON result is.
pub(super) fn push_clock(text: &mut TerminalText, micros: u64) {
    let seconds = micros / 1_000_000;
    let clock = format!(
        "{:02}:{:02}:{:02}.{:06}",
        seconds / 3_600,
        seconds / 60 % 60,
        seconds % 60,
        micros % 1_000_000
    );
    text.push_value(&clock);
}

/// Writes a half-open range as `start to end`, then the microsecond values
/// the commands take (`--from <start> --to <end>`).
pub(super) fn push_range(text: &mut TerminalText, range: Range) {
    push_clock(text, range.from_us);
    text.push_fixed(" to ");
    push_clock(text, range.to_us);
    text.push_fixed(" (--from ")
        .push_unsigned(range.from_us)
        .push_fixed(" --to ")
        .push_unsigned(range.to_us)
        .push_fixed(")");
}

/// Writes a segment's span as `start --> end`, the transcript form.
pub(super) fn push_span(text: &mut TerminalText, start_us: u64, end_us: u64) {
    push_clock(text, start_us);
    text.push_fixed(" --> ");
    push_clock(text, end_us);
}

#[cfg(test)]
mod tests {
    use super::{push_clock, push_range};
    use crate::human::{text::TerminalText, view::Range};

    #[test]
    fn times_are_exact_to_the_microsecond() -> Result<(), crate::human::text::TooLarge> {
        let mut text = TerminalText::result();
        push_clock(&mut text, 0);
        text.push_fixed(" ");
        push_clock(&mut text, 3_723_000_042);
        text.push_fixed(" ");
        push_range(
            &mut text,
            Range {
                from_us: 500_000,
                to_us: 360_000_000_000,
            },
        );
        assert_eq!(
            text.finish()?.as_str(),
            "00:00:00.000000 01:02:03.000042 00:00:00.500000 to 100:00:00.000000 \
             (--from 500000 --to 360000000000)\n"
        );
        Ok(())
    }
}
