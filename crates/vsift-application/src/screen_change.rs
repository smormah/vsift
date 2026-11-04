//! What changed on screen between two times (ADR 0034).
//!
//! The changes come from the visual index (ADR 0018); this module pages
//! them, names them, and compares each one's two exact frames. Taking a frame
//! and reading its pixels are a port, [`ScreenChangeFrames`], which the
//! engine implements with `frame get` (ADR 0019) and the stored image, so
//! every frame a change cites is ordinary frame evidence.

use std::{error::Error, fmt, future::Future};

use vsift_domain::{
    ChangeRegion, CoverageGapReason, CropRect, CursorError, CursorToken, EvidenceId,
    FrameComparison, FrameDimensions, MediaTime, PageLimit, PageLimitError, PlannedScreenChange,
    QueryDigest, RgbPixels, ScreenChangeError, ScreenChangeId, ScreenChangeProfile, SessionId,
    TimeRange, VISUAL_WINDOW_MICROS, VisualCandidateId, VisualCoverageGap, VisualIndex,
    compare_frames, plan_screen_changes,
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
    pub crops: ScreenChangeCrops,
}

/// Whether a call cuts its changes' largest regions out of their frames
/// (ADR 0034 section 4).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ScreenChangeCrops {
    /// Regions are listed as rectangles only: slice 1's answer.
    #[default]
    NoCrops,
    /// Each listed change's [`CROPPED_REGIONS_PER_CHANGE`] largest regions are
    /// cropped from its before and after frames, at most
    /// [`MAX_CROPS_PER_CALL`] crops a call.
    LargestRegions,
}

impl ScreenChangeCrops {
    /// The word the cursor digest binds. These are exactly the words slice 1
    /// bound for its boolean, so a cursor issued before crops existed stays
    /// valid; changing one would reject every such cursor.
    #[must_use]
    pub const fn cursor_identifier(self) -> &'static str {
        match self {
            Self::NoCrops => "no-crops",
            Self::LargestRegions => "crops",
        }
    }
}

/// Regions of one change that are cropped: the largest, by changed pixels.
pub const CROPPED_REGIONS_PER_CHANGE: usize = 2;
/// Most crops one call names: eight regions, each from both frames. It
/// counts names, so a crop two regions name counts twice although it is one
/// record (see [`CropSummary::made`]).
pub const MAX_CROPS_PER_CALL: usize = 16;

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
        request.crops.cursor_identifier()
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

    /// Cuts `rect`, in displayed pixels, out of a frame taken by this port:
    /// a crop evidence item whose parent is that frame.
    fn crop(
        &self,
        frame: &TakenFrame,
        rect: CropRect,
    ) -> impl Future<Output = Result<EvidenceId, FrameRefusal<Self::Error>>> + Send;
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
    /// The crops of the change's largest regions, in region order: empty
    /// without crops, otherwise one entry for each of the first
    /// [`CROPPED_REGIONS_PER_CHANGE`] regions.
    pub crops: Vec<RegionCrops>,
}

/// The crops of one region, each present only when it was made.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RegionCrops {
    /// The region cut out of the before frame.
    pub before: Option<EvidenceId>,
    /// The region cut out of the after frame.
    pub after: Option<EvidenceId>,
}

impl RegionCrops {
    /// Whether both crops of the region were made.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.before.is_some() && self.after.is_some()
    }
}

/// Why a call stopped cropping before every planned region had both crops.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CropStop {
    /// The call made [`MAX_CROPS_PER_CALL`] crops: a published bound.
    CropLimit,
    /// The session's evidence room is full.
    SessionEvidenceBudget,
    /// The call's deadline passed.
    DeadlineExceeded,
    /// The caller cancelled.
    Cancelled,
}

impl CropStop {
    /// Published identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::CropLimit => "crop_limit",
            Self::SessionEvidenceBudget => "session_evidence_budget",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Cancelled => "cancelled",
        }
    }

    /// Whether the stop was the room, the deadline or a cancellation rather
    /// than the published bound, so a rerun of the same call may go on.
    #[must_use]
    pub const fn is_interrupted(self) -> bool {
        !matches!(self, Self::CropLimit)
    }

    const fn after(stop: ScreenChangeStop) -> Self {
        match stop {
            ScreenChangeStop::SessionEvidenceBudget => Self::SessionEvidenceBudget,
            ScreenChangeStop::DeadlineExceeded => Self::DeadlineExceeded,
            ScreenChangeStop::Cancelled => Self::Cancelled,
        }
    }
}

/// What a call's cropping made, and why it stopped early.
///
/// It never changes how the page itself is listed or continued: a crop stop
/// leaves [`ComparedPage::stop`], and so the result's status, as the
/// comparison set it (the planner's ruling of 2026-11-04).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CropSummary {
    /// Crop ids the regions name: a crop named by two regions counts twice.
    ///
    /// One change's after frame is often the next change's before frame, so
    /// a region's after crop can be the exact repeat of the next change's
    /// before crop: the same crop record, named twice. The page lists each
    /// distinct crop record once, so it can list fewer crops than this.
    /// [`MAX_CROPS_PER_CALL`] bounds this count.
    pub made: usize,
    /// Planned regions (the first [`CROPPED_REGIONS_PER_CHANGE`] of each
    /// listed change) without their before or their after crop.
    pub regions_without_crop: usize,
    /// Why cropping stopped before every planned region had both crops.
    pub stop: Option<CropStop>,
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
            crops: Vec::new(),
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

/// The regions of a change that are cropped: its first
/// [`CROPPED_REGIONS_PER_CHANGE`], which the comparison lists largest first.
/// A change whose frames were not compared has none.
fn cropped_regions(change: &ComparedChange) -> &[ChangeRegion] {
    match &change.comparison {
        FrameComparison::Compared(found) => found
            .regions
            .get(..CROPPED_REGIONS_PER_CHANGE.min(found.regions.len()))
            .unwrap_or_default(),
        FrameComparison::DimensionsDiffer => &[],
    }
}

/// Crops the largest regions of a page's compared changes (ADR 0034
/// section 4).
///
/// It runs after the whole page is compared, so a crop never costs a listed
/// change its frames. The changes are taken in page order; for each, its
/// first [`CROPPED_REGIONS_PER_CHANGE`] regions, and for each region the
/// before crop, then the after crop, of the same rectangle. It stops at
/// [`MAX_CROPS_PER_CALL`] crops, or when the port has no room, passes its
/// deadline or is cancelled; a crop made before a stop is kept even when its
/// partner is not, because it is committed evidence. Each planned region gets
/// an entry in its change's [`ComparedChange::crops`].
///
/// # Errors
///
/// Returns the port's error for any other failure.
pub async fn crop_screen_changes<P: ScreenChangeFrames>(
    port: &P,
    changes: &mut [ComparedChange],
) -> Result<CropSummary, P::Error> {
    let mut summary = CropSummary::default();
    for change in changes.iter_mut() {
        let regions: Vec<CropRect> = cropped_regions(change)
            .iter()
            .map(|region| region.rect)
            .collect();
        let mut crops = Vec::with_capacity(regions.len());
        for rect in regions {
            let mut region = RegionCrops::default();
            for (frame, slot) in [
                (&change.before, &mut region.before),
                (&change.after, &mut region.after),
            ] {
                if summary.stop.is_none() {
                    summary.stop = crop_one(port, frame, rect, slot, &mut summary.made).await?;
                }
            }
            if !region.is_complete() {
                summary.regions_without_crop += 1;
            }
            crops.push(region);
        }
        change.crops = crops;
    }
    Ok(summary)
}

/// Makes one crop into `slot`, or says why the call stops cropping.
async fn crop_one<P: ScreenChangeFrames>(
    port: &P,
    frame: &TakenFrame,
    rect: CropRect,
    slot: &mut Option<EvidenceId>,
    made: &mut usize,
) -> Result<Option<CropStop>, P::Error> {
    if *made >= MAX_CROPS_PER_CALL {
        return Ok(Some(CropStop::CropLimit));
    }
    if let Some(stop) = port.stop() {
        return Ok(Some(CropStop::after(stop)));
    }
    match port.crop(frame, rect).await {
        Ok(evidence_id) => {
            *slot = Some(evidence_id);
            *made += 1;
            Ok(None)
        }
        Err(FrameRefusal::NoRoom) => Ok(Some(CropStop::SessionEvidenceBudget)),
        Err(FrameRefusal::Stopped(stop)) => Ok(Some(CropStop::after(stop))),
        Err(FrameRefusal::Failed(error)) => Err(error),
    }
}

#[cfg(test)]
mod tests;
