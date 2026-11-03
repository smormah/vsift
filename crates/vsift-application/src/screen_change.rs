//! What changed on screen between two times (ADR 0034).
//!
//! The changes come from the visual index (ADR 0018); this module pages
//! them, names them, and compares each one's two exact frames. Taking a frame
//! and reading its pixels are a port, [`ScreenChangeFrames`], which the
//! engine implements with `frame get` (ADR 0019) and the stored image, so
//! every frame a change cites is ordinary frame evidence.

use std::{error::Error, fmt, future::Future};

use vsift_domain::{
    CoverageGapReason, CursorError, CursorToken, EvidenceId, FrameComparison, FrameDimensions,
    MediaTime, PageLimit, PageLimitError, PlannedScreenChange, QueryDigest, RgbPixels,
    ScreenChangeError, ScreenChangeId, ScreenChangeProfile, SessionId, TimeRange,
    VISUAL_WINDOW_MICROS, VisualCandidateId, VisualCoverageGap, VisualIndex, compare_frames,
    plan_screen_changes,
};

use crate::transcript::{derived_identity, sha256_hex};
use crate::visual::candidate_query_digest;

/// Changes on a page when the request names no limit.
pub const DEFAULT_SCREEN_CHANGE_PAGE: u16 = 10;
/// Most changes on one page: forty frames, within one evidence call's bound.
pub const MAX_SCREEN_CHANGE_PAGE: u16 = 20;
/// Cursor generation of change pages; the window states are in the digest.
const SCREEN_CHANGE_CURSOR_GENERATION: u64 = 1;

/// A validated page size, 1 through [`MAX_SCREEN_CHANGE_PAGE`].
///
/// # Errors
///
/// Returns [`PageLimitError`] outside that range.
pub fn screen_change_page_limit(limit: Option<u16>) -> Result<PageLimit, PageLimitError> {
    let value = limit.unwrap_or(DEFAULT_SCREEN_CHANGE_PAGE);
    if value > MAX_SCREEN_CHANGE_PAGE {
        return Err(PageLimitError::TooLarge);
    }
    PageLimit::new(value)
}

/// A bounded request for the changes of a source range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenChangePageRequest {
    /// Half-open source range; changes whose after frame lies in it are
    /// returned.
    pub range: TimeRange,
    /// Most changes on this page.
    pub limit: PageLimit,
    /// Opaque continuation token from the previous page of the same query.
    pub cursor: Option<String>,
    /// Whether regions are cropped (ADR 0034 section 4); the cursor is bound
    /// to it.
    pub crops: bool,
}

/// One page of planned changes, before their frames are taken.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenChangePage {
    /// The page's changes, in time order.
    pub changes: Vec<PlannedScreenChange>,
    /// Changes of the analysed part of the range after this page.
    pub later: usize,
    /// Digest the page's cursors carry.
    digest: QueryDigest,
}

/// Why a page request was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenChangeQueryError {
    /// The cursor is malformed, expired, or belongs to another query or
    /// session, or a window of its range changed since it was issued.
    Cursor(CursorError),
}

impl fmt::Display for ScreenChangeQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cursor(error) => error.fmt(formatter),
        }
    }
}

impl Error for ScreenChangeQueryError {}

/// Returns one page of `index`'s changes for `request`.
///
/// The cursor binds the session, an expiry and a digest of the query: the
/// candidates' query digest (source, stream, index profile, range and the
/// state of every window the range touches), the region profile, the page
/// size and whether regions are cropped. Any change to those rejects it.
///
/// # Errors
///
/// Returns [`ScreenChangeQueryError::Cursor`] for a rejected cursor.
pub fn page_screen_changes(
    session_id: &SessionId,
    index: &VisualIndex,
    request: &ScreenChangePageRequest,
    now_micros: u64,
) -> Result<ScreenChangePage, ScreenChangeQueryError> {
    let digest =
        screen_change_query_digest(index, request).map_err(ScreenChangeQueryError::Cursor)?;
    let all = plan_screen_changes(index, request.range);
    let start = match &request.cursor {
        None => 0,
        Some(encoded) => {
            let token = CursorToken::parse(encoded).map_err(ScreenChangeQueryError::Cursor)?;
            token
                .validate_scope(
                    session_id,
                    SCREEN_CHANGE_CURSOR_GENERATION,
                    &digest,
                    now_micros,
                )
                .map_err(ScreenChangeQueryError::Cursor)?;
            let position = all
                .iter()
                .position(|change| change_key(change) == token.last_item_key())
                .ok_or(ScreenChangeQueryError::Cursor(
                    CursorError::InvalidLastItemKey,
                ))?;
            position + 1
        }
    };
    let limit = usize::from(request.limit.get());
    let changes: Vec<PlannedScreenChange> = all.iter().skip(start).take(limit).cloned().collect();
    let later = all.len().saturating_sub(start + changes.len());
    Ok(ScreenChangePage {
        changes,
        later,
        digest,
    })
}

impl ScreenChangePage {
    /// The cursor that continues after `last`, a change of this page.
    ///
    /// # Errors
    ///
    /// Returns [`CursorError`] only for an invalid expiry.
    pub fn cursor_after(
        &self,
        session_id: &SessionId,
        last: &PlannedScreenChange,
        expires_at_micros: u64,
    ) -> Result<CursorToken, CursorError> {
        CursorToken::new(
            session_id.clone(),
            SCREEN_CHANGE_CURSOR_GENERATION,
            self.digest.clone(),
            change_key(last),
            expires_at_micros,
        )
    }
}

/// The last-item key of a change: `<window ordinal>-<after us>`.
fn change_key(change: &PlannedScreenChange) -> String {
    let micros = change.after.as_micros();
    format!("{}-{micros}", micros / VISUAL_WINDOW_MICROS)
}

fn screen_change_query_digest(
    index: &VisualIndex,
    request: &ScreenChangePageRequest,
) -> Result<QueryDigest, CursorError> {
    let candidates = candidate_query_digest(index, request.range)?;
    let material = format!(
        "vsift.screen-changes.v1\n{}\n{}\n{}\n{}",
        candidates.as_str(),
        ScreenChangeProfile::RegionsV1.identifier(),
        request.limit.get(),
        if request.crops { "crops" } else { "no-crops" }
    );
    QueryDigest::parse(sha256_hex(material.as_bytes()))
}

/// Derives a change's identity: the same on every call that lists it.
///
/// # Errors
///
/// Returns the identifier error only if the derived value were not
/// canonical, which would be an internal fault.
pub fn screen_change_id(
    session_id: &SessionId,
    index: &VisualIndex,
    change: &PlannedScreenChange,
) -> Result<ScreenChangeId, vsift_domain::IdentifierError> {
    let stream = index.stream_index().to_string();
    let mut parts: Vec<&str> = vec![
        session_id.as_str(),
        index.source_id().as_str(),
        &stream,
        index.profile().identifier(),
        ScreenChangeProfile::RegionsV1.identifier(),
    ];
    parts.extend(change.candidates.iter().map(VisualCandidateId::as_str));
    ScreenChangeId::parse(derived_identity("chg_", "vsift.screen-change.v1", &parts))
}

/// Where a fresh call continues once a range's pages reach the end of its
/// analysed part: the start of the first window still to analyse, if any.
#[must_use]
pub fn first_unanalysed(gaps: &[VisualCoverageGap]) -> Option<MediaTime> {
    gaps.iter()
        .filter(|gap| gap.reason.is_retryable())
        .map(|gap| gap.range.start())
        .min()
}

/// Whether the gaps include any the index dropped changes in.
#[must_use]
pub fn dropped_changes(gaps: &[VisualCoverageGap]) -> u64 {
    gaps.iter()
        .filter(|gap| gap.reason == CoverageGapReason::CandidateBudgetExhausted)
        .map(|gap| u64::from(gap.dropped_candidates))
        .sum()
}

/// Why a fresh call must go on where this one stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenChangeResumeReason {
    /// The pages reached the end of the analysed part of the range; a fresh
    /// call analyses from there.
    NotAnalyzed,
    /// The session's evidence room is full; a fresh call in a new session.
    SessionEvidenceBudget,
    /// The call's deadline passed.
    DeadlineExceeded,
    /// The caller cancelled.
    Cancelled,
}

impl ScreenChangeResumeReason {
    /// Published identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::NotAnalyzed => "not_analyzed",
            Self::SessionEvidenceBudget => "session_evidence_budget",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Cancelled => "cancelled",
        }
    }

    const fn after(stop: ScreenChangeStop) -> Self {
        match stop {
            ScreenChangeStop::SessionEvidenceBudget => Self::SessionEvidenceBudget,
            ScreenChangeStop::DeadlineExceeded => Self::DeadlineExceeded,
            ScreenChangeStop::Cancelled => Self::Cancelled,
        }
    }
}

/// Where and why a fresh call continues (ADR 0034 section 6a).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenChangeResume {
    /// The `--from` of the fresh call.
    pub from: MediaTime,
    /// Why this call did not go on.
    pub reason: ScreenChangeResumeReason,
}

/// How the pages of a range go on after this one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageContinuation<'a> {
    /// More changes of the analysed part follow: a cursor after this change.
    Next(&'a PlannedScreenChange),
    /// A fresh call goes on from here.
    Resume(ScreenChangeResume),
    /// The range is answered to its end.
    Done,
}

/// Decides how the pages go on (ADR 0034 section 6a).
///
/// A page that stopped early resumes at the after time of its first change
/// not listed, never with a cursor, because the session cannot take the next
/// page. A page that ended normally gives a cursor while changes of the
/// analysed part follow; at the end of them it resumes at the first window
/// still to analyse, if any.
#[must_use]
pub fn page_continuation<'a>(
    page: &'a ScreenChangePage,
    compared: &ComparedPage,
    gaps: &[VisualCoverageGap],
) -> PageContinuation<'a> {
    if let Some(stop) = compared.stop {
        return page.changes.get(compared.changes.len()).map_or(
            PageContinuation::Done,
            |first_not_listed| {
                PageContinuation::Resume(ScreenChangeResume {
                    from: first_not_listed.after,
                    reason: ScreenChangeResumeReason::after(stop),
                })
            },
        );
    }
    match page.changes.last() {
        Some(last) if page.later > 0 => PageContinuation::Next(last),
        _ => first_unanalysed(gaps).map_or(PageContinuation::Done, |from| {
            PageContinuation::Resume(ScreenChangeResume {
                from,
                reason: ScreenChangeResumeReason::NotAnalyzed,
            })
        }),
    }
}

/// A frame the port took for a change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TakenFrame {
    /// The frame evidence item.
    pub evidence_id: EvidenceId,
    /// The frame's actual time.
    pub time: MediaTime,
    /// The frame's displayed size.
    pub dimensions: FrameDimensions,
}

/// Why the port took no frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameRefusal<E> {
    /// The session has no evidence room for it.
    NoRoom,
    /// The call's deadline passed or the caller cancelled.
    Stopped(ScreenChangeStop),
    /// Any other failure, which fails the call.
    Failed(E),
}

/// Why a page stopped before its last change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenChangeStop {
    /// The session's evidence room is full.
    SessionEvidenceBudget,
    /// The call's deadline passed.
    DeadlineExceeded,
    /// The caller cancelled.
    Cancelled,
}

impl ScreenChangeStop {
    /// Published identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::SessionEvidenceBudget => "session_evidence_budget",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Port that takes a change's frames and reads their pixels.
pub trait ScreenChangeFrames: Send + Sync {
    /// Failure of the port, which fails the call; a pixel buffer that does
    /// not match its frame's size is one.
    type Error: Send + From<ScreenChangeError>;

    /// Whether the call must stop before its next frame.
    fn stop(&self) -> Option<ScreenChangeStop>;

    /// The frame shown at `at`, an actual sampled frame time.
    fn before_frame(
        &self,
        at: MediaTime,
    ) -> impl Future<Output = Result<TakenFrame, FrameRefusal<Self::Error>>> + Send;

    /// A visual candidate's own frame.
    fn after_frame(
        &self,
        candidate: &VisualCandidateId,
    ) -> impl Future<Output = Result<TakenFrame, FrameRefusal<Self::Error>>> + Send;

    /// The 8-bit RGB pixels of a frame taken by this port, checked against
    /// the stored image's digest.
    ///
    /// # Errors
    ///
    /// Returns the port's error for a stored image that changed, does not
    /// decode, or is larger than any extracted frame.
    fn pixels(&self, frame: &TakenFrame) -> Result<Vec<u8>, Self::Error>;
}

/// One change with its frames and where they differ.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComparedChange {
    /// The change.
    pub change: PlannedScreenChange,
    /// The frame before it.
    pub before: TakenFrame,
    /// The frame after it.
    pub after: TakenFrame,
    /// Where the two differ.
    pub comparison: FrameComparison,
}

/// The changes of a page that were compared, and why it stopped early.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComparedPage {
    /// Compared changes, in page order.
    pub changes: Vec<ComparedChange>,
    /// Why the page stopped before its last change.
    pub stop: Option<ScreenChangeStop>,
}

/// Takes each change's two frames and compares them, one pair at a time.
///
/// A page stops at the first change whose frames it cannot take for room,
/// the deadline or a cancellation; the changes before it stand. Only one
/// pair's pixels are held at a time.
///
/// # Errors
///
/// Returns the port's error for any other failure.
pub async fn compare_screen_changes<P: ScreenChangeFrames>(
    port: &P,
    changes: &[PlannedScreenChange],
) -> Result<ComparedPage, P::Error> {
    let mut compared = Vec::with_capacity(changes.len());
    for change in changes {
        if let Some(stop) = port.stop() {
            return Ok(ComparedPage {
                changes: compared,
                stop: Some(stop),
            });
        }
        let Some(after_candidate) = change.candidates.last() else {
            continue;
        };
        let frames = match port.before_frame(change.before).await {
            Ok(before) => port
                .after_frame(after_candidate)
                .await
                .map(|after| (before, after)),
            Err(refusal) => Err(refusal),
        };
        let (before, after) = match frames {
            Ok(frames) => frames,
            Err(FrameRefusal::Failed(error)) => return Err(error),
            Err(FrameRefusal::NoRoom) => {
                return Ok(ComparedPage {
                    changes: compared,
                    stop: Some(ScreenChangeStop::SessionEvidenceBudget),
                });
            }
            Err(FrameRefusal::Stopped(stop)) => {
                return Ok(ComparedPage {
                    changes: compared,
                    stop: Some(stop),
                });
            }
        };
        let comparison = compare_pair(port, &before, &after)?;
        compared.push(ComparedChange {
            change: change.clone(),
            before,
            after,
            comparison,
        });
    }
    Ok(ComparedPage {
        changes: compared,
        stop: None,
    })
}

fn compare_pair<P: ScreenChangeFrames>(
    port: &P,
    before: &TakenFrame,
    after: &TakenFrame,
) -> Result<FrameComparison, P::Error> {
    if before.dimensions != after.dimensions {
        return Ok(FrameComparison::DimensionsDiffer);
    }
    let before_pixels = port.pixels(before)?;
    let after_pixels = port.pixels(after)?;
    Ok(compare_frames(
        RgbPixels::new(before.dimensions, &before_pixels)?,
        RgbPixels::new(after.dimensions, &after_pixels)?,
    ))
}

#[cfg(test)]
mod tests;
