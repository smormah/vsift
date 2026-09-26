//! Opt-in cumulative P07 checkpoint: the local-ASR stage of A-08.
//!
//! Journeys drive the compiled `vsift` binary exactly as a headless agent
//! would, with isolated per-user bases and an empty `PATH`: `FFmpeg`, `FFprobe`,
//! whisper.cpp and the model are registered with `setup configure` and
//! `setup configure-model`. A plain `ingest` opens each speech clip, and
//! `transcript retranscribe` is the only command that runs local ASR (D1).
//! The checkpoint cites the speech windows of the P07 speech fixtures
//! (`fixtures/corpus/generated/*-speech.*`, spans from `speech-provenance.json`,
//! words from the frozen scripts in `fixtures/corpus/manifest.json`), splices
//! a bounded retranscription and reads the older revision back (T-06), streams
//! the result, retains and validates the session, checks a real multi-chunk
//! seam (T-03), and proves a supplied-transcript import never touches a broken
//! whisper.cpp and that a missing model fails typed with no revision (T-05).
//!
//! ```console
//! VSIFT_TEST_WHISPER_CLI=<absolute whisper-cli path>
//! VSIFT_TEST_WHISPER_MODEL=<absolute ggml-base.bin or ggml-base-q5_1.bin path>
//! cargo test --release -p vsift-cli --locked --test p07_local_asr_e2e -- --ignored --nocapture
//! ```
//!
//! A release build is recommended: the 148 MB model is hashed before and after
//! every run, which dominates a debug build. The run writes a bounded report
//! to `.vsift/e2e-runs/p07-local-asr-<run-id>/report.json`. A journey that
//! cannot run here is `blocked`, never `passed`, and the test fails unless
//! every journey passed.

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
use vsift_infrastructure::{ExecutableResolver, TrustedExecutable};

type TestResult = Result<(), Box<dyn Error>>;

/// Upper bound for one CLI invocation; a prompt or hang is killed and fails.
/// Recognition allows 120 s per 30 s chunk, so a long clip needs minutes.
const CLI_DEADLINE: Duration = Duration::from_mins(15);
const OWNED_PREFIX: &str = "vsift-p07-asr-e2e-";
const MAX_DIAGNOSTIC_CHARS: usize = 240;
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
/// How far a recognised segment may lie outside the generator's speech span:
/// whisper's base model reports coarse times and ends a segment at its next
/// timestamp token.
const SPAN_TOLERANCE_US: u64 = 1_000_000;
const BLOCKED: &str = "set VSIFT_TEST_WHISPER_CLI and VSIFT_TEST_WHISPER_MODEL to absolute paths and put ffmpeg and ffprobe on PATH";
/// Gap between concatenated utterances in the seam clip.
const SEAM_GAP_US: u64 = 300_000;
/// Utterances of the seam clip, in order; F07's speech crosses the 25-30 s
/// overlap of the first two R0 chunks. Each word must be heard exactly once.
const SEAM_UTTERANCES: [(&str, &[&str]); 6] = [
    ("F03", &["recalculation"]),
    ("F04", &["header"]),
    ("F05", &["banner"]),
    ("F07", &["orange", "spikes", "median"]),
    ("F02", &["queue"]),
    ("F01", &["healthy"]),
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

    fn base(&self, journey: &str) -> PathBuf {
        self.0.join(journey)
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

/// Everything local ASR needs, found or selected for this run.
struct Tools {
    ffmpeg: TrustedExecutable,
    ffprobe: TrustedExecutable,
    whisper: PathBuf,
    model: PathBuf,
}

impl Tools {
    fn discover() -> Option<Self> {
        let resolver = ExecutableResolver::from_current_path();
        let absolute = |name: &str| {
            env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute() && path.is_file())
        };
        Some(Self {
            ffmpeg: resolver.resolve(OsStr::new("ffmpeg")).ok()?,
            ffprobe: resolver.resolve(OsStr::new("ffprobe")).ok()?,
            whisper: absolute("VSIFT_TEST_WHISPER_CLI")?,
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

/// Registers the tools and the model in `base` and returns `setup check`.
fn prepare(base: &Path, tools: &Tools, model: bool) -> Result<Value, StageStop> {
    for (dependency, path) in [
        ("ffmpeg", tools.ffmpeg.path()),
        ("ffprobe", tools.ffprobe.path()),
        ("whisper", tools.whisper.as_path()),
    ] {
        let (code, _) = run_json(
            vsift(base)?
                .args(["setup", "configure", dependency, "--executable"])
                .arg(path)
                .arg("--json"),
        )?;
        ensure(code == Some(0), "setup configure did not succeed")?;
    }
    if model {
        let (code, _) = run_json(
            vsift(base)?
                .args(["setup", "configure-model", "--file"])
                .arg(&tools.model)
                .arg("--json"),
        )?;
        ensure(code == Some(0), "setup configure-model did not succeed")?;
    }
    let (code, check) = run_json(vsift(base)?.args(["setup", "check", "--json"]))?;
    ensure(code == Some(0), "setup check did not exit 0")?;
    Ok(check)
}

/// D4: with everything registered, the first `setup check` runs the
/// local-ASR verification and records the pass; the next reports the record.
/// Returns the reviewed profile the registered model is.
fn setup_verifies_local_asr(base: &Path, first: &Value) -> Result<Value, StageStop> {
    conforms("setup-check-response.schema.json", first)?;
    let profile = first["local_asr"]["model"]["profile"].clone();
    ensure(
        first["local_asr"]["model"]["status"] == "known_pinned"
            && (profile == "base" || profile == "base_q5_1"),
        "setup check did not identify the model as a reviewed profile",
    )?;
    ensure(
        first["local_asr"]["verification"]["status"] == "verified"
            && first["local_asr"]["verification"]["source"] == "ran_now",
        &format!(
            "the first setup check did not verify local ASR: {}",
            first["local_asr"]["verification"]
        ),
    )?;
    let started = Instant::now();
    let (code, second) = run_json(vsift(base)?.args(["setup", "check", "--json"]))?;
    let recorded_ms = started.elapsed().as_millis();
    ensure(code == Some(0), "the second setup check did not exit 0")?;
    ensure(
        second["local_asr"]["verification"]["source"] == "recorded",
        "the second setup check did not report the recorded pass",
    )?;
    ensure(
        first["verification_scope"] == "executable_probe_only"
            && first["local_asr_model"] == "not_checked",
        "the v1 constant fields changed",
    )?;
    Ok(json!({
        "model_profile": profile,
        "first": first["local_asr"]["verification"],
        "second": second["local_asr"]["verification"],
        "recorded_check_ms": recorded_ms,
    }))
}

fn ingest(base: &Path, video: &Path) -> Result<String, StageStop> {
    let (code, opened) = run_json(vsift(base)?.arg("ingest").arg(video).arg("--json"))?;
    ensure(code == Some(0), "plain ingest failed")?;
    ensure(
        opened["data"].get("transcript").is_none(),
        "a plain ingest produced a transcript",
    )?;
    opened["data"]["session_id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| failed("session missing"))
}

/// Runs `transcript retranscribe` and returns its data and elapsed time.
fn retranscribe(
    base: &Path,
    session: &str,
    range: Option<(u64, u64)>,
) -> Result<(Value, u128), StageStop> {
    let mut command = vsift(base)?;
    command.args(["transcript", "retranscribe", session]);
    if let Some((from, to)) = range {
        command
            .args(["--from", &from.to_string()])
            .args(["--to", &to.to_string()]);
    }
    let started = Instant::now();
    let (code, result) = run_json(command.arg("--json"))?;
    let elapsed = started.elapsed().as_millis();
    ensure(
        code == Some(0),
        &format!(
            "retranscribe failed: {} {}",
            result["error"]["code"], result["error"]["remediation"][0]["summary"]
        ),
    )?;
    conforms("operation-response.schema.json", &result)?;
    conforms("transcript-retranscribe-data.schema.json", &result["data"])?;
    Ok((result["data"].clone(), elapsed))
}

/// Reads every segment of a revision (the newest when `revision` is `None`).
fn read_all(
    base: &Path,
    session: &str,
    to_us: u64,
    revision: Option<&str>,
) -> Result<Value, StageStop> {
    let mut command = vsift(base)?;
    command.args(["transcript", "get", session, "--from", "0", "--to"]);
    command.arg(to_us.to_string()).args(["--limit", "100"]);
    if let Some(revision) = revision {
        command.args(["--revision", revision]);
    }
    let (code, page) = run_json(command.arg("--json"))?;
    ensure(code == Some(0), "transcript get failed")?;
    conforms("transcript-get-data.schema.json", &page["data"])?;
    ensure(
        page["data"]["next_cursor"].is_null(),
        "one page did not hold the whole transcript",
    )?;
    Ok(page["data"].clone())
}

/// Lowercase words with punctuation removed, joined by single spaces.
fn normalised(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|character| character.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn page_text(page: &Value) -> String {
    normalised(
        &page["items"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item["text"].as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default(),
    )
}

/// The generator's speech span of a fixture, from `speech-provenance.json`.
fn speech_span(fixture_id: &str) -> Result<(u64, u64), StageStop> {
    let provenance: Value = serde_json::from_slice(&fs::read(fixture("speech-provenance.json"))?)?;
    let variant = provenance["assembly"]["variants"]
        .as_array()
        .and_then(|variants| {
            variants
                .iter()
                .find(|variant| variant["fixture"] == fixture_id)
        })
        .ok_or_else(|| failed("fixture missing from speech provenance"))?;
    Ok((
        variant["speech_start_us"]
            .as_u64()
            .ok_or_else(|| failed("no speech start"))?,
        variant["speech_end_us"]
            .as_u64()
            .ok_or_else(|| failed("no speech end"))?,
    ))
}

/// Every segment lies within the speech span, give or take the tolerance.
fn within_span(page: &Value, (start, end): (u64, u64)) -> Result<(), StageStop> {
    let items = page["items"]
        .as_array()
        .ok_or_else(|| failed("items missing"))?;
    ensure(!items.is_empty(), "no segment was recognised")?;
    for item in items {
        conforms("transcript-segment.schema.json", item)?;
        let (from, to) = (
            item["start_us"].as_u64().unwrap_or(u64::MAX),
            item["end_us"].as_u64().unwrap_or(u64::MAX),
        );
        ensure(
            from + SPAN_TOLERANCE_US >= start && to <= end + SPAN_TOLERANCE_US,
            &format!("a segment [{from}, {to}) lies outside the speech span [{start}, {end})"),
        )?;
    }
    Ok(())
}

fn contains_all(text: &str, words: &[&str]) -> Result<(), StageStop> {
    for word in words {
        ensure(
            text.contains(&normalised(word)),
            &format!("the transcript lacks {word:?}: {text}"),
        )?;
    }
    Ok(())
}

/// A-08 whole-file stage: plain ingest of F05, then local ASR of the whole
/// file, citing the speech window.
fn whole_file(base: &Path, duration: u64) -> Result<(String, Value, u128), StageStop> {
    let session = ingest(base, &fixture("F05-speech.mp4"))?;
    let (data, elapsed) = retranscribe(base, &session, None)?;
    let revision = &data["revision"];
    ensure(revision["revision"] == 1, "the first run is not revision 1")?;
    ensure(revision["supersedes"].is_null(), "revision 1 supersedes")?;
    ensure(
        data["requested_range"].is_null(),
        "a whole-file run reported a requested range",
    )?;
    ensure(
        revision["local_asr"]["model_profile"] == "base"
            || revision["local_asr"]["model_profile"] == "base_q5_1",
        "a reviewed pinned model did not run",
    )?;
    let page = read_all(base, &session, duration, None)?;
    within_span(&page, speech_span("F05")?)?;
    // "invoice 4407" is a reviewed known base-model miss (T-04): on some CPU
    // backends whisper.cpp hears "in voice", so the journey checks only words
    // every reviewed host has heard; accuracy itself is the qualification test's.
    contains_all(&page_text(&page), &["success banner", "submit"])?;
    Ok((session, page, elapsed))
}

/// T-06 bounded stage: retranscribing one range of revision 1 gives a spliced
/// revision 2 that is the default read, while revision 1 stays readable.
fn bounded_revision(base: &Path, session: &str, first: &Value, duration: u64) -> StageResult {
    let first_items = first["items"]
        .as_array()
        .ok_or_else(|| failed("items missing"))?;
    let last = first_items
        .last()
        .ok_or_else(|| failed("revision 1 is empty"))?;
    // Retranscribe from inside the last segment to the end: the range widens
    // to that segment's start, and every earlier segment is carried.
    let from = last["start_us"].as_u64().unwrap_or(0) + 100_000;
    let (data, elapsed) = retranscribe(base, session, Some((from, duration)))?;
    let revision = &data["revision"];
    ensure(
        revision["revision"] == 2,
        "the bounded run is not revision 2",
    )?;
    ensure(
        revision["supersedes"] == first["revision"]["revision_id"],
        "revision 2 does not supersede revision 1",
    )?;
    ensure(
        revision["replaced_range"]["from_us"] == last["start_us"],
        "the range was not widened to the whole segment",
    )?;
    let carried = revision["carried_segment_count"].as_u64().unwrap_or(0);
    ensure(
        carried == u64::try_from(first_items.len() - 1).unwrap_or(u64::MAX),
        "not every segment before the range was carried",
    )?;
    let newest = read_all(base, session, duration, None)?;
    ensure(
        newest["revision"]["revision_id"] == revision["revision_id"],
        "transcript get does not default to the newest revision",
    )?;
    for (item, original) in newest["items"]
        .as_array()
        .ok_or_else(|| failed("items missing"))?
        .iter()
        .zip(first_items)
        .take(first_items.len() - 1)
    {
        ensure(
            item["carried_from"]["segment_id"] == original["segment_id"]
                && item["segment_id"] != original["segment_id"]
                && item["text"] == original["text"],
            "a carried segment does not name its original",
        )?;
    }
    let older = read_all(
        base,
        session,
        duration,
        first["revision"]["revision_id"].as_str(),
    )?;
    ensure(
        older["items"] == first["items"],
        "revision 1 changed after it was superseded",
    )?;
    contains_all(&page_text(&newest), &["success banner", "submit"])?;
    Ok(json!({
        "revision_1": first["revision"]["revision_id"],
        "revision_2": revision["revision_id"],
        "requested_range_us": [from, duration],
        "replaced_range": revision["replaced_range"],
        "carried_segments": carried,
        "recognised_segments": data["recognised_segment_count"],
        "retranscribe_ms": elapsed,
        "old_citation_resolves": true,
    }))
}

/// The newest revision as an evidence stream, then retain and validate.
fn stream_and_retain(base: &Path, session: &str, duration: u64) -> StageResult {
    let output = vsift(base)?
        .args(["transcript", "get", session, "--from", "0", "--to"])
        .arg(duration.to_string())
        .args(["--limit", "100", "--events", "jsonl"])
        .output()?;
    ensure(output.status.code() == Some(0), "the stream failed")?;
    ensure(
        output.stderr.is_empty(),
        "the stream wrote to standard error",
    )?;
    let text = std::str::from_utf8(&output.stdout)?;
    let lines = text
        .trim_end_matches('\n')
        .split('\n')
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    let (terminal, records) = lines
        .split_last()
        .ok_or_else(|| failed("the stream is empty"))?;
    for (sequence, record) in (0_u64..).zip(records) {
        conforms("evidence-event.schema.json", record)?;
        ensure(
            record["sequence"].as_u64() == Some(sequence)
                && record["key"] == record["record"]["segment_id"],
            "an evidence event is out of sequence or mis-keyed",
        )?;
    }
    conforms("terminal-event.schema.json", terminal)?;
    let count = u64::try_from(records.len()).ok();
    ensure(
        terminal["result"]["data"]["record_count"].as_u64() == count,
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
    ensure(
        validated["data"]["artifact_count"] == 2,
        "the bundle does not hold both revisions",
    )?;
    let manifest: Value = serde_json::from_slice(&fs::read(bundle.join("bundle.json"))?)?;
    let mut versions = Vec::new();
    for artifact in manifest["artifacts"]
        .as_array()
        .ok_or_else(|| failed("artifacts missing"))?
    {
        let name = artifact["name"].as_str().unwrap_or_default();
        ensure(
            name.starts_with("artifact-") && !name.contains(['/', '\\']),
            "a record has an unexpected name",
        )?;
        let record: Value = serde_json::from_slice(&fs::read(bundle.join(name))?)?;
        conforms("bundle-transcript-record.schema.json", &record)?;
        versions.push(record["schema_version"].clone());
    }
    Ok(json!({
        "jsonl_evidence_records": records.len(),
        "bundle_validated": true,
        "bundle_record_versions": versions,
    }))
}

/// F08: noisy English with a Spanish sentence.
fn noisy_bilingual(base: &Path) -> StageResult {
    let session = ingest(base, &fixture("F08-speech.mp4"))?;
    let (data, elapsed) = retranscribe(base, &session, None)?;
    let page = read_all(base, &session, 18_000_000, None)?;
    within_span(&page, speech_span("F08")?)?;
    let text = page_text(&page);
    contains_all(&text, &["731", "identificador"])?;
    Ok(json!({
        "segments": data["revision"]["segment_count"],
        "language": data["revision"]["language"],
        "warnings": data["revision"]["warnings"],
        "retranscribe_ms": elapsed,
        "text": bounded(&text),
    }))
}

/// F09: audio starts 0.75 s into the container and speech at 4.0-6.0 s.
///
/// Every time must be anchored at the observed first decoded sample (0.75 s),
/// never at zero. The base model starts a segment that follows leading
/// silence at the beginning of its audio, so F09's segment starts at 0.75 s
/// rather than 4.0 s: the check is that it covers the speech, starts no
/// earlier than the audio and ends by the speech end plus the tolerance.
/// Tighter start times are part of the accuracy measurement (T-04, 3c).
fn offset_audio(base: &Path) -> StageResult {
    let session = ingest(base, &fixture("F09-speech.mkv"))?;
    let (data, elapsed) = retranscribe(base, &session, None)?;
    let page = read_all(base, &session, 12_000_000, None)?;
    let (start, end) = speech_span("F09")?;
    let audio_offset = 750_000;
    for item in page["items"]
        .as_array()
        .ok_or_else(|| failed("items missing"))?
    {
        let (from, to) = (
            item["start_us"].as_u64().unwrap_or(u64::MAX),
            item["end_us"].as_u64().unwrap_or(u64::MAX),
        );
        ensure(
            item["alignment"]["chunk_audio_start_us"] == audio_offset,
            "the chunk's audio is not anchored at the observed 0.75 s start",
        )?;
        ensure(
            from >= audio_offset && from < end && to > start && to <= end + SPAN_TOLERANCE_US,
            &format!("a segment [{from}, {to}) does not cover the speech [{start}, {end})"),
        )?;
    }
    contains_all(&page_text(&page), &["visible"])?;
    let span = (start, end);
    let first = &page["items"][0];
    Ok(json!({
        "speech_span_us": [span.0, span.1],
        "first_segment_us": [first["start_us"], first["end_us"]],
        "first_chunk_audio_start_us": first["alignment"]["chunk_audio_start_us"],
        "segments": data["revision"]["segment_count"],
        "retranscribe_ms": elapsed,
    }))
}

/// Builds the seam clip from the committed speech utterances: each resampled
/// to 16 kHz, followed by a short silence, over a black video. Returns the
/// clip, its duration and where each utterance was placed.
fn build_seam_clip(tools: &Tools, work: &Path) -> Result<(PathBuf, u64, Vec<Value>), StageStop> {
    let clip = work.join("seam-clip.mp4");
    let mut arguments: Vec<String> = vec!["-v".into(), "error".into(), "-y".into()];
    let mut filters = Vec::new();
    let mut expected = Vec::new();
    let mut cursor_us = 0_u64;
    for (index, (fixture_id, _)) in SEAM_UTTERANCES.iter().enumerate() {
        let wav = fixture(&format!("speech/{fixture_id}-utterance.wav"));
        let duration = probe_duration_us(tools, &wav)?;
        arguments.extend(["-i".into(), wav.to_string_lossy().into_owned()]);
        filters.push(format!(
            "[{index}:a]aresample=16000,apad=pad_dur={:.3}[a{index}];",
            to_seconds(SEAM_GAP_US)
        ));
        expected.push(json!({
            "fixture": fixture_id,
            "start_us": cursor_us,
            "end_us": cursor_us + duration,
        }));
        cursor_us += duration + SEAM_GAP_US;
    }
    filters.extend((0..SEAM_UTTERANCES.len()).map(|index| format!("[a{index}]")));
    filters.push(format!(
        "concat=n={}:v=0:a=1[speech]",
        SEAM_UTTERANCES.len()
    ));
    let filter = filters.concat();
    arguments.extend([
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!(
            "color=c=black:s=320x240:r=10:d={:.3}",
            to_seconds(cursor_us)
        ),
        "-filter_complex".into(),
        filter,
        "-map".into(),
        format!("{}:v", SEAM_UTTERANCES.len()),
        "-map".into(),
        "[speech]".into(),
        // FFmpeg's native MPEG-4 Part 2 encoder is in every build, including the
        // pinned CI builds that omit libx264; the black video only carries the speech.
        "-c:v".into(),
        "mpeg4".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-c:a".into(),
        "aac".into(),
        "-shortest".into(),
    ]);
    arguments.push(clip.to_string_lossy().into_owned());
    let status = Process::new(tools.ffmpeg.path())
        .args(&arguments)
        .status()?;
    ensure(status.success(), "ffmpeg could not build the seam clip")?;
    Ok((clip, cursor_us, expected))
}

/// T-03 on real speech: a clip long enough for two R0 chunks, with F07's
/// sentence across their 25-30 s overlap, loses and doubles nothing.
fn seam(base: &Path, tools: &Tools, work: &Path) -> StageResult {
    let (clip, cursor_us, expected) = build_seam_clip(tools, work)?;

    let session = ingest(base, &clip)?;
    let (data, elapsed) = retranscribe(base, &session, None)?;
    let chunks = data["revision"]["local_asr"]["chunk_count"]
        .as_u64()
        .unwrap_or(0);
    ensure(chunks >= 2, "the seam clip did not need two chunks")?;
    let page = read_all(base, &session, cursor_us + 1_000_000, None)?;
    let text = page_text(&page);
    let mut counts = Vec::new();
    for (fixture_id, words) in SEAM_UTTERANCES {
        for word in words {
            let occurrences = text.split(' ').filter(|token| token == word).count();
            counts.push(json!({"fixture": fixture_id, "word": word, "count": occurrences}));
            ensure(
                occurrences == 1,
                &format!("{word:?} ({fixture_id}) was heard {occurrences} times: {text}"),
            )?;
        }
    }
    // The sentence across the seam comes from chunk 0 or chunk 1 once.
    let seam_owner: Vec<Value> = page["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    item["start_us"].as_u64().unwrap_or(0) < 30_000_000
                        && item["end_us"].as_u64().unwrap_or(0) > 25_000_000
                })
                .map(|item| {
                    json!([
                        item["start_us"],
                        item["end_us"],
                        item["alignment"]["chunk_index"]
                    ])
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({
        "clip_duration_us": cursor_us,
        "utterances": expected,
        "chunks": chunks,
        "warnings": data["revision"]["warnings"],
        "word_counts": counts,
        "segments_in_overlap": seam_owner,
        "retranscribe_ms": elapsed,
    }))
}

fn to_seconds(micros: u64) -> f64 {
    let seconds = u32::try_from(micros / 1_000).unwrap_or(u32::MAX);
    f64::from(seconds) / 1_000.0
}

fn probe_duration_us(tools: &Tools, media: &Path) -> Result<u64, StageStop> {
    let output = Process::new(tools.ffprobe.path())
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(media)
        .output()?;
    let text = std::str::from_utf8(&output.stdout)?.trim().to_owned();
    let (seconds, fraction) = text.split_once('.').unwrap_or((text.as_str(), "0"));
    let micros: String = fraction.chars().chain("000000".chars()).take(6).collect();
    Ok(
        seconds.parse::<u64>().map_err(|_| failed("bad duration"))? * 1_000_000
            + micros.parse::<u64>().map_err(|_| failed("bad duration"))?,
    )
}

/// The tripwire: whisper registered as a program that is not whisper, yet an
/// F10 `SubRip` import and its reads succeed, because they never touch it.
fn supplied_transcript_ignores_whisper(root: &OwnedRoot, tools: &Tools) -> StageResult {
    let base = root.base("tripwire");
    let broken = Tools {
        ffmpeg: tools.ffmpeg.clone(),
        ffprobe: tools.ffprobe.clone(),
        whisper: tools.ffprobe.path().to_path_buf(),
        model: tools.model.clone(),
    };
    prepare(&base, &broken, false)?;
    let (code, opened) = run_json(
        vsift(&base)?
            .arg("ingest")
            .arg(fixture("F10.mp4"))
            .arg("--transcript")
            .arg(repository().join("fixtures/corpus/transcripts/F10.srt"))
            .args(["--transcript-offset", "500000", "--json"]),
    )?;
    ensure(
        code == Some(0),
        &format!("the F10 import failed: {}", opened["error"]["code"]),
    )?;
    let session = opened["data"]["session_id"]
        .as_str()
        .ok_or_else(|| failed("session missing"))?;
    let page = read_all(&base, session, 12_000_000, None)?;
    ensure(
        page_text(&page).contains("dialog r17"),
        "the imported transcript does not cite dialog R-17",
    )?;
    Ok(json!({
        "whisper_registered_as": "ffprobe (not whisper)",
        "import": "complete",
        "segments": page["revision"]["segment_count"],
    }))
}

/// T-05: whisper without a model fails typed, before any work, with no revision.
fn missing_model(root: &OwnedRoot, tools: &Tools) -> StageResult {
    let base = root.base("missing-model");
    let check = prepare(&base, tools, false)?;
    conforms("setup-check-response.schema.json", &check)?;
    ensure(
        check["local_asr"]["model"]["status"] == "not_selected"
            && check["local_asr"]["verification"]["not_run_reason"] == "model_not_selected",
        "setup check did not report the missing model (D4)",
    )?;
    let session = ingest(&base, &fixture("F01-speech.mp4"))?;
    let (code, result) = run_json(
        vsift(&base)?
            .args(["transcript", "retranscribe", &session])
            .arg("--json"),
    )?;
    ensure(code == Some(2), "a missing model did not exit 2")?;
    conforms("operation-response.schema.json", &result)?;
    ensure(
        result["error"]["code"] == "MISSING_CAPABILITY",
        "a missing model was not MISSING_CAPABILITY",
    )?;
    let summary = result["error"]["remediation"][0]["summary"]
        .as_str()
        .unwrap_or_default();
    ensure(
        summary.contains("configure-model"),
        "the remediation does not name setup configure-model",
    )?;
    let (_, status) = run_json(vsift(&base)?.args(["session", "status", &session, "--json"]))?;
    ensure(
        status["data"]["artifact_count"] == 0,
        "a failed run committed a revision",
    )?;
    Ok(json!({"code": result["error"]["code"], "artifact_count": 0}))
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
    Err(StageStop::Blocked(BLOCKED.to_owned()))
}

#[tokio::test]
#[ignore = "opt-in P07 local-ASR checkpoint; needs ffmpeg/ffprobe on PATH, VSIFT_TEST_WHISPER_CLI and VSIFT_TEST_WHISPER_MODEL; reports to .vsift/e2e-runs"]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the journey order and the complete evidence record visible together"
)]
async fn local_asr_checkpoint() -> TestResult {
    let started = Instant::now();
    let repository = repository().canonicalize()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_dir = repository
        .join(".vsift/e2e-runs")
        .join(format!("p07-local-asr-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&run_dir)?;
    let root = OwnedRoot::new()?;
    let tools = Tools::discover();
    let base = root.base("asr");
    let mut stages = Vec::new();

    let clock = Instant::now();
    let prepared = tools
        .as_ref()
        .map_or_else(blocked, |tools| prepare(&base, tools, true));
    let setup = prepared.as_ref().ok().cloned();
    let verified = prepared.and_then(|check| setup_verifies_local_asr(&base, &check));
    let model_profile = verified
        .as_ref()
        .map_or(Value::Null, |evidence| evidence["model_profile"].clone());
    let setup = setup.filter(|_| verified.is_ok());
    stages.push(stage("p07_local_asr_setup", clock, verified));

    let duration = 20_000_000;
    let clock = Instant::now();
    let whole = match (&tools, &setup) {
        (Some(_), Some(_)) => whole_file(&base, duration),
        _ => blocked(),
    };
    let (whole_stage, first) = match whole {
        Ok((session, page, elapsed)) => (
            Ok(json!({
                "fixture": "F05-speech.mp4",
                "revision_id": page["revision"]["revision_id"],
                "segments": page["revision"]["segment_count"],
                "first_citation": [page["items"][0]["start_us"], page["items"][0]["end_us"], page["items"][0]["text"]],
                "speech_span_us": speech_span("F05").map_or(Value::Null, |span| json!([span.0, span.1])),
                "retranscribe_ms": elapsed,
            })),
            Some((session, page)),
        ),
        Err(error) => (Err(error), None),
    };
    stages.push(stage("p07_local_asr_whole_file", clock, whole_stage));

    let clock = Instant::now();
    let result = match &first {
        Some((session, page)) => bounded_revision(&base, session, page, duration),
        None => blocked(),
    };
    stages.push(stage("p07_local_asr_bounded_revision", clock, result));

    let clock = Instant::now();
    let result = match &first {
        Some((session, _)) => stream_and_retain(&base, session, duration),
        None => blocked(),
    };
    stages.push(stage("p07_local_asr_stream_and_bundle", clock, result));

    for (name, journey) in [
        (
            "p07_local_asr_f08_noise_spanish",
            noisy_bilingual as fn(&Path) -> StageResult,
        ),
        ("p07_local_asr_f09_offset", offset_audio),
    ] {
        let clock = Instant::now();
        let result = match &setup {
            Some(_) => journey(&base),
            None => blocked(),
        };
        stages.push(stage(name, clock, result));
    }

    let clock = Instant::now();
    let result = match (&tools, &setup) {
        (Some(tools), Some(_)) => seam(&base, tools, &root.0),
        _ => blocked(),
    };
    stages.push(stage("p07_local_asr_multi_chunk_seam", clock, result));

    let clock = Instant::now();
    let result = tools.as_ref().map_or_else(blocked, |tools| {
        supplied_transcript_ignores_whisper(&root, tools)
    });
    stages.push(stage("p07_local_asr_whisper_tripwire", clock, result));

    let clock = Instant::now();
    let result = tools
        .as_ref()
        .map_or_else(blocked, |tools| missing_model(&root, tools));
    stages.push(stage("p07_local_asr_missing_model", clock, result));

    let status_of = |wanted: &str| stages.iter().any(|entry| entry["status"] == wanted);
    let overall = if status_of("failed") {
        "failed"
    } else if status_of("blocked") {
        "blocked"
    } else {
        "passed"
    };
    stages.push(json!({"name": "p07_local_asr", "status": overall}));
    let future_stages: Vec<_> = [
        "p09_source_reinspection",
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
    let checks = setup.clone().unwrap_or(Value::Null);
    let report = json!({
        "schema_version": 1,
        "checkpoint": "P07 local ASR",
        "fixture_manifest": {
            "schema_version": manifest["schema_version"],
            "corpus_id": manifest["corpus_id"],
        },
        "fixtures": "F05-speech.mp4, F08-speech.mp4, F09-speech.mkv, F01-speech.mp4, F10.mp4 with F10.srt, and a seam clip built at run time from the speech utterances",
        "os": env::consts::OS,
        "architecture": env::consts::ARCH,
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "resource_profile": "each CLI call killed after 900 s; empty PATH; whisper.cpp under the 120 s per-chunk deadline",
        "vsift_version": env!("CARGO_PKG_VERSION"),
        "ffmpeg_version": checks["dependencies"][0]["detail"],
        "ffprobe_version": checks["dependencies"][1]["detail"],
        "whisper_version": checks["dependencies"][2]["detail"],
        "model_profile": model_profile,
        "client_versions": [],
        "authorization": "opt-in cargo test invocation; setup configure writes only to isolated temporary per-user bases; no install, download or network access",
        "prior_checkpoints": ["P04: p04_media_e2e", "P05: p05_session_e2e", "P06: p06_setup_e2e", "P07: p07_transcript_e2e"],
        "stages": stages,
        "coverage_gaps": [
            "WER, critical terms and timing (T-04) are measured by the opt-in p07_asr_qualification test, not by this checkpoint",
            "Search and candidates are checked by the P08 checkpoints (p08_search_e2e, p08_candidates_e2e); visual refinement belongs to P09"
        ],
        "future_stages": future_stages,
        "overall": overall,
        "complete_journey": "not_implemented",
        "elapsed_ms": started.elapsed().as_millis(),
    });
    let report_path = run_dir.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("P07 local-ASR checkpoint report: {}", report_path.display());
    println!("p07_local_asr: {overall}");
    if overall == "passed" {
        Ok(())
    } else {
        Err(format!(
            "P07 local-ASR checkpoint {overall}; see {}",
            report_path.display()
        )
        .into())
    }
}
