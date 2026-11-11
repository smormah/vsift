//! The result of `vsift <video>`, command `watch` (ADR 0037 section 9):
//! what the run wrote, where, and the ids and hashes every output names.
//!
//! Additive to JSON v1. Paths are delivered as `frame get` delivers its files
//! (the plain `C:\...` form on Windows whenever it names the same file);
//! `name` members are the file names in display form, hidden characters made
//! visible, because they are derived from the video's untrusted name.
//!
//! A partial run answers `status: partial` through the envelope's coverage,
//! as every other reader of the contract does (the planner's ruling on the
//! Fable 0.3.0 review, P-2, amending ADR 0037's implementation note of
//! 2026-10-28): exit 0 means complete or a supported partial, so a reader
//! that follows the exit rule alone never takes a gap for a whole result.

use std::path::Path;

use serde::Serialize;

use crate::{
    CommandName, CoverageResponse, OperationResponse,
    delivered_path::delivered_path_text,
    render_hidden_characters,
    search::{GAPS_TRUNCATED_REASON, MAX_COVERAGE_RANGES, UNTRANSCRIBED_REASON},
};

/// Envelope reason: the video runs past the first four hours, the part
/// screen changes are found in (L-235).
pub const WATCH_BEYOND_BOUND_REASON: &str = "screen_changes_beyond_bound";
/// Envelope reason: the session ran out of evidence room, so the frames
/// stopped early (L-234).
pub const WATCH_FRAMES_STOPPED_REASON: &str = "frames_stopped_for_room";
/// Envelope reason: other work held the machine's capacity while frames were
/// taken, so some selected changes have no frame (Fable review C-2).
pub const WATCH_FRAMES_SKIPPED_REASON: &str = "frames_skipped_for_capacity";
/// Envelope reason: the bundle `--keep` asked for could not be written.
pub const WATCH_NOT_KEPT_REASON: &str = "not_kept";

/// A file or folder the run wrote.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WatchOutputData {
    /// Its absolute path.
    pub path: String,
    /// Its name in display form.
    pub name: String,
    /// The SHA-256 of a file's bytes; `null` for the folder.
    pub sha256: Option<String>,
}

impl WatchOutputData {
    /// Describes `path`; `None` when the path is not Unicode text, which a
    /// claimed name never is (names are derived from Unicode names only).
    #[must_use]
    pub fn new(path: &Path, sha256: Option<String>) -> Option<Self> {
        let name = path.file_name()?.to_str()?;
        Some(Self {
            path: delivered_path_text(path)?,
            name: render_hidden_characters(name),
            sha256,
        })
    }
}

/// The run's outputs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WatchOutputsData {
    /// The readable transcript, `<stem>.txt`.
    pub transcript: WatchOutputData,
    /// The subtitles, `<stem>.srt`; `null` for a video with no sound track.
    pub subtitles: Option<WatchOutputData>,
    /// The screen-change folder, `<stem>-vsift`.
    pub folder: WatchOutputData,
    /// The screen-change list in the folder.
    pub screen_changes: WatchOutputData,
}

/// A frame written for a screen change.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WatchFrameData {
    /// The change's representative time, in microseconds.
    pub time_us: u64,
    /// The visual candidate (`vcd_`).
    pub candidate_id: String,
    /// The frame's evidence item (`evd_`), in the session the run closed.
    pub evidence_id: String,
    /// The frame file's absolute path.
    pub path: String,
    /// The PNG's SHA-256.
    pub sha256: String,
}

/// A half-open range of the video, in microseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct WatchRangeData {
    /// Inclusive start.
    pub from_us: u64,
    /// Exclusive end.
    pub to_us: u64,
}

/// What became of the bundle `--keep` asked for.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchKeptState {
    /// The bundle is committed at `path`.
    Kept,
    /// It could not be written; the session was left open.
    NotKept,
}

/// The bundle `--keep` asked for (ADR 0037, amendment of 2026-10-30).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WatchKeptData {
    /// Whether it is committed.
    pub state: WatchKeptState,
    /// Where it is, or where it was to go; absolute, as the outputs' paths.
    pub path: String,
    /// Its folder's name in display form, the name `bundle search` shows.
    pub name: String,
    /// The public code of the failure when `not_kept`; `null` when `kept`.
    pub failure_code: Option<String>,
}

impl WatchKeptData {
    /// Describes the bundle at `path`, kept unless `failure_code` says why
    /// not; `None` when the path is not Unicode text, which a name derived
    /// from a video's Unicode name never is.
    #[must_use]
    pub fn new(path: &Path, failure_code: Option<&str>) -> Option<Self> {
        let output = WatchOutputData::new(path, None)?;
        Some(Self {
            state: if failure_code.is_some() {
                WatchKeptState::NotKept
            } else {
                WatchKeptState::Kept
            },
            path: output.path,
            name: output.name,
            failure_code: failure_code.map(str::to_owned),
        })
    }
}

/// `data` of a failed `watch` whose job had sealed, with `--keep`: the error
/// says what failed, and this says what became of the bundle.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WatchFailureData {
    /// The session the run used.
    pub session_id: String,
    /// Whether the run closed it; never when the bundle was not written.
    pub session_closed: bool,
    /// The bundle.
    pub kept: WatchKeptData,
}

/// Whether everything the run set out to write is there.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchCompleteness {
    /// The whole video was transcribed (or has no sound track) and its screen
    /// changes found.
    Complete,
    /// Part of the video was not transcribed, runs past the part screen
    /// changes are found in, the frames stopped for room or some were skipped
    /// while the computer was busy, or the bundle `--keep` asked for could
    /// not be written; the data says which.
    Partial,
}

impl WatchCompleteness {
    /// The completeness of a run with these facts: `partial` when any of
    /// them leaves a gap. The envelope's coverage is derived by the same
    /// rule, so `data.outcome` and `status` never disagree.
    #[must_use]
    pub fn of(
        not_transcribed: &[WatchRangeData],
        screen_changes_beyond_bound: bool,
        frames: WatchFrameGaps,
        kept: Option<&WatchKeptData>,
    ) -> Self {
        if gap_reasons(not_transcribed, screen_changes_beyond_bound, frames, kept).is_empty() {
            Self::Complete
        } else {
            Self::Partial
        }
    }
}

/// Why selected changes have no frame: the two facts of [`WatchData`] the
/// gap rule reads about frames.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WatchFrameGaps {
    /// [`WatchData::frames_stopped_for_room`].
    pub stopped_for_room: bool,
    /// [`WatchData::frames_skipped_for_capacity`].
    pub skipped_for_capacity: u64,
}

/// The distinct reasons a run left a gap, in a fixed order.
fn gap_reasons(
    not_transcribed: &[WatchRangeData],
    screen_changes_beyond_bound: bool,
    frames: WatchFrameGaps,
    kept: Option<&WatchKeptData>,
) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if !not_transcribed.is_empty() {
        reasons.push(UNTRANSCRIBED_REASON);
    }
    if not_transcribed.len() > MAX_COVERAGE_RANGES {
        reasons.push(GAPS_TRUNCATED_REASON);
    }
    if screen_changes_beyond_bound {
        reasons.push(WATCH_BEYOND_BOUND_REASON);
    }
    if frames.stopped_for_room {
        reasons.push(WATCH_FRAMES_STOPPED_REASON);
    }
    if frames.skipped_for_capacity > 0 {
        reasons.push(WATCH_FRAMES_SKIPPED_REASON);
    }
    if kept.is_some_and(|kept| kept.state == WatchKeptState::NotKept) {
        reasons.push(WATCH_NOT_KEPT_REASON);
    }
    reasons
}

/// The envelope coverage of a run: truncated when it left any gap, with the
/// ranges not transcribed as `<from_us>-<to_us>` (the first 100) and the
/// distinct reasons.
fn envelope_coverage(data: &WatchData) -> CoverageResponse {
    let reasons = gap_reasons(
        &data.not_transcribed,
        data.screen_changes_beyond_bound,
        WatchFrameGaps {
            stopped_for_room: data.frames_stopped_for_room,
            skipped_for_capacity: data.frames_skipped_for_capacity,
        },
        data.kept.as_ref(),
    );
    CoverageResponse::new(
        !reasons.is_empty(),
        data.not_transcribed
            .iter()
            .take(MAX_COVERAGE_RANGES)
            .map(|range| format!("{}-{}", range.from_us, range.to_us))
            .collect(),
        reasons.into_iter().map(str::to_owned).collect(),
    )
}

/// The `watch` result: `complete`, or `partial` (exit 0 either way) when the
/// run left a gap, with the envelope's coverage saying which; a bundle that
/// could not be kept adds [`WATCH_NOT_KEPT_WARNING`].
///
/// # Errors
///
/// Returns the serialization error when the data cannot be represented as JSON.
pub fn watch_response(
    data: &WatchData,
) -> Result<OperationResponse<serde_json::Value>, serde_json::Error> {
    let not_kept = data
        .kept
        .as_ref()
        .is_some_and(|kept| kept.state == WatchKeptState::NotKept);
    let response = OperationResponse::complete(CommandName::Watch.identifier(), data)?
        .with_coverage(envelope_coverage(data));
    Ok(if not_kept {
        response.with_warnings(&[WATCH_NOT_KEPT_WARNING])
    } else {
        response
    })
}

/// `data` of a `watch` result.
#[allow(clippy::struct_excessive_bools)] // Independent facts of the published JSON v1 shape, not switches.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WatchData {
    /// The video's content identity, `src_sha256_` and its SHA-256.
    pub source_id: String,
    /// The session the run used.
    pub session_id: String,
    /// Whether the run closed it.
    pub session_closed: bool,
    /// The session's `prepare` job.
    pub job_id: String,
    /// The video's length, in microseconds.
    pub duration_us: u64,
    /// The transcript revision (`trv_`); `null` for a video with no sound
    /// track.
    pub transcript_revision_id: Option<String>,
    /// The visual index revision (`vix_`); `null` for a video with no
    /// picture.
    pub visual_index_revision_id: Option<String>,
    /// Whether the outputs are complete.
    pub outcome: WatchCompleteness,
    /// The ranges that were not transcribed, in time order.
    pub not_transcribed: Vec<WatchRangeData>,
    /// Whether the video runs past the first four hours, the part screen
    /// changes are found in (known limit L-235).
    pub screen_changes_beyond_bound: bool,
    /// Whether the session ran out of evidence room before every selected
    /// change had its frame, so the frames stopped early (known limit L-234).
    pub frames_stopped_for_room: bool,
    /// How many selected changes have no frame because other work held the
    /// machine's capacity when their frame was taken; each is listed without
    /// one (Fable review C-2). Zero for a run that took every frame it chose.
    pub frames_skipped_for_capacity: u64,
    /// Whether the names were shortened, so the subtitles no longer share the
    /// video's name.
    pub names_shortened: bool,
    /// The run's outputs.
    pub outputs: WatchOutputsData,
    /// How many screen changes the visual stream found.
    pub changes_found: u64,
    /// How many of them the screen-change list shows: the settled screens
    /// (the first frame, each screen once motion settles, and a change at
    /// least 5 s after the one listed before it).
    pub changes_listed: u64,
    /// The frames written, at most 180 (known limit L-234).
    pub frames: Vec<WatchFrameData>,
    /// The bundle `--keep` asked for; `null` without the option.
    pub kept: Option<WatchKeptData>,
}

/// Remediation when the readable transcript's name is taken.
pub const WATCH_TRANSCRIPT_EXISTS_REMEDIATION: &str = "The transcript file (.txt) VSift would write beside the video already exists, and VSift never replaces a file. Move it away, or write elsewhere with --output <folder>. A file left by a stopped VSift run is continued by the same command only while VSift's private copy of the video is kept (a day unused); after that, move it away too.";
/// Remediation when the subtitles' name is taken.
pub const WATCH_SUBTITLES_EXISTS_REMEDIATION: &str = "The subtitles file (.srt) VSift would write beside the video already exists, and VSift never replaces a file. Move it away, or write elsewhere with --output <folder>. A file left by a stopped VSift run is continued by the same command only while VSift's private copy of the video is kept (a day unused); after that, move it away too.";
/// Remediation when the screen-change folder's name is taken.
pub const WATCH_FOLDER_EXISTS_REMEDIATION: &str = "The screen-change folder (-vsift) VSift would write beside the video already exists, and VSift never replaces it. Move it away, or write elsewhere with --output <folder>. A file left by a stopped VSift run is continued by the same command only while VSift's private copy of the video is kept (a day unused); after that, move it away too.";
/// Remediation when a file was held open by another program, or was not
/// writable, for the final write's 30 s (known limit L-237).
pub const WATCH_OUTPUT_HELD_REMEDIATION: &str = "A file VSift was writing was held open by another program, or not writable, so VSift could not finish it; the file keeps its last update. Close the file in the other program (or make it writable), then write the files again elsewhere with --output <folder>.";
/// Remediation when another `VSift` run is writing the same files right now:
/// its job is owned by a live process, so this run touched nothing (ADR 0037,
/// implementation note of 2026-11-10).
pub const WATCH_RUN_IN_PROGRESS_REMEDIATION: &str = "Another VSift run is writing these files right now, and VSift never takes over a run that is still working; nothing was changed. Wait for it to finish, or stop it and run the same command again to continue it, or write elsewhere with --output <folder>.";
/// Remediation when the person stopped the run (Ctrl-C): the files keep
/// their last update and the same command continues (ADR 0037 section 8).
pub const WATCH_STOPPED_REMEDIATION: &str = "VSift stopped. The files keep what was transcribed so far, marked IN PROGRESS, and VSift kept its private copy of the video: run the same command again to continue.";
/// Remediation when the folder refuses a write.
pub const WATCH_OUTPUT_REFUSED_REMEDIATION: &str = "VSift could not write its files into the folder (it may be read-only, full, or a synced folder that refuses new files). Choose a folder you can write to with --output <folder>.";
/// Remediation when `--output` is inside `VSift`'s own folders.
pub const WATCH_OUTPUT_INSIDE_VSIFT_REMEDIATION: &str = "--output names a folder inside VSift's own folders, which VSift cleans automatically. Choose another folder.";
/// Remediation when `--output` cannot be used as a folder.
pub const WATCH_OUTPUT_FOLDER_REMEDIATION: &str =
    "--output takes a folder: an existing one, or a new one whose parent folder exists.";
/// Remediation when the video's name is not Unicode text.
pub const WATCH_NAME_NOT_UNICODE_REMEDIATION: &str = "The video's file name is not Unicode text, so VSift cannot name its files after it. Rename the video, then run it again.";
/// Remediation when the video's name has nothing before its extension.
pub const WATCH_NAME_EMPTY_REMEDIATION: &str = "The video's file name has nothing before its extension, so VSift cannot name its files after it. Rename the video, then run it again.";
/// Remediation when the bundle's name is taken under the `--keep` folder.
pub const WATCH_KEEP_NAME_TAKEN_REMEDIATION: &str = "A recording of this name is already kept in that folder, and VSift never replaces it. Keep this one in another folder with --keep <folder>, or rename the video.";
/// Remediation when `--keep` cannot be used as a folder.
pub const WATCH_KEEP_FOLDER_REMEDIATION: &str = "--keep takes a folder on a local drive: an existing one, or a new one whose parent folder exists.";
/// Remediation when `--keep` is inside `VSift`'s own folders.
pub const WATCH_KEEP_INSIDE_VSIFT_REMEDIATION: &str = "--keep names a folder inside VSift's own folders, which VSift cleans automatically. Choose another folder.";
/// Remediation when the video's name cannot name a bundle's folder.
pub const WATCH_KEEP_NAME_NOT_ALLOWED_REMEDIATION: &str = "The video's name is one a kept recording's folder cannot have on every system (a Windows device name). Rename the video, then run it again.";
/// Remediation when the `--keep` folder refuses the check.
pub const WATCH_KEEP_REFUSED_REMEDIATION: &str = "VSift could not write a private folder into that folder (it may be read-only, full, or not able to hold private folders). Choose a folder you can write to with --keep <folder>.";
/// The warning when the bundle could not be written; the outputs stand. The
/// session's id is in the result's `session_id`.
pub const WATCH_NOT_KEPT_WARNING: &str = "VSift could not keep the recording for later search, so it left the session open, with its private copy of the video, until it expires about a day unused. Keep it now with: vsift session retain <session_id> --output <a new folder>. Anything left at the kept name is incomplete and can be removed.";
/// Added to a failure after the job sealed, when the bundle was written.
pub const WATCH_KEPT_AFTER_FAILURE_REMEDIATION: &str = "The transcription was finished, so the recording was still kept for later search, as you asked.";

/// Remediation when a copied frame does not match its digest.
pub const WATCH_FRAME_MISMATCH_REMEDIATION: &str = "A frame copied out of VSift's private copy did not match its recorded SHA-256, so the run kept nothing. Run it again; if it happens again, report it.";
