//! Opt-in P13 checkpoint: a real managed installation through the compiled
//! binary on a host with a catalogue (D-02, D-03, D-06, D-07; ADR 0023 §3;
//! ADR 0032: Ubuntu 24.04 x86-64 and Windows x86-64). On a host whose catalogue
//! offers fewer components (Windows until `VSift`'s whisper.cpp is pinned) it
//! installs and checks what the plan offers, and the local-ASR part waits.
//!
//! From a fresh per-user base with an empty `PATH`, it runs `setup plan`,
//! accepts the plan with `setup install --events jsonl` (which downloads the
//! three pinned publisher artifacts of the accepted catalogue, verifies each
//! by size and SHA-256, stages, smokes and activates them), then `setup
//! check`, which must resolve every tool as `managed_version` and pass the
//! local-ASR verification with the managed tools and model. A rerun of the
//! same accepted plan must report every component `already_current` and
//! download nothing, and `setup plan` must then show every component
//! current with nothing to install. Last it times a warm `setup check` and
//! a first and a warm `frame get` against the managed tools: every use
//! re-hashes the managed version (L-087), and these lines are the evidence
//! of what that costs. It sends no credentials and no personal detail: the
//! publisher requests carry only `VSift`'s neutral user agent.
//!
//! `VSIFT_P13_REAL_INSTALL=1 cargo test --release --locked -p vsift-cli
//! --test p13_managed_install_real -- --ignored --exact --nocapture`
//!
//! A development build reaches no publisher (the P13 network guard), so the
//! checkpoint runs `--release`.
//!
//! The manual workflow `P13 managed smoke` runs it on a hosted runner.

mod published_binary;

use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;
use vsift_infrastructure::accepted_host_catalogue;

type TestResult = Result<(), Box<dyn Error>>;

const OPT_IN: &str = "VSIFT_P13_REAL_INSTALL";
/// The downloads, extraction, smoke and local-ASR check of one run.
const INSTALL_DEADLINE: Duration = Duration::from_mins(30);

struct Base(PathBuf);

impl Base {
    fn vsift(&self) -> Result<Command, Box<dyn Error>> {
        let mut command = published_binary::command()?;
        command
            .env("PATH", "")
            .env("HOME", &self.0)
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env("XDG_DATA_HOME", self.0.join("data"))
            .env("XDG_CACHE_HOME", self.0.join("cache"))
            .env("LOCALAPPDATA", self.0.join("local"))
            .timeout(INSTALL_DEADLINE)
            .write_stdin("");
        Ok(command)
    }
}

impl Drop for Base {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn path_text(path: &Path) -> Result<&str, Box<dyn Error>> {
    path.to_str().ok_or_else(|| "non-UTF-8 test path".into())
}

fn statuses(result: &Value) -> Vec<String> {
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

#[test]
#[ignore = "opt-in real managed installation; set VSIFT_P13_REAL_INSTALL=1 on a host with a managed catalogue and run --release (a development build reaches no publisher)"]
fn real_managed_install_selects_every_tool_and_a_rerun_is_current() -> TestResult {
    if env::var_os(OPT_IN).is_none() {
        return Err(format!("set {OPT_IN}=1 to run the real managed installation").into());
    }
    if accepted_host_catalogue()?.is_none() {
        return Err("the real managed installation runs only on a host with a catalogue".into());
    }
    println!("binary under test: {}", published_binary::report()?);
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let base = Base(env::temp_dir().join(format!("vsift-p13-real-install-{stamp}")));
    fs::create_dir(&base.0)?;

    let plan = base
        .vsift()?
        .args(["setup", "plan", "--profile", "desktop", "--json"])
        .output()?;
    assert!(plan.status.success(), "setup plan failed");
    let plan_value: Value = serde_json::from_slice(&plan.stdout)?;
    let digest = plan_value["data"]["plan_digest"]
        .as_str()
        .ok_or("the plan has no digest")?
        .to_owned();
    let actions = plan_value["data"]["actions"].as_array().map_or(0, Vec::len);
    let planned: Vec<&str> = plan_value["data"]["actions"]
        .as_array()
        .map(|actions| {
            actions
                .iter()
                .filter_map(|action| action["component"].as_str())
                .collect()
        })
        .unwrap_or_default();
    // Every core component, or (Windows) what the catalogue offers.
    let complete = planned.contains(&"whisper_cli");
    println!(
        "plan: {actions} actions ({planned:?}), catalogue {}",
        plan_value["data"]["catalogue_revision"]
    );
    assert!(actions >= 1, "the plan installs nothing");
    let plan_path = base.0.join("plan.json");
    fs::write(&plan_path, &plan.stdout)?;

    let started = Instant::now();
    let install = base
        .vsift()?
        .args(["setup", "install", "--plan", path_text(&plan_path)?])
        .args(["--accept-plan", &digest, "--events", "jsonl"])
        .output()?;
    println!(
        "install: exit {:?} in {:.1} s",
        install.status.code(),
        started.elapsed().as_secs_f64()
    );
    let lines: Vec<Value> = String::from_utf8(install.stdout)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let (terminal, progress) = lines.split_last().ok_or("no events")?;
    let result = &terminal["result"];
    println!("{}", serde_json::to_string_pretty(&result["data"])?);
    assert_eq!(install.status.code(), Some(0), "{}", result["error"]);
    assert_eq!(result["status"], "complete");
    assert_eq!(statuses(result), vec![String::from("activated"); actions]);
    assert!(
        progress
            .iter()
            .any(|event| event["stage"] == "fetching_artifact"),
        "no download progress was reported"
    );
    assert!(
        progress
            .iter()
            .any(|event| event["stage"] == "installing_components" && event["completed"] == actions),
        "the component progress did not finish"
    );

    assert_checked(&base, complete)?;

    let started = Instant::now();
    let rerun = base
        .vsift()?
        .args(["setup", "install", "--plan", path_text(&plan_path)?])
        .args(["--accept-plan", &digest, "--json"])
        .output()?;
    let rerun_value: Value = serde_json::from_slice(&rerun.stdout)?;
    println!(
        "rerun: exit {:?} in {:.1} s",
        rerun.status.code(),
        started.elapsed().as_secs_f64()
    );
    assert_eq!(rerun.status.code(), Some(0));
    assert_eq!(
        statuses(&rerun_value),
        vec![String::from("already_current"); actions]
    );

    assert_installed(&base, &digest, actions, complete)?;
    time_managed_use(&base)
}

/// `setup check` after the install resolves every installed tool as the
/// managed version, and with whisper.cpp installed passes the local-ASR
/// verification.
fn assert_checked(base: &Base, complete: bool) -> TestResult {
    let check = base.vsift()?.args(["setup", "check", "--json"]).output()?;
    let check_value: Value = serde_json::from_slice(&check.stdout)?;
    println!("{}", serde_json::to_string_pretty(&check_value)?);
    assert_eq!(check.status.code(), Some(0));
    for dependency in check_value["dependencies"]
        .as_array()
        .ok_or("no dependencies")?
    {
        if dependency["dependency"] == "whisper" && !complete {
            continue;
        }
        assert_eq!(dependency["lookup"], "managed_version", "{dependency}");
        assert_eq!(dependency["status"], "available", "{dependency}");
    }
    if complete {
        assert_eq!(check_value["status"], "ready");
        assert_eq!(check_value["local_asr"]["model"]["status"], "known_pinned");
        assert_eq!(
            check_value["local_asr"]["verification"]["status"],
            "verified"
        );
    }

    Ok(())
}

/// After the install, `setup check` shows every tool as the managed version
/// and `setup plan` shows every component current with nothing to install,
/// under the accepted digest.
fn assert_installed(base: &Base, digest: &str, actions: usize, complete: bool) -> TestResult {
    let human = base.vsift()?.args(["setup", "check"]).output()?;
    let text = String::from_utf8(human.stdout)?;
    println!("{text}");
    // FFmpeg and FFprobe, and whisper.cpp when the plan installed it.
    assert_eq!(
        text.matches("[managed version]").count(),
        if complete { 3 } else { 2 }
    );

    let replanned = base
        .vsift()?
        .args(["setup", "plan", "--profile", "desktop", "--json"])
        .output()?;
    let replanned: Value = serde_json::from_slice(&replanned.stdout)?;
    let data = &replanned["data"];
    assert_eq!(data["plan_digest"].as_str(), Some(digest));
    assert_eq!(data["install_needed"], false);
    if complete {
        assert_eq!(data["readiness"], "ready");
        assert_eq!(data["local_asr_model"]["status"], "managed_current");
    }
    let replanned_actions = data["actions"].as_array().ok_or("no actions")?;
    assert_eq!(replanned_actions.len(), actions);
    for action in replanned_actions {
        assert_eq!(action["state"], "current", "{action}");
    }
    Ok(())
}

/// L-087 evidence: every use of a managed tool re-hashes its version, so
/// this times a warm `setup check`, then a first `frame get` (which also
/// verifies the media tools once) and a warm one on a corpus fixture.
fn time_managed_use(base: &Base) -> TestResult {
    let started = Instant::now();
    let warm = base.vsift()?.args(["setup", "check", "--json"]).output()?;
    println!(
        "L-087 warm setup check: exit {:?} in {:.2} s",
        warm.status.code(),
        started.elapsed().as_secs_f64()
    );
    assert_eq!(warm.status.code(), Some(0));

    let fixture = fs::canonicalize(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/generated/F01.mp4"),
    )?;
    let opened = base
        .vsift()?
        .args(["ingest", path_text(&fixture)?, "--json"])
        .output()?;
    assert_eq!(opened.status.code(), Some(0), "ingest failed");
    let opened: Value = serde_json::from_slice(&opened.stdout)?;
    let session = opened["data"]["session_id"]
        .as_str()
        .ok_or("ingest returned no session")?
        .to_owned();
    for run in ["first", "warm"] {
        let started = Instant::now();
        let frame = base
            .vsift()?
            .args(["frame", "get", &session, "--at", "0", "--json"])
            .output()?;
        println!(
            "L-087 {run} frame get: exit {:?} in {:.2} s",
            frame.status.code(),
            started.elapsed().as_secs_f64()
        );
        assert_eq!(
            frame.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&frame.stdout)
        );
    }
    Ok(())
}
