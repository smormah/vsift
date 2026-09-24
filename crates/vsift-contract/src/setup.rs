//! Setup check, setup plan and dependency-registration responses.

use serde::{Deserialize, Serialize};
use vsift_application::{
    ManagedSetupAction, ManagedSetupPlan, RuntimeDiagnosis, SetupDependencyDisposition,
    SetupModelDisposition, SetupProfile,
};
use vsift_domain::{DependencyState, DependencyStatus, FailureCode, RuntimeDependency};

use crate::{
    command::CommandName,
    envelope::CONTRACT_VERSION,
    text::{MAX_PROVIDER_DETAIL_BYTES, sanitize_untrusted_text},
};

/// Where the executable probed for one dependency came from.
///
/// The host resolves the precedence (per-call path, then configured user path,
/// then filtered `PATH`) because it owns argument parsing and configuration; the
/// contract only fixes how that provenance is reported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyLookup {
    /// An absolute path supplied for this one invocation.
    ExplicitPath,
    /// A path previously saved with `setup configure`.
    ConfiguredUserPath,
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
            Self::FilteredPath => "filtered_path",
        }
    }
}

/// Backward-compatible v1 `setup check` response.
///
/// Unlike newer commands this is not wrapped in the operation envelope when
/// written as plain JSON; the P01 shape is preserved for existing consumers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SetupCheckResponse {
    schema_version: &'static str,
    command: &'static str,
    profile: &'static str,
    status: &'static str,
    verification_scope: &'static str,
    local_asr_model: &'static str,
    dependencies: Vec<SetupCheckDependencyResponse>,
}

impl SetupCheckResponse {
    /// Creates the compatible setup response for the explicitly resolved profile.
    ///
    /// `lookup` reports, for each probed dependency, where its executable came
    /// from.
    #[must_use]
    pub fn new<F>(diagnosis: &RuntimeDiagnosis, profile: SetupProfile, lookup: F) -> Self
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
                .map(|status| SetupCheckDependencyResponse::new(status, lookup(status.dependency)))
                .collect(),
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SetupRemediationResponse {
    reason: &'static str,
    managed_install: &'static str,
    required_authority: &'static str,
    next_step: &'static str,
    explicit_path_option: &'static str,
}

impl SetupCheckDependencyResponse {
    fn new(status: &DependencyStatus, lookup: DependencyLookup) -> Self {
        let detail = match &status.state {
            DependencyState::Available { version } => {
                Some(sanitize_untrusted_text(version, MAX_PROVIDER_DETAIL_BYTES))
            }
            DependencyState::Unhealthy { .. } => Some(String::from("dependency probe failed")),
            DependencyState::Missing | DependencyState::TimedOut => None,
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
            remediation: (!status.state.is_available()).then_some(SetupRemediationResponse {
                reason: status.state.identifier(),
                managed_install: "unavailable_unqualified",
                required_authority: "user",
                next_step: manual_dependency_step(status.dependency),
                explicit_path_option: explicit_path_option(status.dependency),
            }),
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
    actions: Vec<SetupPlanActionResponse>,
    dependencies: Vec<SetupPlanDependencyResponse>,
}

impl SetupPlanResponse {
    /// Presents an evaluated plan, including the manual guidance for each
    /// dependency and the model.
    ///
    /// Selected paths bound into the plan's digest are deliberately omitted: the
    /// public plan names what will change, not where the user keeps their tools.
    #[must_use]
    pub fn new(plan: &ManagedSetupPlan) -> Self {
        Self {
            profile: plan.profile.identifier().to_owned(),
            readiness: plan.readiness.identifier().to_owned(),
            verification_scope: "executable_probe_and_reviewed_catalogue".to_owned(),
            target: plan.target.identifier().to_owned(),
            local_asr_model: model_response(plan.model),
            managed_install: plan.availability.identifier().to_owned(),
            catalogue_revision: plan.catalogue_revision.clone(),
            stop_new_plans_at: plan.stop_new_plans_date.clone(),
            plan_digest: plan.digest.clone(),
            actions: plan.actions.iter().map(action_response).collect(),
            dependencies: plan
                .dependencies
                .iter()
                .map(|(status, disposition)| dependency_response(status, *disposition))
                .collect(),
        }
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

fn dependency_response(
    status: &DependencyStatus,
    disposition: SetupDependencyDisposition,
) -> SetupPlanDependencyResponse {
    let (required_authority, next_step) = match disposition {
        SetupDependencyDisposition::ExistingProbeOnly => (
            None,
            "Keep this executable selected and verify provider compatibility before use.",
        ),
        SetupDependencyDisposition::ManagedInstall => (
            Some("user"),
            "Review the exact managed action and its digest. Setup install remains unavailable until the complete installer qualifies.",
        ),
        SetupDependencyDisposition::ManualSelection => {
            (Some("user"), manual_plan_step(status.dependency))
        }
    };
    SetupPlanDependencyResponse {
        dependency: status.dependency.identifier().to_owned(),
        status: status.state.identifier().to_owned(),
        disposition: disposition.identifier().to_owned(),
        required_authority: required_authority.map(str::to_owned),
        next_step: next_step.to_owned(),
    }
}

fn model_response(disposition: SetupModelDisposition) -> SetupPlanModelResponse {
    let (status, required_authority, next_step) = match disposition {
        SetupModelDisposition::ConfiguredProbeOnly => (
            "configured_present_unverified",
            None,
            "Keep the configured model selected and validate it with provider preflight before use.",
        ),
        SetupModelDisposition::ManagedInstall => (
            "missing",
            Some("user"),
            "Review the exact managed model action and its digest. Setup install remains unavailable until the complete installer qualifies.",
        ),
        SetupModelDisposition::ManualSelection => (
            "missing",
            Some("user"),
            "For local ASR, configure trusted model weights with setup configure-model --file <absolute-path>. A supplied transcript can skip local ASR.",
        ),
    };
    SetupPlanModelResponse {
        status: status.to_owned(),
        disposition: disposition.identifier().to_owned(),
        required_authority: required_authority.map(str::to_owned),
        next_step: next_step.to_owned(),
    }
}

fn action_response(action: &ManagedSetupAction) -> SetupPlanActionResponse {
    SetupPlanActionResponse {
        id: action.id.clone(),
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

    /// Requires the saved plan to describe exactly the current plan.
    ///
    /// Comparison is over the serialized JSON values, the form the user
    /// reviewed, so any drift in the machine, catalogue or selections since the
    /// plan was saved invalidates it.
    ///
    /// # Errors
    ///
    /// Returns [`FailureCode::InvalidArgument`] when the plans differ, or
    /// [`FailureCode::Internal`] when either cannot be serialized.
    pub fn require_same_plan(&self, current: &SetupPlanResponse) -> Result<(), FailureCode> {
        let saved_data = serde_json::to_value(&self.data).map_err(|_| FailureCode::Internal)?;
        let current_data = serde_json::to_value(current).map_err(|_| FailureCode::Internal)?;
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
