//! `launch_detached` through real processes (ADR 0033 section 5, L-220, L-229).
//!
//! The shape of L-229 needs three processes: this test, a caller whose
//! standard output is a pipe the test reads (as when an AI assistant's tool
//! call runs `vsift job start` and captures its output), and the runner the
//! caller starts with `launch_detached` before it exits. The caller and the
//! runner are this test executable again, running one of the two role tests
//! below, which do nothing unless [`ROLE`] names the folder they share with
//! the test. The runner's life is bounded ([`RUNNER_LIFETIME`]) and it ends as
//! soon as the test creates the `stop` file, so no test leaves it behind.
//!
//! The CLI-level test of the same defect (`job start` answering within
//! seconds while a queued job's runner waits behind a live one) is
//! `a_captured_queued_job_start_answers_while_its_runner_waits` in
//! `vsift-cli`'s `job_cli_contract`.

use std::{
    env,
    error::Error,
    ffi::OsString,
    fs::{self, File},
    io::{self, BufRead as _, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use vsift_infrastructure::launch_detached;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// Set for the caller and inherited by the runner: the folder they share
/// with the test. Absent, the role tests do nothing.
const ROLE: &str = "VSIFT_DETACHED_RUNNER_TEST_FOLDER";
/// The line the caller reports its runner's detachment on starts with this.
const REPORT: &str = "vsift-detachment ";
/// The runner ends by itself after this, if the test never stops or kills it.
const RUNNER_LIFETIME: Duration = Duration::from_secs(60);
/// A safety stop for each wait below, never a timing claim: each wait ends as
/// soon as its condition holds. It is generous because on Windows a process
/// started inside a job object by `process-wrap` can take seconds to start
/// (see `process_supervisor.rs`).
const WAIT: Duration = Duration::from_secs(30);

/// A folder under the temporary folder, removed when it drops.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> TestResult<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!(
            "vsift-detached-runner-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::write(self.file("stop"), b"");
        // A test that failed early may leave a runner still starting: give it
        // a moment to start and see `stop`, and wait for its end, before the
        // folder (and `stop` with it) is removed.
        let started = Instant::now() + Duration::from_secs(5);
        while !self.file("runner.started").exists() && Instant::now() < started {
            thread::sleep(Duration::from_millis(20));
        }
        let _ = wait_for_end(&self.file("runner.lock"));
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The runner: holds a lock while it lives, and waits for `stop`.
#[test]
fn role_runner() -> TestResult {
    let Some(folder) = env::var_os(ROLE).map(PathBuf::from) else {
        return Ok(());
    };
    let lock = File::create(folder.join("runner.lock"))?;
    lock.lock()?;
    fs::write(folder.join("runner.started"), b"")?;
    let until = Instant::now() + RUNNER_LIFETIME;
    while !folder.join("stop").exists() && Instant::now() < until {
        thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// The caller: starts the runner with `launch_detached`, as `job start`
/// does, reports its detachment on its standard output, and exits.
#[test]
fn role_caller() -> TestResult {
    let Some(folder) = env::var_os(ROLE).map(PathBuf::from) else {
        return Ok(());
    };
    let arguments: Vec<OsString> = ["--exact", "role_runner", "--nocapture"]
        .into_iter()
        .map(OsString::from)
        .collect();
    let runner = launch_detached(&env::current_exe()?, &arguments, &folder)?;
    println!("{REPORT}{}", runner.detachment().identifier());
    Ok(())
}

/// The caller's command: this executable, running [`role_caller`], with its
/// standard output a pipe.
fn caller_command(scratch: &Scratch) -> TestResult<Command> {
    let mut command = Command::new(env::current_exe()?);
    command
        .args(["--exact", "role_caller", "--nocapture"])
        .env(ROLE, &scratch.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    Ok(command)
}

/// Reads `output` line by line on a thread of its own; `None` marks
/// end-of-file.
fn read_lines(output: impl io::Read + Send + 'static) -> Receiver<io::Result<Option<String>>> {
    let (sender, lines) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            let failed = line.is_err();
            if sender.send(line.map(Some)).is_err() || failed {
                return;
            }
        }
        let _ = sender.send(Ok(None));
    });
    lines
}

/// The detachment the caller reported, once its output has ended, or `None`
/// when the output did not end within [`WAIT`].
fn detachment_at_end(lines: &Receiver<io::Result<Option<String>>>) -> TestResult<Option<String>> {
    let deadline = Instant::now() + WAIT;
    let mut detachment = None;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match lines.recv_timeout(left) {
            Ok(line) => match line? {
                Some(text) => {
                    if let Some(reported) = text.strip_prefix(REPORT) {
                        detachment = Some(reported.to_owned());
                    }
                }
                None => return Ok(Some(detachment.ok_or("the caller reported nothing")?)),
            },
            Err(RecvTimeoutError::Timeout) => return Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                return Err("the caller's output reader stopped".into());
            }
        }
    }
}

/// Waits until `path` exists.
fn wait_for_file(path: &Path) -> TestResult {
    let deadline = Instant::now() + WAIT;
    while !path.exists() {
        if Instant::now() > deadline {
            return Err("the runner never started".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

/// Whether the runner holding the lock `path` is still alive.
fn holds_lock(path: &Path) -> TestResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    match File::open(path)?.try_lock() {
        Ok(()) => Ok(false),
        Err(fs::TryLockError::WouldBlock) => Ok(true),
        Err(fs::TryLockError::Error(error)) => Err(error.into()),
    }
}

/// Waits until the runner holding the lock `path` has ended.
fn wait_for_end(path: &Path) -> TestResult {
    let deadline = Instant::now() + WAIT;
    while holds_lock(path)? {
        if Instant::now() > deadline {
            return Err("the runner never ended".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

/// L-229: the runner `launch_detached` starts does not hold its caller's
/// output pipe, so a parent that captures the caller's output sees
/// end-of-file once the caller has exited, while the runner is still alive.
/// Before the fix this waited, on Windows, until the runner ended.
#[test]
fn a_runner_does_not_hold_its_callers_output_open() -> TestResult {
    let scratch = Scratch::new("pipe")?;
    let mut caller = caller_command(&scratch)?.spawn()?;
    let lines = read_lines(
        caller
            .stdout
            .take()
            .ok_or("the caller has no output pipe")?,
    );
    let status = caller.wait()?;
    assert!(status.success(), "the caller failed: {status}");
    let detachment = detachment_at_end(&lines)?;
    let lock = scratch.file("runner.lock");
    let Some(detachment) = detachment else {
        fs::write(scratch.file("stop"), b"")?;
        return Err("the caller's output stayed open while its runner lived".into());
    };
    // The output may end before the runner is even running; it lives until
    // `stop`, so finding it alive now means the output ended while it lived.
    wait_for_file(&scratch.file("runner.started"))?;
    let alive = holds_lock(&lock)?;
    fs::write(scratch.file("stop"), b"")?;
    assert!(alive, "the runner was not alive after the output ended");
    let expected: &[&str] = if cfg!(windows) {
        // Whether the test's own job object lets the runner leave it.
        &["left_callers_job", "in_callers_job"]
    } else {
        &["process_group"]
    };
    assert!(expected.contains(&detachment.as_str()), "{detachment}");
    wait_for_end(&lock)
}

/// The planner's condition (2) for the fix, and L-220: when the caller's job
/// object forbids leaving it (as `process-wrap`'s does, like some IDE
/// terminals and service hosts), the runner starts inside it and says so,
/// rather than failing; it still does not hold the caller's output; and
/// whatever ends that job ends the runner.
#[cfg(windows)]
#[tokio::test]
async fn a_caller_whose_job_forbids_leaving_it_gets_a_runner_inside_it() -> TestResult {
    use process_wrap::tokio::{CommandWrap, JobObject, KillOnDrop};
    use tokio::io::AsyncReadExt as _;

    let scratch = Scratch::new("job")?;
    let command = tokio::process::Command::from(caller_command(&scratch)?);
    let mut wrapped = CommandWrap::from(command);
    wrapped.wrap(JobObject);
    wrapped.wrap(KillOnDrop);
    let mut caller = wrapped.spawn()?;
    let mut output = caller
        .stdout()
        .take()
        .ok_or("the caller has no output pipe")?;
    let mut text = String::new();
    tokio::time::timeout(WAIT, output.read_to_string(&mut text))
        .await
        .map_err(|_| "the caller's output stayed open while its runner lived")??;
    let detachment = text
        .lines()
        .find_map(|line| line.strip_prefix(REPORT))
        .ok_or("the caller reported nothing")?;
    assert_eq!(detachment, "in_callers_job");
    let lock = scratch.file("runner.lock");
    wait_for_file(&scratch.file("runner.started"))?;
    assert!(holds_lock(&lock)?, "the runner ended before its job did");
    // Closing the job ends every process in it, the runner too, though it
    // was never told to stop.
    drop(output);
    drop(caller);
    wait_for_end(&lock)
}
