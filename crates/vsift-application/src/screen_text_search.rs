//! Bounded, pageable literal search of one session's screen-text index
//! (`ocr search`, ADR 0030 decision 13).
//!
//! The domain decides what matches, how a line held over consecutive frames is
//! one occurrence, and in which order occurrences rank; this use case binds a
//! page to its continuation cursor. It needs no transcript: it reads one
//! session's index and nothing else, so a later search across kept sessions
//! calls it once per session and merges the pages.
//!
//! A cursor is bound to the session, the index revision (its immutable
//! snapshot), a digest of the index identity, range and normalised query words,
//! and an expiry. When `ocr` commits a new revision the cursor is rejected
//! rather than silently restarting, as a candidates cursor is when a window is
//! analysed.

use vsift_domain::{
    CursorError, CursorToken, MAX_SCREEN_TEXT_LINES, PageLimit, QueryDigest, ScreenTextIndex,
    ScreenTextIndexId, ScreenTextOccurrence, SearchMatch, SearchPosition, SearchQuery, SessionId,
    TimeRange, screen_text_occurrences,
};

use crate::transcript::sha256_hex;

/// Separates the tier from the ordinal in a cursor's last-item key.
const KEY_SEPARATOR: char = '-';

/// A bounded search of one session's screen-text index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTextPageRequest {
    /// The validated literal query.
    pub query: SearchQuery,
    /// Half-open source range; only frames whose time lies in it are searched.
    /// `None` searches the whole index.
    pub range: Option<TimeRange>,
    /// Maximum occurrences on this page.
    pub limit: PageLimit,
    /// Opaque continuation token from the previous page of the same search.
    pub cursor: Option<String>,
}

/// One bounded page of on-screen occurrences.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTextPage<'a> {
    /// Occurrences in rank order: phrase before all-terms, each by first appearance.
    pub hits: Vec<ScreenTextOccurrence<'a>>,
    /// How many occurrences the whole search has, on every page.
    pub total_hits: usize,
    /// Token for the next page, absent on the last page.
    pub next_cursor: Option<CursorToken>,
}

/// Returns one page of `request` over the screen-text `index` of `session_id`.
///
/// # Errors
///
/// [`CursorError`] for a malformed or expired cursor, or one of another
/// session, index revision, range or query.
pub fn page_screen_text<'a>(
    session_id: &SessionId,
    index: &'a ScreenTextIndex,
    request: &ScreenTextPageRequest,
    cursor_expires_at_micros: u64,
    now_micros: u64,
) -> Result<ScreenTextPage<'a>, CursorError> {
    let digest = screen_text_query_digest(index.id(), request.range, &request.query)?;
    let snapshot = u64::from(index.revision().get());
    let after = match &request.cursor {
        None => None,
        Some(encoded) => {
            let token = CursorToken::parse(encoded)?;
            token.validate_scope(session_id, snapshot, &digest, now_micros)?;
            Some(parse_position(token.last_item_key())?)
        }
    };
    let occurrences = screen_text_occurrences(index, &request.query, request.range);
    let total_hits = occurrences.len();
    let limit = usize::from(request.limit.get());
    let mut hits: Vec<ScreenTextOccurrence<'a>> = occurrences
        .into_iter()
        .filter(|hit| after.is_none_or(|after| hit.position() > after))
        .take(limit.saturating_add(1))
        .collect();
    let has_more = hits.len() > limit;
    hits.truncate(limit);
    let next_cursor = match (has_more, hits.last()) {
        (true, Some(last)) => Some(CursorToken::new(
            session_id.clone(),
            snapshot,
            digest,
            position_key(last.position()),
            cursor_expires_at_micros,
        )?),
        _ => None,
    };
    Ok(ScreenTextPage {
        hits,
        total_hits,
        next_cursor,
    })
}

/// Digest of the canonical on-screen search: index identity, range and
/// normalised query words, one a line (they hold no line break).
///
/// # Errors
///
/// [`CursorError::InvalidQueryDigest`] only if the digest were not canonical,
/// which would be an internal fault.
pub fn screen_text_query_digest(
    index: &ScreenTextIndexId,
    range: Option<TimeRange>,
    query: &SearchQuery,
) -> Result<QueryDigest, CursorError> {
    let (from, to) = range.map_or_else(
        || (String::from("-"), String::from("-")),
        |range| {
            (
                range.start().as_micros().to_string(),
                range.end().as_micros().to_string(),
            )
        },
    );
    let mut material = format!("vsift.ocr-search.v1\n{}\n{from}\n{to}", index.as_str());
    for term in query.terms() {
        material.push('\n');
        material.push_str(term);
    }
    QueryDigest::parse(sha256_hex(material.as_bytes()))
}

/// The cursor key of a rank position: `<tier>-<ordinal>`, for example `1-42`.
fn position_key(position: SearchPosition) -> String {
    format!(
        "{}{KEY_SEPARATOR}{}",
        position.tier().tier(),
        position.ordinal()
    )
}

/// Parses a canonical `<tier>-<ordinal>` key within the index's bounds.
fn parse_position(key: &str) -> Result<SearchPosition, CursorError> {
    let invalid = CursorError::InvalidLastItemKey;
    let (tier, ordinal) = key.split_once(KEY_SEPARATOR).ok_or(invalid)?;
    let tier = match tier {
        "1" => SearchMatch::Phrase,
        "2" => SearchMatch::AllTerms,
        _ => return Err(invalid),
    };
    let canonical = !ordinal.starts_with('0') && ordinal.bytes().all(|byte| byte.is_ascii_digit());
    let ordinal = ordinal
        .parse::<u32>()
        .ok()
        .filter(|ordinal| {
            canonical
                && *ordinal >= 1
                && usize::try_from(*ordinal).is_ok_and(|ordinal| ordinal <= MAX_SCREEN_TEXT_LINES)
        })
        .ok_or(invalid)?;
    Ok(SearchPosition::new(tier, ordinal))
}

#[cfg(test)]
mod tests;
