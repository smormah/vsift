//! P13 PR 4: the managed tier of dependency lookup through the engine
//! (D-01, ADR 0007's order).
//!
//! A test managed root holds published versions of the three components,
//! each a small stand-in file under the reviewed names. `setup check` must
//! resolve a per-call path first, then a configured path, then the managed
//! version, then the filtered `PATH`, and report `managed_version` for what
//! the managed tier supplied; the model follows the same order. The stand-ins
//! never need to run: lookup, not execution, is under test.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fmt::Write as _,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use tar::{Builder, Header};
use vsift::{
    Cancellation, DEFAULT_PROBE_TIMEOUT, DiarizerSelection, Engine, EngineConfig, EngineError,
    EnginePorts, ExecutableSelections, HostIsolation, LocalAsrModelStatus, ManagedRootLocation,
    RuntimeDependency, SessionRootLocation, SetupCheckRequest, SpeakersRequest,
    UserConfigurationLocation,
};
use vsift_contract::DependencyLookup;
use vsift_domain::{ArtifactIntegrity, ManagedComponent, SessionId};
use vsift_infrastructure::{
    ArchiveInventoryBounds, ManagedArtifactStore, ManagedRuntimeIdentity, ManagedRuntimeRole,
    ReviewedArchiveFile, ReviewedPayloadArchive, ReviewedRuntimeLayout, StagedManagedCandidate,
    managed_executable_name,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-engine-managed-lookup-test-";
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

    fn engine(&self, managed: &str) -> Engine {
        Engine::new(
            EngineConfig {
                session_root: SessionRootLocation::Explicit(self.path("sessions")),
                user_configuration: UserConfigurationLocation::Explicit(self.path("config")),
                managed_root: ManagedRootLocation::Explicit(self.path(managed)),
                host_isolation: HostIsolation::ProcessOnly,
            },
            EnginePorts::system(),
        )
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

fn integrity_of(bytes: &[u8]) -> Built<ArtifactIntegrity> {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        write!(hex, "{byte:02x}")?;
    }
    Ok(ArtifactIntegrity::from_sha256_hex(
        u64::try_from(bytes.len())?,
        &hex,
    )?)
}

/// Publishes and selects one version of `component` holding `files`.
fn publish(
    store: &ManagedArtifactStore,
    component: ManagedComponent,
    files: &[(String, bool)],
    roles: &[(ManagedRuntimeRole, &str)],
) -> TestResult {
    let mut archive = Builder::new(Vec::new());
    let mut total = 0_u64;
    let mut selected = Vec::new();
    for (name, _) in files {
        let bytes = format!("stand-in {name}").into_bytes();
        let path = format!("root/{name}");
        let mut header = Header::new_gnu();
        header.set_path(&path)?;
        header.set_size(u64::try_from(bytes.len())?);
        header.set_mode(0o755);
        header.set_cksum();
        archive.append(&header, Cursor::new(&bytes))?;
        total += u64::try_from(bytes.len())?;
        selected.push((path, integrity_of(&bytes)?));
    }
    let bytes = archive.into_inner()?;
    let staged = store.import_verified(&bytes[..], integrity_of(&bytes)?)?;
    let reviewed: Vec<ReviewedArchiveFile<'_>> = selected
        .iter()
        .map(|(path, integrity)| ReviewedArchiveFile {
            path,
            integrity: *integrity,
        })
        .collect();
    let executables: Vec<&str> = files
        .iter()
        .filter(|(_, executable)| *executable)
        .map(|(name, _)| name.as_str())
        .collect();
    let candidate = StagedManagedCandidate::prepare_archive(
        staged,
        component,
        roles,
        ReviewedPayloadArchive::Tar {
            max_tar_bytes: u64::try_from(bytes.len())?,
        },
        ArchiveInventoryBounds::new(files.len(), total)?,
        &[],
        &reviewed,
        ReviewedRuntimeLayout {
            max_bytes: total,
            aliases: &[],
            executables: &executables,
        },
    )?;
    let guard = store.try_install_guard()?;
    let identity = ManagedRuntimeIdentity::new(component.identifier(), "fixture-1")?;
    let (published, _) = candidate.publish_and_select(&guard, &identity);
    published?;
    Ok(())
}

fn name(role: ManagedRuntimeRole) -> Built<String> {
    managed_executable_name(role).ok_or_else(|| "no executable name".into())
}

/// Publishes all three components into `root`.
fn install_all(root: &Path) -> TestResult {
    let store = ManagedArtifactStore::at(root.to_path_buf())?;
    let ffmpeg = name(ManagedRuntimeRole::Ffmpeg)?;
    let ffprobe = name(ManagedRuntimeRole::Ffprobe)?;
    let whisper = name(ManagedRuntimeRole::WhisperCli)?;
    publish(
        &store,
        ManagedComponent::MediaTools,
        &[(ffmpeg.clone(), true), (ffprobe.clone(), true)],
        &[
            (ManagedRuntimeRole::Ffmpeg, &ffmpeg),
            (ManagedRuntimeRole::Ffprobe, &ffprobe),
        ],
    )?;
    publish(
        &store,
        ManagedComponent::WhisperCli,
        &[(whisper.clone(), true)],
        &[(ManagedRuntimeRole::WhisperCli, &whisper)],
    )?;
    publish(
        &store,
        ManagedComponent::WhisperModel,
        &[(String::from("model.bin"), false)],
        &[(ManagedRuntimeRole::SpeechModel, "model.bin")],
    )
}

fn check(per_call: ExecutableSelections) -> SetupCheckRequest {
    SetupCheckRequest {
        probe_timeout: DEFAULT_PROBE_TIMEOUT,
        per_call,
        local_asr_budget: Duration::from_secs(5),
    }
}

/// D-01: per call, then configured, then managed, then the filtered `PATH`;
/// the model: configured, then managed.
#[tokio::test]
async fn setup_check_resolves_the_managed_tier_between_configured_and_path() -> TestResult {
    let root = OwnedRoot::new()?;
    install_all(&root.path("managed"))?;
    let engine = root.engine("managed");

    let report = engine
        .check_setup(check(ExecutableSelections::default()))
        .await?;
    for dependency in RuntimeDependency::ALL {
        assert_eq!(
            report.lookup(dependency),
            DependencyLookup::ManagedVersion,
            "{dependency:?}"
        );
    }
    // The managed model is identified (it is no reviewed pin, so it will not
    // run), where without the managed tier nothing is registered.
    assert_eq!(report.local_asr().model, LocalAsrModelStatus::Unrecognised);

    let configured = root.path("configured-ffprobe");
    fs::write(&configured, b"stand-in configured tool")?;
    engine.configure_executable(RuntimeDependency::Ffprobe, &configured)?;
    let configured_model = root.path("configured-model.bin");
    fs::write(&configured_model, b"stand-in configured model")?;
    engine.configure_model(&configured_model)?;
    let per_call = root.path("per-call-whisper");
    fs::write(&per_call, b"stand-in per-call tool")?;
    let report = engine
        .check_setup(check(ExecutableSelections {
            whisper: Some(per_call),
            ..ExecutableSelections::default()
        }))
        .await?;
    assert_eq!(
        report.lookup(RuntimeDependency::Ffmpeg),
        DependencyLookup::ManagedVersion
    );
    assert_eq!(
        report.lookup(RuntimeDependency::Ffprobe),
        DependencyLookup::ConfiguredUserPath
    );
    assert_eq!(
        report.lookup(RuntimeDependency::Whisper),
        DependencyLookup::ExplicitPath
    );

    // With no managed version, lookup falls through to the filtered PATH.
    let empty = root.engine("no-managed-root");
    let report = empty
        .check_setup(check(ExecutableSelections::default()))
        .await?;
    assert_eq!(
        report.lookup(RuntimeDependency::Ffmpeg),
        DependencyLookup::FilteredPath
    );
    assert!(!root.path("no-managed-root").exists());
    Ok(())
}

/// A managed version whose bytes no longer match its manifest is never
/// used: lookup falls through to `PATH` (L-006 for managed tools).
#[tokio::test]
async fn a_changed_managed_version_is_not_used() -> TestResult {
    let root = OwnedRoot::new()?;
    install_all(&root.path("managed"))?;
    let ffmpeg = name(ManagedRuntimeRole::Ffmpeg)?;
    let version = root
        .path("managed")
        .join("versions-v1")
        .join("ffmpeg_ffprobe--fixture-1")
        .join(&ffmpeg);
    let original = fs::read(&version)?;
    let mut altered = original.clone();
    if let Some(byte) = altered.first_mut() {
        *byte ^= 0xff;
    }
    fs::write(&version, &altered)?;
    let report = root
        .engine("managed")
        .check_setup(check(ExecutableSelections::default()))
        .await?;
    assert_eq!(
        report.lookup(RuntimeDependency::Ffmpeg),
        DependencyLookup::FilteredPath
    );
    assert_eq!(
        report.lookup(RuntimeDependency::Whisper),
        DependencyLookup::ManagedVersion
    );
    Ok(())
}

/// Publishes the speaker group (the tool and the two models, as stand-ins)
/// into `root`; `pieces` says which of the three are published.
fn install_speakers(root: &Path, pieces: [bool; 3]) -> TestResult {
    let store = ManagedArtifactStore::at(root.to_path_buf())?;
    let tool = name(ManagedRuntimeRole::SpeakerDiarizer)?;
    if pieces[0] {
        publish(
            &store,
            ManagedComponent::SpeakerDiarizer,
            &[(tool.clone(), true)],
            &[(ManagedRuntimeRole::SpeakerDiarizer, &tool)],
        )?;
    }
    if pieces[1] {
        publish(
            &store,
            ManagedComponent::SpeakerSegmentationModel,
            &[(String::from("segmentation.onnx"), false)],
            &[(
                ManagedRuntimeRole::SpeakerSegmentationModel,
                "segmentation.onnx",
            )],
        )?;
    }
    if pieces[2] {
        publish(
            &store,
            ManagedComponent::SpeakerEmbeddingModel,
            &[(String::from("embedding.onnx"), false)],
            &[(ManagedRuntimeRole::SpeakerEmbeddingModel, "embedding.onnx")],
        )?;
    }
    Ok(())
}

/// The refusal `label_speakers` gives when it cannot select the diarizer, or
/// the error of the stage after the selection (the media tools, then the
/// session): which one is what these tests tell apart. The stand-ins never
/// run; selection, not execution, is under test.
async fn label_with(
    engine: &Engine,
    selection: DiarizerSelection,
) -> Result<EngineError, Box<dyn Error>> {
    let request = SpeakersRequest {
        session: SessionId::parse("ses_0123456789abcdef0123456789abcdef")?,
        revision: None,
        range: None,
        speakers: None,
        diarizer: selection,
        cancellation: Cancellation::new(),
    };
    match engine.label_speakers(request).await {
        Err(error) => Ok(error),
        Ok(_) => Err("labelling an absent session cannot succeed".into()),
    }
}

/// W5c (ADR 0032): `transcript speakers` takes each piece of the diarizer from
/// its flag when given and otherwise from the managed install, which must be
/// the whole group and must verify; the tool alone then falls back to the
/// filtered `PATH`, the models never.
#[tokio::test]
async fn the_diarizer_comes_from_flags_then_the_managed_group_and_never_a_partial_one() -> TestResult
{
    let selected = |error: &EngineError| !matches!(error, EngineError::DiarizerUnavailable);

    // Nothing installed, nothing given: unavailable.
    let root = OwnedRoot::new()?;
    let error = label_with(&root.engine("managed"), DiarizerSelection::default()).await?;
    assert!(!selected(&error), "{error:?}");

    // The whole managed group is found with no flag: selection succeeds and
    // the next stage's error is returned instead.
    install_speakers(&root.path("managed"), [true, true, true])?;
    let error = label_with(&root.engine("managed"), DiarizerSelection::default()).await?;
    assert!(selected(&error), "{error:?}");

    // A partial group is not used: the tool without its two models, a model
    // missing, or the tool missing.
    for pieces in [
        [true, false, false],
        [true, true, false],
        [false, true, true],
    ] {
        let partial = OwnedRoot::new()?;
        install_speakers(&partial.path("managed"), pieces)?;
        let error = label_with(&partial.engine("managed"), DiarizerSelection::default()).await?;
        assert!(!selected(&error), "{pieces:?}: {error:?}");
    }

    // A caller's own models with the managed tool: no managed model is
    // needed, and a model of the caller's alone cannot make a selection.
    let own = OwnedRoot::new()?;
    install_speakers(&own.path("managed"), [true, false, false])?;
    let segmentation = own.path("my-segmentation.onnx");
    let embedding = own.path("my-embedding.onnx");
    fs::write(&segmentation, b"my segmentation model")?;
    fs::write(&embedding, b"my embedding model")?;
    let both = DiarizerSelection {
        executable: None,
        segmentation_model: Some(segmentation.clone()),
        embedding_model: Some(embedding),
    };
    let error = label_with(&own.engine("managed"), both).await?;
    assert!(selected(&error), "{error:?}");
    let one = DiarizerSelection {
        executable: None,
        segmentation_model: Some(segmentation),
        embedding_model: None,
    };
    let error = label_with(&own.engine("managed"), one).await?;
    assert!(!selected(&error), "{error:?}");

    // A changed managed version is never used (L-006 for managed tools).
    let changed = OwnedRoot::new()?;
    install_speakers(&changed.path("managed"), [true, true, true])?;
    let model = changed
        .path("managed")
        .join("versions-v1")
        .join("speaker_segmentation_model--fixture-1")
        .join("segmentation.onnx");
    let mut altered = fs::read(&model)?;
    if let Some(byte) = altered.first_mut() {
        *byte ^= 0xff;
    }
    fs::write(&model, &altered)?;
    let error = label_with(&changed.engine("managed"), DiarizerSelection::default()).await?;
    assert!(!selected(&error), "{error:?}");
    Ok(())
}
