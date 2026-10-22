//! Reading what a revision published since an earlier read (ADR 0033).
//!
//! A run that publishes as it goes grows its revision: every part adds
//! segments after the ones before, and the sealed revision adds the rest. A
//! reader that keeps the `since` token of its last answer asks for what came
//! after it, by `transcript get --since` or `search --since`, instead of
//! reading everything again.
//!
//! A since token is a scoped [`CursorToken`]: bound to the session, to one
//! revision (its number, which no other revision of the session has, as the
//! token's snapshot, and a digest of its identity) and to a watermark (how many
//! of the revision's segments the reader has been given), and it expires with
//! the session. It follows that one revision until it is sealed or stopped,
//! never another: a reader learns from the answer's state when it ends. The
//! same token serves both commands.

use vsift_domain::{
    CursorError, CursorToken, PageLimit, QueryDigest, SearchMatch, SearchQuery, SessionId,
    TimeRange, TranscriptRevision, TranscriptRevisionId, TranscriptSegment,
};

use crate::transcript::sha256_hex;

/// What a since read answers: the segments after the watermark, the token to
/// read on from, and whether more is published after it already.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SincePage<'a> {
    /// The segments, in start order, each with the tier it matched in for a
    /// search (`None` for a read).
    pub segments: Vec<(&'a TranscriptSegment, Option<SearchMatch>)>,
    /// The token to read on from.
    pub next: CursorToken,
    /// Whether the read stopped at its limit with more published after it.
    pub more: bool,
}

/// Digest that binds a since token to one revision.
///
/// # Errors
///
/// [`CursorError::InvalidQueryDigest`] only through an internal fault.
pub fn since_digest(revision: &TranscriptRevisionId) -> Result<QueryDigest, CursorError> {
    QueryDigest::parse(sha256_hex(
        format!("vsift.since.v1\n{}", revision.as_str()).as_bytes(),
    ))
}

/// The token for what `revision` publishes after its first `watermark`
/// segments.
///
/// # Errors
///
/// [`CursorError`] when the token cannot be formed (an expiry of zero).
pub fn since_token(
    session_id: &SessionId,
    revision: &TranscriptRevision,
    watermark: usize,
    expires_at_micros: u64,
) -> Result<CursorToken, CursorError> {
    CursorToken::new(
        session_id.clone(),
        u64::from(revision.number()),
        since_digest(revision.id())?,
        watermark.to_string(),
        expires_at_micros,
    )
}

/// Parses `encoded` as a since token of `session_id`, unexpired at
/// `now_micros`, and returns the number of the revision it follows.
///
/// # Errors
///
/// [`CursorError`] for a malformed, expired or foreign token.
pub fn since_revision(
    session_id: &SessionId,
    encoded: &str,
    now_micros: u64,
) -> Result<(u32, CursorToken), CursorError> {
    let token = CursorToken::parse(encoded)?;
    // The scope is checked against the token's own snapshot and digest here;
    // [`since_watermark`] checks them against the revision it names.
    let number = u32::try_from(token.generation()).map_err(|_| CursorError::InvalidNumber)?;
    token.validate_scope(
        session_id,
        token.generation(),
        token.query_digest(),
        now_micros,
    )?;
    Ok((number, token))
}

/// The watermark `token` holds for `revision`, once the token is shown to be
/// that revision's.
///
/// # Errors
///
/// [`CursorError::WrongGeneration`] or [`CursorError::WrongQuery`] for a token
/// of another revision, and [`CursorError::InvalidLastItemKey`] for a watermark
/// that is not canonical or lies beyond what the revision holds.
pub fn since_watermark(
    token: &CursorToken,
    revision: &TranscriptRevision,
) -> Result<usize, CursorError> {
    if token.generation() != u64::from(revision.number()) {
        return Err(CursorError::WrongGeneration);
    }
    if *token.query_digest() != since_digest(revision.id())? {
        return Err(CursorError::WrongQuery);
    }
    let key = token.last_item_key();
    let canonical =
        key == "0" || (!key.starts_with('0') && key.bytes().all(|b| b.is_ascii_digit()));
    key.parse::<usize>()
        .ok()
        .filter(|watermark| canonical && *watermark <= revision.segments().len())
        .ok_or(CursorError::InvalidLastItemKey)
}

/// At most `limit` segments of `revision` after its first `watermark`, in
/// start order, that intersect `range` (all of them without one) and, with a
/// `query`, match it.
///
/// The next token's watermark is past every segment this read looked at: the
/// last one it returned when it stopped at its limit, every segment otherwise,
/// so a segment outside the range or without a match is not looked at again.
///
/// # Errors
///
/// [`CursorError`] when the next token cannot be formed.
pub fn page_since<'a>(
    session_id: &SessionId,
    revision: &'a TranscriptRevision,
    watermark: usize,
    range: Option<TimeRange>,
    query: Option<&SearchQuery>,
    limit: PageLimit,
    expires_at_micros: u64,
) -> Result<SincePage<'a>, CursorError> {
    let limit = usize::from(limit.get());
    let mut segments = Vec::new();
    let mut seen = watermark;
    let mut more = false;
    for (position, segment) in revision.segments().iter().enumerate().skip(watermark) {
        if range.is_some_and(|range| !segment.intersects(range)) {
            seen = position + 1;
            continue;
        }
        let tier = query.map(|query| query.classify(segment.text().text()));
        if tier == Some(None) {
            // A search, and this segment does not match.
            seen = position + 1;
            continue;
        }
        let tier = tier.flatten();
        if segments.len() == limit {
            more = true;
            break;
        }
        segments.push((segment, tier));
        seen = position + 1;
    }
    Ok(SincePage {
        segments,
        next: since_token(session_id, revision, seen, expires_at_micros)?,
        more,
    })
}
