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
use vsift_application::{
    ClaimedOutputs, FinalisedOutputs, ScreenChangeEntry, ScreenChangeSummary, SessionFrame,
    TranscriptEntry, WatchDocuments, WatchFile, WatchOutputError, WatchOutputKind, WatchOutputs,
    WatchProvenance,
};
use vsift_domain::{
    CandidateReason, WatchOutputNames, readable_time, subrip_cue_text, subrip_time,
};

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

    /// Writes the readable transcript, once, flushes it to the disk and
    /// returns the SHA-256 of what it wrote.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Refused`] when the write fails.
    pub fn write_transcript(&mut self, text: &str) -> Result<String, WatchOutputError> {
        write_once(&mut self.transcript, text, WatchOutputKind::Transcript)
    }

    /// Writes the subtitles, once, flushes them to the disk and returns the
    /// SHA-256 of what it wrote.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Refused`] when the write fails.
    pub fn write_subtitles(&mut self, text: &str) -> Result<String, WatchOutputError> {
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

    /// Writes the screen-change list into the folder, once, and returns its
    /// path and the SHA-256 of what it wrote.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Exists`] or [`WatchOutputError::Refused`].
    pub fn write_screen_changes(
        &mut self,
        text: &str,
    ) -> Result<(PathBuf, String), WatchOutputError> {
        let path = self.write_in_folder(SCREEN_CHANGES_FILE, text.as_bytes())?;
        Ok((path, hex_sha256(text.as_bytes())))
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

/// The folder implementation of the [`WatchOutputs`] port: resolves the
/// folder, claims the outputs there with a [`WatchClaim`] and renders the
/// finished files with this module's writers.
#[derive(Debug)]
pub struct FolderWatchOutputs {
    vsift_roots: Vec<PathBuf>,
    claim: Option<WatchClaim>,
}

impl FolderWatchOutputs {
    /// Outputs that refuse any folder inside `vsift_roots`, the canonical
    /// session and managed roots, which `VSift` cleans automatically.
    #[must_use]
    pub const fn new(vsift_roots: Vec<PathBuf>) -> Self {
        Self {
            vsift_roots,
            claim: None,
        }
    }

    fn claimed(&mut self) -> Result<&mut WatchClaim, WatchOutputError> {
        self.claim.as_mut().ok_or(WatchOutputError::Refused(
            WatchOutputKind::FolderContent,
            io::ErrorKind::NotFound,
        ))
    }
}

impl WatchOutputs for FolderWatchOutputs {
    fn claim(
        &mut self,
        video: &Path,
        output: Option<&Path>,
        names: &WatchOutputNames,
    ) -> Result<ClaimedOutputs, WatchOutputError> {
        let folder = output_folder(video, output, &self.vsift_roots)?;
        let claim = WatchClaim::claim(&folder, names)?;
        let claimed = ClaimedOutputs {
            transcript: claim.transcript_path().to_path_buf(),
            subtitles: claim.subtitles_path().to_path_buf(),
            folder: claim.folder_path().to_path_buf(),
        };
        self.claim = Some(claim);
        Ok(claimed)
    }

    fn copy_frame(
        &mut self,
        frame: &SessionFrame,
        file_name: &str,
    ) -> Result<PathBuf, WatchOutputError> {
        self.claimed()?
            .copy_frame(&frame.path, file_name, &frame.sha256)
    }

    fn finalise(
        &mut self,
        documents: &WatchDocuments,
    ) -> Result<FinalisedOutputs, WatchOutputError> {
        let claim = self.claimed()?;
        let revision = documents
            .transcript_revision
            .as_ref()
            .map(vsift_domain::TranscriptRevisionId::as_str);
        let transcript_sha256 = claim.write_transcript(&render_transcript(
            &documents.provenance,
            revision,
            &documents.entries,
        ))?;
        let subtitles = if revision.is_some() {
            let sha256 = claim.write_subtitles(&render_subrip(&documents.entries))?;
            Some(WatchFile {
                path: claim.subtitles_path().to_path_buf(),
                sha256: Some(sha256),
            })
        } else {
            claim.discard_subtitles();
            None
        };
        let (list, list_sha256) = claim.write_screen_changes(&render_screen_changes(
            &documents.provenance,
            &documents.summary,
            &documents.changes,
        ))?;
        let transcript = WatchFile {
            path: claim.transcript_path().to_path_buf(),
            sha256: Some(transcript_sha256),
        };
        // Finished: nothing this run wrote may be released any more.
        self.claim = None;
        Ok(FinalisedOutputs {
            transcript,
            subtitles,
            screen_changes: WatchFile {
                path: list,
                sha256: Some(list_sha256),
            },
        })
    }

    fn release(&mut self) {
        if let Some(claim) = self.claim.take() {
            claim.release();
        }
    }
}

/// The folder to write into: the video's own, or `output` (an existing
/// folder, or a new one whose parent exists, created without its parents),
/// canonicalised and never inside one of `vsift_roots` (ADR 0037 section 5).
fn output_folder(
    video: &Path,
    output: Option<&Path>,
    vsift_roots: &[PathBuf],
) -> Result<PathBuf, WatchOutputError> {
    let unusable = WatchOutputError::FolderUnusable;
    let folder = match output {
        None => video.parent().map(Path::to_path_buf).ok_or(unusable)?,
        Some(output) => match fs::symlink_metadata(output) {
            Ok(metadata) if metadata.is_dir() => output.to_path_buf(),
            Ok(_) => return Err(WatchOutputError::FolderUnusable),
            Err(_) => {
                fs::create_dir(output).map_err(|_| WatchOutputError::FolderUnusable)?;
                output.to_path_buf()
            }
        },
    };
    let folder = fs::canonicalize(&folder).map_err(|_| WatchOutputError::FolderUnusable)?;
    if vsift_roots.iter().any(|root| folder.starts_with(root)) {
        return Err(WatchOutputError::InsideVsift);
    }
    Ok(folder)
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
) -> Result<String, WatchOutputError> {
    let mut file = handle
        .take()
        .ok_or(WatchOutputError::Refused(kind, io::ErrorKind::Other))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| classify(&error, kind))?;
    Ok(hex_sha256(text.as_bytes()))
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
        "Screen changes in {}, found by {}, {}: {} settled screens of {} changes found, {frames} frames.",
        provenance.video_name,
        provenance.made_by,
        provenance.made_at,
        changes.len(),
        summary.changes_found
    );
    if changes.len() < summary.changes_found {
        text.push_str(
            "Listed: the first frame, each screen once motion settles, and a change at least 5 s after the one listed before it; the others are left out.
",
        );
    }
    if frames < changes.len() {
        let _ = writeln!(
            text,
            "{frames} of {} frames written, spread evenly over the video; the others are listed without one.",
            changes.len()
        );
    }
    if summary.stopped_for_room {
        text.push_str(
            "VSift's private copy of the video ran out of room for frames, so the frames stop early; every settled screen is still listed.\n",
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
        "{:<10}  {:<20}  {:<18}  {:<36}  {:<36}  Frame SHA-256",
        "Time", "Reason", "Frame", "Change", "Frame evidence"
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
            "{:<10}  {:<20}  {file:<18}  {:<36}  {evidence:<36}  {sha256}",
            readable_time(change.time),
            reason_words(change.reason),
            change.candidate_id
        );
    }
    text
}

/// A change's reason in plain words, as the list shows it.
const fn reason_words(reason: CandidateReason) -> &'static str {
    match reason {
        CandidateReason::FirstFrame => "first frame",
        CandidateReason::VisualChange => "visual change",
        CandidateReason::MotionStart => "motion starts",
        CandidateReason::SettledAfterMotion => "settled after motion",
        CandidateReason::PeriodicCoverage => "periodic coverage",
    }
}

#[cfg(test)]
mod tests;
