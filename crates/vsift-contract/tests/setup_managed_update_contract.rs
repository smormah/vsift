//! The additive `managed_updates` array of `setup check` (since `0.2.3`).
//!
//! A managed version the check used that the catalogue supersedes is
//! reported with the pin that replaces it, the reason and the step; the
//! example is exactly what the contract emits; the array is empty, never
//! absent, when nothing is superseded; every reason is schema-valid; and a
//! response from an earlier build, which has no array, is still valid v1.

use std::{fs, io, path::PathBuf};

use serde_json::Value;
use vsift_application::{
    LocalAsrCheckOutcome, LocalAsrModelStatus, LocalAsrSetupStatus, LocalAsrVerificationSource,
    ManagedPlanAvailability, ManagedUpdate, RuntimeDiagnosis, SetupProfile, SupersessionReason,
};
use vsift_contract::{DependencyLookup, SetupCheckResponse, managed_update_next_step};
use vsift_domain::{
    DependencyState, DependencyStatus, ManagedComponent, ReviewedAsrModel, RuntimeDependency,
    RuntimeReadiness,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn schemas() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn load(relative: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(
        schemas().join(relative),
    )?)?)
}

fn validator() -> Result<jsonschema::Validator, Box<dyn std::error::Error>> {
    Ok(jsonschema::validator_for(&load(
        "setup-check-response.schema.json",
    )?)?)
}

fn valid(value: &Value) -> TestResult {
    validator()?
        .validate(value)
        .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(())
}

/// A machine whose managed whisper.cpp is the v1.9.2 that `0.2.2` installed.
fn superseded_whisper(reason: SupersessionReason) -> ManagedUpdate {
    ManagedUpdate {
        component: ManagedComponent::WhisperCli,
        selected_version: String::from("whisper.cpp-v1.9.2-ubuntu-x64"),
        reviewed_version: String::from("whisper.cpp-v1.9.5-ubuntu-x64"),
        reason,
    }
}

fn response(updates: &[ManagedUpdate]) -> Result<Value, Box<dyn std::error::Error>> {
    let details = [
        "ffmpeg version n9.0.1-11-ge47273f4d9-20260831 Copyright (c) 2000-2026 the FFmpeg developers",
        "ffprobe version n9.0.1-11-ge47273f4d9-20260831 Copyright (c) 2007-2026 the FFmpeg developers",
        "whisper.cpp v1.9.2 (reviewed build)",
    ];
    let diagnosis = RuntimeDiagnosis {
        readiness: RuntimeReadiness::Ready,
        dependencies: RuntimeDependency::ALL
            .into_iter()
            .zip(details)
            .map(|(dependency, detail)| DependencyStatus {
                dependency,
                state: DependencyState::Available {
                    version: detail.to_owned(),
                },
            })
            .collect(),
    };
    Ok(serde_json::to_value(
        SetupCheckResponse::new(
            &diagnosis,
            SetupProfile::Desktop,
            |_| DependencyLookup::ManagedVersion,
            ManagedPlanAvailability::Qualified,
            &LocalAsrSetupStatus {
                model: LocalAsrModelStatus::KnownPinned(ReviewedAsrModel::Base),
                verification: LocalAsrCheckOutcome::Verified(LocalAsrVerificationSource::Recorded),
            },
        )
        .with_managed_updates(updates),
    )?)
}

#[test]
fn a_superseded_managed_whisper_matches_the_example() -> TestResult {
    let value = response(&[superseded_whisper(SupersessionReason::SecurityFixes)])?;
    valid(&value)?;
    assert_eq!(value, load("examples/setup-check.managed-update.json")?);
    let update = &value["managed_updates"][0];
    assert_eq!(update["reason"], "security_fixes");
    let step = update["next_step"].as_str().ok_or("no next_step")?;
    assert!(step.contains("whisper.cpp-v1.9.5-ubuntu-x64"), "{step}");
    assert!(step.contains("security fixes"), "{step}");
    assert!(
        step.contains("setup plan") && step.contains("setup install"),
        "{step}"
    );
    Ok(())
}

#[test]
fn nothing_superseded_is_an_empty_array_never_an_absent_one() -> TestResult {
    let value = response(&[])?;
    valid(&value)?;
    assert_eq!(value["managed_updates"], serde_json::json!([]));
    Ok(())
}

#[test]
fn every_reason_and_component_is_schema_valid_and_the_step_is_bounded() -> TestResult {
    for reason in SupersessionReason::ALL {
        for component in [
            ManagedComponent::MediaTools,
            ManagedComponent::WhisperCli,
            ManagedComponent::WhisperModel,
        ] {
            let longest = "a".repeat(64);
            let update = ManagedUpdate {
                component,
                selected_version: longest.clone(),
                reviewed_version: longest,
                reason,
            };
            let value = response(std::slice::from_ref(&update))?;
            valid(&value)?;
            assert_eq!(value["managed_updates"][0]["reason"], reason.identifier());
            assert_eq!(
                value["managed_updates"][0]["component"],
                component.identifier()
            );
            assert!(managed_update_next_step(&update).len() <= 240);
            assert_eq!(
                managed_update_next_step(&update).contains("security fixes"),
                reason == SupersessionReason::SecurityFixes
            );
        }
    }
    Ok(())
}

#[test]
fn a_response_without_the_array_is_still_valid_v1() -> TestResult {
    let mut value = response(&[superseded_whisper(SupersessionReason::Maintenance)])?;
    value
        .as_object_mut()
        .ok_or("not an object")?
        .remove("managed_updates");
    valid(&value)
}
