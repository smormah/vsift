//! Setup command presentation and saved-plan input.
//!
//! The engine performs every setup operation; this module renders its results
//! as human text or v1 JSON and reads the saved plan a user supplies back.

use std::{io::Write, path::Path};

use vsift::{
    DependencyState, FailureCode, LocalAsrCheckOutcome, LocalAsrModelStatus, LocalAsrNotRunReason,
    LocalAsrSetupStatus, LocalAsrVerificationSource, RuntimeDependency, RuntimeDiagnosis,
    RuntimeReadiness,
};
use vsift_contract::{
    CommandName, DependencyLookup, MAX_PROVIDER_DETAIL_BYTES, OperationResponse, SavedSetupPlan,
    SetupCheckResponse, TerminalEventResponse, explicit_path_option, sanitize_untrusted_text,
};

use crate::{
    command::ExecutionProfile,
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
    let response = SetupCheckResponse::new(diagnosis, profile.into(), &lookup, &local_asr);
    let output_result = match mode {
        OutputMode::Human => {
            writer.write_trusted_stdout(&human_result(diagnosis, local_asr, profile, &lookup))
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
    let plan: SavedSetupPlan = read_json_file(path).map_err(|error| error.code())?;
    plan.validate_envelope()?;
    Ok(plan)
}

fn human_result<L>(
    diagnosis: &RuntimeDiagnosis,
    local_asr: LocalAsrSetupStatus,
    profile: ExecutionProfile,
    lookup: &L,
) -> String
where
    L: Fn(RuntimeDependency) -> DependencyLookup,
{
    let mut result = format!(
        "VSift setup check\nProfile: {}\nStatus: {}\n",
        profile.identifier(),
        diagnosis.readiness.identifier()
    );
    for status in &diagnosis.dependencies {
        let provenance = lookup(status.dependency);
        let (marker, detail) =
            human_state(&status.state, provenance != DependencyLookup::FilteredPath);
        result.push('[');
        result.push_str(marker);
        result.push_str("] ");
        result.push_str(status.dependency.display_name());
        result.push_str(" (");
        result.push_str(status.dependency.capability().identifier());
        result.push_str("): ");
        result.push_str(&detail);
        result.push_str(match provenance {
            DependencyLookup::ExplicitPath => " [per-call path]",
            DependencyLookup::ConfiguredUserPath => " [configured user path]",
            DependencyLookup::FilteredPath => " [filtered PATH]",
        });
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
    result.push_str(&human_local_asr(local_asr));
    result.push_str("Executable probes only show that each tool responds. The local ASR check transcribes a short speech clip built into VSift with the selected tools and model. A supplied transcript can avoid local ASR.\n");
    result
}

/// Fixed-prose lines for the local-ASR report; every value is a typed
/// identifier, never a path or tool output.
fn human_local_asr(local_asr: LocalAsrSetupStatus) -> String {
    let model = match local_asr.model {
        LocalAsrModelStatus::NotSelected => {
            String::from("not registered (setup configure-model --file <path>)")
        }
        LocalAsrModelStatus::Unreadable => String::from("registered file cannot be read"),
        LocalAsrModelStatus::Unrecognised => {
            String::from("registered file is not a reviewed model, so it will not run")
        }
        LocalAsrModelStatus::KnownPinned(profile) => {
            format!("reviewed {} profile", profile.identifier())
        }
    };
    let verification = match local_asr.verification {
        LocalAsrCheckOutcome::Verified(LocalAsrVerificationSource::Recorded) => {
            String::from("verified (recorded pass)")
        }
        LocalAsrCheckOutcome::Verified(LocalAsrVerificationSource::RanNow) => {
            String::from("verified (ran now)")
        }
        LocalAsrCheckOutcome::Failed(failure) => format!(
            "failed at the {} step ({}); local ASR will not run until it passes",
            failure.check(),
            failure.reason()
        ),
        LocalAsrCheckOutcome::NotRun(reason) => format!(
            "not run: {}",
            match reason {
                LocalAsrNotRunReason::MediaToolsUnavailable => {
                    "FFmpeg and FFprobe are needed and must pass their own check"
                }
                LocalAsrNotRunReason::WhisperUnavailable => "the whisper.cpp CLI is not available",
                LocalAsrNotRunReason::ModelNotSelected => "no model is registered",
                LocalAsrNotRunReason::ModelNotPinned => {
                    "the registered model is not a reviewed pinned profile"
                }
            }
        ),
    };
    format!("Local ASR model: {model}\nLocal ASR check: {verification}\n")
}

fn human_state(state: &DependencyState, explicit: bool) -> (&'static str, String) {
    match state {
        DependencyState::Available { version } => (
            "ok",
            sanitize_untrusted_text(version, MAX_PROVIDER_DETAIL_BYTES),
        ),
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
    use vsift::{
        AsrFailure, AsrFailureReason, AsrStage, DependencyState, DependencyStatus,
        LocalAsrCheckFailure, LocalAsrCheckOutcome, LocalAsrModelStatus, LocalAsrNotRunReason,
        LocalAsrSetupStatus, LocalAsrVerificationFailure, LocalAsrVerificationSource,
        ReviewedAsrModel, RuntimeDependency, RuntimeDiagnosis, RuntimeReadiness,
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
