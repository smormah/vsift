//! One literal search over retained bundles the caller named (ADR 0035).
//!
//! A kept recording exists only as a retained bundle, so this use case reads
//! bundles and never a session root: a disposable session cannot be reached. It
//! reads each bundle's newest transcript, screen-text index and visual index
//! through the [`OpenedBundle`] port, one bundle at a time, and keeps only the
//! page, the totals and each bundle's coverage.
//!
//! **Bounds.** Which bundles are opened is decided before any is: the first
//! [`MAX_BUNDLES_PER_SEARCH`] in the order they were found. Every manifest and
//! record read counts against one byte budget, charged from what the manifest
//! declares before a record is read. A byte budget and not a clock, because the
//! same request must give the same page however busy the machine is.
//!
//! **Failure.** A bundle that cannot be read does not fail the search: it is
//! listed, and the search goes on. Only a cancellation or a rejected cursor
//! ends it.
//!
//! **Cursor.** Every page reads every bundle, so `total_hits` is exact and a
//! cursor is only a rank position. It is bound to the ordered manifest digests
//! of the bundles searched and the normalised query, so any change to the set
//! rejects it instead of silently restarting.

use std::{collections::BTreeMap, fmt};

use vsift_domain::{
    BundleHitKey, BundleHitKind, BundleHitPosition, CoverageBasis, CursorError, CursorToken,
    FrameRef, MediaTime, PageLimit, QueryDigest, ScreenTextCoverage, ScreenTextIndex,
    ScreenTextIndexId, ScreenTextLine, ScreenTextOccurrence, SearchCoverage, SearchHit,
    SearchMatch, SearchQuery, SessionId, SourceId, TranscriptRevision, TranscriptRevisionId,
    TranscriptSegment, VisualCandidateId, VisualIndex, rank_bundle_hits, screen_text_occurrences,
    search_hits,
};

use crate::{
    SessionStorageError, screen_text_search::screen_text_plan_coverage, transcript::sha256_hex,
};

/// Most paths one search names.
pub const MAX_BUNDLE_SEARCH_PATHS: usize = 64;
/// Most folder entries one search looks at, across all of its paths.
pub const MAX_BUNDLE_SEARCH_ENTRIES: usize = 4_096;
/// Most bundles one search opens. Every page reads them all, and each gets a
/// coverage entry.
pub const MAX_BUNDLES_PER_SEARCH: usize = 64;
/// Most bytes of manifests and records one search reads.
pub const BUNDLE_SEARCH_READ_BUDGET: u64 = 1024 * 1024 * 1024;
/// Most bytes a bundle's manifest holds, charged for every bundle opened.
pub const MAX_BUNDLE_MANIFEST_BYTES: u64 = 128 * 1024;
/// How long a cursor stays usable. The bundles do not expire; a token needs an
/// end, and a day outlasts any one look through the pages.
const CURSOR_LIFETIME_MICROS: u64 = 24 * 60 * 60 * 1_000_000;
/// Separates the parts of a cursor's last-item key.
const KEY_SEPARATOR: char = '-';

/// Why a search stopped, or was refused before it read a bundle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BundleSearchError {
    /// A path named no bundle and no folder holding one.
    NoBundles,
    /// More than [`MAX_BUNDLE_SEARCH_PATHS`] paths were named.
    TooManyPaths,
    /// The paths hold more than [`MAX_BUNDLE_SEARCH_ENTRIES`] entries. A call
    /// over it is refused and not cut, because which entries a listing gives
    /// first is the filesystem's choice and a page must not depend on it.
    TooManyEntries,
    /// The cursor does not continue this search.
    Cursor(CursorError),
    /// The search was cancelled between two reads.
    Cancelled,
}

impl fmt::Display for BundleSearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBundles => formatter.write_str("a path holds no retained bundle"),
            Self::TooManyPaths => formatter.write_str("too many paths were named"),
            Self::TooManyEntries => formatter.write_str("the paths hold too many entries"),
            Self::Cursor(error) => error.fmt(formatter),
            Self::Cancelled => formatter.write_str("the search was cancelled"),
        }
    }
}

impl std::error::Error for BundleSearchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Cursor(error) => Some(error),
            Self::NoBundles | Self::TooManyPaths | Self::TooManyEntries | Self::Cancelled => None,
        }
    }
}

/// Why a bundle, or one record of it, could not be read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BundleUnreadable {
    /// The folder is not private to the current user.
    NotPrivate,
    /// A manifest or record failed verification.
    IntegrityFailure,
    /// It was written by a newer release.
    UnsupportedVersion,
    /// It could not be read for another reason.
    Unreadable,
}

impl BundleUnreadable {
    /// The reason for a storage failure.
    #[must_use]
    pub const fn of(error: &SessionStorageError) -> Self {
        match error {
            SessionStorageError::AccessDenied => Self::NotPrivate,
            SessionStorageError::IntegrityFailure => Self::IntegrityFailure,
            SessionStorageError::UnsupportedVersion => Self::UnsupportedVersion,
            _ => Self::Unreadable,
        }
    }
}

/// A bundle found by looking at folders, before anything of it was read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FoundBundle<L> {
    /// The bundle folder's own name, which is all a result says of its place.
    pub name: String,
    /// Where it is, for the adapter that opens it.
    pub location: L,
}

/// The bundles the paths of one search hold, in the order they are opened.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FoundBundles<L> {
    /// By folder name, then whole path; so never by the order of the paths.
    pub bundles: Vec<FoundBundle<L>>,
    /// Subfolders that hold no `bundle.json`, as an interrupted retain leaves.
    pub skipped_entries: usize,
}

/// What a bundle's manifest says it is.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleIdentity {
    /// The session it was retained from.
    pub session_id: SessionId,
    /// The source recording.
    pub source_id: SourceId,
    /// Whether it holds a copy of the source.
    pub source_included: bool,
    /// SHA-256 of its manifest, which names this bundle exactly.
    pub manifest_sha256: String,
}

/// One opened bundle: its identity, and verified reads of its newest records.
pub trait OpenedBundle {
    /// What its manifest says it is.
    fn identity(&self) -> &BundleIdentity;

    /// What its newest transcript, screen-text and visual records declare they
    /// hold, in bytes; the sum of what the read methods below will read.
    fn record_bytes(&self) -> u64;

    /// Reads its newest transcript revision, if it holds one.
    ///
    /// # Errors
    ///
    /// The record failed verification or could not be read.
    fn read_transcript(&self) -> Result<Option<TranscriptRevision>, BundleUnreadable>;

    /// Reads its newest screen-text index, if `ocr` read the session.
    ///
    /// # Errors
    ///
    /// The record failed verification or could not be read.
    fn read_screen_text(&self) -> Result<Option<ScreenTextIndex>, BundleUnreadable>;

    /// Reads its newest visual index, if it holds one.
    ///
    /// # Errors
    ///
    /// The record failed verification or could not be read.
    fn read_visual(&self) -> Result<Option<VisualIndex>, BundleUnreadable>;
}

/// One search over the bundles found.
#[derive(Clone, Debug)]
pub struct BundleSearchInput<'a, L> {
    /// What the paths hold, in the order to open them.
    pub found: &'a FoundBundles<L>,
    /// The validated literal query.
    pub query: &'a SearchQuery,
    /// Most hits on this page.
    pub limit: PageLimit,
    /// Opaque continuation token from the previous page of the same search.
    pub cursor: Option<&'a str>,
    /// Most bytes to read: [`BUNDLE_SEARCH_READ_BUDGET`], but for a test.
    pub read_budget: u64,
    /// The current time.
    pub now_micros: u64,
}

/// An occurrence of on-screen text, as a hit keeps it once its index is gone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleScreenTextHit {
    /// The line as read on the first frame.
    pub line: ScreenTextLine,
    /// The first frame.
    pub frame: FrameRef,
    /// The candidates whose representative frame that is.
    pub candidates: Vec<VisualCandidateId>,
    /// The last consecutive read frame's time that held the same text there.
    pub last_time: MediaTime,
    /// How many consecutive read frames held it.
    pub frames: u32,
}

/// What a hit found. `T` is a transcript segment as the host presents it,
/// because presenting one needs its revision, which is not kept.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BundleHitEvidence<T> {
    /// A transcript segment.
    Transcript(T),
    /// An occurrence of on-screen text.
    ScreenText(BundleScreenTextHit),
}

/// One hit, naming its bundle, session, source, time and evidence id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleHit<T> {
    /// Its rank across the search.
    pub position: BundleHitPosition,
    /// The bundle folder's name.
    pub bundle: String,
    /// The bundle's identity.
    pub identity: BundleIdentity,
    /// The segment or line id.
    pub evidence_id: String,
    /// A segment's start, or the first frame of a line.
    pub time: MediaTime,
    /// What was found.
    pub evidence: BundleHitEvidence<T>,
    /// The token that continues the search after this hit, so a host that
    /// ends its page early still continues without losing a hit.
    pub continuation: String,
}

impl<T> BundleHit<T> {
    /// What kind of evidence the hit is.
    #[must_use]
    pub const fn kind(&self) -> BundleHitKind {
        match self.evidence {
            BundleHitEvidence::Transcript(_) => BundleHitKind::Transcript,
            BundleHitEvidence::ScreenText(_) => BundleHitKind::ScreenText,
        }
    }
}

/// What a bundle's transcript gave the search.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BundleTranscript {
    /// The bundle holds no transcript.
    None,
    /// Its newest record could not be read.
    Unavailable(BundleUnreadable),
    /// It was searched.
    Read {
        /// The revision searched.
        revision_id: TranscriptRevisionId,
        /// Where its words came from.
        basis: CoverageBasis,
        /// How many parts of the video it does not cover.
        untranscribed_ranges: usize,
        /// How much of the video that is.
        untranscribed_micros: u64,
    },
}

impl BundleTranscript {
    /// Whether words said in this recording could have been missed.
    #[must_use]
    pub const fn has_gap(&self) -> bool {
        !matches!(
            self,
            Self::Read {
                untranscribed_ranges: 0,
                ..
            }
        )
    }
}

/// What a bundle's on-screen text gave the search.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BundleScreenText {
    /// `ocr` did not read the session before it was retained.
    NotRun,
    /// Its newest record could not be read.
    Unavailable(BundleUnreadable),
    /// It was searched.
    Read {
        /// The index searched.
        index_id: ScreenTextIndexId,
        /// The period of its latest plan.
        period_micros: u64,
        /// How much of the plan it has read.
        coverage: ScreenTextCoverage,
        /// Whether every visual window is analysed.
        candidates_complete: bool,
        /// The visual index, when it could not be read: the hits stand, but
        /// the plan, and so the coverage, is not known.
        visual_unavailable: Option<BundleUnreadable>,
    },
}

impl BundleScreenText {
    /// Whether all of the bundle's planned frames were read, so a missing hit
    /// says something.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        match self {
            Self::NotRun | Self::Unavailable(_) => false,
            Self::Read {
                coverage,
                candidates_complete,
                ..
            } => {
                coverage.pending == 0
                    && coverage.failed == 0
                    && coverage.awaiting_analysis == 0
                    && *candidates_complete
            }
        }
    }
}

/// What the search saw of one bundle it read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleSessionCoverage {
    /// The bundle folder's name.
    pub bundle: String,
    /// The bundle's identity.
    pub identity: BundleIdentity,
    /// Its hits in the whole search.
    pub hits: usize,
    /// Its transcript.
    pub transcript: BundleTranscript,
    /// Its on-screen text.
    pub screen_text: BundleScreenText,
}

/// One entry of the coverage list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BundleSessionEntry {
    /// The bundle was opened and searched.
    Searched(Box<BundleSessionCoverage>),
    /// The bundle could not be opened.
    Unavailable {
        /// The bundle folder's name.
        bundle: String,
        /// Why.
        reason: BundleUnreadable,
    },
}

/// Why a bundle found was not searched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BundleNotSearchedReason {
    /// It came after the first [`MAX_BUNDLES_PER_SEARCH`]; it was never opened.
    BundleLimit,
    /// Its manifest or records did not fit in what was left of the budget.
    ReadBudget,
}

/// A bundle found and not searched.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleNotSearched {
    /// The bundle folder's name.
    pub bundle: String,
    /// Why.
    pub reason: BundleNotSearchedReason,
}

/// The counts of a search.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BundleSearchCounts {
    /// Bundles the paths hold.
    pub found: usize,
    /// Bundles searched.
    pub searched: usize,
    /// Bundles that could not be opened.
    pub unavailable: usize,
    /// Bundles not searched, listed or not.
    pub not_searched: usize,
    /// Bundles opened that were the same bundle as one before them.
    pub duplicates: usize,
    /// Subfolders that hold no `bundle.json`.
    pub skipped_entries: usize,
    /// Bundles searched whose on-screen text is not complete.
    pub screen_text_incomplete: usize,
}

/// One page of a search, and everything it could and could not see.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleSearchOutcome<T> {
    /// Hits in rank order, at most the page size.
    pub hits: Vec<BundleHit<T>>,
    /// How many hits the whole search has.
    pub total_hits: usize,
    /// Whether a hit ranks after the last one here.
    pub has_more: bool,
    /// The counts.
    pub counts: BundleSearchCounts,
    /// One entry per bundle opened, in order, duplicates left out. Never cut.
    pub sessions: Vec<BundleSessionEntry>,
    /// The bundles not searched, in order, at most [`MAX_BUNDLES_PER_SEARCH`].
    pub not_searched: Vec<BundleNotSearched>,
}

/// Runs one search: `open` opens a bundle, `present` keeps a matching segment
/// as the host presents it, and `cancelled` is asked between all reads.
///
/// # Errors
///
/// [`BundleSearchError::Cancelled`] for a cancellation, and
/// [`BundleSearchError::Cursor`] for a cursor that does not continue this
/// search. A bundle that cannot be read is reported in the outcome.
pub fn search_bundles<L, B, T>(
    input: &BundleSearchInput<'_, L>,
    mut open: impl FnMut(&L) -> Result<B, BundleUnreadable>,
    present: impl Fn(&TranscriptRevision, &TranscriptSegment) -> T,
    cancelled: impl Fn() -> bool,
) -> Result<BundleSearchOutcome<T>, BundleSearchError>
where
    B: OpenedBundle,
{
    let token = input
        .cursor
        .map(CursorToken::parse)
        .transpose()
        .map_err(BundleSearchError::Cursor)?;
    let after = token
        .as_ref()
        .map(|token| parse_position(token.last_item_key()))
        .transpose()
        .map_err(BundleSearchError::Cursor)?;
    let mut run = Run {
        query: input.query,
        present: &present,
        left: input.read_budget,
        page: Page::new(after, input.limit),
        digests: Vec::new(),
        first_session: None,
        counts: BundleSearchCounts {
            found: input.found.bundles.len(),
            skipped_entries: input.found.skipped_entries,
            ..BundleSearchCounts::default()
        },
        sessions: Vec::new(),
        not_searched: Vec::new(),
    };
    for (index, found) in input.found.bundles.iter().enumerate() {
        if index >= MAX_BUNDLES_PER_SEARCH {
            run.not_searched(found, BundleNotSearchedReason::BundleLimit);
            continue;
        }
        if cancelled() {
            return Err(BundleSearchError::Cancelled);
        }
        if run.left < MAX_BUNDLE_MANIFEST_BYTES {
            run.not_searched(found, BundleNotSearchedReason::ReadBudget);
            continue;
        }
        run.left -= MAX_BUNDLE_MANIFEST_BYTES;
        match open(&found.location) {
            Ok(bundle) => run.search(found, &bundle, &cancelled)?,
            Err(reason) => {
                run.counts.unavailable += 1;
                run.sessions.push(BundleSessionEntry::Unavailable {
                    bundle: found.name.clone(),
                    reason,
                });
            }
        }
    }
    run.finish(token.as_ref(), input.query, input.now_micros)
}

/// The hits of the page so far: the best `limit + 1` after the cursor, so that
/// a further hit is known about. At most one page is held, whatever the search
/// matches.
struct Page<T> {
    after: Option<BundleHitPosition>,
    wanted: usize,
    kept: BTreeMap<BundleHitPosition, PendingHit<T>>,
    total: usize,
}

/// A hit before its continuation is attached.
struct PendingHit<T> {
    bundle: String,
    identity: BundleIdentity,
    evidence_id: String,
    time: MediaTime,
    evidence: BundleHitEvidence<T>,
}

impl<T> Page<T> {
    fn new(after: Option<BundleHitPosition>, limit: PageLimit) -> Self {
        Self {
            after,
            wanted: usize::from(limit.get()).saturating_add(1),
            kept: BTreeMap::new(),
            total: 0,
        }
    }

    /// Counts a hit and builds it only if the page has room for it, so a
    /// common word in a long recording copies at most a page of text.
    fn offer(&mut self, position: BundleHitPosition, build: impl FnOnce() -> PendingHit<T>) {
        self.total = self.total.saturating_add(1);
        if self.after.is_some_and(|after| position <= after) {
            return;
        }
        let beyond = self.kept.len() >= self.wanted
            && self
                .kept
                .last_key_value()
                .is_some_and(|(last, _)| position > *last);
        if beyond {
            return;
        }
        self.kept.insert(position, build());
        if self.kept.len() > self.wanted {
            self.kept.pop_last();
        }
    }
}

/// The state of one search.
struct Run<'a, T, P> {
    query: &'a SearchQuery,
    present: &'a P,
    left: u64,
    page: Page<T>,
    /// The manifest digests of the bundles searched, in rank order.
    digests: Vec<String>,
    first_session: Option<SessionId>,
    counts: BundleSearchCounts,
    sessions: Vec<BundleSessionEntry>,
    not_searched: Vec<BundleNotSearched>,
}

/// One hit of a bundle, before the bundle's hits are ranked.
enum RawHit<'a> {
    Transcript(&'a TranscriptRevision, SearchHit<'a>),
    ScreenText(ScreenTextOccurrence<'a>),
}

impl<T, P> Run<'_, T, P>
where
    P: Fn(&TranscriptRevision, &TranscriptSegment) -> T,
{
    fn not_searched<L>(&mut self, found: &FoundBundle<L>, reason: BundleNotSearchedReason) {
        self.counts.not_searched += 1;
        if self.not_searched.len() < MAX_BUNDLES_PER_SEARCH {
            self.not_searched.push(BundleNotSearched {
                bundle: found.name.clone(),
                reason,
            });
        }
    }

    fn search<L>(
        &mut self,
        found: &FoundBundle<L>,
        bundle: &impl OpenedBundle,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), BundleSearchError> {
        let identity = bundle.identity();
        if self.digests.contains(&identity.manifest_sha256) {
            self.counts.duplicates += 1;
            return Ok(());
        }
        let declared = bundle.record_bytes();
        if declared > self.left {
            self.not_searched(found, BundleNotSearchedReason::ReadBudget);
            return Ok(());
        }
        self.left -= declared;
        let revision = bundle.read_transcript();
        if cancelled() {
            return Err(BundleSearchError::Cancelled);
        }
        let index = bundle.read_screen_text();
        if cancelled() {
            return Err(BundleSearchError::Cancelled);
        }
        // Only an index has a plan to measure, so a session `ocr` never read
        // costs no visual-index read.
        let visual = match &index {
            Ok(Some(_)) => bundle.read_visual(),
            _ => Ok(None),
        };
        let rank = self.digests.len();
        let hits = self.offer_hits(rank, &found.name, identity, &revision, &index);
        let transcript = transcript_coverage(&revision);
        let screen_text = screen_text_coverage(&index, &visual);
        self.counts.searched += 1;
        if !screen_text.is_complete() {
            self.counts.screen_text_incomplete += 1;
        }
        self.digests.push(identity.manifest_sha256.clone());
        if self.first_session.is_none() {
            self.first_session = Some(identity.session_id.clone());
        }
        self.sessions.push(BundleSessionEntry::Searched(Box::new(
            BundleSessionCoverage {
                bundle: found.name.clone(),
                identity: identity.clone(),
                hits,
                transcript,
                screen_text,
            },
        )));
        Ok(())
    }

    /// Ranks one bundle's hits and offers them to the page; returns how many
    /// it has.
    fn offer_hits(
        &mut self,
        rank: usize,
        name: &str,
        identity: &BundleIdentity,
        revision: &Result<Option<TranscriptRevision>, BundleUnreadable>,
        index: &Result<Option<ScreenTextIndex>, BundleUnreadable>,
    ) -> usize {
        let mut raw: Vec<(BundleHitKey, RawHit<'_>)> = Vec::new();
        if let Ok(Some(revision)) = revision {
            raw.extend(search_hits(revision, self.query).into_iter().map(|hit| {
                (
                    BundleHitKey {
                        tier: hit.tier(),
                        time: hit.segment().range().start(),
                        kind: BundleHitKind::Transcript,
                        ordinal: hit.segment().ordinal(),
                    },
                    RawHit::Transcript(revision, hit),
                )
            }));
        }
        if let Ok(Some(index)) = index {
            raw.extend(
                screen_text_occurrences(index, self.query, None)
                    .into_iter()
                    .map(|hit| {
                        (
                            BundleHitKey {
                                tier: hit.tier,
                                time: hit.first.frame.time,
                                kind: BundleHitKind::ScreenText,
                                ordinal: hit.ordinal,
                            },
                            RawHit::ScreenText(hit),
                        )
                    }),
            );
        }
        let count = raw.len();
        // The rank is below MAX_BUNDLES_PER_SEARCH.
        let bundle = u16::try_from(rank).unwrap_or(u16::MAX);
        for (position, hit) in rank_bundle_hits(bundle, raw) {
            let present = self.present;
            self.page.offer(position, || match hit {
                RawHit::Transcript(revision, hit) => PendingHit {
                    bundle: name.to_owned(),
                    identity: identity.clone(),
                    evidence_id: hit.segment().id().as_str().to_owned(),
                    time: hit.segment().range().start(),
                    evidence: BundleHitEvidence::Transcript(present(revision, hit.segment())),
                },
                RawHit::ScreenText(hit) => PendingHit {
                    bundle: name.to_owned(),
                    identity: identity.clone(),
                    evidence_id: hit.line.id.as_str().to_owned(),
                    time: hit.first.frame.time,
                    evidence: BundleHitEvidence::ScreenText(BundleScreenTextHit {
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
                    }),
                },
            });
        }
        count
    }

    fn finish(
        self,
        token: Option<&CursorToken>,
        query: &SearchQuery,
        now_micros: u64,
    ) -> Result<BundleSearchOutcome<T>, BundleSearchError> {
        let scope = self
            .scope(query, now_micros)
            .map_err(BundleSearchError::Cursor)?;
        if let Some(token) = token {
            let Some(scope) = &scope else {
                return Err(BundleSearchError::Cursor(CursorError::WrongSession));
            };
            token
                .validate_scope(&scope.session, scope.generation, &scope.digest, now_micros)
                .map_err(BundleSearchError::Cursor)?;
        }
        let Self {
            page,
            counts,
            sessions,
            not_searched,
            ..
        } = self;
        let Page {
            mut kept,
            wanted,
            total,
            ..
        } = page;
        let has_more = kept.len() >= wanted;
        if has_more {
            kept.pop_last();
        }
        let mut hits = Vec::with_capacity(kept.len());
        if let Some(scope) = &scope {
            for (position, hit) in kept {
                hits.push(BundleHit {
                    position,
                    bundle: hit.bundle,
                    identity: hit.identity,
                    evidence_id: hit.evidence_id,
                    time: hit.time,
                    evidence: hit.evidence,
                    continuation: scope
                        .token_after(position)
                        .map_err(BundleSearchError::Cursor)?
                        .encode(),
                });
            }
        }
        Ok(BundleSearchOutcome {
            hits,
            total_hits: total,
            has_more,
            counts,
            sessions,
            not_searched,
        })
    }

    /// What a cursor of this search is bound to; `None` when no bundle was
    /// searched, so there is nothing to continue.
    fn scope(
        &self,
        query: &SearchQuery,
        now_micros: u64,
    ) -> Result<Option<CursorScope>, CursorError> {
        let Some(session) = self.first_session.clone() else {
            return Ok(None);
        };
        Ok(Some(CursorScope {
            session,
            generation: u64::try_from(self.digests.len()).unwrap_or(u64::MAX),
            digest: bundle_search_digest(&self.digests, query)?,
            expires_micros: now_micros.saturating_add(CURSOR_LIFETIME_MICROS),
        }))
    }
}

struct CursorScope {
    session: SessionId,
    generation: u64,
    digest: QueryDigest,
    expires_micros: u64,
}

impl CursorScope {
    fn token_after(&self, position: BundleHitPosition) -> Result<CursorToken, CursorError> {
        CursorToken::new(
            self.session.clone(),
            self.generation,
            self.digest.clone(),
            position_key(position),
            self.expires_micros,
        )
    }
}

fn transcript_coverage(
    revision: &Result<Option<TranscriptRevision>, BundleUnreadable>,
) -> BundleTranscript {
    match revision {
        Err(reason) => BundleTranscript::Unavailable(*reason),
        Ok(None) => BundleTranscript::None,
        Ok(Some(revision)) => {
            let coverage = SearchCoverage::of(revision, None);
            BundleTranscript::Read {
                revision_id: revision.id().clone(),
                basis: coverage.basis(),
                untranscribed_ranges: coverage.untranscribed().len(),
                untranscribed_micros: coverage
                    .untranscribed()
                    .iter()
                    .map(|gap| {
                        gap.end()
                            .as_micros()
                            .saturating_sub(gap.start().as_micros())
                    })
                    .sum(),
            }
        }
    }
}

fn screen_text_coverage(
    index: &Result<Option<ScreenTextIndex>, BundleUnreadable>,
    visual: &Result<Option<VisualIndex>, BundleUnreadable>,
) -> BundleScreenText {
    match index {
        Err(reason) => BundleScreenText::Unavailable(*reason),
        Ok(None) => BundleScreenText::NotRun,
        Ok(Some(index)) => {
            let (readable, visual_unavailable) = match visual {
                Ok(visual) => (visual.as_ref(), None),
                Err(reason) => (None, Some(*reason)),
            };
            // Without a usable visual index nothing is planned and the
            // candidates are not known: it is never complete.
            let (coverage, candidates_complete) = screen_text_plan_coverage(readable, index, None)
                .unwrap_or_else(|| (ScreenTextCoverage::of(&[], Some(index)), false));
            BundleScreenText::Read {
                index_id: index.id().clone(),
                period_micros: index.period_micros(),
                coverage,
                candidates_complete,
                visual_unavailable,
            }
        }
    }
}

/// Digest of the canonical search: how many bundles, their manifest digests in
/// rank order, and the normalised query words, one a line (they hold no line
/// break).
fn bundle_search_digest(
    manifests: &[String],
    query: &SearchQuery,
) -> Result<QueryDigest, CursorError> {
    let mut material = format!("vsift.bundle-search.v1\n{}", manifests.len());
    for manifest in manifests {
        material.push('\n');
        material.push_str(manifest);
    }
    for term in query.terms() {
        material.push('\n');
        material.push_str(term);
    }
    QueryDigest::parse(sha256_hex(material.as_bytes()))
}

/// The cursor key of a position: `<tier>-<bundle>-<sequence>`, for example `1-3-42`.
fn position_key(position: BundleHitPosition) -> String {
    format!(
        "{}{KEY_SEPARATOR}{}{KEY_SEPARATOR}{}",
        position.tier().tier(),
        position.bundle(),
        position.sequence()
    )
}

/// Parses a canonical `<tier>-<bundle>-<sequence>` key.
fn parse_position(key: &str) -> Result<BundleHitPosition, CursorError> {
    let invalid = CursorError::InvalidLastItemKey;
    let mut parts = key.split(KEY_SEPARATOR);
    let (Some(tier), Some(bundle), Some(sequence), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(invalid);
    };
    let tier = match tier {
        "1" => SearchMatch::Phrase,
        "2" => SearchMatch::AllTerms,
        _ => return Err(invalid),
    };
    let number = |text: &str| -> Option<u32> {
        let canonical = text.bytes().all(|byte| byte.is_ascii_digit())
            && (text == "0" || !text.starts_with('0'));
        canonical.then(|| text.parse().ok()).flatten()
    };
    let bundle = number(bundle)
        .and_then(|bundle| u16::try_from(bundle).ok())
        .filter(|bundle| usize::from(*bundle) < MAX_BUNDLES_PER_SEARCH)
        .ok_or(invalid)?;
    let sequence = number(sequence)
        .filter(|sequence| *sequence >= 1)
        .ok_or(invalid)?;
    Ok(BundleHitPosition::new(tier, bundle, sequence))
}

#[cfg(test)]
mod tests;
