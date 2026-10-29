//! `handoff skeleton` (ADR 0038 section 1): a handoff that is valid but states
//! nothing yet, with the identities `VSift` already knows filled in.
//!
//! An agent that builds the handoff JSON from a text template it fills in by
//! search and replace can corrupt it silently: a case-insensitive replace of a
//! placeholder word also rewrites the member names that spell it. The skeleton
//! removes the template. `VSift` writes the JSON, from its own records, and the
//! agent adds only what it alone knows. The skeleton is checked with the same
//! [`HandoffChecker`] as a draft, so it can never be a shape `handoff check`
//! refuses.

use serde::Serialize;
use serde_json::{Map, Value, json};

use super::{HANDOFF_VERSION, HandoffChecker};

/// The remediation of a `--question` the handoff schema refuses.
pub const HANDOFF_SKELETON_QUESTION_REMEDIATION: &str = "--question needs the user's question in your own words: 1 to 500 characters of plain text, with no line break, link, local path or hidden character.";

/// The members only the agent can supply, in the order it fills them, as
/// RFC 6901 pointers into the handoff. Published in `data.fill`.
pub const HANDOFF_SKELETON_FILL: [&str; 7] = [
    "/capabilities",
    "/claims",
    "/citations",
    "/gaps",
    "/untrusted_instructions",
    "/status",
    "/lifecycle",
];

/// How the session's newest transcript revision was made, as a handoff's
/// `capabilities.transcript_basis` names it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffTranscriptBasis {
    /// A supplied `SubRip` or `WebVTT` file.
    SuppliedTranscript,
    /// Local speech recognition only.
    LocalAsr,
    /// Local speech recognition spliced into supplied text.
    Mixed,
    /// The session has no transcript revision.
    None,
}

impl HandoffTranscriptBasis {
    /// The handoff schema's spelling.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::SuppliedTranscript => "supplied_transcript",
            Self::LocalAsr => "local_asr",
            Self::Mixed => "mixed",
            Self::None => "none",
        }
    }
}

/// What `VSift` knows of one open session that a handoff may state: read by
/// the host from the session's committed records, never from the agent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HandoffSkeletonFacts {
    session_id: String,
    source_id: String,
    revision_id: Option<String>,
    transcript_basis: HandoffTranscriptBasis,
}

impl HandoffSkeletonFacts {
    /// The facts of a session, with its newest transcript revision when it
    /// has one (`revision_id` is `None` exactly when `transcript_basis` is
    /// [`HandoffTranscriptBasis::None`]; the host keeps that so).
    #[must_use]
    pub const fn new(
        session_id: String,
        source_id: String,
        revision_id: Option<String>,
        transcript_basis: HandoffTranscriptBasis,
    ) -> Self {
        Self {
            session_id,
            source_id,
            revision_id,
            transcript_basis,
        }
    }
}

/// Why a skeleton could not be written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffSkeletonError {
    /// The handoff schema refuses the question as written; the caller's
    /// input error ([`HANDOFF_SKELETON_QUESTION_REMEDIATION`]).
    QuestionRefused,
    /// The skeleton `VSift` built failed its own check: a defect, which a
    /// test keeps from shipping.
    NotValid,
}

/// The `data` of a `handoff.skeleton` result
/// (`schemas/v1/handoff-skeleton-data.schema.json`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HandoffSkeletonData {
    handoff_version: &'static str,
    handoff: Value,
    fill: [&'static str; 7],
}

impl HandoffSkeletonData {
    /// The handoff JSON: a complete handoff v1 that `handoff check` accepts.
    #[must_use]
    pub const fn handoff(&self) -> &Value {
        &self.handoff
    }
}

impl HandoffChecker {
    /// Writes the skeleton handoff for `question` in the session `facts`
    /// describes, and checks it as a draft would be checked.
    ///
    /// It states nothing the agent alone knows: status
    /// `insufficient_evidence`, image access `unavailable` (the agent's own
    /// image check may change it), no claims, citations, gaps or untrusted
    /// instructions, and the session left open.
    ///
    /// # Errors
    ///
    /// [`HandoffSkeletonError::QuestionRefused`] when the schema refuses the
    /// question; [`HandoffSkeletonError::NotValid`] when anything else fails
    /// the check.
    pub fn skeleton(
        &self,
        question: &str,
        facts: &HandoffSkeletonFacts,
    ) -> Result<HandoffSkeletonData, HandoffSkeletonError> {
        let mut session = Map::new();
        session.insert("session_id".to_owned(), json!(facts.session_id));
        session.insert("source_id".to_owned(), json!(facts.source_id));
        if let Some(revision_id) = &facts.revision_id {
            session.insert("revision_id".to_owned(), json!(revision_id));
        }
        let handoff = json!({
            "handoff_version": HANDOFF_VERSION,
            "status": "insufficient_evidence",
            "question": question,
            "capabilities": {
                "image_access": "unavailable",
                "transcript_basis": facts.transcript_basis.identifier(),
            },
            "session": Value::Object(session),
            "claims": [],
            "citations": [],
            "gaps": [],
            "untrusted_instructions": [],
            "lifecycle": {"action": "left_open"},
        });
        let check = self.check_value(handoff.clone());
        if check.errors().is_empty() && check.case_notes().is_empty() {
            return Ok(HandoffSkeletonData {
                handoff_version: HANDOFF_VERSION,
                handoff,
                fill: HANDOFF_SKELETON_FILL,
            });
        }
        let question_only = check
            .errors()
            .iter()
            .all(|finding| finding.pointer() == Some("/question"));
        Err(if question_only {
            HandoffSkeletonError::QuestionRefused
        } else {
            HandoffSkeletonError::NotValid
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HandoffChecker, HandoffSkeletonError, HandoffSkeletonFacts, HandoffTranscriptBasis,
    };

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
    const SOURCE: &str =
        "src_sha256_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const REVISION: &str = "trv_0123456789abcdef0123456789abcdef";

    fn facts(revision: Option<&str>, basis: HandoffTranscriptBasis) -> HandoffSkeletonFacts {
        HandoffSkeletonFacts::new(
            SESSION.to_owned(),
            SOURCE.to_owned(),
            revision.map(str::to_owned),
            basis,
        )
    }

    #[test]
    fn a_skeleton_passes_the_check_a_draft_gets() -> TestResult {
        let checker = HandoffChecker::new()?;
        for (revision, basis) in [
            (Some(REVISION), HandoffTranscriptBasis::SuppliedTranscript),
            (Some(REVISION), HandoffTranscriptBasis::LocalAsr),
            (Some(REVISION), HandoffTranscriptBasis::Mixed),
            (None, HandoffTranscriptBasis::None),
        ] {
            let data = checker
                .skeleton(
                    "What does the dialog show after Save?",
                    &facts(revision, basis),
                )
                .map_err(|error| format!("{error:?}"))?;
            let check = checker.check_value(data.handoff().clone());
            assert!(check.is_valid(), "{:?}", check.errors());
            assert_eq!(check.case_notes(), &[]);
            assert_eq!(data.handoff()["session"]["session_id"], SESSION);
            assert_eq!(
                data.handoff()["capabilities"]["transcript_basis"],
                basis.identifier()
            );
            assert_eq!(
                data.handoff()["session"].get("revision_id").is_some(),
                revision.is_some()
            );
            assert_eq!(data.handoff()["claims"], serde_json::json!([]));
        }
        Ok(())
    }

    #[test]
    fn a_question_the_schema_refuses_is_the_callers_error() -> TestResult {
        let checker = HandoffChecker::new()?;
        let basis = HandoffTranscriptBasis::None;
        for question in ["", "two\nlines", &"x".repeat(501)] {
            assert_eq!(
                checker.skeleton(question, &facts(None, basis)),
                Err(HandoffSkeletonError::QuestionRefused),
                "{question:?}"
            );
        }
        Ok(())
    }
}
