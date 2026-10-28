//! Contract for `ingest --url` (ADR 0028, #407): the `origin` of a fetched
//! source in `ingest` data and the `SOURCE_FETCH_FAILED` failure with its
//! `error.reason`, against their examples and the v1 schemas.

use std::{error::Error, fs, io, path::PathBuf};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_application::OpenSessionOutcome;
use vsift_contract::{
    CommandName, LifecycleResponse, OpenData, OperationResponse, TerminalEventResponse,
    source_fetch_remediation,
};
use vsift_domain::{
    AddressCheck, DirectLinkOrigin, FailureCode, HostName, PublicationGuarantee, SessionId,
    SessionLifetime, Sha256Hex, SourceFetchReason, SourceId, SourceOrigin, StorageGeneration,
};

type TestResult = Result<(), Box<dyn Error>>;

const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const SOURCE: &str = "src_sha256_d7ccece71288c5ff35d7b16c871a93a8ed48bbb7380617899575069b4d6545f4";
const EXPIRES_AT: &str = "2026-09-25T00:00:00Z";

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn load(relative: &str) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(
        schema_root().join(relative),
    )?)?)
}

fn regenerate(relative: &str, value: &Value) -> TestResult {
    if std::env::var("VSIFT_REGENERATE_CONTRACT_EXAMPLES").is_ok_and(|value| value == "1") {
        fs::write(
            schema_root().join(relative),
            format!("{}\n", serde_json::to_string_pretty(value)?),
        )?;
    }
    Ok(())
}

struct PublishedSchemas;

impl Retrieve for PublishedSchemas {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn Error + Send + Sync>> {
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

fn validate(schema: &str, instance: &Value) -> TestResult {
    let definition: Value = serde_json::from_str(&fs::read_to_string(schema_root().join(schema))?)?;
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&definition)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema}: {error}")))?;
    Ok(())
}

/// RM09's NASA file, as `ingest --url` would record it: the link's digest,
/// its host, and the size and time of the fetch.
fn nasa_origin() -> Result<SourceOrigin, Box<dyn Error>> {
    let host = HostName::parse("images-assets.nasa.gov")?;
    Ok(SourceOrigin::DirectHttps(DirectLinkOrigin::new(
        Sha256Hex::parse("6c1f0e3b7a5d9e2c4b8a7f6e5d4c3b2a19087f6e5d4c3b2a1908f7e6d5c4b3a2")
            .map_err(|_| "digest")?,
        (host.clone(), host),
        0,
        AddressCheck::Direct,
        1_790_208_000,
        72_873_726,
    )?))
}

#[test]
fn an_ingest_from_a_link_states_its_origin_and_matches_its_example() -> TestResult {
    let opened = OpenSessionOutcome {
        session_id: SessionId::parse(SESSION)?,
        source_id: SourceId::parse(SOURCE)?,
        source_bytes: 72_873_726,
        generation: StorageGeneration::from_value(1),
        publication: PublicationGuarantee::ProcessCrashConsistent,
        lifetime: SessionLifetime::open(1_790_208_000)?,
        time_anchors: vsift_domain::TimeAnchors::none(),
        origin: Some(nasa_origin()?),
    };
    let value = serde_json::to_value(
        OperationResponse::complete("ingest", &OpenData::new(&opened, EXPIRES_AT.to_owned()))?
            .with_lifecycle(LifecycleResponse::ephemeral(EXPIRES_AT.to_owned())),
    )?;
    regenerate("examples/ingest.url.json", &value)?;
    validate("operation-response.schema.json", &value)?;
    validate("ingest-data.schema.json", &value["data"])?;
    assert_eq!(value, load("examples/ingest.url.json")?);
    let origin = &value["data"]["origin"];
    assert_eq!(origin["kind"], "direct_https");
    assert_eq!(origin["final_host"], "images-assets.nasa.gov");
    assert_eq!(origin["retrieved_at"], "2026-09-24T00:00:00Z");
    assert!(origin.get("link").is_none() && origin.get("url").is_none());
    Ok(())
}

#[test]
fn a_refused_link_is_source_fetch_failed_with_its_reason_and_matches_its_example() -> TestResult {
    let response = OperationResponse::<Value>::failure_with_remediation(
        CommandName::Ingest.identifier(),
        FailureCode::SourceFetchFailed,
        source_fetch_remediation(SourceFetchReason::AddressRefused).to_owned(),
    )
    .with_error_reason(SourceFetchReason::AddressRefused.identifier());
    let value = serde_json::to_value(&response)?;
    regenerate("examples/ingest-url-address-refused.json", &value)?;
    validate("operation-response.schema.json", &value)?;
    assert_eq!(value, load("examples/ingest-url-address-refused.json")?);
    assert_eq!(value["error"]["code"], "SOURCE_FETCH_FAILED");
    assert_eq!(value["error"]["reason"], "address_refused");
    assert_eq!(value["error"]["retryable"], false);
    let event = serde_json::to_value(TerminalEventResponse::new(response))?;
    validate("terminal-event.schema.json", &event)?;
    // Every reason validates, and another code carries none.
    for reason in SourceFetchReason::ALL {
        let value = serde_json::to_value(
            OperationResponse::<Value>::failure_with_remediation(
                "ingest",
                FailureCode::SourceFetchFailed,
                source_fetch_remediation(reason).to_owned(),
            )
            .with_error_reason(reason.identifier()),
        )?;
        validate("operation-response.schema.json", &value)?;
    }
    let plain = serde_json::to_value(OperationResponse::<Value>::failure(
        "ingest",
        FailureCode::InvalidSource,
    ))?;
    assert!(plain["error"].get("reason").is_none());
    Ok(())
}
