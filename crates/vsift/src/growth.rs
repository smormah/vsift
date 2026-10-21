//! What a read of a growing transcript adds (ADR 0033): the earlier revision
//! that holds text over what an open revision has not published yet, and the
//! token a reader follows the revision with (`--since`).

use vsift_application::{since_revision, since_token, since_watermark};
use vsift_domain::{
    RevisionState, SearchCoverage, SessionId, TimeRange, TranscriptRevision, TranscriptRevisionId,
};
use vsift_infrastructure::FilesystemSessionStore;

use crate::error::EngineError;

/// A since token that cannot be read or formed, as the read's cursor error.
pub(crate) const fn cursor_error(error: vsift_domain::CursorError) -> EngineError {
    EngineError::TranscriptQuery(vsift_application::TranscriptQueryError::Cursor(error))
}

/// What a read of a growing transcript adds to its answer (ADR 0033).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReadGrowth {
    pending_covered_by: Option<(TranscriptRevisionId, Vec<TimeRange>)>,
    since: Option<(String, RevisionState, bool)>,
}

impl ReadGrowth {
    /// The earlier, sealed revision that holds text over part of what the read
    /// open revision has not published yet, and the pending ranges it covers.
    #[must_use]
    pub fn pending_covered_by(&self) -> Option<(&TranscriptRevisionId, &[TimeRange])> {
        self.pending_covered_by
            .as_ref()
            .map(|(revision, ranges)| (revision, ranges.as_slice()))
    }

    /// The token for what the revision publishes after this read, the state of
    /// the revision it follows, and whether more is published after it
    /// already. Present for a read of an open revision and for a `--since`
    /// read.
    #[must_use]
    pub fn since(&self) -> Option<(&str, RevisionState, bool)> {
        self.since
            .as_ref()
            .map(|(token, state, more)| (token.as_str(), *state, *more))
    }
}

/// The revision a `--since` token follows, and the watermark it holds.
pub(crate) fn since_target(
    store: &FilesystemSessionStore,
    session: &SessionId,
    encoded: &str,
    now: u64,
) -> Result<
    (
        TranscriptRevision,
        vsift_infrastructure::SessionStatus,
        usize,
    ),
    EngineError,
> {
    let now_micros = now.saturating_mul(crate::transcripts::MICROS_PER_SECOND);
    let (number, token) = since_revision(session, encoded, now_micros).map_err(cursor_error)?;
    let (revision, status) = store
        .read_transcript_where_number(session, number, now)?
        .ok_or(EngineError::TranscriptRevisionNotFound)?;
    let watermark = since_watermark(&token, &revision).map_err(cursor_error)?;
    Ok((revision, status, watermark))
}

/// The growth of a read of `revision` whose answer ends at `watermark`
/// published segments (`more` when it stopped at its limit), with `coverage`
/// the read's: the token to read on from, for an open revision or a since
/// read, and the earlier revision over what is pending.
pub(crate) fn read_growth(
    store: &FilesystemSessionStore,
    session: &SessionId,
    revision: &TranscriptRevision,
    coverage: &SearchCoverage,
    since: Option<(String, bool)>,
    expires_at_micros: u64,
    now: u64,
) -> Result<ReadGrowth, EngineError> {
    let state = revision.state();
    let since = match since {
        Some((token, more)) => Some((token, state, more)),
        None if state == RevisionState::Open => Some((
            since_token(
                session,
                revision,
                revision.segments().len(),
                expires_at_micros,
            )
            .map_err(cursor_error)?
            .encode(),
            state,
            false,
        )),
        None => None,
    };
    let pending_covered_by = match (revision.supersedes(), coverage.pending()) {
        (Some(base), pending) if !pending.is_empty() => {
            match store.read_transcript_revision(session, base, now)? {
                Some((base_revision, _)) => {
                    let covered = covered_parts(
                        pending,
                        SearchCoverage::of(&base_revision, coverage.searched()).transcribed(),
                    );
                    (!covered.is_empty()).then(|| (base.clone(), covered))
                }
                None => None,
            }
        }
        _ => None,
    };
    Ok(ReadGrowth {
        pending_covered_by,
        since,
    })
}

/// The parts of `pending` that `covered` covers; both are in start order and
/// merged.
fn covered_parts(pending: &[TimeRange], covered: &[TimeRange]) -> Vec<TimeRange> {
    let mut parts = Vec::new();
    for gap in pending {
        for range in covered {
            let start = gap.start().max(range.start());
            let end = gap.end().min(range.end());
            if let Ok(part) = TimeRange::new(start, end) {
                parts.push(part);
            }
        }
    }
    parts
}
