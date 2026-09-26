//! Strict decoding of evidence records, starting from the frozen bundle
//! example.

use serde_json::Value;
use vsift_application::SessionStorageError;
use vsift_domain::SessionId;

use super::{MAX_EVIDENCE_RECORD_BYTES, decode_evidence_record, encode_evidence_record};

type TestResult = Result<(), Box<dyn std::error::Error>>;
/// A named edit of a stored record.
type Edit = (&'static str, fn(&mut Value));

const EXAMPLE: &str = include_str!("../../../../schemas/v1/examples/bundle-evidence-record.json");

fn session() -> Result<SessionId, Box<dyn std::error::Error>> {
    Ok(SessionId::parse("ses_0123456789abcdef0123456789abcdef")?)
}

fn decoded(value: &Value) -> Result<vsift_domain::EvidenceRecord, SessionStorageError> {
    let bytes = serde_json::to_vec(value).map_err(|_| SessionStorageError::Io)?;
    decode_evidence_record(&bytes, &session().map_err(|_| SessionStorageError::Io)?)
}

#[test]
fn the_example_round_trips() -> TestResult {
    let example: Value = serde_json::from_str(EXAMPLE)?;
    let record = decoded(&example)?;
    let encoded: Value = serde_json::from_slice(&encode_evidence_record(&record)?)?;
    assert_eq!(encoded, example);
    Ok(())
}

#[test]
fn anything_the_record_does_not_define_or_derive_is_rejected() -> TestResult {
    let example: Value = serde_json::from_str(EXAMPLE)?;
    let edits: [Edit; 8] = [
        ("unknown field", |value| value["extra"] = Value::Bool(true)),
        ("unknown item field", |value| {
            value["items"][0]["path"] = "frame.png".into();
        }),
        ("another format", |value| {
            value["format"] = "vsift.visual_index_record".into();
        }),
        ("another session", |value| {
            value["session_id"] = "ses_fedcba9876543210".into();
        }),
        ("an edited timestamp", |value| {
            value["items"][0]["subject"]["frame"]["pts"] = 22.into();
        }),
        ("an inconsistent delta", |value| {
            value["selections"][0]["delta_us"] = 0.into();
        }),
        ("a non-canonical fingerprint", |value| {
            value["tool_fingerprint"] = "ABC".into();
        }),
        ("an edited request", |value| {
            value["request"]["frame_get"]["tolerance_us"] = 0.into();
        }),
    ];
    for (name, edit) in edits {
        let mut value = example.clone();
        edit(&mut value);
        assert_eq!(
            decoded(&value).map(|_| ()),
            Err(SessionStorageError::IntegrityFailure),
            "{name}"
        );
    }
    Ok(())
}

#[test]
fn a_newer_version_is_unsupported_and_an_oversized_record_is_rejected() -> TestResult {
    let mut value: Value = serde_json::from_str(EXAMPLE)?;
    value["schema_version"] = 2.into();
    assert_eq!(
        decoded(&value).map(|_| ()),
        Err(SessionStorageError::UnsupportedVersion)
    );
    let oversized = vec![b' '; MAX_EVIDENCE_RECORD_BYTES + 1];
    assert_eq!(
        decode_evidence_record(&oversized, &session()?).map(|_| ()),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}
