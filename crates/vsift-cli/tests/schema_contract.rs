//! JSON Schema and compatibility fixtures for the public v1 contract.

use std::{fmt::Write as _, fs, io, path::PathBuf};

use assert_cmd::Command;
use serde::Deserialize;
use serde_json::Value;

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn load(relative_path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(
        schema_root().join(relative_path),
    )?)?)
}

fn validate(schema: &Value, instance: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let validator = jsonschema::validator_for(schema)?;
    validator
        .validate(instance)
        .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(())
}

#[test]
fn every_published_example_validates_against_its_schema() -> Result<(), Box<dyn std::error::Error>>
{
    for (schema_path, example_path) in [
        (
            "setup-check-response.schema.json",
            "examples/setup-check.blocked.json",
        ),
        (
            "setup-check-response.schema.json",
            "examples/setup-check.local-asr.json",
        ),
        (
            "operation-response.schema.json",
            "examples/operation-error.json",
        ),
        (
            "setup-plan-unqualified.schema.json",
            "examples/setup-plan.unqualified.json",
        ),
        (
            "setup-plan.schema.json",
            "examples/setup-plan.unavailable.json",
        ),
        ("terminal-event.schema.json", "examples/terminal-event.json"),
        ("config.schema.json", "examples/config.json"),
    ] {
        validate(&load(schema_path)?, &load(example_path)?)?;
    }
    Ok(())
}

#[test]
fn strict_schemas_reject_unknown_fields() -> Result<(), Box<dyn std::error::Error>> {
    for (schema_path, example_path) in [
        (
            "setup-check-response.schema.json",
            "examples/setup-check.blocked.json",
        ),
        (
            "setup-check-response.schema.json",
            "examples/setup-check.local-asr.json",
        ),
        (
            "operation-response.schema.json",
            "examples/operation-error.json",
        ),
        (
            "setup-plan-unqualified.schema.json",
            "examples/setup-plan.unqualified.json",
        ),
        (
            "setup-plan.schema.json",
            "examples/setup-plan.unavailable.json",
        ),
        ("terminal-event.schema.json", "examples/terminal-event.json"),
        ("config.schema.json", "examples/config.json"),
    ] {
        let schema = load(schema_path)?;
        let mut instance = load(example_path)?;
        let object = instance
            .as_object_mut()
            .ok_or_else(|| io::Error::other("contract example is not an object"))?;
        object.insert(String::from("unknown"), Value::Bool(true));
        assert!(!jsonschema::validator_for(&schema)?.is_valid(&instance));
    }
    Ok(())
}

#[test]
fn terminal_event_embeds_a_valid_operation_response() -> Result<(), Box<dyn std::error::Error>> {
    let event = load("examples/terminal-event.json")?;
    validate(&load("operation-response.schema.json")?, &event["result"])?;
    Ok(())
}

#[test]
fn emitted_json_matches_frozen_examples() -> Result<(), Box<dyn std::error::Error>> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| io::Error::other("random source failed"))?;
    let mut suffix = String::with_capacity(32);
    for byte in random {
        write!(&mut suffix, "{byte:02x}")?;
    }
    let base = std::env::temp_dir().join(format!("vsift-schema-test-{suffix}"));
    let setup = Command::cargo_bin("vsift")?
        .args(["setup", "check", "--json", "--timeout-seconds", "1"])
        .env("PATH", "")
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .output()?;
    assert_eq!(
        serde_json::from_slice::<Value>(&setup.stdout)?,
        load("examples/setup-check.blocked.json")?
    );

    let operation = Command::cargo_bin("vsift")?
        .args([
            "setup",
            "install",
            "--plan",
            "missing.json",
            "--accept-plan",
            "unknown",
            "--json",
        ])
        .output()?;
    assert_eq!(
        serde_json::from_slice::<Value>(&operation.stdout)?,
        load("examples/operation-error.json")?
    );

    let event = Command::cargo_bin("vsift")?
        .args([
            "setup",
            "install",
            "--plan",
            "missing.json",
            "--accept-plan",
            "unknown",
            "--events",
            "jsonl",
        ])
        .output()?;
    assert_eq!(
        serde_json::from_slice::<Value>(&event.stdout)?,
        load("examples/terminal-event.json")?
    );
    let plan = Command::cargo_bin("vsift")?
        .args(["setup", "plan", "--profile", "desktop", "--json"])
        .env("PATH", "")
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .output()?;
    let plan_value = serde_json::from_slice::<Value>(&plan.stdout)?;
    validate(&load("setup-plan.schema.json")?, &plan_value)?;
    assert_eq!(
        plan_value["data"]["verification_scope"],
        "executable_probe_and_reviewed_catalogue"
    );
    assert_eq!(plan_value["data"]["readiness"], "blocked");
    if plan_value["data"]["target"] == "ubuntu_24_04_x86_64" {
        assert_eq!(
            plan_value["data"]["managed_install"],
            "catalogue_accepted_install_pending"
        );
        assert_eq!(
            plan_value["data"]["actions"].as_array().map(Vec::len),
            Some(3)
        );
        assert_eq!(
            plan_value["data"]["plan_digest"].as_str().map(str::len),
            Some(64)
        );
    } else {
        assert_eq!(plan_value["data"]["managed_install"], "unavailable_target");
        assert!(
            plan_value["data"]["actions"]
                .as_array()
                .is_some_and(Vec::is_empty)
        );
        assert!(plan_value["data"]["plan_digest"].is_null());
    }
    Ok(())
}

/// Regression for issue #125: the binary emitted `setup.configure-model`
/// envelopes that the published `command` pattern rejected.
#[test]
fn configure_model_json_and_jsonl_output_validate_against_the_schemas()
-> Result<(), Box<dyn std::error::Error>> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| io::Error::other("random source failed"))?;
    let mut suffix = String::with_capacity(32);
    for byte in random {
        write!(&mut suffix, "{byte:02x}")?;
    }
    let base = std::env::temp_dir().join(format!("vsift-schema-model-test-{suffix}"));
    fs::create_dir_all(&base)?;
    let model = base.join("selected-model.bin");
    fs::write(&model, b"fixture only; no model parser runs")?;
    let register = |output_mode: &[&str]| -> Result<Value, Box<dyn std::error::Error>> {
        let output = Command::cargo_bin("vsift")?
            .args(["setup", "configure-model", "--file"])
            .arg(&model)
            .args(output_mode)
            .env("LOCALAPPDATA", &base)
            .env("XDG_CONFIG_HOME", &base)
            .env("HOME", &base)
            .output()?;
        assert!(output.status.success());
        Ok(serde_json::from_slice::<Value>(&output.stdout)?)
    };
    let operation_schema = load("operation-response.schema.json")?;

    let json = register(&["--json"])?;
    validate(&operation_schema, &json)?;
    assert_eq!(json["command"], "setup.configure-model");

    let event = register(&["--events", "jsonl"])?;
    validate(&load("terminal-event.schema.json")?, &event)?;
    validate(&operation_schema, &event["result"])?;
    assert_eq!(event["command"], "setup.configure-model");

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[derive(Deserialize)]
struct SetupV1OldReader {
    schema_version: String,
    command: String,
    status: String,
    dependencies: Vec<Value>,
}

#[test]
fn old_setup_reader_accepts_additive_v1_fields() -> Result<(), Box<dyn std::error::Error>> {
    let mut future = load("examples/setup-check.blocked.json")?;
    let object = future
        .as_object_mut()
        .ok_or_else(|| io::Error::other("setup example is not an object"))?;
    object.insert(String::from("future_additive_field"), Value::Bool(true));

    let old: SetupV1OldReader = serde_json::from_value(future)?;

    assert_eq!(old.schema_version, "1");
    assert_eq!(old.command, "setup.check");
    assert_eq!(old.status, "blocked");
    assert_eq!(old.dependencies.len(), 3);
    Ok(())
}

#[test]
fn additive_setup_fields_do_not_invalidate_the_original_v1_payload()
-> Result<(), Box<dyn std::error::Error>> {
    let mut old = load("examples/setup-check.blocked.json")?;
    let object = old
        .as_object_mut()
        .ok_or_else(|| io::Error::other("setup example is not an object"))?;
    object.remove("verification_scope");
    object.remove("local_asr_model");
    object.remove("local_asr");
    let dependencies = object
        .get_mut("dependencies")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| io::Error::other("dependencies are missing"))?;
    for dependency in dependencies {
        let item = dependency
            .as_object_mut()
            .ok_or_else(|| io::Error::other("dependency is not an object"))?;
        item.remove("lookup");
        item.remove("validation");
        item.remove("remediation");
    }

    validate(&load("setup-check-response.schema.json")?, &old)?;
    Ok(())
}
