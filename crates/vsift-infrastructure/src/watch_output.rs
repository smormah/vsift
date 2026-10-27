//! The files `vsift <video>` writes into a person's folder (ADR 0037
//! sections 4 to 6): the claim that makes sure nothing is ever replaced, and
//! the plain-text and `SubRip` writers.
//!
//! **Never overwrite.** Every output is claimed by exclusive creation before
//! any work: `create_new` for the two files, `create_dir` for the folder. Both
//! fail on any existing name, a link included, so a name that is taken is
//! refused rather than written through. Later writes go only to the handles
//! and the folder this claim created. Released after a failure, the claim
//! removes exactly what it created, and a file only while it is still empty.
//!
//! **Untrusted text.** The renderers take text already in its display form
//! (hidden and control characters made visible by the caller); the `SubRip`
//! writer also keeps each cue to one line without the timing arrow
//! ([`subrip_cue_text`]).

use std::{
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
};

use sha2::{Digest as _, Sha256};
use vsift_domain::{MediaTime, WatchOutputNames, readable_time, subrip_cue_text, subrip_time};

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
}

impl std::fmt::Display for WatchOutputError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exists(kind) => write!(formatter, "the {kind:?} output already exists"),
            Self::Refused(kind, error) => {
                write!(
                    formatter,
                    "the {kind:?} output could not be written: {error}"
                )
            }
            Self::FrameMismatch => formatter.write_str("a copied frame does not match its digest"),
        }
    }
}

impl std::error::Error for WatchOutputError {}

/// The outputs of one run, claimed in one folder.
#[derive(Debug)]
pub struct WatchClaim {
    transcript_path: PathBuf,
    subtitles_path: PathBuf,
    folder_path: PathBuf,
    transcript: Option<File>,
    subtitles: Option<File>,
    created_in_folder: Vec<PathBuf>,
}

impl WatchClaim {
    /// Claims the three outputs in `folder`, in the order transcript,
    /// subtitles, folder. If one is taken or refused, the ones this call
    /// created are removed again and nothing else is touched.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Exists`] for a taken name,
    /// [`WatchOutputError::Refused`] when the folder refuses the write.
    pub fn claim(folder: &Path, names: &WatchOutputNames) -> Result<Self, WatchOutputError> {
        let transcript_path = folder.join(names.transcript());
        let subtitles_path = folder.join(names.subtitles());
        let folder_path = folder.join(names.folder());
        let transcript = create_new(&transcript_path, WatchOutputKind::Transcript)?;
        let subtitles = match create_new(&subtitles_path, WatchOutputKind::Subtitles) {
            Ok(file) => file,
            Err(error) => {
                drop(transcript);
                remove_if_empty(&transcript_path);
                return Err(error);
            }
        };
        if let Err(error) = fs::create_dir(&folder_path) {
            drop((transcript, subtitles));
            remove_if_empty(&transcript_path);
            remove_if_empty(&subtitles_path);
            return Err(classify(&error, WatchOutputKind::Folder));
        }
        Ok(Self {
            transcript_path,
            subtitles_path,
            folder_path,
            transcript: Some(transcript),
            subtitles: Some(subtitles),
            created_in_folder: Vec::new(),
        })
    }

    /// The readable transcript's path.
    #[must_use]
    pub fn transcript_path(&self) -> &Path {
        &self.transcript_path
    }

    /// The subtitles' path.
    #[must_use]
    pub fn subtitles_path(&self) -> &Path {
        &self.subtitles_path
    }

    /// The screen-change folder's path.
    #[must_use]
    pub fn folder_path(&self) -> &Path {
        &self.folder_path
    }

    /// Writes the readable transcript, once, and flushes it to the disk.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Refused`] when the write fails.
    pub fn write_transcript(&mut self, text: &str) -> Result<(), WatchOutputError> {
        write_once(&mut self.transcript, text, WatchOutputKind::Transcript)
    }

    /// Writes the subtitles, once, and flushes them to the disk.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Refused`] when the write fails.
    pub fn write_subtitles(&mut self, text: &str) -> Result<(), WatchOutputError> {
        write_once(&mut self.subtitles, text, WatchOutputKind::Subtitles)
    }

    /// Removes the claimed subtitles file, still empty, for a video with no
    /// sound track (ADR 0037 section 4).
    pub fn discard_subtitles(&mut self) {
        if self.subtitles.take().is_some() {
            remove_if_empty(&self.subtitles_path);
        }
    }

    /// Copies a frame out of the session into the folder as `file_name`
    /// (created exclusively) and checks the copy against `sha256`.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Exists`] or [`WatchOutputError::Refused`] for the
    /// new file, [`WatchOutputError::FrameMismatch`] when the copy's digest
    /// differs (the copy is then removed).
    pub fn copy_frame(
        &mut self,
        source: &Path,
        file_name: &str,
        sha256: &str,
    ) -> Result<PathBuf, WatchOutputError> {
        let kind = WatchOutputKind::FolderContent;
        let mut bytes = Vec::new();
        File::open(source)
            .and_then(|mut file| file.read_to_end(&mut bytes))
            .map_err(|error| classify(&error, kind))?;
        if hex_sha256(&bytes) != sha256 {
            return Err(WatchOutputError::FrameMismatch);
        }
        let path = self.write_in_folder(file_name, &bytes)?;
        Ok(path)
    }

    /// Writes the screen-change list into the folder, once.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Exists`] or [`WatchOutputError::Refused`].
    pub fn write_screen_changes(&mut self, text: &str) -> Result<PathBuf, WatchOutputError> {
        self.write_in_folder(SCREEN_CHANGES_FILE, text.as_bytes())
    }

    /// Gives the claim up after a failure: removes every file this claim
    /// created in its folder, the folder once it is empty, and the two
    /// files while they are still empty. A file that holds a write stays.
    pub fn release(mut self) {
        drop(self.transcript.take());
        drop(self.subtitles.take());
        for path in self.created_in_folder.drain(..) {
            // Best effort: a file that cannot be removed stays, visible to the
            // person, and never hides a failure that is being reported.
            let _ignored = fs::remove_file(path);
        }
        let _ignored = fs::remove_dir(&self.folder_path);
        remove_if_empty(&self.transcript_path);
        remove_if_empty(&self.subtitles_path);
    }

    fn write_in_folder(
        &mut self,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, WatchOutputError> {
        let kind = WatchOutputKind::FolderContent;
        let path = self.folder_path.join(file_name);
        let mut file = create_new(&path, kind)?;
        self.created_in_folder.push(path.clone());
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| classify(&error, kind))?;
        Ok(path)
    }
}

/// The screen-change list's name inside the folder.
pub const SCREEN_CHANGES_FILE: &str = "screen-changes.txt";

fn create_new(path: &Path, kind: WatchOutputKind) -> Result<File, WatchOutputError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| classify(&error, kind))
}

fn classify(error: &io::Error, kind: WatchOutputKind) -> WatchOutputError {
    if error.kind() == io::ErrorKind::AlreadyExists {
        WatchOutputError::Exists(kind)
    } else {
        WatchOutputError::Refused(kind, error.kind())
    }
}

fn write_once(
    handle: &mut Option<File>,
    text: &str,
    kind: WatchOutputKind,
) -> Result<(), WatchOutputError> {
    let mut file = handle
        .take()
        .ok_or(WatchOutputError::Refused(kind, io::ErrorKind::Other))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| classify(&error, kind))
}

/// Removes a file this run created, only while it is still empty, so a
/// release can never remove text that was written.
fn remove_if_empty(path: &Path) {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() == 0) {
        let _ignored = fs::remove_file(path);
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(64);
    for byte in digest {
        let _infallible = write!(text, "{byte:02x}");
    }
    text
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

/// Renders `<stem>.txt`: the header, then one line per entry. `revision` is
/// the transcript revision's id, `None` for a video with no sound track.
#[must_use]
pub fn render_transcript(
    provenance: &WatchProvenance,
    revision: Option<&str>,
    entries: &[TranscriptEntry],
) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "{}: transcript by {}, {} (local speech recognition).",
        provenance.video_name, provenance.made_by, provenance.made_at
    );
    text.push_str(
        "A machine transcription: check it against the recording before you rely on it.\n\n",
    );
    let _ = writeln!(text, "Video SHA-256    {}", provenance.video_sha256);
    match revision {
        Some(revision) => {
            let _ = writeln!(
                text,
                "Transcript       {revision}   in session {} (closed when this run finished; the ids name it, the hashes can be checked any time)",
                provenance.session_id
            );
        }
        None => text.push_str("Transcript       none: this video has no sound track\n"),
    }
    text.push('\n');
    for entry in entries {
        match entry {
            TranscriptEntry::Line {
                start, text: line, ..
            } => {
                let _ = writeln!(text, "[{}] {line}", readable_time(*start));
            }
            TranscriptEntry::Gap { start, end, reason } => {
                let _ = writeln!(
                    text,
                    "[{}-{}] (not transcribed: {reason})",
                    readable_time(*start),
                    readable_time(*end)
                );
            }
        }
    }
    text
}

/// Renders `<stem>.srt`: one cue per line entry, numbered from 1; gaps are
/// left out, since a player would show them as speech.
#[must_use]
pub fn render_subrip(entries: &[TranscriptEntry]) -> String {
    let mut text = String::new();
    let mut number = 0_usize;
    for entry in entries {
        if let TranscriptEntry::Line {
            start,
            end,
            text: line,
        } = entry
        {
            let cue = subrip_cue_text(line);
            if cue.is_empty() {
                continue;
            }
            number += 1;
            let _ = write!(
                text,
                "{number}\n{} --> {}\n{cue}\n\n",
                subrip_time(*start),
                subrip_time(*end)
            );
        }
    }
    text
}

/// One screen change in the list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenChangeEntry {
    /// The change's representative time.
    pub time: MediaTime,
    /// The visual candidate's id.
    pub candidate_id: String,
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
    /// Whether the video runs past the part the visual index covers.
    pub beyond_index_bound: bool,
}

/// Renders `screen-changes.txt`.
#[must_use]
pub fn render_screen_changes(
    provenance: &WatchProvenance,
    summary: &ScreenChangeSummary,
    changes: &[ScreenChangeEntry],
) -> String {
    let frames = changes
        .iter()
        .filter(|change| change.frame.is_some())
        .count();
    let mut text = String::new();
    let _ = writeln!(
        text,
        "Screen changes in {}, found by {}, {}: {} changes, {frames} frames.",
        provenance.video_name,
        provenance.made_by,
        provenance.made_at,
        changes.len()
    );
    if frames < changes.len() {
        let _ = writeln!(
            text,
            "{frames} of {} frames written, spread evenly over the video; the others are listed without one.",
            changes.len()
        );
    }
    if summary.beyond_index_bound {
        text.push_str("Screen changes are found in the first four hours of a video only.\n");
    }
    text.push('\n');
    let _ = writeln!(text, "Video SHA-256    {}", provenance.video_sha256);
    let _ = writeln!(
        text,
        "Visual index     {}   in session {} (closed when this run finished; the ids name it, the hashes can be checked any time)",
        summary.index_id, provenance.session_id
    );
    text.push('\n');
    let _ = writeln!(
        text,
        "{:<10}  {:<18}  {:<36}  {:<36}  Frame SHA-256",
        "Time", "Frame", "Change", "Frame evidence"
    );
    for change in changes {
        let (file, evidence, sha256) =
            change
                .frame
                .as_ref()
                .map_or(("(not written)", "-", "-"), |frame| {
                    (
                        frame.file_name.as_str(),
                        frame.evidence_id.as_str(),
                        frame.sha256.as_str(),
                    )
                });
        let _ = writeln!(
            text,
            "{:<10}  {file:<18}  {:<36}  {evidence:<36}  {sha256}",
            readable_time(change.time),
            change.candidate_id
        );
    }
    text
}

#[cfg(test)]
mod tests;
