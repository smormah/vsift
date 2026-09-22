//! Public command-line contract tests against the compiled executable.

use assert_cmd::Command;
use serde_json::Value;
use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

fn isolated_config_base() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| std::io::Error::other("random source failed"))?;
    let mut suffix = String::with_capacity(32);
    for byte in random {
        write!(&mut suffix, "{byte:02x}")?;
    }
    Ok(std::env::temp_dir().join(format!("vsift-cli-config-test-{suffix}")))
}

fn with_config_base<'a>(command: &'a mut Command, base: &Path) -> &'a mut Command {
    command
        .env("LOCALAPPDATA", base)
        .env("XDG_CONFIG_HOME", base)
        .env("HOME", base)
}

fn config_root(base: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        base.join("Library/Application Support/vsift")
    }
    #[cfg(not(target_os = "macos"))]
    {
        base.join("vsift")
    }
}

fn run(arguments: &[&str]) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    let base = isolated_config_base()?;
    Ok(with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args(arguments)
        .output()?)
}

fn parse_stdout(output: &std::process::Output) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
fn help_and_version_publish_the_r0_namespace() -> Result<(), Box<dyn std::error::Error>> {
    let help = run(&["--help"])?;
    let help_text = String::from_utf8(help.stdout)?;

    assert!(help.status.success());
    assert!(help_text.contains("Sift technical video"));
    for namespace in [
        "setup",
        "ingest",
        "session",
        "transcript",
        "search",
        "candidates",
        "frame",
        "audio",
        "crop",
        "bundle",
        "job",
    ] {
        assert!(help_text.contains(namespace), "missing {namespace}");
    }

    let version = run(&["--version"])?;
    assert!(version.status.success());
    assert_eq!(String::from_utf8(version.stdout)?.trim(), "vsift 0.1.0");
    Ok(())
}

#[test]
fn setup_without_a_leaf_displays_help_and_does_not_probe() -> Result<(), Box<dyn std::error::Error>>
{
    let output = run(&["setup"])?;
    let stdout = String::from_utf8(output.stdout)?;

    assert!(output.status.success());
    assert!(stdout.contains("Usage: setup [COMMAND]"));
    assert!(!stdout.contains("Status:"));
    assert!(output.stderr.is_empty());
    Ok(())
}

#[test]
fn json_parse_failures_have_one_complete_machine_result() -> Result<(), Box<dyn std::error::Error>>
{
    for arguments in [
        vec!["--json", "unknown-command"],
        vec!["setup", "install", "--json"],
    ] {
        let output = run(&arguments)?;
        let value = parse_stdout(&output)?;

        assert_eq!(output.status.code(), Some(2));
        assert_eq!(value["schema_version"], "1");
        assert_eq!(value["command"], "parse");
        assert_eq!(value["status"], "failed");
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
        assert!(output.stderr.is_empty());
        assert_eq!(output.stdout.last(), Some(&b'\n'));
        assert!(!output.stdout[..output.stdout.len().saturating_sub(1)].contains(&b'\n'));
    }
    Ok(())
}

#[test]
fn setup_check_json_is_structural_and_environment_independent()
-> Result<(), Box<dyn std::error::Error>> {
    let base = isolated_config_base()?;
    let output = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args(["setup", "check", "--json", "--timeout-seconds", "1"])
        .env("PATH", "")
        .output()?;
    let value = parse_stdout(&output)?;

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(value["schema_version"], "1");
    assert_eq!(value["command"], "setup.check");
    assert_eq!(value["status"], "blocked");
    assert_eq!(value["verification_scope"], "executable_probe_only");
    assert_eq!(value["local_asr_model"], "not_checked");
    let dependencies = value["dependencies"].as_array();
    assert_eq!(dependencies.map(Vec::len), Some(3));
    if let Some(dependencies) = dependencies {
        assert!(dependencies.iter().all(|item| item["status"] == "missing"));
        assert!(
            dependencies
                .iter()
                .all(|item| item["lookup"] == "filtered_path")
        );
        assert!(
            dependencies
                .iter()
                .all(|item| item["remediation"]["required_authority"] == "user")
        );
    }
    assert!(output.stderr.is_empty());
    Ok(())
}

#[test]
fn off_path_whisper_is_selectable_without_installing_or_disclosing_its_path()
-> Result<(), Box<dyn std::error::Error>> {
    let binary = Command::cargo_bin("vsift")?.get_program().to_os_string();
    let base = isolated_config_base()?;
    let output = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args([
            "setup",
            "check",
            "--json",
            "--timeout-seconds",
            "5",
            "--whisper",
        ])
        .arg(&binary)
        .env("PATH", "")
        .output()?;
    let value = parse_stdout(&output)?;

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(value["dependencies"][2]["status"], "available");
    assert_eq!(value["dependencies"][2]["lookup"], "explicit_path");
    assert_eq!(
        value["dependencies"][2]["validation"],
        "executable_probe_only"
    );
    assert_eq!(value["dependencies"][2]["remediation"], Value::Null);
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains(&binary.to_string_lossy().to_string())
    );
    Ok(())
}

#[test]
fn invalid_explicit_selection_does_not_fall_back_to_path() -> Result<(), Box<dyn std::error::Error>>
{
    let output = run(&["setup", "check", "--json", "--whisper", "relative-whisper"])?;
    let value = parse_stdout(&output)?;

    assert_eq!(value["dependencies"][2]["lookup"], "explicit_path");
    assert_eq!(value["dependencies"][2]["status"], "unhealthy");
    assert_eq!(
        value["dependencies"][2]["remediation"]["reason"],
        "unhealthy"
    );
    assert_eq!(
        value["dependencies"][2]["remediation"]["managed_install"],
        "unavailable_unqualified"
    );
    Ok(())
}

#[test]
fn headless_setup_check_jsonl_returns_one_bounded_terminal_remediation()
-> Result<(), Box<dyn std::error::Error>> {
    let base = isolated_config_base()?;
    let output = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args([
            "setup",
            "check",
            "--events",
            "jsonl",
            "--timeout-seconds",
            "1",
        ])
        .env("PATH", "")
        .output()?;
    let lines: Vec<&[u8]> = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(lines.len(), 1);
    let event: Value = serde_json::from_slice(lines[0])?;
    assert_eq!(event["event"], "terminal");
    assert_eq!(
        event["result"]["data"]["verification_scope"],
        "executable_probe_only"
    );
    assert_eq!(
        event["result"]["data"]["dependencies"][0]["remediation"]["required_authority"],
        "user"
    );
    assert!(output.stderr.is_empty());
    Ok(())
}

#[test]
fn configured_off_path_selection_persists_and_per_call_path_overrides_it()
-> Result<(), Box<dyn std::error::Error>> {
    let base = isolated_config_base()?;
    let binary = Command::cargo_bin("vsift")?.get_program().to_os_string();
    let configured = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args(["setup", "configure", "whisper", "--executable"])
        .arg(&binary)
        .arg("--json")
        .output()?;
    let configured_value = parse_stdout(&configured)?;
    assert!(configured.status.success());
    assert_eq!(configured_value["data"]["source"], "configured_user_path");
    assert_eq!(
        configured_value["data"]["validation"],
        "canonical_file_only"
    );
    assert!(
        !String::from_utf8_lossy(&configured.stdout)
            .contains(&binary.to_string_lossy().to_string())
    );

    let checked = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args(["setup", "check", "--json"])
        .env("PATH", "")
        .output()?;
    let checked_value = parse_stdout(&checked)?;
    assert_eq!(
        checked_value["dependencies"][2]["lookup"],
        "configured_user_path"
    );
    assert_eq!(checked_value["dependencies"][2]["status"], "available");

    let overridden = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args(["setup", "check", "--json", "--whisper", "relative-whisper"])
        .env("PATH", "")
        .output()?;
    let overridden_value = parse_stdout(&overridden)?;
    assert_eq!(
        overridden_value["dependencies"][2]["lookup"],
        "explicit_path"
    );
    assert_eq!(overridden_value["dependencies"][2]["status"], "unhealthy");
    std::fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn model_registration_is_private_and_does_not_claim_compatibility()
-> Result<(), Box<dyn std::error::Error>> {
    let base = isolated_config_base()?;
    std::fs::create_dir_all(&base)?;
    let model = base.join("selected-model.bin");
    std::fs::write(&model, b"fixture only; no model parser runs")?;
    let registered = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args(["setup", "configure-model", "--file"])
        .arg(&model)
        .arg("--json")
        .output()?;
    let response = parse_stdout(&registered)?;
    assert!(registered.status.success());
    assert_eq!(response["command"], "setup.configure-model");
    assert_eq!(
        response["data"]["validation"],
        "canonical_nonempty_file_only"
    );
    assert!(
        !String::from_utf8_lossy(&registered.stdout).contains(&model.to_string_lossy().to_string())
    );

    let stored: Value = serde_json::from_slice(&std::fs::read(
        config_root(&base).join("dependencies-v1.json"),
    )?)?;
    assert_eq!(
        stored["model"],
        std::fs::canonicalize(&model)?.to_string_lossy().as_ref()
    );
    let checked = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args(["setup", "check", "--json"])
        .env("PATH", "")
        .output()?;
    assert_eq!(parse_stdout(&checked)?["local_asr_model"], "not_checked");
    std::fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn invalid_configure_path_has_no_persistent_side_effect() -> Result<(), Box<dyn std::error::Error>>
{
    let base = isolated_config_base()?;
    let output = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args([
            "setup",
            "configure",
            "ffmpeg",
            "--executable",
            "relative",
            "--json",
        ])
        .output()?;
    let value = parse_stdout(&output)?;
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert!(!base.exists());
    Ok(())
}

#[test]
fn corrupt_user_config_fails_closed_without_an_ambient_probe()
-> Result<(), Box<dyn std::error::Error>> {
    let base = isolated_config_base()?;
    let binary = Command::cargo_bin("vsift")?.get_program().to_os_string();
    let configured = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args(["setup", "configure", "whisper", "--executable"])
        .arg(&binary)
        .arg("--json")
        .output()?;
    assert!(configured.status.success());
    std::fs::write(
        config_root(&base).join("dependencies-v1.json"),
        br#"{"schema_version":1,"unknown":true}"#,
    )?;
    let checked = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args(["setup", "check", "--json"])
        .env("PATH", "")
        .output()?;
    let value = parse_stdout(&checked)?;
    assert_eq!(value["error"]["code"], "INTEGRITY_FAILURE");
    assert!(value["data"].is_null());
    std::fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn reviewed_setup_plan_checks_first_and_never_installs() -> Result<(), Box<dyn std::error::Error>> {
    let base = isolated_config_base()?;
    let output = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args(["setup", "plan", "--profile", "desktop", "--json"])
        .env("PATH", "")
        .output()?;
    let value = parse_stdout(&output)?;
    assert!(output.status.success());
    assert_eq!(value["command"], "setup.plan");
    assert_eq!(value["status"], "complete");
    assert_eq!(value["data"]["readiness"], "blocked");
    assert_planning_target(&value["data"], 3);
    assert_eq!(
        value["data"]["dependencies"][0]["required_authority"],
        "user"
    );
    if base.exists() {
        std::fs::remove_dir_all(&base)?;
    }
    Ok(())
}

#[test]
fn reserved_install_remains_unavailable_without_mutation() -> Result<(), Box<dyn std::error::Error>>
{
    let output = run(&[
        "setup",
        "install",
        "--plan",
        "missing.json",
        "--accept-plan",
        "unknown",
        "--json",
    ])?;
    let value = parse_stdout(&output)?;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(value["command"], "setup.install");
    assert_eq!(value["error"]["code"], "COMMAND_NOT_IMPLEMENTED");
    assert_eq!(value["data"], Value::Null);
    Ok(())
}

#[test]
fn setup_plan_uses_configured_off_path_tool_but_does_not_call_it_qualified()
-> Result<(), Box<dyn std::error::Error>> {
    let base = isolated_config_base()?;
    let binary = Command::cargo_bin("vsift")?.get_program().to_os_string();
    let configured = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args(["setup", "configure", "whisper", "--executable"])
        .arg(&binary)
        .arg("--json")
        .output()?;
    assert!(configured.status.success());
    let output = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args(["setup", "plan", "--profile", "worker", "--json"])
        .env("PATH", "")
        .output()?;
    let value = parse_stdout(&output)?;
    assert!(output.status.success());
    assert_eq!(value["data"]["profile"], "worker");
    assert_eq!(value["data"]["dependencies"][2]["status"], "available");
    assert_eq!(
        value["data"]["dependencies"][2]["disposition"],
        "existing_executable_probe_only"
    );
    assert_planning_target(&value["data"], 2);
    std::fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn headless_plan_jsonl_has_one_terminal_and_no_install_authority()
-> Result<(), Box<dyn std::error::Error>> {
    let base = isolated_config_base()?;
    let output = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args(["setup", "plan", "--profile", "worker", "--events", "jsonl"])
        .env("PATH", "")
        .output()?;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout)?;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1);
    let event: Value = serde_json::from_str(lines[0])?;
    assert_eq!(event["event"], "terminal");
    assert_eq!(event["command"], "setup.plan");
    assert_planning_target(&event["result"]["data"], 3);
    if base.exists() {
        std::fs::remove_dir_all(&base)?;
    }
    Ok(())
}

fn assert_planning_target(data: &Value, expected_ubuntu_actions: usize) {
    if data["target"] == "ubuntu_24_04_x86_64" {
        assert_eq!(
            data["managed_install"],
            "catalogue_accepted_install_pending"
        );
        assert_eq!(
            data["actions"].as_array().map(Vec::len),
            Some(expected_ubuntu_actions)
        );
        assert_eq!(data["plan_digest"].as_str().map(str::len), Some(64));
        assert_eq!(data["dependencies"][0]["disposition"], "managed_install");
    } else {
        assert_eq!(data["managed_install"], "unavailable_target");
        assert_eq!(data["plan_digest"], Value::Null);
        assert_eq!(data["actions"], serde_json::json!([]));
        assert_eq!(
            data["dependencies"][0]["disposition"],
            "manual_selection_required"
        );
    }
}

#[test]
fn unqualified_plan_rejects_corrupt_byo_config_before_probing()
-> Result<(), Box<dyn std::error::Error>> {
    let base = isolated_config_base()?;
    let binary = Command::cargo_bin("vsift")?.get_program().to_os_string();
    let configured = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args(["setup", "configure", "whisper", "--executable"])
        .arg(&binary)
        .arg("--json")
        .output()?;
    assert!(configured.status.success());
    std::fs::write(
        config_root(&base).join("dependencies-v1.json"),
        br#"{"schema_version":1,"unknown":true}"#,
    )?;
    let output = with_config_base(&mut Command::cargo_bin("vsift")?, &base)
        .args(["setup", "plan", "--profile", "desktop", "--json"])
        .env("PATH", "")
        .output()?;
    let value = parse_stdout(&output)?;
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(value["error"]["code"], "INTEGRITY_FAILURE");
    assert_eq!(value["data"], Value::Null);
    std::fs::remove_dir_all(&base)?;
    Ok(())
}
