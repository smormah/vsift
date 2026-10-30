//! The stored form of a session's time anchors (time anchoring, ADR 0029).
//!
//! Anchors are written at activation, as an optional member of the session's
//! lifecycle record, and copied into every later generation; `clocks` replaces
//! the anchors that come from clocks on the screen, and only those, and keeps
//! the session's register of clocks beside them (ADR 0029 decision 18). A
//! session without anchors writes nothing, so its record is
//! byte for byte what earlier releases wrote; a release before 0.3.0 refuses
//! a record that has them (`deny_unknown_fields`), the trade-off known limit
//! L-044 accepts for every additive lifecycle member.
//!
//! Each reading stores only what its source said: a zoned reading its local
//! time and offset (the instant follows from them), an instant its
//! microseconds, a zoneless local time its local time. Decoding rebuilds the
//! reading through the domain constructors, so a modified record cannot
//! claim a combination the domain does not allow.

use serde::{Deserialize, Serialize};
use vsift_application::SessionStorageError;
use vsift_domain::{
    AnchorSource, ClockFormat, ClockId, ClockReading, ClockUnit, MAX_SCREEN_CLOCKS, MediaTime,
    OffsetInterval, RegisteredClock, ScreenAnchor, TimeAnchor, TimeAnchors, UtcOffset,
};

/// One stored anchor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredTimeAnchor {
    source: String,
    media_time_us: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    utc_unix_us: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    local: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    utc_offset_seconds: Option<i32>,
    /// The clock on the screen it came from, if one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clock: Option<String>,
    /// How wide that clock's offset interval was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    precision_us: Option<u64>,
}

/// One clock of the session's register.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredScreenClock {
    id: String,
    format: String,
    unit: String,
    offset_low_us: i64,
    offset_high_us: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confirmed_offset_seconds: Option<i32>,
}

/// The stored form of the session's register of clocks.
pub(super) fn encode_screen_clocks(clocks: &[RegisteredClock]) -> Vec<StoredScreenClock> {
    clocks
        .iter()
        .map(|clock| StoredScreenClock {
            id: clock.id.to_string(),
            format: clock.format.identifier().to_owned(),
            unit: clock.unit.identifier().to_owned(),
            offset_low_us: clock.offset.low(),
            offset_high_us: clock.offset.high(),
            confirmed_offset_seconds: clock.confirmed_offset.map(UtcOffset::as_seconds),
        })
        .collect()
}

/// The register a lifecycle record stores, validated.
///
/// # Errors
///
/// [`SessionStorageError::IntegrityFailure`] for more clocks than a session
/// keeps, a repeated id, or a member the domain does not know.
pub(super) fn decode_screen_clocks(
    stored: &[StoredScreenClock],
) -> Result<Vec<RegisteredClock>, SessionStorageError> {
    if stored.len() > MAX_SCREEN_CLOCKS {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let mut clocks: Vec<RegisteredClock> = Vec::with_capacity(stored.len());
    for clock in stored {
        let decoded = decode_screen_clock(clock).ok_or(SessionStorageError::IntegrityFailure)?;
        if clocks.iter().any(|held| held.id == decoded.id) {
            return Err(SessionStorageError::IntegrityFailure);
        }
        clocks.push(decoded);
    }
    Ok(clocks)
}

fn decode_screen_clock(stored: &StoredScreenClock) -> Option<RegisteredClock> {
    Some(RegisteredClock {
        id: ClockId::parse(&stored.id)?,
        format: ClockFormat::from_identifier(&stored.format)?,
        unit: ClockUnit::from_identifier(&stored.unit)?,
        offset: OffsetInterval::new(stored.offset_low_us, stored.offset_high_us)?,
        confirmed_offset: match stored.confirmed_offset_seconds {
            Some(seconds) => Some(UtcOffset::from_seconds(seconds).ok()?),
            None => None,
        },
    })
}

/// The stored form of `anchors`, in ladder order.
pub(super) fn encode_time_anchors(anchors: &TimeAnchors) -> Vec<StoredTimeAnchor> {
    anchors
        .anchors()
        .iter()
        .map(|anchor| {
            let (utc_unix_us, local, utc_offset_seconds) = match anchor.reading() {
                ClockReading::Zoned { local, offset } => {
                    (None, Some(local.to_string()), Some(offset.as_seconds()))
                }
                ClockReading::Utc { unix_micros } => (Some(unix_micros), None, None),
                ClockReading::LocalUnknownZone { local } => (None, Some(local.to_string()), None),
            };
            StoredTimeAnchor {
                source: anchor.source().identifier().to_owned(),
                media_time_us: anchor.media_time().as_micros(),
                utc_unix_us,
                local,
                utc_offset_seconds,
                clock: anchor.screen().map(|screen| screen.clock.to_string()),
                precision_us: anchor.screen().map(|screen| screen.precision_micros),
            }
        })
        .collect()
}

/// The anchors a lifecycle record stores, validated.
///
/// # Errors
///
/// [`SessionStorageError::IntegrityFailure`] for an unknown source, a local
/// time that is not canonical, a combination of members no reading has, or
/// a list the domain refuses (too long, two from one source).
pub(super) fn decode_time_anchors(
    stored: &[StoredTimeAnchor],
) -> Result<TimeAnchors, SessionStorageError> {
    let anchors = stored
        .iter()
        .map(decode_anchor)
        .collect::<Result<Vec<_>, _>>()?;
    TimeAnchors::new(anchors).map_err(|_| SessionStorageError::IntegrityFailure)
}

fn decode_anchor(stored: &StoredTimeAnchor) -> Result<TimeAnchor, SessionStorageError> {
    let source = AnchorSource::from_identifier(&stored.source)
        .ok_or(SessionStorageError::IntegrityFailure)?;
    let local = stored
        .local
        .as_deref()
        .map(|text| match ClockReading::parse(text) {
            Ok(ClockReading::LocalUnknownZone { local }) if local.to_string() == text => Ok(local),
            _ => Err(SessionStorageError::IntegrityFailure),
        })
        .transpose()?;
    let reading = match (stored.utc_unix_us, local, stored.utc_offset_seconds) {
        (Some(unix_micros), None, None) => ClockReading::utc(unix_micros),
        (None, Some(local), Some(seconds)) => {
            UtcOffset::from_seconds(seconds).and_then(|offset| ClockReading::zoned(local, offset))
        }
        (None, Some(local), None) => Ok(ClockReading::LocalUnknownZone { local }),
        _ => return Err(SessionStorageError::IntegrityFailure),
    }
    .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let media_time = MediaTime::from_micros(stored.media_time_us);
    match (source.is_on_screen(), &stored.clock, stored.precision_us) {
        (false, None, None) => Ok(TimeAnchor::new(source, media_time, reading)),
        (true, Some(clock), Some(precision_micros)) => {
            let clock = ClockId::parse(clock).ok_or(SessionStorageError::IntegrityFailure)?;
            Ok(TimeAnchor::on_screen(
                source,
                media_time,
                reading,
                ScreenAnchor {
                    clock,
                    precision_micros,
                },
            ))
        }
        _ => Err(SessionStorageError::IntegrityFailure),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    type TestResult = Result<(), Box<dyn Error>>;

    fn anchors() -> Result<TimeAnchors, Box<dyn Error>> {
        let start = MediaTime::from_micros(0);
        Ok(TimeAnchors::new(vec![
            TimeAnchor::new(
                AnchorSource::Operator,
                start,
                ClockReading::parse("2026-10-21T14:03:12.25+01:00")?,
            ),
            TimeAnchor::new(
                AnchorSource::QuicktimeCreationDate,
                start,
                ClockReading::parse("2026-10-21T14:03:12")?,
            ),
            TimeAnchor::new(
                AnchorSource::ContainerCreationTime,
                start,
                ClockReading::parse("2026-10-21T13:03:13Z")?,
            ),
        ])?)
    }

    #[test]
    fn every_kind_of_reading_round_trips() -> TestResult {
        let anchors = anchors()?;
        let stored = encode_time_anchors(&anchors);
        let json = serde_json::to_string(&stored)?;
        assert_eq!(
            json,
            r#"[{"source":"operator","media_time_us":0,"local":"2026-10-21T14:03:12.250000","utc_offset_seconds":3600},{"source":"quicktime_creationdate","media_time_us":0,"local":"2026-10-21T14:03:12.000000"},{"source":"container_creation_time","media_time_us":0,"utc_unix_us":1792587793000000}]"#
        );
        let read: Vec<StoredTimeAnchor> = serde_json::from_str(&json)?;
        assert_eq!(decode_time_anchors(&read)?, anchors);
        Ok(())
    }

    fn screen_anchors() -> Result<TimeAnchors, Box<dyn Error>> {
        Ok(anchors()?.with_screen_anchors(vec![TimeAnchor::on_screen(
            AnchorSource::ScreenClock,
            MediaTime::from_micros(30_000_000),
            ClockReading::parse("2026-10-21T14:03:42")?,
            ScreenAnchor {
                clock: ClockId::new(2).ok_or("no clock")?,
                precision_micros: 500_000,
            },
        )]))
    }

    fn registered() -> Result<Vec<RegisteredClock>, Box<dyn Error>> {
        Ok(vec![RegisteredClock {
            id: ClockId::new(1).ok_or("no clock")?,
            format: ClockFormat::TwelveHour,
            unit: ClockUnit::Minute,
            offset: OffsetInterval::new(-90, 60).ok_or("no interval")?,
            confirmed_offset: Some(UtcOffset::from_seconds(-18_000)?),
        }])
    }

    #[test]
    fn a_screen_anchor_round_trips_and_a_tag_stays_as_it_was_written() -> TestResult {
        let anchors = screen_anchors()?;
        let json = serde_json::to_string(&encode_time_anchors(&anchors))?;
        assert!(
            json.contains(r#""clock":"c2","precision_us":500000"#),
            "{json}"
        );
        let read: Vec<StoredTimeAnchor> = serde_json::from_str(&json)?;
        assert_eq!(decode_time_anchors(&read)?, anchors);
        Ok(())
    }

    #[test]
    fn a_modified_screen_anchor_is_an_integrity_failure() -> TestResult {
        let json = serde_json::to_string(&encode_time_anchors(&screen_anchors()?))?;
        for (from, to) in [
            (
                r#""clock":"c2","precision_us":500000"#,
                r#""precision_us":500000"#,
            ),
            (r#","precision_us":500000"#, ""),
            (r#""clock":"c2""#, r#""clock":"c9""#),
            (
                r#""source":"operator","media_time_us":0"#,
                r#""source":"operator","media_time_us":0,"clock":"c1","precision_us":1"#,
            ),
        ] {
            let changed = json.replace(from, to);
            assert_ne!(changed, json, "{from}");
            let read: Vec<StoredTimeAnchor> = serde_json::from_str(&changed)?;
            assert_eq!(
                decode_time_anchors(&read),
                Err(SessionStorageError::IntegrityFailure),
                "{to}"
            );
        }
        Ok(())
    }

    #[test]
    fn the_register_round_trips() -> TestResult {
        let clocks = registered()?;
        let json = serde_json::to_string(&encode_screen_clocks(&clocks))?;
        let read: Vec<StoredScreenClock> = serde_json::from_str(&json)?;
        assert_eq!(decode_screen_clocks(&read)?, clocks);
        Ok(())
    }

    #[test]
    fn a_modified_register_is_an_integrity_failure() -> TestResult {
        let json = serde_json::to_string(&encode_screen_clocks(&registered()?))?;
        for (from, to) in [
            (r#""id":"c1""#, r#""id":"c0""#),
            (r#""format":"12_hour""#, r#""format":"sundial""#),
            (r#""unit":"minute""#, r#""unit":"hour""#),
            (r#""offset_low_us":-90"#, r#""offset_low_us":60"#),
            (
                r#""confirmed_offset_seconds":-18000"#,
                r#""confirmed_offset_seconds":-18001"#,
            ),
        ] {
            let changed = json.replace(from, to);
            assert_ne!(changed, json, "{from}");
            let read: Vec<StoredScreenClock> = serde_json::from_str(&changed)?;
            assert_eq!(
                decode_screen_clocks(&read),
                Err(SessionStorageError::IntegrityFailure),
                "{to}"
            );
        }
        let one = serde_json::to_string(&encode_screen_clocks(&registered()?))?;
        let repeated = format!("[{0},{0}]", one.trim_matches(['[', ']']));
        let read: Vec<StoredScreenClock> = serde_json::from_str(&repeated)?;
        assert_eq!(
            decode_screen_clocks(&read),
            Err(SessionStorageError::IntegrityFailure)
        );
        Ok(())
    }

    #[test]
    fn a_modified_record_is_an_integrity_failure() -> TestResult {
        let json = serde_json::to_string(&encode_time_anchors(&anchors()?))?;
        for (from, to) in [
            (r#""source":"operator""#, r#""source":"file_modified""#),
            (
                r#""local":"2026-10-21T14:03:12.250000""#,
                r#""local":"2026-10-21T14:03:12.25""#,
            ),
            (
                r#""local":"2026-10-21T14:03:12.000000""#,
                r#""local":"2026-10-21T14:03:12Z""#,
            ),
            (
                r#""utc_offset_seconds":3600"#,
                r#""utc_offset_seconds":3601"#,
            ),
            (
                r#""utc_unix_us":1792587793000000"#,
                r#""utc_unix_us":1792587793000000,"local":"2026-10-21T13:03:13.000000""#,
            ),
            (
                r#""source":"container_creation_time""#,
                r#""source":"operator""#,
            ),
        ] {
            let changed = json.replace(from, to);
            assert_ne!(changed, json, "{from}");
            let read: Vec<StoredTimeAnchor> = serde_json::from_str(&changed)?;
            assert_eq!(
                decode_time_anchors(&read),
                Err(SessionStorageError::IntegrityFailure),
                "{to}"
            );
        }
        let extra = json.replacen(
            r#""media_time_us":0"#,
            r#""media_time_us":0,"drift_us":1"#,
            1,
        );
        assert!(serde_json::from_str::<Vec<StoredTimeAnchor>>(&extra).is_err());
        Ok(())
    }
}
