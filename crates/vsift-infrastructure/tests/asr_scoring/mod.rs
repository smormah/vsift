//! T-04 accuracy scoring for local ASR (P07 increment 3c). Test-only.
//!
//! Expectations come only from the frozen corpus: each speech fixture's
//! reference is its `audio.script` in `fixtures/corpus/manifest.json`, and its
//! critical terms are the manifest's `expected_terms` that are actually spoken
//! in that script (terms only shown on screen, such as F03's `G18`, are not
//! speech evidence). Nothing is derived from a recognizer run.
//!
//! Both reference and hypothesis are normalised the same way before
//! comparison, so formatting a spoken word differently is not an error:
//!
//! - lowercase;
//! - a comma between digits followed by three digits is a thousands separator
//!   and is removed (`2,048` is `2048`);
//! - a hyphen between letters or digits joins an identifier (`E-409` is
//!   `e409`, `AB-731` is `ab731`);
//! - a colon between digits separates two spoken numbers (`10:32` is spoken
//!   "ten thirty-two", so it is `10 32`);
//! - every other character that is not a letter or digit is removed, except a
//!   decimal point between digits;
//! - a decimal number is compared by value (`125.00` is `125`, `127.50` is
//!   `127.5`);
//! - a number word is its value (`twelve` is `12`, `three` is `3`).
//!
//! The word error rate is the word-level edit distance divided by the
//! reference length. A critical term is found when its normalised words, joined
//! without spaces, equal a run of consecutive hypothesis words joined without
//! spaces, so `safe 12` finds `SAFE-12` while `407` never finds `4407`.

// Each test binary that includes this module uses a different part of it.
#![allow(dead_code)]

use serde_json::Value;

/// Reviewed misses of the pinned base model, reported but not failing its
/// critical-term gate (maintainer decision D6): the base model hears "queued"
/// as "Q" (F04), "4407" as "407" (F05), and loses "E-409" in F08's noise. A
/// missed critical term is a known miss when it contains one of these words
/// (F05's critical term is "invoice 4407").
pub const KNOWN_BASE_MISSES: [(&str, &str); 3] =
    [("F04", "queued"), ("F05", "4407"), ("F08", "E-409")];

/// Single-word number names and their values.
const NUMBER_WORDS: [(&str, &str); 28] = [
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
    ("thirty", "30"),
    ("forty", "40"),
    ("fifty", "50"),
    ("sixty", "60"),
    ("seventy", "70"),
    ("eighty", "80"),
    ("ninety", "90"),
];

/// Normalised words of `text` (see the module documentation).
pub fn normalise(text: &str) -> Vec<String> {
    let characters: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let mut spaced = String::with_capacity(characters.len());
    for (index, &character) in characters.iter().enumerate() {
        let before = index.checked_sub(1).and_then(|at| characters.get(at));
        let after = characters.get(index + 1);
        let digit = |neighbour: Option<&char>| neighbour.is_some_and(char::is_ascii_digit);
        let alphanumeric =
            |neighbour: Option<&char>| neighbour.is_some_and(|c| c.is_alphanumeric());
        match character {
            ',' if digit(before) && thousands_group_follows(&characters, index) => {}
            '-' if alphanumeric(before) && alphanumeric(after) => {}
            ':' if digit(before) && digit(after) => spaced.push(' '),
            '.' if digit(before) && digit(after) => spaced.push('.'),
            other if other.is_alphanumeric() => spaced.push(other),
            _ => spaced.push(' '),
        }
    }
    spaced
        .split_whitespace()
        .map(|word| {
            NUMBER_WORDS
                .iter()
                .find(|(name, _)| *name == word)
                .map_or_else(|| canonical_number(word), |(_, value)| (*value).to_owned())
        })
        .collect()
}

/// Whether exactly three digits follow the comma at `index`, then a
/// non-digit or the end.
fn thousands_group_follows(characters: &[char], index: usize) -> bool {
    let group = characters.get(index + 1..index + 4);
    group.is_some_and(|digits| digits.iter().all(char::is_ascii_digit))
        && !characters.get(index + 4).is_some_and(char::is_ascii_digit)
}

/// A decimal number by value: trailing fractional zeros and a bare point are
/// dropped. Any other word is returned unchanged.
fn canonical_number(word: &str) -> String {
    let is_decimal = word.split_once('.').is_some_and(|(whole, fraction)| {
        !whole.is_empty()
            && whole.chars().all(|c| c.is_ascii_digit())
            && fraction.chars().all(|c| c.is_ascii_digit())
    });
    if is_decimal {
        word.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        word.to_owned()
    }
}

/// Word-level edit counts of a hypothesis against a reference.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Edits {
    pub substitutions: usize,
    pub deletions: usize,
    pub insertions: usize,
    pub reference_words: usize,
}

impl Edits {
    pub const fn errors(self) -> usize {
        self.substitutions + self.deletions + self.insertions
    }

    /// Adds another clip's counts, for a pooled rate.
    pub const fn plus(self, other: Self) -> Self {
        Self {
            substitutions: self.substitutions + other.substitutions,
            deletions: self.deletions + other.deletions,
            insertions: self.insertions + other.insertions,
            reference_words: self.reference_words + other.reference_words,
        }
    }

    /// Word error rate in basis points (1/100 of a percent), so gates compare
    /// integers.
    pub fn wer_basis_points(self) -> usize {
        if self.reference_words == 0 {
            return usize::from(self.errors() > 0) * 10_000;
        }
        self.errors() * 10_000 / self.reference_words
    }
}

/// Minimum word edits turning `reference` into `hypothesis`.
pub fn word_edits(reference: &[String], hypothesis: &[String]) -> Edits {
    // cost, substitutions, deletions, insertions for each prefix pair.
    let mut previous: Vec<(usize, usize, usize, usize)> =
        (0..=hypothesis.len()).map(|j| (j, 0, 0, j)).collect();
    for (i, reference_word) in reference.iter().enumerate() {
        let mut current = vec![(i + 1, 0, i + 1, 0)];
        for (j, hypothesis_word) in hypothesis.iter().enumerate() {
            let (Some(&diagonal), Some(&above), Some(&left)) =
                (previous.get(j), previous.get(j + 1), current.get(j))
            else {
                return Edits::default();
            };
            let substitute = if reference_word == hypothesis_word {
                diagonal
            } else {
                (diagonal.0 + 1, diagonal.1 + 1, diagonal.2, diagonal.3)
            };
            let delete = (above.0 + 1, above.1, above.2 + 1, above.3);
            let insert = (left.0 + 1, left.1, left.2, left.3 + 1);
            let best = [substitute, delete, insert]
                .into_iter()
                .min_by_key(|candidate| candidate.0)
                .unwrap_or(substitute);
            current.push(best);
        }
        previous = current;
    }
    let (_, substitutions, deletions, insertions) = previous.last().copied().unwrap_or_default();
    Edits {
        substitutions,
        deletions,
        insertions,
        reference_words: reference.len(),
    }
}

/// Whether `term`'s words, joined without spaces, equal a run of consecutive
/// `words` joined without spaces.
pub fn contains_term(words: &[String], term: &[String]) -> bool {
    let target: String = term.concat();
    if target.is_empty() {
        return false;
    }
    (0..words.len()).any(|start| {
        let mut joined = String::new();
        words.iter().skip(start).any(|word| {
            joined.push_str(word);
            joined == target
        })
    })
}

/// One speech fixture's frozen expectations.
#[derive(Clone, Debug)]
pub struct SpeechExpectation {
    pub fixture: String,
    pub mode: String,
    pub script: String,
    /// Expected terms spoken in the script, in manifest order.
    pub critical_terms: Vec<String>,
    /// Expected terms only shown on screen, excluded from speech scoring.
    pub visual_only_terms: Vec<String>,
}

impl SpeechExpectation {
    /// Whether the fixture's audio carries added noise.
    pub fn noisy(&self) -> bool {
        self.mode == "noisy-synthetic"
    }
}

/// Speech expectations of every fixture with a spoken script, from the
/// manifest alone.
pub fn speech_expectations(manifest: &Value) -> Result<Vec<SpeechExpectation>, String> {
    let fixtures = manifest["fixtures"]
        .as_array()
        .ok_or("manifest has no fixtures")?;
    let mut expectations = Vec::new();
    for fixture in fixtures {
        let script = fixture["audio"]["script"].as_str().unwrap_or_default();
        if script.is_empty() {
            continue;
        }
        let id = fixture["id"].as_str().ok_or("fixture without id")?;
        let spoken = normalise(script);
        let mut critical_terms = Vec::new();
        let mut visual_only_terms = Vec::new();
        for term in fixture["expected_terms"]
            .as_array()
            .ok_or("fixture without expected_terms")?
        {
            let term = term.as_str().ok_or("expected term is not text")?;
            if contains_term(&spoken, &normalise(term)) {
                critical_terms.push(term.to_owned());
            } else {
                visual_only_terms.push(term.to_owned());
            }
        }
        expectations.push(SpeechExpectation {
            fixture: id.to_owned(),
            mode: fixture["audio"]["mode"]
                .as_str()
                .ok_or("fixture without audio mode")?
                .to_owned(),
            script: script.to_owned(),
            critical_terms,
            visual_only_terms,
        });
    }
    Ok(expectations)
}

/// Whether a missed critical `term` of `fixture` contains a reviewed known
/// miss.
pub fn is_known_miss(fixture: &str, term: &str) -> bool {
    KNOWN_BASE_MISSES.iter().any(|(known_fixture, known)| {
        *known_fixture == fixture && contains_term(&normalise(term), &normalise(known))
    })
}

/// The score of one hypothesis against one fixture's expectations.
#[derive(Clone, Debug)]
pub struct FixtureScore {
    pub edits: Edits,
    pub found: Vec<String>,
    pub missed: Vec<String>,
    /// Missed terms on the reviewed base-model known-miss list.
    pub known_misses: Vec<String>,
}

impl FixtureScore {
    /// Missed terms not on the reviewed known-miss list.
    pub fn unexpected_misses(&self) -> Vec<&String> {
        self.missed
            .iter()
            .filter(|term| !self.known_misses.contains(term))
            .collect()
    }
}

/// Scores `hypothesis` for `expectation`.
pub fn score(expectation: &SpeechExpectation, hypothesis: &str) -> FixtureScore {
    let heard = normalise(hypothesis);
    let edits = word_edits(&normalise(&expectation.script), &heard);
    let (found, missed): (Vec<String>, Vec<String>) = expectation
        .critical_terms
        .iter()
        .cloned()
        .partition(|term| contains_term(&heard, &normalise(term)));
    let known_misses = missed
        .iter()
        .filter(|term| is_known_miss(&expectation.fixture, term))
        .cloned()
        .collect();
    FixtureScore {
        edits,
        found,
        missed,
        known_misses,
    }
}
