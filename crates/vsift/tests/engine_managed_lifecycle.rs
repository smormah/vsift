//! P13 PR 6: `setup list`, `rollback`, `remove` and `repair` through the
//! engine, on every platform.
//!
//! A test managed root holds stand-in versions published through the store,
//! as `setup install` publishes them; nothing here runs them. The engine
//! must never create the root for these commands, must answer `BUSY`
//! without waiting while another command holds the root's install guard,
//! must keep a version a job holds, and `repair` must change nothing.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fmt::Write as _,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use tar::{Builder, Header};
use vsift::{
    Engine, EngineConfig, EngineError, EnginePorts, FailureCode, HostIsolation,
    ManagedLifecycleRefusal, ManagedPlanAvailability, ManagedRemovalTarget, ManagedRootLocation,
    RepairFindingKind, RepairFix, RepairStatus, SelectionStatus, SessionRootLocation,
    UserConfigurationLocation, VersionRemovalStatus,
};
use vsift_domain::{
    ArtifactIntegrity, ManagedComponent, ManagedTarget, ManagedVersionKey, ManagedVersionKeyError,
};
use vsift_infrastructure::{
    ArchiveInventoryBounds, ManagedArtifactStore, ManagedRuntimeIdentity, ReviewedArchiveFile,
    ReviewedPayloadArchive, ReviewedRuntimeLayout, StagedManagedCandidate, detect_managed_target,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-engine-managed-lifecycle-test-";
const CLI: ManagedComponent = ManagedComponent::WhisperCli;

fn key(value: &str) -> Result<ManagedVersionKey, ManagedVersionKeyError> {
    ManagedVersionKey::parse(value)
}
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

    fn managed(&self) -> PathBuf {
        self.0.join("managed")
    }

    fn store(&self) -> Built<ManagedArtifactStore> {
        Ok(ManagedArtifactStore::at(self.managed())?)
    }

    fn engine(&self) -> Engine {
        Engine::new(
            EngineConfig {
                session_root: SessionRootLocation::Explicit(self.0.join("sessions")),
                user_configuration: UserConfigurationLocation::Explicit(self.0.join("config")),
                managed_root: ManagedRootLocation::Explicit(self.managed()),
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

/// Publishes and selects `version` of `component`: one stand-in file whose
/// bytes name the version.
fn publish(store: &ManagedArtifactStore, component: ManagedComponent, version: &str) -> TestResult {
    let contents = format!("stand-in {} {version}", component.identifier()).into_bytes();
    let mut archive = Builder::new(Vec::new());
    let mut header = Header::new_gnu();
    header.set_path("root/tool")?;
    header.set_size(u64::try_from(contents.len())?);
    header.set_mode(0o755);
    header.set_cksum();
    archive.append(&header, Cursor::new(&contents))?;
    let bytes = archive.into_inner()?;
    let staged = store.import_verified(&bytes[..], integrity_of(&bytes)?)?;
    let candidate = StagedManagedCandidate::prepare_archive(
        staged,
        component,
        &[],
        ReviewedPayloadArchive::Tar {
            max_tar_bytes: u64::try_from(bytes.len())?,
        },
        ArchiveInventoryBounds::new(1, u64::try_from(contents.len())?)?,
        &[],
        &[ReviewedArchiveFile {
            path: "root/tool",
            integrity: integrity_of(&contents)?,
        }],
        ReviewedRuntimeLayout {
            max_bytes: u64::try_from(contents.len())?,
            aliases: &[],
            executables: &["tool"],
        },
    )?;
    let guard = store.try_install_guard()?;
    let identity = ManagedRuntimeIdentity::new(component.identifier(), version)?;
    let (published, _) = candidate.publish_and_select(&guard, &identity);
    published?;
    Ok(())
}

fn version_path(root: &OwnedRoot, component: ManagedComponent, version: &str) -> PathBuf {
    root.managed()
        .join("versions-v1")
        .join(format!("{}--{version}", component.identifier()))
}

fn refusal(result: Result<impl std::fmt::Debug, EngineError>) -> Option<ManagedLifecycleRefusal> {
    match result.err()? {
        EngineError::ManagedLifecycle(refusal) => Some(refusal),
        _ => None,
    }
}

/// Every file under `path`, with its bytes, to prove nothing changed.
fn snapshot(path: &Path) -> Built<Vec<(PathBuf, Vec<u8>)>> {
    let mut files = Vec::new();
    let mut pending = vec![path.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in fs::read_dir(&folder)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else {
                files.push((entry.path(), fs::read(entry.path())?));
            }
        }
    }
    files.sort();
    Ok(files)
}

#[test]
fn with_nothing_installed_every_command_answers_and_nothing_is_created() -> TestResult {
    let root = OwnedRoot::new()?;
    let engine = root.engine();
    let listing = engine.list_managed()?;
    assert!(listing.inventory().is_none());
    let expected = if detect_managed_target() == ManagedTarget::Ubuntu2404X86_64 {
        ManagedPlanAvailability::Qualified
    } else {
        ManagedPlanAvailability::TargetUnavailable
    };
    assert_eq!(listing.availability(), expected);
    assert_eq!(
        engine.repair_managed()?.plan().status,
        RepairStatus::NothingInstalled
    );
    let rollback = engine.rollback_managed(CLI, None);
    assert_eq!(
        rollback.as_ref().err().map(EngineError::failure_code),
        Some(FailureCode::InvalidArgument)
    );
    assert_eq!(
        refusal(rollback),
        Some(ManagedLifecycleRefusal::NotInstalled)
    );
    let removal = engine.remove_managed(&ManagedRemovalTarget::Component(CLI))?;
    assert!(removal.versions.is_empty());
    assert_eq!(removal.failure_code(), None);
    let sweep = engine.remove_managed(&ManagedRemovalTarget::StaleStages)?;
    assert_eq!(sweep.stages.map(|stages| stages.removed), Some(0));
    assert!(
        !root.managed().exists(),
        "a lifecycle command created the root"
    );
    Ok(())
}

#[test]
fn rollback_list_and_remove_through_the_engine() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store()?;
    publish(&store, CLI, "1.0.0")?;
    publish(&store, CLI, "1.1.0")?;
    publish(&store, ManagedComponent::MediaTools, "7.0")?;
    let engine = root.engine();

    let listing = engine.list_managed()?;
    let inventory = listing.inventory().ok_or("no inventory")?;
    let cli = inventory.component(CLI).ok_or("no cli")?;
    assert_eq!(cli.selected_version(), Some("1.1.0"));
    assert_eq!(cli.previous_version(), Some("1.0.0"));
    assert_eq!(cli.selection_status(), SelectionStatus::Verified);

    let back = engine.rollback_managed(CLI, None)?;
    assert_eq!((back.selected.as_str(), back.changed), ("1.0.0", true));
    let again = engine.rollback_managed(CLI, Some(&key("1.0.0")?))?;
    assert!(!again.changed);
    assert!(ManagedVersionKey::parse("../escape").is_err());
    assert_eq!(
        refusal(engine.rollback_managed(CLI, Some(&key("9.9.9")?))),
        Some(ManagedLifecycleRefusal::VersionNotInstalled)
    );
    assert_eq!(
        refusal(engine.rollback_managed(ManagedComponent::WhisperModel, None)),
        Some(ManagedLifecycleRefusal::NotInstalled)
    );

    let selected = engine.remove_managed(&ManagedRemovalTarget::Version {
        component: CLI,
        version: key("1.0.0")?,
    });
    assert_eq!(
        refusal(selected),
        Some(ManagedLifecycleRefusal::VersionSelected)
    );
    let removed = engine.remove_managed(&ManagedRemovalTarget::Version {
        component: CLI,
        version: key("1.1.0")?,
    })?;
    assert_eq!(removed.versions[0].status, VersionRemovalStatus::Removed);
    assert!(!version_path(&root, CLI, "1.1.0").exists());
    // The one left has nothing to roll back to.
    assert_eq!(
        refusal(engine.rollback_managed(CLI, None)),
        Some(ManagedLifecycleRefusal::NoPreviousVersion)
    );

    let component = engine.remove_managed(&ManagedRemovalTarget::Component(CLI))?;
    assert_eq!(component.deselected, Some(CLI));
    assert_eq!(component.failure_code(), None);
    let after = engine.list_managed()?;
    let after = after.inventory().ok_or("no inventory")?;
    assert_eq!(after.component(CLI).map(|cli| cli.versions.len()), Some(0));
    assert_eq!(
        after
            .component(ManagedComponent::MediaTools)
            .and_then(|media| media.selected_version()),
        Some("7.0")
    );
    Ok(())
}

#[test]
fn a_held_guard_is_busy_for_changes_but_not_for_reading() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store()?;
    publish(&store, CLI, "1.0.0")?;
    publish(&store, CLI, "1.1.0")?;
    let engine = root.engine();
    let guard = store.try_install_guard()?;
    for result in [
        engine.rollback_managed(CLI, None).map(drop),
        engine
            .remove_managed(&ManagedRemovalTarget::Component(CLI))
            .map(drop),
        engine
            .remove_managed(&ManagedRemovalTarget::StaleStages)
            .map(drop),
    ] {
        let error = result.err().ok_or("a held guard was not busy")?;
        assert!(matches!(error, EngineError::ManagedInstallBusy));
        assert_eq!(error.failure_code(), FailureCode::Busy);
        assert_eq!(error.retry_after_ms(), Some(30_000));
    }
    assert!(engine.list_managed()?.inventory().is_some());
    assert_eq!(
        engine.repair_managed()?.plan().status,
        RepairStatus::Healthy
    );
    drop(guard);
    assert_eq!(engine.rollback_managed(CLI, None)?.selected, "1.0.0");
    Ok(())
}

#[test]
fn a_version_a_job_holds_is_kept_until_the_job_lets_go() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store()?;
    publish(&store, CLI, "1.0.0")?;
    let engine = root.engine();
    // What lookup gives a job: the selected version, verified and held.
    let held = store
        .open_selected_runtime(CLI.identifier())?
        .ok_or("nothing selected")?;
    let report = engine.remove_managed(&ManagedRemovalTarget::Component(CLI))?;
    assert_eq!(report.deselected, Some(CLI));
    assert_eq!(report.versions[0].status, VersionRemovalStatus::InUse);
    assert_eq!(report.failure_code(), Some(FailureCode::Busy));
    assert!(version_path(&root, CLI, "1.0.0").is_dir());
    held.open_reviewed_file("tool")?;
    drop(held);
    let rerun = engine.remove_managed(&ManagedRemovalTarget::Component(CLI))?;
    assert_eq!(rerun.deselected, None);
    assert_eq!(rerun.versions[0].status, VersionRemovalStatus::Removed);
    assert!(!version_path(&root, CLI, "1.0.0").exists());
    Ok(())
}

#[test]
fn repair_plans_existing_commands_and_changes_nothing() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store()?;
    publish(&store, CLI, "1.0.0")?;
    publish(&store, CLI, "1.1.0")?;
    fs::write(version_path(&root, CLI, "1.1.0").join("tool"), b"bit rot")?;
    // An abandoned download.
    let bytes = b"abandoned".to_vec();
    drop(store.import_verified(&bytes[..], integrity_of(&bytes)?)?);
    let engine = root.engine();
    let before = snapshot(&root.managed())?;
    let diagnosis = engine.repair_managed()?;
    let findings: Vec<(RepairFindingKind, RepairFix)> = diagnosis
        .plan()
        .findings
        .iter()
        .map(|finding| (finding.kind, finding.fix.clone()))
        .collect();
    assert_eq!(
        findings,
        [
            (
                RepairFindingKind::SelectedVersionUnverified,
                RepairFix::RollBack(CLI)
            ),
            (RepairFindingKind::StaleStages, RepairFix::RemoveStaleStages),
        ]
    );
    assert_eq!(
        snapshot(&root.managed())?,
        before,
        "repair changed the store"
    );
    // Applying the plan: the rollback, then the sweep.
    assert_eq!(engine.rollback_managed(CLI, None)?.selected, "1.0.0");
    let sweep = engine.remove_managed(&ManagedRemovalTarget::StaleStages)?;
    assert_eq!(sweep.stages.map(|stages| stages.removed), Some(1));
    let fixed = engine.repair_managed()?;
    assert_eq!(
        fixed
            .plan()
            .findings
            .iter()
            .map(|finding| finding.fix.clone())
            .collect::<Vec<_>>(),
        [RepairFix::RemoveVersion(CLI, String::from("1.1.0"))]
    );
    Ok(())
}
