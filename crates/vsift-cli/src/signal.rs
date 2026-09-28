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
//!
//! `job run` (P11 PR 3, ADR 0021 D4) is a worker host and shuts down in two
//! stages instead ([`listen_for_shutdown`]): the first signal stops the
//! request before its next step and, after the operator's drain time
//! (`--drain-timeout-ms`, default 0), cancels the step still running at its
//! next boundary; a second signal escalates as above.

use std::{future::Future, io, time::Duration};

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

/// The two signals a worker host's shutdown drives (ADR 0021 D4).
#[derive(Clone, Debug)]
pub(crate) struct Shutdown {
    /// Fired by the first signal: no further step starts.
    pub(crate) stop: Cancellation,
    /// Fired when the drain time is over: the running step stops at its
    /// next boundary. A second signal escalates it.
    pub(crate) work: Cancellation,
    /// How long the running step may go on after the first signal.
    pub(crate) drain: Duration,
}

impl Shutdown {
    /// A shutdown that has not begun, draining for `drain`.
    pub(crate) fn new(drain: Duration) -> Self {
        Self {
            stop: Cancellation::new(),
            work: Cancellation::new(),
            drain,
        }
    }

    /// Runs one shutdown once its first signal arrived: stops admission,
    /// lets the running step drain until the drain time is over or `second`
    /// arrives (which escalates), then cancels it.
    pub(crate) async fn begin(&self, second: impl Future<Output = ()>) {
        self.stop.cancel();
        if self.drain.is_zero() {
            self.work.cancel();
            second.await;
            self.work.escalate();
            return;
        }
        let mut second = std::pin::pin!(second);
        tokio::select! {
            () = tokio::time::sleep(self.drain) => {
                self.work.cancel();
                second.await;
            }
            () = &mut second => {}
        }
        self.work.escalate();
    }
}

/// Starts turning interruptions into a worker host's shutdown (see
/// [`Shutdown::begin`]).
///
/// # Errors
///
/// As [`listen`].
pub(crate) fn listen_for_shutdown(shutdown: Shutdown) -> io::Result<InterruptListener> {
    let mut interrupts = Interrupts::register()?;
    let task = tokio::spawn(async move {
        interrupts.next().await;
        shutdown.begin(interrupts.next()).await;
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
