//! The values of `vsift <video>` (ADR 0037): the names of its outputs, the
//! readable times it writes, which screen changes get a frame, and the rule
//! that keeps a subtitle cue's text from forging another cue.
//!
//! Everything here is pure and deterministic. The video's file name is
//! untrusted input: it is only ever extended by a fixed suffix, shortened at a
//! character boundary, or refused, never rewritten character by character.

use std::fmt;

use crate::{CandidateReason, MediaTime};

/// The most bytes an output's file name may take, suffix included.
///
/// Conservative for every system `VSift` runs on: Linux and macOS allow 255
/// bytes in a name, Windows 255 UTF-16 units, and a UTF-8 string never has
/// fewer bytes than UTF-16 units. The margin leaves room for the temporary
/// names the growing files use (ADR 0037 section 7).
pub const MAX_OUTPUT_NAME_BYTES: usize = 200;

/// The most frames one run writes (ADR 0037 section 6, known limit L-234).
///
/// Each frame is a `frame get`, which stores two evidence artifacts in the
/// session (the PNG and its record), and a session holds at most 384: 180
/// frames take 360 of them. A run still writes fewer when the session has
/// less room ([`frames_that_fit`]).
pub const MAX_WATCH_FRAMES: usize = 180;

/// The evidence artifacts one frame takes in the session: its PNG and its
/// record.
pub const EVIDENCE_ARTIFACTS_PER_FRAME: usize = 2;

/// The readable transcript's suffix.
const TRANSCRIPT_SUFFIX: &str = ".txt";
/// The subtitles' suffix.
const SUBTITLES_SUFFIX: &str = ".srt";
/// The screen-change folder's suffix.
const FOLDER_SUFFIX: &str = "-vsift";

/// Why no output names can be derived from a video's file name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputNameError {
    /// The file name is not Unicode text, so a name derived from it could not
    /// be shown or checked; the video has to be renamed.
    NotUnicode,
    /// Nothing is left of the name once its extension and any trailing dots
    /// and spaces are removed (for example `.mp4`).
    Empty,
}

impl fmt::Display for OutputNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotUnicode => "the video's file name is not Unicode text",
            Self::Empty => "the video's file name has nothing before its extension",
        })
    }
}

impl std::error::Error for OutputNameError {}

/// The three names one run writes, all in one folder (ADR 0037 section 4):
/// `<stem>.txt`, `<stem>.srt` and `<stem>-vsift`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchOutputNames {
    stem: String,
    shortened: bool,
}

impl WatchOutputNames {
    /// Derives the names from the video's file name (its last component,
    /// without any folder).
    ///
    /// The stem is the name without its last extension, with trailing dots
    /// and spaces removed (Windows drops them from a name, so a stem ending
    /// in one would name another file there). When the longest output name
    /// would exceed [`MAX_OUTPUT_NAME_BYTES`], the stem is shortened at a
    /// character boundary.
    ///
    /// # Errors
    ///
    /// [`OutputNameError`] when the name is not Unicode or leaves no stem.
    pub fn from_video_name(file_name: &std::ffi::OsStr) -> Result<Self, OutputNameError> {
        let name = file_name.to_str().ok_or(OutputNameError::NotUnicode)?;
        let stem = match name.rfind('.') {
            Some(dot) if dot > 0 => &name[..dot],
            _ => name,
        };
        let stem = stem.trim_end_matches(['.', ' ']);
        if stem.is_empty() {
            return Err(OutputNameError::Empty);
        }
        let room = MAX_OUTPUT_NAME_BYTES - FOLDER_SUFFIX.len();
        let shortened = stem.len() > room;
        let stem = if shortened {
            let mut end = room;
            while !stem.is_char_boundary(end) {
                end -= 1;
            }
            stem[..end].trim_end_matches(['.', ' '])
        } else {
            stem
        };
        if stem.is_empty() {
            return Err(OutputNameError::Empty);
        }
        Ok(Self {
            stem: stem.to_owned(),
            shortened,
        })
    }

    /// The readable transcript's name, `<stem>.txt`.
    #[must_use]
    pub fn transcript(&self) -> String {
        format!("{}{TRANSCRIPT_SUFFIX}", self.stem)
    }

    /// The subtitles' name, `<stem>.srt`.
    #[must_use]
    pub fn subtitles(&self) -> String {
        format!("{}{SUBTITLES_SUFFIX}", self.stem)
    }

    /// The screen-change folder's name, `<stem>-vsift`.
    #[must_use]
    pub fn folder(&self) -> String {
        format!("{}{FOLDER_SUFFIX}", self.stem)
    }

    /// Whether the stem was shortened, so the subtitles no longer share the
    /// video's name and a player will not pick them up by itself.
    #[must_use]
    pub const fn shortened(&self) -> bool {
        self.shortened
    }
}

/// A time as a person reads it: `HH:MM:SS`, whole seconds rounded down.
#[must_use]
pub fn readable_time(time: MediaTime) -> String {
    let seconds = time.as_micros() / 1_000_000;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    )
}

/// A time as `SubRip` writes it: `HH:MM:SS,mmm`, milliseconds rounded down.
#[must_use]
pub fn subrip_time(time: MediaTime) -> String {
    let millis = time.as_micros() / 1000;
    format!(
        "{},{:03}",
        readable_time(MediaTime::from_micros(time.as_micros())),
        millis % 1000
    )
}

/// A frame's file name: its time as `HH-MM-SS.mmm.png`, which sorts in time
/// order and holds no `:`, which Windows refuses in a name.
#[must_use]
pub fn frame_file_name(time: MediaTime) -> String {
    let millis = time.as_micros() / 1000;
    let seconds = millis / 1000;
    format!(
        "{:02}-{:02}-{:02}.{:03}.png",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60,
        millis % 1000
    )
}

/// How many frames a run may take from a session with `evidence_slots_left`
/// evidence slots: as many as fit, never more than [`MAX_WATCH_FRAMES`]. The
/// room is measured before the first frame, so a run never asks the session
/// for more than it holds (the real-media run's RM03, 576 changes, failed on
/// its 193rd frame with a bound of 200).
#[must_use]
pub const fn frames_that_fit(evidence_slots_left: usize) -> usize {
    let fit = evidence_slots_left / EVIDENCE_ARTIFACTS_PER_FRAME;
    if fit < MAX_WATCH_FRAMES {
        fit
    } else {
        MAX_WATCH_FRAMES
    }
}

/// Which of `count` screen changes, in time order, get a frame: all of them
/// up to `bound`; above it, `bound` of them spread evenly from the first to
/// the last (ADR 0037 section 6). The positions are ascending and distinct.
#[must_use]
pub fn spread_evenly(count: usize, bound: usize) -> Vec<usize> {
    if count <= bound {
        return (0..count).collect();
    }
    match bound {
        0 => Vec::new(),
        1 => vec![0],
        _ => (0..bound)
            .map(|step| step * (count - 1) / (bound - 1))
            .collect(),
    }
}

/// How long after the last listed screen a plain visual change must come to
/// be listed itself (ADR 0037, the settled-screens amendment): 5 s.
pub const SETTLED_SCREEN_SPACING_MICROS: u64 = 5_000_000;

/// Which of the screen changes, given in time order with their reasons, the
/// list shows: the settled screens (ADR 0037, the settled-screens amendment).
///
/// A first frame and each screen after motion settles are always listed; a
/// plain visual change is listed only when it comes at least
/// [`SETTLED_SCREEN_SPACING_MICROS`] after the last listed entry, whatever
/// that entry's reason; the start of motion and periodic coverage are never
/// listed, because a person reads the screen once it holds still. Footage
/// with a moving camera proposes a change every two seconds or so (the
/// real-media run's RM03, a talk, had 576 in 19 minutes), most of them
/// motion, so the full list hides the screens a person would look at. The
/// positions are ascending and distinct.
#[must_use]
pub fn settled_screens(changes: &[(MediaTime, CandidateReason)]) -> Vec<usize> {
    let mut listed = Vec::new();
    let mut last: Option<MediaTime> = None;
    for (position, &(time, reason)) in changes.iter().enumerate() {
        let keep = match reason {
            CandidateReason::FirstFrame | CandidateReason::SettledAfterMotion => true,
            CandidateReason::VisualChange => last.is_none_or(|last| {
                time.as_micros().saturating_sub(last.as_micros()) >= SETTLED_SCREEN_SPACING_MICROS
            }),
            CandidateReason::MotionStart | CandidateReason::PeriodicCoverage => false,
        };
        if keep {
            listed.push(position);
            last = Some(time);
        }
    }
    listed
}

/// A subtitle cue's text from a segment's display text: one line, holding
/// neither a blank line, which would end the cue, nor the `-->` arrow, which
/// would read as a timing line. Line breaks become spaces and the arrow
/// becomes `->`, so a segment's text can never forge a cue.
#[must_use]
pub fn subrip_cue_text(display_text: &str) -> String {
    let single_line = display_text
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut text = single_line;
    while text.contains("-->") {
        text = text.replace("-->", "->");
    }
    text
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    fn names(name: &str) -> Result<WatchOutputNames, OutputNameError> {
        WatchOutputNames::from_video_name(OsStr::new(name))
    }

    #[test]
    fn names_take_the_stem_and_a_fixed_suffix() -> Result<(), OutputNameError> {
        let derived = names("meeting.mp4")?;
        assert_eq!(derived.transcript(), "meeting.txt");
        assert_eq!(derived.subtitles(), "meeting.srt");
        assert_eq!(derived.folder(), "meeting-vsift");
        assert!(!derived.shortened());
        assert_eq!(
            names("team call.2026.webm")?.transcript(),
            "team call.2026.txt"
        );
        assert_eq!(names("recording")?.subtitles(), "recording.srt");
        assert_eq!(names(".hidden")?.folder(), ".hidden-vsift");
        Ok(())
    }

    #[test]
    fn trailing_dots_and_spaces_are_removed() -> Result<(), OutputNameError> {
        assert_eq!(names("demo. .mp4")?.transcript(), "demo.txt");
        assert_eq!(names(".mp4").map(|_| ()), Ok(()));
        assert_eq!(names(". .mp4"), Err(OutputNameError::Empty));
        assert_eq!(names("..."), Err(OutputNameError::Empty));
        Ok(())
    }

    #[test]
    fn a_long_stem_is_shortened_at_a_character_boundary() -> Result<(), OutputNameError> {
        let long = format!("{}.mp4", "é".repeat(150));
        let derived = names(&long)?;
        assert!(derived.shortened());
        for name in [derived.transcript(), derived.subtitles(), derived.folder()] {
            assert!(name.len() <= MAX_OUTPUT_NAME_BYTES, "{name}");
        }
        assert!(derived.folder().starts_with('é'));
        Ok(())
    }

    #[test]
    fn unicode_is_kept_as_it_is() -> Result<(), OutputNameError> {
        assert_eq!(names("Встреча 会議.mkv")?.transcript(), "Встреча 会議.txt");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_name_that_is_not_unicode_is_refused() {
        use std::os::unix::ffi::OsStrExt as _;
        let name = OsStr::from_bytes(&[0x66, 0xff, 0x2e, 0x6d, 0x70, 0x34]);
        assert_eq!(
            WatchOutputNames::from_video_name(name),
            Err(OutputNameError::NotUnicode)
        );
    }

    #[test]
    fn times_are_written_as_people_and_players_read_them() {
        let time = MediaTime::from_micros(3_754_480_999);
        assert_eq!(readable_time(time), "01:02:34");
        assert_eq!(subrip_time(time), "01:02:34,480");
        assert_eq!(frame_file_name(time), "01-02-34.480.png");
        assert_eq!(readable_time(MediaTime::from_micros(0)), "00:00:00");
        assert_eq!(
            readable_time(MediaTime::from_micros(100 * 3_600_000_000)),
            "100:00:00"
        );
    }

    #[test]
    fn frames_are_spread_evenly_above_the_bound() {
        assert_eq!(spread_evenly(3, 200), vec![0, 1, 2]);
        assert_eq!(spread_evenly(0, 200), Vec::<usize>::new());
        assert_eq!(spread_evenly(10, 4), vec![0, 3, 6, 9]);
        assert_eq!(spread_evenly(10, 1), vec![0]);
        let spread = spread_evenly(340, MAX_WATCH_FRAMES);
        assert_eq!(spread.len(), MAX_WATCH_FRAMES);
        assert_eq!(spread.first(), Some(&0));
        assert_eq!(spread.last(), Some(&339));
        assert!(spread.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn the_frames_never_need_more_room_than_the_session_has() {
        assert_eq!(frames_that_fit(384), MAX_WATCH_FRAMES);
        assert_eq!(frames_that_fit(usize::MAX), MAX_WATCH_FRAMES);
        assert_eq!(frames_that_fit(201), 100);
        assert_eq!(frames_that_fit(1), 0);
        assert_eq!(frames_that_fit(0), 0);
        for slots in 0..=400 {
            assert!(frames_that_fit(slots) * EVIDENCE_ARTIFACTS_PER_FRAME <= slots);
        }
    }

    #[test]
    fn cue_text_can_never_forge_a_cue() {
        assert_eq!(
            subrip_cue_text("one\n\n2\n00:00:01,000 --> 00:00:02,000\nfake"),
            "one 2 00:00:01,000 -> 00:00:02,000 fake"
        );
        assert_eq!(subrip_cue_text("a ---> b"), "a -> b");
        assert_eq!(subrip_cue_text("  plain  "), "plain");
        assert!(!subrip_cue_text("x\r\n\r\ny").contains('\n'));
    }

    fn at(seconds: u64, reason: CandidateReason) -> (MediaTime, CandidateReason) {
        (MediaTime::from_micros(seconds * 1_000_000), reason)
    }

    #[test]
    fn settled_screens_keep_still_screens_and_spaced_changes() {
        use CandidateReason::{
            FirstFrame, MotionStart, PeriodicCoverage, SettledAfterMotion, VisualChange,
        };
        let changes = [
            at(0, FirstFrame),
            at(2, VisualChange),
            at(5, VisualChange),
            at(6, MotionStart),
            at(8, SettledAfterMotion),
            at(9, VisualChange),
            at(12, VisualChange),
            at(13, VisualChange),
            at(20, PeriodicCoverage),
            at(30, VisualChange),
        ];
        assert_eq!(
            settled_screens(&changes),
            vec![0, 2, 4, 7, 9],
            "a change 5 s after the last listed entry is listed, one sooner is not; motion's start and coverage never are"
        );
        assert_eq!(settled_screens(&[]), Vec::<usize>::new());
        assert_eq!(
            settled_screens(&[at(3, VisualChange), at(4, SettledAfterMotion)]),
            vec![0, 1],
            "a change with nothing listed before it is listed, and a settled screen always is"
        );
    }
}
