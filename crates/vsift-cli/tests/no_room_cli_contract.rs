//! CLI contract for a source that does not fit the session root's filesystem
//! (#266), through the real binary and the real engine.
//!
//! A sparse file far larger than any free space stands in for a big video: its
//! length is what the free-space check reads, and nothing is ever written,
//! because the check refuses before the copy starts. **The code stays
//! `STORAGE_IO`** (exit 7): the published code of the CLI path cannot change
//! within v1 (known limit L-127), so the answer's remediation says what
//! happened and what to do.
//!
//! Unix only: Windows reads no free space (L-061), so a source that does not
//! fit is found there by the write that runs out of room.

#![cfg(unix)]

use std::{
    env,
    error::Error,
    fs::{self, File},
    path::{Path, PathBuf},
    process::Command as Process,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;
use vsift_contract::SOURCE_NO_ROOM_REMEDIATION;

type TestResult = Result<(), Box<dyn Error>>;

const PREFIX: &str = "vsift-no-room-cli-";
static NEXT_BASE: AtomicU64 = AtomicU64::new(0);

/// The length of the stand-in source: 4 TiB, below the 16 TiB file limit of
/// the common Linux filesystems, and above any free space a runner has.
const SOURCE_BYTES: u64 = 4 * 1024 * 1024 * 1024 * 1024;

/// A scratch folder holding the stand-in source and an isolated per-user area.
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

/// The free space, in bytes, of the filesystem holding `directory`, read with
/// the system's `df -Pk` (one line per filesystem, available kibibytes in the
/// fourth column from the left and the third from the right).
fn free_bytes(directory: &Path) -> Result<u64, Box<dyn Error>> {
    let output = Process::new("df").arg("-Pk").arg(directory).output()?;
    if !output.status.success() {
        return Err(format!("df failed: {output:?}").into());
    }
    let text = String::from_utf8(output.stdout)?;
    let line = text.lines().last().ok_or("df printed nothing")?;
    let columns: Vec<&str> = line.split_whitespace().collect();
    let available = columns
        .len()
        .checked_sub(3)
        .and_then(|index| columns.get(index))
        .ok_or("df printed too few columns")?;
    Ok(available.parse::<u64>()?.saturating_mul(1024))
}

#[test]
fn a_source_that_does_not_fit_is_refused_before_the_copy_and_says_what_to_do() -> TestResult {
    let base = Base::new()?;
    let source = base.0.join("huge.mp4");
    // Never copied: but if this machine really had room for it, the copy would
    // start and fill the disk with zeros, so it is not run there.
    let free = free_bytes(&base.0)?;
    if free >= SOURCE_BYTES / 2 {
        assert!(
            env::var_os("CI").is_none(),
            "a hosted runner has more than {free} bytes free: the test would copy {SOURCE_BYTES} bytes"
        );
        eprintln!(
            "SKIPPED: {free} bytes are free here, too many to stand in for a source that does not fit"
        );
        return Ok(());
    }
    File::create(&source)?.set_len(SOURCE_BYTES)?;
    let user = base.0.join("user");

    let output = Command::cargo_bin("vsift")?
        .env("XDG_CONFIG_HOME", &user)
        .env("XDG_CACHE_HOME", &user)
        .env("HOME", &user)
        .arg("--session-root")
        .arg(base.0.join("sessions"))
        .arg("ingest")
        .arg(&source)
        .arg("--json")
        .output()?;

    let value: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(value["error"]["code"], "STORAGE_IO", "{value}");
    assert_eq!(
        value["error"]["remediation"][0]["summary"], SOURCE_NO_ROOM_REMEDIATION,
        "{value}"
    );
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    assert!(
        output.stderr.is_empty(),
        "a JSON command wrote to standard error: {output:?}"
    );
    Ok(())
}
