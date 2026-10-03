//! The campaign script's check of the checkout (#273).
//!
//! `run-campaign.ps1` writes each batch's state, freeze, records and summaries
//! inside the repository (`docs/planning/p14-agent-trials/`) and refuses to run
//! on a checkout with uncommitted changes, because the records name a committed
//! state. The first version counted its own state file as such a change, so a
//! real batch never started, and its dry run exited before the check and hid
//! that. These tests run the real script against a throwaway repository: they
//! spend nothing, call no client, no npm and no Docker (a real run is stopped by
//! a fake client that reports the wrong version, which the script checks after
//! the checkout and after it has written its state).
//!
//! The script is a Windows script, so the tests are too.

#![cfg(windows)]

mod common;

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use common::Scratch;
use serde_json::json;

type TestResult = Result<(), Box<dyn Error>>;

const SCRIPT: &str = "tools/vsift-agent-trials/campaigns/run-campaign.ps1";
const OUTPUT_TREE: &str = "docs/planning/p14-agent-trials";
/// Only the script's neutral-path rule reads this: nothing is created there.
const NEUTRAL_ROOT: &str = r"C:\p14-campaign-script-test-root";
/// The version the config pins and the fake client does not report.
const PINNED_CLAUDE: &str = "2.1.284";

/// A throwaway repository holding a copy of the real script and harness.
struct Checkout {
    scratch: Scratch,
    repository: PathBuf,
    config: PathBuf,
    outside: PathBuf,
}

impl Checkout {
    fn new() -> Result<Self, Box<dyn Error>> {
        let scratch = Scratch::new("campaign-script")?;
        let repository = scratch.path().join("repository");
        fs::create_dir(&repository)?;
        let outside = scratch.path().join("outside");
        fs::create_dir(&outside)?;

        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let script = repository.join(SCRIPT);
        fs::create_dir_all(script.parent().ok_or("script has no folder")?)?;
        fs::copy(crate_root.join("campaigns/run-campaign.ps1"), &script)?;

        // The harness the script looks for; `target/` is ignored, as in the
        // real checkout.
        let harness = repository.join("target/release/vsift-agent-trials.exe");
        fs::create_dir_all(harness.parent().ok_or("harness has no folder")?)?;
        fs::copy(env!("CARGO_BIN_EXE_vsift-agent-trials"), &harness)?;

        fs::write(repository.join(".gitignore"), "/target/\n")?;
        fs::write(repository.join("README.md"), "a throwaway checkout\n")?;
        let tree = repository.join(OUTPUT_TREE);
        fs::create_dir_all(&tree)?;
        fs::write(tree.join("README.md"), "the batches' folder\n")?;

        let fake_client = outside.join("fake-claude.cmd");
        fs::write(&fake_client, "@echo off\r\necho 0.0.1 (Fake Client)\r\n")?;
        let config = outside.join("campaign.json");
        fs::write(
            &config,
            serde_json::to_vec_pretty(&json!({
                "root": NEUTRAL_ROOT,
                "claudeExecutable": fake_client,
                "claudeVersion": PINNED_CLAUDE,
            }))?,
        )?;

        let checkout = Self {
            scratch,
            repository,
            config,
            outside,
        };
        checkout.git(&["init", "--quiet"])?;
        checkout.git(&["add", "--all"])?;
        checkout.git(&[
            "-c",
            "user.name=script test",
            "-c",
            "user.email=script-test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--message",
            "the checkout under test",
        ])?;
        Ok(checkout)
    }

    /// Runs Git in the throwaway repository, isolated from the machine's
    /// own Git configuration and hooks.
    fn git(&self, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.repository)
            .args(arguments)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.outside.join("no-such-gitconfig"))
            .output()?;
        assert!(output.status.success(), "git {arguments:?}: {output:?}");
        Ok(output)
    }

    /// Everything Git itself calls a change, with nothing exempted.
    fn raw_changes(&self) -> Result<String, Box<dyn Error>> {
        let output = self.git(&["status", "--porcelain"])?;
        Ok(String::from_utf8(output.stdout)?)
    }

    fn batch_directory(&self, batch: u8) -> PathBuf {
        self.repository
            .join(OUTPUT_TREE)
            .join(format!("batch-{batch}"))
    }

    fn state_file(&self, batch: u8) -> PathBuf {
        self.batch_directory(batch).join("state-claude.json")
    }

    /// Runs the script with PowerShell 7, as the runbook does.
    fn run(&self, batch: u8, extra: &[&str]) -> Result<Output, Box<dyn Error>> {
        let output = Command::new("pwsh")
            .args(["-NoProfile", "-NonInteractive", "-File"])
            .arg(self.repository.join(SCRIPT))
            .args(["-Batch", &batch.to_string()])
            .args(["-Client", "claude", "-Version", "0.1.0", "-Config"])
            .arg(&self.config)
            .arg("-NoBuild")
            .args(extra)
            .current_dir(&self.repository)
            // Git must not look for a repository above the scratch folder
            // (a test that removes `.git` would otherwise find a parent's).
            .env("GIT_CEILING_DIRECTORIES", self.scratch.path())
            .output()
            .map_err(|error| format!("PowerShell 7 (pwsh) must be installed: {error}"))?;
        Ok(output)
    }

    fn dry_run(&self, batch: u8) -> Result<Output, Box<dyn Error>> {
        self.run(batch, &["-DryRun"])
    }

    /// A real run, which the fake client stops at the version pin: after the
    /// checkout check and after the state file was written, and before
    /// anything that installs or spends.
    fn real_run(&self, batch: u8) -> Result<Output, Box<dyn Error>> {
        self.run(batch, &[])
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_dry_run_passed(output: &Output) {
    assert!(output.status.success(), "{}", text(output));
    assert!(text(output).contains("Dry run:"), "{}", text(output));
}

/// A real run reached the client-version pin, so the checkout check passed.
fn assert_stopped_at_the_version_pin(output: &Output) {
    let shown = text(output);
    assert!(!output.status.success(), "{shown}");
    assert!(shown.contains("the campaign pins"), "{shown}");
    assert!(!shown.contains("uncommitted changes"), "{shown}");
}

fn assert_refused_as_uncommitted(output: &Output, expected_path: &str) {
    let shown = text(output);
    assert!(!output.status.success(), "{shown}");
    assert!(shown.contains("uncommitted changes"), "{shown}");
    assert!(
        shown.contains(expected_path),
        "the refusal names the change: {shown}"
    );
}

#[test]
fn a_first_and_a_resumed_run_pass_with_the_campaigns_own_output_uncommitted() -> TestResult {
    let checkout = Checkout::new()?;
    assert!(
        checkout.raw_changes()?.is_empty(),
        "the fixture starts clean"
    );

    // The first real run writes the state file, then meets the version pin.
    // Before the fix it was refused on that file instead.
    assert_stopped_at_the_version_pin(&checkout.real_run(1)?);
    assert!(checkout.state_file(1).is_file(), "the plan was written");
    let raw = checkout.raw_changes()?;
    assert!(
        raw.contains("docs/planning/p14-agent-trials/"),
        "the test would not notice a lost exemption if Git saw nothing: {raw}"
    );

    // A resumed run: the state exists, uncommitted, and changes after every run.
    let state = fs::read_to_string(checkout.state_file(1))?;
    fs::write(checkout.state_file(1), format!("{state}\n"))?;
    assert_stopped_at_the_version_pin(&checkout.real_run(1)?);

    // The dry run takes the same check and passes the same way.
    assert_dry_run_passed(&checkout.dry_run(1)?);
    // Another batch beside the first batch's uncommitted output, and records.
    fs::create_dir_all(checkout.batch_directory(1).join("records"))?;
    fs::write(
        checkout
            .batch_directory(1)
            .join("records/a-trial-claude-p1.json"),
        "{}\n",
    )?;
    assert_dry_run_passed(&checkout.dry_run(2)?);
    assert_stopped_at_the_version_pin(&checkout.real_run(2)?);
    Ok(())
}

#[test]
fn any_other_uncommitted_change_is_refused_before_anything_is_written() -> TestResult {
    let checkout = Checkout::new()?;

    // A tracked file edited.
    fs::write(checkout.repository.join("README.md"), "edited\n")?;
    for refused in [checkout.dry_run(1)?, checkout.real_run(1)?] {
        assert_refused_as_uncommitted(&refused, "README.md");
    }
    assert!(
        !checkout.batch_directory(1).exists(),
        "the refusal came before the plan was written"
    );
    checkout.git(&["checkout", "--quiet", "--", "README.md"])?;

    // A file Git does not know, even beside the campaign's output.
    let stray = checkout.repository.join("stray.txt");
    fs::write(&stray, "not committed\n")?;
    assert_refused_as_uncommitted(&checkout.dry_run(1)?, "stray.txt");
    fs::remove_file(&stray)?;

    // A staged change.
    fs::write(checkout.repository.join("staged.txt"), "staged\n")?;
    checkout.git(&["add", "staged.txt"])?;
    assert_refused_as_uncommitted(&checkout.real_run(2)?, "staged.txt");
    assert!(!checkout.batch_directory(2).exists());
    checkout.git(&["rm", "--quiet", "--force", "staged.txt"])?;

    // Clean again: the same runs now go through.
    assert_dry_run_passed(&checkout.dry_run(1)?);
    assert_stopped_at_the_version_pin(&checkout.real_run(1)?);
    Ok(())
}

#[test]
fn an_unreadable_checkout_is_refused_not_taken_for_clean() -> TestResult {
    let checkout = Checkout::new()?;
    fs::remove_dir_all(checkout.repository.join(".git"))?;
    let output = checkout.dry_run(1)?;
    let shown = text(&output);
    assert!(!output.status.success(), "{shown}");
    assert!(shown.contains("cannot be shown to be clean"), "{shown}");
    assert!(!checkout.batch_directory(1).exists());
    Ok(())
}

#[test]
fn a_batch_directory_must_be_the_campaign_output_or_outside_the_checkout() -> TestResult {
    let checkout = Checkout::new()?;

    // Inside the checkout but not in the campaign's output tree: refused,
    // because exempting it would hide a real change and not exempting it
    // would make the script's own output count as one.
    let inside = checkout.repository.join("tools").join("elsewhere");
    let inside_argument = inside.to_string_lossy().into_owned();
    let refused = checkout.run(1, &["-DryRun", "-BatchDirectory", &inside_argument])?;
    let shown = text(&refused);
    assert!(!refused.status.success(), "{shown}");
    assert!(shown.contains("-BatchDirectory"), "{shown}");
    assert!(!inside.exists(), "nothing was written there");

    // Outside the checkout, anywhere: nothing it writes is a change.
    let outside = checkout.outside.join("batch-elsewhere");
    let outside_argument = outside.to_string_lossy().into_owned();
    let accepted = checkout.run(1, &["-DryRun", "-BatchDirectory", &outside_argument])?;
    assert_dry_run_passed(&accepted);
    assert!(outside.join("state-claude.json").is_file());
    assert!(checkout.raw_changes()?.is_empty());
    Ok(())
}
