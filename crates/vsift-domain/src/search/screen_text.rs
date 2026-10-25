//! Search over on-screen text (OCR, ADR 0030).
//!
//! The same query rules as the transcript's apply to every line read off a
//! frame. A label on screen is usually read on many frames in a row, so hits
//! are *occurrences*: the same words at the same place on consecutive read
//! frames are one hit, which names its first frame and says until when, and on
//! how many frames, the text stayed. Hits rank by tier, then by the time they
//! first appear, then by reading order on that frame.

use crate::{
    MediaTime, PageLimit, ScreenTextFrame, ScreenTextIndex, ScreenTextLine, ScreenTextOutcome,
    TextRegion, TimeRange,
};

use super::{SearchMatch, SearchPosition, SearchQuery, normalise_search_text};

/// One run of a matching line over consecutive read frames.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenTextOccurrence<'a> {
    /// The first frame the text was read on.
    pub first: &'a ScreenTextFrame,
    /// The line as read on that frame.
    pub line: &'a ScreenTextLine,
    /// The time of the last consecutive read frame that held the same text there.
    pub last_time: MediaTime,
    /// How many consecutive read frames held it.
    pub frames: u32,
    /// The tier it matched in.
    pub tier: SearchMatch,
    /// Its 1-based rank within its tier.
    pub ordinal: u32,
}

impl ScreenTextOccurrence<'_> {
    /// The hit's rank position, as a cursor records it.
    #[must_use]
    pub const fn position(&self) -> SearchPosition {
        SearchPosition::new(self.tier, self.ordinal)
    }
}

/// One bounded page of screen-text hits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTextSlice<'a> {
    /// Hits in rank order.
    pub hits: Vec<ScreenTextOccurrence<'a>>,
    /// Whether a further hit ranks after the last one.
    pub has_more: bool,
}

struct Open<'a> {
    first: &'a ScreenTextFrame,
    line: &'a ScreenTextLine,
    words: Vec<String>,
    region: TextRegion,
    last_time: MediaTime,
    last_seen: usize,
    frames: u32,
    tier: SearchMatch,
}

/// Returns at most `limit` occurrences of `query` in `index`, after `after`.
///
/// With a `window`, only frames whose time lies in it are searched. Failed
/// frames hold no text and do not break a run.
#[must_use]
pub fn search_screen_text<'a>(
    index: &'a ScreenTextIndex,
    query: &SearchQuery,
    window: Option<TimeRange>,
    after: Option<SearchPosition>,
    limit: PageLimit,
) -> ScreenTextSlice<'a> {
    let limit = usize::from(limit.get());
    let mut hits: Vec<ScreenTextOccurrence<'a>> = screen_text_occurrences(index, query, window)
        .into_iter()
        .filter(|hit| after.is_none_or(|after| hit.position() > after))
        .take(limit.saturating_add(1))
        .collect();
    let has_more = hits.len() > limit;
    hits.truncate(limit);
    ScreenTextSlice { hits, has_more }
}

/// Every occurrence of `query` in `index` (in `window`), in rank order. The
/// index's own bounds bound the work: at most 200,000 lines are classified.
#[must_use]
pub fn screen_text_occurrences<'a>(
    index: &'a ScreenTextIndex,
    query: &SearchQuery,
    window: Option<TimeRange>,
) -> Vec<ScreenTextOccurrence<'a>> {
    let mut runs: Vec<Open<'a>> = Vec::new();
    let read_frames = index
        .frames()
        .iter()
        .filter(|frame| {
            window.is_none_or(|window| {
                window.start() <= frame.frame.time && frame.frame.time < window.end()
            })
        })
        .filter(|frame| matches!(frame.outcome, ScreenTextOutcome::Read { .. }));
    for (seen, frame) in read_frames.enumerate() {
        let mut extended = vec![false; runs.len()];
        for line in frame.lines() {
            let Some(tier) = query.classify(line.text.as_str()) else {
                continue;
            };
            let words = normalise_search_text(line.text.as_str());
            let continuing = runs.iter().enumerate().position(|(at, run)| {
                !extended.get(at).copied().unwrap_or(true)
                    && seen > 0
                    && run.last_seen == seen - 1
                    && run.words == words
                    && run.region.same_place(line.region)
            });
            if let Some(at) = continuing {
                if let (Some(run), Some(mark)) = (runs.get_mut(at), extended.get_mut(at)) {
                    run.last_time = frame.frame.time;
                    run.last_seen = seen;
                    run.region = line.region;
                    run.frames = run.frames.saturating_add(1);
                    *mark = true;
                }
            } else {
                runs.push(Open {
                    first: frame,
                    line,
                    words,
                    region: line.region,
                    last_time: frame.frame.time,
                    last_seen: seen,
                    frames: 1,
                    tier,
                });
            }
        }
    }
    let mut ranked: Vec<ScreenTextOccurrence<'a>> = Vec::new();
    for tier in SearchMatch::ALL {
        let mut ordinal = 0_u32;
        for run in runs.iter().filter(|run| run.tier == tier) {
            ordinal = ordinal.saturating_add(1);
            ranked.push(ScreenTextOccurrence {
                first: run.first,
                line: run.line,
                last_time: run.last_time,
                frames: run.frames,
                tier,
                ordinal,
            });
        }
    }
    ranked
}
