//! The `handoff.finish` data (0.3.0, M5b) against its published schema and
//! frozen example: every shape a host can publish (checked, retained,
//! closed, stopped at the draft or the lifecycle) validates, and a forged
//! shape (a write after a stop, a finish without its write) is refused. Set
//! `VSIFT_REGENERATE_CONTRACT_EXAMPLES=1` to rewrite the example after an
//! intended change, and review the diff.

use std::{env, fs, io, path::PathBuf};

use jsonschema::{Retrieve, Uri};
use serde_json::{Value, json};
use vsift_contract::{
    BundleData, BundleSourceInclusion, CommandName, HandoffCheck, HandoffChecker,
    HandoffEvidenceKind, HandoffFinishData, HandoffFinishDecision, HandoffFinishRequest,
    HandoffFinishStop, HandoffSessionGap, HandoffSessionRecords, LifecycleResponse,
    OperationResponse, SessionState, StatusData,
};
use vsift_domain::{SessionId, SourceId};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const SOURCE: &str = "src_sha256_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
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

fn is_valid(instance: &Value) -> Result<bool, Box<dyn std::error::Error>> {
    Ok(jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&load("handoff-finish-data.schema.json")?)?
        .is_valid(instance))
}

fn validate(instance: &Value) -> TestResult {
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&load("handoff-finish-data.schema.json")?)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("handoff-finish-data: {error}")))?;
    Ok(())
}

fn regenerate() -> bool {
    env::var("VSIFT_REGENERATE_CONTRACT_EXAMPLES").is_ok_and(|value| value == "1")
}

/// The skill's own REPORT example, a valid report, with its lifecycle
/// action replaced by `action`.
fn report(action: &str) -> Result<String, Box<dyn std::error::Error>> {
    let skill = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills/vsift/SKILL.md"),
    )?;
    let start = skill.find("```vsift-handoff").ok_or("REPORT example")?;
    let end = skill[start + 3..].find("```").ok_or("REPORT example end")? + start + 6;
    let example = &skill[start..end];
    let left_open = "\"lifecycle\": {\"action\": \"left_open\"}";
    if !example.contains(left_open) {
        return Err("the example's lifecycle".into());
    }
    Ok(example.replace(
        left_open,
        &format!("\"lifecycle\": {{\"action\": \"{action}\"}}"),
    ))
}

/// The report checked with a session that holds every cited identity.
fn checked(action: &str) -> Result<HandoffCheck, Box<dyn std::error::Error>> {
    let mut check = HandoffChecker::new()?.check_report(&report(action)?);
    let mut records = HandoffSessionRecords::new();
    records.add_segment(SEGMENT);
    records.add_evidence(FRAME, HandoffEvidenceKind::Frame);
    check.resolve_in_session(SESSION, &records);
    if !check.is_valid() {
        return Err(format!("{:?}", check.errors()).into());
    }
    Ok(check)
}

fn bundle() -> Result<BundleData, Box<dyn std::error::Error>> {
    Ok(BundleData::new(
        &SessionId::parse(SESSION)?,
        &SourceId::parse(SOURCE)?,
        1_048_576,
        BundleSourceInclusion::EvidenceOnly,
        4,
        262_144,
    ))
}

fn closed_status() -> StatusData {
    StatusData {
        session_id: SESSION.to_owned(),
        state: SessionState::Closed,
        source_id: SOURCE.to_owned(),
        source_bytes: 1_048_576,
        artifact_count: 4,
        artifact_bytes: 262_144,
        generation: 7,
        expires_at: "2026-11-10T12:00:00Z".to_owned(),
        time_anchor: None,
        origin: None,
    }
}

/// A retention after a passing draft: the frozen example.
#[test]
fn a_retention_matches_its_frozen_example() -> TestResult {
    let check = checked("retained")?;
    assert_eq!(
        HandoffFinishRequest::Retain.decide(&check),
        HandoffFinishDecision::Proceed
    );
    let data = HandoffFinishData::retained(check, bundle()?);
    assert!(data.finished());
    let value = serde_json::to_value(&data)?;
    validate(&value)?;
    assert_eq!(value["bundle_validated"], json!(true));
    let response = serde_json::to_value(
        OperationResponse::complete(CommandName::HandoffFinish.identifier(), &data)?
            .with_lifecycle(LifecycleResponse::retained()),
    )?;
    let path = schema_root().join("examples/handoff-finish.json");
    if regenerate() {
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&response)?),
        )?;
    }
    assert_eq!(response, load("examples/handoff-finish.json")?);
    Ok(())
}

/// Every other shape a host publishes validates.
#[test]
fn every_published_shape_validates() -> TestResult {
    validate(&serde_json::to_value(HandoffFinishData::checked(checked(
        "left_open",
    )?))?)?;
    validate(&serde_json::to_value(HandoffFinishData::closed(
        checked("closed")?,
        closed_status(),
    ))?)?;
    // Without a write, a session that cannot be read is a gap, not a stop.
    for gap in [
        HandoffSessionGap::SessionClosed,
        HandoffSessionGap::SessionExpired,
        HandoffSessionGap::SessionNotFound,
    ] {
        let mut check = HandoffChecker::new()?.check_report(&report("left_open")?);
        check.session_unavailable(SESSION, gap);
        assert!(check.is_valid());
        let data = HandoffFinishData::checked(check);
        assert!(data.finished());
        validate(&serde_json::to_value(data)?)?;
    }
    for request in [
        HandoffFinishRequest::Check,
        HandoffFinishRequest::Retain,
        HandoffFinishRequest::Close,
    ] {
        let check = HandoffChecker::new()?.check_report("no block");
        let data = HandoffFinishData::stopped(check, request, HandoffFinishStop::Draft);
        assert_eq!(data.stopped_at(), Some(HandoffFinishStop::Draft));
        validate(&serde_json::to_value(data)?)?;
    }
    for (request, action) in [
        (HandoffFinishRequest::Retain, "left_open"),
        (HandoffFinishRequest::Close, "retained"),
    ] {
        let check = checked(action)?;
        assert_eq!(
            request.decide(&check),
            HandoffFinishDecision::Stop(HandoffFinishStop::Lifecycle)
        );
        let data = HandoffFinishData::stopped(check, request, HandoffFinishStop::Lifecycle);
        validate(&serde_json::to_value(data)?)?;
    }
    Ok(())
}

/// The schema refuses what no host may publish: a bundle after a stop, a
/// finished retention without its bundle, a lifecycle stop that agrees.
#[test]
fn forged_shapes_are_refused() -> TestResult {
    let stopped = serde_json::to_value(HandoffFinishData::stopped(
        checked("left_open")?,
        HandoffFinishRequest::Retain,
        HandoffFinishStop::Lifecycle,
    ))?;
    assert!(is_valid(&stopped)?);
    let mut forged = stopped.clone();
    forged["retain"] = serde_json::to_value(bundle()?)?;
    assert!(!is_valid(&forged)?);
    let mut forged = stopped.clone();
    forged["lifecycle"]["agrees"] = json!(true);
    assert!(!is_valid(&forged)?);
    let mut forged = stopped;
    forged["finished"] = json!(true);
    assert!(!is_valid(&forged)?);

    let retained = serde_json::to_value(HandoffFinishData::retained(
        checked("retained")?,
        bundle()?,
    ))?;
    let mut forged = retained.clone();
    forged["retain"] = Value::Null;
    assert!(!is_valid(&forged)?);
    let mut forged = retained;
    forged["bundle_validated"] = json!(false);
    assert!(!is_valid(&forged)?);
    Ok(())
}
