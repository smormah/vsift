//! Opt-in checkpoint: a real managed install made by an earlier release is
//! upgraded to this build's pins on Ubuntu 24.04 x86-64 (the whisper.cpp
//! re-pin of `0.2.3`, L-137, and the `FFmpeg` re-pin of `0.2.4`, L-132).
//!
//! From a fresh per-user base with an empty `PATH`:
//!
//! 1. **The earlier release** (the published binary that `VSIFT_E2E_BINARY`
//!    selects, checked by `published_binary`; `0.2.2` in the workflow) plans
//!    and installs its own catalogue from the publishers: `FFmpeg`, whisper.cpp
//!    v1.9.2 and the model. Where its catalogue pins an address that has since
//!    moved (the `FFmpeg` builds repository, L-254), the workflow hands it the
//!    files of its own plan with `VSIFT_E2E_ARTIFACT_DIR` (`--artifact-dir`).
//! 2. **This build's `setup check`** must use that install and name its
//!    `FFmpeg` (the build of 2026-08-31) and its whisper.cpp as superseded
//!    (`managed_updates`: each earlier key, each pin, `security_fixes`), in
//!    JSON and in text.
//! 3. **This build's `setup plan` and `setup install`** must install only
//!    the `FFmpeg` build of 2026-10-31 and whisper.cpp v1.9.5 (the model is
//!    already current) and select them.
//! 4. **This build's `setup check`** must then name `whisper.cpp v1.9.5
//!    (reviewed build)`, report no update, and pass the local-ASR
//!    verification run now (the recogniser's identity changed, so no earlier
//!    pass counts); `setup list` keeps the earlier `FFmpeg` and v1.9.2 as the
//!    previous versions.
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
    ffi::OsString,
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
/// The earlier releases' `FFmpeg` (`0.2.3` and before), and the pin of `0.2.4`.
const EARLIER_MEDIA: &str = "n9.0.1-11-ge47273f4d9-20260831";
const PINNED_MEDIA: &str = "n9.0.2-39-g7adfbe40dc-20261031";

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
    install_arguments: &[OsString],
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
        .args(install_arguments)
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
    // L-254: the earlier release pins an address that has since moved and
    // cannot be changed, so the job hands it the files of its own plan.
    let earlier_files = published_binary::artifact_dir_arguments()?;
    let earlier = plan_and_install(&base, || base.earlier(), "earlier", &earlier_files)?;
    assert_eq!(statuses(&earlier), ["activated", "activated", "activated"]);

    assert_superseded_before_the_upgrade(&base)?;

    // 3. This build's plan installs only the new FFmpeg and whisper.cpp.
    let upgrade = plan_and_install(&base, || base.current(), "current", &[])?;
    assert_eq!(
        statuses(&upgrade),
        ["activated", "activated", "already_current"]
    );

    assert_upgraded(&base)?;
    assert_transcription_works(&base)
}

/// Step 2: this build names the earlier `FFmpeg` and whisper.cpp as superseded.
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
    assert_eq!(updates.len(), 2, "{updates:?}");
    let text = String::from_utf8(base.current()?.args(["setup", "check"]).output()?.stdout)?;
    println!("{text}");
    for (component, earlier, pinned) in [
        ("ffmpeg_ffprobe", EARLIER_MEDIA, PINNED_MEDIA),
        ("whisper_cli", EARLIER_WHISPER, PINNED_WHISPER),
    ] {
        let update = updates
            .iter()
            .find(|update| update["component"] == component)
            .ok_or_else(|| format!("no update for {component}: {updates:?}"))?;
        assert_eq!(update["selected_version"], earlier);
        assert_eq!(update["reviewed_version"], pinned);
        assert_eq!(update["reason"], "security_fixes");
        assert!(
            text.contains(&format!(
                "Update available (security fixes): {component} is on {earlier}"
            )),
            "{text}"
        );
    }

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
    let components = list["components"].as_array().ok_or("no components")?;
    for (name, earlier, pinned) in [
        ("ffmpeg_ffprobe", EARLIER_MEDIA, PINNED_MEDIA),
        ("whisper_cli", EARLIER_WHISPER, PINNED_WHISPER),
    ] {
        let component = components
            .iter()
            .find(|component| component["component"] == name)
            .ok_or_else(|| format!("no {name} in setup list"))?;
        assert_eq!(component["selected_version"], pinned);
        assert_eq!(component["previous_version"], earlier);
    }

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
