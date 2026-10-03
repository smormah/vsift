//! Conformance of the refusal of an existing folder other accounts can access,
//! and of the refusal of a session root `VSift` did not create, against the
//! published v1 envelope schema and their frozen examples.
//!
//! The first example is the result an agent receives when `setup configure`
//! meets a per-user configuration folder that inherited access for other
//! accounts; the second, when `session list` meets a `--session-root` folder
//! that holds no `VSift` ownership marker.

use std::{fs, io, path::PathBuf};

use serde_json::Value;
use vsift_contract::{
    CommandName, OperationResponse, PrivateFolder, TerminalEventResponse,
    UNOWNED_SESSION_ROOT_REMEDIATION, non_private_folder_summary,
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
