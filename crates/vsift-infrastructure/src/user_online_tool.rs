//! Identifying a yt-dlp or JavaScript runtime the user supplied (ADR 0028
//! section 4.6).
//!
//! `setup configure yt-dlp|js-runtime` records only a path. `setup check`
//! then identifies the file the way the smoke identifies a managed one, with
//! the same fixed invocation and environment, but nothing pins what it must
//! print: the user keeps it current. What is reported is the first line of
//! its version output and the SHA-256 of the file, so a later change to it
//! is visible. Nothing here fetches anything.

use std::{ffi::OsString, fmt, num::NonZeroUsize, path::Path, time::Duration};

use vsift_domain::Sha256Hex;

use crate::{
    HostIsolation, ProcessCancellation, ProcessError, ProcessRequest, ProcessSupervisor,
    ProcessWorkingDirectory, SupervisorPolicy, TerminationReason, TrustedExecutable,
    media_tool_verification::VerificationWorkspace, whisper_build::hash_file_bounded,
};

/// The most a user-supplied tool may weigh: yt-dlp's one-file builds are
/// about 40 MB and Deno about 96 MB.
const MAX_TOOL_BYTES: u64 = 512 * 1024 * 1024;
/// How long its version output may take, start-up of a one-file build
/// included.
const VERSION_DEADLINE: Duration = Duration::from_secs(30);
/// The most of its output that is read.
const OUTPUT_LIMIT_BYTES: usize = 4096;
/// The longest version line that is reported.
const MAX_VERSION_LINE_BYTES: usize = 64;

/// Which user-supplied tool is identified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserOnlineTool {
    /// The user's yt-dlp.
    YtDlp,
    /// The user's JavaScript runtime for yt-dlp (Deno only in `0.3.0`).
    JsRuntime,
}

impl UserOnlineTool {
    /// The arguments that print the version and nothing else.
    const fn version_arguments(self) -> &'static [&'static str] {
        match self {
            Self::YtDlp => &["--ignore-config", "--no-plugin-dirs", "--version"],
            Self::JsRuntime => &["--version"],
        }
    }

    /// The variables beyond the private folders: yt-dlp loads no plugins,
    /// Deno does not look for an update and prints no colour.
    const fn fixed_environment(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::YtDlp => &[("YTDLP_NO_PLUGINS", "1")],
            Self::JsRuntime => &[("DENO_NO_UPDATE_CHECK", "1"), ("NO_COLOR", "1")],
        }
    }

    /// Whether `line` has the shape of what this tool prints: yt-dlp a date
    /// version (`2026.08.19`, with an optional suffix), Deno `deno <version>`.
    fn accepts(self, line: &str) -> bool {
        match self {
            Self::YtDlp => {
                let mut parts = line.split('.');
                let year = parts.next().is_some_and(|part| {
                    part.len() == 4 && part.bytes().all(|byte| byte.is_ascii_digit())
                });
                let rest = parts.all(|part| {
                    !part.is_empty()
                        && part
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                });
                year && rest && line.contains('.')
            }
            Self::JsRuntime => line.starts_with("deno "),
        }
    }
}

/// What identifying a user-supplied tool found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserOnlineToolFacts {
    /// The first line of its version output.
    pub version_line: String,
    /// The SHA-256 of the file as it was identified.
    pub sha256: Sha256Hex,
}

/// Why a user-supplied tool could not be identified. The file's own text is
/// never carried.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserOnlineToolFault {
    /// The file could not be read, was too large or changed while read.
    Unreadable,
    /// It could not be started.
    CouldNotStart,
    /// It started and exited unsuccessfully.
    Failed,
    /// It did not answer in time.
    Deadline,
    /// It printed more than the bound.
    OutputOverBound,
    /// It printed something other than a version line of the expected shape.
    UnexpectedOutput,
    /// The call was cancelled.
    Cancelled,
    /// The private folder it runs in could not be made.
    Workspace,
}

impl UserOnlineToolFault {
    /// A fixed identifier for results; never any text the program printed.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Unreadable | Self::Workspace => "unreadable",
            Self::CouldNotStart => "could_not_start",
            Self::Failed => "failed",
            Self::Deadline => "deadline",
            Self::OutputOverBound => "output_over_bound",
            Self::UnexpectedOutput => "unexpected_output",
            Self::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for UserOnlineToolFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unreadable => "the file could not be read",
            Self::CouldNotStart => "the program could not be started",
            Self::Failed => "the program reported a failure",
            Self::Deadline => "the program did not answer in time",
            Self::OutputOverBound => "the program printed too much",
            Self::UnexpectedOutput => "the program did not print a version line",
            Self::Cancelled => "the check was cancelled",
            Self::Workspace => "no private folder could be made to run it in",
        })
    }
}

impl std::error::Error for UserOnlineToolFault {}

/// Runs `executable` with the fixed version invocation, in a private folder
/// made in `workspace_parent` (removed afterwards) that is also its
/// temporary and home folder, and hashes the file.
///
/// # Errors
///
/// A [`UserOnlineToolFault`] naming only the class of failure.
pub async fn identify_user_online_tool(
    tool: UserOnlineTool,
    executable: &TrustedExecutable,
    workspace_parent: &Path,
    host_isolation: HostIsolation,
    cancellation: ProcessCancellation,
) -> Result<UserOnlineToolFacts, UserOnlineToolFault> {
    let (_, digest) = hash_file_bounded(executable.path(), MAX_TOOL_BYTES)
        .await
        .map_err(|_| UserOnlineToolFault::Unreadable)?;
    let sha256 =
        Sha256Hex::parse(lowercase_hex(&digest)).map_err(|_| UserOnlineToolFault::Unreadable)?;
    let workspace = VerificationWorkspace::create(workspace_parent)
        .map_err(|_| UserOnlineToolFault::Workspace)?;
    let working = ProcessWorkingDirectory::new(workspace.path())
        .map_err(|_| UserOnlineToolFault::Workspace)?;
    let mut request = ProcessRequest::new(executable.clone(), working, VERSION_DEADLINE)
        .map_err(|_| UserOnlineToolFault::CouldNotStart)?
        .with_arguments(tool.version_arguments().iter().map(OsString::from));
    for name in ["TMPDIR", "TEMP", "TMP", "HOME"] {
        request = request
            .with_environment(name, workspace.path().as_os_str())
            .map_err(|_| UserOnlineToolFault::CouldNotStart)?;
    }
    for (name, value) in tool.fixed_environment() {
        request = request
            .with_environment(*name, *value)
            .map_err(|_| UserOnlineToolFault::CouldNotStart)?;
    }
    let limit = NonZeroUsize::new(OUTPUT_LIMIT_BYTES).unwrap_or(NonZeroUsize::MIN);
    let supervisor = ProcessSupervisor::new(
        SupervisorPolicy::default().with_stream_limits(limit, limit),
        host_isolation,
    );
    let outcome = supervisor
        .run(request, cancellation)
        .await
        .map_err(|error| match error {
            ProcessError::CancelledBeforeSpawn | ProcessError::CancelledDuringStart => {
                UserOnlineToolFault::Cancelled
            }
            _ => UserOnlineToolFault::CouldNotStart,
        })?;
    match outcome.termination {
        TerminationReason::Exited if outcome.status.success() => {}
        TerminationReason::Exited => return Err(UserOnlineToolFault::Failed),
        TerminationReason::Deadline => return Err(UserOnlineToolFault::Deadline),
        TerminationReason::OutputLimit(_) => return Err(UserOnlineToolFault::OutputOverBound),
        TerminationReason::Cancelled => return Err(UserOnlineToolFault::Cancelled),
    }
    let version_line = first_line(&outcome.stdout.bytes)?;
    if !tool.accepts(&version_line) {
        return Err(UserOnlineToolFault::UnexpectedOutput);
    }
    Ok(UserOnlineToolFacts {
        version_line,
        sha256,
    })
}

/// The first line of `bytes`, printable ASCII only and at most
/// [`MAX_VERSION_LINE_BYTES`], so nothing the program prints beyond a
/// version can be carried into a result.
fn first_line(bytes: &[u8]) -> Result<String, UserOnlineToolFault> {
    let line = bytes
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.is_empty()
        || line.len() > MAX_VERSION_LINE_BYTES
        || !line.iter().all(|byte| (0x20..0x7f).contains(byte))
    {
        return Err(UserOnlineToolFault::UnexpectedOutput);
    }
    String::from_utf8(line.to_vec()).map_err(|_| UserOnlineToolFault::UnexpectedOutput)
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(DIGITS[usize::from(byte >> 4)]));
        result.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{UserOnlineTool, UserOnlineToolFault, first_line};

    #[test]
    fn a_ytdlp_prints_a_date_version_and_a_runtime_prints_its_name() {
        for line in ["2026.08.19", "2026.08.19.232032", "2026.08.19-nightly"] {
            assert!(UserOnlineTool::YtDlp.accepts(line), "{line}");
        }
        for line in [
            "",
            "2026",
            "26.08.19",
            "yt-dlp 2026.08.19",
            "2026.08.19 x",
            "a.b.c",
        ] {
            assert!(!UserOnlineTool::YtDlp.accepts(line), "{line}");
        }
        assert!(UserOnlineTool::JsRuntime.accepts("deno 2.9.7 (stable, release, x86_64)"));
        assert!(!UserOnlineTool::JsRuntime.accepts("node v22"));
    }

    #[test]
    fn only_a_short_printable_first_line_is_carried() {
        assert_eq!(
            first_line(b"2026.08.19\nsecond\n"),
            Ok(String::from("2026.08.19"))
        );
        assert_eq!(
            first_line(b"2026.08.19\r\n"),
            Ok(String::from("2026.08.19"))
        );
        assert_eq!(first_line(b""), Err(UserOnlineToolFault::UnexpectedOutput));
        assert_eq!(
            first_line(b"\n"),
            Err(UserOnlineToolFault::UnexpectedOutput)
        );
        assert_eq!(
            first_line(b"ab\x1b[31mred"),
            Err(UserOnlineToolFault::UnexpectedOutput)
        );
        assert_eq!(
            first_line(&[b'a'; 65]),
            Err(UserOnlineToolFault::UnexpectedOutput)
        );
        assert_eq!(first_line(&[b'a'; 64]), Ok("a".repeat(64)));
    }
}
