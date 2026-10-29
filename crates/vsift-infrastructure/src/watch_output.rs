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
    fmt::{self, Write as _},
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use sha2::{Digest as _, Sha256};
use vsift_application::{
    ClaimedOutputs, FinalisedOutputs, IdentifierSource, ScreenChangeEntry, ScreenChangeSummary,
    SessionFrame, TranscriptEntry, WatchDocuments, WatchFile, WatchOutputError, WatchOutputKind,
    WatchOutputs, WatchProvenance, WatchSnapshot,
};
use vsift_domain::{
    CandidateReason, WatchOutputNames, readable_time, subrip_cue_text, subrip_time,
};

use crate::RandomIdentifierSource;

/// How long the final write of a file tries a file another program holds
/// open before the run fails with `STORAGE_IO` (ADR 0037 section 7, known
/// limit L-237).
pub const HELD_FILE_WAIT: Duration = Duration::from_secs(30);

/// How long the final write waits between two tries of a held file.
const HELD_FILE_RETRY: Duration = Duration::from_millis(500);

/// The suffix of a snapshot's temporary file, beside the file it replaces.
const PARTIAL_SUFFIX: &str = "vsift-partial";

/// The outputs of one run, claimed in one folder.
#[derive(Debug)]
pub struct WatchClaim {
    transcript_path: PathBuf,
    subtitles_path: PathBuf,
    folder_path: PathBuf,
    /// Whether this claim took over the outputs of an interrupted run of
    /// the same job (ADR 0037 section 8): frames already in the folder are
    /// then kept, and one with the same name and digest is accepted.
    taken_over: bool,
    created_in_folder: Vec<PathBuf>,
}

impl WatchClaim {
    /// Claims the three outputs in `folder`, in the order transcript,
    /// subtitles, folder. If one is taken or refused, the ones this call
    /// created are removed again and nothing else is touched. The two files
    /// are created empty and closed: every later write replaces them whole.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Exists`] for a taken name,
    /// [`WatchOutputError::Refused`] when the folder refuses the write.
    pub fn claim(folder: &Path, names: &WatchOutputNames) -> Result<Self, WatchOutputError> {
        let transcript_path = folder.join(names.transcript());
        let subtitles_path = folder.join(names.subtitles());
        let folder_path = folder.join(names.folder());
        drop(create_new(&transcript_path, WatchOutputKind::Transcript)?);
        if let Err(error) = create_new(&subtitles_path, WatchOutputKind::Subtitles) {
            remove_if_empty(&transcript_path);
            return Err(error);
        }
        if let Err(error) = fs::create_dir(&folder_path) {
            remove_if_empty(&transcript_path);
            remove_if_empty(&subtitles_path);
            return Err(classify(&error, WatchOutputKind::Folder));
        }
        Ok(Self {
            transcript_path,
            subtitles_path,
            folder_path,
            taken_over: false,
            created_in_folder: Vec::new(),
        })
    }

    /// Takes over the outputs an interrupted run of the same job claimed,
    /// which its job's record names: each must still be what that run
    /// created, a regular file or a folder, never a link.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Exists`] for an output that is no longer one.
    pub fn take_over(claimed: &ClaimedOutputs) -> Result<Self, WatchOutputError> {
        let is = |path: &Path, folder: bool| {
            fs::symlink_metadata(path).is_ok_and(|metadata| {
                !metadata.file_type().is_symlink()
                    && if folder {
                        metadata.is_dir()
                    } else {
                        metadata.is_file()
                    }
            })
        };
        for (path, folder, kind) in [
            (&claimed.transcript, false, WatchOutputKind::Transcript),
            (&claimed.subtitles, false, WatchOutputKind::Subtitles),
            (&claimed.folder, true, WatchOutputKind::Folder),
        ] {
            if !is(path, folder) {
                return Err(WatchOutputError::Exists(kind));
            }
        }
        Ok(Self {
            transcript_path: claimed.transcript.clone(),
            subtitles_path: claimed.subtitles.clone(),
            folder_path: claimed.folder.clone(),
            taken_over: true,
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

    /// Replaces the readable transcript with `text` (see
    /// [`replace_claimed`]); the SHA-256 of what it wrote. A file another
    /// program holds open is tried for up to `wait`.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Refused`] when the write fails, and
    /// [`WatchOutputError::Held`] when the file stayed held for `wait`.
    pub fn write_transcript(
        &mut self,
        text: &str,
        random: &str,
        wait: Duration,
    ) -> Result<String, WatchOutputError> {
        replace_claimed(
            &self.transcript_path,
            text.as_bytes(),
            random,
            wait,
            WatchOutputKind::Transcript,
        )
    }

    /// Replaces the subtitles with `text`, as
    /// [`WatchClaim::write_transcript`] does.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Refused`] or [`WatchOutputError::Held`].
    pub fn write_subtitles(
        &mut self,
        text: &str,
        random: &str,
        wait: Duration,
    ) -> Result<String, WatchOutputError> {
        replace_claimed(
            &self.subtitles_path,
            text.as_bytes(),
            random,
            wait,
            WatchOutputKind::Subtitles,
        )
    }

    /// Removes the claimed subtitles file, still empty, for a video with no
    /// sound track (ADR 0037 section 4).
    pub fn discard_subtitles(&mut self) {
        remove_if_empty(&self.subtitles_path);
    }

    /// Copies a frame out of the session into the folder as `file_name`
    /// (created exclusively) and checks the copy against `sha256`. A claim
    /// that took over an interrupted run accepts a frame already there with
    /// that name and digest: `frame get` gave the same frame again.
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
        match self.write_in_folder(file_name, &bytes) {
            Err(WatchOutputError::Exists(_))
                if self.taken_over && self.holds_frame(file_name, sha256) =>
            {
                Ok(self.folder_path.join(file_name))
            }
            written => written,
        }
    }

    /// Whether the folder holds `file_name` as a regular file with `sha256`.
    fn holds_frame(&self, file_name: &str, sha256: &str) -> bool {
        let path = self.folder_path.join(file_name);
        let regular = fs::symlink_metadata(&path)
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink());
        regular && fs::read(&path).is_ok_and(|bytes| hex_sha256(&bytes) == sha256)
    }

    /// Writes the screen-change list into the folder, replacing the one an
    /// interrupted run of the same job wrote; its path and the SHA-256 of
    /// what it wrote.
    ///
    /// # Errors
    ///
    /// [`WatchOutputError::Refused`] or [`WatchOutputError::Held`].
    pub fn write_screen_changes(
        &mut self,
        text: &str,
        random: &str,
    ) -> Result<(PathBuf, String), WatchOutputError> {
        let path = self.folder_path.join(SCREEN_CHANGES_FILE);
        let created = !self.taken_over && !self.created_in_folder.contains(&path);
        let sha256 = replace_claimed(
            &path,
            text.as_bytes(),
            random,
            HELD_FILE_WAIT,
            WatchOutputKind::FolderContent,
        )?;
        if created {
            self.created_in_folder.push(path.clone());
        }
        Ok((path, sha256))
    }

    /// Gives the claim up after a failure: removes every file this claim
    /// created in its folder, the folder once it is empty, and the two
    /// files while they are still empty. A file that holds a write stays.
    pub fn release(mut self) {
        for path in self.created_in_folder.drain(..) {
            // Best effort: a file that cannot be removed stays, visible to the
            // person, and never hides a failure that is being reported.
            let _ignored = fs::remove_file(path);
        }
        // What an interrupted run created stays: the job that claimed it is
        // still unfinished, and a later rerun continues it.
        if !self.taken_over {
            let _ignored = fs::remove_dir(&self.folder_path);
            remove_if_empty(&self.transcript_path);
            remove_if_empty(&self.subtitles_path);
        }
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

/// Replaces the claimed file at `target` with `bytes` (ADR 0037 section 7,
/// the planner's condition B): the bytes go to an exclusive temporary file
/// in the same folder, `.<name>.<random>.vsift-partial`, created new (so an
/// existing name or a link planted there fails the write rather than being
/// written through), flushed, then moved over `target`, the claimed name and
/// never a name read back from the folder. A reader or a crash sees the old
/// file or the new one, whole.
///
/// A file another program holds open (Windows refuses the move) is tried
/// again every half second for up to `wait`; the temporary file is removed
/// whatever happens.
///
/// # Errors
///
/// [`WatchOutputError::Refused`] when the temporary file cannot be written or
/// the move fails, [`WatchOutputError::Held`] when the file stayed held.
pub fn replace_claimed(
    target: &Path,
    bytes: &[u8],
    random: &str,
    wait: Duration,
    kind: WatchOutputKind,
) -> Result<String, WatchOutputError> {
    let refused = |error: io::ErrorKind| WatchOutputError::Refused(kind, error);
    let folder = target.parent().ok_or(refused(io::ErrorKind::NotFound))?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(refused(io::ErrorKind::InvalidInput))?;
    let partial = folder.join(format!(".{name}.{random}.{PARTIAL_SUFFIX}"));
    let mut file = create_new(&partial, kind)?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| classify(&error, kind));
    drop(file);
    let moved = written.and_then(|()| move_over(&partial, target, wait, kind));
    if moved.is_err() {
        let _ignored = fs::remove_file(&partial);
    }
    moved.map(|()| hex_sha256(bytes))
}

/// Moves `partial` over `target`, trying a held target again for `wait`.
fn move_over(
    partial: &Path,
    target: &Path,
    wait: Duration,
    kind: WatchOutputKind,
) -> Result<(), WatchOutputError> {
    let started = Instant::now();
    loop {
        match fs::rename(partial, target) {
            Ok(()) => return Ok(()),
            Err(error) if is_held(&error) => {
                if started.elapsed() >= wait {
                    return Err(WatchOutputError::Held(kind));
                }
                std::thread::sleep(HELD_FILE_RETRY.min(wait));
            }
            Err(error) => return Err(WatchOutputError::Refused(kind, error.kind())),
        }
    }
}

/// Whether a move failed because another program holds the target open:
/// Windows reports a sharing violation (32) or a lock (33), or access
/// denied for a file opened without delete sharing.
fn is_held(error: &io::Error) -> bool {
    cfg!(windows)
        && (error.kind() == io::ErrorKind::PermissionDenied
            || matches!(error.raw_os_error(), Some(32 | 33)))
}

/// The folder implementation of the [`WatchOutputs`] port: resolves the
/// folder, claims the outputs there with a [`WatchClaim`] and renders the
/// files with this module's writers.
pub struct FolderWatchOutputs {
    vsift_roots: Vec<PathBuf>,
    identifiers: Box<dyn IdentifierSource>,
    held_wait: Duration,
    claim: Option<WatchClaim>,
}

impl fmt::Debug for FolderWatchOutputs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FolderWatchOutputs")
            .field("vsift_roots", &self.vsift_roots)
            .field("claim", &self.claim)
            .finish_non_exhaustive()
    }
}

impl FolderWatchOutputs {
    /// Outputs that refuse any folder inside `vsift_roots`, the canonical
    /// session and managed roots, which `VSift` cleans automatically. The
    /// temporary files' random parts come from the operating system's
    /// randomness, through the identifier source.
    #[must_use]
    pub fn new(vsift_roots: Vec<PathBuf>) -> Self {
        Self {
            vsift_roots,
            identifiers: Box::new(RandomIdentifierSource),
            held_wait: HELD_FILE_WAIT,
            claim: None,
        }
    }

    fn claimed(&mut self) -> Result<&mut WatchClaim, WatchOutputError> {
        self.claim.as_mut().ok_or(WatchOutputError::Refused(
            WatchOutputKind::FolderContent,
            io::ErrorKind::NotFound,
        ))
    }

    /// A fresh random part for a temporary file's name.
    fn random(&self, kind: WatchOutputKind) -> Result<String, WatchOutputError> {
        let operation = self
            .identifiers
            .operation_id()
            .map_err(|_| WatchOutputError::Refused(kind, io::ErrorKind::Other))?;
        Ok(operation
            .as_str()
            .trim_start_matches("op_")
            .chars()
            .take(16)
            .collect())
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

    fn planned(
        &mut self,
        video: &Path,
        output: Option<&Path>,
        names: &WatchOutputNames,
    ) -> Result<ClaimedOutputs, WatchOutputError> {
        let folder = output_folder(video, output, &self.vsift_roots)?;
        Ok(ClaimedOutputs {
            transcript: folder.join(names.transcript()),
            subtitles: folder.join(names.subtitles()),
            folder: folder.join(names.folder()),
        })
    }

    fn reclaim(&mut self, claimed: &ClaimedOutputs) -> Result<(), WatchOutputError> {
        self.claim = Some(WatchClaim::take_over(claimed)?);
        Ok(())
    }

    fn write_snapshot(&mut self, snapshot: &WatchSnapshot) -> Result<(), WatchOutputError> {
        let transcript_random = self.random(WatchOutputKind::Transcript)?;
        let subtitles_random = self.random(WatchOutputKind::Subtitles)?;
        let claim = self.claimed()?;
        // One try each: a file another program holds is skipped until the
        // next snapshot (known limit L-237).
        claim.write_transcript(
            &render_transcript_snapshot(snapshot),
            &transcript_random,
            Duration::ZERO,
        )?;
        claim.write_subtitles(
            &render_subrip_snapshot(snapshot),
            &subtitles_random,
            Duration::ZERO,
        )?;
        Ok(())
    }

    fn write_screen_changes(
        &mut self,
        provenance: &WatchProvenance,
        summary: &ScreenChangeSummary,
        changes: &[ScreenChangeEntry],
    ) -> Result<WatchFile, WatchOutputError> {
        let random = self.random(WatchOutputKind::FolderContent)?;
        let (path, sha256) = self.claimed()?.write_screen_changes(
            &render_screen_changes(provenance, summary, changes),
            &random,
        )?;
        Ok(WatchFile {
            path,
            sha256: Some(sha256),
        })
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
        let transcript_random = self.random(WatchOutputKind::Transcript)?;
        let subtitles_random = self.random(WatchOutputKind::Subtitles)?;
        let wait = self.held_wait;
        let claim = self.claimed()?;
        let revision = documents
            .transcript_revision
            .as_ref()
            .map(vsift_domain::TranscriptRevisionId::as_str);
        let transcript_sha256 = claim.write_transcript(
            &render_transcript(&documents.provenance, revision, &documents.entries),
            &transcript_random,
            wait,
        )?;
        let subtitles = if revision.is_some() {
            let sha256 = claim.write_subtitles(
                &render_subrip(&documents.entries),
                &subtitles_random,
                wait,
            )?;
            Some(WatchFile {
                path: claim.subtitles_path().to_path_buf(),
                sha256: Some(sha256),
            })
        } else {
            claim.discard_subtitles();
            None
        };
        let transcript = WatchFile {
            path: claim.transcript_path().to_path_buf(),
            sha256: Some(transcript_sha256),
        };
        // Finished: nothing this run wrote may be released any more.
        self.claim = None;
        Ok(FinalisedOutputs {
            transcript,
            subtitles,
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

/// Removes a file this run created, only while it is still empty, so a
/// release can never remove text that was written.
fn remove_if_empty(path: &Path) {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() == 0) {
        let _ignored = fs::remove_file(path);
    }
}

/// The SHA-256 of a file's bytes, read in pieces: how a rerun of
/// `vsift <video>` checks that an unfinished session holds this video.
///
/// # Errors
///
/// The read's error.
pub fn file_sha256(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 16];
    loop {
        let read = file.read(&mut buffer)?;
        let Some(piece) = buffer.get(..read).filter(|piece| !piece.is_empty()) else {
            break;
        };
        hash.update(piece);
    }
    let mut text = String::with_capacity(64);
    for byte in hash.finalize() {
        let _infallible = write!(text, "{byte:02x}");
    }
    Ok(text)
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
    let revision_line = match revision {
        Some(revision) => format!(
            "Transcript       {revision}   in session {} (closed when this run finished; the ids name it, the hashes can be checked any time)",
            provenance.session_id
        ),
        None => "Transcript       none: this video has no sound track".to_owned(),
    };
    transcript_text(provenance, &revision_line, entries)
}

/// Renders `<stem>.txt` while the job runs (ADR 0037 section 7): the
/// in-progress line first, then the header and what has been published.
#[must_use]
pub fn render_transcript_snapshot(snapshot: &WatchSnapshot) -> String {
    let mut text = match snapshot.duration {
        Some(duration) => format!(
            "IN PROGRESS: transcribed up to {} of {}. VSift is still working; this file is replaced when it finishes.\n",
            readable_time(snapshot.published_until),
            readable_time(duration)
        ),
        None => "IN PROGRESS: nothing transcribed yet. VSift is still working; this file is replaced when it finishes.\n".to_owned(),
    };
    let revision = snapshot.revision.as_ref().map_or(
        "not published yet",
        vsift_domain::TranscriptRevisionId::as_str,
    );
    let revision_line = format!(
        "Transcript       {revision}   in session {} (VSift is still working; the ids name it, the hashes can be checked any time)",
        snapshot.provenance.session_id
    );
    text.push_str(&transcript_text(
        &snapshot.provenance,
        &revision_line,
        &snapshot.entries,
    ));
    text
}

fn transcript_text(
    provenance: &WatchProvenance,
    revision_line: &str,
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
    let _ = writeln!(text, "{revision_line}");
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
    subrip_cues(entries).0
}

/// Renders `<stem>.srt` while the job runs (ADR 0037 section 7): the cues
/// published so far, then one cue from the published time to the end of the
/// video, so a person playing it sees where the text stops.
#[must_use]
pub fn render_subrip_snapshot(snapshot: &WatchSnapshot) -> String {
    let (mut text, number) = subrip_cues(&snapshot.entries);
    if let Some(duration) = snapshot.duration
        && snapshot.published_until < duration
    {
        let _ = write!(
            text,
            "{}\n{} --> {}\n{STILL_TRANSCRIBING}\n\n",
            number + 1,
            subrip_time(snapshot.published_until),
            subrip_time(duration)
        );
    }
    text
}

/// The trailing cue of the subtitles while the job runs.
const STILL_TRANSCRIBING: &str = "(VSift is still transcribing from here)";

/// The cues of the line entries, numbered from 1, and how many there are.
fn subrip_cues(entries: &[TranscriptEntry]) -> (String, usize) {
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
    (text, number)
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
            "Listed: the first frame, each screen once motion settles, and a change at least 5 s after the one listed before it; the others are left out, so a screen shown for less than 5 s after a listed one is missing unless motion settles on it.\n",
        );
        // The run closes its session, so the changes it left out are kept
        // nowhere: the list says how to find every one (the planner's
        // condition, 2026-10-28).
        let _ = writeln!(
            text,
            "To see every change, keep a session of the video: vsift ingest <video>, then vsift candidates <session> --from 0 --to {}, with the session id ingest gives.",
            summary.index_covers_to.as_micros()
        );
    }
    if frames < changes.len() {
        let _ = writeln!(
            text,
            "{frames} of {} frames written, spread evenly over the listed changes; the others are listed with no frame (evidence room).",
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
        "{:<10}  {:<20}  {:<24}  {:<36}  {:<36}  Frame SHA-256",
        "Time", "Reason", "Frame", "Change", "Frame evidence"
    );
    for change in changes {
        let (file, evidence, sha256) =
            change.frame.as_ref().map_or((NO_FRAME, "-", "-"), |frame| {
                (
                    frame.file_name.as_str(),
                    frame.evidence_id.as_str(),
                    frame.sha256.as_str(),
                )
            });
        let _ = writeln!(
            text,
            "{:<10}  {:<20}  {file:<24}  {:<36}  {evidence:<36}  {sha256}",
            readable_time(change.time),
            reason_words(change.reason),
            change.candidate_id
        );
    }
    text
}

/// The mark of a listed change without a frame: frames are bounded by the
/// session's evidence room (known limit L-234), whether the bound spread them
/// or the room stopped them.
const NO_FRAME: &str = "no frame (evidence room)";

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
