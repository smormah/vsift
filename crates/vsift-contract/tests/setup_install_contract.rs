//! The `setup install` contract (P13 PR 4): its data validates against
//! `setup-install.schema.json` for every outcome, the frozen examples match,
//! every typed reason is published and nothing else is, and every failure's
//! remediation is fixed prose within the envelope's bound.

use std::{collections::BTreeSet, fs, io, path::PathBuf};

use serde_json::Value;
use vsift_application::{
    CompatibilitySmokeCheck, CompatibilitySmokeFailure, CompatibilitySmokeFailureReason,
    ComponentInstallFailure, ComponentInstallOutcome, ComponentInstallReport,
    DownloadFailureReason, InstallFailureReason, InstallStep, ManagedInstallReport, StageDisposal,
    StageRetentionReason, StageSweep, VersionRemovalReport, VersionRemovalStatus,
};
use vsift_contract::{
    CommandName, InstallSource, OperationResponse, SetupInstallResponse, TerminalEventResponse,
    setup_install_failure_summary,
};
use vsift_domain::{FailureCode, ManagedComponent};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const REVISION: &str = "ubuntu-24.04-x86_64-2026-09-22-r2";

const SMOKE_REASONS: [CompatibilitySmokeFailureReason; 13] = [
    CompatibilitySmokeFailureReason::MissingExecutable,
    CompatibilitySmokeFailureReason::WrongArchitecture,
    CompatibilitySmokeFailureReason::NotExecutable,
    CompatibilitySmokeFailureReason::BannerMismatch,
    CompatibilitySmokeFailureReason::OutputOverBound,
    CompatibilitySmokeFailureReason::DeadlineExceeded,
    CompatibilitySmokeFailureReason::UnexpectedExtraFile,
    CompatibilitySmokeFailureReason::ChangedContent,
    CompatibilitySmokeFailureReason::ProviderFailed,
    CompatibilitySmokeFailureReason::FixtureMismatch,
    CompatibilitySmokeFailureReason::Preparation,
    CompatibilitySmokeFailureReason::InvalidRequest,
    CompatibilitySmokeFailureReason::Cancelled,
];

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

fn component(
    component: ManagedComponent,
    outcome: ComponentInstallOutcome,
    stage: Option<StageDisposal>,
) -> ComponentInstallReport {
    let version = match component {
        ManagedComponent::MediaTools => "n9.0.1-11-ge47273f4d9-20260831",
        ManagedComponent::WhisperCli => "whisper.cpp-v1.9.2-ubuntu-x64",
        ManagedComponent::WhisperModel => "whisper-base-multilingual-80da2d8",
    };
    ComponentInstallReport {
        component,
        version: version.to_owned(),
        outcome,
        stage,
    }
}

fn activated_report() -> ManagedInstallReport {
    ManagedInstallReport {
        components: [
            ManagedComponent::MediaTools,
            ManagedComponent::WhisperCli,
            ManagedComponent::WhisperModel,
        ]
        .into_iter()
        .map(|name| {
            component(
                name,
                ComponentInstallOutcome::Activated,
                Some(StageDisposal::Discarded),
            )
        })
        .collect(),
    }
}

/// A rerun after the media tools activated, whose recognizer download then
/// failed: the example of a failed install.
fn failed_report() -> ManagedInstallReport {
    ManagedInstallReport {
        components: vec![
            component(
                ManagedComponent::MediaTools,
                ComponentInstallOutcome::AlreadyCurrent,
                None,
            ),
            component(
                ManagedComponent::WhisperCli,
                ComponentInstallOutcome::Failed(ComponentInstallFailure::at(
                    InstallStep::Download,
                    InstallFailureReason::Download(DownloadFailureReason::Offline),
                )),
                None,
            ),
            component(
                ManagedComponent::WhisperModel,
                ComponentInstallOutcome::Failed(ComponentInstallFailure {
                    step: None,
                    reason: InstallFailureReason::Blocked,
                }),
                None,
            ),
        ],
    }
}

fn failure_envelope(report: &ManagedInstallReport) -> Result<Value, Box<dyn std::error::Error>> {
    let (component, failure) = report.first_failure().ok_or("the report did not fail")?;
    let code = report.failure_code().ok_or("the report has no code")?;
    let data = serde_json::to_value(SetupInstallResponse::new(
        Some(REVISION.to_owned()),
        InstallSource::Publisher,
        report,
    ))?;
    Ok(serde_json::to_value(
        OperationResponse::failure_with_remediation(
            CommandName::SetupInstall.identifier(),
            code,
            setup_install_failure_summary(component, failure),
        )
        .with_failure_value(data),
    )?)
}

#[test]
fn a_complete_install_matches_the_frozen_example() -> TestResult {
    let data = SetupInstallResponse::new(
        Some(REVISION.to_owned()),
        InstallSource::Publisher,
        &activated_report(),
    )
    .with_cleanup(
        &StageSweep {
            removed: 1,
            retained: Vec::new(),
            interrupted_selections_removed: 0,
        },
        &[VersionRemovalReport {
            component: ManagedComponent::MediaTools,
            version: String::from("n8.1-2-g0123456789-20260601"),
            status: VersionRemovalStatus::Removed,
        }],
    );
    let response = serde_json::to_value(OperationResponse::complete(
        CommandName::SetupInstall.identifier(),
        &data,
    )?)?;
    validate("operation-response.schema.json", &response)?;
    validate("setup-install.schema.json", &response["data"])?;
    assert_eq!(response, load("examples/setup-install.json")?);
    Ok(())
}

#[test]
fn a_failed_install_keeps_its_components_beside_the_error() -> TestResult {
    let response = failure_envelope(&failed_report())?;
    validate("operation-response.schema.json", &response)?;
    validate("setup-install.schema.json", &response["data"])?;
    assert_eq!(response["error"]["code"], "DOWNLOAD_FAILED");
    assert_eq!(response, load("examples/setup-install.failed.json")?);
    let event = serde_json::to_value(TerminalEventResponse::new(
        OperationResponse::failure_with_remediation(
            CommandName::SetupInstall.identifier(),
            FailureCode::DownloadFailed,
            String::from("fixture"),
        )
        .with_failure_value(response["data"].clone()),
    ))?;
    validate("terminal-event.schema.json", &event)?;
    Ok(())
}

/// Every reason, step and smoke check a report can carry validates, with a
/// provable and an unprovable stage, and its remediation is bounded fixed
/// prose.
#[test]
fn every_failure_validates_and_has_a_bounded_remediation() -> TestResult {
    let mut reasons: Vec<(Option<InstallStep>, InstallFailureReason)> = vec![
        (
            Some(InstallStep::Import),
            InstallFailureReason::ArtifactMissing,
        ),
        (
            Some(InstallStep::Import),
            InstallFailureReason::ArtifactNotRegularFile,
        ),
        (
            Some(InstallStep::Download),
            InstallFailureReason::DigestMismatch,
        ),
        (
            Some(InstallStep::Import),
            InstallFailureReason::SizeMismatch,
        ),
        (
            Some(InstallStep::Stage),
            InstallFailureReason::ReviewMismatch,
        ),
        (Some(InstallStep::Activate), InstallFailureReason::Storage),
        (None, InstallFailureReason::Cancelled),
    ];
    reasons.extend(DownloadFailureReason::ALL.into_iter().map(|reason| {
        (
            Some(InstallStep::Download),
            InstallFailureReason::Download(reason),
        )
    }));
    for check in [
        CompatibilitySmokeCheck::Layout,
        CompatibilitySmokeCheck::Banner,
        CompatibilitySmokeCheck::MediaFixture,
        CompatibilitySmokeCheck::SpeechFixture,
        CompatibilitySmokeCheck::Recheck,
    ] {
        reasons.extend(SMOKE_REASONS.into_iter().map(|reason| {
            (
                Some(InstallStep::Smoke),
                InstallFailureReason::Smoke(CompatibilitySmokeFailure { check, reason }),
            )
        }));
    }
    for (step, reason) in reasons {
        for stage in [
            None,
            Some(StageDisposal::Discarded),
            Some(StageDisposal::Retained(
                StageRetentionReason::OwnershipUnproved,
            )),
            Some(StageDisposal::Retained(
                StageRetentionReason::UnexpectedContent,
            )),
            Some(StageDisposal::Retained(
                StageRetentionReason::StorageFailure,
            )),
        ] {
            let report = ManagedInstallReport {
                components: vec![
                    component(
                        ManagedComponent::MediaTools,
                        ComponentInstallOutcome::Failed(ComponentInstallFailure { step, reason }),
                        stage,
                    ),
                    component(
                        ManagedComponent::WhisperCli,
                        ComponentInstallOutcome::Failed(ComponentInstallFailure {
                            step: None,
                            reason: InstallFailureReason::Blocked,
                        }),
                        None,
                    ),
                ],
            };
            let response = failure_envelope(&report)?;
            validate("operation-response.schema.json", &response)?;
            validate("setup-install.schema.json", &response["data"])?;
            let summary = response["error"]["remediation"][0]["summary"]
                .as_str()
                .ok_or("no remediation")?;
            assert!(summary.len() <= 1024, "{reason:?}: {summary}");
            assert!(summary.contains("ffmpeg_ffprobe"), "{summary}");
        }
    }
    Ok(())
}

/// The schema's reason enum is exactly the identifiers the application
/// produces.
#[test]
fn every_published_reason_is_produced_and_nothing_else_is() -> TestResult {
    let schema = load("setup-install.schema.json")?;
    let published: BTreeSet<String> = schema["$defs"]["component"]["properties"]["reason"]["oneOf"]
        [1]["enum"]
        .as_array()
        .ok_or("reason enum missing")?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let mut produced: BTreeSet<String> = [
        InstallFailureReason::ArtifactMissing,
        InstallFailureReason::ArtifactNotRegularFile,
        InstallFailureReason::DigestMismatch,
        InstallFailureReason::SizeMismatch,
        InstallFailureReason::ReviewMismatch,
        InstallFailureReason::Storage,
        InstallFailureReason::Cancelled,
        InstallFailureReason::Blocked,
    ]
    .into_iter()
    .map(|reason| reason.identifier().to_owned())
    .collect();
    produced.extend(
        DownloadFailureReason::ALL
            .into_iter()
            .map(|reason| reason.identifier().to_owned()),
    );
    produced.extend(
        SMOKE_REASONS
            .into_iter()
            .map(|reason| reason.identifier().to_owned()),
    );
    assert_eq!(published, produced);
    Ok(())
}
