//! Turning a Markdown document into comparable plain text, and finding words
//! and phrases in it.
//!
//! The documents are hard-wrapped, so a sentence the registry quotes can span
//! lines, a table cell or a block-quote prefix. Everything is therefore
//! compared in one form: lower case, links reduced to their text, emphasis
//! and code marks removed, fenced code blocks dropped, table bars, hyphens and
//! line breaks turned into single spaces. The check proves that listed
//! phrases are present or absent in that form; it does not understand
//! sentences.

use std::ops::Range;

/// The comparable form of a Markdown document (or of one registered text).
pub(crate) fn plain_text(markdown: &str) -> String {
    let mut text = String::new();
    let mut fenced = false;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        push_inline(&mut text, trimmed.trim_start_matches(['>', ' ']));
        text.push(' ');
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The words an SVG graphic shows, as plain text for [`plain_text`]: the text
/// nodes of the file, without its tags, comments, `style` and `script`
/// content. A `text`, `title` or `desc` element is set off by a space so
/// two lines never run together; the pieces of one line (`tspan`) are joined
/// as they are drawn, so a word split across them is still one word.
///
/// This reads what a person sees and a screen reader says (L-121), not the
/// attributes: a `font-family` or a colour is not a claim.
pub(crate) fn svg_text(svg: &str) -> String {
    let mut text = String::new();
    let mut skipping: Option<String> = None;
    let mut rest = svg;
    while let Some(open) = rest.find('<') {
        let (before, tag) = rest.split_at(open);
        if skipping.is_none() {
            push_entities(&mut text, before);
        }
        if let Some(body) = tag.strip_prefix("<!--") {
            rest = body.split_once("-->").map_or("", |(_, after)| after);
            continue;
        }
        if let Some(body) = tag.strip_prefix("<![CDATA[") {
            let (content, after) = body.split_once("]]>").unwrap_or((body, ""));
            if skipping.is_none() {
                text.push_str(content);
            }
            rest = after;
            continue;
        }
        let (markup, after) = split_tag(tag);
        rest = after;
        let closing = markup.starts_with("</");
        let name = tag_name(markup);
        if let Some(active) = &skipping {
            if closing && *active == name {
                skipping = None;
            }
            continue;
        }
        if !closing && !markup.ends_with("/>") && matches!(name.as_str(), "style" | "script") {
            skipping = Some(name);
            continue;
        }
        if matches!(name.as_str(), "text" | "title" | "desc") {
            text.push(' ');
        }
    }
    if skipping.is_none() {
        push_entities(&mut text, rest);
    }
    text
}

/// One tag (`<name ...>`, quotes respected) and what follows it. An unclosed
/// tag swallows the rest of the input.
fn split_tag(tag: &str) -> (&str, &str) {
    let mut quote: Option<char> = None;
    for (index, character) in tag.char_indices() {
        match (quote, character) {
            (None, '"' | '\'') => quote = Some(character),
            (Some(open), found) if open == found => quote = None,
            (None, '>') => return tag.split_at(index + 1),
            _ => {}
        }
    }
    (tag, "")
}

/// The lower-case local name of a tag: `<svg:text x="1">` is `text`.
fn tag_name(markup: &str) -> String {
    let inner = markup.trim_start_matches(['<', '/', '?', '!']);
    let name: String = inner
        .chars()
        .take_while(|character| !character.is_whitespace() && !matches!(character, '/' | '>'))
        .collect();
    name.rsplit(':').next().unwrap_or_default().to_lowercase()
}

/// Appends `text` with the XML character references decoded.
fn push_entities(output: &mut String, text: &str) {
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        let (before, reference) = rest.split_at(start);
        output.push_str(before);
        let decoded = reference
            .find(';')
            .filter(|end| *end <= 10)
            .and_then(|end| decode_reference(&reference[1..end]).map(|found| (found, end)));
        if let Some((found, end)) = decoded {
            output.push(found);
            rest = &reference[end + 1..];
        } else {
            output.push('&');
            rest = &reference[1..];
        }
    }
    output.push_str(rest);
}

/// The character a reference such as `amp` or `#x27` names.
fn decode_reference(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let digits = name.strip_prefix('#')?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// Appends one line without its Markdown marks.
fn push_inline(text: &mut String, line: &str) {
    let mut characters = line.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            // A link's destination is not prose: `[text](target)` keeps `text`.
            ']' if characters.peek() == Some(&'(') => {
                characters.next();
                let mut depth = 1_u32;
                for inner in characters.by_ref() {
                    match inner {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            '[' | ']' | '*' | '`' => {}
            '|' | '-' | '\u{a0}' => text.push(' '),
            '\u{2018}' | '\u{2019}' => text.push('\''),
            '\u{201c}' | '\u{201d}' => text.push('"'),
            other => text.extend(other.to_lowercase()),
        }
    }
}

/// Whether `character` can be part of a word.
fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Every place `needle` occurs in `haystack` as whole words: not preceded or
/// followed by a letter, digit or underscore where the needle itself begins
/// or ends with one. Both are expected in [`plain_text`] form.
pub(crate) fn find_all(haystack: &str, needle: &str) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    let begins_with_word = needle.chars().next().is_some_and(is_word);
    let ends_with_word = needle.chars().next_back().is_some_and(is_word);
    haystack
        .match_indices(needle)
        .filter_map(|(start, matched)| {
            let end = start + matched.len();
            let before_ok = !begins_with_word
                || haystack[..start]
                    .chars()
                    .next_back()
                    .is_none_or(|character| !is_word(character));
            let after_ok = !ends_with_word
                || haystack[end..]
                    .chars()
                    .next()
                    .is_none_or(|character| !is_word(character));
            (before_ok && after_ok).then_some(start..end)
        })
        .collect()
}

/// Whether `inner` lies wholly inside `outer`.
pub(crate) fn contains(outer: &Range<usize>, inner: &Range<usize>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

/// The words around `range`, for a message: up to forty characters on each
/// side.
pub(crate) fn snippet(text: &str, range: &Range<usize>) -> String {
    let start = text[..range.start]
        .char_indices()
        .rev()
        .nth(39)
        .map_or(0, |(index, _)| index);
    let end = text[range.end..]
        .char_indices()
        .nth(40)
        .map_or(text.len(), |(index, _)| range.end + index);
    format!("...{}...", &text[start..end])
}

/// The number of whitespace-separated words of a plain text.
pub(crate) fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

#[cfg(test)]
mod tests {
    use super::{contains, find_all, plain_text, snippet, svg_text, word_count};

    #[test]
    fn an_svg_gives_its_words_without_tags_styles_comments_or_attributes() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" aria-label=\"a supported label\">\n\
                   <!-- a supported comment -->\n\
                   <title id=\"t\">The roadmap</title><desc>What is &quot;done&quot; &amp; next</desc>\n\
                   <style>.supported{fill:red} text{white-space:pre}</style>\n\
                   <script>var stable = 1;</script>\n\
                   <rect width=\"1\" fill=\"supported\"/>\n\
                   <text x=\"1\" y=\"2\">First <tspan class=\"b\">line</tspan></text>\n\
                   <text x=\"1\" y=\"3\">sup<tspan>ported</tspan> here &#x41;&#66;</text>\n\
                   <![CDATA[ kept ]]></svg>";
        let text = plain_text(&svg_text(svg));
        assert_eq!(
            text,
            "the roadmap what is \"done\" & next first line supported here ab kept"
        );
        // Words shown to a reader are read; an attribute or a style is not.
        assert_eq!(find_all(&text, "supported").len(), 1);
        assert!(find_all(&text, "stable").is_empty());
        assert!(find_all(&text, "label").is_empty());
    }

    #[test]
    fn a_split_word_stays_one_word_and_two_text_elements_stay_two() {
        let text = plain_text(&svg_text(
            "<svg><text>no</text><text>t supported</text><text>unsup<tspan>ported</tspan></text></svg>",
        ));
        assert_eq!(text, "no t supported unsupported");
        assert_eq!(find_all(&text, "supported").len(), 1, "{text}");
    }

    #[test]
    fn a_tag_with_a_greater_than_in_a_quoted_attribute_ends_at_its_own_bracket() {
        let text = plain_text(&svg_text(
            "<svg><text data-x=\"a>b\" y='>'>shown</text><unclosed",
        ));
        assert_eq!(text, "shown");
    }

    #[test]
    fn an_unterminated_reference_is_kept_as_written() {
        assert_eq!(
            svg_text("<text>fish & chips &bogus; &#xZZ;</text>").trim(),
            "fish & chips &bogus; &#xZZ;"
        );
    }

    #[test]
    fn markup_links_code_fences_quotes_and_wraps_disappear() {
        let markdown = "# A **Stable** heading\n\
                        > a quoted line with a [link text](https://example.test/supported-models)\n\
                        > that wraps, and `code` marks.\n\
                        ```console\nvsift supported --stable\n```\n\
                        | cell one | not-supported |\n\
                        It\u{2019}s \u{201c}quoted\u{201d}.\n";
        assert_eq!(
            plain_text(markdown),
            "# a stable heading a quoted line with a link text that wraps, and code marks. \
             cell one not supported it's \"quoted\"."
        );
    }

    #[test]
    fn a_link_destination_with_parentheses_is_dropped_whole() {
        assert_eq!(
            plain_text("see [x](https://e.test/a_(b)) after"),
            "see x after"
        );
    }

    #[test]
    fn words_are_found_whole_only() {
        let text = plain_text("Unsupported, supported-version and partially_supported; supported.");
        assert_eq!(find_all(&text, "supported").len(), 2, "{text}");
        assert!(find_all(&text, "support").is_empty());
        assert!(find_all("", "supported").is_empty());
        assert!(find_all(&text, "").is_empty());
    }

    #[test]
    fn a_phrase_spans_a_hard_wrap() {
        let text = plain_text("is not\na stable or\n  supported release");
        assert_eq!(
            find_all(&text, "not a stable or supported release").len(),
            1
        );
    }

    #[test]
    fn a_needle_with_punctuation_at_its_ends_matches_beside_words() {
        let text = plain_text("no platform is \"supported\" yet");
        assert_eq!(find_all(&text, "\"supported\"").len(), 1);
    }

    #[test]
    fn containment_and_snippets() {
        assert!(contains(&(2..9), &(3..5)));
        assert!(!contains(&(2..9), &(1..5)));
        assert!(!contains(&(2..9), &(3..10)));
        let text = "a ".repeat(100);
        let shown = snippet(&text, &(100..101));
        assert!(
            shown.starts_with("...") && shown.ends_with("..."),
            "{shown}"
        );
        assert!(shown.len() < 100, "{shown}");
        assert_eq!(word_count("a b  c"), 3);
    }
}
