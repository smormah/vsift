//! `vsift <video>`, explicitly `vsift watch <video>` (ADR 0037 section 3):
//! the use case that turns a video into a readable transcript, subtitles and
//! its screen changes with their frames, in a folder the person can read.
//!
//! [`run_watch`] orchestrates; it adds no pipeline. Every step it takes is an
//! existing operation reached through [`WatchSteps`], which the engine
//! supplies over its own session, job, transcript, candidate and frame use
//! cases (as [`PrepareStreams`](crate::PrepareStreams) supplies the `prepare`
//! job's streams). The one new effect, writing into the person's folder, is
//! the [`WatchOutputs`] port, whose folder implementation lives in the
//! infrastructure.
//!
//! **The order is the safety rule.** The source and the capabilities are
//! checked and every output name is claimed before anything is copied, so a
//! refusal before the work leaves nothing written and no session. After the
//! claim, a failure releases it (the port removes only what it created, never
//! a written file) and the session is closed either way.
//!
//! **The list holds the settled screens** ([`settled_screens`]): every change
//! found is counted, and the first frame, each screen after motion settles
//! and a change at least 5 s after the last listed one are listed.
//!
//! **Frames fit the session's room.** The session's evidence room is measured
//! before the first frame and at most [`frames_that_fit`] frames are asked
//! for, spread evenly over the changes; should the session still refuse a
//! frame for room, the frames stop there and the run is partial, never failed
//! (ADR 0037 section 6, known limit L-234).

use std::{
    ffi::OsStr,
    fmt, io,
    path::{Path, PathBuf},
};

use vsift_domain::{
    CandidateReason, EvidenceId, JobId, MAX_VISUAL_INDEX_DURATION_MICROS, MediaTime,
    OutputNameError, SessionId, SourceId, TimeRange, TranscriptRevisionId, VisualCandidateId,
    VisualIndexId, WatchOutputNames, frame_file_name, frames_that_fit, settled_screens,
    spread_evenly,
};

/// Why a stated gap was not transcribed, in plain words (ADR 0037 section 4).
const GAP_REASON: &str = "the speech tool could not transcribe this part";

/// The session `watch` opened for the video.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenedWatchSession {
    /// The session.
    pub session_id: SessionId,
    /// The video's content identity (`src_sha256_` and its SHA-256).
    pub source_id: SourceId,
}

/// The sealed transcript of a `prepare` job, as `watch` needs it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedTranscript {
    /// The sealed revision.
    pub revision: TranscriptRevisionId,
    /// The range of the video the revision covers: the whole video.
    pub range: TimeRange,
    /// The ranges that were not transcribed, in time order.
    pub untranscribed: Vec<TimeRange>,
}

/// How the `prepare` job ended for `watch`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedRun {
    /// The `prepare` job.
    pub job_id: JobId,
    /// Its sealed transcript; `None` for a video with no sound track.
    pub transcript: Option<PreparedTranscript>,
}

/// One transcript segment, its text already in display form (hidden and
/// control characters made visible: transcript text is untrusted).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptLine {
    /// Start of the segment.
    pub start: MediaTime,
    /// End of the segment.
    pub end: MediaTime,
    /// The segment's text in display form.
    pub text: String,
}

/// One screen change the visual stream found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenChange {
    /// The visual candidate.
    pub id: VisualCandidateId,
    /// Its representative time.
    pub time: MediaTime,
    /// Why the visual stream proposed it.
    pub reason: CandidateReason,
}

/// Every screen change of the analysed video.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenChanges {
    /// The visual index revision.
    pub index: VisualIndexId,
    /// The end of the range the index covers.
    pub covered_to: MediaTime,
    /// The changes, in any order: `watch` sorts them by time.
    pub changes: Vec<ScreenChange>,
}

/// A frame stored in the session, ready to be copied out.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionFrame {
    /// The frame's evidence item.
    pub evidence_id: EvidenceId,
    /// The session's content-addressed file.
    pub path: PathBuf,
    /// The PNG's recorded SHA-256, 64 lowercase hexadecimal digits.
    pub sha256: String,
}

/// Why a frame was not taken.
#[derive(Debug)]
pub enum FrameRefusal<E> {
    /// The session has no evidence room left for it.
    NoRoom,
    /// Any other failure, which ends the run.
    Failed(E),
}

/// The existing operations `watch` composes, supplied by the engine.
///
/// The engine's implementation holds the run's cancellation and progress
/// observer; every method is one existing operation, never a rule of its
/// own.
pub trait WatchSteps: Send + Sync {
    /// The engine's error.
    type Error: Send;

    /// Accepts a video that is a regular file; anything else (nothing there,
    /// a folder, a link) answers with `ingest`'s own refusal, before any
    /// other check (ADR 0037 section 2, the planner's condition C).
    ///
    /// # Errors
    ///
    /// `ingest`'s refusal of the source.
    fn check_source(&self, video: &Path) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Video tools and speech recognition are configured, checked without
    /// running a tool (design note C9).
    ///
    /// # Errors
    ///
    /// The missing capability.
    fn check_capabilities(&self) -> Result<(), Self::Error>;

    /// Copies the video into a new private session, as `ingest` does.
    ///
    /// # Errors
    ///
    /// Every error of `ingest`.
    fn open_session(
        &self,
        video: &Path,
    ) -> impl Future<Output = Result<OpenedWatchSession, Self::Error>> + Send;

    /// Starts the session's `prepare` job in the watched mode and runs it in
    /// this process to its end (ADR 0033).
    ///
    /// # Errors
    ///
    /// Every error of the job; a video with no sound track is not one.
    fn prepare(
        &self,
        session: &SessionId,
    ) -> impl Future<Output = Result<PreparedRun, Self::Error>> + Send;

    /// Every segment of `revision` within `range`, in time order.
    ///
    /// # Errors
    ///
    /// Every error of the transcript read.
    fn transcript_lines(
        &self,
        session: &SessionId,
        revision: &TranscriptRevisionId,
        range: TimeRange,
    ) -> Result<Vec<TranscriptLine>, Self::Error>;

    /// Every screen change of the analysed video; `None` when the video has
    /// no picture.
    ///
    /// # Errors
    ///
    /// Every error of the candidate read.
    fn screen_changes(
        &self,
        session: &SessionId,
    ) -> impl Future<Output = Result<Option<ScreenChanges>, Self::Error>> + Send;

    /// How many evidence artifacts the session can still store.
    ///
    /// # Errors
    ///
    /// Storage failures.
    fn evidence_slots_left(&self, session: &SessionId) -> Result<usize, Self::Error>;

    /// One change's frame, stored in the session (a `frame get`).
    ///
    /// # Errors
    ///
    /// [`FrameRefusal::NoRoom`] when the session's evidence is full, and
    /// [`FrameRefusal::Failed`] for any other failure.
    fn take_frame(
        &self,
        session: &SessionId,
        change: &VisualCandidateId,
    ) -> impl Future<Output = Result<SessionFrame, FrameRefusal<Self::Error>>> + Send;

    /// What every output names about the run that made it.
    ///
    /// # Errors
    ///
    /// The clock's failure.
    fn provenance(
        &self,
        video_name: &OsStr,
        source: &SourceId,
        session: &SessionId,
    ) -> Result<WatchProvenance, Self::Error>;

    /// Closes the session; whether it closed.
    fn close_session(&self, session: &SessionId) -> bool;
}

/// Which output an error is about.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchOutputKind {
    /// The readable transcript, `<stem>.txt`.
    Transcript,
    /// The subtitles, `<stem>.srt`.
    Subtitles,
    /// The screen-change folder, `<stem>-vsift`.
    Folder,
    /// A frame or the screen-change list inside the folder.
    FolderContent,
}

/// Why an output could not be claimed or written.
#[derive(Debug)]
pub enum WatchOutputError {
    /// Something already exists at the output's name; nothing was replaced.
    Exists(WatchOutputKind),
    /// The folder refused the write (read-only, no room, a synced folder
    /// that rejects new files).
    Refused(WatchOutputKind, io::ErrorKind),
    /// A frame copied out of the session does not have its recorded SHA-256.
    FrameMismatch,
    /// `--output` names a folder whose parent does not exist, or something
    /// that is not a folder.
    FolderUnusable,
    /// The folder lies inside `VSift`'s own session or managed root, which
    /// `VSift` cleans automatically.
    InsideVsift,
}

impl fmt::Display for WatchOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exists(kind) => write!(formatter, "the {kind:?} output already exists"),
            Self::Refused(kind, error) => {
                write!(
                    formatter,
                    "the {kind:?} output could not be written: {error}"
                )
            }
            Self::FrameMismatch => formatter.write_str("a copied frame does not match its digest"),
            Self::FolderUnusable => formatter.write_str("the output folder cannot be used"),
            Self::InsideVsift => {
                formatter.write_str("the output folder is inside VSift's own folders")
            }
        }
    }
}

impl std::error::Error for WatchOutputError {}

/// The paths a claim holds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimedOutputs {
    /// The readable transcript.
    pub transcript: PathBuf,
    /// The subtitles.
    pub subtitles: PathBuf,
    /// The screen-change folder.
    pub folder: PathBuf,
}

/// Facts every output names about the run that made it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchProvenance {
    /// The video's file name, in display form.
    pub video_name: String,
    /// `VSift` and its version, for example `VSift 0.3.0`.
    pub made_by: String,
    /// When the run finished, in UTC to the minute (`2026-11-20 14:05 UTC`).
    pub made_at: String,
    /// The video's SHA-256, 64 lowercase hexadecimal digits.
    pub video_sha256: String,
    /// The session the run used.
    pub session_id: String,
}

/// One entry of the transcript, in time order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TranscriptEntry {
    /// A segment: its range and its text in display form.
    Line {
        /// Start of the segment.
        start: MediaTime,
        /// End of the segment.
        end: MediaTime,
        /// The segment's text, hidden characters made visible.
        text: String,
    },
    /// A range that was not transcribed, and why, in plain words.
    Gap {
        /// Start of the range.
        start: MediaTime,
        /// End of the range.
        end: MediaTime,
        /// Why it was not transcribed.
        reason: String,
    },
}

impl TranscriptEntry {
    const fn start(&self) -> MediaTime {
        match self {
            Self::Line { start, .. } | Self::Gap { start, .. } => *start,
        }
    }
}

/// One screen change in the list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenChangeEntry {
    /// The change's representative time.
    pub time: MediaTime,
    /// The visual candidate's id.
    pub candidate_id: String,
    /// Why the visual stream proposed it.
    pub reason: CandidateReason,
    /// The frame written for it, when one was.
    pub frame: Option<WrittenFrame>,
}

/// A frame written into the folder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrittenFrame {
    /// Its file name in the folder.
    pub file_name: String,
    /// The frame's evidence id.
    pub evidence_id: String,
    /// The PNG's SHA-256.
    pub sha256: String,
}

/// What the screen-change list says about the run as a whole.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenChangeSummary {
    /// The visual index revision's id.
    pub index_id: String,
    /// How many screen changes the visual stream found; the list holds the
    /// settled screens among them.
    pub changes_found: usize,
    /// The end of the range the visual index covers: the `--to` a person
    /// gives `candidates` to see every change.
    pub index_covers_to: MediaTime,
    /// Whether the video runs past the part the visual index covers.
    pub beyond_index_bound: bool,
    /// Whether the session ran out of evidence room before every selected
    /// change had its frame (known limit L-234).
    pub stopped_for_room: bool,
}

/// Everything the finished files hold, for the port to render and write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchDocuments {
    /// What every output names about the run.
    pub provenance: WatchProvenance,
    /// The transcript revision; `None` for a video with no sound track,
    /// which then has no subtitles.
    pub transcript_revision: Option<TranscriptRevisionId>,
    /// The transcript's segments and stated gaps, in time order.
    pub entries: Vec<TranscriptEntry>,
    /// The screen-change list's summary.
    pub summary: ScreenChangeSummary,
    /// The settled screens, in time order, with the frames written.
    pub changes: Vec<ScreenChangeEntry>,
}

/// The finished files the port wrote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalisedOutputs {
    /// The readable transcript.
    pub transcript: WatchFile,
    /// The subtitles; `None` for a video with no sound track.
    pub subtitles: Option<WatchFile>,
    /// The screen-change list in the folder.
    pub screen_changes: WatchFile,
}

/// Port that writes `watch`'s outputs into a person's folder (ADR 0037
/// sections 5 to 7).
///
/// The folder implementation claims each name by exclusive creation and never
/// replaces a file it did not create; every later write goes only to what the
/// claim created.
pub trait WatchOutputs: Send {
    /// Resolves the folder (the video's own, or `output`: an existing folder
    /// or a new one whose parent exists, never inside `VSift`'s own roots)
    /// and claims the three outputs there, before any work.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Exists`] for a taken name,
    /// [`WatchOutputError::Refused`] when the folder refuses,
    /// [`WatchOutputError::FolderUnusable`] or
    /// [`WatchOutputError::InsideVsift`] for the folder itself.
    fn claim(
        &mut self,
        video: &Path,
        output: Option<&Path>,
        names: &WatchOutputNames,
    ) -> Result<ClaimedOutputs, WatchOutputError>;

    /// Copies a frame out of the session into the claimed folder as
    /// `file_name`, checked against its recorded SHA-256; the copy's path.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::FrameMismatch`] when the copy's digest differs,
    /// and the claim's write errors.
    fn copy_frame(
        &mut self,
        frame: &SessionFrame,
        file_name: &str,
    ) -> Result<PathBuf, WatchOutputError>;

    /// Writes the finished files: the transcript, the subtitles (or removes
    /// their claim, still empty, for a video with no sound track) and the
    /// screen-change list.
    ///
    /// # Errors
    ///
    /// The claim's write errors.
    fn finalise(
        &mut self,
        documents: &WatchDocuments,
    ) -> Result<FinalisedOutputs, WatchOutputError>;

    /// Gives the claim up after a failure: removes what this run created and
    /// still holds no written text. Does nothing without a claim.
    fn release(&mut self);
}

/// A file `watch` wrote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchFile {
    /// Its absolute path.
    pub path: PathBuf,
    /// The SHA-256 of a file's bytes; `None` for the folder.
    pub sha256: Option<String>,
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
#[allow(clippy::struct_excessive_bools)] // Independent facts the run reports, not switches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchOutcome {
    /// The video's content identity (`src_sha256_` and its SHA-256).
    pub source_id: SourceId,
    /// The session the run used, closed when it finished.
    pub session_id: SessionId,
    /// Whether the session was closed.
    pub session_closed: bool,
    /// The `prepare` job.
    pub job_id: JobId,
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
    /// How many screen changes the visual stream found.
    pub changes_found: usize,
    /// How many of them the list shows: the settled screens
    /// ([`settled_screens`]).
    pub changes_listed: usize,
    /// The frames written, at most
    /// [`MAX_WATCH_FRAMES`](vsift_domain::MAX_WATCH_FRAMES).
    pub frames: Vec<WatchFrame>,
    /// Whether the video runs past the part the visual index covers.
    pub beyond_visual_bound: bool,
    /// Whether the names were shortened, so the subtitles no longer share
    /// the video's name.
    pub names_shortened: bool,
    /// Whether the session ran out of evidence room before every selected
    /// change had its frame: the run stopped taking frames, listed the rest
    /// without one, and is partial (known limit L-234).
    pub frames_stopped_for_room: bool,
}

/// Why a `watch` run failed.
#[derive(Debug)]
pub enum WatchError<E> {
    /// An existing operation failed with the engine's error.
    Step(E),
    /// No output names can be derived from the video's file name.
    UnusableName(OutputNameError),
    /// An output could not be claimed or written.
    Output(WatchOutputError),
}

/// Watches one `video` (ADR 0037 section 3): writes `<stem>.txt`,
/// `<stem>.srt` and `<stem>-vsift/` beside it or into `output`.
///
/// `video` and `output` are absolute. Nothing is copied or written before
/// every output name is claimed; after the claim, a failure releases it and
/// the session is closed either way.
///
/// # Errors
///
/// [`WatchError::Step`] for the operations' errors, and
/// [`WatchError::UnusableName`] or [`WatchError::Output`] for the outputs.
pub async fn run_watch<S: WatchSteps, O: WatchOutputs>(
    steps: &S,
    outputs: &mut O,
    video: &Path,
    output: Option<&Path>,
) -> Result<WatchOutcome, WatchError<S::Error>> {
    steps.check_source(video).await.map_err(WatchError::Step)?;
    steps.check_capabilities().map_err(WatchError::Step)?;
    let file_name = video
        .file_name()
        .ok_or(WatchError::UnusableName(OutputNameError::Empty))?;
    let names = WatchOutputNames::from_video_name(file_name).map_err(WatchError::UnusableName)?;
    let claimed = outputs
        .claim(video, output, &names)
        .map_err(WatchError::Output)?;
    let session = match steps.open_session(video).await {
        Ok(session) => session,
        Err(error) => {
            outputs.release();
            return Err(WatchError::Step(error));
        }
    };
    let run = WatchRun {
        steps,
        session: &session,
        file_name,
        names: &names,
        claimed,
    };
    let written = Box::pin(run.write(outputs)).await;
    let closed = steps.close_session(&session.session_id);
    match written {
        Ok(mut outcome) => {
            outcome.session_closed = closed;
            Ok(outcome)
        }
        Err(error) => {
            outputs.release();
            Err(error)
        }
    }
}

/// One run, from the open session to the finished files.
struct WatchRun<'a, S> {
    steps: &'a S,
    session: &'a OpenedWatchSession,
    file_name: &'a OsStr,
    names: &'a WatchOutputNames,
    claimed: ClaimedOutputs,
}

/// The frames a run took, and every change as the list shows it.
struct TakenFrames {
    frames: Vec<WatchFrame>,
    listed: Vec<ScreenChangeEntry>,
    stopped_for_room: bool,
}

impl<S: WatchSteps> WatchRun<'_, S> {
    async fn write<O: WatchOutputs>(
        self,
        outputs: &mut O,
    ) -> Result<WatchOutcome, WatchError<S::Error>> {
        let session = &self.session.session_id;
        let prepared = self
            .steps
            .prepare(session)
            .await
            .map_err(WatchError::Step)?;
        let entries = match &prepared.transcript {
            Some(transcript) => self.transcript_entries(transcript)?,
            None => Vec::new(),
        };
        let changes = self
            .steps
            .screen_changes(session)
            .await
            .map_err(WatchError::Step)?;
        let duration = prepared
            .transcript
            .as_ref()
            .map(|transcript| transcript.range.end())
            .or_else(|| changes.as_ref().map(|changes| changes.covered_to))
            .unwrap_or(MediaTime::from_micros(0));
        let beyond_visual_bound = duration.as_micros() > MAX_VISUAL_INDEX_DURATION_MICROS;
        let mut found = changes
            .as_ref()
            .map(|changes| changes.changes.clone())
            .unwrap_or_default();
        found.sort_by_key(|change| change.time);
        let reasons: Vec<(MediaTime, CandidateReason)> = found
            .iter()
            .map(|change| (change.time, change.reason))
            .collect();
        let settled: Vec<ScreenChange> = settled_screens(&reasons)
            .into_iter()
            .filter_map(|position| found.get(position).cloned())
            .collect();
        let taken = self.take_frames(outputs, &settled).await?;
        let provenance = self
            .steps
            .provenance(self.file_name, &self.session.source_id, session)
            .map_err(WatchError::Step)?;
        let transcript_revision = prepared
            .transcript
            .as_ref()
            .map(|transcript| transcript.revision.clone());
        let documents = WatchDocuments {
            provenance,
            transcript_revision: transcript_revision.clone(),
            entries,
            summary: ScreenChangeSummary {
                index_id: changes.as_ref().map_or_else(
                    || "none".to_owned(),
                    |changes| changes.index.as_str().to_owned(),
                ),
                changes_found: found.len(),
                index_covers_to: changes
                    .as_ref()
                    .map_or(MediaTime::from_micros(0), |changes| changes.covered_to),
                beyond_index_bound: beyond_visual_bound,
                stopped_for_room: taken.stopped_for_room,
            },
            changes: taken.listed,
        };
        let finalised = outputs.finalise(&documents).map_err(WatchError::Output)?;
        Ok(WatchOutcome {
            source_id: self.session.source_id.clone(),
            session_id: session.clone(),
            session_closed: false,
            job_id: prepared.job_id,
            transcript_revision,
            visual_index: changes.map(|changes| changes.index),
            duration,
            not_transcribed: prepared
                .transcript
                .map(|transcript| transcript.untranscribed)
                .unwrap_or_default(),
            transcript: finalised.transcript,
            subtitles: finalised.subtitles,
            folder: WatchFile {
                path: self.claimed.folder,
                sha256: None,
            },
            screen_changes: finalised.screen_changes,
            changes_found: found.len(),
            changes_listed: settled.len(),
            frames: taken.frames,
            beyond_visual_bound,
            names_shortened: self.names.shortened(),
            frames_stopped_for_room: taken.stopped_for_room,
        })
    }

    /// The sealed transcript's segments with its untranscribed ranges as
    /// stated gaps, in time order (ADR 0037 section 4: gaps are stated, not
    /// hidden).
    fn transcript_entries(
        &self,
        transcript: &PreparedTranscript,
    ) -> Result<Vec<TranscriptEntry>, WatchError<S::Error>> {
        let lines = self
            .steps
            .transcript_lines(
                &self.session.session_id,
                &transcript.revision,
                transcript.range,
            )
            .map_err(WatchError::Step)?;
        let mut entries: Vec<TranscriptEntry> = lines
            .into_iter()
            .map(|line| TranscriptEntry::Line {
                start: line.start,
                end: line.end,
                text: line.text,
            })
            .chain(
                transcript
                    .untranscribed
                    .iter()
                    .map(|gap| TranscriptEntry::Gap {
                        start: gap.start(),
                        end: gap.end(),
                        reason: GAP_REASON.to_owned(),
                    }),
            )
            .collect();
        entries.sort_by_key(TranscriptEntry::start);
        Ok(entries)
    }

    /// The frames of the listed changes, at most [`frames_that_fit`] the
    /// room measured before the first, spread evenly, and every change as
    /// the list shows it.
    async fn take_frames<O: WatchOutputs>(
        &self,
        outputs: &mut O,
        changes: &[ScreenChange],
    ) -> Result<TakenFrames, WatchError<S::Error>> {
        let session = &self.session.session_id;
        let mut listed: Vec<ScreenChangeEntry> = changes
            .iter()
            .map(|change| ScreenChangeEntry {
                time: change.time,
                candidate_id: change.id.as_str().to_owned(),
                reason: change.reason,
                frame: None,
            })
            .collect();
        if changes.is_empty() {
            return Ok(TakenFrames {
                frames: Vec::new(),
                listed,
                stopped_for_room: false,
            });
        }
        let room = frames_that_fit(
            self.steps
                .evidence_slots_left(session)
                .map_err(WatchError::Step)?,
        );
        let mut frames = Vec::new();
        for position in spread_evenly(changes.len(), room) {
            let (Some(change), Some(entry)) = (changes.get(position), listed.get_mut(position))
            else {
                continue;
            };
            let stored = match self.steps.take_frame(session, &change.id).await {
                Ok(stored) => stored,
                Err(FrameRefusal::NoRoom) => {
                    return Ok(TakenFrames {
                        frames,
                        listed,
                        stopped_for_room: true,
                    });
                }
                Err(FrameRefusal::Failed(error)) => return Err(WatchError::Step(error)),
            };
            let file_name = frame_file_name(change.time);
            let path = outputs
                .copy_frame(&stored, &file_name)
                .map_err(WatchError::Output)?;
            entry.frame = Some(WrittenFrame {
                file_name,
                evidence_id: stored.evidence_id.as_str().to_owned(),
                sha256: stored.sha256.clone(),
            });
            frames.push(WatchFrame {
                time: change.time,
                candidate_id: change.id.clone(),
                evidence_id: stored.evidence_id,
                path,
                sha256: stored.sha256,
            });
        }
        Ok(TakenFrames {
            frames,
            listed,
            stopped_for_room: false,
        })
    }
}

#[cfg(test)]
mod tests;
