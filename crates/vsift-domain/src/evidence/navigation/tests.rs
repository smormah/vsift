use proptest::prelude::{ProptestConfig, prop, prop_assert, prop_assert_eq, proptest};

use super::{
    BurstCount, BurstExtent, BurstRange, FrameListing, FrameSelection, FrameSelectionError,
    FrameTolerance, ListedFrame, ListingTail, MAX_BURST_RANGE_MICROS, MAX_LISTED_FRAMES,
    NavigationError, NeighbourCount, NeighbourStop, plan_burst, plan_neighbours, select_frame,
};
use crate::{MediaTime, TimeRange};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const fn at(micros: u64) -> MediaTime {
    MediaTime::from_micros(micros)
}

fn range(start: u64, end: u64) -> Result<TimeRange, Box<dyn std::error::Error>> {
    Ok(TimeRange::new(at(start), at(end))?)
}

/// F01's frames (20 fps, time base 1/10240, origin 0) in `[start, end)`.
fn f01(
    start: u64,
    end: u64,
    tail: ListingTail,
) -> Result<FrameListing, Box<dyn std::error::Error>> {
    let frames: Vec<ListedFrame> = (0..120_i64)
        .zip((0..120_u64).map(|index| at(index * 50_000)))
        .map(|(index, time)| ListedFrame {
            pts: index * 512,
            time,
        })
        .filter(|frame| frame.time >= at(start) && frame.time < at(end))
        .collect();
    Ok(FrameListing::new(range(start, end)?, frames, tail)?)
}

fn tolerance(micros: u64) -> Result<FrameTolerance, NavigationError> {
    FrameTolerance::new(micros)
}

#[test]
fn at_or_after_takes_the_first_frame_at_or_after_the_request() -> TestResult {
    let listing = f01(0, 6_000_000, ListingTail::EndOfStream)?;
    let select = |micros, policy| select_frame(&listing, at(micros), policy, FrameTolerance::MAX);
    let exact = select(2_000_000, FrameSelection::AtOrAfter)?;
    assert_eq!((exact.frame.pts, exact.timing.delta_micros()), (20_480, 0));
    let between = select(1_025_000, FrameSelection::AtOrAfter)?;
    assert_eq!(
        (between.timing.actual(), between.timing.delta_micros()),
        (at(1_050_000), 25_000)
    );
    assert_eq!(
        select(5_950_000, FrameSelection::AtOrAfter)?.frame.time,
        at(5_950_000)
    );
    assert_eq!(
        select(5_970_000, FrameSelection::AtOrAfter),
        Err(FrameSelectionError::AfterFinalFrame)
    );
    let displayed = select(5_970_000, FrameSelection::DisplayedAt)?;
    assert_eq!(
        (displayed.frame.time, displayed.timing.delta_micros()),
        (at(5_950_000), -20_000)
    );
    for policy in FrameSelection::ALL {
        assert_eq!(
            select(6_000_000, policy),
            Err(FrameSelectionError::AtOrAfterEnd)
        );
    }
    assert_eq!(FrameSelection::default(), FrameSelection::AtOrAfter);
    Ok(())
}

#[test]
fn tolerance_bounds_the_distance_in_the_policys_direction() -> TestResult {
    let listing = f01(0, 6_000_000, ListingTail::EndOfStream)?;
    assert_eq!(
        select_frame(
            &listing,
            at(1_025_000),
            FrameSelection::AtOrAfter,
            tolerance(24_999)?
        ),
        Err(FrameSelectionError::NoFrameWithinTolerance)
    );
    assert!(
        select_frame(
            &listing,
            at(1_025_000),
            FrameSelection::AtOrAfter,
            tolerance(25_000)?
        )
        .is_ok()
    );
    assert_eq!(
        select_frame(
            &listing,
            at(1_050_000),
            FrameSelection::AtOrAfter,
            FrameTolerance::EXACT
        )?
        .timing
        .delta_micros(),
        0
    );
    assert_eq!(
        select_frame(
            &listing,
            at(1_049_999),
            FrameSelection::DisplayedAt,
            tolerance(49_998)?
        ),
        Err(FrameSelectionError::NoFrameWithinTolerance)
    );
    assert_eq!(
        FrameTolerance::new(10_000_001),
        Err(NavigationError::ToleranceTooLarge)
    );
    Ok(())
}

#[test]
fn a_listing_that_cannot_decide_says_so() -> TestResult {
    // Frames 1.0 s..1.5 s listed; more may follow.
    let listing = f01(1_000_000, 1_500_000, ListingTail::MoreMayFollow)?;
    let exact = FrameTolerance::EXACT;
    assert_eq!(
        select_frame(&listing, at(900_000), FrameSelection::AtOrAfter, exact),
        Err(FrameSelectionError::OutsideListing)
    );
    assert_eq!(
        select_frame(&listing, at(1_500_000), FrameSelection::AtOrAfter, exact),
        Err(FrameSelectionError::OutsideListing)
    );
    // No frame in [1.46, 1.5) and 1.5 s onwards is unknown.
    let sparse = FrameListing::new(
        range(1_000_000, 1_500_000)?,
        vec![ListedFrame {
            pts: 10_240,
            time: at(1_000_000),
        }],
        ListingTail::MoreMayFollow,
    )?;
    assert_eq!(
        select_frame(
            &sparse,
            at(1_460_000),
            FrameSelection::AtOrAfter,
            tolerance(100_000)?
        ),
        Err(FrameSelectionError::OutsideListing)
    );
    assert_eq!(
        select_frame(
            &sparse,
            at(1_100_000),
            FrameSelection::AtOrAfter,
            tolerance(100_000)?
        ),
        Err(FrameSelectionError::NoFrameWithinTolerance)
    );
    // Displayed-at before the first listed frame of a later listing is
    // undecidable within the tolerance, decidable beyond it.
    let later = FrameListing::new(
        range(1_000_000, 1_500_000)?,
        vec![ListedFrame {
            pts: 12_288,
            time: at(1_200_000),
        }],
        ListingTail::MoreMayFollow,
    )?;
    assert_eq!(
        select_frame(
            &later,
            at(1_100_000),
            FrameSelection::DisplayedAt,
            tolerance(200_000)?
        ),
        Err(FrameSelectionError::OutsideListing)
    );
    assert_eq!(
        select_frame(
            &later,
            at(1_100_000),
            FrameSelection::DisplayedAt,
            tolerance(100_000)?
        ),
        Err(FrameSelectionError::NoFrameWithinTolerance)
    );
    let from_start = FrameListing::new(
        range(0, 1_500_000)?,
        vec![ListedFrame {
            pts: 12_288,
            time: at(1_200_000),
        }],
        ListingTail::MoreMayFollow,
    )?;
    assert_eq!(
        select_frame(
            &from_start,
            at(1_100_000),
            FrameSelection::DisplayedAt,
            exact
        ),
        Err(FrameSelectionError::BeforeFirstFrame)
    );
    Ok(())
}

#[test]
fn listings_must_be_ordered_bounded_and_inside_their_range() -> TestResult {
    let frame = |pts, micros| ListedFrame {
        pts,
        time: at(micros),
    };
    let covered = range(0, 1_000_000)?;
    for frames in [
        vec![frame(2, 20), frame(1, 30)],
        vec![frame(1, 20), frame(1, 30)],
        vec![frame(1, 30), frame(2, 20)],
        vec![frame(1, 1_000_000)],
    ] {
        assert_eq!(
            FrameListing::new(covered, frames, ListingTail::EndOfStream),
            Err(NavigationError::ListingInconsistent)
        );
    }
    // Two timestamps of a sub-microsecond time base may share a time.
    assert!(
        FrameListing::new(
            covered,
            vec![frame(1, 20), frame(2, 20)],
            ListingTail::EndOfStream
        )
        .is_ok()
    );
    let too_many: Vec<ListedFrame> = (0_i64..)
        .zip(0_u64..)
        .take(MAX_LISTED_FRAMES + 1)
        .map(|(pts, micros)| frame(pts, micros))
        .collect();
    assert_eq!(
        FrameListing::new(covered, too_many, ListingTail::EndOfStream),
        Err(NavigationError::ListingTooLarge)
    );
    Ok(())
}

#[test]
fn neighbours_are_consecutive_frames_with_typed_side_stops() -> TestResult {
    let whole = f01(0, 6_000_000, ListingTail::EndOfStream)?;
    let three = NeighbourCount::new(3)?;
    let middle = plan_neighbours(&whole, 20_480, three)?;
    let times = |frames: &[ListedFrame]| {
        frames
            .iter()
            .map(|frame| frame.time.as_micros())
            .collect::<Vec<_>>()
    };
    assert_eq!(times(&middle.before), vec![1_850_000, 1_900_000, 1_950_000]);
    assert_eq!(times(&middle.after), vec![2_050_000, 2_100_000, 2_150_000]);
    assert_eq!((middle.before_stop, middle.after_stop), (None, None));
    let first = plan_neighbours(&whole, 0, three)?;
    assert_eq!(
        (first.before.len(), first.before_stop),
        (0, Some(NeighbourStop::StartOfStream))
    );
    let last = plan_neighbours(&whole, 119 * 512, three)?;
    assert_eq!(
        (last.after.len(), last.after_stop),
        (0, Some(NeighbourStop::EndOfStream))
    );
    let window = f01(1_000_000, 2_000_000, ListingTail::MoreMayFollow)?;
    let edge = plan_neighbours(&window, 10_240 + 512, three)?;
    assert_eq!(
        (
            edge.before.len(),
            edge.before_stop,
            edge.after.len(),
            edge.after_stop
        ),
        (1, Some(NeighbourStop::SearchWindow), 3, None)
    );
    assert_eq!(
        plan_neighbours(&whole, 100, three),
        Err(FrameSelectionError::AnchorNotListed)
    );
    assert_eq!(
        NeighbourCount::new(0),
        Err(NavigationError::NeighbourCountOutOfRange)
    );
    assert_eq!(
        NeighbourCount::new(21),
        Err(NavigationError::NeighbourCountOutOfRange)
    );
    assert_eq!(
        NeighbourStop::ALL.map(NeighbourStop::identifier),
        ["start_of_stream", "end_of_stream", "search_window"]
    );
    Ok(())
}

#[test]
fn bursts_spread_even_targets_and_remove_duplicates() -> TestResult {
    let whole = f01(0, 6_000_000, ListingTail::EndOfStream)?;
    let plan = plan_burst(
        &whole,
        BurstRange::new(range(1_000_000, 2_000_000)?)?,
        BurstCount::new(4)?,
    )?;
    let times: Vec<u64> = plan
        .frames
        .iter()
        .map(|frame| frame.time.as_micros())
        .collect();
    assert_eq!(times, vec![1_000_000, 1_250_000, 1_500_000, 1_750_000]);
    assert_eq!(
        (plan.targets, plan.distinct(), plan.extent),
        (4, 4, BurstExtent::Requested)
    );
    // 100 targets over 0.2 s of a 20 fps stream name only four frames.
    let dense = plan_burst(
        &whole,
        BurstRange::new(range(1_000_000, 1_200_000)?)?,
        BurstCount::new(100)?,
    )?;
    assert_eq!((dense.targets, dense.distinct()), (100, 4));
    // A range past the end is clipped to the source.
    let clipped = plan_burst(
        &whole,
        BurstRange::new(range(5_800_000, 7_000_000)?)?,
        BurstCount::new(3)?,
    )?;
    assert_eq!(clipped.extent, BurstExtent::ClippedAtEndOfStream);
    assert_eq!(clipped.planned, range(5_800_000, 6_000_000)?);
    assert_eq!(
        plan_burst(
            &whole,
            BurstRange::new(range(6_000_000, 7_000_000)?)?,
            BurstCount::new(3)?
        ),
        Err(FrameSelectionError::AtOrAfterEnd)
    );
    let window = f01(1_000_000, 2_000_000, ListingTail::MoreMayFollow)?;
    assert_eq!(
        plan_burst(
            &window,
            BurstRange::new(range(1_500_000, 2_500_000)?)?,
            BurstCount::new(3)?
        ),
        Err(FrameSelectionError::OutsideListing)
    );
    assert_eq!(
        BurstCount::new(0),
        Err(NavigationError::BurstCountOutOfRange)
    );
    assert_eq!(
        BurstCount::new(101),
        Err(NavigationError::BurstCountOutOfRange)
    );
    assert_eq!(
        BurstRange::new(range(0, MAX_BURST_RANGE_MICROS + 1)?),
        Err(NavigationError::BurstRangeTooLong)
    );
    assert!(BurstRange::new(range(0, MAX_BURST_RANGE_MICROS)?).is_ok());
    Ok(())
}

/// An arbitrary listing: strictly increasing timestamps, non-decreasing times
/// inside `[start, start + span)`.
fn listing_strategy() -> impl prop::strategy::Strategy<Value = (u64, u64, Vec<u64>, bool)> {
    (
        0_u64..5_000_000,
        1_u64..=70_000_000,
        prop::collection::vec(0_u64..70_000_000, 0..200),
        prop::bool::ANY,
    )
}

fn build_listing(
    start: u64,
    span: u64,
    offsets: &[u64],
    ends_stream: bool,
) -> Result<FrameListing, Box<dyn std::error::Error>> {
    let mut times: Vec<u64> = offsets.iter().map(|offset| start + offset % span).collect();
    times.sort_unstable();
    let frames = times
        .iter()
        .enumerate()
        .map(|(index, micros)| {
            i64::try_from(index).map(|pts| ListedFrame {
                pts: pts * 7 + 3,
                time: at(*micros),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let tail = if ends_stream {
        ListingTail::EndOfStream
    } else {
        ListingTail::MoreMayFollow
    };
    Ok(FrameListing::new(
        range(start, start + span)?,
        frames,
        tail,
    )?)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn bursts_are_bounded_ordered_distinct_and_inside_their_range(
        (start, span, offsets, ends_stream) in listing_strategy(),
        from_offset in 0_u64..70_000_000,
        length in 1_u64..=MAX_BURST_RANGE_MICROS,
        count in 1_u8..=100,
    ) {
        let listing = build_listing(start, span, &offsets, ends_stream);
        prop_assert!(listing.is_ok());
        let Ok(listing) = listing else { return Ok(()); };
        let from = start + from_offset % span;
        let requested = range(from, from + length);
        prop_assert!(requested.is_ok());
        let Ok(requested) = requested else { return Ok(()); };
        let (Ok(burst_range), Ok(budget)) = (BurstRange::new(requested), BurstCount::new(count)) else {
            return Ok(());
        };
        match plan_burst(&listing, burst_range, budget) {
            Ok(plan) => {
                prop_assert!(plan.distinct() <= usize::from(count));
                prop_assert_eq!(plan.targets, count);
                prop_assert!(plan.frames.windows(2).all(|pair| matches!(pair, [a, b] if a.pts < b.pts)));
                prop_assert!(plan.frames.iter().all(|frame| frame.time >= plan.planned.start() && frame.time < plan.planned.end()));
                prop_assert!(plan.frames.iter().all(|frame| listing.frames().contains(frame)));
                prop_assert_eq!(plan.planned.start(), requested.start());
                prop_assert!(plan.planned.end() <= requested.end());
                // The first target is the range start: its frame is the first in range.
                let first_in_range = listing.frames().iter().find(|frame| frame.time >= plan.planned.start() && frame.time < plan.planned.end());
                prop_assert_eq!(plan.frames.first(), first_in_range);
            }
            Err(error) => prop_assert!(matches!(error, FrameSelectionError::OutsideListing | FrameSelectionError::AtOrAfterEnd)),
        }
    }

    #[test]
    fn neighbours_are_bounded_consecutive_and_stop_for_a_reason(
        (start, span, offsets, ends_stream) in listing_strategy(),
        pick in 0_usize..200,
        count in 1_u8..=20,
    ) {
        let listing = build_listing(start, span, &offsets, ends_stream);
        prop_assert!(listing.is_ok());
        let Ok(listing) = listing else { return Ok(()); };
        let frames = listing.frames();
        let Some(anchor) = frames.get(pick % frames.len().max(1)).copied() else { return Ok(()); };
        let per_side = NeighbourCount::new(count);
        prop_assert!(per_side.is_ok());
        let Ok(per_side) = per_side else { return Ok(()); };
        let plan = plan_neighbours(&listing, anchor.pts, per_side);
        prop_assert!(plan.is_ok());
        let Ok(plan) = plan else { return Ok(()); };
        let index = frames.iter().position(|frame| *frame == anchor).unwrap_or(0);
        let mut joined = plan.before.clone();
        joined.push(plan.anchor);
        joined.extend(plan.after.iter().copied());
        let first = index - plan.before.len();
        prop_assert_eq!(frames.get(first..first + joined.len()), Some(joined.as_slice()));
        prop_assert!(plan.before.len() <= usize::from(count) && plan.after.len() <= usize::from(count));
        prop_assert_eq!(plan.before_stop.is_some(), plan.before.len() < usize::from(count));
        prop_assert_eq!(plan.after_stop.is_some(), plan.after.len() < usize::from(count));
    }

    #[test]
    fn a_selected_frame_obeys_its_policy_and_tolerance(
        (start, span, offsets, ends_stream) in listing_strategy(),
        request_offset in 0_u64..80_000_000,
        tolerance_micros in 0_u64..=10_000_000,
        displayed_at in prop::bool::ANY,
    ) {
        let listing = build_listing(start, span, &offsets, ends_stream);
        prop_assert!(listing.is_ok());
        let Ok(listing) = listing else { return Ok(()); };
        let request = at(start + request_offset);
        let policy = if displayed_at { FrameSelection::DisplayedAt } else { FrameSelection::AtOrAfter };
        let limit = FrameTolerance::new(tolerance_micros);
        prop_assert!(limit.is_ok());
        let Ok(limit) = limit else { return Ok(()); };
        if let Ok(selected) = select_frame(&listing, request, policy, limit) {
            prop_assert!(listing.frames().contains(&selected.frame));
            prop_assert!(selected.timing.delta_micros().unsigned_abs() <= tolerance_micros);
            match policy {
                FrameSelection::AtOrAfter => {
                    prop_assert!(selected.frame.time >= request);
                    prop_assert!(listing.frames().iter().all(|frame| frame.time < request || frame.time >= selected.frame.time));
                }
                FrameSelection::DisplayedAt => {
                    prop_assert!(selected.frame.time <= request);
                    prop_assert!(listing.frames().iter().all(|frame| frame.time > request || frame.time <= selected.frame.time));
                }
            }
        }
    }
}
