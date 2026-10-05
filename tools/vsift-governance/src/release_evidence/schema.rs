//! The release evidence ledger, schema version 1.
//!
//! The ledger is `docs/planning/p14-evidence-ledger.json`. It holds one entry
//! per evidence item the P14 plan defines (`RQ-01` to `RQ-20`), and says for
//! each what it proves and does not prove, who produces it, which release it
//! must be recorded for, its status, and the links that back the status.
//!
//! It is separate from the delivery ledger because that ledger refuses a
//! `verification` list or a merge commit before a packet completes and rejects
//! unknown fields, so evidence gathered pull request by pull request cannot
//! live there (ADR 0024, "What P14 delivers").
//!
//! Every struct rejects unknown fields and every closed set is an enum, so a
//! misspelt status or an invented field fails the parse instead of being
//! ignored.

use serde::Deserialize;

use super::values::{CommitSha, IsoDate, ReleaseVersion};

/// The only schema version this checker reads.
pub(crate) const SCHEMA_VERSION: &str = "1";

/// The whole ledger file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvidenceLedger {
    /// Always [`SCHEMA_VERSION`].
    pub(crate) schema_version: String,
    /// The packet that owns the evidence: `P14`.
    pub(crate) packet: String,
    /// The plan document whose evidence-items table defines the item set.
    pub(crate) plan: String,
    /// The recorded result of the stable-over-candidate delta check.
    ///
    /// Null until that check exists and has run (P14 PR 8 builds it). Stable
    /// completeness needs it whenever an item carries its candidate evidence
    /// to the stable release (ADR 0024 decision A).
    pub(crate) release_delta: Option<ReleaseDelta>,
    /// One entry per evidence item, in plan order.
    pub(crate) items: Vec<EvidenceItem>,
}

/// The recorded outcome of comparing the stable commit with the accepted
/// release candidate: they may differ only in version strings, in documents
/// that ship inside the artifacts and in the work record (ADR 0024 decision A;
/// the work record was added in P14 PR 10b).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReleaseDelta {
    /// The accepted release candidate the stable was cut from.
    pub(crate) candidate_version: ReleaseVersion,
    /// The commit the candidate was built from.
    pub(crate) candidate_commit: CommitSha,
    /// The stable version.
    pub(crate) stable_version: ReleaseVersion,
    /// The commit the stable version is built from.
    pub(crate) stable_commit: CommitSha,
    /// What the check concluded.
    pub(crate) verdict: DeltaVerdict,
    /// The run or record that performed the check.
    pub(crate) check: EvidenceReference,
    /// When the check ran.
    pub(crate) date: IsoDate,
}

/// The delta check's conclusion.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeltaVerdict {
    /// Only version strings, shipped documents and the work record differ.
    Allowed,
    /// Something else differs; the stable may not carry candidate evidence.
    Rejected,
}

/// One evidence item (`RQ-nn`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvidenceItem {
    /// `RQ-nn`, as the plan's table spells it.
    pub(crate) id: String,
    /// A short name.
    pub(crate) title: String,
    /// The requirements, threats, verification rows and limits it supports.
    pub(crate) supports: Supports,
    /// What a pass shows, in the plan's words.
    pub(crate) proves: String,
    /// What a pass does not show.
    pub(crate) does_not_prove: String,
    /// What produces the evidence.
    pub(crate) producer: Producer,
    /// Repository paths whose change makes earlier evidence stale (the
    /// staleness rule): evidence recorded at an earlier commit still counts
    /// for a later one only when no file under these paths changed between
    /// the two. A directory names everything below it; `.` names everything.
    pub(crate) scope: Vec<String>,
    /// For which publications it must be recorded.
    pub(crate) gate: Gate,
    /// Where the item stands.
    pub(crate) status: Status,
    /// The version and commit the recorded evidence is for. Present exactly
    /// when the status is `running`, `passed` or `failed`.
    pub(crate) applies_to: Option<Subject>,
    /// Links that back the status. Empty (and omitted from the file) when
    /// the status is `planned`, `waived` or `not_applicable`.
    #[serde(default)]
    pub(crate) evidence: Vec<EvidenceEntry>,
    /// Earlier evidence that does not count (another commit, a source build,
    /// a partial run). It is kept, with its limits, so the next session sees
    /// what exists without mistaking it for a pass.
    #[serde(default)]
    pub(crate) prior: Vec<EvidenceEntry>,
    /// Tracked issue numbers; required when the status is `failed`
    /// (governance rule 14).
    #[serde(default)]
    pub(crate) issues: Vec<u32>,
    /// The maintainer's decision; present exactly when the status is `waived`.
    pub(crate) decision: Option<Decision>,
    /// Why the item does not apply; present when the status is
    /// `not_applicable`, optional when `waived`.
    pub(crate) reason: Option<String>,
    /// When the status last changed.
    pub(crate) date: IsoDate,
}

/// What an evidence item supports. Every id must exist in its source.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Supports {
    /// `R-nn` ids of the delivery ledger.
    pub(crate) requirements: Vec<String>,
    /// `SEC-nn` ids of the threat model.
    pub(crate) threats: Vec<String>,
    /// Row ids of `verification.md` (`A-08`, `R-SEC03`, `RQ-01`, ...).
    pub(crate) verification: Vec<String>,
    /// `L-nnn` ids of the known-limits register.
    pub(crate) limits: Vec<String>,
}

/// What produces an evidence item and which P14 pull request builds it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Producer {
    /// The kind of producer.
    pub(crate) kind: ProducerKind,
    /// The P14 pull request (0 to 13) that builds the producer, or runs it
    /// when it already exists.
    pub(crate) p14_pr: u8,
    /// The workflow, procedure or reading, in words.
    pub(crate) description: String,
}

/// The kinds of producer the plan uses.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProducerKind {
    /// A GitHub Actions workflow on hosted runners.
    Workflow,
    /// A documented procedure the packet owner's session runs.
    Procedure,
    /// An action only the maintainer can take.
    Maintainer,
    /// A reading or review recorded by the packet owner.
    Review,
    /// A check of the `vsift-governance` tool.
    GovernanceCheck,
}

/// For which publications an item must be recorded.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Gate {
    /// The rule for the release candidate.
    pub(crate) candidate: CandidateRule,
    /// The rule for the stable release.
    pub(crate) stable: StableRule,
}

/// Whether the release candidate needs the item.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CandidateRule {
    /// It must be `passed` (or waived, or not applicable) for the candidate.
    Required,
    /// The completeness check does not need it. Used for the check's own
    /// entry, which cannot be a precondition of itself.
    NotRequired,
}

/// What the stable release needs of an item that the candidate needed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StableRule {
    /// Recorded again for the stable version: the evidence is about the
    /// published bytes or a dated reading, so the candidate's does not carry.
    Repeat,
    /// The candidate's evidence carries to the stable when the recorded
    /// delta check allows it.
    Carry,
    /// The stable release does not need it.
    NotRequired,
}

/// Where an evidence item stands.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    /// Nothing counted has run.
    Planned,
    /// A run or procedure has started and has not finished.
    Running,
    /// The pass rule of the plan is met for [`EvidenceItem::applies_to`].
    Passed,
    /// A run finished against the pass rule; an issue tracks it.
    Failed,
    /// The maintainer decided the release goes without it.
    Waived,
    /// The item does not apply, for a stated reason.
    NotApplicable,
}

impl Status {
    /// The word the file and the messages use for the status.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Waived => "waived",
            Self::NotApplicable => "not_applicable",
        }
    }
}

/// The release the recorded evidence is for.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Subject {
    /// The version, such as `0.2.0-rc.1`.
    pub(crate) version: ReleaseVersion,
    /// The commit the version was built from.
    pub(crate) commit: CommitSha,
}

/// One link that backs a status, with its date and what it shows.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvidenceEntry {
    /// What the link points at.
    pub(crate) reference: EvidenceReference,
    /// The date of the evidence (the run, the record), not of the edit.
    pub(crate) date: IsoDate,
    /// What it shows and its limits, in a sentence.
    pub(crate) note: String,
}

/// A typed link. Nothing here is fetched: the check can verify a record
/// path exists and that a number or identifier is well formed, not that a
/// run passed.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum EvidenceReference {
    /// A GitHub Actions run of this repository.
    WorkflowRun {
        /// The workflow's display name.
        workflow: String,
        /// The run id.
        run_id: u64,
    },
    /// A pull request of this repository.
    PullRequest {
        /// Its number.
        number: u32,
    },
    /// An issue of this repository.
    Issue {
        /// Its number.
        number: u32,
    },
    /// A file of this repository (a qualification record, an ADR).
    Record {
        /// Its repository-relative path.
        path: String,
    },
}

/// A maintainer decision recorded for a waiver.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Decision {
    /// What the maintainer decided, in a sentence.
    pub(crate) statement: String,
    /// Where it is recorded.
    pub(crate) reference: EvidenceReference,
    /// When it was decided.
    pub(crate) date: IsoDate,
}
