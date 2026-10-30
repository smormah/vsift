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
//! a written file) and the session is closed, except after Ctrl-C (below).
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
//!
//! **Files grow while the job runs** (section 7). The engine tells the use
//! case through [`WatchProgress`] after each chunk of speech and when the
//! visual stream is done; the callback only records. Beside the job, on the
//! same task, the use case writes a snapshot of what was published at most
//! once every [`SNAPSHOT_INTERVAL_SECONDS`] (read through the [`Clock`]),
//! and the frames and their list as soon as the visual stream is done.
//!
//! **Ctrl-C keeps the session** (section 8). The files keep their last
//! snapshot and marker, and the same command continues the job: the job's
//! own record of its claimed names ([`WatchClaimLog`]) is what finds it,
//! never the text of a file in the person's folder.
//!
//! **`--keep` writes a bundle once the job has sealed** (the amendment of
//! 2026-10-30). The bundle comes after the outputs and before the close, and
//! the session closes only after it is committed: a bundle that cannot be
//! written leaves the session open, and a final write that fails on a sealed
//! job does not stop the bundle, which does not need the outputs. A run that
//! stops or fails before the seal keeps nothing. The option is never recorded
//! in the job: the run that finishes does what its own command line says.

use std::{
    ffi::OsStr,
    fmt, io,
    path::{Path, PathBuf},
};

use crate::{Clock, SessionStorageError};
use vsift_domain::{
    CandidateReason, EvidenceId, FailureCode, JobId, MAX_VISUAL_INDEX_DURATION_MICROS, MediaTime,
    OutputNameError, SessionId, SourceId, TimeRange, TranscriptRevisionId, VisualCandidateId,
    VisualIndexId, WatchOutputNames, frame_file_name, frames_that_fit, settled_screens,
    spread_evenly,
};

mod follow;

use follow::{WatchSignal, join};

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

/// How the `prepare` job stopped short of its end.
#[derive(Debug)]
pub enum PrepareStop<E> {
    /// The caller cancelled the run (Ctrl-C): the job is interrupted, its
    /// finished chunks kept, and the same command continues it.
    Interrupted,
    /// Any other failure, which ends the run.
    Failed(E),
}

/// What the `prepare` job tells the use case while it runs (the planner's
/// call 1, 2026-10-29). The engine invokes it on the job's own task, so it
/// records and returns: it never writes, reads or waits.
pub trait WatchProgress: Send + Sync {
    /// A chunk of speech finished, so a part of the open revision may have
    /// been published.
    fn published(&self);

    /// The visual stream has indexed the whole video (`StreamEnd::Indexed`):
    /// the frames can be written while the transcription runs.
    fn visual_indexed(&self);
}

/// What a read of the open revision found published.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedPart {
    /// The open revision.
    pub revision: TranscriptRevisionId,
    /// The segments the read returned, in time order, text in display form.
    pub lines: Vec<TranscriptLine>,
    /// How far the revision has published.
    pub published_until: MediaTime,
    /// The video's length.
    pub duration: MediaTime,
    /// The token the next read follows the revision with; `None` when the
    /// read gave none (the next read then reads everything again).
    pub since: Option<String>,
}

/// An interrupted run of the same video, found by its claimed names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnfinishedWatch {
    /// Its session, still open.
    pub session: OpenedWatchSession,
    /// Its unfinished `prepare` job.
    pub job: JobId,
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

    /// Starts the session's `prepare` job in the watched mode (ADR 0033); its
    /// id.
    ///
    /// # Errors
    ///
    /// Every error of `job start`.
    fn start_job(&self, session: &SessionId) -> Result<JobId, Self::Error>;

    /// Keeps the names this run claimed in the job's own record, in the
    /// session, so that a rerun finds the job by them and never by reading
    /// the person's folder (ADR 0037 section 8, the planner's call 2).
    ///
    /// # Errors
    ///
    /// Storage failures.
    fn record_claim(
        &self,
        session: &SessionId,
        job: &JobId,
        claimed: &ClaimedOutputs,
    ) -> Result<(), Self::Error>;

    /// Runs the `prepare` job in this process to its end. `progress` is told
    /// after each chunk of speech (a part may have been published) and when
    /// the visual stream has indexed the whole video; it records and
    /// returns.
    ///
    /// # Errors
    ///
    /// [`PrepareStop::Interrupted`] when the caller cancelled the run, which
    /// leaves the job resumable, and [`PrepareStop::Failed`] for every other
    /// error of the job; a video with no sound track is not one.
    fn prepare(
        &self,
        session: &SessionId,
        job: &JobId,
        progress: &dyn WatchProgress,
    ) -> impl Future<Output = Result<PreparedRun, PrepareStop<Self::Error>>> + Send;

    /// What the job's open revision has published: everything when `since`
    /// is `None`, else only what it published after that token (a `--since`
    /// read). `None` before the first part.
    ///
    /// # Errors
    ///
    /// Every error of the transcript read.
    fn published(
        &self,
        session: &SessionId,
        since: Option<&str>,
    ) -> Result<Option<PublishedPart>, Self::Error>;

    /// An open session of this video whose unfinished `prepare` job claimed
    /// exactly `claimed`, found through the job's own record (ADR 0037
    /// section 8). An expired session is not found.
    ///
    /// # Errors
    ///
    /// Storage failures and reading the video.
    fn find_unfinished(
        &self,
        video: &Path,
        claimed: &ClaimedOutputs,
    ) -> impl Future<Output = Result<Option<UnfinishedWatch>, Self::Error>> + Send;

    /// The clock the snapshots are spaced by.
    fn clock(&self) -> &dyn Clock;

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

    /// Writes the session as a retained bundle at `bundle`, as `session
    /// retain` does, without the source (ADR 0013).
    ///
    /// # Errors
    ///
    /// The public code of `session retain`'s failure: the run reports it and
    /// leaves the session open, so the work is not lost.
    fn keep_session(&self, session: &SessionId, bundle: &Path) -> Result<(), FailureCode>;

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
    /// The bundle `--keep` writes, `<folder>/<name>`.
    Kept,
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
    /// Another program held the file open for the final write's 30 s
    /// (known limit L-237); the last snapshot stays, with its marker.
    Held(WatchOutputKind),
    /// `--output` names a folder whose parent does not exist, or something
    /// that is not a folder.
    FolderUnusable,
    /// The folder lies inside `VSift`'s own session or managed root, which
    /// `VSift` cleans automatically.
    InsideVsift,
    /// `--keep` names a folder whose parent does not exist, something that
    /// is not a folder, or a place a bundle cannot be (a network path).
    KeepFolderUnusable,
    /// `--keep` names a folder inside `VSift`'s own roots.
    KeepInsideVsift,
    /// The video's name cannot name a bundle's folder (a Windows device
    /// name, which a bundle refuses on every system).
    KeepNameNotAllowed,
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
            Self::Held(kind) => {
                write!(
                    formatter,
                    "the {kind:?} output is held open by another program"
                )
            }
            Self::FolderUnusable => formatter.write_str("the output folder cannot be used"),
            Self::InsideVsift => {
                formatter.write_str("the output folder is inside VSift's own folders")
            }
            Self::KeepFolderUnusable => formatter.write_str("the keep folder cannot be used"),
            Self::KeepInsideVsift => {
                formatter.write_str("the keep folder is inside VSift's own folders")
            }
            Self::KeepNameNotAllowed => {
                formatter.write_str("the video's name cannot name a bundle's folder")
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

/// Port that keeps the names a `watch` run claimed in its job's own record,
/// in the session (ADR 0037 section 8, the planner's call 2 of 2026-10-29):
/// a small versioned record beside the job, removed with it.
pub trait WatchClaimLog {
    /// Records the names `job` claimed.
    ///
    /// # Errors
    ///
    /// Storage failures, and a folder or name that is not Unicode text.
    fn record_watch_claim(
        &self,
        session: &SessionId,
        job: &JobId,
        claimed: &ClaimedOutputs,
    ) -> Result<(), SessionStorageError>;

    /// The claims recorded by the session's jobs; a record that cannot be
    /// read is left out, so it is never misread as a claim.
    ///
    /// # Errors
    ///
    /// Storage failures of the session's jobs.
    fn watch_claims(
        &self,
        session: &SessionId,
    ) -> Result<Vec<(JobId, ClaimedOutputs)>, SessionStorageError>;
}

/// Facts every output names about the run that made it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchProvenance {
    /// The video's file name, in display form.
    pub video_name: String,
    /// `VSift` and its version, for example `VSift 0.3.0`.
    pub made_by: String,
    /// When the file was written, in UTC to the minute (`2026-11-20 14:05
    /// UTC`).
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
}

/// The transcript as far as it has been published, for a file that grows
/// (ADR 0037 section 7).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchSnapshot {
    /// Where the run is: what the files' markers say.
    pub state: SnapshotState,
    /// What every output names about the run.
    pub provenance: WatchProvenance,
    /// The open revision; `None` before its first part.
    pub revision: Option<TranscriptRevisionId>,
    /// The published segments, in time order.
    pub entries: Vec<TranscriptEntry>,
    /// How far the revision has published.
    pub published_until: MediaTime,
    /// The video's length, once a part has said it.
    pub duration: Option<MediaTime>,
}

/// Where the run is when a snapshot is written, so that the files' markers
/// stay true (the planner's condition of 2026-10-29).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotState {
    /// The job runs: the files are replaced when it finishes.
    Running,
    /// The person stopped the run (Ctrl-C): the files say where, and that
    /// the same command continues it while `VSift` keeps its private copy.
    Stopped,
    /// The run failed after the files were written: they say so, and that
    /// they will not be finished.
    Failed,
}

/// The finished files the port wrote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalisedOutputs {
    /// The readable transcript.
    pub transcript: WatchFile,
    /// The subtitles; `None` for a video with no sound track.
    pub subtitles: Option<WatchFile>,
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

    /// The paths [`WatchOutputs::claim`] would claim, without creating
    /// anything: for a rerun whose names are taken.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::FolderUnusable`] or
    /// [`WatchOutputError::InsideVsift`] for the folder.
    fn planned(
        &mut self,
        video: &Path,
        output: Option<&Path>,
        names: &WatchOutputNames,
    ) -> Result<ClaimedOutputs, WatchOutputError>;

    /// Takes over the outputs an interrupted run of the same job claimed
    /// (its job's record names them; never decided by reading the folder):
    /// the two files and the folder must still be a regular file and a
    /// folder, never a link. Frames already in the folder are kept.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Exists`] when an output is no longer what the
    /// run created (a link, a folder where a file was).
    fn reclaim(&mut self, claimed: &ClaimedOutputs) -> Result<(), WatchOutputError>;

    /// Resolves the `--keep` folder (an existing folder, or a new one whose
    /// parent exists, never inside `VSift`'s own roots) and checks, before
    /// any work, that a bundle can be written at `<folder>/<name>`; that
    /// path. The name is checked, not held: `retain_bundle` must create the
    /// bundle's folder itself (ADR 0013).
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Exists`] for a taken name,
    /// [`WatchOutputError::Refused`] when the folder refuses,
    /// [`WatchOutputError::KeepFolderUnusable`],
    /// [`WatchOutputError::KeepInsideVsift`] and
    /// [`WatchOutputError::KeepNameNotAllowed`].
    fn prepare_keep(
        &mut self,
        folder: &Path,
        names: &WatchOutputNames,
    ) -> Result<PathBuf, WatchOutputError>;

    /// Writes a snapshot of the transcript and the subtitles with their
    /// in-progress markers, each to an exclusive temporary file moved over
    /// the claimed name only (the planner's condition B). One attempt: a
    /// file another program holds open is skipped until the next snapshot.
    ///
    /// # Errors
    ///
    /// The write's error; the use case goes on without it.
    fn write_snapshot(&mut self, snapshot: &WatchSnapshot) -> Result<(), WatchOutputError>;

    /// Writes the screen-change list into the folder, once the frames are
    /// written (ADR 0037 section 7).
    ///
    /// # Errors
    ///
    /// The claim's write errors.
    fn write_screen_changes(
        &mut self,
        provenance: &WatchProvenance,
        summary: &ScreenChangeSummary,
        changes: &[ScreenChangeEntry],
    ) -> Result<WatchFile, WatchOutputError>;

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

    /// Writes the finished transcript and subtitles over their snapshots, or
    /// removes the subtitles' claim, still empty, for a video with no sound
    /// track. A file another program holds open is tried for up to 30 s.
    ///
    /// # Errors
    ///
    /// The claim's write errors, and [`WatchOutputError::Held`].
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

/// What became of the bundle `--keep` asked for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchKept {
    /// Where the bundle is, or where it was to go.
    pub path: PathBuf,
    /// Why it was not written; `None` when it is committed.
    pub failure: Option<FailureCode>,
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
    /// The bundle `--keep` asked for; `None` without the option.
    pub kept: Option<WatchKept>,
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
    /// The caller stopped the run (Ctrl-C). The files keep their last
    /// snapshot and its marker, the session is kept, and the same command
    /// continues the job (ADR 0037 section 8).
    Stopped {
        /// How far the transcript had been published.
        transcribed_to: MediaTime,
        /// The video's length, when a part had said it.
        duration: Option<MediaTime>,
    },
    /// The run failed after the job sealed, and `--keep` still wrote, or
    /// tried to write, the bundle: a person never loses the kept copy to a
    /// failed output.
    FailedAfterKeep {
        /// What failed.
        error: Box<Self>,
        /// What became of the bundle.
        kept: WatchKept,
        /// The session.
        session: SessionId,
        /// Whether it was closed: never when the bundle was not written.
        session_closed: bool,
    },
}

/// The least time between two snapshots of a file that grows, in seconds
/// (ADR 0037 section 7).
pub const SNAPSHOT_INTERVAL_SECONDS: u64 = 10;

/// Watches one `video` (ADR 0037 section 3): writes `<stem>.txt`,
/// `<stem>.srt` and `<stem>-vsift/` beside it or into `output`, and, with
/// `keep`, a retained bundle under that folder once the job has sealed.
///
/// `video`, `output` and `keep` are absolute. Nothing is copied or written before
/// every output name is claimed. When a name is taken, the run continues an
/// interrupted run of the same video only if its job's own record claimed
/// exactly these names in a session still open (section 8); otherwise it is
/// refused as before. While the job runs the transcript and the subtitles
/// grow by snapshots, and the frames are written once the visual stream is
/// done. Stopped by the caller, the run keeps the files with their last
/// snapshot and the session; after any other failure it releases the claim
/// and closes the session. With `keep`, a session whose job has sealed closes
/// only once its bundle is committed.
///
/// # Errors
///
/// [`WatchError::Step`] for the operations' errors,
/// [`WatchError::UnusableName`] or [`WatchError::Output`] for the outputs,
/// [`WatchError::Stopped`] when the caller stopped the run, and
/// [`WatchError::FailedAfterKeep`] for a failure after the job sealed, with
/// `keep`.
pub async fn run_watch<S: WatchSteps, O: WatchOutputs>(
    steps: &S,
    outputs: &mut O,
    video: &Path,
    output: Option<&Path>,
    keep: Option<&Path>,
) -> Result<WatchOutcome, WatchError<S::Error>> {
    steps.check_source(video).await.map_err(WatchError::Step)?;
    steps.check_capabilities().map_err(WatchError::Step)?;
    let file_name = video
        .file_name()
        .ok_or(WatchError::UnusableName(OutputNameError::Empty))?;
    let names = WatchOutputNames::from_video_name(file_name).map_err(WatchError::UnusableName)?;
    let (claimed, unfinished) = claim_or_continue(steps, outputs, video, output, &names).await?;
    // After the claim, so a run refused for a taken output leaves no new keep
    // folder; before the session, so a refusal costs nothing.
    let bundle = match keep.map(|folder| outputs.prepare_keep(folder, &names)) {
        Some(Err(error)) => {
            outputs.release();
            return Err(WatchError::Output(error));
        }
        Some(Ok(bundle)) => Some(bundle),
        None => None,
    };
    let (session, job) = match unfinished {
        Some(unfinished) => (unfinished.session, Some(unfinished.job)),
        None => match steps.open_session(video).await {
            Ok(session) => (session, None),
            Err(error) => {
                outputs.release();
                return Err(WatchError::Step(error));
            }
        },
    };
    let run = WatchRun {
        steps,
        session: &session,
        file_name,
        names: &names,
        claimed,
        taken_over: job.is_some(),
    };
    let (written, sealed) = Box::pin(run.write(outputs, job)).await;
    if let Err(stopped @ WatchError::Stopped { .. }) = written {
        return Err(stopped);
    }
    // A run that did not reach the seal keeps nothing. Past it, the bundle
    // does not need the outputs, so a failed one does not stop it.
    let kept = bundle.filter(|_| sealed).map(|path| WatchKept {
        failure: steps.keep_session(&session.session_id, &path).err(),
        path,
    });
    // Never a close that loses the kept copy: a session whose bundle failed
    // stays open for `session retain`.
    let keep_failed = kept.as_ref().is_some_and(|kept| kept.failure.is_some());
    let closed = !keep_failed && steps.close_session(&session.session_id);
    match written {
        Ok(mut outcome) => {
            outcome.session_closed = closed;
            outcome.kept = kept;
            Ok(outcome)
        }
        Err(error) => {
            outputs.release();
            Err(match kept {
                Some(kept) => WatchError::FailedAfterKeep {
                    error: Box::new(error),
                    kept,
                    session: session.session_id.clone(),
                    session_closed: closed,
                },
                None => error,
            })
        }
    }
}

/// Claims the names, or, when one is taken, takes over the outputs of an
/// unfinished run of the same video that claimed exactly these names.
async fn claim_or_continue<S: WatchSteps, O: WatchOutputs>(
    steps: &S,
    outputs: &mut O,
    video: &Path,
    output: Option<&Path>,
    names: &WatchOutputNames,
) -> Result<(ClaimedOutputs, Option<UnfinishedWatch>), WatchError<S::Error>> {
    match outputs.claim(video, output, names) {
        Ok(claimed) => Ok((claimed, None)),
        Err(taken @ WatchOutputError::Exists(_)) => {
            let planned = outputs
                .planned(video, output, names)
                .map_err(WatchError::Output)?;
            // Decided by the session's record of the job, never by the text of
            // a file in the person's folder (the planner's condition A): an
            // expired session is gone, so its files are refused.
            let Some(unfinished) = steps
                .find_unfinished(video, &planned)
                .await
                .map_err(WatchError::Step)?
            else {
                return Err(WatchError::Output(taken));
            };
            outputs.reclaim(&planned).map_err(WatchError::Output)?;
            Ok((planned, Some(unfinished)))
        }
        Err(error) => Err(WatchError::Output(error)),
    }
}

/// One run, from the open session to the finished files.
struct WatchRun<'a, S> {
    steps: &'a S,
    session: &'a OpenedWatchSession,
    file_name: &'a OsStr,
    names: &'a WatchOutputNames,
    claimed: ClaimedOutputs,
    /// Whether the run took over the files of an interrupted run, which
    /// already hold a snapshot.
    taken_over: bool,
}

/// The frames a run took, and every change as the list shows it.
struct TakenFrames {
    frames: Vec<WatchFrame>,
    listed: Vec<ScreenChangeEntry>,
    stopped_for_room: bool,
}

/// The screen changes, their frames and the list, written once the visual
/// stream is done.
struct Screens {
    index: Option<VisualIndexId>,
    covered_to: MediaTime,
    changes_found: usize,
    changes_listed: usize,
    taken: TakenFrames,
    list: WatchFile,
}

/// What the run has followed of the open revision, for the snapshots.
struct Following<E> {
    revision: Option<TranscriptRevisionId>,
    lines: Vec<TranscriptLine>,
    since: Option<String>,
    published_until: MediaTime,
    duration: Option<MediaTime>,
    last_snapshot_at: Option<u64>,
    /// Whether this run wrote a snapshot into the files.
    snapshot_written: bool,
    /// Whether the job ended well: its revision is sealed.
    sealed: bool,
    screens: Option<Result<Screens, WatchError<E>>>,
}

impl<S: WatchSteps> WatchRun<'_, S> {
    /// Runs the job and writes the files. A failure once the files hold a
    /// snapshot (this run's, or the interrupted run's it took over) rewrites
    /// them, best effort, to say the run failed and they will not be
    /// finished: they are never removed and never left saying `VSift` is
    /// working. Also says whether the job sealed, whatever became of the
    /// files.
    async fn write<O: WatchOutputs>(
        self,
        outputs: &mut O,
        unfinished_job: Option<JobId>,
    ) -> (Result<WatchOutcome, WatchError<S::Error>>, bool) {
        let mut following = Following {
            revision: None,
            lines: Vec::new(),
            since: None,
            published_until: MediaTime::from_micros(0),
            duration: None,
            last_snapshot_at: None,
            snapshot_written: false,
            sealed: false,
            screens: None,
        };
        let written = self.run(outputs, unfinished_job, &mut following).await;
        if let Err(error) = &written
            && !matches!(error, WatchError::Stopped { .. })
            && (following.snapshot_written || self.taken_over)
        {
            self.refresh(&mut following);
            let _best_effort = self.snapshot(outputs, &mut following, SnapshotState::Failed);
        }
        (written, following.sealed)
    }

    async fn run<O: WatchOutputs>(
        &self,
        outputs: &mut O,
        unfinished_job: Option<JobId>,
        following: &mut Following<S::Error>,
    ) -> Result<WatchOutcome, WatchError<S::Error>> {
        let session = &self.session.session_id;
        let job = if let Some(job) = unfinished_job {
            job
        } else {
            let job = self.steps.start_job(session).map_err(WatchError::Step)?;
            self.steps
                .record_claim(session, &job, &self.claimed)
                .map_err(WatchError::Step)?;
            job
        };
        let signal = WatchSignal::default();
        let prepare = async {
            let prepared = self.steps.prepare(session, &job, &signal).await;
            signal.close();
            prepared
        };
        let (prepared, ()) = join(prepare, self.follow(outputs, &signal, following)).await;
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(PrepareStop::Interrupted) => {
                // The files keep what was published, and say where it stopped.
                self.refresh(following);
                let _advisory = self.snapshot(outputs, following, SnapshotState::Stopped);
                return Err(WatchError::Stopped {
                    transcribed_to: following.published_until,
                    duration: following.duration,
                });
            }
            Err(PrepareStop::Failed(error)) => return Err(WatchError::Step(error)),
        };
        following.sealed = true;
        let screens = match following.screens.take() {
            Some(screens) => screens?,
            None => self.screens(outputs).await?,
        };
        self.finish(outputs, prepared, screens)
    }

    /// Follows the job until it ends: a snapshot after a publication, at
    /// most once every [`SNAPSHOT_INTERVAL_SECONDS`], and the frames once
    /// the visual stream is done.
    async fn follow<O: WatchOutputs>(
        &self,
        outputs: &mut O,
        signal: &WatchSignal,
        following: &mut Following<S::Error>,
    ) {
        while let Some(notice) = signal.next().await {
            if notice.indexed && following.screens.is_none() {
                following.screens = Some(self.screens(outputs).await);
            }
            if notice.published && self.snapshot_due(following) {
                self.refresh(following);
                if self
                    .snapshot(outputs, following, SnapshotState::Running)
                    .is_ok()
                {
                    following.last_snapshot_at = self.steps.clock().now_unix_seconds().ok();
                }
            }
        }
    }

    fn snapshot_due(&self, following: &Following<S::Error>) -> bool {
        let Some(last) = following.last_snapshot_at else {
            return true;
        };
        self.steps
            .clock()
            .now_unix_seconds()
            .is_ok_and(|now| now.saturating_sub(last) >= SNAPSHOT_INTERVAL_SECONDS)
    }

    /// Reads what the open revision published since the last read. A failed
    /// read changes nothing: snapshots are advisory, and the next one reads
    /// everything again.
    fn refresh(&self, following: &mut Following<S::Error>) {
        let read = self
            .steps
            .published(&self.session.session_id, following.since.as_deref());
        match read {
            Ok(Some(part)) => {
                if following.since.is_none() {
                    following.lines.clear();
                }
                following.lines.extend(part.lines);
                following.revision = Some(part.revision);
                following.since = part.since;
                following.published_until = part.published_until;
                following.duration = Some(part.duration);
            }
            Ok(None) => {}
            Err(_) => following.since = None,
        }
    }

    /// Writes one snapshot of what has been published; its failure is the
    /// port's, and the use case goes on (a file held open is tried again at
    /// the next one).
    fn snapshot<O: WatchOutputs>(
        &self,
        outputs: &mut O,
        following: &mut Following<S::Error>,
        state: SnapshotState,
    ) -> Result<(), WatchError<S::Error>> {
        let provenance = self
            .steps
            .provenance(
                self.file_name,
                &self.session.source_id,
                &self.session.session_id,
            )
            .map_err(WatchError::Step)?;
        outputs
            .write_snapshot(&WatchSnapshot {
                state,
                provenance,
                revision: following.revision.clone(),
                entries: following
                    .lines
                    .iter()
                    .map(|line| TranscriptEntry::Line {
                        start: line.start,
                        end: line.end,
                        text: line.text.clone(),
                    })
                    .collect(),
                published_until: following.published_until,
                duration: following.duration,
            })
            .map_err(WatchError::Output)?;
        following.snapshot_written = true;
        Ok(())
    }

    /// The screen changes, the settled screens among them, their frames and
    /// the list, written into the folder.
    async fn screens<O: WatchOutputs>(
        &self,
        outputs: &mut O,
    ) -> Result<Screens, WatchError<S::Error>> {
        let session = &self.session.session_id;
        let changes = self
            .steps
            .screen_changes(session)
            .await
            .map_err(WatchError::Step)?;
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
        let covered_to = changes
            .as_ref()
            .map_or(MediaTime::from_micros(0), |changes| changes.covered_to);
        let summary = ScreenChangeSummary {
            index_id: changes.as_ref().map_or_else(
                || "none".to_owned(),
                |changes| changes.index.as_str().to_owned(),
            ),
            changes_found: found.len(),
            index_covers_to: covered_to,
            // The index stops at its bound, so an index that reaches it
            // covers part of a longer video.
            beyond_index_bound: covered_to.as_micros() >= MAX_VISUAL_INDEX_DURATION_MICROS,
            stopped_for_room: taken.stopped_for_room,
        };
        let list = outputs
            .write_screen_changes(&provenance, &summary, &taken.listed)
            .map_err(WatchError::Output)?;
        Ok(Screens {
            index: changes.map(|changes| changes.index),
            covered_to,
            changes_found: found.len(),
            changes_listed: settled.len(),
            taken,
            list,
        })
    }

    /// The finished files, once the job has sealed its revision.
    fn finish<O: WatchOutputs>(
        &self,
        outputs: &mut O,
        prepared: PreparedRun,
        screens: Screens,
    ) -> Result<WatchOutcome, WatchError<S::Error>> {
        let session = &self.session.session_id;
        let entries = match &prepared.transcript {
            Some(transcript) => self.transcript_entries(transcript)?,
            None => Vec::new(),
        };
        let duration = prepared
            .transcript
            .as_ref()
            .map_or(screens.covered_to, |transcript| transcript.range.end());
        let beyond_visual_bound = duration.as_micros() > MAX_VISUAL_INDEX_DURATION_MICROS;
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
        };
        let finalised = outputs.finalise(&documents).map_err(WatchError::Output)?;
        Ok(WatchOutcome {
            source_id: self.session.source_id.clone(),
            session_id: session.clone(),
            session_closed: false,
            job_id: prepared.job_id,
            transcript_revision,
            visual_index: screens.index,
            duration,
            not_transcribed: prepared
                .transcript
                .map(|transcript| transcript.untranscribed)
                .unwrap_or_default(),
            transcript: finalised.transcript,
            subtitles: finalised.subtitles,
            folder: WatchFile {
                path: self.claimed.folder.clone(),
                sha256: None,
            },
            screen_changes: screens.list,
            changes_found: screens.changes_found,
            changes_listed: screens.changes_listed,
            frames: screens.taken.frames,
            beyond_visual_bound,
            names_shortened: self.names.shortened(),
            frames_stopped_for_room: screens.taken.stopped_for_room,
            kept: None,
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
