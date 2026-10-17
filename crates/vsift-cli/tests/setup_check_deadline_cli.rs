//! Issue #391 through the binary: `setup check` with tools that are installed
//! and do not answer in time.
//!
//! The first `vsift setup check` after a restart reported an installed,
//! healthy `FFmpeg` and `FFprobe` as `[timeout]`, called the setup `blocked`
//! and advised installing them; `vsift setup check --timeout-seconds 1`
//! showed the same on warm tools. This is that reproduction made independent
//! of how fast the machine is: the three tools are stand-ins that exist, can
//! be started and never answer, so the first is stopped at the deadline and
//! the two after it are never started, on every machine.
//!
//! What must hold, in the readable answer and in the JSON one: each tool is
//! reported as a timeout with the published `timed_out`, the setup is still
//! `blocked` and the exit status is still 2; the answer says which of the two
//! things happened to each tool and under which deadline, and tells the
//! reader to run the check again or to allow more time; and nothing in it
//! says to install or locate a tool, or shows where the stand-in is.
//!
//! The target has no libtest harness (`harness = false` in `Cargo.toml`). The
//! stand-in is a copy of this executable, started by `vsift` with a probe's
//! own argument list (`-version`, `--help`), which libtest would reject, and
//! with a closed environment, so only its file name can tell it which part to
//! play: the reason `p14_installed_binary_e2e` has none either. It always
//! runs (a name filter on the command line does not skip it); `--list` lists
//! it, and `--ignored` alone leaves it out, as for any test that is not
//! ignored.

use std::{
    env,
    error::Error,
    ffi::{OsStr, OsString},
    fs, io,
    path::{Path, PathBuf},
    process::{ExitCode, Output},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const TEST_NAME: &str = "a_tool_that_does_not_answer_in_time_gets_no_advice_for_a_missing_tool";
const OWNED_PREFIX: &str = "vsift-setup-deadline-";
/// A copy of this executable under this name is the stand-in tool.
const SILENT_TOOL_STEM: &str = "vsift-test-silent-tool";
/// Far past any probe deadline (60 seconds at most): the stand-in is always
/// stopped by `vsift`, never by its own exit.
const SILENT_FOR: Duration = Duration::from_secs(300);
/// Upper bound for one `vsift` invocation; a hang is killed and fails.
const CLI_DEADLINE: Duration = Duration::from_secs(120);

/// Words that tell a reader to get a tool or to point `VSift` at one: the
/// advice for a missing tool before issue #396 and since.
const INSTALL_ADVICE: [&str; 10] = [
    "Install or locate",
    "install or locate",
    "Install a trusted",
    "No need to search folders",
    "setup install",
    "setup plan",
    "setup configure ",
    "reviewed build",
    "Managed installation",
    "absolute path",
];

const AFTER_WHAT_HAPPENED: &str = " A first run after an install or restart can be slow. Run setup check again, or allow more time with --timeout-seconds (up to 60).";

fn main() -> ExitCode {
    if let Some(code) = run_as_silent_tool() {
        return code;
    }
    let arguments: Vec<OsString> = env::args_os().skip(1).collect();
    let has = |flag: &str| arguments.iter().any(|argument| argument == flag);
    if has("--list") {
        println!("{TEST_NAME}: test\n\n1 test, 0 benchmarks");
        return ExitCode::SUCCESS;
    }
    if has("--ignored") && !has("--include-ignored") {
        println!("setup_check_deadline_cli: not an ignored test; nothing run");
        return ExitCode::SUCCESS;
    }
    match run() {
        Ok(()) => {
            println!("test {TEST_NAME} ... ok");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("test {TEST_NAME} ... FAILED\n{error}");
            ExitCode::FAILURE
        }
    }
}

/// The stand-in's side: started under the stand-in's name, this executable
/// writes nothing and does not finish until it is stopped.
fn run_as_silent_tool() -> Option<ExitCode> {
    let executable = env::current_exe().ok()?;
    if executable.file_stem()?.to_str()? != SILENT_TOOL_STEM {
        return None;
    }
    thread::sleep(SILENT_FOR);
    Some(ExitCode::from(70))
}

/// A temporary folder this test created and alone may remove.
struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> TestResult<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!("{OWNED_PREFIX}{}-{stamp}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    /// This executable, copied under the stand-in's name.
    fn silent_tool(&self) -> TestResult<PathBuf> {
        let source = env::current_exe()?;
        let mut name = OsString::from(SILENT_TOOL_STEM);
        if let Some(extension) = source.extension() {
            name.push(".");
            name.push(extension);
        }
        let tools = self.0.join("tools");
        fs::create_dir(&tools)?;
        let path = tools.join(name);
        fs::copy(&source, &path)?;
        Ok(path)
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if !self
            .0
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            return;
        }
        // A stand-in that was just stopped can hold its file for a moment.
        for _ in 0..20 {
            if fs::remove_dir_all(&self.0).is_ok() {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

fn ensure(condition: bool, expectation: impl FnOnce() -> String) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(expectation()).into())
    }
}

/// `vsift setup check --timeout-seconds 1` with the stand-in selected as all
/// three tools, isolated per-user state and nothing on `PATH`.
fn setup_check(root: &OwnedRoot, tool: &Path, mode: &[&str]) -> TestResult<Output> {
    let base = root.0.join("state");
    Ok(Command::cargo_bin("vsift")?
        .args(["setup", "check", "--timeout-seconds", "1"])
        .arg("--ffmpeg")
        .arg(tool)
        .arg("--ffprobe")
        .arg(tool)
        .arg("--whisper")
        .arg(tool)
        .args(mode)
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("XDG_DATA_HOME", base.join("data"))
        .env("XDG_CACHE_HOME", base.join("cache"))
        .env("HOME", &base)
        .env("PATH", "")
        .timeout(CLI_DEADLINE)
        .output()?)
}

fn schema() -> TestResult<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/v1/setup-check-response.schema.json");
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

fn run() -> TestResult {
    let root = OwnedRoot::new()?;
    let tool = root.silent_tool()?;
    let tool_text = tool.to_string_lossy().into_owned();

    // The readable answer.
    let human = setup_check(&root, &tool, &[])?;
    let text = String::from_utf8(human.stdout)?;
    ensure(human.status.code() == Some(2), || {
        format!("exit {:?}, expected 2: {text}", human.status.code())
    })?;
    ensure(human.stderr.is_empty(), || {
        format!("standard error: {}", String::from_utf8_lossy(&human.stderr))
    })?;
    for expected in [
        "Status: blocked\n",
        "[timeout] FFmpeg (media_processing): found, but it did not answer in time [per-call path]\n  It was started and had not answered when the deadline passed: 1 second, which the three tool checks share.",
        "[timeout] FFprobe (media_processing): found, but not started before the deadline passed [per-call path]\n  It was not started, because the deadline had already passed: 1 second, which the three tool checks share. Nothing is known about it yet.",
        "[timeout] Whisper (transcription): found, but not started before the deadline passed [per-call path]\n  It was not started, because the deadline had already passed: 1 second, which the three tool checks share. Nothing is known about it yet.",
        "Local ASR check: not run: no model is registered\n",
    ] {
        ensure(text.contains(expected), || {
            format!("the readable answer lacks {expected:?}: {text}")
        })?;
    }
    ensure(
        text.matches(
            "Run setup check again, or allow more time with --timeout-seconds (up to 60).\n",
        )
        .count()
            == 3,
        || format!("each tool has the timeout advice once: {text}"),
    )?;
    for phrase in INSTALL_ADVICE.into_iter().chain([
        "[missing]",
        "[unhealthy]",
        "not found",
        tool_text.as_str(),
    ]) {
        ensure(!text.contains(phrase), || {
            format!("the readable answer says {phrase:?}: {text}")
        })?;
    }

    // The JSON answer.
    let json = setup_check(&root, &tool, &["--json"])?;
    ensure(json.status.code() == Some(2), || {
        format!("JSON exit {:?}, expected 2", json.status.code())
    })?;
    ensure(json.stderr.is_empty(), || {
        String::from("the JSON answer wrote to standard error")
    })?;
    let raw = String::from_utf8(json.stdout)?;
    ensure(!raw.contains(tool_text.as_str()), || {
        format!("the JSON answer shows the tool's path: {raw}")
    })?;
    let value: Value = serde_json::from_str(&raw)?;
    jsonschema::validator_for(&schema()?)?
        .validate(&value)
        .map_err(|error| io::Error::other(format!("not a setup.check answer: {error}")))?;
    ensure(value["status"] == "blocked", || format!("{value}"))?;
    ensure(
        value["local_asr"]["verification"]["not_run_reason"] == "model_not_selected",
        || format!("{value}"),
    )?;
    let expected_steps = [
        format!(
            "FFmpeg was found and started but did not answer before the deadline all tool checks share (1 second).{AFTER_WHAT_HAPPENED}"
        ),
        format!(
            "FFprobe was found but not started: the deadline all tool checks share (1 second) had already passed.{AFTER_WHAT_HAPPENED}"
        ),
        format!(
            "whisper.cpp was found but not started: the deadline all tool checks share (1 second) had already passed.{AFTER_WHAT_HAPPENED}"
        ),
    ];
    for (index, expected_step) in expected_steps.iter().enumerate() {
        let dependency = &value["dependencies"][index];
        ensure(dependency["status"] == "timed_out", || {
            format!("{dependency}")
        })?;
        ensure(dependency["lookup"] == "explicit_path", || {
            format!("{dependency}")
        })?;
        ensure(dependency["remediation"]["reason"] == "timed_out", || {
            format!("{dependency}")
        })?;
        let step = dependency["remediation"]["next_step"]
            .as_str()
            .ok_or_else(|| io::Error::other(format!("no next_step: {dependency}")))?;
        ensure(step == expected_step, || {
            format!("next_step is {step:?}, expected {expected_step:?}")
        })?;
        for phrase in INSTALL_ADVICE {
            ensure(!step.contains(phrase), || {
                format!("next_step says {phrase:?}: {step}")
            })?;
        }
    }
    Ok(())
}
