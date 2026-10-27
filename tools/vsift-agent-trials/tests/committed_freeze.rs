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

/// The whole-freeze digest of the candidate's cut: `freeze_sha256` of both committed files, copied from them.
///
/// `the_committed_freezes_still_hold` passes if a pull request edits a frozen component and regenerates
/// `freeze.json` in the same change, because the file then agrees with the tree. Pinning the digest here makes that
/// visible: changing this constant is the decision to cut another candidate and write a new freeze
/// (`docs/operations/release.md` section 6.8), and it shows in the diff of a test, not only of a data file.
///
/// **Draft freeze, rewritten at the merge rebase** (W6g, ADR 0037, 2026-10-27): `references/commands.md` classes
/// `vsift watch` as never, so the `skill` component moved `abbc2e96...` to `d2b52131...` and this digest `6b2744db...`
/// to the value below; the other six components are unchanged.
///
/// The freeze of 2026-10-27 is the **0.3.0 development freeze; the final freeze is recorded at the 0.3.0 cut**. The
/// progress story's fifth pull request changed the skill again (`references/resume.md` starts each of several
/// recordings, or one with no rush, with `--mode queued` and reports a waiting job's `queue_position`;
/// `references/commands.md` allows the form), so the `skill` component moved `bef93e2b...` to `abbc2e96...` and this
/// digest `2cda9657...` to the value below; the other six components are unchanged. It was written on the `main`
/// the pull request was rebased onto to merge. The freeze before it, also of 2026-10-27: the progress story's fourth pull request changed the skill again (PREPARE adds `--first` when the user named a moment,
/// `references/resume.md` says `candidates` answers without analysing once the `visual` stream has succeeded,
/// `references/commands.md` allows the `--first` form), so the `skill` component moved `e9158295...` to `bef93e2b...`
/// and this digest `ac04d1f2...` to the value below; the other six components are unchanged. It was written on the
/// `main` the pull request was rebased onto to merge. The freeze of 2026-10-26 is the one before it: the
/// progress story's third pull request changed the skill (PREPARE starts the transcription with `job start`,
/// `references/resume.md` follows one in the background, `references/commands.md` classes `job start`), so the
/// `skill` component moved `e9fb4dc1...` to `e9158295...` and this digest `6d42bc6f...` to the value below; the other
/// six components are unchanged. It was written on the `main` the pull request was rebased onto to merge, and later
/// 0.3.0 changes to the skill move it again. The freeze of 2026-10-25 (the OCR pull request, `commands.md` names
/// `ocr`, `bd205d6c...` to `e9fb4dc1...`, digest `30293090...` to `6d42bc6f...`) is the one before it. The batches
/// for `0.2.3` run from `release/0.2.x` under `30293090...`. No batch has run under it.
///
/// The freeze of 2026-10-19 is new on purpose (#468, the toolchain re-pin to Rust 1.99.1), and this time the
/// `grader` component changed, `57507fad...` to `9ecd1121...`: the new pedantic lint `clippy::assert_is_empty`
/// fires in the test modules of the grader's `src`, and the maintainer decided on 2026-10-19 to rewrite every
/// site (`assert!(x.is_empty())` to `assert_eq!` against an empty value) and accept a new grader freeze. No
/// grading code changed. This digest moved `10180cd8...` to the value below; the other six components are
/// unchanged. The seventh candidate's batches ran under `10180cd8...`, and that file is kept with their records
/// as `freeze.json` in `batch-2-0.2.2-rc.1/` and `batch-3-0.2.2-rc.1/` (the cut of `0.2.3-rc.1` moved it there). The eighth
/// candidate keeps this freeze. No batch has run under it.
///
/// The freeze of 2026-10-17 is new on purpose (#367), written on top of #366's: the skill shortlists the screens
/// of a range with no transcript with `candidates --omit periodic` (`docs/agents/skill.md`, "Shortlisting the screens
/// of a long recording"), so the `skill` component changed, `13b485f3...` to `bd205d6c...`, and this digest,
/// `cfaf9783...` to the value below. The other six components are unchanged. No batch has run under it.
///
/// The second freeze of 2026-10-17 is new on purpose too (#366): the skill checks a long report whole through
/// the PowerShell form on Windows and in parts in Git Bash (`docs/agents/skill.md`, "A long report's check on
/// Windows"), so the `skill` component changed, `7f445a9a...` to `13b485f3...`, and this digest, `f52265e7...` to the
/// value below. The other six components are unchanged. No batch has run under it.
///
/// The freeze of 2026-10-17 is new on purpose, in the `0.2.2` cycle rather than at a cut: the skill's "search
/// first" rule became one that always applies (issue #395; `docs/agents/skill.md`, "The change for 0.2.2"), so the
/// `skill` component changed, `256b4840...` to `7f445a9a...`, and this digest, `a48567ac...` to the value below. The
/// other six components are the fifth candidate's, byte for byte. It binds the next counted batch; no batch has run
/// under it. The sixth candidate's batches ran under `a48567ac...`, and that file is kept beside their records as
/// `freeze-0.2.1-rc.3.json` in both folders.
///
/// The fifth candidate's freeze is new on purpose (`0.2.1-rc.2`, 2026-10-11): the agent skill changed again
/// for the patch release `0.2.1` (the brief view, the coverage of a read, five-minute `candidates` ranges,
/// `BUSY` while a transcription runs, the newest ids after a range retranscription, a long report checked in
/// two parts, `session list` paging and whole recordings out of scope: `docs/agents/skill.md`, "Since the first
/// candidate"), so the `skill` component and with it this digest changed, `2f8686b7...` to `256b4840...` and
/// `dbc4c22c...` to the value below. The other six components are the ones the third and fourth candidates were
/// frozen with, byte for byte. The fourth candidate's whole-freeze digest was
/// `dbc4c22cac4324dc24040b597fff4da2e43011b833af8a142c970cc26fb98c9e`; no batch ran under it, so no copy of
/// that file was kept beside records, and it stays in the history of `main` (`0.2.1-rc.1`, `cb8cb3a`). The third
/// candidate's was `654955dd210eae2707b15a5334a3390edca9b7e17e300ebe68310f4b815ba5c6` (a copy of that file is kept
/// in `batch-2-rc.3/` and `batch-3-rc.3/`); the first two candidates' was
/// `1e89b5cc488e7245d1a6d63ec8809c1f8a5c137ee87f5ed05f9b692c2af6e392` (copies in `batch-2-rc.1/` and
/// `batch-2-rc.2/`).
const CUT_FREEZE_SHA256: &str = "403e0c6a4d8cd2646b3f69941a93e42e5e5a5b9080534c0a335c13d3e6565bae";

#[test]
fn the_committed_freezes_are_the_ones_written_at_the_cut() -> TestResult {
    for batch in BATCHES {
        let recorded = freeze::load(&freeze_file(batch))?;
        assert_eq!(
            recorded.freeze_sha256, CUT_FREEZE_SHA256,
            "batch {batch}: the freeze file is not the one written at the candidate's cut; a new freeze means a new \
             candidate (docs/operations/release.md section 6.8), and this constant changes with it"
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
