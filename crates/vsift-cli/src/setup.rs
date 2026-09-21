//! Setup command composition and presentation.

use std::io::Write;

use serde::Serialize;
use vsift_application::{
    AcceptedManagedCatalogue, DependencyProbe, DiagnoseRuntime, ManagedSetupAction,
    RuntimeDiagnosis, SetupDependencyDisposition, SetupModelDisposition, SetupProfile,
    SetupSelectionState, plan_managed_setup,
};
use vsift_domain::{
    DependencyState, FailureCode, ManagedTarget, RuntimeDependency, RuntimeReadiness,
};
use vsift_infrastructure::{ExplicitProbePaths, UserDependencyConfigStore};

use crate::{
    command::{ExecutionProfile, SetupConfigureArguments, SetupConfigureModelArguments},
    output::{
        OperationResponse, OutputMode, OutputWriter, ProcessExit, SetupCheckResponse,
        TerminalEventResponse, explicit_path_option, sanitize_untrusted_text, setup_exit,
    },
};

/// Executes the read-only setup check with explicit dependencies and output streams.
pub(crate) async fn run_setup_check<P, StandardOutput, StandardError>(
    probe: P,
    profile: ExecutionProfile,
    mode: OutputMode,
    selections: &ExplicitProbePaths,
    per_call: &ExplicitProbePaths,
    writer: &mut OutputWriter<StandardOutput, StandardError>,
) -> ProcessExit
where
    P: DependencyProbe,
    StandardOutput: Write,
    StandardError: Write,
{
    let diagnosis = DiagnoseRuntime::new(probe).execute().await;
    let response = SetupCheckResponse::new(&diagnosis, profile, selections, per_call);
    let output_result = match mode {
        OutputMode::Human => {
            writer.write_trusted_stdout(&human_result(&diagnosis, profile, selections, per_call))
        }
        OutputMode::Json => writer.write_json(&response),
        OutputMode::JsonLines => OperationResponse::complete("setup.check", &response)
            .map(TerminalEventResponse::new)
            .map_err(crate::output::OutputError::Serialization)
            .and_then(|event| writer.write_json(&event)),
    };

    if let Err(error) = output_result {
        writer.write_safe_diagnostic(&error.to_string());
        return ProcessExit::StorageOrIo;
    }

    setup_exit(diagnosis.readiness)
}

#[derive(Serialize)]
struct SetupPlanResponse {
    profile: &'static str,
    readiness: &'static str,
    verification_scope: &'static str,
    target: &'static str,
    local_asr_model: SetupPlanModelResponse,
    managed_install: &'static str,
    catalogue_revision: Option<String>,
    stop_new_plans_at: Option<String>,
    plan_digest: Option<String>,
    actions: Vec<SetupPlanActionResponse>,
    dependencies: Vec<SetupPlanDependencyResponse>,
}

#[derive(Serialize)]
struct SetupPlanDependencyResponse {
    dependency: &'static str,
    status: &'static str,
    disposition: &'static str,
    required_authority: Option<&'static str>,
    next_step: &'static str,
}

#[derive(Serialize)]
struct SetupPlanModelResponse {
    status: &'static str,
    disposition: &'static str,
    required_authority: Option<&'static str>,
    next_step: &'static str,
}

#[derive(Serialize)]
struct SetupPlanActionResponse {
    id: String,
    component: &'static str,
    version: String,
    publisher: String,
    source_url: String,
    bytes: u64,
    sha256: String,
    format: &'static str,
    archive_limits: Option<SetupArchiveLimitsResponse>,
    selected_files: Vec<SetupArchiveSelectionResponse>,
    archive_links: Vec<SetupArchiveLinkResponse>,
    runtime_copies: Vec<SetupRuntimeCopyResponse>,
    licence: String,
    notice_url: String,
    source_code_url: String,
    trust_limit: String,
    licence_scope: &'static str,
    destination: &'static str,
    permissions: &'static str,
    change: &'static str,
    required_authority: &'static str,
    files: Vec<SetupPlanFileResponse>,
}

#[derive(Serialize)]
struct SetupPlanFileResponse {
    name: String,
    bytes: u64,
    sha256: String,
    mode: &'static str,
}

#[derive(Serialize)]
struct SetupArchiveLimitsResponse {
    max_stream_bytes: u64,
    entries: usize,
    expanded_bytes: u64,
}

#[derive(Serialize)]
struct SetupArchiveSelectionResponse {
    archive_path: String,
    runtime_name: String,
    bytes: u64,
    sha256: String,
}

#[derive(Serialize)]
struct SetupArchiveLinkResponse {
    archive_path: String,
    target: String,
}

#[derive(Serialize)]
struct SetupRuntimeCopyResponse {
    name: String,
    source_selected: String,
}

/// Inspects current selections and builds a deterministic reviewed managed plan.
pub(crate) async fn plan<P: DependencyProbe>(
    probe: P,
    profile: ExecutionProfile,
    target: ManagedTarget,
    selections: SetupSelectionState,
    now_unix_seconds: u64,
    catalogue: Option<AcceptedManagedCatalogue>,
) -> Result<OperationResponse<serde_json::Value>, FailureCode> {
    let plan = plan_managed_setup(
        match profile {
            ExecutionProfile::Desktop => SetupProfile::Desktop,
            ExecutionProfile::Worker => SetupProfile::Worker,
        },
        DiagnoseRuntime::new(probe).execute().await,
        target,
        selections,
        now_unix_seconds,
        catalogue,
    );
    let dependencies = plan
        .dependencies
        .iter()
        .map(|(status, disposition)| {
            let (required_authority, next_step) = match disposition {
                SetupDependencyDisposition::ExistingProbeOnly => (
                    None,
                    "Keep this executable selected and verify provider compatibility before use.",
                ),
                SetupDependencyDisposition::ManagedInstall => (
                    Some("user"),
                    "Review the exact managed action and its digest. Setup install remains unavailable until the complete installer qualifies.",
                ),
                SetupDependencyDisposition::ManualSelection => (
                    Some("user"),
                    manual_plan_step(status.dependency),
                ),
            };
            SetupPlanDependencyResponse {
                dependency: status.dependency.identifier(),
                status: status.state.identifier(),
                disposition: disposition.identifier(),
                required_authority,
                next_step,
            }
        })
        .collect();
    let model = model_response(plan.model);
    let actions = plan.actions.iter().map(action_response).collect();
    OperationResponse::complete(
        "setup.plan",
        &SetupPlanResponse {
            profile: profile.identifier(),
            readiness: plan.readiness.identifier(),
            verification_scope: "executable_probe_and_reviewed_catalogue",
            target: plan.target.identifier(),
            local_asr_model: model,
            managed_install: plan.availability.identifier(),
            catalogue_revision: plan.catalogue_revision,
            stop_new_plans_at: plan.stop_new_plans_date,
            plan_digest: plan.digest,
            actions,
            dependencies,
        },
    )
    .map_err(|_| FailureCode::Internal)
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
        status,
        disposition: disposition.identifier(),
        required_authority,
        next_step,
    }
}

fn action_response(action: &ManagedSetupAction) -> SetupPlanActionResponse {
    SetupPlanActionResponse {
        id: action.id.clone(),
        component: action.artifact.component.identifier(),
        version: action.artifact.version.clone(),
        publisher: action.artifact.publisher.clone(),
        source_url: action.artifact.source_url.clone(),
        bytes: action.artifact.integrity.bytes(),
        sha256: action.artifact.integrity.sha256_hex(),
        format: action.artifact.format.identifier(),
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
        licence_scope: "disclosure_not_legal_clearance",
        destination: "private_per_user_managed_runtime",
        permissions: "private_user_only",
        change: "planned_download_verify_extract_smoke_activate",
        required_authority: "user",
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
                },
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

#[derive(Serialize)]
struct ConfiguredSelectionResponse {
    dependency: &'static str,
    source: &'static str,
    validation: &'static str,
    next_step: &'static str,
}

/// Persists one explicit BYO executable selection without running it.
pub(crate) fn configure(
    arguments: &SetupConfigureArguments,
) -> Result<OperationResponse<serde_json::Value>, FailureCode> {
    let dependency = arguments.dependency.into();
    let store =
        UserDependencyConfigStore::default_location().map_err(crate::setup_config_failure)?;
    store
        .configure(dependency, &arguments.executable)
        .map_err(crate::setup_config_failure)?;
    OperationResponse::complete(
        "setup.configure",
        &ConfiguredSelectionResponse {
            dependency: dependency.identifier(),
            source: "configured_user_path",
            validation: "canonical_file_only",
            next_step: "Run setup check to probe the selected executable; model and provider compatibility remain unverified.",
        },
    )
    .map_err(|_| FailureCode::Internal)
}

#[derive(Serialize)]
struct ConfiguredModelResponse {
    source: &'static str,
    validation: &'static str,
    next_step: &'static str,
}

/// Persists one BYO model file path without reading its contents or running ASR.
pub(crate) fn configure_model(
    arguments: &SetupConfigureModelArguments,
) -> Result<OperationResponse<serde_json::Value>, FailureCode> {
    let store =
        UserDependencyConfigStore::default_location().map_err(crate::setup_config_failure)?;
    store
        .configure_model(&arguments.file)
        .map_err(crate::setup_config_failure)?;
    OperationResponse::complete(
        "setup.configure-model",
        &ConfiguredModelResponse {
            source: "configured_user_path",
            validation: "canonical_nonempty_file_only",
            next_step: "Model format and provider compatibility remain unverified; setup check still probes executables only.",
        },
    )
    .map_err(|_| FailureCode::Internal)
}

fn human_result(
    diagnosis: &RuntimeDiagnosis,
    profile: ExecutionProfile,
    selections: &ExplicitProbePaths,
    per_call: &ExplicitProbePaths,
) -> String {
    let mut result = format!(
        "VSift setup check\nProfile: {}\nStatus: {}\n",
        profile.identifier(),
        diagnosis.readiness.identifier()
    );
    for status in &diagnosis.dependencies {
        let (marker, detail) = human_state(
            &status.state,
            selections.for_dependency(status.dependency).is_some(),
        );
        result.push('[');
        result.push_str(marker);
        result.push_str("] ");
        result.push_str(status.dependency.display_name());
        result.push_str(" (");
        result.push_str(status.dependency.capability().identifier());
        result.push_str("): ");
        result.push_str(&detail);
        if per_call.for_dependency(status.dependency).is_some() {
            result.push_str(" [per-call path]");
        } else if selections.for_dependency(status.dependency).is_some() {
            result.push_str(" [configured user path]");
        } else {
            result.push_str(" [filtered PATH]");
        }
        result.push('\n');
        if !status.state.is_available() {
            result.push_str("  Install or locate this trusted tool, then rerun setup check with its absolute path using ");
            result.push_str(explicit_path_option(status.dependency));
            result.push_str(". Managed installation is not yet qualified for this target.\n");
        }
    }
    if diagnosis.readiness == RuntimeReadiness::Blocked {
        result.push_str("Media executable probes are blocked until FFmpeg and FFprobe respond.\n");
    }
    result.push_str("This check only probes executable responses; it does not validate provider compatibility or a local ASR model. A supplied transcript can avoid local ASR.\n");
    result
}

fn human_state(state: &DependencyState, explicit: bool) -> (&'static str, String) {
    match state {
        DependencyState::Available { version } => ("ok", sanitize_untrusted_text(version, 240)),
        DependencyState::Missing => (
            "missing",
            String::from(if explicit {
                "explicit path not found"
            } else {
                "not found on PATH"
            }),
        ),
        DependencyState::Unhealthy { .. } => ("unhealthy", String::from("dependency probe failed")),
        DependencyState::TimedOut => ("timeout", String::from("probe exceeded its deadline")),
    }
}

#[cfg(test)]
mod tests {
    use std::future::ready;

    use vsift_application::{DependencyProbe, SetupSelectionState};
    use vsift_domain::{DependencyState, DependencyStatus, ManagedTarget, RuntimeDependency};
    use vsift_infrastructure::{ExplicitProbePaths, accepted_ubuntu_catalogue};

    use super::{plan, run_setup_check};
    use crate::{
        command::ExecutionProfile,
        output::{OutputMode, OutputWriter, ProcessExit},
    };

    struct FixedProbe {
        state: DependencyState,
    }

    struct MissingOneProbe {
        missing: RuntimeDependency,
    }

    impl DependencyProbe for FixedProbe {
        fn probe(
            &self,
            dependency: RuntimeDependency,
        ) -> impl Future<Output = DependencyStatus> + Send {
            ready(DependencyStatus {
                dependency,
                state: self.state.clone(),
            })
        }
    }

    impl DependencyProbe for MissingOneProbe {
        fn probe(
            &self,
            dependency: RuntimeDependency,
        ) -> impl Future<Output = DependencyStatus> + Send {
            let state = if dependency == self.missing {
                DependencyState::Missing
            } else {
                DependencyState::Available {
                    version: String::from("fixture 1"),
                }
            };
            ready(DependencyStatus { dependency, state })
        }
    }

    #[tokio::test]
    async fn setup_json_is_deterministic_for_ready_degraded_and_blocked_states()
    -> Result<(), Box<dyn std::error::Error>> {
        for (state, expected_exit, expected_status) in [
            (
                DependencyState::Available {
                    version: String::from("fixture 1"),
                },
                ProcessExit::Success,
                "ready",
            ),
            (
                DependencyState::Missing,
                ProcessExit::UsageOrCapability,
                "blocked",
            ),
        ] {
            let mut stdout = Vec::new();
            let mut writer = OutputWriter::new(&mut stdout, Vec::<u8>::new());
            let exit = run_setup_check(
                FixedProbe { state },
                ExecutionProfile::Desktop,
                OutputMode::Json,
                &ExplicitProbePaths::default(),
                &ExplicitProbePaths::default(),
                &mut writer,
            )
            .await;
            let value: serde_json::Value = serde_json::from_slice(&stdout)?;

            assert_eq!(exit, expected_exit);
            assert_eq!(value["status"], expected_status);
            assert_eq!(value["dependencies"].as_array().map(Vec::len), Some(3));
        }

        let mut stdout = Vec::new();
        let mut writer = OutputWriter::new(&mut stdout, Vec::<u8>::new());
        let exit = run_setup_check(
            MissingOneProbe {
                missing: RuntimeDependency::Whisper,
            },
            ExecutionProfile::Desktop,
            OutputMode::Json,
            &ExplicitProbePaths::default(),
            &ExplicitProbePaths::default(),
            &mut writer,
        )
        .await;
        let value: serde_json::Value = serde_json::from_slice(&stdout)?;

        assert_eq!(exit, ProcessExit::Success);
        assert_eq!(value["status"], "degraded");
        Ok(())
    }

    #[tokio::test]
    async fn untrusted_probe_controls_do_not_reach_human_terminal()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut stdout = Vec::new();
        let mut writer = OutputWriter::new(&mut stdout, Vec::<u8>::new());

        let exit = run_setup_check(
            FixedProbe {
                state: DependencyState::Available {
                    version: String::from("fake\u{1b}]8;;file:///secret\u{7}version"),
                },
            },
            ExecutionProfile::Desktop,
            OutputMode::Human,
            &ExplicitProbePaths::default(),
            &ExplicitProbePaths::default(),
            &mut writer,
        )
        .await;

        assert_eq!(exit, ProcessExit::Success);
        assert!(String::from_utf8_lossy(&stdout).contains("Profile: desktop"));
        assert!(!stdout.contains(&0x1b));
        assert!(!stdout.contains(&0x07));
        Ok(())
    }

    #[tokio::test]
    async fn accepted_ubuntu_plan_is_reviewable_and_matches_public_schema()
    -> Result<(), Box<dyn std::error::Error>> {
        let response = plan(
            FixedProbe {
                state: DependencyState::Missing,
            },
            ExecutionProfile::Desktop,
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_800_000_000,
            Some(accepted_ubuntu_catalogue()?),
        )
        .await
        .map_err(|error| std::io::Error::other(format!("plan failed: {error:?}")))?;
        let value = serde_json::to_value(response)?;
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../schemas/v1/setup-plan.schema.json"))?;
        jsonschema::validator_for(&schema)?
            .validate(&value)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let data = &value["data"];
        assert_eq!(
            data["managed_install"],
            "catalogue_accepted_install_pending"
        );
        assert_eq!(data["target"], "ubuntu_24_04_x86_64");
        assert_eq!(data["actions"].as_array().map(Vec::len), Some(3));
        assert_eq!(
            data["actions"][0]["files"].as_array().map(Vec::len),
            Some(3)
        );
        assert_eq!(
            data["actions"][1]["files"].as_array().map(Vec::len),
            Some(12)
        );
        assert_eq!(
            data["actions"][1]["archive_links"].as_array().map(Vec::len),
            Some(8)
        );
        assert_eq!(
            data["actions"][1]["runtime_copies"]
                .as_array()
                .map(Vec::len),
            Some(6)
        );
        assert!(
            data["actions"][2]["source_url"]
                .as_str()
                .is_some_and(|url| url.contains("/resolve/80da2d8b"))
        );
        assert_eq!(data["plan_digest"].as_str().map(str::len), Some(64));
        Ok(())
    }

    #[tokio::test]
    async fn unaccepted_target_never_offers_managed_actions()
    -> Result<(), Box<dyn std::error::Error>> {
        let response = plan(
            FixedProbe {
                state: DependencyState::Missing,
            },
            ExecutionProfile::Worker,
            ManagedTarget::MacOsArm64,
            SetupSelectionState::default(),
            1_800_000_000,
            Some(accepted_ubuntu_catalogue()?),
        )
        .await
        .map_err(|error| std::io::Error::other(format!("plan failed: {error:?}")))?;
        let value = serde_json::to_value(response)?;
        assert_eq!(value["data"]["managed_install"], "unavailable_target");
        assert!(
            value["data"]["actions"]
                .as_array()
                .is_some_and(Vec::is_empty)
        );
        assert!(value["data"]["plan_digest"].is_null());
        Ok(())
    }
}
