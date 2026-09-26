//! The results of the evidence navigation commands (P09, ADR 0019): the
//! `frame get`, `frame neighbours`, `frame burst` and `crop` data, their
//! published `frame_evidence` record, their JSON Lines evidence stream, and
//! the fixed prose for their partial results and failures.
//!
//! An evidence call returns the lineage record the engine committed (or the
//! complete record an identical earlier request committed, `reused`), and
//! one delivered file per item. A result is built from that record alone, so
//! a reused result is byte for byte the result of the call that committed
//! it, except for `reused` itself.
//!
//! **Items and their upsert key.** Each item is published as a
//! `frame_evidence` record keyed by its `evidence_id`. An item's identity
//! digests only what fixes its pixels (ADR 0019), so two requests that
//! resolve to one frame name one item; everything request-specific (the
//! requested time, the policy, the delta, the candidate) lives in the
//! result's `request` and `selections`. The stored record also notes, per
//! copy of an item, how the committing call checked the source (D1); that
//! differs between calls that name one item, so it is published once per
//! result as `source_check` and kept out of the item, which keeps the
//! stream's promise that one key always carries one record.
//!
//! **Files (D2).** Pixels are delivered only as the absolute path of the
//! committed session artifact, in `files`, valid while the session exists.
//! A path is the one place public output names a local path; records and
//! bundles never do. A path that is not valid UTF-8 cannot be written as
//! JSON text, so it is a typed [`EvidencePresentationError`] rather than a
//! lossy string.

use std::{error::Error, fmt, path::Path};

use serde::Serialize;
use vsift_domain::{
    BurstExtent, EvidenceDetail, EvidenceId, EvidenceItem, EvidenceMediaKind, EvidenceOperation,
    EvidenceRecord, EvidenceRequest, EvidenceSelection, EvidenceSubject, FrameRef,
    FrameSelectionError, NeighbourStop, PartialReason, SourceCheck,
};

use crate::{
    CommandName, EvidenceEventResponse, LifecycleResponse, OperationResponse,
    TerminalEventResponse, stream::EvidenceStream,
};

/// Media type of every delivered image: 8-bit RGB PNG at native resolution.
const PNG_MEDIA_TYPE: &str = "image/png";

/// Remediation when the session has no room for more evidence (ADR 0019 D4).
pub const EVIDENCE_BUDGET_REMEDIATION: &str = "This session has no room for more evidence: it holds at most 160 evidence artifacts (images, audio clips and their records) within 256 artifacts and 10 GiB. Nothing was changed. Keep what it holds with session retain --output <new-directory>, then open a new session of the same video with ingest and continue there.";

/// Remediation when `FFmpeg` or `FFprobe` is missing for evidence.
pub const EVIDENCE_TOOLS_REMEDIATION: &str = "Frames, crops and audio clips are decoded from the video with FFmpeg and FFprobe; Whisper and a model are not needed. Nothing was changed. Install or locate trusted builds, register them with setup configure ffmpeg|ffprobe --executable <path>, then run setup check.";

/// Remediation for a burst range longer than sixty seconds.
pub const BURST_RANGE_REMEDIATION: &str = "A frame burst covers at most 60 s. Nothing was changed. Run candidates over the longer range to find the moments where the screen changed, then burst or frame get around them, or split the range into bursts of at most 60 s.";

/// Remediation when the video has no video stream to take frames from.
pub const NO_FRAMES_REMEDIATION: &str = "The source has no video stream, so it has no frames. Nothing was changed. Use transcript get or search for its speech instead.";

/// Remediation when a `--candidate` is not in the session's visual index.
pub const UNKNOWN_CANDIDATE_REMEDIATION: &str = "This session's visual index has no candidate with that identity. Nothing was changed. Pass a candidate_id that candidates returned for this session.";

/// Remediation when an evidence identity is not in the session.
pub const UNKNOWN_EVIDENCE_REMEDIATION: &str = "This session holds no evidence item with that identity. Nothing was changed. Pass an evidence_id that frame get, frame neighbours, frame burst or crop returned for this session.";

/// Remediation when the parent evidence is of a kind the command cannot use.
pub const EVIDENCE_KIND_REMEDIATION: &str = "The evidence item is of a kind this command cannot use: frame neighbours needs a whole frame from frame get, frame neighbours or frame burst, and crop needs a frame or an earlier crop, never an audio clip. Nothing was changed.";

/// Remediation for a crop rectangle that is not inside its parent image.
pub const CROP_OUTSIDE_REMEDIATION: &str = "The crop rectangle must lie wholly inside the parent image: x + width at most its width and y + height at most its height, in the parent image's own displayed pixels, with width and height at least 1. Nothing was changed.";

/// Remediation when a delivered file's path cannot be written as JSON text.
pub const EVIDENCE_PATH_REMEDIATION: &str = "The evidence was committed, but the session folder's path is not valid UTF-8, so its files cannot be named in JSON. Run the command again with --session-root set to a folder whose path is valid UTF-8.";

/// Remediation when no displayed frame satisfies a frame request, chosen by
/// the typed reason (ADR 0019 decision 2).
#[must_use]
pub const fn frame_selection_summary(error: FrameSelectionError) -> &'static str {
    match error {
        FrameSelectionError::AtOrAfterEnd => {
            "The requested time is at or after the end of the video, so no frame is displayed there. Nothing was changed. Request a time before the end of the video."
        }
        FrameSelectionError::AfterFinalFrame => {
            "No frame is displayed at or after the requested time: it follows the video's final frame. Nothing was changed. Use --select displayed-at to take the frame on screen at that time, or request an earlier time."
        }
        FrameSelectionError::BeforeFirstFrame => {
            "No frame is displayed yet at the requested time: it precedes the video's first frame. Nothing was changed. Use the default at-or-after selection, or request a later time."
        }
        FrameSelectionError::NoFrameWithinTolerance => {
            "The nearest frame the selection allows lies further from the requested time than the tolerance (1 s unless --tolerance-us says otherwise, at most 10 s). Nothing was changed. Raise --tolerance-us, use the other --select policy, or request another time."
        }
        FrameSelectionError::OutsideListing => {
            "The video is too dense there for one frame listing (at most 1,200 frames), so the frames could not be decided. Nothing was changed. Request a shorter range, for example a burst of at most 20 s of 60 fps video."
        }
        FrameSelectionError::AnchorNotListed => {
            "The frame the evidence names could not be found in the video again. Nothing was changed. Request the frame with frame get first."
        }
    }
}

/// The fixed warning of a partial evidence result, chosen by its reason.
#[must_use]
pub const fn partial_evidence_warning(reason: PartialReason) -> &'static str {
    match reason {
        PartialReason::FrameBudget => {
            "The call returned the most frames one call may (100); the items returned are committed evidence. Request the rest with another call."
        }
        PartialReason::PixelBudget => {
            "The call reached its limit of 200 megapixels decoded; the items returned are committed evidence. Request the rest with another call or fewer frames."
        }
        PartialReason::ByteBudget => {
            "The call reached its limit of 256 MiB of images or the session's remaining space; the items returned are committed evidence. Request fewer frames, or retain the session and open a new one."
        }
        PartialReason::SessionEvidenceBudget => {
            "The session's 160 evidence artifacts are used up; the items returned are committed evidence. Keep them with session retain, then open a new session to continue."
        }
        PartialReason::DeadlineExceeded => {
            "The call reached its 120 s deadline; the items returned are committed evidence. Repeat the request for the rest."
        }
        PartialReason::Cancelled => {
            "The call was cancelled; the items returned are committed evidence."
        }
    }
}

/// One file delivered with an evidence result (ADR 0019 D2).
#[derive(Clone, Copy, Debug)]
pub struct DeliveredEvidenceFile<'a> {
    /// The item the file shows or plays.
    pub evidence_id: &'a EvidenceId,
    /// What the file is.
    pub kind: EvidenceMediaKind,
    /// Absolute path of the committed session artifact.
    pub path: &'a Path,
}

/// Everything a host presents about one evidence call.
#[derive(Clone, Copy, Debug)]
pub struct EvidencePresentation<'a> {
    /// The call's lineage record.
    pub record: &'a EvidenceRecord,
    /// Whether the record was committed by an earlier identical request.
    pub reused: bool,
    /// One verified file per item, in the record's item order.
    pub files: &'a [DeliveredEvidenceFile<'a>],
}

/// Why an evidence result could not be presented.
#[derive(Debug)]
pub enum EvidencePresentationError {
    /// A delivered file's path is not valid UTF-8.
    NonUtf8Path,
    /// The record is of an operation this result does not present.
    OperationMismatch,
    /// The files are not exactly one per item, in item order, of the item's
    /// kind; an internal fault.
    FileMismatch,
    /// The result could not be represented as JSON.
    Serialization(serde_json::Error),
}

impl fmt::Display for EvidencePresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonUtf8Path => formatter.write_str("an evidence file path is not valid UTF-8"),
            Self::OperationMismatch => {
                formatter.write_str("the evidence record is of another operation")
            }
            Self::FileMismatch => {
                formatter.write_str("the delivered files do not match the record's items")
            }
            Self::Serialization(error) => error.fmt(formatter),
        }
    }
}

impl Error for EvidencePresentationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Serialization(error) => Some(error),
            Self::NonUtf8Path | Self::OperationMismatch | Self::FileMismatch => None,
        }
    }
}

impl From<serde_json::Error> for EvidencePresentationError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

/// The public command of an evidence operation.
const fn command_of(operation: EvidenceOperation) -> CommandName {
    match operation {
        EvidenceOperation::FrameGet => CommandName::FrameGet,
        EvidenceOperation::FrameNeighbours => CommandName::FrameNeighbours,
        EvidenceOperation::FrameBurst => CommandName::FrameBurst,
        EvidenceOperation::Crop => CommandName::Crop,
        EvidenceOperation::Audio => CommandName::Audio,
    }
}

/// A rectangle in an image's pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct RectData {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// The canonical request, with the fields of its operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum RequestData {
    FrameGet {
        at_us: u64,
        policy: &'static str,
        tolerance_us: u64,
        candidate_id: Option<String>,
    },
    FrameNeighbours {
        anchor_evidence_id: String,
        count: u8,
    },
    Range {
        from_us: u64,
        to_us: u64,
    },
    FrameBurst {
        from_us: u64,
        to_us: u64,
        max_frames: u8,
    },
    Crop {
        parent_evidence_id: String,
        rect: RectData,
    },
}

impl RequestData {
    pub(crate) fn new(request: &EvidenceRequest) -> Self {
        match request {
            EvidenceRequest::FrameAt {
                at,
                selection,
                tolerance,
                candidate,
            } => Self::FrameGet {
                at_us: at.as_micros(),
                policy: selection.identifier(),
                tolerance_us: tolerance.as_micros(),
                candidate_id: candidate.as_ref().map(|id| id.as_str().to_owned()),
            },
            EvidenceRequest::Neighbours { anchor, count } => Self::FrameNeighbours {
                anchor_evidence_id: anchor.as_str().to_owned(),
                count: count.get(),
            },
            EvidenceRequest::Burst { range, max_frames } => Self::FrameBurst {
                from_us: range.range().start().as_micros(),
                to_us: range.range().end().as_micros(),
                max_frames: max_frames.get(),
            },
            EvidenceRequest::Crop { parent, rect } => Self::Crop {
                parent_evidence_id: parent.as_str().to_owned(),
                rect: RectData {
                    x: rect.x(),
                    y: rect.y(),
                    width: rect.width(),
                    height: rect.height(),
                },
            },
            EvidenceRequest::Audio { range } => Self::Range {
                from_us: range.range().start().as_micros(),
                to_us: range.range().end().as_micros(),
            },
        }
    }
}

/// Which item a requested time resolved to.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct SelectionData {
    role: &'static str,
    evidence_id: String,
    requested_us: u64,
    actual_us: u64,
    delta_us: i64,
}

impl SelectionData {
    pub(crate) fn new(selection: &EvidenceSelection) -> Self {
        let timing = selection.timing();
        Self {
            role: selection.role().identifier(),
            evidence_id: selection.evidence().as_str().to_owned(),
            requested_us: timing.requested().as_micros(),
            actual_us: timing.actual().as_micros(),
            delta_us: timing.delta_micros(),
        }
    }
}

/// One delivered file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct FileData {
    evidence_id: String,
    media_type: &'static str,
    path: String,
}

/// The media type a delivered file of `kind` has.
pub(crate) const fn media_type(kind: EvidenceMediaKind) -> &'static str {
    match kind {
        EvidenceMediaKind::FramePng => PNG_MEDIA_TYPE,
        EvidenceMediaKind::AudioWav => "audio/wav",
    }
}

/// The delivered files, checked to be one per item in item order.
pub(crate) fn files_data(
    record: &EvidenceRecord,
    files: &[DeliveredEvidenceFile<'_>],
) -> Result<Vec<FileData>, EvidencePresentationError> {
    if files.len() != record.items().len() {
        return Err(EvidencePresentationError::FileMismatch);
    }
    record
        .items()
        .iter()
        .zip(files)
        .map(|(item, file)| {
            if file.evidence_id != item.id() || file.kind != item.media().kind() {
                return Err(EvidencePresentationError::FileMismatch);
            }
            Ok(FileData {
                evidence_id: item.id().as_str().to_owned(),
                media_type: media_type(file.kind),
                path: file
                    .path
                    .to_str()
                    .ok_or(EvidencePresentationError::NonUtf8Path)?
                    .to_owned(),
            })
        })
        .collect()
}

/// How the call that committed the record checked the source copy: every
/// item a call commits carries the call's own check.
pub(crate) fn record_source_check(record: &EvidenceRecord) -> &'static str {
    record
        .items()
        .first()
        .map_or(SourceCheck::FullHash, EvidenceItem::source_check)
        .identifier()
}

const fn stop_identifier(stop: Option<NeighbourStop>) -> Option<&'static str> {
    match stop {
        Some(stop) => Some(stop.identifier()),
        None => None,
    }
}

const fn extent_identifier(extent: BurstExtent) -> &'static str {
    match extent {
        BurstExtent::Requested => "requested",
        BurstExtent::ClippedAtEndOfStream => "clipped_at_end_of_stream",
    }
}

/// Why each side of a neighbours result holds fewer frames than asked.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct NeighboursData {
    before_stop: Option<&'static str>,
    after_stop: Option<&'static str>,
}

/// A half-open range of source time.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct PlannedRange {
    start_us: u64,
    end_us: u64,
}

/// A burst's plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct BurstData {
    planned: PlannedRange,
    extent: &'static str,
    targets: u8,
    distinct: u8,
}

/// One displayed frame: its stream timestamp and the displayed frame's size.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct FrameRefData {
    time_base_numerator: u32,
    time_base_denominator: u32,
    pts: i64,
    time_us: u64,
    width: u32,
    height: u32,
}

impl FrameRefData {
    const fn new(frame: &FrameRef) -> Self {
        Self {
            time_base_numerator: frame.time_base.numerator(),
            time_base_denominator: frame.time_base.denominator(),
            pts: frame.pts,
            time_us: frame.time.as_micros(),
            width: frame.dimensions.width(),
            height: frame.dimensions.height(),
        }
    }
}

/// Where a crop lies, in its parent's and the source frame's pixels.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct CropData {
    parent_evidence_id: String,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    frame_x: u32,
    frame_y: u32,
}

/// The delivered image's content address and size.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ImageData {
    media_type: &'static str,
    width: u32,
    height: u32,
    sha256: String,
    bytes: u64,
}

/// One published `frame_evidence` record: a whole displayed frame or a crop
/// of one.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FrameEvidenceData {
    evidence_id: String,
    source_id: String,
    stream_index: u32,
    kind: &'static str,
    frame: FrameRefData,
    crop: Option<CropData>,
    image: ImageData,
    profile: &'static str,
    tool_fingerprint: Option<String>,
}

impl FrameEvidenceData {
    /// The item's identity, the record's upsert key.
    pub(crate) fn evidence_id(&self) -> &str {
        &self.evidence_id
    }

    /// Presents one frame or crop item of `record`; `None` for an audio clip.
    fn new(record: &EvidenceRecord, item: &EvidenceItem) -> Option<Self> {
        let (kind, frame, crop) = match item.subject() {
            EvidenceSubject::Frame(frame) => ("frame", frame, None),
            EvidenceSubject::Crop { frame, region } => (
                "crop",
                frame,
                Some(CropData {
                    parent_evidence_id: region.parent.as_str().to_owned(),
                    x: region.rect.x(),
                    y: region.rect.y(),
                    width: region.rect.width(),
                    height: region.rect.height(),
                    frame_x: region.frame_rect.x(),
                    frame_y: region.frame_rect.y(),
                }),
            ),
            EvidenceSubject::Audio { .. } => return None,
        };
        let dimensions = item.subject().image_dimensions()?;
        Some(Self {
            evidence_id: item.id().as_str().to_owned(),
            source_id: record.source_id().as_str().to_owned(),
            stream_index: frame.stream_index,
            kind,
            frame: FrameRefData::new(frame),
            crop,
            image: ImageData {
                media_type: PNG_MEDIA_TYPE,
                width: dimensions.width(),
                height: dimensions.height(),
                sha256: item.media().sha256().as_str().to_owned(),
                bytes: item.media().bytes(),
            },
            profile: record.profile().identifier(),
            tool_fingerprint: item
                .tool_fingerprint()
                .map(|fingerprint| fingerprint.as_str().to_owned()),
        })
    }
}

/// Everything a frame result states besides its items.
#[derive(Clone, Debug, Eq, PartialEq)]
struct FrameSummary {
    command: CommandName,
    session_id: String,
    source_id: String,
    operation: &'static str,
    request: RequestData,
    request_key: String,
    reused: bool,
    profile: &'static str,
    tool_fingerprint: Option<String>,
    source_check: &'static str,
    selections: Vec<SelectionData>,
    neighbours: Option<NeighboursData>,
    burst: Option<BurstData>,
    files: Vec<FileData>,
    partial: Option<PartialReason>,
}

impl FrameSummary {
    fn new(presentation: &EvidencePresentation<'_>) -> Result<Self, EvidencePresentationError> {
        let record = presentation.record;
        let operation = record.request().operation();
        if operation == EvidenceOperation::Audio {
            return Err(EvidencePresentationError::OperationMismatch);
        }
        let (neighbours, burst) = match record.detail() {
            EvidenceDetail::Neighbours {
                before_stop,
                after_stop,
            } => (
                Some(NeighboursData {
                    before_stop: stop_identifier(before_stop),
                    after_stop: stop_identifier(after_stop),
                }),
                None,
            ),
            EvidenceDetail::Burst {
                planned,
                extent,
                targets,
                distinct,
            } => (
                None,
                Some(BurstData {
                    planned: PlannedRange {
                        start_us: planned.start().as_micros(),
                        end_us: planned.end().as_micros(),
                    },
                    extent: extent_identifier(extent),
                    targets,
                    distinct,
                }),
            ),
            EvidenceDetail::Single => (None, None),
            EvidenceDetail::Audio { .. } => {
                return Err(EvidencePresentationError::OperationMismatch);
            }
        };
        Ok(Self {
            command: command_of(operation),
            session_id: record.session_id().as_str().to_owned(),
            source_id: record.source_id().as_str().to_owned(),
            operation: operation.identifier(),
            request: RequestData::new(record.request()),
            request_key: record.request_key().as_str().to_owned(),
            reused: presentation.reused,
            profile: record.profile().identifier(),
            tool_fingerprint: record
                .tool_fingerprint()
                .map(|fingerprint| fingerprint.as_str().to_owned()),
            source_check: record_source_check(record),
            selections: record.selections().iter().map(SelectionData::new).collect(),
            neighbours,
            burst,
            files: files_data(record, presentation.files)?,
            partial: record.partial(),
        })
    }

    fn items(record: &EvidenceRecord) -> Result<Vec<FrameEvidenceData>, EvidencePresentationError> {
        record
            .items()
            .iter()
            .map(|item| {
                FrameEvidenceData::new(record, item)
                    .ok_or(EvidencePresentationError::OperationMismatch)
            })
            .collect()
    }
}

/// Data of a complete or partial `frame get`, `frame neighbours`,
/// `frame burst` or `crop` result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FrameData {
    session_id: String,
    source_id: String,
    operation: &'static str,
    request: RequestData,
    request_key: String,
    reused: bool,
    profile: &'static str,
    tool_fingerprint: Option<String>,
    source_check: &'static str,
    selections: Vec<SelectionData>,
    neighbours: Option<NeighboursData>,
    burst: Option<BurstData>,
    items: Vec<FrameEvidenceData>,
    files: Vec<FileData>,
    partial_reason: Option<&'static str>,
}

/// Data of the terminal event that ends a frame or crop evidence stream: the
/// result without its items, which were the preceding evidence events, and
/// `record_count` saying how many there were.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FrameStreamData {
    session_id: String,
    source_id: String,
    operation: &'static str,
    request: RequestData,
    request_key: String,
    reused: bool,
    profile: &'static str,
    tool_fingerprint: Option<String>,
    source_check: &'static str,
    selections: Vec<SelectionData>,
    neighbours: Option<NeighboursData>,
    burst: Option<BurstData>,
    record_count: usize,
    files: Vec<FileData>,
    partial_reason: Option<&'static str>,
}

/// Completes an evidence result with its lifecycle and, when the call
/// stopped short, the `partial` status and the reason's warning.
pub(crate) fn finish_evidence(
    command: CommandName,
    data: &impl Serialize,
    partial: Option<PartialReason>,
    lifecycle: LifecycleResponse,
) -> Result<OperationResponse<serde_json::Value>, serde_json::Error> {
    let response = match partial {
        Some(reason) => OperationResponse::partial(
            command.identifier(),
            data,
            partial_evidence_warning(reason),
        )?,
        None => OperationResponse::complete(command.identifier(), data)?,
    };
    Ok(response.with_lifecycle(lifecycle))
}

/// The complete or partial result of a `frame get`, `frame neighbours`,
/// `frame burst` or `crop` call.
///
/// # Errors
///
/// Returns [`EvidencePresentationError`] for an audio record, files that do
/// not match the items, a path that is not valid UTF-8 or data that cannot be
/// represented as JSON.
pub fn frame_response(
    presentation: &EvidencePresentation<'_>,
    lifecycle: LifecycleResponse,
) -> Result<OperationResponse<serde_json::Value>, EvidencePresentationError> {
    let summary = FrameSummary::new(presentation)?;
    let items = FrameSummary::items(presentation.record)?;
    let data = FrameData {
        session_id: summary.session_id,
        source_id: summary.source_id,
        operation: summary.operation,
        request: summary.request,
        request_key: summary.request_key,
        reused: summary.reused,
        profile: summary.profile,
        tool_fingerprint: summary.tool_fingerprint,
        source_check: summary.source_check,
        selections: summary.selections,
        neighbours: summary.neighbours,
        burst: summary.burst,
        items,
        files: summary.files,
        partial_reason: summary.partial.map(PartialReason::identifier),
    };
    Ok(finish_evidence(
        summary.command,
        &data,
        summary.partial,
        lifecycle,
    )?)
}

/// One frame or crop result as a JSON Lines evidence stream.
///
/// The records are the result's items, in the record's item order, as
/// `frame_evidence` evidence events numbered from 0; the terminal event
/// follows with the next sequence number and carries everything else. A
/// burst holds at most 100 items, so the stream is bounded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameEvidenceStream {
    records: Vec<EvidenceEventResponse>,
    terminal: TerminalEventResponse,
}

impl FrameEvidenceStream {
    /// Presents one frame or crop result as an evidence stream.
    ///
    /// # Errors
    ///
    /// As [`frame_response`].
    pub fn new(
        presentation: &EvidencePresentation<'_>,
        lifecycle: LifecycleResponse,
    ) -> Result<Self, EvidencePresentationError> {
        let summary = FrameSummary::new(presentation)?;
        let records: Vec<_> = (0_u64..)
            .zip(FrameSummary::items(presentation.record)?)
            .map(|(sequence, item)| {
                EvidenceEventResponse::frame_evidence(sequence, summary.command, item)
            })
            .collect();
        let data = FrameStreamData {
            session_id: summary.session_id,
            source_id: summary.source_id,
            operation: summary.operation,
            request: summary.request,
            request_key: summary.request_key,
            reused: summary.reused,
            profile: summary.profile,
            tool_fingerprint: summary.tool_fingerprint,
            source_check: summary.source_check,
            selections: summary.selections,
            neighbours: summary.neighbours,
            burst: summary.burst,
            record_count: records.len(),
            files: summary.files,
            partial_reason: summary.partial.map(PartialReason::identifier),
        };
        let result = finish_evidence(summary.command, &data, summary.partial, lifecycle)?;
        let terminal_sequence = u64::try_from(records.len()).unwrap_or(u64::MAX);
        Ok(Self {
            records,
            terminal: TerminalEventResponse::at_sequence(result, terminal_sequence),
        })
    }
}

impl EvidenceStream for FrameEvidenceStream {
    fn records(&self) -> &[EvidenceEventResponse] {
        &self.records
    }

    fn terminal(&self) -> &TerminalEventResponse {
        &self.terminal
    }
}

#[cfg(test)]
mod tests {
    use vsift_domain::PartialReason;

    use super::partial_evidence_warning;

    #[test]
    fn every_partial_reason_has_its_own_warning() {
        for (index, reason) in PartialReason::ALL.into_iter().enumerate() {
            let warning = partial_evidence_warning(reason);
            assert!(warning.ends_with('.'));
            assert!(
                PartialReason::ALL[index + 1..]
                    .iter()
                    .all(|other| partial_evidence_warning(*other) != warning)
            );
        }
    }
}
