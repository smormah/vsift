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
    let register = ClockRegister::of(&[], &analysis);
    assert_eq!(register.id_of(0), ClockId::new(1));
    assert_eq!(register.id_of(1), ClockId::new(2));
    assert_eq!(register.entries().len(), 2);
    Ok(())
}

#[test]
fn an_id_survives_when_a_later_read_finds_an_earlier_clock() -> TestResult {
    // The second call has read earlier frames, so the other clock now comes first.
    let later = minute_clock(19 * 3_600, 200, 500, 200)?;
    let first = ClockRegister::of(&[], &analyse(later.clone()));
    let mut reads = later;
    reads.extend(minute_clock(14 * 3_600, 0, 500, 1800)?);
    let analysis = analyse(reads);
    let second = ClockRegister::of(first.entries(), &analysis);
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
    let register = ClockRegister::of(&[], &analysis);
    assert_eq!(register.entries().len(), MAX_SCREEN_CLOCKS);
    assert_eq!(register.id_of(8), None);
    assert_eq!(register.id_of(7), ClockId::new(8));
    Ok(())
}

#[test]
fn a_confirmation_stays_with_its_clock() -> TestResult {
    let analysis = analyse(minute_clock(14 * 3_600, 0, 300, 1800)?);
    let mut entries = ClockRegister::of(&[], &analysis).entries().to_vec();
    let offset = UtcOffset::from_seconds(-18_000)?;
    entries[0].confirmed_offset = Some(offset);
    let again = ClockRegister::of(&entries, &analysis);
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
        ClockRegister::of(&[], &analysis).screen_anchors(&analysis),
        Vec::new()
    );
    Ok(())
}
