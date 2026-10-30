//! The data of `ocr`, the on-screen text part of `search`, and the fixed prose
//! of their failures (ADR 0030).
//!
//! On-screen text is derived, untrusted evidence: every line is published with
//! the exact frame it was read from (stream, timestamp, time base, time), its
//! region in that frame's displayed pixels, the engine's own uncalibrated
//! confidence, and the engine build that read it. Text is presented twice, as
//! read (`text`) and with hidden characters made visible (`display_text`).
//!
//! **Coverage is never hidden.** An `ocr` result counts the range's planned
//! read targets (its candidates and one frame a period) read, failed,
//! unresolved and still pending, and is `partial` while any is pending or
//! failed, or while the range's candidates are not all known. A `search` first
//! page carries the same counts in its `screen_text` member, whose `state` is
//! `not_run` until `ocr` has read the session.

use serde::Serialize;
use vsift_application::{ScreenTextStop, SessionStorageError};
use vsift_domain::{
    FrameRef, MediaTime, ScreenTextCoverage, ScreenTextIndex, ScreenTextLine, SearchMatch,
    SearchQuery, SessionId, TimeRange, VisualCandidateId, VisualCoverageGap,
};

use crate::{
    CommandName, ConfidenceResponse, CoverageResponse, LifecycleResponse, OperationResponse,
    search::SearchQueryData, text::render_hidden_characters, transcript::RangeData,
};

/// Most candidate identities one hit lists: a frame is rarely the
/// representative of more than one or two candidates, and the list is a cross
/// reference, not coverage.
const MAX_HIT_CANDIDATES: usize = 32;

/// The `scope` of every screen-text coverage block.
pub const SCREEN_TEXT_SCOPE: &str = "screen_text";

/// Envelope coverage reason: planned frames of the range are not read yet.
pub const FRAMES_PENDING_REASON: &str = "frames_pending";

/// Envelope coverage reason: reads of the range wait for their visual window to
/// be analysed (a call that may not analyse windows, ADR 0033).
pub const FRAMES_AWAITING_ANALYSIS_REASON: &str = "frames_awaiting_analysis";

/// Envelope coverage reason: planned frames of the range could not be read.
pub const FRAMES_FAILED_REASON: &str = "frames_failed";

/// Fixed warning of an `ocr` result whose range is not wholly read.
pub const SCREEN_TEXT_PARTIAL_WARNING: &str = "Not every planned frame of the range has been read: coverage counts the frames read, failed and still pending, and says whether the range's candidates are all known. Run ocr again for the same range to continue; it reads only what is pending.";

/// Remediation when no OCR runner is given, configured or found.
pub const OCR_RUNNER_REMEDIATION: &str = "On-screen text is read by the OCR runner, vsift-ocr-runner, with its ONNX Runtime library and models beside it; it is optional and nothing else needs it. Nothing was changed. Install it, or register a working one with setup configure ocr-runner --executable <path>, then run ocr again. No need to search folders: ask the user where it is.";

/// Remediation when the OCR runner could not run, load its models or answer.
pub const OCR_FAILED_REMEDIATION: &str = "The OCR runner could not read the frames: it did not start, could not load its runtime or models, did not answer in its protocol, or its files changed during the call. Nothing was committed. Check that its onnxruntime folder and models sit beside it, or register another build with setup configure ocr-runner --executable <path>; after an update, run ocr again: the new build starts its own index.";

/// Remediation for an `ocr` call that found no room: the session's index, or its
/// budget of index revisions, is full.
pub const OCR_INDEX_FULL_REMEDIATION: &str = "The session's screen-text index has no room for more frames (it holds at most 4,096 frames and 200,000 lines, in at most 64 revisions), so nothing was read or committed. Search what it holds with ocr search or search; to read more of the video, open a new session for the rest of it, or read a narrower range with a longer --period-us there.";

/// Everything a host presents about one `ocr` call.
#[derive(Clone, Debug)]
pub struct ScreenTextPresentation<'a> {
    /// The session.
    pub session_id: &'a SessionId,
    /// The newest index after the call, if any frame has been read.
    pub index: Option<&'a ScreenTextIndex>,
    /// The range read, clipped to the video.
    pub range: TimeRange,
    /// The period of the frames read between candidates.
    pub period_micros: u64,
    /// How much of the range's plan the index has read.
    pub coverage: ScreenTextCoverage,
    /// Whether every visual window of the range is analysed.
    pub candidates_complete: bool,
    /// Frames this call read or found unreadable.
    pub read_now: usize,
    /// Why the call stopped early, if it did.
    pub stop: Option<ScreenTextStop>,
    /// The visual index's gaps in the range.
    pub visual_gaps: &'a [VisualCoverageGap],
}

/// The engine build that read an index, as published.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct EngineData {
    profile: &'static str,
    runner_version: String,
    runner_sha256: String,
    runtime_version: String,
    detector_sha256: String,
    recognizer_sha256: String,
}

/// One revision of a session's screen-text index, as published.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct IndexData {
    index_id: String,
    revision: u32,
    engine: EngineData,
    frames: usize,
    lines: usize,
}

impl IndexData {
    pub(crate) fn new(index: &ScreenTextIndex) -> Self {
        let scope = index.scope();
        Self {
            index_id: index.id().as_str().to_owned(),
            revision: index.revision().get(),
            engine: EngineData {
                profile: scope.profile.identifier(),
                runner_version: scope.provider.runner_version.as_str().to_owned(),
                runner_sha256: scope.provider.runner_sha256.as_str().to_owned(),
                runtime_version: scope.provider.runtime_version.as_str().to_owned(),
                detector_sha256: scope.provider.detector_sha256.as_str().to_owned(),
                recognizer_sha256: scope.provider.recognizer_sha256.as_str().to_owned(),
            },
            frames: index.frames().len(),
            lines: index.frames().iter().map(|frame| frame.lines().len()).sum(),
        }
    }
}

/// How much of a range's plan an index has read.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct CoverageData {
    scope: &'static str,
    period_us: Option<u64>,
    planned: usize,
    read: usize,
    failed: usize,
    unresolved: usize,
    pending: usize,
    awaiting_analysis: usize,
    candidates_complete: bool,
}

impl CoverageData {
    pub(crate) const fn new(
        coverage: ScreenTextCoverage,
        period_micros: Option<u64>,
        candidates_complete: bool,
    ) -> Self {
        Self {
            scope: SCREEN_TEXT_SCOPE,
            period_us: period_micros,
            planned: coverage.planned,
            read: coverage.read,
            failed: coverage.failed,
            unresolved: coverage.unresolved,
            pending: coverage.pending,
            awaiting_analysis: coverage.awaiting_analysis,
            candidates_complete,
        }
    }

    const fn is_complete(self) -> bool {
        self.pending == 0
            && self.failed == 0
            && self.awaiting_analysis == 0
            && self.candidates_complete
    }
}

/// A visual-index gap of the range, as `candidates` reports it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct GapData {
    from_us: u64,
    to_us: u64,
    reason: &'static str,
    #[serde(skip)]
    retryable: bool,
}

/// Data of a complete or partial `ocr` result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScreenTextData {
    session_id: String,
    range: RangeData,
    index: Option<IndexData>,
    read_now: usize,
    stopped: Option<&'static str>,
    coverage: CoverageData,
    candidate_gaps: Vec<GapData>,
}

impl ScreenTextData {
    /// Presents one `ocr` call.
    #[must_use]
    pub fn new(call: &ScreenTextPresentation<'_>) -> Self {
        Self {
            session_id: call.session_id.as_str().to_owned(),
            range: RangeData::new(call.range),
            index: call.index.map(IndexData::new),
            read_now: call.read_now,
            stopped: call.stop.map(ScreenTextStop::identifier),
            coverage: CoverageData::new(
                call.coverage,
                Some(call.period_micros),
                call.candidates_complete,
            ),
            candidate_gaps: call
                .visual_gaps
                .iter()
                .take(crate::MAX_COVERAGE_RANGES)
                .map(|gap| GapData {
                    from_us: gap.range.start().as_micros(),
                    to_us: gap.range.end().as_micros(),
                    reason: gap.reason.identifier(),
                    retryable: gap.reason.is_retryable(),
                })
                .collect(),
        }
    }
}

/// The complete `ocr` result: `partial`, with a warning, while the range is
/// not wholly read.
///
/// # Errors
///
/// Returns the serialization error when the data cannot be represented as JSON.
pub fn screen_text_response(
    call: &ScreenTextPresentation<'_>,
    lifecycle: LifecycleResponse,
) -> Result<OperationResponse<serde_json::Value>, serde_json::Error> {
    let data = ScreenTextData::new(call);
    let complete = data.coverage.is_complete();
    let response = OperationResponse::complete(CommandName::Ocr.identifier(), &data)?
        .with_lifecycle(lifecycle)
        .with_coverage(envelope_coverage(&data));
    Ok(if complete {
        response
    } else {
        response.with_warnings(&[SCREEN_TEXT_PARTIAL_WARNING])
    })
}

/// The envelope coverage of an `ocr` result: truncated while the range is not
/// wholly read; its gaps are then the visual index's gaps that a later
/// `candidates` call can still fill, and its reasons name what is missing:
/// `frames_pending`, `frames_failed`, then each of those gaps' reasons once. A
/// complete result has none, whatever final gaps `candidate_gaps` lists.
fn envelope_coverage(data: &ScreenTextData) -> CoverageResponse {
    if data.coverage.is_complete() {
        return CoverageResponse::new(false, Vec::new(), Vec::new());
    }
    let mut reasons = Vec::new();
    if data.coverage.pending > 0 {
        reasons.push(FRAMES_PENDING_REASON.to_owned());
    }
    if data.coverage.failed > 0 {
        reasons.push(FRAMES_FAILED_REASON.to_owned());
    }
    if data.coverage.awaiting_analysis > 0 {
        reasons.push(FRAMES_AWAITING_ANALYSIS_REASON.to_owned());
    }
    let open: Vec<&GapData> = data
        .candidate_gaps
        .iter()
        .filter(|gap| gap.retryable)
        .collect();
    for gap in &open {
        if !reasons.iter().any(|reason| reason == gap.reason) {
            reasons.push(gap.reason.to_owned());
        }
    }
    CoverageResponse::new(
        true,
        open.iter()
            .map(|gap| format!("{}-{}", gap.from_us, gap.to_us))
            .collect(),
        reasons,
    )
}

/// One screen-text hit as a host presents it, borrowed from the engine's result.
#[derive(Clone, Debug)]
pub struct ScreenTextHitPresentation<'a> {
    /// The line as read on the first frame.
    pub line: &'a ScreenTextLine,
    /// The first frame.
    pub frame: &'a FrameRef,
    /// The candidates whose representative frame that is.
    pub candidates: &'a [VisualCandidateId],
    /// The last consecutive read frame's time that held the same text there.
    pub last_time: MediaTime,
    /// How many consecutive read frames held it.
    pub frames: u32,
    /// How it matched.
    pub tier: SearchMatch,
}

/// The on-screen text part of a `search` first page, as a host presents it.
#[derive(Clone, Debug)]
pub struct ScreenTextSearchPresentation<'a> {
    /// The index searched, if `ocr` has read the session.
    pub index: Option<&'a ScreenTextIndex>,
    /// Hits in rank order, at most the page size.
    pub hits: Vec<ScreenTextHitPresentation<'a>>,
    /// How many hits there are in all.
    pub total_hits: usize,
    /// How much of the searched range's plan the index has read.
    pub coverage: ScreenTextCoverage,
    /// Whether every visual window of the range is analysed.
    pub candidates_complete: bool,
    /// The period of the index's latest plan.
    pub period_micros: Option<u64>,
    /// An index that could not be read, if one could not; `search` only.
    pub unavailable: Option<ScreenTextUnavailable>,
}

/// A region of a frame's displayed pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct RegionData {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// The exact frame a line was read from, in `frame_evidence`'s terms.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct FrameData {
    stream_index: u32,
    time_base_numerator: u32,
    time_base_denominator: u32,
    pts: i64,
    time_us: u64,
    width: u32,
    height: u32,
}

/// One occurrence of the query in on-screen text.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScreenTextHitData {
    line_id: String,
    #[serde(rename = "match")]
    tier: &'static str,
    text: String,
    display_text: String,
    confidence: ConfidenceResponse,
    frame: FrameData,
    region: RegionData,
    last_time_us: u64,
    frames: u32,
    candidate_ids: Vec<String>,
}

impl ScreenTextHitData {
    /// Presents one occurrence.
    #[must_use]
    pub fn new(hit: &ScreenTextHitPresentation<'_>) -> Self {
        let text = hit.line.text.as_str().to_owned();
        Self {
            line_id: hit.line.id.as_str().to_owned(),
            tier: hit.tier.identifier(),
            display_text: render_hidden_characters(&text),
            text,
            confidence: ConfidenceResponse::from(hit.line.confidence),
            frame: FrameData {
                stream_index: hit.frame.stream_index,
                time_base_numerator: hit.frame.time_base.numerator(),
                time_base_denominator: hit.frame.time_base.denominator(),
                pts: hit.frame.pts,
                time_us: hit.frame.time.as_micros(),
                width: hit.frame.dimensions.width(),
                height: hit.frame.dimensions.height(),
            },
            region: RegionData {
                x: hit.line.region.x(),
                y: hit.line.region.y(),
                width: hit.line.region.width(),
                height: hit.line.region.height(),
            },
            last_time_us: hit.last_time.as_micros(),
            frames: hit.frames,
            candidate_ids: hit
                .candidates
                .iter()
                .take(MAX_HIT_CANDIDATES)
                .map(|id| id.as_str().to_owned())
                .collect(),
        }
    }
}

/// An index a `search` first page could not read for its `screen_text` member.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnreadableIndex {
    /// The screen-text index (`ocr`'s record): nothing about the screen is known.
    ScreenText,
    /// The visual index: hits are listed, but the plan, and so the coverage, is not known.
    Visual,
}

impl UnreadableIndex {
    /// Stable identifier, as the member names it.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::ScreenText => "screen_text",
            Self::Visual => "visual",
        }
    }
}

/// Why an index could not be read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnreadableReason {
    /// Its record failed verification.
    IntegrityFailure,
    /// Its record was written by a newer release.
    UnsupportedVersion,
    /// It could not be read for another reason (storage, or the session's state).
    Unreadable,
}

impl UnreadableReason {
    /// The reason for a session-store failure.
    #[must_use]
    pub const fn of(error: &SessionStorageError) -> Self {
        match error {
            SessionStorageError::IntegrityFailure => Self::IntegrityFailure,
            SessionStorageError::UnsupportedVersion => Self::UnsupportedVersion,
            _ => Self::Unreadable,
        }
    }

    /// Stable identifier, as the member names it.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::IntegrityFailure => "integrity_failure",
            Self::UnsupportedVersion => "unsupported_version",
            Self::Unreadable => "unreadable",
        }
    }
}

/// An index the `screen_text` member of a `search` first page could not read,
/// and why. The search itself is unchanged: its status, coverage and transcript
/// hits never depend on on-screen text (ADR 0030 decision 10, RV-2 finding 2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenTextUnavailable {
    /// Which index.
    pub index: UnreadableIndex,
    /// Why.
    pub reason: UnreadableReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct UnavailableData {
    index: &'static str,
    reason: &'static str,
}

/// The `screen_text` member of a `search` first page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScreenTextSearchData {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    unavailable: Option<UnavailableData>,
    index: Option<IndexData>,
    hits: Vec<ScreenTextHitData>,
    total_hits: usize,
    coverage: CoverageData,
}

impl ScreenTextSearchData {
    /// Presents the on-screen text part of a first page.
    #[must_use]
    pub fn new(part: &ScreenTextSearchPresentation<'_>) -> Self {
        let coverage =
            CoverageData::new(part.coverage, part.period_micros, part.candidates_complete);
        let state = match (part.unavailable, part.index) {
            (
                Some(ScreenTextUnavailable {
                    index: UnreadableIndex::ScreenText,
                    ..
                }),
                _,
            ) => "unavailable",
            (_, None) => "not_run",
            (_, Some(_)) if coverage.is_complete() => "complete",
            (_, Some(_)) => "partial",
        };
        Self {
            state,
            unavailable: part.unavailable.map(|unavailable| UnavailableData {
                index: unavailable.index.identifier(),
                reason: unavailable.reason.identifier(),
            }),
            index: part.index.map(IndexData::new),
            hits: part.hits.iter().map(ScreenTextHitData::new).collect(),
            total_hits: part.total_hits,
            coverage,
        }
    }
}

/// Fixed warning of an `ocr search` page whose range is not wholly read.
pub const OCR_SEARCH_PARTIAL_WARNING: &str = "Not every planned frame of the searched range has been read, so text shown there may not be found: coverage counts the frames read, failed and still pending, and says whether the range's candidates are all known (state not_run: ocr has not read this session). Run ocr for the range, then search again.";

/// Remediation for an `ocr search` cursor that cannot continue.
pub const OCR_SEARCH_CURSOR_REMEDIATION: &str = "The cursor cannot continue this on-screen search: it belongs to another session, query or range, it expired, or ocr has read more frames since (each ocr call commits a new index revision). Run ocr search again without --cursor and pass back only the next_cursor it returns.";

/// Envelope coverage reason: `ocr` has not read the session.
pub const NOT_RUN_REASON: &str = "not_run";

/// Envelope coverage reason: a visual window of the range is not analysed, so
/// its candidates are not in the plan.
pub const CANDIDATES_INCOMPLETE_REASON: &str = "candidates_incomplete";

/// One `ocr search` page as a host presents it (ADR 0030 decision 13).
#[derive(Clone, Debug)]
pub struct ScreenTextSearchPagePresentation<'a> {
    /// The session searched.
    pub session_id: &'a SessionId,
    /// The validated query.
    pub query: &'a SearchQuery,
    /// The requested range, if any.
    pub range: Option<TimeRange>,
    /// The hits, the total, the index and the coverage.
    pub part: ScreenTextSearchPresentation<'a>,
    /// The encoded continuation token, absent on the last page.
    pub next_cursor: Option<&'a str>,
}

/// Data of a complete or partial `ocr search` page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScreenTextSearchPageData {
    session_id: String,
    query: SearchQueryData,
    range: Option<RangeData>,
    state: &'static str,
    index: Option<IndexData>,
    hits: Vec<ScreenTextHitData>,
    total_hits: usize,
    next_cursor: Option<String>,
    coverage: CoverageData,
}

impl ScreenTextSearchPageData {
    /// Presents one page.
    #[must_use]
    pub fn new(page: &ScreenTextSearchPagePresentation<'_>) -> Self {
        let part = ScreenTextSearchData::new(&page.part);
        Self {
            session_id: page.session_id.as_str().to_owned(),
            query: SearchQueryData::new(page.query),
            range: page.range.map(RangeData::new),
            state: part.state,
            index: part.index,
            hits: part.hits,
            total_hits: part.total_hits,
            next_cursor: page.next_cursor.map(str::to_owned),
            coverage: part.coverage,
        }
    }
}

/// The complete `ocr search` result: `partial`, with a warning and truncated
/// envelope coverage, while its state is not `complete`.
///
/// # Errors
///
/// Returns the serialization error when the data cannot be represented as JSON.
pub fn screen_text_search_response(
    page: &ScreenTextSearchPagePresentation<'_>,
    lifecycle: LifecycleResponse,
) -> Result<OperationResponse<serde_json::Value>, serde_json::Error> {
    let data = ScreenTextSearchPageData::new(page);
    let mut reasons = Vec::new();
    if data.index.is_none() {
        reasons.push(NOT_RUN_REASON.to_owned());
    } else {
        if data.coverage.pending > 0 {
            reasons.push(FRAMES_PENDING_REASON.to_owned());
        }
        if data.coverage.failed > 0 {
            reasons.push(FRAMES_FAILED_REASON.to_owned());
        }
        if data.coverage.awaiting_analysis > 0 {
            reasons.push(FRAMES_AWAITING_ANALYSIS_REASON.to_owned());
        }
        if !data.coverage.candidates_complete {
            reasons.push(CANDIDATES_INCOMPLETE_REASON.to_owned());
        }
    }
    let complete = data.state == "complete";
    let response = OperationResponse::complete(CommandName::OcrSearch.identifier(), &data)?
        .with_lifecycle(lifecycle)
        .with_coverage(CoverageResponse::new(!complete, Vec::new(), reasons));
    Ok(if complete {
        response
    } else {
        response.with_warnings(&[OCR_SEARCH_PARTIAL_WARNING])
    })
}
