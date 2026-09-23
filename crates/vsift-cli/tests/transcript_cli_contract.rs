//! Public CLI contract for supplied-transcript import and `transcript get`.
//!
//! Rejections run everywhere because they happen before any provider runs.
//! Importing F10 probes the video with the real `FFprobe`, so that journey is
//! `#[ignore]`d and opt-in (it needs `ffmpeg` and `ffprobe` on `PATH`):
//!
//! `cargo test -p vsift-cli --locked --test transcript_cli_contract -- --ignored`

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
use jsonschema::{Retrieve, Uri};
use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-transcript-cli-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

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

    fn sessions(&self) -> PathBuf {
        self.path("private sessions")
    }

    fn write(&self, name: &str, bytes: &[u8]) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.path(name);
        fs::write(&path, bytes)?;
        Ok(path)
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

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

struct PublishedSchemas;

impl Retrieve for PublishedSchemas {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let name = uri
            .as_str()
            .strip_prefix(SCHEMA_BASE)
            .filter(|name| !name.contains(['/', '\\']))
            .ok_or_else(|| format!("unpublished schema reference: {uri}"))?;
        Ok(serde_json::from_str(&fs::read_to_string(
            schema_root().join(name),
        )?)?)
    }
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    let schema: Value =
        serde_json::from_str(&fs::read_to_string(schema_root().join(schema_path))?)?;
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

/// Runs `vsift` with an isolated per-user base and the root's session store.
fn vsift(root: &OwnedRoot, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    let base = root.path("user");
    Ok(Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .arg("--session-root")
        .arg(root.sessions())
        .args(arguments)
        .output()?)
}

fn json(output: &Output) -> Result<Value, Box<dyn Error>> {
    let value: Value = serde_json::from_slice(&output.stdout)?;
    validate("operation-response.schema.json", &value)?;
    Ok(value)
}

fn path_text(path: &Path) -> Result<&str, Box<dyn Error>> {
    Ok(path.to_str().ok_or("non-UTF-8 test path")?)
}

#[test]
fn malformed_sidecars_fail_before_mutation_with_typed_remediation() -> TestResult {
    let root = OwnedRoot::new()?;
    let source = root.write("original.mp4", b"\0\0\0\x18ftypisomsource-content")?;
    let malformed = root.write(
        "captions.srt",
        b"1\n00:00:01,000 --> 00:00:02,000\nfine\n\n2\n00:00:61,000 --> 00:00:62,000\nbad\n",
    )?;

    let output = vsift(
        &root,
        &[
            "ingest",
            path_text(&source)?,
            "--transcript",
            path_text(&malformed)?,
            "--transcript-offset",
            "-250000",
            "--json",
        ],
    )?;
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stderr.is_empty());
    let value = json(&output)?;
    assert_eq!(value["command"], "ingest");
    assert_eq!(value["error"]["code"], "INVALID_SOURCE");
    let summary = value["error"]["remediation"][0]["summary"]
        .as_str()
        .ok_or("remediation missing")?;
    assert!(summary.contains("invalid_timestamp"), "{summary}");
    assert!(summary.contains("line 6"), "{summary}");
    assert!(
        !summary.contains("bad"),
        "remediation echoed transcript text"
    );
    assert_eq!(value["error"]["remediation"][0]["command"], Value::Null);
    assert!(!root.sessions().exists());

    // The same failure as a JSON Lines terminal event, and as human text.
    let events = vsift(
        &root,
        &[
            "ingest",
            path_text(&source)?,
            "--transcript",
            path_text(&malformed)?,
            "--events",
            "jsonl",
        ],
    )?;
    let event: Value = serde_json::from_slice(&events.stdout)?;
    validate("terminal-event.schema.json", &event)?;
    assert_eq!(event["result"]["error"]["code"], "INVALID_SOURCE");
    let human = vsift(
        &root,
        &[
            "ingest",
            path_text(&source)?,
            "--transcript",
            path_text(&malformed)?,
        ],
    )?;
    assert_eq!(human.status.code(), Some(3));
    let diagnostic = String::from_utf8(human.stderr)?;
    assert!(diagnostic.contains("invalid_timestamp"), "{diagnostic}");
    assert!(!root.sessions().exists());
    Ok(())
}

#[test]
fn offsets_are_signed_bounded_and_require_a_transcript() -> TestResult {
    let root = OwnedRoot::new()?;
    let source = root.write("original.mp4", b"\0\0\0\x18ftypisomsource-content")?;
    let sidecar = root.write(
        "captions.vtt",
        b"WEBVTT\n\n00:01.000 --> 00:02.000\nHello\n",
    )?;

    let out_of_range = vsift(
        &root,
        &[
            "ingest",
            path_text(&source)?,
            "--transcript",
            path_text(&sidecar)?,
            "--transcript-offset=-86400000001",
            "--json",
        ],
    )?;
    assert_eq!(out_of_range.status.code(), Some(2));
    let value = json(&out_of_range)?;
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert!(
        value["error"]["remediation"][0]["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("offset_out_of_range"))
    );

    let orphan_offset = vsift(
        &root,
        &[
            "ingest",
            path_text(&source)?,
            "--transcript-offset",
            "5",
            "--json",
        ],
    )?;
    assert_eq!(orphan_offset.status.code(), Some(2));
    assert_eq!(json(&orphan_offset)?["command"], "parse");
    assert!(!root.sessions().exists());
    Ok(())
}

#[test]
fn transcript_get_rejects_bad_requests_and_sessions_without_a_transcript() -> TestResult {
    let root = OwnedRoot::new()?;
    let source = root.write("original.mp4", b"\0\0\0\x18ftypisomsource-content")?;
    let opened = json(&vsift(&root, &["ingest", path_text(&source)?, "--json"])?)?;
    validate("ingest-data.schema.json", &opened["data"])?;
    assert!(opened["data"].get("transcript").is_none());
    let session = opened["data"]["session_id"]
        .as_str()
        .ok_or("session missing")?;

    let without = vsift(
        &root,
        &[
            "transcript",
            "get",
            session,
            "--from",
            "0",
            "--to",
            "1000000",
            "--json",
        ],
    )?;
    assert_eq!(without.status.code(), Some(2));
    let value = json(&without)?;
    assert_eq!(value["command"], "transcript.get");
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");

    let empty_range = vsift(
        &root,
        &[
            "transcript",
            "get",
            session,
            "--from",
            "5",
            "--to",
            "5",
            "--json",
        ],
    )?;
    assert_eq!(json(&empty_range)?["error"]["code"], "INVALID_ARGUMENT");

    for arguments in [["--limit", "0"], ["--limit", "101"], ["--from", "-1"]] {
        let mut full = vec!["transcript", "get", session, "--from", "0", "--to", "10"];
        if arguments[0] == "--from" {
            full = vec!["transcript", "get", session, "--to", "10"];
        }
        full.extend(arguments);
        full.push("--json");
        let output = vsift(&root, &full)?;
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert_eq!(json(&output)?["command"], "parse", "{arguments:?}");
    }

    let retranscribe = vsift(
        &root,
        &[
            "transcript",
            "retranscribe",
            session,
            "--from",
            "0",
            "--to",
            "10",
            "--json",
        ],
    )?;
    assert_eq!(retranscribe.status.code(), Some(2));
    let value = json(&retranscribe)?;
    assert_eq!(value["command"], "transcript.retranscribe");
    assert_eq!(value["error"]["code"], "COMMAND_NOT_IMPLEMENTED");
    Ok(())
}

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Opt-in: the supplied-transcript path through the binary on F10.
#[test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
fn f10_import_and_paged_retrieval_through_the_binary() -> TestResult {
    let root = OwnedRoot::new()?;
    let video = repository("fixtures/corpus/generated/F10.mp4");
    let sidecar = repository("fixtures/corpus/transcripts/F10.vtt");
    let output = vsift(
        &root,
        &[
            "ingest",
            path_text(&video)?,
            "--transcript",
            path_text(&sidecar)?,
            "--transcript-offset",
            "500000",
            "--json",
        ],
    )?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let opened = json(&output)?;
    validate("ingest-data.schema.json", &opened["data"])?;
    let transcript = &opened["data"]["transcript"];
    assert_eq!(transcript["alignment"]["origin"], "imported_webvtt");
    assert_eq!(transcript["alignment"]["offset_us"], 500_000);
    assert_eq!(transcript["segment_count"], 3);
    assert_eq!(transcript["source_segments"][0]["end_us"], 12_000_000);
    let session = opened["data"]["session_id"]
        .as_str()
        .ok_or("session missing")?
        .to_owned();

    let mut cursor: Option<String> = None;
    let mut texts = Vec::new();
    loop {
        let mut arguments = vec![
            "transcript",
            "get",
            session.as_str(),
            "--from",
            "0",
            "--to",
            "12000000",
            "--limit",
            "2",
            "--json",
        ];
        if let Some(cursor) = cursor.as_deref() {
            arguments.extend(["--cursor", cursor]);
        }
        let page = vsift(&root, &arguments)?;
        assert_eq!(page.status.code(), Some(0));
        let page = json(&page)?;
        validate("transcript-get-data.schema.json", &page["data"])?;
        assert_eq!(page["lifecycle"]["mode"], "ephemeral");
        for item in page["data"]["items"].as_array().ok_or("items missing")? {
            texts.push((
                item["start_us"].as_u64().ok_or("start missing")?,
                item["end_us"].as_u64().ok_or("end missing")?,
                item["text"].as_str().ok_or("text missing")?.to_owned(),
            ));
        }
        match page["data"]["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }
    assert_eq!(texts.len(), 3);
    assert_eq!(
        texts[1],
        (
            5_000_000,
            9_000_000,
            "Dialog R-17 is displayed now.".to_owned()
        )
    );

    let status = json(&vsift(&root, &["session", "status", &session, "--json"])?)?;
    assert_eq!(status["data"]["artifact_count"], 1);
    Ok(())
}
