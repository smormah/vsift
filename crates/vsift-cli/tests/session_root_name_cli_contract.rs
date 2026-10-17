//! CLI contract for the session root's own folder name (#232).
//!
//! On Linux CI, a session root whose own final name held an escape sequence,
//! a bell, a line break and a C1 control was refused as an unexplained
//! unavailable root (`STORAGE_IO`, no remediation), while the same characters
//! in its parent folder worked. The cause was not the controls: the name also
//! held two colons (`https:` and `Forged:`), and a colon is refused in a root
//! name on every platform on purpose (an alternate data stream on Windows).
//! **The code stays** (`STORAGE_IO`, exit 7; changing a published failure code
//! is not additive within v1, known limit L-127) and the refusal gains a
//! remediation that states the rule. Control characters work in a root name
//! where the operating system allows them (Linux and macOS); Windows refuses
//! them in any name, and `VSift` now says so up front.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;
use vsift_contract::SESSION_ROOT_NAME_REMEDIATION;

type TestResult = Result<(), Box<dyn Error>>;

const PREFIX: &str = "vsift-root-name-cli-";
/// An ISO media header: enough for `ingest` to accept the file as a source.
const MEDIA: &[u8] = b"\0\0\0\x18ftypisomsession-root-name-contract";
static NEXT_BASE: AtomicU64 = AtomicU64::new(0);

/// A scratch folder holding an isolated per-user area, a source and roots.
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

    /// Runs `vsift --session-root <root> <arguments> --json` with an isolated
    /// per-user area.
    fn run(&self, root: &Path, arguments: &[&OsStr]) -> Result<Output, Box<dyn Error>> {
        let user = self.0.join("user");
        Ok(Command::cargo_bin("vsift")?
            .env("LOCALAPPDATA", &user)
            .env("XDG_CONFIG_HOME", &user)
            .env("XDG_CACHE_HOME", &user)
            .env("HOME", &user)
            .arg("--session-root")
            .arg(root)
            .args(arguments)
            .arg("--json")
            .output()?)
    }

    fn source(&self) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.0.join("recording.mp4");
        fs::write(&path, MEDIA)?;
        Ok(path)
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

fn answer(output: &Output) -> Result<Value, Box<dyn Error>> {
    assert!(
        output.stderr.is_empty(),
        "a JSON command wrote to standard error: {output:?}"
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// A root name with a colon is refused before anything is created, with the
/// code it always had and the remediation that states the rule. So is a name
/// that ends with a dot, and a reserved device name, on every platform.
#[test]
fn a_root_name_vsift_does_not_use_keeps_its_code_and_states_the_rule() -> TestResult {
    let base = Base::new()?;
    let source = base.source()?;
    for name in ["sessions:stream", "sessions.", "NUL"] {
        let root = base.0.join(name);
        let output = base.run(&root, &["ingest".as_ref(), source.as_os_str()])?;
        let value = answer(&output)?;
        assert_eq!(output.status.code(), Some(7), "{name}: {value}");
        assert_eq!(value["error"]["code"], "STORAGE_IO", "{name}: {value}");
        assert_eq!(
            value["error"]["remediation"][0]["summary"], SESSION_ROOT_NAME_REMEDIATION,
            "{name}: {value}"
        );
        assert!(
            fs::read_dir(&base.0)?
                .filter_map(Result::ok)
                .all(|entry| entry.file_name() == "recording.mp4"),
            "{name}: something was created"
        );
    }
    Ok(())
}

/// On Linux and macOS a root whose own name holds an escape sequence, a bell,
/// a line break and a C1 control (and no colon) is created and used like any
/// other: the controls were never what refused the SEC-T02 name.
#[cfg(unix)]
#[test]
fn a_root_name_with_control_characters_works_where_the_system_allows_them() -> TestResult {
    let base = Base::new()?;
    let source = base.source()?;
    let root = base.0.join(
        "roo\u{202e}ts\u{200b}\u{1b}]8;;https//example.invalid\u{7}x\u{1b}[31m\nForged line\u{85}",
    );
    let output = base.run(&root, &["ingest".as_ref(), source.as_os_str()])?;
    let value = answer(&output)?;
    assert_eq!(output.status.code(), Some(0), "{value}");
    assert_eq!(value["status"], "complete", "{value}");
    let session = value["data"]["session_id"]
        .as_str()
        .ok_or("no session id")?
        .to_owned();

    let output = base.run(&root, &["session".as_ref(), "list".as_ref()])?;
    let value = answer(&output)?;
    assert_eq!(output.status.code(), Some(0), "{value}");
    assert!(value.to_string().contains(&session), "{value}");
    Ok(())
}
