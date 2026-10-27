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
    process::{Output, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
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
            request: JobRequest::Retranscribe {
                range: requested,
                share: vsift_domain::RecognitionShare::Whole,
            },
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
    run_publishing(
        store,
        resolved,
        operation,
        fails_at,
        vsift_application::Publication::AtTheSeal,
    )
}

/// [`run`], publishing the revision as `publication` says (on a stopwatch
/// that never moves, so an interval of zero publishes after every chunk that
/// settles something).
fn run_publishing(
    store: &FilesystemSessionStore,
    resolved: &Resolved,
    operation: Option<&OperationId>,
    fails_at: Option<u32>,
    publication: vsift_application::Publication,
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
            publication,
            stopwatch: &vsift_application::StoppedStopwatch,
            seed: None,
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

/// RV-1 finding 7: `job cancel` of an interrupted job that published part of
/// its revision, while the session is busy, cancels the job but cannot seal
/// the revision as stopped, and says so (`revision_left_open`). A write to the
/// session is then refused `BUSY` with `job cancel <job>` as the remediation,
/// since waiting for an ended job would not help; cancelling again seals it.
#[test]
fn a_cancel_that_cannot_seal_its_revision_says_so_and_names_the_cancel() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root)?;
    let store = store(&root)?;
    let resolved = resolve(&store, &session, None)?;
    let job = resolved.spec.job_id.clone();
    let publication = vsift_application::Publication::AsItGoes {
        interval: Duration::ZERO,
    };
    assert!(!run_publishing(
        &store,
        &resolved,
        None,
        Some(1),
        publication
    )?);
    drop(store);
    let to = SOURCE_MICROS.to_string();
    let read = [
        "transcript",
        "get",
        SESSION,
        "--from",
        "0",
        "--to",
        &to,
        "--json",
    ];
    let open = json(&vsift(&root, &read)?)?;
    assert_eq!(open["data"]["revision"]["state"], "open", "{open}");

    // Another process holds the session's writer lock while the job is
    // cancelled, so the stopped seal finds the session busy every time.
    let writer = fs::OpenOptions::new().read(true).write(true).open(
        root.sessions()
            .join("coordination")
            .join(format!("{SESSION}.writer.lock")),
    )?;
    writer.try_lock()?;
    let cancelled = job_result(
        &vsift(&root, &["job", "cancel", job.as_str(), "--json"])?,
        "job.cancel",
    )?;
    writer.unlock()?;
    drop(writer);
    assert_eq!(cancelled["data"]["state"], "cancelled");
    assert_eq!(
        cancelled["warnings"],
        json!([vsift_contract::REVISION_LEFT_OPEN_WARNING])
    );
    let still = json(&vsift(&root, &read)?)?;
    assert_eq!(still["data"]["revision"]["state"], "open");

    let bundle = root.path("bundle");
    let refused = assert_failure(
        &vsift(
            &root,
            &[
                "session",
                "retain",
                SESSION,
                "--output",
                bundle.to_str().ok_or("path")?,
                "--json",
            ],
        )?,
        "session.retain",
        "BUSY",
        4,
    )?;
    assert_eq!(
        refused["error"]["affected_ids"],
        json!([SESSION, job.as_str()])
    );
    assert_eq!(
        refused["error"]["remediation"][0]["summary"],
        vsift_contract::REVISION_LEFT_OPEN_REMEDIATION
    );
    assert_eq!(
        refused["error"]["remediation"][0]["command"],
        json!({"executable": "vsift", "arguments": ["job", "cancel", job.as_str()]})
    );

    let again = job_result(
        &vsift(&root, &["job", "cancel", job.as_str(), "--json"])?,
        "job.cancel",
    )?;
    assert_eq!(again["warnings"], json!([]));
    let stopped = json(&vsift(&root, &read)?)?;
    assert_eq!(stopped["data"]["revision"]["state"], "stopped");
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
    // L-070: the remediation must name the way on that works. A renewal only
    // extends an open session, so it must not be advised, and following it
    // is refused.
    assert!(!summary.contains("session renew") && summary.contains("ingest"));
    let renewed = vsift(&root, &["session", "renew", SESSION, "--json"])?;
    assert_ne!(renewed.status.code(), Some(0));
    let renewed: Value = serde_json::from_slice(&renewed.stdout)?;
    assert_eq!(renewed["status"], "failed");

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

// ------------------------------------------------------------ job start (ADR 0033)

/// A complete `job start` result whose data is schema-valid.
fn start_result(output: &Output) -> Built<Value> {
    let value = json(output)?;
    assert_eq!(output.status.code(), Some(0), "{value}");
    assert_eq!(value["command"], "job.start");
    assert_eq!(value["status"], "complete");
    validate("job-start-data.schema.json", &value["data"])?;
    Ok(value)
}

/// Asks `job status` until the job has no live owner, for at most 30 s.
fn settled_status(root: &OwnedRoot, job: &str) -> Built<Value> {
    let waited = std::time::Instant::now();
    loop {
        let status = job_result(
            &vsift(root, &["job", "status", job, "--json"])?,
            "job.status",
        )?;
        if status["data"]["live_owner"] == false && status["data"]["state"] != "queued" {
            return Ok(status);
        }
        if waited.elapsed() > Duration::from_secs(30) {
            return Err(format!("the runner never settled: {status}").into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// ADR 0033 section 5: on a session with no transcript yet (its length is not
/// known), `job start` is watched: it starts the background runner, which
/// takes the job before the call answers. Without whisper.cpp the runner's
/// transcript stream fails with `MISSING_CAPABILITY`, so the job ends failed
/// and no runner is left. A second `job start` starts the failed job again.
#[test]
fn job_start_hands_a_watched_job_to_a_background_runner() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root)?;
    let started = start_result(&vsift(
        &root,
        &["job", "start", session.as_str(), "--json"],
    )?)?;
    let data = &started["data"];
    assert_eq!(data["start"]["state"], "created");
    assert_eq!(data["start"]["mode"], "watched");
    assert_eq!(data["start"]["mode_basis"], "length_unknown");
    assert_eq!(data["start"]["runner"], "background");
    assert!(data["start"]["detachment"].is_string());
    assert_eq!(data["outcome"], Value::Null);
    assert_eq!(data["job"]["kind"], "prepare");
    assert_eq!(data["job"]["request"]["mode"], "watched");
    assert_no_path(&started, &root.0);
    let job = data["job"]["job_id"].as_str().ok_or("no job")?.to_owned();

    let status = settled_status(&root, &job)?;
    assert_eq!(status["data"]["state"], "failed", "{status}");
    assert_eq!(status["data"]["failure"]["code"], "MISSING_CAPABILITY");
    let streams = status["data"]["streams"].as_array().ok_or("no streams")?;
    // The visual stream runs first (D6) and failed before it analysed anything:
    // the transcript never started.
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0]["stream"], "visual");
    assert_eq!(streams[0]["state"], "not_started");
    assert_eq!(streams[0]["job_id"], Value::Null);

    let again = start_result(&vsift(
        &root,
        &["job", "start", session.as_str(), "--json"],
    )?)?;
    assert_eq!(again["data"]["start"]["state"], "restarted");
    assert_eq!(again["data"]["job"]["job_id"], job.as_str());
    settled_status(&root, &job)?;
    Ok(())
}

/// A watched `prepare` job of `session` that a runner started and that the
/// test process holds as its owner.
fn started_prepare(
    root: &OwnedRoot,
    session: &SessionId,
) -> Built<(vsift_infrastructure::FilesystemJobOwner, JobId)> {
    let spec =
        vsift_application::prepare_spec(session, vsift_domain::JobMode::Watched, None, None)?;
    let job = spec.job_id.clone();
    let (mut owner, created) = store(root)?.open_or_create(&spec, now()?)?;
    assert!(created);
    owner.apply(&JobChange::Start, now()?)?;
    Ok((owner, job))
}

/// Starts the session's watched job again and checks the answer came only
/// once a runner held the job or had ended it (RV-1 finding 3), then that
/// the runner ran it (without whisper.cpp its transcript stream fails).
fn restart_watched(root: &OwnedRoot, session: &SessionId, job: &JobId, start: &str) -> TestResult {
    let started = start_result(&vsift(
        root,
        &[
            "job",
            "start",
            session.as_str(),
            "--mode",
            "watched",
            "--json",
        ],
    )?)?;
    let data = &started["data"];
    assert_eq!(data["job"]["job_id"], job.as_str());
    assert_eq!(data["start"]["state"], start, "{started}");
    assert_eq!(data["start"]["runner"], "background");
    assert!(
        data["job"]["live_owner"] == true || data["job"]["state"] == "failed",
        "answered before a runner held the job: {started}"
    );
    let status = settled_status(root, job.as_str())?;
    assert_eq!(status["data"]["state"], "failed", "{status}");
    assert_eq!(status["data"]["failure"]["code"], "MISSING_CAPABILITY");
    Ok(())
}

/// RV-1 finding 3: `job start` of a session whose watched `prepare` was
/// interrupted (its runner died) waits, as for a new job, until the new
/// runner holds the job or has ended it, rather than answering at once
/// because the job is not `queued`.
#[test]
fn job_start_of_an_interrupted_watched_job_waits_for_its_runner() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root)?;
    let (owner, job) = started_prepare(&root, &session)?;
    drop(owner);
    let status = job_result(
        &vsift(&root, &["job", "status", job.as_str(), "--json"])?,
        "job.status",
    )?;
    assert_eq!(status["data"]["state"], "interrupted");
    restart_watched(&root, &session, &job, "waiting")
}

/// RV-1 finding 3: a watched `prepare` whose cancellation its runner never
/// finished (the runner died while `cancelling`) is cancelled, so `job start`
/// starts it again (`restarted`) and its runner runs it, instead of handing it
/// as waiting to a runner that refuses a cancelled job.
#[test]
fn job_start_of_a_job_left_cancelling_starts_it_again() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root)?;
    let (owner, job) = started_prepare(&root, &session)?;
    let asked = job_result(
        &vsift(&root, &["job", "cancel", job.as_str(), "--json"])?,
        "job.cancel",
    )?;
    assert_eq!(asked["data"]["state"], "cancelling");
    drop(owner);
    restart_watched(&root, &session, &job, "restarted")
}

/// RV-1 finding 1: a queued job whose runner died (its entry old and at the
/// head of the root's queue, its job queued with no live owner) does not hold
/// up a queued job started after it: that job's runner takes the turn and runs
/// it (without whisper.cpp its stream fails), and the dead entry keeps its
/// place at the head for when its job runs again.
#[test]
fn a_dead_head_of_the_queue_does_not_hold_up_a_live_job() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root)?;
    let store = store(&root)?;
    let dead = vsift_application::prepare_spec(
        &session,
        vsift_domain::JobMode::Queued,
        Some(range(0, 30 * SECOND)?),
        None,
    )?;
    drop(store.open_or_create(&dead, now()?)?);
    store.enqueue_job(&session, &dead.job_id, now()?.saturating_sub(3_600))?;
    drop(store);

    let started = start_result(&vsift(
        &root,
        &[
            "job",
            "start",
            session.as_str(),
            "--mode",
            "queued",
            "--json",
        ],
    )?)?;
    let job = started["data"]["job"]["job_id"]
        .as_str()
        .ok_or("no job")?
        .to_owned();
    assert_ne!(job, dead.job_id.as_str());
    let status = match settled_status(&root, &job) {
        Ok(status) => status,
        Err(error) => {
            // Starved behind the dead entry, the runner would wait for ever.
            let _ = vsift(&root, &["job", "cancel", &job, "--json"]);
            return Err(error);
        }
    };
    assert_eq!(status["data"]["state"], "failed", "{status}");
    assert_eq!(status["data"]["failure"]["code"], "MISSING_CAPABILITY");
    let head = job_result(
        &vsift(&root, &["job", "status", dead.job_id.as_str(), "--json"])?,
        "job.status",
    )?;
    assert_eq!(head["data"]["state"], "queued");
    assert_eq!(head["data"]["queue_position"], 0, "{head}");
    Ok(())
}

/// ADR 0033 section 6: `--mode quick` runs the job in the foreground and
/// answers when it ends, here with the stream's failure; `job status` then
/// reports the failed `prepare` job, and the readable text of `job start`
/// names it too.
#[test]
fn a_quick_job_start_runs_in_the_foreground() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root)?;
    let output = vsift(
        &root,
        &[
            "job",
            "start",
            session.as_str(),
            "--mode",
            "quick",
            "--json",
        ],
    )?;
    assert_failure(&output, "job.start", "MISSING_CAPABILITY", 2)?;
    let listed = json(&vsift(
        &root,
        &["session", "status", session.as_str(), "--json"],
    )?)?;
    let jobs = listed["data"]["jobs"].as_array().ok_or("no jobs")?;
    let prepare = jobs
        .iter()
        .find(|job| job["kind"] == "prepare")
        .ok_or("no prepare job")?;
    assert_eq!(prepare["state"], "failed");
    assert_eq!(prepare["live_owner"], false);
    let job = prepare["job_id"].as_str().ok_or("no job id")?;
    let status = job_result(
        &vsift(&root, &["job", "status", job, "--json"])?,
        "job.status",
    )?;
    assert_eq!(status["data"]["request"]["mode"], "quick");

    // Readable text: the job and its stream, no progress line (standard
    // error is not a terminal here).
    let text = vsift(&root, &["job", "status", job])?;
    assert_eq!(text.status.code(), Some(0));
    let shown = String::from_utf8(text.stdout)?;
    assert!(shown.contains("(prepare)"), "{shown}");
    assert!(shown.contains("Mode: quick"), "{shown}");
    assert!(shown.contains("Stream visual: not_started"), "{shown}");
    assert!(!String::from_utf8(text.stderr)?.contains('\r'));
    Ok(())
}

/// D5: `--first <from_us>:<to_us>` is checked before anything is read (an
/// empty, reversed, malformed or over-long window is a parse failure) and a
/// valid one is recorded on the job, which a later start finds again.
#[test]
fn a_first_window_is_checked_and_recorded() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root)?;
    for bad in ["10:5", "5:5", "five:ten", "0:1800000001", "100"] {
        let output = vsift(
            &root,
            &["job", "start", session.as_str(), "--first", bad, "--json"],
        )?;
        let value = assert_failure(&output, "parse", "INVALID_ARGUMENT", 2)?;
        assert_no_path(&value, &root.0);
    }
    let output = vsift(
        &root,
        &[
            "job",
            "start",
            session.as_str(),
            "--mode",
            "quick",
            "--first",
            "60000000:70000000",
            "--json",
        ],
    )?;
    // Without media tools the quick run fails; the job keeps its window.
    assert_failure(&output, "job.start", "MISSING_CAPABILITY", 2)?;
    let listed = json(&vsift(
        &root,
        &["session", "status", session.as_str(), "--json"],
    )?)?;
    let job = listed["data"]["jobs"]
        .as_array()
        .and_then(|jobs| jobs.iter().find(|job| job["kind"] == "prepare"))
        .and_then(|job| job["job_id"].as_str())
        .ok_or("no prepare job")?
        .to_owned();
    let status = job_result(
        &vsift(&root, &["job", "status", &job, "--json"])?,
        "job.status",
    )?;
    assert_eq!(
        status["data"]["request"]["first"],
        json!({"from_us": 60_000_000, "to_us": 70_000_000})
    );
    Ok(())
}

/// ADR 0033 section 6: `--mode queued` hands the job to a background runner
/// that waits its turn in the root's queue; alone in it, it runs at once and,
/// without media tools, ends failed and leaves the queue.
#[test]
fn a_queued_job_start_takes_its_turn_in_the_background() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root)?;
    let started = start_result(&vsift(
        &root,
        &[
            "job",
            "start",
            session.as_str(),
            "--mode",
            "queued",
            "--json",
        ],
    )?)?;
    assert_eq!(started["data"]["start"]["mode"], "queued");
    assert_eq!(started["data"]["start"]["mode_basis"], "requested");
    assert_eq!(started["data"]["start"]["runner"], "background");
    assert_eq!(started["data"]["job"]["request"]["mode"], "queued");
    let job = started["data"]["job"]["job_id"]
        .as_str()
        .ok_or("no job")?
        .to_owned();
    let status = settled_status(&root, &job)?;
    assert_eq!(status["data"]["state"], "failed", "{status}");
    // An ended job has no place in the queue.
    assert_eq!(status["data"].get("queue_position"), None);
    assert!(
        !root.sessions().join("job-queue").exists()
            || fs::read_dir(root.sessions().join("job-queue"))?.count() == 0
    );
    Ok(())
}

/// Runs `vsift` as [`vsift`] does but as a caller that captures its output
/// through pipes and reads them to their end, as an AI assistant runs a
/// command, on a thread of its own: the channel hears the output once both
/// pipes have closed.
fn captured(root: &OwnedRoot, arguments: &[&str]) -> Built<mpsc::Receiver<io::Result<Output>>> {
    let base = root.path("user");
    let child = std::process::Command::new(Command::cargo_bin("vsift")?.get_program())
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .env("PATH", "")
        .arg("--session-root")
        .arg(root.sessions())
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(child.wait_with_output());
    });
    Ok(receiver)
}

/// L-229 (fix F8, ADR 0026 amendment of 2026-10-27): a caller that captures
/// the output of `job start --mode queued` hears the answer within seconds
/// while the queued job's background runner waits behind a live job, because
/// the runner inherits none of the caller's handles. Before the fix the runner
/// held the caller's output pipe on Windows, so the caller waited until the
/// runner ended. The job ahead is one the test process holds as its owner, so
/// the runner waits until it is cancelled.
#[test]
fn a_captured_queued_job_start_answers_while_its_runner_waits() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root)?;
    let store = store(&root)?;
    let ahead = vsift_application::prepare_spec(
        &session,
        vsift_domain::JobMode::Queued,
        Some(range(0, 30 * SECOND)?),
        None,
    )?;
    let (owner, created) = store.open_or_create(&ahead, now()?)?;
    assert!(created);
    store.enqueue_job(&session, &ahead.job_id, now()?.saturating_sub(3_600))?;

    let answer = captured(
        &root,
        &[
            "job",
            "start",
            session.as_str(),
            "--mode",
            "queued",
            "--json",
        ],
    )?;
    let Ok(output) = answer.recv_timeout(Duration::from_secs(30)) else {
        // The runner waits for ever behind the live job: cancel the
        // waiting job so that the runner ends and the pipes close.
        let listed = json(&vsift(
            &root,
            &["session", "status", session.as_str(), "--json"],
        )?)?;
        if let Some(waiting) = listed["data"]["jobs"].as_array().and_then(|jobs| {
            jobs.iter()
                .filter_map(|job| job["job_id"].as_str())
                .find(|job| *job != ahead.job_id.as_str())
        }) {
            let _ = vsift(&root, &["job", "cancel", waiting, "--json"]);
            let _ = settled_status(&root, waiting);
        }
        let _ = answer.recv_timeout(Duration::from_secs(30));
        return Err(
            "the captured output of `job start` stayed open while its runner waited".into(),
        );
    };
    let output = output?;
    let started = start_result(&output)?;
    let job = started["data"]["job"]["job_id"]
        .as_str()
        .ok_or("no job")?
        .to_owned();
    assert_ne!(job, ahead.job_id.as_str());
    let status = job_result(
        &vsift(&root, &["job", "status", &job, "--json"])?,
        "job.status",
    )?;
    assert_eq!(status["data"]["state"], "queued", "{status}");
    assert_eq!(status["data"]["live_owner"], true, "{status}");
    assert_eq!(status["data"]["queue_position"], 1, "{status}");

    job_result(
        &vsift(&root, &["job", "cancel", &job, "--json"])?,
        "job.cancel",
    )?;
    let ended = settled_status(&root, &job)?;
    assert_eq!(ended["data"]["state"], "cancelled", "{ended}");
    drop(owner);
    Ok(())
}
