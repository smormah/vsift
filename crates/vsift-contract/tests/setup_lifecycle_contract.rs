//! The `setup list`, `setup rollback`, `setup remove` and `setup repair`
//! contracts (P13 PR 6): every outcome validates against its schema, the
//! frozen examples match, every closed value the application produces is
//! published and nothing else is, repair's commands are fixed words and
//! managed keys, and every remediation is bounded fixed prose.
//!
//! Set `VSIFT_UPDATE_SCHEMA_EXAMPLES=1` to rewrite this file's frozen
//! examples after a deliberate change, then review the diff.

use std::{collections::BTreeSet, env, fs, io, path::PathBuf};

use serde_json::Value;
use vsift_application::{
    ComponentInventory, ManagedInventory, ManagedLifecycleRefusal, ManagedPlanAvailability,
    ManagedRemovalTarget, ManagedSelection, ManagedVersionFault, ManagedVersionRecord,
    ManagedVersionState, PreviousSelection, RemovalReport, RepairFindingKind, RepairFix,
    RollbackOutcome, SelectionRecord, SelectionStatus, StageRetentionReason, StageSweep,
    VersionRemovalReport, VersionRemovalStatus, diagnose_managed_store,
};
use vsift_contract::{
    CommandName, OperationResponse, SetupListResponse, SetupRemoveResponse, SetupRepairResponse,
    SetupRollbackResponse, TerminalEventResponse, managed_lifecycle_remediation,
    setup_remove_failure_summary,
};
use vsift_domain::{FailureCode, ManagedComponent, ManagedVersionKey};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const MEDIA: &str = "n9.0.2-22-g46d8f462ee-20261003";
const MEDIA_OLD: &str = "n8.1-2-g0123456789-20260601";
const CLI: &str = "whisper.cpp-v1.9.2-ubuntu-x64";
const MODEL: &str = "whisper-base-multilingual-80da2d8";

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

/// Compares `value` with a frozen example, or rewrites it on request.
fn frozen(example: &str, value: &Value) -> TestResult {
    let path = schema_root().join("examples").join(example);
    if env::var_os("VSIFT_UPDATE_SCHEMA_EXAMPLES").is_some() {
        fs::write(&path, format!("{}\n", serde_json::to_string_pretty(value)?))?;
        return Ok(());
    }
    assert_eq!(*value, load(&format!("examples/{example}"))?, "{example}");
    Ok(())
}

fn sha(seed: &str) -> String {
    let mut hex = String::new();
    for byte in seed.bytes().cycle().take(32) {
        hex.push('0');
        hex.push(char::from(b"0123456789abcdef"[usize::from(byte & 0x0f)]));
    }
    hex
}

fn verified(version: &str) -> ManagedVersionRecord {
    ManagedVersionRecord {
        version: version.to_owned(),
        state: ManagedVersionState::Verified {
            manifest_sha256: sha(version),
        },
    }
}

fn selected(version: &str, previous: Option<&str>) -> ManagedSelection {
    ManagedSelection::Recorded(SelectionRecord {
        version: version.to_owned(),
        manifest_sha256: sha(version),
        previous: previous.map(|previous| PreviousSelection {
            version: previous.to_owned(),
            manifest_sha256: sha(previous),
        }),
    })
}

/// A healthy store: the media tools with a previous version, the CLI and
/// the model once each.
fn healthy() -> ManagedInventory {
    let mut inventory = ManagedInventory::empty();
    inventory.components = vec![
        ComponentInventory {
            component: ManagedComponent::MediaTools,
            selection: selected(MEDIA, Some(MEDIA_OLD)),
            versions: vec![verified(MEDIA_OLD), verified(MEDIA)],
        },
        ComponentInventory {
            component: ManagedComponent::WhisperCli,
            selection: selected(CLI, None),
            versions: vec![verified(CLI)],
        },
        ComponentInventory {
            component: ManagedComponent::WhisperModel,
            selection: selected(MODEL, None),
            versions: vec![verified(MODEL)],
        },
    ];
    inventory
}

/// A store with every kind of problem.
fn troubled() -> ManagedInventory {
    let mut inventory = healthy();
    inventory.components[0].versions[1].state =
        ManagedVersionState::Unverified(ManagedVersionFault::ChangedContent);
    inventory.components[1].versions.push(ManagedVersionRecord {
        version: String::from("whisper.cpp-v1.9.1-ubuntu-x64"),
        state: ManagedVersionState::RemovalInterrupted,
    });
    inventory.components[1].versions.push(ManagedVersionRecord {
        version: String::from("whisper.cpp-v1.9.0-ubuntu-x64"),
        state: ManagedVersionState::Unverified(ManagedVersionFault::InvalidManifest),
    });
    inventory.components[2].selection = ManagedSelection::Unreadable;
    inventory.stale_stages = 2;
    inventory.retained_stages = vec![StageRetentionReason::UnexpectedContent];
    inventory.interrupted_selections = 1;
    inventory.unexpected_entries = 1;
    inventory
}

fn complete<T: serde::Serialize>(
    command: CommandName,
    data: &T,
) -> Result<Value, Box<dyn std::error::Error>> {
    let response = serde_json::to_value(OperationResponse::complete(command.identifier(), data)?)?;
    validate("operation-response.schema.json", &response)?;
    let event = serde_json::to_value(TerminalEventResponse::new(OperationResponse::complete(
        command.identifier(),
        data,
    )?))?;
    validate("terminal-event.schema.json", &event)?;
    Ok(response)
}

#[test]
fn setup_list_validates_for_every_store_and_matches_the_frozen_example() -> TestResult {
    let response = complete(
        CommandName::SetupList,
        &SetupListResponse::new(ManagedPlanAvailability::Qualified, Some(&healthy())),
    )?;
    validate("setup-list.schema.json", &response["data"])?;
    frozen("setup-list.json", &response)?;
    for (availability, inventory) in [
        (ManagedPlanAvailability::TargetUnavailable, None),
        (ManagedPlanAvailability::Qualified, Some(troubled())),
        (
            ManagedPlanAvailability::CatalogueExpired,
            Some(ManagedInventory::empty()),
        ),
    ] {
        let data = serde_json::to_value(SetupListResponse::new(availability, inventory.as_ref()))?;
        validate("setup-list.schema.json", &data)?;
        assert_eq!(data["components"].as_array().map(Vec::len), Some(3));
    }
    let absent = serde_json::to_value(SetupListResponse::new(
        ManagedPlanAvailability::TargetUnavailable,
        None,
    ))?;
    assert_eq!(absent["managed_folder"], "absent");
    assert_eq!(absent["managed_install"], "unavailable_target");
    Ok(())
}

#[test]
fn setup_rollback_matches_the_frozen_example() -> TestResult {
    let rolled_back = complete(
        CommandName::SetupRollback,
        &SetupRollbackResponse::new(&RollbackOutcome {
            component: ManagedComponent::MediaTools,
            selected: MEDIA_OLD.to_owned(),
            replaced: Some(MEDIA.to_owned()),
            changed: true,
        }),
    )?;
    validate("setup-rollback.schema.json", &rolled_back["data"])?;
    frozen("setup-rollback.json", &rolled_back)?;
    let unchanged = serde_json::to_value(SetupRollbackResponse::new(&RollbackOutcome {
        component: ManagedComponent::WhisperCli,
        selected: CLI.to_owned(),
        replaced: None,
        changed: false,
    }))?;
    validate("setup-rollback.schema.json", &unchanged)?;
    assert_eq!(unchanged["status"], "already_selected");
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "every removal status and its schema enum, checked in one place"
)]
fn setup_remove_validates_every_status_and_keeps_its_data_beside_a_failure() -> TestResult {
    let sweep = RemovalReport {
        deselected: None,
        versions: Vec::new(),
        stages: Some(StageSweep {
            removed: 2,
            retained: Vec::new(),
            interrupted_selections_removed: 1,
        }),
    };
    let response = complete(
        CommandName::SetupRemove,
        &SetupRemoveResponse::new(&ManagedRemovalTarget::StaleStages, &sweep),
    )?;
    validate("setup-remove.schema.json", &response["data"])?;
    frozen("setup-remove.json", &response)?;

    let component = ManagedRemovalTarget::Component(ManagedComponent::WhisperCli);
    let in_use = RemovalReport {
        deselected: Some(ManagedComponent::WhisperCli),
        versions: vec![
            VersionRemovalReport {
                component: ManagedComponent::WhisperCli,
                version: String::from("whisper.cpp-v1.9.1-ubuntu-x64"),
                status: VersionRemovalStatus::Removed,
            },
            VersionRemovalReport {
                component: ManagedComponent::WhisperCli,
                version: CLI.to_owned(),
                status: VersionRemovalStatus::InUse,
            },
        ],
        stages: None,
    };
    let code = in_use
        .failure_code()
        .ok_or("an in-use removal did not fail")?;
    assert_eq!(code, FailureCode::Busy);
    let failed = serde_json::to_value(
        OperationResponse::failure_with_remediation(
            CommandName::SetupRemove.identifier(),
            code,
            setup_remove_failure_summary(code).to_owned(),
        )
        .with_retry_after(30_000)
        .with_failure_value(serde_json::to_value(SetupRemoveResponse::new(
            &component, &in_use,
        ))?),
    )?;
    validate("operation-response.schema.json", &failed)?;
    validate("setup-remove.schema.json", &failed["data"])?;
    frozen("setup-remove.failed.json", &failed)?;

    let statuses = [
        VersionRemovalStatus::Removed,
        VersionRemovalStatus::AlreadyAbsent,
        VersionRemovalStatus::InUse,
        VersionRemovalStatus::Selected,
        VersionRemovalStatus::UnexpectedContent,
        VersionRemovalStatus::StorageFailure,
    ];
    for status in statuses {
        let report = RemovalReport {
            deselected: None,
            versions: vec![VersionRemovalReport {
                component: ManagedComponent::WhisperModel,
                version: MODEL.to_owned(),
                status,
            }],
            stages: None,
        };
        let target = ManagedRemovalTarget::Version {
            component: ManagedComponent::WhisperModel,
            version: ManagedVersionKey::parse(MODEL)?,
        };
        validate(
            "setup-remove.schema.json",
            &serde_json::to_value(SetupRemoveResponse::new(&target, &report))?,
        )?;
        if let Some(code) = report.failure_code() {
            assert!(setup_remove_failure_summary(code).len() <= 1024);
        }
    }
    let schema = load("setup-remove.schema.json")?;
    let published: BTreeSet<String> =
        schema["$defs"]["version_outcome"]["properties"]["status"]["enum"]
            .as_array()
            .ok_or("status enum missing")?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
    let produced: BTreeSet<String> = statuses
        .into_iter()
        .map(|status| status.identifier().to_owned())
        .collect();
    assert_eq!(published, produced);
    let retained = RemovalReport {
        deselected: None,
        versions: Vec::new(),
        stages: Some(StageSweep {
            removed: 0,
            retained: vec![
                StageRetentionReason::OwnershipUnproved,
                StageRetentionReason::UnexpectedContent,
                StageRetentionReason::UnexpectedContent,
                StageRetentionReason::StorageFailure,
            ],
            interrupted_selections_removed: 0,
        }),
    };
    assert_eq!(retained.failure_code(), Some(FailureCode::StorageIo));
    validate(
        "setup-remove.schema.json",
        &serde_json::to_value(SetupRemoveResponse::new(
            &ManagedRemovalTarget::StaleStages,
            &retained,
        ))?,
    )?;
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "every finding kind, fix and fault against the schemas, checked in one place"
)]
fn setup_repair_plans_every_finding_with_fixed_words_and_keys() -> TestResult {
    let plan = diagnose_managed_store(Some(&troubled()));
    let response = complete(
        CommandName::SetupRepair,
        &SetupRepairResponse::new(ManagedPlanAvailability::Qualified, &plan),
    )?;
    let data = &response["data"];
    validate("setup-repair.schema.json", data)?;
    frozen("setup-repair.json", &response)?;
    assert_eq!(data["status"], "needs_repair");
    let findings = data["findings"].as_array().ok_or("no findings")?;
    for finding in findings {
        let summary = finding["summary"].as_str().ok_or("no summary")?;
        assert!(summary.len() <= 1024, "{summary}");
        assert!(!summary.contains('/') || summary.contains("vsift/managed-v1"));
        if let Some(arguments) = finding["command"]["arguments"].as_array() {
            assert_eq!(finding["command"]["executable"], "vsift");
            for argument in arguments.iter().filter_map(Value::as_str) {
                assert!(
                    argument.starts_with("--") || vsift_domain::is_canonical_managed_key(argument),
                    "{argument}"
                );
            }
        }
    }
    for (inventory, status) in [(None, "nothing_installed"), (Some(healthy()), "healthy")] {
        let plan = diagnose_managed_store(inventory.as_ref());
        let data = serde_json::to_value(SetupRepairResponse::new(
            ManagedPlanAvailability::TargetUnavailable,
            &plan,
        ))?;
        validate("setup-repair.schema.json", &data)?;
        assert_eq!(data["status"], status);
    }
    // Every kind and fix the application can produce is published.
    let schema = load("setup-repair.schema.json")?;
    let enumerated = |member: &str| -> Result<BTreeSet<String>, Box<dyn std::error::Error>> {
        Ok(schema["$defs"]["finding"]["properties"][member]["enum"]
            .as_array()
            .ok_or("enum missing")?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect())
    };
    let kinds: BTreeSet<String> = [
        RepairFindingKind::SelectionUnreadable,
        RepairFindingKind::SelectedVersionUnverified,
        RepairFindingKind::VersionUnverified,
        RepairFindingKind::RemovalInterrupted,
        RepairFindingKind::StaleStages,
        RepairFindingKind::StagesNeedManualRemoval,
        RepairFindingKind::InterruptedSelection,
        RepairFindingKind::UnexpectedEntries,
    ]
    .into_iter()
    .map(|kind| kind.identifier().to_owned())
    .collect();
    assert_eq!(enumerated("kind")?, kinds);
    let component = ManagedComponent::WhisperCli;
    let fixes: BTreeSet<String> = [
        RepairFix::RollBack(component),
        RepairFix::RollBackTo(component, CLI.to_owned()),
        RepairFix::RemoveVersion(component, CLI.to_owned()),
        RepairFix::RemoveComponentAndReinstall(component),
        RepairFix::RemoveStaleStages,
        RepairFix::Manual,
    ]
    .iter()
    .map(|fix| fix.identifier().to_owned())
    .collect();
    assert_eq!(enumerated("fix")?, fixes);
    let faults: BTreeSet<String> = [
        ManagedVersionFault::MissingManifest,
        ManagedVersionFault::InvalidManifest,
        ManagedVersionFault::ChangedContent,
        ManagedVersionFault::UnexpectedEntry,
        ManagedVersionFault::Unreadable,
    ]
    .into_iter()
    .map(|fault| fault.identifier().to_owned())
    .collect();
    let list_schema = load("setup-list.schema.json")?;
    let listed: BTreeSet<String> = list_schema["$defs"]["version"]["properties"]["fault"]["oneOf"]
        [1]["enum"]
        .as_array()
        .ok_or("fault enum missing")?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    assert_eq!(listed, faults);
    let selections: BTreeSet<String> = [
        SelectionStatus::None,
        SelectionStatus::Verified,
        SelectionStatus::Unverified,
        SelectionStatus::Unreadable,
    ]
    .into_iter()
    .map(|status| status.identifier().to_owned())
    .collect();
    let published: BTreeSet<String> = list_schema["$defs"]["component"]["properties"]["selection"]
        ["enum"]
        .as_array()
        .ok_or("selection enum missing")?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    assert_eq!(published, selections);
    Ok(())
}

#[test]
fn every_refusal_has_a_bounded_remediation_and_a_known_code() -> TestResult {
    for refusal in [
        ManagedLifecycleRefusal::NotInstalled,
        ManagedLifecycleRefusal::VersionNotInstalled,
        ManagedLifecycleRefusal::NoPreviousVersion,
        ManagedLifecycleRefusal::VersionUnverified,
        ManagedLifecycleRefusal::VersionSelected,
        ManagedLifecycleRefusal::StoreUnsafe,
        ManagedLifecycleRefusal::StoreIo,
    ] {
        let (summary, arguments) = managed_lifecycle_remediation(refusal);
        let response = serde_json::to_value(OperationResponse::failure_with_suggested_command(
            CommandName::SetupRollback.identifier(),
            refusal.failure_code(),
            summary.to_owned(),
            arguments,
        ))?;
        validate("operation-response.schema.json", &response)?;
        assert!(summary.len() <= 1024);
        assert!(matches!(
            refusal.failure_code(),
            FailureCode::InvalidArgument | FailureCode::IntegrityFailure | FailureCode::StorageIo
        ));
    }
    Ok(())
}
