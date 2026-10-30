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
//! The summary lists every anchor, so clocks on the screen, at other positions
//! of the timeline, add entries and the `consistency` verdict covers them,
//! without any member changing meaning. A gap a clock showed divides the
//! timeline into `segments`, and each item after it says which segment it is
//! in. A detected gap changes no value: only a person accepts one (ADR 0029
//! decision 13).

use serde::Serialize;
use vsift_domain::{CivilDateTime, ClockReading, ClockTime, MediaTime, TimeAnchors, TimeRange};

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
    clock_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    segment: Option<usize>,
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
            at: Some(ClockData::new(clock.reading)?),
            ..Self::of(&clock)
        })
    }

    fn of(clock: &ClockTime) -> Self {
        Self {
            source: clock.source.identifier(),
            confidence: clock.confidence.identifier(),
            clock_id: clock.clock.map(|id| id.to_string()),
            segment: clock.segment,
            at: None,
            start: None,
            end: None,
        }
    }

    /// The clock times of a range's ends; `None` without an anchor.
    #[must_use]
    pub fn range(anchors: &TimeAnchors, range: TimeRange) -> Option<Self> {
        let start = anchors.clock_time_at(range.start())?;
        let end = anchors.clock_time_at(range.end())?;
        Some(Self {
            start: Some(ClockData::new(start.reading)?),
            end: Some(ClockData::new(end.reading)?),
            ..Self::of(&start)
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
    #[serde(skip_serializing_if = "Option::is_none")]
    clock_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    precision_us: Option<u64>,
}

/// One stretch of the timeline between gaps a clock showed. The first has none
/// of the gap's members.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SegmentData {
    segment: usize,
    from_us: u64,
    to_us: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gap_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clock_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    jump_us: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uncertainty_us: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    adjusted_by_us: Option<i64>,
}

fn segments(anchors: &TimeAnchors) -> Vec<SegmentData> {
    let gaps = anchors.gaps();
    if gaps.is_empty() {
        return Vec::new();
    }
    let mut list = vec![SegmentData {
        segment: 0,
        from_us: 0,
        to_us: gaps.first().map(|gap| gap.at.as_micros()),
        gap_id: None,
        state: None,
        clock_id: None,
        jump_us: None,
        uncertainty_us: None,
        adjusted_by_us: None,
    }];
    let mut adjusted = 0;
    for (index, gap) in gaps.iter().enumerate() {
        adjusted += gap.adjustment_micros();
        list.push(SegmentData {
            segment: index + 1,
            from_us: gap.at.as_micros(),
            to_us: gaps.get(index + 1).map(|next| next.at.as_micros()),
            gap_id: Some(gap.id.to_string()),
            state: Some(gap.state.identifier()),
            clock_id: Some(gap.clock.to_string()),
            jump_us: Some(gap.jump_micros()),
            uncertainty_us: Some(gap.uncertainty_micros()),
            adjusted_by_us: Some(adjusted),
        });
    }
    list
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    segments: Vec<SegmentData>,
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
                        clock_id: anchor.screen().map(|screen| screen.clock.to_string()),
                        precision_us: anchor.screen().map(|screen| screen.precision_micros),
                    })
                })
                .collect::<Option<Vec<_>>>()?,
            segments: segments(anchors),
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
    fn a_detected_gap_marks_the_item_and_changes_no_value() -> TestResult {
        let plain = anchors("2026-10-21T14:00:00Z", &[])?;
        let gap = vsift_domain::ClockGap {
            id: vsift_domain::GapId::new(1).ok_or("no gap")?,
            clock: vsift_domain::ClockId::new(2).ok_or("no clock")?,
            at: MediaTime::from_micros(300_000_000),
            jump_low_micros: 430_000_000,
            jump_high_micros: 440_000_000,
            state: vsift_domain::GapState::Detected,
        };
        let marked = plain.with_gaps(vec![gap]);
        let after = MediaTime::from_micros(400_000_000);
        assert_eq!(
            serde_json::to_value(ClockTimeData::point(&marked, after))?,
            json!({
                "source": "operator",
                "confidence": "exact",
                "segment": 1,
                "at": {"utc": "2026-10-21T14:06:40.000000Z", "local": null, "utc_offset": null}
            })
        );
        assert_eq!(
            serde_json::to_value(TimeAnchorData::new(&marked))?["segments"],
            json!([
                {"segment": 0, "from_us": 0, "to_us": 300_000_000},
                {"segment": 1, "from_us": 300_000_000, "to_us": null, "gap_id": "g1",
                 "state": "detected", "clock_id": "c2", "jump_us": 435_000_000,
                 "uncertainty_us": 5_000_000, "adjusted_by_us": 0}
            ])
        );
        let accepted = plain.with_gaps(vec![vsift_domain::ClockGap {
            state: vsift_domain::GapState::Accepted,
            ..gap
        }]);
        assert_eq!(
            serde_json::to_value(ClockTimeData::point(&accepted, after))?["at"]["utc"],
            json!("2026-10-21T14:13:55.000000Z")
        );
        Ok(())
    }

    #[test]
    fn a_session_with_no_gap_and_no_screen_anchor_adds_no_member() -> TestResult {
        let anchors = anchors("2026-10-21T14:03:12Z", &[])?;
        let summary = serde_json::to_value(TimeAnchorData::new(&anchors))?;
        assert_eq!(summary.get("segments"), None);
        assert_eq!(summary["anchors"][0].get("clock_id"), None);
        assert_eq!(summary["anchors"][0].get("precision_us"), None);
        let point =
            serde_json::to_value(ClockTimeData::point(&anchors, MediaTime::from_micros(0)))?;
        assert_eq!(point.get("segment"), None);
        assert_eq!(point.get("clock_id"), None);
        Ok(())
    }

    #[test]
    fn no_anchor_presents_nothing() {
        let none = TimeAnchors::none();
        assert_eq!(TimeAnchorData::new(&none), None);
        assert_eq!(ClockTimeData::point(&none, MediaTime::from_micros(0)), None);
    }
}
