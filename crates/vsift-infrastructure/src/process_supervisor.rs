//! Bounded, shell-free lifecycle supervision for external provider processes.

use std::{
    error::Error,
    ffi::{OsStr, OsString},
    fmt,
    future::pending,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    time::Duration,
};

#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
#[cfg(windows)]
use process_wrap::tokio::{CreationFlags, JobObject};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::{mpsc, watch},
    task::JoinSet,
    time::{Instant, sleep_until, timeout},
};
#[cfg(windows)]
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::executable::{ExecutableProvenance, TrustedExecutable};

/// The default diagnostic limit for each external process stream.
pub const DEFAULT_STREAM_LIMIT: usize = 64 * 1024;

/// Identifies one captured process output stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// Describes the process-tree mechanism applied by this host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessContainment {
    /// Descendants are assigned to a Windows Job Object during suspended creation.
    WindowsJobObject,
    /// Descendants inherit a Unix process group used for lifecycle signals.
    UnixProcessGroup,
    /// The target does not provide a qualified descendant-containment adapter.
    Unsupported,
}

/// Hard worker isolation actually inherited by the provider process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HardIsolation {
    /// No qualified filesystem, network, CPU, memory, or PID boundary was attested.
    NotEnforced,
    /// A trusted Linux worker host attested all strict inherited controls.
    InheritedStrictLinux,
}

/// The isolation level required for a process request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IsolationRequirement {
    /// Require descendant lifecycle containment, but no whole-machine sandbox claim.
    ProcessTree,
    /// Require qualified inherited filesystem, network, CPU, memory, and PID controls.
    StrictWorker,
}

/// Trusted host report about controls established outside this process supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostIsolation {
    /// The host has not established a strict inherited worker boundary.
    ProcessOnly,
    /// The Linux worker host has established and verified the strict R0 boundary.
    #[cfg(target_os = "linux")]
    StrictLinux,
}

impl HostIsolation {
    /// The least isolation every provider run under this host requires: on
    /// an attested strict Linux worker every provider runs as a strict
    /// worker request (P11 PR 2), so a supervisor whose effective controls
    /// ever fell short would refuse it rather than run it unconfined.
    #[must_use]
    pub const fn minimum_requirement(self) -> IsolationRequirement {
        match self {
            Self::ProcessOnly => IsolationRequirement::ProcessTree,
            #[cfg(target_os = "linux")]
            Self::StrictLinux => IsolationRequirement::StrictWorker,
        }
    }
}

/// Immutable limits applied to every process run by one supervisor.
#[derive(Clone, Copy, Debug)]
pub struct SupervisorPolicy {
    stdout_limit: NonZeroUsize,
    stderr_limit: NonZeroUsize,
    graceful_shutdown: Duration,
    forced_shutdown: Duration,
}

impl SupervisorPolicy {
    /// Creates a policy with per-stream byte and shutdown-time limits.
    #[must_use]
    pub const fn new(
        stream_limit: NonZeroUsize,
        graceful_shutdown: Duration,
        forced_shutdown: Duration,
    ) -> Self {
        Self {
            stdout_limit: stream_limit,
            stderr_limit: stream_limit,
            graceful_shutdown,
            forced_shutdown,
        }
    }

    /// Returns the maximum retained bytes for each output stream.
    #[must_use]
    pub const fn stream_limit(self) -> NonZeroUsize {
        self.stdout_limit
    }

    /// Sets independent structured-output and diagnostic limits.
    #[must_use]
    pub const fn with_stream_limits(mut self, stdout: NonZeroUsize, stderr: NonZeroUsize) -> Self {
        self.stdout_limit = stdout;
        self.stderr_limit = stderr;
        self
    }
}

impl Default for SupervisorPolicy {
    fn default() -> Self {
        let stream_limit = match NonZeroUsize::new(DEFAULT_STREAM_LIMIT) {
            Some(value) => value,
            None => NonZeroUsize::MIN,
        };
        Self::new(stream_limit, Duration::from_secs(5), Duration::from_secs(5))
    }
}

/// An absolute canonical working directory approved for an external process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessWorkingDirectory(PathBuf);

impl ProcessWorkingDirectory {
    /// Validates and canonicalizes an explicit process working directory.
    ///
    /// # Errors
    ///
    /// Returns a typed path or filesystem-inspection failure when the directory cannot be used.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, ProcessRequestError> {
        let path = path.as_ref();
        if !path.is_absolute() {
            return Err(ProcessRequestError::WorkingDirectoryNotAbsolute);
        }
        let canonical =
            std::fs::canonicalize(path).map_err(ProcessRequestError::WorkingDirectoryInspection)?;
        if !canonical.is_dir() {
            return Err(ProcessRequestError::WorkingDirectoryNotDirectory);
        }
        Ok(Self(canonical))
    }

    /// The folder that holds `executable`: the working directory every media
    /// and speech provider runs in (#469).
    ///
    /// Windows starts no process whose working directory is longer than 258
    /// characters (`CreateProcessW` fails with `ERROR_DIRECTORY`, whatever
    /// `LongPathsEnabled` says), so a provider run in the folder of the file it
    /// reads could not start once a long per-user folder or session root
    /// pushed that folder past the limit. An executable's own folder never
    /// has that problem: Windows starts a program only from a path shorter
    /// than `MAX_PATH` (L-094), so the folder of one it can start is a valid
    /// working directory, and `setup check` runs the tools from the same
    /// folder. It adds nothing to the library search either: the program's
    /// own folder is searched before the working directory anyway (SEC-02).
    /// Providers are given absolute paths for everything they read or write,
    /// so nothing depends on the working directory's contents.
    ///
    /// # Errors
    ///
    /// As [`Self::new`]; a canonical executable path always has a folder,
    /// so [`ProcessRequestError::WorkingDirectoryNotDirectory`] stands for
    /// one that has none.
    pub fn of_executable(executable: &TrustedExecutable) -> Result<Self, ProcessRequestError> {
        let folder = executable
            .path()
            .parent()
            .ok_or(ProcessRequestError::WorkingDirectoryNotDirectory)?;
        Self::new(folder)
    }

    /// Returns the canonical directory path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// Fully explicit request for one directly spawned provider process.
#[derive(Clone, Debug)]
pub struct ProcessRequest {
    executable: TrustedExecutable,
    arguments: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
    working_directory: ProcessWorkingDirectory,
    deadline: Duration,
    isolation: IsolationRequirement,
}

impl ProcessRequest {
    /// Creates a shell-free request with null stdin and an empty child environment.
    ///
    /// # Errors
    ///
    /// Returns a typed validation failure for a zero or unrepresentable deadline.
    pub fn new(
        executable: TrustedExecutable,
        working_directory: ProcessWorkingDirectory,
        deadline: Duration,
    ) -> Result<Self, ProcessRequestError> {
        if deadline.is_zero() {
            return Err(ProcessRequestError::DeadlineMustBePositive);
        }
        if std::time::Instant::now().checked_add(deadline).is_none() {
            return Err(ProcessRequestError::DeadlineOutOfRange);
        }
        Ok(Self {
            executable,
            arguments: Vec::new(),
            environment: Vec::new(),
            working_directory,
            deadline,
            isolation: IsolationRequirement::ProcessTree,
        })
    }

    /// Appends one argument exactly as one operating-system argument.
    #[must_use]
    pub fn with_argument(mut self, argument: impl Into<OsString>) -> Self {
        self.arguments.push(argument.into());
        self
    }

    /// Appends arguments without parsing, interpolation, or shell expansion.
    #[must_use]
    pub fn with_arguments(
        mut self,
        arguments: impl IntoIterator<Item = impl Into<OsString>>,
    ) -> Self {
        self.arguments.extend(arguments.into_iter().map(Into::into));
        self
    }

    /// Adds one explicitly allowlisted environment entry to the otherwise empty child environment.
    ///
    /// # Errors
    ///
    /// Returns a typed validation failure for a non-portable name or a value containing a null.
    pub fn with_environment(
        mut self,
        name: impl Into<OsString>,
        value: impl Into<OsString>,
    ) -> Result<Self, ProcessRequestError> {
        let name = name.into();
        let value = value.into();
        validate_environment_name(&name)?;
        if value.to_string_lossy().contains('\0') {
            return Err(ProcessRequestError::InvalidEnvironmentValue);
        }
        self.environment.push((name, value));
        Ok(self)
    }

    /// Changes the required isolation level; unsupported requirements fail before spawn.
    #[must_use]
    pub const fn requiring_isolation(mut self, isolation: IsolationRequirement) -> Self {
        self.isolation = isolation;
        self
    }
}

/// A clonable caller-cancellation signal for a supervised process operation.
///
/// It has two steps. [`ProcessCancellation::cancel`] stops the operation at
/// its next boundary and stops a running provider tree gracefully (on Unix a
/// `SIGTERM` to its process group, then a kill after the graceful budget).
/// [`ProcessCancellation::escalate`] additionally skips whatever remains of
/// that graceful wait, so a caller that asked twice (a second Ctrl-C) waits
/// only for the forced kill and the reap, never for a provider that ignores
/// `SIGTERM`. Neither step ever abandons a provider: the supervisor still
/// reaps the whole tree before it returns (SEC-04).
///
/// A [`ProcessCancellation::child`] observes its parent as well as its own
/// signal: cancelling the parent stops every child, while cancelling a child
/// stops only that child. A worker host gives each request its own child of
/// the process-wide signal, so one request can be stopped without the others
/// and a shutdown still reaches them all (P11, ADR 0021).
#[derive(Clone, Debug)]
pub struct ProcessCancellation {
    sender: watch::Sender<CancellationState>,
    parent: Option<Box<ProcessCancellation>>,
}

/// The steps of a cancellation, in order; a signal only moves forward.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum CancellationState {
    Active,
    Cancelled,
    Escalated,
}

impl ProcessCancellation {
    /// Creates an uncancelled signal.
    #[must_use]
    pub fn new() -> Self {
        let (sender, _) = watch::channel(CancellationState::Active);
        Self {
            sender,
            parent: None,
        }
    }

    /// A new signal that is also cancelled (or escalated) whenever this one
    /// is, but whose own cancellation leaves this one untouched.
    #[must_use]
    pub fn child(&self) -> Self {
        let (sender, _) = watch::channel(CancellationState::Active);
        Self {
            sender,
            parent: Some(Box::new(self.clone())),
        }
    }

    /// Requests cancellation. Repeated requests are idempotent.
    pub fn cancel(&self) {
        self.advance(CancellationState::Cancelled);
    }

    /// Requests cancellation and that running providers be killed without
    /// the graceful wait. Implies [`ProcessCancellation::cancel`];
    /// repeated requests are idempotent.
    pub fn escalate(&self) {
        self.advance(CancellationState::Escalated);
    }

    /// Reports whether cancellation was already requested, here or on an
    /// ancestor.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state() >= CancellationState::Cancelled
    }

    /// Reports whether the graceful stop was skipped by a second request,
    /// here or on an ancestor.
    #[must_use]
    pub fn is_escalated(&self) -> bool {
        self.state() == CancellationState::Escalated
    }

    /// The furthest step this signal or any ancestor has reached.
    fn state(&self) -> CancellationState {
        let own = *self.sender.borrow();
        self.parent
            .as_ref()
            .map_or(own, |parent| own.max(parent.state()))
    }

    fn advance(&self, next: CancellationState) {
        self.sender.send_if_modified(|state| {
            if *state < next {
                *state = next;
                true
            } else {
                false
            }
        });
    }

    /// Resolves once this signal or an ancestor reaches `step`. A chain is
    /// a handful of links at most (the process signal and one request), and
    /// each level waits on its own signal and its parent's at once.
    fn reached(
        &self,
        step: CancellationState,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            let mut receiver = self.sender.subscribe();
            loop {
                if *receiver.borrow_and_update() >= step {
                    return;
                }
                match &self.parent {
                    Some(parent) => {
                        tokio::select! {
                            changed = receiver.changed() => {
                                if changed.is_err() {
                                    return;
                                }
                            }
                            () = parent.reached(step) => return,
                        }
                    }
                    None => {
                        if receiver.changed().await.is_err() {
                            return;
                        }
                    }
                }
            }
        })
    }

    async fn cancelled(&self) {
        self.reached(CancellationState::Cancelled).await;
    }

    /// Resolves once this signal or an ancestor is cancelled (at once if it
    /// already is): a worker host waits on it between the units of work it
    /// schedules itself, such as a backoff before a retry (P11).
    pub async fn wait_cancelled(&self) {
        self.cancelled().await;
    }

    /// Resolves once the cancellation is escalated. A caller races it
    /// against the graceful wait only, so a dropped sender (impossible while
    /// `self` lives) could never end a wait early.
    #[cfg(unix)]
    async fn escalated(&self) {
        self.reached(CancellationState::Escalated).await;
    }
}

impl Default for ProcessCancellation {
    fn default() -> Self {
        Self::new()
    }
}

/// A bounded byte capture and whether the underlying stream reached EOF.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapturedOutput {
    /// Retained bytes, never larger than the configured stream limit.
    pub bytes: Vec<u8>,
    /// Whether bytes beyond the retention limit were observed.
    pub truncated: bool,
    /// Whether the stream reached EOF before the post-termination drain deadline.
    pub complete: bool,
}

/// Reports whether one deterministic process control was applied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlStatus {
    /// The control was applied to this invocation.
    Applied,
    /// The control was not applied to this invocation.
    NotApplied,
}

/// Why the supervised process stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminationReason {
    /// The process tree and both streams completed without a supervisor intervention.
    Exited,
    /// The operation deadline elapsed.
    Deadline,
    /// The caller cancellation signal won the terminal transition, or the
    /// provider ended unsuccessfully after cancellation was requested (see
    /// [`completed_termination`]).
    Cancelled,
    /// A provider emitted more than the configured output budget.
    OutputLimit(OutputStream),
}

/// Controls actually applied to one process invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectiveControls {
    /// How the executable was selected.
    pub executable_provenance: ExecutableProvenance,
    /// Descendant lifecycle containment applied by the platform adapter.
    pub process_containment: ProcessContainment,
    /// Hard isolation inherited from the trusted worker host.
    pub hard_isolation: HardIsolation,
    /// Standard-output retained-byte limit.
    pub stream_limit: usize,
    /// Standard-error diagnostic retained-byte limit.
    pub stderr_limit: usize,
    /// Whether stdin was explicitly connected to the null device.
    pub null_stdin: ControlStatus,
    /// Whether the inherited environment was cleared before adding allowlisted entries.
    pub cleared_environment: ControlStatus,
    /// Whether an explicit canonical working directory was applied.
    pub explicit_working_directory: ControlStatus,
    /// Whether the platform adapter can attempt a graceful tree-wide stop.
    pub graceful_tree_stop: ControlStatus,
    /// Whether the provider is out of reach of the interrupts sent to the
    /// host itself, so that only the supervisor decides when it stops: on
    /// Unix it leads a process group of its own, which a terminal's
    /// interrupt or a signal to the host's group does not reach; on Windows
    /// it has a console of its own, which a console Ctrl-C or Ctrl-Break sent
    /// to the host's console does not reach (#404).
    pub interrupt_isolation: ControlStatus,
}

/// Result of a completed, cancelled, limited, or timed-out provider process.
#[derive(Debug)]
pub struct ProcessOutcome {
    /// Exit status observed after the complete process tree was reaped.
    pub status: ExitStatus,
    /// Bounded standard output capture.
    pub stdout: CapturedOutput,
    /// Bounded standard error capture.
    pub stderr: CapturedOutput,
    /// Terminal transition selected by the supervisor.
    pub termination: TerminationReason,
    /// Controls actually applied to this invocation.
    pub controls: EffectiveControls,
}

/// Supervises directly spawned provider processes under immutable limits.
#[derive(Clone, Copy, Debug)]
pub struct ProcessSupervisor {
    policy: SupervisorPolicy,
    host_isolation: HostIsolation,
}

impl ProcessSupervisor {
    /// Creates a supervisor with an explicit policy and trusted host isolation report.
    #[must_use]
    pub const fn new(policy: SupervisorPolicy, host_isolation: HostIsolation) -> Self {
        Self {
            policy,
            host_isolation,
        }
    }

    /// Runs one provider with bounded pipes, deadline/cancellation and tree cleanup.
    ///
    /// # Errors
    ///
    /// Returns a typed isolation, spawn, pipe, wait, termination, or reap failure. Deadlines,
    /// cancellation, output limits, and non-zero provider exits are successful supervised outcomes.
    ///
    /// The request's deadline is the provider's own running time: it starts
    /// once the provider has started, not when this is called: on a busy
    /// Windows machine the start itself can take seconds (#419).
    pub async fn run(
        &self,
        request: ProcessRequest,
        cancellation: ProcessCancellation,
    ) -> Result<ProcessOutcome, ProcessError> {
        self.run_started_by(request, cancellation, start_child)
            .await
    }

    /// [`ProcessSupervisor::run`] with the operating-system start given, so
    /// the tests can make a start slow where every real one is fast.
    async fn run_started_by<S>(
        &self,
        request: ProcessRequest,
        cancellation: ProcessCancellation,
        start: S,
    ) -> Result<ProcessOutcome, ProcessError>
    where
        S: FnOnce(CommandWrap) -> std::io::Result<Box<dyn ChildWrapper>> + Send + 'static,
    {
        let hard_isolation = effective_hard_isolation(self.host_isolation);
        let strict_required = request.isolation == IsolationRequirement::StrictWorker
            || self.host_isolation.minimum_requirement() == IsolationRequirement::StrictWorker;
        if strict_required && hard_isolation != HardIsolation::InheritedStrictLinux {
            return Err(ProcessError::IsolationUnavailable);
        }
        if cancellation.is_cancelled() {
            return Err(ProcessError::CancelledBeforeSpawn);
        }

        let controls = EffectiveControls {
            executable_provenance: request.executable.provenance(),
            process_containment: platform_containment(),
            hard_isolation,
            stream_limit: self.policy.stdout_limit.get(),
            stderr_limit: self.policy.stderr_limit.get(),
            null_stdin: ControlStatus::Applied,
            cleared_environment: ControlStatus::Applied,
            explicit_working_directory: ControlStatus::Applied,
            graceful_tree_stop: if cfg!(unix) {
                ControlStatus::Applied
            } else {
                ControlStatus::NotApplied
            },
            interrupt_isolation: platform_interrupt_isolation(),
        };
        if controls.process_containment == ProcessContainment::Unsupported {
            return Err(ProcessError::IsolationUnavailable);
        }

        // Boxed: the start's state would otherwise be part of every caller's
        // future, and the callers' futures are already large.
        let (mut spawned, deadline) =
            Box::pin(self.start_with_deadline(request, &cancellation, start)).await?;
        let mut captures = StreamCaptures::default();

        let trigger = wait_for_trigger(
            spawned.child.as_mut(),
            &mut spawned.signals,
            &mut spawned.drains,
            &mut captures,
            &cancellation,
            deadline,
        )
        .await;

        let (status, termination) = match trigger {
            Trigger::Completed(status) => (
                status,
                completed_termination(status, cancellation.is_cancelled()),
            ),
            Trigger::Deadline => (
                terminate_and_reap(spawned.child.as_mut(), self.policy, &cancellation).await?,
                TerminationReason::Deadline,
            ),
            Trigger::Cancelled => (
                terminate_and_reap(spawned.child.as_mut(), self.policy, &cancellation).await?,
                TerminationReason::Cancelled,
            ),
            Trigger::OutputLimit(stream) => (
                terminate_and_reap(spawned.child.as_mut(), self.policy, &cancellation).await?,
                TerminationReason::OutputLimit(stream),
            ),
            Trigger::ProcessWaitFailed(error) => {
                force_terminate_and_reap(spawned.child.as_mut(), self.policy.forced_shutdown)
                    .await?;
                spawned.drains.abort_all();
                return Err(ProcessError::Wait(error));
            }
            Trigger::StreamReadFailed(stream, error) => {
                force_terminate_and_reap(spawned.child.as_mut(), self.policy.forced_shutdown)
                    .await?;
                spawned.drains.abort_all();
                return Err(ProcessError::StreamRead(stream, error));
            }
            Trigger::OutputTaskFailed => {
                force_terminate_and_reap(spawned.child.as_mut(), self.policy.forced_shutdown)
                    .await?;
                spawned.drains.abort_all();
                return Err(ProcessError::OutputTaskFailed);
            }
        };

        collect_after_exit(
            &mut spawned.drains,
            &mut captures,
            self.policy.forced_shutdown,
        )
        .await?;
        spawned.drains.abort_all();

        Ok(ProcessOutcome {
            status,
            stdout: captures.stdout.unwrap_or_default(),
            stderr: captures.stderr.unwrap_or_default(),
            termination,
            controls,
        })
    }
}

impl ProcessSupervisor {
    /// Starts the provider and takes its deadline once it runs (#419).
    ///
    /// A start can take seconds on a busy Windows machine
    /// ([`start_off_runtime`]). Counted against the deadline, it left a
    /// provider given N seconds N minus the start, and a provider that would
    /// have answered at once was reported as one that did not answer in time
    /// (#128). The deadline is checked before the start too, so one the clock
    /// cannot hold never starts a provider.
    async fn start_with_deadline<S>(
        &self,
        request: ProcessRequest,
        cancellation: &ProcessCancellation,
        start: S,
    ) -> Result<(SpawnedProcess, Instant), ProcessError>
    where
        S: FnOnce(CommandWrap) -> std::io::Result<Box<dyn ChildWrapper>> + Send + 'static,
    {
        let running_time = request.deadline;
        if Instant::now().checked_add(running_time).is_none() {
            return Err(ProcessError::DeadlineOutOfRange);
        }
        let mut spawned = spawn_process(
            request,
            self.policy.stdout_limit.get(),
            self.policy.stderr_limit.get(),
            Starting {
                cancellation,
                forced_shutdown: self.policy.forced_shutdown,
            },
            start,
        )
        .await?;
        let Some(deadline) = Instant::now().checked_add(running_time) else {
            force_terminate_and_reap(spawned.child.as_mut(), self.policy.forced_shutdown).await?;
            spawned.drains.abort_all();
            return Err(ProcessError::DeadlineOutOfRange);
        };
        Ok((spawned, deadline))
    }
}

struct SpawnedProcess {
    child: Box<dyn ChildWrapper>,
    signals: mpsc::Receiver<StreamSignal>,
    drains: JoinSet<DrainResult>,
}

async fn spawn_process<S>(
    request: ProcessRequest,
    stdout_limit: usize,
    stderr_limit: usize,
    starting: Starting<'_>,
    start: S,
) -> Result<SpawnedProcess, ProcessError>
where
    S: FnOnce(CommandWrap) -> std::io::Result<Box<dyn ChildWrapper>> + Send + 'static,
{
    let ProcessRequest {
        executable,
        arguments,
        environment,
        working_directory,
        deadline: _,
        isolation: _,
    } = request;
    let mut command = Command::new(executable.path());
    command
        .args(arguments)
        .env_clear()
        .envs(environment)
        .current_dir(working_directory.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut wrapped = CommandWrap::from(command);
    add_platform_containment(&mut wrapped);
    wrapped.wrap(KillOnDrop);
    let mut child = start_off_runtime(wrapped, starting, start).await?;
    let stdout = child
        .stdout()
        .take()
        .ok_or(ProcessError::PipeUnavailable(OutputStream::Stdout))?;
    let stderr = child
        .stderr()
        .take()
        .ok_or(ProcessError::PipeUnavailable(OutputStream::Stderr))?;

    let (signal_sender, signals) = mpsc::channel(4);
    let mut drains = JoinSet::new();
    drains.spawn(drain_stream(
        OutputStream::Stdout,
        stdout,
        stdout_limit,
        signal_sender.clone(),
    ));
    drains.spawn(drain_stream(
        OutputStream::Stderr,
        stderr,
        stderr_limit,
        signal_sender,
    ));
    Ok(SpawnedProcess {
        child,
        signals,
        drains,
    })
}

/// The operating-system start of a fully configured provider command.
fn start_child(mut command: CommandWrap) -> std::io::Result<Box<dyn ChildWrapper>> {
    command.spawn()
}

/// Runs `start` on the runtime's blocking pool, so that the start never
/// holds the thread that drives other requests' timers, pipes and
/// cancellations (#419).
///
/// The start blocks, and on Windows it can block for seconds. `process-wrap`
/// 10 creates the provider suspended, assigns it to its Job Object and then
/// resumes it by walking a `CreateToolhelp32Snapshot` of every thread on the
/// machine, one `Thread32Next` call per thread. Measured on a Windows 11
/// desktop with about 17,500 threads (release build, 2026-10-17): the snapshot
/// takes about 30 ms and the walk about 330 ms; six walks at once in one
/// process take 2.4 to 3.2 s each, because they wait for each other inside
/// the process, while six in separate processes take about 0.4 s each. On a
/// runtime thread that time stopped every other task of that thread: four
/// requests on one current-thread runtime started one after another.
///
/// Moving the start does not make it shorter. Removing the walk needs the
/// provider's main thread, which neither `std` nor `tokio` hands out, or a
/// process-scoped thread list (`PssCaptureSnapshot`, about 0.5 ms here), and
/// either way a call that only `unsafe` code can make, which `VSift`'s crates
/// do not contain (L-170).
///
/// The start itself cannot be interrupted, but the run does not wait for it
/// once its caller cancels: it answers [`ProcessError::CancelledDuringStart`]
/// at once, and a task of the runtime ends the provider when the start
/// returns (killed and reaped through its tree, as any stopped run is). That
/// end comes after the run has answered, so for the rest of the start and the
/// kill the provider outlives its run (L-170). If the runtime shuts down
/// first, that task is dropped with it, and so is the child once the start
/// returns: `KillOnDrop` ends the provider (on Windows the kill-on-close job
/// ends its whole tree; on Unix only the provider itself is signalled).
///
/// A start that returns when the cancellation also has is taken: the run's
/// wait sees the cancellation first and ends the provider before it answers.
/// A start task that panicked, or that never ran because the runtime was
/// shutting down, is [`ProcessError::StartTaskFailed`]; a child it had made
/// is dropped and ended the same way.
async fn start_off_runtime<S>(
    command: CommandWrap,
    starting: Starting<'_>,
    start: S,
) -> Result<Box<dyn ChildWrapper>, ProcessError>
where
    S: FnOnce(CommandWrap) -> std::io::Result<Box<dyn ChildWrapper>> + Send + 'static,
{
    let mut task = tokio::task::spawn_blocking(move || start(command));
    tokio::select! {
        biased;
        started = &mut task => started
            .map_err(|_| ProcessError::StartTaskFailed)?
            .map_err(ProcessError::Spawn),
        () = starting.cancellation.cancelled() => {
            tokio::spawn(end_once_started(task, starting.forced_shutdown));
            Err(ProcessError::CancelledDuringStart)
        }
    }
}

/// What a start needs besides the command: the caller's cancellation, and
/// the forced budget for ending a provider whose start the caller left.
#[derive(Clone, Copy)]
struct Starting<'a> {
    cancellation: &'a ProcessCancellation,
    forced_shutdown: Duration,
}

/// Ends the provider of a start whose run was cancelled before the start
/// returned (see [`start_off_runtime`]). Nobody waits for this task, so a
/// failure to end the tree has no one to report to; the dropped child is
/// then ended by `KillOnDrop` and, on Windows, by its kill-on-close job.
async fn end_once_started(
    task: tokio::task::JoinHandle<std::io::Result<Box<dyn ChildWrapper>>>,
    forced_shutdown: Duration,
) {
    if let Ok(Ok(mut child)) = task.await {
        let _ = force_terminate_and_reap(child.as_mut(), forced_shutdown).await;
    }
}

impl Default for ProcessSupervisor {
    fn default() -> Self {
        Self::new(SupervisorPolicy::default(), HostIsolation::ProcessOnly)
    }
}

#[derive(Default)]
struct StreamCaptures {
    stdout: Option<CapturedOutput>,
    stderr: Option<CapturedOutput>,
}

impl StreamCaptures {
    fn is_complete(&self) -> bool {
        self.stdout.as_ref().is_some_and(|value| value.complete)
            && self.stderr.as_ref().is_some_and(|value| value.complete)
    }

    fn record(&mut self, stream: OutputStream, capture: CapturedOutput) {
        match stream {
            OutputStream::Stdout => self.stdout = Some(capture),
            OutputStream::Stderr => self.stderr = Some(capture),
        }
    }
}

enum StreamSignal {
    Limit(OutputStream, CapturedOutput),
    Failed(OutputStream, CapturedOutput, std::io::ErrorKind),
}

struct DrainResult {
    stream: OutputStream,
    capture: CapturedOutput,
    error: Option<std::io::Error>,
}

enum Trigger {
    Completed(ExitStatus),
    Deadline,
    Cancelled,
    OutputLimit(OutputStream),
    ProcessWaitFailed(std::io::Error),
    StreamReadFailed(OutputStream, std::io::Error),
    OutputTaskFailed,
}

async fn wait_for_trigger(
    child: &mut dyn ChildWrapper,
    signals: &mut mpsc::Receiver<StreamSignal>,
    drains: &mut JoinSet<DrainResult>,
    captures: &mut StreamCaptures,
    cancellation: &ProcessCancellation,
    deadline: Instant,
) -> Trigger {
    let wait = child.wait();
    tokio::pin!(wait);
    let deadline_sleep = sleep_until(deadline);
    tokio::pin!(deadline_sleep);

    loop {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => return Trigger::Cancelled,
            () = &mut deadline_sleep => return Trigger::Deadline,
            result = &mut wait => {
                match result {
                    Ok(status) => {
                        if captures.is_complete() {
                            return Trigger::Completed(status);
                        }
                        return wait_for_streams(
                            status,
                            signals,
                            drains,
                            captures,
                            cancellation,
                            deadline,
                        ).await;
                    }
                    Err(error) => return Trigger::ProcessWaitFailed(error),
                }
            }
            signal = next_signal(signals) => {
                return handle_stream_signal(signal, captures);
            }
            joined = drains.join_next(), if !drains.is_empty() => {
                if let Some(trigger) = handle_drain_result(joined, captures) {
                    return trigger;
                }
            }
        }
    }
}

async fn wait_for_streams(
    status: ExitStatus,
    signals: &mut mpsc::Receiver<StreamSignal>,
    drains: &mut JoinSet<DrainResult>,
    captures: &mut StreamCaptures,
    cancellation: &ProcessCancellation,
    deadline: Instant,
) -> Trigger {
    let deadline_sleep = sleep_until(deadline);
    tokio::pin!(deadline_sleep);
    loop {
        if captures.is_complete() {
            return Trigger::Completed(status);
        }
        tokio::select! {
            biased;
            () = cancellation.cancelled() => return Trigger::Cancelled,
            () = &mut deadline_sleep => return Trigger::Deadline,
            signal = next_signal(signals) => {
                return handle_stream_signal(signal, captures);
            }
            joined = drains.join_next(), if !drains.is_empty() => {
                if let Some(trigger) = handle_drain_result(joined, captures) {
                    return trigger;
                }
            }
        }
    }
}

async fn next_signal(signals: &mut mpsc::Receiver<StreamSignal>) -> StreamSignal {
    match signals.recv().await {
        Some(signal) => signal,
        None => pending().await,
    }
}

fn handle_stream_signal(signal: StreamSignal, captures: &mut StreamCaptures) -> Trigger {
    match signal {
        StreamSignal::Limit(stream, capture) => {
            captures.record(stream, capture);
            Trigger::OutputLimit(stream)
        }
        StreamSignal::Failed(stream, capture, kind) => {
            captures.record(stream, capture);
            Trigger::StreamReadFailed(
                stream,
                std::io::Error::new(kind, "provider output read failed"),
            )
        }
    }
}

fn handle_drain_result(
    joined: Option<Result<DrainResult, tokio::task::JoinError>>,
    captures: &mut StreamCaptures,
) -> Option<Trigger> {
    match joined {
        Some(Ok(result)) => {
            let stream = result.stream;
            let truncated = result.capture.truncated;
            captures.record(stream, result.capture);
            match result.error {
                Some(error) => Some(Trigger::StreamReadFailed(stream, error)),
                None if truncated => Some(Trigger::OutputLimit(stream)),
                None => None,
            }
        }
        Some(Err(_)) => Some(Trigger::OutputTaskFailed),
        None => None,
    }
}

async fn drain_stream<R>(
    stream: OutputStream,
    mut reader: R,
    limit: usize,
    signals: mpsc::Sender<StreamSignal>,
) -> DrainResult
where
    R: AsyncRead + Unpin,
{
    let mut retained = Vec::with_capacity(limit.min(8 * 1024));
    let mut buffer = [0_u8; 8 * 1024];
    let mut truncated = false;
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => {
                return DrainResult {
                    stream,
                    capture: CapturedOutput {
                        bytes: retained,
                        truncated,
                        complete: true,
                    },
                    error: None,
                };
            }
            Ok(read) => {
                let remaining = limit.saturating_sub(retained.len());
                let retained_from_chunk = remaining.min(read);
                retained.extend_from_slice(&buffer[..retained_from_chunk]);
                if retained_from_chunk < read && !truncated {
                    truncated = true;
                    let _ = signals
                        .send(StreamSignal::Limit(
                            stream,
                            CapturedOutput {
                                bytes: retained.clone(),
                                truncated: true,
                                complete: false,
                            },
                        ))
                        .await;
                }
            }
            Err(error) => {
                let capture = CapturedOutput {
                    bytes: retained,
                    truncated,
                    complete: false,
                };
                let _ = signals
                    .send(StreamSignal::Failed(stream, capture.clone(), error.kind()))
                    .await;
                return DrainResult {
                    stream,
                    capture,
                    error: Some(error),
                };
            }
        }
    }
}

async fn collect_after_exit(
    drains: &mut JoinSet<DrainResult>,
    captures: &mut StreamCaptures,
    drain_deadline: Duration,
) -> Result<(), ProcessError> {
    let collect = async {
        while let Some(joined) = drains.join_next().await {
            match joined {
                Ok(result) => {
                    let stream = result.stream;
                    captures.record(stream, result.capture);
                    if let Some(error) = result.error {
                        return Err(ProcessError::StreamRead(stream, error));
                    }
                }
                Err(_) => return Err(ProcessError::OutputTaskFailed),
            }
        }
        Ok(())
    };
    if let Ok(result) = timeout(drain_deadline, collect).await {
        result
    } else {
        drains.abort_all();
        Ok(())
    }
}

/// Stops the tree gracefully where the platform can (Unix: `SIGTERM` to the
/// process group, at most the graceful budget, cut short by an escalated
/// cancellation), then kills and reaps it within the forced budget.
async fn terminate_and_reap(
    child: &mut dyn ChildWrapper,
    policy: SupervisorPolicy,
    cancellation: &ProcessCancellation,
) -> Result<ExitStatus, ProcessError> {
    #[cfg(unix)]
    if !cancellation.is_escalated() {
        const SIGTERM: i32 = 15;
        if child.signal(SIGTERM).is_ok() {
            let graceful = tokio::select! {
                biased;
                () = cancellation.escalated() => None,
                result = timeout(policy.graceful_shutdown, child.wait()) => result.ok(),
            };
            if let Some(result) = graceful {
                return result.map_err(ProcessError::Wait);
            }
        }
    }

    // Windows has no graceful tree stop: the Job Object is killed at once.
    #[cfg(not(unix))]
    let _ = (policy.graceful_shutdown, cancellation);

    force_terminate_and_reap(child, policy.forced_shutdown).await
}

/// Kills the tree and reaps it within `forced_shutdown`.
///
/// What the reap waits for is the provider (the group's leader) and the
/// members of its group that are `VSift`'s own children; on Unix a
/// descendant that was re-parented (its parent exited) is not `VSift`'s to
/// wait for, so the reap does not cover it, and only the group signal
/// reaches it. On Windows the job's end covers the whole tree.
///
/// **macOS only** (#436): a kill refused with `PermissionDenied` is not taken
/// as a failure at once; the reap decides. A signal to a process group whose
/// members have all exited but are not yet reaped is refused there with
/// `EPERM` (XNU's `killpg1` leaves zombies out of the group it signals and
/// answers `EPERM`, not `ESRCH`, when it found no one), so a provider that
/// ended just as its deadline or cancellation came made the run fail with
/// `Terminate(PermissionDenied)`. A provider reaped within the budget has
/// ended, and its status is the answer; one that is not keeps the refusal as
/// its failure, after the budget (up to 5 s by default) rather than at once.
/// The same `EPERM` comes from a group with a live member `VSift` may not
/// signal: if its leader has exited, the run is reported as ended while that
/// member lives (L-177). Elsewhere a refused kill fails at once, as before:
/// Linux signals zombies, so it never answers this way for a group that has
/// ended.
async fn force_terminate_and_reap(
    child: &mut dyn ChildWrapper,
    forced_shutdown: Duration,
) -> Result<ExitStatus, ProcessError> {
    let refused = match child.start_kill() {
        Ok(()) => None,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::InvalidInput | std::io::ErrorKind::NotFound
            ) =>
        {
            None
        }
        #[cfg(target_os = "macos")]
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Some(error),
        Err(error) => return Err(ProcessError::Terminate(error)),
    };
    match timeout(forced_shutdown, child.wait()).await {
        Ok(result) => result.map_err(ProcessError::Wait),
        Err(_) => Err(refused.map_or(ProcessError::ReapDeadlineExceeded, ProcessError::Terminate)),
    }
}

/// Why a provider that ended by itself stopped.
///
/// A provider that failed after the caller asked the operation to stop is
/// reported as cancelled, not as its own failure: the two can meet (a
/// provider failing on its own just as the caller cancels, or a signal from
/// outside `VSift` that reaches the provider and the host together), and
/// reporting that as a decoding or recognition failure would record a false
/// gap (an `undecodable` window, a failed chunk counting towards poisoning
/// it) for media that is fine. A host's own console interrupt no longer
/// reaches a provider at all (see `add_platform_containment`, #404). A
/// provider that succeeded keeps its result; the caller's next boundary sees
/// the cancellation.
#[must_use]
pub fn completed_termination(status: ExitStatus, cancel_requested: bool) -> TerminationReason {
    if cancel_requested && !status.success() {
        TerminationReason::Cancelled
    } else {
        TerminationReason::Exited
    }
}

fn validate_environment_name(name: &OsStr) -> Result<(), ProcessRequestError> {
    let name = name.to_string_lossy();
    let mut characters = name.chars();
    let starts_validly = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic());
    if !starts_validly
        || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        return Err(ProcessRequestError::InvalidEnvironmentName);
    }
    Ok(())
}

/// Windows: the provider runs in a Job Object (its whole tree is killed with
/// the job, and with `VSift` when `VSift` ends) and in a console of its own,
/// created without a window.
///
/// The console of its own is what keeps a stop the supervisor's. A console
/// Ctrl-C or Ctrl-Break reaches every process attached to the console, so a
/// provider sharing `VSift`'s console died of the very interrupt that stopped
/// `VSift` (whisper.cpp ends with `STATUS_CONTROL_C_EXIT`, `FFmpeg` stops and
/// exits with code 255, measured on Windows 11). When that death was seen
/// before `VSift` had recorded its own cancellation, the run judged it as the
/// provider's own failure (a recognizer that ended abnormally, audio that
/// could not be decoded) and a request the shutdown had stopped was recorded
/// as failed (#404). In a console of its own a provider hears no interrupt; it stops
/// when the supervisor kills its job, and that stop is always the
/// cancellation. `CREATE_NO_WINDOW` rather than `DETACHED_PROCESS`: a
/// provider that starts console programs of its own (a batch-file wrapper,
/// say) gives them its hidden console instead of a new visible window each.
#[cfg(windows)]
fn add_platform_containment(command: &mut CommandWrap) {
    command.wrap(CreationFlags(CREATE_NO_WINDOW));
    command.wrap(JobObject);
}

#[cfg(unix)]
fn add_platform_containment(command: &mut CommandWrap) {
    command.wrap(ProcessGroup::leader());
}

#[cfg(not(any(unix, windows)))]
fn add_platform_containment(_command: &mut CommandWrap) {}

/// Whether this platform's containment also keeps the host's interrupts from
/// the provider: a process group of its own on Unix, a console of its own on
/// Windows (`add_platform_containment`).
const fn platform_interrupt_isolation() -> ControlStatus {
    if cfg!(any(unix, windows)) {
        ControlStatus::Applied
    } else {
        ControlStatus::NotApplied
    }
}

const fn platform_containment() -> ProcessContainment {
    #[cfg(windows)]
    {
        ProcessContainment::WindowsJobObject
    }
    #[cfg(unix)]
    {
        ProcessContainment::UnixProcessGroup
    }
    #[cfg(not(any(unix, windows)))]
    {
        ProcessContainment::Unsupported
    }
}

const fn effective_hard_isolation(host: HostIsolation) -> HardIsolation {
    match host {
        HostIsolation::ProcessOnly => HardIsolation::NotEnforced,
        #[cfg(target_os = "linux")]
        HostIsolation::StrictLinux => HardIsolation::InheritedStrictLinux,
    }
}

/// A typed invalid process-request failure.
#[derive(Debug)]
pub enum ProcessRequestError {
    /// The working directory path was relative.
    WorkingDirectoryNotAbsolute,
    /// The working directory could not be inspected.
    WorkingDirectoryInspection(std::io::Error),
    /// The working directory path did not identify a directory.
    WorkingDirectoryNotDirectory,
    /// A zero-duration execution deadline was supplied.
    DeadlineMustBePositive,
    /// The execution deadline could not be represented by the host monotonic clock.
    DeadlineOutOfRange,
    /// An environment variable name was not portable ASCII identifier syntax.
    InvalidEnvironmentName,
    /// An environment variable value contained a null character.
    InvalidEnvironmentValue,
}

impl fmt::Display for ProcessRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorkingDirectoryNotAbsolute => {
                formatter.write_str("process working directory must be absolute")
            }
            Self::WorkingDirectoryInspection(_) => {
                formatter.write_str("process working directory could not be inspected")
            }
            Self::WorkingDirectoryNotDirectory => {
                formatter.write_str("process working directory is not a directory")
            }
            Self::DeadlineMustBePositive => {
                formatter.write_str("process deadline must be positive")
            }
            Self::DeadlineOutOfRange => formatter.write_str("process deadline is out of range"),
            Self::InvalidEnvironmentName => {
                formatter.write_str("process environment name is invalid")
            }
            Self::InvalidEnvironmentValue => {
                formatter.write_str("process environment value is invalid")
            }
        }
    }
}

impl Error for ProcessRequestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::WorkingDirectoryInspection(error) => Some(error),
            Self::WorkingDirectoryNotAbsolute
            | Self::WorkingDirectoryNotDirectory
            | Self::DeadlineMustBePositive
            | Self::DeadlineOutOfRange
            | Self::InvalidEnvironmentName
            | Self::InvalidEnvironmentValue => None,
        }
    }
}

/// A typed process-supervision infrastructure failure.
#[derive(Debug)]
pub enum ProcessError {
    /// Required process-tree or strict-worker isolation was unavailable.
    IsolationUnavailable,
    /// Caller cancellation was observed before the operating-system spawn boundary.
    CancelledBeforeSpawn,
    /// The validated deadline was not representable by the runtime monotonic clock.
    DeadlineOutOfRange,
    /// The operating system could not create or contain the child.
    Spawn(std::io::Error),
    /// The caller cancelled while the child was being started. The run did
    /// not wait for the start; the child is ended once the start returns.
    CancelledDuringStart,
    /// The task that starts the child off the async runtime stopped without
    /// returning (it panicked, or the runtime shut down); any child it had
    /// made is ended with it.
    StartTaskFailed,
    /// A configured process pipe was not returned by the child API.
    PipeUnavailable(OutputStream),
    /// Reading a process pipe failed.
    StreamRead(OutputStream, std::io::Error),
    /// A process output drain task stopped without returning a typed result.
    OutputTaskFailed,
    /// Waiting for the process tree failed.
    Wait(std::io::Error),
    /// Forcefully terminating the process tree failed.
    Terminate(std::io::Error),
    /// The process tree could not be observed as reaped within the forced-stop deadline.
    ReapDeadlineExceeded,
}

impl fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IsolationUnavailable => formatter.write_str("required isolation is unavailable"),
            Self::CancelledBeforeSpawn => {
                formatter.write_str("provider process was cancelled before start")
            }
            Self::DeadlineOutOfRange => formatter.write_str("process deadline is out of range"),
            Self::Spawn(_) => formatter.write_str("provider process could not be started"),
            Self::CancelledDuringStart => {
                formatter.write_str("provider process was cancelled while it was starting")
            }
            Self::StartTaskFailed => {
                formatter.write_str("provider process start stopped unexpectedly")
            }
            Self::PipeUnavailable(_) => formatter.write_str("provider process pipe is unavailable"),
            Self::StreamRead(_, _) => {
                formatter.write_str("provider process output could not be read")
            }
            Self::OutputTaskFailed => {
                formatter.write_str("provider process output monitor stopped unexpectedly")
            }
            Self::Wait(_) => {
                formatter.write_str("provider process completion could not be observed")
            }
            Self::Terminate(_) => {
                formatter.write_str("provider process tree could not be terminated")
            }
            Self::ReapDeadlineExceeded => {
                formatter.write_str("provider process tree did not stop within the forced deadline")
            }
        }
    }
}

impl Error for ProcessError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Spawn(error)
            | Self::StreamRead(_, error)
            | Self::Wait(error)
            | Self::Terminate(error) => Some(error),
            Self::IsolationUnavailable
            | Self::CancelledBeforeSpawn
            | Self::DeadlineOutOfRange
            | Self::CancelledDuringStart
            | Self::StartTaskFailed
            | Self::PipeUnavailable(_)
            | Self::OutputTaskFailed
            | Self::ReapDeadlineExceeded => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        num::NonZeroUsize,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use process_wrap::tokio::{ChildWrapper, CommandWrap};

    use super::{
        HostIsolation, IsolationRequirement, ProcessCancellation, ProcessError, ProcessRequest,
        ProcessRequestError, ProcessSupervisor, ProcessWorkingDirectory, SupervisorPolicy,
        TerminationReason, completed_termination,
    };

    #[test]
    fn rejects_relative_working_directories() {
        let result = ProcessWorkingDirectory::new(PathBuf::from("relative"));

        assert!(matches!(
            result,
            Err(ProcessRequestError::WorkingDirectoryNotAbsolute)
        ));
    }

    #[test]
    fn rejects_invalid_environment_names() -> Result<(), Box<dyn std::error::Error>> {
        let executable = crate::TrustedExecutable::explicit(std::env::current_exe()?)?;
        let directory = ProcessWorkingDirectory::new(std::env::current_dir()?)?;
        let request = ProcessRequest::new(executable, directory, Duration::from_secs(1))?;

        let result = request.with_environment("SECRET=OTHER", "value");

        assert!(matches!(
            result,
            Err(ProcessRequestError::InvalidEnvironmentName)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn strict_isolation_fails_closed_without_host_attestation()
    -> Result<(), Box<dyn std::error::Error>> {
        let executable = crate::TrustedExecutable::explicit(std::env::current_exe()?)?;
        let directory = ProcessWorkingDirectory::new(std::env::current_dir()?)?;
        let request = ProcessRequest::new(executable, directory, Duration::from_secs(1))?
            .requiring_isolation(IsolationRequirement::StrictWorker);
        let limit = NonZeroUsize::new(1024).ok_or("test limit must be non-zero")?;
        let supervisor = ProcessSupervisor::new(
            SupervisorPolicy::new(limit, Duration::from_millis(10), Duration::from_secs(1)),
            HostIsolation::ProcessOnly,
        );

        let result = supervisor.run(request, ProcessCancellation::new()).await;

        assert!(matches!(result, Err(ProcessError::IsolationUnavailable)));
        Ok(())
    }

    #[tokio::test]
    async fn cancellation_is_sticky_across_subscription_races()
    -> Result<(), Box<dyn std::error::Error>> {
        for _ in 0..128 {
            let cancellation = ProcessCancellation::new();
            let waiter_signal = cancellation.clone();
            let waiter = tokio::spawn(async move {
                waiter_signal.cancelled().await;
            });
            tokio::task::yield_now().await;
            cancellation.cancel();

            tokio::time::timeout(Duration::from_secs(1), waiter).await??;
        }
        Ok(())
    }

    /// A provider that failed once the caller had cancelled is cancelled
    /// (it may have died of the same console interrupt); a success, or a
    /// failure nobody cancelled, is the provider's own exit.
    #[test]
    fn a_failure_after_cancellation_is_the_cancellation() {
        #[cfg(windows)]
        let (success, failure) = {
            use std::os::windows::process::ExitStatusExt;
            (
                std::process::ExitStatus::from_raw(0),
                std::process::ExitStatus::from_raw(0xC000_013A),
            )
        };
        #[cfg(unix)]
        let (success, failure) = {
            use std::os::unix::process::ExitStatusExt;
            (
                std::process::ExitStatus::from_raw(0),
                std::process::ExitStatus::from_raw(2),
            )
        };
        assert_eq!(
            completed_termination(failure, true),
            TerminationReason::Cancelled
        );
        assert_eq!(
            completed_termination(failure, false),
            TerminationReason::Exited
        );
        assert_eq!(
            completed_termination(success, true),
            TerminationReason::Exited
        );
    }

    /// A cancellation only moves forward: escalating implies cancelling, a
    /// later plain cancel keeps it escalated, and a waiter for the first
    /// step is released by the second.
    #[tokio::test]
    async fn escalation_implies_cancellation_and_never_goes_back()
    -> Result<(), Box<dyn std::error::Error>> {
        let cancellation = ProcessCancellation::new();
        assert!(!cancellation.is_cancelled() && !cancellation.is_escalated());
        let waiter_signal = cancellation.clone();
        let waiter = tokio::spawn(async move { waiter_signal.cancelled().await });
        tokio::task::yield_now().await;

        cancellation.escalate();
        tokio::time::timeout(Duration::from_secs(1), waiter).await??;
        cancellation.cancel();
        assert!(cancellation.is_cancelled() && cancellation.is_escalated());

        let plain = ProcessCancellation::new();
        plain.cancel();
        assert!(plain.is_cancelled() && !plain.is_escalated());
        Ok(())
    }

    /// A request for a child that ends at once: this test binary, asked to
    /// run a test that does not exist.
    fn quick_child(deadline: Duration) -> Result<ProcessRequest, Box<dyn std::error::Error>> {
        let executable = crate::TrustedExecutable::explicit(std::env::current_exe()?)?;
        let directory = ProcessWorkingDirectory::new(std::env::current_dir()?)?;
        Ok(ProcessRequest::new(executable, directory, deadline)?
            .with_arguments(["--exact", "no_test_has_this_name"]))
    }

    fn quick_supervisor() -> Result<ProcessSupervisor, Box<dyn std::error::Error>> {
        let limit = NonZeroUsize::new(64 * 1024).ok_or("test limit must be non-zero")?;
        Ok(ProcessSupervisor::new(
            SupervisorPolicy::new(limit, Duration::from_millis(100), Duration::from_secs(20)),
            HostIsolation::ProcessOnly,
        ))
    }

    /// #419: the deadline is the provider's own running time. A start that
    /// takes longer than the whole deadline (2.5 s here; a Windows start
    /// takes up to five on a busy desktop) still leaves the provider its two
    /// seconds, so a child that ends at once is `Exited`. When the
    /// deadline was taken before the start, it had passed before the child
    /// ran and the run was reported as `Deadline` (#128).
    #[tokio::test]
    async fn a_slow_start_is_not_taken_from_the_deadline() -> Result<(), Box<dyn std::error::Error>>
    {
        let slow_start = |mut command: CommandWrap| {
            std::thread::sleep(Duration::from_millis(2500));
            command.spawn()
        };

        let outcome = quick_supervisor()?
            .run_started_by(
                quick_child(Duration::from_secs(2))?,
                ProcessCancellation::new(),
                slow_start,
            )
            .await?;

        assert_eq!(outcome.termination, TerminationReason::Exited);
        assert!(outcome.status.success(), "the child failed: {outcome:?}");
        Ok(())
    }

    /// A started child that records whether the supervisor tried to kill it.
    #[derive(Debug)]
    struct RecordsKill {
        inner: Box<dyn ChildWrapper>,
        killed: Arc<AtomicBool>,
    }

    impl ChildWrapper for RecordsKill {
        fn inner(&self) -> &dyn ChildWrapper {
            self.inner.as_ref()
        }

        fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
            self.inner.as_mut()
        }

        fn into_inner(self: Box<Self>) -> Box<dyn ChildWrapper> {
            self.inner
        }

        fn start_kill(&mut self) -> std::io::Result<()> {
            self.killed.store(true, Ordering::SeqCst);
            self.inner.start_kill()
        }
    }

    /// #446 review: a cancellation while the provider is starting is
    /// answered at once, without waiting for a three-second start, and the
    /// provider is ended (killed and reaped) when the start returns.
    #[tokio::test]
    async fn a_cancellation_during_a_start_is_answered_at_once_and_ends_the_provider()
    -> Result<(), Box<dyn std::error::Error>> {
        let killed = Arc::new(AtomicBool::new(false));
        let start = {
            let killed = Arc::clone(&killed);
            move |mut command: CommandWrap| -> std::io::Result<Box<dyn ChildWrapper>> {
                std::thread::sleep(Duration::from_secs(3));
                let inner = command.spawn()?;
                Ok(Box::new(RecordsKill { inner, killed }))
            }
        };
        let cancellation = ProcessCancellation::new();
        let stopper = {
            let cancellation = cancellation.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                cancellation.cancel();
            })
        };
        let started = tokio::time::Instant::now();

        let result = quick_supervisor()?
            .run_started_by(quick_child(Duration::from_secs(60))?, cancellation, start)
            .await;
        let answered_after = started.elapsed();
        stopper.await?;

        assert!(
            matches!(result, Err(ProcessError::CancelledDuringStart)),
            "{result:?}"
        );
        assert!(
            answered_after < Duration::from_millis(2500),
            "the run waited for the start: {answered_after:?}"
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        while !killed.load(Ordering::SeqCst) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the provider was never ended"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    }

    /// #446 review: a start that never runs, because the runtime it was
    /// given to has shut down, is `StartTaskFailed`, not a panic, and
    /// nothing is started.
    #[test]
    fn a_start_on_a_runtime_that_has_shut_down_is_start_task_failed()
    -> Result<(), Box<dyn std::error::Error>> {
        let gone = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()?;
        let handle = gone.handle().clone();
        drop(gone);
        let ran = Arc::new(AtomicBool::new(false));
        let start = {
            let ran = Arc::clone(&ran);
            move |mut command: CommandWrap| {
                ran.store(true, Ordering::SeqCst);
                command.spawn()
            }
        };
        let driver = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let supervisor = quick_supervisor()?;
        let request = quick_child(Duration::from_secs(60))?;

        let result = driver.block_on(async {
            let _inside = handle.enter();
            supervisor
                .run_started_by(request, ProcessCancellation::new(), start)
                .await
        });

        assert!(
            matches!(result, Err(ProcessError::StartTaskFailed)),
            "{result:?}"
        );
        assert!(!ran.load(Ordering::SeqCst), "the start ran");
        Ok(())
    }

    /// #419: the start runs off the runtime's thread. On this current-thread
    /// runtime a ticker keeps ticking while a start blocks for a second;
    /// with the start on the runtime's own thread it could not tick at all
    /// until the start had returned.
    #[tokio::test]
    async fn the_runtime_keeps_running_while_a_provider_starts()
    -> Result<(), Box<dyn std::error::Error>> {
        let starting = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let ticker = {
            let starting = Arc::clone(&starting);
            let finished = Arc::clone(&finished);
            tokio::spawn(async move {
                let mut ticks_while_starting = 0_u32;
                while !finished.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    if starting.load(Ordering::SeqCst) {
                        ticks_while_starting += 1;
                    }
                }
                ticks_while_starting
            })
        };
        let start = {
            let starting = Arc::clone(&starting);
            move |mut command: CommandWrap| {
                starting.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_secs(1));
                starting.store(false, Ordering::SeqCst);
                command.spawn()
            }
        };

        let outcome = quick_supervisor()?
            .run_started_by(
                quick_child(Duration::from_secs(60))?,
                ProcessCancellation::new(),
                start,
            )
            .await;
        finished.store(true, Ordering::SeqCst);
        let ticks_while_starting = ticker.await?;

        assert_eq!(outcome?.termination, TerminationReason::Exited);
        assert!(
            ticks_while_starting >= 5,
            "the runtime ticked {ticks_while_starting} times during a one-second start"
        );
        Ok(())
    }

    /// P11 PR 2: a process-only host asks nothing more of a request; a
    /// strict worker host requires the strict boundary of every provider.
    #[test]
    fn a_host_sets_the_least_isolation_of_every_provider() {
        assert_eq!(
            HostIsolation::ProcessOnly.minimum_requirement(),
            IsolationRequirement::ProcessTree
        );
        #[cfg(target_os = "linux")]
        assert_eq!(
            HostIsolation::StrictLinux.minimum_requirement(),
            IsolationRequirement::StrictWorker
        );
    }

    /// P11: a request's child signal stops with the process-wide one, but
    /// cancelling one request never stops another or the process.
    #[tokio::test]
    async fn a_child_follows_its_parent_but_not_the_other_way()
    -> Result<(), Box<dyn std::error::Error>> {
        let process = ProcessCancellation::new();
        let first = process.child();
        let second = process.child();
        let nested = first.child();

        first.cancel();
        assert!(first.is_cancelled() && nested.is_cancelled());
        assert!(!process.is_cancelled() && !second.is_cancelled());

        let waiter_signal = second.clone();
        let waiter = tokio::spawn(async move { waiter_signal.cancelled().await });
        let escalated_signal = nested.clone();
        tokio::task::yield_now().await;
        process.escalate();
        tokio::time::timeout(Duration::from_secs(1), waiter).await??;
        assert!(second.is_cancelled() && second.is_escalated());
        assert!(escalated_signal.is_escalated());
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod unix_tests {
    use std::{num::NonZeroUsize, path::PathBuf, time::Duration};

    use tokio::time::Instant;

    use process_wrap::tokio::{CommandWrap, KillOnDrop};
    use rustix::process::{Pid, WaitId, WaitIdOptions, test_kill_process_group, waitid};

    use super::{
        HostIsolation, ProcessCancellation, ProcessRequest, ProcessSupervisor,
        ProcessWorkingDirectory, SupervisorPolicy, TerminationReason, add_platform_containment,
        terminate_and_reap,
    };

    /// #436: a provider's group whose leader has exited, and is not yet
    /// reaped, is ended by its reap with the leader's own status. On macOS
    /// both signals to such a group are refused with `EPERM` (XNU leaves
    /// zombies out of the group it signals), which made the run fail with
    /// `Terminate(PermissionDenied)` when a provider ended just as its
    /// deadline or cancellation came. Elsewhere the signals succeed, so this
    /// passes with or without the fix there.
    #[tokio::test]
    async fn a_group_whose_leader_has_exited_is_reaped_not_refused()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", "exit 7"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut wrapped = CommandWrap::from(command);
        add_platform_containment(&mut wrapped);
        wrapped.wrap(KillOnDrop);
        let mut child = wrapped.spawn()?;
        let pid = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .and_then(Pid::from_raw)
            .ok_or("the leader has no process id")?;
        // Waits, without reaping it, until the leader has exited.
        let deadline = Instant::now() + Duration::from_secs(60);
        while waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOWAIT | WaitIdOptions::NOHANG,
        )
        .map_err(|errno| std::io::Error::from_raw_os_error(errno.raw_os_error()))?
        .is_none()
        {
            if Instant::now() > deadline {
                return Err("the leader never exited".into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // The scenario itself: a signal to the group is refused on macOS
        // (no live member to signal) and accepted on Linux (zombies count).
        let probe = test_kill_process_group(pid);
        #[cfg(target_os = "macos")]
        assert_eq!(probe, Err(rustix::io::Errno::PERM), "macOS did not refuse");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(probe, Ok(()), "the group of an exited leader was refused");
        let limit = NonZeroUsize::new(1024).ok_or("test limit must be non-zero")?;
        let policy =
            SupervisorPolicy::new(limit, Duration::from_millis(100), Duration::from_secs(20));

        let status =
            terminate_and_reap(child.as_mut(), policy, &ProcessCancellation::new()).await?;

        assert_eq!(status.code(), Some(7));
        Ok(())
    }

    /// SEC-04 with a second request: a provider tree that ignores `SIGTERM`
    /// would hold a plain cancellation for the whole graceful budget; an
    /// escalation kills it at once, and the supervisor still returns only
    /// after reaping it.
    #[tokio::test]
    async fn an_escalated_cancellation_skips_the_graceful_wait()
    -> Result<(), Box<dyn std::error::Error>> {
        let shell = crate::TrustedExecutable::explicit(PathBuf::from("/bin/sh"))?;
        let directory = ProcessWorkingDirectory::new(std::env::temp_dir())?;
        let request = ProcessRequest::new(shell, directory, Duration::from_secs(60))?
            .with_arguments(["-c", "trap '' TERM; /bin/sleep 30"]);
        let limit = NonZeroUsize::new(1024).ok_or("test limit must be non-zero")?;
        let supervisor = ProcessSupervisor::new(
            SupervisorPolicy::new(limit, Duration::from_secs(30), Duration::from_secs(5)),
            HostIsolation::ProcessOnly,
        );
        let cancellation = ProcessCancellation::new();
        let signal = cancellation.clone();
        let started = Instant::now();
        let stopper = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            signal.cancel();
            tokio::time::sleep(Duration::from_millis(300)).await;
            signal.escalate();
        });

        let outcome = supervisor.run(request, cancellation).await?;
        stopper.await?;

        assert_eq!(outcome.termination, TerminationReason::Cancelled);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the graceful wait was not skipped"
        );
        Ok(())
    }
}

/// #436: a kill refused with `PermissionDenied` is decided by the reap.
#[cfg(test)]
mod refused_kill_tests {
    use std::{future::Future, io, pin::Pin, process::ExitStatus, time::Duration};

    use process_wrap::tokio::ChildWrapper;

    use super::{ProcessError, force_terminate_and_reap};

    /// A tree whose kill is refused with `PermissionDenied`, as macOS refuses
    /// the kill of a process group whose members have all exited; `ended`
    /// says whether its reap then finds it gone.
    #[derive(Debug)]
    struct RefusingTree {
        ended: bool,
    }

    impl ChildWrapper for RefusingTree {
        fn inner(&self) -> &dyn ChildWrapper {
            self
        }

        fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
            self
        }

        fn into_inner(self: Box<Self>) -> Box<dyn ChildWrapper> {
            self
        }

        fn start_kill(&mut self) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }

        fn wait(&mut self) -> Pin<Box<dyn Future<Output = io::Result<ExitStatus>> + Send + '_>> {
            let ended = self.ended;
            Box::pin(async move {
                if ended {
                    Ok(exited_with_one())
                } else {
                    std::future::pending().await
                }
            })
        }
    }

    fn exited_with_one() -> ExitStatus {
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(1)
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(1 << 8)
        }
    }

    /// macOS: the group had already gone, so the refusal is not a failure
    /// and the reap's status is the answer. Before #436 this was
    /// `Terminate(PermissionDenied)`. Elsewhere a refusal still fails at
    /// once, without waiting for the reap.
    #[tokio::test]
    async fn a_refused_kill_of_a_tree_that_has_ended_is_its_end_on_macos_only()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut tree = RefusingTree { ended: true };

        let result = force_terminate_and_reap(&mut tree, Duration::from_secs(20)).await;

        #[cfg(target_os = "macos")]
        assert_eq!(result?, exited_with_one());
        #[cfg(not(target_os = "macos"))]
        assert!(
            matches!(
                &result,
                Err(ProcessError::Terminate(error))
                    if error.kind() == io::ErrorKind::PermissionDenied
            ),
            "{result:?}"
        );
        Ok(())
    }

    /// A tree that refuses its kill and runs on is still a termination
    /// failure, with the refusal as its cause: on macOS once the budget is
    /// spent, elsewhere at once.
    #[tokio::test]
    async fn a_refused_kill_of_a_tree_that_runs_on_stays_a_termination_failure() {
        let mut tree = RefusingTree { ended: false };

        let result = force_terminate_and_reap(&mut tree, Duration::from_millis(100)).await;

        assert!(
            matches!(
                &result,
                Err(ProcessError::Terminate(error))
                    if error.kind() == io::ErrorKind::PermissionDenied
            ),
            "{result:?}"
        );
    }
}
