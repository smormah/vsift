//! `vsift skill install`, `skill status` and `skill show` (ADR 0038 section
//! 3): the CLI composes the engine's facade and presents its results.

use vsift::{AgentClient, Engine, FailureCode, ReplacePolicy, SkillError, SkillTarget};
use vsift_contract::{
    CommandName, OperationResponse, SKILL_HOME_UNKNOWN_REMEDIATION,
    SKILL_INSTALL_REFUSED_REMEDIATION, SKILL_PROJECT_REMEDIATION, SKILL_SHOW_ENTRY_REMEDIATION,
    SkillShowData, shipped_skill,
};

use crate::{
    CommandFailure,
    command::{SkillInstallArguments, SkillShowArguments, SkillStatusArguments},
    complete,
};

fn target(project: Option<&std::path::Path>) -> SkillTarget {
    project.map_or(SkillTarget::User, |project| {
        SkillTarget::Project(project.to_path_buf())
    })
}

fn failure(error: SkillError) -> CommandFailure {
    match error {
        SkillError::HomeUnknown => CommandFailure::with_remediation(
            error.failure_code(),
            SKILL_HOME_UNKNOWN_REMEDIATION.to_owned(),
        ),
        SkillError::ProjectNotFolder => CommandFailure::with_remediation(
            error.failure_code(),
            SKILL_PROJECT_REMEDIATION.to_owned(),
        ),
        SkillError::Folder(_) | SkillError::Embedded => CommandFailure::from(error.failure_code()),
    }
}

/// Installs the skill for the clients `arguments` name. A folder left as it
/// was makes the result `INVALID_ARGUMENT`, with every folder's outcome kept
/// as the failure's data and a remediation naming `--replace` (ADR 0038 D3).
pub(crate) fn install(
    engine: &Engine,
    arguments: &SkillInstallArguments,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let clients: Vec<AgentClient> = arguments.clients.iter().copied().map(Into::into).collect();
    let policy = if arguments.replace {
        ReplacePolicy::Replace
    } else {
        ReplacePolicy::Keep
    };
    let data = engine
        .install_skill(&clients, &target(arguments.project.as_deref()), policy)
        .map_err(failure)?;
    if data.any_refused() {
        let value = serde_json::to_value(&data).map_err(|_| FailureCode::Internal)?;
        return Err(CommandFailure::with_remediation(
            FailureCode::InvalidArgument,
            SKILL_INSTALL_REFUSED_REMEDIATION.to_owned(),
        )
        .with_data(value));
    }
    Ok(complete(CommandName::SkillInstall, &data)?)
}

/// Reports what is at each skill folder, read only.
pub(crate) fn status(
    engine: &Engine,
    arguments: &SkillStatusArguments,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let data = engine
        .skill_status(&target(arguments.project.as_deref()))
        .map_err(failure)?;
    Ok(complete(CommandName::SkillStatus, &data)?)
}

/// Prints one text file of the shipped skill, named as the manifest names
/// it; anything else (a path, a folder, the image) is refused.
pub(crate) fn show(
    arguments: &SkillShowArguments,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let bundle = shipped_skill().map_err(|_| FailureCode::Internal)?;
    let data = SkillShowData::of(&bundle, &arguments.entry).ok_or_else(|| {
        CommandFailure::with_remediation(
            FailureCode::InvalidArgument,
            SKILL_SHOW_ENTRY_REMEDIATION.to_owned(),
        )
    })?;
    Ok(complete(CommandName::SkillShow, &data)?)
}
