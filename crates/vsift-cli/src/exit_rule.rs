//! The exit rule across every command (the Fable 0.3.0 review, P-2): exit 0
//! means a `complete` or `partial` envelope, and every other exit carries an
//! `error.code` of that exit's class (`cli-v1.md`, "Exit and error
//! taxonomy").
//!
//! Each published example envelope, in `schemas/v1/examples/` and in the
//! frozen `v0.1.0` copy, is listed in [`EXITS`] with the exit status the CLI
//! gives that answer. The test refuses an envelope example the table does
//! not list, so a later command cannot add an answer without stating its
//! exit, and then holds every listed answer to the rule. The table is the
//! author's statement of each exit; the commands' own tests hold the real
//! process to it (for `watch` and the unattended `setup`, the two that broke
//! the rule before 0.3.0's freeze, `watch`'s contract tests and
//! `door::tests`).

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;
use vsift::FailureCode;

use crate::output::ProcessExit;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Every example envelope and the exit status the CLI gives that answer.
const EXITS: &[(&str, u8)] = &[
    ("audio.json", 0),
    ("bundle-search.json", 0),
    ("candidates.brief.json", 0),
    ("candidates.events.jsonl", 0),
    ("candidates.json", 0),
    ("candidates.omit-periodic.brief.json", 0),
    ("candidates.omit-periodic.json", 0),
    ("candidates.partial.brief.json", 0),
    ("candidates.partial.json", 0),
    ("changes.crops.json", 0),
    ("changes.events.jsonl", 0),
    ("changes.json", 0),
    ("changes.partial.json", 0),
    ("clocks.json", 0),
    ("clocks.not-run.json", 0),
    ("crop.json", 0),
    ("frame-burst.partial.json", 0),
    ("frame-get.busy-transcription.json", 4),
    ("frame-get.busy.json", 4),
    ("frame-get.events.jsonl", 0),
    ("frame-get.json", 0),
    ("frame-neighbours.json", 0),
    ("handoff-check.json", 0),
    ("handoff-skeleton.json", 0),
    ("ingest-copy-stopped-early.json", 3),
    ("ingest-copy-too-slow.json", 3),
    ("ingest-source-access-denied.json", 7),
    ("ingest-source-is-folder.json", 7),
    ("ingest-source-not-found.json", 7),
    ("ingest-source-socket.json", 7),
    ("ingest-source-special-file.json", 3),
    ("ingest-url-address-refused.json", 3),
    ("ingest.page.json", 0),
    ("ingest.time-anchor.json", 0),
    ("ingest.transcript.json", 0),
    ("ingest.url.json", 0),
    ("job-batch.events.jsonl", 0),
    ("job-batch.json", 0),
    ("job-cancel.json", 0),
    ("job-resume.json", 0),
    ("job-run.json", 0),
    ("job-run.partial.json", 0),
    ("job-run.replayed.json", 0),
    ("job-run.retranscribe-partial.json", 0),
    ("job-start.watched.json", 0),
    ("job-status.json", 0),
    ("job-status.prepare.json", 0),
    ("media-tool-verification-failed.json", 2),
    ("ocr-search.json", 0),
    ("ocr-search.not-run.json", 0),
    ("ocr.json", 0),
    ("ocr.partial.json", 0),
    ("operation-error.json", 2),
    ("parse-failure.json", 2),
    ("retranscribe-cancelled.json", 6),
    ("retranscribe-unusable-output.json", 2),
    ("retranscribe-unusable-output.short-range.json", 2),
    ("search.brief.json", 0),
    ("search.events.jsonl", 0),
    ("search.json", 0),
    ("search.open.json", 0),
    ("search.screen-text-unavailable.json", 0),
    ("search.screen-text.json", 0),
    ("session-root-name-not-allowed.json", 7),
    ("session-root-own-access-missing.json", 7),
    ("session-root-unowned.json", 7),
    ("setup-install.failed.json", 7),
    ("setup-install.json", 0),
    ("setup-install.missing-library.json", 2),
    ("setup-install.optional-not-installed.json", 0),
    ("setup-list.json", 0),
    ("setup-plan.optional-offered.json", 0),
    ("setup-plan.timed-out.json", 0),
    ("setup-plan.unavailable.json", 0),
    ("setup-plan.unqualified.json", 0),
    ("setup-remove.failed.json", 4),
    ("setup-remove.json", 0),
    ("setup-repair.json", 0),
    ("setup-rollback.json", 0),
    ("setup.fail.json", 7),
    ("setup.pass.json", 0),
    ("skill-install.json", 0),
    ("skill-show.json", 0),
    ("skill-status.json", 0),
    ("storage-not-private.json", 7),
    ("terminal-event.json", 2),
    ("transcript-get.asr.json", 0),
    ("transcript-get.brief.json", 0),
    ("transcript-get.clock-time.json", 0),
    ("transcript-get.events.jsonl", 0),
    ("transcript-get.json", 0),
    ("transcript-get.open.json", 0),
    ("transcript-get.partial.json", 0),
    ("transcript-get.pending-transcription.json", 2),
    ("transcript-rejected.json", 3),
    ("transcript-retranscribe.events.jsonl", 0),
    ("transcript-retranscribe.json", 0),
    ("transcript-retranscribe.partial.json", 0),
    ("transcript-retranscribe.unplaced.json", 0),
    ("watch.json", 0),
    ("workspace-init.json", 0),
];

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Whether `value` is a v1 operation envelope (not a stored record, a
/// request or `setup check`'s own result, which has no envelope).
fn is_envelope(value: &Value) -> bool {
    [
        "schema_version",
        "command",
        "status",
        "data",
        "error",
        "coverage",
    ]
    .iter()
    .all(|member| value.get(member).is_some())
}

/// The envelope a published file answers with: the document itself, the
/// `result` of a terminal event, or the last terminal event of a JSON Lines
/// stream; `None` for a file that holds no envelope.
fn envelope(path: &Path) -> Result<Option<Value>, Box<dyn std::error::Error>> {
    let text = fs::read_to_string(path)?;
    let documents = if path
        .extension()
        .is_some_and(|extension| extension == "jsonl")
    {
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<Vec<Value>, _>>()?
    } else {
        vec![serde_json::from_str(&text)?]
    };
    Ok(documents.into_iter().rev().find_map(|document| {
        if document["event"] == "terminal" {
            Some(document["result"].clone())
        } else {
            is_envelope(&document).then_some(document)
        }
    }))
}

/// The exit status of a failure code's class, as the process gives it.
fn exit_of(identifier: &str) -> Option<u8> {
    FailureCode::ALL
        .into_iter()
        .find(|code| code.identifier() == identifier)
        .map(|code| ProcessExit::from(code.class()).code())
}

/// Holds one answer to the rule; returns what is wrong with it, if anything.
fn breaks_the_rule(envelope: &Value, exit: u8) -> Option<String> {
    let status = envelope["status"].as_str().unwrap_or_default();
    let code = envelope["error"]["code"].as_str();
    match (exit, status) {
        (0, "complete" | "partial") => {
            (!envelope["error"].is_null()).then(|| "a success carries an error".to_owned())
        }
        (0, other) => Some(format!("exit 0 with status {other}")),
        (_, "complete" | "partial") => Some(format!(
            "exit {exit} with status {status}: a non-zero exit is a failed or cancelled envelope"
        )),
        (_, _) => match code.map(exit_of) {
            Some(Some(class)) if class == exit => None,
            Some(Some(class)) => Some(format!(
                "exit {exit}, but error.code {} has class {class}",
                code.unwrap_or_default()
            )),
            Some(None) => Some(format!("unknown error.code {}", code.unwrap_or_default())),
            None => Some(format!("exit {exit} without an error.code")),
        },
    }
}

/// Checks every envelope example in `folder` against [`EXITS`] and the rule.
fn check_folder(folder: &str, listed: &mut BTreeSet<&'static str>) -> TestResult {
    let mut problems = Vec::new();
    for entry in fs::read_dir(repository(folder))? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(envelope) = envelope(&path)? else {
            continue;
        };
        let Some(&(listed_name, exit)) = EXITS.iter().find(|(example, _)| *example == name) else {
            problems.push(format!(
                "{folder}/{name}: an envelope example the exit table does not list"
            ));
            continue;
        };
        listed.insert(listed_name);
        if let Some(problem) = breaks_the_rule(&envelope, exit) {
            problems.push(format!("{folder}/{name}: {problem}"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    Ok(())
}

/// Every published example, current and frozen, keeps the exit rule, and the
/// table names no example that is gone.
#[test]
fn every_example_keeps_the_exit_rule() -> TestResult {
    let mut listed = BTreeSet::new();
    check_folder("schemas/v1/examples", &mut listed)?;
    check_folder("schemas/v1/frozen/v0.1.0/examples", &mut listed)?;
    let stale: Vec<&str> = EXITS
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !listed.contains(name))
        .collect();
    assert!(stale.is_empty(), "listed but not published: {stale:?}");
    Ok(())
}

/// The rule itself refuses each way of breaking it, including the two the
/// review found: a `complete` answer with a non-zero exit (the unattended
/// `setup`'s old fail) and a non-zero exit without an error.
#[test]
fn the_rule_refuses_each_break() {
    let complete = serde_json::json!({"status": "complete", "error": null});
    let failed = |code: &str| serde_json::json!({"status": "failed", "error": {"code": code}});
    assert_eq!(breaks_the_rule(&complete, 0), None);
    assert!(breaks_the_rule(&complete, 2).is_some());
    assert!(breaks_the_rule(&complete, 7).is_some());
    assert_eq!(breaks_the_rule(&failed("DOWNLOAD_FAILED"), 7), None);
    assert!(breaks_the_rule(&failed("DOWNLOAD_FAILED"), 2).is_some());
    assert!(breaks_the_rule(&failed("NOT_A_CODE"), 2).is_some());
    assert!(breaks_the_rule(&failed("BUSY"), 0).is_some());
    assert!(breaks_the_rule(&serde_json::json!({"status": "failed", "error": null}), 2).is_some());
}
