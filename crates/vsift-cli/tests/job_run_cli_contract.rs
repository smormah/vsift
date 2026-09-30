//! Public CLI contract of `job run` (P11 PR 3, ADR 0021): one versioned
//! request in a worker workspace, through the binary.
//!
//! Every default test runs without `FFmpeg`: the requests ingest a
//! placeholder source (a plain ingest runs no provider), retain and close
//! it. They check the result and event schemas, replay, conflict, busy and
//! refusals with their exit codes, O-01 (sentinel values in input paths, a
//! sidecar, the environment and proxy credentials never reach stdout,
//! stderr or events), and O-04 for a single request: a shutdown signal
//! stops the request (at once, or after its drain time), the stream says
//! so, the process exits 6, and the next delivery continues it.
//!
//! On Unix the shutdown is `SIGTERM` from `kill` and runs in CI. On Windows
//! a console Ctrl-Break is sent through the opt-in helper
//! `tools/send-console-ctrl.ps1`, as in `interrupt_cli_contract`:
//!
//! `cargo test -p vsift-cli --locked --test job_run_cli_contract -- --ignored`
//!
//! The opt-in `provider_output_never_reaches_output` also needs `FFmpeg`
//! and `FFprobe` on `PATH`: a candidates step over a source that is not a
//! video makes the provider fail, and its output must not reach the caller.
//! The opt-in `every_step_runs_with_real_tools` runs every step kind over
//! the F01 speech fixture; it needs `FFmpeg` and `FFprobe` on `PATH`,
//! `VSIFT_TEST_WHISPER_CLI` and `VSIFT_TEST_WHISPER_MODEL`.

use std::{
    env,
    error::Error,
    fs,
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_contract::{REQUEST_FILE_REMEDIATION, REQUEST_STOPPED_REMEDIATION};
use vsift_domain::OperationId;
use vsift_infrastructure::{FilesystemSessionStore, WorkerRequestClaim};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-job-run-cli-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SOURCE: &[u8] = b"\0\0\0\x18ftypisomjob-run-source";
const OPERATION: &str = "op_5b1e0c7a9d2f4e6b8a3c1d0e9f7a6b5c";
/// Directory names and texts that must never reach the caller (O-01).
const PATH_SENTINEL: &str = "sentinelpathcomponent7f3a";
const SIDECAR_SENTINEL: &str = "SENTINEL-SIDECAR-TEXT-41c9";
const ENVIRONMENT_SENTINEL: &str = "SENTINEL-ENV-TOKEN-9c1d";
const PROXY_SENTINEL: &str = "SENTINEL-PROXY-PASSWORD-5e2b";
/// How long a stopped request may take to end.
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(15);

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

/// A temporary layout: workspace, input root, bundle root, request files.
struct Layout(PathBuf);

impl Layout {
    fn new() -> Built<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        let layout = Self(path);
        fs::create_dir_all(layout.inputs().join(PATH_SENTINEL))?;
        fs::create_dir(layout.bundles())?;
        fs::write(layout.inputs().join(PATH_SENTINEL).join("talk.mp4"), SOURCE)?;
        Ok(layout)
    }

    fn path(&self, child: &str) -> PathBuf {
        self.0.join(child)
    }

    fn workspace(&self) -> PathBuf {
        self.path("workspace")
    }

    fn inputs(&self) -> PathBuf {
        self.path("inputs")
    }

    fn bundles(&self) -> PathBuf {
        self.path("bundles")
    }

    /// `vsift` with an isolated per-user base and the sentinel environment.
    fn command(&self) -> Command {
        let base = self.path("user");
        let mut command = Command::new(assert_cmd::cargo::cargo_bin("vsift"));
        command
            .env("LOCALAPPDATA", &base)
            .env("XDG_CONFIG_HOME", &base)
            .env("XDG_CACHE_HOME", &base)
            .env("HOME", &base)
            .env("VSIFT_TEST_SECRET_TOKEN", ENVIRONMENT_SENTINEL)
            .env(
                "HTTP_PROXY",
                format!("http://worker:{PROXY_SENTINEL}@127.0.0.1:9"),
            )
            .env(
                "HTTPS_PROXY",
                format!("http://worker:{PROXY_SENTINEL}@127.0.0.1:9"),
            )
            .stdin(Stdio::null());
        command
    }

    fn init(&self, capacity: u16) -> TestResult {
        let output = self
            .command()
            .arg("--session-root")
            .arg(self.workspace())
            .args([
                "session",
                "init-workspace",
                "--durability",
                "ephemeral",
                "--admission-slots",
                &capacity.to_string(),
                "--json",
            ])
            .output()?;
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        Ok(())
    }

    fn request_file(&self, name: &str, text: &str) -> Built<PathBuf> {
        let path = self.path(name);
        fs::write(&path, text)?;
        Ok(path)
    }

    /// `job run` of `request` with the layout's roots and `extra` flags.
    fn job_run(&self, request: &Path, extra: &[&str]) -> Command {
        let mut command = self.command();
        command
            .arg("--session-root")
            .arg(self.workspace())
            .args(["job", "run", "--request"])
            .arg(request)
            .arg("--input-root")
            .arg(self.inputs())
            .arg("--bundle-root")
            .arg(self.bundles())
            .args(extra);
        command
    }

    /// Asserts that no sentinel and no absolute path of the layout is in
    /// `output`'s streams (O-01).
    fn assert_nothing_sensitive(&self, output: &Output) {
        for stream in [&output.stdout, &output.stderr] {
            let text = String::from_utf8_lossy(stream);
            for sentinel in [
                PATH_SENTINEL,
                SIDECAR_SENTINEL,
                ENVIRONMENT_SENTINEL,
                PROXY_SENTINEL,
            ] {
                assert!(!text.contains(sentinel), "{sentinel} leaked: {text}");
            }
            for path in [self.inputs(), self.workspace(), self.bundles()] {
                let path = path.display().to_string();
                assert!(!text.contains(&path), "{path} leaked: {text}");
                assert!(
                    !text.contains(&path.replace('\\', "\\\\")),
                    "{path} leaked: {text}"
                );
            }
        }
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// An ingest of the sentinel directory's source, then `steps`.
fn request(operation: &str, steps: &str) -> String {
    format!(
        r#"{{"schema_version":"1","operation_id":"{operation}","durability":"ephemeral","target":{{"ingest":{{"source":"{PATH_SENTINEL}/talk.mp4","transcript":null}}}},"steps":[{steps}]}}"#
    )
}

const RETAIN_AND_CLOSE: &str =
    r#"{"retain":{"bundle_name":"talk-review","include_source":false}},{"close":{}}"#;

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
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
            schema_root().join(name),
        )?)?)
    }
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    let schema: Value =
        serde_json::from_str(&fs::read_to_string(schema_root().join(schema_path))?)?;
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

/// The one schema-valid `--json` result, with its job result validated.
fn json_result(output: &Output) -> Built<Value> {
    let value: Value = serde_json::from_slice(&output.stdout)?;
    validate("operation-response.schema.json", &value)?;
    if !value["data"].is_null() {
        validate("job-result.schema.json", &value["data"])?;
    }
    Ok(value)
}

/// Every line of an event stream, each validated by its kind's schema, the
/// sequence contiguous and the terminal event last.
fn events(stdout: &[u8]) -> Built<Vec<Value>> {
    let mut lines = Vec::new();
    for line in stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: Value = serde_json::from_slice(line)?;
        let schema = match value["event"].as_str() {
            Some("lifecycle") => "lifecycle-event.schema.json",
            Some("progress") => "progress-event.schema.json",
            Some("result") => "result-event.schema.json",
            Some("terminal") => "terminal-event.schema.json",
            other => return Err(format!("unexpected event {other:?}").into()),
        };
        validate(schema, &value)?;
        if value["event"] == "result" {
            validate("job-result.schema.json", &value["result"])?;
        }
        assert_eq!(value["sequence"], lines.len(), "{value}");
        lines.push(value);
    }
    let terminal = lines.last().ok_or("empty stream")?;
    assert_eq!(terminal["event"], "terminal");
    validate("operation-response.schema.json", &terminal["result"])?;
    Ok(lines)
}

/// The lifecycle kinds of a stream, in order.
fn lifecycle_kinds(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|event| event["event"] == "lifecycle")
        .filter_map(|event| event["kind"].as_str().map(str::to_owned))
        .collect()
}

/// A request runs once and every redelivery returns its recorded result;
/// nothing names a path.
#[test]
fn a_request_runs_and_replays() -> TestResult {
    let layout = Layout::new()?;
    layout.init(2)?;
    let file = layout.request_file("request.json", &request(OPERATION, RETAIN_AND_CLOSE))?;

    let output = layout.job_run(&file, &["--json"]).output()?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    layout.assert_nothing_sensitive(&output);
    let first = json_result(&output)?;
    assert_eq!(first["command"], "job.run");
    assert_eq!(first["status"], "complete");
    assert_eq!(first["operation_id"], OPERATION);
    assert_eq!(first["lifecycle"]["mode"], "durable_worker");
    assert_eq!(first["data"]["replayed"], false);
    assert!(
        layout
            .bundles()
            .join("talk-review")
            .join("bundle.json")
            .exists()
    );

    let output = layout.job_run(&file, &["--json"]).output()?;
    assert_eq!(output.status.code(), Some(0));
    let replayed = json_result(&output)?;
    let mut expected = first.clone();
    expected["data"]["replayed"] = Value::Bool(true);
    assert_eq!(replayed, expected);
    Ok(())
}

/// Asserts SEC-T02's terminal rules on one human stream: no control
/// character but line breaks, no raw hidden character, no terminal link.
fn assert_terminal_safe(stream: &[u8]) -> Built<String> {
    let text = String::from_utf8(stream.to_vec())?;
    for character in text.chars() {
        assert!(
            character == '\n' || !character.is_control(),
            "control U+{:04X} in {text:?}",
            u32::from(character)
        );
        assert!(
            !vsift_contract::is_hidden_character(character),
            "raw hidden U+{:04X} in {text:?}",
            u32::from(character)
        );
    }
    assert!(!text.contains("\u{1b}]8;"));
    Ok(text)
}

/// Human mode (P13 PR 2b): the job result is readable text on stdout with
/// nothing on stderr, the replay says so, and neither names a path; a
/// refused request with hostile text in its paths writes only its fixed
/// error, on stderr, echoing nothing (O-01, SEC-T02).
#[test]
fn human_output_is_readable_and_echoes_nothing() -> TestResult {
    let layout = Layout::new()?;
    layout.init(2)?;
    let file = layout.request_file("request.json", &request(OPERATION, RETAIN_AND_CLOSE))?;

    let output = layout.job_run(&file, &[]).output()?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    layout.assert_nothing_sensitive(&output);
    assert!(output.stderr.is_empty(), "{output:?}");
    let text = assert_terminal_safe(&output.stdout)?;
    assert!(text.starts_with("Worker request: complete\n"), "{text}");
    for step in [
        "0. ingest: complete",
        "1. retain: complete",
        "2. close: complete",
    ] {
        assert!(text.contains(step), "{step}: {text}");
    }
    assert!(
        text.contains(&format!("\nOperation: {OPERATION}\n")),
        "{text}"
    );
    assert!(!text.contains("\"command\""), "{text}");

    let output = layout.job_run(&file, &[]).output()?;
    assert_eq!(output.status.code(), Some(0));
    let replayed = assert_terminal_safe(&output.stdout)?;
    assert!(
        replayed.starts_with("Worker request: complete (replayed"),
        "{replayed}"
    );

    let hostile = layout.request_file(
        "hostile.json",
        &format!(
            r#"{{"schema_version":"1","operation_id":"{OPERATION}","durability":"ephemeral","target":{{"ingest":{{"source":"{PATH_SENTINEL}\u202e\u200b\u001b]8;;https://example.invalid\u0007x/talk.mp4","transcript":null}}}},"steps":[]}}"#
        ),
    )?;
    let output = layout.job_run(&hostile, &[]).output()?;
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    layout.assert_nothing_sensitive(&output);
    assert!(output.stdout.is_empty(), "{output:?}");
    let error = assert_terminal_safe(&output.stderr)?;
    assert!(
        error.starts_with("Error: ") && error.contains("(INVALID_ARGUMENT)"),
        "{error}"
    );
    assert!(!error.contains("example.invalid"), "{error}");
    Ok(())
}

/// The event stream of a fresh run and of a replay follow the contract:
/// started with readiness, admitted, progress of the request's steps, the
/// result, finished, stopped at the end of input, then the terminal event
/// with the `--json` response.
#[test]
fn the_event_stream_follows_the_contract() -> TestResult {
    let layout = Layout::new()?;
    layout.init(2)?;
    let file = layout.request_file("request.json", &request(OPERATION, RETAIN_AND_CLOSE))?;
    for replay in [false, true] {
        let output = layout.job_run(&file, &["--events", "jsonl"]).output()?;
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        layout.assert_nothing_sensitive(&output);
        let events = events(&output.stdout)?;
        assert_eq!(
            lifecycle_kinds(&events),
            ["started", "request_admitted", "request_finished", "stopped"]
        );
        let started = &events[0];
        assert_eq!(
            started["readiness"]["publication"],
            "process_crash_consistent"
        );
        assert_eq!(started["readiness"]["admission_capacity"], 2);
        assert_eq!(started["readiness"]["concurrency"], 1);
        let result = events
            .iter()
            .find(|event| event["event"] == "result")
            .ok_or("no result event")?;
        assert_eq!(result["result"]["replayed"], replay);
        assert_eq!(result["line"], Value::Null);
        let finished = events
            .iter()
            .find(|event| event["kind"] == "request_finished")
            .ok_or("no request_finished")?;
        assert_eq!(finished["status"], "complete");
        assert_eq!(finished["request_operation_id"], OPERATION);
        let stopped = &events[events.len() - 2];
        assert_eq!(stopped["reason"], "end_of_input");
        let terminal = &events[events.len() - 1];
        assert_eq!(terminal["result"]["data"], result["result"]);
        if !replay {
            let progress: Vec<&Value> = events
                .iter()
                .filter(|event| event["event"] == "progress")
                .collect();
            assert!(!progress.is_empty());
            for event in progress {
                assert_eq!(event["stage"], "running_request");
                assert_eq!(event["unit"], "steps");
                assert_eq!(event["total"], 3);
                assert_eq!(event["request_operation_id"], OPERATION);
            }
        }
    }
    Ok(())
}

/// Refusals through the binary: an unreadable or malformed request file,
/// the same id with another request, and a request another process holds.
#[test]
fn refusals_have_their_codes_and_exit_statuses() -> TestResult {
    let layout = Layout::new()?;
    layout.init(2)?;

    let output = layout
        .job_run(&layout.path("missing.json"), &["--json"])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let value = json_result(&output)?;
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(
        value["error"]["remediation"][0]["summary"],
        REQUEST_FILE_REMEDIATION
    );
    layout.assert_nothing_sensitive(&output);

    let malformed = layout.request_file(
        "malformed.json",
        &format!(r#"{{"schema_version":"1","operation_id":"{OPERATION}","{PATH_SENTINEL}":1}}"#),
    )?;
    let output = layout.job_run(&malformed, &["--json"]).output()?;
    assert_eq!(output.status.code(), Some(2));
    let value = json_result(&output)?;
    assert_eq!(value["data"], Value::Null);
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    layout.assert_nothing_sensitive(&output);

    let newer = layout.request_file("newer.json", r#"{"schema_version":"2"}"#)?;
    let output = layout.job_run(&newer, &["--json"]).output()?;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json_result(&output)?["error"]["code"], "UNSUPPORTED_SCHEMA");

    let file = layout.request_file("request.json", &request(OPERATION, RETAIN_AND_CLOSE))?;
    let other = layout.request_file(
        "other.json",
        &request(
            OPERATION,
            r#"{"retain":{"bundle_name":"other","include_source":false}}"#,
        ),
    )?;
    let held = FilesystemSessionStore::open_existing(layout.workspace())?;
    let owner = match held.claim_worker_request(&OperationId::parse(OPERATION)?)? {
        WorkerRequestClaim::Owned(owner) => owner,
        WorkerRequestClaim::HeldElsewhere(_) => return Err("not claimed".into()),
    };
    let output = layout.job_run(&file, &["--json"]).output()?;
    assert_eq!(output.status.code(), Some(4), "{output:?}");
    let value = json_result(&output)?;
    assert_eq!(value["status"], "failed");
    assert_eq!(value["error"]["code"], "BUSY");
    assert_eq!(value["error"]["retry_after_ms"], 2_000);
    assert_eq!(value["data"]["failure"]["code"], "BUSY");
    drop(owner);

    assert_eq!(
        layout.job_run(&file, &["--json"]).output()?.status.code(),
        Some(0)
    );
    let output = layout.job_run(&other, &["--json"]).output()?;
    assert_eq!(output.status.code(), Some(2));
    let value = json_result(&output)?;
    assert_eq!(value["error"]["code"], "IDEMPOTENCY_CONFLICT");
    assert_eq!(value["data"]["failure"]["code"], "IDEMPOTENCY_CONFLICT");
    assert!(!layout.bundles().join("other").exists());
    Ok(())
}

/// O-01: a sidecar whose text holds a sentinel, rejected, reports fixed
/// prose; the failure names the session but no path and no text.
#[test]
fn a_rejected_sidecar_never_reaches_output() -> TestResult {
    let layout = Layout::new()?;
    layout.init(2)?;
    fs::write(
        layout.inputs().join(PATH_SENTINEL).join("talk.srt"),
        format!("1\n00:00:01,000 --> {SIDECAR_SENTINEL}\n{SIDECAR_SENTINEL}\n"),
    )?;
    let file = layout.request_file(
        "request.json",
        &format!(
            r#"{{"schema_version":"1","operation_id":"{OPERATION}","durability":"ephemeral","target":{{"ingest":{{"source":"{PATH_SENTINEL}/talk.mp4","transcript":{{"path":"{PATH_SENTINEL}/talk.srt","offset_us":0}}}}}},"steps":[]}}"#
        ),
    )?;
    for mode in [["--json", ""], ["--events", "jsonl"]] {
        let arguments: Vec<&str> = mode
            .iter()
            .copied()
            .filter(|argument| !argument.is_empty())
            .collect();
        let output = layout.job_run(&file, &arguments).output()?;
        assert_ne!(output.status.code(), Some(0), "{output:?}");
        layout.assert_nothing_sensitive(&output);
    }
    Ok(())
}

/// O-01 with a provider: a candidates step over a source that is not a
/// video makes `FFprobe` fail; its output never reaches the caller.
#[test]
#[ignore = "opt-in: needs FFmpeg and FFprobe on PATH"]
fn provider_output_never_reaches_output() -> TestResult {
    let layout = Layout::new()?;
    layout.init(2)?;
    let file = layout.request_file(
        "request.json",
        &request(OPERATION, r#"{"candidates":{"range":null}}"#),
    )?;
    for mode in [["--json", ""], ["--events", "jsonl"]] {
        let arguments: Vec<&str> = mode
            .iter()
            .copied()
            .filter(|argument| !argument.is_empty())
            .collect();
        let output = layout.job_run(&file, &arguments).output()?;
        assert_ne!(output.status.code(), Some(0), "{output:?}");
        layout.assert_nothing_sensitive(&output);
    }
    Ok(())
}

/// Reads a child's stdout lines on a thread, so a test can wait for one.
fn line_reader(child: &mut Child) -> Built<mpsc::Receiver<String>> {
    let stdout = child.stdout.take().ok_or("no stdout")?;
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

/// Waits for a line holding `needle`, returning every line read so far.
fn wait_for_line(lines: &mpsc::Receiver<String>, needle: &str) -> Built<Vec<String>> {
    let started = Instant::now();
    let mut seen = Vec::new();
    while started.elapsed() < Duration::from_secs(60) {
        if let Ok(line) = lines.recv_timeout(Duration::from_millis(100)) {
            let found = line.contains(needle);
            seen.push(line);
            if found {
                return Ok(seen);
            }
        }
    }
    Err(format!("never saw {needle}: {seen:?}").into())
}

/// Waits for the stopped child within the budget; returns its exit code
/// and the rest of its stream.
fn finish(
    mut child: Child,
    lines: &mpsc::Receiver<String>,
    mut seen: Vec<String>,
) -> Built<(Option<i32>, Vec<String>)> {
    let sent = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if sent.elapsed() > SHUTDOWN_BUDGET {
            let _ = child.kill();
            return Err("the stopped request outlived its shutdown budget".into());
        }
        thread::sleep(Duration::from_millis(10));
    };
    while let Ok(line) = lines.recv_timeout(Duration::from_millis(500)) {
        seen.push(line);
    }
    Ok((status.code(), seen))
}

/// A signal sender: `kill -s TERM` on Unix, the console helper on Windows.
type Signal = fn(&Child) -> TestResult;

/// O-04 for one request: the test holds the workspace's only admission
/// unit, so the request waits for it; a shutdown signal then stops it.
/// Without a drain time the waiting step is cancelled at once; with one it
/// may finish (the test frees the unit), and no later step starts. Either
/// way the stream says `draining` and `stopped: shutdown`, the result is
/// `cancelled`, the process exits 6, and the next delivery continues the
/// request to its end in the same session.
fn shutdown_stops_a_request(signal: Signal, windows_console: bool) -> TestResult {
    for drain in [false, true] {
        let layout = Layout::new()?;
        layout.init(1)?;
        let file = layout.request_file("request.json", &request(OPERATION, r#"{"close":{}}"#))?;
        let store = FilesystemSessionStore::open_existing(layout.workspace())?;
        let permit = store.try_admit(1)?;
        let mut extra = vec!["--events", "jsonl", "--admission-wait-ms", "60000"];
        if drain {
            extra.extend(["--drain-timeout-ms", "20000"]);
        }
        let mut command = layout.job_run(&file, &extra);
        command.stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        if windows_console {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        #[cfg(not(windows))]
        let _ = windows_console;
        let mut child = command.spawn()?;
        let lines = line_reader(&mut child)?;
        let mut seen = wait_for_line(&lines, "\"admission_waiting\"")?;
        signal(&child)?;
        if drain {
            seen.extend(wait_for_line(&lines, "\"draining\"")?);
            drop(permit);
        } else {
            drop(permit);
        }
        let (code, seen) = finish(child, &lines, seen)?;
        let stdout: Vec<u8> = seen.join("\n").into_bytes();
        let events = events(&stdout)?;
        assert_eq!(code, Some(6), "{seen:?}");
        let kinds = lifecycle_kinds(&events);
        assert!(kinds.contains(&"draining".to_owned()), "{kinds:?}");
        assert_eq!(kinds.last().map(String::as_str), Some("stopped"));
        let stopped = events
            .iter()
            .rfind(|event| event["kind"] == "stopped")
            .ok_or("no stopped")?;
        assert_eq!(stopped["reason"], "shutdown");
        let terminal = events.last().ok_or("no terminal")?;
        assert_eq!(terminal["result"]["status"], "cancelled");
        assert_eq!(terminal["result"]["error"]["code"], "CANCELLED");
        assert_eq!(
            terminal["result"]["error"]["remediation"][0]["summary"],
            REQUEST_STOPPED_REMEDIATION
        );
        let steps = &terminal["result"]["data"]["steps"];
        if drain {
            // The waiting ingest finished during the drain; close never
            // started.
            assert_eq!(steps[0]["status"], "complete", "{steps}");
            assert_eq!(steps[1]["status"], "cancelled", "{steps}");
        } else {
            assert_eq!(steps[0]["status"], "cancelled", "{steps}");
        }

        let output = layout.job_run(&file, &["--json"]).output()?;
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let value = json_result(&output)?;
        assert_eq!(value["data"]["attempt"], 2);
        if drain {
            assert_eq!(
                value["data"]["session_id"],
                terminal["result"]["data"]["session_id"]
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
fn send_sigterm(child: &Child) -> TestResult {
    let killed = Command::new("kill")
        .args(["-s", "TERM", &child.id().to_string()])
        .status()?;
    if !killed.success() {
        return Err("kill failed".into());
    }
    Ok(())
}

/// O-04 on Unix: `SIGTERM`.
#[cfg(unix)]
#[test]
fn sigterm_stops_a_request_resumably() -> TestResult {
    shutdown_stops_a_request(send_sigterm, false)
}

#[cfg(windows)]
fn send_ctrl_break(child: &Child) -> TestResult {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let helper =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/send-console-ctrl.ps1");
    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(helper)
        .args(["-ProcessId", &child.id().to_string(), "-Event", "CtrlBreak"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.code() != Some(0) {
        return Err(format!("the console helper failed: {status}").into());
    }
    Ok(())
}

/// O-04 on Windows: a console Ctrl-Break, through the opt-in helper.
#[cfg(windows)]
#[test]
#[ignore = "opt-in: sends console control events through tools/send-console-ctrl.ps1"]
fn ctrl_break_stops_a_request_resumably() -> TestResult {
    shutdown_stops_a_request(send_ctrl_break, true)
}

/// Opt-in real-tool run of every step kind: ingest, a whisper.cpp
/// retranscription (its P10 job under the derived operation id), visual
/// candidates until nothing is unanalysed, a retained bundle and a close,
/// then a replay. The recognition's chunk progress names its job.
#[test]
#[ignore = "opt-in: needs FFmpeg and FFprobe on PATH, VSIFT_TEST_WHISPER_CLI and VSIFT_TEST_WHISPER_MODEL"]
fn every_step_runs_with_real_tools() -> TestResult {
    let whisper =
        PathBuf::from(env::var_os("VSIFT_TEST_WHISPER_CLI").ok_or("VSIFT_TEST_WHISPER_CLI")?);
    let model =
        PathBuf::from(env::var_os("VSIFT_TEST_WHISPER_MODEL").ok_or("VSIFT_TEST_WHISPER_MODEL")?);
    let layout = Layout::new()?;
    layout.init(8)?;
    let configured = layout
        .command()
        .args(["setup", "configure", "whisper", "--executable"])
        .arg(&whisper)
        .arg("--json")
        .output()?;
    assert_eq!(configured.status.code(), Some(0), "{configured:?}");
    let configured = layout
        .command()
        .args(["setup", "configure-model", "--file"])
        .arg(&model)
        .arg("--json")
        .output()?;
    assert_eq!(configured.status.code(), Some(0), "{configured:?}");
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/corpus/generated/F01-speech.mp4"),
        layout.inputs().join(PATH_SENTINEL).join("talk.mp4"),
    )?;
    let file = layout.request_file(
        "request.json",
        &request(
            OPERATION,
            r#"{"retranscribe":{"range":null}},{"candidates":{"range":null}},{"retain":{"bundle_name":"f01-review","include_source":false}},{"close":{}}"#,
        ),
    )?;
    let output = layout.job_run(&file, &["--events", "jsonl"]).output()?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    layout.assert_nothing_sensitive(&output);
    let events = events(&output.stdout)?;
    let terminal = events.last().ok_or("no terminal")?;
    let data = &terminal["result"]["data"];
    assert_eq!(data["status"], "complete", "{data}");
    assert_eq!(
        statuses_of(data),
        ["ingest", "retranscribe", "candidates", "retain", "close"]
    );
    let job = data["steps"][1]["job_id"].as_str().ok_or("no job")?;
    assert!(events.iter().any(|event| event["event"] == "progress"
        && event["stage"] == "recognising_speech"
        && event["job_id"] == job));
    assert!(data["steps"][2]["outputs"]["visual_index_id"].is_string());

    // The recognition is a P10 job the job commands know.
    let status = layout
        .command()
        .arg("--session-root")
        .arg(layout.workspace())
        .args(["job", "status", job, "--json"])
        .output()?;
    assert_eq!(status.status.code(), Some(0), "{status:?}");
    let status: Value = serde_json::from_slice(&status.stdout)?;
    assert_eq!(status["data"]["state"], "succeeded");

    let output = layout.job_run(&file, &["--json"]).output()?;
    assert_eq!(output.status.code(), Some(0));
    let replayed = json_result(&output)?;
    assert_eq!(replayed["data"]["replayed"], true);
    assert_eq!(replayed["data"]["steps"], data["steps"]);
    Ok(())
}

fn statuses_of(data: &Value) -> Vec<String> {
    data["steps"]
        .as_array()
        .map(|steps| {
            steps
                .iter()
                .filter(|step| step["status"] == "complete" || step["status"] == "partial")
                .filter_map(|step| step["kind"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}
