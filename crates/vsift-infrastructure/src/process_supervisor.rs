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

#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::{mpsc, watch},
    task::JoinSet,
    time::{Instant, sleep_until, timeout},
};

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

/// Immutable limits applied to every process run by one supervisor.
#[derive(Clone, Copy, Debug)]
pub struct SupervisorPolicy {
    stream_limit: NonZeroUsize,
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
            stream_limit,
            graceful_shutdown,
            forced_shutdown,
        }
    }

    /// Returns the maximum retained bytes for each output stream.
    #[must_use]
    pub const fn stream_limit(self) -> NonZeroUsize {
        self.stream_limit
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
#[derive(Clone, Debug)]
pub struct ProcessCancellation {
    sender: watch::Sender<CancellationState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CancellationState {
    Active,
    Cancelled,
}

impl ProcessCancellation {
    /// Creates an uncancelled signal.
    #[must_use]
    pub fn new() -> Self {
        let (sender, _) = watch::channel(CancellationState::Active);
        Self { sender }
    }

    /// Requests cancellation. Repeated requests are idempotent.
    pub fn cancel(&self) {
        self.sender.send_replace(CancellationState::Cancelled);
    }

    /// Reports whether cancellation was already requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.sender.borrow() == CancellationState::Cancelled
    }

    async fn cancelled(&self) {
        let mut receiver = self.sender.subscribe();
        loop {
            if *receiver.borrow_and_update() == CancellationState::Cancelled {
                return;
            }
            if receiver.changed().await.is_err() {
                return;
            }
        }
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
    /// The caller cancellation signal won the terminal transition.
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
    /// Per-stream retained-byte limit.
    pub stream_limit: usize,
    /// Whether stdin was explicitly connected to the null device.
    pub null_stdin: ControlStatus,
    /// Whether the inherited environment was cleared before adding allowlisted entries.
    pub cleared_environment: ControlStatus,
    /// Whether an explicit canonical working directory was applied.
    pub explicit_working_directory: ControlStatus,
    /// Whether the platform adapter can attempt a graceful tree-wide stop.
    pub graceful_tree_stop: ControlStatus,
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
    pub async fn run(
        &self,
        request: ProcessRequest,
        cancellation: ProcessCancellation,
    ) -> Result<ProcessOutcome, ProcessError> {
        let hard_isolation = effective_hard_isolation(self.host_isolation);
        if request.isolation == IsolationRequirement::StrictWorker
            && hard_isolation != HardIsolation::InheritedStrictLinux
        {
            return Err(ProcessError::IsolationUnavailable);
        }
        if cancellation.is_cancelled() {
            return Err(ProcessError::CancelledBeforeSpawn);
        }

        let controls = EffectiveControls {
            executable_provenance: request.executable.provenance(),
            process_containment: platform_containment(),
            hard_isolation,
            stream_limit: self.policy.stream_limit.get(),
            null_stdin: ControlStatus::Applied,
            cleared_environment: ControlStatus::Applied,
            explicit_working_directory: ControlStatus::Applied,
            graceful_tree_stop: if cfg!(unix) {
                ControlStatus::Applied
            } else {
                ControlStatus::NotApplied
            },
        };
        if controls.process_containment == ProcessContainment::Unsupported {
            return Err(ProcessError::IsolationUnavailable);
        }

        let deadline = Instant::now()
            .checked_add(request.deadline)
            .ok_or(ProcessError::DeadlineOutOfRange)?;
        let mut spawned = spawn_process(request, self.policy.stream_limit.get())?;
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
            Trigger::Completed(status) => (status, TerminationReason::Exited),
            Trigger::Deadline => (
                terminate_and_reap(spawned.child.as_mut(), self.policy).await?,
                TerminationReason::Deadline,
            ),
            Trigger::Cancelled => (
                terminate_and_reap(spawned.child.as_mut(), self.policy).await?,
                TerminationReason::Cancelled,
            ),
            Trigger::OutputLimit(stream) => (
                terminate_and_reap(spawned.child.as_mut(), self.policy).await?,
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

struct SpawnedProcess {
    child: Box<dyn ChildWrapper>,
    signals: mpsc::Receiver<StreamSignal>,
    drains: JoinSet<DrainResult>,
}

fn spawn_process(
    request: ProcessRequest,
    stream_limit: usize,
) -> Result<SpawnedProcess, ProcessError> {
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
    let mut child = wrapped.spawn().map_err(ProcessError::Spawn)?;
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
        stream_limit,
        signal_sender.clone(),
    ));
    drains.spawn(drain_stream(
        OutputStream::Stderr,
        stderr,
        stream_limit,
        signal_sender,
    ));
    Ok(SpawnedProcess {
        child,
        signals,
        drains,
    })
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

async fn terminate_and_reap(
    child: &mut dyn ChildWrapper,
    policy: SupervisorPolicy,
) -> Result<ExitStatus, ProcessError> {
    #[cfg(unix)]
    {
        const SIGTERM: i32 = 15;
        if child.signal(SIGTERM).is_ok()
            && let Ok(result) = timeout(policy.graceful_shutdown, child.wait()).await
        {
            return result.map_err(ProcessError::Wait);
        }
    }

    #[cfg(not(unix))]
    let _ = policy.graceful_shutdown;

    force_terminate_and_reap(child, policy.forced_shutdown).await
}

async fn force_terminate_and_reap(
    child: &mut dyn ChildWrapper,
    forced_shutdown: Duration,
) -> Result<ExitStatus, ProcessError> {
    if let Err(error) = child.start_kill()
        && error.kind() != std::io::ErrorKind::InvalidInput
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(ProcessError::Terminate(error));
    }
    match timeout(forced_shutdown, child.wait()).await {
        Ok(result) => result.map_err(ProcessError::Wait),
        Err(_) => Err(ProcessError::ReapDeadlineExceeded),
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

#[cfg(windows)]
fn add_platform_containment(command: &mut CommandWrap) {
    command.wrap(JobObject);
}

#[cfg(unix)]
fn add_platform_containment(command: &mut CommandWrap) {
    command.wrap(ProcessGroup::leader());
}

#[cfg(not(any(unix, windows)))]
fn add_platform_containment(_command: &mut CommandWrap) {}

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
            | Self::PipeUnavailable(_)
            | Self::OutputTaskFailed
            | Self::ReapDeadlineExceeded => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, path::PathBuf, time::Duration};

    use super::{
        HostIsolation, IsolationRequirement, ProcessCancellation, ProcessError, ProcessRequest,
        ProcessRequestError, ProcessSupervisor, ProcessWorkingDirectory, SupervisorPolicy,
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
}
