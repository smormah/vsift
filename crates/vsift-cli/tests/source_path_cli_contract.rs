//! CLI contract for a path that names no file, names a folder, or may not be
//! read (#369).
//!
//! `vsift ingest <a name that is not a file>` answered `STORAGE_IO` (exit 7)
//! with an empty `remediation`, so the commonest first mistake (a mistyped name,
//! a command run from another folder) said nothing about what was wrong, and an
//! agent told to stop and report the code stopped there. **The code and the
//! exit status stay** (changing a published failure code is not additive within
//! v1, known limit L-127, maintainer decision of 2026-10-04): the answer gains a
//! remediation that says which of three things the operating system told apart.
//! The video's path and a supplied transcript's path are answered alike.
//!
//! A folder is the one case that differs by platform and always has: Windows
//! refuses to open a folder as a file (`STORAGE_IO`, now with its remediation),
//! Linux and macOS open it and find it is no regular file (`INVALID_SOURCE`,
//! unchanged). A link as the file itself is #265's, in `source_link_cli_contract`
//! and unchanged here.

use std::{
    env,
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;
use vsift_contract::{SOURCE_IS_FOLDER_REMEDIATION, SOURCE_NOT_FOUND_REMEDIATION};

type TestResult = Result<(), Box<dyn Error>>;

const PREFIX: &str = "vsift-source-path-cli-";
/// An ISO media header: enough for `ingest` to accept the file as a source.
const MEDIA: &[u8] = b"\0\0\0\x18ftypisomsource-path-contract";
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

    /// Runs `vsift` in `folder`, with an isolated per-user area and `--json`.
    fn run_in(&self, folder: &Path, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
        let user = self.0.join("user");
        Ok(Command::cargo_bin("vsift")?
            .current_dir(folder)
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

    fn run(&self, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
        self.run_in(&self.0, arguments)
    }

    /// A folder of its own, made on demand.
    fn folder(&self, name: &str) -> Result<PathBuf, Box<dyn Error>> {
        let folder = self.0.join(name);
        fs::create_dir_all(&folder)?;
        Ok(folder)
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

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// The envelope, validated against the published v1 schema.
fn json(output: &Output) -> Result<Value, Box<dyn Error>> {
    assert!(
        output.stderr.is_empty(),
        "a JSON failure wrote to the error stream"
    );
    let value: Value = serde_json::from_slice(&output.stdout)?;
    let schema: Value = serde_json::from_str(&fs::read_to_string(repository(
        "schemas/v1/operation-response.schema.json",
    ))?)?;
    jsonschema::validator_for(&schema)?
        .validate(&value)
        .map_err(|error| io::Error::other(format!("operation-response: {error}")))?;
    Ok(value)
}

fn path_text(path: &Path) -> Result<&str, Box<dyn Error>> {
    path.to_str().ok_or_else(|| "a path is not UTF-8".into())
}

/// The answer 0.2.0 gave (`STORAGE_IO`, exit 7, its fixed message, not
/// retryable), with the remediation it lacked.
fn assert_the_storage_io_answer(output: &Output, remediation: &str) -> TestResult {
    let value = json(output)?;
    assert_eq!(value["command"], "ingest", "{value}");
    assert_eq!(value["status"], "failed", "{value}");
    assert!(value["data"].is_null(), "{value}");
    assert_eq!(value["error"]["code"], "STORAGE_IO", "{value}");
    assert_eq!(
        value["error"]["message"], "Storage or output I/O prevented completion.",
        "{value}"
    );
    assert_eq!(value["error"]["retryable"], false, "{value}");
    let entries = value["error"]["remediation"]
        .as_array()
        .ok_or("remediation is not a list")?;
    assert_eq!(entries.len(), 1, "{value}");
    assert_eq!(entries[0]["summary"], remediation, "{value}");
    assert_eq!(entries[0]["required_authority"], "none", "{value}");
    assert!(entries[0]["command"].is_null(), "{value}");
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    Ok(())
}

#[test]
fn ingest_of_a_path_that_names_no_file_keeps_its_code_and_says_so() -> TestResult {
    let base = Base::new()?;
    let missing_file = base.0.join("no-such-video.mp4");
    let missing_folder = base.0.join("no-such-folder").join("video.mp4");
    for path in [&missing_file, &missing_folder] {
        let output = base.run(&["ingest", path_text(path)?])?;
        assert_the_storage_io_answer(&output, SOURCE_NOT_FOUND_REMEDIATION)?;
    }

    // A typo leaves nothing behind to clean up.
    let listed = json(&base.run(&["session", "list"])?)?;
    assert_eq!(listed["data"]["items"], serde_json::json!([]), "{listed}");
    Ok(())
}

/// What the remediation says about a relative path is true: it is read from
/// the folder the command ran in. The same name is a file from one folder and
/// no file from another.
#[test]
fn a_relative_path_is_read_from_the_folder_the_command_runs_in() -> TestResult {
    let base = Base::new()?;
    let here = base.folder("here")?;
    let elsewhere = base.folder("elsewhere")?;
    fs::write(here.join("source.mp4"), MEDIA)?;

    let found = json(&base.run_in(&here, &["ingest", "source.mp4"])?)?;
    assert_eq!(found["status"], "complete", "{found}");

    let output = base.run_in(&elsewhere, &["ingest", "source.mp4"])?;
    assert_the_storage_io_answer(&output, SOURCE_NOT_FOUND_REMEDIATION)?;
    assert!(SOURCE_NOT_FOUND_REMEDIATION.contains("folder the command was run in"));
    Ok(())
}

/// A supplied transcript's path is the same mistake and is answered alike; the
/// video, which exists, is not what fails.
#[test]
fn a_transcript_path_that_names_no_file_is_answered_alike() -> TestResult {
    let base = Base::new()?;
    let source = base.0.join("source.mp4");
    fs::write(&source, MEDIA)?;
    let missing = base.0.join("no-such-transcript.srt");

    let output = base.run(&[
        "ingest",
        path_text(&source)?,
        "--transcript",
        path_text(&missing)?,
    ])?;
    assert_the_storage_io_answer(&output, SOURCE_NOT_FOUND_REMEDIATION)
}

/// A folder named as the video: the platform that cannot open a folder as a
/// file keeps `STORAGE_IO` and says it is a folder; the platforms that open it
/// and find it is no regular file keep `INVALID_SOURCE`, which this test pins as
/// the documented answer it is.
#[test]
fn a_folder_named_as_the_source_keeps_the_answer_of_its_platform() -> TestResult {
    let base = Base::new()?;
    let folder = base.folder("a-folder")?;

    let output = base.run(&["ingest", path_text(&folder)?])?;
    if cfg!(windows) {
        assert_the_storage_io_answer(&output, SOURCE_IS_FOLDER_REMEDIATION)
    } else {
        let value = json(&output)?;
        assert_eq!(value["error"]["code"], "INVALID_SOURCE", "{value}");
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        Ok(())
    }
}

/// A file the account may not read. Unix only (a Windows account cannot be
/// denied a file it made without editing its access list; the classification of
/// the refusal is unit-tested for every platform), and an account that reads
/// anything, root, cannot be refused: the test says it checked nothing there.
#[cfg(unix)]
#[test]
fn a_file_that_may_not_be_read_keeps_its_code_and_says_so() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;

    use vsift_contract::SOURCE_ACCESS_DENIED_REMEDIATION;

    let base = Base::new()?;
    let locked = base.0.join("locked.mp4");
    fs::write(&locked, MEDIA)?;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))?;
    if fs::File::open(&locked).is_ok() {
        eprintln!(
            "SKIPPED: this account can read a file with no permissions; the test checks nothing here"
        );
        return Ok(());
    }

    let output = base.run(&["ingest", path_text(&locked)?])?;
    assert_the_storage_io_answer(&output, SOURCE_ACCESS_DENIED_REMEDIATION)
}

/// The person who reads the terminal gets the same words: the human form
/// prints the remediation after `Fix:`, on the error stream, as for every
/// other failure that has one.
#[test]
fn the_human_form_prints_the_same_remediation() -> TestResult {
    let base = Base::new()?;
    let user = base.0.join("user");
    let output = Command::cargo_bin("vsift")?
        .current_dir(&base.0)
        .env("LOCALAPPDATA", &user)
        .env("XDG_CONFIG_HOME", &user)
        .env("XDG_CACHE_HOME", &user)
        .env("HOME", &user)
        .arg("--session-root")
        .arg(base.0.join("sessions"))
        .args(["ingest", "no-such-video.mp4"])
        .output()?;
    assert_eq!(output.stdout, [] as [u8; 0]);
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stderr)?,
        format!(
            "Error: Storage or output I/O prevented completion. (STORAGE_IO)\nFix: {SOURCE_NOT_FOUND_REMEDIATION}\n"
        )
    );
    Ok(())
}
