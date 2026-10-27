//! `vsift <video>` through the binary (ADR 0037), with no media tools needed:
//! the bare-argument rule's answers. A folder named as the video gets the same
//! answer `ingest` gives it, under the command `watch` (the planner's
//! condition C); a missing video that looks like a path is "not found"; a
//! mistyped command stays an unknown command; and a video that cannot be
//! watched leaves nothing beside it.

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
use vsift_contract::SOURCE_NOT_FOUND_REMEDIATION;

type TestResult = Result<(), Box<dyn Error>>;

const PREFIX: &str = "vsift-watch-cli-";
static NEXT_BASE: AtomicU64 = AtomicU64::new(0);

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

    /// Runs `vsift` in the base folder, with an isolated per-user area.
    fn run(&self, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
        let user = self.0.join("user");
        Ok(Command::cargo_bin("vsift")?
            .current_dir(&self.0)
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
}

impl Drop for Base {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn json(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn text(path: &Path) -> Result<&str, Box<dyn Error>> {
    path.to_str().ok_or_else(|| "not Unicode".into())
}

#[test]
fn a_folder_named_as_the_video_gets_ingests_own_answer() -> TestResult {
    let base = Base::new()?;
    let folder = base.0.join("a folder");
    fs::create_dir(&folder)?;
    let watched = base.run(&[text(&folder)?])?;
    let ingested = base.run(&["ingest", text(&folder)?])?;
    let (watched_value, ingested_value) = (json(&watched)?, json(&ingested)?);
    assert_eq!(watched_value["command"], "watch", "{watched_value}");
    assert_eq!(watched_value["error"], ingested_value["error"], "{watched_value}");
    assert_eq!(watched.status.code(), ingested.status.code());
    Ok(())
}

#[test]
fn a_missing_video_is_not_found_and_a_mistyped_command_is_unknown() -> TestResult {
    let base = Base::new()?;
    let missing = base.run(&["meetng.mp4"])?;
    let value = json(&missing)?;
    assert_eq!(value["command"], "watch", "{value}");
    assert_eq!(value["error"]["code"], "STORAGE_IO", "{value}");
    assert_eq!(value["error"]["remediation"][0]["summary"], SOURCE_NOT_FOUND_REMEDIATION);

    let mistyped = base.run(&["serach"])?;
    let value = json(&mistyped)?;
    assert_eq!(value["command"], "parse", "{value}");
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT", "{value}");
    Ok(())
}

#[test]
fn nothing_is_written_beside_a_video_that_could_not_be_watched() -> TestResult {
    let base = Base::new()?;
    let folder = base.0.join("videos");
    fs::create_dir(&folder)?;
    let video = folder.join("clip.mp4");
    fs::write(&video, b"\0\0\0\x18ftypisomnot-a-real-video")?;
    let output = base.run(&[text(&video)?])?;
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    let left: Vec<_> = fs::read_dir(&folder)?.collect::<Result<_, _>>()?;
    assert_eq!(left.len(), 1, "only the video is there: {left:?}");
    Ok(())
}
