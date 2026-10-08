//! Conformance of the refusal of an existing folder other accounts can access,
//! of the refusal of a session root `VSift` did not create, and of a source
//! copy that was too slow, against the published v1 envelope schema and their
//! examples.
//!
//! The first example is the result an agent receives when `setup configure`
//! meets a per-user configuration folder that inherited access for other
//! accounts; the second, when `session list` meets a `--session-root` folder
//! that holds no `VSift` ownership marker; the third, when `ingest` could not
//! copy the video within the ten-minute limit.

use std::{fs, io, path::PathBuf};

use serde_json::Value;
use vsift_contract::{
    CommandName, OperationResponse, PrivateFolder, SOURCE_COPY_TOO_SLOW_REMEDIATION,
    TerminalEventResponse, UNOWNED_SESSION_ROOT_REMEDIATION, non_private_folder_summary,
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

fn refused(command: CommandName, folder: PrivateFolder) -> OperationResponse<Value> {
    OperationResponse::failure_with_remediation(
        command.identifier(),
        FailureCode::StorageIo,
        non_private_folder_summary(folder),
    )
}

#[test]
fn a_refused_configuration_folder_matches_the_frozen_example() -> TestResult {
    let response = serde_json::to_value(refused(
        CommandName::SetupConfigure,
        PrivateFolder::UserConfiguration,
    ))?;
    validate("operation-response.schema.json", &response)?;
    assert_eq!(response, load("examples/storage-not-private.json")?);
    Ok(())
}

/// The answer to a session root `VSift` did not create (#261): the failure code
/// is the 0.1.0 one, `INTEGRITY_FAILURE`, and the remediation explains it.
#[test]
fn a_session_root_vsift_did_not_create_matches_the_frozen_example() -> TestResult {
    let response = OperationResponse::<Value>::failure_with_remediation(
        CommandName::SessionList.identifier(),
        FailureCode::IntegrityFailure,
        UNOWNED_SESSION_ROOT_REMEDIATION.to_owned(),
    );
    let value = serde_json::to_value(&response)?;
    validate("operation-response.schema.json", &value)?;
    assert_eq!(value, load("examples/session-root-unowned.json")?);
    let event = serde_json::to_value(TerminalEventResponse::new(response))?;
    validate("terminal-event.schema.json", &event)?;
    Ok(())
}

/// The answer to a source copy that outran the ten-minute limit (#325): the
/// failure code is the one this case always had, `INVALID_SOURCE`, with its
/// fixed message, and the remediation says the copy was slow and the video
/// was not judged.
#[test]
fn a_source_copy_that_was_too_slow_matches_the_example() -> TestResult {
    let response = OperationResponse::<Value>::failure_with_remediation(
        CommandName::Ingest.identifier(),
        FailureCode::InvalidSource,
        SOURCE_COPY_TOO_SLOW_REMEDIATION.to_owned(),
    );
    let value = serde_json::to_value(&response)?;
    validate("operation-response.schema.json", &value)?;
    assert_eq!(value, load("examples/ingest-copy-too-slow.json")?);
    assert_eq!(value["error"]["code"], "INVALID_SOURCE");
    assert_eq!(value["error"]["retryable"], false);
    let event = serde_json::to_value(TerminalEventResponse::new(response))?;
    validate("terminal-event.schema.json", &event)?;
    Ok(())
}

#[test]
fn every_folder_kind_produces_schema_valid_json_and_jsonl_failures() -> TestResult {
    for folder in PrivateFolder::ALL {
        let response = refused(CommandName::SessionList, folder);
        let value = serde_json::to_value(&response)?;
        validate("operation-response.schema.json", &value)?;
        let summary = value["error"]["remediation"][0]["summary"]
            .as_str()
            .ok_or("summary missing")?;
        assert!(summary.starts_with(&format!(
            "The VSift {} folder is accessible to other accounts,",
            folder.identifier()
        )));
        let event = serde_json::to_value(TerminalEventResponse::new(response))?;
        validate("terminal-event.schema.json", &event)?;
    }
    Ok(())
}
