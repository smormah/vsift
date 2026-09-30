//! `handoff check` (P13 PR 5, issue #213, ADR 0023 decisions G and H): the
//! one check of an agent's draft report, shared by the CLI command and the
//! trial grader so that the two can never disagree.
//!
//! A report is Markdown that ends with one fenced `vsift-handoff` block of
//! JSON. [`HandoffChecker::check_report`] reads the whole report:
//!
//! 1. it finds the one block and parses it (bounded, strict UTF-8 JSON);
//! 2. it reads every closed value written in another letter case as the
//!    schema's spelling and notes it ([`HandoffRule::LetterCase`]);
//! 3. it validates the JSON against `skills/vsift/handoff.schema.json`,
//!    embedded here with `include_str!` so the skill keeps owning it, with
//!    the subset validator of the `schema` module;
//! 4. it applies the rules of `references/handoff.md` that one schema cannot
//!    express (every cited `e` id exists, a resume card when the work can
//!    continue, a card of at most 2 KiB, visual claims on inspected pixels,
//!    given budget limits are the profile's);
//! 5. it checks the whole report's text for paths, links and raw hidden or
//!    control characters.
//!
//! [`HandoffCheck::resolve_in_session`] then optionally resolves the cited
//! identities against one session's records, which a host reads.
//!
//! **Nothing of the draft is repeated** in a finding (SEC-16): the draft can
//! carry hostile evidence text. A finding is a JSON pointer made of the
//! schema's own member names and array indices, a line number, a closed
//! [`HandoffRule`], the schema's allowed values and fixed prose.

mod block;
mod budget;
mod case;
mod report_text;
mod rule;
mod rules;
mod schema;
mod session;

use serde::Serialize;
use serde_json::Value;

pub use budget::{COMPACT_BUDGET, HandoffBudgetLimits, STANDARD_BUDGET, budget_profile};
pub use case::{HandoffVocabulary, handoff_vocabulary};
pub use rule::{HandoffFinding, HandoffRule, HandoffRuleScope};
pub use rules::{MAX_RESUME_CARD_BYTES, handoff_is_cut_short};
pub use schema::HandoffSchemaError;
pub use session::{
    HandoffEvidenceKind, HandoffSessionCheck, HandoffSessionGap, HandoffSessionRecords,
};

use schema::CompiledSchema;

/// `skills/vsift/handoff.schema.json`, the skill-owned handoff v1 schema,
/// as the binary embeds it. A test checks it equals the skill's file.
pub const HANDOFF_SCHEMA_JSON: &str = include_str!("../../../../skills/vsift/handoff.schema.json");

/// The handoff version this check reads (the schema's `handoff_version`).
pub const HANDOFF_VERSION: &str = "1";

/// The largest report `handoff check` reads: 64 KiB of UTF-8.
pub const MAX_HANDOFF_REPORT_BYTES: usize = 64 * 1024;

/// The most findings of one kind (errors, warnings, case notes) a result
/// lists; `truncated` says when more were found.
pub const MAX_HANDOFF_FINDINGS: usize = 100;

/// The remediation of a draft larger than [`MAX_HANDOFF_REPORT_BYTES`].
pub const HANDOFF_DRAFT_TOO_LARGE_REMEDIATION: &str =
    "The draft report is larger than 64 KiB; shorten it and check it again.";

/// The remediation of a draft that is not UTF-8 text.
pub const HANDOFF_DRAFT_NOT_UTF8_REMEDIATION: &str = "The draft report is not UTF-8 text; pass the report itself, in the quoted heredoc or single-quoted here-string the skill shows, or with --file.";

/// The remediation of a `--file` that is relative or cannot be read.
pub const HANDOFF_FILE_REMEDIATION: &str =
    "--file needs the absolute path of an existing, readable file holding the draft report.";

/// The compiled handoff schema and its closed vocabularies.
#[derive(Debug)]
pub struct HandoffChecker {
    schema: CompiledSchema,
    vocabulary: HandoffVocabulary,
}

impl HandoffChecker {
    /// Compiles the embedded [`HANDOFF_SCHEMA_JSON`].
    ///
    /// # Errors
    ///
    /// [`HandoffSchemaError`] when the schema is not JSON or uses a feature
    /// the check does not implement; a test keeps this from shipping.
    pub fn new() -> Result<Self, HandoffSchemaError> {
        let schema: Value =
            serde_json::from_str(HANDOFF_SCHEMA_JSON).map_err(|_| HandoffSchemaError::NotJson)?;
        Self::with_schema(&schema)
    }

    /// Compiles another copy of the handoff schema: the trial grader reads
    /// the copy of the skill a trial ran with.
    ///
    /// # Errors
    ///
    /// As [`HandoffChecker::new`].
    pub fn with_schema(schema: &Value) -> Result<Self, HandoffSchemaError> {
        Ok(Self {
            schema: CompiledSchema::compile(schema)?,
            vocabulary: handoff_vocabulary(schema),
        })
    }

    /// The schema's closed vocabularies.
    #[must_use]
    pub const fn vocabulary(&self) -> &HandoffVocabulary {
        &self.vocabulary
    }

    /// Checks a whole draft report: its one block, the schema, the handoff
    /// rules and the report's text.
    #[must_use]
    pub fn check_report(&self, report: &str) -> HandoffCheck {
        let mut check = match block::extract(report, MAX_HANDOFF_REPORT_BYTES) {
            Ok(block) => self.check_value(block.value),
            Err(finding) => HandoffCheck {
                handoff: None,
                errors: vec![finding],
                warnings: Vec::new(),
                case_notes: Vec::new(),
                session: None,
            },
        };
        check
            .errors
            .extend(report_text::report_text_findings(report));
        check
    }

    /// Checks one handoff JSON value on its own (no report text): letter
    /// case, the schema and the handoff rules.
    #[must_use]
    pub fn check_value(&self, mut handoff: Value) -> HandoffCheck {
        let case_notes = case::normalize_case(&self.vocabulary, &mut handoff);
        let mut errors: Vec<HandoffFinding> = self
            .schema
            .errors(&handoff)
            .into_iter()
            .map(schema::SchemaError::into_finding)
            .collect();
        let rules = rules::rule_findings(&handoff);
        errors.extend(rules.errors);
        HandoffCheck {
            handoff: Some(handoff),
            errors,
            warnings: rules.warnings,
            case_notes,
            session: None,
        }
    }

    /// The instance pointers of the schema errors alone, for the
    /// differential test against a general JSON Schema validator.
    #[doc(hidden)]
    #[must_use]
    pub fn schema_error_pointers(&self, handoff: &Value) -> Vec<String> {
        self.schema
            .errors(handoff)
            .iter()
            .map(|error| error.pointer().to_owned())
            .collect()
    }
}

/// The one `vsift-handoff` block of `report`, parsed (bounded, strict
/// JSON), before any other check.
///
/// # Errors
///
/// The [`HandoffRuleScope::Block`] finding that says why there is no one
/// readable block.
pub fn extract_handoff_block(report: &str) -> Result<Value, HandoffFinding> {
    block::extract(report, MAX_HANDOFF_REPORT_BYTES).map(|block| block.value)
}

/// The report-text findings of `report` alone: local paths, links and raw
/// hidden or control characters, by line
/// ([`HandoffRuleScope::ReportText`]).
#[must_use]
pub fn handoff_report_text_findings(report: &str) -> Vec<HandoffFinding> {
    report_text::report_text_findings(report)
}

/// Whether `pattern` (a pattern of the handoff schema) has fixed prose of
/// its own; a test requires one for every pattern the schema uses.
#[doc(hidden)]
#[must_use]
pub fn handoff_pattern_has_message(pattern: &str) -> bool {
    schema::has_pattern_message(pattern)
}

/// The outcome of one check, before it is published.
#[derive(Clone, Debug, PartialEq)]
pub struct HandoffCheck {
    handoff: Option<Value>,
    errors: Vec<HandoffFinding>,
    warnings: Vec<HandoffFinding>,
    case_notes: Vec<HandoffFinding>,
    session: Option<HandoffSessionCheck>,
}

impl HandoffCheck {
    /// The handoff JSON with its closed values in the schema's letter case,
    /// when the report had one readable block.
    #[must_use]
    pub const fn handoff(&self) -> Option<&Value> {
        self.handoff.as_ref()
    }

    /// Whether the draft passed every check that ran.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// Every problem.
    #[must_use]
    pub fn errors(&self) -> &[HandoffFinding] {
        &self.errors
    }

    /// Harmless departures, such as a citation no claim uses.
    #[must_use]
    pub fn warnings(&self) -> &[HandoffFinding] {
        &self.warnings
    }

    /// Closed values read in the schema's letter case.
    #[must_use]
    pub fn case_notes(&self) -> &[HandoffFinding] {
        &self.case_notes
    }

    /// How the session was used, when one was named.
    #[must_use]
    pub const fn session(&self) -> Option<&HandoffSessionCheck> {
        self.session.as_ref()
    }

    /// Resolves the cited identities against `records` of `session_id`.
    /// Without a readable handoff nothing is cited, so nothing is checked.
    pub fn resolve_in_session(&mut self, session_id: &str, records: &HandoffSessionRecords) {
        let (check, findings) = session::resolve(
            self.handoff.as_ref().unwrap_or(&Value::Null),
            session_id,
            records,
        );
        self.errors.extend(findings);
        self.session = Some(check);
    }

    /// Records that `session_id` could not be read for `gap`.
    pub fn session_unavailable(&mut self, session_id: &str, gap: HandoffSessionGap) {
        self.session = Some(HandoffSessionCheck::gap(session_id, gap));
    }
}

/// The `data` of a `handoff.check` result
/// (`schemas/v1/handoff-check-data.schema.json`).
///
/// `valid` is whether the draft passed every check that ran; the command
/// answers with it and exits 0 whenever it could read the draft (ADR 0023
/// decision H1).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HandoffCheckData {
    valid: bool,
    handoff_version: &'static str,
    errors: Vec<HandoffFinding>,
    warnings: Vec<HandoffFinding>,
    case_notes: Vec<HandoffFinding>,
    truncated: bool,
    session: Option<HandoffSessionCheck>,
}

impl HandoffCheckData {
    /// The published form of `check`, each list bounded to
    /// [`MAX_HANDOFF_FINDINGS`].
    #[must_use]
    pub fn new(check: HandoffCheck) -> Self {
        let valid = check.is_valid();
        let truncated = [&check.errors, &check.warnings, &check.case_notes]
            .iter()
            .any(|list| list.len() > MAX_HANDOFF_FINDINGS);
        let bounded = |mut list: Vec<HandoffFinding>| {
            list.truncate(MAX_HANDOFF_FINDINGS);
            list
        };
        Self {
            valid,
            handoff_version: HANDOFF_VERSION,
            errors: bounded(check.errors),
            warnings: bounded(check.warnings),
            case_notes: bounded(check.case_notes),
            truncated,
            session: check.session,
        }
    }

    /// Whether the draft passed.
    #[must_use]
    pub const fn valid(&self) -> bool {
        self.valid
    }
}

#[cfg(test)]
mod tests;
