//! `vsift <video>`, explicitly `vsift watch <video>` (ADR 0037): one action
//! that turns a video into a readable transcript, subtitles and its screen
//! changes with their frames, written beside it or into a folder the caller
//! names.
//!
//! [`Engine::watch`] is a composition of the engine's own operations, never a
//! second pipeline: it checks what it needs without running a tool, claims
//! the output names, ingests the video, runs the session's `prepare` job in
//! the watched mode in this process (ADR 0033), reads the sealed transcript
//! and the screen changes, copies the frames out of the session, writes the
//! files and closes the session.
//!
//! The first pull request (ADR 0037 section 13) writes every file once, when
//! the job has ended, and writes the frames after the job; the files that
//! grow while it runs, frames written during the transcription and continuing
//! an interrupted run come with the second.

use std::{
    ffi::OsStr,
    fmt,
    path::{Path, PathBuf},
};

use vsift_contract::render_hidden_characters;
use vsift_domain::{
    JobMode, MAX_VISUAL_INDEX_DURATION_MICROS, MAX_WATCH_FRAMES, MediaTime, OutputNameError,
    SearchCoverage, SessionId, TimeRange, TranscriptRevision, TranscriptRevisionId,
    VisualCandidate, VisualCandidateId, VisualIndexId, WatchOutputNames, frame_file_name,
    spread_evenly,
};
use vsift_infrastructure::{
    ManagedArtifactStore, ScreenChangeEntry, ScreenChangeSummary, TranscriptEntry, WatchClaim,
    WatchOutputError, WatchProvenance, WrittenFrame, render_screen_changes, render_subrip,
    render_transcript,
};

pub use vsift_infrastructure::WatchOutputKind;

use crate::{
    AdmissionWait, Cancellation, CandidateOmission, CandidatesRange, CandidatesRequest,
    DurabilityRequirement, EvidenceId, FrameGetRequest, FrameTarget, IngestRequest,
    JobResumeRequest, JobStartRequest, ProgressObserver, RetranscribePublication, TranscriptQuery,
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
        }
    }
}

/// A file `watch` wrote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchFile {
    /// Its absolute path.
    pub path: PathBuf,
}

/// A frame `watch` wrote for a screen change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchFrame {
    /// The change's representative time.
    pub time: MediaTime,
    /// The visual candidate.
    pub candidate_id: VisualCandidateId,
    /// The frame's evidence item, in the session the run closed.
    pub evidence_id: EvidenceId,
    /// The frame file's absolute path.
    pub path: PathBuf,
    /// The PNG's SHA-256, 64 lowercase hexadecimal digits.
    pub sha256: String,
}

/// What one `watch` run wrote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchOutcome {
    /// The video's content identity (`src_sha256_` and its SHA-256).
    pub source_id: vsift_domain::SourceId,
    /// The session the run used, closed when it finished.
    pub session_id: SessionId,
    /// Whether the session was closed.
    pub session_closed: bool,
    /// The `prepare` job.
    pub job_id: vsift_domain::JobId,
    /// The transcript revision; `None` for a video with no sound track.
    pub transcript_revision: Option<TranscriptRevisionId>,
    /// The visual index revision; `None` when the video has no picture.
    pub visual_index: Option<VisualIndexId>,
    /// The video's length.
    pub duration: MediaTime,
    /// The ranges that were not transcribed, in time order.
    pub not_transcribed: Vec<TimeRange>,
    /// The readable transcript.
    pub transcript: WatchFile,
    /// The subtitles; `None` for a video with no sound track.
    pub subtitles: Option<WatchFile>,
    /// The screen-change folder.
    pub folder: WatchFile,
    /// The screen-change list in the folder.
    pub screen_changes: WatchFile,
    /// How many screen changes were found.
    pub changes_found: usize,
    /// The frames written, at most [`MAX_WATCH_FRAMES`].
    pub frames: Vec<WatchFrame>,
    /// Whether the video runs past the part the visual index covers.
    pub beyond_visual_bound: bool,
    /// Whether the names were shortened, so the subtitles no longer share
    /// the video's name.
    pub names_shortened: bool,
}

impl Engine {
    /// Watches one video (ADR 0037): writes `<stem>.txt`, `<stem>.srt` and
    /// `<stem>-vsift/` beside it or into `request.output`.
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
        self.watch_preflight()?;
        let source = absolute(&request.source)?;
        let file_name = source
            .file_name()
            .ok_or(EngineError::Watch(WatchFailure::UnusableName(
                OutputNameError::Empty,
            )))?
            .to_os_string();
        let names = WatchOutputNames::from_video_name(&file_name)
            .map_err(|error| EngineError::Watch(WatchFailure::UnusableName(error)))?;
        let folder = self.output_folder(&source, request.output.as_deref())?;
        let mut claim =
            WatchClaim::claim(&folder, &names).map_err(|error| EngineError::Watch(error.into()))?;
        let ingested = self
            .ingest(IngestRequest {
                source,
                transcript: None,
                cancellation: request.cancellation.clone(),
                durability: DurabilityRequirement::Ephemeral,
            })
            .await;
        let session = match ingested {
            Ok(outcome) => outcome.session,
            Err(error) => {
                claim.release();
                return Err(error);
            }
        };
        let session_id = session.session_id.clone();
        let source_id = session.source_id.clone();
        let written = Box::pin(self.watch_session(
            &request,
            &session_id,
            &source_id,
            &file_name,
            &names,
            &mut claim,
        ))
        .await;
        let closed = self.close_session(&session_id).is_ok();
        match written {
            Ok(mut outcome) => {
                outcome.session_closed = closed;
                Ok(outcome)
            }
            Err(error) => {
                claim.release();
                Err(error)
            }
        }
    }

    async fn watch_session(
        &self,
        request: &WatchRequest,
        session: &SessionId,
        source_id: &vsift_domain::SourceId,
        file_name: &OsStr,
        names: &WatchOutputNames,
        claim: &mut WatchClaim,
    ) -> Result<WatchOutcome, EngineError> {
        let plan = self.job_start(&JobStartRequest {
            session: session.clone(),
            mode: Some(JobMode::Watched),
            first: None,
        })?;
        let job_id = plan.job_id().clone();
        let resumed = self
            .job_resume(JobResumeRequest {
                job: job_id.clone(),
                cancellation: request.cancellation.clone(),
                progress: request.progress.clone(),
                admission: AdmissionWait::Immediate,
                publication: RetranscribePublication::AsItGoes {
                    interval: crate::PUBLICATION_INTERVAL,
                },
            })
            .await;
        let revision = match resumed {
            Ok(report) => Some(report.outcome().revision().clone()),
            Err(EngineError::NoAudioStream) => None,
            Err(error) => return Err(error),
        };
        let SealedTranscript {
            entries,
            gaps: not_transcribed,
            duration,
        } = match &revision {
            Some(revision) => self.watch_transcript(session, revision)?,
            None => SealedTranscript::default(),
        };
        let (changes, index) = self.watch_changes(session, &request.cancellation).await?;
        let duration = duration
            .or_else(|| index.as_ref().map(|(_, end)| *end))
            .unwrap_or(MediaTime::from_micros(0));
        let beyond_visual_bound = duration.as_micros() > MAX_VISUAL_INDEX_DURATION_MICROS;
        let (frames, listed) = self
            .watch_frames(session, &changes, claim, &request.cancellation)
            .await?;
        let provenance = WatchProvenance {
            video_name: render_hidden_characters(&file_name.to_string_lossy()),
            made_by: format!("VSift {}", env!("CARGO_PKG_VERSION")),
            made_at: self.watch_made_at()?,
            video_sha256: source_id
                .as_str()
                .trim_start_matches("src_sha256_")
                .to_owned(),
            session_id: session.as_str().to_owned(),
        };
        let revision_id = revision.as_ref().map(TranscriptRevision::id);
        claim
            .write_transcript(&render_transcript(
                &provenance,
                revision_id.map(TranscriptRevisionId::as_str),
                &entries,
            ))
            .map_err(watch_error)?;
        let subtitles = if revision.is_some() {
            claim
                .write_subtitles(&render_subrip(&entries))
                .map_err(watch_error)?;
            Some(WatchFile {
                path: claim.subtitles_path().to_path_buf(),
            })
        } else {
            claim.discard_subtitles();
            None
        };
        let summary = ScreenChangeSummary {
            index_id: index
                .as_ref()
                .map_or_else(|| "none".to_owned(), |(id, _)| id.as_str().to_owned()),
            beyond_index_bound: beyond_visual_bound,
        };
        let list = claim
            .write_screen_changes(&render_screen_changes(&provenance, &summary, &listed))
            .map_err(watch_error)?;
        Ok(WatchOutcome {
            source_id: source_id.clone(),
            session_id: session.clone(),
            session_closed: false,
            job_id,
            transcript_revision: revision_id.cloned(),
            visual_index: index.map(|(id, _)| id),
            duration,
            not_transcribed,
            transcript: WatchFile {
                path: claim.transcript_path().to_path_buf(),
            },
            subtitles,
            folder: WatchFile {
                path: claim.folder_path().to_path_buf(),
            },
            screen_changes: WatchFile { path: list },
            changes_found: changes.len(),
            frames,
            beyond_visual_bound,
            names_shortened: names.shortened(),
        })
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

    /// The folder to write into: the video's own, or `output` (an existing
    /// folder, or a new one whose parent exists), never inside `VSift`'s
    /// session or managed root.
    fn output_folder(&self, source: &Path, output: Option<&Path>) -> Result<PathBuf, EngineError> {
        let unusable = EngineError::Watch(WatchFailure::OutputFolderUnusable);
        let folder = match output {
            None => source.parent().map(Path::to_path_buf).ok_or(unusable)?,
            Some(output) => {
                let output = absolute(output)?;
                match std::fs::symlink_metadata(&output) {
                    Ok(metadata) if metadata.is_dir() => output,
                    Ok(_) => return Err(unusable),
                    Err(_) => {
                        std::fs::create_dir(&output)
                            .map_err(|_| EngineError::Watch(WatchFailure::OutputFolderUnusable))?;
                        output
                    }
                }
            }
        };
        let folder = std::fs::canonicalize(&folder)
            .map_err(|_| EngineError::Watch(WatchFailure::OutputFolderUnusable))?;
        for root in self.vsift_roots() {
            if folder.starts_with(&root) {
                return Err(EngineError::Watch(WatchFailure::OutputInsideVsift));
            }
        }
        Ok(folder)
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

    /// The sealed transcript as entries in time order, its untranscribed
    /// ranges and the video's length.
    fn watch_transcript(
        &self,
        session: &SessionId,
        revision: &TranscriptRevision,
    ) -> Result<SealedTranscript, EngineError> {
        let source = revision.source_segment().range();
        let mut entries = Vec::new();
        let mut cursor = None;
        loop {
            let page = self.transcript(TranscriptQuery {
                session: session.clone(),
                revision: Some(revision.id().clone()),
                from_micros: source.start().as_micros(),
                to_micros: source.end().as_micros(),
                limit: Some(READ_PAGE),
                cursor: cursor.take(),
                since: None,
            })?;
            entries.extend(page.segments().iter().map(|segment| TranscriptEntry::Line {
                start: segment.range().start(),
                end: segment.range().end(),
                text: render_hidden_characters(segment.text().text()),
            }));
            match page.next_cursor() {
                Some(next) => cursor = Some(next.to_owned()),
                None => break,
            }
        }
        let coverage = SearchCoverage::of(revision, None);
        let gaps = coverage.untranscribed().to_vec();
        entries.extend(gaps.iter().map(|gap| TranscriptEntry::Gap {
            start: gap.start(),
            end: gap.end(),
            reason: "the speech tool could not transcribe this part".to_owned(),
        }));
        entries.sort_by_key(|entry| match entry {
            TranscriptEntry::Line { start, .. } | TranscriptEntry::Gap { start, .. } => *start,
        });
        Ok(SealedTranscript {
            entries,
            gaps,
            duration: Some(source.end()),
        })
    }

    /// Every screen change of the analysed video, and the index revision with
    /// the end of the range it covers; nothing when the video has no picture.
    async fn watch_changes(
        &self,
        session: &SessionId,
        cancellation: &Cancellation,
    ) -> Result<(Vec<VisualCandidate>, Option<(VisualIndexId, MediaTime)>), EngineError> {
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
                    return Ok((Vec::new(), None));
                }
                Err(error) => return Err(error),
            };
            index = Some((page.index().id().clone(), page.range().end()));
            changes.extend(page.candidates().iter().cloned());
            match page.next_cursor() {
                Some(next) => cursor = Some(next.to_owned()),
                None => break,
            }
        }
        changes.sort_by_key(VisualCandidate::representative);
        Ok((changes, index))
    }

    /// The frames of the selected changes (at most [`MAX_WATCH_FRAMES`],
    /// spread evenly), and every change as the list shows it.
    async fn watch_frames(
        &self,
        session: &SessionId,
        changes: &[VisualCandidate],
        claim: &mut WatchClaim,
        cancellation: &Cancellation,
    ) -> Result<(Vec<WatchFrame>, Vec<ScreenChangeEntry>), EngineError> {
        let selected = spread_evenly(changes.len(), MAX_WATCH_FRAMES);
        let mut frames = Vec::with_capacity(selected.len());
        let mut listed: Vec<ScreenChangeEntry> = changes
            .iter()
            .map(|change| ScreenChangeEntry {
                time: change.representative(),
                candidate_id: change.id().as_str().to_owned(),
                frame: None,
            })
            .collect();
        for position in selected {
            let Some(change) = changes.get(position) else {
                continue;
            };
            let frame = self
                .watch_frame(session, change, claim, cancellation)
                .await?;
            if let Some(entry) = listed.get_mut(position) {
                entry.frame = Some(WrittenFrame {
                    file_name: frame_file_name(frame.time),
                    evidence_id: frame.evidence_id.as_str().to_owned(),
                    sha256: frame.sha256.clone(),
                });
            }
            frames.push(frame);
        }
        Ok((frames, listed))
    }

    /// One change's frame, copied out of the session into the folder and
    /// checked against its digest.
    async fn watch_frame(
        &self,
        session: &SessionId,
        change: &VisualCandidate,
        claim: &mut WatchClaim,
        cancellation: &Cancellation,
    ) -> Result<WatchFrame, EngineError> {
        let results = self
            .frame_get(FrameGetRequest {
                session: session.clone(),
                target: FrameTarget::Candidate(change.id().clone()),
                cancellation: cancellation.clone(),
            })
            .await?;
        let file = results
            .files()
            .first()
            .ok_or(EngineError::Watch(WatchFailure::FrameMismatch))?;
        let time = change.representative();
        let sha256 = file.sha256().as_str().to_owned();
        let path = claim
            .copy_frame(file.path(), &frame_file_name(time), &sha256)
            .map_err(watch_error)?;
        Ok(WatchFrame {
            time,
            candidate_id: change.id().clone(),
            evidence_id: file.evidence_id().clone(),
            path,
            sha256,
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

/// The sealed transcript as `watch` writes it.
#[derive(Default)]
struct SealedTranscript {
    /// Segments and gaps, in time order.
    entries: Vec<TranscriptEntry>,
    /// The ranges that were not transcribed.
    gaps: Vec<TimeRange>,
    /// The video's length, from the transcript's source segment.
    duration: Option<MediaTime>,
}

fn watch_error(error: WatchOutputError) -> EngineError {
    EngineError::Watch(error.into())
}

/// `path` made absolute against the working directory, without resolving
/// links (the source's own checks refuse a link).
fn absolute(path: &Path) -> Result<PathBuf, EngineError> {
    std::path::absolute(path).map_err(|_| EngineError::WorkingDirectoryUnavailable)
}
