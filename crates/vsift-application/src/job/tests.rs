//! Jobs over an in-memory store (P10 PR 2): keys, checkpointed recognition,
//! resume, exactly-once commit, the retry table and cancellation (X-01,
//! X-02, X-03, X-06 serialisation, X-09).

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    future::Future,
    num::NonZeroU16,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use proptest::prelude::{ProptestConfig, prop_assert_eq, proptest};
use vsift_domain::{
    AdmissionBudget, AdmissionWait, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, ChunkCheckpoint, ChunkPlan, ChunkTime, CueText, FailureCode, Jitter, JobId,
    JobState, LanguageTag, MediaTime, OperationId, PlannedChunk, ProgressStage, ProgressUpdate,
    ProviderChunkOutput, ProviderSegment, ProviderToken, ProviderTokenKind, SessionId, Sha256Hex,
    SourceId, SourceSegment, SourceSegmentId, StorageGeneration, TimeRange, TranscriptRevision,
    TranscriptRevisionId,
};

use super::{
    CancelOutcome, CancelRequest, CheckpointRead, CheckpointStoreError, ChunkCheckpoints,
    CommitGuard, CommitLedger, JobChange, JobLiveness, JobOwner, JobRecord, JobRequest,
    JobRunError, JobSpec, JobStore, JobStoreError, JobView, RecognitionScope,
    RetranscriptionOutcome, RetranscriptionPorts, RetranscriptionRun, RetryTimer, RevisionStore,
    SessionHead, cancel_job, commit_operation_id, job_id, job_status, recognition_key,
    retranscribe_operation_key, retranscribe_request_digest, run_retranscription,
};
use crate::{
    AsrCancellation, AsrFailureReason, AsrStage, CheckpointScope, NoProgress, ProgressSink,
    RecognizerIdentity, SessionStorageError, SpeechAudioError, SpeechAudioSource, SpeechPcm,
    SpeechRecognitionError, SpeechRecognizer, TranscribeRangeRequest, transcribe_range,
    transcribe_range_checkpointed,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const SESSION: &str = "ses_0123456789abcdef";
const SECOND: u64 = 1_000_000;
const NOW: u64 = 1_790_000_000;

fn session() -> Built<SessionId> {
    Ok(SessionId::parse(SESSION)?)
}

fn source_id() -> Built<SourceId> {
    Ok(SourceId::from_sha256(DIGEST)?)
}

fn identity() -> Built<RecognizerIdentity> {
    Ok(RecognizerIdentity {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(DIGEST)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(DIGEST)?),
        decoding: AsrDecodingProfile::R0V1,
        threads: NonZeroU16::new(4).ok_or("zero")?,
    })
}

fn segment() -> Built<SourceSegment> {
    Ok(SourceSegment::whole_file(
        SourceSegmentId::parse("sgm_0123456789abcdef")?,
        MediaTime::from_micros(120 * SECOND),
    )?)
}

fn range(start: u64, end: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(start),
        MediaTime::from_micros(end),
    )?)
}

fn operation(text: &str) -> Built<OperationId> {
    Ok(OperationId::parse(text)?)
}

// ---------------------------------------------------------------- keys

#[test]
fn keys_are_deterministic_and_separate_what_they_should() -> TestResult {
    let session = session()?;
    let other_session = SessionId::parse("ses_fedcba9876543210")?;
    let whole = retranscribe_request_digest(&session, None)?;
    assert_eq!(whole, retranscribe_request_digest(&session, None)?);
    assert_ne!(
        whole,
        retranscribe_request_digest(&session, Some(range(0, SECOND)?))?
    );
    assert_ne!(whole, retranscribe_request_digest(&other_session, None)?);
    assert_ne!(
        retranscribe_request_digest(&session, Some(range(0, SECOND)?))?,
        retranscribe_request_digest(&session, Some(range(0, 2 * SECOND)?))?
    );

    let source = source_id()?;
    let recognizer = identity()?;
    let scope = RecognitionScope {
        session_id: &session,
        source_id: &source,
        audio_stream: 1,
        replaced_range: range(0, 70 * SECOND)?,
        plan: ChunkPlan::R0,
        recognizer: &recognizer,
        verification: None,
    };
    let key = recognition_key(&scope)?;
    assert_eq!(key, recognition_key(&scope)?);
    for changed in [
        RecognitionScope {
            audio_stream: 2,
            ..scope
        },
        RecognitionScope {
            replaced_range: range(0, 60 * SECOND)?,
            ..scope
        },
        RecognitionScope {
            session_id: &other_session,
            ..scope
        },
    ] {
        assert_ne!(recognition_key(&changed)?, key);
    }
    let fingerprint = crate::MediaToolFingerprint::from_digest([7; 32]);
    assert_ne!(
        recognition_key(&RecognitionScope {
            verification: Some(&fingerprint),
            ..scope
        })?,
        key
    );

    // The base revision is in the operation key, not the recognition key.
    let base = TranscriptRevisionId::parse("trv_0123456789abcdef")?;
    let without = retranscribe_operation_key(&key, None)?;
    let with = retranscribe_operation_key(&key, Some(&base))?;
    assert_ne!(without, with);
    assert!(with.as_str().starts_with("opk_sha256_"));

    let job = job_id(&session, &with)?;
    assert_eq!(job, job_id(&session, &with)?);
    assert_ne!(job, job_id(&session, &without)?);
    assert_ne!(job, job_id(&other_session, &with)?);
    assert_eq!(job.as_str().len(), "job_".len() + 32);

    let first = commit_operation_id(&job, 0, 1)?;
    assert_eq!(first, commit_operation_id(&job, 0, 1)?);
    assert_ne!(first, commit_operation_id(&job, 0, 2)?);
    assert_ne!(first, commit_operation_id(&job, 1, 1)?);
    assert_eq!(first.as_str().len(), "op_".len() + 32);
    Ok(())
}

// ---------------------------------------------------------------- fakes

/// Speech in every chunk, except those listed as silent or without audio.
#[derive(Default)]
struct FakeAudio {
    silent: Vec<u32>,
    empty: Vec<u32>,
    calls: AtomicUsize,
}

impl SpeechAudioSource for FakeAudio {
    fn speech_pcm(
        &self,
        chunk: &PlannedChunk,
    ) -> impl Future<Output = Result<SpeechPcm, SpeechAudioError>> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let samples = usize::try_from(chunk.window().duration_micros() / 1_000 * 16).unwrap_or(0);
        let answer = if self.empty.contains(&chunk.index()) {
            Err(SpeechAudioError::NoAudio)
        } else {
            let level = if self.silent.contains(&chunk.index()) {
                0
            } else {
                3_000
            };
            Ok(SpeechPcm {
                actual_start: chunk.window().start(),
                samples: vec![level; samples],
            })
        };
        std::future::ready(answer)
    }
}

/// What the recognizer does at one chunk.
#[derive(Clone, Copy)]
enum Fault {
    /// Fails with this error.
    Fails(SpeechRecognitionError),
    /// Sets the shared flag (a cancellation or an interruption) first.
    Raises,
    /// Sets the shared flag, then fails with this error: a provider that
    /// died of the same console interrupt that cancelled the caller.
    RaisesAndFails(SpeechRecognitionError),
}

/// A deterministic recognizer: chunk `i` always says the same words.
#[derive(Default)]
struct FakeRecognizer {
    faults: Mutex<BTreeMap<u32, Fault>>,
    flag: Arc<AtomicBool>,
    calls: AtomicUsize,
}

impl FakeRecognizer {
    fn fault_at(&self, chunk: u32, fault: Fault) {
        if let Ok(mut faults) = self.faults.lock() {
            faults.insert(chunk, fault);
        }
    }

    fn clear(&self) {
        if let Ok(mut faults) = self.faults.lock() {
            faults.clear();
        }
    }
}

/// One segment per chunk: 10 s into the first chunk and 6 s (past the 5 s
/// overlap) into later ones, or a third of the way into a shorter window;
/// two seconds long, or a third of the window.
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
                // Not exactly representable in decimal, so a lossy store
                // would change the confidence.
                probability: 0.1 + f64::from(chunk.index()) / 7.0,
            }],
        }],
    })
}

impl SpeechRecognizer for FakeRecognizer {
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
        let fault = self
            .faults
            .lock()
            .ok()
            .and_then(|faults| faults.get(&chunk.index()).copied());
        let answer = match fault {
            Some(Fault::Fails(error)) => Err(error),
            Some(Fault::Raises) => {
                self.flag.store(true, Ordering::SeqCst);
                words(chunk)
            }
            Some(Fault::RaisesAndFails(error)) => {
                self.flag.store(true, Ordering::SeqCst);
                Err(error)
            }
            None => words(chunk),
        };
        std::future::ready(answer)
    }
}

struct Flag(Arc<AtomicBool>);

impl AsrCancellation for Flag {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Never waits; records every backoff.
#[derive(Default)]
struct InstantTimer {
    sleeps: Mutex<Vec<Duration>>,
}

impl InstantTimer {
    fn sleeps(&self) -> Vec<Duration> {
        self.sleeps
            .lock()
            .map(|sleeps| sleeps.clone())
            .unwrap_or_default()
    }
}

impl RetryTimer for InstantTimer {
    fn jitter(&self) -> Jitter {
        Jitter::from_bits(1 << 31)
    }

    fn sleep(&self, delay: Duration) -> impl Future<Output = ()> + Send {
        if let Ok(mut sleeps) = self.sleeps.lock() {
            sleeps.push(delay);
        }
        std::future::ready(())
    }
}

#[derive(Default)]
struct CountingGuard(usize);

impl CommitGuard for CountingGuard {
    fn verify(&mut self) -> Result<(), SessionStorageError> {
        self.0 += 1;
        Ok(())
    }
}

/// A publish the in-memory store is told to answer differently.
#[derive(Clone, Copy, Debug)]
enum PublishFault {
    /// Returns this error without committing.
    Refuses(SessionStorageError),
    /// Commits, then reports an I/O failure (the pointer moved, the
    /// acknowledgement was lost).
    CommitsThenFails,
    /// Another writer commits a plain generation first (a renewal).
    RenewedFirst,
}

#[derive(Default)]
struct Shared {
    jobs: BTreeMap<String, JobRecord>,
    owners: BTreeSet<String>,
    by_operation: BTreeMap<String, JobId>,
    checkpoints: BTreeMap<(String, u32), ChunkCheckpoint>,
    chain: Vec<(u64, String)>,
    revisions: Vec<TranscriptRevision>,
    publish_faults: VecDeque<PublishFault>,
    busy_admissions: u32,
    admitted_weights: Vec<u16>,
    publishes: usize,
}

impl Shared {
    fn generation(&self) -> u64 {
        u64::try_from(self.chain.len()).unwrap_or(u64::MAX)
    }
}

#[derive(Clone, Default)]
struct MemoryStore(Arc<Mutex<Shared>>);

impl MemoryStore {
    fn lock(&self) -> Result<MutexGuard<'_, Shared>, SessionStorageError> {
        self.0.lock().map_err(|_| SessionStorageError::Io)
    }

    fn with<T>(&self, action: impl FnOnce(&mut Shared) -> T) -> Built<T> {
        let mut shared = self.0.lock().map_err(|_| "poisoned")?;
        Ok(action(&mut shared))
    }

    fn view(shared: &Shared, record: &JobRecord) -> JobView {
        JobView {
            record: record.clone(),
            liveness: if shared.owners.contains(record.job_id.as_str()) {
                JobLiveness::Owned
            } else {
                JobLiveness::Unowned
            },
            checkpoints: shared
                .checkpoints
                .keys()
                .filter(|(job, _)| job == record.job_id.as_str())
                .count(),
        }
    }
}

struct MemoryOwner {
    shared: Arc<Mutex<Shared>>,
    record: JobRecord,
}

impl Drop for MemoryOwner {
    fn drop(&mut self) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.owners.remove(self.record.job_id.as_str());
        }
    }
}

impl MemoryOwner {
    fn job(&self) -> String {
        self.record.job_id.as_str().to_owned()
    }
}

impl ChunkCheckpoints for MemoryOwner {
    fn load(&self, index: u32) -> CheckpointRead {
        match self.shared.lock() {
            Ok(shared) => shared
                .checkpoints
                .get(&(self.job(), index))
                .cloned()
                .map_or(CheckpointRead::Absent, CheckpointRead::Found),
            Err(_) => CheckpointRead::Absent,
        }
    }

    fn store(&self, checkpoint: &ChunkCheckpoint) -> Result<(), CheckpointStoreError> {
        let mut shared = self
            .shared
            .lock()
            .map_err(|_| CheckpointStoreError::Storage(SessionStorageError::Io))?;
        shared
            .checkpoints
            .insert((self.job(), checkpoint.index()), checkpoint.clone());
        Ok(())
    }

    fn discard(&self, index: u32) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.checkpoints.remove(&(self.job(), index));
        }
    }
}

impl JobOwner for MemoryOwner {
    fn record(&self) -> &JobRecord {
        &self.record
    }

    fn apply(&mut self, change: &JobChange, now: u64) -> Result<(), JobStoreError> {
        let mut shared = self.shared.lock().map_err(|_| SessionStorageError::Io)?;
        let job = self.job();
        let stored = shared.jobs.get(&job).ok_or(JobStoreError::NotFound)?;
        if stored.epoch != self.record.epoch || stored.attempt != self.record.attempt {
            return Err(JobStoreError::StaleOwner);
        }
        let next = stored.changed(change, now)?;
        if change.discards_checkpoints() {
            shared.checkpoints.retain(|(owner, _), _| *owner != job);
        }
        shared.jobs.insert(job, next.clone());
        self.record = next;
        Ok(())
    }

    fn bind_operation(
        &mut self,
        operation_id: &OperationId,
        now: u64,
    ) -> Result<(), JobStoreError> {
        let mut shared = self.shared.lock().map_err(|_| SessionStorageError::Io)?;
        shared
            .by_operation
            .insert(operation_id.as_str().to_owned(), self.record.job_id.clone());
        let job = self.job();
        if let Some(stored) = shared.jobs.get_mut(&job)
            && !stored.operation_ids.contains(operation_id)
        {
            stored.operation_ids.push(operation_id.clone());
            stored.updated_at_unix_seconds = now;
            self.record = stored.clone();
        }
        Ok(())
    }

    fn cancel_requested(&self) -> bool {
        self.shared.lock().is_ok_and(|shared| {
            shared
                .jobs
                .get(&self.job())
                .is_some_and(|record| record.state == JobState::Cancelling)
        })
    }
}

impl JobStore for MemoryStore {
    type Owner = MemoryOwner;

    fn job(
        &self,
        _session_id: &SessionId,
        job_id: &JobId,
    ) -> Result<Option<JobView>, JobStoreError> {
        let shared = self.lock()?;
        Ok(shared
            .jobs
            .get(job_id.as_str())
            .map(|record| Self::view(&shared, record)))
    }

    fn job_session(&self, job_id: &JobId) -> Result<Option<SessionId>, JobStoreError> {
        let shared = self.lock()?;
        Ok(shared
            .jobs
            .get(job_id.as_str())
            .map(|record| record.session_id.clone()))
    }

    fn job_for_operation(
        &self,
        _session_id: &SessionId,
        operation_id: &OperationId,
    ) -> Result<Option<JobId>, JobStoreError> {
        Ok(self
            .lock()?
            .by_operation
            .get(operation_id.as_str())
            .cloned())
    }

    fn session_jobs(&self, _session_id: &SessionId) -> Result<Vec<JobView>, JobStoreError> {
        let shared = self.lock()?;
        Ok(shared
            .jobs
            .values()
            .map(|record| Self::view(&shared, record))
            .collect())
    }

    fn open_or_create(
        &self,
        spec: &JobSpec,
        now: u64,
    ) -> Result<(Self::Owner, bool), JobStoreError> {
        let mut shared = self.lock()?;
        let key = spec.job_id.as_str().to_owned();
        if shared.owners.contains(&key) {
            return Err(JobStoreError::Storage(SessionStorageError::Busy));
        }
        let created = !shared.jobs.contains_key(&key);
        if created {
            shared
                .jobs
                .insert(key.clone(), JobRecord::queued(spec, now));
        }
        let record = shared
            .jobs
            .get(&key)
            .cloned()
            .ok_or(JobStoreError::NotFound)?;
        shared.owners.insert(key);
        Ok((
            MemoryOwner {
                shared: Arc::clone(&self.0),
                record,
            },
            created,
        ))
    }

    fn acquire(
        &self,
        _session_id: &SessionId,
        job_id: &JobId,
    ) -> Result<Self::Owner, JobStoreError> {
        let mut shared = self.lock()?;
        let key = job_id.as_str().to_owned();
        if shared.owners.contains(&key) {
            return Err(JobStoreError::Storage(SessionStorageError::Busy));
        }
        let record = shared
            .jobs
            .get(&key)
            .cloned()
            .ok_or(JobStoreError::NotFound)?;
        shared.owners.insert(key);
        Ok(MemoryOwner {
            shared: Arc::clone(&self.0),
            record,
        })
    }

    fn request_cancel(
        &self,
        _session_id: &SessionId,
        job_id: &JobId,
        now: u64,
    ) -> Result<CancelRequest, JobStoreError> {
        let mut shared = self.lock()?;
        let live = shared.owners.contains(job_id.as_str());
        let record = shared
            .jobs
            .get_mut(job_id.as_str())
            .ok_or(JobStoreError::NotFound)?;
        if record.state.is_terminal() {
            return Ok(CancelRequest::Terminal(record.state));
        }
        if !live {
            return Ok(CancelRequest::NotLive);
        }
        match record.state {
            JobState::Committing => Ok(CancelRequest::TooLate),
            JobState::Cancelling => Ok(CancelRequest::AlreadyRequested),
            _ => {
                *record = record.changed(&JobChange::RequestCancel, now)?;
                Ok(CancelRequest::Requested)
            }
        }
    }
}

impl CommitLedger for MemoryStore {
    fn committed_generation(
        &self,
        _session_id: &SessionId,
        operation_id: &OperationId,
        above: StorageGeneration,
    ) -> Result<Option<StorageGeneration>, SessionStorageError> {
        Ok(self
            .lock()?
            .chain
            .iter()
            .find(|(generation, operation)| {
                *generation > above.value() && operation == operation_id.as_str()
            })
            .map(|(generation, _)| StorageGeneration::from_value(*generation)))
    }
}

impl RevisionStore for MemoryStore {
    type Permit = ();

    fn admit(&self, weight: NonZeroU16) -> Result<Self::Permit, SessionStorageError> {
        let mut shared = self.lock()?;
        shared.admitted_weights.push(weight.get());
        if shared.busy_admissions > 0 {
            shared.busy_admissions -= 1;
            return Err(SessionStorageError::Busy);
        }
        Ok(())
    }

    fn head(&self, _session_id: &SessionId, _now: u64) -> Result<SessionHead, SessionStorageError> {
        let shared = self.lock()?;
        Ok(SessionHead {
            generation: StorageGeneration::from_value(shared.generation()),
            newest: shared.revisions.last().cloned(),
        })
    }

    fn revision(
        &self,
        _session_id: &SessionId,
        revision_id: &TranscriptRevisionId,
        _now: u64,
    ) -> Result<Option<TranscriptRevision>, SessionStorageError> {
        Ok(self
            .lock()?
            .revisions
            .iter()
            .find(|revision| revision.id() == revision_id)
            .cloned())
    }

    fn publish_revision(
        &self,
        _session_id: &SessionId,
        operation_id: &OperationId,
        expected: StorageGeneration,
        revision: &TranscriptRevision,
        _now: u64,
    ) -> Result<StorageGeneration, SessionStorageError> {
        let mut shared = self.lock()?;
        shared.publishes += 1;
        let fault = shared.publish_faults.pop_front();
        match fault {
            Some(PublishFault::Refuses(error)) => return Err(error),
            Some(PublishFault::RenewedFirst) => {
                let generation = shared.generation() + 1;
                shared
                    .chain
                    .push((generation, "op_renewal0000000000".to_owned()));
            }
            Some(PublishFault::CommitsThenFails) | None => {}
        }
        if shared.generation() != expected.value() {
            return Err(SessionStorageError::StateConflict);
        }
        let generation = shared.generation() + 1;
        shared
            .chain
            .push((generation, operation_id.as_str().to_owned()));
        shared.revisions.push(revision.clone());
        if matches!(fault, Some(PublishFault::CommitsThenFails)) {
            return Err(SessionStorageError::Io);
        }
        Ok(StorageGeneration::from_value(generation))
    }
}

// ---------------------------------------------------------------- harness

/// One session with the fakes; resolves a request the way the engine does.
struct Harness {
    store: MemoryStore,
    audio: FakeAudio,
    recognizer: FakeRecognizer,
    timer: InstantTimer,
    source: SourceSegment,
    source_id: SourceId,
    identity: RecognizerIdentity,
    admission: AdmissionWait,
}

/// A request resolved against the session's current head.
struct Resolved {
    spec: JobSpec,
    replaced: TimeRange,
    base: Option<TranscriptRevision>,
    observed: StorageGeneration,
    requested: Option<TimeRange>,
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
            AsrFailureReason::Cancelled => FailureCode::Cancelled,
            AsrFailureReason::Deadline => FailureCode::DeadlineExceeded,
            AsrFailureReason::Busy => FailureCode::Busy,
            _ => FailureCode::MissingCapability,
        },
        JobRunError::Storage(_) | JobRunError::Job(_) => FailureCode::StorageIo,
        JobRunError::Assembly(_) | JobRunError::Key(_) => FailureCode::Internal,
    }
}

impl Harness {
    fn new() -> Built<Self> {
        Ok(Self {
            store: MemoryStore::default(),
            audio: FakeAudio::default(),
            recognizer: FakeRecognizer::default(),
            timer: InstantTimer::default(),
            source: segment()?,
            source_id: source_id()?,
            identity: identity()?,
            admission: AdmissionWait::Immediate,
        })
    }

    fn resolve(&self, requested: Option<TimeRange>) -> Built<Resolved> {
        let session = session()?;
        let head = self.store.head(&session, NOW)?;
        let replaced =
            crate::retranscription_range(head.newest.as_ref(), requested, self.source.range());
        let key = recognition_key(&RecognitionScope {
            session_id: &session,
            source_id: &self.source_id,
            audio_stream: 1,
            replaced_range: replaced,
            plan: ChunkPlan::R0,
            recognizer: &self.identity,
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
        &self,
        resolved: &Resolved,
        operation_id: Option<&OperationId>,
    ) -> Result<RetranscriptionOutcome, JobRunError> {
        let flag = Flag(Arc::clone(&self.recognizer.flag));
        let mut guard = CountingGuard::default();
        run_retranscription(
            RetranscriptionRun {
                spec: &resolved.spec,
                operation_id,
                transcribe: TranscribeRangeRequest {
                    source_segment: &self.source,
                    range: resolved.replaced,
                    plan: ChunkPlan::R0,
                    audio_stream: 1,
                    expected: &self.identity,
                },
                source_id: &self.source_id,
                requested: resolved.requested,
                base: resolved.base.as_ref(),
                observed: resolved.observed,
                now: NOW,
                admission: self.admission,
            },
            RetranscriptionPorts {
                store: &self.store,
                audio: &self.audio,
                recognizer: &self.recognizer,
                cancellation: &flag,
                timer: &self.timer,
                classify,
                progress: &NoProgress,
            },
            &mut guard,
        )
        .await
    }

    async fn request(
        &self,
        requested: Option<TimeRange>,
        operation_id: Option<&OperationId>,
    ) -> Built<Result<RetranscriptionOutcome, JobRunError>> {
        let resolved = self.resolve(requested)?;
        Ok(self.run(&resolved, operation_id).await)
    }

    fn record(&self, job: &JobId) -> Built<JobRecord> {
        self.store
            .with(|shared| shared.jobs.get(job.as_str()).cloned())?
            .ok_or_else(|| "no such job".into())
    }

    fn checkpoints(&self, job: &JobId) -> Built<usize> {
        self.store.with(|shared| {
            shared
                .checkpoints
                .keys()
                .filter(|(owner, _)| owner == job.as_str())
                .count()
        })
    }

    fn publishes(&self) -> Built<usize> {
        self.store.with(|shared| shared.publishes)
    }

    fn generation(&self) -> Built<u64> {
        self.store.with(|shared| shared.generation())
    }
}

// ---------------------------------------------------------------- checkpoints

/// Records every progress report.
#[derive(Default)]
struct RecordedProgress(Mutex<Vec<ProgressUpdate>>);

impl ProgressSink for RecordedProgress {
    fn report(&self, update: ProgressUpdate) {
        if let Ok(mut reports) = self.0.lock() {
            reports.push(update);
        }
    }
}

impl RecordedProgress {
    /// The reports so far, emptied.
    fn take(&self) -> Built<Vec<(u64, Option<u64>)>> {
        let mut reports = self.0.lock().map_err(|_| "progress lock poisoned")?;
        Ok(reports
            .drain(..)
            .map(|update| {
                assert_eq!(update.stage, ProgressStage::RecognisingSpeech);
                (update.completed, update.total)
            })
            .collect())
    }
}

/// Checkpoints are written per chunk; a second run over all of them decodes
/// and recognises nothing and returns the same transcription. Both report
/// their progress chunk by chunk (P11): 0 of 3 once planned, then each
/// chunk, reused or not.
#[tokio::test]
async fn a_run_over_its_own_checkpoints_decodes_and_recognises_nothing() -> TestResult {
    let harness = Harness::new()?;
    let resolved = harness.resolve(None)?;
    let (owner, _) = harness.store.open_or_create(&resolved.spec, NOW)?;
    let request = TranscribeRangeRequest {
        source_segment: &harness.source,
        range: range(0, 70 * SECOND)?,
        plan: ChunkPlan::R0,
        audio_stream: 1,
        expected: &harness.identity,
    };
    let progress = RecordedProgress::default();
    let scope = CheckpointScope {
        checkpoints: &owner,
        key: &resolved.spec.recognition_key,
        progress: &progress,
    };
    let every_chunk = [(0, Some(3)), (1, Some(3)), (2, Some(3)), (3, Some(3))];
    let never = Flag(Arc::new(AtomicBool::new(false)));
    let (first, used) =
        transcribe_range_checkpointed(request, scope, &harness.audio, &harness.recognizer, &never)
            .await?;
    assert_eq!(used.reused, 0);
    assert_eq!(harness.checkpoints(&resolved.spec.job_id)?, 3);
    assert_eq!(progress.take()?, every_chunk);
    let decoded = harness.audio.calls.load(Ordering::SeqCst);
    let recognised = harness.recognizer.calls.load(Ordering::SeqCst);

    let (second, used) =
        transcribe_range_checkpointed(request, scope, &harness.audio, &harness.recognizer, &never)
            .await?;
    assert_eq!(used.reused, 3);
    assert_eq!(used.discarded, 0);
    assert_eq!(second, first);
    assert_eq!(harness.audio.calls.load(Ordering::SeqCst), decoded);
    assert_eq!(harness.recognizer.calls.load(Ordering::SeqCst), recognised);
    assert_eq!(progress.take()?, every_chunk);

    // Without checkpoints the same range gives the same transcription.
    let plain = transcribe_range(request, &harness.audio, &harness.recognizer, &never).await?;
    assert_eq!(plain, first);
    Ok(())
}

/// S-08 at the use-case level: a checkpoint of another run, one whose audio
/// cannot be the chunk's and one whose output the domain rejects are all
/// removed and redone, never used.
#[tokio::test]
async fn foreign_or_invalid_checkpoints_are_discarded_and_redone() -> TestResult {
    let harness = Harness::new()?;
    let resolved = harness.resolve(None)?;
    let (owner, _) = harness.store.open_or_create(&resolved.spec, NOW)?;
    let request = TranscribeRangeRequest {
        source_segment: &harness.source,
        range: range(0, 70 * SECOND)?,
        plan: ChunkPlan::R0,
        audio_stream: 1,
        expected: &harness.identity,
    };
    let never = Flag(Arc::new(AtomicBool::new(false)));
    let scope = CheckpointScope {
        checkpoints: &owner,
        key: &resolved.spec.recognition_key,
        progress: &NoProgress,
    };
    let (control, _) =
        transcribe_range_checkpointed(request, scope, &harness.audio, &harness.recognizer, &never)
            .await?;

    let chunks = vsift_domain::plan_chunks(harness.source.id(), request.range, ChunkPlan::R0)?;
    let foreign = vsift_domain::RecognitionKey::new(Sha256Hex::parse("f".repeat(64))?);
    let mut forged = words(&chunks[2]).map_err(|_| "no words")?;
    forged.segments[0].tokens[0].probability = 7.0;
    for checkpoint in [
        ChunkCheckpoint::new(
            foreign,
            &chunks[0],
            vsift_domain::CheckpointOutcome::NoAudio,
        ),
        ChunkCheckpoint::new(
            resolved.spec.recognition_key.clone(),
            &chunks[1],
            vsift_domain::CheckpointOutcome::Silent {
                audio: range(90 * SECOND, 95 * SECOND)?,
            },
        ),
        ChunkCheckpoint::new(
            resolved.spec.recognition_key.clone(),
            &chunks[2],
            vsift_domain::CheckpointOutcome::Recognised {
                audio: chunks[2].window(),
                output: forged,
            },
        ),
    ] {
        owner.store(&checkpoint)?;
    }
    let recognised = harness.recognizer.calls.load(Ordering::SeqCst);
    let (redone, used) =
        transcribe_range_checkpointed(request, scope, &harness.audio, &harness.recognizer, &never)
            .await?;
    assert_eq!(used.reused, 0);
    assert_eq!(used.discarded, 3);
    assert_eq!(redone, control);
    assert_eq!(
        harness.recognizer.calls.load(Ordering::SeqCst),
        recognised + 3
    );
    // The redone chunks were stored again, as this run's.
    for index in 0..3 {
        assert!(matches!(
            owner.load(index),
            CheckpointRead::Found(checkpoint)
                if checkpoint.key() == &resolved.spec.recognition_key
        ));
    }
    Ok(())
}

/// #332: the floor is judged before anything stored. A checkpoint that holds
/// recognised output, or a decoded range, for a window under 100 ms was stored
/// before the rule existed; a run that finds one does not reuse it. It is
/// discarded, the chunk is the gap the rule gives, and nothing is decoded or
/// recognised. The gap the rule itself stores is reused like any other.
#[tokio::test]
async fn a_checkpoint_stored_for_a_window_under_the_floor_is_not_reused_as_speech() -> TestResult {
    let harness = Harness::new()?;
    let resolved = harness.resolve(None)?;
    let (owner, _) = harness.store.open_or_create(&resolved.spec, NOW)?;
    let request = TranscribeRangeRequest {
        source_segment: &harness.source,
        range: range(2 * SECOND, 2 * SECOND + 50_000)?,
        plan: ChunkPlan::R0,
        audio_stream: 1,
        expected: &harness.identity,
    };
    let never = Flag(Arc::new(AtomicBool::new(false)));
    let scope = CheckpointScope {
        checkpoints: &owner,
        key: &resolved.spec.recognition_key,
        progress: &NoProgress,
    };
    let chunks = vsift_domain::plan_chunks(harness.source.id(), request.range, ChunkPlan::R0)?;
    let [chunk] = chunks.as_slice() else {
        return Err("a range of 50 ms is one chunk".into());
    };
    assert!(chunk.is_below_recognition_floor());

    for stored in [
        vsift_domain::CheckpointOutcome::Recognised {
            audio: chunk.window(),
            output: words(chunk).map_err(|_| "no words")?,
        },
        vsift_domain::CheckpointOutcome::Silent {
            audio: chunk.window(),
        },
    ] {
        owner.store(&ChunkCheckpoint::new(
            resolved.spec.recognition_key.clone(),
            chunk,
            stored,
        ))?;
        let (transcription, used) = transcribe_range_checkpointed(
            request,
            scope,
            &harness.audio,
            &harness.recognizer,
            &never,
        )
        .await?;
        assert_eq!((used.reused, used.discarded), (0, 1));
        assert!(transcription.segments.is_empty());
        assert_eq!(
            transcription
                .run
                .chunks()
                .first()
                .map(vsift_domain::AsrChunkRecord::outcome),
            Some(vsift_domain::AsrChunkOutcome::NoAudio)
        );
        // What the run stored in its place is the gap, as this run's.
        let CheckpointRead::Found(replacement) = owner.load(chunk.index()) else {
            return Err("the gap was not stored".into());
        };
        assert_eq!(replacement.key(), &resolved.spec.recognition_key);
        assert!(matches!(
            replacement.into_outcome(),
            vsift_domain::CheckpointOutcome::NoAudio
        ));
    }

    // The stored gap is the rule's own answer and is reused.
    let (again, used) =
        transcribe_range_checkpointed(request, scope, &harness.audio, &harness.recognizer, &never)
            .await?;
    assert_eq!((used.reused, used.discarded), (1, 0));
    assert!(again.segments.is_empty());
    assert_eq!(harness.audio.calls.load(Ordering::SeqCst), 0);
    assert_eq!(harness.recognizer.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

/// Silent and empty chunks are checkpointed too, and resumed as gaps.
#[tokio::test]
async fn silent_and_empty_chunks_resume_as_gaps() -> TestResult {
    let mut harness = Harness::new()?;
    harness.audio = FakeAudio {
        silent: vec![1],
        empty: vec![2],
        calls: AtomicUsize::new(0),
    };
    let first = harness.request(None, None).await??;
    let control = first.revision.clone();

    // A second session-free run of the same keys from scratch, interrupted
    // after chunk 0, then resumed, gives the same revision.
    let other = Harness {
        audio: FakeAudio {
            silent: vec![1],
            empty: vec![2],
            calls: AtomicUsize::new(0),
        },
        ..Harness::new()?
    };
    other.recognizer.fault_at(0, Fault::Raises);
    let interrupted = other.request(None, None).await?;
    assert!(interrupted.is_err());
    other.recognizer.clear();
    other.recognizer.flag.store(false, Ordering::SeqCst);
    let resumed = other.request(None, None).await??;
    assert_eq!(resumed.revision, control);
    assert!(resumed.report.resumed);
    Ok(())
}

// ---------------------------------------------------------------- resume

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        ..ProptestConfig::default()
    })]

    /// X-01: interrupting a run at any chunk and resuming it gives exactly
    /// the revision an uninterrupted run gives, reusing the chunks finished
    /// before the interruption.
    #[test]
    fn an_interrupted_run_resumes_to_the_same_revision(chunk in 0_u32..5, cancel in proptest::bool::ANY) {
        let result: Built<()> = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(Into::into)
            .and_then(|runtime| runtime.block_on(async {
                let requested = Some(range(0, 115 * SECOND)?);
                let control = Harness::new()?.request(requested, None).await??;
                let harness = Harness::new()?;
                let fault = if cancel {
                    Fault::Raises
                } else {
                    Fault::Fails(SpeechRecognitionError::Deadline)
                };
                harness.recognizer.fault_at(chunk, fault);
                let failure = harness.request(requested, None).await?;
                if failure.is_ok() {
                    return Err("the interrupted run committed".into());
                }
                harness.recognizer.clear();
                harness.recognizer.flag.store(false, Ordering::SeqCst);
                let resolved = harness.resolve(requested)?;
                assert_eq!(harness.record(&resolved.spec.job_id)?.state, JobState::Interrupted);
                let recognised = harness.recognizer.calls.load(Ordering::SeqCst);
                let resumed = harness.run(&resolved, None).await?;
                assert_eq!(&resumed.revision, &control.revision);
                assert!(resumed.report.resumed || chunk == 0);
                // A cancellation lands after the chunk was stored; a failure
                // before.
                let finished = if cancel { chunk + 1 } else { chunk };
                assert_eq!(resumed.report.chunks_reused, finished);
                let redone = harness.recognizer.calls.load(Ordering::SeqCst) - recognised;
                assert_eq!(redone, usize::try_from(5 - finished)?);
                assert_eq!(harness.checkpoints(&resolved.spec.job_id)?, 0);
                Ok(())
            }));
        prop_assert_eq!(result.map_err(|error| error.to_string()), Ok(()));
    }
}

// ---------------------------------------------------------------- X-02 / X-03

/// X-02: a commit whose acknowledgement was lost is found in the chain; a
/// retry with the same operation id replays it without a new generation.
#[tokio::test]
async fn a_lost_acknowledgement_replays_without_a_new_generation() -> TestResult {
    let harness = Harness::new()?;
    let op = operation("op_aaaaaaaaaaaaaaaa")?;
    harness.store.with(|shared| {
        shared
            .publish_faults
            .push_back(PublishFault::CommitsThenFails);
    })?;
    let first = harness.request(None, Some(&op)).await??;
    assert!(!first.report.replayed);
    assert_eq!(first.report.operation_id, op);
    assert_eq!(harness.generation()?, 1);
    assert_eq!(
        harness.record(&first.report.job_id)?.state,
        JobState::Succeeded
    );

    let replayed = harness.request(None, Some(&op)).await??;
    assert!(replayed.report.replayed);
    assert_eq!(replayed.revision, first.revision);
    assert_eq!(replayed.report.job_id, first.report.job_id);
    assert_eq!(harness.generation()?, 1);
    assert_eq!(harness.publishes()?, 1);
    Ok(())
}

/// X-02: a crash between the pointer and the job record leaves the record
/// committing; the retry reconciles it from the chain and replays.
#[tokio::test]
async fn a_crash_after_the_pointer_is_reconciled_from_the_chain() -> TestResult {
    let harness = Harness::new()?;
    let op = operation("op_bbbbbbbbbbbbbbbb")?;
    let first = harness.request(None, Some(&op)).await??;
    let job = first.report.job_id.clone();
    // Put the record back to what a crash just after the pointer rename
    // leaves: committing, with the commit it was about to make.
    harness.store.with(|shared| {
        if let Some(record) = shared.jobs.get_mut(job.as_str()) {
            record.state = JobState::Committing;
        }
    })?;
    let replayed = harness.request(None, Some(&op)).await??;
    assert!(replayed.report.replayed);
    assert_eq!(replayed.revision, first.revision);
    assert_eq!(harness.record(&job)?.state, JobState::Succeeded);
    assert_eq!(harness.publishes()?, 1);

    // Without an operation id the request is a new one: the session's
    // newest revision is now the base, so it is a new job and revision.
    let again = harness.request(None, None).await??;
    assert!(!again.report.replayed);
    assert_ne!(again.report.job_id, job);
    assert_eq!(again.revision.number(), 2);
    Ok(())
}

/// X-03: the same operation id for a different range is a conflict and
/// changes nothing; a live owner makes an identical request busy.
#[tokio::test]
async fn an_operation_id_reused_for_another_request_conflicts() -> TestResult {
    let harness = Harness::new()?;
    let op = operation("op_cccccccccccccccc")?;
    let first = harness
        .request(Some(range(0, 40 * SECOND)?), Some(&op))
        .await??;
    let conflict = harness
        .request(Some(range(0, 50 * SECOND)?), Some(&op))
        .await?;
    assert_eq!(
        conflict,
        Err(JobRunError::IdempotencyConflict {
            job: first.report.job_id.clone()
        })
    );
    assert_eq!(harness.generation()?, 1);

    // A live owner of the same job: busy, naming the job.
    let resolved = harness.resolve(Some(range(0, 40 * SECOND)?))?;
    let held = harness.store.open_or_create(&resolved.spec, NOW)?;
    let busy = harness.run(&resolved, None).await;
    assert_eq!(
        busy,
        Err(JobRunError::Busy {
            job: resolved.spec.job_id.clone()
        })
    );
    drop(held);
    Ok(())
}

// ---------------------------------------------------------- X-07 admission

/// Counts the admission waits a run announces.
#[derive(Default)]
struct WaitRecorder(AtomicUsize);

impl ProgressSink for WaitRecorder {
    fn report(&self, _update: ProgressUpdate) {}

    fn admission_waiting(&self, weight: NonZeroU16) {
        assert_eq!(weight.get(), 4, "the recognizer's threads");
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl Harness {
    /// Runs the whole-range request with `admission` and `progress`.
    async fn run_admitted(
        &self,
        admission: AdmissionWait,
        progress: &dyn ProgressSink,
    ) -> Built<Result<RetranscriptionOutcome, JobRunError>> {
        let resolved = self.resolve(None)?;
        let flag = Flag(Arc::clone(&self.recognizer.flag));
        let mut guard = CountingGuard::default();
        Ok(run_retranscription(
            RetranscriptionRun {
                spec: &resolved.spec,
                operation_id: None,
                transcribe: TranscribeRangeRequest {
                    source_segment: &self.source,
                    range: resolved.replaced,
                    plan: ChunkPlan::R0,
                    audio_stream: 1,
                    expected: &self.identity,
                },
                source_id: &self.source_id,
                requested: None,
                base: None,
                observed: resolved.observed,
                now: NOW,
                admission,
            },
            RetranscriptionPorts {
                store: &self.store,
                audio: &self.audio,
                recognizer: &self.recognizer,
                cancellation: &flag,
                timer: &self.timer,
                classify,
                progress,
            },
            &mut guard,
        )
        .await)
    }
}

/// X-07: an attempt reserves the recognizer's threads, not one slot; a
/// bounded wait polls with jittered backoff, says once that it waits,
/// reports the time it waited and then runs.
#[tokio::test]
async fn a_bounded_admission_wait_polls_until_the_capacity_frees() -> TestResult {
    let harness = Harness::new()?;
    harness.store.with(|shared| shared.busy_admissions = 3)?;
    let waits = WaitRecorder::default();
    let budget = AdmissionBudget::new(Duration::from_secs(5))?;
    let committed = harness
        .run_admitted(AdmissionWait::Bounded(budget), &waits)
        .await??;
    assert_eq!(committed.revision.number(), 1);
    // Full jitter at one half: 25, 50 and 100 ms below 50, 100 and 200 ms.
    let polls = vec![
        Duration::from_millis(25),
        Duration::from_millis(50),
        Duration::from_millis(100),
    ];
    assert_eq!(harness.timer.sleeps(), polls);
    assert_eq!(committed.report.admission_wait, Duration::from_millis(175));
    assert_eq!(waits.0.load(Ordering::SeqCst), 1);
    assert_eq!(
        harness
            .store
            .with(|shared| shared.admitted_weights.clone())?,
        vec![4, 4, 4, 4]
    );
    Ok(())
}

/// X-07: a bounded wait never waits longer than its budget; then it
/// answers busy with a retry hint, leaves the job resumable and is not
/// retried automatically (the budget is spent).
#[tokio::test]
async fn admission_wait_is_bounded_then_busy() -> TestResult {
    let harness = Harness::new()?;
    harness
        .store
        .with(|shared| shared.busy_admissions = u32::MAX)?;
    let budget = AdmissionBudget::new(Duration::from_millis(900))?;
    let resolved = harness.resolve(None)?;
    let outcome = harness
        .run_admitted(AdmissionWait::Bounded(budget), &NoProgress)
        .await?;
    assert_eq!(
        outcome,
        Err(JobRunError::AdmissionBusy {
            job: resolved.spec.job_id.clone(),
            retry_after: Duration::from_secs(2),
        })
    );
    let slept: Duration = harness.timer.sleeps().iter().sum();
    assert_eq!(slept, budget.duration());
    let record = harness.record(&resolved.spec.job_id)?;
    assert_eq!(record.state, JobState::Interrupted);
    Ok(())
}

/// An immediate wait keeps P10's behaviour: contention is retried at most
/// twice by the retry policy, never polled.
#[tokio::test]
async fn an_immediate_admission_keeps_the_bounded_retries() -> TestResult {
    let harness = Harness::new()?;
    harness
        .store
        .with(|shared| shared.busy_admissions = u32::MAX)?;
    let outcome = harness
        .run_admitted(AdmissionWait::Immediate, &NoProgress)
        .await?;
    assert_eq!(
        outcome,
        Err(JobRunError::Storage(SessionStorageError::Busy))
    );
    assert_eq!(
        harness.timer.sleeps(),
        vec![Duration::from_millis(100), Duration::from_millis(200)]
    );
    Ok(())
}

// ---------------------------------------------------------------- X-09 retries

/// X-09: busy admission and a busy writer are retried at most twice with
/// jittered backoff; a third busy leaves the job resumable.
#[tokio::test]
async fn busy_contention_is_retried_twice_then_left_resumable() -> TestResult {
    let harness = Harness::new()?;
    harness.store.with(|shared| {
        shared.busy_admissions = 1;
        shared
            .publish_faults
            .push_back(PublishFault::Refuses(SessionStorageError::Busy));
    })?;
    let committed = harness.request(None, None).await??;
    assert_eq!(committed.revision.number(), 1);
    assert_eq!(
        harness.timer.sleeps(),
        vec![Duration::from_millis(100), Duration::from_millis(200)]
    );

    let other = Harness::new()?;
    other.store.with(|shared| {
        for _ in 0..3 {
            shared
                .publish_faults
                .push_back(PublishFault::Refuses(SessionStorageError::Busy));
        }
    })?;
    let resolved = other.resolve(None)?;
    let busy = other.run(&resolved, None).await;
    assert_eq!(busy, Err(JobRunError::Storage(SessionStorageError::Busy)));
    assert_eq!(other.timer.sleeps().len(), 2);
    let record = other.record(&resolved.spec.job_id)?;
    assert_eq!(record.state, JobState::Interrupted);
    assert_eq!(other.checkpoints(&resolved.spec.job_id)?, 5);
    // The next request resumes from every checkpoint and commits.
    let recognised = other.recognizer.calls.load(Ordering::SeqCst);
    let resumed = other.run(&resolved, None).await?;
    assert_eq!(resumed.report.chunks_reused, 5);
    assert_eq!(other.recognizer.calls.load(Ordering::SeqCst), recognised);
    Ok(())
}

/// X-09: a generation moved by a renewal is followed: the commit is retried
/// against the new head instead of failing after all the work.
#[tokio::test]
async fn a_renewal_during_the_run_is_followed_not_reported_busy() -> TestResult {
    let harness = Harness::new()?;
    harness
        .store
        .with(|shared| shared.publish_faults.push_back(PublishFault::RenewedFirst))?;
    let committed = harness.request(None, None).await??;
    assert_eq!(harness.generation()?, 2);
    assert_eq!(committed.revision.number(), 1);
    assert_eq!(harness.publishes()?, 2);
    Ok(())
}

/// A deterministic failure at one chunk poisons the job on its third
/// occurrence: it fails for good and cannot be resumed.
#[tokio::test]
async fn three_identical_failures_at_one_chunk_poison_the_job() -> TestResult {
    let harness = Harness::new()?;
    harness
        .recognizer
        .fault_at(1, Fault::Fails(SpeechRecognitionError::Deadline));
    let resolved = harness.resolve(None)?;
    for attempt in 1..=3 {
        let failure = harness.run(&resolved, None).await;
        assert!(
            matches!(failure, Err(JobRunError::Asr { failure, .. })
                if failure.chunk == Some(1) && failure.failure.stage == AsrStage::Recognition),
            "attempt {attempt}: {failure:?}"
        );
    }
    let record = harness.record(&resolved.spec.job_id)?;
    assert_eq!(record.state, JobState::Failed);
    assert_eq!(harness.checkpoints(&resolved.spec.job_id)?, 0);
    let refused = super::resumable_request(&harness.store, &session()?, &resolved.spec.job_id, NOW);
    assert_eq!(
        refused,
        Err(JobRunError::NotResumable {
            job: resolved.spec.job_id.clone(),
            state: JobState::Failed
        })
    );
    // Rerunning the same command restarts the job in a new epoch.
    harness.recognizer.clear();
    let restarted = harness.run(&resolved, None).await?;
    assert!(!restarted.report.resumed);
    assert_eq!(harness.record(&resolved.spec.job_id)?.epoch, 1);
    Ok(())
}

/// A provider that fails after the caller cancelled (on Windows a console
/// Ctrl-C reaches the provider too) is recorded as the cancellation: the
/// job stays interrupted and resumable, the failure never counts towards
/// poisoning its chunk, and the caller is told `CANCELLED`.
#[tokio::test]
async fn a_failure_after_the_callers_cancellation_is_the_cancellation() -> TestResult {
    let harness = Harness::new()?;
    harness
        .recognizer
        .fault_at(1, Fault::RaisesAndFails(SpeechRecognitionError::Io));
    let resolved = harness.resolve(None)?;
    for attempt in 1..=3 {
        harness.recognizer.flag.store(false, Ordering::SeqCst);
        let failure = harness.run(&resolved, None).await;
        assert!(
            matches!(&failure, Err(JobRunError::Asr { failure, .. })
                if failure.chunk == Some(1)
                    && failure.failure.reason == AsrFailureReason::Cancelled),
            "attempt {attempt}: {failure:?}"
        );
        let record = harness.record(&resolved.spec.job_id)?;
        assert_eq!(record.state, JobState::Interrupted, "attempt {attempt}");
        assert!(
            record
                .failures
                .iter()
                .all(|failure| failure.code == FailureCode::Cancelled)
        );
    }
    assert_eq!(harness.publishes()?, 0);
    assert_eq!(harness.checkpoints(&resolved.spec.job_id)?, 1);

    harness.recognizer.clear();
    harness.recognizer.flag.store(false, Ordering::SeqCst);
    let resumed = harness.run(&resolved, None).await?;
    assert!(resumed.report.resumed);
    assert_eq!(resumed.report.chunks_reused, 1);
    Ok(())
}

// ---------------------------------------------------------------- rebase

/// Another revision committed during the run: re-widening the request over
/// it gives the same range, so the result is spliced onto the new newest
/// revision; a different range fails the job as superseded.
#[tokio::test]
async fn a_superseded_base_is_respliced_or_the_job_fails() -> TestResult {
    let harness = Harness::new()?;
    let first = harness.request(None, None).await??.revision;
    // The request resolves against revision 1...
    let resolved = harness.resolve(Some(range(90 * SECOND, 100 * SECOND)?))?;
    // ...then another run commits revision 2 over a range elsewhere.
    let second = harness
        .request(Some(range(10 * SECOND, 12 * SECOND)?), None)
        .await??
        .revision;
    assert_eq!(second.supersedes(), Some(first.id()));
    let spliced = harness.run(&resolved, None).await?;
    assert_eq!(spliced.revision.number(), 3);
    assert_eq!(spliced.revision.supersedes(), Some(second.id()));

    // A request whose widened range changes under the new newest revision:
    // it resolves to the whole 10-12 s segment of revision 1, then revision 2
    // replaces 9-12 s with a segment ending at 11 s, which the request no
    // longer cuts.
    let harness = Harness::new()?;
    harness.request(None, None).await??;
    let resolved = harness.resolve(Some(range(11 * SECOND, 11 * SECOND + 1)?))?;
    assert_eq!(resolved.replaced, range(10 * SECOND, 12 * SECOND)?);
    harness
        .request(Some(range(9 * SECOND, 10 * SECOND + SECOND / 2)?), None)
        .await??;
    let replaced_before = resolved.replaced;
    let now = crate::retranscription_range(
        harness.store.head(&session()?, NOW)?.newest.as_ref(),
        resolved.requested,
        harness.source.range(),
    );
    if now == replaced_before {
        return Err("the second revision did not change the widened range".into());
    }
    let superseded = harness.run(&resolved, None).await;
    assert_eq!(
        superseded,
        Err(JobRunError::Superseded {
            job: resolved.spec.job_id.clone()
        })
    );
    assert_eq!(
        harness.record(&resolved.spec.job_id)?.state,
        JobState::Failed
    );
    Ok(())
}

// ---------------------------------------------------------------- cancellation

/// X-06 ordering: a cancellation requested through the job while it runs
/// wins before the commit: no generation, the job is cancelled and its
/// checkpoints are gone. A repeated cancel is idempotent.
#[tokio::test]
async fn a_cancel_requested_during_the_run_wins_before_the_commit() -> TestResult {
    let harness = Harness::new()?;
    let resolved = harness.resolve(None)?;
    let job = resolved.spec.job_id.clone();
    // The recognizer asks for cancellation through the store while chunk 1
    // runs, as `job cancel` from another process would.
    let store = harness.store.clone();
    let requester = session()?;
    let cancelling = {
        let job = job.clone();
        move || -> Result<CancelRequest, JobStoreError> {
            store.request_cancel(&requester, &job, NOW)
        }
    };
    harness.recognizer.fault_at(1, Fault::Raises);
    // The flag the recognizer raises is not the caller's here: the run must
    // stop because of the job request alone.
    let flag = Arc::clone(&harness.recognizer.flag);
    let watcher = std::thread::spawn(move || {
        while !flag.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        cancelling()
    });
    let never = Flag(Arc::new(AtomicBool::new(false)));
    let mut guard = CountingGuard::default();
    // The slow store waits at chunk 2 until the request has reached the record.
    let outcome = run_retranscription(
        RetranscriptionRun {
            spec: &resolved.spec,
            operation_id: None,
            transcribe: TranscribeRangeRequest {
                source_segment: &harness.source,
                range: resolved.replaced,
                plan: ChunkPlan::R0,
                audio_stream: 1,
                expected: &harness.identity,
            },
            source_id: &harness.source_id,
            requested: None,
            base: None,
            observed: resolved.observed,
            now: NOW,
            admission: AdmissionWait::Immediate,
        },
        RetranscriptionPorts {
            store: &SlowStore(&harness.store),
            audio: &harness.audio,
            recognizer: &harness.recognizer,
            cancellation: &never,
            timer: &harness.timer,
            classify,
            progress: &NoProgress,
        },
        &mut guard,
    )
    .await;
    let filed = watcher.join().map_err(|_| "watcher panicked")??;
    assert_eq!(filed, CancelRequest::Requested);
    assert_eq!(outcome, Err(JobRunError::Cancelled { job: job.clone() }));
    assert_eq!(harness.generation()?, 0);
    assert_eq!(harness.record(&job)?.state, JobState::Cancelled);
    assert_eq!(harness.checkpoints(&job)?, 0);
    assert_eq!(
        cancel_job(&harness.store, &session()?, &job, NOW)?,
        CancelOutcome::AlreadyEnded(JobState::Cancelled)
    );
    Ok(())
}

/// A store that waits, at each checkpoint read, until a pending cancel
/// request (raised by the recognizer's flag) has reached the record, so the
/// test is deterministic.
struct SlowStore<'a>(&'a MemoryStore);

impl SlowStore<'_> {
    fn settle(&self) {
        for _ in 0..10_000 {
            let raised = self
                .0
                .with(|shared| {
                    shared
                        .jobs
                        .values()
                        .any(|record| record.state == JobState::Cancelling)
                })
                .unwrap_or(true);
            if raised {
                return;
            }
            std::thread::yield_now();
        }
    }
}

impl JobStore for SlowStore<'_> {
    type Owner = SlowOwner;

    fn job(
        &self,
        session_id: &SessionId,
        job_id: &JobId,
    ) -> Result<Option<JobView>, JobStoreError> {
        self.0.job(session_id, job_id)
    }

    fn job_session(&self, job_id: &JobId) -> Result<Option<SessionId>, JobStoreError> {
        self.0.job_session(job_id)
    }

    fn job_for_operation(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
    ) -> Result<Option<JobId>, JobStoreError> {
        self.0.job_for_operation(session_id, operation_id)
    }

    fn session_jobs(&self, session_id: &SessionId) -> Result<Vec<JobView>, JobStoreError> {
        self.0.session_jobs(session_id)
    }

    fn open_or_create(
        &self,
        spec: &JobSpec,
        now: u64,
    ) -> Result<(Self::Owner, bool), JobStoreError> {
        self.0
            .open_or_create(spec, now)
            .map(|(owner, created)| (SlowOwner(owner, self.0.clone()), created))
    }

    fn acquire(
        &self,
        session_id: &SessionId,
        job_id: &JobId,
    ) -> Result<Self::Owner, JobStoreError> {
        self.0
            .acquire(session_id, job_id)
            .map(|owner| SlowOwner(owner, self.0.clone()))
    }

    fn request_cancel(
        &self,
        session_id: &SessionId,
        job_id: &JobId,
        now: u64,
    ) -> Result<CancelRequest, JobStoreError> {
        self.0.request_cancel(session_id, job_id, now)
    }
}

impl CommitLedger for SlowStore<'_> {
    fn committed_generation(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        above: StorageGeneration,
    ) -> Result<Option<StorageGeneration>, SessionStorageError> {
        self.0.committed_generation(session_id, operation_id, above)
    }
}

impl RevisionStore for SlowStore<'_> {
    type Permit = ();

    fn admit(&self, weight: NonZeroU16) -> Result<Self::Permit, SessionStorageError> {
        self.0.admit(weight)
    }

    fn head(&self, session_id: &SessionId, now: u64) -> Result<SessionHead, SessionStorageError> {
        self.0.head(session_id, now)
    }

    fn revision(
        &self,
        session_id: &SessionId,
        revision_id: &TranscriptRevisionId,
        now: u64,
    ) -> Result<Option<TranscriptRevision>, SessionStorageError> {
        self.0.revision(session_id, revision_id, now)
    }

    fn publish_revision(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        expected: StorageGeneration,
        revision: &TranscriptRevision,
        now: u64,
    ) -> Result<StorageGeneration, SessionStorageError> {
        self.0
            .publish_revision(session_id, operation_id, expected, revision, now)
    }
}

struct SlowOwner(MemoryOwner, MemoryStore);

impl ChunkCheckpoints for SlowOwner {
    fn load(&self, index: u32) -> CheckpointRead {
        if index > 1 {
            SlowStore(&self.1).settle();
        }
        self.0.load(index)
    }

    fn store(&self, checkpoint: &ChunkCheckpoint) -> Result<(), CheckpointStoreError> {
        self.0.store(checkpoint)
    }

    fn discard(&self, index: u32) {
        self.0.discard(index);
    }
}

impl JobOwner for SlowOwner {
    fn record(&self) -> &JobRecord {
        self.0.record()
    }

    fn apply(&mut self, change: &JobChange, now: u64) -> Result<(), JobStoreError> {
        self.0.apply(change, now)
    }

    fn bind_operation(
        &mut self,
        operation_id: &OperationId,
        now: u64,
    ) -> Result<(), JobStoreError> {
        self.0.bind_operation(operation_id, now)
    }

    fn cancel_requested(&self) -> bool {
        self.0.cancel_requested()
    }
}

/// Cancelling a live committing job is too late; cancelling an interrupted
/// one cancels it and removes its checkpoints; a job whose commit landed
/// reports too late even though its record said committing.
#[tokio::test]
async fn cancellation_is_serialised_with_the_commit() -> TestResult {
    let harness = Harness::new()?;
    let session = session()?;
    harness
        .recognizer
        .fault_at(2, Fault::Fails(SpeechRecognitionError::Deadline));
    let resolved = harness.resolve(None)?;
    assert!(harness.run(&resolved, None).await.is_err());
    let job = resolved.spec.job_id.clone();
    assert_eq!(harness.checkpoints(&job)?, 2);

    // Live and committing: too late.
    {
        let mut owner = harness.store.acquire(&session, &job)?;
        owner.apply(&JobChange::Start, NOW)?;
        owner.apply(
            &JobChange::Commit(super::JobCommit {
                operation_id: operation("op_dddddddddddddddd")?,
                observed_generation: StorageGeneration::INITIAL,
                revision_id: TranscriptRevisionId::parse("trv_0123456789abcdef")?,
            }),
            NOW,
        )?;
        assert_eq!(
            cancel_job(&harness.store, &session, &job, NOW)?,
            CancelOutcome::TooLate(JobState::Committing)
        );
        // The owner ends without committing (process exit).
    }
    // Unowned committing, commit not in the chain: interrupted, then cancelled.
    assert_eq!(
        job_status(&harness.store, &session, &job, NOW)?
            .record
            .state,
        JobState::Interrupted
    );
    assert_eq!(
        cancel_job(&harness.store, &session, &job, NOW)?,
        CancelOutcome::Cancelled
    );
    assert_eq!(harness.checkpoints(&job)?, 0);
    assert_eq!(
        cancel_job(&harness.store, &session, &job, NOW)?,
        CancelOutcome::AlreadyEnded(JobState::Cancelled)
    );

    // A job whose commit landed: cancel reconciles it and reports too late.
    let other = Harness::new()?;
    let committed = other.request(None, None).await??;
    let landed = committed.report.job_id.clone();
    other.store.with(|shared| {
        if let Some(record) = shared.jobs.get_mut(landed.as_str()) {
            record.state = JobState::Committing;
        }
    })?;
    assert_eq!(
        cancel_job(&other.store, &session, &landed, NOW)?,
        CancelOutcome::TooLate(JobState::Succeeded)
    );
    assert_eq!(other.record(&landed)?.state, JobState::Succeeded);
    Ok(())
}

/// X-05: a stale owner (whose attempt is no longer the job's) cannot change
/// the job, so a stale attempt cannot commit.
#[test]
fn a_stale_attempt_cannot_change_the_job() -> TestResult {
    let harness = Harness::new()?;
    let resolved = harness.resolve(None)?;
    let (mut stale, _) = harness.store.open_or_create(&resolved.spec, NOW)?;
    stale.apply(&JobChange::Start, NOW)?;
    let snapshot = stale.record().clone();
    drop(stale);
    // A newer attempt takes over.
    let mut current = harness.store.acquire(&session()?, &resolved.spec.job_id)?;
    current.apply(&JobChange::Interrupt(None), NOW)?;
    current.apply(&JobChange::Start, NOW)?;
    drop(current);
    let mut revived = MemoryOwner {
        shared: Arc::clone(&harness.store.0),
        record: snapshot,
    };
    let commit = JobChange::Commit(super::JobCommit {
        operation_id: operation("op_eeeeeeeeeeeeeeee")?,
        observed_generation: StorageGeneration::INITIAL,
        revision_id: TranscriptRevisionId::parse("trv_0123456789abcdef")?,
    });
    assert_eq!(revived.apply(&commit, NOW), Err(JobStoreError::StaleOwner));
    Ok(())
}

/// The record's change rules: attempts are bounded, restart starts a new
/// epoch from nothing, and illegal changes are refused.
#[test]
fn record_changes_follow_the_state_graph() -> TestResult {
    let harness = Harness::new()?;
    let spec = harness.resolve(None)?.spec;
    let queued = JobRecord::queued(&spec, NOW);
    assert!(matches!(
        queued.changed(&JobChange::Succeed, NOW),
        Err(JobStoreError::Transition(_))
    ));
    assert!(matches!(
        queued.changed(&JobChange::Restart, NOW),
        Err(JobStoreError::Transition(_))
    ));
    let mut record = queued.changed(&JobChange::Start, NOW)?;
    assert_eq!(record.attempt, 1);
    for _ in 1..vsift_domain::MAX_JOB_ATTEMPTS {
        record = record.changed(&JobChange::Interrupt(None), NOW)?;
        record = record.changed(&JobChange::Start, NOW)?;
    }
    assert_eq!(record.attempt, vsift_domain::MAX_JOB_ATTEMPTS);
    let interrupted = record.changed(&JobChange::Interrupt(None), NOW)?;
    assert_eq!(
        interrupted.changed(&JobChange::Start, NOW),
        Err(JobStoreError::Storage(
            SessionStorageError::CapacityExhausted
        ))
    );
    let failed = record.changed(
        &JobChange::Fail(Some(vsift_domain::AttemptFailure {
            chunk: Some(2),
            code: FailureCode::DeadlineExceeded,
        })),
        NOW,
    )?;
    let restarted = failed.changed(&JobChange::Restart, NOW)?;
    assert_eq!(restarted.state, JobState::Queued);
    assert_eq!(restarted.epoch, 1);
    assert_eq!(restarted.attempt, 0);
    assert!(restarted.failures.is_empty());
    Ok(())
}
