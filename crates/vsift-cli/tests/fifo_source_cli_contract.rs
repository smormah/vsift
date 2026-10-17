//! CLI contract for a named pipe, a device or a socket named as the source of
//! `ingest`, or as its supplied transcript (#264).
//!
//! `vsift ingest <pipe>` where nothing ever opens the pipe for writing used to
//! wait for ever: the source was opened for reading before its file type was
//! checked, and opening a pipe for reading blocks until a writer appears.
//! Since 0.2.0 it ended at once with `INVALID_SOURCE`, like a folder or a text
//! file named `.mp4`, after an open that did not wait. Now the directory entry
//! says what the path is and nothing opens it, so a device is not acted on
//! either; each kind keeps the code it had (a pipe and a device
//! `INVALID_SOURCE`, a socket `STORAGE_IO`, known limit L-127) and carries the
//! remediation that says it is not a regular file. The tests give the command
//! a deadline, because a build that waits must fail a test, not hang the run.

#![cfg(unix)]

use std::{
    env,
    error::Error,
    fs,
    path::PathBuf,
    process::Command as Process,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;
use vsift_contract::SOURCE_SPECIAL_FILE_REMEDIATION;

type TestResult = Result<(), Box<dyn Error>>;

const PREFIX: &str = "vsift-fifo-source-cli-";
static NEXT_BASE: AtomicU64 = AtomicU64::new(0);

/// A scratch folder holding the pipe and an isolated per-user area.
struct Base(PathBuf);

impl Base {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_BASE.fetch_add(1, Ordering::Relaxed);
        let path =
            env::temp_dir().join(format!("{PREFIX}{}-{stamp}-{sequence}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Base {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// Runs `vsift --json` with `arguments` under an isolated per-user area and a
/// session root in `base`, ended after 30 s; returns the exit status and the
/// JSON answer.
fn ingest(
    base: &Base,
    arguments: &[&std::ffi::OsStr],
) -> Result<(Option<i32>, Value), Box<dyn Error>> {
    let user = base.0.join("user");
    let output = Command::cargo_bin("vsift")?
        .env("XDG_CONFIG_HOME", &user)
        .env("XDG_CACHE_HOME", &user)
        .env("HOME", &user)
        .arg("--session-root")
        .arg(base.0.join("sessions"))
        .arg("ingest")
        .args(arguments)
        .arg("--json")
        .timeout(Duration::from_secs(30))
        .output()
        .map_err(|error| format!("ingest did not end within 30 s: {error}"))?;
    assert!(
        output.stderr.is_empty(),
        "a JSON command wrote to standard error: {output:?}"
    );
    Ok((
        output.status.code(),
        serde_json::from_slice(&output.stdout)?,
    ))
}

/// Asserts that `value` is the special-file refusal with `code`.
fn assert_special_file(value: &Value, code: &str) {
    assert_eq!(value["status"], "failed", "{value}");
    assert_eq!(value["error"]["code"], code, "{value}");
    assert_eq!(
        value["error"]["remediation"][0]["summary"], SOURCE_SPECIAL_FILE_REMEDIATION,
        "{value}"
    );
}

#[test]
fn a_named_pipe_with_no_writer_is_refused_as_an_invalid_source() -> TestResult {
    let base = Base::new()?;
    let pipe = base.0.join("pipe.mp4");
    let made = Process::new("mkfifo").arg(&pipe).status()?;
    assert!(made.success(), "mkfifo failed: {made:?}");

    let (status, value) = ingest(&base, &[pipe.as_os_str()])?;
    assert_eq!(status, Some(3), "{value}");
    assert_special_file(&value, "INVALID_SOURCE");

    // The same pipe as a supplied transcript: refused alike, before the video
    // (which does not exist) is looked at.
    let video = base.0.join("absent.mp4");
    let (status, value) = ingest(
        &base,
        &[video.as_os_str(), "--transcript".as_ref(), pipe.as_os_str()],
    )?;
    assert_eq!(status, Some(3), "{value}");
    assert_special_file(&value, "INVALID_SOURCE");
    Ok(())
}

/// A device keeps `INVALID_SOURCE`, the documented answer, without being
/// opened; a socket keeps `STORAGE_IO`, the answer of its failed open.
#[test]
fn a_device_and_a_socket_keep_their_codes_and_say_what_they_are() -> TestResult {
    let base = Base::new()?;
    let (status, value) = ingest(&base, &["/dev/null".as_ref()])?;
    assert_eq!(status, Some(3), "{value}");
    assert_special_file(&value, "INVALID_SOURCE");

    // A short path: a socket's address is limited to about a hundred bytes.
    let socket = PathBuf::from(format!(
        "/tmp/vsift-fifo-cli-{}-{}.sock",
        std::process::id(),
        NEXT_BASE.fetch_add(1, Ordering::Relaxed)
    ));
    let listener = std::os::unix::net::UnixListener::bind(&socket)?;
    let answer = ingest(&base, &[socket.as_os_str()]);
    drop(listener);
    let _ = fs::remove_file(&socket);
    let (status, value) = answer?;
    assert_eq!(status, Some(7), "{value}");
    assert_special_file(&value, "STORAGE_IO");
    Ok(())
}
