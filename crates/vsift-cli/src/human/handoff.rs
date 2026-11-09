//! Human text of `handoff check` (P13 PR 5), `handoff skeleton` (ADR 0038)
//! and `handoff finish` (0.3.0, M5b).
//!
//! A finding holds no draft text (SEC-16): its pointer is built from the
//! schema's member names and indices, its allowed values are the schema's
//! own and its message is fixed prose. Each is still written through
//! [`TerminalText::push_value`], which applies the control and
//! hidden-character rules to anything a result carries.

use serde::Deserialize;

use super::{
    push_outcome,
    text::{RenderedText, TerminalText, TooLarge},
    view::Envelope,
};

/// The `data` of a `handoff.check` result.
#[derive(Debug, Deserialize)]
pub(crate) struct HandoffCheck {
    valid: bool,
    handoff_version: String,
    errors: Vec<Finding>,
    warnings: Vec<Finding>,
    case_notes: Vec<Finding>,
    truncated: bool,
    session: Option<SessionCheck>,
}

/// One finding.
#[derive(Debug, Deserialize)]
struct Finding {
    pointer: Option<String>,
    line: Option<u64>,
    rule: String,
    allowed: Option<Vec<String>>,
    message: String,
}

/// How `--session` was used.
#[derive(Debug, Deserialize)]
struct SessionCheck {
    session_id: String,
    resolved: bool,
    gap: Option<String>,
    identities_checked: u64,
}

pub(super) fn check(envelope: &Envelope<HandoffCheck>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    if data.valid {
        text.push_fixed("Handoff check: valid");
    } else {
        text.push_fixed("Handoff check: not valid, ")
            .push_unsigned(count(&data.errors))
            .push_fixed(" problem(s) to fix before sending");
    }
    text.end_line();
    push_check(&mut text, data);
    push_outcome(&mut text, envelope);
    text.finish()
}

/// A list's length for the terminal.
fn count<T>(list: &[T]) -> u64 {
    u64::try_from(list.len()).unwrap_or(u64::MAX)
}

/// Everything of a check after its first line.
fn push_check(text: &mut TerminalText, data: &HandoffCheck) {
    text.push_fixed("Handoff version: ")
        .push_value(&data.handoff_version)
        .end_line();
    if let Some(session) = &data.session {
        text.push_fixed("Session: ").push_value(&session.session_id);
        if session.resolved {
            text.push_fixed(", ")
                .push_unsigned(session.identities_checked)
                .push_fixed(" cited identities checked");
        } else {
            text.push_fixed(", not checked (");
            match &session.gap {
                Some(gap) => text.push_value(gap),
                None => text.push_fixed("no records"),
            };
            text.push_fixed(")");
        }
        text.end_line();
    }
    push_findings(text, "Problems:", &data.errors, "  Allowed: ");
    push_findings(text, "Warnings:", &data.warnings, "  Allowed: ");
    push_findings(
        text,
        "Read in the schema's letter case:",
        &data.case_notes,
        "  Read as: ",
    );
    if data.truncated {
        text.blank_line();
        text.push_fixed(
            "Only the first 100 findings of each kind are listed; fix these and check again.",
        )
        .end_line();
    }
}

/// The `data` of a `handoff.finish` result.
#[derive(Debug, Deserialize)]
pub(crate) struct HandoffFinish {
    finished: bool,
    stopped_at: Option<FinishStop>,
    requested: FinishRequest,
    check: HandoffCheck,
    lifecycle: FinishLifecycle,
    retain: Option<RetainedBundle>,
    bundle_validated: bool,
    close: Option<ClosedSession>,
}

/// `data.requested`.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FinishRequest {
    Check,
    Retain,
    Close,
}

/// `data.stopped_at`.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FinishStop {
    Draft,
    Lifecycle,
}

/// `data.lifecycle`: closed words only.
#[derive(Debug, Deserialize)]
struct FinishLifecycle {
    stated: Option<String>,
    requested: Option<String>,
}

/// What a person needs of `data.retain`.
#[derive(Debug, Deserialize)]
struct RetainedBundle {
    session_id: String,
    source_included: bool,
    artifact_count: u64,
    artifact_bytes: u64,
}

/// What a person needs of `data.close`.
#[derive(Debug, Deserialize)]
struct ClosedSession {
    session_id: String,
    state: String,
}

/// `handoff finish`. The first line says plainly what was and was not
/// written, and why: an exit of 0 with nothing retained must not read as a
/// success (the planner's ruling of 2026-11-09).
pub(super) fn finish(envelope: &Envelope<HandoffFinish>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    let refused = match data.requested {
        FinishRequest::Check => "Not finished: ",
        FinishRequest::Retain => "Not retained: ",
        FinishRequest::Close => "Not closed: ",
    };
    match (data.stopped_at, data.finished) {
        (Some(FinishStop::Draft), _) => {
            text.push_fixed(refused)
                .push_fixed("the draft did not pass (")
                .push_unsigned(count(&data.check.errors))
                .push_fixed(" problem(s)); fix them and run it again");
        }
        (Some(FinishStop::Lifecycle), _) => {
            text.push_fixed(refused)
                .push_fixed("the draft's lifecycle.action is ");
            match &data.lifecycle.stated {
                Some(stated) => text.push_value(stated),
                None => text.push_fixed("not a known action"),
            };
            text.push_fixed(", not ");
            match &data.lifecycle.requested {
                Some(requested) => text.push_value(requested),
                None => text.push_fixed("the action asked for"),
            };
            text.push_fixed("; make them agree and run it again");
        }
        (None, false) => {
            text.push_fixed(refused).push_fixed("nothing was written");
        }
        (None, true) => {
            text.push_fixed(match data.requested {
                FinishRequest::Check => {
                    "Finished: the draft passed; nothing was retained or closed (none was asked)"
                }
                FinishRequest::Retain => {
                    "Retained: the draft passed, and the bundle was written and validated"
                }
                FinishRequest::Close => "Closed: the draft passed, and the session was closed",
            });
        }
    }
    text.end_line();
    push_check(&mut text, &data.check);
    if let Some(bundle) = &data.retain {
        text.blank_line();
        text.push_fixed("Retained session ")
            .push_value(&bundle.session_id)
            .push_fixed(" to the folder you named: ")
            .push_unsigned(bundle.artifact_count)
            .push_fixed(" artifacts (")
            .push_unsigned(bundle.artifact_bytes)
            .push_fixed(" bytes)")
            .end_line();
        text.push_fixed(if bundle.source_included {
            "Source copy: included in the bundle"
        } else {
            "Source copy: not included; re-extraction needs the matching original video"
        })
        .end_line();
        if data.bundle_validated {
            text.push_fixed("Bundle: validated; no bundle validate is needed")
                .end_line();
        }
    }
    if let Some(closed) = &data.close {
        text.blank_line();
        text.push_fixed("Closed session ")
            .push_value(&closed.session_id)
            .push_fixed(", state ")
            .push_value(&closed.state)
            .end_line();
    }
    push_outcome(&mut text, envelope);
    text.finish()
}

/// The `data` of a `handoff.skeleton` result.
#[derive(Debug, Deserialize)]
pub(crate) struct HandoffSkeleton {
    handoff: serde_json::Value,
    fill: Vec<String>,
}

/// The skeleton as the fenced block a report ends with, ready to copy, and
/// the members left to fill in. Each line of the JSON is written through
/// [`TerminalText::push_value`]: the question is the caller's text.
pub(super) fn skeleton(envelope: &Envelope<HandoffSkeleton>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    let json = serde_json::to_string_pretty(&data.handoff).map_err(|_| TooLarge)?;
    text.push_fixed("```vsift-handoff").end_line();
    for line in json.lines() {
        text.push_value(line).end_line();
    }
    text.push_fixed("```").end_line();
    text.blank_line();
    text.push_fixed("Fill in: ");
    for (index, member) in data.fill.iter().enumerate() {
        if index > 0 {
            text.push_fixed(", ");
        }
        text.push_value(member);
    }
    text.end_line();
    push_outcome(&mut text, envelope);
    text.finish()
}

fn push_findings(
    text: &mut TerminalText,
    heading: &'static str,
    findings: &[Finding],
    allowed_label: &'static str,
) {
    if findings.is_empty() {
        return;
    }
    text.blank_line();
    text.push_fixed(heading).end_line();
    for finding in findings {
        text.push_fixed("- ");
        match (&finding.pointer, finding.line) {
            (Some(pointer), _) => {
                text.push_fixed("at ").push_value(pointer);
            }
            (None, Some(line)) => {
                text.push_fixed("line ").push_unsigned(line);
            }
            (None, None) => {
                text.push_fixed("report");
            }
        }
        text.push_fixed(" (")
            .push_value(&finding.rule)
            .push_fixed(")")
            .end_line();
        text.push_fixed("  ")
            .push_value(&finding.message)
            .end_line();
        if let Some(allowed) = &finding.allowed {
            text.push_fixed(allowed_label);
            for (index, value) in allowed.iter().enumerate() {
                if index > 0 {
                    text.push_fixed(", ");
                }
                text.push_value(value);
            }
            text.end_line();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use serde_json::json;
    use vsift_contract::{CommandName, is_hidden_character};

    use super::super::{render_value, tests::check_snapshot};

    /// The frozen example renders; its snapshot is what a person reads.
    #[test]
    fn the_frozen_example_renders_as_readable_text() -> Result<(), Box<dyn std::error::Error>> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schemas/v1/examples/handoff-check.json");
        let value: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
        let rendered = render_value(CommandName::HandoffCheck, &value)?.ok_or("no renderer")?;
        let text = rendered.as_str();
        assert!(text.chars().all(|character| character == '\n'
            || !(character.is_control() || is_hidden_character(character))));
        check_snapshot("handoff-check", text)
    }

    /// The frozen `handoff finish` example (a retention) renders; its
    /// snapshot is what a person reads.
    #[test]
    fn the_frozen_finish_example_renders_as_readable_text() -> Result<(), Box<dyn std::error::Error>>
    {
        let rendered =
            render_value(CommandName::HandoffFinish, &finish_example()?)?.ok_or("no renderer")?;
        let text = rendered.as_str();
        assert!(text.starts_with("Retained: the draft passed"));
        check_snapshot("handoff-finish", text)
    }

    fn finish_example() -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schemas/v1/examples/handoff-finish.json");
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    /// A stop before any write says so on its first line, for each write
    /// (the planner's ruling of 2026-11-09: an exit of 0 with no bundle must
    /// not read as a success).
    #[test]
    fn a_stop_says_on_its_first_line_that_nothing_was_written()
    -> Result<(), Box<dyn std::error::Error>> {
        let example = finish_example()?;
        for (requested, stopped_at, first_line) in [
            (
                "retain",
                "draft",
                "Not retained: the draft did not pass (1 problem(s)); fix them and run it again",
            ),
            (
                "close",
                "draft",
                "Not closed: the draft did not pass (1 problem(s)); fix them and run it again",
            ),
            (
                "retain",
                "lifecycle",
                "Not retained: the draft's lifecycle.action is left_open, not retained; make them agree and run it again",
            ),
        ] {
            let mut value = example.clone();
            value["data"]["finished"] = json!(false);
            value["data"]["stopped_at"] = json!(stopped_at);
            value["data"]["requested"] = json!(requested);
            value["data"]["retain"] = serde_json::Value::Null;
            value["data"]["bundle_validated"] = json!(false);
            value["data"]["lifecycle"]["stated"] = json!("left_open");
            value["data"]["check"]["errors"] = json!([{
                "pointer": "/status",
                "line": null,
                "rule": "value_not_allowed",
                "allowed": ["complete", "partial", "insufficient_evidence"],
                "message": "This closed value is not allowed; write exactly one of the allowed values."
            }]);
            let rendered =
                render_value(CommandName::HandoffFinish, &value)?.ok_or("no renderer")?;
            assert_eq!(rendered.as_str().lines().next(), Some(first_line));
        }
        Ok(())
    }
}
