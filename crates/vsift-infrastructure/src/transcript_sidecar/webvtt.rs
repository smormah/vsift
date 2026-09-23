//! `WebVTT` cue syntax.
//!
//! The file starts with a `WEBVTT` signature, optionally followed by a space or
//! tab and free text, then optional header lines up to the first blank line.
//! After that, blocks are cues, `NOTE` comments, or `STYLE`/`REGION`
//! definitions. The policy:
//!
//! - a `Language:` header line with a well-formed tag sets the transcript
//!   language; any other header line is ignored, but a header line containing
//!   `-->` is rejected because it would be a cue without a separating blank line;
//! - `NOTE`, `STYLE` and `REGION` blocks are skipped; they carry no cue text;
//! - a cue block is an optional identifier line then a timing line
//!   `[HH:]MM:SS.mmm --> [HH:]MM:SS.mmm [settings]`; settings only affect
//!   rendering and are ignored;
//! - any other block is untimed text and rejects the file;
//! - `<v Name>` voice spans give a provider speaker label when a cue names
//!   exactly one voice.

use vsift_domain::{
    LanguageTag, ParsedTranscript, TranscriptFormat, TranscriptImportError, TranscriptRejection,
};

use super::{Clock, CueCollector, Line, Lines, parse_timing};

pub(super) fn parse(lines: &mut Lines<'_>) -> Result<ParsedTranscript, TranscriptImportError> {
    let signature = lines.next_line()?.ok_or(TranscriptImportError::at_line(
        TranscriptRejection::InvalidHeader,
        1,
    ))?;
    let after = signature.text.strip_prefix("WEBVTT").unwrap_or("-");
    if !(after.is_empty() || after.starts_with([' ', '\t'])) || after.contains("-->") {
        return Err(TranscriptImportError::at_line(
            TranscriptRejection::InvalidHeader,
            signature.number,
        ));
    }
    let language = header(lines)?;
    let mut collector = CueCollector::new(TranscriptFormat::WebVtt);
    while let Some(first) = lines.next_non_blank()? {
        if !first.text.contains("-->") && is_definition(first) {
            lines.skip_block()?;
            continue;
        }
        let timing_line = if first.text.contains("-->") {
            first
        } else {
            match lines.next_line()? {
                Some(line) if line.text.contains("-->") => line,
                _ => {
                    return Err(TranscriptImportError::at_line(
                        TranscriptRejection::UntimedText,
                        first.number,
                    ));
                }
            }
        };
        let timing = parse_timing(timing_line, Clock::WebVtt)?;
        let payload = lines.cue_payload(timing_line)?;
        collector.push(timing_line, timing, &payload)?;
    }
    collector.finish(language)
}

/// Reads header lines up to the first blank line and returns a declared language.
fn header(lines: &mut Lines<'_>) -> Result<Option<LanguageTag>, TranscriptImportError> {
    let mut language = None;
    while let Some(line) = lines.next_line()? {
        if line.is_blank() {
            break;
        }
        if line.text.contains("-->") {
            return Err(TranscriptImportError::at_line(
                TranscriptRejection::InvalidHeader,
                line.number,
            ));
        }
        if let Some((key, value)) = line.text.split_once(':')
            && key.trim().eq_ignore_ascii_case("language")
        {
            language = LanguageTag::parse(value.trim()).ok();
        }
    }
    Ok(language)
}

/// Whether a block is a comment or a style or region definition.
fn is_definition(first: Line<'_>) -> bool {
    ["NOTE", "STYLE", "REGION"].iter().any(|keyword| {
        first
            .text
            .strip_prefix(keyword)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t']))
    })
}
