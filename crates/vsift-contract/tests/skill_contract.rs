//! The `skill.install`, `skill.status` and `skill.show` data (ADR 0038
//! section 3) against their published schemas and frozen examples. Set
//! `VSIFT_REGENERATE_CONTRACT_EXAMPLES=1` to rewrite the examples after an
//! intended change, and review the diff.

use std::{env, fs, io, path::PathBuf};

use serde::Serialize;
use serde_json::{Value, json};
use vsift_contract::{
    CommandName, OperationResponse, SkillInstallData, SkillInstallFolderData,
    SkillInstallFolderOutcome, SkillShowData, SkillStatusData, shipped_skill,
};
use vsift_domain::{SkillFolderState, SkillInstallRefusal, SkillLayout, SkillScope};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A digest of the right shape; the examples do not pin the skill's bytes,
/// which change with every skill edit.
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

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

/// The data against its schema, and the whole response against the
/// envelope and the frozen example `examples/<example>`.
fn check_example<T: Serialize>(
    command: CommandName,
    schema: &str,
    example: &str,
    data: &T,
) -> TestResult {
    validate(schema, &serde_json::to_value(data)?)?;
    let response = serde_json::to_value(OperationResponse::complete(command.identifier(), data)?)?;
    validate("operation-response.schema.json", &response)?;
    let path = schema_root().join("examples").join(example);
    if regenerate() {
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&response)?),
        )?;
    }
    assert_eq!(response, load(&format!("examples/{example}"))?);
    Ok(())
}

#[test]
fn an_install_matches_its_frozen_example() -> TestResult {
    let data = SkillInstallData::new(
        SkillScope::User,
        DIGEST.to_owned(),
        vec![
            SkillInstallFolderData::new(SkillLayout::Claude, SkillInstallFolderOutcome::Installed),
            SkillInstallFolderData::new(
                SkillLayout::Agents,
                SkillInstallFolderOutcome::Replaced(SkillFolderState::Different),
            ),
        ],
    );
    assert!(!data.any_refused());
    check_example(
        CommandName::SkillInstall,
        "skill-install-data.schema.json",
        "skill-install.json",
        &data,
    )
}

#[test]
fn every_install_outcome_validates_and_a_refusal_is_marked() -> TestResult {
    let outcomes = [
        SkillInstallFolderOutcome::Installed,
        SkillInstallFolderOutcome::Unchanged,
        SkillInstallFolderOutcome::Replaced(SkillFolderState::Different),
        SkillInstallFolderOutcome::Replaced(SkillFolderState::Link),
        SkillInstallFolderOutcome::Refused(SkillInstallRefusal::DifferentWithoutReplace),
        SkillInstallFolderOutcome::Refused(SkillInstallRefusal::LinkWithoutReplace),
        SkillInstallFolderOutcome::Refused(SkillInstallRefusal::NotVsift),
        SkillInstallFolderOutcome::Refused(SkillInstallRefusal::ParentNotFolder),
    ];
    for outcome in outcomes {
        let data = SkillInstallData::new(
            SkillScope::Project,
            DIGEST.to_owned(),
            vec![SkillInstallFolderData::new(SkillLayout::Agents, outcome)],
        );
        let value = serde_json::to_value(&data)?;
        validate("skill-install-data.schema.json", &value)?;
        assert_eq!(
            data.any_refused(),
            matches!(outcome, SkillInstallFolderOutcome::Refused(_)),
            "{outcome:?}"
        );
    }
    let mut broken = serde_json::to_value(SkillInstallData::new(
        SkillScope::User,
        DIGEST.to_owned(),
        vec![SkillInstallFolderData::new(
            SkillLayout::Claude,
            SkillInstallFolderOutcome::Installed,
        )],
    ))?;
    broken["folders"][0]["refusal"] = json!("not_vsift");
    assert!(validate("skill-install-data.schema.json", &broken).is_err());
    Ok(())
}

#[test]
fn a_status_matches_its_frozen_example() -> TestResult {
    let bundle = shipped_skill()?;
    let data = SkillStatusData::new(
        SkillScope::User,
        &bundle,
        &[
            (SkillLayout::Claude, SkillFolderState::Current),
            (SkillLayout::Agents, SkillFolderState::Absent),
        ],
    );
    let mut value = serde_json::to_value(&data)?;
    validate("skill-status-data.schema.json", &value)?;
    // The example pins the shape, not this release's digest and file list.
    value["skill_digest"] = json!(DIGEST);
    value["files"] = json!(["SKILL.md", "references/commands.md"]);
    check_example(
        CommandName::SkillStatus,
        "skill-status-data.schema.json",
        "skill-status.json",
        &value,
    )
}

#[test]
fn a_show_matches_its_frozen_example() -> TestResult {
    let bundle = shipped_skill()?;
    let data = SkillShowData::of(&bundle, "references/commands.md").ok_or("commands.md")?;
    validate("skill-show-data.schema.json", &serde_json::to_value(&data)?)?;
    let example =
        json!({"entry": "references/budgets.md", "text": "# Budgets\n\nThe compact profile ...\n"});
    check_example(
        CommandName::SkillShow,
        "skill-show-data.schema.json",
        "skill-show.json",
        &example,
    )
}
