//! Literal transcript search: query normalisation, matching, ranking and the
//! honest coverage of what was searched.
//!
//! Search is computed on demand from one immutable transcript revision
//! (ADR 0018); there is no persisted index. Every rule that decides whether a
//! segment matches, and in which order matches are returned, lives here so the
//! engine, the contract and the fuzz harness share one implementation.
//!
//! # Normalisation
//!
//! Query text and segment text are normalised the same way into words, so a
//! different spelling of the same spoken token still matches:
//!
//! - letters are lowercased (Unicode simple and special lowercasing from the
//!   standard library);
//! - a comma after an ASCII digit and followed by exactly three ASCII digits
//!   is a thousands separator and is removed (`2,048` is `2048`);
//! - a hyphen between two letters or digits joins them (`E-409` is `e409`,
//!   `AB-731` is `ab731`);
//! - a colon between two ASCII digits separates two numbers (`10:32` is
//!   `10 32`);
//! - a full stop between two ASCII digits is a decimal point, and a decimal
//!   is compared by value (`125.00` is `125`, `127.50` is `127.5`);
//! - the number words `zero` to `twenty` and the tens `thirty` to `ninety`
//!   are their digits (`twelve` is `12`); compounds such as `thirty-two` are
//!   joined by the hyphen rule and are not converted;
//! - every other character that is not a letter or digit separates words.
//!
//! No Unicode normalisation (NFC/NFKC) or accent folding is applied: it needs
//! Unicode tables the standard library does not provide, so a precomposed and
//! a decomposed spelling of the same accented word are different words. The
//! same text always normalises to the same words, and normalising the words
//! of a normalised text again changes nothing.
//!
//! # Matching and ranking
//!
//! A segment matches in one of two tiers, decided within that one segment:
//!
//! 1. **phrase:** the query words joined without spaces equal a run of
//!    consecutive segment words joined without spaces, so `AB 731` finds
//!    `AB-731` and `dialog r 17` finds `Dialog R-17`, while `407` never finds
//!    `4407` (a word is never matched in part);
//! 2. **all terms:** every query word equals some segment word, in any order.
//!
//! A phrase that continues from one segment into the next is not found; each
//! segment is matched alone. Hits are ranked by tier, then by segment start,
//! then by segment ordinal. A revision's ordinals follow start order, so the
//! rank is exactly (tier, ordinal), which is what a continuation cursor
//! records.

mod coverage;

#[cfg(test)]
mod tests;

use std::{error::Error, fmt};

pub use coverage::{CoverageBasis, SearchCoverage};

use crate::{PageLimit, TimeRange, TranscriptRevision, TranscriptSegment};

/// Largest accepted query, in UTF-8 bytes, before normalisation.
pub const MAX_SEARCH_QUERY_BYTES: usize = 256;

/// Largest number of words a normalised query may have.
pub const MAX_SEARCH_TERMS: usize = 16;

/// Single-word number names and the digits they are compared as.
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

/// Returns the normalised words of `text` (see the module documentation).
///
/// Every word is non-empty and holds only letters, digits and, between two
/// ASCII digits, a decimal point.
#[must_use]
pub fn normalise_search_text(text: &str) -> Vec<String> {
    let mut words = Words::default();
    words.fill(text);
    words.iter().map(str::to_owned).collect()
}

/// Why a query was rejected before any transcript was read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchQueryRejection {
    /// The query has no letter or digit, so it has no words to search for.
    Empty,
    /// The query is longer than [`MAX_SEARCH_QUERY_BYTES`].
    TooLong,
    /// The query has more than [`MAX_SEARCH_TERMS`] words once normalised.
    TooManyTerms,
    /// The query contains a control character (including tab and newline).
    ControlCharacter,
}

impl SearchQueryRejection {
    /// Every rejection, in declaration order.
    pub const ALL: [Self; 4] = [
        Self::Empty,
        Self::TooLong,
        Self::TooManyTerms,
        Self::ControlCharacter,
    ];

    /// Stable reason identifier used in public remediation.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::TooLong => "too_long",
            Self::TooManyTerms => "too_many_terms",
            Self::ControlCharacter => "control_character",
        }
    }
}

impl fmt::Display for SearchQueryRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "search query has no letters or digits",
            Self::TooLong => "search query exceeds 256 bytes",
            Self::TooManyTerms => "search query has more than 16 words",
            Self::ControlCharacter => "search query contains a control character",
        })
    }
}

impl Error for SearchQueryRejection {}

/// A validated, normalised literal query.
///
/// The query is data only: it is never interpreted as a pattern, regular
/// expression or command, so no input can change what the search executes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchQuery {
    terms: Vec<String>,
    phrase: String,
}

impl SearchQuery {
    /// Validates and normalises `text`.
    ///
    /// The byte bound is checked first, so a long input is rejected before
    /// any work proportional to it; control characters are rejected before
    /// normalisation would turn them into separators and hide them.
    ///
    /// # Errors
    ///
    /// Returns the [`SearchQueryRejection`] that applies.
    pub fn parse(text: &str) -> Result<Self, SearchQueryRejection> {
        if text.len() > MAX_SEARCH_QUERY_BYTES {
            return Err(SearchQueryRejection::TooLong);
        }
        if text.chars().any(char::is_control) {
            return Err(SearchQueryRejection::ControlCharacter);
        }
        let terms = normalise_search_text(text);
        if terms.is_empty() {
            return Err(SearchQueryRejection::Empty);
        }
        if terms.len() > MAX_SEARCH_TERMS {
            return Err(SearchQueryRejection::TooManyTerms);
        }
        let phrase = terms.concat();
        Ok(Self { terms, phrase })
    }

    /// The normalised query words, in query order.
    #[must_use]
    pub fn terms(&self) -> &[String] {
        &self.terms
    }

    /// Classifies `text` against this query, or `None` when it does not match.
    #[must_use]
    pub fn classify(&self, text: &str) -> Option<SearchMatch> {
        Matcher::new(self).classify(text)
    }
}

/// The tier in which a segment matched; earlier tiers rank first.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SearchMatch {
    /// Tier 1: the whole query occurs as consecutive words.
    Phrase,
    /// Tier 2: every query word occurs, in any order.
    AllTerms,
}

impl SearchMatch {
    /// Every tier, in rank order.
    pub const ALL: [Self; 2] = [Self::Phrase, Self::AllTerms];

    /// Stable identifier written to a hit's `match` field.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Phrase => "phrase",
            Self::AllTerms => "all_terms",
        }
    }

    /// The 1-based tier number, as a continuation cursor records it.
    #[must_use]
    pub const fn tier(self) -> u8 {
        match self {
            Self::Phrase => 1,
            Self::AllTerms => 2,
        }
    }

    /// The tier with number `tier`, if any.
    #[must_use]
    pub const fn from_tier(tier: u8) -> Option<Self> {
        match tier {
            1 => Some(Self::Phrase),
            2 => Some(Self::AllTerms),
            _ => None,
        }
    }
}

/// The rank of one hit: its tier, then its segment ordinal.
///
/// Ordering positions orders hits exactly as a search returns them, so the
/// position of the last hit on a page is all a cursor needs to continue.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SearchPosition {
    tier: SearchMatch,
    ordinal: u32,
}

impl SearchPosition {
    /// The position of the segment at `ordinal` matched in `tier`.
    #[must_use]
    pub const fn new(tier: SearchMatch, ordinal: u32) -> Self {
        Self { tier, ordinal }
    }

    /// The tier of the hit.
    #[must_use]
    pub const fn tier(self) -> SearchMatch {
        self.tier
    }

    /// The 1-based ordinal of the hit's segment in its revision.
    #[must_use]
    pub const fn ordinal(self) -> u32 {
        self.ordinal
    }
}

/// One matching segment and the tier it matched in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchHit<'a> {
    segment: &'a TranscriptSegment,
    tier: SearchMatch,
}

impl<'a> SearchHit<'a> {
    /// The matching segment, unchanged from its revision.
    #[must_use]
    pub const fn segment(&self) -> &'a TranscriptSegment {
        self.segment
    }

    /// The tier in which it matched.
    #[must_use]
    pub const fn tier(&self) -> SearchMatch {
        self.tier
    }

    /// The hit's rank position.
    #[must_use]
    pub const fn position(&self) -> SearchPosition {
        SearchPosition::new(self.tier, self.segment.ordinal())
    }
}

/// One bounded page of hits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchSlice<'a> {
    /// Hits in rank order.
    pub hits: Vec<SearchHit<'a>>,
    /// Whether a further hit ranks after the last one.
    pub has_more: bool,
}

/// Returns at most `limit` hits of `query` in `revision`, ranked by tier and
/// ordinal, after position `after` when continuing a previous page.
///
/// With a `window`, only segments intersecting it (starting before its end
/// and ending after its start) are searched. The scan stops as soon as the
/// page and the "more" flag are decided, so early pages of a common query do
/// not read the whole revision.
#[must_use]
pub fn search_revision<'a>(
    revision: &'a TranscriptRevision,
    query: &SearchQuery,
    window: Option<TimeRange>,
    after: Option<SearchPosition>,
    limit: PageLimit,
) -> SearchSlice<'a> {
    let limit = usize::from(limit.get());
    let wanted = limit.saturating_add(1);
    let mut matcher = Matcher::new(query);
    let segments = revision.segments();
    let mut hits = match after.map(SearchPosition::tier) {
        Some(SearchMatch::AllTerms) => {
            let skip = after.map_or(0, |position| {
                usize::try_from(position.ordinal()).unwrap_or(usize::MAX)
            });
            let mut all_terms = Vec::new();
            for segment in in_window(segments.iter().skip(skip), window) {
                if matcher.classify(segment.text().text()) == Some(SearchMatch::AllTerms) {
                    all_terms.push(SearchHit {
                        segment,
                        tier: SearchMatch::AllTerms,
                    });
                    if all_terms.len() == wanted {
                        break;
                    }
                }
            }
            all_terms
        }
        None | Some(SearchMatch::Phrase) => {
            let phrase_after = after.map_or(0, SearchPosition::ordinal);
            let mut phrases = Vec::new();
            let mut all_terms = Vec::new();
            for segment in in_window(segments.iter(), window) {
                match matcher.classify(segment.text().text()) {
                    Some(SearchMatch::Phrase) if segment.ordinal() > phrase_after => {
                        phrases.push(SearchHit {
                            segment,
                            tier: SearchMatch::Phrase,
                        });
                        if phrases.len() == wanted {
                            break;
                        }
                    }
                    Some(SearchMatch::AllTerms) if all_terms.len() < wanted => {
                        all_terms.push(SearchHit {
                            segment,
                            tier: SearchMatch::AllTerms,
                        });
                    }
                    Some(SearchMatch::Phrase | SearchMatch::AllTerms) | None => {}
                }
            }
            phrases.extend(all_terms);
            phrases
        }
    };
    let has_more = hits.len() > limit;
    hits.truncate(limit);
    SearchSlice { hits, has_more }
}

/// Segments of `segments` (in start order) that intersect `window`, or all of
/// them without a window.
fn in_window<'a>(
    segments: impl Iterator<Item = &'a TranscriptSegment>,
    window: Option<TimeRange>,
) -> impl Iterator<Item = &'a TranscriptSegment> {
    segments
        .take_while(move |segment| {
            window.is_none_or(|window| segment.range().start() < window.end())
        })
        .filter(move |segment| window.is_none_or(|window| segment.intersects(window)))
}

/// Classifies texts against one query, reusing its word buffers.
struct Matcher<'q> {
    query: &'q SearchQuery,
    words: Words,
}

impl<'q> Matcher<'q> {
    fn new(query: &'q SearchQuery) -> Self {
        Self {
            query,
            words: Words::default(),
        }
    }

    fn classify(&mut self, text: &str) -> Option<SearchMatch> {
        self.words.fill(text);
        if self.contains_phrase() {
            return Some(SearchMatch::Phrase);
        }
        let all_terms = self
            .query
            .terms
            .iter()
            .all(|term| self.words.iter().any(|word| word == term));
        all_terms.then_some(SearchMatch::AllTerms)
    }

    /// Whether the query words joined without spaces equal a run of
    /// consecutive words joined without spaces. Words are never empty, so
    /// every step consumes part of the phrase and the scan is bounded by the
    /// number of words times the phrase length.
    fn contains_phrase(&self) -> bool {
        let phrase = self.query.phrase.as_str();
        (0..self.words.len()).any(|start| {
            let mut rest = phrase;
            for word in self.words.iter().skip(start) {
                match rest.strip_prefix(word) {
                    Some("") => return true,
                    Some(remaining) => rest = remaining,
                    None => return false,
                }
            }
            false
        })
    }
}

/// Reusable buffers holding the normalised words of one text.
///
/// Searching normalises every segment of a revision on each request, so the
/// buffers are kept across segments instead of allocating one string per word.
#[derive(Debug, Default)]
struct Words {
    lowered: Vec<char>,
    spaced: String,
    text: String,
    bounds: Vec<(usize, usize)>,
}

impl Words {
    fn fill(&mut self, source: &str) {
        self.lowered.clear();
        self.lowered
            .extend(source.chars().flat_map(char::to_lowercase));
        self.spaced.clear();
        for (index, &character) in self.lowered.iter().enumerate() {
            let before = index.checked_sub(1).and_then(|at| self.lowered.get(at));
            let after = self.lowered.get(index + 1);
            let digit = |neighbour: Option<&char>| neighbour.is_some_and(char::is_ascii_digit);
            let alphanumeric =
                |neighbour: Option<&char>| neighbour.is_some_and(|value| value.is_alphanumeric());
            match character {
                ',' if digit(before) && thousands_group_follows(&self.lowered, index) => {}
                '-' if alphanumeric(before) && alphanumeric(after) => {}
                '.' if digit(before) && digit(after) => self.spaced.push('.'),
                other if other.is_alphanumeric() => self.spaced.push(other),
                _ => self.spaced.push(' '),
            }
        }
        self.text.clear();
        self.bounds.clear();
        for word in self.spaced.split_whitespace() {
            let start = self.text.len();
            self.text.push_str(canonical_word(word));
            self.bounds.push((start, self.text.len()));
        }
    }

    fn len(&self) -> usize {
        self.bounds.len()
    }

    fn iter(&self) -> impl Iterator<Item = &str> {
        self.bounds
            .iter()
            .filter_map(|&(start, end)| self.text.get(start..end))
    }
}

/// Whether exactly three ASCII digits follow the comma at `index`, then a
/// character that is not an ASCII digit, or the end.
fn thousands_group_follows(characters: &[char], index: usize) -> bool {
    let group = characters.get(index + 1..index + 4);
    group.is_some_and(|digits| digits.iter().all(char::is_ascii_digit))
        && !characters.get(index + 4).is_some_and(char::is_ascii_digit)
}

/// The comparable form of one word: a number word's digits, a decimal by
/// value, or the word itself.
fn canonical_word(word: &str) -> &str {
    if let Some((_, digits)) = NUMBER_WORDS.iter().find(|(name, _)| *name == word) {
        return digits;
    }
    let is_decimal = word.split_once('.').is_some_and(|(whole, fraction)| {
        !whole.is_empty()
            && whole.bytes().all(|byte| byte.is_ascii_digit())
            && !fraction.is_empty()
            && fraction.bytes().all(|byte| byte.is_ascii_digit())
    });
    if is_decimal {
        word.trim_end_matches('0').trim_end_matches('.')
    } else {
        word
    }
}
