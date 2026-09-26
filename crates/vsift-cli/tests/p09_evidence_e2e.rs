//! Opt-in cumulative P09 checkpoint: evidence navigation (V-01, V-06..V-08).
//!
//! Journeys drive the compiled `vsift` binary exactly as a headless agent
//! would, with isolated per-user bases and an empty `PATH`: `FFmpeg` and
//! `FFprobe` are registered with `setup configure`. Expectations come only
//! from the frozen corpus truth: the independent `ffprobe` frame lists the
//! fixture verifier recorded (`fixtures/corpus/generated/verification.json`),
//! the manifest (`fixtures/corpus/manifest.json`) and `FFmpeg`'s own decode
//! of the fixture for pixel comparisons; nothing is derived from the results
//! being scored. The stages:
//!
//! - `p09_frame_exact` (V-01): F01 at a keyframe, just before one, between
//!   frames, the final frame, after it (at-or-after refused with its typed
//!   reason, displayed-at the final frame) and at the end; F09's variable
//!   frame rate; the rotated F01 variant at its displayed 720x1280, pixel for
//!   pixel equal to `FFmpeg`'s decode;
//! - `p09_candidate_frames` (V-01 with P08): every candidate `candidates`
//!   returns for F01-F10 and F12, through `frame get --candidate`, is the
//!   candidate's own frame (delta 0) at its displayed dimensions;
//! - `p09_neighbours_burst` (V-07): neighbours stop at the start and end of
//!   the stream with typed side stops; bursts of 0, 1, 12, 100 and 101
//!   frames; a 60 s range is clipped to the video and 61 s is refused with
//!   the remediation to use `candidates`;
//! - `p09_reuse` (V-08): a repeated request is `reused` without writing, and
//!   two requests that resolve to one frame share one item and one file.
//!
//! ```console
//! cargo test -p vsift-cli --locked --test p09_evidence_e2e -- --ignored --nocapture
//! ```
//!
//! The run writes `.vsift/e2e-runs/p09-<run-id>/report.json` and prints
//! `p09_evidence: passed` when every stage passed. A stage that cannot run
//! here is `blocked`, never `passed`, and the test fails unless every stage
//! passed.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command as Process,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::{Value, json};
use vsift_contract::BURST_RANGE_REMEDIATION;
use vsift_infrastructure::{ExecutableResolver, TrustedExecutable};

type TestResult = Result<(), Box<dyn Error>>;

/// Upper bound for one CLI invocation.
const CLI_DEADLINE: Duration = Duration::from_mins(5);
const OWNED_PREFIX: &str = "vsift-p09-evidence-e2e-";
const MAX_DIAGNOSTIC_CHARS: usize = 240;
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SECOND: u64 = 1_000_000;
const MISSING_MEDIA_TOOLS: &str = "FFmpeg or FFprobe is not on PATH; install or locate trusted builds, then run setup configure ffmpeg|ffprobe --executable <absolute-path>";
/// Fixtures with visual truth and generated media.
const VISUAL_FIXTURES: [(&str, &str); 11] = [
    ("F01", "F01.mp4"),
    ("F02", "F02.mp4"),
    ("F03", "F03.mp4"),
    ("F04", "F04.mp4"),
    ("F05", "F05.mp4"),
    ("F06", "F06.mp4"),
    ("F07", "F07.mp4"),
    ("F08", "F08.mp4"),
    ("F09", "F09.mkv"),
    ("F10", "F10.mp4"),
    ("F12", "F12.mp4"),
];

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn base(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

enum StageStop {
    Failed(String),
    Blocked(String),
}

impl<E: Error> From<E> for StageStop {
    fn from(error: E) -> Self {
        Self::Failed(bounded(&error.to_string()))
    }
}

type StageResult = Result<Value, StageStop>;

fn bounded(text: &str) -> String {
    text.chars().take(MAX_DIAGNOSTIC_CHARS).collect()
}

fn ensure(condition: bool, expectation: &str) -> Result<(), StageStop> {
    if condition {
        Ok(())
    } else {
        Err(StageStop::Failed(bounded(expectation)))
    }
}

fn failed(expectation: &str) -> StageStop {
    StageStop::Failed(bounded(expectation))
}

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> PathBuf {
    repository().join("fixtures/corpus/generated").join(name)
}

struct MediaTools {
    ffmpeg: TrustedExecutable,
    ffprobe: TrustedExecutable,
}

impl MediaTools {
    fn discover() -> Option<Self> {
        let resolver = ExecutableResolver::from_current_path();
        Some(Self {
            ffmpeg: resolver.resolve(OsStr::new("ffmpeg")).ok()?,
            ffprobe: resolver.resolve(OsStr::new("ffprobe")).ok()?,
        })
    }

    /// Decodes an image file to packed 8-bit RGB with `FFmpeg`.
    fn decode_image(&self, image: &Path) -> Result<Vec<u8>, StageStop> {
        let output = Process::new(self.ffmpeg.path())
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(image)
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
            .output()?;
        ensure(output.status.success(), "FFmpeg could not decode an image")?;
        Ok(output.stdout)
    }

    /// Decodes the displayed frame with timestamp `pts` of `source`'s first
    /// video stream to packed 8-bit RGB with `FFmpeg`'s own defaults
    /// (display rotation applied): the independent reference.
    fn decode_reference(&self, source: &Path, pts: i64) -> Result<Vec<u8>, StageStop> {
        let output = Process::new(self.ffmpeg.path())
            .args(["-v", "error", "-nostdin", "-copyts", "-i"])
            .arg(source)
            .args(["-map", "0:v:0", "-vf"])
            .arg(format!("select=eq(pts\\,{pts})"))
            .args([
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "pipe:1",
            ])
            .output()?;
        ensure(
            output.status.success(),
            "FFmpeg could not decode the reference frame",
        )?;
        Ok(output.stdout)
    }
}

/// A bounded `vsift` invocation with isolated per-user state and no ambient `PATH`.
fn vsift(base: &Path) -> Result<Command, StageStop> {
    let mut command = Command::cargo_bin("vsift")?;
    command
        .env("LOCALAPPDATA", base)
        .env("XDG_CONFIG_HOME", base)
        .env("HOME", base)
        .env("PATH", "")
        .arg("--session-root")
        .arg(base.join("sessions"))
        .timeout(CLI_DEADLINE);
    Ok(command)
}

fn run_json(command: &mut Command) -> Result<(Option<i32>, Value), StageStop> {
    let output = command.output()?;
    ensure(
        output.stderr.is_empty(),
        "a JSON command wrote to standard error",
    )?;
    Ok((
        output.status.code(),
        serde_json::from_slice(&output.stdout)?,
    ))
}

struct PublishedSchemas;

impl Retrieve for PublishedSchemas {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let name = uri
            .as_str()
            .strip_prefix(SCHEMA_BASE)
            .filter(|name| !name.contains(['/', '\\']))
            .ok_or_else(|| format!("unpublished schema reference: {uri}"))?;
        Ok(serde_json::from_slice(&fs::read(
            repository().join("schemas/v1").join(name),
        )?)?)
    }
}

fn conforms(schema: &str, instance: &Value) -> Result<(), StageStop> {
    let definition: Value =
        serde_json::from_slice(&fs::read(repository().join("schemas/v1").join(schema))?)?;
    let validator = jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&definition)
        .map_err(|error| failed(&error.to_string()))?;
    ensure(
        validator.is_valid(instance),
        &format!("an emitted document does not conform to {schema}"),
    )
}

/// Registers the media tools in `base`.
fn prepare(base: &Path, tools: &MediaTools) -> Result<(), StageStop> {
    for (dependency, path) in [
        ("ffmpeg", tools.ffmpeg.path()),
        ("ffprobe", tools.ffprobe.path()),
    ] {
        let (code, _) = run_json(
            vsift(base)?
                .args(["setup", "configure", dependency, "--executable"])
                .arg(path)
                .arg("--json"),
        )?;
        ensure(code == Some(0), "setup configure did not succeed")?;
    }
    Ok(())
}

fn ingest(base: &Path, video: &Path) -> Result<String, StageStop> {
    let (code, opened) = run_json(vsift(base)?.arg("ingest").arg(video).arg("--json"))?;
    ensure(
        code == Some(0),
        &format!("ingest failed: {}", opened["error"]["code"]),
    )?;
    opened["data"]["session_id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| failed("session missing"))
}

/// One evidence command through the binary: exit code, result and wall
/// time; a successful result is validated against its schemas.
fn evidence(base: &Path, arguments: &[&str]) -> Result<(Option<i32>, Value, Duration), StageStop> {
    let started = Instant::now();
    let (code, result) = run_json(vsift(base)?.args(arguments).arg("--json"))?;
    let elapsed = started.elapsed();
    conforms("operation-response.schema.json", &result)?;
    if result["error"].is_null() {
        conforms("frame-data.schema.json", &result["data"])?;
    }
    Ok((code, result, elapsed))
}

/// A successful evidence command's result.
fn evidence_ok(base: &Path, arguments: &[&str]) -> Result<(Value, Duration), StageStop> {
    let (code, result, elapsed) = evidence(base, arguments)?;
    ensure(
        code == Some(0),
        &format!(
            "{arguments:?} failed: {} {}",
            result["error"]["code"], result["error"]["remediation"][0]["summary"]
        ),
    )?;
    Ok((result, elapsed))
}

fn u64_of(value: &Value) -> Result<u64, StageStop> {
    value
        .as_u64()
        .ok_or_else(|| failed("an expected integer is missing"))
}

/// The independent `ffprobe` frame times the fixture verifier recorded.
fn verified_frame_times(name: &str) -> Result<Vec<u64>, StageStop> {
    let report: Value = serde_json::from_slice(&fs::read(fixture("verification.json"))?)?;
    report["frame_timestamps_us"][name]
        .as_array()
        .ok_or_else(|| failed("the verifier recorded no frame times for this fixture"))?
        .iter()
        .map(u64_of)
        .collect()
}

/// The frame the truth says a policy names: the first at or after `at`, or
/// the last at or before it.
fn truth_frame(times: &[u64], at: u64, displayed_at: bool) -> Option<u64> {
    if displayed_at {
        times.iter().rev().find(|time| **time <= at).copied()
    } else {
        times.iter().find(|time| **time >= at).copied()
    }
}

/// The single selection and item of a `frame get` result.
fn single(result: &Value) -> Result<(&Value, &Value), StageStop> {
    let data = &result["data"];
    ensure(
        data["selections"].as_array().map(Vec::len) == Some(1)
            && data["items"].as_array().map(Vec::len) == Some(1)
            && data["files"].as_array().map(Vec::len) == Some(1),
        "a frame get did not return exactly one frame",
    )?;
    Ok((&data["selections"][0], &data["items"][0]))
}

fn file_path(result: &Value, index: usize) -> Result<PathBuf, StageStop> {
    let path = PathBuf::from(
        result["data"]["files"][index]["path"]
            .as_str()
            .ok_or_else(|| failed("a delivered file has no path"))?,
    );
    ensure(path.is_absolute(), "a delivered path is not absolute")?;
    ensure(path.is_file(), "a delivered file does not exist")?;
    Ok(path)
}

/// V-01 on F01, F09 and the rotated F01 variant against the frozen frame
/// lists.
#[allow(
    clippy::too_many_lines,
    reason = "One fixture's V-01 cases read best in one journey"
)]
fn frame_exact_stage(base: &Path, tools: &MediaTools) -> StageResult {
    let f01 = ingest(base, &fixture("F01.mp4"))?;
    let truth = verified_frame_times("F01")?;
    ensure(
        truth.len() == 120 && truth.last() == Some(&5_950_000),
        "the F01 truth is not 120 frames ending at 5.95 s",
    )?;
    let mut cases = Vec::new();
    let mut timings = Vec::new();
    for (label, at, displayed_at, expected_delta) in [
        ("keyframe", 2_000_000_u64, false, 0_i64),
        ("before_keyframe", 1_950_000, false, 0),
        ("between_frames", 1_025_000, false, 25_000),
        ("final_frame", 5_950_000, false, 0),
        ("after_final_displayed_at", 5_970_000, true, -20_000),
        ("between_displayed_at", 1_025_000, true, -25_000),
    ] {
        let at_text = at.to_string();
        let mut arguments = vec!["frame", "get", f01.as_str(), "--at", at_text.as_str()];
        if displayed_at {
            arguments.extend(["--select", "displayed-at"]);
        }
        let (result, elapsed) = evidence_ok(base, &arguments)?;
        timings.push(elapsed.as_millis());
        let (selection, item) = single(&result)?;
        let expected = truth_frame(&truth, at, displayed_at)
            .ok_or_else(|| failed("the truth names no frame"))?;
        let actual = u64_of(&selection["actual_us"])?;
        ensure(
            actual == expected
                && selection["delta_us"].as_i64() == Some(expected_delta)
                && u64_of(&selection["requested_us"])? == at
                && u64_of(&item["frame"]["time_us"])? == expected,
            &format!("{label}: {at} gave {actual}, the truth {expected}"),
        )?;
        ensure(
            item["image"]["width"] == 1_280 && item["image"]["height"] == 720,
            &format!("{label}: the image is not F01's 1280x720"),
        )?;
        file_path(&result, 0)?;
        cases.push(json!({"case": label, "requested_us": at, "actual_us": actual, "delta_us": selection["delta_us"], "ms": elapsed.as_millis()}));
    }
    // After the final frame, at-or-after has no frame; at the end neither
    // policy has one. Both are typed, and nothing is committed.
    let (_, before) = run_json(vsift(base)?.args(["session", "status", &f01, "--json"]))?;
    for (label, at, select, reason) in [
        (
            "after_final_at_or_after",
            "5970000",
            None,
            "follows the video's final frame",
        ),
        (
            "at_end_at_or_after",
            "6000000",
            None,
            "at or after the end of the video",
        ),
        (
            "at_end_displayed_at",
            "6000000",
            Some("displayed-at"),
            "at or after the end of the video",
        ),
    ] {
        let mut arguments = vec!["frame", "get", f01.as_str(), "--at", at];
        if let Some(select) = select {
            arguments.extend(["--select", select]);
        }
        let (code, result, _) = evidence(base, &arguments)?;
        ensure(
            code == Some(2)
                && result["error"]["code"] == "INVALID_ARGUMENT"
                && result["error"]["remediation"][0]["summary"]
                    .as_str()
                    .is_some_and(|summary| summary.contains(reason)),
            &format!(
                "{label}: {} {}",
                result["error"]["code"], result["error"]["remediation"][0]["summary"]
            ),
        )?;
        cases.push(json!({"case": label, "requested_us": at, "error": result["error"]["code"]}));
    }
    let (_, after) = run_json(vsift(base)?.args(["session", "status", &f01, "--json"]))?;
    ensure(
        before["data"]["artifact_count"] == after["data"]["artifact_count"],
        "a refused frame request committed something",
    )?;
    // A tolerance smaller than the distance to the next frame is refused.
    let (code, tight, _) = evidence(
        base,
        &[
            "frame",
            "get",
            &f01,
            "--at",
            "1025000",
            "--tolerance-us",
            "10000",
        ],
    )?;
    ensure(
        code == Some(2) && tight["error"]["code"] == "INVALID_ARGUMENT",
        "a 10 ms tolerance 25 ms from a frame was not refused",
    )?;

    // F09: variable frame rate with a 2 s container origin.
    let f09 = ingest(base, &fixture("F09.mkv"))?;
    let f09_truth = verified_frame_times("F09")?;
    let mut vfr = Vec::new();
    for (at, displayed_at) in [(3_200_000_u64, false), (3_200_000, true), (0, false)] {
        let at_text = at.to_string();
        let mut arguments = vec!["frame", "get", f09.as_str(), "--at", at_text.as_str()];
        if displayed_at {
            arguments.extend(["--select", "displayed-at"]);
        }
        let (result, _) = evidence_ok(base, &arguments)?;
        let (selection, _) = single(&result)?;
        let expected = truth_frame(&f09_truth, at, displayed_at)
            .ok_or_else(|| failed("the F09 truth names no frame"))?;
        let actual = u64_of(&selection["actual_us"])?;
        ensure(
            actual == expected,
            &format!("F09 {at}: {actual}, the truth {expected}"),
        )?;
        vfr.push(json!({"requested_us": at, "displayed_at": displayed_at, "actual_us": actual}));
    }
    ensure(
        vfr[0]["actual_us"] == 3_250_000 && vfr[1]["actual_us"] == 3_150_000,
        "F09 3.2 s did not give 3.25 s at-or-after and 3.15 s displayed-at",
    )?;

    // The rotated variant: displayed 720x1280, equal to FFmpeg's own decode.
    let rotated_source = fixture("F01-rotation-90.mp4");
    let rotated = ingest(base, &rotated_source)?;
    let (result, _) = evidence_ok(base, &["frame", "get", &rotated, "--at", "2000000"])?;
    let (_, item) = single(&result)?;
    ensure(
        item["image"]["width"] == 720
            && item["image"]["height"] == 1_280
            && item["frame"]["width"] == 720
            && item["frame"]["height"] == 1_280,
        "the rotated variant is not displayed at 720x1280",
    )?;
    let pts = item["frame"]["pts"]
        .as_i64()
        .ok_or_else(|| failed("no pts"))?;
    let pixels = tools.decode_image(&file_path(&result, 0)?)?;
    let reference = tools.decode_reference(&rotated_source, pts)?;
    ensure(
        reference.len() == 720 * 1_280 * 3 && pixels == reference,
        "the rotated frame differs from FFmpeg's decode",
    )?;
    Ok(json!({
        "f01_cases": cases,
        "f01_cold_frame_get_ms": timings,
        "f09_variable_frame_rate": vfr,
        "rotated": {"width": 720, "height": 1_280, "pts": pts, "pixel_equal_to_ffmpeg_decode": true},
    }))
}

/// Every candidate of every visual fixture is its own frame.
fn candidate_frames_stage(base: &Path) -> StageResult {
    let mut per_fixture = Vec::new();
    let mut total = 0_usize;
    for (name, file) in VISUAL_FIXTURES {
        let session = ingest(base, &fixture(file))?;
        let (code, page) = run_json(
            vsift(base)?
                .args(["candidates", &session, "--from", "0", "--to", "600000000"])
                .args(["--limit", "100", "--json"]),
        )?;
        ensure(
            code == Some(0) && page["data"]["next_cursor"].is_null(),
            &format!(
                "{name}: candidates failed or paged: {}",
                page["error"]["code"]
            ),
        )?;
        let items = page["data"]["items"]
            .as_array()
            .ok_or_else(|| failed("no candidates"))?;
        ensure(!items.is_empty(), &format!("{name} has no candidates"))?;
        for candidate in items {
            let id = candidate["candidate_id"]
                .as_str()
                .ok_or_else(|| failed("no candidate id"))?;
            let (result, _) = evidence_ok(base, &["frame", "get", &session, "--candidate", id])?;
            let (selection, item) = single(&result)?;
            let representative = u64_of(&candidate["representative_us"])?;
            ensure(
                selection["delta_us"] == 0
                    && u64_of(&selection["actual_us"])? == representative
                    && result["data"]["request"]["candidate_id"] == id
                    && result["data"]["request"]["tolerance_us"] == 0
                    && item["image"]["width"] == candidate["displayed_dimensions"]["width"]
                    && item["image"]["height"] == candidate["displayed_dimensions"]["height"],
                &format!("{name} {id}: not its own frame at {representative}"),
            )?;
        }
        total += items.len();
        per_fixture.push(json!({"fixture": name, "candidates": items.len()}));
    }
    Ok(json!({"candidates_extracted_at_delta_0": total, "per_fixture": per_fixture}))
}

fn selection_times(result: &Value, role: &str) -> Result<Vec<u64>, StageStop> {
    result["data"]["selections"]
        .as_array()
        .ok_or_else(|| failed("no selections"))?
        .iter()
        .filter(|selection| selection["role"] == role)
        .map(|selection| u64_of(&selection["actual_us"]))
        .collect()
}

fn evidence_id(result: &Value) -> Result<String, StageStop> {
    result["data"]["items"][0]["evidence_id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| failed("no evidence id"))
}

/// V-07: side stops, burst sizes and range bounds on F01.
#[allow(
    clippy::too_many_lines,
    reason = "Neighbours and every burst size read best in one journey"
)]
fn neighbours_burst_stage(base: &Path) -> StageResult {
    let truth = verified_frame_times("F01")?;
    let session = ingest(base, &fixture("F01.mp4"))?;
    let (first, _) = evidence_ok(base, &["frame", "get", &session, "--at", "0"])?;
    let (start, _) = evidence_ok(
        base,
        &[
            "frame",
            "neighbours",
            &session,
            &evidence_id(&first)?,
            "--count",
            "2",
        ],
    )?;
    ensure(
        start["data"]["neighbours"]
            == json!({"before_stop": "start_of_stream", "after_stop": null})
            && selection_times(&start, "before")?.is_empty()
            && selection_times(&start, "after")? == truth[1..3],
        "neighbours of the first frame are not the next two with a start stop",
    )?;
    let (last, _) = evidence_ok(base, &["frame", "get", &session, "--at", "5950000"])?;
    let (end, _) = evidence_ok(
        base,
        &[
            "frame",
            "neighbours",
            &session,
            &evidence_id(&last)?,
            "--count",
            "3",
        ],
    )?;
    ensure(
        end["data"]["neighbours"] == json!({"before_stop": null, "after_stop": "end_of_stream"})
            && selection_times(&end, "before")? == truth[116..119]
            && selection_times(&end, "after")?.is_empty(),
        "neighbours of the final frame are not the three before with an end stop",
    )?;
    let (middle, _) = evidence_ok(base, &["frame", "get", &session, "--at", "3000000"])?;
    let (twenty, _) = evidence_ok(
        base,
        &[
            "frame",
            "neighbours",
            &session,
            &evidence_id(&middle)?,
            "--count",
            "20",
        ],
    )?;
    ensure(
        twenty["data"]["neighbours"] == json!({"before_stop": null, "after_stop": null})
            && selection_times(&twenty, "before")? == truth[40..60]
            && selection_times(&twenty, "after")? == truth[61..81],
        "twenty neighbours each side are not the consecutive frames",
    )?;

    let mut bursts = Vec::new();
    for count in ["0", "101"] {
        let (code, result, _) = evidence(
            base,
            &[
                "frame",
                "burst",
                &session,
                "--from",
                "0",
                "--to",
                "1000000",
                "--max-frames",
                count,
            ],
        )?;
        ensure(
            code == Some(2) && result["command"] == "parse",
            &format!("--max-frames {count} was not a parse error"),
        )?;
        bursts.push(json!({"max_frames": count, "outcome": "parse_error"}));
    }
    // One frame, twelve by default, and a hundred distinct frames over the
    // whole 6 s video (targets 60 ms apart on a 50 ms grid), each in a fresh
    // session so the evidence budget is not a factor.
    for (count, expected_distinct) in [(Some("1"), 1_u64), (None, 12), (Some("100"), 100)] {
        let fresh = ingest(base, &fixture("F01.mp4"))?;
        let mut arguments = vec![
            "frame",
            "burst",
            fresh.as_str(),
            "--from",
            "0",
            "--to",
            "6000000",
        ];
        if let Some(count) = count {
            arguments.extend(["--max-frames", count]);
        }
        let (result, elapsed) = evidence_ok(base, &arguments)?;
        let targets = u64::from(count.map_or(Ok(12_u8), str::parse)?);
        let data = &result["data"];
        let expected: Vec<u64> = (0..targets)
            .filter_map(|index| truth_frame(&truth, index * 6 * SECOND / targets, false))
            .collect();
        ensure(
            result["status"] == "complete"
                && data["partial_reason"].is_null()
                && u64_of(&data["burst"]["targets"])? == targets
                && u64_of(&data["burst"]["distinct"])? == expected_distinct
                && data["items"].as_array().map(Vec::len)
                    == usize::try_from(expected_distinct).ok()
                && selection_times(&result, "target")? == expected,
            &format!("a burst of {targets} did not name the truth's frames"),
        )?;
        bursts.push(json!({"max_frames": targets, "distinct": expected_distinct, "ms": elapsed.as_millis()}));
    }
    let fresh = ingest(base, &fixture("F01.mp4"))?;
    let (clipped, _) = evidence_ok(
        base,
        &["frame", "burst", &fresh, "--from", "0", "--to", "60000000"],
    )?;
    ensure(
        clipped["data"]["burst"]["extent"] == "clipped_at_end_of_stream"
            && clipped["data"]["burst"]["planned"] == json!({"start_us": 0, "end_us": 6_000_000}),
        "a 60 s burst over the 6 s video was not clipped to it",
    )?;
    let (code, long, _) = evidence(
        base,
        &["frame", "burst", &fresh, "--from", "0", "--to", "61000000"],
    )?;
    ensure(
        code == Some(2)
            && long["error"]["code"] == "INVALID_ARGUMENT"
            && long["error"]["remediation"][0]["summary"] == BURST_RANGE_REMEDIATION,
        "a 61 s burst was not refused with the candidates remediation",
    )?;
    Ok(json!({
        "neighbours": {
            "first_frame": start["data"]["neighbours"],
            "final_frame": end["data"]["neighbours"],
            "twenty_each_side": selection_times(&twenty, "before")?.len() + selection_times(&twenty, "after")?.len(),
        },
        "bursts": bursts,
        "sixty_seconds": clipped["data"]["burst"],
        "sixty_one_seconds": long["error"]["code"],
    }))
}

fn artifact_count(base: &Path, session: &str) -> Result<u64, StageStop> {
    let (_, status) = run_json(vsift(base)?.args(["session", "status", session, "--json"]))?;
    u64_of(&status["data"]["artifact_count"])
}

/// V-08: repeated requests and requests that resolve to one frame.
fn reuse_stage(base: &Path) -> StageResult {
    let session = ingest(base, &fixture("F01.mp4"))?;
    let (first, cold) = evidence_ok(base, &["frame", "get", &session, "--at", "1025000"])?;
    let committed = artifact_count(base, &session)?;
    let mut warm = Vec::new();
    for _ in 0..3 {
        let (again, elapsed) = evidence_ok(base, &["frame", "get", &session, "--at", "1025000"])?;
        warm.push(elapsed.as_millis());
        let mut unreused = again.clone();
        unreused["data"]["reused"] = Value::Bool(false);
        ensure(
            again["data"]["reused"] == true && unreused["data"] == first["data"],
            "a repeated request was not answered from its record",
        )?;
    }
    ensure(
        artifact_count(base, &session)? == committed,
        "a reused request wrote something",
    )?;
    // 1.04 s is another request for the same frame (1.05 s): a new record,
    // the same item and the same file.
    let (shared, _) = evidence_ok(base, &["frame", "get", &session, "--at", "1040000"])?;
    ensure(
        shared["data"]["reused"] == false
            && shared["data"]["request_key"] != first["data"]["request_key"]
            && evidence_id(&shared)? == evidence_id(&first)?
            && shared["data"]["files"][0]["path"] == first["data"]["files"][0]["path"]
            && artifact_count(base, &session)? == committed + 1,
        "two requests for one frame did not share one item and one file",
    )?;
    ensure(
        shared["data"]["source_check"] == "identity"
            && first["data"]["source_check"] == "full_hash",
        "the source checks are not one full hash, then identity",
    )?;
    Ok(json!({
        "cold_ms": cold.as_millis(),
        "warm_reused_ms": warm,
        "shared_item": evidence_id(&shared)?,
        "artifacts_after_second_request": committed + 1,
    }))
}

fn stage(name: &str, started: Instant, result: StageResult) -> Value {
    let elapsed_ms = started.elapsed().as_millis();
    match result {
        Ok(evidence) => json!({
            "name": name, "status": "passed", "elapsed_ms": elapsed_ms, "evidence": evidence,
        }),
        Err(StageStop::Failed(diagnostic)) => json!({
            "name": name, "status": "failed", "elapsed_ms": elapsed_ms, "diagnostic": diagnostic,
        }),
        Err(StageStop::Blocked(remediation)) => json!({
            "name": name, "status": "blocked", "elapsed_ms": elapsed_ms, "remediation": remediation,
        }),
    }
}

fn blocked<T>() -> Result<T, StageStop> {
    Err(StageStop::Blocked(MISSING_MEDIA_TOOLS.to_owned()))
}

/// Runs a stage in its own base with the media tools registered.
fn with_tools(
    root: &OwnedRoot,
    name: &str,
    tools: Option<&MediaTools>,
    run: impl FnOnce(&Path, &MediaTools) -> StageResult,
) -> StageResult {
    let Some(tools) = tools else {
        return blocked();
    };
    let base = root.base(name);
    fs::create_dir_all(&base)?;
    prepare(&base, tools)?;
    run(&base, tools)
}

#[tokio::test]
#[ignore = "opt-in P09 evidence checkpoint; needs ffmpeg and ffprobe on PATH; reports to .vsift/e2e-runs"]
async fn evidence_checkpoint() -> TestResult {
    let started = Instant::now();
    let repository = repository().canonicalize()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_dir = repository
        .join(".vsift/e2e-runs")
        .join(format!("p09-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&run_dir)?;
    let root = OwnedRoot::new()?;
    let tools = MediaTools::discover();
    let mut stages = Vec::new();

    let clock = Instant::now();
    let result = with_tools(&root, "exact", tools.as_ref(), frame_exact_stage);
    stages.push(stage("p09_frame_exact", clock, result));

    let clock = Instant::now();
    let result = with_tools(&root, "candidates", tools.as_ref(), |base, _| {
        candidate_frames_stage(base)
    });
    stages.push(stage("p09_candidate_frames", clock, result));

    let clock = Instant::now();
    let result = with_tools(&root, "neighbours", tools.as_ref(), |base, _| {
        neighbours_burst_stage(base)
    });
    stages.push(stage("p09_neighbours_burst", clock, result));

    let clock = Instant::now();
    let result = with_tools(&root, "reuse", tools.as_ref(), |base, _| reuse_stage(base));
    stages.push(stage("p09_reuse", clock, result));

    let status_of = |wanted: &str| stages.iter().any(|entry| entry["status"] == wanted);
    let overall = if status_of("failed") {
        "failed"
    } else if status_of("blocked") {
        "blocked"
    } else {
        "passed"
    };
    stages.push(json!({"name": "p09_evidence", "status": overall}));
    let pending: Vec<_> = [
        "p09_crop",
        "p09_audio",
        "p09_malformed",
        "p09_stream_and_bundle",
        "p09_perf",
    ]
    .into_iter()
    .map(|name| json!({"name": name, "status": "not_implemented", "packet": "P09 PR 4"}))
    .collect();
    let future_stages: Vec<_> = [
        "p10_recovery",
        "p11_worker_batch",
        "p12_agent_clients",
        "p13_distribution",
        "p14_release_qualification",
    ]
    .into_iter()
    .map(|name| json!({"name": name, "status": "not_implemented"}))
    .collect();
    let manifest: Value =
        serde_json::from_slice(&fs::read(repository.join("fixtures/corpus/manifest.json"))?)?;
    let report = json!({
        "schema_version": 1,
        "checkpoint": "P09 evidence navigation",
        "fixture_manifest": {
            "schema_version": manifest["schema_version"],
            "corpus_id": manifest["corpus_id"],
        },
        "fixtures": "F01-F10 and F12, F01-rotation-90.mp4, with the frame lists of fixtures/corpus/generated/verification.json",
        "os": env::consts::OS,
        "architecture": env::consts::ARCH,
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "resource_profile": "each CLI call killed after 300 s; empty PATH; each evidence call under its 120 s deadline",
        "vsift_version": env!("CARGO_PKG_VERSION"),
        "authorization": "opt-in cargo test invocation; setup configure writes only to isolated temporary per-user bases; no install, download or network access",
        "prior_checkpoints": ["P08: p08_search_e2e", "P08: p08_candidates_e2e"],
        "stages": stages,
        "pending_p09_stages": pending,
        "coverage_gaps": [
            "Crops, audio clips, malformed sources, streams and bundles, and performance are PR 4 stages"
        ],
        "future_stages": future_stages,
        "overall": overall,
        "complete_journey": "not_implemented",
        "elapsed_ms": started.elapsed().as_millis(),
    });
    let report_path = run_dir.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("P09 evidence checkpoint report: {}", report_path.display());
    println!("p09_evidence: {overall}");
    if overall == "passed" {
        Ok(())
    } else {
        Err(format!(
            "P09 evidence checkpoint {overall}; see {}",
            report_path.display()
        )
        .into())
    }
}
