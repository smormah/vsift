//! `vsift <video>` through the engine (ADR 0037): one run writes the readable
//! transcript, the subtitles and the screen changes with their frames beside
//! the video, names the video's hash and every frame's hash, and closes its
//! session; a second run is refused before anything is copied, and nothing the
//! first wrote is replaced; an output folder inside `VSift`'s own session root
//! is refused.
//!
//! The recogniser is a deterministic stand-in; the video is decoded and its
//! frames taken by real `FFmpeg` from a fixture of the corpus, so these tests
//! are opt-in:
//!
//! `cargo test -p vsift --locked --test engine_watch -- --ignored`
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
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest as _, Sha256};
use vsift::{
    AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider, AsrProviderBuild, Cancellation,
    ChunkTime, Clock, ClockError, CueText, Engine, EngineConfig, EngineError, EnginePorts,
    FailureCode, HostIsolation, IdentifierGenerationError, IdentifierSource, LanguageTag,
    LocalAsrVerification, LocalAsrVerifier, ManagedRootLocation, MediaToolVerification,
    MediaToolVerifier, OperationId, PlannedChunk, ProgressObserver, ProviderChunkOutput,
    ProviderSegment, RecognizerIdentity, SessionId, SessionPhase, SessionRootLocation, Sha256Hex,
    SpeechPcm, SpeechRecognitionError, SpeechRecognizer, UserConfigurationLocation, WatchFailure,
    WatchOutputKind, WatchRequest,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-engine-watch-test-";
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

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

struct Identifiers(AtomicU64);

impl IdentifierSource for Identifiers {
    fn session_id(&self) -> Result<SessionId, IdentifierGenerationError> {
        let next = self.0.fetch_add(1, Ordering::SeqCst);
        SessionId::parse(format!("ses_{next:032x}"))
            .map_err(|_| IdentifierGenerationError::NonCanonical)
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

/// Says one sentence two seconds into every chunk, naming the chunk.
#[derive(Clone)]
struct Sentences;

impl SpeechRecognizer for Sentences {
    fn identity(
        &self,
    ) -> impl Future<Output = Result<RecognizerIdentity, SpeechRecognitionError>> + Send {
        std::future::ready(
            Sha256Hex::parse(DIGEST)
                .map_err(|_| SpeechRecognitionError::Io)
                .map(|digest| RecognizerIdentity {
                    provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, digest.clone()),
                    model: AsrModel::new(AsrModelProfile::Base, digest),
                    decoding: AsrDecodingProfile::R0V1,
                    threads: NonZeroU16::MIN,
                }),
        )
    }

    fn recognize(
        &self,
        chunk: &PlannedChunk,
        _pcm: &SpeechPcm,
    ) -> impl Future<Output = Result<ProviderChunkOutput, SpeechRecognitionError>> + Send {
        let text = format!("chunk {} says hello --> world", chunk.index());
        std::future::ready((|| {
            Ok(ProviderChunkOutput {
                language: LanguageTag::parse("en").ok(),
                segments: vec![ProviderSegment {
                    start: ChunkTime::from_millis(2_000).ok_or(SpeechRecognitionError::Io)?,
                    end: ChunkTime::from_millis(4_000).ok_or(SpeechRecognitionError::Io)?,
                    text: Some(
                        CueText::new(text.clone(), text).map_err(|_| SpeechRecognitionError::Io)?,
                    ),
                    tokens: Vec::new(),
                }],
            })
        })())
    }
}

#[derive(Clone)]
struct Passing;

impl LocalAsrVerifier for Passing {
    fn verify(&self) -> impl Future<Output = LocalAsrVerification> + Send {
        std::future::ready(LocalAsrVerification::Verified)
    }
}

impl MediaToolVerifier for Passing {
    fn verify(&self) -> impl Future<Output = MediaToolVerification> + Send {
        std::future::ready(MediaToolVerification::Verified)
    }
}

fn engine(root: &OwnedRoot) -> Engine {
    Engine::new(
        EngineConfig {
            session_root: SessionRootLocation::Explicit(root.path("sessions")),
            user_configuration: UserConfigurationLocation::Explicit(root.path("config")),
            managed_root: ManagedRootLocation::Explicit(root.path("managed")),
            host_isolation: HostIsolation::ProcessOnly,
        },
        EnginePorts::new(Wall, Identifiers(AtomicU64::new(1)))
            .with_media_tool_verifier(Passing)
            .with_speech_recognizer(Sentences, Passing),
    )
}

/// A copy of a corpus fixture with speech and a changing picture, in its own
/// folder, as a person's video would be.
fn video(root: &OwnedRoot) -> Built<PathBuf> {
    let folder = root.path("videos");
    fs::create_dir(&folder)?;
    let video = folder.join("team meeting.mp4");
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/corpus/generated/F05-speech.mp4"),
        &video,
    )?;
    Ok(video)
}

fn request(source: &Path, output: Option<PathBuf>) -> WatchRequest {
    WatchRequest {
        source: source.to_path_buf(),
        output,
        cancellation: Cancellation::new(),
        progress: ProgressObserver::none(),
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut text, byte| {
            let _infallible = write!(text, "{byte:02x}");
            text
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn one_run_writes_the_three_outputs_beside_the_video_and_closes_its_session() -> TestResult {
    let root = OwnedRoot::new()?;
    let engine = engine(&root);
    let video = video(&root)?;
    let outcome = engine.watch(request(&video, None)).await?;
    let folder = video.parent().ok_or("no folder")?;

    let transcript = fs::read_to_string(folder.join("team meeting.txt"))?;
    let video_sha256 = hex(&fs::read(&video)?);
    assert!(transcript.starts_with("team meeting.mp4: transcript by VSift "));
    assert!(transcript.contains(&format!("Video SHA-256    {video_sha256}\n")));
    let revision = outcome.transcript_revision.as_ref().ok_or("no revision")?;
    assert!(transcript.contains(&format!("Transcript       {}", revision.as_str())));
    assert!(transcript.contains("] chunk 0 says hello --> world\n"));

    let subtitles = fs::read_to_string(folder.join("team meeting.srt"))?;
    assert!(
        subtitles.starts_with("1\n00:00:02,000 --> 00:00:04,000\nchunk 0 says hello -> world\n\n")
    );

    let list = fs::read_to_string(folder.join("team meeting-vsift/screen-changes.txt"))?;
    assert!(list.contains(&format!("Video SHA-256    {video_sha256}\n")));
    assert_eq!(
        outcome.frames.len(),
        outcome.changes_found.min(vsift::MAX_WATCH_FRAMES)
    );
    for frame in &outcome.frames {
        assert_eq!(
            hex(&fs::read(&frame.path)?),
            frame.sha256,
            "the frame is the evidence"
        );
        assert!(list.contains(frame.evidence_id.as_str()));
        assert!(list.contains(&frame.sha256));
    }

    assert!(outcome.session_closed);
    assert_eq!(
        engine.session_status(&outcome.session_id)?.phase(),
        SessionPhase::Closed
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn a_second_run_is_refused_and_replaces_nothing() -> TestResult {
    let root = OwnedRoot::new()?;
    let engine = engine(&root);
    let video = video(&root)?;
    engine.watch(request(&video, None)).await?;
    let folder = video.parent().ok_or("no folder")?;
    let before = fs::read(folder.join("team meeting.txt"))?;

    let refused = engine.watch(request(&video, None)).await;
    assert!(matches!(
        refused,
        Err(EngineError::Watch(WatchFailure::OutputExists(
            WatchOutputKind::Transcript
        )))
    ));
    assert_eq!(fs::read(folder.join("team meeting.txt"))?, before);
    assert_eq!(
        fs::read_dir(folder)?.count(),
        4,
        "the video and the first run's outputs"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn an_output_folder_is_created_and_never_inside_the_session_root() -> TestResult {
    let root = OwnedRoot::new()?;
    let engine = engine(&root);
    let video = video(&root)?;
    let output = root.path("shared");
    let outcome = engine.watch(request(&video, Some(output.clone()))).await?;
    assert!(output.join("team meeting.txt").is_file());
    assert_eq!(
        outcome.transcript.path,
        fs::canonicalize(&output)?.join("team meeting.txt")
    );

    fs::create_dir_all(root.path("sessions/inside"))?;
    let refused = engine
        .watch(request(&video, Some(root.path("sessions/inside"))))
        .await;
    let Err(error) = refused else {
        return Err("an output inside the session root was accepted".into());
    };
    assert_eq!(error.failure_code(), FailureCode::InvalidArgument);
    assert!(matches!(
        error,
        EngineError::Watch(WatchFailure::OutputInsideVsift)
    ));
    Ok(())
}

/// A generated video that shows a new still picture every two seconds for
/// eight minutes, so
/// the visual stream finds more screen changes than one session has evidence
/// room to take frames for, with a tone so the stand-in recogniser has audio.
/// No real media: `FFmpeg` makes it from its own test sources.
fn busy_video(root: &OwnedRoot) -> Built<PathBuf> {
    let folder = root.path("videos");
    fs::create_dir(&folder)?;
    let video = folder.join("busy screen.mp4");
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "nullsrc=s=160x96:r=0.5:d=480,geq=lum='random(1)*255':cb=128:cr=128,fps=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=16000:duration=480",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ])
        .arg(&video)
        .status()?;
    if !status.success() {
        return Err("FFmpeg could not generate the busy video".into());
    }
    Ok(video)
}

/// Each frame takes two of the session's evidence slots (the PNG and its
/// record), so a video with more changes than the session has room for must
/// get the frames that fit, every change listed, and never `RESOURCE_LIMIT`
/// (the real-media run's RM03, 576 changes, reproduced without real media).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn more_changes_than_the_session_has_room_for_still_finish() -> TestResult {
    let root = OwnedRoot::new()?;
    let engine = engine(&root);
    let video = busy_video(&root)?;
    let outcome = engine.watch(request(&video, None)).await?;
    assert!(
        outcome.changes_found > 192,
        "the video must find more changes than 384 evidence slots hold frames for: {}",
        outcome.changes_found
    );
    assert_eq!(outcome.frames.len(), vsift::MAX_WATCH_FRAMES);
    let list = fs::read_to_string(
        video
            .with_file_name("busy screen-vsift")
            .join("screen-changes.txt"),
    )?;
    assert!(list.contains("(not written)"));
    for frame in &outcome.frames {
        assert_eq!(hex(&fs::read(&frame.path)?), frame.sha256);
    }
    assert!(outcome.session_closed);
    Ok(())
}

/// The frame cap fits a fresh session's evidence room by design: each frame
/// takes `EVIDENCE_ARTIFACTS_PER_FRAME` of the `MAX_EVIDENCE_ARTIFACTS` slots
/// (known limit L-234). Raising either side breaks this, not a run on RM03.
#[test]
fn the_frame_cap_fits_a_fresh_sessions_evidence_room() {
    // `frames_that_fit` never gives more than half the room, so the whole cap
    // from a fresh session's evidence budget means two slots a frame fit it.
    assert_eq!(
        vsift::frames_that_fit(vsift_infrastructure::MAX_EVIDENCE_ARTIFACTS),
        vsift::MAX_WATCH_FRAMES
    );
}
