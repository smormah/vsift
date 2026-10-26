//! What a read of a growing transcript adds to its answer (ADR 0033).
//!
//! A transcription that publishes as it goes keeps its revision **open** until
//! its run ends. A `transcript get` or `search` of an open revision is
//! `partial`: what the run has not published yet is listed as untranscribed,
//! and also as pending (`transcript_coverage.pending_ranges`), with the reason
//! `transcription_pending` and a fixed warning. Where an earlier, sealed
//! revision still holds text over the pending part, the answer names it
//! (`pending_covered_by`) so a reader can read it with `--revision`.
//!
//! Every answer over an open revision, and every answer to `--since`, carries a
//! `since` member: a token for what is published after this read, the state of
//! the revision the token follows, and whether more is already there. A sealed
//! revision read without `--since` carries none, so every such answer is
//! exactly what it was before 0.3.0.

use serde::Serialize;
use vsift_domain::{RevisionState, TimeRange, TranscriptRevisionId};

use crate::{search::bounded, transcript::RangeData};

/// The envelope coverage reason of a read of an open revision: part of the
/// range will still be published by the run that makes the revision.
pub const TRANSCRIPTION_PENDING_REASON: &str = "transcription_pending";

/// Fixed warning of a read of an open revision (ADR 0033): says what is
/// pending, that it is not silence, and how to read on.
pub const TRANSCRIPTION_PENDING_WARNING: &str = "The transcription of this video is still running, so part of the requested range is not published yet: it is listed as pending, and as not transcribed, which does not mean nothing was said there. What is shown is final and can be cited. To read what is published next, pass the answer's since token back with --since; where an earlier revision still holds text over the pending part, it is named under pending_covered_by and can be read with --revision.";

/// An earlier, sealed revision that holds text over part of what an open
/// revision has not published yet.
#[derive(Clone, Copy, Debug)]
pub struct PendingCover<'a> {
    /// The earlier revision: the one the open revision supersedes.
    pub revision: &'a TranscriptRevisionId,
    /// The pending ranges that revision covers, in start order.
    pub ranges: &'a [TimeRange],
}

/// The `since` token an answer hands back (ADR 0033).
#[derive(Clone, Copy, Debug)]
pub struct SinceRead<'a> {
    /// The opaque token for what is published after this read.
    pub token: &'a str,
    /// The state of the revision the token follows.
    pub state: RevisionState,
    /// Whether more is published after the token already: this answer was cut
    /// at its limit.
    pub more: bool,
}

/// What a read of a growing transcript adds to its answer; [`Default`] for a
/// read of a sealed revision without `--since`, which adds nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReadGrowth<'a> {
    /// The earlier revision that holds text over the pending part, if any.
    pub pending_covered_by: Option<PendingCover<'a>>,
    /// The token to read on from, for an open revision or a `--since` read.
    pub since: Option<SinceRead<'a>>,
}

/// `transcript_coverage.pending_covered_by`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct PendingCoverData {
    revision_id: String,
    ranges: Vec<RangeData>,
}

impl PendingCoverData {
    pub(crate) fn new(cover: PendingCover<'_>) -> Option<Self> {
        (!cover.ranges.is_empty()).then(|| Self {
            revision_id: cover.revision.as_str().to_owned(),
            ranges: bounded(cover.ranges),
        })
    }
}

/// The `since` member of a read's data.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct SinceData {
    since_token: String,
    revision_state: &'static str,
    more: bool,
}

impl SinceData {
    pub(crate) fn new(since: SinceRead<'_>) -> Self {
        Self {
            since_token: since.token.to_owned(),
            revision_state: since.state.identifier(),
            more: since.more,
        }
    }
}
