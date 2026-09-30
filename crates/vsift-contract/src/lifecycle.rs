//! `setup list`, `setup rollback`, `setup remove` and `setup repair` data
//! and remediation (P13 PR 6, ADR 0023 §3 steps 5 and 6).
//!
//! Every value is a closed identifier, a count, or a managed component or
//! version key (canonical: lowercase ASCII letters, digits, `.`, `_` and `-`,
//! at most 64 bytes); nothing here carries a path, provider output or text a
//! user typed. `setup repair`'s findings each name the existing command that
//! fixes them as an executable and argument array of fixed words and those
//! keys; repair itself changes nothing.

use serde::Serialize;
use vsift_application::{
    ManagedInventory, ManagedLifecycleRefusal, ManagedPlanAvailability, ManagedRemovalTarget,
    ManagedVersionFault, RemovalReport, RepairFinding, RepairFindingKind, RepairFix, RepairPlan,
    RepairStatus, RollbackOutcome, StageSweep, VersionRemovalReport, diagnose_managed_store,
};
use vsift_domain::{FailureCode, ManagedComponent};

/// A `vsift` command a finding suggests, run without a shell.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SuggestedCommandResponse {
    executable: &'static str,
    arguments: Vec<String>,
}

impl SuggestedCommandResponse {
    fn vsift(arguments: &[&str]) -> Self {
        Self {
            executable: "vsift",
            arguments: arguments
                .iter()
                .map(|argument| (*argument).to_owned())
                .collect(),
        }
    }

    /// The argument array, after the executable.
    #[must_use]
    pub fn arguments(&self) -> Vec<&str> {
        self.arguments.iter().map(String::as_str).collect()
    }
}

/// Data of a `setup list` result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SetupListResponse {
    managed_install: &'static str,
    managed_folder: &'static str,
    components: Vec<ListComponentResponse>,
    stale_stages: usize,
    retained_stages: usize,
    next_step: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ListComponentResponse {
    component: &'static str,
    selection: &'static str,
    selected_version: Option<String>,
    previous_version: Option<String>,
    versions: Vec<ListVersionResponse>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ListVersionResponse {
    version: String,
    selected: bool,
    previous: bool,
    state: &'static str,
    fault: Option<&'static str>,
}

impl SetupListResponse {
    /// Presents what the store holds (`None`: no managed folder) and
    /// whether managed installation is available on this host.
    #[must_use]
    pub fn new(
        availability: ManagedPlanAvailability,
        inventory: Option<&ManagedInventory>,
    ) -> Self {
        let empty = ManagedInventory::empty();
        let shown = inventory.unwrap_or(&empty);
        let next_step = match (inventory, diagnose_managed_store(inventory).status) {
            (None, _) | (_, RepairStatus::NothingInstalled) => {
                "Nothing is managed for this user. Where managed installation is available, run setup plan, review it, then setup install; elsewhere install the tools yourself and register them with setup configure."
            }
            (Some(_), RepairStatus::Healthy) => {
                "Commands use each component's selected version while it verifies. setup rollback <component> returns to its previous version; setup remove removes a version or a component."
            }
            (Some(_), RepairStatus::NeedsRepair) => {
                "Something in the managed folder needs attention. Run setup repair: it changes nothing and names the command that fixes each problem."
            }
        };
        Self {
            managed_install: availability.identifier(),
            managed_folder: if inventory.is_some() {
                "present"
            } else {
                "absent"
            },
            components: shown
                .components
                .iter()
                .map(|installed| {
                    let selected = installed.selected_version();
                    let previous = installed.previous_version();
                    ListComponentResponse {
                        component: installed.component.identifier(),
                        selection: installed.selection_status().identifier(),
                        selected_version: selected.map(str::to_owned),
                        previous_version: previous.map(str::to_owned),
                        versions: installed
                            .versions
                            .iter()
                            .map(|record| ListVersionResponse {
                                version: record.version.clone(),
                                selected: Some(record.version.as_str()) == selected,
                                previous: Some(record.version.as_str()) == previous,
                                state: record.state.identifier(),
                                fault: record.state.fault().map(ManagedVersionFault::identifier),
                            })
                            .collect(),
                    }
                })
                .collect(),
            stale_stages: shown.stale_stages,
            retained_stages: shown.retained_stages.len(),
            next_step,
        }
    }
}

/// Data of a `setup rollback` result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SetupRollbackResponse {
    component: &'static str,
    status: &'static str,
    selected_version: String,
    replaced_version: Option<String>,
    next_step: &'static str,
}

impl SetupRollbackResponse {
    /// Presents one rollback.
    #[must_use]
    pub fn new(outcome: &RollbackOutcome) -> Self {
        Self {
            component: outcome.component.identifier(),
            status: if outcome.changed {
                "rolled_back"
            } else {
                "already_selected"
            },
            selected_version: outcome.selected.clone(),
            replaced_version: outcome.replaced.clone(),
            next_step: if outcome.changed {
                "Commands started from now on use the selected version; a running job keeps the version it started with. Run setup rollback again to return to the version it replaced."
            } else {
                "That version was already selected and verifies; nothing was changed."
            },
        }
    }
}

/// Data of a `setup remove` result, successful or failed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SetupRemoveResponse {
    target: &'static str,
    component: Option<&'static str>,
    deselected: bool,
    versions: Vec<VersionOutcomeResponse>,
    stages: Option<StageSweepResponse>,
    next_step: &'static str,
}

/// One version a removal or cleanup handled.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct VersionOutcomeResponse {
    component: &'static str,
    version: String,
    status: &'static str,
}

/// What a stale-stage sweep did.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct StageSweepResponse {
    removed: usize,
    retained: usize,
    retention_reasons: Vec<&'static str>,
    interrupted_selections_removed: usize,
}

fn version_outcomes(reports: &[VersionRemovalReport]) -> Vec<VersionOutcomeResponse> {
    reports
        .iter()
        .map(|report| VersionOutcomeResponse {
            component: report.component.identifier(),
            version: report.version.clone(),
            status: report.status.identifier(),
        })
        .collect()
}

fn stage_sweep(sweep: &StageSweep) -> StageSweepResponse {
    let mut retention_reasons: Vec<&'static str> = sweep
        .retained
        .iter()
        .map(|reason| reason.identifier())
        .collect();
    retention_reasons.sort_unstable();
    retention_reasons.dedup();
    StageSweepResponse {
        removed: sweep.removed,
        retained: sweep.retained.len(),
        retention_reasons,
        interrupted_selections_removed: sweep.interrupted_selections_removed,
    }
}

impl SetupRemoveResponse {
    /// Presents one removal of `target`.
    #[must_use]
    pub fn new(target: &ManagedRemovalTarget, report: &RemovalReport) -> Self {
        let (target_identifier, component) = match target {
            ManagedRemovalTarget::Version { component, .. } => ("version", Some(*component)),
            ManagedRemovalTarget::Component(component) => ("component", Some(*component)),
            ManagedRemovalTarget::StaleStages => ("stale_stages", None),
        };
        Self {
            target: target_identifier,
            component: component.map(ManagedComponent::identifier),
            deselected: report.deselected.is_some(),
            versions: version_outcomes(&report.versions),
            stages: report.stages.as_ref().map(stage_sweep),
            next_step: match report.failure_code() {
                None => "Everything asked for is removed. Run setup list to see what is installed.",
                Some(FailureCode::Busy) => {
                    "A running job uses a version reported in_use, so it was kept; everything else asked for is removed. Run the same setup remove again once the job has ended."
                }
                Some(_) => {
                    "Some content was kept because VSift could not prove it its own or could not remove it. Run setup repair to see what remains and how to remove it."
                }
            },
        }
    }
}

/// Fixed-prose remediation for a removal that could not remove everything.
#[must_use]
pub const fn setup_remove_failure_summary(code: FailureCode) -> &'static str {
    match code {
        FailureCode::Busy => {
            "A running job uses a managed version, so VSift kept it; it never removes a version in use. Run the same setup remove again once the job has ended."
        }
        FailureCode::InvalidArgument => {
            "The version is the selected one, so it was kept. Roll back to another version first, or remove the whole component with setup remove <component>."
        }
        _ => {
            "VSift kept content it could not prove its own (an invalid manifest, a link, a folder or an unknown name) or could not remove. Run setup repair; it names what to delete yourself. Nothing outside the managed folder was touched."
        }
    }
}

/// Data of a `setup repair` result: a diagnosis and the plan that fixes it.
/// Repair changes nothing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SetupRepairResponse {
    managed_install: &'static str,
    status: &'static str,
    findings: Vec<RepairFindingResponse>,
    next_step: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct RepairFindingResponse {
    kind: &'static str,
    component: Option<&'static str>,
    version: Option<String>,
    count: Option<usize>,
    fault: Option<&'static str>,
    fix: &'static str,
    summary: String,
    command: Option<SuggestedCommandResponse>,
}

impl SetupRepairResponse {
    /// Presents one diagnosis.
    #[must_use]
    pub fn new(availability: ManagedPlanAvailability, plan: &RepairPlan) -> Self {
        Self {
            managed_install: availability.identifier(),
            status: plan.status.identifier(),
            findings: plan.findings.iter().map(finding_response).collect(),
            next_step: match plan.status {
                RepairStatus::NothingInstalled => {
                    "Nothing is managed for this user, so there is nothing to repair."
                }
                RepairStatus::Healthy => "Nothing needs repairing.",
                RepairStatus::NeedsRepair => {
                    "Nothing was changed. Run each finding's command, in order, then run setup repair again; a finding without a command needs you to delete what it names yourself. Nothing here downloads: a reinstall is a new setup plan accepted with setup install."
                }
            },
        }
    }
}

/// The `vsift` command that applies `fix`, if `VSift` can apply it.
#[must_use]
pub fn repair_fix_command(fix: &RepairFix) -> Option<SuggestedCommandResponse> {
    Some(match fix {
        RepairFix::RollBack(component) => {
            SuggestedCommandResponse::vsift(&["setup", "rollback", component.identifier()])
        }
        RepairFix::RollBackTo(component, version) => SuggestedCommandResponse::vsift(&[
            "setup",
            "rollback",
            component.identifier(),
            "--version",
            version,
        ]),
        RepairFix::RemoveVersion(component, version) => SuggestedCommandResponse::vsift(&[
            "setup",
            "remove",
            component.identifier(),
            "--version",
            version,
        ]),
        RepairFix::RemoveComponentAndReinstall(component) => {
            SuggestedCommandResponse::vsift(&["setup", "remove", component.identifier()])
        }
        RepairFix::RemoveStaleStages => {
            SuggestedCommandResponse::vsift(&["setup", "remove", "--stale-stages"])
        }
        RepairFix::Manual => return None,
    })
}

const MANUAL_REMOVAL: &str = "VSift does not remove what it cannot prove its own. Delete it yourself, or delete the whole managed folder (on Ubuntu by default ~/.local/share/vsift/managed-v1, or vsift/managed-v1 under $XDG_DATA_HOME) and run setup plan and setup install again.";

fn finding_summary(finding: &RepairFinding) -> String {
    let component = finding
        .component
        .map_or("", |component| component.identifier());
    let version = finding.version.as_deref().unwrap_or("");
    let count = finding.count.unwrap_or(0);
    let fix = match &finding.fix {
        RepairFix::RollBack(_) => {
            " Roll back to the verified version selected before it.".to_owned()
        }
        RepairFix::RollBackTo(_, version) => {
            format!(" Select the verified version {version} instead.")
        }
        RepairFix::RemoveVersion(..) => " Remove that version.".to_owned(),
        RepairFix::RemoveComponentAndReinstall(_) => {
            " No verified version is left to select: remove the component, then run setup plan and setup install again (or install the tool yourself and register it with setup configure).".to_owned()
        }
        RepairFix::RemoveStaleStages => " Remove them with setup remove --stale-stages.".to_owned(),
        RepairFix::Manual => format!(" {MANUAL_REMOVAL}"),
    };
    let problem = match finding.kind {
        RepairFindingKind::SelectionUnreadable => format!(
            "The selection of {component} cannot be read, so commands do not use a managed {component}."
        ),
        RepairFindingKind::SelectedVersionUnverified => format!(
            "The selected {component} version {version} is missing or does not verify against its manifest, so commands do not use it."
        ),
        RepairFindingKind::VersionUnverified => format!(
            "The {component} version {version} does not verify against its manifest ({}).",
            finding
                .fault
                .map_or("unverified", |fault| fault.identifier())
        ),
        RepairFindingKind::RemovalInterrupted => {
            format!("The removal of the {component} version {version} was interrupted.")
        }
        RepairFindingKind::StaleStages => format!(
            "Abandoned stages from an interrupted or failed installation: {count}. They only hold disk space."
        ),
        RepairFindingKind::StagesNeedManualRemoval => format!(
            "Stages that hold a link, a folder or an unknown name, or that nothing proves VSift's own: {count}."
        ),
        RepairFindingKind::InterruptedSelection => format!(
            "Half-written selection pointers from an interrupted selection: {count}. Commands ignore them."
        ),
        RepairFindingKind::UnexpectedEntries => format!(
            "Entries of the managed folder that VSift did not make: {count}. Commands ignore them."
        ),
    };
    problem + &fix
}

fn finding_response(finding: &RepairFinding) -> RepairFindingResponse {
    RepairFindingResponse {
        kind: finding.kind.identifier(),
        component: finding.component.map(ManagedComponent::identifier),
        version: finding.version.clone(),
        count: finding.count,
        fault: finding.fault.map(ManagedVersionFault::identifier),
        fix: finding.fix.identifier(),
        summary: finding_summary(finding),
        command: repair_fix_command(&finding.fix),
    }
}

/// Fixed-prose remediation for a lifecycle command that changed nothing,
/// and the `vsift` arguments it suggests.
#[must_use]
pub const fn managed_lifecycle_remediation(
    refusal: ManagedLifecycleRefusal,
) -> (&'static str, &'static [&'static str]) {
    match refusal {
        ManagedLifecycleRefusal::NotInstalled => (
            "No managed version of that component is installed, so nothing was changed. Run setup list to see what is installed.",
            &["setup", "list"],
        ),
        ManagedLifecycleRefusal::VersionNotInstalled => (
            "That version of the component is not installed, so nothing was changed. Run setup list to see the installed versions.",
            &["setup", "list"],
        ),
        ManagedLifecycleRefusal::NoPreviousVersion => (
            "No earlier version of that component is recorded and still installed, so there is nothing to roll back to and nothing was changed. Run setup list, then setup rollback <component> --version <version> to select another installed version.",
            &["setup", "list"],
        ),
        ManagedLifecycleRefusal::VersionUnverified => (
            "The version to select does not verify against its manifest, or has none, so it was not selected and the selection is unchanged. Run setup repair for the fix.",
            &["setup", "repair"],
        ),
        ManagedLifecycleRefusal::VersionSelected => (
            "That version is the selected one, so it was not removed and nothing was changed. Roll back to another version first, or remove the whole component with setup remove <component>.",
            &["setup", "list"],
        ),
        ManagedLifecycleRefusal::StoreUnsafe => (
            "VSift could not prove its managed folder private and its own (a link, another owner, open permissions or a changed marker), so it changed nothing. Delete the managed folder yourself (on Ubuntu by default ~/.local/share/vsift/managed-v1, or vsift/managed-v1 under $XDG_DATA_HOME) and install again, or install the tools yourself and register them with setup configure.",
            &[],
        ),
        ManagedLifecycleRefusal::StoreIo => (
            "VSift could not read or change its managed folder, so nothing was changed. Check the disk and the folder's permissions, then run the same command again.",
            &[],
        ),
    }
}

/// Data of the cleanup `setup install` runs around its transaction: the
/// stale-stage sweep before it and the bounded version cleanup after it.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct InstallCleanupResponse {
    stale_stages_removed: usize,
    stale_stages_retained: usize,
    versions: Vec<VersionOutcomeResponse>,
}

impl InstallCleanupResponse {
    /// Presents the sweep and the version cleanup.
    #[must_use]
    pub fn new(stages: &StageSweep, versions: &[VersionRemovalReport]) -> Self {
        Self {
            stale_stages_removed: stages.removed,
            stale_stages_retained: stages.retained.len(),
            versions: version_outcomes(versions),
        }
    }
}
