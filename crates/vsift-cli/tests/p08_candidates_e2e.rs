//! Opt-in cumulative P08 checkpoint: visual candidates (V-02..V-05, S-11).
//!
//! Journeys drive the compiled `vsift` binary exactly as a headless agent
//! would, with isolated per-user bases and an empty `PATH`: `FFmpeg` and
//! `FFprobe` are registered with `setup configure` (and, for the optional
//! local-ASR variant, whisper.cpp and its model). Only the continuation stage
//! drives the engine library directly, to lower its per-call window budget.
//! Expectations come only from the frozen corpus truth
//! (`fixtures/corpus/manifest.json`, `speech-provenance.json`); nothing is
//! derived from the candidates being scored. The stages:
//!
//! - `p08_candidates_fixtures`: F01-F10 and F12 through `candidates`, scored
//!   against the truth: every `stable` event of at least 1 s must be hit
//!   (F04-E02, F05-E02 and F12-E02 are corpus limitations, issue #159), no
//!   change candidate in the static F01 and F07, and a warm second call must
//!   return the same candidates without analysing;
//! - `p08_lead_lag`: F03, F04, F05 and F09 speech variants imported with a
//!   `SubRip` file written at run time from the frozen script and speech
//!   placement; a searched term's hit time, widened by 10 s either side, must
//!   hold a candidate inside the visual event the term names (and, with
//!   `VSIFT_TEST_WHISPER_CLI`/`VSIFT_TEST_WHISPER_MODEL`, the same after local
//!   speech recognition instead of the import);
//! - `p08_candidates_budget`: a 3-minute clip built at run time, indexed by an
//!   engine whose budget is one window per call, continues from
//!   `not_analyzed` on each call; the binary then reads it warm without tools;
//! - `p08_candidates_stream_and_bundle`: the `--events jsonl` stream, then
//!   `session retain` and `bundle validate`, the record conforming to its
//!   bundle schema;
//! - `p08_candidates_malformed`: F11's damaged tail and an audio-only file;
//! - `p08_candidates_motion` (V-03): scroll (slow and fast, under a sticky
//!   header) and zoom clips built at run time, each stop followed within 1 s
//!   by a `settled_after_motion` candidate, and a bounded candidate count;
//! - `p08_candidates_s11`: a 30-minute session built at run time; the cold
//!   analysis time and the warm page times through the binary.
//!
//! Clips are encoded with `FFmpeg`'s native MPEG-4 Part 2 encoder, present in
//! every build including the pinned CI builds that omit libx264.
//!
//! ```console
//! cargo test --release -p vsift-cli --locked --test p08_candidates_e2e -- --ignored --nocapture
//! ```
//!
//! The run writes `.vsift/e2e-runs/p08-candidates-<run-id>/report.json` and
//! prints `p08_candidates: passed` when every stage passed. A stage that cannot
//! run here is `blocked`, never `passed`, and the test fails unless every
//! stage passed.

use std::{
    collections::BTreeMap,
    env,
    error::Error,
    ffi::OsStr,
    fmt::Write as _,
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::Command as Process,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::{Value, json};
use vsift::{
    Cancellation, CandidatesRange, CandidatesRequest, CoverageGapReason, Engine, EngineConfig,
    EnginePorts, HostIsolation, IngestRequest, SessionRootLocation, UserConfigurationLocation,
};
use vsift_infrastructure::{ExecutableResolver, TrustedExecutable};

type TestResult = Result<(), Box<dyn Error>>;

/// Upper bound for one CLI invocation; 30 windows of analysis in a debug
/// build need minutes.
const CLI_DEADLINE: Duration = Duration::from_mins(15);
const OWNED_PREFIX: &str = "vsift-p08-candidates-e2e-";
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
/// Static fixtures: any change candidate in them is false.
const STATIC_FIXTURES: [&str; 2] = ["F01", "F07"];
/// Events drawn with the pixels of the state before them (issue #159).
const CORPUS_LIMITATIONS: [&str; 3] = ["F04-E02", "F05-E02", "F12-E02"];
/// Speech variant, searched term and the visual event the term names.
const LEAD_LAG: [(&str, &str, &str, &str); 4] = [
    ("F03", "F03-speech.mp4", "127.50", "F03-E02"),
    ("F04", "F04-speech.mp4", "1017", "F04-E03"),
    ("F05", "F05-speech.mp4", "E-409", "F05-E03"),
    ("F09", "F09-speech.mkv", "marker beta", "F09-E01"),
];
/// How far either side of a search hit an agent looks for candidates.
const LEAD_LAG_US: u64 = 10 * SECOND;
/// A stop must be followed by a settled candidate within this time.
const SETTLE_TOLERANCE_US: u64 = SECOND;
/// The warm p95 page target of the verification plan's reference profile.
const P95_TARGET: Duration = Duration::from_millis(250);

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
            .args(["-v", "error", "-y"])
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
}

/// The optional local-ASR variant's recognizer and model.
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

/// Registers the media tools (and whisper.cpp with its model) in `base`.
fn prepare(base: &Path, tools: &MediaTools, whisper: Option<&Whisper>) -> StageResult {
    let mut selections = vec![
        ("ffmpeg", tools.ffmpeg.path().to_path_buf()),
        ("ffprobe", tools.ffprobe.path().to_path_buf()),
    ];
    if let Some(whisper) = whisper {
        selections.push(("whisper", whisper.cli.clone()));
    }
    for (dependency, path) in selections {
        let (code, _) = run_json(
            vsift(base)?
                .args(["setup", "configure", dependency, "--executable"])
                .arg(path)
                .arg("--json"),
        )?;
        ensure(code == Some(0), "setup configure did not succeed")?;
    }
    if let Some(whisper) = whisper {
        let (code, _) = run_json(
            vsift(base)?
                .args(["setup", "configure-model", "--file"])
                .arg(&whisper.model)
                .arg("--json"),
        )?;
        ensure(code == Some(0), "setup configure-model did not succeed")?;
    }
    let (code, check) = run_json(vsift(base)?.args(["setup", "check", "--json"]))?;
    ensure(code == Some(0), "setup check did not exit 0")?;
    Ok(check)
}

fn ingest(base: &Path, video: &Path, transcript: Option<&Path>) -> Result<String, StageStop> {
    let mut command = vsift(base)?;
    command.arg("ingest").arg(video);
    if let Some(transcript) = transcript {
        command
            .arg("--transcript")
            .arg(transcript)
            .args(["--transcript-offset", "0"]);
    }
    let (code, opened) = run_json(command.arg("--json"))?;
    ensure(
        code == Some(0),
        &format!("ingest failed: {}", opened["error"]["code"]),
    )?;
    opened["data"]["session_id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| failed("session missing"))
}

/// One `candidates --json` call: exit code, result and wall time.
fn candidates_call(
    base: &Path,
    session: &str,
    (from, to): (u64, u64),
    limit: u16,
    cursor: Option<&str>,
) -> Result<(Option<i32>, Value, Duration), StageStop> {
    let mut command = vsift(base)?;
    command
        .args(["candidates", session, "--from"])
        .arg(from.to_string())
        .arg("--to")
        .arg(to.to_string())
        .args(["--limit", &limit.to_string()]);
    if let Some(cursor) = cursor {
        command.args(["--cursor", cursor]);
    }
    let started = Instant::now();
    let (code, result) = run_json(command.arg("--json"))?;
    let elapsed = started.elapsed();
    conforms("operation-response.schema.json", &result)?;
    if result["error"].is_null() {
        conforms("candidates-data.schema.json", &result["data"])?;
    }
    Ok((code, result, elapsed))
}

/// Every candidate of `range`, paging at `limit`; the first page's result
/// and time (which include any analysis), and every page's time.
struct Paged {
    items: Vec<Value>,
    first: Value,
    first_elapsed: Duration,
    page_times: Vec<Duration>,
}

fn candidates_all(
    base: &Path,
    session: &str,
    range: (u64, u64),
    limit: u16,
) -> Result<Paged, StageStop> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    let mut first = None;
    let mut first_elapsed = Duration::ZERO;
    let mut page_times = Vec::new();
    loop {
        let (code, result, elapsed) =
            candidates_call(base, session, range, limit, cursor.as_deref())?;
        ensure(
            code == Some(0),
            &format!(
                "candidates failed: {} {}",
                result["error"]["code"], result["error"]["remediation"][0]["summary"]
            ),
        )?;
        page_times.push(elapsed);
        items.extend(
            result["data"]["items"]
                .as_array()
                .ok_or_else(|| failed("items missing"))?
                .iter()
                .cloned(),
        );
        cursor = result["data"]["next_cursor"].as_str().map(str::to_owned);
        if first.is_none() {
            first = Some(result);
            first_elapsed = elapsed;
        }
        if cursor.is_none() {
            break;
        }
    }
    Ok(Paged {
        items,
        first: first.ok_or_else(|| failed("no page"))?,
        first_elapsed,
        page_times,
    })
}

/// Microseconds as seconds, for the report.
fn seconds(micros: u64) -> f64 {
    f64::from(u32::try_from(micros / 1_000).unwrap_or(u32::MAX)) / 1_000.0
}

/// A count as a float, for the report's rates.
fn count(value: impl TryInto<u32>) -> f64 {
    f64::from(value.try_into().unwrap_or(u32::MAX))
}

fn manifest() -> Result<Value, StageStop> {
    Ok(serde_json::from_slice(&fs::read(
        repository().join("fixtures/corpus/manifest.json"),
    )?)?)
}

fn manifest_fixture<'a>(manifest: &'a Value, id: &str) -> Result<&'a Value, StageStop> {
    manifest["fixtures"]
        .as_array()
        .and_then(|fixtures| fixtures.iter().find(|entry| entry["id"] == id))
        .ok_or_else(|| failed("fixture missing from the manifest"))
}

fn u64_of(value: &Value) -> Result<u64, StageStop> {
    value.as_u64().ok_or_else(|| failed("a number is missing"))
}

fn representative(item: &Value) -> u64 {
    item["representative_us"].as_u64().unwrap_or(u64::MAX)
}

fn is_change(item: &Value) -> bool {
    item["reasons"].as_array().is_some_and(|reasons| {
        reasons.iter().any(|reason| {
            reason == "visual_change"
                || reason == "motion_start"
                || reason == "settled_after_motion"
        })
    })
}

/// Scores one fixture's candidates against its frozen truth.
fn score(entry: &Value, items: &[Value]) -> Result<Value, StageStop> {
    let id = entry["id"].as_str().unwrap_or_default();
    let events: Vec<&Value> = entry["events"]
        .as_array()
        .map(|events| {
            events
                .iter()
                .filter(|event| event["kind"] != "speech" && event["kind"] != "malformed")
                .collect()
        })
        .unwrap_or_default();
    let boundaries: Vec<u64> = events
        .iter()
        .flat_map(|event| [event["start_us"].as_u64(), event["end_us"].as_u64()])
        .flatten()
        .collect();
    let mut scored = Vec::new();
    for event in &events {
        let (start, end) = (u64_of(&event["start_us"])?, u64_of(&event["end_us"])?);
        let hit = items
            .iter()
            .map(representative)
            .find(|time| (start..end).contains(time));
        let event_id = event["id"].as_str().unwrap_or_default();
        // As in the always-run recorded gate: every stable event of at
        // least 1 s, including F12-E02, which only periodic coverage hits.
        let gated = event["kind"] == "stable" && end - start >= SECOND;
        scored.push(json!({
            "event": event_id,
            "kind": event["kind"],
            "window_us": [start, end],
            "hit": hit.is_some(),
            "timestamp_error_us": hit.map(|time| time - start),
            "gated": gated,
            "corpus_limitation": CORPUS_LIMITATIONS.contains(&event_id),
        }));
    }
    let false_changes = items
        .iter()
        .filter(|item| is_change(item))
        .filter(|item| {
            let window = &item["change_window"];
            let (after, at_or_before) = (
                window["from_us"].as_u64().unwrap_or(0),
                window["to_us"].as_u64().unwrap_or(0),
            );
            !boundaries
                .iter()
                .any(|boundary| after < *boundary && *boundary <= at_or_before)
        })
        .count();
    let change_candidates = items.iter().filter(|item| is_change(item)).count();
    Ok(json!({
        "fixture": id,
        "candidates": items.len(),
        "change_candidates": change_candidates,
        "false_changes": false_changes,
        "static": STATIC_FIXTURES.contains(&id),
        "events": scored,
    }))
}

/// V-02, V-05: every fixture through the binary, scored against the truth.
#[allow(
    clippy::too_many_lines,
    reason = "One stage: index, score, gate and summarise every fixture"
)]
fn fixtures_stage(
    base: &Path,
    tools: &MediaTools,
) -> Result<(Value, BTreeMap<String, String>), StageStop> {
    let check = prepare(base, tools, None)?;
    let manifest = manifest()?;
    let mut sessions = BTreeMap::new();
    let mut per_fixture = Vec::new();
    let mut media_us = 0_u64;
    let mut analysis = Duration::ZERO;
    let mut total_candidates = 0_usize;
    let mut failures = Vec::new();
    let mut warm_ms = Vec::new();
    for (id, file) in VISUAL_FIXTURES {
        let entry = manifest_fixture(&manifest, id)?;
        let duration = u64_of(&entry["duration_us"])?;
        let session = ingest(base, &fixture(file), None)?;
        let paged = candidates_all(base, &session, (0, duration), 100)?;
        ensure(
            paged.first["status"] == "complete",
            &format!(
                "{id}: the first call was not complete: {}",
                paged.first["coverage"]
            ),
        )?;
        let warm = candidates_all(base, &session, (0, duration), 100)?;
        ensure(
            warm.items == paged.items,
            &format!("{id}: a warm call returned other candidates"),
        )?;
        warm_ms.push(warm.first_elapsed.as_millis());
        let scored = score(entry, &paged.items)?;
        for event in scored["events"].as_array().into_iter().flatten() {
            if event["gated"] == true && event["hit"] == false {
                failures.push(format!("{} missed", event["event"]));
            }
        }
        if scored["static"] == true && scored["change_candidates"] != 0 {
            failures.push(format!("{id} has change candidates"));
        }
        media_us += duration;
        analysis += paged.first_elapsed;
        total_candidates += paged.items.len();
        let mut row = scored;
        row["first_call_ms"] = json!(paged.first_elapsed.as_millis());
        row["warm_call_ms"] = json!(warm.first_elapsed.as_millis());
        per_fixture.push(row);
        sessions.insert(id.to_owned(), session);
    }
    ensure(failures.is_empty(), &failures.join("; "))?;
    let mut by_kind: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    let mut errors = Vec::new();
    let mut false_changes = 0_u64;
    for row in &per_fixture {
        false_changes += row["false_changes"].as_u64().unwrap_or(0);
        for event in row["events"].as_array().into_iter().flatten() {
            let kind = event["kind"].as_str().unwrap_or("unknown").to_owned();
            let entry = by_kind.entry(kind).or_insert((0, 0));
            entry.1 += 1;
            if event["hit"] == true {
                entry.0 += 1;
            }
            if let Some(error) = event["timestamp_error_us"].as_u64() {
                errors.push(error);
            }
        }
    }
    errors.sort_unstable();
    let media_seconds = seconds(media_us);
    let minutes = media_seconds / 60.0;
    Ok((
        json!({
            "fixtures": per_fixture,
            "recall_by_kind": by_kind
                .iter()
                .map(|(kind, (hit, total))| json!({"kind": kind, "hit": hit, "events": total}))
                .collect::<Vec<_>>(),
            "timestamp_error_us": {
                "median": errors.get(errors.len() / 2),
                "max": errors.last(),
            },
            "false_changes": false_changes,
            "false_changes_per_minute": count(false_changes) / minutes,
            "candidates_per_minute": count(total_candidates) / minutes,
            "media_seconds": media_seconds,
            "cold_calls_seconds": analysis.as_secs_f64(),
            "media_seconds_per_second": media_seconds / analysis.as_secs_f64().max(0.001),
            "warm_call_ms": warm_ms,
            "ffmpeg_version": check["dependencies"][0]["detail"],
        }),
        sessions,
    ))
}

/// The frozen script as one `SubRip` cue over the generator's speech span.
fn write_srt(path: &Path, fixture_id: &str, manifest: &Value) -> Result<(), StageStop> {
    let provenance: Value = serde_json::from_slice(&fs::read(fixture("speech-provenance.json"))?)?;
    let variant = provenance["assembly"]["variants"]
        .as_array()
        .and_then(|variants| {
            variants
                .iter()
                .find(|variant| variant["fixture"] == fixture_id)
        })
        .ok_or_else(|| failed("fixture missing from speech provenance"))?;
    let script = manifest_fixture(manifest, fixture_id)?["audio"]["script"]
        .as_str()
        .ok_or_else(|| failed("no script"))?;
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
    let text = format!(
        "1\n{} --> {}\n{script}\n",
        stamp(u64_of(&variant["speech_start_us"])?),
        stamp(u64_of(&variant["speech_end_us"])?)
    );
    fs::write(path, text)?;
    Ok(())
}

/// The first search hit's start time, or `None` when nothing matched.
fn search_hit(base: &Path, session: &str, term: &str) -> Result<Option<u64>, StageStop> {
    let (code, result) =
        run_json(vsift(base)?.args(["search", session, "--query", term, "--json"]))?;
    ensure(code == Some(0), &format!("search {term:?} failed"))?;
    conforms("search-data.schema.json", &result["data"])?;
    Ok(result["data"]["items"][0]["start_us"].as_u64())
}

/// V-04: candidates within 10 s of the hit include one inside `event`.
fn lead_lag_holds(
    base: &Path,
    session: &str,
    hit: u64,
    duration: u64,
    (start, end): (u64, u64),
) -> Result<Value, StageStop> {
    let range = (
        hit.saturating_sub(LEAD_LAG_US),
        (hit + LEAD_LAG_US).min(duration),
    );
    let paged = candidates_all(base, session, range, 100)?;
    let inside: Vec<u64> = paged
        .items
        .iter()
        .map(representative)
        .filter(|time| (start..end).contains(time))
        .collect();
    ensure(
        !inside.is_empty(),
        &format!("no candidate in [{start}, {end}) within 10 s of the hit at {hit}"),
    )?;
    Ok(json!({
        "hit_us": hit,
        "range_us": [range.0, range.1],
        "candidates_in_range": paged.items.len(),
        "inside_event_us": inside,
    }))
}

fn event_window(
    manifest: &Value,
    fixture_id: &str,
    event_id: &str,
) -> Result<(u64, u64), StageStop> {
    let event = manifest_fixture(manifest, fixture_id)?["events"]
        .as_array()
        .and_then(|events| events.iter().find(|event| event["id"] == event_id))
        .ok_or_else(|| failed("event missing from the manifest"))?;
    Ok((u64_of(&event["start_us"])?, u64_of(&event["end_us"])?))
}

fn lead_lag_stage(root: &OwnedRoot, tools: &MediaTools, whisper: Option<&Whisper>) -> StageResult {
    let manifest = manifest()?;
    let base = root.base("lead-lag");
    prepare(&base, tools, None)?;
    let mut supplied = Vec::new();
    for (id, file, term, event) in LEAD_LAG {
        let srt = base.join(format!("{id}.srt"));
        write_srt(&srt, id, &manifest)?;
        let session = ingest(&base, &fixture(file), Some(&srt))?;
        let hit = search_hit(&base, &session, term)?
            .ok_or_else(|| failed(&format!("{term:?} was not found in {id}")))?;
        let duration = u64_of(&manifest_fixture(&manifest, id)?["duration_us"])?;
        let evidence = lead_lag_holds(
            &base,
            &session,
            hit,
            duration,
            event_window(&manifest, id, event)?,
        )?;
        supplied.push(json!({"fixture": id, "term": term, "event": event, "evidence": evidence}));
    }
    let asr = match whisper {
        None => json!("not run: set VSIFT_TEST_WHISPER_CLI and VSIFT_TEST_WHISPER_MODEL"),
        Some(whisper) => {
            let base = root.base("lead-lag-asr");
            prepare(&base, tools, Some(whisper))?;
            let mut rows = Vec::new();
            for (id, file, term, event) in LEAD_LAG {
                let session = ingest(&base, &fixture(file), None)?;
                let (code, result) = run_json(
                    vsift(&base)?
                        .args(["transcript", "retranscribe", &session])
                        .arg("--json"),
                )?;
                ensure(
                    code == Some(0),
                    &format!("retranscribe of {id} failed: {}", result["error"]["code"]),
                )?;
                let duration = u64_of(&manifest_fixture(&manifest, id)?["duration_us"])?;
                match search_hit(&base, &session, term)? {
                    Some(hit) => {
                        let evidence = lead_lag_holds(
                            &base,
                            &session,
                            hit,
                            duration,
                            event_window(&manifest, id, event)?,
                        )?;
                        rows.push(json!({"fixture": id, "term": term, "heard": true, "evidence": evidence}));
                    }
                    None => rows.push(json!({"fixture": id, "term": term, "heard": false})),
                }
            }
            let heard = rows.iter().filter(|row| row["heard"] == true).count();
            ensure(
                heard >= 3,
                &format!("local ASR found only {heard} of 4 terms: {rows:?}"),
            )?;
            json!(rows)
        }
    };
    Ok(json!({"supplied_transcript": supplied, "local_asr": asr}))
}

/// Builds a clip of F02 looped `loops` times, video only, as MPEG-4 Part 2.
fn looped_clip(
    tools: &MediaTools,
    work: &Path,
    name: &str,
    loops: u32,
    scale: &str,
) -> Result<PathBuf, StageStop> {
    let clip = work.join(name);
    let source = fixture("F02.mp4");
    let repeat = loops.saturating_sub(1).to_string();
    let filter = format!("scale={scale}");
    tools.build(&[
        OsStr::new("-stream_loop"),
        OsStr::new(&repeat),
        OsStr::new("-i"),
        source.as_os_str(),
        OsStr::new("-map"),
        OsStr::new("0:v:0"),
        OsStr::new("-vf"),
        OsStr::new(&filter),
        OsStr::new("-r"),
        OsStr::new("10"),
        OsStr::new("-c:v"),
        OsStr::new("mpeg4"),
        OsStr::new("-q:v"),
        OsStr::new("5"),
        OsStr::new("-an"),
        clip.as_os_str(),
    ])?;
    Ok(clip)
}

/// A 3-minute session indexed one window per call by the engine library,
/// then read warm through the binary without any tool.
async fn budget_stage(root: &OwnedRoot, tools: &MediaTools) -> StageResult {
    let base = root.base("budget");
    fs::create_dir_all(&base)?;
    let clip = looped_clip(tools, &base, "three-minutes.mp4", 15, "640:360")?;
    let engine = Engine::new(
        EngineConfig {
            session_root: SessionRootLocation::Explicit(base.join("sessions")),
            user_configuration: UserConfigurationLocation::Explicit(base.join("engine-config")),
            host_isolation: HostIsolation::ProcessOnly,
        },
        EnginePorts::system().with_visual_window_budget(NonZeroUsize::MIN),
    );
    let session = engine
        .ingest(IngestRequest {
            source: clip,
            transcript: None,
        })
        .await
        .map_err(|error| failed(&error.to_string()))?
        .session
        .session_id;
    let request = || CandidatesRequest {
        session: session.clone(),
        range: CandidatesRange {
            from_micros: 0,
            to_micros: 180 * SECOND,
        },
        limit: Some(100),
        cursor: None,
        cancellation: Cancellation::new(),
    };
    let mut calls = Vec::new();
    for call in 0..4 {
        let results = engine
            .candidates(request())
            .await
            .map_err(|error| failed(&error.to_string()))?;
        let not_analyzed = results
            .gaps()
            .iter()
            .filter(|gap| gap.reason == CoverageGapReason::NotAnalyzed)
            .count();
        calls.push(json!({
            "call": call,
            "analysed_now": results.analysed_now(),
            "index_revision": results.index().number().get(),
            "not_analyzed_gaps": not_analyzed,
            "gaps": results.gaps().len(),
        }));
        let windows = results.index().windows().len();
        ensure(
            results.analysed_now() == usize::from(call < 3),
            &format!("call {call} analysed {} windows", results.analysed_now()),
        )?;
        ensure(
            windows == (call + 1).min(3),
            &format!("call {call} left {windows} windows indexed"),
        )?;
        ensure(
            (not_analyzed == 0) == (call >= 2),
            &format!("call {call} reported {not_analyzed} not_analyzed gaps"),
        )?;
    }
    // The binary reads it warm: no tool is configured and PATH is empty.
    let (code, result, elapsed) =
        candidates_call(&base, session.as_str(), (0, 180 * SECOND), 100, None)?;
    ensure(
        code == Some(0) && result["status"] == "complete",
        &format!("the warm read failed: {}", result["error"]["code"]),
    )?;
    Ok(json!({
        "clip": "F02 looped 15 times (180 s), 640x360, 10 fps, mpeg4",
        "engine_calls": calls,
        "warm_cli_ms": elapsed.as_millis(),
        "warm_cli_items": result["data"]["items"].as_array().map(Vec::len),
    }))
}

fn stream_and_bundle_stage(base: &Path, session: &str) -> StageResult {
    let output = vsift(base)?
        .args(["candidates", session, "--from", "0", "--to", "12000000"])
        .args(["--events", "jsonl"])
        .output()?;
    ensure(output.status.code() == Some(0), "the stream failed")?;
    ensure(
        output.stderr.is_empty(),
        "the stream wrote to standard error",
    )?;
    let lines = std::str::from_utf8(&output.stdout)?
        .trim_end_matches('\n')
        .split('\n')
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    let (terminal, records) = lines
        .split_last()
        .ok_or_else(|| failed("the stream is empty"))?;
    for (sequence, record) in (0_u64..).zip(records) {
        conforms("evidence-event.schema.json", record)?;
        conforms("visual-candidate.schema.json", &record["record"])?;
        ensure(
            record["sequence"].as_u64() == Some(sequence)
                && record["record_type"] == "visual_candidate"
                && record["key"] == record["record"]["candidate_id"],
            "an evidence event is out of sequence or mis-keyed",
        )?;
    }
    conforms("terminal-event.schema.json", terminal)?;
    conforms(
        "candidates-stream-data.schema.json",
        &terminal["result"]["data"],
    )?;
    ensure(
        terminal["result"]["data"]["record_count"].as_u64() == u64::try_from(records.len()).ok(),
        "the terminal event does not count the records",
    )?;
    let bundle = base.join("bundle");
    let (code, retained) = run_json(
        vsift(base)?
            .args(["session", "retain", session, "--output"])
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
    let mut kinds = Vec::new();
    for artifact in manifest["artifacts"]
        .as_array()
        .ok_or_else(|| failed("artifacts missing"))?
    {
        let name = artifact["name"].as_str().unwrap_or_default();
        ensure(
            name.starts_with("artifact-") && !name.contains(['/', '\\']),
            "a record has an unexpected name",
        )?;
        kinds.push(artifact["kind"].clone());
        if artifact["kind"] == "visual_index_record" {
            let record: Value = serde_json::from_slice(&fs::read(bundle.join(name))?)?;
            conforms("bundle-visual-index-record.schema.json", &record)?;
        }
    }
    ensure(
        kinds.iter().any(|kind| kind == "visual_index_record"),
        "the bundle carries no visual-index record",
    )?;
    Ok(json!({
        "jsonl_evidence_records": records.len(),
        "bundle_artifacts": validated["data"]["artifact_count"],
        "bundle_artifact_kinds": kinds,
    }))
}

/// Damaged and unsuitable media: F11's damaged tail (only its audio is
/// damaged, so the video is analysed completely), a video truncated at run
/// time (its window is recorded once as `undecodable`), and an audio-only file.
fn malformed_stage(base: &Path) -> StageResult {
    let damaged = ingest(base, &fixture("F11-damaged-tail.mp4"), None)?;
    let (code, result, _) = candidates_call(base, &damaged, (0, 60 * SECOND), 100, None)?;
    ensure(
        code == Some(0),
        &format!("the damaged tail failed with {}", result["error"]["code"]),
    )?;
    let duration = u64_of(&result["data"]["index"]["duration_us"])?;
    ensure(
        result["data"]["items"]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| representative(item) < duration)),
        "a candidate lies past the damaged file's decodable duration",
    )?;
    let damaged_outcome = json!({
        "status": result["status"],
        "coverage": result["coverage"],
        "candidates": result["data"]["items"].as_array().map(Vec::len),
    });

    // The first 60 % of F05 (its index precedes its media data), so the
    // container probes whole but the video breaks off.
    let bytes = fs::read(fixture("F05.mp4"))?;
    let truncated = base.join("truncated.mp4");
    fs::write(
        &truncated,
        bytes.get(..bytes.len() * 6 / 10).unwrap_or_default(),
    )?;
    let session = ingest(base, &truncated, None)?;
    let (code, first, _) = candidates_call(base, &session, (0, 20 * SECOND), 100, None)?;
    ensure(
        code == Some(0)
            && first["status"] == "partial"
            && first["coverage"]["reasons"] == json!(["undecodable"]),
        &format!(
            "the truncated video was not an undecodable gap: {} {}",
            first["error"]["code"], first["coverage"]
        ),
    )?;
    let (_, again, _) = candidates_call(base, &session, (0, 20 * SECOND), 100, None)?;
    ensure(
        again["data"]["index"]["number"] == first["data"]["index"]["number"],
        "an undecodable window was analysed again",
    )?;

    let audio = ingest(base, &fixture("F01-audio-only.m4a"), None)?;
    let (code, result, _) = candidates_call(base, &audio, (0, 60 * SECOND), 100, None)?;
    ensure(
        code == Some(2) && result["error"]["code"] == "INVALID_ARGUMENT",
        &format!("an audio-only file gave {}", result["error"]["code"]),
    )?;
    ensure(
        result["error"]["remediation"][0]["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("no video stream")),
        "the audio-only remediation does not say there is no video stream",
    )?;
    let (_, status) = run_json(vsift(base)?.args(["session", "status", &audio, "--json"]))?;
    ensure(
        status["data"]["artifact_count"] == 0,
        "a rejected call committed an index",
    )?;
    Ok(json!({
        "damaged_tail": damaged_outcome,
        "truncated_video": {"status": first["status"], "coverage": first["coverage"], "candidates": first["data"]["items"].as_array().map(Vec::len)},
        "audio_only": result["error"]["code"],
    }))
}

/// Writes a random still 1280 x `height` cell pattern to `path`.
fn pattern(tools: &MediaTools, path: &Path, height: u32) -> Result<(), StageStop> {
    let source = format!(
        "life=size=128x{}:ratio=0.35:seed=7:rate=1:death_color=0x202020:life_color=0xd0d0d0",
        height / 10
    );
    let filter = format!("scale=1280:{height}:flags=neighbor");
    tools.build(&[
        OsStr::new("-f"),
        OsStr::new("lavfi"),
        OsStr::new("-i"),
        OsStr::new(&source),
        OsStr::new("-vf"),
        OsStr::new(&filter),
        OsStr::new("-frames:v"),
        OsStr::new("1"),
        path.as_os_str(),
    ])
}

/// Encodes `still` looped for `seconds` through `filter` at 25 fps.
fn motion_clip(
    tools: &MediaTools,
    still: &Path,
    seconds: u32,
    filter: &str,
    clip: &Path,
) -> Result<(), StageStop> {
    let duration = seconds.to_string();
    tools.build(&[
        OsStr::new("-loop"),
        OsStr::new("1"),
        OsStr::new("-framerate"),
        OsStr::new("25"),
        OsStr::new("-t"),
        OsStr::new(&duration),
        OsStr::new("-i"),
        still.as_os_str(),
        OsStr::new("-vf"),
        OsStr::new(filter),
        OsStr::new("-c:v"),
        OsStr::new("mpeg4"),
        OsStr::new("-q:v"),
        OsStr::new("3"),
        clip.as_os_str(),
    ])
}

/// Checks one motion clip: a `settled_after_motion` candidate within 1 s of
/// each stop, in order, and no more candidates than `bound`.
fn motion_holds(
    base: &Path,
    clip: &Path,
    seconds: u64,
    stops: &[u64],
    bound: usize,
) -> Result<Value, StageStop> {
    let session = ingest(base, clip, None)?;
    let paged = candidates_all(base, &session, (0, seconds * SECOND), 100)?;
    let settled: Vec<u64> = paged
        .items
        .iter()
        .filter(|item| {
            item["reasons"].as_array().is_some_and(|reasons| {
                reasons
                    .iter()
                    .any(|reason| reason == "settled_after_motion")
            })
        })
        .map(representative)
        .collect();
    let mut matched = Vec::new();
    let mut after = 0_u64;
    for stop in stops {
        let found = settled
            .iter()
            .copied()
            .find(|time| *time >= after && *stop <= *time && *time <= stop + SETTLE_TOLERANCE_US);
        let time = found.ok_or_else(|| {
            failed(&format!(
                "no settled_after_motion within 1 s of the stop at {stop} us: {settled:?}"
            ))
        })?;
        matched.push(time);
        after = time;
    }
    ensure(
        paged.items.len() <= bound,
        &format!("{} candidates exceed the bound {bound}", paged.items.len()),
    )?;
    let mut reasons = String::new();
    for item in &paged.items {
        let _ = write!(
            reasons,
            "{}:{} ",
            representative(item),
            item["reasons"][0].as_str().unwrap_or_default()
        );
    }
    Ok(json!({
        "stops_us": stops,
        "settled_after_motion_us": matched,
        "candidates": paged.items.len(),
        "bound": bound,
        "timeline": reasons.trim_end(),
    }))
}

/// V-03 on clips built at run time from a random cell pattern.
fn motion_stage(root: &OwnedRoot, tools: &MediaTools) -> StageResult {
    let base = root.base("motion");
    prepare(&base, tools, None)?;
    let tall = base.join("tall.png");
    pattern(tools, &tall, 2880)?;
    let still = base.join("still.png");
    pattern(tools, &still, 720)?;
    // Still until 3 s, a slow scroll (100 px/s) until 7 s, still until 10 s,
    // a fast scroll (700 px/s) until 12 s, still until 16 s; a sticky header
    // band stays fixed over the scrolling content.
    let scroll = base.join("scroll.mp4");
    motion_clip(
        tools,
        &tall,
        16,
        "crop=1280:720:0:'if(lt(t,3),0,if(lt(t,7),(t-3)*100,if(lt(t,10),400,if(lt(t,12),400+(t-10)*700,1800))))',drawbox=x=0:y=0:w=1280:h=96:color=0x204080:t=fill,format=yuv420p",
        &scroll,
    )?;
    // Still until 4 s, a zoom to 1.6x until 7 s, still until 12 s.
    let zoom = base.join("zoom.mp4");
    motion_clip(
        tools,
        &still,
        12,
        "zoompan=z='if(lt(in,100),1,if(lt(in,175),1+(in-100)*0.008,1.6))':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d=1:s=1280x720:fps=25,format=yuv420p",
        &zoom,
    )?;
    // At most: the first frame, one candidate per 10 s cell, and a motion
    // start and a settled candidate per motion, plus one of slack each.
    let scroll_evidence = motion_holds(
        &base,
        &scroll,
        16,
        &[7 * SECOND, 12 * SECOND],
        1 + 2 + 2 * 3,
    )?;
    let zoom_evidence = motion_holds(&base, &zoom, 12, &[7 * SECOND], 1 + 2 + 3)?;
    Ok(json!({"scroll_with_sticky_header": scroll_evidence, "zoom": zoom_evidence}))
}

fn p95(durations: &[Duration]) -> Duration {
    let mut sorted = durations.to_vec();
    sorted.sort_unstable();
    let rank = (sorted.len() * 95).div_ceil(100).saturating_sub(1);
    sorted.get(rank).copied().unwrap_or_default()
}

/// S-11 on a 30-minute session: cold analysis, then warm pages through the
/// binary.
fn s11_stage(root: &OwnedRoot, tools: &MediaTools) -> StageResult {
    let base = root.base("s11");
    prepare(&base, tools, None)?;
    let built = Instant::now();
    let clip = looped_clip(tools, &base, "thirty-minutes.mp4", 150, "640:360")?;
    let build_ms = built.elapsed().as_millis();
    let session = ingest(&base, &clip, None)?;
    let whole = (0, 1_900 * SECOND);
    let mut cold = Vec::new();
    for _ in 0..3 {
        let (code, result, elapsed) = candidates_call(&base, &session, whole, 20, None)?;
        ensure(
            code == Some(0),
            &format!("the cold call failed: {}", result["error"]["code"]),
        )?;
        cold.push(json!({"ms": elapsed.as_millis(), "status": result["status"], "reasons": result["coverage"]["reasons"]}));
        if result["status"] == "complete" {
            break;
        }
    }
    let mut report = Vec::new();
    for limit in [20_u16, 100] {
        let paged = candidates_all(&base, &session, whole, limit)?;
        let p95 = p95(&paged.page_times);
        report.push(json!({
            "limit": limit,
            "pages": paged.page_times.len(),
            "candidates": paged.items.len(),
            "p95_ms": p95.as_millis(),
            "max_ms": paged.page_times.iter().max().copied().unwrap_or_default().as_millis(),
        }));
        if !cfg!(debug_assertions) {
            ensure(
                p95 <= P95_TARGET,
                &format!(
                    "limit {limit}: p95 warm page {} ms exceeds 250 ms",
                    p95.as_millis()
                ),
            )?;
        }
    }
    Ok(json!({
        "clip": "F02 looped 150 times (30 min), 640x360, 10 fps, mpeg4",
        "clip_build_ms": build_ms,
        "cold_calls": cold,
        "warm_pages_through_the_binary": report,
        "build_profile": if cfg!(debug_assertions) { "debug (target not asserted)" } else { "release" },
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

#[tokio::test]
#[ignore = "opt-in P08 candidates checkpoint; needs ffmpeg and ffprobe on PATH; reports to .vsift/e2e-runs"]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the journey order and the complete evidence record visible together"
)]
async fn candidates_checkpoint() -> TestResult {
    let started = Instant::now();
    let repository = repository().canonicalize()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_dir = repository
        .join(".vsift/e2e-runs")
        .join(format!("p08-candidates-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&run_dir)?;
    let root = OwnedRoot::new()?;
    let tools = MediaTools::discover();
    let whisper = Whisper::discover();
    let mut stages = Vec::new();

    let clock = Instant::now();
    let fixtures_base = root.base("fixtures");
    let fixtures = tools
        .as_ref()
        .map_or_else(blocked, |tools| fixtures_stage(&fixtures_base, tools));
    let sessions = fixtures
        .as_ref()
        .map(|(_, sessions)| sessions.clone())
        .unwrap_or_default();
    stages.push(stage(
        "p08_candidates_fixtures",
        clock,
        fixtures.map(|(evidence, _)| evidence),
    ));

    let clock = Instant::now();
    let result = tools.as_ref().map_or_else(blocked, |tools| {
        lead_lag_stage(&root, tools, whisper.as_ref())
    });
    stages.push(stage("p08_lead_lag", clock, result));

    let clock = Instant::now();
    let result = match tools.as_ref() {
        Some(tools) => budget_stage(&root, tools).await,
        None => blocked(),
    };
    stages.push(stage("p08_candidates_budget", clock, result));

    let clock = Instant::now();
    let result = match (tools.as_ref(), sessions.get("F02")) {
        (Some(_), Some(session)) => stream_and_bundle_stage(&fixtures_base, session),
        (Some(_), None) => Err(failed("the fixtures stage opened no F02 session")),
        (None, _) => blocked(),
    };
    stages.push(stage("p08_candidates_stream_and_bundle", clock, result));

    let clock = Instant::now();
    let result = match tools.as_ref() {
        Some(_) if fixtures_base.join("sessions").exists() => malformed_stage(&fixtures_base),
        Some(_) => Err(failed("the fixtures stage prepared no base")),
        None => blocked(),
    };
    stages.push(stage("p08_candidates_malformed", clock, result));

    let clock = Instant::now();
    let result = tools
        .as_ref()
        .map_or_else(blocked, |tools| motion_stage(&root, tools));
    stages.push(stage("p08_candidates_motion", clock, result));

    let clock = Instant::now();
    let result = tools
        .as_ref()
        .map_or_else(blocked, |tools| s11_stage(&root, tools));
    stages.push(stage("p08_candidates_s11", clock, result));

    let status_of = |wanted: &str| stages.iter().any(|entry| entry["status"] == wanted);
    let overall = if status_of("failed") {
        "failed"
    } else if status_of("blocked") {
        "blocked"
    } else {
        "passed"
    };
    stages.push(json!({"name": "p08_candidates", "status": overall}));
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
        "checkpoint": "P08 visual candidates",
        "fixture_manifest": {
            "schema_version": manifest["schema_version"],
            "corpus_id": manifest["corpus_id"],
        },
        "fixtures": "F01-F10 and F12, the F03/F04/F05/F09 speech variants with SubRip files written from the frozen scripts, F11-damaged-tail.mp4, F01-audio-only.m4a, and scroll, zoom, 3-minute and 30-minute clips built at run time",
        "os": env::consts::OS,
        "architecture": env::consts::ARCH,
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "resource_profile": "each CLI call killed after 900 s; empty PATH; each window decode under its 120 s deadline",
        "vsift_version": env!("CARGO_PKG_VERSION"),
        "local_asr_variant": whisper.is_some(),
        "authorization": "opt-in cargo test invocation; setup configure writes only to isolated temporary per-user bases; clips are built in a temporary directory; no install, download or network access",
        "prior_checkpoints": ["P07: p07_transcript_e2e", "P07: p07_local_asr_e2e", "P08: p08_search_e2e"],
        "stages": stages,
        "coverage_gaps": [
            "F04-E02, F05-E02 and F12-E02 are drawn with the pixels of the state before them and are reported, not gated (issue #159)",
            "Real screen recordings with heavier compression noise are not in the corpus"
        ],
        "future_stages": future_stages,
        "overall": overall,
        "complete_journey": "not_implemented",
        "elapsed_ms": started.elapsed().as_millis(),
    });
    let report_path = run_dir.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "P08 candidates checkpoint report: {}",
        report_path.display()
    );
    println!("p08_candidates: {overall}");
    if overall == "passed" {
        Ok(())
    } else {
        Err(format!(
            "P08 candidates checkpoint {overall}; see {}",
            report_path.display()
        )
        .into())
    }
}
