//! Cross-platform adversarial qualification tests for the secure process supervisor.

use std::{
    env,
    error::Error,
    ffi::OsString,
    io::{self, Write},
    num::NonZeroUsize,
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
    for _ in 0..64 {
        match stream {
            OutputStream::Stdout => io::stdout().write_all(&bytes)?,
            OutputStream::Stderr => io::stderr().write_all(&bytes)?,
        }
    }
    Ok(())
}

fn fixture_both_floods() -> Result<(), Box<dyn Error>> {
    let bytes = vec![b'x'; 8 * 1024];
    for _ in 0..64 {
        io::stdout().write_all(&bytes)?;
        io::stderr().write_all(&bytes)?;
    }
    Ok(())
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
        std::fs::write(marker, child.id().to_string())?;
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
    let request = fixture_request("inspect", Duration::from_secs(3))?
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
    let request = ProcessRequest::new(executable, working_directory, Duration::from_secs(3))?
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
        let request = fixture_request(mode, Duration::from_secs(3))?;
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
        let request = fixture_request(mode, Duration::from_secs(3))?;
        let outcome = supervisor(64 * 1024)?
            .run(request, ProcessCancellation::new())
            .await?;

        assert_eq!(outcome.termination, TerminationReason::Exited);
        assert!(outcome.stdout.complete);
        assert!(!outcome.stdout.bytes.is_empty());
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
    let cancelled_request = fixture_request("sleep", Duration::from_secs(5))?;
    let cancelled_outcome = supervisor(64 * 1024)?
        .run(cancelled_request, cancellation)
        .await?;
    cancellation_task.await?;
    assert_eq!(cancelled_outcome.termination, TerminationReason::Cancelled);
    Ok(())
}

#[tokio::test]
async fn p06_descendants_and_inherited_pipe_holders_are_terminated() -> Result<(), Box<dyn Error>> {
    for mode in ["descendant", "pipe-holder"] {
        let marker = unique_marker_path(mode);
        let request = fixture_request(mode, Duration::from_secs(10))?
            .with_environment(DESCENDANT_MARKER, marker.as_os_str())?;
        let cancellation = ProcessCancellation::new();
        let cancellation_trigger = cancellation.clone();
        let supervisor = supervisor(64 * 1024)?;
        let run = tokio::spawn(async move { supervisor.run(request, cancellation).await });
        let descendant_result = wait_for_descendant_marker(&marker, Duration::from_secs(5)).await;
        cancellation_trigger.cancel();
        let outcome = run.await??;
        let _ = std::fs::remove_file(&marker);
        let descendant_pid = descendant_result?;

        assert_eq!(outcome.termination, TerminationReason::Cancelled);
        assert!(wait_until_process_gone(
            descendant_pid,
            Duration::from_secs(3)
        )?);
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
        SupervisorPolicy::new(
            stream_limit,
            Duration::from_millis(100),
            Duration::from_secs(3),
        ),
        HostIsolation::ProcessOnly,
    ))
}

async fn wait_for_descendant_marker(
    marker: &Path,
    deadline: Duration,
) -> Result<u32, Box<dyn Error>> {
    let started = StandardInstant::now();
    while started.elapsed() < deadline {
        match std::fs::read_to_string(marker) {
            Ok(value) => return Ok(value.parse()?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err("fixture did not create its descendant marker before the test deadline".into())
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
