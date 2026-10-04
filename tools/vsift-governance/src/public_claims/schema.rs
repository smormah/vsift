//! The public-claims registry, schema version 1.
//!
//! The registry is `docs/planning/public-claims.json`. It holds what the
//! public documents may say about support, stability and qualification (ADR
//! 0024 decision G): each statement that uses a controlled word, the rung of
//! the ladder at which it may appear, and the evidence items that must be
//! `passed` first; plus the phrases that are never claimed.
//!
//! Every struct rejects unknown fields and every closed set is an enum.

use serde::Deserialize;

/// The only schema version this checker reads.
pub(crate) const SCHEMA_VERSION: &str = "1";

/// The whole registry file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaimsRegistry {
    /// Always [`SCHEMA_VERSION`].
    pub(crate) schema_version: String,
    /// How far up the ladder the public documents may go today. A pull
    /// request that cuts the release candidate sets `candidate`; the one that
    /// completes P14 sets `after_p14`.
    pub(crate) current_rung: Rung,
    /// The documents the check reads, by repository-relative path.
    pub(crate) documents: Vec<String>,
    /// Documents that carry public statements but are not read yet, each with
    /// the pull request that brings it under the check.
    pub(crate) unscanned_documents: Vec<UnscannedDocument>,
    /// Words that may appear in a scanned document only inside a registered
    /// statement: `supported`, `stable`, `qualified` and their kin.
    pub(crate) controlled_words: Vec<String>,
    /// Phrases that are never claimed, or not before their evidence exists.
    pub(crate) banned_phrases: Vec<BannedPhrases>,
    /// The registered statements.
    pub(crate) statements: Vec<Statement>,
}

/// A rung of the claims ladder (ADR 0024 decision G).
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Rung {
    /// The 0.1.0 pre-release: the existing wording, no support, no
    /// announcement.
    Now,
    /// A published `0.2.0-rc.N`: "release candidate under qualification" and
    /// the evidence summary as it stands.
    Candidate,
    /// The stable release is published and the evidence ledger is complete.
    AfterP14,
}

impl Rung {
    /// The word the file and the messages use.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Now => "now",
            Self::Candidate => "candidate",
            Self::AfterP14 => "after_p14",
        }
    }
}

/// A document the check does not read yet.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UnscannedDocument {
    /// Its repository-relative path.
    pub(crate) path: String,
    /// The P14 pull request that brings it under the check.
    pub(crate) owner: String,
    /// Why it is not read today.
    pub(crate) reason: String,
}

/// A group of phrases that are not claimed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BannedPhrases {
    /// `BAN-nn`.
    pub(crate) id: String,
    /// The phrases, written as they would read in prose. Matching ignores
    /// case, Markdown markup, line breaks and the difference between a
    /// hyphen and a space.
    pub(crate) phrases: Vec<String>,
    /// Why the claim is not made.
    pub(crate) reason: String,
    /// Evidence items that, once all `passed`, lift the ban. Empty means the
    /// claim is never made in P14.
    pub(crate) lifted_by: Vec<String>,
}

/// A registered statement: a fragment of a public document that is allowed
/// to use a controlled word or a banned phrase.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Statement {
    /// A statement that asserts something about `VSift`. It needs evidence
    /// and a rung.
    Claim {
        /// `CL-nnn`.
        id: String,
        /// The first rung at which the statement may appear.
        rung: Rung,
        /// The documents it may appear in.
        documents: Vec<String>,
        /// The words. Matching ignores case, Markdown markup, line breaks and
        /// the difference between a hyphen and a space.
        text: String,
        /// Evidence items that must be `passed` while the statement is in use.
        #[serde(default)]
        requires: Vec<String>,
        /// Repository records the statement rests on when no item of the
        /// evidence ledger does yet (the existing pre-release wording).
        #[serde(default)]
        basis: Vec<String>,
        /// The known-limits register entries (`L-004`) the wording leans on.
        /// Each must exist. A statement above the `now` rung may be in use
        /// only once the maintainer has reviewed every one of them (accepted
        /// or rescheduled; see `register-review-sheet.md`).
        #[serde(default)]
        limits: Vec<String>,
        /// What the wording rests on and where it stops, for the reviewer.
        note: String,
    },
    /// A use of a controlled word that asserts nothing: a negation ("not
    /// supported"), a condition, a definition or the name of a thing. It
    /// needs no evidence and is allowed at every rung.
    NonClaim {
        /// `NC-nnn`.
        id: String,
        /// The documents it may appear in.
        documents: Vec<String>,
        /// The words.
        text: String,
        /// Why the use asserts nothing, for the reviewer.
        note: String,
    },
}

impl Statement {
    /// The statement's identifier.
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Claim { id, .. } | Self::NonClaim { id, .. } => id,
        }
    }

    /// The words of the statement.
    pub(crate) fn text(&self) -> &str {
        match self {
            Self::Claim { text, .. } | Self::NonClaim { text, .. } => text,
        }
    }

    /// The documents the statement may appear in.
    pub(crate) fn documents(&self) -> &[String] {
        match self {
            Self::Claim { documents, .. } | Self::NonClaim { documents, .. } => documents,
        }
    }

    /// The reviewer's note.
    pub(crate) fn note(&self) -> &str {
        match self {
            Self::Claim { note, .. } | Self::NonClaim { note, .. } => note,
        }
    }
}
