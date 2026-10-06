//! CLI contract for a source that is too big for `ingest`, through the real
//! binary and the real engine: one that does not fit the session root's
//! filesystem (#266) and one that is over the source-size limit (#310).
//!
//! A sparse file stands in for a big video: its length is what the checks read,
//! and nothing is ever written, because both refuse before the copy starts.
//!
//! * **Over the 20 GiB limit, whatever the free space: `INVALID_SOURCE`**
//!   (exit 3), the answer 0.1.0 gave. The room check added for #266 once ran
//!   first and answered `STORAGE_IO` when such a source was also larger than the
//!   free space, which changed a published code (known limits L-126, L-127).
//! * **Within the limit but larger than the free space: `STORAGE_IO`** (exit 7),
//!   the published code of the CLI path, with a remediation that says what
//!   happened and what to do (known limit L-127). A source within the limit can
//!   only be larger than the free space on a small enough filesystem, so that
//!   test makes its source a little larger than what is free where the session
//!   root is, and looks for such a filesystem (see [`small_filesystem`]).
//!
//! Unix only: Windows reads no free space (L-061), so a source that does not
//! fit is found there by the write that runs out of room.

#![cfg(unix)]

use std::{
    env,
    error::Error,
    fs::{self, File},
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command as Process, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;
use vsift_contract::SOURCE_NO_ROOM_REMEDIATION;
use vsift_infrastructure::MAX_SOURCE_BYTES;

type TestResult = Result<(), Box<dyn Error>>;

const PREFIX: &str = "vsift-no-room-cli-";
static NEXT_BASE: AtomicU64 = AtomicU64::new(0);

/// The length of the stand-in for a source over the limit: 4 TiB, far above the
/// 20 GiB limit and any free space a machine has, and below the 16 TiB file
/// limit of the common Linux filesystems.
const OVER_THE_LIMIT_BYTES: u64 = 4 * 1024 * 1024 * 1024 * 1024;

/// How far beyond the free space the stand-in for a source that does not fit
/// is: far more than the 16 MiB margin the check keeps, so that other writers
/// to the same disk while the test runs do not change the answer.
const BEYOND_FREE_BYTES: u64 = 1024 * 1024 * 1024;

/// What a refused ingest may leave in the session root: its small records. A
/// copy that had started would have left far more.
const NOTHING_COPIED_BYTES: u64 = 1024 * 1024;

/// A scratch folder holding a stand-in source or a session root, and an
/// isolated per-user area.
struct Base(PathBuf);

impl Base {
    /// A new scratch folder in the temporary folder.
    fn new() -> Result<Self, Box<dyn Error>> {
        Self::in_directory(&env::temp_dir())
    }

    /// A new scratch folder in `parent`.
    fn in_directory(parent: &Path) -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_BASE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("{PREFIX}{}-{stamp}-{sequence}", std::process::id()));
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

/// The bytes of every file under `directory`, so a test can say that nothing
/// was copied. A folder that does not exist holds none.
fn bytes_under(directory: &Path) -> Result<u64, Box<dyn Error>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    let mut total = 0_u64;
    for entry in entries {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let bytes = if metadata.is_dir() {
            bytes_under(&entry.path())?
        } else {
            metadata.len()
        };
        total = total.saturating_add(bytes);
    }
    Ok(total)
}

/// A scratch folder for a session root, on a filesystem so small that a source
/// the size limit accepts does not fit it, and the length of such a source: a
/// little more than what is free there. `None` on a machine that has no such
/// filesystem, which is one with more than the limit free everywhere we look.
///
/// The temporary folder is tried first; then `/dev/shm`, the shared-memory
/// filesystem of Linux, which is sized by the machine's memory (a hosted runner
/// has a few GiB) and exists in a container too. A filesystem that reports
/// nothing available is passed over: the best-effort check does not refuse on
/// that (known limit L-061).
fn small_filesystem() -> Result<Option<(Base, u64)>, Box<dyn Error>> {
    for parent in [env::temp_dir(), PathBuf::from("/dev/shm")] {
        if !parent.is_dir() {
            continue;
        }
        let candidate = Base::in_directory(&parent)?;
        let free = free_bytes(&candidate.0)?;
        let size = free.saturating_add(BEYOND_FREE_BYTES);
        if free > 0 && size <= MAX_SOURCE_BYTES {
            return Ok(Some((candidate, size)));
        }
    }
    Ok(None)
}

/// `vsift --session-root <session_root> ingest <source> --json`, with the
/// per-user areas of the command redirected into `scratch`.
fn ingest(scratch: &Base, session_root: &Path, source: &Path) -> Result<Output, Box<dyn Error>> {
    let user = scratch.0.join("user");
    Ok(Command::cargo_bin("vsift")?
        .env("XDG_CONFIG_HOME", &user)
        .env("XDG_CACHE_HOME", &user)
        .env("HOME", &user)
        .arg("--session-root")
        .arg(session_root)
        .arg("ingest")
        .arg(source)
        .arg("--json")
        .output()?)
}

/// #310: a source over the size limit is `INVALID_SOURCE` whatever the free
/// space, as it was in 0.1.0. The stand-in is larger than any disk, so the room
/// check of #266, run first, answered `STORAGE_IO` (this test failed on that
/// code); with the limit answering first the answer does not depend on the
/// disk at all, and a machine that did have the room for it would answer the
/// same, at once, without a copy.
#[test]
fn a_source_over_the_size_limit_is_invalid_whatever_the_free_space() -> TestResult {
    let base = Base::new()?;
    let source = base.0.join("huge.mp4");
    File::create(&source)?.set_len(OVER_THE_LIMIT_BYTES)?;
    let sessions = base.0.join("sessions");

    let output = ingest(&base, &sessions, &source)?;

    let value: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(value["error"]["code"], "INVALID_SOURCE", "{value}");
    assert_ne!(
        value["error"]["remediation"][0]["summary"], SOURCE_NO_ROOM_REMEDIATION,
        "the answer for a source over the limit must not be the one for no room: {value}"
    );
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        output.stderr.is_empty(),
        "a JSON command wrote to standard error: {output:?}"
    );
    assert!(
        bytes_under(&sessions)? < NOTHING_COPIED_BYTES,
        "the source was copied"
    );
    Ok(())
}

/// #266, unchanged by #310: a source the limit accepts that does not fit the
/// session root's filesystem is refused before the copy, as `STORAGE_IO` with
/// the remediation that says what to do.
#[test]
fn a_source_within_the_limit_that_does_not_fit_is_refused_before_the_copy() -> TestResult {
    let Some((small, source_bytes)) = small_filesystem()? else {
        // A hosted Linux runner always has a small enough /dev/shm; if it does
        // not, this test would silently stop guarding the wiring of the check.
        assert!(
            env::var_os("CI").is_none() || !cfg!(target_os = "linux"),
            "a hosted Linux runner has more than the size limit free in its temporary folder and in /dev/shm"
        );
        eprintln!(
            "SKIPPED: more than the size limit is free in the temporary folder and in /dev/shm, so no source the limit accepts can fail to fit"
        );
        return Ok(());
    };
    let base = Base::new()?;
    let source = base.0.join("large.mp4");
    File::create(&source)?.set_len(source_bytes)?;
    let sessions = small.0.join("sessions");

    let output = ingest(&base, &sessions, &source)?;

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
    assert!(
        bytes_under(&sessions)? < NOTHING_COPIED_BYTES,
        "the source was copied"
    );
    Ok(())
}
