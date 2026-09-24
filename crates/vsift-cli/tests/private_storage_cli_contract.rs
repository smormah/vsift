//! CLI contract for per-user folders under a parent that passes access for
//! other accounts to new children (the maintainer's `%LOCALAPPDATA%`).
//!
//! Folders `VSift` creates there are private and protected, so `setup
//! configure`, `ingest` and `session retain` succeed. An existing folder other
//! accounts can access is refused with `STORAGE_IO` and a fixed-prose
//! remediation naming the folder kind, and it is left untouched. The fixture
//! parent grants `BUILTIN\Users` inheritable read access with the system
//! `icacls.exe`; results are read back as SDDL, independent of the display
//! language.

#![cfg(windows)]

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

type TestResult = Result<(), Box<dyn Error>>;

const PREFIX: &str = "vsift-cli-private-storage-";

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

/// A temporary `LOCALAPPDATA` that passes `BUILTIN\Users` read access to new children.
struct HostileLocalAppData(PathBuf);

impl HostileLocalAppData {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            env::temp_dir().join(format!("{PREFIX}{}-{stamp}-{sequence}", std::process::id()));
        fs::create_dir(&path)?;
        let base = Self(path);
        icacls(&[
            base.0.as_os_str(),
            "/grant".as_ref(),
            "*S-1-5-32-545:(OI)(CI)(RX)".as_ref(),
            "/Q".as_ref(),
        ])?;
        Ok(base)
    }

    fn join(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }

    fn vsift(&self) -> Result<Command, Box<dyn Error>> {
        let mut command = Command::cargo_bin("vsift")?;
        command.env("LOCALAPPDATA", &self.0);
        Ok(command)
    }
}

impl Drop for HostileLocalAppData {
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

/// Runs the system `icacls.exe` with explicit arguments and no shell.
fn icacls(arguments: &[&OsStr]) -> TestResult {
    let system_root = env::var_os("SystemRoot").ok_or("SystemRoot is not set")?;
    let status = std::process::Command::new(PathBuf::from(system_root).join("System32/icacls.exe"))
        .args(arguments)
        .output()?
        .status;
    if !status.success() {
        return Err("icacls failed".into());
    }
    Ok(())
}

/// The directory's DACL in SDDL form, for example `D:PAI(A;OICI;FA;;;SY)...`.
fn dacl_sddl(directory: &Path, base: &HostileLocalAppData) -> Result<String, Box<dyn Error>> {
    let saved = base.join("saved-acl.txt");
    icacls(&[
        directory.as_os_str(),
        "/save".as_ref(),
        saved.as_os_str(),
        "/Q".as_ref(),
    ])?;
    let bytes = fs::read(&saved)?;
    fs::remove_file(&saved)?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let text = String::from_utf16(&units)?;
    let sddl = text
        .lines()
        .find(|line| line.starts_with("D:"))
        .ok_or("icacls saved no DACL")?;
    Ok(sddl.to_owned())
}

fn assert_private_and_protected(directory: &Path, base: &HostileLocalAppData) -> TestResult {
    let sddl = dacl_sddl(directory, base)?;
    assert!(sddl.starts_with("D:P"), "the DACL is not protected");
    assert!(!sddl.contains("ID;"), "an inherited entry remained");
    assert!(!sddl.contains(";BU)"), "BUILTIN\\Users kept access");
    Ok(())
}

fn schema_valid(value: &Value) -> TestResult {
    let schema: Value = serde_json::from_slice(&fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schemas/v1/operation-response.schema.json"),
    )?)?;
    jsonschema::validator_for(&schema)?
        .validate(value)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn succeeded(output: &Output) -> Result<Value, Box<dyn Error>> {
    let value: Value = serde_json::from_slice(&output.stdout)?;
    assert!(output.status.success(), "the command failed");
    assert_eq!(value["status"], "complete");
    schema_valid(&value)?;
    Ok(value)
}

/// Asserts the typed refusal of an existing folder other accounts can access.
fn assert_refused_as_not_private(
    output: &Output,
    command: &str,
    folder: &str,
    base: &HostileLocalAppData,
) -> TestResult {
    let value: Value = serde_json::from_slice(&output.stdout)?;
    schema_valid(&value)?;
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(value["command"], command);
    assert_eq!(value["status"], "failed");
    assert_eq!(value["error"]["code"], "STORAGE_IO");
    assert_eq!(value["error"]["retryable"], false);
    let remediation = value["error"]["remediation"]
        .as_array()
        .ok_or("remediation is not an array")?;
    assert_eq!(remediation.len(), 1);
    assert_eq!(remediation[0]["required_authority"], "none");
    assert_eq!(remediation[0]["command"], Value::Null);
    let summary = remediation[0]["summary"]
        .as_str()
        .ok_or("summary is not a string")?;
    let expected_start = format!("The VSift {folder} folder is accessible to other accounts,");
    assert!(summary.starts_with(&expected_start), "unexpected summary");
    // The fixture directory's unique name would appear in any form of its path,
    // escaped or not.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let unique_name = base
        .0
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or("fixture name is not UTF-8")?;
    assert!(
        !stdout.contains(unique_name),
        "the output carried a user path"
    );
    assert!(output.stderr.is_empty());
    Ok(())
}

#[test]
fn setup_configure_and_check_succeed_under_a_parent_that_passes_access_on() -> TestResult {
    let base = HostileLocalAppData::new()?;
    let binary = Command::cargo_bin("vsift")?.get_program().to_os_string();
    let configured = base
        .vsift()?
        .args(["setup", "configure", "ffmpeg", "--executable"])
        .arg(&binary)
        .arg("--json")
        .output()?;
    let value = succeeded(&configured)?;
    assert_eq!(value["command"], "setup.configure");
    assert_private_and_protected(&base.join("vsift"), &base)?;

    let model = base.join("model.bin");
    fs::write(&model, b"fixture only")?;
    let registered = base
        .vsift()?
        .args(["setup", "configure-model", "--file"])
        .arg(&model)
        .arg("--json")
        .output()?;
    succeeded(&registered)?;

    let checked = base
        .vsift()?
        .args(["setup", "check", "--json", "--timeout-seconds", "1"])
        .env("PATH", "")
        .output()?;
    let value: Value = serde_json::from_slice(&checked.stdout)?;
    assert_eq!(value["dependencies"][0]["lookup"], "configured_user_path");
    Ok(())
}

#[test]
fn ingest_and_retain_create_private_session_and_bundle_folders() -> TestResult {
    let base = HostileLocalAppData::new()?;
    let source = base.join("original media.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomsource-content")?;
    let opened = base
        .vsift()?
        .arg("ingest")
        .arg(&source)
        .arg("--json")
        .output()?;
    let opened = succeeded(&opened)?;
    let id = opened["data"]["session_id"]
        .as_str()
        .ok_or("missing session id")?;
    assert_private_and_protected(&base.join("VSift-sessions"), &base)?;

    let bundle = base.join("retained");
    let retained = base
        .vsift()?
        .args(["session", "retain", id, "--output"])
        .arg(&bundle)
        .arg("--json")
        .output()?;
    succeeded(&retained)?;
    assert_private_and_protected(&bundle, &base)?;
    Ok(())
}

#[test]
fn an_existing_configuration_folder_other_accounts_can_read_is_refused_untouched() -> TestResult {
    let base = HostileLocalAppData::new()?;
    let folder = base.join("vsift");
    fs::create_dir(&folder)?;
    // Content, as an older folder would hold, means no concurrent creator can
    // still be finishing it, so the refusal is immediate.
    fs::write(folder.join("notes.txt"), b"user content")?;
    let before = dacl_sddl(&folder, &base)?;
    assert!(
        before.contains(";BU)"),
        "the fixture did not inherit the entry"
    );
    let binary = Command::cargo_bin("vsift")?.get_program().to_os_string();

    let configured = base
        .vsift()?
        .args(["setup", "configure", "ffmpeg", "--executable"])
        .arg(&binary)
        .arg("--json")
        .output()?;
    assert_refused_as_not_private(&configured, "setup.configure", "user_configuration", &base)?;
    let example: Value = serde_json::from_slice(&fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schemas/v1/examples/storage-not-private.json"),
    )?)?;
    let emitted: Value = serde_json::from_slice(&configured.stdout)?;
    assert!(
        emitted == example,
        "the result differs from the frozen example"
    );

    let checked = base
        .vsift()?
        .args(["setup", "check", "--json"])
        .env("PATH", "")
        .output()?;
    assert_refused_as_not_private(&checked, "setup.check", "user_configuration", &base)?;

    let human = base
        .vsift()?
        .args(["setup", "configure", "ffmpeg", "--executable"])
        .arg(&binary)
        .output()?;
    assert_eq!(human.status.code(), Some(7));
    let diagnostics = String::from_utf8_lossy(&human.stderr);
    assert!(diagnostics.contains("The VSift user_configuration folder is accessible"));

    assert_eq!(dacl_sddl(&folder, &base)?, before);
    assert_eq!(fs::read_dir(&folder)?.count(), 1);
    Ok(())
}

#[test]
fn an_existing_session_folder_other_accounts_can_read_is_refused_untouched() -> TestResult {
    let base = HostileLocalAppData::new()?;
    let folder = base.join("VSift-sessions");
    fs::create_dir(&folder)?;
    fs::write(folder.join("notes.txt"), b"user content")?;
    let before = dacl_sddl(&folder, &base)?;
    let listed = base.vsift()?.args(["session", "list", "--json"]).output()?;
    assert_refused_as_not_private(&listed, "session.list", "session_root", &base)?;
    assert_eq!(dacl_sddl(&folder, &base)?, before);
    assert_eq!(fs::read_dir(&folder)?.count(), 1);
    Ok(())
}
