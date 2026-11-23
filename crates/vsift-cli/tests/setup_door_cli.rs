//! The setup door, plain `vsift setup` (ADR 0036), through the compiled
//! binary, where nobody is at a terminal.
//!
//! Every run has an empty `PATH`, a fresh per-user base and standard input
//! that is not a terminal, so whatever the machine has installed, `VSift`
//! finds no tool, and a question or a wait would fail the test by its
//! deadline. What must hold: without a terminal the door reports where the
//! setup stands, read-only, and never asks or installs; with `--json` or
//! `--events` it refuses with a typed failure an assistant can read, and
//! nothing runs. The unattended form (`--unattended --install …`) asks
//! nothing either; its runs here never reach the network, which a debug
//! build refuses unless a variable this test removes allows it.

use std::{
    env,
    error::Error,
    fs, io,
    path::PathBuf,
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-setup-door-cli-";

/// Long enough for the tool checks to find nothing; a question waiting for
/// an answer would run into it.
const CLI_DEADLINE: Duration = Duration::from_secs(120);

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

/// A temporary folder this test made and alone removes.
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
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// Runs `vsift` with `arguments`, isolated, with standard input a closed
/// pipe.
fn vsift(root: &OwnedRoot, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    let base = root.0.join("user");
    Ok(Command::cargo_bin("vsift")?
        .args(arguments)
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("XDG_DATA_HOME", base.join("data"))
        .env("XDG_CACHE_HOME", base.join("cache"))
        .env("HOME", &base)
        .env("PATH", "")
        .env_remove("VSIFT_DEV_PUBLISHER_NETWORK")
        .write_stdin("y\n")
        .timeout(CLI_DEADLINE)
        .output()?)
}

/// The identifier base of the published schemas.
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";

fn schema_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/v1")
        .join(name)
}

/// Resolves a schema's sibling `$ref`s (`setup.schema.json` names
/// `setup-install.schema.json`) to the local copies, so validation never
/// reaches the network.
struct LocalSchemas;

impl Retrieve for LocalSchemas {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let name = uri
            .as_str()
            .strip_prefix(SCHEMA_BASE)
            .filter(|name| !name.contains(['/', '\\']))
            .ok_or_else(|| format!("unpublished schema reference: {uri}"))?;
        Ok(serde_json::from_str(&fs::read_to_string(schema_file(
            name,
        ))?)?)
    }
}

fn validate(schema: &str, instance: &Value) -> TestResult {
    let schema: Value = serde_json::from_str(&fs::read_to_string(schema_file(schema))?)?;
    jsonschema::options()
        .with_retriever(LocalSchemas)
        .build(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{error}")))?;
    Ok(())
}

/// Nobody at a terminal: the table and the verdict, why nothing was asked,
/// and `setup check`'s exit status (2, nothing found). A `y` waiting on
/// standard input is not an answer, because nobody was asked.
#[test]
fn without_a_terminal_the_door_reports_and_installs_nothing() -> TestResult {
    let root = OwnedRoot::new()?;
    for arguments in [vec!["setup"], vec!["setup", "--include", "web-videos"]] {
        let output = vsift(&root, &arguments)?;
        let stdout = String::from_utf8(output.stdout)?;

        assert_eq!(output.status.code(), Some(2), "{stdout}");
        assert!(
            stdout.starts_with(&format!("VSift {} setup\n", env!("CARGO_PKG_VERSION"))),
            "{stdout}"
        );
        assert!(
            stdout.contains("Your setup isn't complete: video and speech are missing.\n"),
            "{stdout}"
        );
        assert!(
            stdout.contains(
                "Nobody is here to answer, so nothing was installed.\nTo install without \
                 questions: vsift setup --unattended --install video,speech\n"
            ),
            "{stdout}"
        );
        assert!(
            !stdout.contains("[y/N]") && !stdout.contains("Installing"),
            "{stdout}"
        );
        assert!(
            stdout
                .chars()
                .all(|character| character == '\n' || !character.is_control()),
            "{stdout:?}"
        );
        assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    }
    Ok(())
}

/// `--json` and `--events jsonl` ask for an answer no person reads: a typed
/// `INVALID_ARGUMENT` under the `setup` command, pointing at `setup check`
/// and the plan route, before anything runs.
#[test]
fn json_and_events_are_refused_with_a_typed_failure() -> TestResult {
    let root = OwnedRoot::new()?;
    let output = vsift(&root, &["--json", "setup"])?;
    let value: Value = serde_json::from_slice(&output.stdout)?;

    assert_eq!(output.status.code(), Some(2));
    validate("operation-response.schema.json", &value)?;
    assert_eq!(value["command"], "setup");
    assert_eq!(value["status"], "failed");
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    let remediation = value["error"]["remediation"][0]["summary"]
        .as_str()
        .ok_or("no remediation")?;
    assert!(remediation.contains("setup check --json"), "{remediation}");
    assert!(
        remediation.contains("setup --unattended --install <parts> --json"),
        "{remediation}"
    );

    let output = vsift(&root, &["--events", "jsonl", "setup"])?;
    let stdout = String::from_utf8(output.stdout)?;
    let last: Value = serde_json::from_str(stdout.lines().last().ok_or("no event")?)?;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(last["command"], "setup");
    assert!(stdout.contains("INVALID_ARGUMENT"), "{stdout}");
    Ok(())
}

/// `--include` belongs to plain `vsift setup`: the subcommands keep their own
/// arguments, so `setup check --include web-videos` is a parse failure.
#[test]
fn the_door_options_do_not_reach_the_subcommands() -> TestResult {
    let root = OwnedRoot::new()?;
    let output = vsift(
        &root,
        &["--json", "setup", "check", "--include", "web-videos"],
    )?;
    let value: Value = serde_json::from_slice(&output.stdout)?;

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(value["command"], "parse");
    Ok(())
}

/// `--unattended` needs the parts it installs, and `--install` and
/// `--accept-plan` belong to `--unattended`: each alone is a parse failure,
/// before anything runs.
#[test]
fn the_unattended_options_need_each_other() -> TestResult {
    let root = OwnedRoot::new()?;
    for arguments in [
        vec!["--json", "setup", "--unattended"],
        vec!["--json", "setup", "--install", "video"],
        vec!["--json", "setup", "--accept-plan", "abc"],
        vec!["--json", "setup", "--unattended", "--install", "audio"],
    ] {
        let output = vsift(&root, &arguments)?;
        let value: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert_eq!(value["command"], "parse", "{arguments:?}");
    }
    let output = vsift(
        &root,
        &[
            "--events",
            "jsonl",
            "setup",
            "--unattended",
            "--install",
            "video",
        ],
    )?;
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(output.status.code(), Some(2));
    assert!(stdout.contains("INVALID_ARGUMENT"), "{stdout}");
    assert!(!root.0.join("user").join("vsift").exists());
    Ok(())
}

/// Speech has no reviewed catalogue on Windows and macOS yet: naming it is
/// an error, `MISSING_CAPABILITY` under `setup` with the do-it-yourself
/// step, `data` null, and nothing is created. On Ubuntu, where it can be
/// installed, the run reaches the install, which the debug build's refused
/// network stops: a fail, which since the Fable review's P-2 is a `failed`
/// envelope too, so the two are told apart by `data`. The fail's `data` is
/// the `setup` result with `outcome` `fail`, and its `error.code` is the
/// install's own, whose class is the exit status.
#[test]
fn an_unattended_run_never_asks_and_reports_pass_fail_or_error() -> TestResult {
    let root = OwnedRoot::new()?;
    let output = vsift(
        &root,
        &["--json", "setup", "--unattended", "--install", "speech"],
    )?;
    let value: Value = serde_json::from_slice(&output.stdout)?;
    validate("operation-response.schema.json", &value)?;
    assert_eq!(value["command"], "setup");
    assert_eq!(value["status"], "failed", "{value}");
    if value["data"].is_null() {
        assert_eq!(value["error"]["code"], "MISSING_CAPABILITY", "{value}");
        assert_eq!(output.status.code(), Some(2));
        let remediation = value["error"]["remediation"][0]["summary"]
            .as_str()
            .ok_or("no remediation")?;
        assert!(remediation.contains("install.md"), "{remediation}");
        assert!(
            !root
                .0
                .join("user")
                .join("vsift")
                .join("managed-v1")
                .exists()
        );
    } else {
        validate("setup.schema.json", &value["data"])?;
        assert_eq!(value["data"]["outcome"], "fail", "{value}");
        assert_eq!(value["data"]["parts"][1]["part"], "speech", "{value}");
        assert_eq!(value["data"]["parts"][1]["status"], "not_set_up", "{value}");
        assert_eq!(
            value["error"]["remediation"][0]["summary"],
            vsift_contract::SETUP_FAILED_REMEDIATION,
            "{value}"
        );
        // Every non-zero exit carries an error of its class.
        let code = value["error"]["code"].as_str().ok_or("no error code")?;
        assert_eq!(output.status.code(), Some(exit_of(code)?), "{value}");
    }
    Ok(())
}

/// The documented exit status of a failure code's class (`cli-v1.md`,
/// "Exit and error taxonomy"), for the codes an install can end with.
fn exit_of(code: &str) -> Result<i32, Box<dyn Error>> {
    Ok(match code {
        "MISSING_CAPABILITY" => 2,
        "BUSY" => 4,
        "DEADLINE_EXCEEDED" | "RESOURCE_LIMIT" => 5,
        "CANCELLED" => 6,
        "STORAGE_IO" | "INTEGRITY_FAILURE" | "DOWNLOAD_FAILED" => 7,
        other => return Err(format!("an install does not end with {other}").into()),
    })
}

/// The schema check of the fail branch above runs only where speech can be
/// installed; this holds its helper, with the local resolver for
/// `setup.schema.json`'s `$ref`, on every platform, against the published
/// fail.
#[test]
fn the_published_fail_validates_through_the_local_schemas() -> TestResult {
    let example: Value = serde_json::from_str(&fs::read_to_string(schema_file(
        "examples/setup.fail.json",
    ))?)?;
    validate("operation-response.schema.json", &example)?;
    validate("setup.schema.json", &example["data"])?;
    let mut broken = example["data"].clone();
    broken["outcome"] = Value::from("partial");
    assert!(validate("setup.schema.json", &broken).is_err());
    Ok(())
}
