//! `transcript retranscribe` through the engine library (P07 increment 3b).
//!
//! A host-supplied recognizer and verifier stand in for whisper.cpp, and a
//! passing media-tool verifier for the reviewed fixture, so these tests count
//! exactly when recognition and verification run. Everything decided before
//! audio is decoded runs everywhere: `ingest` and `transcript get` never call
//! the recognizer (D1), dependencies and the model are checked before any
//! work, an unpinned model is refused (D5), and a failed verification writes
//! nothing. The opt-in tests decode real speech with `FFmpeg`, so they need
//! `ffmpeg` and `ffprobe` on `PATH`:
//!
//! `cargo test -p vsift --locked --test engine_retranscribe -- --ignored`

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    future::Future,
    num::NonZeroU16,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use vsift::{
    AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider, AsrProviderBuild, Cancellation,
    ChunkTime, CueText, Engine, EngineConfig, EngineError, EnginePorts, FailureCode, HostIsolation,
    IngestRequest, LanguageTag, LocalAsrVerification, LocalAsrVerificationFailure,
    LocalAsrVerifier, MediaToolVerification, MediaToolVerifier, PlannedChunk, ProviderChunkOutput,
    ProviderSegment, ProviderToken, ProviderTokenKind, RecognizerIdentity, RetranscribeRange,
    RetranscribeRequest, RuntimeDependency, SessionId, SessionRootLocation, Sha256Hex, SpeechPcm,
    SpeechRecognitionError, SpeechRecognizer, TranscriptQuery, TranscriptWarningKind,
    UserConfigurationLocation,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-engine-retranscribe-test-";
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const PLACEHOLDER_SOURCE: &[u8] = b"\0\0\0\x18ftypisomengine-retranscribe-source";
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

/// What the fake recognizer answers for each chunk.
#[derive(Clone, Copy)]
enum Speech {
    /// One segment from 0.5 s to 3 s of every chunk.
    Words,
    /// Nothing at all.
    Nothing,
}

/// A recognizer double that counts every call.
#[derive(Clone)]
struct CountingRecognizer {
    profile: AsrModelProfile,
    speech: Speech,
    identities: Arc<AtomicUsize>,
    recognitions: Arc<AtomicUsize>,
}

impl CountingRecognizer {
    fn new(profile: AsrModelProfile, speech: Speech) -> Self {
        Self {
            profile,
            speech,
            identities: Arc::new(AtomicUsize::new(0)),
            recognitions: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn calls(&self) -> (usize, usize) {
        (
            self.identities.load(Ordering::SeqCst),
            self.recognitions.load(Ordering::SeqCst),
        )
    }
}

impl SpeechRecognizer for CountingRecognizer {
    fn identity(
        &self,
    ) -> impl Future<Output = Result<RecognizerIdentity, SpeechRecognitionError>> + Send {
        std::future::ready(self.answer_identity())
    }

    fn recognize(
        &self,
        chunk: &PlannedChunk,
        _pcm: &SpeechPcm,
    ) -> impl Future<Output = Result<ProviderChunkOutput, SpeechRecognitionError>> + Send {
        std::future::ready(self.answer(chunk))
    }
}

impl CountingRecognizer {
    fn answer_identity(&self) -> Result<RecognizerIdentity, SpeechRecognitionError> {
        self.identities.fetch_add(1, Ordering::SeqCst);
        let digest = Sha256Hex::parse(DIGEST).map_err(|_| SpeechRecognitionError::Io)?;
        Ok(RecognizerIdentity {
            provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, digest.clone()),
            model: AsrModel::new(self.profile, digest),
            decoding: AsrDecodingProfile::R0V1,
            threads: NonZeroU16::MIN,
        })
    }

    fn answer(&self, chunk: &PlannedChunk) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
        self.recognitions.fetch_add(1, Ordering::SeqCst);
        let segments = match self.speech {
            Speech::Nothing => Vec::new(),
            Speech::Words => {
                let words = format!("words heard in chunk {}", chunk.index());
                vec![ProviderSegment {
                    start: ChunkTime::from_millis(500).ok_or(SpeechRecognitionError::Io)?,
                    end: ChunkTime::from_millis(3_000).ok_or(SpeechRecognitionError::Io)?,
                    text: Some(
                        CueText::new(words.clone(), words)
                            .map_err(|_| SpeechRecognitionError::Io)?,
                    ),
                    tokens: vec![ProviderToken {
                        kind: ProviderTokenKind::Text,
                        probability: 0.9,
                    }],
                }]
            }
        };
        Ok(ProviderChunkOutput {
            language: LanguageTag::parse("en").ok(),
            segments,
        })
    }
}

#[derive(Clone)]
struct CountingAsrVerifier {
    result: LocalAsrVerification,
    calls: Arc<AtomicUsize>,
}

impl CountingAsrVerifier {
    fn returning(result: LocalAsrVerification) -> Self {
        Self {
            result,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl LocalAsrVerifier for CountingAsrVerifier {
    fn verify(&self) -> impl Future<Output = LocalAsrVerification> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::future::ready(self.result)
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
    recognizer: CountingRecognizer,
    verifier: CountingAsrVerifier,
}

impl Harness {
    fn new(recognizer: CountingRecognizer, verifier: CountingAsrVerifier) -> Built<Self> {
        Ok(Self {
            root: OwnedRoot::new()?,
            recognizer,
            verifier,
        })
    }

    fn passing(profile: AsrModelProfile, speech: Speech) -> Built<Self> {
        Self::new(
            CountingRecognizer::new(profile, speech),
            CountingAsrVerifier::returning(LocalAsrVerification::Verified),
        )
    }

    fn engine(&self) -> Engine {
        self.engine_with(
            EnginePorts::system()
                .with_media_tool_verifier(PassingMediaTools)
                .with_speech_recognizer(self.recognizer.clone(), self.verifier.clone()),
        )
    }

    fn engine_with(&self, ports: EnginePorts) -> Engine {
        Engine::new(
            EngineConfig {
                session_root: SessionRootLocation::Explicit(self.root.path("sessions")),
                user_configuration: UserConfigurationLocation::Explicit(self.root.path("config")),
                host_isolation: HostIsolation::ProcessOnly,
            },
            ports,
        )
    }

    fn write(&self, name: &str, content: &[u8]) -> Built<PathBuf> {
        let path = self.root.path(name);
        fs::write(&path, content)?;
        Ok(path)
    }

    /// Plain files registered as the media tools: resolvable, never runnable.
    fn stand_in_tools(&self, engine: &Engine) -> TestResult {
        let ffmpeg = self.write("ffmpeg-stand-in.exe", b"not a program")?;
        let ffprobe = self.write("ffprobe-stand-in.exe", b"not a program")?;
        engine.configure_executable(RuntimeDependency::Ffmpeg, &ffmpeg)?;
        engine.configure_executable(RuntimeDependency::Ffprobe, &ffprobe)?;
        Ok(())
    }

    async fn plain_session(&self, engine: &Engine, source: PathBuf) -> Built<SessionId> {
        Ok(engine
            .ingest(IngestRequest {
                source,
                transcript: None,
            })
            .await?
            .session
            .session_id)
    }
}

fn request(session: &SessionId, range: Option<(u64, u64)>) -> RetranscribeRequest {
    RetranscribeRequest {
        session: session.clone(),
        range: range.map(|(from_micros, to_micros)| RetranscribeRange {
            from_micros,
            to_micros,
        }),
        cancellation: Cancellation::new(),
    }
}

fn read(session: &SessionId, revision: Option<vsift::TranscriptRevisionId>) -> TranscriptQuery {
    TranscriptQuery {
        session: session.clone(),
        revision,
        from_micros: 0,
        to_micros: 60 * SECOND,
        limit: Some(100),
        cursor: None,
    }
}

/// D1: plain ingest and transcript reads never resolve, identify or run a recognizer.
#[tokio::test]
async fn ingest_and_transcript_get_never_call_the_recognizer() -> TestResult {
    let harness = Harness::passing(AsrModelProfile::Base, Speech::Words)?;
    let engine = harness.engine();
    let source = harness.write("placeholder.mp4", PLACEHOLDER_SOURCE)?;
    let session = harness.plain_session(&engine, source).await?;
    let missing = engine.transcript(read(&session, None));
    assert!(matches!(missing, Err(EngineError::TranscriptUnavailable)));
    assert_eq!(
        missing.err().map(|error| error.failure_code()),
        Some(FailureCode::InvalidArgument)
    );
    assert_eq!(harness.recognizer.calls(), (0, 0));
    assert_eq!(harness.verifier.calls(), 0);
    Ok(())
}

/// D5: an unpinned model is refused before any session access or verification.
#[tokio::test]
async fn unpinned_models_are_refused_before_any_work() -> TestResult {
    let harness = Harness::passing(AsrModelProfile::Unreviewed, Speech::Words)?;
    let engine = harness.engine();
    harness.stand_in_tools(&engine)?;
    let session = SessionId::parse("ses_0123456789abcdef")?;
    let error = engine
        .retranscribe(request(&session, None))
        .await
        .err()
        .ok_or("an unpinned model ran")?;
    assert!(matches!(error, EngineError::LocalAsrModelNotPinned));
    assert_eq!(error.failure_code(), FailureCode::MissingCapability);
    assert_eq!(harness.recognizer.calls(), (1, 0));
    assert_eq!(harness.verifier.calls(), 0);
    assert!(!harness.root.path("sessions").exists());
    Ok(())
}

/// Missing whisper.cpp or model, a model file `VSift` cannot identify as
/// pinned, and malformed ranges all fail before anything runs.
#[tokio::test]
async fn missing_dependencies_and_bad_ranges_fail_before_any_work() -> TestResult {
    let harness = Harness::passing(AsrModelProfile::Base, Speech::Words)?;
    let session = SessionId::parse("ses_0123456789abcdef")?;

    let whisper_only = harness.engine_with(EnginePorts::system());
    harness.stand_in_tools(&whisper_only)?;
    let whisper = harness.write("whisper-cli-stand-in.exe", b"not a program")?;
    whisper_only.configure_executable(RuntimeDependency::Whisper, &whisper)?;
    let no_model = whisper_only
        .retranscribe(request(&session, None))
        .await
        .err()
        .ok_or("ran without a model")?;
    assert!(matches!(no_model, EngineError::ModelNotSelected));
    assert_eq!(no_model.failure_code(), FailureCode::MissingCapability);

    let model = harness.write("not-the-base-model.bin", b"ggml model stand-in")?;
    whisper_only.configure_model(&model)?;
    let unpinned = whisper_only
        .retranscribe(request(&session, None))
        .await
        .err()
        .ok_or("ran an unpinned model")?;
    assert!(matches!(unpinned, EngineError::LocalAsrModelNotPinned));

    let engine = harness.engine();
    for (from, to) in [(5, 5), (7, 3)] {
        let invalid = engine
            .retranscribe(request(&session, Some((from, to))))
            .await
            .err()
            .ok_or("an empty range ran")?;
        assert!(matches!(invalid, EngineError::InvalidTimeRange));
        assert_eq!(invalid.failure_code(), FailureCode::InvalidArgument);
    }
    assert_eq!(harness.recognizer.calls(), (0, 0));
    assert!(!harness.root.path("sessions").exists());
    Ok(())
}

/// A failed local-ASR preflight is typed, runs no recognition and writes nothing.
#[tokio::test]
async fn a_failed_verification_writes_nothing() -> TestResult {
    let harness = Harness::new(
        CountingRecognizer::new(AsrModelProfile::Base, Speech::Words),
        CountingAsrVerifier::returning(LocalAsrVerification::Failed(
            LocalAsrVerificationFailure::UnexpectedTranscript,
        )),
    )?;
    let engine = harness.engine();
    harness.stand_in_tools(&engine)?;
    let source = harness.write("placeholder.mp4", PLACEHOLDER_SOURCE)?;
    let session = harness.plain_session(&engine, source).await?;
    let before = engine.session_status(&session)?;
    let error = engine
        .retranscribe(request(&session, None))
        .await
        .err()
        .ok_or("verification did not stop the run")?;
    assert!(matches!(
        error,
        EngineError::LocalAsrVerificationFailed(LocalAsrVerificationFailure::UnexpectedTranscript)
    ));
    assert_eq!(error.failure_code(), FailureCode::MissingCapability);
    assert_eq!(harness.verifier.calls(), 1);
    assert_eq!(harness.recognizer.calls().1, 0);
    let after = engine.session_status(&session)?;
    assert_eq!(after.generation(), before.generation());
    assert_eq!(after.artifact_count(), 0);
    // A missing session fails before verification runs.
    let unknown = engine
        .retranscribe(request(&SessionId::parse("ses_fedcba9876543210")?, None))
        .await
        .err()
        .ok_or("a missing session ran")?;
    assert_ne!(unknown.failure_code(), FailureCode::MissingCapability);
    assert_eq!(harness.verifier.calls(), 1);
    Ok(())
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/corpus/generated")
        .join(name)
}

/// D3/T-06: a whole-source run, then a bounded one, gives a complete spliced
/// revision that becomes the default read while the first stays readable;
/// the verification pass is recorded once.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn bounded_retranscription_splices_and_keeps_earlier_revisions() -> TestResult {
    let harness = Harness::passing(AsrModelProfile::Base, Speech::Words)?;
    let engine = harness.engine();
    let session = harness
        .plain_session(&engine, fixture("F05-speech.mp4"))
        .await?;
    let first = engine.retranscribe(request(&session, None)).await?;
    let first = first.revision().clone();
    assert_eq!(first.number(), 1);
    assert_eq!(first.supersedes(), None);
    assert_eq!(first.segments().len(), 1);
    assert_eq!(harness.verifier.calls(), 1);

    let outcome = engine
        .retranscribe(request(&session, Some((SECOND, 2 * SECOND))))
        .await?;
    let second = outcome.revision();
    assert_eq!(second.number(), 2);
    assert_eq!(second.supersedes(), Some(first.id()));
    // 1-2 s cuts the first revision's 0.5-3 s segment, so all of it is replaced.
    assert_eq!(
        second
            .replaced_range()
            .map(|range| (range.start().as_micros(), range.end().as_micros())),
        Some((500_000, 3 * SECOND))
    );
    assert_eq!(harness.verifier.calls(), 1, "the pass was not reused");

    let newest = engine.transcript(read(&session, None))?;
    assert_eq!(newest.revision().id(), second.id());
    let older = engine.transcript(read(&session, Some(first.id().clone())))?;
    assert_eq!(older.revision(), &first);
    assert_eq!(older.segments()[0].id(), first.segments()[0].id());
    let unknown = engine.transcript(read(
        &session,
        Some(vsift::TranscriptRevisionId::parse("trv_2222222222222222")?),
    ));
    assert!(matches!(
        unknown,
        Err(EngineError::TranscriptRevisionNotFound)
    ));

    // A range past the end of the video is refused, not clamped.
    let beyond = engine
        .retranscribe(request(&session, Some((19 * SECOND, 90 * SECOND))))
        .await
        .err()
        .ok_or("a range beyond the source ran")?;
    assert!(matches!(beyond, EngineError::RangeOutsideSource));
    assert_eq!(engine.session_status(&session)?.artifact_count(), 2);
    Ok(())
}

/// A run that hears nothing is recorded as a revision without segments.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn a_run_that_hears_no_speech_is_recorded() -> TestResult {
    let harness = Harness::passing(AsrModelProfile::Base, Speech::Nothing)?;
    let engine = harness.engine();
    let session = harness
        .plain_session(&engine, fixture("F01-speech.mp4"))
        .await?;
    let outcome = engine.retranscribe(request(&session, None)).await?;
    assert!(outcome.revision().segments().is_empty());
    assert!(
        outcome
            .revision()
            .warnings()
            .as_slice()
            .iter()
            .any(|warning| warning.kind() == TranscriptWarningKind::NoSpeechRecognised)
    );
    let read_back = engine.transcript(read(&session, None))?;
    assert!(read_back.segments().is_empty());
    Ok(())
}

/// A video without audio is a typed invalid argument; nothing is committed.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn a_video_without_audio_is_refused() -> TestResult {
    let harness = Harness::passing(AsrModelProfile::Base, Speech::Words)?;
    let engine = harness.engine();
    let session = harness.plain_session(&engine, fixture("F10.mp4")).await?;
    let error = engine
        .retranscribe(request(&session, None))
        .await
        .err()
        .ok_or("a silent video was transcribed")?;
    assert!(matches!(error, EngineError::NoAudioStream));
    assert_eq!(error.failure_code(), FailureCode::InvalidArgument);
    assert_eq!(harness.recognizer.calls().1, 0);
    assert_eq!(engine.session_status(&session)?.artifact_count(), 0);
    Ok(())
}
