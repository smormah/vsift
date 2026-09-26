//! Evidence items and the lineage record of one evidence call (P09 PR 2,
//! ADR 0019).
//!
//! An [`EvidenceItem`] is one extracted piece of source evidence: a whole
//! displayed frame, a crop of one, or an audio clip. Its identity is fixed by
//! the facts that fix its pixels or samples (the session and source, the
//! stream, the adapter profile, the provider's fingerprint, the exact frame
//! timestamp and time base, and the crop region or clip range), so two
//! requests that resolve to the same frame share one item.
//!
//! An [`EvidenceRecord`] is the lineage of one extracting call: its request
//! key and parameters, the [`EvidenceSelection`]s that say which item each
//! requested time resolved to (with requested and actual time and their
//! delta), the items themselves and why the call stopped short, if it did.
//! Request-specific facts (the requested time, the selection policy, the
//! candidate a frame was asked for by) live in the record's request and
//! selections, never in an item. Records never hold an operation identifier
//! or a path.
//!
//! This module holds values and their structural rules only; identities are
//! derived and re-derived by the application layer.

use std::{error::Error, fmt};

use crate::{
    BurstCount, BurstExtent, BurstRange, CropRect, EvidenceId, FrameDimensions, FrameSelection,
    FrameTiming, FrameTolerance, MediaTime, NeighbourCount, NeighbourStop, OperationKey, SessionId,
    Sha256Hex, SourceId, TimeRange, VisualCandidateId,
};

/// Longest audio clip: thirty seconds (ADR 0019 D5).
pub const MAX_AUDIO_CLIP_MICROS: u64 = 30_000_000;
/// Sample rate of every evidence audio clip (16 kHz mono signed 16-bit).
pub const AUDIO_CLIP_SAMPLE_RATE: u32 = 16_000;
/// Most items one record holds: a burst's largest frame budget.
pub const MAX_RECORD_ITEMS: usize = 100;
/// Most selections one record holds: one per burst target.
pub const MAX_RECORD_SELECTIONS: usize = 100;

/// The adapter profile evidence is extracted with.
///
/// It names everything about extraction the provider's fingerprint does not:
/// the PNG and WAV encodings, integer-timestamp selection and the crop
/// filter. A change to any of them is a new profile, so items extracted
/// under different rules never share an identity.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum EvidenceProfile {
    /// Native-resolution 8-bit RGB PNG frames and crops, and 16 kHz mono
    /// signed 16-bit WAV clips (ADR 0019).
    #[default]
    P09R0,
}

impl EvidenceProfile {
    /// Returns the stable identifier used in records and identities.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::P09R0 => "p09-r0-v1",
        }
    }
}

/// How the call that extracted an item proved it read the session's
/// committed source bytes (ADR 0019 D1).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SourceCheck {
    /// The copy's on-disk identity matched the one recorded after an earlier
    /// full verification; its bytes were not hashed again.
    Identity,
    /// The copy's bytes were hashed in full and matched the committed source.
    FullHash,
}

impl SourceCheck {
    /// Every check.
    pub const ALL: [Self; 2] = [Self::Identity, Self::FullHash];

    /// Returns the stable identifier used in records.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::FullHash => "full_hash",
        }
    }
}

/// The evidence operation a record describes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EvidenceOperation {
    /// One frame at a time or of a visual candidate.
    FrameGet,
    /// Consecutive frames around an earlier frame item.
    FrameNeighbours,
    /// Frames spread evenly over a range.
    FrameBurst,
    /// A rectangle of an earlier frame or crop item.
    Crop,
    /// An audio clip of a range.
    Audio,
}

impl EvidenceOperation {
    /// Every operation.
    pub const ALL: [Self; 5] = [
        Self::FrameGet,
        Self::FrameNeighbours,
        Self::FrameBurst,
        Self::Crop,
        Self::Audio,
    ];

    /// Returns the stable identifier used in records and request keys.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::FrameGet => "frame_get",
            Self::FrameNeighbours => "frame_neighbours",
            Self::FrameBurst => "frame_burst",
            Self::Crop => "crop",
            Self::Audio => "audio",
        }
    }
}

/// A stream's positive rational time base.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TimeBase {
    numerator: u32,
    denominator: u32,
}

impl TimeBase {
    /// Creates a time base.
    ///
    /// # Errors
    ///
    /// Returns [`EvidenceRecordError::InvalidItem`] for a zero numerator or
    /// denominator.
    pub const fn new(numerator: u32, denominator: u32) -> Result<Self, EvidenceRecordError> {
        if numerator == 0 || denominator == 0 {
            Err(EvidenceRecordError::InvalidItem)
        } else {
            Ok(Self {
                numerator,
                denominator,
            })
        }
    }

    /// Returns the numerator.
    #[must_use]
    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    /// Returns the denominator.
    #[must_use]
    pub const fn denominator(self) -> u32 {
        self.denominator
    }
}

/// What kind of committed media file an item names.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EvidenceMediaKind {
    /// An 8-bit RGB PNG image of a frame or crop.
    FramePng,
    /// A 16 kHz mono signed 16-bit WAV clip.
    AudioWav,
}

impl EvidenceMediaKind {
    /// Returns the stable identifier, the session artifact kind's.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::FramePng => "frame_png",
            Self::AudioWav => "audio_wav",
        }
    }
}

/// The content address of the committed media file an item names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceMedia {
    kind: EvidenceMediaKind,
    sha256: Sha256Hex,
    bytes: u64,
}

impl EvidenceMedia {
    /// Describes a committed media file.
    ///
    /// # Errors
    ///
    /// Returns [`EvidenceRecordError::InvalidItem`] for an empty file.
    pub fn new(
        kind: EvidenceMediaKind,
        sha256: Sha256Hex,
        bytes: u64,
    ) -> Result<Self, EvidenceRecordError> {
        if bytes == 0 {
            return Err(EvidenceRecordError::InvalidItem);
        }
        Ok(Self {
            kind,
            sha256,
            bytes,
        })
    }

    /// Returns the media kind.
    #[must_use]
    pub const fn kind(&self) -> EvidenceMediaKind {
        self.kind
    }

    /// Returns the file's SHA-256.
    #[must_use]
    pub const fn sha256(&self) -> &Sha256Hex {
        &self.sha256
    }

    /// Returns the file's size in bytes.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }
}

/// One exactly identified displayed frame of a video stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameRef {
    /// Original index of the video stream.
    pub stream_index: u32,
    /// The stream's time base, which gives `pts` its meaning.
    pub time_base: TimeBase,
    /// Presentation timestamp in the stream's time base, as decoded.
    pub pts: i64,
    /// Normalized presentation time.
    pub time: MediaTime,
    /// Orientation-correct displayed dimensions of the whole frame.
    pub dimensions: FrameDimensions,
}

/// Where a crop lies: in the image it was cut from and in the source frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CropRegion {
    /// The frame or crop item the rectangle was cut from.
    pub parent: EvidenceId,
    /// The rectangle in the parent image's pixels.
    pub rect: CropRect,
    /// The same rectangle in the displayed source frame's pixels, so lineage
    /// always names source pixels ([`CropRect::compose`]).
    pub frame_rect: CropRect,
}

/// What an item shows or plays.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvidenceSubject {
    /// A whole displayed frame.
    Frame(FrameRef),
    /// A rectangle of a displayed frame.
    Crop {
        /// The frame the rectangle is cut from.
        frame: FrameRef,
        /// The rectangle and its lineage.
        region: CropRegion,
    },
    /// An audio clip.
    Audio {
        /// Original index of the audio stream.
        stream_index: u32,
        /// The clipped half-open range the clip was decoded for.
        range: TimeRange,
        /// Normalized time of the first decoded sample.
        actual_start: MediaTime,
    },
}

impl EvidenceSubject {
    /// The media kind that shows this subject.
    #[must_use]
    pub const fn media_kind(&self) -> EvidenceMediaKind {
        match self {
            Self::Frame(_) | Self::Crop { .. } => EvidenceMediaKind::FramePng,
            Self::Audio { .. } => EvidenceMediaKind::AudioWav,
        }
    }

    /// The pixel size of the image, for frames and crops.
    #[must_use]
    pub const fn image_dimensions(&self) -> Option<FrameDimensions> {
        match self {
            Self::Frame(frame) => Some(frame.dimensions),
            Self::Crop { region, .. } => Some(region.frame_rect.dimensions()),
            Self::Audio { .. } => None,
        }
    }

    /// The frame shown, for frames and crops.
    #[must_use]
    pub const fn frame(&self) -> Option<&FrameRef> {
        match self {
            Self::Frame(frame) | Self::Crop { frame, .. } => Some(frame),
            Self::Audio { .. } => None,
        }
    }

    /// The source time the item shows: the frame's time or the clip's first
    /// decoded sample.
    #[must_use]
    pub const fn actual_time(&self) -> MediaTime {
        match self {
            Self::Frame(frame) | Self::Crop { frame, .. } => frame.time,
            Self::Audio { actual_start, .. } => *actual_start,
        }
    }
}

/// The parts of an [`EvidenceItem`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceItemParts {
    /// Identity derived from the facts that fix the item's content.
    pub id: EvidenceId,
    /// What the item shows or plays.
    pub subject: EvidenceSubject,
    /// The committed media file.
    pub media: EvidenceMedia,
    /// The media provider's fingerprint, or `None` when it could not be
    /// computed (then nothing is ever reused for the item).
    pub tool_fingerprint: Option<Sha256Hex>,
    /// How the call that carries this copy of the item checked the source.
    pub source_check: SourceCheck,
}

/// One extracted piece of source evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceItem {
    id: EvidenceId,
    subject: EvidenceSubject,
    media: EvidenceMedia,
    tool_fingerprint: Option<Sha256Hex>,
    source_check: SourceCheck,
}

impl EvidenceItem {
    /// Validates an item's structure.
    ///
    /// # Errors
    ///
    /// Returns [`EvidenceRecordError::InvalidItem`] when the media kind does
    /// not show the subject, a crop rectangle is not inside its frame or
    /// disagrees with its size in the parent, and
    /// [`EvidenceRecordError::AudioRangeTooLong`] for a clip over 30 s.
    pub fn new(parts: EvidenceItemParts) -> Result<Self, EvidenceRecordError> {
        if parts.media.kind() != parts.subject.media_kind() {
            return Err(EvidenceRecordError::InvalidItem);
        }
        match &parts.subject {
            EvidenceSubject::Frame(_) => {}
            EvidenceSubject::Crop { frame, region } => {
                let rect = region.frame_rect;
                CropRect::new(
                    rect.x(),
                    rect.y(),
                    rect.width(),
                    rect.height(),
                    frame.dimensions,
                )
                .map_err(|_| EvidenceRecordError::InvalidItem)?;
                if region.rect.dimensions() != rect.dimensions() {
                    return Err(EvidenceRecordError::InvalidItem);
                }
            }
            EvidenceSubject::Audio { range, .. } => {
                if range.duration_micros() > MAX_AUDIO_CLIP_MICROS {
                    return Err(EvidenceRecordError::AudioRangeTooLong);
                }
            }
        }
        Ok(Self {
            id: parts.id,
            subject: parts.subject,
            media: parts.media,
            tool_fingerprint: parts.tool_fingerprint,
            source_check: parts.source_check,
        })
    }

    /// Returns the item's identity.
    #[must_use]
    pub const fn id(&self) -> &EvidenceId {
        &self.id
    }

    /// Returns what the item shows or plays.
    #[must_use]
    pub const fn subject(&self) -> &EvidenceSubject {
        &self.subject
    }

    /// Returns the committed media file.
    #[must_use]
    pub const fn media(&self) -> &EvidenceMedia {
        &self.media
    }

    /// Returns the media provider's fingerprint, if it could be computed.
    #[must_use]
    pub const fn tool_fingerprint(&self) -> Option<&Sha256Hex> {
        self.tool_fingerprint.as_ref()
    }

    /// Returns how the call that carries this copy checked the source.
    #[must_use]
    pub const fn source_check(&self) -> SourceCheck {
        self.source_check
    }

    /// Whether `other` is the same item: every field but the source check,
    /// which belongs to the call that carries the copy.
    #[must_use]
    pub fn same_content(&self, other: &Self) -> bool {
        self.id == other.id
            && self.subject == other.subject
            && self.media == other.media
            && self.tool_fingerprint == other.tool_fingerprint
    }
}

/// A half-open audio range of at most [`MAX_AUDIO_CLIP_MICROS`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioRange(TimeRange);

impl AudioRange {
    /// Validates an audio range.
    ///
    /// # Errors
    ///
    /// Returns [`EvidenceRecordError::AudioRangeTooLong`] beyond thirty seconds.
    pub const fn new(range: TimeRange) -> Result<Self, EvidenceRecordError> {
        if range.duration_micros() > MAX_AUDIO_CLIP_MICROS {
            Err(EvidenceRecordError::AudioRangeTooLong)
        } else {
            Ok(Self(range))
        }
    }

    /// Returns the requested range.
    #[must_use]
    pub const fn range(self) -> TimeRange {
        self.0
    }
}

/// The canonical parameters of one evidence request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvidenceRequest {
    /// One frame named by a time, or by a visual candidate's representative
    /// time (then at-or-after with tolerance zero).
    FrameAt {
        /// Requested normalized time.
        at: MediaTime,
        /// Selection policy.
        selection: FrameSelection,
        /// How far the frame may lie from `at`.
        tolerance: FrameTolerance,
        /// The candidate the frame was asked for by, if any.
        candidate: Option<VisualCandidateId>,
    },
    /// Consecutive frames on each side of an earlier frame item.
    Neighbours {
        /// The frame item neighbours are taken around.
        anchor: EvidenceId,
        /// Frames per side.
        count: NeighbourCount,
    },
    /// Frames spread evenly over a range.
    Burst {
        /// The requested range.
        range: BurstRange,
        /// Most frames.
        max_frames: BurstCount,
    },
    /// A rectangle of an earlier frame or crop item, in that item's pixels.
    Crop {
        /// The frame or crop item cut from.
        parent: EvidenceId,
        /// The rectangle in the parent's pixels.
        rect: CropRect,
    },
    /// An audio clip.
    Audio {
        /// The requested range.
        range: AudioRange,
    },
}

impl EvidenceRequest {
    /// Returns the operation the request names.
    #[must_use]
    pub const fn operation(&self) -> EvidenceOperation {
        match self {
            Self::FrameAt { .. } => EvidenceOperation::FrameGet,
            Self::Neighbours { .. } => EvidenceOperation::FrameNeighbours,
            Self::Burst { .. } => EvidenceOperation::FrameBurst,
            Self::Crop { .. } => EvidenceOperation::Crop,
            Self::Audio { .. } => EvidenceOperation::Audio,
        }
    }

    /// The earlier item the request is taken from: a neighbours anchor or a
    /// crop parent.
    #[must_use]
    pub const fn parent(&self) -> Option<&EvidenceId> {
        match self {
            Self::Neighbours { anchor, .. } => Some(anchor),
            Self::Crop { parent, .. } => Some(parent),
            Self::FrameAt { .. } | Self::Burst { .. } | Self::Audio { .. } => None,
        }
    }
}

/// Why an item was selected.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SelectionRole {
    /// The item a single-item request named (`frame get`, `crop`, `audio`).
    Requested,
    /// A neighbour before the anchor.
    Before,
    /// A neighbour after the anchor.
    After,
    /// The frame a burst target named.
    Target,
}

impl SelectionRole {
    /// Every role.
    pub const ALL: [Self; 4] = [Self::Requested, Self::Before, Self::After, Self::Target];

    /// Returns the stable identifier used in records.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Before => "before",
            Self::After => "after",
            Self::Target => "target",
        }
    }
}

/// Which item a requested time resolved to.
///
/// `requested` is the time the role refers to: the requested time of a
/// frame, the parent frame's time of a crop, the anchor's time of a
/// neighbour, a burst's target time, and a clip's requested start.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceSelection {
    role: SelectionRole,
    evidence: EvidenceId,
    timing: FrameTiming,
}

impl EvidenceSelection {
    /// Records a selection.
    ///
    /// # Errors
    ///
    /// Returns [`EvidenceRecordError::InvalidTiming`] when the delta does not
    /// fit a signed 64-bit value.
    pub fn new(
        role: SelectionRole,
        evidence: EvidenceId,
        requested: MediaTime,
        actual: MediaTime,
    ) -> Result<Self, EvidenceRecordError> {
        Ok(Self {
            role,
            evidence,
            timing: FrameTiming::new(requested, actual)
                .map_err(|_| EvidenceRecordError::InvalidTiming)?,
        })
    }

    /// Returns why the item was selected.
    #[must_use]
    pub const fn role(&self) -> SelectionRole {
        self.role
    }

    /// Returns the selected item.
    #[must_use]
    pub const fn evidence(&self) -> &EvidenceId {
        &self.evidence
    }

    /// Returns the requested and actual time and their delta.
    #[must_use]
    pub const fn timing(&self) -> FrameTiming {
        self.timing
    }
}

/// What a call learned about its request beyond the items.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceDetail {
    /// A single frame or crop.
    Single,
    /// Neighbours, with why a side holds fewer frames than asked.
    Neighbours {
        /// Why the frames before the anchor ran short, if they did.
        before_stop: Option<NeighbourStop>,
        /// Why the frames after the anchor ran short, if they did.
        after_stop: Option<NeighbourStop>,
    },
    /// A burst's plan.
    Burst {
        /// The range the targets were spread over.
        planned: TimeRange,
        /// Whether `planned` is the requested range or its clipped part.
        extent: BurstExtent,
        /// Number of evenly spaced targets.
        targets: u8,
        /// Number of distinct frames the targets named.
        distinct: u8,
    },
    /// An audio clip.
    Audio {
        /// Whether the requested range ran past the source and was clipped.
        range_clipped: bool,
    },
}

/// Why a call returned fewer items than its request named.
///
/// Items extracted before the stop are committed and returned with the
/// reason; a call that extracted nothing fails instead.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PartialReason {
    /// The per-call frame budget.
    FrameBudget,
    /// The per-call decoded-pixel budget.
    PixelBudget,
    /// The per-call or per-session byte budget.
    ByteBudget,
    /// The session's evidence artifact budget.
    SessionEvidenceBudget,
    /// The call's deadline.
    DeadlineExceeded,
    /// The caller cancelled.
    Cancelled,
}

impl PartialReason {
    /// Every reason.
    pub const ALL: [Self; 6] = [
        Self::FrameBudget,
        Self::PixelBudget,
        Self::ByteBudget,
        Self::SessionEvidenceBudget,
        Self::DeadlineExceeded,
        Self::Cancelled,
    ];

    /// Returns the stable identifier used in records and results.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::FrameBudget => "frame_budget",
            Self::PixelBudget => "pixel_budget",
            Self::ByteBudget => "byte_budget",
            Self::SessionEvidenceBudget => "session_evidence_budget",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Cancelled => "cancelled",
        }
    }
}

/// The parts of an [`EvidenceRecord`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceRecordParts {
    /// The request key: a digest of everything that decides the result.
    pub request_key: OperationKey,
    /// Session the record belongs to.
    pub session_id: SessionId,
    /// Source the items were extracted from.
    pub source_id: SourceId,
    /// Adapter profile.
    pub profile: EvidenceProfile,
    /// The media provider's fingerprint, if it could be computed.
    pub tool_fingerprint: Option<Sha256Hex>,
    /// The request's canonical parameters.
    pub request: EvidenceRequest,
    /// Which item each requested time resolved to.
    pub selections: Vec<EvidenceSelection>,
    /// Operation-specific detail.
    pub detail: EvidenceDetail,
    /// The distinct items, in extraction order.
    pub items: Vec<EvidenceItem>,
    /// Why the call stopped short, if it did.
    pub partial: Option<PartialReason>,
}

/// The lineage of one extracting evidence call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceRecord {
    parts: EvidenceRecordParts,
}

impl EvidenceRecord {
    /// Validates a record's structure.
    ///
    /// # Errors
    ///
    /// Returns an [`EvidenceRecordError`] when the record is empty or over
    /// its bounds, an item repeats or is never selected, a selection names
    /// no item or disagrees with its item's time, the detail, roles or item
    /// kinds do not fit the operation, or an item's fingerprint differs from
    /// the record's.
    pub fn new(parts: EvidenceRecordParts) -> Result<Self, EvidenceRecordError> {
        validate_record(&parts)?;
        Ok(Self { parts })
    }

    /// Returns the request key.
    #[must_use]
    pub const fn request_key(&self) -> &OperationKey {
        &self.parts.request_key
    }

    /// Returns the session.
    #[must_use]
    pub const fn session_id(&self) -> &SessionId {
        &self.parts.session_id
    }

    /// Returns the source.
    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.parts.source_id
    }

    /// Returns the adapter profile.
    #[must_use]
    pub const fn profile(&self) -> EvidenceProfile {
        self.parts.profile
    }

    /// Returns the provider fingerprint, if it could be computed.
    #[must_use]
    pub const fn tool_fingerprint(&self) -> Option<&Sha256Hex> {
        self.parts.tool_fingerprint.as_ref()
    }

    /// Returns the request.
    #[must_use]
    pub const fn request(&self) -> &EvidenceRequest {
        &self.parts.request
    }

    /// Returns the selections.
    #[must_use]
    pub fn selections(&self) -> &[EvidenceSelection] {
        &self.parts.selections
    }

    /// Returns the operation-specific detail.
    #[must_use]
    pub const fn detail(&self) -> EvidenceDetail {
        self.parts.detail
    }

    /// Returns the items.
    #[must_use]
    pub fn items(&self) -> &[EvidenceItem] {
        &self.parts.items
    }

    /// Returns the item with `id`, if the record holds it.
    #[must_use]
    pub fn item(&self, id: &EvidenceId) -> Option<&EvidenceItem> {
        self.parts.items.iter().find(|item| item.id() == id)
    }

    /// Returns why the call stopped short, if it did.
    #[must_use]
    pub const fn partial(&self) -> Option<PartialReason> {
        self.parts.partial
    }
}

fn validate_record(parts: &EvidenceRecordParts) -> Result<(), EvidenceRecordError> {
    let items = &parts.items;
    let selections = &parts.selections;
    if items.is_empty() || selections.is_empty() {
        return Err(EvidenceRecordError::Empty);
    }
    if items.len() > MAX_RECORD_ITEMS || selections.len() > MAX_RECORD_SELECTIONS {
        return Err(EvidenceRecordError::TooLarge);
    }
    for (index, item) in items.iter().enumerate() {
        if items
            .iter()
            .skip(index + 1)
            .any(|later| later.id() == item.id())
        {
            return Err(EvidenceRecordError::DuplicateItem);
        }
        if item.tool_fingerprint() != parts.tool_fingerprint.as_ref() {
            return Err(EvidenceRecordError::InvalidItem);
        }
        if !selections
            .iter()
            .any(|selection| selection.evidence() == item.id())
        {
            return Err(EvidenceRecordError::UnselectedItem);
        }
    }
    for selection in selections {
        let item = items
            .iter()
            .find(|item| item.id() == selection.evidence())
            .ok_or(EvidenceRecordError::UnknownSelection)?;
        if item.subject().actual_time() != selection.timing().actual() {
            return Err(EvidenceRecordError::InvalidTiming);
        }
    }
    validate_operation(parts)
}

/// Checks that the detail, roles and item kinds fit the request.
fn validate_operation(parts: &EvidenceRecordParts) -> Result<(), EvidenceRecordError> {
    let roles_are = |allowed: &[SelectionRole]| {
        parts
            .selections
            .iter()
            .all(|selection| allowed.contains(&selection.role()))
    };
    let single = parts.selections.len() == 1 && parts.items.len() == 1;
    let all_frames = parts
        .items
        .iter()
        .all(|item| matches!(item.subject(), EvidenceSubject::Frame(_)));
    let fits = match (&parts.request, parts.detail) {
        (EvidenceRequest::FrameAt { at, .. }, EvidenceDetail::Single) => {
            single
                && all_frames
                && roles_are(&[SelectionRole::Requested])
                && parts
                    .selections
                    .iter()
                    .all(|selection| selection.timing().requested() == *at)
        }
        (EvidenceRequest::Neighbours { count, .. }, EvidenceDetail::Neighbours { .. }) => {
            all_frames
                && roles_are(&[SelectionRole::Before, SelectionRole::After])
                && parts.selections.len() <= 2 * usize::from(count.get())
        }
        (
            EvidenceRequest::Burst { range, max_frames },
            EvidenceDetail::Burst {
                planned,
                extent,
                targets,
                distinct,
            },
        ) => {
            let requested = range.range();
            let planned_fits = planned.start() == requested.start()
                && planned.end() <= requested.end()
                && (extent == BurstExtent::ClippedAtEndOfStream || planned == requested);
            all_frames
                && roles_are(&[SelectionRole::Target])
                && planned_fits
                && targets == max_frames.get()
                && distinct <= targets
                && parts.items.len() <= usize::from(distinct)
                && parts.selections.len() <= usize::from(targets)
                && parts.selections.iter().all(|selection| {
                    let target = selection.timing().requested();
                    target >= planned.start() && target < planned.end()
                })
        }
        (EvidenceRequest::Crop { parent, rect }, EvidenceDetail::Single) => {
            single
                && roles_are(&[SelectionRole::Requested])
                && parts.items.iter().all(|item| {
                    matches!(
                        item.subject(),
                        EvidenceSubject::Crop { region, .. }
                            if region.parent == *parent && region.rect == *rect
                    )
                })
        }
        (EvidenceRequest::Audio { range }, EvidenceDetail::Audio { range_clipped }) => {
            single
                && roles_are(&[SelectionRole::Requested])
                && parts.items.iter().all(|item| match item.subject() {
                    EvidenceSubject::Audio { range: clipped, .. } => {
                        clipped.start() == range.range().start()
                            && clipped.end() <= range.range().end()
                            && range_clipped == (clipped.end() < range.range().end())
                    }
                    EvidenceSubject::Frame(_) | EvidenceSubject::Crop { .. } => false,
                })
                && parts
                    .selections
                    .iter()
                    .all(|selection| selection.timing().requested() == range.range().start())
        }
        _ => false,
    };
    if fits {
        Ok(())
    } else {
        Err(EvidenceRecordError::OperationMismatch)
    }
}

/// Why an evidence item or record was rejected.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EvidenceRecordError {
    /// An item's fields are inconsistent (media kind, geometry, fingerprint).
    InvalidItem,
    /// An audio range is longer than thirty seconds.
    AudioRangeTooLong,
    /// A record holds no item or no selection.
    Empty,
    /// A record holds more items or selections than its bound.
    TooLarge,
    /// Two items of a record have one identity.
    DuplicateItem,
    /// An item no selection names.
    UnselectedItem,
    /// A selection names an item the record does not hold.
    UnknownSelection,
    /// A selection's time disagrees with its item, or its delta overflows.
    InvalidTiming,
    /// The detail, roles or item kinds do not fit the request's operation.
    OperationMismatch,
}

impl fmt::Display for EvidenceRecordError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidItem => "evidence item is inconsistent",
            Self::AudioRangeTooLong => "audio range exceeds thirty seconds",
            Self::Empty => "evidence record holds no item",
            Self::TooLarge => "evidence record exceeds its bounds",
            Self::DuplicateItem => "evidence record repeats an item",
            Self::UnselectedItem => "evidence record holds an item no selection names",
            Self::UnknownSelection => "evidence selection names an item the record lacks",
            Self::InvalidTiming => "evidence selection time disagrees with its item",
            Self::OperationMismatch => "evidence record does not fit its operation",
        })
    }
}

impl Error for EvidenceRecordError {}

#[cfg(test)]
mod tests;
