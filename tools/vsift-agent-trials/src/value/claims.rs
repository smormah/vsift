//! The cross-arm grader's reading of a free-text report (the value
//! measurement's method, sections 7 and 12.2).
//!
//! Every arm, raw or `VSift`, writes a plain answer, so the grader reads prose
//! and nothing else: it never sees the arm. A report is cut into statements
//! (sentences, list items and table rows); a statement that gives a time is
//! a timed claim. A truth item's key fact is matched when a timed claim states
//! it (the key-fact matcher of [`crate::truth`]) at a time that overlaps the
//! item's window. A claim that states the fact at a time outside every window
//! of that fact, or a statement holding a term the truth contradicts, is a
//! wrong claim. A statement about the recording with neither a time nor any
//! evidence behind it is unverifiable, which is reported and never gated.
//!
//! A written time is read as the span it can mean: `0:12` is any instant in
//! the twelfth second, `12.5 s` any instant in that tenth. So a reader who
//! writes whole seconds is not marked wrong for an event that starts at
//! 12.4 s, and nobody is credited for a time written more coarsely than a
//! second.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::truth::{KeyFact, contains_sequence, fact_words};

/// One thing the truth says happens, with the window it happens in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TruthItem {
    /// The item's identifier (a manifest event or a task's truth item).
    pub id: String,
    /// When it is on screen or said, in microseconds from the start.
    pub start_us: u64,
    /// The end of that window.
    pub end_us: u64,
    /// How far either side of the window a claim's time may lie and still
    /// count (the labels' agreement for real items; zero for the corpus).
    pub tolerance_us: u64,
    /// The facts a report must state about it.
    pub facts: Vec<KeyFact>,
    /// Terms that, stated anywhere, contradict the truth (for example
    /// another row number), each in the matcher's words.
    pub contradicts: Vec<Vec<String>>,
}

impl TruthItem {
    fn overlaps(&self, time: TimeSpan) -> bool {
        let start = self.start_us.saturating_sub(self.tolerance_us);
        let end = self.end_us.saturating_add(self.tolerance_us);
        time.start_us <= end && time.end_us >= start
    }
}

/// A time a report gives, as the span of instants it can mean.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimeSpan {
    /// The earliest instant.
    pub start_us: u64,
    /// The latest instant.
    pub end_us: u64,
}

/// One statement of a report, as the grader read it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Statement {
    /// Its position among the report's statements, from 0.
    pub index: usize,
    /// Its text.
    pub text: String,
    /// The times it gives.
    pub times: Vec<TimeSpan>,
    /// Whether it names any evidence: a file, a frame, a clip, a transcript
    /// line or an identity.
    pub cites_evidence: bool,
}

/// One key fact and whether a report matched it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FactResult {
    /// The truth item.
    pub item: String,
    /// The fact's term.
    pub term: String,
    /// Whether a timed claim stated it inside the item's window.
    pub matched: bool,
    /// The statement that matched it, by index.
    pub statement: Option<usize>,
}

/// Why a claim is wrong.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WrongKind {
    /// It states a fact at times that lie outside every window of that fact.
    OutsideWindow,
    /// It holds a term the truth contradicts.
    Contradicts,
}

/// One wrong claim.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WrongClaim {
    /// The statement, by index.
    pub statement: usize,
    /// The truth item it is wrong about.
    pub item: String,
    /// The fact or contradicting term, as written in the truth.
    pub term: String,
    /// Why.
    pub kind: WrongKind,
}

/// The grade of one report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimGrade {
    /// How many statements the report held.
    pub statements: usize,
    /// How many of them gave a time.
    pub timed_claims: usize,
    /// Every key fact of the task, matched or not.
    pub facts: Vec<FactResult>,
    /// Matched facts over all facts, from 0 to 1 (0 for a task with none).
    pub score: f64,
    /// Every wrong claim; one statement is wrong at most once per item.
    pub wrong: Vec<WrongClaim>,
    /// Statements about the recording with no time and no evidence.
    pub unverifiable: Vec<usize>,
    /// A person should read the run: the prose and the mechanical grade may
    /// disagree (a fact stated outside its window, or times inside a window
    /// while no fact matched there).
    pub needs_reading: bool,
}

/// Verbs that mark a statement as being about what the recording shows or
/// says, for the unverifiable count (method 7). Coarse on purpose, and in
/// the record so that a reader can judge it.
pub const RECORDING_VERBS: [&str; 24] = [
    "shows",
    "shown",
    "appears",
    "appear",
    "displays",
    "displayed",
    "says",
    "said",
    "says,",
    "clicks",
    "clicked",
    "opens",
    "opened",
    "changes",
    "changed",
    "types",
    "typed",
    "scrolls",
    "scrolled",
    "selects",
    "selected",
    "mentions",
    "mentioned",
    "reads",
];

/// File extensions that make a word a file reference.
const EVIDENCE_EXTENSIONS: [&str; 14] = [
    "png", "jpg", "jpeg", "webp", "gif", "bmp", "wav", "srt", "vtt", "json", "txt", "mp4", "mkv",
    "webm",
];

/// Words that name evidence by themselves.
const EVIDENCE_WORDS: [&str; 9] = [
    "frame",
    "frames",
    "clip",
    "clips",
    "segment",
    "segments",
    "screenshot",
    "sheet",
    "cue",
];

// The patterns carry no boundary of their own (the regex crate has no
// look-behind, and a consumed boundary would hide the second time of
// `0:12-0:15`); `bounded` checks the characters around each match instead.
static CLOCK: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?:([0-9]{1,2}):)?([0-9]{1,3}):([0-5][0-9])(?:[.,]([0-9]{1,3}))?").ok()
});

static SECONDS: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i-u)([0-9]{1,5})(?:\.([0-9]{1,3}))?[ \t]?(?:seconds|second|secs|sec|s)").ok()
});

static FILE_CLOCK: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"([0-9]{1,2})[-_]([0-5][0-9])[-_]([0-5][0-9])(?:[._]([0-9]{1,3}))?").ok()
});

/// Whether a match stands alone: the character before it is not one of
/// `before`, and the character after it is not one of `after` (letters and
/// digits are always refused after a unit).
fn bounded(text: &str, start: usize, end: usize, before: &str, after: &str) -> bool {
    let previous = text[..start].chars().next_back();
    let next = text[end..].chars().next();
    previous.is_none_or(|character| !before.contains(character))
        && next.is_none_or(|character| !after.contains(character))
}

const DIGITS: &str = "0123456789";
const LETTERS_AND_DIGITS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// What lies between two times that makes them one interval.
const RANGE_JOINERS: [&str; 6] = ["-", "\u{2013}", "\u{2014}", "to", "and", "through"];

/// Splits a report into statements: fenced code is skipped, each line is
/// cut at sentence ends, and a table row or list item is one statement.
#[must_use]
pub fn statements(report: &str) -> Vec<Statement> {
    let mut texts = Vec::new();
    let mut in_code = false;
    for line in report.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code || trimmed.is_empty() || is_table_rule(trimmed) {
            continue;
        }
        if trimmed.starts_with('|') {
            texts.push(trimmed.to_owned());
            continue;
        }
        texts.extend(sentences(trimmed));
    }
    texts
        .into_iter()
        .enumerate()
        .map(|(index, text)| {
            let (times, file_times) = times_in(&text);
            let cites_evidence = cites_evidence(&text) || !file_times.is_empty();
            let times = if times.is_empty() { file_times } else { times };
            Statement {
                index,
                text,
                times,
                cites_evidence,
            }
        })
        .collect()
}

fn is_table_rule(line: &str) -> bool {
    line.starts_with('|')
        && line
            .chars()
            .all(|character| matches!(character, '|' | '-' | ':' | ' '))
}

/// Cuts a line at a full stop, question or exclamation mark followed by a
/// space and a capital letter or a list marker; a decimal point is never a
/// cut because no space follows it.
fn sentences(line: &str) -> Vec<String> {
    let characters: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    for (position, character) in characters.iter().enumerate() {
        current.push(*character);
        let ends = matches!(character, '.' | '?' | '!')
            && characters.get(position + 1) == Some(&' ')
            && characters
                .get(position + 2)
                .is_some_and(|next| next.is_uppercase());
        if ends {
            out.push(current.trim().to_owned());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_owned());
    }
    out
}

fn precision_us(fraction: Option<&str>) -> u64 {
    match fraction.map(str::len) {
        Some(1) => 100_000,
        Some(2) => 10_000,
        Some(3) => 1_000,
        _ => 1_000_000,
    }
}

fn fraction_us(fraction: Option<&str>) -> u64 {
    fraction.map_or(0, |digits| {
        let value: u64 = digits.parse().unwrap_or(0);
        match digits.len() {
            1 => value * 100_000,
            2 => value * 10_000,
            _ => value * 1_000,
        }
    })
}

/// A parsed time with where it was written.
#[derive(Clone, Copy, Debug)]
struct Found {
    start: usize,
    end: usize,
    span: TimeSpan,
}

fn parse_number(text: Option<regex::Match<'_>>) -> u64 {
    text.and_then(|found| found.as_str().parse().ok())
        .unwrap_or(0)
}

/// Every time a statement gives, outside file names, and the times written
/// inside file names (`frame_00-01-12.500.png`).
fn times_in(text: &str) -> (Vec<TimeSpan>, Vec<TimeSpan>) {
    let files = file_tokens(text);
    let inside_file = |start: usize| files.iter().any(|(from, to)| start >= *from && start < *to);
    let mut found = Vec::new();
    if let Some(clock) = CLOCK.as_ref() {
        for captures in clock.captures_iter(text) {
            let Some(whole) = captures.get(0) else {
                continue;
            };
            let digits_colon_dot = "0123456789:.";
            let digits_colon = "0123456789:";
            if inside_file(whole.start())
                || !bounded(
                    text,
                    whole.start(),
                    whole.end(),
                    digits_colon_dot,
                    digits_colon,
                )
            {
                continue;
            }
            let hours = parse_number(captures.get(1));
            let minutes = parse_number(captures.get(2));
            let seconds = parse_number(captures.get(3));
            let fraction = captures.get(4).map(|found| found.as_str());
            let start_us =
                ((hours * 3_600 + minutes * 60 + seconds) * 1_000_000) + fraction_us(fraction);
            found.push(Found {
                start: whole.start(),
                end: whole.end(),
                span: TimeSpan {
                    start_us,
                    end_us: start_us + precision_us(fraction) - 1,
                },
            });
        }
    }
    if let Some(seconds) = SECONDS.as_ref() {
        for captures in seconds.captures_iter(text) {
            let Some(all) = captures.get(0) else {
                continue;
            };
            if inside_file(all.start())
                || !bounded(
                    text,
                    all.start(),
                    all.end(),
                    "0123456789.:",
                    LETTERS_AND_DIGITS,
                )
                || found
                    .iter()
                    .any(|other| all.start() < other.end && all.end() > other.start)
            {
                continue;
            }
            let whole = parse_number(captures.get(1));
            let fraction = captures.get(2).map(|found| found.as_str());
            let start_us = whole * 1_000_000 + fraction_us(fraction);
            found.push(Found {
                start: all.start(),
                end: all.end(),
                span: TimeSpan {
                    start_us,
                    end_us: start_us + precision_us(fraction) - 1,
                },
            });
        }
    }
    found.sort_by_key(|item| item.start);
    let mut file_times = Vec::new();
    if let Some(file_clock) = FILE_CLOCK.as_ref() {
        for (from, to) in &files {
            let token = &text[*from..*to];
            for captures in file_clock.captures_iter(token) {
                let Some(all) = captures.get(0) else {
                    continue;
                };
                if !bounded(token, all.start(), all.end(), DIGITS, DIGITS) {
                    continue;
                }
                let hours = parse_number(captures.get(1));
                let minutes = parse_number(captures.get(2));
                let seconds = parse_number(captures.get(3));
                let fraction = captures.get(4).map(|found| found.as_str());
                let start_us =
                    ((hours * 3_600 + minutes * 60 + seconds) * 1_000_000) + fraction_us(fraction);
                file_times.push(TimeSpan {
                    start_us,
                    end_us: start_us + precision_us(fraction) - 1,
                });
            }
        }
    }
    (join_ranges(text, &found), file_times)
}

/// Joins two times written as a range (`0:12-0:15`, `12 s to 15 s`,
/// `between 0:12 and 0:15`) into one span.
fn join_ranges(text: &str, found: &[Found]) -> Vec<TimeSpan> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < found.len() {
        let current = found[index];
        if let Some(next) = found.get(index + 1) {
            let between = text
                .get(current.end..next.start)
                .unwrap_or_default()
                .trim()
                .to_lowercase();
            if RANGE_JOINERS.contains(&between.as_str())
                && next.span.end_us >= current.span.start_us
            {
                out.push(TimeSpan {
                    start_us: current.span.start_us,
                    end_us: next.span.end_us,
                });
                index += 2;
                continue;
            }
        }
        out.push(current.span);
        index += 1;
    }
    out
}

/// The byte ranges of words that are file references: a word with a known
/// extension, or a path with a slash and a dot.
fn file_tokens(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    let mut push = |from: usize, to: usize| {
        let word = text[from..to].trim_matches(|character: char| {
            matches!(
                character,
                '`' | '(' | ')' | ',' | ';' | ':' | '"' | '\'' | '[' | ']'
            ) || character == '.'
        });
        let lower = word.to_ascii_lowercase();
        let has_extension = lower
            .rsplit_once('.')
            .is_some_and(|(_, extension)| EVIDENCE_EXTENSIONS.contains(&extension));
        if has_extension || (lower.contains('/') && lower.contains('.')) {
            out.push((from, to));
        }
    };
    for (position, character) in text.char_indices() {
        if character.is_whitespace() {
            if let Some(from) = start.take() {
                push(from, position);
            }
        } else if start.is_none() {
            start = Some(position);
        }
    }
    if let Some(from) = start {
        push(from, text.len());
    }
    out
}

fn cites_evidence(text: &str) -> bool {
    if !file_tokens(text).is_empty() || !crate::cold::identities_in(text).is_empty() {
        return true;
    }
    let words = fact_words(text);
    words
        .iter()
        .any(|word| EVIDENCE_WORDS.contains(&word.as_str()))
        || words
            .windows(2)
            .any(|pair| pair[0] == "line" && pair[1].parse::<u64>().is_ok())
}

fn about_the_recording(statement: &Statement, items: &[TruthItem]) -> bool {
    let lower: Vec<String> = statement
        .text
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_owned)
        .collect();
    lower
        .iter()
        .any(|word| RECORDING_VERBS.contains(&word.as_str()))
        || items
            .iter()
            .flat_map(|item| &item.facts)
            .any(|fact| fact.stated_in(&statement.text))
}

/// Each key fact of the truth, with the first timed statement that states
/// it inside its item's widened window.
fn matched_facts(statements: &[Statement], items: &[TruthItem]) -> Vec<FactResult> {
    let mut facts = Vec::new();
    for item in items {
        for fact in &item.facts {
            let matched = statements.iter().find(|statement| {
                fact.stated_in(&statement.text)
                    && statement.times.iter().any(|time| item.overlaps(*time))
            });
            facts.push(FactResult {
                item: item.id.clone(),
                term: fact.term.clone(),
                matched: matched.is_some(),
                statement: matched.map(|statement| statement.index),
            });
        }
    }
    facts
}

/// The wrong claims: a statement holding a `contradicts` term, or a timed
/// statement of a fact whose every time lies outside every window of it.
fn wrong_claims(statements: &[Statement], items: &[TruthItem]) -> Vec<WrongClaim> {
    let mut wrong = Vec::new();
    for statement in statements {
        let words = fact_words(&statement.text);
        for item in items {
            if let Some(term) = item
                .contradicts
                .iter()
                .find(|term| contains_sequence(&words, term))
            {
                wrong.push(WrongClaim {
                    statement: statement.index,
                    item: item.id.clone(),
                    term: term.join(" "),
                    kind: WrongKind::Contradicts,
                });
                continue;
            }
            if statement.times.is_empty() {
                continue;
            }
            let stated = item
                .facts
                .iter()
                .find(|fact| fact.stated_in(&statement.text));
            let Some(fact) = stated else {
                continue;
            };
            // The same fact may belong to several items (a header that stays
            // on screen): the claim is wrong only when no item holding that
            // fact covers any of its times.
            let any_window = items
                .iter()
                .filter(|other| {
                    other
                        .facts
                        .iter()
                        .any(|candidate| candidate.words == fact.words)
                })
                .any(|other| statement.times.iter().any(|time| other.overlaps(*time)));
            if !any_window {
                wrong.push(WrongClaim {
                    statement: statement.index,
                    item: item.id.clone(),
                    term: fact.term.clone(),
                    kind: WrongKind::OutsideWindow,
                });
            }
        }
    }
    wrong
}

/// Grades one report against a task phase's truth.
#[must_use]
pub fn grade_report(report: &str, items: &[TruthItem]) -> ClaimGrade {
    let statements = statements(report);
    let facts = matched_facts(&statements, items);
    let wrong = wrong_claims(&statements, items);
    let unverifiable = statements
        .iter()
        .filter(|statement| {
            statement.times.is_empty()
                && !statement.cites_evidence
                && about_the_recording(statement, items)
        })
        .map(|statement| statement.index)
        .collect();
    let matched = facts.iter().filter(|fact| fact.matched).count();
    #[allow(
        clippy::cast_precision_loss,
        reason = "Counts of facts are small; the share is reported to two places"
    )]
    let score = if facts.is_empty() {
        0.0
    } else {
        matched as f64 / facts.len() as f64
    };
    let times_in_window_unmatched = facts.iter().any(|fact| !fact.matched)
        && statements.iter().any(|statement| {
            statement
                .times
                .iter()
                .any(|time| items.iter().any(|item| item.overlaps(*time)))
        });
    let needs_reading = wrong
        .iter()
        .any(|claim| claim.kind == WrongKind::OutsideWindow)
        || (matched == 0 && times_in_window_unmatched);
    ClaimGrade {
        statements: statements.len(),
        timed_claims: statements
            .iter()
            .filter(|statement| !statement.times.is_empty())
            .count(),
        facts,
        score,
        wrong,
        unverifiable,
        needs_reading,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(item: &str, term: &str) -> KeyFact {
        KeyFact {
            event: item.to_owned(),
            term: term.to_owned(),
            words: fact_words(term),
            alternatives: Vec::new(),
        }
    }

    fn items() -> Vec<TruthItem> {
        vec![
            TruthItem {
                id: "E1".to_owned(),
                start_us: 12_400_000,
                end_us: 14_000_000,
                tolerance_us: 0,
                facts: vec![fact("E1", "order 1017"), fact("E1", "highlighted")],
                contradicts: vec![fact_words("order 1016")],
            },
            TruthItem {
                id: "E2".to_owned(),
                start_us: 70_000_000,
                end_us: 75_000_000,
                tolerance_us: 1_000_000,
                facts: vec![fact("E2", "header")],
                contradicts: Vec::new(),
            },
        ]
    }

    fn span(text: &str) -> Vec<TimeSpan> {
        statements(text)
            .into_iter()
            .flat_map(|statement| statement.times)
            .collect()
    }

    /// The patterns are built once; a pattern the regex crate refused would
    /// silently read no times, so this pins that all three compile with the
    /// workspace's feature set (no Unicode tables).
    #[test]
    fn the_time_patterns_compile() {
        assert!(CLOCK.is_some() && SECONDS.is_some() && FILE_CLOCK.is_some());
    }

    #[test]
    fn times_are_read_as_the_span_they_can_mean() {
        assert_eq!(
            span("It shows at 0:12."),
            vec![TimeSpan {
                start_us: 12_000_000,
                end_us: 12_999_999
            }]
        );
        assert_eq!(
            span("At 1:02:03.5 it ends."),
            vec![TimeSpan {
                start_us: 3_723_500_000,
                end_us: 3_723_599_999
            }]
        );
        assert_eq!(
            span("It starts at 12.5 s"),
            vec![TimeSpan {
                start_us: 12_500_000,
                end_us: 12_599_999
            }]
        );
        assert_eq!(
            span("Between 0:12 and 0:15 the row stays."),
            vec![TimeSpan {
                start_us: 12_000_000,
                end_us: 15_999_999
            }]
        );
        assert_eq!(
            span("From 12 seconds to 15 seconds."),
            vec![TimeSpan {
                start_us: 12_000_000,
                end_us: 15_999_999
            }]
        );
    }

    #[test]
    fn ratios_versions_and_durations_are_not_times() {
        assert_eq!(span("The video is 16:9 at version 1.2.3."), Vec::new());
        assert_eq!(span("The tooltip lasts 500 ms."), Vec::new());
    }

    #[test]
    fn a_time_in_a_file_name_is_evidence_and_counts_only_alone() {
        let found = statements("Order 1017 is highlighted (frame_00-00-12.500.png).");
        assert_eq!(found.len(), 1);
        assert!(found[0].cites_evidence);
        assert_eq!(
            found[0].times,
            vec![TimeSpan {
                start_us: 12_500_000,
                end_us: 12_500_999
            }]
        );
        let both = statements("At 0:13 order 1017 is highlighted (frames/f_00-00-40.png).");
        assert_eq!(
            both[0].times,
            vec![TimeSpan {
                start_us: 13_000_000,
                end_us: 13_999_999
            }]
        );
    }

    #[test]
    fn fenced_code_and_table_rules_are_not_statements() {
        let report = "Answer.\n```\nffmpeg -ss 00:01:12 -i a.mp4\n```\n| Time | What |\n| --- | --- |\n| 0:13 | order 1017 highlighted |\n";
        let found = statements(report);
        assert_eq!(found.len(), 3, "{found:?}");
        assert_eq!(found[2].times.len(), 1);
    }

    #[test]
    fn a_right_report_in_other_spellings_scores_one() {
        let report = "Order one thousand seventeen is highlighted at 0:13 (frame t13.png).\n\
                      - The header stays on screen from 1:10 to 1:15, see sheet-70s.png.";
        let grade = grade_report(report, &items());
        assert!(
            (grade.score - 1.0).abs() < f64::EPSILON,
            "{:?}",
            grade.facts
        );
        assert_eq!(grade.wrong, Vec::new());
        assert_eq!(grade.unverifiable, Vec::<usize>::new());
        assert!(!grade.needs_reading);
    }

    /// The grader's honesty: right words at wrong times, a contradicting
    /// value and assertions with nothing behind them score nothing.
    #[test]
    fn a_deliberately_wrong_report_scores_zero_with_its_faults_counted() {
        let report = "Order 1017 is highlighted at 0:40.\n\
                      Then order 1016 shows in the table.\n\
                      The header appears near the end.\n\
                      The header stays at 2:30.";
        let grade = grade_report(report, &items());
        assert!(grade.score.abs() < f64::EPSILON, "{:?}", grade.facts);
        let kinds: Vec<(usize, WrongKind)> = grade
            .wrong
            .iter()
            .map(|claim| (claim.statement, claim.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (0, WrongKind::OutsideWindow),
                (1, WrongKind::Contradicts),
                (3, WrongKind::OutsideWindow)
            ]
        );
        assert_eq!(grade.unverifiable, vec![1, 2]);
        assert!(grade.needs_reading);
    }

    #[test]
    fn the_tolerance_widens_a_window_and_a_coarse_time_overlaps_it() {
        let header = |report: &str| {
            grade_report(report, &items())
                .facts
                .iter()
                .any(|fact| fact.term == "header" && fact.matched)
        };
        assert!(header("The header is visible at 1:16."));
        assert!(!header("The header is visible at 1:17."));
        // A whole second covers an event that starts inside it.
        let row = grade_report("Order 1017 is highlighted at 0:12.", &items());
        assert!(row.facts[0].matched, "{:?}", row.facts);
    }

    #[test]
    fn a_range_written_with_a_dash_keeps_both_ends() {
        assert_eq!(
            span("Visible 0:12-0:15."),
            vec![TimeSpan {
                start_us: 12_000_000,
                end_us: 15_999_999
            }]
        );
    }

    #[test]
    fn a_task_without_facts_scores_zero_not_a_division_error() {
        assert!(grade_report("Nothing.", &[]).score.abs() < f64::EPSILON);
    }
}
