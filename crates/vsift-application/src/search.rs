//! Bounded, pageable literal search of one committed transcript revision.
//!
//! The domain decides what matches and in which order; this use case binds a
//! page to its continuation cursor. Search is computed on demand from the
//! immutable revision, so a cursor stays valid for as long as the revision and
//! the session do, and the same request always returns the same page
//! (ADR 0018).

use vsift_domain::{
    CursorError, CursorToken, PageLimit, QueryDigest, SearchCoverage, SearchHit, SearchMatch,
    SearchPosition, SearchQuery, SessionId, TimeRange, TranscriptRevision, TranscriptRevisionId,
    search_revision,
};

use crate::{TranscriptQueryError, transcript::sha256_hex};

/// Separates the tier from the ordinal in a cursor's last-item key.
const KEY_SEPARATOR: char = '-';

/// A bounded search request over one revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchPageRequest {
    /// The validated literal query.
    pub query: SearchQuery,
    /// Half-open source range; only segments intersecting it are searched.
    /// `None` searches the whole revision.
    pub range: Option<TimeRange>,
    /// Maximum hits on this page.
    pub limit: PageLimit,
    /// Opaque continuation token from the previous page of the same query.
    pub cursor: Option<String>,
}

/// One bounded page of search hits with the coverage of the searched range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchPage<'a> {
    /// Hits in rank order: tier, then start, then ordinal.
    pub hits: Vec<SearchHit<'a>>,
    /// Token for the next page, absent on the last page.
    pub next_cursor: Option<CursorToken>,
    /// What the searched range's transcript covers and does not.
    pub coverage: SearchCoverage,
}

/// Returns one page of `request` over `revision`.
///
/// A cursor is bound to the session, the revision number (its immutable
/// snapshot), a digest of the revision identity, range and normalised query
/// words, and an expiry; its last-item key is the rank position
/// `<tier>-<ordinal>` of the previous page's last hit. A cursor from any other
/// query, session or revision, or an expired one, is rejected rather than
/// silently restarting. The page size is not part of the digest, so a caller
/// may change it between pages.
///
/// # Errors
///
/// Returns [`TranscriptQueryError::Cursor`] for a rejected cursor.
pub fn page_search<'a>(
    session_id: &SessionId,
    revision: &'a TranscriptRevision,
    request: &SearchPageRequest,
    cursor_expires_at_micros: u64,
    now_micros: u64,
) -> Result<SearchPage<'a>, TranscriptQueryError> {
    let digest = search_query_digest(revision.id(), request.range, &request.query)
        .map_err(TranscriptQueryError::Cursor)?;
    let snapshot = u64::from(revision.number());
    let after = match &request.cursor {
        None => None,
        Some(encoded) => {
            let token = CursorToken::parse(encoded).map_err(TranscriptQueryError::Cursor)?;
            token
                .validate_scope(session_id, snapshot, &digest, now_micros)
                .map_err(TranscriptQueryError::Cursor)?;
            Some(parse_position(token.last_item_key(), revision)?)
        }
    };
    let slice = search_revision(
        revision,
        &request.query,
        request.range,
        after,
        request.limit,
    );
    let next_cursor = match (slice.has_more, slice.hits.last()) {
        (true, Some(last)) => Some(
            CursorToken::new(
                session_id.clone(),
                snapshot,
                digest,
                position_key(last.position()),
                cursor_expires_at_micros,
            )
            .map_err(TranscriptQueryError::Cursor)?,
        ),
        _ => None,
    };
    Ok(SearchPage {
        hits: slice.hits,
        next_cursor,
        coverage: SearchCoverage::of(revision, request.range),
    })
}

/// The cursor key of a rank position: `<tier>-<ordinal>`, for example `1-42`.
fn position_key(position: SearchPosition) -> String {
    format!(
        "{}{KEY_SEPARATOR}{}",
        position.tier().tier(),
        position.ordinal()
    )
}

/// Parses a canonical `<tier>-<ordinal>` key naming a segment of `revision`.
fn parse_position(
    key: &str,
    revision: &TranscriptRevision,
) -> Result<SearchPosition, TranscriptQueryError> {
    let invalid = TranscriptQueryError::Cursor(CursorError::InvalidLastItemKey);
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
        .filter(|ordinal| canonical && *ordinal >= 1)
        .ok_or(invalid)?;
    if usize::try_from(ordinal).map_or(true, |ordinal| ordinal > revision.segments().len()) {
        return Err(invalid);
    }
    Ok(SearchPosition::new(tier, ordinal))
}

/// Digest of the canonical search: revision identity, range and normalised
/// query words.
///
/// Normalised words hold only letters, digits and points, never a line
/// break, so one word per line is an unambiguous encoding. Two queries that
/// normalise to the same words are the same search and share cursors.
///
/// # Errors
///
/// Returns [`CursorError::InvalidQueryDigest`] only if the digest were not
/// canonical, which would be an internal fault.
pub fn search_query_digest(
    revision: &TranscriptRevisionId,
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
    let mut material = format!("vsift.search.v1\n{}\n{from}\n{to}", revision.as_str());
    for term in query.terms() {
        material.push('\n');
        material.push_str(term);
    }
    QueryDigest::parse(sha256_hex(material.as_bytes()))
}

#[cfg(test)]
mod tests;
