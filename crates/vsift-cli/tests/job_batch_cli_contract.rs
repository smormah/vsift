//! Public CLI contract of `job batch` (P11 PR 4, ADR 0021 section 5): a
//! finite JSON Lines file of worker requests in a worker workspace, through
//! the binary.
//!
//! Every default test runs without `FFmpeg`: the requests ingest a
//! placeholder source (a plain ingest runs no provider), retain and close
//! it. They check the event and summary schemas and order, the D5 exit rule,
//! the limits (1,000 lines, concurrency, an unreadable file), O-01
//! (sentinel paths, environment and proxy credentials never reach the
//! output), O-02 (a property test over random batches: every event line is
//! bounded and schema-valid, the event count is bounded by the lines),
//! O-03 (a supervisor tells ready, busy, unhealthy and missing capability
//! apart from typed events and codes), X-08 through the binary (a paused
//! stdout reader stops the batch from starting requests, and it finishes
//! once read again) and O-04 for a batch (a shutdown signal stops the
//! reading and the running request, the process exits 6 in bounded time,
//! and the redelivered file completes).
//!
//! On Unix the shutdown is `SIGTERM` from `kill` and runs in CI. On Windows
//! a console Ctrl-Break is sent through the opt-in helper
//! `tools/send-console-ctrl.ps1`:
//!
//! `cargo test -p vsift-cli --locked --test job_batch_cli_contract -- --ignored`

use std::{
    env,
    error::Error,
    fs,
    io::{self, BufRead, BufReader, Read},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use jsonschema::{Retrieve, Uri};
use proptest::prelude::*;
use serde_json::Value;
use vsift_contract::{
    BATCH_CONCURRENCY_REMEDIATION, BATCH_FILE_REMEDIATION, BATCH_LINE_LIMIT_REMEDIATION,
    BATCH_LINES_FAILED_REMEDIATION, BATCH_STOPPED_REMEDIATION,
};
use vsift_infrastructure::FilesystemSessionStore;

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-job-batch-cli-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SOURCE: &[u8] = b"\0\0\0\x18ftypisomjob-batch-source";
/// Directory names and texts that must never reach the caller (O-01).
const PATH_SENTINEL: &str = "sentinelbatchcomponent4d2e";
const ENVIRONMENT_SENTINEL: &str = "SENTINEL-ENV-TOKEN-7a3f";
const PROXY_SENTINEL: &str = "SENTINEL-PROXY-PASSWORD-2c9b";
/// How long a stopped batch may take to end (O-04).
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(15);
/// Longest event line but the terminal one.
const MAX_EVENT_LINE_BYTES: usize = 65_536;

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

    fn requests(&self) -> PathBuf {
        self.path("requests.jsonl")
    }

    /// `vsift` with an isolated per-user base, no `PATH` and the sentinel
    /// environment.
    fn command(&self) -> Command {
        let base = self.path("user");
        let mut command = Command::new(assert_cmd::cargo::cargo_bin("vsift"));
        command
            .env("LOCALAPPDATA", &base)
            .env("XDG_CONFIG_HOME", &base)
            .env("XDG_CACHE_HOME", &base)
            .env("HOME", &base)
            .env("PATH", "")
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

    fn write_batch(&self, lines: &[String]) -> TestResult {
        let mut text = lines.join("\n");
        text.push('\n');
        fs::write(self.requests(), text)?;
        Ok(())
    }

    /// `job batch` of the layout's file with its roots and `extra` flags.
    fn job_batch(&self, extra: &[&str]) -> Command {
        let mut command = self.command();
        command
            .arg("--session-root")
            .arg(self.workspace())
            .args(["job", "batch", "--requests"])
            .arg(self.requests())
            .arg("--input-root")
            .arg(self.inputs())
            .arg("--bundle-root")
            .arg(self.bundles())
            .args(extra);
        command
    }

    /// Asserts that no sentinel and no absolute path of the layout is in
    /// `output`'s streams (O-01).
    fn assert_nothing_sensitive(&self, stdout: &[u8], stderr: &[u8]) {
        for stream in [stdout, stderr] {
            let text = String::from_utf8_lossy(stream);
            for sentinel in [PATH_SENTINEL, ENVIRONMENT_SENTINEL, PROXY_SENTINEL] {
                assert!(!text.contains(sentinel), "{sentinel} leaked: {text}");
            }
            for path in [
                self.inputs(),
                self.workspace(),
                self.bundles(),
                self.0.clone(),
            ] {
                let path = path.display().to_string();
                assert!(!text.contains(&path), "{path} leaked: {text}");
                assert!(
                    !text.contains(&path.replace('\\', "\\\\")),
                    "{path} leaked: {text}"
                );
            }
        }
    }

    /// Request records the workspace holds: one per request that started.
    fn recorded_requests(&self) -> Built<usize> {
        let root = self.workspace().join("worker-requests");
        if !root.exists() {
            return Ok(0);
        }
        let mut records = 0;
        for bucket in fs::read_dir(root)? {
            for entry in fs::read_dir(bucket?.path())? {
                let name = entry?.file_name();
                if name.to_string_lossy().ends_with(".json") {
                    records += 1;
                }
            }
        }
        Ok(records)
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

/// The `index`-th operation id of a test.
fn operation(index: usize) -> String {
    format!("op_cli{index:026}")
}

/// An ingest of the sentinel directory's source under `operation(index)`,
/// then `steps`.
fn request(index: usize, steps: &str) -> String {
    format!(
        r#"{{"schema_version":"1","operation_id":"{}","durability":"ephemeral","target":{{"ingest":{{"source":"{PATH_SENTINEL}/talk.mp4","transcript":null}}}},"steps":[{steps}]}}"#,
        operation(index)
    )
}

fn close_only(index: usize) -> String {
    request(index, r#"{"close":{}}"#)
}

fn retain_and_close(index: usize) -> String {
    request(
        index,
        &format!(
            r#"{{"retain":{{"bundle_name":"bundle-{index}","include_source":false}}}},{{"close":{{}}}}"#
        ),
    )
}

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

/// The one schema-valid `--json` result, with its batch summary validated.
fn json_result(stdout: &[u8]) -> Built<Value> {
    let value: Value = serde_json::from_slice(stdout)?;
    validate("operation-response.schema.json", &value)?;
    if !value["data"].is_null() {
        validate("job-batch-data.schema.json", &value["data"])?;
    }
    Ok(value)
}

/// Every line of an event stream, each validated by its kind's schema and
/// bounded, the sequence contiguous and the terminal event last.
fn parse_events(stdout: &[u8]) -> Built<Vec<Value>> {
    let mut lines = Vec::new();
    let raw: Vec<&[u8]> = stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    for (index, line) in raw.iter().enumerate() {
        if index + 1 < raw.len() {
            assert!(
                line.len() < MAX_EVENT_LINE_BYTES,
                "an event line is too long"
            );
        }
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
    if !terminal["result"]["data"].is_null() {
        validate("job-batch-data.schema.json", &terminal["result"]["data"])?;
    }
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

/// The position of the first event matching `predicate`.
fn position(events: &[Value], predicate: impl Fn(&Value) -> bool) -> Built<usize> {
    events
        .iter()
        .position(predicate)
        .ok_or_else(|| "no such event".into())
}

/// The item of `line` in a summary.
fn item(summary: &Value, line: u64) -> Built<Value> {
    summary["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["line"] == line))
        .cloned()
        .ok_or_else(|| format!("no item for line {line}: {summary}").into())
}

/// A mixed batch through the binary: every event is schema-valid and in
/// the contract's order (started first; per request admitted, its progress,
/// its result, then finished; stopped; the terminal event carrying the
/// `--json` response), refused lines have a `request_finished` and no
/// result, the exit is D5's (a refused line: 2) and nothing sensitive
/// reaches the output. `--json` prints the same summary alone.
#[test]
fn a_mixed_batch_streams_the_contract() -> TestResult {
    let layout = Layout::new()?;
    layout.init(4)?;
    layout.write_batch(&[
        retain_and_close(1),
        String::new(),
        format!(r#"{{"schema_version":"1","{PATH_SENTINEL}":true}}"#),
        close_only(1),
        close_only(5),
    ])?;
    let output = layout
        .job_batch(&["--concurrency", "2", "--events", "jsonl"])
        .output()?;
    layout.assert_nothing_sensitive(&output.stdout, &output.stderr);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let events = parse_events(&output.stdout)?;
    let kinds = lifecycle_kinds(&events);
    assert_eq!(kinds.first().map(String::as_str), Some("started"));
    assert_eq!(kinds.last().map(String::as_str), Some("stopped"));
    assert_eq!(events[0]["readiness"]["concurrency"], 2);
    assert_eq!(events[0]["readiness"]["admission_capacity"], 4);
    for line in [1_u64, 5] {
        let admitted = position(&events, |event| {
            event["kind"] == "request_admitted" && event["line"] == line
        })?;
        let result = position(&events, |event| {
            event["event"] == "result" && event["line"] == line
        })?;
        let finished = position(&events, |event| {
            event["kind"] == "request_finished" && event["line"] == line
        })?;
        assert!(admitted < result && result < finished, "line {line}");
        let operation_id = events[admitted]["request_operation_id"].clone();
        for (index, event) in events.iter().enumerate() {
            if event["event"] == "progress" && event["request_operation_id"] == operation_id {
                assert!(admitted < index && index < result, "line {line}");
            }
        }
    }
    for line in [3_u64, 4] {
        let finished = &events[position(&events, |event| {
            event["kind"] == "request_finished" && event["line"] == line
        })?];
        assert_eq!(finished["status"], "rejected");
        assert_eq!(finished["code"], "INVALID_ARGUMENT");
        assert!(!events.iter().any(|event| {
            (event["event"] == "result" || event["kind"] == "request_admitted")
                && event["line"] == line
        }));
    }
    assert!(!events.iter().any(|event| event["line"] == 2));
    let stopped = &events[events.len() - 2];
    assert_eq!(stopped["reason"], "end_of_input");
    let terminal = &events[events.len() - 1]["result"];
    assert_eq!(terminal["command"], "job.batch");
    assert_eq!(terminal["status"], "failed");
    assert_eq!(terminal["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(
        terminal["error"]["remediation"][0]["summary"],
        BATCH_LINES_FAILED_REMEDIATION
    );
    let data = &terminal["data"];
    assert_eq!(data["counts"]["complete"], 2);
    assert_eq!(data["counts"]["rejected"], 2);
    assert_eq!(item(data, 3)?["rejection"], "malformed_request");
    assert_eq!(item(data, 4)?["rejection"], "duplicate_operation_id");

    // `--json`: the summary alone; the file again replays what ended.
    let output = layout
        .job_batch(&["--concurrency", "2", "--json"])
        .output()?;
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    layout.assert_nothing_sensitive(&output.stdout, &output.stderr);
    let value = json_result(&output.stdout)?;
    assert_eq!(value["data"]["counts"], data["counts"]);
    assert_eq!(value["operation_id"], Value::Null);
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

/// Human mode (P13 PR 2b): the batch summary is readable text on stdout,
/// and a batch that ends in a failure adds its error on stderr. A refused
/// line with hostile text is counted and named by its line only: nothing
/// of it is echoed (O-01, SEC-T02).
#[test]
fn human_output_summarises_the_batch_and_echoes_nothing() -> TestResult {
    let layout = Layout::new()?;
    layout.init(2)?;
    let hostile = format!(
        r#"{{"schema_version":"1","operation_id":"op_cli{PATH_SENTINEL}","\u202e\u001b]8;;https://example.invalid\u0007":1}}"#
    );
    layout.write_batch(&[close_only(1), close_only(2), hostile])?;
    let output = layout.job_batch(&[]).output()?;
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    layout.assert_nothing_sensitive(&output.stdout, &output.stderr);
    let summary = assert_terminal_safe(&output.stdout)?;
    assert!(
        summary.starts_with("Worker batch ended: end_of_input\n"),
        "{summary}"
    );
    assert!(
        summary.contains("Requests: 2 complete, 0 partial, 0 failed, 0 cancelled, 1 rejected"),
        "{summary}"
    );
    for line in 1..=3 {
        assert!(summary.contains(&format!("  line {line}  ")), "{summary}");
    }
    assert!(!summary.contains("example.invalid") && !summary.contains("\"command\""));
    let error = assert_terminal_safe(&output.stderr)?;
    assert!(
        error.starts_with("Error: ") && error.contains("(INVALID_ARGUMENT)"),
        "{error}"
    );
    assert!(!error.contains("example.invalid"), "{error}");
    Ok(())
}

/// A batch whose every request completes exits 0 with status `complete`.
#[test]
fn a_complete_batch_succeeds() -> TestResult {
    let layout = Layout::new()?;
    layout.init(2)?;
    layout.write_batch(&(1..=4).map(close_only).collect::<Vec<_>>())?;
    let output = layout
        .job_batch(&["--concurrency", "2", "--json"])
        .output()?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value = json_result(&output.stdout)?;
    assert_eq!(value["status"], "complete");
    assert_eq!(value["error"], Value::Null);
    assert_eq!(value["data"]["counts"]["complete"], 4);
    assert_eq!(value["data"]["termination_reason"], "end_of_input");
    Ok(())
}

/// Limits and refusals before any work: more than 1,000 lines (exit 5,
/// `line_limit`, nothing run), a missing file (exit 7, `input_error`), a
/// concurrency above the capacity and a root that is not a workspace (exit
/// 2, one terminal event, no `started`).
#[test]
fn limits_are_refused_before_any_work() -> TestResult {
    let layout = Layout::new()?;
    layout.init(2)?;
    layout.write_batch(&(1..=1_001).map(close_only).collect::<Vec<_>>())?;
    let output = layout.job_batch(&["--json"]).output()?;
    assert_eq!(output.status.code(), Some(5), "{output:?}");
    let value = json_result(&output.stdout)?;
    assert_eq!(value["error"]["code"], "RESOURCE_LIMIT");
    assert_eq!(
        value["error"]["remediation"][0]["summary"],
        BATCH_LINE_LIMIT_REMEDIATION
    );
    assert_eq!(value["data"]["termination_reason"], "line_limit");
    assert_eq!(value["data"]["not_started_from_line"], 1);
    assert_eq!(layout.recorded_requests()?, 0);

    fs::remove_file(layout.requests())?;
    let output = layout.job_batch(&["--events", "jsonl"]).output()?;
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    let events = parse_events(&output.stdout)?;
    assert_eq!(lifecycle_kinds(&events), ["started", "stopped"]);
    assert_eq!(events[1]["reason"], "input_error");
    let terminal = &events[2]["result"];
    assert_eq!(terminal["error"]["code"], "STORAGE_IO");
    assert_eq!(
        terminal["error"]["remediation"][0]["summary"],
        BATCH_FILE_REMEDIATION
    );
    layout.assert_nothing_sensitive(&output.stdout, &output.stderr);

    layout.write_batch(&[close_only(1)])?;
    let output = layout
        .job_batch(&["--concurrency", "3", "--events", "jsonl"])
        .output()?;
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let events = parse_events(&output.stdout)?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["result"]["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(
        events[0]["result"]["error"]["remediation"][0]["summary"],
        BATCH_CONCURRENCY_REMEDIATION
    );

    let desktop = Layout::new()?;
    desktop.write_batch(&[close_only(1)])?;
    fs::create_dir(desktop.workspace())?;
    let output = desktop.job_batch(&["--json"]).output()?;
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(desktop.recorded_requests()?, 0);
    Ok(())
}

/// O-03: from typed events and codes alone a supervisor tells a ready host
/// (`started` with its readiness) from a busy one (`admission_waiting`,
/// then `BUSY`, retryable, exit 4), an unhealthy one (no `started`: the
/// workspace cannot be used, `STORAGE_IO`, exit 7) and a missing capability
/// (`MISSING_CAPABILITY` for a step whose tool is absent; strict isolation
/// this host cannot attest is `ISOLATION_UNAVAILABLE` before any work).
#[test]
fn lifecycle_events_distinguish_ready_busy_unhealthy_missing() -> TestResult {
    let layout = Layout::new()?;
    layout.init(2)?;

    // Busy: every admission unit is held elsewhere.
    layout.write_batch(&[close_only(1)])?;
    let store = FilesystemSessionStore::open_existing(layout.workspace())?;
    let permit = store.try_admit(2)?;
    let output = layout
        .job_batch(&["--admission-wait-ms", "400", "--events", "jsonl"])
        .output()?;
    drop(permit);
    assert_eq!(output.status.code(), Some(4), "{output:?}");
    let busy = parse_events(&output.stdout)?;
    let started = &busy[0];
    assert_eq!(started["kind"], "started");
    assert_eq!(
        started["readiness"]["publication"],
        "process_crash_consistent"
    );
    assert_eq!(started["readiness"]["isolation"], "process_only");
    assert!(lifecycle_kinds(&busy).contains(&"admission_waiting".to_owned()));
    let finished = &busy[position(&busy, |event| event["kind"] == "request_finished")?];
    assert_eq!(finished["code"], "BUSY");
    let result = &busy[position(&busy, |event| event["event"] == "result")?];
    assert_eq!(result["result"]["failure"]["retryable"], true);

    // Missing capability: a candidates step without FFmpeg on PATH.
    layout.write_batch(&[request(2, r#"{"candidates":{"range":null}}"#)])?;
    let output = layout.job_batch(&["--events", "jsonl"]).output()?;
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let missing = parse_events(&output.stdout)?;
    assert_eq!(missing[0]["kind"], "started");
    let finished = &missing[position(&missing, |event| event["kind"] == "request_finished")?];
    assert_eq!(finished["code"], "MISSING_CAPABILITY");

    // Unhealthy: the workspace is gone; nothing starts.
    let gone = Layout::new()?;
    gone.write_batch(&[close_only(3)])?;
    let output = gone.job_batch(&["--events", "jsonl"]).output()?;
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    let unhealthy = parse_events(&output.stdout)?;
    assert_eq!(unhealthy.len(), 1);
    assert_eq!(unhealthy[0]["result"]["error"]["code"], "STORAGE_IO");

    // Missing capability of the host: strict isolation is attested first.
    #[cfg(not(target_os = "linux"))]
    {
        let output = layout
            .command()
            .args(["--host-isolation", "strict-linux", "--session-root"])
            .arg(layout.workspace())
            .args(["job", "batch", "--requests"])
            .arg(layout.requests())
            .arg("--input-root")
            .arg(layout.inputs())
            .args(["--events", "jsonl"])
            .output()?;
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        let refused = parse_events(&output.stdout)?;
        assert_eq!(refused.len(), 1);
        assert_eq!(
            refused[0]["result"]["error"]["code"],
            "ISOLATION_UNAVAILABLE"
        );
    }
    Ok(())
}

/// X-08 through the binary: while nobody reads the batch's stdout, it
/// starts only as many requests as the pipe, the bounded event queue and
/// the running slots hold, and then nothing more; read again, it runs every
/// line. On Linux the process's resident memory stays bounded meanwhile.
#[test]
fn a_paused_stdout_reader_bounds_memory_and_admission() -> TestResult {
    let layout = Layout::new()?;
    layout.init(2)?;
    let total = 120;
    layout.write_batch(&(1..=total).map(close_only).collect::<Vec<_>>())?;
    let mut child = layout
        .job_batch(&["--concurrency", "2", "--events", "jsonl"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    // Wait until the started count stops growing while stdout is unread.
    let mut previous = usize::MAX;
    let mut stable_since = Instant::now();
    let started = Instant::now();
    loop {
        thread::sleep(Duration::from_millis(250));
        let now = layout.recorded_requests()?;
        if now != previous {
            previous = now;
            stable_since = Instant::now();
        }
        if stable_since.elapsed() > Duration::from_millis(1_500) {
            break;
        }
        if started.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            return Err("the batch never paused".into());
        }
    }
    assert!(
        child.try_wait()?.is_none(),
        "the batch ended while nobody read it"
    );
    let paused = layout.recorded_requests()?;
    assert!(
        paused < total / 2,
        "{paused} of {total} requests started while stdout was unread"
    );
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string(format!("/proc/{}/status", child.id()))?;
        let resident_kib: u64 = status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse().ok())
            .ok_or("no VmRSS")?;
        assert!(resident_kib < 256 * 1024, "{resident_kib} KiB resident");
    }
    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .ok_or("no stdout")?
        .read_to_end(&mut stdout)?;
    let status = child.wait()?;
    assert_eq!(status.code(), Some(0));
    let events = parse_events(&stdout)?;
    let terminal = &events[events.len() - 1]["result"];
    assert_eq!(terminal["data"]["counts"]["complete"], total);
    assert_eq!(layout.recorded_requests()?, total);
    Ok(())
}

/// One generated batch line (O-02).
#[derive(Clone, Debug)]
enum Generated {
    Close,
    RetainAndClose,
    Blank,
    Malformed,
    Duplicate,
    TooLong,
}

fn generated_line() -> impl Strategy<Value = Generated> {
    prop_oneof![
        Just(Generated::Close),
        Just(Generated::RetainAndClose),
        Just(Generated::Blank),
        Just(Generated::Malformed),
        Just(Generated::Duplicate),
        Just(Generated::TooLong),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 12,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// O-02: whatever a batch holds, every event line but the terminal one
    /// is within 64 KiB and schema-valid (every string member an enum or a
    /// bounded pattern), no path reaches the output, the summary has one
    /// item per non-blank line, and the stream has at most a fixed number
    /// of events per line: the host's buffers and labels grow with the
    /// lines of the file only, never with what they contain.
    #[test]
    fn events_are_bounded_by_the_lines(lines in prop::collection::vec(generated_line(), 1..10)) {
        let checked = (|| -> TestResult {
            let layout = Layout::new()?;
            layout.init(2)?;
            let mut text = Vec::new();
            for (index, generated) in lines.iter().enumerate() {
                text.push(match generated {
                    Generated::Close => close_only(index + 1),
                    Generated::RetainAndClose => retain_and_close(index + 1),
                    Generated::Blank => String::new(),
                    Generated::Malformed => format!("{{\"{PATH_SENTINEL}\":{index}}}"),
                    Generated::Duplicate => close_only(1),
                    Generated::TooLong => format!("{{\"x\":\"{}\"}}", "y".repeat(70_000)),
                });
            }
            layout.write_batch(&text)?;
            let output = layout.job_batch(&["--concurrency", "2", "--events", "jsonl"]).output()?;
            layout.assert_nothing_sensitive(&output.stdout, &output.stderr);
            let events = parse_events(&output.stdout)?;
            let non_blank = lines.iter().filter(|line| !matches!(line, Generated::Blank)).count();
            let terminal = &events[events.len() - 1]["result"];
            assert_eq!(terminal["data"]["items"].as_array().map(Vec::len), Some(non_blank));
            // started, stopped, the terminal event and at most a draining;
            // per line: admitted, result, finished, and per step at most
            // one progress and one admission notice (each request has at
            // most three steps here), plus its held progress.
            assert!(events.len() <= 4 + lines.len() * (3 + 2 * 3 + 1), "{} events", events.len());
            Ok(())
        })();
        prop_assert!(checked.is_ok(), "{checked:?}");
    }
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

/// Waits for the stopped child within the budget; returns its exit code,
/// how long it took after the signal, and the rest of its stream.
fn finish(
    mut child: Child,
    lines: &mpsc::Receiver<String>,
    mut seen: Vec<String>,
) -> Built<(Option<i32>, Duration, Vec<String>)> {
    let sent = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if sent.elapsed() > SHUTDOWN_BUDGET {
            let _ = child.kill();
            return Err("the stopped batch outlived its shutdown budget".into());
        }
        thread::sleep(Duration::from_millis(10));
    };
    let took = sent.elapsed();
    while let Ok(line) = lines.recv_timeout(Duration::from_millis(500)) {
        seen.push(line);
    }
    Ok((status.code(), took, seen))
}

/// A signal sender: `kill -s TERM` on Unix, the console helper on Windows.
type Signal = fn(&Child) -> TestResult;

/// O-04 for a batch: the only admission unit is held, so the first request
/// waits; a shutdown signal then stops the batch. Without a drain time the
/// waiting ingest is cancelled at once; with one it may finish (the unit is
/// freed) and its close never starts. Either way no further line starts,
/// the stream says `draining` then `stopped: shutdown`, the summary names
/// line 2 as the first not started, the process exits 6 within the budget,
/// and delivering the file again completes every line.
fn shutdown_stops_a_batch(signal: Signal, windows_console: bool) -> TestResult {
    for drain in [false, true] {
        let layout = Layout::new()?;
        layout.init(1)?;
        layout.write_batch(&(1..=3).map(close_only).collect::<Vec<_>>())?;
        let store = FilesystemSessionStore::open_existing(layout.workspace())?;
        let permit = store.try_admit(1)?;
        let mut extra = vec!["--events", "jsonl", "--admission-wait-ms", "60000"];
        if drain {
            extra.extend(["--drain-timeout-ms", "20000"]);
        }
        let mut command = layout.job_batch(&extra);
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
        }
        drop(permit);
        let (code, took, seen) = finish(child, &lines, seen)?;
        let stdout: Vec<u8> = seen.join("\n").into_bytes();
        let events = parse_events(&stdout)?;
        assert_eq!(code, Some(6), "{seen:?}");
        assert!(took < SHUTDOWN_BUDGET);
        let kinds = lifecycle_kinds(&events);
        assert!(kinds.contains(&"draining".to_owned()), "{kinds:?}");
        assert_eq!(kinds.last().map(String::as_str), Some("stopped"));
        assert!(
            !events
                .iter()
                .any(|event| event["kind"] == "request_admitted" && event["line"] != 1),
            "{kinds:?}"
        );
        let stopped = &events[events.len() - 2];
        assert_eq!(stopped["reason"], "shutdown");
        let terminal = &events[events.len() - 1]["result"];
        assert_eq!(terminal["status"], "cancelled");
        assert_eq!(terminal["error"]["code"], "CANCELLED");
        assert_eq!(
            terminal["error"]["remediation"][0]["summary"],
            BATCH_STOPPED_REMEDIATION
        );
        assert_eq!(terminal["data"]["termination_reason"], "shutdown");
        assert_eq!(terminal["data"]["not_started_from_line"], 2);
        let result = &events[position(&events, |event| event["event"] == "result")?]["result"];
        let steps = &result["steps"];
        if drain {
            assert_eq!(steps[0]["status"], "complete", "{steps}");
            assert_eq!(steps[1]["status"], "cancelled", "{steps}");
        } else {
            assert_eq!(steps[0]["status"], "cancelled", "{steps}");
        }

        let output = layout.job_batch(&["--json"]).output()?;
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let value = json_result(&output.stdout)?;
        assert_eq!(value["data"]["counts"]["complete"], 3);
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
fn sigterm_stops_a_batch_resumably() -> TestResult {
    shutdown_stops_a_batch(send_sigterm, false)
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
fn ctrl_break_stops_a_batch_resumably() -> TestResult {
    shutdown_stops_a_batch(send_ctrl_break, true)
}
