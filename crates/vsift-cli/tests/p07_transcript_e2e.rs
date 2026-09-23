//! Opt-in cumulative P07 checkpoint: the supplied-transcript stage of A-09.
//!
//! Journeys drive the compiled `vsift` binary exactly as a headless agent
//! would, with an isolated per-user base and an empty `PATH`: `FFmpeg` and
//! `FFprobe` are registered with `setup configure`, and whisper.cpp is
//! therefore absent, which proves the supplied-transcript path never needs
//! local ASR. The F10 fixture video and its sidecars are imported with the
//! explicit +500 ms offset, and the cited segment is compared with the frozen
//! truth in `fixtures/corpus/manifest.json`. The local-ASR stage stays
//! `not_implemented` until the whisper.cpp adapter lands.
//!
//! `cargo test -p vsift-cli --locked --test p07_transcript_e2e -- --ignored --nocapture`
//!
//! The run writes a bounded report to `.vsift/e2e-runs/p07-<run-id>/report.json`.
//! A journey that cannot run here is `blocked`, never `passed`, and the test
//! fails unless every P07 journey passed.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::{Value, json};
use vsift_infrastructure::{
    ExecutableResolver, HostIsolation, ProcessCancellation, ProcessRequest, ProcessSupervisor,
    ProcessWorkingDirectory, SupervisorPolicy, TerminationReason, TrustedExecutable,
};

type TestResult = Result<(), Box<dyn Error>>;

/// Upper bound for one CLI invocation; a prompt or hang is killed and fails.
const CLI_DEADLINE: Duration = Duration::from_secs(60);
const OWNED_PREFIX: &str = "vsift-p07-e2e-";
const MAX_DIAGNOSTIC_CHARS: usize = 240;
const OFFSET_US: &str = "500000";
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

    fn base(&self, journey: &str) -> PathBuf {
        self.0.join(journey)
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

type StageResult = Result<Value, StageStop>;

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

/// F10's frozen truth: the window in which dialog R-17 is displayed.
/// All values are microseconds on the normalized source timeline.
struct Truth {
    start: u64,
    end: u64,
    duration: u64,
}

fn f10_truth() -> Result<Truth, Box<dyn Error>> {
    let manifest: Value = serde_json::from_slice(&fs::read(
        repository().join("fixtures/corpus/manifest.json"),
    )?)?;
    let fixture = manifest["fixtures"]
        .as_array()
        .and_then(|fixtures| fixtures.iter().find(|fixture| fixture["id"] == "F10"))
        .ok_or("F10 missing from the manifest")?;
    let event = fixture["events"]
        .as_array()
        .and_then(|events| events.iter().find(|event| event["id"] == "F10-E01"))
        .ok_or("F10-E01 missing from the manifest")?;
    Ok(Truth {
        start: event["start_us"].as_u64().ok_or("start_us missing")?,
        end: event["end_us"].as_u64().ok_or("end_us missing")?,
        duration: fixture["duration_us"].as_u64().ok_or("duration missing")?,
    })
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

/// Imports one F10 sidecar and cites the truth window through `transcript get`.
fn import_and_cite(
    root: &OwnedRoot,
    tools: Option<&MediaTools>,
    sidecar: &str,
    origin: &str,
) -> StageResult {
    let tools = tools.ok_or_else(|| StageStop::Blocked(MISSING_MEDIA_TOOLS.to_owned()))?;
    let truth = f10_truth().map_err(|error| StageStop::Failed(bounded(&error.to_string())))?;
    let base = root.base(origin);
    let check = prepare(&base, tools)?;
    let video = repository().join("fixtures/corpus/generated/F10.mp4");
    let sidecar = repository()
        .join("fixtures/corpus/transcripts")
        .join(sidecar);

    let started = Instant::now();
    let (code, opened) = run_json(
        vsift(&base)?
            .arg("ingest")
            .arg(&video)
            .arg("--transcript")
            .arg(&sidecar)
            .args(["--transcript-offset", OFFSET_US, "--json"]),
    )?;
    let ingest_ms = started.elapsed().as_millis();
    ensure(
        code == Some(0),
        &format!("ingest with transcript failed: {}", opened["error"]["code"]),
    )?;
    let transcript = &opened["data"]["transcript"];
    ensure(
        transcript["alignment"]["origin"] == origin,
        "alignment origin is not the sidecar format",
    )?;
    ensure(
        transcript["source_segments"][0]["end_us"].as_u64() == Some(truth.duration),
        "probed duration differs from the fixture truth",
    )?;
    ensure(
        transcript["warnings"].as_array().is_some_and(Vec::is_empty),
        "the F10 import produced warnings",
    )?;
    let session = opened["data"]["session_id"]
        .as_str()
        .ok_or_else(|| StageStop::Failed("session missing".to_owned()))?
        .to_owned();

    let (code, cited) = run_json(vsift(&base)?.args([
        "transcript",
        "get",
        &session,
        "--from",
        &truth.start.to_string(),
        "--to",
        &truth.end.to_string(),
        "--json",
    ]))?;
    ensure(code == Some(0), "transcript get failed")?;
    let items = cited["data"]["items"]
        .as_array()
        .ok_or_else(|| StageStop::Failed("items missing".to_owned()))?;
    ensure(
        items.len() == 1,
        "the truth window did not cite exactly one segment",
    )?;
    let segment = &items[0];
    ensure(
        segment["text"]
            .as_str()
            .is_some_and(|text| text.contains("Dialog R-17")),
        "the cited segment does not say dialog R-17",
    )?;
    ensure(
        segment["start_us"].as_u64() == Some(truth.start)
            && segment["end_us"].as_u64() == Some(truth.end),
        "the cited range does not match F10-E01",
    )?;
    ensure(
        segment["confidence"]["value_basis_points"].is_null()
            && segment["confidence"]["origin"] == "unavailable",
        "imported text claimed a confidence",
    )?;
    Ok(json!({
        "session_id_prefix": "ses_",
        "revision_id": transcript["revision_id"],
        "sidecar_sha256": transcript["sidecar"]["sha256"],
        "segment_count": transcript["segment_count"],
        "cited_segment_id": segment["segment_id"],
        "cited_range_us": [segment["start_us"], segment["end_us"]],
        "truth_range_us": [truth.start, truth.end],
        "cue_timing_us": [segment["alignment"]["cue_start_us"], segment["alignment"]["cue_end_us"]],
        "offset_us": segment["alignment"]["offset_us"],
        "whisper_status": check["dependencies"][2]["status"],
        "ffmpeg_version": check["dependencies"][0]["detail"],
        "ffprobe_version": check["dependencies"][1]["detail"],
        "ingest_ms": ingest_ms,
    }))
}

/// T-02: a wrong offset is rejected with typed remediation and opens nothing.
fn wrong_offset_rejected(root: &OwnedRoot, tools: Option<&MediaTools>) -> StageResult {
    let tools = tools.ok_or_else(|| StageStop::Blocked(MISSING_MEDIA_TOOLS.to_owned()))?;
    let base = root.base("wrong-offset");
    prepare(&base, tools)?;
    let (code, rejected) = run_json(
        vsift(&base)?
            .arg("ingest")
            .arg(repository().join("fixtures/corpus/generated/F10.mp4"))
            .arg("--transcript")
            .arg(repository().join("fixtures/corpus/transcripts/F10.srt"))
            .args(["--transcript-offset=-60000000", "--json"]),
    )?;
    ensure(code == Some(2), "a wrong offset did not exit 2")?;
    ensure(
        rejected["error"]["code"] == "INVALID_ARGUMENT",
        "a wrong offset was not INVALID_ARGUMENT",
    )?;
    let summary = rejected["error"]["remediation"][0]["summary"]
        .as_str()
        .unwrap_or_default();
    ensure(
        summary.contains("no_cues_within_source"),
        "the rejection did not name its typed reason",
    )?;
    let (_, listed) = run_json(vsift(&base)?.args(["session", "list", "--json"]))?;
    let open = listed["data"]["items"].as_array().map_or(0, |items| {
        items.iter().filter(|item| item["state"] == "open").count()
    });
    ensure(open == 0, "a rejected import left an open session")?;
    Ok(json!({"code": rejected["error"]["code"], "open_sessions": open}))
}

fn stage(name: &str, started: Instant, result: StageResult) -> Value {
    let elapsed_ms = started.elapsed().as_millis();
    match result {
        Ok(evidence) => json!({
            "name": name, "status": "passed", "elapsed_ms": elapsed_ms, "evidence": evidence,
        }),
        Err(StageStop::Failed(diagnostic)) => json!({
            "name": name, "status": "failed", "elapsed_ms": elapsed_ms, "diagnostic": diagnostic,
        }),
        Err(StageStop::Blocked(remediation)) => json!({
            "name": name, "status": "blocked", "elapsed_ms": elapsed_ms, "remediation": remediation,
        }),
    }
}

async fn tool_line(
    executable: TrustedExecutable,
    root: &Path,
    arguments: &[&str],
) -> Option<String> {
    let cwd = ProcessWorkingDirectory::new(root).ok()?;
    let request = ProcessRequest::new(executable, cwd, Duration::from_secs(5))
        .ok()?
        .with_arguments(arguments.iter().copied());
    let limit = NonZeroUsize::new(64 * 1024)?;
    let supervisor = ProcessSupervisor::new(
        SupervisorPolicy::new(limit, Duration::from_secs(1), Duration::from_secs(1)),
        HostIsolation::ProcessOnly,
    );
    let result = supervisor
        .run(request, ProcessCancellation::new())
        .await
        .ok()?;
    if result.termination != TerminationReason::Exited
        || !result.status.success()
        || !result.stdout.complete
    {
        return None;
    }
    std::str::from_utf8(&result.stdout.bytes).ok().map(|value| {
        value
            .lines()
            .next()
            .map_or_else(String::new, |line| line.chars().take(160).collect())
    })
}

async fn repository_state(repository: &Path) -> (Option<String>, Option<bool>) {
    let Ok(git) = ExecutableResolver::from_current_path().resolve(OsStr::new("git")) else {
        return (None, None);
    };
    let revision = tool_line(git.clone(), repository, &["rev-parse", "HEAD"]).await;
    let dirty = tool_line(git, repository, &["status", "--porcelain"])
        .await
        .map(|line| !line.is_empty());
    (revision, dirty)
}

#[tokio::test]
#[ignore = "opt-in P07 checkpoint; needs ffmpeg and ffprobe on PATH; reports to .vsift/e2e-runs"]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the journey order and the complete evidence record visible together"
)]
async fn supplied_transcript_checkpoint() -> TestResult {
    let started = Instant::now();
    let repository = repository().canonicalize()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_dir = repository
        .join(".vsift/e2e-runs")
        .join(format!("p07-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&run_dir)?;
    let root = OwnedRoot::new()?;
    let tools = MediaTools::discover();

    let mut stages = Vec::new();
    for (name, sidecar, origin) in [
        ("p07_supplied_transcript_srt", "F10.srt", "imported_srt"),
        (
            "p07_supplied_transcript_webvtt",
            "F10.vtt",
            "imported_webvtt",
        ),
    ] {
        let clock = Instant::now();
        let result = import_and_cite(&root, tools.as_ref(), sidecar, origin);
        stages.push(stage(name, clock, result));
    }
    let clock = Instant::now();
    let result = wrong_offset_rejected(&root, tools.as_ref());
    stages.push(stage("p07_wrong_offset_rejected", clock, result));

    let status_of = |wanted: &str| stages.iter().any(|entry| entry["status"] == wanted);
    let overall = if status_of("failed") {
        "failed"
    } else if status_of("blocked") {
        "blocked"
    } else {
        "passed"
    };
    let future_stages: Vec<_> = [
        "p07_local_asr",
        "p08_candidates_search",
        "p09_source_reinspection",
        "p10_recovery",
        "p11_worker_batch",
        "p12_agent_clients",
        "p13_distribution",
        "p14_release_qualification",
    ]
    .into_iter()
    .map(|name| json!({"name": name, "status": "not_implemented"}))
    .collect();
    let manifest: Value =
        serde_json::from_slice(&fs::read(repository.join("fixtures/corpus/manifest.json"))?)?;
    let provenance: Value = serde_json::from_slice(&fs::read(
        repository.join("fixtures/corpus/generated/provenance.json"),
    )?)?;
    let f10_sha256 = provenance["fixtures"]
        .as_array()
        .and_then(|fixtures| fixtures.iter().find(|fixture| fixture["fixture"] == "F10"))
        .map(|fixture| fixture["sha256"].clone())
        .unwrap_or_default();
    let versions = stages
        .first()
        .map(|entry| entry["evidence"].clone())
        .unwrap_or_default();
    let (revision, dirty) = repository_state(&repository).await;
    let report = json!({
        "schema_version": 1,
        "checkpoint": "P07 supplied transcript",
        "revision": revision,
        "dirty": dirty,
        "fixture_manifest": {
            "schema_version": manifest["schema_version"],
            "corpus_id": manifest["corpus_id"],
        },
        "fixture": "F10.mp4 with fixtures/corpus/transcripts/F10.srt and F10.vtt",
        "fixture_sha256": f10_sha256,
        "os": env::consts::OS,
        "architecture": env::consts::ARCH,
        "filesystem": env::var("VSIFT_E2E_FILESYSTEM").map_or_else(
            |_| "uninspected".to_owned(),
            |value| format!("operator reported: {}", value.chars().take(60).collect::<String>()),
        ),
        "resource_profile": "each CLI call killed after 60 s; empty PATH; FFprobe under the P04 probe bounds",
        "vsift_version": env!("CARGO_PKG_VERSION"),
        "ffmpeg_version": versions["ffmpeg_version"],
        "ffprobe_version": versions["ffprobe_version"],
        "whisper_version": null,
        "model_version": null,
        "client_versions": [],
        "authorization": "opt-in cargo test invocation; setup configure writes only to isolated temporary per-user bases; no install, download or network access",
        "prior_checkpoints": ["P04: p04_media_e2e", "P05: p05_session_e2e", "P06: p06_setup_e2e"],
        "stages": stages,
        "coverage_gaps": [
            "Local ASR (whisper.cpp adapter, chunking, preflight) is P07 increment 3",
            "The automatic media-tool preflight does not yet run before the probe",
            "Search, candidates and visual refinement belong to P08 and P09"
        ],
        "future_stages": future_stages,
        "overall": overall,
        "complete_journey": "not_implemented",
        "elapsed_ms": started.elapsed().as_millis(),
    });
    let report_path = run_dir.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("P07 checkpoint report: {}", report_path.display());
    if overall == "passed" {
        Ok(())
    } else {
        Err(format!("P07 checkpoint {overall}; see {}", report_path.display()).into())
    }
}
