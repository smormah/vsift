//! The cold-workspace assertions: a cold agent sees the CLI and nothing
//! else. Every discovery location a client reads is checked, in the
//! workspace and in every folder above it.

mod common;

use std::{error::Error, fs};

use common::Scratch;
use vsift_agent_trials::cold::assert_cold_workspace;

type TestResult = Result<(), Box<dyn Error>>;

/// A workspace that is cold: the video, a transcript, the allow rule.
fn cold(scratch: &Scratch) -> Result<std::path::PathBuf, Box<dyn Error>> {
    scratch.write(
        "trial/workspace/.claude/settings.json",
        "{\"permissions\": {\"allow\": [\"Bash(vsift:*)\"]}}\n",
    )?;
    scratch.write(
        "trial/workspace/walkthrough.srt",
        "1\n00:00:00,000 --> 00:00:01,000\nHello\n",
    )?;
    scratch.write(
        "trial/workspace/walkthrough.mp4",
        "VSift appears in these bytes, which are media\n",
    )?;
    scratch.write("trial/workspace/.env", "SERVICE_API_KEY=SVC-CANARY-1\n")?;
    scratch.write(
        "trial/workspace/.home/.config/vsift/config.json",
        "{\"vsift\": true}\n",
    )?;
    Ok(scratch.path().join("trial").join("workspace"))
}

fn refusal(
    scratch: &Scratch,
    workspace: &std::path::Path,
    homes: &[&std::path::Path],
) -> Result<String, Box<dyn Error>> {
    let parent = scratch.parent();
    match assert_cold_workspace(workspace, parent.as_deref(), homes) {
        Ok(proved) => Err(format!("a polluted workspace was accepted: {proved:?}").into()),
        Err(error) => Ok(error.to_string()),
    }
}

#[test]
fn a_clean_workspace_passes_and_says_what_was_proved() -> TestResult {
    let scratch = Scratch::new("cold-clean")?;
    let workspace = cold(&scratch)?;
    let home = scratch.path().join("home");
    fs::create_dir_all(&home)?;
    fs::write(home.join(".claude.json"), "{}")?;
    let parent = scratch.parent();
    let proved = assert_cold_workspace(&workspace, parent.as_deref(), &[home.as_path()])?;
    assert_eq!(proved.len(), 6, "{proved:?}");
    // The allow rule in the settings and the media bytes do not count; the
    // per-user base is not scanned.
    Ok(())
}

#[test]
fn a_skill_command_agent_or_plugin_folder_anywhere_above_is_refused() -> TestResult {
    for folder in [
        ".claude/skills/vsift",
        ".claude/commands",
        ".claude/agents",
        ".claude/plugins",
        ".agents/skills/vsift",
        ".codex",
        "skills/vsift",
    ] {
        // In the workspace itself.
        let scratch = Scratch::new("cold-skill-here")?;
        let workspace = cold(&scratch)?;
        fs::create_dir_all(workspace.join(folder))?;
        let message = refusal(&scratch, &workspace, &[])?;
        assert!(message.contains("exists in"), "{folder}: {message}");

        // In the trial folder, and in the root above it.
        for above in ["trial", ""] {
            let scratch = Scratch::new("cold-skill-above")?;
            let workspace = cold(&scratch)?;
            fs::create_dir_all(scratch.path().join(above).join(folder))?;
            let message = refusal(&scratch, &workspace, &[])?;
            assert!(
                message.contains(folder.split('/').next_back().unwrap_or(folder))
                    || message.contains("exists in"),
                "{folder} {above}: {message}"
            );
        }
    }
    Ok(())
}

#[test]
fn an_instruction_file_that_mentions_the_tool_is_refused_wherever_a_client_reads_it() -> TestResult
{
    for (relative, text) in [
        ("trial/workspace/AGENTS.md", "Run vsift for video.\n"),
        ("trial/workspace/CLAUDE.md", "The VSift CLI is installed.\n"),
        ("trial/CLAUDE.md", "use VSIFT\n"),
        ("AGENTS.md", "vsift handoff\n"),
        ("trial/workspace/AGENTS.override.md", "vsift\n"),
        ("trial/workspace/CLAUDE.local.md", "vsift\n"),
    ] {
        let scratch = Scratch::new("cold-instructions")?;
        let workspace = cold(&scratch)?;
        scratch.write(relative, text)?;
        let message = refusal(&scratch, &workspace, &[])?;
        assert!(message.contains("mentions VSift"), "{relative}: {message}");
    }
    // An instruction file that does not mention it is not this check's
    // business.
    let scratch = Scratch::new("cold-instructions-other")?;
    let workspace = cold(&scratch)?;
    scratch.write("trial/CLAUDE.md", "Prefer small commits.\n")?;
    let parent = scratch.parent();
    assert_cold_workspace(&workspace, parent.as_deref(), &[])?;
    Ok(())
}

#[test]
fn a_repository_checkout_above_the_workspace_is_refused() -> TestResult {
    for mark in [
        ".git/HEAD",
        "fixtures/corpus/manifest.json",
        "docs/agents/skill.md",
    ] {
        let scratch = Scratch::new("cold-repository")?;
        let workspace = cold(&scratch)?;
        scratch.write(mark, "x")?;
        let message = refusal(&scratch, &workspace, &[])?;
        assert!(message.contains("repository mark"), "{mark}: {message}");
    }
    Ok(())
}

#[test]
fn a_file_that_names_the_tool_or_a_stray_claude_file_is_refused() -> TestResult {
    let scratch = Scratch::new("cold-readme")?;
    let workspace = cold(&scratch)?;
    scratch.write("trial/workspace/README.md", "This folder is for VSift.\n")?;
    let message = refusal(&scratch, &workspace, &[])?;
    assert!(message.contains("README.md mentions VSift"), "{message}");

    let scratch = Scratch::new("cold-stray")?;
    let workspace = cold(&scratch)?;
    scratch.write("trial/workspace/.claude/settings.local.json", "{}")?;
    let message = refusal(&scratch, &workspace, &[])?;
    assert!(
        message.contains(".claude holds settings.local.json"),
        "{message}"
    );
    Ok(())
}

#[test]
fn a_client_home_with_a_skill_a_rule_or_a_memory_file_is_refused() -> TestResult {
    for name in [
        "skills",
        "commands",
        "agents",
        "plugins",
        "prompts",
        "rules",
        "CLAUDE.md",
        "AGENTS.md",
    ] {
        let scratch = Scratch::new("cold-home")?;
        let workspace = cold(&scratch)?;
        let home = scratch.path().join("home");
        fs::create_dir_all(home.join(name))?;
        let message = refusal(&scratch, &workspace, &[home.as_path()])?;
        assert!(
            message.contains(&format!("the client home holds {name}")),
            "{name}: {message}"
        );
    }
    Ok(())
}

#[test]
fn every_problem_is_listed_not_only_the_first() -> TestResult {
    let scratch = Scratch::new("cold-all")?;
    let workspace = cold(&scratch)?;
    fs::create_dir_all(workspace.join(".claude").join("skills"))?;
    scratch.write("trial/workspace/AGENTS.md", "vsift\n")?;
    scratch.write(".git/HEAD", "x")?;
    let message = refusal(&scratch, &workspace, &[])?;
    for part in [".claude/skills", "AGENTS.md", ".git"] {
        assert!(message.contains(part), "{part}: {message}");
    }
    Ok(())
}
