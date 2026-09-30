//! The one builder every human renderer writes through.
//!
//! [`TerminalText`] is the only way text reaches a terminal in human mode:
//! renderers never format into a `String` of their own. It holds three rules
//! that no renderer can bypass:
//!
//! - **No control character but its own line breaks.** Every character
//!   pushed is checked; a control character (C0, C1, `ESC`, `DEL`, and the
//!   line break of a pushed value) becomes U+FFFD, so neither an ANSI or OSC
//!   sequence (an OSC-8 link, a colour) nor a forged line can be written.
//!   Line breaks come only from [`TerminalText::end_line`] and from the line
//!   structure of a quoted block, each of which starts with its quote prefix.
//! - **No hidden character.** Every character of `display_text`'s hidden set
//!   ([`vsift_contract::is_hidden_character`]) is written as `<U+XXXX>`, with
//!   [`vsift_contract::terminal_safe_text`], so nothing can reorder or hide
//!   what the reader sees (Trojan Source).
//! - **Bounded.** The whole text stays within the result budget, and a quoted
//!   line of untrusted text continues on a new quoted line rather than
//!   growing without bound.
//!
//! Untrusted text has one entry, [`TerminalText::push_untrusted`], which
//! takes only a [`DisplayText`]: a `display_text` or `display_label` read
//! from a result, or raw text rendered with the same rule. Evidence text is
//! therefore always quoted in its display form, never from `text`, `label`
//! or `original_text`.
//!
//! A delivered file path has its own entry, [`TerminalText::push_path_line`]
//! (P13 PR 2b): it is not evidence, but it holds the session root the user
//! chose, so it passes the same character rules, and it stands alone on its
//! line so it can be copied whole. The builder reports whether the path was
//! written exactly, so the renderer can say when it was not.

use std::fmt;

use serde::Deserialize;
use vsift_contract::{
    is_hidden_character, render_hidden_characters, sanitize_untrusted_text, terminal_safe_text,
};

/// Largest human result, including its last line break: the budget of a
/// `--json` result (`docs/contracts/cli-v1.md`, output protocol).
pub(crate) const MAX_RESULT_BYTES: usize = 1_048_576;

/// Largest human failure on stderr, including its last line break. Each
/// line stays within the protocol's 4,096-byte diagnostic; a parser
/// explanation quoted over several lines is the only long part, and at most
/// 4 KiB of it is read ([`DisplayText::render_lines`]).
pub(crate) const MAX_FAILURE_BYTES: usize = 65_536;

/// Most bytes of untrusted text on one line, after its quote prefix. A longer
/// line continues on the next quoted line; an inline value is cut there.
///
/// A diagnostic line is at most 4,096 bytes; this leaves room for a label or
/// prefix of up to 96 bytes on the same line.
pub(crate) const MAX_UNTRUSTED_LINE_BYTES: usize = 4_000;

/// Largest raw text [`DisplayText::render`] reads before rendering; the
/// rendered form can grow by up to `HIDDEN_CHARACTER_GROWTH`.
const MAX_RENDERED_SOURCE_BYTES: usize = 4_096;

/// Written after inline untrusted text that did not fit on its line.
const CUT_MARKER: &str = " [cut]";

/// Untrusted text in its display form: hidden characters already written as
/// `<U+XXXX>` (`display_text`'s rule, ADR 0008 note of 2026-09-29).
///
/// A renderer obtains one only by reading a `display_text` or
/// `display_label` field of a result (the type deserializes from exactly
/// that string), or through [`DisplayText::render`] for untrusted text that
/// has no display field, such as the parser's explanation of a rejected
/// command line or a provider's version line.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(transparent)]
pub(crate) struct DisplayText(String);

impl DisplayText {
    /// Renders raw untrusted text with `display_text`'s rule, after bounding
    /// it (at most 4 KiB of it is read) and replacing its control
    /// characters, line breaks included.
    pub(crate) fn render(raw: &str) -> Self {
        Self(render_hidden_characters(&sanitize_untrusted_text(
            raw,
            MAX_RENDERED_SOURCE_BYTES,
        )))
    }

    /// Renders raw untrusted text that has its own lines, such as the
    /// parser's explanation: each line as [`DisplayText::render`] does, the
    /// line breaks kept for [`Placement::Quoted`] to quote one by one.
    pub(crate) fn render_lines(raw: &str) -> Self {
        let bounded = &raw[..floor_char_boundary(raw, MAX_RENDERED_SOURCE_BYTES)];
        let lines: Vec<String> = bounded
            .split('\n')
            .map(|line| {
                // A control character of one byte becomes U+FFFD, three
                // bytes: this budget keeps every character of the line.
                let whole_line = line.len().saturating_mul(3);
                render_hidden_characters(&sanitize_untrusted_text(line, whole_line))
            })
            .collect();
        Self(lines.join("\n"))
    }
}

/// The largest index at most `index` that is a character boundary of `text`.
fn floor_char_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    (0..=index)
        .rev()
        .find(|&candidate| text.is_char_boundary(candidate))
        .unwrap_or(0)
}

/// Where [`TerminalText::push_untrusted`] places untrusted text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Placement {
    /// On the current line, after its label: line breaks become U+FFFD and
    /// text past [`MAX_UNTRUSTED_LINE_BYTES`] is cut, marked `[cut]`.
    Inline,
    /// As a quoted block: every line of the text on a line of its own after
    /// the quote prefix, a long line continuing on the next quoted line.
    Quoted,
}

/// The prefix of every line of a quoted block. No other line starts with
/// it, so a reader can always tell quoted evidence from `VSift`'s own text.
pub(crate) const QUOTE_PREFIX: &str = "  | ";

/// What [`TerminalText::push_path_line`] wrote for a path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PathLine {
    /// The path, character for character: it can be copied from the line.
    Exact,
    /// The path with a control character as U+FFFD or a hidden character as
    /// `<U+XXXX>`: readable, but not the path itself.
    Altered,
    /// A fixed statement instead of a path longer than
    /// [`MAX_UNTRUSTED_LINE_BYTES`], which a line cannot hold whole and which
    /// is never cut.
    TooLong,
}

/// Written instead of a path that does not fit on one line.
const PATH_TOO_LONG: &str = "(a path longer than 4000 bytes; read it with --json)";

/// The human text of one result or failure, built by [`TerminalText`].
///
/// Only the builder creates one, so a writer that takes it writes nothing
/// the builder's rules did not check.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct RenderedText(String);

impl RenderedText {
    /// The complete text, ending with a line break.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// The text was larger than its budget; nothing of it is written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TooLarge;

impl fmt::Display for TooLarge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the human result exceeds the output byte budget")
    }
}

/// Builds terminal-safe, bounded human text (see the module documentation).
#[derive(Debug)]
pub(crate) struct TerminalText {
    text: String,
    /// Byte index where the current line starts.
    line_start: usize,
    limit: usize,
    exceeded: bool,
}

impl TerminalText {
    /// A builder for a result on stdout, within the result budget.
    pub(crate) fn result() -> Self {
        Self::within(MAX_RESULT_BYTES)
    }

    /// A builder for a failure on stderr, within the failure budget.
    pub(crate) fn failure() -> Self {
        Self::within(MAX_FAILURE_BYTES)
    }

    fn within(limit: usize) -> Self {
        Self {
            text: String::with_capacity(limit.min(4_096)),
            line_start: 0,
            limit,
            exceeded: false,
        }
    }

    /// Writes fixed words of `VSift`'s own.
    pub(crate) fn push_fixed(&mut self, words: &'static str) -> &mut Self {
        self.push_checked(words);
        self
    }

    /// Writes a value a result carries that is not evidence: an identifier,
    /// an enum identifier, a timestamp, a digest, a URL of the reviewed
    /// catalogue or a fixed-prose message.
    ///
    /// It is never cut, so an identifier is always whole; it still passes
    /// the control and hidden-character rules, so even a value that broke
    /// its contract could not drive the terminal.
    pub(crate) fn push_value(&mut self, value: &str) -> &mut Self {
        self.push_checked(value);
        self
    }

    /// Writes an unsigned number in decimal.
    pub(crate) fn push_unsigned(&mut self, number: u64) -> &mut Self {
        self.push_checked(&number.to_string());
        self
    }

    /// Writes a signed number in decimal.
    pub(crate) fn push_signed(&mut self, number: i64) -> &mut Self {
        self.push_checked(&number.to_string());
        self
    }

    /// Writes untrusted text in its display form: the only entry for text
    /// that came from evidence, a provider or a command line.
    pub(crate) fn push_untrusted(&mut self, text: &DisplayText, placement: Placement) -> &mut Self {
        match placement {
            Placement::Inline => self.push_inline(&text.0),
            Placement::Quoted => self.push_quoted(&text.0),
        }
        self
    }

    /// Writes a delivered file path on a line of its own, after `indent`:
    /// the current line is ended first, and the path's line is ended after.
    ///
    /// The path is never cut, since a cut path names another file: one
    /// longer than [`MAX_UNTRUSTED_LINE_BYTES`] once rendered is replaced by
    /// a fixed statement. Its characters pass the control and
    /// hidden-character rules; the answer says whether the line holds the
    /// path exactly.
    pub(crate) fn push_path_line(&mut self, indent: &'static str, path: &str) -> PathLine {
        if self.line_start != self.text.len() {
            self.end_line();
        }
        let mut rendered = String::with_capacity(path.len());
        let mut piece = String::with_capacity(12);
        let mut altered = false;
        for character in path.chars() {
            safe_piece(character, &mut piece);
            altered |= piece.chars().ne(std::iter::once(character));
            rendered.push_str(&piece);
        }
        self.push_checked(indent);
        let written = if rendered.len() > MAX_UNTRUSTED_LINE_BYTES {
            self.push_raw(PATH_TOO_LONG);
            PathLine::TooLong
        } else {
            self.push_raw(&rendered);
            if altered {
                PathLine::Altered
            } else {
                PathLine::Exact
            }
        };
        self.end_line();
        written
    }

    /// Ends the current line.
    pub(crate) fn end_line(&mut self) -> &mut Self {
        self.push_raw("\n");
        self.line_start = self.text.len();
        self
    }

    /// Writes an empty line, unless the text is empty or already ends with
    /// one; it separates sections.
    pub(crate) fn blank_line(&mut self) -> &mut Self {
        if !self.text.is_empty() && !self.text.ends_with("\n\n") {
            if self.line_start != self.text.len() {
                self.end_line();
            }
            self.end_line();
        }
        self
    }

    /// Finishes the text, ending its last line; a blank line at the end is
    /// dropped.
    ///
    /// # Errors
    ///
    /// [`TooLarge`] when the text exceeded its budget; nothing of it may be
    /// written then, as with a `--json` result.
    pub(crate) fn finish(mut self) -> Result<RenderedText, TooLarge> {
        if self.line_start != self.text.len() {
            self.end_line();
        }
        while self.text.ends_with("\n\n") {
            self.text.pop();
        }
        if self.exceeded {
            Err(TooLarge)
        } else {
            Ok(RenderedText(self.text))
        }
    }

    /// Bytes on the current line so far.
    fn line_bytes(&self) -> usize {
        self.text.len() - self.line_start
    }

    fn push_inline(&mut self, text: &str) {
        let budget = self.line_bytes().saturating_add(MAX_UNTRUSTED_LINE_BYTES);
        let mut piece = String::with_capacity(12);
        for character in text.chars() {
            safe_piece(character, &mut piece);
            if self.line_bytes() + piece.len() > budget {
                self.push_raw(CUT_MARKER);
                return;
            }
            self.push_raw(&piece);
        }
    }

    fn push_quoted(&mut self, text: &str) {
        if self.line_start != self.text.len() {
            self.end_line();
        }
        let mut piece = String::with_capacity(12);
        for line in text.split('\n') {
            self.push_raw(QUOTE_PREFIX);
            let mut written = 0_usize;
            for character in line.chars() {
                safe_piece(character, &mut piece);
                if written > 0 && written + piece.len() > MAX_UNTRUSTED_LINE_BYTES {
                    self.end_line();
                    self.push_raw(QUOTE_PREFIX);
                    written = 0;
                }
                self.push_raw(&piece);
                written += piece.len();
            }
            self.end_line();
        }
    }

    fn push_checked(&mut self, value: &str) {
        let mut piece = String::with_capacity(12);
        for character in value.chars() {
            safe_piece(character, &mut piece);
            self.push_raw(&piece);
        }
    }

    /// Appends already-checked text, or records that the budget is spent.
    fn push_raw(&mut self, checked: &str) {
        if self.exceeded || self.text.len() + checked.len() > self.limit {
            self.exceeded = true;
            return;
        }
        self.text.push_str(checked);
    }
}

/// The terminal-safe form of one character, in `piece`: the character
/// itself, U+FFFD for a control character, or `<U+XXXX>` for a hidden one
/// (both from [`terminal_safe_text`], the contract's rule).
fn safe_piece(character: char, piece: &mut String) {
    piece.clear();
    if character.is_control() || is_hidden_character(character) {
        let mut buffer = [0_u8; 4];
        piece.push_str(&terminal_safe_text(character.encode_utf8(&mut buffer), 16));
    } else {
        piece.push(character);
    }
}

#[cfg(test)]
mod tests {
    use proptest::{
        collection,
        prelude::{Strategy, TestCaseError, prop_assert, prop_assert_eq, proptest},
        sample,
    };
    use vsift_contract::is_hidden_character;

    use super::{
        DisplayText, MAX_UNTRUSTED_LINE_BYTES, PathLine, Placement, QUOTE_PREFIX, TerminalText,
        TooLarge,
    };

    /// Characters a hostile string is built from: controls of every kind,
    /// hidden characters, the separators, escape introducers and plain text.
    const HOSTILE: &[char] = &[
        '\u{0}',
        '\u{7}',
        '\u{8}',
        '\t',
        '\n',
        '\r',
        '\u{b}',
        '\u{c}',
        '\u{1b}',
        '\u{7f}',
        '\u{80}',
        '\u{85}',
        '\u{9b}',
        '\u{9d}',
        '\u{9c}',
        '\u{ad}',
        '\u{34f}',
        '\u{61c}',
        '\u{200b}',
        '\u{200d}',
        '\u{200e}',
        '\u{2028}',
        '\u{2029}',
        '\u{202a}',
        '\u{202e}',
        '\u{2066}',
        '\u{2069}',
        '\u{feff}',
        '\u{fe0f}',
        '\u{e0041}',
        '\u{e0100}',
        '\u{fff9}',
        ']',
        '[',
        ';',
        '8',
        'a',
        'Z',
        ' ',
        '<',
        '+',
        '>',
        '\u{e9}',
        '\u{5d0}',
        '\u{1f600}',
    ];

    fn hostile_string() -> impl Strategy<Value = String> {
        collection::vec(sample::select(HOSTILE), 0..512)
            .prop_map(|characters| characters.into_iter().collect())
    }

    /// Every line of `text` but the line breaks the builder wrote is free of
    /// controls and hidden characters.
    fn assert_terminal_safe(text: &str) {
        for character in text.chars() {
            assert!(
                character == '\n' || !character.is_control(),
                "control U+{:04X} in {text:?}",
                u32::from(character)
            );
            assert!(
                !is_hidden_character(character),
                "hidden U+{:04X} in {text:?}",
                u32::from(character)
            );
        }
    }

    proptest! {
        /// SEC-T02 (P13 PR 2a): whatever is pushed, and however, the text
        /// never holds a control character other than its own line breaks,
        /// nor a hidden character, and every quoted line carries the prefix.
        #[test]
        fn output_never_holds_a_control_or_hidden_character(
            value in hostile_string(),
            inline in hostile_string(),
            quoted in hostile_string(),
            lines in hostile_string(),
        ) {
            let mut text = TerminalText::result();
            text.push_fixed("Label: ")
                .push_value(&value)
                .push_fixed(" ")
                .push_untrusted(&DisplayText(inline.clone()), Placement::Inline)
                .push_untrusted(&DisplayText::render(&inline), Placement::Inline)
                .end_line()
                .push_untrusted(&DisplayText(quoted.clone()), Placement::Quoted)
                .push_untrusted(&DisplayText::render_lines(&lines), Placement::Quoted);
            let rendered = text.finish().map_err(|_| TestCaseError::fail("too large"))?;
            let output = rendered.as_str();
            assert_terminal_safe(output);
            prop_assert!(output.ends_with('\n'));
            // The first line is the label line; every later line is quoted.
            for line in output.lines().skip(1) {
                prop_assert!(line.starts_with(QUOTE_PREFIX), "{line:?}");
            }
        }

        /// SEC-T02 (P13 PR 2b): a delivered path, whatever it holds, is
        /// one terminal-safe line of its own after its indent, never cut;
        /// the builder says whether that line is the path exactly.
        #[test]
        fn a_path_stands_alone_on_one_safe_line(
            label in hostile_string(),
            path in hostile_string(),
            long in collection::vec(sample::select(HOSTILE), 0..4_200),
        ) {
            for path in [path, long.into_iter().collect::<String>()] {
                let mut text = TerminalText::result();
                text.push_fixed("File: ").push_value(&label);
                let written = text.push_path_line("    ", &path);
                text.push_fixed("After");
                let rendered = text.finish().map_err(|_| TestCaseError::fail("too large"))?;
                let output = rendered.as_str();
                assert_terminal_safe(output);
                let lines: Vec<&str> = output.lines().collect();
                prop_assert_eq!(lines.len(), 3, "{:?}", output);
                let shown = lines[1].strip_prefix("    ");
                prop_assert!(shown.is_some(), "{:?}", lines[1]);
                let shown = shown.unwrap_or_default();
                prop_assert!(shown.len() <= MAX_UNTRUSTED_LINE_BYTES);
                prop_assert_eq!(lines[2], "After");
                let hostile = path
                    .chars()
                    .any(|character| character.is_control() || is_hidden_character(character));
                match written {
                    PathLine::Exact => {
                        prop_assert!(!hostile);
                        prop_assert_eq!(shown, path.as_str());
                    }
                    PathLine::Altered => prop_assert!(hostile),
                    PathLine::TooLong => prop_assert_eq!(shown, super::PATH_TOO_LONG),
                }
            }
        }
    }

    #[test]
    fn a_path_is_exact_altered_or_too_long() -> Result<(), TooLarge> {
        let mut text = TerminalText::result();
        text.push_fixed("Files:");
        let exact = text.push_path_line("  ", r"\\?\C:\root\artifact.png");
        let altered = text.push_path_line("  ", "/root/a\u{202e}b\nc.png");
        let long = text.push_path_line("  ", &"p".repeat(MAX_UNTRUSTED_LINE_BYTES + 1));
        assert_eq!(
            (exact, altered, long),
            (PathLine::Exact, PathLine::Altered, PathLine::TooLong)
        );
        assert_eq!(
            text.finish()?.as_str(),
            "Files:\n  \\\\?\\C:\\root\\artifact.png\n  /root/a<U+202E>b\u{fffd}c.png\n  \
             (a path longer than 4000 bytes; read it with --json)\n"
        );
        Ok(())
    }

    #[test]
    fn values_keep_visible_text_and_replace_controls_and_hidden_characters() -> Result<(), TooLarge>
    {
        let mut text = TerminalText::result();
        text.push_value("ses_0123\u{1b}]8;;x\u{7}\u{202e}ab\ncd");
        assert_eq!(
            text.finish()?.as_str(),
            "ses_0123\u{fffd}]8;;x\u{fffd}<U+202E>ab\u{fffd}cd\n"
        );
        Ok(())
    }

    #[test]
    fn a_quoted_block_quotes_each_line_and_continues_long_ones() -> Result<(), TooLarge> {
        let mut text = TerminalText::result();
        text.push_fixed("Evidence:");
        let long = "x".repeat(MAX_UNTRUSTED_LINE_BYTES + 5);
        text.push_untrusted(
            &DisplayText(format!("first\u{1b}[31m\nsecond\n{long}")),
            Placement::Quoted,
        );
        let rendered = text.finish()?;
        let lines: Vec<&str> = rendered.as_str().lines().collect();
        assert_eq!(lines[0], "Evidence:");
        assert_eq!(lines[1], "  | first\u{fffd}[31m");
        assert_eq!(lines[2], "  | second");
        assert_eq!(
            lines[3].len(),
            QUOTE_PREFIX.len() + MAX_UNTRUSTED_LINE_BYTES
        );
        assert_eq!(lines[4], "  | xxxxx");
        Ok(())
    }

    #[test]
    fn a_long_line_never_splits_a_notation() -> Result<(), TooLarge> {
        let mut text = TerminalText::result();
        let hidden = "\u{202e}".repeat(MAX_UNTRUSTED_LINE_BYTES);
        text.push_untrusted(&DisplayText(hidden), Placement::Quoted);
        let rendered = text.finish()?;
        for line in rendered.as_str().lines() {
            let quoted = line.strip_prefix(QUOTE_PREFIX).unwrap_or(line);
            assert!(quoted.len() <= MAX_UNTRUSTED_LINE_BYTES);
            assert_eq!(quoted.len() % "<U+202E>".len(), 0, "{quoted}");
        }
        Ok(())
    }

    #[test]
    fn inline_text_is_cut_and_marked_at_the_line_budget() -> Result<(), TooLarge> {
        let mut text = TerminalText::result();
        text.push_fixed("Detail: ").push_untrusted(
            &DisplayText("y".repeat(MAX_UNTRUSTED_LINE_BYTES * 2)),
            Placement::Inline,
        );
        let rendered = text.finish()?;
        let line = rendered.as_str().trim_end_matches('\n');
        assert!(line.ends_with(" [cut]"));
        assert_eq!(
            line.len(),
            "Detail: ".len() + MAX_UNTRUSTED_LINE_BYTES + " [cut]".len()
        );
        Ok(())
    }

    #[test]
    fn text_over_its_budget_is_refused_whole() {
        let mut text = TerminalText::failure();
        for _ in 0..17 {
            text.push_value(&"z".repeat(4_000)).end_line();
        }
        assert_eq!(text.finish(), Err(TooLarge));
    }

    #[test]
    fn rendering_raw_text_applies_the_display_rule() {
        assert_eq!(
            DisplayText::render("a\u{202e}b\nc\u{1b}"),
            DisplayText("a<U+202E>b\u{fffd}c\u{fffd}".to_owned())
        );
        assert_eq!(
            DisplayText::render_lines("a\u{202e}b\nc\u{1b}"),
            DisplayText("a<U+202E>b\nc\u{fffd}".to_owned())
        );
    }

    #[test]
    fn blank_lines_separate_sections_once() -> Result<(), TooLarge> {
        let mut text = TerminalText::result();
        text.blank_line().push_fixed("a").blank_line().blank_line();
        text.push_fixed("b").blank_line();
        assert_eq!(text.finish()?.as_str(), "a\n\nb\n");
        Ok(())
    }
}
