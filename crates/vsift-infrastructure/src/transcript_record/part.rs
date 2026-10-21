//! Versioned storage record of one part of an open transcript revision (ADR
//! 0033).
//!
//! A run that publishes as it goes commits its revision in parts. Each part is
//! an immutable, content-addressed session artifact of kind `transcript_part`
//! holding the revision as it stands when the part is written (its identity,
//! its run with every chunk's outcome so far, the time it is published until)
//! and the segments added since the part before, nothing earlier. A reader
//! rebuilds the open revision from all of a session's parts, in order: the last
//! part's revision with every part's segments, validated as an open revision
//! by the domain ([`TranscriptRevision::new_open`]), so a part can neither
//! change nor drop what an earlier part published. The generation that seals
//! the revision retires its parts.
//!
//! Decoding is strict: unknown fields and versions are refused, every part
//! names the same revision, number, source, supersession and run settings, the
//! parts are numbered from 1 with no gap, each part's segments continue the
//! ordinals of the one before, and the time a part is published until never
//! goes back.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};
use vsift_application::SessionStorageError;
use vsift_domain::{JobId, MediaTime, TranscriptProvenance, TranscriptRevision};

use super::StoredAsrTranscript;

/// Largest encoded part a session will store or read: a part holds its
/// revision's run and the segments it adds, never more than a whole record.
pub const MAX_TRANSCRIPT_PART_BYTES: usize = super::MAX_TRANSCRIPT_RECORD_BYTES;

pub use vsift_domain::MAX_TRANSCRIPT_PARTS;

const PART_FORMAT: &str = "vsift.transcript_part";
const PART_VERSION: u16 = 1;

/// One part of an open revision, as a store keeps it.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredPart {
    schema_version: u16,
    format: String,
    /// 1-based position among the revision's parts.
    part: u32,
    /// [`vsift_domain::MERGE_RULES_VERSION`] of the run that wrote it.
    merge_rules: u32,
    /// The job whose run publishes the revision: the one a cancellation of
    /// the session's transcription names, and the only one that may continue
    /// it.
    job_id: String,
    /// Ordinal of the first segment this part adds; one more than the last
    /// segment of the part before (1 for the first part).
    first_ordinal: u32,
    /// The revision as it stands, with only the segments this part adds.
    revision: StoredAsrTranscript,
}

/// An open revision read back from its parts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenRevisionRecord {
    /// The revision as its parts publish it.
    pub revision: TranscriptRevision,
    /// How many parts it has.
    pub parts: NonZeroU32,
    /// The merge-rules version of the run that wrote its last part.
    pub merge_rules: u32,
    /// The job whose run publishes it.
    pub job_id: JobId,
}

/// Encodes part `part` of the open revision `open`: the revision as it stands,
/// with its segments from position `first` on (the ones the parts before did not
/// publish).
///
/// # Errors
///
/// [`SessionStorageError::IntegrityFailure`] for a revision that is not open
/// or a `first` beyond its segments, and
/// [`SessionStorageError::CapacityExhausted`] when the part would exceed
/// [`MAX_TRANSCRIPT_PART_BYTES`] or be past [`MAX_TRANSCRIPT_PARTS`].
pub fn encode_transcript_part(
    open: &TranscriptRevision,
    job: &JobId,
    part: NonZeroU32,
    first: usize,
    merge_rules: u32,
) -> Result<Vec<u8>, SessionStorageError> {
    let (TranscriptProvenance::LocalAsr(run), Some(_)) =
        (open.provenance(), open.published_until())
    else {
        return Err(SessionStorageError::IntegrityFailure);
    };
    if open.state() != vsift_domain::RevisionState::Open || first > open.segments().len() {
        return Err(SessionStorageError::IntegrityFailure);
    }
    if usize::try_from(part.get()).map_or(true, |part| part > MAX_TRANSCRIPT_PARTS) {
        return Err(SessionStorageError::CapacityExhausted);
    }
    let stored = StoredPart {
        schema_version: PART_VERSION,
        format: PART_FORMAT.to_owned(),
        part: part.get(),
        merge_rules,
        job_id: job.as_str().to_owned(),
        first_ordinal: u32::try_from(first + 1)
            .map_err(|_| SessionStorageError::IntegrityFailure)?,
        revision: StoredAsrTranscript::from_revision_from(open, run, first),
    };
    let bytes = serde_json::to_vec(&stored).map_err(|_| SessionStorageError::Io)?;
    if bytes.len() > MAX_TRANSCRIPT_PART_BYTES {
        return Err(SessionStorageError::CapacityExhausted);
    }
    Ok(bytes)
}

/// Rebuilds the open revision from all of its parts, in the order they were
/// committed.
///
/// # Errors
///
/// [`SessionStorageError::UnsupportedVersion`] for a newer part, and
/// [`SessionStorageError::IntegrityFailure`] for anything malformed, oversized,
/// out of order, inconsistent between parts or violating a domain invariant.
pub fn decode_transcript_parts<B: AsRef<[u8]>>(
    parts: &[B],
) -> Result<OpenRevisionRecord, SessionStorageError> {
    #[derive(Deserialize)]
    struct VersionProbe {
        schema_version: u16,
    }
    let damaged = SessionStorageError::IntegrityFailure;
    if parts.is_empty() || parts.len() > MAX_TRANSCRIPT_PARTS {
        return Err(damaged);
    }
    let mut segments = Vec::new();
    let mut last = None;
    let mut until = None::<MediaTime>;
    for (index, bytes) in parts.iter().enumerate() {
        let bytes = bytes.as_ref();
        if bytes.len() > MAX_TRANSCRIPT_PART_BYTES {
            return Err(damaged);
        }
        let probe: VersionProbe = serde_json::from_slice(bytes).map_err(|_| damaged)?;
        if probe.schema_version > PART_VERSION {
            return Err(SessionStorageError::UnsupportedVersion);
        }
        let stored: StoredPart = serde_json::from_slice(bytes).map_err(|_| damaged)?;
        if stored.schema_version != PART_VERSION
            || stored.format != PART_FORMAT
            || usize::try_from(stored.part).ok() != Some(index + 1)
            || usize::try_from(stored.first_ordinal).ok() != Some(segments.len() + 1)
        {
            return Err(damaged);
        }
        let merge_rules = stored.merge_rules;
        let job = JobId::parse(stored.job_id).map_err(|_| damaged)?;
        if let Some((_, _, _, before)) = &last
            && *before != job
        {
            return Err(damaged);
        }
        let (revision, published_until) = stored.revision.into_parts().ok_or(damaged)?;
        let published_until = published_until.ok_or(damaged)?;
        if until.is_some_and(|before| published_until < before) {
            return Err(damaged);
        }
        until = Some(published_until);
        let mut revision = revision;
        segments.append(&mut revision.segments);
        if let Some((before, _, _, _)) = &last
            && !same_revision(before, &revision)
        {
            return Err(damaged);
        }
        last = Some((revision, published_until, merge_rules, job));
    }
    let (mut revision, published_until, merge_rules, job_id) = last.ok_or(damaged)?;
    revision.segments = segments;
    let revision = TranscriptRevision::new_open(revision, published_until).map_err(|_| damaged)?;
    Ok(OpenRevisionRecord {
        revision,
        parts: NonZeroU32::new(u32::try_from(parts.len()).map_err(|_| damaged)?).ok_or(damaged)?,
        merge_rules,
        job_id,
    })
}

/// Whether two parts describe the same revision: the same identity, number,
/// source, supersession and run settings (the chunk outcomes and what was
/// published may only grow).
fn same_revision(
    before: &vsift_domain::TranscriptRevisionParts,
    after: &vsift_domain::TranscriptRevisionParts,
) -> bool {
    let settings = |parts: &vsift_domain::TranscriptRevisionParts| match &parts.provenance {
        TranscriptProvenance::LocalAsr(run) => Some((
            run.provider().clone(),
            run.model().clone(),
            run.decoding(),
            run.plan(),
            run.threads(),
            run.audio_stream(),
            run.covered_range(),
        )),
        TranscriptProvenance::Imported { .. } => None,
    };
    before.id == after.id
        && before.number == after.number
        && before.source_id == after.source_id
        && before.source_segment == after.source_segment
        && before.supersedes == after.supersedes
        && before.replaced_range == after.replaced_range
        && settings(before).is_some()
        && settings(before) == settings(after)
}
