//! Starting a background runner that outlives its caller (ADR 0033 section 5).
//!
//! `job start` in the watched mode records its job and starts the same binary
//! again, by its explicit path, to run it. The runner must survive the
//! caller: the caller's terminal closing, a Ctrl-C there, and the caller's
//! process group being killed. No shell is used: the executable and its
//! arguments are passed as they are.
//!
//! - **Unix:** the runner leads a process group of its own
//!   (`process_group(0)`), so a terminal's interrupt and a kill of the
//!   caller's group do not reach it.
//! - **Windows:** the runner starts in a hidden console of its own
//!   (`CREATE_NO_WINDOW`, so the providers it starts share it rather than
//!   opening windows) and a process group of its own
//!   (`CREATE_NEW_PROCESS_GROUP`), so a Ctrl-C in the caller's console does not
//!   reach it. It leaves the caller's job object (`CREATE_BREAKAWAY_FROM_JOB`)
//!   when that job allows it, so a terminal or IDE that ends its job object
//!   does not end the runner; when the job forbids it, the runner starts inside
//!   it and [`DetachedRunner::detachment`] says so (known limit L-220).
//!
//! Its standard streams are connected to nothing and its working folder is
//! the executable's own (#469): it holds no handle on the caller's console or
//! folder. On Windows that needs more than null streams: the standard library
//! passes a new process every inheritable handle of its caller, so a runner
//! started with it held the caller's output pipe when the caller's parent gave
//! it one, and that parent waited for end-of-file until the transcription ended
//! (L-229). The runner is therefore started by `vsift-windows-security`'s
//! `start_detached` (ADR 0026, amendment of 2026-10-27), whose inheritance list
//! holds only the runner's own null handle; this crate stays free of `unsafe`.

use std::{ffi::OsString, io, path::Path, process::ExitStatus};

/// The started runner, as each platform holds it. Both answer `try_wait`,
/// `kill` and `wait` as `std::process::Child` does.
#[cfg(not(windows))]
type RunnerProcess = std::process::Child;
/// See the other platforms' alias.
#[cfg(windows)]
type RunnerProcess = vsift_windows_security::DetachedProcess;

/// How far a started runner is detached from its caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Detachment {
    /// Unix: a process group of its own.
    ProcessGroup,
    /// Windows: a console and process group of its own, outside the caller's
    /// job object.
    LeftCallersJob,
    /// Windows: a console and process group of its own, but inside the
    /// caller's job object, which forbids leaving it: whatever ends that job
    /// ends the runner too.
    InCallersJob,
}

impl Detachment {
    /// Stable lowercase identifier used in contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::ProcessGroup => "process_group",
            Self::LeftCallersJob => "left_callers_job",
            Self::InCallersJob => "in_callers_job",
        }
    }
}

/// A runner started in the background.
///
/// Dropping it leaves the runner running; it is never waited for, and on Unix
/// it is reaped by the system once its caller has exited.
#[derive(Debug)]
pub struct DetachedRunner {
    child: RunnerProcess,
    detachment: Detachment,
}

impl DetachedRunner {
    /// How far the runner is detached from its caller.
    #[must_use]
    pub const fn detachment(&self) -> Detachment {
        self.detachment
    }

    /// The runner's exit status, if it has already ended.
    ///
    /// # Errors
    ///
    /// The system's failure to query the process.
    pub fn try_exited(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Ends a runner that did not take its job in time, and waits for it.
    ///
    /// # Errors
    ///
    /// The system's failure to end or wait for the process.
    pub fn kill(mut self) -> io::Result<()> {
        if self.child.try_wait()?.is_none() {
            self.child.kill()?;
        }
        self.child.wait().map(|_| ())
    }
}

/// Starts `executable` with `arguments`, detached from this process (see the
/// module), its standard streams connected to nothing and its working folder
/// `working_directory`.
///
/// # Errors
///
/// The system's failure to start the process.
pub fn launch_detached(
    executable: &Path,
    arguments: &[OsString],
    working_directory: &Path,
) -> io::Result<DetachedRunner> {
    platform_launch(executable, arguments, working_directory)
}

#[cfg(unix)]
fn platform_launch(
    executable: &Path,
    arguments: &[OsString],
    working_directory: &Path,
) -> io::Result<DetachedRunner> {
    use std::{
        os::unix::process::CommandExt,
        process::{Command, Stdio},
    };

    let child = Command::new(executable)
        .args(arguments)
        .current_dir(working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    Ok(DetachedRunner {
        child,
        detachment: Detachment::ProcessGroup,
    })
}

/// The runner starts with a hidden console and a process group of its own,
/// and inherits only its null standard streams (`start_detached`). It first
/// asks to leave the caller's job object and, when the job forbids that,
/// starts inside it (L-220).
#[cfg(windows)]
fn platform_launch(
    executable: &Path,
    arguments: &[OsString],
    working_directory: &Path,
) -> io::Result<DetachedRunner> {
    use vsift_windows_security::{CallersJob, start_detached};

    /// `ERROR_ACCESS_DENIED`: what starting a process fails with when the
    /// caller's job object does not allow it to leave.
    const ACCESS_DENIED: i32 = 5;

    match start_detached(executable, arguments, working_directory, CallersJob::Leave) {
        Ok(child) => Ok(DetachedRunner {
            child,
            detachment: Detachment::LeftCallersJob,
        }),
        Err(error) if error.raw_os_error() == Some(ACCESS_DENIED) => Ok(DetachedRunner {
            child: start_detached(executable, arguments, working_directory, CallersJob::Stay)?,
            detachment: Detachment::InCallersJob,
        }),
        Err(error) => Err(error),
    }
}

#[cfg(not(any(unix, windows)))]
fn platform_launch(
    _executable: &Path,
    _arguments: &[OsString],
    _working_directory: &Path,
) -> io::Result<DetachedRunner> {
    Err(io::ErrorKind::Unsupported.into())
}
