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
//! folder.

use std::{
    ffi::OsString,
    io,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
};

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
    child: Child,
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
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .current_dir(working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    platform_launch(command)
}

#[cfg(unix)]
fn platform_launch(mut command: Command) -> io::Result<DetachedRunner> {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
    Ok(DetachedRunner {
        child: command.spawn()?,
        detachment: Detachment::ProcessGroup,
    })
}

#[cfg(windows)]
fn platform_launch(mut command: Command) -> io::Result<DetachedRunner> {
    use std::os::windows::process::CommandExt;

    use windows::Win32::System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW,
    };

    /// `ERROR_ACCESS_DENIED`: what starting a process fails with when the
    /// caller's job object does not allow it to leave.
    const ACCESS_DENIED: i32 = 5;

    let own = CREATE_NO_WINDOW.0 | CREATE_NEW_PROCESS_GROUP.0;
    command.creation_flags(own | CREATE_BREAKAWAY_FROM_JOB.0);
    match command.spawn() {
        Ok(child) => Ok(DetachedRunner {
            child,
            detachment: Detachment::LeftCallersJob,
        }),
        Err(error) if error.raw_os_error() == Some(ACCESS_DENIED) => {
            command.creation_flags(own);
            Ok(DetachedRunner {
                child: command.spawn()?,
                detachment: Detachment::InCallersJob,
            })
        }
        Err(error) => Err(error),
    }
}

#[cfg(not(any(unix, windows)))]
fn platform_launch(_command: Command) -> io::Result<DetachedRunner> {
    Err(io::ErrorKind::Unsupported.into())
}
