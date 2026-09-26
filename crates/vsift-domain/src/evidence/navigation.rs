//! Evidence navigation (P09, ADR 0019): which displayed frames a request names.
//!
//! A frame request never guesses a frame from a frame rate. The media adapter
//! first *lists* the displayed frames of a bounded stretch of the stream (the
//! actual decoded timestamps, see [`FrameListing`]); the pure functions here
//! then choose frames from that listing, and the adapter extracts exactly the
//! chosen timestamps. So the same listing always yields the same choice on
//! every platform, and a choice can always say *why* it stopped short:
//!
//! - [`select_frame`]: one frame for a requested time, by a
//!   [`FrameSelection`] policy within a [`FrameTolerance`].
//! - [`plan_neighbours`]: up to [`MAX_NEIGHBOUR_COUNT`] consecutive frames
//!   on each side of an anchor frame, with a typed [`NeighbourStop`] for a
//!   side that ran short.
//! - [`plan_burst`]: up to [`MAX_BURST_FRAMES`] frames spread evenly in time
//!   over a range of at most [`MAX_BURST_RANGE_MICROS`], deduplicated, with
//!   the number of distinct frames reported.
//!
//! A listing covers a half-open range completely: every displayed frame
//! whose normalized time lies in it is listed. Its [`ListingTail`] says
//! whether the stream ends at the range's end. Nothing outside the covered
//! range is assumed, so a request the listing cannot decide is
//! [`FrameSelectionError::OutsideListing`] rather than a silent guess.

use std::{error::Error, fmt};

use crate::{FrameTiming, MediaTime, TimeRange};

/// Largest frame-selection tolerance: ten seconds.
pub const MAX_FRAME_TOLERANCE_MICROS: u64 = 10_000_000;
/// Most frames one listing holds. Sixty seconds at 20 frames per second; a
/// denser stream is listed in several passes.
pub const MAX_LISTED_FRAMES: usize = 1_200;
/// Most neighbours on each side of an anchor frame.
pub const MAX_NEIGHBOUR_COUNT: u8 = 20;
/// Most frames in one burst.
pub const MAX_BURST_FRAMES: u8 = 100;
/// Longest burst range: sixty seconds.
pub const MAX_BURST_RANGE_MICROS: u64 = 60_000_000;

/// Which displayed frame a requested time names.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum FrameSelection {
    /// The first frame whose presentation time is at or after the request.
    /// The default: the returned frame was on screen no earlier than asked,
    /// so evidence is never taken from before the moment an agent named.
    #[default]
    AtOrAfter,
    /// The frame on screen at the request: the last frame whose
    /// presentation time is at or before it.
    DisplayedAt,
}

impl FrameSelection {
    /// Every policy.
    pub const ALL: [Self; 2] = [Self::AtOrAfter, Self::DisplayedAt];

    /// Returns the stable identifier used in machine-readable contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::AtOrAfter => "at_or_after",
            Self::DisplayedAt => "displayed_at",
        }
    }
}

/// How far, in microseconds, a selected frame's time may lie from the request.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FrameTolerance(u64);

impl FrameTolerance {
    /// Only a frame exactly at the requested time.
    pub const EXACT: Self = Self(0);
    /// The largest accepted tolerance.
    pub const MAX: Self = Self(MAX_FRAME_TOLERANCE_MICROS);

    /// Creates a tolerance of `micros` microseconds.
    ///
    /// # Errors
    ///
    /// Returns [`NavigationError::ToleranceTooLarge`] above
    /// [`MAX_FRAME_TOLERANCE_MICROS`].
    pub const fn new(micros: u64) -> Result<Self, NavigationError> {
        if micros > MAX_FRAME_TOLERANCE_MICROS {
            Err(NavigationError::ToleranceTooLarge)
        } else {
            Ok(Self(micros))
        }
    }

    /// Returns the tolerance in microseconds.
    #[must_use]
    pub const fn as_micros(self) -> u64 {
        self.0
    }
}

/// One displayed frame found by a listing: its stream timestamp, which is
/// what the adapter extracts exactly, and its normalized time.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ListedFrame {
    /// Presentation timestamp in the stream's own time base.
    pub pts: i64,
    /// Normalized presentation time ([`crate::StreamTime::to_media_time`]).
    pub time: MediaTime,
}

/// What follows the covered range of a listing.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ListingTail {
    /// The covered range ends where the source ends; no frame follows.
    EndOfStream,
    /// Frames may follow the covered range; they were not listed.
    MoreMayFollow,
}

/// Every displayed frame of one stream in a covered half-open range, in
/// presentation order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameListing {
    covered: TimeRange,
    frames: Vec<ListedFrame>,
    tail: ListingTail,
}

impl FrameListing {
    /// Validates a listing.
    ///
    /// # Errors
    ///
    /// Returns [`NavigationError::ListingTooLarge`] beyond
    /// [`MAX_LISTED_FRAMES`] and [`NavigationError::ListingInconsistent`]
    /// unless timestamps strictly increase, times never decrease and every
    /// time lies in `covered`.
    pub fn new(
        covered: TimeRange,
        frames: Vec<ListedFrame>,
        tail: ListingTail,
    ) -> Result<Self, NavigationError> {
        if frames.len() > MAX_LISTED_FRAMES {
            return Err(NavigationError::ListingTooLarge);
        }
        let ordered = frames
            .windows(2)
            .all(|pair| matches!(pair, [before, after] if before.pts < after.pts && before.time <= after.time));
        let inside = frames
            .iter()
            .all(|frame| frame.time >= covered.start() && frame.time < covered.end());
        if !ordered || !inside {
            return Err(NavigationError::ListingInconsistent);
        }
        Ok(Self {
            covered,
            frames,
            tail,
        })
    }

    /// Returns the range in which every displayed frame is listed.
    #[must_use]
    pub const fn covered(&self) -> TimeRange {
        self.covered
    }

    /// Returns the listed frames in presentation order.
    #[must_use]
    pub fn frames(&self) -> &[ListedFrame] {
        &self.frames
    }

    /// Returns what follows the covered range.
    #[must_use]
    pub const fn tail(&self) -> ListingTail {
        self.tail
    }

    /// Index of the first frame at or after `time`.
    fn first_at_or_after(&self, time: MediaTime) -> usize {
        self.frames.partition_point(|frame| frame.time < time)
    }

    fn ends_stream(&self) -> bool {
        self.tail == ListingTail::EndOfStream
    }
}

/// A frame chosen for a requested time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectedFrame {
    /// The chosen displayed frame.
    pub frame: ListedFrame,
    /// Requested and actual time, with their signed difference.
    pub timing: FrameTiming,
}

/// Chooses the frame `request` names under `policy`.
///
/// # Errors
///
/// - [`FrameSelectionError::AtOrAfterEnd`]: the request is at or after the
///   end of the source.
/// - [`FrameSelectionError::AfterFinalFrame`]: at-or-after a time later than
///   the final frame.
/// - [`FrameSelectionError::BeforeFirstFrame`]: displayed-at a time before
///   the first frame.
/// - [`FrameSelectionError::NoFrameWithinTolerance`]: the nearest frame the
///   policy allows is further than `tolerance`.
/// - [`FrameSelectionError::OutsideListing`]: the listing does not cover
///   enough of the stream to decide.
pub fn select_frame(
    listing: &FrameListing,
    request: MediaTime,
    policy: FrameSelection,
    tolerance: FrameTolerance,
) -> Result<SelectedFrame, FrameSelectionError> {
    let covered = listing.covered;
    if listing.ends_stream() && request >= covered.end() {
        return Err(FrameSelectionError::AtOrAfterEnd);
    }
    if request < covered.start() || request >= covered.end() {
        return Err(FrameSelectionError::OutsideListing);
    }
    let tolerance = tolerance.as_micros();
    let frame = match policy {
        FrameSelection::AtOrAfter => {
            let Some(frame) = listing.frames.get(listing.first_at_or_after(request)) else {
                return Err(if listing.ends_stream() {
                    FrameSelectionError::AfterFinalFrame
                } else if covered.end().as_micros() - request.as_micros() > tolerance {
                    FrameSelectionError::NoFrameWithinTolerance
                } else {
                    FrameSelectionError::OutsideListing
                });
            };
            if frame.time.as_micros() - request.as_micros() > tolerance {
                return Err(FrameSelectionError::NoFrameWithinTolerance);
            }
            *frame
        }
        FrameSelection::DisplayedAt => {
            let after = listing
                .frames
                .partition_point(|frame| frame.time <= request);
            let Some(frame) = after
                .checked_sub(1)
                .and_then(|index| listing.frames.get(index))
            else {
                return Err(if covered.start().as_micros() == 0 {
                    FrameSelectionError::BeforeFirstFrame
                } else if request.as_micros() - covered.start().as_micros() >= tolerance {
                    FrameSelectionError::NoFrameWithinTolerance
                } else {
                    FrameSelectionError::OutsideListing
                });
            };
            if request.as_micros() - frame.time.as_micros() > tolerance {
                return Err(FrameSelectionError::NoFrameWithinTolerance);
            }
            *frame
        }
    };
    let timing =
        FrameTiming::new(request, frame.time).map_err(|_| FrameSelectionError::OutsideListing)?;
    Ok(SelectedFrame { frame, timing })
}

/// Number of neighbours to take on each side of an anchor, 1 through
/// [`MAX_NEIGHBOUR_COUNT`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NeighbourCount(u8);

impl NeighbourCount {
    /// Creates a per-side neighbour count.
    ///
    /// # Errors
    ///
    /// Returns [`NavigationError::NeighbourCountOutOfRange`] outside
    /// `1..=MAX_NEIGHBOUR_COUNT`.
    pub const fn new(count: u8) -> Result<Self, NavigationError> {
        if count == 0 || count > MAX_NEIGHBOUR_COUNT {
            Err(NavigationError::NeighbourCountOutOfRange)
        } else {
            Ok(Self(count))
        }
    }

    /// Returns the per-side count.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Why one side of a neighbour plan holds fewer frames than asked.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NeighbourStop {
    /// No displayed frame precedes the ones returned.
    StartOfStream,
    /// No displayed frame follows the ones returned.
    EndOfStream,
    /// The listing searched ended; more frames may lie beyond it.
    SearchWindow,
}

impl NeighbourStop {
    /// Every stop reason.
    pub const ALL: [Self; 3] = [Self::StartOfStream, Self::EndOfStream, Self::SearchWindow];

    /// Returns the stable identifier used in machine-readable contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::StartOfStream => "start_of_stream",
            Self::EndOfStream => "end_of_stream",
            Self::SearchWindow => "search_window",
        }
    }
}

/// Consecutive displayed frames around an anchor frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NeighbourPlan {
    /// The anchor frame itself.
    pub anchor: ListedFrame,
    /// Frames immediately before the anchor, in presentation order.
    pub before: Vec<ListedFrame>,
    /// Why `before` is short, or `None` when it holds the requested count.
    pub before_stop: Option<NeighbourStop>,
    /// Frames immediately after the anchor, in presentation order.
    pub after: Vec<ListedFrame>,
    /// Why `after` is short, or `None` when it holds the requested count.
    pub after_stop: Option<NeighbourStop>,
}

/// Plans `count` consecutive frames on each side of the listed frame whose
/// timestamp is `anchor_pts`.
///
/// Neighbours are consecutive displayed frames, never time-spaced samples:
/// "the frame before" means the previous frame the stream shows.
///
/// # Errors
///
/// Returns [`FrameSelectionError::AnchorNotListed`] when no listed frame
/// has `anchor_pts`.
pub fn plan_neighbours(
    listing: &FrameListing,
    anchor_pts: i64,
    count: NeighbourCount,
) -> Result<NeighbourPlan, FrameSelectionError> {
    let index = listing
        .frames
        .binary_search_by_key(&anchor_pts, |frame| frame.pts)
        .map_err(|_| FrameSelectionError::AnchorNotListed)?;
    let count = usize::from(count.get());
    let anchor = listing
        .frames
        .get(index)
        .copied()
        .ok_or(FrameSelectionError::AnchorNotListed)?;
    let first = index.saturating_sub(count);
    let before = listing
        .frames
        .get(first..index)
        .unwrap_or_default()
        .to_vec();
    let last = index
        .saturating_add(1)
        .saturating_add(count)
        .min(listing.frames.len());
    let after = listing
        .frames
        .get(index.saturating_add(1)..last)
        .unwrap_or_default()
        .to_vec();
    let before_stop = (before.len() < count).then(|| {
        if listing.covered.start().as_micros() == 0 {
            NeighbourStop::StartOfStream
        } else {
            NeighbourStop::SearchWindow
        }
    });
    let after_stop = (after.len() < count).then(|| {
        if listing.ends_stream() {
            NeighbourStop::EndOfStream
        } else {
            NeighbourStop::SearchWindow
        }
    });
    Ok(NeighbourPlan {
        anchor,
        before,
        before_stop,
        after,
        after_stop,
    })
}

/// Most frames a burst may return, 1 through [`MAX_BURST_FRAMES`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BurstCount(u8);

impl BurstCount {
    /// Creates a burst frame budget.
    ///
    /// # Errors
    ///
    /// Returns [`NavigationError::BurstCountOutOfRange`] outside
    /// `1..=MAX_BURST_FRAMES`.
    pub const fn new(count: u8) -> Result<Self, NavigationError> {
        if count == 0 || count > MAX_BURST_FRAMES {
            Err(NavigationError::BurstCountOutOfRange)
        } else {
            Ok(Self(count))
        }
    }

    /// Returns the budget.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// A burst's half-open range, at most [`MAX_BURST_RANGE_MICROS`] long.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BurstRange(TimeRange);

impl BurstRange {
    /// Validates a burst range.
    ///
    /// # Errors
    ///
    /// Returns [`NavigationError::BurstRangeTooLong`] beyond sixty seconds.
    pub const fn new(range: TimeRange) -> Result<Self, NavigationError> {
        if range.duration_micros() > MAX_BURST_RANGE_MICROS {
            Err(NavigationError::BurstRangeTooLong)
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

/// Whether a burst was planned over its whole requested range.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BurstExtent {
    /// The whole requested range.
    Requested,
    /// The requested range ran past the end of the source and was clipped to it.
    ClippedAtEndOfStream,
}

/// Distinct frames spread evenly in time over a range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BurstPlan {
    /// The range the targets were spread over.
    pub planned: TimeRange,
    /// Whether `planned` is the requested range or its clipped part.
    pub extent: BurstExtent,
    /// Number of evenly spaced target times (the budget).
    pub targets: u8,
    /// The distinct frames chosen, in presentation order. Fewer than
    /// `targets` when several targets named the same frame or a target had
    /// no frame before the range's end.
    pub frames: Vec<ListedFrame>,
}

impl BurstPlan {
    /// Returns the number of distinct frames chosen.
    #[must_use]
    pub fn distinct(&self) -> usize {
        self.frames.len()
    }

    /// Returns the evenly spaced target times: `start + k * duration /
    /// targets` for `k` in `0..targets`.
    #[must_use]
    pub fn target_times(&self) -> Vec<MediaTime> {
        burst_targets(self.planned, self.targets)
    }

    /// Returns the frame `target` names: the first chosen frame at or after
    /// it, or `None` when no chosen frame is.
    ///
    /// Every chosen frame is the first listed frame at or after its own
    /// target, so the first chosen frame at or after any target is exactly
    /// the frame that target named.
    #[must_use]
    pub fn frame_for(&self, target: MediaTime) -> Option<&ListedFrame> {
        self.frames.iter().find(|frame| frame.time >= target)
    }
}

/// The evenly spaced targets of a burst over `planned`.
fn burst_targets(planned: TimeRange, targets: u8) -> Vec<MediaTime> {
    let span = u128::from(planned.duration_micros());
    let start = planned.start().as_micros();
    (0..targets)
        .map(|step| {
            // step < targets, so the offset is below the span and fits in u64.
            let offset =
                u64::try_from(u128::from(step) * span / u128::from(targets)).unwrap_or(u64::MAX);
            MediaTime::from_micros(start.saturating_add(offset))
        })
        .collect()
}

/// Plans a burst: `count` target times evenly spaced from the range's start
/// (`start + k * duration / count`), each naming the first frame at or after
/// it and before the range's end; a frame named twice is returned once.
///
/// Even time targets make a burst describe a stretch of time the way a
/// person would sample it, independent of the frame rate; deduplication
/// keeps a static or slow stream from returning the same image repeatedly.
///
/// # Errors
///
/// Returns [`FrameSelectionError::AtOrAfterEnd`] when the range starts at or
/// after the end of the source and [`FrameSelectionError::OutsideListing`]
/// when the listing does not cover the range (or, for a range past the
/// source's end, its part before the end).
pub fn plan_burst(
    listing: &FrameListing,
    range: BurstRange,
    count: BurstCount,
) -> Result<BurstPlan, FrameSelectionError> {
    let requested = range.range();
    let covered = listing.covered;
    if listing.ends_stream() && requested.start() >= covered.end() {
        return Err(FrameSelectionError::AtOrAfterEnd);
    }
    if requested.start() < covered.start() {
        return Err(FrameSelectionError::OutsideListing);
    }
    let (planned, extent) = if requested.end() <= covered.end() {
        (requested, BurstExtent::Requested)
    } else if listing.ends_stream() {
        let clipped = TimeRange::new(requested.start(), covered.end())
            .map_err(|_| FrameSelectionError::AtOrAfterEnd)?;
        (clipped, BurstExtent::ClippedAtEndOfStream)
    } else {
        return Err(FrameSelectionError::OutsideListing);
    };
    let targets = count.get();
    let mut frames: Vec<ListedFrame> = Vec::with_capacity(usize::from(targets));
    for target in burst_targets(planned, targets) {
        let Some(frame) = listing.frames.get(listing.first_at_or_after(target)) else {
            break;
        };
        if frame.time >= planned.end() {
            break;
        }
        if frames.last().is_none_or(|last| last.pts != frame.pts) {
            frames.push(*frame);
        }
    }
    Ok(BurstPlan {
        planned,
        extent,
        targets,
        frames,
    })
}

/// Why a navigation value was rejected.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NavigationError {
    /// A tolerance exceeds [`MAX_FRAME_TOLERANCE_MICROS`].
    ToleranceTooLarge,
    /// A neighbour count is outside `1..=MAX_NEIGHBOUR_COUNT`.
    NeighbourCountOutOfRange,
    /// A burst budget is outside `1..=MAX_BURST_FRAMES`.
    BurstCountOutOfRange,
    /// A burst range exceeds [`MAX_BURST_RANGE_MICROS`].
    BurstRangeTooLong,
    /// A listing holds more than [`MAX_LISTED_FRAMES`] frames.
    ListingTooLarge,
    /// A listing is out of order or holds a frame outside its covered range.
    ListingInconsistent,
}

impl fmt::Display for NavigationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ToleranceTooLarge => "frame tolerance exceeds ten seconds",
            Self::NeighbourCountOutOfRange => "neighbour count must be 1 through 20",
            Self::BurstCountOutOfRange => "burst frame count must be 1 through 100",
            Self::BurstRangeTooLong => "burst range exceeds sixty seconds",
            Self::ListingTooLarge => "frame listing holds too many frames",
            Self::ListingInconsistent => "frame listing is out of order or outside its range",
        })
    }
}

impl Error for NavigationError {}

/// Why a listing could not name the requested frames.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FrameSelectionError {
    /// The request is at or after the end of the source.
    AtOrAfterEnd,
    /// No frame is displayed at or after the request: it follows the final frame.
    AfterFinalFrame,
    /// No frame is displayed yet at the request: it precedes the first frame.
    BeforeFirstFrame,
    /// The nearest frame the policy allows is further than the tolerance.
    NoFrameWithinTolerance,
    /// The listing does not cover enough of the stream to decide.
    OutsideListing,
    /// No listed frame has the anchor's timestamp.
    AnchorNotListed,
}

impl FrameSelectionError {
    /// Every selection failure.
    pub const ALL: [Self; 6] = [
        Self::AtOrAfterEnd,
        Self::AfterFinalFrame,
        Self::BeforeFirstFrame,
        Self::NoFrameWithinTolerance,
        Self::OutsideListing,
        Self::AnchorNotListed,
    ];

    /// Returns the stable identifier used in machine-readable contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::AtOrAfterEnd => "at_or_after_end",
            Self::AfterFinalFrame => "after_final_frame",
            Self::BeforeFirstFrame => "before_first_frame",
            Self::NoFrameWithinTolerance => "no_frame_within_tolerance",
            Self::OutsideListing => "outside_listing",
            Self::AnchorNotListed => "anchor_not_listed",
        }
    }
}

impl fmt::Display for FrameSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AtOrAfterEnd => "the requested time is at or after the end of the source",
            Self::AfterFinalFrame => "no frame is displayed at or after the requested time",
            Self::BeforeFirstFrame => "no frame is displayed yet at the requested time",
            Self::NoFrameWithinTolerance => "no displayed frame is within the tolerance",
            Self::OutsideListing => "the frame listing does not cover the request",
            Self::AnchorNotListed => "the anchor frame is not in the listing",
        })
    }
}

impl Error for FrameSelectionError {}

#[cfg(test)]
mod tests;
