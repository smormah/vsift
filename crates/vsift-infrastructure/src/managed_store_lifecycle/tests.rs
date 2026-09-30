//! Store-level tests of inspection, rollback, removal, cleanup and the
//! stale-stage sweep (P13 PR 6): in-use versions, a concurrent guard,
//! corrupted and interrupted versions, interrupted selections and
//! removals, and links planted in the managed root, which are never
//! followed or deleted through.
//!
//! Windows cannot create a junction without a shell or a new dependency, so
//! the Windows link cases use a directory symbolic link, which is the same
//! kind of name-surrogate reparse point; they are skipped when the account
//! may not create links.

use std::{
    error::Error,
    fmt::Write as _,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use tar::{Builder, Header};
use vsift_application::{
    ManagedLifecycleRefusal, ManagedRemovalTarget, ManagedSelection, ManagedStoreMaintenance,
    ManagedStoreReader, ManagedVersionFault, ManagedVersionState, RepairFindingKind, RepairFix,
    SelectionStatus, StageRetentionReason, VersionRemovalStatus, clean_up_versions,
    diagnose_managed_store, remove_managed, roll_back_component,
};
use vsift_domain::{
    ArtifactIntegrity, ManagedComponent, ManagedVersionKey, ManagedVersionKeyError,
};

use super::GuardedManagedStore;
use crate::{
    ArchiveInventoryBounds, ManagedArtifactError, ManagedArtifactStore, ManagedInstallGuard,
    ManagedRuntimeIdentity, PublishedManagedRuntime, ReviewedArchiveFile, ReviewedPayloadArchive,
    ReviewedRuntimeLayout,
    managed_artifact_store::{ManagedPublicationBoundary, ManagedRemovalBoundary},
};

type TestResult = Result<(), Box<dyn Error>>;

const CLI: ManagedComponent = ManagedComponent::WhisperCli;

fn key(value: &str) -> Result<ManagedVersionKey, ManagedVersionKeyError> {
    ManagedVersionKey::parse(value)
}

/// A fresh parent folder, removed on drop; the root is `managed` inside it.
struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| std::io::Error::other("random source failed"))?;
        let name = hex(&random)?;
        let parent = std::env::temp_dir().join(format!("vsift-lifecycle-test-{name}"));
        fs::create_dir(&parent)?;
        Ok(Self(parent))
    }

    fn root(&self) -> PathBuf {
        self.0.join("managed")
    }

    fn store(&self) -> Result<ManagedArtifactStore, Box<dyn Error>> {
        Ok(ManagedArtifactStore::at(self.root())?)
    }

    fn version_path(&self, version: &str) -> PathBuf {
        self.root()
            .join("versions-v1")
            .join(format!("whisper_cli--{version}"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn hex(bytes: &[u8]) -> Result<String, std::fmt::Error> {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(text, "{byte:02x}")?;
    }
    Ok(text)
}

fn integrity_of(bytes: &[u8]) -> Result<ArtifactIntegrity, Box<dyn Error>> {
    let digest = Sha256::digest(bytes);
    Ok(ArtifactIntegrity::from_sha256_hex(
        u64::try_from(bytes.len())?,
        &hex(&digest)?,
    )?)
}

/// A one-file tar whose `tool` holds `contents`.
fn tool_archive(contents: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut archive = Builder::new(Vec::new());
    let mut header = Header::new_gnu();
    header.set_path("root/tool")?;
    header.set_size(u64::try_from(contents.len())?);
    header.set_mode(0o755);
    header.set_cksum();
    archive.append(&header, Cursor::new(contents))?;
    Ok(archive.into_inner()?)
}

/// Publishes and selects `version` of the whisper.cpp CLI, whose `tool`
/// holds the version's own bytes.
fn publish(
    store: &ManagedArtifactStore,
    guard: &ManagedInstallGuard,
    version: &str,
) -> Result<PublishedManagedRuntime, Box<dyn Error>> {
    let contents = format!("tool {version}");
    let archive = tool_archive(contents.as_bytes())?;
    let artifact = store.import_verified(&archive[..], integrity_of(&archive)?)?;
    let selected = [ReviewedArchiveFile {
        path: "root/tool",
        integrity: integrity_of(contents.as_bytes())?,
    }];
    let payload = artifact.stage_reviewed_payload(
        ReviewedPayloadArchive::Tar {
            max_tar_bytes: 10_000,
        },
        ArchiveInventoryBounds::new(2, 100)?,
        &[],
        &selected,
    )?;
    let mut runtime = payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
        max_bytes: 100,
        aliases: &[],
        executables: &["tool"],
    })?;
    let published = runtime
        .publish_and_select(guard, &ManagedRuntimeIdentity::new("whisper_cli", version)?)
        .map_err(|error| std::io::Error::other(format!("publish {version}: {error:?}")))?;
    runtime.discard()?;
    payload.discard()?;
    artifact.discard()?;
    Ok(published)
}

fn selected(store: &ManagedArtifactStore) -> Result<Option<String>, Box<dyn Error>> {
    Ok(store
        .open_selected_runtime("whisper_cli")?
        .map(|runtime| runtime.identity().version().to_owned()))
}

fn cli_state(
    maintainer: &GuardedManagedStore<'_>,
    version: &str,
) -> Result<Option<ManagedVersionState>, Box<dyn Error>> {
    let inventory = maintainer.inventory()?.ok_or("no managed root")?;
    Ok(inventory
        .component(CLI)
        .and_then(|cli| cli.version(version))
        .map(|record| record.state.clone()))
}

/// Creates a private directory the way `VSift` does (owner-only on Unix; on
/// Windows the private root's inherited DACL).
fn private_dir(path: &Path) -> TestResult {
    fs::create_dir(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// A private regular file, as `VSift` writes one.
fn private_file(path: &Path, contents: &[u8]) -> TestResult {
    fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Creates a directory link, or `false` when this account may not.
fn directory_link(target: &Path, link: &Path) -> Result<bool, Box<dyn Error>> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)?;
        Ok(true)
    }
    #[cfg(windows)]
    match std::os::windows::fs::symlink_dir(target, link) {
        Ok(()) => Ok(true),
        // ERROR_PRIVILEGE_NOT_HELD: this account may not create links.
        Err(error) if error.raw_os_error() == Some(1314) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Creates a file link, or `false` when this account may not.
fn file_link(target: &Path, link: &Path) -> Result<bool, Box<dyn Error>> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)?;
        Ok(true)
    }
    #[cfg(windows)]
    match std::os::windows::fs::symlink_file(target, link) {
        Ok(()) => Ok(true),
        Err(error) if error.raw_os_error() == Some(1314) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn stage_name(fill: char) -> String {
    format!("stage-{}", fill.to_string().repeat(32))
}

#[test]
fn inspection_and_an_existing_guard_never_create_the_root() -> TestResult {
    let fixture = Fixture::new()?;
    let store = fixture.store()?;
    assert!(store.inspect()?.is_none());
    assert!(store.try_existing_install_guard()?.is_none());
    assert!(!fixture.root().exists());
    Ok(())
}

#[test]
fn the_inventory_reports_the_selection_its_previous_version_and_verification() -> TestResult {
    let fixture = Fixture::new()?;
    let store = fixture.store()?;
    let guard = store.try_install_guard()?;
    drop(publish(&store, &guard, "1.0.0")?);
    drop(publish(&store, &guard, "1.1.0")?);
    let inventory = store.inspect()?.ok_or("no root")?;
    let cli = inventory.component(CLI).ok_or("no cli")?;
    assert_eq!(cli.selected_version(), Some("1.1.0"));
    assert_eq!(cli.previous_version(), Some("1.0.0"));
    assert_eq!(cli.selection_status(), SelectionStatus::Verified);
    let versions: Vec<(&str, &str)> = cli
        .versions
        .iter()
        .map(|record| (record.version.as_str(), record.state.identifier()))
        .collect();
    assert_eq!(versions, [("1.0.0", "verified"), ("1.1.0", "verified")]);
    for other in [ManagedComponent::MediaTools, ManagedComponent::WhisperModel] {
        let component = inventory.component(other).ok_or("missing component")?;
        assert_eq!(component.selection, ManagedSelection::None);
        assert!(component.versions.is_empty());
    }
    assert_eq!(inventory.stale_stages, 0);
    assert_eq!(inventory.unexpected_entries, 0);
    Ok(())
}

#[test]
fn a_concurrent_guard_is_busy_for_lifecycle_commands_too() -> TestResult {
    let fixture = Fixture::new()?;
    let first = fixture.store()?;
    let held = first.try_install_guard()?;
    let second = fixture.store()?;
    assert!(matches!(
        second.try_existing_install_guard(),
        Err(ManagedArtifactError::Busy)
    ));
    drop(held);
    assert!(second.try_existing_install_guard()?.is_some());
    Ok(())
}

#[test]
fn rollback_swaps_the_selection_atomically_and_refuses_an_unverified_previous() -> TestResult {
    let fixture = Fixture::new()?;
    let store = fixture.store()?;
    let guard = store.try_install_guard()?;
    drop(publish(&store, &guard, "1.0.0")?);
    drop(publish(&store, &guard, "1.1.0")?);
    let maintainer = GuardedManagedStore::new(&store, &guard);

    let back = roll_back_component(&maintainer, CLI, None)?;
    assert_eq!(back.selected, "1.0.0");
    assert_eq!(back.replaced.as_deref(), Some("1.1.0"));
    assert_eq!(selected(&store)?.as_deref(), Some("1.0.0"));
    let forward = roll_back_component(&maintainer, CLI, None)?;
    assert_eq!(forward.selected, "1.1.0");
    assert_eq!(selected(&store)?.as_deref(), Some("1.1.0"));

    // The previous version's bytes change: it no longer verifies, so the
    // rollback is refused and the selection stays.
    fs::write(fixture.version_path("1.0.0").join("tool"), b"tampered")?;
    assert_eq!(
        cli_state(&maintainer, "1.0.0")?,
        Some(ManagedVersionState::Unverified(
            ManagedVersionFault::ChangedContent
        ))
    );
    assert_eq!(
        roll_back_component(&maintainer, CLI, None),
        Err(ManagedLifecycleRefusal::VersionUnverified)
    );
    assert_eq!(selected(&store)?.as_deref(), Some("1.1.0"));

    // A version without its manifest is never selected either.
    drop(publish(&store, &guard, "1.2.0")?);
    fs::remove_file(fixture.version_path("1.1.0").join("version-v1"))?;
    assert_eq!(
        cli_state(&maintainer, "1.1.0")?,
        Some(ManagedVersionState::Unverified(
            ManagedVersionFault::MissingManifest
        ))
    );
    assert_eq!(
        roll_back_component(&maintainer, CLI, None),
        Err(ManagedLifecycleRefusal::VersionUnverified)
    );
    assert_eq!(
        roll_back_component(&maintainer, CLI, Some(&key("1.1.0")?)),
        Err(ManagedLifecycleRefusal::VersionUnverified)
    );
    assert_eq!(selected(&store)?.as_deref(), Some("1.2.0"));
    Ok(())
}

#[test]
fn an_interrupted_selection_keeps_the_old_selection_and_the_sweep_clears_it() -> TestResult {
    let fixture = Fixture::new()?;
    let store = fixture.store()?;
    let guard = store.try_install_guard()?;
    drop(publish(&store, &guard, "1.0.0")?);
    drop(publish(&store, &guard, "1.1.0")?);
    let identity = ManagedRuntimeIdentity::new("whisper_cli", "1.0.0")?;
    assert!(
        store
            .select_verified_runtime_at_boundary(
                &guard,
                &identity,
                None,
                Some(ManagedPublicationBoundary::PointerPrepared),
            )
            .is_err()
    );
    assert_eq!(selected(&store)?.as_deref(), Some("1.1.0"));
    let maintainer = GuardedManagedStore::new(&store, &guard);
    let inventory = maintainer.inventory()?.ok_or("no root")?;
    assert_eq!(inventory.interrupted_selections, 1);
    let plan = diagnose_managed_store(Some(&inventory));
    assert_eq!(
        plan.findings
            .iter()
            .map(|finding| (finding.kind, finding.fix.clone()))
            .collect::<Vec<_>>(),
        [(
            RepairFindingKind::InterruptedSelection,
            RepairFix::RemoveStaleStages
        )]
    );
    let sweep = maintainer.sweep_stages()?;
    assert_eq!(sweep.interrupted_selections_removed, 1);
    assert_eq!(
        maintainer
            .inventory()?
            .ok_or("no root")?
            .interrupted_selections,
        0
    );
    // A rerun of the rollback completes.
    let rollback = roll_back_component(&maintainer, CLI, None)?;
    assert_eq!(rollback.selected, "1.0.0");
    Ok(())
}

#[test]
fn a_selection_pointer_from_before_the_previous_field_is_still_read() -> TestResult {
    let fixture = Fixture::new()?;
    let store = fixture.store()?;
    let guard = store.try_install_guard()?;
    let published = publish(&store, &guard, "1.0.0")?;
    let manifest = published.manifest_sha256().to_owned();
    drop(published);
    let pointer = fixture
        .root()
        .join("current-v1")
        .join("whisper_cli.current");
    fs::remove_file(&pointer)?;
    private_file(
        &pointer,
        format!(
            "VSIFT-MANAGED-POINTER-v1\ncomponent=whisper_cli\nversion=1.0.0\nmanifest_sha256={manifest}\n"
        )
        .as_bytes(),
    )?;
    assert_eq!(selected(&store)?.as_deref(), Some("1.0.0"));
    let inventory = store.inspect()?.ok_or("no root")?;
    let cli = inventory.component(CLI).ok_or("no cli")?;
    assert_eq!(cli.selection_status(), SelectionStatus::Verified);
    assert_eq!(cli.previous_version(), None);
    Ok(())
}

#[test]
fn an_unreadable_pointer_is_reported_and_deselecting_removes_it() -> TestResult {
    let fixture = Fixture::new()?;
    let store = fixture.store()?;
    let guard = store.try_install_guard()?;
    drop(publish(&store, &guard, "1.0.0")?);
    let pointer = fixture
        .root()
        .join("current-v1")
        .join("whisper_cli.current");
    fs::remove_file(&pointer)?;
    private_file(&pointer, b"not a pointer\n")?;
    assert!(store.open_selected_runtime("whisper_cli").is_err());
    let maintainer = GuardedManagedStore::new(&store, &guard);
    let inventory = maintainer.inventory()?.ok_or("no root")?;
    let cli = inventory.component(CLI).ok_or("no cli")?;
    assert_eq!(cli.selection_status(), SelectionStatus::Unreadable);
    let plan = diagnose_managed_store(Some(&inventory));
    assert_eq!(
        plan.findings[0].fix,
        RepairFix::RollBackTo(CLI, String::from("1.0.0"))
    );
    // The planned fix: selecting the verified version replaces the pointer.
    roll_back_component(&maintainer, CLI, Some(&key("1.0.0")?))?;
    assert_eq!(selected(&store)?.as_deref(), Some("1.0.0"));
    assert!(maintainer.deselect(CLI)?);
    assert!(!maintainer.deselect(CLI)?);
    assert_eq!(selected(&store)?, None);
    Ok(())
}

#[test]
fn a_corrupted_version_can_be_removed_but_unknown_content_is_kept() -> TestResult {
    let fixture = Fixture::new()?;
    let store = fixture.store()?;
    let guard = store.try_install_guard()?;
    for version in ["1.0.0", "1.1.0", "1.2.0"] {
        drop(publish(&store, &guard, version)?);
    }
    let maintainer = GuardedManagedStore::new(&store, &guard);
    fs::write(fixture.version_path("1.0.0").join("tool"), b"bit rot")?;
    let extra = fixture.version_path("1.1.0").join("notes.txt");
    fs::write(&extra, b"keep")?;
    assert_eq!(
        cli_state(&maintainer, "1.1.0")?,
        Some(ManagedVersionState::Unverified(
            ManagedVersionFault::UnexpectedEntry
        ))
    );
    let plan = diagnose_managed_store(maintainer.inventory()?.as_ref());
    let fixes: Vec<RepairFix> = plan
        .findings
        .iter()
        .map(|finding| finding.fix.clone())
        .collect();
    assert_eq!(
        fixes,
        [
            RepairFix::RemoveVersion(CLI, String::from("1.0.0")),
            RepairFix::Manual
        ]
    );
    let corrupted = remove_managed(
        &maintainer,
        &ManagedRemovalTarget::Version {
            component: CLI,
            version: key("1.0.0")?,
        },
    )?;
    assert_eq!(corrupted.versions[0].status, VersionRemovalStatus::Removed);
    assert!(!fixture.version_path("1.0.0").exists());
    let foreign = remove_managed(
        &maintainer,
        &ManagedRemovalTarget::Version {
            component: CLI,
            version: key("1.1.0")?,
        },
    )?;
    assert_eq!(
        foreign.versions[0].status,
        VersionRemovalStatus::UnexpectedContent
    );
    assert_eq!(fs::read(&extra)?, b"keep");
    assert_eq!(selected(&store)?.as_deref(), Some("1.2.0"));
    Ok(())
}

#[test]
fn a_version_in_use_is_never_removed_by_remove_or_cleanup() -> TestResult {
    let fixture = Fixture::new()?;
    let store = fixture.store()?;
    let guard = store.try_install_guard()?;
    for version in ["1.0.0", "1.1.0", "1.2.0", "1.3.0"] {
        drop(publish(&store, &guard, version)?);
    }
    // A job holds 1.0.0, the oldest; cleanup keeps 1.3.0 (selected) and
    // 1.2.0 (previous), removes 1.1.0 and keeps the held one.
    let held =
        store.open_published_runtime(&ManagedRuntimeIdentity::new("whisper_cli", "1.0.0")?)?;
    let maintainer = GuardedManagedStore::new(&store, &guard);
    let cleaned = clean_up_versions(&maintainer)?;
    let outcomes: Vec<(&str, VersionRemovalStatus)> = cleaned
        .iter()
        .map(|report| (report.version.as_str(), report.status))
        .collect();
    assert_eq!(
        outcomes,
        [
            ("1.0.0", VersionRemovalStatus::InUse),
            ("1.1.0", VersionRemovalStatus::Removed)
        ]
    );
    let removal = remove_managed(
        &maintainer,
        &ManagedRemovalTarget::Version {
            component: CLI,
            version: key("1.0.0")?,
        },
    )?;
    assert_eq!(removal.versions[0].status, VersionRemovalStatus::InUse);
    held.open_reviewed_file("tool")?;
    drop(held);
    assert_eq!(
        clean_up_versions(&maintainer)?
            .iter()
            .map(|report| report.status)
            .collect::<Vec<_>>(),
        [VersionRemovalStatus::Removed]
    );
    let remaining: Vec<String> = maintainer
        .inventory()?
        .ok_or("no root")?
        .component(CLI)
        .ok_or("no cli")?
        .versions
        .iter()
        .map(|record| record.version.clone())
        .collect();
    assert_eq!(remaining, ["1.2.0", "1.3.0"]);
    Ok(())
}

#[test]
fn removing_a_component_deselects_first_then_removes_every_version() -> TestResult {
    let fixture = Fixture::new()?;
    let store = fixture.store()?;
    let guard = store.try_install_guard()?;
    drop(publish(&store, &guard, "1.0.0")?);
    drop(publish(&store, &guard, "1.1.0")?);
    let maintainer = GuardedManagedStore::new(&store, &guard);
    let report = remove_managed(&maintainer, &ManagedRemovalTarget::Component(CLI))?;
    assert_eq!(report.deselected, Some(CLI));
    assert!(
        report
            .versions
            .iter()
            .all(|version| version.status == VersionRemovalStatus::Removed)
    );
    assert_eq!(report.failure_code(), None);
    assert_eq!(selected(&store)?, None);
    let cli = maintainer.inventory()?.ok_or("no root")?;
    assert!(cli.component(CLI).ok_or("no cli")?.versions.is_empty());
    // Again: nothing to do, nothing changes.
    let again = remove_managed(&maintainer, &ManagedRemovalTarget::Component(CLI))?;
    assert_eq!(again.deselected, None);
    assert!(again.versions.is_empty());
    Ok(())
}

#[test]
fn an_interrupted_removal_is_reported_and_finished_by_the_next() -> TestResult {
    let fixture = Fixture::new()?;
    let store = fixture.store()?;
    let guard = store.try_install_guard()?;
    drop(publish(&store, &guard, "1.0.0")?);
    drop(publish(&store, &guard, "1.1.0")?);
    let identity = ManagedRuntimeIdentity::new("whisper_cli", "1.0.0")?;
    for boundary in [
        ManagedRemovalBoundary::PayloadFiles,
        ManagedRemovalBoundary::UseLock,
        ManagedRemovalBoundary::VersionManifest,
    ] {
        assert!(
            store
                .remove_published_runtime_at_boundary(&guard, &identity, Some(boundary))
                .is_err()
        );
        let maintainer = GuardedManagedStore::new(&store, &guard);
        assert_eq!(
            cli_state(&maintainer, "1.0.0")?,
            Some(ManagedVersionState::RemovalInterrupted)
        );
        let plan = diagnose_managed_store(maintainer.inventory()?.as_ref());
        assert_eq!(
            plan.findings[0].fix,
            RepairFix::RemoveVersion(CLI, String::from("1.0.0"))
        );
        assert!(store.open_published_runtime(&identity).is_err());
    }
    let maintainer = GuardedManagedStore::new(&store, &guard);
    assert_eq!(
        maintainer.remove_version(CLI, "1.0.0"),
        VersionRemovalStatus::Removed
    );
    assert_eq!(cli_state(&maintainer, "1.0.0")?, None);
    Ok(())
}

/// Builds every kind of stage an interrupted or failed installation can
/// leave, and every kind of foreign content, then sweeps.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one fixture of every stage kind, swept once, reads best as one test"
)]
fn the_sweep_removes_only_positively_identified_stages() -> TestResult {
    let fixture = Fixture::new()?;
    let store = fixture.store()?;
    let guard = store.try_install_guard()?;
    let root = fixture.root();

    // An artifact whose run was killed after the download.
    let archive = tool_archive(b"tool")?;
    let killed_download = store.import_verified(&archive[..], integrity_of(&archive)?)?;
    // A candidate killed during the smoke: artifact, payload, runtime and
    // a smoke folder with a provider's output in it.
    let killed_smoke = store.import_verified(&archive[..], integrity_of(&archive)?)?;
    let payload = killed_smoke.stage_reviewed_payload(
        ReviewedPayloadArchive::Tar {
            max_tar_bytes: 10_000,
        },
        ArchiveInventoryBounds::new(2, 100)?,
        &[],
        &[ReviewedArchiveFile {
            path: "root/tool",
            integrity: integrity_of(b"tool")?,
        }],
    )?;
    let runtime = payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
        max_bytes: 100,
        aliases: &[],
        executables: &["tool"],
    })?;
    drop(runtime);
    drop(payload);
    drop(killed_smoke);
    drop(killed_download);
    let mut stages: Vec<PathBuf> = fs::read_dir(&root)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("stage-"))
        })
        .collect();
    stages.sort();
    let smoke_stage = stages
        .iter()
        .find(|stage| stage.join("runtime.pending").exists())
        .ok_or("no prepared stage")?;
    private_dir(&smoke_stage.join("smoke.pending"))?;
    private_file(&smoke_stage.join("smoke.pending").join("out.txt"), b"x")?;

    // A creation killed before its marker, and one killed mid-marker.
    private_dir(&root.join(stage_name('a')))?;
    private_dir(&root.join(stage_name('b')))?;
    private_file(&root.join(stage_name('b')).join("stage-v1"), b"VSIFT-MAN")?;

    // Foreign content: an unknown file in a marked stage, a stage with no
    // marker holding something else, and a link where a stage should be.
    private_dir(&root.join(stage_name('c')))?;
    private_file(
        &root.join(stage_name('c')).join("stage-v1"),
        b"VSIFT-MANAGED-STAGE-v1\n",
    )?;
    private_file(&root.join(stage_name('c')).join("keep.txt"), b"keep")?;
    private_dir(&root.join(stage_name('d')))?;
    private_file(&root.join(stage_name('d')).join("keep.txt"), b"keep")?;
    let outside = fixture.0.join("outside");
    fs::create_dir(&outside)?;
    fs::write(outside.join("precious.txt"), b"precious")?;
    let linked_stage = directory_link(&outside, &root.join(stage_name('e')))?;
    // A link inside a payload folder of an otherwise valid stage.
    let linked_file = if linked_stage {
        private_dir(&root.join(stage_name('f')))?;
        private_file(
            &root.join(stage_name('f')).join("stage-v1"),
            b"VSIFT-MANAGED-STAGE-v1\n",
        )?;
        private_dir(&root.join(stage_name('f')).join("payload.pending"))?;
        file_link(
            &outside.join("precious.txt"),
            &root
                .join(stage_name('f'))
                .join("payload.pending")
                .join("tool"),
        )?
    } else {
        false
    };
    // Not a stage name at all: never touched.
    fs::create_dir(root.join("notes"))?;

    let maintainer = GuardedManagedStore::new(&store, &guard);
    let before = maintainer.inventory()?.ok_or("no root")?;
    let retained_expected = 2 + usize::from(linked_stage) + usize::from(linked_file);
    assert_eq!(before.stale_stages, 4);
    assert_eq!(before.retained_stages.len(), retained_expected);
    assert_eq!(before.unexpected_entries, 1);

    let sweep = maintainer.sweep_stages()?;
    assert_eq!(sweep.removed, 4);
    assert_eq!(sweep.retained.len(), retained_expected);
    assert!(
        sweep
            .retained
            .contains(&StageRetentionReason::UnexpectedContent)
    );
    assert!(
        sweep
            .retained
            .contains(&StageRetentionReason::OwnershipUnproved)
    );

    let after = maintainer.inventory()?.ok_or("no root")?;
    assert_eq!(after.stale_stages, 0);
    assert_eq!(after.retained_stages.len(), retained_expected);
    for stage in &stages {
        assert!(!stage.exists(), "{} survived", stage.display());
    }
    assert!(!root.join(stage_name('a')).exists());
    assert!(!root.join(stage_name('b')).exists());
    assert_eq!(
        fs::read(root.join(stage_name('c')).join("keep.txt"))?,
        b"keep"
    );
    assert_eq!(
        fs::read(root.join(stage_name('d')).join("keep.txt"))?,
        b"keep"
    );
    assert_eq!(fs::read(outside.join("precious.txt"))?, b"precious");
    assert!(root.join("notes").is_dir());
    if linked_stage {
        assert!(
            fs::symlink_metadata(root.join(stage_name('e')))?
                .file_type()
                .is_symlink()
        );
    }
    Ok(())
}

#[test]
fn a_linked_version_folder_is_not_followed_or_deleted_through() -> TestResult {
    let fixture = Fixture::new()?;
    let store = fixture.store()?;
    let guard = store.try_install_guard()?;
    drop(publish(&store, &guard, "1.0.0")?);
    let outside = fixture.0.join("outside");
    fs::create_dir(&outside)?;
    fs::write(outside.join("tool"), b"precious")?;
    if !directory_link(&outside, &fixture.version_path("9.9.9"))? {
        println!("skipped: this account may not create links");
        return Ok(());
    }
    let maintainer = GuardedManagedStore::new(&store, &guard);
    assert_eq!(
        cli_state(&maintainer, "9.9.9")?,
        Some(ManagedVersionState::Unverified(
            ManagedVersionFault::UnexpectedEntry
        ))
    );
    assert_eq!(
        roll_back_component(&maintainer, CLI, Some(&key("9.9.9")?)),
        Err(ManagedLifecycleRefusal::VersionUnverified)
    );
    assert_eq!(
        maintainer.remove_version(CLI, "9.9.9"),
        VersionRemovalStatus::UnexpectedContent
    );
    let report = remove_managed(&maintainer, &ManagedRemovalTarget::Component(CLI))?;
    assert_eq!(
        report.failure_code(),
        Some(vsift_domain::FailureCode::StorageIo)
    );
    assert_eq!(fs::read(outside.join("tool"))?, b"precious");
    assert!(
        fs::symlink_metadata(fixture.version_path("9.9.9"))?
            .file_type()
            .is_symlink()
    );
    Ok(())
}

#[test]
fn a_changed_root_marker_fails_inspection_closed() -> TestResult {
    let fixture = Fixture::new()?;
    let store = fixture.store()?;
    drop(store.try_install_guard()?);
    let marker = fixture.root().join("owner-v1");
    fs::remove_file(&marker)?;
    private_file(&marker, b"someone else's folder\n")?;
    assert_eq!(
        store.inspect(),
        Err(vsift_application::ManagedStoreFault::Unsafe)
    );
    assert!(matches!(
        store.try_existing_install_guard(),
        Err(ManagedArtifactError::UnsafeStorage)
    ));
    Ok(())
}
