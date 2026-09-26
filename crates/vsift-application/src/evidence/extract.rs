//! The five evidence use cases and the budgeted extraction they share.

use std::collections::BTreeSet;

use vsift_domain::{
    AudioRange, BurstCount, BurstRange, CropRect, CropRegion, EvidenceDetail, EvidenceItem,
    EvidenceItemParts, EvidenceMedia, EvidenceMediaKind, EvidenceRecord, EvidenceRecordParts,
    EvidenceRequest, EvidenceSelection, EvidenceSubject, FrameRef, FrameSelection,
    FrameSelectionError, FrameTolerance, ListedFrame, MediaTime, NeighbourCount, NeighbourPlan,
    NeighbourStop, PartialReason, SelectionRole, TimeRange, VisualCandidateId, plan_burst,
    plan_neighbours, select_frame,
};

use super::{
    AudioExtractor, EvidenceCall, EvidenceControl, EvidenceError, EvidenceExtraction,
    EvidenceMediaError, EvidenceStop, ExtractedMedia, FrameExtractor, VideoStreamFacts,
    evidence_id, evidence_request_key, media_digest,
};

/// Half-widths of the listings a neighbours call tries, widest last. Each
/// listing covers the anchor and at most 59 s, inside the 60 s listing
/// bound; a wider one is only listed when a side ran short at the edge of
/// the narrower one.
const NEIGHBOUR_LISTING_HALF_WIDTHS: [u64; 3] = [2_000_000, 10_000_000, 29_000_000];

/// A `frame get` request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameAtRequest {
    /// Requested normalized time.
    pub at: MediaTime,
    /// Selection policy.
    pub selection: FrameSelection,
    /// How far the frame may lie from `at`.
    pub tolerance: FrameTolerance,
    /// The visual candidate whose representative time `at` is. The frame
    /// must then lie exactly at `at` (at or after, tolerance zero); any
    /// other outcome means the index and the source disagree.
    pub candidate: Option<VisualCandidateId>,
}

/// A `crop` request: a rectangle in the parent item's image pixels.
#[derive(Clone, Copy, Debug)]
pub struct CropRequest<'a> {
    /// The frame or crop item cut from.
    pub parent: &'a EvidenceItem,
    /// Left coordinate in the parent image.
    pub x: u32,
    /// Top coordinate in the parent image.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// Extracts the frame a time (or a visual candidate) names.
///
/// The listing covers only what the policy and tolerance can reach: from
/// the request to one microsecond past the tolerance for at-or-after, and
/// the mirror image for displayed-at.
///
/// # Errors
///
/// [`EvidenceError::Selection`] when no frame satisfies the policy and
/// tolerance, [`EvidenceError::CandidateMismatch`] when a candidate's frame
/// is not exactly at its time, and the shared failures of every call.
pub async fn extract_frame_at<X: FrameExtractor, C: EvidenceControl>(
    call: &EvidenceCall<'_, C>,
    extractor: &X,
    request: FrameAtRequest,
) -> Result<EvidenceExtraction, EvidenceError> {
    let stream = extractor.stream();
    let at = request.at;
    let for_candidate = request.candidate.is_some();
    let not_selected = |error| {
        if for_candidate {
            EvidenceError::CandidateMismatch
        } else {
            EvidenceError::Selection(error)
        }
    };
    if at >= stream.duration {
        return Err(not_selected(FrameSelectionError::AtOrAfterEnd));
    }
    let reach = request.tolerance.as_micros().saturating_add(1);
    let listed = match request.selection {
        FrameSelection::AtOrAfter => {
            time_range(at.as_micros(), at.as_micros().saturating_add(reach))
        }
        FrameSelection::DisplayedAt => time_range(
            at.as_micros().saturating_sub(reach),
            at.as_micros().saturating_add(1),
        ),
    }?;
    let mut collector = Collector::new(call)?;
    collector.check_stop()?;
    let listing = extractor
        .list_frames(listed)
        .await
        .map_err(EvidenceError::Media)?;
    let selected =
        select_frame(&listing, at, request.selection, request.tolerance).map_err(not_selected)?;
    if for_candidate && selected.frame.time != at {
        return Err(EvidenceError::CandidateMismatch);
    }
    let extracted = collector
        .extract_frames(extractor, vec![selected.frame])
        .await?;
    let selections = extracted
        .iter()
        .map(|(frame, id)| {
            EvidenceSelection::new(SelectionRole::Requested, id.clone(), at, frame.time)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(EvidenceError::Record)?;
    collector.finish(
        EvidenceRequest::FrameAt {
            at,
            selection: request.selection,
            tolerance: request.tolerance,
            candidate: request.candidate,
        },
        selections,
        EvidenceDetail::Single,
    )
}

/// Extracts up to `count` consecutive frames on each side of an earlier
/// frame item.
///
/// A narrow listing around the anchor is tried first and widened only when
/// a side stopped at the listing's edge, so a normal frame rate costs one
/// listing. Frames nearest the anchor are extracted first, alternating
/// sides, so a budget stop keeps the closest ones.
///
/// # Errors
///
/// [`EvidenceError::KindMismatch`] for an anchor that is not a whole frame,
/// [`EvidenceError::ParentMismatch`] when the anchor does not describe the
/// probed stream or is not listed where it says, and the shared failures.
pub async fn extract_neighbours<X: FrameExtractor, C: EvidenceControl>(
    call: &EvidenceCall<'_, C>,
    extractor: &X,
    anchor: &EvidenceItem,
    count: NeighbourCount,
) -> Result<EvidenceExtraction, EvidenceError> {
    let stream = extractor.stream();
    let EvidenceSubject::Frame(frame) = anchor.subject() else {
        return Err(EvidenceError::KindMismatch);
    };
    check_frame_stream(frame, &stream)?;
    let mut collector = Collector::new(call)?;
    let mut plan: Option<NeighbourPlan> = None;
    for half_width in NEIGHBOUR_LISTING_HALF_WIDTHS {
        collector.check_stop()?;
        let time = frame.time.as_micros();
        let listed = time_range(
            time.saturating_sub(half_width),
            time.saturating_add(half_width).saturating_add(1),
        )?;
        let listing = extractor
            .list_frames(listed)
            .await
            .map_err(EvidenceError::Media)?;
        if listing.covered().end() <= frame.time {
            // A dense stream filled the listing's frame bound before the
            // anchor; a wider listing would only end earlier.
            break;
        }
        let widened = plan_neighbours(&listing, frame.pts, count)
            .map_err(|_| EvidenceError::ParentMismatch)?;
        let complete = widened.before_stop != Some(NeighbourStop::SearchWindow)
            && widened.after_stop != Some(NeighbourStop::SearchWindow);
        plan = Some(widened);
        if complete {
            break;
        }
    }
    let plan = plan.ok_or(EvidenceError::Media(EvidenceMediaError::ResourceLimit))?;
    let mut prioritized = Vec::with_capacity(plan.before.len() + plan.after.len());
    let mut before = plan.before.iter().rev();
    let mut after = plan.after.iter();
    loop {
        let next_after = after.next();
        let next_before = before.next();
        if next_after.is_none() && next_before.is_none() {
            break;
        }
        prioritized.extend(next_after.copied());
        prioritized.extend(next_before.copied());
    }
    if prioritized.is_empty() {
        return Err(EvidenceError::Selection(
            FrameSelectionError::OutsideListing,
        ));
    }
    let mut extracted = collector.extract_frames(extractor, prioritized).await?;
    extracted.sort_by_key(|(listed, _)| listed.pts);
    let selections = extracted
        .iter()
        .map(|(listed, id)| {
            let role = if listed.pts < frame.pts {
                SelectionRole::Before
            } else {
                SelectionRole::After
            };
            EvidenceSelection::new(role, id.clone(), frame.time, listed.time)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(EvidenceError::Record)?;
    collector.finish(
        EvidenceRequest::Neighbours {
            anchor: anchor.id().clone(),
            count,
        },
        selections,
        EvidenceDetail::Neighbours {
            before_stop: plan.before_stop,
            after_stop: plan.after_stop,
        },
    )
}

/// Extracts the distinct frames `max_frames` evenly spaced targets over a
/// range name.
///
/// # Errors
///
/// [`EvidenceError::Selection`] when the range starts at or after the end
/// of the source, holds no displayed frame, or is denser than one listing
/// covers; and the shared failures.
pub async fn extract_burst<X: FrameExtractor, C: EvidenceControl>(
    call: &EvidenceCall<'_, C>,
    extractor: &X,
    range: BurstRange,
    max_frames: BurstCount,
) -> Result<EvidenceExtraction, EvidenceError> {
    let stream = extractor.stream();
    if range.range().start() >= stream.duration {
        return Err(EvidenceError::Selection(FrameSelectionError::AtOrAfterEnd));
    }
    let mut collector = Collector::new(call)?;
    collector.check_stop()?;
    let listing = extractor
        .list_frames(range.range())
        .await
        .map_err(EvidenceError::Media)?;
    let plan = plan_burst(&listing, range, max_frames).map_err(EvidenceError::Selection)?;
    if plan.frames.is_empty() {
        return Err(EvidenceError::Selection(
            FrameSelectionError::NoFrameWithinTolerance,
        ));
    }
    let distinct = u8::try_from(plan.distinct())
        .map_err(|_| EvidenceError::Media(EvidenceMediaError::Invalid))?;
    let extracted = collector
        .extract_frames(extractor, plan.frames.clone())
        .await?;
    let mut selections = Vec::new();
    for target in plan.target_times() {
        let Some(frame) = plan.frame_for(target) else {
            break;
        };
        if let Some((listed, id)) = extracted.iter().find(|(listed, _)| listed.pts == frame.pts) {
            selections.push(
                EvidenceSelection::new(SelectionRole::Target, id.clone(), target, listed.time)
                    .map_err(EvidenceError::Record)?,
            );
        }
    }
    collector.finish(
        EvidenceRequest::Burst { range, max_frames },
        selections,
        EvidenceDetail::Burst {
            planned: plan.planned,
            extent: plan.extent,
            targets: plan.targets,
            distinct,
        },
    )
}

/// Extracts a rectangle of an earlier frame or crop item by decoding the
/// frame again and cropping in the provider (ADR 0019 D7).
///
/// The rectangle is given in the parent image's pixels and composed back to
/// the source frame's, so a crop of a crop names source pixels.
///
/// # Errors
///
/// [`EvidenceError::KindMismatch`] for an audio parent,
/// [`EvidenceError::CropOutsideParent`] for a rectangle not contained by
/// the parent image, [`EvidenceError::ParentMismatch`] for a parent that does
/// not describe the probed stream, and the shared failures.
pub async fn extract_crop<X: FrameExtractor, C: EvidenceControl>(
    call: &EvidenceCall<'_, C>,
    extractor: &X,
    request: CropRequest<'_>,
) -> Result<EvidenceExtraction, EvidenceError> {
    let stream = extractor.stream();
    let parent = request.parent;
    let (frame, outer) = match parent.subject() {
        EvidenceSubject::Frame(frame) => (frame, None),
        EvidenceSubject::Crop { frame, region } => (frame, Some(region.frame_rect)),
        EvidenceSubject::Audio { .. } => return Err(EvidenceError::KindMismatch),
    };
    check_frame_stream(frame, &stream)?;
    let parent_dimensions = parent
        .subject()
        .image_dimensions()
        .ok_or(EvidenceError::KindMismatch)?;
    let rect = CropRect::new(
        request.x,
        request.y,
        request.width,
        request.height,
        parent_dimensions,
    )
    .map_err(|_| EvidenceError::CropOutsideParent)?;
    let frame_rect = match outer {
        None => rect,
        Some(outer) => outer
            .compose(rect)
            .map_err(|_| EvidenceError::CropOutsideParent)?,
    };
    let mut collector = Collector::new(call)?;
    collector.check_stop()?;
    let image = extractor
        .crop(frame.pts, frame_rect)
        .await
        .map_err(EvidenceError::Media)?;
    if image.pts != frame.pts || image.time != frame.time {
        return Err(EvidenceError::Media(EvidenceMediaError::Invalid));
    }
    let subject = EvidenceSubject::Crop {
        frame: *frame,
        region: CropRegion {
            parent: parent.id().clone(),
            rect,
            frame_rect,
        },
    };
    let id = collector
        .admit_item(subject, EvidenceMediaKind::FramePng, image.png)?
        .ok_or(EvidenceError::BudgetExhausted)?;
    let selection = EvidenceSelection::new(SelectionRole::Requested, id, frame.time, frame.time)
        .map_err(EvidenceError::Record)?;
    collector.finish(
        EvidenceRequest::Crop {
            parent: parent.id().clone(),
            rect,
        },
        vec![selection],
        EvidenceDetail::Single,
    )
}

/// Extracts an audio clip of a range, clipped to the source.
///
/// # Errors
///
/// [`EvidenceError::RangeOutsideSource`] for a range that starts at or after
/// the end of the source, and the shared failures.
pub async fn extract_audio<X: AudioExtractor, C: EvidenceControl>(
    call: &EvidenceCall<'_, C>,
    extractor: &X,
    range: AudioRange,
) -> Result<EvidenceExtraction, EvidenceError> {
    let requested = range.range();
    let duration = extractor.duration();
    if requested.start() >= duration {
        return Err(EvidenceError::RangeOutsideSource);
    }
    let range_clipped = requested.end() > duration;
    let clipped = TimeRange::new(requested.start(), requested.end().min(duration))
        .map_err(|_| EvidenceError::RangeOutsideSource)?;
    let mut collector = Collector::new(call)?;
    collector.check_stop()?;
    let clip = extractor
        .clip(clipped)
        .await
        .map_err(EvidenceError::Media)?;
    let subject = EvidenceSubject::Audio {
        stream_index: extractor.stream_index(),
        range: clipped,
        actual_start: clip.actual_start,
    };
    let id = collector
        .admit_item(subject, EvidenceMediaKind::AudioWav, clip.wav)?
        .ok_or(EvidenceError::BudgetExhausted)?;
    let selection = EvidenceSelection::new(
        SelectionRole::Requested,
        id,
        requested.start(),
        clip.actual_start,
    )
    .map_err(EvidenceError::Record)?;
    collector.finish(
        EvidenceRequest::Audio { range },
        vec![selection],
        EvidenceDetail::Audio { range_clipped },
    )
}

fn time_range(start: u64, end: u64) -> Result<TimeRange, EvidenceError> {
    TimeRange::new(MediaTime::from_micros(start), MediaTime::from_micros(end))
        .map_err(|_| EvidenceError::RangeOutsideSource)
}

/// A parent frame must describe the stream the probe selected now.
fn check_frame_stream(frame: &FrameRef, stream: &VideoStreamFacts) -> Result<(), EvidenceError> {
    if frame.stream_index != stream.stream_index
        || frame.time_base != stream.time_base
        || frame.dimensions != stream.displayed
        || frame.time >= stream.duration
    {
        return Err(EvidenceError::ParentMismatch);
    }
    Ok(())
}

/// Accumulates one call's items and new media under its budget.
struct Collector<'c, 'a, C> {
    call: &'c EvidenceCall<'a, C>,
    items: Vec<EvidenceItem>,
    media: Vec<ExtractedMedia>,
    new_digests: BTreeSet<String>,
    bytes: u64,
    partial: Option<PartialReason>,
}

impl<'c, 'a, C: EvidenceControl> Collector<'c, 'a, C> {
    /// Starts a call; the session must have room for the record and one
    /// media file before any provider runs.
    fn new(call: &'c EvidenceCall<'a, C>) -> Result<Self, EvidenceError> {
        if call.budget.session_slots < 2 || call.budget.max_frames == 0 {
            return Err(EvidenceError::BudgetExhausted);
        }
        Ok(Self {
            call,
            items: Vec::new(),
            media: Vec::new(),
            new_digests: BTreeSet::new(),
            bytes: 0,
            partial: None,
        })
    }

    /// Fails with the stop reason when the call must stop before a
    /// provider run and nothing was extracted yet.
    fn check_stop(&self) -> Result<(), EvidenceError> {
        match self.call.control.stop() {
            Some(stop) => Err(EvidenceError::Stopped(stop)),
            None => Ok(()),
        }
    }

    /// Extracts `prioritized` frames in runs, most important first, and
    /// returns each extracted frame with its item identity.
    async fn extract_frames<X: FrameExtractor>(
        &mut self,
        extractor: &X,
        mut prioritized: Vec<ListedFrame>,
    ) -> Result<Vec<(ListedFrame, vsift_domain::EvidenceId)>, EvidenceError> {
        let stream = extractor.stream();
        let budget = self.call.budget;
        if prioritized.len() > budget.max_frames {
            prioritized.truncate(budget.max_frames);
            self.partial = Some(PartialReason::FrameBudget);
        }
        let pixels = u64::from(stream.displayed.width()) * u64::from(stream.displayed.height());
        let fitting = usize::try_from(budget.max_pixels / pixels.max(1)).unwrap_or(usize::MAX);
        if prioritized.len() > fitting {
            prioritized.truncate(fitting);
            self.partial = Some(PartialReason::PixelBudget);
        }
        if prioritized.is_empty() {
            return Err(EvidenceError::BudgetExhausted);
        }
        let per_run = extractor.max_frames_per_run().max(1);
        let mut extracted = Vec::with_capacity(prioritized.len());
        'runs: for run in prioritized.chunks(per_run) {
            if let Some(stop) = self.call.control.stop() {
                self.stop(stop)?;
                break;
            }
            let mut pts: Vec<i64> = run.iter().map(|frame| frame.pts).collect();
            pts.sort_unstable();
            let images = match extractor.frames(&pts).await {
                Ok(images) => images,
                Err(EvidenceMediaError::Deadline) if !self.items.is_empty() => {
                    self.partial = Some(PartialReason::DeadlineExceeded);
                    break;
                }
                Err(EvidenceMediaError::Cancelled) if !self.items.is_empty() => {
                    self.partial = Some(PartialReason::Cancelled);
                    break;
                }
                Err(error) => return Err(EvidenceError::Media(error)),
            };
            if images.len() != pts.len() {
                return Err(EvidenceError::Media(EvidenceMediaError::Invalid));
            }
            let mut images: Vec<Option<super::ExtractedFrame>> =
                images.into_iter().map(Some).collect();
            for frame in run {
                let image = pts
                    .iter()
                    .position(|value| *value == frame.pts)
                    .and_then(|index| images.get_mut(index))
                    .and_then(Option::take)
                    .ok_or(EvidenceError::Media(EvidenceMediaError::Invalid))?;
                if image.pts != frame.pts || image.time != frame.time {
                    return Err(EvidenceError::Media(EvidenceMediaError::Invalid));
                }
                let subject = EvidenceSubject::Frame(FrameRef {
                    stream_index: stream.stream_index,
                    time_base: stream.time_base,
                    pts: frame.pts,
                    time: frame.time,
                    dimensions: stream.displayed,
                });
                match self.admit_item(subject, EvidenceMediaKind::FramePng, image.png)? {
                    Some(id) => extracted.push((*frame, id)),
                    None => break 'runs,
                }
            }
        }
        if extracted.is_empty() {
            return Err(self.nothing_extracted());
        }
        Ok(extracted)
    }

    /// Records a stop after a run; fails when nothing was extracted.
    fn stop(&mut self, stop: EvidenceStop) -> Result<(), EvidenceError> {
        if self.items.is_empty() {
            return Err(EvidenceError::Stopped(stop));
        }
        self.partial = Some(match stop {
            EvidenceStop::DeadlineExceeded => PartialReason::DeadlineExceeded,
            EvidenceStop::Cancelled => PartialReason::Cancelled,
        });
        Ok(())
    }

    /// The failure of a call that extracted nothing.
    fn nothing_extracted(&self) -> EvidenceError {
        match self.partial {
            Some(PartialReason::DeadlineExceeded) => {
                EvidenceError::Stopped(EvidenceStop::DeadlineExceeded)
            }
            Some(PartialReason::Cancelled) => EvidenceError::Stopped(EvidenceStop::Cancelled),
            Some(
                PartialReason::FrameBudget
                | PartialReason::PixelBudget
                | PartialReason::ByteBudget
                | PartialReason::SessionEvidenceBudget,
            )
            | None => EvidenceError::BudgetExhausted,
        }
    }

    /// Admits one extracted file as an item, or returns `None` (recording
    /// the partial reason) when the byte budget or the session's evidence
    /// slots stop it. A file the session or this call already holds takes
    /// no new slot.
    fn admit_item(
        &mut self,
        subject: EvidenceSubject,
        kind: EvidenceMediaKind,
        bytes: Vec<u8>,
    ) -> Result<Option<vsift_domain::EvidenceId>, EvidenceError> {
        let budget = self.call.budget;
        let size = u64::try_from(bytes.len())
            .map_err(|_| EvidenceError::Media(EvidenceMediaError::ResourceLimit))?;
        if self.bytes.saturating_add(size) > budget.max_bytes {
            self.partial = Some(PartialReason::ByteBudget);
            return Ok(None);
        }
        let digest = media_digest(&bytes)?;
        let known = self.call.known_media.contains(digest.as_str())
            || self.new_digests.contains(digest.as_str());
        // One slot is always kept for the call's record.
        if !known && self.media.len().saturating_add(2) > budget.session_slots {
            self.partial = Some(PartialReason::SessionEvidenceBudget);
            return Ok(None);
        }
        let media =
            EvidenceMedia::new(kind, digest.clone(), size).map_err(EvidenceError::Record)?;
        let scope = self.call.scope;
        let id = evidence_id(&scope, &subject, &media).map_err(EvidenceError::Identity)?;
        let item = EvidenceItem::new(EvidenceItemParts {
            id: id.clone(),
            subject,
            media,
            tool_fingerprint: scope.tool_fingerprint.cloned(),
            source_check: self.call.source_check,
        })
        .map_err(EvidenceError::Record)?;
        if !known {
            self.new_digests.insert(digest.as_str().to_owned());
            self.media.push(ExtractedMedia {
                kind,
                sha256: digest,
                bytes,
            });
        }
        self.bytes = self.bytes.saturating_add(size);
        if !self.items.iter().any(|existing| existing.id() == &id) {
            self.items.push(item);
        }
        Ok(Some(id))
    }

    /// Builds the call's record.
    fn finish(
        self,
        request: EvidenceRequest,
        selections: Vec<EvidenceSelection>,
        detail: EvidenceDetail,
    ) -> Result<EvidenceExtraction, EvidenceError> {
        if self.items.is_empty() {
            return Err(self.nothing_extracted());
        }
        let scope = self.call.scope;
        let record = EvidenceRecord::new(EvidenceRecordParts {
            request_key: evidence_request_key(&scope, &request).map_err(EvidenceError::Identity)?,
            session_id: scope.session_id.clone(),
            source_id: scope.source_id.clone(),
            profile: scope.profile,
            tool_fingerprint: scope.tool_fingerprint.cloned(),
            request,
            selections,
            detail,
            items: self.items,
            partial: self.partial,
        })
        .map_err(EvidenceError::Record)?;
        Ok(EvidenceExtraction {
            record,
            media: self.media,
        })
    }
}
