//! Clock faces in on-screen text (time anchoring stage 2, ADR 0029 decision 10).
//!
//! This is step 1 of recognition: one line of OCR output, untrusted, read by a
//! closed grammar. A face is only a candidate. A time on a screen is usually
//! not the present (a chat message, a calendar, a line of a log), so only how a
//! face moves against the video can make it a clock ([`crate::analyse_clocks`]).
//!
//! Nothing is repaired: `O` is not read as `0`. A line a person would call a
//! clock but the grammar does not accept is simply not seen.

use std::{error::Error, fmt};

use crate::{CivilDateTime, ClockReading, UtcOffset};

/// Longest line the grammar reads; anything longer is refused unread.
pub const MAX_CLOCK_LINE_BYTES: usize = 64;
/// Lowest engine score a line may have, in basis points. This only trims junk
/// and is no defence against a misread: the engine's score is uncalibrated, and
/// on compressed clocks wrong readings scored as high as right ones (ADR 0029
/// decisions 10 and 22).
pub const MIN_CLOCK_LINE_BASIS_POINTS: u16 = 8_000;

const SECONDS_PER_MINUTE: u32 = 60;
const SECONDS_PER_HOUR: u32 = 3_600;
const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
/// The longest month name.
const MAX_MONTH_WORD_BYTES: usize = 9;

/// What a face shows, which decides how far one reading narrows the time.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ClockUnit {
    /// The face shows minutes: a reading holds for 60 s.
    Minute,
    /// The face shows seconds: a reading holds for 1 s.
    Second,
}

impl ClockUnit {
    /// How long one reading of this face holds, in microseconds.
    #[must_use]
    pub const fn as_micros(self) -> i64 {
        match self {
            Self::Minute => 60_000_000,
            Self::Second => 1_000_000,
        }
    }
}

/// The class of a face. Faces of one class in one place can form a clock.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ClockFormat {
    /// `14:03` or `14:03:12`.
    TwentyFourHour,
    /// `2:03 PM` or `2:03:12 pm`.
    TwelveHour,
    /// A date and a time with no zone: `2026-10-21 14:03`, `21 Oct 2026 14:03`.
    Dated,
    /// A date and a time that states its zone: `2026-10-21T14:03:12Z`.
    DatedWithZone,
}

impl ClockFormat {
    /// Every class.
    pub const ALL: [Self; 4] = [
        Self::TwentyFourHour,
        Self::TwelveHour,
        Self::Dated,
        Self::DatedWithZone,
    ];

    /// Stable public identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::TwentyFourHour => "24_hour",
            Self::TwelveHour => "12_hour",
            Self::Dated => "dated",
            Self::DatedWithZone => "dated_with_zone",
        }
    }
}

/// Why a line is not a clock face.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ClockFaceRefusal {
    /// The line is longer than [`MAX_CLOCK_LINE_BYTES`].
    TooLong,
    /// The line holds no time the grammar accepts.
    NoTime,
    /// The line holds two or more times (`9:00 - 10:00`, `1:23 / 4:56`).
    SeveralTimes,
}

impl fmt::Display for ClockFaceRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooLong => "the line is too long to be a clock",
            Self::NoTime => "the line holds no time",
            Self::SeveralTimes => "the line holds several times",
        })
    }
}

impl Error for ClockFaceRefusal {}

/// One clock face read off one line.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ClockFace {
    format: ClockFormat,
    unit: ClockUnit,
    second_of_day: u32,
    reading: Option<ClockReading>,
}

impl ClockFace {
    /// The face's class.
    #[must_use]
    pub const fn format(self) -> ClockFormat {
        self.format
    }

    /// What the face shows.
    #[must_use]
    pub const fn unit(self) -> ClockUnit {
        self.unit
    }

    /// The time of day it shows, in seconds past midnight.
    #[must_use]
    pub const fn second_of_day(self) -> u32 {
        self.second_of_day
    }

    /// The date and time, with as much as the face says of its zone; `None`
    /// for a time of day, which cannot be written as a date and time.
    #[must_use]
    pub const fn reading(self) -> Option<ClockReading> {
        self.reading
    }

    /// What the face shows as one number, in microseconds: past the epoch for
    /// a dated face, past midnight for a time of day. Time-of-day values wrap
    /// at 24 hours, which the clock model undoes.
    #[must_use]
    pub fn value_micros(self) -> i64 {
        match self.reading {
            Some(ClockReading::Utc { unix_micros }) => unix_micros,
            Some(ClockReading::Zoned { local, .. } | ClockReading::LocalUnknownZone { local }) => {
                local.as_unix_micros_if_utc()
            }
            None => i64::from(self.second_of_day) * 1_000_000,
        }
    }

    /// Whether the face wraps at 24 hours: it has no date.
    #[must_use]
    pub const fn wraps(self) -> bool {
        self.reading.is_none()
    }

    /// Whether a moving face of this kind can be an elapsed timer: seconds, no
    /// date and no AM or PM (ADR 0029 decision 10).
    #[must_use]
    pub const fn could_be_a_counter(self) -> bool {
        matches!(self.format, ClockFormat::TwentyFourHour) && matches!(self.unit, ClockUnit::Second)
    }
}

/// One time-shaped token of a line.
struct Token {
    /// Where the token starts.
    start: usize,
    /// Just past its digits.
    end: usize,
    hour_digits: usize,
    hour: u32,
    minute: u32,
    second: Option<u32>,
}

/// The stated zone of a dated face.
enum StatedZone {
    Utc,
    Offset(UtcOffset),
}

/// Reads the clock face of one line of on-screen text.
///
/// # Errors
///
/// [`ClockFaceRefusal::TooLong`], [`ClockFaceRefusal::SeveralTimes`] or
/// [`ClockFaceRefusal::NoTime`].
pub fn read_clock_face(line: &str) -> Result<ClockFace, ClockFaceRefusal> {
    if line.len() > MAX_CLOCK_LINE_BYTES {
        return Err(ClockFaceRefusal::TooLong);
    }
    let tokens = time_shaped_tokens(line);
    let [token] = tokens.as_slice() else {
        return Err(if tokens.is_empty() {
            ClockFaceRefusal::NoTime
        } else {
            ClockFaceRefusal::SeveralTimes
        });
    };
    face_of(line, token).ok_or(ClockFaceRefusal::NoTime)
}

/// Every `H:MM` or `H:MM:SS` of the line that stands on its own at its left
/// edge, valid or not: `25:10` beside `14:03` still makes two times.
fn time_shaped_tokens(line: &str) -> Vec<Token> {
    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at].is_ascii_digit()
            && stands_alone_at_left(line, at)
            && let Some(token) = token_at(bytes, at)
        {
            at = token.end;
            tokens.push(token);
        } else {
            at += 1;
        }
    }
    tokens
}

fn stands_alone_at_left(line: &str, at: usize) -> bool {
    let bytes = line.as_bytes();
    let Some(&before) = at.checked_sub(1).and_then(|index| bytes.get(index)) else {
        return true;
    };
    if matches!(before, b'T' | b't') {
        return iso_date_ending_at(&line[..at - 1]).is_some();
    }
    !(before.is_ascii_alphanumeric()
        || !before.is_ascii()
        || matches!(before, b':' | b'.' | b'/' | b'-' | b'+'))
}

fn token_at(bytes: &[u8], start: usize) -> Option<Token> {
    let hour_digits = bytes[start..]
        .iter()
        .take(3)
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if !(1..=2).contains(&hour_digits) {
        return None;
    }
    let hour_end = start + hour_digits;
    let minute = two_digits_after_colon(bytes, hour_end)?;
    let minute_end = hour_end + 3;
    let second = two_digits_after_colon(bytes, minute_end);
    let end = minute_end + if second.is_some() { 3 } else { 0 };
    Some(Token {
        start,
        end,
        hour_digits,
        hour: number(&bytes[start..hour_end])?,
        minute,
        second,
    })
}

fn two_digits_after_colon(bytes: &[u8], at: usize) -> Option<u32> {
    match bytes.get(at..at + 3)? {
        [b':', tens, ones] if tens.is_ascii_digit() && ones.is_ascii_digit() => {
            number(&[*tens, *ones])
        }
        _ => None,
    }
}

fn number(bytes: &[u8]) -> Option<u32> {
    bytes.iter().try_fold(0_u32, |value, byte| {
        byte.is_ascii_digit()
            .then(|| value * 10 + u32::from(byte - b'0'))
    })
}

fn face_of(line: &str, token: &Token) -> Option<ClockFace> {
    let bytes = line.as_bytes();
    let date = date_before(&line[..token.start]);
    let mut end = token.end;
    if date.is_some() {
        end = past_fraction(bytes, end);
    }
    let (after_meridiem, afternoon) = match meridiem_at(bytes, end) {
        Some((next, afternoon)) => (next, Some(afternoon)),
        None => (end, None),
    };
    end = after_meridiem;
    let zone = match date {
        Some(_) => zone_at(line, end),
        None => None,
    };
    if let Some((next, _)) = &zone {
        end = *next;
    }
    if !stands_alone_at_right(bytes, end) {
        return None;
    }
    let hour = hour_of_day(token, afternoon)?;
    let second = token.second.unwrap_or(0);
    if token.minute > 59 || second > 59 {
        return None;
    }
    let unit = if token.second.is_some() {
        ClockUnit::Second
    } else {
        ClockUnit::Minute
    };
    let reading = match date {
        Some((year, month, day)) => {
            let local = CivilDateTime::new(
                year,
                month,
                day,
                u8::try_from(hour).ok()?,
                u8::try_from(token.minute).ok()?,
                u8::try_from(second).ok()?,
                0,
            )
            .ok()?;
            Some(match &zone {
                None => ClockReading::LocalUnknownZone { local },
                Some((_, StatedZone::Utc)) => {
                    ClockReading::utc(local.as_unix_micros_if_utc()).ok()?
                }
                Some((_, StatedZone::Offset(offset))) => {
                    ClockReading::zoned(local, *offset).ok()?
                }
            })
        }
        None => None,
    };
    let format = match (&zone, date, afternoon) {
        (Some(_), _, _) => ClockFormat::DatedWithZone,
        (None, Some(_), _) => ClockFormat::Dated,
        (None, None, Some(_)) => ClockFormat::TwelveHour,
        (None, None, None) => ClockFormat::TwentyFourHour,
    };
    Some(ClockFace {
        format,
        unit,
        second_of_day: hour * SECONDS_PER_HOUR + token.minute * SECONDS_PER_MINUTE + second,
        reading,
    })
}

/// The hour of the day. A one-digit hour needs AM or PM: `1:23` is a duration
/// far more often than a time. A dated face is plainly a time and may have one.
fn hour_of_day(token: &Token, afternoon: Option<bool>) -> Option<u32> {
    match afternoon {
        Some(afternoon) => (1..=12).contains(&token.hour).then(|| {
            let hour = token.hour % 12;
            if afternoon { hour + 12 } else { hour }
        }),
        None => (token.hour_digits == 2 && token.hour <= 23).then_some(token.hour),
    }
}

/// Past `.fff` at `at`: a dated face may show a fraction, which is ignored.
fn past_fraction(bytes: &[u8], at: usize) -> usize {
    if bytes.get(at) != Some(&b'.') {
        return at;
    }
    let digits = bytes[at + 1..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits == 0 { at } else { at + 1 + digits }
}

/// `AM`, `pm` or `p.m.` at `at`, after at most one space: where it ends, and
/// whether it is the afternoon.
fn meridiem_at(bytes: &[u8], at: usize) -> Option<(usize, bool)> {
    let start = if bytes.get(at) == Some(&b' ') {
        at + 1
    } else {
        at
    };
    let afternoon = match bytes.get(start)? {
        b'A' | b'a' => false,
        b'P' | b'p' => true,
        _ => return None,
    };
    let mut next = start + 1;
    if bytes.get(next) == Some(&b'.') {
        next += 1;
    }
    if bytes.get(next) == Some(&b' ') {
        next += 1;
    }
    if !matches!(bytes.get(next), Some(b'M' | b'm')) {
        return None;
    }
    next += 1;
    if bytes.get(next) == Some(&b'.') {
        next += 1;
    }
    Some((next, afternoon))
}

/// A stated zone at `at`, after at most one space. An abbreviation such as
/// `IST` or `CST` names several zones, so it states none.
fn zone_at(line: &str, at: usize) -> Option<(usize, StatedZone)> {
    let bytes = line.as_bytes();
    let start = if bytes.get(at) == Some(&b' ') {
        at + 1
    } else {
        at
    };
    let rest = &line[start..];
    for (name, length) in [("Z", 1), ("z", 1), ("UTC", 3), ("GMT", 3)] {
        if rest.starts_with(name) {
            return Some((start + length, StatedZone::Utc));
        }
    }
    let sign = match bytes.get(start)? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let (hours, minutes, length) = match bytes.get(start + 1..)? {
        [h1, h2, b':', m1, m2, ..] => (number(&[*h1, *h2])?, number(&[*m1, *m2])?, 6),
        [h1, h2, m1, m2, ..] => (number(&[*h1, *h2])?, number(&[*m1, *m2])?, 5),
        _ => return None,
    };
    let seconds = i32::try_from(hours * SECONDS_PER_HOUR + minutes * SECONDS_PER_MINUTE).ok()?;
    if minutes > 59 {
        return None;
    }
    if sign < 0 && seconds == 0 {
        // RFC 3339 4.3, as `ClockReading::parse` reads it.
        return Some((start + length, StatedZone::Utc));
    }
    let offset = UtcOffset::from_seconds(sign * seconds).ok()?;
    Some((start + length, StatedZone::Offset(offset)))
}

fn stands_alone_at_right(bytes: &[u8], at: usize) -> bool {
    match bytes.get(at) {
        None => true,
        Some(b'.') => !bytes.get(at + 1).is_some_and(u8::is_ascii_digit),
        Some(&after) => {
            !(after.is_ascii_alphanumeric()
                || !after.is_ascii()
                || matches!(after, b':' | b'/' | b'-'))
        }
    }
}

/// The date that ends just before a time: ISO, or with a month name.
fn date_before(prefix: &str) -> Option<(u16, u8, u8)> {
    let trimmed = prefix.strip_suffix([' ', 'T', 't'])?;
    iso_date_ending_at(trimmed).or_else(|| month_date_ending_at(trimmed))
}

/// `YYYY-MM-DD` at the end of `text`, standing alone.
fn iso_date_ending_at(text: &str) -> Option<(u16, u8, u8)> {
    let bytes = text.as_bytes();
    let start = bytes.len().checked_sub(10)?;
    let date = &bytes[start..];
    if date[4] != b'-' || date[7] != b'-' {
        return None;
    }
    if start > 0 && bytes[start - 1].is_ascii_alphanumeric() {
        return None;
    }
    Some((
        u16::try_from(number(&date[..4])?).ok()?,
        u8::try_from(number(&date[5..7])?).ok()?,
        u8::try_from(number(&date[8..])?).ok()?,
    ))
}

/// `21 Oct 2026` or `Oct 21, 2026` at the end of `text`; a comma may follow.
fn month_date_ending_at(text: &str) -> Option<(u16, u8, u8)> {
    let mut words = text.trim_end_matches(',').rsplit(' ');
    let year = words.next()?;
    if year.len() != 4 {
        return None;
    }
    let year = u16::try_from(number(year.as_bytes())?).ok()?;
    let second = words.next()?;
    let first = words.next()?;
    if let Some(month) = month_named(second) {
        return Some((year, month, day_number(first)?));
    }
    let day = second.strip_suffix(',').unwrap_or(second);
    Some((year, month_named(first)?, day_number(day)?))
}

fn day_number(word: &str) -> Option<u8> {
    if !(1..=2).contains(&word.len()) {
        return None;
    }
    u8::try_from(number(word.as_bytes())?).ok()
}

fn month_named(word: &str) -> Option<u8> {
    if word.len() < 3
        || word.len() > MAX_MONTH_WORD_BYTES
        || !word.bytes().all(|byte| byte.is_ascii_alphabetic())
    {
        return None;
    }
    let start = word[..3].to_ascii_lowercase();
    let index = MONTHS.iter().position(|month| *month == start)?;
    u8::try_from(index + 1).ok()
}

#[cfg(test)]
mod tests;
