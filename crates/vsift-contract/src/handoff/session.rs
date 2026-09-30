//! Resolving a handoff's cited identities against one session's records
//! (`handoff check --session`).
//!
//! The host reads the session read-only (it never renews it) and hands this
//! module the identities the session holds; this module decides what the
//! handoff cites and reports what does not resolve. A session that is
//! closed, expired or not found is a [`HandoffSessionGap`], not a failure:
//! the draft is still checked, without resolution.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::Value;

use super::{
    rule::{HandoffFinding, HandoffRule},
    schema::{child_pointer, index_pointer},
};

/// What kind of evidence an evidence identity names.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum HandoffEvidenceKind {
    /// A whole frame (`frame get`, `frame neighbours`, `frame burst`).
    Frame,
    /// A crop.
    Crop,
    /// An audio clip.
    Audio,
}

impl HandoffEvidenceKind {
    /// The kind a citation's or resume item's `type`/`kind` names.
    fn named(name: &str) -> Option<Self> {
        match name {
            "frame" => Some(Self::Frame),
            "crop" => Some(Self::Crop),
            "audio" => Some(Self::Audio),
            _ => None,
        }
    }
}

/// The identities one open session holds, read from its committed records.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HandoffSessionRecords {
    segments: BTreeSet<String>,
    evidence: BTreeMap<String, HandoffEvidenceKind>,
}

impl HandoffSessionRecords {
    /// No identities yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a transcript segment identity of any committed revision.
    pub fn add_segment(&mut self, segment_id: &str) {
        self.segments.insert(segment_id.to_owned());
    }

    /// Adds an evidence item identity with its kind.
    pub fn add_evidence(&mut self, evidence_id: &str, kind: HandoffEvidenceKind) {
        self.evidence.insert(evidence_id.to_owned(), kind);
    }
}

/// Why the cited identities were not resolved in the session named.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffSessionGap {
    /// The session was closed; its records are no longer read.
    SessionClosed,
    /// The session expired.
    SessionExpired,
    /// The session root holds no such session.
    SessionNotFound,
}

/// How the session named with `--session` was used.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HandoffSessionCheck {
    session_id: String,
    resolved: bool,
    gap: Option<HandoffSessionGap>,
    identities_checked: u32,
}

impl HandoffSessionCheck {
    /// The session could not be read; nothing was resolved.
    #[must_use]
    pub fn gap(session_id: &str, gap: HandoffSessionGap) -> Self {
        Self {
            session_id: session_id.to_owned(),
            resolved: false,
            gap: Some(gap),
            identities_checked: 0,
        }
    }

    /// Whether the cited identities were resolved.
    #[must_use]
    pub const fn resolved(&self) -> bool {
        self.resolved
    }

    /// Why they were not, if they were not.
    #[must_use]
    pub const fn session_gap(&self) -> Option<HandoffSessionGap> {
        self.gap
    }
}

/// Resolves every identity `handoff` cites against `records` of
/// `session_id`: each citation's `segment_id` and `evidence_id` (with the
/// citation's type), a crop's `parent_evidence_id`, and the resume card's
/// evidence and findings to verify. A session the handoff names
/// (`session.session_id`, `resume.session_id`) must be `session_id`.
pub(crate) fn resolve(
    handoff: &Value,
    session_id: &str,
    records: &HandoffSessionRecords,
) -> (HandoffSessionCheck, Vec<HandoffFinding>) {
    let mut resolver = Resolver {
        records,
        findings: Vec::new(),
        checked: 0,
    };
    for (pointer, named) in [
        ("/session/session_id", &handoff["session"]["session_id"]),
        ("/resume/session_id", &handoff["resume"]["session_id"]),
    ] {
        if let Some(named) = named.as_str()
            && named != session_id
        {
            resolver.findings.push(HandoffFinding::rule_at(
                pointer.to_owned(),
                HandoffRule::SessionMismatch,
            ));
        }
    }
    for (index, citation) in handoff["citations"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let pointer = index_pointer("/citations", index);
        if let Some(segment) = citation["segment_id"].as_str() {
            resolver.segment(&child_pointer(&pointer, "segment_id"), segment);
        }
        let kind = citation["type"]
            .as_str()
            .and_then(HandoffEvidenceKind::named);
        if let Some(evidence) = citation["evidence_id"].as_str() {
            resolver.evidence(&child_pointer(&pointer, "evidence_id"), evidence, kind);
        }
        if let Some(parent) = citation["parent_evidence_id"].as_str() {
            resolver.evidence(&child_pointer(&pointer, "parent_evidence_id"), parent, None);
        }
    }
    for (index, item) in handoff["resume"]["evidence"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let pointer = child_pointer(&index_pointer("/resume/evidence", index), "id");
        match (item["kind"].as_str(), item["id"].as_str()) {
            (Some("transcript_segment"), Some(id)) => resolver.segment(&pointer, id),
            (Some(kind), Some(id)) => {
                if let Some(kind) = HandoffEvidenceKind::named(kind) {
                    resolver.evidence(&pointer, id, Some(kind));
                }
            }
            _ => {}
        }
    }
    for (index, item) in handoff["resume"]["to_verify"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let pointer = child_pointer(&index_pointer("/resume/to_verify", index), "id");
        match item["id"].as_str() {
            Some(id) if id.starts_with("tsg_") => resolver.segment(&pointer, id),
            Some(id) if id.starts_with("evd_") => resolver.evidence(&pointer, id, None),
            _ => {}
        }
    }
    let check = HandoffSessionCheck {
        session_id: session_id.to_owned(),
        resolved: true,
        gap: None,
        identities_checked: resolver.checked,
    };
    (check, resolver.findings)
}

struct Resolver<'records> {
    records: &'records HandoffSessionRecords,
    findings: Vec<HandoffFinding>,
    checked: u32,
}

impl Resolver<'_> {
    fn segment(&mut self, pointer: &str, id: &str) {
        self.checked = self.checked.saturating_add(1);
        if !self.records.segments.contains(id) {
            self.findings.push(HandoffFinding::rule_at(
                pointer.to_owned(),
                HandoffRule::SegmentNotInSession,
            ));
        }
    }

    fn evidence(&mut self, pointer: &str, id: &str, kind: Option<HandoffEvidenceKind>) {
        self.checked = self.checked.saturating_add(1);
        match self.records.evidence.get(id) {
            None => self.findings.push(HandoffFinding::rule_at(
                pointer.to_owned(),
                HandoffRule::EvidenceNotInSession,
            )),
            Some(held) if kind.is_some_and(|kind| kind != *held) => {
                self.findings.push(HandoffFinding::rule_at(
                    pointer.to_owned(),
                    HandoffRule::EvidenceKindMismatch,
                ));
            }
            Some(_) => {}
        }
    }
}
