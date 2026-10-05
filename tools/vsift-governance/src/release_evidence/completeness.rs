//! The completeness check (evidence item RQ-20): is there, for a given
//! release, a recorded answer for everything the release must show?
//!
//! Structure rules ([`super::structure`]) hold at every commit. This check
//! runs on demand, for one release, and is meant to be a release gate (P14
//! PRs 8, 10, 12):
//!
//! ```text
//! vsift-governance release-evidence --complete-for 0.2.0-rc.1
//! vsift-governance release-evidence --complete-for 0.2.0
//! ```
//!
//! **Candidate.** Every item whose candidate rule is `required` must be
//! `passed` for the candidate, or `waived` by a recorded maintainer decision,
//! or `not_applicable` with a reason. A `passed` entry counts when it is for
//! the candidate's own version and commit, or, under the **staleness rule**
//! (ADR 0024, "Details left to their pull requests"), when it was recorded at
//! an earlier commit and no file under the item's `scope` changed between that
//! commit and the candidate's. The second case asks Git, through
//! [`ChangeOracle`]; if Git cannot tell, the entry does not count.
//!
//! **Stable.** An item with the stable rule `repeat` needs its own entry for
//! the stable version and commit; carry-forward never applies, because the
//! evidence is about the published bytes or is a dated reading. An item with
//! the rule `carry` needs the candidate evidence above, for the candidate
//! named by the ledger's `release_delta`, and the delta must be recorded and
//! allowed.
//!
//! # Extension point for P14 PR 8
//!
//! ADR 0024 decision A says the stable commit may differ from the accepted
//! candidate only in version strings and in documents that ship inside the
//! artifacts (since P14 PR 10b also the work record, which no artifact holds
//! and which must change when the candidate's evidence is recorded), and that
//! a mechanical check enforces it. That check is release
//! tooling (PR 8). It is not built here. This module only *consumes* its
//! result: `release_delta`, a record of the candidate and stable versions and
//! commits, the check that ran and its verdict. Until PR 8 writes one, a stable
//! target with a `carry` item fails with the message that the delta is not
//! recorded, which is the safe answer.

use std::{path::Path, process::Command};

use super::{
    schema::{
        CandidateRule, DeltaVerdict, EvidenceItem, EvidenceLedger, ReleaseDelta, StableRule,
        Status, Subject,
    },
    values::{CommitSha, ReleaseVersion},
};

/// The release a completeness check is run for.
#[derive(Clone, Debug)]
pub(crate) enum Target {
    /// A release candidate (`-rc.N`) built from `commit`.
    Candidate {
        /// The candidate's version.
        version: ReleaseVersion,
        /// The commit the candidate is built from.
        commit: CommitSha,
    },
    /// The stable release built from `commit`, cut from the candidate that
    /// the ledger's `release_delta` names.
    Stable {
        /// The stable version.
        version: ReleaseVersion,
        /// The commit the stable release is built from.
        commit: CommitSha,
    },
}

/// Tells which files of a scope changed between two commits.
pub(crate) trait ChangeOracle {
    /// The files below any path of `scope` that differ between `from` and
    /// `to`, or the reason that cannot be told.
    fn changed_files(
        &self,
        from: &CommitSha,
        to: &CommitSha,
        scope: &[String],
    ) -> Result<Vec<String>, String>;
}

/// Checks the ledger for completeness against `target`.
pub(crate) fn check_completeness(
    ledger: &EvidenceLedger,
    target: &Target,
    oracle: &dyn ChangeOracle,
) -> Vec<String> {
    let mut messages = Vec::new();
    match target {
        Target::Candidate { version, commit } => {
            if !version.is_candidate() {
                messages.push(format!(
                    "{version} is not a release candidate; a stable version is checked as a \
                     stable target"
                ));
                return messages;
            }
            for item in &ledger.items {
                if item.gate.candidate == CandidateRule::Required
                    && let Err(why) = holds_for(item, version, commit, Carry::ByScope, oracle)
                {
                    messages.push(format!("{}: {why}", item.id));
                }
            }
        }
        Target::Stable { version, commit } => {
            check_stable(&mut messages, ledger, version, commit, oracle);
        }
    }
    messages
}

/// The stable target: repeated items at the stable commit, carried items at
/// the accepted candidate, and a recorded, allowed delta between the two.
fn check_stable(
    messages: &mut Vec<String>,
    ledger: &EvidenceLedger,
    version: &ReleaseVersion,
    commit: &CommitSha,
    oracle: &dyn ChangeOracle,
) {
    if version.is_candidate() {
        messages.push(format!(
            "{version} is a release candidate, not a stable version; check it as a candidate"
        ));
        return;
    }
    let delta = if let Some(delta) = &ledger.release_delta {
        accepted_delta(messages, delta, version, commit)
    } else {
        messages.push(String::from(
            "the stable-over-candidate delta is not recorded (release_delta is null): \
             ADR 0024 decision A requires the check that only version strings, shipped \
             documents and the work record differ, which P14 PR 8 builds, before a stable \
             release may carry candidate evidence",
        ));
        None
    };
    for item in &ledger.items {
        let result = match item.gate.stable {
            StableRule::NotRequired => continue,
            StableRule::Repeat => holds_for(item, version, commit, Carry::Never, oracle),
            StableRule::Carry => match delta {
                Some(delta) => holds_for(
                    item,
                    &delta.candidate_version,
                    &delta.candidate_commit,
                    Carry::ByScope,
                    oracle,
                ),
                None => Err(String::from(
                    "carries candidate evidence, which needs a recorded, allowed delta",
                )),
            },
        };
        if let Err(why) = result {
            messages.push(format!("{}: {why}", item.id));
        }
    }
}

/// The recorded delta, if it is for this stable release and allows carrying.
fn accepted_delta<'a>(
    messages: &mut Vec<String>,
    delta: &'a ReleaseDelta,
    version: &ReleaseVersion,
    commit: &CommitSha,
) -> Option<&'a ReleaseDelta> {
    let mut accepted = true;
    if delta.stable_version != *version || delta.stable_commit != *commit {
        messages.push(format!(
            "release_delta is for {} at {}, not for {version} at {commit}",
            delta.stable_version, delta.stable_commit
        ));
        accepted = false;
    }
    if delta.verdict != DeltaVerdict::Allowed {
        messages.push(format!(
            "release_delta: the check of {} found that the stable release differs from {} in \
             more than version strings, shipped documents and the work record",
            delta.date, delta.candidate_version
        ));
        accepted = false;
    }
    accepted.then_some(delta)
}

/// Whether older evidence may count for a later commit.
#[derive(Clone, Copy)]
enum Carry {
    /// When no file in the item's scope changed in between (the staleness
    /// rule).
    ByScope,
    /// Never: the item needs an entry for exactly this version and commit.
    Never,
}

/// Whether `item` has an answer for the release `version` built from
/// `commit`, or why it has none.
fn holds_for(
    item: &EvidenceItem,
    version: &ReleaseVersion,
    commit: &CommitSha,
    carry: Carry,
    oracle: &dyn ChangeOracle,
) -> Result<(), String> {
    match item.status {
        Status::Waived | Status::NotApplicable => Ok(()),
        Status::Planned | Status::Running | Status::Failed => Err(format!(
            "is {}; it must be passed, waived by the maintainer or not applicable for {version}",
            item.status.label()
        )),
        Status::Passed => {
            let Some(Subject {
                version: recorded_version,
                commit: recorded_commit,
            }) = &item.applies_to
            else {
                return Err(String::from("is passed without a version and commit"));
            };
            if recorded_version == version && recorded_commit == commit {
                return Ok(());
            }
            match carry {
                Carry::Never => Err(format!(
                    "is passed for {recorded_version}, and this release needs its own entry for {version}"
                )),
                Carry::ByScope => carried_by_scope(
                    item,
                    recorded_version,
                    recorded_commit,
                    version,
                    commit,
                    oracle,
                ),
            }
        }
    }
}

/// The staleness rule: an entry from another commit counts when nothing in
/// the item's scope changed since.
fn carried_by_scope(
    item: &EvidenceItem,
    recorded_version: &ReleaseVersion,
    recorded_commit: &CommitSha,
    version: &ReleaseVersion,
    commit: &CommitSha,
    oracle: &dyn ChangeOracle,
) -> Result<(), String> {
    match oracle.changed_files(recorded_commit, commit, &item.scope) {
        Err(error) => Err(format!(
            "is passed for {recorded_version}, and whether its scope changed before {version} \
             cannot be told: {error}"
        )),
        Ok(changed) if changed.is_empty() => Ok(()),
        Ok(changed) => {
            let shown: Vec<&str> = changed.iter().take(3).map(String::as_str).collect();
            Err(format!(
                "is passed for {recorded_version}, but {} file(s) in its scope changed before \
                 {version} (for example {}); record it again",
                changed.len(),
                shown.join(", ")
            ))
        }
    }
}

/// Git, asked from a checkout.
pub(crate) struct GitOracle<'a> {
    root: &'a Path,
}

impl<'a> GitOracle<'a> {
    /// An oracle that runs `git` in the checkout at `root`.
    pub(crate) fn new(root: &'a Path) -> Self {
        Self { root }
    }

    /// The commit `HEAD` names.
    pub(crate) fn head(&self) -> Result<CommitSha, String> {
        let output = self.run(&["rev-parse", "--verify", "HEAD"])?;
        CommitSha::try_from(output.trim().to_owned()).map_err(|error| error.to_string())
    }

    /// Runs `git` with explicit arguments and no shell, and returns standard
    /// output. `--literal-pathspecs` stops a path from being read as a glob
    /// or a pathspec magic word.
    fn run(&self, arguments: &[&str]) -> Result<String, String> {
        let output = Command::new("git")
            .arg("--literal-pathspecs")
            .args(arguments)
            .current_dir(self.root)
            .output()
            .map_err(|error| format!("git could not be started: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "git {} failed: {}",
                arguments.first().copied().unwrap_or("command"),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        String::from_utf8(output.stdout).map_err(|_| String::from("git printed non-UTF-8 text"))
    }
}

impl ChangeOracle for GitOracle<'_> {
    fn changed_files(
        &self,
        from: &CommitSha,
        to: &CommitSha,
        scope: &[String],
    ) -> Result<Vec<String>, String> {
        if let Some(path) = scope
            .iter()
            .find(|path| !crate::repository::is_repository_relative(path))
        {
            return Err(format!("scope path {path:?} is not repository-relative"));
        }
        let mut arguments = vec![
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            from.as_str(),
            to.as_str(),
            "--",
        ];
        arguments.extend(scope.iter().map(String::as_str));
        let listing = self.run(&arguments)?;
        Ok(listing
            .split('\0')
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect())
    }
}

#[cfg(test)]
mod tests;
