//! Supplied-transcript import preparation and bounded transcript retrieval.

use std::{ffi::OsStr, path::PathBuf};

use vsift_application::{
    SuppliedTranscriptError, TranscriptImportRequest, TranscriptPageRequest, page_transcript,
};
use vsift_domain::{
    MediaTime, PageLimit, RuntimeDependency, SessionId, TimeRange, TranscriptImportError,
    TranscriptOffset, TranscriptRevision, TranscriptRevisionId, TranscriptSegment,
};
use vsift_infrastructure::{
    ExecutableResolutionError, ExecutableResolver, MediaProviderConformance, TrustedExecutable,
    read_supplied_transcript,
};

use crate::{
    engine::{Engine, absolute_selection},
    error::{EngineError, ExecutableRejection, SessionRootError, TranscriptSourceError},
    sessions::{SessionSnapshot, SuppliedTranscriptRequest},
};

const MICROS_PER_SECOND: u64 = 1_000_000;

/// A bounded request for the transcript segments of one session in a range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptQuery {
    /// Session whose transcript is read.
    pub session: SessionId,
    /// Revision to read; `None` reads the newest. Every revision a session
    /// ever committed stays readable, so an older citation always resolves.
    pub revision: Option<TranscriptRevisionId>,
    /// Inclusive source-timeline start, in microseconds.
    pub from_micros: u64,
    /// Exclusive source-timeline end, in microseconds.
    pub to_micros: u64,
    /// Page size from 1 to 100; `None` selects the default of 20.
    pub limit: Option<u16>,
    /// Opaque continuation token from the previous page of the same query.
    pub cursor: Option<String>,
}

/// One bounded page of a session's transcript.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptExcerpt {
    session: SessionSnapshot,
    revision: TranscriptRevision,
    range: TimeRange,
    segments: Vec<TranscriptSegment>,
    next_cursor: Option<String>,
}

impl TranscriptExcerpt {
    /// Session state observed when the page was read.
    #[must_use]
    pub const fn session(&self) -> &SessionSnapshot {
        &self.session
    }

    /// The revision the page was read from, including its alignment metadata.
    #[must_use]
    pub const fn revision(&self) -> &TranscriptRevision {
        &self.revision
    }

    /// The requested half-open source range.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        self.range
    }

    /// Segments intersecting the range, in start order.
    #[must_use]
    pub fn segments(&self) -> &[TranscriptSegment] {
        &self.segments
    }

    /// Opaque token for the next page, absent on the last page.
    #[must_use]
    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }
}

impl Engine {
    /// Reads one bounded page of a session's newest transcript revision, or of
    /// the revision `query.revision` names.
    ///
    /// Segments intersecting `[from, to)` are returned in start order. A
    /// continuation cursor is bound to the session, the revision, the range and
    /// the session's current expiry. This never runs a provider.
    ///
    /// # Errors
    ///
    /// Fails for an invalid range, page size or cursor, a missing root or
    /// session, a closed or expired session, a session without a transcript, a
    /// revision the session does not hold, or a stored record that fails its
    /// integrity checks.
    pub fn transcript(&self, query: TranscriptQuery) -> Result<TranscriptExcerpt, EngineError> {
        let range = TimeRange::new(
            MediaTime::from_micros(query.from_micros),
            MediaTime::from_micros(query.to_micros),
        )
        .map_err(|_| EngineError::InvalidTimeRange)?;
        let limit = query
            .limit
            .map_or(Ok(PageLimit::DEFAULT), PageLimit::new)
            .map_err(|_| EngineError::InvalidPageLimit)?;
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        let (revision, status) = match &query.revision {
            None => store
                .read_transcript(&query.session, now)?
                .ok_or(EngineError::TranscriptUnavailable)?,
            Some(revision) => store
                .read_transcript_revision(&query.session, revision, now)?
                .ok_or(EngineError::TranscriptRevisionNotFound)?,
        };
        let expires_at = status
            .lifetime()
            .expires_at_unix_seconds()
            .saturating_mul(MICROS_PER_SECOND);
        let page = page_transcript(
            &query.session,
            &revision,
            &TranscriptPageRequest {
                range,
                limit,
                cursor: query.cursor,
            },
            expires_at,
            now.saturating_mul(MICROS_PER_SECOND),
        )
        .map_err(EngineError::TranscriptQuery)?;
        let segments = page.segments.into_iter().cloned().collect();
        let next_cursor = page.next_cursor.map(|cursor| cursor.encode());
        Ok(TranscriptExcerpt {
            session: SessionSnapshot::observe(&status, now),
            revision,
            range,
            segments,
            next_cursor,
        })
    }

    /// Validates and reads everything a transcript import needs before the
    /// session root is touched: the offset, the sidecar and the media tools.
    pub(crate) fn prepare_transcript_import(
        &self,
        request: &SuppliedTranscriptRequest,
    ) -> Result<(TranscriptImportRequest, MediaProviderConformance), EngineError> {
        let offset = TranscriptOffset::from_micros(request.offset_micros).map_err(|rejection| {
            EngineError::TranscriptRejected(TranscriptImportError::new(rejection))
        })?;
        let path = absolute_selection(&request.path)?;
        let supplied = read_supplied_transcript(&path).map_err(|error| match error {
            SuppliedTranscriptError::Rejected(rejection) => {
                EngineError::TranscriptRejected(rejection)
            }
            SuppliedTranscriptError::InvalidPath => {
                EngineError::TranscriptSource(TranscriptSourceError::InvalidPath)
            }
            SuppliedTranscriptError::NotRegularFile => {
                EngineError::TranscriptSource(TranscriptSourceError::NotRegularFile)
            }
            SuppliedTranscriptError::Io => EngineError::TranscriptSource(TranscriptSourceError::Io),
        })?;
        let tools = self.media_tools()?;
        Ok((TranscriptImportRequest { supplied, offset }, tools))
    }

    /// Resolves `FFmpeg` and `FFprobe` with the same precedence as `setup
    /// check`: a configured user selection first, then the filtered `PATH`.
    pub(crate) fn media_tools(&self) -> Result<MediaProviderConformance, EngineError> {
        let configured = self.user_configuration()?.read()?;
        let resolver = ExecutableResolver::from_current_path();
        let ffmpeg = resolve_tool(&resolver, configured.ffmpeg, RuntimeDependency::Ffmpeg)?;
        let ffprobe = resolve_tool(&resolver, configured.ffprobe, RuntimeDependency::Ffprobe)?;
        Ok(MediaProviderConformance::r0(ffmpeg, ffprobe))
    }
}

pub(crate) fn resolve_tool(
    path_lookup: &ExecutableResolver,
    configured: Option<PathBuf>,
    dependency: RuntimeDependency,
) -> Result<TrustedExecutable, EngineError> {
    let selection = match configured {
        Some(path) => TrustedExecutable::explicit(path),
        None => path_lookup.resolve(OsStr::new(dependency.identifier())),
    };
    selection.map_err(|error| match error {
        ExecutableResolutionError::NotFound => EngineError::MediaToolUnavailable(dependency),
        other => EngineError::Executable(ExecutableRejection::from(&other)),
    })
}
