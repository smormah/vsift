//! The grammar of ADR 0029 decision 10.

use std::error::Error;

use super::*;

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn time_of_day_faces_name_their_class_unit_and_second() -> TestResult {
    for (line, format, unit, second) in [
        (
            "14:03",
            ClockFormat::TwentyFourHour,
            ClockUnit::Minute,
            50_580,
        ),
        (
            "14:03:12",
            ClockFormat::TwentyFourHour,
            ClockUnit::Second,
            50_592,
        ),
        ("00:00", ClockFormat::TwentyFourHour, ClockUnit::Minute, 0),
        (
            "23:59:59",
            ClockFormat::TwentyFourHour,
            ClockUnit::Second,
            86_399,
        ),
        (
            "2:03 PM",
            ClockFormat::TwelveHour,
            ClockUnit::Minute,
            50_580,
        ),
        ("2:03PM", ClockFormat::TwelveHour, ClockUnit::Minute, 50_580),
        (
            "2:03:12 pm",
            ClockFormat::TwelveHour,
            ClockUnit::Second,
            50_592,
        ),
        (
            "2:03 p.m.",
            ClockFormat::TwelveHour,
            ClockUnit::Minute,
            50_580,
        ),
        ("12:03 AM", ClockFormat::TwelveHour, ClockUnit::Minute, 180),
        (
            "12:03 PM",
            ClockFormat::TwelveHour,
            ClockUnit::Minute,
            43_380,
        ),
    ] {
        let face = read_clock_face(line)?;
        assert_eq!(face.format(), format, "{line}");
        assert_eq!(face.unit(), unit, "{line}");
        assert_eq!(face.second_of_day(), second, "{line}");
        assert_eq!(face.reading(), None, "{line}");
        assert!(face.wraps(), "{line}");
    }
    Ok(())
}

#[test]
fn a_time_inside_a_longer_line_is_read() -> TestResult {
    assert_eq!(read_clock_face("Tue 2:03 PM")?.second_of_day(), 50_580);
    assert_eq!(
        read_clock_face("Recording 14:03:12 live")?.second_of_day(),
        50_592
    );
    assert_eq!(read_clock_face("Página 14:03")?.second_of_day(), 50_580);
    Ok(())
}

#[test]
fn dated_faces_carry_their_reading() -> TestResult {
    let expected = ClockReading::parse("2026-10-21T14:03:12")?;
    for line in [
        "2026-10-21 14:03:12",
        "2026-10-21T14:03:12",
        "21 Oct 2026 14:03:12",
        "Oct 21, 2026 14:03:12",
        "October 21, 2026 2:03:12 PM",
        "2026-10-21 14:03:12.250",
        "2026-10-21 14:03:12 IST",
    ] {
        let face = read_clock_face(line)?;
        assert_eq!(face.format(), ClockFormat::Dated, "{line}");
        assert_eq!(face.reading(), Some(expected), "{line}");
        assert!(!face.wraps(), "{line}");
    }
    Ok(())
}

#[test]
fn only_a_stated_zone_makes_an_instant() -> TestResult {
    let instant = ClockReading::parse("2026-10-21T14:03:12Z")?;
    for line in [
        "2026-10-21T14:03:12Z",
        "2026-10-21 14:03:12 UTC",
        "2026-10-21 14:03:12 GMT",
        "2026-10-21T14:03:12.250Z",
        "2026-10-21 14:03:12 -00:00",
    ] {
        let face = read_clock_face(line)?;
        assert_eq!(face.format(), ClockFormat::DatedWithZone, "{line}");
        assert_eq!(face.reading(), Some(instant), "{line}");
    }
    let zoned = read_clock_face("2026-10-21 14:03:12 -0500")?;
    assert_eq!(
        zoned.reading(),
        Some(ClockReading::parse("2026-10-21T14:03:12-05:00")?)
    );
    assert_eq!(
        read_clock_face("2026-10-21 14:03+01:00")?.reading(),
        Some(ClockReading::parse("2026-10-21T14:03:00+01:00")?)
    );
    Ok(())
}

#[test]
fn a_zone_on_a_time_of_day_is_not_read() {
    assert_eq!(read_clock_face("14:03Z"), Err(ClockFaceRefusal::NoTime));
}

#[test]
fn what_a_screen_shows_for_something_else_is_refused() {
    for line in [
        "16:9",
        "1:23",
        "10:30:00.0",
        "1.2.3:45",
        "25:10",
        "14:75",
        "14:03:61",
        "13:03 PM",
        "0:03 AM",
        "v2:03",
        "a14:03",
        "14:03a",
        "14:03/2",
        "10/21/2026",
        "see 2:3 PM",
        "12:34:56:78",
        "10/21/2026 14:03x",
        "14-03",
        "1403",
        "14.03",
        "",
    ] {
        assert_eq!(
            read_clock_face(line),
            Err(ClockFaceRefusal::NoTime),
            "{line}"
        );
    }
}

#[test]
fn two_times_in_a_line_are_refused_even_when_one_is_not_valid() {
    for line in [
        "9:00 - 10:00",
        "1:23 / 4:56",
        "14:03 to 15:04",
        "25:10 14:03",
    ] {
        assert_eq!(
            read_clock_face(line),
            Err(ClockFaceRefusal::SeveralTimes),
            "{line}"
        );
    }
}

#[test]
fn a_long_line_is_refused_unread() {
    let line = format!("{} 14:03", "x".repeat(MAX_CLOCK_LINE_BYTES));
    assert_eq!(read_clock_face(&line), Err(ClockFaceRefusal::TooLong));
}

#[test]
fn a_date_that_cannot_be_is_no_face() {
    for line in ["2026-13-21 14:03", "2026-02-30 14:03", "32 Oct 2026 14:03"] {
        assert_eq!(
            read_clock_face(line),
            Err(ClockFaceRefusal::NoTime),
            "{line}"
        );
    }
}

#[test]
fn a_seconds_face_of_a_day_can_be_a_counter_and_no_other_can() -> TestResult {
    assert!(read_clock_face("01:03:22")?.could_be_a_counter());
    assert!(!read_clock_face("01:03")?.could_be_a_counter());
    assert!(!read_clock_face("1:03:22 PM")?.could_be_a_counter());
    assert!(!read_clock_face("2026-10-21 01:03:22")?.could_be_a_counter());
    Ok(())
}

#[test]
fn values_read_as_one_number() -> TestResult {
    assert_eq!(read_clock_face("00:01:01")?.value_micros(), 61_000_000);
    assert_eq!(
        read_clock_face("2026-10-21T13:03:12Z")?.value_micros(),
        1_792_587_792_000_000
    );
    // A zoned face's value is its own local time: an offset is not a clock's business.
    assert_eq!(
        read_clock_face("2026-10-21 14:03:12 +01:00")?.value_micros(),
        read_clock_face("2026-10-21 14:03:12")?.value_micros()
    );
    Ok(())
}
