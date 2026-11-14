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
//!
//! Since 2026-11-14 the script runs on capped API keys by default
//! (`-Credential ApiKey`). The checkout tests name `-Credential SignIn`, which
//! needs no key; the credential tests at the end use fake keys and fake
//! client homes, and check that no key value is ever printed.

#![cfg(windows)]

mod common;

use std::{
    collections::BTreeSet,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use common::Scratch;
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn Error>>;

const SCRIPT: &str = "tools/vsift-agent-trials/campaigns/run-campaign.ps1";
const OUTPUT_TREE: &str = "docs/planning/p14-agent-trials";
/// Only the script's neutral-path rule reads this: nothing is created there.
const NEUTRAL_ROOT: &str = r"C:\p14-campaign-script-test-root";
/// The version the config pins and the fake client does not report.
const PINNED_CLAUDE: &str = "2.1.284";
/// The user environment variable the script reads Claude Code's key from.
const CLAUDE_KEY_VARIABLE: &str = "VSIFT_TRIALS_ANTHROPIC_API_KEY";
/// A made-up key: long enough to look like one, and never a real one.
const FAKE_CLAUDE_KEY: &str = "sk-ant-api00-FAKE-campaign-script-test-0000000000";
/// A made-up OpenAI key for the fake Codex API-key login.
const FAKE_OPENAI_KEY: &str = "sk-proj-FAKE-campaign-script-test-0000000000000";

/// A throwaway repository holding a copy of the real script and harness.
struct Checkout {
    repository: PathBuf,
    config: PathBuf,
    outside: PathBuf,
    /// The Claude Code client home the configuration names (empty: no sign-in).
    claude_home: PathBuf,
    /// The Codex client home the configuration names (empty until a test
    /// writes its `auth.json`).
    codex_home: PathBuf,
    // Last: fields drop in order, and on Windows a folder something still
    // holds open cannot be removed (#423).
    scratch: Scratch,
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
        let claude_home = outside.join("claude-home");
        let codex_home = outside.join("codex-home");
        fs::create_dir(&claude_home)?;
        fs::create_dir(&codex_home)?;
        let config = outside.join("campaign.json");
        fs::write(
            &config,
            serde_json::to_vec_pretty(&json!({
                "root": NEUTRAL_ROOT,
                "clientHome": claude_home,
                "codexClientHome": codex_home,
                "claudeExecutable": fake_client,
                "claudeVersion": PINNED_CLAUDE,
            }))?,
        )?;

        let checkout = Self {
            repository,
            config,
            outside,
            claude_home,
            codex_home,
            scratch,
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
        self.run_as("claude", batch, extra)
    }

    /// [`Self::run`] for either client, on the sign-in path unless `extra`
    /// names a credential.
    fn run_as(&self, client: &str, batch: u8, extra: &[&str]) -> Result<Output, Box<dyn Error>> {
        self.run_with_key(client, batch, extra, None)
    }

    /// Runs the script with `claude_key` (or none) as the process's value of
    /// the key variable.
    fn run_with_key(
        &self,
        client: &str,
        batch: u8,
        extra: &[&str],
        claude_key: Option<&str>,
    ) -> Result<Output, Box<dyn Error>> {
        let credential: &[&str] = if extra.contains(&"-Credential") {
            &[]
        } else {
            &["-Credential", "SignIn"]
        };
        let mut command = Command::new("pwsh");
        command
            .args(["-NoProfile", "-NonInteractive", "-File"])
            .arg(self.repository.join(SCRIPT))
            .args(["-Batch", &batch.to_string()])
            .args(["-Client", client, "-Version", "0.1.0", "-Config"])
            .arg(&self.config)
            .arg("-NoBuild")
            .args(credential)
            .args(extra)
            .current_dir(&self.repository)
            // Git must not look for a repository above the scratch folder
            // (a test that removes `.git` would otherwise find a parent's).
            .env("GIT_CEILING_DIRECTORIES", self.scratch.path())
            .env_remove(CLAUDE_KEY_VARIABLE)
            .env_remove("ANTHROPIC_API_KEY");
        if let Some(key) = claude_key {
            command.env(CLAUDE_KEY_VARIABLE, key);
        }
        let output = command
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
    assert_eq!(checkout.raw_changes()?, "");
    Ok(())
}

/// The realistic cold setting lets the agent read any file the user can read
/// (an allow rule cannot confine `cat` to the workspace), so on the
/// maintainer's machine Claude Code keeps the strict one. The script refuses
/// the realistic one unless the operator states the machine is isolated, and
/// refuses it before it reads or writes anything.
#[test]
fn the_realistic_claude_cold_setting_needs_a_machine_declared_isolated() -> TestResult {
    let checkout = Checkout::new()?;
    let mentions = |output: &Output, words: &str| text(output).contains(words);

    // The default is the strict setting.
    let strict = checkout.dry_run(1)?;
    assert_dry_run_passed(&strict);
    assert!(
        mentions(&strict, "Cold setting: strict"),
        "{}",
        text(&strict)
    );

    // The realistic one is refused without the statement, in a dry run and a
    // real one, and nothing is written.
    for extra in [
        &["-DryRun", "-ColdVariant", "realistic"][..],
        &["-ColdVariant", "realistic"][..],
    ] {
        let refused = checkout.run(2, extra)?;
        assert!(!refused.status.success(), "{}", text(&refused));
        assert!(
            mentions(&refused, "isolated machine") && mentions(&refused, "-IsolatedMachine"),
            "{}",
            text(&refused)
        );
    }
    assert!(!checkout.batch_directory(2).exists(), "nothing was written");

    // With the statement it is accepted (a dry run: no client is called).
    let accepted = checkout.run(
        3,
        &["-DryRun", "-ColdVariant", "realistic", "-IsolatedMachine"],
    )?;
    assert_dry_run_passed(&accepted);
    assert!(
        mentions(&accepted, "Cold setting: realistic"),
        "{}",
        text(&accepted)
    );

    // The statement alone, or with the strict setting, means nothing.
    for extra in [
        &["-DryRun", "-IsolatedMachine"][..],
        &["-DryRun", "-ColdVariant", "strict", "-IsolatedMachine"][..],
    ] {
        let refused = checkout.run(1, extra)?;
        assert!(!refused.status.success(), "{}", text(&refused));
        assert!(mentions(&refused, "only goes with"), "{}", text(&refused));
    }
    Ok(())
}

/// Codex's cold run is always the realistic setting, inside the container.
#[test]
fn the_codex_cold_run_is_the_container_setting_and_has_no_strict_variant() -> TestResult {
    let checkout = Checkout::new()?;
    let codex = checkout.run_as("codex", 1, &["-DryRun"])?;
    assert_dry_run_passed(&codex);
    assert!(
        text(&codex).contains("Cold setting: realistic (the container)"),
        "{}",
        text(&codex)
    );
    // Naming the realistic setting for Codex is harmless; the strict one does
    // not exist for it.
    let named = checkout.run_as("codex", 1, &["-DryRun", "-ColdVariant", "realistic"])?;
    assert_dry_run_passed(&named);
    let strict = checkout.run_as("codex", 1, &["-DryRun", "-ColdVariant", "strict"])?;
    assert!(!strict.status.success(), "{}", text(&strict));
    assert!(
        text(&strict).contains("no strict Codex variant"),
        "{}",
        text(&strict)
    );
    // And the machine statement is for Claude Code only.
    let declared = checkout.run_as("codex", 1, &["-DryRun", "-IsolatedMachine"])?;
    assert!(!declared.status.success(), "{}", text(&declared));
    Ok(())
}

impl Checkout {
    /// The harness copy the script runs, as the script runs it.
    fn harness(&self, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
        Ok(Command::new(
            self.repository
                .join("target/release/vsift-agent-trials.exe"),
        )
        .args(arguments)
        .output()?)
    }

    /// Counts the next run of `client`'s plan in `batch`, as a finished trial
    /// would, and writes the record it names (just enough for a summary).
    fn count_next_run(&self, client: &str, batch: u8) -> Result<(), Box<dyn Error>> {
        let state = self
            .batch_directory(batch)
            .join(format!("state-{client}.json"));
        let state_argument = state.to_string_lossy().into_owned();
        let next = self.harness(&["campaign", "next", "--state", &state_argument])?;
        assert!(next.status.success(), "{}", text(&next));
        let run: Value = serde_json::from_slice(&next.stdout)?;
        let run_id = run["run_id"].as_str().ok_or("no run id")?;
        let trial = format!("trial-{client}-{batch}");
        let marked = self.harness(&[
            "campaign",
            "mark",
            "--state",
            &state_argument,
            "--run",
            run_id,
            "--outcome",
            "counted",
            "--trial",
            &trial,
        ])?;
        assert!(marked.status.success(), "{}", text(&marked));
        let records = self.batch_directory(batch).join("records");
        fs::create_dir_all(&records)?;
        fs::write(
            records.join(format!("{trial}-p1.json")),
            serde_json::to_vec(&json!({ "trial_id": trial, "valid": true }))?,
        )?;
        Ok(())
    }
}

/// The clients of the runs a batch's summary lists.
fn summary_clients(checkout: &Checkout, batch: u8) -> Result<BTreeSet<String>, Box<dyn Error>> {
    let summary: Value = serde_json::from_slice(&fs::read(
        checkout.batch_directory(batch).join("summary.json"),
    )?)?;
    Ok(summary["runs"]
        .as_array()
        .ok_or("the summary lists no runs")?
        .iter()
        .filter_map(|run| run["client"].as_str().map(str::to_owned))
        .collect())
}

/// #282: the batch's summary is rewritten after every counted run, and used to
/// be made from the running client's plan alone, so the second client's run
/// replaced the first client's summary with one that had none of its runs.
/// The summary now covers every plan the batch folder holds, whichever client
/// writes it.
#[test]
fn the_summary_covers_every_client_of_the_batch_whichever_one_writes_it() -> TestResult {
    let checkout = Checkout::new()?;
    // Each client's plan: the real script writes one before it stops (Claude
    // at the version pin) and so does a dry run (Codex).
    assert_stopped_at_the_version_pin(&checkout.real_run(1)?);
    assert_dry_run_passed(&checkout.run_as("codex", 1, &["-DryRun"])?);
    checkout.count_next_run("claude", 1)?;
    checkout.count_next_run("codex", 1)?;
    let both: BTreeSet<String> = ["claude", "codex"].map(str::to_owned).into();

    for writer in ["codex", "claude"] {
        let summary = checkout.run_as(writer, 1, &["-SummaryOnly"])?;
        assert!(summary.status.success(), "{}", text(&summary));
        assert_eq!(
            summary_clients(&checkout, 1)?,
            both,
            "the summary written by the {writer} client lost the other client's runs"
        );
    }
    Ok(())
}

#[test]
fn a_summary_only_run_calls_no_client_and_plans_nothing() -> TestResult {
    let checkout = Checkout::new()?;
    // No batch folder: nothing to summarise, and nothing is made.
    let refused = checkout.run(3, &["-SummaryOnly"])?;
    assert!(!refused.status.success(), "{}", text(&refused));
    assert!(
        text(&refused).contains("nothing to summarise"),
        "{}",
        text(&refused)
    );
    assert!(!checkout.batch_directory(3).exists());

    // A folder with one client's plan: the other client gets none.
    assert_stopped_at_the_version_pin(&checkout.real_run(1)?);
    let summary = checkout.run_as("codex", 1, &["-SummaryOnly"])?;
    assert!(summary.status.success(), "{}", text(&summary));
    assert!(
        !checkout
            .batch_directory(1)
            .join("state-codex.json")
            .exists()
    );
    assert!(checkout.batch_directory(1).join("summary.json").is_file());
    Ok(())
}

/// Asserts that no fake key value appears in the script's output.
fn assert_no_key_shown(output: &Output) {
    let shown = text(output);
    for key in [FAKE_CLAUDE_KEY, FAKE_OPENAI_KEY] {
        assert!(!shown.contains(key), "a key value was printed: {shown}");
    }
}

/// Whether the user's own environment holds the key variable. Only `reg`'s
/// exit status is read: its output, which would show the value, goes
/// nowhere.
fn user_environment_holds_the_key_variable() -> Result<bool, Box<dyn Error>> {
    let status = Command::new("reg")
        .args(["query", r"HKCU\Environment", "/v", CLAUDE_KEY_VARIABLE])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(status.success())
}

/// The default credential is the capped API key, and a Claude Code batch on it
/// starts only with the key variable set and a client home that holds no
/// sign-in, so a run cannot fall back to a person's sign-in. Both refusals
/// come before the plan is written, in a dry run too, and the key's value is
/// never printed.
#[test]
fn a_claude_batch_on_the_api_key_needs_the_key_and_a_home_without_a_sign_in() -> TestResult {
    let checkout = Checkout::new()?;
    let api_key = ["-Credential", "ApiKey"];

    // With the key and a clean home the dry run passes and names the
    // credential it checked, and a real run reaches the client-version pin.
    let accepted = checkout.run_with_key(
        "claude",
        2,
        &["-DryRun", "-Credential", "ApiKey"],
        Some(FAKE_CLAUDE_KEY),
    )?;
    assert_dry_run_passed(&accepted);
    assert!(
        text(&accepted).contains("Credential: the API key in VSIFT_TRIALS_ANTHROPIC_API_KEY"),
        "{}",
        text(&accepted)
    );
    assert_no_key_shown(&accepted);
    let real = checkout.run_with_key("claude", 2, &api_key, Some(FAKE_CLAUDE_KEY))?;
    assert_stopped_at_the_version_pin(&real);
    assert_no_key_shown(&real);

    // A sign-in in the client home is refused, whatever the key.
    fs::write(
        checkout.claude_home.join(".credentials.json"),
        r#"{"claudeAiOauth":{"accessToken":"fake"}}"#,
    )?;
    for extra in [&["-DryRun", "-Credential", "ApiKey"][..], &api_key[..]] {
        let refused = checkout.run_with_key("claude", 3, extra, Some(FAKE_CLAUDE_KEY))?;
        assert!(!refused.status.success(), "{}", text(&refused));
        assert!(
            text(&refused).contains("holds a sign-in"),
            "{}",
            text(&refused)
        );
        assert_no_key_shown(&refused);
    }
    assert!(!checkout.batch_directory(3).exists(), "nothing was written");
    fs::remove_file(checkout.claude_home.join(".credentials.json"))?;

    // No key: refused, naming the variable. The script also reads the user's
    // own environment, so this holds only where that does not set it.
    if user_environment_holds_the_key_variable()? {
        eprintln!("skipped the missing-key case: the user environment sets {CLAUDE_KEY_VARIABLE}");
    } else {
        let refused =
            checkout.run_with_key("claude", 3, &["-DryRun", "-Credential", "ApiKey"], None)?;
        assert!(!refused.status.success(), "{}", text(&refused));
        assert!(
            text(&refused).contains("No Claude API key")
                && text(&refused).contains(CLAUDE_KEY_VARIABLE),
            "{}",
            text(&refused)
        );
        assert!(!checkout.batch_directory(3).exists(), "nothing was written");
    }

    // The sign-in path needs neither.
    let signed_in =
        checkout.run_with_key("claude", 3, &["-DryRun", "-Credential", "SignIn"], None)?;
    assert_dry_run_passed(&signed_in);
    assert!(
        text(&signed_in).contains("Credential: the client homes' own sign-ins"),
        "{}",
        text(&signed_in)
    );
    Ok(())
}

/// The API key is the default: a run that names no credential is refused
/// for a missing key, never run on a sign-in.
#[test]
fn the_api_key_is_the_default_credential() -> TestResult {
    let checkout = Checkout::new()?;
    let output = Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-File"])
        .arg(checkout.repository.join(SCRIPT))
        .args([
            "-Batch", "2", "-Client", "claude", "-Version", "0.1.0", "-Config",
        ])
        .arg(&checkout.config)
        .args(["-NoBuild", "-DryRun"])
        .current_dir(&checkout.repository)
        .env("GIT_CEILING_DIRECTORIES", checkout.scratch.path())
        .env(CLAUDE_KEY_VARIABLE, FAKE_CLAUDE_KEY)
        .output()?;
    assert_dry_run_passed(&output);
    assert!(
        text(&output).contains("Credential: the API key in VSIFT_TRIALS_ANTHROPIC_API_KEY"),
        "{}",
        text(&output)
    );
    assert_no_key_shown(&output);
    Ok(())
}

/// A Codex batch on the API key needs a trial home whose `auth.json` is an
/// API-key login only (`codex login --with-api-key`): a ChatGPT sign-in, or a
/// login without a key, is refused before the plan is written.
#[test]
fn a_codex_batch_on_the_api_key_needs_an_api_key_login_only() -> TestResult {
    let checkout = Checkout::new()?;
    let auth = checkout.codex_home.join("auth.json");
    let api_key = ["-DryRun", "-Credential", "ApiKey"];

    let missing = checkout.run_as("codex", 2, &api_key)?;
    assert!(!missing.status.success(), "{}", text(&missing));
    assert!(
        text(&missing).contains("No Codex API-key login"),
        "{}",
        text(&missing)
    );

    for refused_login in [
        json!({"OPENAI_API_KEY": null, "tokens": {"id_token": "fake", "access_token": "fake"}}),
        json!({"auth_mode": "apikey", "OPENAI_API_KEY": "", "tokens": null}),
    ] {
        fs::write(&auth, serde_json::to_vec(&refused_login)?)?;
        let refused = checkout.run_as("codex", 2, &api_key)?;
        assert!(!refused.status.success(), "{}", text(&refused));
        assert!(
            text(&refused).contains("not an API-key login only"),
            "{}",
            text(&refused)
        );
    }
    assert!(!checkout.batch_directory(2).exists(), "nothing was written");

    fs::write(
        &auth,
        serde_json::to_vec(&json!({
            "auth_mode": "apikey", "OPENAI_API_KEY": FAKE_OPENAI_KEY,
            "tokens": null, "last_refresh": null
        }))?,
    )?;
    let accepted = checkout.run_as("codex", 2, &api_key)?;
    assert_dry_run_passed(&accepted);
    assert!(
        text(&accepted).contains("Credential: the API-key login in the Codex trial home"),
        "{}",
        text(&accepted)
    );
    assert_no_key_shown(&accepted);
    Ok(())
}
