//! P13 PR 3: the managed-install compatibility smoke and its failure cleanup
//! on real private staging (D-06).
//!
//! Each test stages a small fixture executable (`testbin/smoke_fixture_tool.rs`)
//! through the production path: exact-byte import, bounded tar payload staging
//! and the reviewed runtime layout. The smoke then runs it by explicit path
//! from the unactivated stage, with a private smoke directory in the same
//! stage, through the process supervisor. The fixture picks its behaviour
//! from the name it is staged under (a good or wrong banner, unbounded
//! output, a hang, a failed exit, or a write into its own installation).
//!
//! Every test checks that nothing was published or selected, and that each
//! stage was removed or, when its ownership or content cannot be proved,
//! kept untouched and reported.

use std::{
    env,
    error::Error,
    fs,
    future::Future,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use tar::{Builder, Header};
use vsift_application::{
    CompatibilitySmokeCheck as Check, CompatibilitySmokeFailure,
    CompatibilitySmokeFailureReason as Reason, LocalAsrVerification, LocalAsrVerificationFailure,
    LocalAsrVerifier, MediaToolCheck, MediaToolFailure, MediaToolVerification, MediaToolVerifier,
    ReviewedCompatibilityPolicy, SmokeStageOutcome, StageDisposal, StageRetentionReason,
    smoke_before_activation,
};
use vsift_domain::{ArtifactIntegrity, ManagedComponent, SharedLibraryName};
use vsift_infrastructure::{
    ArchiveInventoryBounds, HostIsolation, ManagedArtifactStore, ManagedCandidateFailure,
    ManagedRuntimeRole, MediaProviderConformance, MediaSmokeRequest, ProcessCancellation,
    ReviewedArchiveFile, ReviewedFixtureVerifiers, ReviewedPayloadArchive, ReviewedRuntimeLayout,
    SmokeCompanions, SmokeFixtureVerifiers, SpeechSmokeRequest, StagedCompatibilitySmoke,
    StagedManagedCandidate, TrustedExecutable, reviewed_compatibility_policy,
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const FIXTURE_TOOL: &str = env!("CARGO_BIN_EXE_vsift-smoke-fixture");
const FIXTURE_BUILD: &str = "vsift-smoke-fixture";
const SCRIBBLE_FILE: &str = "vsift-smoke-scribble";
const ROOT_MARKER: &str = "owner-v1";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A fresh temporary parent for one managed root, removed on drop.
struct TestRoot {
    parent: PathBuf,
}

impl TestRoot {
    fn new() -> TestResult<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let parent = env::temp_dir().join(format!(
            "vsift-p13-smoke-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&parent)?;
        Ok(Self { parent })
    }

    fn root(&self) -> PathBuf {
        self.parent.join("managed")
    }

    fn store(&self) -> TestResult<ManagedArtifactStore> {
        Ok(ManagedArtifactStore::at(self.root())?)
    }

    /// Every `stage-*` directory in the managed root.
    fn stages(&self) -> TestResult<Vec<PathBuf>> {
        let mut stages = Vec::new();
        for entry in fs::read_dir(self.root())? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with("stage-") {
                stages.push(entry.path());
            }
        }
        Ok(stages)
    }

    fn only_stage(&self) -> TestResult<PathBuf> {
        let mut stages = self.stages()?;
        match (stages.pop(), stages.is_empty()) {
            (Some(stage), true) => Ok(stage),
            _ => Err("expected exactly one stage".into()),
        }
    }

    /// Nothing was published or selected, and the store reports no runtime.
    fn assert_nothing_activated(&self) -> TestResult {
        let root = self.root();
        assert!(!root.join("versions-v1").exists());
        assert!(!root.join("current-v1").exists());
        let store = self.store()?;
        for component in ["ffmpeg_ffprobe", "whisper_cli", "whisper_model"] {
            assert!(store.open_selected_runtime(component)?.is_none());
        }
        Ok(())
    }

    /// The root holds only its ownership marker: every stage was removed.
    fn assert_only_marker_remains(&self) -> TestResult {
        let names: Vec<String> = fs::read_dir(self.root())?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<Result<_, _>>()?;
        assert_eq!(names, [ROOT_MARKER]);
        Ok(())
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.parent);
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(DIGITS[usize::from(byte >> 4)]));
        result.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    result
}

fn integrity_of(bytes: &[u8]) -> TestResult<ArtifactIntegrity> {
    Ok(ArtifactIntegrity::from_sha256_hex(
        u64::try_from(bytes.len())?,
        &hex(&Sha256::digest(bytes)),
    )?)
}

/// A staged fixture tool under the name `<tool>-<behaviour>` for this host.
fn tool(stem: &str) -> String {
    format!("{stem}{}", env::consts::EXE_SUFFIX)
}

fn fixture_tool_bytes() -> TestResult<Vec<u8>> {
    Ok(fs::read(FIXTURE_TOOL)?)
}

/// One file of a reviewed test archive.
struct ArchiveFile {
    name: String,
    bytes: Vec<u8>,
    executable: bool,
}

fn executable(stem: &str) -> TestResult<ArchiveFile> {
    Ok(ArchiveFile {
        name: tool(stem),
        bytes: fixture_tool_bytes()?,
        executable: true,
    })
}

/// Imports a reviewed tar of `files` and prepares it as one candidate.
fn stage(
    store: &ManagedArtifactStore,
    component: ManagedComponent,
    files: &[ArchiveFile],
    roles: &[(ManagedRuntimeRole, &str)],
) -> TestResult<StagedManagedCandidate> {
    let mut archive = Builder::new(Vec::new());
    let mut paths = Vec::with_capacity(files.len());
    let mut total = 0_u64;
    for file in files {
        let path = format!("root/{}", file.name);
        let mut header = Header::new_gnu();
        header.set_path(&path)?;
        header.set_size(u64::try_from(file.bytes.len())?);
        header.set_mode(0o755);
        header.set_cksum();
        archive.append(&header, Cursor::new(&file.bytes))?;
        total += u64::try_from(file.bytes.len())?;
        paths.push(path);
    }
    let bytes = archive.into_inner()?;
    let staged = store.import_verified(&bytes[..], integrity_of(&bytes)?)?;
    let selected = files
        .iter()
        .zip(&paths)
        .map(|(file, path)| {
            Ok(ReviewedArchiveFile {
                path: path.as_str(),
                integrity: integrity_of(&file.bytes)?,
            })
        })
        .collect::<TestResult<Vec<_>>>()?;
    let executables: Vec<&str> = files
        .iter()
        .filter(|file| file.executable)
        .map(|file| file.name.as_str())
        .collect();
    Ok(StagedManagedCandidate::prepare_archive(
        staged,
        component,
        roles,
        ReviewedPayloadArchive::Tar {
            max_tar_bytes: u64::try_from(bytes.len())?,
        },
        ArchiveInventoryBounds::new(files.len(), total)?,
        &[],
        &selected,
        ReviewedRuntimeLayout {
            max_bytes: total,
            aliases: &[],
            executables: &executables,
        },
    )?)
}

/// Media tools staged as `ffmpeg-<ffmpeg>` and `ffprobe-<ffprobe>`.
fn stage_media(
    store: &ManagedArtifactStore,
    ffmpeg: &str,
    ffprobe: &str,
) -> TestResult<StagedManagedCandidate> {
    let ffmpeg_stem = format!("ffmpeg-{ffmpeg}");
    let ffprobe_stem = format!("ffprobe-{ffprobe}");
    let ffmpeg = tool(&ffmpeg_stem);
    let ffprobe = tool(&ffprobe_stem);
    stage(
        store,
        ManagedComponent::MediaTools,
        &[executable(&ffmpeg_stem)?, executable(&ffprobe_stem)?],
        &[
            (ManagedRuntimeRole::Ffmpeg, &ffmpeg),
            (ManagedRuntimeRole::Ffprobe, &ffprobe),
        ],
    )
}

fn stage_model(store: &ManagedArtifactStore) -> TestResult<StagedManagedCandidate> {
    let bytes = b"fixture speech model bytes".to_vec();
    let staged = store.import_verified(&bytes[..], integrity_of(&bytes)?)?;
    Ok(StagedManagedCandidate::prepare_raw_file(
        staged,
        ManagedComponent::WhisperModel,
        &[(ManagedRuntimeRole::SpeechModel, "model.bin")],
        "model.bin",
    )?)
}

fn stage_whisper(
    store: &ManagedArtifactStore,
    behaviour: &str,
) -> TestResult<StagedManagedCandidate> {
    let name = tool(&format!("whisper-{behaviour}"));
    stage(
        store,
        ManagedComponent::WhisperCli,
        &[executable(&format!("whisper-{behaviour}"))?],
        &[(ManagedRuntimeRole::WhisperCli, &name)],
    )
}

/// The reviewed policy with the fixture tool's banners and a test deadline.
fn policy(media_deadline_seconds: u64) -> TestResult<ReviewedCompatibilityPolicy> {
    Ok(ReviewedCompatibilityPolicy {
        expected_ffmpeg_version: format!("ffmpeg version {FIXTURE_BUILD}"),
        expected_ffprobe_version: format!("ffprobe version {FIXTURE_BUILD}"),
        media_deadline_seconds,
        inference_deadline_seconds: 30,
        ..reviewed_compatibility_policy()?
    })
}

/// What the fixed verifiers were asked to run.
#[derive(Debug, Default)]
struct Requests {
    media: Vec<(PathBuf, PathBuf, PathBuf)>,
    speech: Vec<(PathBuf, PathBuf, PathBuf)>,
}

/// Verifiers with fixed verdicts that record the explicit paths they were
/// given; `litter` leaves a file in the smoke directory, as a provider that
/// escaped its workspace would.
#[derive(Clone)]
struct FixedVerifiers {
    media: MediaToolVerification,
    speech: LocalAsrVerification,
    litter: bool,
    requests: Arc<Mutex<Requests>>,
}

impl FixedVerifiers {
    fn passing() -> Self {
        Self {
            media: MediaToolVerification::Verified,
            speech: LocalAsrVerification::Verified,
            litter: false,
            requests: Arc::default(),
        }
    }

    fn requests(&self) -> std::sync::MutexGuard<'_, Requests> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct Fixed<T>(T);

impl MediaToolVerifier for Fixed<MediaToolVerification> {
    fn verify(&self) -> impl Future<Output = MediaToolVerification> + Send {
        std::future::ready(self.0)
    }
}

impl LocalAsrVerifier for Fixed<LocalAsrVerification> {
    fn verify(&self) -> impl Future<Output = LocalAsrVerification> + Send {
        std::future::ready(self.0)
    }
}

impl SmokeFixtureVerifiers for FixedVerifiers {
    type Media = Fixed<MediaToolVerification>;
    type Speech = Fixed<LocalAsrVerification>;

    fn media(&self, request: MediaSmokeRequest) -> Self::Media {
        if self.litter {
            let _ = fs::write(request.workspace_parent.join("left-behind"), b"x");
        }
        self.requests().media.push((
            request.tools.ffmpeg.path().to_path_buf(),
            request.tools.ffprobe.path().to_path_buf(),
            request.workspace_parent,
        ));
        Fixed(self.media)
    }

    fn speech(&self, request: SpeechSmokeRequest) -> Self::Speech {
        self.requests().speech.push((
            request.whisper.executable().path().to_path_buf(),
            request.whisper.model().to_path_buf(),
            request.workspace_parent,
        ));
        Fixed(self.speech)
    }
}

fn smoke<V>(policy: ReviewedCompatibilityPolicy, verifiers: V) -> StagedCompatibilitySmoke<V> {
    smoke_with(policy, SmokeCompanions::default(), verifiers)
}

fn smoke_with<V>(
    policy: ReviewedCompatibilityPolicy,
    companions: SmokeCompanions,
    verifiers: V,
) -> StagedCompatibilitySmoke<V> {
    StagedCompatibilitySmoke::new(
        policy,
        HostIsolation::ProcessOnly,
        companions,
        verifiers,
        ProcessCancellation::new(),
    )
}

/// The failure a smoke reported, with each stage's disposal.
fn failed<C>(
    outcome: SmokeStageOutcome<C>,
) -> TestResult<(
    CompatibilitySmokeFailure,
    Vec<(ManagedComponent, StageDisposal)>,
)> {
    match outcome {
        SmokeStageOutcome::Failed { failure, stages } => Ok((failure, stages)),
        SmokeStageOutcome::Passed(_) => Err("the smoke unexpectedly passed".into()),
    }
}

fn is_within(path: &Path, directory: &Path) -> TestResult<bool> {
    Ok(fs::canonicalize(path)?.starts_with(fs::canonicalize(directory)?))
}

fn companion_media() -> TestResult<MediaProviderConformance> {
    Ok(MediaProviderConformance::r0(
        TrustedExecutable::explicit(FIXTURE_TOOL)?,
        TrustedExecutable::explicit(FIXTURE_TOOL)?,
    ))
}

/// Good fixture tools pass every step, run only from the private stage, and
/// come back unactivated; the smoke directory is gone and discard removes
/// the stage completely.
#[tokio::test]
async fn passing_media_tools_run_from_the_stage_and_stay_unactivated() -> TestResult {
    let root = TestRoot::new()?;
    let store = root.store()?;
    let candidate = stage_media(&store, "good", "good")?;
    let stage_path = root.only_stage()?;
    let verifiers = FixedVerifiers::passing();
    let outcome =
        smoke_before_activation(&smoke(policy(30)?, verifiers.clone()), vec![candidate]).await;

    let SmokeStageOutcome::Passed(candidates) = outcome else {
        return Err("good fixture tools did not pass".into());
    };
    {
        let requests = verifiers.requests();
        let [(ffmpeg, ffprobe, workspace)] = requests.media.as_slice() else {
            return Err("the media fixture ran other than once".into());
        };
        let runtime = stage_path.join("runtime.pending");
        assert!(is_within(ffmpeg, &runtime)?);
        assert!(is_within(ffprobe, &runtime)?);
        assert!(workspace.starts_with(&stage_path));
        assert!(workspace.ends_with("smoke.pending"));
        assert!(requests.speech.is_empty());
    }
    assert!(!stage_path.join("smoke.pending").exists());
    root.assert_nothing_activated()?;
    for candidate in candidates {
        assert_eq!(candidate.discard(), StageDisposal::Discarded);
    }
    root.assert_only_marker_remains()?;
    Ok(())
}

/// The library the loader-failure fixture names.
fn libgomp() -> TestResult<SharedLibraryName> {
    SharedLibraryName::parse("libgomp.so.1").ok_or_else(|| "libgomp.so.1 is a library name".into())
}

/// Each banner failure is typed; the stage is removed and nothing activated.
#[tokio::test]
async fn banner_failures_are_typed_and_the_stage_is_discarded() -> TestResult {
    // Only the hang uses a short deadline: a freshly written executable's
    // first start can take seconds while a host scanner inspects it.
    for (behaviour, deadline, reason) in [
        ("badbanner", 30, Reason::BannerMismatch),
        ("flood", 30, Reason::OutputOverBound),
        ("hang", 2, Reason::DeadlineExceeded),
        ("fail", 30, Reason::ProviderFailed),
        // The loader's missing-library words name the library (#256); the
        // same words around a hostile "name" name nothing.
        ("missinglib", 30, Reason::MissingSharedLibrary(libgomp()?)),
        ("hostilelib", 30, Reason::ProviderFailed),
    ] {
        let root = TestRoot::new()?;
        let store = root.store()?;
        let candidate = stage_media(&store, behaviour, "good")?;
        let verifiers = FixedVerifiers::passing();
        let (failure, stages) = failed(
            smoke_before_activation(
                &smoke(policy(deadline)?, verifiers.clone()),
                vec![candidate],
            )
            .await,
        )?;
        assert_eq!(failure.check, Check::Banner, "{behaviour}");
        assert_eq!(failure.reason, reason, "{behaviour}");
        assert_eq!(
            stages,
            [(ManagedComponent::MediaTools, StageDisposal::Discarded)]
        );
        assert!(verifiers.requests().media.is_empty());
        root.assert_nothing_activated()?;
        root.assert_only_marker_remains()?;
    }
    Ok(())
}

/// The banner of the second tool is checked as well.
#[tokio::test]
async fn a_wrong_ffprobe_banner_fails_too() -> TestResult {
    let root = TestRoot::new()?;
    let store = root.store()?;
    let candidate = stage_media(&store, "good", "badbanner")?;
    let (failure, _) = failed(
        smoke_before_activation(
            &smoke(policy(30)?, FixedVerifiers::passing()),
            vec![candidate],
        )
        .await,
    )?;
    assert_eq!(
        failure,
        CompatibilitySmokeFailure {
            check: Check::Banner,
            reason: Reason::BannerMismatch,
        }
    );
    root.assert_nothing_activated()?;
    root.assert_only_marker_remains()?;
    Ok(())
}

/// The existing F01 media verifier runs against the staged tools: a fake
/// that prints only the right banner cannot pass it.
#[tokio::test]
async fn the_reviewed_media_verifier_rejects_a_banner_only_fake() -> TestResult {
    let root = TestRoot::new()?;
    let store = root.store()?;
    let candidate = stage_media(&store, "good", "good")?;
    let (failure, stages) = failed(
        smoke_before_activation(
            &smoke(policy(30)?, ReviewedFixtureVerifiers),
            vec![candidate],
        )
        .await,
    )?;
    assert_eq!(failure.check, Check::MediaFixture);
    assert!(
        matches!(
            failure.reason,
            Reason::FixtureMismatch | Reason::ProviderFailed
        ),
        "{failure:?}"
    );
    assert_eq!(
        stages,
        [(ManagedComponent::MediaTools, StageDisposal::Discarded)]
    );
    root.assert_nothing_activated()?;
    root.assert_only_marker_remains()?;
    Ok(())
}

/// A role naming a file the runtime lacks is a missing executable.
#[tokio::test]
async fn a_missing_executable_fails_the_layout() -> TestResult {
    let root = TestRoot::new()?;
    let store = root.store()?;
    let ffmpeg = tool("ffmpeg-good");
    let ffprobe = tool("ffprobe-good");
    let candidate = stage(
        &store,
        ManagedComponent::MediaTools,
        &[executable("ffmpeg-good")?],
        &[
            (ManagedRuntimeRole::Ffmpeg, &ffmpeg),
            (ManagedRuntimeRole::Ffprobe, &ffprobe),
        ],
    )?;
    let (failure, stages) = failed(
        smoke_before_activation(
            &smoke(policy(30)?, FixedVerifiers::passing()),
            vec![candidate],
        )
        .await,
    )?;
    assert_eq!(
        failure,
        CompatibilitySmokeFailure {
            check: Check::Layout,
            reason: Reason::MissingExecutable,
        }
    );
    assert_eq!(
        stages,
        [(ManagedComponent::MediaTools, StageDisposal::Discarded)]
    );
    root.assert_nothing_activated()?;
    root.assert_only_marker_remains()?;
    Ok(())
}

/// An executable built for another machine is refused before it runs.
#[tokio::test]
async fn a_foreign_executable_is_the_wrong_architecture() -> TestResult {
    let root = TestRoot::new()?;
    let store = root.store()?;
    // A 64-bit little-endian ELF header for PowerPC 64 (machine 21), which no
    // supported host runs natively.
    let mut foreign = vec![0_u8; 128];
    foreign[..4].copy_from_slice(b"\x7fELF");
    foreign[4] = 2;
    foreign[5] = 1;
    foreign[18..20].copy_from_slice(&21_u16.to_le_bytes());
    let ffmpeg = tool("ffmpeg-good");
    let ffprobe = tool("ffprobe-good");
    let candidate = stage(
        &store,
        ManagedComponent::MediaTools,
        &[
            ArchiveFile {
                name: ffmpeg.clone(),
                bytes: foreign,
                executable: true,
            },
            executable("ffprobe-good")?,
        ],
        &[
            (ManagedRuntimeRole::Ffmpeg, &ffmpeg),
            (ManagedRuntimeRole::Ffprobe, &ffprobe),
        ],
    )?;
    let (failure, stages) = failed(
        smoke_before_activation(
            &smoke(policy(30)?, FixedVerifiers::passing()),
            vec![candidate],
        )
        .await,
    )?;
    assert_eq!(
        failure,
        CompatibilitySmokeFailure {
            check: Check::Layout,
            reason: Reason::WrongArchitecture,
        }
    );
    assert_eq!(
        stages,
        [(ManagedComponent::MediaTools, StageDisposal::Discarded)]
    );
    root.assert_nothing_activated()?;
    root.assert_only_marker_remains()?;
    Ok(())
}

/// A file reviewed as data, or a script, is not executable; no shell runs it.
#[tokio::test]
async fn data_and_scripts_are_not_executable() -> TestResult {
    let script = b"#!/bin/sh\necho ffmpeg version vsift-smoke-fixture\n".to_vec();
    for (bytes, reviewed_executable) in [(fixture_tool_bytes()?, false), (script, true)] {
        let root = TestRoot::new()?;
        let store = root.store()?;
        let ffmpeg = tool("ffmpeg-good");
        let ffprobe = tool("ffprobe-good");
        let candidate = stage(
            &store,
            ManagedComponent::MediaTools,
            &[
                ArchiveFile {
                    name: ffmpeg.clone(),
                    bytes,
                    executable: reviewed_executable,
                },
                executable("ffprobe-good")?,
            ],
            &[
                (ManagedRuntimeRole::Ffmpeg, &ffmpeg),
                (ManagedRuntimeRole::Ffprobe, &ffprobe),
            ],
        )?;
        let (failure, stages) = failed(
            smoke_before_activation(
                &smoke(policy(30)?, FixedVerifiers::passing()),
                vec![candidate],
            )
            .await,
        )?;
        assert_eq!(
            failure,
            CompatibilitySmokeFailure {
                check: Check::Layout,
                reason: Reason::NotExecutable,
            }
        );
        assert_eq!(
            stages,
            [(ManagedComponent::MediaTools, StageDisposal::Discarded)]
        );
        root.assert_nothing_activated()?;
        root.assert_only_marker_remains()?;
    }
    Ok(())
}

/// An unexpected file in the staged runtime fails the layout, and cleanup
/// keeps the stage and the file rather than deleting what it did not create.
#[tokio::test]
async fn an_extra_runtime_file_fails_and_the_stage_is_retained() -> TestResult {
    let root = TestRoot::new()?;
    let store = root.store()?;
    let candidate = stage_media(&store, "good", "good")?;
    let stage_path = root.only_stage()?;
    let extra = stage_path.join("runtime.pending").join("unexpected");
    fs::write(&extra, b"not reviewed")?;
    let (failure, stages) = failed(
        smoke_before_activation(
            &smoke(policy(30)?, FixedVerifiers::passing()),
            vec![candidate],
        )
        .await,
    )?;
    assert_eq!(
        failure,
        CompatibilitySmokeFailure {
            check: Check::Layout,
            reason: Reason::UnexpectedExtraFile,
        }
    );
    assert_eq!(
        stages,
        [(
            ManagedComponent::MediaTools,
            StageDisposal::Retained(StageRetentionReason::UnexpectedContent)
        )]
    );
    assert_eq!(fs::read(&extra)?, b"not reviewed");
    assert!(
        stage_path
            .join("runtime.pending")
            .join(tool("ffmpeg-good"))
            .is_file()
    );
    root.assert_nothing_activated()?;
    Ok(())
}

/// A provider that writes into its own installation fails the recheck; the
/// file it wrote is kept and reported, never deleted.
#[tokio::test]
async fn a_provider_writing_into_its_install_fails_the_recheck() -> TestResult {
    let root = TestRoot::new()?;
    let store = root.store()?;
    let candidate = stage_media(&store, "scribble", "good")?;
    let stage_path = root.only_stage()?;
    let (failure, stages) = failed(
        smoke_before_activation(
            &smoke(policy(30)?, FixedVerifiers::passing()),
            vec![candidate],
        )
        .await,
    )?;
    assert_eq!(
        failure,
        CompatibilitySmokeFailure {
            check: Check::Recheck,
            reason: Reason::UnexpectedExtraFile,
        }
    );
    assert_eq!(
        stages,
        [(
            ManagedComponent::MediaTools,
            StageDisposal::Retained(StageRetentionReason::UnexpectedContent)
        )]
    );
    assert!(
        stage_path
            .join("runtime.pending")
            .join(SCRIBBLE_FILE)
            .is_file()
    );
    root.assert_nothing_activated()?;
    Ok(())
}

/// Anything left in the smoke directory fails the recheck and is kept.
#[tokio::test]
async fn a_leftover_in_the_smoke_directory_is_kept_and_reported() -> TestResult {
    let root = TestRoot::new()?;
    let store = root.store()?;
    let candidate = stage_media(&store, "good", "good")?;
    let stage_path = root.only_stage()?;
    let verifiers = FixedVerifiers {
        litter: true,
        ..FixedVerifiers::passing()
    };
    let (failure, stages) =
        failed(smoke_before_activation(&smoke(policy(30)?, verifiers), vec![candidate]).await)?;
    assert_eq!(
        failure,
        CompatibilitySmokeFailure {
            check: Check::Recheck,
            reason: Reason::UnexpectedExtraFile,
        }
    );
    assert_eq!(
        stages,
        [(
            ManagedComponent::MediaTools,
            StageDisposal::Retained(StageRetentionReason::UnexpectedContent)
        )]
    );
    assert!(
        stage_path
            .join("smoke.pending")
            .join("left-behind")
            .is_file()
    );
    root.assert_nothing_activated()?;
    Ok(())
}

/// A stage whose ownership marker was replaced is not smoked and not
/// cleaned: every file in it is left exactly where it was.
#[tokio::test]
async fn an_unprovable_stage_is_left_untouched_and_reported() -> TestResult {
    let root = TestRoot::new()?;
    let store = root.store()?;
    let candidate = stage_media(&store, "good", "good")?;
    let stage_path = root.only_stage()?;
    let marker = stage_path.join("stage-v1");
    let length = fs::metadata(&marker)?.len();
    fs::write(&marker, vec![b'x'; usize::try_from(length)?])?;
    let before = tree(&stage_path)?;
    let (failure, stages) = failed(
        smoke_before_activation(
            &smoke(policy(30)?, FixedVerifiers::passing()),
            vec![candidate],
        )
        .await,
    )?;
    assert_eq!(
        failure,
        CompatibilitySmokeFailure {
            check: Check::Layout,
            reason: Reason::ChangedContent,
        }
    );
    assert_eq!(
        stages,
        [(
            ManagedComponent::MediaTools,
            StageDisposal::Retained(StageRetentionReason::OwnershipUnproved)
        )]
    );
    assert_eq!(tree(&stage_path)?, before);
    root.assert_nothing_activated()?;
    Ok(())
}

/// A staged file whose bytes changed fails the layout; the reviewed names
/// in the owned stage are still removed.
#[tokio::test]
async fn changed_bytes_fail_the_layout_and_the_stage_is_discarded() -> TestResult {
    let root = TestRoot::new()?;
    let store = root.store()?;
    let candidate = stage_media(&store, "good", "good")?;
    let stage_path = root.only_stage()?;
    let target = stage_path
        .join("runtime.pending")
        .join(tool("ffprobe-good"));
    let mut bytes = fs::read(&target)?;
    if let Some(last) = bytes.last_mut() {
        *last ^= 0xff;
    }
    fs::write(&target, bytes)?;
    let (failure, stages) = failed(
        smoke_before_activation(
            &smoke(policy(30)?, FixedVerifiers::passing()),
            vec![candidate],
        )
        .await,
    )?;
    assert_eq!(
        failure,
        CompatibilitySmokeFailure {
            check: Check::Layout,
            reason: Reason::ChangedContent,
        }
    );
    assert_eq!(
        stages,
        [(ManagedComponent::MediaTools, StageDisposal::Discarded)]
    );
    root.assert_nothing_activated()?;
    root.assert_only_marker_remains()?;
    Ok(())
}

/// A staged model is smoked with an already selected recognizer and media
/// tools; the verifier sees the staged model by its path inside the stage.
#[tokio::test]
async fn a_staged_model_is_smoked_with_selected_companions() -> TestResult {
    let root = TestRoot::new()?;
    let store = root.store()?;
    let candidate = stage_model(&store)?;
    let stage_path = root.only_stage()?;
    let companions = SmokeCompanions {
        media: Some(companion_media()?),
        whisper: Some(TrustedExecutable::explicit(FIXTURE_TOOL)?),
        model: None,
    };
    let verifiers = FixedVerifiers::passing();
    let outcome = smoke_before_activation(
        &smoke_with(policy(30)?, companions, verifiers.clone()),
        vec![candidate],
    )
    .await;
    let SmokeStageOutcome::Passed(candidates) = outcome else {
        return Err("the staged model did not pass".into());
    };
    {
        let requests = verifiers.requests();
        let [(whisper, model, workspace)] = requests.speech.as_slice() else {
            return Err("the speech fixture ran other than once".into());
        };
        assert!(is_within(model, &stage_path.join("runtime.pending"))?);
        assert_eq!(fs::canonicalize(whisper)?, fs::canonicalize(FIXTURE_TOOL)?);
        assert!(workspace.starts_with(&stage_path));
        assert!(requests.media.is_empty());
    }
    root.assert_nothing_activated()?;
    for candidate in candidates {
        assert_eq!(candidate.discard(), StageDisposal::Discarded);
    }
    root.assert_only_marker_remains()?;

    let root = TestRoot::new()?;
    let store = root.store()?;
    let candidate = stage_model(&store)?;
    let (failure, stages) = failed(
        smoke_before_activation(
            &smoke(policy(30)?, FixedVerifiers::passing()),
            vec![candidate],
        )
        .await,
    )?;
    assert_eq!(
        failure,
        CompatibilitySmokeFailure {
            check: Check::Layout,
            reason: Reason::MissingExecutable,
        }
    );
    assert_eq!(
        stages,
        [(ManagedComponent::WhisperModel, StageDisposal::Discarded)]
    );
    root.assert_only_marker_remains()?;
    Ok(())
}

/// A staged recognizer and model: the recognizer's startup check and the
/// speech fixture both gate them, and a failure discards both stages.
#[tokio::test]
async fn a_staged_recognizer_and_model_fail_together() -> TestResult {
    let companions = || -> TestResult<SmokeCompanions> {
        Ok(SmokeCompanions {
            media: Some(companion_media()?),
            ..SmokeCompanions::default()
        })
    };
    let transcript = FixedVerifiers {
        speech: LocalAsrVerification::Failed(LocalAsrVerificationFailure::UnexpectedTranscript),
        ..FixedVerifiers::passing()
    };
    for (behaviour, deadline, verifiers, expected) in [
        (
            "good",
            30,
            transcript,
            CompatibilitySmokeFailure {
                check: Check::SpeechFixture,
                reason: Reason::FixtureMismatch,
            },
        ),
        (
            "hang",
            2,
            FixedVerifiers::passing(),
            CompatibilitySmokeFailure {
                check: Check::Banner,
                reason: Reason::DeadlineExceeded,
            },
        ),
        // #256: the reviewed whisper.cpp build on a minimal Ubuntu image
        // cannot start without the OpenMP runtime; the loader says which
        // library, and the smoke carries that name.
        (
            "missinglib",
            30,
            FixedVerifiers::passing(),
            CompatibilitySmokeFailure {
                check: Check::Banner,
                reason: Reason::MissingSharedLibrary(libgomp()?),
            },
        ),
    ] {
        let root = TestRoot::new()?;
        let store = root.store()?;
        let candidates = vec![stage_whisper(&store, behaviour)?, stage_model(&store)?];
        let (failure, stages) = failed(
            smoke_before_activation(
                &smoke_with(policy(deadline)?, companions()?, verifiers),
                candidates,
            )
            .await,
        )?;
        assert_eq!(failure, expected, "{behaviour}");
        assert_eq!(
            stages,
            [
                (ManagedComponent::WhisperCli, StageDisposal::Discarded),
                (ManagedComponent::WhisperModel, StageDisposal::Discarded),
            ]
        );
        root.assert_nothing_activated()?;
        root.assert_only_marker_remains()?;
    }
    Ok(())
}

/// A media-fixture failure from the verifier maps to its smoke reason.
#[tokio::test]
async fn media_fixture_failures_are_typed() -> TestResult {
    for (media_failure, reason) in [
        (MediaToolFailure::UnexpectedResult, Reason::FixtureMismatch),
        (MediaToolFailure::OutputLimit, Reason::OutputOverBound),
        (MediaToolFailure::Deadline, Reason::DeadlineExceeded),
        (MediaToolFailure::ProcessFailure, Reason::ProviderFailed),
    ] {
        let root = TestRoot::new()?;
        let store = root.store()?;
        let candidate = stage_media(&store, "good", "good")?;
        let verifiers = FixedVerifiers {
            media: MediaToolVerification::Failed {
                check: MediaToolCheck::Probe,
                failure: media_failure,
            },
            ..FixedVerifiers::passing()
        };
        let (failure, stages) =
            failed(smoke_before_activation(&smoke(policy(30)?, verifiers), vec![candidate]).await)?;
        assert_eq!(
            failure,
            CompatibilitySmokeFailure {
                check: Check::MediaFixture,
                reason,
            }
        );
        assert_eq!(
            stages,
            [(ManagedComponent::MediaTools, StageDisposal::Discarded)]
        );
        root.assert_nothing_activated()?;
        root.assert_only_marker_remains()?;
    }
    Ok(())
}

/// A cancelled smoke runs nothing and still cleans up.
#[tokio::test]
async fn a_cancelled_smoke_runs_nothing() -> TestResult {
    let root = TestRoot::new()?;
    let store = root.store()?;
    let candidate = stage_media(&store, "good", "good")?;
    let cancellation = ProcessCancellation::new();
    cancellation.cancel();
    let verifiers = FixedVerifiers::passing();
    let smoke = StagedCompatibilitySmoke::new(
        policy(30)?,
        HostIsolation::ProcessOnly,
        SmokeCompanions::default(),
        verifiers.clone(),
        cancellation,
    );
    let (failure, stages) = failed(smoke_before_activation(&smoke, vec![candidate]).await)?;
    assert_eq!(failure.reason, Reason::Cancelled);
    assert_eq!(
        stages,
        [(ManagedComponent::MediaTools, StageDisposal::Discarded)]
    );
    assert!(verifiers.requests().media.is_empty());
    root.assert_only_marker_remains()?;
    Ok(())
}

/// Every file and directory under `directory`, with each file's bytes.
fn tree(directory: &Path) -> TestResult<Vec<(PathBuf, Option<Vec<u8>>)>> {
    let mut entries = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path.clone());
                entries.push((path, None));
            } else {
                let bytes = fs::read(&path)?;
                entries.push((path, Some(bytes)));
            }
        }
    }
    entries.sort();
    Ok(entries)
}

/// A candidate that cannot be prepared never exists: its stage is removed
/// and the typed failure says so.
#[test]
fn a_candidate_that_cannot_be_prepared_is_discarded() -> TestResult {
    let root = TestRoot::new()?;
    let store = root.store()?;
    let bytes = b"fixture speech model bytes".to_vec();
    for (role_name, payload_name) in [("../outside", "model.bin"), ("model.bin", "../model.bin")] {
        let staged = store.import_verified(&bytes[..], integrity_of(&bytes)?)?;
        let Err(error) = StagedManagedCandidate::prepare_raw_file(
            staged,
            ManagedComponent::WhisperModel,
            &[(ManagedRuntimeRole::SpeechModel, role_name)],
            payload_name,
        ) else {
            return Err("a path-like name was accepted".into());
        };
        if role_name == "model.bin" {
            assert!(matches!(error.failure, ManagedCandidateFailure::Payload(_)));
        } else {
            assert!(matches!(
                error.failure,
                ManagedCandidateFailure::InvalidReview
            ));
        }
        assert_eq!(error.stage, StageDisposal::Discarded);
        root.assert_only_marker_remains()?;
    }
    root.assert_nothing_activated()?;
    Ok(())
}
