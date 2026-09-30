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

use super::{
    HumanDetail, failure::render_failure, render_host, render_value, result, text::DisplayText,
};

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
    plan["data"]["managed_install"] = json!("catalogue_accepted");
    plan["data"]["catalogue_revision"] = json!("2026-09-20.1");
    plan["data"]["stop_new_plans_at"] = json!("2026-12-31T00:00:00Z");
    plan["data"]["plan_digest"] = json!("a".repeat(64));
    plan["data"]["install_needed"] = json!(true);
    plan["data"]["actions"] = json!([{
        "id": "install-whisper_model",
        "state": "pending",
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
    assert!(text.contains("from ggml-org [pending]"));
    assert!(!text.contains("nothing to install"));

    // P13 PR 4: after the install the same plan says each action is current
    // and that nothing is left to install.
    plan["data"]["install_needed"] = json!(false);
    plan["data"]["actions"][0]["state"] = json!("current");
    plan["data"]["local_asr_model"]["status"] = json!("managed_current");
    plan["data"]["readiness"] = json!("ready");
    let installed = render(CommandName::SetupPlan, &plan)?;
    assert!(installed.contains("from ggml-org [current]"));
    assert!(installed.contains("nothing to install"));
    assert!(installed.contains("Readiness: ready"));
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

/// P13: `setup install` lists each component with its status, the step and
/// typed reason of a failure and the stage's disposal; a failure also
/// renders its error, with the remediation, for stderr.
#[test]
fn setup_install_lists_each_component_and_its_outcome() -> TestResult {
    let complete = render_value(CommandName::SetupInstall, &example("setup-install.json")?)?
        .ok_or("no renderer")?;
    assert_terminal_safe(complete.as_str());
    check_snapshot("setup-install", complete.as_str())?;
    assert!(
        complete
            .as_str()
            .contains("[activated] ffmpeg_ffprobe n9.0.1-11-ge47273f4d9-20260831")
    );

    let failed_value = example("setup-install.failed.json")?;
    let failed = render_value(CommandName::SetupInstall, &failed_value)?.ok_or("no renderer")?;
    assert_terminal_safe(failed.as_str());
    check_snapshot("setup-install-failed", failed.as_str())?;
    assert!(failed.as_str().contains(
        "[failed] whisper_cli whisper.cpp-v1.9.2-ubuntu-x64: download offline, DOWNLOAD_FAILED"
    ));
    assert!(failed.as_str().contains("Status: failed"));
    let error = render_failure(&failed_value, None)?;
    assert_terminal_safe(error.as_str());
    check_snapshot("failure-setup-install-download", error.as_str())?;
    assert!(error.as_str().contains("(DOWNLOAD_FAILED)"));
    Ok(())
}

/// P13 PR 6: `setup list`, `setup rollback`, `setup remove` and `setup
/// repair` render every component, version, outcome and finding, and a
/// repair finding's command as a `Run:` line of fixed words and keys.
#[test]
fn setup_lifecycle_results_render() -> TestResult {
    for (command, file, snapshot, expected) in [
        (
            CommandName::SetupList,
            "setup-list.json",
            "setup-list",
            "  [verified] n9.0.1-11-ge47273f4d9-20260831, selected",
        ),
        (
            CommandName::SetupRollback,
            "setup-rollback.json",
            "setup-rollback",
            "ffmpeg_ffprobe: rolled_back, selected n8.1-2-g0123456789-20260601",
        ),
        (
            CommandName::SetupRemove,
            "setup-remove.json",
            "setup-remove",
            "Abandoned stages: removed 2, kept 0",
        ),
        (
            CommandName::SetupRepair,
            "setup-repair.json",
            "setup-repair",
            "  Run: vsift setup rollback ffmpeg_ffprobe",
        ),
        (
            CommandName::SetupRemove,
            "setup-remove.failed.json",
            "setup-remove-failed",
            "[in_use] whisper_cli whisper.cpp-v1.9.2-ubuntu-x64",
        ),
    ] {
        let text = render(command, &example(file)?)?;
        check_snapshot(snapshot, &text)?;
        assert!(text.contains(expected), "{snapshot}: {text}");
    }
    let failed = render_failure(&example("setup-remove.failed.json")?, None)?;
    assert_terminal_safe(failed.as_str());
    assert!(failed.as_str().contains("(BUSY)"));
    assert!(failed.as_str().contains("Retry after: 30000 ms"));
    let install = render(CommandName::SetupInstall, &example("setup-install.json")?)?;
    assert!(install.contains("Abandoned stages: removed 1, kept 0"));
    assert!(install.contains("Cleanup: [removed] ffmpeg_ffprobe n8.1-2-g0123456789-20260601"));
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

/// Every frozen example of a PR 2b command renders; its snapshot is the
/// text a person reads.
#[test]
fn part_two_examples_render_as_readable_text() -> TestResult {
    for (command, file, snapshot) in [
        (CommandName::Candidates, "candidates.json", "candidates"),
        (
            CommandName::Candidates,
            "candidates.partial.json",
            "candidates-partial",
        ),
        (CommandName::FrameGet, "frame-get.json", "frame-get"),
        (
            CommandName::FrameNeighbours,
            "frame-neighbours.json",
            "frame-neighbours",
        ),
        (
            CommandName::FrameBurst,
            "frame-burst.partial.json",
            "frame-burst-partial",
        ),
        (CommandName::Crop, "crop.json", "crop"),
        (CommandName::Audio, "audio.json", "audio"),
        (CommandName::JobStatus, "job-status.json", "job-status"),
        (CommandName::JobResume, "job-resume.json", "job-resume"),
        (CommandName::JobCancel, "job-cancel.json", "job-cancel"),
        (CommandName::JobRun, "job-run.json", "job-run"),
        (
            CommandName::JobRun,
            "job-run.partial.json",
            "job-run-partial",
        ),
        (
            CommandName::JobRun,
            "job-run.replayed.json",
            "job-run-replayed",
        ),
        (CommandName::JobBatch, "job-batch.json", "job-batch"),
    ] {
        let text = render(command, &example(file)?)?;
        check_snapshot(snapshot, &text)?;
    }
    Ok(())
}

/// Every command that completes has a renderer: `result` answers `None`
/// only for `setup check`, rendered from its typed report, and for the
/// commands that only ever fail.
#[test]
fn only_setup_check_and_rejected_command_lines_have_no_result_renderer() -> TestResult {
    for command in [CommandName::SetupCheck, CommandName::Parse] {
        assert!(render_value(command, &json!({}))?.is_none(), "{command:?}");
    }
    for command in [
        CommandName::SetupInstall,
        CommandName::SetupList,
        CommandName::SetupRollback,
        CommandName::SetupRemove,
        CommandName::SetupRepair,
        CommandName::Candidates,
        CommandName::FrameGet,
        CommandName::Crop,
        CommandName::Audio,
        CommandName::JobStatus,
        CommandName::JobRun,
        CommandName::JobBatch,
    ] {
        assert!(render_value(command, &json!({})).is_err(), "{command:?}");
    }
    Ok(())
}

/// L-016 and SEC-T02 (P13 PR 2b): a delivered path stands alone on its
/// line under its file's label. An exact path is the line itself; a path
/// with controls or hidden characters (the session root is the user's
/// choice) reaches the terminal inert and is flagged; the extended-length
/// form gets its note once.
#[test]
fn delivered_paths_stand_alone_and_stay_inert() -> TestResult {
    let hostile = "/root/a\u{202e}gnp.exe\u{200b}/\u{1b}]8;;https://example.invalid\u{7}x\u{1b}\\\n\
                   Forged: line\u{2028}\u{85}/artifact.png";
    let extended = r"\\?\C:\Users\someone\AppData\Local\vsift\sessions\ses_0123456789abcdef0123456789abcdef\artifacts\artifact-4ab8.png";
    for (command, file) in [
        (CommandName::FrameGet, "frame-get.json"),
        (CommandName::FrameNeighbours, "frame-neighbours.json"),
        (CommandName::FrameBurst, "frame-burst.partial.json"),
        (CommandName::Crop, "crop.json"),
        (CommandName::Audio, "audio.json"),
    ] {
        let mut value = example(file)?;
        value["data"]["files"][0]["path"] = json!(hostile);
        if value["data"]["files"][1].is_object() {
            value["data"]["files"][1]["path"] = json!(extended);
        }
        let text = render(command, &value)?;
        assert!(!text.contains("\u{1b}]8;") && !text.contains('\u{2028}'));
        let lines: Vec<&str> = text.lines().collect();
        let shown = lines
            .iter()
            .position(|line| line.contains("<U+202E>gnp.exe<U+200B>"))
            .ok_or("the hostile path is missing")?;
        assert!(lines[shown].starts_with("    /root/a<U+202E>"), "{text}");
        assert!(
            lines[shown].ends_with("\u{fffd}Forged: line<U+2028>\u{fffd}/artifact.png"),
            "{text}"
        );
        assert!(lines[shown - 1].starts_with("  File ("), "{text}");
        assert!(!lines.iter().any(|line| line.starts_with("Forged")));
        assert_eq!(text.matches("is not shown exactly").count(), 1, "{text}");
        if value["data"]["files"][1].is_object() {
            assert!(
                lines.contains(&format!("    {extended}").as_str()),
                "{text}"
            );
            assert_eq!(text.matches("extended-length form").count(), 1, "{text}");
        } else {
            assert!(!text.contains("extended-length form"), "{text}");
        }
    }
    Ok(())
}

/// A path longer than a line is replaced by a statement, never cut; an
/// exact extended-length path is its own line, as `--json` gives it.
#[test]
fn a_long_or_extended_path_is_never_cut() -> TestResult {
    let long = format!("/{}", "p".repeat(4_200));
    let mut value = example("frame-get.json")?;
    value["data"]["files"][0]["path"] = json!(long);
    let text = render(CommandName::FrameGet, &value)?;
    assert!(!text.contains(&"p".repeat(100)), "a path was cut");
    assert!(text.contains("\n    (a path longer than 4000 bytes; read it with --json)\n"));
    assert!(text.contains("is not shown exactly"));

    let extended = format!(r"\\?\C:\{}\artifact.png", "d".repeat(300));
    let mut value = example("audio.json")?;
    value["data"]["files"][0]["path"] = json!(extended);
    let text = render(CommandName::Audio, &value)?;
    check_snapshot("audio-extended-path", &text)?;
    assert!(text.lines().any(|line| line == format!("    {extended}")));
    Ok(())
}

/// A failed worker request renders its job result for stdout and its error
/// for stderr; a refusal before the request ran has an error only.
#[test]
fn a_failed_worker_request_renders_its_result_and_its_error() -> TestResult {
    let mut failed = example("job-run.json")?;
    failed["status"] = json!("failed");
    failed["data"]["status"] = json!("failed");
    failed["data"]["steps"][1]["status"] = json!("failed");
    failed["data"]["steps"][1]["outputs"] = json!(null);
    failed["data"]["steps"][1]["failure"] =
        json!({"code": "BUSY", "retryable": true, "retry_after_ms": 2000});
    failed["data"]["steps"][2]["status"] = json!("not_started");
    failed["data"]["steps"][2]["outputs"] = json!(null);
    failed["data"]["steps"][2]["coverage"] = json!(null);
    failed["data"]["steps"][3]["status"] = json!("not_started");
    failed["data"]["steps"][3]["outputs"] = json!(null);
    failed["data"]["failure"] = json!({
        "code": "BUSY", "retryable": true, "retry_after_ms": 2000, "step": 1, "rejection": null
    });
    failed["error"] = json!({
        "code": "BUSY",
        "message": "The resource is busy.",
        "retryable": true,
        "retry_after_ms": 2000,
        "affected_ids": ["ses_0123456789abcdef0123456789abcdef"],
        "remediation": [{"summary": "Deliver the same request again after the retry hint.", "command": null, "required_authority": "none"}]
    });
    let text = render_host(CommandName::JobRun, &failed)?;
    let result = text.result.ok_or("no result")?;
    let error = text.failure.ok_or("no error")?;
    assert_terminal_safe(result.as_str());
    check_snapshot("job-run-failed", result.as_str())?;
    check_snapshot("job-run-failed-stderr", error.as_str())?;

    let text = render_host(CommandName::JobRun, &example("operation-error.json")?)?;
    assert!(text.result.is_none());
    assert!(
        text.failure
            .ok_or("no error")?
            .as_str()
            .starts_with("Error: ")
    );
    Ok(())
}

/// A result that does not have its published shape fails to render
/// instead of printing half of it.
#[test]
fn a_result_without_its_published_shape_is_refused() {
    let broken = json!({"status": "complete", "data": {"session_id": 3}});
    assert!(render_value(CommandName::Ingest, &broken).is_err());
}
