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
    /// sentence states, normalised ([`normalize`]).
    ///
    /// # Errors
    ///
    /// As [`CorpusTruth::event`].
    pub fn key_facts(&self, event_id: &str) -> Result<Vec<KeyFact>, TrialError> {
        let (fixture, event) = self.event(event_id)?;
        let truth = normalize(&event.truth);
        Ok(fixture
            .expected_terms
            .iter()
            .filter_map(|term| {
                let words = normalize(term);
                (!words.is_empty() && contains_sequence(&truth, &words)).then(|| KeyFact {
                    event: event.id.clone(),
                    term: term.clone(),
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
    /// The term normalised.
    pub words: Vec<String>,
}

impl KeyFact {
    /// Whether `text` states this fact.
    #[must_use]
    pub fn stated_in(&self, text: &str) -> bool {
        contains_sequence(&normalize(text), &self.words)
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

/// Normalises text for fact matching, close to the CLI's search rules:
/// lower case; a hyphen between letters or digits joins them (`E-409` is
/// `e409`); a colon between digits separates (`10:32` is `10 32`); a
/// decimal loses trailing zeros (`127.50` is `127.5`); number words up to
/// twenty are digits; `milliseconds` is `ms` and `seconds` is `s`; any
/// other punctuation separates words.
#[must_use]
pub fn normalize(text: &str) -> Vec<String> {
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
            let word = match word {
                "milliseconds" | "millisecond" => "ms",
                "seconds" | "second" => "s",
                other => other,
            };
            NUMBER_WORDS
                .iter()
                .find(|(name, _)| *name == word)
                .map_or_else(|| word.to_owned(), |(_, digits)| (*digits).to_owned())
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
    }
}
