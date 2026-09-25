//! The additive `local_asr` object of `setup check` (maintainer decision D4,
//! P07 increment 3c).
//!
//! Every model status, verification outcome, not-run reason and failure check
//! and reason serializes to a schema-valid response; the frozen example is
//! exactly what the contract emits; the legacy constant fields are unchanged;
//! and a response written before 3c (without `local_asr`) is still valid v1.

use std::{collections::BTreeSet, fs, io, path::PathBuf};

use serde_json::Value;
use vsift_application::{
    AsrFailure, AsrFailureReason, AsrStage, LocalAsrCheckFailure, LocalAsrCheckOutcome,
    LocalAsrModelStatus, LocalAsrNotRunReason, LocalAsrSetupStatus, LocalAsrVerificationFailure,
    LocalAsrVerificationSource, RuntimeDiagnosis, SetupProfile,
};
use vsift_contract::{DependencyLookup, SetupCheckResponse};
use vsift_domain::{
    DependencyState, DependencyStatus, ProviderOutputError, ReviewedAsrModel, RuntimeDependency,
    RuntimeReadiness, TranscriptRevisionError,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const STAGES: [AsrStage; 6] = [
    AsrStage::Planning,
    AsrStage::RecognizerIdentity,
    AsrStage::AudioExtraction,
    AsrStage::Recognition,
    AsrStage::OutputValidation,
    AsrStage::Assembly,
];

const REASONS: [AsrFailureReason; 17] = [
    AsrFailureReason::InvalidRange,
    AsrFailureReason::TooManyChunks,
    AsrFailureReason::ModelChanged,
    AsrFailureReason::ModelUnavailable,
    AsrFailureReason::UnpinnedModel,
    AsrFailureReason::Cancelled,
    AsrFailureReason::Deadline,
    AsrFailureReason::Busy,
    AsrFailureReason::ResourceLimit,
    AsrFailureReason::AudioUnavailable,
    AsrFailureReason::ProviderFailed,
    AsrFailureReason::UnparseableOutput,
    AsrFailureReason::MalformedOutput(ProviderOutputError::TooManySegments),
    AsrFailureReason::AbnormalTermination,
    AsrFailureReason::Workspace,
    AsrFailureReason::Io,
    AsrFailureReason::InvalidRun(TranscriptRevisionError::InvalidAsrRun),
];

const MODELS: [LocalAsrModelStatus; 5] = [
    LocalAsrModelStatus::NotSelected,
    LocalAsrModelStatus::Unreadable,
    LocalAsrModelStatus::Unrecognised,
    LocalAsrModelStatus::KnownPinned(ReviewedAsrModel::Base),
    LocalAsrModelStatus::KnownPinned(ReviewedAsrModel::BaseQ5_1),
];

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn load(relative_path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(
        schema_root().join(relative_path),
    )?)?)
}

fn validator() -> Result<jsonschema::Validator, Box<dyn std::error::Error>> {
    Ok(jsonschema::validator_for(&load(
        "setup-check-response.schema.json",
    )?)?)
}

fn diagnosis(state: &DependencyState, readiness: RuntimeReadiness) -> RuntimeDiagnosis {
    RuntimeDiagnosis {
        readiness,
        dependencies: RuntimeDependency::ALL
            .into_iter()
            .map(|dependency| DependencyStatus {
                dependency,
                state: state.clone(),
            })
            .collect(),
    }
}

fn response(local_asr: LocalAsrSetupStatus) -> Result<Value, serde_json::Error> {
    serde_json::to_value(SetupCheckResponse::new(
        &diagnosis(&DependencyState::Missing, RuntimeReadiness::Blocked),
        SetupProfile::Desktop,
        |_| DependencyLookup::FilteredPath,
        &local_asr,
    ))
}

/// Every verification outcome `setup check` can report.
fn outcomes() -> Vec<LocalAsrCheckOutcome> {
    let mut outcomes = vec![
        LocalAsrCheckOutcome::Verified(LocalAsrVerificationSource::Recorded),
        LocalAsrCheckOutcome::Verified(LocalAsrVerificationSource::RanNow),
        LocalAsrCheckOutcome::Failed(LocalAsrCheckFailure::BudgetExceeded),
    ];
    for failure in [
        LocalAsrVerificationFailure::FixtureIntegrity,
        LocalAsrVerificationFailure::Workspace,
        LocalAsrVerificationFailure::FixtureMedia,
        LocalAsrVerificationFailure::UnexpectedTranscript,
    ] {
        outcomes.push(LocalAsrCheckOutcome::Failed(
            LocalAsrCheckFailure::Verification(failure),
        ));
    }
    for stage in STAGES {
        for reason in REASONS {
            outcomes.push(LocalAsrCheckOutcome::Failed(
                LocalAsrCheckFailure::Verification(LocalAsrVerificationFailure::Transcription(
                    AsrFailure { stage, reason },
                )),
            ));
        }
    }
    for reason in [
        LocalAsrNotRunReason::MediaToolsUnavailable,
        LocalAsrNotRunReason::WhisperUnavailable,
        LocalAsrNotRunReason::ModelNotSelected,
        LocalAsrNotRunReason::ModelNotPinned,
    ] {
        outcomes.push(LocalAsrCheckOutcome::NotRun(reason));
    }
    outcomes
}

fn schema_enum(
    schema: &Value,
    pointer: &str,
) -> Result<BTreeSet<String>, Box<dyn std::error::Error>> {
    Ok(schema
        .pointer(pointer)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{pointer} is not an enum"))?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect())
}

#[test]
fn a_verified_setup_matches_the_frozen_example() -> TestResult {
    let available = |detail: &str| DependencyState::Available {
        version: detail.to_owned(),
    };
    let mut report = diagnosis(&DependencyState::Missing, RuntimeReadiness::Ready);
    for (status, detail) in report.dependencies.iter_mut().zip([
        "ffmpeg version n9.0.1-11-ge47273f4d9-20260831 Copyright (c) 2000-2026 the FFmpeg developers",
        "ffprobe version n9.0.1-11-ge47273f4d9-20260831 Copyright (c) 2007-2026 the FFmpeg developers",
        "whisper.cpp v1.9.2 (reviewed build)",
    ]) {
        status.state = available(detail);
    }
    let value = serde_json::to_value(SetupCheckResponse::new(
        &report,
        SetupProfile::Desktop,
        |_| DependencyLookup::ConfiguredUserPath,
        &LocalAsrSetupStatus {
            model: LocalAsrModelStatus::KnownPinned(ReviewedAsrModel::Base),
            verification: LocalAsrCheckOutcome::Verified(LocalAsrVerificationSource::RanNow),
        },
    ))?;
    validator()?
        .validate(&value)
        .map_err(|error| io::Error::other(error.to_string()))?;
    assert_eq!(value, load("examples/setup-check.local-asr.json")?);
    // D4: the legacy fields keep their v1 constants whatever local ASR says.
    assert_eq!(value["verification_scope"], "executable_probe_only");
    assert_eq!(value["local_asr_model"], "not_checked");
    Ok(())
}

#[test]
fn every_model_status_and_outcome_is_schema_valid_and_consistent() -> TestResult {
    let validator = validator()?;
    for model in MODELS {
        for verification in outcomes() {
            let value = response(LocalAsrSetupStatus {
                model,
                verification,
            })?;
            validator
                .validate(&value)
                .map_err(|error| io::Error::other(format!("{verification:?}: {error}")))?;
            let local = &value["local_asr"];
            assert_eq!(local["model"]["status"], model.identifier());
            assert_eq!(
                local["model"]["profile"].as_str(),
                model.profile().map(ReviewedAsrModel::identifier)
            );
            let reported = &local["verification"];
            assert_eq!(reported["status"], verification.identifier());
            let present: Vec<&str> = ["source", "not_run_reason", "check", "reason"]
                .into_iter()
                .filter(|field| !reported[*field].is_null())
                .collect();
            match verification {
                LocalAsrCheckOutcome::Verified(source) => {
                    assert_eq!(present, ["source"]);
                    assert_eq!(reported["source"], source.identifier());
                }
                LocalAsrCheckOutcome::Failed(failure) => {
                    assert_eq!(present, ["check", "reason"]);
                    assert_eq!(reported["check"], failure.check());
                    assert_eq!(reported["reason"], failure.reason());
                }
                LocalAsrCheckOutcome::NotRun(reason) => {
                    assert_eq!(present, ["not_run_reason"]);
                    assert_eq!(reported["not_run_reason"], reason.identifier());
                }
            }
        }
    }
    Ok(())
}

/// The schema lists exactly the identifiers the contract can emit, so a new
/// stage or reason fails here until the schema and documentation say so.
#[test]
fn schema_enums_are_exactly_the_emitted_identifiers() -> TestResult {
    let schema = load("setup-check-response.schema.json")?;
    let verification = "/properties/local_asr/properties/verification/properties";
    let mut checks = BTreeSet::new();
    let mut reasons = BTreeSet::new();
    let mut not_run = BTreeSet::new();
    let mut sources = BTreeSet::new();
    for outcome in outcomes() {
        match outcome {
            LocalAsrCheckOutcome::Verified(source) => {
                sources.insert(source.identifier().to_owned());
            }
            LocalAsrCheckOutcome::Failed(failure) => {
                checks.insert(failure.check().to_owned());
                reasons.insert(failure.reason().to_owned());
            }
            LocalAsrCheckOutcome::NotRun(reason) => {
                not_run.insert(reason.identifier().to_owned());
            }
        }
    }
    let without_null = |mut set: BTreeSet<String>| {
        set.remove("null");
        set
    };
    for (field, emitted) in [
        ("check", checks),
        ("reason", reasons),
        ("not_run_reason", not_run),
        ("source", sources),
    ] {
        let published = schema
            .pointer(&format!("{verification}/{field}/enum"))
            .and_then(Value::as_array)
            .ok_or("enum missing")?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        assert_eq!(without_null(published), emitted, "{field}");
    }
    let model_statuses: BTreeSet<String> = MODELS
        .iter()
        .map(|model| model.identifier().to_owned())
        .collect();
    assert_eq!(
        schema_enum(
            &schema,
            "/properties/local_asr/properties/model/properties/status/enum"
        )?,
        model_statuses
    );
    Ok(())
}

#[test]
fn a_response_from_before_local_asr_reporting_is_still_valid_v1() -> TestResult {
    let mut earlier = load("examples/setup-check.local-asr.json")?;
    earlier
        .as_object_mut()
        .ok_or("the example is not an object")?
        .remove("local_asr");
    validator()?
        .validate(&earlier)
        .map_err(|error| io::Error::other(error.to_string()))?;

    let mut inconsistent = load("examples/setup-check.local-asr.json")?;
    inconsistent["local_asr"]["verification"]["not_run_reason"] = Value::from("model_not_selected");
    assert!(!validator()?.is_valid(&inconsistent));
    let mut unpinned_profile = load("examples/setup-check.local-asr.json")?;
    unpinned_profile["local_asr"]["model"]["status"] = Value::from("unrecognised");
    assert!(!validator()?.is_valid(&unpinned_profile));
    Ok(())
}
