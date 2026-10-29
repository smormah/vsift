//! The live view of `vsift <video>` while it works (ADR 0037 section 9):
//! from the `prepare` job's progress to the lines on standard error.
//!
//! In readable-text mode in the real process, standard error shows one line
//! per stream, redrawn in place at most once a second and cleared before the
//! finish screen or the failure is written ([`WatchDisplay::InPlace`]).
//! Where nothing can be redrawn, because standard error is not a terminal or
//! the terminal is `TERM=dumb`, one plain line is written as each stream
//! ends ([`WatchDisplay::Finished`]). `--json` and `--events jsonl` show
//! nothing of it.
//!
//! Three parts are kept apart so each is tested on its own: the
//! [`WatchBoard`] turns progress observations into the view's state, the
//! renderer in `human::watch_progress` turns a state into text, and
//! [`with_watch_progress`] draws, throttled by a [`RedrawGate`] that is
//! given the time rather than reading a clock. The engine calls the
//! observer on the command's own task, so the observer only updates the
//! board and signals; the lines are written from the task that owns the
//! output writer, as ADR 0033's live line is.

use std::{
    io::{IsTerminal as _, Write},
    pin::pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tokio::sync::Notify;
use vsift::{JobProgress, ProgressObserver, ProgressStage, ProgressUpdate};

use crate::{
    human::{
        self, DisplayText, StreamFigures, StreamView, Styling, WatchPhase, WatchProgressView,
        WatchStream,
    },
    output::OutputWriter,
};

/// The least time between two redraws of the view (ADR 0037 section 9).
const REDRAW_INTERVAL: Duration = Duration::from_secs(1);

/// How the run's progress is shown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WatchDisplay {
    /// Nothing: `--json`, or a host that is not the real process.
    Hidden,
    /// Redrawn in place on a terminal: bars, symbols and colour when the
    /// styling is rich, words only otherwise.
    InPlace(Styling),
    /// One plain line as each stream ends: no terminal, or one that cannot
    /// move its cursor.
    Finished,
}

/// What the process knows about its standard error, read once.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Each is one independent fact read from the process; the decision combines them"
)]
pub(crate) struct TerminalFacts {
    /// Standard error is a terminal.
    pub(crate) terminal: bool,
    /// `TERM` is `dumb`: the terminal takes no cursor movement.
    pub(crate) dumb: bool,
    /// The console takes virtual terminal sequences (always, off Windows).
    pub(crate) sequences: bool,
    /// `NO_COLOR` is set to anything.
    pub(crate) no_color: bool,
    /// `TERM` allows colour.
    pub(crate) colour_term: bool,
}

impl TerminalFacts {
    /// The facts of this process's standard error and environment.
    ///
    /// The colour decision and the Windows console handling are the setup
    /// door's own (ADR 0036 section 8, through `anstyle-query`); a console
    /// that refuses virtual terminal sequences cannot have its cursor moved
    /// either, so it gets the plain lines.
    pub(crate) fn of_process() -> Self {
        let terminal = std::io::stderr().is_terminal();
        let dumb = std::env::var_os("TERM").is_some_and(|term| term == "dumb");
        Self {
            terminal,
            dumb,
            sequences: terminal && !dumb && crate::door::windows_console_takes_colour(),
            no_color: anstyle_query::no_color(),
            colour_term: anstyle_query::term_supports_color(),
        }
    }

    /// How the progress is shown on a standard error with these facts.
    pub(crate) const fn display(self) -> WatchDisplay {
        if !self.terminal || self.dumb || !self.sequences {
            WatchDisplay::Finished
        } else if self.no_color || !self.colour_term {
            WatchDisplay::InPlace(Styling::Plain)
        } else {
            WatchDisplay::InPlace(Styling::Rich)
        }
    }
}

/// The newest state of the run's view, and which streams' ends have been
/// announced in plain lines.
#[derive(Debug)]
pub(crate) struct WatchBoard {
    view: WatchProgressView,
    announced: [bool; 2],
}

impl WatchBoard {
    /// The board of a run that has not started its job: it shows the
    /// private copy of `video` (a name in display form). `writing` names the
    /// files written as the run goes, for the view's last line; it is empty
    /// when the caller cannot name them.
    pub(crate) const fn new(video: DisplayText, writing: Vec<DisplayText>) -> Self {
        Self {
            view: WatchProgressView::copying(video, writing),
            announced: [false; 2],
        }
    }

    /// The view's state now.
    pub(crate) const fn view(&self) -> &WatchProgressView {
        &self.view
    }

    /// Takes in one progress observation; answers whether the view changed.
    ///
    /// `analysing_video` is the screen changes and `recognising_speech` the
    /// speech; the first of either ends the copy. `copying_source` keeps
    /// the copy line, which carries no figures. A stream that is done stays
    /// done.
    pub(crate) fn observe(&mut self, progress: &JobProgress) -> bool {
        let update = progress.update;
        let stream = match update.stage {
            ProgressStage::AnalysingVideo => WatchStream::ScreenChanges,
            ProgressStage::RecognisingSpeech => WatchStream::Speech,
            ProgressStage::CopyingSource
            | ProgressStage::RunningRequest
            | ProgressStage::FetchingArtifact
            | ProgressStage::InstallingComponents => return false,
        };
        let before = self.shown(stream);
        self.view.phase = WatchPhase::Streams;
        if let Some(total) = update.media.and_then(|media| media.total_us()) {
            // The visual stream measures the whole video; the speech's range
            // stands in until it reports.
            if stream == WatchStream::ScreenChanges || self.view.duration_us.is_none() {
                self.view.duration_us = Some(total);
            }
        }
        let slot = match stream {
            WatchStream::ScreenChanges => &mut self.view.screen_changes,
            WatchStream::Speech => &mut self.view.speech,
        };
        if !matches!(slot, StreamView::Done { .. }) {
            *slot = stream_view(&update);
        }
        before != self.shown(stream)
    }

    /// What the view shows of the run as a whole and of `stream`.
    const fn shown(&self, stream: WatchStream) -> (WatchPhase, Option<u64>, StreamView) {
        (
            self.view.phase,
            self.view.duration_us,
            self.view.stream(stream),
        )
    }

    /// The streams that ended since the last call, each answered once.
    pub(crate) fn newly_finished(&mut self) -> Vec<WatchStream> {
        WatchStream::ALL
            .into_iter()
            .zip(self.announced.iter_mut())
            .filter_map(|(stream, announced)| {
                let done = matches!(self.view.stream(stream), StreamView::Done { .. });
                (done && !*announced).then(|| {
                    *announced = true;
                    stream
                })
            })
            .collect()
    }
}

/// Where a stream stands after `update`: done once every unit is, otherwise
/// working with the media figures it reports (ADR 0033's estimate among
/// them). The progress carries no count of the changes found, so a done
/// stream's count stays unknown rather than guessed.
fn stream_view(update: &ProgressUpdate) -> StreamView {
    if update.total.is_some_and(|total| update.completed >= total) {
        return StreamView::Done { found: None };
    }
    StreamView::Working(update.media.map(|media| StreamFigures {
        done_us: media.done_us(),
        total_us: media.total_us(),
        percent: media.percent(),
        left_us: media.estimate_remaining_us(),
    }))
}

/// When the view may be redrawn next.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Redraw {
    /// Now.
    Now,
    /// Not before this time.
    NotBefore(Instant),
}

/// At most one redraw per interval. It is given the time of each question
/// rather than reading a clock, so its rule is tested without waiting.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RedrawGate {
    interval: Duration,
    last: Option<Instant>,
}

impl RedrawGate {
    /// A gate that allows one redraw per `interval`.
    pub(crate) const fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: None,
        }
    }

    /// When a redraw asked for at `now` may happen.
    pub(crate) fn when(&self, now: Instant) -> Redraw {
        match self.last {
            Some(last) if now.saturating_duration_since(last) < self.interval => {
                Redraw::NotBefore(last + self.interval)
            }
            _ => Redraw::Now,
        }
    }

    /// Records a redraw at `now`.
    pub(crate) const fn drawn(&mut self, now: Instant) {
        self.last = Some(now);
    }
}

/// What is on the screen, and when it may change.
struct Screen {
    display: WatchDisplay,
    gate: RedrawGate,
    /// Lines of the view drawn in place.
    drawn: usize,
    /// A redraw that waits for the gate.
    pending: Option<Instant>,
}

impl Screen {
    /// The board changed at `now`: an in-place view is redrawn when the gate
    /// allows, or once it does; a stream that ended gets its plain line at
    /// once.
    fn changed<StandardOutput, StandardError>(
        &mut self,
        writer: &mut OutputWriter<StandardOutput, StandardError>,
        board: &Mutex<WatchBoard>,
        now: Instant,
    ) where
        StandardOutput: Write,
        StandardError: Write,
    {
        match self.display {
            WatchDisplay::Hidden => {}
            WatchDisplay::InPlace(_) => match self.gate.when(now) {
                Redraw::Now => self.redraw(writer, board, now),
                Redraw::NotBefore(at) => {
                    self.pending.get_or_insert(at);
                }
            },
            WatchDisplay::Finished => announce(writer, board),
        }
    }

    /// Draws the view in place now.
    fn redraw<StandardOutput, StandardError>(
        &mut self,
        writer: &mut OutputWriter<StandardOutput, StandardError>,
        board: &Mutex<WatchBoard>,
        now: Instant,
    ) where
        StandardOutput: Write,
        StandardError: Write,
    {
        let WatchDisplay::InPlace(styling) = self.display else {
            return;
        };
        self.pending = None;
        self.gate.drawn(now);
        let rendered = board
            .lock()
            .ok()
            .and_then(|board| human::watch_progress(board.view(), styling).ok());
        if let Some(text) = rendered {
            self.drawn = writer.write_error_block(&text, self.drawn);
        }
    }

    /// The run ended: the view drawn in place is cleared, and a stream that
    /// ended unannounced gets its plain line.
    fn finish<StandardOutput, StandardError>(
        &mut self,
        writer: &mut OutputWriter<StandardOutput, StandardError>,
        board: &Mutex<WatchBoard>,
    ) where
        StandardOutput: Write,
        StandardError: Write,
    {
        match self.display {
            WatchDisplay::Hidden => {}
            WatchDisplay::InPlace(_) => writer.clear_error_block(self.drawn),
            WatchDisplay::Finished => announce(writer, board),
        }
    }
}

/// Writes the plain line of each stream that ended since the last time.
fn announce<StandardOutput, StandardError>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    board: &Mutex<WatchBoard>,
) where
    StandardOutput: Write,
    StandardError: Write,
{
    let Ok(mut board) = board.lock() else {
        return;
    };
    for stream in board.newly_finished() {
        if let Ok(line) = human::watch_stream_finished(board.view(), stream) {
            writer.write_rendered_stderr(&line);
        }
    }
}

/// Runs `vsift <video>` with its progress shown as `display` says, starting
/// from `board`, and returns the run's result once the view is cleared or
/// every ended stream is announced.
pub(crate) async fn with_watch_progress<StandardOutput, StandardError, Run, Work, Output>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    display: WatchDisplay,
    board: WatchBoard,
    run: Run,
) -> Output
where
    StandardOutput: Write,
    StandardError: Write,
    Run: FnOnce(ProgressObserver) -> Work,
    Work: Future<Output = Output>,
{
    if display == WatchDisplay::Hidden {
        return run(ProgressObserver::none()).await;
    }
    let board = Arc::new(Mutex::new(board));
    let changed = Arc::new(Notify::new());
    let kept = Arc::clone(&board);
    let signal = Arc::clone(&changed);
    let observer = ProgressObserver::new(move |progress| {
        let moved = kept.lock().is_ok_and(|mut board| board.observe(progress));
        if moved {
            signal.notify_one();
        }
    });
    let mut screen = Screen {
        display,
        gate: RedrawGate::new(REDRAW_INTERVAL),
        drawn: 0,
        pending: None,
    };
    // The copy line at once: the engine reports nothing while it copies.
    if let WatchDisplay::InPlace(_) = display {
        screen.redraw(writer, &board, now());
    }
    let mut work = pin!(run(observer));
    let result = loop {
        let pending = screen.pending;
        let due = pending.map_or_else(tokio::time::Instant::now, tokio::time::Instant::from_std);
        tokio::select! {
            biased;
            result = &mut work => break result,
            () = changed.notified() => screen.changed(writer, &board, now()),
            () = tokio::time::sleep_until(due), if pending.is_some() => {
                screen.redraw(writer, &board, now());
            }
        }
    };
    screen.finish(writer, &board);
    result
}

/// The time now, on the runtime's clock.
fn now() -> Instant {
    tokio::time::Instant::now().into_std()
}

#[cfg(test)]
mod tests;
