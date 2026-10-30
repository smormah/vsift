//! One literal search over retained bundles (ADR 0035).

use std::path::PathBuf;

use vsift_application::{
    BUNDLE_SEARCH_READ_BUDGET, BundleSearchError, BundleSearchInput, BundleSearchOutcome,
    search_bundles,
};
use vsift_contract::TranscriptSegmentData;
use vsift_domain::{PageLimit, SearchQuery};
use vsift_infrastructure::FilesystemSessionStore;

use crate::{
    engine::{Engine, absolute_selection},
    error::EngineError,
    verification::Cancellation,
};

const MICROS_PER_SECOND: u64 = 1_000_000;

/// A literal search of the retained bundles a caller named.
#[derive(Clone, Debug)]
pub struct BundleSearchRequest {
    /// Bundles, or folders of bundles; a relative path is read from the
    /// working directory.
    pub paths: Vec<PathBuf>,
    /// Literal query text, at most 256 bytes, normalised as `search` does.
    pub query: String,
    /// Page size from 1 to 100; `None` selects the default of 20.
    pub limit: Option<u16>,
    /// Opaque continuation token from the previous page of the same search.
    pub cursor: Option<String>,
    /// Asked between bundles and between the records of one bundle.
    pub cancellation: Cancellation,
}

/// One page of a search over bundles, and what it could and could not see.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleSearchResults {
    query: SearchQuery,
    outcome: BundleSearchOutcome<TranscriptSegmentData>,
}

impl BundleSearchResults {
    /// The query as it was normalised.
    #[must_use]
    pub const fn query(&self) -> &SearchQuery {
        &self.query
    }

    /// The page, the counts and the coverage of every bundle.
    #[must_use]
    pub const fn outcome(&self) -> &BundleSearchOutcome<TranscriptSegmentData> {
        &self.outcome
    }
}

impl Engine {
    /// Searches the newest transcript and on-screen text of the retained
    /// bundles `request.paths` hold for a literal query (ADR 0035).
    ///
    /// It never opens the session root, so a disposable session cannot be
    /// reached, and it writes nothing, runs no tool and keeps no index. The
    /// bundles to open are chosen from their folder names alone, then at most
    /// 1 GiB of manifests and records is read. A bundle that cannot be read is
    /// reported in the result, and the search goes on.
    ///
    /// A hit is a lead, not a citation: the session that made its ids is gone.
    /// To cite a moment, open the source with `ingest` and cite from there.
    ///
    /// # Errors
    ///
    /// Fails for a rejected query or page size, a path that holds no bundle,
    /// too many paths or entries, a cursor that does not continue this search,
    /// and a cancellation.
    pub fn search_bundles(
        &self,
        request: BundleSearchRequest,
    ) -> Result<BundleSearchResults, EngineError> {
        let query = SearchQuery::parse(&request.query).map_err(EngineError::SearchQueryRejected)?;
        let limit = request
            .limit
            .map_or(Ok(PageLimit::DEFAULT), PageLimit::new)
            .map_err(|_| EngineError::InvalidPageLimit)?;
        let paths = request
            .paths
            .iter()
            .map(|path| absolute_selection(path))
            .collect::<Result<Vec<_>, _>>()?;
        let found = FilesystemSessionStore::find_bundles(&paths)?;
        let now = self.now_unix_seconds()?;
        let cancellation = request.cancellation;
        let outcome = search_bundles(
            &BundleSearchInput {
                found: &found,
                query: &query,
                limit,
                cursor: request.cursor.as_deref(),
                read_budget: BUNDLE_SEARCH_READ_BUDGET,
                now_micros: now.saturating_mul(MICROS_PER_SECOND),
            },
            |path| FilesystemSessionStore::open_bundle_for_search(path),
            TranscriptSegmentData::new,
            || cancellation.is_cancelled(),
        )
        .map_err(EngineError::from)?;
        Ok(BundleSearchResults { query, outcome })
    }
}

impl From<BundleSearchError> for EngineError {
    fn from(error: BundleSearchError) -> Self {
        Self::BundleSearch(error)
    }
}
