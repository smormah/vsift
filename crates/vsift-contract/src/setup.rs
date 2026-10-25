//! Setup check, setup plan and dependency-registration responses.

use std::time::Duration;

use crate::optional_component::{
    OptionalCheckResponse, OptionalComponentText, include_command_arguments,
    optional_component_text,
};
use serde::{Deserialize, Serialize};
use vsift_application::{
    LocalAsrCheckOutcome, LocalAsrSetupStatus, ManagedPlanAvailability, ManagedPlanObservation,
    ManagedSetupAction, ManagedSetupPlan, ManagedUpdate, ObservedModel, OptionalCapability,
    OptionalComponent, OptionalComponentDisposition, OptionalComponentPlan, PlannedSupply,
    RuntimeDiagnosis, SetupDependencyDisposition, SetupModelDisposition, SetupProfile,
    SupersessionReason,
};
use vsift_domain::ManagedComponent;
use vsift_domain::{
    DependencyState, DependencyStatus, FailureCode, ProbeTimeout, ProbeTimeoutKind,
    ReviewedAsrModel, RuntimeDependency,
};

use crate::{
    command::CommandName,
    envelope::CONTRACT_VERSION,
    text::{MAX_PROVIDER_DETAIL_BYTES, sanitize_untrusted_text},
};

/// Where the executable probed for one dependency came from.
///
/// The engine resolves the precedence (per-call path, then configured user
/// path, then the managed version `setup install` selected, then filtered
/// `PATH`); the contract only fixes how that provenance is reported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyLookup {
    /// An absolute path supplied for this one invocation.
    ExplicitPath,
    /// A path previously saved with `setup configure`.
    ConfiguredUserPath,
    /// The version `setup install` selected in the private managed root,
    /// opened only after every file matched its manifest's SHA-256 (P13).
    ManagedVersion,
    /// A search of the filtered `PATH`.
    FilteredPath,
}

impl DependencyLookup {
    /// Returns the stable machine-readable lookup identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::ExplicitPath => "explicit_path",
            Self::ConfiguredUserPath => "configured_user_path",
            Self::ManagedVersion => "managed_version",
            Self::FilteredPath => "filtered_path",
        }
    }
}

/// Backward-compatible v1 `setup check` response.
///
/// Unlike newer commands this is not wrapped in the operation envelope when
/// written as plain JSON; the P01 shape is preserved for existing consumers.
/// `verification_scope` and `local_asr_model` keep their original constant
/// values for v1 compatibility (maintainer decision D4); they describe the
/// executable probes only. The additive `local_asr` object carries the
/// model identity and the local-ASR functional verification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SetupCheckResponse {
    schema_version: &'static str,
    command: &'static str,
    profile: &'static str,
    status: &'static str,
    verification_scope: &'static str,
    local_asr_model: &'static str,
    dependencies: Vec<SetupCheckDependencyResponse>,
    local_asr: LocalAsrSetupResponse,
    managed_updates: Vec<ManagedUpdateResponse>,
    optional_components: Vec<OptionalCheckResponse>,
}

/// One entry of `managed_updates` (since `0.2.3`): a managed version this
/// check used that the current catalogue supersedes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ManagedUpdateResponse {
    component: &'static str,
    selected_version: String,
    reviewed_version: String,
    reason: &'static str,
    next_step: String,
}

impl ManagedUpdateResponse {
    fn new(update: &ManagedUpdate) -> Self {
        Self {
            component: update.component.identifier(),
            selected_version: update.selected_version.clone(),
            reviewed_version: update.reviewed_version.clone(),
            reason: update.reason.identifier(),
            next_step: managed_update_next_step(update),
        }
    }
}

/// What to do about a superseded managed version, for the JSON `next_step`
/// and the human text alike.
///
/// The step is the ordinary managed install: the current plan proposes the
/// new pin because the selected version is not the catalogue's, and
/// accepting it installs and selects that pin; the earlier version stays
/// installed, and `setup rollback` can select it again. Versions are
/// canonical managed keys of at most 64 bytes, so the text stays well under
/// the 240-byte bound of a `next_step`.
#[must_use]
pub fn managed_update_next_step(update: &ManagedUpdate) -> String {
    let why = match update.reason {
        SupersessionReason::SecurityFixes => {
            ", which carries security fixes the selected one lacks"
        }
        SupersessionReason::Maintenance => "",
    };
    format!(
        "The reviewed build is now {}{why}: review setup plan, then accept it with setup \
         install, which installs and selects it.",
        update.reviewed_version
    )
}

impl SetupCheckResponse {
    /// Creates the compatible setup response for the explicitly resolved profile.
    ///
    /// `lookup` reports, for each probed dependency, where its executable came
    /// from; `managed_install` is whether managed installation can supply a
    /// missing dependency on this host (the availability `setup plan`
    /// reports); `local_asr` is the model and verification report.
    #[must_use]
    pub fn new<F>(
        diagnosis: &RuntimeDiagnosis,
        profile: SetupProfile,
        lookup: F,
        managed_install: ManagedPlanAvailability,
        local_asr: &LocalAsrSetupStatus,
    ) -> Self
    where
        F: Fn(RuntimeDependency) -> DependencyLookup,
    {
        Self {
            schema_version: CONTRACT_VERSION,
            command: CommandName::SetupCheck.identifier(),
            profile: profile.identifier(),
            status: diagnosis.readiness.identifier(),
            verification_scope: "executable_probe_only",
            local_asr_model: "not_checked",
            dependencies: diagnosis
                .dependencies
                .iter()
                .map(|status| {
                    SetupCheckDependencyResponse::new(
                        status,
                        lookup(status.dependency),
                        managed_install,
                    )
                })
                .collect(),
            local_asr: LocalAsrSetupResponse::new(*local_asr),
            managed_updates: Vec::new(),
            optional_components: Vec::new(),
        }
    }

    /// Sets, per dependency, whether managed installation can supply it here
    /// (ADR 0032): a target's catalogue may offer only some core components,
    /// so the host's availability is `unavailable_target` for a dependency
    /// whose component it does not offer.
    #[must_use]
    pub fn with_managed_install_for<F>(mut self, availability: F) -> Self
    where
        F: Fn(RuntimeDependency) -> ManagedPlanAvailability,
    {
        for dependency in &mut self.dependencies {
            if let Some(remediation) = dependency.remediation.as_mut() {
                remediation.managed_install =
                    availability(dependency.runtime_dependency).identifier();
            }
        }
        self
    }

    /// Sets what the check found about each optional component (the
    /// additive `optional_components` array, since `0.3.0`; empty until
    /// set). The engine lists every optional component of this release.
    #[must_use]
    pub fn with_optional_components(
        mut self,
        components: &[(OptionalComponent, OptionalCapability)],
        profile: SetupProfile,
    ) -> Self {
        self.optional_components = components
            .iter()
            .map(|(component, capability)| {
                OptionalCheckResponse::new(*component, capability, profile)
            })
            .collect();
        self
    }

    /// Adds the managed versions the check used that the current catalogue
    /// supersedes (the additive `managed_updates` array; empty when none).
    #[must_use]
    pub fn with_managed_updates(mut self, updates: &[ManagedUpdate]) -> Self {
        self.managed_updates = updates.iter().map(ManagedUpdateResponse::new).collect();
        self
    }
}

/// The `local_asr` object of a `setup check` response.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct LocalAsrSetupResponse {
    model: LocalAsrModelResponse,
    verification: LocalAsrVerificationResponse,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct LocalAsrModelResponse {
    status: &'static str,
    profile: Option<&'static str>,
}

/// One flat object for every outcome, so a reader switches on `status` and
/// finds each detail field present (null when it does not apply).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct LocalAsrVerificationResponse {
    status: &'static str,
    source: Option<&'static str>,
    not_run_reason: Option<&'static str>,
    check: Option<&'static str>,
    reason: Option<&'static str>,
}

impl LocalAsrSetupResponse {
    fn new(status: LocalAsrSetupStatus) -> Self {
        let verification = status.verification;
        let (source, not_run_reason, check, reason) = match verification {
            LocalAsrCheckOutcome::Verified(source) => (Some(source.identifier()), None, None, None),
            LocalAsrCheckOutcome::Failed(failure) => {
                (None, None, Some(failure.check()), Some(failure.reason()))
            }
            LocalAsrCheckOutcome::NotRun(reason) => (None, Some(reason.identifier()), None, None),
        };
        Self {
            model: LocalAsrModelResponse {
                status: status.model.identifier(),
                profile: status.model.profile().map(ReviewedAsrModel::identifier),
            },
            verification: LocalAsrVerificationResponse {
                status: verification.identifier(),
                source,
                not_run_reason,
                check,
                reason,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SetupCheckDependencyResponse {
    #[serde(skip)]
    runtime_dependency: RuntimeDependency,
    dependency: &'static str,
    capability: &'static str,
    status: &'static str,
    detail: Option<String>,
    lookup: &'static str,
    validation: &'static str,
    remediation: Option<SetupRemediationResponse>,
}

/// What to do about a dependency that is not available.
///
/// `managed_install` and `explicit_path_option` are facts about this host and
/// this command (whether managed installation exists here, which option names
/// the executable); they are present for every reason because the published
/// schema requires them. The advice is `next_step`, and it differs by reason.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SetupRemediationResponse {
    reason: &'static str,
    managed_install: &'static str,
    required_authority: &'static str,
    next_step: String,
    explicit_path_option: &'static str,
}

impl SetupCheckDependencyResponse {
    fn new(
        status: &DependencyStatus,
        lookup: DependencyLookup,
        managed_install: ManagedPlanAvailability,
    ) -> Self {
        let detail = match &status.state {
            DependencyState::Available { version } => {
                Some(sanitize_untrusted_text(version, MAX_PROVIDER_DETAIL_BYTES))
            }
            DependencyState::Unhealthy { .. } => Some(String::from("dependency probe failed")),
            DependencyState::Missing | DependencyState::TimedOut(_) => None,
        };
        Self {
            runtime_dependency: status.dependency,
            dependency: status.dependency.identifier(),
            capability: status.dependency.capability().identifier(),
            status: status.state.identifier(),
            detail,
            lookup: lookup.identifier(),
            validation: if status.state.is_available() {
                "executable_probe_only"
            } else {
                "not_validated"
            },
            remediation: check_next_step(status).map(|next_step| SetupRemediationResponse {
                reason: status.state.identifier(),
                managed_install: managed_install.identifier(),
                required_authority: "user",
                next_step,
                explicit_path_option: explicit_path_option(status.dependency),
            }),
        }
    }
}

/// The `next_step` of a `setup check` dependency, or `None` for one that
/// responded.
///
/// A tool whose probe timed out was found, so it gets advice of its own:
/// telling its owner to install it is false (issue #391). A tool that was
/// found and failed its probe keeps the advice of a missing one, which is
/// true of it: another executable, or a repaired one, is the fix.
fn check_next_step(status: &DependencyStatus) -> Option<String> {
    match &status.state {
        DependencyState::Available { .. } => None,
        DependencyState::Missing | DependencyState::Unhealthy { .. } => {
            Some(missing_tool_step(status.dependency).to_owned())
        }
        DependencyState::TimedOut(timeout) => {
            Some(timed_out_check_step(status.dependency, *timeout))
        }
    }
}

/// The sentence every piece of advice for a missing or failing tool ends
/// its instructions with (issue #396).
///
/// `VSift` needs only the absolute path of a tool the user already has, and it
/// cannot know where a person or an agent may look. Advice that said
/// "install or locate" this tool was read by a cold agent as an invitation to
/// search the machine for it, and it listed folders and tried to read a
/// configuration file. The advice now says what `VSift` needs and who knows it.
/// The readable output of `setup check` prints it on a line of its own; the
/// JSON texts and the error remediations carry it inline.
pub const NO_FOLDER_SEARCH_ADVICE: &str = "No need to search folders: ask the user where it is.";

/// The `next_step` of `setup check` and of `setup plan` for a tool that is
/// missing, or was found and failed its probe (issue #396).
///
/// It names the two ways a tool becomes known to `VSift` that need no search:
/// install a trusted one, or register one that works with `setup configure`
/// and the absolute path the user gives; then [`NO_FOLDER_SEARCH_ADVICE`].
/// The managed installation is not named here: `setup check` reports whether
/// it is offered in `managed_install`, and a plan offers it as an action of
/// its own, never with this step. Each text fits the 240 characters the
/// published `setup check` schema allows a `next_step`, which is why the
/// whisper.cpp form leaves out "for local ASR" and keeps the supplied
/// transcript's sentence.
const fn missing_tool_step(dependency: RuntimeDependency) -> &'static str {
    match dependency {
        RuntimeDependency::Ffmpeg => {
            "Install a trusted FFmpeg or register a working one with setup configure ffmpeg --executable <absolute-path>, then run setup check. No need to search folders: ask the user where it is."
        }
        RuntimeDependency::Ffprobe => {
            "Install a trusted FFprobe or register a working one with setup configure ffprobe --executable <absolute-path>, then run setup check. No need to search folders: ask the user where it is."
        }
        RuntimeDependency::Whisper => {
            "Install a trusted whisper.cpp CLI or register a working one with setup configure whisper --executable <absolute-path>, then run setup check. No need to search folders: ask the user where it is. A supplied transcript can skip local ASR."
        }
    }
}

/// The name a `next_step` uses for a dependency that was found.
const fn found_tool_name(dependency: RuntimeDependency) -> &'static str {
    match dependency {
        RuntimeDependency::Ffmpeg => "FFmpeg",
        RuntimeDependency::Ffprobe => "FFprobe",
        RuntimeDependency::Whisper => "whisper.cpp",
    }
}

/// The longest deadline `setup check --timeout-seconds` accepts for all the
/// probes together: the figure the advice for a timed-out probe names as
/// "up to 60", and the bound the CLI's host policy applies.
pub const MAX_PROBE_TIMEOUT: Duration = Duration::from_secs(60);

/// Whether a caller whose probe timed out can still be offered more time.
///
/// "Allow more time with --timeout-seconds" is advice only while the option
/// has more to give. A probe that had the whole maximum and still had no
/// answer is not helped by it: what is left to say is that the tool itself
/// may hang.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeTimeHeadroom {
    /// The deadline that applied was shorter than the option's maximum.
    MoreTimeAllowed,
    /// The deadline that applied was the option's maximum, or longer (a
    /// library host may pass any duration).
    AtTheMaximum,
}

impl ProbeTimeHeadroom {
    /// The headroom left to a caller whose probes shared `deadline`.
    #[must_use]
    pub fn of(deadline: Duration) -> Self {
        if deadline < MAX_PROBE_TIMEOUT {
            Self::MoreTimeAllowed
        } else {
            Self::AtTheMaximum
        }
    }
}

/// The `setup check` advice for a tool whose probe timed out: what happened,
/// under which deadline, and what helps. It never says to install the tool,
/// because the tool was found.
///
/// The two kinds read differently because they are different facts: a tool
/// that was never started cannot be said to have given no answer. More time
/// is offered only while `--timeout-seconds` has more to give; at its maximum
/// the text says so and what a tool that never answers means. The text fits
/// the 240 characters the published schema allows a `next_step`, for every
/// dependency and deadline, which is why it is shorter than the readable
/// form and leaves out its example.
fn timed_out_check_step(dependency: RuntimeDependency, timeout: ProbeTimeout) -> String {
    let tool = found_tool_name(dependency);
    let deadline = probe_deadline_text(timeout.shared_deadline);
    let what_happened = match timeout.kind {
        ProbeTimeoutKind::NoAnswer => format!(
            "{tool} was found and started but did not answer before the deadline all tool checks share ({deadline})"
        ),
        ProbeTimeoutKind::NotStarted => format!(
            "{tool} was found but not started: the deadline all tool checks share ({deadline}) had already passed"
        ),
    };
    match ProbeTimeHeadroom::of(timeout.shared_deadline) {
        ProbeTimeHeadroom::MoreTimeAllowed => format!(
            "{what_happened}. A first run after an install or restart can be slow. Run setup check again, or allow more time with --timeout-seconds (up to 60)."
        ),
        ProbeTimeHeadroom::AtTheMaximum => {
            let if_it_never_answers = match timeout.kind {
                ProbeTimeoutKind::NoAnswer => {
                    "If it never answers, the tool itself hangs: run it yourself to see."
                }
                ProbeTimeoutKind::NotStarted => {
                    "If a tool never answers, that tool hangs: run it yourself to see."
                }
            };
            format!(
                "{what_happened}; --timeout-seconds cannot allow more. Run setup check again. {if_it_never_answers}"
            )
        }
    }
}

/// A probe deadline in words: `30 seconds`, `1 second`, `1500 ms` for one that
/// is not a whole number of seconds, and `over an hour` beyond that.
///
/// The CLI only ever applies whole seconds from 1 to 60; a library host may
/// pass any duration, and the text must stay true and short for all of them
/// (it goes into a `next_step` of at most 240 characters). No probe waits an
/// hour, so nothing is lost by not printing a larger figure.
#[must_use]
pub fn probe_deadline_text(deadline: Duration) -> String {
    const HOUR: Duration = Duration::from_secs(3_600);
    if deadline > HOUR {
        String::from("over an hour")
    } else if deadline.subsec_nanos() != 0 {
        // Rounded up, so a deadline under a millisecond is not printed as none.
        format!("{} ms", deadline.as_nanos().div_ceil(1_000_000))
    } else if deadline.as_secs() == 1 {
        String::from("1 second")
    } else {
        format!("{} seconds", deadline.as_secs())
    }
}

/// Returns the per-call option that selects an explicit executable for a
/// dependency.
///
/// The option name is part of the v1 `setup check` response
/// (`explicit_path_option`), so it is fixed here; the CLI reuses it in human
/// guidance so both forms name the same flag.
#[must_use]
pub const fn explicit_path_option(dependency: RuntimeDependency) -> &'static str {
    match dependency {
        RuntimeDependency::Ffmpeg => "--ffmpeg",
        RuntimeDependency::Ffprobe => "--ffprobe",
        RuntimeDependency::Whisper => "--whisper",
    }
}

/// Data of a `setup plan` result: the reviewable, read-only plan.
///
/// The same type is decoded strictly from a saved plan, which is why every nested
/// type rejects unknown fields: acceptance must compare exactly what the user
/// reviewed, and an unreviewed field must never ride along.
///
/// Two kinds of member share it (P13 PR 4). The **intent** (profile, target,
/// availability, catalogue, digest, every action's reviewed artifact, each
/// dependency's disposition and next step) is what the digest binds and what
/// acceptance compares. The **observed state** (`readiness`, `install_needed`,
/// each dependency's and the model's `status`, each action's `state`) is what
/// commands would use now, managed versions included; it changes as `setup
/// install` applies the plan, so acceptance ignores it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupPlanResponse {
    profile: String,
    readiness: String,
    verification_scope: String,
    target: String,
    local_asr_model: SetupPlanModelResponse,
    managed_install: String,
    catalogue_revision: Option<String>,
    stop_new_plans_at: Option<String>,
    plan_digest: Option<String>,
    install_needed: bool,
    actions: Vec<SetupPlanActionResponse>,
    dependencies: Vec<SetupPlanDependencyResponse>,
    /// The optional components the plan offers, includes or cannot offer
    /// here (#409, since `0.3.0`). A saved plan from an earlier release has
    /// none.
    #[serde(default)]
    optional_components: Vec<SetupPlanOptionalResponse>,
    /// The instruction to whoever runs setup, when the plan offers or
    /// includes an optional component: tell the operator and ask before
    /// accepting. `null` otherwise.
    #[serde(default)]
    operator_decision_required: Option<String>,
}

impl SetupPlanResponse {
    /// Presents an evaluated plan with what the machine shows now, including
    /// the manual guidance for each dependency and the model.
    ///
    /// Selected paths bound into the plan's digest are deliberately omitted: the
    /// public plan names what will change, not where the user keeps their tools.
    #[must_use]
    pub fn new(plan: &ManagedSetupPlan, observation: &ManagedPlanObservation) -> Self {
        let model_current = observation.is_current(ManagedComponent::WhisperModel);
        Self {
            profile: plan.profile.identifier().to_owned(),
            readiness: observation.readiness.identifier().to_owned(),
            verification_scope: "executable_probe_and_reviewed_catalogue".to_owned(),
            target: plan.target.identifier().to_owned(),
            local_asr_model: model_response(plan.model, observation.model, model_current),
            managed_install: plan.availability.identifier().to_owned(),
            catalogue_revision: plan.catalogue_revision.clone(),
            stop_new_plans_at: plan.stop_new_plans_date.clone(),
            plan_digest: plan.digest.clone(),
            install_needed: plan
                .actions
                .iter()
                .any(|action| !observation.is_current(action.artifact.component)),
            actions: plan
                .actions
                .iter()
                .map(|action| {
                    action_response(action, observation.is_current(action.artifact.component))
                })
                .collect(),
            dependencies: plan
                .dependencies
                .iter()
                .map(|(status, disposition)| {
                    let observed = observation
                        .dependencies
                        .iter()
                        .find(|observed| observed.dependency == status.dependency)
                        .unwrap_or(status);
                    dependency_response(
                        observed,
                        status,
                        *disposition,
                        plan.supply_of(status.dependency),
                    )
                })
                .collect(),
            optional_components: plan
                .optional
                .iter()
                .map(|optional| optional_response(plan.profile, optional, observation))
                .collect(),
            operator_decision_required: plan
                .operator_decision_required()
                .then(|| OPERATOR_DECISION_INSTRUCTION.to_owned()),
        }
        .with_optional_install_needed(plan, observation)
    }

    /// `install_needed` also counts an included optional component that is
    /// not current yet.
    fn with_optional_install_needed(
        mut self,
        plan: &ManagedSetupPlan,
        observation: &ManagedPlanObservation,
    ) -> Self {
        self.install_needed = self.install_needed
            || plan
                .optional_actions()
                .any(|action| !observation.is_current(action.artifact.component));
        self
    }

    /// The optional components' entries, for presentation.
    #[must_use]
    pub fn optional_components(&self) -> Vec<OptionalComponentView<'_>> {
        self.optional_components
            .iter()
            .map(|optional| OptionalComponentView {
                name: &optional.name,
                status: &optional.status,
                summary: &optional.summary,
                add_command: optional
                    .add_command
                    .as_ref()
                    .map(|command| command.arguments.as_slice()),
                next_step: &optional.next_step,
            })
            .collect()
    }

    /// The instruction to ask the operator, when the plan carries one.
    #[must_use]
    pub fn operator_decision_required(&self) -> Option<&str> {
        self.operator_decision_required.as_deref()
    }

    /// The plan without its observed state: what the digest binds and
    /// acceptance compares.
    fn intent(&self) -> Self {
        let mut intent = self.clone();
        intent.readiness.clear();
        intent.install_needed = false;
        intent.local_asr_model.status.clear();
        for dependency in &mut intent.dependencies {
            dependency.status.clear();
        }
        for action in &mut intent.actions {
            action.state.clear();
        }
        for optional in &mut intent.optional_components {
            for action in &mut optional.actions {
                action.state.clear();
            }
        }
        intent
    }
}

/// The instruction a plan that offers or includes an optional component
/// carries for whoever runs setup (#409): an agent that has never seen the
/// skill relays it too, because it is in the plan.
pub const OPERATOR_DECISION_INSTRUCTION: &str = "Before accepting this plan, tell the operator about each optional component in optional_components in one or two plain sentences (what it does, its licence, its download size, and its terms notice when it has one) and ask whether to include it. Include it only after a yes: run its add_command, review that plan and accept it instead. Never accept a plan that includes it without a yes, and never leave it unmentioned. Local investigation does not need it.";

/// A borrowed view of one optional component of a plan, for presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OptionalComponentView<'plan> {
    /// The optional component's name (`online_links`).
    pub name: &'plan str,
    /// `included`, `not_included` or `unavailable`.
    pub status: &'plan str,
    /// The one plain sentence about it.
    pub summary: &'plan str,
    /// The arguments of the `vsift` command that adds it, when it can be.
    pub add_command: Option<&'plan [String]>,
    /// What to do next about it.
    pub next_step: &'plan str,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SetupPlanOptionalResponse {
    name: String,
    status: String,
    installs: Vec<String>,
    summary: String,
    purpose: String,
    download_bytes: Option<u64>,
    licences: Vec<SetupPlanLicenceResponse>,
    terms_notice: Option<String>,
    add_command: Option<SetupPlanCommandResponse>,
    actions: Vec<SetupPlanActionResponse>,
    next_step: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SetupPlanLicenceResponse {
    component: String,
    licence: String,
}

/// A `vsift` command, as an executable and an argument array, run without
/// a shell.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SetupPlanCommandResponse {
    executable: String,
    arguments: Vec<String>,
}

fn optional_response(
    profile: SetupProfile,
    optional: &OptionalComponentPlan,
    observation: &ManagedPlanObservation,
) -> SetupPlanOptionalResponse {
    let text = optional_component_text(optional.component);
    let name = optional.component.identifier();
    let available = optional.disposition != OptionalComponentDisposition::Unavailable;
    let download_bytes = available.then(|| optional.download_bytes());
    let add_command =
        (optional.disposition == OptionalComponentDisposition::NotIncluded).then(|| {
            SetupPlanCommandResponse {
                executable: String::from("vsift"),
                arguments: include_command_arguments(optional.component, profile)
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            }
        });
    let next_step = match optional.disposition {
        OptionalComponentDisposition::Included => format!(
            "Included: accepting this plan installs the {} components after the core components. If one cannot be installed, the core is still installed and VSift is ready for local videos.",
            text.name_in_prose
        ),
        OptionalComponentDisposition::NotIncluded => format!(
            "Not included. Ask the operator first; to add it, run vsift setup plan --profile {} --include {name} --json and review that plan instead.",
            profile.identifier()
        ),
        OptionalComponentDisposition::Unavailable => {
            format!("This plan cannot include it here. {}", text.unavailable)
        }
    };
    SetupPlanOptionalResponse {
        name: name.to_owned(),
        status: optional.disposition.identifier().to_owned(),
        installs: optional
            .component
            .components()
            .iter()
            .map(|installed| installed.identifier().to_owned())
            .collect(),
        summary: optional_summary(text, optional.disposition, download_bytes),
        purpose: text.purpose.to_owned(),
        download_bytes,
        licences: optional
            .artifacts
            .iter()
            .map(|artifact| SetupPlanLicenceResponse {
                component: artifact.component.identifier().to_owned(),
                licence: artifact.licence.clone(),
            })
            .collect(),
        terms_notice: text.terms_notice.map(str::to_owned),
        add_command,
        actions: optional
            .actions
            .iter()
            .map(|action| {
                action_response(action, observation.is_current(action.artifact.component))
            })
            .collect(),
        next_step,
    }
}

/// The one plain sentence about an optional component that an agent relays
/// to the operator: what it is, its size, its licences and any terms.
fn optional_summary(
    text: OptionalComponentText,
    disposition: OptionalComponentDisposition,
    download_bytes: Option<u64>,
) -> String {
    let size = download_bytes.map_or_else(String::new, |bytes| {
        format!(
            " (optional, about {} MB to download)",
            bytes.div_ceil(1_000_000)
        )
    });
    let lead = match disposition {
        OptionalComponentDisposition::Included => "Included",
        OptionalComponentDisposition::NotIncluded => "Not included",
        OptionalComponentDisposition::Unavailable => "Not available here",
    };
    format!("{lead}{size}: {}", text.subject)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SetupPlanDependencyResponse {
    dependency: String,
    status: String,
    disposition: String,
    required_authority: Option<String>,
    next_step: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SetupPlanModelResponse {
    status: String,
    disposition: String,
    required_authority: Option<String>,
    next_step: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SetupPlanActionResponse {
    id: String,
    state: String,
    component: String,
    version: String,
    publisher: String,
    source_url: String,
    bytes: u64,
    sha256: String,
    format: String,
    archive_limits: Option<SetupArchiveLimitsResponse>,
    selected_files: Vec<SetupArchiveSelectionResponse>,
    archive_links: Vec<SetupArchiveLinkResponse>,
    runtime_copies: Vec<SetupRuntimeCopyResponse>,
    licence: String,
    notice_url: String,
    source_code_url: String,
    trust_limit: String,
    licence_scope: String,
    destination: String,
    permissions: String,
    change: String,
    required_authority: String,
    files: Vec<SetupPlanFileResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SetupPlanFileResponse {
    name: String,
    bytes: u64,
    sha256: String,
    mode: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SetupArchiveLimitsResponse {
    max_stream_bytes: u64,
    entries: usize,
    expanded_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SetupArchiveSelectionResponse {
    archive_path: String,
    runtime_name: String,
    bytes: u64,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SetupArchiveLinkResponse {
    archive_path: String,
    target: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SetupRuntimeCopyResponse {
    name: String,
    source_selected: String,
}

/// One dependency of a plan: the status commands would see now (`observed`,
/// managed versions included) beside the plan's intent for it.
///
/// The next step is part of the intent, which acceptance compares, so it is
/// chosen from what the plan's own probe saw (`planned`) and from the plan's
/// own actions (`supply`), and never from the observed status, which changes
/// as `setup install` applies the plan.
fn dependency_response(
    observed: &DependencyStatus,
    planned: &DependencyStatus,
    disposition: SetupDependencyDisposition,
    supply: PlannedSupply,
) -> SetupPlanDependencyResponse {
    let (required_authority, next_step) = match disposition {
        SetupDependencyDisposition::ExistingProbeOnly => (
            None,
            "Keep this executable selected and verify provider compatibility before use.",
        ),
        SetupDependencyDisposition::ManagedInstall => (
            Some("user"),
            "Review the exact managed action, then run setup install with this saved plan and its digest; a component already installed at this version is reported already_current.",
        ),
        SetupDependencyDisposition::ManualSelection => (
            Some("user"),
            match (&planned.state, supply) {
                (DependencyState::TimedOut(_), PlannedSupply::NotInstalled) => {
                    timed_out_plan_step(planned.dependency)
                }
                (DependencyState::TimedOut(_), PlannedSupply::Installed) => {
                    timed_out_beside_planned_build_step(planned.dependency)
                }
                (
                    DependencyState::Available { .. }
                    | DependencyState::Missing
                    | DependencyState::Unhealthy { .. },
                    PlannedSupply::Installed | PlannedSupply::NotInstalled,
                ) => missing_tool_step(planned.dependency),
            },
        ),
    };
    SetupPlanDependencyResponse {
        dependency: observed.dependency.identifier().to_owned(),
        status: observed.state.identifier().to_owned(),
        disposition: disposition.identifier().to_owned(),
        required_authority: required_authority.map(str::to_owned),
        next_step: next_step.to_owned(),
    }
}

fn model_response(
    disposition: SetupModelDisposition,
    observed: ObservedModel,
    model_current: bool,
) -> SetupPlanModelResponse {
    let (status, required_authority, next_step) = match disposition {
        SetupModelDisposition::ConfiguredProbeOnly => (
            "configured_present_unverified",
            None,
            "Keep the configured model selected and validate it with provider preflight before use.",
        ),
        SetupModelDisposition::ManagedInstall => (
            "missing",
            Some("user"),
            "Review the exact managed model action, then run setup install with this saved plan and its digest; a model already installed at this version is reported already_current.",
        ),
        SetupModelDisposition::ManualSelection => (
            "missing",
            Some("user"),
            "For local ASR, configure trusted model weights with setup configure-model --file <absolute-path>. A supplied transcript can skip local ASR.",
        ),
    };
    let status = if observed == ObservedModel::Managed && model_current {
        "managed_current"
    } else {
        status
    };
    SetupPlanModelResponse {
        status: status.to_owned(),
        disposition: disposition.identifier().to_owned(),
        required_authority: required_authority.map(str::to_owned),
        next_step: next_step.to_owned(),
    }
}

fn action_response(action: &ManagedSetupAction, current: bool) -> SetupPlanActionResponse {
    SetupPlanActionResponse {
        id: action.id.clone(),
        state: if current { "current" } else { "pending" }.to_owned(),
        component: action.artifact.component.identifier().to_owned(),
        version: action.artifact.version.clone(),
        publisher: action.artifact.publisher.clone(),
        source_url: action.artifact.source_url.clone(),
        bytes: action.artifact.integrity.bytes(),
        sha256: action.artifact.integrity.sha256_hex(),
        format: action.artifact.format.identifier().to_owned(),
        archive_limits: action
            .artifact
            .archive_limits
            .map(|limits| SetupArchiveLimitsResponse {
                max_stream_bytes: limits.max_stream_bytes,
                entries: limits.entries,
                expanded_bytes: limits.expanded_bytes,
            }),
        selected_files: action
            .artifact
            .selected_files
            .iter()
            .map(|file| SetupArchiveSelectionResponse {
                archive_path: file.archive_path.clone(),
                runtime_name: file.runtime_name.clone(),
                bytes: file.integrity.bytes(),
                sha256: file.integrity.sha256_hex(),
            })
            .collect(),
        archive_links: action
            .artifact
            .archive_links
            .iter()
            .map(|link| SetupArchiveLinkResponse {
                archive_path: link.archive_path.clone(),
                target: link.target.clone(),
            })
            .collect(),
        runtime_copies: action
            .artifact
            .runtime_copies
            .iter()
            .map(|copy| SetupRuntimeCopyResponse {
                name: copy.name.clone(),
                source_selected: copy.source_selected.clone(),
            })
            .collect(),
        licence: action.artifact.licence.clone(),
        notice_url: action.artifact.notice_url.clone(),
        source_code_url: action.artifact.source_code_url.clone(),
        trust_limit: action.artifact.trust_limit.clone(),
        licence_scope: "disclosure_not_legal_clearance".to_owned(),
        destination: "private_per_user_managed_runtime".to_owned(),
        permissions: "private_user_only".to_owned(),
        change: "planned_download_verify_extract_smoke_activate".to_owned(),
        required_authority: "user".to_owned(),
        files: action
            .artifact
            .files
            .iter()
            .map(|file| SetupPlanFileResponse {
                name: file.name.clone(),
                bytes: file.integrity.bytes(),
                sha256: file.integrity.sha256_hex(),
                mode: if file.executable {
                    "owner_executable"
                } else {
                    "owner_read_write"
                }
                .to_owned(),
            })
            .collect(),
    }
}

/// The plan's next step for a tool whose probe timed out while the plan
/// installs the build that supplies it anyway, because the tool it is paired
/// with is missing or failing.
///
/// `FFmpeg` and `FFprobe` come in one managed build, and a managed version
/// is used ahead of a tool found on `PATH`. So "this plan proposes no install
/// because of it" would be true and still mislead: accepting the plan
/// replaces the tool. The step says what the plan does to it and that the
/// timeout is not the reason. whisper.cpp has no pair, and its build is
/// planned only when its own probe showed it unusable, so it cannot be here
/// with a timeout; it keeps the ordinary text so that this stays total.
const fn timed_out_beside_planned_build_step(dependency: RuntimeDependency) -> &'static str {
    match dependency {
        RuntimeDependency::Ffmpeg => {
            "FFmpeg was found, but its check did not finish before the deadline the tool checks share: it did not answer in time, or it was not started because the time had run out. This plan cannot tell whether it works. It proposes installing the paired FFmpeg and FFprobe build because FFprobe is missing or failed its check, not because of this timeout; that build includes FFmpeg, and VSift then uses it instead of this one unless you registered this one with setup configure. Review that action before you accept."
        }
        RuntimeDependency::Ffprobe => {
            "FFprobe was found, but its check did not finish before the deadline the tool checks share: it did not answer in time, or it was not started because the time had run out. This plan cannot tell whether it works. It proposes installing the paired FFmpeg and FFprobe build because FFmpeg is missing or failed its check, not because of this timeout; that build includes FFprobe, and VSift then uses it instead of this one unless you registered this one with setup configure. Review that action before you accept."
        }
        RuntimeDependency::Whisper => timed_out_plan_step(dependency),
    }
}

/// The plan's next step for a tool whose probe timed out when no action of
/// the plan installs what supplies it: the tool was found, the plan learned
/// nothing about it and proposes nothing for it (issue #391).
///
/// One text serves both kinds of timeout, and names no deadline: the next
/// step is part of the plan's intent, which the digest stands for, and the
/// digest binds only that the probe timed out. `setup plan` has no option to
/// allow more time, so the text sends the reader to `setup check`, which has.
const fn timed_out_plan_step(dependency: RuntimeDependency) -> &'static str {
    match dependency {
        RuntimeDependency::Ffmpeg => {
            "FFmpeg was found, but its check did not finish before the deadline the tool checks share: it did not answer in time, or it was not started because the time had run out. This plan cannot tell whether it works, so it decides nothing for it and proposes no install because of it. A first run after an install or restart can be slow. Run setup check until it answers (it can be given more time with --timeout-seconds, up to 60), then run setup plan again."
        }
        RuntimeDependency::Ffprobe => {
            "FFprobe was found, but its check did not finish before the deadline the tool checks share: it did not answer in time, or it was not started because the time had run out. This plan cannot tell whether it works, so it decides nothing for it and proposes no install because of it. A first run after an install or restart can be slow. Run setup check until it answers (it can be given more time with --timeout-seconds, up to 60), then run setup plan again."
        }
        RuntimeDependency::Whisper => {
            "whisper.cpp was found, but its check did not finish before the deadline the tool checks share: it did not answer in time, or it was not started because the time had run out. This plan cannot tell whether it works, so it decides nothing for it and proposes no install because of it. A first run after an install or restart can be slow. Run setup check until it answers (it can be given more time with --timeout-seconds, up to 60), then run setup plan again."
        }
    }
}

/// A saved `setup plan --json` result supplied back for acceptance.
///
/// This is strict request input, not a tolerant response reader: unknown fields
/// are rejected at every level, the envelope members a successful plan holds as
/// `null` (`error`, `coverage`, `lifecycle`) are refused while decoding when
/// they hold anything else, and [`SavedSetupPlan::validate_envelope`] rejects
/// anything other than an unmodified successful v1 plan. The host performs the
/// bounded read and decode; this type owns what a valid saved plan is.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavedSetupPlan {
    schema_version: String,
    command: String,
    operation_id: Option<String>,
    status: String,
    data: SetupPlanResponse,
    warnings: Vec<String>,
    error: Option<NullOnly>,
    coverage: Option<NullOnly>,
    lifecycle: Option<NullOnly>,
}

/// The type of an envelope member a saved plan holds as `null`: it has no
/// value, so `Option<NullOnly>` decodes `null` (or an absent member) as `None`
/// and refuses every other value while decoding.
///
/// These members were `Option<serde_json::Value>` until #394, so any JSON was
/// decoded into the plan and only [`SavedSetupPlan::validate_envelope`] refused
/// it. A decoded plan then held whatever the file said, which a strict request
/// type should not: the hosted fuzz run of `0.2.1-rc.3` found a `lifecycle` of
/// about 8.2e118 that did not read back the same after being written out. Now a
/// decoded plan always writes out as `null` there, and reads back the same.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(
    clippy::empty_enums,
    reason = "a member with no value is the point; `Infallible` has no serde impls"
)]
enum NullOnly {}

impl SavedSetupPlan {
    /// Requires the decoded document to be an unmodified successful v1
    /// `setup.plan` result.
    ///
    /// `error`, `coverage` and `lifecycle` need no check here: a plan that
    /// holds anything but `null` there is refused while decoding, because
    /// their type has no value but `null`.
    ///
    /// # Errors
    ///
    /// Returns [`FailureCode::InvalidArgument`] when the version, command, status
    /// or any other envelope field differs from what `setup plan --json` emits.
    pub fn validate_envelope(&self) -> Result<(), FailureCode> {
        if self.schema_version != CONTRACT_VERSION
            || self.command != CommandName::SetupPlan.identifier()
            || self.operation_id.is_some()
            || self.status != "complete"
            || !self.warnings.is_empty()
        {
            return Err(FailureCode::InvalidArgument);
        }
        Ok(())
    }

    /// The optional components the saved plan includes, which the current
    /// plan must include too before it can accept it (#409).
    ///
    /// # Errors
    ///
    /// Returns [`FailureCode::InvalidArgument`] for an unknown component or
    /// status.
    pub fn included_optional(
        &self,
    ) -> Result<vsift_application::OptionalComponentSet, FailureCode> {
        self.data.optional_components.iter().try_fold(
            vsift_application::OptionalComponentSet::EMPTY,
            |set, optional| {
                let component = OptionalComponent::from_identifier(&optional.name)
                    .ok_or(FailureCode::InvalidArgument)?;
                match optional.status.as_str() {
                    "included" => Ok(set.with(component)),
                    "not_included" | "unavailable" => Ok(set),
                    _ => Err(FailureCode::InvalidArgument),
                }
            },
        )
    }

    /// Returns the profile that must be re-observed before acceptance.
    ///
    /// # Errors
    ///
    /// Returns [`FailureCode::InvalidArgument`] for an unknown profile.
    pub fn profile(&self) -> Result<SetupProfile, FailureCode> {
        match self.data.profile.as_str() {
            "desktop" => Ok(SetupProfile::Desktop),
            "worker" => Ok(SetupProfile::Worker),
            _ => Err(FailureCode::InvalidArgument),
        }
    }

    /// Requires the saved plan to describe exactly the current plan's intent.
    ///
    /// Comparison is over the serialized JSON values, the form the user
    /// reviewed, so any drift in the catalogue, the target, the selections or
    /// the tools outside the managed store since the plan was saved
    /// invalidates it. The observed state (readiness, statuses, each action's
    /// `state`) is left out: it changes as `setup install` applies the plan,
    /// and a rerun of the same accepted command must continue.
    ///
    /// # Errors
    ///
    /// Returns [`FailureCode::InvalidArgument`] when the plans differ, or
    /// [`FailureCode::Internal`] when either cannot be serialized.
    pub fn require_same_plan(&self, current: &SetupPlanResponse) -> Result<(), FailureCode> {
        let saved_data =
            serde_json::to_value(self.data.intent()).map_err(|_| FailureCode::Internal)?;
        let current_data =
            serde_json::to_value(current.intent()).map_err(|_| FailureCode::Internal)?;
        if saved_data == current_data {
            Ok(())
        } else {
            Err(FailureCode::InvalidArgument)
        }
    }
}

/// Data of a successful `setup configure` result.
///
/// Registration only canonicalizes and stores the path; the response says so, so
/// an agent does not mistake a saved path for a verified tool.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConfiguredSelectionResponse {
    dependency: &'static str,
    source: &'static str,
    validation: &'static str,
    next_step: &'static str,
}

impl ConfiguredSelectionResponse {
    /// Describes a newly saved executable selection for `dependency`.
    #[must_use]
    pub const fn new(dependency: RuntimeDependency) -> Self {
        Self {
            dependency: dependency.identifier(),
            source: "configured_user_path",
            validation: "canonical_file_only",
            next_step: "Run setup check to probe the selected executable; model and provider compatibility remain unverified.",
        }
    }

    /// Describes a newly saved OCR runner selection (ADR 0030).
    #[must_use]
    pub const fn ocr_runner() -> Self {
        Self {
            dependency: "ocr_runner",
            source: "configured_user_path",
            validation: "canonical_file_only",
            next_step: "Run ocr on a session to use it; the runner is identified (describe) when it runs, and its runtime and models must sit beside it.",
        }
    }
}

/// Data of a successful `setup configure-model` result.
///
/// The model bytes are neither read nor run at registration, which the response
/// states explicitly.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConfiguredModelResponse {
    source: &'static str,
    validation: &'static str,
    next_step: &'static str,
}

impl ConfiguredModelResponse {
    /// Describes a newly saved model-file selection.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            source: "configured_user_path",
            validation: "canonical_nonempty_file_only",
            next_step: "Model format and provider compatibility remain unverified; setup check still probes executables only.",
        }
    }
}

impl Default for ConfiguredModelResponse {
    fn default() -> Self {
        Self::new()
    }
}
