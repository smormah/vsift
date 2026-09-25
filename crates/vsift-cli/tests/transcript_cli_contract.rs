//! Public CLI contract for supplied-transcript import and `transcript get`.
//!
//! Rejections run everywhere because they happen before any provider runs.
//! Importing F10 probes the video with the real `FFprobe`, so that journey is
//! `#[ignore]`d and opt-in (it needs `ffmpeg` and `ffprobe` on `PATH`):
//!
//! `cargo test -p vsift-cli --locked --test transcript_cli_contract -- --ignored`
//!
//! The `--events jsonl` evidence stream is read from a transcript session
//! committed directly through the session store (see [`seed_f10_session`]), so
//! the stream contract runs everywhere without `FFprobe`.

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    fs, io,
    num::NonZeroU32,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift::{
    DurabilityRequirement, MediaTime, OperationId, SessionId, StorageGeneration, TranscriptOffset,
};
use vsift_application::{
    ForegroundSessionPort, ImportedRevisionRequest, InitializeSessionStorage,
    InitializeSessionStorageRequest, build_imported_revision,
};
use vsift_infrastructure::{FilesystemSessionStore, SourceSnapshot, read_supplied_transcript};

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
    // D1: the fixed remediation points at both ways to get a transcript.
    let remedy = value["error"]["remediation"][0]["summary"]
        .as_str()
        .ok_or("remediation missing")?;
    assert!(remedy.contains("transcript retranscribe") && remedy.contains("ingest --transcript"));

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

    // Local ASR is reachable, and with no tools at all it fails typed before
    // touching the session: nothing on PATH and nothing configured.
    let retranscribe = vsift_without_path(
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
    assert_eq!(value["error"]["code"], "MISSING_CAPABILITY");
    let remedy = value["error"]["remediation"][0]["summary"]
        .as_str()
        .ok_or("remediation missing")?;
    assert!(remedy.contains("whisper") && !remedy.contains(path_text(&root.0)?));

    // Both range flags or neither.
    for partial in [["--from", "0"], ["--to", "10"]] {
        let mut full = vec!["transcript", "retranscribe", session];
        full.extend(partial);
        full.push("--json");
        let output = vsift_without_path(&root, &full)?;
        assert_eq!(output.status.code(), Some(2), "{partial:?}");
        assert_eq!(json(&output)?["command"], "parse", "{partial:?}");
    }
    Ok(())
}

/// Runs `vsift` like [`vsift`] with an empty `PATH`, so no provider is found.
fn vsift_without_path(root: &OwnedRoot, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    let base = root.path("user");
    Ok(Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .env("PATH", "")
        .arg("--session-root")
        .arg(root.sessions())
        .args(arguments)
        .output()?)
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

/// Commits the F10 `SubRip` transcript (+500 ms) into the root's session store.
///
/// Importing through the binary needs a real `FFprobe` to measure the video,
/// so this commits the same state through the store with F10's known 12 s
/// duration. The binary then reads it exactly as it reads an imported session;
/// the opt-in journeys cover the import itself.
async fn seed_f10_session(root: &OwnedRoot) -> Result<String, Box<dyn Error>> {
    let source = root.write("f10-stand-in.mp4", b"\0\0\0\x18ftypisomtranscript-stream")?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let session_id = SessionId::parse("ses_0123456789abcdef0123456789abcdef")?;
    let opener = OperationId::parse("op_0123456789abcdef")?;
    let store = FilesystemSessionStore::provision_default(root.sessions())?;
    let registration = store.register_session(&session_id, &opener, now)?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            opener,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    drop(registration);
    let store = FilesystemSessionStore::open_existing(root.sessions())?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_1111111111111111")?,
        &source,
    )?;
    let supplied = read_supplied_transcript(&repository("fixtures/corpus/transcripts/F10.srt"))?;
    let revision = build_imported_revision(ImportedRevisionRequest {
        session_id: &session_id,
        source_id: snapshot.id(),
        source_duration: MediaTime::from_micros(12_000_000),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(500_000)?,
        number: NonZeroU32::MIN,
    })?;
    store.activate_with_transcript(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        now,
        &revision,
    )?;
    Ok(session_id.as_str().to_owned())
}

/// Splits `--events jsonl` stdout into its lines, proving each is a complete,
/// newline-terminated JSON value that validates against its event schemas.
fn stream(output: &Output) -> Result<Vec<Value>, Box<dyn Error>> {
    assert!(
        output.stderr.is_empty(),
        "a JSON Lines command wrote to stderr"
    );
    let text = std::str::from_utf8(&output.stdout)?;
    let body = text
        .strip_suffix('\n')
        .ok_or("the stream does not end with a newline")?;
    let mut lines = Vec::new();
    for line in body.split('\n') {
        let value: Value = serde_json::from_str(line)?;
        match value["event"].as_str() {
            Some("evidence") => {
                validate("evidence-event.schema.json", &value)?;
                validate("transcript-segment.schema.json", &value["record"])?;
                assert!(
                    value["key"] == value["record"]["segment_id"],
                    "an evidence key is not its segment identity"
                );
            }
            Some("terminal") => {
                validate("terminal-event.schema.json", &value)?;
                validate("operation-response.schema.json", &value["result"])?;
                if value["result"]["status"] == "complete" {
                    validate(
                        "transcript-get-stream-data.schema.json",
                        &value["result"]["data"],
                    )?;
                }
            }
            _ => return Err("a stream line has no published event kind".into()),
        }
        lines.push(value);
    }
    for (index, line) in lines.iter().enumerate() {
        assert_eq!(line["sequence"], index, "sequence is not contiguous");
        assert_eq!(line["command"], "transcript.get");
    }
    let (terminal, records) = lines.split_last().ok_or("empty stream")?;
    assert_eq!(
        terminal["event"], "terminal",
        "the last line is not terminal"
    );
    assert!(
        records.iter().all(|line| line["event"] == "evidence"),
        "an event other than evidence precedes the terminal event"
    );
    Ok(lines)
}

fn transcript_get<'a>(session: &'a str, range: [&'a str; 2], extra: &[&'a str]) -> Vec<&'a str> {
    let mut arguments = vec![
        "transcript",
        "get",
        session,
        "--from",
        range[0],
        "--to",
        range[1],
    ];
    arguments.extend_from_slice(extra);
    arguments
}

/// `--events jsonl` streams one evidence event per segment, then exactly one
/// terminal event whose cursor continues the stream on the next call; the
/// records are the `--json` page items, which keep their existing shape.
#[tokio::test]
async fn transcript_get_streams_records_then_one_terminal_event() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_f10_session(&root).await?;
    let full = ["0", "12000000"];

    let first = vsift(
        &root,
        &transcript_get(&session, full, &["--limit", "2", "--events", "jsonl"]),
    )?;
    assert_eq!(first.status.code(), Some(0));
    let first = stream(&first)?;
    assert_eq!(first.len(), 3);
    let terminal = &first[2]["result"];
    assert_eq!(terminal["status"], "complete");
    assert_eq!(terminal["data"]["record_count"], 2);
    assert!(
        terminal["data"]["session_id"] == session.as_str(),
        "the terminal event names another session"
    );
    assert_eq!(terminal["lifecycle"]["mode"], "ephemeral");
    let cursor = terminal["data"]["next_cursor"]
        .as_str()
        .ok_or("the first page has no cursor")?;

    let second = vsift(
        &root,
        &transcript_get(
            &session,
            full,
            &["--limit", "2", "--cursor", cursor, "--events", "jsonl"],
        ),
    )?;
    assert_eq!(second.status.code(), Some(0));
    let second = stream(&second)?;
    assert_eq!(second.len(), 2);
    assert_eq!(second[1]["result"]["data"]["record_count"], 1);
    assert!(second[1]["result"]["data"]["next_cursor"].is_null());

    let streamed: Vec<&Value> = first[..2]
        .iter()
        .chain(&second[..1])
        .map(|line| &line["record"])
        .collect();
    let keys: BTreeSet<&str> = first[..2]
        .iter()
        .chain(&second[..1])
        .filter_map(|line| line["key"].as_str())
        .collect();
    assert_eq!(keys.len(), 3);

    let page = json(&vsift(
        &root,
        &transcript_get(&session, full, &["--limit", "100", "--json"]),
    )?)?;
    validate("transcript-get-data.schema.json", &page["data"])?;
    assert!(page["data"].get("record_count").is_none());
    let items: Vec<&Value> = page["data"]["items"]
        .as_array()
        .ok_or("items missing")?
        .iter()
        .collect();
    assert!(
        streamed == items,
        "the streamed records differ from the --json page items"
    );
    assert!(
        first[2]["result"]["data"]["revision"] == page["data"]["revision"],
        "the stream and the page describe different revisions"
    );
    Ok(())
}

/// The stream never holds more records than the page limit.
#[tokio::test]
async fn the_stream_is_bounded_by_the_page_limit() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_f10_session(&root).await?;

    for limit in ["1", "2", "3", "100"] {
        let output = vsift(
            &root,
            &transcript_get(
                &session,
                ["0", "12000000"],
                &["--limit", limit, "--events", "jsonl"],
            ),
        )?;
        assert_eq!(output.status.code(), Some(0), "limit {limit}");
        let lines = stream(&output)?;
        let expected = limit.parse::<usize>()?.min(3);
        assert_eq!(lines.len(), expected + 1, "limit {limit}");
        assert_eq!(
            lines[expected]["result"]["data"]["record_count"], expected,
            "limit {limit}"
        );
    }
    Ok(())
}

/// A range no segment intersects is a complete stream of one terminal event.
#[tokio::test]
async fn an_empty_range_streams_only_the_terminal_event() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_f10_session(&root).await?;

    let output = vsift(
        &root,
        &transcript_get(&session, ["4000000", "5000000"], &["--events", "jsonl"]),
    )?;
    assert_eq!(output.status.code(), Some(0));
    let lines = stream(&output)?;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["result"]["status"], "complete");
    assert_eq!(lines[0]["result"]["data"]["record_count"], 0);
    assert!(lines[0]["result"]["data"]["next_cursor"].is_null());
    Ok(())
}

/// Requests that fail stay a single terminal failure event at sequence 0.
#[tokio::test]
async fn rejected_stream_requests_are_one_terminal_failure_event() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_f10_session(&root).await?;

    for (label, arguments) in [
        (
            "empty range",
            transcript_get(&session, ["5", "5"], &["--events", "jsonl"]),
        ),
        (
            "foreign cursor",
            transcript_get(
                &session,
                ["0", "12000000"],
                &["--cursor", "v1|not-a-cursor", "--events", "jsonl"],
            ),
        ),
        (
            "unknown session",
            transcript_get(
                "ses_ffffffffffffffffffffffffffffffff",
                ["0", "12000000"],
                &["--events", "jsonl"],
            ),
        ),
    ] {
        let output = vsift(&root, &arguments)?;
        assert_ne!(output.status.code(), Some(0), "{label}");
        let lines = stream(&output)?;
        assert_eq!(lines.len(), 1, "{label}");
        assert_eq!(lines[0]["result"]["status"], "failed", "{label}");
        assert!(lines[0]["result"]["data"].is_null(), "{label}");
    }
    Ok(())
}
