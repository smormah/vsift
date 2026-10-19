//! Opt-in live checks of the visual index over the synthetic corpus (P08).
//!
//! Needs `ffmpeg` and `ffprobe` on `PATH`:
//!
//! ```console
//! cargo test --release -p vsift-infrastructure --locked --test p08_candidates_fixtures -- --ignored --nocapture
//! ```
//!
//! - `p08_candidates_fixtures` decodes every visual fixture through
//!   `FfmpegMedia::visual_samples` exactly as an index extension does,
//!   scores the candidates against the frozen truth with the same gate as the
//!   always-run recorded-sample test, and reports the decode throughput in
//!   media seconds per wall-clock second.
//! - `records_visual_samples_and_reports_drift` decodes the same fixtures and
//!   reports how far the result differs from the committed recorded samples
//!   in `tests/data/visual_samples/`. With `VSIFT_RECORD_VISUAL_SAMPLES=1` it
//!   rewrites them, with the `FFmpeg` version and date as provenance; review
//!   the diff and the recall report before committing a re-recording.

mod candidate_recall;
mod visual_decode;

use std::{env, error::Error, fs, time::Duration};

use candidate_recall::{
    Built, RECORDED_FIXTURES, RecordedFixture, index_recorded, load_recorded, manifest,
    recall_report, recorded_path, recorded_text, repository,
};
use serde_json::{Value, json};
use visual_decode::{Decoded, decode_file, ffmpeg_version, today, tools};
use vsift_domain::{VisualIndex, VisualSample};
use vsift_infrastructure::MediaProviderConformance;

type TestResult = Result<(), Box<dyn Error>>;

fn fixture_file(fixture: &str) -> String {
    let extension = if fixture == "F09" { "mkv" } else { "mp4" };
    format!("fixtures/corpus/generated/{fixture}.{extension}")
}

/// Indexes every window of one fixture through the real adapter; returns
/// the decoded samples, the index and the time spent indexing.
async fn decode_fixture(
    fixture: &str,
    conformance: &MediaProviderConformance,
) -> Built<(RecordedFixture, VisualIndex, Duration)> {
    let Decoded {
        recorded,
        index,
        elapsed,
        ..
    } = decode_file(&repository(&fixture_file(fixture)), fixture, conformance).await?;
    Ok((recorded, index, elapsed))
}

/// Counts samples whose time, blocks or hash differ from the committed ones.
fn drift(committed: &RecordedFixture, fresh: &RecordedFixture) -> Value {
    let committed_samples: Vec<&VisualSample> = committed
        .windows
        .iter()
        .flat_map(|window| window.samples.iter())
        .collect();
    let fresh_samples: Vec<&VisualSample> = fresh
        .windows
        .iter()
        .flat_map(|window| window.samples.iter())
        .collect();
    let paired = committed_samples.iter().zip(&fresh_samples);
    let time_drift = paired
        .clone()
        .filter(|(left, right)| left.time() != right.time())
        .count();
    let block_drift = paired
        .clone()
        .filter(|(left, right)| left.blocks() != right.blocks())
        .count();
    let largest_block_delta = paired
        .clone()
        .flat_map(|(left, right)| {
            left.blocks()
                .iter()
                .zip(right.blocks())
                .map(|(a, b)| a.abs_diff(*b))
        })
        .max()
        .unwrap_or(0);
    let hash_drift = paired
        .filter(|(left, right)| left.hash() != right.hash())
        .count();
    json!({
        "fixture": fresh.fixture,
        "source_changed": committed.source_sha256 != fresh.source_sha256,
        "committed_samples": committed_samples.len(),
        "fresh_samples": fresh_samples.len(),
        "time_drift": time_drift,
        "block_drift": block_drift,
        "largest_block_delta": largest_block_delta,
        "hash_drift": hash_drift,
    })
}

#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
async fn records_visual_samples_and_reports_drift() -> TestResult {
    let (ffmpeg, ffprobe) = tools()?;
    let provenance = json!({
        "ffmpeg": ffmpeg_version(&ffmpeg)?,
        "operation": "FfmpegMedia::visual_samples",
        "profile": "r0-visual-v1: 60 s windows with a 0.5 s lead-in, select >= 0.5 s apart, scale=128:72:flags=area, format=gray, showinfo, -fps_mode passthrough, rawvideo gray",
        "recorded": today()?,
    });
    let conformance = MediaProviderConformance::r0(ffmpeg, ffprobe);
    let write = env::var("VSIFT_RECORD_VISUAL_SAMPLES").is_ok_and(|value| value == "1");
    let mut report = Vec::new();
    for fixture in RECORDED_FIXTURES {
        let (fresh, _, _) = decode_fixture(fixture, &conformance).await?;
        match load_recorded(fixture) {
            Ok(committed) => report.push(drift(&committed, &fresh)),
            Err(_) => report.push(json!({ "fixture": fixture, "committed": null })),
        }
        if write {
            let path = recorded_path(fixture);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, recorded_text(&fresh, &provenance)?)?;
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "provenance": provenance,
            "written": write,
            "drift": report,
        }))?
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
async fn p08_candidates_fixtures() -> TestResult {
    let (ffmpeg, ffprobe) = tools()?;
    let conformance = MediaProviderConformance::r0(ffmpeg, ffprobe);
    let truth = manifest()?;
    let mut indexed = Vec::new();
    let mut decode_time = Duration::ZERO;
    for fixture in RECORDED_FIXTURES {
        let (fresh, index, elapsed) = decode_fixture(fixture, &conformance).await?;
        decode_time += elapsed;
        // The live index must equal the one the recorded-sample replay builds
        // from the same samples, apart from the session its identities name.
        let replayed = index_recorded(&fresh).await?;
        if replayed.windows().len() != index.windows().len()
            || replayed
                .windows()
                .iter()
                .zip(index.windows())
                .any(|(left, right)| {
                    left.candidates().len() != right.candidates().len()
                        || left
                            .candidates()
                            .iter()
                            .zip(right.candidates())
                            .any(|(a, b)| a.draft() != b.draft())
                })
        {
            return Err(format!("{fixture}: the live index differs from its replay").into());
        }
        indexed.push((fresh, index));
    }
    let report = recall_report(&truth, &indexed)?;
    let media_seconds = indexed
        .iter()
        .map(|(recorded, _)| recorded.duration.as_micros())
        .sum::<u64>();
    let throughput = f64::from(u32::try_from(media_seconds / 1_000)?)
        / 1_000.0
        / decode_time.as_secs_f64().max(f64::EPSILON);
    let mut json = report.json;
    json["throughput_media_seconds_per_second"] = json!(throughput);
    json["decode_seconds"] = json!(decode_time.as_secs_f64());
    println!("{}", serde_json::to_string_pretty(&json)?);
    assert!(report.failures.is_empty(), "{:#?}", report.failures);
    Ok(())
}
