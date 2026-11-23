//! A stand-in for yt-dlp and `FFmpeg` in the page tests
//! (`tests/page_acquisition.rs`). It is never shipped or run outside those
//! tests, it opens no socket, and it uses only the standard library.
//!
//! `VSift` runs yt-dlp with a closed argument list and an environment it
//! chooses, so nothing here is told what to do through either: the test writes
//! a scenario to `scenario.txt` next to the executable, and the stand-in
//! appends what it was given to `calls.log` next to it. (The temporary folder `VSift` gives it goes with the session, so a
//! record kept there could not be read afterwards.)
//!
//! A copy staged under a name that starts `ffmpeg` joins: it writes its two
//! inputs, one after the other, to its last argument. A copy staged under any
//! other name is yt-dlp.
//!
//! The scenario's lines, each `<key> <rest of line>`:
//! - `info <file>`: phase 1 prints that file's bytes;
//! - `write <name> <hex magic> <bytes>`: phase 2 writes a file of that many
//!   bytes, beginning with the magic, into the folder after `--paths`;
//! - `link <name> <target>`: phase 2 makes a symbolic link there (Unix only);
//! - `outside <name>`: phase 2 writes a file beside the folder, not in it;
//! - `stderr <text>`: writes the line to standard error;
//! - `exit1 <code>`, `exit2 <code>`: the run's exit status for that phase;
//! - `hang-phase <1|2>`: sleeps past any deadline in that phase;
//! - `grow <name> <hex magic> <bytes>`: writes that much, then stalls;
//! - `join-fail`: the `FFmpeg` copy exits 1.

use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    thread,
    time::Duration,
};

const HANG: Duration = Duration::from_secs(600);

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            let _ = writeln!(io::stderr(), "stand-in failed: {error}");
            ExitCode::from(70)
        }
    }
}

fn beside_me(name: &str) -> io::Result<PathBuf> {
    let me = env::current_exe()?;
    let folder = me
        .parent()
        .ok_or_else(|| io::Error::other("no parent folder"))?;
    Ok(folder.join(name))
}

fn run() -> io::Result<u8> {
    let arguments: Vec<String> = env::args_os()
        .skip(1)
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    record(&arguments)?;
    let scenario = fs::read_to_string(beside_me("scenario.txt")?)?;
    let name = env::current_exe()?
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.starts_with("ffmpeg") {
        join(&arguments, &scenario)
    } else {
        yt_dlp(&arguments, &scenario)
    }
}

/// Appends one call: its arguments, then every variable's name, and the value
/// of the ones `VSift` sets (a value holds no secret the test did not plant).
fn record(arguments: &[String]) -> io::Result<()> {
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(beside_me("calls.log")?)?;
    writeln!(log, "call")?;
    for argument in arguments {
        writeln!(log, "arg {argument}")?;
    }
    for (name, value) in env::vars_os() {
        writeln!(
            log,
            "env {}={}",
            name.to_string_lossy(),
            value.to_string_lossy()
        )?;
    }
    Ok(())
}

fn lines<'a>(scenario: &'a str, key: &str) -> Vec<&'a str> {
    scenario
        .lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(found, _)| *found == key)
        .map(|(_, rest)| rest)
        .collect()
}

fn exit_code(scenario: &str, key: &str) -> u8 {
    lines(scenario, key)
        .first()
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

fn yt_dlp(arguments: &[String], scenario: &str) -> io::Result<u8> {
    let phase_one = arguments.iter().any(|argument| argument == "--simulate");
    let phase = if phase_one { "1" } else { "2" };
    for line in lines(scenario, "stderr") {
        writeln!(io::stderr(), "{line}")?;
    }
    if lines(scenario, "hang-phase").contains(&phase) {
        thread::sleep(HANG);
    }
    if phase_one {
        for file in lines(scenario, "info") {
            io::stdout().write_all(&fs::read(beside_me(file)?)?)?;
        }
        return Ok(exit_code(scenario, "exit1"));
    }
    let output = arguments
        .iter()
        .position(|argument| argument == "--paths")
        .and_then(|index| arguments.get(index + 1))
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("no --paths"))?;
    for line in lines(scenario, "write") {
        write_media(&output, line)?;
    }
    for line in lines(scenario, "outside") {
        let beside = output
            .parent()
            .ok_or_else(|| io::Error::other("no parent folder"))?;
        fs::write(beside.join(line), b"outside")?;
    }
    for line in lines(scenario, "link") {
        make_link(&output, line)?;
    }
    for line in lines(scenario, "grow") {
        write_media(&output, line)?;
        thread::sleep(HANG);
    }
    Ok(exit_code(scenario, "exit2"))
}

/// Writes `<name> <hex magic> <bytes>` into `folder`.
fn write_media(folder: &Path, line: &str) -> io::Result<()> {
    let mut parts = line.split(' ');
    let (Some(name), Some(magic), Some(bytes)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(io::Error::other("a write takes three words"));
    };
    let mut body = decode_hex(magic)?;
    let bytes: usize = bytes.parse().map_err(io::Error::other)?;
    body.resize(bytes.max(body.len()), 0x5a);
    fs::write(folder.join(name), body)
}

fn decode_hex(text: &str) -> io::Result<Vec<u8>> {
    (0..text.len() / 2)
        .map(|index| {
            let pair = text
                .get(index * 2..index * 2 + 2)
                .ok_or_else(|| io::Error::other("odd hex"))?;
            u8::from_str_radix(pair, 16).map_err(io::Error::other)
        })
        .collect()
}

#[cfg(unix)]
fn make_link(folder: &Path, line: &str) -> io::Result<()> {
    let (name, target) = line
        .split_once(' ')
        .ok_or_else(|| io::Error::other("a link takes two words"))?;
    std::os::unix::fs::symlink(target, folder.join(name))
}

#[cfg(not(unix))]
fn make_link(_folder: &Path, _line: &str) -> io::Result<()> {
    Err(io::Error::other("links are tested on Unix"))
}

/// Writes every `-i` input, in order, to the last argument, as a stream copy
/// of two files would hold both.
fn join(arguments: &[String], scenario: &str) -> io::Result<u8> {
    if scenario.lines().any(|line| line == "join-fail") {
        return Ok(1);
    }
    let output = arguments
        .last()
        .ok_or_else(|| io::Error::other("no output"))?;
    let mut bytes = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        if argument == "-i"
            && let Some(input) = arguments.get(index + 1)
        {
            bytes.extend(fs::read(input)?);
        }
    }
    fs::write(output, bytes)?;
    Ok(0)
}
