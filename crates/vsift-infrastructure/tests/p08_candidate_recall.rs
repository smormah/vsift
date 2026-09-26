//! Always-run V-02..V-05 recall gate of the visual index over recorded
//! samples (P08 PR 3).
//!
//! The recorded samples in `tests/data/visual_samples/` are what real
//! `FFmpeg` decoded for each visual fixture through
//! `FfmpegMedia::visual_samples`; this test replays them through the real
//! index extension and scores the candidates against the frozen manifest
//! truth only. It prints the recall report; run with `--nocapture` to see it.
//! The opt-in `p08_candidates_fixtures` test runs the same gate on a live
//! decode.

mod candidate_recall;

use std::error::Error;

use candidate_recall::{
    CORPUS_LIMITATIONS, RECORDED_FIXTURES, RecordedFixture, RecordedWindow, VisualEvent,
    false_changes, index_recorded, is_visually_identical_to_previous, load_recorded, manifest,
    parse_recorded, recall_report, recorded_text, score_event,
};
use serde_json::json;
use vsift_domain::{MediaTime, VISUAL_BLOCKS, VisualHash, VisualSample};

type TestResult = Result<(), Box<dyn Error>>;

#[tokio::test]
async fn recorded_fixtures_meet_the_candidate_recall_gate() -> TestResult {
    let truth = manifest()?;
    let mut indexed = Vec::new();
    for fixture in RECORDED_FIXTURES {
        let recorded = load_recorded(fixture)?;
        let index = index_recorded(&recorded).await?;
        indexed.push((recorded, index));
    }
    let report = recall_report(&truth, &indexed)?;
    println!("{}", serde_json::to_string_pretty(&report.json)?);
    assert!(report.failures.is_empty(), "{:#?}", report.failures);
    assert_eq!(report.json["gate"]["gated_events"], json!(10));
    assert_eq!(report.json["false_change_candidates"], json!(0));
    // The reviewed limitations are exactly the unhit visual events.
    let missed: Vec<&str> = report.json["events"]
        .as_array()
        .ok_or("no events")?
        .iter()
        .filter(|event| event["hit"] == json!(false))
        .filter_map(|event| event["event"].as_str())
        .collect();
    assert_eq!(missed, vec!["F04-E02", "F05-E02"]);
    assert_eq!(CORPUS_LIMITATIONS.len(), 3);
    Ok(())
}

#[test]
fn recorded_samples_round_trip_through_their_text_form() -> TestResult {
    let recorded = load_recorded("F09")?;
    let text = recorded_text(&recorded, &json!({"ffmpeg": "test"}))?;
    assert_eq!(parse_recorded(&serde_json::from_str(&text)?)?, recorded);
    Ok(())
}

fn level_sample(time_us: u64, level: u8) -> VisualSample {
    VisualSample::from_parts(
        MediaTime::from_micros(time_us),
        [level; VISUAL_BLOCKS],
        VisualHash::from_bits(u64::from(level)),
    )
}

fn event(id: &str, kind: &str, start_us: u64, end_us: u64) -> VisualEvent {
    VisualEvent {
        fixture: "FXX".to_owned(),
        id: id.to_owned(),
        kind: kind.to_owned(),
        start_us,
        end_us,
    }
}

/// The scoring itself, on a synthetic recording: a change at 2 s is a hit
/// bracketed by its change window, a change with no truth boundary is false,
/// and an event drawn like the state before it is recognised as identical.
#[tokio::test]
async fn scoring_follows_the_truth_windows_only() -> TestResult {
    let samples: Vec<VisualSample> = (0..20_u64)
        .map(|step| {
            let time = step * 500_000;
            let level = match time {
                0..2_000_000 => 40,
                2_000_000..7_000_000 => 90,
                _ => 150,
            };
            level_sample(time, level)
        })
        .collect();
    let recorded = RecordedFixture {
        fixture: "FXX".to_owned(),
        duration: MediaTime::from_micros(10_000_000),
        source_sha256: "0".repeat(64),
        windows: vec![RecordedWindow {
            ordinal: 0,
            samples,
        }],
    };
    let index = index_recorded(&recorded).await?;
    let changed = event("FXX-E01", "change", 1_800_000, 5_000_000);
    let score = score_event(&index, &changed)?;
    assert_eq!(score.timestamp_error_us, Some(200_000));
    assert_eq!(score.brackets_start, Some(true));
    let unseen = event("FXX-E02", "stable", 5_000_000, 6_000_000);
    assert!(score_event(&index, &unseen)?.hit.is_none());
    let events = [changed.clone(), unseen.clone()];
    let false_found = false_changes(&index, &events);
    assert_eq!(
        false_found
            .iter()
            .map(|candidate| candidate.representative().as_micros())
            .collect::<Vec<_>>(),
        vec![7_000_000]
    );
    assert!(is_visually_identical_to_previous(&recorded, &unseen));
    assert!(!is_visually_identical_to_previous(&recorded, &changed));
    Ok(())
}
