//! The clock model of ADR 0029 decisions 10 to 13.

use std::error::Error;

use proptest::prelude::*;

use super::*;
use crate::FrameDimensions;

type TestResult = Result<(), Box<dyn Error>>;

const SECOND: u64 = 1_000_000;
const GRID: u64 = 10;
/// Long enough for every model to be judged: 30 minutes on the 10 s grid.
const DURATION: MediaTime = MediaTime::from_micros(4 * 3_600 * SECOND);

fn region(x: u32, y: u32) -> Result<TextRegion, Box<dyn Error>> {
    let frame = FrameDimensions::new(1920, 1080)?;
    Ok(TextRegion::new(x, y, 60, 14, frame)?)
}

fn read(at_seconds: u64, text: &str, place: TextRegion) -> Result<ClockRead, Box<dyn Error>> {
    Ok(ClockRead {
        time: MediaTime::from_micros(at_seconds * SECOND),
        pts: i64::try_from(at_seconds)?,
        region: place,
        face: read_clock_face(text)?,
    })
}

fn minute_text(second_of_day: u64) -> String {
    let second = second_of_day % 86_400;
    format!("{:02}:{:02}", second / 3_600, second / 60 % 60)
}

fn second_text(second_of_day: u64) -> String {
    let second = second_of_day % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        second / 3_600,
        second / 60 % 60,
        second % 60
    )
}

/// Reads of a clock every 10 s from `from` to `to`, showing `start` at time 0.
fn minute_reads(
    start: u64,
    from: u64,
    to: u64,
    place: TextRegion,
) -> Result<Vec<ClockRead>, Box<dyn Error>> {
    (from / GRID..to / GRID)
        .map(|step| {
            let at = step * GRID;
            read(at, &minute_text(start + at), place)
        })
        .collect()
}

fn analyse(reads: Vec<ClockRead>) -> ClockAnalysis {
    analyse_reads(reads, ClockRefusals::default(), DURATION)
}

fn only_clock(analysis: &ClockAnalysis) -> Result<&ScreenClock, Box<dyn Error>> {
    match analysis.clocks.as_slice() {
        [clock] => Ok(clock),
        clocks => Err(format!("{} clocks", clocks.len()).into()),
    }
}

/// The offset a clock showing `start` at time 0 has, as the model counts it.
fn offset_of(start: u64) -> i64 {
    i64::try_from(start * SECOND).unwrap_or(i64::MAX)
}

#[test]
fn a_moving_face_is_a_clock_and_its_interval_holds_the_true_offset() -> TestResult {
    let start = 14 * 3_600 + 3 * 60 + 25;
    let analysis = analyse(minute_reads(start, 0, 300, region(1800, 1040)?)?);
    let clock = only_clock(&analysis)?;
    assert_eq!(clock.kind(), ClockKind::Clock);
    assert_eq!(clock.format(), ClockFormat::TwentyFourHour);
    let offset = clock.tightest_offset().ok_or("no offset")?;
    assert!(offset.contains(offset_of(start)), "{offset:?}");
    assert!(
        offset.width() <= 10_250_000,
        "a 10 s grid narrows a minute to 10 s: {offset:?}"
    );
    Ok(())
}

#[test]
fn a_time_that_stands_still_is_not_a_clock() -> TestResult {
    let place = region(300, 200)?;
    let reads = (0..30)
        .map(|step| read(step * GRID, "10:32", place))
        .collect::<Result<Vec<_>, _>>()?;
    let analysis = analyse(reads);
    assert_eq!(analysis.clocks, Vec::new());
    assert_eq!(analysis.refusals.stood_still, 30);
    Ok(())
}

#[test]
fn a_countdown_is_not_a_clock() -> TestResult {
    let place = region(300, 200)?;
    let reads = (0..30_u64)
        .map(|step| read(step * GRID, &second_text(7_200 - step * GRID), place))
        .collect::<Result<Vec<_>, _>>()?;
    let analysis = analyse(reads);
    assert_eq!(analysis.clocks, Vec::new());
    assert_eq!(analysis.refusals.irregular, 30);
    Ok(())
}

#[test]
fn a_log_scrolling_past_is_not_a_clock() -> TestResult {
    let mut reads = Vec::new();
    for step in 0..30_u32 {
        let time = second_text(50_000 + u64::from(step) * 7);
        reads.push(read(
            u64::from(step) * GRID,
            &time,
            region(40, 900 - step * 16)?,
        )?);
    }
    assert_eq!(analyse(reads).clocks, Vec::new());
    Ok(())
}

#[test]
fn a_clock_keeps_its_offset_past_midnight() -> TestResult {
    let start = 86_400 - 90;
    let analysis = analyse(minute_reads(start, 0, 300, region(1800, 1040)?)?);
    let clock = only_clock(&analysis)?;
    assert_eq!(clock.runs().len(), 1);
    assert_eq!(clock.jumps(), []);
    assert!(
        clock
            .tightest_offset()
            .ok_or("no offset")?
            .contains(offset_of(start))
    );
    Ok(())
}

#[test]
fn two_zones_are_two_clocks_and_nothing_is_a_jump() -> TestResult {
    let start = 14 * 3_600;
    let mut reads = minute_reads(start, 0, 300, region(1800, 1040)?)?;
    reads.extend(minute_reads(start + 5 * 3_600, 0, 300, region(200, 40)?)?);
    let analysis = analyse(reads);
    assert_eq!(analysis.clocks.len(), 2);
    assert!(analysis.clocks.iter().all(|clock| clock.jumps().is_empty()));
    Ok(())
}

#[test]
fn a_change_of_presenter_on_whole_hours_is_two_clocks_not_a_cut() -> TestResult {
    let place = region(1800, 1040)?;
    let mut reads = minute_reads(14 * 3_600, 0, 300, place)?;
    reads.extend(minute_reads(19 * 3_600, 300, 600, place)?);
    let analysis = analyse(reads);
    assert_eq!(analysis.clocks.len(), 2);
    assert!(analysis.clocks.iter().all(|clock| clock.jumps().is_empty()));
    Ok(())
}

#[test]
fn a_cut_is_a_gap_of_its_length() -> TestResult {
    let place = region(1800, 1040)?;
    let start = 14 * 3_600;
    let mut reads = minute_reads(start, 0, 300, place)?;
    reads.extend(minute_reads(start + 437, 300, 600, place)?);
    let analysis = analyse(reads);
    let clock = only_clock(&analysis)?;
    let [jump] = clock.jumps() else {
        return Err("no single jump".into());
    };
    assert_eq!(jump.kind, ClockJumpKind::Gap);
    assert!(jump.jump.contains(437_000_000), "{:?}", jump.jump);
    Ok(())
}

#[test]
fn machines_a_little_apart_read_as_a_gap_which_is_why_it_changes_no_value() -> TestResult {
    let place = region(1800, 1040)?;
    let mut reads = minute_reads(14 * 3_600, 0, 300, place)?;
    reads.extend(minute_reads(14 * 3_600 + 75, 300, 600, place)?);
    let analysis = analyse(reads);
    assert_eq!(
        only_clock(&analysis)?.jumps().first().map(|jump| jump.kind),
        Some(ClockJumpKind::Gap)
    );
    Ok(())
}

#[test]
fn a_clock_set_back_is_a_backward_jump() -> TestResult {
    let place = region(1800, 1040)?;
    let mut reads = minute_reads(14 * 3_600 + 500, 0, 300, place)?;
    reads.extend(minute_reads(14 * 3_600, 300, 600, place)?);
    let analysis = analyse(reads);
    assert_eq!(
        only_clock(&analysis)?.jumps().first().map(|jump| jump.kind),
        Some(ClockJumpKind::Backward)
    );
    Ok(())
}

#[test]
fn a_stopped_screen_that_resumes_true_is_a_freeze_and_no_jump() -> TestResult {
    let place = region(1800, 1040)?;
    let start = 14 * 3_600;
    let mut reads = minute_reads(start, 0, 300, place)?;
    for at in (300..500).step_by(10) {
        reads.push(read(at, &minute_text(start + 290), place)?);
    }
    reads.extend(minute_reads(start, 500, 800, place)?);
    let analysis = analyse(reads);
    let clock = only_clock(&analysis)?;
    assert_eq!(clock.jumps(), []);
    assert_eq!(clock.excursions(), []);
    assert_eq!(clock.freezes().len(), 1);
    assert!(
        clock
            .tightest_offset()
            .ok_or("no offset")?
            .contains(offset_of(start))
    );
    Ok(())
}

#[test]
fn a_timer_past_its_first_hour_is_a_counter() -> TestResult {
    // It started 100 s into the recording, so its offset is -100 s: it read
    // zero during the recording, though every read of it shows hour 01.
    let place = region(900, 40)?;
    let reads = (0..30_u64)
        .map(|step| {
            let at = 3_800 + step * GRID;
            read(at, &second_text(at - 100), place)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let analysis = analyse_reads(
        reads,
        ClockRefusals::default(),
        MediaTime::from_micros(4_200 * SECOND),
    );
    let clock = only_clock(&analysis)?;
    assert_eq!(clock.kind(), ClockKind::Counter);
    assert_eq!(clock.anchor(), None);
    Ok(())
}

#[test]
fn a_timer_in_its_first_hour_is_a_counter_however_it_began() -> TestResult {
    let place = region(900, 40)?;
    let reads = (0..30_u64)
        .map(|step| read(step * GRID, &second_text(120 + step * GRID), place))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(only_clock(&analyse(reads))?.kind(), ClockKind::Counter);
    Ok(())
}

/// A known limit (L-213): nothing tells a timer that began before the recording
/// and is past its first hour from a clock. It has no date, so it gives no anchor.
#[test]
fn a_timer_begun_before_the_recording_and_past_its_hour_reads_as_a_clock() -> TestResult {
    let place = region(900, 40)?;
    let reads = (0..30_u64)
        .map(|step| read(step * GRID, &second_text(3_700 + step * GRID), place))
        .collect::<Result<Vec<_>, _>>()?;
    let analysis = analyse(reads);
    let clock = only_clock(&analysis)?;
    assert_eq!(clock.kind(), ClockKind::Clock);
    assert_eq!(clock.anchor(), None);
    Ok(())
}

#[test]
fn a_seconds_clock_far_from_zero_is_a_clock() -> TestResult {
    let place = region(900, 40)?;
    let reads = (0..30_u64)
        .map(|step| read(step * GRID, &second_text(50_000 + step * GRID), place))
        .collect::<Result<Vec<_>, _>>()?;
    let analysis = analyse(reads);
    assert_eq!(only_clock(&analysis)?.kind(), ClockKind::Clock);
    Ok(())
}

#[test]
fn a_counter_near_the_recording_length_keeps_its_rule_at_the_cost_of_midnight() -> TestResult {
    // A real clock at 00:30:00 reads as a counter; ADR 0029 decision 10 accepts that.
    let place = region(900, 40)?;
    let reads = (0..30_u64)
        .map(|step| read(step * GRID, &second_text(1_800 + step * GRID), place))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(only_clock(&analyse(reads))?.kind(), ClockKind::Counter);
    Ok(())
}

#[test]
fn a_time_of_day_gives_no_anchor_and_a_dated_clock_gives_its_reading() -> TestResult {
    let place = region(1800, 1040)?;
    let analysis = analyse(minute_reads(14 * 3_600, 0, 300, place)?);
    assert_eq!(only_clock(&analysis)?.anchor(), None);

    let reads = (0..30_u64)
        .map(|step| {
            let at = step * GRID;
            read(
                at,
                &format!("2026-10-21T{}Z", second_text(50_000 + at)),
                place,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let analysis = analyse(reads);
    let clock = only_clock(&analysis)?;
    assert_eq!(clock.format(), ClockFormat::DatedWithZone);
    let (time, reading, precision) = clock.anchor().ok_or("no anchor")?;
    assert_eq!(time, MediaTime::from_micros(0));
    assert_eq!(reading, ClockReading::parse("2026-10-21T13:53:20Z")?);
    assert!(precision.width() <= 1_250_000);
    Ok(())
}

#[test]
fn a_moved_window_is_one_clock() -> TestResult {
    let start = 14 * 3_600;
    let mut reads = minute_reads(start, 0, 300, region(1800, 1040)?)?;
    reads.extend(minute_reads(start, 300, 600, region(40, 40)?)?);
    let analysis = analyse(reads);
    assert_eq!(only_clock(&analysis)?.read_count(), 60);
    Ok(())
}

#[test]
fn the_search_narrows_a_rollover_and_stops() -> TestResult {
    let place = region(1800, 1040)?;
    let start = 14 * 3_600 + 25;
    let mut reads = minute_reads(start, 0, 300, place)?;
    let mut rounds = 0;
    loop {
        let analysis = analyse(reads.clone());
        let next = analysis.next_reads();
        if next.is_empty() {
            let offset = only_clock(&analysis)?
                .tightest_offset()
                .ok_or("no offset")?;
            assert!(offset.contains(offset_of(start)), "{offset:?}");
            assert!(offset.width() <= 1_000_000, "{offset:?}");
            break;
        }
        rounds += 1;
        assert!(rounds <= 8, "a search of a 10 s bracket ends");
        for time in next {
            let at = time.as_micros();
            let shown = start * SECOND + at;
            reads.push(ClockRead {
                time,
                pts: i64::try_from(at)?,
                region: place,
                face: read_clock_face(&minute_text(shown / SECOND))?,
            });
        }
    }
    Ok(())
}

#[test]
fn a_clock_that_only_has_seconds_needs_no_search() -> TestResult {
    let place = region(900, 40)?;
    let reads = (0..30_u64)
        .map(|step| read(step * GRID, &second_text(50_000 + step * GRID), place))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(analyse(reads).next_reads(), Vec::new());
    Ok(())
}

#[test]
fn the_search_never_brackets_across_an_excursions_reads() -> TestResult {
    let place = region(1800, 1040)?;
    let start = 14 * 3_600;
    // 14:02 is read as 14:08 across the rollover from 14:02 to 14:03.
    let mut reads = minute_reads(start, 0, 130, place)?;
    for at in [130, 140, 150, 160, 170] {
        reads.push(read(at, "14:08", place)?);
    }
    reads.extend(minute_reads(start, 180, 300, place)?);
    let analysis = analyse(reads);
    let clock = only_clock(&analysis)?;
    assert_eq!(clock.excursions().len(), 1);
    for time in analysis.next_reads() {
        assert!(
            !(130 * SECOND..=180 * SECOND).contains(&time.as_micros()),
            "{time:?} lies inside the excursion"
        );
    }
    Ok(())
}

/// The planner's condition (ADR 0029 decision 12): a misread ends that bracket's
/// search at the width it has, moves the search to the next rollover, and shows.
#[test]
fn a_misread_costs_one_rollover_and_the_report_shows_which() -> TestResult {
    let place = region(1800, 1040)?;
    let start = 14 * 3_600;
    let mut reads = minute_reads(start, 0, 130, place)?;
    for at in [130, 140, 150, 160, 170] {
        reads.push(read(at, "14:08", place)?);
    }
    reads.extend(minute_reads(start, 180, 300, place)?);
    let analysis = analyse(reads);
    let clock = only_clock(&analysis)?;
    let [bracket] = clock
        .stopped_brackets()
        .try_into()
        .map_err(|_| "no single bracket")?;
    assert_eq!(
        bracket,
        (
            MediaTime::from_micros(120 * SECOND),
            MediaTime::from_micros(180 * SECOND)
        )
    );
    let next = analysis.next_reads();
    assert_eq!(
        next.first(),
        Some(&MediaTime::from_micros(55 * SECOND)),
        "the search stays with the first rollover it can use"
    );
    assert!(
        next.iter().any(|time| time.as_micros() > 180 * SECOND),
        "and the last rollover is still searched: {next:?}"
    );
    Ok(())
}

#[test]
fn a_clock_with_no_excursion_has_no_stopped_bracket() -> TestResult {
    let analysis = analyse(minute_reads(14 * 3_600, 0, 300, region(1800, 1040)?)?);
    assert_eq!(only_clock(&analysis)?.stopped_brackets(), Vec::new());
    Ok(())
}

#[test]
fn a_rate_is_judged_across_runs_and_only_when_proved() -> TestResult {
    let place = region(900, 40)?;
    let steady = (0..60_u64)
        .map(|step| read(step * GRID, &second_text(50_000 + step * GRID), place))
        .collect::<Result<Vec<_>, _>>()?;
    assert!(
        only_clock(&analyse(steady))?
            .rate()
            .is_none_or(|rate| !rate.differs)
    );

    let fast = (0..60_u64)
        .map(|step| {
            read(
                step * GRID,
                &second_text(50_000 + step * GRID * 101 / 100),
                place,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let analysis = analyse(fast);
    let rate = only_clock(&analysis)?.rate().ok_or("no rate")?;
    assert!(rate.differs, "{rate:?}");
    assert!(rate.drift_micros > 0, "{rate:?}");
    Ok(())
}

#[test]
fn a_stop_never_reads_as_a_rate() -> TestResult {
    let place = region(1800, 1040)?;
    let start = 14 * 3_600;
    let mut reads = minute_reads(start, 0, 300, place)?;
    for at in (300..500).step_by(10) {
        reads.push(read(at, &minute_text(start + 290), place)?);
    }
    reads.extend(minute_reads(start - 200, 500, 800, place)?);
    assert!(
        only_clock(&analyse(reads))?
            .rate()
            .is_none_or(|rate| !rate.differs)
    );
    Ok(())
}

#[test]
fn intervals_meet_only_where_they_share_a_point() {
    let at = MediaTime::from_micros(0);
    let first = OffsetInterval::of_reading(0, ClockUnit::Second, at);
    let touching = OffsetInterval::of_reading(1_250_000, ClockUnit::Second, at);
    assert_eq!(first.meet(touching), None);
    let overlapping = OffsetInterval::of_reading(1_000_000, ClockUnit::Second, at);
    assert_eq!(
        first.meet(overlapping).map(OffsetInterval::width),
        Some(250_000)
    );
}

/// The digit `position` of `HH:MM` changed to the first other digit that leaves
/// a valid time.
fn misread(text: &str, position: usize) -> Option<String> {
    let index = [0, 1, 3, 4][position];
    (b'0'..=b'9').find_map(|digit| {
        let mut bytes = text.as_bytes().to_vec();
        if bytes[index] == digit {
            return None;
        }
        bytes[index] = digit;
        let changed = String::from_utf8(bytes).ok()?;
        let parsed = read_clock_face(&changed).ok()?;
        let original = read_clock_face(text).ok()?;
        // Next to the true minute a misread can fit the interval: a real limit
        // of a minute face, and a different question.
        let apart = parsed.second_of_day().abs_diff(original.second_of_day());
        (apart >= 180).then_some(changed)
    })
}

proptest! {
    #[test]
    fn the_true_offset_is_always_inside(start in 0_u64..86_400, minutes in 3_u64..30) {
        let Ok(place) = region(1800, 1040) else { return Err(TestCaseError::fail("region")); };
        let Ok(reads) = minute_reads(start, 0, minutes * 60, place) else {
            return Err(TestCaseError::fail("reads"));
        };
        let analysis = analyse(reads);
        prop_assert_eq!(analysis.clocks.len(), 1);
        for clock in &analysis.clocks {
            prop_assert_eq!(clock.jumps(), &[]);
            for run in clock.runs() {
                let true_offset = offset_of(start);
                let day = 86_400_000_000;
                prop_assert!(
                    [true_offset - day, true_offset, true_offset + day]
                        .iter()
                        .any(|offset| run.offset().contains(*offset)),
                    "{:?} {}", run.offset(), start
                );
            }
        }
    }

    #[test]
    fn the_order_reads_arrive_in_changes_nothing(start in 0_u64..86_400, minutes in 3_u64..12) {
        let Ok(place) = region(1800, 1040) else { return Err(TestCaseError::fail("region")); };
        let Ok(reads) = minute_reads(start, 0, minutes * 60, place) else {
            return Err(TestCaseError::fail("reads"));
        };
        let reversed: Vec<ClockRead> = reads.iter().rev().copied().collect();
        prop_assert_eq!(analyse(reads), analyse(reversed));
    }

    #[test]
    fn a_read_that_fits_never_widens_the_interval(start in 0_u64..86_400, minutes in 4_u64..12) {
        let Ok(place) = region(1800, 1040) else { return Err(TestCaseError::fail("region")); };
        let Ok(reads) = minute_reads(start, 0, minutes * 60, place) else {
            return Err(TestCaseError::fail("reads"));
        };
        let width = |reads: &[ClockRead]| {
            analyse(reads.to_vec()).clocks.first().and_then(ScreenClock::tightest_offset)
                .map(OffsetInterval::width)
        };
        let half = reads.len() / 2;
        prop_assert!(width(&reads) <= width(&reads[..half]).or(Some(i64::MAX)));
    }

    /// The planner's rule (ADR 0029 decision 13): a misread held across every
    /// read of one minute, at each digit position, is an excursion. No gap, the
    /// offset unchanged, and the reads reported, never lost.
    #[test]
    fn a_held_misread_is_an_excursion(
        hour in 0_u64..24,
        minute_of_hour in 0_u64..60,
        position in 0_usize..4,
        at_minute in 3_u64..12,
    ) {
        let start = hour * 3_600 + minute_of_hour * 60;
        let Ok(place) = region(1800, 1040) else { return Err(TestCaseError::fail("region")); };
        let mut reads = Vec::new();
        let mut held = 0;
        for step in 0..18 * 6_u64 {
            let at = step * GRID;
            let mut text = minute_text(start + at);
            if at / 60 == at_minute
                && let Some(wrong) = misread(&text, position)
            {
                text = wrong;
                held += 1;
            }
            let Ok(next) = read(at, &text, place) else {
                return Err(TestCaseError::fail("read"));
            };
            reads.push(next);
        }
        prop_assume!(held == 6);
        let analysis = analyse(reads);
        prop_assert_eq!(analysis.clocks.len(), 1);
        for clock in &analysis.clocks {
            prop_assert_eq!(clock.jumps(), &[]);
            prop_assert_eq!(clock.excursions().len(), 1);
            let reported: usize = clock.excursions().iter().map(|stretch| stretch.reads().len()).sum();
            prop_assert_eq!(reported, 6);
            prop_assert_eq!(clock.read_count(), 18 * 6 - 6);
            let Some(offset) = clock.tightest_offset() else {
                return Err(TestCaseError::fail("no offset"));
            };
            let true_offset = offset_of(start);
            let day = 86_400_000_000;
            prop_assert!(
                [true_offset - day, true_offset, true_offset + day]
                    .iter()
                    .any(|candidate| offset.contains(*candidate)),
                "{:?}", offset
            );
        }
    }
}
