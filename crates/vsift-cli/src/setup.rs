//! Setup command composition and presentation.

use std::io::Write;

use vsift_application::{DependencyProbe, DiagnoseRuntime, RuntimeDiagnosis};
use vsift_domain::{DependencyState, RuntimeReadiness};

use crate::{
    command::ExecutionProfile,
    output::{
        OperationResponse, OutputMode, OutputWriter, ProcessExit, SetupCheckResponse,
        TerminalEventResponse, sanitize_untrusted_text, setup_exit,
    },
};

/// Executes the read-only setup check with explicit dependencies and output streams.
pub(crate) async fn run_setup_check<P, StandardOutput, StandardError>(
    probe: P,
    profile: ExecutionProfile,
    mode: OutputMode,
    writer: &mut OutputWriter<StandardOutput, StandardError>,
) -> ProcessExit
where
    P: DependencyProbe,
    StandardOutput: Write,
    StandardError: Write,
{
    let diagnosis = DiagnoseRuntime::new(probe).execute().await;
    let response = SetupCheckResponse::new(&diagnosis, profile);
    let output_result = match mode {
        OutputMode::Human => writer.write_trusted_stdout(&human_result(&diagnosis, profile)),
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

fn human_result(diagnosis: &RuntimeDiagnosis, profile: ExecutionProfile) -> String {
    let mut result = format!(
        "VSift setup check\nProfile: {}\nStatus: {}\n",
        profile.identifier(),
        diagnosis.readiness.identifier()
    );
    for status in &diagnosis.dependencies {
        let (marker, detail) = human_state(&status.state);
        result.push('[');
        result.push_str(marker);
        result.push_str("] ");
        result.push_str(status.dependency.display_name());
        result.push_str(" (");
        result.push_str(status.dependency.capability().identifier());
        result.push_str("): ");
        result.push_str(&detail);
        result.push('\n');
    }
    if diagnosis.readiness == RuntimeReadiness::Blocked {
        result.push_str(
            "Installation assistance is not available yet; install the missing media dependencies and run this check again.\n",
        );
    }
    result
}

fn human_state(state: &DependencyState) -> (&'static str, String) {
    match state {
        DependencyState::Available { version } => ("ok", sanitize_untrusted_text(version, 240)),
        DependencyState::Missing => ("missing", String::from("not found on PATH")),
        DependencyState::Unhealthy { .. } => ("unhealthy", String::from("dependency probe failed")),
        DependencyState::TimedOut => ("timeout", String::from("probe exceeded its deadline")),
    }
}

#[cfg(test)]
mod tests {
    use std::future::ready;

    use vsift_application::DependencyProbe;
    use vsift_domain::{DependencyState, DependencyStatus, RuntimeDependency};

    use super::run_setup_check;
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
            &mut writer,
        )
        .await;

        assert_eq!(exit, ProcessExit::Success);
        assert!(String::from_utf8_lossy(&stdout).contains("Profile: desktop"));
        assert!(!stdout.contains(&0x1b));
        assert!(!stdout.contains(&0x07));
        Ok(())
    }
}
