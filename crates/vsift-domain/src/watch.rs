//! The values of `vsift <video>` (ADR 0037): the names of its outputs, the
//! readable times it writes, which screen changes get a frame, and the rule
//! that keeps a subtitle cue's text from forging another cue.
//!
//! Everything here is pure and deterministic. The video's file name is
//! untrusted input: it is only ever extended by a fixed suffix, shortened at a
//! character boundary, or refused, never rewritten character by character.

use std::fmt;

use crate::MediaTime;

/// The most bytes an output's file name may take, suffix included.
///
/// Conservative for every system `VSift` runs on: Linux and macOS allow 255
/// bytes in a name, Windows 255 UTF-16 units, and a UTF-8 string never has
/// fewer bytes than UTF-16 units. The margin leaves room for the temporary
/// names the growing files use (ADR 0037 section 7).
pub const MAX_OUTPUT_NAME_BYTES: usize = 200;

/// The most frames one run writes (ADR 0037 section 6, known limit L-234).
pub const MAX_WATCH_FRAMES: usize = 200;

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
    fn cue_text_can_never_forge_a_cue() {
        assert_eq!(
            subrip_cue_text("one\n\n2\n00:00:01,000 --> 00:00:02,000\nfake"),
            "one 2 00:00:01,000 -> 00:00:02,000 fake"
        );
        assert_eq!(subrip_cue_text("a ---> b"), "a -> b");
        assert_eq!(subrip_cue_text("  plain  "), "plain");
        assert!(!subrip_cue_text("x\r\n\r\ny").contains('\n'));
    }
}
