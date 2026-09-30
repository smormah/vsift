//! Renderer tests over the frozen v1 examples, and the golden snapshots
//! that let a reviewer read what each command prints.
//!
//! The snapshots in `crates/vsift-cli/tests/human_output/` are for
//! readability review only; human text is not a contract. Set
//! `VSIFT_UPDATE_HUMAN_SNAPSHOTS=1` to rewrite them after a deliberate
//! change, then review the diff.

use std::{env, fs, path::PathBuf};

use serde_json::{Value, json};
use vsift::RuntimeDependency;
use vsift_contract::{
    CommandName, ConfiguredModelResponse, ConfiguredSelectionResponse, OperationResponse,
    is_hidden_character,
};

use super::{HumanDetail, failure::render_failure, render_value, result, text::DisplayText};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn example(name: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(repository(
        &format!("schemas/v1/examples/{name}"),
    ))?)?)
}

/// Compares `text` with the snapshot `name`, or rewrites it on request.
pub(crate) fn check_snapshot(name: &str, text: &str) -> TestResult {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/human_output")
        .join(format!("{name}.txt"));
    if env::var_os("VSIFT_UPDATE_HUMAN_SNAPSHOTS").is_some() {
        fs::write(&path, text)?;
        return Ok(());
    }
    let expected = fs::read_to_string(&path)
        .map_err(|error| {
            format!("snapshot {name} unreadable ({error}); set VSIFT_UPDATE_HUMAN_SNAPSHOTS=1")
        })?
        .replace("\r\n", "\n");
    assert_eq!(text, expected, "snapshot {name} differs");
    Ok(())
}

/// Asserts the terminal rules on `text`: no control character but line
/// breaks, no hidden character, and no line over the diagnostic budget.
fn assert_terminal_safe(text: &str) {
    for character in text.chars() {
        assert!(
            character == '\n' || !character.is_control(),
            "control U+{:04X}",
            u32::from(character)
        );
        assert!(
            !is_hidden_character(character),
            "hidden U+{:04X}",
            u32::from(character)
        );
    }
    for line in text.lines() {
        assert!(line.len() < 4_096, "a line of {} bytes", line.len());
    }
}

fn render(command: CommandName, value: &Value) -> Result<String, Box<dyn std::error::Error>> {
    let rendered = render_value(command, value)?.ok_or("no renderer")?;
    let text = rendered.as_str().to_owned();
    assert_terminal_safe(&text);
    Ok(text)
}

/// Every frozen example of a PR 2a command renders; its snapshot is the
/// text a person reads.
#[test]
fn frozen_examples_render_as_readable_text() -> TestResult {
    for (command, file, snapshot) in [
        (CommandName::Ingest, "ingest.transcript.json", "ingest"),
        (
            CommandName::TranscriptGet,
            "transcript-get.json",
            "transcript-get",
        ),
        (
            CommandName::TranscriptGet,
            "transcript-get.asr.json",
            "transcript-get-asr",
        ),
        (
            CommandName::TranscriptRetranscribe,
            "transcript-retranscribe.json",
            "transcript-retranscribe",
        ),
        (CommandName::Search, "search.json", "search"),
        (
            CommandName::SetupPlan,
            "setup-plan.unqualified.json",
            "setup-plan-unqualified",
        ),
        (
            CommandName::SetupPlan,
            "setup-plan.unavailable.json",
            "setup-plan-unavailable",
        ),
        (
            CommandName::SessionInitWorkspace,
            "workspace-init.json",
            "session-init-workspace",
        ),
    ] {
        let text = render(command, &example(file)?)?;
        check_snapshot(snapshot, &text)?;
    }
    Ok(())
}

/// A plan with a reviewed action names its download, digest, licence,
/// notices, trust limit and files, each URL on its own line.
#[test]
fn a_plan_with_an_action_names_everything_the_user_accepts() -> TestResult {
    let mut plan = example("setup-plan.unavailable.json")?;
    plan["data"]["target"] = json!("ubuntu_24_04_x86_64");
    plan["data"]["managed_install"] = json!("catalogue_accepted_install_pending");
    plan["data"]["catalogue_revision"] = json!("2026-09-20.1");
    plan["data"]["stop_new_plans_at"] = json!("2026-12-31T00:00:00Z");
    plan["data"]["plan_digest"] = json!("a".repeat(64));
    plan["data"]["actions"] = json!([{
        "id": "install-whisper_model",
        "component": "whisper_model",
        "version": "base-q5_1",
        "publisher": "ggml-org",
        "source_url": "https://example.invalid/ggml-base-q5_1.bin",
        "bytes": 59_707_625,
        "sha256": "b".repeat(64),
        "format": "raw_file",
        "archive_limits": null,
        "selected_files": [],
        "archive_links": [],
        "runtime_copies": [],
        "licence": "MIT",
        "notice_url": "https://example.invalid/LICENSE",
        "source_code_url": "https://example.invalid/source",
        "trust_limit": "Publisher bytes pinned by digest; model quality is not verified.",
        "licence_scope": "disclosure_not_legal_clearance",
        "destination": "private_per_user_managed_runtime",
        "permissions": "private_user_only",
        "change": "planned_download_verify_extract_smoke_activate",
        "required_authority": "user",
        "files": [{"name": "ggml-base-q5_1.bin", "bytes": 59_707_625, "sha256": "b".repeat(64), "mode": "owner_read_write"}]
    }]);
    let text = render(CommandName::SetupPlan, &plan)?;
    check_snapshot("setup-plan-action", &text)?;
    assert!(text.contains(&"a".repeat(64)), "the digest is cut");
    assert!(
        text.lines()
            .any(|line| line.trim() == "https://example.invalid/ggml-base-q5_1.bin")
    );
    Ok(())
}

#[test]
fn setup_registrations_say_what_was_registered_and_what_is_next() -> TestResult {
    let selection = OperationResponse::complete(
        CommandName::SetupConfigure.identifier(),
        &ConfiguredSelectionResponse::new(RuntimeDependency::Ffprobe),
    )?;
    let model = OperationResponse::complete(
        CommandName::SetupConfigureModel.identifier(),
        &ConfiguredModelResponse::new(),
    )?;
    let selection = result(CommandName::SetupConfigure, &selection)?.ok_or("no renderer")?;
    let model = result(CommandName::SetupConfigureModel, &model)?.ok_or("no renderer")?;
    check_snapshot("setup-configure", selection.as_str())?;
    check_snapshot("setup-configure-model", model.as_str())?;
    Ok(())
}

/// Every failure example renders as message, remediation and suggested
/// command, on stderr.
#[test]
fn frozen_failures_render_as_message_remediation_and_command() -> TestResult {
    for (file, snapshot) in [
        ("parse-failure.json", "failure-parse"),
        ("operation-error.json", "failure-not-implemented"),
        ("transcript-rejected.json", "failure-transcript-rejected"),
        (
            "retranscribe-cancelled.json",
            "failure-retranscribe-cancelled",
        ),
        ("storage-not-private.json", "failure-storage-not-private"),
        (
            "media-tool-verification-failed.json",
            "failure-media-tool-verification",
        ),
    ] {
        let rendered = render_failure(&example(file)?, None)?;
        assert_terminal_safe(rendered.as_str());
        assert!(rendered.as_str().starts_with("Error: "));
        check_snapshot(snapshot, rendered.as_str())?;
    }
    let cancelled = render_failure(&example("retranscribe-cancelled.json")?, None)?;
    assert!(
        cancelled
            .as_str()
            .lines()
            .any(|line| line.starts_with("Run: vsift job resume job_")),
        "{}",
        cancelled.as_str()
    );
    Ok(())
}

/// A parser explanation is quoted under its untrusted label; a fixed reason
/// follows the message.
#[test]
fn a_failure_shows_its_human_detail() -> TestResult {
    let explanation = DisplayText::render_lines(
        "error: invalid value 'a\u{202e}b\u{1b}[31m' for '--rect'\n  tip",
    );
    let parse = render_failure(
        &example("parse-failure.json")?,
        Some(HumanDetail::Parser(&explanation)),
    )?;
    assert_terminal_safe(parse.as_str());
    let lines: Vec<&str> = parse.as_str().lines().collect();
    assert_eq!(
        lines[1],
        "The parser explains (this quotes the command line, which is untrusted text):"
    );
    assert_eq!(
        lines[2],
        "  | error: invalid value 'a<U+202E>b\u{fffd}[31m' for '--rect'"
    );
    assert_eq!(lines[3], "  |   tip");
    assert!(lines[4].starts_with("Fix: The command line was rejected (invalid_value)."));
    assert_eq!(lines[5], "Run: vsift crop --help");

    let refused = render_failure(
        &example("operation-error.json")?,
        Some(HumanDetail::Reason(
            "execution profile is denied by host policy",
        )),
    )?;
    assert!(
        refused
            .as_str()
            .contains("\nReason: execution profile is denied by host policy\n")
    );
    Ok(())
}

/// Defence in depth: a result whose display fields broke their contract
/// (raw escapes, an OSC-8 link, bidirectional and separator characters) and
/// whose identifiers carry controls still reaches the terminal inert.
#[test]
fn hostile_display_fields_and_identifiers_stay_inert() -> TestResult {
    let mut page = example("transcript-get.json")?;
    page["data"]["session_id"] = json!("ses_0123\u{1b}[2J\u{9b}31m");
    page["data"]["items"][0]["display_text"] = json!(
        "\u{1b}]8;;https://example.invalid\u{7}click\u{1b}]8;;\u{7} \u{202e}DELIAF\u{202c}\nnext\u{2028}line\u{85}"
    );
    page["data"]["items"][0]["speaker"] = json!({
        "label": "ignored",
        "display_label": "Adm\u{200b}in\u{1b}[8m\nForged: line",
        "origin": "imported_webvtt_voice"
    });
    let text = render(CommandName::TranscriptGet, &page)?;
    assert!(!text.contains("\u{1b}]8;"));
    assert!(text.contains("  | \u{fffd}]8;;https://example.invalid\u{fffd}click"));
    assert!(text.contains("<U+202E>DELIAF<U+202C>"));
    assert!(text.contains("\n  | next<U+2028>line\u{fffd}\n"));
    assert!(text.contains("): Adm<U+200B>in\u{fffd}[8m\u{fffd}Forged: line\n"));
    // Raw `text` is never read: the example's plain text is not printed
    // where the display text was replaced.
    assert!(!text.contains("This synthetic sidecar is aligned with"));
    Ok(())
}

/// Commands left for P13 PR 2b print their indented JSON result.
#[test]
fn part_two_commands_have_no_renderer_yet() -> TestResult {
    for command in [
        CommandName::Candidates,
        CommandName::FrameGet,
        CommandName::FrameNeighbours,
        CommandName::FrameBurst,
        CommandName::Crop,
        CommandName::Audio,
        CommandName::JobStatus,
        CommandName::JobResume,
        CommandName::JobCancel,
        CommandName::JobRun,
        CommandName::JobBatch,
    ] {
        assert!(render_value(command, &json!({}))?.is_none(), "{command:?}");
    }
    Ok(())
}

/// A result that does not have its published shape fails to render
/// instead of printing half of it.
#[test]
fn a_result_without_its_published_shape_is_refused() {
    let broken = json!({"status": "complete", "data": {"session_id": 3}});
    assert!(render_value(CommandName::Ingest, &broken).is_err());
}
