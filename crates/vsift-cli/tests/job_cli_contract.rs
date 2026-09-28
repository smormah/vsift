//! Public CLI contract for the job commands (P10 PR 3, ADR 0020 section 6):
//! `job status`, `job resume` and `job cancel`, the `jobs` of
//! `session status`, and `transcript retranscribe --operation-id`.
//!
//! Every test runs everywhere, without `FFmpeg` or whisper.cpp: jobs are put
//! in each state through the application's own use case over the real
//! session store, with deterministic stand-in audio and recognition, and the
//! binary then reads, cancels or resumes them. A job the test process holds
//! stands for a live owner in another process (X-06 in its admitting,
//! running and committing phases). The owner's side of a cancellation (its
//! watcher stopping the provider) is covered in `vsift-infrastructure`; real
//! runs interrupted by Ctrl-C, `SIGTERM` and `job cancel` are in the opt-in
//! `p10_recovery_e2e`.

use std::{
    env,
    error::Error,
    fs,
    future::Future,
    io,
    num::NonZeroU16,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::{Value, json};
use vsift::{
    AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider, AsrProviderBuild, ChunkPlan,
    ChunkTime, CueText, DurabilityRequirement, FailureCode, JobId, LanguageTag, MediaTime,
    OperationId, PlannedChunk, ProviderChunkOutput, ProviderSegment, ProviderToken,
    ProviderTokenKind, RecognizerIdentity, SessionId, Sha256Hex, SpeechPcm, SpeechRecognitionError,
    SpeechRecognizer, StorageGeneration, TimeRange, TranscriptRevisionId,
};
use vsift_application::{
    AsrCancellation, CommitGuard, InitializeSessionStorage, InitializeSessionStorageRequest,
    JobChange, JobCommit, JobOwner, JobRequest, JobSpec, JobStore, RecognitionScope,
    RetranscriptionPorts, RetranscriptionRun, RetryTimer, RevisionStore, SessionStorageError,
    SpeechAudioError, SpeechAudioSource, TranscribeRangeRequest, commit_operation_id, job_id,
    recognition_key, retranscribe_operation_key, retranscribe_request_digest,
    retranscription_range, run_retranscription, whole_file_source_segment,
};
use vsift_domain::{Jitter, TranscriptRevision, plan_chunks};
use vsift_infrastructure::{FilesystemSessionStore, SourceSnapshot};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-job-cli-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const PLACEHOLDER_SOURCE: &[u8] = b"\0\0\0\x18ftypisomjob-cli-source";
const OPERATION: &str = "op_0123456789abcdef0123456789abcdef";
const UNKNOWN_JOB: &str = "job_0123456789abcdef0123456789abcdef";
const SECOND: u64 = 1_000_000;
/// The stand-in source: three R0 chunks (0-30, 25-55 and 50-60 s).
const SOURCE_MICROS: u64 = 60 * SECOND;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Built<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self, child: &str) -> PathBuf {
        self.0.join(child)
    }

    fn sessions(&self) -> PathBuf {
        self.path("sessions")
    }
}

impl Drop for OwnedRoot {
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

/// Runs `vsift` with an isolated per-user base, the root's session store
/// and no tools on `PATH`.
fn vsift(root: &OwnedRoot, arguments: &[&str]) -> Built<Output> {
    let base = root.path("user");
    Ok(Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .env("PATH", "")
        .arg("--session-root")
        .arg(root.sessions())
        .args(arguments)
        .output()?)
}

/// One schema-valid JSON result.
fn json(output: &Output) -> Built<Value> {
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    let value: Value = serde_json::from_slice(&output.stdout)?;
    validate("operation-response.schema.json", &value)?;
    Ok(value)
}

/// A complete job result whose data is schema-valid job data.
fn job_result(output: &Output, command: &str) -> Built<Value> {
    let value = json(output)?;
    assert_eq!(output.status.code(), Some(0), "{value}");
    assert_eq!(value["command"], command);
    assert_eq!(value["status"], "complete");
    validate("job-data.schema.json", &value["data"])?;
    Ok(value)
}

fn now() -> Built<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn block_on<T>(future: impl Future<Output = T>) -> Built<T> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?
        .block_on(future))
}

/// Opens one session over a placeholder source through the store, as
/// `ingest` would, without any provider.
fn seed_session(root: &OwnedRoot) -> Built<SessionId> {
    let source = root.path("placeholder.mp4");
    fs::write(&source, PLACEHOLDER_SOURCE)?;
    let now = now()?;
    let session = SessionId::parse(SESSION)?;
    let opener = OperationId::parse("op_0123456789abcdef")?;
    let store = FilesystemSessionStore::provision_default(root.sessions())?;
    let registration = store.register_session(&session, &opener, now)?;
    block_on(
        InitializeSessionStorage::new(store).execute(InitializeSessionStorageRequest::new(
            session.clone(),
            opener,
            DurabilityRequirement::Ephemeral,
        )),
    )??;
    drop(registration);
    let store = FilesystemSessionStore::open_existing(root.sessions())?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session,
        &OperationId::parse("op_1111111111111111")?,
        &source,
    )?;
    store.activate_source(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        now,
    )?;
    Ok(session)
}

fn identity() -> Result<RecognizerIdentity, SpeechRecognitionError> {
    let digest = Sha256Hex::parse(DIGEST).map_err(|_| SpeechRecognitionError::Io)?;
    Ok(RecognizerIdentity {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, digest.clone()),
        model: AsrModel::new(AsrModelProfile::Base, digest),
        decoding: AsrDecodingProfile::R0V1,
        threads: NonZeroU16::MIN,
    })
}

/// One segment per chunk, inside every window.
fn words(chunk: &PlannedChunk) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    let text = format!("words of chunk {}", chunk.index());
    let text = CueText::new(text.clone(), text).map_err(|_| SpeechRecognitionError::Io)?;
    let third = chunk.window().duration_micros() / 3_000;
    let start = if chunk.index() == 0 { 10_000 } else { 6_000 }.min(third);
    let length = 2_000.min(third);
    Ok(ProviderChunkOutput {
        language: LanguageTag::parse("en").ok(),
        segments: vec![ProviderSegment {
            start: ChunkTime::from_millis(start).ok_or(SpeechRecognitionError::Io)?,
            end: ChunkTime::from_millis(start + length).ok_or(SpeechRecognitionError::Io)?,
            text: Some(text),
            tokens: vec![ProviderToken {
                kind: ProviderTokenKind::Text,
                probability: 0.5,
            }],
        }],
    })
}

/// A deterministic recognizer that fails with a deadline at one chunk.
struct Recognizer {
    fails_at: Option<u32>,
}

impl SpeechRecognizer for Recognizer {
    fn identity(
        &self,
    ) -> impl Future<Output = Result<RecognizerIdentity, SpeechRecognitionError>> + Send {
        std::future::ready(identity())
    }

    fn recognize(
        &self,
        chunk: &PlannedChunk,
        _pcm: &SpeechPcm,
    ) -> impl Future<Output = Result<ProviderChunkOutput, SpeechRecognitionError>> + Send {
        std::future::ready(if self.fails_at == Some(chunk.index()) {
            Err(SpeechRecognitionError::Deadline)
        } else {
            words(chunk)
        })
    }
}

/// Speech in every chunk.
struct Audio;

impl SpeechAudioSource for Audio {
    fn speech_pcm(
        &self,
        chunk: &PlannedChunk,
    ) -> impl Future<Output = Result<SpeechPcm, SpeechAudioError>> + Send {
        let samples = usize::try_from(chunk.window().duration_micros() / 1_000 * 16).unwrap_or(0);
        std::future::ready(Ok(SpeechPcm {
            actual_start: chunk.window().start(),
            samples: vec![3_000; samples],
        }))
    }
}

struct Never;

impl AsrCancellation for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

struct Instant0;

impl RetryTimer for Instant0 {
    fn jitter(&self) -> Jitter {
        Jitter::NONE
    }

    fn sleep(&self, _delay: Duration) -> impl Future<Output = ()> + Send {
        std::future::ready(())
    }
}

struct Unchanged;

impl CommitGuard for Unchanged {
    fn verify(&mut self) -> Result<(), SessionStorageError> {
        Ok(())
    }
}

/// A retranscription request resolved against the session's head, as the
/// engine resolves it.
struct Resolved {
    spec: JobSpec,
    segment: vsift::SourceSegment,
    replaced: TimeRange,
    base: Option<TranscriptRevision>,
    observed: StorageGeneration,
    requested: Option<TimeRange>,
}

fn resolve(
    store: &FilesystemSessionStore,
    session: &SessionId,
    requested: Option<TimeRange>,
) -> Built<Resolved> {
    let now = now()?;
    let status = store.session_status(session)?;
    let segment =
        whole_file_source_segment(status.source_id(), MediaTime::from_micros(SOURCE_MICROS))?;
    let head = store.head(session, now)?;
    let replaced = retranscription_range(head.newest.as_ref(), requested, segment.range());
    let identity = identity().map_err(|_| "no identity")?;
    let key = recognition_key(&RecognitionScope {
        session_id: session,
        source_id: status.source_id(),
        audio_stream: 1,
        replaced_range: replaced,
        plan: ChunkPlan::R0,
        recognizer: &identity,
        verification: None,
    })?;
    let operation_key =
        retranscribe_operation_key(&key, head.newest.as_ref().map(TranscriptRevision::id))?;
    let planned = plan_chunks(segment.id(), replaced, ChunkPlan::R0)?;
    Ok(Resolved {
        spec: JobSpec {
            session_id: session.clone(),
            job_id: job_id(session, &operation_key)?,
            request_digest: retranscribe_request_digest(session, requested)?,
            operation_key,
            recognition_key: key,
            request: JobRequest::Retranscribe { range: requested },
            planned_chunks: Some(u32::try_from(planned.len())?),
        },
        segment,
        replaced,
        base: head.newest,
        observed: head.generation,
        requested,
    })
}

/// Runs a resolved request through the application's use case, as a
/// retranscription in another process would; returns whether it committed.
fn run(
    store: &FilesystemSessionStore,
    resolved: &Resolved,
    operation: Option<&OperationId>,
    fails_at: Option<u32>,
) -> Built<bool> {
    let status = store.session_status(&resolved.spec.session_id)?;
    let identity = identity().map_err(|_| "no identity")?;
    let outcome = block_on(run_retranscription(
        RetranscriptionRun {
            spec: &resolved.spec,
            operation_id: operation,
            transcribe: TranscribeRangeRequest {
                source_segment: &resolved.segment,
                range: resolved.replaced,
                plan: ChunkPlan::R0,
                audio_stream: 1,
                expected: &identity,
            },
            source_id: status.source_id(),
            requested: resolved.requested,
            base: resolved.base.as_ref(),
            observed: resolved.observed,
            now: now()?,
            admission: vsift::AdmissionWait::Immediate,
        },
        RetranscriptionPorts {
            store,
            audio: &Audio,
            recognizer: &Recognizer { fails_at },
            cancellation: &Never,
            timer: &Instant0,
            classify: |error| match error {
                vsift_application::JobRunError::Asr { .. } => FailureCode::DeadlineExceeded,
                _ => FailureCode::Internal,
            },
            progress: &vsift_application::NoProgress,
        },
        &mut Unchanged,
    ))?;
    Ok(outcome.is_ok())
}

fn range(from: u64, to: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(from),
        MediaTime::from_micros(to),
    )?)
}

fn store(root: &OwnedRoot) -> Built<FilesystemSessionStore> {
    Ok(FilesystemSessionStore::open_existing(root.sessions())?)
}

fn job_directory(root: &OwnedRoot, job: &JobId) -> PathBuf {
    root.sessions()
        .join("sessions")
        .join(SESSION)
        .join("jobs")
        .join(job.as_str())
}

fn checkpoint_files(root: &OwnedRoot, job: &JobId) -> Built<usize> {
    let chunks = job_directory(root, job).join("chunks");
    if !chunks.exists() {
        return Ok(0);
    }
    Ok(fs::read_dir(chunks)?.count())
}

fn assert_failure(output: &Output, command: &str, code: &str, exit: i32) -> Built<Value> {
    let value = json(output)?;
    assert_eq!(output.status.code(), Some(exit), "{value}");
    assert_eq!(value["command"], command);
    assert_eq!(value["error"]["code"], code, "{value}");
    Ok(value)
}

fn assert_no_path(value: &Value, root: &Path) {
    let text = value.to_string();
    let root = root.to_string_lossy();
    assert!(!text.contains(root.as_ref()), "a path leaked: {text}");
    assert!(
        !text.contains("words of chunk"),
        "transcript text leaked: {text}"
    );
}

// ------------------------------------------------------------ grammar

/// The job commands take exactly one job id and no session; malformed ids
/// and operation ids are parse failures before anything is read, as are a
/// `job run` and `job batch` without their input root or with their bounds
/// exceeded; a batch outside an existing workspace creates nothing.
#[test]
fn job_grammar_is_validated_before_any_io() -> TestResult {
    let root = OwnedRoot::new()?;
    for arguments in [
        vec!["job", "status", "--json"],
        vec!["job", "status", "job-not-an-id", "--json"],
        vec!["job", "cancel", SESSION, "--json"],
        vec!["job", "resume", UNKNOWN_JOB, SESSION, "--json"],
        vec![
            "transcript",
            "retranscribe",
            SESSION,
            "--operation-id",
            "op-not-an-id",
            "--json",
        ],
        vec![
            "transcript",
            "retranscribe",
            SESSION,
            "--operation-id",
            "op_SHOUTED0123456789AB",
            "--json",
        ],
        // `job batch` (P11 PR 4) needs the operator's input root, and
        // bounds its concurrency, drain and admission wait.
        vec!["job", "batch", "--requests", "requests.jsonl", "--json"],
        vec![
            "job",
            "batch",
            "--requests",
            "requests.jsonl",
            "--input-root",
            "inputs",
            "--concurrency",
            "0",
            "--json",
        ],
        vec![
            "job",
            "batch",
            "--requests",
            "requests.jsonl",
            "--input-root",
            "inputs",
            "--concurrency",
            "17",
            "--json",
        ],
        vec![
            "job",
            "batch",
            "--requests",
            "requests.jsonl",
            "--input-root",
            "inputs",
            "--drain-timeout-ms",
            "300001",
            "--json",
        ],
        // `job run` (P11 PR 3) needs the operator's input root, and bounds
        // its drain and admission wait.
        vec!["job", "run", "--request", "request.json", "--json"],
        vec![
            "job",
            "run",
            "--request",
            "request.json",
            "--input-root",
            "inputs",
            "--drain-timeout-ms",
            "300001",
            "--json",
        ],
        vec![
            "job",
            "run",
            "--request",
            "request.json",
            "--input-root",
            "inputs",
            "--admission-wait-ms",
            "60001",
            "--json",
        ],
    ] {
        let output = vsift(&root, &arguments)?;
        let value = assert_failure(&output, "parse", "INVALID_ARGUMENT", 2)?;
        assert!(output.stderr.is_empty(), "{arguments:?}");
        assert_eq!(value["status"], "failed");
    }
    // Nothing was created by a rejected command.
    assert!(!root.sessions().exists());
    Ok(())
}

/// `job batch` (P11 PR 4) runs only in an existing worker workspace: a
/// missing root is refused before the request file is opened, and nothing
/// is created.
#[test]
fn a_batch_outside_a_workspace_creates_nothing() -> TestResult {
    let root = OwnedRoot::new()?;
    let output = vsift(
        &root,
        &[
            "job",
            "batch",
            "--requests",
            "requests.jsonl",
            "--input-root",
            "inputs",
            "--json",
        ],
    )?;
    assert_failure(&output, "job.batch", "STORAGE_IO", 7)?;
    assert!(!root.sessions().exists());
    Ok(())
}

/// An unknown job is `INVALID_ARGUMENT` with a fixed remediation, with or
/// without a session root; `--events jsonl` gives the one terminal event.
#[test]
fn an_unknown_job_is_an_invalid_argument() -> TestResult {
    let root = OwnedRoot::new()?;
    for seeded in [false, true] {
        if seeded {
            seed_session(&root)?;
        }
        for (verb, command) in [
            ("status", "job.status"),
            ("cancel", "job.cancel"),
            ("resume", "job.resume"),
        ] {
            let output = vsift(&root, &["job", verb, UNKNOWN_JOB, "--json"])?;
            let value = assert_failure(&output, command, "INVALID_ARGUMENT", 2)?;
            let summary = value["error"]["remediation"][0]["summary"]
                .as_str()
                .ok_or("no remediation")?;
            assert!(summary.starts_with("No session of this session root holds a job"));
            assert_eq!(value["error"]["affected_ids"], json!([]));

            let output = vsift(&root, &["job", verb, UNKNOWN_JOB, "--events", "jsonl"])?;
            assert_eq!(output.status.code(), Some(2));
            let text = String::from_utf8(output.stdout)?;
            assert_eq!(text.lines().count(), 1);
            let event: Value = serde_json::from_str(text.trim_end())?;
            validate("terminal-event.schema.json", &event)?;
            assert_eq!(event["result"]["error"]["code"], "INVALID_ARGUMENT");
        }
    }
    Ok(())
}

// ------------------------------------------------------------ states

/// A succeeded job reports its revision, generation and operation id; a
/// cancel is too late (warning `cancellation_too_late`, idempotent); resume
/// refuses it; `session status` lists it. Nothing names a path or text.
#[test]
fn a_succeeded_job_reports_its_result_and_a_late_cancel_changes_nothing() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root)?;
    let operation = OperationId::parse(OPERATION)?;
    let store = store(&root)?;
    let resolved = resolve(&store, &session, Some(range(0, 20 * SECOND)?))?;
    assert!(run(&store, &resolved, Some(&operation), None)?);
    drop(store);
    let job = resolved.spec.job_id.clone();

    let status = job_result(
        &vsift(&root, &["job", "status", job.as_str(), "--json"])?,
        "job.status",
    )?;
    let data = &status["data"];
    assert_eq!(data["state"], "succeeded");
    assert_eq!(data["session_id"], SESSION);
    assert_eq!(data["kind"], "retranscribe");
    assert_eq!(data["live_owner"], false);
    assert_eq!(data["resumable"], false);
    assert_eq!(data["resumable_reason"], "succeeded");
    assert_eq!(data["operation_id"], OPERATION);
    assert_eq!(
        data["request"]["range"],
        json!({"from_us": 0, "to_us": 20_000_000})
    );
    assert_eq!(data["progress"]["chunks_checkpointed"], 0);
    assert_eq!(data["attempts"], 1);
    assert_eq!(data["result"]["generation"], 2);
    assert!(
        data["result"]["revision_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("trv_"))
    );
    assert_eq!(data["failure"], Value::Null);
    assert_no_path(&status, &root.0);

    for _ in 0..2 {
        let cancelled = job_result(
            &vsift(&root, &["job", "cancel", job.as_str(), "--json"])?,
            "job.cancel",
        )?;
        assert_eq!(cancelled["data"]["state"], "succeeded");
        assert_eq!(cancelled["data"]["result"], data["result"]);
        let warning = cancelled["warnings"][0].as_str().ok_or("no warning")?;
        assert!(warning.starts_with("cancellation_too_late:"));
    }

    let refused = vsift(&root, &["job", "resume", job.as_str(), "--json"])?;
    let value = assert_failure(&refused, "job.resume", "INVALID_ARGUMENT", 2)?;
    assert_eq!(value["error"]["affected_ids"], json!([job.as_str()]));

    let listed = json(&vsift(&root, &["session", "status", SESSION, "--json"])?)?;
    assert_eq!(listed["data"]["session_id"], SESSION);
    assert_eq!(listed["data"]["generation"], 2);
    assert_eq!(listed["data"]["jobs_truncated"], false);
    assert_eq!(
        listed["data"]["jobs"],
        json!([{
            "job_id": job.as_str(),
            "kind": "retranscribe",
            "state": "succeeded",
            "live_owner": false,
            "resumable": false,
            "resumable_reason": "succeeded",
        }])
    );
    Ok(())
}

/// X-06 while admitting and running: a live owner (this test process) is
/// asked to stop; the job is `cancelling` and the request idempotent; `job
/// resume` is `BUSY` with the job and a retry hint (X-09). When the owner
/// ends without reacting, the next status reconciles it as `cancelled`.
#[test]
fn a_live_job_is_busy_and_cancel_only_asks_its_owner() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root)?;
    let store = store(&root)?;
    for phase in ["admitting", "running"] {
        let requested = if phase == "admitting" {
            range(0, 10 * SECOND)?
        } else {
            range(0, 20 * SECOND)?
        };
        let resolved = resolve(&store, &session, Some(requested))?;
        let job = resolved.spec.job_id.clone();
        let (mut owner, created) = store.open_or_create(&resolved.spec, now()?)?;
        assert!(created);
        if phase == "running" {
            owner.apply(&JobChange::Start, now()?)?;
        }

        let status = job_result(
            &vsift(&root, &["job", "status", job.as_str(), "--json"])?,
            "job.status",
        )?;
        assert_eq!(status["data"]["live_owner"], true, "{phase}");
        assert_eq!(status["data"]["resumable"], false, "{phase}");
        assert_eq!(status["data"]["resumable_reason"], "live_owner", "{phase}");
        assert_eq!(status["data"]["progress"]["chunks_total"], 1, "{phase}");

        let busy = vsift(&root, &["job", "resume", job.as_str(), "--json"])?;
        let value = assert_failure(&busy, "job.resume", "BUSY", 4)?;
        assert_eq!(value["error"]["retryable"], true);
        assert_eq!(value["error"]["retry_after_ms"], 2_000);
        assert_eq!(value["error"]["affected_ids"], json!([job.as_str()]));

        for _ in 0..2 {
            let asked = job_result(
                &vsift(&root, &["job", "cancel", job.as_str(), "--json"])?,
                "job.cancel",
            )?;
            assert_eq!(asked["data"]["state"], "cancelling", "{phase}");
            assert_eq!(asked["data"]["live_owner"], true, "{phase}");
            assert_eq!(asked["warnings"], json!([]));
        }
        assert!(owner.cancel_requested(), "{phase}");
        drop(owner);
        let ended = job_result(
            &vsift(&root, &["job", "status", job.as_str(), "--json"])?,
            "job.status",
        )?;
        assert_eq!(ended["data"]["state"], "cancelled", "{phase}");
        assert_eq!(ended["data"]["resumable_reason"], "cancelled", "{phase}");
    }
    Ok(())
}

/// X-06 while committing: a cancel that loses to the commit is too late;
/// the job keeps committing under its owner and nothing is cancelled.
#[test]
fn a_cancel_while_committing_is_too_late() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root)?;
    let store = store(&root)?;
    let resolved = resolve(&store, &session, None)?;
    let job = resolved.spec.job_id.clone();
    let (mut owner, _) = store.open_or_create(&resolved.spec, now()?)?;
    owner.apply(&JobChange::Start, now()?)?;
    owner.apply(
        &JobChange::Commit(JobCommit {
            operation_id: commit_operation_id(&job, 0, 1)?,
            observed_generation: resolved.observed,
            revision_id: TranscriptRevisionId::parse("trv_0123456789abcdef")?,
        }),
        now()?,
    )?;
    for _ in 0..2 {
        let late = job_result(
            &vsift(&root, &["job", "cancel", job.as_str(), "--json"])?,
            "job.cancel",
        )?;
        assert_eq!(late["data"]["state"], "committing");
        assert_eq!(late["data"]["live_owner"], true);
        let warning = late["warnings"][0].as_str().ok_or("no warning")?;
        assert!(warning.starts_with("cancellation_too_late:"));
    }
    assert!(!owner.cancel_requested());
    // The owner ends without its commit landing: recovery makes it
    // interrupted, never cancelled.
    drop(owner);
    let status = job_result(
        &vsift(&root, &["job", "status", job.as_str(), "--json"])?,
        "job.status",
    )?;
    assert_eq!(status["data"]["state"], "interrupted");
    assert_eq!(status["data"]["resumable"], true);
    Ok(())
}

/// An interrupted job reports its checkpoints and last failure; `job
/// cancel` cancels it and removes its checkpoints, and a repeated cancel
/// changes nothing.
#[test]
fn an_interrupted_job_is_cancelled_and_its_checkpoints_removed() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root)?;
    let store = store(&root)?;
    let resolved = resolve(&store, &session, None)?;
    let job = resolved.spec.job_id.clone();
    assert!(!run(&store, &resolved, None, Some(1))?);
    drop(store);
    assert_eq!(checkpoint_files(&root, &job)?, 1);

    let status = job_result(
        &vsift(&root, &["job", "status", job.as_str(), "--json"])?,
        "job.status",
    )?;
    let data = &status["data"];
    assert_eq!(data["state"], "interrupted");
    assert_eq!(data["resumable"], true);
    assert_eq!(data["resumable_reason"], "interrupted");
    assert_eq!(
        data["progress"],
        json!({"chunks_total": 3, "chunks_checkpointed": 1})
    );
    assert_eq!(
        data["failure"],
        json!({"code": "DEADLINE_EXCEEDED", "retryable": false})
    );
    assert_eq!(data["operation_id"], Value::Null);
    assert_eq!(data["request"]["range"], Value::Null);

    let listed = json(&vsift(&root, &["session", "status", SESSION, "--json"])?)?;
    assert_eq!(listed["data"]["jobs"][0]["state"], "interrupted");
    assert_eq!(listed["data"]["jobs"][0]["resumable"], true);

    let cancelled = job_result(
        &vsift(&root, &["job", "cancel", job.as_str(), "--json"])?,
        "job.cancel",
    )?;
    assert_eq!(cancelled["data"]["state"], "cancelled");
    assert_eq!(cancelled["data"]["progress"]["chunks_checkpointed"], 0);
    assert_eq!(cancelled["warnings"], json!([]));
    assert_eq!(checkpoint_files(&root, &job)?, 0);
    let again = job_result(
        &vsift(&root, &["job", "cancel", job.as_str(), "--json"])?,
        "job.cancel",
    )?;
    assert_eq!(again["data"], cancelled["data"]);
    Ok(())
}

/// A job whose session was closed cannot continue: `job resume` fails before
/// anything runs with the session and job named and the renew-or-reopen
/// remediation; `job status` says why it is not resumable.
#[test]
fn a_closed_sessions_job_cannot_resume() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root)?;
    let store = store(&root)?;
    let resolved = resolve(&store, &session, None)?;
    let job = resolved.spec.job_id.clone();
    assert!(!run(&store, &resolved, None, Some(0))?);
    drop(store);
    let closed = vsift(&root, &["session", "close", SESSION, "--json"])?;
    assert_eq!(
        closed.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&closed.stdout)
    );

    let refused = vsift(&root, &["job", "resume", job.as_str(), "--json"])?;
    let value = assert_failure(&refused, "job.resume", "INVALID_ARGUMENT", 2)?;
    assert_eq!(
        value["error"]["affected_ids"],
        json!([SESSION, job.as_str()])
    );
    let summary = value["error"]["remediation"][0]["summary"]
        .as_str()
        .ok_or("no remediation")?;
    assert!(summary.contains("session renew") && summary.contains("ingest"));

    let status = job_result(
        &vsift(&root, &["job", "status", job.as_str(), "--json"])?,
        "job.status",
    )?;
    assert_eq!(status["data"]["resumable"], false);
    assert_eq!(status["data"]["resumable_reason"], "session_not_open");
    Ok(())
}

/// D-1 and D-4 through the flag: a request that reuses an operation id bound
/// to another request is `IDEMPOTENCY_CONFLICT` (exit 2, not retryable)
/// with the job named, before any tool is needed (none is on `PATH`); the
/// same request with the same id replays nothing without tools either,
/// because the committed revision answers it.
#[test]
fn an_operation_id_reused_for_another_request_conflicts_through_the_flag() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root)?;
    let operation = OperationId::parse(OPERATION)?;
    let store = store(&root)?;
    let resolved = resolve(&store, &session, Some(range(0, 20 * SECOND)?))?;
    assert!(run(&store, &resolved, Some(&operation), None)?);
    drop(store);
    let job = resolved.spec.job_id.clone();
    let generation =
        |root: &OwnedRoot| -> Built<Value> {
            Ok(json(&vsift(root, &["session", "status", SESSION, "--json"])?)?["data"]["generation"]
            .clone())
        };
    let before = generation(&root)?;

    let conflict = vsift(
        &root,
        &[
            "transcript",
            "retranscribe",
            SESSION,
            "--from",
            "0",
            "--to",
            "10000000",
            "--operation-id",
            OPERATION,
            "--json",
        ],
    )?;
    let value = assert_failure(
        &conflict,
        "transcript.retranscribe",
        "IDEMPOTENCY_CONFLICT",
        2,
    )?;
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["retry_after_ms"], Value::Null);
    assert_eq!(value["error"]["affected_ids"], json!([job.as_str()]));
    assert_eq!(generation(&root)?, before);

    let replayed = vsift(
        &root,
        &[
            "transcript",
            "retranscribe",
            SESSION,
            "--from",
            "0",
            "--to",
            "20000000",
            "--operation-id",
            OPERATION,
            "--json",
        ],
    )?;
    let value = json(&replayed)?;
    assert_eq!(replayed.status.code(), Some(0), "{value}");
    validate("transcript-retranscribe-data.schema.json", &value["data"])?;
    assert_eq!(value["operation_id"], OPERATION);
    assert_eq!(value["data"]["job"]["replayed"], true);
    assert_eq!(value["data"]["job"]["job_id"], job.as_str());
    assert_eq!(generation(&root)?, before);
    Ok(())
}
