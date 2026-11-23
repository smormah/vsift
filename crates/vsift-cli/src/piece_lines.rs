//! The setup door's progress while it installs (ADR 0036 section 5): one
//! line per piece.
//!
//! On a terminal that takes symbols and colour, each piece has a bar, the
//! megabytes so far of its reviewed size and its stage, and the lines are
//! redrawn in place on standard output at most four times a second, then
//! cleared before the results are written. Without bars (the unattended
//! form), one plain line is written per piece as it finishes. The engine
//! reports pieces on the command's own task, so the observer only keeps the
//! newest state of each; the lines are written from the task that owns the
//! output writer, as the live line of ADR 0033 is.

use std::{
    io::Write,
    pin::pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use vsift::{PieceProgress, ProgressObserver};

use crate::{human, output::OutputWriter};

/// The least time between two redraws of the bars.
const REDRAW_INTERVAL: Duration = Duration::from_millis(250);

/// How the install's pieces are shown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PieceDisplay {
    /// Bars redrawn in place, for a terminal that takes them.
    Bars,
    /// One plain line per piece as it finishes.
    Finished,
}

/// The newest state of each piece, in the order they were first reported,
/// and how many finished lines are written.
#[derive(Default)]
struct Board {
    pieces: Vec<PieceProgress>,
    changed: bool,
    written: Vec<bool>,
}

impl Board {
    fn update(&mut self, progress: PieceProgress) {
        if let Some(piece) = self
            .pieces
            .iter_mut()
            .find(|piece| piece.component == progress.component)
        {
            *piece = progress;
        } else {
            self.pieces.push(progress);
            self.written.push(false);
        }
        self.changed = true;
    }
}

/// Runs an install with its pieces shown as `display` says, and returns
/// the install's result once the bars are cleared or every finished line is
/// written.
pub(crate) async fn with_piece_display<StandardOutput, StandardError, Run, Work, Output>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    display: PieceDisplay,
    run: Run,
) -> Output
where
    StandardOutput: Write,
    StandardError: Write,
    Run: FnOnce(ProgressObserver) -> Work,
    Work: Future<Output = Output>,
{
    let board: Arc<Mutex<Board>> = Arc::default();
    let kept = Arc::clone(&board);
    let observer = ProgressObserver::none().with_install_pieces(move |piece| {
        if let Ok(mut board) = kept.lock() {
            board.update(*piece);
        }
    });
    let mut work = pin!(run(observer));
    let mut tick = tokio::time::interval(REDRAW_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut drawn = 0_usize;
    let result = loop {
        tokio::select! {
            biased;
            result = &mut work => break result,
            _ = tick.tick() => {
                if let Ok(mut board) = board.lock() {
                    drawn = show(writer, display, &mut board, drawn);
                }
            }
        }
    };
    match display {
        PieceDisplay::Bars => writer.clear_live_block(drawn),
        PieceDisplay::Finished => {
            if let Ok(mut board) = board.lock() {
                let _written = show(writer, display, &mut board, drawn);
            }
        }
    }
    result
}

/// Shows what changed; returns how many bar lines are on the screen.
fn show<StandardOutput, StandardError>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    display: PieceDisplay,
    board: &mut Board,
    drawn: usize,
) -> usize
where
    StandardOutput: Write,
    StandardError: Write,
{
    if !board.changed {
        return drawn;
    }
    board.changed = false;
    match display {
        PieceDisplay::Bars => {
            let lines = human::piece_bars(&board.pieces);
            writer.write_live_block(&lines, drawn);
            lines.len()
        }
        PieceDisplay::Finished => {
            for (piece, written) in board.pieces.iter().zip(board.written.iter_mut()) {
                if !*written && human::piece_finished(piece.stage) {
                    *written = true;
                    if let Ok(line) = human::piece_finished_line(piece) {
                        let _shown = writer
                            .write_rendered_stdout(&line)
                            .and_then(|()| writer.flush_stdout());
                    }
                }
            }
            0
        }
    }
}
