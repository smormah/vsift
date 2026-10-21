//! Bounded literal search of a session's transcript.

use vsift_application::{
    ScreenTextPageRequest, SearchPageRequest, clip_to_source, missing_windows, page_screen_text,
    page_search,
};
use vsift_domain::{
    CursorError, FrameRef, MediaTime, PageLimit, ReadTarget, ScreenTextCoverage, ScreenTextIndex,
    ScreenTextLine, ScreenTextOccurrence, SearchCoverage, SearchMatch, SearchQuery, SessionId,
    TimeRange, TranscriptRevision, TranscriptRevisionId, TranscriptSegment, VisualCandidateId,
    VisualIndex, plan_reads, screen_text_occurrences,
};
use vsift_infrastructure::FilesystemSessionStore;

use crate::{
    engine::Engine,
    error::{EngineError, SessionRootError},
    screen_text::OcrError,
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

/// One occurrence of the query in on-screen text (ADR 0030): a line read on one
/// or more consecutive read frames, cited by its first frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTextResultHit {
    line: ScreenTextLine,
    frame: FrameRef,
    candidates: Vec<VisualCandidateId>,
    last_time: MediaTime,
    frames: u32,
    tier: SearchMatch,
}

impl ScreenTextResultHit {
    /// The line as read on the first frame.
    #[must_use]
    pub const fn line(&self) -> &ScreenTextLine {
        &self.line
    }

    /// The first frame the text was read on.
    #[must_use]
    pub const fn frame(&self) -> &FrameRef {
        &self.frame
    }

    /// The visual candidates whose representative frame that is.
    #[must_use]
    pub fn candidates(&self) -> &[VisualCandidateId] {
        &self.candidates
    }

    /// The last consecutive read frame's time that held the same text there.
    #[must_use]
    pub const fn last_time(&self) -> MediaTime {
        self.last_time
    }

    /// How many consecutive read frames held it.
    #[must_use]
    pub const fn frames(&self) -> u32 {
        self.frames
    }

    /// Whether it matched as a phrase or by all terms.
    #[must_use]
    pub const fn tier(&self) -> SearchMatch {
        self.tier
    }
}

/// The on-screen text part of a search: hits and how much of the range was read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTextSearch {
    index: Option<ScreenTextIndex>,
    hits: Vec<ScreenTextResultHit>,
    total_hits: usize,
    coverage: ScreenTextCoverage,
    candidates_complete: bool,
    period_micros: Option<u64>,
}

impl ScreenTextSearch {
    /// The index searched; `None` when `ocr` has not read this session.
    #[must_use]
    pub const fn index(&self) -> Option<&ScreenTextIndex> {
        self.index.as_ref()
    }

    /// Hits in rank order, at most the page size.
    #[must_use]
    pub fn hits(&self) -> &[ScreenTextResultHit] {
        &self.hits
    }

    /// How many hits there are in all; more than [`Self::hits`] when the page cut them.
    #[must_use]
    pub const fn total_hits(&self) -> usize {
        self.total_hits
    }

    /// How much of the searched range's plan (its candidates and one frame a
    /// period) the index has read.
    #[must_use]
    pub const fn coverage(&self) -> ScreenTextCoverage {
        self.coverage
    }

    /// Whether every visual window of the range is analysed, so the plan holds
    /// all of its candidates.
    #[must_use]
    pub const fn candidates_complete(&self) -> bool {
        self.candidates_complete
    }

    /// The period of the index's latest plan.
    #[must_use]
    pub const fn period_micros(&self) -> Option<u64> {
        self.period_micros
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
    screen_text: Option<ScreenTextSearch>,
}

impl SearchResults {
    /// The on-screen text part (ADR 0030) of a first page; `None` on a page a
    /// transcript cursor continues, whose first page already held it.
    #[must_use]
    pub const fn screen_text(&self) -> Option<&ScreenTextSearch> {
        self.screen_text.as_ref()
    }

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
        let first_page = request.cursor.is_none();
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
        let screen_text = if first_page {
            Some(search_screen_text_part(
                &store,
                &request.session,
                &query,
                range,
                limit,
                now,
            )?)
        } else {
            None
        };
        Ok(SearchResults {
            session: SessionSnapshot::observe(&status, now),
            revision,
            query,
            range,
            hits,
            next_cursor,
            coverage,
            screen_text,
        })
    }
}

/// The on-screen text part of a first page: hits of the session's newest
/// screen-text index, and the coverage of the range's plan, from the indexes as
/// they are (no tool runs and nothing is written).
fn search_screen_text_part(
    store: &FilesystemSessionStore,
    session: &SessionId,
    query: &SearchQuery,
    range: Option<TimeRange>,
    limit: PageLimit,
    now: u64,
) -> Result<ScreenTextSearch, EngineError> {
    let (index, _) = store.read_screen_text_index(session, now)?;
    let visual = store
        .read_visual_index(session, now)?
        .map(|(index, _)| index);
    let Some(index) = index else {
        return Ok(ScreenTextSearch {
            index: None,
            hits: Vec::new(),
            total_hits: 0,
            coverage: ScreenTextCoverage::default(),
            candidates_complete: false,
            period_micros: None,
        });
    };
    let occurrences = screen_text_occurrences(&index, query, range);
    let total_hits = occurrences.len();
    let hits = occurrences
        .iter()
        .take(usize::from(limit.get()))
        .map(result_hit)
        .collect();
    let (coverage, candidates_complete) = plan_coverage(visual.as_ref(), &index, range)?;
    Ok(ScreenTextSearch {
        period_micros: Some(index.period_micros()),
        index: Some(index),
        hits,
        total_hits,
        coverage,
        candidates_complete,
    })
}

/// One occurrence as the engine's result hit.
fn result_hit(hit: &ScreenTextOccurrence<'_>) -> ScreenTextResultHit {
    ScreenTextResultHit {
        line: hit.line.clone(),
        frame: hit.first.frame,
        candidates: hit
            .first
            .targets
            .iter()
            .filter_map(|target| target.candidate.clone())
            .collect(),
        last_time: hit.last_time,
        frames: hit.frames,
        tier: hit.tier,
    }
}

/// How much of the plan of `range` (the whole video when `None`) `index` has
/// read, and whether every visual window of it is analysed. Without a visual
/// index nothing is planned and the candidates are not known.
fn plan_coverage(
    visual: Option<&VisualIndex>,
    index: &ScreenTextIndex,
    range: Option<TimeRange>,
) -> Result<(ScreenTextCoverage, bool), EngineError> {
    let (plan, candidates_complete) = match visual {
        Some(visual) => {
            let whole = TimeRange::new(MediaTime::from_micros(0), visual.duration())
                .map_err(|_| EngineError::InvalidTimeRange)?;
            let searched =
                range.map_or(Ok(whole), |range| clip_to_source(range, visual.duration()));
            match searched {
                Ok(searched) => {
                    let candidates: Vec<(VisualCandidateId, MediaTime)> = visual
                        .candidates_in(searched)
                        .map(|candidate| (candidate.id().clone(), candidate.representative()))
                        .collect();
                    let plan = plan_reads(searched, &candidates, index.period_micros())
                        .unwrap_or_default();
                    let complete =
                        missing_windows(Some(visual), visual.duration(), searched).is_empty();
                    (plan, complete)
                }
                Err(_) => (Vec::<ReadTarget>::new(), true),
            }
        }
        None => (Vec::new(), false),
    };
    Ok((
        ScreenTextCoverage::of(&plan, Some(index)),
        candidates_complete,
    ))
}

/// A bounded literal search of one session's on-screen text (`ocr search`,
/// ADR 0030 decision 13): with or without a transcript.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTextSearchRequest {
    /// Session whose screen-text index is searched.
    pub session: SessionId,
    /// Literal query text, at most 256 bytes, normalised as `search` does.
    pub query: String,
    /// Only frames whose time lies in this range are searched; `None`, the whole index.
    pub range: Option<SearchRange>,
    /// Page size from 1 to 100; `None` selects the default of 20.
    pub limit: Option<u16>,
    /// Opaque continuation token from the previous page of the same search.
    pub cursor: Option<String>,
}

/// One bounded page of on-screen hits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTextSearchResults {
    session: SessionSnapshot,
    query: SearchQuery,
    range: Option<TimeRange>,
    part: ScreenTextSearch,
    next_cursor: Option<String>,
}

impl ScreenTextSearchResults {
    /// Session state observed when the search ran.
    #[must_use]
    pub const fn session(&self) -> &SessionSnapshot {
        &self.session
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

    /// The page's hits, the total, the index and the coverage of the searched range.
    #[must_use]
    pub const fn screen_text(&self) -> &ScreenTextSearch {
        &self.part
    }

    /// Opaque token for the next page, absent on the last page.
    #[must_use]
    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }
}

impl Engine {
    /// Searches a session's screen-text index for a literal query and pages
    /// every occurrence (ADR 0030 decision 13).
    ///
    /// It needs no transcript. The query is validated before anything is
    /// read; the result is computed from the immutable index on each call, runs
    /// no tool and writes nothing, and states how much of the searched range's
    /// plan has been read, because text on an unread frame cannot be found. A
    /// session `ocr` has not read answers no hits and says so.
    ///
    /// # Errors
    ///
    /// Fails for a rejected query, an invalid range, page size or cursor (also
    /// a cursor once `ocr` has committed a new revision), a missing root or
    /// session, a closed or expired session, or a stored record that fails its
    /// integrity checks.
    pub fn search_screen_text(
        &self,
        request: ScreenTextSearchRequest,
    ) -> Result<ScreenTextSearchResults, EngineError> {
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
        let (index, status) = store.read_screen_text_index(&request.session, now)?;
        let session = SessionSnapshot::observe(&status, now);
        let Some(index) = index else {
            if request.cursor.is_some() {
                return Err(EngineError::Ocr(OcrError::Cursor(
                    CursorError::WrongGeneration,
                )));
            }
            return Ok(ScreenTextSearchResults {
                session,
                query,
                range,
                part: ScreenTextSearch {
                    index: None,
                    hits: Vec::new(),
                    total_hits: 0,
                    coverage: ScreenTextCoverage::default(),
                    candidates_complete: false,
                    period_micros: None,
                },
                next_cursor: None,
            });
        };
        let expires_at = status
            .lifetime()
            .expires_at_unix_seconds()
            .saturating_mul(MICROS_PER_SECOND);
        let page = page_screen_text(
            &request.session,
            &index,
            &ScreenTextPageRequest {
                query: query.clone(),
                range,
                limit,
                cursor: request.cursor,
            },
            expires_at,
            now.saturating_mul(MICROS_PER_SECOND),
        )
        .map_err(|error| EngineError::Ocr(OcrError::Cursor(error)))?;
        let hits = page.hits.iter().map(result_hit).collect();
        let total_hits = page.total_hits;
        let next_cursor = page.next_cursor.map(|cursor| cursor.encode());
        let visual = store
            .read_visual_index(&request.session, now)?
            .map(|(visual, _)| visual);
        let (coverage, candidates_complete) = plan_coverage(visual.as_ref(), &index, range)?;
        Ok(ScreenTextSearchResults {
            session,
            query,
            range,
            part: ScreenTextSearch {
                period_micros: Some(index.period_micros()),
                index: Some(index),
                hits,
                total_hits,
                coverage,
                candidates_complete,
            },
            next_cursor,
        })
    }
}
