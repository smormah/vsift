//! A stand-in for Claude Code and Codex in the harness's own tests.
//!
//! It contacts nothing. Started by `run` in a trial workspace, it records
//! the arguments, the working directory and the environment variable names
//! it received in `.stub/invocation.json`, replays the recorded event
//! stream named by `.stub/behaviour.json` on stdout, writes a line to
//! stderr, optionally writes a Codex session rollout below `CODEX_HOME`,
//! optionally sleeps (to exercise the timeout) and exits with the
//! configured code. `--version` prints a version and exits.

use std::{env, fs, path::Path, process::ExitCode, thread, time::Duration};

use serde_json::{Value, json};

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.first().map(String::as_str) == Some("--version") {
        println!("vsift-trials-stub-client 0.0.0");
        return ExitCode::SUCCESS;
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
    if let Some(replay) = behaviour["replay"].as_str()
        && let Ok(stream) = fs::read_to_string(stub.join(replay))
    {
        print!("{stream}");
    }
    // Codex writes a session rollout below its home unless it is ephemeral.
    if let Some(rollout) = behaviour["rollout"].as_str()
        && let Ok(text) = fs::read_to_string(stub.join(rollout))
        && let Some(home) = env::var_os("CODEX_HOME")
    {
        let directory = Path::new(&home)
            .join("sessions")
            .join("2026")
            .join("09")
            .join("29");
        if fs::create_dir_all(&directory).is_err()
            || fs::write(directory.join("rollout-stub.jsonl"), text).is_err()
        {
            return ExitCode::from(91);
        }
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
