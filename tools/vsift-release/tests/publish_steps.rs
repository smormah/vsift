//! Runs the Release workflow's publishing shell against stub commands (P14
//! PR 8, `docs/operations/release.md` section 6.9).
//!
//! `publish-steps.sh` extracts the script of the privileged `publish` job's
//! steps and of the `plan` job's registry step from `.github/workflows/
//! release.yml` itself and runs them with stub `npm`, `gh`, `curl` and
//! `sleep`, so the channel logic that decides which dist-tag moves is executed,
//! not only read: a pre-release moves `next` and never `latest`, a stable
//! version moves `latest` and never `next`, each step refuses the other
//! channel's version before anything is published, `latest` never moves
//! backwards, a partial stable publish is completed by a re-run, and a
//! registry that cannot be reached never fails the plan job. No real service
//! is contacted and nothing is published.
//!
//! The script needs GNU coreutils and `base64 -w`, which the publish job's own
//! script needs too (it runs on Ubuntu), so the test runs on Linux; on another
//! system run the script by hand in a POSIX shell with those tools (Git Bash
//! on Windows works).

#![cfg(target_os = "linux")]

use std::{error::Error, path::Path, process::Command};

#[test]
fn the_publish_jobs_shell_moves_one_dist_tag_and_refuses_the_other_channel()
-> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new("bash")
        .arg(root.join("tests/publish-steps.sh"))
        .arg(root.join("../../.github/workflows/release.yml"))
        .output()?;
    let report = String::from_utf8_lossy(&output.stdout);
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "publish-steps.sh failed:\n{report}\n{errors}"
    );
    assert!(report.contains("failed 0"), "{report}");
    assert!(!report.contains("FAIL"), "{report}");
    Ok(())
}
