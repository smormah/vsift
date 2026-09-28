//! P10 PR 2 (ADR 0020): recoverable jobs in session storage. Records,
//! ownership and the root index; strict decoding of damaged, forged and
//! future records and checkpoints (S-08); kills at every job fault point and
//! a rerun that resumes from validated checkpoints to a byte-identical
//! revision (X-01); exactly-once commit and replay (X-02); idempotency
//! conflicts and identical concurrent requests (X-03); concurrent
//! retranscriptions and renewals (X-04); and a suspended owner that keeps
//! its job while a stale attempt cannot change it (X-05).

use std::{
    fs,
    future::Future,
    num::NonZeroU16,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use vsift_application::{
    AsrCancellation, AsrFailureReason, CancelOutcome, CancelRequest, CheckpointRead,
    ChunkCheckpoints, CommitGuard, JobChange, JobCommit, JobLiveness, JobOwner, JobRecord,
    JobRequest, JobRunError, JobSpec, JobStore, JobStoreError, NoProgress, RecognitionScope,
    RecognizerIdentity, RetranscriptionOutcome, RetranscriptionPorts, RetranscriptionRun,
    RetryTimer, RevisionStore, SessionStorageError, SpeechAudioError, SpeechAudioSource, SpeechPcm,
    SpeechRecognitionError, SpeechRecognizer, TranscribeRangeRequest, cancel_job, job_id,
    job_status, recognition_key, retranscribe_operation_key, retranscribe_request_digest,
    retranscription_range, run_retranscription, whole_file_source_segment,
};
use vsift_domain::{
    AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider, AsrProviderBuild,
    CheckpointOutcome, ChunkCheckpoint, ChunkPlan, ChunkTime, CueText, FailureCode, Jitter, JobId,
    JobState, LanguageTag, MediaTime, OperationId, PlannedChunk, ProviderChunkOutput,
    ProviderSegment, ProviderToken, ProviderTokenKind, Sha256Hex, SourceId, SourceSegment,
    StorageGeneration, TimeRange, TranscriptRevision, TranscriptRevisionId,
};

use super::{
    FilesystemSessionStore, SESSIONS_DIRECTORY, StoredDurability,
    p10_tests::{
        Built, SESSION, SOURCE_BYTES, activate, assert_committed_state_is_whole, initialize_as,
        now, open_session, operation, session_id, session_path,
    },
    sha256_hex,
    tests::{Fixture, TestResult},
};
use crate::{
    BundleSourcePolicy, encode_transcript_record,
    fault_point::{FAULT_EXIT_CODE, FAULT_MARKER, FAULT_POINT_VARIABLE, FaultPoint},
};

const SECOND: u64 = 1_000_000;
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const CHILD_ROOT: &str = "VSIFT_TEST_JOB_ROOT";
const CHILD_READY: &str = "VSIFT_TEST_JOB_READY";
const OPERATION: &str = "op_4444444444444444";
/// The requested range of most tests: five R0 chunks.
const REQUESTED: (u64, u64) = (0, 115 * SECOND);

fn range(start: u64, end: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(start),
        MediaTime::from_micros(end),
    )?)
}

fn requested() -> Built<Option<TimeRange>> {
    Ok(Some(range(REQUESTED.0, REQUESTED.1)?))
}

fn identity() -> Built<RecognizerIdentity> {
    Ok(RecognizerIdentity {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(DIGEST)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(DIGEST)?),
        decoding: AsrDecodingProfile::R0V1,
        threads: NonZeroU16::MIN,
    })
}

fn source_id() -> Built<SourceId> {
    Ok(SourceId::from_sha256(&sha256_hex(SOURCE_BYTES))?)
}

fn source_segment() -> Built<SourceSegment> {
    Ok(whole_file_source_segment(
        &source_id()?,
        MediaTime::from_micros(120 * SECOND),
    )?)
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

/// A deterministic recognizer: chunk `i` always says the same words. It can
/// fail at one chunk and pause before answering (so concurrent runs overlap).
#[derive(Default)]
struct Words {
    calls: AtomicUsize,
    fail_at: Option<u32>,
    pause: Option<Duration>,
}

impl Words {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

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
                probability: 0.1 + f64::from(chunk.index()) / 7.0,
            }],
        }],
    })
}

impl SpeechRecognizer for Words {
    fn identity(
        &self,
    ) -> impl Future<Output = Result<RecognizerIdentity, SpeechRecognitionError>> + Send {
        std::future::ready(identity().map_err(|_| SpeechRecognitionError::Io))
    }

    fn recognize(
        &self,
        chunk: &PlannedChunk,
        _pcm: &SpeechPcm,
    ) -> impl Future<Output = Result<ProviderChunkOutput, SpeechRecognitionError>> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(pause) = self.pause {
            thread::sleep(pause);
        }
        let answer = if self.fail_at == Some(chunk.index()) {
            Err(SpeechRecognitionError::Deadline)
        } else {
            words(chunk)
        };
        std::future::ready(answer)
    }
}

struct Never;

impl AsrCancellation for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

struct InstantTimer;

impl RetryTimer for InstantTimer {
    fn jitter(&self) -> Jitter {
        Jitter::from_bits(1 << 31)
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

fn classify(error: &JobRunError) -> FailureCode {
    match error {
        JobRunError::Busy { .. }
        | JobRunError::Superseded { .. }
        | JobRunError::AdmissionBusy { .. }
        | JobRunError::Storage(SessionStorageError::Busy) => FailureCode::Busy,
        JobRunError::IdempotencyConflict { .. } => FailureCode::IdempotencyConflict,
        JobRunError::NotResumable { .. } => FailureCode::InvalidArgument,
        JobRunError::Cancelled { .. } => FailureCode::Cancelled,
        JobRunError::Asr { failure, .. } => match failure.failure.reason {
            AsrFailureReason::Deadline => FailureCode::DeadlineExceeded,
            AsrFailureReason::Cancelled => FailureCode::Cancelled,
            AsrFailureReason::Busy => FailureCode::Busy,
            _ => FailureCode::MissingCapability,
        },
        JobRunError::Storage(_) | JobRunError::Job(_) => FailureCode::StorageIo,
        JobRunError::Assembly(_) | JobRunError::Key(_) => FailureCode::Internal,
    }
}

/// A request resolved against the session's head, as the engine does.
struct Resolved {
    spec: JobSpec,
    replaced: TimeRange,
    base: Option<TranscriptRevision>,
    observed: StorageGeneration,
    requested: Option<TimeRange>,
}

fn resolve(store: &FilesystemSessionStore, requested: Option<TimeRange>) -> Built<Resolved> {
    let session = session_id()?;
    let segment = source_segment()?;
    let source = source_id()?;
    let recognizer = identity()?;
    let head = store.head(&session, now()?)?;
    let replaced = retranscription_range(head.newest.as_ref(), requested, segment.range());
    let key = recognition_key(&RecognitionScope {
        session_id: &session,
        source_id: &source,
        audio_stream: 1,
        replaced_range: replaced,
        plan: ChunkPlan::R0,
        recognizer: &recognizer,
        verification: None,
    })?;
    let operation_key =
        retranscribe_operation_key(&key, head.newest.as_ref().map(TranscriptRevision::id))?;
    Ok(Resolved {
        spec: JobSpec {
            job_id: job_id(&session, &operation_key)?,
            session_id: session.clone(),
            request_digest: retranscribe_request_digest(&session, requested)?,
            operation_key,
            recognition_key: key,
            request: JobRequest::Retranscribe { range: requested },
            planned_chunks: None,
        },
        replaced,
        base: head.newest,
        observed: head.generation,
        requested,
    })
}

async fn run(
    store: &FilesystemSessionStore,
    resolved: &Resolved,
    operation_id: Option<&OperationId>,
    recognizer: &Words,
) -> Built<Result<RetranscriptionOutcome, JobRunError>> {
    let segment = source_segment()?;
    let source = source_id()?;
    let identity = identity()?;
    Ok(run_retranscription(
        RetranscriptionRun {
            spec: &resolved.spec,
            operation_id,
            transcribe: TranscribeRangeRequest {
                source_segment: &segment,
                range: resolved.replaced,
                plan: ChunkPlan::R0,
                audio_stream: 1,
                expected: &identity,
            },
            source_id: &source,
            requested: resolved.requested,
            base: resolved.base.as_ref(),
            observed: resolved.observed,
            now: now()?,
            admission: vsift_domain::AdmissionWait::Immediate,
        },
        RetranscriptionPorts {
            store,
            audio: &Audio,
            recognizer,
            cancellation: &Never,
            timer: &InstantTimer,
            classify,
            progress: &NoProgress,
        },
        &mut Unchanged,
    )
    .await)
}

fn block_on<T>(future: impl Future<Output = T>) -> Built<T> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?
        .block_on(future))
}

/// Resolves and runs the standard request in a fresh store instance.
fn request_once(
    root: &Path,
    operation_id: Option<&OperationId>,
    recognizer: &Words,
) -> Built<Result<RetranscriptionOutcome, JobRunError>> {
    let store = FilesystemSessionStore::open_existing(root)?;
    let resolved = resolve(&store, requested()?)?;
    block_on(run(&store, &resolved, operation_id, recognizer))?
}

fn job_path(fixture: &Fixture, job: &JobId) -> PathBuf {
    session_path(fixture).join("jobs").join(job.as_str())
}

fn chunk_files(fixture: &Fixture, job: &JobId) -> Built<Vec<String>> {
    let chunks = job_path(fixture, job).join("chunks");
    if !chunks.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(chunks)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name
            .strip_suffix(".json")
            .is_some_and(|ordinal| !ordinal.starts_with('.'))
        {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

fn transcript_records(store: &FilesystemSessionStore) -> Built<usize> {
    let session = store
        .root
        .open_dir(PathBuf::from(SESSIONS_DIRECTORY).join(SESSION))?;
    let committed =
        super::chain::read_committed_manifest(&session, &session_id()?, super::ChainCheck::Full)?;
    Ok(committed
        .manifest
        .lifecycle
        .ok_or("no lifecycle")?
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == super::StoredArtifactKind::TranscriptRecord)
        .count())
}

/// The encoded revision an uninterrupted run over a fresh session commits.
fn control_record() -> Built<Vec<u8>> {
    let fixture = Fixture::new()?;
    open_session(&fixture, StoredDurability::Ephemeral)?;
    let outcome = request_once(&fixture.path, None, &Words::default())??;
    Ok(encode_transcript_record(&outcome.revision)?)
}

// ---------------------------------------------------------------- records

#[test]
fn a_job_is_created_indexed_owned_and_listed() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let resolved = resolve(&store, requested()?)?;
    let job = resolved.spec.job_id.clone();
    let session = session_id()?;
    let (owner, created) = store.open_or_create(&resolved.spec, now()?)?;
    assert!(created);
    assert_eq!(owner.record().state, JobState::Queued);
    assert_eq!(store.job_session(&job)?, Some(session.clone()));
    let view = store.job(&session, &job)?.ok_or("no view")?;
    assert_eq!(view.liveness, JobLiveness::Owned);
    // Another owner, even in this process, is refused while the lock is held.
    let other = FilesystemSessionStore::open_existing(&fixture.path)?;
    assert!(matches!(
        other.open_or_create(&resolved.spec, now()?),
        Err(JobStoreError::Storage(SessionStorageError::Busy))
    ));
    drop(owner);
    let (again, created) = other.open_or_create(&resolved.spec, now()?)?;
    assert!(!created);
    drop(again);
    assert_eq!(
        store.job(&session, &job)?.map(|view| view.liveness),
        Some(JobLiveness::Unowned)
    );
    let listed = store.session_jobs(&session)?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].record.job_id, job);
    // A spec whose keys differ from the stored record's is refused.
    let mut forged = resolved.spec.clone();
    forged.request = JobRequest::Retranscribe { range: None };
    assert!(matches!(
        store.open_or_create(&forged, now()?),
        Err(JobStoreError::Storage(
            SessionStorageError::IntegrityFailure
        ))
    ));
    Ok(())
}

/// S-08 for the job record: truncated, forged (another job's keys, a
/// mismatched id, an unknown field, a commit on a running job) and empty
/// records fail closed; a newer record is unsupported.
#[test]
fn damaged_forged_or_future_job_records_fail_closed() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let resolved = resolve(&store, requested()?)?;
    let job = resolved.spec.job_id.clone();
    drop(store.open_or_create(&resolved.spec, now()?)?);
    let path = job_path(&fixture, &job).join("job.json");
    let original = fs::read_to_string(&path)?;
    let other_key = format!("opk_sha256_{}", "e".repeat(64));
    for (label, damaged, expected) in [
        (
            "truncated",
            original[..original.len() / 2].to_owned(),
            SessionStorageError::IntegrityFailure,
        ),
        (
            "another job's key",
            original.replace(resolved.spec.operation_key.as_str(), &other_key),
            SessionStorageError::IntegrityFailure,
        ),
        (
            "unknown field",
            original.replacen('{', "{\"extra\":1,", 1),
            SessionStorageError::IntegrityFailure,
        ),
        (
            "commit on a queued job",
            original.replace(
                "\"commit\":null",
                &format!(
                    "\"commit\":{{\"operation_id\":\"{OPERATION}\",\"observed_generation\":2,\"revision_id\":\"trv_0123456789abcdef\"}}"
                ),
            ),
            SessionStorageError::IntegrityFailure,
        ),
        (
            "future version",
            original.replace("\"schema_version\":1", "\"schema_version\":2"),
            SessionStorageError::UnsupportedVersion,
        ),
        ("empty", String::new(), SessionStorageError::IntegrityFailure),
    ] {
        fs::write(&path, &damaged)?;
        assert_eq!(
            store.job(&session_id()?, &job).err(),
            Some(JobStoreError::Storage(expected)),
            "{label}"
        );
    }
    fs::write(&path, original)?;
    assert!(store.job(&session_id()?, &job)?.is_some());
    Ok(())
}

/// S-08 for checkpoints: a stored checkpoint reads back; truncated, forged,
/// future-version and misnamed ones are unusable and removed.
#[test]
fn checkpoints_read_back_and_damaged_ones_are_removed() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let resolved = resolve(&store, requested()?)?;
    let (owner, _) = store.open_or_create(&resolved.spec, now()?)?;
    let segment = source_segment()?;
    let chunks = vsift_domain::plan_chunks(segment.id(), resolved.replaced, ChunkPlan::R0)?;
    let checkpoint = ChunkCheckpoint::new(
        resolved.spec.recognition_key.clone(),
        &chunks[1],
        CheckpointOutcome::Recognised {
            audio: chunks[1].window(),
            output: words(&chunks[1]).map_err(|_| "no words")?,
        },
    );
    owner.store(&checkpoint)?;
    assert_eq!(owner.load(1), CheckpointRead::Found(checkpoint.clone()));
    assert_eq!(owner.load(0), CheckpointRead::Absent);
    let path = job_path(&fixture, &resolved.spec.job_id)
        .join("chunks")
        .join("00002.json");
    let original = fs::read_to_string(&path)?;
    for (label, damaged) in [
        ("truncated", original[..original.len() / 3].to_owned()),
        (
            "forged",
            original.replace("words of chunk 1", "invented words"),
        ),
        (
            "future",
            original.replace("\"schema_version\":1", "\"schema_version\":2"),
        ),
        (
            "another ordinal",
            original.replace("\"ordinal\":2", "\"ordinal\":3"),
        ),
    ] {
        fs::write(&path, &damaged)?;
        assert_eq!(owner.load(1), CheckpointRead::Unusable, "{label}");
        assert!(!path.exists(), "{label}: an unusable checkpoint was kept");
    }
    owner.store(&checkpoint)?;
    owner.discard(1);
    assert_eq!(owner.load(1), CheckpointRead::Absent);
    Ok(())
}

/// X-05: an owner whose attempt is no longer the job's current one cannot
/// change it, so a stale attempt cannot commit.
#[test]
fn a_stale_owner_cannot_change_the_job() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let resolved = resolve(&store, requested()?)?;
    let (mut owner, _) = store.open_or_create(&resolved.spec, now()?)?;
    owner.apply(&JobChange::Start, now()?)?;
    // Another attempt's record, as if the lock had been lost and a newer
    // attempt had started.
    let path = job_path(&fixture, &resolved.spec.job_id).join("job.json");
    let text = fs::read_to_string(&path)?.replace("\"attempt\":1", "\"attempt\":2");
    fs::write(&path, text)?;
    let commit = JobChange::Commit(JobCommit {
        operation_id: operation(OPERATION)?,
        observed_generation: StorageGeneration::from_value(2),
        revision_id: TranscriptRevisionId::parse("trv_0123456789abcdef")?,
    });
    assert_eq!(owner.apply(&commit, now()?), Err(JobStoreError::StaleOwner));
    Ok(())
}

/// A cancellation requested by another process reaches the live owner under
/// the state lock: the commit transition is refused and cancel wins.
#[test]
fn a_cancel_request_reaches_the_owner_and_wins_before_the_commit() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let resolved = resolve(&store, requested()?)?;
    let session = session_id()?;
    let job = resolved.spec.job_id.clone();
    let (mut owner, _) = store.open_or_create(&resolved.spec, now()?)?;
    owner.apply(&JobChange::Start, now()?)?;
    let segment = source_segment()?;
    let chunks = vsift_domain::plan_chunks(segment.id(), resolved.replaced, ChunkPlan::R0)?;
    owner.store(&ChunkCheckpoint::new(
        resolved.spec.recognition_key.clone(),
        &chunks[0],
        CheckpointOutcome::NoAudio,
    ))?;
    let other = FilesystemSessionStore::open_existing(&fixture.path)?;
    assert_eq!(
        other.request_cancel(&session, &job, now()?)?,
        CancelRequest::Requested
    );
    assert_eq!(
        other.request_cancel(&session, &job, now()?)?,
        CancelRequest::AlreadyRequested
    );
    assert!(owner.cancel_requested());
    let commit = JobChange::Commit(JobCommit {
        operation_id: operation(OPERATION)?,
        observed_generation: StorageGeneration::from_value(2),
        revision_id: TranscriptRevisionId::parse("trv_0123456789abcdef")?,
    });
    assert!(matches!(
        owner.apply(&commit, now()?),
        Err(JobStoreError::Transition(_))
    ));
    owner.apply(&JobChange::Cancel, now()?)?;
    assert!(chunk_files(&fixture, &job)?.is_empty());
    drop(owner);
    assert_eq!(
        cancel_job(&other, &session, &job, now()?)?,
        CancelOutcome::AlreadyEnded(JobState::Cancelled)
    );
    Ok(())
}

/// X-06 (running, P10 PR 3): `job cancel` from another process only records
/// `cancelling`; the owner's watcher reads it within its poll interval and
/// fires the run's cancellation, which is what stops a running provider
/// (the supervisor kills it). The watcher never fires for a job nobody asked
/// to stop, and returns the run's own result.
#[test]
fn the_owner_notices_a_cancel_request_within_its_poll_interval() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let resolved = resolve(&store, requested()?)?;
    let session = session_id()?;
    let job = resolved.spec.job_id.clone();
    let (mut owner, _) = store.open_or_create(&resolved.spec, now()?)?;
    owner.apply(&JobChange::Start, now()?)?;
    assert_eq!(
        store.recorded_job_state(&session, &job)?,
        Some(JobState::Running)
    );

    // Unasked, the watcher only returns what the run returns.
    let quiet = crate::ProcessCancellation::new();
    let answered = block_on(store.watch_for_job_cancel(&session, &job, &quiet, async {
        tokio::time::sleep(super::jobs::JOB_CANCEL_POLL * 3).await;
        7_u8
    }))?;
    assert_eq!(answered, 7);
    assert!(!quiet.is_cancelled());

    // A run that stops only when its cancellation fires, as a provider does.
    let cancellation = crate::ProcessCancellation::new();
    let root = fixture.path.clone();
    let (asked_session, asked_job, asked_now) = (session.clone(), job.clone(), now()?);
    let requester = thread::spawn(move || -> Result<(CancelRequest, Instant), String> {
        thread::sleep(Duration::from_millis(300));
        let other =
            FilesystemSessionStore::open_existing(&root).map_err(|error| error.to_string())?;
        let requested = other
            .request_cancel(&asked_session, &asked_job, asked_now)
            .map_err(|error| error.to_string())?;
        Ok((requested, Instant::now()))
    });
    let signal = cancellation.clone();
    let stopped = block_on(
        store.watch_for_job_cancel(&session, &job, &cancellation, async {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !signal.is_cancelled() && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Instant::now()
        }),
    )?;
    let (answer, asked_at) = requester.join().map_err(|_| "requester panicked")??;
    assert_eq!(answer, CancelRequest::Requested);
    assert!(cancellation.is_cancelled(), "the watcher never fired");
    // One poll interval (250 ms) plus one record read; the bound leaves
    // room for a loaded machine without hiding a watcher that never polls.
    let noticed = stopped.saturating_duration_since(asked_at);
    assert!(
        noticed <= Duration::from_secs(1),
        "noticed after {noticed:?}"
    );
    assert!(owner.cancel_requested());
    owner.apply(&JobChange::Cancel, now()?)?;
    Ok(())
}

/// At 64 jobs the oldest ended one no caller's operation id pins is pruned;
/// when every ended job is pinned, a new job does not fit.
#[test]
fn ended_jobs_are_pruned_at_the_bound_unless_an_operation_id_pins_them() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let base = resolve(&store, requested()?)?.spec;
    let session = session_id()?;
    let spec = |number: usize| -> Built<JobSpec> {
        let key = retranscribe_operation_key(
            &base.recognition_key,
            Some(&TranscriptRevisionId::parse(format!("trv_{number:016x}"))?),
        )?;
        Ok(JobSpec {
            job_id: job_id(&session, &key)?,
            operation_key: key,
            ..base.clone()
        })
    };
    let mut first = None;
    for number in 0..vsift_application::MAX_SESSION_JOBS {
        let (mut owner, created) = store.open_or_create(&spec(number)?, now()? + number as u64)?;
        assert!(created);
        owner.apply(&JobChange::Start, now()?)?;
        owner.apply(&JobChange::Fail(None), now()? + number as u64)?;
        if number > 0 {
            owner.bind_operation(&operation(&format!("op_{number:016x}"))?, now()?)?;
        } else {
            first = Some(owner.record().job_id.clone());
        }
    }
    let first = first.ok_or("no first job")?;
    // The one unpinned job is pruned to make room, with its index entry.
    let (owner, created) = store.open_or_create(&spec(1_000)?, now()?)?;
    assert!(created);
    drop(owner);
    assert!(store.job(&session, &first)?.is_none());
    assert_eq!(store.job_session(&first)?, None);
    // Now every ended job is pinned or live-free but pinned: no room.
    let (mut owner, _) = store.open_or_create(&spec(1_000)?, now()?)?;
    owner.apply(&JobChange::Start, now()?)?;
    owner.apply(&JobChange::Fail(None), now()?)?;
    owner.bind_operation(&operation("op_00000000000003e8")?, now()?)?;
    drop(owner);
    assert!(matches!(
        store.open_or_create(&spec(1_001)?, now()?),
        Err(JobStoreError::Storage(
            SessionStorageError::CapacityExhausted
        ))
    ));
    Ok(())
}

/// Cleaning a session removes its jobs and their index entries; a retained
/// bundle never holds a job file.
#[test]
fn cleanup_removes_jobs_and_their_index_and_retain_never_exports_them() -> TestResult {
    let fixture = Fixture::new()?;
    // Initialized and activated only: a bundle validates every record, and
    // the other tests' evidence records are stand-ins.
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let hooks = super::commit::CommitHooks::new();
    initialize_as(&store, StoredDurability::Ephemeral, &hooks)?;
    activate(&fixture, &store, &hooks)?;
    let recognizer = Words {
        fail_at: Some(2),
        ..Words::default()
    };
    let resolved = resolve(&store, requested()?)?;
    assert!(block_on(run(&store, &resolved, None, &recognizer))??.is_err());
    let job = resolved.spec.job_id.clone();
    assert_eq!(chunk_files(&fixture, &job)?.len(), 2);

    let bundle = fixture.path.with_extension("bundle");
    store.retain_bundle(&session_id()?, &bundle, BundleSourcePolicy::EvidenceOnly)?;
    for entry in fs::read_dir(&bundle)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        assert!(
            name == "bundle.json" || name.starts_with("artifact-"),
            "{name}"
        );
    }
    fs::remove_dir_all(&bundle)?;

    let status = store.session_status(&session_id()?)?;
    store.close_session(
        &session_id()?,
        &operation("op_5555555555555555")?,
        status.generation(),
    )?;
    assert_eq!(
        store.clean_session(&session_id()?, now()?, false)?,
        super::CleanOutcome::Removed
    );
    assert_eq!(store.job_session(&job)?, None);
    assert!(!session_path(&fixture).exists());
    Ok(())
}

// ---------------------------------------------------------------- X-01

/// X-01 in process: an attempt that failed at chunk 2 is resumed from its
/// two checkpoints and commits the byte-identical revision.
#[test]
fn a_resumed_job_commits_the_byte_identical_revision() -> TestResult {
    let control = control_record()?;
    let fixture = Fixture::new()?;
    open_session(&fixture, StoredDurability::Ephemeral)?;
    let failing = Words {
        fail_at: Some(2),
        ..Words::default()
    };
    assert!(request_once(&fixture.path, None, &failing)?.is_err());
    let recognizer = Words::default();
    let outcome = request_once(&fixture.path, None, &recognizer)??;
    assert!(outcome.report.resumed);
    assert_eq!(outcome.report.chunks_reused, 2);
    assert_eq!(recognizer.calls(), 3);
    assert_eq!(encode_transcript_record(&outcome.revision)?, control);
    assert!(chunk_files(&fixture, &outcome.report.job_id)?.is_empty());
    Ok(())
}

/// Where the child stops: a point and its arrival.
const KILL_POINTS: [(FaultPoint, u32); 11] = [
    (FaultPoint::JobCreate, 1),
    (FaultPoint::JobIndex, 1),
    (FaultPoint::ChunkRecognised, 3),
    (FaultPoint::CheckpointWrite, 3),
    (FaultPoint::CheckpointFlush, 3),
    (FaultPoint::CheckpointRename, 3),
    (FaultPoint::JobCommitting, 1),
    (FaultPoint::ManifestRename, 1),
    (FaultPoint::PointerRename, 1),
    (FaultPoint::JobSucceeded, 1),
    (FaultPoint::CheckpointDeletion, 1),
];

/// X-01 and X-02: a child process running the job is killed at every job
/// fault point (and at the manifest and pointer renames of its commit); the
/// same request with the same operation id then resumes from the validated
/// checkpoints it finds, or replays the landed commit, and the session holds
/// exactly one revision, byte-identical to an uninterrupted run's.
#[test]
fn a_job_killed_at_every_job_point_resumes_or_replays_exactly_once() -> TestResult {
    let control = control_record()?;
    for point in FaultPoint::JOB {
        assert!(
            KILL_POINTS.iter().any(|(covered, _)| *covered == point),
            "{point} is not killed at"
        );
    }
    for (point, arrival) in KILL_POINTS {
        let fixture = Fixture::new()?;
        open_session(&fixture, StoredDurability::Ephemeral)?;
        let output = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "filesystem_session_store::job_tests::job_crash_child",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD_ROOT, &fixture.path)
            .env(FAULT_POINT_VARIABLE, format!("{}:{arrival}", point.name()))
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
        assert!(
            stderr.contains(&format!("{FAULT_MARKER}={point}")),
            "{point}: {stderr}"
        );

        let store = FilesystemSessionStore::open_existing(&fixture.path)?;
        assert_committed_state_is_whole(&store)?;
        let job = resolve(&store, requested()?)
            .map(|resolved| resolved.spec.job_id)
            .ok();
        let found = match &job {
            Some(job) => chunk_files(&fixture, job)?.len(),
            None => 0,
        };
        let recognizer = Words::default();
        let op = operation(OPERATION)?;
        let outcome = request_once(&fixture.path, Some(&op), &recognizer)??;
        assert_eq!(
            encode_transcript_record(&outcome.revision)?,
            control,
            "{point}"
        );
        assert_eq!(transcript_records(&store)?, 1, "{point}");
        assert_committed_state_is_whole(&store)?;
        let landed = matches!(
            point,
            FaultPoint::PointerRename | FaultPoint::JobSucceeded | FaultPoint::CheckpointDeletion
        );
        if landed {
            assert!(outcome.report.replayed, "{point}");
            assert_eq!(recognizer.calls(), 0, "{point}");
        } else {
            assert!(!outcome.report.replayed, "{point}");
            // Only validated checkpoints were reused, and every one found.
            let reused = usize::try_from(outcome.report.chunks_reused)?;
            assert_eq!(reused, found, "{point}");
            assert_eq!(recognizer.calls(), 5 - reused, "{point}");
        }
        let state = store
            .job(&session_id()?, &outcome.report.job_id)?
            .map(|view| view.record.state);
        assert_eq!(state, Some(JobState::Succeeded), "{point}");
    }
    Ok(())
}

/// Child of the kill test: runs the standard request under the fault point
/// its environment names.
#[test]
#[ignore = "run only as a child process by the job kill test"]
fn job_crash_child() -> TestResult {
    let root = PathBuf::from(std::env::var_os(CHILD_ROOT).ok_or("missing root")?);
    let op = operation(OPERATION)?;
    let outcome = request_once(&root, Some(&op), &Words::default())?;
    Err(format!("the fault point was not reached: {outcome:?}").into())
}

/// A checkpoint damaged while the job was interrupted is discarded and its
/// chunk redone; the revision is still byte-identical.
#[test]
fn a_checkpoint_damaged_between_attempts_is_redone() -> TestResult {
    let control = control_record()?;
    let fixture = Fixture::new()?;
    open_session(&fixture, StoredDurability::Ephemeral)?;
    let failing = Words {
        fail_at: Some(3),
        ..Words::default()
    };
    let failure = request_once(&fixture.path, None, &failing)?;
    let job = failure
        .err()
        .and_then(|error| error.job().cloned())
        .ok_or("no job")?;
    let path = job_path(&fixture, &job).join("chunks").join("00002.json");
    let text = fs::read_to_string(&path)?.replace("words of chunk 1", "invented words");
    fs::write(&path, text)?;
    let recognizer = Words::default();
    let outcome = request_once(&fixture.path, None, &recognizer)??;
    assert_eq!(outcome.report.chunks_reused, 2);
    assert_eq!(outcome.report.checkpoints_discarded, 1);
    assert_eq!(recognizer.calls(), 3);
    assert_eq!(encode_transcript_record(&outcome.revision)?, control);
    Ok(())
}

// ---------------------------------------------------------------- X-03

/// X-03: the same operation id for another range is a conflict that changes
/// nothing.
#[test]
fn an_operation_id_reused_for_another_range_conflicts() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let op = operation(OPERATION)?;
    let first = request_once(&fixture.path, Some(&op), &Words::default())??;
    let before = store.session_status(&session_id()?)?.generation();
    let other = resolve(&store, Some(range(0, 60 * SECOND)?))?;
    let conflict = block_on(run(&store, &other, Some(&op), &Words::default()))??;
    assert_eq!(
        conflict,
        Err(JobRunError::IdempotencyConflict {
            job: first.report.job_id
        })
    );
    assert_eq!(store.session_status(&session_id()?)?.generation(), before);
    Ok(())
}

/// X-03: 2, 4 and 8 identical concurrent requests with one operation id run
/// the work once: exactly one commits, the rest are busy with the job or
/// replay its result, and the session gains exactly one revision.
#[test]
fn identical_concurrent_requests_commit_once() -> TestResult {
    for count in [2, 4, 8] {
        let fixture = Fixture::new()?;
        open_session(&fixture, StoredDurability::Ephemeral)?;
        let root = Arc::new(fixture.path.clone());
        let workers: Vec<_> = (0..count)
            .map(|_| {
                let root = Arc::clone(&root);
                thread::spawn(move || concurrent_request(&root).map_err(|error| error.to_string()))
            })
            .collect();
        let mut committed = 0;
        let mut jobs = Vec::new();
        for worker in workers {
            match worker.join().map_err(|_| "worker panicked")?? {
                Ok(outcome) if !outcome.report.replayed => committed += 1,
                Ok(outcome) => jobs.push(outcome.report.job_id),
                Err(JobRunError::Busy { job }) => jobs.push(job),
                Err(JobRunError::Storage(SessionStorageError::Busy)) => {}
                // The variant path tells a job-store failure from a session
                // one; both display the same text.
                Err(other) => return Err(format!("{count}: {other:?}").into()),
            }
        }
        assert_eq!(committed, 1, "{count} requests");
        let store = FilesystemSessionStore::open_existing(&fixture.path)?;
        assert_eq!(transcript_records(&store)?, 1, "{count} requests");
        let expected = resolve_first_job(&store)?;
        assert!(jobs.iter().all(|job| *job == expected), "{count} requests");
        assert_committed_state_is_whole(&store)?;
    }
    Ok(())
}

/// One of several identical concurrent requests: a recognizer slow enough
/// that the requests overlap.
fn concurrent_request(root: &Path) -> Built<Result<RetranscriptionOutcome, JobRunError>> {
    let recognizer = Words {
        pause: Some(Duration::from_millis(20)),
        ..Words::default()
    };
    request_once(root, Some(&operation(OPERATION)?), &recognizer)
}

/// The job the standard request named before anything was committed.
fn resolve_first_job(store: &FilesystemSessionStore) -> Built<JobId> {
    let jobs = store.session_jobs(&session_id()?)?;
    match jobs.as_slice() {
        [view] => Ok(view.record.job_id.clone()),
        _ => Err(format!("expected one job, found {}", jobs.len()).into()),
    }
}

// ---------------------------------------------------------------- X-04

/// X-04: 2, 4 and 8 retranscriptions of different ranges with renewals on
/// one session: nothing deadlocks, every retranscription commits (following
/// the session as it moves) or reports busy, and the chain stays whole with
/// consecutive revision numbers.
#[test]
fn concurrent_retranscriptions_and_renewals_do_not_deadlock() -> TestResult {
    for count in [2_u64, 4, 8] {
        let fixture = Fixture::new()?;
        open_session(&fixture, StoredDurability::Ephemeral)?;
        let root = Arc::new(fixture.path.clone());
        let started = Instant::now();
        let mut workers = Vec::new();
        for number in 0..count {
            let root = Arc::clone(&root);
            workers.push(thread::spawn(move || {
                x04_worker(&root, number).map_err(|error| error.to_string())
            }));
        }
        let mut outcomes = Vec::new();
        for worker in workers {
            outcomes.push(worker.join().map_err(|_| "worker panicked")??);
        }
        assert!(started.elapsed() < Duration::from_secs(60), "{count}");
        let store = FilesystemSessionStore::open_existing(&fixture.path)?;
        assert_committed_state_is_whole(&store)?;
        let committed = outcomes
            .iter()
            .filter(|outcome| *outcome == "committed")
            .count();
        assert_eq!(
            transcript_records(&store)?,
            committed,
            "{count}: {outcomes:?}"
        );
        if let Some((newest, _)) = store.read_transcript(&session_id()?, now()?)? {
            assert_eq!(usize::try_from(newest.number())?, committed, "{count}");
        }
    }
    Ok(())
}

/// One worker of the X-04 test: odd numbers renew the session, even numbers
/// retranscribe their own 10 s range.
fn x04_worker(root: &Path, number: u64) -> Built<String> {
    let store = FilesystemSessionStore::open_existing(root)?;
    if number % 2 == 1 {
        let status = match store.session_status(&session_id()?) {
            Ok(status) => status,
            // A read can meet a concurrent publication and report BUSY, the
            // documented X-04 outcome; it is not a deadlock.
            Err(SessionStorageError::Busy) => return Ok("renewal busy".to_owned()),
            Err(other) => return Err(other.into()),
        };
        return Ok(
            match store.renew_session(
                &session_id()?,
                &operation(&format!("op_{:016x}", 0x9000 + number))?,
                status.generation(),
                now()?,
            ) {
                Ok(_) => "renewed".to_owned(),
                Err(SessionStorageError::Busy | SessionStorageError::StateConflict) => {
                    "renewal busy".to_owned()
                }
                Err(other) => return Err(other.into()),
            },
        );
    }
    let start = number * 12 * SECOND;
    let resolved = match resolve(&store, Some(range(start, start + 10 * SECOND)?)) {
        Ok(resolved) => resolved,
        Err(error)
            if matches!(
                error.downcast_ref::<SessionStorageError>(),
                Some(SessionStorageError::Busy)
            ) =>
        {
            return Ok("busy".to_owned());
        }
        Err(other) => return Err(other),
    };
    let recognizer = Words {
        pause: Some(Duration::from_millis(10)),
        ..Words::default()
    };
    Ok(
        match block_on(run(&store, &resolved, None, &recognizer))?? {
            Ok(_) => "committed".to_owned(),
            Err(
                JobRunError::Busy { .. }
                | JobRunError::Storage(SessionStorageError::Busy)
                | JobRunError::Superseded { .. },
            ) => "busy".to_owned(),
            Err(other) => return Err(other.to_string().into()),
        },
    )
}

/// X-04/X-09: a renewal committed after the request was resolved moves the
/// generation; the commit follows it instead of failing after all the work,
/// and the renewal is kept.
#[test]
fn a_renewal_after_resolution_is_followed_by_the_commit() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let resolved = resolve(&store, requested()?)?;
    let renewed = store.renew_session(
        &session_id()?,
        &operation("op_6666666666666666")?,
        resolved.observed,
        now()?,
    )?;
    let outcome = block_on(run(&store, &resolved, None, &Words::default()))??;
    let outcome = outcome.map_err(|error| error.to_string())?;
    let status = store.session_status(&session_id()?)?;
    assert_eq!(
        status.generation(),
        StorageGeneration::from_value(renewed.value() + 1)
    );
    assert_eq!(outcome.revision.number(), 1);
    assert_committed_state_is_whole(&store)?;
    Ok(())
}

// ---------------------------------------------------------------- X-05

/// X-05: a child process owns the job and then stops making progress (a
/// sleeping stand-in for a suspended process). Its lock is never taken over:
/// the same request is busy with the job, status reports it live, and a
/// cancel is only requested. Once the child is gone the job reconciles and
/// the cancel completes.
#[test]
fn a_suspended_owner_keeps_its_job() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let ready = fixture.path.with_extension("ready");
    let mut child = Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "filesystem_session_store::job_tests::suspended_owner_child",
            "--ignored",
            "--nocapture",
        ])
        .env(CHILD_ROOT, &fixture.path)
        .env(CHILD_READY, &ready)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready.exists() {
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err("the child never took the job".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    let resolved = resolve(&store, requested()?)?;
    let job = resolved.spec.job_id.clone();
    let session = session_id()?;
    let result = (|| -> TestResult {
        let busy = block_on(run(&store, &resolved, None, &Words::default()))??;
        assert_eq!(busy, Err(JobRunError::Busy { job: job.clone() }));
        let view = job_status(&store, &session, &job, now()?)?;
        assert_eq!(view.liveness, JobLiveness::Owned);
        assert_eq!(view.record.state, JobState::Running);
        assert_eq!(
            cancel_job(&store, &session, &job, now()?)?,
            CancelOutcome::Requested
        );
        assert!(matches!(
            store.acquire(&session, &job),
            Err(JobStoreError::Storage(SessionStorageError::Busy))
        ));
        Ok(())
    })();
    child.kill()?;
    child.wait()?;
    let _ = fs::remove_file(&ready);
    result?;
    // The owner is gone: the requested cancellation completes on reconcile.
    let view = job_status(&store, &session, &job, now()?)?;
    assert_eq!(view.record.state, JobState::Cancelled);
    Ok(())
}

/// Child of the suspended-owner test: takes the job, starts an attempt,
/// signals readiness and then makes no progress.
#[test]
#[ignore = "run only as a child process by the suspended-owner test"]
fn suspended_owner_child() -> TestResult {
    let root = PathBuf::from(std::env::var_os(CHILD_ROOT).ok_or("missing root")?);
    let ready = PathBuf::from(std::env::var_os(CHILD_READY).ok_or("missing ready")?);
    let store = FilesystemSessionStore::open_existing(root)?;
    let resolved = resolve(&store, requested()?)?;
    let (mut owner, _) = store.open_or_create(&resolved.spec, now()?)?;
    owner.apply(&JobChange::Start, now()?)?;
    fs::write(ready, b"owned")?;
    thread::sleep(Duration::from_secs(120));
    drop(owner);
    Err("the parent never ended the suspended owner".into())
}

/// X-05 with a real suspension (Unix, opt-in): `SIGSTOP` the owning child;
/// its lock still excludes every other owner.
#[cfg(unix)]
#[test]
#[ignore = "opt-in: suspends a child process with SIGSTOP"]
fn a_sigstopped_owner_keeps_its_job() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let ready = fixture.path.with_extension("ready");
    let mut child = Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "filesystem_session_store::job_tests::suspended_owner_child",
            "--ignored",
            "--nocapture",
        ])
        .env(CHILD_ROOT, &fixture.path)
        .env(CHILD_READY, &ready)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let stopped = Command::new("kill")
        .args(["-STOP", &child.id().to_string()])
        .status()?;
    let resolved = resolve(&store, requested()?)?;
    let outcome = block_on(run(&store, &resolved, None, &Words::default()))??;
    child.kill()?;
    child.wait()?;
    assert!(stopped.success());
    assert_eq!(
        outcome,
        Err(JobRunError::Busy {
            job: resolved.spec.job_id
        })
    );
    Ok(())
}

/// A job's record answers `job_session` only while its session lives, and a
/// committed job's record keeps the revision it published.
#[test]
fn a_succeeded_job_records_its_revision_and_publication() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let outcome = request_once(&fixture.path, None, &Words::default())??;
    let view = store
        .job(&session_id()?, &outcome.report.job_id)?
        .ok_or("no job")?;
    let JobRecord { state, commit, .. } = view.record;
    assert_eq!(state, JobState::Succeeded);
    let commit = commit.ok_or("no commit")?;
    assert_eq!(commit.revision_id, *outcome.revision.id());
    assert_eq!(commit.operation_id, outcome.report.operation_id);
    // The commit's own operation id replays the result.
    let replayed = request_once(&fixture.path, Some(&commit.operation_id), &Words::default())??;
    assert!(replayed.report.replayed);
    assert_eq!(replayed.revision, outcome.revision);
    assert!(Path::new(&job_path(&fixture, &outcome.report.job_id)).exists());
    Ok(())
}
