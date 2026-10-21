//! Supplied-transcript import preparation and bounded transcript retrieval.

use std::{ffi::OsStr, path::PathBuf};

use vsift_application::{
    SuppliedTranscriptError, TranscriptImportRequest, TranscriptPage, TranscriptPageRequest,
    page_transcript,
};
use vsift_domain::{
    MediaTime, PageLimit, RuntimeDependency, SearchCoverage, SessionId, TimeRange,
    TranscriptImportError, TranscriptOffset, TranscriptRevision, TranscriptRevisionId,
    TranscriptSegment,
};
use vsift_infrastructure::{
    ContainedFile, ExecutableResolutionError, ExecutableResolver, MediaProviderConformance,
    TrustedExecutable, read_supplied_transcript, read_supplied_transcript_contained,
};

use crate::{
    engine::{Engine, absolute_selection},
    error::{EngineError, ExecutableRejection, SessionRootError, TranscriptSourceError},
    managed::ManagedLookup,
    sessions::{SessionSnapshot, SuppliedTranscriptRequest},
};

pub(crate) const MICROS_PER_SECOND: u64 = 1_000_000;

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
    /// A `since` token of an earlier answer (ADR 0033): read only what the
    /// revision it follows published after it, in start order. The revision is
    /// the token's, and the range is clipped to the source.
    pub since: Option<String>,
}

/// One bounded page of a session's transcript.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptExcerpt {
    session: SessionSnapshot,
    revision: TranscriptRevision,
    range: TimeRange,
    segments: Vec<TranscriptSegment>,
    next_cursor: Option<String>,
    coverage: SearchCoverage,
    growth: crate::growth::ReadGrowth,
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

    /// What the revision's transcript covers of the requested range (#373),
    /// the coverage of the whole range and not of this page, by the rule
    /// `search` uses. A host that reads the page tells a part nobody
    /// transcribed from a silence with this: an empty page there is a gap.
    #[must_use]
    pub const fn coverage(&self) -> &SearchCoverage {
        &self.coverage
    }

    /// What the read of a growing transcript adds (ADR 0033).
    #[must_use]
    pub const fn growth(&self) -> &crate::growth::ReadGrowth {
        &self.growth
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
        if let Some(since) = &query.since {
            return transcript_since(&store, &query, range, limit, since, now);
        }
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
        let TranscriptPage {
            segments,
            next_cursor,
            coverage,
        } = page;
        let segments = segments.into_iter().cloned().collect();
        let next_cursor = next_cursor.map(|cursor| cursor.encode());
        let growth = crate::growth::read_growth(
            &store,
            &query.session,
            &revision,
            &coverage,
            None,
            expires_at,
            now,
        )?;
        Ok(TranscriptExcerpt {
            session: SessionSnapshot::observe(&status, now),
            revision,
            range,
            segments,
            next_cursor,
            coverage,
            growth,
        })
    }
}

/// A `transcript get --since` (ADR 0033): what the token's revision
/// published after it, in `range` clipped to the source, at most `limit`.
fn transcript_since(
    store: &vsift_infrastructure::FilesystemSessionStore,
    query: &TranscriptQuery,
    range: TimeRange,
    limit: PageLimit,
    since: &str,
    now: u64,
) -> Result<TranscriptExcerpt, EngineError> {
    let (revision, status, watermark) =
        crate::growth::since_target(store, &query.session, since, now)?;
    if query
        .revision
        .as_ref()
        .is_some_and(|wanted| wanted != revision.id())
    {
        return Err(EngineError::TranscriptQuery(
            vsift_application::TranscriptQueryError::Cursor(vsift_domain::CursorError::WrongQuery),
        ));
    }
    let source = revision.source_segment().range();
    let range = TimeRange::new(
        range.start().max(source.start()),
        range.end().min(source.end()),
    )
    .map_err(|_| EngineError::InvalidTimeRange)?;
    let expires_at = status
        .lifetime()
        .expires_at_unix_seconds()
        .saturating_mul(MICROS_PER_SECOND);
    let page = vsift_application::page_since(
        &query.session,
        &revision,
        watermark,
        Some(range),
        None,
        limit,
        expires_at,
    )
    .map_err(crate::growth::cursor_error)?;
    let coverage = SearchCoverage::of(&revision, Some(range));
    let segments = page
        .segments
        .iter()
        .map(|(segment, _)| (*segment).clone())
        .collect();
    let growth = crate::growth::read_growth(
        store,
        &query.session,
        &revision,
        &coverage,
        Some((page.next.encode(), page.more)),
        expires_at,
        now,
    )?;
    Ok(TranscriptExcerpt {
        session: SessionSnapshot::observe(&status, now),
        revision,
        range,
        segments,
        next_cursor: None,
        coverage,
        growth,
    })
}

impl Engine {
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
        let supplied = read_supplied_transcript(&path).map_err(transcript_source_error)?;
        let tools = self.media_tools()?;
        Ok((TranscriptImportRequest { supplied, offset }, tools))
    }

    /// [`Self::prepare_transcript_import`] for a sidecar a worker request
    /// named inside the operator's input root, already opened there (P11).
    pub(crate) fn prepare_contained_transcript_import(
        &self,
        file: ContainedFile,
        offset_micros: i64,
    ) -> Result<(TranscriptImportRequest, MediaProviderConformance), EngineError> {
        let offset = TranscriptOffset::from_micros(offset_micros).map_err(|rejection| {
            EngineError::TranscriptRejected(TranscriptImportError::new(rejection))
        })?;
        let supplied = read_supplied_transcript_contained(file).map_err(transcript_source_error)?;
        let tools = self.media_tools()?;
        Ok((TranscriptImportRequest { supplied, offset }, tools))
    }

    /// Resolves `FFmpeg` and `FFprobe` with the same precedence as `setup
    /// check`: a configured user selection first, then the managed version
    /// `setup install` selected, then the filtered `PATH`. A managed tool
    /// carries its version's hold, so the version stays in use for as long
    /// as the returned tools live.
    pub(crate) fn media_tools(&self) -> Result<MediaProviderConformance, EngineError> {
        let configured = self.user_configuration()?.read()?;
        let resolver = ExecutableResolver::from_current_path();
        let mut managed = self.managed_lookup();
        let ffmpeg = resolve_media_tool(
            &resolver,
            &mut managed,
            configured.ffmpeg,
            RuntimeDependency::Ffmpeg,
        )?;
        let ffprobe = resolve_media_tool(
            &resolver,
            &mut managed,
            configured.ffprobe,
            RuntimeDependency::Ffprobe,
        )?;
        Ok(MediaProviderConformance::r0(ffmpeg, ffprobe))
    }
}

/// One media tool: `configured` when given, else the managed version, else
/// the filtered `PATH`.
pub(crate) fn resolve_media_tool(
    path_lookup: &ExecutableResolver,
    managed: &mut ManagedLookup<'_>,
    configured: Option<PathBuf>,
    dependency: RuntimeDependency,
) -> Result<TrustedExecutable, EngineError> {
    if configured.is_none()
        && let Some(executable) = managed.executable(dependency)
    {
        return Ok(executable);
    }
    resolve_tool(path_lookup, configured, dependency)
}

impl Engine {}

/// How a supplied transcript that could not be read is reported.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Result::map_err requires ownership of the source error"
)]
fn transcript_source_error(error: SuppliedTranscriptError) -> EngineError {
    match error {
        SuppliedTranscriptError::Rejected(rejection) => EngineError::TranscriptRejected(rejection),
        SuppliedTranscriptError::InvalidPath => {
            EngineError::TranscriptSource(TranscriptSourceError::InvalidPath)
        }
        SuppliedTranscriptError::NotRegularFile => {
            EngineError::TranscriptSource(TranscriptSourceError::NotRegularFile)
        }
        SuppliedTranscriptError::Io => EngineError::TranscriptSource(TranscriptSourceError::Io),
        SuppliedTranscriptError::SourcePath(fault) => {
            EngineError::TranscriptSource(TranscriptSourceError::SourcePath(fault))
        }
        SuppliedTranscriptError::SymbolicLink => {
            EngineError::TranscriptSource(TranscriptSourceError::SymbolicLink)
        }
        SuppliedTranscriptError::SpecialFile(kind) => {
            EngineError::TranscriptSource(TranscriptSourceError::SpecialFile(kind))
        }
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
