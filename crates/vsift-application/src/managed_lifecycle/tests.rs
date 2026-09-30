//! The lifecycle rules over an in-memory store: rollback targets and their
//! verification, removal order and refusals, bounded cleanup and the repair
//! plan for every finding.

use std::cell::RefCell;

use vsift_domain::{FailureCode, ManagedComponent, ManagedVersionKey, ManagedVersionKeyError};

use super::{
    ComponentInventory, ManagedInventory, ManagedLifecycleRefusal, ManagedRemovalTarget,
    ManagedSelection, ManagedStoreFault, ManagedStoreMaintenance, ManagedStoreReader,
    ManagedVersionFault, ManagedVersionRecord, ManagedVersionState, PreviousSelection,
    RepairFindingKind, RepairFix, RepairStatus, SelectionFailure, SelectionRecord, SelectionStatus,
    StageSweep, VersionRemovalStatus, clean_up_versions, diagnose_managed_store, remove_managed,
    roll_back_component,
};
use crate::StageRetentionReason;

const CLI: ManagedComponent = ManagedComponent::WhisperCli;

fn key(value: &str) -> Result<ManagedVersionKey, ManagedVersionKeyError> {
    ManagedVersionKey::parse(value)
}

fn verified(version: &str) -> ManagedVersionRecord {
    ManagedVersionRecord {
        version: version.to_owned(),
        state: ManagedVersionState::Verified {
            manifest_sha256: sha(version),
        },
    }
}

fn sha(version: &str) -> String {
    format!("{version:0>64}")
}

fn recorded(version: &str, previous: Option<&str>) -> ManagedSelection {
    ManagedSelection::Recorded(SelectionRecord {
        version: version.to_owned(),
        manifest_sha256: sha(version),
        previous: previous.map(|previous| PreviousSelection {
            version: previous.to_owned(),
            manifest_sha256: sha(previous),
        }),
    })
}

fn inventory_with(
    selection: ManagedSelection,
    versions: Vec<ManagedVersionRecord>,
) -> ManagedInventory {
    let mut inventory = ManagedInventory::empty();
    if let Some(component) = inventory
        .components
        .iter_mut()
        .find(|component| component.component == CLI)
    {
        *component = ComponentInventory {
            component: CLI,
            selection,
            versions,
        };
    }
    inventory
}

/// An in-memory store that records every change it is asked to make.
struct FakeStore {
    inventory: RefCell<Option<ManagedInventory>>,
    in_use: Vec<String>,
    selections: RefCell<Vec<(String, Option<String>)>>,
    removals: RefCell<Vec<String>>,
    deselections: RefCell<usize>,
    refuse_selection: bool,
}

impl FakeStore {
    fn new(inventory: Option<ManagedInventory>) -> Self {
        Self {
            inventory: RefCell::new(inventory),
            in_use: Vec::new(),
            selections: RefCell::new(Vec::new()),
            removals: RefCell::new(Vec::new()),
            deselections: RefCell::new(0),
            refuse_selection: false,
        }
    }

    fn cli(&self) -> Option<ComponentInventory> {
        self.inventory
            .borrow()
            .as_ref()
            .and_then(|inventory| inventory.component(CLI).cloned())
    }
}

impl ManagedStoreReader for FakeStore {
    fn inventory(&self) -> Result<Option<ManagedInventory>, ManagedStoreFault> {
        Ok(self.inventory.borrow().clone())
    }
}

impl ManagedStoreMaintenance for FakeStore {
    fn select(
        &self,
        _component: ManagedComponent,
        version: &str,
        expected_manifest_sha256: Option<&str>,
    ) -> Result<(), SelectionFailure> {
        if self.refuse_selection {
            return Err(SelectionFailure::Unverified);
        }
        self.selections.borrow_mut().push((
            version.to_owned(),
            expected_manifest_sha256.map(str::to_owned),
        ));
        Ok(())
    }

    fn deselect(&self, _component: ManagedComponent) -> Result<bool, ManagedStoreFault> {
        *self.deselections.borrow_mut() += 1;
        let mut inventory = self.inventory.borrow_mut();
        if let Some(component) = inventory
            .as_mut()
            .and_then(|inventory| inventory.components.iter_mut().find(|c| c.component == CLI))
        {
            component.selection = ManagedSelection::None;
        }
        Ok(true)
    }

    fn remove_version(&self, _component: ManagedComponent, version: &str) -> VersionRemovalStatus {
        if self.in_use.iter().any(|held| held == version) {
            return VersionRemovalStatus::InUse;
        }
        if self
            .cli()
            .and_then(|cli| cli.selected_version().map(str::to_owned))
            == Some(version.to_owned())
        {
            return VersionRemovalStatus::Selected;
        }
        self.removals.borrow_mut().push(version.to_owned());
        VersionRemovalStatus::Removed
    }

    fn sweep_stages(&self) -> Result<StageSweep, ManagedStoreFault> {
        Ok(StageSweep {
            removed: 2,
            retained: vec![StageRetentionReason::UnexpectedContent],
            interrupted_selections_removed: 1,
        })
    }
}

#[test]
fn rollback_selects_the_recorded_previous_version_with_its_manifest()
-> Result<(), Box<dyn std::error::Error>> {
    let store = FakeStore::new(Some(inventory_with(
        recorded("b", Some("a")),
        vec![verified("a"), verified("b")],
    )));
    let outcome = roll_back_component(&store, CLI, None)?;
    assert_eq!(outcome.selected, "a");
    assert_eq!(outcome.replaced.as_deref(), Some("b"));
    assert!(outcome.changed);
    assert_eq!(
        *store.selections.borrow(),
        vec![(String::from("a"), Some(sha("a")))]
    );
    Ok(())
}

#[test]
fn rollback_refuses_without_a_verified_previous_version() {
    for (inventory, expected) in [
        (None, ManagedLifecycleRefusal::NotInstalled),
        (
            Some(ManagedInventory::empty()),
            ManagedLifecycleRefusal::NotInstalled,
        ),
        (
            Some(inventory_with(recorded("b", None), vec![verified("b")])),
            ManagedLifecycleRefusal::NoPreviousVersion,
        ),
        // The previous version was removed since.
        (
            Some(inventory_with(
                recorded("b", Some("a")),
                vec![verified("b")],
            )),
            ManagedLifecycleRefusal::NoPreviousVersion,
        ),
        (
            Some(inventory_with(
                ManagedSelection::Unreadable,
                vec![verified("a"), verified("b")],
            )),
            ManagedLifecycleRefusal::NoPreviousVersion,
        ),
        // Corrupted, being removed, or holding another manifest.
        (
            Some(inventory_with(
                recorded("b", Some("a")),
                vec![
                    ManagedVersionRecord {
                        version: String::from("a"),
                        state: ManagedVersionState::Unverified(ManagedVersionFault::ChangedContent),
                    },
                    verified("b"),
                ],
            )),
            ManagedLifecycleRefusal::VersionUnverified,
        ),
        (
            Some(inventory_with(
                recorded("b", Some("a")),
                vec![
                    ManagedVersionRecord {
                        version: String::from("a"),
                        state: ManagedVersionState::RemovalInterrupted,
                    },
                    verified("b"),
                ],
            )),
            ManagedLifecycleRefusal::VersionUnverified,
        ),
        (
            Some(inventory_with(
                recorded("b", Some("a")),
                vec![
                    ManagedVersionRecord {
                        version: String::from("a"),
                        state: ManagedVersionState::Verified {
                            manifest_sha256: sha("other"),
                        },
                    },
                    verified("b"),
                ],
            )),
            ManagedLifecycleRefusal::VersionUnverified,
        ),
    ] {
        let store = FakeStore::new(inventory);
        assert_eq!(roll_back_component(&store, CLI, None), Err(expected));
        assert!(store.selections.borrow().is_empty());
    }
}

#[test]
fn rollback_to_a_named_version_requires_it_verified_and_is_idempotent()
-> Result<(), Box<dyn std::error::Error>> {
    let inventory = inventory_with(
        recorded("b", Some("a")),
        vec![
            verified("a"),
            verified("b"),
            ManagedVersionRecord {
                version: String::from("c"),
                state: ManagedVersionState::Unverified(ManagedVersionFault::MissingManifest),
            },
        ],
    );
    let store = FakeStore::new(Some(inventory));
    assert_eq!(
        roll_back_component(&store, CLI, Some(&key("z")?)),
        Err(ManagedLifecycleRefusal::VersionNotInstalled)
    );
    assert_eq!(
        roll_back_component(&store, CLI, Some(&key("c")?)),
        Err(ManagedLifecycleRefusal::VersionUnverified)
    );
    let same = roll_back_component(&store, CLI, Some(&key("b")?))?;
    assert!(!same.changed);
    assert!(store.selections.borrow().is_empty());
    let named = roll_back_component(&store, CLI, Some(&key("a")?))?;
    assert!(named.changed);
    assert_eq!(*store.selections.borrow(), vec![(String::from("a"), None)]);
    Ok(())
}

#[test]
fn a_selection_the_store_refuses_is_unverified_and_changes_nothing() {
    let mut store = FakeStore::new(Some(inventory_with(
        recorded("b", Some("a")),
        vec![verified("a"), verified("b")],
    )));
    store.refuse_selection = true;
    assert_eq!(
        roll_back_component(&store, CLI, None),
        Err(ManagedLifecycleRefusal::VersionUnverified)
    );
}

#[test]
fn removing_the_selected_version_is_refused_and_a_missing_one_is_already_absent()
-> Result<(), Box<dyn std::error::Error>> {
    let store = FakeStore::new(Some(inventory_with(
        recorded("b", Some("a")),
        vec![verified("a"), verified("b")],
    )));
    assert_eq!(
        remove_managed(
            &store,
            &ManagedRemovalTarget::Version {
                component: CLI,
                version: key("b")?,
            },
        ),
        Err(ManagedLifecycleRefusal::VersionSelected)
    );
    let absent = remove_managed(
        &store,
        &ManagedRemovalTarget::Version {
            component: CLI,
            version: key("z")?,
        },
    )?;
    assert_eq!(
        absent.versions[0].status,
        VersionRemovalStatus::AlreadyAbsent
    );
    assert_eq!(absent.failure_code(), None);
    let removed = remove_managed(
        &store,
        &ManagedRemovalTarget::Version {
            component: CLI,
            version: key("a")?,
        },
    )?;
    assert_eq!(removed.versions[0].status, VersionRemovalStatus::Removed);
    assert_eq!(*store.removals.borrow(), vec![String::from("a")]);
    Ok(())
}

#[test]
fn removing_a_component_deselects_first_and_keeps_versions_in_use()
-> Result<(), Box<dyn std::error::Error>> {
    let mut store = FakeStore::new(Some(inventory_with(
        recorded("b", Some("a")),
        vec![verified("a"), verified("b")],
    )));
    store.in_use = vec![String::from("a")];
    let report = remove_managed(&store, &ManagedRemovalTarget::Component(CLI))?;
    assert_eq!(report.deselected, Some(CLI));
    assert_eq!(*store.deselections.borrow(), 1);
    assert_eq!(report.versions.len(), 2);
    assert_eq!(report.versions[0].status, VersionRemovalStatus::InUse);
    assert_eq!(report.versions[1].status, VersionRemovalStatus::Removed);
    assert_eq!(report.failure_code(), Some(FailureCode::Busy));
    Ok(())
}

#[test]
fn removing_a_component_that_is_not_installed_changes_nothing()
-> Result<(), Box<dyn std::error::Error>> {
    for inventory in [None, Some(ManagedInventory::empty())] {
        let store = FakeStore::new(inventory);
        let report = remove_managed(&store, &ManagedRemovalTarget::Component(CLI))?;
        assert_eq!(report.deselected, None);
        assert!(report.versions.is_empty());
        assert_eq!(*store.deselections.borrow(), 0);
    }
    Ok(())
}

#[test]
fn a_sweep_with_retained_stages_is_a_storage_failure() -> Result<(), Box<dyn std::error::Error>> {
    let store = FakeStore::new(Some(ManagedInventory::empty()));
    let report = remove_managed(&store, &ManagedRemovalTarget::StaleStages)?;
    let stages = report
        .stages
        .as_ref()
        .ok_or(ManagedLifecycleRefusal::StoreIo)?;
    assert_eq!(stages.removed, 2);
    assert_eq!(report.failure_code(), Some(FailureCode::StorageIo));
    let nothing = remove_managed(&FakeStore::new(None), &ManagedRemovalTarget::StaleStages)?;
    assert_eq!(nothing.stages, Some(StageSweep::default()));
    assert_eq!(nothing.failure_code(), None);
    Ok(())
}

#[test]
fn storage_outranks_a_version_in_use_in_the_failure_code() {
    use super::{RemovalReport, VersionRemovalReport};
    let report = RemovalReport {
        deselected: None,
        versions: [
            VersionRemovalStatus::InUse,
            VersionRemovalStatus::UnexpectedContent,
            VersionRemovalStatus::Removed,
        ]
        .into_iter()
        .map(|status| VersionRemovalReport {
            component: CLI,
            version: String::from("a"),
            status,
        })
        .collect(),
        stages: None,
    };
    assert_eq!(report.failure_code(), Some(FailureCode::StorageIo));
}

#[test]
fn cleanup_keeps_the_selected_and_previous_versions_and_what_is_in_use()
-> Result<(), Box<dyn std::error::Error>> {
    let mut store = FakeStore::new(Some(inventory_with(
        recorded("c", Some("b")),
        vec![verified("a"), verified("b"), verified("c"), verified("d")],
    )));
    store.in_use = vec![String::from("d")];
    let removed = clean_up_versions(&store)?;
    let outcomes: Vec<(&str, VersionRemovalStatus)> = removed
        .iter()
        .map(|report| (report.version.as_str(), report.status))
        .collect();
    assert_eq!(
        outcomes,
        vec![
            ("a", VersionRemovalStatus::Removed),
            ("d", VersionRemovalStatus::InUse)
        ]
    );
    Ok(())
}

#[test]
fn cleanup_leaves_a_component_whose_pointer_cannot_be_read()
-> Result<(), Box<dyn std::error::Error>> {
    let store = FakeStore::new(Some(inventory_with(
        ManagedSelection::Unreadable,
        vec![verified("a"), verified("b"), verified("c")],
    )));
    assert!(clean_up_versions(&store)?.is_empty());
    assert!(clean_up_versions(&FakeStore::new(None))?.is_empty());
    Ok(())
}

#[test]
fn repair_of_nothing_and_of_a_healthy_store() {
    assert_eq!(
        diagnose_managed_store(None).status,
        RepairStatus::NothingInstalled
    );
    let healthy = inventory_with(recorded("b", Some("a")), vec![verified("a"), verified("b")]);
    let plan = diagnose_managed_store(Some(&healthy));
    assert_eq!(plan.status, RepairStatus::Healthy);
    assert!(plan.findings.is_empty());
    assert_eq!(
        healthy
            .component(CLI)
            .map(ComponentInventory::selection_status),
        Some(SelectionStatus::Verified)
    );
}

#[test]
fn a_broken_selection_is_fixed_by_rollback_when_a_verified_version_remains() {
    let corrupted = ManagedVersionRecord {
        version: String::from("b"),
        state: ManagedVersionState::Unverified(ManagedVersionFault::ChangedContent),
    };
    for (selection, versions, fix) in [
        (
            recorded("b", Some("a")),
            vec![verified("a"), corrupted.clone()],
            RepairFix::RollBack(CLI),
        ),
        (
            recorded("b", None),
            vec![verified("a"), corrupted.clone()],
            RepairFix::RollBackTo(CLI, String::from("a")),
        ),
        (
            recorded("b", None),
            vec![verified("a"), corrupted.clone(), verified("c")],
            RepairFix::RemoveComponentAndReinstall(CLI),
        ),
        (
            recorded("b", None),
            vec![corrupted.clone()],
            RepairFix::RemoveComponentAndReinstall(CLI),
        ),
        // The pointer names a version that no longer exists.
        (
            recorded("z", Some("a")),
            vec![verified("a")],
            RepairFix::RollBack(CLI),
        ),
    ] {
        let plan = diagnose_managed_store(Some(&inventory_with(selection, versions)));
        assert_eq!(plan.status, RepairStatus::NeedsRepair);
        assert_eq!(
            plan.findings[0].kind,
            RepairFindingKind::SelectedVersionUnverified
        );
        assert_eq!(plan.findings[0].fix, fix);
    }
    let unreadable = diagnose_managed_store(Some(&inventory_with(
        ManagedSelection::Unreadable,
        vec![verified("a")],
    )));
    assert_eq!(
        unreadable.findings[0].kind,
        RepairFindingKind::SelectionUnreadable
    );
    assert_eq!(
        unreadable.findings[0].fix,
        RepairFix::RollBackTo(CLI, String::from("a"))
    );
}

#[test]
fn unselected_faults_stages_pointers_and_entries_each_get_their_fix() {
    let mut inventory = inventory_with(
        recorded("a", None),
        vec![
            verified("a"),
            ManagedVersionRecord {
                version: String::from("b"),
                state: ManagedVersionState::Unverified(ManagedVersionFault::ChangedContent),
            },
            ManagedVersionRecord {
                version: String::from("c"),
                state: ManagedVersionState::RemovalInterrupted,
            },
            ManagedVersionRecord {
                version: String::from("d"),
                state: ManagedVersionState::Unverified(ManagedVersionFault::InvalidManifest),
            },
        ],
    );
    inventory.stale_stages = 3;
    inventory.interrupted_selections = 1;
    inventory.retained_stages = vec![StageRetentionReason::OwnershipUnproved];
    inventory.unexpected_entries = 2;
    let plan = diagnose_managed_store(Some(&inventory));
    let summary: Vec<(RepairFindingKind, RepairFix, Option<usize>)> = plan
        .findings
        .iter()
        .map(|finding| (finding.kind, finding.fix.clone(), finding.count))
        .collect();
    assert_eq!(
        summary,
        vec![
            (
                RepairFindingKind::VersionUnverified,
                RepairFix::RemoveVersion(CLI, String::from("b")),
                None
            ),
            (
                RepairFindingKind::RemovalInterrupted,
                RepairFix::RemoveVersion(CLI, String::from("c")),
                None
            ),
            (
                RepairFindingKind::VersionUnverified,
                RepairFix::Manual,
                None
            ),
            (
                RepairFindingKind::StaleStages,
                RepairFix::RemoveStaleStages,
                Some(3)
            ),
            (
                RepairFindingKind::InterruptedSelection,
                RepairFix::RemoveStaleStages,
                Some(1)
            ),
            (
                RepairFindingKind::StagesNeedManualRemoval,
                RepairFix::Manual,
                Some(1)
            ),
            (
                RepairFindingKind::UnexpectedEntries,
                RepairFix::Manual,
                Some(2)
            ),
        ]
    );
}
