//! Runtime dependency probing through the secure process supervisor.

use std::{ffi::OsStr, time::Duration};

use tokio::time::Instant;
use vsift_application::DependencyProbe;
use vsift_domain::{DependencyState, DependencyStatus, RuntimeDependency};

use crate::{
    ExecutableResolutionError, ExecutableResolver, ProcessCancellation, ProcessError,
    ProcessRequest, ProcessSupervisor, ProcessWorkingDirectory, TerminationReason,
};

const MAX_DIAGNOSTIC_LENGTH: usize = 240;

/// Probes approved runtime executables without invoking a command shell.
///
/// One instance represents one setup-check operation. Its deadline is shared across every
/// dependency probe, preventing a sequence of slow providers from multiplying the caller's
/// configured operation timeout.
pub struct ProcessDependencyProbe {
    operation_deadline: Instant,
    resolver: ExecutableResolver,
    supervisor: ProcessSupervisor,
}

impl ProcessDependencyProbe {
    /// Creates a probe with one total deadline and a safely filtered snapshot of `PATH`.
    #[must_use]
    pub fn new(operation_timeout: Duration) -> Self {
        Self::configured(
            operation_timeout,
            ExecutableResolver::from_current_path(),
            ProcessSupervisor::default(),
        )
    }

    /// Creates a probe with explicit resolution and supervision policies.
    #[must_use]
    pub fn configured(
        operation_timeout: Duration,
        resolver: ExecutableResolver,
        supervisor: ProcessSupervisor,
    ) -> Self {
        let now = Instant::now();
        let operation_deadline = match now.checked_add(operation_timeout) {
            Some(deadline) => deadline,
            None => now,
        };
        Self {
            operation_deadline,
            resolver,
            supervisor,
        }
    }
}

impl Default for ProcessDependencyProbe {
    fn default() -> Self {
        Self::new(Duration::from_secs(5))
    }
}

impl DependencyProbe for ProcessDependencyProbe {
    async fn probe(&self, dependency: RuntimeDependency) -> DependencyStatus {
        let specification = ProbeSpecification::for_dependency(dependency);
        let state = self.probe_process(specification).await;
        DependencyStatus { dependency, state }
    }
}

impl ProcessDependencyProbe {
    async fn probe_process(&self, specification: ProbeSpecification) -> DependencyState {
        let executable = match self.resolver.resolve(OsStr::new(specification.executable)) {
            Ok(executable) => executable,
            Err(ExecutableResolutionError::NotFound) => return DependencyState::Missing,
            Err(error) => {
                return DependencyState::Unhealthy {
                    message: truncate(&error.to_string()),
                };
            }
        };
        let Some(working_directory_path) = executable.path().parent() else {
            return DependencyState::Unhealthy {
                message: String::from("provider executable has no working directory"),
            };
        };
        let working_directory = match ProcessWorkingDirectory::new(working_directory_path) {
            Ok(directory) => directory,
            Err(error) => {
                return DependencyState::Unhealthy {
                    message: truncate(&error.to_string()),
                };
            }
        };
        let Some(remaining) = self
            .operation_deadline
            .checked_duration_since(Instant::now())
        else {
            return DependencyState::TimedOut;
        };
        if remaining.is_zero() {
            return DependencyState::TimedOut;
        }
        let request = match ProcessRequest::new(executable, working_directory, remaining) {
            Ok(request) => request.with_arguments(specification.arguments.iter().copied()),
            Err(error) => {
                return DependencyState::Unhealthy {
                    message: truncate(&error.to_string()),
                };
            }
        };

        match self
            .supervisor
            .run(request, ProcessCancellation::new())
            .await
        {
            Ok(outcome) if outcome.termination == TerminationReason::Deadline => {
                DependencyState::TimedOut
            }
            Ok(outcome) if outcome.status.success() => DependencyState::Available {
                version: first_non_empty_line(&outcome.stdout.bytes, &outcome.stderr.bytes),
            },
            Ok(outcome) => DependencyState::Unhealthy {
                message: process_failure_message(outcome.termination, outcome.status),
            },
            Err(ProcessError::Spawn(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                DependencyState::Missing
            }
            Err(error) => DependencyState::Unhealthy {
                message: truncate(&error.to_string()),
            },
        }
    }
}

#[derive(Clone, Copy)]
struct ProbeSpecification {
    executable: &'static str,
    arguments: &'static [&'static str],
}

impl ProbeSpecification {
    const fn for_dependency(dependency: RuntimeDependency) -> Self {
        match dependency {
            RuntimeDependency::Ffmpeg => Self {
                executable: "ffmpeg",
                arguments: &["-version"],
            },
            RuntimeDependency::Ffprobe => Self {
                executable: "ffprobe",
                arguments: &["-version"],
            },
            RuntimeDependency::Whisper => Self {
                executable: "whisper-cli",
                arguments: &["--help"],
            },
        }
    }
}

fn process_failure_message(
    termination: TerminationReason,
    status: std::process::ExitStatus,
) -> String {
    match termination {
        TerminationReason::OutputLimit(_) => {
            String::from("provider output exceeded the diagnostic limit")
        }
        TerminationReason::Cancelled => String::from("provider probe was cancelled"),
        TerminationReason::Deadline => String::from("provider probe exceeded its deadline"),
        TerminationReason::Exited => truncate(&format!("process exited with {status}")),
    }
}

fn first_non_empty_line(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout_text = String::from_utf8_lossy(stdout);
    let stderr_text = String::from_utf8_lossy(stderr);

    stdout_text
        .lines()
        .chain(stderr_text.lines())
        .find(|line| !line.trim().is_empty())
        .map_or_else(|| String::from("detected"), |line| truncate(line.trim()))
}

fn truncate(value: &str) -> String {
    value.chars().take(MAX_DIAGNOSTIC_LENGTH).collect()
}

#[cfg(test)]
mod tests {
    use super::{MAX_DIAGNOSTIC_LENGTH, first_non_empty_line, truncate};

    #[test]
    fn returns_first_non_empty_line_from_standard_output() {
        let result = first_non_empty_line(b"\nffmpeg version 1\nmore", b"ignored");

        assert_eq!(result, "ffmpeg version 1");
    }

    #[test]
    fn falls_back_to_standard_error() {
        let result = first_non_empty_line(b"", b"whisper help\nmore");

        assert_eq!(result, "whisper help");
    }

    #[test]
    fn bounds_diagnostics() {
        let input = "a".repeat(MAX_DIAGNOSTIC_LENGTH + 20);

        assert_eq!(truncate(&input).len(), MAX_DIAGNOSTIC_LENGTH);
    }
}
