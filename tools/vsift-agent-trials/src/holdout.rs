//! The hold-out scenarios (P14, plan section 7).
//!
//! The skill, the grader and the help text were tuned against the scenarios
//! of `tools/vsift-agent-trials/scenarios/` (P12) and, for the help text,
//! against the cold scenarios of `tools/vsift-agent-trials/cold/`. A
//! **hold-out** is a scenario nothing was tuned on: one question per
//! transcript path (a supplied transcript; local speech recognition), with an
//! event no earlier round asked about and truth read from the corpus
//! manifest only. A hold-out that fails is a finding about the skill, never
//! an edit to the grader or the scenario.
//!
//! The separation is mechanical, so it cannot erode by accident:
//!
//! - the hold-outs live in their own folder, `tools/vsift-agent-trials/
//!   holdout/`, which no tuning test reads, and are listed in `INDEX.json`
//!   with the SHA-256 of each file: editing one trips the check until the
//!   index is changed in the open, in the same diff;
//! - no hold-out names an event (as a truth event, an expectation or a fact
//!   to hold back) that any tuning or cold scenario names, and no hold-out
//!   id is a tuning id;
//! - both transcript paths are covered, and each entry says which path its
//!   scenario really takes.
//!
//! This proves the files are separate and unchanged, not that nobody looked
//! at the hold-outs while tuning: a maintainer who reads them has seen them
//! (known limit L-119).

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    error::{TrialError, read_json},
    scenario::{ColdUsefulness, Expectation, Scenario},
    skill::{file_digest, files_below},
};

/// The index, relative to the repository.
pub const INDEX_PATH: &str = "tools/vsift-agent-trials/holdout/INDEX.json";

/// The transcript path a hold-out exercises.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptPath {
    /// The user supplies a transcript sidecar.
    SuppliedTranscript,
    /// No transcript: local speech recognition.
    LocalAsr,
}

/// One hold-out.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HoldoutEntry {
    /// The scenario's identifier (its file's stem).
    pub id: String,
    /// The transcript path it exercises.
    pub path: TranscriptPath,
    /// The file name in the hold-out folder.
    pub file: String,
    /// SHA-256 of the file when it was frozen.
    pub sha256: String,
}

/// `INDEX.json`.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HoldoutIndex {
    /// Record version, 1.
    pub schema_version: u32,
    /// The date the files were frozen, written before the first counted run.
    pub frozen: String,
    /// The rules, in words, for a reader of the file.
    pub rules: Vec<String>,
    /// The hold-outs.
    pub entries: Vec<HoldoutEntry>,
}

impl HoldoutIndex {
    /// Reads the index of a repository, or `None` when it has none.
    ///
    /// # Errors
    ///
    /// [`TrialError`] when the file exists and is malformed.
    pub fn load(repository: &Path) -> Result<Option<Self>, TrialError> {
        let path = repository.join(INDEX_PATH);
        if !path.is_file() {
            return Ok(None);
        }
        serde_json::from_value(read_json(&path)?)
            .map(Some)
            .map_err(|error| TrialError::json(INDEX_PATH, error))
    }

    /// Whether a scenario identifier is a hold-out.
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.entries.iter().any(|entry| entry.id == id)
    }
}

/// The folder of the hold-out scenarios.
#[must_use]
pub fn holdout_directory(repository: &Path) -> PathBuf {
    repository
        .join("tools")
        .join("vsift-agent-trials")
        .join("holdout")
}

/// The events a scenario names, however it names them.
fn events_of(scenario: &Scenario) -> BTreeSet<String> {
    let mut events: BTreeSet<String> = scenario.truth_events.iter().cloned().collect();
    for phase in &scenario.phases {
        for expectation in &phase.expectations {
            match expectation {
                Expectation::UntrustedListed { event }
                | Expectation::TransientHonest { event }
                | Expectation::TranscriptOnlySupport { event, .. } => {
                    events.insert(event.clone());
                }
                _ => {}
            }
        }
    }
    if let Some(cold) = &scenario.cold
        && let ColdUsefulness::MissingToolsExplained {
            must_not_state_facts_of,
            ..
        } = &cold.usefulness
    {
        events.insert(must_not_state_facts_of.clone());
    }
    events
}

fn scenarios_in(directory: &Path) -> Result<Vec<Scenario>, TrialError> {
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    files_below(directory)?
        .into_iter()
        .filter(|(relative, _)| {
            Path::new(relative)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
                && relative != "INDEX.json"
        })
        .map(|(_, path)| Scenario::load(&path))
        .collect()
}

/// Every problem with the hold-out set of a repository; empty when it is
/// sound.
///
/// # Errors
///
/// [`TrialError`] when a file cannot be read or parsed.
pub fn check(repository: &Path) -> Result<Vec<String>, TrialError> {
    let mut problems = Vec::new();
    let Some(index) = HoldoutIndex::load(repository)? else {
        return Ok(vec![format!("{INDEX_PATH} does not exist")]);
    };
    let directory = holdout_directory(repository);
    let tuning_root = directory
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let mut tuning = scenarios_in(&tuning_root.join("scenarios"))?;
    tuning.extend(scenarios_in(&tuning_root.join("cold"))?);
    let tuning_ids: BTreeSet<&str> = tuning.iter().map(|scenario| scenario.id.as_str()).collect();
    let tuning_events: BTreeSet<String> = tuning.iter().flat_map(events_of).collect();

    let listed: BTreeSet<&str> = index
        .entries
        .iter()
        .map(|entry| entry.file.as_str())
        .collect();
    for (relative, _) in files_below(&directory)? {
        if relative != "INDEX.json" && !listed.contains(relative.as_str()) {
            problems.push(format!(
                "{relative} is in the hold-out folder but not in the index"
            ));
        }
    }
    let mut paths = BTreeSet::new();
    for entry in &index.entries {
        let file = directory.join(&entry.file);
        if !file.is_file() {
            problems.push(format!(
                "{}: the file {} does not exist",
                entry.id, entry.file
            ));
            continue;
        }
        if file_digest(&file)? != entry.sha256 {
            problems.push(format!(
                "{}: {} changed since it was frozen on {} (a hold-out is edited only with its index entry, in the open)",
                entry.id, entry.file, index.frozen
            ));
        }
        let scenario = Scenario::load(&file)?;
        if scenario.id != entry.id || entry.file != format!("{}.json", entry.id) {
            problems.push(format!(
                "{}: the file's scenario id is {}",
                entry.id, scenario.id
            ));
        }
        if tuning_ids.contains(entry.id.as_str()) {
            problems.push(format!("{} is also a tuning scenario id", entry.id));
        }
        let takes = match (&scenario.transcript, scenario.tools.whisper) {
            (Some(_), _) => Some(TranscriptPath::SuppliedTranscript),
            (None, true) => Some(TranscriptPath::LocalAsr),
            (None, false) => None,
        };
        if takes != Some(entry.path) {
            problems.push(format!(
                "{}: the index says {:?} but the scenario takes {takes:?}",
                entry.id, entry.path
            ));
        }
        if scenario.cold.is_some() {
            problems.push(format!(
                "{} is a cold scenario; hold-outs are skill trials",
                entry.id
            ));
        }
        let events = events_of(&scenario);
        if events.is_empty() {
            problems.push(format!("{} names no truth event", entry.id));
        }
        for event in events.intersection(&tuning_events) {
            problems.push(format!(
                "{}: {event} is named by a tuning or cold scenario",
                entry.id
            ));
        }
        paths.insert(entry.path);
    }
    for path in [TranscriptPath::SuppliedTranscript, TranscriptPath::LocalAsr] {
        if !paths.contains(&path) {
            problems.push(format!("no hold-out covers {path:?}"));
        }
    }
    Ok(problems)
}
