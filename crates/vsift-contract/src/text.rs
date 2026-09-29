//! Bounded handling of untrusted text that reaches public output.
//!
//! Two rules live here, so every host places untrusted text in its output the
//! same way:
//!
//! - [`sanitize_untrusted_text`] replaces control characters, which could
//!   forge records or drive a terminal;
//! - [`render_hidden_characters`] makes the characters of
//!   [`is_hidden_character`] visible as `<U+XXXX>` notation, for the
//!   `display_text` a reader quotes (ADR 0008, note of 2026-09-29).

/// Largest provider-reported detail, such as a probed version line, carried in a
/// public response.
///
/// Provider text is untrusted and unbounded; a small fixed budget keeps a hostile
/// or verbose executable from inflating results or hiding content in them.
pub const MAX_PROVIDER_DETAIL_BYTES: usize = 240;

/// The Unicode version whose character data [`is_hidden_character`] follows.
pub const HIDDEN_CHARACTER_UNICODE_VERSION: &str = "16.0.0";

/// Largest number of bytes [`render_hidden_characters`] writes for one input
/// byte: a two-byte character such as U+00AD becomes the eight bytes of
/// `<U+00AD>`. Three- and four-byte characters grow less.
pub const HIDDEN_CHARACTER_GROWTH: usize = 4;

/// Code point ranges (inclusive, ascending, disjoint) of the hidden
/// characters, from Unicode 16.0.0.
///
/// The set is the union of:
///
/// - general category **Cf** (format): bidirectional controls, zero-width
///   characters, joiners, the byte-order mark, invisible operators, tag
///   characters, and the prepended number signs and annotation marks whose
///   effect is on layout rather than content;
/// - the derived property **`Default_Ignorable_Code_Point`**: characters a
///   renderer shows as nothing when it does not support them, such as the
///   combining grapheme joiner, the Hangul fillers, the Mongolian and general
///   variation selectors (a known channel for smuggling bytes into text) and
///   the ranges Unicode reserves for future ones;
/// - general categories **Zl** and **Zp** (U+2028, U+2029), which some
///   renderers show as a line break and others as nothing. Import already
///   rejects them, like every other control character; they are listed so the
///   rule does not depend on that.
///
/// Characters with a visible glyph or a visible width are not in the set:
/// spaces of any width (Zs), private-use characters and unassigned code
/// points outside the reserved ranges above, which render as a visible
/// replacement glyph, and noncharacters. Control characters (Cc) are not in
/// it either: import rejects them and [`sanitize_untrusted_text`] replaces
/// any that remain.
const HIDDEN_CHARACTER_RANGES: [(u32, u32); 25] = [
    (0x00AD, 0x00AD),   // soft hyphen (Cf)
    (0x034F, 0x034F),   // combining grapheme joiner (ignorable)
    (0x0600, 0x0605),   // Arabic number signs (Cf)
    (0x061C, 0x061C),   // Arabic letter mark (Cf, bidirectional)
    (0x06DD, 0x06DD),   // Arabic end of ayah (Cf)
    (0x070F, 0x070F),   // Syriac abbreviation mark (Cf)
    (0x0890, 0x0891),   // Arabic pound and piastre marks above (Cf)
    (0x08E2, 0x08E2),   // Arabic disputed end of ayah (Cf)
    (0x115F, 0x1160),   // Hangul choseong and jungseong fillers (ignorable)
    (0x17B4, 0x17B5),   // Khmer inherent vowels (ignorable)
    (0x180B, 0x180F),   // Mongolian variation selectors and vowel separator
    (0x200B, 0x200F),   // zero-width space, joiners, left-to-right and right-to-left marks
    (0x2028, 0x202E),   // line and paragraph separators, bidirectional embeddings and overrides
    (0x2060, 0x206F), // word joiner, invisible operators, bidirectional isolates, deprecated formats
    (0x3164, 0x3164), // Hangul filler (ignorable)
    (0xFE00, 0xFE0F), // variation selectors (ignorable)
    (0xFEFF, 0xFEFF), // zero-width no-break space, byte-order mark (Cf)
    (0xFFA0, 0xFFA0), // halfwidth Hangul filler (ignorable)
    (0xFFF0, 0xFFFB), // reserved ignorables and interlinear annotation marks
    (0x110BD, 0x110BD), // Kaithi number sign (Cf)
    (0x110CD, 0x110CD), // Kaithi number sign above (Cf)
    (0x13430, 0x1343F), // Egyptian hieroglyph format controls (Cf)
    (0x1BCA0, 0x1BCA3), // shorthand format controls (Cf)
    (0x1D173, 0x1D17A), // musical symbol format controls (Cf)
    (0xE0000, 0xE0FFF), // tags and variation selectors supplement, and their reserved plane block
];

/// Converts untrusted provider or parser text into bounded single-line,
/// terminal-safe text.
///
/// Every control character, including newlines and escape sequences, becomes
/// U+FFFD so the text cannot forge extra records in a line-oriented stream or
/// drive a terminal. The result never exceeds `maximum_bytes` and is truncated on
/// a character boundary. Hosts use the same function for human output so JSON and
/// text agree on what a provider said.
#[must_use]
pub fn sanitize_untrusted_text(value: &str, maximum_bytes: usize) -> String {
    let mut result = String::with_capacity(value.len().min(maximum_bytes));
    for character in value.chars() {
        let replacement = if character.is_control() {
            '\u{fffd}'
        } else {
            character
        };
        if result.len() + replacement.len_utf8() > maximum_bytes {
            break;
        }
        result.push(replacement);
    }
    result
}

/// Whether `character` hides or reorders text instead of showing a glyph:
/// a format character, a default-ignorable code point, or a line or
/// paragraph separator (the set is documented on the table it reads).
///
/// A reader cannot see these characters, yet they can reverse the order in
/// which text is displayed (Trojan Source), split or join words, or carry
/// data invisibly. `display_text` shows each of them as notation.
#[must_use]
pub fn is_hidden_character(character: char) -> bool {
    let code = u32::from(character);
    HIDDEN_CHARACTER_RANGES
        .iter()
        .any(|&(first, last)| (first..=last).contains(&code))
}

/// Returns `value` with every [hidden character](is_hidden_character)
/// written as visible `<U+XXXX>` notation: `U+`, the code point in uppercase
/// hexadecimal with at least four digits, between angle brackets.
///
/// Everything else is kept exactly, including line separators (`\n`) and
/// text that already reads `<U+202E>`: nothing is escaped, so rendering is
/// idempotent and never escapes its own output twice. The price is that
/// `<U+202E>` in the result may also have been written literally in the
/// source; the unrendered text says which. The result is at most
/// [`HIDDEN_CHARACTER_GROWTH`] times as long as `value`.
#[must_use]
pub fn render_hidden_characters(value: &str) -> String {
    if !value.chars().any(is_hidden_character) {
        return value.to_owned();
    }
    let mut rendered = String::with_capacity(value.len() * 2);
    for character in value.chars() {
        if is_hidden_character(character) {
            push_notation(&mut rendered, u32::from(character));
        } else {
            rendered.push(character);
        }
    }
    rendered
}

/// Appends `<U+XXXX>` for `code`: uppercase hexadecimal, at least four
/// digits, as many as a scalar value needs (at most six).
///
/// The digits are written directly rather than through `write!`, whose
/// `fmt::Result` a `String` can never fail with but a caller would still
/// have to discard.
fn push_notation(rendered: &mut String, code: u32) {
    let digits = match code {
        0..=0xFFFF => 4,
        0x1_0000..=0xF_FFFF => 5,
        _ => 6,
    };
    rendered.push_str("<U+");
    for shift in (0..digits).rev() {
        let nibble = (code >> (shift * 4)) & 0xF;
        if let Some(digit) = char::from_digit(nibble, 16) {
            rendered.push(digit.to_ascii_uppercase());
        }
    }
    rendered.push('>');
}

#[cfg(test)]
mod tests {
    use super::{
        HIDDEN_CHARACTER_GROWTH, HIDDEN_CHARACTER_RANGES, is_hidden_character,
        render_hidden_characters, sanitize_untrusted_text,
    };

    #[test]
    fn terminal_controls_are_replaced_before_human_display() {
        let safe = sanitize_untrusted_text("ok\u{1b}]8;;file:///secret\u{7}bad\n", 128);

        assert!(!safe.contains('\u{1b}'));
        assert!(!safe.contains('\u{7}'));
        assert!(!safe.contains('\n'));
        assert!(safe.contains('\u{fffd}'));
    }

    #[test]
    fn diagnostic_size_is_bounded_before_writing() {
        let value = "a".repeat(10_000);
        let safe = sanitize_untrusted_text(&value, 4_095);

        assert_eq!(safe.len(), 4_095);
    }

    #[test]
    fn truncation_never_splits_a_character() {
        let safe = sanitize_untrusted_text("ab\u{e9}", 3);

        assert_eq!(safe, "ab");
    }

    #[test]
    fn the_table_is_ascending_disjoint_and_holds_only_scalar_values() {
        let mut previous_last: Option<u32> = None;
        for &(first, last) in &HIDDEN_CHARACTER_RANGES {
            assert!(first <= last, "{first:X}..{last:X} is reversed");
            assert!(
                previous_last.is_none_or(|previous| first > previous + 1),
                "{first:X} overlaps or touches the previous range"
            );
            assert!(char::from_u32(first).is_some() && char::from_u32(last).is_some());
            previous_last = Some(last);
        }
    }

    /// Every character named in the ADR 0008 note of 2026-09-29, by group.
    #[test]
    fn the_documented_characters_are_hidden() {
        let groups: [(&str, &[u32]); 8] = [
            (
                "bidirectional controls",
                &[
                    0x061C, 0x200E, 0x200F, 0x202A, 0x202B, 0x202C, 0x202D, 0x202E, 0x2066, 0x2067,
                    0x2068, 0x2069,
                ],
            ),
            (
                "zero-width and joiners",
                &[0x200B, 0x200C, 0x200D, 0x2060, 0xFEFF, 0x034F],
            ),
            ("invisible operators", &[0x2061, 0x2062, 0x2063, 0x2064]),
            ("soft hyphen", &[0x00AD]),
            ("separators", &[0x2028, 0x2029]),
            (
                "fillers",
                &[0x115F, 0x1160, 0x3164, 0xFFA0, 0x17B4, 0x17B5, 0x180E],
            ),
            (
                "variation selectors",
                &[0xFE00, 0xFE0F, 0x180B, 0x180F, 0xE0100, 0xE01EF],
            ),
            (
                "tags and other format",
                &[
                    0xE0001, 0xE0020, 0xE0041, 0xE007F, 0x0600, 0x06DD, 0x070F, 0x08E2, 0xFFF9,
                    0xFFFB, 0x110BD, 0x13430, 0x1BCA3, 0x1D173,
                ],
            ),
        ];
        for (group, codes) in groups {
            for &code in codes {
                let character = char::from_u32(code);
                assert!(
                    character.is_some_and(is_hidden_character),
                    "{group}: U+{code:04X} is not hidden"
                );
            }
        }
    }

    #[test]
    fn visible_text_spaces_and_controls_are_not_hidden() {
        for character in [
            'a',
            'Z',
            '0',
            ' ',
            '<',
            '+',
            '\u{e9}',
            '\u{5d0}',
            '\u{627}',
            '\u{4e2d}',
            '\u{1f600}',
            '\u{a0}',
            '\u{2003}',
            '\u{202f}',
            '\u{3000}',
            '\u{fffd}',
            '\u{e000}',
            '\u{fffe}',
            '\u{2800}',
            '\n',
            '\t',
            '\u{1b}',
            '\u{85}',
        ] {
            assert!(
                !is_hidden_character(character),
                "U+{:04X} is hidden",
                u32::from(character)
            );
        }
    }

    #[test]
    fn hidden_characters_render_as_uppercase_notation_of_at_least_four_digits() {
        assert_eq!(
            render_hidden_characters("a\u{202E}b\u{200b}c\u{ad}d\u{e0041}"),
            "a<U+202E>b<U+200B>c<U+00AD>d<U+E0041>"
        );
        assert_eq!(
            render_hidden_characters("two\nlines \u{2066}x\u{2069}"),
            "two\nlines <U+2066>x<U+2069>"
        );
    }

    #[test]
    fn plain_text_and_literal_notation_are_unchanged() {
        for text in [
            "Dialog R-17 is displayed now.",
            "caf\u{e9} \u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{1f600}",
            "The text literally reads <U+202E> and <U+",
            "<U+200B><u+feff><U+ZZZZ>",
        ] {
            assert_eq!(render_hidden_characters(text), text);
        }
    }

    #[test]
    fn rendering_is_idempotent_and_leaves_nothing_hidden() {
        for text in [
            "Status: \u{202E}DELIAF\u{202C} build\u{200B} pass\u{200D}ed",
            "<U+202E> and \u{202E}",
            "\u{fe0f}\u{e0100}\u{2028}\u{2029}",
        ] {
            let once = render_hidden_characters(text);
            assert!(!once.chars().any(is_hidden_character), "{once}");
            assert_eq!(render_hidden_characters(&once), once);
        }
    }

    #[test]
    fn notation_has_four_to_six_uppercase_digits() {
        for (code, expected) in [
            (0xAD, "<U+00AD>"),
            (0xFEFF, "<U+FEFF>"),
            (0xE_0041, "<U+E0041>"),
            (0x10_FFFF, "<U+10FFFF>"),
        ] {
            let mut rendered = String::new();
            super::push_notation(&mut rendered, code);
            assert_eq!(rendered, expected);
        }
    }

    #[test]
    fn growth_is_bounded() {
        for character in ['\u{ad}', '\u{200b}', '\u{e0041}'] {
            let text = character.to_string().repeat(512);
            let rendered = render_hidden_characters(&text);
            assert!(rendered.len() <= text.len() * HIDDEN_CHARACTER_GROWTH);
        }
        assert_eq!(render_hidden_characters("\u{ad}").len(), 8);
    }
}
