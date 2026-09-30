//! Setup command presentation and saved-plan input.
//!
//! The engine performs every setup operation; this module writes the check's
//! result as v1 JSON or, through `human::setup_check`, as human text, and
//! reads the saved plan a user supplies back.

use std::{io::Write, path::Path};

use vsift::{
    Cancellation, Engine, FailureCode, LocalAsrSetupStatus, MANAGED_INSTALL_RETRY_AFTER_MS,
    ManagedPlanAvailability, ManagedRemovalTarget, ProgressObserver, RuntimeDependency,
    RuntimeDiagnosis, SetupInstallRequest,
};
use vsift_contract::{
    CommandName, DependencyLookup, JsonLimits, OperationResponse, STALE_PLAN_REMEDIATION,
    SavedSetupPlan, SetupCheckResponse, SetupInstallResponse, SetupListResponse,
    SetupRemoveResponse, SetupRepairResponse, SetupRollbackResponse, TerminalEventResponse,
    setup_install_failure_summary, setup_remove_failure_summary,
};

use crate::{
    CommandFailure,
    command::{
        ExecutionProfile, SetupInstallArguments, SetupRemoveArguments, SetupRollbackArguments,
    },
    config::{ConfigLayer, EffectiveConfig, HostPolicy},
    json_input::read_json_file,
    output::{OutputMode, OutputWriter, ProcessExit, setup_exit},
};

/// Writes one completed setup check and returns its compatible exit status.
///
/// `lookup` reports where the executable probed for each dependency came from.
/// The exit status still depends only on the executable probes: a local-ASR
/// verification that did not pass is reported, not a failed check, because
/// media inspection and supplied transcripts need no speech recognition.
pub(crate) fn present_setup_check<L, StandardOutput, StandardError>(
    diagnosis: &RuntimeDiagnosis,
    local_asr: LocalAsrSetupStatus,
    managed_install: ManagedPlanAvailability,
    profile: ExecutionProfile,
    mode: OutputMode,
    lookup: L,
    writer: &mut OutputWriter<StandardOutput, StandardError>,
) -> ProcessExit
where
    L: Fn(RuntimeDependency) -> DependencyLookup,
    StandardOutput: Write,
    StandardError: Write,
{
    let response = SetupCheckResponse::new(
        diagnosis,
        profile.into(),
        &lookup,
        managed_install,
        &local_asr,
    );
    let output_result = match mode {
        OutputMode::Human => {
            match crate::human::setup_check(diagnosis, local_asr, managed_install, profile, &lookup)
            {
                Ok(text) => writer.write_rendered_stdout(&text),
                Err(_) => Err(crate::output::OutputError::TooLarge),
            }
        }
        OutputMode::Json => writer.write_json(&response),
        OutputMode::JsonLines => {
            OperationResponse::complete(CommandName::SetupCheck.identifier(), &response)
                .map(TerminalEventResponse::new)
                .map_err(crate::output::OutputError::Serialization)
                .and_then(|event| writer.write_json(&event))
        }
    };

    if let Err(error) = output_result {
        writer.write_safe_diagnostic(&error.to_string());
        return ProcessExit::StorageOrIo;
    }

    setup_exit(diagnosis.readiness)
}

/// Reads one bounded, strict `setup plan --json` result supplied for acceptance.
pub(crate) fn read_saved_plan(path: &Path) -> Result<SavedSetupPlan, FailureCode> {
    let plan: SavedSetupPlan =
        read_json_file(path, JsonLimits::DOCUMENT).map_err(|error| error.code())?;
    plan.validate_envelope()?;
    Ok(plan)
}

/// `setup install`: reads the saved plan, then asks the engine to take the
/// install guard, revalidate the plan against the machine and the accepted
/// digest, and install its components. A transaction that stops at a
/// component is a failure with that component's code and remediation, and
/// with every component's outcome as the failure's data.
pub(crate) async fn install(
    engine: &Engine,
    arguments: SetupInstallArguments,
    cancellation: &Cancellation,
    progress: ProgressObserver,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let saved = read_saved_plan(&arguments.plan).map_err(|code| match code {
        FailureCode::InvalidArgument => {
            CommandFailure::with_remediation(code, STALE_PLAN_REMEDIATION.to_owned())
        }
        other => CommandFailure::from(other),
    })?;
    let profile = saved.profile().map_err(|code| {
        CommandFailure::with_remediation(code, STALE_PLAN_REMEDIATION.to_owned())
    })?;
    let config = EffectiveConfig::resolve(
        ConfigLayer {
            profile: Some(ExecutionProfile::from(profile)),
            probe_timeout_seconds: None,
        },
        ConfigLayer::default(),
        ConfigLayer::default(),
        &HostPolicy::local_r0(),
    )
    .map_err(|_| CommandFailure::from(FailureCode::InvalidArgument))?;
    let outcome = engine
        .install_setup(
            SetupInstallRequest {
                saved_plan: saved,
                accepted_digest: arguments.accept_plan,
                artifact_directory: arguments.artifact_dir,
                probe_timeout: config.probe_timeout,
            },
            cancellation,
            progress,
        )
        .await?;
    let data = SetupInstallResponse::new(
        outcome.catalogue_revision().map(str::to_owned),
        outcome.source(),
        outcome.report(),
    )
    .with_cleanup(&outcome.cleanup().stages, &outcome.cleanup().versions);
    match outcome.report().first_failure() {
        None => OperationResponse::complete(CommandName::SetupInstall.identifier(), &data)
            .map_err(|_| CommandFailure::from(FailureCode::Internal)),
        Some((component, failure)) => {
            let code = failure
                .reason
                .failure_code()
                .unwrap_or(FailureCode::Internal);
            let data = serde_json::to_value(&data)
                .map_err(|_| CommandFailure::from(FailureCode::Internal))?;
            Err(CommandFailure::with_remediation(
                code,
                setup_install_failure_summary(component, failure),
            )
            .with_data(data))
        }
    }
}

fn completed<T: serde::Serialize>(
    command: CommandName,
    data: &T,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    OperationResponse::complete(command.identifier(), data)
        .map_err(|_| CommandFailure::from(FailureCode::Internal))
}

/// `setup list`: every managed component, its selected and previous
/// versions and whether each version verifies. Reads only.
pub(crate) fn list(
    engine: &Engine,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let listing = engine.list_managed()?;
    completed(
        CommandName::SetupList,
        &SetupListResponse::new(listing.availability(), listing.inventory()),
    )
}

/// `setup repair`: the diagnosis and the plan of existing commands that
/// fixes it. Reads only; a store that needs repair is a complete result.
pub(crate) fn repair(
    engine: &Engine,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let diagnosis = engine.repair_managed()?;
    completed(
        CommandName::SetupRepair,
        &SetupRepairResponse::new(diagnosis.availability(), diagnosis.plan()),
    )
}

/// `setup rollback`: selects the previous or the named verified version.
pub(crate) fn rollback(
    engine: &Engine,
    arguments: &SetupRollbackArguments,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let outcome =
        engine.rollback_managed(arguments.component.into(), arguments.version.as_ref())?;
    completed(
        CommandName::SetupRollback,
        &SetupRollbackResponse::new(&outcome),
    )
}

/// `setup remove`: removes a version, a component or abandoned stages. What
/// could not be removed makes the result a failure whose data reports every
/// item, like a failed `setup install`.
pub(crate) fn remove(
    engine: &Engine,
    arguments: SetupRemoveArguments,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let target = match (arguments.component, arguments.version) {
        (Some(component), Some(version)) => ManagedRemovalTarget::Version {
            component: component.into(),
            version,
        },
        (Some(component), None) => ManagedRemovalTarget::Component(component.into()),
        // The parser requires a component or `--stale-stages`, and
        // `--version` only with a component.
        (None, _) => ManagedRemovalTarget::StaleStages,
    };
    let report = engine.remove_managed(&target)?;
    let data = SetupRemoveResponse::new(&target, &report);
    match report.failure_code() {
        None => completed(CommandName::SetupRemove, &data),
        Some(code) => {
            let data = serde_json::to_value(&data)
                .map_err(|_| CommandFailure::from(FailureCode::Internal))?;
            let failure = CommandFailure::with_remediation(
                code,
                setup_remove_failure_summary(code).to_owned(),
            )
            .with_data(data);
            Err(if code == FailureCode::Busy {
                failure.with_retry_after_ms(MANAGED_INSTALL_RETRY_AFTER_MS)
            } else {
                failure
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use vsift::{
        AsrFailure, AsrFailureReason, AsrStage, DependencyState, DependencyStatus,
        LocalAsrCheckFailure, LocalAsrCheckOutcome, LocalAsrModelStatus, LocalAsrNotRunReason,
        LocalAsrSetupStatus, LocalAsrVerificationFailure, LocalAsrVerificationSource,
        ManagedPlanAvailability, ReviewedAsrModel, RuntimeDependency, RuntimeDiagnosis,
        RuntimeReadiness,
    };
    use vsift_contract::DependencyLookup;

    use super::present_setup_check;

    const NOTHING_REGISTERED: LocalAsrSetupStatus = LocalAsrSetupStatus {
        model: LocalAsrModelStatus::NotSelected,
        verification: LocalAsrCheckOutcome::NotRun(LocalAsrNotRunReason::MediaToolsUnavailable),
    };
    use crate::{
        command::ExecutionProfile,
        output::{OutputMode, OutputWriter, ProcessExit},
    };

    fn diagnosis(readiness: RuntimeReadiness, state: &DependencyState) -> RuntimeDiagnosis {
        RuntimeDiagnosis {
            readiness,
            dependencies: RuntimeDependency::ALL
                .into_iter()
                .map(|dependency| DependencyStatus {
                    dependency,
                    state: state.clone(),
                })
                .collect(),
        }
    }

    const fn filtered_path(_dependency: RuntimeDependency) -> DependencyLookup {
        DependencyLookup::FilteredPath
    }

    #[test]
    fn setup_json_is_deterministic_for_ready_degraded_and_blocked_states()
    -> Result<(), Box<dyn std::error::Error>> {
        let available = DependencyState::Available {
            version: String::from("fixture 1"),
        };
        for (readiness, state, expected_exit, expected_status) in [
            (
                RuntimeReadiness::Ready,
                &available,
                ProcessExit::Success,
                "ready",
            ),
            (
                RuntimeReadiness::Degraded,
                &available,
                ProcessExit::Success,
                "degraded",
            ),
            (
                RuntimeReadiness::Blocked,
                &DependencyState::Missing,
                ProcessExit::UsageOrCapability,
                "blocked",
            ),
        ] {
            let mut stdout = Vec::new();
            let mut writer = OutputWriter::new(&mut stdout, Vec::<u8>::new());
            let exit = present_setup_check(
                &diagnosis(readiness, state),
                NOTHING_REGISTERED,
                ManagedPlanAvailability::Qualified,
                ExecutionProfile::Desktop,
                OutputMode::Json,
                filtered_path,
                &mut writer,
            );
            let value: serde_json::Value = serde_json::from_slice(&stdout)?;

            assert_eq!(exit, expected_exit);
            assert_eq!(value["status"], expected_status);
            assert_eq!(value["dependencies"].as_array().map(Vec::len), Some(3));
        }
        Ok(())
    }

    #[test]
    fn untrusted_probe_controls_do_not_reach_human_terminal() {
        let mut stdout = Vec::new();
        let mut writer = OutputWriter::new(&mut stdout, Vec::<u8>::new());

        let exit = present_setup_check(
            &diagnosis(
                RuntimeReadiness::Ready,
                &DependencyState::Available {
                    version: String::from("fake\u{1b}]8;;file:///secret\u{7}version"),
                },
            ),
            NOTHING_REGISTERED,
            ManagedPlanAvailability::Qualified,
            ExecutionProfile::Desktop,
            OutputMode::Human,
            filtered_path,
            &mut writer,
        );

        assert_eq!(exit, ProcessExit::Success);
        assert!(String::from_utf8_lossy(&stdout).contains("Profile: desktop"));
        assert!(!stdout.contains(&0x1b));
        assert!(!stdout.contains(&0x07));
    }

    #[test]
    fn human_output_names_where_each_probed_executable_came_from() {
        let mut stdout = Vec::new();
        let mut writer = OutputWriter::new(&mut stdout, Vec::<u8>::new());

        present_setup_check(
            &diagnosis(RuntimeReadiness::Blocked, &DependencyState::Missing),
            NOTHING_REGISTERED,
            ManagedPlanAvailability::Qualified,
            ExecutionProfile::Worker,
            OutputMode::Human,
            |dependency| match dependency {
                RuntimeDependency::Ffmpeg => DependencyLookup::ExplicitPath,
                RuntimeDependency::Ffprobe => DependencyLookup::ConfiguredUserPath,
                RuntimeDependency::Whisper => DependencyLookup::FilteredPath,
            },
            &mut writer,
        );
        let text = String::from_utf8_lossy(&stdout);

        assert!(text.contains("explicit path not found [per-call path]"));
        assert!(text.contains("explicit path not found [configured user path]"));
        assert!(text.contains("not found on PATH [filtered PATH]"));
    }

    /// D4: the human report names the model profile and the verification
    /// outcome with typed identifiers only, and a failed verification does
    /// not change the probe-based exit status.
    #[test]
    fn human_output_reports_the_local_asr_model_and_verification() {
        let ready = diagnosis(
            RuntimeReadiness::Ready,
            &DependencyState::Available {
                version: String::from("fixture 1"),
            },
        );
        for (local_asr, model, verification) in [
            (
                LocalAsrSetupStatus {
                    model: LocalAsrModelStatus::KnownPinned(ReviewedAsrModel::BaseQ5_1),
                    verification: LocalAsrCheckOutcome::Verified(
                        LocalAsrVerificationSource::Recorded,
                    ),
                },
                "Local ASR model: reviewed base_q5_1 profile",
                "Local ASR check: verified (recorded pass)",
            ),
            (
                LocalAsrSetupStatus {
                    model: LocalAsrModelStatus::KnownPinned(ReviewedAsrModel::Base),
                    verification: LocalAsrCheckOutcome::Failed(LocalAsrCheckFailure::Verification(
                        LocalAsrVerificationFailure::Transcription(AsrFailure {
                            stage: AsrStage::Recognition,
                            reason: AsrFailureReason::Deadline,
                        }),
                    )),
                },
                "Local ASR model: reviewed base profile",
                "Local ASR check: failed at the recognition step (deadline)",
            ),
            (
                LocalAsrSetupStatus {
                    model: LocalAsrModelStatus::Unrecognised,
                    verification: LocalAsrCheckOutcome::NotRun(
                        LocalAsrNotRunReason::ModelNotPinned,
                    ),
                },
                "not a reviewed model",
                "Local ASR check: not run: the registered model is not a reviewed pinned profile",
            ),
        ] {
            let mut stdout = Vec::new();
            let mut writer = OutputWriter::new(&mut stdout, Vec::<u8>::new());
            let exit = present_setup_check(
                &ready,
                local_asr,
                ManagedPlanAvailability::Qualified,
                ExecutionProfile::Desktop,
                OutputMode::Human,
                filtered_path,
                &mut writer,
            );
            let text = String::from_utf8_lossy(&stdout);
            assert_eq!(exit, ProcessExit::Success);
            assert!(text.contains(model), "{text}");
            assert!(text.contains(verification), "{text}");
        }
    }
}
