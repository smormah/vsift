//! Conformance of serialized contract values with the published v1 schemas and
//! frozen examples.
//!
//! The CLI suite proves the binary's output; these tests prove the contract
//! crate itself, so any host built on it emits schema-valid, example-identical
//! JSON without running the CLI.

use std::{fs, io, path::PathBuf};

use serde_json::Value;
use vsift_application::{RuntimeDiagnosis, SetupProfile, SetupSelectionState, plan_managed_setup};
use vsift_contract::{
    BundleData, BundleSourceInclusion, CleanData, CleanItem, CleanItemOutcome, CommandName,
    ConfiguredSelectionResponse, DependencyLookup, LifecycleResponse, ListedSession,
    OperationResponse, PageData, SavedSetupPlan, SessionState, SetupCheckResponse,
    SetupPlanResponse, StatusData, TerminalEventResponse,
};
use vsift_domain::{
    DependencyState, DependencyStatus, FailureCode, ManagedTarget, RuntimeDependency,
    RuntimeReadiness, SessionId, SourceId,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const SOURCE: &str = "src_sha256_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const EXPIRES_AT: &str = "2026-09-24T00:00:00Z";

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

fn diagnosis(state: &DependencyState, readiness: RuntimeReadiness) -> RuntimeDiagnosis {
    RuntimeDiagnosis {
        readiness,
        dependencies: [
            RuntimeDependency::Ffmpeg,
            RuntimeDependency::Ffprobe,
            RuntimeDependency::Whisper,
        ]
        .into_iter()
        .map(|dependency| DependencyStatus {
            dependency,
            state: state.clone(),
        })
        .collect(),
    }
}

fn unavailable_plan() -> Result<OperationResponse<Value>, serde_json::Error> {
    let plan = plan_managed_setup(
        SetupProfile::Desktop,
        diagnosis(&DependencyState::Missing, RuntimeReadiness::Blocked),
        ManagedTarget::WindowsX86_64,
        SetupSelectionState::default(),
        1_800_000_000,
        None,
    );
    OperationResponse::complete("setup.plan", &SetupPlanResponse::new(&plan))
}

fn status(state: SessionState) -> StatusData {
    StatusData {
        session_id: String::from(SESSION),
        state,
        source_id: String::from(SOURCE),
        source_bytes: 1_024,
        artifact_count: 2,
        artifact_bytes: 512,
        generation: 3,
        expires_at: String::from(EXPIRES_AT),
    }
}

#[test]
fn failure_envelope_matches_the_frozen_example() -> TestResult {
    let response = serde_json::to_value(OperationResponse::failure(
        "setup.install",
        FailureCode::CommandNotImplemented,
    ))?;

    validate("operation-response.schema.json", &response)?;
    assert_eq!(response, load("examples/operation-error.json")?);
    Ok(())
}

#[test]
fn terminal_event_matches_the_frozen_example() -> TestResult {
    let event = serde_json::to_value(TerminalEventResponse::new(OperationResponse::failure(
        "setup.install",
        FailureCode::CommandNotImplemented,
    )))?;

    validate("terminal-event.schema.json", &event)?;
    validate("operation-response.schema.json", &event["result"])?;
    assert_eq!(event, load("examples/terminal-event.json")?);
    Ok(())
}

#[test]
fn published_failure_codes_produce_schema_valid_envelopes() -> TestResult {
    for code in [
        FailureCode::Internal,
        FailureCode::InvalidArgument,
        FailureCode::UnsupportedSchema,
        FailureCode::MissingCapability,
        FailureCode::CommandNotImplemented,
        FailureCode::InvalidSource,
        FailureCode::Busy,
        FailureCode::DeadlineExceeded,
        FailureCode::ResourceLimit,
        FailureCode::Cancelled,
        FailureCode::StorageIo,
        FailureCode::IntegrityFailure,
    ] {
        let response = serde_json::to_value(OperationResponse::failure("session.status", code))?;
        validate("operation-response.schema.json", &response)?;
        assert_eq!(response["error"]["code"], code.identifier());
    }
    Ok(())
}

#[test]
fn blocked_setup_check_matches_the_frozen_example() -> TestResult {
    let response = serde_json::to_value(SetupCheckResponse::new(
        &diagnosis(&DependencyState::Missing, RuntimeReadiness::Blocked),
        SetupProfile::Desktop,
        |_| DependencyLookup::FilteredPath,
    ))?;

    validate("setup-check-response.schema.json", &response)?;
    assert_eq!(response, load("examples/setup-check.blocked.json")?);
    Ok(())
}

#[test]
fn ready_setup_check_reports_sanitized_detail_and_lookup_provenance() -> TestResult {
    let state = DependencyState::Available {
        version: format!("fake\u{1b}]8;;file:///secret\u{7}{}", "v".repeat(400)),
    };
    let response = serde_json::to_value(SetupCheckResponse::new(
        &diagnosis(&state, RuntimeReadiness::Ready),
        SetupProfile::Worker,
        |dependency| match dependency {
            RuntimeDependency::Ffmpeg => DependencyLookup::ExplicitPath,
            RuntimeDependency::Ffprobe => DependencyLookup::ConfiguredUserPath,
            RuntimeDependency::Whisper => DependencyLookup::FilteredPath,
        },
    ))?;

    validate("setup-check-response.schema.json", &response)?;
    assert_eq!(response["profile"], "worker");
    assert_eq!(response["status"], "ready");
    let dependencies = response["dependencies"]
        .as_array()
        .ok_or("dependencies missing")?;
    let lookups: Vec<&Value> = dependencies
        .iter()
        .map(|dependency| &dependency["lookup"])
        .collect();
    assert_eq!(
        lookups,
        ["explicit_path", "configured_user_path", "filtered_path"]
    );
    for dependency in dependencies {
        let detail = dependency["detail"].as_str().ok_or("detail missing")?;
        assert!(detail.len() <= 240);
        assert!(!detail.contains('\u{1b}'));
        assert!(!detail.contains('\u{7}'));
        assert_eq!(dependency["validation"], "executable_probe_only");
        assert!(dependency["remediation"].is_null());
    }
    Ok(())
}

#[test]
fn unavailable_setup_plan_matches_the_frozen_example() -> TestResult {
    let response = serde_json::to_value(unavailable_plan()?)?;

    validate("setup-plan.schema.json", &response)?;
    assert_eq!(response, load("examples/setup-plan.unavailable.json")?);
    Ok(())
}

#[test]
fn saved_plan_round_trips_and_is_accepted_only_unchanged() -> TestResult {
    let bytes = serde_json::to_vec(&unavailable_plan()?)?;
    let saved: SavedSetupPlan = serde_json::from_slice(&bytes)?;
    let current = SetupPlanResponse::new(&plan_managed_setup(
        SetupProfile::Desktop,
        diagnosis(&DependencyState::Missing, RuntimeReadiness::Blocked),
        ManagedTarget::WindowsX86_64,
        SetupSelectionState::default(),
        1_800_000_000,
        None,
    ));
    let changed = SetupPlanResponse::new(&plan_managed_setup(
        SetupProfile::Worker,
        diagnosis(&DependencyState::Missing, RuntimeReadiness::Blocked),
        ManagedTarget::WindowsX86_64,
        SetupSelectionState::default(),
        1_800_000_000,
        None,
    ));

    assert_eq!(saved.validate_envelope(), Ok(()));
    assert_eq!(saved.profile(), Ok(SetupProfile::Desktop));
    assert_eq!(saved.require_same_plan(&current), Ok(()));
    assert_eq!(
        saved.require_same_plan(&changed),
        Err(FailureCode::InvalidArgument)
    );
    Ok(())
}

#[test]
fn saved_plan_rejects_unknown_fields_at_every_level() -> TestResult {
    let original = serde_json::to_value(unavailable_plan()?)?;
    for pointer in ["", "/data", "/data/local_asr_model", "/data/dependencies/0"] {
        let mut instance = original.clone();
        instance
            .pointer_mut(pointer)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| io::Error::other(format!("{pointer} is not an object")))?
            .insert(String::from("unreviewed"), Value::Bool(true));

        assert!(
            serde_json::from_value::<SavedSetupPlan>(instance).is_err(),
            "{pointer}"
        );
    }
    Ok(())
}

#[test]
fn saved_plan_rejects_a_modified_envelope() -> TestResult {
    let original = serde_json::to_value(unavailable_plan()?)?;
    for (field, replacement) in [
        ("schema_version", serde_json::json!("2")),
        ("command", serde_json::json!("setup.check")),
        ("operation_id", serde_json::json!("op_0123456789abcdef")),
        ("status", serde_json::json!("partial")),
        ("warnings", serde_json::json!(["edited"])),
        ("error", serde_json::json!({})),
        ("coverage", serde_json::json!({})),
        ("lifecycle", serde_json::json!({})),
    ] {
        let mut instance = original.clone();
        instance[field] = replacement;
        let saved: SavedSetupPlan = serde_json::from_value(instance)?;

        assert_eq!(
            saved.validate_envelope(),
            Err(FailureCode::InvalidArgument),
            "{field}"
        );
    }

    let mut unknown_profile = original;
    unknown_profile["data"]["profile"] = serde_json::json!("server");
    let saved: SavedSetupPlan = serde_json::from_value(unknown_profile)?;
    assert_eq!(saved.profile(), Err(FailureCode::InvalidArgument));
    Ok(())
}

#[test]
fn configured_selection_is_a_schema_valid_operation() -> TestResult {
    let response = serde_json::to_value(OperationResponse::complete(
        "setup.configure",
        &ConfiguredSelectionResponse::new(RuntimeDependency::Whisper),
    )?)?;

    validate("operation-response.schema.json", &response)?;
    assert_eq!(response["data"]["dependency"], "whisper");
    assert_eq!(response["data"]["source"], "configured_user_path");
    Ok(())
}

#[test]
fn session_results_are_schema_valid_operations() -> TestResult {
    let session = SessionId::parse(SESSION)?;
    let source = SourceId::parse(SOURCE)?;
    let ephemeral = || LifecycleResponse::ephemeral(String::from(EXPIRES_AT));

    let responses = [
        OperationResponse::complete("session.status", &status(SessionState::Open))?
            .with_lifecycle(ephemeral()),
        OperationResponse::partial(
            "session.list",
            &PageData {
                items: vec![
                    ListedSession::indexed(&session, status(SessionState::Closed)),
                    ListedSession::initializing(&session),
                    ListedSession::unavailable(&session, FailureCode::IntegrityFailure),
                ],
                next_cursor: Some(4),
            },
            "Some session records could not be read.",
        )?,
        OperationResponse::partial(
            "session.clean",
            &CleanData {
                items: vec![
                    CleanItem::examined(&session, CleanItemOutcome::Eligible),
                    CleanItem::skipped(&session, FailureCode::Busy),
                ],
                next_cursor: None,
                dry_run: true,
            },
            "Some sessions were not cleaned.",
        )?,
        OperationResponse::complete(
            "bundle.validate",
            &BundleData::new(
                &session,
                &source,
                1_024,
                BundleSourceInclusion::EvidenceOnly,
                2,
                512,
            ),
        )?
        .with_lifecycle(LifecycleResponse::retained()),
    ];
    for response in responses {
        validate(
            "operation-response.schema.json",
            &serde_json::to_value(response)?,
        )?;
    }
    Ok(())
}

#[test]
fn listed_session_shape_is_stable() -> TestResult {
    let session = SessionId::parse(SESSION)?;
    let page = serde_json::to_value(PageData {
        items: vec![ListedSession::indexed(&session, status(SessionState::Open))],
        next_cursor: None,
    })?;

    assert_eq!(
        page,
        serde_json::json!({
            "items": [{
                "session_id": SESSION,
                "state": "open",
                "status": {
                    "session_id": SESSION,
                    "state": "open",
                    "source_id": SOURCE,
                    "source_bytes": 1_024,
                    "artifact_count": 2,
                    "artifact_bytes": 512,
                    "generation": 3,
                    "expires_at": EXPIRES_AT
                },
                "error_code": null
            }],
            "next_cursor": null
        })
    );
    Ok(())
}

/// The members of the published `error.code` enum.
fn published_failure_codes() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    load("operation-response.schema.json")?
        .pointer("/$defs/error/properties/code/enum")
        .and_then(Value::as_array)
        .ok_or("operation-response.schema.json has no error code enum")?
        .iter()
        .map(|code| {
            code.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "error code enum member is not a string".into())
        })
        .collect()
}

#[test]
fn every_failure_code_is_published_and_nothing_else_is() -> TestResult {
    let published = published_failure_codes()?;

    for code in FailureCode::ALL {
        assert!(
            published.iter().any(|member| member == code.identifier()),
            "{} is missing from the published error code enum",
            code.identifier()
        );
    }
    for member in &published {
        assert!(
            FailureCode::ALL
                .iter()
                .any(|code| code.identifier() == member),
            "{member} is published but no failure code produces it"
        );
    }
    Ok(())
}

#[test]
fn every_failure_code_produces_schema_valid_json_and_jsonl_failures() -> TestResult {
    for code in FailureCode::ALL {
        let event = serde_json::to_value(TerminalEventResponse::new(OperationResponse::failure(
            CommandName::SessionStatus.identifier(),
            code,
        )))?;

        validate("terminal-event.schema.json", &event)?;
        validate("operation-response.schema.json", &event["result"])?;
        assert_eq!(event["result"]["error"]["code"], code.identifier());
    }
    Ok(())
}

#[test]
fn every_command_name_produces_schema_valid_json_and_jsonl_envelopes() -> TestResult {
    for name in CommandName::ALL {
        let complete = serde_json::to_value(OperationResponse::complete(
            name.identifier(),
            &serde_json::json!({}),
        )?)?;
        let event = serde_json::to_value(TerminalEventResponse::new(OperationResponse::failure(
            name.identifier(),
            FailureCode::CommandNotImplemented,
        )))?;

        validate("operation-response.schema.json", &complete)?;
        validate("terminal-event.schema.json", &event)?;
        validate("operation-response.schema.json", &event["result"])?;
        assert_eq!(complete["command"], name.identifier());
        assert_eq!(event["command"], name.identifier());
    }
    Ok(())
}

#[test]
fn command_patterns_still_reject_malformed_identifiers() -> TestResult {
    let original = serde_json::to_value(TerminalEventResponse::new(OperationResponse::failure(
        CommandName::SetupConfigureModel.identifier(),
        FailureCode::CommandNotImplemented,
    )))?;
    let operation_validator = jsonschema::validator_for(&load("operation-response.schema.json")?)?;
    let event_validator = jsonschema::validator_for(&load("terminal-event.schema.json")?)?;

    for malformed in [
        "",
        "Setup.check",
        "setup.",
        ".check",
        "setup..check",
        "setup.check.extra",
        "-setup",
        "setup-",
        "setup.configure-",
        "setup.-model",
        "setup.configure--model",
        "setup.configure_model",
        "setup check",
    ] {
        let mut event = original.clone();
        event["command"] = Value::from(malformed);
        event["result"]["command"] = Value::from(malformed);

        assert!(
            !event_validator.is_valid(&event),
            "event accepted {malformed:?}"
        );
        assert!(
            !operation_validator.is_valid(&event["result"]),
            "operation accepted {malformed:?}"
        );
    }
    Ok(())
}
