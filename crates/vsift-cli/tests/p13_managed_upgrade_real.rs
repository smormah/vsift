//! Opt-in checkpoint: a real managed install made by an earlier release is
//! upgraded to this build's pins on Ubuntu 24.04 x86-64 (the whisper.cpp
//! re-pin of `0.2.3`, L-137).
//!
//! From a fresh per-user base with an empty `PATH`:
//!
//! 1. **The earlier release** (the published binary that `VSIFT_E2E_BINARY`
//!    selects, checked by `published_binary`; `0.2.2` in the workflow) plans
//!    and installs its own catalogue from the publishers: `FFmpeg`, whisper.cpp
//!    v1.9.2 and the model.
//! 2. **This build's `setup check`** must use that install and name its
//!    whisper.cpp as superseded (`managed_updates`: the v1.9.2 key, the v1.9.5
//!    pin, `security_fixes`), in JSON and in text.
//! 3. **This build's `setup plan` and `setup install`** must install only
//!    whisper.cpp v1.9.5 (the media tools and the model are already current)
//!    and select it.
//! 4. **This build's `setup check`** must then name `whisper.cpp v1.9.5
//!    (reviewed build)`, report no update, and pass the local-ASR
//!    verification run now (the recogniser's identity changed, so no earlier
//!    pass counts); `setup list` keeps v1.9.2 as the previous version.
//! 5. **A transcription works:** plain `ingest` of F05's speech variant and
//!    `transcript retranscribe` on the managed tools hear its words.
//!
//! Every download is the publisher's pinned file, verified by size and
//! SHA-256 by the binary that asks for it; the requests carry only `VSift`'s
//! neutral user agent, and nothing is downloaded outside the hosted runner.
//!
//! ```console
//! VSIFT_P13_REAL_UPGRADE=1 VSIFT_E2E_BINARY=<published vsift> \
//! VSIFT_E2E_EXPECTED_VERSION=0.2.2 VSIFT_E2E_EXPECTED_COMMIT=<its tag commit> \
//! cargo test --release --locked -p vsift-cli --test p13_managed_upgrade_real \
//!   -- --ignored --exact --nocapture
//! ```
//!
//! A development build reaches no publisher (the P13 network guard), so the
//! checkpoint runs `--release`. The workflow `P13 managed smoke` runs it.

mod published_binary;

use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Output,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;
use vsift_domain::ManagedTarget;
use vsift_infrastructure::detect_managed_target;

type TestResult = Result<(), Box<dyn Error>>;

const OPT_IN: &str = "VSIFT_P13_REAL_UPGRADE";
/// The downloads, extraction, smoke and local-ASR check of one install.
const DEADLINE: Duration = Duration::from_mins(30);
/// The earlier release's whisper.cpp, and the pin that replaces it.
const EARLIER_WHISPER: &str = "whisper.cpp-v1.9.2-ubuntu-x64";
const PINNED_WHISPER: &str = "whisper.cpp-v1.9.5-ubuntu-x64";

struct Base(PathBuf);

impl Base {
    fn prepare(&self, mut command: Command) -> Command {
        command
            .env("PATH", "")
            .env("HOME", &self.0)
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env("XDG_DATA_HOME", self.0.join("data"))
            .env("XDG_CACHE_HOME", self.0.join("cache"))
            .timeout(DEADLINE)
            .write_stdin("");
        command
    }

    /// The earlier release, which the override names.
    fn earlier(&self) -> Result<Command, Box<dyn Error>> {
        Ok(self.prepare(published_binary::command()?))
    }

    /// This build.
    fn current(&self) -> Result<Command, Box<dyn Error>> {
        Ok(self.prepare(Command::cargo_bin("vsift")?))
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

fn json(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
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

/// `setup plan`, then `setup install` of exactly that plan, by one binary.
fn plan_and_install(
    base: &Base,
    mut binary: impl FnMut() -> Result<Command, Box<dyn Error>>,
    name: &str,
) -> Result<Value, Box<dyn Error>> {
    let plan = binary()?
        .args(["setup", "plan", "--profile", "desktop", "--json"])
        .output()?;
    assert!(plan.status.success(), "{name}: setup plan failed");
    let plan_value = json(&plan)?;
    println!(
        "{name} plan: catalogue {}, actions {}",
        plan_value["data"]["catalogue_revision"],
        serde_json::to_string(&plan_value["data"]["actions"].as_array().map(|actions| {
            actions
                .iter()
                .map(|action| {
                    (
                        action["component"].clone(),
                        action["version"].clone(),
                        action["state"].clone(),
                    )
                })
                .collect::<Vec<_>>()
        }))?
    );
    let digest = plan_value["data"]["plan_digest"]
        .as_str()
        .ok_or("the plan has no digest")?
        .to_owned();
    let plan_path = base.0.join(format!("plan-{name}.json"));
    fs::write(&plan_path, &plan.stdout)?;
    let install = binary()?
        .args(["setup", "install", "--plan", path_text(&plan_path)?])
        .args(["--accept-plan", &digest, "--json"])
        .output()?;
    let install_value = json(&install)?;
    println!(
        "{name} install: {}",
        serde_json::to_string_pretty(&install_value["data"])?
    );
    assert_eq!(
        install.status.code(),
        Some(0),
        "{name}: {}",
        install_value["error"]
    );
    Ok(install_value)
}

fn whisper_entry(check: &Value) -> Result<&Value, Box<dyn Error>> {
    check["dependencies"]
        .as_array()
        .ok_or("no dependencies")?
        .iter()
        .find(|dependency| dependency["dependency"] == "whisper")
        .ok_or_else(|| "no whisper entry".into())
}

fn data(base: &Base, arguments: &[&str]) -> Result<Value, Box<dyn Error>> {
    let output = base.current()?.args(arguments).arg("--json").output()?;
    let value = json(&output)?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{arguments:?}: {}",
        value["error"]
    );
    Ok(value["data"].clone())
}

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
#[ignore = "opt-in real managed upgrade; set VSIFT_P13_REAL_UPGRADE=1 and VSIFT_E2E_BINARY (the earlier release) on Ubuntu 24.04 x86-64 and run --release"]
fn an_earlier_releases_managed_install_upgrades_to_the_current_pins() -> TestResult {
    if env::var_os(OPT_IN).is_none() {
        return Err(format!("set {OPT_IN}=1 to run the real managed upgrade").into());
    }
    if env::var_os(published_binary::BINARY_VARIABLE).is_none() {
        return Err(format!(
            "set {} to the earlier release's executable; this checkpoint upgrades its install",
            published_binary::BINARY_VARIABLE
        )
        .into());
    }
    if detect_managed_target() != ManagedTarget::Ubuntu2404X86_64 {
        return Err("the real managed upgrade runs only on Ubuntu 24.04 x86-64".into());
    }
    println!("earlier release: {}", published_binary::report()?);
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let base = Base(env::temp_dir().join(format!("vsift-p13-real-upgrade-{stamp}")));
    fs::create_dir(&base.0)?;

    // 1. The earlier release installs its own catalogue.
    let earlier = plan_and_install(&base, || base.earlier(), "earlier")?;
    assert_eq!(statuses(&earlier), ["activated", "activated", "activated"]);

    assert_superseded_before_the_upgrade(&base)?;

    // 3. This build's plan installs only the new whisper.cpp.
    let upgrade = plan_and_install(&base, || base.current(), "current")?;
    assert_eq!(
        statuses(&upgrade),
        ["already_current", "activated", "already_current"]
    );

    assert_upgraded(&base)?;
    assert_transcription_works(&base)
}

/// Step 2: this build names the earlier whisper.cpp as superseded.
fn assert_superseded_before_the_upgrade(base: &Base) -> TestResult {
    let before = json(
        &base
            .current()?
            .args(["setup", "check", "--json"])
            .output()?,
    )?;
    println!("check before: {}", serde_json::to_string_pretty(&before)?);
    let whisper = whisper_entry(&before)?;
    assert_eq!(whisper["lookup"], "managed_version");
    assert_eq!(whisper["detail"], "whisper.cpp v1.9.2 (reviewed build)");
    let updates = before["managed_updates"]
        .as_array()
        .ok_or("no managed_updates")?;
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert_eq!(updates[0]["component"], "whisper_cli");
    assert_eq!(updates[0]["selected_version"], EARLIER_WHISPER);
    assert_eq!(updates[0]["reviewed_version"], PINNED_WHISPER);
    assert_eq!(updates[0]["reason"], "security_fixes");
    let text = String::from_utf8(base.current()?.args(["setup", "check"]).output()?.stdout)?;
    println!("{text}");
    assert!(
        text.contains(&format!(
            "[update, security] whisper_cli: the selected managed version is {EARLIER_WHISPER}"
        )),
        "{text}"
    );

    Ok(())
}

/// Step 4: the new pin is selected, verified now, and nothing is superseded.
fn assert_upgraded(base: &Base) -> TestResult {
    let after = json(
        &base
            .current()?
            .args(["setup", "check", "--json"])
            .output()?,
    )?;
    println!("check after: {}", serde_json::to_string_pretty(&after)?);
    assert_eq!(after["status"], "ready");
    let whisper = whisper_entry(&after)?;
    assert_eq!(whisper["lookup"], "managed_version");
    assert_eq!(whisper["detail"], "whisper.cpp v1.9.5 (reviewed build)");
    assert_eq!(after["managed_updates"], serde_json::json!([]));
    assert_eq!(after["local_asr"]["verification"]["status"], "verified");
    assert_eq!(after["local_asr"]["verification"]["source"], "ran_now");
    let list = data(base, &["setup", "list"])?;
    let cli = list["components"]
        .as_array()
        .ok_or("no components")?
        .iter()
        .find(|component| component["component"] == "whisper_cli")
        .ok_or("no whisper_cli in setup list")?;
    assert_eq!(cli["selected_version"], PINNED_WHISPER);
    assert_eq!(cli["previous_version"], EARLIER_WHISPER);

    Ok(())
}

/// Step 5: a transcription on the upgraded tools.
fn assert_transcription_works(base: &Base) -> TestResult {
    let video = repository()
        .join("fixtures/corpus/generated/F05-speech.mp4")
        .canonicalize()?;
    let opened = data(base, &["ingest", path_text(&video)?])?;
    let session = opened["session_id"]
        .as_str()
        .ok_or("ingest returned no session")?
        .to_owned();
    let retranscribed = data(base, &["transcript", "retranscribe", &session])?;
    println!(
        "retranscribe: {}",
        serde_json::to_string_pretty(&retranscribed["revision"]["local_asr"])?
    );
    let page = data(
        base,
        &[
            "transcript",
            "get",
            &session,
            "--from",
            "0",
            "--to",
            "20000000",
            "--limit",
            "100",
        ],
    )?;
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
