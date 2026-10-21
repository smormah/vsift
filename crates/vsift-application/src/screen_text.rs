//! Reading on-screen text off a session's frames (OCR, ADR 0030).
//!
//! [`read_screen_text_within`] resolves a plan's pending read targets to exact
//! displayed frames, extracts them through the media port, has the OCR engine
//! read them through [`ScreenTextReader`], and returns the frames read and the
//! targets that resolved to no frame. [`merge_screen_text`] folds those into the
//! next revision of the session's index. Nothing here commits: the caller does,
//! and a call that stops early still returns what it read.
//!
//! Engine output is untrusted (SEC-28): a line whose region leaves the frame or
//! whose text is empty, too long or holds control characters is not repaired;
//! the frame's reading is recorded as [`ScreenTextFailure::EngineFailed`].

use std::{error::Error, fmt, future::Future, num::NonZeroU32};

use vsift_domain::{
    Confidence, ConfidenceOrigin, FrameDimensions, FrameRef, FrameSelection, FrameTolerance,
    ListedFrame, MAX_SCREEN_TEXT_LINES_PER_FRAME, MediaTime, ReadTarget, ScreenText,
    ScreenTextError, ScreenTextFailure, ScreenTextFrame, ScreenTextIndex, ScreenTextIndexId,
    ScreenTextIndexParts, ScreenTextLine, ScreenTextLineId, ScreenTextOutcome, ScreenTextProvider,
    ScreenTextScope, TextRegion, TimeRange, select_frame,
};

use crate::{
    EvidenceControl, EvidenceMediaError, EvidenceStop, FrameExtractor, VideoStreamFacts,
    transcript::derived_identity,
};

/// The longest stretch one frame listing covers when several targets share it.
pub const MAX_SHARED_LISTING_MICROS: u64 = 20_000_000;
/// Most frames one call reads.
pub const MAX_SCREEN_TEXT_FRAMES_PER_CALL: u16 = 200;
/// Frames one call reads unless the caller chooses.
pub const DEFAULT_SCREEN_TEXT_FRAMES_PER_CALL: u16 = 40;

/// One frame image handed to the engine.
#[derive(Clone, Copy, Debug)]
pub struct ScreenTextImage<'a> {
    /// The frame's 8-bit RGB PNG, as the media port extracted it.
    pub png: &'a [u8],
    /// Its displayed dimensions.
    pub dimensions: FrameDimensions,
}

/// One line as the engine reported it, before it is checked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineLine {
    /// Left edge in the frame's pixels.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// The text read.
    pub text: String,
    /// The engine's score in basis points (0 to 10,000).
    pub confidence_basis_points: u16,
}

/// What the engine reported for one image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineReading {
    /// The image was read.
    Lines {
        /// The lines, in reading order.
        lines: Vec<EngineLine>,
        /// True when the image held more lines than the engine reported.
        truncated: bool,
    },
    /// The engine refused the image.
    Rejected,
    /// The engine ran but could not read this image.
    Failed,
}

/// Why the engine could not read a batch at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenTextReadError {
    /// The engine is not configured or cannot be started.
    Unavailable,
    /// Admission capacity was unavailable for the engine's threads.
    Busy,
    /// Its runtime or a model could not be loaded.
    ModelUnavailable,
    /// The run exceeded its deadline.
    Deadline,
    /// The caller cancelled.
    Cancelled,
    /// The run's output exceeded its bounds.
    ResourceLimit,
    /// The engine ended with a failure of its own.
    ProviderFailed,
    /// The engine's output did not follow its protocol.
    UnparseableOutput,
    /// The engine was killed by the system or crashed.
    AbnormalTermination,
    /// The private folder the images are handed over in could not be used.
    Workspace,
}

impl ScreenTextReadError {
    /// Every failure.
    pub const ALL: [Self; 10] = [
        Self::Unavailable,
        Self::Busy,
        Self::ModelUnavailable,
        Self::Deadline,
        Self::Cancelled,
        Self::ResourceLimit,
        Self::ProviderFailed,
        Self::UnparseableOutput,
        Self::AbnormalTermination,
        Self::Workspace,
    ];

    /// Stable identifier, as public results name it.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Unavailable => "engine_unavailable",
            Self::Busy => "busy",
            Self::ModelUnavailable => "model_unavailable",
            Self::Deadline => "deadline",
            Self::Cancelled => "cancelled",
            Self::ResourceLimit => "resource_limit",
            Self::ProviderFailed => "provider_failed",
            Self::UnparseableOutput => "unparseable_output",
            Self::AbnormalTermination => "abnormal_termination",
            Self::Workspace => "workspace",
        }
    }
}

impl fmt::Display for ScreenTextReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "the OCR engine is not available",
            Self::Busy => "admission for the OCR engine is busy",
            Self::ModelUnavailable => "the OCR engine could not load its runtime or models",
            Self::Deadline => "the OCR engine exceeded its deadline",
            Self::Cancelled => "the OCR run was cancelled",
            Self::ResourceLimit => "the OCR engine's output exceeded its bounds",
            Self::ProviderFailed => "the OCR engine failed",
            Self::UnparseableOutput => "the OCR engine's output did not follow its protocol",
            Self::AbnormalTermination => "the OCR engine ended abnormally",
            Self::Workspace => "the OCR hand-over folder could not be used",
        })
    }
}

impl Error for ScreenTextReadError {}

/// Port: the OCR engine.
pub trait ScreenTextReader: Send + Sync {
    /// The engine build that reads: its runner, runtime and models.
    fn identity(
        &self,
    ) -> impl Future<Output = Result<ScreenTextProvider, ScreenTextReadError>> + Send;

    /// Most images one [`Self::read`] call takes.
    fn max_images_per_run(&self) -> usize;

    /// Reads `images`, answering one reading per image in order.
    fn read(
        &self,
        images: &[ScreenTextImage<'_>],
    ) -> impl Future<Output = Result<Vec<EngineReading>, ScreenTextReadError>> + Send;
}

/// Why reading stopped before every pending target of the plan was attempted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenTextStop {
    /// The call's frame budget was used.
    FrameLimit,
    /// The call's deadline passed.
    Deadline,
    /// The caller cancelled.
    Cancelled,
    /// The engine failed on a batch; the frames before it are kept.
    Engine(ScreenTextReadError),
    /// The media tools failed on a batch; the frames before it are kept.
    Media(EvidenceMediaError),
}

impl ScreenTextStop {
    /// Stable identifier, as public results name it.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::FrameLimit => "frame_limit",
            Self::Deadline => "deadline_exceeded",
            Self::Cancelled => "cancelled",
            Self::Engine(_) => "engine_failed",
            Self::Media(_) => "media_failed",
        }
    }
}

/// Why a call could not read anything it could keep.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenTextBuildError {
    /// The engine could not report its identity.
    Engine(ScreenTextReadError),
    /// The previous index belongs to another session, source, stream or profile.
    ScopeMismatch,
    /// An identity could not be derived, or a revision would break a bound.
    Index(ScreenTextError),
}

impl fmt::Display for ScreenTextBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Engine(error) => error.fmt(formatter),
            Self::ScopeMismatch => formatter.write_str(
                "the previous screen-text index does not describe this session's stream",
            ),
            Self::Index(error) => error.fmt(formatter),
        }
    }
}

impl Error for ScreenTextBuildError {}

/// What one call read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTextExtension {
    /// The index scope the engine build reads in.
    pub scope: ScreenTextScope,
    /// The frames read (or failed) this call, in timestamp order.
    pub frames: Vec<ScreenTextFrame>,
    /// Targets that resolved to frames the continued index already holds, by timestamp.
    pub joined: Vec<(i64, Vec<ReadTarget>)>,
    /// Targets that resolved to no frame.
    pub unresolved: Vec<ReadTarget>,
    /// Why the call stopped early, if it did.
    pub stop: Option<ScreenTextStop>,
}

/// A request to read a plan's pending targets.
#[derive(Clone, Copy, Debug)]
pub struct ReadScreenTextRequest<'a> {
    /// The session, source and stream, and the profile; the provider is the engine's own.
    pub session_id: &'a vsift_domain::SessionId,
    /// The session's source.
    pub source_id: &'a vsift_domain::SourceId,
    /// The profile the engine reads with.
    pub profile: vsift_domain::ScreenTextProfile,
    /// The session's newest index, if any.
    pub previous: Option<&'a ScreenTextIndex>,
    /// Every target of the plan, in time order.
    pub plan: &'a [ReadTarget],
    /// Most frames this call reads.
    pub max_frames: u16,
}

/// Derives the identity of revision `number` of an index of `scope`.
///
/// # Errors
///
/// [`ScreenTextError::InvalidFrames`] only if the derived value were not canonical.
pub fn screen_text_index_id(
    scope: &ScreenTextScope,
    number: NonZeroU32,
) -> Result<ScreenTextIndexId, ScreenTextError> {
    ScreenTextIndexId::parse(scope_identity(
        "sti_",
        "vsift.screen-text-index.v1",
        scope,
        &[&number.get().to_string()],
    ))
    .map_err(|_| ScreenTextError::InvalidFrames)
}

/// Derives the identity of the `ordinal`-th line (from 1) read off the frame at `pts`.
///
/// # Errors
///
/// [`ScreenTextError::DuplicateLine`] only if the derived value were not canonical.
pub fn screen_text_line_id(
    scope: &ScreenTextScope,
    pts: i64,
    ordinal: usize,
) -> Result<ScreenTextLineId, ScreenTextError> {
    ScreenTextLineId::parse(scope_identity(
        "stl_",
        "vsift.screen-text-line.v1",
        scope,
        &[&pts.to_string(), &ordinal.to_string()],
    ))
    .map_err(|_| ScreenTextError::DuplicateLine)
}

fn scope_identity(prefix: &str, domain: &str, scope: &ScreenTextScope, tail: &[&str]) -> String {
    let stream = scope.stream_index.to_string();
    let mut parts = vec![
        scope.session_id.as_str(),
        scope.source_id.as_str(),
        stream.as_str(),
        scope.profile.identifier(),
        scope.provider.runner_sha256.as_str(),
        scope.provider.runner_version.as_str(),
        scope.provider.runtime_version.as_str(),
        scope.provider.detector_sha256.as_str(),
        scope.provider.recognizer_sha256.as_str(),
    ];
    parts.extend_from_slice(tail);
    derived_identity(prefix, domain, &parts)
}

/// Checks that every identity in `index` is the one its content derives.
///
/// # Errors
///
/// [`ScreenTextError::InvalidFrames`] for the revision identity and
/// [`ScreenTextError::DuplicateLine`] for a line identity.
pub fn verify_screen_text_identities(index: &ScreenTextIndex) -> Result<(), ScreenTextError> {
    if screen_text_index_id(index.scope(), index.revision())? != *index.id() {
        return Err(ScreenTextError::InvalidFrames);
    }
    for frame in index.frames() {
        for (ordinal, line) in frame.lines().iter().enumerate() {
            if screen_text_line_id(index.scope(), frame.frame.pts, ordinal + 1)? != line.id {
                return Err(ScreenTextError::DuplicateLine);
            }
        }
    }
    Ok(())
}

/// The index `provider` continues: `previous` when the same engine build read it.
#[must_use]
pub fn continued_index<'a>(
    previous: Option<&'a ScreenTextIndex>,
    provider: &ScreenTextProvider,
) -> Option<&'a ScreenTextIndex> {
    previous.filter(|index| index.scope().provider == *provider)
}

/// The targets of `plan` that `continued` (see [`continued_index`]) has not attempted.
#[must_use]
pub fn pending_targets<'a>(
    plan: &'a [ReadTarget],
    continued: Option<&ScreenTextIndex>,
) -> Vec<&'a ReadTarget> {
    plan.iter()
        .filter(|target| continued.is_none_or(|index| !index.attempted(target)))
        .collect()
}

/// Reads the plan's pending targets, at most `max_frames` new frames, in time order.
///
/// # Errors
///
/// [`ScreenTextBuildError::Engine`] when the engine cannot report its identity,
/// [`ScreenTextBuildError::ScopeMismatch`] for a previous index of another stream
/// or session, and [`ScreenTextBuildError::Index`] when an identity cannot be derived.
#[allow(
    clippy::too_many_lines,
    reason = "the order of resolution, budget and reading is the contract; keep it in one place"
)]
pub async fn read_screen_text_within<X: FrameExtractor, R: ScreenTextReader, C: EvidenceControl>(
    request: ReadScreenTextRequest<'_>,
    extractor: &X,
    reader: &R,
    control: &C,
) -> Result<ScreenTextExtension, ScreenTextBuildError> {
    let stream = extractor.stream();
    let provider = reader
        .identity()
        .await
        .map_err(ScreenTextBuildError::Engine)?;
    let scope = ScreenTextScope {
        session_id: request.session_id.clone(),
        source_id: request.source_id.clone(),
        stream_index: stream.stream_index,
        profile: request.profile,
        provider: provider.clone(),
    };
    if let Some(previous) = request.previous
        && (previous.scope().session_id != scope.session_id
            || previous.scope().source_id != scope.source_id
            || previous.scope().stream_index != scope.stream_index
            || previous.scope().profile != scope.profile)
    {
        return Err(ScreenTextBuildError::ScopeMismatch);
    }
    let continued = continued_index(request.previous, &provider);
    let pending = pending_targets(request.plan, continued);
    let mut extension = ScreenTextExtension {
        scope,
        frames: Vec::new(),
        joined: Vec::new(),
        unresolved: Vec::new(),
        stop: None,
    };
    let budget = usize::from(request.max_frames.clamp(1, MAX_SCREEN_TEXT_FRAMES_PER_CALL));
    let mut resolved: Vec<(ListedFrame, Vec<ReadTarget>)> = Vec::new();
    // Resolve targets in time order; targets close together share one listing.
    let mut remaining = pending.as_slice();
    while let Some(first) = remaining.first() {
        if resolved.len() >= budget {
            break;
        }
        if let Some(stop) = stopped(control) {
            extension.stop = Some(stop);
            break;
        }
        let group_end = remaining
            .iter()
            .position(|target| {
                target.time.as_micros() - first.time.as_micros() > MAX_SHARED_LISTING_MICROS
            })
            .unwrap_or(remaining.len());
        let (group, rest) = remaining.split_at(group_end);
        let Some(last) = group.last() else { break };
        if first.time >= stream.duration {
            extension
                .unresolved
                .extend(group.iter().map(|target| (*target).clone()));
            remaining = rest;
            continue;
        }
        let reach = FrameTolerance::DEFAULT.as_micros().saturating_add(1);
        let Ok(listed) = TimeRange::new(
            first.time,
            MediaTime::from_micros(last.time.as_micros().saturating_add(reach)),
        ) else {
            extension
                .unresolved
                .extend(group.iter().map(|target| (*target).clone()));
            remaining = rest;
            continue;
        };
        let listing = match extractor.list_frames(listed).await {
            Ok(listing) => listing,
            Err(error) => {
                extension.stop = Some(media_stop(error));
                break;
            }
        };
        remaining = rest;
        for target in group {
            let tolerance = if target.candidate.is_some() {
                FrameTolerance::EXACT
            } else {
                FrameTolerance::DEFAULT
            };
            let Ok(selected) =
                select_frame(&listing, target.time, FrameSelection::AtOrAfter, tolerance)
            else {
                extension.unresolved.push((*target).clone());
                continue;
            };
            let pts = selected.frame.pts;
            if continued.is_some_and(|index| index.frame_at_pts(pts).is_some()) {
                match extension.joined.iter_mut().find(|(held, _)| *held == pts) {
                    Some((_, targets)) => targets.push((*target).clone()),
                    None => extension.joined.push((pts, vec![(*target).clone()])),
                }
            } else {
                match resolved.iter_mut().find(|(frame, _)| frame.pts == pts) {
                    Some((_, targets)) => targets.push((*target).clone()),
                    None => resolved.push((selected.frame, vec![(*target).clone()])),
                }
            }
        }
    }
    resolved.sort_by_key(|(frame, _)| frame.pts);
    if resolved.len() > budget || (extension.stop.is_none() && !remaining.is_empty()) {
        // Frames past the budget stay pending: their targets were not attempted.
        resolved.truncate(budget);
        extension.stop = extension.stop.or(Some(ScreenTextStop::FrameLimit));
    }
    let batch_size = extractor
        .max_frames_per_run()
        .min(reader.max_images_per_run())
        .max(1);
    for batch in resolved.chunks(batch_size) {
        if let Some(stop) = stopped(control) {
            extension.stop = Some(stop);
            break;
        }
        let pts: Vec<i64> = batch.iter().map(|(frame, _)| frame.pts).collect();
        let images = match extractor.frames(&pts).await {
            Ok(images) if images.len() == batch.len() => images,
            Ok(_) => {
                extension.stop = Some(ScreenTextStop::Media(EvidenceMediaError::FrameNotFound));
                break;
            }
            Err(error) => {
                extension.stop = Some(media_stop(error));
                break;
            }
        };
        let handed: Vec<ScreenTextImage<'_>> = images
            .iter()
            .map(|image| ScreenTextImage {
                png: &image.png,
                dimensions: stream.displayed,
            })
            .collect();
        let readings = match reader.read(&handed).await {
            Ok(readings) if readings.len() == batch.len() => readings,
            Ok(_) => {
                extension.stop = Some(ScreenTextStop::Engine(
                    ScreenTextReadError::UnparseableOutput,
                ));
                break;
            }
            Err(ScreenTextReadError::Cancelled) => {
                extension.stop = Some(ScreenTextStop::Cancelled);
                break;
            }
            Err(ScreenTextReadError::Deadline) => {
                extension.stop = Some(ScreenTextStop::Deadline);
                break;
            }
            Err(error) => {
                extension.stop = Some(ScreenTextStop::Engine(error));
                break;
            }
        };
        for ((listed, targets), reading) in batch.iter().zip(readings) {
            let frame = frame_ref(&stream, *listed);
            let outcome = checked_outcome(&extension.scope, &frame, reading)?;
            extension.frames.push(ScreenTextFrame {
                frame,
                targets: targets.clone(),
                outcome,
            });
        }
    }
    Ok(extension)
}

const fn frame_ref(stream: &VideoStreamFacts, frame: ListedFrame) -> FrameRef {
    FrameRef {
        stream_index: stream.stream_index,
        time_base: stream.time_base,
        pts: frame.pts,
        time: frame.time,
        dimensions: stream.displayed,
    }
}

fn stopped<C: EvidenceControl>(control: &C) -> Option<ScreenTextStop> {
    control.stop().map(|stop| match stop {
        EvidenceStop::DeadlineExceeded => ScreenTextStop::Deadline,
        EvidenceStop::Cancelled => ScreenTextStop::Cancelled,
    })
}

const fn media_stop(error: EvidenceMediaError) -> ScreenTextStop {
    match error {
        EvidenceMediaError::Cancelled => ScreenTextStop::Cancelled,
        EvidenceMediaError::Deadline => ScreenTextStop::Deadline,
        other => ScreenTextStop::Media(other),
    }
}

/// The engine's reading of one frame, checked: any line that is not valid makes
/// the whole reading a failure, never a repaired line.
fn checked_outcome(
    scope: &ScreenTextScope,
    frame: &FrameRef,
    reading: EngineReading,
) -> Result<ScreenTextOutcome, ScreenTextBuildError> {
    let (lines, truncated) = match reading {
        EngineReading::Lines { lines, truncated } => (lines, truncated),
        EngineReading::Rejected => {
            return Ok(ScreenTextOutcome::Failed(ScreenTextFailure::ImageRejected));
        }
        EngineReading::Failed => {
            return Ok(ScreenTextOutcome::Failed(ScreenTextFailure::EngineFailed));
        }
    };
    if lines.len() > MAX_SCREEN_TEXT_LINES_PER_FRAME {
        return Ok(ScreenTextOutcome::Failed(ScreenTextFailure::EngineFailed));
    }
    let mut checked = Vec::with_capacity(lines.len());
    for (ordinal, line) in lines.into_iter().enumerate() {
        let region = TextRegion::new(line.x, line.y, line.width, line.height, frame.dimensions);
        let text = ScreenText::parse(line.text);
        let confidence = Confidence::provider_score(
            line.confidence_basis_points,
            ConfidenceOrigin::ProviderUncalibrated,
        );
        let (Ok(region), Ok(text), Ok(confidence)) = (region, text, confidence) else {
            return Ok(ScreenTextOutcome::Failed(ScreenTextFailure::EngineFailed));
        };
        let id = screen_text_line_id(scope, frame.pts, ordinal + 1)
            .map_err(ScreenTextBuildError::Index)?;
        checked.push(ScreenTextLine {
            id,
            region,
            text,
            confidence,
        });
    }
    Ok(ScreenTextOutcome::Read {
        lines: checked,
        truncated,
    })
}

/// The next revision: the continued index's frames with this call's frames,
/// joined targets and unresolved targets added, or `None` when nothing changed.
///
/// `previous` read by another engine build is not continued: the revision starts a
/// new index of the extension's engine.
///
/// # Errors
///
/// [`ScreenTextBuildError::ScopeMismatch`] when the continued index describes another
/// scope, and [`ScreenTextBuildError::Index`] when the revision would break a bound.
pub fn merge_screen_text(
    previous: Option<&ScreenTextIndex>,
    extension: &ScreenTextExtension,
    period_micros: u64,
) -> Result<Option<ScreenTextIndex>, ScreenTextBuildError> {
    let scope = &extension.scope;
    let base = continued_index(previous, &scope.provider);
    if base.is_some_and(|index| index.scope() != scope) {
        return Err(ScreenTextBuildError::ScopeMismatch);
    }
    let mut frames: Vec<ScreenTextFrame> = base
        .map(|index| index.frames().to_vec())
        .unwrap_or_default();
    let mut unresolved: Vec<ReadTarget> = base
        .map(|index| index.unresolved().to_vec())
        .unwrap_or_default();
    let mut changed = base.is_some_and(|index| index.period_micros() != period_micros);
    let added = extension
        .joined
        .iter()
        .map(|(pts, targets)| (*pts, targets.as_slice(), None))
        .chain(
            extension
                .frames
                .iter()
                .map(|frame| (frame.frame.pts, frame.targets.as_slice(), Some(frame))),
        );
    for (pts, targets, frame) in added {
        match frames.binary_search_by_key(&pts, |held| held.frame.pts) {
            Ok(at) => {
                if let Some(held) = frames.get_mut(at) {
                    for target in targets {
                        if !held.targets.contains(target) {
                            held.targets.push(target.clone());
                            changed = true;
                        }
                    }
                }
            }
            Err(at) => {
                if let Some(frame) = frame {
                    frames.insert(at, frame.clone());
                    changed = true;
                }
            }
        }
    }
    for target in &extension.unresolved {
        if !unresolved.contains(target) {
            unresolved.push(target.clone());
            changed = true;
        }
    }
    if !changed && base.is_some() {
        return Ok(None);
    }
    if base.is_none() && frames.is_empty() && unresolved.is_empty() {
        return Ok(None);
    }
    let number = base
        .map_or(Some(NonZeroU32::MIN), |index| {
            index.revision().checked_add(1)
        })
        .ok_or(ScreenTextBuildError::Index(ScreenTextError::TooLarge))?;
    let id = screen_text_index_id(scope, number).map_err(ScreenTextBuildError::Index)?;
    ScreenTextIndex::new(ScreenTextIndexParts {
        id,
        revision: number,
        scope: scope.clone(),
        period_micros,
        frames,
        unresolved,
    })
    .map(Some)
    .map_err(ScreenTextBuildError::Index)
}

#[cfg(test)]
mod tests;
