//! The freeze: what may not change between the first counted trial of a
//! batch and its last (P14, plan section 7, ADR 0024 decision D).
//!
//! At the candidate commit the skill, the grader, the scenarios (the
//! tuning, cold and hold-out sets), the trial settings and the corpus truth
//! are recorded by digest. A change to any of them after the first counted
//! trial of a batch voids the batch, and a failure is a finding, never a
//! reason to edit the grader. `freeze write` records the digests;
//! `freeze check` recomputes them and fails on any difference; `prepare`
//! stamps each trial with the freeze's own digest, so a record says which
//! frozen state it ran under.
//!
//! The cold baseline (batch 1) and the cold final round (batch 3) must be
//! comparable, so `freeze check --only grader,cold,settings,truth` checks
//! just the parts that must not move between them (the skill, the tuning
//! scenarios and the hold-outs are not part of a cold trial).
//!
//! The grader is its source, not a binary: the digest covers every file of
//! the crate's `src` folder and its `Cargo.toml`, so it follows what the
//! harness would do, whichever machine built it.

use std::{collections::BTreeMap, fmt::Write as _, path::Path};

use serde::{Deserialize, Serialize};

use crate::{
    error::{TrialError, read_json, write_json},
    skill::{directory_digest, file_digest, sha256_hex},
};

/// The harness crate, relative to the repository.
const CRATE: &str = "tools/vsift-agent-trials";

/// The component names `freeze` knows.
pub const COMPONENTS: [&str; 7] = [
    "skill",
    "grader",
    "scenarios",
    "cold",
    "holdout",
    "settings",
    "truth",
];

/// What was frozen.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Freeze {
    /// Record version, 1.
    pub schema_version: u32,
    /// The commit under test, as the operator names it.
    pub commit: String,
    /// The digest of each component.
    pub components: BTreeMap<String, String>,
    /// One digest over every component.
    pub freeze_sha256: String,
}

fn component(repository: &Path, name: &str) -> Result<String, TrialError> {
    let harness = repository.join(CRATE);
    match name {
        "skill" => directory_digest(&repository.join("skills").join("vsift")),
        "grader" => {
            let mut text = directory_digest(&harness.join("src"))?;
            text.push_str(&file_digest(&harness.join("Cargo.toml"))?);
            Ok(sha256_hex(text.as_bytes()))
        }
        "scenarios" | "cold" | "holdout" => directory_digest(&harness.join(name)),
        "settings" => {
            let mut text = file_digest(&harness.join("claude-trial-settings.json"))?;
            text.push_str(&file_digest(
                &harness.join("claude-cold-trial-settings.json"),
            )?);
            Ok(sha256_hex(text.as_bytes()))
        }
        "truth" => {
            let corpus = repository.join("fixtures").join("corpus");
            let mut text = file_digest(&corpus.join("manifest.json"))?;
            text.push_str(&file_digest(
                &corpus.join("generated").join("speech-provenance.json"),
            )?);
            Ok(sha256_hex(text.as_bytes()))
        }
        other => Err(TrialError::Refused(format!(
            "{other:?} is not a freeze component; they are {COMPONENTS:?}"
        ))),
    }
}

fn combined(components: &BTreeMap<String, String>) -> String {
    let text = components
        .iter()
        .fold(String::new(), |mut text, (name, digest)| {
            let _ = writeln!(text, "{name}={digest}");
            text
        });
    sha256_hex(text.as_bytes())
}

/// Computes the freeze of a repository.
///
/// # Errors
///
/// [`TrialError`] when a component cannot be read.
pub fn compute(repository: &Path, commit: &str) -> Result<Freeze, TrialError> {
    let mut components = BTreeMap::new();
    for name in COMPONENTS {
        components.insert(name.to_owned(), component(repository, name)?);
    }
    Ok(Freeze {
        schema_version: 1,
        freeze_sha256: combined(&components),
        commit: commit.to_owned(),
        components,
    })
}

/// Writes a repository's freeze to a file.
///
/// # Errors
///
/// [`TrialError`] when a component cannot be read or the file written.
pub fn write(repository: &Path, commit: &str, output: &Path) -> Result<Freeze, TrialError> {
    let freeze = compute(repository, commit)?;
    write_json(output, &freeze)?;
    Ok(freeze)
}

/// Reads a freeze file.
///
/// # Errors
///
/// [`TrialError`] when it is missing or malformed.
pub fn load(path: &Path) -> Result<Freeze, TrialError> {
    serde_json::from_value(read_json(path)?)
        .map_err(|error| TrialError::json(path.display().to_string(), error))
}

/// Every difference between a freeze file and the repository now; empty
/// when nothing moved. `only` limits the check to the named components.
///
/// # Errors
///
/// [`TrialError`] when the file or a component cannot be read, or `only`
/// names a component that does not exist.
pub fn check(
    repository: &Path,
    freeze_file: &Path,
    only: Option<&[String]>,
) -> Result<Vec<String>, TrialError> {
    let recorded = load(freeze_file)?;
    if combined(&recorded.components) != recorded.freeze_sha256 {
        return Ok(vec![
            "the freeze file's own digest does not match its components".to_owned(),
        ]);
    }
    let names: Vec<String> = match only {
        Some(names) => names.to_vec(),
        None => COMPONENTS.iter().map(|name| (*name).to_owned()).collect(),
    };
    let mut problems = Vec::new();
    for name in names {
        let now = component(repository, &name)?;
        match recorded.components.get(&name) {
            Some(then) if *then == now => {}
            Some(_) => problems.push(format!("{name} changed since the freeze")),
            None => problems.push(format!("{name} is not in the freeze file")),
        }
    }
    Ok(problems)
}
