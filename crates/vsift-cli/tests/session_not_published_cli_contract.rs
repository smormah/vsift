//! CLI contract for a session id that names no published session (#277).
//!
//! A registration whose opening never finished (an `ingest` killed while it
//! opened, or, found by the P14 load campaign, a failed open that left its
//! registration behind) is listed as `initializing`, and `session status` of it
//! answered `STORAGE_IO` with no remediation: the answer of a disk that failed,
//! for a session the same tool had just listed. It is now `INVALID_ARGUMENT`
//! with a remediation that says the storage is fine and what the entry is; an id
//! that never existed gets the same answer. A failed `ingest` itself no longer
//! leaves a registration at all.

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
use vsift_contract::SESSION_NOT_PUBLISHED_REMEDIATION;
use vsift_domain::{OperationId, SessionId};
use vsift_infrastructure::FilesystemSessionStore;

type TestResult = Result<(), Box<dyn Error>>;

const PREFIX: &str = "vsift-not-published-cli-";
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

/// Registers a session as an opener killed before it published would leave it.
fn leave_a_registration(base: &Base) -> Result<String, Box<dyn Error>> {
    let store = FilesystemSessionStore::provision_default(base.0.join("sessions"))?;
    let session = SessionId::parse("ses_0123456789abcdef0123456789abcdef")?;
    let opener = OperationId::parse("op_0123456789abcdef")?;
    drop(store.register_session(&session, &opener, 1_800_000_000)?);
    Ok(session.as_str().to_owned())
}

#[test]
fn a_session_that_never_finished_opening_is_not_a_disk_failure() -> TestResult {
    let base = Base::new()?;
    let initializing = leave_a_registration(&base)?;

    let listed = json(&base.run(&["session", "list"])?)?;
    let items = listed["data"]["items"]
        .as_array()
        .ok_or("session list holds no items")?;
    let [only] = items.as_slice() else {
        return Err(format!("expected one registration: {listed}").into());
    };
    assert_eq!(only["state"], "initializing");
    assert_eq!(only["session_id"], initializing.as_str());

    for id in [
        initializing.as_str(),
        "ses_00000000000000000000000000000001",
    ] {
        for command in ["status", "renew", "close"] {
            let output = base.run(&["session", command, id])?;
            let value = json(&output)?;
            assert_eq!(
                value["error"]["code"], "INVALID_ARGUMENT",
                "{command} {id}: {value}"
            );
            assert_eq!(
                value["error"]["remediation"][0]["summary"], SESSION_NOT_PUBLISHED_REMEDIATION,
                "{command} {id}: {value}"
            );
            assert_eq!(output.status.code(), Some(2), "{command} {id}: {output:?}");
        }
    }
    Ok(())
}

/// The other half of #277 through the binary: a source that is not media is
/// refused after its session was registered, and nothing is left listed.
#[test]
fn a_failed_ingest_leaves_no_session_to_list() -> TestResult {
    let base = Base::new()?;
    let not_media = base.0.join("not-media.mp4");
    fs::write(&not_media, b"plain text, not a video")?;

    let refused = base.run(&["ingest", path_text(&not_media)?])?;
    assert_eq!(json(&refused)?["error"]["code"], "INVALID_SOURCE");

    let listed = json(&base.run(&["session", "list"])?)?;
    assert_eq!(
        listed["data"]["items"].as_array().map(Vec::len),
        Some(0),
        "{listed}"
    );
    Ok(())
}
