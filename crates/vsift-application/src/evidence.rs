//! Evidence navigation use cases (P09 PR 2, ADR 0019): exact frames, their
//! neighbours, bursts, crops and audio clips, with request keys, item
//! identities, reuse and lineage.
//!
//! # Identities
//!
//! Two identities are derived with SHA-256, like transcript and visual
//! identities:
//!
//! - The **request key** ([`evidence_request_key`], an `opk_sha256_` value)
//!   digests everything that decides a call's result: the session and
//!   source, the stream selector, the operation and its canonical parameters,
//!   the adapter profile and the media provider's fingerprint. A later call
//!   with the same key can be answered from the committed record without
//!   running a provider (V-08); another provider or profile is another key.
//! - The **item identity** ([`evidence_id`], an `evd_` value) digests only
//!   the facts that fix the item's pixels or samples: the session and
//!   source, the stream, the profile, the provider fingerprint, the exact
//!   frame timestamp and time base, and the crop region or clipped audio
//!   range. Requested times, policies and candidates are request facts, so
//!   two requests that resolve to one frame share one item. When the
//!   provider's fingerprint cannot be computed the item identity also
//!   digests the media's own SHA-256, so items from unidentified tools never
//!   collide, and nothing is reused for them.
//!
//! # Calls
//!
//! A call lists the displayed frames it needs through the [`FrameExtractor`]
//! port, chooses frames with the domain's navigation rules, extracts exactly
//! those timestamps and builds an [`EvidenceRecord`]. Every call is bounded
//! by an [`EvidenceBudget`] (frames, decoded pixels, image bytes and the
//! session's evidence slots) and an [`EvidenceControl`] (deadline and
//! cancellation). Hitting a budget, the deadline or a cancellation after
//! some frames were extracted returns those frames with a
//! [`PartialReason`]; with nothing extracted the call fails.

use std::{collections::BTreeSet, error::Error, fmt, future::Future};

use vsift_domain::{
    CropRect, EvidenceId, EvidenceMedia, EvidenceMediaKind, EvidenceProfile, EvidenceRecord,
    EvidenceRecordError, EvidenceRequest, EvidenceSubject, FrameDimensions, FrameListing,
    FrameSelectionError, MediaTime, OperationKey, SessionId, Sha256Hex, SourceId, TimeBase,
    TimeRange,
};

use crate::transcript::{derived_identity, sha256_hex};

mod extract;

pub use extract::{
    CropRequest, FrameAtRequest, extract_audio, extract_burst, extract_crop, extract_frame_at,
    extract_neighbours,
};

/// Most frames one call returns.
pub const MAX_FRAMES_PER_CALL: usize = 100;
/// Most decoded pixels one call extracts: two hundred megapixels.
pub const MAX_PIXELS_PER_CALL: u64 = 200_000_000;
/// Most image bytes one call returns: 256 MiB.
pub const MAX_IMAGE_BYTES_PER_CALL: u64 = 256 * 1024 * 1024;

/// The session, source, profile and provider an evidence identity is
/// derived in.
#[derive(Clone, Copy, Debug)]
pub struct EvidenceScope<'a> {
    /// Session the evidence belongs to.
    pub session_id: &'a SessionId,
    /// Source the evidence is extracted from.
    pub source_id: &'a SourceId,
    /// Adapter profile.
    pub profile: EvidenceProfile,
    /// The media provider's fingerprint, when it could be computed.
    pub tool_fingerprint: Option<&'a Sha256Hex>,
}

/// Why an evidence identity could not be derived or did not re-derive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceIdentityError {
    /// A derived value was not canonical; an internal fault.
    NotCanonical,
    /// A record's request key or an item identity is not the one its
    /// content derives.
    Mismatch,
}

impl fmt::Display for EvidenceIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotCanonical => "evidence identity could not be derived",
            Self::Mismatch => "evidence identity does not match its content",
        })
    }
}

impl Error for EvidenceIdentityError {}

/// The stream selector a request key digests.
///
/// Keys are derived before the source is probed, so they name the rule that
/// selects a stream, not the stream index it selects.
const fn stream_selector(request: &EvidenceRequest) -> &'static str {
    match request {
        EvidenceRequest::Audio { .. } => "audio:speech_default",
        EvidenceRequest::FrameAt { .. }
        | EvidenceRequest::Neighbours { .. }
        | EvidenceRequest::Burst { .. }
        | EvidenceRequest::Crop { .. } => "video:first_decodable",
    }
}

/// The canonical parameter lines of a request.
fn canonical_parameters(request: &EvidenceRequest) -> Vec<String> {
    match request {
        EvidenceRequest::FrameAt {
            at,
            selection,
            tolerance,
            candidate,
        } => vec![
            format!("at_us={}", at.as_micros()),
            format!("policy={}", selection.identifier()),
            format!("tolerance_us={}", tolerance.as_micros()),
            format!(
                "candidate={}",
                candidate.as_ref().map_or("none", |id| id.as_str())
            ),
        ],
        EvidenceRequest::Neighbours { anchor, count } => vec![
            format!("anchor={}", anchor.as_str()),
            format!("count={}", count.get()),
        ],
        EvidenceRequest::Burst { range, max_frames } => vec![
            format!("from_us={}", range.range().start().as_micros()),
            format!("to_us={}", range.range().end().as_micros()),
            format!("max_frames={}", max_frames.get()),
        ],
        EvidenceRequest::Crop { parent, rect } => vec![
            format!("parent={}", parent.as_str()),
            format!("rect={}", rect_text(*rect)),
        ],
        EvidenceRequest::Audio { range } => vec![
            format!("from_us={}", range.range().start().as_micros()),
            format!("to_us={}", range.range().end().as_micros()),
        ],
    }
}

fn rect_text(rect: CropRect) -> String {
    format!(
        "{},{},{},{}",
        rect.x(),
        rect.y(),
        rect.width(),
        rect.height()
    )
}

fn fingerprint_text(fingerprint: Option<&Sha256Hex>) -> &str {
    fingerprint.map_or("none", Sha256Hex::as_str)
}

/// Derives the request key of `request` in `scope`.
///
/// # Errors
///
/// Returns [`EvidenceIdentityError::NotCanonical`] only if the digest were
/// not canonical, which would be an internal fault.
pub fn evidence_request_key(
    scope: &EvidenceScope<'_>,
    request: &EvidenceRequest,
) -> Result<OperationKey, EvidenceIdentityError> {
    let mut material = String::from("vsift.evidence-request.v1");
    for line in [
        scope.session_id.as_str(),
        scope.source_id.as_str(),
        stream_selector(request),
        request.operation().identifier(),
    ] {
        material.push('\n');
        material.push_str(line);
    }
    for line in canonical_parameters(request) {
        material.push('\n');
        material.push_str(&line);
    }
    for line in [
        scope.profile.identifier(),
        fingerprint_text(scope.tool_fingerprint),
    ] {
        material.push('\n');
        material.push_str(line);
    }
    OperationKey::from_sha256(&sha256_hex(material.as_bytes()))
        .map_err(|_| EvidenceIdentityError::NotCanonical)
}

/// Derives the identity of the item that shows `subject` with `media`.
///
/// # Errors
///
/// Returns [`EvidenceIdentityError::NotCanonical`] only if the derived
/// value were not canonical, which would be an internal fault.
pub fn evidence_id(
    scope: &EvidenceScope<'_>,
    subject: &EvidenceSubject,
    media: &EvidenceMedia,
) -> Result<EvidenceId, EvidenceIdentityError> {
    let mut parts: Vec<String> = vec![
        scope.session_id.as_str().to_owned(),
        scope.source_id.as_str().to_owned(),
        scope.profile.identifier().to_owned(),
        fingerprint_text(scope.tool_fingerprint).to_owned(),
    ];
    match subject {
        EvidenceSubject::Frame(frame) => {
            parts.push("frame".to_owned());
            parts.extend(frame_parts(frame.stream_index, frame.time_base, frame.pts));
        }
        EvidenceSubject::Crop { frame, region } => {
            parts.push("crop".to_owned());
            parts.extend(frame_parts(frame.stream_index, frame.time_base, frame.pts));
            parts.push(region.parent.as_str().to_owned());
            parts.push(rect_text(region.rect));
            parts.push(rect_text(region.frame_rect));
        }
        EvidenceSubject::Audio {
            stream_index,
            range,
            ..
        } => {
            parts.push("audio".to_owned());
            parts.push(stream_index.to_string());
            parts.push(range.start().as_micros().to_string());
            parts.push(range.end().as_micros().to_string());
        }
    }
    // Without a provider fingerprint nothing says which tool made the
    // bytes, so the bytes themselves are part of the identity.
    if scope.tool_fingerprint.is_none() {
        parts.push(format!("content={}", media.sha256().as_str()));
    }
    let borrowed: Vec<&str> = parts.iter().map(String::as_str).collect();
    EvidenceId::parse(derived_identity(
        "evd_",
        "vsift.evidence-item.v1",
        &borrowed,
    ))
    .map_err(|_| EvidenceIdentityError::NotCanonical)
}

fn frame_parts(stream_index: u32, time_base: TimeBase, pts: i64) -> [String; 3] {
    [
        stream_index.to_string(),
        format!("{}/{}", time_base.numerator(), time_base.denominator()),
        pts.to_string(),
    ]
}

/// Checks that a record's request key and every item identity are the ones
/// their content derives.
///
/// A stored record is decoded structurally by the domain; this closes the
/// remaining gap, so a record whose identities were edited is rejected.
///
/// # Errors
///
/// Returns [`EvidenceIdentityError::Mismatch`] for any identity that does
/// not re-derive.
pub fn verify_evidence_record(record: &EvidenceRecord) -> Result<(), EvidenceIdentityError> {
    let scope = EvidenceScope {
        session_id: record.session_id(),
        source_id: record.source_id(),
        profile: record.profile(),
        tool_fingerprint: record.tool_fingerprint(),
    };
    if evidence_request_key(&scope, record.request())? != *record.request_key() {
        return Err(EvidenceIdentityError::Mismatch);
    }
    for item in record.items() {
        if evidence_id(&scope, item.subject(), item.media())? != *item.id() {
            return Err(EvidenceIdentityError::Mismatch);
        }
    }
    Ok(())
}

/// Returns the committed record a call with `key` can be answered from.
///
/// Only a complete record is reused: a partial one stopped on a budget,
/// deadline or cancellation that a new call might not meet. No record is
/// reused when the provider's fingerprint is unknown, because then nothing
/// proves the same tools would produce the same bytes. The newest match is
/// returned.
#[must_use]
pub fn find_reusable_record<'a>(
    records: &'a [EvidenceRecord],
    key: &OperationKey,
    tool_fingerprint: Option<&Sha256Hex>,
) -> Option<&'a EvidenceRecord> {
    tool_fingerprint?;
    records
        .iter()
        .rev()
        .find(|record| record.request_key() == key && record.partial().is_none())
}

/// Returns the item with `id` from the newest record that holds it.
#[must_use]
pub fn find_evidence_item<'a>(
    records: &'a [EvidenceRecord],
    id: &EvidenceId,
) -> Option<&'a vsift_domain::EvidenceItem> {
    records.iter().rev().find_map(|record| record.item(id))
}

/// Facts about the video stream evidence is taken from, from the probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoStreamFacts {
    /// Original stream index.
    pub stream_index: u32,
    /// The stream's time base.
    pub time_base: TimeBase,
    /// Orientation-correct displayed dimensions.
    pub displayed: FrameDimensions,
    /// Probed normalized source duration.
    pub duration: MediaTime,
}

/// One extracted image of an exactly identified frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractedFrame {
    /// The frame's stream timestamp.
    pub pts: i64,
    /// The frame's normalized time.
    pub time: MediaTime,
    /// 8-bit RGB PNG bytes, checked by the adapter.
    pub png: Vec<u8>,
}

/// One extracted audio clip.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractedClip {
    /// Normalized time of the first decoded sample.
    pub actual_start: MediaTime,
    /// A canonical 16 kHz mono signed 16-bit WAV file.
    pub wav: Vec<u8>,
}

/// Why a media provider run for evidence failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceMediaError {
    /// Admission capacity was unavailable; retryable.
    Busy,
    /// The run exceeded its deadline; retryable.
    Deadline,
    /// The caller cancelled; retryable.
    Cancelled,
    /// The session's source copy changed.
    SourceChanged,
    /// A listed timestamp decoded no frame on extraction.
    FrameNotFound,
    /// The selected audio stream decoded no sample in the range.
    NoDecodedAudio,
    /// The provider rejected or could not decode the media, or its output
    /// was malformed.
    Undecodable,
    /// Provider output exceeded its bounds.
    ResourceLimit,
    /// The provider could not run.
    Unavailable,
    /// The adapter rejected the request; an internal fault.
    Invalid,
}

impl fmt::Display for EvidenceMediaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Busy => "media admission is busy",
            Self::Deadline => "a media run exceeded its deadline",
            Self::Cancelled => "a media run was cancelled",
            Self::SourceChanged => "the session's source copy changed",
            Self::FrameNotFound => "a listed frame could not be extracted",
            Self::NoDecodedAudio => "the range holds no decodable audio",
            Self::Undecodable => "the media tools could not decode the source",
            Self::ResourceLimit => "media output exceeded its bounds",
            Self::Unavailable => "the media tools could not run",
            Self::Invalid => "the media adapter rejected the request",
        })
    }
}

impl Error for EvidenceMediaError {}

/// Port that lists and extracts frames of one probed video stream of the
/// bound source.
pub trait FrameExtractor: Send + Sync {
    /// The stream frames are taken from.
    fn stream(&self) -> VideoStreamFacts;

    /// Most frames one [`Self::frames`] run may return for this stream.
    fn max_frames_per_run(&self) -> usize;

    /// Lists every displayed frame whose time lies in `range` (clipped to
    /// the source), at most sixty seconds.
    fn list_frames(
        &self,
        range: TimeRange,
    ) -> impl Future<Output = Result<FrameListing, EvidenceMediaError>> + Send;

    /// Extracts the frames with exactly these strictly increasing
    /// timestamps, in timestamp order.
    fn frames(
        &self,
        pts: &[i64],
    ) -> impl Future<Output = Result<Vec<ExtractedFrame>, EvidenceMediaError>> + Send;

    /// Extracts `rect` (in displayed frame pixels) of the frame with this
    /// timestamp.
    fn crop(
        &self,
        pts: i64,
        rect: CropRect,
    ) -> impl Future<Output = Result<ExtractedFrame, EvidenceMediaError>> + Send;
}

/// Port that decodes audio clips of one probed audio stream of the bound
/// source.
pub trait AudioExtractor: Send + Sync {
    /// Original index of the audio stream.
    fn stream_index(&self) -> u32;

    /// Probed normalized source duration.
    fn duration(&self) -> MediaTime;

    /// Decodes `range` (inside the source, at most thirty seconds).
    fn clip(
        &self,
        range: TimeRange,
    ) -> impl Future<Output = Result<ExtractedClip, EvidenceMediaError>> + Send;
}

/// Why a call stopped between provider runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceStop {
    /// The call's deadline passed.
    DeadlineExceeded,
    /// The caller cancelled.
    Cancelled,
}

/// Port asked before every provider run whether the call must stop.
pub trait EvidenceControl: Send + Sync {
    /// Returns why the call must stop, or `None` to continue.
    fn stop(&self) -> Option<EvidenceStop>;
}

/// What one call may extract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvidenceBudget {
    /// Most frames.
    pub max_frames: usize,
    /// Most decoded pixels.
    pub max_pixels: u64,
    /// Most media bytes: the per-call image bound or what the session has
    /// left, whichever is smaller.
    pub max_bytes: u64,
    /// Evidence artifacts the session can still take, including this call's
    /// record.
    pub session_slots: usize,
}

impl EvidenceBudget {
    /// The per-call budget of ADR 0019 with the session's remaining
    /// evidence slots and bytes.
    #[must_use]
    pub const fn per_call(session_slots: usize, session_bytes_left: u64) -> Self {
        Self {
            max_frames: MAX_FRAMES_PER_CALL,
            max_pixels: MAX_PIXELS_PER_CALL,
            max_bytes: if session_bytes_left < MAX_IMAGE_BYTES_PER_CALL {
                session_bytes_left
            } else {
                MAX_IMAGE_BYTES_PER_CALL
            },
            session_slots,
        }
    }
}

/// Everything a call needs besides its request and extractor.
pub struct EvidenceCall<'a, C> {
    /// Identity scope.
    pub scope: EvidenceScope<'a>,
    /// How the call checked the source copy (ADR 0019 D1).
    pub source_check: vsift_domain::SourceCheck,
    /// Budget.
    pub budget: EvidenceBudget,
    /// SHA-256 digests of media the session already holds; extracting one
    /// again takes no new slot.
    pub known_media: &'a BTreeSet<String>,
    /// Deadline and cancellation.
    pub control: &'a C,
}

/// One media file a call extracted that the session does not hold yet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractedMedia {
    /// What the file is.
    pub kind: EvidenceMediaKind,
    /// The file's SHA-256.
    pub sha256: Sha256Hex,
    /// The file.
    pub bytes: Vec<u8>,
}

/// What an extracting call produced: its record and the new media files to
/// commit with it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceExtraction {
    /// The call's lineage record.
    pub record: EvidenceRecord,
    /// Media files the session does not hold yet, each once.
    pub media: Vec<ExtractedMedia>,
}

/// Why an evidence call failed. Nothing is committed for a failed call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceError {
    /// The listing could not name a frame for the request.
    Selection(FrameSelectionError),
    /// A provider run failed with nothing extracted, or in a way that
    /// invalidates what was.
    Media(EvidenceMediaError),
    /// The call stopped before it extracted anything.
    Stopped(EvidenceStop),
    /// Not even one item fits the budget or the session's evidence slots.
    BudgetExhausted,
    /// A visual candidate's frame is not at its representative time.
    CandidateMismatch,
    /// A parent item does not describe the probed stream, or its frame is
    /// not where the item says.
    ParentMismatch,
    /// A parent item is of a kind the operation cannot use.
    KindMismatch,
    /// A crop rectangle is not contained by its parent image.
    CropOutsideParent,
    /// A range starts at or after the end of the source.
    RangeOutsideSource,
    /// The record could not be built; an internal fault.
    Record(EvidenceRecordError),
    /// An identity could not be derived; an internal fault.
    Identity(EvidenceIdentityError),
}

impl fmt::Display for EvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Selection(error) => error.fmt(formatter),
            Self::Media(error) => error.fmt(formatter),
            Self::Stopped(EvidenceStop::DeadlineExceeded) => {
                formatter.write_str("the evidence call exceeded its deadline")
            }
            Self::Stopped(EvidenceStop::Cancelled) => {
                formatter.write_str("the evidence call was cancelled")
            }
            Self::BudgetExhausted => formatter.write_str("no evidence fits the remaining budget"),
            Self::CandidateMismatch => {
                formatter.write_str("the candidate's frame is not at its representative time")
            }
            Self::ParentMismatch => {
                formatter.write_str("the parent evidence does not describe the source stream")
            }
            Self::KindMismatch => formatter.write_str("the parent evidence is of another kind"),
            Self::CropOutsideParent => {
                formatter.write_str("the crop rectangle is outside the parent image")
            }
            Self::RangeOutsideSource => {
                formatter.write_str("the range starts at or after the end of the source")
            }
            Self::Record(error) => error.fmt(formatter),
            Self::Identity(error) => error.fmt(formatter),
        }
    }
}

impl Error for EvidenceError {}

/// The SHA-256 of media bytes as a domain digest.
fn media_digest(bytes: &[u8]) -> Result<Sha256Hex, EvidenceError> {
    Sha256Hex::parse(sha256_hex(bytes))
        .map_err(|_| EvidenceError::Identity(EvidenceIdentityError::NotCanonical))
}

#[cfg(test)]
mod tests;
