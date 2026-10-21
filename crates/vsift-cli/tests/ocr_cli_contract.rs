//! Public CLI contract for `ocr` and the `screen_text` member of `search`
//! (OCR, ADR 0030).
//!
//! Sessions are committed directly through the session store, as
//! `search_cli_contract` does, so every journey runs everywhere without the
//! media tools or the OCR runner: a screen-text index is committed as the
//! record `ocr` would write, and the binary then reads it exactly as it reads
//! one it made. Reading frames with a real runner is the hosted P16 workflow's
//! check; the runner adapter's own behaviour is pinned by
//! `vsift-infrastructure`'s `ocr_runner` tests against a stand-in.

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
use vsift::{
    DurabilityRequirement, MediaTime, OperationId, SessionId, StorageGeneration, TranscriptOffset,
};
use vsift_application::{
    ForegroundSessionPort, ImportedRevisionRequest, InitializeSessionStorage,
    InitializeSessionStorageRequest, build_imported_revision, screen_text_index_id,
    screen_text_line_id,
};
use vsift_domain::{
    Confidence, ConfidenceOrigin, FrameDimensions, FrameRef, ProviderLabel, ReadTarget, ScreenText,
    ScreenTextFrame, ScreenTextIndex, ScreenTextIndexParts, ScreenTextLine, ScreenTextOutcome,
    ScreenTextProfile, ScreenTextProvider, ScreenTextScope, SessionArtifactKind, Sha256Hex,
    SourceId, TextRegion, TimeBase,
};
use vsift_infrastructure::{
    FilesystemSessionStore, SourceSnapshot, encode_screen_text_index_record,
    read_supplied_transcript,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-ocr-cli-";
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

/// Runs `vsift` with an isolated per-user base, the root's session store and
/// a `PATH` that holds no tool at all.
fn vsift(root: &OwnedRoot, arguments: &[&str]) -> Built<Output> {
    let base = root.path("user");
    let empty = root.path("empty-path");
    fs::create_dir_all(&empty)?;
    Ok(Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .env("PATH", &empty)
        .arg("--session-root")
        .arg(root.sessions())
        .args(arguments)
        .output()?)
}

/// Parses a `--json` result and validates its envelope and data.
fn json(output: &Output) -> Built<Value> {
    assert!(output.stderr.is_empty(), "a JSON command wrote to stderr");
    let value: Value = serde_json::from_slice(&output.stdout)?;
    validate("operation-response.schema.json", &value)?;
    if value["error"].is_null() {
        match value["command"].as_str() {
            Some("search") => validate("search-data.schema.json", &value["data"])?,
            Some("ocr") => validate("screen-text-data.schema.json", &value["data"])?,
            Some("ocr.search") => validate("ocr-search-data.schema.json", &value["data"])?,
            _ => {}
        }
    }
    Ok(value)
}

/// The store, the session and its source, and the generation after its
/// transcript (the F10 import) was committed.
struct Seeded {
    store: FilesystemSessionStore,
    session_id: SessionId,
    source_id: SourceId,
    generation: StorageGeneration,
}

async fn seed_session(root: &OwnedRoot) -> Built<Seeded> {
    seed(root, true).await
}

/// A session over a stand-in source, with the F10 import as its transcript or,
/// for a silent recording, without any.
async fn seed(root: &OwnedRoot, with_transcript: bool) -> Built<Seeded> {
    let source = root.path("stand-in.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomocr-contract")?;
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
    let activation = OperationId::parse("op_2222222222222222")?;
    let generation = if with_transcript {
        let supplied =
            read_supplied_transcript(&repository("fixtures/corpus/transcripts/F10.srt"))?;
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
            &activation,
            StorageGeneration::INITIAL,
            now,
            &revision,
        )?
    } else {
        store.activate_source(&snapshot, &activation, StorageGeneration::INITIAL, now)?
    };
    let source_id = snapshot.id().clone();
    drop(snapshot);
    Ok(Seeded {
        store,
        session_id,
        source_id,
        generation,
    })
}

/// Commits the index `ocr` would have written after reading the frames at
/// 2 s and 3 s, which show the dialog's title in one place.
fn commit_index(seeded: &Seeded) -> TestResult {
    let digest = |fill: char| Sha256Hex::parse(fill.to_string().repeat(64));
    let scope = ScreenTextScope {
        session_id: seeded.session_id.clone(),
        source_id: seeded.source_id.clone(),
        stream_index: 0,
        profile: ScreenTextProfile::PpOcrV5LatinV1,
        provider: ScreenTextProvider {
            runner_sha256: digest('1')?,
            runner_version: ProviderLabel::parse("0.1.0")?,
            runtime_version: ProviderLabel::parse("1.31.0")?,
            detector_sha256: digest('2')?,
            recognizer_sha256: digest('3')?,
        },
    };
    let dimensions = FrameDimensions::new(1280, 720)?;
    let mut frames = Vec::new();
    for seconds in [2_u64, 3] {
        let pts = i64::try_from(seconds * 25)?;
        frames.push(ScreenTextFrame {
            frame: FrameRef {
                stream_index: 0,
                time_base: TimeBase::new(1, 25)?,
                pts,
                time: MediaTime::from_micros(seconds * SECOND),
                dimensions,
            },
            targets: vec![ReadTarget {
                time: MediaTime::from_micros(seconds * SECOND),
                candidate: None,
            }],
            outcome: ScreenTextOutcome::Read {
                lines: vec![ScreenTextLine {
                    id: screen_text_line_id(&scope, pts, 1)?,
                    region: TextRegion::new(400, 280, 480, 40, dimensions)?,
                    text: ScreenText::parse("Dialog R-17")?,
                    confidence: Confidence::provider_score(
                        9_640,
                        ConfidenceOrigin::ProviderUncalibrated,
                    )?,
                }],
                truncated: false,
            },
        });
    }
    let number = NonZeroU32::MIN;
    let index = ScreenTextIndex::new(ScreenTextIndexParts {
        id: screen_text_index_id(&scope, number)?,
        revision: number,
        scope,
        period_micros: SECOND,
        frames,
        unresolved: Vec::new(),
    })?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    seeded.store.publish_artifact(
        &seeded.session_id,
        &OperationId::parse("op_3333333333333333")?,
        seeded.generation,
        SessionArtifactKind::ScreenTextIndexRecord,
        &encode_screen_text_index_record(&index)?,
        now,
    )?;
    Ok(())
}

/// With no runner given, configured or on `PATH`, `ocr` fails with a missing
/// capability and the fixed remediation, before touching media or storage.
#[tokio::test]
async fn ocr_without_a_runner_is_a_missing_capability_with_its_remediation() -> TestResult {
    let root = OwnedRoot::new()?;
    let seeded = seed_session(&root).await?;
    let output = vsift(&root, &["ocr", seeded.session_id.as_str(), "--json"])?;
    assert_eq!(output.status.code(), Some(2));
    let value = json(&output)?;
    assert_eq!(value["command"], "ocr");
    assert_eq!(value["status"], "failed");
    assert_eq!(value["error"]["code"], "MISSING_CAPABILITY");
    assert_eq!(
        value["error"]["remediation"][0]["summary"],
        vsift_contract::OCR_RUNNER_REMEDIATION
    );
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let (index, _) = seeded
        .store
        .read_screen_text_index(&seeded.session_id, now)?;
    assert_eq!(index, None, "nothing was committed");
    Ok(())
}

/// The bounds of `--period-us`, `--max-frames` and `--threads` are the
/// parser's: a value outside them is a usage error before anything runs.
#[test]
fn ocr_bounds_are_checked_before_anything_runs() -> TestResult {
    let root = OwnedRoot::new()?;
    for (flag, value) in [
        ("--period-us", "999999"),
        ("--period-us", "600000001"),
        ("--max-frames", "0"),
        ("--max-frames", "201"),
        ("--threads", "9"),
    ] {
        let output = vsift(&root, &["ocr", SESSION, flag, value, "--json"])?;
        assert_eq!(output.status.code(), Some(2), "{flag} {value}");
        let value: Value = serde_json::from_slice(&output.stdout)?;
        validate("operation-response.schema.json", &value)?;
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    }
    Ok(())
}

/// `setup configure ocr-runner` records the runner apart from the other
/// tools: after it, `ocr` no longer reports a missing runner (it goes on to
/// need the media tools, which this `PATH` does not hold).
#[tokio::test]
async fn a_configured_runner_is_used_by_ocr() -> TestResult {
    let root = OwnedRoot::new()?;
    let seeded = seed_session(&root).await?;
    let runner = root.path("vsift-ocr-runner.exe");
    fs::write(&runner, b"not a runner; ocr fails before it would run it")?;
    let runner_text = runner.to_str().ok_or("path")?;
    let configured = json(&vsift(
        &root,
        &[
            "setup",
            "configure",
            "ocr-runner",
            "--executable",
            runner_text,
            "--json",
        ],
    )?)?;
    assert_eq!(configured["status"], "complete");
    assert_eq!(configured["data"]["dependency"], "ocr_runner");
    let output = vsift(&root, &["ocr", seeded.session_id.as_str(), "--json"])?;
    let value = json(&output)?;
    assert_eq!(value["status"], "failed");
    let summary = value["error"]["remediation"][0]["summary"]
        .as_str()
        .ok_or("no remediation")?;
    assert_ne!(summary, vsift_contract::OCR_RUNNER_REMEDIATION);
    assert!(summary.contains("FFmpeg"), "{value}");
    Ok(())
}

/// Before `ocr` has read a session, a search first page says so; a page a
/// cursor continues carries no `screen_text` member at all.
#[tokio::test]
async fn a_search_first_page_says_screen_text_is_not_run_and_continued_pages_omit_it() -> TestResult
{
    let root = OwnedRoot::new()?;
    let seeded = seed_session(&root).await?;
    let session = seeded.session_id.as_str();
    let first = json(&vsift(
        &root,
        &[
            "search",
            session,
            "--query",
            "synthetic",
            "--limit",
            "1",
            "--json",
        ],
    )?)?;
    let part = &first["data"]["screen_text"];
    assert_eq!(part["state"], "not_run");
    assert_eq!(part["hits"], serde_json::json!([]));
    assert_eq!(first["status"], "complete");
    let cursor = first["data"]["next_cursor"]
        .as_str()
        .ok_or("no cursor")?
        .to_owned();
    let next = json(&vsift(
        &root,
        &[
            "search",
            session,
            "--query",
            "synthetic",
            "--limit",
            "1",
            "--cursor",
            &cursor,
            "--json",
        ],
    )?)?;
    assert!(next["data"].get("screen_text").is_none(), "{next}");
    Ok(())
}

/// After `ocr` committed an index, search finds the dialog's title on screen
/// with its frame, time and region, next to the spoken words.
#[tokio::test]
async fn search_finds_text_read_off_the_screen_with_its_frame_time_and_region() -> TestResult {
    let root = OwnedRoot::new()?;
    let seeded = seed_session(&root).await?;
    commit_index(&seeded)?;
    let value = json(&vsift(
        &root,
        &[
            "search",
            seeded.session_id.as_str(),
            "--query",
            "R-17",
            "--json",
        ],
    )?)?;
    assert_eq!(value["status"], "complete");
    assert_eq!(
        value["data"]["items"][0]["text"],
        "Dialog R-17 is displayed now."
    );
    let part = &value["data"]["screen_text"];
    // No visual index: the candidates are not known, so the part is partial.
    assert_eq!(part["state"], "partial");
    assert_eq!(part["coverage"]["candidates_complete"], false);
    assert_eq!(part["total_hits"], 1);
    let hit = &part["hits"][0];
    assert_eq!(hit["text"], "Dialog R-17");
    assert_eq!(hit["match"], "phrase");
    assert_eq!(hit["frame"]["pts"], 50);
    assert_eq!(hit["frame"]["time_base_denominator"], 25);
    assert_eq!(hit["frame"]["time_us"], 2 * SECOND);
    assert_eq!(hit["last_time_us"], 3 * SECOND);
    assert_eq!(hit["frames"], 2);
    assert_eq!(
        hit["region"],
        serde_json::json!({"x": 400, "y": 280, "width": 480, "height": 40})
    );

    let human = vsift(
        &root,
        &["search", seeded.session_id.as_str(), "--query", "R-17"],
    )?;
    assert_eq!(human.status.code(), Some(0));
    let text = String::from_utf8(human.stdout)?;
    assert!(text.contains("On-screen hits: 1 of 1"), "{text}");
    assert!(text.contains("region: 400,280 480x40"), "{text}");
    assert!(text.contains("  | Dialog R-17"), "{text}");
    Ok(())
}

/// On a session without any transcript (a silent recording), `search` refuses
/// as it always has, and `ocr search` finds and pages what `ocr` read.
#[tokio::test]
async fn ocr_search_finds_on_screen_text_without_a_transcript() -> TestResult {
    let root = OwnedRoot::new()?;
    let seeded = seed(&root, false).await?;
    let session = seeded.session_id.as_str().to_owned();
    let before = json(&vsift(
        &root,
        &["ocr", "search", &session, "--query", "R-17", "--json"],
    )?)?;
    assert_eq!(before["command"], "ocr.search");
    assert_eq!(before["status"], "partial");
    assert_eq!(before["data"]["state"], "not_run");
    commit_index(&seeded)?;
    let refused = vsift(&root, &["search", &session, "--query", "R-17", "--json"])?;
    assert_eq!(refused.status.code(), Some(2));
    let page = json(&vsift(
        &root,
        &[
            "ocr",
            "search",
            &session,
            "--query",
            "dialog r 17",
            "--json",
        ],
    )?)?;
    assert_eq!(page["data"]["total_hits"], 1);
    let hit = &page["data"]["hits"][0];
    assert_eq!(hit["text"], "Dialog R-17");
    assert_eq!(hit["frame"]["time_us"], 2 * SECOND);
    assert_eq!(hit["frames"], 2);
    assert_eq!(page["data"]["next_cursor"], Value::Null);

    let human = vsift(&root, &["ocr", "search", &session, "--query", "R-17"])?;
    assert_eq!(human.status.code(), Some(0));
    let text = String::from_utf8(human.stdout)?;
    assert!(text.contains("On-screen search of session"), "{text}");
    assert!(text.contains("  | Dialog R-17"), "{text}");

    let forged = vsift(
        &root,
        &[
            "ocr", "search", &session, "--query", "R-17", "--cursor", "v1.bad", "--json",
        ],
    )?;
    assert_eq!(forged.status.code(), Some(2));
    let refusal: Value = serde_json::from_slice(&forged.stdout)?;
    assert_eq!(refusal["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(
        refusal["error"]["remediation"][0]["summary"],
        vsift_contract::OCR_SEARCH_CURSOR_REMEDIATION
    );
    Ok(())
}
