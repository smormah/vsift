//! V-02..V-05 candidate recall scoring for the visual index (P08). Test-only.
//!
//! Expectations come only from the frozen corpus truth in
//! `fixtures/corpus/manifest.json`; nothing is derived from the index being
//! scored. The inputs are recorded samples: per fixture, the time, block
//! means and hash of every sample real `FFmpeg` decoded through
//! `FfmpegMedia::visual_samples` (see `tests/data/visual_samples/`), so the
//! always-run gate needs no media tool and the opt-in live test can compare
//! a fresh decode with them.
//!
//! Scoring, for every visual event of the recorded fixtures (all kinds except
//! `speech` and `malformed`):
//!
//! - **hit**: some candidate's representative time lies inside the event's
//!   `[start, end)` window;
//! - **timestamp error**: the first such candidate's representative time
//!   minus the event start;
//! - **change window brackets the start**: that candidate is exactly at the
//!   start, or its recorded change happened after its previous sample and at
//!   or before it with the event start in between;
//! - **false change candidates**: change candidates (`visual_change`,
//!   `motion_start`, `settled_after_motion`) whose change interval contains
//!   no boundary (start or end) of any visual event of the fixture;
//! - recall by event kind, transients separately, and candidates per minute.
//!
//! The gate (decided for P08, pending ADR 0018 confirmation) requires every
//! `stable` event of at least one second to be hit and no false change in the
//! static fixtures F01 and F07. Other events are reported, not gated.
//! A corpus limitation is an event whose generated pixels are identical to
//! the state before it (see `tools/generate_p04_fixtures.py` `scene()`); the
//! truth is never changed, and each limitation is re-verified from the
//! recorded samples on every run.

// Each test binary that includes this module uses a different part of it.
#![allow(dead_code)]

use std::{error::Error, fmt::Write as _, fs, future::Future, path::PathBuf};

use serde_json::{Value, json};
use vsift_application::{
    ExtendVisualIndexRequest, VisualIndexScope, VisualSampler, VisualSamplingError,
    extend_visual_index,
};
use vsift_domain::{
    CandidateReason, FrameDimensions, MediaTime, SessionId, SourceId, TimeRange, VISUAL_BLOCKS,
    VisualCandidate, VisualHash, VisualIndex, VisualIndexProfile, VisualSample, VisualWindow,
};

pub type Built<T> = Result<T, Box<dyn Error>>;

/// Fixtures with visual truth and generated media.
pub const RECORDED_FIXTURES: [&str; 11] = [
    "F01", "F02", "F03", "F04", "F05", "F06", "F07", "F08", "F09", "F10", "F12",
];

/// Static fixtures: any change candidate in them is false.
pub const STATIC_FIXTURES: [&str; 2] = ["F01", "F07"];

/// Reviewed corpus limitations: events drawn with exactly the pixels of the
/// state before them, so no visual method can see them begin.
pub const CORPUS_LIMITATIONS: [(&str, &str); 3] = [
    (
        "F04-E02",
        "the scroll is drawn with the pixels of F04-E01 (row 1001 queued); no scrolling is rendered",
    ),
    (
        "F05-E02",
        "the loading indicator is not drawn; the screen keeps the pixels of F05-E01",
    ),
    (
        "F12-E02",
        "drawn with the pixels of F12-E01; the event is hit only by periodic coverage",
    ),
];

/// Session identity the scored indexes are built in.
pub const SCORING_SESSION: &str = "ses_0000000000000000visual";

pub fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

pub fn manifest() -> Built<Value> {
    Ok(serde_json::from_slice(&fs::read(repository(
        "fixtures/corpus/manifest.json",
    ))?)?)
}

pub fn recorded_path(fixture: &str) -> PathBuf {
    repository(&format!(
        "crates/vsift-infrastructure/tests/data/visual_samples/{fixture}.json"
    ))
}

/// One fixture's recorded samples.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedFixture {
    pub fixture: String,
    pub duration: MediaTime,
    pub source_sha256: String,
    pub windows: Vec<RecordedWindow>,
}

/// The samples one window's decode returned, lead-in included.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedWindow {
    pub ordinal: u32,
    pub samples: Vec<VisualSample>,
}

pub fn blocks_hex(sample: &VisualSample) -> String {
    let mut text = String::with_capacity(VISUAL_BLOCKS * 2);
    for block in sample.blocks() {
        let _ = write!(text, "{block:02x}");
    }
    text
}

fn parse_blocks(text: &str) -> Option<[u8; VISUAL_BLOCKS]> {
    if text.len() != VISUAL_BLOCKS * 2 || !text.is_ascii() {
        return None;
    }
    let mut blocks = [0_u8; VISUAL_BLOCKS];
    for (position, block) in blocks.iter_mut().enumerate() {
        *block = u8::from_str_radix(text.get(position * 2..position * 2 + 2)?, 16).ok()?;
    }
    Some(blocks)
}

/// Serializes recorded samples with their provenance, one sample per line
/// so a re-recording diffs sample by sample.
pub fn recorded_text(recorded: &RecordedFixture, provenance: &Value) -> Built<String> {
    let mut text = String::from("{\n");
    writeln!(
        text,
        "  \"fixture\": {},",
        serde_json::to_string(&recorded.fixture)?
    )?;
    writeln!(
        text,
        "  \"provenance\": {},",
        serde_json::to_string(provenance)?
    )?;
    writeln!(
        text,
        "  \"source_sha256\": {},",
        serde_json::to_string(&recorded.source_sha256)?
    )?;
    writeln!(
        text,
        "  \"duration_us\": {},",
        recorded.duration.as_micros()
    )?;
    text.push_str("  \"windows\": [\n");
    for (window_position, window) in recorded.windows.iter().enumerate() {
        writeln!(
            text,
            "    {{\"ordinal\": {}, \"samples\": [",
            window.ordinal
        )?;
        for (position, sample) in window.samples.iter().enumerate() {
            let line = json!({
                "time_us": sample.time().as_micros(),
                "hash": sample.hash().to_hex(),
                "blocks": blocks_hex(sample),
            });
            let separator = if position + 1 == window.samples.len() {
                ""
            } else {
                ","
            };
            writeln!(text, "      {}{separator}", serde_json::to_string(&line)?)?;
        }
        let separator = if window_position + 1 == recorded.windows.len() {
            ""
        } else {
            ","
        };
        writeln!(text, "    ]}}{separator}")?;
    }
    text.push_str("  ]\n}\n");
    Ok(text)
}

/// Loads one fixture's committed recorded samples.
pub fn load_recorded(fixture: &str) -> Built<RecordedFixture> {
    let value: Value = serde_json::from_slice(&fs::read(recorded_path(fixture))?)?;
    parse_recorded(&value)
}

pub fn parse_recorded(value: &Value) -> Built<RecordedFixture> {
    let mut windows = Vec::new();
    for window in value["windows"].as_array().ok_or("windows missing")? {
        let mut samples = Vec::new();
        for sample in window["samples"].as_array().ok_or("samples missing")? {
            samples.push(VisualSample::from_parts(
                MediaTime::from_micros(sample["time_us"].as_u64().ok_or("time missing")?),
                parse_blocks(sample["blocks"].as_str().ok_or("blocks missing")?)
                    .ok_or("blocks malformed")?,
                VisualHash::parse_hex(sample["hash"].as_str().ok_or("hash missing")?)?,
            ));
        }
        windows.push(RecordedWindow {
            ordinal: u32::try_from(window["ordinal"].as_u64().ok_or("ordinal missing")?)?,
            samples,
        });
    }
    Ok(RecordedFixture {
        fixture: value["fixture"]
            .as_str()
            .ok_or("fixture missing")?
            .to_owned(),
        duration: MediaTime::from_micros(value["duration_us"].as_u64().ok_or("duration missing")?),
        source_sha256: value["source_sha256"]
            .as_str()
            .ok_or("source digest missing")?
            .to_owned(),
        windows,
    })
}

/// Replays recorded samples through the visual-sampling port.
pub struct RecordedSampler<'a> {
    pub windows: &'a [RecordedWindow],
}

impl VisualSampler for RecordedSampler<'_> {
    fn window_samples(
        &self,
        window: VisualWindow,
    ) -> impl Future<Output = Result<Vec<VisualSample>, VisualSamplingError>> + Send {
        std::future::ready(
            self.windows
                .iter()
                .find(|recorded| recorded.ordinal == window.ordinal())
                .map(|recorded| recorded.samples.clone())
                .ok_or(VisualSamplingError::Undecodable),
        )
    }
}

/// Builds the whole index of a recorded fixture.
pub async fn index_recorded(recorded: &RecordedFixture) -> Built<VisualIndex> {
    let session = SessionId::parse(SCORING_SESSION)?;
    let source = SourceId::from_sha256(&recorded.source_sha256)?;
    let scope = VisualIndexScope {
        session_id: &session,
        source_id: &source,
        stream_index: 0,
        // Dimensions are carried for presentation only and never change
        // which candidates a window's samples produce.
        displayed_dimensions: FrameDimensions::new(1280, 720)?,
        duration: recorded.duration,
        profile: VisualIndexProfile::R0,
    };
    let range = TimeRange::new(MediaTime::from_micros(0), recorded.duration)?;
    let extension = extend_visual_index(
        ExtendVisualIndexRequest {
            scope,
            previous: None,
            range,
        },
        &RecordedSampler {
            windows: &recorded.windows,
        },
    )
    .await?;
    if extension.stop.is_some() {
        return Err("recorded fixture was not indexed completely".into());
    }
    Ok(extension.revision.ok_or("no revision")?)
}

/// A visual event of the manifest.
#[derive(Clone, Debug)]
pub struct VisualEvent {
    pub fixture: String,
    pub id: String,
    pub kind: String,
    pub start_us: u64,
    pub end_us: u64,
}

impl VisualEvent {
    pub const fn duration_us(&self) -> u64 {
        self.end_us - self.start_us
    }

    /// Every `stable` event of at least one second is gated, including a
    /// reviewed limitation that coverage still hits (F12-E02).
    pub fn is_gated(&self) -> bool {
        self.kind == "stable" && self.duration_us() >= 1_000_000
    }

    pub fn limitation(&self) -> Option<&'static str> {
        CORPUS_LIMITATIONS
            .iter()
            .find(|(id, _)| *id == self.id)
            .map(|(_, reason)| *reason)
    }
}

pub fn visual_events(manifest: &Value, fixture: &str) -> Built<Vec<VisualEvent>> {
    let entry = manifest["fixtures"]
        .as_array()
        .ok_or("fixtures missing")?
        .iter()
        .find(|entry| entry["id"] == fixture)
        .ok_or("fixture missing")?;
    let mut events = Vec::new();
    for event in entry["events"].as_array().ok_or("events missing")? {
        let kind = event["kind"].as_str().ok_or("kind missing")?;
        if kind == "speech" || kind == "malformed" {
            continue;
        }
        events.push(VisualEvent {
            fixture: fixture.to_owned(),
            id: event["id"].as_str().ok_or("id missing")?.to_owned(),
            kind: kind.to_owned(),
            start_us: event["start_us"].as_u64().ok_or("start missing")?,
            end_us: event["end_us"].as_u64().ok_or("end missing")?,
        });
    }
    Ok(events)
}

/// Whether the recorded samples inside `event` repeat the pixels of the last
/// sample before it (no block mean moves by more than one level).
pub fn is_visually_identical_to_previous(recorded: &RecordedFixture, event: &VisualEvent) -> bool {
    let samples: Vec<&VisualSample> = recorded
        .windows
        .iter()
        .flat_map(|window| window.samples.iter())
        .collect();
    let Some(before) = samples
        .iter()
        .rev()
        .find(|sample| sample.time().as_micros() < event.start_us)
    else {
        return false;
    };
    let inside: Vec<&&VisualSample> = samples
        .iter()
        .filter(|sample| (event.start_us..event.end_us).contains(&sample.time().as_micros()))
        .collect();
    !inside.is_empty()
        && inside.iter().all(|sample| {
            sample
                .blocks()
                .iter()
                .zip(before.blocks())
                .all(|(left, right)| left.abs_diff(*right) <= 1)
        })
}

const fn is_change_reason(reason: CandidateReason) -> bool {
    matches!(
        reason,
        CandidateReason::VisualChange
            | CandidateReason::MotionStart
            | CandidateReason::SettledAfterMotion
    )
}

/// The outcome of one event.
#[derive(Clone, Debug)]
pub struct EventScore {
    pub event: VisualEvent,
    pub hit: Option<VisualCandidate>,
    pub timestamp_error_us: Option<i64>,
    pub brackets_start: Option<bool>,
}

pub fn score_event(index: &VisualIndex, event: &VisualEvent) -> Built<EventScore> {
    let window = TimeRange::new(
        MediaTime::from_micros(event.start_us),
        MediaTime::from_micros(event.end_us),
    )?;
    let hit = index.candidates_in(window).next().cloned();
    let timestamp_error_us = match &hit {
        Some(candidate) => Some(
            i64::try_from(candidate.representative().as_micros())? - i64::try_from(event.start_us)?,
        ),
        None => None,
    };
    let brackets_start = hit.as_ref().map(|candidate| {
        candidate.representative().as_micros() == event.start_us
            || candidate.change().is_some_and(|change| {
                change.previous_sample().as_micros() < event.start_us
                    && event.start_us <= candidate.representative().as_micros()
            })
    });
    Ok(EventScore {
        event: event.clone(),
        hit,
        timestamp_error_us,
        brackets_start,
    })
}

/// Change candidates whose change interval holds no visual event boundary.
pub fn false_changes(index: &VisualIndex, events: &[VisualEvent]) -> Vec<VisualCandidate> {
    let duration = index.duration().as_micros();
    let boundaries: Vec<u64> = events
        .iter()
        .flat_map(|event| [event.start_us, event.end_us])
        .filter(|boundary| *boundary > 0 && *boundary < duration)
        .collect();
    index
        .windows()
        .iter()
        .flat_map(|window| window.candidates().iter())
        .filter(|candidate| is_change_reason(candidate.reason()))
        .filter(|candidate| {
            let Some(change) = candidate.change() else {
                return true;
            };
            let after = change.previous_sample().as_micros();
            let by = candidate.representative().as_micros();
            !boundaries
                .iter()
                .any(|boundary| after < *boundary && *boundary <= by)
        })
        .cloned()
        .collect()
}

fn ratio(numerator: usize, denominator: usize) -> Value {
    json!({ "hit": numerator, "total": denominator })
}

fn per_minute(count: usize, micros: u64) -> Built<f64> {
    let count = f64::from(u32::try_from(count)?);
    let millis = f64::from(u32::try_from(micros / 1_000)?);
    Ok(if millis == 0.0 {
        0.0
    } else {
        count * 60_000.0 / millis
    })
}

fn fixture_row(recorded: &RecordedFixture, index: &VisualIndex, false_count: usize) -> Value {
    json!({
        "fixture": recorded.fixture,
        "duration_us": index.duration().as_micros(),
        "samples": recorded.windows.iter().map(|window| window.samples.len()).sum::<usize>(),
        "candidates": index
            .windows()
            .iter()
            .flat_map(|window| window.candidates().iter())
            .map(|candidate| json!({
                "representative_us": candidate.representative().as_micros(),
                "reason": candidate.reason().identifier(),
                "stability": candidate.stability().identifier(),
                "visual_hash": candidate.hash().to_hex(),
            }))
            .collect::<Vec<_>>(),
        "false_change_candidates": false_count,
    })
}

/// The recall report over indexed fixtures, with the gate's failures.
pub struct RecallReport {
    pub json: Value,
    pub failures: Vec<String>,
}

pub fn recall_report(
    manifest: &Value,
    indexed: &[(RecordedFixture, VisualIndex)],
) -> Built<RecallReport> {
    let mut failures = Vec::new();
    let mut event_rows = Vec::new();
    let mut by_kind: Vec<(String, usize, usize)> = Vec::new();
    let mut gated = (0_usize, 0_usize);
    let mut false_total = 0_usize;
    let mut candidate_total = 0_usize;
    let mut media_micros = 0_u64;
    let mut fixture_rows = Vec::new();
    for (recorded, index) in indexed {
        let events = visual_events(manifest, &recorded.fixture)?;
        let false_found = false_changes(index, &events);
        let candidates = index
            .windows()
            .iter()
            .map(|window| window.candidates().len())
            .sum::<usize>();
        false_total += false_found.len();
        candidate_total += candidates;
        media_micros += index.duration().as_micros();
        if STATIC_FIXTURES.contains(&recorded.fixture.as_str()) && !false_found.is_empty() {
            failures.push(format!(
                "{}: {} false change candidate(s) in a static fixture",
                recorded.fixture,
                false_found.len()
            ));
        }
        fixture_rows.push(fixture_row(recorded, index, false_found.len()));
        for event in &events {
            if let Some(reason) = event.limitation()
                && !is_visually_identical_to_previous(recorded, event)
            {
                failures.push(format!(
                    "{}: listed as a corpus limitation ({reason}) but its pixels differ from the previous state",
                    event.id
                ));
            }
            let score = score_event(index, event)?;
            let hit = score.hit.is_some();
            match by_kind.iter_mut().find(|(kind, ..)| *kind == event.kind) {
                Some(entry) => {
                    entry.1 += usize::from(hit);
                    entry.2 += 1;
                }
                None => by_kind.push((event.kind.clone(), usize::from(hit), 1)),
            }
            if event.is_gated() {
                gated.1 += 1;
                gated.0 += usize::from(hit);
                if !hit {
                    failures.push(format!("{}: gated stable event was not hit", event.id));
                }
            }
            event_rows.push(json!({
                "event": event.id,
                "kind": event.kind,
                "start_us": event.start_us,
                "end_us": event.end_us,
                "gated": event.is_gated(),
                "corpus_limitation": event.limitation(),
                "hit": hit,
                "candidate": score.hit.as_ref().map(|candidate| json!({
                    "reason": candidate.reason().identifier(),
                    "representative_us": candidate.representative().as_micros(),
                    "stability": candidate.stability().identifier(),
                })),
                "timestamp_error_us": score.timestamp_error_us,
                "change_window_brackets_start": score.brackets_start,
            }));
        }
    }
    let transient = by_kind
        .iter()
        .find(|(kind, ..)| kind == "transient")
        .map_or_else(|| ratio(0, 0), |(_, hit, total)| ratio(*hit, *total));
    let json = json!({
        "profile": VisualIndexProfile::R0.identifier(),
        "media_seconds": media_micros / 1_000_000,
        "fixtures": fixture_rows,
        "events": event_rows,
        "recall_by_kind": by_kind
            .iter()
            .map(|(kind, hit, total)| (kind.clone(), ratio(*hit, *total)))
            .collect::<serde_json::Map<String, Value>>(),
        "transient_recall": transient,
        "false_change_candidates": false_total,
        "false_change_candidates_per_minute": per_minute(false_total, media_micros)?,
        "candidates": candidate_total,
        "candidates_per_minute": per_minute(candidate_total, media_micros)?,
        "gate": {
            "rule": "every stable event of at least 1 s is hit; no false change in F01 or F07; other events are reported",
            "gated_events": gated.1,
            "gated_hits": gated.0,
            "passed": failures.is_empty(),
        },
    });
    Ok(RecallReport { json, failures })
}
