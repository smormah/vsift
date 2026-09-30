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
    Engine, EngineConfig, EnginePorts, ExecutableSelections, HostIsolation, LocalAsrModelStatus,
    ManagedRootLocation, RuntimeDependency, SessionRootLocation, SetupCheckRequest,
    UserConfigurationLocation,
};
use vsift_contract::DependencyLookup;
use vsift_domain::{ArtifactIntegrity, ManagedComponent};
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
        probe_timeout: Duration::from_secs(5),
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
