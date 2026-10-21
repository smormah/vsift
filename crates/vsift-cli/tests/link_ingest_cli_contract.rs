//! Public CLI contract for `ingest --url` (ADR 0028, #407), through the
//! binary, with no network: every case is refused before a connection, and
//! a development build resolves no name (the P13 network guard), so a public
//! name answers `offline` here. The fetch itself is tested against a local
//! HTTPS server in `vsift-infrastructure`'s `link_transfer`.

use std::{
    env,
    error::Error,
    fs, io,
    path::PathBuf,
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-link-ingest-cli-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
/// A token every link carries; no output may hold it.
const SENTINEL: &str = "SENTINEL-CLI-4b1d";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Built<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn sessions(&self) -> PathBuf {
        self.0.join("private sessions")
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

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

struct PublishedSchemas;

impl Retrieve for PublishedSchemas {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let name = uri
            .as_str()
            .strip_prefix(SCHEMA_BASE)
            .filter(|name| !name.contains(['/', '\\']))
            .ok_or_else(|| format!("unpublished schema reference: {uri}"))?;
        Ok(serde_json::from_str(&fs::read_to_string(
            repository("schemas/v1").join(name),
        )?)?)
    }
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    let schema: Value = serde_json::from_str(&fs::read_to_string(
        repository("schemas/v1").join(schema_path),
    )?)?;
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

fn vsift(root: &OwnedRoot, arguments: &[&str], stdin: Option<&str>) -> Built<Output> {
    let base = root.0.join("user");
    let mut command = Command::cargo_bin("vsift")?;
    command
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("XDG_CACHE_HOME", &base)
        .env("HOME", &base)
        .env("PATH", "")
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .env_remove("VSIFT_DEV_PUBLISHER_NETWORK")
        .arg("--session-root")
        .arg(root.sessions())
        .args(arguments);
    if let Some(text) = stdin {
        command.write_stdin(text);
    }
    Ok(command.output()?)
}

/// A refused `ingest --url`: the code, its reason, a remediation that names
/// the online step, exit 3, no token anywhere and no session.
fn refused(root: &OwnedRoot, output: &Output, reason: &str) -> Built<Value> {
    let stdout = String::from_utf8(output.stdout.clone())?;
    let stderr = String::from_utf8(output.stderr.clone())?;
    assert!(
        !stdout.contains(SENTINEL) && !stderr.contains(SENTINEL),
        "the token leaked: {stdout}{stderr}"
    );
    let value: Value = serde_json::from_str(&stdout)?;
    validate("operation-response.schema.json", &value)?;
    assert_eq!(value["command"], "ingest");
    assert_eq!(value["status"], "failed");
    assert_eq!(value["error"]["code"], "SOURCE_FETCH_FAILED", "{value}");
    assert_eq!(value["error"]["reason"], reason, "{value}");
    let summary = value["error"]["remediation"][0]["summary"]
        .as_str()
        .ok_or("no remediation")?;
    assert!(
        summary.contains("local video files are not affected"),
        "{summary}"
    );
    assert!(summary.contains(&format!("(reason {reason})")), "{summary}");
    assert_eq!(output.status.code(), Some(3));
    let sessions = root.sessions().join("sessions");
    if sessions.exists() {
        assert_eq!(
            fs::read_dir(&sessions)?.count(),
            0,
            "a refused fetch left a session"
        );
    }
    Ok(value)
}

#[test]
fn links_that_break_a_rule_are_refused_before_anything_is_sent() -> TestResult {
    let root = OwnedRoot::new()?;
    for (link, reason) in [
        (
            format!("http://videos.example.org/a.mp4?token={SENTINEL}"),
            "not_https",
        ),
        (
            format!("ftp://videos.example.org/{SENTINEL}.mp4"),
            "not_https",
        ),
        (format!("/home/example/{SENTINEL}.mp4"), "not_https"),
        (
            format!("https://user:{SENTINEL}@videos.example.org/a.mp4"),
            "credentials_in_link",
        ),
        (
            format!("https://videos.example.org:8443/a.mp4?token={SENTINEL}"),
            "port_not_allowed",
        ),
        (
            format!("https://127.0.0.1/a.mp4?token={SENTINEL}"),
            "address_refused",
        ),
        (
            format!("https://169.254.169.254/latest/meta-data/{SENTINEL}"),
            "address_refused",
        ),
        (
            format!("https://[::1]/a.mp4?token={SENTINEL}"),
            "address_refused",
        ),
        (
            format!("https://10.0.0.8/{SENTINEL}/a.mp4"),
            "address_refused",
        ),
        // A development build resolves no name, so a public name is offline.
        (
            format!("https://videos.example.org/{SENTINEL}/a.mp4"),
            "offline",
        ),
    ] {
        let output = vsift(&root, &["ingest", "--url", &link, "--json"], None)?;
        refused(&root, &output, reason)?;
    }
    Ok(())
}

#[test]
fn the_link_can_come_from_standard_input() -> TestResult {
    let root = OwnedRoot::new()?;
    let output = vsift(
        &root,
        &["ingest", "--url", "-", "--json"],
        Some(&format!("https://127.0.0.1/a.mp4?token={SENTINEL}\n")),
    )?;
    refused(&root, &output, "address_refused")?;
    let output = vsift(
        &root,
        &["ingest", "--url", "-", "--json"],
        Some("not a link\r\n"),
    )?;
    refused(&root, &output, "not_https")?;
    Ok(())
}

#[test]
fn the_options_are_checked_as_arguments() -> TestResult {
    let root = OwnedRoot::new()?;
    for arguments in [
        vec!["ingest", "--json"],
        vec![
            "ingest",
            "video.mp4",
            "--url",
            "https://videos.example.org/a.mp4",
            "--json",
        ],
        vec![
            "ingest",
            "video.mp4",
            "--max-download-bytes",
            "100",
            "--json",
        ],
        vec![
            "ingest",
            "--url",
            "https://videos.example.org/a.mp4",
            "--max-download-bytes",
            "0",
            "--json",
        ],
        vec![
            "ingest",
            "--url",
            "https://videos.example.org/a.mp4",
            "--max-download-bytes",
            "21474836481",
            "--json",
        ],
        vec![
            "ingest",
            "--url",
            "https://videos.example.org/a.mp4",
            "--max-download-seconds",
            "14401",
            "--json",
        ],
    ] {
        let output = vsift(&root, &arguments, None)?;
        let value: Value = serde_json::from_slice(&output.stdout)?;
        validate("operation-response.schema.json", &value)?;
        assert_eq!(
            value["error"]["code"], "INVALID_ARGUMENT",
            "{arguments:?}: {value}"
        );
        assert!(value["error"].get("reason").is_none());
    }
    Ok(())
}

#[test]
fn a_local_ingest_is_unchanged() -> TestResult {
    let root = OwnedRoot::new()?;
    let source = root.0.join("recording.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomlocal-contract")?;
    let output = vsift(
        &root,
        &["ingest", source.to_str().ok_or("path")?, "--json"],
        None,
    )?;
    let value: Value = serde_json::from_slice(&output.stdout)?;
    validate("operation-response.schema.json", &value)?;
    validate("ingest-data.schema.json", &value["data"])?;
    assert_eq!(value["status"], "complete");
    assert!(value["data"].get("origin").is_none(), "{value}");
    Ok(())
}
