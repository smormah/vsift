//! The public-claims registry and its check (P14 PR 1; ADR 0024 decision G).
//!
//! The ladder: *now* (the 0.1.0 pre-release: the existing wording, no
//! "supported", no announcement), the *candidate* (a published
//! `0.2.0-rc.N`: "release candidate under qualification" and the evidence
//! summary as it stands) and *after P14* (the stable release is published and
//! the evidence ledger is complete: matrix-backed "supported" per cell,
//! qualified models per client, measured numbers with their conditions).
//! `docs/planning/public-claims.json` lists, per rung, the statements the
//! public documents may use, the evidence items that must be `passed` before
//! each may appear, and the phrases that are never claimed.
//!
//! The check runs inside the normal `check` command (so on every pull
//! request) and as `public-claims` on its own. It proves that controlled
//! words appear only inside listed statements, that a listed claim in use is
//! allowed at the current rung and has its evidence `passed`, and that banned
//! phrases are absent. It cannot prove a sentence true. A reviewer still reads
//! the words (known limit L-101).

mod check;
mod register;
mod schema;
mod text;

use std::path::Path;

use check::check_registry;
use schema::ClaimsRegistry;

use crate::{
    release_evidence,
    repository::{DiskRepository, Repository},
};

/// The registry file, relative to the repository root.
pub(crate) const REGISTRY_PATH: &str = "docs/planning/public-claims.json";

/// Appends the claims check's findings.
pub(crate) fn validate_public_claims(messages: &mut Vec<String>, repository: &dyn Repository) {
    messages.extend(findings(repository));
}

fn findings(repository: &dyn Repository) -> Vec<String> {
    let registry = match load(repository) {
        Ok(registry) => registry,
        Err(error) => return vec![error],
    };
    let ledger = match release_evidence::load(repository) {
        Ok(ledger) => ledger,
        Err(error) => {
            return vec![format!(
                "the claims check needs the evidence ledger: {error}"
            )];
        }
    };
    check_registry(&registry, &ledger, repository)
        .into_iter()
        .map(|message| format!("{REGISTRY_PATH}: {message}"))
        .collect()
}

fn load(repository: &dyn Repository) -> Result<ClaimsRegistry, String> {
    let text = repository.read_text(REGISTRY_PATH)?;
    serde_json::from_str(&text)
        .map_err(|error| format!("{REGISTRY_PATH} could not be read: {error}"))
}

/// Runs the `public-claims` command.
pub(crate) fn run_public_claims(root: &Path) -> Result<String, Vec<String>> {
    let findings = findings(&DiskRepository::new(root));
    if findings.is_empty() {
        Ok(String::from(
            "VSift public claims agree with the evidence ledger (this proves recorded evidence \
             and absent banned words, not that a sentence is true).",
        ))
    } else {
        Err(findings)
    }
}
