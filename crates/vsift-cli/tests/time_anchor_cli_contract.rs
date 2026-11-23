//! Public CLI contract for time anchoring (0.3.0, ADR 0029): `ingest
//! --recorded-at`, the `time_anchor` summary of `ingest` and `session status`,
//! and the `clock_time` of every timestamped item.
//!
//! Every journey runs without `FFprobe`: the binary runs with an isolated
//! per-user base and an empty `PATH`, so a plain ingest finds no verified
//! media tools and reads no container tag, and the transcript journeys commit
//! their session directly through the store, as `search_cli_contract` does.
//! Reading real container tags is the opt-in hosted check (ADR 0029).

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
use serde_json::{Value, json};
use vsift::{
    DurabilityRequirement, MediaTime, OperationId, SessionId, StorageGeneration, TranscriptOffset,
};
use vsift_application::{
    ForegroundSessionPort, ImportedRevisionRequest, InitializeSessionStorage,
    InitializeSessionStorageRequest, build_imported_revision,
};
use vsift_domain::{AnchorSource, ClockReading, TimeAnchor, TimeAnchors};
use vsift_infrastructure::{FilesystemSessionStore, SourceSnapshot, read_supplied_transcript};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-time-anchor-cli-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
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

/// Runs `vsift` with an isolated per-user base, no `PATH` and the root's
/// session store.
fn vsift(root: &OwnedRoot, arguments: &[&str]) -> Built<Output> {
    let base = root.path("user");
    Ok(Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("XDG_CACHE_HOME", &base)
        .env("HOME", &base)
        .env("PATH", "")
        .arg("--session-root")
        .arg(root.sessions())
        .args(arguments)
        .output()?)
}

fn json(output: &Output) -> Built<Value> {
    let value: Value = serde_json::from_slice(&output.stdout)?;
    validate("operation-response.schema.json", &value)?;
    Ok(value)
}

fn stand_in_source(root: &OwnedRoot) -> Built<PathBuf> {
    let source = root.path("recording.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomtime-anchor-contract")?;
    Ok(source)
}

fn ingest(root: &OwnedRoot, extra: &[&str]) -> Built<Value> {
    let source = stand_in_source(root)?;
    let source = source.to_str().ok_or("non-UTF-8 temporary path")?;
    let mut arguments = vec!["ingest", source];
    arguments.extend_from_slice(extra);
    arguments.push("--json");
    json(&vsift(root, &arguments)?)
}

#[test]
fn a_stated_recording_time_is_the_sessions_exact_anchor() -> TestResult {
    let root = OwnedRoot::new()?;
    let opened = ingest(&root, &["--recorded-at", "2026-10-21T14:03:12.5+01:00"])?;
    assert_eq!(opened["status"], "complete", "{opened}");
    validate("ingest-data.schema.json", &opened["data"])?;
    let expected = json!({
        "consistency": "single",
        "source": "operator",
        "confidence": "exact",
        "recording_start": {
            "utc": "2026-10-21T13:03:12.500000Z",
            "local": "2026-10-21T14:03:12.500000",
            "utc_offset": "+01:00"
        },
        "anchors": [{
            "source": "operator",
            "confidence": "exact",
            "media_time_us": 0,
            "time": {
                "utc": "2026-10-21T13:03:12.500000Z",
                "local": "2026-10-21T14:03:12.500000",
                "utc_offset": "+01:00"
            }
        }]
    });
    assert_eq!(opened["data"]["time_anchor"], expected);
    let id = opened["data"]["session_id"]
        .as_str()
        .ok_or("no session id")?;

    let status = json(&vsift(&root, &["session", "status", id, "--json"])?)?;
    assert_eq!(status["data"]["time_anchor"], expected, "{status}");
    Ok(())
}

#[test]
fn a_local_time_with_no_zone_is_said_to_have_none() -> TestResult {
    let root = OwnedRoot::new()?;
    let opened = ingest(&root, &["--recorded-at", "2026-10-21T14:03:12"])?;
    validate("ingest-data.schema.json", &opened["data"])?;
    assert_eq!(
        opened["data"]["time_anchor"]["recording_start"],
        json!({"utc": null, "local": "2026-10-21T14:03:12.000000", "utc_offset": null})
    );
    Ok(())
}

/// Without `--recorded-at` and with no verified media tools, the result is
/// exactly what it was before time anchoring: no `time_anchor` member.
#[test]
fn a_plain_ingest_without_tools_has_no_time_anchor() -> TestResult {
    let root = OwnedRoot::new()?;
    let opened = ingest(&root, &[])?;
    assert_eq!(opened["status"], "complete", "{opened}");
    assert!(opened["data"].get("time_anchor").is_none(), "{opened}");
    let id = opened["data"]["session_id"]
        .as_str()
        .ok_or("no session id")?;
    let status = json(&vsift(&root, &["session", "status", id, "--json"])?)?;
    assert!(status["data"].get("time_anchor").is_none(), "{status}");
    Ok(())
}

#[test]
fn an_unreadable_recorded_at_is_refused_before_anything_is_read() -> TestResult {
    let root = OwnedRoot::new()?;
    for value in [
        "yesterday",
        "2026-10-21 14:03:12",
        "2026-02-30T00:00:00Z",
        "2026-10-21T14:03:12+15:00",
    ] {
        let output = vsift(
            &root,
            &["ingest", "missing.mp4", "--recorded-at", value, "--json"],
        )?;
        let refused = json(&output)?;
        assert_eq!(
            refused["error"]["code"], "INVALID_ARGUMENT",
            "{value}: {refused}"
        );
    }
    assert!(
        !root.sessions().exists(),
        "a refused ingest created the session root"
    );
    Ok(())
}

/// Commits one open session with the F10 import (+500 ms) and `anchors`.
async fn seed_anchored_session(root: &OwnedRoot, anchors: &TimeAnchors) -> Built<String> {
    let source = stand_in_source(root)?;
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
    let supplied = read_supplied_transcript(&repository("fixtures/corpus/transcripts/F10.srt"))?;
    let revision = build_imported_revision(ImportedRevisionRequest {
        session_id: &session_id,
        source_id: snapshot.id(),
        source_duration: MediaTime::from_micros(12 * SECOND),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(500_000)?,
        number: NonZeroU32::MIN,
    })?;
    store.activate_with_transcript(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        now,
        anchors,
        &revision,
    )?;
    Ok(session_id.as_str().to_owned())
}

fn conflicting_anchors() -> Built<TimeAnchors> {
    let start = MediaTime::from_micros(0);
    Ok(TimeAnchors::new(vec![
        TimeAnchor::new(
            AnchorSource::QuicktimeCreationDate,
            start,
            ClockReading::parse("2026-10-21T14:00:00+0100")?,
        ),
        TimeAnchor::new(
            AnchorSource::ContainerCreationTime,
            start,
            ClockReading::parse("2026-10-21T15:30:00Z")?,
        ),
    ])?)
}

/// F10's first cue lies at 1-4 s after the +500 ms offset: by the recorder's
/// tag (14:00:00 at +01:00) it was said from 14:00:01 to 14:00:04 local.
#[tokio::test]
async fn transcript_and_search_items_carry_their_clock_times() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_anchored_session(&root, &conflicting_anchors()?).await?;
    let first_cue = json!({
        "source": "quicktime_creationdate",
        "confidence": "high",
        "start": {"utc": "2026-10-21T13:00:01.000000Z", "local": "2026-10-21T14:00:01.000000", "utc_offset": "+01:00"},
        "end": {"utc": "2026-10-21T13:00:04.000000Z", "local": "2026-10-21T14:00:04.000000", "utc_offset": "+01:00"}
    });

    let status = json(&vsift(&root, &["session", "status", &session, "--json"])?)?;
    assert_eq!(status["data"]["time_anchor"]["consistency"], "conflicting");

    let page = json(&vsift(
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
    )?)?;
    validate("transcript-get-data.schema.json", &page["data"])?;
    assert_eq!(page["data"]["items"][0]["clock_time"], first_cue, "{page}");

    let brief = json(&vsift(
        &root,
        &[
            "transcript",
            "get",
            &session,
            "--from",
            "0",
            "--to",
            "12000000",
            "--view",
            "brief",
            "--json",
        ],
    )?)?;
    validate("transcript-get-brief-data.schema.json", &brief["data"])?;
    assert_eq!(brief["data"]["items"][0]["clock_time"], first_cue);

    let search = json(&vsift(
        &root,
        &["search", &session, "--query", "sidecar", "--json"],
    )?)?;
    validate("search-data.schema.json", &search["data"])?;
    assert_eq!(
        search["data"]["items"][0]["clock_time"], first_cue,
        "{search}"
    );

    let streamed = vsift(
        &root,
        &[
            "search", &session, "--query", "sidecar", "--events", "jsonl",
        ],
    )?;
    let text = std::str::from_utf8(&streamed.stdout)?;
    let record: Value = serde_json::from_str(text.lines().next().ok_or("empty stream")?)?;
    validate("transcript-segment.schema.json", &record["record"])?;
    assert_eq!(record["record"]["clock_time"], first_cue);
    Ok(())
}

/// A session without anchors answers transcript reads as before.
#[tokio::test]
async fn items_of_a_session_without_anchors_have_no_clock_time() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = seed_anchored_session(&root, &TimeAnchors::none()).await?;
    let page = json(&vsift(
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
    )?)?;
    let items = page["data"]["items"].as_array().ok_or("no items")?;
    assert_ne!(*items, [] as [Value; 0]);
    assert!(
        items.iter().all(|item| item.get("clock_time").is_none()),
        "{page}"
    );
    Ok(())
}
