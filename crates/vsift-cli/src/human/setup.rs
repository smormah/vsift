//! `setup check`, `setup plan`, `setup configure` and
//! `setup configure-model`.

use vsift::{
    DependencyState, LocalAsrCheckOutcome, LocalAsrModelStatus, LocalAsrNotRunReason,
    LocalAsrSetupStatus, LocalAsrVerificationSource, ManagedPlanAvailability, RuntimeDependency,
    RuntimeDiagnosis, RuntimeReadiness,
};
use vsift_contract::{
    DependencyLookup, MAX_PROVIDER_DETAIL_BYTES, explicit_path_option, sanitize_untrusted_text,
};

use super::{
    push_outcome,
    text::{DisplayText, Placement, RenderedText, TerminalText, TooLarge},
    view::{
        ConfiguredModel, ConfiguredSelection, Envelope, InstallComponent, PlanAction, PlanModel,
        PlanStep, SetupInstall, SetupPlan,
    },
};
use crate::command::ExecutionProfile;

/// `setup check`, rendered from its typed report: the probe of each
/// dependency, where its executable came from, and the local-ASR check.
///
/// A probed version line is provider text, so it is shown in its display
/// form, bounded to the provider-detail budget.
pub(crate) fn setup_check<L>(
    diagnosis: &RuntimeDiagnosis,
    local_asr: LocalAsrSetupStatus,
    managed_install: ManagedPlanAvailability,
    profile: ExecutionProfile,
    lookup: &L,
) -> Result<RenderedText, TooLarge>
where
    L: Fn(RuntimeDependency) -> DependencyLookup,
{
    let mut text = TerminalText::result();
    text.push_fixed("VSift setup check").end_line();
    text.push_fixed("Profile: ")
        .push_value(profile.identifier())
        .end_line();
    text.push_fixed("Status: ")
        .push_value(diagnosis.readiness.identifier())
        .end_line();
    for status in &diagnosis.dependencies {
        let provenance = lookup(status.dependency);
        text.push_fixed("[")
            .push_fixed(state_marker(&status.state))
            .push_fixed("] ")
            .push_value(status.dependency.display_name())
            .push_fixed(" (")
            .push_value(status.dependency.capability().identifier())
            .push_fixed("): ");
        match &status.state {
            DependencyState::Available { version } => {
                let bounded = sanitize_untrusted_text(version, MAX_PROVIDER_DETAIL_BYTES);
                text.push_untrusted(&DisplayText::render(&bounded), Placement::Inline);
            }
            DependencyState::Missing => {
                text.push_fixed(if provenance == DependencyLookup::FilteredPath {
                    "not found on PATH"
                } else {
                    "explicit path not found"
                });
            }
            DependencyState::Unhealthy { .. } => {
                text.push_fixed("dependency probe failed");
            }
            DependencyState::TimedOut => {
                text.push_fixed("probe exceeded its deadline");
            }
        }
        text.push_fixed(match provenance {
            DependencyLookup::ExplicitPath => " [per-call path]",
            DependencyLookup::ConfiguredUserPath => " [configured user path]",
            DependencyLookup::ManagedVersion => " [managed version]",
            DependencyLookup::FilteredPath => " [filtered PATH]",
        })
        .end_line();
        if !status.state.is_available() {
            text.push_fixed(
                "  Install or locate this trusted tool, then rerun setup check with its absolute \
                 path using ",
            )
            .push_value(explicit_path_option(status.dependency))
            .push_fixed(if managed_install == ManagedPlanAvailability::Qualified {
                ". Or let VSift install the reviewed build: review setup plan, then accept it \
                 with setup install."
            } else {
                ". Managed installation is not available for this target."
            })
            .end_line();
        }
    }
    if diagnosis.readiness == RuntimeReadiness::Blocked {
        text.push_fixed("Media executable probes are blocked until FFmpeg and FFprobe respond.")
            .end_line();
    }
    push_local_asr(&mut text, local_asr);
    text.push_fixed(
        "Executable probes only show that each tool responds. The local ASR check transcribes a \
         short speech clip built into VSift with the selected tools and model. A supplied \
         transcript can avoid local ASR.",
    )
    .end_line();
    text.finish()
}

const fn state_marker(state: &DependencyState) -> &'static str {
    match state {
        DependencyState::Available { .. } => "ok",
        DependencyState::Missing => "missing",
        DependencyState::Unhealthy { .. } => "unhealthy",
        DependencyState::TimedOut => "timeout",
    }
}

/// The local-ASR lines: every value is a typed identifier, never a path or
/// tool output.
fn push_local_asr(text: &mut TerminalText, local_asr: LocalAsrSetupStatus) {
    text.push_fixed("Local ASR model: ");
    match local_asr.model {
        LocalAsrModelStatus::NotSelected => {
            text.push_fixed("not registered (setup configure-model --file <path>)");
        }
        LocalAsrModelStatus::Unreadable => {
            text.push_fixed("registered file cannot be read");
        }
        LocalAsrModelStatus::Unrecognised => {
            text.push_fixed("registered file is not a reviewed model, so it will not run");
        }
        LocalAsrModelStatus::KnownPinned(profile) => {
            text.push_fixed("reviewed ")
                .push_value(profile.identifier())
                .push_fixed(" profile");
        }
    }
    text.end_line();
    text.push_fixed("Local ASR check: ");
    match local_asr.verification {
        LocalAsrCheckOutcome::Verified(LocalAsrVerificationSource::Recorded) => {
            text.push_fixed("verified (recorded pass)");
        }
        LocalAsrCheckOutcome::Verified(LocalAsrVerificationSource::RanNow) => {
            text.push_fixed("verified (ran now)");
        }
        LocalAsrCheckOutcome::Failed(failure) => {
            text.push_fixed("failed at the ")
                .push_value(failure.check())
                .push_fixed(" step (")
                .push_value(failure.reason())
                .push_fixed("); local ASR will not run until it passes");
        }
        LocalAsrCheckOutcome::NotRun(reason) => {
            text.push_fixed("not run: ").push_fixed(match reason {
                LocalAsrNotRunReason::MediaToolsUnavailable => {
                    "FFmpeg and FFprobe are needed and must pass their own check"
                }
                LocalAsrNotRunReason::WhisperUnavailable => "the whisper.cpp CLI is not available",
                LocalAsrNotRunReason::ModelNotSelected => "no model is registered",
                LocalAsrNotRunReason::ModelNotPinned => {
                    "the registered model is not a reviewed pinned profile"
                }
            });
        }
    }
    text.end_line();
}

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
    push_outcome(&mut text, envelope);
    text.finish()
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
