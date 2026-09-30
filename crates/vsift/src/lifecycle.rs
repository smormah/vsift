//! `setup list`, `setup rollback`, `setup remove` and `setup repair`: the
//! managed store's lifecycle after installation (P13 PR 6, ADR 0023 §3
//! steps 5 and 6).
//!
//! `list` and `repair` only read: they never create the managed root, never
//! take the install guard and never change anything; `repair` answers with a
//! plan of existing commands (ADR 0007: "repair ... emits a plan"), which
//! the user runs. `rollback` and `remove` take the root's install guard
//! without waiting (a held guard is [`EngineError::ManagedInstallBusy`]) and
//! never create the root: on a machine where nothing was installed there is
//! nothing to change. None of them downloads anything; a reinstall is always
//! a new reviewed plan accepted by `setup install`.
//!
//! These commands work on every platform: managed installation is qualified
//! on Ubuntu 24.04 x86-64 only (ADR 0023 decision E), so elsewhere the store
//! is normally absent, `list` and `repair` report that and the managed
//! availability, and `rollback` answers that nothing is installed.

use vsift_application::{
    ManagedInventory, ManagedLifecycleRefusal, ManagedPlanAvailability, ManagedRemovalTarget,
    ManagedStoreReader, RemovalReport, RepairPlan, RollbackOutcome, StageSweep,
    VersionRemovalReport, clean_up_versions, diagnose_managed_store, remove_managed,
    roll_back_component,
};
use vsift_domain::{ManagedComponent, ManagedVersionKey};
use vsift_infrastructure::{
    GuardedManagedStore, ManagedArtifactError, ManagedArtifactStore, ManagedInstallGuard,
    ManagedStoreInspector,
};

use crate::{engine::Engine, error::EngineError};

/// What `setup list` found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedListing {
    availability: ManagedPlanAvailability,
    inventory: Option<ManagedInventory>,
}

impl ManagedListing {
    /// Whether managed installation can install on this host.
    #[must_use]
    pub const fn availability(&self) -> ManagedPlanAvailability {
        self.availability
    }

    /// What the store holds, or `None` when this user has no managed folder.
    #[must_use]
    pub const fn inventory(&self) -> Option<&ManagedInventory> {
        self.inventory.as_ref()
    }
}

/// What `setup repair` found and the plan that fixes it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedRepairDiagnosis {
    availability: ManagedPlanAvailability,
    plan: RepairPlan,
}

impl ManagedRepairDiagnosis {
    /// Whether managed installation can install on this host.
    #[must_use]
    pub const fn availability(&self) -> ManagedPlanAvailability {
        self.availability
    }

    /// The findings and the command that fixes each.
    #[must_use]
    pub const fn plan(&self) -> &RepairPlan {
        &self.plan
    }
}

/// What `setup install` cleaned up around its transaction: the stale-stage
/// sweep before it and the bounded version cleanup after it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InstallCleanup {
    /// The sweep of stages abandoned by earlier runs.
    pub stages: StageSweep,
    /// Each version bounded cleanup handled.
    pub versions: Vec<VersionRemovalReport>,
}

/// A store fault as the refusal a lifecycle command reports.
fn store_refusal(error: &ManagedArtifactError) -> EngineError {
    match error {
        ManagedArtifactError::Busy => EngineError::ManagedInstallBusy,
        ManagedArtifactError::UnsafeStorage | ManagedArtifactError::Transfer(_) => {
            EngineError::ManagedLifecycle(ManagedLifecycleRefusal::StoreUnsafe)
        }
        ManagedArtifactError::Unavailable | ManagedArtifactError::Io => {
            EngineError::ManagedLifecycle(ManagedLifecycleRefusal::StoreIo)
        }
    }
}

impl Engine {
    /// Lists every managed component, its selected and previous versions,
    /// and whether each published version verifies. Reads only.
    ///
    /// # Errors
    ///
    /// The managed folder cannot be proved private and `VSift`'s own, or read;
    /// or the built-in catalogue failed its own check.
    pub fn list_managed(&self) -> Result<ManagedListing, EngineError> {
        Ok(ManagedListing {
            availability: self.managed_install_availability()?,
            inventory: self.managed_inventory()?,
        })
    }

    /// Diagnoses the managed store and plans its repair with existing
    /// commands. Reads only: nothing is changed, downloaded or selected.
    ///
    /// # Errors
    ///
    /// As [`Self::list_managed`].
    pub fn repair_managed(&self) -> Result<ManagedRepairDiagnosis, EngineError> {
        let listing = self.list_managed()?;
        Ok(ManagedRepairDiagnosis {
            availability: listing.availability,
            plan: diagnose_managed_store(listing.inventory.as_ref()),
        })
    }

    /// Selects the version `component` had before (`version: None`), or the
    /// named installed version, after verifying it, in one atomic rename.
    ///
    /// # Errors
    ///
    /// [`EngineError::ManagedInstallBusy`] when another command holds the
    /// managed folder, or a typed [`EngineError::ManagedLifecycle`] refusal:
    /// nothing installed, no such or no earlier version, a version that does
    /// not verify, or a store that cannot be proved `VSift`'s own. The
    /// selection is then unchanged.
    pub fn rollback_managed(
        &self,
        component: ManagedComponent,
        version: Option<&ManagedVersionKey>,
    ) -> Result<RollbackOutcome, EngineError> {
        let (store, guard) =
            self.existing_managed_guard()?
                .ok_or(EngineError::ManagedLifecycle(
                    ManagedLifecycleRefusal::NotInstalled,
                ))?;
        roll_back_component(
            &GuardedManagedStore::new(&store, &guard),
            component,
            version,
        )
        .map_err(EngineError::ManagedLifecycle)
    }

    /// Removes what `target` names: one unselected version, a whole
    /// component (its selection first, then every version), or abandoned
    /// stages and half-written selection pointers. A version a running job
    /// holds is kept and reported, never removed.
    ///
    /// # Errors
    ///
    /// [`EngineError::ManagedInstallBusy`] when another command holds the
    /// managed folder, or a typed refusal that changed nothing (the version
    /// named is selected, or the store cannot be proved `VSift`'s own). What
    /// could not be removed is reported in the [`RemovalReport`], not here.
    pub fn remove_managed(
        &self,
        target: &ManagedRemovalTarget,
    ) -> Result<RemovalReport, EngineError> {
        let Some((store, guard)) = self.existing_managed_guard()? else {
            return Ok(RemovalReport {
                deselected: None,
                versions: Vec::new(),
                stages: matches!(target, ManagedRemovalTarget::StaleStages)
                    .then(StageSweep::default),
            });
        };
        remove_managed(&GuardedManagedStore::new(&store, &guard), target)
            .map_err(EngineError::ManagedLifecycle)
    }

    fn managed_inventory(&self) -> Result<Option<ManagedInventory>, EngineError> {
        let Some(store) = self.managed_store() else {
            return Ok(None);
        };
        ManagedStoreInspector::new(store)
            .inventory()
            .map_err(|fault| EngineError::ManagedLifecycle(fault.into()))
    }

    /// The managed store and its install guard, taken without waiting and
    /// without creating the root; `None` when there is no root.
    fn existing_managed_guard(
        &self,
    ) -> Result<Option<(ManagedArtifactStore, ManagedInstallGuard)>, EngineError> {
        let Some(store) = self.managed_store() else {
            return Ok(None);
        };
        let guard = store
            .try_existing_install_guard()
            .map_err(|error| store_refusal(&error))?;
        Ok(guard.map(|guard| (store, guard)))
    }
}

/// The stale-stage sweep `setup install` runs under its guard once the
/// plan is accepted, before anything is staged: no stage can be live then.
/// A sweep that cannot run reports nothing; the transaction still decides
/// the command's result.
pub(crate) fn sweep_before_install(
    store: &ManagedArtifactStore,
    guard: &ManagedInstallGuard,
) -> StageSweep {
    use vsift_application::ManagedStoreMaintenance as _;
    GuardedManagedStore::new(store, guard)
        .sweep_stages()
        .unwrap_or_default()
}

/// The bounded cleanup `setup install` runs under its guard after the
/// transaction: every component keeps its selected and previous versions
/// and any version a job holds.
pub(crate) fn clean_up_after_install(
    store: &ManagedArtifactStore,
    guard: &ManagedInstallGuard,
) -> Vec<VersionRemovalReport> {
    clean_up_versions(&GuardedManagedStore::new(store, guard)).unwrap_or_default()
}
