//! Worker batches through the engine (P11 PR 4, ADR 0021 section 5):
//! `Engine::run_work_batch` in an ephemeral worker workspace.
//!
//! Everything here runs without `FFmpeg`: the requests ingest a placeholder
//! source (a plain ingest runs no provider), retain and close it. The tests
//! cover line isolation (a malformed, blank, duplicate and refused line
//! beside requests that run), X-08 (the next line is read only when a slot
//! frees; a host that stops reading events holds the batch back), a
//! shutdown that stops the reading and leaves started requests resumable,
//! two batches sharing one workspace, and S-07 for batches: a process
//! killed at each request fault point in the middle of a batch leaves a
//! workspace that a rerun of the same file completes with the results of an
//! uninterrupted control and one session per operation id.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    num::NonZeroU16,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use tokio::sync::mpsc;
use vsift::{
    AdmissionBudget, AdmissionWait, BatchEvent, BatchLineEnd, Cancellation, DurabilityRequirement,
    Engine, EngineConfig, EngineError, EnginePorts, HostIsolation, ProgressObserver,
    SessionListEntry, SessionRootLocation, UserConfigurationLocation, WorkBatchRun, WorkerFailure,
    WorkspaceInitRequest, WorkspacePolicy, WorkspaceRetention,
};
use vsift_contract::{BatchOutcome, BatchTermination, JobBatchData};
use vsift_domain::FailureCode;
use vsift_infrastructure::FilesystemSessionStore;

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-engine-batch-test-";
const SOURCE: &[u8] = b"\0\0\0\x18ftypisomengine-batch-source";
const CHILD_ROOT: &str = "VSIFT_BATCH_TEST_ROOT";
const FAULT_POINT: &str = "VSIFT_FAULT_POINT";
const FAULT_EXIT_CODE: i32 = 91;
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
/// How long a test waits for an event it expects.
const EVENT_WAIT: Duration = Duration::from_secs(30);

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

/// A temporary directory holding the workspace, the input and bundle roots
/// and the batch files; removed with everything in it.
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
        fs::create_dir_all(layout.inputs().join("videos"))?;
        fs::create_dir(layout.bundles())?;
        fs::write(layout.inputs().join("videos/talk.mp4"), SOURCE)?;
        Ok(layout)
    }

    fn at(path: PathBuf) -> Self {
        Self(path)
    }

    fn workspace(&self) -> PathBuf {
        self.0.join("workspace")
    }

    fn inputs(&self) -> PathBuf {
        self.0.join("inputs")
    }

    fn bundles(&self) -> PathBuf {
        self.0.join("bundles")
    }

    fn requests(&self) -> PathBuf {
        self.0.join("requests.jsonl")
    }

    fn engine(&self) -> Arc<Engine> {
        Arc::new(Engine::new(
            EngineConfig {
                session_root: SessionRootLocation::Explicit(self.workspace()),
                user_configuration: UserConfigurationLocation::Explicit(self.0.join("config")),
                host_isolation: HostIsolation::ProcessOnly,
            },
            EnginePorts::system(),
        ))
    }

    /// Creates the ephemeral workspace with `capacity` admission units.
    fn init(&self, capacity: u16) -> Built<Arc<Engine>> {
        let engine = self.engine();
        engine.init_workspace(WorkspaceInitRequest {
            policy: WorkspacePolicy::new(
                DurabilityRequirement::Ephemeral,
                NonZeroU16::new(capacity).ok_or("zero")?,
                WorkspaceRetention::from_hours(2)?,
            )?,
        })?;
        Ok(engine)
    }

    fn store(&self) -> Built<FilesystemSessionStore> {
        Ok(FilesystemSessionStore::open_existing(self.workspace())?)
    }

    fn write_batch(&self, lines: &[String]) -> TestResult {
        let mut text = lines.join("\n");
        text.push('\n');
        fs::write(self.requests(), text)?;
        Ok(())
    }

    /// A run of the layout's batch file, reporting to `events`.
    fn run(
        &self,
        concurrency: u16,
        admission: AdmissionWait,
        events: mpsc::Sender<BatchEvent>,
    ) -> Built<WorkBatchRun> {
        Ok(WorkBatchRun {
            requests: self.requests(),
            input_root: self.inputs(),
            bundle_root: Some(self.bundles()),
            admission,
            concurrency: NonZeroU16::new(concurrency).ok_or("zero")?,
            stop: Cancellation::new(),
            cancellation: Cancellation::new(),
            progress: Arc::new(|_, _| ProgressObserver::none()),
            events,
        })
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
                if entry?.file_name().to_string_lossy().ends_with(".json") {
                    records += 1;
                }
            }
        }
        Ok(records)
    }

    /// The sessions of the workspace that opened.
    fn opened_sessions(engine: &Engine) -> Built<usize> {
        let mut opened = 0;
        let mut cursor = None;
        loop {
            let page = engine.list_sessions(cursor)?;
            opened += page
                .entries()
                .iter()
                .filter(|entry| matches!(entry, SessionListEntry::Indexed(_)))
                .count();
            match page.next_cursor() {
                Some(next) => cursor = Some(next),
                None => return Ok(opened),
            }
        }
    }
}

impl Drop for Layout {
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

/// A request that ingests `videos/talk.mp4` under `operation`, then
/// `steps`.
fn request(operation: &str, durability: &str, steps: &str) -> String {
    format!(
        r#"{{"schema_version":"1","operation_id":"{operation}","durability":"{durability}","target":{{"ingest":{{"source":"videos/talk.mp4","transcript":null}}}},"steps":[{steps}]}}"#
    )
}

/// The `index`-th operation id of a test.
fn operation(index: usize) -> String {
    format!("op_batch{index:024}")
}

fn close_only(index: usize) -> String {
    request(&operation(index), "ephemeral", r#"{"close":{}}"#)
}

fn retain_and_close(index: usize) -> String {
    request(
        &operation(index),
        "ephemeral",
        &format!(
            r#"{{"retain":{{"bundle_name":"bundle-{index}","include_source":false}}}},{{"close":{{}}}}"#
        ),
    )
}

fn bounded(millis: u64) -> Built<AdmissionWait> {
    Ok(AdmissionWait::Bounded(AdmissionBudget::new(
        Duration::from_millis(millis),
    )?))
}

/// Runs a batch while collecting every event it sends, and returns both.
async fn run_collecting(
    engine: &Arc<Engine>,
    layout: &Layout,
    concurrency: u16,
    admission: AdmissionWait,
) -> Built<(Result<JobBatchData, EngineError>, Vec<BatchEvent>)> {
    let (sender, mut receiver) = mpsc::channel(4);
    let run = layout.run(concurrency, admission, sender)?;
    let batch = tokio::spawn(Arc::clone(engine).run_work_batch(run));
    let mut events = Vec::new();
    while let Some(event) = receiver.recv().await {
        events.push(event);
    }
    Ok((batch.await?, events))
}

/// The summary as JSON, validated against its published schema.
fn summary_value(summary: &JobBatchData) -> Built<Value> {
    let value = serde_json::to_value(summary)?;
    validate("job-batch-data.schema.json", &value)?;
    Ok(value)
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
        .map_err(|error| std::io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

/// The line of an event.
const fn line_of(event: &BatchEvent) -> u32 {
    match event {
        BatchEvent::Admitted { line, .. } | BatchEvent::Finished { line, .. } => *line,
    }
}

/// The result of a line's `Finished` event, as JSON, when it has one.
fn result_of(events: &[BatchEvent], line: u32) -> Built<Option<Value>> {
    for event in events {
        if let BatchEvent::Finished {
            line: finished,
            end: BatchLineEnd::Ran(outcome),
        } = event
            && *finished == line
        {
            let value = serde_json::to_value(outcome.result())?;
            validate("job-result.schema.json", &value)?;
            return Ok(Some(value));
        }
    }
    Ok(None)
}

/// The item of `line` in a summary.
fn item(summary: &Value, line: u32) -> Built<Value> {
    summary["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["line"] == line))
        .cloned()
        .ok_or_else(|| format!("no item for line {line}: {summary}").into())
}

/// X-11 (without providers): every line is independent. A request that
/// runs, a blank line (skipped, still counted), a malformed line, a line
/// that reuses an operation id, a durable request in an ephemeral
/// workspace and a missing source are each reported alone, and the batch's
/// outcome is the most severe failure; every admitted line is admitted
/// before it finishes, and a refused line is never admitted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_line_is_isolated_and_reported() -> TestResult {
    let layout = Layout::new()?;
    let engine = layout.init(4)?;
    layout.write_batch(&[
        retain_and_close(1),
        String::new(),
        "not json".to_owned(),
        close_only(1),
        request(&operation(5), "durable", r#"{"close":{}}"#),
        close_only(6).replace("videos/talk.mp4", "videos/missing.mp4"),
        close_only(7),
    ])?;
    let (summary, events) = run_collecting(&engine, &layout, 2, bounded(30_000)?).await?;
    let summary = summary?;
    assert_eq!(summary.termination(), BatchTermination::EndOfInput);
    assert_eq!(
        summary.outcome(),
        BatchOutcome::Failed(FailureCode::InvalidArgument)
    );
    let value = summary_value(&summary)?;
    assert_eq!(value["counts"]["complete"], 2, "{value}");
    assert_eq!(value["counts"]["rejected"], 3, "{value}");
    assert_eq!(value["counts"]["failed"], 1, "{value}");
    assert_eq!(value["items"].as_array().map(Vec::len), Some(6));
    assert_eq!(item(&value, 1)?["status"], "complete");
    assert_eq!(item(&value, 3)?["rejection"], "malformed_request");
    assert_eq!(item(&value, 3)?["operation_id"], Value::Null);
    assert_eq!(item(&value, 4)?["rejection"], "duplicate_operation_id");
    assert_eq!(item(&value, 4)?["operation_id"], operation(1));
    assert_eq!(item(&value, 5)?["rejection"], "workspace_not_durable");
    assert_eq!(item(&value, 6)?["status"], "failed");
    assert_eq!(item(&value, 6)?["code"], "INVALID_ARGUMENT");
    assert_eq!(item(&value, 7)?["status"], "complete");
    assert_eq!(value["not_started_from_line"], Value::Null);

    // Admitted before finished; refused lines never admitted.
    for line in [1, 5, 6, 7] {
        let admitted = events
            .iter()
            .position(|event| matches!(event, BatchEvent::Admitted { line: l, .. } if *l == line));
        let finished = events
            .iter()
            .position(|event| matches!(event, BatchEvent::Finished { line: l, .. } if *l == line));
        assert!(
            matches!((admitted, finished), (Some(a), Some(f)) if a < f),
            "line {line}"
        );
    }
    for line in [2, 3, 4] {
        assert!(!events.iter().any(|event| matches!(
            event,
            BatchEvent::Admitted { line: l, .. } if *l == line
        )));
    }
    assert!(!events.iter().any(|event| line_of(event) == 2));
    assert!(result_of(&events, 3)?.is_none());
    assert_eq!(
        result_of(&events, 5)?.ok_or("no result for line 5")?["failure"]["rejection"],
        "workspace_not_durable"
    );
    assert_eq!(Layout::opened_sessions(&engine)?, 2);

    // Delivering the same file again replays what ended.
    let (again, _) = run_collecting(&engine, &layout, 2, bounded(30_000)?).await?;
    let again = summary_value(&again?)?;
    assert_eq!(again["counts"], value["counts"]);
    assert_eq!(Layout::opened_sessions(&engine)?, 2);
    Ok(())
}

/// Before any work: a concurrency above the workspace's capacity, and a
/// root that is not a worker workspace; a file of more than 1,000 lines is
/// refused whole (`line_limit`, nothing run) and a missing file is an
/// `input_error`.
#[tokio::test]
async fn limits_are_checked_before_any_work() -> TestResult {
    let layout = Layout::new()?;
    let engine = layout.init(2)?;
    layout.write_batch(&[close_only(1)])?;
    let (sender, _receiver) = mpsc::channel(4);
    let refused = Arc::clone(&engine)
        .run_work_batch(layout.run(3, AdmissionWait::Immediate, sender)?)
        .await;
    assert!(matches!(
        refused,
        Err(EngineError::Worker(
            WorkerFailure::ConcurrencyExceedsCapacity
        ))
    ));

    let lines: Vec<String> = (1..=1_001).map(close_only).collect();
    layout.write_batch(&lines)?;
    let (summary, events) = run_collecting(&engine, &layout, 2, bounded(30_000)?).await?;
    let summary = summary?;
    assert!(events.is_empty());
    assert_eq!(summary.termination(), BatchTermination::LineLimit);
    assert_eq!(summary.not_started_from_line(), Some(1));
    assert_eq!(
        summary.outcome(),
        BatchOutcome::Failed(FailureCode::ResourceLimit)
    );
    summary_value(&summary)?;
    assert_eq!(Layout::opened_sessions(&engine)?, 0);

    fs::remove_file(layout.requests())?;
    let (summary, _) = run_collecting(&engine, &layout, 1, bounded(30_000)?).await?;
    let summary = summary?;
    assert_eq!(summary.termination(), BatchTermination::InputError);
    assert_eq!(
        summary.outcome(),
        BatchOutcome::Failed(FailureCode::StorageIo)
    );

    let desktop = Layout::new()?;
    desktop.write_batch(&[close_only(1)])?;
    let (sender, _receiver) = mpsc::channel(4);
    let refused = desktop
        .engine()
        .run_work_batch(desktop.run(1, AdmissionWait::Immediate, sender)?)
        .await;
    assert!(refused.is_err());
    assert!(!desktop.workspace().exists());
    Ok(())
}

/// Receives the next event within [`EVENT_WAIT`].
async fn next_event(receiver: &mut mpsc::Receiver<BatchEvent>) -> Built<BatchEvent> {
    tokio::time::timeout(EVENT_WAIT, receiver.recv())
        .await?
        .ok_or_else(|| "the batch ended early".into())
}

/// X-08: with every admission unit held, `concurrency` requests start and
/// wait; the line after them is malformed, so had it been read its refusal
/// would be reported at once. It is not: nothing more happens until a
/// request ends, and the malformed line is reported only after one of the
/// running requests finished.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn batch_reads_ahead_at_most_concurrency_lines() -> TestResult {
    for concurrency in [1_u16, 2, 3] {
        let layout = Layout::new()?;
        let engine = layout.init(3)?;
        let running = usize::from(concurrency);
        let mut lines: Vec<String> = (1..=running).map(close_only).collect();
        lines.push("not json".to_owned());
        lines.push(close_only(9));
        layout.write_batch(&lines)?;
        let permit = layout.store()?.try_admit(3)?;
        let (sender, mut receiver) = mpsc::channel(4);
        let batch = tokio::spawn(Arc::clone(&engine).run_work_batch(layout.run(
            concurrency,
            bounded(30_000)?,
            sender,
        )?));
        let mut seen = Vec::new();
        for _ in 0..running {
            seen.push(next_event(&mut receiver).await?);
        }
        assert!(
            seen.iter()
                .all(|event| matches!(event, BatchEvent::Admitted { .. })),
            "{seen:?}"
        );
        // Nothing else happens while every slot waits.
        let quiet = tokio::time::timeout(Duration::from_millis(750), receiver.recv()).await;
        assert!(quiet.is_err(), "concurrency {concurrency}: {quiet:?}");
        drop(permit);
        while let Some(event) = receiver.recv().await {
            seen.push(event);
        }
        let summary = batch.await??;
        let malformed = u32::try_from(running + 1)?;
        let refused = seen
            .iter()
            .position(|event| line_of(event) == malformed)
            .ok_or("the malformed line was not reported")?;
        let first_finished = seen
            .iter()
            .position(|event| matches!(event, BatchEvent::Finished { .. }))
            .ok_or("nothing finished")?;
        assert!(first_finished < refused, "{seen:?}");
        let value = summary_value(&summary)?;
        assert_eq!(value["counts"]["complete"], running + 1);
        assert_eq!(value["counts"]["rejected"], 1);
    }
    Ok(())
}

/// X-08: a host that stops reading events holds the batch back. With a
/// one-event channel nobody reads, at most `concurrency` requests start;
/// once the host reads again, every line runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_host_that_stops_reading_holds_the_batch_back() -> TestResult {
    let layout = Layout::new()?;
    let engine = layout.init(4)?;
    let lines: Vec<String> = (1..=12).map(close_only).collect();
    layout.write_batch(&lines)?;
    let (sender, mut receiver) = mpsc::channel(1);
    let batch = tokio::spawn(Arc::clone(&engine).run_work_batch(layout.run(
        2,
        bounded(30_000)?,
        sender,
    )?));
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert!(!batch.is_finished());
    // Counted from the request records on disk: listing sessions while
    // requests register theirs may answer the documented `BUSY`.
    let started = layout.recorded_requests()?;
    assert!(
        started <= 2,
        "{started} requests started while the host was not reading"
    );
    let mut events = 0;
    while receiver.recv().await.is_some() {
        events += 1;
    }
    let summary = batch.await??;
    assert_eq!(events, 24);
    assert_eq!(summary_value(&summary)?["counts"]["complete"], 12);
    assert_eq!(Layout::opened_sessions(&engine)?, 12);
    Ok(())
}

/// O-04 for a batch: a shutdown while two requests wait for admission stops
/// the reading (the first line not started is named), stops the running
/// requests at their next boundary (`cancelled`, resumable) and makes the
/// batch `shutdown` (D5: stopped). Delivering the file again completes every
/// line, the stopped ones as their second attempt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shutdown_stops_the_batch_and_leaves_it_resumable() -> TestResult {
    let layout = Layout::new()?;
    let engine = layout.init(2)?;
    let lines: Vec<String> = (1..=5).map(close_only).collect();
    layout.write_batch(&lines)?;
    let permit = layout.store()?.try_admit(2)?;
    let (sender, mut receiver) = mpsc::channel(4);
    let run = layout.run(2, bounded(30_000)?, sender)?;
    let stop = run.stop.clone();
    let cancellation = run.cancellation.clone();
    let batch = tokio::spawn(Arc::clone(&engine).run_work_batch(run));
    for _ in 0..2 {
        assert!(matches!(
            next_event(&mut receiver).await?,
            BatchEvent::Admitted { .. }
        ));
    }
    stop.cancel();
    cancellation.cancel();
    let mut events = Vec::new();
    while let Some(event) = receiver.recv().await {
        events.push(event);
    }
    drop(permit);
    let summary = batch.await??;
    assert_eq!(summary.termination(), BatchTermination::Shutdown);
    assert_eq!(summary.not_started_from_line(), Some(3));
    assert_eq!(summary.outcome(), BatchOutcome::Stopped);
    let value = summary_value(&summary)?;
    assert_eq!(value["counts"]["cancelled"], 2, "{value}");
    for line in [1, 2] {
        let result = result_of(&events, line)?.ok_or("no result")?;
        assert_eq!(result["status"], "cancelled");
        assert_eq!(result["failure"]["code"], "CANCELLED");
    }

    let (again, _) = run_collecting(&engine, &layout, 2, bounded(30_000)?).await?;
    let again = summary_value(&again?)?;
    assert_eq!(again["counts"]["complete"], 5, "{again}");
    assert_eq!(again["termination_reason"], "end_of_input");
    assert_eq!(Layout::opened_sessions(&engine)?, 5);
    Ok(())
}

/// Two batches, from two engines, share one workspace: their own lines all
/// complete, and the one request both files hold commits once. The second
/// delivery of it is, as the request table documents, either its recorded
/// result (`replayed`) or `BUSY` while the other runs it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_batches_share_one_workspace() -> TestResult {
    let layout = Layout::new()?;
    let first_engine = layout.init(4)?;
    let second_engine = layout.engine();
    let other = Layout::new()?;
    let shared = retain_and_close(50);
    let first_lines: Vec<String> = (1..=5).map(close_only).chain([shared.clone()]).collect();
    let second_lines: Vec<String> = (11..=15).map(close_only).chain([shared]).collect();
    layout.write_batch(&first_lines)?;
    fs::write(other.0.join("requests.jsonl"), second_lines.join("\n"))?;

    let (first_sender, first_events) = mpsc::channel(4);
    let (second_sender, second_events) = mpsc::channel(4);
    let first_run = layout.run(2, bounded(30_000)?, first_sender)?;
    let mut second_run = layout.run(2, bounded(30_000)?, second_sender)?;
    second_run.requests = other.0.join("requests.jsonl");
    let first = tokio::spawn(Arc::clone(&first_engine).run_work_batch(first_run));
    let second = tokio::spawn(second_engine.run_work_batch(second_run));
    let drain = |mut receiver: mpsc::Receiver<BatchEvent>| async move {
        let mut events = Vec::new();
        while let Some(event) = receiver.recv().await {
            events.push(event);
        }
        events
    };
    let (first_seen, second_seen) = tokio::join!(drain(first_events), drain(second_events));
    let first = summary_value(&first.await??)?;
    let second = summary_value(&second.await??)?;
    let shared_results = [
        result_of(&first_seen, 6)?.ok_or("no shared result")?,
        result_of(&second_seen, 6)?.ok_or("no shared result")?,
    ];
    let fresh = shared_results
        .iter()
        .filter(|result| result["status"] == "complete" && result["replayed"] == false)
        .count();
    assert_eq!(fresh, 1, "{shared_results:?}");
    for result in &shared_results {
        let documented = result["status"] == "complete"
            || (result["status"] == "failed" && result["failure"]["code"] == "BUSY");
        assert!(documented, "{result}");
    }
    for summary in [&first, &second] {
        for line in 1..=5 {
            assert_eq!(item(summary, line)?["status"], "complete", "{summary}");
        }
    }
    assert_eq!(Layout::opened_sessions(&first_engine)?, 11);
    Ok(())
}

/// The child of [`a_kill_mid_batch_then_a_rerun_matches_the_control`]: runs
/// the batch of the layout at `VSIFT_BATCH_TEST_ROOT`, stopping at the point
/// `VSIFT_FAULT_POINT` names.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "internal child entry launched by the batch kill test"]
async fn worker_batch_child() -> TestResult {
    let layout = Layout::at(PathBuf::from(
        env::var_os(CHILD_ROOT).ok_or("missing child root")?,
    ));
    let (summary, _) = run_collecting(&layout.engine(), &layout, 2, bounded(30_000)?).await?;
    // Keep the layout: the parent owns and removes it.
    std::mem::forget(layout);
    Err(format!(
        "the fault point was not reached: {:?}",
        summary.map(|s| s.termination())
    )
    .into())
}

/// A result with what may differ between a control and a continued run
/// removed: timings, identities and the attempt.
fn shape(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(steps) = value["steps"].as_array_mut() {
        for step in steps.iter_mut() {
            step["elapsed_ms"] = Value::from(0);
            step["admission_wait_ms"] = Value::from(0);
            if step["kind"] == "retain" {
                step["outputs"]["bundle_sha256"] = Value::Null;
            }
        }
    }
    for member in [
        "attempt",
        "session_id",
        "source_id",
        "lifecycle",
        "replayed",
    ] {
        value[member] = Value::Null;
    }
    value
}

/// S-07 for batches: a process killed at each request fault point in the
/// middle of a batch leaves a workspace in which a rerun of the same file
/// completes every line with the result of an uninterrupted control, one
/// session per operation id.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_kill_mid_batch_then_a_rerun_matches_the_control() -> TestResult {
    let lines: Vec<String> = (1..=4).map(retain_and_close).collect();
    let control_layout = Layout::new()?;
    let control_engine = control_layout.init(2)?;
    control_layout.write_batch(&lines)?;
    let (control, control_events) =
        run_collecting(&control_engine, &control_layout, 2, bounded(30_000)?).await?;
    assert_eq!(control?.outcome(), BatchOutcome::Success);

    for point in [
        "request-accept",
        "request-step:1",
        "request-step:2",
        "request-step:3",
        "request-complete",
    ] {
        let layout = Layout::new()?;
        let engine = layout.init(2)?;
        layout.write_batch(&lines)?;
        let output = Command::new(env::current_exe()?)
            .args(["--exact", "worker_batch_child", "--ignored", "--nocapture"])
            .env(CHILD_ROOT, &layout.0)
            .env(FAULT_POINT, point)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(FAULT_EXIT_CODE),
            "{point}: {stderr}"
        );

        let (rerun, events) = run_collecting(&engine, &layout, 2, bounded(30_000)?).await?;
        let rerun = rerun?;
        assert_eq!(rerun.outcome(), BatchOutcome::Success, "{point}");
        for line in 1..=4 {
            let expected = result_of(&control_events, line)?.ok_or("no control result")?;
            let continued = result_of(&events, line)?.ok_or("no rerun result")?;
            assert_eq!(shape(&continued), shape(&expected), "{point} line {line}");
        }
        assert_eq!(Layout::opened_sessions(&engine)?, 4, "{point}");
    }
    Ok(())
}
