//! Installing the agent skill into the folders AI coding clients read, and
//! reporting what is there (ADR 0038 section 3).
//!
//! The decision of what to do at each folder is the domain's
//! ([`install_step`]); this use case applies it through the
//! [`SkillFolders`] port, which owns the disk. One folder is written for every
//! layout the named clients read, so clients that share a folder have it
//! written once.

use std::fmt;

use vsift_domain::{
    ReplacePolicy, SkillFolderState, SkillInstallRefusal, SkillInstallStep, SkillLayout,
    install_step,
};

/// Why a skill folder could not be read or written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillFolderError {
    /// The disk refused a read or a write; whatever was at the folder before
    /// is there again.
    Io,
    /// A written file did not read back as the skill's bytes; nothing was
    /// moved into place.
    VerificationFailed,
}

impl fmt::Display for SkillFolderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Io => "a skill folder could not be read or written",
            Self::VerificationFailed => "a written skill file did not verify",
        })
    }
}

impl std::error::Error for SkillFolderError {}

/// The skill folders below one home or project folder.
///
/// An implementation holds the skill's bytes and the base folder. Every step
/// is all or nothing for its folder: a failed step leaves the folder as it
/// was, and no step follows a link.
pub trait SkillFolders {
    /// What is at the folder of `layout`, read without following a link.
    ///
    /// # Errors
    ///
    /// [`SkillFolderError::Io`] when it cannot be read.
    fn inspect(&self, layout: SkillLayout) -> Result<SkillFolderState, SkillFolderError>;

    /// Writes the skill where nothing is, by staging it beside the folder,
    /// verifying every file and moving it into place.
    ///
    /// # Errors
    ///
    /// [`SkillFolderError`]; nothing is left at the folder's place.
    fn create(&self, layout: SkillLayout) -> Result<(), SkillFolderError>;

    /// Swaps the skill in for what is there: stage and verify, move the old
    /// folder (or link) aside under a name `VSift` made, move the staged one
    /// into place, and only then remove the old one; on any failure the old
    /// one is put back.
    ///
    /// # Errors
    ///
    /// [`SkillFolderError`]; the folder holds what it held before.
    fn swap(&self, layout: SkillLayout) -> Result<(), SkillFolderError>;
}

/// What an install did at one folder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillInstallOutcome {
    /// The skill was written where nothing was.
    Installed,
    /// The same bytes were already there; nothing was written.
    Unchanged,
    /// The skill replaced what was there (`--replace`), which is named.
    Replaced(SkillFolderState),
    /// Nothing was changed, for the reason named.
    Refused(SkillInstallRefusal),
}

/// One folder's outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SkillFolderOutcome {
    /// The folder.
    pub layout: SkillLayout,
    /// What happened there.
    pub outcome: SkillInstallOutcome,
}

/// Installs the skill into the folder of each of `layouts`, in order, under
/// `policy`. A refusal at one folder does not stop the others.
///
/// # Errors
///
/// The first [`SkillFolderError`]; folders before it keep their outcome on
/// the disk, the failed folder holds what it held, and later folders are not
/// touched.
pub fn install_skill(
    folders: &impl SkillFolders,
    layouts: &[SkillLayout],
    policy: ReplacePolicy,
) -> Result<Vec<SkillFolderOutcome>, SkillFolderError> {
    let mut outcomes = Vec::with_capacity(layouts.len());
    for &layout in layouts {
        let state = folders.inspect(layout)?;
        let outcome = match install_step(state, policy) {
            SkillInstallStep::Create => {
                folders.create(layout)?;
                SkillInstallOutcome::Installed
            }
            SkillInstallStep::KeepCurrent => SkillInstallOutcome::Unchanged,
            SkillInstallStep::Swap => {
                folders.swap(layout)?;
                SkillInstallOutcome::Replaced(state)
            }
            SkillInstallStep::Refuse(reason) => SkillInstallOutcome::Refused(reason),
        };
        outcomes.push(SkillFolderOutcome { layout, outcome });
    }
    Ok(outcomes)
}

/// What is at the folder of each of `layouts`, read only.
///
/// # Errors
///
/// [`SkillFolderError::Io`] when a folder cannot be read.
pub fn skill_folder_states(
    folders: &impl SkillFolders,
    layouts: &[SkillLayout],
) -> Result<Vec<(SkillLayout, SkillFolderState)>, SkillFolderError> {
    layouts
        .iter()
        .map(|&layout| folders.inspect(layout).map(|state| (layout, state)))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::BTreeMap};

    use vsift_domain::{ReplacePolicy, SkillFolderState, SkillInstallRefusal, SkillLayout};

    use super::{
        SkillFolderError, SkillFolderOutcome, SkillFolders, SkillInstallOutcome, install_skill,
    };

    /// Folders in memory: each holds a state, and records the steps taken.
    struct Fake {
        states: RefCell<BTreeMap<SkillLayout, SkillFolderState>>,
        steps: RefCell<Vec<(&'static str, SkillLayout)>>,
        fail_create: bool,
    }

    impl Fake {
        fn new(states: &[(SkillLayout, SkillFolderState)], fail_create: bool) -> Self {
            Self {
                states: RefCell::new(states.iter().copied().collect()),
                steps: RefCell::new(Vec::new()),
                fail_create,
            }
        }
    }

    impl SkillFolders for Fake {
        fn inspect(&self, layout: SkillLayout) -> Result<SkillFolderState, SkillFolderError> {
            Ok(self
                .states
                .borrow()
                .get(&layout)
                .copied()
                .unwrap_or(SkillFolderState::Absent))
        }

        fn create(&self, layout: SkillLayout) -> Result<(), SkillFolderError> {
            self.steps.borrow_mut().push(("create", layout));
            if self.fail_create {
                return Err(SkillFolderError::Io);
            }
            self.states
                .borrow_mut()
                .insert(layout, SkillFolderState::Current);
            Ok(())
        }

        fn swap(&self, layout: SkillLayout) -> Result<(), SkillFolderError> {
            self.steps.borrow_mut().push(("swap", layout));
            self.states
                .borrow_mut()
                .insert(layout, SkillFolderState::Current);
            Ok(())
        }
    }

    #[test]
    fn each_folder_gets_its_own_outcome_and_a_refusal_stops_nothing_else()
    -> Result<(), SkillFolderError> {
        let fake = Fake::new(&[(SkillLayout::Claude, SkillFolderState::Different)], false);
        let both = [SkillLayout::Claude, SkillLayout::Agents];
        let outcomes = install_skill(&fake, &both, ReplacePolicy::Keep)?;
        assert_eq!(
            outcomes,
            [
                SkillFolderOutcome {
                    layout: SkillLayout::Claude,
                    outcome: SkillInstallOutcome::Refused(
                        SkillInstallRefusal::DifferentWithoutReplace
                    ),
                },
                SkillFolderOutcome {
                    layout: SkillLayout::Agents,
                    outcome: SkillInstallOutcome::Installed,
                },
            ]
        );
        assert_eq!(*fake.steps.borrow(), [("create", SkillLayout::Agents)]);

        let again = install_skill(&fake, &both, ReplacePolicy::Replace)?;
        assert_eq!(
            again[0].outcome,
            SkillInstallOutcome::Replaced(SkillFolderState::Different)
        );
        assert_eq!(again[1].outcome, SkillInstallOutcome::Unchanged);
        Ok(())
    }

    #[test]
    fn a_failed_write_stops_before_the_next_folder() {
        let fake = Fake::new(&[], true);
        let both = [SkillLayout::Claude, SkillLayout::Agents];
        let result = install_skill(&fake, &both, ReplacePolicy::Keep);
        assert_eq!(result, Err(SkillFolderError::Io));
        assert_eq!(*fake.steps.borrow(), [("create", SkillLayout::Claude)]);
    }
}
