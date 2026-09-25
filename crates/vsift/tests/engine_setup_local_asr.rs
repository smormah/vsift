//! `setup check`'s local-ASR report through the engine library (maintainer
//! decision D4, P07 increment 3c).
//!
//! Every tool is selected per call with an absolute path, so nothing here
//! depends on the machine's `PATH`. Plain files stand in for `FFmpeg`,
//! `FFprobe` and whisper.cpp: they resolve but never run. A host-supplied
//! recognizer and verifier stand in for whisper.cpp and the reviewed speech
//! fixture, so each test counts exactly when verification runs. Real tools
//! are exercised by the opt-in `p07_local_asr_e2e` checkpoint and
//! `setup_check_verifies_real_local_asr` in `p07_asr_qualification`.

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
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use vsift::{
    AsrDecodingProfile, AsrFailure, AsrFailureReason, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrStage, DEFAULT_LOCAL_ASR_CHECK_BUDGET, Engine, EngineConfig, EnginePorts,
    ExecutableSelections, HostIsolation, LanguageTag, LocalAsrCheckFailure, LocalAsrCheckOutcome,
    LocalAsrModelStatus, LocalAsrNotRunReason, LocalAsrSetupStatus, LocalAsrVerification,
    LocalAsrVerificationFailure, LocalAsrVerificationSource, LocalAsrVerifier, MediaToolCheck,
    MediaToolFailure, MediaToolVerification, MediaToolVerifier, PlannedChunk, ProviderChunkOutput,
    RecognizerIdentity, ReviewedAsrModel, SessionRootLocation, SetupCheckRequest, Sha256Hex,
    SpeechPcm, SpeechRecognitionError, SpeechRecognizer, UserConfigurationLocation,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-engine-setup-asr-test-";
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

    fn write(&self, name: &str, content: &[u8]) -> Built<PathBuf> {
        let path = self.path(name);
        fs::write(&path, content)?;
        Ok(path)
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

/// What the stand-in recognizer reports as its identity.
#[derive(Clone, Copy)]
enum Identity {
    Profile(AsrModelProfile),
    Unreadable,
}

#[derive(Clone)]
struct StandInRecognizer {
    identity: Identity,
    recognitions: Arc<AtomicUsize>,
}

impl StandInRecognizer {
    fn new(identity: Identity) -> Self {
        Self {
            identity,
            recognitions: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl SpeechRecognizer for StandInRecognizer {
    fn identity(
        &self,
    ) -> impl Future<Output = Result<RecognizerIdentity, SpeechRecognitionError>> + Send {
        let identity = match self.identity {
            Identity::Unreadable => Err(SpeechRecognitionError::ModelUnavailable),
            Identity::Profile(profile) => Sha256Hex::parse(DIGEST)
                .map_err(|_| SpeechRecognitionError::Io)
                .map(|digest| RecognizerIdentity {
                    provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, digest.clone()),
                    model: AsrModel::new(profile, digest),
                    decoding: AsrDecodingProfile::R0V1,
                    threads: NonZeroU16::MIN,
                }),
        };
        std::future::ready(identity)
    }

    fn recognize(
        &self,
        _chunk: &PlannedChunk,
        _pcm: &SpeechPcm,
    ) -> impl Future<Output = Result<ProviderChunkOutput, SpeechRecognitionError>> + Send {
        self.recognitions.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(ProviderChunkOutput {
            language: LanguageTag::parse("en").ok(),
            segments: Vec::new(),
        }))
    }
}

/// How the stand-in verifier answers.
#[derive(Clone, Copy)]
enum Verdict {
    Answer(LocalAsrVerification),
    /// Never answers, as a hung recognizer would.
    Hang,
}

#[derive(Clone)]
struct CountingVerifier {
    verdict: Verdict,
    calls: Arc<AtomicUsize>,
}

impl CountingVerifier {
    fn new(verdict: Verdict) -> Self {
        Self {
            verdict,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl LocalAsrVerifier for CountingVerifier {
    fn verify(&self) -> impl Future<Output = LocalAsrVerification> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let verdict = self.verdict;
        async move {
            match verdict {
                Verdict::Answer(answer) => answer,
                Verdict::Hang => std::future::pending().await,
            }
        }
    }
}

#[derive(Clone, Copy)]
struct FixedMediaTools(MediaToolVerification);

impl MediaToolVerifier for FixedMediaTools {
    fn verify(&self) -> impl Future<Output = MediaToolVerification> + Send {
        std::future::ready(self.0)
    }
}

struct Harness {
    root: OwnedRoot,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    whisper: PathBuf,
}

impl Harness {
    fn new() -> Built<Self> {
        let root = OwnedRoot::new()?;
        Ok(Self {
            ffmpeg: root.write("ffmpeg-stand-in.exe", b"not a program")?,
            ffprobe: root.write("ffprobe-stand-in.exe", b"not a program")?,
            whisper: root.write("whisper-stand-in.exe", b"not a program")?,
            root,
        })
    }

    fn engine(&self, ports: EnginePorts) -> Engine {
        Engine::new(
            EngineConfig {
                session_root: SessionRootLocation::Explicit(self.root.path("sessions")),
                user_configuration: UserConfigurationLocation::Explicit(self.root.path("config")),
                host_isolation: HostIsolation::ProcessOnly,
            },
            ports,
        )
    }

    /// An engine whose recognizer and verifier are the host's, with media
    /// tools that pass (or fail) their own verification.
    fn host_engine(
        &self,
        recognizer: StandInRecognizer,
        verifier: CountingVerifier,
        media: MediaToolVerification,
    ) -> Engine {
        self.engine(
            EnginePorts::system()
                .with_media_tool_verifier(FixedMediaTools(media))
                .with_speech_recognizer(recognizer, verifier),
        )
    }

    fn selections(&self) -> ExecutableSelections {
        ExecutableSelections {
            ffmpeg: Some(self.ffmpeg.clone()),
            ffprobe: Some(self.ffprobe.clone()),
            whisper: Some(self.whisper.clone()),
        }
    }

    async fn check(
        &self,
        engine: &Engine,
        selections: ExecutableSelections,
        budget: Duration,
    ) -> Built<LocalAsrSetupStatus> {
        Ok(*engine
            .check_setup(SetupCheckRequest {
                probe_timeout: Duration::from_secs(5),
                per_call: selections,
                local_asr_budget: budget,
            })
            .await?
            .local_asr())
    }
}

const fn not_run(model: LocalAsrModelStatus, reason: LocalAsrNotRunReason) -> LocalAsrSetupStatus {
    LocalAsrSetupStatus {
        model,
        verification: LocalAsrCheckOutcome::NotRun(reason),
    }
}

/// With whisper.cpp, each missing piece is reported in the order a
/// retranscription resolves them, and nothing runs or is written.
#[tokio::test]
async fn whisper_setups_report_the_first_missing_piece_without_running_anything() -> TestResult {
    let harness = Harness::new()?;
    let verifier = CountingVerifier::new(Verdict::Answer(LocalAsrVerification::Verified));
    let engine = harness.engine(EnginePorts::system().with_local_asr_verifier(verifier.clone()));
    let missing = harness.root.path("missing.exe");

    let no_media = ExecutableSelections {
        ffprobe: Some(missing.clone()),
        ..harness.selections()
    };
    assert_eq!(
        harness
            .check(&engine, no_media, DEFAULT_LOCAL_ASR_CHECK_BUDGET)
            .await?,
        not_run(
            LocalAsrModelStatus::NotSelected,
            LocalAsrNotRunReason::MediaToolsUnavailable
        )
    );
    let no_whisper = ExecutableSelections {
        whisper: Some(missing),
        ..harness.selections()
    };
    assert_eq!(
        harness
            .check(&engine, no_whisper, DEFAULT_LOCAL_ASR_CHECK_BUDGET)
            .await?,
        not_run(
            LocalAsrModelStatus::NotSelected,
            LocalAsrNotRunReason::WhisperUnavailable
        )
    );
    assert_eq!(
        harness
            .check(
                &engine,
                harness.selections(),
                DEFAULT_LOCAL_ASR_CHECK_BUDGET
            )
            .await?,
        not_run(
            LocalAsrModelStatus::NotSelected,
            LocalAsrNotRunReason::ModelNotSelected
        )
    );

    let model = harness
        .root
        .write("ggml-model.bin", b"not a reviewed model")?;
    engine.configure_model(&model)?;
    assert_eq!(
        harness
            .check(
                &engine,
                harness.selections(),
                DEFAULT_LOCAL_ASR_CHECK_BUDGET
            )
            .await?,
        not_run(
            LocalAsrModelStatus::Unrecognised,
            LocalAsrNotRunReason::ModelNotPinned
        )
    );
    fs::remove_file(&model)?;
    assert_eq!(
        harness
            .check(
                &engine,
                harness.selections(),
                DEFAULT_LOCAL_ASR_CHECK_BUDGET
            )
            .await?,
        not_run(
            LocalAsrModelStatus::Unreadable,
            LocalAsrNotRunReason::ModelNotPinned
        )
    );
    assert_eq!(verifier.calls(), 0);
    assert!(!harness.root.path("sessions").exists());
    Ok(())
}

/// A pass is run once, recorded, then reported from the record; the check
/// never creates a session.
#[tokio::test]
async fn a_pass_runs_once_and_is_then_reported_from_the_record() -> TestResult {
    for profile in [ReviewedAsrModel::Base, ReviewedAsrModel::BaseQ5_1] {
        let harness = Harness::new()?;
        let verifier = CountingVerifier::new(Verdict::Answer(LocalAsrVerification::Verified));
        let recognizer = StandInRecognizer::new(Identity::Profile(profile.profile()));
        let engine = harness.host_engine(
            recognizer.clone(),
            verifier.clone(),
            MediaToolVerification::Verified,
        );
        for source in [
            LocalAsrVerificationSource::RanNow,
            LocalAsrVerificationSource::Recorded,
        ] {
            assert_eq!(
                harness
                    .check(
                        &engine,
                        harness.selections(),
                        DEFAULT_LOCAL_ASR_CHECK_BUDGET
                    )
                    .await?,
                LocalAsrSetupStatus {
                    model: LocalAsrModelStatus::KnownPinned(profile),
                    verification: LocalAsrCheckOutcome::Verified(source),
                }
            );
        }
        assert_eq!(verifier.calls(), 1);
        assert_eq!(recognizer.recognitions.load(Ordering::SeqCst), 0);
        assert!(!harness.root.path("sessions").exists());
    }
    Ok(())
}

/// Every verification failure is reported with its check and reason and is
/// never recorded, so the next check verifies again.
#[tokio::test]
async fn failures_are_typed_and_never_recorded() -> TestResult {
    for (failure, check, reason) in [
        (
            LocalAsrVerificationFailure::FixtureIntegrity,
            "preparation",
            "fixture_integrity",
        ),
        (
            LocalAsrVerificationFailure::Workspace,
            "preparation",
            "workspace",
        ),
        (
            LocalAsrVerificationFailure::FixtureMedia,
            "fixture_probe",
            "fixture_media",
        ),
        (
            LocalAsrVerificationFailure::Transcription(AsrFailure {
                stage: AsrStage::Recognition,
                reason: AsrFailureReason::AbnormalTermination,
            }),
            "recognition",
            "abnormal_termination",
        ),
        (
            LocalAsrVerificationFailure::UnexpectedTranscript,
            "transcript",
            "unexpected_transcript",
        ),
    ] {
        let harness = Harness::new()?;
        let verifier =
            CountingVerifier::new(Verdict::Answer(LocalAsrVerification::Failed(failure)));
        let engine = harness.host_engine(
            StandInRecognizer::new(Identity::Profile(AsrModelProfile::Base)),
            verifier.clone(),
            MediaToolVerification::Verified,
        );
        for _ in 0..2 {
            let status = harness
                .check(
                    &engine,
                    harness.selections(),
                    DEFAULT_LOCAL_ASR_CHECK_BUDGET,
                )
                .await?;
            let LocalAsrCheckOutcome::Failed(reported) = status.verification else {
                return Err(format!("{failure:?} was not reported as failed").into());
            };
            assert_eq!(reported, LocalAsrCheckFailure::Verification(failure));
            assert_eq!((reported.check(), reported.reason()), (check, reason));
        }
        assert_eq!(verifier.calls(), 2, "{failure:?}");
    }
    Ok(())
}

/// A verification that outlives the check's own budget is cancelled and
/// reported as `budget_exceeded`, well before the default budget.
#[tokio::test]
async fn a_verification_past_the_budget_is_stopped_and_reported() -> TestResult {
    let harness = Harness::new()?;
    let verifier = CountingVerifier::new(Verdict::Hang);
    let engine = harness.host_engine(
        StandInRecognizer::new(Identity::Profile(AsrModelProfile::Base)),
        verifier.clone(),
        MediaToolVerification::Verified,
    );
    let started = Instant::now();
    let status = harness
        .check(&engine, harness.selections(), Duration::from_millis(100))
        .await?;
    assert_eq!(
        status.verification,
        LocalAsrCheckOutcome::Failed(LocalAsrCheckFailure::BudgetExceeded)
    );
    assert_eq!(verifier.calls(), 1);
    assert!(started.elapsed() < DEFAULT_LOCAL_ASR_CHECK_BUDGET);
    Ok(())
}

/// An unpinned or unidentifiable model, or media tools that fail their own
/// check, stop before the local-ASR verification runs.
#[tokio::test]
async fn unpinned_models_and_failing_media_tools_are_not_verified() -> TestResult {
    let failing_media = MediaToolVerification::Failed {
        check: MediaToolCheck::Probe,
        failure: MediaToolFailure::UnexpectedResult,
    };
    for (identity, media, expected) in [
        (
            Identity::Profile(AsrModelProfile::Unreviewed),
            MediaToolVerification::Verified,
            not_run(
                LocalAsrModelStatus::Unrecognised,
                LocalAsrNotRunReason::ModelNotPinned,
            ),
        ),
        (
            Identity::Unreadable,
            MediaToolVerification::Verified,
            not_run(
                LocalAsrModelStatus::Unreadable,
                LocalAsrNotRunReason::ModelNotPinned,
            ),
        ),
        (
            Identity::Profile(AsrModelProfile::BaseQ5_1),
            failing_media,
            not_run(
                LocalAsrModelStatus::KnownPinned(ReviewedAsrModel::BaseQ5_1),
                LocalAsrNotRunReason::MediaToolsUnavailable,
            ),
        ),
    ] {
        let harness = Harness::new()?;
        let verifier = CountingVerifier::new(Verdict::Answer(LocalAsrVerification::Verified));
        let engine = harness.host_engine(StandInRecognizer::new(identity), verifier.clone(), media);
        assert_eq!(
            harness
                .check(
                    &engine,
                    harness.selections(),
                    DEFAULT_LOCAL_ASR_CHECK_BUDGET
                )
                .await?,
            expected
        );
        assert_eq!(verifier.calls(), 0);
    }
    Ok(())
}
