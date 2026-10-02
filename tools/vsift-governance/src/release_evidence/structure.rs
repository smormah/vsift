//! The structure rules of the release evidence ledger: what must hold of the
//! file at every commit, whatever the release stage (ADR 0024, decision G and
//! "What P14 delivers" item 1). Completeness, which depends on a candidate
//! version, is in [`super::completeness`].
//!
//! Each rule is its own function so a failing ledger names the rule it broke
//! and each rule has a test.

use std::collections::BTreeSet;

use super::{
    COMPLETENESS_ITEM,
    facts::Facts,
    schema::{
        CandidateRule, Decision, EvidenceEntry, EvidenceItem, EvidenceLedger, EvidenceReference,
        ProducerKind, ReleaseDelta, SCHEMA_VERSION, StableRule, Status,
    },
};
use crate::repository::{Repository, is_repository_relative};

/// The packet whose evidence the ledger holds.
const PACKET: &str = "P14";

/// The highest pull-request number P14's plan has.
const LAST_P14_PR: u8 = 13;

/// A verification row that every release must show evidence for even though
/// no requirement owns it: the scan reading (`R-SEC03`, P14's own `tests`
/// entry in the delivery ledger).
const REQUIRED_ROWS: [&str; 1] = ["R-SEC03"];

/// Runs every structure rule and returns one message per finding.
pub(crate) fn check_structure(
    ledger: &EvidenceLedger,
    facts: &Facts,
    repository: &dyn Repository,
) -> Vec<String> {
    let mut messages = Vec::new();
    check_header(&mut messages, ledger);
    check_item_set(&mut messages, ledger, facts);
    for item in &ledger.items {
        check_text(&mut messages, item);
        check_supports(&mut messages, item, facts);
        check_scope(&mut messages, item, repository);
        check_gate(&mut messages, item);
        check_status(&mut messages, item);
        check_dates(&mut messages, item);
        check_entries(&mut messages, item, repository);
    }
    check_coverage(&mut messages, ledger, facts);
    if let Some(delta) = &ledger.release_delta {
        check_release_delta(&mut messages, delta, repository);
    }
    messages
}

/// The schema version and the owning packet.
fn check_header(messages: &mut Vec<String>, ledger: &EvidenceLedger) {
    if ledger.schema_version != SCHEMA_VERSION {
        messages.push(format!(
            "schema_version must be {SCHEMA_VERSION:?}, found {:?}",
            ledger.schema_version
        ));
    }
    if ledger.packet != PACKET {
        messages.push(format!(
            "packet must be {PACKET:?}, found {:?}",
            ledger.packet
        ));
    }
}

/// The ledger holds exactly the items the plan defines, once each, in the
/// plan's order.
fn check_item_set(messages: &mut Vec<String>, ledger: &EvidenceLedger, facts: &Facts) {
    let mut seen = BTreeSet::new();
    for item in &ledger.items {
        if !seen.insert(item.id.as_str()) {
            messages.push(format!("{} appears more than once", item.id));
        }
    }
    for defined in &facts.plan_items {
        if !seen.contains(defined.as_str()) {
            messages.push(format!(
                "{defined} is defined by the plan but missing from the ledger"
            ));
        }
    }
    for item in &ledger.items {
        if !facts.plan_items.contains(&item.id) {
            messages.push(format!("{} is not defined by the plan", item.id));
        }
    }
    let in_plan_order: Vec<&str> = facts
        .plan_items
        .iter()
        .map(String::as_str)
        .filter(|id| seen.contains(id))
        .collect();
    let in_ledger_order: Vec<&str> = ledger
        .items
        .iter()
        .map(|item| item.id.as_str())
        .filter(|id| facts.plan_items.iter().any(|defined| defined == id))
        .collect();
    if in_plan_order.len() == in_ledger_order.len() && in_plan_order != in_ledger_order {
        messages.push(String::from("items are not in the plan's order"));
    }
}

/// Every descriptive field says something.
fn check_text(messages: &mut Vec<String>, item: &EvidenceItem) {
    for (field, text) in [
        ("title", &item.title),
        ("proves", &item.proves),
        ("does_not_prove", &item.does_not_prove),
        ("producer.description", &item.producer.description),
    ] {
        if text.trim().is_empty() {
            messages.push(format!("{}: {field} must not be empty", item.id));
        }
    }
    if item.producer.p14_pr > LAST_P14_PR {
        messages.push(format!(
            "{}: producer.p14_pr {} is beyond P14's last pull request ({LAST_P14_PR})",
            item.id, item.producer.p14_pr
        ));
    }
}

/// Every id an item supports exists in the document that owns it, and an item
/// supports something (the completeness check's own entry excepted: it checks
/// what the others support).
fn check_supports(messages: &mut Vec<String>, item: &EvidenceItem, facts: &Facts) {
    let supports = &item.supports;
    for (kind, ids, known) in [
        ("requirement", &supports.requirements, &facts.requirements),
        ("threat", &supports.threats, &facts.threats),
        (
            "verification row",
            &supports.verification,
            &facts.verification_rows,
        ),
        ("limit", &supports.limits, &facts.limits),
    ] {
        let mut seen = BTreeSet::new();
        for id in ids {
            if !known.contains(id) {
                messages.push(format!("{}: unknown {kind} {id}", item.id));
            }
            if !seen.insert(id.as_str()) {
                messages.push(format!("{}: {kind} {id} is listed twice", item.id));
            }
        }
    }
    let supports_nothing = supports.requirements.is_empty()
        && supports.threats.is_empty()
        && supports.verification.is_empty()
        && supports.limits.is_empty();
    if supports_nothing && item.id != COMPLETENESS_ITEM {
        messages.push(format!(
            "{}: supports no requirement, threat, row or limit",
            item.id
        ));
    }
    if !facts.verification_rows.contains(&item.id) {
        messages.push(format!(
            "{}: verification.md has no row for this item",
            item.id
        ));
    }
}

/// The staleness scope names real places inside the repository.
fn check_scope(messages: &mut Vec<String>, item: &EvidenceItem, repository: &dyn Repository) {
    if item.scope.is_empty() {
        messages.push(format!("{}: scope must name at least one path", item.id));
    }
    let mut seen = BTreeSet::new();
    for path in &item.scope {
        if !is_repository_relative(path) {
            messages.push(format!(
                "{}: scope path {path:?} is not repository-relative",
                item.id
            ));
        } else if !repository.exists(path) {
            messages.push(format!("{}: scope path {path} does not exist", item.id));
        }
        if !seen.insert(path.as_str()) {
            messages.push(format!("{}: scope path {path} is listed twice", item.id));
        }
    }
}

/// A stable release cannot carry evidence the candidate did not need.
fn check_gate(messages: &mut Vec<String>, item: &EvidenceItem) {
    if item.gate.candidate == CandidateRule::NotRequired
        && item.gate.stable != StableRule::NotRequired
    {
        messages.push(format!(
            "{}: an item the candidate does not need cannot be needed by the stable release \
             (a stable release cannot carry evidence the candidate never required)",
            item.id
        ));
    }
}

/// What each status must and must not come with.
fn check_status(messages: &mut Vec<String>, item: &EvidenceItem) {
    let id = &item.id;
    let counted = matches!(
        item.status,
        Status::Running | Status::Passed | Status::Failed
    );
    if counted != item.applies_to.is_some() {
        messages.push(format!(
            "{id}: applies_to must be given exactly when the status is running, passed or failed"
        ));
    }
    if counted && item.evidence.is_empty() {
        messages.push(format!(
            "{id}: a {} item needs at least one evidence link",
            item.status.label()
        ));
    }
    if matches!(item.status, Status::Planned | Status::NotApplicable) && !item.evidence.is_empty() {
        messages.push(format!(
            "{id}: a {} item has no counted evidence; earlier material belongs in prior",
            item.status.label()
        ));
    }
    if item.status == Status::Passed
        && !item
            .evidence
            .iter()
            .any(|entry| !matches!(entry.reference, EvidenceReference::Issue { .. }))
    {
        messages.push(format!(
            "{id}: an issue alone is not evidence of a pass; link a run, a pull request or a record"
        ));
    }
    if item.status == Status::Passed
        && item.producer.kind == ProducerKind::Workflow
        && !item
            .evidence
            .iter()
            .any(|entry| matches!(entry.reference, EvidenceReference::WorkflowRun { .. }))
    {
        messages.push(format!(
            "{id}: a passed item produced by a workflow links at least one run of it"
        ));
    }
    if item.status == Status::Failed && item.issues.is_empty() {
        messages.push(format!(
            "{id}: a failed item must name the tracked issue (governance rule 14)"
        ));
    }
    if item.issues.contains(&0) {
        messages.push(format!("{id}: issue number 0 does not exist"));
    }
    if (item.status == Status::Waived) != item.decision.is_some() {
        messages.push(format!(
            "{id}: a decision is given exactly when the status is waived"
        ));
    }
    match (&item.status, &item.reason) {
        (Status::NotApplicable, None) => {
            messages.push(format!("{id}: a not_applicable item needs a reason"));
        }
        (Status::NotApplicable | Status::Waived, Some(reason)) if reason.trim().is_empty() => {
            messages.push(format!("{id}: reason must not be empty"));
        }
        (Status::Planned | Status::Running | Status::Passed | Status::Failed, Some(_)) => {
            messages.push(format!(
                "{id}: a reason belongs only to a waived or not_applicable item"
            ));
        }
        _ => {}
    }
}

/// The dates agree: counted evidence is not newer than the status that rests
/// on it, and a decision is not newer than the item that records it.
fn check_dates(messages: &mut Vec<String>, item: &EvidenceItem) {
    for entry in &item.evidence {
        if entry.date > item.date {
            messages.push(format!(
                "{}: evidence dated {} is newer than the item's date {}",
                item.id, entry.date, item.date
            ));
        }
    }
    if let Some(decision) = &item.decision
        && decision.date > item.date
    {
        messages.push(format!(
            "{}: the decision dated {} is newer than the item's date {}",
            item.id, decision.date, item.date
        ));
    }
}

/// Every link is well formed, dated and explained, and a record it names
/// exists.
fn check_entries(messages: &mut Vec<String>, item: &EvidenceItem, repository: &dyn Repository) {
    let mut check = |entry: &EvidenceEntry, list: &str| {
        check_reference(messages, &item.id, list, &entry.reference, repository);
        if entry.note.trim().is_empty() {
            messages.push(format!("{}: a {list} link has no note", item.id));
        }
    };
    for entry in &item.evidence {
        check(entry, "evidence");
    }
    for entry in &item.prior {
        check(entry, "prior");
    }
    if let Some(Decision {
        statement,
        reference,
        ..
    }) = &item.decision
    {
        if statement.trim().is_empty() {
            messages.push(format!("{}: the decision has no statement", item.id));
        }
        check_reference(messages, &item.id, "decision", reference, repository);
    }
}

/// One link: identifiers are positive and a record path is inside the
/// repository and exists. Nothing is fetched.
fn check_reference(
    messages: &mut Vec<String>,
    owner: &str,
    list: &str,
    reference: &EvidenceReference,
    repository: &dyn Repository,
) {
    match reference {
        EvidenceReference::WorkflowRun { workflow, run_id } => {
            if workflow.trim().is_empty() || *run_id == 0 {
                messages.push(format!(
                    "{owner}: a {list} workflow run needs a workflow name and a run id"
                ));
            }
        }
        EvidenceReference::PullRequest { number } | EvidenceReference::Issue { number } => {
            if *number == 0 {
                messages.push(format!("{owner}: a {list} link has number 0"));
            }
        }
        EvidenceReference::Record { path } => {
            if !is_repository_relative(path) {
                messages.push(format!(
                    "{owner}: {list} record {path:?} is not repository-relative"
                ));
            } else if !repository.is_file(path) {
                messages.push(format!("{owner}: {list} record {path} does not exist"));
            }
        }
    }
}

/// Every requirement, threat, required row and P14-owned limit is supported
/// by at least one item, so nothing the plan lists is silently unowned.
fn check_coverage(messages: &mut Vec<String>, ledger: &EvidenceLedger, facts: &Facts) {
    let supported = |select: fn(&EvidenceItem) -> &Vec<String>, id: &str| {
        ledger
            .items
            .iter()
            .any(|item| select(item).iter().any(|listed| listed == id))
    };
    for id in &facts.requirements {
        if !supported(|item| &item.supports.requirements, id) {
            messages.push(format!("requirement {id} is supported by no evidence item"));
        }
    }
    for id in &facts.threats {
        if !supported(|item| &item.supports.threats, id) {
            messages.push(format!("threat {id} is supported by no evidence item"));
        }
    }
    for id in REQUIRED_ROWS {
        if !supported(|item| &item.supports.verification, id) {
            messages.push(format!(
                "verification row {id} is supported by no evidence item"
            ));
        }
    }
    for id in &facts.p14_limits {
        if !supported(|item| &item.supports.limits, id) {
            messages.push(format!(
                "limit {id} names P14 as its owner and is supported by no evidence item"
            ));
        }
    }
}

/// The recorded delta names a candidate and the stable cut from it.
fn check_release_delta(
    messages: &mut Vec<String>,
    delta: &ReleaseDelta,
    repository: &dyn Repository,
) {
    if !delta.candidate_version.is_candidate() {
        messages.push(format!(
            "release_delta.candidate_version {} is not a release candidate",
            delta.candidate_version
        ));
    }
    if delta.stable_version.is_candidate() {
        messages.push(format!(
            "release_delta.stable_version {} is a release candidate, not a stable version",
            delta.stable_version
        ));
    }
    if delta.candidate_version.without_candidate() != delta.stable_version {
        messages.push(format!(
            "release_delta: {} was not cut from {}",
            delta.stable_version, delta.candidate_version
        ));
    }
    check_reference(messages, "release_delta", "check", &delta.check, repository);
}

#[cfg(test)]
mod tests;
