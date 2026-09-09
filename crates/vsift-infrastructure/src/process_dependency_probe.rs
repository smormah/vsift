use std::{io, time::Duration};

use tokio::{process::Command, time::timeout};
use vsift_application::DependencyProbe;
use vsift_domain::{DependencyState, DependencyStatus, RuntimeDependency};

const MAX_DIAGNOSTIC_LENGTH: usize = 240;

/// Probes approved runtime executables without invoking a command shell.
pub struct ProcessDependencyProbe {
    timeout: Duration,
}

impl ProcessDependencyProbe {
    /// Creates a probe with a bounded execution time for each dependency.
    #[must_use]
    pub const fn new(timeout: Duration) -> Self {
        Self { timeout }
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
        let state = probe_process(specification, self.timeout).await;

        DependencyStatus { dependency, state }
    }
}

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

async fn probe_process(specification: ProbeSpecification, deadline: Duration) -> DependencyState {
    let mut command = Command::new(specification.executable);
    command.args(specification.arguments).kill_on_drop(true);

    match timeout(deadline, command.output()).await {
        Err(_) => DependencyState::TimedOut,
        Ok(Err(error)) if error.kind() == io::ErrorKind::NotFound => DependencyState::Missing,
        Ok(Err(error)) => DependencyState::Unhealthy {
            message: truncate(&error.to_string()),
        },
        Ok(Ok(output)) if output.status.success() => DependencyState::Available {
            version: first_non_empty_line(&output.stdout, &output.stderr),
        },
        Ok(Ok(output)) => DependencyState::Unhealthy {
            message: truncate(&format!("process exited with {}", output.status)),
        },
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
