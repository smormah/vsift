//! Reads a retained bundle's records so the grader can resolve citations.
//!
//! The bundle is first validated with `vsift bundle validate` (the CLI's own
//! strict decoding); this module then reads the validated transcript and
//! evidence records to look up identities and times. It never trusts a
//! handoff's copy of a value: a citation must name an identity the bundle
//! holds with the citation's type, every other member the handoff gives is
//! compared with the record `VSift` wrote, and a member it leaves out is
//! taken from that record.

use std::{collections::BTreeMap, path::Path};

use serde_json::Value;

use crate::error::{TrialError, read_json};

/// One transcript segment of a retained revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Segment {
    /// Start, microseconds.
    pub start_us: u64,
    /// End, microseconds.
    pub end_us: u64,
    /// Sanitized text.
    pub text: String,
}

/// One frame selection of an evidence record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Selection {
    /// The requested time.
    pub requested_us: u64,
    /// The frame's time.
    pub actual_us: u64,
    /// `actual - requested`.
    pub delta_us: i64,
    /// The visual candidate the request named, if it named one.
    pub candidate_id: Option<String>,
}

/// One crop item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Crop {
    /// The parent frame or crop.
    pub parent_evidence_id: String,
    /// The rectangle in the parent's pixels.
    pub rect: [u64; 4],
    /// The frame's time.
    pub actual_us: u64,
}

/// One audio item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Clip {
    /// The clipped range's start.
    pub start_us: u64,
    /// The clipped range's end.
    pub end_us: u64,
    /// The first decoded sample.
    pub actual_start_us: u64,
}

/// Everything a bundle lets the grader resolve.
#[derive(Clone, Debug, Default)]
pub struct BundleIndex {
    /// The retained session.
    pub session_id: Option<String>,
    /// Segments by `(revision_id, segment_id)`.
    pub segments: BTreeMap<(String, String), Segment>,
    /// Frame selections by evidence identity (one identity may have several).
    pub selections: BTreeMap<String, Vec<Selection>>,
    /// Frame times by evidence identity.
    pub frames: BTreeMap<String, u64>,
    /// Crops by evidence identity.
    pub crops: BTreeMap<String, Crop>,
    /// Clips by evidence identity.
    pub clips: BTreeMap<String, Clip>,
    /// The source's duration as `VSift` indexed it (the visual index
    /// record's `duration_us`), when the bundle holds an index. The grader
    /// derives a looped clip's real period from it (issue #219).
    pub source_duration_us: Option<u64>,
}

fn unsigned(value: &Value) -> u64 {
    value.as_u64().unwrap_or_default()
}

impl BundleIndex {
    /// Reads every transcript and evidence record of a validated bundle.
    ///
    /// # Errors
    ///
    /// [`TrialError`] when `bundle.json` or a listed record cannot be read.
    pub fn read(bundle: &Path) -> Result<Self, TrialError> {
        let manifest = read_json(&bundle.join("bundle.json"))?;
        let mut index = Self {
            session_id: manifest["session_id"].as_str().map(str::to_owned),
            ..Self::default()
        };
        for artifact in manifest["artifacts"].as_array().into_iter().flatten() {
            let name = artifact["name"].as_str().unwrap_or_default();
            if name.contains(['/', '\\']) || !name.starts_with("artifact-") {
                return Err(TrialError::Invalid(format!(
                    "the bundle lists an unexpected artifact name {name:?}"
                )));
            }
            match artifact["kind"].as_str() {
                Some("transcript_record") => index.add_transcript(&read_json(&bundle.join(name))?),
                Some("evidence_record") => index.add_evidence(&read_json(&bundle.join(name))?),
                Some("visual_index_record") => {
                    let record = read_json(&bundle.join(name))?;
                    index.source_duration_us = record["duration_us"].as_u64();
                }
                _ => {}
            }
        }
        Ok(index)
    }

    fn add_transcript(&mut self, record: &Value) {
        let revision = record["revision_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        for segment in record["segments"].as_array().into_iter().flatten() {
            self.segments.insert(
                (
                    revision.clone(),
                    segment["id"].as_str().unwrap_or_default().to_owned(),
                ),
                Segment {
                    start_us: unsigned(&segment["start_us"]),
                    end_us: unsigned(&segment["end_us"]),
                    text: segment["text"].as_str().unwrap_or_default().to_owned(),
                },
            );
        }
    }

    fn add_evidence(&mut self, record: &Value) {
        let candidate_id = record["request"]["frame_get"]["candidate_id"]
            .as_str()
            .map(str::to_owned);
        for selection in record["selections"].as_array().into_iter().flatten() {
            let id = selection["evidence_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            self.selections.entry(id).or_default().push(Selection {
                requested_us: unsigned(&selection["requested_us"]),
                actual_us: unsigned(&selection["actual_us"]),
                delta_us: selection["delta_us"].as_i64().unwrap_or_default(),
                candidate_id: candidate_id.clone(),
            });
        }
        for item in record["items"].as_array().into_iter().flatten() {
            let id = item["evidence_id"].as_str().unwrap_or_default().to_owned();
            let subject = &item["subject"];
            if subject["frame"].is_object() {
                self.frames
                    .insert(id, unsigned(&subject["frame"]["time_us"]));
            } else if subject["crop"].is_object() {
                let region = &subject["crop"]["region"];
                self.crops.insert(
                    id,
                    Crop {
                        parent_evidence_id: region["parent_evidence_id"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                        rect: [
                            unsigned(&region["x"]),
                            unsigned(&region["y"]),
                            unsigned(&region["width"]),
                            unsigned(&region["height"]),
                        ],
                        actual_us: unsigned(&subject["crop"]["frame"]["time_us"]),
                    },
                );
            } else if subject["audio"].is_object() {
                let audio = &subject["audio"];
                self.clips.insert(
                    id,
                    Clip {
                        start_us: unsigned(&audio["start_us"]),
                        end_us: unsigned(&audio["end_us"]),
                        actual_start_us: unsigned(&audio["actual_start_us"]),
                    },
                );
            }
        }
    }
}

/// Where a resolved citation lies and what it holds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Resolved {
    /// A transcript segment and its text.
    Transcript {
        /// The revision the segment belongs to.
        revision_id: String,
        /// The segment's range.
        start_us: u64,
        /// The segment's end.
        end_us: u64,
        /// Its sanitized text.
        text: String,
    },
    /// A frame or crop at a time.
    Visual {
        /// The frame's time.
        actual_us: u64,
        /// Whether the model says it looked at the pixels.
        pixels_inspected: bool,
    },
    /// An audio clip.
    Audio {
        /// Its range start.
        start_us: u64,
        /// Its range end.
        end_us: u64,
    },
}

/// A citation member the handoff may leave out (handoff v1 as revised on
/// 2026-09-29): `None` when it is absent or `null`, so it is not checked;
/// otherwise its value, which must equal the record. A member of the wrong
/// JSON type is given but never equal, so it fails.
fn given(citation: &Value, key: &str) -> Option<Value> {
    match &citation[key] {
        Value::Null => None,
        value => Some(value.clone()),
    }
}

/// Whether an optional member is absent or equals `recorded`.
fn agrees(citation: &Value, key: &str, recorded: &Value) -> bool {
    given(citation, key).is_none_or(|value| &value == recorded)
}

impl BundleIndex {
    /// Resolves one handoff citation through its `VSift` identity. Every
    /// optional member the handoff gives (times, revision, candidate,
    /// parent, rectangle, range) is compared with the record; a missing one
    /// is taken from the record, so later checks see `VSift`'s value.
    ///
    /// # Errors
    ///
    /// Why the citation does not resolve: the identity is not in the bundle
    /// with the citation's type, or a given member differs from the record.
    pub fn resolve(&self, citation: &Value) -> Result<Resolved, String> {
        let text = |key: &str| citation[key].as_str().unwrap_or_default().to_owned();
        let id = citation["id"].as_str().unwrap_or("?");
        match citation["type"].as_str() {
            Some("transcript_segment") => self.resolve_segment(id, citation),
            Some("frame") => {
                let evidence = text("evidence_id");
                let selections = self
                    .selections
                    .get(&evidence)
                    .ok_or_else(|| format!("{id}: the frame is not in the bundle"))?;
                let candidate = citation["candidate_id"].as_str().map(str::to_owned);
                let selection = selections
                    .iter()
                    .find(|selection| {
                        agrees(citation, "requested_us", &Value::from(selection.requested_us))
                            && agrees(citation, "actual_us", &Value::from(selection.actual_us))
                            && agrees(citation, "delta_us", &Value::from(selection.delta_us))
                            && (candidate.is_none() || candidate == selection.candidate_id)
                    })
                    .ok_or_else(|| {
                        format!(
                            "{id}: requested, actual, delta or candidate differ from every selection of the frame"
                        )
                    })?;
                Ok(Resolved::Visual {
                    actual_us: selection.actual_us,
                    pixels_inspected: citation["pixels_inspected"] == true,
                })
            }
            Some("crop") => {
                let crop = self
                    .crops
                    .get(&text("evidence_id"))
                    .ok_or_else(|| format!("{id}: the crop is not in the bundle"))?;
                let [x, y, width, height] = crop.rect;
                let rect = serde_json::json!({"x": x, "y": y, "width": width, "height": height});
                let rect_agrees = given(citation, "rect").is_none_or(|cited| {
                    ["x", "y", "width", "height"]
                        .iter()
                        .all(|key| cited[key] == rect[key])
                });
                if !agrees(
                    citation,
                    "parent_evidence_id",
                    &Value::from(crop.parent_evidence_id.as_str()),
                ) || !rect_agrees
                    || !agrees(citation, "actual_us", &Value::from(crop.actual_us))
                {
                    return Err(format!("{id}: the crop's parent, rectangle or time differ"));
                }
                Ok(Resolved::Visual {
                    actual_us: crop.actual_us,
                    pixels_inspected: citation["pixels_inspected"] == true,
                })
            }
            Some("audio") => {
                let clip = self
                    .clips
                    .get(&text("evidence_id"))
                    .ok_or_else(|| format!("{id}: the clip is not in the bundle"))?;
                let range_agrees = given(citation, "range").is_none_or(|range| {
                    range["start_us"].as_u64() == Some(clip.start_us)
                        && range["end_us"].as_u64() == Some(clip.end_us)
                });
                if !range_agrees
                    || !agrees(
                        citation,
                        "actual_start_us",
                        &Value::from(clip.actual_start_us),
                    )
                {
                    return Err(format!("{id}: the clip's range or first sample differ"));
                }
                Ok(Resolved::Audio {
                    start_us: clip.start_us,
                    end_us: clip.end_us,
                })
            }
            _ => Err(format!("{id}: unknown citation type")),
        }
    }

    /// A transcript segment by its identity. Segment identities are derived
    /// from their revision, so one names one segment; the revision, when
    /// given, must be that segment's, and the times must be its times.
    fn resolve_segment(&self, id: &str, citation: &Value) -> Result<Resolved, String> {
        let segment_id = citation["segment_id"].as_str().unwrap_or_default();
        let matches: Vec<(&String, &Segment)> = self
            .segments
            .iter()
            .filter(|((_, segment), _)| segment == segment_id)
            .map(|((revision, _), segment)| (revision, segment))
            .collect();
        let (revision, segment) = match matches.as_slice() {
            [] => return Err(format!("{id}: the segment is not in the bundle")),
            [only] => *only,
            _ => {
                let cited = citation["revision_id"].as_str().ok_or_else(|| {
                    format!("{id}: the segment is in several revisions; name its revision_id")
                })?;
                matches
                    .iter()
                    .find(|(revision, _)| revision.as_str() == cited)
                    .copied()
                    .ok_or_else(|| format!("{id}: the segment is not in the bundle"))?
            }
        };
        if !agrees(citation, "revision_id", &Value::from(revision.as_str())) {
            return Err(format!("{id}: the segment is not in the cited revision"));
        }
        if !agrees(citation, "start_us", &Value::from(segment.start_us))
            || !agrees(citation, "end_us", &Value::from(segment.end_us))
        {
            return Err(format!("{id}: the segment's times differ from the record"));
        }
        Ok(Resolved::Transcript {
            revision_id: revision.clone(),
            start_us: segment.start_us,
            end_us: segment.end_us,
            text: segment.text.clone(),
        })
    }
}
