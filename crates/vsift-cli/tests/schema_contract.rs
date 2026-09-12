//! JSON Schema and compatibility fixtures for the public v1 contract.

use std::{fs, io, path::PathBuf};

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
            "operation-response.schema.json",
            "examples/operation-error.json",
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
            "operation-response.schema.json",
            "examples/operation-error.json",
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
    let setup = Command::cargo_bin("vsift")?
        .args(["setup", "check", "--json", "--timeout-seconds", "1"])
        .env("PATH", "")
        .output()?;
    assert_eq!(
        serde_json::from_slice::<Value>(&setup.stdout)?,
        load("examples/setup-check.blocked.json")?
    );

    let operation = Command::cargo_bin("vsift")?
        .args(["setup", "plan", "--profile", "desktop", "--json"])
        .output()?;
    assert_eq!(
        serde_json::from_slice::<Value>(&operation.stdout)?,
        load("examples/operation-error.json")?
    );

    let event = Command::cargo_bin("vsift")?
        .args(["setup", "plan", "--profile", "desktop", "--events", "jsonl"])
        .output()?;
    assert_eq!(
        serde_json::from_slice::<Value>(&event.stdout)?,
        load("examples/terminal-event.json")?
    );
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
