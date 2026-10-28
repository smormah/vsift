//! The stored form of a session's time anchors (time anchoring, ADR 0029).
//!
//! Anchors are written once, at activation, as an optional member of the
//! session's lifecycle record, and copied unchanged into every later
//! generation. A session without anchors writes nothing, so its record is
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
use vsift_domain::{AnchorSource, ClockReading, MediaTime, TimeAnchor, TimeAnchors, UtcOffset};

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
    Ok(TimeAnchor::new(
        source,
        MediaTime::from_micros(stored.media_time_us),
        reading,
    ))
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
