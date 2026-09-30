//! The closed rule identifiers of `handoff.check` and their fixed prose.

use serde::Serialize;

/// Which part of a draft report a rule is about.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffRuleScope {
    /// Finding the one `vsift-handoff` block and reading its JSON.
    Block,
    /// `handoff.schema.json`.
    Schema,
    /// The rules of `references/handoff.md` that one schema cannot express.
    Handoff,
    /// The whole report's text: paths, links and raw hidden or control
    /// characters.
    ReportText,
    /// The cited identities against the records of a session (`--session`).
    Session,
    /// A closed value read in the schema's letter case.
    LetterCase,
}

/// One closed rule identifier of a `handoff.check` finding (published v1
/// values; `schemas/v1/handoff-check-data.schema.json` lists them).
///
/// A finding never repeats text from the draft (SEC-16): it names a JSON
/// pointer made of the schema's own member names and array indices, a line
/// number, this rule, the values the schema allows where the rule has them,
/// and the rule's fixed prose.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffRule {
    /// The report has no `vsift-handoff` block.
    HandoffBlockMissing,
    /// The report has more than one `vsift-handoff` block.
    HandoffBlockRepeated,
    /// The `vsift-handoff` block is never closed.
    HandoffBlockUnclosed,
    /// The block is not one JSON value.
    HandoffJsonInvalid,
    /// The block nests objects and arrays deeper than the check reads.
    HandoffJsonTooDeep,
    /// The value has another JSON type than the schema allows.
    WrongType,
    /// A closed value is not one of the allowed words.
    ValueNotAllowed,
    /// A required member is missing.
    MemberMissing,
    /// An object has a member the schema does not define.
    MemberUnknown,
    /// Text is shorter than the schema allows.
    TextTooShort,
    /// Text is longer than the schema allows.
    TextTooLong,
    /// An identity or other value does not have its required form.
    FormMismatch,
    /// Text holds something the schema forbids (a path, a link, a raw
    /// hidden or control character).
    TextForbidden,
    /// A value is not allowed together with the object's other members.
    ValueForbidden,
    /// A list has fewer items than the schema requires.
    TooFewItems,
    /// A list has more items than the schema allows.
    TooManyItems,
    /// A list repeats an item that must be unique.
    DuplicateItems,
    /// A number is below its minimum.
    BelowMinimum,
    /// A number is above its maximum.
    AboveMaximum,
    /// A value matches none of the shapes allowed there.
    NoMatchingShape,
    /// A value matches more than one of the shapes allowed there.
    AmbiguousShape,
    /// Two citations share an `id`.
    CitationIdRepeated,
    /// A claim or instruction cites an `e` id that `citations` lacks.
    CitationMissing,
    /// A frame or crop claims inspected pixels without image access.
    PixelsWithoutImageAccess,
    /// A claim that rests on evidence cites only images nobody looked at.
    UninspectedImagesOnly,
    /// The work was cut short and can continue, but there is no resume card.
    ResumeCardMissing,
    /// The resume card is larger than 2 KiB.
    ResumeCardTooLarge,
    /// A finding to verify again ends before it starts.
    WindowReversed,
    /// A given budget limit differs from the profile without an override.
    BudgetLimitDiffers,
    /// A citation that no claim or instruction uses (a warning).
    CitationUnused,
    /// A closed value written in another letter case, read as the schema's
    /// spelling (a note).
    LetterCase,
    /// The report holds a raw hidden or bidirectional character.
    HiddenCharacter,
    /// The report holds a raw control character.
    ControlCharacter,
    /// The report holds a live web link.
    WebLink,
    /// The report holds a local path, a drive letter or a home folder.
    LocalPath,
    /// The handoff names another session than the one checked.
    SessionMismatch,
    /// A cited transcript segment is not in the session's transcripts.
    SegmentNotInSession,
    /// A cited evidence identity is not in the session's evidence records.
    EvidenceNotInSession,
    /// A cited evidence identity is in the session as another kind.
    EvidenceKindMismatch,
}

impl HandoffRule {
    /// Every rule, in declaration order.
    pub const ALL: [Self; 39] = [
        Self::HandoffBlockMissing,
        Self::HandoffBlockRepeated,
        Self::HandoffBlockUnclosed,
        Self::HandoffJsonInvalid,
        Self::HandoffJsonTooDeep,
        Self::WrongType,
        Self::ValueNotAllowed,
        Self::MemberMissing,
        Self::MemberUnknown,
        Self::TextTooShort,
        Self::TextTooLong,
        Self::FormMismatch,
        Self::TextForbidden,
        Self::ValueForbidden,
        Self::TooFewItems,
        Self::TooManyItems,
        Self::DuplicateItems,
        Self::BelowMinimum,
        Self::AboveMaximum,
        Self::NoMatchingShape,
        Self::AmbiguousShape,
        Self::CitationIdRepeated,
        Self::CitationMissing,
        Self::PixelsWithoutImageAccess,
        Self::UninspectedImagesOnly,
        Self::ResumeCardMissing,
        Self::ResumeCardTooLarge,
        Self::WindowReversed,
        Self::BudgetLimitDiffers,
        Self::CitationUnused,
        Self::LetterCase,
        Self::HiddenCharacter,
        Self::ControlCharacter,
        Self::WebLink,
        Self::LocalPath,
        Self::SessionMismatch,
        Self::SegmentNotInSession,
        Self::EvidenceNotInSession,
        Self::EvidenceKindMismatch,
    ];

    /// The rule's published identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::HandoffBlockMissing => "handoff_block_missing",
            Self::HandoffBlockRepeated => "handoff_block_repeated",
            Self::HandoffBlockUnclosed => "handoff_block_unclosed",
            Self::HandoffJsonInvalid => "handoff_json_invalid",
            Self::HandoffJsonTooDeep => "handoff_json_too_deep",
            Self::WrongType => "wrong_type",
            Self::ValueNotAllowed => "value_not_allowed",
            Self::MemberMissing => "member_missing",
            Self::MemberUnknown => "member_unknown",
            Self::TextTooShort => "text_too_short",
            Self::TextTooLong => "text_too_long",
            Self::FormMismatch => "form_mismatch",
            Self::TextForbidden => "text_forbidden",
            Self::ValueForbidden => "value_forbidden",
            Self::TooFewItems => "too_few_items",
            Self::TooManyItems => "too_many_items",
            Self::DuplicateItems => "duplicate_items",
            Self::BelowMinimum => "below_minimum",
            Self::AboveMaximum => "above_maximum",
            Self::NoMatchingShape => "no_matching_shape",
            Self::AmbiguousShape => "ambiguous_shape",
            Self::CitationIdRepeated => "citation_id_repeated",
            Self::CitationMissing => "citation_missing",
            Self::PixelsWithoutImageAccess => "pixels_without_image_access",
            Self::UninspectedImagesOnly => "uninspected_images_only",
            Self::ResumeCardMissing => "resume_card_missing",
            Self::ResumeCardTooLarge => "resume_card_too_large",
            Self::WindowReversed => "window_reversed",
            Self::BudgetLimitDiffers => "budget_limit_differs",
            Self::CitationUnused => "citation_unused",
            Self::LetterCase => "letter_case",
            Self::HiddenCharacter => "hidden_character",
            Self::ControlCharacter => "control_character",
            Self::WebLink => "web_link",
            Self::LocalPath => "local_path",
            Self::SessionMismatch => "session_mismatch",
            Self::SegmentNotInSession => "segment_not_in_session",
            Self::EvidenceNotInSession => "evidence_not_in_session",
            Self::EvidenceKindMismatch => "evidence_kind_mismatch",
        }
    }

    /// The variant's position in [`HandoffRule::ALL`]: an exhaustive match,
    /// so a new rule does not compile until it has a position, and the
    /// constant assertion below requires `ALL` to list each once, in order.
    const fn ordinal(self) -> usize {
        match self {
            Self::HandoffBlockMissing => 0,
            Self::HandoffBlockRepeated => 1,
            Self::HandoffBlockUnclosed => 2,
            Self::HandoffJsonInvalid => 3,
            Self::HandoffJsonTooDeep => 4,
            Self::WrongType => 5,
            Self::ValueNotAllowed => 6,
            Self::MemberMissing => 7,
            Self::MemberUnknown => 8,
            Self::TextTooShort => 9,
            Self::TextTooLong => 10,
            Self::FormMismatch => 11,
            Self::TextForbidden => 12,
            Self::ValueForbidden => 13,
            Self::TooFewItems => 14,
            Self::TooManyItems => 15,
            Self::DuplicateItems => 16,
            Self::BelowMinimum => 17,
            Self::AboveMaximum => 18,
            Self::NoMatchingShape => 19,
            Self::AmbiguousShape => 20,
            Self::CitationIdRepeated => 21,
            Self::CitationMissing => 22,
            Self::PixelsWithoutImageAccess => 23,
            Self::UninspectedImagesOnly => 24,
            Self::ResumeCardMissing => 25,
            Self::ResumeCardTooLarge => 26,
            Self::WindowReversed => 27,
            Self::BudgetLimitDiffers => 28,
            Self::CitationUnused => 29,
            Self::LetterCase => 30,
            Self::HiddenCharacter => 31,
            Self::ControlCharacter => 32,
            Self::WebLink => 33,
            Self::LocalPath => 34,
            Self::SessionMismatch => 35,
            Self::SegmentNotInSession => 36,
            Self::EvidenceNotInSession => 37,
            Self::EvidenceKindMismatch => 38,
        }
    }

    /// Which part of the report the rule is about.
    #[must_use]
    pub const fn scope(self) -> HandoffRuleScope {
        match self {
            Self::HandoffBlockMissing
            | Self::HandoffBlockRepeated
            | Self::HandoffBlockUnclosed
            | Self::HandoffJsonInvalid
            | Self::HandoffJsonTooDeep => HandoffRuleScope::Block,
            Self::WrongType
            | Self::ValueNotAllowed
            | Self::MemberMissing
            | Self::MemberUnknown
            | Self::TextTooShort
            | Self::TextTooLong
            | Self::FormMismatch
            | Self::TextForbidden
            | Self::ValueForbidden
            | Self::TooFewItems
            | Self::TooManyItems
            | Self::DuplicateItems
            | Self::BelowMinimum
            | Self::AboveMaximum
            | Self::NoMatchingShape
            | Self::AmbiguousShape => HandoffRuleScope::Schema,
            Self::CitationIdRepeated
            | Self::CitationMissing
            | Self::PixelsWithoutImageAccess
            | Self::UninspectedImagesOnly
            | Self::ResumeCardMissing
            | Self::ResumeCardTooLarge
            | Self::WindowReversed
            | Self::BudgetLimitDiffers
            | Self::CitationUnused => HandoffRuleScope::Handoff,
            Self::LetterCase => HandoffRuleScope::LetterCase,
            Self::HiddenCharacter | Self::ControlCharacter | Self::WebLink | Self::LocalPath => {
                HandoffRuleScope::ReportText
            }
            Self::SessionMismatch
            | Self::SegmentNotInSession
            | Self::EvidenceNotInSession
            | Self::EvidenceKindMismatch => HandoffRuleScope::Session,
        }
    }

    /// The rule's fixed prose. A schema rule may carry more specific fixed
    /// prose for one pattern or combination instead (`messages`).
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::HandoffBlockMissing => {
                "The report has no vsift-handoff block; end it with exactly one fenced vsift-handoff block holding the JSON."
            }
            Self::HandoffBlockRepeated => {
                "The report has more than one vsift-handoff block; keep exactly one, at the end."
            }
            Self::HandoffBlockUnclosed => {
                "The vsift-handoff block is not closed; end it with a fence line of at least as many backticks."
            }
            Self::HandoffJsonInvalid => {
                "The vsift-handoff block is not one valid JSON value; fix the JSON syntax near this line."
            }
            Self::HandoffJsonTooDeep => {
                "The vsift-handoff block nests objects and arrays too deeply; use the shape handoff.md shows."
            }
            Self::WrongType => "This value has the wrong JSON type; use one of the allowed types.",
            Self::ValueNotAllowed => {
                "This closed value is not allowed; write exactly one of the allowed values."
            }
            Self::MemberMissing => "This required member is missing; add it.",
            Self::MemberUnknown => {
                "This object has a member the schema does not define; use only the allowed members."
            }
            Self::TextTooShort => "This text is empty or too short; write it out.",
            Self::TextTooLong => "This text is too long; shorten it.",
            Self::FormMismatch => {
                "This value does not have its required form; copy it exactly as VSift printed it."
            }
            Self::TextForbidden => {
                "This text holds something the handoff forbids: a raw control or hidden character, an absolute path, a home folder or a link."
            }
            Self::ValueForbidden => {
                "This value is not allowed together with the object's other members."
            }
            Self::TooFewItems => "This list needs more items.",
            Self::TooManyItems => "This list has too many items; keep the most important ones.",
            Self::DuplicateItems => "This list repeats an item; list each item once.",
            Self::BelowMinimum => "This number is below its minimum.",
            Self::AboveMaximum => "This number is above its maximum.",
            Self::NoMatchingShape => {
                "This value matches none of the shapes allowed here; compare it with handoff.md."
            }
            Self::AmbiguousShape => {
                "This value matches more than one of the shapes allowed here; keep only the members of one."
            }
            Self::CitationIdRepeated => {
                "This citation repeats an id another citation already has; give each citation its own e id."
            }
            Self::CitationMissing => {
                "This cites an e id that is not in citations; add the citation or cite an existing one."
            }
            Self::PixelsWithoutImageAccess => {
                "Image access is unavailable, so pixels_inspected must be false."
            }
            Self::UninspectedImagesOnly => {
                "This claim rests on evidence but cites only images whose pixels were not inspected; cite a transcript segment or an inspected image, or mark it unsupported."
            }
            Self::ResumeCardMissing => {
                "The work was cut short and can continue (an exhausted budget, or a cancelled or interrupted transcription), so add the resume card resume.md shows."
            }
            Self::ResumeCardTooLarge => {
                "The resume card is larger than 2 KiB; keep only what the next run needs."
            }
            Self::WindowReversed => {
                "This finding's window ends before it starts; from_us must be at most to_us."
            }
            Self::BudgetLimitDiffers => {
                "This limit differs from the profile's; leave budget.limits out, or set budget.overrides to true when the user changed it."
            }
            Self::CitationUnused => {
                "No claim or instruction uses this citation; cite it or leave it out."
            }
            Self::LetterCase => {
                "This closed value was read in the schema's letter case; write it exactly as listed."
            }
            Self::HiddenCharacter => {
                "This line holds a raw hidden or bidirectional character; quote evidence from display_text, where it reads as <U+XXXX>."
            }
            Self::ControlCharacter => "This line holds a raw control character; remove it.",
            Self::WebLink => {
                "This line holds a web link; write an address seen in evidence only defanged (hxxps) in a code span of the Markdown, and never in the JSON."
            }
            Self::LocalPath => {
                "This line holds a local path, a drive letter or a home folder; cite evidence ids instead."
            }
            Self::SessionMismatch => {
                "This names another session than the one checked; name the session whose evidence you cite."
            }
            Self::SegmentNotInSession => {
                "This transcript segment is not in the session's transcripts; copy the segment_id from transcript get or search."
            }
            Self::EvidenceNotInSession => {
                "This evidence id is not in the session's evidence records; copy the evidence_id from the command that extracted it."
            }
            Self::EvidenceKindMismatch => {
                "This evidence id is in the session as another kind of evidence; cite it with the type of the command that extracted it."
            }
        }
    }
}

// Compile-time guard for `HandoffRule::ALL` (see `ordinal`).
const _: () = {
    let mut index = 0;
    while index < HandoffRule::ALL.len() {
        assert!(HandoffRule::ALL[index].ordinal() == index);
        index += 1;
    }
};

/// One problem, warning or note of a `handoff.check`.
///
/// Every member is either `VSift`'s own text or a value the draft cannot
/// choose: the pointer is built from the schema's member names and array
/// indices only (an object's unknown member is reported at the object), the
/// line is a number, and `allowed` holds the schema's own values.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HandoffFinding {
    pointer: Option<String>,
    line: Option<u32>,
    rule: HandoffRule,
    allowed: Option<Vec<String>>,
    message: &'static str,
}

impl HandoffFinding {
    /// A finding at a JSON pointer into the handoff.
    #[must_use]
    pub(crate) const fn at(
        pointer: String,
        rule: HandoffRule,
        allowed: Option<Vec<String>>,
        message: &'static str,
    ) -> Self {
        Self {
            pointer: Some(pointer),
            line: None,
            rule,
            allowed,
            message,
        }
    }

    /// A finding at a JSON pointer with the rule's own prose.
    #[must_use]
    pub(crate) const fn rule_at(pointer: String, rule: HandoffRule) -> Self {
        Self::at(pointer, rule, None, rule.message())
    }

    /// A finding about a line of the report, outside any JSON pointer.
    #[must_use]
    pub(crate) const fn on_line(line: Option<u32>, rule: HandoffRule) -> Self {
        Self {
            pointer: None,
            line,
            rule,
            allowed: None,
            message: rule.message(),
        }
    }

    /// The RFC 6901 pointer into the handoff JSON, when the finding is
    /// about one of its values.
    #[must_use]
    pub fn pointer(&self) -> Option<&str> {
        self.pointer.as_deref()
    }

    /// The 1-based line of the report, when the finding is about the
    /// report's text or its block.
    #[must_use]
    pub const fn line(&self) -> Option<u32> {
        self.line
    }

    /// The rule.
    #[must_use]
    pub const fn rule(&self) -> HandoffRule {
        self.rule
    }

    /// The allowed values, when the rule has them.
    #[must_use]
    pub fn allowed(&self) -> Option<&[String]> {
        self.allowed.as_deref()
    }

    /// The fixed prose.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }
}

#[cfg(test)]
mod tests {
    use super::HandoffRule;

    #[test]
    fn identifiers_are_distinct_and_serialize_as_themselves() -> Result<(), serde_json::Error> {
        for (index, rule) in HandoffRule::ALL.into_iter().enumerate() {
            assert!(
                HandoffRule::ALL[index + 1..]
                    .iter()
                    .all(|other| other.identifier() != rule.identifier())
            );
            assert_eq!(
                serde_json::to_value(rule)?,
                serde_json::Value::from(rule.identifier())
            );
        }
        Ok(())
    }
}
