//! The value freeze: what may not change between the first counted run of
//! the value measurement and its last (planner's ruling 12.1, design
//! section 8).
//!
//! It is the P14 freeze's idea applied to this mode's own inputs: the task
//! files, the raw guide, the one settings file every arm gets, the grader
//! (the crate's source, as the P14 freeze reads it), the public method note,
//! the skill and the corpus truth. `value freeze write` records their
//! digests and one digest over them; `value freeze check` recomputes them
//! and names every difference. The hosted workflow refuses a counted
//! dispatch unless the committed freeze still holds, and `value prepare`
//! stamps each trial with the freeze file's digest, so every record says
//! which frozen state it ran under.
//!
//! The P14 freeze is separate and unchanged in what it covers.

use std::{collections::BTreeMap, path::Path};

use super::prepare::{METHOD_NOTE, VALUE_SETTINGS};
use crate::{
    error::TrialError,
    freeze::{Freeze, combined, component, load},
    skill::{directory_digest, file_digest},
};

/// The value task files, relative to the repository.
pub const VALUE_TASKS: &str = "tools/vsift-agent-trials/value/tasks";

/// The frozen raw guide's folder, relative to the repository.
pub const RAW_GUIDE: &str = "tools/vsift-agent-trials/value/raw-guide";

/// The component names of the value freeze.
pub const VALUE_COMPONENTS: [&str; 7] = [
    "tasks",
    "raw_guide",
    "settings",
    "grader",
    "method",
    "skill",
    "truth",
];

fn value_component(repository: &Path, name: &str) -> Result<String, TrialError> {
    match name {
        "tasks" => directory_digest(&repository.join(VALUE_TASKS)),
        "raw_guide" => {
            let guide = repository.join(RAW_GUIDE);
            if !guide.join("SKILL.md").is_file() {
                return Err(TrialError::Refused(format!(
                    "the raw guide is not frozen yet: {RAW_GUIDE}/SKILL.md is missing"
                )));
            }
            directory_digest(&guide)
        }
        "settings" => file_digest(&repository.join(VALUE_SETTINGS)),
        "method" => file_digest(&repository.join(METHOD_NOTE)),
        "grader" | "skill" | "truth" => component(repository, name),
        other => Err(TrialError::Refused(format!(
            "{other:?} is not a value freeze component; they are {VALUE_COMPONENTS:?}"
        ))),
    }
}

/// Computes the value freeze of a repository.
///
/// # Errors
///
/// [`TrialError`] when a component cannot be read, including a raw guide
/// that is not in the repository yet.
pub fn compute(repository: &Path, commit: &str) -> Result<Freeze, TrialError> {
    let mut components = BTreeMap::new();
    for name in VALUE_COMPONENTS {
        components.insert(name.to_owned(), value_component(repository, name)?);
    }
    Ok(Freeze {
        schema_version: 1,
        freeze_sha256: combined(&components),
        commit: commit.to_owned(),
        components,
    })
}

/// Writes a repository's value freeze to a file.
///
/// # Errors
///
/// [`TrialError`] when a component cannot be read or the file written.
pub fn write(repository: &Path, commit: &str, output: &Path) -> Result<Freeze, TrialError> {
    let freeze = compute(repository, commit)?;
    crate::error::write_json(output, &freeze)?;
    Ok(freeze)
}

/// Every difference between a value freeze file and the repository now;
/// empty when nothing moved.
///
/// # Errors
///
/// [`TrialError`] when the file is missing or malformed, or a component
/// cannot be read.
pub fn check(repository: &Path, freeze_file: &Path) -> Result<Vec<String>, TrialError> {
    let recorded = load(freeze_file)?;
    if combined(&recorded.components) != recorded.freeze_sha256 {
        return Ok(vec![
            "the freeze file's own digest does not match its components".to_owned(),
        ]);
    }
    let mut problems = Vec::new();
    for name in VALUE_COMPONENTS {
        let now = value_component(repository, name)?;
        match recorded.components.get(name) {
            Some(then) if *then == now => {}
            Some(_) => problems.push(format!("{name} changed since the value freeze")),
            None => problems.push(format!("{name} is not in the value freeze file")),
        }
    }
    for name in recorded.components.keys() {
        if !VALUE_COMPONENTS.contains(&name.as_str()) {
            problems.push(format!("{name} is not a value freeze component"));
        }
    }
    Ok(problems)
}
