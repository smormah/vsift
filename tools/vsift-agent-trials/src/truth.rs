//! The frozen corpus truth, read from `fixtures/corpus`.
//!
//! Scenarios name manifest event identifiers only; every window, script and
//! key fact comes from `manifest.json` and the speech placement from
//! `generated/speech-provenance.json`, so a scenario can never disagree
//! with the truth by carrying its own copy.

use std::{collections::BTreeMap, path::Path};

use serde::Deserialize;

use crate::error::{TrialError, read_text};

/// One manifest event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Event {
    /// For example `F05-E03`.
    pub id: String,
    /// `stable`, `change`, `transient`, ...
    pub kind: String,
    /// Window start, microseconds.
    pub start_us: u64,
    /// Window end (excluded), microseconds.
    pub end_us: u64,
    /// Whether the event is critical.
    pub critical: bool,
    /// The reviewed truth sentence.
    pub truth: String,
}

/// A fixture's audio description.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Audio {
    /// `none`, `clean-synthetic`, ...
    pub mode: String,
    /// The spoken script.
    pub script: String,
}

/// One manifest fixture.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Fixture {
    /// `F01` .. `F12`.
    pub id: String,
    /// Length of the fixture video.
    pub duration_us: u64,
    /// Audio and script.
    pub audio: Audio,
    /// Truth events.
    pub events: Vec<Event>,
    /// Terms the fixture's evidence must support.
    pub expected_terms: Vec<String>,
}

#[derive(Deserialize)]
struct Manifest {
    fixtures: Vec<Fixture>,
}

/// Where a speech variant's utterance was placed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeechSpan {
    /// First spoken microsecond.
    pub start_us: u64,
    /// End of speech.
    pub end_us: u64,
}

/// The corpus truth the grader and the preparer read.
#[derive(Clone, Debug)]
pub struct CorpusTruth {
    fixtures: BTreeMap<String, Fixture>,
    speech: BTreeMap<String, SpeechSpan>,
}

impl CorpusTruth {
    /// Reads `manifest.json` and `generated/speech-provenance.json` below
    /// `corpus` (the repository's `fixtures/corpus`).
    ///
    /// # Errors
    ///
    /// [`TrialError`] when either file is missing or malformed.
    pub fn load(corpus: &Path) -> Result<Self, TrialError> {
        let manifest: Manifest =
            serde_json::from_str(&read_text(&corpus.join("manifest.json"))?)
                .map_err(|error| TrialError::json("fixtures/corpus/manifest.json", error))?;
        let provenance: serde_json::Value = serde_json::from_str(&read_text(
            &corpus.join("generated").join("speech-provenance.json"),
        )?)
        .map_err(|error| TrialError::json("speech-provenance.json", error))?;
        let mut speech = BTreeMap::new();
        for variant in provenance["assembly"]["variants"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let (Some(fixture), Some(start_us), Some(end_us)) = (
                variant["fixture"].as_str(),
                variant["speech_start_us"].as_u64(),
                variant["speech_end_us"].as_u64(),
            ) {
                speech.insert(fixture.to_owned(), SpeechSpan { start_us, end_us });
            }
        }
        Ok(Self {
            fixtures: manifest
                .fixtures
                .into_iter()
                .map(|fixture| (fixture.id.clone(), fixture))
                .collect(),
            speech,
        })
    }

    /// A fixture by identifier.
    ///
    /// # Errors
    ///
    /// [`TrialError::Invalid`] when the manifest has no such fixture.
    pub fn fixture(&self, id: &str) -> Result<&Fixture, TrialError> {
        self.fixtures
            .get(id)
            .ok_or_else(|| TrialError::Invalid(format!("the manifest has no fixture {id}")))
    }

    /// An event by identifier, with its fixture.
    ///
    /// # Errors
    ///
    /// [`TrialError::Invalid`] when the manifest has no such event.
    pub fn event(&self, id: &str) -> Result<(&Fixture, &Event), TrialError> {
        let fixture_id = id.split('-').next().unwrap_or_default();
        let fixture = self.fixture(fixture_id)?;
        fixture
            .events
            .iter()
            .find(|event| event.id == id)
            .map(|event| (fixture, event))
            .ok_or_else(|| TrialError::Invalid(format!("the manifest has no event {id}")))
    }

    /// Where the speech variant of a fixture speaks, if it has one.
    #[must_use]
    pub fn speech_span(&self, fixture: &str) -> Option<SpeechSpan> {
        self.speech.get(fixture).copied()
    }

    /// The key facts of an event: the fixture's expected terms that its truth
    /// sentence states, in the key-fact matcher's words ([`fact_words`]).
    ///
    /// # Errors
    ///
    /// As [`CorpusTruth::event`].
    pub fn key_facts(&self, event_id: &str) -> Result<Vec<KeyFact>, TrialError> {
        let (fixture, event) = self.event(event_id)?;
        let truth = fact_words(&event.truth);
        Ok(fixture
            .expected_terms
            .iter()
            .filter_map(|term| {
                let words = fact_words(term);
                (!words.is_empty() && contains_sequence(&truth, &words)).then(|| KeyFact {
                    event: event.id.clone(),
                    term: term.clone(),
                    alternatives: time_alternatives(term),
                    words,
                })
            })
            .collect())
    }
}

/// One fact the handoff should state and cite.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyFact {
    /// The event whose truth states it.
    pub event: String,
    /// The manifest term.
    pub term: String,
    /// The term in the matcher's words ([`fact_words`]).
    pub words: Vec<String>,
    /// Other spellings of the same term that also state it: a clock time
    /// `10:32` written `10.32` ([`time_alternatives`]).
    pub alternatives: Vec<Vec<String>>,
}

impl KeyFact {
    /// Whether `text` states this fact.
    #[must_use]
    pub fn stated_in(&self, text: &str) -> bool {
        let text = fact_words(text);
        contains_sequence(&text, &self.words)
            || self
                .alternatives
                .iter()
                .any(|words| contains_sequence(&text, words))
    }
}

/// Number words the CLI's search also reads as digits.
const NUMBER_WORDS: [(&str, &str); 21] = [
    ("zero", "0"),
    ("one", "1"),
    ("two", "2"),
    ("three", "3"),
    ("four", "4"),
    ("five", "5"),
    ("six", "6"),
    ("seven", "7"),
    ("eight", "8"),
    ("nine", "9"),
    ("ten", "10"),
    ("eleven", "11"),
    ("twelve", "12"),
    ("thirteen", "13"),
    ("fourteen", "14"),
    ("fifteen", "15"),
    ("sixteen", "16"),
    ("seventeen", "17"),
    ("eighteen", "18"),
    ("nineteen", "19"),
    ("twenty", "20"),
];

/// The tens the key-fact matcher also reads (P12 PR 3i); `twenty` is in
/// [`NUMBER_WORDS`].
const TENS_WORDS: [(&str, u64); 8] = [
    ("twenty", 20),
    ("thirty", 30),
    ("forty", 40),
    ("fifty", 50),
    ("sixty", 60),
    ("seventy", 70),
    ("eighty", 80),
    ("ninety", 90),
];

/// Normalises text for fact matching, close to the CLI's search rules:
/// lower case; a hyphen between letters or digits joins them (`E-409` is
/// `e409`); a colon between digits separates (`10:32` is `10 32`); a
/// decimal loses trailing zeros (`127.50` is `127.5`); number words up to
/// twenty are digits; `milliseconds` is `ms` and `seconds` is `s`; any
/// other punctuation separates words.
#[must_use]
pub fn normalize(text: &str) -> Vec<String> {
    tokens(text)
        .into_iter()
        .map(|word| {
            NUMBER_WORDS
                .iter()
                .find(|(name, _)| *name == word)
                .map_or(word, |(_, digits)| (*digits).to_owned())
        })
        .collect()
}

/// The words the key-fact matcher compares (P12 PR 3i): [`normalize`], with
/// every English cardinal number written in words, up to 999,999, read as
/// its digits. The table, applied only here:
///
/// | Words | Read as |
/// | --- | --- |
/// | `zero` to `nineteen` | `0` to `19` |
/// | `twenty`, `thirty`, ... `ninety` | `20`, `30`, ... `90` |
/// | a ten and a unit, spaced or hyphenated (`forty two`, `forty-two`) | `42` |
/// | a number below 100, then `hundred` (and optionally `and`) | `100` times it, plus what follows (`eight hundred forty` is `840`) |
/// | a number below 1,000, then `thousand` | `1000` times it, plus what follows (`two thousand forty-eight` is `2048`) |
///
/// Only words are joined, never digits (`10 32` stays two words), and only
/// in this grammar: `one two` stays `1 2`, and `hundred` or `thousand`
/// without a number before it stays a word. Nothing else is a synonym: a
/// "submission button" does not state "Submit".
#[must_use]
pub fn fact_words(text: &str) -> Vec<String> {
    let words = tokens(text);
    let mut read = Vec::with_capacity(words.len());
    let mut index = 0;
    while index < words.len() {
        if let Some((value, used)) = read_number(&words[index..]) {
            read.push(value.to_string());
            index += used;
        } else {
            read.push(words[index].clone());
            index += 1;
        }
    }
    read
}

/// Other spellings of a term that holds a clock time `H:MM` or `HH:MM`
/// (P12 PR 3i): the same term with a full stop for the colon (`10.32`), in
/// the matcher's words. A term without a clock time has none.
#[must_use]
pub fn time_alternatives(term: &str) -> Vec<Vec<String>> {
    let characters: Vec<char> = term.chars().collect();
    let is_clock = |index: usize| {
        let digit = |offset: usize| characters.get(offset).is_some_and(char::is_ascii_digit);
        let hour_digits = (1..=2)
            .rev()
            .find(|width| index >= *width && (index - width..index).all(digit))
            .unwrap_or(0);
        let hour_starts = index - hour_digits;
        let before_hour = hour_starts
            .checked_sub(1)
            .and_then(|previous| characters.get(previous))
            .is_some_and(char::is_ascii_alphanumeric);
        let minutes = digit(index + 1) && digit(index + 2) && !digit(index + 3);
        let minute_value = characters
            .get(index + 1)
            .and_then(|tens| tens.to_digit(10))
            .is_some_and(|tens| tens < 6);
        hour_digits > 0 && !before_hour && minutes && minute_value
    };
    let dotted: String = characters
        .iter()
        .enumerate()
        .map(|(index, character)| {
            if *character == ':' && is_clock(index) {
                '.'
            } else {
                *character
            }
        })
        .collect();
    if dotted == term {
        Vec::new()
    } else {
        vec![fact_words(&dotted)]
    }
}

/// What one word is to the number grammar of [`fact_words`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NumberWord {
    /// `zero` to `nineteen`.
    Small(u64),
    /// `twenty` to `ninety`.
    Ten(u64),
    /// A ten and a unit written as one word (`fortytwo`, from `forty-two`).
    TenAndUnit(u64),
    /// `hundred`.
    Hundred,
    /// `thousand`.
    Thousand,
    /// `and`, only between `hundred` or `thousand` and what follows.
    And,
}

fn number_word(word: &str) -> Option<NumberWord> {
    let small = |text: &str| {
        NUMBER_WORDS
            .iter()
            .take(20)
            .find(|(name, _)| *name == text)
            .and_then(|(_, digits)| digits.parse::<u64>().ok())
    };
    if let Some(value) = small(word) {
        return Some(NumberWord::Small(value));
    }
    if let Some((_, value)) = TENS_WORDS.iter().find(|(name, _)| *name == word) {
        return Some(NumberWord::Ten(*value));
    }
    match word {
        "hundred" => return Some(NumberWord::Hundred),
        "thousand" => return Some(NumberWord::Thousand),
        "and" => return Some(NumberWord::And),
        _ => {}
    }
    TENS_WORDS.iter().find_map(|(name, tens)| {
        word.strip_prefix(name)
            .and_then(small)
            .filter(|unit| (1..10).contains(unit))
            .map(|unit| NumberWord::TenAndUnit(tens + unit))
    })
}

/// Reads the longest cardinal number at the start of `words` in the
/// grammar of [`fact_words`]: its value and how many words it took, or
/// `None` when the first word does not start one.
fn read_number(words: &[String]) -> Option<(u64, usize)> {
    let kinds: Vec<Option<NumberWord>> = words.iter().map(|word| number_word(word)).collect();
    let starts = |kind: Option<NumberWord>| {
        matches!(
            kind,
            Some(NumberWord::Small(_) | NumberWord::Ten(_) | NumberWord::TenAndUnit(_))
        )
    };
    if !starts(kinds.first().copied().flatten()) {
        return None;
    }
    // `thousands` is what a `thousand` closed; `current` is below 1,000.
    let (mut thousands, mut current) = (None::<u64>, 0_u64);
    let mut hundred_seen = false;
    let mut previous: Option<NumberWord> = None;
    let mut used = 0;
    for (index, kind) in kinds.iter().enumerate() {
        let Some(kind) = *kind else { break };
        let fits = match (previous, kind) {
            (
                None | Some(NumberWord::Hundred | NumberWord::Thousand | NumberWord::And),
                NumberWord::Small(_) | NumberWord::Ten(_) | NumberWord::TenAndUnit(_),
            ) => true,
            (Some(NumberWord::Ten(_)), NumberWord::Small(unit)) => (1..10).contains(&unit),
            (
                Some(NumberWord::Small(_) | NumberWord::Ten(_) | NumberWord::TenAndUnit(_)),
                NumberWord::Hundred,
            ) => !hundred_seen && current > 0 && current < 100,
            (
                Some(
                    NumberWord::Small(_)
                    | NumberWord::Ten(_)
                    | NumberWord::TenAndUnit(_)
                    | NumberWord::Hundred,
                ),
                NumberWord::Thousand,
            ) => thousands.is_none() && current > 0,
            (Some(NumberWord::Hundred | NumberWord::Thousand), NumberWord::And) => {
                starts(kinds.get(index + 1).copied().flatten())
            }
            _ => false,
        };
        if !fits {
            break;
        }
        match kind {
            NumberWord::Small(value) | NumberWord::Ten(value) | NumberWord::TenAndUnit(value) => {
                current += value;
            }
            NumberWord::Hundred => {
                current *= 100;
                hundred_seen = true;
            }
            NumberWord::Thousand => {
                thousands = Some(current * 1_000);
                current = 0;
                hundred_seen = false;
            }
            NumberWord::And => {}
        }
        previous = Some(kind);
        used = index + 1;
    }
    Some((thousands.unwrap_or(0) + current, used))
}

/// The cleaned words of [`normalize`], before number words are read.
fn tokens(text: &str) -> Vec<String> {
    let characters: Vec<char> = text.to_lowercase().chars().collect();
    let mut cleaned = String::with_capacity(characters.len());
    for (index, character) in characters.iter().enumerate() {
        let before = index
            .checked_sub(1)
            .and_then(|previous| characters.get(previous));
        let after = characters.get(index + 1);
        let between = |test: fn(&char) -> bool| before.is_some_and(test) && after.is_some_and(test);
        match character {
            '-' if between(|value| value.is_alphanumeric()) => {}
            '.' if between(char::is_ascii_digit) => cleaned.push('.'),
            ',' if between(char::is_ascii_digit) => {}
            value if value.is_alphanumeric() => cleaned.push(*value),
            _ => cleaned.push(' '),
        }
    }
    cleaned
        .split_whitespace()
        .map(|word| {
            let word = if word.contains('.') {
                word.trim_end_matches('0').trim_end_matches('.')
            } else {
                word
            };
            match word {
                "milliseconds" | "millisecond" => "ms",
                "seconds" | "second" => "s",
                other => other,
            }
            .to_owned()
        })
        .collect()
}

/// Whether `words` occurs contiguously in `text`.
#[must_use]
pub fn contains_sequence(text: &[String], words: &[String]) -> bool {
    !words.is_empty() && text.windows(words.len()).any(|window| window == words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        normalize(text)
    }

    #[test]
    fn normalisation_follows_the_search_rules() {
        assert_eq!(words("Error E-409!"), vec!["error", "e409"]);
        assert_eq!(words("at 10:32"), vec!["at", "10", "32"]);
        assert_eq!(words("127.50 and 125.00"), vec!["127.5", "and", "125"]);
        assert_eq!(words("rises to twelve"), vec!["rises", "to", "12"]);
        assert_eq!(words("840 milliseconds"), vec!["840", "ms"]);
        assert!(contains_sequence(
            &words("labels show 840 ms"),
            &words("840 milliseconds")
        ));
        // `normalize` itself stays as it was: only single words to twenty.
        assert_eq!(words("eight hundred forty"), vec!["8", "hundred", "forty"]);
    }

    /// The documented table of [`fact_words`], each row with its reading.
    #[test]
    fn the_fact_matcher_reads_numbers_written_in_words() {
        for (text, read) in [
            ("rises to twelve", "rises to 12"),
            ("twelve", "12"),
            ("zero", "0"),
            ("nineteen", "19"),
            ("twenty", "20"),
            ("ninety", "90"),
            ("forty two", "42"),
            ("forty-two", "42"),
            ("Forty-Two", "42"),
            ("one hundred", "100"),
            ("one hundred and twenty", "120"),
            ("one hundred twenty", "120"),
            ("eight hundred forty milliseconds", "840 ms"),
            ("eight hundred and forty-one", "841"),
            ("two thousand forty-eight", "2048"),
            ("two thousand and forty eight", "2048"),
            ("four thousand four hundred seven", "4407"),
            ("one thousand seventeen", "1017"),
            ("twelve thousand", "12000"),
            (
                "nine hundred ninety-nine thousand nine hundred ninety-nine",
                "999999",
            ),
            // Not joined: digits, counting, a lone multiplier, a trailing `and`.
            ("10 32", "10 32"),
            ("one two three", "1 2 3"),
            ("twelve twelve", "12 12"),
            ("forty twelve", "40 12"),
            ("a hundred frames", "a hundred frames"),
            ("thousand", "thousand"),
            ("one hundred and", "100 and"),
            ("two hundred three hundred", "203 hundred"),
            ("fortyzero", "fortyzero"),
            ("twelve 12", "12 12"),
        ] {
            assert_eq!(fact_words(text).join(" "), read, "{text}");
        }
    }

    fn fact(term: &str) -> KeyFact {
        KeyFact {
            event: "F00-E01".to_owned(),
            term: term.to_owned(),
            words: fact_words(term),
            alternatives: time_alternatives(term),
        }
    }

    #[test]
    fn key_facts_match_numbers_and_clock_times_in_either_spelling() {
        for (term, text) in [
            ("twelve", "the queue depth is 12"),
            ("twelve", "the queue depth rises to twelve"),
            ("three", "the retry limit says 3"),
            (
                "840 milliseconds",
                "the p95 line reaches eight hundred forty milliseconds",
            ),
            ("840 milliseconds", "it spikes to 840 ms"),
            ("2048", "build two thousand forty-eight"),
            ("120", "the median stays at one hundred and twenty"),
            ("10:32", "the spike is at 10:32"),
            ("10:32", "the spike is at 10.32"),
            ("10:32", "at 10:32:00"),
            ("10:30", "at 10.30"),
        ] {
            assert!(fact(term).stated_in(text), "{term} in {text}");
        }
        for (term, text) in [
            ("10:32", "at 1032"),
            ("10:32", "at 10.33"),
            ("10:32", "at 110.32"),
            ("840 milliseconds", "8 hundred 40 ms"),
            ("twelve", "one two"),
            ("Submit", "the submission button"),
            ("2.5 seconds", "2:5 s"),
        ] {
            assert!(!fact(term).stated_in(text), "{term} in {text}");
        }
        assert!(time_alternatives("2.5 seconds").is_empty());
        assert!(time_alternatives("E-409").is_empty());
        assert_eq!(time_alternatives("at 9:05"), vec![fact_words("at 9.05")]);
        assert!(time_alternatives("ratio 1:100").is_empty());
        assert!(time_alternatives("ab10:32").is_empty());
    }
}
