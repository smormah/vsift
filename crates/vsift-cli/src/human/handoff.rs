//! Human text of `handoff check` (P13 PR 5).
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
            .push_unsigned(u64::try_from(data.errors.len()).unwrap_or(u64::MAX))
            .push_fixed(" problem(s) to fix before sending");
    }
    text.end_line();
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
    push_findings(&mut text, "Problems:", &data.errors, "  Allowed: ");
    push_findings(&mut text, "Warnings:", &data.warnings, "  Allowed: ");
    push_findings(
        &mut text,
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
}
