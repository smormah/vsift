//! The rules of `references/handoff.md` that one schema cannot express.
//!
//! They read the handoff leniently, so they report what they can even when
//! the schema also refuses the handoff; a value of the wrong type is simply
//! not read here.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{
    budget::budget_profile,
    rule::{HandoffFinding, HandoffRule},
    schema::{child_pointer, index_pointer},
};

/// The largest serialized resume card the skill allows.
pub const MAX_RESUME_CARD_BYTES: usize = 2 * 1024;

/// The gap reasons that say work stopped early and another run can pick it
/// up: the agent's budget ran out (`budget_exhausted`), or a transcription
/// was cancelled or interrupted with its checkpoints kept (`cancelled`).
const RESUMABLE_GAP_REASONS: [&str; 2] = ["budget_exhausted", "cancelled"];

/// Problems and warnings of the handoff rules.
pub(crate) struct RuleFindings {
    pub(crate) errors: Vec<HandoffFinding>,
    pub(crate) warnings: Vec<HandoffFinding>,
}

/// Every rule problem and warning of `handoff`.
///
/// A citation that no claim or instruction uses is a warning, not a
/// problem (2026-09-29, P12 PR 3g): it names real evidence and misleads no
/// reader. A claim that rests on evidence but cites none is refused by the
/// schema (`minItems`), so it is not reported again here.
pub(crate) fn rule_findings(handoff: &Value) -> RuleFindings {
    let mut findings = RuleFindings {
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    citation_findings(handoff, &mut findings);
    resume_findings(handoff, &mut findings.errors);
    budget_findings(handoff, &mut findings.errors);
    findings
}

fn is_visual(citation: &Value) -> bool {
    matches!(citation["type"].as_str(), Some("frame" | "crop"))
}

fn citation_findings(handoff: &Value, findings: &mut RuleFindings) {
    let citations: &[Value] = handoff["citations"].as_array().map_or(&[], Vec::as_slice);
    let mut by_id: BTreeMap<&str, (usize, &Value)> = BTreeMap::new();
    for (index, citation) in citations.iter().enumerate() {
        let Some(id) = citation["id"].as_str() else {
            continue;
        };
        if by_id.contains_key(id) {
            findings.errors.push(HandoffFinding::rule_at(
                child_pointer(&index_pointer("/citations", index), "id"),
                HandoffRule::CitationIdRepeated,
            ));
        } else {
            by_id.insert(id, (index, citation));
        }
    }
    if handoff["capabilities"]["image_access"] == "unavailable" {
        for (index, citation) in citations.iter().enumerate() {
            if is_visual(citation) && citation["pixels_inspected"] != false {
                findings.errors.push(HandoffFinding::rule_at(
                    child_pointer(&index_pointer("/citations", index), "pixels_inspected"),
                    HandoffRule::PixelsWithoutImageAccess,
                ));
            }
        }
    }
    let mut used = BTreeSet::new();
    for (claim_index, claim) in handoff["claims"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let claim_pointer = index_pointer("/claims", claim_index);
        let citations_pointer = child_pointer(&claim_pointer, "citations");
        let mut resolved = false;
        let mut grounded = false;
        for (reference_index, reference) in claim["citations"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let Some(reference) = reference.as_str() else {
                continue;
            };
            used.insert(reference.to_owned());
            match by_id.get(reference) {
                None => findings.errors.push(HandoffFinding::rule_at(
                    index_pointer(&citations_pointer, reference_index),
                    HandoffRule::CitationMissing,
                )),
                Some((_, citation)) => {
                    resolved = true;
                    grounded |= !is_visual(citation) || citation["pixels_inspected"] == true;
                }
            }
        }
        let rests_on_evidence = matches!(
            claim["support"].as_str(),
            Some("supported" | "partially_supported" | "contradicted")
        );
        if rests_on_evidence && resolved && !grounded {
            findings.errors.push(HandoffFinding::rule_at(
                citations_pointer,
                HandoffRule::UninspectedImagesOnly,
            ));
        }
    }
    for (index, item) in handoff["untrusted_instructions"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if let Some(reference) = item["citation"].as_str() {
            used.insert(reference.to_owned());
            if !by_id.contains_key(reference) {
                findings.errors.push(HandoffFinding::rule_at(
                    child_pointer(&index_pointer("/untrusted_instructions", index), "citation"),
                    HandoffRule::CitationMissing,
                ));
            }
        }
    }
    for (id, (index, _)) in &by_id {
        if !used.contains(*id) {
            findings.warnings.push(HandoffFinding::rule_at(
                index_pointer("/citations", *index),
                HandoffRule::CitationUnused,
            ));
        }
    }
    findings
        .warnings
        .sort_by(|left, right| left.pointer().cmp(&right.pointer()));
}

/// Whether the work was cut short and can continue, which is when a resume
/// card is required: an exhausted budget limit, or a gap saying the budget
/// ran out or a job was cancelled or interrupted (reason `cancelled` or code
/// `CANCELLED`). A report that is `partial` only because a capability is
/// missing (images, speech recognition, tools) or the session expired needs
/// none: resuming cannot fix those (supervisor's decision, 2026-09-29). A
/// card that is given is still validated in full.
#[must_use]
pub fn handoff_is_cut_short(handoff: &Value) -> bool {
    let exhausted = handoff["budget"]["exhausted"]
        .as_array()
        .is_some_and(|limits| !limits.is_empty());
    exhausted
        || handoff["gaps"].as_array().into_iter().flatten().any(|gap| {
            gap["reason"]
                .as_str()
                .is_some_and(|reason| RESUMABLE_GAP_REASONS.contains(&reason))
                || gap["code"] == "CANCELLED"
        })
}

fn resume_findings(handoff: &Value, errors: &mut Vec<HandoffFinding>) {
    let resume = &handoff["resume"];
    if resume.is_null() {
        if handoff_is_cut_short(handoff) {
            errors.push(HandoffFinding::rule_at(
                "/resume".to_owned(),
                HandoffRule::ResumeCardMissing,
            ));
        }
        return;
    }
    let size = serde_json::to_string(resume).map_or(usize::MAX, |text| text.len());
    if size > MAX_RESUME_CARD_BYTES {
        errors.push(HandoffFinding::rule_at(
            "/resume".to_owned(),
            HandoffRule::ResumeCardTooLarge,
        ));
    }
    // P12 PR 3i: a finding to verify again names the window it holds for.
    for (index, item) in resume["to_verify"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if let (Some(from), Some(to)) = (item["from_us"].as_u64(), item["to_us"].as_u64())
            && from > to
        {
            errors.push(HandoffFinding::rule_at(
                index_pointer("/resume/to_verify", index),
                HandoffRule::WindowReversed,
            ));
        }
    }
}

/// `budget.limits` is optional because the profile implies it; limits that
/// are given must be the profile's own unless `budget.overrides` says the
/// user changed them. Nothing grades usage from these values.
fn budget_findings(handoff: &Value, errors: &mut Vec<HandoffFinding>) {
    let budget = &handoff["budget"];
    let limits = &budget["limits"];
    if !limits.is_object() || budget["overrides"] == true {
        return;
    }
    let Some(profile) = budget["profile"].as_str().and_then(budget_profile) else {
        return;
    };
    for (name, expected) in profile.members() {
        if limits[name].as_u64() != Some(expected) {
            errors.push(HandoffFinding::at(
                child_pointer("/budget/limits", name),
                HandoffRule::BudgetLimitDiffers,
                Some(vec![expected.to_string()]),
                HandoffRule::BudgetLimitDiffers.message(),
            ));
        }
    }
}
