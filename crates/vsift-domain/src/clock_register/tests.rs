//! The session's named clocks (ADR 0029 decision 18).

use std::error::Error;

use super::*;
use crate::{
    ClockRead, ClockRefusals, FrameDimensions, MediaTime, TextRegion, analyse_reads,
    read_clock_face,
};

type TestResult = Result<(), Box<dyn Error>>;

const SECOND: u64 = 1_000_000;

fn minute_clock(start: u64, from: u64, to: u64, x: u32) -> Result<Vec<ClockRead>, Box<dyn Error>> {
    let frame = FrameDimensions::new(1920, 1080)?;
    let region = TextRegion::new(x, 1040, 60, 14, frame)?;
    (from / 10..to / 10)
        .map(|step| {
            let at = step * 10;
            let shown = (start + at) % 86_400;
            Ok(ClockRead {
                time: MediaTime::from_micros(at * SECOND),
                pts: i64::try_from(at)?,
                region,
                face: read_clock_face(&format!("{:02}:{:02}", shown / 3_600, shown / 60 % 60))?,
            })
        })
        .collect()
}

fn analyse(reads: Vec<ClockRead>) -> ClockAnalysis {
    analyse_reads(
        reads,
        ClockRefusals::default(),
        MediaTime::from_micros(3_600 * SECOND),
    )
}

#[test]
fn clocks_are_named_in_order_of_first_appearance() -> TestResult {
    let mut reads = minute_clock(14 * 3_600, 0, 300, 1800)?;
    reads.extend(minute_clock(19 * 3_600, 100, 300, 200)?);
    let analysis = analyse(reads);
    let register = ClockRegister::of(&[], &[], &analysis);
    assert_eq!(register.id_of(0), ClockId::new(1));
    assert_eq!(register.id_of(1), ClockId::new(2));
    assert_eq!(register.entries().len(), 2);
    Ok(())
}

#[test]
fn an_id_survives_when_a_later_read_finds_an_earlier_clock() -> TestResult {
    // The second call has read earlier frames, so the other clock now comes first.
    let later = minute_clock(19 * 3_600, 200, 500, 200)?;
    let first = ClockRegister::of(&[], &[], &analyse(later.clone()));
    let mut reads = later;
    reads.extend(minute_clock(14 * 3_600, 0, 500, 1800)?);
    let analysis = analyse(reads);
    let second = ClockRegister::of(first.entries(), first.gaps(), &analysis);
    assert_eq!(
        second.id_of(0),
        ClockId::new(2),
        "the new clock comes first in time"
    );
    assert_eq!(
        second.id_of(1),
        ClockId::new(1),
        "the known clock keeps its id"
    );
    Ok(())
}

#[test]
fn a_clock_past_the_bound_takes_no_id_and_the_rest_are_kept() -> TestResult {
    let mut reads = Vec::new();
    for clock in 0..9_u32 {
        reads.extend(minute_clock(
            u64::from(clock) * 3_600,
            0,
            300,
            100 + clock * 200,
        )?);
    }
    let analysis = analyse(reads);
    assert_eq!(analysis.clocks.len(), 9);
    let register = ClockRegister::of(&[], &[], &analysis);
    assert_eq!(register.entries().len(), MAX_SCREEN_CLOCKS);
    assert_eq!(register.id_of(8), None);
    assert_eq!(register.id_of(7), ClockId::new(8));
    Ok(())
}

#[test]
fn a_confirmation_stays_with_its_clock() -> TestResult {
    let analysis = analyse(minute_clock(14 * 3_600, 0, 300, 1800)?);
    let mut entries = ClockRegister::of(&[], &[], &analysis).entries().to_vec();
    let offset = UtcOffset::from_seconds(-18_000)?;
    entries[0].confirmed_offset = Some(offset);
    let again = ClockRegister::of(&entries, &[], &analysis);
    assert_eq!(
        again
            .entries()
            .first()
            .and_then(|entry| entry.confirmed_offset),
        Some(offset)
    );
    Ok(())
}

#[test]
fn a_clock_with_no_date_gives_no_anchor() -> TestResult {
    let analysis = analyse(minute_clock(14 * 3_600, 0, 300, 1800)?);
    assert_eq!(
        ClockRegister::of(&[], &[], &analysis).screen_anchors(&analysis, &TimeAnchors::none()),
        Vec::new()
    );
    Ok(())
}

fn with_cut(start: u64, cut_at: u64, cut: u64) -> Result<ClockAnalysis, Box<dyn Error>> {
    let mut reads = minute_clock(start, 0, cut_at, 1800)?;
    reads.extend(minute_clock(start + cut, cut_at, cut_at + 300, 1800)?);
    Ok(analyse(reads))
}

fn stated(text: &str) -> Result<TimeAnchors, Box<dyn Error>> {
    Ok(TimeAnchors::from_ingest(
        Some(ClockReading::parse(text)?),
        &[],
        0,
    ))
}

#[test]
fn a_gap_is_detected_named_and_kept_through_a_recount() -> TestResult {
    let analysis = with_cut(14 * 3_600, 300, 437)?;
    let first = ClockRegister::of(&[], &[], &analysis);
    let [gap] = first.gaps() else {
        return Err("no single gap".into());
    };
    assert_eq!(gap.state, GapState::Detected);
    assert_eq!(gap.id, GapId::new(1).ok_or("no id")?);
    let again = ClockRegister::of(first.entries(), first.gaps(), &analysis);
    assert_eq!(again.gaps(), first.gaps());
    Ok(())
}

#[test]
fn only_a_statement_accepts_a_gap_and_it_stays_accepted() -> TestResult {
    let analysis = with_cut(14 * 3_600, 300, 437)?;
    let mut register = ClockRegister::of(&[], &[], &analysis);
    let id = GapId::new(1).ok_or("no id")?;
    register.accept_gap(id)?;
    let again = ClockRegister::of(register.entries(), register.gaps(), &analysis);
    assert_eq!(
        again.gaps().first().map(|gap| gap.state),
        Some(GapState::Accepted)
    );
    assert_eq!(
        register.accept_gap(GapId::new(9).ok_or("no id")?),
        Err(ClockStatementError::UnknownGap)
    );
    Ok(())
}

#[test]
fn an_undone_detection_drops_a_detected_gap_but_never_an_accepted_one() -> TestResult {
    let cut = with_cut(14 * 3_600, 300, 437)?;
    let whole = analyse(minute_clock(14 * 3_600, 0, 600, 1800)?);
    let mut register = ClockRegister::of(&[], &[], &cut);
    assert_eq!(
        ClockRegister::of(register.entries(), register.gaps(), &whole).gaps(),
        []
    );
    register.accept_gap(GapId::new(1).ok_or("no id")?)?;
    assert_eq!(
        ClockRegister::of(register.entries(), register.gaps(), &whole)
            .gaps()
            .len(),
        1
    );
    Ok(())
}

#[test]
fn one_cut_seen_on_two_clocks_is_one_gap() -> TestResult {
    let mut reads = minute_clock(14 * 3_600, 0, 300, 1800)?;
    reads.extend(minute_clock(14 * 3_600 + 437, 300, 600, 1800)?);
    reads.extend(minute_clock(19 * 3_600, 0, 300, 200)?);
    reads.extend(minute_clock(19 * 3_600 + 437, 300, 600, 200)?);
    let analysis = analyse(reads);
    assert_eq!(analysis.clocks.len(), 2);
    let register = ClockRegister::of(&[], &[], &analysis);
    assert_eq!(register.gaps().len(), 1);
    assert_eq!(register.conflicts(&analysis), Vec::new());
    Ok(())
}

#[test]
fn a_jump_another_clock_does_not_share_is_a_conflict() -> TestResult {
    let mut reads = minute_clock(14 * 3_600, 0, 300, 1800)?;
    reads.extend(minute_clock(14 * 3_600 + 437, 300, 600, 1800)?);
    reads.extend(minute_clock(19 * 3_600, 0, 600, 200)?);
    let analysis = analyse(reads);
    let register = ClockRegister::of(&[], &[], &analysis);
    assert!(
        matches!(
            register.conflicts(&analysis).as_slice(),
            [ClockConflict::JumpNotShared { .. }]
        ),
        "{:?}",
        register.conflicts(&analysis)
    );
    Ok(())
}

#[test]
fn a_clock_set_back_is_a_conflict_and_no_gap() -> TestResult {
    let mut reads = minute_clock(14 * 3_600 + 500, 0, 300, 1800)?;
    reads.extend(minute_clock(14 * 3_600, 300, 600, 1800)?);
    let analysis = analyse(reads);
    let register = ClockRegister::of(&[], &[], &analysis);
    assert_eq!(register.gaps(), []);
    assert!(matches!(
        register.conflicts(&analysis).as_slice(),
        [ClockConflict::Backward { .. }]
    ));
    Ok(())
}

#[test]
fn an_observation_is_an_inference_and_names_the_offset_it_fits() -> TestResult {
    let analysis = analyse(minute_clock(9 * 3_600 + 12, 0, 300, 1800)?);
    let register = ClockRegister::of(&[], &[], &analysis);
    let anchors = stated("2026-10-21T14:00:12Z")?;
    let [observation] = register
        .observations(&analysis, &anchors)
        .try_into()
        .map_err(|_| "no observation")?;
    assert_eq!(observation.anchor, AnchorSource::Operator);
    assert_eq!(observation.fits, vec![UtcOffset::from_seconds(-18_000)?]);
    // Only a person applies it: the anchors are untouched.
    assert_eq!(register.screen_anchors(&analysis, &anchors), Vec::new());
    Ok(())
}

#[test]
fn a_time_of_day_can_fit_two_offsets_and_a_stray_difference_none() -> TestResult {
    let two = analyse(minute_clock(10 * 3_600, 0, 300, 1800)?);
    let register = ClockRegister::of(&[], &[], &two);
    let anchors = stated("2026-10-21T00:00:00Z")?;
    let [observation] = register
        .observations(&two, &anchors)
        .try_into()
        .map_err(|_| "no observation")?;
    assert_eq!(
        observation.fits,
        vec![
            UtcOffset::from_seconds(-50_400)?,
            UtcOffset::from_seconds(36_000)?
        ]
    );
    let stray = analyse(minute_clock(10 * 3_600 + 420, 0, 300, 1800)?);
    let register = ClockRegister::of(&[], &[], &stray);
    let [observation] = register
        .observations(&stray, &anchors)
        .try_into()
        .map_err(|_| "no observation")?;
    assert_eq!(observation.fits, Vec::new());
    Ok(())
}

#[test]
fn no_anchor_that_names_an_instant_means_no_observation() -> TestResult {
    let analysis = analyse(minute_clock(14 * 3_600, 0, 300, 1800)?);
    let register = ClockRegister::of(&[], &[], &analysis);
    assert_eq!(
        register.observations(&analysis, &stated("2026-10-21T14:00:00")?),
        Vec::new()
    );
    Ok(())
}

#[test]
fn a_confirmed_clock_of_no_date_takes_the_nearest_date_and_gives_an_instant() -> TestResult {
    // Just before midnight in New York, already the next day in UTC.
    let analysis = analyse(minute_clock(23 * 3_600 + 50 * 60 + 12, 0, 300, 1800)?);
    let mut register = ClockRegister::of(&[], &[], &analysis);
    let anchors = stated("2026-10-22T03:50:12Z")?;
    let id = ClockId::new(1).ok_or("no clock")?;
    register.confirm_zone(id, UtcOffset::from_seconds(-14_400)?, &analysis, &anchors)?;
    let [anchor] = register
        .screen_anchors(&analysis, &anchors)
        .try_into()
        .map_err(|_| "no anchor")?;
    assert_eq!(anchor.source(), AnchorSource::ScreenClockConfirmed);
    let reading = anchor.reading();
    assert_eq!(reading.offset(), Some(UtcOffset::from_seconds(-14_400)?));
    let local = reading.local().ok_or("no local")?;
    assert_eq!((local.month(), local.day(), local.hour()), (10, 21, 23));
    let instant = reading.utc_unix_micros().ok_or("no instant")?;
    let expected = ClockReading::parse("2026-10-22T03:50:12Z")?
        .utc_unix_micros()
        .ok_or("no instant")?;
    assert!(
        (instant - expected).abs() <= 11_000_000,
        "{}",
        instant - expected
    );
    Ok(())
}

#[test]
fn a_clock_of_no_date_with_no_absolute_anchor_cannot_be_confirmed() -> TestResult {
    let analysis = analyse(minute_clock(14 * 3_600, 0, 300, 1800)?);
    let mut register = ClockRegister::of(&[], &[], &analysis);
    let id = ClockId::new(1).ok_or("no clock")?;
    assert_eq!(
        register.confirm_zone(id, UtcOffset::ZERO, &analysis, &TimeAnchors::none()),
        Err(ClockStatementError::NoDate)
    );
    assert_eq!(
        register.confirm_zone(
            ClockId::new(5).ok_or("no clock")?,
            UtcOffset::ZERO,
            &analysis,
            &stated("2026-10-21T14:00:00Z")?
        ),
        Err(ClockStatementError::UnknownClock)
    );
    Ok(())
}

const PLACE: u32 = 1800;
const A: u64 = 14 * 3_600;

/// A, then B, then A: B's turn is a clock of its own, named and observed, and A
/// carries on with no jump (ADR 0029 decision 13, the planner's review).
fn assert_two_clocks(analysis: &ClockAnalysis, stated_at: &str) -> TestResult {
    assert_eq!(analysis.clocks.len(), 2, "{:?}", analysis.clocks);
    let register = ClockRegister::of(&[], &[], analysis);
    assert!(register.gaps().is_empty(), "{:?}", register.gaps());
    for (index, clock) in analysis.clocks.iter().enumerate() {
        assert_eq!(clock.jumps(), [], "clock {index} jumped");
        assert!(register.id_of(index).is_some(), "clock {index} has no id");
    }
    let observations = register.observations(analysis, &stated(stated_at)?);
    assert_eq!(observations.len(), 2, "{observations:?}");
    Ok(())
}

#[test]
fn a_presenter_who_hands_back_on_whole_hours_keeps_their_clock() -> TestResult {
    let mut reads = minute_clock(A, 0, 300, PLACE)?;
    reads.extend(minute_clock(A + 5 * 3_600, 300, 900, PLACE)?);
    reads.extend(minute_clock(A, 900, 1200, PLACE)?);
    assert_two_clocks(&analyse(reads), "2026-10-21T14:00:00Z")
}

#[test]
fn machines_thirty_seconds_apart_taking_turns_are_two_clocks() -> TestResult {
    let mut reads = minute_clock(A, 0, 300, PLACE)?;
    reads.extend(minute_clock(A + 30, 300, 900, PLACE)?);
    reads.extend(minute_clock(A, 900, 1200, PLACE)?);
    assert_two_clocks(&analyse(reads), "2026-10-21T14:00:00Z")
}

#[test]
fn turns_back_and_forth_are_two_clocks_with_all_their_reads() -> TestResult {
    let b = A + 5 * 3_600;
    let mut reads = minute_clock(A, 0, 300, PLACE)?;
    reads.extend(minute_clock(b, 300, 600, PLACE)?);
    reads.extend(minute_clock(A, 600, 900, PLACE)?);
    reads.extend(minute_clock(b, 900, 1200, PLACE)?);
    reads.extend(minute_clock(A, 1200, 1500, PLACE)?);
    let analysis = analyse(reads);
    assert_two_clocks(&analysis, "2026-10-21T14:00:00Z")?;
    let total: usize = analysis.clocks.iter().map(ScreenClock::read_count).sum();
    assert_eq!(total, 150, "no read is lost");
    Ok(())
}

#[test]
fn a_held_misread_inside_a_turn_taker_stays_an_excursion() -> TestResult {
    let mut reads = minute_clock(A, 0, 240, PLACE)?;
    let frame = FrameDimensions::new(1920, 1080)?;
    let region = TextRegion::new(PLACE, 1040, 60, 14, frame)?;
    for at in [240_u64, 250, 260, 270, 280, 290] {
        reads.push(ClockRead {
            time: MediaTime::from_micros(at * SECOND),
            pts: i64::try_from(at)?,
            region,
            face: read_clock_face("14:09")?,
        });
    }
    reads.extend(minute_clock(A, 300, 600, PLACE)?);
    reads.extend(minute_clock(A + 5 * 3_600, 600, 900, PLACE)?);
    reads.extend(minute_clock(A, 900, 1200, PLACE)?);
    let analysis = analyse(reads);
    assert_two_clocks(&analysis, "2026-10-21T14:00:00Z")?;
    let excursions: usize = analysis
        .clocks
        .iter()
        .map(|clock| clock.excursions().len())
        .sum();
    assert_eq!(excursions, 1, "the misread is not a third clock");
    Ok(())
}

#[test]
fn the_search_never_brackets_across_another_clocks_turn() -> TestResult {
    // A's rollover from 14:04 to 14:05 falls inside B's turn, so the two reads
    // either side of the turn are one minute apart.
    let mut reads = minute_clock(A + 45, 0, 250, PLACE)?;
    reads.extend(minute_clock(A + 45 + 5 * 3_600, 250, 280, PLACE)?);
    reads.extend(minute_clock(A + 45, 280, 500, PLACE)?);
    let analysis = analyse(reads);
    assert_eq!(analysis.clocks.len(), 2);
    let before = MediaTime::from_micros(240 * SECOND);
    let after = MediaTime::from_micros(280 * SECOND);
    let interrupted = analysis
        .clocks
        .iter()
        .find(|clock| clock.first_time() == Some(MediaTime::from_micros(0)))
        .ok_or("no interrupted clock")?;
    assert_eq!(interrupted.stopped_brackets(), vec![(before, after)]);
    assert!(
        !analysis
            .next_reads()
            .contains(&MediaTime::from_micros(260 * SECOND)),
        "the interrupted clock's midpoint would read the other clock"
    );
    Ok(())
}

#[test]
fn a_turn_that_never_hands_back_is_still_a_detected_gap() -> TestResult {
    let mut reads = minute_clock(A, 0, 300, PLACE)?;
    reads.extend(minute_clock(A + 437, 300, 600, PLACE)?);
    let analysis = analyse(reads);
    assert_eq!(analysis.clocks.len(), 1);
    assert_eq!(ClockRegister::of(&[], &[], &analysis).gaps().len(), 1);
    Ok(())
}
