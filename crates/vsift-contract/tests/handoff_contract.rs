//! The `handoff.check` data (P13 PR 5) and the `handoff.skeleton` data (ADR
//! 0038) against their published schemas and frozen examples. Set `VSIFT_REGENERATE_CONTRACT_EXAMPLES=1` to rewrite the
//! example after an intended change, and review the diff.

use std::{collections::BTreeSet, env, fs, io, path::PathBuf};

use serde_json::{Value, json};
use vsift_contract::{
    CommandName, HandoffCheckData, HandoffChecker, HandoffEvidenceKind, HandoffRule,
    HandoffSessionGap, HandoffSessionRecords, HandoffSkeletonFacts, HandoffTranscriptBasis,
    OperationResponse,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const SEGMENT: &str = "tsg_0123456789abcdef0123456789abcdef";
const FRAME: &str = "evd_0123456789abcdef0123456789abcdef";

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn load(relative_path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(
        schema_root().join(relative_path),
    )?)?)
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    let schema = load(schema_path)?;
    jsonschema::validator_for(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

fn regenerate() -> bool {
    env::var("VSIFT_REGENERATE_CONTRACT_EXAMPLES").is_ok_and(|value| value == "1")
}

/// A draft with one of each kind of finding: a gap kind outside the
/// vocabulary (#213's example), a closed value in another letter case, an
/// unused citation, a missing required member, a link in the prose and a
/// frame the session does not hold. The `handoff_check` fuzz target's seed
/// copies the file.
fn draft() -> Result<String, Box<dyn std::error::Error>> {
    Ok(fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/handoff/draft-with-findings.md"),
    )?
    .replace("\r\n", "\n"))
}

fn response(data: &HandoffCheckData) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::to_value(OperationResponse::complete(
        CommandName::HandoffCheck.identifier(),
        data,
    )?)?)
}

#[test]
fn a_checked_draft_matches_its_frozen_example() -> TestResult {
    let checker = HandoffChecker::new()?;
    let mut check = checker.check_report(&draft()?);
    let mut records = HandoffSessionRecords::new();
    records.add_segment(SEGMENT);
    check.resolve_in_session(SESSION, &records);
    let data = HandoffCheckData::new(check);
    assert!(!data.valid());
    let value = serde_json::to_value(&data)?;
    validate("handoff-check-data.schema.json", &value)?;
    let response = response(&data)?;
    validate("operation-response.schema.json", &response)?;
    let path = schema_root().join("examples/handoff-check.json");
    if regenerate() {
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&response)?),
        )?;
    }
    assert_eq!(response, load("examples/handoff-check.json")?);
    Ok(())
}

#[test]
fn valid_drafts_and_session_gaps_validate() -> TestResult {
    let checker = HandoffChecker::new()?;
    let skill = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills/vsift/SKILL.md"),
    )?;
    let mut valid = checker.check_report(&skill[skill.find("### 7. REPORT").ok_or("REPORT")?..]);
    let mut records = HandoffSessionRecords::new();
    records.add_segment(SEGMENT);
    records.add_evidence(FRAME, HandoffEvidenceKind::Frame);
    valid.resolve_in_session(SESSION, &records);
    let data = HandoffCheckData::new(valid);
    assert!(data.valid());
    validate(
        "handoff-check-data.schema.json",
        &serde_json::to_value(&data)?,
    )?;
    for gap in [
        HandoffSessionGap::SessionClosed,
        HandoffSessionGap::SessionExpired,
        HandoffSessionGap::SessionNotFound,
    ] {
        let mut check = checker.check_report("no block");
        check.session_unavailable(SESSION, gap);
        let value = serde_json::to_value(HandoffCheckData::new(check))?;
        validate("handoff-check-data.schema.json", &value)?;
    }
    Ok(())
}

#[test]
fn the_schema_lists_exactly_the_published_rules() -> TestResult {
    let schema = load("handoff-check-data.schema.json")?;
    let listed: BTreeSet<&str> = schema["$defs"]["finding"]["properties"]["rule"]["enum"]
        .as_array()
        .ok_or("rule enum")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let published: BTreeSet<&str> = HandoffRule::ALL
        .iter()
        .map(|rule| rule.identifier())
        .collect();
    assert_eq!(listed, published);
    Ok(())
}

#[test]
fn a_forged_verdict_is_refused_by_the_schema() -> TestResult {
    let checker = HandoffChecker::new()?;
    let mut value = serde_json::to_value(HandoffCheckData::new(checker.check_report(&draft()?)))?;
    value["valid"] = json!(true);
    let schema = load("handoff-check-data.schema.json")?;
    assert!(!jsonschema::validator_for(&schema)?.is_valid(&value));
    Ok(())
}

/// `handoff.skeleton` (ADR 0038): the data against its schema and frozen
/// example, and the handoff in it against the skill's handoff schema.
#[test]
fn a_skeleton_matches_its_frozen_example_and_the_handoff_schema() -> TestResult {
    let checker = HandoffChecker::new()?;
    let facts = HandoffSkeletonFacts::new(
        SESSION.to_owned(),
        "src_sha256_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
        Some("trv_0123456789abcdef0123456789abcdef".to_owned()),
        HandoffTranscriptBasis::SuppliedTranscript,
    );
    let data = checker
        .skeleton("What does the dialog show after Save is pressed?", &facts)
        .map_err(|error| format!("{error:?}"))?;
    let value = serde_json::to_value(&data)?;
    validate("handoff-skeleton-data.schema.json", &value)?;
    let skill_schema: Value = serde_json::from_str(&fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills/vsift/handoff.schema.json"),
    )?)?;
    jsonschema::validator_for(&skill_schema)?
        .validate(&value["handoff"])
        .map_err(|error| io::Error::other(format!("handoff.schema.json: {error}")))?;
    let mut check = checker.check_value(value["handoff"].clone());
    check.resolve_in_session(SESSION, &HandoffSessionRecords::new());
    assert!(check.is_valid(), "{:?}", check.errors());

    let response = serde_json::to_value(OperationResponse::complete(
        CommandName::HandoffSkeleton.identifier(),
        &data,
    )?)?;
    validate("operation-response.schema.json", &response)?;
    let path = schema_root().join("examples/handoff-skeleton.json");
    if regenerate() {
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&response)?),
        )?;
    }
    assert_eq!(response, load("examples/handoff-skeleton.json")?);
    Ok(())
}
