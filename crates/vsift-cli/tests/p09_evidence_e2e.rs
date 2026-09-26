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
//!   two requests that resolve to one frame share one item and one file;
//! - `p09_crop` (V-06): crops of the rotated variant equal `FFmpeg`'s decode of
//!   the same displayed region (edges included, one pixel more refused), a
//!   crop of a crop names and shows source pixels, F03's G18 cell turns from
//!   green to red at 4 s, and a tiny glyph keeps its native size;
//! - `p09_audio`: clips report their first decoded sample (F01's audio-only
//!   variant 64 ms, F09 750 ms) as 16 kHz mono, a range past the end is
//!   clipped, and a source without audio, a range over 30 s and one that
//!   starts after the end are refused with their remediation;
//! - `p09_malformed`: F11's damaged audio, F11's truncated file and a video
//!   cut short at run time are `INVALID_SOURCE` with nothing committed, while
//!   their decodable parts stay usable;
//! - `p09_stream_and_bundle`: every command's `--events jsonl` stream, then
//!   `session retain` and `bundle validate` with the evidence records;
//! - `p09_mechanical_journey_supplied` and `p09_mechanical_journey_local_asr`
//!   (the test spine's mechanical checkpoint): one continuous journey per
//!   transcript path over the F03 speech variant, from the video to cited
//!   evidence without an agent: ingest with a `SubRip` file written at run
//!   time from the frozen script and speech placement, or plain ingest then
//!   `transcript retranscribe` (needs `VSIFT_TEST_WHISPER_CLI` and
//!   `VSIFT_TEST_WHISPER_MODEL`, otherwise `blocked`); then `search` for the
//!   critical term, `candidates` within 10 s of the hit, `frame get
//!   --candidate` for a candidate inside the critical event, `crop` of the
//!   changed cell and `audio` over the cited segment, every citation checked
//!   against the frozen truth, and finally `session retain` and `bundle
//!   validate` with the transcript, candidate, frame, crop and clip lineage;
//! - `p09_perf` (recorded, not gated): warm reuse through the binary, cold
//!   frames and a 12-frame burst on a 1080p clip built at run time (about
//!   1 GiB in a release build and 128 MiB in a debug build, which hashes too
//!   slowly for more; `VSIFT_TEST_P09_PERF_MB` sets another size), and the warm cost as
//!   the session's manifest chain grows to 256 generations.
//!
//! Clips are encoded with `FFmpeg`'s native MPEG-4 Part 2 encoder, present in
//! every build including the pinned CI builds that omit libx264.
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
use vsift_contract::{
    AUDIO_RANGE_REMEDIATION, AUDIO_RANGE_START_REMEDIATION, BURST_RANGE_REMEDIATION,
    CROP_OUTSIDE_REMEDIATION, NO_AUDIO_CLIP_REMEDIATION,
};
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

    /// Runs `ffmpeg` with a closed argument list to build a clip.
    fn build(&self, arguments: &[&OsStr]) -> Result<(), StageStop> {
        let output = Process::new(self.ffmpeg.path())
            .args(["-v", "error", "-nostdin", "-y"])
            .args(arguments)
            .output()?;
        ensure(
            output.status.success(),
            &format!(
                "ffmpeg could not build a clip: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        )
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
        let data = if arguments.first() == Some(&"audio") {
            "audio-data.schema.json"
        } else {
            "frame-data.schema.json"
        };
        conforms(data, &result["data"])?;
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

/// The packed RGB of `rect` (`[x, y, width, height]`) inside an image of
/// `width` pixels per row.
fn region(image: &[u8], width: u64, [x, y, w, h]: [u64; 4]) -> Result<Vec<u8>, StageStop> {
    let mut bytes = Vec::new();
    for row in y..y + h {
        let start = usize::try_from((row * width + x) * 3)?;
        let end = start + usize::try_from(w * 3)?;
        bytes.extend_from_slice(
            image
                .get(start..end)
                .ok_or_else(|| failed("a region lies outside the reference image"))?,
        );
    }
    Ok(bytes)
}

/// Mean red, green and blue of packed 8-bit RGB.
fn mean_colour(rgb: &[u8]) -> Result<(u64, u64, u64), StageStop> {
    let pixels = u64::try_from(rgb.len() / 3)?;
    ensure(pixels > 0, "an image is empty")?;
    let mut sums = (0_u64, 0_u64, 0_u64);
    for pixel in rgb.as_chunks::<3>().0 {
        sums.0 += u64::from(pixel[0]);
        sums.1 += u64::from(pixel[1]);
        sums.2 += u64::from(pixel[2]);
    }
    Ok((sums.0 / pixels, sums.1 / pixels, sums.2 / pixels))
}

fn rect_text([x, y, w, h]: [u64; 4]) -> String {
    format!("{x},{y},{w},{h}")
}

/// One successful crop: its result and the decoded pixels of its file.
fn crop_pixels(
    base: &Path,
    tools: &MediaTools,
    session: &str,
    parent: &str,
    rect: [u64; 4],
) -> Result<(Value, Vec<u8>), StageStop> {
    let text = rect_text(rect);
    let (result, _) = evidence_ok(base, &["crop", session, parent, "--rect", &text])?;
    let (_, item) = single(&result)?;
    ensure(
        item["kind"] == "crop"
            && u64_of(&item["image"]["width"])? == rect[2]
            && u64_of(&item["image"]["height"])? == rect[3],
        &format!("the crop {text} is not a native-size crop"),
    )?;
    let pixels = tools.decode_image(&file_path(&result, 0)?)?;
    Ok((result, pixels))
}

/// V-06: crops of the rotated variant equal `FFmpeg`'s decode of the same
/// region, edges and nesting; F03's cell changes colour at 4 s; a tiny glyph
/// keeps its native size.
#[allow(
    clippy::too_many_lines,
    reason = "Every V-06 crop case reads best in one journey"
)]
fn crop_stage(base: &Path, tools: &MediaTools) -> StageResult {
    let source = fixture("F01-rotation-90.mp4");
    let session = ingest(base, &source)?;
    let (frame, _) = evidence_ok(base, &["frame", "get", &session, "--at", "2000000"])?;
    let parent = evidence_id(&frame)?;
    let pts = frame["data"]["items"][0]["frame"]["pts"]
        .as_i64()
        .ok_or_else(|| failed("no pts"))?;
    let reference = tools.decode_reference(&source, pts)?;
    let mut cases = Vec::new();
    for rect in [
        [100, 200, 300, 150],
        [0, 0, 720, 1_280],
        [719, 1_279, 1, 1],
        [420, 1_000, 300, 280],
    ] {
        let (result, pixels) = crop_pixels(base, tools, &session, &parent, rect)?;
        let item = &result["data"]["items"][0];
        ensure(
            pixels == region(&reference, 720, rect)?
                && u64_of(&item["crop"]["frame_x"])? == rect[0]
                && u64_of(&item["crop"]["frame_y"])? == rect[1]
                && item["crop"]["parent_evidence_id"] == parent.as_str()
                && result["data"]["selections"][0]["delta_us"] == 0,
            &format!("the crop {} differs from FFmpeg's decode", rect_text(rect)),
        )?;
        cases.push(json!({"rect": rect_text(rect), "pixel_equal": true}));
    }
    // One pixel past the displayed frame is refused before any tool runs.
    let (code, outside, _) = evidence(
        base,
        &["crop", &session, &parent, "--rect", "421,1000,300,280"],
    )?;
    ensure(
        code == Some(2)
            && outside["error"]["code"] == "INVALID_ARGUMENT"
            && outside["error"]["remediation"][0]["summary"] == CROP_OUTSIDE_REMEDIATION,
        "a crop one pixel past the frame was not refused",
    )?;
    // A crop of a crop names source pixels and equals that source region.
    let (outer, _) = crop_pixels(base, tools, &session, &parent, [100, 200, 300, 150])?;
    ensure(
        outer["data"]["reused"] == true,
        "a repeated crop was not reused",
    )?;
    let outer_id = evidence_id(&outer)?;
    let (nested, pixels) = crop_pixels(base, tools, &session, &outer_id, [10, 20, 50, 40])?;
    let nested_item = &nested["data"]["items"][0];
    ensure(
        nested_item["crop"]["frame_x"] == 110
            && nested_item["crop"]["frame_y"] == 220
            && nested_item["crop"]["x"] == 10
            && nested_item["crop"]["parent_evidence_id"] == outer_id.as_str()
            && pixels == region(&reference, 720, [110, 220, 50, 40])?,
        "a crop of a crop does not name and show its source pixels",
    )?;

    // F03's G18 cell: green before 4 s, red from 4 s.
    let f03 = ingest(base, &fixture("F03.mp4"))?;
    let mut colours = Vec::new();
    for at in ["3900000", "4000000"] {
        let (cell_frame, _) = evidence_ok(
            base,
            &["frame", "get", &f03, "--at", at, "--tolerance-us", "0"],
        )?;
        let (_, pixels) = crop_pixels(
            base,
            tools,
            &f03,
            &evidence_id(&cell_frame)?,
            [850, 420, 280, 70],
        )?;
        colours.push(mean_colour(&pixels)?);
    }
    let [(red_before, green_before, _), (red_after, green_after, _)] = colours[..] else {
        return Err(failed("expected two cell crops"));
    };
    ensure(
        green_before > red_before + 40 && red_after > green_after + 40,
        &format!("the G18 cell did not turn from green to red: {colours:?}"),
    )?;

    // A tiny glyph (the first 4x-scaled 5x7 letter of F01's title, 20x28
    // pixels) keeps its native size: nothing is scaled or invented.
    let f01 = ingest(base, &fixture("F01.mp4"))?;
    let (title_frame, _) = evidence_ok(base, &["frame", "get", &f01, "--at", "1000000"])?;
    let title_pts = title_frame["data"]["items"][0]["frame"]["pts"]
        .as_i64()
        .ok_or_else(|| failed("no pts"))?;
    let f01_reference = tools.decode_reference(&fixture("F01.mp4"), title_pts)?;
    let glyph = [35, 20, 20, 28];
    let (_, glyph_pixels) = crop_pixels(base, tools, &f01, &evidence_id(&title_frame)?, glyph)?;
    let bright = glyph_pixels
        .as_chunks::<3>()
        .0
        .iter()
        .filter(|pixel| pixel.iter().all(|channel| *channel > 200))
        .count();
    ensure(
        glyph_pixels == region(&f01_reference, 1_280, glyph)?
            && bright > 0
            && bright < glyph_pixels.len() / 3,
        "the tiny glyph crop is not the glyph at native size",
    )?;
    Ok(json!({
        "rotated_crops": cases,
        "outside_by_one_pixel": outside["error"]["code"],
        "crop_of_crop": {"frame_x": 110, "frame_y": 220, "pixel_equal": true},
        "f03_g18_mean_rgb": {"at_3_9_s": colours[0], "at_4_0_s": colours[1]},
        "tiny_glyph": {"rect": rect_text(glyph), "native_size": true, "pixel_equal": true, "glyph_pixels": bright},
    }))
}

/// The format of a WAV clip: channels, sample rate and bits per sample, and
/// its number of samples.
fn wav_format(bytes: &[u8]) -> Result<(u16, u32, u16, usize), StageStop> {
    let field = |range: std::ops::Range<usize>| {
        bytes
            .get(range)
            .ok_or_else(|| failed("the clip is shorter than a WAV header"))
    };
    ensure(
        field(0..4)? == b"RIFF" && field(8..12)? == b"WAVE",
        "the clip is not a WAV file",
    )?;
    let channels = u16::from_le_bytes(field(22..24)?.try_into()?);
    let rate = u32::from_le_bytes(field(24..28)?.try_into()?);
    let bits = u16::from_le_bytes(field(34..36)?.try_into()?);
    Ok((channels, rate, bits, bytes.len().saturating_sub(44) / 2))
}

/// V-01 for sound: clips report their first decoded sample, are clipped to
/// the source and refuse what has no audio or is too long.
fn audio_stage(base: &Path) -> StageResult {
    let mut clips = Vec::new();
    for (file, to, expected_start) in [
        ("F01-audio-only.m4a", 1_000_000_u64, 64_000_u64),
        ("F09.mkv", 2_000_000, 750_000),
    ] {
        let session = ingest(base, &fixture(file))?;
        let to_text = to.to_string();
        let (result, elapsed) =
            evidence_ok(base, &["audio", &session, "--from", "0", "--to", &to_text])?;
        let data = &result["data"];
        let item = &data["items"][0];
        let (channels, rate, bits, samples) = wav_format(&fs::read(file_path(&result, 0)?)?)?;
        let requested = usize::try_from(to * 16 / 1_000)?;
        ensure(
            u64_of(&item["actual_start_us"])? == expected_start
                && u64_of(&data["selections"][0]["actual_us"])? == expected_start
                && data["range_clipped"] == false
                && (channels, rate, bits) == (1, 16_000, 16)
                && samples <= requested
                && samples + requested / 2 >= requested,
            &format!("{file}: the clip does not start at {expected_start} us as 16 kHz mono"),
        )?;
        clips.push(json!({"fixture": file, "actual_start_us": expected_start, "samples": samples, "ms": elapsed.as_millis()}));
    }
    let session = ingest(base, &fixture("F01-audio-only.m4a"))?;
    let (clipped, _) = evidence_ok(
        base,
        &["audio", &session, "--from", "5000000", "--to", "8000000"],
    )?;
    let end = u64_of(&clipped["data"]["items"][0]["range"]["end_us"])?;
    ensure(
        clipped["data"]["range_clipped"] == true && end < 8_000_000 && end > 5_000_000,
        "a range past the end of the audio was not clipped to it",
    )?;
    let mut refusals = Vec::new();
    for (file, from, to, summary) in [
        ("F10.mp4", "0", "1000000", NO_AUDIO_CLIP_REMEDIATION),
        (
            "F01-audio-only.m4a",
            "0",
            "30000001",
            AUDIO_RANGE_REMEDIATION,
        ),
        (
            "F01-audio-only.m4a",
            "7000000",
            "8000000",
            AUDIO_RANGE_START_REMEDIATION,
        ),
    ] {
        let target = ingest(base, &fixture(file))?;
        let (code, result, _) = evidence(base, &["audio", &target, "--from", from, "--to", to])?;
        ensure(
            code == Some(2)
                && result["error"]["code"] == "INVALID_ARGUMENT"
                && result["error"]["remediation"][0]["summary"] == summary
                && artifact_count(base, &target)? == 0,
            &format!("{file} {from}-{to} was not refused with its remediation"),
        )?;
        refusals.push(
            json!({"fixture": file, "from_us": from, "to_us": to, "code": result["error"]["code"]}),
        );
    }
    Ok(json!({"clips": clips, "clipped_end_us": end, "refusals": refusals}))
}

/// Damaged and cut-short media: typed `INVALID_SOURCE` with nothing
/// committed; the decodable part stays usable.
fn malformed_stage(base: &Path) -> StageResult {
    let mut outcomes = Vec::new();
    let damaged = ingest(base, &fixture("F11-damaged-tail.mp4"))?;
    let truncated_fixture = ingest(base, &fixture("F11-truncated.mp4"))?;
    let bytes = fs::read(fixture("F05.mp4"))?;
    let cut = base.join("truncated-at-run-time.mp4");
    fs::write(&cut, bytes.get(..bytes.len() * 6 / 10).unwrap_or_default())?;
    let cut_session = ingest(base, &cut)?;
    for (label, session, arguments) in [
        (
            "f11_damaged_tail_audio",
            &damaged,
            vec!["audio", "--from", "0", "--to", "1000000"],
        ),
        (
            "f11_truncated_frame",
            &truncated_fixture,
            vec!["frame", "get", "--at", "0"],
        ),
        (
            "cut_frame_past_the_data",
            &cut_session,
            vec!["frame", "get", "--at", "18000000"],
        ),
        (
            "cut_burst_across_the_data",
            &cut_session,
            vec!["frame", "burst", "--from", "0", "--to", "20000000"],
        ),
    ] {
        let before = artifact_count(base, session)?;
        // The session identity follows the command words.
        let split = if arguments[0] == "frame" { 2 } else { 1 };
        let mut full: Vec<&str> = arguments[..split].to_vec();
        full.push(session);
        full.extend_from_slice(&arguments[split..]);
        let (code, result, _) = evidence(base, &full)?;
        ensure(
            code == Some(3)
                && result["error"]["code"] == "INVALID_SOURCE"
                && artifact_count(base, session)? == before,
            &format!(
                "{label}: {} (exit {code:?}) or something was committed",
                result["error"]["code"]
            ),
        )?;
        outcomes.push(json!({"case": label, "code": "INVALID_SOURCE", "remediation": result["error"]["remediation"][0]["summary"].is_string()}));
    }
    // What decodes stays usable: the damaged file's video and the cut
    // file's first second.
    evidence_ok(base, &["frame", "get", &damaged, "--at", "0"])?;
    evidence_ok(base, &["frame", "get", &cut_session, "--at", "1000000"])?;
    Ok(json!({"refusals": outcomes, "decodable_parts_usable": true}))
}

/// Splits a JSON Lines stream into validated lines.
fn stream_lines(base: &Path, arguments: &[&str], command: &str) -> Result<Vec<Value>, StageStop> {
    let output = vsift(base)?
        .args(arguments)
        .args(["--events", "jsonl"])
        .output()?;
    ensure(
        output.status.code() == Some(0) && output.stderr.is_empty(),
        &format!("the {command} stream failed"),
    )?;
    let lines = std::str::from_utf8(&output.stdout)?
        .trim_end_matches('\n')
        .split('\n')
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    let (terminal, records) = lines
        .split_last()
        .ok_or_else(|| failed("the stream is empty"))?;
    let (record_schema, data_schema) = if command == "audio" {
        (
            "audio-evidence.schema.json",
            "audio-stream-data.schema.json",
        )
    } else {
        (
            "frame-evidence.schema.json",
            "frame-stream-data.schema.json",
        )
    };
    for (sequence, record) in (0_u64..).zip(records) {
        conforms("evidence-event.schema.json", record)?;
        conforms(record_schema, &record["record"])?;
        ensure(
            record["sequence"].as_u64() == Some(sequence)
                && record["command"] == command
                && record["key"] == record["record"]["evidence_id"],
            "an evidence event is out of sequence or mis-keyed",
        )?;
    }
    conforms("terminal-event.schema.json", terminal)?;
    conforms(data_schema, &terminal["result"]["data"])?;
    ensure(
        terminal["result"]["data"]["record_count"].as_u64() == u64::try_from(records.len()).ok()
            && terminal["sequence"].as_u64() == u64::try_from(records.len()).ok(),
        "the terminal event does not count the records",
    )?;
    Ok(lines)
}

/// Every command streams, then the session is retained and its bundle,
/// evidence records included, validates.
#[allow(
    clippy::too_many_lines,
    reason = "Every stream and the bundle checks read best in one journey"
)]
fn stream_and_bundle_stage(base: &Path) -> StageResult {
    let session = ingest(base, &fixture("F01.mp4"))?;
    let get = stream_lines(
        base,
        &["frame", "get", &session, "--at", "1025000"],
        "frame.get",
    )?;
    let anchor = get[0]["key"]
        .as_str()
        .ok_or_else(|| failed("no key"))?
        .to_owned();
    let mut counts = Vec::new();
    counts.push(json!({"command": "frame.get", "events": get.len()}));
    for (arguments, command) in [
        (
            vec![
                "frame",
                "neighbours",
                session.as_str(),
                anchor.as_str(),
                "--count",
                "2",
            ],
            "frame.neighbours",
        ),
        (
            vec![
                "frame",
                "burst",
                session.as_str(),
                "--from",
                "0",
                "--to",
                "6000000",
                "--max-frames",
                "4",
            ],
            "frame.burst",
        ),
        (
            vec![
                "crop",
                session.as_str(),
                anchor.as_str(),
                "--rect",
                "150,235,550,55",
            ],
            "crop",
        ),
        (
            vec!["audio", session.as_str(), "--from", "0", "--to", "2000000"],
            "audio",
        ),
    ] {
        let lines = stream_lines(base, &arguments, command)?;
        counts.push(json!({"command": command, "events": lines.len()}));
    }
    let bundle = base.join("bundle");
    let (code, retained) = run_json(
        vsift(base)?
            .args(["session", "retain", &session, "--output"])
            .arg(&bundle)
            .arg("--json"),
    )?;
    ensure(
        code == Some(0),
        &format!("session retain failed: {}", retained["error"]["code"]),
    )?;
    let (code, validated) = run_json(
        vsift(base)?
            .args(["bundle", "validate"])
            .arg(&bundle)
            .arg("--json"),
    )?;
    ensure(code == Some(0), "bundle validate failed")?;
    let manifest: Value = serde_json::from_slice(&fs::read(bundle.join("bundle.json"))?)?;
    let mut kinds = std::collections::BTreeMap::<String, u64>::new();
    let root_text = base.to_string_lossy().to_string();
    for artifact in manifest["artifacts"]
        .as_array()
        .ok_or_else(|| failed("artifacts missing"))?
    {
        let name = artifact["name"].as_str().unwrap_or_default();
        let kind = artifact["kind"].as_str().unwrap_or_default().to_owned();
        *kinds.entry(kind.clone()).or_default() += 1;
        if kind == "evidence_record" {
            let text = fs::read_to_string(bundle.join(name))?;
            ensure(
                !text.contains(&root_text) && !text.contains("\"path\""),
                "an evidence record holds a path",
            )?;
            conforms(
                "bundle-evidence-record.schema.json",
                &serde_json::from_str(&text)?,
            )?;
        }
    }
    ensure(
        kinds.get("evidence_record") == Some(&5)
            && kinds.contains_key("frame_png")
            && kinds.get("audio_wav") == Some(&1),
        &format!("the bundle does not carry the evidence: {kinds:?}"),
    )?;
    Ok(json!({
        "streams": counts,
        "bundle_artifacts": validated["data"]["artifact_count"],
        "bundle_artifact_kinds": kinds,
    }))
}

fn p95(durations: &[Duration]) -> Duration {
    let mut sorted = durations.to_vec();
    sorted.sort_unstable();
    let rank = (sorted.len() * 95).div_ceil(100).saturating_sub(1);
    sorted.get(rank).copied().unwrap_or_default()
}

fn millis(durations: &[Duration]) -> Vec<u128> {
    durations.iter().map(Duration::as_millis).collect()
}

/// Performance record (not a gate): warm reuse, cold frames and a burst on
/// a large 1080p clip built at run time, and the manifest-chain walk.
#[allow(
    clippy::too_many_lines,
    reason = "The measurements and their record read best together"
)]
fn perf_stage(base: &Path, tools: &MediaTools) -> StageResult {
    // Warm reuse on F01.
    let f01 = ingest(base, &fixture("F01.mp4"))?;
    evidence_ok(base, &["frame", "get", &f01, "--at", "1025000"])?;
    let mut warm = Vec::new();
    for _ in 0..20 {
        warm.push(evidence_ok(base, &["frame", "get", &f01, "--at", "1025000"])?.1);
    }

    // A 1080p clip: F07 (1920x1080) with temporal noise so it does not
    // compress, 60 s encoded once with FFmpeg's native MPEG-4 encoder, then
    // looped by stream copy to the target size.
    // A debug build hashes too slowly for a gigabyte inside the CLI deadline.
    let target_mb: u64 = env::var("VSIFT_TEST_P09_PERF_MB")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(if cfg!(debug_assertions) { 128 } else { 1_024 });
    let built = Instant::now();
    let segment = base.join("segment-1080p.mp4");
    tools.build(&[
        OsStr::new("-stream_loop"),
        OsStr::new("2"),
        OsStr::new("-i"),
        fixture("F07.mp4").as_os_str(),
        OsStr::new("-map"),
        OsStr::new("0:v:0"),
        OsStr::new("-vf"),
        OsStr::new("noise=alls=10:allf=t"),
        OsStr::new("-t"),
        OsStr::new("30"),
        OsStr::new("-c:v"),
        OsStr::new("mpeg4"),
        OsStr::new("-q:v"),
        OsStr::new("5"),
        OsStr::new("-g"),
        OsStr::new("40"),
        OsStr::new("-an"),
        segment.as_os_str(),
    ])?;
    let segment_bytes = fs::metadata(&segment)?.len();
    let loops = (target_mb * 1_048_576)
        .div_ceil(segment_bytes.max(1))
        .max(1);
    let clip = base.join("large-1080p.mp4");
    let repeat = (loops - 1).to_string();
    tools.build(&[
        OsStr::new("-stream_loop"),
        OsStr::new(&repeat),
        OsStr::new("-i"),
        segment.as_os_str(),
        OsStr::new("-c"),
        OsStr::new("copy"),
        clip.as_os_str(),
    ])?;
    fs::remove_file(&segment)?;
    let clip_bytes = fs::metadata(&clip)?.len();
    let build_ms = built.elapsed().as_millis();
    let ingested = Instant::now();
    let large = ingest(base, &clip)?;
    let ingest_ms = ingested.elapsed().as_millis();
    fs::remove_file(&clip)?;
    let duration_s = loops * 30;
    let burst_end = (duration_s.min(60) * SECOND).to_string();
    // The first call hashes the 1 GB copy in full (D1); later calls compare
    // its identity.
    let (_, first_call) = evidence_ok(base, &["frame", "get", &large, "--at", "1000000"])?;
    let mut cold = Vec::new();
    for index in 1..=10_u64 {
        let at = (index * duration_s * SECOND / 11).to_string();
        let (result, elapsed) = evidence_ok(base, &["frame", "get", &large, "--at", &at])?;
        ensure(
            result["data"]["reused"] == false
                && result["data"]["items"][0]["image"]["width"] == 1_920,
            "a cold 1080p frame was reused or not 1920 wide",
        )?;
        cold.push(elapsed);
    }
    let (burst, burst_time) = evidence_ok(
        base,
        &["frame", "burst", &large, "--from", "0", "--to", &burst_end],
    )?;
    ensure(
        burst["data"]["items"].as_array().map(Vec::len) == Some(12),
        "the 12-frame 1080p burst did not return 12 frames",
    )?;
    let mut large_warm = Vec::new();
    for _ in 0..10 {
        large_warm.push(evidence_ok(base, &["frame", "get", &large, "--at", "1000000"])?.1);
    }

    // The manifest chain: every read walks it, so a warm call's cost grows
    // with the session's generations. Renewals add one generation each.
    let chain = ingest(base, &fixture("F01.mp4"))?;
    evidence_ok(base, &["frame", "get", &chain, "--at", "1025000"])?;
    let mut walk = Vec::new();
    let mut generation = 0_u64;
    for target in [2_u64, 64, 128, 256] {
        while generation < target {
            let (code, renewed) =
                run_json(vsift(base)?.args(["session", "renew", &chain, "--json"]))?;
            ensure(code == Some(0), "a renewal failed")?;
            generation = u64_of(&renewed["data"]["generation"])?;
        }
        let mut samples = Vec::new();
        for _ in 0..5 {
            samples.push(evidence_ok(base, &["frame", "get", &chain, "--at", "1025000"])?.1);
        }
        walk.push(json!({"generation": generation, "warm_reuse_p95_ms": p95(&samples).as_millis(), "warm_reuse_ms": millis(&samples)}));
    }
    let warm_p95 = p95(&warm);
    Ok(json!({
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "warm_reuse_f01": {"p95_ms": warm_p95.as_millis(), "target_ms": 250, "meets_target": warm_p95 <= Duration::from_millis(250), "samples_ms": millis(&warm)},
        "large_clip": {
            "recipe": "F07 1920x1080 with noise=alls=10:allf=t (about 39 Mbit/s), a 30 s segment encoded with mpeg4 -q:v 5 -g 40, looped by stream copy",
            "bytes": clip_bytes,
            "duration_s": duration_s,
            "build_ms": build_ms,
            "ingest_ms": ingest_ms,
            "first_call_with_full_hash_ms": first_call.as_millis(),
            "cold_frame_get_p95_ms": p95(&cold).as_millis(),
            "cold_frame_get_ms": millis(&cold),
            "burst_12_frames_ms": burst_time.as_millis(),
            "burst_range_us": [0, duration_s.min(60) * SECOND],
            "warm_reuse_p95_ms": p95(&large_warm).as_millis(),
        },
        "manifest_chain_walk": walk,
    }))
}

/// The optional local-ASR path's recognizer and model.
struct Whisper {
    cli: PathBuf,
    model: PathBuf,
}

impl Whisper {
    fn discover() -> Option<Self> {
        let absolute = |name: &str| {
            env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute() && path.is_file())
        };
        Some(Self {
            cli: absolute("VSIFT_TEST_WHISPER_CLI")?,
            model: absolute("VSIFT_TEST_WHISPER_MODEL")?,
        })
    }
}

/// The journey's fixture: F03's speech variant, the term the script says
/// when the cell changes, the critical event that shows it, and the cell.
const JOURNEY_FIXTURE: &str = "F03";
const JOURNEY_FILE: &str = "F03-speech.mp4";
const JOURNEY_TERM: &str = "127.50";
const JOURNEY_EVENT: &str = "F03-E02";
/// Cell G18, drawn at (850, 420) with size 280x70 by the frozen generator
/// recipe (`tools/generate_p04_fixtures.py`) and checked by the fixture
/// verifier's "F03 G18 fill" pixel check; the manifest names the cell but
/// holds no geometry.
const JOURNEY_CELL: [u64; 4] = [850, 420, 280, 70];
/// How far either side of a search hit an agent looks for candidates.
const LEAD_LAG_US: u64 = 10 * SECOND;
/// Local speech recognition places segments within this much of the
/// generator's speech span (the P07 local-ASR checkpoint's tolerance).
const ASR_SPAN_TOLERANCE_US: u64 = SECOND;

/// Which transcript path a journey takes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TranscriptPath {
    /// A `SubRip` file written at run time from the frozen script.
    Supplied,
    /// whisper.cpp through `transcript retranscribe`.
    LocalAsr,
}

impl TranscriptPath {
    const fn label(self) -> &'static str {
        match self {
            Self::Supplied => "supplied_transcript",
            Self::LocalAsr => "local_asr",
        }
    }

    /// How far a segment may lie outside the speech span.
    const fn tolerance(self) -> u64 {
        match self {
            Self::Supplied => 0,
            Self::LocalAsr => ASR_SPAN_TOLERANCE_US,
        }
    }
}

/// The frozen truth one journey is checked against.
struct JourneyTruth {
    duration: u64,
    speech: (u64, u64),
    term_words: (u64, u64),
    event: (u64, u64),
    script: String,
}

fn manifest_fixture(manifest: &Value, id: &str) -> Result<Value, StageStop> {
    manifest["fixtures"]
        .as_array()
        .and_then(|fixtures| fixtures.iter().find(|entry| entry["id"] == id))
        .cloned()
        .ok_or_else(|| failed("fixture missing from the manifest"))
}

fn journey_truth() -> Result<JourneyTruth, StageStop> {
    let manifest: Value = serde_json::from_slice(&fs::read(
        repository().join("fixtures/corpus/manifest.json"),
    )?)?;
    let entry = manifest_fixture(&manifest, JOURNEY_FIXTURE)?;
    let event = entry["events"]
        .as_array()
        .and_then(|events| events.iter().find(|event| event["id"] == JOURNEY_EVENT))
        .ok_or_else(|| failed("the journey's event is missing from the manifest"))?;
    ensure(
        event["critical"] == true,
        "the journey's event is not critical",
    )?;
    let provenance: Value = serde_json::from_slice(&fs::read(fixture("speech-provenance.json"))?)?;
    let variant = provenance["assembly"]["variants"]
        .as_array()
        .and_then(|variants| {
            variants
                .iter()
                .find(|variant| variant["fixture"] == JOURNEY_FIXTURE)
        })
        .ok_or_else(|| failed("the journey's fixture is missing from speech provenance"))?;
    let word = variant["tts_word_timings"]
        .as_array()
        .and_then(|words| words.iter().find(|word| word["text"] == JOURNEY_TERM))
        .ok_or_else(|| failed("the term has no word timing"))?;
    Ok(JourneyTruth {
        duration: u64_of(&entry["duration_us"])?,
        speech: (
            u64_of(&variant["speech_start_us"])?,
            u64_of(&variant["speech_end_us"])?,
        ),
        term_words: (u64_of(&word["start_us"])?, u64_of(&word["end_us"])?),
        event: (u64_of(&event["start_us"])?, u64_of(&event["end_us"])?),
        script: entry["audio"]["script"]
            .as_str()
            .ok_or_else(|| failed("no script"))?
            .to_owned(),
    })
}

/// A `SubRip` file of one cue: the frozen script over the generator's speech
/// span, as an agent's supplied transcript would be.
fn write_srt(path: &Path, truth: &JourneyTruth) -> Result<(), StageStop> {
    let stamp = |micros: u64| {
        let millis = micros / 1_000;
        format!(
            "{:02}:{:02}:{:02},{:03}",
            millis / 3_600_000,
            millis / 60_000 % 60,
            millis / 1_000 % 60,
            millis % 1_000
        )
    };
    fs::write(
        path,
        format!(
            "1\n{} --> {}\n{}\n",
            stamp(truth.speech.0),
            stamp(truth.speech.1),
            truth.script
        ),
    )?;
    Ok(())
}

/// Registers whisper.cpp and its model next to the media tools.
fn prepare_whisper(base: &Path, whisper: &Whisper) -> Result<(), StageStop> {
    let (code, _) = run_json(
        vsift(base)?
            .args(["setup", "configure", "whisper", "--executable"])
            .arg(&whisper.cli)
            .arg("--json"),
    )?;
    ensure(code == Some(0), "setup configure whisper did not succeed")?;
    let (code, _) = run_json(
        vsift(base)?
            .args(["setup", "configure-model", "--file"])
            .arg(&whisper.model)
            .arg("--json"),
    )?;
    ensure(code == Some(0), "setup configure-model did not succeed")
}

fn timed_json(command: &mut Command) -> Result<(Option<i32>, Value, Duration), StageStop> {
    let started = Instant::now();
    let (code, value) = run_json(command)?;
    Ok((code, value, started.elapsed()))
}

/// Opens the session by the path's route: ingest with the `SubRip` file, or
/// plain ingest then local speech recognition.
fn open_journey(
    base: &Path,
    path: TranscriptPath,
    truth: &JourneyTruth,
) -> Result<(String, Value), StageStop> {
    let video = fixture(JOURNEY_FILE);
    let mut command = vsift(base)?;
    command.arg("ingest").arg(&video);
    if path == TranscriptPath::Supplied {
        let srt = base.join("F03-speech.srt");
        write_srt(&srt, truth)?;
        command
            .arg("--transcript")
            .arg(&srt)
            .args(["--transcript-offset", "0"]);
    }
    let (code, opened, ingest_time) = timed_json(command.arg("--json"))?;
    ensure(
        code == Some(0),
        &format!("ingest failed: {}", opened["error"]["code"]),
    )?;
    let session = opened["data"]["session_id"]
        .as_str()
        .ok_or_else(|| failed("session missing"))?
        .to_owned();
    let mut timings = json!({"ingest_ms": ingest_time.as_millis()});
    if path == TranscriptPath::LocalAsr {
        let (code, result, elapsed) = timed_json(
            vsift(base)?
                .args(["transcript", "retranscribe", &session])
                .arg("--json"),
        )?;
        ensure(
            code == Some(0),
            &format!(
                "transcript retranscribe failed: {}",
                result["error"]["code"]
            ),
        )?;
        conforms("transcript-retranscribe-data.schema.json", &result["data"])?;
        timings["retranscribe_ms"] = json!(elapsed.as_millis());
    }
    Ok((session, timings))
}

/// Every artifact of a retained bundle by kind; evidence records conform
/// to their bundle schema and hold no path.
fn bundle_records(bundle: &Path, kind: &str) -> Result<Vec<Value>, StageStop> {
    let manifest: Value = serde_json::from_slice(&fs::read(bundle.join("bundle.json"))?)?;
    let mut records = Vec::new();
    for artifact in manifest["artifacts"]
        .as_array()
        .ok_or_else(|| failed("artifacts missing"))?
    {
        if artifact["kind"] != kind {
            continue;
        }
        let name = artifact["name"].as_str().unwrap_or_default();
        ensure(
            name.starts_with("artifact-") && !name.contains(['/', '\\']),
            "a bundle artifact has an unexpected name",
        )?;
        records.push(serde_json::from_slice(&fs::read(bundle.join(name))?)?);
    }
    Ok(records)
}

/// One continuous journey from the video to cited, validated evidence.
#[allow(
    clippy::too_many_lines,
    reason = "The journey's steps and their checks read best in one place"
)]
fn mechanical_journey(base: &Path, tools: &MediaTools, path: TranscriptPath) -> StageResult {
    let truth = journey_truth()?;
    let started = Instant::now();
    let (session, mut timings) = open_journey(base, path, &truth)?;

    // 1. Search: the segment that says the term, inside the speech span and
    // over the term's words.
    let (code, search, elapsed) =
        timed_json(vsift(base)?.args(["search", &session, "--query", JOURNEY_TERM, "--json"]))?;
    timings["search_ms"] = json!(elapsed.as_millis());
    ensure(code == Some(0), "search failed")?;
    conforms("search-data.schema.json", &search["data"])?;
    let segment = &search["data"]["items"][0];
    let (segment_start, segment_end) = (u64_of(&segment["start_us"])?, u64_of(&segment["end_us"])?);
    let tolerance = path.tolerance();
    ensure(
        segment_start + tolerance >= truth.speech.0
            && segment_end <= truth.speech.1 + tolerance
            && segment_start < truth.term_words.1 + tolerance
            && segment_end + tolerance > truth.term_words.0,
        &format!(
            "the cited segment [{segment_start}, {segment_end}) is not inside the speech span {:?} over the term {:?}",
            truth.speech, truth.term_words
        ),
    )?;

    // 2. Candidates within the lead/lag window of the hit.
    let window = (
        segment_start.saturating_sub(LEAD_LAG_US),
        (segment_start + LEAD_LAG_US).min(truth.duration),
    );
    let (code, page, elapsed) = timed_json(
        vsift(base)?
            .args(["candidates", &session, "--from"])
            .arg(window.0.to_string())
            .arg("--to")
            .arg(window.1.to_string())
            .args(["--limit", "100", "--json"]),
    )?;
    timings["candidates_ms"] = json!(elapsed.as_millis());
    ensure(code == Some(0), "candidates failed")?;
    conforms("candidates-data.schema.json", &page["data"])?;
    let candidate = page["data"]["items"]
        .as_array()
        .and_then(|items| {
            items.iter().find(|item| {
                item["representative_us"]
                    .as_u64()
                    .is_some_and(|time| (truth.event.0..truth.event.1).contains(&time))
            })
        })
        .ok_or_else(|| failed("no candidate within the lead/lag window lies inside the event"))?
        .clone();
    let candidate_id = candidate["candidate_id"]
        .as_str()
        .ok_or_else(|| failed("no candidate id"))?
        .to_owned();
    let representative = u64_of(&candidate["representative_us"])?;

    // 3. The candidate's own frame.
    let (frame, elapsed) = evidence_ok(
        base,
        &["frame", "get", &session, "--candidate", &candidate_id],
    )?;
    timings["frame_get_ms"] = json!(elapsed.as_millis());
    let (selection, frame_item) = single(&frame)?;
    let frame_time = u64_of(&selection["actual_us"])?;
    ensure(
        frame["data"]["request"]["candidate_id"] == candidate_id.as_str()
            && selection["delta_us"] == 0
            && frame_time == representative
            && (truth.event.0..truth.event.1).contains(&frame_time)
            && frame_item["image"]["width"] == 1_440
            && frame_item["image"]["height"] == 900,
        "the candidate's frame is not its own 1440x900 frame inside the event",
    )?;
    let frame_id = evidence_id(&frame)?;

    // 4. The cell, cropped from that frame.
    let [x, y, width, height] = JOURNEY_CELL;
    let rect = rect_text(JOURNEY_CELL);
    let (crop, elapsed) = evidence_ok(base, &["crop", &session, &frame_id, "--rect", &rect])?;
    timings["crop_ms"] = json!(elapsed.as_millis());
    let (crop_selection, crop_item) = single(&crop)?;
    ensure(
        crop_item["crop"]["parent_evidence_id"] == frame_id.as_str()
            && u64_of(&crop_item["crop"]["frame_x"])? == x
            && u64_of(&crop_item["crop"]["frame_y"])? == y
            && u64_of(&crop_item["image"]["width"])? == width
            && u64_of(&crop_item["image"]["height"])? == height
            && u64_of(&crop_selection["actual_us"])? == frame_time,
        "the crop does not name its frame and the cell's region",
    )?;
    let (red, green, blue) = mean_colour(&tools.decode_image(&file_path(&crop, 0)?)?)?;
    ensure(
        red > green + 40,
        &format!("the cell is not red after the change: ({red}, {green}, {blue})"),
    )?;

    // 5. The audio of the cited segment (at most 30 s).
    let audio_to = segment_end.min(segment_start + 30 * SECOND);
    let (audio, elapsed) = evidence_ok(
        base,
        &[
            "audio",
            &session,
            "--from",
            &segment_start.to_string(),
            "--to",
            &audio_to.to_string(),
        ],
    )?;
    timings["audio_ms"] = json!(elapsed.as_millis());
    let clip = &audio["data"]["items"][0];
    let actual_start = u64_of(&clip["actual_start_us"])?;
    let (channels, rate, bits, samples) = wav_format(&fs::read(file_path(&audio, 0)?)?)?;
    let expected_samples = usize::try_from((audio_to - segment_start) * 16 / 1_000)?;
    ensure(
        u64_of(&clip["range"]["start_us"])? == segment_start
            && u64_of(&clip["range"]["end_us"])? == audio_to
            && audio["data"]["range_clipped"] == false
            && actual_start >= segment_start
            && actual_start < segment_start + 100_000
            && (channels, rate, bits) == (1, 16_000, 16)
            && samples <= expected_samples
            && samples + expected_samples / 10 >= expected_samples,
        "the clip does not cover the cited segment from its first sample",
    )?;

    // 6. Retain the whole session and validate it, lineage included.
    let bundle = base.join("bundle");
    let (code, retained, elapsed) = timed_json(
        vsift(base)?
            .args(["session", "retain", &session, "--output"])
            .arg(&bundle)
            .arg("--json"),
    )?;
    ensure(
        code == Some(0),
        &format!("session retain failed: {}", retained["error"]["code"]),
    )?;
    let (code, validated, validate_time) = timed_json(
        vsift(base)?
            .args(["bundle", "validate"])
            .arg(&bundle)
            .arg("--json"),
    )?;
    ensure(code == Some(0), "bundle validate failed")?;
    timings["retain_ms"] = json!(elapsed.as_millis());
    timings["bundle_validate_ms"] = json!(validate_time.as_millis());
    let transcripts = bundle_records(&bundle, "transcript_record")?;
    let indexes = bundle_records(&bundle, "visual_index_record")?;
    let evidence_records = bundle_records(&bundle, "evidence_record")?;
    for record in &transcripts {
        conforms("bundle-transcript-record.schema.json", record)?;
    }
    for record in &indexes {
        conforms("bundle-visual-index-record.schema.json", record)?;
    }
    let root_text = base.to_string_lossy().to_string();
    for record in &evidence_records {
        conforms("bundle-evidence-record.schema.json", record)?;
        let text = record.to_string();
        ensure(
            !text.contains(&root_text) && !text.contains("\"path\""),
            "an evidence record holds a path",
        )?;
    }
    let candidate_indexed = indexes.iter().any(|index| {
        index["windows"].as_array().is_some_and(|windows| {
            windows.iter().any(|window| {
                window["candidates"].as_array().is_some_and(|candidates| {
                    candidates.iter().any(|entry| {
                        entry["id"] == candidate_id.as_str()
                            && entry["representative_us"] == representative
                    })
                })
            })
        })
    });
    let frame_linked = evidence_records.iter().any(|record| {
        record["request"]["frame_get"]["candidate_id"] == candidate_id.as_str()
            && record["items"][0]["evidence_id"] == frame_id.as_str()
    });
    let crop_linked = evidence_records.iter().any(|record| {
        record["request"]["crop"]["parent_evidence_id"] == frame_id.as_str()
            && record["items"][0]["subject"]["crop"]["region"]["parent_evidence_id"]
                == frame_id.as_str()
    });
    ensure(
        transcripts.len() == 1
            && evidence_records.len() == 3
            && candidate_indexed
            && frame_linked
            && crop_linked,
        "the bundle does not carry the transcript, the candidate and the frame, crop and clip lineage",
    )?;
    timings["total_ms"] = json!(started.elapsed().as_millis());
    Ok(json!({
        "path": path.label(),
        "fixture": JOURNEY_FILE,
        "term": JOURNEY_TERM,
        "segment_us": [segment_start, segment_end],
        "speech_span_us": [truth.speech.0, truth.speech.1],
        "term_words_us": [truth.term_words.0, truth.term_words.1],
        "candidate": {"id": candidate_id, "representative_us": representative, "window_us": [window.0, window.1]},
        "event": {"id": JOURNEY_EVENT, "window_us": [truth.event.0, truth.event.1]},
        "frame": {"evidence_id": frame_id, "actual_us": frame_time, "delta_us": 0},
        "crop": {"rect": rect, "mean_rgb": [red, green, blue], "parent": "the frame"},
        "audio": {"range_us": [segment_start, audio_to], "actual_start_us": actual_start, "samples": samples},
        "bundle": {"artifacts": validated["data"]["artifact_count"], "evidence_records": evidence_records.len()},
        "timings": timings,
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
#[allow(
    clippy::too_many_lines,
    reason = "Keep the journey order and the complete evidence record visible together"
)]
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

    let clock = Instant::now();
    let result = with_tools(&root, "crop", tools.as_ref(), crop_stage);
    stages.push(stage("p09_crop", clock, result));

    let clock = Instant::now();
    let result = with_tools(&root, "audio", tools.as_ref(), |base, _| audio_stage(base));
    stages.push(stage("p09_audio", clock, result));

    let clock = Instant::now();
    let result = with_tools(&root, "malformed", tools.as_ref(), |base, _| {
        malformed_stage(base)
    });
    stages.push(stage("p09_malformed", clock, result));

    let clock = Instant::now();
    let result = with_tools(&root, "bundle", tools.as_ref(), |base, _| {
        stream_and_bundle_stage(base)
    });
    stages.push(stage("p09_stream_and_bundle", clock, result));

    let clock = Instant::now();
    let result = with_tools(&root, "journey-supplied", tools.as_ref(), |base, tools| {
        mechanical_journey(base, tools, TranscriptPath::Supplied)
    });
    stages.push(stage("p09_mechanical_journey_supplied", clock, result));

    let clock = Instant::now();
    let whisper = Whisper::discover();
    let result = match whisper.as_ref() {
        Some(whisper) => with_tools(&root, "journey-asr", tools.as_ref(), |base, tools| {
            prepare_whisper(base, whisper)?;
            mechanical_journey(base, tools, TranscriptPath::LocalAsr)
        }),
        None => Err(StageStop::Blocked(
            "set VSIFT_TEST_WHISPER_CLI and VSIFT_TEST_WHISPER_MODEL to an absolute whisper-cli and ggml model".to_owned(),
        )),
    };
    stages.push(stage("p09_mechanical_journey_local_asr", clock, result));

    let clock = Instant::now();
    let result = with_tools(&root, "perf", tools.as_ref(), perf_stage);
    stages.push(stage("p09_perf", clock, result));

    let status_of = |wanted: &str| stages.iter().any(|entry| entry["status"] == wanted);
    let overall = if status_of("failed") {
        "failed"
    } else if status_of("blocked") {
        "blocked"
    } else {
        "passed"
    };
    stages.push(json!({"name": "p09_evidence", "status": overall}));
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
        "local_asr_path": whisper.is_some(),
        "fixtures": "F01-F10 and F12, F01-rotation-90.mp4, F01-audio-only.m4a, F03-speech.mp4 with a SubRip file written from the frozen script, F11-damaged-tail.mp4, F11-truncated.mp4, with the frame lists of fixtures/corpus/generated/verification.json; F05 cut short and a 1080p clip of about 1 GiB built at run time",
        "os": env::consts::OS,
        "architecture": env::consts::ARCH,
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "resource_profile": "each CLI call killed after 300 s; empty PATH; each evidence call under its 120 s deadline",
        "vsift_version": env!("CARGO_PKG_VERSION"),
        "authorization": "opt-in cargo test invocation; setup configure writes only to isolated temporary per-user bases; no install, download or network access",
        "prior_checkpoints": ["P08: p08_search_e2e", "P08: p08_candidates_e2e"],
        "stages": stages,
        "coverage_gaps": [
            "Tiny text is measured on synthetic 5x7 glyphs only; real screen text with anti-aliasing and compression is not in the corpus",
            "A burst over more than 1,200 frames (60 fps over more than 20 s) is refused as outside_listing",
            "Performance is recorded, not gated; the build profile is in the p09_perf evidence"
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
