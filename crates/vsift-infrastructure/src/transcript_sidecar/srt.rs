//! `SubRip` cue syntax.
//!
//! A cue block is a cue number line, a `H:MM:SS,mmm --> H:MM:SS,mmm` timing
//! line and zero or more text lines, ended by a blank line. The policy is
//! strict so that nothing is guessed:
//!
//! - the cue number must be decimal digits (its value is not otherwise used);
//! - the timing line may contain nothing after the end timestamp (no
//!   coordinates or other extensions);
//! - a block whose first line is not a cue number is untimed text, unless it
//!   looks like a timing line, in which case the cue number is missing;
//! - a blank line inside cue text therefore ends the cue, and the text after it
//!   is rejected as untimed rather than silently merged.

use vsift_domain::{
    ParsedTranscript, TranscriptFormat, TranscriptImportError, TranscriptRejection,
};

use super::{Clock, CueCollector, Lines, parse_timing};

const MAX_CUE_NUMBER_DIGITS: usize = 10;

pub(super) fn parse(lines: &mut Lines<'_>) -> Result<ParsedTranscript, TranscriptImportError> {
    let mut collector = CueCollector::new(TranscriptFormat::Srt);
    while let Some(number) = lines.next_non_blank()? {
        let label = number.text.trim();
        if label.is_empty()
            || label.len() > MAX_CUE_NUMBER_DIGITS
            || !label.bytes().all(|byte| byte.is_ascii_digit())
        {
            let rejection = if label.contains("-->") {
                TranscriptRejection::InvalidCueNumber
            } else {
                TranscriptRejection::UntimedText
            };
            return Err(TranscriptImportError::at_line(rejection, number.number));
        }
        let timing_line = match lines.next_line()? {
            Some(line) if !line.is_blank() => line,
            _ => {
                return Err(TranscriptImportError::at_line(
                    TranscriptRejection::InvalidTimingLine,
                    number.number,
                ));
            }
        };
        let timing = parse_timing(timing_line, Clock::SubRip)?;
        let payload = lines.cue_payload(timing_line)?;
        collector.push(timing_line, timing, &payload)?;
    }
    collector.finish(None)
}
