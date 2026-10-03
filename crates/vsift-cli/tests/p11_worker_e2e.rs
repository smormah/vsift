//! Opt-in P11 checkpoint: the single-host worker run (X-07, X-08, X-11,
//! O-01, O-03 and O-04 through the public CLI, with real tools).
//!
//! Every stage drives the compiled `vsift` binary as a supervisor would: an
//! isolated per-user base with `FFmpeg`, `FFprobe`, whisper.cpp and its model
//! registered by `setup configure`, an empty `PATH`, a worker workspace made
//! by `session init-workspace`, an operator input root holding the corpus
//! files and a bundle root. Expectations come from the frozen corpus truth
//! (the F01, F03 and F10 manifest entries, F03's speech provenance) and from
//! an uninterrupted control batch, never from the results being scored.
//! The stages:
//!
//! - `p11_batch_mechanical`: one mixed `job batch` in an ephemeral workspace:
//!   F03's speech variant (ingest, local recognition, candidates, retain), F10
//!   with its SRT sidecar at +500 ms (ingest with import, candidates,
//!   retain), F01 (ingest, candidates, retain, close), a malformed line and a
//!   path climbing out of the input root. The exit is D5's (2: two refused
//!   lines) and each line is reported alone. Then the batch's outputs are used
//!   as an agent would: `search` for F03's critical term and for F10's dialog
//!   id, `candidates` near the hit, `frame get --candidate` inside the
//!   critical event, and `bundle validate` of every retained bundle, whose
//!   manifest digest must be the one the request recorded; every citation is
//!   checked against the frozen truth;
//! - `p11_admission_ladder`: four candidate requests over a 180 s clip at
//!   `--concurrency` 1, 2 and 4 in a four-unit workspace. The stream never has
//!   more requests in flight than the concurrency, and a sampler of the
//!   batch's provider processes never sees more weight than the capacity (a
//!   visual window's `FFmpeg` weighs 2, a probe 1), so at concurrency 4 at
//!   most two windows run although four requests are in flight. Each rung
//!   gives the same candidates;
//! - `p11_shutdown_and_redelivery`: a batch of two long recognitions (F03's
//!   speech variant looped to 81 s, lines 1 and 4), a visual request and an
//!   import runs once as a control; the same file in another workspace is
//!   stopped once the first recognition has a chunk checkpoint and the
//!   second has been admitted (`SIGTERM` on Unix, a console Ctrl-Break
//!   through `tools/send-console-ctrl.ps1` on Windows): it exits 6 within
//!   15 s with `termination_reason` `shutdown`, both recognitions' requests
//!   `cancelled`, and no provider left running. The first recognition is then
//!   finished by `job resume`; the file delivered again continues the second
//!   and completes, and its bundles hold exactly the control's segments and
//!   candidates; a third delivery replays every line unchanged;
//! - `p11_durable_workspace`: a durable workspace is created only on the
//!   qualified profile (Ubuntu 24.04 with local ext4), where a durable
//!   request runs and replays with `os_crash_durable` publication. Anywhere
//!   else the stage checks that the command refuses with
//!   `MISSING_CAPABILITY` and creates nothing, and reports `blocked` with
//!   that reason; it is not required off the profile.
//!
//! Clips are built without re-encoding (`-c copy`), so no encoder is needed.
//!
//! ```console
//! VSIFT_TEST_WHISPER_CLI=<abs> VSIFT_TEST_WHISPER_MODEL=<abs> \
//!   cargo test -p vsift-cli --locked --test p11_worker_e2e -- --ignored --nocapture
//! ```
//!
//! The run writes `.vsift/e2e-runs/p11-<run-id>/report.json` and prints
//! `p11_worker: passed` when every required stage passed. A stage that
//! cannot run here is `blocked`, never `passed`.

mod published_binary;

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    ffi::OsStr,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command as Process, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::{Value, json};
use vsift_infrastructure::{ExecutableResolver, FilesystemSessionStore, TrustedExecutable};

type TestResult = Result<(), Box<dyn Error>>;

/// Upper bound for one CLI invocation.
const CLI_DEADLINE: Duration = Duration::from_mins(10);
const OWNED_PREFIX: &str = "vsift-p11-worker-e2e-";
const MAX_DIAGNOSTIC_CHARS: usize = 400;
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SECOND: u64 = 1_000_000;
const MISSING_TOOLS: &str = "FFmpeg or FFprobe is not on PATH, or VSIFT_TEST_WHISPER_CLI and VSIFT_TEST_WHISPER_MODEL do not name an absolute whisper-cli and ggml model";
/// Longest event line but the terminal one (ADR 0021 section 7).
const MAX_EVENT_LINE_BYTES: usize = 65_536;
/// A stopped batch must end within this (O-04; the batch contract test's
/// budget: the providers' 5 s graceful and 5 s forced budgets and margin).
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(15);
/// After the batch ended, nothing it started may still run after this.
const DESCENDANT_BUDGET: Duration = Duration::from_secs(10);
/// Admission units of the mechanical and shutdown workspaces: enough that a
/// recognition's eight threads leave room for visual windows.
const WIDE_CAPACITY: u16 = 16;
/// Admission units of the ladder's workspace.
const LADDER_CAPACITY: u16 = 4;
/// Loops of F03-speech (9 s) in the long speech clip: 81 s, several chunks.
const SPEECH_LOOPS: u32 = 8;
/// Loops of F02 (12 s) in the visual clip: 180 s, three 60 s windows.
const VISUAL_LOOPS: u32 = 14;
/// The mechanical journey's speech fixture, as in the P09 and P10
/// checkpoints: F03's speech variant, the term the script says when the
/// cell changes and the critical event that shows it.
const SPEECH_FIXTURE: &str = "F03";
const SPEECH_TERM: &str = "127.50";
const SPEECH_EVENT: &str = "F03-E02";
const LEAD_LAG_US: u64 = 10 * SECOND;
/// Local speech recognition places segments within this much of the
/// generator's speech span (the P07 local-ASR checkpoint's tolerance).
const ASR_SPAN_TOLERANCE_US: u64 = SECOND;
/// F10's sidecar is shifted by the explicit +500 ms offset (P07).
const F10_OFFSET_US: i64 = 500_000;

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

    fn path(&self, name: &str) -> PathBuf {
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

fn corpus(name: &str) -> PathBuf {
    repository().join("fixtures/corpus").join(name)
}

fn u64_of(value: &Value) -> Result<u64, StageStop> {
    value
        .as_u64()
        .ok_or_else(|| failed("an expected integer is missing"))
}

fn str_of(value: &Value) -> Result<String, StageStop> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| failed("an expected string is missing"))
}

fn array_of(value: &Value) -> Result<&Vec<Value>, StageStop> {
    value
        .as_array()
        .ok_or_else(|| failed("an expected array is missing"))
}

/// The real tools this checkpoint needs.
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

    /// Loops the generated fixture `name` `loops` more times into `clip`
    /// without re-encoding.
    fn loop_clip(&self, name: &str, loops: u32, clip: &Path) -> Result<(), StageStop> {
        let output = Process::new(self.ffmpeg.path())
            .args(["-v", "error", "-nostdin", "-y", "-stream_loop"])
            .arg(loops.to_string())
            .arg("-i")
            .arg(corpus("generated").join(name))
            .args(["-c", "copy"])
            .arg(clip)
            .output()?;
        ensure(
            output.status.success(),
            &format!(
                "ffmpeg could not loop {name}: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        )
    }
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

/// One worker host: an isolated per-user base with the tools registered,
/// and an operator input root. Workspaces live beside them.
struct Host {
    base: PathBuf,
}

impl Host {
    fn new(base: PathBuf, tools: &Tools) -> Result<Self, StageStop> {
        fs::create_dir_all(base.join("user"))?;
        fs::create_dir_all(base.join("inputs"))?;
        let host = Self { base };
        for (dependency, path) in [
            ("ffmpeg", tools.ffmpeg.path()),
            ("ffprobe", tools.ffprobe.path()),
            ("whisper", tools.whisper.as_path()),
        ] {
            let (code, _) = run_json(
                host.command()?
                    .args(["setup", "configure", dependency, "--executable"])
                    .arg(path)
                    .arg("--json"),
            )?;
            ensure(code == Some(0), "setup configure did not succeed")?;
        }
        let (code, _) = run_json(
            host.command()?
                .args(["setup", "configure-model", "--file"])
                .arg(&tools.model)
                .arg("--json"),
        )?;
        ensure(code == Some(0), "setup configure-model did not succeed")?;
        Ok(host)
    }

    fn inputs(&self) -> PathBuf {
        self.base.join("inputs")
    }

    /// Copies a corpus file into the input root under `relative`.
    fn stage_input(&self, from: &Path, relative: &str) -> Result<(), StageStop> {
        let target = self.inputs().join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(from, target)?;
        Ok(())
    }

    /// Applies the isolated per-user environment and an empty `PATH`.
    fn isolate<'command>(&self, command: &'command mut Process) -> &'command mut Process {
        let user = self.base.join("user");
        command
            .env("LOCALAPPDATA", &user)
            .env("XDG_CONFIG_HOME", &user)
            .env("XDG_CACHE_HOME", &user)
            .env("HOME", &user)
            .env("PATH", "")
            .stdin(Stdio::null())
    }

    /// A bounded `vsift` invocation in the isolated environment.
    fn command(&self) -> Result<Command, StageStop> {
        let user = self.base.join("user");
        let mut command = published_binary::command()?;
        command
            .env("LOCALAPPDATA", &user)
            .env("XDG_CONFIG_HOME", &user)
            .env("XDG_CACHE_HOME", &user)
            .env("HOME", &user)
            .env("PATH", "")
            .timeout(CLI_DEADLINE);
        Ok(command)
    }

    /// Absolute paths of this host that must never reach the output (O-01).
    fn assert_no_paths(&self, output: &[u8]) -> Result<(), StageStop> {
        let text = String::from_utf8_lossy(output);
        let base = self.base.display().to_string();
        ensure(
            !text.contains(&base) && !text.contains(&base.replace('\\', "\\\\")),
            "an absolute path of the host reached the batch's output",
        )
    }
}

/// A worker workspace of a host, with its bundle root.
struct Workspace {
    root: PathBuf,
    bundles: PathBuf,
}

impl Workspace {
    fn create(
        host: &Host,
        name: &str,
        durability: &str,
        capacity: u16,
    ) -> Result<(Self, Value), StageStop> {
        let root = host.base.join(name);
        let bundles = host.base.join(format!("{name}-bundles"));
        let (code, value) = run_json(host.command()?.arg("--session-root").arg(&root).args([
            "session",
            "init-workspace",
            "--durability",
            durability,
            "--admission-slots",
            &capacity.to_string(),
            "--json",
        ]))?;
        if code != Some(0) {
            return Err(StageStop::Failed(bounded(&format!(
                "session init-workspace failed: {}",
                value["error"]["code"]
            ))));
        }
        conforms("workspace-data.schema.json", &value["data"])?;
        fs::create_dir_all(&bundles)?;
        Ok((Self { root, bundles }, value))
    }

    /// A command that must succeed in this workspace.
    fn ok_json(&self, host: &Host, arguments: &[&str]) -> Result<(Value, Duration), StageStop> {
        let started = Instant::now();
        let (code, value) = run_json(
            host.command()?
                .arg("--session-root")
                .arg(&self.root)
                .args(arguments)
                .arg("--json"),
        )?;
        ensure(
            code == Some(0),
            &format!(
                "{arguments:?} failed: {} {}",
                value["error"]["code"], value["error"]["remediation"][0]["summary"]
            ),
        )?;
        Ok((value, started.elapsed()))
    }

    /// `job batch` of `requests` with the host's roots, ready to spawn.
    fn batch(&self, host: &Host, requests: &Path, concurrency: u16) -> Result<Process, StageStop> {
        let mut command = published_binary::process()?;
        host.isolate(&mut command)
            .arg("--session-root")
            .arg(&self.root)
            .args(["job", "batch", "--requests"])
            .arg(requests)
            .arg("--input-root")
            .arg(host.inputs())
            .arg("--bundle-root")
            .arg(&self.bundles)
            .args([
                "--concurrency",
                &concurrency.to_string(),
                "--events",
                "jsonl",
            ]);
        Ok(command)
    }

    /// Runs a batch to its end.
    fn run_batch(
        &self,
        host: &Host,
        requests: &Path,
        concurrency: u16,
    ) -> Result<BatchRun, StageStop> {
        let started = Instant::now();
        let output = self
            .batch(host, requests, concurrency)?
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;
        ensure(
            output.stderr.is_empty(),
            "the batch wrote to standard error",
        )?;
        host.assert_no_paths(&output.stdout)?;
        Ok(BatchRun {
            code: output.status.code(),
            events: parse_events(&output.stdout)?,
            elapsed: started.elapsed(),
        })
    }

    /// Every chunk checkpoint of any job of any session of the workspace.
    fn checkpoints(&self) -> usize {
        let Ok(sessions) = fs::read_dir(self.root.join("sessions")) else {
            return 0;
        };
        let mut found = 0;
        for session in sessions.flatten() {
            let Ok(jobs) = fs::read_dir(session.path().join("jobs")) else {
                continue;
            };
            for job in jobs.flatten() {
                let Ok(chunks) = fs::read_dir(job.path().join("chunks")) else {
                    continue;
                };
                found += chunks
                    .flatten()
                    .filter(|chunk| {
                        chunk
                            .path()
                            .extension()
                            .is_some_and(|extension| extension == "json")
                    })
                    .count();
            }
        }
        found
    }
}

fn run_json(command: &mut Command) -> Result<(Option<i32>, Value), StageStop> {
    let output = command.output()?;
    ensure(
        output.stderr.is_empty(),
        "a JSON command wrote to standard error",
    )?;
    let value: Value = serde_json::from_slice(&output.stdout)?;
    conforms("operation-response.schema.json", &value)?;
    Ok((output.status.code(), value))
}

/// What one batch ended with.
struct BatchRun {
    code: Option<i32>,
    events: Vec<Value>,
    elapsed: Duration,
}

impl BatchRun {
    /// The terminal result (the `--json` response).
    fn terminal(&self) -> Result<&Value, StageStop> {
        self.events
            .last()
            .map(|event| &event["result"])
            .ok_or_else(|| failed("the batch wrote no event"))
    }

    /// Each line's `job-result`, by line.
    fn results(&self) -> Result<BTreeMap<u64, Value>, StageStop> {
        let mut results = BTreeMap::new();
        for event in &self.events {
            if event["event"] == "result" {
                results.insert(u64_of(&event["line"])?, event["result"].clone());
            }
        }
        Ok(results)
    }

    fn result(&self, line: u64) -> Result<Value, StageStop> {
        self.results()?
            .remove(&line)
            .ok_or_else(|| failed(&format!("line {line} has no result")))
    }

    fn lifecycle_kinds(&self) -> Vec<String> {
        self.events
            .iter()
            .filter(|event| event["event"] == "lifecycle")
            .filter_map(|event| event["kind"].as_str().map(str::to_owned))
            .collect()
    }

    /// The most requests admitted and not yet finished at any point.
    fn peak_in_flight(&self) -> usize {
        let mut in_flight = 0_usize;
        let mut peak = 0;
        for event in &self.events {
            match event["kind"].as_str() {
                Some("request_admitted") => in_flight += 1,
                Some("request_finished") if event["status"] != "rejected" => {
                    in_flight = in_flight.saturating_sub(1);
                }
                _ => {}
            }
            peak = peak.max(in_flight);
        }
        peak
    }
}

/// Every line of an event stream, each validated by its kind's schema and
/// bounded, the sequence contiguous and the terminal event last.
fn parse_events(stdout: &[u8]) -> Result<Vec<Value>, StageStop> {
    let raw: Vec<&[u8]> = stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    let mut events = Vec::new();
    for (index, line) in raw.iter().enumerate() {
        ensure(
            index + 1 == raw.len() || line.len() < MAX_EVENT_LINE_BYTES,
            "an event line is over 64 KiB",
        )?;
        let event: Value = serde_json::from_slice(line)?;
        let schema = match event["event"].as_str() {
            Some("lifecycle") => "lifecycle-event.schema.json",
            Some("progress") => "progress-event.schema.json",
            Some("result") => "result-event.schema.json",
            Some("terminal") => "terminal-event.schema.json",
            _ => return Err(failed("an event of an unexpected kind")),
        };
        conforms(schema, &event)?;
        if event["event"] == "result" {
            conforms("job-result.schema.json", &event["result"])?;
        }
        ensure(
            event["sequence"] == events.len(),
            "the event sequence is not contiguous",
        )?;
        events.push(event);
    }
    let terminal = events
        .last()
        .ok_or_else(|| failed("the batch wrote no event"))?;
    ensure(
        terminal["event"] == "terminal",
        "the stream does not end with the terminal event",
    )?;
    conforms("operation-response.schema.json", &terminal["result"])?;
    if !terminal["result"]["data"].is_null() {
        conforms("job-batch-data.schema.json", &terminal["result"]["data"])?;
    }
    Ok(events)
}

/// One job-request line: an ingest of `source` (with an optional sidecar
/// and offset) under `operation`, then `steps`.
fn request_line(
    operation: &str,
    durability: &str,
    source: &str,
    transcript: Option<(&str, i64)>,
    steps: &Value,
) -> String {
    let transcript = transcript.map_or(
        Value::Null,
        |(path, offset_us)| json!({"path": path, "offset_us": offset_us}),
    );
    json!({
        "schema_version": "1",
        "operation_id": operation,
        "durability": durability,
        "target": {"ingest": {"source": source, "transcript": transcript}},
        "steps": steps,
    })
    .to_string()
}

fn write_requests(path: &Path, lines: &[String]) -> Result<(), StageStop> {
    let mut text = lines.join("\n");
    text.push('\n');
    fs::write(path, text)?;
    Ok(())
}

/// The kind and status of every step of a result.
fn step_statuses(result: &Value) -> Result<Vec<(String, String)>, StageStop> {
    array_of(&result["steps"])?
        .iter()
        .map(|step| Ok((str_of(&step["kind"])?, str_of(&step["status"])?)))
        .collect()
}

/// The first step of `kind` in a result.
fn step<'result>(result: &'result Value, kind: &str) -> Result<&'result Value, StageStop> {
    array_of(&result["steps"])?
        .iter()
        .find(|step| step["kind"] == kind)
        .ok_or_else(|| failed(&format!("the result has no {kind} step")))
}

fn all_complete(result: &Value, kinds: &[&str]) -> Result<(), StageStop> {
    let statuses = step_statuses(result)?;
    let wanted: Vec<(String, String)> = kinds
        .iter()
        .map(|kind| ((*kind).to_owned(), "complete".to_owned()))
        .collect();
    ensure(
        result["status"] == "complete" && statuses == wanted,
        &format!("a request did not complete every step: {statuses:?}"),
    )
}

/// A frozen fixture entry of the corpus manifest.
fn manifest_entry(id: &str) -> Result<Value, StageStop> {
    let manifest: Value = serde_json::from_slice(&fs::read(corpus("manifest.json"))?)?;
    manifest["fixtures"]
        .as_array()
        .and_then(|fixtures| fixtures.iter().find(|entry| entry["id"] == id))
        .cloned()
        .ok_or_else(|| failed("a fixture is missing from the manifest"))
}

fn event_window(entry: &Value, event_id: &str) -> Result<(u64, u64), StageStop> {
    let event = entry["events"]
        .as_array()
        .and_then(|events| events.iter().find(|event| event["id"] == event_id))
        .ok_or_else(|| failed("an event is missing from the manifest"))?;
    ensure(event["critical"] == true, "the cited event is not critical")?;
    Ok((u64_of(&event["start_us"])?, u64_of(&event["end_us"])?))
}

/// The frozen truth of F03's speech variant.
struct SpeechTruth {
    duration: u64,
    speech: (u64, u64),
    term_words: (u64, u64),
    event: (u64, u64),
}

fn speech_truth() -> Result<SpeechTruth, StageStop> {
    let entry = manifest_entry(SPEECH_FIXTURE)?;
    let provenance: Value = serde_json::from_slice(&fs::read(
        corpus("generated").join("speech-provenance.json"),
    )?)?;
    let variant = provenance["assembly"]["variants"]
        .as_array()
        .and_then(|variants| {
            variants
                .iter()
                .find(|variant| variant["fixture"] == SPEECH_FIXTURE)
        })
        .ok_or_else(|| failed("F03 is missing from the speech provenance"))?;
    let word = variant["tts_word_timings"]
        .as_array()
        .and_then(|words| words.iter().find(|word| word["text"] == SPEECH_TERM))
        .ok_or_else(|| failed("the term has no word timing"))?;
    Ok(SpeechTruth {
        duration: u64_of(&entry["duration_us"])?,
        speech: (
            u64_of(&variant["speech_start_us"])?,
            u64_of(&variant["speech_end_us"])?,
        ),
        term_words: (u64_of(&word["start_us"])?, u64_of(&word["end_us"])?),
        event: event_window(&entry, SPEECH_EVENT)?,
    })
}

/// Every artifact of a retained bundle of one kind.
fn bundle_records(bundle: &Path, kind: &str) -> Result<Vec<Value>, StageStop> {
    let manifest: Value = serde_json::from_slice(&fs::read(bundle.join("bundle.json"))?)?;
    let mut records = Vec::new();
    for artifact in array_of(&manifest["artifacts"])? {
        if artifact["kind"] != kind {
            continue;
        }
        let name = str_of(&artifact["name"])?;
        ensure(
            name.starts_with("artifact-") && !name.contains(['/', '\\']),
            "a bundle artifact has an unexpected name",
        )?;
        records.push(serde_json::from_slice(&fs::read(bundle.join(name))?)?);
    }
    Ok(records)
}

/// The representative times of the newest visual-index record of a bundle.
fn bundle_candidates(bundle: &Path) -> Result<Vec<u64>, StageStop> {
    let mut records = bundle_records(bundle, "visual_index_record")?;
    for record in &records {
        conforms("bundle-visual-index-record.schema.json", record)?;
    }
    records.sort_by_key(|record| record["revision"].as_u64());
    let newest = records
        .last()
        .ok_or_else(|| failed("the bundle carries no visual-index record"))?;
    let mut times = Vec::new();
    for window in array_of(&newest["windows"])? {
        if let Some(candidates) = window["candidates"].as_array() {
            for candidate in candidates {
                times.push(u64_of(&candidate["representative_us"])?);
            }
        }
    }
    Ok(times)
}

/// Every transcript segment of a bundle as (start, end, text).
fn bundle_segments(bundle: &Path) -> Result<Vec<(u64, u64, String)>, StageStop> {
    let mut segments = Vec::new();
    for record in bundle_records(bundle, "transcript_record")? {
        conforms("bundle-transcript-record.schema.json", &record)?;
        for segment in array_of(&record["segments"])? {
            segments.push((
                u64_of(&segment["start_us"])?,
                u64_of(&segment["end_us"])?,
                str_of(&segment["text"])?,
            ));
        }
    }
    Ok(segments)
}

/// Validates the bundle a request's `retain` step wrote, through the binary
/// and the storage crate: it names the request's session and source, holds
/// the recorded artifact count, and its manifest's SHA-256 is the digest the
/// request recorded. Returns the bundle's directory.
fn validated_bundle(
    host: &Host,
    workspace: &Workspace,
    result: &Value,
) -> Result<PathBuf, StageStop> {
    let retain = step(result, "retain")?;
    let name = str_of(&retain["outputs"]["bundle_name"])?;
    let bundle = workspace.bundles.join(&name);
    let (validated, _) =
        workspace.ok_json(host, &["bundle", "validate", &bundle.to_string_lossy()])?;
    let status = FilesystemSessionStore::validate_bundle(&bundle)
        .map_err(|error| failed(&format!("the storage crate refused the bundle: {error}")))?;
    ensure(
        validated["data"]["session_id"] == result["session_id"]
            && validated["data"]["source_id"] == result["source_id"]
            && validated["data"]["artifact_count"] == retain["outputs"]["artifact_count"]
            && retain["outputs"]["bundle_sha256"] == status.manifest_sha256(),
        &format!("the bundle {name} is not the one its request recorded"),
    )?;
    Ok(bundle)
}

/// Stage 1: a mixed batch, then its outputs used and cited.
#[allow(
    clippy::too_many_lines,
    reason = "The batch, its outputs and their citations read best in one place"
)]
fn mechanical_stage(root: &OwnedRoot, tools: &Tools) -> StageResult {
    let host = Host::new(root.path("mechanical"), tools)?;
    host.stage_input(&corpus("generated/F03-speech.mp4"), "corpus/F03-speech.mp4")?;
    host.stage_input(&corpus("generated/F10.mp4"), "corpus/F10.mp4")?;
    host.stage_input(&corpus("transcripts/F10.srt"), "corpus/F10.srt")?;
    host.stage_input(&corpus("generated/F01.mp4"), "corpus/F01.mp4")?;
    let (workspace, _) = Workspace::create(&host, "workspace", "ephemeral", WIDE_CAPACITY)?;
    let requests = host.base.join("mechanical.jsonl");
    write_requests(
        &requests,
        &[
            request_line(
                "op_p11mechanicalf03speech00001",
                "ephemeral",
                "corpus/F03-speech.mp4",
                None,
                &json!([
                    {"retranscribe": {"range": null}},
                    {"candidates": {"range": null}},
                    {"retain": {"bundle_name": "f03", "include_source": false}},
                ]),
            ),
            request_line(
                "op_p11mechanicalf10import00002",
                "ephemeral",
                "corpus/F10.mp4",
                Some(("corpus/F10.srt", F10_OFFSET_US)),
                &json!([
                    {"candidates": {"range": null}},
                    {"retain": {"bundle_name": "f10", "include_source": false}},
                ]),
            ),
            request_line(
                "op_p11mechanicalf01closed00003",
                "ephemeral",
                "corpus/F01.mp4",
                None,
                &json!([
                    {"candidates": {"range": null}},
                    {"retain": {"bundle_name": "f01", "include_source": true}},
                    {"close": {}},
                ]),
            ),
            r#"{"schema_version":"1","operation_id":"op_p11mechanicalmalformed004","durability":"ephemeral"}"#
                .to_owned(),
            request_line(
                "op_p11mechanicaloutside000005",
                "ephemeral",
                "../F01.mp4",
                None,
                &json!([{"close": {}}]),
            ),
        ],
    )?;
    let run = workspace.run_batch(&host, &requests, 3)?;
    let terminal = run.terminal()?;
    let summary = &terminal["data"];
    ensure(
        run.code == Some(2)
            && terminal["error"]["code"] == "INVALID_ARGUMENT"
            && summary["counts"]["complete"] == 3
            && summary["counts"]["rejected"] == 2
            && summary["termination_reason"] == "end_of_input",
        &format!(
            "the mixed batch is not D5's usage failure with three complete lines: exit {:?}, {summary}",
            run.code
        ),
    )?;
    let started = &run.events[0];
    ensure(
        started["kind"] == "started"
            && started["readiness"]["publication"] == "process_crash_consistent"
            && started["readiness"]["concurrency"] == 3
            && started["readiness"]["admission_capacity"] == WIDE_CAPACITY,
        "the batch did not start with its readiness",
    )?;
    let rejections: BTreeMap<u64, Value> = array_of(&summary["items"])?
        .iter()
        .filter(|item| item["status"] == "rejected")
        .filter_map(|item| Some((item["line"].as_u64()?, item["rejection"].clone())))
        .collect();
    ensure(
        rejections.get(&4) == Some(&json!("malformed_request"))
            && rejections.get(&5) == Some(&json!("invalid_path")),
        &format!("the refused lines are not reported alone: {rejections:?}"),
    )?;
    let results = run.results()?;
    ensure(
        results.keys().copied().collect::<Vec<_>>() == [1, 2, 3],
        "the refused lines have results, or a request has none",
    )?;
    let f03 = run.result(1)?;
    let f10 = run.result(2)?;
    let f01 = run.result(3)?;
    all_complete(&f03, &["ingest", "retranscribe", "candidates", "retain"])?;
    all_complete(&f10, &["ingest", "candidates", "retain"])?;
    all_complete(&f01, &["ingest", "candidates", "retain", "close"])?;
    ensure(
        !step(&f10, "ingest")?["outputs"]["revision_id"].is_null(),
        "F10's ingest did not import its sidecar",
    )?;
    let recognition_job = str_of(&step(&f03, "retranscribe")?["job_id"])?;
    ensure(
        run.events.iter().any(|event| {
            event["event"] == "progress"
                && event["stage"] == "recognising_speech"
                && event["job_id"] == recognition_job.as_str()
        }),
        "the recognition reported no chunk progress",
    )?;

    // F03: search for the critical term, candidates near the hit, the frame
    // of the one inside the critical event.
    let truth = speech_truth()?;
    let f03_session = str_of(&f03["session_id"])?;
    let (search, _) =
        workspace.ok_json(&host, &["search", &f03_session, "--query", SPEECH_TERM])?;
    conforms("search-data.schema.json", &search["data"])?;
    let hit = array_of(&search["data"]["items"])?
        .first()
        .ok_or_else(|| failed("F03's critical term was not found"))?
        .clone();
    let (segment_start, segment_end) = (u64_of(&hit["start_us"])?, u64_of(&hit["end_us"])?);
    let tolerance = ASR_SPAN_TOLERANCE_US;
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
    let window = (
        segment_start.saturating_sub(LEAD_LAG_US),
        (segment_start + LEAD_LAG_US).min(truth.duration),
    );
    let (page, _) = workspace.ok_json(
        &host,
        &[
            "candidates",
            &f03_session,
            "--from",
            &window.0.to_string(),
            "--to",
            &window.1.to_string(),
            "--limit",
            "100",
        ],
    )?;
    conforms("candidates-data.schema.json", &page["data"])?;
    let candidate = array_of(&page["data"]["items"])?
        .iter()
        .find(|item| {
            item["representative_us"]
                .as_u64()
                .is_some_and(|time| (truth.event.0..truth.event.1).contains(&time))
        })
        .ok_or_else(|| failed("no candidate near the hit lies inside the critical event"))?
        .clone();
    let candidate_id = str_of(&candidate["candidate_id"])?;
    let representative = u64_of(&candidate["representative_us"])?;
    let (frame, _) = workspace.ok_json(
        &host,
        &["frame", "get", &f03_session, "--candidate", &candidate_id],
    )?;
    conforms("frame-data.schema.json", &frame["data"])?;
    ensure(
        frame["data"]["selections"][0]["delta_us"] == 0
            && u64_of(&frame["data"]["selections"][0]["actual_us"])? == representative,
        "the candidate's frame is not its own",
    )?;
    let f03_bundle = validated_bundle(&host, &workspace, &f03)?;
    let cited = bundle_segments(&f03_bundle)?
        .iter()
        .any(|(start, end, _)| *start == segment_start && *end == segment_end);
    ensure(cited, "F03's bundle does not carry the cited segment")?;
    ensure(
        bundle_candidates(&f03_bundle)?.contains(&representative),
        "F03's bundle does not carry the cited candidate",
    )?;

    // F10: the imported dialog id on its frozen truth window.
    let f10_truth = event_window(&manifest_entry("F10")?, "F10-E01")?;
    let f10_session = str_of(&f10["session_id"])?;
    let (dialog, _) = workspace.ok_json(&host, &["search", &f10_session, "--query", "R-17"])?;
    conforms("search-data.schema.json", &dialog["data"])?;
    let dialog_items = array_of(&dialog["data"]["items"])?;
    ensure(
        dialog_items.len() == 1
            && dialog_items[0]["start_us"].as_u64() == Some(f10_truth.0)
            && dialog_items[0]["end_us"].as_u64() == Some(f10_truth.1),
        &format!("F10's dialog id is not cited on its truth window {f10_truth:?}"),
    )?;
    let f10_bundle = validated_bundle(&host, &workspace, &f10)?;
    ensure(
        bundle_segments(&f10_bundle)?
            .iter()
            .any(|(start, end, text)| (*start, *end) == f10_truth && text.contains("R-17")),
        "F10's bundle does not carry the imported segment",
    )?;

    // F01: a closed request's bundle; its candidates lie in its one event.
    let f01_event = event_window(&manifest_entry("F01")?, "F01-E01")?;
    let f01_bundle = validated_bundle(&host, &workspace, &f01)?;
    let f01_candidates = bundle_candidates(&f01_bundle)?;
    ensure(
        !f01_candidates.is_empty()
            && f01_candidates
                .iter()
                .all(|time| (f01_event.0..f01_event.1).contains(time)),
        "F01's candidates do not lie in its frozen event",
    )?;

    Ok(json!({
        "exit": run.code,
        "batch_ms": run.elapsed.as_millis(),
        "counts": summary["counts"],
        "rejections": {"4": "malformed_request", "5": "invalid_path"},
        "f03": {
            "session_steps_ms": array_of(&f03["steps"])?.iter().map(|step| step["elapsed_ms"].clone()).collect::<Vec<_>>(),
            "segment_us": [segment_start, segment_end],
            "speech_span_us": [truth.speech.0, truth.speech.1],
            "candidate_us": representative,
            "event_us": [truth.event.0, truth.event.1],
        },
        "f10": {"segment_us": [f10_truth.0, f10_truth.1]},
        "f01": {"candidates": f01_candidates.len(), "event_us": [f01_event.0, f01_event.1]},
        "bundles_validated": 3,
    }))
}

/// One sample of the batch's provider processes.
#[derive(Clone, Copy, Default)]
struct ProviderSample {
    ffmpeg: u16,
    ffprobe: u16,
    whisper: u16,
}

impl ProviderSample {
    fn of(names: &[String]) -> Self {
        let mut sample = Self::default();
        for name in names {
            let name = name.to_ascii_lowercase();
            if name.starts_with("ffmpeg") {
                sample.ffmpeg += 1;
            } else if name.starts_with("ffprobe") {
                sample.ffprobe += 1;
            } else if name.starts_with("whisper") {
                sample.whisper += 1;
            }
        }
        sample
    }

    /// The admission weight the sample holds without a recognition: a
    /// visual window's `FFmpeg` 2, a probe 1 (ADR 0021 section 5a).
    const fn visual_weight(self) -> u16 {
        self.ffmpeg * 2 + self.ffprobe
    }
}

/// Samples the provider processes `parent` runs until stopped.
struct ProviderSampler {
    stop: Arc<AtomicBool>,
    #[cfg(windows)]
    lister: Child,
    thread: thread::JoinHandle<Vec<ProviderSample>>,
}

impl ProviderSampler {
    /// On Windows one `PowerShell` process lists the children every 50 ms
    /// (starting one per sample would take most of a second each).
    #[cfg(windows)]
    fn start(parent: u32) -> Result<Self, StageStop> {
        let script = format!(
            "while ($true) {{ $names = @(Get-CimInstance Win32_Process -Filter 'ParentProcessId={parent}' | ForEach-Object {{ $_.Name }}); [Console]::Out.WriteLine('sample:' + ($names -join ',')); [Console]::Out.Flush(); Start-Sleep -Milliseconds 50 }}"
        );
        let mut lister = Process::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = lister
            .stdout
            .take()
            .ok_or_else(|| failed("the sampler has no stdout"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn(move || {
            BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
                .filter_map(|line| {
                    line.strip_prefix("sample:").map(|names| {
                        let names: Vec<String> = names
                            .split(',')
                            .filter(|name| !name.is_empty())
                            .map(str::to_owned)
                            .collect();
                        ProviderSample::of(&names)
                    })
                })
                .collect()
        });
        Ok(Self {
            stop,
            lister,
            thread,
        })
    }

    /// Elsewhere a thread runs `ps` every 50 ms.
    #[cfg(not(windows))]
    #[allow(
        clippy::unnecessary_wraps,
        reason = "The same signature as the Windows sampler, which can fail to start"
    )]
    fn start(parent: u32) -> Result<Self, StageStop> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            let mut samples = Vec::new();
            while !stopping.load(Ordering::Relaxed) {
                if let Ok(output) = Process::new("ps")
                    .args(["-A", "-o", "ppid=,comm="])
                    .stdin(Stdio::null())
                    .output()
                {
                    let names: Vec<String> = String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .filter_map(|line| {
                            let mut fields = line.split_whitespace();
                            let ppid: u32 = fields.next()?.parse().ok()?;
                            let command = fields.next()?;
                            (ppid == parent).then(|| {
                                Path::new(command)
                                    .file_name()
                                    .map_or_else(String::new, |name| {
                                        name.to_string_lossy().into_owned()
                                    })
                            })
                        })
                        .collect();
                    samples.push(ProviderSample::of(&names));
                }
                thread::sleep(Duration::from_millis(50));
            }
            samples
        });
        Ok(Self { stop, thread })
    }

    fn finish(self) -> Result<Vec<ProviderSample>, StageStop> {
        self.stop.store(true, Ordering::Relaxed);
        #[cfg(windows)]
        {
            let mut lister = self.lister;
            let _ = lister.kill();
            let _ = lister.wait();
        }
        self.thread
            .join()
            .map_err(|_| failed("the provider sampler stopped abnormally"))
    }
}

/// Reads a child's stdout lines on a thread.
fn line_reader(child: &mut Child) -> Result<mpsc::Receiver<String>, StageStop> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| failed("the batch has no stdout"))?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                return;
            }
        }
    });
    Ok(receiver)
}

/// Every line a finished child wrote.
fn drain_lines(lines: &mpsc::Receiver<String>, seen: &mut Vec<String>) {
    while let Ok(line) = lines.recv_timeout(Duration::from_millis(500)) {
        seen.push(line);
    }
}

/// Stage 2: the admission ladder.
#[allow(
    clippy::too_many_lines,
    reason = "Each rung's run, sampling and checks read best in one place"
)]
fn ladder_stage(root: &OwnedRoot, tools: &Tools) -> StageResult {
    let host = Host::new(root.path("ladder"), tools)?;
    let clip = host.inputs().join("ladder/F02-180s.mp4");
    fs::create_dir_all(host.inputs().join("ladder"))?;
    tools.loop_clip("F02.mp4", VISUAL_LOOPS, &clip)?;
    let mut rungs = Vec::new();
    let mut reference: Option<Vec<u64>> = None;
    for concurrency in [1_u16, 2, 4] {
        let (workspace, _) = Workspace::create(
            &host,
            &format!("ladder-{concurrency}"),
            "ephemeral",
            LADDER_CAPACITY,
        )?;
        let requests = host.base.join(format!("ladder-{concurrency}.jsonl"));
        let lines: Vec<String> = (1..=4)
            .map(|index| {
                request_line(
                    &format!("op_p11ladder{concurrency}request{index:016}"),
                    "ephemeral",
                    "ladder/F02-180s.mp4",
                    None,
                    &json!([{"candidates": {"range": null}}]),
                )
            })
            .collect();
        write_requests(&requests, &lines)?;
        let started = Instant::now();
        let mut child = workspace
            .batch(&host, &requests, concurrency)?
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let lines_read = line_reader(&mut child)?;
        let watcher = ProviderSampler::start(child.id())?;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() > CLI_DEADLINE {
                let _ = child.kill();
                let _ = watcher.finish();
                return Err(failed("a ladder batch outlived its deadline"));
            }
            thread::sleep(Duration::from_millis(50));
        };
        let elapsed = started.elapsed();
        let observed = watcher.finish()?;
        let mut seen = Vec::new();
        drain_lines(&lines_read, &mut seen);
        let stdout = seen.join("\n").into_bytes();
        host.assert_no_paths(&stdout)?;
        let run = BatchRun {
            code: status.code(),
            events: parse_events(&stdout)?,
            elapsed,
        };
        ensure(
            run.code == Some(0) && run.terminal()?["data"]["counts"]["complete"] == 4,
            &format!("a ladder batch at concurrency {concurrency} did not complete"),
        )?;
        let peak_in_flight = run.peak_in_flight();
        ensure(
            peak_in_flight == usize::from(concurrency),
            &format!(
                "at concurrency {concurrency} the stream had {peak_in_flight} requests in flight"
            ),
        )?;
        let peak_weight = observed
            .iter()
            .map(|sample| sample.visual_weight())
            .max()
            .unwrap_or_default();
        let peak_windows = observed
            .iter()
            .map(|sample| sample.ffmpeg)
            .max()
            .unwrap_or_default();
        ensure(
            !observed.is_empty() && observed.iter().all(|sample| sample.whisper == 0),
            "the ladder's provider sampler saw nothing, or a recognizer",
        )?;
        ensure(
            peak_weight <= LADDER_CAPACITY,
            &format!(
                "at concurrency {concurrency} the providers held weight {peak_weight}, above the capacity {LADDER_CAPACITY}"
            ),
        )?;
        let mut counts = Vec::new();
        let mut admission_wait_ms = 0;
        for result in run.results()?.values() {
            all_complete(result, &["ingest", "candidates"])?;
            counts.push(u64_of(
                &step(result, "candidates")?["outputs"]["candidate_count"],
            )?);
            for step in array_of(&result["steps"])? {
                admission_wait_ms += u64_of(&step["admission_wait_ms"])?;
            }
        }
        match &reference {
            Some(expected) => ensure(
                expected == &counts,
                "a ladder rung found other candidates than the first",
            )?,
            None => reference = Some(counts.clone()),
        }
        rungs.push(json!({
            "concurrency": concurrency,
            "elapsed_ms": elapsed.as_millis(),
            "peak_in_flight": peak_in_flight,
            "samples": observed.len(),
            "peak_sampled_weight": peak_weight,
            "peak_sampled_windows": peak_windows,
            "admission_wait_ms": admission_wait_ms,
            "candidate_counts": counts,
        }));
    }
    Ok(json!({
        "clip": format!("F02.mp4 looped {VISUAL_LOOPS} more times (-c copy), 180 s"),
        "capacity": LADDER_CAPACITY,
        "weights": "visual window 2, probe 1",
        "rungs": rungs,
    }))
}

/// The processes `parent` started that are still running.
fn children_of(parent: u32) -> Result<BTreeSet<u32>, StageStop> {
    #[cfg(windows)]
    let output = Process::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "Get-CimInstance Win32_Process -Filter 'ParentProcessId={parent}' | Where-Object {{ $_.Name -ne 'conhost.exe' }} | ForEach-Object {{ $_.ProcessId }}"
            ),
        ])
        .stdin(Stdio::null())
        .output()?;
    #[cfg(not(windows))]
    let output = Process::new("pgrep")
        .args(["-P", &parent.to_string()])
        .stdin(Stdio::null())
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter_map(|pid| pid.parse().ok())
        .collect())
}

/// Whether process `pid` still runs.
fn alive(pid: u32) -> Result<bool, StageStop> {
    #[cfg(windows)]
    let output = Process::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ 'alive' }}"),
        ])
        .stdin(Stdio::null())
        .output()?;
    #[cfg(not(windows))]
    let output = Process::new("kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    #[cfg(windows)]
    return Ok(String::from_utf8_lossy(&output.stdout).contains("alive"));
    #[cfg(not(windows))]
    return Ok(output.status.success());
}

/// Sends the platform's shutdown signal to `child`: `SIGTERM` on Unix, a
/// console Ctrl-Break through the helper on Windows (Ctrl-Break, because a
/// process that inherited "ignore Ctrl-C" is never told about a Ctrl-C).
fn shut_down(child: &Child) -> Result<(), StageStop> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let status = Process::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(repository().join("tools/send-console-ctrl.ps1"))
            .args(["-ProcessId", &child.id().to_string(), "-Event", "CtrlBreak"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        ensure(status.code() == Some(0), "the console helper failed")
    }
    #[cfg(not(windows))]
    {
        let status = Process::new("kill")
            .args(["-s", "TERM", &child.id().to_string()])
            .status()?;
        ensure(status.success(), "kill -s TERM failed")
    }
}

/// What a line of a batch left behind that its control must match: the
/// request and step statuses, the counts it reported, and the evidence of
/// its bundle.
fn evidence_of(workspace: &Workspace, host: &Host, result: &Value) -> Result<Value, StageStop> {
    let bundle = validated_bundle(host, workspace, result)?;
    let candidates = if step(result, "candidates").is_ok() {
        Some(bundle_candidates(&bundle)?)
    } else {
        None
    };
    let mut counts = Vec::new();
    for step in array_of(&result["steps"])? {
        counts.push(json!({
            "kind": step["kind"],
            "status": step["status"],
            "candidate_count": step["outputs"]["candidate_count"],
            "artifact_count": step["outputs"]["artifact_count"],
        }));
    }
    Ok(json!({
        "status": result["status"],
        "steps": counts,
        "segments": bundle_segments(&bundle)?,
        "candidates": candidates,
    }))
}

/// Stage 3: shutdown mid-batch, then `job resume` and redelivery against a
/// control.
#[allow(
    clippy::too_many_lines,
    reason = "The control, the stop, the two resumptions and the comparison read best in one place"
)]
fn shutdown_stage(root: &OwnedRoot, tools: &Tools) -> StageResult {
    let host = Host::new(root.path("shutdown"), tools)?;
    fs::create_dir_all(host.inputs().join("long"))?;
    tools.loop_clip(
        "F03-speech.mp4",
        SPEECH_LOOPS,
        &host.inputs().join("long/F03-speech-81s.mp4"),
    )?;
    tools.loop_clip(
        "F02.mp4",
        VISUAL_LOOPS,
        &host.inputs().join("long/F02-180s.mp4"),
    )?;
    host.stage_input(&corpus("generated/F10.mp4"), "long/F10.mp4")?;
    host.stage_input(&corpus("transcripts/F10.srt"), "long/F10.srt")?;
    let requests = host.base.join("shutdown.jsonl");
    write_requests(
        &requests,
        &[
            request_line(
                "op_p11shutdownrecognition00001",
                "ephemeral",
                "long/F03-speech-81s.mp4",
                None,
                &json!([
                    {"retranscribe": {"range": null}},
                    {"retain": {"bundle_name": "speech", "include_source": false}},
                    {"close": {}},
                ]),
            ),
            request_line(
                "op_p11shutdownvisual000000002",
                "ephemeral",
                "long/F02-180s.mp4",
                None,
                &json!([
                    {"candidates": {"range": null}},
                    {"retain": {"bundle_name": "visual", "include_source": false}},
                    {"close": {}},
                ]),
            ),
            request_line(
                "op_p11shutdownimport000000003",
                "ephemeral",
                "long/F10.mp4",
                Some(("long/F10.srt", F10_OFFSET_US)),
                &json!([
                    {"candidates": {"range": null}},
                    {"retain": {"bundle_name": "import", "include_source": false}},
                    {"close": {}},
                ]),
            ),
            request_line(
                "op_p11shutdownrecognition00004",
                "ephemeral",
                "long/F03-speech-81s.mp4",
                None,
                &json!([
                    {"retranscribe": {"range": null}},
                    {"retain": {"bundle_name": "speech-again", "include_source": false}},
                    {"close": {}},
                ]),
            ),
        ],
    )?;

    // The control: the same file, uninterrupted, in its own workspace.
    let (control_workspace, _) = Workspace::create(&host, "control", "ephemeral", WIDE_CAPACITY)?;
    let control = control_workspace.run_batch(&host, &requests, 2)?;
    ensure(
        control.code == Some(0) && control.terminal()?["data"]["counts"]["complete"] == 4,
        "the control batch did not complete",
    )?;
    let mut control_evidence = BTreeMap::new();
    for (line, result) in control.results()? {
        control_evidence.insert(line, evidence_of(&control_workspace, &host, &result)?);
    }

    // The interrupted batch, stopped once the first recognition has a
    // checkpoint and the second (line 4) has been admitted.
    let (workspace, _) = Workspace::create(&host, "interrupted", "ephemeral", WIDE_CAPACITY)?;
    let mut command = workspace.batch(&host, &requests, 2)?;
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn()?;
    let lines = line_reader(&mut child)?;
    let started = Instant::now();
    let mut seen = Vec::new();
    loop {
        while let Ok(line) = lines.try_recv() {
            seen.push(line);
        }
        let second_admitted = seen
            .iter()
            .any(|line| line.contains(r#""kind":"request_admitted","line":4,"#));
        if second_admitted && workspace.checkpoints() > 0 {
            break;
        }
        if child.try_wait()?.is_some() {
            return Err(failed(
                "the batch ended before its first checkpoint and its fourth line",
            ));
        }
        if started.elapsed() > CLI_DEADLINE {
            let _ = child.kill();
            return Err(failed("no chunk checkpoint appeared"));
        }
        thread::sleep(Duration::from_millis(20));
    }
    let mut providers = children_of(child.id())?;
    let signalled_after = started.elapsed();
    let sent = Instant::now();
    shut_down(&child)?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if sent.elapsed() > SHUTDOWN_BUDGET {
            let _ = child.kill();
            return Err(failed("the stopped batch outlived its shutdown budget"));
        }
        thread::sleep(Duration::from_millis(10));
    };
    let stopped_after = sent.elapsed();
    providers.extend(children_of(child.id())?);
    let deadline = Instant::now() + DESCENDANT_BUDGET;
    loop {
        let mut running = Vec::new();
        for provider in &providers {
            if alive(*provider)? {
                running.push(*provider);
            }
        }
        if running.is_empty() {
            break;
        }
        ensure(
            Instant::now() < deadline,
            &format!("providers still running after the batch ended: {running:?}"),
        )?;
        thread::sleep(Duration::from_millis(200));
    }
    drain_lines(&lines, &mut seen);
    let stdout = seen.join("\n").into_bytes();
    host.assert_no_paths(&stdout)?;
    let stopped = BatchRun {
        code: status.code(),
        events: parse_events(&stdout)?,
        elapsed: started.elapsed(),
    };
    let terminal = stopped.terminal()?;
    let kinds = stopped.lifecycle_kinds();
    ensure(
        stopped.code == Some(6)
            && terminal["error"]["code"] == "CANCELLED"
            && terminal["data"]["termination_reason"] == "shutdown"
            && kinds.contains(&"draining".to_owned())
            && kinds.last().map(String::as_str) == Some("stopped"),
        &format!(
            "the stopped batch is not a shutdown: exit {:?}, {kinds:?}, {}",
            stopped.code, terminal["data"]
        ),
    )?;
    let interrupted = stopped.result(1)?;
    let recognition = step(&interrupted, "retranscribe")?;
    ensure(
        interrupted["status"] == "cancelled" && recognition["status"] == "cancelled",
        &format!("the recognition's request was not cancelled: {interrupted}"),
    )?;
    let job = str_of(&recognition["job_id"])?;
    let second = stopped.result(4)?;
    ensure(
        second["status"] == "cancelled",
        &format!("the second recognition's request was not cancelled: {second}"),
    )?;
    let stopped_statuses: Vec<Value> = array_of(&terminal["data"]["items"])?
        .iter()
        .map(|item| json!([item["line"], item["status"]]))
        .collect();

    // Resume the first recognition by its job id, then deliver the file
    // again: the first request replays that job's commit, the second
    // continues from its first unfinished step.
    let (status_before, _) = workspace.ok_json(&host, &["job", "status", &job])?;
    conforms("job-data.schema.json", &status_before["data"])?;
    ensure(
        status_before["data"]["state"] == "interrupted"
            && u64_of(&status_before["data"]["progress"]["chunks_checkpointed"])? >= 1,
        &format!("the stopped recognition is not interrupted with a checkpoint: {status_before}"),
    )?;
    let (resumed, resume_time) = workspace.ok_json(&host, &["job", "resume", &job])?;
    conforms("job-resume-data.schema.json", &resumed["data"])?;
    ensure(
        resumed["data"]["job"]["state"] == "succeeded",
        "job resume did not finish the recognition",
    )?;
    let redelivered = workspace.run_batch(&host, &requests, 2)?;
    ensure(
        redelivered.code == Some(0) && redelivered.terminal()?["data"]["counts"]["complete"] == 4,
        &format!(
            "the redelivered batch did not complete: exit {:?}",
            redelivered.code
        ),
    )?;
    let continued = redelivered.result(1)?;
    ensure(
        u64_of(&continued["attempt"])? >= 2
            && step(&continued, "retranscribe")?["job_id"] == job.as_str(),
        "the redelivery did not continue the stopped request with its job",
    )?;
    let second_continued = redelivered.result(4)?;
    ensure(
        u64_of(&second_continued["attempt"])? >= 2,
        "the redelivery did not continue the second stopped request",
    )?;
    for (line, result) in redelivered.results()? {
        let evidence = evidence_of(&workspace, &host, &result)?;
        ensure(
            control_evidence.get(&line) == Some(&evidence),
            &format!("line {line} after the shutdown differs from the control"),
        )?;
    }

    // A third delivery replays every line unchanged.
    let replayed = workspace.run_batch(&host, &requests, 2)?;
    ensure(replayed.code == Some(0), "the third delivery failed")?;
    let first = redelivered.results()?;
    for (line, mut result) in replayed.results()? {
        ensure(
            result["replayed"] == true,
            &format!("line {line} was not replayed"),
        )?;
        result["replayed"] = json!(false);
        let mut expected = first
            .get(&line)
            .cloned()
            .ok_or_else(|| failed("a replayed line was not delivered before"))?;
        expected["replayed"] = json!(false);
        ensure(
            result == expected,
            &format!("line {line}'s replay differs from its recorded result"),
        )?;
    }

    Ok(json!({
        "interruption": if cfg!(windows) { "console Ctrl-Break (helper)" } else { "SIGTERM" },
        "control_ms": control.elapsed.as_millis(),
        "signalled_after_ms": signalled_after.as_millis(),
        "stopped_after_ms": stopped_after.as_millis(),
        "providers_seen": providers.len(),
        "stopped_items": stopped_statuses,
        "not_started_from_line": terminal["data"]["not_started_from_line"],
        "job": job,
        "job_resume_ms": resume_time.as_millis(),
        "redelivery_ms": redelivered.elapsed.as_millis(),
        "redelivery_attempt_line_1": continued["attempt"],
        "redelivery_line_4": {
            "attempt": second_continued["attempt"],
            "steps_at_stop": step_statuses(&second)?,
            "chunks_reused": step(&second_continued, "retranscribe")?["outputs"]["chunks_reused"],
        },
        "replay_ms": replayed.elapsed.as_millis(),
        "compared_with_control": ["request and step statuses", "candidate and artifact counts", "bundle transcript segments", "bundle visual-index candidates"],
    }))
}

/// Stage 4: a durable workspace, only on the qualified profile.
fn durable_stage(root: &OwnedRoot, tools: &Tools) -> StageResult {
    let host = Host::new(root.path("durable"), tools)?;
    host.stage_input(&corpus("generated/F10.mp4"), "durable/F10.mp4")?;
    host.stage_input(&corpus("transcripts/F10.srt"), "durable/F10.srt")?;
    let workspace_root = host.base.join("workspace");
    let (code, created) = run_json(
        host.command()?
            .arg("--session-root")
            .arg(&workspace_root)
            .args([
                "session",
                "init-workspace",
                "--durability",
                "durable",
                "--admission-slots",
                "4",
                "--json",
            ]),
    )?;
    if code != Some(0) {
        ensure(
            created["error"]["code"] == "MISSING_CAPABILITY" && !workspace_root.exists(),
            &format!(
                "a refused durable workspace is not MISSING_CAPABILITY with nothing created: {}",
                created["error"]["code"]
            ),
        )?;
        return Err(StageStop::Blocked(format!(
            "durable workspaces are qualified only on Ubuntu 24.04 with the root on local ext4; this host ({}) refused one with MISSING_CAPABILITY and created nothing, as required",
            env::consts::OS
        )));
    }
    ensure(
        env::consts::OS == "linux" && created["data"]["publication"] == "os_crash_durable",
        "a durable workspace was created off the qualified profile",
    )?;
    let workspace = Workspace {
        root: workspace_root,
        bundles: host.base.join("workspace-bundles"),
    };
    fs::create_dir_all(&workspace.bundles)?;
    let requests = host.base.join("durable.jsonl");
    write_requests(
        &requests,
        &[request_line(
            "op_p11durablerequest000000001",
            "durable",
            "durable/F10.mp4",
            Some(("durable/F10.srt", F10_OFFSET_US)),
            &json!([
                {"retain": {"bundle_name": "durable", "include_source": false}},
                {"close": {}},
            ]),
        )],
    )?;
    let run = workspace.run_batch(&host, &requests, 1)?;
    ensure(
        run.code == Some(0) && run.events[0]["readiness"]["publication"] == "os_crash_durable",
        "the durable batch did not complete with durable publication",
    )?;
    let result = run.result(1)?;
    all_complete(&result, &["ingest", "retain", "close"])?;
    ensure(
        result["publication"] == "os_crash_durable"
            && result["controls"]["free_space_reserve"] == "enforced",
        "the durable request did not publish durably with the free-space reserve",
    )?;
    validated_bundle(&host, &workspace, &result)?;
    let again = workspace.run_batch(&host, &requests, 1)?;
    ensure(
        again.code == Some(0) && again.result(1)?["replayed"] == true,
        "the durable request was not replayed",
    )?;
    Ok(json!({
        "publication": "os_crash_durable",
        "batch_ms": run.elapsed.as_millis(),
        "replayed": true,
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

/// The stages every host must pass, and the one required only on the
/// qualified durable profile.
type StageFn = fn(&OwnedRoot, &Tools) -> StageResult;
const REQUIRED_STAGES: [(&str, StageFn); 3] = [
    ("p11_batch_mechanical", mechanical_stage),
    ("p11_admission_ladder", ladder_stage),
    ("p11_shutdown_and_redelivery", shutdown_stage),
];
const DURABLE_STAGE: &str = "p11_durable_workspace";

#[test]
#[ignore = "opt-in P11 worker checkpoint; needs ffmpeg/ffprobe on PATH, VSIFT_TEST_WHISPER_CLI and VSIFT_TEST_WHISPER_MODEL; reports to .vsift/e2e-runs"]
fn single_host_worker_run() -> TestResult {
    let started = Instant::now();
    let repository = repository().canonicalize()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_dir = repository
        .join(".vsift/e2e-runs")
        .join(format!("p11-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&run_dir)?;
    let root = OwnedRoot::new()?;
    let tools = Tools::discover();
    let mut stages = Vec::new();
    let run = |name: &str, body: StageFn| {
        let clock = Instant::now();
        let result = match tools.as_ref() {
            Some(tools) => body(&root, tools),
            None => Err(StageStop::Blocked(MISSING_TOOLS.to_owned())),
        };
        stage(name, clock, result)
    };
    for (name, body) in REQUIRED_STAGES {
        stages.push(run(name, body));
    }
    let durable = run(DURABLE_STAGE, durable_stage);
    // Off the qualified profile the durable stage is blocked by the platform,
    // not by a missing tool, and is not required there.
    let durable_required = durable["status"] != "blocked" || tools.is_none();
    stages.push(durable);

    let required: Vec<&Value> = stages
        .iter()
        .filter(|entry| entry["name"] != DURABLE_STAGE || durable_required)
        .collect();
    let status_of = |wanted: &str| required.iter().any(|entry| entry["status"] == wanted);
    let overall = if status_of("failed") {
        "failed"
    } else if status_of("blocked") {
        "blocked"
    } else {
        "passed"
    };
    let future_stages: Vec<_> = [
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
        "checkpoint": "P11 single-host worker run",
        "fixture_manifest": {
            "schema_version": manifest["schema_version"],
            "corpus_id": manifest["corpus_id"],
        },
        "fixtures": "F01.mp4, F03-speech.mp4, F10.mp4 with F10.srt at +500 ms; F03-speech looped to 81 s and F02 looped to 180 s at run time without re-encoding; the F01, F03 and F10 manifest entries and F03's speech provenance",
        "os": env::consts::OS,
        "architecture": env::consts::ARCH,
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "resource_profile": "ephemeral worker workspaces of 16 admission units (4 for the ladder); each CLI call killed after 600 s; empty PATH; a stopped batch must end within 15 s and leave no provider running 10 s later",
        "vsift_version": env!("CARGO_PKG_VERSION"),
        "binary_under_test": published_binary::report()?,
        "authorization": "opt-in cargo test invocation; setup configure writes only to isolated temporary per-user bases; no install, download or network access",
        "prior_checkpoints": ["P09: p09_evidence_e2e", "P10: p10_recovery_e2e"],
        "stages": stages,
        "platform_blocked": if durable_required { json!([]) } else { json!([DURABLE_STAGE]) },
        "coverage_gaps": [
            "The provider sampler sees processes, not admission reservations; a sample is a lower bound of the weight held",
            "On Windows the shutdown is a console Ctrl-Break: a process that inherited 'ignore Ctrl-C' never sees a Ctrl-C",
            "Strict Linux isolation is not exercised here; SEC-T01 is met for P11 by non-adversarial evidence, its adversarial evidence deferred as technical debt (L-068)",
            "No OS or storage crash: durable publication is the P10 crash campaign's, rerun with worker requests in P11 PR 3"
        ],
        "future_stages": future_stages,
        "overall": overall,
        "complete_journey": "not_implemented",
        "elapsed_ms": started.elapsed().as_millis(),
    });
    let report_path = run_dir.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("P11 worker checkpoint report: {}", report_path.display());
    println!("p11_worker: {overall}");
    if overall == "passed" {
        Ok(())
    } else {
        Err(format!(
            "P11 worker checkpoint {overall}; see {}",
            report_path.display()
        )
        .into())
    }
}
