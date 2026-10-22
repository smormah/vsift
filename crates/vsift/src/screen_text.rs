//! On-screen text of a session's video: `ocr` (ADR 0030).
//!
//! [`Engine::read_screen_text`] reads text off frames of a source range with
//! the OCR engine and commits what it read to the session's screen-text index.
//! It is opt-in and bounded: a call reads at most its frame budget, within its
//! deadline, and a later call continues where it stopped. The order of steps
//! is part of the contract:
//!
//! 1. The request is validated, and the OCR runner is resolved: the request's
//!    own executable, else the one `setup configure ocr-runner` recorded, else
//!    `vsift-ocr-runner` on the filtered `PATH` (a host may supply its own
//!    reader instead). No runner is `MISSING_CAPABILITY` before any work.
//! 2. The range's visual candidates are listed through [`Engine::candidates`],
//!    which analyses missing windows as it does for its own command; windows it
//!    could not analyse are reported as gaps and their candidates are planned
//!    by a later call.
//! 3. The plan is every candidate's representative frame in the range and one
//!    frame every period. Targets the index has attempted are skipped.
//! 4. The media tools and the session's copy are prepared as for evidence, the
//!    pending targets are resolved to exact frames, extracted, handed to the
//!    runner in a private work folder and read, at most the frame budget.
//! 5. The copy's identity is compared a last time and the frames read are
//!    committed as one new index revision (merged onto another call's, at most
//!    three times). A call that stopped after reading something keeps it and
//!    is `partial`; one that read nothing and stopped fails.

use std::{
    fmt,
    future::Future,
    num::NonZeroU16,
    path::PathBuf,
    pin::Pin,
    time::{Duration, Instant},
};

use vsift_application::{
    DEFAULT_SCREEN_TEXT_FRAMES_PER_CALL, EngineReading, EvidenceControl, EvidenceMediaError,
    EvidenceStop, MAX_SCREEN_TEXT_FRAMES_PER_CALL, ReadScreenTextRequest, ScreenTextBuildError,
    ScreenTextImage, ScreenTextReadError, ScreenTextReader, ScreenTextStop, SessionStorageError,
    merge_screen_text, read_screen_text_within,
};
use vsift_domain::{
    CandidateOmission, CursorError, DEFAULT_READ_PERIOD_MICROS, FailureCode,
    MAX_READ_PERIOD_MICROS, MIN_READ_PERIOD_MICROS, MediaTime, ReadTarget, ScreenTextCoverage,
    ScreenTextError, ScreenTextIndex, ScreenTextProfile, ScreenTextProvider, SessionArtifactKind,
    SessionId, TimeRange, VisualCandidateId, VisualCoverageGap, VisualStreamError, plan_reads,
};
use vsift_infrastructure::{
    BoundSource, ExecutableResolutionError, ExecutableResolver, FfmpegFrameExtractor, FfmpegMedia,
    FilesystemSessionStore, OcrRunner, OcrScreenTextReader, ProcessCancellation,
    ProcessWorkingDirectory, TrustedExecutable, encode_screen_text_index_record,
};

use crate::{
    CandidatesRange, CandidatesRequest,
    asr::{admission_fits, open_status, probe_error, snapshot_storage_error},
    engine::Engine,
    error::{EngineError, ExecutableRejection, SessionRootError},
    sessions::SessionSnapshot,
    verification::Cancellation,
};

/// How long one `ocr` call may prepare, extract and read, after its
/// candidates. Every media and runner process of the call is given at most
/// the time left, so a run started late cannot carry the call past it (RV-2
/// finding 4); only the final commit follows it.
const CALL_DEADLINE: Duration = Duration::from_secs(300);
/// How often a commit that lost a race with another writer is merged and retried.
const MAX_COMMIT_ATTEMPTS: usize = 3;
/// Inference threads unless the caller chooses: two, so a reading and a frame
/// extraction fit the default admission capacity of four together.
pub const DEFAULT_OCR_THREADS: u16 = 2;
/// The runner's name on `PATH`.
pub const OCR_RUNNER_NAME: &str = "vsift-ocr-runner";
/// The longest range one call plans: four hours, the longest source.
const MAX_PLANNED_MICROS: u64 = 4 * 60 * 60 * 1_000_000;

/// A half-open source range to read, in microseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenTextRange {
    /// Inclusive start.
    pub from_micros: u64,
    /// Exclusive end.
    pub to_micros: u64,
}

/// Which visual windows a call plans from.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ScreenTextWindows {
    /// Analyse the range's missing windows first, as `candidates` does, then
    /// plan the whole range: what `vsift ocr` does.
    #[default]
    AnalyseMissing,
    /// Never analyse: plan only the frames inside windows the visual index has
    /// already recorded, and count the periodic reads of the other windows as
    /// awaiting analysis (ADR 0033's background `prepare` job, which reads text
    /// after its visual stream has analysed the windows). Fails with
    /// [`OcrError::WindowsNotAnalysed`] while no window is analysed.
    AnalysedOnly,
}

/// A bounded request to read on-screen text off a session's frames.
#[derive(Clone, Debug)]
pub struct ScreenTextRequest {
    /// The session.
    pub session: SessionId,
    /// The range; `None` for the whole video. A range past the end is clipped.
    pub range: Option<ScreenTextRange>,
    /// The period of the frames read between candidates, from one second to
    /// ten minutes; `None` for ten seconds.
    pub period_micros: Option<u64>,
    /// Most frames this call reads, 1 to 200; `None` for 40.
    pub max_frames: Option<u16>,
    /// The runner executable for this call only; `None` resolves it.
    pub runner: Option<PathBuf>,
    /// Inference threads, 1 to the root's admission capacity; `None` for two.
    pub threads: Option<NonZeroU16>,
    /// Whether the call may analyse visual windows itself.
    pub windows: ScreenTextWindows,
    /// Stops the call at its next step; what was read before is committed.
    pub cancellation: Cancellation,
}

/// Why the plan of a range is not wholly read yet, beside its counts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTextResults {
    session: SessionSnapshot,
    index: Option<ScreenTextIndex>,
    range: TimeRange,
    plan: Vec<ReadTarget>,
    coverage: ScreenTextCoverage,
    read_now: usize,
    stop: Option<ScreenTextStop>,
    visual_gaps: Vec<VisualCoverageGap>,
    period_micros: u64,
}

impl ScreenTextResults {
    /// Session state observed after the call.
    #[must_use]
    pub const fn session(&self) -> &SessionSnapshot {
        &self.session
    }

    /// The newest index after the call, if any frame has been read.
    #[must_use]
    pub const fn index(&self) -> Option<&ScreenTextIndex> {
        self.index.as_ref()
    }

    /// The range read, clipped to the video.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        self.range
    }

    /// The range's read targets.
    #[must_use]
    pub fn plan(&self) -> &[ReadTarget] {
        &self.plan
    }

    /// How much of the plan the index has read.
    #[must_use]
    pub const fn coverage(&self) -> ScreenTextCoverage {
        self.coverage
    }

    /// Frames this call read or found unreadable.
    #[must_use]
    pub const fn read_now(&self) -> usize {
        self.read_now
    }

    /// Why this call stopped before the plan was done, if it did.
    #[must_use]
    pub const fn stop(&self) -> Option<ScreenTextStop> {
        self.stop
    }

    /// The visual index's gaps in the range, as `candidates` reports them.
    #[must_use]
    pub fn visual_gaps(&self) -> &[VisualCoverageGap] {
        &self.visual_gaps
    }

    /// Whether the plan holds every candidate the range will have: no window
    /// is still to be analysed (`not_analyzed`) or to be tried again
    /// (`deadline_exceeded`). A window the media tools cannot decode, a cell
    /// without a decoded frame and a window over its candidate budget are
    /// final: no later call adds candidates there, and their periodic reads
    /// are planned like any other.
    #[must_use]
    pub fn candidates_complete(&self) -> bool {
        !self.visual_gaps.iter().any(|gap| gap.reason.is_retryable())
    }

    /// The period the plan used.
    #[must_use]
    pub const fn period_micros(&self) -> u64 {
        self.period_micros
    }
}

/// Why `ocr` failed (ADR 0030); every case committed nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OcrError {
    /// No OCR runner was given, configured or found on `PATH`.
    RunnerUnavailable,
    /// The runner could not identify itself.
    Failed(ScreenTextReadError),
    /// Reading stopped (deadline, cancellation or a failure) before anything was read.
    Stopped(ScreenTextStop),
    /// A new index revision would break a bound or an identity rule.
    Index(ScreenTextError),
    /// The session holds as many screen-text revisions as it may.
    BudgetExhausted,
    /// The read period is outside one second to ten minutes.
    InvalidPeriod,
    /// The frame budget is outside 1 to 200.
    InvalidBudget,
    /// A call that may not analyse windows found none analysed in the session.
    WindowsNotAnalysed,
    /// An `ocr search` cursor is malformed, expired, or of another search,
    /// session or index revision.
    Cursor(CursorError),
}

impl OcrError {
    /// The stable public failure code: a runner that cannot run, load its models
    /// or answer in its protocol is a missing capability, as for whisper.cpp.
    #[must_use]
    pub const fn failure_code(self) -> FailureCode {
        match self {
            Self::RunnerUnavailable => FailureCode::MissingCapability,
            Self::Failed(error) => read_failure_code(error),
            Self::Stopped(stop) => stop_failure_code(stop),
            Self::Index(ScreenTextError::TooLarge) | Self::BudgetExhausted => {
                FailureCode::ResourceLimit
            }
            Self::Index(_) => FailureCode::Internal,
            Self::InvalidPeriod
            | Self::InvalidBudget
            | Self::Cursor(_)
            | Self::WindowsNotAnalysed => FailureCode::InvalidArgument,
        }
    }
}

const fn read_failure_code(error: ScreenTextReadError) -> FailureCode {
    match error {
        ScreenTextReadError::Unavailable
        | ScreenTextReadError::ModelUnavailable
        | ScreenTextReadError::ProviderFailed
        | ScreenTextReadError::UnparseableOutput => FailureCode::MissingCapability,
        ScreenTextReadError::Busy => FailureCode::Busy,
        ScreenTextReadError::Deadline => FailureCode::DeadlineExceeded,
        ScreenTextReadError::Cancelled => FailureCode::Cancelled,
        ScreenTextReadError::ResourceLimit | ScreenTextReadError::AbnormalTermination => {
            FailureCode::ResourceLimit
        }
        ScreenTextReadError::Workspace => FailureCode::StorageIo,
        // The runner's files changed between `describe` and a `read`: what it
        // would have read cannot be attributed to the build the index records.
        ScreenTextReadError::BuildChanged => FailureCode::IntegrityFailure,
    }
}

const fn stop_failure_code(stop: ScreenTextStop) -> FailureCode {
    match stop {
        ScreenTextStop::FrameLimit
        | ScreenTextStop::IndexFull
        | ScreenTextStop::Media(EvidenceMediaError::ResourceLimit) => FailureCode::ResourceLimit,
        ScreenTextStop::Deadline | ScreenTextStop::Media(EvidenceMediaError::Deadline) => {
            FailureCode::DeadlineExceeded
        }
        ScreenTextStop::Cancelled | ScreenTextStop::Media(EvidenceMediaError::Cancelled) => {
            FailureCode::Cancelled
        }
        ScreenTextStop::Engine(error) => read_failure_code(error),
        ScreenTextStop::Media(EvidenceMediaError::Busy) => FailureCode::Busy,
        ScreenTextStop::Media(EvidenceMediaError::SourceChanged) => FailureCode::IntegrityFailure,
        ScreenTextStop::Media(
            EvidenceMediaError::Undecodable | EvidenceMediaError::FrameNotFound,
        ) => FailureCode::InvalidSource,
        ScreenTextStop::Media(
            EvidenceMediaError::Unavailable
            | EvidenceMediaError::Invalid
            | EvidenceMediaError::NoDecodedAudio,
        ) => FailureCode::MissingCapability,
    }
}

impl fmt::Display for OcrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunnerUnavailable => {
                formatter.write_str("no OCR runner was given, configured or found")
            }
            Self::Failed(error) => error.fmt(formatter),
            Self::Stopped(stop) => formatter.write_str(match stop {
                ScreenTextStop::FrameLimit => "reading screen text reached its frame budget",
                ScreenTextStop::Deadline => "reading screen text exceeded its deadline",
                ScreenTextStop::Cancelled => "reading screen text was cancelled",
                ScreenTextStop::Engine(_) => "the OCR runner failed before anything was read",
                ScreenTextStop::Media(_) => "the media tools failed before anything was read",
                ScreenTextStop::IndexFull => "the screen-text index has no room for more frames",
            }),
            Self::Index(error) => error.fmt(formatter),
            Self::BudgetExhausted => {
                formatter.write_str("the session holds as many screen-text revisions as it may")
            }
            Self::InvalidPeriod => {
                formatter.write_str("the read period is outside one second to ten minutes")
            }
            Self::InvalidBudget => formatter.write_str("the frame budget is outside 1 to 200"),
            Self::WindowsNotAnalysed => {
                formatter.write_str("no visual window of the session is analysed yet")
            }
            Self::Cursor(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for OcrError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Failed(error) => Some(error),
            Self::Index(error) => Some(error),
            Self::Cursor(error) => Some(error),
            Self::RunnerUnavailable
            | Self::Stopped(_)
            | Self::BudgetExhausted
            | Self::InvalidPeriod
            | Self::InvalidBudget
            | Self::WindowsNotAnalysed => None,
        }
    }
}

struct CallControl {
    deadline: Instant,
    cancellation: ProcessCancellation,
}

impl EvidenceControl for CallControl {
    fn stop(&self) -> Option<EvidenceStop> {
        if self.cancellation.is_cancelled() {
            Some(EvidenceStop::Cancelled)
        } else if Instant::now() >= self.deadline {
            Some(EvidenceStop::DeadlineExceeded)
        } else {
            None
        }
    }
}

impl Engine {
    /// Reads on-screen text off the frames of a range (see the module documentation).
    ///
    /// # Errors
    ///
    /// Fails for an invalid range, period, budget or thread count; a missing
    /// root or session, or a closed or expired one; no OCR runner
    /// ([`OcrError::RunnerUnavailable`]); the failures of `candidates`;
    /// missing or failing media tools; a runner that cannot identify itself
    /// ([`OcrError::Failed`]); a stop before
    /// anything was read; a copy that changed; and a record or session over its
    /// bounds. Contention while a transcription runs is named as for evidence.
    pub async fn read_screen_text(
        &self,
        request: ScreenTextRequest,
    ) -> Result<ScreenTextResults, EngineError> {
        let session = request.session.clone();
        self.read_screen_text_unnamed(request)
            .await
            .map_err(|error| self.name_running_transcription(&session, error))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "The stage order is the contract; keep it visible in one place"
    )]
    async fn read_screen_text_unnamed(
        &self,
        request: ScreenTextRequest,
    ) -> Result<ScreenTextResults, EngineError> {
        let period = request.period_micros.unwrap_or(DEFAULT_READ_PERIOD_MICROS);
        if !(MIN_READ_PERIOD_MICROS..=MAX_READ_PERIOD_MICROS).contains(&period) {
            return Err(EngineError::Ocr(OcrError::InvalidPeriod));
        }
        let max_frames = request
            .max_frames
            .unwrap_or(DEFAULT_SCREEN_TEXT_FRAMES_PER_CALL);
        if max_frames == 0 || max_frames > MAX_SCREEN_TEXT_FRAMES_PER_CALL {
            return Err(EngineError::Ocr(OcrError::InvalidBudget));
        }
        let requested = match request.range {
            Some(range) => TimeRange::new(
                MediaTime::from_micros(range.from_micros),
                MediaTime::from_micros(range.to_micros),
            )
            .map_err(|_| EngineError::InvalidTimeRange)?,
            None => TimeRange::new(
                MediaTime::from_micros(0),
                MediaTime::from_micros(MAX_PLANNED_MICROS),
            )
            .map_err(|_| EngineError::InvalidTimeRange)?,
        };
        let threads = request
            .threads
            .unwrap_or(NonZeroU16::new(DEFAULT_OCR_THREADS).unwrap_or(NonZeroU16::MIN));
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        open_status(&store, &request.session, now)?;
        admission_fits(&store, threads)?;
        let runner = match self.host_screen_text_reader() {
            Some(_) => None,
            None => Some(OcrRunner::new(
                self.resolve_ocr_runner(request.runner.clone())?,
                threads,
                self.config().host_isolation.into_infrastructure(),
            )),
        };
        let (plan, searched, visual_gaps, awaiting_analysis) = match request.windows {
            ScreenTextWindows::AnalyseMissing => {
                let (candidates, searched, visual_gaps) =
                    self.all_candidates(&request, requested).await?;
                let plan = plan_reads(searched, &candidates, period)
                    .map_err(|_| EngineError::Ocr(OcrError::InvalidPeriod))?;
                (plan, searched, visual_gaps, 0)
            }
            ScreenTextWindows::AnalysedOnly => {
                analysed_plan(&store, &request.session, requested, period, now)?
            }
        };
        let call_deadline = Instant::now() + CALL_DEADLINE;
        let (previous, status) =
            store.read_screen_text_index(&request.session, self.now_unix_seconds()?)?;
        let revisions_left =
            store.screen_text_revisions_left(&request.session, self.now_unix_seconds()?)?;
        let cancellation = request.cancellation.0.clone();
        let tools = self.media_tools()?;
        self.ensure_media_tools_verified_with(&tools, &cancellation)
            .await?;
        let bound = BoundSource::open_committed(&store, &request.session, now)
            .map_err(|error| EngineError::Storage(snapshot_storage_error(&error)))?;
        let media = FfmpegMedia::new(
            tools,
            self.config().host_isolation.into_infrastructure(),
            &store,
        )
        .with_call_deadline(call_deadline);
        let description = media
            .probe(&bound, cancellation.clone())
            .await
            .map_err(|error| EngineError::SourceProbe(probe_error(&error)))?;
        let (stream, displayed) =
            description
                .visual_video_stream()
                .map_err(|error| match error {
                    VisualStreamError::Absent => EngineError::NoVideoStream,
                    VisualStreamError::Unsupported => EngineError::UnsupportedVideoStream,
                })?;
        let extractor = FfmpegFrameExtractor::new(
            &media,
            &bound,
            &description,
            stream,
            displayed,
            cancellation.clone(),
        )
        .map_err(EngineError::EvidenceMedia)?;
        let control = CallControl {
            deadline: call_deadline,
            cancellation: cancellation.clone(),
        };
        let source_id = status.source_id().clone();
        let read_request = ReadScreenTextRequest {
            session_id: &request.session,
            source_id: &source_id,
            profile: ScreenTextProfile::PpOcrV5LatinV1,
            previous: previous.as_ref(),
            plan: &plan,
            max_frames,
            revisions_left,
        };
        let extension = match (&runner, self.host_screen_text_reader()) {
            (_, Some(host)) => {
                read_screen_text_within(read_request, &extractor, &host, &control).await
            }
            (Some(runner), None) => {
                let work = store.session_work_directory(&request.session, now)?;
                let workspace = ProcessWorkingDirectory::new(work.path()).map_err(|_| {
                    EngineError::Ocr(OcrError::Failed(ScreenTextReadError::Workspace))
                })?;
                let reader = OcrScreenTextReader::new(
                    runner.clone().with_call_deadline(call_deadline),
                    workspace,
                    &store,
                    cancellation.clone(),
                );
                let read =
                    read_screen_text_within(read_request, &extractor, &reader, &control).await;
                drop(reader);
                drop(work);
                read
            }
            (None, None) => return Err(EngineError::Ocr(OcrError::RunnerUnavailable)),
        };
        drop(extractor);
        drop(media);
        let extension = extension.map_err(|error| match error {
            ScreenTextBuildError::Engine(error) => EngineError::Ocr(OcrError::Failed(error)),
            ScreenTextBuildError::IndexFull => EngineError::Ocr(OcrError::BudgetExhausted),
            ScreenTextBuildError::ScopeMismatch => {
                EngineError::Storage(SessionStorageError::IntegrityFailure)
            }
            ScreenTextBuildError::Index(error) => EngineError::Ocr(OcrError::Index(error)),
        })?;
        let read_now = extension.frames.len();
        if read_now == 0
            && extension.unresolved.is_empty()
            && extension.joined.is_empty()
            && let Some(stop) = extension.stop
            && stop != ScreenTextStop::FrameLimit
        {
            return Err(EngineError::Ocr(OcrError::Stopped(stop)));
        }
        let snapshot = bound
            .release_identity_checked()
            .map_err(|error| EngineError::Storage(snapshot_storage_error(&error)))?;
        let index = self.commit_screen_text(
            &store,
            &request.session,
            previous,
            status,
            &extension,
            period,
        )?;
        drop(snapshot);
        let now = self.now_unix_seconds()?;
        let status = crate::sessions::published_status(&store, &request.session)?;
        let coverage = ScreenTextCoverage {
            awaiting_analysis,
            ..ScreenTextCoverage::of(
                &plan,
                index
                    .as_ref()
                    .filter(|index| index.scope().provider == extension.scope.provider),
            )
        };
        Ok(ScreenTextResults {
            session: SessionSnapshot::observe(&status, now),
            index,
            range: searched,
            plan,
            coverage,
            read_now,
            stop: extension.stop,
            visual_gaps,
            period_micros: period,
        })
    }

    /// Commits the call's reading onto the newest index, merged again after
    /// another writer's commit, at most three times.
    fn commit_screen_text(
        &self,
        store: &FilesystemSessionStore,
        session: &SessionId,
        mut previous: Option<ScreenTextIndex>,
        mut status: vsift_infrastructure::SessionStatus,
        extension: &vsift_application::ScreenTextExtension,
        period: u64,
    ) -> Result<Option<ScreenTextIndex>, EngineError> {
        for _ in 0..MAX_COMMIT_ATTEMPTS {
            let merged = merge_screen_text(previous.as_ref(), extension, period).map_err(
                |error| match error {
                    ScreenTextBuildError::ScopeMismatch | ScreenTextBuildError::Engine(_) => {
                        EngineError::Storage(SessionStorageError::IntegrityFailure)
                    }
                    ScreenTextBuildError::IndexFull => EngineError::Ocr(OcrError::BudgetExhausted),
                    ScreenTextBuildError::Index(error) => EngineError::Ocr(OcrError::Index(error)),
                },
            )?;
            let Some(revision) = merged else {
                return Ok(previous);
            };
            let record = encode_screen_text_index_record(&revision)?;
            match store.publish_artifact(
                session,
                &self.new_operation_id()?,
                status.generation(),
                SessionArtifactKind::ScreenTextIndexRecord,
                &record,
                self.now_unix_seconds()?,
            ) {
                Ok(_) => return Ok(Some(revision)),
                Err(SessionStorageError::StateConflict) => {
                    let (newest, current) =
                        store.read_screen_text_index(session, self.now_unix_seconds()?)?;
                    previous = newest;
                    status = current;
                }
                Err(SessionStorageError::CapacityExhausted) => {
                    return Err(EngineError::Ocr(OcrError::BudgetExhausted));
                }
                Err(error) => return Err(EngineError::Storage(error)),
            }
        }
        Err(EngineError::Storage(SessionStorageError::Busy))
    }

    /// Every candidate of the range, through `candidates` (which analyses
    /// missing windows), the range clipped to the video, and the visual gaps.
    async fn all_candidates(
        &self,
        request: &ScreenTextRequest,
        requested: TimeRange,
    ) -> Result<
        (
            Vec<(VisualCandidateId, MediaTime)>,
            TimeRange,
            Vec<VisualCoverageGap>,
        ),
        EngineError,
    > {
        let mut candidates = Vec::new();
        let mut cursor = None;
        loop {
            let page = self
                .candidates(CandidatesRequest {
                    session: request.session.clone(),
                    range: CandidatesRange {
                        from_micros: requested.start().as_micros(),
                        to_micros: requested.end().as_micros(),
                    },
                    limit: Some(100),
                    cursor: cursor.take(),
                    omission: CandidateOmission::Nothing,
                    cancellation: request.cancellation.clone(),
                })
                .await?;
            candidates.extend(
                page.candidates()
                    .iter()
                    .map(|candidate| (candidate.id().clone(), candidate.representative())),
            );
            match page.next_cursor() {
                Some(next) => cursor = Some(next.to_owned()),
                None => return Ok((candidates, page.searched(), page.gaps().to_vec())),
            }
        }
    }

    /// The runner executable: `selected`, else the configured one, else `PATH`.
    fn resolve_ocr_runner(
        &self,
        selected: Option<PathBuf>,
    ) -> Result<TrustedExecutable, EngineError> {
        let configured = match selected {
            Some(path) => Some(path),
            None => self
                .user_configuration()?
                .read_ocr_runner()
                .map_err(EngineError::from)?,
        };
        match configured {
            Some(path) => TrustedExecutable::explicit(path),
            None => ExecutableResolver::from_current_path()
                .resolve(std::ffi::OsStr::new(OCR_RUNNER_NAME)),
        }
        .map_err(|error| match error {
            ExecutableResolutionError::NotFound => EngineError::Ocr(OcrError::RunnerUnavailable),
            other => EngineError::Executable(ExecutableRejection::from(&other)),
        })
    }

    /// Saves the user-selected OCR runner without running it.
    ///
    /// # Errors
    ///
    /// Fails when the path is not an absolute regular file or the per-user
    /// configuration cannot be written.
    pub fn configure_ocr_runner(&self, executable: &std::path::Path) -> Result<(), EngineError> {
        self.user_configuration()?
            .configure_ocr_runner(executable)
            .map_err(EngineError::from)
    }

    /// Reads the session's newest screen-text index without running anything.
    ///
    /// # Errors
    ///
    /// Fails for a missing root or session, a closed or expired session, and a
    /// stored record that fails its checks.
    pub fn screen_text_index(
        &self,
        session: &SessionId,
    ) -> Result<Option<ScreenTextIndex>, EngineError> {
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        Ok(store.read_screen_text_index(session, now)?.0)
    }
}

type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ScreenTextProvider, ScreenTextReadError>> + Send + 'a>>;
type ReadingFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<EngineReading>, ScreenTextReadError>> + Send + 'a>>;

/// Object-safe form of [`ScreenTextReader`], so a host's reader can be stored.
pub(crate) trait HostScreenTextReader: Send + Sync {
    fn identity_boxed(&self) -> ProviderFuture<'_>;
    fn max_images_boxed(&self) -> usize;
    fn read_boxed<'a>(
        &'a self,
        build: &'a ScreenTextProvider,
        images: &'a [ScreenTextImage<'a>],
    ) -> ReadingFuture<'a>;
}

impl<Reader: ScreenTextReader> HostScreenTextReader for Reader {
    fn identity_boxed(&self) -> ProviderFuture<'_> {
        Box::pin(self.identity())
    }

    fn max_images_boxed(&self) -> usize {
        self.max_images_per_run()
    }

    fn read_boxed<'a>(
        &'a self,
        build: &'a ScreenTextProvider,
        images: &'a [ScreenTextImage<'a>],
    ) -> ReadingFuture<'a> {
        Box::pin(self.read(build, images))
    }
}

/// A borrowed host reader as the application's port.
pub(crate) struct HostScreenTextRef<'a>(pub(crate) &'a dyn HostScreenTextReader);

impl ScreenTextReader for HostScreenTextRef<'_> {
    async fn identity(&self) -> Result<ScreenTextProvider, ScreenTextReadError> {
        self.0.identity_boxed().await
    }

    fn max_images_per_run(&self) -> usize {
        self.0.max_images_boxed()
    }

    async fn read(
        &self,
        build: &ScreenTextProvider,
        images: &[ScreenTextImage<'_>],
    ) -> Result<Vec<EngineReading>, ScreenTextReadError> {
        self.0.read_boxed(build, images).await
    }
}

/// The plan of `requested` from the windows the session's visual index already
/// holds ([`ScreenTextWindows::AnalysedOnly`]): the range clipped to the video,
/// the visual gaps, and the periodic reads left out until their windows are
/// analysed. Reads the index; analyses nothing.
fn analysed_plan(
    store: &FilesystemSessionStore,
    session: &SessionId,
    requested: TimeRange,
    period: u64,
    now: u64,
) -> Result<(Vec<ReadTarget>, TimeRange, Vec<VisualCoverageGap>, usize), EngineError> {
    let (visual, _) = store
        .read_visual_index(session, now)?
        .ok_or(EngineError::Ocr(OcrError::WindowsNotAnalysed))?;
    let planned =
        vsift_application::plan_analysed_windows(&visual, requested, period).map_err(|error| {
            match error {
                vsift_application::AnalysedPlanError::RangeOutsideSource => {
                    EngineError::RangeOutsideSource
                }
                vsift_application::AnalysedPlanError::Period => {
                    EngineError::Ocr(OcrError::InvalidPeriod)
                }
            }
        })?;
    Ok((
        planned.plan,
        planned.range,
        planned.gaps,
        planned.awaiting_analysis,
    ))
}
