//! Bounded handling of untrusted text that reaches public output.

/// Largest provider-reported detail, such as a probed version line, carried in a
/// public response.
///
/// Provider text is untrusted and unbounded; a small fixed budget keeps a hostile
/// or verbose executable from inflating results or hiding content in them.
pub const MAX_PROVIDER_DETAIL_BYTES: usize = 240;

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

#[cfg(test)]
mod tests {
    use super::sanitize_untrusted_text;

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
}
