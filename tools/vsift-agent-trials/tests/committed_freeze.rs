//! The committed freezes of the release candidate's agent-trial batches (P14
//! PR 10b).
//!
//! Batches 2 and 3 run against the published candidate, with the skill, the
//! grader, the scenarios, the settings and the corpus truth frozen at the
//! candidate's cut. The freeze is written once, at that commit, and committed
//! (`batch-2/freeze.json`, `batch-3/freeze.json`), so that the cut itself
//! records what the trials are bound to and the campaign script uses the
//! committed file instead of writing a fresh one at the batch's start. This
//! test is the mechanical half: a change to any frozen component after the cut
//! fails the Quality jobs of every pull request until the maintainer decides,
//! on purpose, to cut another candidate and write a new freeze.
//!
//! Batch 1's freeze (the baseline on 0.1.0) is history: the skill and the
//! grader changed after it, as the plan allows between batches, and it is not
//! checked here.

mod common;

use std::error::Error;

use common::repository;
use vsift_agent_trials::freeze;

type TestResult = Result<(), Box<dyn Error>>;

/// The batches of the candidate that carry a committed freeze.
const BATCHES: [u32; 2] = [2, 3];

fn freeze_file(batch: u32) -> std::path::PathBuf {
    repository()
        .join("docs")
        .join("planning")
        .join("p14-agent-trials")
        .join(format!("batch-{batch}"))
        .join("freeze.json")
}

#[test]
fn the_committed_freezes_still_hold() -> TestResult {
    for batch in BATCHES {
        let file = freeze_file(batch);
        assert!(file.is_file(), "batch {batch} has no committed freeze");
        let problems = freeze::check(&repository(), &file, None)?;
        assert_eq!(
            problems,
            Vec::<String>::new(),
            "batch {batch}: something frozen at the candidate's cut changed; a change to the \
             skill, the grader, the scenarios, the settings or the truth needs a second candidate \
             and a new `freeze write` (docs/operations/release.md section 6.8)"
        );
    }
    Ok(())
}

/// Both batches test the same candidate, so they are bound to the same
/// frozen state: the two files differ at most by nothing.
#[test]
fn both_batches_are_bound_to_the_same_frozen_state() -> TestResult {
    let two = freeze::load(&freeze_file(2))?;
    let three = freeze::load(&freeze_file(3))?;
    assert_eq!(two, three);
    assert_eq!(two.components.len(), freeze::COMPONENTS.len());
    Ok(())
}
