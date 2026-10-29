//! Installing the agent skill into AI coding clients' folders, and reporting
//! what is there (ADR 0038 section 3): the engine's facade over the
//! application's use case and the filesystem skill folders.

use std::{fmt, path::PathBuf};

use vsift_application::{
    SkillFolderError, SkillInstallOutcome, install_skill, skill_folder_states,
};
use vsift_contract::{
    SkillInstallData, SkillInstallFolderData, SkillInstallFolderOutcome, SkillStatusData,
    shipped_skill, skill_digest,
};
use vsift_domain::{AgentClient, FailureCode, ReplacePolicy, SkillBundle, SkillLayout, SkillScope};
use vsift_infrastructure::{FilesystemSkillFolders, SkillBaseError};

use crate::engine::Engine;

/// Whose skill folders a request is about.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SkillTarget {
    /// The person's own, below their home folder.
    User,
    /// One project's, below the folder named; it must already exist.
    Project(PathBuf),
}

impl SkillTarget {
    const fn scope(&self) -> SkillScope {
        match self {
            Self::User => SkillScope::User,
            Self::Project(_) => SkillScope::Project,
        }
    }
}

/// Why a skill request could not run, or stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillError {
    /// No absolute home folder is known.
    HomeUnknown,
    /// `--project` does not name an existing folder.
    ProjectNotFolder,
    /// A folder could not be read or written; the failed folder holds what it
    /// held before.
    Folder(SkillFolderError),
    /// The embedded skill is not a skill: a defect a test keeps from shipping.
    Embedded,
}

impl SkillError {
    /// The public failure code: the caller's input for the first two, the
    /// disk for a folder that could not be written, a defect otherwise.
    #[must_use]
    pub const fn failure_code(self) -> FailureCode {
        match self {
            Self::HomeUnknown | Self::ProjectNotFolder => FailureCode::InvalidArgument,
            Self::Folder(SkillFolderError::Io) => FailureCode::StorageIo,
            Self::Folder(SkillFolderError::VerificationFailed) => FailureCode::IntegrityFailure,
            Self::Embedded => FailureCode::Internal,
        }
    }
}

impl fmt::Display for SkillError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeUnknown => formatter.write_str("no home folder is known"),
            Self::ProjectNotFolder => formatter.write_str("the project is not an existing folder"),
            Self::Folder(error) => error.fmt(formatter),
            Self::Embedded => formatter.write_str("the embedded skill is not a skill"),
        }
    }
}

impl std::error::Error for SkillError {}

fn folders(target: &SkillTarget) -> Result<(FilesystemSkillFolders, SkillBundle), SkillError> {
    let bundle = shipped_skill().map_err(|_| SkillError::Embedded)?;
    let folders = match target {
        SkillTarget::User => FilesystemSkillFolders::for_user(bundle.clone()),
        SkillTarget::Project(project) => {
            FilesystemSkillFolders::for_project(project, bundle.clone())
        }
    }
    .map_err(|error| match error {
        SkillBaseError::HomeUnknown => SkillError::HomeUnknown,
        SkillBaseError::ProjectNotFolder => SkillError::ProjectNotFolder,
    })?;
    Ok((folders, bundle))
}

impl Engine {
    /// Copies the skill this binary ships into the folder each of `clients`
    /// reads, once per folder, under `policy`. A folder that holds something
    /// else is left as it was and its outcome says why
    /// ([`SkillInstallData::any_refused`]).
    ///
    /// # Errors
    ///
    /// [`SkillError`] when the base folder is unknown or a folder cannot be
    /// written.
    pub fn install_skill(
        &self,
        clients: &[AgentClient],
        target: &SkillTarget,
        policy: ReplacePolicy,
    ) -> Result<SkillInstallData, SkillError> {
        let (folders, bundle) = folders(target)?;
        let layouts = SkillLayout::of_clients(clients);
        let outcomes = install_skill(&folders, &layouts, policy).map_err(SkillError::Folder)?;
        let folders = outcomes
            .into_iter()
            .map(|folder| {
                let outcome = match folder.outcome {
                    SkillInstallOutcome::Installed => SkillInstallFolderOutcome::Installed,
                    SkillInstallOutcome::Unchanged => SkillInstallFolderOutcome::Unchanged,
                    SkillInstallOutcome::Replaced(state) => {
                        SkillInstallFolderOutcome::Replaced(state)
                    }
                    SkillInstallOutcome::Refused(reason) => {
                        SkillInstallFolderOutcome::Refused(reason)
                    }
                };
                SkillInstallFolderData::new(folder.layout, outcome)
            })
            .collect();
        Ok(SkillInstallData::new(
            target.scope(),
            skill_digest(&bundle),
            folders,
        ))
    }

    /// What is at each skill folder of `target`, read only and without
    /// following a link.
    ///
    /// # Errors
    ///
    /// [`SkillError`] when the base folder is unknown or a folder cannot be
    /// read.
    pub fn skill_status(&self, target: &SkillTarget) -> Result<SkillStatusData, SkillError> {
        let (folders, bundle) = folders(target)?;
        let states =
            skill_folder_states(&folders, &SkillLayout::ALL).map_err(SkillError::Folder)?;
        Ok(SkillStatusData::new(target.scope(), &bundle, &states))
    }
}
