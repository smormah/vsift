//! What changed on screen between two times: `changes` (ADR 0034).
//!
//! [`Engine::screen_changes`] pages the changes of a source range with the
//! exact frame before and after each and the rectangles where the two
//! differ. The order of steps decides what a call costs:
//!
//! 1. The range is read from the session's visual index exactly as
//!    `candidates` reads it ([`Engine::indexed_range`]): a call without a
//!    cursor first analyses the range's missing windows (at most 30); a call
//!    with a cursor never analyses.
//! 2. The page's changes are planned from the index (ADR 0034 section 2).
//! 3. Each change's two frames are taken with `frame get`, so every image is
//!    ordinary frame evidence, reused when the session already holds it, and
//!    the stored images' pixels are compared, one pair at a time.
//! 4. A page stops at the first change whose frames do not fit the session's
//!    evidence room, the call's deadline or a cancellation; `resume` then
//!    says where a fresh call continues.
//! 5. With crops, and only once the whole page is compared, each listed
//!    change's two largest regions are cut out of its two frames with `crop`
//!    (at most 16 crops a call). A crop stop is reported in the crop summary
//!    alone: it never changes which changes are listed or how the pages go on.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use vsift_application::{
    ComparedChange, CropSummary, EvidenceMediaError, PageContinuation, ScreenChangeCrops,
    ScreenChangeFrameRefusal, ScreenChangeFrames, ScreenChangePageRequest, ScreenChangeQueryError,
    ScreenChangeResume, ScreenChangeStop, SessionStorageError, TakenFrame, analysed_ranges,
    clip_to_source, compare_screen_changes, crop_screen_changes, page_continuation,
    page_screen_changes, screen_change_id, screen_change_page_limit, visual_coverage_gaps,
};
use vsift_domain::{
    CropRect, EvidenceId, EvidenceItem, FrameSelection, MediaTime, ScreenChangeError,
    ScreenChangeId, SessionId, TimeRange, VisualCandidateId, VisualCoverageGap, VisualIndex,
};
use vsift_infrastructure::{PngPixelsError, read_png_pixels};

use crate::{
    candidates::CandidatesRange,
    engine::Engine,
    error::EngineError,
    evidence::{
        CropEvidenceRequest, CropRectangle, EvidenceFile, EvidenceResults, FrameGetRequest,
        FrameTarget,
    },
    sessions::SessionSnapshot,
    verification::Cancellation,
};

/// How long one call may spend taking and comparing frames, after any
/// analysis; the same bound as one evidence call (ADR 0019).
const FRAMES_DEADLINE: Duration = Duration::from_secs(120);
const MICROS_PER_SECOND: u64 = 1_000_000;

/// A bounded request for the screen changes of one source range.
#[derive(Clone, Debug)]
pub struct ScreenChangesRequest {
    /// Session whose video is read.
    pub session: SessionId,
    /// Requested range; changes whose after frame lies in it are returned.
    /// A range that runs past the end of the video is clipped to it.
    pub range: CandidatesRange,
    /// Page size from 1 to 20; `None` selects the default of 10.
    pub limit: Option<u16>,
    /// Opaque continuation token from the previous page of the same range.
    /// A call with a cursor never analyses anything.
    pub cursor: Option<String>,
    /// Whether the changes' two largest regions are cropped from their
    /// frames (ADR 0034 section 4); the cursor is bound to it.
    pub crops: ScreenChangeCrops,
    /// Stops analysis at its next window and the frames at their next one.
    pub cancellation: Cancellation,
}

/// One listed change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenChangeItem {
    /// The change's identity, the same on every call.
    pub id: ScreenChangeId,
    /// The change, its frames and where they differ.
    pub compared: ComparedChange,
}

/// One bounded page of screen changes and the coverage of its range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenChangesResults {
    session: SessionSnapshot,
    index: VisualIndex,
    range: TimeRange,
    searched: TimeRange,
    changes: Vec<ScreenChangeItem>,
    frames: Vec<EvidenceItem>,
    files: Vec<EvidenceFile>,
    next_cursor: Option<String>,
    resume: Option<ScreenChangeResume>,
    analysed: Vec<TimeRange>,
    gaps: Vec<VisualCoverageGap>,
    stop: Option<ScreenChangeStop>,
    crops: Option<CropSummary>,
}

impl ScreenChangesResults {
    /// Session state observed by the call.
    #[must_use]
    pub const fn session(&self) -> &SessionSnapshot {
        &self.session
    }

    /// The index revision the changes were read from.
    #[must_use]
    pub const fn index(&self) -> &VisualIndex {
        &self.index
    }

    /// The requested range.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        self.range
    }

    /// The range clipped to the video.
    #[must_use]
    pub const fn searched(&self) -> TimeRange {
        self.searched
    }

    /// The page's changes, in time order.
    #[must_use]
    pub fn changes(&self) -> &[ScreenChangeItem] {
        &self.changes
    }

    /// Each distinct frame the changes cite, once, in order of first use.
    #[must_use]
    pub fn frames(&self) -> &[EvidenceItem] {
        &self.frames
    }

    /// One verified file per frame, in the same order.
    #[must_use]
    pub fn files(&self) -> &[EvidenceFile] {
        &self.files
    }

    /// Token for the next page of this session, when more changes of the
    /// analysed part follow and the page was not stopped.
    #[must_use]
    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }

    /// Where a fresh call continues, when this one could not go on.
    #[must_use]
    pub const fn resume(&self) -> Option<ScreenChangeResume> {
        self.resume
    }

    /// The analysed parts of the range.
    #[must_use]
    pub fn analysed(&self) -> &[TimeRange] {
        &self.analysed
    }

    /// The index's gaps in the range.
    #[must_use]
    pub fn gaps(&self) -> &[VisualCoverageGap] {
        &self.gaps
    }

    /// Why the page stopped before its last change, if it did.
    #[must_use]
    pub const fn stop(&self) -> Option<ScreenChangeStop> {
        self.stop
    }

    /// What the call's cropping made, when crops were asked for.
    #[must_use]
    pub const fn crops(&self) -> Option<CropSummary> {
        self.crops
    }
}

impl Engine {
    /// Pages the screen changes of a source range, with each change's two
    /// exact frames and where they differ (see the module documentation for
    /// the order).
    ///
    /// # Errors
    ///
    /// The failures of [`Engine::candidates`] for the range, a rejected
    /// cursor ([`EngineError::ScreenChangeQuery`]), the failures of
    /// [`Engine::frame_get`] for a frame, [`EngineError::EvidenceBudgetExhausted`]
    /// when not even the first change's frames fit the session, and a stored
    /// frame whose bytes no longer match or decode.
    pub async fn screen_changes(
        &self,
        request: ScreenChangesRequest,
    ) -> Result<ScreenChangesResults, EngineError> {
        let session = request.session.clone();
        self.page_screen_changes(request)
            .await
            .map_err(|error| self.name_running_transcription(&session, error))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "The stage order is the contract; keep it visible in one place"
    )]
    async fn page_screen_changes(
        &self,
        request: ScreenChangesRequest,
    ) -> Result<ScreenChangesResults, EngineError> {
        let requested = TimeRange::new(
            MediaTime::from_micros(request.range.from_micros),
            MediaTime::from_micros(request.range.to_micros),
        )
        .map_err(|_| EngineError::InvalidTimeRange)?;
        let limit =
            screen_change_page_limit(request.limit).map_err(|_| EngineError::InvalidPageLimit)?;
        let indexed = self
            .indexed_range(
                &request.session,
                requested,
                request.cursor.is_some(),
                &request.cancellation,
            )
            .await?;
        let index = indexed.index;
        let searched = clip_to_source(requested, index.duration())
            .map_err(|_| EngineError::RangeOutsideSource)?;
        let now_micros = indexed.now.saturating_mul(MICROS_PER_SECOND);
        let page = page_screen_changes(
            &request.session,
            &index,
            &ScreenChangePageRequest {
                range: requested,
                limit,
                cursor: request.cursor.clone(),
                crops: request.crops,
            },
            now_micros,
        )
        .map_err(EngineError::ScreenChangeQuery)?;
        let port = EngineFrames {
            engine: self,
            session: &request.session,
            cancellation: &request.cancellation,
            deadline: Instant::now() + FRAMES_DEADLINE,
            taken: Mutex::new(Vec::new()),
        };
        let mut compared = compare_screen_changes(&port, &page.changes).await?;
        if compared.changes.is_empty() && !page.changes.is_empty() {
            match compared.stop {
                Some(ScreenChangeStop::SessionEvidenceBudget) => {
                    return Err(EngineError::EvidenceBudgetExhausted);
                }
                Some(ScreenChangeStop::DeadlineExceeded) if indexed.analysed_now == 0 => {
                    return Err(EngineError::EvidenceMedia(EvidenceMediaError::Deadline));
                }
                Some(ScreenChangeStop::Cancelled) if indexed.analysed_now == 0 => {
                    return Err(EngineError::EvidenceMedia(EvidenceMediaError::Cancelled));
                }
                _ => {}
            }
        }
        // Crops come only now, after every listed change has its frames, so
        // a crop never costs a change its place on the page.
        let crops = match request.crops {
            ScreenChangeCrops::NoCrops => None,
            ScreenChangeCrops::LargestRegions => {
                Some(crop_screen_changes(&port, &mut compared.changes).await?)
            }
        };
        let gaps = visual_coverage_gaps(Some(&index), index.duration(), requested, indexed.stop);
        let expires_at = indexed
            .status
            .lifetime()
            .expires_at_unix_seconds()
            .saturating_mul(MICROS_PER_SECOND);
        let (next_cursor, resume) = match page_continuation(&page, &compared, &gaps) {
            PageContinuation::Next(last) => (
                Some(
                    page.cursor_after(&request.session, last, expires_at)
                        .map_err(|error| {
                            EngineError::ScreenChangeQuery(ScreenChangeQueryError::Cursor(error))
                        })?
                        .encode(),
                ),
                None,
            ),
            PageContinuation::Resume(resume) => (None, Some(resume)),
            PageContinuation::Done => (None, None),
        };
        let changes = compared
            .changes
            .into_iter()
            .map(|change| {
                screen_change_id(&request.session, &index, &change.change)
                    .map(|id| ScreenChangeItem {
                        id,
                        compared: change,
                    })
                    .map_err(|_| EngineError::EvidenceAssembly)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (frames, files) = port.into_frames();
        let analysed = analysed_ranges(&index, requested);
        Ok(ScreenChangesResults {
            session: SessionSnapshot::observe(&indexed.status, indexed.now),
            index,
            range: requested,
            searched,
            changes,
            frames,
            files,
            next_cursor,
            resume,
            analysed,
            gaps,
            stop: compared.stop,
            crops,
        })
    }
}

/// The engine's frames for one call: `frame get`, then the stored image.
struct EngineFrames<'a> {
    engine: &'a Engine,
    session: &'a SessionId,
    cancellation: &'a Cancellation,
    deadline: Instant,
    /// Each frame taken, once, in order of first use.
    taken: Mutex<Vec<(EvidenceItem, EvidenceFile)>>,
}

/// The refusal an evidence call's failure is for this call: no room, a stop,
/// or a failure of the call.
fn refusal(error: EngineError) -> ScreenChangeFrameRefusal<EngineError> {
    match error {
        EngineError::EvidenceBudgetExhausted => ScreenChangeFrameRefusal::NoRoom,
        EngineError::EvidenceMedia(EvidenceMediaError::Deadline) => {
            ScreenChangeFrameRefusal::Stopped(ScreenChangeStop::DeadlineExceeded)
        }
        EngineError::EvidenceMedia(EvidenceMediaError::Cancelled) => {
            ScreenChangeFrameRefusal::Stopped(ScreenChangeStop::Cancelled)
        }
        error => ScreenChangeFrameRefusal::Failed(error),
    }
}

impl EngineFrames<'_> {
    async fn take(
        &self,
        target: FrameTarget,
    ) -> Result<TakenFrame, ScreenChangeFrameRefusal<EngineError>> {
        let results = self
            .engine
            .frame_get(FrameGetRequest {
                session: self.session.clone(),
                target,
                cancellation: self.cancellation.clone(),
            })
            .await
            .map_err(refusal)?;
        let mismatch = || ScreenChangeFrameRefusal::Failed(EngineError::EvidenceAssembly);
        let item = self
            .keep(&results)
            .map_err(ScreenChangeFrameRefusal::Failed)?;
        let dimensions = item.subject().image_dimensions().ok_or_else(mismatch)?;
        Ok(TakenFrame {
            evidence_id: item.id().clone(),
            time: item.subject().actual_time(),
            dimensions,
        })
    }

    /// Keeps the one item of an evidence call and its file, once, in order of
    /// first use, and returns the item.
    fn keep(&self, results: &EvidenceResults) -> Result<EvidenceItem, EngineError> {
        let item = results
            .record()
            .items()
            .first()
            .ok_or(EngineError::EvidenceAssembly)?;
        let file = results
            .files()
            .iter()
            .find(|file| file.evidence_id() == item.id())
            .ok_or(EngineError::EvidenceAssembly)?;
        let mut taken = self
            .taken
            .lock()
            .map_err(|_| EngineError::EvidenceAssembly)?;
        if !taken.iter().any(|(known, _)| known.id() == item.id()) {
            taken.push((item.clone(), file.clone()));
        }
        Ok(item.clone())
    }

    fn into_frames(self) -> (Vec<EvidenceItem>, Vec<EvidenceFile>) {
        self.taken
            .into_inner()
            .unwrap_or_default()
            .into_iter()
            .unzip()
    }

    fn file(&self, id: &EvidenceId) -> Result<EvidenceFile, EngineError> {
        self.taken
            .lock()
            .map_err(|_| EngineError::EvidenceAssembly)?
            .iter()
            .find(|(item, _)| item.id() == id)
            .map(|(_, file)| file.clone())
            .ok_or(EngineError::EvidenceAssembly)
    }
}

impl ScreenChangeFrames for EngineFrames<'_> {
    type Error = EngineError;

    fn stop(&self) -> Option<ScreenChangeStop> {
        if self.cancellation.is_cancelled() {
            Some(ScreenChangeStop::Cancelled)
        } else if Instant::now() >= self.deadline {
            Some(ScreenChangeStop::DeadlineExceeded)
        } else {
            None
        }
    }

    async fn before_frame(
        &self,
        at: MediaTime,
    ) -> Result<TakenFrame, ScreenChangeFrameRefusal<EngineError>> {
        // A sampled frame's own time, at or after with no tolerance: the
        // frame the index compared, as `frame get --at <t> --tolerance-us 0`
        // returns it.
        self.take(FrameTarget::At {
            at_micros: at.as_micros(),
            selection: FrameSelection::AtOrAfter,
            tolerance_micros: Some(0),
        })
        .await
    }

    async fn after_frame(
        &self,
        candidate: &VisualCandidateId,
    ) -> Result<TakenFrame, ScreenChangeFrameRefusal<EngineError>> {
        self.take(FrameTarget::Candidate(candidate.clone())).await
    }

    async fn crop(
        &self,
        frame: &TakenFrame,
        rect: CropRect,
    ) -> Result<EvidenceId, ScreenChangeFrameRefusal<EngineError>> {
        // `crop` itself: the parent is the frame's evidence item and the
        // rectangle the region's, in the same displayed pixels, so the crop
        // is the source's own pixels and an ordinary crop record.
        let results = self
            .engine
            .crop(CropEvidenceRequest {
                session: self.session.clone(),
                parent: frame.evidence_id.clone(),
                rect: CropRectangle {
                    x: rect.x(),
                    y: rect.y(),
                    width: rect.width(),
                    height: rect.height(),
                },
                cancellation: self.cancellation.clone(),
            })
            .await
            .map_err(refusal)?;
        self.keep(&results)
            .map(|item| item.id().clone())
            .map_err(ScreenChangeFrameRefusal::Failed)
    }

    fn pixels(&self, frame: &TakenFrame) -> Result<Vec<u8>, EngineError> {
        let file = self.file(&frame.evidence_id)?;
        read_png_pixels(file.path(), file.sha256().as_str(), frame.dimensions).map_err(|error| {
            match error {
                PngPixelsError::TooLarge => {
                    EngineError::EvidenceMedia(EvidenceMediaError::ResourceLimit)
                }
                PngPixelsError::Invalid | PngPixelsError::Changed => {
                    EngineError::Storage(SessionStorageError::IntegrityFailure)
                }
            }
        })
    }
}

impl From<ScreenChangeError> for EngineError {
    fn from(_: ScreenChangeError) -> Self {
        Self::Storage(SessionStorageError::IntegrityFailure)
    }
}
