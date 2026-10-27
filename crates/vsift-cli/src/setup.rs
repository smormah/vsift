//! Setup command presentation and saved-plan input.
//!
//! The engine performs every setup operation; this module writes the check's
//! result as v1 JSON or, through `human::setup_check`, as human text, and
//! reads the saved plan a user supplies back.

use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use vsift::{
    Cancellation, Engine, FailureCode, LocalAsrSetupStatus, MANAGED_INSTALL_RETRY_AFTER_MS,
    ManagedPlanAvailability, ManagedRemovalTarget, ManagedUpdate, OptionalCapability,
    OptionalComponent, ProgressObserver, RuntimeDependency, RuntimeDiagnosis, SetupInstallRequest,
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

/// What a setup check found about managed installation on this host.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ManagedSetupFacts<'report> {
    /// Whether managed installation can supply a missing tool here.
    pub(crate) availability: ManagedPlanAvailability,
    /// The availability for each dependency, in `RuntimeDependency::ALL`
    /// order (ADR 0032: a catalogue may offer only some components).
    pub(crate) by_dependency: [ManagedPlanAvailability; 3],
    /// The managed versions the check opened that the catalogue supersedes.
    pub(crate) updates: &'report [ManagedUpdate],
    /// Whether each optional component is installed, in plan order.
    pub(crate) optional: &'report [(OptionalComponent, OptionalCapability)],
}

/// Writes one completed setup check and returns its compatible exit status.
///
/// `lookup` reports where the executable probed for each dependency came from.
/// The exit status still depends only on the executable probes: a local-ASR
/// verification that did not pass is reported, not a failed check, because
/// media inspection and supplied transcripts need no speech recognition.
pub(crate) fn present_setup_check<L, StandardOutput, StandardError>(
    diagnosis: &RuntimeDiagnosis,
    local_asr: LocalAsrSetupStatus,
    managed: ManagedSetupFacts<'_>,
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
    let ManagedSetupFacts {
        availability: managed_install,
        by_dependency,
        updates: managed_updates,
        optional,
    } = managed;
    let response = SetupCheckResponse::new(
        diagnosis,
        profile.into(),
        &lookup,
        managed_install,
        &local_asr,
    )
    .with_managed_install_for(|dependency| by_dependency[dependency_slot(dependency)])
    .with_managed_updates(managed_updates)
    .with_optional_components(optional, profile.into());
    let output_result = match mode {
        OutputMode::Human => {
            match crate::human::setup_check(diagnosis, local_asr, managed, profile, &lookup) {
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
/// The position of `dependency` in [`RuntimeDependency::ALL`], the order of
/// [`ManagedSetupFacts::by_dependency`].
pub(crate) const fn dependency_slot(dependency: RuntimeDependency) -> usize {
    match dependency {
        RuntimeDependency::Ffmpeg => 0,
        RuntimeDependency::Ffprobe => 1,
        RuntimeDependency::Whisper => 2,
    }
}

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
    install_saved(
        engine,
        saved,
        arguments.accept_plan,
        arguments.artifact_dir,
        config.probe_timeout,
        cancellation,
        progress,
    )
    .await
}

/// Installs a saved plan accepted by `accepted_digest`: the one install
/// path, shared by `setup install` and the setup door (ADR 0036), which
/// accepts the plan it showed by that plan's own digest.
pub(crate) async fn install_saved(
    engine: &Engine,
    saved: SavedSetupPlan,
    accepted_digest: String,
    artifact_directory: Option<PathBuf>,
    probe_timeout: Duration,
    cancellation: &Cancellation,
    progress: ProgressObserver,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let outcome = engine
        .install_setup(
            SetupInstallRequest {
                saved_plan: saved,
                accepted_digest,
                artifact_directory,
                probe_timeout,
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
        // The core is installed. When an optional component was not, the
        // result is still a success, `partial` with a warning so it is seen
        // (ADR 0032; exit 0).
        None => match data.optional_warning() {
            Some(warning) => {
                OperationResponse::partial(CommandName::SetupInstall.identifier(), &data, warning)
            }
            None => OperationResponse::complete(CommandName::SetupInstall.identifier(), &data),
        }
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
        ManagedPlanAvailability, OptionalCapability, OptionalComponent, ReviewedAsrModel,
        RuntimeDependency, RuntimeDiagnosis, RuntimeReadiness,
    };
    use vsift_contract::DependencyLookup;

    use super::{ManagedSetupFacts, present_setup_check};

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
                ManagedSetupFacts {
                    availability: ManagedPlanAvailability::Qualified,
                    by_dependency: [ManagedPlanAvailability::Qualified; 3],
                    updates: &[],
                    optional: &[(
                        OptionalComponent::OnlineLinks,
                        OptionalCapability::NotInstalled,
                    )],
                },
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
            ManagedSetupFacts {
                availability: ManagedPlanAvailability::Qualified,
                by_dependency: [ManagedPlanAvailability::Qualified; 3],
                updates: &[],
                optional: &[(
                    OptionalComponent::OnlineLinks,
                    OptionalCapability::NotInstalled,
                )],
            },
            ExecutionProfile::Desktop,
            OutputMode::Human,
            filtered_path,
            &mut writer,
        );

        assert_eq!(exit, ProcessExit::Success);
        assert!(String::from_utf8_lossy(&stdout).starts_with("VSift setup check\n"));
        assert!(!stdout.contains(&0x1b));
        assert!(!stdout.contains(&0x07));
    }

    #[test]
    fn human_output_says_when_a_path_that_was_given_is_not_found() {
        let mut stdout = Vec::new();
        let mut writer = OutputWriter::new(&mut stdout, Vec::<u8>::new());

        present_setup_check(
            &diagnosis(RuntimeReadiness::Blocked, &DependencyState::Missing),
            NOTHING_REGISTERED,
            ManagedSetupFacts {
                availability: ManagedPlanAvailability::Qualified,
                by_dependency: [ManagedPlanAvailability::Qualified; 3],
                updates: &[],
                optional: &[(
                    OptionalComponent::OnlineLinks,
                    OptionalCapability::NotInstalled,
                )],
            },
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

        assert!(
            text.contains("The path given for FFmpeg was not found.\n"),
            "{text}"
        );
        assert!(
            text.contains("The path given for FFprobe was not found.\n"),
            "{text}"
        );
        assert!(!text.contains("The path given for whisper.cpp"), "{text}");
        assert!(!text.contains("[per-call path]") && !text.contains("[filtered PATH]"));
    }

    /// The human report shows where speech stands from the model and the
    /// verification, in plain words, and a failed verification does not
    /// change the probe-based exit status.
    #[test]
    fn human_output_reports_the_speech_model_and_verification() {
        let ready = diagnosis(
            RuntimeReadiness::Ready,
            &DependencyState::Available {
                version: String::from("fixture 1"),
            },
        );
        for (local_asr, row, verdict) in [
            (
                LocalAsrSetupStatus {
                    model: LocalAsrModelStatus::KnownPinned(ReviewedAsrModel::BaseQ5_1),
                    verification: LocalAsrCheckOutcome::Verified(
                        LocalAsrVerificationSource::Recorded,
                    ),
                },
                "  Speech      ready ",
                "Setup complete: video and speech both work.",
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
                "  Speech      test failed ",
                "Your setup isn't complete: speech failed its test.",
            ),
            (
                LocalAsrSetupStatus {
                    model: LocalAsrModelStatus::Unrecognised,
                    verification: LocalAsrCheckOutcome::NotRun(
                        LocalAsrNotRunReason::ModelNotPinned,
                    ),
                },
                "  Speech      unknown model ",
                "Your setup isn't complete: the speech model is not one VSift knows.",
            ),
        ] {
            let mut stdout = Vec::new();
            let mut writer = OutputWriter::new(&mut stdout, Vec::<u8>::new());
            let exit = present_setup_check(
                &ready,
                local_asr,
                ManagedSetupFacts {
                    availability: ManagedPlanAvailability::Qualified,
                    by_dependency: [ManagedPlanAvailability::Qualified; 3],
                    updates: &[],
                    optional: &[(
                        OptionalComponent::OnlineLinks,
                        OptionalCapability::NotInstalled,
                    )],
                },
                ExecutionProfile::Desktop,
                OutputMode::Human,
                filtered_path,
                &mut writer,
            );
            let text = String::from_utf8_lossy(&stdout);
            assert_eq!(exit, ProcessExit::Success);
            assert!(text.contains(row), "{text}");
            assert!(text.contains(verdict), "{text}");
        }
    }
}
