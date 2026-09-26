//! Building and paging a session's visual-candidate index (P08).
//!
//! The domain decides what a window's samples mean; this module decides
//! which windows to analyse, identifies the candidates, and pages them. A
//! call analyses the missing windows of a requested range in ascending order,
//! at most [`MAX_WINDOWS_PER_EXTENSION`] (30 minutes of media), so a long
//! source is indexed over several calls and every call's cost is bounded.
//! What a call could not analyse is reported as coverage, never skipped
//! silently:
//!
//! - a transient failure (the decode's deadline, admission busy,
//!   cancellation) stops the call; the window and every later one stay
//!   unanalysed and a later call retries them;
//! - a provider rejection is permanent for that window: it is recorded as
//!   `undecodable` so later calls do not decode it again;
//! - any other failure (a changed source, a provider that cannot run) fails
//!   the call and nothing is committed.
//!
//! Identities are derived from content with SHA-256 like transcript
//! identities: a candidate from the session, source, stream, profile, window
//! ordinal and representative time. Windows never merge and an analysed
//! window is never analysed again, so a candidate keeps its identity in
//! every later revision of the index.

use std::{
    error::Error,
    fmt,
    future::Future,
    num::{NonZeroU32, NonZeroUsize},
};

use vsift_domain::{
    CoverageGapReason, CursorError, CursorToken, FrameDimensions, MediaTime, PageLimit,
    QueryDigest, SessionId, SourceId, TimeRange, VISUAL_WINDOW_MICROS, VisualAnalysisError,
    VisualCandidate, VisualCandidateId, VisualCoverageGap, VisualIndex, VisualIndexError,
    VisualIndexId, VisualIndexParts, VisualIndexProfile, VisualIndexWindow, VisualSample,
    VisualWindow, VisualWindowOutcome, analyse_window, visual_window_count,
};

use crate::transcript::{derived_identity, sha256_hex};

/// Most windows one call analyses: 30 minutes of media.
pub const MAX_WINDOWS_PER_EXTENSION: usize = 30;
/// Cursor generation of candidate pages. Index revisions are not a
/// generation: a cursor is bound to the state of the windows its own range
/// covers through the query digest instead, so it survives unrelated
/// extensions of the index.
const CANDIDATE_CURSOR_GENERATION: u64 = 1;

/// Why a window's samples could not be decoded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualSamplingError {
    /// Admission capacity was unavailable; retryable.
    Busy,
    /// The decode exceeded its deadline; retryable.
    Deadline,
    /// The caller cancelled; retryable.
    Cancelled,
    /// The provider rejected or could not decode the window, or produced
    /// output outside its bounds; recorded as `undecodable`.
    Undecodable,
    /// The source changed or the provider could not run; the call fails.
    Io,
}

impl fmt::Display for VisualSamplingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Busy => "visual sampling admission is busy",
            Self::Deadline => "visual sampling exceeded its deadline",
            Self::Cancelled => "visual sampling was cancelled",
            Self::Undecodable => "the window's frames could not be decoded",
            Self::Io => "visual sampling could not run",
        })
    }
}

impl Error for VisualSamplingError {}

/// Port that decodes one window's samples from the bound source.
///
/// An implementation returns the window's samples in time order, including
/// at most one lead-in sample before the window start (see
/// [`VisualWindow::lead_in_start`]).
pub trait VisualSampler: Send + Sync {
    /// Decodes `window`'s samples.
    fn window_samples(
        &self,
        window: VisualWindow,
    ) -> impl Future<Output = Result<Vec<VisualSample>, VisualSamplingError>> + Send;
}

/// The source and stream an index describes, inside one session.
#[derive(Clone, Copy, Debug)]
pub struct VisualIndexScope<'a> {
    /// Session holding the index.
    pub session_id: &'a SessionId,
    /// Source the index describes.
    pub source_id: &'a SourceId,
    /// Selected video stream index.
    pub stream_index: u32,
    /// The stream's orientation-correct displayed dimensions.
    pub displayed_dimensions: FrameDimensions,
    /// Probed normalized source duration.
    pub duration: MediaTime,
    /// Analysis profile.
    pub profile: VisualIndexProfile,
}

impl VisualIndexScope<'_> {
    /// Whether `index` describes this scope's source, stream, dimensions,
    /// duration and profile (the session is checked through identities).
    #[must_use]
    pub fn describes(&self, index: &VisualIndex) -> bool {
        index.source_id() == self.source_id
            && index.stream_index() == self.stream_index
            && index.displayed_dimensions() == self.displayed_dimensions
            && index.duration() == self.duration
            && index.profile() == self.profile
    }
}

/// Derives a candidate's identity.
///
/// # Errors
///
/// Returns [`VisualIndexError::InvalidCandidate`] only if the derived value
/// were not canonical, which would be an internal fault.
pub fn visual_candidate_id(
    scope: &VisualIndexScope<'_>,
    window_ordinal: u32,
    representative: MediaTime,
) -> Result<VisualCandidateId, VisualIndexError> {
    VisualCandidateId::parse(derived_identity(
        "vcd_",
        "vsift.visual-candidate.v1",
        &[
            scope.session_id.as_str(),
            scope.source_id.as_str(),
            &scope.stream_index.to_string(),
            scope.profile.identifier(),
            &window_ordinal.to_string(),
            &representative.as_micros().to_string(),
        ],
    ))
    .map_err(|_| VisualIndexError::InvalidCandidate)
}

/// Derives the identity of revision `number` of an index.
///
/// # Errors
///
/// Returns [`VisualIndexError::InvalidWindow`] only if the derived value
/// were not canonical, which would be an internal fault.
pub fn visual_index_id(
    scope: &VisualIndexScope<'_>,
    number: NonZeroU32,
) -> Result<VisualIndexId, VisualIndexError> {
    VisualIndexId::parse(derived_identity(
        "vix_",
        "vsift.visual-index.v1",
        &[
            scope.session_id.as_str(),
            scope.source_id.as_str(),
            &scope.stream_index.to_string(),
            scope.profile.identifier(),
            &number.get().to_string(),
        ],
    ))
    .map_err(|_| VisualIndexError::InvalidWindow)
}

/// Checks that every identity in `index` is the one its content derives.
///
/// A stored index is decoded structurally by the domain; this closes the
/// remaining gap, so a record whose identities were edited is rejected.
///
/// # Errors
///
/// Returns [`VisualIndexError::InvalidCandidate`] for a candidate and
/// [`VisualIndexError::InvalidWindow`] for the revision identity, or for a
/// scope that does not describe the index.
pub fn verify_visual_index_identities(
    scope: &VisualIndexScope<'_>,
    index: &VisualIndex,
) -> Result<(), VisualIndexError> {
    if !scope.describes(index) || visual_index_id(scope, index.number())? != *index.id() {
        return Err(VisualIndexError::InvalidWindow);
    }
    for window in index.windows() {
        for candidate in window.candidates() {
            if visual_candidate_id(scope, window.window().ordinal(), candidate.representative())?
                != *candidate.id()
            {
                return Err(VisualIndexError::InvalidCandidate);
            }
        }
    }
    Ok(())
}

/// A request to analyse the missing windows of a range.
#[derive(Clone, Copy, Debug)]
pub struct ExtendVisualIndexRequest<'a> {
    /// Session, source and stream.
    pub scope: VisualIndexScope<'a>,
    /// The session's newest committed index, if any.
    pub previous: Option<&'a VisualIndex>,
    /// Requested source range; windows intersecting it are analysed.
    pub range: TimeRange,
}

/// Why a call stopped before analysing every missing window of its range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualExtensionStop {
    /// [`MAX_WINDOWS_PER_EXTENSION`] windows were analysed; a later call continues.
    WindowLimit,
    /// Window `ordinal`'s decode exceeded its deadline.
    Deadline {
        /// Ordinal of the window that timed out.
        ordinal: u32,
    },
    /// Admission was busy before window `ordinal`.
    Busy {
        /// Ordinal of the window that was not started.
        ordinal: u32,
    },
    /// The call was cancelled at window `ordinal`.
    Cancelled {
        /// Ordinal of the window that was cancelled.
        ordinal: u32,
    },
}

/// The outcome of one extension call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualIndexExtension {
    /// The new revision to commit, when the call recorded any window.
    pub revision: Option<VisualIndex>,
    /// Ordinals of the windows recorded by this call, ascending.
    pub recorded: Vec<u32>,
    /// Why the call stopped early, if it did.
    pub stop: Option<VisualExtensionStop>,
}

impl VisualIndexExtension {
    /// The windows this call recorded, in ascending ordinal order, as they
    /// stand in its revision; empty when nothing was recorded.
    #[must_use]
    pub fn recorded_windows(&self) -> Vec<VisualIndexWindow> {
        self.revision.as_ref().map_or_else(Vec::new, |revision| {
            self.recorded
                .iter()
                .filter_map(|ordinal| revision.window(*ordinal).cloned())
                .collect()
        })
    }
}

/// Why an extension call failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualIndexBuildError {
    /// The requested range starts at or after the source end.
    RangeOutsideSource,
    /// The previous index describes another source, stream, duration or profile.
    ScopeMismatch,
    /// Sampling failed in a way that must not be recorded.
    Sampling(VisualSamplingError),
    /// An assembled window or index violated an invariant; an internal fault.
    Invalid(VisualIndexError),
}

impl fmt::Display for VisualIndexBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RangeOutsideSource => {
                formatter.write_str("requested range starts after the source end")
            }
            Self::ScopeMismatch => {
                formatter.write_str("stored visual index describes another source or stream")
            }
            Self::Sampling(error) => error.fmt(formatter),
            Self::Invalid(error) => error.fmt(formatter),
        }
    }
}

impl Error for VisualIndexBuildError {}

/// Clips `range` to the source.
///
/// # Errors
///
/// Returns [`VisualIndexBuildError::RangeOutsideSource`] when nothing of the
/// range lies inside the source.
pub fn clip_to_source(
    range: TimeRange,
    duration: MediaTime,
) -> Result<TimeRange, VisualIndexBuildError> {
    TimeRange::new(range.start(), range.end().min(duration))
        .map_err(|_| VisualIndexBuildError::RangeOutsideSource)
}

/// Analyses the missing windows of `request.range` in ascending order.
///
/// At most [`MAX_WINDOWS_PER_EXTENSION`] windows are decoded. The new
/// revision holds every window of the previous one unchanged plus the
/// windows recorded now.
///
/// # Errors
///
/// Returns [`VisualIndexBuildError`] for a range outside the source, a
/// previous index of another scope, a sampling failure that must not be
/// recorded, or an internal fault.
pub async fn extend_visual_index(
    request: ExtendVisualIndexRequest<'_>,
    sampler: &impl VisualSampler,
) -> Result<VisualIndexExtension, VisualIndexBuildError> {
    extend_visual_index_within(request, sampler, MAX_WINDOW_BUDGET).await
}

/// [`MAX_WINDOWS_PER_EXTENSION`] as a budget.
const MAX_WINDOW_BUDGET: NonZeroUsize = match NonZeroUsize::new(MAX_WINDOWS_PER_EXTENSION) {
    Some(budget) => budget,
    None => NonZeroUsize::MIN,
};

/// [`extend_visual_index`] with a smaller per-call window budget.
///
/// A host that must answer sooner than 30 windows of decoding allow (and
/// the opt-in continuation tests) lowers the budget; a budget above
/// [`MAX_WINDOWS_PER_EXTENSION`] is clamped to it, so no caller can make one
/// call unbounded.
///
/// # Errors
///
/// As for [`extend_visual_index`].
pub async fn extend_visual_index_within(
    request: ExtendVisualIndexRequest<'_>,
    sampler: &impl VisualSampler,
    window_budget: NonZeroUsize,
) -> Result<VisualIndexExtension, VisualIndexBuildError> {
    let budget = window_budget.get().min(MAX_WINDOWS_PER_EXTENSION);
    let scope = request.scope;
    let range = clip_to_source(request.range, scope.duration)?;
    if let Some(previous) = request.previous
        && !scope.describes(previous)
    {
        return Err(VisualIndexBuildError::ScopeMismatch);
    }
    let mut windows: Vec<VisualIndexWindow> = request
        .previous
        .map(|previous| previous.windows().to_vec())
        .unwrap_or_default();
    let mut recorded = Vec::new();
    let mut stop = None;
    for ordinal in window_ordinals(range, scope.duration) {
        if request
            .previous
            .is_some_and(|previous| previous.window(ordinal).is_some())
        {
            continue;
        }
        if recorded.len() == budget {
            stop = Some(VisualExtensionStop::WindowLimit);
            break;
        }
        let window =
            VisualWindow::new(ordinal, scope.duration).map_err(VisualIndexBuildError::Invalid)?;
        let outcome = match sampler.window_samples(window).await {
            Ok(decoded) => match analyse_window(window, &decoded, scope.profile.policy()) {
                Ok(analysis) => identified_outcome(&scope, analysis)?,
                Err(
                    VisualAnalysisError::TooManySamples
                    | VisualAnalysisError::SampleOutsideWindow
                    | VisualAnalysisError::UnorderedSamples,
                ) => VisualWindowOutcome::Undecodable,
            },
            Err(VisualSamplingError::Undecodable) => VisualWindowOutcome::Undecodable,
            Err(VisualSamplingError::Deadline) => {
                stop = Some(VisualExtensionStop::Deadline { ordinal });
                break;
            }
            Err(VisualSamplingError::Busy) => {
                stop = Some(VisualExtensionStop::Busy { ordinal });
                break;
            }
            Err(VisualSamplingError::Cancelled) => {
                stop = Some(VisualExtensionStop::Cancelled { ordinal });
                break;
            }
            Err(VisualSamplingError::Io) => {
                return Err(VisualIndexBuildError::Sampling(VisualSamplingError::Io));
            }
        };
        windows.push(
            VisualIndexWindow::new(window, outcome, scope.profile.policy())
                .map_err(VisualIndexBuildError::Invalid)?,
        );
        recorded.push(ordinal);
    }
    if recorded.is_empty() {
        return Ok(VisualIndexExtension {
            revision: None,
            recorded,
            stop,
        });
    }
    windows.sort_by_key(|window| window.window().ordinal());
    let number = request
        .previous
        .map_or(Some(NonZeroU32::MIN), |previous| {
            previous.number().checked_add(1)
        })
        .ok_or(VisualIndexBuildError::Invalid(
            VisualIndexError::InvalidWindow,
        ))?;
    let revision = assemble_revision(&scope, number, windows)?;
    Ok(VisualIndexExtension {
        revision: Some(revision),
        recorded,
        stop,
    })
}

fn assemble_revision(
    scope: &VisualIndexScope<'_>,
    number: NonZeroU32,
    windows: Vec<VisualIndexWindow>,
) -> Result<VisualIndex, VisualIndexBuildError> {
    VisualIndex::new(VisualIndexParts {
        id: visual_index_id(scope, number).map_err(VisualIndexBuildError::Invalid)?,
        number,
        source_id: scope.source_id.clone(),
        stream_index: scope.stream_index,
        displayed_dimensions: scope.displayed_dimensions,
        duration: scope.duration,
        profile: scope.profile,
        windows,
    })
    .map_err(VisualIndexBuildError::Invalid)
}

/// Carries the windows one call recorded onto the session's newest
/// committed index, for a commit that lost a race with another call.
///
/// Two calls may analyse overlapping ranges concurrently; the one that
/// commits second finds a newer revision than the one it started from.
/// Windows are pure functions of the source, stream and profile, so a
/// window both calls analysed is identical in both, and the union is what
/// either call would have produced from the other's revision: `newest` is
/// kept unchanged and every recorded window it lacks is added. The result
/// is revision `newest + 1`, or `None` when `newest` already holds every
/// recorded window (nothing is left to commit).
///
/// # Errors
///
/// Returns [`VisualIndexBuildError::ScopeMismatch`] when `newest` describes
/// another source or stream, and [`VisualIndexBuildError::Invalid`] for an
/// internal fault.
pub fn merge_visual_extension(
    scope: &VisualIndexScope<'_>,
    newest: Option<&VisualIndex>,
    recorded: &[VisualIndexWindow],
) -> Result<Option<VisualIndex>, VisualIndexBuildError> {
    if newest.is_some_and(|index| !scope.describes(index)) {
        return Err(VisualIndexBuildError::ScopeMismatch);
    }
    let mut windows: Vec<VisualIndexWindow> = newest
        .map(|index| index.windows().to_vec())
        .unwrap_or_default();
    let before = windows.len();
    for window in recorded {
        let ordinal = window.window().ordinal();
        if !windows
            .iter()
            .any(|existing| existing.window().ordinal() == ordinal)
        {
            windows.push(window.clone());
        }
    }
    if windows.len() == before {
        return Ok(None);
    }
    windows.sort_by_key(|window| window.window().ordinal());
    let number = newest
        .map_or(Some(NonZeroU32::MIN), |index| index.number().checked_add(1))
        .ok_or(VisualIndexBuildError::Invalid(
            VisualIndexError::InvalidWindow,
        ))?;
    assemble_revision(scope, number, windows).map(Some)
}

/// The parts of `range` (clipped to the source) that analysed windows
/// cover, merged and in time order.
///
/// Undecodable and unanalysed windows are not analysed; frameless cells and
/// budget-exhausted windows lie inside analysed windows and are reported as
/// gaps by [`visual_coverage_gaps`] as well, so the two lists may overlap.
#[must_use]
pub fn analysed_ranges(index: &VisualIndex, range: TimeRange) -> Vec<TimeRange> {
    let Ok(range) = clip_to_source(range, index.duration()) else {
        return Vec::new();
    };
    let mut ranges: Vec<TimeRange> = Vec::new();
    for ordinal in index.window_ordinals(range) {
        let Some(window) = index.window(ordinal) else {
            continue;
        };
        if matches!(window.outcome(), VisualWindowOutcome::Undecodable) {
            continue;
        }
        let part = window.window().range();
        let Ok(part) = TimeRange::new(part.start().max(range.start()), part.end().min(range.end()))
        else {
            continue;
        };
        match ranges.last_mut() {
            Some(last) if last.end() == part.start() => {
                if let Ok(joined) = TimeRange::new(last.start(), part.end()) {
                    *last = joined;
                }
            }
            _ => ranges.push(part),
        }
    }
    ranges
}

/// Whether any window intersecting `range` is recorded in `index`
/// (analysed or undecodable), so something of the range is indexed.
#[must_use]
pub fn indexes_any_of(index: Option<&VisualIndex>, range: TimeRange) -> bool {
    index.is_some_and(|index| {
        index
            .window_ordinals(range)
            .any(|ordinal| index.window(ordinal).is_some())
    })
}

/// Ordinals of the windows of `range` (clipped to the source) that `index`
/// has not recorded; every window when there is no index.
#[must_use]
pub fn missing_windows(
    index: Option<&VisualIndex>,
    duration: MediaTime,
    range: TimeRange,
) -> Vec<u32> {
    let Ok(range) = clip_to_source(range, duration) else {
        return Vec::new();
    };
    window_ordinals(range, duration)
        .filter(|ordinal| index.is_none_or(|index| index.window(*ordinal).is_none()))
        .collect()
}

fn identified_outcome(
    scope: &VisualIndexScope<'_>,
    analysis: vsift_domain::WindowAnalysis,
) -> Result<VisualWindowOutcome, VisualIndexBuildError> {
    let ordinal = analysis.window.ordinal();
    let mut candidates = Vec::with_capacity(analysis.candidates.len());
    for draft in analysis.candidates {
        let id = visual_candidate_id(scope, ordinal, draft.representative)
            .map_err(VisualIndexBuildError::Invalid)?;
        candidates.push(VisualCandidate::new(id, draft));
    }
    Ok(VisualWindowOutcome::Analysed {
        sample_count: analysis.sample_count,
        candidates,
        dropped_candidates: analysis.dropped_candidates,
        frameless_cells: analysis.frameless_cells,
    })
}

fn window_ordinals(range: TimeRange, duration: MediaTime) -> std::ops::Range<u32> {
    let count = visual_window_count(duration);
    let first = u32::try_from(range.start().as_micros() / VISUAL_WINDOW_MICROS)
        .unwrap_or(u32::MAX)
        .min(count);
    let last = u32::try_from(range.end().as_micros().div_ceil(VISUAL_WINDOW_MICROS))
        .unwrap_or(u32::MAX)
        .min(count);
    first..last
}

/// The honest coverage gaps of `range`, after an extension call.
///
/// With no index at all, every window of the range is `not_analyzed`. A
/// window at which the call stopped on its deadline is reported as
/// `deadline_exceeded` rather than `not_analyzed`, so the caller learns why.
/// Gaps are in time order and clipped to the range and the source.
#[must_use]
pub fn visual_coverage_gaps(
    index: Option<&VisualIndex>,
    duration: MediaTime,
    range: TimeRange,
    stop: Option<VisualExtensionStop>,
) -> Vec<VisualCoverageGap> {
    let Ok(range) = clip_to_source(range, duration) else {
        return Vec::new();
    };
    let mut gaps = match index {
        Some(index) => index.coverage_gaps(range),
        None => window_ordinals(range, duration)
            .filter_map(|ordinal| VisualWindow::new(ordinal, duration).ok())
            .filter_map(|window| {
                TimeRange::new(
                    window.range().start().max(range.start()),
                    window.range().end().min(range.end()),
                )
                .ok()
            })
            .map(|part| VisualCoverageGap {
                range: part,
                reason: CoverageGapReason::NotAnalyzed,
                dropped_candidates: 0,
            })
            .collect(),
    };
    if let Some(VisualExtensionStop::Deadline { ordinal }) = stop
        && let Ok(window) = VisualWindow::new(ordinal, duration)
    {
        for gap in &mut gaps {
            if gap.reason == CoverageGapReason::NotAnalyzed
                && window.range().start() <= gap.range.start()
                && gap.range.end() <= window.range().end()
            {
                gap.reason = CoverageGapReason::DeadlineExceeded;
            }
        }
    }
    gaps
}

/// A bounded request for candidates in a source range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidatePageRequest {
    /// Half-open source range; candidates whose representative time lies in
    /// it are returned.
    pub range: TimeRange,
    /// Maximum candidates on this page.
    pub limit: PageLimit,
    /// Opaque continuation token from the previous page of the same query.
    pub cursor: Option<String>,
}

/// One bounded page of candidates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidatePage<'a> {
    /// Candidates in time order.
    pub candidates: Vec<&'a VisualCandidate>,
    /// Token for the next page, absent on the last page.
    pub next_cursor: Option<CursorToken>,
}

/// Why a candidate page request was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateQueryError {
    /// The cursor is malformed, expired, or belongs to another query or
    /// session, or the windows of its range changed since it was issued.
    Cursor(CursorError),
}

impl fmt::Display for CandidateQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cursor(error) => error.fmt(formatter),
        }
    }
}

impl Error for CandidateQueryError {}

/// Returns one page of `index`'s candidates for `request`.
///
/// A cursor binds the session, an expiry and a digest of the query: the
/// source, stream, profile and range, and the state of every window the
/// range touches (not analysed, undecodable, or the identities of its
/// candidates). Analysing windows outside the range leaves the cursor valid;
/// any change inside it rejects the cursor rather than silently skipping or
/// repeating candidates.
///
/// # Errors
///
/// Returns [`CandidateQueryError::Cursor`] for a rejected cursor.
pub fn page_candidates<'a>(
    session_id: &SessionId,
    index: &'a VisualIndex,
    request: &CandidatePageRequest,
    cursor_expires_at_micros: u64,
    now_micros: u64,
) -> Result<CandidatePage<'a>, CandidateQueryError> {
    let digest =
        candidate_query_digest(index, request.range).map_err(CandidateQueryError::Cursor)?;
    let matching: Vec<&VisualCandidate> = index.candidates_in(request.range).collect();
    let start = match &request.cursor {
        None => 0,
        Some(encoded) => {
            let token = CursorToken::parse(encoded).map_err(CandidateQueryError::Cursor)?;
            token
                .validate_scope(session_id, CANDIDATE_CURSOR_GENERATION, &digest, now_micros)
                .map_err(CandidateQueryError::Cursor)?;
            let position = matching
                .iter()
                .position(|candidate| candidate_key(candidate) == token.last_item_key())
                .ok_or(CandidateQueryError::Cursor(CursorError::InvalidLastItemKey))?;
            position + 1
        }
    };
    let limit = usize::from(request.limit.get());
    let candidates: Vec<&VisualCandidate> =
        matching.iter().skip(start).take(limit).copied().collect();
    let has_more = matching.len() > start + candidates.len();
    let next_cursor = match (has_more, candidates.last()) {
        (true, Some(last)) => Some(
            CursorToken::new(
                session_id.clone(),
                CANDIDATE_CURSOR_GENERATION,
                digest,
                candidate_key(last),
                cursor_expires_at_micros,
            )
            .map_err(CandidateQueryError::Cursor)?,
        ),
        _ => None,
    };
    Ok(CandidatePage {
        candidates,
        next_cursor,
    })
}

/// The last-item key of a candidate: `<window ordinal>-<representative us>`.
fn candidate_key(candidate: &VisualCandidate) -> String {
    let micros = candidate.representative().as_micros();
    format!("{}-{micros}", micros / VISUAL_WINDOW_MICROS)
}

/// Digest of a candidate query and the state of the windows it touches.
///
/// # Errors
///
/// Returns [`CursorError::InvalidQueryDigest`] only if the digest were not
/// canonical, which would be an internal fault.
pub fn candidate_query_digest(
    index: &VisualIndex,
    range: TimeRange,
) -> Result<QueryDigest, CursorError> {
    let mut material = format!(
        "vsift.candidates.v1\n{}\n{}\n{}\n{}\n{}",
        index.source_id().as_str(),
        index.stream_index(),
        index.profile().identifier(),
        range.start().as_micros(),
        range.end().as_micros()
    );
    for ordinal in index.window_ordinals(range) {
        material.push('\n');
        material.push_str(&ordinal.to_string());
        match index.window(ordinal).map(VisualIndexWindow::outcome) {
            None => material.push_str(":not_analyzed"),
            Some(VisualWindowOutcome::Undecodable) => material.push_str(":undecodable"),
            Some(VisualWindowOutcome::Analysed { candidates, .. }) => {
                material.push_str(":analysed");
                for candidate in candidates {
                    material.push(':');
                    material.push_str(candidate.id().as_str());
                }
            }
        }
    }
    QueryDigest::parse(sha256_hex(material.as_bytes()))
}

#[cfg(test)]
mod tests;
