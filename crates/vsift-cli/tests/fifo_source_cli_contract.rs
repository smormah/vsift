//! CLI contract for a named pipe named as the source of `ingest` (#264).
//!
//! `vsift ingest <pipe>` where nothing ever opens the pipe for writing used to
//! wait for ever: the source was opened for reading before its file type was
//! checked, and opening a pipe for reading blocks until a writer appears. It
//! now ends at once with `INVALID_SOURCE`, like a folder or a text file named
//! `.mp4`. The test gives the command a deadline, because a build that waits
//! must fail this test, not hang the run.

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

#[test]
fn a_named_pipe_with_no_writer_is_refused_as_an_invalid_source() -> TestResult {
    let base = Base::new()?;
    let pipe = base.0.join("pipe.mp4");
    let made = Process::new("mkfifo").arg(&pipe).status()?;
    assert!(made.success(), "mkfifo failed: {made:?}");
    let user = base.0.join("user");

    let output = Command::cargo_bin("vsift")?
        .env("XDG_CONFIG_HOME", &user)
        .env("XDG_CACHE_HOME", &user)
        .env("HOME", &user)
        .arg("--session-root")
        .arg(base.0.join("sessions"))
        .arg("ingest")
        .arg(&pipe)
        .arg("--json")
        .timeout(Duration::from_secs(30))
        .output()
        .map_err(|error| format!("ingest of a named pipe did not end within 30 s: {error}"))?;

    assert!(!output.status.success(), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(value["error"]["code"], "INVALID_SOURCE", "{value}");
    assert!(
        output.stderr.is_empty(),
        "a JSON command wrote to standard error: {output:?}"
    );
    Ok(())
}
