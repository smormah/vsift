//! Recoverable jobs through the engine (P10 PR 2, ADR 0020).
//!
//! Everything decided before audio is decoded runs everywhere: unknown jobs,
//! an operation id that replays its commit (X-02) or conflicts with another
//! request (X-03) before any preflight or hash, and status and cancel of a
//! succeeded job. The opt-in tests decode real speech with `FFmpeg` (on
//! `PATH`) through a host recognizer that can fail once, and show that the
//! same request resumes from its checkpoints to the revision an uninterrupted
//! run commits (X-01), and that a job is resumed and cancelled by its id:
//!
//! `cargo test -p vsift --locked --test engine_jobs -- --ignored`
//!
//! One more opt-in test interrupts a real whisper.cpp run after its first
//! checkpoint (by cancelling it) and resumes it; it also needs
//! `VSIFT_TEST_WHISPER_CLI` and `VSIFT_TEST_WHISPER_MODEL`.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    future::Future,
    num::NonZeroU16,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use vsift::{
    AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider, AsrProviderBuild, Cancellation,
    ChunkTime, Clock, ClockError, CueText, Engine, EngineConfig, EngineError, EnginePorts,
    FailureCode, HostIsolation, IdentifierGenerationError, IdentifierSource, IngestRequest,
    JobCancelOutcome, JobId, JobProgress, JobResumeRequest, JobState, LanguageTag,
    LocalAsrVerification, LocalAsrVerifier, MediaToolVerification, MediaToolVerifier, OperationId,
    PlannedChunk, ProgressObserver, ProgressStage, ProviderChunkOutput, ProviderSegment,
    ProviderToken, ProviderTokenKind, RecognizerIdentity, RetranscribeRange, RetranscribeRequest,
    RuntimeDependency, SessionId, SessionRootLocation, Sha256Hex, SpeechPcm,
    SpeechRecognitionError, SpeechRecognizer, UserConfigurationLocation,
};
use vsift_application::{
    AsrCancellation, CommitGuard, JobRequest, JobSpec, RecognitionScope, RetranscriptionPorts,
    RetranscriptionRun, RetryTimer, RevisionStore, SessionStorageError, SpeechAudioError,
    SpeechAudioSource, TranscribeRangeRequest, job_id, recognition_key, retranscribe_operation_key,
    retranscribe_request_digest, retranscription_range, run_retranscription,
    whole_file_source_segment,
};
use vsift_domain::{ChunkPlan, Jitter, MediaTime, TimeRange, TranscriptRevision};
use vsift_infrastructure::FilesystemSessionStore;

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-engine-jobs-test-";
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const PLACEHOLDER_SOURCE: &[u8] = b"\0\0\0\x18ftypisomengine-jobs-source";
const FIXED_SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const OPERATION: &str = "op_0123456789abcdef0123456789abcdef";
const SECOND: u64 = 1_000_000;

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

/// The same session identity in every harness, so revisions of separate
/// roots can be compared whole.
struct FixedIdentifiers(AtomicU64);

impl IdentifierSource for FixedIdentifiers {
    fn session_id(&self) -> Result<SessionId, IdentifierGenerationError> {
        SessionId::parse(FIXED_SESSION).map_err(|_| IdentifierGenerationError::NonCanonical)
    }

    fn operation_id(&self) -> Result<OperationId, IdentifierGenerationError> {
        let next = self.0.fetch_add(1, Ordering::SeqCst);
        OperationId::parse(format!("op_{next:032x}"))
            .map_err(|_| IdentifierGenerationError::NonCanonical)
    }
}

/// The operating-system clock.
struct Wall;

impl Clock for Wall {
    fn now_unix_seconds(&self) -> Result<u64, ClockError> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .map_err(|_| ClockError::BeforeUnixEpoch)
    }
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

/// One segment per chunk, placed inside every window whatever its length.
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

/// A deterministic host recognizer that counts calls and can fail once at
/// one chunk with a deadline.
#[derive(Clone)]
struct Recognizer {
    recognitions: Arc<AtomicUsize>,
    fail_once_at: Arc<AtomicU64>,
}

impl Default for Recognizer {
    fn default() -> Self {
        Self::failing_once_at(Self::NEVER)
    }
}

impl Recognizer {
    const NEVER: u64 = u64::MAX;

    fn failing_once_at(chunk: u64) -> Self {
        Self {
            recognitions: Arc::new(AtomicUsize::new(0)),
            fail_once_at: Arc::new(AtomicU64::new(chunk)),
        }
    }

    fn calls(&self) -> usize {
        self.recognitions.load(Ordering::SeqCst)
    }
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
        self.recognitions.fetch_add(1, Ordering::SeqCst);
        let fails = self
            .fail_once_at
            .compare_exchange(
                u64::from(chunk.index()),
                Self::NEVER,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok();
        std::future::ready(if fails {
            Err(SpeechRecognitionError::Deadline)
        } else {
            words(chunk)
        })
    }
}

#[derive(Clone, Default)]
struct CountingVerifier(Arc<AtomicUsize>);

impl LocalAsrVerifier for CountingVerifier {
    fn verify(&self) -> impl Future<Output = LocalAsrVerification> + Send {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::future::ready(LocalAsrVerification::Verified)
    }
}

#[derive(Clone)]
struct PassingMediaTools;

impl MediaToolVerifier for PassingMediaTools {
    fn verify(&self) -> impl Future<Output = MediaToolVerification> + Send {
        std::future::ready(MediaToolVerification::Verified)
    }
}

struct Harness {
    root: OwnedRoot,
    recognizer: Recognizer,
    verifier: CountingVerifier,
}

impl Harness {
    fn new(recognizer: Recognizer) -> Built<Self> {
        Ok(Self {
            root: OwnedRoot::new()?,
            recognizer,
            verifier: CountingVerifier::default(),
        })
    }

    fn engine(&self) -> Engine {
        Engine::new(
            EngineConfig {
                session_root: SessionRootLocation::Explicit(self.root.path("sessions")),
                user_configuration: UserConfigurationLocation::Explicit(self.root.path("config")),
                host_isolation: HostIsolation::ProcessOnly,
            },
            EnginePorts::new(Wall, FixedIdentifiers(AtomicU64::new(1)))
                .with_media_tool_verifier(PassingMediaTools)
                .with_speech_recognizer(self.recognizer.clone(), self.verifier.clone()),
        )
    }

    fn store(&self) -> Built<FilesystemSessionStore> {
        Ok(FilesystemSessionStore::open_existing(
            self.root.path("sessions"),
        )?)
    }

    /// Plain files registered as the media tools: resolvable, never runnable.
    fn stand_in_tools(&self, engine: &Engine) -> TestResult {
        for (dependency, name) in [
            (RuntimeDependency::Ffmpeg, "ffmpeg-stand-in.exe"),
            (RuntimeDependency::Ffprobe, "ffprobe-stand-in.exe"),
        ] {
            let path = self.root.path(name);
            fs::write(&path, b"not a program")?;
            engine.configure_executable(dependency, &path)?;
        }
        Ok(())
    }

    async fn session(&self, engine: &Engine, source: PathBuf) -> Built<SessionId> {
        Ok(engine
            .ingest(IngestRequest {
                source,
                transcript: None,
                cancellation: Cancellation::new(),
                durability: vsift::DurabilityRequirement::Ephemeral,
            })
            .await?
            .session
            .session_id)
    }
}

fn request(
    session: &SessionId,
    range: Option<(u64, u64)>,
    operation_id: Option<&OperationId>,
) -> RetranscribeRequest {
    RetranscribeRequest {
        session: session.clone(),
        range: range.map(|(from_micros, to_micros)| RetranscribeRange {
            from_micros,
            to_micros,
        }),
        operation_id: operation_id.cloned(),
        cancellation: Cancellation::new(),
        progress: ProgressObserver::none(),
        admission: vsift::AdmissionWait::Immediate,
    }
}

/// An observer that records every progress observation (P11).
fn recording_observer() -> (ProgressObserver, Arc<Mutex<Vec<JobProgress>>>) {
    let seen: Arc<Mutex<Vec<JobProgress>>> = Arc::default();
    let recorder = Arc::clone(&seen);
    let observer = ProgressObserver::new(move |progress| {
        if let Ok(mut seen) = recorder.lock() {
            seen.push(progress.clone());
        }
    });
    (observer, seen)
}

// ------------------------------------------------ everywhere, before decoding

#[tokio::test]
async fn unknown_jobs_are_not_found() -> TestResult {
    let harness = Harness::new(Recognizer::default())?;
    let engine = harness.engine();
    let job = JobId::parse("job_0123456789abcdef0123456789abcdef")?;
    for error in [
        engine.job_status(&job).err(),
        engine.job_cancel(&job).err(),
        engine
            .job_resume(JobResumeRequest {
                job: job.clone(),
                cancellation: Cancellation::new(),
                progress: ProgressObserver::none(),
                admission: vsift::AdmissionWait::Immediate,
            })
            .await
            .err(),
    ] {
        let error = error.ok_or("an unknown job was found")?;
        assert!(matches!(error, EngineError::JobNotFound), "{error:?}");
        assert_eq!(error.failure_code(), FailureCode::InvalidArgument);
    }
    Ok(())
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

/// Commits a retranscription of `range` with the application use case and
/// fake audio, as a request with `operation` would, without the engine's
/// preflights; returns its job and revision.
async fn committed_through_the_use_case(
    store: &FilesystemSessionStore,
    session: &SessionId,
    range: Option<TimeRange>,
    operation: &OperationId,
) -> Built<(JobId, TranscriptRevision)> {
    let now = Wall.now_unix_seconds()?;
    let status = store.session_status(session)?;
    let segment =
        whole_file_source_segment(status.source_id(), MediaTime::from_micros(60 * SECOND))?;
    let head = store.head(session, now)?;
    let replaced = retranscription_range(head.newest.as_ref(), range, segment.range());
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
    let spec = JobSpec {
        session_id: session.clone(),
        job_id: job_id(session, &operation_key)?,
        request_digest: retranscribe_request_digest(session, range)?,
        operation_key,
        recognition_key: key,
        request: JobRequest::Retranscribe { range },
        planned_chunks: None,
    };
    let outcome = run_retranscription(
        RetranscriptionRun {
            spec: &spec,
            operation_id: Some(operation),
            transcribe: TranscribeRangeRequest {
                source_segment: &segment,
                range: replaced,
                plan: ChunkPlan::R0,
                audio_stream: 1,
                expected: &identity,
            },
            source_id: status.source_id(),
            requested: range,
            base: head.newest.as_ref(),
            observed: head.generation,
            now,
            admission: vsift_domain::AdmissionWait::Immediate,
        },
        RetranscriptionPorts {
            store,
            audio: &Audio,
            recognizer: &Recognizer::default(),
            cancellation: &Never,
            timer: &Instant0,
            classify: |_| FailureCode::Internal,
            progress: &vsift_application::NoProgress,
        },
        &mut Unchanged,
    )
    .await?;
    Ok((outcome.report.job_id, outcome.revision))
}

/// X-02 and X-03 through the engine: a retry with the operation id of a
/// committed job returns that commit before any preflight, hash or
/// recognition, with no new generation; the same id with another range is
/// `IDEMPOTENCY_CONFLICT` and changes nothing. The job reports its revision,
/// and cancelling it is too late.
#[tokio::test]
async fn a_committed_operation_is_replayed_before_any_check_or_hash() -> TestResult {
    let harness = Harness::new(Recognizer::default())?;
    let engine = harness.engine();
    harness.stand_in_tools(&engine)?;
    let source = harness.root.path("placeholder.mp4");
    fs::write(&source, PLACEHOLDER_SOURCE)?;
    let session = harness.session(&engine, source).await?;
    let operation = OperationId::parse(OPERATION)?;
    let range = Some(TimeRange::new(
        MediaTime::from_micros(0),
        MediaTime::from_micros(20 * SECOND),
    )?);
    let (job, revision) =
        committed_through_the_use_case(&harness.store()?, &session, range, &operation).await?;
    let before = engine.session_status(&session)?.generation();

    let (observer, seen) = recording_observer();
    let replayed = engine
        .retranscribe(RetranscribeRequest {
            progress: observer,
            ..request(&session, Some((0, 20 * SECOND)), Some(&operation))
        })
        .await?;
    assert!(replayed.job().replayed());
    // A replay does no work, so it reports no progress.
    assert!(seen.lock().map_err(|_| "poisoned")?.is_empty());
    assert_eq!(replayed.job().job_id(), &job);
    assert_eq!(replayed.job().operation_id(), &operation);
    assert_eq!(replayed.revision(), &revision);
    assert_eq!(engine.session_status(&session)?.generation(), before);
    assert_eq!(harness.recognizer.calls(), 0);
    assert_eq!(harness.verifier.0.load(Ordering::SeqCst), 0);

    let conflict = engine
        .retranscribe(request(&session, Some((0, 10 * SECOND)), Some(&operation)))
        .await
        .err()
        .ok_or("a reused operation id ran another request")?;
    assert_eq!(
        conflict,
        EngineError::IdempotencyConflict { job: job.clone() }
    );
    assert_eq!(conflict.failure_code(), FailureCode::IdempotencyConflict);
    assert_eq!(conflict.affected_job(), Some(&job));
    assert_eq!(conflict.retry_after_ms(), None);
    assert_eq!(engine.session_status(&session)?.generation(), before);
    assert_eq!(harness.verifier.0.load(Ordering::SeqCst), 0);

    let status = engine.job_status(&job)?;
    assert_eq!(status.state(), JobState::Succeeded);
    assert_eq!(status.session_id(), &session);
    assert_eq!(status.revision_id(), Some(revision.id()));
    assert_eq!(status.operation_ids(), &[operation]);
    assert!(!status.live());
    assert_eq!(
        engine.job_cancel(&job)?.outcome(),
        JobCancelOutcome::TooLate(JobState::Succeeded)
    );
    assert!(matches!(
        engine
            .job_resume(JobResumeRequest {
                job: job.clone(),
                cancellation: Cancellation::new(),
                progress: ProgressObserver::none(),
                admission: vsift::AdmissionWait::Immediate,
            })
            .await,
        Err(EngineError::JobNotResumable {
            state: JobState::Succeeded,
            ..
        })
    ));
    Ok(())
}

// ---------------------------------------------------- opt-in: real decoding

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/corpus/generated")
        .join(name)
}

/// A clip of several R0 chunks, looping a committed speech fixture without
/// re-encoding it.
fn looped_clip(root: &OwnedRoot, name: &str, loops: u32) -> Built<PathBuf> {
    let clip = root.path("looped.mp4");
    let status = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-n",
            "-stream_loop",
            &loops.to_string(),
            "-i",
        ])
        .arg(fixture(name))
        .args(["-c", "copy"])
        .arg(&clip)
        .stdin(Stdio::null())
        .status()?;
    if !status.success() {
        return Err("ffmpeg could not build the looped clip".into());
    }
    Ok(clip)
}

/// X-01 through the engine: a run that fails at its third chunk is
/// interrupted with its first two chunks checkpointed; the same request
/// resumes from them and commits exactly the revision an uninterrupted run
/// commits in another root with the same identities.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn an_interrupted_retranscription_resumes_to_the_same_revision() -> TestResult {
    let control = Harness::new(Recognizer::default())?;
    let engine = control.engine();
    let clip = looped_clip(&control.root, "F05-speech.mp4", 3)?;
    let session = control.session(&engine, clip.clone()).await?;
    let expected = engine.retranscribe(request(&session, None, None)).await?;
    let recognised = control.recognizer.calls();
    assert!(
        recognised >= 3,
        "the clip has {recognised} recognised chunks"
    );

    let harness = Harness::new(Recognizer::failing_once_at(2))?;
    let engine = harness.engine();
    let session = harness.session(&engine, clip).await?;
    let failed = engine
        .retranscribe(request(&session, None, None))
        .await
        .err()
        .ok_or("the failing run committed")?;
    assert_eq!(failed.failure_code(), FailureCode::DeadlineExceeded);
    let before_resume = harness.recognizer.calls();
    let resumed = engine.retranscribe(request(&session, None, None)).await?;
    assert!(resumed.job().resumed());
    assert_eq!(resumed.job().chunks_reused(), 2);
    assert!(!resumed.job().replayed());
    // Only the chunks after the checkpoints were recognised again.
    assert_eq!(
        harness.recognizer.calls() - before_resume,
        recognised - (before_resume - 1)
    );
    assert_eq!(resumed.revision(), expected.revision());
    assert_eq!(engine.session_status(&session)?.artifact_count(), 1);
    Ok(())
}

/// A job is resumed and cancelled by its id: the interrupted job reports
/// itself interrupted with its checkpoints, `job_resume` commits it, and a
/// later cancel is too late.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn a_job_is_resumed_and_cancelled_by_its_id() -> TestResult {
    let harness = Harness::new(Recognizer::failing_once_at(1))?;
    let engine = harness.engine();
    let clip = looped_clip(&harness.root, "F05-speech.mp4", 3)?;
    let session = harness.session(&engine, clip).await?;
    let operation = OperationId::parse(OPERATION)?;
    assert!(
        engine
            .retranscribe(request(&session, None, Some(&operation)))
            .await
            .is_err()
    );
    let store = harness.store()?;
    let jobs = vsift_application::JobStore::session_jobs(&store, &session)?;
    let job = match jobs.as_slice() {
        [view] => view.record.job_id.clone(),
        _ => return Err(format!("expected one job, found {}", jobs.len()).into()),
    };
    let status = engine.job_status(&job)?;
    assert_eq!(status.state(), JobState::Interrupted);
    assert_eq!(status.checkpoints(), 1);
    let (observer, seen) = recording_observer();
    let resumed = engine
        .job_resume(JobResumeRequest {
            job: job.clone(),
            cancellation: Cancellation::new(),
            progress: observer,
            admission: vsift::AdmissionWait::Immediate,
        })
        .await?;
    // P11: the resumed run reports every chunk of its job, the reused one
    // included, from 0 to the whole plan.
    let seen = seen.lock().map_err(|_| "poisoned")?.clone();
    let counts: Vec<u64> = seen
        .iter()
        .map(|progress| progress.update.completed)
        .collect();
    let total = seen.first().and_then(|progress| progress.update.total);
    assert!(total.is_some_and(|total| counts == (0..=total).collect::<Vec<_>>()));
    assert!(
        seen.iter()
            .all(|progress| progress.job.as_ref() == Some(&job)
                && progress.update.stage == ProgressStage::RecognisingSpeech
                && progress.update.total == total)
    );
    assert_eq!(resumed.outcome().job().job_id(), &job);
    assert_eq!(resumed.outcome().job().operation_id(), &operation);
    assert_eq!(resumed.outcome().job().chunks_reused(), 1);
    assert_eq!(engine.job_status(&job)?.state(), JobState::Succeeded);
    assert_eq!(engine.job_status(&job)?.checkpoints(), 0);
    assert_eq!(
        engine.job_cancel(&job)?.outcome(),
        JobCancelOutcome::TooLate(JobState::Succeeded)
    );
    Ok(())
}

// -------------------------------------------- opt-in: real whisper.cpp run

fn absolute(variable: &str) -> Built<PathBuf> {
    let path = PathBuf::from(env::var_os(variable).ok_or(variable.to_owned())?);
    if !path.is_absolute() {
        return Err(format!("{variable} is not absolute").into());
    }
    Ok(path)
}

/// Any `chunks/<n>.json` checkpoint under `directory`.
fn has_checkpoint(directory: &Path) -> bool {
    let mut pending = vec![directory.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(entries) = fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|parent| parent == "chunks")
                && path
                    .extension()
                    .is_some_and(|extension| extension == "json")
            {
                return true;
            }
        }
    }
    false
}

/// An engine with real whisper.cpp and model, and the reviewed checks.
fn whisper_engine(root: &OwnedRoot) -> Built<Engine> {
    let engine = Engine::new(
        EngineConfig {
            session_root: SessionRootLocation::Explicit(root.path("sessions")),
            user_configuration: UserConfigurationLocation::Explicit(root.path("config")),
            host_isolation: HostIsolation::ProcessOnly,
        },
        EnginePorts::new(Wall, FixedIdentifiers(AtomicU64::new(1))),
    );
    engine.configure_executable(
        RuntimeDependency::Whisper,
        &absolute("VSIFT_TEST_WHISPER_CLI")?,
    )?;
    engine.configure_model(&absolute("VSIFT_TEST_WHISPER_MODEL")?)?;
    Ok(engine)
}

/// Opt-in real-tool check: a whisper.cpp retranscription of a clip of at
/// least three chunks, built at run time, is interrupted (cancelled) after
/// its first chunk checkpoint; running it again resumes from the
/// checkpoints and gives the segments of an uninterrupted control run.
#[tokio::test]
#[ignore = "opt-in: needs ffmpeg/ffprobe on PATH, VSIFT_TEST_WHISPER_CLI and VSIFT_TEST_WHISPER_MODEL"]
async fn a_real_whisper_run_interrupted_after_its_first_checkpoint_resumes() -> TestResult {
    let control_root = OwnedRoot::new()?;
    let clip = looped_clip(&control_root, "F01-speech.mp4", 13)?;
    let control_engine = whisper_engine(&control_root)?;
    let control_session = control_engine
        .ingest(IngestRequest {
            source: clip.clone(),
            transcript: None,
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Ephemeral,
        })
        .await?
        .session
        .session_id;
    let control = control_engine
        .retranscribe(request(&control_session, None, None))
        .await?;
    assert!(
        !control.revision().segments().is_empty(),
        "the control run heard nothing"
    );

    let root = OwnedRoot::new()?;
    let engine = whisper_engine(&root)?;
    let session = engine
        .ingest(IngestRequest {
            source: clip,
            transcript: None,
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Ephemeral,
        })
        .await?
        .session
        .session_id;
    let cancellation = Cancellation::new();
    let sessions = root.path("sessions");
    let signal = cancellation.clone();
    let watcher = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(600);
        while Instant::now() < deadline {
            if has_checkpoint(&sessions) {
                signal.cancel();
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        false
    });
    let interrupted = engine
        .retranscribe(RetranscribeRequest {
            cancellation,
            ..request(&session, None, None)
        })
        .await
        .err()
        .ok_or("the run finished before it was interrupted")?;
    assert!(watcher.join().map_err(|_| "watcher panicked")?);
    assert_eq!(interrupted.failure_code(), FailureCode::Cancelled);
    let resumed = engine.retranscribe(request(&session, None, None)).await?;
    assert!(resumed.job().resumed());
    assert!(resumed.job().chunks_reused() >= 1);
    let spoken = |revision: &TranscriptRevision| -> Vec<(u64, u64, String)> {
        revision
            .segments()
            .iter()
            .map(|segment| {
                (
                    segment.range().start().as_micros(),
                    segment.range().end().as_micros(),
                    segment.text().text().to_owned(),
                )
            })
            .collect()
    };
    assert_eq!(spoken(resumed.revision()), spoken(control.revision()));
    assert_eq!(
        resumed.revision().id(),
        control.revision().id(),
        "the same identities give the same revision"
    );
    Ok(())
}
