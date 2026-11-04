//! The data of `changes`, its JSON Lines evidence stream, its envelope
//! coverage and the fixed prose for its partial results and failures (ADR
//! 0034).
//!
//! A page lists the screen changes whose after frame lies in the requested
//! range. Each change is published as a `screen_change` record: its
//! identity, what kind of change the visual index recorded, the index's
//! change window, the exact frame before and after it (ordinary
//! `frame_evidence`, cited by `evidence_id`) and the rectangles where the two
//! frames differ under the profile `screen-change-regions-v1`. The page also
//! lists each frame its changes cite once, as `frame get` publishes it, and
//! one delivered file per frame.
//!
//! Coverage is the visual index's, reported as `candidates` reports it. A
//! page that stopped before its last change (the session's evidence room,
//! the deadline or a cancellation) has no `next_cursor`; it names the stop in
//! `partial_reason` and where a fresh call continues in `resume`. A result
//! with any gap or a stop is `partial`, with a fixed warning, and exits 0.
//!
//! A change is cited by its before and after `evidence_id`s, never by its
//! `change_id`, which `handoff check` does not know (ADR 0034 section 5).

use serde::Serialize;
use vsift_application::{ComparedChange, ScreenChangeStop};
use vsift_domain::{
    ChangeRegion, EvidenceItem, EvidenceProfile, FrameComparison, MAX_CHANGE_REGIONS, MediaTime,
    SCREEN_CHANGE_JOIN_DISTANCE, SCREEN_CHANGE_PIXEL_LEVEL, SCREEN_CHANGE_TILE,
    SCREEN_CHANGE_TILE_PIXELS, ScreenChangeId, ScreenChangeKind, ScreenChangeProfile, SessionId,
    TimeAnchors, TimeRange, VisualCoverageGap, VisualIndex,
};

use crate::{
    ClockTimeData, CommandName, DeliveredEvidenceFile, EvidenceEventResponse,
    EvidencePresentationError, FrameEvidenceData, LifecycleResponse, OperationResponse,
    TerminalEventResponse,
    candidates::{IndexData, VisualCoverageData, envelope_coverage_with_stop},
    navigation::FileData,
    stream::EvidenceStream,
    transcript::RangeData,
};

/// Fixed warning of a `changes` result whose range has gaps.
pub const CHANGES_COVERAGE_WARNING: &str = "Part of the requested range has no screen changes listed because it is not analysed yet, could not be analysed, or changed more often than the index keeps; coverage lists each gap and its reason. Run changes again without --cursor (from resume.from_us when it is set) to continue not_analyzed and deadline_exceeded parts.";

/// Fixed warning of a `changes` page that stopped before its last change.
pub const CHANGES_STOPPED_WARNING: &str = "The page stopped before its last change (partial_reason says why): the changes listed are complete and their frames are committed evidence, but there is no next_cursor. resume.from_us is where a fresh call without --cursor continues; after session_evidence_budget, keep this session with session retain and continue in a new session opened with ingest.";

/// Remediation when a `changes` cursor is rejected.
pub const CHANGES_CURSOR_REMEDIATION: &str = "The cursor does not continue a page of this changes request: it belongs to another session, range or --limit, it has expired, or a window of its range was analysed after it was issued. Nothing was changed. Pass back only the next_cursor of the previous page with the same session, --from, --to and --limit, or run changes without --cursor.";

/// Remediation when a `changes` cursor is given for a session without a
/// visual index.
pub const CHANGES_CURSOR_WITHOUT_INDEX_REMEDIATION: &str = "This session has no screen changes yet, so the cursor cannot continue any page of it. Nothing was changed. Run changes without --cursor first; pass back only the next_cursor it returns, with the same session, range and --limit.";

/// One listed change, as the engine names it.
#[derive(Clone, Copy, Debug)]
pub struct ListedScreenChange<'a> {
    /// The change's identity.
    pub id: &'a ScreenChangeId,
    /// The change, its two frames and where they differ.
    pub compared: &'a ComparedChange,
}

/// Where a fresh call continues when a page could not answer its range to
/// the end (ADR 0034 section 6a).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenChangeResumePresentation {
    /// The `--from` of the fresh call.
    pub from: MediaTime,
    /// The published identifier of why this call did not go on.
    pub reason: &'static str,
}

/// Everything a host presents about one page of screen changes.
///
/// The fields borrow the engine's typed result, so every host presents the
/// same page identically.
#[derive(Clone, Copy, Debug)]
pub struct ScreenChangesPresentation<'a> {
    /// Session that was read.
    pub session_id: &'a SessionId,
    /// The index revision the changes were read from.
    pub index: &'a VisualIndex,
    /// The requested range.
    pub range: TimeRange,
    /// The requested range clipped to the video.
    pub searched: TimeRange,
    /// The page's changes, in time order.
    pub changes: &'a [ListedScreenChange<'a>],
    /// Each distinct frame the changes cite, once, in order of first use.
    pub frames: &'a [EvidenceItem],
    /// One verified file per frame, in the same order.
    pub files: &'a [DeliveredEvidenceFile<'a>],
    /// Token for the next page, when more changes of the analysed part
    /// follow and the page was not stopped.
    pub next_cursor: Option<&'a str>,
    /// Where a fresh call continues, when this one could not go on.
    pub resume: Option<ScreenChangeResumePresentation>,
    /// Parts of the searched range analysed windows cover, merged.
    pub analysed: &'a [TimeRange],
    /// Typed gaps in the visual coverage of the searched range.
    pub gaps: &'a [VisualCoverageGap],
    /// Why the page stopped before its last change, if it did.
    pub stop: Option<ScreenChangeStop>,
    /// The session's clock-time anchors, from which every frame's
    /// `clock_time` is derived (ADR 0029).
    pub time_anchors: &'a TimeAnchors,
}

/// The region rule and its thresholds (`screen-change-regions-v1`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct ProfileData {
    identifier: &'static str,
    pixel_level: u8,
    tile: u32,
    tile_pixels: u16,
    join_distance: u32,
    max_regions: usize,
}

impl ProfileData {
    const fn regions_v1() -> Self {
        Self {
            identifier: ScreenChangeProfile::RegionsV1.identifier(),
            pixel_level: SCREEN_CHANGE_PIXEL_LEVEL,
            tile: SCREEN_CHANGE_TILE,
            tile_pixels: SCREEN_CHANGE_TILE_PIXELS,
            join_distance: SCREEN_CHANGE_JOIN_DISTANCE,
            max_regions: MAX_CHANGE_REGIONS,
        }
    }
}

/// One of a change's two frames.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ChangeFrameData {
    evidence_id: String,
    time_us: u64,
    /// The frame's real-world clock time (ADR 0029), as its
    /// `frame_evidence` record states it; absent without a time anchor.
    #[serde(skip_serializing_if = "Option::is_none")]
    clock_time: Option<ClockTimeData>,
}

/// A rectangle where the two frames differ, in displayed pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct RegionData {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    changed_pixels: u64,
}

impl RegionData {
    const fn new(region: &ChangeRegion) -> Self {
        Self {
            x: region.rect.x(),
            y: region.rect.y(),
            width: region.rect.width(),
            height: region.rect.height(),
            changed_pixels: region.changed_pixels,
        }
    }
}

/// One published `screen_change` record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScreenChangeData {
    change_id: String,
    kind: &'static str,
    motion: Option<&'static str>,
    settled: Option<bool>,
    candidate_ids: Vec<String>,
    change_window: RangeData,
    before: ChangeFrameData,
    after: ChangeFrameData,
    comparison: &'static str,
    regions: Vec<RegionData>,
    regions_left_out: u32,
    changed_area: Option<RegionData>,
    no_region_at_threshold: bool,
}

impl ScreenChangeData {
    /// The change's identity, the record's upsert key.
    pub(crate) fn change_id(&self) -> &str {
        &self.change_id
    }

    /// Presents one listed change, with its frames' clock times from
    /// `anchors`.
    #[must_use]
    pub fn new(listed: &ListedScreenChange<'_>, anchors: &TimeAnchors) -> Self {
        let compared = listed.compared;
        let change = &compared.change;
        let (motion, settled) = match change.kind {
            ScreenChangeKind::Change => (None, None),
            ScreenChangeKind::Motion(part) => (Some(part.identifier()), Some(part.settled())),
        };
        let frame = |evidence_id: &vsift_domain::EvidenceId, time: MediaTime| ChangeFrameData {
            evidence_id: evidence_id.as_str().to_owned(),
            time_us: time.as_micros(),
            clock_time: ClockTimeData::point(anchors, time),
        };
        let (comparison, regions, regions_left_out, changed_area, no_region) = match &compared
            .comparison
        {
            FrameComparison::Compared(found) => (
                "compared",
                found.regions.iter().map(RegionData::new).collect(),
                found.regions_left_out,
                found.changed_area.as_ref().map(RegionData::new),
                found.regions.is_empty(),
            ),
            FrameComparison::DimensionsDiffer => ("dimensions_differ", Vec::new(), 0, None, false),
        };
        Self {
            change_id: listed.id.as_str().to_owned(),
            kind: change.kind.identifier(),
            motion,
            settled,
            candidate_ids: change
                .candidates
                .iter()
                .map(|candidate| candidate.as_str().to_owned())
                .collect(),
            // The index's own bracket: the screen changed after the sampled
            // frame at `from_us` and at or before the one at `to_us`.
            change_window: RangeData {
                from_us: change.before.as_micros(),
                to_us: change.after.as_micros(),
            },
            before: frame(&compared.before.evidence_id, compared.before.time),
            after: frame(&compared.after.evidence_id, compared.after.time),
            comparison,
            regions,
            regions_left_out,
            changed_area,
            no_region_at_threshold: no_region,
        }
    }
}

/// Where a fresh call continues.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct ResumeData {
    from_us: u64,
    reason: &'static str,
}

/// Everything a page states besides its changes and frames.
struct PageSummary {
    session_id: String,
    range: RangeData,
    index: IndexData,
    profile: ProfileData,
    coverage: VisualCoverageData,
    files: Vec<FileData>,
    partial_reason: Option<&'static str>,
    next_cursor: Option<String>,
    resume: Option<ResumeData>,
}

impl PageSummary {
    fn new(page: &ScreenChangesPresentation<'_>) -> Result<Self, EvidencePresentationError> {
        Ok(Self {
            session_id: page.session_id.as_str().to_owned(),
            range: RangeData::new(page.range),
            index: IndexData::new(page.index),
            profile: ProfileData::regions_v1(),
            coverage: VisualCoverageData::of(page.searched, page.analysed, page.gaps),
            files: files(page)?,
            partial_reason: page.stop.map(ScreenChangeStop::identifier),
            next_cursor: page.next_cursor.map(str::to_owned),
            resume: page.resume.map(|resume| ResumeData {
                from_us: resume.from.as_micros(),
                reason: resume.reason,
            }),
        })
    }
}

/// The delivered files, checked to be one per frame in frame order.
fn files(page: &ScreenChangesPresentation<'_>) -> Result<Vec<FileData>, EvidencePresentationError> {
    if page.files.len() != page.frames.len() {
        return Err(EvidencePresentationError::FileMismatch);
    }
    page.frames
        .iter()
        .zip(page.files)
        .map(|(frame, file)| {
            if file.evidence_id != frame.id() || file.kind != frame.media().kind() {
                return Err(EvidencePresentationError::FileMismatch);
            }
            FileData::of(file)
        })
        .collect()
}

/// The page's frames as `frame_evidence` records, extracted under the one
/// evidence profile `frame get` uses.
fn frames(
    page: &ScreenChangesPresentation<'_>,
) -> Result<Vec<FrameEvidenceData>, EvidencePresentationError> {
    page.frames
        .iter()
        .map(|item| {
            FrameEvidenceData::of_item(
                page.index.source_id(),
                EvidenceProfile::P09R0,
                item,
                page.time_anchors,
            )
            .ok_or(EvidencePresentationError::OperationMismatch)
        })
        .collect()
}

fn changes(page: &ScreenChangesPresentation<'_>) -> Vec<ScreenChangeData> {
    page.changes
        .iter()
        .map(|listed| ScreenChangeData::new(listed, page.time_anchors))
        .collect()
}

/// Data of a complete or partial `changes` result: one bounded page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScreenChangesData {
    session_id: String,
    range: RangeData,
    index: IndexData,
    profile: ProfileData,
    coverage: VisualCoverageData,
    changes: Vec<ScreenChangeData>,
    frames: Vec<FrameEvidenceData>,
    files: Vec<FileData>,
    partial_reason: Option<&'static str>,
    next_cursor: Option<String>,
    resume: Option<ResumeData>,
}

/// Data of the terminal event that ends a `changes` evidence stream: the
/// page without its changes and frames, which were the preceding evidence
/// events, and `record_count` saying how many events there were.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScreenChangesStreamData {
    session_id: String,
    range: RangeData,
    index: IndexData,
    profile: ProfileData,
    coverage: VisualCoverageData,
    record_count: usize,
    files: Vec<FileData>,
    partial_reason: Option<&'static str>,
    next_cursor: Option<String>,
    resume: Option<ResumeData>,
}

/// Completes a `changes` response with its lifecycle, its coverage and, for
/// a range with gaps or a page that stopped, the `partial` status and the
/// warnings.
fn finish(
    response: OperationResponse<serde_json::Value>,
    page: &ScreenChangesPresentation<'_>,
    lifecycle: LifecycleResponse,
) -> OperationResponse<serde_json::Value> {
    // The part of the range a stopped page left unlisted: from the first
    // change it did not list to the end of the searched range.
    let stopped = page.stop.map(|stop| {
        let unlisted = page
            .resume
            .and_then(|resume| TimeRange::new(resume.from, page.searched.end()).ok())
            .unwrap_or(page.searched);
        (unlisted, stop.identifier())
    });
    let mut response = response
        .with_lifecycle(lifecycle)
        .with_coverage(envelope_coverage_with_stop(page.gaps, stopped));
    if !page.gaps.is_empty() {
        response = response.with_warnings(&[CHANGES_COVERAGE_WARNING]);
    }
    if page.stop.is_some() {
        response = response.with_warnings(&[CHANGES_STOPPED_WARNING]);
    }
    response
}

/// The complete or partial `changes` result for one page.
///
/// # Errors
///
/// Returns [`EvidencePresentationError`] for files that do not match the
/// frames, a frame that is not an image, a path that is not valid UTF-8 or
/// data that cannot be represented as JSON.
pub fn screen_changes_response(
    page: &ScreenChangesPresentation<'_>,
    lifecycle: LifecycleResponse,
) -> Result<OperationResponse<serde_json::Value>, EvidencePresentationError> {
    let summary = PageSummary::new(page)?;
    let data = ScreenChangesData {
        session_id: summary.session_id,
        range: summary.range,
        index: summary.index,
        profile: summary.profile,
        coverage: summary.coverage,
        changes: changes(page),
        frames: frames(page)?,
        files: summary.files,
        partial_reason: summary.partial_reason,
        next_cursor: summary.next_cursor,
        resume: summary.resume,
    };
    let response = OperationResponse::complete(CommandName::Changes.identifier(), &data)?;
    Ok(finish(response, page, lifecycle))
}

/// One `changes` page as a JSON Lines evidence stream.
///
/// The records are the page's frames as `frame_evidence` events, in order of
/// first use, then its changes as `screen_change` events, in time order,
/// numbered from 0; the terminal event follows with the next sequence number
/// and carries everything else. A page holds at most 20 changes and 40
/// frames, so the stream is bounded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenChangesEvidenceStream {
    records: Vec<EvidenceEventResponse>,
    terminal: TerminalEventResponse,
}

impl ScreenChangesEvidenceStream {
    /// Presents one page of screen changes as an evidence stream.
    ///
    /// # Errors
    ///
    /// As [`screen_changes_response`].
    pub fn new(
        page: &ScreenChangesPresentation<'_>,
        lifecycle: LifecycleResponse,
    ) -> Result<Self, EvidencePresentationError> {
        let command = CommandName::Changes;
        let summary = PageSummary::new(page)?;
        let mut records: Vec<EvidenceEventResponse> = Vec::new();
        let next =
            |records: &[EvidenceEventResponse]| u64::try_from(records.len()).unwrap_or(u64::MAX);
        for frame in frames(page)? {
            let sequence = next(&records);
            records.push(EvidenceEventResponse::frame_evidence(
                sequence, command, frame,
            ));
        }
        for change in changes(page) {
            let sequence = next(&records);
            records.push(EvidenceEventResponse::screen_change(
                sequence, command, change,
            ));
        }
        let data = ScreenChangesStreamData {
            session_id: summary.session_id,
            range: summary.range,
            index: summary.index,
            profile: summary.profile,
            coverage: summary.coverage,
            record_count: records.len(),
            files: summary.files,
            partial_reason: summary.partial_reason,
            next_cursor: summary.next_cursor,
            resume: summary.resume,
        };
        let result = finish(
            OperationResponse::complete(command.identifier(), &data)?,
            page,
            lifecycle,
        );
        let terminal_sequence = u64::try_from(records.len()).unwrap_or(u64::MAX);
        Ok(Self {
            records,
            terminal: TerminalEventResponse::at_sequence(result, terminal_sequence),
        })
    }
}

impl EvidenceStream for ScreenChangesEvidenceStream {
    fn records(&self) -> &[EvidenceEventResponse] {
        &self.records
    }

    fn terminal(&self) -> &TerminalEventResponse {
        &self.terminal
    }
}
