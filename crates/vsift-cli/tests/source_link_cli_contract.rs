//! CLI contract for a symbolic link named as the source (#265).
//!
//! `vsift ingest <link>` answered `STORAGE_IO` with no remediation, the answer of
//! a disk that failed, found by the P14 malicious-media campaign. A link is never
//! followed, and it is the caller's path that is wrong, so it is `INVALID_SOURCE`
//! (exit 3, the source class) with a remediation that says links are not followed
//! and to name the file itself. A supplied transcript that is a link is refused
//! as an invalid source too. The worker path's answer for a link in an input root
//! (`path_outside_input_root`, `INVALID_ARGUMENT`) is a different question, asked
//! of an operator's folder, and is unchanged (`engine_worker`).

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

    /// A file and a link to it; `None` when the platform will not make links
    /// (an unprivileged Windows account), where there is nothing to check.
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
        Ok(made.is_ok().then_some((file, link)))
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

#[test]
fn ingest_of_a_link_is_an_invalid_source_with_a_remediation() -> TestResult {
    let base = Base::new()?;
    let Some((_, link)) = base.file_and_link("source.mp4", b"\0\0\0\x18ftypisomlink-source")?
    else {
        return Ok(());
    };

    let output = base.run(&["ingest", path_text(&link)?])?;
    let value = json(&output)?;

    assert_eq!(value["error"]["code"], "INVALID_SOURCE", "{value}");
    assert_eq!(
        value["error"]["remediation"][0]["summary"], SOURCE_IS_LINK_REMEDIATION,
        "{value}"
    );
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    Ok(())
}

#[test]
fn a_transcript_that_is_a_link_is_an_invalid_source_too() -> TestResult {
    let base = Base::new()?;
    let Some((_, link)) =
        base.file_and_link("walkthrough.srt", b"1\n00:00:00,500 --> 00:00:01,000\nx\n")?
    else {
        return Ok(());
    };
    let media = base.0.join("source.mp4");
    fs::write(&media, b"\0\0\0\x18ftypisomlink-source")?;

    let output = base.run(&[
        "ingest",
        path_text(&media)?,
        "--transcript",
        path_text(&link)?,
    ])?;
    let value = json(&output)?;

    assert_eq!(value["error"]["code"], "INVALID_SOURCE", "{value}");
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    Ok(())
}
