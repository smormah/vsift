//! Checks the handoff an agent wrote, with the same check as `vsift handoff
//! check` (P13 PR 5).
//!
//! Extraction, letter-case reading, schema validation, the rules of
//! `references/handoff.md` and the report-text rules all live in
//! `vsift_contract::HandoffChecker`, which the command uses too, so the
//! command and the grader cannot disagree. The grader adds only what is its
//! own:
//!
//! - its private markers (the trial root and the operating-system user
//!   name) to the report-text check;
//! - a cross-check of every schema verdict against `jsonschema`, a general
//!   draft 2020-12 validator: a difference is a grader defect and fails the
//!   handoff rather than passing it silently;
//! - the rule that the budget table it grades with is the one the check
//!   knows ([`HandoffSchema::new`]).

use serde_json::Value;
use vsift_contract::{
    COMPACT_BUDGET, HandoffBudgetLimits, HandoffChecker, HandoffFinding, HandoffRuleScope,
    STANDARD_BUDGET, extract_handoff_block, handoff_report_text_findings,
};

use crate::{
    error::TrialError,
    policy::{BudgetLimits, Budgets},
};

pub use vsift_contract::MAX_RESUME_CARD_BYTES as MAX_RESUME_BYTES;

/// One finding as a grade detail: where, which rule and its fixed prose.
/// A finding never holds the draft's text.
#[must_use]
pub fn describe(finding: &HandoffFinding) -> String {
    let location = match (finding.pointer(), finding.line()) {
        (Some(""), _) => "the handoff".to_owned(),
        (Some(pointer), _) => pointer.to_owned(),
        (None, Some(line)) => format!("line {line}"),
        (None, None) => "the report".to_owned(),
    };
    let allowed = finding
        .allowed()
        .map(|allowed| format!(" (allowed: {})", allowed.join(", ")))
        .unwrap_or_default();
    format!(
        "{location}: {}{allowed}: {}",
        finding.rule().identifier(),
        finding.message()
    )
}

/// Extracts the one `vsift-handoff` block of a final message and parses it.
///
/// # Errors
///
/// What is wrong, as [`describe`] writes it.
pub fn extract(message: &str) -> Result<Value, String> {
    extract_handoff_block(message).map_err(|finding| describe(&finding))
}

/// Local facts the text checks look for: the trial root and the operating
/// system user name, which must never reach a report.
#[derive(Clone, Debug, Default)]
pub struct PrivateMarkers {
    /// Lower-case strings that must not appear (the trial root in both
    /// slash styles, the user name).
    pub strings: Vec<String>,
}

/// Every text-safety problem of a final message: the check's report-text
/// rules, then the grader's private markers.
#[must_use]
pub fn text_problems(message: &str, markers: &PrivateMarkers) -> Vec<String> {
    let mut problems: Vec<String> = handoff_report_text_findings(message)
        .iter()
        .map(describe)
        .collect();
    let lowered = message.to_lowercase();
    if markers
        .strings
        .iter()
        .any(|marker| !marker.is_empty() && lowered.contains(marker.as_str()))
    {
        problems.push("the report names the trial root or the user name".to_owned());
    }
    problems
}

/// What the handoff checks found in one final message.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReportFindings {
    /// The handoff with its closed values in the schema's letter case, when
    /// the message had one readable block.
    pub handoff: Option<Value>,
    /// Block, schema and rule problems: they fail `handoff_valid`.
    pub problems: Vec<String>,
    /// Letter-case notes, then warnings (such as a citation no claim uses):
    /// recorded beside `handoff_valid`, failing nothing.
    pub notes: Vec<String>,
}

/// The shared handoff check, with the grader's cross-check.
pub struct HandoffSchema {
    checker: HandoffChecker,
    reference: jsonschema::Validator,
}

impl HandoffSchema {
    /// Compiles `handoff.schema.json` with the shared check and with
    /// `jsonschema`; `budgets` are the profiles of `references/budgets.md`,
    /// which must be the check's own (`vsift_contract::COMPACT_BUDGET` and
    /// `STANDARD_BUDGET`).
    ///
    /// # Errors
    ///
    /// [`TrialError::Invalid`] when the schema does not compile in either,
    /// or the budget table differs from the check's.
    pub fn new(schema: &Value, budgets: Budgets) -> Result<Self, TrialError> {
        if !same_limits(budgets.compact, COMPACT_BUDGET)
            || !same_limits(budgets.standard, STANDARD_BUDGET)
        {
            return Err(TrialError::Invalid(
                "budgets.md differs from the handoff check's budget profiles".to_owned(),
            ));
        }
        let checker = HandoffChecker::with_schema(schema)
            .map_err(|error| TrialError::Invalid(format!("handoff.schema.json: {error}")))?;
        let reference = jsonschema::options()
            .build(schema)
            .map_err(|error| TrialError::Invalid(format!("handoff.schema.json: {error}")))?;
        Ok(Self { checker, reference })
    }

    /// Checks a final message's handoff block: letter case, the schema and
    /// the handoff rules. Its text is checked by [`text_problems`].
    #[must_use]
    pub fn check_report(&self, message: &str) -> ReportFindings {
        let check = self.checker.check_report(message);
        let mut problems: Vec<String> = check
            .errors()
            .iter()
            .filter(|finding| finding.rule().scope() != HandoffRuleScope::ReportText)
            .map(describe)
            .collect();
        if let Some(handoff) = check.handoff() {
            let ours = !check
                .errors()
                .iter()
                .any(|finding| finding.rule().scope() == HandoffRuleScope::Schema);
            if ours != self.reference.is_valid(handoff) {
                problems.push(
                    "grader defect: the handoff check and jsonschema disagree on this handoff"
                        .to_owned(),
                );
            }
        }
        let notes = check
            .case_notes()
            .iter()
            .chain(check.warnings())
            .map(describe)
            .collect();
        ReportFindings {
            handoff: check.handoff().cloned(),
            problems,
            notes,
        }
    }
}

fn same_limits(parsed: BudgetLimits, known: HandoffBudgetLimits) -> bool {
    parsed.images_per_step == known.images_per_step
        && parsed.images_total == known.images_total
        && parsed.image_bytes == known.image_bytes
        && parsed.page_limit == known.page_limit
        && parsed.tool_calls == known.tool_calls
        && parsed.refinement_depth == known.refinement_depth
        && parsed.wall_time_s == known.wall_time_s
        && parsed.burst_frames == known.burst_frames
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_one_block_is_extracted() {
        let message = "## Problem\n\nx\n\n```vsift-handoff\n{\"handoff_version\": \"1\"}\n```\n";
        assert_eq!(
            extract(message).map(|value| value["handoff_version"].clone()),
            Ok(Value::String("1".to_owned()))
        );
        assert!(extract("no block").is_err());
        assert!(extract("```vsift-handoff\n{}\n```\n```vsift-handoff\n{}\n```").is_err());
        assert!(extract("```vsift-handoff\n{\n").is_err());
    }

    #[test]
    fn text_problems_add_the_private_markers() {
        let markers = PrivateMarkers {
            strings: vec!["c:\\vsift-trials".to_owned()],
        };
        assert!(text_problems("All fine at 10:32.", &markers).is_empty());
        for bad in [
            "see C:\\vsift-trials\\a",
            "open hxxps://x or https://x",
            "hidden \u{202E}text",
            "at /home/someone/x",
            "esc \u{1b}[31m",
            "named C:\\VSIFT-TRIALS",
        ] {
            assert!(!text_problems(bad, &markers).is_empty(), "{bad}");
        }
        assert_eq!(
            text_problems("in vsift-trials", &markers).len(),
            0,
            "only the whole root is a marker"
        );
    }
}
