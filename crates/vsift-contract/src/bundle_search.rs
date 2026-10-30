//! The data of `bundle search` (ADR 0035): one question over retained bundles.
//!
//! A hit names its bundle, session, source, time and evidence id, and carries
//! the transcript segment or on-screen line in the shapes `transcript get` and
//! `ocr search` publish. No absolute path is ever presented: a bundle is named
//! by its folder's own name.
//!
//! **The status is the transcript's**, as for `search`: `partial` for a
//! transcript gap or a bundle not searched. On-screen text never changes it,
//! because a bundle cannot run `ocr`, so a bundle retained without it would be
//! `partial` for ever. Its coverage is in each `sessions` entry and counted in
//! `bundles.screen_text_incomplete`.
//!
//! **A page fits the result limit.** Text can make 100 full hits larger than a
//! host's 1 MiB result, so a page ends early at [`MAX_HIT_BYTES`] and its
//! `next_cursor` goes on from the last hit shown: what gives way is the page
//! size, never a hit.

use serde::Serialize;
use vsift_application::{
    BundleHit, BundleHitEvidence, BundleNotSearched, BundleNotSearchedReason, BundleScreenText,
    BundleScreenTextHit, BundleSearchCounts, BundleSearchError, BundleSearchOutcome,
    BundleSessionCoverage, BundleSessionEntry, BundleTranscript, BundleUnreadable,
};
use vsift_domain::SearchQuery;

use crate::{
    CommandName, CoverageResponse, LifecycleResponse, OperationResponse, ResultView,
    ScreenTextHitPresentation, TranscriptSegmentData, sanitize_untrusted_text,
    screen_text::{CoverageData, ScreenTextHitData},
    search::SearchQueryData,
    transcript::BriefTranscriptSegmentData,
    view::BRIEF_VIEW,
};

/// Most bytes the hits of one page serialise to. With the rest of the data, at
/// most 64 entries of each list, a page stays within a host's 1 MiB result.
const MAX_HIT_BYTES: usize = 768 * 1024;

/// Most bytes of a bundle's name as presented: a folder name is the user's own
/// text and untrusted input, and a result repeats it on every hit.
const MAX_BUNDLE_NAME_BYTES: usize = 128;

/// Envelope reason: a bundle holds no transcript.
pub const NO_TRANSCRIPT_REASON: &str = "no_transcript";

/// Envelope reason: part of a recording has no transcript.
const UNTRANSCRIBED_REASON: &str = crate::search::UNTRANSCRIBED_REASON;

/// Envelope reason: a bundle, or its transcript, could not be read.
pub const BUNDLE_UNAVAILABLE_REASON: &str = "bundle_unavailable";

/// Envelope reason: bundles past the limit were not opened.
pub const BUNDLE_LIMIT_REASON: &str = "bundle_limit";

/// Envelope reason: bundles did not fit in the read budget.
pub const READ_BUDGET_REASON: &str = "read_budget";

/// Fixed warning of a result that is not complete.
pub const BUNDLE_SEARCH_PARTIAL_WARNING: &str = "Not every recording could be searched in full: sessions says what each one holds and not_searched names the bundles left out, so words said there cannot be found. To search bundles left out, name them in another call.";

/// Fixed warning of a result with a bundle that is not private.
pub const BUNDLE_NOT_PRIVATE_WARNING: &str = "A bundle was left out because its folder is not private to you: a copied bundle takes its new folder's permissions, while a move within a drive keeps them. Retain the session again into that folder, or restrict the bundle's folder to your own account, then search again.";

/// Remediation for a path that holds no bundle.
pub const NO_BUNDLES_REMEDIATION: &str = "A path names no retained bundle and no folder holding one, so nothing was searched for it: a bundle is a folder with a bundle.json, and a folder of bundles is read one level down. Check the path; session retain <session> --output <new-folder> makes a bundle.";

/// Remediation for more paths than one search names.
pub const TOO_MANY_PATHS_REMEDIATION: &str =
    "A search names at most 64 paths. Name fewer, or put the bundles in one folder and name that.";

/// Remediation for paths that hold too many entries.
pub const TOO_MANY_ENTRIES_REMEDIATION: &str = "The paths hold more than 4,096 entries in all, so nothing was searched. Name fewer folders, or name the bundles themselves.";

/// Remediation for a cursor that does not continue this search.
pub const BUNDLE_CURSOR_REMEDIATION: &str = "The cursor cannot continue this search: it belongs to another query or another set of bundles, a bundle was added, removed, renamed or retained again, or it expired. Run the search again without --cursor and pass back only the next_cursor it returns.";

/// Which transcript state a [`BundleTranscript`] is.
const fn transcript_state(transcript: &BundleTranscript) -> &'static str {
    match transcript {
        BundleTranscript::None => "none",
        BundleTranscript::Unavailable(_) => "unavailable",
        BundleTranscript::Read {
            untranscribed_ranges: 0,
            ..
        } => "complete",
        BundleTranscript::Read { .. } => "partial",
    }
}

/// The stable identifier of an unreadable reason.
const fn unreadable_identifier(reason: BundleUnreadable) -> &'static str {
    match reason {
        BundleUnreadable::NotPrivate => "not_private",
        BundleUnreadable::IntegrityFailure => "integrity_failure",
        BundleUnreadable::UnsupportedVersion => "unsupported_version",
        BundleUnreadable::Unreadable => "unreadable",
    }
}

/// A bundle's name as a result presents it.
fn name(bundle: &str) -> String {
    sanitize_untrusted_text(bundle, MAX_BUNDLE_NAME_BYTES)
}

/// A transcript hit's segment, in the view the page asked for.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
enum HitSegment {
    Full(Box<TranscriptSegmentData>),
    Brief(Box<BriefTranscriptSegmentData>),
}

/// One hit of the page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct HitData {
    kind: &'static str,
    #[serde(rename = "match")]
    tier: &'static str,
    bundle: String,
    manifest_sha256: String,
    session_id: String,
    source_id: String,
    evidence_id: String,
    time_us: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    segment: Option<HitSegment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<ScreenTextHitData>,
}

impl HitData {
    fn new(hit: &BundleHit<TranscriptSegmentData>, view: ResultView) -> Self {
        let (segment, line) = match &hit.evidence {
            BundleHitEvidence::Transcript(segment) => (
                Some(match view {
                    ResultView::Full => HitSegment::Full(Box::new(segment.clone())),
                    ResultView::Brief => HitSegment::Brief(Box::new(segment.into())),
                }),
                None,
            ),
            BundleHitEvidence::ScreenText(hit_on_screen) => {
                (None, Some(screen_text_hit(hit, hit_on_screen)))
            }
        };
        Self {
            kind: hit.kind().identifier(),
            tier: hit.position.tier().identifier(),
            bundle: name(&hit.bundle),
            manifest_sha256: hit.identity.manifest_sha256.clone(),
            session_id: hit.identity.session_id.as_str().to_owned(),
            source_id: hit.identity.source_id.as_str().to_owned(),
            evidence_id: hit.evidence_id.clone(),
            time_us: hit.time.as_micros(),
            segment,
            line,
        }
    }
}

fn screen_text_hit<T>(hit: &BundleHit<T>, line: &BundleScreenTextHit) -> ScreenTextHitData {
    ScreenTextHitData::new(&ScreenTextHitPresentation {
        line: &line.line,
        frame: &line.frame,
        candidates: &line.candidates,
        last_time: line.last_time,
        frames: line.frames,
        tier: hit.position.tier(),
    })
}

/// A bundle's transcript in the coverage list.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct TranscriptData {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    revision_id: Option<String>,
    basis: Option<&'static str>,
    untranscribed_ranges: usize,
    untranscribed_us: u64,
}

impl TranscriptData {
    fn new(transcript: &BundleTranscript) -> Self {
        let state = transcript_state(transcript);
        match transcript {
            BundleTranscript::None => Self {
                state,
                reason: None,
                revision_id: None,
                basis: None,
                untranscribed_ranges: 0,
                untranscribed_us: 0,
            },
            BundleTranscript::Unavailable(reason) => Self {
                state,
                reason: Some(unreadable_identifier(*reason)),
                revision_id: None,
                basis: None,
                untranscribed_ranges: 0,
                untranscribed_us: 0,
            },
            BundleTranscript::Read {
                revision_id,
                basis,
                untranscribed_ranges,
                untranscribed_micros,
            } => Self {
                state,
                reason: None,
                revision_id: Some(revision_id.as_str().to_owned()),
                basis: Some(basis.identifier()),
                untranscribed_ranges: *untranscribed_ranges,
                untranscribed_us: *untranscribed_micros,
            },
        }
    }
}

/// A bundle's on-screen text in the coverage list.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ScreenTextData {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    index_id: Option<String>,
    coverage: CoverageData,
}

impl ScreenTextData {
    fn new(screen_text: &BundleScreenText) -> Self {
        let state = if screen_text.is_complete() {
            "complete"
        } else {
            match screen_text {
                BundleScreenText::NotRun => "not_run",
                BundleScreenText::Unavailable(_) => "unavailable",
                BundleScreenText::Read { .. } => "partial",
            }
        };
        match screen_text {
            BundleScreenText::NotRun => Self {
                state,
                reason: None,
                index_id: None,
                coverage: CoverageData::new(Default::default(), None, false),
            },
            BundleScreenText::Unavailable(reason) => Self {
                state,
                reason: Some(unreadable_identifier(*reason)),
                index_id: None,
                coverage: CoverageData::new(Default::default(), None, false),
            },
            BundleScreenText::Read {
                index_id,
                period_micros,
                coverage,
                candidates_complete,
                visual_unavailable,
            } => Self {
                state,
                reason: visual_unavailable.map(unreadable_identifier),
                index_id: Some(index_id.as_str().to_owned()),
                coverage: CoverageData::new(*coverage, Some(*period_micros), *candidates_complete),
            },
        }
    }
}

/// One entry of the coverage list.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SessionData {
    bundle: String,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    manifest_sha256: Option<String>,
    session_id: Option<String>,
    source_id: Option<String>,
    source_included: Option<bool>,
    hits: usize,
    transcript: Option<TranscriptData>,
    screen_text: Option<ScreenTextData>,
}

impl SessionData {
    fn searched(coverage: &BundleSessionCoverage) -> Self {
        Self {
            bundle: name(&coverage.bundle),
            state: "searched",
            reason: None,
            manifest_sha256: Some(coverage.identity.manifest_sha256.clone()),
            session_id: Some(coverage.identity.session_id.as_str().to_owned()),
            source_id: Some(coverage.identity.source_id.as_str().to_owned()),
            source_included: Some(coverage.identity.source_included),
            hits: coverage.hits,
            transcript: Some(TranscriptData::new(&coverage.transcript)),
            screen_text: Some(ScreenTextData::new(&coverage.screen_text)),
        }
    }

    fn new(entry: &BundleSessionEntry) -> Self {
        match entry {
            BundleSessionEntry::Searched(coverage) => Self::searched(coverage),
            BundleSessionEntry::Unavailable { bundle, reason } => Self {
                bundle: name(bundle),
                state: "unavailable",
                reason: Some(unreadable_identifier(*reason)),
                manifest_sha256: None,
                session_id: None,
                source_id: None,
                source_included: None,
                hits: 0,
                transcript: None,
                screen_text: None,
            },
        }
    }
}

/// A bundle found and not searched.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct NotSearchedData {
    bundle: String,
    reason: &'static str,
}

impl NotSearchedData {
    fn new(bundle: &BundleNotSearched) -> Self {
        Self {
            bundle: name(&bundle.bundle),
            reason: reason_identifier(bundle.reason),
        }
    }
}

const fn reason_identifier(reason: BundleNotSearchedReason) -> &'static str {
    match reason {
        BundleNotSearchedReason::BundleLimit => BUNDLE_LIMIT_REASON,
        BundleNotSearchedReason::ReadBudget => READ_BUDGET_REASON,
    }
}

/// The counts of a search.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct CountsData {
    found: usize,
    searched: usize,
    unavailable: usize,
    not_searched: usize,
    duplicates: usize,
    skipped_entries: usize,
    screen_text_incomplete: usize,
}

impl From<BundleSearchCounts> for CountsData {
    fn from(counts: BundleSearchCounts) -> Self {
        Self {
            found: counts.found,
            searched: counts.searched,
            unavailable: counts.unavailable,
            not_searched: counts.not_searched,
            duplicates: counts.duplicates,
            skipped_entries: counts.skipped_entries,
            screen_text_incomplete: counts.screen_text_incomplete,
        }
    }
}

/// Data of a `bundle search` result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BundleSearchData {
    #[serde(skip_serializing_if = "Option::is_none")]
    view: Option<&'static str>,
    query: SearchQueryData,
    hits: Vec<HitData>,
    total_hits: usize,
    next_cursor: Option<String>,
    bundles: CountsData,
    sessions: Vec<SessionData>,
    not_searched: Vec<NotSearchedData>,
}

impl BundleSearchData {
    /// Presents one page, cut to fit the result limit when its text is long.
    #[must_use]
    pub fn new(
        query: &SearchQuery,
        outcome: &BundleSearchOutcome<TranscriptSegmentData>,
        view: ResultView,
    ) -> Self {
        let mut hits = Vec::with_capacity(outcome.hits.len());
        let mut bytes = 0_usize;
        for hit in &outcome.hits {
            let data = HitData::new(hit, view);
            bytes = bytes.saturating_add(serialized_length(&data));
            if bytes > MAX_HIT_BYTES && !hits.is_empty() {
                break;
            }
            hits.push(data);
        }
        let shown = hits.len();
        let more = outcome.has_more || shown < outcome.hits.len();
        Self {
            view: match view {
                ResultView::Full => None,
                ResultView::Brief => Some(BRIEF_VIEW),
            },
            query: SearchQueryData::new(query),
            hits,
            total_hits: outcome.total_hits,
            next_cursor: more
                .then(|| shown.checked_sub(1).and_then(|last| outcome.hits.get(last)))
                .flatten()
                .map(|hit| hit.continuation.clone()),
            bundles: outcome.counts.into(),
            sessions: outcome.sessions.iter().map(SessionData::new).collect(),
            not_searched: outcome
                .not_searched
                .iter()
                .map(NotSearchedData::new)
                .collect(),
        }
    }
}

fn serialized_length(hit: &HitData) -> usize {
    serde_json::to_vec(hit).map_or(usize::MAX, |bytes| bytes.len())
}

/// The distinct reasons a result is `partial`, in a fixed order.
fn partial_reasons(outcome: &BundleSearchOutcome<TranscriptSegmentData>) -> Vec<String> {
    let searched = || {
        outcome.sessions.iter().filter_map(|entry| match entry {
            BundleSessionEntry::Searched(coverage) => Some(&coverage.transcript),
            BundleSessionEntry::Unavailable { .. } => None,
        })
    };
    let unavailable = outcome.counts.unavailable > 0
        || searched().any(|transcript| matches!(transcript, BundleTranscript::Unavailable(_)));
    let left_out = |reason: BundleNotSearchedReason| {
        outcome
            .not_searched
            .iter()
            .any(|bundle| bundle.reason == reason)
    };
    [
        (
            searched().any(|transcript| matches!(transcript, BundleTranscript::None)),
            NO_TRANSCRIPT_REASON,
        ),
        (
            searched().any(|transcript| {
                matches!(transcript, BundleTranscript::Read { .. }) && transcript.has_gap()
            }),
            UNTRANSCRIBED_REASON,
        ),
        (unavailable, BUNDLE_UNAVAILABLE_REASON),
        (
            left_out(BundleNotSearchedReason::BundleLimit),
            BUNDLE_LIMIT_REASON,
        ),
        (
            left_out(BundleNotSearchedReason::ReadBudget),
            READ_BUDGET_REASON,
        ),
    ]
    .into_iter()
    .filter_map(|(applies, reason)| applies.then(|| reason.to_owned()))
    .collect()
}

/// The complete `bundle search` result.
///
/// # Errors
///
/// Returns the serialization error when the data cannot be represented as JSON.
pub fn bundle_search_response(
    query: &SearchQuery,
    outcome: &BundleSearchOutcome<TranscriptSegmentData>,
    view: ResultView,
) -> Result<OperationResponse<serde_json::Value>, serde_json::Error> {
    let data = BundleSearchData::new(query, outcome, view);
    let reasons = partial_reasons(outcome);
    let not_private = outcome.sessions.iter().any(|entry| {
        matches!(
            entry,
            BundleSessionEntry::Unavailable {
                reason: BundleUnreadable::NotPrivate,
                ..
            }
        )
    });
    let response = OperationResponse::complete(CommandName::BundleSearch.identifier(), &data)?
        .with_lifecycle(LifecycleResponse::retained())
        .with_coverage(CoverageResponse::new(
            !reasons.is_empty(),
            Vec::new(),
            reasons.clone(),
        ));
    let mut warnings = Vec::new();
    if !reasons.is_empty() {
        warnings.push(BUNDLE_SEARCH_PARTIAL_WARNING);
    }
    if not_private {
        warnings.push(BUNDLE_NOT_PRIVATE_WARNING);
    }
    Ok(if warnings.is_empty() {
        response
    } else {
        response.with_warnings(&warnings)
    })
}

/// Fixed-prose remediation for a search refused before or instead of a result;
/// `None` for a cancellation, which has none.
#[must_use]
pub const fn bundle_search_remediation(error: BundleSearchError) -> Option<&'static str> {
    match error {
        BundleSearchError::NoBundles => Some(NO_BUNDLES_REMEDIATION),
        BundleSearchError::TooManyPaths => Some(TOO_MANY_PATHS_REMEDIATION),
        BundleSearchError::TooManyEntries => Some(TOO_MANY_ENTRIES_REMEDIATION),
        BundleSearchError::Cursor(_) => Some(BUNDLE_CURSOR_REMEDIATION),
        BundleSearchError::Cancelled => None,
    }
}
