//! Bounded literal search of a session's transcript.

use vsift_application::{SearchPageRequest, page_search};
use vsift_domain::{
    MediaTime, PageLimit, SearchCoverage, SearchMatch, SearchQuery, SessionId, TimeRange,
    TranscriptRevision, TranscriptRevisionId, TranscriptSegment,
};

use crate::{
    engine::Engine,
    error::{EngineError, SessionRootError},
    sessions::SessionSnapshot,
};

const MICROS_PER_SECOND: u64 = 1_000_000;

/// A half-open source range to restrict a search to, in microseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchRange {
    /// Inclusive start.
    pub from_micros: u64,
    /// Exclusive end.
    pub to_micros: u64,
}

/// A bounded literal search of one session's transcript.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchRequest {
    /// Session whose transcript is searched.
    pub session: SessionId,
    /// Literal query text, at most 256 bytes; it is normalised, never
    /// interpreted as a pattern.
    pub query: String,
    /// Revision to search; `None` searches the newest.
    pub revision: Option<TranscriptRevisionId>,
    /// Only segments intersecting this range are searched; `None` searches
    /// the whole revision.
    pub range: Option<SearchRange>,
    /// Page size from 1 to 100; `None` selects the default of 20.
    pub limit: Option<u16>,
    /// Opaque continuation token from the previous page of the same search.
    pub cursor: Option<String>,
}

/// One segment that matched, and how.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResultHit {
    segment: TranscriptSegment,
    tier: SearchMatch,
}

impl SearchResultHit {
    /// The matching segment, exactly as `transcript get` returns it.
    #[must_use]
    pub const fn segment(&self) -> &TranscriptSegment {
        &self.segment
    }

    /// Whether it matched as a phrase or by all terms.
    #[must_use]
    pub const fn tier(&self) -> SearchMatch {
        self.tier
    }
}

/// One bounded page of search results.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResults {
    session: SessionSnapshot,
    revision: TranscriptRevision,
    query: SearchQuery,
    range: Option<TimeRange>,
    hits: Vec<SearchResultHit>,
    next_cursor: Option<String>,
    coverage: SearchCoverage,
}

impl SearchResults {
    /// Session state observed when the search ran.
    #[must_use]
    pub const fn session(&self) -> &SessionSnapshot {
        &self.session
    }

    /// The revision that was searched.
    #[must_use]
    pub const fn revision(&self) -> &TranscriptRevision {
        &self.revision
    }

    /// The query as it was normalised.
    #[must_use]
    pub const fn query(&self) -> &SearchQuery {
        &self.query
    }

    /// The requested range, if the search was restricted to one.
    #[must_use]
    pub const fn range(&self) -> Option<TimeRange> {
        self.range
    }

    /// Hits in rank order: phrase before all-terms, then by start.
    #[must_use]
    pub fn hits(&self) -> &[SearchResultHit] {
        &self.hits
    }

    /// Opaque token for the next page, absent on the last page.
    #[must_use]
    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }

    /// What the searched range's transcript covers, and what it does not.
    #[must_use]
    pub const fn coverage(&self) -> &SearchCoverage {
        &self.coverage
    }
}

impl Engine {
    /// Searches a session's newest transcript revision, or the revision
    /// `request.revision` names, for a literal query.
    ///
    /// The query is validated before anything is read, then matched against
    /// every segment of the revision (or those intersecting `request.range`)
    /// and ranked (ADR 0018). Search is computed from the immutable revision on
    /// each call; it never runs a provider and writes nothing. The result
    /// states which parts of the searched range no transcript covers, because
    /// words said there cannot be found.
    ///
    /// # Errors
    ///
    /// Fails for a rejected query, an invalid range, page size or cursor, a
    /// missing root or session, a closed or expired session, a session without
    /// a transcript, a revision the session does not hold, or a stored record
    /// that fails its integrity checks.
    pub fn search(&self, request: SearchRequest) -> Result<SearchResults, EngineError> {
        let query = SearchQuery::parse(&request.query).map_err(EngineError::SearchQueryRejected)?;
        let range = request
            .range
            .map(|range| {
                TimeRange::new(
                    MediaTime::from_micros(range.from_micros),
                    MediaTime::from_micros(range.to_micros),
                )
            })
            .transpose()
            .map_err(|_| EngineError::InvalidTimeRange)?;
        let limit = request
            .limit
            .map_or(Ok(PageLimit::DEFAULT), PageLimit::new)
            .map_err(|_| EngineError::InvalidPageLimit)?;
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        let (revision, status) = match &request.revision {
            None => store
                .read_transcript(&request.session, now)?
                .ok_or(EngineError::TranscriptUnavailable)?,
            Some(revision) => store
                .read_transcript_revision(&request.session, revision, now)?
                .ok_or(EngineError::TranscriptRevisionNotFound)?,
        };
        let expires_at = status
            .lifetime()
            .expires_at_unix_seconds()
            .saturating_mul(MICROS_PER_SECOND);
        let page = page_search(
            &request.session,
            &revision,
            &SearchPageRequest {
                query: query.clone(),
                range,
                limit,
                cursor: request.cursor,
            },
            expires_at,
            now.saturating_mul(MICROS_PER_SECOND),
        )
        .map_err(EngineError::TranscriptQuery)?;
        let hits = page
            .hits
            .iter()
            .map(|hit| SearchResultHit {
                segment: hit.segment().clone(),
                tier: hit.tier(),
            })
            .collect();
        let next_cursor = page.next_cursor.map(|cursor| cursor.encode());
        let coverage = page.coverage;
        Ok(SearchResults {
            session: SessionSnapshot::observe(&status, now),
            revision,
            query,
            range,
            hits,
            next_cursor,
            coverage,
        })
    }
}
