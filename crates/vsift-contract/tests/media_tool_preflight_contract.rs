//! Conformance of the failed media-tool preflight result against the published
//! v1 envelope schema and its frozen example.
//!
//! The example is the result an agent receives when `FFmpeg` is registered as
//! `FFprobe` and `ingest --transcript` runs: the preflight fails at the probe
//! check before anything is written.

use std::{fs, io, path::PathBuf};

use serde_json::Value;
use vsift_application::{MediaToolCheck, MediaToolFailure, MediaToolPreflightFailure};
use vsift_contract::{
    CommandName, OperationResponse, TerminalEventResponse, media_tool_verification_summary,
};
use vsift_domain::FailureCode;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn load(relative_path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(
        schema_root().join(relative_path),
    )?)?)
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    jsonschema::validator_for(&load(schema_path)?)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

fn failed_preflight(check: MediaToolCheck, failure: MediaToolFailure) -> OperationResponse<Value> {
    OperationResponse::failure_with_remediation(
        CommandName::Ingest.identifier(),
        FailureCode::MissingCapability,
        media_tool_verification_summary(MediaToolPreflightFailure { check, failure }),
    )
}

#[test]
fn failed_preflight_matches_the_frozen_example() -> TestResult {
    let response = serde_json::to_value(failed_preflight(
        MediaToolCheck::Probe,
        MediaToolFailure::ProviderRejected,
    ))?;

    validate("operation-response.schema.json", &response)?;
    assert_eq!(
        response,
        load("examples/media-tool-verification-failed.json")?
    );
    Ok(())
}

#[test]
fn every_check_and_reason_produces_schema_valid_json_and_jsonl_failures() -> TestResult {
    for check in [
        MediaToolCheck::Preparation,
        MediaToolCheck::Probe,
        MediaToolCheck::Frame,
        MediaToolCheck::Audio,
    ] {
        for failure in [
            MediaToolFailure::FixtureIntegrity,
            MediaToolFailure::Workspace,
            MediaToolFailure::ProcessFailure,
            MediaToolFailure::ProviderRejected,
            MediaToolFailure::Deadline,
            MediaToolFailure::OutputLimit,
            MediaToolFailure::UnexpectedResult,
            MediaToolFailure::Cancelled,
        ] {
            let response = failed_preflight(check, failure);
            let value = serde_json::to_value(&response)?;
            validate("operation-response.schema.json", &value)?;
            let summary = value["error"]["remediation"][0]["summary"]
                .as_str()
                .ok_or("summary missing")?;
            assert!(
                summary.contains(&format!(
                    "at the {} step ({}).",
                    check.identifier(),
                    failure.identifier()
                )),
                "{summary}"
            );
            let event = serde_json::to_value(TerminalEventResponse::new(response))?;
            validate("terminal-event.schema.json", &event)?;
        }
    }
    Ok(())
}
