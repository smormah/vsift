//! Opt-in P13 checkpoint: a real managed installation through the compiled
//! binary on a host with a catalogue (D-02, D-03, D-06, D-07; ADR 0023 §3;
//! ADR 0032: Ubuntu 24.04 x86-64, Windows x86-64 and macOS arm64). On a host
//! whose catalogue offered fewer components it would install and check what
//! the plan offers, and the local-ASR part would wait; every catalogue offers
//! all three today.
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
//! of what that costs. With whisper.cpp installed it then transcribes F05's
//! speech variant with the managed tools alone, and the transcript must hear
//! its words: the transcription check of each target (ADR 0032). It sends no credentials and no personal detail: the
//! publisher requests carry only `VSift`'s neutral user agent.
//!
//! `VSIFT_P13_REAL_INSTALL=1 cargo test --release --locked -p vsift-cli
//! --test p13_managed_install_real -- --ignored --exact --nocapture`
//!
//! A development build reaches no publisher (the P13 network guard), so the
//! checkpoint runs `--release`.
//!
//! The manual workflow `P13 managed smoke` runs it on a hosted runner.
//!
//! A second checkpoint does the same with `--include online_links` (ADR 0028
//! section 4, ADR 0032): the real yt-dlp and Deno are downloaded, smoked
//! (offline, no URL is fetched) and activated after the core, `setup check`
//! reports them as managed, `setup list` shows them, and removing the
//! runtime leaves the local-video readiness as it was.
//!
//! A further checkpoint does the same with `--include ocr` on every system
//! with a catalogue (ADR 0030, ADR 0032, W5b): the OCR runner archive is
//! downloaded, smoked (`describe`, which hashes its library and models) and
//! activated after the core, `setup check` reports the group as installed and
//! managed, and `vsift ocr` then reads F05's on-screen text with the managed
//! runner alone (the `PATH` is empty): the index must name the pinned runner,
//! the pinned models and ONNX Runtime 1.31.0, and `ocr search` must find the
//! invoice number the clip draws for its whole length.
//!
//! A third checkpoint is for Windows and macOS, where no managed yt-dlp is
//! offered (ADR 0028 section 4.6): the publisher's pinned `yt-dlp.exe` or
//! `yt-dlp_macos` (2026.08.19), fetched by the workflow, is registered with
//! `setup configure yt-dlp`, and `setup check` must run it under `VSift`'s
//! restricted environment and report it as user-managed with that version
//! and the pinned digest. No page is fetched.

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
use vsift_domain::ManagedComponent;
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
        // The Windows OCR job reads the installed library's imports after the
        // test (W5b, L-225): it asks for the folder to be kept and removes it
        // itself.
        if env::var_os("VSIFT_P13_KEEP_BASE").is_none() {
            let _ = fs::remove_dir_all(&self.0);
        }
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
    // Every core component, or what a smaller catalogue would offer.
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
    time_managed_use(&base)?;
    if complete {
        assert_transcription_works(&base)?;
    }
    Ok(())
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
    let check = base.vsift()?.args(["setup", "check", "--json"]).output()?;
    let check: Value = serde_json::from_slice(&check.stdout)?;
    println!("{}", serde_json::to_string_pretty(&check)?);
    assert_managed_lookups(&check, complete)?;

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

/// `ffmpeg` and `ffprobe`, and `whisper` when the plan installed it, each
/// resolve to the managed version in a `setup check --json` response, which is
/// a versioned contract. A tool found any other way (an explicit or
/// configured path, or the filtered `PATH`) fails here, and so does a missing
/// row. The readable table is not read: its wording is not a contract.
fn assert_managed_lookups(check: &Value, complete: bool) -> TestResult {
    let dependencies = check["dependencies"].as_array().ok_or("no dependencies")?;
    let required: &[&str] = if complete {
        &["ffmpeg", "ffprobe", "whisper"]
    } else {
        &["ffmpeg", "ffprobe"]
    };
    for name in required {
        let dependency = dependencies
            .iter()
            .find(|dependency| dependency["dependency"] == *name)
            .ok_or_else(|| format!("no {name} row in the check"))?;
        if dependency["lookup"] != "managed_version" || dependency["status"] != "available" {
            return Err(
                format!("{name} is not the available managed version: {dependency}").into(),
            );
        }
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

/// The transcription check: F05's speech variant, opened with a plain
/// `ingest` and transcribed by `transcript retranscribe` on the managed tools
/// alone (the `PATH` is empty), must hear its words. On macOS this is also the
/// Metal check: `VSift` runs whisper.cpp with the GPU off (`-ng`), so it must
/// transcribe on the CPU whether or not the runner has Metal.
fn assert_transcription_works(base: &Base) -> TestResult {
    let video = fs::canonicalize(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/corpus/generated/F05-speech.mp4"),
    )?;
    let data = |arguments: &[&str]| -> Result<Value, Box<dyn Error>> {
        let output = base.vsift()?.args(arguments).arg("--json").output()?;
        let value: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(
            output.status.code(),
            Some(0),
            "{arguments:?}: {}",
            value["error"]
        );
        Ok(value["data"].clone())
    };
    let opened = data(&["ingest", path_text(&video)?])?;
    let session = opened["session_id"]
        .as_str()
        .ok_or("ingest returned no session")?
        .to_owned();
    let started = Instant::now();
    let retranscribed = data(&["transcript", "retranscribe", &session])?;
    println!(
        "transcription: {:.1} s, {}",
        started.elapsed().as_secs_f64(),
        serde_json::to_string_pretty(&retranscribed["revision"]["local_asr"])?
    );
    let page = data(&[
        "transcript",
        "get",
        &session,
        "--from",
        "0",
        "--to",
        "20000000",
        "--limit",
        "100",
    ])?;
    let heard = page["items"]
        .as_array()
        .ok_or("no transcript items")?
        .iter()
        .filter_map(|item| item["text"].as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    println!("heard: {heard}");
    for word in ["success banner", "submit"] {
        assert!(
            heard.contains(word),
            "the transcript lacks {word:?}: {heard}"
        );
    }
    Ok(())
}

/// The checkpoint with the optional online-links group. Ubuntu 24.04 only: the
/// managed yt-dlp and Deno are offered nowhere else.
#[test]
#[ignore = "opt-in real managed installation with online_links; set VSIFT_P13_REAL_INSTALL=1 on Ubuntu 24.04 x86-64 and run --release"]
fn real_managed_install_with_online_links_installs_reports_and_removes_the_pair() -> TestResult {
    if env::var_os(OPT_IN).is_none() {
        return Err(format!("set {OPT_IN}=1 to run the real managed installation").into());
    }
    if accepted_host_catalogue()?.is_none() {
        return Err("the real managed installation runs only on a host with a catalogue".into());
    }
    println!("binary under test: {}", published_binary::report()?);
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let base = Base(env::temp_dir().join(format!("vsift-p13-real-online-links-{stamp}")));
    fs::create_dir(&base.0)?;

    let plan = base
        .vsift()?
        .args(["setup", "plan", "--profile", "desktop"])
        .args(["--include", "online_links", "--json"])
        .output()?;
    assert!(plan.status.success(), "setup plan failed");
    let plan_value: Value = serde_json::from_slice(&plan.stdout)?;
    let digest = plan_value["data"]["plan_digest"]
        .as_str()
        .ok_or("the plan has no digest")?
        .to_owned();
    let group = &plan_value["data"]["optional_components"][0];
    assert_eq!(group["name"], "online_links");
    assert_eq!(group["status"], "included", "{group}");
    let optional_actions = group["actions"].as_array().map_or(0, Vec::len);
    assert_eq!(optional_actions, 2, "yt-dlp and Deno: {group}");
    let plan_path = base.0.join("plan.json");
    fs::write(&plan_path, &plan.stdout)?;

    let started = Instant::now();
    let install = base
        .vsift()?
        .args(["setup", "install", "--plan", path_text(&plan_path)?])
        .args(["--accept-plan", &digest, "--json"])
        .output()?;
    println!(
        "install: exit {:?} in {:.1} s",
        install.status.code(),
        started.elapsed().as_secs_f64()
    );
    let installed: Value = serde_json::from_slice(&install.stdout)?;
    println!("{}", serde_json::to_string_pretty(&installed["data"])?);
    assert_eq!(install.status.code(), Some(0), "{}", installed["error"]);
    assert_eq!(installed["status"], "complete", "no partial result");
    assert_eq!(installed["data"]["core"], "ready");
    let optional = installed["data"]["optional_components"]
        .as_array()
        .ok_or("no optional components")?;
    assert_eq!(optional.len(), optional_actions);
    for component in optional {
        assert_eq!(component["status"], "activated", "{component}");
    }

    let managed_row = |base: &Base| -> Result<Value, Box<dyn Error>> {
        let check = base.vsift()?.args(["setup", "check", "--json"]).output()?;
        assert_eq!(check.status.code(), Some(0));
        let value: Value = serde_json::from_slice(&check.stdout)?;
        assert_eq!(value["status"], "ready");
        value["optional_components"]
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["name"] == "online_links"))
            .cloned()
            .ok_or_else(|| "no online_links row".into())
    };
    let row = managed_row(&base)?;
    assert_eq!(row["status"], "installed", "{row}");
    let components = row["components"].as_array().ok_or("no components")?;
    let names: Vec<&str> = components
        .iter()
        .filter_map(|component| component["component"].as_str())
        .collect();
    assert_eq!(names, ["js_runtime_deno", "yt_dlp"]);
    assert!(components.iter().all(|c| c["source"] == "managed"));

    let list = base.vsift()?.args(["setup", "list", "--json"]).output()?;
    assert_eq!(list.status.code(), Some(0));
    let listed = String::from_utf8(list.stdout)?;
    assert!(listed.contains("yt_dlp") && listed.contains("js_runtime_deno"));

    // Removing the runtime takes only the runtime; local investigation is
    // as ready as before.
    let removed = base
        .vsift()?
        .args(["setup", "remove", "js_runtime_deno", "--json"])
        .output()?;
    assert_eq!(removed.status.code(), Some(0));
    let row = managed_row(&base)?;
    assert_eq!(row["status"], "not_installed", "{row}");
    Ok(())
}

/// The SHA-256 digests the managed OCR runner must read with: the pinned runner
/// executable and the two pinned models, from the host's own catalogue.
struct PinnedOcr {
    runner: String,
    detector: String,
    recognizer: String,
}

impl PinnedOcr {
    fn of_host() -> Result<Self, Box<dyn Error>> {
        let catalogue = accepted_host_catalogue()?
            .ok_or("the host has no managed catalogue to install from")?;
        let runner = catalogue
            .optional_artifacts
            .iter()
            .find(|artifact| artifact.component == ManagedComponent::OcrRunner)
            .ok_or("the catalogue offers no OCR runner")?;
        let pinned = |predicate: &dyn Fn(&str, bool) -> bool| -> Result<String, Box<dyn Error>> {
            runner
                .files
                .iter()
                .find(|file| predicate(&file.name, file.executable))
                .map(|file| file.integrity.sha256_hex())
                .ok_or_else(|| "the pinned archive lacks a file".into())
        };
        Ok(Self {
            runner: pinned(&|_, executable| executable)?,
            detector: pinned(&|name, _| name == "ch_PP-OCRv5_det_mobile.onnx")?,
            recognizer: pinned(&|name, _| name == "latin_PP-OCRv5_rec_mobile.onnx")?,
        })
    }
}

/// Plans with `--include ocr` and installs the plan: the group is included
/// with its one action, and the install leaves the core ready and the runner
/// activated.
fn install_the_ocr_group(base: &Base) -> TestResult {
    let plan = base
        .vsift()?
        .args(["setup", "plan", "--profile", "desktop"])
        .args(["--include", "ocr", "--json"])
        .output()?;
    assert!(plan.status.success(), "setup plan failed");
    let plan_value: Value = serde_json::from_slice(&plan.stdout)?;
    let digest = plan_value["data"]["plan_digest"]
        .as_str()
        .ok_or("the plan has no digest")?
        .to_owned();
    let group = plan_value["data"]["optional_components"]
        .as_array()
        .and_then(|groups| groups.iter().find(|group| group["name"] == "ocr"))
        .ok_or("the plan lists no ocr group")?;
    assert_eq!(group["status"], "included", "{group}");
    assert_eq!(group["installs"], serde_json::json!(["ocr_runner"]));
    assert_eq!(
        group["actions"].as_array().map_or(0, Vec::len),
        1,
        "{group}"
    );
    let plan_path = base.0.join("plan.json");
    fs::write(&plan_path, &plan.stdout)?;

    let started = Instant::now();
    let install = base
        .vsift()?
        .args(["setup", "install", "--plan", path_text(&plan_path)?])
        .args(["--accept-plan", &digest, "--json"])
        .output()?;
    println!(
        "install: exit {:?} in {:.1} s",
        install.status.code(),
        started.elapsed().as_secs_f64()
    );
    let installed: Value = serde_json::from_slice(&install.stdout)?;
    println!("{}", serde_json::to_string_pretty(&installed["data"])?);
    assert_eq!(install.status.code(), Some(0), "{}", installed["error"]);
    assert_eq!(installed["status"], "complete", "no partial result");
    assert_eq!(installed["data"]["core"], "ready");
    let optional = installed["data"]["optional_components"]
        .as_array()
        .ok_or("no optional components")?;
    assert_eq!(optional.len(), 1);
    assert_eq!(optional[0]["component"], "ocr_runner");
    assert_eq!(optional[0]["status"], "activated", "{}", optional[0]);
    Ok(())
}

/// `setup check` reports the group as installed from the managed store.
fn assert_the_ocr_row_is_installed(base: &Base) -> TestResult {
    let check = base.vsift()?.args(["setup", "check", "--json"]).output()?;
    assert_eq!(check.status.code(), Some(0));
    let check: Value = serde_json::from_slice(&check.stdout)?;
    assert_eq!(check["status"], "ready");
    let row = check["optional_components"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["name"] == "ocr"))
        .ok_or("no ocr row in the check")?;
    assert_eq!(row["status"], "installed", "{row}");
    let components = row["components"].as_array().ok_or("no components")?;
    assert_eq!(components.len(), 1);
    assert_eq!(components[0]["component"], "ocr_runner");
    assert_eq!(components[0]["source"], "managed");
    Ok(())
}

/// `ocr` and `ocr search` on F05 with the managed runner alone: the `PATH` is
/// empty and no runner is registered or given. The index must name the pinned
/// runner and models, and the search must find the invoice number F05 draws
/// for its whole length (the corpus README, the P12 truth amendment): the
/// number or the word, whichever the recogniser reads whole.
fn assert_the_managed_runner_reads_f05(base: &Base, pinned: &PinnedOcr) -> TestResult {
    let data = |arguments: &[&str]| -> Result<Value, Box<dyn Error>> {
        let output = base.vsift()?.args(arguments).arg("--json").output()?;
        let value: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(
            output.status.code(),
            Some(0),
            "{arguments:?}: {}",
            value["error"]
        );
        Ok(value)
    };
    let video = fs::canonicalize(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/generated/F05.mp4"),
    )?;
    let session = data(&["ingest", path_text(&video)?])?["data"]["session_id"]
        .as_str()
        .ok_or("ingest returned no session")?
        .to_owned();
    let started = Instant::now();
    let mut answer = data(&["ocr", &session])?;
    for _ in 0..4 {
        if answer["status"] == "complete" {
            break;
        }
        answer = data(&["ocr", &session])?;
    }
    println!(
        "ocr: {:.1} s, {}",
        started.elapsed().as_secs_f64(),
        serde_json::to_string_pretty(&answer["data"])?
    );
    assert_eq!(
        answer["status"], "complete",
        "{}",
        answer["data"]["coverage"]
    );
    assert_eq!(answer["data"]["coverage"]["pending"], 0);
    let engine = &answer["data"]["index"]["engine"];
    assert_eq!(
        engine["runner_sha256"], pinned.runner,
        "the managed runner read"
    );
    assert_eq!(engine["detector_sha256"], pinned.detector);
    assert_eq!(engine["recognizer_sha256"], pinned.recognizer);
    assert_eq!(engine["runtime_version"], "1.31.0");
    assert!(
        answer["data"]["index"]["lines"].as_u64().unwrap_or(0) > 0,
        "the runner read no line of F05"
    );
    let mut hits = 0_usize;
    for query in ["4407", "invoice"] {
        let found = data(&["ocr", "search", &session, &format!("--query={query}")])?;
        let count = found["data"]["hits"].as_array().map_or(0, Vec::len);
        println!("ocr search {query:?}: {count} hits");
        hits += count;
    }
    assert!(
        hits > 0,
        "ocr search found neither `4407` nor `invoice` on F05"
    );
    Ok(())
}

/// The checkpoint with the optional `ocr` group, on every system with a
/// catalogue: a real install of the group, `setup check`, then `ocr` and `ocr
/// search` through the managed runner alone.
#[test]
#[ignore = "opt-in real managed installation with ocr; set VSIFT_P13_REAL_INSTALL=1 on a host with a managed catalogue and run --release"]
fn real_managed_install_with_ocr_installs_and_reads_on_screen_text() -> TestResult {
    if env::var_os(OPT_IN).is_none() {
        return Err(format!("set {OPT_IN}=1 to run the real managed installation").into());
    }
    let pinned = PinnedOcr::of_host()?;
    println!("binary under test: {}", published_binary::report()?);
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let base = Base(env::temp_dir().join(format!("vsift-p13-real-ocr-{stamp}")));
    fs::create_dir(&base.0)?;

    install_the_ocr_group(&base)?;
    assert_the_ocr_row_is_installed(&base)?;
    assert_the_managed_runner_reads_f05(&base, &pinned)
}

/// The pinned publisher file for this system (ADR 0028 section 4.1): its
/// name, size and SHA-256, from the release's signed sums.
#[cfg(windows)]
const PINNED_USER_YT_DLP: (u64, &str) = (
    17_840_399,
    "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a",
);
#[cfg(target_os = "macos")]
const PINNED_USER_YT_DLP: (u64, &str) = (
    37_146_048,
    "0f192b7ec147ab6288885d6351d9ab67367640029b4377576ef46dd79cf7b202",
);

/// A yt-dlp the user supplies, as on Windows and macOS. The workflow fetches
/// the pinned file and checks its size; the digest `setup check` reports is
/// the proof of its bytes.
#[cfg(any(windows, target_os = "macos"))]
#[test]
#[ignore = "opt-in: set VSIFT_P13_REAL_USER_YT_DLP=1 and VSIFT_P13_USER_YT_DLP to the pinned yt-dlp.exe or yt-dlp_macos of 2026.08.19, on Windows or macOS"]
fn real_user_supplied_yt_dlp_is_registered_and_reported_as_user_managed() -> TestResult {
    if env::var_os("VSIFT_P13_REAL_USER_YT_DLP").is_none() {
        return Err(
            "set VSIFT_P13_REAL_USER_YT_DLP=1 to run the user-supplied yt-dlp check".into(),
        );
    }
    let supplied = PathBuf::from(
        env::var_os("VSIFT_P13_USER_YT_DLP").ok_or("set VSIFT_P13_USER_YT_DLP to the file")?,
    );
    let (bytes, digest) = PINNED_USER_YT_DLP;
    assert_eq!(fs::metadata(&supplied)?.len(), bytes, "not the pinned file");
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let base = Base(env::temp_dir().join(format!("vsift-p13-real-user-ytdlp-{stamp}")));
    fs::create_dir(&base.0)?;

    let configured = base
        .vsift()?
        .args(["setup", "configure", "yt-dlp", "--executable"])
        .arg(&supplied)
        .arg("--json")
        .output()?;
    assert_eq!(configured.status.code(), Some(0));
    let configured: Value = serde_json::from_slice(&configured.stdout)?;
    assert_eq!(configured["data"]["dependency"], "yt_dlp");

    let started = Instant::now();
    let check = base
        .vsift()?
        .args(["setup", "check", "--json", "--timeout-seconds", "60"])
        .output()?;
    println!(
        "setup check: exit {:?} in {:.1} s",
        check.status.code(),
        started.elapsed().as_secs_f64()
    );
    let value: Value = serde_json::from_slice(&check.stdout)?;
    let row = value["optional_components"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["name"] == "online_links"))
        .ok_or("no online_links row")?;
    println!("{}", serde_json::to_string_pretty(row)?);
    assert_eq!(row["status"], "installed", "{row}");
    let component = &row["components"][0];
    assert_eq!(component["component"], "yt_dlp");
    assert_eq!(component["source"], "user_managed");
    assert_eq!(component["version"], "2026.08.19");
    assert_eq!(component["sha256"], digest);
    assert!(
        row["next_step"]
            .as_str()
            .is_some_and(|step| step.contains("No JavaScript runtime is registered")),
        "{row}"
    );
    Ok(())
}

/// The managed-version check is not satisfied by a tool found another way,
/// by one that is not available, or by a missing row, and is by the managed
/// version; it needs no network and no install.
#[test]
fn a_tool_found_another_way_does_not_pass_the_managed_check() -> TestResult {
    let row = |name: &str, lookup: &str, status: &str| serde_json::json!({"dependency": name, "lookup": lookup, "status": status});
    let managed = |names: &[&str]| {
        serde_json::json!({"dependencies": names
            .iter()
            .map(|name| row(name, "managed_version", "available"))
            .collect::<Vec<_>>()})
    };
    assert_managed_lookups(&managed(&["ffmpeg", "ffprobe"]), false)?;
    assert_managed_lookups(&managed(&["ffmpeg", "ffprobe", "whisper"]), true)?;

    let found_on_path = serde_json::json!({"dependencies": [
        row("ffmpeg", "managed_version", "available"),
        row("ffprobe", "filtered_path", "available"),
    ]});
    assert!(assert_managed_lookups(&found_on_path, false).is_err());
    let unavailable = serde_json::json!({"dependencies": [
        row("ffmpeg", "managed_version", "available"),
        row("ffprobe", "managed_version", "missing"),
    ]});
    assert!(assert_managed_lookups(&unavailable, false).is_err());
    assert!(assert_managed_lookups(&managed(&["ffmpeg"]), false).is_err());
    // Whisper counts only when the plan installed it.
    assert!(assert_managed_lookups(&managed(&["ffmpeg", "ffprobe"]), true).is_err());
    Ok(())
}
