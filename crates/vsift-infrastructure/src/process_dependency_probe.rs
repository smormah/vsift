//! Runtime dependency probing through the secure process supervisor.

use std::{ffi::OsStr, path::PathBuf, time::Duration};

use tokio::time::Instant;
use vsift_application::DependencyProbe;
use vsift_domain::{
    DependencyState, DependencyStatus, ProbeTimeout, ProbeTimeoutKind, RuntimeDependency,
};

use crate::{
    ExecutableResolutionError, ExecutableResolver, ProcessCancellation, ProcessError,
    ProcessRequest, ProcessSupervisor, ProcessWorkingDirectory, TerminationReason,
    TrustedExecutable, WhisperBuildRecognition, identify_whisper_build,
};

/// The total time the probes of one diagnostic share when the caller names none.
///
/// It was five seconds until `0.2.1`. That was enough for tools that are
/// already in the operating system's file cache and too little for the first
/// start after an install or a restart: on one Windows machine with real-time
/// virus scanning, a full `FFmpeg` build of about 200 MB was reported to take
/// 0.8 s and `FFprobe` 4.7 s cold, so two healthy tools used up the whole
/// deadline and were reported as timed out (issue #391). Thirty seconds is a
/// choice, not a measurement of slow machines: more than five times that
/// report's cold total, room for three slow starts in a row, and half the
/// 60-second cap a caller may ask for. The cost is the other side of the same
/// number: a tool that truly hangs now holds the diagnostic for up to thirty
/// seconds instead of five.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(30);

const MAX_DIAGNOSTIC_LENGTH: usize = 240;
/// Detail when a tool responded but printed no line that is safe to echo.
const DETECTED_DETAIL: &str = "detected";
const REVIEWED_WHISPER_DETAIL: &str = "whisper.cpp v1.9.2 (reviewed build)";
const UNRECOGNISED_WHISPER_DETAIL: &str = "whisper-cli (build not recognised)";
/// Log prefixes of the ggml backend loader, whose lines name absolute library paths.
const LOADER_LOG_PREFIXES: [&str; 3] = ["load_backend:", "ggml_", "register_backend"];

/// Explicit executable selections resolved for one read-only diagnostic operation.
///
/// The caller may combine per-call paths with validated private per-user configuration.
/// Paths are never inferred from an untrusted project directory.
#[derive(Clone, Debug, Default)]
pub struct ExplicitProbePaths {
    /// Absolute path to an existing `FFmpeg` executable, if selected.
    pub ffmpeg: Option<PathBuf>,
    /// Absolute path to an existing `FFprobe` executable, if selected.
    pub ffprobe: Option<PathBuf>,
    /// Absolute path to an existing whisper.cpp CLI executable, if selected.
    pub whisper: Option<PathBuf>,
}

impl ExplicitProbePaths {
    /// Returns the selected path for one dependency without falling back to `PATH`.
    #[must_use]
    pub fn for_dependency(&self, dependency: RuntimeDependency) -> Option<&PathBuf> {
        match dependency {
            RuntimeDependency::Ffmpeg => self.ffmpeg.as_ref(),
            RuntimeDependency::Ffprobe => self.ffprobe.as_ref(),
            RuntimeDependency::Whisper => self.whisper.as_ref(),
        }
    }
}

/// Probes approved runtime executables without invoking a command shell.
///
/// One instance represents one setup-check operation. Its deadline is shared across every
/// dependency probe, preventing a sequence of slow providers from multiplying the caller's
/// configured operation timeout.
///
/// The price of sharing is that a slow probe leaves less, or nothing, for the ones after it. A
/// probe whose turn comes after the deadline is reported as timed out without being started, and
/// says so ([`ProbeTimeoutKind::NotStarted`]): it is a different fact from a tool that was started
/// and did not answer, and neither is a missing tool. A tool that cannot be found is still reported
/// missing after the deadline, because resolution comes before the deadline is looked at.
pub struct ProcessDependencyProbe {
    operation_deadline: Instant,
    /// The deadline as the caller set it, reported with every probe that timed out.
    operation_timeout: Duration,
    resolver: ExecutableResolver,
    supervisor: ProcessSupervisor,
    explicit_paths: ExplicitProbePaths,
}

impl ProcessDependencyProbe {
    /// Creates a probe with one total deadline and a safely filtered snapshot of `PATH`.
    #[must_use]
    pub fn new(operation_timeout: Duration) -> Self {
        Self::with_explicit_paths(operation_timeout, ExplicitProbePaths::default())
    }

    /// Creates a probe with explicit-path selections ahead of filtered `PATH` lookup.
    #[must_use]
    pub fn with_explicit_paths(
        operation_timeout: Duration,
        explicit_paths: ExplicitProbePaths,
    ) -> Self {
        Self::configured_with_explicit_paths(
            operation_timeout,
            ExecutableResolver::from_current_path(),
            ProcessSupervisor::default(),
            explicit_paths,
        )
    }

    /// Creates a probe with explicit resolution and supervision policies.
    #[must_use]
    pub fn configured(
        operation_timeout: Duration,
        resolver: ExecutableResolver,
        supervisor: ProcessSupervisor,
    ) -> Self {
        Self::configured_with_explicit_paths(
            operation_timeout,
            resolver,
            supervisor,
            ExplicitProbePaths::default(),
        )
    }

    /// Creates a probe with explicit paths and injected process policies for tests/hosts.
    #[must_use]
    pub fn configured_with_explicit_paths(
        operation_timeout: Duration,
        resolver: ExecutableResolver,
        supervisor: ProcessSupervisor,
        explicit_paths: ExplicitProbePaths,
    ) -> Self {
        let now = Instant::now();
        // A timeout too large to be an instant on this clock was always treated as
        // none at all; it is reported as the zero it was applied as.
        let (operation_deadline, operation_timeout) = match now.checked_add(operation_timeout) {
            Some(deadline) => (deadline, operation_timeout),
            None => (now, Duration::ZERO),
        };
        Self {
            operation_deadline,
            operation_timeout,
            resolver,
            supervisor,
            explicit_paths,
        }
    }

    /// A probe of this operation that gave no result by the shared deadline.
    const fn timed_out(&self, kind: ProbeTimeoutKind) -> DependencyState {
        DependencyState::TimedOut(ProbeTimeout {
            kind,
            shared_deadline: self.operation_timeout,
        })
    }
}

impl Default for ProcessDependencyProbe {
    fn default() -> Self {
        Self::new(DEFAULT_PROBE_TIMEOUT)
    }
}

impl DependencyProbe for ProcessDependencyProbe {
    async fn probe(&self, dependency: RuntimeDependency) -> DependencyStatus {
        let specification = ProbeSpecification::for_dependency(dependency);
        let state = self.probe_process(dependency, specification).await;
        DependencyStatus { dependency, state }
    }
}

impl ProcessDependencyProbe {
    async fn probe_process(
        &self,
        dependency: RuntimeDependency,
        specification: ProbeSpecification,
    ) -> DependencyState {
        let resolution = self.explicit_paths.for_dependency(dependency).map_or_else(
            || self.resolver.resolve(OsStr::new(specification.executable)),
            TrustedExecutable::explicit,
        );
        let executable = match resolution {
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
        // The executable is found and its folder is usable; only now is the shared
        // deadline looked at, so "found but not started" is true of what follows.
        let Some(remaining) = self
            .operation_deadline
            .checked_duration_since(Instant::now())
        else {
            return self.timed_out(ProbeTimeoutKind::NotStarted);
        };
        if remaining.is_zero() {
            return self.timed_out(ProbeTimeoutKind::NotStarted);
        }
        let request = match ProcessRequest::new(executable.clone(), working_directory, remaining) {
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
                self.timed_out(ProbeTimeoutKind::NoAnswer)
            }
            Ok(outcome) if outcome.status.success() => DependencyState::Available {
                version: match specification.detail {
                    ProbeDetailPolicy::VersionLine { prefix } => {
                        version_line(prefix, &outcome.stdout.bytes, &outcome.stderr.bytes)
                    }
                    ProbeDetailPolicy::BuildIdentity => build_identity_detail(&executable).await,
                },
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
    detail: ProbeDetailPolicy,
}

/// What a successful probe may report as its `detail`.
///
/// Provider output is untrusted and can name absolute paths, which the setup
/// contract never echoes, so no tool's output is passed through wholesale.
/// Each tool states the one line shape it may contribute, or that it
/// contributes none.
#[derive(Clone, Copy)]
enum ProbeDetailPolicy {
    /// Only the first line starting with this reviewed version banner.
    VersionLine { prefix: &'static str },
    /// Never the tool's output: fixed prose saying whether the executable's
    /// bytes are a reviewed build. whisper.cpp prints loader logs naming
    /// absolute library paths first and has no version banner on `--help`.
    BuildIdentity,
}

impl ProbeSpecification {
    const fn for_dependency(dependency: RuntimeDependency) -> Self {
        match dependency {
            RuntimeDependency::Ffmpeg => Self {
                executable: "ffmpeg",
                arguments: &["-version"],
                detail: ProbeDetailPolicy::VersionLine {
                    prefix: "ffmpeg version ",
                },
            },
            RuntimeDependency::Ffprobe => Self {
                executable: "ffprobe",
                arguments: &["-version"],
                detail: ProbeDetailPolicy::VersionLine {
                    prefix: "ffprobe version ",
                },
            },
            RuntimeDependency::Whisper => Self {
                executable: "whisper-cli",
                arguments: &["--help"],
                detail: ProbeDetailPolicy::BuildIdentity,
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

/// Returns the first line starting with `prefix` that is safe to echo.
fn version_line(prefix: &str, stdout: &[u8], stderr: &[u8]) -> String {
    let stdout_text = String::from_utf8_lossy(stdout);
    let stderr_text = String::from_utf8_lossy(stderr);

    stdout_text
        .lines()
        .chain(stderr_text.lines())
        .map(str::trim)
        .find(|line| line.starts_with(prefix) && safe_to_echo(line))
        .map_or_else(|| String::from(DETECTED_DETAIL), truncate)
}

/// Defence in depth for every tool: a line that looks like it names a path,
/// or that carries a ggml loader log, is never echoed whatever selected it.
fn safe_to_echo(line: &str) -> bool {
    !LOADER_LOG_PREFIXES
        .iter()
        .any(|prefix| line.contains(prefix))
        && !contains_path_shape(line)
}

/// Whether text contains a drive-letter path (`C:\` or `C:/`), a UNC or
/// escaped separator pair (`\\`), or a `/segment/` run.
fn contains_path_shape(text: &str) -> bool {
    let bytes = text.as_bytes();
    let drive = bytes.windows(3).any(|window| {
        window[0].is_ascii_alphabetic() && window[1] == b':' && matches!(window[2], b'\\' | b'/')
    });
    let doubled_backslash = text.contains("\\\\");
    let mut slash_segment = false;
    let mut segment_length: Option<usize> = None;
    for character in text.chars() {
        match (character, segment_length) {
            ('/', Some(length)) if length > 0 => slash_segment = true,
            ('/', _) => segment_length = Some(0),
            (character, Some(length)) if !character.is_whitespace() => {
                segment_length = Some(length + 1);
            }
            (_, _) => segment_length = None,
        }
    }
    drive || doubled_backslash || slash_segment
}

async fn build_identity_detail(executable: &TrustedExecutable) -> String {
    match identify_whisper_build(executable).await {
        Ok(identity) => match identity.recognition() {
            WhisperBuildRecognition::ReviewedV1_9_2 => String::from(REVIEWED_WHISPER_DETAIL),
            WhisperBuildRecognition::Unrecognised => String::from(UNRECOGNISED_WHISPER_DETAIL),
        },
        Err(_) => String::from(UNRECOGNISED_WHISPER_DETAIL),
    }
}

fn truncate(value: &str) -> String {
    value.chars().take(MAX_DIAGNOSTIC_LENGTH).collect()
}

#[cfg(test)]
mod tests {
    use super::{MAX_DIAGNOSTIC_LENGTH, contains_path_shape, safe_to_echo, truncate, version_line};

    /// The first line whisper-cli v1.9.2 printed on Windows, verbatim.
    const WHISPER_LOADER_LINE: &str = r"load_backend: loaded CPU backend from C:\tools\whisper.cpp\v1.9.2\Release\ggml-cpu-haswell.dll";

    #[test]
    fn returns_the_version_banner_from_standard_output() {
        let result = version_line(
            "ffmpeg version ",
            b"\nffmpeg version 9.0-full_build-www.gyan.dev Copyright (c) 2000-2026 the FFmpeg developers\nbuilt with gcc",
            b"ignored",
        );

        assert_eq!(
            result,
            "ffmpeg version 9.0-full_build-www.gyan.dev Copyright (c) 2000-2026 the FFmpeg developers"
        );
    }

    #[test]
    fn falls_back_to_standard_error_for_the_banner() {
        let result = version_line("ffprobe version ", b"", b"ffprobe version 7.1\nmore");

        assert_eq!(result, "ffprobe version 7.1");
    }

    /// The observed whisper-cli first line is neither a banner nor echoable.
    #[test]
    fn the_observed_whisper_loader_line_is_never_a_detail() {
        assert!(!safe_to_echo(WHISPER_LOADER_LINE));
        let stderr = format!("{WHISPER_LOADER_LINE}\n\nusage: whisper-cli [options]");
        assert_eq!(
            version_line("ffmpeg version ", b"", stderr.as_bytes()),
            "detected"
        );
        let disguised = format!("ffmpeg version {WHISPER_LOADER_LINE}");
        assert_eq!(
            version_line("ffmpeg version ", disguised.as_bytes(), b""),
            "detected"
        );
    }

    #[test]
    fn lines_with_path_shapes_or_loader_logs_are_not_echoed() {
        assert!(contains_path_shape(WHISPER_LOADER_LINE));
        for unsafe_line in [
            "ffmpeg version 7 C:/tools/ffmpeg.exe",
            r"ffmpeg version 7 \\server\share",
            "ffmpeg version 7 --prefix=/usr/local/bin",
            "ffmpeg version 7 ggml_backend x",
            "ffmpeg version 7 register_backend: cpu",
        ] {
            assert!(!safe_to_echo(unsafe_line), "{unsafe_line}");
            assert_eq!(
                version_line("ffmpeg version ", unsafe_line.as_bytes(), b""),
                "detected"
            );
        }
        for safe_line in [
            "ffmpeg version n9.0.1-11-ge47273f4d9 Copyright (c) 2000-2026 the FFmpeg developers",
            "ffprobe version 6.1.1-3ubuntu5 Copyright (c) 2007-2023 and/or later",
        ] {
            assert!(safe_to_echo(safe_line), "{safe_line}");
        }
    }

    #[test]
    fn bounds_diagnostics() {
        let input = "a".repeat(MAX_DIAGNOSTIC_LENGTH + 20);

        assert_eq!(truncate(&input).len(), MAX_DIAGNOSTIC_LENGTH);
    }
}
