//! Setup check, setup plan and dependency-registration responses.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use vsift_application::{
    LocalAsrCheckOutcome, LocalAsrSetupStatus, ManagedPlanAvailability, ManagedPlanObservation,
    ManagedSetupAction, ManagedSetupPlan, ObservedModel, RuntimeDiagnosis,
    SetupDependencyDisposition, SetupModelDisposition, SetupProfile,
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
        }
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
/// telling its owner to install or locate it is false (issue #391). A tool
/// that was found and failed its probe keeps the advice of a missing one,
/// which is true of it: another executable, or a repaired one, is the fix.
fn check_next_step(status: &DependencyStatus) -> Option<String> {
    match &status.state {
        DependencyState::Available { .. } => None,
        DependencyState::Missing | DependencyState::Unhealthy { .. } => {
            Some(manual_dependency_step(status.dependency).to_owned())
        }
        DependencyState::TimedOut(timeout) => {
            Some(timed_out_check_step(status.dependency, *timeout))
        }
    }
}

const fn manual_dependency_step(dependency: RuntimeDependency) -> &'static str {
    match dependency {
        RuntimeDependency::Ffmpeg => {
            "Install or locate a trusted FFmpeg executable, then rerun setup check."
        }
        RuntimeDependency::Ffprobe => {
            "Install or locate a trusted FFprobe executable, then rerun setup check."
        }
        RuntimeDependency::Whisper => {
            "Install or locate a trusted whisper.cpp CLI executable for local ASR, then rerun setup check. A supplied transcript can skip local ASR."
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

/// The `setup check` advice for a tool whose probe timed out: what happened,
/// under which deadline, and the two things that help. It never says to
/// install or locate the tool, because the tool was found.
///
/// The two kinds read differently because they are different facts: a tool
/// that was never started cannot be said to have given no answer. The text
/// fits the 240 characters the published schema allows a `next_step`, for
/// every dependency and deadline, which is why it is shorter than the
/// readable form and leaves out its example.
fn timed_out_check_step(dependency: RuntimeDependency, timeout: ProbeTimeout) -> String {
    let tool = found_tool_name(dependency);
    let deadline = probe_deadline_text(timeout.shared_deadline);
    let what_happened = match timeout.kind {
        ProbeTimeoutKind::NoAnswer => format!(
            "{tool} was found and started but did not answer before the deadline all tool checks share ({deadline})."
        ),
        ProbeTimeoutKind::NotStarted => format!(
            "{tool} was found but not started: the deadline all tool checks share ({deadline}) had already passed."
        ),
    };
    format!(
        "{what_happened} A first run after an install or restart can be slow. Run setup check again, or allow more time with --timeout-seconds (up to 60)."
    )
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
                    dependency_response(observed, status, *disposition)
                })
                .collect(),
        }
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
        intent
    }
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
/// chosen from what the plan's own probe saw (`planned`) and never from the
/// observed status, which changes as `setup install` applies the plan.
fn dependency_response(
    observed: &DependencyStatus,
    planned: &DependencyStatus,
    disposition: SetupDependencyDisposition,
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
            match planned.state {
                DependencyState::TimedOut(_) => timed_out_plan_step(planned.dependency),
                DependencyState::Available { .. }
                | DependencyState::Missing
                | DependencyState::Unhealthy { .. } => manual_plan_step(planned.dependency),
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

const fn manual_plan_step(dependency: RuntimeDependency) -> &'static str {
    match dependency {
        RuntimeDependency::Ffmpeg => {
            "Install or locate trusted FFmpeg, then run setup configure ffmpeg --executable <absolute-path> and setup check."
        }
        RuntimeDependency::Ffprobe => {
            "Install or locate trusted FFprobe, then run setup configure ffprobe --executable <absolute-path> and setup check."
        }
        RuntimeDependency::Whisper => {
            "For local ASR, install or locate a trusted whisper.cpp CLI, then run setup configure whisper --executable <absolute-path> and setup check. A supplied transcript can skip local ASR."
        }
    }
}

/// The plan's next step for a tool whose probe timed out: the tool was found,
/// the plan learned nothing about it and therefore proposes nothing for it
/// (issue #391).
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
/// are rejected at every level, and [`SavedSetupPlan::validate_envelope`] rejects
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
    error: Option<serde_json::Value>,
    coverage: Option<serde_json::Value>,
    lifecycle: Option<serde_json::Value>,
}

impl SavedSetupPlan {
    /// Requires the decoded document to be an unmodified successful v1
    /// `setup.plan` result.
    ///
    /// # Errors
    ///
    /// Returns [`FailureCode::InvalidArgument`] when the version, command, status
    /// or any envelope field differs from what `setup plan --json` emits.
    pub fn validate_envelope(&self) -> Result<(), FailureCode> {
        if self.schema_version != CONTRACT_VERSION
            || self.command != CommandName::SetupPlan.identifier()
            || self.operation_id.is_some()
            || self.status != "complete"
            || !self.warnings.is_empty()
            || self.error.is_some()
            || self.coverage.is_some()
            || self.lifecycle.is_some()
        {
            return Err(FailureCode::InvalidArgument);
        }
        Ok(())
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
