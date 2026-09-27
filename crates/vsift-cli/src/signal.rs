//! Console interruption of long commands (P10 PR 3, ADR 0020 section 5;
//! supersedes ADR 0017 decision 4).
//!
//! While a long command runs, the first `SIGINT` or `SIGTERM` (Unix), or
//! console Ctrl-C or Ctrl-Break (Windows), cancels the one [`Cancellation`]
//! the command was given: the command stops at its next boundary, a running
//! provider is stopped by the supervisor and reaped, and the command answers
//! with its documented terminal result (`CANCELLED`, or the partial result
//! of `candidates` and the evidence commands). A second interruption
//! escalates: providers are killed without the graceful wait, so the command
//! ends within the supervisor's forced budget. The process never exits
//! while a provider it started is still running (SEC-04): it only returns
//! once the command has reaped everything.
//!
//! Handlers are registered before the command starts, so no early signal is
//! missed, and only for long commands: every other command keeps the
//! operating system's default and ends at once.

use std::io;

use vsift::Cancellation;

/// Turns interruptions into cancellation while it is held; dropping it
/// stops listening (the handlers stay registered with the runtime, so a
/// later signal is consumed rather than ending the process mid-output).
pub(crate) struct InterruptListener {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for InterruptListener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Starts turning interruptions into `cancellation`: the first cancels it,
/// the second escalates it.
///
/// # Errors
///
/// The operating system refused the handler; the command then runs with the
/// default behaviour (an interruption ends the process, and a committed
/// result is still never partial).
pub(crate) fn listen(cancellation: Cancellation) -> io::Result<InterruptListener> {
    let mut interrupts = Interrupts::register()?;
    let task = tokio::spawn(async move {
        interrupts.next().await;
        cancellation.cancel();
        interrupts.next().await;
        cancellation.escalate();
    });
    Ok(InterruptListener { task })
}

/// The interruptions a Unix command honours: `SIGINT` (Ctrl-C at a
/// terminal) and `SIGTERM` (a supervisor asking it to stop).
#[cfg(unix)]
struct Interrupts {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl Interrupts {
    fn register() -> io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Self {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
        })
    }

    /// Waits for the next interruption. A stream that ended never delivers
    /// one again, so the wait then lasts until the command finishes.
    async fn next(&mut self) {
        let received = tokio::select! {
            received = self.interrupt.recv() => received,
            received = self.terminate.recv() => received,
        };
        if received.is_none() {
            std::future::pending::<()>().await;
        }
    }
}

/// The interruptions a Windows console command honours: Ctrl-C and
/// Ctrl-Break, delivered to every process attached to the console.
#[cfg(windows)]
struct Interrupts {
    ctrl_c: tokio::signal::windows::CtrlC,
    ctrl_break: tokio::signal::windows::CtrlBreak,
}

#[cfg(windows)]
impl Interrupts {
    fn register() -> io::Result<Self> {
        Ok(Self {
            ctrl_c: tokio::signal::windows::ctrl_c()?,
            ctrl_break: tokio::signal::windows::ctrl_break()?,
        })
    }

    /// Waits for the next interruption. A stream that ended never delivers
    /// one again, so the wait then lasts until the command finishes.
    async fn next(&mut self) {
        let received = tokio::select! {
            received = self.ctrl_c.recv() => received,
            received = self.ctrl_break.recv() => received,
        };
        if received.is_none() {
            std::future::pending::<()>().await;
        }
    }
}

/// Targets without console interruptions VSift supports: nothing is
/// trapped and the default behaviour stays.
#[cfg(not(any(unix, windows)))]
struct Interrupts;

#[cfg(not(any(unix, windows)))]
impl Interrupts {
    fn register() -> io::Result<Self> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    async fn next(&mut self) {
        std::future::pending::<()>().await;
    }
}
