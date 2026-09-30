//! The checks of a whole report's text: no local path or home folder, no
//! live link and no raw hidden or control character, anywhere in it (the
//! Markdown and the block alike). `references/safety.md` states the rules;
//! the trial grader applied them to every final message since P12.

use super::{
    block::line_number,
    rule::{HandoffFinding, HandoffRule},
};
use crate::is_hidden_character;

/// Link schemes and prefixes a report may not hold (lower case).
const LINK_MARKERS: [&str; 5] = ["http://", "https://", "ftp://", "file://", "www."];

/// Home-folder and local-path prefixes a report may not hold (lower case).
const HOME_MARKERS: [&str; 8] = [
    "/home/",
    "/users/",
    "/root/",
    "~/",
    "~\\",
    "appdata",
    "%userprofile%",
    "$home",
];

/// One finding per rule and line that breaks it, in line order.
pub(crate) fn report_text_findings(report: &str) -> Vec<HandoffFinding> {
    let mut findings = Vec::new();
    for (index, line) in report.lines().enumerate() {
        let number = Some(line_number(index));
        if line.chars().any(is_hidden_character) {
            findings.push(HandoffFinding::on_line(
                number,
                HandoffRule::HiddenCharacter,
            ));
        }
        if line
            .chars()
            .any(|value| value.is_control() && !matches!(value, '\t' | '\r'))
        {
            findings.push(HandoffFinding::on_line(
                number,
                HandoffRule::ControlCharacter,
            ));
        }
        let lowered = line.to_lowercase();
        if LINK_MARKERS.iter().any(|marker| lowered.contains(marker)) {
            findings.push(HandoffFinding::on_line(number, HandoffRule::WebLink));
        }
        if has_extended_length_path(line)
            || HOME_MARKERS.iter().any(|marker| lowered.contains(marker))
            || has_drive_path(line)
        {
            findings.push(HandoffFinding::on_line(number, HandoffRule::LocalPath));
        }
    }
    findings
}

/// Whether the text holds a Windows extended-length path: the `\\?\`
/// prefix (or `//?/`) followed by a drive (`C:`) or the UNC form (`UNC\`).
/// The bare prefix is not a path: the skill itself tells agents to retry an
/// image without it, and a report may say so (A-09, 2026-09-29).
fn has_extended_length_path(text: &str) -> bool {
    ["\\\\?\\", "//?/"].iter().any(|prefix| {
        text.match_indices(prefix).any(|(index, _)| {
            let rest: Vec<char> = text[index + prefix.len()..].chars().take(4).collect();
            let drive = rest.len() >= 2 && rest[0].is_ascii_alphabetic() && rest[1] == ':';
            let unc = rest.len() == 4
                && rest[..3]
                    .iter()
                    .collect::<String>()
                    .eq_ignore_ascii_case("unc")
                && matches!(rest[3], '\\' | '/');
            drive || unc
        })
    })
}

/// Whether the text holds `X:\` or `X:/` after a non-letter.
fn has_drive_path(text: &str) -> bool {
    let characters: Vec<char> = text.chars().collect();
    characters.windows(3).enumerate().any(|(index, window)| {
        let boundary = index == 0 || !characters[index - 1].is_alphanumeric();
        boundary
            && window[0].is_ascii_alphabetic()
            && window[1] == ':'
            && matches!(window[2], '\\' | '/')
    })
}

#[cfg(test)]
mod tests {
    use super::report_text_findings;
    use crate::HandoffRule;

    fn rules(text: &str) -> Vec<HandoffRule> {
        report_text_findings(text)
            .iter()
            .map(crate::HandoffFinding::rule)
            .collect()
    }

    #[test]
    fn paths_links_and_hidden_characters_are_found_by_line() {
        assert!(rules("All fine at 10:32.").is_empty());
        // `display_text`'s notation, quoted as the skill says (P12 PR 3h).
        let quoted =
            "> `Status shown to reviewers: <U+202E>DELIAF<U+202C> build<U+200B> pass<U+200D>ed`";
        assert!(rules(quoted).is_empty());
        assert_eq!(rules("see C:\\trials\\a"), vec![HandoffRule::LocalPath]);
        assert_eq!(rules("open https://x"), vec![HandoffRule::WebLink]);
        assert_eq!(
            rules("hidden \u{202E}text"),
            vec![HandoffRule::HiddenCharacter]
        );
        assert_eq!(rules("at /home/someone/x"), vec![HandoffRule::LocalPath]);
        assert_eq!(rules("esc \u{1b}[31m"), vec![HandoffRule::ControlCharacter]);
        let findings = report_text_findings("fine\nhttps://x\nfine");
        assert_eq!(
            findings.first().and_then(crate::HandoffFinding::line),
            Some(2)
        );
    }

    /// A-09 (2026-09-29): the bare `\\?\` prefix in prose is not a path; the
    /// prefix followed by a drive or a UNC share is.
    #[test]
    fn only_an_actual_extended_length_path_is_a_path() {
        assert!(rules("retried without the `\\\\?\\` prefix").is_empty());
        for bad in [
            "opened \\\\?\\C:\\trials\\frame.png",
            "opened \\\\?\\UNC\\server\\share\\frame.png",
            "opened //?/c:/trials/frame.png",
        ] {
            assert!(rules(bad).contains(&HandoffRule::LocalPath), "{bad}");
        }
    }
}
