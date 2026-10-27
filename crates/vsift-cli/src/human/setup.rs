//! `setup plan`, `setup install`, `setup list`, `setup rollback`, `setup
//! remove`, `setup repair`, `setup configure` and `setup configure-model`
//! (`setup check` has its own view in `setup_check`).

use super::{
    push_outcome,
    text::{RenderedText, TerminalText, TooLarge},
    view::{
        ConfiguredModel, ConfiguredSelection, Envelope, InstallCleanup, InstallComponent,
        ListVersion, PlanAction, PlanModel, PlanStep, SetupInstall, SetupList, SetupPlan,
        SetupRemove, SetupRepair, SetupRollback, StageSweep, VersionOutcome,
    },
};

/// `setup plan`: readiness, what each dependency needs, and every reviewed
/// managed action with its digest.
pub(super) fn plan(envelope: &Envelope<SetupPlan>) -> Result<RenderedText, TooLarge> {
    let plan = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("VSift setup plan (read-only; nothing was changed)")
        .end_line();
    text.push_fixed("Profile: ")
        .push_value(&plan.profile)
        .end_line();
    text.push_fixed("Readiness: ")
        .push_value(&plan.readiness)
        .end_line();
    if let Some(target) = &plan.target {
        text.push_fixed("Target: ").push_value(target).end_line();
    }
    text.push_fixed("Verification: ")
        .push_value(&plan.verification_scope)
        .end_line();
    text.push_fixed("Managed installation: ")
        .push_value(&plan.managed_install)
        .end_line();
    if let Some(revision) = &plan.catalogue_revision {
        text.push_fixed("Catalogue: ").push_value(revision);
        if let Some(stop) = &plan.stop_new_plans_at {
            text.push_fixed(" (no new plans after ")
                .push_value(stop)
                .push_fixed(")");
        }
        text.end_line();
    }
    text.push_fixed("Plan digest: ");
    match &plan.plan_digest {
        Some(digest) => text.push_value(digest),
        None => text.push_fixed("none (nothing to install from this plan)"),
    };
    text.end_line();
    push_plan_optional(&mut text, plan);
    text.blank_line();
    for dependency in &plan.dependencies {
        text.push_value(&dependency.dependency).push_fixed(": ");
        push_step(&mut text, &dependency.step);
    }
    text.push_fixed("Local ASR model: ");
    match &plan.local_asr_model {
        PlanModel::Summary(summary) => {
            text.push_value(summary).end_line();
        }
        PlanModel::Step(step) => push_step(&mut text, step),
    }
    if !plan.actions.is_empty() {
        text.blank_line();
        text.push_fixed("Reviewed managed actions:").end_line();
        if plan.install_needed == Some(false) {
            text.push_fixed(
                "  Every managed component is installed at the plan's version; nothing to \
                 install.",
            )
            .end_line();
        }
    }
    for action in &plan.actions {
        push_plan_action(&mut text, action);
    }
    for optional in &plan.optional_components {
        if !optional.actions.is_empty() {
            text.blank_line();
            text.push_fixed("Reviewed optional actions (")
                .push_value(&optional.name)
                .push_fixed(", installed after the core):")
                .end_line();
        }
        for action in &optional.actions {
            push_plan_action(&mut text, action);
        }
    }
    push_outcome(&mut text, envelope);
    text.finish()
}

/// The optional components a plan offers, includes or cannot offer, shown
/// near the top with the instruction to ask the operator (ADR 0032): opt-in
/// must not mean hidden.
fn push_plan_optional(text: &mut TerminalText, plan: &SetupPlan) {
    if plan.optional_components.is_empty() {
        return;
    }
    text.blank_line();
    text.push_fixed("Optional components:").end_line();
    for optional in &plan.optional_components {
        text.push_fixed("  ")
            .push_value(&optional.summary)
            .end_line();
        if let Some(command) = &optional.add_command {
            text.push_fixed("  To add it: ")
                .push_value(&command.executable);
            for argument in &command.arguments {
                text.push_fixed(" ").push_value(argument);
            }
            text.end_line();
        }
        text.push_fixed("  Next: ")
            .push_value(&optional.next_step)
            .end_line();
    }
    if let Some(instruction) = &plan.operator_decision_required {
        text.push_fixed("Ask the operator: ")
            .push_value(instruction)
            .end_line();
    }
}

/// One reviewed managed action of `setup plan` with its observed state.
fn push_plan_action(text: &mut TerminalText, action: &PlanAction) {
    text.push_fixed("  ")
        .push_value(&action.id)
        .push_fixed(": ")
        .push_value(&action.component)
        .push_fixed(" ")
        .push_value(&action.version)
        .push_fixed(" from ")
        .push_value(&action.publisher);
    if let Some(state) = &action.state {
        text.push_fixed(" [").push_value(state).push_fixed("]");
    }
    text.end_line();
    text.push_fixed("    Download (")
        .push_unsigned(action.bytes)
        .push_fixed(" bytes, sha256 ")
        .push_value(&action.sha256)
        .push_fixed("):")
        .end_line();
    text.push_fixed("    ")
        .push_value(&action.source_url)
        .end_line();
    text.push_fixed("    Licence: ")
        .push_value(&action.licence)
        .end_line();
    text.push_fixed("    Notices:").end_line();
    text.push_fixed("    ")
        .push_value(&action.notice_url)
        .end_line();
    text.push_fixed("    Trust limit: ")
        .push_value(&action.trust_limit)
        .end_line();
    for file in &action.files {
        text.push_fixed("    File ")
            .push_value(&file.name)
            .push_fixed(" (")
            .push_unsigned(file.bytes)
            .push_fixed(" bytes, ")
            .push_value(&file.mode)
            .push_fixed(")")
            .end_line();
    }
}

/// `setup install`: each component of the accepted plan with what happened
/// to it, on a success and (on stdout, before the error on stderr) on a
/// failure. Every value is a closed identifier or a reviewed version.
pub(super) fn install(envelope: &Envelope<SetupInstall>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("VSift setup install").end_line();
    if let Some(revision) = &data.catalogue_revision {
        text.push_fixed("Catalogue: ")
            .push_value(revision)
            .end_line();
    }
    text.push_fixed("Source: ")
        .push_value(&data.source)
        .end_line();
    if data.components.is_empty() {
        text.push_fixed("No component needed installing.")
            .end_line();
    }
    for component in &data.components {
        push_component(&mut text, component);
    }
    if let Some(core) = &data.core {
        text.push_fixed("Core: ").push_value(core).end_line();
    }
    for component in &data.optional_components {
        push_component(&mut text, component);
    }
    for group in &data.optional_groups {
        text.push_fixed("Optional component ")
            .push_value(&group.name)
            .push_fixed(": ")
            .push_value(&group.status)
            .end_line();
        text.push_fixed("  ")
            .push_value(&group.next_step)
            .end_line();
    }
    push_install_cleanup(&mut text, &data.cleanup);
    text.push_fixed("Next: ")
        .push_value(&data.next_step)
        .end_line();
    push_outcome(&mut text, envelope);
    text.finish()
}

fn push_component(text: &mut TerminalText, component: &InstallComponent) {
    text.push_fixed("[")
        .push_value(&component.status)
        .push_fixed("] ")
        .push_value(&component.component)
        .push_fixed(" ")
        .push_value(&component.version);
    if let Some(reason) = &component.reason {
        text.push_fixed(": ");
        if let Some(step) = &component.step {
            text.push_value(step).push_fixed(" ");
        }
        if let Some(check) = &component.smoke_check {
            text.push_fixed("(")
                .push_value(check)
                .push_fixed(" check) ");
        }
        text.push_value(reason);
        if let Some(library) = &component.missing_shared_library {
            text.push_fixed(" (missing shared library ")
                .push_value(library)
                .push_fixed(")");
        }
        if let Some(code) = &component.failure_code {
            text.push_fixed(", ").push_value(code);
        }
    }
    if let Some(stage) = &component.stage
        && component.status != "activated"
    {
        text.push_fixed("; stage ").push_value(stage);
        if let Some(reason) = &component.retention_reason {
            text.push_fixed(" (").push_value(reason).push_fixed(")");
        }
    }
    text.end_line();
}

/// What the install's sweep and bounded cleanup did, when they did anything.
fn push_install_cleanup(text: &mut TerminalText, cleanup: &InstallCleanup) {
    if cleanup.stale_stages_removed > 0 || cleanup.stale_stages_retained > 0 {
        text.push_fixed("Abandoned stages: removed ")
            .push_unsigned(cleanup.stale_stages_removed)
            .push_fixed(", kept ")
            .push_unsigned(cleanup.stale_stages_retained)
            .end_line();
    }
    for version in &cleanup.versions {
        text.push_fixed("Cleanup: ");
        push_version_outcome(text, version);
    }
}

fn push_version_outcome(text: &mut TerminalText, version: &VersionOutcome) {
    text.push_fixed("[")
        .push_value(&version.status)
        .push_fixed("] ")
        .push_value(&version.component)
        .push_fixed(" ")
        .push_value(&version.version)
        .end_line();
}

/// `setup list`: each component's selection and versions.
pub(super) fn list(envelope: &Envelope<SetupList>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("VSift setup list").end_line();
    text.push_fixed("Managed installation: ")
        .push_value(&data.managed_install)
        .end_line();
    text.push_fixed("Managed folder: ")
        .push_value(&data.managed_folder)
        .end_line();
    for component in &data.components {
        text.push_value(&component.component).push_fixed(": ");
        if component.versions.is_empty() && component.selection == "none" {
            text.push_fixed("nothing installed").end_line();
            continue;
        }
        text.push_fixed("selection ")
            .push_value(&component.selection)
            .end_line();
        for version in &component.versions {
            push_list_version(&mut text, version);
        }
    }
    if data.stale_stages > 0 || data.retained_stages > 0 {
        text.push_fixed("Abandoned stages: ")
            .push_unsigned(data.stale_stages)
            .push_fixed(" removable, ")
            .push_unsigned(data.retained_stages)
            .push_fixed(" to remove yourself")
            .end_line();
    }
    text.push_fixed("Next: ")
        .push_value(&data.next_step)
        .end_line();
    push_outcome(&mut text, envelope);
    text.finish()
}

fn push_list_version(text: &mut TerminalText, version: &ListVersion) {
    text.push_fixed("  [")
        .push_value(&version.state)
        .push_fixed("] ")
        .push_value(&version.version);
    if let Some(fault) = &version.fault {
        text.push_fixed(" (").push_value(fault).push_fixed(")");
    }
    if version.selected {
        text.push_fixed(", selected");
    }
    if version.previous {
        text.push_fixed(", previous");
    }
    text.end_line();
}

/// `setup rollback`: what is selected now and what it replaced.
pub(super) fn rollback(envelope: &Envelope<SetupRollback>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("VSift setup rollback").end_line();
    text.push_value(&data.component)
        .push_fixed(": ")
        .push_value(&data.status)
        .push_fixed(", selected ")
        .push_value(&data.selected_version);
    if let Some(replaced) = &data.replaced_version {
        text.push_fixed(" (replaced ")
            .push_value(replaced)
            .push_fixed(")");
    }
    text.end_line();
    text.push_fixed("Next: ")
        .push_value(&data.next_step)
        .end_line();
    push_outcome(&mut text, envelope);
    text.finish()
}

/// `setup remove`: each version handled and what the sweep did.
pub(super) fn remove(envelope: &Envelope<SetupRemove>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("VSift setup remove").end_line();
    text.push_fixed("Target: ").push_value(&data.target);
    if let Some(component) = &data.component {
        text.push_fixed(" ").push_value(component);
    }
    text.end_line();
    if data.deselected {
        text.push_fixed("Selection removed: commands no longer use a managed version of it.")
            .end_line();
    }
    if data.versions.is_empty() && data.stages.is_none() {
        text.push_fixed("Nothing was installed to remove.")
            .end_line();
    }
    for version in &data.versions {
        push_version_outcome(&mut text, version);
    }
    if let Some(stages) = &data.stages {
        push_stage_sweep(&mut text, stages);
    }
    text.push_fixed("Next: ")
        .push_value(&data.next_step)
        .end_line();
    push_outcome(&mut text, envelope);
    text.finish()
}

fn push_stage_sweep(text: &mut TerminalText, stages: &StageSweep) {
    text.push_fixed("Abandoned stages: removed ")
        .push_unsigned(stages.removed)
        .push_fixed(", kept ")
        .push_unsigned(stages.retained);
    for (index, reason) in stages.retention_reasons.iter().enumerate() {
        text.push_fixed(if index == 0 { " (" } else { ", " })
            .push_value(reason);
    }
    if !stages.retention_reasons.is_empty() {
        text.push_fixed(")");
    }
    text.end_line();
    text.push_fixed("Half-written selection pointers removed: ")
        .push_unsigned(stages.interrupted_selections_removed)
        .end_line();
}

/// `setup repair`: the diagnosis and, for each finding, its fixed prose and
/// the command that fixes it.
pub(super) fn repair(envelope: &Envelope<SetupRepair>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("VSift setup repair (changes nothing)")
        .end_line();
    text.push_fixed("Managed installation: ")
        .push_value(&data.managed_install)
        .end_line();
    text.push_fixed("Diagnosis: ")
        .push_value(&data.status)
        .end_line();
    for finding in &data.findings {
        text.push_fixed("[")
            .push_value(&finding.kind)
            .push_fixed("] ")
            .push_value(&finding.summary)
            .end_line();
        if let Some(command) = &finding.command {
            text.push_fixed("  Run: ").push_value(&command.executable);
            for argument in &command.arguments {
                text.push_fixed(" ").push_value(argument);
            }
            text.end_line();
        }
    }
    text.push_fixed("Next: ")
        .push_value(&data.next_step)
        .end_line();
    push_outcome(&mut text, envelope);
    text.finish()
}

/// Writes a planned step: status, disposition, authority, then its next
/// step on a line of its own.
fn push_step(text: &mut TerminalText, step: &PlanStep) {
    text.push_value(&step.status)
        .push_fixed(", ")
        .push_value(&step.disposition);
    if let Some(authority) = &step.required_authority {
        text.push_fixed(" (needs ")
            .push_value(authority)
            .push_fixed(" authority)");
    }
    text.end_line();
    text.push_fixed("  Next: ")
        .push_value(&step.next_step)
        .end_line();
}

/// `setup configure`: the registered executable's dependency.
pub(super) fn configure(
    envelope: &Envelope<ConfiguredSelection>,
) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("Registered ")
        .push_value(&data.dependency)
        .push_fixed(" as a ")
        .push_value(&data.source)
        .push_fixed(" (")
        .push_value(&data.validation)
        .push_fixed(")")
        .end_line();
    text.push_fixed("Next: ")
        .push_value(&data.next_step)
        .end_line();
    push_outcome(&mut text, envelope);
    text.finish()
}

/// `setup configure-model`: the registered model file.
pub(super) fn configure_model(
    envelope: &Envelope<ConfiguredModel>,
) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("Registered the local ASR model as a ")
        .push_value(&data.source)
        .push_fixed(" (")
        .push_value(&data.validation)
        .push_fixed(")")
        .end_line();
    text.push_fixed("Next: ")
        .push_value(&data.next_step)
        .end_line();
    push_outcome(&mut text, envelope);
    text.finish()
}
