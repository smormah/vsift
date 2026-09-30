//! Opt-in P13 stage of the end-to-end spine: the installed-user and
//! managed-dependency run on Ubuntu 24.04 x86-64 (ADR 0023 §3 step 7,
//! decision H9; the "Installed-user and managed-dependency run" of
//! `docs/planning/e2e-test-spine.md`).
//!
//! It drives the compiled release binary from a fresh per-user base with an
//! empty `PATH` and nothing configured, so no `FFmpeg`, whisper.cpp, model or
//! Rust toolchain is reachable, and every tool the journey uses is one
//! `setup install` downloaded, verified, smoked and activated itself:
//!
//! - `p13_clean_host`: `setup check` is not ready, `setup list` and `setup
//!   repair` report no managed folder and create none;
//! - `p13_install_killed_in_download`: the accepted `setup install` is killed
//!   by the operating system (`SIGKILL`) while its first download is being
//!   written; `setup repair` then names the abandoned stage and its fix,
//!   nothing is selected, and the guard is free;
//! - `p13_install_killed_in_smoke`: the rerun is killed while a component's
//!   compatibility smoke runs, with the same checks (every selection that
//!   exists verifies);
//! - `p13_install_rerun_completes`: the same accepted command completes,
//!   sweeps the abandoned stages, and `setup check` resolves every tool as
//!   `managed_version` and verifies local speech recognition;
//! - `p13_managed_journey`: the A-08 local-ASR journey on the managed tools
//!   alone: plain `ingest` of F05's speech variant, `transcript retranscribe`,
//!   the cited speech window and words, `search`, `candidates`, `frame get
//!   --candidate`, `audio`, `session retain` and `bundle validate`;
//! - `p13_uninstall_and_reinstall`: `setup remove whisper_model` deselects
//!   and removes the model, so local ASR is no longer ready and a
//!   retranscription fails typed; the same accepted plan reinstalls only the
//!   model.
//!
//! ```console
//! VSIFT_P13_INSTALL_E2E=1 cargo test --release --locked -p vsift-cli \
//!   --test p13_install_e2e -- --ignored --exact --nocapture
//! ```
//!
//! A development build reaches no publisher (the P13 network guard), so the
//! stage runs `--release`, on a disposable hosted runner: the manual
//! workflow `P13 managed smoke`, job `install-e2e`. It downloads the three
//! pinned publisher artifacts up to four times and sends no credentials and
//! no personal detail: the requests carry only `VSift`'s neutral user agent.
//! The clean install of the native archive and of the npm packages without
//! Rust belongs to P13 PRs 9 and 11. It writes a bounded report to
//! `.vsift/e2e-runs/p13-<run-id>/report.json`, and fails unless every stage
//! passed.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command as Process, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::{Value, json};
use vsift_domain::ManagedTarget;
use vsift_infrastructure::detect_managed_target;

type TestResult = Result<(), Box<dyn Error>>;

const OPT_IN: &str = "VSIFT_P13_INSTALL_E2E";
/// One CLI call: the downloads, extraction, smoke and local ASR included.
const CLI_DEADLINE: Duration = Duration::from_mins(30);
/// How long a kill waits for the moment it is aimed at.
const KILL_WAIT: Duration = Duration::from_mins(20);
const OWNED_PREFIX: &str = "vsift-p13-install-e2e-";
const MAX_DIAGNOSTIC_CHARS: usize = 400;
/// The download is killed once its artifact holds this many bytes.
const KILL_AFTER_BYTES: u64 = 1024 * 1024;
/// whisper's base model reports coarse times (as the P07 stage allows).
const SPAN_TOLERANCE_US: u64 = 1_000_000;

/// A fresh per-user base, removed on drop.
struct Base(PathBuf);

impl Base {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!("{OWNED_PREFIX}{}-{stamp}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn managed_root(&self) -> PathBuf {
        self.0.join("data/vsift/managed-v1")
    }

    fn environment(&self) -> [(&'static str, PathBuf); 6] {
        [
            ("PATH", PathBuf::new()),
            ("HOME", self.0.clone()),
            ("XDG_CONFIG_HOME", self.0.join("config")),
            ("XDG_DATA_HOME", self.0.join("data")),
            ("XDG_CACHE_HOME", self.0.join("cache")),
            ("XDG_STATE_HOME", self.0.join("state")),
        ]
    }

    /// The binary with this base, an empty `PATH`, no stdin and a deadline.
    fn vsift(&self) -> Result<Command, StageStop> {
        let mut command = Command::cargo_bin("vsift")?;
        for (name, value) in self.environment() {
            command.env(name, value);
        }
        command
            .arg("--session-root")
            .arg(self.0.join("sessions"))
            .timeout(CLI_DEADLINE)
            .write_stdin("");
        Ok(command)
    }

    /// The same, as a child the stage can kill.
    fn spawn(&self, arguments: &[&OsStr]) -> Result<Child, StageStop> {
        let binary = assert_cmd::cargo::cargo_bin("vsift");
        let mut command = Process::new(binary);
        for (name, value) in self.environment() {
            command.env(name, value);
        }
        Ok(command
            .arg("--session-root")
            .arg(self.0.join("sessions"))
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?)
    }
}

impl Drop for Base {
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

fn failed(expectation: &str) -> StageStop {
    StageStop::Failed(bounded(expectation))
}

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> PathBuf {
    repository().join("fixtures/corpus/generated").join(name)
}

fn run_json(command: &mut Command) -> Result<(Option<i32>, Value), StageStop> {
    let output = command.output()?;
    Ok((
        output.status.code(),
        serde_json::from_slice(&output.stdout)?,
    ))
}

fn u64_of(value: &Value) -> Result<u64, StageStop> {
    value.as_u64().ok_or_else(|| failed("a number is missing"))
}

/// The saved plan's path and the digest that accepts it.
struct AcceptedPlan {
    path: PathBuf,
    digest: String,
}

impl AcceptedPlan {
    fn arguments(&self) -> Vec<&OsStr> {
        vec![
            OsStr::new("setup"),
            OsStr::new("install"),
            OsStr::new("--plan"),
            self.path.as_os_str(),
            OsStr::new("--accept-plan"),
            OsStr::new(self.digest.as_str()),
            OsStr::new("--json"),
        ]
    }
}

fn component_statuses(result: &Value) -> Vec<String> {
    result["data"]["components"]
        .as_array()
        .map(|components| {
            components
                .iter()
                .map(|component| component["status"].as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// `setup repair` and `setup list` after a kill: the store is consistent,
/// every existing selection verifies, the guard is free, and repair names
/// each abandoned stage with its command. Returns both results.
fn after_a_kill(base: &Base) -> Result<Value, StageStop> {
    let (code, repair) = run_json(base.vsift()?.args(["setup", "repair", "--json"]))?;
    ensure(code == Some(0), "setup repair failed after a kill")?;
    let (code, listing) = run_json(base.vsift()?.args(["setup", "list", "--json"]))?;
    ensure(code == Some(0), "setup list failed after a kill")?;
    let findings = repair["data"]["findings"]
        .as_array()
        .ok_or_else(|| failed("repair has no findings list"))?;
    for finding in findings {
        ensure(
            finding["fix"] != "manual" && !finding["command"].is_null(),
            &format!("a killed install left something repair cannot name: {finding}"),
        )?;
        ensure(
            matches!(
                finding["kind"].as_str(),
                Some("stale_stages" | "interrupted_selection")
            ),
            &format!("a killed install left an unexpected finding: {finding}"),
        )?;
    }
    let stages = stages(&base.managed_root()).len();
    let reported = findings
        .iter()
        .find(|finding| finding["kind"] == "stale_stages")
        .and_then(|finding| finding["count"].as_u64())
        .unwrap_or(0);
    ensure(
        usize::try_from(reported)? == stages,
        &format!("repair reports {reported} abandoned stages, the folder holds {stages}"),
    )?;
    for component in listing["data"]["components"]
        .as_array()
        .ok_or_else(|| failed("setup list has no components"))?
    {
        ensure(
            matches!(component["selection"].as_str(), Some("none" | "verified")),
            &format!("a killed install left a selection that does not verify: {component}"),
        )?;
    }
    // The install guard died with the process: a guarded command is not
    // busy. A rollback with no earlier version is refused without a change.
    let (_, rollback) = run_json(
        base.vsift()?
            .args(["setup", "rollback", "whisper_model"])
            .arg("--json"),
    )?;
    ensure(
        rollback["error"]["code"] != "BUSY",
        "the killed install's guard is still held",
    )?;
    Ok(json!({
        "repair_status": repair["data"]["status"],
        "findings": findings.iter().map(|finding| json!({
            "kind": finding["kind"], "count": finding["count"], "fix": finding["fix"],
        })).collect::<Vec<_>>(),
        "stages_on_disk": stages,
        "selections": listing["data"]["components"].as_array().map(|components| components
            .iter()
            .map(|component| json!([component["component"], component["selection"]]))
            .collect::<Vec<_>>()),
    }))
}

/// Starts the accepted install and kills it (`SIGKILL`) once `ready`
/// holds; returns how long it ran.
fn kill_install_when(
    base: &Base,
    plan: &AcceptedPlan,
    ready: impl Fn(&Path) -> bool,
    what: &str,
) -> Result<Duration, StageStop> {
    let started = Instant::now();
    let mut child = base.spawn(&plan.arguments())?;
    let root = base.managed_root();
    loop {
        if ready(&root) {
            break;
        }
        if let Some(status) = child.try_wait()? {
            return Err(failed(&format!(
                "the install ended ({status}) before {what}"
            )));
        }
        if started.elapsed() > KILL_WAIT {
            child.kill()?;
            child.wait()?;
            return Err(failed(&format!("{what} never happened")));
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.kill()?;
    child.wait()?;
    Ok(started.elapsed())
}

/// Every `stage-*` folder of the managed root.
fn stages(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("stage-"))
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default()
}

fn clean_host(base: &Base) -> StageResult {
    let (_, check) = run_json(base.vsift()?.args(["setup", "check", "--json"]))?;
    ensure(
        check["status"] != "ready",
        "a fresh host with an empty PATH reports ready",
    )?;
    let (code, listing) = run_json(base.vsift()?.args(["setup", "list", "--json"]))?;
    ensure(code == Some(0), "setup list failed")?;
    ensure(
        listing["data"]["managed_folder"] == "absent",
        "setup list found a managed folder on a fresh host",
    )?;
    let (code, repair) = run_json(base.vsift()?.args(["setup", "repair", "--json"]))?;
    ensure(
        code == Some(0) && repair["data"]["status"] == "nothing_installed",
        "setup repair did not report nothing installed",
    )?;
    ensure(
        !base.managed_root().exists(),
        "a read-only command created the managed folder",
    )?;
    Ok(json!({
        "setup_check_status": check["status"],
        "dependencies": check["dependencies"].as_array().map(|dependencies| dependencies
            .iter()
            .map(|dependency| json!([dependency["dependency"], dependency["status"]]))
            .collect::<Vec<_>>()),
    }))
}

fn accept_plan(base: &Base) -> Result<AcceptedPlan, StageStop> {
    let output = base
        .vsift()?
        .args(["setup", "plan", "--profile", "desktop", "--json"])
        .output()?;
    ensure(output.status.success(), "setup plan failed")?;
    let plan: Value = serde_json::from_slice(&output.stdout)?;
    let digest = plan["data"]["plan_digest"]
        .as_str()
        .ok_or_else(|| failed("the plan has no digest"))?
        .to_owned();
    ensure(
        plan["data"]["actions"].as_array().map_or(0, Vec::len) == 3,
        "the plan does not install three components",
    )?;
    let path = base.0.join("plan.json");
    fs::write(&path, &output.stdout)?;
    Ok(AcceptedPlan { path, digest })
}

fn killed_in_download(base: &Base, plan: &AcceptedPlan) -> StageResult {
    let ran = kill_install_when(
        base,
        plan,
        |root| {
            stages(root).iter().any(|stage| {
                fs::metadata(stage.join("artifact.pending"))
                    .is_ok_and(|metadata| metadata.len() >= KILL_AFTER_BYTES)
            })
        },
        "the first download reached 1 MiB",
    )?;
    let evidence = after_a_kill(base)?;
    ensure(
        evidence["stages_on_disk"] == 1,
        "the killed download did not leave exactly one stage",
    )?;
    Ok(json!({"killed_after_ms": ran.as_millis(), "after": evidence}))
}

fn killed_in_smoke(base: &Base, plan: &AcceptedPlan) -> StageResult {
    let ran = kill_install_when(
        base,
        plan,
        |root| {
            stages(root)
                .iter()
                .any(|stage| stage.join("smoke.pending").exists())
        },
        "a compatibility smoke started",
    )?;
    let evidence = after_a_kill(base)?;
    Ok(json!({"killed_after_ms": ran.as_millis(), "after": evidence}))
}

fn rerun_completes(base: &Base, plan: &AcceptedPlan) -> StageResult {
    let started = Instant::now();
    let (code, result) = run_json(
        base.vsift()?
            .args(["setup", "install", "--plan"])
            .arg(&plan.path)
            .args(["--accept-plan", &plan.digest, "--json"]),
    )?;
    let elapsed = started.elapsed();
    ensure(
        code == Some(0),
        &format!("the rerun failed: {}", result["error"]),
    )?;
    let statuses = component_statuses(&result);
    ensure(
        statuses.len() == 3
            && statuses
                .iter()
                .all(|status| status == "activated" || status == "already_current"),
        &format!("the rerun did not install every component: {statuses:?}"),
    )?;
    let swept = u64_of(&result["data"]["cleanup"]["stale_stages_removed"])?;
    ensure(swept >= 1, "the rerun swept no abandoned stage")?;
    // A provider the killed smoke started can still be finishing while the
    // rerun sweeps (L-055), so the sweep may keep that stage once; repair
    // then names it and `setup remove --stale-stages` removes it.
    let kept = stages(&base.managed_root()).len();
    if kept > 0 {
        let (code, repair) = run_json(base.vsift()?.args(["setup", "repair", "--json"]))?;
        ensure(
            code == Some(0)
                && repair["data"]["findings"]
                    .as_array()
                    .is_some_and(|findings| {
                        findings
                            .iter()
                            .any(|finding| finding["kind"] == "stale_stages")
                    }),
            "a stage the rerun kept is not named by repair",
        )?;
        ok_data(
            base,
            &["setup", "remove", "--stale-stages"],
            "setup remove --stale-stages",
        )?;
    }
    ensure(
        stages(&base.managed_root()).is_empty(),
        "stages remain after the rerun and the sweep",
    )?;
    let (code, repair) = run_json(base.vsift()?.args(["setup", "repair", "--json"]))?;
    ensure(
        code == Some(0) && repair["data"]["status"] == "healthy",
        "the store is not healthy after the rerun",
    )?;
    let (code, check) = run_json(base.vsift()?.args(["setup", "check", "--json"]))?;
    ensure(
        code == Some(0) && check["status"] == "ready",
        "setup check is not ready after the install",
    )?;
    for dependency in check["dependencies"]
        .as_array()
        .ok_or_else(|| failed("no dependencies"))?
    {
        ensure(
            dependency["lookup"] == "managed_version",
            &format!("a tool is not the managed version: {dependency}"),
        )?;
    }
    ensure(
        check["local_asr"]["verification"]["status"] == "verified",
        "local ASR does not verify with the managed tools",
    )?;
    Ok(json!({
        "statuses": statuses,
        "stale_stages_removed": swept,
        "stages_kept_by_the_rerun": kept,
        "install_ms": elapsed.as_millis(),
        "tools": check["dependencies"].as_array().map(|dependencies| dependencies
            .iter()
            .map(|dependency| json!([dependency["dependency"], dependency["detail"]]))
            .collect::<Vec<_>>()),
        "model": check["local_asr"]["model"],
    }))
}

/// The generator's speech span of a fixture, from `speech-provenance.json`.
fn speech_span(fixture_id: &str) -> Result<(u64, u64), StageStop> {
    let provenance: Value = serde_json::from_slice(&fs::read(fixture("speech-provenance.json"))?)?;
    let variant = provenance["assembly"]["variants"]
        .as_array()
        .and_then(|variants| {
            variants
                .iter()
                .find(|variant| variant["fixture"] == fixture_id)
        })
        .ok_or_else(|| failed("fixture missing from speech provenance"))?;
    Ok((
        u64_of(&variant["speech_start_us"])?,
        u64_of(&variant["speech_end_us"])?,
    ))
}

fn normalised(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|character| character.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn ok_data(base: &Base, arguments: &[&str], what: &str) -> Result<Value, StageStop> {
    let (code, result) = run_json(base.vsift()?.args(arguments).arg("--json"))?;
    ensure(
        code == Some(0),
        &format!("{what} failed: {}", result["error"]),
    )?;
    Ok(result["data"].clone())
}

/// Plain `ingest` of F05's speech variant and `transcript retranscribe` on
/// the managed recognizer and model, cited against the speech window and
/// its words; returns the session, the speech span and the evidence.
fn managed_transcript(base: &Base) -> Result<(String, (u64, u64), Value), StageStop> {
    let video = fixture("F05-speech.mp4").canonicalize()?;
    let video_text = video
        .to_str()
        .ok_or_else(|| failed("non-UTF-8 fixture path"))?;
    let opened = ok_data(base, &["ingest", video_text], "ingest")?;
    let session = opened["session_id"]
        .as_str()
        .ok_or_else(|| failed("ingest returned no session"))?
        .to_owned();
    let retranscribed = ok_data(
        base,
        &["transcript", "retranscribe", &session],
        "transcript retranscribe",
    )?;
    let duration = 20_000_000_u64;
    let page = ok_data(
        base,
        &[
            "transcript",
            "get",
            &session,
            "--from",
            "0",
            "--to",
            &duration.to_string(),
            "--limit",
            "100",
        ],
        "transcript get",
    )?;
    let items = page["items"]
        .as_array()
        .ok_or_else(|| failed("no transcript items"))?;
    ensure(!items.is_empty(), "the managed recognizer heard nothing")?;
    let (span_start, span_end) = speech_span("F05")?;
    for item in items {
        let (from, to) = (u64_of(&item["start_us"])?, u64_of(&item["end_us"])?);
        ensure(
            from + SPAN_TOLERANCE_US >= span_start && to <= span_end + SPAN_TOLERANCE_US,
            &format!("a segment [{from}, {to}) lies outside the speech span"),
        )?;
    }
    let text = normalised(
        &items
            .iter()
            .filter_map(|item| item["text"].as_str())
            .collect::<Vec<_>>()
            .join(" "),
    );
    for word in ["success banner", "submit"] {
        ensure(
            text.contains(word),
            &format!("the transcript lacks {word:?}: {text}"),
        )?;
    }
    let evidence = json!({
        "fixture": "F05-speech.mp4",
        "revision": retranscribed["revision"]["revision"],
        "model_profile": retranscribed["revision"]["local_asr"]["model_profile"],
        "segments": items.len(),
    });
    Ok((session, (span_start, span_end), evidence))
}

/// The A-08 journey on the managed tools alone.
fn managed_journey(base: &Base) -> StageResult {
    let started = Instant::now();
    let (session, (span_start, span_end), mut evidence) = managed_transcript(base)?;
    let duration = 20_000_000_u64;
    let search = ok_data(base, &["search", &session, "--query", "banner"], "search")?;
    let hit = &search["items"][0];
    ensure(
        u64_of(&hit["start_us"])? + SPAN_TOLERANCE_US >= span_start
            && u64_of(&hit["end_us"])? <= span_end + SPAN_TOLERANCE_US,
        "search found no segment inside the speech span",
    )?;
    let candidates = ok_data(
        base,
        &[
            "candidates",
            &session,
            "--from",
            "0",
            "--to",
            &duration.to_string(),
            "--limit",
            "100",
        ],
        "candidates",
    )?;
    let candidate = candidates["items"][0]["candidate_id"]
        .as_str()
        .ok_or_else(|| failed("no candidate"))?
        .to_owned();
    let frame = ok_data(
        base,
        &["frame", "get", &session, "--candidate", &candidate],
        "frame get",
    )?;
    ensure(
        frame["items"]
            .as_array()
            .is_some_and(|items| items.len() == 1),
        "frame get did not deliver one frame",
    )?;
    let audio_from = u64_of(&hit["start_us"])?;
    let audio_to = u64_of(&hit["end_us"])?.min(audio_from + 30_000_000);
    let audio = ok_data(
        base,
        &[
            "audio",
            &session,
            "--from",
            &audio_from.to_string(),
            "--to",
            &audio_to.to_string(),
        ],
        "audio",
    )?;
    ensure(
        audio["items"]
            .as_array()
            .is_some_and(|items| items.len() == 1),
        "audio did not deliver one clip",
    )?;
    let bundle = base.0.join("bundle");
    let bundle_text = bundle
        .to_str()
        .ok_or_else(|| failed("non-UTF-8 bundle path"))?;
    ok_data(
        base,
        &["session", "retain", &session, "--output", bundle_text],
        "session retain",
    )?;
    ok_data(
        base,
        &["bundle", "validate", bundle_text],
        "bundle validate",
    )?;
    evidence["search_hit_us"] = json!([hit["start_us"], hit["end_us"]]);
    evidence["candidates"] = json!(candidates["items"].as_array().map_or(0, Vec::len));
    evidence["journey_ms"] = json!(started.elapsed().as_millis());
    Ok(evidence)
}

fn uninstall_and_reinstall(base: &Base, plan: &AcceptedPlan) -> StageResult {
    let removed = ok_data(
        base,
        &["setup", "remove", "whisper_model"],
        "setup remove whisper_model",
    )?;
    ensure(
        removed["deselected"] == true
            && removed["versions"].as_array().is_some_and(|versions| {
                versions
                    .iter()
                    .all(|version| version["status"] == "removed")
            }),
        &format!("the model was not deselected and removed: {removed}"),
    )?;
    let (_, check) = run_json(base.vsift()?.args(["setup", "check", "--json"]))?;
    ensure(
        check["local_asr"]["verification"]["status"] != "verified",
        "local ASR still verifies without the model",
    )?;
    let video = fixture("F01-speech.mp4").canonicalize()?;
    let video_text = video
        .to_str()
        .ok_or_else(|| failed("non-UTF-8 fixture path"))?;
    let opened = ok_data(base, &["ingest", video_text], "ingest")?;
    let session = opened["session_id"]
        .as_str()
        .ok_or_else(|| failed("ingest returned no session"))?
        .to_owned();
    let (code, refused) = run_json(
        base.vsift()?
            .args(["transcript", "retranscribe", &session])
            .arg("--json"),
    )?;
    ensure(
        code != Some(0) && refused["error"]["code"] == "MISSING_CAPABILITY",
        &format!(
            "a retranscription without the model did not fail typed: {}",
            refused["error"]["code"]
        ),
    )?;
    let (code, result) = run_json(
        base.vsift()?
            .args(["setup", "install", "--plan"])
            .arg(&plan.path)
            .args(["--accept-plan", &plan.digest, "--json"]),
    )?;
    ensure(
        code == Some(0),
        &format!("the reinstall failed: {}", result["error"]),
    )?;
    let statuses = component_statuses(&result);
    ensure(
        statuses == ["already_current", "already_current", "activated"],
        &format!("the reinstall did not install only the model: {statuses:?}"),
    )?;
    let (code, check) = run_json(base.vsift()?.args(["setup", "check", "--json"]))?;
    ensure(
        code == Some(0)
            && check["status"] == "ready"
            && check["local_asr"]["verification"]["status"] == "verified",
        "local ASR does not verify after the reinstall",
    )?;
    Ok(json!({
        "removed_versions": removed["versions"],
        "retranscribe_without_model": refused["error"]["code"],
        "reinstall": statuses,
    }))
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

fn blocked(reason: &str) -> StageResult {
    Err(StageStop::Blocked(reason.to_owned()))
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the stage order and the complete evidence record visible together"
)]
#[ignore = "opt-in P13 install stage; set VSIFT_P13_INSTALL_E2E=1 on Ubuntu 24.04 x86-64 and run --release (a development build reaches no publisher); reports to .vsift/e2e-runs"]
fn p13_install_checkpoint() -> TestResult {
    if env::var_os(OPT_IN).is_none() {
        return Err(format!("set {OPT_IN}=1 to run the P13 install stage").into());
    }
    if detect_managed_target() != ManagedTarget::Ubuntu2404X86_64 {
        return Err("the P13 install stage runs only on Ubuntu 24.04 x86-64".into());
    }
    let started = Instant::now();
    let repository = repository().canonicalize()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_dir = repository
        .join(".vsift/e2e-runs")
        .join(format!("p13-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&run_dir)?;
    let base = Base::new()?;
    let mut stages = Vec::new();

    let clock = Instant::now();
    stages.push(stage("p13_clean_host", clock, clean_host(&base)));

    let clock = Instant::now();
    let plan = accept_plan(&base);
    let result = match &plan {
        Ok(plan) => killed_in_download(&base, plan),
        Err(StageStop::Failed(reason) | StageStop::Blocked(reason)) => blocked(reason),
    };
    stages.push(stage("p13_install_killed_in_download", clock, result));
    let plan = plan.ok();

    let clock = Instant::now();
    let result = plan.as_ref().map_or_else(
        || blocked("no accepted plan"),
        |plan| killed_in_smoke(&base, plan),
    );
    stages.push(stage("p13_install_killed_in_smoke", clock, result));

    let clock = Instant::now();
    let result = plan.as_ref().map_or_else(
        || blocked("no accepted plan"),
        |plan| rerun_completes(&base, plan),
    );
    let installed = result.is_ok();
    stages.push(stage("p13_install_rerun_completes", clock, result));

    let clock = Instant::now();
    let result = if installed {
        managed_journey(&base)
    } else {
        blocked("the managed install did not complete")
    };
    stages.push(stage("p13_managed_journey", clock, result));

    let clock = Instant::now();
    let result = match (&plan, installed) {
        (Some(plan), true) => uninstall_and_reinstall(&base, plan),
        _ => blocked("the managed install did not complete"),
    };
    stages.push(stage("p13_uninstall_and_reinstall", clock, result));

    let status_of = |wanted: &str| stages.iter().any(|entry| entry["status"] == wanted);
    let overall = if status_of("failed") {
        "failed"
    } else if status_of("blocked") {
        "blocked"
    } else {
        "passed"
    };
    let manifest: Value =
        serde_json::from_slice(&fs::read(repository.join("fixtures/corpus/manifest.json"))?)?;
    let report = json!({
        "schema_version": 1,
        "checkpoint": "P13 installed-user and managed-dependency run",
        "fixture_manifest": {
            "schema_version": manifest["schema_version"],
            "corpus_id": manifest["corpus_id"],
        },
        "fixtures": "F05-speech.mp4 and F01-speech.mp4",
        "os": env::consts::OS,
        "architecture": env::consts::ARCH,
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "resource_profile": "each CLI call killed after 30 min; empty PATH; nothing configured; no Rust toolchain reachable from the binary",
        "vsift_version": env!("CARGO_PKG_VERSION"),
        "authorization": "opt-in cargo test invocation (VSIFT_P13_INSTALL_E2E=1) accepting the reviewed plan's digest; downloads only the pinned catalogue artifacts into isolated temporary per-user bases",
        "prior_checkpoints": ["P06: p06_setup_e2e", "P07: p07_local_asr_e2e", "P08-P11 checkpoints", "P12: p12_skill_procedure_e2e"],
        "stages": stages,
        "coverage_gaps": [
            "the clean install of the native archive and of the npm packages without Rust is P13 PRs 9 and 11",
            "power loss is qualified by the P13 managed power loss workflow, not by this stage",
            "an agent run from a clean install is P14's release checkpoint (ADR 0023 decision H10)"
        ],
        "future_stages": [{"name": "p14_release_qualification", "status": "not_implemented"}],
        "overall": overall,
        "complete_journey": "not_implemented",
        "elapsed_ms": started.elapsed().as_millis(),
    });
    let report_path = run_dir.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    println!("P13 install checkpoint report: {}", report_path.display());
    println!("p13_install: {overall}");
    if overall == "passed" {
        Ok(())
    } else {
        Err(format!(
            "P13 install checkpoint {overall}; see {}",
            report_path.display()
        )
        .into())
    }
}
