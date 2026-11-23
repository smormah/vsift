//! Public CLI contract for `changes` (ADR 0034).
//!
//! A session is opened through the binary over a stand-in source, and its
//! visual index is committed directly through the session store (see
//! [`seed_index`]), so the request checks run everywhere without `FFmpeg`:
//! every run has an empty `PATH`. A range whose windows are all indexed and
//! that holds no change is a warm read that resolves no tool; a range with a
//! change needs `FFmpeg` for its frames, and one with a window to analyse
//! needs it for the analysis. The opt-in `changes_real` test runs the
//! binary on a real recording.

use std::{
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
use vsift_contract::{
    CHANGES_COVERAGE_WARNING, CHANGES_CURSOR_REMEDIATION, CHANGES_CURSOR_WITHOUT_INDEX_REMEDIATION,
    EVIDENCE_TOOLS_REMEDIATION, VISUAL_TOOLS_REMEDIATION,
};
use vsift_domain::{
    FrameDimensions, SessionArtifactKind, VISUAL_BLOCKS, VisualCandidate, VisualChangePolicy,
    VisualHash, VisualIndex, VisualIndexParts, VisualIndexProfile, VisualIndexWindow, VisualSample,
    VisualWindow, VisualWindowOutcome, analyse_window,
};
use vsift_infrastructure::{FilesystemSessionStore, encode_visual_index_record};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-changes-cli-";
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
/// page, its data.
fn json(output: &Output) -> Built<Value> {
    assert!(output.stderr.is_empty(), "a JSON command wrote to stderr");
    let value: Value = serde_json::from_slice(&output.stdout)?;
    validate("operation-response.schema.json", &value)?;
    if value["command"] == "changes" && value["error"].is_null() {
        validate("changes-data.schema.json", &value["data"])?;
    }
    Ok(value)
}

/// Opens a session over a stand-in source through the binary; plain
/// `ingest` runs no provider.
fn open_session(root: &OwnedRoot) -> Built<String> {
    let source = root.path("stand-in.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomchanges-contract")?;
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

/// Commits a visual index for the session's source: window 0 analysed (the
/// screen changes at 20 s and 40 s), window 1 undecodable, window 2 missing.
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

fn changes<'a>(session: &'a str, from: &'a str, to: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut arguments = vec!["changes", session, "--from", from, "--to", to];
    arguments.extend_from_slice(extra);
    arguments
}

fn summary(value: &Value) -> &Value {
    &value["error"]["remediation"][0]["summary"]
}

/// The grammar bounds the page to 1 to 20 changes and needs both ends of
/// the range; each refusal is a parse failure before any work.
#[test]
fn the_grammar_refuses_bad_pages_and_half_ranges() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = "ses_0123456789abcdef0123456789abcdef";
    for arguments in [
        changes(session, "0", "1000", &["--limit", "0", "--json"]),
        changes(session, "0", "1000", &["--limit", "21", "--json"]),
        vec!["changes", session, "--from", "0", "--json"],
        vec!["changes", session, "--to", "1000", "--json"],
        vec!["changes", "--from", "0", "--to", "1000", "--json"],
    ] {
        let output = vsift(&root, &arguments)?;
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        let value = json(&output)?;
        assert_eq!(value["command"], "parse", "{arguments:?}");
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT", "{arguments:?}");
    }
    Ok(())
}

/// A cursor for a session without an index, and a forged one for a session
/// with one, are `INVALID_ARGUMENT` with remediations that name `changes`;
/// neither analyses anything, so neither needs a tool.
#[test]
fn cursors_are_judged_without_tools_and_name_changes() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = open_session(&root)?;
    let without = vsift(
        &root,
        &changes(&session, "0", "60000000", &["--cursor", "v1.any", "--json"]),
    )?;
    assert_eq!(without.status.code(), Some(2));
    let value = json(&without)?;
    assert_eq!(value["command"], "changes");
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(*summary(&value), CHANGES_CURSOR_WITHOUT_INDEX_REMEDIATION);

    seed_index(&root, &session)?;
    let forged = vsift(
        &root,
        &changes(
            &session,
            "0",
            "150000000",
            &["--cursor", "v1.forged", "--json"],
        ),
    )?;
    assert_eq!(forged.status.code(), Some(2));
    let value = json(&forged)?;
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(*summary(&value), CHANGES_CURSOR_REMEDIATION);
    Ok(())
}

/// An indexed range with no change is read without any tool and complete:
/// no change, no frame, no file, no cursor and no `resume`; as a stream it is
/// the terminal event alone, and as text it says how many changes it found.
#[test]
fn an_indexed_range_without_changes_is_complete_and_needs_no_tool() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = open_session(&root)?;
    let index = seed_index(&root, &session)?;
    let output = vsift(
        &root,
        &changes(&session, "0", "20000000", &["--limit", "20", "--json"]),
    )?;
    assert_eq!(output.status.code(), Some(0));
    let value = json(&output)?;
    assert_eq!(value["status"], "complete");
    assert_eq!(
        value["coverage"],
        serde_json::json!({"truncated": false, "gaps": [], "reasons": []})
    );
    let data = &value["data"];
    assert_eq!(data["index"]["index_id"], index.id().as_str());
    assert_eq!(data["profile"]["identifier"], "screen-change-regions-v1");
    for member in ["changes", "frames", "files"] {
        assert_eq!(data[member], serde_json::json!([]), "{member}");
    }
    for member in ["partial_reason", "next_cursor", "resume"] {
        assert_eq!(data[member], Value::Null, "{member}");
    }
    // Without --crops there is no crops member at all (slice 1's answer).
    assert!(data.get("crops").is_none());

    // With --crops the same range says it made none, and is still complete.
    let cropped = vsift(
        &root,
        &changes(&session, "0", "20000000", &["--crops", "--json"]),
    )?;
    assert_eq!(cropped.status.code(), Some(0));
    let cropped = json(&cropped)?;
    assert_eq!(cropped["status"], "complete");
    assert_eq!(cropped["warnings"], serde_json::json!([]));
    assert_eq!(
        cropped["data"]["crops"],
        serde_json::json!({"limit": 16, "made": 0, "regions_without_crop": 0, "stop": null})
    );
    let readable = vsift(&root, &changes(&session, "0", "20000000", &["--crops"]))?;
    assert_eq!(readable.status.code(), Some(0));
    let text = String::from_utf8(readable.stdout)?;
    assert!(text.contains("Crops: 0 made, at most 16 a call;"), "{text}");

    let events = vsift(
        &root,
        &changes(&session, "0", "20000000", &["--events", "jsonl"]),
    )?;
    assert_eq!(events.status.code(), Some(0));
    assert_eq!(events.stderr, [] as [u8; 0]);
    let text = std::str::from_utf8(&events.stdout)?;
    let lines: Vec<Value> = text
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert_eq!(lines.len(), 1);
    let terminal = &lines[0];
    validate("terminal-event.schema.json", terminal)?;
    validate("operation-response.schema.json", &terminal["result"])?;
    validate(
        "changes-stream-data.schema.json",
        &terminal["result"]["data"],
    )?;
    assert_eq!(terminal["command"], "changes");
    assert_eq!(terminal["result"]["data"]["record_count"], 0);

    let readable = vsift(&root, &changes(&session, "0", "20000000", &[]))?;
    assert_eq!(readable.status.code(), Some(0));
    let text = String::from_utf8(readable.stdout)?;
    assert!(text.starts_with("Screen changes of session "), "{text}");
    assert!(text.contains("Changes on this page: 0\n"), "{text}");
    Ok(())
}

/// A range over an undecodable window is `partial` (exit 0) with the typed
/// gap and the fixed warning; one with a window to analyse, or a change
/// whose frames must be taken, needs the media tools and says which.
#[test]
fn gaps_are_partial_and_analysis_and_frames_need_media_tools() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = open_session(&root)?;
    seed_index(&root, &session)?;
    let output = vsift(
        &root,
        &changes(&session, "60000000", "120000000", &["--json"]),
    )?;
    assert_eq!(output.status.code(), Some(0));
    let value = json(&output)?;
    assert_eq!(value["status"], "partial");
    assert_eq!(
        value["warnings"],
        serde_json::json!([CHANGES_COVERAGE_WARNING])
    );
    assert_eq!(value["coverage"]["truncated"], true);
    assert_eq!(
        value["coverage"]["reasons"],
        serde_json::json!(["undecodable"])
    );
    assert_eq!(
        value["data"]["coverage"]["gaps"][0]["reason"],
        "undecodable"
    );
    assert_eq!(value["data"]["resume"], Value::Null);

    let analysis = vsift(&root, &changes(&session, "0", "150000000", &["--json"]))?;
    assert_eq!(analysis.status.code(), Some(2));
    let value = json(&analysis)?;
    assert_eq!(value["error"]["code"], "MISSING_CAPABILITY");
    assert_eq!(*summary(&value), VISUAL_TOOLS_REMEDIATION);

    let frames = vsift(&root, &changes(&session, "0", "60000000", &["--json"]))?;
    assert_eq!(frames.status.code(), Some(2));
    let value = json(&frames)?;
    assert_eq!(value["error"]["code"], "MISSING_CAPABILITY", "{value}");
    assert_eq!(*summary(&value), EVIDENCE_TOOLS_REMEDIATION);
    Ok(())
}

/// An empty or reversed range and one past the video are typed invalid
/// arguments; an unknown session is a typed failure.
#[test]
fn bad_ranges_and_sessions_are_invalid_arguments() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = open_session(&root)?;
    seed_index(&root, &session)?;
    for (from, to) in [("5", "5"), ("10", "5"), ("150000000", "160000000")] {
        let output = vsift(&root, &changes(&session, from, to, &["--json"]))?;
        assert_eq!(output.status.code(), Some(2), "{from}-{to}");
        assert_eq!(json(&output)?["error"]["code"], "INVALID_ARGUMENT");
    }
    let unknown = vsift(
        &root,
        &changes("ses_ffffffffffffffff", "0", "60000000", &["--json"]),
    )?;
    assert_ne!(unknown.status.code(), Some(0));
    assert!(json(&unknown)?["error"]["code"].is_string());
    Ok(())
}
