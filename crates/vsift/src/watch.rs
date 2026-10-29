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
//! The adapter tells the use case's [`WatchProgress`] about the `prepare`
//! job from the job's own progress: after each chunk of speech, and when the
//! visual stream has indexed the whole video. The job's observer only queues
//! the notice; the adapter hands it on between the job's steps, on the same
//! task, so nothing the use case does runs inside the job's call.

use std::{
    ffi::OsStr,
    fmt,
    path::{Path, PathBuf},
};

use vsift_application::{
    ClaimedOutputs, Clock, FrameRefusal, JobStore as _, OpenedWatchSession, PrepareStop,
    PreparedRun, PreparedTranscript, PublishedPart, ScreenChange, ScreenChanges, SessionFrame,
    TranscriptLine, UnfinishedWatch, WatchClaimLog as _, WatchError, WatchOutputError,
    WatchProgress, WatchProvenance, WatchSteps, run_watch,
};
use vsift_contract::render_hidden_characters;
use vsift_domain::{
    JobId, JobMode, MAX_VISUAL_INDEX_DURATION_MICROS, MediaTime, ProgressStage, SearchCoverage,
    SessionId, SessionPhase, SourceId, TimeRange, TranscriptRevisionId, VisualCandidateId,
};
use vsift_infrastructure::{FolderWatchOutputs, ManagedArtifactStore, file_sha256};

pub use vsift_application::{WatchFile, WatchFrame, WatchOutcome, WatchOutputKind};
pub use vsift_domain::{OutputNameError, WatchOutputNames};

use crate::{
    AdmissionWait, Cancellation, CandidateOmission, CandidatesRange, CandidatesRequest,
    DurabilityRequirement, FrameGetRequest, FrameTarget, IngestRequest, JobProgress,
    JobResumeRequest, JobStartRequest, ProgressObserver, RetranscribePublication, SessionListEntry,
    TranscriptQuery,
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
    /// Stops the run at its next provider boundary. The files keep their
    /// last snapshot and the session is kept, so the same request continues
    /// the job ([`WatchFailure::Stopped`]).
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
    /// Another program held a file open for the final write's 30 s (known
    /// limit L-237); the file keeps its last snapshot and marker.
    OutputHeld(WatchOutputKind),
    /// The caller stopped the run (Ctrl-C). The files keep their last
    /// snapshot and marker, and the session is kept: the same request
    /// continues the job (ADR 0037 section 8).
    Stopped {
        /// How far the transcript had been published, in microseconds.
        transcribed_to_us: u64,
        /// The video's length in microseconds, once a part had said it.
        duration_us: Option<u64>,
    },
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
            Self::OutputRefused(_) | Self::OutputHeld(_) => vsift_domain::FailureCode::StorageIo,
            Self::FrameMismatch => vsift_domain::FailureCode::IntegrityFailure,
            Self::Stopped { .. } => vsift_domain::FailureCode::Cancelled,
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
            Self::OutputHeld(_) => formatter.write_str("another program held an output open"),
            Self::Stopped { .. } => {
                formatter.write_str("the run was stopped; run it again to continue")
            }
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
            WatchOutputError::Held(kind) => Self::OutputHeld(kind),
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
    /// nothing that exists is ever replaced, except the files of an
    /// interrupted run of the same video, which its job's record names and
    /// this run continues. A failure after the claim releases it (removing
    /// only what this run created and never a written file) and closes the
    /// session; a cancellation keeps both.
    ///
    /// # Errors
    ///
    /// [`EngineError::Watch`] for the outputs and for a run the caller
    /// stopped ([`WatchFailure::Stopped`]), `MISSING_CAPABILITY` errors
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
                WatchError::Stopped {
                    transcribed_to,
                    duration,
                } => EngineError::Watch(WatchFailure::Stopped {
                    transcribed_to_us: transcribed_to.as_micros(),
                    duration_us: duration.map(MediaTime::as_micros),
                }),
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

    /// Starts the session's `prepare` job in the watched mode (ADR 0037
    /// section 3, step 4).
    fn watch_start(&self, session: &SessionId) -> Result<JobId, EngineError> {
        let plan = self.job_start(&JobStartRequest {
            session: session.clone(),
            mode: Some(JobMode::Watched),
            first: None,
        })?;
        Ok(plan.job_id().clone())
    }

    /// Keeps the names the run claimed in the job's own record.
    fn watch_record_claim(
        &self,
        session: &SessionId,
        job: &JobId,
        claimed: &ClaimedOutputs,
    ) -> Result<(), EngineError> {
        let (store, _) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(
            crate::error::SessionRootError::Missing,
        ))?;
        Ok(store.record_watch_claim(session, job, claimed)?)
    }

    /// Runs the `prepare` job in this process to its end; a new job's first
    /// run and an interrupted job's continuation alike (`job resume` reuses
    /// every finished chunk).
    async fn watch_run(
        &self,
        job_id: JobId,
        cancellation: &Cancellation,
        progress: &ProgressObserver,
    ) -> Result<PreparedRun, EngineError> {
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

    /// What the session's open revision has published: all of it, or what
    /// it published after `since` (a `--since` read). `None` before its first
    /// part.
    fn watch_published(
        &self,
        session: &SessionId,
        since: Option<&str>,
    ) -> Result<Option<PublishedPart>, EngineError> {
        let (revision, range) = if since.is_some() {
            // A `--since` read follows the token's revision, clipped to the
            // source.
            (None, (0, u64::MAX))
        } else {
            let (store, now) = self.existing_store()?;
            let Some(store) = store else {
                return Ok(None);
            };
            let Some((revision, _)) = store.read_transcript(session, now)? else {
                return Ok(None);
            };
            let range = revision.source_segment().range();
            (
                Some(revision.id().clone()),
                (range.start().as_micros(), range.end().as_micros()),
            )
        };
        let mut lines = Vec::new();
        let mut cursor = None;
        loop {
            let page = self.transcript(TranscriptQuery {
                session: session.clone(),
                revision: revision.clone(),
                from_micros: range.0,
                to_micros: range.1,
                limit: Some(READ_PAGE),
                cursor: cursor.take(),
                since: since.map(str::to_owned),
            })?;
            lines.extend(page.segments().iter().map(|segment| TranscriptLine {
                start: segment.range().start(),
                end: segment.range().end(),
                text: render_hidden_characters(segment.text().text()),
            }));
            if let Some(next) = page.next_cursor() {
                cursor = Some(next.to_owned());
                continue;
            }
            let read = page.revision();
            let duration = read.source_segment().range().end();
            return Ok(Some(PublishedPart {
                revision: read.id().clone(),
                lines,
                published_until: read.published_until().unwrap_or(duration),
                duration,
                since: page.growth().since().map(|(token, _, _)| token.to_owned()),
            }));
        }
    }

    /// An open session of `video` whose unfinished `prepare` job claimed
    /// exactly `claimed`, by the job's own record (ADR 0037 section 8). The
    /// records are compared first, so the video is hashed only when one
    /// matches.
    fn watch_unfinished(
        &self,
        video: &Path,
        claimed: &ClaimedOutputs,
    ) -> Result<Option<UnfinishedWatch>, EngineError> {
        let (store, now) = self.existing_store()?;
        let Some(store) = store else {
            return Ok(None);
        };
        let mut video_source = None;
        let mut cursor = None;
        loop {
            let page = self.list_sessions(cursor)?;
            for entry in page.entries() {
                let SessionListEntry::Indexed(snapshot) = entry else {
                    continue;
                };
                if snapshot.phase() != SessionPhase::Open || snapshot.lifetime().expired(now) {
                    continue;
                }
                let session = snapshot.session_id();
                let Ok(claims) = store.watch_claims(session) else {
                    continue;
                };
                for (job, recorded) in claims {
                    let unfinished = store
                        .job(session, &job)
                        .ok()
                        .flatten()
                        .is_some_and(|view| !view.record.state.is_terminal());
                    if recorded != *claimed || !unfinished {
                        continue;
                    }
                    if video_source.is_none() {
                        let digest = file_sha256(video)
                            .map_err(|_| EngineError::Watch(WatchFailure::OutputFolderUnusable))?;
                        video_source = SourceId::from_sha256(&digest).ok();
                    }
                    if video_source.as_ref() == Some(snapshot.source_id()) {
                        return Ok(Some(UnfinishedWatch {
                            session: OpenedWatchSession {
                                session_id: session.clone(),
                                source_id: snapshot.source_id().clone(),
                            },
                            job,
                        }));
                    }
                }
            }
            match page.next_cursor() {
                Some(next) => cursor = Some(next),
                None => return Ok(None),
            }
        }
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

    fn start_job(&self, session: &SessionId) -> Result<JobId, EngineError> {
        self.engine.watch_start(session)
    }

    fn record_claim(
        &self,
        session: &SessionId,
        job: &JobId,
        claimed: &ClaimedOutputs,
    ) -> Result<(), EngineError> {
        self.engine.watch_record_claim(session, job, claimed)
    }

    async fn prepare(
        &self,
        _session: &SessionId,
        job: &JobId,
        progress: &dyn WatchProgress,
    ) -> Result<PreparedRun, PrepareStop<EngineError>> {
        // The job's observer must outlive the call, so it only queues the
        // notice; this task hands each on between the job's steps.
        let (notices, mut received) = tokio::sync::mpsc::unbounded_channel();
        let observer = self.progress.also(move |update: &JobProgress| {
            let notice = match update.update.stage {
                ProgressStage::RecognisingSpeech => Some(Notice::Published),
                ProgressStage::AnalysingVideo
                    if update.update.total == Some(update.update.completed) =>
                {
                    Some(Notice::Indexed)
                }
                _ => None,
            };
            if let Some(notice) = notice {
                // The receiver lives as long as the job: a send never fails
                // while it runs, and a notice after it ended has no reader.
                let _unread = notices.send(notice);
            }
        });
        let run = self
            .engine
            .watch_run(job.clone(), self.cancellation, &observer);
        let mut run = Box::pin(run);
        let ran = loop {
            tokio::select! {
                ran = &mut run => break ran,
                Some(notice) = received.recv() => notice.tell(progress),
            }
        };
        while let Ok(notice) = received.try_recv() {
            notice.tell(progress);
        }
        ran.map_err(|error| {
            if self.cancellation.is_cancelled()
                && error.failure_code() == vsift_domain::FailureCode::Cancelled
            {
                PrepareStop::Interrupted
            } else {
                PrepareStop::Failed(error)
            }
        })
    }

    fn published(
        &self,
        session: &SessionId,
        since: Option<&str>,
    ) -> Result<Option<PublishedPart>, EngineError> {
        self.engine.watch_published(session, since)
    }

    fn find_unfinished(
        &self,
        video: &Path,
        claimed: &ClaimedOutputs,
    ) -> impl Future<Output = Result<Option<UnfinishedWatch>, EngineError>> + Send {
        std::future::ready(self.engine.watch_unfinished(video, claimed))
    }

    fn clock(&self) -> &dyn Clock {
        self.engine.clock()
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
        self.engine.evidence_slots_if_reused(session)
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

/// What the `prepare` job's progress tells the use case.
#[derive(Clone, Copy, Debug)]
enum Notice {
    /// A chunk of speech finished.
    Published,
    /// The visual stream indexed the whole video.
    Indexed,
}

impl Notice {
    fn tell(self, progress: &dyn WatchProgress) {
        match self {
            Self::Published => progress.published(),
            Self::Indexed => progress.visual_indexed(),
        }
    }
}

/// `path` made absolute against the working directory, without resolving
/// links (the source's own checks refuse a link).
fn absolute(path: &Path) -> Result<PathBuf, EngineError> {
    std::path::absolute(path).map_err(|_| EngineError::WorkingDirectoryUnavailable)
}
