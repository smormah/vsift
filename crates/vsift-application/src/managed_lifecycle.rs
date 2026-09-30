//! The managed store's lifecycle after installation (P13 PR 6, ADR 0023 §3
//! steps 5 and 6): what is installed (`setup list`), returning to the
//! version selected before (`setup rollback`), removing a version, a
//! component or abandoned stages (`setup remove`), the read-only diagnosis
//! and repair plan (`setup repair`), and the bounded cleanup that keeps the
//! selected and one previous version of each component.
//!
//! The rules live here; infrastructure only reports what the private store
//! holds and performs one guarded change at a time behind
//! [`ManagedStoreMaintenance`]:
//!
//! - a version is selected only after it verifies (its manifest exists and
//!   every file matches it); a rollback to the recorded previous version also
//!   requires the manifest the pointer recorded;
//! - the selected version is never removed on its own: `setup remove
//!   <component>` deselects first, then removes, and a version a running job
//!   holds is kept and reported, never removed;
//! - `setup repair` changes nothing (ADR 0007 and ADR 0023: it "diagnoses and
//!   emits a plan"); each finding names the existing command that fixes it,
//!   and nothing here ever downloads, so a reinstall is always a new accepted
//!   plan.

use std::{error::Error, fmt};

use vsift_domain::{FailureCode, ManagedComponent, ManagedVersionKey};

use crate::StageRetentionReason;

/// How many versions of a component bounded cleanup keeps: the selected one
/// and the one selected before it (ADR 0023 §3 step 6).
pub const RETAINED_MANAGED_VERSIONS: usize = 2;

/// Every managed component, in the order installation and listings use.
pub const MANAGED_COMPONENTS: [ManagedComponent; 3] = [
    ManagedComponent::MediaTools,
    ManagedComponent::WhisperCli,
    ManagedComponent::WhisperModel,
];

/// Why a published version does not verify.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedVersionFault {
    /// The version has no manifest, so nothing in it can be checked.
    MissingManifest,
    /// The manifest cannot be read as one, or names another version.
    InvalidManifest,
    /// A file named by the manifest is missing, or its bytes or mode differ.
    ChangedContent,
    /// The version holds a link, a folder or a name the manifest does not
    /// give, or is not a private folder.
    UnexpectedEntry,
    /// The version could not be read.
    Unreadable,
}

impl ManagedVersionFault {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::MissingManifest => "missing_manifest",
            Self::InvalidManifest => "invalid_manifest",
            Self::ChangedContent => "changed_content",
            Self::UnexpectedEntry => "unexpected_entry",
            Self::Unreadable => "unreadable",
        }
    }

    /// Whether `setup remove` can remove a version with this fault: only
    /// when its manifest still names every file it may hold.
    #[must_use]
    pub const fn removable(self) -> bool {
        matches!(self, Self::ChangedContent)
    }
}

/// What one published version is now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedVersionState {
    /// Every file matches the manifest, whose SHA-256 is given.
    Verified {
        /// SHA-256 of the version's manifest: its content identity.
        manifest_sha256: String,
    },
    /// A removal started and was interrupted; nothing may open it, and
    /// `setup remove` finishes it.
    RemovalInterrupted,
    /// The version does not verify and is never run or selected.
    Unverified(ManagedVersionFault),
}

impl ManagedVersionState {
    /// Stable identifier: `verified`, `removal_interrupted` or `unverified`.
    #[must_use]
    pub const fn identifier(&self) -> &'static str {
        match self {
            Self::Verified { .. } => "verified",
            Self::RemovalInterrupted => "removal_interrupted",
            Self::Unverified(_) => "unverified",
        }
    }

    /// The fault of an unverified version.
    #[must_use]
    pub const fn fault(&self) -> Option<ManagedVersionFault> {
        match self {
            Self::Unverified(fault) => Some(*fault),
            Self::Verified { .. } | Self::RemovalInterrupted => None,
        }
    }
}

/// One published version of a component.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedVersionRecord {
    /// The version key, a bounded canonical identifier.
    pub version: String,
    /// Whether it verifies.
    pub state: ManagedVersionState,
}

/// The version a selection pointer records as selected before the current
/// one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreviousSelection {
    /// The version key.
    pub version: String,
    /// The SHA-256 of its manifest when it was selected.
    pub manifest_sha256: String,
}

/// A readable selection pointer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionRecord {
    /// The selected version key.
    pub version: String,
    /// The SHA-256 of the manifest the pointer names.
    pub manifest_sha256: String,
    /// The version selected before it, if the pointer records one.
    pub previous: Option<PreviousSelection>,
}

/// What a component's selection pointer says.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedSelection {
    /// No version is selected.
    None,
    /// The pointer names a version.
    Recorded(SelectionRecord),
    /// The pointer exists but cannot be read as one; lookup ignores it.
    Unreadable,
}

/// What the selection of a component amounts to for lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionStatus {
    /// Nothing is selected.
    None,
    /// The selected version verifies and its manifest is the one the
    /// pointer names: commands use it.
    Verified,
    /// The pointer names a version that is missing, does not verify, is
    /// being removed or holds another manifest: commands do not use it.
    Unverified,
    /// The pointer cannot be read: commands do not use it.
    Unreadable,
}

impl SelectionStatus {
    /// Stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Verified => "verified",
            Self::Unverified => "unverified",
            Self::Unreadable => "unreadable",
        }
    }
}

/// Everything the store holds for one component.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentInventory {
    /// The component.
    pub component: ManagedComponent,
    /// Its selection pointer.
    pub selection: ManagedSelection,
    /// Its published versions, ordered by version key.
    pub versions: Vec<ManagedVersionRecord>,
}

impl ComponentInventory {
    /// The record of `version`, if it is published.
    #[must_use]
    pub fn version(&self, version: &str) -> Option<&ManagedVersionRecord> {
        self.versions
            .iter()
            .find(|record| record.version == version)
    }

    /// The selected version key, when the pointer names one.
    #[must_use]
    pub fn selected_version(&self) -> Option<&str> {
        match &self.selection {
            ManagedSelection::Recorded(record) => Some(record.version.as_str()),
            ManagedSelection::None | ManagedSelection::Unreadable => None,
        }
    }

    /// The previous version key the pointer records.
    #[must_use]
    pub fn previous_version(&self) -> Option<&str> {
        match &self.selection {
            ManagedSelection::Recorded(SelectionRecord {
                previous: Some(previous),
                ..
            }) => Some(previous.version.as_str()),
            _ => None,
        }
    }

    /// Whether commands use the selected version.
    #[must_use]
    pub fn selection_status(&self) -> SelectionStatus {
        match &self.selection {
            ManagedSelection::None => SelectionStatus::None,
            ManagedSelection::Unreadable => SelectionStatus::Unreadable,
            ManagedSelection::Recorded(record) => match self.version(&record.version) {
                Some(ManagedVersionRecord {
                    state: ManagedVersionState::Verified { manifest_sha256 },
                    ..
                }) if *manifest_sha256 == record.manifest_sha256 => SelectionStatus::Verified,
                _ => SelectionStatus::Unverified,
            },
        }
    }

    /// The versions that verify, other than the selected one, by key.
    fn other_verified(&self) -> impl Iterator<Item = &ManagedVersionRecord> {
        let selected = self.selected_version();
        self.versions.iter().filter(move |record| {
            Some(record.version.as_str()) != selected
                && matches!(record.state, ManagedVersionState::Verified { .. })
        })
    }
}

/// Everything the private managed store holds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedInventory {
    /// Every managed component, in [`MANAGED_COMPONENTS`] order, also those
    /// with nothing installed.
    pub components: Vec<ComponentInventory>,
    /// Abandoned stages a sweep would remove.
    pub stale_stages: usize,
    /// Stages a sweep would keep, and why: nothing proves them `VSift`'s own,
    /// or they hold a link, a folder or an unknown name.
    pub retained_stages: Vec<StageRetentionReason>,
    /// Selection pointers left half-written by an interrupted selection;
    /// they are never read, and a sweep removes them.
    pub interrupted_selections: usize,
    /// Entries of the store no `VSift` operation creates. They are never
    /// removed; the user removes them.
    pub unexpected_entries: usize,
}

impl ManagedInventory {
    /// An inventory with nothing installed.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            components: MANAGED_COMPONENTS
                .into_iter()
                .map(|component| ComponentInventory {
                    component,
                    selection: ManagedSelection::None,
                    versions: Vec::new(),
                })
                .collect(),
            stale_stages: 0,
            retained_stages: Vec::new(),
            interrupted_selections: 0,
            unexpected_entries: 0,
        }
    }

    /// The inventory of `component`.
    #[must_use]
    pub fn component(&self, component: ManagedComponent) -> Option<&ComponentInventory> {
        self.components
            .iter()
            .find(|inventory| inventory.component == component)
    }
}

/// A failure of the store itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedStoreFault {
    /// The managed folder, or one of its own folders, cannot be proved
    /// `VSift`'s and private (a link, another owner, open permissions, a
    /// changed marker).
    Unsafe,
    /// Reading or changing it failed.
    Io,
}

/// Why a version could not be selected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionFailure {
    /// The version is missing, being removed, or does not verify (or holds
    /// another manifest than the one the pointer recorded).
    Unverified,
    /// The store failed.
    Store(ManagedStoreFault),
}

/// What happened to one version `setup remove` or cleanup tried to remove.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionRemovalStatus {
    /// It was removed, or its interrupted removal finished.
    Removed,
    /// It was not installed; nothing to do.
    AlreadyAbsent,
    /// A running job holds it; it was kept and a later `setup remove`
    /// removes it.
    InUse,
    /// It is the selected version and was kept.
    Selected,
    /// Its content could not be proved `VSift`'s own (an invalid manifest, a
    /// link, a folder or an unknown name), so it was kept for the user.
    UnexpectedContent,
    /// Storage failed while removing it; what remains is kept.
    StorageFailure,
}

impl VersionRemovalStatus {
    /// Stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::AlreadyAbsent => "already_absent",
            Self::InUse => "in_use",
            Self::Selected => "selected",
            Self::UnexpectedContent => "unexpected_content",
            Self::StorageFailure => "storage_failure",
        }
    }

    /// The failure code a removal that ends this way reports, or `None`
    /// when it did what was asked.
    #[must_use]
    pub const fn failure_code(self) -> Option<FailureCode> {
        match self {
            Self::Removed | Self::AlreadyAbsent => None,
            Self::InUse => Some(FailureCode::Busy),
            Self::Selected => Some(FailureCode::InvalidArgument),
            Self::UnexpectedContent | Self::StorageFailure => Some(FailureCode::StorageIo),
        }
    }
}

/// What a stale-stage sweep did.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StageSweep {
    /// Abandoned stages removed.
    pub removed: usize,
    /// Stages kept, and why.
    pub retained: Vec<StageRetentionReason>,
    /// Half-written selection pointers removed.
    pub interrupted_selections_removed: usize,
}

/// Reads the private managed store without changing it.
pub trait ManagedStoreReader {
    /// What the store holds, or `None` when this user has no managed folder.
    ///
    /// # Errors
    ///
    /// The folder or one of its own folders cannot be proved private and
    /// `VSift`'s, or cannot be read.
    fn inventory(&self) -> Result<Option<ManagedInventory>, ManagedStoreFault>;
}

/// Changes the private managed store, one guarded step at a time.
///
/// Implementations hold the root's install guard for their whole life, so
/// no installation, rollback or removal runs beside them; each step is
/// crash-consistent on its own (an atomic rename, or a removal that a rerun
/// finishes).
pub trait ManagedStoreMaintenance: ManagedStoreReader {
    /// Selects `version` of `component` after verifying it; with
    /// `expected_manifest_sha256`, its manifest must be exactly that one.
    ///
    /// # Errors
    ///
    /// [`SelectionFailure::Unverified`] when it does not verify; the
    /// selection is then unchanged.
    fn select(
        &self,
        component: ManagedComponent,
        version: &str,
        expected_manifest_sha256: Option<&str>,
    ) -> Result<(), SelectionFailure>;

    /// Removes `component`'s selection pointer (and any half-written one),
    /// so commands stop using it. `true` when a pointer was removed.
    ///
    /// # Errors
    ///
    /// The store failed; the selection is then unchanged.
    fn deselect(&self, component: ManagedComponent) -> Result<bool, ManagedStoreFault>;

    /// Removes one unselected version unless a running job holds it.
    fn remove_version(&self, component: ManagedComponent, version: &str) -> VersionRemovalStatus;

    /// Removes every abandoned stage and half-written pointer it can prove
    /// `VSift`'s own, and reports the rest.
    ///
    /// # Errors
    ///
    /// The store itself cannot be read.
    fn sweep_stages(&self) -> Result<StageSweep, ManagedStoreFault>;
}

/// Why a lifecycle command changed nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedLifecycleRefusal {
    /// No version of the component is installed.
    NotInstalled,
    /// The version named is not installed.
    VersionNotInstalled,
    /// No earlier version is recorded, or it is no longer installed.
    NoPreviousVersion,
    /// The version to select does not verify, or lacks its manifest.
    VersionUnverified,
    /// The version named is the selected one; remove the component, or roll
    /// back first.
    VersionSelected,
    /// The store cannot be proved `VSift`'s own and private.
    StoreUnsafe,
    /// The store could not be read or changed.
    StoreIo,
}

impl ManagedLifecycleRefusal {
    /// The public failure code.
    #[must_use]
    pub const fn failure_code(self) -> FailureCode {
        match self {
            Self::NotInstalled
            | Self::VersionNotInstalled
            | Self::NoPreviousVersion
            | Self::VersionSelected => FailureCode::InvalidArgument,
            Self::VersionUnverified => FailureCode::IntegrityFailure,
            Self::StoreUnsafe | Self::StoreIo => FailureCode::StorageIo,
        }
    }
}

impl fmt::Display for ManagedStoreFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unsafe => "the managed folder cannot be proved private and VSift's own",
            Self::Io => "the managed folder could not be read or changed",
        })
    }
}

impl Error for ManagedStoreFault {}

impl fmt::Display for ManagedLifecycleRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotInstalled => "no managed version of the component is installed",
            Self::VersionNotInstalled => "the managed version is not installed",
            Self::NoPreviousVersion => "no earlier managed version is recorded and installed",
            Self::VersionUnverified => "the managed version does not verify",
            Self::VersionSelected => "the managed version is the selected one",
            Self::StoreUnsafe => "the managed folder cannot be proved private and VSift's own",
            Self::StoreIo => "the managed folder could not be read or changed",
        })
    }
}

impl Error for ManagedLifecycleRefusal {}

impl From<ManagedStoreFault> for ManagedLifecycleRefusal {
    fn from(fault: ManagedStoreFault) -> Self {
        match fault {
            ManagedStoreFault::Unsafe => Self::StoreUnsafe,
            ManagedStoreFault::Io => Self::StoreIo,
        }
    }
}

/// What `setup rollback` did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RollbackOutcome {
    /// The component.
    pub component: ManagedComponent,
    /// The version selected now.
    pub selected: String,
    /// The version selected before, which a second rollback returns to.
    pub replaced: Option<String>,
    /// `false` when `selected` was already the verified selection.
    pub changed: bool,
}

/// Selects the version `component` had before (`version: None`), or the
/// named installed version, after verifying it.
///
/// # Errors
///
/// A typed refusal; the selection is then unchanged.
pub fn roll_back_component<M: ManagedStoreMaintenance>(
    store: &M,
    component: ManagedComponent,
    version: Option<&ManagedVersionKey>,
) -> Result<RollbackOutcome, ManagedLifecycleRefusal> {
    let inventory = store
        .inventory()?
        .ok_or(ManagedLifecycleRefusal::NotInstalled)?;
    let installed = inventory
        .component(component)
        .ok_or(ManagedLifecycleRefusal::NotInstalled)?;
    if installed.versions.is_empty() {
        return Err(ManagedLifecycleRefusal::NotInstalled);
    }
    let replaced = installed.selected_version().map(str::to_owned);
    let (target, expected_manifest) = if let Some(version) = version.map(ManagedVersionKey::as_str)
    {
        let record = installed
            .version(version)
            .ok_or(ManagedLifecycleRefusal::VersionNotInstalled)?;
        if replaced.as_deref() == Some(version)
            && installed.selection_status() == SelectionStatus::Verified
        {
            return Ok(RollbackOutcome {
                component,
                selected: version.to_owned(),
                replaced: installed.previous_version().map(str::to_owned),
                changed: false,
            });
        }
        if !matches!(record.state, ManagedVersionState::Verified { .. }) {
            return Err(ManagedLifecycleRefusal::VersionUnverified);
        }
        (version.to_owned(), None)
    } else {
        let ManagedSelection::Recorded(SelectionRecord {
            previous: Some(previous),
            ..
        }) = &installed.selection
        else {
            return Err(ManagedLifecycleRefusal::NoPreviousVersion);
        };
        let record = installed
            .version(&previous.version)
            .ok_or(ManagedLifecycleRefusal::NoPreviousVersion)?;
        match &record.state {
            ManagedVersionState::Verified { manifest_sha256 }
                if *manifest_sha256 == previous.manifest_sha256 => {}
            _ => return Err(ManagedLifecycleRefusal::VersionUnverified),
        }
        (
            previous.version.clone(),
            Some(previous.manifest_sha256.clone()),
        )
    };
    store
        .select(component, &target, expected_manifest.as_deref())
        .map_err(|failure| match failure {
            SelectionFailure::Unverified => ManagedLifecycleRefusal::VersionUnverified,
            SelectionFailure::Store(fault) => fault.into(),
        })?;
    Ok(RollbackOutcome {
        component,
        selected: target,
        replaced,
        changed: true,
    })
}

/// What `setup remove` is asked to remove.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedRemovalTarget {
    /// One unselected version of a component.
    Version {
        /// The component.
        component: ManagedComponent,
        /// The version key, checked canonical.
        version: ManagedVersionKey,
    },
    /// A whole component: its selection, then every version.
    Component(ManagedComponent),
    /// Abandoned stages and half-written selection pointers.
    StaleStages,
}

/// One version a removal or cleanup handled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VersionRemovalReport {
    /// The component.
    pub component: ManagedComponent,
    /// The version key.
    pub version: String,
    /// What happened to it.
    pub status: VersionRemovalStatus,
}

/// What `setup remove` did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemovalReport {
    /// The component whose selection was removed, when one was.
    pub deselected: Option<ManagedComponent>,
    /// Each version handled, in order.
    pub versions: Vec<VersionRemovalReport>,
    /// What a stale-stage sweep did, when one ran.
    pub stages: Option<StageSweep>,
}

impl RemovalReport {
    /// The failure code of the removal: storage first (the user must act),
    /// then a version in use (retry later), then a selected version, or
    /// `None` when everything asked for is gone.
    #[must_use]
    pub fn failure_code(&self) -> Option<FailureCode> {
        let codes = self
            .versions
            .iter()
            .filter_map(|report| report.status.failure_code())
            .chain(
                self.stages
                    .as_ref()
                    .filter(|sweep| !sweep.retained.is_empty())
                    .map(|_| FailureCode::StorageIo),
            );
        let mut worst = None;
        for code in codes {
            worst = Some(match (worst, code) {
                (Some(FailureCode::StorageIo), _) | (_, FailureCode::StorageIo) => {
                    FailureCode::StorageIo
                }
                (Some(FailureCode::Busy), _) | (_, FailureCode::Busy) => FailureCode::Busy,
                (_, other) => other,
            });
        }
        worst
    }
}

/// Removes what `target` names.
///
/// # Errors
///
/// A refusal that changed nothing: the store is unsafe or unreadable, or
/// the version named is the selected one.
pub fn remove_managed<M: ManagedStoreMaintenance>(
    store: &M,
    target: &ManagedRemovalTarget,
) -> Result<RemovalReport, ManagedLifecycleRefusal> {
    let mut report = RemovalReport {
        deselected: None,
        versions: Vec::new(),
        stages: None,
    };
    match target {
        ManagedRemovalTarget::StaleStages => {
            if store.inventory()?.is_some() {
                report.stages = Some(store.sweep_stages()?);
            } else {
                report.stages = Some(StageSweep::default());
            }
        }
        ManagedRemovalTarget::Version { component, version } => {
            let status = match store
                .inventory()?
                .as_ref()
                .and_then(|inventory| inventory.component(*component))
            {
                Some(installed) if installed.selected_version() == Some(version.as_str()) => {
                    return Err(ManagedLifecycleRefusal::VersionSelected);
                }
                Some(installed) if installed.version(version.as_str()).is_some() => {
                    store.remove_version(*component, version.as_str())
                }
                _ => VersionRemovalStatus::AlreadyAbsent,
            };
            report.versions.push(VersionRemovalReport {
                component: *component,
                version: version.as_str().to_owned(),
                status,
            });
        }
        ManagedRemovalTarget::Component(component) => {
            let Some(inventory) = store.inventory()? else {
                return Ok(report);
            };
            let Some(installed) = inventory.component(*component) else {
                return Ok(report);
            };
            if installed.selection != ManagedSelection::None && store.deselect(*component)? {
                report.deselected = Some(*component);
            }
            for record in &installed.versions {
                report.versions.push(VersionRemovalReport {
                    component: *component,
                    version: record.version.clone(),
                    status: store.remove_version(*component, &record.version),
                });
            }
        }
    }
    Ok(report)
}

/// Bounded cleanup after an installation: for each component whose pointer
/// can be read, every version other than the selected one and the one
/// selected before it is removed unless a running job holds it. A
/// component whose pointer cannot be read is left alone, because what it
/// selects is unknown.
///
/// # Errors
///
/// The store cannot be read or proved `VSift`'s own; nothing was removed.
pub fn clean_up_versions<M: ManagedStoreMaintenance>(
    store: &M,
) -> Result<Vec<VersionRemovalReport>, ManagedLifecycleRefusal> {
    let Some(inventory) = store.inventory()? else {
        return Ok(Vec::new());
    };
    let mut removed = Vec::new();
    for installed in &inventory.components {
        let keep: Vec<&str> = match &installed.selection {
            ManagedSelection::Unreadable => continue,
            ManagedSelection::None => Vec::new(),
            ManagedSelection::Recorded(record) => [Some(record.version.as_str())]
                .into_iter()
                .chain([record
                    .previous
                    .as_ref()
                    .map(|previous| previous.version.as_str())])
                .flatten()
                .take(RETAINED_MANAGED_VERSIONS)
                .collect(),
        };
        for record in &installed.versions {
            if keep.contains(&record.version.as_str()) {
                continue;
            }
            removed.push(VersionRemovalReport {
                component: installed.component,
                version: record.version.clone(),
                status: store.remove_version(installed.component, &record.version),
            });
        }
    }
    Ok(removed)
}

/// What a finding of `setup repair` is about.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepairFindingKind {
    /// The selection pointer cannot be read; commands ignore it.
    SelectionUnreadable,
    /// The selected version is missing, does not verify or is being
    /// removed; commands do not use it.
    SelectedVersionUnverified,
    /// An unselected version does not verify.
    VersionUnverified,
    /// A version's removal was interrupted.
    RemovalInterrupted,
    /// Abandoned stages from an interrupted or failed installation.
    StaleStages,
    /// Stages nothing proves `VSift`'s own, or holding a link, a folder or an
    /// unknown name.
    StagesNeedManualRemoval,
    /// Half-written selection pointers from an interrupted selection.
    InterruptedSelection,
    /// Entries of the managed folder no `VSift` operation creates.
    UnexpectedEntries,
}

impl RepairFindingKind {
    /// Stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::SelectionUnreadable => "selection_unreadable",
            Self::SelectedVersionUnverified => "selected_version_unverified",
            Self::VersionUnverified => "version_unverified",
            Self::RemovalInterrupted => "removal_interrupted",
            Self::StaleStages => "stale_stages",
            Self::StagesNeedManualRemoval => "stages_need_manual_removal",
            Self::InterruptedSelection => "interrupted_selection",
            Self::UnexpectedEntries => "unexpected_entries",
        }
    }
}

/// The existing command that fixes a finding. Repair never applies it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepairFix {
    /// `setup rollback <component>`: return to the recorded previous version.
    RollBack(ManagedComponent),
    /// `setup rollback <component> --version <version>`: select that
    /// verified version.
    RollBackTo(ManagedComponent, String),
    /// `setup remove <component> --version <version>`.
    RemoveVersion(ManagedComponent, String),
    /// `setup remove <component>`, then a new reviewed plan and `setup
    /// install` (or the manual path): no verified version is left to select.
    RemoveComponentAndReinstall(ManagedComponent),
    /// `setup remove --stale-stages`.
    RemoveStaleStages,
    /// Nothing `VSift` may remove: the user deletes it, or the whole managed
    /// folder, and installs again.
    Manual,
}

impl RepairFix {
    /// Stable identifier.
    #[must_use]
    pub const fn identifier(&self) -> &'static str {
        match self {
            Self::RollBack(_) => "rollback",
            Self::RollBackTo(..) => "rollback_to_version",
            Self::RemoveVersion(..) => "remove_version",
            Self::RemoveComponentAndReinstall(_) => "remove_component_and_reinstall",
            Self::RemoveStaleStages => "remove_stale_stages",
            Self::Manual => "manual",
        }
    }
}

/// One problem `setup repair` found and the command that fixes it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairFinding {
    /// What is wrong.
    pub kind: RepairFindingKind,
    /// The component concerned, if one.
    pub component: Option<ManagedComponent>,
    /// The version concerned, if one.
    pub version: Option<String>,
    /// How many items, for a finding about stages, pointers or entries.
    pub count: Option<usize>,
    /// The version fault, for an unverified version.
    pub fault: Option<ManagedVersionFault>,
    /// How to fix it.
    pub fix: RepairFix,
}

/// Whether the store needs anything.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepairStatus {
    /// There is no managed folder.
    NothingInstalled,
    /// Nothing needs fixing.
    Healthy,
    /// At least one finding.
    NeedsRepair,
}

impl RepairStatus {
    /// Stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::NothingInstalled => "nothing_installed",
            Self::Healthy => "healthy",
            Self::NeedsRepair => "needs_repair",
        }
    }
}

/// `setup repair`'s read-only diagnosis: the findings, each with the
/// command that fixes it, in the order to run them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairPlan {
    /// The overall status.
    pub status: RepairStatus,
    /// Each finding.
    pub findings: Vec<RepairFinding>,
}

/// Diagnoses `inventory` (`None`: no managed folder) and plans its repair
/// with existing commands only. Pure: it reads nothing and changes nothing.
#[must_use]
pub fn diagnose_managed_store(inventory: Option<&ManagedInventory>) -> RepairPlan {
    let Some(inventory) = inventory else {
        return RepairPlan {
            status: RepairStatus::NothingInstalled,
            findings: Vec::new(),
        };
    };
    let mut findings = Vec::new();
    for installed in &inventory.components {
        diagnose_component(installed, &mut findings);
    }
    if inventory.stale_stages > 0 || inventory.interrupted_selections > 0 {
        if inventory.stale_stages > 0 {
            findings.push(count_finding(
                RepairFindingKind::StaleStages,
                inventory.stale_stages,
                RepairFix::RemoveStaleStages,
            ));
        }
        if inventory.interrupted_selections > 0 {
            findings.push(count_finding(
                RepairFindingKind::InterruptedSelection,
                inventory.interrupted_selections,
                RepairFix::RemoveStaleStages,
            ));
        }
    }
    if !inventory.retained_stages.is_empty() {
        findings.push(count_finding(
            RepairFindingKind::StagesNeedManualRemoval,
            inventory.retained_stages.len(),
            RepairFix::Manual,
        ));
    }
    if inventory.unexpected_entries > 0 {
        findings.push(count_finding(
            RepairFindingKind::UnexpectedEntries,
            inventory.unexpected_entries,
            RepairFix::Manual,
        ));
    }
    RepairPlan {
        status: if findings.is_empty() {
            RepairStatus::Healthy
        } else {
            RepairStatus::NeedsRepair
        },
        findings,
    }
}

const fn count_finding(kind: RepairFindingKind, count: usize, fix: RepairFix) -> RepairFinding {
    RepairFinding {
        kind,
        component: None,
        version: None,
        count: Some(count),
        fault: None,
        fix,
    }
}

fn diagnose_component(installed: &ComponentInventory, findings: &mut Vec<RepairFinding>) {
    let component = installed.component;
    let selected = installed.selected_version();
    // A verified version to select instead of a broken selection: the
    // recorded previous one first, then the only other verified one.
    let replacement = || -> RepairFix {
        let previous_verifies = match &installed.selection {
            ManagedSelection::Recorded(SelectionRecord {
                previous: Some(previous),
                ..
            }) => matches!(
                installed.version(&previous.version),
                Some(ManagedVersionRecord {
                    state: ManagedVersionState::Verified { manifest_sha256 },
                    ..
                }) if *manifest_sha256 == previous.manifest_sha256
            ),
            _ => false,
        };
        if previous_verifies {
            return RepairFix::RollBack(component);
        }
        let mut others = installed.other_verified();
        match (others.next(), others.next()) {
            (Some(only), None) => RepairFix::RollBackTo(component, only.version.clone()),
            _ => RepairFix::RemoveComponentAndReinstall(component),
        }
    };
    match installed.selection_status() {
        SelectionStatus::None | SelectionStatus::Verified => {}
        SelectionStatus::Unreadable => findings.push(RepairFinding {
            kind: RepairFindingKind::SelectionUnreadable,
            component: Some(component),
            version: None,
            count: None,
            fault: None,
            fix: replacement(),
        }),
        SelectionStatus::Unverified => findings.push(RepairFinding {
            kind: RepairFindingKind::SelectedVersionUnverified,
            component: Some(component),
            version: selected.map(str::to_owned),
            count: None,
            fault: selected
                .and_then(|version| installed.version(version))
                .and_then(|record| record.state.fault()),
            fix: replacement(),
        }),
    }
    for record in &installed.versions {
        if Some(record.version.as_str()) == selected {
            continue;
        }
        let (kind, fault) = match record.state {
            ManagedVersionState::Verified { .. } => continue,
            ManagedVersionState::RemovalInterrupted => {
                (RepairFindingKind::RemovalInterrupted, None)
            }
            ManagedVersionState::Unverified(fault) => {
                (RepairFindingKind::VersionUnverified, Some(fault))
            }
        };
        let fix = if fault.is_none_or(ManagedVersionFault::removable) {
            RepairFix::RemoveVersion(component, record.version.clone())
        } else {
            RepairFix::Manual
        };
        findings.push(RepairFinding {
            kind,
            component: Some(component),
            version: Some(record.version.clone()),
            count: None,
            fault,
            fix,
        });
    }
}

#[cfg(test)]
mod tests;
