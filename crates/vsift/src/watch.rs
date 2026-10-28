//! `vsift <video>`, explicitly `vsift watch <video>` (ADR 0037): one action
//! that turns a video into a readable transcript, subtitles and its screen
//! changes with their frames, written beside it or into a folder the caller
//! names.
//!
//! The orchestration is the application's [`run_watch`] use case; this module
//! is its facade, [`Engine::watch`], and supplies the use case's two ports:
//! [`WatchSteps`] over the engine's own operations (ingest, the `prepare` job
//! in the watched mode in this process, the transcript and candidate reads,
//! `frame get` and closing the session), and the infrastructure's
//! [`FolderWatchOutputs`] for the person's folder.
//!
//! Every file is written once, when the job has ended; the files that grow
//! while it runs, frames written during the transcription and continuing an
//! interrupted run are the second pull request's (ADR 0037 section 13).

use std::{
    ffi::OsStr,
    fmt,
    path::{Path, PathBuf},
};

use vsift_application::{
    FrameRefusal, OpenedWatchSession, PreparedRun, PreparedTranscript, ScreenChange, ScreenChanges,
    SessionFrame, TranscriptLine, WatchError, WatchOutputError, WatchProvenance, WatchSteps,
    run_watch,
};
use vsift_contract::render_hidden_characters;
use vsift_domain::{
    JobMode, MAX_VISUAL_INDEX_DURATION_MICROS, SearchCoverage, SessionId, SourceId, TimeRange,
    TranscriptRevisionId, VisualCandidateId,
};
use vsift_infrastructure::{FolderWatchOutputs, ManagedArtifactStore};

pub use vsift_application::{WatchFile, WatchFrame, WatchOutcome, WatchOutputKind};
pub use vsift_domain::OutputNameError;

use crate::{
    AdmissionWait, Cancellation, CandidateOmission, CandidatesRange, CandidatesRequest,
    DurabilityRequirement, FrameGetRequest, FrameTarget, IngestRequest, JobResumeRequest,
    JobStartRequest, ProgressObserver, RetranscribePublication, TranscriptQuery,
    engine::{Engine, ManagedRootLocation},
    error::EngineError,
};

/// A page of transcript segments or screen changes `watch` reads at a time.
const READ_PAGE: u16 = 100;

/// A request to watch one video.
#[derive(Clone, Debug)]
pub struct WatchRequest {
    /// The video; a relative path is resolved against the process working
    /// directory.
    pub source: PathBuf,
    /// The folder to write into; `None` writes beside the video.
    pub output: Option<PathBuf>,
    /// Stops the run at its next provider boundary; the claimed outputs are
    /// then released and the session closed.
    pub cancellation: Cancellation,
    /// Receives the `prepare` job's progress (ADR 0033).
    pub progress: ProgressObserver,
}

/// Why `watch` could not write its outputs (ADR 0037 section 5).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchFailure {
    /// No output names can be derived from the video's file name.
    UnusableName(OutputNameError),
    /// A file or folder already exists at an output's name; nothing was
    /// replaced.
    OutputExists(WatchOutputKind),
    /// The folder refused the write.
    OutputRefused(WatchOutputKind),
    /// `--output` names a folder inside `VSift`'s own session or managed
    /// root, which `VSift` cleans automatically.
    OutputInsideVsift,
    /// `--output` names a folder whose parent does not exist, or something
    /// that is not a folder.
    OutputFolderUnusable,
    /// A frame copied out of the session does not have its recorded SHA-256.
    FrameMismatch,
}

impl WatchFailure {
    /// The stable public failure code: no new code (ADR 0037 section 9).
    #[must_use]
    pub const fn failure_code(self) -> vsift_domain::FailureCode {
        match self {
            Self::UnusableName(_)
            | Self::OutputExists(_)
            | Self::OutputInsideVsift
            | Self::OutputFolderUnusable => vsift_domain::FailureCode::InvalidArgument,
            Self::OutputRefused(_) => vsift_domain::FailureCode::StorageIo,
            Self::FrameMismatch => vsift_domain::FailureCode::IntegrityFailure,
        }
    }
}

impl fmt::Display for WatchFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnusableName(error) => write!(formatter, "{error}"),
            Self::OutputExists(_) => formatter.write_str("an output already exists"),
            Self::OutputRefused(_) => formatter.write_str("the folder refused an output"),
            Self::OutputInsideVsift => {
                formatter.write_str("the output folder is inside VSift's own folders")
            }
            Self::OutputFolderUnusable => formatter.write_str("the output folder cannot be used"),
            Self::FrameMismatch => formatter.write_str("a copied frame does not match its digest"),
        }
    }
}

impl std::error::Error for WatchFailure {}

impl From<WatchOutputError> for WatchFailure {
    fn from(error: WatchOutputError) -> Self {
        match error {
            WatchOutputError::Exists(kind) => Self::OutputExists(kind),
            WatchOutputError::Refused(kind, _) => Self::OutputRefused(kind),
            WatchOutputError::FrameMismatch => Self::FrameMismatch,
            WatchOutputError::FolderUnusable => Self::OutputFolderUnusable,
            WatchOutputError::InsideVsift => Self::OutputInsideVsift,
        }
    }
}

impl Engine {
    /// Watches one video (ADR 0037): writes `<stem>.txt`, `<stem>.srt` and
    /// `<stem>-vsift/` beside it or into `request.output`, through the
    /// application's [`run_watch`].
    ///
    /// Nothing is copied or written before every output name is claimed, and
    /// nothing that exists is ever replaced. A failure or cancellation after
    /// the claim releases it (removing only what this run created and never
    /// a written file) and closes the session.
    ///
    /// # Errors
    ///
    /// [`EngineError::Watch`] for the outputs, `MISSING_CAPABILITY` errors
    /// when video tools or speech recognition are not set up, and every error
    /// of ingest, the `prepare` job, the reads and `frame get`.
    pub async fn watch(&self, request: WatchRequest) -> Result<WatchOutcome, EngineError> {
        let video = absolute(&request.source)?;
        let output = request.output.as_deref().map(absolute).transpose()?;
        let steps = EngineWatchSteps {
            engine: self,
            cancellation: &request.cancellation,
            progress: &request.progress,
        };
        let mut outputs = FolderWatchOutputs::new(self.vsift_roots());
        Box::pin(run_watch(&steps, &mut outputs, &video, output.as_deref()))
            .await
            .map_err(|error| match error {
                WatchError::Step(error) => error,
                WatchError::UnusableName(error) => {
                    EngineError::Watch(WatchFailure::UnusableName(error))
                }
                WatchError::Output(error) => EngineError::Watch(error.into()),
            })
    }

    /// The canonical session and managed roots that exist.
    fn vsift_roots(&self) -> Vec<PathBuf> {
        let managed = match &self.config().managed_root {
            ManagedRootLocation::PlatformDefault => ManagedArtifactStore::default_location()
                .ok()
                .map(|store| store.root_path().to_path_buf()),
            ManagedRootLocation::Explicit(path) => Some(path.clone()),
        };
        [self.session_root_path().ok(), managed]
            .into_iter()
            .flatten()
            .filter_map(|root| std::fs::canonicalize(root).ok())
            .collect()
    }

    /// Opens the session of the video, as `ingest` does.
    async fn watch_ingest(
        &self,
        source: &Path,
        cancellation: &Cancellation,
    ) -> Result<vsift_application::OpenSessionOutcome, EngineError> {
        Ok(self
            .ingest(IngestRequest {
                source: source.to_path_buf(),
                transcript: None,
                cancellation: cancellation.clone(),
                durability: DurabilityRequirement::Ephemeral,
            })
            .await?
            .session)
    }

    /// Video tools and speech recognition must be configured. Resolving them
    /// runs no tool (design note C9): a broken tool is found by the run.
    fn watch_preflight(&self) -> Result<(), EngineError> {
        drop(self.media_tools()?);
        if self.host_asr().is_some() {
            return Ok(());
        }
        let store = self.user_configuration()?;
        let mut managed = self.managed_lookup();
        drop(crate::asr::resolve_recognizer(
            &mut managed,
            store.read()?.whisper,
        )?);
        if store.read_model()?.is_none() && managed.model().is_none() {
            return Err(EngineError::ModelNotSelected);
        }
        Ok(())
    }

    /// Starts the session's `prepare` job in the watched mode and runs it in
    /// this process (ADR 0037 section 3, step 4).
    async fn watch_prepare(
        &self,
        session: &SessionId,
        cancellation: &Cancellation,
        progress: &ProgressObserver,
    ) -> Result<PreparedRun, EngineError> {
        let plan = self.job_start(&JobStartRequest {
            session: session.clone(),
            mode: Some(JobMode::Watched),
            first: None,
        })?;
        let job_id = plan.job_id().clone();
        let resumed = self
            .job_resume(JobResumeRequest {
                job: job_id.clone(),
                cancellation: cancellation.clone(),
                progress: progress.clone(),
                admission: AdmissionWait::Immediate,
                publication: RetranscribePublication::AsItGoes {
                    interval: crate::PUBLICATION_INTERVAL,
                },
            })
            .await;
        let transcript = match resumed {
            Ok(report) => {
                let revision = report.outcome().revision();
                Some(PreparedTranscript {
                    revision: revision.id().clone(),
                    range: revision.source_segment().range(),
                    untranscribed: SearchCoverage::of(revision, None).untranscribed().to_vec(),
                })
            }
            Err(EngineError::NoAudioStream) => None,
            Err(error) => return Err(error),
        };
        Ok(PreparedRun { job_id, transcript })
    }

    /// Every segment of `revision` within `range`, its text in display form.
    fn watch_transcript(
        &self,
        session: &SessionId,
        revision: &TranscriptRevisionId,
        range: TimeRange,
    ) -> Result<Vec<TranscriptLine>, EngineError> {
        let mut lines = Vec::new();
        let mut cursor = None;
        loop {
            let page = self.transcript(TranscriptQuery {
                session: session.clone(),
                revision: Some(revision.clone()),
                from_micros: range.start().as_micros(),
                to_micros: range.end().as_micros(),
                limit: Some(READ_PAGE),
                cursor: cursor.take(),
                since: None,
            })?;
            lines.extend(page.segments().iter().map(|segment| TranscriptLine {
                start: segment.range().start(),
                end: segment.range().end(),
                text: render_hidden_characters(segment.text().text()),
            }));
            match page.next_cursor() {
                Some(next) => cursor = Some(next.to_owned()),
                None => break,
            }
        }
        Ok(lines)
    }

    /// Every screen change of the analysed video, with the index revision and
    /// the end of the range it covers; nothing when the video has no picture.
    async fn watch_changes(
        &self,
        session: &SessionId,
        cancellation: &Cancellation,
    ) -> Result<Option<ScreenChanges>, EngineError> {
        let mut changes = Vec::new();
        let mut index;
        let mut cursor = None;
        loop {
            let page = match self
                .candidates(CandidatesRequest {
                    session: session.clone(),
                    range: CandidatesRange {
                        from_micros: 0,
                        to_micros: MAX_VISUAL_INDEX_DURATION_MICROS,
                    },
                    limit: Some(READ_PAGE),
                    cursor: cursor.take(),
                    omission: CandidateOmission::Nothing,
                    cancellation: cancellation.clone(),
                })
                .await
            {
                Ok(page) => page,
                Err(EngineError::VisualToolUnavailable(_) | EngineError::NoVideoStream) => {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
            index = Some((page.index().id().clone(), page.range().end()));
            changes.extend(page.candidates().iter().map(|candidate| ScreenChange {
                id: candidate.id().clone(),
                time: candidate.representative(),
                reason: candidate.reason(),
            }));
            match page.next_cursor() {
                Some(next) => cursor = Some(next.to_owned()),
                None => break,
            }
        }
        Ok(index.map(|(index, covered_to)| ScreenChanges {
            index,
            covered_to,
            changes,
        }))
    }

    /// One change's frame, stored in the session by `frame get`. A session
    /// whose evidence is full answers [`FrameRefusal::NoRoom`], which ends
    /// the frames, not the run (known limit L-234).
    async fn watch_frame(
        &self,
        session: &SessionId,
        change: &VisualCandidateId,
        cancellation: &Cancellation,
    ) -> Result<SessionFrame, FrameRefusal<EngineError>> {
        let results = self
            .frame_get(FrameGetRequest {
                session: session.clone(),
                target: FrameTarget::Candidate(change.clone()),
                cancellation: cancellation.clone(),
            })
            .await
            .map_err(|error| match error {
                EngineError::EvidenceBudgetExhausted => FrameRefusal::NoRoom,
                error => FrameRefusal::Failed(error),
            })?;
        let file = results
            .files()
            .first()
            .ok_or(FrameRefusal::Failed(EngineError::Watch(
                WatchFailure::FrameMismatch,
            )))?;
        Ok(SessionFrame {
            evidence_id: file.evidence_id().clone(),
            path: file.path().to_path_buf(),
            sha256: file.sha256().as_str().to_owned(),
        })
    }

    /// What every output names about the run that made it.
    fn watch_provenance(
        &self,
        file_name: &OsStr,
        source_id: &SourceId,
        session: &SessionId,
    ) -> Result<WatchProvenance, EngineError> {
        Ok(WatchProvenance {
            video_name: render_hidden_characters(&file_name.to_string_lossy()),
            made_by: format!("VSift {}", env!("CARGO_PKG_VERSION")),
            made_at: self.watch_made_at()?,
            video_sha256: source_id
                .as_str()
                .trim_start_matches("src_sha256_")
                .to_owned(),
            session_id: session.as_str().to_owned(),
        })
    }

    /// When the run finished, in UTC to the minute: `2026-11-20 14:05 UTC`.
    fn watch_made_at(&self) -> Result<String, EngineError> {
        let seconds = self
            .clock()
            .now_unix_seconds()
            .map_err(EngineError::Clock)?;
        let rfc3339 = vsift_contract::utc_seconds_rfc3339(seconds).unwrap_or_default();
        Ok(match (rfc3339.get(..10), rfc3339.get(11..16)) {
            (Some(date), Some(time)) => format!("{date} {time} UTC"),
            _ => "at a time after 9999".to_owned(),
        })
    }
}

/// The engine's operations as the `watch` use case composes them, for one
/// run: they carry its cancellation and progress observer.
struct EngineWatchSteps<'a> {
    engine: &'a Engine,
    cancellation: &'a Cancellation,
    progress: &'a ProgressObserver,
}

impl WatchSteps for EngineWatchSteps<'_> {
    type Error = EngineError;

    async fn check_source(&self, video: &Path) -> Result<(), EngineError> {
        if std::fs::symlink_metadata(video).is_ok_and(|metadata| metadata.is_file()) {
            return Ok(());
        }
        // Nothing there, a folder or a link: `ingest`'s own refusal names the
        // mistake before anything else is checked or claimed (ADR 0037
        // section 2, the planner's condition C).
        let opened = self.engine.watch_ingest(video, self.cancellation).await?;
        drop(self.engine.close_session(&opened.session_id));
        Ok(())
    }

    fn check_capabilities(&self) -> Result<(), EngineError> {
        self.engine.watch_preflight()
    }

    async fn open_session(&self, video: &Path) -> Result<OpenedWatchSession, EngineError> {
        let opened = self.engine.watch_ingest(video, self.cancellation).await?;
        Ok(OpenedWatchSession {
            session_id: opened.session_id,
            source_id: opened.source_id,
        })
    }

    async fn prepare(&self, session: &SessionId) -> Result<PreparedRun, EngineError> {
        Box::pin(
            self.engine
                .watch_prepare(session, self.cancellation, self.progress),
        )
        .await
    }

    fn transcript_lines(
        &self,
        session: &SessionId,
        revision: &TranscriptRevisionId,
        range: TimeRange,
    ) -> Result<Vec<TranscriptLine>, EngineError> {
        self.engine.watch_transcript(session, revision, range)
    }

    async fn screen_changes(
        &self,
        session: &SessionId,
    ) -> Result<Option<ScreenChanges>, EngineError> {
        self.engine.watch_changes(session, self.cancellation).await
    }

    fn evidence_slots_left(&self, session: &SessionId) -> Result<usize, EngineError> {
        self.engine.evidence_slots_left(session)
    }

    async fn take_frame(
        &self,
        session: &SessionId,
        change: &VisualCandidateId,
    ) -> Result<SessionFrame, FrameRefusal<EngineError>> {
        self.engine
            .watch_frame(session, change, self.cancellation)
            .await
    }

    fn provenance(
        &self,
        video_name: &OsStr,
        source: &SourceId,
        session: &SessionId,
    ) -> Result<WatchProvenance, EngineError> {
        self.engine.watch_provenance(video_name, source, session)
    }

    fn close_session(&self, session: &SessionId) -> bool {
        self.engine.close_session(session).is_ok()
    }
}

/// `path` made absolute against the working directory, without resolving
/// links (the source's own checks refuse a link).
fn absolute(path: &Path) -> Result<PathBuf, EngineError> {
    std::path::absolute(path).map_err(|_| EngineError::WorkingDirectoryUnavailable)
}
