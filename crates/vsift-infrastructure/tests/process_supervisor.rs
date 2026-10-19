//! Cross-platform adversarial qualification tests for the secure process supervisor.

use std::{
    env,
    error::Error,
    ffi::OsString,
    fmt,
    io::{self, Write},
    num::{NonZeroUsize, ParseIntError},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant as StandardInstant},
};

#[cfg(target_os = "linux")]
use vsift_infrastructure::HardIsolation;
use vsift_infrastructure::{
    ControlStatus, ExecutableResolutionError, ExecutableResolver, HostIsolation, OutputStream,
    ProcessCancellation, ProcessContainment, ProcessError, ProcessRequest, ProcessSupervisor,
    ProcessWorkingDirectory, SupervisorPolicy, TerminationReason, TrustedExecutable,
};

const FIXTURE_MODE: &str = "VSIFT_PROCESS_FIXTURE_MODE";
const DESCENDANT_MARKER: &str = "VSIFT_DESCENDANT_MARKER";

/// The deadline of every run here whose fixture ends by itself or is ended by
/// the behaviour under test (an output limit, a cancellation).
///
/// It is a safety stop, not a timing claim: no test here says how fast a
/// provider starts, and each run returns as soon as its condition holds. It is
/// generous because the supervisor's deadline starts before the spawn, and on
/// Windows the spawn is slow and uneven (#128; the cost itself is #419,
/// L-170). `process-wrap`'s Job Object wrapper creates the child suspended and
/// resumes it by walking a snapshot of every thread on the machine: on a
/// Windows 11 desktop running about 20,000 threads that took 0.4 s for one
/// spawn, about 3 s for each of the six tests that spawn at once at the start
/// of this suite, and 3 to 5 s with other work on the machine. The three
/// seconds these tests used to give were spent before the fixture ran. A
/// broken behaviour still fails: a flood that is never limited, an exit that
/// is never seen or streams that never end reach this deadline and are
/// reported as `Deadline`, never as the outcome the test expects.
const FIXTURE_DEADLINE: Duration = Duration::from_secs(60);

/// How long a test waits for its fixture to report its descendant's process
/// id, which includes the slow spawn of [`FIXTURE_DEADLINE`]. The wait ends as
/// soon as the id is written.
const MARKER_WAIT: Duration = Duration::from_secs(60);

/// The deadline of a run that the test itself cancels once the fixture has
/// reported its descendant: longer than [`MARKER_WAIT`], so the test's
/// cancellation always comes before the supervisor's deadline.
const CANCELLED_FIXTURE_DEADLINE: Duration = Duration::from_secs(120);

/// How long the supervisor waits for a killed tree to be reaped and its
/// streams to close, and how long a test waits for a killed descendant to
/// leave the process table. Both waits end as soon as their condition holds.
/// Two of them together stay well under the 60 s the sleeping fixtures sleep,
/// so a descendant the supervisor failed to stop is still alive when the test
/// looks for it, and cannot pass the test by ending on its own.
const REAP_DEADLINE: Duration = Duration::from_secs(20);

#[test]
fn process_fixture() -> Result<(), Box<dyn Error>> {
    let Ok(mode) = env::var(FIXTURE_MODE) else {
        return Ok(());
    };

    match mode.as_str() {
        "inspect" => fixture_inspect(),
        "stdout-flood" => fixture_flood(OutputStream::Stdout),
        "stderr-flood" => fixture_flood(OutputStream::Stderr),
        "both-flood" => fixture_both_floods(),
        "exact-over" => {
            io::stdout().write_all(&vec![b'x'; 1025])?;
            std::thread::sleep(Duration::from_secs(60));
            Ok(())
        }
        "invalid-bytes" => {
            io::stdout().write_all(&[0xff, 0xfe, 0x00, 0x80])?;
            Ok(())
        }
        "no-newline" => {
            io::stdout().write_all(b"complete without newline")?;
            Ok(())
        }
        "delayed" => {
            std::thread::sleep(Duration::from_millis(100));
            io::stdout().write_all(b"delayed")?;
            Ok(())
        }
        "sleep" => {
            std::thread::sleep(Duration::from_secs(60));
            Ok(())
        }
        "announce-and-sleep" => fixture_announce_and_sleep(),
        "interrupt-host" => fixture_interrupt_host(),
        "descendant" => fixture_descendant(ParentLifecycle::Wait),
        "pipe-holder" => fixture_descendant(ParentLifecycle::Exit),
        #[cfg(target_os = "linux")]
        "strict-cpu-pressure" => fixture_cpu_pressure(),
        #[cfg(target_os = "linux")]
        "strict-memory-pressure" => fixture_memory_pressure(),
        #[cfg(target_os = "linux")]
        "strict-pid-pressure" => fixture_pid_pressure(),
        other => Err(format!("unknown fixture mode: {other}").into()),
    }
}

fn fixture_inspect() -> Result<(), Box<dyn Error>> {
    let current_directory = env::current_dir()?;
    let allowed = match env::var("VSIFT_ALLOWED") {
        Ok(value) => value,
        Err(_) => String::from("missing"),
    };
    writeln!(io::stdout(), "CWD={}", current_directory.display())?;
    writeln!(
        io::stdout(),
        "PATH_PRESENT={}",
        env::var_os("PATH").is_some()
    )?;
    writeln!(io::stdout(), "ALLOWED={allowed}")?;
    Ok(())
}

fn fixture_flood(stream: OutputStream) -> Result<(), Box<dyn Error>> {
    let bytes = vec![b'x'; 8 * 1024];
    loop {
        match stream {
            OutputStream::Stdout => io::stdout().write_all(&bytes)?,
            OutputStream::Stderr => io::stderr().write_all(&bytes)?,
        }
    }
}

fn fixture_both_floods() -> Result<(), Box<dyn Error>> {
    let bytes = vec![b'x'; 8 * 1024];
    loop {
        io::stdout().write_all(&bytes)?;
        io::stderr().write_all(&bytes)?;
    }
}

/// A provider in the middle of its work: it writes its process id to the
/// marker, then sleeps, as whisper.cpp does mid-chunk.
fn fixture_announce_and_sleep() -> Result<(), Box<dyn Error>> {
    let marker = env::var_os(DESCENDANT_MARKER).ok_or("no marker path was given")?;
    std::fs::write(marker, format!("{}\n", std::process::id()))?;
    std::thread::sleep(Duration::from_secs(60));
    Ok(())
}

/// How long the interrupt host waits between hearing the interrupt and
/// cancelling. A loaded runner's host can act on an interrupt well after the
/// operating system delivered it; the wait makes that late reaction certain,
/// so a provider that heard the same interrupt has ended long before the
/// cancellation exists.
const HOST_REACTION: Duration = Duration::from_secs(1);

/// How long the interrupt test waits for each line of its host: a safety
/// stop, not a timing claim. The first line comes after the host's provider
/// has started and written its marker, which includes the slow Windows spawn
/// of [`MARKER_WAIT`]; each wait ends as soon as its line is printed.
const HOST_WAIT: Duration = Duration::from_secs(120);

/// A host as `vsift` is one: it runs a provider under the supervisor, and an
/// interrupt (a console Ctrl-Break on Windows, `SIGINT` to its process group
/// on Unix) makes it cancel, [`HOST_REACTION`] later. It prints `HOST_READY`
/// once the provider is at work and `TERMINATION=<reason>` at the end.
fn fixture_interrupt_host() -> Result<(), Box<dyn Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let mut interrupt = HostInterrupt::listen()?;
        let marker = PathBuf::from(env::var_os(DESCENDANT_MARKER).ok_or("no marker path")?);
        let request = fixture_request("announce-and-sleep", CANCELLED_FIXTURE_DEADLINE)?
            .with_environment(DESCENDANT_MARKER, marker.as_os_str())?;
        let cancellation = ProcessCancellation::new();
        let trigger = cancellation.clone();
        let supervisor = supervisor(64 * 1024)?;
        let run = tokio::spawn(async move { supervisor.run(request, cancellation).await });
        wait_for_descendant_marker(&marker, MARKER_WAIT).await?;
        writeln!(io::stdout(), "HOST_READY")?;
        io::stdout().flush()?;
        interrupt.next().await;
        tokio::time::sleep(HOST_REACTION).await;
        trigger.cancel();
        let outcome = run.await??;
        writeln!(io::stdout(), "TERMINATION={:?}", outcome.termination)?;
        io::stdout().flush()?;
        Ok::<(), Box<dyn Error>>(())
    })
}

/// The interrupt the host listens for.
#[cfg(windows)]
struct HostInterrupt(tokio::signal::windows::CtrlBreak);

#[cfg(windows)]
impl HostInterrupt {
    fn listen() -> io::Result<Self> {
        tokio::signal::windows::ctrl_break().map(Self)
    }

    async fn next(&mut self) {
        let _ = self.0.recv().await;
    }
}

/// The interrupt the host listens for.
#[cfg(unix)]
struct HostInterrupt(tokio::signal::unix::Signal);

#[cfg(unix)]
impl HostInterrupt {
    fn listen() -> io::Result<Self> {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).map(Self)
    }

    async fn next(&mut self) {
        let _ = self.0.recv().await;
    }
}

#[derive(Clone, Copy)]
enum ParentLifecycle {
    Wait,
    Exit,
}

fn fixture_descendant(parent_lifecycle: ParentLifecycle) -> Result<(), Box<dyn Error>> {
    let executable = env::current_exe()?;
    let mut command = Command::new(executable);
    command
        .args([
            "--exact",
            "process_fixture",
            "--nocapture",
            "--test-threads=1",
        ])
        .env_clear()
        .env(FIXTURE_MODE, "sleep")
        .stdin(Stdio::null());
    if matches!(parent_lifecycle, ParentLifecycle::Wait) {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let mut child = command.spawn()?;
    if let Some(marker) = env::var_os(DESCENDANT_MARKER) {
        // The line feed is what tells the reader that the id is whole: creating
        // the file and writing it are two steps, and the test can read between
        // them (#321). See `parse_descendant_marker`.
        std::fs::write(marker, format!("{}\n", child.id()))?;
    }
    writeln!(io::stdout(), "DESCENDANT_PID={}", child.id())?;
    io::stdout().flush()?;
    if matches!(parent_lifecycle, ParentLifecycle::Exit) {
        return Ok(());
    }
    let _ = child.wait()?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn fixture_cpu_pressure() -> Result<(), Box<dyn Error>> {
    let deadline = StandardInstant::now() + Duration::from_secs(2);
    let mut workers = Vec::new();
    for _ in 0..4 {
        workers.push(std::thread::spawn(move || {
            let mut value = 0_u64;
            while StandardInstant::now() < deadline {
                value = value.wrapping_add(1);
                std::hint::black_box(value);
            }
        }));
    }
    for worker in workers {
        worker
            .join()
            .map_err(|_| "CPU pressure worker stopped unexpectedly")?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn fixture_memory_pressure() -> Result<(), Box<dyn Error>> {
    const BLOCK_SIZE: usize = 8 * 1024 * 1024;
    const BLOCK_COUNT: usize = 96;
    let mut allocations = Vec::with_capacity(BLOCK_COUNT);
    for _ in 0..BLOCK_COUNT {
        let mut block = vec![0_u8; BLOCK_SIZE];
        for byte in block.iter_mut().step_by(4096) {
            *byte = 1;
        }
        allocations.push(block);
    }
    std::hint::black_box(&allocations);
    Err("memory pressure unexpectedly exceeded the worker limit".into())
}

#[cfg(target_os = "linux")]
fn fixture_pid_pressure() -> Result<(), Box<dyn Error>> {
    let sleep = Path::new("/bin/sleep");
    if !sleep.is_file() {
        return Err("strict PID pressure fixture requires /bin/sleep".into());
    }

    let mut children = Vec::new();
    let mut limit_observed = false;
    for _ in 0..128 {
        if let Ok(child) = Command::new(sleep)
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            children.push(child);
        } else {
            limit_observed = true;
            break;
        }
    }

    let mut cleanup_failure = None;
    for child in &mut children {
        if let Err(error) = child.kill()
            && error.kind() != io::ErrorKind::InvalidInput
            && cleanup_failure.is_none()
        {
            cleanup_failure = Some(error);
        }
        if let Err(error) = child.wait()
            && cleanup_failure.is_none()
        {
            cleanup_failure = Some(error);
        }
    }
    if let Some(error) = cleanup_failure {
        return Err(error.into());
    }
    if !limit_observed {
        return Err("PID pressure did not reach the worker limit".into());
    }
    Ok(())
}

#[tokio::test]
async fn p01_uses_exact_arguments_null_stdin_allowlisted_environment_and_explicit_cwd()
-> Result<(), Box<dyn Error>> {
    let request = fixture_request("inspect", FIXTURE_DEADLINE)?
        .with_environment("VSIFT_ALLOWED", "expected")?;
    let outcome = supervisor(64 * 1024)?
        .run(request, ProcessCancellation::new())
        .await?;
    let stdout = String::from_utf8_lossy(&outcome.stdout.bytes);

    assert!(outcome.status.success());
    assert_eq!(outcome.termination, TerminationReason::Exited);
    assert_eq!(outcome.controls.null_stdin, ControlStatus::Applied);
    assert_eq!(outcome.controls.cleared_environment, ControlStatus::Applied);
    assert_eq!(
        outcome.controls.explicit_working_directory,
        ControlStatus::Applied
    );
    if cfg!(windows) {
        assert_eq!(
            outcome.controls.process_containment,
            ProcessContainment::WindowsJobObject
        );
    } else {
        assert_eq!(
            outcome.controls.process_containment,
            ProcessContainment::UnixProcessGroup
        );
    }
    assert_eq!(outcome.controls.interrupt_isolation, ControlStatus::Applied);
    assert!(stdout.contains("PATH_PRESENT=false"));
    assert!(stdout.contains("ALLOWED=expected"));
    Ok(())
}

#[tokio::test]
async fn p01_shell_metacharacters_remain_one_argument() -> Result<(), Box<dyn Error>> {
    let marker = unique_marker_path("injection-marker");
    let executable = TrustedExecutable::explicit(env::current_exe()?)?;
    let working_directory = ProcessWorkingDirectory::new(env::current_dir()?)?;
    let malicious_filter = format!("process_fixture & echo compromised > {}", marker.display());
    let request = ProcessRequest::new(executable, working_directory, FIXTURE_DEADLINE)?
        .with_arguments(["--exact", malicious_filter.as_str(), "--nocapture"]);

    let outcome = supervisor(64 * 1024)?
        .run(request, ProcessCancellation::new())
        .await?;

    assert!(outcome.status.success());
    assert!(!marker.exists());
    Ok(())
}

#[test]
fn p02_resolver_rejects_relative_paths_and_provider_components() -> Result<(), Box<dyn Error>> {
    let relative = ExecutableResolver::from_directories([PathBuf::from(".")]);
    assert!(matches!(
        relative,
        Err(ExecutableResolutionError::SearchDirectoryNotAbsolute)
    ));

    let resolver = ExecutableResolver::from_directories([env::current_dir()?])?;
    let traversal = resolver.resolve(Path::new("../hostile").as_os_str());
    assert!(matches!(
        traversal,
        Err(ExecutableResolutionError::InvalidProviderName)
    ));
    Ok(())
}

#[tokio::test]
async fn p03_caps_stdout_stderr_and_combined_floods_during_read() -> Result<(), Box<dyn Error>> {
    for (mode, expected_stream) in [
        ("stdout-flood", Some(OutputStream::Stdout)),
        ("stderr-flood", Some(OutputStream::Stderr)),
        ("both-flood", None),
        ("exact-over", Some(OutputStream::Stdout)),
    ] {
        let request = fixture_request(mode, FIXTURE_DEADLINE)?;
        let outcome = supervisor(1024)?
            .run(request, ProcessCancellation::new())
            .await?;

        let TerminationReason::OutputLimit(actual_stream) = outcome.termination else {
            return Err(format!("{mode} did not terminate at its output limit").into());
        };
        if let Some(expected_stream) = expected_stream {
            assert_eq!(actual_stream, expected_stream);
        }
        assert!(outcome.stdout.bytes.len() <= 1024);
        assert!(outcome.stderr.bytes.len() <= 1024);
    }
    Ok(())
}

#[tokio::test]
async fn p04_preserves_invalid_bytes_and_handles_no_newline_and_delayed_output()
-> Result<(), Box<dyn Error>> {
    for mode in ["invalid-bytes", "no-newline", "delayed"] {
        let request = fixture_request(mode, FIXTURE_DEADLINE)?;
        let outcome = supervisor(64 * 1024)?
            .run(request, ProcessCancellation::new())
            .await?;

        assert_eq!(outcome.termination, TerminationReason::Exited);
        assert!(outcome.stdout.complete);
        assert_ne!(outcome.stdout.bytes, [] as [u8; 0]);
    }
    Ok(())
}

#[tokio::test]
async fn p05_deadline_and_caller_cancellation_reap_the_process_tree() -> Result<(), Box<dyn Error>>
{
    let pre_cancelled = ProcessCancellation::new();
    pre_cancelled.cancel();
    let pre_cancelled_request = fixture_request("inspect", Duration::from_secs(1))?;
    let pre_cancelled_result = supervisor(64 * 1024)?
        .run(pre_cancelled_request, pre_cancelled)
        .await;
    assert!(matches!(
        pre_cancelled_result,
        Err(ProcessError::CancelledBeforeSpawn)
    ));

    let deadline_request = fixture_request("sleep", Duration::from_millis(100))?;
    let deadline_outcome = supervisor(64 * 1024)?
        .run(deadline_request, ProcessCancellation::new())
        .await?;
    assert_eq!(deadline_outcome.termination, TerminationReason::Deadline);

    let cancellation = ProcessCancellation::new();
    let cancellation_trigger = cancellation.clone();
    let cancellation_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancellation_trigger.cancel();
    });
    let cancelled_request = fixture_request("sleep", FIXTURE_DEADLINE)?;
    let cancelled = supervisor(64 * 1024)?
        .run(cancelled_request, cancellation)
        .await;
    cancellation_task.await?;
    // A cancellation that lands while the provider is still starting (a
    // Windows start takes 0.4 s or more on a desktop, L-170) is answered at
    // once, and the provider is ended when its start returns (#446 review;
    // that end is pinned by the supervisor's unit tests). One that lands
    // once it runs ends and reaps it before the run answers.
    match cancelled {
        Ok(outcome) => assert_eq!(outcome.termination, TerminationReason::Cancelled),
        Err(ProcessError::CancelledDuringStart) => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[tokio::test]
async fn p06_descendants_and_inherited_pipe_holders_are_terminated() -> Result<(), Box<dyn Error>> {
    for mode in ["descendant", "pipe-holder"] {
        let marker = unique_marker_path(mode);
        let request = fixture_request(mode, CANCELLED_FIXTURE_DEADLINE)?
            .with_environment(DESCENDANT_MARKER, marker.as_os_str())?;
        let cancellation = ProcessCancellation::new();
        let cancellation_trigger = cancellation.clone();
        let supervisor = supervisor(64 * 1024)?;
        let run = tokio::spawn(async move { supervisor.run(request, cancellation).await });
        let descendant_result = wait_for_descendant_marker(&marker, MARKER_WAIT).await;
        cancellation_trigger.cancel();
        let outcome = run.await??;
        let _ = std::fs::remove_file(&marker);
        let descendant_pid = descendant_result?;

        assert_eq!(outcome.termination, TerminationReason::Cancelled);
        assert!(wait_until_process_gone(descendant_pid, REAP_DEADLINE)?);
    }
    Ok(())
}

/// #404: an interrupt aimed at the host never reaches the provider, so the
/// provider stops only when the host cancels it, and the run is cancelled.
///
/// The host here hears the interrupt and cancels a second later
/// ([`HOST_REACTION`]). On Windows the interrupt is a console Ctrl-Break, which
/// reaches every process attached to the console: a provider that shared the
/// host's console died of it at once (`STATUS_CONTROL_C_EXIT`), the
/// supervisor saw a provider that ended abnormally before any cancellation
/// existed, and the run was reported `Exited`, which a recognition records as
/// its own failure. With a console of its own the provider is still at work
/// when the host cancels. On Unix the interrupt is `SIGINT` to the host's
/// process group, which a provider leading its own group never received; the
/// test holds that too. The provider is gone afterwards on both.
#[tokio::test]
async fn an_interrupt_to_the_host_never_ends_its_provider_first() -> Result<(), Box<dyn Error>> {
    use std::io::{BufRead, BufReader};

    let marker = unique_marker_path("interrupt-host-provider");
    let _ = std::fs::remove_file(&marker);
    let mut command = Command::new(env::current_exe()?);
    command
        .args([
            "--exact",
            "process_fixture",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(FIXTURE_MODE, "interrupt-host")
        .env(DESCENDANT_MARKER, &marker)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    separate_from_this_test(&mut command);
    let mut host = command.spawn()?;
    let stdout = host
        .stdout
        .take()
        .ok_or("the host has no standard output")?;
    let (lines, received) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let wait_for = |prefix: &str| -> Result<String, Box<dyn Error>> {
        let deadline = StandardInstant::now() + HOST_WAIT;
        loop {
            let left = deadline.saturating_duration_since(StandardInstant::now());
            let line = received
                .recv_timeout(left)
                .map_err(|_| format!("the host never printed {prefix}"))?;
            // The harness's own words can precede it on the line.
            if let Some((_, rest)) = line.split_once(prefix) {
                return Ok(format!("{prefix}{rest}"));
            }
        }
    };
    let termination = wait_for("HOST_READY")
        .and_then(|_| interrupt_host(&host))
        .and_then(|()| wait_for("TERMINATION="));
    if termination.is_err() {
        let _ = host.kill();
    }
    let _ = host.wait()?;
    let _ = reader.join();
    let provider = wait_for_descendant_marker(&marker, Duration::from_secs(1)).await;
    let _ = std::fs::remove_file(&marker);
    assert_eq!(termination?, "TERMINATION=Cancelled");
    assert!(wait_until_process_gone(provider?, REAP_DEADLINE)?);
    Ok(())
}

/// Starts the host apart from this test, so the interrupt sent to it reaches
/// nothing else: on Windows in a console of its own, on Unix in a process
/// group of its own.
#[cfg(windows)]
fn separate_from_this_test(command: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW)
}

/// See the Windows variant.
#[cfg(unix)]
fn separate_from_this_test(command: &mut Command) -> &mut Command {
    use std::os::unix::process::CommandExt;
    command.process_group(0)
}

/// Sends the host its interrupt (Windows only): a console Ctrl-Break to its
/// console, through the repository's helper, as the P10 and P11 checkpoints
/// do. The helper starts in a console of its own as well, then detaches from
/// it and attaches to the host's, so this test's console is never touched.
#[cfg(windows)]
fn interrupt_host(host: &std::process::Child) -> Result<(), Box<dyn Error>> {
    let system_root = env::var_os("SystemRoot").ok_or("SystemRoot is unavailable")?;
    let powershell = PathBuf::from(system_root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let helper = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tools")
        .join("send-console-ctrl.ps1");
    let mut command = Command::new(powershell);
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(helper)
        .args(["-ProcessId", &host.id().to_string(), "-Event", "CtrlBreak"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let status = separate_from_this_test(&mut command).status()?;
    if status.code() != Some(0) {
        return Err(format!("the console helper failed: {status}").into());
    }
    Ok(())
}

/// Sends the host its interrupt (Unix only): `SIGINT` to its whole process
/// group, as a terminal's Ctrl-C reaches the foreground group.
#[cfg(unix)]
fn interrupt_host(host: &std::process::Child) -> Result<(), Box<dyn Error>> {
    let status = Command::new("kill")
        .args(["-s", "INT", "--", &format!("-{}", host.id())])
        .stdin(Stdio::null())
        .status()?;
    if !status.success() {
        return Err(format!("kill -s INT failed: {status}").into());
    }
    Ok(())
}

/// #321: the fixture creates its marker and then writes it, so a reader can
/// find the file empty. The reader this replaced parsed whatever it read, and
/// an empty file failed the test with `ParseIntError { kind: Empty }` (once in
/// 1,500 hosted Windows repetitions). Here the file is empty on purpose when
/// the wait starts and is written 150 ms later.
#[tokio::test]
async fn a_marker_its_writer_has_created_but_not_yet_written_is_waited_for()
-> Result<(), Box<dyn Error>> {
    let marker = unique_marker_path("marker-created-then-written");
    std::fs::write(&marker, "")?;
    let late_marker = marker.clone();
    let writer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        std::fs::write(late_marker, "4242\n")
    });

    let read = wait_for_descendant_marker(&marker, Duration::from_secs(5)).await;
    let written = writer.await;
    let _ = std::fs::remove_file(&marker);
    written??;

    assert!(matches!(read, Ok(4242)), "{read:?}");
    Ok(())
}

/// A marker that never becomes a whole line ends the wait at its deadline, as
/// one that never appears does. It is never read as a process id: the first
/// bytes of `4242` are `42`, the id of some other process, and the test would
/// go on to watch that one.
#[tokio::test]
async fn a_marker_that_is_never_finished_ends_at_the_deadline() -> Result<(), Box<dyn Error>> {
    for (label, unfinished) in [("marker-left-empty", ""), ("marker-left-partial", "42")] {
        let marker = unique_marker_path(label);
        std::fs::write(&marker, unfinished)?;

        let read = wait_for_descendant_marker(&marker, Duration::from_millis(200)).await;
        let _ = std::fs::remove_file(&marker);

        assert!(
            matches!(read, Err(MarkerError::NotWrittenBeforeDeadline)),
            "{unfinished:?}: {read:?}"
        );
    }
    let never_created = unique_marker_path("marker-never-created");
    let read = wait_for_descendant_marker(&never_created, Duration::from_millis(200)).await;
    assert!(
        matches!(read, Err(MarkerError::NotWrittenBeforeDeadline)),
        "{read:?}"
    );
    Ok(())
}

/// A whole line that is not a process id is the fixture's mistake and fails at
/// once; waiting would only hide it behind the deadline's message.
#[tokio::test]
async fn a_finished_marker_that_is_not_a_process_id_fails_at_once() -> Result<(), Box<dyn Error>> {
    let marker = unique_marker_path("marker-not-a-process-id");
    std::fs::write(&marker, "not a process id\n")?;

    let started = StandardInstant::now();
    let read = wait_for_descendant_marker(&marker, Duration::from_secs(60)).await;
    let _ = std::fs::remove_file(&marker);

    assert!(
        matches!(read, Err(MarkerError::NotAProcessId(_))),
        "{read:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(30));
    Ok(())
}

#[test]
fn only_a_whole_line_is_taken_for_the_descendant_process_id() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        parse_descendant_marker("")?,
        DescendantMarker::StillBeingWritten
    );
    assert_eq!(
        parse_descendant_marker("42")?,
        DescendantMarker::StillBeingWritten
    );
    assert_eq!(
        parse_descendant_marker("4242\n")?,
        DescendantMarker::Complete(4242)
    );
    assert_eq!(
        parse_descendant_marker("4294967295\n")?,
        DescendantMarker::Complete(u32::MAX)
    );
    for not_an_id in ["\n", "42\n43\n", "-1\n", "4294967296\n", "42 \n"] {
        assert!(parse_descendant_marker(not_an_id).is_err(), "{not_an_id:?}");
    }
    Ok(())
}

#[tokio::test]
async fn p07_spawn_race_fails_without_an_unmanaged_process() -> Result<(), Box<dyn Error>> {
    let directory = unique_marker_path("spawn-race");
    std::fs::create_dir(&directory)?;
    let copied_executable = directory.join(if cfg!(windows) {
        "fixture.exe"
    } else {
        "fixture"
    });
    std::fs::copy(env::current_exe()?, &copied_executable)?;
    let executable = TrustedExecutable::explicit(&copied_executable)?;
    let working_directory = ProcessWorkingDirectory::new(&directory)?;
    std::fs::remove_file(&copied_executable)?;
    let request = ProcessRequest::new(executable, working_directory, Duration::from_secs(1))?;

    let result = supervisor(1024)?
        .run(request, ProcessCancellation::new())
        .await;
    std::fs::remove_dir(&directory)?;

    assert!(matches!(result, Err(ProcessError::Spawn(_))));
    Ok(())
}

#[tokio::test]
async fn p08_process_only_host_cannot_claim_strict_worker_isolation() -> Result<(), Box<dyn Error>>
{
    let request = fixture_request("inspect", Duration::from_secs(1))?
        .requiring_isolation(vsift_infrastructure::IsolationRequirement::StrictWorker);
    let result = supervisor(1024)?
        .run(request, ProcessCancellation::new())
        .await;

    assert!(matches!(
        result,
        Err(vsift_infrastructure::ProcessError::IsolationUnavailable)
    ));
    Ok(())
}

/// SEC-T01 (non-adversarial evidence, accepted for P11 on 2026-09-28): inside
/// the hardened CI container, `VSift`'s own strict-Linux attestation must
/// succeed, so `--host-isolation strict-linux` is proven to run on a real
/// strict host rather than only on parsed fixtures. Outside that container
/// the test does nothing.
#[cfg(target_os = "linux")]
#[test]
fn p11_strict_linux_attestation_holds_inside_the_hardened_container() -> Result<(), Box<dyn Error>>
{
    if env::var_os("VSIFT_STRICT_WORKER_TEST").is_none() {
        return Ok(());
    }
    match vsift_infrastructure::attest_strict_linux_host() {
        Ok(_) => Ok(()),
        Err(gaps) => {
            Err(format!("strict-linux attestation refused the hardened container: {gaps:?}").into())
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn p08_qualified_linux_host_reports_inherited_kernel_controls() -> Result<(), Box<dyn Error>>
{
    if env::var_os("VSIFT_STRICT_WORKER_TEST").is_none() {
        return Ok(());
    }

    assert_cgroup_limit("/sys/fs/cgroup/memory.max")?;
    assert_cgroup_limit("/sys/fs/cgroup/pids.max")?;
    let cpu_limit = std::fs::read_to_string("/sys/fs/cgroup/cpu.max")?;
    if cpu_limit.trim().starts_with("max") {
        return Err("strict worker CPU quota is not bounded".into());
    }
    let address = std::net::SocketAddr::from(([169, 254, 169, 254], 80));
    if std::net::TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok() {
        return Err("strict worker unexpectedly reached a network address".into());
    }
    let write_probe = env::current_dir()?.join("vsift-strict-write-probe");
    if std::fs::write(&write_probe, b"unexpected").is_ok() {
        let _ = std::fs::remove_file(&write_probe);
        return Err("strict worker repository mount is writable".into());
    }

    let stream_limit = NonZeroUsize::new(1024).ok_or("strict stream limit must be non-zero")?;
    let supervisor = ProcessSupervisor::new(
        SupervisorPolicy::new(
            stream_limit,
            Duration::from_millis(100),
            Duration::from_secs(3),
        ),
        HostIsolation::StrictLinux,
    );
    let request = fixture_request("inspect", Duration::from_secs(3))?
        .requiring_isolation(vsift_infrastructure::IsolationRequirement::StrictWorker);
    let outcome = supervisor.run(request, ProcessCancellation::new()).await?;

    assert!(outcome.status.success());
    assert_eq!(
        outcome.controls.hard_isolation,
        HardIsolation::InheritedStrictLinux
    );
    assert_eq!(
        outcome.controls.process_containment,
        ProcessContainment::UnixProcessGroup
    );

    assert_process_group_escape_remains_in_worker_cgroup()?;

    let throttled_before = cgroup_cpu_throttle_count()?;
    let cpu_outcome = run_strict_fixture(&supervisor, "strict-cpu-pressure", 10).await?;
    assert!(cpu_outcome.status.success());
    let throttled_after = cgroup_cpu_throttle_count()?;
    if throttled_after <= throttled_before {
        return Err("CPU pressure was not throttled by the worker quota".into());
    }

    let pid_outcome = run_strict_fixture(&supervisor, "strict-pid-pressure", 10).await?;
    assert!(pid_outcome.status.success());

    let memory_outcome = run_strict_fixture(&supervisor, "strict-memory-pressure", 15).await?;
    if memory_outcome.status.success() {
        return Err("memory pressure was not contained by the worker limit".into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn run_strict_fixture(
    supervisor: &ProcessSupervisor,
    mode: &str,
    deadline_seconds: u64,
) -> Result<vsift_infrastructure::ProcessOutcome, Box<dyn Error>> {
    let request = fixture_request(mode, Duration::from_secs(deadline_seconds))?
        .requiring_isolation(vsift_infrastructure::IsolationRequirement::StrictWorker);
    Ok(supervisor.run(request, ProcessCancellation::new()).await?)
}

#[cfg(target_os = "linux")]
fn assert_process_group_escape_remains_in_worker_cgroup() -> Result<(), Box<dyn Error>> {
    use std::os::unix::process::CommandExt;

    let executable = env::current_exe()?;
    let mut escaped = Command::new(executable)
        .args([
            "--exact",
            "process_fixture",
            "--nocapture",
            "--test-threads=1",
        ])
        .env_clear()
        .env(FIXTURE_MODE, "sleep")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    let pid = escaped.id();

    let checks = (|| -> Result<(), Box<dyn Error>> {
        let process_group = linux_process_group(pid)?;
        if process_group != pid {
            return Err("fixture did not enter a distinct process group".into());
        }
        let worker_cgroup = std::fs::read_to_string("/proc/self/cgroup")?;
        let escaped_cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup"))?;
        if escaped_cgroup != worker_cgroup {
            return Err("process-group escape also escaped the worker cgroup".into());
        }
        Ok(())
    })();

    let kill_result = escaped.kill();
    let wait_result = escaped.wait();
    checks?;
    if let Err(error) = kill_result
        && error.kind() != io::ErrorKind::InvalidInput
    {
        return Err(error.into());
    }
    wait_result?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn linux_process_group(pid: u32) -> Result<u32, Box<dyn Error>> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let fields = stat
        .rsplit_once(')')
        .ok_or("process stat omitted its command terminator")?
        .1;
    let process_group = fields
        .split_whitespace()
        .nth(2)
        .ok_or("process stat omitted its process group")?;
    Ok(process_group.parse()?)
}

#[cfg(target_os = "linux")]
fn cgroup_cpu_throttle_count() -> Result<u64, Box<dyn Error>> {
    let statistics = std::fs::read_to_string("/sys/fs/cgroup/cpu.stat")?;
    for line in statistics.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() == Some("nr_throttled") {
            return Ok(fields
                .next()
                .ok_or("CPU statistics omitted the throttle count")?
                .parse()?);
        }
    }
    Err("CPU statistics omitted nr_throttled".into())
}

#[cfg(target_os = "linux")]
fn assert_cgroup_limit(path: &str) -> Result<(), Box<dyn Error>> {
    let value = std::fs::read_to_string(path)?;
    if value.trim() == "max" {
        return Err(format!("strict worker limit is unbounded: {path}").into());
    }
    Ok(())
}

fn fixture_request(mode: &str, deadline: Duration) -> Result<ProcessRequest, Box<dyn Error>> {
    let executable = TrustedExecutable::explicit(env::current_exe()?)?;
    let working_directory = ProcessWorkingDirectory::new(env::current_dir()?)?;
    let request = ProcessRequest::new(executable, working_directory, deadline)?
        .with_arguments([
            "--exact",
            "process_fixture",
            "--nocapture",
            "--test-threads=1",
        ])
        .with_environment(FIXTURE_MODE, mode)?;
    Ok(request)
}

fn supervisor(stream_limit: usize) -> Result<ProcessSupervisor, Box<dyn Error>> {
    let stream_limit =
        NonZeroUsize::new(stream_limit).ok_or("test stream limit must be non-zero")?;
    Ok(ProcessSupervisor::new(
        SupervisorPolicy::new(stream_limit, Duration::from_millis(100), REAP_DEADLINE),
        HostIsolation::ProcessOnly,
    ))
}

/// What the content of a descendant marker says.
#[derive(Debug, Eq, PartialEq)]
enum DescendantMarker {
    /// The fixture has created the file but has not finished writing it.
    StillBeingWritten,
    /// The whole marker: the process id of the fixture's descendant.
    Complete(u32),
}

/// Why no descendant process id was read from a marker.
#[derive(Debug)]
enum MarkerError {
    /// The marker did not exist, or held no whole line, when the wait ended.
    NotWrittenBeforeDeadline,
    /// The marker held a whole line that is not a process id.
    NotAProcessId(ParseIntError),
    /// The marker could not be read, for a reason other than not existing yet.
    Read(io::Error),
}

impl fmt::Display for MarkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotWrittenBeforeDeadline => formatter
                .write_str("fixture did not write its descendant marker before the test deadline"),
            Self::NotAProcessId(error) => {
                write!(formatter, "descendant marker is not a process id: {error}")
            }
            Self::Read(error) => write!(formatter, "descendant marker could not be read: {error}"),
        }
    }
}

impl Error for MarkerError {}

/// Reads what a marker holds: the fixture writes the id and a line feed, and
/// only the line feed says that the id is whole.
///
/// The fixture is another process. It creates the file and then writes it, so
/// a read can land between the two and find the file empty (#321), and no read
/// of a file that is still being written is promised all of a write. Anything
/// without the final line feed is therefore not yet the marker, whatever
/// digits it holds. Renaming a finished file into place would hide the gap
/// too, but that is one more operation on a new file for a scanner to get in
/// the way of on Windows; the terminator needs nothing from the filesystem.
fn parse_descendant_marker(content: &str) -> Result<DescendantMarker, ParseIntError> {
    match content.strip_suffix('\n') {
        None => Ok(DescendantMarker::StillBeingWritten),
        Some(line) => line.parse().map(DescendantMarker::Complete),
    }
}

/// Waits, up to `deadline`, for the fixture's marker to hold its descendant's
/// process id, looking every 25 ms.
async fn wait_for_descendant_marker(marker: &Path, deadline: Duration) -> Result<u32, MarkerError> {
    let started = StandardInstant::now();
    while started.elapsed() < deadline {
        match std::fs::read_to_string(marker) {
            Ok(content) => {
                let read = parse_descendant_marker(&content).map_err(MarkerError::NotAProcessId)?;
                if let DescendantMarker::Complete(pid) = read {
                    return Ok(pid);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(MarkerError::Read(error)),
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Err(MarkerError::NotWrittenBeforeDeadline)
}

fn wait_until_process_gone(pid: u32, deadline: Duration) -> Result<bool, Box<dyn Error>> {
    let started = StandardInstant::now();
    while started.elapsed() < deadline {
        if !process_exists(pid)? {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Ok(!process_exists(pid)?)
}

#[cfg(windows)]
fn process_exists(pid: u32) -> Result<bool, Box<dyn Error>> {
    let system_root = env::var_os("SystemRoot").ok_or("SystemRoot is unavailable")?;
    let tasklist = PathBuf::from(system_root)
        .join("System32")
        .join("tasklist.exe");
    let output = Command::new(tasklist)
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .stdin(Stdio::null())
        .output()?;
    let pid_field = format!("\"{pid}\"");
    Ok(String::from_utf8_lossy(&output.stdout).contains(&pid_field))
}

#[cfg(unix)]
fn process_exists(pid: u32) -> Result<bool, Box<dyn Error>> {
    let executable = if Path::new("/bin/ps").is_file() {
        Path::new("/bin/ps")
    } else {
        Path::new("/usr/bin/ps")
    };
    let output = Command::new(executable)
        .args(["-p", &pid.to_string(), "-o", "pid="])
        .stdin(Stdio::null())
        .output()?;
    Ok(output.status.success() && !output.stdout.is_empty())
}

fn unique_marker_path(label: &str) -> PathBuf {
    let mut name = OsString::from("vsift-");
    name.push(label);
    name.push("-");
    name.push(std::process::id().to_string());
    env::temp_dir().join(name)
}
