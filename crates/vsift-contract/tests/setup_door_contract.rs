//! The `setup` result of the unattended setup door (ADR 0036 section 6):
//! its published schema, its frozen examples and the closed values of its
//! fields.
//!
//! `VSIFT_UPDATE_SCHEMA_EXAMPLES=1` rewrites the examples from the types.

use std::{env, fs, io, path::PathBuf};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_contract::{
    SETUP_FAILED_REMEDIATION, SETUP_NEXT_AGAIN, SETUP_NEXT_CONNECTION, SetupOutcome, SetupPartName,
    SetupPartResult, SetupPartStatus, SetupResponse, setup_door_response, setup_failure_code,
};
use vsift_domain::FailureCode;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn load(relative_path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(
        schema_root().join(relative_path),
    )?)?)
}

/// Resolves sibling `$ref`s by their published identifier to the local copy.
struct PublishedSchemas;

impl Retrieve for PublishedSchemas {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let name = uri
            .as_str()
            .strip_prefix(SCHEMA_BASE)
            .filter(|name| !name.contains(['/', '\\']))
            .ok_or_else(|| format!("unpublished schema reference: {uri}"))?;
        Ok(serde_json::from_str(&fs::read_to_string(
            schema_root().join(name),
        )?)?)
    }
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&load(schema_path)?)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

fn frozen(example: &str, value: &Value) -> TestResult {
    let path = schema_root().join("examples").join(example);
    if env::var_os("VSIFT_UPDATE_SCHEMA_EXAMPLES").is_some() {
        fs::write(&path, format!("{}\n", serde_json::to_string_pretty(value)?))?;
        return Ok(());
    }
    assert_eq!(*value, load(&format!("examples/{example}"))?, "{example}");
    Ok(())
}

fn envelope(
    response: &SetupResponse,
    install_failure: Option<FailureCode>,
) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::to_value(setup_door_response(
        response,
        install_failure,
    )?)?)
}

fn part(
    part: SetupPartName,
    requested: bool,
    status: SetupPartStatus,
    next_step: Option<&str>,
) -> SetupPartResult {
    SetupPartResult {
        part,
        requested,
        status,
        next_step: next_step.map(str::to_owned),
    }
}

/// A pass: speech installed, video already working and not named.
#[test]
fn a_pass_carries_the_install_and_validates() -> TestResult {
    let response = SetupResponse {
        outcome: SetupOutcome::Pass,
        parts: vec![
            part(SetupPartName::Video, false, SetupPartStatus::Ready, None),
            part(SetupPartName::Speech, true, SetupPartStatus::Ready, None),
        ],
        install: Some(load("examples/setup-install.json")?["data"].take()),
    };
    let value = envelope(&response, None)?;
    validate("operation-response.schema.json", &value)?;
    validate("setup.schema.json", &value["data"])?;
    assert_eq!(value["command"], "setup");
    frozen("setup.pass.json", &value)?;
    let read: SetupResponse = serde_json::from_value(value["data"].clone())?;
    assert_eq!(read, response);
    Ok(())
}

/// A fail: video already worked, speech's download stopped, and each part
/// that does not work names its next step. The envelope is `failed` with
/// the install's own code, whose class is the exit status, and `data` is
/// the result beside the error (Fable review P-2).
#[test]
fn a_fail_names_each_parts_next_step_and_validates() -> TestResult {
    let response = SetupResponse {
        outcome: SetupOutcome::Fail,
        parts: vec![
            part(SetupPartName::Video, true, SetupPartStatus::Ready, None),
            part(
                SetupPartName::Speech,
                true,
                SetupPartStatus::NotSetUp,
                Some(SETUP_NEXT_CONNECTION),
            ),
            part(
                SetupPartName::WebVideos,
                true,
                SetupPartStatus::NotAdded,
                Some(SETUP_NEXT_AGAIN),
            ),
        ],
        install: Some(load("examples/setup-install.failed.json")?["data"].take()),
    };
    let value = envelope(&response, Some(FailureCode::DownloadFailed))?;
    validate("operation-response.schema.json", &value)?;
    validate("setup.schema.json", &value["data"])?;
    assert_eq!(value["status"], "failed");
    assert_eq!(value["error"]["code"], "DOWNLOAD_FAILED");
    assert_eq!(
        value["error"]["remediation"][0]["summary"],
        SETUP_FAILED_REMEDIATION
    );
    frozen("setup.fail.json", &value)?;
    Ok(())
}

/// The failure code follows the outcome: none for a pass (whatever the
/// install reported), the install's own code for a fail, and
/// `MISSING_CAPABILITY` for a fail the install gave no code for.
#[test]
fn the_failure_code_follows_the_outcome() -> TestResult {
    assert_eq!(
        setup_failure_code(SetupOutcome::Pass, Some(FailureCode::DownloadFailed)),
        None
    );
    assert_eq!(
        setup_failure_code(SetupOutcome::Fail, Some(FailureCode::StorageIo)),
        Some(FailureCode::StorageIo)
    );
    assert_eq!(
        setup_failure_code(SetupOutcome::Fail, None),
        Some(FailureCode::MissingCapability)
    );
    let response = SetupResponse {
        outcome: SetupOutcome::Fail,
        parts: vec![part(
            SetupPartName::Video,
            true,
            SetupPartStatus::NotSetUp,
            Some(SETUP_NEXT_AGAIN),
        )],
        install: None,
    };
    let value = envelope(&response, None)?;
    validate("operation-response.schema.json", &value)?;
    assert_eq!(value["error"]["code"], "MISSING_CAPABILITY");
    assert_eq!(value["data"]["outcome"], "fail");
    Ok(())
}

/// Nothing to install is a pass with `install` null; a closed value the
/// schema does not know is refused.
#[test]
fn nothing_to_install_is_null_and_unknown_values_are_refused() -> TestResult {
    let response = SetupResponse {
        outcome: SetupOutcome::Pass,
        parts: vec![
            part(SetupPartName::Video, true, SetupPartStatus::Ready, None),
            part(SetupPartName::Speech, true, SetupPartStatus::Ready, None),
        ],
        install: None,
    };
    let mut value = envelope(&response, None)?;
    validate("setup.schema.json", &value["data"])?;
    assert_eq!(value["data"]["install"], Value::Null);
    value["data"]["outcome"] = Value::from("partial");
    assert!(validate("setup.schema.json", &value["data"]).is_err());
    value["data"]["outcome"] = Value::from("pass");
    value["data"]["parts"][0]["status"] = Value::from("installed");
    assert!(validate("setup.schema.json", &value["data"]).is_err());
    Ok(())
}
