//! Campaign plans and their state (P14, plan section 7).
//!
//! The agent rounds are three batches, each started by the maintainer's go
//! because each spends the maintainer's Claude and Codex allowances:
//!
//! | Batch | When | Runs |
//! | --- | --- | --- |
//! | 1 | against the published 0.1.0 | 8 pilots (a dry run per client and mode, two scenarios each, compact tier) and the 12-run cold baseline (compact tier, 3 cold scenarios x 2 runs x 2 models) |
//! | 2 | on the candidate | the 34-run counted set with the skill (review tier 2 x 12, compact tier 2 x 5) |
//! | 3 | on the candidate | the cold final round: compact 12 and review 6 (18 runs) |
//!
//! That is 20 + 34 + 18 = 72 planned runs and a reserve of 12 more, the
//! plan's 84. A **plan** is a pure list ([`plan`]); a **state file** holds it
//! with every attempt, one file per client so that the Windows Claude Code
//! loop and the Codex container loop never write the same file. The campaign
//! scripts only ask `campaign next`, run what it says and report `campaign
//! mark`: the order, the counting rules and the retry limits live here, where
//! tests hold them.
//!
//! **Counting rules.** A trial that ran under its configuration counts,
//! pass or fail ([`Outcome::Counted`]). A phase the client's own
//! configuration report invalidated ([`Outcome::Invalid`]), a harness error
//! and a usage-limited phase are not counted and the run stays pending. A
//! run with three invalid or errored attempts is **blocked**: `campaign next`
//! stops and tells the operator, because the plan's counts are never reduced
//! silently (usage-limited attempts do not count toward the three: waiting
//! costs nothing). A reserve run is added by hand and capped.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{
    error::{TrialError, read_json, write_json},
    scenario::TrialMode,
};

/// The review-tier Claude Code model.
pub const CLAUDE_REVIEW_MODEL: &str = "claude-opus-5-5";
/// The compact-tier Claude Code model.
pub const CLAUDE_COMPACT_MODEL: &str = "claude-sonnet-5-5";
/// The review-tier Codex model.
pub const CODEX_REVIEW_MODEL: &str = "gpt-6-astra";
/// The compact-tier Codex model.
pub const CODEX_COMPACT_MODEL: &str = "gpt-6-sol";

/// Attempts a run may fail to count (invalid or errored) before it blocks.
pub const MAX_FAILED_ATTEMPTS: usize = 3;

/// Reserve runs one state file may add (the plan's 12 reserve runs are shared
/// by two clients and three batches; the operator keeps the total).
pub const MAX_RESERVE_PER_STATE: usize = 6;

/// The named clients.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum ClientName {
    /// Claude Code, on the maintainer's Windows 11 machine.
    Claude,
    /// Codex, in the Linux container.
    Codex,
}

impl ClientName {
    /// The name in file names and run identifiers.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    /// The model of a tier.
    #[must_use]
    pub const fn model(self, tier: Tier) -> &'static str {
        match (self, tier) {
            (Self::Claude, Tier::Review) => CLAUDE_REVIEW_MODEL,
            (Self::Claude, Tier::Compact) => CLAUDE_COMPACT_MODEL,
            (Self::Codex, Tier::Review) => CODEX_REVIEW_MODEL,
            (Self::Codex, Tier::Compact) => CODEX_COMPACT_MODEL,
        }
    }
}

/// The model tiers of ADR 0022.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// The strong models.
    Review,
    /// The small models.
    Compact,
}

/// What a planned run is for.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    /// A dry run that checks the environment; never counted in a gate.
    Pilot,
    /// A run of a batch's counted set.
    Counted,
    /// An extra run the operator added, under the rule stated before the
    /// batch.
    Reserve,
}

/// One planned run.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlannedRun {
    /// A stable identifier: batch, client, kind, scenario and repeat.
    pub run_id: String,
    /// The batch, 1 to 3.
    pub batch: u8,
    /// What the run is for.
    pub kind: RunKind,
    /// The client.
    pub client: ClientName,
    /// The model.
    pub model: String,
    /// The tier of the model.
    pub tier: Tier,
    /// The scenario identifier.
    pub scenario: String,
    /// Skill or cold.
    pub mode: TrialMode,
}

/// How an attempt ended.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The trial ran under its configuration: it counts, pass or fail.
    Counted,
    /// The client reported it ignored its configuration: not counted.
    Invalid,
    /// The client stopped at its usage limit: not counted, run again later.
    UsageLimited,
    /// The harness failed before or after the client ran: not counted.
    HarnessError,
}

/// One attempt at a run.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Attempt {
    /// The trial's identifier (its folder name), when one was made.
    pub trial_id: Option<String>,
    /// How it ended.
    pub outcome: Outcome,
    /// A short note by the operator's script (never a path or a prompt).
    pub note: Option<String>,
}

/// Where a run stands.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Not yet counted.
    Pending,
    /// Counted.
    Counted,
    /// Too many failed attempts: the operator decides.
    Blocked,
}

/// A planned run and its attempts.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RunState {
    /// The plan.
    pub run: PlannedRun,
    /// Where it stands.
    pub status: Status,
    /// Every attempt, oldest first.
    pub attempts: Vec<Attempt>,
}

/// One client's state file.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CampaignState {
    /// Record version, 1.
    pub schema_version: u32,
    /// The batch.
    pub batch: u8,
    /// The client this file belongs to.
    pub client: ClientName,
    /// The published version under test (`0.1.0`, `0.2.0-rc.1`).
    pub version: String,
    /// Every run.
    pub runs: Vec<RunState>,
}

/// The scenarios and repeats of one tier of one batch.
type Table = &'static [(&'static str, usize)];

const PILOT_SKILL: Table = &[("A-08-f05-local-asr", 1), ("A-09-f05-supplied", 1)];
const PILOT_COLD: Table = &[("C-01-f05-supplied", 1), ("C-02-f05-local-asr", 1)];
const COLD_ROUND: Table = &[
    ("C-01-f05-supplied", 2),
    ("C-02-f05-local-asr", 2),
    ("C-03-f03-missing-tools", 2),
];
const COLD_REVIEW_ROUND: Table = &[
    ("C-01-f05-supplied", 1),
    ("C-02-f05-local-asr", 1),
    ("C-03-f03-missing-tools", 1),
];
const SKILL_REVIEW: Table = &[
    ("A-08-f05-local-asr", 3),
    ("A-09-f05-supplied", 3),
    ("H-01-f10-supplied-sidecar", 1),
    ("H-02-f01-local-asr", 1),
    ("A-01-f01-do-not-install", 1),
    ("A-09-f05-blurred", 3),
];
const SKILL_COMPACT: Table = &[
    ("A-08-f05-local-asr", 2),
    ("A-09-f05-supplied", 2),
    ("SEC-T02-f12-webvtt", 1),
];

fn mode_of(scenario: &str) -> TrialMode {
    if scenario.starts_with("C-") {
        TrialMode::Cold
    } else {
        TrialMode::Skill
    }
}

fn push_table(
    runs: &mut Vec<PlannedRun>,
    batch: u8,
    client: ClientName,
    tier: Tier,
    kind: RunKind,
    table: Table,
) {
    for (scenario, repeats) in table {
        for repeat in 1..=*repeats {
            runs.push(PlannedRun {
                run_id: format!(
                    "b{batch}-{}-{}-{}-{}-{repeat}",
                    client.slug(),
                    match tier {
                        Tier::Review => "review",
                        Tier::Compact => "compact",
                    },
                    match kind {
                        RunKind::Pilot => "pilot",
                        RunKind::Counted => "counted",
                        RunKind::Reserve => "reserve",
                    },
                    scenario.to_ascii_lowercase()
                ),
                batch,
                kind,
                client,
                model: client.model(tier).to_owned(),
                tier,
                scenario: (*scenario).to_owned(),
                mode: mode_of(scenario),
            });
        }
    }
}

/// The plan of one batch for one client.
///
/// # Errors
///
/// [`TrialError::Refused`] for a batch other than 1, 2 or 3.
pub fn plan(batch: u8, client: ClientName) -> Result<Vec<PlannedRun>, TrialError> {
    let mut runs = Vec::new();
    match batch {
        1 => {
            push_table(
                &mut runs,
                1,
                client,
                Tier::Compact,
                RunKind::Pilot,
                PILOT_SKILL,
            );
            push_table(
                &mut runs,
                1,
                client,
                Tier::Compact,
                RunKind::Pilot,
                PILOT_COLD,
            );
            push_table(
                &mut runs,
                1,
                client,
                Tier::Compact,
                RunKind::Counted,
                COLD_ROUND,
            );
        }
        2 => {
            push_table(
                &mut runs,
                2,
                client,
                Tier::Review,
                RunKind::Counted,
                SKILL_REVIEW,
            );
            push_table(
                &mut runs,
                2,
                client,
                Tier::Compact,
                RunKind::Counted,
                SKILL_COMPACT,
            );
        }
        3 => {
            push_table(
                &mut runs,
                3,
                client,
                Tier::Compact,
                RunKind::Counted,
                COLD_ROUND,
            );
            push_table(
                &mut runs,
                3,
                client,
                Tier::Review,
                RunKind::Counted,
                COLD_REVIEW_ROUND,
            );
        }
        other => {
            return Err(TrialError::Refused(format!(
                "there is no batch {other}; the batches are 1, 2 and 3"
            )));
        }
    }
    Ok(runs)
}

impl CampaignState {
    /// A fresh state file for one client's share of a batch.
    ///
    /// # Errors
    ///
    /// As [`plan`].
    pub fn new(batch: u8, client: ClientName, version: &str) -> Result<Self, TrialError> {
        crate::install::validate_version(version)?;
        Ok(Self {
            schema_version: 1,
            batch,
            client,
            version: version.to_owned(),
            runs: plan(batch, client)?
                .into_iter()
                .map(|run| RunState {
                    run,
                    status: Status::Pending,
                    attempts: Vec::new(),
                })
                .collect(),
        })
    }

    /// Reads a state file.
    ///
    /// # Errors
    ///
    /// [`TrialError`] when it is missing or malformed.
    pub fn load(path: &Path) -> Result<Self, TrialError> {
        serde_json::from_value(read_json(path)?)
            .map_err(|error| TrialError::json(path.display().to_string(), error))
    }

    /// Writes the state file.
    ///
    /// # Errors
    ///
    /// [`TrialError`] when it cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), TrialError> {
        write_json(path, self)
    }

    /// The next run to do: the first that is pending. `Err` carries the
    /// identifier of a blocked run that stops the campaign, so that the
    /// operator decides and counts are never reduced silently.
    ///
    /// # Errors
    ///
    /// The identifier of the first blocked run.
    pub fn next(&self) -> Result<Option<&RunState>, String> {
        for state in &self.runs {
            match state.status {
                Status::Blocked => return Err(state.run.run_id.clone()),
                Status::Pending => return Ok(Some(state)),
                Status::Counted => {}
            }
        }
        Ok(None)
    }

    /// Records one attempt at a run.
    ///
    /// # Errors
    ///
    /// [`TrialError::Invalid`] when the run is unknown or already counted.
    pub fn mark(
        &mut self,
        run_id: &str,
        trial_id: Option<String>,
        outcome: Outcome,
        note: Option<String>,
    ) -> Result<Status, TrialError> {
        let state = self
            .runs
            .iter_mut()
            .find(|state| state.run.run_id == run_id)
            .ok_or_else(|| TrialError::Invalid(format!("no run {run_id}")))?;
        if state.status == Status::Counted {
            return Err(TrialError::Invalid(format!("{run_id} is already counted")));
        }
        state.attempts.push(Attempt {
            trial_id,
            outcome,
            note,
        });
        let failed = state
            .attempts
            .iter()
            .filter(|attempt| matches!(attempt.outcome, Outcome::Invalid | Outcome::HarnessError))
            .count();
        state.status = if outcome == Outcome::Counted {
            Status::Counted
        } else if failed >= MAX_FAILED_ATTEMPTS {
            Status::Blocked
        } else {
            Status::Pending
        };
        Ok(state.status)
    }

    /// Adds a reserve run at the end (the operator's rule for using the
    /// reserve is stated before the batch).
    ///
    /// # Errors
    ///
    /// [`TrialError::Refused`] past [`MAX_RESERVE_PER_STATE`].
    pub fn add_reserve(&mut self, scenario: &str, tier: Tier) -> Result<String, TrialError> {
        let used = self
            .runs
            .iter()
            .filter(|state| state.run.kind == RunKind::Reserve)
            .count();
        if used >= MAX_RESERVE_PER_STATE {
            return Err(TrialError::Refused(format!(
                "this state file already holds {MAX_RESERVE_PER_STATE} reserve runs"
            )));
        }
        let run_id = format!(
            "b{}-{}-reserve-{}-{}",
            self.batch,
            self.client.slug(),
            scenario.to_ascii_lowercase(),
            used + 1
        );
        self.runs.push(RunState {
            run: PlannedRun {
                run_id: run_id.clone(),
                batch: self.batch,
                kind: RunKind::Reserve,
                client: self.client,
                model: self.client.model(tier).to_owned(),
                tier,
                scenario: scenario.to_owned(),
                mode: mode_of(scenario),
            },
            status: Status::Pending,
            attempts: Vec::new(),
        });
        Ok(run_id)
    }

    /// A one-paragraph status for the operator.
    #[must_use]
    pub fn describe(&self) -> String {
        let count = |status: Status| {
            self.runs
                .iter()
                .filter(|state| state.status == status)
                .count()
        };
        let limited = self
            .runs
            .iter()
            .flat_map(|state| &state.attempts)
            .filter(|attempt| attempt.outcome == Outcome::UsageLimited)
            .count();
        format!(
            "batch {} ({}, {}): {} counted, {} pending, {} blocked of {} runs; {} usage-limited attempt(s) so far",
            self.batch,
            self.client.slug(),
            self.version,
            count(Status::Counted),
            count(Status::Pending),
            count(Status::Blocked),
            self.runs.len(),
            limited
        )
    }
}
