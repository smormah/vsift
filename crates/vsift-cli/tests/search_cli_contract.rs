//! Public CLI contract for `search` (P08, ADR 0018).
//!
//! Sessions are committed directly through the session store (see
//! [`seed_session`]), exactly as `transcript_cli_contract` does, so every
//! journey runs everywhere without `FFprobe`: the binary then reads the
//! record exactly as it reads an imported or transcribed session. The opt-in
//! `p08_search_e2e` checkpoint imports F10 through the binary instead.

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    fs, io,
    num::{NonZeroU16, NonZeroU32},
    path::PathBuf,
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift::{
    DurabilityRequirement, MediaTime, OperationId, SessionId, StorageGeneration, TranscriptOffset,
    TranscriptRevision,
};
use vsift_application::{
    AsrRevisionRequest, AsrTranscription, ForegroundSessionPort, ImportedRevisionRequest,
    InitializeSessionStorage, InitializeSessionStorageRequest, build_asr_revision,
    build_imported_revision,
};
use vsift_domain::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, ChunkPlan, ChunkTime, CueText, ProviderChunkOutput,
    ProviderSegment, ProviderToken, ProviderTokenKind, Sha256Hex, SourceId, SourceSegment,
    TimeRange, merge_chunks, plan_chunks, validate_chunk_output,
};
use vsift_infrastructure::{FilesystemSessionStore, SourceSnapshot, read_supplied_transcript};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-search-cli-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const DIGEST: &str = "95e3c0b0e778ad9499eb0125f97c1dcf437dd9eb4ea77050b043574f93c2631d";
const SECOND: u64 = 1_000_000;

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

    fn path(&self, child: &str) -> PathBuf {
        self.0.join(child)
    }

    fn sessions(&self) -> PathBuf {
        self.path("private sessions")
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

/// Runs `vsift` with an isolated per-user base and the root's session store.
fn vsift(root: &OwnedRoot, arguments: &[&str]) -> Built<Output> {
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

/// Parses a `--json` result, validating the envelope and, for a successful
/// search, its data.
fn json(output: &Output) -> Built<Value> {
    assert!(output.stderr.is_empty(), "a JSON command wrote to stderr");
    let value: Value = serde_json::from_slice(&output.stdout)?;
    validate("operation-response.schema.json", &value)?;
    if value["command"] == "search" && value["error"].is_null() {
        validate("search-data.schema.json", &value["data"])?;
    }
    Ok(value)
}

/// Splits `--events jsonl` stdout into validated lines.
fn stream(output: &Output) -> Built<Vec<Value>> {
    assert!(
        output.stderr.is_empty(),
        "a JSON Lines command wrote to stderr"
    );
    let text = std::str::from_utf8(&output.stdout)?;
    let body = text
        .strip_suffix('\n')
        .ok_or("the stream does not end with a newline")?;
    let mut lines = Vec::new();
    for (sequence, line) in body.split('\n').enumerate() {
        let value: Value = serde_json::from_str(line)?;
        assert_eq!(value["sequence"], sequence, "sequence is not contiguous");
        assert_eq!(value["command"], "search");
        match value["event"].as_str() {
            Some("evidence") => {
                validate("evidence-event.schema.json", &value)?;
                validate("transcript-segment.schema.json", &value["record"])?;
            }
            Some("terminal") => {
                validate("terminal-event.schema.json", &value)?;
                validate("operation-response.schema.json", &value["result"])?;
                if value["result"]["error"].is_null() {
                    validate("search-stream-data.schema.json", &value["result"]["data"])?;
                }
            }
            _ => return Err("a stream line has no published event kind".into()),
        }
        lines.push(value);
    }
    let last = lines.last().ok_or("empty stream")?;
    assert_eq!(last["event"], "terminal", "the last line is not terminal");
    Ok(lines)
}

/// Creates the root's store and one open session over a stand-in source,
/// then commits the revision `build` makes for that source as its transcript.
async fn seed_session(
    root: &OwnedRoot,
    build: impl FnOnce(&SessionId, &SourceId) -> Built<TranscriptRevision>,
) -> Built<String> {
    let source = root.path("stand-in.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomsearch-contract")?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let session_id = SessionId::parse(SESSION)?;
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
    let revision = build(&session_id, snapshot.id())?;
    store.activate_with_transcript(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        now,
        &revision,
    )?;
    Ok(session_id.as_str().to_owned())
}

/// The F10 `SubRip` import (+500 ms) of a 12 s source.
fn f10(session_id: &SessionId, source_id: &SourceId) -> Built<TranscriptRevision> {
    let supplied = read_supplied_transcript(&repository("fixtures/corpus/transcripts/F10.srt"))?;
    Ok(build_imported_revision(ImportedRevisionRequest {
        session_id,
        source_id,
        source_duration: MediaTime::from_micros(12 * SECOND),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(500_000)?,
        number: NonZeroU32::MIN,
    })?)
}

/// A local-ASR revision of a 12 s source that transcribed only 0-6 s, where
/// it heard "Dialog R-17 is displayed now." at 1-4 s.
fn half_transcribed(session_id: &SessionId, source_id: &SourceId) -> Built<TranscriptRevision> {
    let source = vsift_application::whole_file_source_segment(
        source_id,
        MediaTime::from_micros(12 * SECOND),
    )?;
    let covered = TimeRange::new(
        MediaTime::from_micros(0),
        MediaTime::from_micros(6 * SECOND),
    )?;
    let transcription = heard(&source, covered, "Dialog R-17 is displayed now.")?;
    Ok(build_asr_revision(AsrRevisionRequest {
        session_id,
        source_id,
        source_segment: &source,
        number: NonZeroU32::MIN,
        transcription,
        splice: None,
    })?)
}

fn heard(source: &SourceSegment, covered: TimeRange, words: &str) -> Built<AsrTranscription> {
    let planned = plan_chunks(source.id(), covered, ChunkPlan::R0)?;
    let run = AsrRun::new(AsrRunParts {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(DIGEST)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(DIGEST)?),
        decoding: AsrDecodingProfile::R0V1,
        plan: ChunkPlan::R0,
        threads: NonZeroU16::new(4).ok_or("zero")?,
        audio_stream: 1,
        chunks: planned
            .into_iter()
            .map(|chunk| {
                AsrChunkRecord::new(chunk, AsrChunkOutcome::Transcribed { audio: covered })
            })
            .collect(),
    })?;
    let chunk = run.chunks().first().ok_or("no chunk")?.chunk().clone();
    let output = ProviderChunkOutput {
        language: None,
        segments: vec![ProviderSegment {
            start: ChunkTime::from_millis(1_000).ok_or("time")?,
            end: ChunkTime::from_millis(4_000).ok_or("time")?,
            text: Some(CueText::new(words.to_owned(), words.to_owned())?),
            tokens: vec![ProviderToken {
                kind: ProviderTokenKind::Text,
                probability: 0.75,
            }],
        }],
    };
    let (segments, language, warnings) =
        validate_chunk_output(&chunk, covered, source.range(), output)?.into_parts();
    Ok(AsrTranscription {
        run,
        language,
        segments: merge_chunks(&[segments]).segments,
        warnings,
    })
}

fn search<'a>(session: &'a str, query: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut arguments = vec!["search", session, "--query", query];
    arguments.extend_from_slice(extra);
    arguments
}

/// F10-E01: "R-17" and the spoken "dialog r 17" both cite the dialog segment
/// on its frozen truth window; the result is complete, with the supplied
/// transcript's coverage stated.
#[tokio::test]
async fn search_finds_f10_dialog_r17_as_a_phrase() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root, f10).await?;
    for query in ["R-17", "dialog r 17"] {
        let output = vsift(&root, &search(&session, query, &["--json"]))?;
        assert_eq!(output.status.code(), Some(0), "{query}");
        let value = json(&output)?;
        assert_eq!(value["command"], "search");
        assert_eq!(value["status"], "complete");
        assert_eq!(value["lifecycle"]["mode"], "ephemeral");
        assert_eq!(
            value["coverage"],
            serde_json::json!({"truncated": false, "gaps": [], "reasons": []})
        );
        let data = &value["data"];
        let items = data["items"].as_array().ok_or("items missing")?;
        assert_eq!(items.len(), 1, "{query}");
        assert_eq!(items[0]["text"], "Dialog R-17 is displayed now.");
        assert_eq!(items[0]["start_us"], 5_000_000);
        assert_eq!(items[0]["end_us"], 9_000_000);
        assert_eq!(data["hits"][0]["segment_id"], items[0]["segment_id"]);
        assert_eq!(data["hits"][0]["match"], "phrase");
        assert_eq!(data["transcript_coverage"]["basis"], "supplied_transcript");
        assert!(data["range"].is_null());
    }
    Ok(())
}

/// `--events jsonl` streams the matching segments as `transcript_segment`
/// evidence, then one terminal event with the hits; its records are the
/// `--json` items.
#[tokio::test]
async fn search_streams_matching_records_then_one_terminal_event() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root, f10).await?;
    let output = vsift(
        &root,
        &search(&session, "synthetic", &["--events", "jsonl"]),
    )?;
    assert_eq!(output.status.code(), Some(0));
    let lines = stream(&output)?;
    assert_eq!(lines.len(), 3);
    let terminal = &lines[2]["result"];
    assert_eq!(terminal["status"], "complete");
    assert_eq!(terminal["data"]["record_count"], 2);
    for (line, hit) in lines[..2]
        .iter()
        .zip(terminal["data"]["hits"].as_array().ok_or("hits missing")?)
    {
        assert_eq!(line["record_type"], "transcript_segment");
        assert_eq!(line["key"], hit["segment_id"]);
    }
    let page = json(&vsift(&root, &search(&session, "synthetic", &["--json"]))?)?;
    let items: Vec<&Value> = page["data"]["items"]
        .as_array()
        .ok_or("items missing")?
        .iter()
        .collect();
    let records: Vec<&Value> = lines[..2].iter().map(|line| &line["record"]).collect();
    assert!(records == items, "streamed records differ from the items");

    // The records are the same evidence transcript get returns.
    let transcript = vsift(
        &root,
        &[
            "transcript",
            "get",
            &session,
            "--from",
            "0",
            "--to",
            "12000000",
            "--json",
        ],
    )?;
    let transcript: Value = serde_json::from_slice(&transcript.stdout)?;
    let segments = transcript["data"]["items"].as_array().ok_or("no items")?;
    for record in records {
        assert!(
            segments.contains(record),
            "a search record is not a transcript get item"
        );
    }
    Ok(())
}

/// C-03 through the binary: pages at `--limit 1` cover every hit once, a
/// cursor may be reused, and a cursor used for another search is rejected.
#[tokio::test]
async fn search_pages_with_cursors_scoped_to_the_search() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root, f10).await?;
    let mut cursor: Option<String> = None;
    let mut seen = Vec::new();
    let mut first_cursor = None;
    loop {
        let mut extra = vec!["--limit", "1", "--json"];
        if let Some(cursor) = cursor.as_deref() {
            extra.extend(["--cursor", cursor]);
        }
        let page = json(&vsift(&root, &search(&session, "synthetic", &extra))?)?;
        for hit in page["data"]["hits"].as_array().ok_or("hits missing")? {
            seen.push(hit["segment_id"].as_str().ok_or("id")?.to_owned());
        }
        match page["data"]["next_cursor"].as_str() {
            Some(next) => {
                first_cursor.get_or_insert_with(|| next.to_owned());
                cursor = Some(next.to_owned());
            }
            None => break,
        }
    }
    assert_eq!(seen.len(), 2);
    assert_eq!(seen.iter().collect::<BTreeSet<_>>().len(), 2);
    let cursor = first_cursor.ok_or("no cursor")?;

    let reused = |query: &str, extra: &[&str]| -> Built<Output> {
        let mut arguments = vec!["--limit", "1", "--cursor", cursor.as_str(), "--json"];
        arguments.extend_from_slice(extra);
        vsift(&root, &search(&session, query, &arguments))
    };
    let again = json(&reused("synthetic", &[])?)?;
    assert_eq!(again["data"]["hits"][0]["segment_id"], seen[1].as_str());
    for (label, output) in [
        ("another query", reused("sidecar", &[])?),
        (
            "another range",
            reused("synthetic", &["--from", "0", "--to", "5000000"])?,
        ),
    ] {
        assert_eq!(output.status.code(), Some(2), "{label}");
        let value = json(&output)?;
        assert_eq!(value["command"], "search", "{label}");
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT", "{label}");
    }
    Ok(())
}

/// Rejected queries fail before any read with fixed remediation naming the
/// reason, and never repeat the query.
#[tokio::test]
async fn rejected_queries_name_their_reason() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root, f10).await?;
    let long = "a".repeat(257);
    let many = vec!["w"; 17].join(" ");
    for (query, reason) in [
        ("", "empty"),
        ("!? ...", "empty"),
        (long.as_str(), "too_long"),
        (many.as_str(), "too_many_terms"),
        ("secret\tvalue", "control_character"),
    ] {
        let output = vsift(&root, &search(&session, query, &["--json"]))?;
        assert_eq!(output.status.code(), Some(2), "{reason}");
        let value = json(&output)?;
        assert_eq!(value["command"], "search");
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
        let summary = value["error"]["remediation"][0]["summary"]
            .as_str()
            .ok_or("remediation missing")?;
        assert!(summary.contains(reason), "{summary}");
        assert!(!summary.contains("secret"), "remediation echoed the query");

        let events = vsift(&root, &search(&session, query, &["--events", "jsonl"]))?;
        let lines = stream(&events)?;
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["result"]["error"]["code"], "INVALID_ARGUMENT");
    }
    Ok(())
}

/// Page sizes and range flags are validated by the grammar; an empty range,
/// an unknown revision, a session without a transcript and a closed session
/// are typed invalid arguments.
#[tokio::test]
async fn search_rejects_bad_requests_and_sessions() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root, f10).await?;
    for extra in [
        vec!["--limit", "0"],
        vec!["--limit", "101"],
        vec!["--from", "0"],
        vec!["--to", "10"],
        vec!["--revision", "latest"],
    ] {
        let mut arguments = search(&session, "dialog", &extra);
        arguments.push("--json");
        let output = vsift(&root, &arguments)?;
        assert_eq!(output.status.code(), Some(2), "{extra:?}");
        assert_eq!(json(&output)?["command"], "parse", "{extra:?}");
    }
    for limit in ["1", "100"] {
        let output = vsift(
            &root,
            &search(&session, "dialog", &["--limit", limit, "--json"]),
        )?;
        assert_eq!(output.status.code(), Some(0), "limit {limit}");
    }
    let empty = vsift(
        &root,
        &search(&session, "dialog", &["--from", "5", "--to", "5", "--json"]),
    )?;
    assert_eq!(empty.status.code(), Some(2));
    assert_eq!(json(&empty)?["error"]["code"], "INVALID_ARGUMENT");

    let unknown = vsift(
        &root,
        &search(
            &session,
            "dialog",
            &["--revision", "trv_2222222222222222", "--json"],
        ),
    )?;
    let value = json(&unknown)?;
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert!(
        value["error"]["remediation"][0]["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("revision_id"))
    );

    let closed = vsift(&root, &["session", "close", &session, "--json"])?;
    assert_eq!(closed.status.code(), Some(0));
    let after_close = vsift(&root, &search(&session, "dialog", &["--json"]))?;
    assert_eq!(after_close.status.code(), Some(2));
    assert_eq!(json(&after_close)?["error"]["code"], "INVALID_ARGUMENT");

    // A session without a transcript names both ways to get one.
    let other = OwnedRoot::new()?;
    let source = other.path("original.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomsource-content")?;
    let opened = json(&vsift(
        &other,
        &["ingest", source.to_str().ok_or("path")?, "--json"],
    )?)?;
    let bare = opened["data"]["session_id"].as_str().ok_or("session")?;
    let without = vsift(&other, &search(bare, "dialog", &["--json"]))?;
    assert_eq!(without.status.code(), Some(2));
    let value = json(&without)?;
    let remedy = value["error"]["remediation"][0]["summary"]
        .as_str()
        .ok_or("remediation missing")?;
    assert!(remedy.contains("transcript retranscribe") && remedy.contains("ingest --transcript"));
    Ok(())
}

/// A search over a range no transcript fully covers still succeeds (exit 0)
/// as `partial`, with the gap in the envelope coverage.
#[tokio::test]
async fn an_incompletely_transcribed_search_is_partial_and_exits_zero() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_session(&root, half_transcribed).await?;
    let output = vsift(&root, &search(&session, "R-17", &["--json"]))?;
    assert_eq!(output.status.code(), Some(0));
    let value = json(&output)?;
    assert_eq!(value["status"], "partial");
    assert_eq!(
        value["coverage"],
        serde_json::json!({
            "truncated": true,
            "gaps": ["6000000-12000000"],
            "reasons": ["untranscribed_range"],
        })
    );
    assert_eq!(value["data"]["hits"][0]["match"], "phrase");
    assert_eq!(value["data"]["transcript_coverage"]["basis"], "local_asr");

    let events = vsift(&root, &search(&session, "R-17", &["--events", "jsonl"]))?;
    assert_eq!(events.status.code(), Some(0));
    let lines = stream(&events)?;
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1]["result"]["status"], "partial");

    let inside = vsift(
        &root,
        &search(
            &session,
            "R-17",
            &["--from", "0", "--to", "6000000", "--json"],
        ),
    )?;
    assert_eq!(json(&inside)?["status"], "complete");
    Ok(())
}
