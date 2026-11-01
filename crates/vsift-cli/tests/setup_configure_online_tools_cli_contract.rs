//! Public CLI contract for registering a yt-dlp and its JavaScript runtime
//! (`setup configure yt-dlp|js-runtime`, ADR 0028 section 4.6).
//!
//! Registration only records a canonical path; nothing is run and no network
//! is reached, so the files here are plain stand-ins.

use std::{
    env,
    error::Error,
    fs,
    path::PathBuf,
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-online-tools-cli-";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

/// A private temporary folder, removed when it drops.
struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self, child: &str) -> PathBuf {
        self.0.join(child)
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn vsift(root: &OwnedRoot, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    let base = root.path("user");
    let empty = root.path("empty-path");
    fs::create_dir_all(&empty)?;
    Ok(Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .env("PATH", &empty)
        .args(arguments)
        .output()?)
}

fn json(output: &Output) -> Result<Value, Box<dyn Error>> {
    assert!(output.stderr.is_empty(), "a JSON command wrote to stderr");
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn configure(root: &OwnedRoot, name: &str, executable: &PathBuf) -> Result<Output, Box<dyn Error>> {
    let executable = executable.to_str().ok_or("path is not UTF-8")?;
    vsift(
        root,
        &[
            "setup",
            "configure",
            name,
            "--executable",
            executable,
            "--json",
        ],
    )
}

/// Each tool is recorded under the identifier the JSON results use. yt-dlp
/// has a hidden `yt_dlp` spelling, because `--include online_links` and the
/// results use the underscore; the runtime has none (an alias on a stable
/// command line stays for good). The response says which tool and that
/// nothing was run.
#[test]
fn a_ytdlp_and_its_runtime_register_under_their_json_identifiers() -> TestResult {
    let root = OwnedRoot::new()?;
    let yt_dlp = root.path("yt-dlp");
    let deno = root.path("deno");
    fs::write(&yt_dlp, b"not run: registration only records the path")?;
    fs::write(&deno, b"not run either")?;
    for (spelling, identifier, file) in [
        ("yt-dlp", "yt_dlp", &yt_dlp),
        ("yt_dlp", "yt_dlp", &yt_dlp),
        ("js-runtime", "js_runtime", &deno),
    ] {
        let value = json(&configure(&root, spelling, file)?)?;
        assert_eq!(value["status"], "complete", "{spelling}: {value}");
        assert_eq!(value["command"], "setup.configure");
        assert_eq!(value["data"]["dependency"], identifier);
        assert_eq!(value["data"]["source"], "configured_user_path");
        assert_eq!(value["data"]["validation"], "canonical_file_only");
    }
    let refused = configure(&root, "js_runtime", &deno)?;
    assert_eq!(refused.status.code(), Some(2), "js_runtime has no alias");
    Ok(())
}

/// A relative path, a missing file and a directory are refused with the same
/// typed failure as the other tools, and nothing is recorded.
#[test]
fn an_executable_that_is_not_an_absolute_regular_file_is_refused() -> TestResult {
    let root = OwnedRoot::new()?;
    let missing = root.path("missing");
    let output = configure(&root, "yt-dlp", &missing)?;
    assert!(!output.status.success());
    let value = json(&output)?;
    assert_eq!(value["status"], "failed", "{value}");
    assert!(value["error"]["code"].is_string(), "{value}");
    let relative = vsift(
        &root,
        &[
            "setup",
            "configure",
            "js-runtime",
            "--executable",
            "deno",
            "--json",
        ],
    )?;
    assert!(!relative.status.success());
    assert_eq!(json(&relative)?["status"], "failed");
    assert!(
        !root
            .path("user")
            .join("vsift")
            .join("yt-dlp-v1.json")
            .exists()
    );
    Ok(())
}

/// The `online_links` row of `setup check`.
fn online_links(root: &OwnedRoot) -> Result<Value, Box<dyn Error>> {
    let output = vsift(
        root,
        &["setup", "check", "--json", "--timeout-seconds", "5"],
    )?;
    let value: Value = serde_json::from_slice(&output.stdout)?;
    value["optional_components"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["name"] == "online_links"))
        .cloned()
        .ok_or_else(|| "no online_links row".into())
}

/// A runtime without a yt-dlp of the user's is not used (the runtime follows
/// yt-dlp's source), so the check says so instead of reporting it in use. The
/// stand-in is this build's own binary, whose version line does not begin
/// `deno `: a runtime that is not Deno is not healthy.
#[test]
fn a_runtime_without_a_ytdlp_is_reported_unused_and_a_non_deno_is_not_healthy() -> TestResult {
    let root = OwnedRoot::new()?;
    let not_deno = PathBuf::from(Command::cargo_bin("vsift")?.get_program());
    let value = json(&configure(&root, "js-runtime", &not_deno)?)?;
    assert_eq!(
        value["status"], "complete",
        "configure runs nothing: {value}"
    );
    let row = online_links(&root)?;
    let step = row["next_step"].as_str().ok_or("no next_step")?;
    assert!(step.contains("is not used"), "{row}");
    assert!(step.contains("none is"), "{row}");
    assert!(step.contains("Only Deno is accepted"), "{row}");
    assert_ne!(row["status"], "installed", "{row}");
    assert_eq!(row["components"], serde_json::json!([]), "{row}");
    Ok(())
}

/// A registered yt-dlp that fails the fixed version request makes the group
/// not installed, with the reason in words and no add command, and nothing it
/// printed is carried. The stand-in rejects the arguments and exits non-zero.
#[test]
fn a_ytdlp_that_cannot_answer_the_version_request_is_reported_unusable() -> TestResult {
    let root = OwnedRoot::new()?;
    let stand_in = PathBuf::from(Command::cargo_bin("vsift")?.get_program());
    json(&configure(&root, "yt-dlp", &stand_in)?)?;
    let row = online_links(&root)?;
    assert_eq!(row["status"], "not_installed", "{row}");
    assert_eq!(row["add_command"], Value::Null, "{row}");
    assert_eq!(row["components"], serde_json::json!([]), "{row}");
    let step = row["next_step"].as_str().ok_or("no next_step")?;
    assert!(step.contains("could not be used"), "{row}");
    assert!(step.contains("setup configure yt-dlp"), "{row}");
    Ok(())
}
