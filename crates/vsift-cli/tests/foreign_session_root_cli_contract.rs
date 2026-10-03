//! CLI contract for a `--session-root` that names a folder `VSift` did not
//! create (#261).
//!
//! `VSift` never adopts such a folder: its ownership marker is what makes a
//! folder `VSift`'s, and the creator writes it last. The refusal was always
//! `INTEGRITY_FAILURE` (exit 7), which says stored data is damaged and nothing
//! else, so a person who had made the folder themselves could not tell what
//! went wrong. The failure code is kept (v1 is additive only); the remediation
//! now says the folder holds no `VSift` marker, was not created by `VSift` and is
//! left untouched, and what to do.
//!
//! The folders are made private to the current user first, as a person's own
//! `mkdir` in their profile is, so that the ownership check (not the privacy
//! check, which has its own remediation) is the one that refuses them.

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
use vsift_contract::UNOWNED_SESSION_ROOT_REMEDIATION;

type TestResult = Result<(), Box<dyn Error>>;

const PREFIX: &str = "vsift-foreign-root-cli-";
const SOURCE_BYTES: &[u8] = b"\0\0\0\x18ftypisomforeign-root-source";

static NEXT_BASE: AtomicU64 = AtomicU64::new(0);

/// A scratch base holding an isolated per-user area and the folders under test.
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

    fn path(&self, child: &str) -> PathBuf {
        self.0.join(child)
    }

    /// A folder this test made, private to the current user.
    fn their_folder(&self, name: &str) -> Result<PathBuf, Box<dyn Error>> {
        let folder = self.path(name);
        fs::create_dir(&folder)?;
        make_private(&folder)?;
        Ok(folder)
    }

    fn vsift(&self, root: &Path, arguments: &[&str]) -> Result<Command, Box<dyn Error>> {
        let user = self.path("user");
        let mut command = Command::cargo_bin("vsift")?;
        command
            .env("LOCALAPPDATA", &user)
            .env("XDG_CONFIG_HOME", &user)
            .env("XDG_CACHE_HOME", &user)
            .env("HOME", &user)
            .arg("--session-root")
            .arg(root)
            .args(arguments);
        Ok(command)
    }

    fn json(&self, root: &Path, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
        Ok(self.vsift(root, arguments)?.arg("--json").output()?)
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

#[cfg(unix)]
fn make_private(folder: &Path) -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(folder, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// Removes inherited access and grants the three principals `VSift` trusts, with
/// the system `icacls.exe` and explicit arguments (no shell), so the folder
/// passes the privacy check on any runner whatever its profile folder grants.
#[cfg(windows)]
fn make_private(folder: &Path) -> TestResult {
    let system_root = env::var_os("SystemRoot").ok_or("SystemRoot is not set")?;
    let user = env::var("USERNAME")?;
    let status = std::process::Command::new(PathBuf::from(system_root).join("System32/icacls.exe"))
        .arg(folder)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg("*S-1-5-18:(OI)(CI)F")
        .arg("/grant:r")
        .arg("*S-1-5-32-544:(OI)(CI)F")
        .arg("/grant:r")
        .arg(format!("{user}:(OI)(CI)F"))
        .arg("/Q")
        .output()?
        .status;
    assert!(status.success(), "icacls could not make the folder private");
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

/// Asserts the typed refusal: exit 7, `INTEGRITY_FAILURE`, the fixed
/// remediation, schema-valid JSON on stdout, nothing on stderr and no path.
fn assert_refused_as_unowned(output: &Output, command: &str, folder: &Path) -> TestResult {
    let value: Value = serde_json::from_slice(&output.stdout)?;
    schema_valid(&value)?;
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    assert_eq!(value["command"], command);
    assert_eq!(value["status"], "failed");
    assert_eq!(value["error"]["code"], "INTEGRITY_FAILURE", "{value}");
    assert_eq!(value["error"]["retryable"], false);
    let remediation = value["error"]["remediation"]
        .as_array()
        .ok_or("remediation is not an array")?;
    assert_eq!(remediation.len(), 1, "{value}");
    assert_eq!(remediation[0]["summary"], UNOWNED_SESSION_ROOT_REMEDIATION);
    assert_eq!(remediation[0]["required_authority"], "none");
    assert_eq!(remediation[0]["command"], Value::Null);
    let unique_name = folder
        .parent()
        .and_then(Path::file_name)
        .and_then(std::ffi::OsStr::to_str)
        .ok_or("fixture name is not UTF-8")?;
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains(unique_name),
        "the output carried a user path"
    );
    assert!(output.stderr.is_empty(), "{output:?}");
    Ok(())
}

fn names(folder: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut names = fs::read_dir(folder)?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    names.sort();
    Ok(names)
}

#[test]
fn a_folder_vsift_did_not_create_is_refused_with_a_remediation_and_left_untouched() -> TestResult {
    let base = Base::new()?;
    let folder = base.their_folder("their folder")?;
    // Content means no creator can still be finishing it, so the refusal is
    // immediate.
    fs::write(folder.join("notes.txt"), b"my own notes")?;
    let source = base.path("original media.mp4");
    fs::write(&source, SOURCE_BYTES)?;
    let source = source.to_str().ok_or("source path is not UTF-8")?;

    assert_refused_as_unowned(
        &base.json(&folder, &["session", "list"])?,
        "session.list",
        &folder,
    )?;
    assert_refused_as_unowned(&base.json(&folder, &["ingest", source])?, "ingest", &folder)?;
    assert_refused_as_unowned(
        &base.json(
            &folder,
            &[
                "session",
                "init-workspace",
                "--durability",
                "ephemeral",
                "--admission-slots",
                "3",
            ],
        )?,
        "session.init-workspace",
        &folder,
    )?;

    // The same answer as the frozen example an agent can read.
    let example: Value = serde_json::from_slice(&fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schemas/v1/examples/session-root-unowned.json"),
    )?)?;
    let emitted: Value = serde_json::from_slice(&base.json(&folder, &["session", "list"])?.stdout)?;
    assert!(
        emitted == example,
        "the result differs from the frozen example"
    );

    // The words a person sees without --json.
    let human = base.vsift(&folder, &["session", "list"])?.output()?;
    assert_eq!(human.status.code(), Some(7));
    let diagnostics = String::from_utf8_lossy(&human.stderr);
    assert!(
        diagnostics.contains("The VSift session_root folder holds no VSift ownership marker"),
        "{diagnostics}"
    );
    assert!(
        diagnostics.contains("does not exist yet"),
        "the fix is stated: {diagnostics}"
    );
    assert!(human.stdout.is_empty());

    // Nothing was created in, changed in or removed from the person's folder.
    assert_eq!(names(&folder)?, ["notes.txt"]);
    assert_eq!(fs::read(folder.join("notes.txt"))?, b"my own notes");
    Ok(())
}

/// The case of the issue: an empty folder made with `mkdir`. A just-made empty
/// folder could be a concurrent creator's first step, so the refusal waits out
/// the documented five seconds first.
#[test]
fn an_empty_folder_the_user_made_is_refused_the_same_way() -> TestResult {
    let base = Base::new()?;
    let folder = base.their_folder("mkdir made")?;
    assert_refused_as_unowned(
        &base.json(&folder, &["session", "list"])?,
        "session.list",
        &folder,
    )?;
    assert!(names(&folder)?.is_empty(), "the folder was written to");
    Ok(())
}

/// The distinction the fix draws: a marker that is there and wrong is damage
/// and keeps the bare `INTEGRITY_FAILURE`; only a folder with no marker at all
/// is described as one `VSift` did not create.
#[test]
fn a_vsift_root_with_a_wrong_marker_is_not_described_as_foreign() -> TestResult {
    let base = Base::new()?;
    let root = base.path("vsift root");
    let source = base.path("original media.mp4");
    fs::write(&source, SOURCE_BYTES)?;
    let source = source.to_str().ok_or("source path is not UTF-8")?;
    let opened = base.json(&root, &["ingest", source])?;
    assert!(opened.status.success(), "{opened:?}");

    let marker = root.join("ownership.json");
    let original = fs::read(&marker)?;
    for wrong in [b"not json".as_slice(), b"".as_slice()] {
        fs::remove_file(&marker)?;
        fs::write(&marker, wrong)?;
        let listed = base.json(&root, &["session", "list"])?;
        let value: Value = serde_json::from_slice(&listed.stdout)?;
        assert_eq!(listed.status.code(), Some(7), "{listed:?}");
        assert_eq!(value["error"]["code"], "INTEGRITY_FAILURE");
        assert_eq!(
            value["error"]["remediation"].as_array().map(Vec::len),
            Some(0),
            "a damaged marker is not explained as a foreign folder: {value}"
        );
    }

    // A root whose marker is gone is indistinguishable from a folder VSift did
    // not create, and is described that way; its sessions are untouched.
    fs::remove_file(&marker)?;
    assert_refused_as_unowned(
        &base.json(&root, &["session", "list"])?,
        "session.list",
        &root,
    )?;
    assert!(root.join("sessions").is_dir());

    // With the marker back the root works again.
    fs::write(&marker, original)?;
    let listed = base.json(&root, &["session", "list"])?;
    assert!(listed.status.success(), "{listed:?}");
    Ok(())
}
