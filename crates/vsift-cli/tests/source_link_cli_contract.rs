//! CLI contract for a symbolic link named as the source (#265).
//!
//! `vsift ingest <link>` answered `STORAGE_IO` with no remediation, the answer of
//! a disk that failed, found by the P14 malicious-media campaign. A link is not
//! followed, and the file it points to is not read. **The code stays
//! `STORAGE_IO`** (exit 7): changing a published failure code is not additive
//! within v1 (known limit L-127, maintainer decision of 2026-10-04), so the
//! answer gains a remediation that says links are not followed and to name the
//! file itself. A supplied transcript that is a link gets the same. The worker
//! path's answer for a link in an input root (`path_outside_input_root`,
//! `INVALID_ARGUMENT`) is a different question, asked of an operator's folder,
//! and is unchanged (`engine_worker`).

use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;
use vsift_contract::SOURCE_IS_LINK_REMEDIATION;

type TestResult = Result<(), Box<dyn Error>>;

const PREFIX: &str = "vsift-source-link-cli-";
static NEXT_BASE: AtomicU64 = AtomicU64::new(0);

/// A scratch folder holding an isolated per-user area and the session root.
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

    fn run(&self, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
        let user = self.0.join("user");
        Ok(Command::cargo_bin("vsift")?
            .env("LOCALAPPDATA", &user)
            .env("XDG_CONFIG_HOME", &user)
            .env("XDG_CACHE_HOME", &user)
            .env("HOME", &user)
            .arg("--session-root")
            .arg(self.0.join("sessions"))
            .args(arguments)
            .arg("--json")
            .output()?)
    }

    /// A file and a link to it.
    ///
    /// `Ok(None)` is an **explicit skip**: it only happens on Windows, for an
    /// account that does not hold the privilege to create links (error 1314),
    /// and never when `CI` is set (a hosted run can always make links and fails
    /// instead of skipping). The test prints why it checked nothing.
    fn file_and_link(
        &self,
        name: &str,
        bytes: &[u8],
    ) -> Result<Option<(PathBuf, PathBuf)>, Box<dyn Error>> {
        let file = self.0.join(name);
        fs::write(&file, bytes)?;
        let link = self.0.join(format!("link-to-{name}"));
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&file, &link);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&file, &link);
        match made {
            Ok(()) => Ok(Some((file, link))),
            Err(error)
                if cfg!(windows)
                    && error.raw_os_error() == Some(1314)
                    && env::var_os("CI").is_none() =>
            {
                eprintln!(
                    "SKIPPED: this account may not create symbolic links ({error}); the test checks nothing here"
                );
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
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

fn json(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn path_text(path: &Path) -> Result<&str, Box<dyn Error>> {
    path.to_str().ok_or_else(|| "a path is not UTF-8".into())
}

/// The answer of 0.1.0, with the remediation it lacked.
fn assert_the_link_answer(output: &Output) -> TestResult {
    let value = json(output)?;
    assert_eq!(value["error"]["code"], "STORAGE_IO", "{value}");
    assert_eq!(
        value["error"]["remediation"][0]["summary"], SOURCE_IS_LINK_REMEDIATION,
        "{value}"
    );
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    Ok(())
}

#[test]
fn ingest_of_a_link_keeps_its_code_and_says_what_happened() -> TestResult {
    let base = Base::new()?;
    let Some((_, link)) = base.file_and_link("source.mp4", b"\0\0\0\x18ftypisomlink-source")?
    else {
        return Ok(());
    };

    assert_the_link_answer(&base.run(&["ingest", path_text(&link)?])?)
}

#[test]
fn a_transcript_that_is_a_link_gets_the_same_answer() -> TestResult {
    let base = Base::new()?;
    let Some((_, link)) =
        base.file_and_link("walkthrough.srt", b"1\n00:00:00,500 --> 00:00:01,000\nx\n")?
    else {
        return Ok(());
    };
    let media = base.0.join("source.mp4");
    fs::write(&media, b"\0\0\0\x18ftypisomlink-source")?;

    assert_the_link_answer(&base.run(&[
        "ingest",
        path_text(&media)?,
        "--transcript",
        path_text(&link)?,
    ])?)
}

/// Only the file itself is checked: a link in a parent folder is followed, as
/// the operating system resolves any path, and the file behind it is accepted.
/// (`cli-v1.md` says so; the worker path refuses links anywhere on a path.)
#[test]
fn a_link_in_a_parent_folder_is_followed_and_the_file_is_ingested() -> TestResult {
    let base = Base::new()?;
    let folder = base.0.join("real");
    fs::create_dir(&folder)?;
    fs::write(folder.join("source.mp4"), b"\0\0\0\x18ftypisomlink-source")?;
    let linked = base.0.join("linked-folder");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&folder, &linked);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(&folder, &linked);
    match made {
        Ok(()) => {}
        Err(error)
            if cfg!(windows)
                && error.raw_os_error() == Some(1314)
                && env::var_os("CI").is_none() =>
        {
            eprintln!(
                "SKIPPED: this account may not create symbolic links ({error}); the test checks nothing here"
            );
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    }

    let output = base.run(&["ingest", path_text(&linked.join("source.mp4"))?])?;
    let value = json(&output)?;

    assert_eq!(value["status"], "complete", "{value}");
    Ok(())
}
