//! One test per completeness rule, with a scripted change oracle, and the Git
//! oracle against a throwaway repository.

use std::{error::Error, fs, path::Path, process::Command};

use serde_json::{Value, json};

use super::{ChangeOracle, GitOracle, Target, check_completeness};
use crate::release_evidence::{
    fixture::{LATER_COMMIT, RECORDED_COMMIT, baseline_value, insert, ledger, set},
    values::{CommitSha, ReleaseVersion},
};

type Outcome = Result<(), Box<dyn Error>>;

/// An oracle that answers every question the same way.
struct Scripted(Result<Vec<String>, String>);

impl ChangeOracle for Scripted {
    fn changed_files(
        &self,
        _from: &CommitSha,
        _to: &CommitSha,
        _scope: &[String],
    ) -> Result<Vec<String>, String> {
        self.0.clone()
    }
}

fn unchanged() -> Scripted {
    Scripted(Ok(Vec::new()))
}

fn version(text: &str) -> Result<ReleaseVersion, Box<dyn Error>> {
    Ok(ReleaseVersion::parse(text)?)
}

fn commit(text: &str) -> Result<CommitSha, Box<dyn Error>> {
    Ok(CommitSha::try_from(text.to_owned())?)
}

fn candidate(text: &str, at: &str) -> Result<Target, Box<dyn Error>> {
    Ok(Target::Candidate {
        version: version(text)?,
        commit: commit(at)?,
    })
}

fn stable(text: &str, at: &str) -> Result<Target, Box<dyn Error>> {
    Ok(Target::Stable {
        version: version(text)?,
        commit: commit(at)?,
    })
}

/// The messages for `value` against `target`.
fn check(value: &Value, target: &Target, oracle: &Scripted) -> Result<Vec<String>, Box<dyn Error>> {
    Ok(check_completeness(&ledger(value)?, target, oracle))
}

fn assert_flagged(found: &[String], needle: &str) {
    assert!(
        found.iter().any(|message| message.contains(needle)),
        "expected a message containing {needle:?}, got {found:#?}"
    );
}

/// The baseline with RQ-01 passed for `rc.1` at the recorded commit, so both
/// required items are answered.
fn answered() -> Result<Value, Box<dyn Error>> {
    let mut value = baseline_value();
    set(&mut value, "/items/0/status", json!("passed"))?;
    insert(
        &mut value,
        "/items/0",
        "applies_to",
        json!({ "version": "0.2.0-rc.1", "commit": RECORDED_COMMIT }),
    )?;
    insert(
        &mut value,
        "/items/0",
        "evidence",
        json!([{
            "reference": { "type": "workflow_run", "workflow": "Hosted", "run_id": 1 },
            "date": "2026-10-05",
            "note": "All jobs green."
        }]),
    )?;
    Ok(value)
}

#[test]
fn a_candidate_with_every_required_item_passed_is_complete() -> Outcome {
    let found = check(
        &answered()?,
        &candidate("0.2.0-rc.1", RECORDED_COMMIT)?,
        &unchanged(),
    )?;
    assert!(found.is_empty(), "{found:#?}");
    Ok(())
}

#[test]
fn a_planned_running_or_failed_item_is_incomplete() -> Outcome {
    let target = candidate("0.2.0-rc.1", RECORDED_COMMIT)?;
    let found = check(&baseline_value(), &target, &unchanged())?;
    assert_flagged(&found, "RQ-01: is planned");
    assert_eq!(
        found.len(),
        1,
        "RQ-02 is passed and RQ-20 is not required: {found:#?}"
    );

    for status in ["running", "failed"] {
        let mut value = answered()?;
        set(&mut value, "/items/0/status", json!(status))?;
        let found = check(&value, &target, &unchanged())?;
        assert_flagged(&found, &format!("RQ-01: is {status}"));
    }
    Ok(())
}

#[test]
fn a_waived_or_not_applicable_item_is_complete() -> Outcome {
    let target = candidate("0.2.0-rc.1", RECORDED_COMMIT)?;
    let mut value = baseline_value();
    set(&mut value, "/items/0/status", json!("waived"))?;
    let found = check(&value, &target, &unchanged())?;
    assert!(found.is_empty(), "{found:#?}");

    set(&mut value, "/items/0/status", json!("not_applicable"))?;
    let found = check(&value, &target, &unchanged())?;
    assert!(found.is_empty(), "{found:#?}");
    Ok(())
}

#[test]
fn the_completeness_check_does_not_wait_for_itself() -> Outcome {
    let target = candidate("0.2.0-rc.1", RECORDED_COMMIT)?;
    let found = check(&answered()?, &target, &unchanged())?;
    assert!(
        !found.iter().any(|message| message.starts_with("RQ-20")),
        "{found:#?}"
    );
    Ok(())
}

#[test]
fn evidence_from_an_earlier_commit_counts_when_its_scope_did_not_change() -> Outcome {
    let found = check(
        &answered()?,
        &candidate("0.2.0-rc.2", LATER_COMMIT)?,
        &unchanged(),
    )?;
    assert!(found.is_empty(), "{found:#?}");
    Ok(())
}

#[test]
fn evidence_whose_scope_changed_does_not_count() -> Outcome {
    let oracle = Scripted(Ok(vec![String::from("crates/vsift/src/lib.rs")]));
    let found = check(
        &answered()?,
        &candidate("0.2.0-rc.2", LATER_COMMIT)?,
        &oracle,
    )?;
    assert_flagged(&found, "1 file(s) in its scope changed");
    assert_flagged(&found, "crates/vsift/src/lib.rs");
    assert_eq!(found.len(), 2, "both passed items are stale: {found:#?}");
    Ok(())
}

#[test]
fn evidence_whose_staleness_cannot_be_told_does_not_count() -> Outcome {
    let oracle = Scripted(Err(String::from("git failed: bad object")));
    let found = check(
        &answered()?,
        &candidate("0.2.0-rc.2", LATER_COMMIT)?,
        &oracle,
    )?;
    assert_flagged(&found, "cannot be told: git failed: bad object");
    Ok(())
}

#[test]
fn a_stable_version_is_not_a_candidate_target() -> Outcome {
    let found = check(
        &answered()?,
        &candidate("0.2.0", RECORDED_COMMIT)?,
        &unchanged(),
    )?;
    assert_flagged(&found, "is not a release candidate");
    Ok(())
}

/// The baseline answered for the candidate, with a recorded delta to the
/// stable `0.2.0` at the later commit and RQ-01 repeated for it.
fn stable_ready() -> Result<Value, Box<dyn Error>> {
    let mut value = answered()?;
    set(
        &mut value,
        "/release_delta",
        json!({
            "candidate_version": "0.2.0-rc.1",
            "candidate_commit": RECORDED_COMMIT,
            "stable_version": "0.2.0",
            "stable_commit": LATER_COMMIT,
            "verdict": "allowed",
            "check": { "type": "record", "path": "docs/planning/p14-qualification.md" },
            "date": "2026-11-01"
        }),
    )?;
    set(
        &mut value,
        "/items/0/applies_to",
        json!({ "version": "0.2.0", "commit": LATER_COMMIT }),
    )?;
    Ok(value)
}

#[test]
fn a_stable_with_repeated_items_and_an_allowed_delta_is_complete() -> Outcome {
    let found = check(
        &stable_ready()?,
        &stable("0.2.0", LATER_COMMIT)?,
        &unchanged(),
    )?;
    assert!(found.is_empty(), "{found:#?}");
    Ok(())
}

/// The record `vsift-release publish-plan` writes (`release-delta.json`), as
/// its own test pins it: the release tool and this check share one example, so
/// the stable check accepts exactly what the plan job writes.
#[test]
fn the_release_tools_delta_record_is_accepted_as_it_is() -> Outcome {
    let example = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../vsift-release/tests/release-delta.example.json"),
    )?;
    let mut value = stable_ready()?;
    set(
        &mut value,
        "/release_delta",
        serde_json::from_str(&example)?,
    )?;
    let found = check(&value, &stable("0.2.0", LATER_COMMIT)?, &unchanged())?;
    assert!(found.is_empty(), "{found:#?}");
    Ok(())
}
#[test]
fn a_stable_cannot_carry_without_a_recorded_delta() -> Outcome {
    let mut value = stable_ready()?;
    set(&mut value, "/release_delta", Value::Null)?;
    let found = check(&value, &stable("0.2.0", LATER_COMMIT)?, &unchanged())?;
    assert_flagged(&found, "delta is not recorded");
    assert_flagged(&found, "RQ-02: carries candidate evidence");
    Ok(())
}

#[test]
fn a_rejected_or_foreign_delta_does_not_allow_carrying() -> Outcome {
    let mut value = stable_ready()?;
    set(&mut value, "/release_delta/verdict", json!("rejected"))?;
    let found = check(&value, &stable("0.2.0", LATER_COMMIT)?, &unchanged())?;
    assert_flagged(
        &found,
        "the check of 2026-11-01 found that the stable release differs from 0.2.0-rc.1",
    );

    let mut value = stable_ready()?;
    set(&mut value, "/release_delta/stable_version", json!("0.3.0"))?;
    let found = check(&value, &stable("0.2.0", LATER_COMMIT)?, &unchanged())?;
    assert_flagged(&found, "release_delta is for 0.3.0");

    let found = check(
        &stable_ready()?,
        &stable("0.2.0", RECORDED_COMMIT)?,
        &unchanged(),
    )?;
    assert_flagged(&found, "release_delta is for 0.2.0 at");
    Ok(())
}

#[test]
fn a_repeated_item_needs_its_own_entry_for_the_stable() -> Outcome {
    let mut value = stable_ready()?;
    set(
        &mut value,
        "/items/0/applies_to",
        json!({ "version": "0.2.0-rc.1", "commit": RECORDED_COMMIT }),
    )?;
    let found = check(&value, &stable("0.2.0", LATER_COMMIT)?, &unchanged())?;
    assert_flagged(
        &found,
        "RQ-01: is passed for 0.2.0-rc.1, and this release needs its own entry",
    );

    set(&mut value, "/items/0/status", json!("planned"))?;
    set(&mut value, "/items/0/applies_to", Value::Null)?;
    set(&mut value, "/items/0/evidence", json!([]))?;
    let found = check(&value, &stable("0.2.0", LATER_COMMIT)?, &unchanged())?;
    assert_flagged(&found, "RQ-01: is planned");
    Ok(())
}

#[test]
fn a_candidate_version_is_not_a_stable_target() -> Outcome {
    let found = check(
        &stable_ready()?,
        &stable("0.2.0-rc.1", LATER_COMMIT)?,
        &unchanged(),
    )?;
    assert_flagged(&found, "is a release candidate, not a stable version");
    Ok(())
}

/// A throwaway repository removed when dropped.
struct TemporaryRepository(std::path::PathBuf);

impl Drop for TemporaryRepository {
    fn drop(&mut self) {
        // Best effort: a leftover folder in the temporary directory is harmless.
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Runs `git` in `directory` with a synthetic identity and no signing or
/// hooks, so the result does not depend on anyone's configuration.
fn git(directory: &Path, arguments: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=governance-test",
            "-c",
            "user.email=governance-test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.autocrlf=false",
        ])
        .args(arguments)
        .current_dir(directory)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

#[test]
fn the_git_oracle_reports_the_changed_files_of_a_scope() -> Outcome {
    let directory = std::env::temp_dir().join(format!(
        "vsift-governance-git-oracle-{}",
        std::process::id()
    ));
    // A folder an earlier, killed run left behind would make the first commit
    // empty; a missing folder is the normal case.
    let _ = fs::remove_dir_all(&directory);
    let guard = TemporaryRepository(directory.clone());
    fs::create_dir_all(directory.join("a"))?;
    fs::create_dir_all(directory.join("b"))?;
    git(&directory, &["init", "--quiet"])?;
    fs::write(directory.join("a/x.txt"), "one")?;
    fs::write(directory.join("b/y.txt"), "one")?;
    git(&directory, &["add", "."])?;
    git(
        &directory,
        &["commit", "--quiet", "--no-verify", "-m", "first"],
    )?;
    let oracle = GitOracle::new(&directory);
    let first = oracle.head()?;

    fs::write(directory.join("a/x.txt"), "two")?;
    git(&directory, &["add", "."])?;
    git(
        &directory,
        &["commit", "--quiet", "--no-verify", "-m", "second"],
    )?;
    let second = oracle.head()?;
    assert_ne!(first, second);

    let changed = |scope: &[&str]| {
        let scope: Vec<String> = scope.iter().map(|path| (*path).to_owned()).collect();
        oracle.changed_files(&first, &second, &scope)
    };
    assert_eq!(changed(&["a"])?, ["a/x.txt"]);
    assert_eq!(changed(&["."])?, ["a/x.txt"]);
    assert!(changed(&["b"])?.is_empty(), "b did not change");
    assert!(
        changed(&["a/y.txt"])?.is_empty(),
        "a path that is not there"
    );
    assert!(
        oracle
            .changed_files(&second, &second, &[String::from(".")])?
            .is_empty(),
        "a commit does not differ from itself"
    );
    assert!(
        changed(&["../outside"]).is_err(),
        "a path outside the repository is refused before git runs"
    );
    let unknown = commit("3333333333333333333333333333333333333333")?;
    assert!(
        oracle
            .changed_files(&unknown, &second, &[String::from(".")])
            .is_err(),
        "a commit git does not have cannot be compared"
    );
    drop(guard);
    Ok(())
}
