//! A transcription published as it goes, through the engine (ADR 0033): read
//! while it runs, followed with a since token, and sealed with every segment it
//! published; cancelled while it runs, it keeps them as a stopped revision. A
//! watched `prepare` job reports its progress in media time while it runs, and
//! `session close` stops it; with a first window it transcribes that window
//! first, indexes the video, and transcribes the rest without hearing the
//! window again.
//!
//! The recogniser is a deterministic stand-in that holds at one chunk until the
//! test lets it go; the audio is decoded by real `FFmpeg` from a looped speech
//! fixture of the corpus, so these tests are opt-in:
//!
//! `cargo test -p vsift --locked --test engine_growth -- --ignored`
//!
//! with `ffmpeg` and `ffprobe` on `PATH`.

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
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use vsift::{
    AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider, AsrProviderBuild, Cancellation,
    ChunkTime, Clock, ClockError, CueText, Engine, EngineConfig, EnginePorts, FailureCode,
    HostIsolation, IdentifierGenerationError, IdentifierSource, IngestRequest, JobCancelOutcome,
    LanguageTag, LocalAsrVerification, LocalAsrVerifier, ManagedRootLocation,
    MediaToolVerification, MediaToolVerifier, OperationId, PlannedChunk, ProgressObserver,
    ProviderChunkOutput, ProviderSegment, RecognizerIdentity, RetranscribePublication,
    RetranscribeRequest, SearchRequest, SessionId, SessionRootLocation, Sha256Hex, SpeechPcm,
    SpeechRecognitionError, SpeechRecognizer, TranscriptQuery, UserConfigurationLocation,
};
use vsift::{
    JobMode, JobResumeRequest, JobStartRequest, JobStartState, JobState, PrepareStream,
    SessionPhase,
};
use vsift_domain::RevisionState;

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-engine-growth-test-";
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const FIXED_SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const HELD_AT: u32 = 3;

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

/// Two sentences per chunk, at 2 and 12 s into it, inside every window.
fn words(chunk: &PlannedChunk) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    let index = chunk.index();
    let window_ms = chunk.window().duration_micros() / 1_000;
    let mut segments = Vec::new();
    for (start, number) in [(2_000, "first"), (12_000, "second")] {
        if start + 3_000 > window_ms {
            continue;
        }
        let text = format!("chunk {index} says its {number} sentence");
        segments.push(ProviderSegment {
            start: ChunkTime::from_millis(start).ok_or(SpeechRecognitionError::Io)?,
            end: ChunkTime::from_millis(start + 3_000).ok_or(SpeechRecognitionError::Io)?,
            text: Some(CueText::new(text.clone(), text).map_err(|_| SpeechRecognitionError::Io)?),
            tokens: Vec::new(),
        });
    }
    Ok(ProviderChunkOutput {
        language: LanguageTag::parse("en").ok(),
        segments,
    })
}

/// A recogniser that holds at chunk [`HELD_AT`] until it is let go.
#[derive(Clone, Default)]
struct HeldRecogniser {
    reached: Arc<AtomicBool>,
    released: Arc<AtomicBool>,
    /// Chunks it was asked to recognise.
    calls: Arc<AtomicU64>,
}

impl SpeechRecognizer for HeldRecogniser {
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        let held = chunk.index() == HELD_AT;
        let reached = Arc::clone(&self.reached);
        let released = Arc::clone(&self.released);
        let answer = words(chunk);
        async move {
            if held {
                reached.store(true, Ordering::SeqCst);
                while !released.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
            answer
        }
    }
}

#[derive(Clone, Default)]
struct PassingVerifier;

impl LocalAsrVerifier for PassingVerifier {
    fn verify(&self) -> impl Future<Output = LocalAsrVerification> + Send {
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

fn engine(root: &OwnedRoot, recogniser: &HeldRecogniser) -> Engine {
    Engine::new(
        EngineConfig {
            session_root: SessionRootLocation::Explicit(root.path("sessions")),
            user_configuration: UserConfigurationLocation::Explicit(root.path("config")),
            managed_root: ManagedRootLocation::Explicit(root.path("managed")),
            host_isolation: HostIsolation::ProcessOnly,
        },
        EnginePorts::new(Wall, FixedIdentifiers(AtomicU64::new(1)))
            .with_media_tool_verifier(PassingMediaTools)
            .with_speech_recognizer(recogniser.clone(), PassingVerifier),
    )
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/corpus/generated")
        .join(name)
}

/// 140 s of speech (six R0 chunks): the 20 s F05 fixture looped without
/// re-encoding.
fn looped_clip(root: &OwnedRoot) -> Built<PathBuf> {
    let clip = root.path("looped.mp4");
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-n", "-stream_loop", "6", "-i"])
        .arg(fixture("F05-speech.mp4"))
        .args(["-c", "copy"])
        .arg(&clip)
        .stdin(Stdio::null())
        .status()?;
    if !status.success() {
        return Err("ffmpeg could not build the looped clip".into());
    }
    Ok(clip)
}

async fn session(engine: &Engine, source: &Path) -> Built<SessionId> {
    Ok(engine
        .ingest(IngestRequest {
            source: source.to_path_buf(),
            transcript: None,
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Ephemeral,
        })
        .await?
        .session
        .session_id)
}

fn published_as_it_goes(session: &SessionId) -> RetranscribeRequest {
    RetranscribeRequest {
        session: session.clone(),
        range: None,
        operation_id: None,
        cancellation: Cancellation::new(),
        progress: ProgressObserver::none(),
        admission: vsift::AdmissionWait::Immediate,
        publication: RetranscribePublication::AsItGoes {
            interval: Duration::ZERO,
        },
        share: vsift::RecognitionShare::Whole,
        seed: None,
    }
}

fn whole(session: &SessionId, since: Option<String>) -> TranscriptQuery {
    TranscriptQuery {
        session: session.clone(),
        revision: None,
        from_micros: 0,
        to_micros: 10_000_000_000,
        limit: Some(100),
        cursor: None,
        since,
    }
}

/// Waits, a minute at most, until the recogniser holds.
async fn until_held(recogniser: &HeldRecogniser) -> TestResult {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !recogniser.reached.load(Ordering::SeqCst) {
        if Instant::now() > deadline {
            return Err("the run never reached the held chunk".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}

/// ADR 0033: while the run holds at its fourth chunk, a read in another engine
/// sees the open revision, partial, with what is pending and a since token; a
/// search over it is partial too; once let go, the run seals a revision that
/// begins with every published segment, and the since token reads the rest.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn a_transcription_is_read_while_it_runs_and_seals_what_it_published() -> TestResult {
    let root = OwnedRoot::new()?;
    let recogniser = HeldRecogniser::default();
    let runner = Arc::new(engine(&root, &recogniser));
    let clip = looped_clip(&root)?;
    let session_id = session(&runner, &clip).await?;
    let running = {
        let runner = Arc::clone(&runner);
        let request = published_as_it_goes(&session_id);
        tokio::spawn(async move { runner.retranscribe(request).await })
    };
    until_held(&recogniser).await?;

    let reader = engine(&root, &recogniser);
    let read = reader.transcript(whole(&session_id, None))?;
    assert_eq!(read.revision().state(), RevisionState::Open);
    assert_ne!(read.segments().len(), 0, "nothing was published");
    assert_ne!(read.coverage().pending().len(), 0, "nothing is pending");
    let (token, state, _) = read.growth().since().ok_or("no since token")?;
    assert_eq!(state, RevisionState::Open);
    let token = token.to_owned();
    let published = read.revision().clone();
    let searched = reader.search(SearchRequest {
        session: session_id.clone(),
        query: "sentence".to_owned(),
        revision: None,
        range: None,
        limit: Some(100),
        cursor: None,
        since: None,
    })?;
    assert!(!searched.coverage().is_complete());
    assert_eq!(searched.hits().len(), published.segments().len());

    recogniser.released.store(true, Ordering::SeqCst);
    let outcome = running.await??;
    let sealed = outcome.revision();
    assert_eq!(sealed.state(), RevisionState::Sealed);
    assert_eq!(sealed.id(), published.id());
    assert_eq!(
        sealed.segments().get(..published.segments().len()),
        Some(published.segments())
    );
    // The token reads what the sealed revision added after it.
    let rest = reader.transcript(whole(&session_id, Some(token)))?;
    assert_eq!(
        rest.segments(),
        &sealed.segments()[published.segments().len()..]
    );
    let (_, state, more) = rest.growth().since().ok_or("no since token")?;
    assert_eq!(state, RevisionState::Sealed);
    assert!(!more);
    Ok(())
}

/// ADR 0033, D1: a job cancelled while its run holds keeps what it published:
/// its run stops, the job is cancelled, and the session's newest revision is
/// the stopped one, which begins with every segment the open one had.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn a_job_cancelled_while_it_runs_keeps_what_it_published() -> TestResult {
    let root = OwnedRoot::new()?;
    let recogniser = HeldRecogniser::default();
    let runner = Arc::new(engine(&root, &recogniser));
    let clip = looped_clip(&root)?;
    let session_id = session(&runner, &clip).await?;
    let running = {
        let runner = Arc::clone(&runner);
        let request = published_as_it_goes(&session_id);
        tokio::spawn(async move { runner.retranscribe(request).await })
    };
    until_held(&recogniser).await?;
    let other = engine(&root, &recogniser);
    let published = other
        .transcript(whole(&session_id, None))?
        .revision()
        .clone();
    let jobs = other.session_jobs(&session_id)?;
    let job = jobs
        .entries()
        .first()
        .map(|entry| entry.job_id().clone())
        .ok_or("no job")?;
    assert_eq!(
        other.job_cancel(&job)?.outcome(),
        JobCancelOutcome::Requested
    );
    recogniser.released.store(true, Ordering::SeqCst);
    let failure = running
        .await?
        .err()
        .ok_or("the cancelled run committed a sealed revision")?;
    assert_eq!(failure.failure_code(), FailureCode::Cancelled);
    let stopped = other.transcript(whole(&session_id, None))?;
    assert_eq!(stopped.revision().state(), RevisionState::Stopped);
    assert_eq!(stopped.revision().id(), published.id());
    assert_eq!(
        stopped
            .revision()
            .segments()
            .get(..published.segments().len()),
        Some(published.segments())
    );
    assert_eq!(stopped.coverage().pending(), []);
    Ok(())
}

fn resumed(job: &vsift::JobId) -> JobResumeRequest {
    JobResumeRequest {
        job: job.clone(),
        cancellation: Cancellation::new(),
        progress: ProgressObserver::none(),
        admission: vsift::AdmissionWait::Immediate,
        publication: RetranscribePublication::AtTheSeal,
    }
}

/// ADR 0033 sections 5 and 6: a watched `prepare` job runs its transcript as
/// a job of its own that publishes as it goes. While it holds, `job status`
/// in another engine reports the stream's job, its media progress and how far
/// it is published, and `job start` again finds the running job; once it
/// seals, the job succeeded and `job start` answers that it is done.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn a_watched_job_reports_its_progress_while_it_publishes() -> TestResult {
    let root = OwnedRoot::new()?;
    let recogniser = HeldRecogniser::default();
    let runner = Arc::new(engine(&root, &recogniser));
    let clip = looped_clip(&root)?;
    let session_id = session(&runner, &clip).await?;
    let request = JobStartRequest {
        session: session_id.clone(),
        mode: Some(JobMode::Watched),
        first: None,
    };
    let plan = runner.job_start(&request)?;
    assert_eq!(plan.start(), JobStartState::Created);
    assert_eq!(plan.status().state(), JobState::Queued);
    let job = plan.job_id().clone();
    let running = {
        let runner = Arc::clone(&runner);
        let request = resumed(&job);
        tokio::spawn(async move { runner.job_resume(request).await })
    };
    until_held(&recogniser).await?;
    // The runner records its progress at most once a second.
    tokio::time::sleep(vsift::PROGRESS_RECORD_INTERVAL * 2).await;

    let other = engine(&root, &recogniser);
    let status = other.job_status(&job)?;
    assert!(status.live());
    assert_eq!(status.mode(), Some(JobMode::Watched));
    // The video was indexed first (D6); the transcript runs.
    let [visual, stream] = status.streams() else {
        return Err("not two streams".into());
    };
    assert_eq!(visual.stream(), PrepareStream::Visual);
    assert_eq!(visual.state(), Some(JobState::Succeeded));
    assert_eq!(stream.stream(), PrepareStream::Transcript);
    assert!(stream.live());
    assert_eq!(stream.state(), Some(JobState::Running));
    let media = stream.media().ok_or("no media progress")?;
    assert_eq!(media.done_until_us, 80_000_000, "three chunks reach 80 s");
    assert!(media.total_us().is_some_and(|total| total > 130_000_000));
    assert_eq!(media.fresh_chunks, 3);
    assert!(stream.published_until_us().is_some_and(|until| until > 0));
    assert_eq!(other.job_start(&request)?.start(), JobStartState::Running);
    let read = other.transcript(whole(&session_id, None))?;
    assert_eq!(read.revision().state(), RevisionState::Open);

    recogniser.released.store(true, Ordering::SeqCst);
    let report = running.await??;
    assert_eq!(report.status().state(), JobState::Succeeded);
    assert_eq!(report.outcome().revision().state(), RevisionState::Sealed);
    let status = other.job_status(&job)?;
    assert_eq!(
        status.streams().last().and_then(vsift::StreamStatus::state),
        Some(JobState::Succeeded)
    );
    let again = other.job_start(&request)?;
    assert_eq!(again.start(), JobStartState::Done);
    assert_eq!(again.job_id(), &job);
    Ok(())
}

/// ADR 0033 section 5: `session close` asks the running `prepare` job to stop
/// and waits for it: the runner seals what it published as stopped, the job is
/// cancelled, nothing of the session runs any more, and the session closes.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn session_close_stops_a_running_prepare_job() -> TestResult {
    let root = OwnedRoot::new()?;
    let recogniser = HeldRecogniser::default();
    let runner = Arc::new(engine(&root, &recogniser));
    let clip = looped_clip(&root)?;
    let session_id = session(&runner, &clip).await?;
    let plan = runner.job_start(&JobStartRequest {
        session: session_id.clone(),
        mode: Some(JobMode::Watched),
        first: None,
    })?;
    let job = plan.job_id().clone();
    let running = {
        let runner = Arc::clone(&runner);
        let request = resumed(&job);
        tokio::spawn(async move { runner.job_resume(request).await })
    };
    until_held(&recogniser).await?;
    let closing = {
        let other = engine(&root, &recogniser);
        let session_id = session_id.clone();
        tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            other
                .close_session(&session_id)
                .map(|closed| (closed, started.elapsed()))
        })
    };
    // The stand-in recogniser does not hear the cancellation: let it finish
    // its chunk, as a provider killed by the supervisor would end it.
    tokio::time::sleep(Duration::from_millis(500)).await;
    recogniser.released.store(true, Ordering::SeqCst);
    let failure = running.await?.err().ok_or("the job ran to its seal")?;
    assert_eq!(failure.failure_code(), FailureCode::Cancelled);
    let (closed, waited) = closing.await??;
    assert_eq!(closed.phase(), SessionPhase::Closed);
    assert!(
        waited < vsift::SESSION_CLOSE_JOB_WAIT,
        "the close waited it out"
    );
    let other = engine(&root, &recogniser);
    let status = other.job_status(&job)?;
    assert_eq!(status.state(), JobState::Cancelled);
    assert!(!status.live());
    assert!(
        other
            .session_jobs(&session_id)?
            .entries()
            .iter()
            .all(|entry| !entry.live())
    );
    Ok(())
}

/// ADR 0033 D5 and D6: a watched job with a first window seals that window as
/// a revision of its own before anything else, then indexes the video (so
/// `candidates` is a warm read), then transcribes the whole source, taking the
/// window's chunk from the first run instead of recognising it again; a read of
/// the window while the whole run holds names the window's revision for what
/// is still pending.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn a_first_window_is_transcribed_first_and_heard_once() -> TestResult {
    let root = OwnedRoot::new()?;
    let recogniser = HeldRecogniser::default();
    let runner = Arc::new(engine(&root, &recogniser));
    let clip = looped_clip(&root)?;
    let session_id = session(&runner, &clip).await?;
    let plan = runner.job_start(&JobStartRequest {
        session: session_id.clone(),
        mode: Some(JobMode::Watched),
        first: Some(vsift::RetranscribeRange {
            from_micros: 60_000_000,
            to_micros: 70_000_000,
        }),
    })?;
    let job = plan.job_id().clone();
    let running = {
        let runner = Arc::clone(&runner);
        let request = resumed(&job);
        tokio::spawn(async move { runner.job_resume(request).await })
    };
    until_held(&recogniser).await?;
    tokio::time::sleep(vsift::PROGRESS_RECORD_INTERVAL * 2).await;

    let other = engine(&root, &recogniser);
    let status = other.job_status(&job)?;
    let reported: Vec<(PrepareStream, Option<JobState>)> = status
        .streams()
        .iter()
        .map(|stream| (stream.stream(), stream.state()))
        .collect();
    assert_eq!(
        reported,
        [
            (PrepareStream::FirstWindow, Some(JobState::Succeeded)),
            (PrepareStream::Visual, Some(JobState::Succeeded)),
            (PrepareStream::Transcript, Some(JobState::Running)),
        ]
    );
    // The whole index is there: candidates analyse nothing.
    let warm = other
        .candidates(vsift::CandidatesRequest {
            session: session_id.clone(),
            range: vsift::CandidatesRange {
                from_micros: 0,
                to_micros: 140_000_000,
            },
            limit: Some(5),
            cursor: None,
            omission: vsift::CandidateOmission::Nothing,
            cancellation: Cancellation::new(),
        })
        .await?;
    assert_eq!(warm.analysed_now(), 0);
    // The window is read from the open whole run, and what it has not
    // published yet is covered by the window's own sealed revision.
    let window = other.transcript(TranscriptQuery {
        from_micros: 55_000_000,
        to_micros: 75_000_000,
        ..whole(&session_id, None)
    })?;
    assert_eq!(window.revision().state(), RevisionState::Open);

    recogniser.released.store(true, Ordering::SeqCst);
    let report = running.await??;
    assert_eq!(report.status().state(), JobState::Succeeded);
    // The window, [50, 80) s on the chunk grid, was recognised once: one
    // chunk for the first window and five of the whole run's six.
    assert_eq!(recogniser.calls.load(Ordering::SeqCst), 6);
    assert!(report.outcome().job().chunks_reused() >= 1);
    Ok(())
}

/// D6: a source with no video skips the visual stream, and the job goes on to
/// its transcript; the stream is reported as skipped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn a_source_without_video_skips_the_visual_stream() -> TestResult {
    let root = OwnedRoot::new()?;
    let recogniser = HeldRecogniser::default();
    recogniser.released.store(true, Ordering::SeqCst);
    let runner = engine(&root, &recogniser);
    let looped = looped_clip(&root)?;
    let audio_only = root.path("audio-only.mp4");
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-n", "-i"])
        .arg(&looped)
        .args(["-vn", "-c", "copy"])
        .arg(&audio_only)
        .stdin(Stdio::null())
        .status()?;
    if !status.success() {
        return Err("ffmpeg could not drop the video".into());
    }
    let session_id = session(&runner, &audio_only).await?;
    let plan = runner.job_start(&JobStartRequest {
        session: session_id,
        mode: Some(JobMode::Quick),
        first: None,
    })?;
    let report = runner.job_resume(resumed(plan.job_id())).await?;
    assert_eq!(report.status().state(), JobState::Succeeded);
    let phases: Vec<(PrepareStream, vsift::StreamPhase)> = report
        .status()
        .streams()
        .iter()
        .map(|stream| (stream.stream(), stream.phase()))
        .collect();
    assert_eq!(
        phases,
        [
            (PrepareStream::Visual, vsift::StreamPhase::Skipped),
            (PrepareStream::Transcript, vsift::StreamPhase::Done),
        ]
    );
    Ok(())
}
