//! Worker requests through the engine (P11 PR 3, ADR 0021 sections 2, 4
//! and 6): `Engine::run_work_request` in an ephemeral worker workspace.
//!
//! Everything here runs without `FFmpeg`: the requests ingest a placeholder
//! source (a plain ingest runs no provider), retain it and close it. They
//! cover the request record's table (replay, conflict, busy, continue), the
//! refusals that need no write, contained inputs, the X-09 retry and
//! deadline rules, a shutdown between steps, and S-07: a process killed at
//! each request fault point (`request-accept`, `request-step`,
//! `request-complete`) leaves a request that the next delivery completes,
//! with one session and the same result as an uninterrupted control. The
//! kill test drives this test binary as its child, with the infrastructure's
//! `fault-injection` feature (a development dependency only).

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    num::NonZeroU16,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift::{
    AdmissionBudget, AdmissionWait, Cancellation, DurabilityRequirement, Engine, EngineConfig,
    EngineError, EnginePorts, HostIsolation, JobProgress, ProgressObserver, ProgressStage,
    SessionListEntry, SessionRootLocation, UserConfigurationLocation, WorkOutcome, WorkRequestRun,
    WorkerFailure, WorkspaceInitRequest, WorkspacePolicy, WorkspaceRetention,
};
use vsift_contract::{WorkRequest, decode_work_request};
use vsift_domain::OperationId;
use vsift_infrastructure::{FilesystemSessionStore, WorkerRequestClaim};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-engine-worker-test-";
const SOURCE: &[u8] = b"\0\0\0\x18ftypisomengine-worker-source";
const OPERATION: &str = "op_7c2d4e6f8a0b1c3d5e7f9a1b3c5d7e9f";
const CHILD_ROOT: &str = "VSIFT_WORKER_TEST_ROOT";
const CHILD_REQUEST: &str = "VSIFT_WORKER_TEST_REQUEST";
const FAULT_POINT: &str = "VSIFT_FAULT_POINT";
const FAULT_EXIT_CODE: i32 = 91;
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

/// A temporary directory holding the workspace, the input root and the
/// bundle root; removed with everything in it.
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

    fn engine(&self) -> Engine {
        Engine::new(
            EngineConfig {
                session_root: SessionRootLocation::Explicit(self.workspace()),
                user_configuration: UserConfigurationLocation::Explicit(self.0.join("config")),
                host_isolation: HostIsolation::ProcessOnly,
            },
            EnginePorts::system(),
        )
    }

    /// Creates the ephemeral workspace with `capacity` admission units.
    fn init(&self, capacity: u16) -> Built<Engine> {
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

    fn run(&self, request: &WorkRequest) -> WorkRequestRun {
        WorkRequestRun {
            request: request.clone(),
            input_root: self.inputs(),
            bundle_root: Some(self.bundles()),
            admission: AdmissionWait::Immediate,
            concurrency: NonZeroU16::MIN,
            stop: Cancellation::new(),
            cancellation: Cancellation::new(),
            progress: ProgressObserver::none(),
        }
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

/// An ingest of `videos/talk.mp4`, then `steps` (JSON members of `steps`).
fn ingest_request(operation: &str, steps: &str, extra: &str) -> Built<WorkRequest> {
    let text = format!(
        r#"{{"schema_version":"1","operation_id":"{operation}","durability":"ephemeral",{extra}"target":{{"ingest":{{"source":"videos/talk.mp4","transcript":null}}}},"steps":[{steps}]}}"#
    );
    Ok(decode_work_request(text.as_bytes())?)
}

fn retain_and_close(bundle: &str) -> String {
    format!(r#"{{"retain":{{"bundle_name":"{bundle}","include_source":false}}}},{{"close":{{}}}}"#)
}

fn data(outcome: &WorkOutcome) -> Built<Value> {
    Ok(serde_json::to_value(outcome.result())?)
}

fn statuses(value: &Value) -> Vec<(String, String)> {
    value["steps"]
        .as_array()
        .map(|steps| {
            steps
                .iter()
                .map(|step| {
                    (
                        step["kind"].as_str().unwrap_or_default().to_owned(),
                        step["status"].as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
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

/// Every result conforms to the published job-result schema.
fn conforms(value: &Value) -> TestResult {
    let schema: Value = serde_json::from_str(&fs::read_to_string(
        schema_root().join("job-result.schema.json"),
    )?)?;
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&schema)?
        .validate(value)
        .map_err(|error| std::io::Error::other(format!("job-result: {error}")))?;
    Ok(())
}

/// The result with its timings removed: what an uninterrupted control and a
/// continued run must agree on besides identities.
fn shape(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(steps) = value["steps"].as_array_mut() {
        for step in steps {
            step["elapsed_ms"] = Value::from(0);
            step["admission_wait_ms"] = Value::from(0);
        }
    }
    for member in ["attempt", "session_id", "source_id", "lifecycle"] {
        value[member] = Value::Null;
    }
    // A bundle's manifest names its session.
    if let Some(steps) = value["steps"].as_array_mut() {
        for step in steps.iter_mut().filter(|step| step["kind"] == "retain") {
            step["outputs"]["bundle_sha256"] = Value::Null;
        }
    }
    value
}

/// A request that ingests, retains and closes completes, and every
/// redelivery returns the recorded result (`replayed: true`) with no new
/// work: one session, one bundle.
#[tokio::test]
async fn a_request_runs_once_and_is_replayed() -> TestResult {
    let layout = Layout::new()?;
    let engine = layout.init(2)?;
    let request = ingest_request(OPERATION, &retain_and_close("talk-review"), "")?;
    let first = engine.run_work_request(layout.run(&request)).await;
    let value = data(&first)?;
    conforms(&value)?;
    assert_eq!(value["status"], "complete", "{value}");
    assert_eq!(value["replayed"], false);
    assert_eq!(value["attempt"], 1);
    assert_eq!(
        statuses(&value),
        [
            ("ingest".to_owned(), "complete".to_owned()),
            ("retain".to_owned(), "complete".to_owned()),
            ("close".to_owned(), "complete".to_owned())
        ]
    );
    assert_eq!(value["steps"][0]["outputs"]["generation"], 1);
    assert_eq!(value["steps"][1]["outputs"]["bundle_name"], "talk-review");
    assert_eq!(value["lifecycle"]["mode"], "durable_worker");
    assert_eq!(value["publication"], "process_crash_consistent");
    let bundle = engine.validate_bundle(&layout.bundles().join("talk-review"))?;
    assert_eq!(
        value["steps"][1]["outputs"]["bundle_sha256"],
        bundle.manifest_sha256()
    );

    for _ in 0..2 {
        let again = engine.run_work_request(layout.run(&request)).await;
        let replayed = data(&again)?;
        conforms(&replayed)?;
        let mut expected = value.clone();
        expected["replayed"] = Value::Bool(true);
        assert_eq!(replayed, expected);
    }
    assert_eq!(Layout::opened_sessions(&engine)?, 1);
    Ok(())
}

/// The same operation id with another request is `IDEMPOTENCY_CONFLICT`
/// and changes nothing; a malformed spacing of the same request is not.
#[tokio::test]
async fn another_request_under_the_same_id_is_a_conflict() -> TestResult {
    let layout = Layout::new()?;
    let engine = layout.init(2)?;
    let request = ingest_request(OPERATION, &retain_and_close("first"), "")?;
    let first = engine.run_work_request(layout.run(&request)).await;
    assert_eq!(data(&first)?["status"], "complete");

    let other = ingest_request(OPERATION, &retain_and_close("second"), "")?;
    let conflict = engine.run_work_request(layout.run(&other)).await;
    let value = data(&conflict)?;
    conforms(&value)?;
    assert_eq!(value["status"], "failed");
    assert_eq!(value["failure"]["code"], "IDEMPOTENCY_CONFLICT");
    assert_eq!(value["steps"], Value::Array(Vec::new()));
    assert!(matches!(
        conflict.cause(),
        Some(EngineError::Worker(WorkerFailure::Conflict))
    ));
    assert!(!layout.bundles().join("second").exists());

    // Spacing and member order do not change the digest: a replay.
    let spaced = decode_work_request(
        format!(
            r#"{{ "steps": [{}], "target": {{"ingest": {{"transcript": null, "source": "videos/talk.mp4"}}}}, "durability": "ephemeral", "deadline_ms": null, "operation_id": "{OPERATION}", "schema_version": "1" }}"#,
            retain_and_close("first")
        )
        .as_bytes(),
    )?;
    let replayed = engine.run_work_request(layout.run(&spaced)).await;
    assert_eq!(data(&replayed)?["replayed"], true);
    assert_eq!(Layout::opened_sessions(&engine)?, 1);
    Ok(())
}

/// A request another process holds is `BUSY` with the 2 s hint; with
/// another digest it is a conflict even while held.
#[tokio::test]
async fn a_request_held_elsewhere_is_busy() -> TestResult {
    let layout = Layout::new()?;
    let engine = layout.init(2)?;
    let request = ingest_request(OPERATION, &retain_and_close("held"), "")?;
    let store = layout.store()?;
    let held = match store.claim_worker_request(&OperationId::parse(OPERATION)?)? {
        WorkerRequestClaim::Owned(owner) => owner,
        WorkerRequestClaim::HeldElsewhere(_) => return Err("not claimed".into()),
    };
    let busy = engine.run_work_request(layout.run(&request)).await;
    let value = data(&busy)?;
    conforms(&value)?;
    assert_eq!(value["failure"]["code"], "BUSY");
    assert_eq!(value["failure"]["retry_after_ms"], 2_000);
    assert_eq!(Layout::opened_sessions(&engine)?, 0);
    drop(held);

    let ran = engine.run_work_request(layout.run(&request)).await;
    assert_eq!(data(&ran)?["status"], "complete");
    Ok(())
}

/// D4: a shutdown between steps stops the request before its next step,
/// `cancelled` and resumable; the next delivery continues from that step
/// (attempt 2) in the same session, and nothing is done twice.
#[tokio::test]
async fn a_stopped_request_continues_from_its_next_step() -> TestResult {
    let layout = Layout::new()?;
    let engine = layout.init(2)?;
    let request = ingest_request(OPERATION, &retain_and_close("resumed"), "")?;
    let stop = Cancellation::new();
    let trigger = stop.clone();
    let mut run = layout.run(&request);
    run.stop = stop;
    // Stop once the ingest has finished.
    run.progress = ProgressObserver::new(move |progress: &JobProgress| {
        if progress.update.stage == ProgressStage::RunningRequest && progress.update.completed == 1
        {
            trigger.cancel();
        }
    });
    let stopped = engine.run_work_request(run).await;
    let value = data(&stopped)?;
    conforms(&value)?;
    assert_eq!(value["status"], "cancelled", "{value}");
    assert!(stopped.stopped_by_shutdown());
    assert_eq!(
        statuses(&value),
        [
            ("ingest".to_owned(), "complete".to_owned()),
            ("retain".to_owned(), "cancelled".to_owned()),
            ("close".to_owned(), "not_started".to_owned())
        ]
    );
    let session = value["session_id"].clone();

    let continued = engine.run_work_request(layout.run(&request)).await;
    let value = data(&continued)?;
    assert_eq!(value["status"], "complete", "{value}");
    assert_eq!(value["attempt"], 2);
    assert_eq!(value["session_id"], session);
    assert_eq!(Layout::opened_sessions(&engine)?, 1);
    Ok(())
}

/// Refusals that need no write leave no record and no session: a durable
/// request in an ephemeral workspace, a missing input, a hard link out of
/// the input root, a retain without a bundle root, an unknown session, and
/// a root that is not a worker workspace.
#[tokio::test]
async fn refusals_write_nothing() -> TestResult {
    let layout = Layout::new()?;
    let engine = layout.init(2)?;
    let operation = OperationId::parse(OPERATION)?;
    let store = layout.store()?;

    let durable = decode_work_request(
        format!(
            r#"{{"schema_version":"1","operation_id":"{OPERATION}","durability":"durable","target":{{"ingest":{{"source":"videos/talk.mp4","transcript":null}}}},"steps":[]}}"#
        )
        .as_bytes(),
    )?;
    let refused = engine.run_work_request(layout.run(&durable)).await;
    let value = data(&refused)?;
    conforms(&value)?;
    assert_eq!(value["failure"]["rejection"], "workspace_not_durable");
    assert_eq!(value["failure"]["code"], "INVALID_ARGUMENT");

    let missing = decode_work_request(
        format!(
            r#"{{"schema_version":"1","operation_id":"{OPERATION}","durability":"ephemeral","target":{{"ingest":{{"source":"videos/missing.mp4","transcript":null}}}},"steps":[]}}"#
        )
        .as_bytes(),
    )?;
    let value = data(&engine.run_work_request(layout.run(&missing)).await)?;
    assert_eq!(value["failure"]["code"], "INVALID_ARGUMENT");

    // S-02: a hard link to a file outside the root has two links.
    fs::write(layout.0.join("outside.mp4"), SOURCE)?;
    fs::hard_link(
        layout.0.join("outside.mp4"),
        layout.inputs().join("videos/linked.mp4"),
    )?;
    let linked = decode_work_request(
        format!(
            r#"{{"schema_version":"1","operation_id":"{OPERATION}","durability":"ephemeral","target":{{"ingest":{{"source":"videos/linked.mp4","transcript":null}}}},"steps":[]}}"#
        )
        .as_bytes(),
    )?;
    let value = data(&engine.run_work_request(layout.run(&linked)).await)?;
    assert_eq!(value["failure"]["code"], "INVALID_SOURCE");

    let request = ingest_request(OPERATION, &retain_and_close("nowhere"), "")?;
    let mut run = layout.run(&request);
    run.bundle_root = None;
    let value = data(&engine.run_work_request(run).await)?;
    assert_eq!(value["failure"]["code"], "INVALID_ARGUMENT");

    let unknown = decode_work_request(
        format!(
            r#"{{"schema_version":"1","operation_id":"{OPERATION}","durability":"ephemeral","target":{{"session_id":"ses_0123456789abcdef0123456789abcdef"}},"steps":[{{"close":{{}}}}]}}"#
        )
        .as_bytes(),
    )?;
    let value = data(&engine.run_work_request(layout.run(&unknown)).await)?;
    assert_eq!(value["failure"]["code"], "INVALID_ARGUMENT");

    assert_eq!(store.read_worker_request(&operation)?, None);
    assert_eq!(Layout::opened_sessions(&engine)?, 0);

    // A desktop root runs no request.
    let desktop = Layout::new()?;
    FilesystemSessionStore::provision_default(desktop.workspace())?;
    let engine = desktop.engine();
    let request = ingest_request(OPERATION, "", "")?;
    let refused = engine.run_work_request(desktop.run(&request)).await;
    assert!(matches!(
        refused.cause(),
        Some(EngineError::Worker(WorkerFailure::WorkspaceRequired))
    ));
    Ok(())
}

/// S-02 on Unix: a symbolic link out of the input root is the contract's
/// `path_outside_input_root` refusal.
#[cfg(unix)]
#[tokio::test]
async fn a_link_out_of_the_input_root_is_refused() -> TestResult {
    let layout = Layout::new()?;
    let engine = layout.init(2)?;
    fs::write(layout.0.join("secret.mp4"), SOURCE)?;
    std::os::unix::fs::symlink(
        layout.0.join("secret.mp4"),
        layout.inputs().join("videos/secret.mp4"),
    )?;
    let request = decode_work_request(
        format!(
            r#"{{"schema_version":"1","operation_id":"{OPERATION}","durability":"ephemeral","target":{{"ingest":{{"source":"videos/secret.mp4","transcript":null}}}},"steps":[]}}"#
        )
        .as_bytes(),
    )?;
    let value = data(&engine.run_work_request(layout.run(&request)).await)?;
    conforms(&value)?;
    assert_eq!(value["failure"]["rejection"], "path_outside_input_root");
    assert_eq!(Layout::opened_sessions(&engine)?, 0);
    Ok(())
}

/// X-09: a busy admission is retried with jitter until it frees, within
/// the admission wait; the wait is reported. While it stays busy past the
/// wait, the request fails `BUSY` with the 2 s hint and stays resumable.
#[tokio::test]
async fn busy_admission_is_retried_within_the_wait() -> TestResult {
    let layout = Layout::new()?;
    let engine = layout.init(1)?;
    let store = layout.store()?;
    let request = ingest_request(OPERATION, r#"{"close":{}}"#, "")?;

    let permit = store.try_admit(1)?;
    let mut run = layout.run(&request);
    run.admission = AdmissionWait::Bounded(AdmissionBudget::new(Duration::from_millis(400))?);
    let busy = engine.run_work_request(run).await;
    let value = data(&busy)?;
    conforms(&value)?;
    assert_eq!(value["status"], "failed", "{value}");
    assert_eq!(value["steps"][0]["failure"]["code"], "BUSY");
    assert_eq!(value["failure"]["retry_after_ms"], 2_000);
    assert!(value["steps"][0]["admission_wait_ms"].as_u64() >= Some(300));
    assert_eq!(Layout::opened_sessions(&engine)?, 0);

    // Freed while it waits: the same request continues and completes. The
    // slot is released from the engine's admission-waiting callback, so the
    // request is known to be waiting first; a fixed sleep raced a slow
    // runner's setup and let the request find the slot already free (#190).
    // A fallback releases it after 5 s should the callback never fire.
    let slot = Arc::new(Mutex::new(Some(permit)));
    let release = {
        let slot = Arc::clone(&slot);
        move || {
            if let Ok(mut held) = slot.lock() {
                drop(held.take());
            }
        }
    };
    let fallback = {
        let release = release.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(5));
            release();
        })
    };
    let mut run = layout.run(&request);
    run.admission = AdmissionWait::Bounded(AdmissionBudget::new(Duration::from_secs(20))?);
    run.progress = ProgressObserver::none().with_admission_waiting(move |_| release());
    let ran = engine.run_work_request(run).await;
    fallback.join().map_err(|_| "fallback releaser panicked")?;
    let value = data(&ran)?;
    assert_eq!(value["status"], "complete", "{value}");
    assert_eq!(value["attempt"], 2);
    assert!(value["steps"][0]["admission_wait_ms"].as_u64() >= Some(1));
    assert_eq!(Layout::opened_sessions(&engine)?, 1);
    Ok(())
}

/// X-09: a deadline too short for a step is `DEADLINE_EXCEEDED` before the
/// step starts; a busy step is not retried past the deadline; a permanent
/// failure is not retried at all and ends the request (replayed after).
#[tokio::test]
async fn deadlines_and_permanent_failures_are_not_retried() -> TestResult {
    let layout = Layout::new()?;
    let engine = layout.init(1)?;
    let short = ingest_request(OPERATION, "", r#""deadline_ms":500,"#)?;
    let started = Instant::now();
    let value = data(&engine.run_work_request(layout.run(&short)).await)?;
    conforms(&value)?;
    assert_eq!(value["failure"]["code"], "DEADLINE_EXCEEDED", "{value}");
    assert_eq!(value["steps"][0]["status"], "failed");
    assert!(started.elapsed() < Duration::from_secs(5));

    let store = layout.store()?;
    let permit = store.try_admit(1)?;
    let bounded = ingest_request(
        "op_1111111111111111111111111111111a",
        "",
        r#""deadline_ms":1600,"#,
    )?;
    let mut run = layout.run(&bounded);
    run.admission = AdmissionWait::Bounded(AdmissionBudget::MAX);
    let started = Instant::now();
    let value = data(&engine.run_work_request(run).await)?;
    assert_eq!(value["failure"]["code"], "DEADLINE_EXCEEDED", "{value}");
    assert!(started.elapsed() < Duration::from_secs(10));
    drop(permit);

    fs::write(layout.inputs().join("videos/notes.txt"), b"not a video")?;
    let invalid = decode_work_request(
        br#"{"schema_version":"1","operation_id":"op_2222222222222222222222222222222b","durability":"ephemeral","target":{"ingest":{"source":"videos/notes.txt","transcript":null}},"steps":[{"close":{}}]}"#,
    )?;
    let mut run = layout.run(&invalid);
    run.admission = AdmissionWait::Bounded(AdmissionBudget::MAX);
    let started = Instant::now();
    let failed = engine.run_work_request(run).await;
    let value = data(&failed)?;
    assert_eq!(value["failure"]["code"], "INVALID_SOURCE", "{value}");
    assert_eq!(value["steps"][0]["admission_wait_ms"], 0);
    assert!(started.elapsed() < Duration::from_secs(10));
    let replayed = data(&engine.run_work_request(layout.run(&invalid)).await)?;
    assert_eq!(replayed["replayed"], true);
    assert_eq!(replayed["failure"]["code"], "INVALID_SOURCE");
    Ok(())
}

/// A retain that finds its bundle already there (a try that stopped after
/// retaining) accepts it when it is this session's; a directory that is
/// not is refused.
#[tokio::test]
async fn an_existing_bundle_is_accepted_only_when_it_is_the_requests() -> TestResult {
    let layout = Layout::new()?;
    let engine = layout.init(2)?;
    let first = ingest_request(OPERATION, &retain_and_close("taken"), "")?;
    assert_eq!(
        data(&engine.run_work_request(layout.run(&first)).await)?["status"],
        "complete"
    );
    // Another request retaining into the same name meets a bundle of
    // another session.
    let other = ingest_request(
        "op_3333333333333333333333333333333c",
        &retain_and_close("taken"),
        "",
    )?;
    let value = data(&engine.run_work_request(layout.run(&other)).await)?;
    conforms(&value)?;
    assert_eq!(value["status"], "failed");
    assert_eq!(value["steps"][1]["failure"]["code"], "INVALID_ARGUMENT");
    Ok(())
}

/// The child of [`a_kill_at_every_request_fault_point_recovers`]: runs the
/// request in `VSIFT_WORKER_TEST_REQUEST` in the layout at
/// `VSIFT_WORKER_TEST_ROOT`, stopping at the point `VSIFT_FAULT_POINT` names.
#[tokio::test]
#[ignore = "internal child entry launched by the request kill test"]
async fn worker_request_child() -> TestResult {
    let layout = Layout::at(PathBuf::from(
        env::var_os(CHILD_ROOT).ok_or("missing child root")?,
    ));
    let request = decode_work_request(
        env::var(CHILD_REQUEST)
            .map_err(|_| "missing child request")?
            .as_bytes(),
    )?;
    let outcome = layout.engine().run_work_request(layout.run(&request)).await;
    // Keep the layout: the parent owns and removes it.
    std::mem::forget(layout);
    Err(format!(
        "the fault point was not reached: {:?}",
        outcome.result().status()
    )
    .into())
}

/// S-07 for worker requests: a process killed at each request fault point
/// (after the attempt is accepted, after each step, after the result is
/// recorded) leaves a request the next delivery completes: one session, the
/// result of an uninterrupted control, and the same recorded result for
/// every delivery after.
#[tokio::test]
async fn a_kill_at_every_request_fault_point_recovers() -> TestResult {
    let steps = retain_and_close("killed");
    let text = format!(
        r#"{{"schema_version":"1","operation_id":"{OPERATION}","durability":"ephemeral","target":{{"ingest":{{"source":"videos/talk.mp4","transcript":null}}}},"steps":[{steps}]}}"#
    );
    let request = decode_work_request(text.as_bytes())?;
    let control_layout = Layout::new()?;
    let control_engine = control_layout.init(2)?;
    let control = shape(&data(
        &control_engine
            .run_work_request(control_layout.run(&request))
            .await,
    )?);
    assert_eq!(control["status"], "complete");

    for point in [
        "request-accept",
        "request-step:1",
        "request-step:2",
        "request-step:3",
        "request-complete",
    ] {
        let layout = Layout::new()?;
        let engine = layout.init(2)?;
        let output = Command::new(env::current_exe()?)
            .args([
                "--exact",
                "worker_request_child",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD_ROOT, &layout.0)
            .env(CHILD_REQUEST, &text)
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

        let rerun = data(&engine.run_work_request(layout.run(&request)).await)?;
        conforms(&rerun)?;
        assert_eq!(rerun["status"], "complete", "{point}: {rerun}");
        let mut expected = control.clone();
        if point == "request-complete" {
            expected["replayed"] = Value::Bool(true);
        }
        assert_eq!(shape(&rerun), expected, "{point}");
        assert_eq!(Layout::opened_sessions(&engine)?, 1, "{point}");
        // Every later delivery replays the same recorded result.
        let first = data(&engine.run_work_request(layout.run(&request)).await)?;
        let second = data(&engine.run_work_request(layout.run(&request)).await)?;
        assert_eq!(first, second, "{point}");
        assert_eq!(first["replayed"], true);
        let mut fresh = first.clone();
        fresh["replayed"] = Value::Bool(false);
        if point != "request-complete" {
            assert_eq!(fresh, rerun, "{point}");
        }
    }
    Ok(())
}
