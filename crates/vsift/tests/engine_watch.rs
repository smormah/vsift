//! `vsift <video>` through the engine (ADR 0037): one run writes the readable
//! transcript, the subtitles and the screen changes with their frames beside
//! the video, names the video's hash and every frame's hash, and closes its
//! session; a second run is refused before anything is copied, and nothing the
//! first wrote is replaced; an output folder inside `VSift`'s own session root
//! is refused. With `--keep`, a finished run leaves a bundle that `bundle
//! search` finds, even when its final write fails.
//!
//! The recogniser is a deterministic stand-in; the video is decoded and its
//! frames taken by real `FFmpeg` from a fixture of the corpus, so these tests
//! are opt-in:
//!
//! `cargo test -p vsift --locked --test engine_watch -- --ignored`
//!
//! with `ffmpeg` and `ffprobe` on `PATH`. The `Nightly` workflow runs them on
//! Ubuntu; the pull-request gate does not.

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
    AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider, AsrProviderBuild,
    BundleSearchRequest, Cancellation, ChunkTime, Clock, ClockError, CueText, Engine, EngineConfig,
    EngineError, EnginePorts, FailureCode, HostIsolation, IdentifierGenerationError,
    IdentifierSource, LanguageTag, LocalAsrVerification, LocalAsrVerifier, ManagedRootLocation,
    MediaToolVerification, MediaToolVerifier, OperationId, PlannedChunk, ProgressObserver,
    ProviderChunkOutput, ProviderSegment, RecognizerIdentity, SessionId, SessionPhase,
    SessionRootLocation, Sha256Hex, SpeechPcm, SpeechRecognitionError, SpeechRecognizer,
    UserConfigurationLocation, WatchFailure, WatchOutputKind, WatchRequest,
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
        keep: None,
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
        outcome.changes_listed.min(vsift::MAX_WATCH_FRAMES)
    );
    assert!(outcome.changes_listed <= outcome.changes_found);
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

/// ADR 0037 section 8: a run stopped once the speech starts keeps its files,
/// each saying it is unfinished, and its session; the same request then
/// continues the job in that session and finishes the same files.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn a_stopped_run_keeps_its_session_and_the_same_request_continues_it() -> TestResult {
    let root = OwnedRoot::new()?;
    let engine = engine(&root);
    let video = video(&root)?;
    let cancellation = Cancellation::new();
    let stop = cancellation.clone();
    let stopped = engine
        .watch(WatchRequest {
            source: video.clone(),
            output: None,
            keep: None,
            cancellation,
            progress: ProgressObserver::new(move |progress| {
                if progress.update.stage == vsift::ProgressStage::RecognisingSpeech {
                    stop.cancel();
                }
            }),
        })
        .await;
    assert!(
        matches!(
            stopped,
            Err(EngineError::Watch(WatchFailure::Stopped { .. }))
        ),
        "{stopped:?}"
    );
    let folder = video.parent().ok_or("no folder")?;
    let text = fs::read_to_string(folder.join("team meeting.txt"))?;
    assert!(text.starts_with("STOPPED: "), "{text}");

    let outcome = engine.watch(request(&video, None)).await?;
    let text = fs::read_to_string(folder.join("team meeting.txt"))?;
    assert!(!text.contains("IN PROGRESS"), "{text}");
    assert!(text.contains("] chunk 0 says hello --> world\n"), "{text}");
    assert!(outcome.session_closed);
    assert_eq!(
        fs::read_dir(folder)?.count(),
        4,
        "the video and the three outputs, and no temporary file"
    );
    Ok(())
}

/// Every file called `name` under `folder`.
fn files_named(folder: &Path, name: &str) -> Built<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(folder)? {
        let path = entry?.path();
        if path.is_dir() {
            found.extend(files_named(&path, name)?);
        } else if path.file_name() == Some(OsStr::new(name)) {
            found.push(path);
        }
    }
    Ok(found)
}

/// The planner's condition on call 2 (2026-10-29): the job's record of its
/// claimed names is untrusted session content and only a key. A record that
/// names another folder never matches the folder the rerun resolves for
/// itself, so the rerun refuses the files in its way and writes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn a_record_that_names_another_folder_is_refused() -> TestResult {
    let root = OwnedRoot::new()?;
    let engine = engine(&root);
    let video = video(&root)?;
    let cancellation = Cancellation::new();
    let stop = cancellation.clone();
    let stopped = engine
        .watch(WatchRequest {
            source: video.clone(),
            output: None,
            keep: None,
            cancellation,
            progress: ProgressObserver::new(move |progress| {
                if progress.update.stage == vsift::ProgressStage::RecognisingSpeech {
                    stop.cancel();
                }
            }),
        })
        .await;
    assert!(
        matches!(
            stopped,
            Err(EngineError::Watch(WatchFailure::Stopped { .. }))
        ),
        "{stopped:?}"
    );
    let records = files_named(&root.path("sessions"), "watch-outputs.json")?;
    let [record] = records.as_slice() else {
        return Err(format!("one record expected, found {records:?}").into());
    };
    let folder = fs::canonicalize(video.parent().ok_or("no folder")?)?;
    let elsewhere = root.path("elsewhere");
    fs::create_dir(&elsewhere)?;
    let text = fs::read_to_string(record)?;
    let recorded = serde_json::to_string(folder.to_str().ok_or("not unicode")?)?;
    let moved = serde_json::to_string(
        fs::canonicalize(&elsewhere)?
            .to_str()
            .ok_or("not unicode")?,
    )?;
    assert!(text.contains(&recorded), "{text}");
    fs::write(record, text.replace(&recorded, &moved))?;
    let before = fs::read(folder.join("team meeting.txt"))?;

    let refused = engine.watch(request(&video, None)).await;
    assert!(
        matches!(
            refused,
            Err(EngineError::Watch(WatchFailure::OutputExists(
                WatchOutputKind::Transcript
            )))
        ),
        "{refused:?}"
    );
    assert_eq!(fs::read(folder.join("team meeting.txt"))?, before);
    assert_eq!(
        fs::read_dir(&elsewhere)?.count(),
        0,
        "nothing is written there"
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
        "the video must find more changes than 384 evidence slots hold frames for"
    );
    assert!(
        outcome.changes_listed < outcome.changes_found,
        "a new picture every two seconds lists the settled screens, not every change"
    );
    assert_eq!(
        outcome.frames.len(),
        outcome.changes_listed.min(vsift::MAX_WATCH_FRAMES)
    );
    let list = fs::read_to_string(
        video
            .with_file_name("busy screen-vsift")
            .join("screen-changes.txt"),
    )?;
    assert!(list.contains("settled screens of"));
    assert_eq!(
        list.contains("no frame (evidence room)"),
        outcome.changes_listed > vsift::MAX_WATCH_FRAMES
    );
    for frame in &outcome.frames {
        assert_eq!(hex(&fs::read(&frame.path)?), frame.sha256);
    }
    assert!(outcome.session_closed);
    Ok(())
}

/// The frame cap fits a fresh session's evidence room by design: each frame
/// takes `EVIDENCE_ARTIFACTS_PER_FRAME` of the `MAX_EVIDENCE_ARTIFACTS` slots
/// (known limit L-234). Raising either side breaks this, not a run on RM03.
/// A second video, the same fixture under another name, in its own folder.
fn second_video(root: &OwnedRoot) -> Built<PathBuf> {
    let folder = root.path("more videos");
    fs::create_dir(&folder)?;
    let video = folder.join("second call.mp4");
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/corpus/generated/F05-speech.mp4"),
        &video,
    )?;
    Ok(video)
}

/// Every recording `bundle search` finds the words in, by its bundle's name.
fn searched(engine: &Engine, folder: &Path) -> Built<Vec<String>> {
    let results = engine.search_bundles(BundleSearchRequest {
        paths: vec![folder.to_path_buf()],
        query: "says hello".to_owned(),
        limit: Some(100),
        cursor: None,
        cancellation: Cancellation::new(),
    })?;
    let mut bundles: Vec<String> = results
        .outcome()
        .hits
        .iter()
        .map(|hit| hit.bundle.clone())
        .collect();
    bundles.sort();
    bundles.dedup();
    Ok(bundles)
}

fn keeping(source: &Path, keep: &Path) -> WatchRequest {
    WatchRequest {
        keep: Some(keep.to_path_buf()),
        ..request(source, None)
    }
}

/// The story end to end: the action with `--keep` for two recordings into one
/// folder, then `bundle search` on the folder finds the words of both.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn two_kept_recordings_are_found_by_a_search_of_their_folder() -> TestResult {
    let root = OwnedRoot::new()?;
    let engine = engine(&root);
    let keep = root.path("kept calls");
    let first = engine.watch(keeping(&video(&root)?, &keep)).await?;
    let second = engine.watch(keeping(&second_video(&root)?, &keep)).await?;
    for outcome in [&first, &second] {
        assert!(outcome.session_closed, "the session still closes");
        let kept = outcome.kept.as_ref().ok_or("nothing kept")?;
        assert_eq!(kept.failure, None);
        let bundle = engine.validate_bundle(&kept.path)?;
        assert!(
            matches!(
                bundle.source_retention(),
                vsift::SourceRetention::EvidenceOnly
            ),
            "the video is not copied in"
        );
    }
    assert_eq!(
        searched(&engine, &keep)?,
        vec!["second call".to_owned(), "team meeting".to_owned()]
    );
    Ok(())
}

/// A name already kept is refused before any work: nothing is written beside
/// the video, nothing kept is replaced, and no session is left.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn a_name_already_kept_is_refused_before_any_work() -> TestResult {
    let root = OwnedRoot::new()?;
    let engine = engine(&root);
    let video = video(&root)?;
    let keep = root.path("kept");
    fs::create_dir(&keep)?;
    fs::create_dir(keep.join("team meeting"))?;
    let refused = engine.watch(keeping(&video, &keep)).await;
    assert!(
        matches!(
            refused,
            Err(EngineError::Watch(WatchFailure::OutputExists(
                WatchOutputKind::Kept
            )))
        ),
        "{refused:?}"
    );
    assert_eq!(
        fs::read_dir(video.parent().ok_or("no folder")?)?.count(),
        1,
        "only the video is there"
    );
    assert_eq!(engine.list_sessions(None)?.entries().len(), 0);
    Ok(())
}

/// The planner's condition: with `--keep`, once the job has sealed the
/// session closes only after a bundle is committed. The folder is moved away
/// during the transcription, so the final write is refused on a sealed job.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn a_final_write_refused_on_a_sealed_job_still_gives_a_bundle() -> TestResult {
    let root = OwnedRoot::new()?;
    let engine = engine(&root);
    let video = video(&root)?;
    let keep = root.path("kept");
    let output = root.path("notes");
    let moved = root.path("notes moved");
    let (from, to) = (output.clone(), moved.clone());
    let failed = engine
        .watch(WatchRequest {
            output: Some(output),
            progress: ProgressObserver::new(move |progress| {
                if progress.update.stage == vsift::ProgressStage::RecognisingSpeech {
                    let _first_only = fs::rename(&from, &to);
                }
            }),
            ..keeping(&video, &keep)
        })
        .await;
    let Err(EngineError::WatchFailedAfterKeep(failed)) = failed else {
        return Err(format!("{failed:?}").into());
    };
    assert_eq!(failed.error.failure_code(), FailureCode::StorageIo);
    assert_eq!(failed.kept.failure, None);
    assert!(failed.session_closed, "the bundle is committed");
    assert_eq!(
        searched(&engine, &keep)?,
        vec!["team meeting".to_owned()],
        "the kept copy is not lost"
    );
    Ok(())
}

/// Two rows of the rerun table through a real job: the option is not
/// recorded, so a stopped run with it, run again without it, keeps nothing,
/// and a stopped run without it, run again with it, keeps.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn the_run_that_finishes_does_what_its_own_command_line_says() -> TestResult {
    for (first, again) in [(true, false), (false, true)] {
        let root = OwnedRoot::new()?;
        let engine = engine(&root);
        let video = video(&root)?;
        let keep = root.path("kept");
        let cancellation = Cancellation::new();
        let stop = cancellation.clone();
        let stopped = engine
            .watch(WatchRequest {
                keep: first.then(|| keep.clone()),
                cancellation,
                progress: ProgressObserver::new(move |progress| {
                    if progress.update.stage == vsift::ProgressStage::RecognisingSpeech {
                        stop.cancel();
                    }
                }),
                ..request(&video, None)
            })
            .await;
        assert!(
            matches!(
                stopped,
                Err(EngineError::Watch(WatchFailure::Stopped { .. }))
            ),
            "{stopped:?}"
        );
        assert!(
            !keep.join("team meeting").exists(),
            "a stopped run keeps nothing"
        );
        let outcome = engine
            .watch(WatchRequest {
                keep: again.then(|| keep.clone()),
                ..request(&video, None)
            })
            .await?;
        assert!(outcome.session_closed);
        assert_eq!(outcome.kept.is_some(), again);
        assert_eq!(keep.join("team meeting").is_dir(), again);
    }
    Ok(())
}

#[test]
fn the_frame_cap_fits_a_fresh_sessions_evidence_room() {
    // `frames_that_fit` never gives more than half the room, so the whole cap
    // from a fresh session's evidence budget means two slots a frame fit it.
    assert_eq!(
        vsift::frames_that_fit(vsift_infrastructure::MAX_EVIDENCE_ARTIFACTS),
        vsift::MAX_WATCH_FRAMES
    );
}
