//! Public command-line contract tests against the compiled executable.

use assert_cmd::Command;
use serde_json::Value;

fn run(arguments: &[&str]) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    Ok(Command::cargo_bin("vsift")?.args(arguments).output()?)
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
    let output = Command::cargo_bin("vsift")?
        .args(["setup", "check", "--json", "--timeout-seconds", "1"])
        .env("PATH", "")
        .output()?;
    let value = parse_stdout(&output)?;

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(value["schema_version"], "1");
    assert_eq!(value["command"], "setup.check");
    assert_eq!(value["status"], "blocked");
    let dependencies = value["dependencies"].as_array();
    assert_eq!(dependencies.map(Vec::len), Some(3));
    if let Some(dependencies) = dependencies {
        assert!(dependencies.iter().all(|item| item["status"] == "missing"));
    }
    assert!(output.stderr.is_empty());
    Ok(())
}

#[test]
fn reserved_operations_fail_without_claiming_implementation()
-> Result<(), Box<dyn std::error::Error>> {
    let output = run(&["session", "list", "--json"])?;
    let value = parse_stdout(&output)?;

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(value["command"], "session.list");
    assert_eq!(value["error"]["code"], "COMMAND_NOT_IMPLEMENTED");
    assert_eq!(value["data"], Value::Null);
    Ok(())
}
