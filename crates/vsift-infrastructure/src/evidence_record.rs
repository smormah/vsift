//! Versioned storage record of one evidence call (P09 PR 2, ADR 0019).
//!
//! Every extracting evidence call commits one immutable, content-addressed
//! session artifact of kind `evidence_record` beside the media it extracted,
//! and the record travels unchanged into retained bundles. It holds the
//! call's request key and canonical parameters, which item each requested
//! time resolved to (with requested and actual time and their delta), the
//! items themselves (stream, exact timestamp and time base, crop region or
//! clip range, media digest and size, provider fingerprint and source check)
//! and why the call stopped short, if it did. It never holds an operation
//! identifier, a path or any media bytes.
//!
//! Decoding is strict, like the transcript and visual-index records: unknown
//! fields, versions and values are rejected, the record is rebuilt through
//! the domain constructors (bounds, one selection per requested time naming
//! an item of the record, operation-specific shapes) and the request key and
//! every item identity are re-derived by the application, so a record whose
//! identities were edited is rejected.

use serde::{Deserialize, Serialize};
use vsift_application::{SessionStorageError, verify_evidence_record};
use vsift_domain::{
    AUDIO_CLIP_SAMPLE_RATE, AudioRange, BurstCount, BurstExtent, BurstRange, CropRect, CropRegion,
    EvidenceDetail, EvidenceId, EvidenceItem, EvidenceItemParts, EvidenceMedia, EvidenceMediaKind,
    EvidenceProfile, EvidenceRecord, EvidenceRecordParts, EvidenceRequest, EvidenceSelection,
    EvidenceSubject, FrameDimensions, FrameRef, FrameSelection, FrameTolerance, MediaTime,
    NeighbourCount, NeighbourStop, OperationKey, PartialReason, SelectionRole, SessionId,
    Sha256Hex, SourceCheck, SourceId, TimeBase, TimeRange, VisualCandidateId,
};

/// Largest encoded evidence record a session will store or read.
///
/// A burst of 100 frames is the largest record: about 100 items of 600
/// bytes and 100 selections of 150, well under a quarter of this bound.
pub const MAX_EVIDENCE_RECORD_BYTES: usize = 256 * 1024;
/// Largest evidence WAV clip: a 44-byte header and 30 s of 16 kHz mono
/// 16-bit samples (960,044 bytes) fit in one mebibyte.
pub const MAX_AUDIO_WAV_BYTES: usize = 1024 * 1024;
/// Most evidence artifacts one session holds: frame and crop images, audio
/// clips and evidence records together (ADR 0019 D4). The session's 256
/// artifact slots keep the rest for transcripts and visual indexes.
pub const MAX_EVIDENCE_ARTIFACTS: usize = 160;
const RECORD_VERSION: u16 = 1;
const RECORD_FORMAT: &str = "vsift.evidence_record";

/// Encodes an evidence record as its bounded storage record.
///
/// # Errors
///
/// Returns [`SessionStorageError::CapacityExhausted`] when the record would
/// exceed [`MAX_EVIDENCE_RECORD_BYTES`].
pub fn encode_evidence_record(record: &EvidenceRecord) -> Result<Vec<u8>, SessionStorageError> {
    let bytes = serde_json::to_vec(&StoredEvidenceRecord::from_record(record))
        .map_err(|_| SessionStorageError::Io)?;
    if bytes.len() > MAX_EVIDENCE_RECORD_BYTES {
        return Err(SessionStorageError::CapacityExhausted);
    }
    Ok(bytes)
}

/// Decodes and revalidates a stored evidence record of `session_id`.
///
/// # Errors
///
/// Returns [`SessionStorageError::UnsupportedVersion`] for a newer record and
/// [`SessionStorageError::IntegrityFailure`] for anything malformed,
/// oversized, of another session, structurally invalid or with an identity
/// that does not re-derive.
pub fn decode_evidence_record(
    bytes: &[u8],
    session_id: &SessionId,
) -> Result<EvidenceRecord, SessionStorageError> {
    #[derive(Deserialize)]
    struct VersionProbe {
        schema_version: u16,
    }
    if bytes.len() > MAX_EVIDENCE_RECORD_BYTES {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let probe: VersionProbe =
        serde_json::from_slice(bytes).map_err(|_| SessionStorageError::IntegrityFailure)?;
    match probe.schema_version {
        RECORD_VERSION => {}
        newer if newer > RECORD_VERSION => return Err(SessionStorageError::UnsupportedVersion),
        _ => return Err(SessionStorageError::IntegrityFailure),
    }
    let stored: StoredEvidenceRecord =
        serde_json::from_slice(bytes).map_err(|_| SessionStorageError::IntegrityFailure)?;
    let record = stored
        .into_record(session_id)
        .ok_or(SessionStorageError::IntegrityFailure)?;
    verify_evidence_record(&record).map_err(|_| SessionStorageError::IntegrityFailure)?;
    Ok(record)
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredEvidenceRecord {
    schema_version: u16,
    format: String,
    request_key: String,
    session_id: String,
    source_id: String,
    profile: StoredProfile,
    tool_fingerprint: Option<String>,
    request: StoredRequest,
    selections: Vec<StoredSelection>,
    detail: StoredDetail,
    items: Vec<StoredItem>,
    partial_reason: Option<StoredPartial>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum StoredProfile {
    #[serde(rename = "p09-r0-v1")]
    P09R0,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum StoredRequest {
    FrameGet {
        at_us: u64,
        policy: StoredPolicy,
        tolerance_us: u64,
        candidate_id: Option<String>,
    },
    FrameNeighbours {
        anchor_evidence_id: String,
        count: u8,
    },
    FrameBurst {
        from_us: u64,
        to_us: u64,
        max_frames: u8,
    },
    Crop {
        parent_evidence_id: String,
        rect: StoredRect,
    },
    Audio {
        from_us: u64,
        to_us: u64,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredPolicy {
    AtOrAfter,
    DisplayedAt,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredSelection {
    role: StoredRole,
    evidence_id: String,
    requested_us: u64,
    actual_us: u64,
    delta_us: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredRole {
    Requested,
    Before,
    After,
    Target,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum StoredDetail {
    Single,
    Neighbours {
        before_stop: Option<StoredStop>,
        after_stop: Option<StoredStop>,
    },
    Burst {
        planned: StoredRange,
        extent: StoredExtent,
        targets: u8,
        distinct: u8,
    },
    Audio {
        range_clipped: bool,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredStop {
    StartOfStream,
    EndOfStream,
    SearchWindow,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredExtent {
    Requested,
    ClippedAtEndOfStream,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredRange {
    start_us: u64,
    end_us: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredPartial {
    FrameBudget,
    PixelBudget,
    ByteBudget,
    SessionEvidenceBudget,
    DeadlineExceeded,
    Cancelled,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredItem {
    evidence_id: String,
    stream_index: u32,
    subject: StoredSubject,
    media: StoredMedia,
    tool_fingerprint: Option<String>,
    source_check: StoredSourceCheck,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum StoredSubject {
    Frame(StoredFrame),
    Crop {
        frame: StoredFrame,
        region: StoredRegion,
    },
    Audio {
        start_us: u64,
        end_us: u64,
        actual_start_us: u64,
        sample_rate: u32,
        channels: u8,
        sample_format: StoredSampleFormat,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredFrame {
    time_base_numerator: u32,
    time_base_denominator: u32,
    pts: i64,
    time_us: u64,
    width: u32,
    height: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredRegion {
    parent_evidence_id: String,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    frame_x: u32,
    frame_y: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredSampleFormat {
    S16le,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredMedia {
    kind: StoredMediaKind,
    sha256: String,
    bytes: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredMediaKind {
    FramePng,
    AudioWav,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredSourceCheck {
    Identity,
    FullHash,
}

impl StoredEvidenceRecord {
    fn from_record(record: &EvidenceRecord) -> Self {
        Self {
            schema_version: RECORD_VERSION,
            format: RECORD_FORMAT.to_owned(),
            request_key: record.request_key().as_str().to_owned(),
            session_id: record.session_id().as_str().to_owned(),
            source_id: record.source_id().as_str().to_owned(),
            profile: match record.profile() {
                EvidenceProfile::P09R0 => StoredProfile::P09R0,
            },
            tool_fingerprint: record
                .tool_fingerprint()
                .map(|value| value.as_str().to_owned()),
            request: StoredRequest::from_request(record.request()),
            selections: record
                .selections()
                .iter()
                .map(StoredSelection::from_selection)
                .collect(),
            detail: StoredDetail::from_detail(record.detail()),
            items: record.items().iter().map(StoredItem::from_item).collect(),
            partial_reason: record.partial().map(|reason| match reason {
                PartialReason::FrameBudget => StoredPartial::FrameBudget,
                PartialReason::PixelBudget => StoredPartial::PixelBudget,
                PartialReason::ByteBudget => StoredPartial::ByteBudget,
                PartialReason::SessionEvidenceBudget => StoredPartial::SessionEvidenceBudget,
                PartialReason::DeadlineExceeded => StoredPartial::DeadlineExceeded,
                PartialReason::Cancelled => StoredPartial::Cancelled,
            }),
        }
    }

    fn into_record(self, session_id: &SessionId) -> Option<EvidenceRecord> {
        if self.format != RECORD_FORMAT || self.session_id != session_id.as_str() {
            return None;
        }
        let mut items = Vec::with_capacity(self.items.len());
        for item in self.items {
            items.push(item.into_item()?);
        }
        let mut selections = Vec::with_capacity(self.selections.len());
        for selection in self.selections {
            selections.push(selection.into_selection()?);
        }
        EvidenceRecord::new(EvidenceRecordParts {
            request_key: OperationKey::parse(self.request_key).ok()?,
            session_id: session_id.clone(),
            source_id: SourceId::parse(self.source_id).ok()?,
            profile: match self.profile {
                StoredProfile::P09R0 => EvidenceProfile::P09R0,
            },
            tool_fingerprint: digest(self.tool_fingerprint).ok()?,
            request: self.request.into_request()?,
            selections,
            detail: self.detail.into_detail()?,
            items,
            partial: self.partial_reason.map(|reason| match reason {
                StoredPartial::FrameBudget => PartialReason::FrameBudget,
                StoredPartial::PixelBudget => PartialReason::PixelBudget,
                StoredPartial::ByteBudget => PartialReason::ByteBudget,
                StoredPartial::SessionEvidenceBudget => PartialReason::SessionEvidenceBudget,
                StoredPartial::DeadlineExceeded => PartialReason::DeadlineExceeded,
                StoredPartial::Cancelled => PartialReason::Cancelled,
            }),
        })
        .ok()
    }
}

/// An optional digest; a present one must be canonical.
fn digest(value: Option<String>) -> Result<Option<Sha256Hex>, vsift_domain::DigestError> {
    value.map(Sha256Hex::parse).transpose()
}

/// A positive rectangle, validated against the smallest frame holding it.
fn positive_rect(x: u32, y: u32, width: u32, height: u32) -> Option<CropRect> {
    let frame = FrameDimensions::new(x.checked_add(width)?, y.checked_add(height)?).ok()?;
    CropRect::new(x, y, width, height, frame).ok()
}

fn time_range(start_us: u64, end_us: u64) -> Option<TimeRange> {
    TimeRange::new(
        MediaTime::from_micros(start_us),
        MediaTime::from_micros(end_us),
    )
    .ok()
}

impl StoredRequest {
    fn from_request(request: &EvidenceRequest) -> Self {
        match request {
            EvidenceRequest::FrameAt {
                at,
                selection,
                tolerance,
                candidate,
            } => Self::FrameGet {
                at_us: at.as_micros(),
                policy: match selection {
                    FrameSelection::AtOrAfter => StoredPolicy::AtOrAfter,
                    FrameSelection::DisplayedAt => StoredPolicy::DisplayedAt,
                },
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
                rect: StoredRect {
                    x: rect.x(),
                    y: rect.y(),
                    width: rect.width(),
                    height: rect.height(),
                },
            },
            EvidenceRequest::Audio { range } => Self::Audio {
                from_us: range.range().start().as_micros(),
                to_us: range.range().end().as_micros(),
            },
        }
    }

    fn into_request(self) -> Option<EvidenceRequest> {
        Some(match self {
            Self::FrameGet {
                at_us,
                policy,
                tolerance_us,
                candidate_id,
            } => EvidenceRequest::FrameAt {
                at: MediaTime::from_micros(at_us),
                selection: match policy {
                    StoredPolicy::AtOrAfter => FrameSelection::AtOrAfter,
                    StoredPolicy::DisplayedAt => FrameSelection::DisplayedAt,
                },
                tolerance: FrameTolerance::new(tolerance_us).ok()?,
                candidate: match candidate_id {
                    None => None,
                    Some(id) => Some(VisualCandidateId::parse(id).ok()?),
                },
            },
            Self::FrameNeighbours {
                anchor_evidence_id,
                count,
            } => EvidenceRequest::Neighbours {
                anchor: EvidenceId::parse(anchor_evidence_id).ok()?,
                count: NeighbourCount::new(count).ok()?,
            },
            Self::FrameBurst {
                from_us,
                to_us,
                max_frames,
            } => EvidenceRequest::Burst {
                range: BurstRange::new(time_range(from_us, to_us)?).ok()?,
                max_frames: BurstCount::new(max_frames).ok()?,
            },
            Self::Crop {
                parent_evidence_id,
                rect,
            } => EvidenceRequest::Crop {
                parent: EvidenceId::parse(parent_evidence_id).ok()?,
                rect: positive_rect(rect.x, rect.y, rect.width, rect.height)?,
            },
            Self::Audio { from_us, to_us } => EvidenceRequest::Audio {
                range: AudioRange::new(time_range(from_us, to_us)?).ok()?,
            },
        })
    }
}

impl StoredSelection {
    fn from_selection(selection: &EvidenceSelection) -> Self {
        let timing = selection.timing();
        Self {
            role: match selection.role() {
                SelectionRole::Requested => StoredRole::Requested,
                SelectionRole::Before => StoredRole::Before,
                SelectionRole::After => StoredRole::After,
                SelectionRole::Target => StoredRole::Target,
            },
            evidence_id: selection.evidence().as_str().to_owned(),
            requested_us: timing.requested().as_micros(),
            actual_us: timing.actual().as_micros(),
            delta_us: timing.delta_micros(),
        }
    }

    fn into_selection(self) -> Option<EvidenceSelection> {
        let selection = EvidenceSelection::new(
            match self.role {
                StoredRole::Requested => SelectionRole::Requested,
                StoredRole::Before => SelectionRole::Before,
                StoredRole::After => SelectionRole::After,
                StoredRole::Target => SelectionRole::Target,
            },
            EvidenceId::parse(self.evidence_id).ok()?,
            MediaTime::from_micros(self.requested_us),
            MediaTime::from_micros(self.actual_us),
        )
        .ok()?;
        (selection.timing().delta_micros() == self.delta_us).then_some(selection)
    }
}

const fn stop_from(stop: NeighbourStop) -> StoredStop {
    match stop {
        NeighbourStop::StartOfStream => StoredStop::StartOfStream,
        NeighbourStop::EndOfStream => StoredStop::EndOfStream,
        NeighbourStop::SearchWindow => StoredStop::SearchWindow,
    }
}

const fn stop_into(stop: StoredStop) -> NeighbourStop {
    match stop {
        StoredStop::StartOfStream => NeighbourStop::StartOfStream,
        StoredStop::EndOfStream => NeighbourStop::EndOfStream,
        StoredStop::SearchWindow => NeighbourStop::SearchWindow,
    }
}

impl StoredDetail {
    fn from_detail(detail: EvidenceDetail) -> Self {
        match detail {
            EvidenceDetail::Single => Self::Single,
            EvidenceDetail::Neighbours {
                before_stop,
                after_stop,
            } => Self::Neighbours {
                before_stop: before_stop.map(stop_from),
                after_stop: after_stop.map(stop_from),
            },
            EvidenceDetail::Burst {
                planned,
                extent,
                targets,
                distinct,
            } => Self::Burst {
                planned: StoredRange {
                    start_us: planned.start().as_micros(),
                    end_us: planned.end().as_micros(),
                },
                extent: match extent {
                    BurstExtent::Requested => StoredExtent::Requested,
                    BurstExtent::ClippedAtEndOfStream => StoredExtent::ClippedAtEndOfStream,
                },
                targets,
                distinct,
            },
            EvidenceDetail::Audio { range_clipped } => Self::Audio { range_clipped },
        }
    }

    fn into_detail(self) -> Option<EvidenceDetail> {
        Some(match self {
            Self::Single => EvidenceDetail::Single,
            Self::Neighbours {
                before_stop,
                after_stop,
            } => EvidenceDetail::Neighbours {
                before_stop: before_stop.map(stop_into),
                after_stop: after_stop.map(stop_into),
            },
            Self::Burst {
                planned,
                extent,
                targets,
                distinct,
            } => EvidenceDetail::Burst {
                planned: time_range(planned.start_us, planned.end_us)?,
                extent: match extent {
                    StoredExtent::Requested => BurstExtent::Requested,
                    StoredExtent::ClippedAtEndOfStream => BurstExtent::ClippedAtEndOfStream,
                },
                targets,
                distinct,
            },
            Self::Audio { range_clipped } => EvidenceDetail::Audio { range_clipped },
        })
    }
}

impl StoredFrame {
    const fn from_frame(frame: &FrameRef) -> Self {
        Self {
            time_base_numerator: frame.time_base.numerator(),
            time_base_denominator: frame.time_base.denominator(),
            pts: frame.pts,
            time_us: frame.time.as_micros(),
            width: frame.dimensions.width(),
            height: frame.dimensions.height(),
        }
    }

    fn into_frame(self, stream_index: u32) -> Option<FrameRef> {
        Some(FrameRef {
            stream_index,
            time_base: TimeBase::new(self.time_base_numerator, self.time_base_denominator).ok()?,
            pts: self.pts,
            time: MediaTime::from_micros(self.time_us),
            dimensions: FrameDimensions::new(self.width, self.height).ok()?,
        })
    }
}

impl StoredItem {
    fn from_item(item: &EvidenceItem) -> Self {
        let (stream_index, subject) = match item.subject() {
            EvidenceSubject::Frame(frame) => (
                frame.stream_index,
                StoredSubject::Frame(StoredFrame::from_frame(frame)),
            ),
            EvidenceSubject::Crop { frame, region } => (
                frame.stream_index,
                StoredSubject::Crop {
                    frame: StoredFrame::from_frame(frame),
                    region: StoredRegion {
                        parent_evidence_id: region.parent.as_str().to_owned(),
                        x: region.rect.x(),
                        y: region.rect.y(),
                        width: region.rect.width(),
                        height: region.rect.height(),
                        frame_x: region.frame_rect.x(),
                        frame_y: region.frame_rect.y(),
                    },
                },
            ),
            EvidenceSubject::Audio {
                stream_index,
                range,
                actual_start,
            } => (
                *stream_index,
                StoredSubject::Audio {
                    start_us: range.start().as_micros(),
                    end_us: range.end().as_micros(),
                    actual_start_us: actual_start.as_micros(),
                    sample_rate: AUDIO_CLIP_SAMPLE_RATE,
                    channels: 1,
                    sample_format: StoredSampleFormat::S16le,
                },
            ),
        };
        let media = item.media();
        Self {
            evidence_id: item.id().as_str().to_owned(),
            stream_index,
            subject,
            media: StoredMedia {
                kind: match media.kind() {
                    EvidenceMediaKind::FramePng => StoredMediaKind::FramePng,
                    EvidenceMediaKind::AudioWav => StoredMediaKind::AudioWav,
                },
                sha256: media.sha256().as_str().to_owned(),
                bytes: media.bytes(),
            },
            tool_fingerprint: item
                .tool_fingerprint()
                .map(|value| value.as_str().to_owned()),
            source_check: match item.source_check() {
                SourceCheck::Identity => StoredSourceCheck::Identity,
                SourceCheck::FullHash => StoredSourceCheck::FullHash,
            },
        }
    }

    fn into_item(self) -> Option<EvidenceItem> {
        let subject = match self.subject {
            StoredSubject::Frame(frame) => {
                EvidenceSubject::Frame(frame.into_frame(self.stream_index)?)
            }
            StoredSubject::Crop { frame, region } => {
                let frame = frame.into_frame(self.stream_index)?;
                let frame_rect = CropRect::new(
                    region.frame_x,
                    region.frame_y,
                    region.width,
                    region.height,
                    frame.dimensions,
                )
                .ok()?;
                EvidenceSubject::Crop {
                    frame,
                    region: CropRegion {
                        parent: EvidenceId::parse(region.parent_evidence_id).ok()?,
                        rect: positive_rect(region.x, region.y, region.width, region.height)?,
                        frame_rect,
                    },
                }
            }
            StoredSubject::Audio {
                start_us,
                end_us,
                actual_start_us,
                sample_rate,
                channels,
                sample_format: StoredSampleFormat::S16le,
            } => {
                if sample_rate != AUDIO_CLIP_SAMPLE_RATE || channels != 1 {
                    return None;
                }
                EvidenceSubject::Audio {
                    stream_index: self.stream_index,
                    range: time_range(start_us, end_us)?,
                    actual_start: MediaTime::from_micros(actual_start_us),
                }
            }
        };
        let media = EvidenceMedia::new(
            match self.media.kind {
                StoredMediaKind::FramePng => EvidenceMediaKind::FramePng,
                StoredMediaKind::AudioWav => EvidenceMediaKind::AudioWav,
            },
            Sha256Hex::parse(self.media.sha256).ok()?,
            self.media.bytes,
        )
        .ok()?;
        let limit = match media.kind() {
            EvidenceMediaKind::FramePng => crate::MAX_FRAME_BYTES,
            EvidenceMediaKind::AudioWav => MAX_AUDIO_WAV_BYTES,
        };
        if media.bytes() > u64::try_from(limit).ok()? {
            return None;
        }
        EvidenceItem::new(EvidenceItemParts {
            id: EvidenceId::parse(self.evidence_id).ok()?,
            subject,
            media,
            tool_fingerprint: digest(self.tool_fingerprint).ok()?,
            source_check: match self.source_check {
                StoredSourceCheck::Identity => SourceCheck::Identity,
                StoredSourceCheck::FullHash => SourceCheck::FullHash,
            },
        })
        .ok()
    }
}

#[cfg(test)]
mod tests;
