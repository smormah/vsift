//! Real process-to-process P05 CLI and v1 JSON contract regression.

use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!("vsift-p05-cli-{}-{stamp}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("vsift-p05-cli-"))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn run(root: &Path, arguments: &[&str]) -> Result<Value, Box<dyn Error>> {
    let output = Command::cargo_bin("vsift")?
        .arg("--session-root")
        .arg(root)
        .args(arguments)
        .arg("--json")
        .output()?;
    let value: Value = serde_json::from_slice(&output.stdout)?;
    if !output.status.success() {
        return Err(format!("command {arguments:?} failed: {value}").into());
    }
    let schema: Value = serde_json::from_slice(&fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schemas/v1/operation-response.schema.json"),
    )?)?;
    jsonschema::validator_for(&schema)?
        .validate(&value)
        .map_err(|error| error.to_string())?;
    Ok(value)
}

#[test]
fn disposable_open_status_export_close_and_cleanup_survive_process_restarts() -> TestResult {
    let temp = OwnedRoot::new()?;
    let root = temp.0.join("private sessions");
    let source = temp.0.join("original media.mp4");
    let original = b"\0\0\0\x18ftypisomsource-content";
    fs::write(&source, original)?;

    let opened = Command::cargo_bin("vsift")?
        .arg("--session-root")
        .arg(&root)
        .arg("ingest")
        .arg(&source)
        .arg("--json")
        .output()?;
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stdout)
    );
    let opened: Value = serde_json::from_slice(&opened.stdout)?;
    assert_eq!(opened["command"], "ingest");
    assert_eq!(opened["data"]["publication"], "process_crash_consistent");
    assert_eq!(opened["lifecycle"]["mode"], "ephemeral");
    let id = opened["data"]["session_id"]
        .as_str()
        .ok_or("missing session id")?;

    let status = run(&root, &["session", "status", id])?;
    assert_eq!(status["data"]["state"], "open");
    assert_eq!(status["data"]["source_bytes"], original.len());
    let list = run(&root, &["session", "list"])?;
    assert_eq!(list["data"]["items"][0]["session_id"], id);

    let evidence = temp.0.join("evidence");
    let evidence_result = Command::cargo_bin("vsift")?
        .arg("--session-root")
        .arg(&root)
        .args(["session", "retain", id, "--output"])
        .arg(&evidence)
        .arg("--json")
        .output()?;
    assert!(
        evidence_result.status.success(),
        "{}",
        String::from_utf8_lossy(&evidence_result.stdout)
    );
    let evidence_result: Value = serde_json::from_slice(&evidence_result.stdout)?;
    assert_eq!(evidence_result["data"]["source_included"], false);
    assert_eq!(
        evidence_result["data"]["reextraction_requires_matching_original"],
        true
    );
    assert!(!evidence.join("source.media").exists());
    let validated = Command::cargo_bin("vsift")?
        .args(["bundle", "validate"])
        .arg(&evidence)
        .arg("--json")
        .output()?;
    assert!(validated.status.success());

    let portable = temp.0.join("portable");
    let retained = Command::cargo_bin("vsift")?
        .arg("--session-root")
        .arg(&root)
        .args(["session", "retain", id, "--output"])
        .arg(&portable)
        .args(["--include-source", "--json"])
        .output()?;
    assert!(
        retained.status.success(),
        "{}",
        String::from_utf8_lossy(&retained.stdout)
    );
    assert_eq!(fs::read(portable.join("source.media"))?, original);
    run(&root, &["session", "close", id])?;
    let cleaned = run(&root, &["session", "clean", "--expired"])?;
    assert_eq!(cleaned["data"]["items"][0]["outcome"], "removed");
    assert_eq!(fs::read(&source)?, original);
    assert_eq!(fs::read(portable.join("source.media"))?, original);
    assert!(evidence.join("bundle.json").exists());
    Ok(())
}

#[test]
fn reserved_transcription_and_invalid_bundle_fail_with_typed_v1_results_without_mutation()
-> TestResult {
    let temp = OwnedRoot::new()?;
    let root = temp.0.join("private sessions");
    let source = temp.0.join("original.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomsource-content")?;

    let reserved = Command::cargo_bin("vsift")?
        .arg("--session-root")
        .arg(&root)
        .arg("ingest")
        .arg(&source)
        .args(["--transcript", "auto", "--json"])
        .output()?;
    assert_eq!(reserved.status.code(), Some(2));
    let reserved: Value = serde_json::from_slice(&reserved.stdout)?;
    assert_eq!(reserved["command"], "ingest");
    assert_eq!(reserved["error"]["code"], "COMMAND_NOT_IMPLEMENTED");
    assert!(!root.exists());

    let invalid = Command::cargo_bin("vsift")?
        .args(["bundle", "validate"])
        .arg(temp.0.join("missing-bundle"))
        .arg("--json")
        .output()?;
    assert!(!invalid.status.success());
    let invalid: Value = serde_json::from_slice(&invalid.stdout)?;
    assert_eq!(invalid["command"], "bundle.validate");
    assert!(invalid["error"]["code"].is_string());
    assert_eq!(fs::read(&source)?, b"\0\0\0\x18ftypisomsource-content");
    Ok(())
}
