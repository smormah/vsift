//! Opt-in cumulative P08 checkpoint: transcript search on the supplied-
//! transcript path (stage `p08_search_supplied`).
//!
//! The journey drives the compiled `vsift` binary exactly as a headless agent
//! would, with an isolated per-user base and an empty `PATH`: `FFmpeg` and
//! `FFprobe` are registered with `setup configure`, and whisper.cpp is absent.
//! It imports the F10 fixture video with `fixtures/corpus/transcripts/F10.srt`
//! and the explicit +500 ms offset, then searches for `R-17` and for the
//! spoken spelling `dialog r 17`. Both must find exactly the segment F10-E01's
//! frozen truth window cites, as a phrase, with complete coverage from the
//! supplied transcript; the `--events jsonl` stream must carry the same
//! record, and `transcript get` over the truth window must cite the same
//! segment. Visual candidates are checked by the separate
//! `p08_candidates_e2e` checkpoint.
//!
//! `cargo test -p vsift-cli --locked --test p08_search_e2e -- --ignored --nocapture`
//!
//! The run writes a bounded report to `.vsift/e2e-runs/p08-<run-id>/report.json`.
//! A stage that cannot run here is `blocked`, never `passed`, and the test
//! fails unless it passed.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::{Value, json};
use vsift_infrastructure::{ExecutableResolver, TrustedExecutable};

type TestResult = Result<(), Box<dyn Error>>;

/// Upper bound for one CLI invocation; a prompt or hang is killed and fails.
const CLI_DEADLINE: Duration = Duration::from_secs(60);
const OWNED_PREFIX: &str = "vsift-p08-e2e-";
const MAX_DIAGNOSTIC_CHARS: usize = 240;
const OFFSET_US: &str = "500000";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const MISSING_MEDIA_TOOLS: &str = "FFmpeg or FFprobe is not on PATH; install or locate trusted builds, then run setup configure ffmpeg|ffprobe --executable <absolute-path>";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

/// A temporary directory this checkpoint created and alone may remove.
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
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

enum StageStop {
    Failed(String),
    Blocked(String),
}

impl<E: Error> From<E> for StageStop {
    fn from(error: E) -> Self {
        Self::Failed(bounded(&error.to_string()))
    }
}

fn bounded(text: &str) -> String {
    text.chars().take(MAX_DIAGNOSTIC_CHARS).collect()
}

fn ensure(condition: bool, expectation: &str) -> Result<(), StageStop> {
    if condition {
        Ok(())
    } else {
        Err(StageStop::Failed(bounded(expectation)))
    }
}

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct MediaTools {
    ffmpeg: TrustedExecutable,
    ffprobe: TrustedExecutable,
}

impl MediaTools {
    fn discover() -> Option<Self> {
        let resolver = ExecutableResolver::from_current_path();
        Some(Self {
            ffmpeg: resolver.resolve(OsStr::new("ffmpeg")).ok()?,
            ffprobe: resolver.resolve(OsStr::new("ffprobe")).ok()?,
        })
    }
}

/// F10-E01's frozen truth window, in source microseconds.
fn f10_truth() -> Result<(u64, u64), Box<dyn Error>> {
    let manifest: Value = serde_json::from_slice(&fs::read(
        repository().join("fixtures/corpus/manifest.json"),
    )?)?;
    let event = manifest["fixtures"]
        .as_array()
        .and_then(|fixtures| fixtures.iter().find(|fixture| fixture["id"] == "F10"))
        .and_then(|fixture| fixture["events"].as_array())
        .and_then(|events| events.iter().find(|event| event["id"] == "F10-E01"))
        .ok_or("F10-E01 missing from the manifest")?;
    Ok((
        event["start_us"].as_u64().ok_or("start_us missing")?,
        event["end_us"].as_u64().ok_or("end_us missing")?,
    ))
}

/// A bounded `vsift` invocation with isolated per-user state and no ambient `PATH`.
fn vsift(base: &Path) -> Result<Command, StageStop> {
    let mut command = Command::cargo_bin("vsift")?;
    command
        .env("LOCALAPPDATA", base)
        .env("XDG_CONFIG_HOME", base)
        .env("HOME", base)
        .env("PATH", "")
        .arg("--session-root")
        .arg(base.join("sessions"))
        .timeout(CLI_DEADLINE);
    Ok(command)
}

fn run_json(command: &mut Command) -> Result<(Option<i32>, Value), StageStop> {
    let output = command.output()?;
    ensure(
        output.stderr.is_empty(),
        "a JSON command wrote to standard error",
    )?;
    Ok((
        output.status.code(),
        serde_json::from_slice(&output.stdout)?,
    ))
}

/// Resolves sibling `$ref`s by their published identifier to the local copy.
struct PublishedSchemas;

impl Retrieve for PublishedSchemas {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let name = uri
            .as_str()
            .strip_prefix(SCHEMA_BASE)
            .filter(|name| !name.contains(['/', '\\']))
            .ok_or_else(|| format!("unpublished schema reference: {uri}"))?;
        Ok(serde_json::from_slice(&fs::read(
            repository().join("schemas/v1").join(name),
        )?)?)
    }
}

/// Requires `instance` to conform to the published v1 schema `schema`.
fn conforms(schema: &str, instance: &Value) -> Result<(), StageStop> {
    let definition: Value =
        serde_json::from_slice(&fs::read(repository().join("schemas/v1").join(schema))?)?;
    let validator = jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&definition)
        .map_err(|error| StageStop::Failed(bounded(&error.to_string())))?;
    ensure(
        validator.is_valid(instance),
        &format!("an emitted document does not conform to {schema}"),
    )
}

fn prepare(base: &Path, tools: &MediaTools) -> Result<Value, StageStop> {
    for (dependency, tool) in [("ffmpeg", &tools.ffmpeg), ("ffprobe", &tools.ffprobe)] {
        let (code, _) = run_json(
            vsift(base)?
                .args(["setup", "configure", dependency, "--executable"])
                .arg(tool.path())
                .arg("--json"),
        )?;
        ensure(code == Some(0), "setup configure did not succeed")?;
    }
    let (code, check) = run_json(vsift(base)?.args(["setup", "check", "--json"]))?;
    ensure(code == Some(0), "setup check did not exit 0")?;
    ensure(
        check["dependencies"][2]["status"] == "missing",
        "whisper.cpp was visible; the journey must prove ASR is not needed",
    )?;
    Ok(check)
}

/// Searches `query` with `--json` and returns the one hit's item, after
/// checking it is a complete, phrase-ranked, schema-valid result.
fn search_one(base: &Path, session: &str, query: &str) -> Result<(Value, u128), StageStop> {
    let started = Instant::now();
    let (code, result) =
        run_json(vsift(base)?.args(["search", session, "--query", query, "--json"]))?;
    let elapsed = started.elapsed().as_millis();
    ensure(code == Some(0), &format!("search {query:?} failed"))?;
    conforms("operation-response.schema.json", &result)?;
    conforms("search-data.schema.json", &result["data"])?;
    ensure(
        result["status"] == "complete" && result["coverage"]["truncated"] == false,
        "a supplied-transcript search was not complete",
    )?;
    let data = &result["data"];
    ensure(
        data["transcript_coverage"]["basis"] == "supplied_transcript",
        "the coverage basis is not the supplied transcript",
    )?;
    let items = data["items"]
        .as_array()
        .ok_or_else(|| StageStop::Failed("items missing".to_owned()))?;
    ensure(
        items.len() == 1,
        &format!("{query:?} did not find exactly one segment"),
    )?;
    ensure(
        data["hits"][0]["match"] == "phrase"
            && data["hits"][0]["segment_id"] == items[0]["segment_id"],
        &format!("{query:?} did not match as a phrase"),
    )?;
    Ok((items[0].clone(), elapsed))
}

/// The stage: import F10, search two spellings, stream, and cite.
#[allow(
    clippy::too_many_lines,
    reason = "Keep the journey steps and their evidence record together"
)]
fn search_supplied(root: &OwnedRoot, tools: Option<&MediaTools>) -> Result<Value, StageStop> {
    let tools = tools.ok_or_else(|| StageStop::Blocked(MISSING_MEDIA_TOOLS.to_owned()))?;
    let (truth_start, truth_end) =
        f10_truth().map_err(|error| StageStop::Failed(bounded(&error.to_string())))?;
    let base = root.0.join("supplied");
    let check = prepare(&base, tools)?;
    let (code, opened) = run_json(
        vsift(&base)?
            .arg("ingest")
            .arg(repository().join("fixtures/corpus/generated/F10.mp4"))
            .arg("--transcript")
            .arg(repository().join("fixtures/corpus/transcripts/F10.srt"))
            .args(["--transcript-offset", OFFSET_US, "--json"]),
    )?;
    ensure(
        code == Some(0),
        &format!("ingest with transcript failed: {}", opened["error"]["code"]),
    )?;
    let session = opened["data"]["session_id"]
        .as_str()
        .ok_or_else(|| StageStop::Failed("session missing".to_owned()))?
        .to_owned();

    let (written, written_ms) = search_one(&base, &session, "R-17")?;
    let (spoken, spoken_ms) = search_one(&base, &session, "dialog r 17")?;
    ensure(
        written["segment_id"] == spoken["segment_id"],
        "the two spellings found different segments",
    )?;
    ensure(
        written["start_us"].as_u64() == Some(truth_start)
            && written["end_us"].as_u64() == Some(truth_end),
        "the found segment is not on F10-E01's truth window",
    )?;
    ensure(
        written["text"]
            .as_str()
            .is_some_and(|text| text.contains("Dialog R-17")),
        "the found segment does not say dialog R-17",
    )?;

    let output = vsift(&base)?
        .args(["search", &session, "--query", "R-17", "--events", "jsonl"])
        .output()?;
    ensure(output.status.code() == Some(0), "the search stream failed")?;
    ensure(
        output.stderr.is_empty(),
        "the search stream wrote to stderr",
    )?;
    let lines = std::str::from_utf8(&output.stdout)?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    ensure(
        lines.len() == 2,
        "the stream is not one record and one terminal event",
    )?;
    conforms("evidence-event.schema.json", &lines[0])?;
    conforms("terminal-event.schema.json", &lines[1])?;
    conforms(
        "search-stream-data.schema.json",
        &lines[1]["result"]["data"],
    )?;
    ensure(
        lines[0]["record"] == written && lines[0]["key"] == written["segment_id"],
        "the streamed record differs from the --json item",
    )?;
    ensure(
        lines[1]["result"]["data"]["record_count"] == 1,
        "the terminal event does not count the streamed record",
    )?;

    let (code, cited) = run_json(vsift(&base)?.args([
        "transcript",
        "get",
        &session,
        "--from",
        &truth_start.to_string(),
        "--to",
        &truth_end.to_string(),
        "--json",
    ]))?;
    ensure(code == Some(0), "transcript get failed")?;
    ensure(
        cited["data"]["items"][0] == written,
        "transcript get cites a different record for the truth window",
    )?;
    Ok(json!({
        "session_id_prefix": "ses_",
        "found_segment_id": written["segment_id"],
        "found_range_us": [written["start_us"], written["end_us"]],
        "truth_range_us": [truth_start, truth_end],
        "queries": ["R-17", "dialog r 17"],
        "match": "phrase",
        "coverage_basis": "supplied_transcript",
        "search_ms": [written_ms, spoken_ms],
        "jsonl_evidence_records": 1,
        "whisper_status": check["dependencies"][2]["status"],
        "ffmpeg_version": check["dependencies"][0]["detail"],
        "ffprobe_version": check["dependencies"][1]["detail"],
    }))
}

#[test]
#[ignore = "opt-in P08 checkpoint; needs ffmpeg and ffprobe on PATH; reports to .vsift/e2e-runs"]
fn search_checkpoint() -> TestResult {
    let started = Instant::now();
    let repository = repository().canonicalize()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_dir = repository
        .join(".vsift/e2e-runs")
        .join(format!("p08-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&run_dir)?;
    let root = OwnedRoot::new()?;
    let tools = MediaTools::discover();

    let clock = Instant::now();
    let result = search_supplied(&root, tools.as_ref());
    let elapsed_ms = clock.elapsed().as_millis();
    let stage = match &result {
        Ok(evidence) => json!({
            "name": "p08_search_supplied", "status": "passed",
            "elapsed_ms": elapsed_ms, "evidence": evidence,
        }),
        Err(StageStop::Failed(diagnostic)) => json!({
            "name": "p08_search_supplied", "status": "failed",
            "elapsed_ms": elapsed_ms, "diagnostic": diagnostic,
        }),
        Err(StageStop::Blocked(remediation)) => json!({
            "name": "p08_search_supplied", "status": "blocked",
            "elapsed_ms": elapsed_ms, "remediation": remediation,
        }),
    };
    let overall = stage["status"].clone();
    let future_stages: Vec<_> = [
        "p10_recovery",
        "p11_worker_batch",
        "p12_agent_clients",
        "p13_distribution",
        "p14_release_qualification",
    ]
    .into_iter()
    .map(|name| json!({"name": name, "status": "not_implemented"}))
    .collect();
    let report = json!({
        "schema_version": 1,
        "checkpoint": "P08 transcript search (supplied transcript)",
        "fixture": "F10.mp4 with fixtures/corpus/transcripts/F10.srt (+500 ms)",
        "os": env::consts::OS,
        "architecture": env::consts::ARCH,
        "resource_profile": "each CLI call killed after 60 s; empty PATH",
        "vsift_version": env!("CARGO_PKG_VERSION"),
        "authorization": "opt-in cargo test invocation; setup configure writes only to an isolated temporary per-user base; no install, download or network access",
        "prior_checkpoints": ["P07: p07_transcript_e2e", "P07: p07_local_asr_e2e"],
        "stages": [stage],
        "coverage_gaps": [
            "Search after local ASR is exercised by the engine and CLI contract tests on committed records, not by a whisper.cpp run here",
            "Visual candidates are checked by the separate p08_candidates_e2e checkpoint"
        ],
        "future_stages": future_stages,
        "overall": overall,
        "complete_journey": "not_implemented",
        "elapsed_ms": started.elapsed().as_millis(),
    });
    let report_path = run_dir.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("P08 checkpoint report: {}", report_path.display());
    if overall == "passed" {
        Ok(())
    } else {
        Err(format!("P08 checkpoint {overall}; see {}", report_path.display()).into())
    }
}
