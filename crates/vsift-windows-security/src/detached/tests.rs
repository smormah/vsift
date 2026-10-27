//! `start_detached` through real processes (L-229).
//!
//! The shape of the defect needs three processes: this test, a caller whose
//! standard output is a pipe the test reads (so the handle is inheritable in
//! the caller, as when an agent's tool call runs `vsift job start`), and a
//! long-lived child the caller starts before it exits. The caller and the
//! child are this test executable again, running one of the two role tests
//! below, which do nothing unless [`ROLE`] names the folder they share with
//! the test. The child's life is bounded ([`CHILD_LIFETIME`]) and it ends as
//! soon as the test creates the `stop` file, so no test leaves it behind.

use std::{
    env,
    error::Error,
    ffi::OsString,
    fmt::Write as _,
    fs::{self, File},
    io::{self, BufRead as _, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use super::{CallersJob, start_detached};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// Set for the caller and inherited by the child: the folder they share with
/// the test. Absent, the role tests do nothing.
const ROLE: &str = "VSIFT_WINDOWS_SECURITY_TEST_FOLDER";
/// Set for the caller: how it starts its children, comma-separated
/// ([`Launch`]).
const LAUNCH: &str = "VSIFT_WINDOWS_SECURITY_TEST_LAUNCH";
/// Each line the caller reports starts with this.
const REPORT: &str = "vsift-report ";
/// The child ends by itself after this, if the test never stops it.
const CHILD_LIFETIME: Duration = Duration::from_secs(60);
/// A safety stop for each wait below, never a timing claim: each wait ends as
/// soon as its condition holds.
const WAIT: Duration = Duration::from_secs(30);
/// Arguments the child is given beyond the test filter: extra filters, which
/// match no test, so they reach the child unchanged and it can report them.
/// They exercise the quoting.
const AWKWARD_ARGUMENTS: [&str; 5] = ["", "a b", r#"quote " inside"#, r"trail\", r#"x\"y"#];

/// How the caller starts a child.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Launch {
    /// With [`start_detached`].
    Function,
    /// With `std::process::Command`, its streams set to null, as
    /// `launch_detached` did before the fix.
    Standard,
    /// With [`start_detached`], then ended through `kill`.
    Kill,
}

impl Launch {
    const fn identifier(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Standard => "standard",
            Self::Kill => "kill",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [Self::Function, Self::Standard, Self::Kill]
            .into_iter()
            .find(|launch| launch.identifier() == text)
    }
}

/// A folder under the temporary folder, removed when it drops. It is not
/// canonicalised: the child's working folder is compared as given.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> TestResult<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!(
            "vsift-windows-security-detached-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// Ends every child that is still waiting.
    fn stop(&self) -> TestResult {
        fs::write(self.file("stop"), b"")?;
        Ok(())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::write(self.file("stop"), b"");
        // A test that failed early may leave children still running: wait for
        // each started one to see `stop` and end before the folder (and `stop`
        // with it) is removed.
        if let Ok(entries) = fs::read_dir(&self.0) {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if path
                    .extension()
                    .is_some_and(|extension| extension == "started")
                {
                    let _ = wait_for_end(&path.with_extension("lock"));
                }
            }
        }
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn child_arguments() -> Vec<OsString> {
    let mut arguments: Vec<OsString> = ["--exact", "detached::tests::role_child", "--nocapture"]
        .into_iter()
        .map(OsString::from)
        .collect();
    arguments.extend(AWKWARD_ARGUMENTS.into_iter().map(OsString::from));
    arguments
}

/// The child: records what it was given (its working folder, environment and
/// arguments), holds a lock while it lives, and waits for `stop`.
#[test]
fn role_child() -> TestResult {
    let Some(folder) = env::var_os(ROLE).map(PathBuf::from) else {
        return Ok(());
    };
    let id = std::process::id();
    let mut seen = String::new();
    writeln!(&mut seen, "folder {:?}", env::current_dir()?)?;
    let mut variables: Vec<(OsString, OsString)> = env::vars_os().collect();
    variables.sort();
    for (name, value) in variables {
        writeln!(&mut seen, "variable {name:?}={value:?}")?;
    }
    for argument in env::args_os().skip(1) {
        writeln!(&mut seen, "argument {argument:?}")?;
    }
    fs::write(folder.join(format!("{id}.seen")), seen)?;
    let lock = File::create(folder.join(format!("{id}.lock")))?;
    lock.lock()?;
    fs::write(folder.join(format!("{id}.started")), b"")?;
    let until = Instant::now() + CHILD_LIFETIME;
    while !folder.join("stop").exists() && Instant::now() < until {
        thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// The caller: starts one child for each launch [`LAUNCH`] names, reports
/// each on its standard output, and exits.
#[test]
fn role_caller() -> TestResult {
    let Some(folder) = env::var_os(ROLE).map(PathBuf::from) else {
        return Ok(());
    };
    let executable = env::current_exe()?;
    let arguments = child_arguments();
    for name in env::var(LAUNCH)?.split(',') {
        let launch = Launch::parse(name).ok_or("an unknown launch")?;
        match launch {
            Launch::Function => {
                let child = start_detached(&executable, &arguments, &folder, CallersJob::Stay)?;
                println!("{REPORT}{} {}", launch.identifier(), child.id());
            }
            Launch::Standard => {
                let child = Command::new(&executable)
                    .args(&arguments)
                    .current_dir(&folder)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()?;
                println!("{REPORT}{} {}", launch.identifier(), child.id());
            }
            Launch::Kill => {
                let mut child = start_detached(&executable, &arguments, &folder, CallersJob::Stay)?;
                let running = child.try_wait()?.is_none();
                child.kill()?;
                let status = child.wait()?;
                let after = child.try_wait()?;
                println!(
                    "{REPORT}{} {} running={running} code={:?} after={:?}",
                    launch.identifier(),
                    child.id(),
                    status.code(),
                    after.and_then(|status| status.code()),
                );
            }
        }
    }
    Ok(())
}

/// One line of the caller's output, or its end.
enum Line {
    Text(String),
    End,
}

/// A caller started with its standard output a pipe the test reads.
struct Caller {
    lines: Receiver<io::Result<Line>>,
}

impl Caller {
    fn start(scratch: &Scratch, launches: &[Launch]) -> TestResult<Self> {
        let launch: Vec<&str> = launches.iter().map(|launch| launch.identifier()).collect();
        let mut caller = Command::new(env::current_exe()?)
            .args(["--exact", "detached::tests::role_caller", "--nocapture"])
            .env(ROLE, &scratch.0)
            .env(LAUNCH, launch.join(","))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let output = caller
            .stdout
            .take()
            .ok_or("the caller has no output pipe")?;
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let failed = line.is_err();
                if sender.send(line.map(Line::Text)).is_err() || failed {
                    return;
                }
            }
            let _ = sender.send(Ok(Line::End));
        });
        // The caller exits as soon as it has started its children.
        let status = caller.wait()?;
        if !status.success() {
            return Err(format!("the caller failed: {status}").into());
        }
        Ok(Self { lines })
    }

    /// The caller's reports, read until end-of-file or `within` has passed.
    /// The flag says whether end-of-file came.
    fn reports(&self, within: Duration) -> TestResult<(Vec<String>, bool)> {
        let deadline = Instant::now() + within;
        let mut reports = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => match line? {
                    Line::Text(text) => {
                        if let Some(report) = text.strip_prefix(REPORT) {
                            reports.push(report.to_owned());
                        }
                    }
                    Line::End => return Ok((reports, true)),
                },
                Err(RecvTimeoutError::Timeout) => return Ok((reports, false)),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("the caller's output reader stopped".into());
                }
            }
        }
    }
}

/// The process identifier a report names after its launch.
fn reported_id(report: &str, launch: Launch) -> TestResult<u32> {
    let mut words = report.split(' ');
    if words.next() != Some(launch.identifier()) {
        return Err(format!("not a {} report: {report}", launch.identifier()).into());
    }
    Ok(words.next().ok_or("a report without an id")?.parse()?)
}

/// Waits until `path` exists.
fn wait_for_file(path: &Path) -> TestResult {
    let deadline = Instant::now() + WAIT;
    while !path.exists() {
        if Instant::now() > deadline {
            return Err("a child never started".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

/// Whether the child holding the lock `path` is still alive.
fn holds_lock(path: &Path) -> TestResult<bool> {
    match File::open(path)?.try_lock() {
        Ok(()) => Ok(false),
        Err(fs::TryLockError::WouldBlock) => Ok(true),
        Err(fs::TryLockError::Error(error)) => Err(error.into()),
    }
}

/// Waits until the child holding the lock `path` has ended.
fn wait_for_end(path: &Path) -> TestResult {
    let deadline = Instant::now() + WAIT;
    while holds_lock(path)? {
        if Instant::now() > deadline {
            return Err("a child never ended".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

/// L-229: a child started with `start_detached` does not hold its caller's
/// output pipe, so the caller's reader sees end-of-file once the caller has
/// exited, while the child is still alive.
#[test]
fn a_child_started_with_the_function_does_not_hold_the_callers_pipe() -> TestResult {
    let scratch = Scratch::new()?;
    let caller = Caller::start(&scratch, &[Launch::Function])?;
    let (reports, ended) = caller.reports(WAIT)?;
    assert!(ended, "the caller's output stayed open: {reports:?}");
    let [report] = reports.as_slice() else {
        return Err(format!("expected one report: {reports:?}").into());
    };
    let id = reported_id(report, Launch::Function)?;
    wait_for_file(&scratch.file(&format!("{id}.started")))?;
    let lock = scratch.file(&format!("{id}.lock"));
    assert!(holds_lock(&lock)?, "the child ended before the check");
    scratch.stop()?;
    wait_for_end(&lock)
}

/// The control: the same child started by the standard library holds the
/// caller's pipe, so the reader sees no end-of-file while the child lives.
/// This is the defect, and it shows the test above can tell.
#[test]
fn a_child_started_by_the_standard_library_holds_the_callers_pipe() -> TestResult {
    let scratch = Scratch::new()?;
    let caller = Caller::start(&scratch, &[Launch::Standard])?;
    let (reports, ended) = caller.reports(Duration::from_secs(3))?;
    assert!(!ended, "the standard library no longer passes the pipe on");
    let [report] = reports.as_slice() else {
        return Err(format!("expected one report: {reports:?}").into());
    };
    let id = reported_id(report, Launch::Standard)?;
    let lock = scratch.file(&format!("{id}.lock"));
    wait_for_file(&scratch.file(&format!("{id}.started")))?;
    assert!(holds_lock(&lock)?, "the child ended before the check");
    scratch.stop()?;
    wait_for_end(&lock)?;
    // Only the child's end released the pipe.
    let (_, ended) = caller.reports(WAIT)?;
    assert!(ended, "the caller's output never closed");
    Ok(())
}

/// The planner's condition (2026-10-27): the child sees the working folder,
/// environment and arguments that the standard library gives it, which is
/// what `launch_detached` gave the runner before the fix.
#[test]
fn the_child_sees_what_the_standard_library_would_give_it() -> TestResult {
    let scratch = Scratch::new()?;
    let caller = Caller::start(&scratch, &[Launch::Standard, Launch::Function])?;
    // The standard library's child keeps the pipe open, so the reports are
    // read as they come rather than to end-of-file.
    let (reports, _) = caller.reports(Duration::from_secs(5))?;
    let [standard, function] = reports.as_slice() else {
        return Err(format!("expected two reports: {reports:?}").into());
    };
    let standard = reported_id(standard, Launch::Standard)?;
    let function = reported_id(function, Launch::Function)?;
    for id in [standard, function] {
        wait_for_file(&scratch.file(&format!("{id}.started")))?;
    }
    let seen_standard = fs::read_to_string(scratch.file(&format!("{standard}.seen")))?;
    let seen_function = fs::read_to_string(scratch.file(&format!("{function}.seen")))?;
    assert_eq!(seen_function, seen_standard);
    assert!(
        seen_function.starts_with(&format!("folder {:?}\n", scratch.0)),
        "the child's working folder is not the one given"
    );
    let arguments: Vec<String> = seen_function
        .lines()
        .filter_map(|line| line.strip_prefix("argument "))
        .map(str::to_owned)
        .collect();
    let expected: Vec<String> = child_arguments()
        .iter()
        .map(|argument| format!("{argument:?}"))
        .collect();
    assert_eq!(arguments, expected);
    scratch.stop()?;
    for id in [standard, function] {
        wait_for_end(&scratch.file(&format!("{id}.lock")))?;
    }
    Ok(())
}

/// `try_wait`, `kill` and `wait` answer as `std::process::Child` does: not
/// ended, then ended with exit code 1.
#[test]
fn a_detached_process_can_be_ended_and_waited_for() -> TestResult {
    let scratch = Scratch::new()?;
    let caller = Caller::start(&scratch, &[Launch::Kill])?;
    let (reports, ended) = caller.reports(WAIT)?;
    assert!(ended, "the caller's output stayed open: {reports:?}");
    let [report] = reports.as_slice() else {
        return Err(format!("expected one report: {reports:?}").into());
    };
    let id = reported_id(report, Launch::Kill)?;
    assert_eq!(
        report,
        &format!("kill {id} running=true code=Some(1) after=Some(1)")
    );
    Ok(())
}

#[test]
fn a_relative_path_a_batch_file_and_a_nul_are_refused() -> TestResult {
    let scratch = Scratch::new()?;
    let executable = env::current_exe()?;
    let refusals = [
        start_detached(Path::new("relative.exe"), &[], &scratch.0, CallersJob::Stay),
        start_detached(&scratch.file("run.bat"), &[], &scratch.0, CallersJob::Stay),
        start_detached(&scratch.file("run.CMD."), &[], &scratch.0, CallersJob::Stay),
        start_detached(
            &executable,
            &[OsString::from("a\0b")],
            &scratch.0,
            CallersJob::Stay,
        ),
        start_detached(&executable, &[], &scratch.file("a\0b"), CallersJob::Stay),
    ];
    for refused in refusals {
        assert!(
            matches!(&refused, Err(error) if error.kind() == io::ErrorKind::InvalidInput),
            "{refused:?}"
        );
    }
    Ok(())
}
