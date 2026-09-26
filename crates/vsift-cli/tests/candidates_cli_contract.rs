//! Public CLI contract for `candidates` (P08, ADR 0018).
//!
//! A session is opened through the binary over a stand-in source, and its
//! visual index is committed directly through the session store (see
//! [`seed_index`]), so every journey runs everywhere without `FFmpeg`: the
//! binary then reads the record exactly as it reads one it built itself. A
//! request whose windows are all indexed is a warm read that resolves no
//! tool, which the empty `PATH` of every run proves. The opt-in
//! `p08_candidates_e2e` checkpoint builds indexes through the binary instead.

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    fs, io,
    num::NonZeroU32,
    path::PathBuf,
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift::{MediaTime, OperationId, SessionId, SourceId};
use vsift_application::{VisualIndexScope, visual_candidate_id, visual_index_id};
use vsift_contract::{CANDIDATE_CURSOR_REMEDIATION, VISUAL_TOOLS_REMEDIATION};
use vsift_domain::{
    FrameDimensions, SessionArtifactKind, VISUAL_BLOCKS, VisualCandidate, VisualChangePolicy,
    VisualHash, VisualIndex, VisualIndexParts, VisualIndexProfile, VisualIndexWindow, VisualSample,
    VisualWindow, VisualWindowOutcome, analyse_window,
};
use vsift_infrastructure::{FilesystemSessionStore, encode_visual_index_record};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-candidates-cli-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SECOND: u64 = 1_000_000;
/// The stand-in video is taken to last 150 s: windows 0 and 1 are indexed
/// (analysed and undecodable), window 2 is not.
const DURATION: u64 = 150 * SECOND;

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

/// Runs `vsift` with an isolated per-user base, the root's session store and
/// an empty `PATH`, so no media tool can be found.
fn vsift(root: &OwnedRoot, arguments: &[&str]) -> Built<Output> {
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

/// Parses a `--json` result, validating the envelope and, for a successful
/// page, its data and every item.
fn json(output: &Output) -> Built<Value> {
    assert!(output.stderr.is_empty(), "a JSON command wrote to stderr");
    let value: Value = serde_json::from_slice(&output.stdout)?;
    validate("operation-response.schema.json", &value)?;
    if value["command"] == "candidates" && value["error"].is_null() {
        validate("candidates-data.schema.json", &value["data"])?;
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
        assert_eq!(value["command"], "candidates");
        match value["event"].as_str() {
            Some("evidence") => {
                validate("evidence-event.schema.json", &value)?;
                validate("visual-candidate.schema.json", &value["record"])?;
            }
            Some("terminal") => {
                validate("terminal-event.schema.json", &value)?;
                validate("operation-response.schema.json", &value["result"])?;
                if value["result"]["error"].is_null() {
                    validate(
                        "candidates-stream-data.schema.json",
                        &value["result"]["data"],
                    )?;
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

/// Opens a session over a stand-in source through the binary; plain
/// `ingest` runs no provider.
fn open_session(root: &OwnedRoot) -> Built<String> {
    let source = root.path("stand-in.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomcandidates-contract")?;
    let opened = json(&vsift(
        root,
        &["ingest", source.to_str().ok_or("path")?, "--json"],
    )?)?;
    Ok(opened["data"]["session_id"]
        .as_str()
        .ok_or("no session")?
        .to_owned())
}

/// A screen that changes every 20 s of its window: 40, 90, 140 grey levels.
fn window_samples(window: VisualWindow) -> Vec<VisualSample> {
    let mut samples = Vec::new();
    let mut time = window.lead_in_start().as_micros();
    while time < window.range().end().as_micros() {
        let level = u8::try_from(40 + ((time % (60 * SECOND)) / (20 * SECOND)) * 50).unwrap_or(0);
        samples.push(VisualSample::from_parts(
            MediaTime::from_micros(time),
            [level; VISUAL_BLOCKS],
            VisualHash::from_bits(u64::from(level)),
        ));
        time += SECOND / 2;
    }
    samples
}

/// Commits a visual index for the session's source: window 0 analysed,
/// window 1 undecodable, window 2 missing.
fn seed_index(root: &OwnedRoot, session: &str) -> Built<VisualIndex> {
    let store = FilesystemSessionStore::open_existing(root.sessions())?;
    let session_id = SessionId::parse(session)?;
    let status = store.session_status(&session_id)?;
    let source: SourceId = status.source_id().clone();
    let scope = VisualIndexScope {
        session_id: &session_id,
        source_id: &source,
        stream_index: 0,
        displayed_dimensions: FrameDimensions::new(1440, 900)?,
        duration: MediaTime::from_micros(DURATION),
        profile: VisualIndexProfile::R0,
    };
    let first = VisualWindow::new(0, scope.duration)?;
    let analysis = analyse_window(first, &window_samples(first), VisualChangePolicy::R0)?;
    let mut candidates = Vec::new();
    for draft in analysis.candidates {
        candidates.push(VisualCandidate::new(
            visual_candidate_id(&scope, 0, draft.representative)?,
            draft,
        ));
    }
    let windows = vec![
        VisualIndexWindow::new(
            first,
            VisualWindowOutcome::Analysed {
                sample_count: analysis.sample_count,
                candidates,
                dropped_candidates: analysis.dropped_candidates,
                frameless_cells: analysis.frameless_cells,
            },
            VisualChangePolicy::R0,
        )?,
        VisualIndexWindow::new(
            VisualWindow::new(1, scope.duration)?,
            VisualWindowOutcome::Undecodable,
            VisualChangePolicy::R0,
        )?,
    ];
    let number = NonZeroU32::MIN;
    let index = VisualIndex::new(VisualIndexParts {
        id: visual_index_id(&scope, number)?,
        number,
        source_id: source.clone(),
        stream_index: 0,
        displayed_dimensions: scope.displayed_dimensions,
        duration: scope.duration,
        profile: VisualIndexProfile::R0,
        windows,
    })?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    store.publish_artifact(
        &session_id,
        &OperationId::parse("op_3333333333333333")?,
        status.generation(),
        SessionArtifactKind::VisualIndexRecord,
        &encode_visual_index_record(&session_id, &index)?,
        now,
    )?;
    Ok(index)
}

fn candidates<'a>(session: &'a str, from: &'a str, to: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut arguments = vec!["candidates", session, "--from", from, "--to", to];
    arguments.extend_from_slice(extra);
    arguments
}

fn ids(page: &Value) -> Built<Vec<String>> {
    page["data"]["items"]
        .as_array()
        .ok_or("items missing")?
        .iter()
        .map(|item| {
            item["candidate_id"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "candidate_id missing".into())
        })
        .collect()
}

/// A fully indexed range is read without any tool (the `PATH` is empty) and
/// is complete: candidates in time order, one at every screen change and
/// every 10 s cell, with empty envelope coverage.
#[tokio::test]
async fn an_indexed_range_is_read_without_tools_and_complete() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = open_session(&root)?;
    let index = seed_index(&root, &session)?;
    let output = vsift(&root, &candidates(&session, "0", "60000000", &["--json"]))?;
    assert_eq!(output.status.code(), Some(0));
    let value = json(&output)?;
    assert_eq!(value["command"], "candidates");
    assert_eq!(value["status"], "complete");
    assert_eq!(
        value["coverage"],
        serde_json::json!({"truncated": false, "gaps": [], "reasons": []})
    );
    let data = &value["data"];
    assert_eq!(data["index"]["index_id"], index.id().as_str());
    assert_eq!(data["index"]["duration_us"], DURATION);
    let expected: Vec<String> = index
        .windows()
        .iter()
        .flat_map(VisualIndexWindow::candidates)
        .map(|candidate| candidate.id().as_str().to_owned())
        .collect();
    assert_eq!(ids(&value)?, expected);
    let times: Vec<u64> = data["items"]
        .as_array()
        .ok_or("items")?
        .iter()
        .filter_map(|item| item["representative_us"].as_u64())
        .collect();
    assert!(times.windows(2).all(|pair| pair[0] < pair[1]));
    for change in [20 * SECOND, 40 * SECOND] {
        assert!(times.contains(&change), "no candidate at {change} us");
    }
    for cell in 0..6 {
        assert!(
            times
                .iter()
                .any(|time| (cell * 10 * SECOND..(cell + 1) * 10 * SECOND).contains(time)),
            "cell {cell} has no candidate"
        );
    }
    Ok(())
}

/// `--events jsonl` streams the same records as `visual_candidate` evidence
/// events keyed by candidate identity, then one terminal event.
#[tokio::test]
async fn candidates_stream_records_then_one_terminal_event() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = open_session(&root)?;
    seed_index(&root, &session)?;
    let page = json(&vsift(
        &root,
        &candidates(&session, "0", "60000000", &["--json"]),
    )?)?;
    let events = vsift(
        &root,
        &candidates(&session, "0", "60000000", &["--events", "jsonl"]),
    )?;
    assert_eq!(events.status.code(), Some(0));
    let lines = stream(&events)?;
    let items = page["data"]["items"].as_array().ok_or("items")?;
    let (terminal, records) = lines.split_last().ok_or("empty")?;
    assert_eq!(records.len(), items.len());
    for (record, item) in records.iter().zip(items) {
        assert_eq!(record["record_type"], "visual_candidate");
        assert_eq!(record["key"], item["candidate_id"]);
        assert_eq!(&record["record"], item);
    }
    assert_eq!(terminal["result"]["data"]["record_count"], items.len());
    assert_eq!(terminal["result"]["status"], "complete");
    Ok(())
}

/// Pages of every size cover the range without a gap or a duplicate; a
/// cursor is reusable and bound to its range.
#[tokio::test]
async fn candidates_page_with_cursors_scoped_to_the_range() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = open_session(&root)?;
    seed_index(&root, &session)?;
    let whole = ids(&json(&vsift(
        &root,
        &candidates(&session, "0", "60000000", &["--limit", "100", "--json"]),
    )?)?)?;
    assert!(whole.len() > 2);
    let mut cursor: Option<String> = None;
    let mut seen = Vec::new();
    let mut second_cursor = None;
    loop {
        let mut extra = vec!["--limit", "1", "--json"];
        if let Some(cursor) = cursor.as_deref() {
            extra.extend(["--cursor", cursor]);
        }
        let page = json(&vsift(
            &root,
            &candidates(&session, "0", "60000000", &extra),
        )?)?;
        seen.extend(ids(&page)?);
        match page["data"]["next_cursor"].as_str() {
            Some(next) => {
                if cursor.is_some() {
                    second_cursor.get_or_insert_with(|| next.to_owned());
                }
                cursor = Some(next.to_owned());
            }
            None => break,
        }
    }
    assert_eq!(seen, whole);
    assert_eq!(seen.iter().collect::<BTreeSet<_>>().len(), seen.len());

    let cursor = second_cursor.ok_or("no second cursor")?;
    let reuse = |from: &str, to: &str| -> Built<Output> {
        vsift(
            &root,
            &candidates(
                &session,
                from,
                to,
                &["--limit", "1", "--cursor", cursor.as_str(), "--json"],
            ),
        )
    };
    let first = json(&reuse("0", "60000000")?)?;
    let again = json(&reuse("0", "60000000")?)?;
    assert_eq!(ids(&first)?, ids(&again)?);
    assert_eq!(ids(&first)?.first(), whole.get(2));
    let moved = reuse("0", "30000000")?;
    assert_eq!(moved.status.code(), Some(2));
    assert_eq!(json(&moved)?["error"]["code"], "INVALID_ARGUMENT");
    let forged = vsift(
        &root,
        &candidates(
            &session,
            "0",
            "60000000",
            &["--cursor", "v1.forged", "--json"],
        ),
    )?;
    assert_eq!(forged.status.code(), Some(2));
    assert_eq!(json(&forged)?["error"]["code"], "INVALID_ARGUMENT");
    Ok(())
}

/// A range with an undecodable window is `partial` (exit 0) with the typed
/// gap; one that needs analysis without media tools is a missing capability
/// that names what to install and writes nothing.
#[tokio::test]
async fn gaps_are_partial_and_analysis_needs_media_tools() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = open_session(&root)?;
    seed_index(&root, &session)?;
    let output = vsift(&root, &candidates(&session, "0", "120000000", &["--json"]))?;
    assert_eq!(output.status.code(), Some(0));
    let value = json(&output)?;
    assert_eq!(value["status"], "partial");
    assert_eq!(
        value["coverage"],
        serde_json::json!({
            "truncated": true,
            "gaps": ["60000000-120000000"],
            "reasons": ["undecodable"],
        })
    );
    assert_eq!(
        value["data"]["coverage"]["gaps"][0]["reason"],
        "undecodable"
    );

    let before = fs::read_dir(root.sessions())?.count();
    let missing = vsift(&root, &candidates(&session, "0", "150000000", &["--json"]))?;
    assert_eq!(missing.status.code(), Some(2));
    let value = json(&missing)?;
    assert_eq!(value["error"]["code"], "MISSING_CAPABILITY");
    assert_eq!(
        value["error"]["remediation"][0]["summary"],
        VISUAL_TOOLS_REMEDIATION
    );
    assert_eq!(fs::read_dir(root.sessions())?.count(), before);
    // A continuation never analyses, so it needs no tool: its cursor is
    // judged (and here rejected) instead.
    let page = json(&vsift(
        &root,
        &candidates(
            &session,
            "0",
            "150000000",
            &["--cursor", "v1.forged", "--json"],
        ),
    )?)?;
    assert_eq!(page["error"]["code"], "INVALID_ARGUMENT");
    Ok(())
}

/// Page sizes are validated by the grammar; an empty range, a range past
/// the video, an unknown or closed session and a cursor for a session
/// without an index are typed invalid arguments.
#[tokio::test]
async fn candidates_reject_bad_requests_and_sessions() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = open_session(&root)?;
    for extra in [vec!["--limit", "0"], vec!["--limit", "101"]] {
        let mut arguments = candidates(&session, "0", "60000000", &extra);
        arguments.push("--json");
        let output = vsift(&root, &arguments)?;
        assert_eq!(output.status.code(), Some(2), "{extra:?}");
        assert_eq!(json(&output)?["command"], "parse", "{extra:?}");
    }
    let lone = vsift(&root, &["candidates", &session, "--from", "0", "--json"])?;
    assert_eq!(lone.status.code(), Some(2));
    assert_eq!(json(&lone)?["command"], "parse");

    let without = vsift(
        &root,
        &candidates(&session, "0", "60000000", &["--cursor", "v1.any", "--json"]),
    )?;
    assert_eq!(without.status.code(), Some(2));
    let value = json(&without)?;
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(
        value["error"]["remediation"][0]["summary"],
        CANDIDATE_CURSOR_REMEDIATION
    );

    seed_index(&root, &session)?;
    for limit in ["1", "100"] {
        let output = vsift(
            &root,
            &candidates(&session, "0", "60000000", &["--limit", limit, "--json"]),
        )?;
        assert_eq!(output.status.code(), Some(0), "limit {limit}");
    }
    for (from, to) in [("5", "5"), ("10", "5"), ("150000000", "160000000")] {
        let output = vsift(&root, &candidates(&session, from, to, &["--json"]))?;
        assert_eq!(output.status.code(), Some(2), "{from}-{to}");
        assert_eq!(json(&output)?["error"]["code"], "INVALID_ARGUMENT");
    }
    let empty = json(&vsift(
        &root,
        &candidates(&session, "59000000", "59100000", &["--json"]),
    )?)?;
    assert_eq!(empty["status"], "complete");
    assert_eq!(empty["data"]["items"], serde_json::json!([]));
    assert_eq!(empty["data"]["next_cursor"], Value::Null);

    let unknown = vsift(
        &root,
        &candidates("ses_ffffffffffffffff", "0", "60000000", &["--json"]),
    )?;
    assert_ne!(unknown.status.code(), Some(0));
    assert!(json(&unknown)?["error"]["code"].is_string());

    let closed = vsift(&root, &["session", "close", &session, "--json"])?;
    assert_eq!(closed.status.code(), Some(0));
    let after_close = vsift(&root, &candidates(&session, "0", "60000000", &["--json"]))?;
    assert_eq!(after_close.status.code(), Some(2));
    assert_eq!(json(&after_close)?["error"]["code"], "INVALID_ARGUMENT");
    Ok(())
}

/// Without `--json`, the result is the same document, indented.
#[tokio::test]
async fn human_output_is_the_indented_result() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = open_session(&root)?;
    seed_index(&root, &session)?;
    let output = vsift(&root, &candidates(&session, "0", "60000000", &[]))?;
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout)?;
    assert!(text.contains("\n  \"command\": \"candidates\""));
    let value: Value = serde_json::from_str(&text)?;
    validate("candidates-data.schema.json", &value["data"])?;
    Ok(())
}
