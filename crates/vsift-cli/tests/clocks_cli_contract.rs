//! Public CLI contract for `clocks` (time anchoring stage 2, ADR 0029).
//!
//! Sessions are committed directly through the session store, as
//! `ocr_cli_contract` does, so every journey runs everywhere without the media
//! tools or the OCR runner: a screen-text index is committed as the record `ocr`
//! would write, and the binary then reads it exactly as it reads one it made.
//! `--max-frames 0` is what lets a journey run without a runner. Reading frames
//! around a rollover with a real runner is the hosted P16 workflow's check.

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
    ClockReading, Confidence, ConfidenceOrigin, FrameDimensions, FrameRef, ProviderLabel,
    ReadTarget, ScreenText, ScreenTextFrame, ScreenTextIndex, ScreenTextIndexParts, ScreenTextLine,
    ScreenTextOutcome, ScreenTextProfile, ScreenTextProvider, ScreenTextScope, SessionArtifactKind,
    Sha256Hex, SourceId, TextRegion, TimeAnchors, TimeBase,
};
use vsift_infrastructure::{
    FilesystemSessionStore, SourceSnapshot, encode_screen_text_index_record,
    read_supplied_transcript,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-clocks-cli-";
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
            Some("clocks") => validate("clocks-data.schema.json", &value["data"])?,
            Some("session.status") => {
                validate("time-anchor.schema.json", &value["data"]["time_anchor"])?;
            }
            _ => {}
        }
    }
    Ok(value)
}

/// A session over a stand-in source, stated to start at 14:00:00 UTC, or not.
struct Seeded {
    store: FilesystemSessionStore,
    session_id: SessionId,
    source_id: SourceId,
    generation: StorageGeneration,
}

async fn seed(root: &OwnedRoot, stated: Option<&str>) -> Built<Seeded> {
    let source = root.path("stand-in.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomclocks-contract")?;
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
        offset: TranscriptOffset::from_micros(0)?,
        number: NonZeroU32::MIN,
    })?;
    let anchors = match stated {
        Some(text) => TimeAnchors::from_ingest(Some(ClockReading::parse(text)?), &[], now),
        None => TimeAnchors::none(),
    };
    let generation = store.activate_with_transcript(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        now,
        &anchors,
        &revision,
    )?;
    let source_id = snapshot.id().clone();
    drop(snapshot);
    Ok(Seeded {
        store,
        session_id,
        source_id,
        generation,
    })
}

/// Commits the index `ocr` would have written after reading each of `reads`,
/// (seconds into the video, the line read).
fn commit_reads(seeded: &Seeded, reads: &[(u64, String)]) -> TestResult {
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
    let dimensions = FrameDimensions::new(1920, 1080)?;
    let mut frames = Vec::new();
    for (seconds, line) in reads {
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
                    region: TextRegion::new(1800, 1040, 60, 14, dimensions)?,
                    text: ScreenText::parse(line.as_str())?,
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

fn second_text(second: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        second / 3_600 % 24,
        second / 60 % 60,
        second % 60
    )
}

/// A seconds clock read every second for 12 s, 437 s ahead from 6 s on: the
/// shape of a cut, and of two machines a little apart.
fn clock_with_a_jump() -> Vec<(u64, String)> {
    let start = 14 * 3_600;
    (0..12)
        .map(|at| {
            let jump = if at < 6 { 0 } else { 437 };
            (at, second_text(start + at + jump))
        })
        .collect()
}

fn clocks_json(root: &OwnedRoot, arguments: &[&str]) -> Built<Value> {
    let mut all = vec!["clocks", SESSION, "--max-frames", "0"];
    all.extend_from_slice(arguments);
    all.push("--json");
    let output = vsift(root, &all)?;
    json(&output)
}

/// The first segment of `transcript get` at or after 6 s, the gap's position.
fn item_after_the_gap(root: &OwnedRoot) -> Built<Value> {
    let to = (12 * SECOND).to_string();
    let output = vsift(
        root,
        &[
            "transcript",
            "get",
            SESSION,
            "--from",
            "0",
            "--to",
            &to,
            "--json",
        ],
    )?;
    let page: Value = serde_json::from_slice(&output.stdout)?;
    page["data"]["items"]
        .as_array()
        .and_then(|segments| {
            segments.iter().find(|segment| {
                segment["start_us"]
                    .as_u64()
                    .is_some_and(|at| at >= 6 * SECOND)
            })
        })
        .cloned()
        .ok_or_else(|| "no segment after the gap".into())
}

#[tokio::test]
async fn before_ocr_it_says_so_and_changes_nothing() -> TestResult {
    let root = OwnedRoot::new()?;
    let _seeded = seed(&root, Some("2026-10-21T14:00:00Z")).await?;
    let value = clocks_json(&root, &[])?;
    assert_eq!(value["status"], "partial");
    assert_eq!(value["data"]["state"], "not_run");
    assert_eq!(value["data"]["index"], Value::Null);
    assert_eq!(value["coverage"]["reasons"], serde_json::json!(["not_run"]));
    assert_eq!(value["warnings"][0], vsift_contract::CLOCKS_PARTIAL_WARNING);
    Ok(())
}

#[tokio::test]
async fn a_clock_is_found_with_the_frames_that_prove_it_and_never_needs_a_runner() -> TestResult {
    let root = OwnedRoot::new()?;
    let seeded = seed(&root, Some("2026-10-21T14:00:00Z")).await?;
    let reads: Vec<(u64, String)> = (0..12).map(|at| (at, second_text(50_400 + at))).collect();
    commit_reads(&seeded, &reads)?;
    let value = clocks_json(&root, &[])?;
    let clock = &value["data"]["clocks"][0];
    assert_eq!(clock["clock_id"], "c1");
    assert_eq!(clock["kind"], "clock");
    assert_eq!(clock["format"], "24_hour");
    assert_eq!(clock["reads"], 12);
    assert_eq!(clock["first"]["time_us"], 0);
    assert_eq!(clock["first"]["region"]["x"], 1800);
    assert_eq!(clock["confirmed_utc_offset"], Value::Null);
    Ok(())
}

#[tokio::test]
async fn an_observation_is_an_inference_and_changes_no_clock_time() -> TestResult {
    let root = OwnedRoot::new()?;
    let seeded = seed(&root, Some("2026-10-21T14:00:00Z")).await?;
    let reads: Vec<(u64, String)> = (0..12)
        .map(|at| (at, second_text(9 * 3_600 + at)))
        .collect();
    commit_reads(&seeded, &reads)?;
    let before = item_after_the_gap(&root)?;
    let value = clocks_json(&root, &[])?;
    let observation = &value["data"]["observations"][0];
    assert_eq!(observation["inference"], true);
    assert_eq!(observation["anchor"], "operator");
    assert_eq!(observation["fits"], serde_json::json!(["-05:00"]));
    assert_eq!(item_after_the_gap(&root)?, before, "nothing is applied");
    Ok(())
}

/// The planner's ruling (ADR 0029 decision 13): only a person's word moves a
/// clock time, and each item after a gap says which segment it is in.
#[tokio::test]
async fn a_gap_marks_what_follows_and_only_accepting_it_moves_a_clock_time() -> TestResult {
    let root = OwnedRoot::new()?;
    let seeded = seed(&root, Some("2026-10-21T14:00:00Z")).await?;
    commit_reads(&seeded, &clock_with_a_jump())?;
    let before = item_after_the_gap(&root)?;
    assert_eq!(before["clock_time"].get("segment"), None);

    let found = clocks_json(&root, &[])?;
    let jump = &found["data"]["clocks"][0]["jumps"][0];
    assert_eq!(jump["kind"], "gap");
    assert_eq!(jump["gap_id"], "g1");
    let summary = &found["data"]["time_anchor"];
    assert_eq!(summary["segments"][1]["state"], "detected");
    assert_eq!(summary["segments"][1]["adjusted_by_us"], 0);

    let detected = item_after_the_gap(&root)?;
    assert_eq!(detected["clock_time"]["segment"], 1);
    assert_eq!(
        detected["clock_time"]["start"], before["clock_time"]["start"],
        "a detected gap changes no value"
    );

    let accepted = clocks_json(&root, &["--accept-gap", "g1"])?;
    assert_eq!(
        accepted["data"]["time_anchor"]["segments"][1]["state"],
        "accepted"
    );
    let moved = item_after_the_gap(&root)?;
    assert_ne!(moved["clock_time"]["start"], before["clock_time"]["start"]);
    assert_eq!(moved["clock_time"]["segment"], 1);

    let again = clocks_json(&root, &[])?;
    assert_eq!(
        again["data"]["time_anchor"]["segments"][1]["state"], "accepted",
        "an accepted gap stays accepted"
    );
    Ok(())
}

#[tokio::test]
async fn a_held_misread_is_reported_as_an_excursion_with_both_explanations() -> TestResult {
    let root = OwnedRoot::new()?;
    let seeded = seed(&root, Some("2026-10-21T14:00:00Z")).await?;
    let mut reads: Vec<(u64, String)> = (0..12).map(|at| (at, second_text(50_400 + at))).collect();
    for read in reads.iter_mut().filter(|read| (4..7).contains(&read.0)) {
        read.1 = second_text(50_400 + read.0 + 300);
    }
    commit_reads(&seeded, &reads)?;
    let value = clocks_json(&root, &[])?;
    let clock = &value["data"]["clocks"][0];
    assert_eq!(clock["jumps"], serde_json::json!([]));
    assert_eq!(clock["excursions_total"], 1);
    let excursion = &clock["excursions"][0];
    assert_eq!(excursion["reads"], 3);
    assert_eq!(
        excursion["explanations"],
        serde_json::json!(["misread", "another_clock"])
    );
    assert_eq!(value["data"]["time_anchor"].get("segments"), None);
    Ok(())
}

#[tokio::test]
async fn a_zone_only_the_user_states_makes_a_clock_an_anchor() -> TestResult {
    let root = OwnedRoot::new()?;
    let seeded = seed(&root, Some("2026-10-21T19:00:00Z")).await?;
    let reads: Vec<(u64, String)> = (0..12)
        .map(|at| (at, second_text(14 * 3_600 + at)))
        .collect();
    commit_reads(&seeded, &reads)?;
    let found = clocks_json(&root, &[])?;
    let anchors = &found["data"]["time_anchor"]["anchors"];
    assert_eq!(anchors.as_array().map(Vec::len), Some(1));

    let confirmed = clocks_json(&root, &["--screen-zone", "c1=-05:00"])?;
    assert_eq!(
        confirmed["data"]["clocks"][0]["confirmed_utc_offset"],
        "-05:00"
    );
    let anchors = &confirmed["data"]["time_anchor"]["anchors"];
    assert_eq!(anchors[1]["source"], "screen_clock_confirmed");
    assert_eq!(anchors[1]["confidence"], "medium");
    assert_eq!(anchors[1]["clock_id"], "c1");
    assert_eq!(confirmed["data"]["time_anchor"]["source"], "operator");
    assert_eq!(
        confirmed["data"]["time_anchor"]["consistency"],
        "consistent"
    );
    Ok(())
}

#[tokio::test]
async fn a_clock_of_no_date_with_nothing_to_date_it_is_refused_with_its_remediation() -> TestResult
{
    let root = OwnedRoot::new()?;
    let seeded = seed(&root, None).await?;
    let reads: Vec<(u64, String)> = (0..12)
        .map(|at| (at, second_text(14 * 3_600 + at)))
        .collect();
    commit_reads(&seeded, &reads)?;
    let output = vsift(
        &root,
        &[
            "clocks",
            SESSION,
            "--max-frames",
            "0",
            "--screen-zone",
            "c1=-05:00",
            "--json",
        ],
    )?;
    assert!(!output.status.success());
    let value = json(&output)?;
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(
        value["error"]["remediation"][0]["summary"],
        vsift_contract::CLOCK_RULES_REMEDIATION
    );
    Ok(())
}

#[tokio::test]
async fn statements_about_what_the_session_does_not_have_are_refused() -> TestResult {
    let root = OwnedRoot::new()?;
    let seeded = seed(&root, Some("2026-10-21T14:00:00Z")).await?;
    commit_reads(&seeded, &clock_with_a_jump())?;
    for statement in [["--accept-gap", "g9"], ["--screen-zone", "c7=+01:00"]] {
        let mut arguments = vec!["clocks", SESSION, "--max-frames", "0"];
        arguments.extend_from_slice(&statement);
        arguments.push("--json");
        let output = vsift(&root, &arguments)?;
        assert!(!output.status.success(), "{statement:?}");
        assert_eq!(json(&output)?["error"]["code"], "INVALID_ARGUMENT");
    }
    Ok(())
}

#[test]
fn bounds_and_forms_are_checked_before_anything_runs() -> TestResult {
    let root = OwnedRoot::new()?;
    for arguments in [
        vec!["clocks", SESSION, "--max-frames", "201", "--json"],
        vec![
            "clocks",
            SESSION,
            "--screen-zone",
            "c1=America/New_York",
            "--json",
        ],
        vec!["clocks", SESSION, "--screen-zone", "c1", "--json"],
        vec!["clocks", SESSION, "--accept-gap", "c1", "--json"],
    ] {
        let output = vsift(&root, &arguments)?;
        assert!(!output.status.success(), "{arguments:?}");
        assert_eq!(json(&output)?["error"]["code"], "INVALID_ARGUMENT");
    }
    Ok(())
}

#[tokio::test]
async fn a_rollover_to_read_needs_a_runner_and_says_so() -> TestResult {
    let root = OwnedRoot::new()?;
    let seeded = seed(&root, Some("2026-10-21T14:00:00Z")).await?;
    let reads: Vec<(u64, String)> = (0..30)
        .map(|step| {
            let shown = 14 * 3_600 + step * 10;
            (
                step * 10,
                format!("{:02}:{:02}", shown / 3_600, shown / 60 % 60),
            )
        })
        .collect();
    commit_reads(&seeded, &reads)?;
    let output = vsift(&root, &["clocks", SESSION, "--json"])?;
    assert!(!output.status.success());
    let value = json(&output)?;
    assert_eq!(value["error"]["code"], "MISSING_CAPABILITY");
    assert_eq!(
        value["error"]["remediation"][0]["summary"],
        vsift_contract::OCR_RUNNER_REMEDIATION
    );
    let pending = clocks_json(&root, &[])?;
    assert_eq!(pending["status"], "partial");
    assert!(
        pending["data"]["reads_pending"]
            .as_u64()
            .is_some_and(|reads| reads > 0)
    );
    Ok(())
}

#[tokio::test]
async fn a_session_that_never_ran_clocks_gains_no_member() -> TestResult {
    let root = OwnedRoot::new()?;
    let seeded = seed(&root, Some("2026-10-21T14:00:00Z")).await?;
    commit_reads(&seeded, &clock_with_a_jump())?;
    let output = vsift(&root, &["session", "status", SESSION, "--json"])?;
    let value = json(&output)?;
    let summary = &value["data"]["time_anchor"];
    assert_eq!(summary.get("segments"), None);
    assert_eq!(summary["anchors"].as_array().map(Vec::len), Some(1));
    assert_eq!(summary["anchors"][0].get("clock_id"), None);
    assert_eq!(
        item_after_the_gap(&root)?["clock_time"].get("segment"),
        None
    );
    Ok(())
}

#[tokio::test]
async fn a_person_reads_what_a_gap_does_and_does_not_do() -> TestResult {
    let root = OwnedRoot::new()?;
    let seeded = seed(&root, Some("2026-10-21T14:00:00Z")).await?;
    commit_reads(&seeded, &clock_with_a_jump())?;
    let output = vsift(&root, &["clocks", SESSION, "--max-frames", "0"])?;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout)?;
    assert!(text.contains("Clock c1"), "{text}");
    assert!(text.contains("zone unknown"), "{text}");
    assert!(
        text.contains("changes no clock time until a person accepts it"),
        "{text}"
    );
    Ok(())
}
