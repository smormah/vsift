//! Bounded reading and parsing of supplied `SubRip` and `WebVTT` transcripts.
//!
//! Sidecar bytes are untrusted. The byte and line policy here is applied before
//! any cue is interpreted, and parsing streams line by line so memory stays
//! proportional to one cue rather than to the file:
//!
//! - at most [`MAX_SUPPLIED_TRANSCRIPT_BYTES`]; larger files are rejected;
//! - UTF-8 only: a UTF-8 byte-order mark is removed, UTF-16/UTF-32 marks and
//!   invalid UTF-8 are rejected with the line number (text is never lossily
//!   repaired, because repaired text is not what the author wrote);
//! - lines end with LF, CRLF or CR and are at most [`MAX_LINE_BYTES`];
//! - control characters other than tab, and the Unicode line and paragraph
//!   separators, are rejected, so cue text cannot forge records or drive a
//!   terminal;
//! - the format is detected from content: a `WEBVTT` signature selects
//!   `WebVTT`, anything else is parsed as `SubRip` and rejected if it is not.
//!
//! Which cues are kept, and how they align with the source, is decided by the
//! domain (`ParsedTranscript` and `align_imported_cues`).

mod srt;
mod webvtt;

use std::{io::Read, num::NonZeroU32, path::Path};

use sha2::{Digest, Sha256};
use vsift_application::{SuppliedTranscript, SuppliedTranscriptError};
use vsift_domain::{
    CueSource, CueText, CueTiming, ImportedCue, MAX_CUE_TEXT_BYTES, MAX_SUPPLIED_TRANSCRIPT_BYTES,
    MAX_TRANSCRIPT_CUES, ParsedTranscript, SidecarIdentity, SpeakerLabel, TranscriptFormat,
    TranscriptImportError, TranscriptRejection, TranscriptWarningKind, TranscriptWarnings,
};

use crate::source_snapshot::{SourceError, open_source};

/// Longest accepted sidecar line in UTF-8 bytes, excluding its terminator.
pub const MAX_LINE_BYTES: usize = 4_096;
const HEX: &[u8; 16] = b"0123456789abcdef";
const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";
const MAX_CHARACTER_REFERENCE_BYTES: usize = 32;
/// Longest tag or override block recognised as markup. Searching for a closing
/// delimiter only this far keeps markup removal linear in the line length, so a
/// line of unclosed `<` or `{\` cannot make parsing quadratic.
const MAX_MARKUP_BYTES: usize = 256;

/// Reads, identifies and parses one supplied transcript file.
///
/// The path passes the same local, no-follow, regular-file policy as source
/// media. The digest covers exactly the bytes that were parsed.
///
/// # Errors
///
/// Returns a typed path, I/O or content rejection; nothing is imported.
pub fn read_supplied_transcript(
    path: &Path,
) -> Result<SuppliedTranscript, SuppliedTranscriptError> {
    let (file, metadata) = open_source(path).map_err(|error| match error {
        SourceError::NotRegularFile => SuppliedTranscriptError::NotRegularFile,
        SourceError::Io(_) => SuppliedTranscriptError::Io,
        _ => SuppliedTranscriptError::InvalidPath,
    })?;
    if metadata.len() > MAX_SUPPLIED_TRANSCRIPT_BYTES {
        return Err(SuppliedTranscriptError::Rejected(
            TranscriptImportError::new(TranscriptRejection::TooLarge),
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_SUPPLIED_TRANSCRIPT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SuppliedTranscriptError::Io)?;
    let transcript =
        parse_supplied_transcript(&bytes).map_err(SuppliedTranscriptError::Rejected)?;
    let length = u64::try_from(bytes.len()).map_err(|_| SuppliedTranscriptError::Io)?;
    let sidecar = SidecarIdentity::new(sha256_hex(&bytes), length).map_err(|_| {
        SuppliedTranscriptError::Rejected(TranscriptImportError::new(TranscriptRejection::NoCues))
    })?;
    Ok(SuppliedTranscript {
        transcript,
        sidecar,
    })
}

/// Parses supplied transcript bytes under the documented malformed-data policy.
///
/// This is the whole untrusted-input surface of the importer, exposed so it can
/// be exercised directly by property tests and, later, fuzz targets.
///
/// # Errors
///
/// Returns the first [`TranscriptImportError`]; parsing never partially succeeds.
pub fn parse_supplied_transcript(bytes: &[u8]) -> Result<ParsedTranscript, TranscriptImportError> {
    if bytes.len() > usize::try_from(MAX_SUPPLIED_TRANSCRIPT_BYTES).unwrap_or(usize::MAX) {
        return Err(TranscriptImportError::new(TranscriptRejection::TooLarge));
    }
    if bytes.starts_with(b"\xff\xfe")
        || bytes.starts_with(b"\xfe\xff")
        || bytes.starts_with(b"\0\0\xfe\xff")
    {
        return Err(TranscriptImportError::new(
            TranscriptRejection::UnsupportedEncoding,
        ));
    }
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|error| {
        let valid = bytes.get(..error.valid_up_to()).unwrap_or_default();
        TranscriptImportError::at_line(TranscriptRejection::InvalidUtf8, line_of(valid))
    })?;
    let mut lines = Lines::new(text);
    if text.starts_with("WEBVTT") {
        webvtt::parse(&mut lines)
    } else {
        srt::parse(&mut lines)
    }
}

/// 1-based number of the line containing the end of `prefix`.
fn line_of(prefix: &[u8]) -> u32 {
    let mut line = 1_u32;
    let mut previous = 0_u8;
    for byte in prefix {
        if (*byte == b'\n' && previous != b'\r') || *byte == b'\r' {
            line = line.saturating_add(1);
        }
        previous = *byte;
    }
    line
}

/// One validated sidecar line and its 1-based number.
#[derive(Clone, Copy, Debug)]
struct Line<'a> {
    number: u32,
    text: &'a str,
}

impl Line<'_> {
    fn is_blank(self) -> bool {
        self.text.trim().is_empty()
    }
}

/// Streams validated lines, splitting on LF, CRLF or CR.
struct Lines<'a> {
    rest: &'a str,
    number: u32,
    finished: bool,
}

impl<'a> Lines<'a> {
    const fn new(text: &'a str) -> Self {
        Self {
            rest: text,
            number: 0,
            finished: false,
        }
    }

    /// Returns the next line, `None` at the end, or the line's policy violation.
    fn next_line(&mut self) -> Result<Option<Line<'a>>, TranscriptImportError> {
        if self.finished {
            return Ok(None);
        }
        self.number = self.number.saturating_add(1);
        let (text, rest) = if let Some(index) = self.rest.find(['\r', '\n']) {
            let (text, terminator) = self.rest.split_at(index);
            let rest = terminator
                .strip_prefix("\r\n")
                .or_else(|| terminator.strip_prefix('\r'))
                .or_else(|| terminator.strip_prefix('\n'))
                .unwrap_or_default();
            (text, rest)
        } else {
            self.finished = true;
            (self.rest, "")
        };
        self.rest = rest;
        if text.len() > MAX_LINE_BYTES {
            return Err(TranscriptImportError::at_line(
                TranscriptRejection::LineTooLong,
                self.number,
            ));
        }
        if text.chars().any(|character| {
            (character.is_control() && character != '\t')
                || matches!(character, '\u{2028}' | '\u{2029}')
        }) {
            return Err(TranscriptImportError::at_line(
                TranscriptRejection::ControlCharacter,
                self.number,
            ));
        }
        if self.finished && text.is_empty() {
            return Ok(None);
        }
        Ok(Some(Line {
            number: self.number,
            text,
        }))
    }

    /// Returns the next non-blank line, skipping blank separators.
    fn next_non_blank(&mut self) -> Result<Option<Line<'a>>, TranscriptImportError> {
        while let Some(line) = self.next_line()? {
            if !line.is_blank() {
                return Ok(Some(line));
            }
        }
        Ok(None)
    }

    /// Consumes lines up to the next blank line or the end.
    fn skip_block(&mut self) -> Result<(), TranscriptImportError> {
        while let Some(line) = self.next_line()? {
            if line.is_blank() {
                break;
            }
        }
        Ok(())
    }

    /// Collects the payload lines of one cue, bounded by the cue-text limit.
    fn cue_payload(&mut self, timing: Line<'a>) -> Result<Vec<&'a str>, TranscriptImportError> {
        let mut payload = Vec::new();
        let mut bytes = 0_usize;
        while let Some(line) = self.next_line()? {
            if line.is_blank() {
                break;
            }
            bytes = bytes.saturating_add(line.text.len()).saturating_add(1);
            if bytes > MAX_CUE_TEXT_BYTES + 1 {
                return Err(TranscriptImportError::at_line(
                    TranscriptRejection::CueTextTooLong,
                    timing.number,
                ));
            }
            payload.push(line.text);
        }
        Ok(payload)
    }
}

/// Which markup a format recognises in cue payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Markup {
    /// `SubRip`: HTML-like `<b>`, `<i>`, `<u>`, `<font>` tags and `{\...}` override blocks.
    SubRip,
    /// `WebVTT`: cue span tags, timestamp tags and character references.
    WebVtt,
}

/// Accumulates cues and parse warnings for one file.
struct CueCollector {
    format: TranscriptFormat,
    cues: Vec<ImportedCue>,
    warnings: TranscriptWarnings,
    timed: u32,
}

impl CueCollector {
    fn new(format: TranscriptFormat) -> Self {
        Self {
            format,
            cues: Vec::new(),
            warnings: TranscriptWarnings::default(),
            timed: 0,
        }
    }

    /// Adds one timed cue; empty payloads are skipped with a warning.
    fn push(
        &mut self,
        timing_line: Line<'_>,
        timing: CueTiming,
        payload: &[&str],
    ) -> Result<(), TranscriptImportError> {
        self.timed = self.timed.saturating_add(1);
        if usize::try_from(self.timed).map_or(true, |count| count > MAX_TRANSCRIPT_CUES) {
            return Err(TranscriptImportError::at_line(
                TranscriptRejection::TooManyCues,
                timing_line.number,
            ));
        }
        let source = CueSource::new(
            NonZeroU32::new(self.timed).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(timing_line.number).unwrap_or(NonZeroU32::MIN),
        );
        let markup = match self.format {
            TranscriptFormat::Srt => Markup::SubRip,
            TranscriptFormat::WebVtt => Markup::WebVtt,
        };
        let rendered = render_payload(payload, markup);
        if rendered.text.trim().is_empty() {
            self.warnings
                .record(TranscriptWarningKind::EmptyCuesSkipped, source);
            return Ok(());
        }
        // Surrounding whitespace is layout, not content; trimming it here means
        // the original differs from the plain text only by markup.
        let joined = payload.join("\n");
        let original = joined.trim().to_owned();
        if rendered.text != original {
            self.warnings
                .record(TranscriptWarningKind::MarkupRemoved, source);
        }
        let speaker = match rendered.speaker {
            SpeakerAnnotation::None => None,
            SpeakerAnnotation::One(label) => {
                let parsed = SpeakerLabel::parse(label).ok();
                if parsed.is_none() {
                    self.warnings
                        .record(TranscriptWarningKind::SpeakerLabelDiscarded, source);
                }
                parsed
            }
            SpeakerAnnotation::Ambiguous => {
                self.warnings
                    .record(TranscriptWarningKind::SpeakerLabelDiscarded, source);
                None
            }
        };
        let text = CueText::new(rendered.text, original)
            .map_err(|rejection| TranscriptImportError::at_line(rejection, timing_line.number))?;
        self.cues.push(ImportedCue {
            source,
            timing,
            text,
            speaker,
        });
        Ok(())
    }

    fn finish(
        self,
        language: Option<vsift_domain::LanguageTag>,
    ) -> Result<ParsedTranscript, TranscriptImportError> {
        ParsedTranscript::new(self.format, language, self.cues, self.warnings)
    }
}

/// Voice annotations found in one cue.
#[derive(Debug, Eq, PartialEq)]
enum SpeakerAnnotation {
    None,
    One(String),
    Ambiguous,
}

/// Plain text and voice annotation derived from one cue payload.
#[derive(Debug, Eq, PartialEq)]
struct RenderedPayload {
    text: String,
    speaker: SpeakerAnnotation,
}

/// Removes recognised markup and decodes character references.
///
/// A `<` starts a tag only when a `>` closes it on the same line and the tag
/// name has the format's shape; otherwise it is kept as text. Unknown named
/// character references are kept literally. Tabs become spaces so plain text
/// holds no control character other than the line separator. The original
/// payload is kept separately by the caller, so nothing is lost.
fn render_payload(payload: &[&str], markup: Markup) -> RenderedPayload {
    let mut speakers: Vec<String> = Vec::new();
    let mut lines = Vec::with_capacity(payload.len());
    for line in payload {
        let mut plain = String::with_capacity(line.len());
        let mut rest = *line;
        while let Some(character) = rest.chars().next() {
            let (consumed, replacement) = match character {
                '<' => tag(rest, markup, &mut speakers),
                '&' if markup == Markup::WebVtt => character_reference(rest),
                '{' if markup == Markup::SubRip => override_block(rest),
                '\t' => (1, Some(' ')),
                _ => (character.len_utf8(), Some(character)),
            };
            if let Some(replacement) = replacement {
                plain.push(replacement);
            }
            rest = rest.get(consumed..).unwrap_or_default();
        }
        lines.push(plain);
    }
    let text = lines.join("\n").trim().to_owned();
    let speaker = match speakers.as_slice() {
        [] => SpeakerAnnotation::None,
        [first, others @ ..] if others.iter().all(|other| other == first) => {
            SpeakerAnnotation::One(first.clone())
        }
        _ => SpeakerAnnotation::Ambiguous,
    };
    RenderedPayload { text, speaker }
}

/// Handles a `<` at the start of `rest`: returns bytes consumed and any kept character.
fn tag(rest: &str, markup: Markup, speakers: &mut Vec<String>) -> (usize, Option<char>) {
    let Some(close) = window(rest, MAX_MARKUP_BYTES).find('>') else {
        return (1, Some('<'));
    };
    let inner = rest.get(1..close).unwrap_or_default();
    let name_start = inner.strip_prefix('/').unwrap_or(inner);
    let shaped = match name_start.chars().next() {
        Some(first) if first.is_ascii_alphabetic() => true,
        Some(first) if first.is_ascii_digit() => markup == Markup::WebVtt && inner == name_start,
        _ => false,
    };
    if !shaped || inner.contains('<') {
        return (1, Some('<'));
    }
    if markup == Markup::WebVtt && !inner.starts_with('/') {
        let name_end = inner
            .find(|character: char| character == '.' || character.is_whitespace())
            .unwrap_or(inner.len());
        if inner.get(..name_end) == Some("v") {
            let annotation = inner
                .get(name_end..)
                .unwrap_or_default()
                .trim_start_matches(|character: char| !character.is_whitespace())
                .trim();
            if !annotation.is_empty() {
                speakers.push(annotation.to_owned());
            }
        }
    }
    (close + 1, None)
}

/// Decodes a `WebVTT` character reference at the start of `rest`, or keeps `&`.
fn character_reference(rest: &str) -> (usize, Option<char>) {
    let candidate = window(rest, MAX_CHARACTER_REFERENCE_BYTES);
    let Some(end) = candidate.find(';') else {
        return (1, Some('&'));
    };
    let name = candidate.get(1..end).unwrap_or_default();
    let decoded = match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        "lrm" => Some('\u{200e}'),
        "rlm" => Some('\u{200f}'),
        numeric => numeric
            .strip_prefix("#x")
            .or_else(|| numeric.strip_prefix("#X"))
            .map_or_else(
                || {
                    numeric
                        .strip_prefix('#')
                        .and_then(|digits| digits.parse::<u32>().ok())
                },
                |hex| u32::from_str_radix(hex, 16).ok(),
            )
            .and_then(char::from_u32)
            .filter(|character| {
                !character.is_control() && !matches!(character, '\u{2028}' | '\u{2029}')
            }),
    };
    decoded.map_or((1, Some('&')), |character| (end + 1, Some(character)))
}

/// Removes a `SubRip` `{\...}` override block at the start of `rest`, or keeps `{`.
fn override_block(rest: &str) -> (usize, Option<char>) {
    if rest.get(1..2) == Some("\\")
        && let Some(close) = window(rest, MAX_MARKUP_BYTES).find('}')
    {
        return (close + 1, None);
    }
    (1, Some('{'))
}

/// The longest prefix of `text` of at most `maximum` bytes that ends on a
/// character boundary.
fn window(text: &str, maximum: usize) -> &str {
    let mut end = text.len().min(maximum);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default()
}

/// Which timestamp syntax a timing line uses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Clock {
    /// `H:MM:SS,mmm`, hours required (one to four digits).
    SubRip,
    /// `[HH:]MM:SS.mmm`, hours optional (two to four digits when present).
    WebVtt,
}

/// Parses a `start --> end` line; `WebVTT` cue settings after the end are ignored.
fn parse_timing(line: Line<'_>, clock: Clock) -> Result<CueTiming, TranscriptImportError> {
    let invalid_line =
        TranscriptImportError::at_line(TranscriptRejection::InvalidTimingLine, line.number);
    let invalid_time =
        TranscriptImportError::at_line(TranscriptRejection::InvalidTimestamp, line.number);
    let (start, end) = line.text.split_once("-->").ok_or(invalid_line)?;
    let start = start.trim();
    let mut end_tokens = end.split_whitespace();
    let end = end_tokens.next().ok_or(invalid_line)?;
    if start.is_empty() || (clock == Clock::SubRip && end_tokens.next().is_some()) {
        return Err(invalid_line);
    }
    let start = parse_clock(start, clock).ok_or(invalid_time)?;
    let end = parse_clock(end, clock).ok_or(invalid_time)?;
    CueTiming::new(start, end)
        .map_err(|rejection| TranscriptImportError::at_line(rejection, line.number))
}

/// Parses one timestamp into microseconds with checked arithmetic.
fn parse_clock(value: &str, clock: Clock) -> Option<u64> {
    let separator = match clock {
        Clock::SubRip => ',',
        Clock::WebVtt => '.',
    };
    let (whole, fraction) = value.split_once(separator)?;
    let parts: Vec<&str> = whole.split(':').collect();
    let (hours, minutes, seconds) = match (clock, parts.as_slice()) {
        (Clock::SubRip, [hours, minutes, seconds]) if (1..=4).contains(&hours.len()) => {
            (*hours, *minutes, *seconds)
        }
        (Clock::WebVtt, [hours, minutes, seconds]) if (2..=4).contains(&hours.len()) => {
            (*hours, *minutes, *seconds)
        }
        (Clock::WebVtt, [minutes, seconds]) => ("0", *minutes, *seconds),
        _ => return None,
    };
    let hours = digits(hours, hours.len())?;
    let minutes = digits(minutes, 2)?;
    let seconds = digits(seconds, 2)?;
    let millis = digits(fraction, 3)?;
    if minutes >= 60 || seconds >= 60 {
        return None;
    }
    hours
        .checked_mul(60)?
        .checked_add(minutes)?
        .checked_mul(60)?
        .checked_add(seconds)?
        .checked_mul(1_000)?
        .checked_add(millis)?
        .checked_mul(1_000)
}

/// Parses exactly `length` ASCII digits.
fn digits(value: &str, length: usize) -> Option<u64> {
    if value.len() != length || length == 0 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    text
}
