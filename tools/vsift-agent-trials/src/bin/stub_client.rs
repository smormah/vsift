//! A stand-in for Claude Code and Codex in the harness's own tests.
//!
//! It contacts nothing. Started by `run` in a trial workspace, it records
//! the arguments, the working directory and the environment variable names
//! it received in `.stub/invocation.json`, replays the recorded event
//! stream named by `.stub/behaviour.json` on stdout, writes a line to
//! stderr, optionally sleeps (to exercise the timeout) and exits with the
//! configured code. `--version` prints a version and exits.
//!
//! It also stands in for the `vsift` executable where a test prepares a
//! clean-install trial without a real install: `setup check`, `setup plan`
//! and `setup install` answer with canned JSON of the published shapes, and
//! `setup install` checks that it was given the saved plan unmodified and
//! its own digest, as the real command does.

use std::{env, fs, path::Path, process::ExitCode, thread, time::Duration};

use serde_json::{Value, json};

/// The digest the canned plan carries.
const PLAN_DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn print(value: &Value) {
    println!("{value}");
}

/// The canned answers of a `vsift setup` command.
fn setup(arguments: &[String]) -> ExitCode {
    match arguments.first().map(String::as_str) {
        Some("check") => {
            print(&json!({
                "schema_version": "1",
                "command": "setup.check",
                "status": "ready",
                "dependencies": [
                    {"dependency": "ffmpeg", "status": "ready", "lookup": "managed"},
                    {"dependency": "ffprobe", "status": "ready", "lookup": "managed"},
                ],
                "local_asr": {"verification": {"status": "verified"}}
            }));
            ExitCode::SUCCESS
        }
        Some("plan") => {
            print(&json!({
                "schema_version": "1",
                "command": "setup.plan",
                "status": "complete",
                "data": {
                    "managed_install": "catalogue_accepted",
                    "install_needed": true,
                    "plan_digest": PLAN_DIGEST
                }
            }));
            ExitCode::SUCCESS
        }
        Some("install") => {
            let value = |flag: &str| {
                arguments
                    .iter()
                    .position(|argument| argument == flag)
                    .and_then(|index| arguments.get(index + 1))
            };
            let plan: Option<Value> = value("--plan")
                .and_then(|file| fs::read_to_string(file).ok())
                .and_then(|text| serde_json::from_str(&text).ok());
            let accepted = value("--accept-plan").map(String::as_str) == Some(PLAN_DIGEST)
                && plan
                    .as_ref()
                    .is_some_and(|plan| plan["data"]["plan_digest"] == PLAN_DIGEST);
            if accepted {
                print(
                    &json!({"schema_version": "1", "command": "setup.install", "status": "complete"}),
                );
                ExitCode::SUCCESS
            } else {
                print(
                    &json!({"schema_version": "1", "command": "setup.install", "status": "failed",
                              "error": {"code": "INVALID_ARGUMENT"}}),
                );
                ExitCode::from(2)
            }
        }
        _ => {
            print(&json!({"schema_version": "1", "status": "complete"}));
            ExitCode::SUCCESS
        }
    }
}

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.first().map(String::as_str) == Some("--version") {
        // The line the published `vsift --version` prints, so that the opt-in
        // install test (a loopback registry, the real npm and the real
        // launcher) can use this binary as the native executable.
        println!("vsift 0.1.0 (0123456789ab)");
        return ExitCode::SUCCESS;
    }
    if arguments.first().map(String::as_str) == Some("setup") {
        return setup(&arguments[1..]);
    }
    let stub = Path::new(".stub");
    let behaviour: Value = fs::read_to_string(stub.join("behaviour.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    let mut names: Vec<String> = env::vars_os()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .collect();
    names.sort();
    let invocation = json!({
        "arguments": arguments,
        "environment_names": names,
        "cwd": env::current_dir().map(|path| path.display().to_string()).unwrap_or_default(),
    });
    if fs::write(stub.join("invocation.json"), invocation.to_string()).is_err() {
        return ExitCode::from(90);
    }
    // The stream is in the workspace (`replay`) or, for a cold trial whose
    // workspace must not mention the tool, at an absolute path (`replay_file`).
    if let Some(replay) = behaviour["replay"].as_str()
        && let Ok(stream) = fs::read_to_string(stub.join(replay))
    {
        print!("{stream}");
    } else if let Some(file) = behaviour["replay_file"].as_str()
        && let Ok(stream) = fs::read_to_string(file)
    {
        print!("{stream}");
    }
    eprintln!("stub client stderr line");
    if let Some(sleep) = behaviour["sleep_ms"].as_u64() {
        thread::sleep(Duration::from_millis(sleep));
    }
    let code = behaviour["exit_code"]
        .as_u64()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(0);
    ExitCode::from(code)
}
