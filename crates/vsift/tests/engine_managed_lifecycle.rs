//! P13 PR 6: `setup list`, `rollback`, `remove` and `repair` through the
//! engine, on every platform.
//!
//! A test managed root holds stand-in versions published through the store,
//! as `setup install` publishes them; nothing here runs them. The engine
//! must never create the root for these commands, must answer `BUSY`
//! without waiting while another command holds the root's install guard,
//! must keep a version a job holds, and `repair` must change nothing.
//! `setup install` must refuse a plan it does not accept before it creates
//! the root (the P06 journey on 0.3.0-rc.1).

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
    Cancellation, CorePartSet, DEFAULT_PROBE_TIMEOUT, Engine, EngineConfig, EngineError,
    EnginePorts, FailureCode, HostIsolation, ManagedLifecycleRefusal, ManagedPlanAvailability,
    ManagedRemovalTarget, ManagedRootLocation, OptionalComponentSet, PlanAcceptanceError,
    ProgressObserver, RepairFindingKind, RepairFix, RepairStatus, SelectionStatus,
    SessionRootLocation, SetupInstallRequest, SetupPlanRequest, SetupProfile,
    UserConfigurationLocation, VersionRemovalStatus,
};
use vsift_contract::{OperationResponse, SavedSetupPlan};
use vsift_domain::{
    ArtifactIntegrity, ManagedComponent, ManagedVersionKey, ManagedVersionKeyError,
};
use vsift_infrastructure::{
    ArchiveInventoryBounds, ManagedArtifactStore, ManagedRuntimeIdentity, ReviewedArchiveFile,
    ReviewedPayloadArchive, ReviewedRuntimeLayout, StagedManagedCandidate,
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
    let expected = if vsift_infrastructure::accepted_host_catalogue()?.is_some() {
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
    assert_eq!(removal.versions, [] as [vsift::VersionRemovalReport; 0]);
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

/// The current plan, saved as `setup plan --json` writes it, with its digest.
async fn saved_plan(engine: &Engine) -> Built<(serde_json::Value, Option<String>)> {
    let plan = engine
        .plan_setup(SetupPlanRequest {
            profile: SetupProfile::Desktop,
            include: OptionalComponentSet::EMPTY,
            parts: CorePartSet::ALL,
            probe_timeout: DEFAULT_PROBE_TIMEOUT,
        })
        .await?;
    let envelope = OperationResponse::complete("setup.plan", plan.presentation())?;
    let saved = serde_json::to_value(envelope)?;
    let digest = saved["data"]["plan_digest"].as_str().map(str::to_owned);
    Ok((saved, digest))
}

/// What `setup install` answers for `saved` and `digest`.
async fn install(
    engine: &Engine,
    saved: &serde_json::Value,
    digest: &str,
) -> Built<Result<vsift::SetupInstallOutcome, EngineError>> {
    let saved_plan: SavedSetupPlan = serde_json::from_value(saved.clone())?;
    Ok(engine
        .install_setup(
            SetupInstallRequest {
                saved_plan,
                accepted_digest: digest.to_owned(),
                parts: CorePartSet::ALL,
                artifact_directory: None,
                probe_timeout: DEFAULT_PROBE_TIMEOUT,
            },
            &Cancellation::new(),
            ProgressObserver::none(),
        )
        .await)
}

/// The P06 journey on 0.3.0-rc.1: a refused acceptance must change nothing.
/// A wrong digest, then a saved plan that no longer matches, each return the
/// typed acceptance error, and the managed root, its marker and its lock
/// file are never created, because the plan is checked before the install
/// guard is taken. Off a qualified host there is nothing to accept at all.
#[tokio::test]
async fn a_refused_acceptance_creates_no_managed_root() -> TestResult {
    let root = OwnedRoot::new()?;
    let engine = root.engine();
    let qualified = engine.list_managed()?.availability() == ManagedPlanAvailability::Qualified;
    let (saved, digest) = saved_plan(&engine).await?;

    let wrong = install(&engine, &saved, &"0".repeat(64))
        .await?
        .err()
        .ok_or("a wrong digest was accepted")?;
    let expected = if qualified && digest.is_some() {
        PlanAcceptanceError::DigestMismatch
    } else {
        PlanAcceptanceError::ManagedUnavailable
    };
    assert!(
        matches!(wrong, EngineError::PlanAcceptance(error) if error == expected),
        "{wrong:?}"
    );
    assert_eq!(wrong.failure_code(), FailureCode::InvalidArgument);
    assert!(!root.managed().exists(), "a wrong digest created the root");

    let mut changed = saved.clone();
    changed["data"]["catalogue_revision"] = serde_json::Value::from("changed-2099-01-01-r9");
    let digest = digest.unwrap_or_else(|| "0".repeat(64));
    let stale = install(&engine, &changed, &digest)
        .await?
        .err()
        .ok_or("a changed plan was accepted")?;
    if qualified {
        assert!(
            matches!(
                stale,
                EngineError::SavedPlanRejected(FailureCode::InvalidArgument)
            ),
            "{stale:?}"
        );
    } else {
        assert!(
            matches!(
                stale,
                EngineError::PlanAcceptance(PlanAcceptanceError::ManagedUnavailable)
            ),
            "{stale:?}"
        );
    }
    assert_eq!(stale.failure_code(), FailureCode::InvalidArgument);
    assert!(!root.managed().exists(), "a changed plan created the root");
    assert!(
        !root.0.join("config").exists(),
        "a refusal created the configuration"
    );
    Ok(())
}

/// The plan is checked before the guard is taken: a stale plan is refused as
/// stale even while another installation holds the guard, not answered
/// `BUSY` for a retry that could never succeed.
#[tokio::test]
async fn a_stale_plan_is_refused_before_a_held_guard_is_reached() -> TestResult {
    let root = OwnedRoot::new()?;
    let engine = root.engine();
    if engine.list_managed()?.availability() != ManagedPlanAvailability::Qualified {
        return Ok(());
    }
    let (saved, _) = saved_plan(&engine).await?;
    let guard = root.store()?.try_install_guard()?;
    let refused = install(&engine, &saved, &"0".repeat(64)).await;
    drop(guard);
    let error = refused?.err().ok_or("a wrong digest was accepted")?;
    assert!(
        matches!(
            error,
            EngineError::PlanAcceptance(PlanAcceptanceError::DigestMismatch)
        ),
        "{error:?}"
    );
    Ok(())
}
