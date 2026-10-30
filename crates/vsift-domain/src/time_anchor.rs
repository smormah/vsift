//! Time anchoring: the real-world clock time of positions on the source timeline.
//!
//! A recording's evidence is located by its time into the video. To match a
//! remark with a server log, a reader also needs the clock time it was said at.
//! A [`TimeAnchor`] ties one position of the source timeline to one clock
//! reading, from one named source with a fixed confidence; [`TimeAnchors`]
//! holds a session's anchors in ladder order and derives the clock time of any
//! position from the first of them (ADR 0029).
//!
//! Stage 1 (0.3.0) knows three sources, all anchored at the start of the
//! timeline: a time the operator states, the recording app's local-time tag
//! and the container's UTC creation time. A file's modification time is
//! deliberately not a source: for most files a person is sent it is the time
//! of a copy, a download or an unpacking, and nothing in the file says which.
//! Stage 2 adds sources (clocks visible on screen) at other positions; nothing
//! here assumes a single anchor or position zero.
//!
//! Zones are kept honest. A reading is an exact UTC instant with its local time
//! when the zone is known ([`ClockReading::Zoned`]), an instant whose local zone
//! is not known ([`ClockReading::Utc`]), or a local time whose zone is not known
//! ([`ClockReading::LocalUnknownZone`]), which names no instant at all.
//!
//! Calendar arithmetic is the proleptic Gregorian calendar of RFC 3339 with no
//! leap seconds, implemented here so the domain needs no date library.

use std::{error::Error, fmt};

use crate::MediaTime;

/// Most anchors one session holds.
pub const MAX_TIME_ANCHORS: usize = 16;
/// Most clocks on the screen one session keeps track of.
pub const MAX_SCREEN_CLOCKS: usize = 8;
/// Longest clock text the parser reads; anything longer is refused unread.
pub const MAX_CLOCK_TEXT_BYTES: usize = 40;
/// How far apart two comparable anchors may be and still agree, in
/// microseconds. Container tags carry whole seconds and a recorder writes its
/// tags a moment apart, so two seconds separates rounding from disagreement.
pub const ANCHOR_AGREEMENT_MICROS: i64 = 2_000_000;
/// The earliest clock time a container tag may claim (1995-01-01T00:00:00Z):
/// earlier values are the zero dates and epoch defaults of tools that wrote no
/// real time, not recordings.
pub const EARLIEST_TAGGED_RECORDING_UNIX_SECONDS: i64 = 788_918_400;
/// How far past the ingesting host's clock a container tag may claim: a day,
/// for a recorder whose clock runs ahead.
pub const TAG_FUTURE_ALLOWANCE_SECONDS: i64 = 86_400;

const MICROS_PER_SECOND: i64 = 1_000_000;
const SECONDS_PER_DAY: i64 = 86_400;
const MAX_OFFSET_SECONDS: i32 = 14 * 3_600;
/// 0001-01-01T00:00:00Z, the first instant a reading may name.
const MIN_UNIX_MICROS: i64 = -62_135_596_800 * MICROS_PER_SECOND;
/// 9999-12-31T23:59:59.999999Z, the last instant a reading may name.
const MAX_UNIX_MICROS: i64 = 253_402_300_800 * MICROS_PER_SECOND - 1;

/// Why a clock time could not be built or read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeAnchorError {
    /// The text is longer than [`MAX_CLOCK_TEXT_BYTES`] or not ASCII.
    TooLong,
    /// The text is not `YYYY-MM-DDTHH:MM:SS[.fraction][Z|±HH:MM|±HHMM]`.
    Malformed,
    /// A date or time field is out of range (a 13th month, a 30 February, a
    /// leap second, a year outside 1 to 9999).
    OutOfRange,
    /// A UTC offset beyond ±14:00 or not a whole minute.
    InvalidOffset,
    /// More anchors than [`MAX_TIME_ANCHORS`], or two from one stage-1 source.
    TooManyAnchors,
}

impl fmt::Display for TimeAnchorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooLong => "the clock time is too long or not ASCII",
            Self::Malformed => {
                "the clock time is not of the form YYYY-MM-DDTHH:MM:SS[.fraction][Z|+HH:MM|-HH:MM]"
            }
            Self::OutOfRange => "a field of the clock time is out of range",
            Self::InvalidOffset => {
                "the UTC offset is beyond 14 hours or not a whole number of minutes"
            }
            Self::TooManyAnchors => "too many time anchors, or two from one source",
        })
    }
}

impl Error for TimeAnchorError {}

/// A calendar date and time of day with no zone: what a clock on the wall of
/// the recording's place showed.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CivilDateTime {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    micros: u32,
}

impl CivilDateTime {
    /// Builds a validated date and time.
    ///
    /// # Errors
    ///
    /// [`TimeAnchorError::OutOfRange`] for a year outside 1 to 9999, a day the
    /// month does not have, an hour past 23, a minute or second past 59 (no
    /// leap second) or a fraction of a million microseconds or more.
    pub fn new(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        micros: u32,
    ) -> Result<Self, TimeAnchorError> {
        if !(1..=9999).contains(&year)
            || !(1..=12).contains(&month)
            || day == 0
            || day > days_in_month(year, month)
            || hour > 23
            || minute > 59
            || second > 59
            || micros >= 1_000_000
        {
            return Err(TimeAnchorError::OutOfRange);
        }
        Ok(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            micros,
        })
    }

    /// The date and time this many microseconds after the Unix epoch, read on
    /// a clock that shows UTC; `None` outside years 1 to 9999.
    #[must_use]
    pub fn from_unix_micros(unix_micros: i64) -> Option<Self> {
        if !(MIN_UNIX_MICROS..=MAX_UNIX_MICROS).contains(&unix_micros) {
            return None;
        }
        let seconds = unix_micros.div_euclid(MICROS_PER_SECOND);
        let micros = u32::try_from(unix_micros.rem_euclid(MICROS_PER_SECOND)).ok()?;
        let days = seconds.div_euclid(SECONDS_PER_DAY);
        let of_day = seconds.rem_euclid(SECONDS_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        Self::new(
            u16::try_from(year).ok()?,
            u8::try_from(month).ok()?,
            u8::try_from(day).ok()?,
            u8::try_from(of_day / 3_600).ok()?,
            u8::try_from(of_day % 3_600 / 60).ok()?,
            u8::try_from(of_day % 60).ok()?,
            micros,
        )
        .ok()
    }

    /// Microseconds since the Unix epoch if this date and time were read on a
    /// clock that shows UTC. Every valid value is representable.
    #[must_use]
    pub fn as_unix_micros_if_utc(self) -> i64 {
        let days = days_from_civil(
            i64::from(self.year),
            i64::from(self.month),
            i64::from(self.day),
        );
        let seconds = days * SECONDS_PER_DAY
            + i64::from(self.hour) * 3_600
            + i64::from(self.minute) * 60
            + i64::from(self.second);
        seconds * MICROS_PER_SECOND + i64::from(self.micros)
    }

    /// The date and time `delta_micros` later (earlier when negative); `None`
    /// outside years 1 to 9999.
    #[must_use]
    pub fn checked_add_micros(self, delta_micros: i64) -> Option<Self> {
        Self::from_unix_micros(self.as_unix_micros_if_utc().checked_add(delta_micros)?)
    }

    /// Calendar year, 1 to 9999.
    #[must_use]
    pub const fn year(self) -> u16 {
        self.year
    }

    /// Month, 1 to 12.
    #[must_use]
    pub const fn month(self) -> u8 {
        self.month
    }

    /// Day of the month, from 1.
    #[must_use]
    pub const fn day(self) -> u8 {
        self.day
    }

    /// Hour, 0 to 23.
    #[must_use]
    pub const fn hour(self) -> u8 {
        self.hour
    }

    /// Minute, 0 to 59.
    #[must_use]
    pub const fn minute(self) -> u8 {
        self.minute
    }

    /// Second, 0 to 59.
    #[must_use]
    pub const fn second(self) -> u8 {
        self.second
    }

    /// Microseconds past the second, 0 to 999,999.
    #[must_use]
    pub const fn micros(self) -> u32 {
        self.micros
    }
}

impl fmt::Display for CivilDateTime {
    /// The canonical text, `YYYY-MM-DDTHH:MM:SS.ffffff`: RFC 3339's date and
    /// time with exactly six fraction digits and no zone.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}",
            self.year, self.month, self.day, self.hour, self.minute, self.second, self.micros
        )
    }
}

/// The difference between a place's local time and UTC, in whole minutes
/// within ±14:00 (the range real zones use).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UtcOffset(i32);

impl UtcOffset {
    /// UTC itself.
    pub const ZERO: Self = Self(0);

    /// Builds an offset of `seconds` east of UTC (negative: west).
    ///
    /// # Errors
    ///
    /// [`TimeAnchorError::InvalidOffset`] beyond ±14:00 or for a part of a minute.
    pub fn from_seconds(seconds: i32) -> Result<Self, TimeAnchorError> {
        if seconds.abs() > MAX_OFFSET_SECONDS || seconds % 60 != 0 {
            return Err(TimeAnchorError::InvalidOffset);
        }
        Ok(Self(seconds))
    }

    /// Seconds east of UTC.
    #[must_use]
    pub const fn as_seconds(self) -> i32 {
        self.0
    }
}

impl fmt::Display for UtcOffset {
    /// RFC 3339's offset form, `+HH:MM` or `-HH:MM` (`+00:00` for UTC).
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { '-' } else { '+' };
        let minutes = self.0.unsigned_abs() / 60;
        write!(formatter, "{sign}{:02}:{:02}", minutes / 60, minutes % 60)
    }
}

/// One clock reading, with as much as its source says about the zone.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ClockReading {
    /// A local time whose UTC offset is known: an exact instant, and the time
    /// the local clock showed.
    Zoned {
        /// The local date and time.
        local: CivilDateTime,
        /// The local clock's offset from UTC.
        offset: UtcOffset,
    },
    /// An exact instant whose local zone is not known.
    Utc {
        /// Microseconds since the Unix epoch.
        unix_micros: i64,
    },
    /// A local time whose zone is not known. It names no instant: matching it
    /// with UTC logs needs the zone from elsewhere.
    LocalUnknownZone {
        /// The local date and time.
        local: CivilDateTime,
    },
}

impl ClockReading {
    /// An instant with its local zone unknown.
    ///
    /// # Errors
    ///
    /// [`TimeAnchorError::OutOfRange`] outside years 1 to 9999.
    pub fn utc(unix_micros: i64) -> Result<Self, TimeAnchorError> {
        if !(MIN_UNIX_MICROS..=MAX_UNIX_MICROS).contains(&unix_micros) {
            return Err(TimeAnchorError::OutOfRange);
        }
        Ok(Self::Utc { unix_micros })
    }

    /// A local time with its offset.
    ///
    /// # Errors
    ///
    /// [`TimeAnchorError::OutOfRange`] when the instant it names falls outside
    /// years 1 to 9999.
    pub fn zoned(local: CivilDateTime, offset: UtcOffset) -> Result<Self, TimeAnchorError> {
        let reading = Self::Zoned { local, offset };
        reading
            .utc_unix_micros()
            .filter(|micros| (MIN_UNIX_MICROS..=MAX_UNIX_MICROS).contains(micros))
            .ok_or(TimeAnchorError::OutOfRange)?;
        Ok(reading)
    }

    /// Parses an RFC 3339 date and time, also accepting what recorders write:
    /// `YYYY-MM-DDTHH:MM:SS`, an optional fraction (digits past the sixth are
    /// dropped, not rounded), then `Z`, an offset as `±HH:MM` or `±HHMM`, or
    /// nothing. `Z` and `-00:00` (RFC 3339's "offset unknown") give an instant
    /// with no local zone; an offset gives an instant with its local time; no
    /// zone at all gives a local time whose zone is not known. `T` and `Z` may
    /// be lower case. Nothing else is accepted: no spaces, no week dates, no
    /// leap seconds.
    ///
    /// # Errors
    ///
    /// [`TimeAnchorError::TooLong`], [`TimeAnchorError::Malformed`],
    /// [`TimeAnchorError::OutOfRange`] or [`TimeAnchorError::InvalidOffset`].
    pub fn parse(text: &str) -> Result<Self, TimeAnchorError> {
        if text.len() > MAX_CLOCK_TEXT_BYTES || !text.is_ascii() {
            return Err(TimeAnchorError::TooLong);
        }
        let bytes = text.as_bytes();
        if bytes.len() < 19
            || bytes[4] != b'-'
            || bytes[7] != b'-'
            || !matches!(bytes[10], b'T' | b't')
            || bytes[13] != b':'
            || bytes[16] != b':'
        {
            return Err(TimeAnchorError::Malformed);
        }
        let year = digits(&bytes[0..4])?;
        let month = digits(&bytes[5..7])?;
        let day = digits(&bytes[8..10])?;
        let hour = digits(&bytes[11..13])?;
        let minute = digits(&bytes[14..16])?;
        let second = digits(&bytes[17..19])?;
        let mut rest = &bytes[19..];
        let mut micros = 0_u32;
        if let Some((b'.', after)) = rest.split_first() {
            let length = after
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            if length == 0 {
                return Err(TimeAnchorError::Malformed);
            }
            let mut scale = 100_000_u32;
            for &digit in after.iter().take(length.min(6)) {
                micros += u32::from(digit - b'0') * scale;
                scale /= 10;
            }
            rest = &after[length..];
        }
        let local = CivilDateTime::new(
            u16::try_from(year).map_err(|_| TimeAnchorError::OutOfRange)?,
            u8::try_from(month).map_err(|_| TimeAnchorError::OutOfRange)?,
            u8::try_from(day).map_err(|_| TimeAnchorError::OutOfRange)?,
            u8::try_from(hour).map_err(|_| TimeAnchorError::OutOfRange)?,
            u8::try_from(minute).map_err(|_| TimeAnchorError::OutOfRange)?,
            u8::try_from(second).map_err(|_| TimeAnchorError::OutOfRange)?,
            micros,
        )?;
        match rest {
            [] => Ok(Self::LocalUnknownZone { local }),
            [b'Z' | b'z'] => Ok(Self::Utc {
                unix_micros: local.as_unix_micros_if_utc(),
            }),
            [sign @ (b'+' | b'-'), zone @ ..] => {
                let (hours, minutes) = match zone {
                    [h1, h2, b':', m1, m2] | [h1, h2, m1, m2] => {
                        (digits(&[*h1, *h2])?, digits(&[*m1, *m2])?)
                    }
                    _ => return Err(TimeAnchorError::Malformed),
                };
                if minutes > 59 {
                    return Err(TimeAnchorError::InvalidOffset);
                }
                let magnitude = i32::try_from(hours * 3_600 + minutes * 60)
                    .map_err(|_| TimeAnchorError::InvalidOffset)?;
                if *sign == b'-' && magnitude == 0 {
                    // RFC 3339 4.3: "-00:00" states the instant in UTC and
                    // says the local offset is unknown.
                    return Ok(Self::Utc {
                        unix_micros: local.as_unix_micros_if_utc(),
                    });
                }
                let seconds = if *sign == b'-' { -magnitude } else { magnitude };
                Self::zoned(local, UtcOffset::from_seconds(seconds)?)
            }
            _ => Err(TimeAnchorError::Malformed),
        }
    }

    /// The exact instant, in microseconds since the Unix epoch, when the
    /// reading names one.
    #[must_use]
    pub fn utc_unix_micros(self) -> Option<i64> {
        match self {
            Self::Zoned { local, offset } => local
                .as_unix_micros_if_utc()
                .checked_sub(i64::from(offset.as_seconds()) * MICROS_PER_SECOND),
            Self::Utc { unix_micros } => Some(unix_micros),
            Self::LocalUnknownZone { .. } => None,
        }
    }

    /// The local date and time, when the reading knows it.
    #[must_use]
    pub const fn local(self) -> Option<CivilDateTime> {
        match self {
            Self::Zoned { local, .. } | Self::LocalUnknownZone { local } => Some(local),
            Self::Utc { .. } => None,
        }
    }

    /// The local clock's offset from UTC, when known.
    #[must_use]
    pub const fn offset(self) -> Option<UtcOffset> {
        match self {
            Self::Zoned { offset, .. } => Some(offset),
            Self::Utc { .. } | Self::LocalUnknownZone { .. } => None,
        }
    }

    /// The reading `delta_micros` later (earlier when negative), in the same
    /// zone; `None` when it would leave years 1 to 9999.
    #[must_use]
    pub fn shifted(self, delta_micros: i64) -> Option<Self> {
        match self {
            Self::Zoned { local, offset } => {
                Self::zoned(local.checked_add_micros(delta_micros)?, offset).ok()
            }
            Self::Utc { unix_micros } => Self::utc(unix_micros.checked_add(delta_micros)?).ok(),
            Self::LocalUnknownZone { local } => Some(Self::LocalUnknownZone {
                local: local.checked_add_micros(delta_micros)?,
            }),
        }
    }

    /// Whether a container tag's reading can be a recording's time: no earlier
    /// than [`EARLIEST_TAGGED_RECORDING_UNIX_SECONDS`] and no later than a day
    /// past `now_unix_seconds`. A local time with no zone is judged as if it
    /// were UTC, with fourteen hours' latitude either way.
    #[must_use]
    pub fn is_plausible_recording_time(self, now_unix_seconds: u64) -> bool {
        let Ok(now) = i64::try_from(now_unix_seconds) else {
            return false;
        };
        let (micros, latitude) = match self.utc_unix_micros() {
            Some(micros) => (micros, 0),
            None => match self.local() {
                Some(local) => (
                    local.as_unix_micros_if_utc(),
                    i64::from(MAX_OFFSET_SECONDS) * MICROS_PER_SECOND,
                ),
                None => return false,
            },
        };
        let earliest = EARLIEST_TAGGED_RECORDING_UNIX_SECONDS * MICROS_PER_SECOND - latitude;
        let latest = (now + TAG_FUTURE_ALLOWANCE_SECONDS) * MICROS_PER_SECOND + latitude;
        (earliest..=latest).contains(&micros)
    }
}

/// Where an anchor's clock reading came from. Declaration order is the ladder:
/// a session's clock times come from the first source it has.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AnchorSource {
    /// The operator stated the recording's start (`ingest --recorded-at`).
    Operator,
    /// The recording app's own local-time tag
    /// (`com.apple.quicktime.creationdate`), which usually carries its zone.
    QuicktimeCreationDate,
    /// A clock on the screen whose zone the user confirmed: an instant. The
    /// user confirmed a zone, not that the clock was right, so it does not
    /// displace the recorder's tag (ADR 0029 decision 15).
    ScreenClockConfirmed,
    /// The container's UTC creation time (`creation_time`: an MP4 movie
    /// header or Matroska's `DateUTC`). Many tools write the time they encoded
    /// or remuxed the file, and some devices the end of the recording.
    ContainerCreationTime,
    /// A clock on the screen that states its zone: an instant.
    ScreenTimestamp,
    /// A dated clock on the screen with no zone: a local time, no instant.
    ScreenClock,
}

impl AnchorSource {
    /// Every source, in ladder order.
    pub const ALL: [Self; 6] = [
        Self::Operator,
        Self::QuicktimeCreationDate,
        Self::ScreenClockConfirmed,
        Self::ContainerCreationTime,
        Self::ScreenTimestamp,
        Self::ScreenClock,
    ];

    /// Stable public and stored identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::QuicktimeCreationDate => "quicktime_creationdate",
            Self::ScreenClockConfirmed => "screen_clock_confirmed",
            Self::ContainerCreationTime => "container_creation_time",
            Self::ScreenTimestamp => "screen_timestamp",
            Self::ScreenClock => "screen_clock",
        }
    }

    /// The source named by `identifier`, if any.
    #[must_use]
    pub fn from_identifier(identifier: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|source| source.identifier() == identifier)
    }

    /// How far a reading from this source is trusted; fixed per source.
    #[must_use]
    pub const fn confidence(self) -> AnchorConfidence {
        match self {
            Self::Operator => AnchorConfidence::Exact,
            Self::QuicktimeCreationDate => AnchorConfidence::High,
            Self::ScreenClockConfirmed
            | Self::ContainerCreationTime
            | Self::ScreenTimestamp
            | Self::ScreenClock => AnchorConfidence::Medium,
        }
    }

    /// Whether the source is a clock on the screen, which may give one anchor
    /// per clock; each stage-1 source gives at most one.
    #[must_use]
    pub const fn is_on_screen(self) -> bool {
        matches!(
            self,
            Self::ScreenClockConfirmed | Self::ScreenTimestamp | Self::ScreenClock
        )
    }
}

/// A clock on the screen, in the order VSift first registered it: `c1`, `c2`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ClockId(u8);

impl ClockId {
    /// The clock of this ordinal, from 1 to [`MAX_SCREEN_CLOCKS`].
    #[must_use]
    pub const fn new(ordinal: u8) -> Option<Self> {
        if ordinal == 0 || ordinal as usize > MAX_SCREEN_CLOCKS {
            None
        } else {
            Some(Self(ordinal))
        }
    }

    /// The clock `c<ordinal>` names.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('c')?;
        if digits.is_empty() || digits.starts_with('0') {
            return None;
        }
        Self::new(digits.parse().ok()?)
    }

    /// Its ordinal, from 1.
    #[must_use]
    pub const fn ordinal(self) -> u8 {
        self.0
    }
}

impl fmt::Display for ClockId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "c{}", self.0)
    }
}

/// How far an anchor is trusted, strongest first.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AnchorConfidence {
    /// Stated by the operator: taken as exact.
    Exact,
    /// Written by the recording app as it recorded.
    High,
    /// Written by whatever last made the file: often not the recording's start.
    Medium,
}

impl AnchorConfidence {
    /// Stable public identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::High => "high",
            Self::Medium => "medium",
        }
    }
}

/// Whether a session's anchors agree.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AnchorConsistency {
    /// One anchor, or several of which no two can be compared (an instant
    /// with no zone and a local time with no zone).
    Single,
    /// Every two anchors that can be compared agree within
    /// [`ANCHOR_AGREEMENT_MICROS`], and at least two can be.
    Consistent,
    /// Two comparable anchors disagree. Clock times still come from the
    /// ladder's first anchor; the reader is told they are disputed.
    Conflicting,
}

impl AnchorConsistency {
    /// Stable public identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Consistent => "consistent",
            Self::Conflicting => "conflicting",
        }
    }
}

/// One position of the source timeline tied to one clock reading.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TimeAnchor {
    source: AnchorSource,
    media_time: MediaTime,
    reading: ClockReading,
    screen: Option<ScreenAnchor>,
}

/// What an anchor from a clock on the screen adds: which clock, and how far
/// its reading can be from the truth.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ScreenAnchor {
    /// The clock.
    pub clock: ClockId,
    /// How wide its offset interval is, in microseconds.
    pub precision_micros: u64,
}

impl TimeAnchor {
    /// Ties `reading` to `media_time`, from `source`.
    #[must_use]
    pub const fn new(source: AnchorSource, media_time: MediaTime, reading: ClockReading) -> Self {
        Self {
            source,
            media_time,
            reading,
            screen: None,
        }
    }

    /// An anchor from a clock on the screen.
    #[must_use]
    pub const fn on_screen(
        source: AnchorSource,
        media_time: MediaTime,
        reading: ClockReading,
        screen: ScreenAnchor,
    ) -> Self {
        Self {
            source,
            media_time,
            reading,
            screen: Some(screen),
        }
    }

    /// Which clock on the screen it came from, if one did.
    #[must_use]
    pub const fn screen(&self) -> Option<ScreenAnchor> {
        self.screen
    }

    /// Where the reading came from.
    #[must_use]
    pub const fn source(&self) -> AnchorSource {
        self.source
    }

    /// The source's fixed confidence.
    #[must_use]
    pub const fn confidence(&self) -> AnchorConfidence {
        self.source.confidence()
    }

    /// The timeline position the reading belongs to.
    #[must_use]
    pub const fn media_time(&self) -> MediaTime {
        self.media_time
    }

    /// The clock reading at [`Self::media_time`].
    #[must_use]
    pub const fn reading(&self) -> ClockReading {
        self.reading
    }

    /// How far its reading can be from the truth, by what it was read off.
    const fn precision_micros(&self) -> i64 {
        match self.screen {
            Some(detail) => detail.precision_micros.cast_signed(),
            None => 0,
        }
    }

    /// The clock reading at another timeline position: the anchor's reading
    /// moved by the distance between the two. `None` outside years 1 to 9999.
    #[must_use]
    pub fn reading_at(&self, media_time: MediaTime) -> Option<ClockReading> {
        let to = i128::from(media_time.as_micros());
        let from = i128::from(self.media_time.as_micros());
        let delta = i64::try_from(to - from).ok()?;
        self.reading.shifted(delta)
    }
}

/// The clock time of one timeline position and where it came from.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ClockTime {
    /// The clock reading.
    pub reading: ClockReading,
    /// The anchor's source.
    pub source: AnchorSource,
    /// The anchor's confidence.
    pub confidence: AnchorConfidence,
    /// The clock on the screen it came from, if one did.
    pub clock: Option<ClockId>,
    /// Which segment of the timeline it lies in, from 0, when a gap divides the
    /// timeline. After a `detected` gap this says the time may be off by it.
    pub segment: Option<usize>,
}

/// Most gaps one session keeps.
pub const MAX_CLOCK_GAPS: usize = 32;

/// A gap a clock showed, in the order VSift first registered it: `g1`, `g2`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GapId(u8);

impl GapId {
    /// The gap of this ordinal, from 1 to [`MAX_CLOCK_GAPS`].
    #[must_use]
    pub const fn new(ordinal: u8) -> Option<Self> {
        if ordinal == 0 || ordinal as usize > MAX_CLOCK_GAPS {
            None
        } else {
            Some(Self(ordinal))
        }
    }

    /// The gap `g<ordinal>` names.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('g')?;
        if digits.is_empty() || digits.starts_with('0') {
            return None;
        }
        Self::new(digits.parse().ok()?)
    }

    /// Its ordinal, from 1.
    #[must_use]
    pub const fn ordinal(self) -> u8 {
        self.0
    }
}

impl fmt::Display for GapId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "g{}", self.0)
    }
}

/// Whether a person has said a gap is real.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GapState {
    /// Found by `clocks`. It changes no value: this design cannot tell a cut
    /// from a change of machine (ADR 0029 decision 13).
    Detected,
    /// A person accepted it, so clock times after it move by its length.
    Accepted,
}

impl GapState {
    /// Every state.
    pub const ALL: [Self; 2] = [Self::Detected, Self::Accepted];

    /// Stable public identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Detected => "detected",
            Self::Accepted => "accepted",
        }
    }

    /// The state named by `identifier`, if any.
    #[must_use]
    pub fn from_identifier(identifier: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|state| state.identifier() == identifier)
    }
}

/// Where a clock showed time may be missing from the video.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ClockGap {
    /// Its id.
    pub id: GapId,
    /// The clock that showed it.
    pub clock: ClockId,
    /// The first moment after it: where the next segment starts.
    pub at: MediaTime,
    /// The clock's jump, in microseconds: the lower bound.
    pub jump_low_micros: i64,
    /// Just past the upper bound.
    pub jump_high_micros: i64,
    /// Whether a person accepted it.
    pub state: GapState,
}

impl ClockGap {
    /// The jump's middle, which is what an accepted gap moves clock times by.
    #[must_use]
    pub const fn jump_micros(&self) -> i64 {
        self.jump_low_micros + (self.jump_high_micros - self.jump_low_micros) / 2
    }

    /// How far the middle can be from the true jump.
    #[must_use]
    pub const fn uncertainty_micros(&self) -> i64 {
        (self.jump_high_micros - self.jump_low_micros) / 2
    }

    /// What it adds to the clock times after it: nothing until accepted.
    #[must_use]
    pub const fn adjustment_micros(&self) -> i64 {
        match self.state {
            GapState::Detected => 0,
            GapState::Accepted => self.jump_micros(),
        }
    }
}

/// A session's anchors in ladder order: at most one per stage-1 source, and one
/// per clock and moment for a clock on the screen, with the gaps that divide
/// its timeline into segments.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct TimeAnchors {
    anchors: Vec<TimeAnchor>,
    gaps: Vec<ClockGap>,
}

impl TimeAnchors {
    /// No anchors: clock times are unknown.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            anchors: Vec::new(),
            gaps: Vec::new(),
        }
    }

    /// These anchors with `gaps`, in timeline order, at most [`MAX_CLOCK_GAPS`].
    #[must_use]
    pub fn with_gaps(&self, mut gaps: Vec<ClockGap>) -> Self {
        gaps.sort_by_key(|gap| (gap.at, gap.id));
        gaps.truncate(MAX_CLOCK_GAPS);
        Self {
            anchors: self.anchors.clone(),
            gaps,
        }
    }

    /// The gaps, in timeline order.
    #[must_use]
    pub fn gaps(&self) -> &[ClockGap] {
        &self.gaps
    }

    /// How many gaps lie at or before `media_time`: the segment it is in.
    fn segment_at(&self, media_time: MediaTime) -> usize {
        self.gaps.iter().filter(|gap| gap.at <= media_time).count()
    }

    /// What accepted gaps add at `media_time`, counted from the start.
    fn adjustment_at(&self, media_time: MediaTime) -> i64 {
        self.gaps
            .iter()
            .filter(|gap| gap.at <= media_time)
            .map(ClockGap::adjustment_micros)
            .sum()
    }

    /// `anchor`'s reading at `media_time`, moved by the accepted gaps between
    /// them. A detected gap moves nothing.
    fn reading_of(&self, anchor: &TimeAnchor, media_time: MediaTime) -> Option<ClockReading> {
        let moved = self.adjustment_at(media_time) - self.adjustment_at(anchor.media_time);
        anchor.reading_at(media_time)?.shifted(moved)
    }

    /// Orders `anchors` by the ladder.
    ///
    /// # Errors
    ///
    /// [`TimeAnchorError::TooManyAnchors`] for more than [`MAX_TIME_ANCHORS`],
    /// two anchors from one stage-1 source, or two of one clock at one moment.
    pub fn new(mut anchors: Vec<TimeAnchor>) -> Result<Self, TimeAnchorError> {
        if anchors.len() > MAX_TIME_ANCHORS {
            return Err(TimeAnchorError::TooManyAnchors);
        }
        anchors.sort_by_key(ladder_key);
        if anchors.windows(2).any(|pair| {
            pair[0].source == pair[1].source
                && (!pair[0].source.is_on_screen() || ladder_key(&pair[0]) == ladder_key(&pair[1]))
        }) {
            return Err(TimeAnchorError::TooManyAnchors);
        }
        Ok(Self {
            anchors,
            gaps: Vec::new(),
        })
    }

    /// These anchors with their screen anchors replaced by `screen`, the most
    /// precise first where the bound of [`MAX_TIME_ANCHORS`] leaves no room:
    /// the report lists every clock, so a dropped anchor loses nothing.
    #[must_use]
    pub fn with_screen_anchors(&self, mut screen: Vec<TimeAnchor>) -> Self {
        let mut anchors: Vec<TimeAnchor> = self
            .anchors
            .iter()
            .filter(|anchor| !anchor.source.is_on_screen())
            .copied()
            .collect();
        screen.sort_by_key(|anchor| anchor.screen.map(|detail| detail.precision_micros));
        screen.truncate(MAX_TIME_ANCHORS.saturating_sub(anchors.len()));
        anchors.extend(screen);
        anchors.sort_by_key(ladder_key);
        anchors.dedup_by_key(|anchor| ladder_key(anchor));
        Self {
            anchors,
            gaps: self.gaps.clone(),
        }
    }

    /// The anchors of a newly ingested source: the operator's statement, if
    /// any, at the start of the timeline, and each container tag whose reading
    /// is a plausible recording time at `now_unix_seconds` (implausible ones
    /// are dropped, not stored), at most one per source (the first of each
    /// wins). A tag claiming to be the operator is ignored.
    #[must_use]
    pub fn from_ingest(
        stated: Option<ClockReading>,
        tags: &[TimeAnchor],
        now_unix_seconds: u64,
    ) -> Self {
        let mut anchors: Vec<TimeAnchor> = stated
            .map(|reading| {
                TimeAnchor::new(AnchorSource::Operator, MediaTime::from_micros(0), reading)
            })
            .into_iter()
            .collect();
        for tag in tags {
            if tag.source != AnchorSource::Operator
                && tag.reading.is_plausible_recording_time(now_unix_seconds)
                && anchors.iter().all(|anchor| anchor.source != tag.source)
                && anchors.len() < MAX_TIME_ANCHORS
            {
                anchors.push(*tag);
            }
        }
        anchors.sort_by_key(|anchor| anchor.source);
        Self {
            anchors,
            gaps: Vec::new(),
        }
    }

    /// Every anchor, in ladder order.
    #[must_use]
    pub fn anchors(&self) -> &[TimeAnchor] {
        &self.anchors
    }

    /// Whether the session has no anchor.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }

    /// The anchor clock times come from: the ladder's first.
    #[must_use]
    pub fn selected(&self) -> Option<&TimeAnchor> {
        self.anchors.first()
    }

    /// Whether the anchors agree; `None` without anchors.
    ///
    /// Two anchors are compared at the start of the timeline, by their
    /// instants, and only when both name one. A local time whose zone is
    /// unknown is never compared, not even with another one or with a zoned
    /// reading's local time: a clock face is not an instant (maintainer,
    /// 2026-10-21; ADR 0029 section 7). So `single` means that no two
    /// absolute anchors exist to compare.
    #[must_use]
    pub fn consistency(&self) -> Option<AnchorConsistency> {
        self.selected()?;
        let start = MediaTime::from_micros(0);
        let readings: Vec<(ClockReading, i64)> = self
            .anchors
            .iter()
            .filter_map(|anchor| Some((self.reading_of(anchor, start)?, anchor.precision_micros())))
            .collect();
        let mut compared = false;
        for (index, (first, first_precision)) in readings.iter().enumerate() {
            for (second, second_precision) in &readings[index + 1..] {
                let allowed = ANCHOR_AGREEMENT_MICROS + first_precision + second_precision;
                match apart_micros(*first, *second) {
                    Some(apart) if apart > allowed => {
                        return Some(AnchorConsistency::Conflicting);
                    }
                    Some(_) => compared = true,
                    None => {}
                }
            }
        }
        Some(if compared {
            AnchorConsistency::Consistent
        } else {
            AnchorConsistency::Single
        })
    }

    /// The clock time of `media_time`, from the selected anchor; `None`
    /// without anchors or outside years 1 to 9999.
    #[must_use]
    pub fn clock_time_at(&self, media_time: MediaTime) -> Option<ClockTime> {
        let anchor = self.selected()?;
        Some(ClockTime {
            reading: self.reading_of(anchor, media_time)?,
            source: anchor.source,
            confidence: anchor.confidence(),
            clock: anchor.screen.map(|detail| detail.clock),
            segment: (!self.gaps.is_empty()).then(|| self.segment_at(media_time)),
        })
    }
}

/// The order of the ladder: by source, then by clock, then by position.
fn ladder_key(anchor: &TimeAnchor) -> (AnchorSource, Option<ClockId>, MediaTime) {
    (
        anchor.source,
        anchor.screen.map(|detail| detail.clock),
        anchor.media_time,
    )
}

/// How far apart two readings are, when both are absolute instants.
fn apart_micros(first: ClockReading, second: ClockReading) -> Option<i64> {
    let (a, b) = (first.utc_unix_micros()?, second.utc_unix_micros()?);
    a.checked_sub(b).map(i64::abs)
}

fn digits(bytes: &[u8]) -> Result<u32, TimeAnchorError> {
    bytes.iter().try_fold(0_u32, |value, &byte| {
        if byte.is_ascii_digit() {
            Ok(value * 10 + u32::from(byte - b'0'))
        } else {
            Err(TimeAnchorError::Malformed)
        }
    })
}

const fn is_leap_year(year: u16) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

const fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days from 1970-01-01 to a proleptic Gregorian date (H. Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The proleptic Gregorian date `days` after 1970-01-01 (H. Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn Error>>;

    fn tag(source: AnchorSource, reading: ClockReading) -> TimeAnchor {
        TimeAnchor::new(source, MediaTime::from_micros(0), reading)
    }

    fn civil(text: &str) -> Result<CivilDateTime, Box<dyn Error>> {
        ClockReading::parse(text)?
            .local()
            .ok_or_else(|| "no local time".into())
    }

    #[test]
    fn a_utc_reading_names_its_instant_and_no_local_time() -> TestResult {
        let reading = ClockReading::parse("2026-10-21T13:03:12.250000Z")?;
        assert_eq!(reading.utc_unix_micros(), Some(1_792_587_792_250_000));
        assert_eq!(reading.local(), None);
        assert_eq!(reading.offset(), None);
        Ok(())
    }

    #[test]
    fn an_offset_gives_the_instant_and_the_local_time() -> TestResult {
        let colon = ClockReading::parse("2026-10-21T14:03:12+01:00")?;
        let compact = ClockReading::parse("2026-10-21T14:03:12+0100")?;
        assert_eq!(colon, compact);
        assert_eq!(colon.utc_unix_micros(), Some(1_792_587_792_000_000));
        assert_eq!(colon.offset().map(UtcOffset::as_seconds), Some(3_600));
        let west = ClockReading::parse("2026-10-21T08:33:12-04:30")?;
        assert_eq!(west.utc_unix_micros(), Some(1_792_587_792_000_000));
        Ok(())
    }

    #[test]
    fn no_zone_is_a_local_time_with_no_instant() -> TestResult {
        let reading = ClockReading::parse("2026-10-21T14:03:12")?;
        assert_eq!(reading.utc_unix_micros(), None);
        assert_eq!(reading.local(), Some(civil("2026-10-21T14:03:12")?));
        Ok(())
    }

    #[test]
    fn minus_zero_is_an_instant_with_an_unknown_zone() -> TestResult {
        let reading = ClockReading::parse("2026-10-21T13:03:12-00:00")?;
        assert_eq!(reading, ClockReading::parse("2026-10-21T13:03:12Z")?);
        Ok(())
    }

    #[test]
    fn fractions_past_six_digits_are_dropped() -> TestResult {
        let reading = ClockReading::parse("2026-10-21T13:03:12.1234569Z")?;
        assert_eq!(
            reading.utc_unix_micros(),
            Some(1_792_587_792_123_456),
            "truncated, not rounded"
        );
        Ok(())
    }

    #[test]
    fn malformed_and_out_of_range_texts_are_refused() {
        for (text, error) in [
            ("2026-10-21 13:03:12Z", TimeAnchorError::Malformed),
            ("2026-10-21T13:03Z", TimeAnchorError::Malformed),
            ("2026-10-21T13:03:12.Z", TimeAnchorError::Malformed),
            ("2026-10-21T13:03:12+1", TimeAnchorError::Malformed),
            ("2026-10-21T13:03:12 ", TimeAnchorError::Malformed),
            ("+2026-10-21T13:03:12", TimeAnchorError::Malformed),
            ("2026-13-21T13:03:12Z", TimeAnchorError::OutOfRange),
            ("2025-02-29T13:03:12Z", TimeAnchorError::OutOfRange),
            ("2026-10-21T24:00:00Z", TimeAnchorError::OutOfRange),
            ("2026-10-21T23:59:60Z", TimeAnchorError::OutOfRange),
            ("0000-10-21T13:03:12Z", TimeAnchorError::OutOfRange),
            ("2026-10-21T13:03:12+15:00", TimeAnchorError::InvalidOffset),
            ("2026-10-21T13:03:12+01:60", TimeAnchorError::InvalidOffset),
            (
                "2026-10-21T13:03:12.000000000000000000000Z",
                TimeAnchorError::TooLong,
            ),
            ("2026-10-21T13:03:12\u{00e9}", TimeAnchorError::TooLong),
        ] {
            assert_eq!(ClockReading::parse(text), Err(error), "{text}");
        }
    }

    #[test]
    fn leap_days_and_century_rules_hold() -> TestResult {
        assert!(ClockReading::parse("2024-02-29T00:00:00Z").is_ok());
        assert!(ClockReading::parse("2000-02-29T00:00:00Z").is_ok());
        assert_eq!(
            ClockReading::parse("1900-02-29T00:00:00Z"),
            Err(TimeAnchorError::OutOfRange)
        );
        let before = civil("2024-02-28T23:59:59.500000")?;
        let after = before.checked_add_micros(1_000_000).ok_or("out of range")?;
        assert_eq!(after, civil("2024-02-29T00:00:00.500000")?);
        Ok(())
    }

    #[test]
    fn civil_dates_round_trip_through_unix_time() -> TestResult {
        for text in [
            "0001-01-01T00:00:00",
            "1969-12-31T23:59:59.999999",
            "1970-01-01T00:00:00",
            "2000-03-01T12:00:00",
            "2026-10-21T14:03:12.250000",
            "9999-12-31T23:59:59.999999",
        ] {
            let value = civil(text)?;
            assert_eq!(
                CivilDateTime::from_unix_micros(value.as_unix_micros_if_utc()),
                Some(value),
                "{text}"
            );
        }
        assert_eq!(civil("1970-01-01T00:00:00")?.as_unix_micros_if_utc(), 0);
        assert_eq!(CivilDateTime::from_unix_micros(MAX_UNIX_MICROS + 1), None);
        Ok(())
    }

    #[test]
    fn canonical_texts_round_trip() -> TestResult {
        let local = civil("2026-10-21T14:03:12.25")?;
        assert_eq!(local.to_string(), "2026-10-21T14:03:12.250000");
        assert_eq!(civil(&local.to_string())?, local);
        for (seconds, text) in [
            (0, "+00:00"),
            (3_600, "+01:00"),
            (-16_200, "-04:30"),
            (50_400, "+14:00"),
        ] {
            assert_eq!(UtcOffset::from_seconds(seconds)?.to_string(), text);
        }
        Ok(())
    }

    #[test]
    fn a_shift_crosses_midnight_in_the_reading_zone() -> TestResult {
        let reading = ClockReading::parse("2026-12-31T23:59:30+05:30")?;
        let later = reading.shifted(45_000_000).ok_or("out of range")?;
        assert_eq!(later.local(), Some(civil("2027-01-01T00:00:15")?));
        assert_eq!(later.offset(), reading.offset());
        Ok(())
    }

    #[test]
    fn container_tags_outside_the_plausible_window_are_dropped() -> TestResult {
        let now = 1_792_587_792; // 2026-10-21T13:03:12Z
        let epoch_default = ClockReading::parse("1970-01-01T00:00:00Z")?;
        let mac_zero = ClockReading::parse("1904-01-01T00:00:00Z")?;
        let future = ClockReading::parse("2026-10-23T13:03:13Z")?;
        let fine = ClockReading::parse("2026-10-21T12:00:00Z")?;
        let anchors = TimeAnchors::from_ingest(
            None,
            &[
                tag(AnchorSource::ContainerCreationTime, epoch_default),
                tag(AnchorSource::ContainerCreationTime, mac_zero),
                tag(AnchorSource::QuicktimeCreationDate, future),
            ],
            now,
        );
        assert!(anchors.is_empty());
        let anchors =
            TimeAnchors::from_ingest(None, &[tag(AnchorSource::ContainerCreationTime, fine)], now);
        assert_eq!(anchors.anchors().len(), 1);
        Ok(())
    }

    #[test]
    fn the_ladder_prefers_the_operator_then_the_recorder_tag() -> TestResult {
        let now = 1_792_587_792;
        let stated = ClockReading::parse("2026-10-21T10:00:00+01:00")?;
        let recorder = ClockReading::parse("2026-10-21T10:00:01+0100")?;
        let created = ClockReading::parse("2026-10-21T09:00:00Z")?;
        let tags = [
            tag(AnchorSource::ContainerCreationTime, created),
            tag(AnchorSource::QuicktimeCreationDate, recorder),
        ];
        let anchors = TimeAnchors::from_ingest(Some(stated), &tags, now);
        let order: Vec<AnchorSource> = anchors.anchors().iter().map(TimeAnchor::source).collect();
        assert_eq!(
            order,
            vec![
                AnchorSource::Operator,
                AnchorSource::QuicktimeCreationDate,
                AnchorSource::ContainerCreationTime
            ]
        );
        assert_eq!(
            anchors.selected().map(TimeAnchor::source),
            Some(AnchorSource::Operator)
        );
        assert_eq!(anchors.consistency(), Some(AnchorConsistency::Consistent));
        let without_operator = TimeAnchors::from_ingest(None, &tags, now);
        assert_eq!(
            without_operator.selected().map(TimeAnchor::source),
            Some(AnchorSource::QuicktimeCreationDate)
        );
        Ok(())
    }

    #[test]
    fn disagreeing_comparable_anchors_conflict() -> TestResult {
        let now = 1_792_587_792;
        let stated = ClockReading::parse("2026-10-21T10:00:00Z")?;
        let encoded = ClockReading::parse("2026-10-21T11:30:00Z")?;
        let anchors = TimeAnchors::from_ingest(
            Some(stated),
            &[tag(AnchorSource::ContainerCreationTime, encoded)],
            now,
        );
        assert_eq!(anchors.consistency(), Some(AnchorConsistency::Conflicting));
        assert_eq!(
            anchors
                .clock_time_at(MediaTime::from_micros(0))
                .map(|clock| clock.source),
            Some(AnchorSource::Operator),
            "the ladder still decides"
        );
        Ok(())
    }

    #[test]
    fn an_instant_and_a_zoneless_local_time_cannot_be_compared() -> TestResult {
        let now = 1_792_587_792;
        let local = ClockReading::parse("2026-10-21T14:00:00")?;
        let created = ClockReading::parse("2026-10-21T09:00:00Z")?;
        let anchors = TimeAnchors::from_ingest(
            None,
            &[
                tag(AnchorSource::QuicktimeCreationDate, local),
                tag(AnchorSource::ContainerCreationTime, created),
            ],
            now,
        );
        assert_eq!(anchors.consistency(), Some(AnchorConsistency::Single));
        assert_eq!(TimeAnchors::none().consistency(), None);
        Ok(())
    }

    /// "A clock face is not an instant" (maintainer, 2026-10-21; ADR 0029
    /// section 7): consistency compares absolute anchors only. A zoned
    /// reading and a local time with no zone are never compared by their
    /// local times, however far apart they look (the presenter's zone is not
    /// the recorder's), and neither are two local times with no zone.
    #[test]
    fn consistency_never_compares_a_local_time_whose_zone_is_unknown() -> TestResult {
        let now = 1_792_587_792;
        let zoneless = ClockReading::parse("2026-10-21T14:00:00")?;
        let zoned = ClockReading::parse("2026-10-21T09:00:00+01:00")?;
        let other_zoneless = ClockReading::parse("2026-10-21T19:00:00")?;
        for (stated, recorder) in [(zoneless, zoned), (zoneless, other_zoneless)] {
            let anchors = TimeAnchors::from_ingest(
                Some(stated),
                &[tag(AnchorSource::QuicktimeCreationDate, recorder)],
                now,
            );
            assert_eq!(anchors.anchors().len(), 2);
            assert_eq!(
                anchors.consistency(),
                Some(AnchorConsistency::Single),
                "{recorder:?}"
            );
        }
        // Two absolute anchors are still compared, whatever their zones.
        // 09:00+01:00 is 08:00Z; 05:00-05:00 is 10:00Z.
        let elsewhere = ClockReading::parse("2026-10-21T05:00:00-05:00")?;
        let anchors = TimeAnchors::from_ingest(
            Some(zoned),
            &[tag(AnchorSource::QuicktimeCreationDate, elsewhere)],
            now,
        );
        assert_eq!(anchors.consistency(), Some(AnchorConsistency::Conflicting));
        let same_instant = ClockReading::parse("2026-10-21T03:00:00-05:00")?;
        let utc = ClockReading::parse("2026-10-21T08:00:00Z")?;
        let anchors = TimeAnchors::from_ingest(
            Some(same_instant),
            &[tag(AnchorSource::ContainerCreationTime, utc)],
            now,
        );
        assert_eq!(anchors.consistency(), Some(AnchorConsistency::Consistent));
        Ok(())
    }

    #[test]
    fn a_clock_time_moves_with_the_timeline() -> TestResult {
        let anchors = TimeAnchors::from_ingest(
            Some(ClockReading::parse("2026-10-21T14:03:12+01:00")?),
            &[],
            0,
        );
        let clock = anchors
            .clock_time_at(MediaTime::from_micros(90_500_000))
            .ok_or("no clock time")?;
        assert_eq!(
            clock.reading.local(),
            Some(civil("2026-10-21T14:04:42.500000")?)
        );
        assert_eq!(clock.reading.utc_unix_micros(), Some(1_792_587_882_500_000));
        assert_eq!(clock.confidence, AnchorConfidence::Exact);
        assert_eq!(
            TimeAnchors::none().clock_time_at(MediaTime::from_micros(0)),
            None
        );
        Ok(())
    }

    fn screen_anchor(
        source: AnchorSource,
        ordinal: u8,
        at_seconds: u64,
        reading: ClockReading,
        precision_micros: u64,
    ) -> Result<TimeAnchor, Box<dyn Error>> {
        let clock = ClockId::new(ordinal).ok_or("no clock")?;
        Ok(TimeAnchor::on_screen(
            source,
            MediaTime::from_micros(at_seconds * 1_000_000),
            reading,
            ScreenAnchor {
                clock,
                precision_micros,
            },
        ))
    }

    fn gap(
        ordinal: u8,
        at_seconds: u64,
        jump_seconds: i64,
        state: GapState,
    ) -> Result<ClockGap, Box<dyn Error>> {
        Ok(ClockGap {
            id: GapId::new(ordinal).ok_or("no gap")?,
            clock: ClockId::new(1).ok_or("no clock")?,
            at: MediaTime::from_micros(at_seconds * 1_000_000),
            jump_low_micros: jump_seconds * 1_000_000 - 1_000,
            jump_high_micros: jump_seconds * 1_000_000 + 1_000,
            state,
        })
    }

    fn stated(text: &str) -> Result<TimeAnchors, Box<dyn Error>> {
        Ok(TimeAnchors::from_ingest(
            Some(ClockReading::parse(text)?),
            &[],
            0,
        ))
    }

    /// The planner's ruling (ADR 0029 decision 13): a detected gap marks the
    /// items after it and changes no value.
    #[test]
    fn a_detected_gap_changes_no_value_and_marks_the_segment() -> TestResult {
        let plain = stated("2026-10-21T14:00:00Z")?;
        let marked = plain.with_gaps(vec![gap(1, 300, 437, GapState::Detected)?]);
        let after = MediaTime::from_micros(400_000_000);
        let (before_gap, past_gap) = (
            marked
                .clock_time_at(MediaTime::from_micros(0))
                .ok_or("no time")?,
            marked.clock_time_at(after).ok_or("no time")?,
        );
        assert_eq!(before_gap.segment, Some(0));
        assert_eq!(past_gap.segment, Some(1));
        assert_eq!(
            past_gap.reading,
            plain.clock_time_at(after).ok_or("no time")?.reading,
            "a detected gap moves nothing"
        );
        assert_eq!(
            plain.clock_time_at(after).and_then(|clock| clock.segment),
            None
        );
        Ok(())
    }

    #[test]
    fn an_accepted_gap_moves_only_what_lies_after_it() -> TestResult {
        let anchors =
            stated("2026-10-21T14:00:00Z")?.with_gaps(vec![gap(1, 300, 437, GapState::Accepted)?]);
        let at = |seconds: u64| {
            anchors
                .clock_time_at(MediaTime::from_micros(seconds * 1_000_000))
                .and_then(|clock| clock.reading.utc_unix_micros())
        };
        let start = at(0).ok_or("no time")?;
        assert_eq!(at(299), Some(start + 299_000_000));
        assert_eq!(at(300), Some(start + (300 + 437) * 1_000_000));
        Ok(())
    }

    #[test]
    fn gaps_add_up_and_a_screen_anchor_past_one_is_measured_from_where_it_is() -> TestResult {
        let local = ClockReading::parse("2026-10-21T14:10:00")?;
        let anchors = TimeAnchors::none()
            .with_screen_anchors(vec![screen_anchor(
                AnchorSource::ScreenClock,
                1,
                400,
                local,
                1,
            )?])
            .with_gaps(vec![
                gap(1, 300, 100, GapState::Accepted)?,
                gap(2, 500, 50, GapState::Accepted)?,
            ]);
        let at = |seconds: u64| {
            anchors
                .clock_time_at(MediaTime::from_micros(seconds * 1_000_000))
                .and_then(|clock| clock.reading.local())
        };
        assert_eq!(at(400), Some(civil("2026-10-21T14:10:00")?));
        assert_eq!(at(0), Some(civil("2026-10-21T14:01:40")?));
        assert_eq!(at(500), Some(civil("2026-10-21T14:12:30")?));
        Ok(())
    }

    #[test]
    fn an_anchor_past_a_detected_gap_disagrees_until_a_person_accepts_it() -> TestResult {
        let base = stated("2026-10-21T14:00:00Z")?;
        // 300 s in, but the clock says 437 s more than that.
        let reading = ClockReading::parse("2026-10-21T14:12:17Z")?;
        let anchors = base.with_screen_anchors(vec![screen_anchor(
            AnchorSource::ScreenTimestamp,
            1,
            300,
            reading,
            1,
        )?]);
        let detected = anchors.with_gaps(vec![gap(1, 300, 437, GapState::Detected)?]);
        assert_eq!(detected.consistency(), Some(AnchorConsistency::Conflicting));
        let accepted = anchors.with_gaps(vec![gap(1, 300, 437, GapState::Accepted)?]);
        assert_eq!(accepted.consistency(), Some(AnchorConsistency::Consistent));
        Ok(())
    }

    #[test]
    fn gap_ids_read_back_and_refuse_what_is_not_one() {
        for ordinal in [1, 2, 32] {
            let id = GapId::new(ordinal);
            assert_eq!(id.and_then(|id| GapId::parse(&id.to_string())), id);
        }
        for text in ["g0", "g33", "g", "c1", "g01"] {
            assert_eq!(GapId::parse(text), None, "{text}");
        }
    }

    #[test]
    fn the_ladders_confidence_never_rises_as_it_goes_down() {
        for pair in AnchorSource::ALL.windows(2) {
            assert!(pair[0].confidence() <= pair[1].confidence(), "{pair:?}");
        }
    }

    #[test]
    fn a_confirmed_clock_ranks_below_the_recorder_and_above_the_container() {
        let rank = |source| AnchorSource::ALL.iter().position(|held| *held == source);
        assert!(
            rank(AnchorSource::QuicktimeCreationDate) < rank(AnchorSource::ScreenClockConfirmed)
        );
        assert!(
            rank(AnchorSource::ScreenClockConfirmed) < rank(AnchorSource::ContainerCreationTime)
        );
        assert!(rank(AnchorSource::ContainerCreationTime) < rank(AnchorSource::ScreenTimestamp));
        assert!(rank(AnchorSource::ScreenTimestamp) < rank(AnchorSource::ScreenClock));
    }

    #[test]
    fn clock_ids_read_back_and_refuse_what_is_not_one() {
        for ordinal in 1..=8 {
            let id = ClockId::new(ordinal);
            assert_eq!(id.and_then(|id| ClockId::parse(&id.to_string())), id);
        }
        for text in ["c0", "c9", "c", "1", "c01", "d1", "c1 "] {
            assert_eq!(ClockId::parse(text), None, "{text}");
        }
    }

    #[test]
    fn running_clocks_leaves_a_stage_1_selection_alone() -> TestResult {
        let created = ClockReading::parse("2026-10-21T09:00:00Z")?;
        let anchors = TimeAnchors::from_ingest(
            None,
            &[tag(AnchorSource::ContainerCreationTime, created)],
            1_792_587_792,
        );
        let on_screen = ClockReading::parse("2026-10-21T09:00:30Z")?;
        let with_clock = anchors.with_screen_anchors(vec![screen_anchor(
            AnchorSource::ScreenTimestamp,
            1,
            30,
            on_screen,
            500_000,
        )?]);
        assert_eq!(
            with_clock.selected().map(TimeAnchor::source),
            Some(AnchorSource::ContainerCreationTime)
        );
        assert_eq!(with_clock.anchors().len(), 2);
        Ok(())
    }

    #[test]
    fn a_session_with_no_other_anchor_takes_its_clock_time_from_a_dated_clock() -> TestResult {
        let local = ClockReading::parse("2026-10-21T14:00:30")?;
        let anchors = TimeAnchors::none().with_screen_anchors(vec![screen_anchor(
            AnchorSource::ScreenClock,
            2,
            30,
            local,
            500_000,
        )?]);
        let clock = anchors
            .clock_time_at(MediaTime::from_micros(90_000_000))
            .ok_or("no clock time")?;
        assert_eq!(clock.source, AnchorSource::ScreenClock);
        assert_eq!(clock.clock, ClockId::new(2));
        assert_eq!(clock.reading.local(), Some(civil("2026-10-21T14:01:30")?));
        assert_eq!(clock.reading.utc_unix_micros(), None);
        Ok(())
    }

    #[test]
    fn a_screen_anchor_is_compared_with_its_precision_allowed() -> TestResult {
        let created = ClockReading::parse("2026-10-21T09:00:00Z")?;
        let base = TimeAnchors::from_ingest(
            None,
            &[tag(AnchorSource::ContainerCreationTime, created)],
            1_792_587_792,
        );
        // Five seconds off at time 0: disputed unless the clock's width covers it.
        let reading = ClockReading::parse("2026-10-21T09:00:35Z")?;
        let exact = base.with_screen_anchors(vec![screen_anchor(
            AnchorSource::ScreenTimestamp,
            1,
            30,
            reading,
            0,
        )?]);
        assert_eq!(exact.consistency(), Some(AnchorConsistency::Conflicting));
        let wide = base.with_screen_anchors(vec![screen_anchor(
            AnchorSource::ScreenTimestamp,
            1,
            30,
            reading,
            4_000_000,
        )?]);
        assert_eq!(wide.consistency(), Some(AnchorConsistency::Consistent));
        Ok(())
    }

    #[test]
    fn two_clocks_may_each_give_an_anchor_but_a_stage_1_source_only_one() -> TestResult {
        let reading = ClockReading::parse("2026-10-21T14:00:30")?;
        let first = screen_anchor(AnchorSource::ScreenClock, 1, 30, reading, 1)?;
        let second = screen_anchor(AnchorSource::ScreenClock, 2, 30, reading, 1)?;
        assert!(TimeAnchors::new(vec![first, second]).is_ok());
        assert_eq!(
            TimeAnchors::new(vec![first, first]),
            Err(TimeAnchorError::TooManyAnchors)
        );
        Ok(())
    }

    #[test]
    fn the_bound_keeps_the_most_precise_screen_anchors() -> TestResult {
        let reading = ClockReading::parse("2026-10-21T14:00:30")?;
        let stated = ClockReading::parse("2026-10-21T14:00:00")?;
        let base = TimeAnchors::from_ingest(Some(stated), &[], 0);
        let many = (1..=8_u8)
            .flat_map(|clock| (0..4_u64).map(move |at| (clock, at)))
            .map(|(clock, at)| {
                screen_anchor(
                    AnchorSource::ScreenClock,
                    clock,
                    at,
                    reading,
                    u64::from(clock),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let bounded = base.with_screen_anchors(many);
        assert_eq!(bounded.anchors().len(), MAX_TIME_ANCHORS);
        assert!(bounded.anchors().iter().all(|anchor| {
            anchor
                .screen()
                .is_none_or(|detail| detail.precision_micros <= 4)
        }));
        Ok(())
    }

    #[test]
    fn stored_anchor_lists_are_bounded_and_one_per_source() -> TestResult {
        let reading = ClockReading::parse("2026-10-21T10:00:00Z")?;
        let anchor = TimeAnchor::new(AnchorSource::Operator, MediaTime::from_micros(0), reading);
        assert_eq!(
            TimeAnchors::new(vec![anchor, anchor]),
            Err(TimeAnchorError::TooManyAnchors)
        );
        assert_eq!(
            TimeAnchors::new(vec![anchor; MAX_TIME_ANCHORS + 1]),
            Err(TimeAnchorError::TooManyAnchors)
        );
        assert!(TimeAnchors::new(vec![anchor]).is_ok());
        for source in AnchorSource::ALL {
            assert_eq!(
                AnchorSource::from_identifier(source.identifier()),
                Some(source)
            );
        }
        Ok(())
    }
}
