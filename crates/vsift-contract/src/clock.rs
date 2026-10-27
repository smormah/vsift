//! Time anchoring on the wire (ADR 0029): the session's `time_anchor` summary
//! and the `clock_time` member of every timestamped item.
//!
//! Both are additive and appear only when the session has an anchor, so a
//! session without one answers byte for byte as before. A clock reading is
//! always the same three members, `utc`, `local` and `utc_offset`, each a
//! string or `null`: `utc` is RFC 3339 in UTC with six fraction digits and
//! `Z`, present when the reading names an instant; `local` is the local
//! clock's date and time with six fraction digits and no zone, present when
//! the source said what the local clock showed; `utc_offset` is `+HH:MM` or
//! `-HH:MM`, present only when both are. A local time with no offset is a
//! local time whose zone is not known: it names no instant.
//!
//! The summary lists every anchor, so a later source (stage 2: clocks visible
//! on screen, at other positions of the timeline) adds entries and the
//! `consistency` verdict covers them, without any member changing meaning.

use serde::Serialize;
use vsift_domain::{CivilDateTime, ClockReading, MediaTime, TimeAnchors, TimeRange};

/// One clock reading on the wire.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ClockData {
    utc: Option<String>,
    local: Option<String>,
    utc_offset: Option<String>,
}

impl ClockData {
    /// Presents `reading`; `None` when its instant cannot be written as a
    /// year from 1 to 9999.
    #[must_use]
    pub fn new(reading: ClockReading) -> Option<Self> {
        let utc = match reading.utc_unix_micros() {
            Some(micros) => Some(format!("{}Z", CivilDateTime::from_unix_micros(micros)?)),
            None => None,
        };
        Some(Self {
            utc,
            local: reading.local().map(|local| local.to_string()),
            utc_offset: reading.offset().map(|offset| offset.to_string()),
        })
    }
}

/// The `clock_time` member of a timestamped item: the real-world clock time
/// of the item's position on the timeline, from the session's selected
/// anchor. `at` is a point's time (a frame, a candidate's representative
/// frame, an audio clip's first sample); `start` and `end` are a range's
/// (a transcript segment, a candidate's span). Members that do not apply are
/// absent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ClockTimeData {
    source: &'static str,
    confidence: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    at: Option<ClockData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start: Option<ClockData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end: Option<ClockData>,
}

impl ClockTimeData {
    /// The clock time of one point; `None` without an anchor.
    #[must_use]
    pub fn point(anchors: &TimeAnchors, at: MediaTime) -> Option<Self> {
        let clock = anchors.clock_time_at(at)?;
        Some(Self {
            source: clock.source.identifier(),
            confidence: clock.confidence.identifier(),
            at: Some(ClockData::new(clock.reading)?),
            start: None,
            end: None,
        })
    }

    /// The clock times of a range's ends; `None` without an anchor.
    #[must_use]
    pub fn range(anchors: &TimeAnchors, range: TimeRange) -> Option<Self> {
        let start = anchors.clock_time_at(range.start())?;
        let end = anchors.clock_time_at(range.end())?;
        Some(Self {
            source: start.source.identifier(),
            confidence: start.confidence.identifier(),
            at: None,
            start: Some(ClockData::new(start.reading)?),
            end: Some(ClockData::new(end.reading)?),
        })
    }

    /// The clock times of a range with a point inside it (a candidate's span
    /// and its representative frame); `None` without an anchor.
    #[must_use]
    pub fn range_with_point(
        anchors: &TimeAnchors,
        range: TimeRange,
        at: MediaTime,
    ) -> Option<Self> {
        let mut data = Self::range(anchors, range)?;
        data.at = Some(ClockData::new(anchors.clock_time_at(at)?.reading)?);
        Some(data)
    }
}

/// One anchor in the session's `time_anchor.anchors` list.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct AnchorData {
    source: &'static str,
    confidence: &'static str,
    media_time_us: u64,
    time: ClockData,
}

/// The session's `time_anchor` summary in `ingest` and `session status`
/// results: whether the anchors agree, the one clock times come from, the
/// recording's start by it, and every anchor in ladder order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TimeAnchorData {
    consistency: &'static str,
    source: &'static str,
    confidence: &'static str,
    recording_start: ClockData,
    anchors: Vec<AnchorData>,
}

impl TimeAnchorData {
    /// Presents a session's anchors; `None` when it has none.
    #[must_use]
    pub fn new(anchors: &TimeAnchors) -> Option<Self> {
        let consistency = anchors.consistency()?;
        let start = anchors.clock_time_at(MediaTime::from_micros(0))?;
        Some(Self {
            consistency: consistency.identifier(),
            source: start.source.identifier(),
            confidence: start.confidence.identifier(),
            recording_start: ClockData::new(start.reading)?,
            anchors: anchors
                .anchors()
                .iter()
                .map(|anchor| {
                    Some(AnchorData {
                        source: anchor.source().identifier(),
                        confidence: anchor.confidence().identifier(),
                        media_time_us: anchor.media_time().as_micros(),
                        time: ClockData::new(anchor.reading())?,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::json;
    use vsift_domain::{AnchorSource, TimeAnchor};

    use super::*;

    type TestResult = Result<(), Box<dyn Error>>;

    fn anchors(stated: &str, tags: &[(AnchorSource, &str)]) -> Result<TimeAnchors, Box<dyn Error>> {
        let start = MediaTime::from_micros(0);
        let mut list = vec![TimeAnchor::new(
            AnchorSource::Operator,
            start,
            ClockReading::parse(stated)?,
        )];
        for (source, text) in tags {
            list.push(TimeAnchor::new(*source, start, ClockReading::parse(text)?));
        }
        Ok(TimeAnchors::new(list)?)
    }

    #[test]
    fn a_zoned_anchor_gives_utc_local_and_offset() -> TestResult {
        let anchors = anchors(
            "2026-10-21T14:03:12+01:00",
            &[(AnchorSource::ContainerCreationTime, "2026-10-21T13:03:13Z")],
        )?;
        let range = TimeRange::new(
            MediaTime::from_micros(1_500_000),
            MediaTime::from_micros(4_250_000),
        )?;
        assert_eq!(
            serde_json::to_value(ClockTimeData::range(&anchors, range))?,
            json!({
                "source": "operator",
                "confidence": "exact",
                "start": {"utc": "2026-10-21T13:03:13.500000Z", "local": "2026-10-21T14:03:13.500000", "utc_offset": "+01:00"},
                "end": {"utc": "2026-10-21T13:03:16.250000Z", "local": "2026-10-21T14:03:16.250000", "utc_offset": "+01:00"}
            })
        );
        assert_eq!(
            serde_json::to_value(TimeAnchorData::new(&anchors))?,
            json!({
                "consistency": "consistent",
                "source": "operator",
                "confidence": "exact",
                "recording_start": {"utc": "2026-10-21T13:03:12.000000Z", "local": "2026-10-21T14:03:12.000000", "utc_offset": "+01:00"},
                "anchors": [
                    {"source": "operator", "confidence": "exact", "media_time_us": 0,
                     "time": {"utc": "2026-10-21T13:03:12.000000Z", "local": "2026-10-21T14:03:12.000000", "utc_offset": "+01:00"}},
                    {"source": "container_creation_time", "confidence": "medium", "media_time_us": 0,
                     "time": {"utc": "2026-10-21T13:03:13.000000Z", "local": null, "utc_offset": null}}
                ]
            })
        );
        Ok(())
    }

    #[test]
    fn a_zoneless_local_time_names_no_instant() -> TestResult {
        let anchors = anchors("2026-10-21T14:03:12", &[])?;
        assert_eq!(
            serde_json::to_value(ClockTimeData::point(
                &anchors,
                MediaTime::from_micros(60_000_000)
            ))?,
            json!({
                "source": "operator",
                "confidence": "exact",
                "at": {"utc": null, "local": "2026-10-21T14:04:12.000000", "utc_offset": null}
            })
        );
        Ok(())
    }

    #[test]
    fn no_anchor_presents_nothing() {
        let none = TimeAnchors::none();
        assert_eq!(TimeAnchorData::new(&none), None);
        assert_eq!(ClockTimeData::point(&none, MediaTime::from_micros(0)), None);
    }
}
