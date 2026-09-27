//! X-06 at the process boundary (P10 PR 3, ADR 0020 section 5): an
//! interrupted long command ends with exactly one terminal result, commits
//! nothing partial and leaves nothing running.
//!
//! The long command is `ingest` of a large zero-filled (sparse) file, which
//! runs everywhere without any provider: the copy is interrupted once it has
//! started. On Unix `SIGINT` and `SIGTERM` are sent with `kill` and run in
//! CI. On Windows a console Ctrl-C or Ctrl-Break is sent through the opt-in
//! PowerShell helper `tools/send-console-ctrl.ps1` (platform invoke through
//! .NET, no `unsafe` in `VSift`), because Windows delivers console events to
//! every process on a console:
//!
//! `cargo test -p vsift-cli --locked --test interrupt_cli_contract -- --ignored`
//!
//! It sends Ctrl-Break, and also Ctrl-C when `VSIFT_TEST_CONSOLE_CTRL_C` is
//! set (see the test for why).
//!
//! Interruptions of real provider runs (whisper.cpp and `FFmpeg`) are in the
//! opt-in `p10_recovery_e2e`.

use std::{
    env,
    error::Error,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-interrupt-cli-";
/// Large enough that the copy is still running when it is interrupted,
/// within the 20 GiB source bound; sparse, so it costs no disk to create.
const SOURCE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
/// How long an interrupted command may take to end (SEC-04: the 5 s
/// graceful and 5 s forced provider budgets).
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(10);

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Built<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self, child: &str) -> PathBuf {
        self.0.join(child)
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// A zero-filled file of [`SOURCE_BYTES`], extended without writing.
fn sparse_source(root: &OwnedRoot) -> Built<PathBuf> {
    let path = root.path("large.mp4");
    fs::File::create(&path)?.set_len(SOURCE_BYTES)?;
    Ok(path)
}

/// The private copy `ingest` is writing, if it has begun.
fn staged_copy(directory: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(directory).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = staged_copy(&path) {
                return Some(found);
            }
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("source-"))
            && path
                .extension()
                .is_some_and(|extension| extension == "media")
        {
            return Some(path);
        }
    }
    None
}

/// Starts `vsift ingest` of the large source with an isolated per-user base.
fn start_ingest(root: &OwnedRoot, source: &Path, command: &mut Command) -> Built<Child> {
    let base = root.path("user");
    Ok(command
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .arg("--session-root")
        .arg(root.path("sessions"))
        .arg("ingest")
        .arg(source)
        .arg("--json")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?)
}

/// Waits until the copy has begun writing, so the interruption lands
/// mid-copy rather than before the handler exists or after the copy ended.
fn wait_for_copy(root: &OwnedRoot, child: &mut Child) -> TestResult {
    let started = Instant::now();
    loop {
        if let Some(copy) = staged_copy(&root.path("sessions"))
            && fs::metadata(&copy).is_ok_and(|metadata| metadata.len() > 0)
        {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            return Err(format!("ingest ended before its copy began: {status}").into());
        }
        if started.elapsed() > Duration::from_secs(60) {
            return Err("the copy never began".into());
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// Waits for the interrupted command within the shutdown budget and checks
/// its one terminal result: `CANCELLED`, exit 6, no session opened and the
/// partial private copy removed.
fn assert_cancelled_within_budget(root: &OwnedRoot, mut child: Child, sent: Instant) -> TestResult {
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if sent.elapsed() > SHUTDOWN_BUDGET {
            let _ = child.kill();
            return Err("the interrupted command outlived its shutdown budget".into());
        }
        thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .ok_or("no stdout")?
        .read_to_string(&mut stdout)?;
    assert_eq!(status.code(), Some(6), "{stdout}");
    assert_eq!(stdout.lines().count(), 1, "one terminal result: {stdout}");
    let value: Value = serde_json::from_str(stdout.trim_end())?;
    assert_eq!(value["command"], "ingest");
    assert_eq!(value["status"], "cancelled");
    assert_eq!(value["error"]["code"], "CANCELLED");
    assert_eq!(value["data"], Value::Null);
    assert_eq!(
        staged_copy(&root.path("sessions")),
        None,
        "partial copy kept"
    );
    Ok(())
}

#[cfg(unix)]
fn interrupt_ingest_with(signal: &str) -> TestResult {
    let root = OwnedRoot::new()?;
    let source = sparse_source(&root)?;
    let mut child = start_ingest(
        &root,
        &source,
        &mut Command::new(assert_cmd::cargo::cargo_bin("vsift")),
    )?;
    wait_for_copy(&root, &mut child)?;
    let sent = Instant::now();
    let killed = Command::new("kill")
        .args(["-s", signal, &child.id().to_string()])
        .status()?;
    assert!(killed.success());
    assert_cancelled_within_budget(&root, child, sent)
}

/// `SIGINT` (Ctrl-C at a terminal) cancels a copy in progress.
#[cfg(unix)]
#[test]
fn sigint_cancels_an_ingest_copy() -> TestResult {
    interrupt_ingest_with("INT")
}

/// `SIGTERM` (a supervisor stopping the process) cancels it the same way.
#[cfg(unix)]
#[test]
fn sigterm_cancels_an_ingest_copy() -> TestResult {
    interrupt_ingest_with("TERM")
}

/// Sends a console event to `process` through the PowerShell helper.
#[cfg(windows)]
fn send_console_event(process: u32, event: &str) -> TestResult {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let helper =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/send-console-ctrl.ps1");
    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(helper)
        .args(["-ProcessId", &process.to_string(), "-Event", event])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.code() != Some(0) {
        return Err(format!("the console helper failed: {status}").into());
    }
    Ok(())
}

/// Windows: a console Ctrl-Break (and, when `VSIFT_TEST_CONSOLE_CTRL_C` is
/// set, a Ctrl-C) cancels a copy in progress. `vsift` runs in a console of
/// its own so the event reaches nothing else. Windows never tells a process
/// that inherited the "ignore Ctrl-C" attribute about a Ctrl-C (a test
/// runner started by a service or by some IDE and agent hosts has it), so
/// Ctrl-C is checked only where the caller says the attribute is clear.
#[cfg(windows)]
#[test]
#[ignore = "opt-in: sends console control events through tools/send-console-ctrl.ps1"]
fn console_interrupts_cancel_an_ingest_copy() -> TestResult {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut events = vec!["CtrlBreak"];
    if env::var_os("VSIFT_TEST_CONSOLE_CTRL_C").is_some() {
        events.push("CtrlC");
    }
    for event in events {
        let root = OwnedRoot::new()?;
        let source = sparse_source(&root)?;
        let mut command = Command::new(assert_cmd::cargo::cargo_bin("vsift"));
        command.creation_flags(CREATE_NO_WINDOW);
        let mut child = start_ingest(&root, &source, &mut command)?;
        wait_for_copy(&root, &mut child)?;
        let sent = Instant::now();
        send_console_event(child.id(), event)?;
        assert_cancelled_within_budget(&root, child, sent)?;
    }
    Ok(())
}
