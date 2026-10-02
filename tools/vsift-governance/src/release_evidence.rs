//! The release evidence ledger and its checks (P14 PR 1; ADR 0024,
//! "What P14 delivers" item 1).
//!
//! `docs/planning/p14-evidence-ledger.json` holds one entry per evidence item
//! the P14 plan defines. Two checks read it:
//!
//! - **Structure** ([`structure`]) runs inside the normal `check` command, so
//!   the Governance job enforces it on every pull request: the schema, the
//!   item set against the plan, every referenced identifier against the
//!   document that owns it, the rules each status carries, and that nothing
//!   the plan lists is unowned.
//! - **Completeness** ([`completeness`]) runs on demand for one release
//!   (`release-evidence --complete-for <version>`) and fails unless everything
//!   that release must show is answered.
//!
//! Neither check fetches anything. A link in the ledger is a claim: the checks
//! can say a record path exists and an identifier is well formed, not that the
//! run behind a link passed (known limit L-101). The ledger is evidence
//! bookkeeping for people who read it; it does not replace reading the runs.

mod completeness;
mod facts;
mod schema;
mod structure;
mod values;

#[cfg(test)]
pub(crate) mod fixture;

use std::path::Path;

pub(crate) use schema::{EvidenceLedger, Status};
pub(crate) use values::{CommitSha, ReleaseVersion};

use completeness::{GitOracle, Target, check_completeness};
use facts::Facts;
use structure::check_structure;

use crate::repository::{DiskRepository, Repository};

/// The ledger file, relative to the repository root.
pub(crate) const LEDGER_PATH: &str = "docs/planning/p14-evidence-ledger.json";

/// The evidence item that is the completeness check itself (the plan's
/// RQ-20). It checks the others, so it supports nothing of its own and the
/// completeness check does not wait for it.
pub(crate) const COMPLETENESS_ITEM: &str = "RQ-20";

/// A request to check completeness for one release.
#[derive(Clone, Debug)]
pub(crate) struct CompletenessRequest {
    /// The release version; a version with `-rc.N` is checked as a candidate,
    /// one without as the stable release.
    pub(crate) version: ReleaseVersion,
    /// The commit the release is built from; `HEAD` of the checkout when
    /// not given.
    pub(crate) commit: Option<CommitSha>,
}

/// Reads and parses the ledger.
pub(crate) fn load(repository: &dyn Repository) -> Result<EvidenceLedger, String> {
    let text = repository.read_text(LEDGER_PATH)?;
    serde_json::from_str(&text).map_err(|error| format!("{LEDGER_PATH} could not be read: {error}"))
}

/// Appends the ledger's structure findings. This is the part of the checks
/// that runs on every pull request.
pub(crate) fn validate_release_evidence(messages: &mut Vec<String>, repository: &dyn Repository) {
    messages.extend(structure_findings(repository));
}

fn structure_findings(repository: &dyn Repository) -> Vec<String> {
    let ledger = match load(repository) {
        Ok(ledger) => ledger,
        Err(error) => return vec![error],
    };
    let facts = match Facts::read(repository, &ledger.plan) {
        Ok(facts) => facts,
        Err(errors) => return errors,
    };
    check_structure(&ledger, &facts, repository)
        .into_iter()
        .map(|message| format!("{LEDGER_PATH}: {message}"))
        .collect()
}

/// Runs the `release-evidence` command: the structure rules, and, when asked,
/// completeness for one release. Returns the findings and a sentence saying
/// what was checked when there are none.
pub(crate) fn run_release_evidence(
    root: &Path,
    request: Option<&CompletenessRequest>,
) -> Result<String, Vec<String>> {
    let repository = DiskRepository::new(root);
    let findings = structure_findings(&repository);
    if !findings.is_empty() {
        return Err(findings);
    }
    let Some(request) = request else {
        return Ok(String::from("VSift release evidence ledger is valid."));
    };
    let ledger = load(&repository).map_err(|error| vec![error])?;
    let oracle = GitOracle::new(root);
    let commit = match &request.commit {
        Some(commit) => commit.clone(),
        None => oracle.head().map_err(|error| {
            vec![format!(
                "the commit to check could not be resolved (give --commit): {error}"
            )]
        })?,
    };
    let target = if request.version.is_candidate() {
        Target::Candidate {
            version: request.version,
            commit: commit.clone(),
        }
    } else {
        Target::Stable {
            version: request.version,
            commit: commit.clone(),
        }
    };
    let findings: Vec<String> = check_completeness(&ledger, &target, &oracle)
        .into_iter()
        .map(|message| {
            format!(
                "{LEDGER_PATH}: incomplete for {}: {message}",
                request.version
            )
        })
        .collect();
    if findings.is_empty() {
        let short: String = commit.as_str().chars().take(12).collect();
        Ok(format!(
            "VSift release evidence is complete for {} at {short}.",
            request.version
        ))
    } else {
        Err(findings)
    }
}
