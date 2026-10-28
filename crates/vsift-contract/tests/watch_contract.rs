//! `watch`, from `vsift <video>`, through the v1 contract (ADR 0037 section
//! 9): the result validates against the published schema, the frozen example
//! is what the code gives, and the schema refuses what the action never
//! reports (a hash on the folder, a partial run called complete, more frames
//! than the bound of 180).

use std::{fs, io, path::PathBuf};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_contract::{
    CommandName, OperationResponse, WatchCompleteness, WatchData, WatchFrameData, WatchOutputData,
    WatchOutputsData, WatchRangeData,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SCHEMA: &str = "watch-data.schema.json";
const EXAMPLE: &str = "examples/watch.json";
const SECOND: u64 = 1_000_000;
const VIDEO_SHA256: &str = "3f9a0c5d1e7b2a4c6f8e0d2b4a6c8e0f1a3b5c7d9e1f2a4b6c8d0e2f4a6be21c";
const FOLDER: &str = "/home/example/videos";

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn load(relative_path: &str) -> Built<Value> {
    Ok(serde_json::from_str(&fs::read_to_string(
        repository("schemas/v1").join(relative_path),
    )?)?)
}

struct PublishedSchemas;

impl Retrieve for PublishedSchemas {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let name = uri
            .as_str()
            .strip_prefix(SCHEMA_BASE)
            .filter(|name| !name.contains(['/', '\\']))
            .ok_or_else(|| format!("unpublished schema reference: {uri}"))?;
        Ok(serde_json::from_str(&fs::read_to_string(
            repository("schemas/v1").join(name),
        )?)?)
    }
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    let schema = load(schema_path)?;
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

/// Rewrites the example from `value` when `VSIFT_REGENERATE_CONTRACT_EXAMPLES`
/// is `1`, as the other contract tests do; review the diff.
fn regenerate(relative_path: &str, value: &Value) -> TestResult {
    if std::env::var("VSIFT_REGENERATE_CONTRACT_EXAMPLES").is_ok_and(|setting| setting == "1") {
        fs::write(
            repository("schemas/v1").join(relative_path),
            format!("{}\n", serde_json::to_string_pretty(value)?),
        )?;
    }
    Ok(())
}

/// `relative` is the output's path from the video's folder; its name is the
/// last part, as `WatchOutputData::new` gives it.
fn file(relative: &str, sha256: Option<&str>) -> WatchOutputData {
    WatchOutputData {
        path: format!("{FOLDER}/{relative}"),
        name: relative.rsplit('/').next().unwrap_or(relative).to_owned(),
        sha256: sha256.map(str::to_owned),
    }
}

fn frame(seconds: u64, millis: u64, suffix: &str, sha256: &str) -> WatchFrameData {
    let hours = seconds / 3_600;
    let minutes = seconds / 60 % 60;
    let rest = seconds % 60;
    WatchFrameData {
        time_us: seconds * SECOND + millis * 1_000,
        candidate_id: format!("vcd_7f20c4{suffix}"),
        evidence_id: format!("evd_c19a3b{suffix}"),
        path: format!("{FOLDER}/meeting-vsift/{hours:02}-{minutes:02}-{rest:02}.{millis:03}.png"),
        sha256: sha256.to_owned(),
    }
}

/// A 48-minute meeting, watched once: every output, the run's ids and every
/// hash, with half a minute the speech tool did not transcribe.
fn meeting() -> WatchData {
    WatchData {
        source_id: format!("src_sha256_{VIDEO_SHA256}"),
        session_id: "ses_51d2a70123456789abcdef0123456789".to_owned(),
        session_closed: true,
        job_id: "job_0123456789abcdef0123456789abcdef".to_owned(),
        duration_us: 2_880 * SECOND,
        transcript_revision_id: Some("trv_8c41f00123456789abcdef0123456789".to_owned()),
        visual_index_revision_id: Some("vix_02b7e10123456789abcdef0123456789".to_owned()),
        outcome: WatchCompleteness::Partial,
        not_transcribed: vec![WatchRangeData {
            from_us: 720 * SECOND,
            to_us: 750 * SECOND,
        }],
        screen_changes_beyond_bound: false,
        frames_stopped_for_room: false,
        names_shortened: false,
        outputs: WatchOutputsData {
            transcript: file(
                "meeting.txt",
                Some("5b0e7c2f9a1d3e5b7c9f1a3d5e7b9c1f3a5d7e9b1c3f5a7d9e1b3c5f7a9d1e3b"),
            ),
            subtitles: Some(file(
                "meeting.srt",
                Some("0c2e4a6b8d0f2a4c6e8b0d2f4a6c8e0b2d4f6a8c0e2b4d6f8a0c2e4b6d8f0a2c"),
            )),
            folder: file("meeting-vsift", None),
            screen_changes: file(
                "meeting-vsift/screen-changes.txt",
                Some("e1d3c5b7a9f1e3d5c7b9a1f3e5d7c9b1a3f5e7d9c1b3a5f7e9d1c3b5a7f9e1d3"),
            ),
        },
        changes_found: 2,
        frames: vec![
            frame(
                72,
                480,
                "0123456789abcdef0123456789",
                "9b1f2e0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            ),
            frame(
                295,
                40,
                "fedcba9876543210fedcba9876",
                "41c0a90123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            ),
        ],
    }
}

fn response(data: &WatchData) -> Built<Value> {
    Ok(serde_json::to_value(OperationResponse::complete(
        CommandName::Watch.identifier(),
        data,
    )?)?)
}

/// The result of a run validates, is the frozen example, and names the
/// command `watch` (the bare `vsift <video>` included).
#[test]
fn a_watch_result_is_the_published_example() -> TestResult {
    let response = response(&meeting())?;
    validate("operation-response.schema.json", &response)?;
    validate(SCHEMA, &response["data"])?;
    regenerate(EXAMPLE, &response)?;
    assert_eq!(response, load(EXAMPLE)?);
    assert_eq!(response["command"], "watch");
    assert_eq!(response["status"], "complete");
    assert_eq!(response["data"]["outcome"], "partial");
    Ok(())
}

/// A video with no sound track has no subtitles and no transcript revision,
/// and its result still validates.
#[test]
fn a_silent_video_has_no_subtitles_or_revision() -> TestResult {
    let mut data = meeting();
    data.outputs.subtitles = None;
    data.transcript_revision_id = None;
    data.not_transcribed.clear();
    data.outcome = WatchCompleteness::Complete;
    let response = response(&data)?;
    validate(SCHEMA, &response["data"])?;
    assert_eq!(response["data"]["outputs"]["subtitles"], Value::Null);
    Ok(())
}

/// The schema refuses what the action never reports.
#[test]
fn the_schema_refuses_what_watch_never_reports() -> TestResult {
    let valid = response(&meeting())?["data"].clone();
    validate(SCHEMA, &valid)?;

    // The folder has no hash; a file always has one.
    let mut broken = valid.clone();
    broken["outputs"]["folder"]["sha256"] = Value::from(VIDEO_SHA256);
    assert!(validate(SCHEMA, &broken).is_err());
    let mut broken = valid.clone();
    broken["outputs"]["transcript"]["sha256"] = Value::Null;
    assert!(validate(SCHEMA, &broken).is_err());

    // A hash is 64 lowercase hexadecimal characters.
    let mut broken = valid.clone();
    broken["frames"][0]["sha256"] = Value::from(VIDEO_SHA256.to_uppercase());
    assert!(validate(SCHEMA, &broken).is_err());

    // A run with a gap, or past the four-hour bound, is never complete.
    let mut broken = valid.clone();
    broken["outcome"] = Value::from("complete");
    assert!(validate(SCHEMA, &broken).is_err());
    let mut broken = valid.clone();
    broken["not_transcribed"] = serde_json::json!([]);
    broken["screen_changes_beyond_bound"] = Value::Bool(true);
    broken["outcome"] = Value::from("complete");
    assert!(validate(SCHEMA, &broken).is_err());

    // Frames that stopped for room make the run partial.
    let mut broken = valid.clone();
    broken["not_transcribed"] = serde_json::json!([]);
    broken["outcome"] = Value::from("complete");
    validate(SCHEMA, &broken)?;
    broken["frames_stopped_for_room"] = Value::Bool(true);
    assert!(validate(SCHEMA, &broken).is_err());
    broken["outcome"] = Value::from("partial");
    validate(SCHEMA, &broken)?;

    // At most 180 frames (known limit L-234).
    let mut broken = valid.clone();
    let one = valid["frames"][0].clone();
    broken["frames"] = Value::Array(vec![one; 181]);
    assert!(validate(SCHEMA, &broken).is_err());

    // No member is optional, and none is added.
    let mut broken = valid.clone();
    if let Some(members) = broken.as_object_mut() {
        members.remove("session_closed");
    }
    assert!(validate(SCHEMA, &broken).is_err());
    let mut broken = valid;
    broken["model"] = Value::from("base.en");
    assert!(validate(SCHEMA, &broken).is_err());
    Ok(())
}
