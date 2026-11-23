//! The live view of `vsift <video>` while it works (ADR 0037 section 9): a
//! state goes in, text comes out.
//!
//! On a terminal that takes symbols and colour each stream has a bar and its
//! status word gains its symbol and colour; with `NO_COLOR` the same lines
//! are words only (ADR 0036 section 8). A status word is always present, so
//! meaning never depends on colour or on the bar:
//!
//! ```text
//! VSift  meeting.mp4  (48:00)
//!
//!   Screen changes   ████████████████   done, 14 found
//!   Speech           ███████░░░░░░░░░   12:30 of 48:00   about 9 min left
//!
//!   Writing as it goes: meeting.txt, meeting.srt (open them any time)
//! ```
//!
//! Without a terminal, or on one that cannot move its cursor, nothing is
//! redrawn: [`watch_stream_finished`] is the one plain line written when a
//! stream ends. Drawing, the redraw throttle and the engine's progress live
//! in `watch_lines`; nothing here reads a terminal or a clock.

use super::text::{DisplayText, Placement, RenderedText, Styling, TerminalText, Tone, TooLarge};
use crate::live_line::{clock, time_left};

/// The columns of the longest stream name, `Screen changes`, and the gap
/// after it.
const STREAM_NAME_COLUMNS: usize = 14 + 3;

/// The cells of a stream's bar.
const BAR_CELLS: u64 = 16;

/// Between a stream's figures and its time left.
const GAP: &str = "   ";

/// One of the run's two streams, as a person meets it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WatchStream {
    /// The visual stream: the screen changes found in the video.
    ScreenChanges,
    /// The transcript stream: the speech recognised.
    Speech,
}

impl WatchStream {
    /// Both streams, in the order the view lists them.
    pub(crate) const ALL: [Self; 2] = [Self::ScreenChanges, Self::Speech];

    const fn name(self) -> &'static str {
        match self {
            Self::ScreenChanges => "Screen changes",
            Self::Speech => "Speech",
        }
    }
}

/// How far a working stream has come in media time, as the engine measured
/// it (ADR 0033).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StreamFigures {
    /// Media done so far.
    pub(crate) done_us: u64,
    /// The media in all, when the end is known.
    pub(crate) total_us: Option<u64>,
    /// The whole percent done, when the end is known.
    pub(crate) percent: Option<u8>,
    /// ADR 0033's estimate of the wall time left, once it exists.
    pub(crate) left_us: Option<u64>,
}

/// Where one stream stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StreamView {
    /// Not reported yet.
    Waiting,
    /// Working: its figures, when it reports media time.
    Working(Option<StreamFigures>),
    /// Every unit is done; `found` is the number of changes, when the
    /// progress carries one.
    Done {
        /// How many it found, when known; never guessed.
        found: Option<u64>,
    },
}

/// Which part of the run the view shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WatchPhase {
    /// Before the job starts: the private copy of the video.
    Copying,
    /// The job's streams.
    Streams,
}

/// Everything the live view shows.
#[derive(Clone, Debug)]
pub(crate) struct WatchProgressView {
    /// The video's name, in display form.
    pub(crate) video: DisplayText,
    /// The video's length, once a stream reported it.
    pub(crate) duration_us: Option<u64>,
    /// Which part of the run is shown.
    pub(crate) phase: WatchPhase,
    /// The visual stream.
    pub(crate) screen_changes: StreamView,
    /// The transcript stream.
    pub(crate) speech: StreamView,
    /// The files written as the run goes, in display form; the line is left
    /// out when the caller has none to name.
    pub(crate) writing: Vec<DisplayText>,
}

impl WatchProgressView {
    /// A view of a run that has not started its job yet.
    pub(crate) const fn copying(video: DisplayText, writing: Vec<DisplayText>) -> Self {
        Self {
            video,
            duration_us: None,
            phase: WatchPhase::Copying,
            screen_changes: StreamView::Waiting,
            speech: StreamView::Waiting,
            writing,
        }
    }

    /// Where `stream` stands.
    pub(crate) const fn stream(&self, stream: WatchStream) -> StreamView {
        match stream {
            WatchStream::ScreenChanges => self.screen_changes,
            WatchStream::Speech => self.speech,
        }
    }
}

/// Renders the live view for a terminal that redraws it: bars, symbols and
/// colour for [`Styling::Rich`], words only for [`Styling::Plain`].
pub(crate) fn watch_progress(
    view: &WatchProgressView,
    styling: Styling,
) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    if view.phase == WatchPhase::Copying {
        text.push_fixed("Copying ")
            .push_untrusted(&view.video, Placement::Inline)
            .push_fixed(" into a private session")
            .end_line();
        return text.finish();
    }
    text.push_fixed("VSift  ")
        .push_untrusted(&view.video, Placement::Inline);
    if let Some(duration) = view.duration_us {
        text.push_fixed("  (")
            .push_value(&clock(duration))
            .push_fixed(")");
    }
    text.end_line();
    text.blank_line();
    for stream in WatchStream::ALL {
        push_stream(&mut text, stream, view.stream(stream), styling);
    }
    if !view.writing.is_empty() {
        text.blank_line();
        text.push_fixed("  Writing as it goes: ");
        for (index, name) in view.writing.iter().enumerate() {
            if index > 0 {
                text.push_fixed(", ");
            }
            text.push_untrusted(name, Placement::Inline);
        }
        text.push_fixed(" (open them any time)").end_line();
    }
    text.finish()
}

/// The one plain line written when `stream` ends, where nothing is redrawn
/// (no terminal, or `TERM=dumb`).
pub(crate) fn watch_stream_finished(
    view: &WatchProgressView,
    stream: WatchStream,
) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    push_stream(&mut text, stream, view.stream(stream), Styling::Plain);
    text.finish()
}

/// Writes one stream's line: its name, its bar when `styling` takes one,
/// and its status.
fn push_stream(text: &mut TerminalText, stream: WatchStream, view: StreamView, styling: Styling) {
    let name = stream.name();
    text.push_fixed("  ").push_fixed(name);
    for _ in name.chars().count()..STREAM_NAME_COLUMNS {
        text.push_fixed(" ");
    }
    if styling == Styling::Rich {
        text.push_value(&bar(view)).push_fixed(GAP);
    }
    match view {
        StreamView::Waiting => {
            text.push_status("waiting", Tone::Quiet, styling);
        }
        StreamView::Working(None) => {
            text.push_fixed("working");
        }
        StreamView::Working(Some(figures)) => push_figures(text, figures, styling),
        StreamView::Done { found } => {
            text.push_status("done", Tone::Good, styling);
            if let Some(found) = found {
                text.push_fixed(", ")
                    .push_unsigned(found)
                    .push_fixed(" found");
            }
        }
    }
    text.end_line();
}

/// Writes a working stream's figures: where it is, of how much (with the
/// percent when there is no bar to show it), and the time left once ADR
/// 0033's estimate exists.
fn push_figures(text: &mut TerminalText, figures: StreamFigures, styling: Styling) {
    text.push_value(&clock(figures.done_us));
    let Some(total) = figures.total_us else {
        // No end: neither a percent nor an estimate can exist.
        text.push_fixed(" so far");
        return;
    };
    text.push_fixed(" of ").push_value(&clock(total));
    if styling == Styling::Plain
        && let Some(percent) = figures.percent
    {
        text.push_fixed(" (")
            .push_unsigned(u64::from(percent))
            .push_fixed("%)");
    }
    text.push_fixed(GAP);
    match figures.left_us {
        Some(left) => text.push_value(&time_left(left)),
        None => text.push_fixed("working out how long"),
    };
}

/// A stream's bar: full when done, filled to the percent while it works,
/// empty before.
fn bar(view: StreamView) -> String {
    let filled = match view {
        StreamView::Done { .. } => BAR_CELLS,
        StreamView::Working(Some(StreamFigures {
            percent: Some(percent),
            ..
        })) => u64::from(percent).min(100) * BAR_CELLS / 100,
        StreamView::Waiting | StreamView::Working(_) => 0,
    };
    (0..BAR_CELLS)
        .map(|cell| {
            if cell < filled {
                '\u{2588}'
            } else {
                '\u{2591}'
            }
        })
        .collect()
}
