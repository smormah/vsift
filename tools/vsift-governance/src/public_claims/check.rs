//! The rules of the public-claims check.
//!
//! What the check proves: a controlled word (`supported`, `stable`,
//! `qualified` and kin) appears in a scanned document only inside a statement
//! the registry lists; a listed claim in use is allowed at the current rung and
//! every evidence item it needs is `passed`; banned phrases are absent unless
//! their lifting evidence is `passed`; no registry entry is stale or
//! misplaced. What it does not prove: that a sentence is true. A claim with
//! recorded evidence can still be worded wrongly, and a wrongly worded
//! sentence that avoids every controlled word and banned phrase passes. The
//! reviewer still reads the words.

use std::{collections::BTreeSet, ops::Range};

use super::{
    schema::{BannedPhrases, ClaimsRegistry, Rung, SCHEMA_VERSION, Statement},
    text::{contains, find_all, plain_text, snippet, word_count},
};
use crate::{
    release_evidence::{EvidenceLedger, Status},
    repository::{Repository, is_repository_relative},
};

/// A scanned document in comparable form.
struct Document<'a> {
    path: &'a str,
    text: String,
}

/// Runs every rule and returns one message per finding.
pub(crate) fn check_registry(
    registry: &ClaimsRegistry,
    ledger: &EvidenceLedger,
    repository: &dyn Repository,
) -> Vec<String> {
    let mut messages = Vec::new();
    if registry.schema_version != SCHEMA_VERSION {
        messages.push(format!(
            "schema_version must be {SCHEMA_VERSION:?}, found {:?}",
            registry.schema_version
        ));
    }
    check_document_lists(&mut messages, registry, repository);
    check_vocabulary(&mut messages, registry, ledger);
    check_statements(&mut messages, registry, ledger, repository);

    let documents = read_documents(&mut messages, registry, repository);
    let controlled = normalised(&registry.controlled_words);
    let statements: Vec<(usize, String)> = registry
        .statements
        .iter()
        .enumerate()
        .map(|(index, statement)| (index, plain_text(statement.text())))
        .collect();
    let mut used = vec![false; registry.statements.len()];
    for document in &documents {
        check_document(
            &mut messages,
            registry,
            ledger,
            document,
            &controlled,
            &statements,
            &mut used,
        );
    }
    check_use(&mut messages, registry, ledger, &used);
    messages
}

/// The plain form of each text.
fn normalised(texts: &[String]) -> Vec<String> {
    texts.iter().map(|text| plain_text(text)).collect()
}

/// The scanned and unscanned lists: real files, no overlap, no repeats.
fn check_document_lists(
    messages: &mut Vec<String>,
    registry: &ClaimsRegistry,
    repository: &dyn Repository,
) {
    if registry.documents.is_empty() {
        messages.push(String::from("documents must name at least one document"));
    }
    let mut seen = BTreeSet::new();
    for path in &registry.documents {
        if !is_repository_relative(path) || !repository.is_file(path) {
            messages.push(format!("scanned document {path} does not exist"));
        }
        if !seen.insert(path.as_str()) {
            messages.push(format!("scanned document {path} is listed twice"));
        }
    }
    for document in &registry.unscanned_documents {
        if !is_repository_relative(&document.path) || !repository.is_file(&document.path) {
            messages.push(format!(
                "unscanned document {} does not exist",
                document.path
            ));
        }
        if registry.documents.contains(&document.path) {
            messages.push(format!(
                "{} is both scanned and listed as unscanned",
                document.path
            ));
        }
        if document.owner.trim().is_empty() || document.reason.trim().is_empty() {
            messages.push(format!(
                "unscanned document {} needs an owner and a reason",
                document.path
            ));
        }
    }
}

/// The controlled words and the banned phrases.
fn check_vocabulary(
    messages: &mut Vec<String>,
    registry: &ClaimsRegistry,
    ledger: &EvidenceLedger,
) {
    if registry.controlled_words.is_empty() {
        messages.push(String::from("controlled_words must not be empty"));
    }
    for word in &registry.controlled_words {
        let plain = plain_text(word);
        if plain.is_empty() || word_count(&plain) != 1 {
            messages.push(format!("controlled word {word:?} must be a single word"));
        }
    }
    let mut ids = BTreeSet::new();
    for banned in &registry.banned_phrases {
        if !ids.insert(banned.id.as_str()) {
            messages.push(format!("banned phrase group {} appears twice", banned.id));
        }
        if banned.phrases.is_empty() || banned.phrases.iter().any(|p| plain_text(p).is_empty()) {
            messages.push(format!("{} needs non-empty phrases", banned.id));
        }
        if banned.reason.trim().is_empty() {
            messages.push(format!("{} needs a reason", banned.id));
        }
        for item in &banned.lifted_by {
            if status_of(ledger, item).is_none() {
                messages.push(format!(
                    "{} is lifted by {item}, which is not an evidence item",
                    banned.id
                ));
            }
        }
    }
}

/// Each statement is well formed and refers to things that exist.
fn check_statements(
    messages: &mut Vec<String>,
    registry: &ClaimsRegistry,
    ledger: &EvidenceLedger,
    repository: &dyn Repository,
) {
    let mut ids = BTreeSet::new();
    for statement in &registry.statements {
        let id = statement.id();
        if !ids.insert(id) {
            messages.push(format!("statement {id} appears twice"));
        }
        let plain = plain_text(statement.text());
        if word_count(&plain) < 2 {
            messages.push(format!(
                "{id}: a statement is a fragment of at least two words, not a single word"
            ));
        }
        if statement.documents().is_empty() {
            messages.push(format!("{id}: documents must not be empty"));
        }
        if statement.note().trim().is_empty() {
            messages.push(format!(
                "{id}: a statement needs a note saying what its wording rests on or why it \
                 asserts nothing"
            ));
        }
        for path in statement.documents() {
            if !registry.documents.contains(path) {
                messages.push(format!("{id}: {path} is not a scanned document"));
            }
        }
        match statement {
            Statement::Claim {
                rung,
                requires,
                basis,
                ..
            } => {
                for item in requires {
                    if status_of(ledger, item).is_none() {
                        messages.push(format!(
                            "{id}: requires {item}, which is not an evidence item"
                        ));
                    }
                }
                for path in basis {
                    if !is_repository_relative(path) || !repository.is_file(path) {
                        messages.push(format!("{id}: basis {path} does not exist"));
                    }
                }
                if requires.is_empty() && basis.is_empty() {
                    messages.push(format!(
                        "{id}: a claim names the evidence items it requires or the record it rests on"
                    ));
                }
                if *rung > Rung::Now && requires.is_empty() {
                    messages.push(format!(
                        "{id}: a claim above the now rung requires at least one evidence item"
                    ));
                }
            }
            Statement::NonClaim { .. } => {
                let uses_a_word = registry
                    .controlled_words
                    .iter()
                    .any(|word| !find_all(&plain, &plain_text(word)).is_empty())
                    || registry.banned_phrases.iter().any(|banned| {
                        banned
                            .phrases
                            .iter()
                            .any(|phrase| !find_all(&plain, &plain_text(phrase)).is_empty())
                    });
                if !uses_a_word {
                    messages.push(format!(
                        "{id}: a non-claim excuses a controlled word or banned phrase, and this text uses none"
                    ));
                }
            }
        }
    }
}

/// Reads and normalises every scanned document that can be read.
fn read_documents<'a>(
    messages: &mut Vec<String>,
    registry: &'a ClaimsRegistry,
    repository: &dyn Repository,
) -> Vec<Document<'a>> {
    let mut documents = Vec::new();
    for path in &registry.documents {
        match repository.read_text(path) {
            Ok(markdown) => documents.push(Document {
                path,
                text: plain_text(&markdown),
            }),
            Err(error) => messages.push(format!("scanned document could not be read: {error}")),
        }
    }
    documents
}

/// One document: where each statement occurs, which controlled words and
/// banned phrases are covered, and which statements are in use.
fn check_document(
    messages: &mut Vec<String>,
    registry: &ClaimsRegistry,
    ledger: &EvidenceLedger,
    document: &Document<'_>,
    controlled: &[String],
    statements: &[(usize, String)],
    used: &mut [bool],
) {
    // Where each registered statement occurs in this document.
    let mut covers: Vec<(usize, Range<usize>)> = Vec::new();
    for (index, plain) in statements {
        let places = find_all(&document.text, plain);
        if places.is_empty() {
            continue;
        }
        if let Some(statement) = registry.statements.get(*index) {
            if !statement
                .documents()
                .iter()
                .any(|path| path == document.path)
            {
                messages.push(format!(
                    "{}: statement {} is used here but registered only for {}",
                    document.path,
                    statement.id(),
                    statement.documents().join(", ")
                ));
                continue;
            }
            if let Some(flag) = used.get_mut(*index) {
                *flag = true;
            }
            covers.extend(places.into_iter().map(|place| (*index, place)));
        }
    }

    for word in controlled {
        for place in find_all(&document.text, word) {
            if !covers.iter().any(|(_, outer)| contains(outer, &place)) {
                messages.push(format!(
                    "{}: {:?} is used outside a registered statement: {}",
                    document.path,
                    word,
                    snippet(&document.text, &place)
                ));
            }
        }
    }

    for banned in &registry.banned_phrases {
        if is_lifted(banned, ledger) {
            continue;
        }
        for phrase in &banned.phrases {
            for place in find_all(&document.text, &plain_text(phrase)) {
                let excused = covers.iter().any(|(index, outer)| {
                    matches!(
                        registry.statements.get(*index),
                        Some(Statement::NonClaim { .. })
                    ) && contains(outer, &place)
                });
                if !excused {
                    messages.push(format!(
                        "{}: banned phrase {phrase:?} ({}): {}",
                        document.path,
                        banned.id,
                        snippet(&document.text, &place)
                    ));
                }
            }
        }
    }
}

/// Whether every item that lifts the ban has `passed`. A ban nothing lifts is
/// never lifted.
fn is_lifted(banned: &BannedPhrases, ledger: &EvidenceLedger) -> bool {
    !banned.lifted_by.is_empty()
        && banned
            .lifted_by
            .iter()
            .all(|item| status_of(ledger, item) == Some(Status::Passed))
}

/// The statements in use are allowed at the current rung and backed by passed
/// evidence; the ones that should be in use but are not are stale.
fn check_use(
    messages: &mut Vec<String>,
    registry: &ClaimsRegistry,
    ledger: &EvidenceLedger,
    used: &[bool],
) {
    for (index, statement) in registry.statements.iter().enumerate() {
        let in_use = used.get(index).copied().unwrap_or(false);
        match statement {
            Statement::Claim {
                id, rung, requires, ..
            } => {
                if in_use {
                    if *rung > registry.current_rung {
                        messages.push(format!(
                            "{id} is a {} claim in use while the current rung is {}",
                            rung.label(),
                            registry.current_rung.label()
                        ));
                    }
                    for item in requires {
                        match status_of(ledger, item) {
                            Some(Status::Passed) | None => {}
                            Some(status) => messages.push(format!(
                                "{id} is in use but requires {item} to be passed; it is {}",
                                status.label()
                            )),
                        }
                    }
                } else if *rung <= registry.current_rung {
                    messages.push(stale(id));
                }
            }
            Statement::NonClaim { id, .. } => {
                if !in_use {
                    messages.push(stale(id));
                }
            }
        }
    }
}

/// The message for a registered statement that no scanned document uses.
fn stale(id: &str) -> String {
    format!(
        "{id} is registered but appears in none of its documents; remove it or restore the words \
         (a stale entry would let the words come back unreviewed)"
    )
}

/// The status of the evidence item `id`, if the ledger has it.
fn status_of(ledger: &EvidenceLedger, id: &str) -> Option<Status> {
    ledger
        .items
        .iter()
        .find(|item| item.id == id)
        .map(|item| item.status)
}

#[cfg(test)]
mod tests;
