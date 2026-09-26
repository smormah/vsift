//! Visual candidates of a session's video: `candidates` (P08, ADR 0018).
//!
//! [`Engine::candidates`] pages the candidates of a source range from the
//! session's visual-candidate index, building the index as it goes. The
//! order of steps is part of the contract, because it decides what a call
//! costs and what it may fail with:
//!
//! 1. The request is validated and the session's newest index is read. When
//!    every window of the range is already recorded, or the call continues a
//!    page with a cursor, the page is returned at once: no tool is resolved,
//!    no provider runs and nothing is written (a warm read).
//! 2. Otherwise `FFmpeg` and `FFprobe` are resolved and proven by the automatic
//!    media-tool preflight, the committed source copy is bound (hashed once,
//!    compared by identity before every provider call, issue #148), probed,
//!    and its first decodable video stream selected.
//! 3. The range's missing windows are analysed in ascending order, at most
//!    30 (30 minutes of media) per call; the rest stay `not_analyzed` and a
//!    later call continues them.
//! 4. The copy is fully verified again, and the windows this call recorded
//!    are committed as one new index revision. If another call committed
//!    first, the newest revision is re-read and this call's windows are
//!    merged onto it (windows are pure, so the union is safe), at most three
//!    times.
//! 5. The page is read from the committed revision, with honest coverage.
//!
//! A stop on a deadline, busy admission or cancellation is a failure only
//! when nothing new was analysed and nothing of the range was indexed
//! before; otherwise the result is `partial` with the unanalysed windows as
//! gaps.

use vsift_application::{
    CandidatePageRequest, ExtendVisualIndexRequest, SessionStorageError, VisualExtensionStop,
    VisualIndexBuildError, VisualIndexScope, VisualSamplingError, analysed_ranges, clip_to_source,
    extend_visual_index_within, indexes_any_of, merge_visual_extension, missing_windows,
    page_candidates, visual_coverage_gaps, whole_file_source_segment,
};
use vsift_domain::{
    MediaSelection, MediaTime, PageLimit, SessionArtifactKind, SessionId, SourceSegmentId,
    TimeRange, VisualCandidate, VisualCoverageGap, VisualIndex, VisualIndexError,
    VisualIndexProfile, VisualStreamError,
};
use vsift_infrastructure::{
    BoundSource, FfmpegMedia, FfmpegVisualSampler, FilesystemSessionStore,
    MediaProviderConformance, SessionStatus, encode_visual_index_record,
};

use crate::{
    asr::{open_status, probe_error, snapshot_storage_error},
    engine::Engine,
    error::{EngineError, SessionRootError},
    sessions::SessionSnapshot,
    verification::Cancellation,
};

const MICROS_PER_SECOND: u64 = 1_000_000;

/// How often a commit that lost a race with another call is merged and
/// retried before the call reports `BUSY`.
const MAX_COMMIT_ATTEMPTS: usize = 3;

/// A half-open source range whose candidates are requested, in microseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandidatesRange {
    /// Inclusive start.
    pub from_micros: u64,
    /// Exclusive end.
    pub to_micros: u64,
}

/// A bounded request for the visual candidates of one source range.
#[derive(Clone, Debug)]
pub struct CandidatesRequest {
    /// Session whose video is read.
    pub session: SessionId,
    /// Requested range; a range that runs past the end of the video is
    /// clipped to it, one that starts at or after the end is rejected.
    pub range: CandidatesRange,
    /// Page size from 1 to 100; `None` selects the default of 20.
    pub limit: Option<u16>,
    /// Opaque continuation token from the previous page of the same range.
    /// A call with a cursor never analyses anything.
    pub cursor: Option<String>,
    /// Signal that stops analysis at its next window; windows analysed
    /// before it are still committed.
    pub cancellation: Cancellation,
}

/// One bounded page of visual candidates and the coverage of its range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidatesResults {
    session: SessionSnapshot,
    index: VisualIndex,
    source_segment_id: SourceSegmentId,
    range: TimeRange,
    searched: TimeRange,
    candidates: Vec<VisualCandidate>,
    next_cursor: Option<String>,
    analysed: Vec<TimeRange>,
    gaps: Vec<VisualCoverageGap>,
    analysed_now: usize,
}

impl CandidatesResults {
    /// Session state observed after the call.
    #[must_use]
    pub const fn session(&self) -> &SessionSnapshot {
        &self.session
    }

    /// The index revision the page was read from.
    #[must_use]
    pub const fn index(&self) -> &VisualIndex {
        &self.index
    }

    /// The source segment the candidates' times lie in (the whole file).
    #[must_use]
    pub const fn source_segment_id(&self) -> &SourceSegmentId {
        &self.source_segment_id
    }

    /// The requested range.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        self.range
    }

    /// The requested range clipped to the video.
    #[must_use]
    pub const fn searched(&self) -> TimeRange {
        self.searched
    }

    /// Candidates of this page in time order.
    #[must_use]
    pub fn candidates(&self) -> &[VisualCandidate] {
        &self.candidates
    }

    /// Opaque token for the next page, absent on the last page.
    #[must_use]
    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }

    /// Parts of the searched range analysed windows cover, merged.
    #[must_use]
    pub fn analysed(&self) -> &[TimeRange] {
        &self.analysed
    }

    /// Typed gaps in the visual coverage of the searched range.
    #[must_use]
    pub fn gaps(&self) -> &[VisualCoverageGap] {
        &self.gaps
    }

    /// How many windows this call analysed (or recorded as undecodable);
    /// zero for a warm read.
    #[must_use]
    pub const fn analysed_now(&self) -> usize {
        self.analysed_now
    }
}

/// What the analysis part of a call produced.
struct Analysis {
    newest: VisualIndex,
    status: SessionStatus,
    stop: Option<VisualExtensionStop>,
    recorded: usize,
}

impl Engine {
    /// Pages the visual candidates of a source range, analysing the range's
    /// missing windows first (see the module documentation for the order).
    ///
    /// # Errors
    ///
    /// Fails for an empty range, an invalid page size or cursor, a missing
    /// root or session, a closed or expired session, and a range that starts
    /// at or after the end of the video. When windows must be analysed, also
    /// for missing or failing media tools, a source without a video stream
    /// (`NoVideoStream`) or without a decodable one, a probe failure, a copy
    /// that changed, a stop before anything of the range was indexed, a
    /// record or session over its bounds, and a stored record that fails its
    /// integrity checks.
    pub async fn candidates(
        &self,
        request: CandidatesRequest,
    ) -> Result<CandidatesResults, EngineError> {
        let requested = TimeRange::new(
            MediaTime::from_micros(request.range.from_micros),
            MediaTime::from_micros(request.range.to_micros),
        )
        .map_err(|_| EngineError::InvalidTimeRange)?;
        let limit = request
            .limit
            .map_or(Ok(PageLimit::DEFAULT), PageLimit::new)
            .map_err(|_| EngineError::InvalidPageLimit)?;
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        let (newest, status) = match store.read_visual_index(&request.session, now)? {
            Some((index, status)) => (Some(index), status),
            None => (None, open_status(&store, &request.session, now)?),
        };
        let warm = match &newest {
            Some(index) => {
                clip_to_source(requested, index.duration())
                    .map_err(|_| EngineError::RangeOutsideSource)?;
                request.cursor.is_some()
                    || missing_windows(Some(index), index.duration(), requested).is_empty()
            }
            // Without an index no cursor can be valid, and analysing on a
            // continuation call would change the pages it continues.
            None if request.cursor.is_some() => {
                return Err(EngineError::CandidateCursorWithoutIndex);
            }
            None => false,
        };
        let (index, status, stop, analysed_now) = match newest {
            Some(index) if warm => (index, status, None, 0),
            newest => {
                let analysis = self
                    .analyse_missing(&store, &request, requested, newest, status, now)
                    .await?;
                (
                    analysis.newest,
                    analysis.status,
                    analysis.stop,
                    analysis.recorded,
                )
            }
        };
        page_results(
            &request,
            requested,
            limit,
            index,
            &status,
            stop,
            analysed_now,
            now,
        )
    }

    /// Analyses the range's missing windows and commits them (steps 2-4).
    #[allow(
        clippy::too_many_lines,
        reason = "The stage order is the contract; keep it visible in one place"
    )]
    async fn analyse_missing(
        &self,
        store: &FilesystemSessionStore,
        request: &CandidatesRequest,
        requested: TimeRange,
        mut newest: Option<VisualIndex>,
        mut status: SessionStatus,
        now: u64,
    ) -> Result<Analysis, EngineError> {
        let cancellation = request.cancellation.0.clone();
        let tools = self.visual_media_tools()?;
        self.ensure_media_tools_verified_with(&tools, &cancellation)
            .await?;
        let bound = BoundSource::open_committed(store, &request.session, now)
            .map_err(|error| EngineError::Storage(snapshot_storage_error(&error)))?;
        let media = FfmpegMedia::new(
            tools,
            self.config().host_isolation.into_infrastructure(),
            store,
        );
        let description = media
            .probe(&bound, cancellation.clone())
            .await
            .map_err(|error| EngineError::SourceProbe(probe_error(&error)))?;
        let (stream_index, displayed_dimensions) =
            description
                .visual_video_stream()
                .map_err(|error| match error {
                    VisualStreamError::Absent => EngineError::NoVideoStream,
                    VisualStreamError::Unsupported => EngineError::UnsupportedVideoStream,
                })?;
        clip_to_source(requested, description.duration)
            .map_err(|_| EngineError::RangeOutsideSource)?;
        let source_id = bound.snapshot().id().clone();
        let scope = VisualIndexScope {
            session_id: &request.session,
            source_id: &source_id,
            stream_index,
            displayed_dimensions,
            duration: description.duration,
            profile: VisualIndexProfile::R0,
        };
        let sampler = FfmpegVisualSampler::new(
            &media,
            &bound,
            &description,
            MediaSelection {
                video: Some(stream_index),
                audio: None,
            },
            cancellation.clone(),
        );
        let extension = extend_visual_index_within(
            ExtendVisualIndexRequest {
                scope,
                previous: newest.as_ref(),
                range: requested,
            },
            &sampler,
            self.visual_window_budget(),
        )
        .await;
        drop(sampler);
        let extension = match extension {
            Ok(extension) => extension,
            // The sampler cannot tell a changed copy from a provider that
            // could not run; the binding can, without reading the copy.
            Err(VisualIndexBuildError::Sampling(VisualSamplingError::Io))
                if bound.check_identity().is_err() =>
            {
                return Err(EngineError::Storage(SessionStorageError::IntegrityFailure));
            }
            Err(VisualIndexBuildError::ScopeMismatch) => {
                return Err(EngineError::Storage(SessionStorageError::IntegrityFailure));
            }
            Err(VisualIndexBuildError::RangeOutsideSource) => {
                return Err(EngineError::RangeOutsideSource);
            }
            Err(error) => return Err(EngineError::VisualIndexBuild(error)),
        };
        let stop = extension.stop;
        let recorded = extension.recorded_windows();
        if recorded.is_empty() {
            if let Some(stop) = stop
                && !indexes_any_of(newest.as_ref(), requested)
            {
                return Err(EngineError::VisualAnalysisStopped(stop));
            }
            // Missing windows were found, so a call that recorded none
            // stopped, and one that stopped with something indexed has an
            // index.
            let newest = newest.ok_or(EngineError::VisualIndexBuild(
                VisualIndexBuildError::Invalid(VisualIndexError::InvalidWindow),
            ))?;
            return Ok(Analysis {
                newest,
                status,
                stop,
                recorded: 0,
            });
        }

        // The closing full verification: nothing is committed unless the
        // copy still holds the committed bytes. The snapshot keeps the
        // session held until the commit is done.
        let snapshot = bound
            .release_verified()
            .map_err(|error| EngineError::Storage(snapshot_storage_error(&error)))?;
        let mut committed = None;
        for _ in 0..MAX_COMMIT_ATTEMPTS {
            let merged =
                merge_visual_extension(&scope, newest.as_ref(), &recorded).map_err(|error| {
                    match error {
                        VisualIndexBuildError::ScopeMismatch => {
                            EngineError::Storage(SessionStorageError::IntegrityFailure)
                        }
                        other => EngineError::VisualIndexBuild(other),
                    }
                })?;
            let Some(revision) = merged else {
                // Another call committed every window this one analysed.
                committed = newest.take();
                break;
            };
            let record = encode_visual_index_record(&request.session, &revision)?;
            match store.publish_artifact(
                &request.session,
                &self.new_operation_id()?,
                status.generation(),
                SessionArtifactKind::VisualIndexRecord,
                &record,
                self.now_unix_seconds()?,
            ) {
                Ok(_) => {
                    committed = Some(revision);
                    break;
                }
                Err(SessionStorageError::StateConflict) => {
                    // Another writer moved the generation (another index
                    // revision, a renewal, a transcript), or the session
                    // closed: re-read, which fails for a closed session.
                    let now = self.now_unix_seconds()?;
                    match store.read_visual_index(&request.session, now)? {
                        Some((index, current)) => {
                            newest = Some(index);
                            status = current;
                        }
                        None => status = open_status(store, &request.session, now)?,
                    }
                }
                Err(error) => return Err(EngineError::Storage(error)),
            }
        }
        drop(snapshot);
        let newest = committed.ok_or(EngineError::Storage(SessionStorageError::Busy))?;
        let status = store.session_status(&request.session)?;
        Ok(Analysis {
            newest,
            status,
            stop,
            recorded: recorded.len(),
        })
    }

    /// Resolves `FFmpeg` and `FFprobe` for visual analysis: a configured
    /// selection first, then the filtered `PATH`.
    fn visual_media_tools(&self) -> Result<MediaProviderConformance, EngineError> {
        self.media_tools().map_err(|error| match error {
            EngineError::MediaToolUnavailable(dependency) => {
                EngineError::VisualToolUnavailable(dependency)
            }
            other => other,
        })
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "One call's page inputs, gathered from two paths"
)]
fn page_results(
    request: &CandidatesRequest,
    requested: TimeRange,
    limit: PageLimit,
    index: VisualIndex,
    status: &SessionStatus,
    stop: Option<VisualExtensionStop>,
    analysed_now: usize,
    now: u64,
) -> Result<CandidatesResults, EngineError> {
    let searched =
        clip_to_source(requested, index.duration()).map_err(|_| EngineError::RangeOutsideSource)?;
    let expires_at = status
        .lifetime()
        .expires_at_unix_seconds()
        .saturating_mul(MICROS_PER_SECOND);
    let page = page_candidates(
        &request.session,
        &index,
        &CandidatePageRequest {
            range: requested,
            limit,
            cursor: request.cursor.clone(),
        },
        expires_at,
        now.saturating_mul(MICROS_PER_SECOND),
    )
    .map_err(EngineError::CandidateQuery)?;
    let candidates = page.candidates.into_iter().cloned().collect();
    let next_cursor = page.next_cursor.map(|cursor| cursor.encode());
    let gaps = visual_coverage_gaps(Some(&index), index.duration(), requested, stop);
    let analysed = analysed_ranges(&index, requested);
    let source_segment_id = whole_file_source_segment(index.source_id(), index.duration())
        .map_err(|_| EngineError::Storage(SessionStorageError::IntegrityFailure))?
        .id()
        .clone();
    Ok(CandidatesResults {
        session: SessionSnapshot::observe(status, now),
        index,
        source_segment_id,
        range: requested,
        searched,
        candidates,
        next_cursor,
        analysed,
        gaps,
        analysed_now,
    })
}
