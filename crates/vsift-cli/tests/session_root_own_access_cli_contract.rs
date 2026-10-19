//! CLI contract for a session root that has lost its user's own entry
//! (Windows, #443).
//!
//! Before ADR 0026 several commands that used a new session root at the same
//! moment could leave it with only `SYSTEM` and Administrators in its DACL;
//! every later command was then refused with a bare "session storage root is
//! unavailable". **The code stays** (`STORAGE_IO`, exit 7; changing a
//! published failure code is not additive within v1, known limit L-127) and
//! the refusal gains a remediation that says how to recover the folder.
//! `VSift` never gives the entry back itself.
#![cfg(windows)]

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Output,
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;
use vsift_contract::SESSION_ROOT_OWN_ACCESS_REMEDIATION;

type TestResult = Result<(), Box<dyn Error>>;

const PREFIX: &str = "vsift-root-own-access-cli-";
/// An ISO media header: enough for `ingest` to accept the file as a source.
const MEDIA: &[u8] = b"\0\0\0\x18ftypisomsession-root-own-access-contract";

/// A scratch folder holding an isolated per-user area, a source and a root.
struct Base(PathBuf);

impl Base {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!("{PREFIX}{}-{stamp}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    /// Runs `vsift --session-root <root> <arguments> --json` with an isolated
    /// per-user area.
    fn run(&self, root: &Path, arguments: &[&OsStr]) -> Result<Output, Box<dyn Error>> {
        let user = self.0.join("user");
        Ok(Command::cargo_bin("vsift")?
            .env("LOCALAPPDATA", &user)
            .arg("--session-root")
            .arg(root)
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

/// The current account as `DOMAIN\name`, the form the remediation tells a
/// person to type.
fn account() -> Result<String, Box<dyn Error>> {
    let domain = env::var("USERDOMAIN")?;
    let name = env::var("USERNAME")?;
    Ok(format!("{domain}\\{name}"))
}

/// Runs the system `icacls.exe` with explicit arguments (never a shell).
fn icacls(arguments: &[&OsStr]) -> TestResult {
    let system_root = env::var_os("SystemRoot").ok_or("SystemRoot is not set")?;
    let program = PathBuf::from(system_root)
        .join("System32")
        .join("icacls.exe");
    let output = std::process::Command::new(program)
        .args(arguments)
        .output()?;
    if !output.status.success() {
        return Err(format!("icacls {arguments:?} failed: {output:?}").into());
    }
    Ok(())
}

/// Gives the account its entry back when dropped, exactly as the remediation
/// says, so the fixture can be removed even when an assertion fails.
struct GiveAccessBack(PathBuf);

impl Drop for GiveAccessBack {
    fn drop(&mut self) {
        if let Ok(account) = account() {
            let grant = format!("{account}:(OI)(CI)F");
            let _ = icacls(&[self.0.as_os_str(), "/grant".as_ref(), grant.as_ref()]);
        }
    }
}

/// Whether `icacls` lists an entry for `account` on `path`.
fn holds_entry_for(path: &Path, account: &str) -> Result<bool, Box<dyn Error>> {
    let system_root = env::var_os("SystemRoot").ok_or("SystemRoot is not set")?;
    let program = PathBuf::from(system_root)
        .join("System32")
        .join("icacls.exe");
    let output = std::process::Command::new(program).arg(path).output()?;
    if !output.status.success() {
        return Err(format!("icacls could not list {}: {output:?}", path.display()).into());
    }
    let listing = String::from_utf8_lossy(&output.stdout).to_lowercase();
    Ok(listing.contains(&format!("{}:", account.to_lowercase())))
}

fn answer(output: &Output) -> Result<Value, Box<dyn Error>> {
    assert!(
        output.stderr.is_empty(),
        "a JSON command wrote to standard error: {output:?}"
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// A root `VSift` created, whose DACL then loses the user's own entry, is
/// refused with the code it always had and the remediation that says how to
/// recover it; the folder is left as it was; and the remediation's own command
/// makes it usable again.
#[test]
fn a_root_without_the_users_entry_is_refused_with_its_recovery_and_left_alone() -> TestResult {
    let base = Base::new()?;
    let source = base.0.join("recording.mp4");
    fs::write(&source, MEDIA)?;
    let root = base.0.join("sessions");
    let created = base.run(&root, &["ingest".as_ref(), source.as_os_str()])?;
    assert_eq!(created.status.code(), Some(0), "{}", answer(&created)?);

    let restore = GiveAccessBack(root.clone());
    let account = account()?;
    icacls(&[root.as_os_str(), "/remove:g".as_ref(), account.as_ref()])?;
    assert!(
        !holds_entry_for(&root, &account)?,
        "the entry was not removed"
    );
    // An account that runs elevated still opens the root through the
    // Administrators entry (a hosted runner does); VSift refuses it either way,
    // from the DACL read without opening or from its validation after opening.
    let opened_before = fs::read_dir(&root).is_ok();

    let refused = base.run(&root, &["session".as_ref(), "list".as_ref()])?;
    let value = answer(&refused)?;
    assert_eq!(refused.status.code(), Some(7), "{value}");
    assert_eq!(value["error"]["code"], "STORAGE_IO", "{value}");
    assert_eq!(
        value["error"]["remediation"][0]["summary"], SESSION_ROOT_OWN_ACCESS_REMEDIATION,
        "{value}"
    );
    assert!(
        !value.to_string().contains(&*base.0.to_string_lossy()),
        "the answer names a path: {value}"
    );
    assert!(
        !holds_entry_for(&root, &account)?,
        "VSift gave the entry back itself"
    );
    assert_eq!(fs::read_dir(&root).is_ok(), opened_before);

    // The remediation's command: the account's entry back, then the root
    // works again.
    drop(restore);
    let listed = base.run(&root, &["session".as_ref(), "list".as_ref()])?;
    assert_eq!(listed.status.code(), Some(0), "{}", answer(&listed)?);
    Ok(())
}
