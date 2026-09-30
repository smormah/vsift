//! A stand-in for `FFmpeg`, `FFprobe` and `whisper-cli` in the P13
//! compatibility-smoke tests (`tests/p13_smoke_executor.rs`). It is never
//! shipped or run outside those tests.
//!
//! The smoke runs staged executables by explicit path with a closed argument
//! list and an empty environment, so a fake cannot be told what to do by an
//! argument or a variable the way `tests/process_supervisor.rs` re-invokes its
//! own test binary. It reads its behaviour from its own file name instead,
//! `<tool>-<behaviour>` with an optional platform suffix: the tests stage it
//! under the names they need. It uses only the standard library, so it stays
//! a small file to copy, hash and stage several times per test.
//!
//! Behaviours: `good` prints the tool's reviewed fixture banner (or a usage
//! line for `--help`); `badbanner` prints another banner; `flood` writes to
//! standard output without end; `hang` sleeps past any smoke deadline;
//! `fail` exits unsuccessfully; `scribble` writes a file into its own
//! directory, then behaves as `good`. For any other argument list it writes
//! nothing and exits successfully, so a real fixture check sees output that
//! is not the fixture's truth.

use std::{
    env,
    io::{self, Write},
    process::ExitCode,
    thread,
    time::Duration,
};

/// The banner build the smoke tests' policy expects.
const FIXTURE_BUILD: &str = "vsift-smoke-fixture";
/// The file `scribble` writes next to itself.
const SCRIBBLE_FILE: &str = "vsift-smoke-scribble";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(_) => ExitCode::from(70),
    }
}

fn run() -> io::Result<ExitCode> {
    let executable = env::current_exe()?;
    let stem = executable
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_owned();
    // The production name `whisper-cli` is the good recognizer, so a
    // runtime laid out under the reviewed names behaves well.
    let (tool, behaviour) = if stem == "whisper-cli" {
        ("whisper", "good")
    } else {
        stem.split_once('-').unwrap_or((stem.as_str(), "good"))
    };
    let arguments: Vec<String> = env::args().skip(1).collect();
    let banner = matches!(arguments.as_slice(), [only] if only == "-version");
    let help = matches!(arguments.as_slice(), [only] if only == "--help");
    let mut stdout = io::stdout().lock();
    match behaviour {
        "flood" => {
            let chunk = [b'x'; 8192];
            loop {
                stdout.write_all(&chunk)?;
            }
        }
        "hang" => {
            thread::sleep(Duration::from_secs(300));
            Ok(ExitCode::SUCCESS)
        }
        "fail" => {
            writeln!(io::stderr(), "{tool}: fixture failure")?;
            Ok(ExitCode::from(3))
        }
        "badbanner" => {
            if banner {
                writeln!(stdout, "{tool} version another-build Copyright (c) fixture")?;
            }
            Ok(ExitCode::SUCCESS)
        }
        "scribble" | "good" => {
            if behaviour == "scribble"
                && let Some(directory) = executable.parent()
            {
                std::fs::write(directory.join(SCRIBBLE_FILE), b"written by a provider")?;
            }
            if banner {
                writeln!(
                    stdout,
                    "{tool} version {FIXTURE_BUILD} Copyright (c) fixture"
                )?;
                writeln!(stdout, "built with the standard library")?;
            } else if help {
                writeln!(io::stderr(), "usage: {tool} [options] file0 file1 ...")?;
            }
            Ok(ExitCode::SUCCESS)
        }
        _ => Ok(ExitCode::from(64)),
    }
}
