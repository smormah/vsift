//! The live view of `vsift <video>` (ADR 0037 section 9): its three forms
//! as golden snapshots (`crates/vsift-cli/tests/human_output/watch-progress-*.txt`,
//! rewritten with `VSIFT_UPDATE_HUMAN_SNAPSHOTS=1`), the board that builds
//! its state from the engine's progress, the display decision, the redraw
//! throttle on given times, and the drawing.

use std::time::{Duration, Instant};

use vsift::{JobProgress, MediaProgress, ProgressObserver, ProgressStage, ProgressUpdate};

use super::{Redraw, RedrawGate, TerminalFacts, WatchBoard, WatchDisplay, with_watch_progress};
use crate::{
    human::{self, DisplayText, StreamView, Styling, WatchStream, tests::check_snapshot},
    output::OutputWriter,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The video of every scenario: 48:00.
const VIDEO_S: u64 = 2_880;

/// Media figures: `done_s` of the video done, the fresh part `fresh_s`
/// measured over `wall_s` seconds in `chunks` chunks.
const fn media(done_s: u64, fresh_s: u64, wall_s: u64, chunks: u32) -> MediaProgress {
    MediaProgress {
        from_us: 0,
        done_until_us: done_s * 1_000_000,
        to_us: Some(VIDEO_S * 1_000_000),
        elapsed_us: wall_s * 1_000_000,
        fresh_media_us: fresh_s * 1_000_000,
        fresh_elapsed_us: wall_s * 1_000_000,
        fresh_chunks: chunks,
    }
}

fn observed(stage: ProgressStage, completed: u64, total: u64, media: MediaProgress) -> JobProgress {
    JobProgress {
        job: None,
        update: ProgressUpdate {
            stage,
            completed,
            total: Some(total),
            media: Some(media),
        },
    }
}

/// The visual stream: 96 windows of 30 s.
fn screen(done_s: u64, wall_s: u64) -> JobProgress {
    let windows = done_s / 30;
    let chunks = u32::try_from(windows).unwrap_or(u32::MAX);
    observed(
        ProgressStage::AnalysingVideo,
        windows,
        VIDEO_S / 30,
        media(done_s, done_s, wall_s, chunks),
    )
}

/// The transcript stream: 96 chunks of 30 s.
fn speech(done_s: u64, wall_s: u64) -> JobProgress {
    let chunks = done_s / 30;
    let fresh = u32::try_from(chunks).unwrap_or(u32::MAX);
    observed(
        ProgressStage::RecognisingSpeech,
        chunks,
        VIDEO_S / 30,
        media(done_s, done_s, wall_s, fresh),
    )
}

fn board() -> WatchBoard {
    WatchBoard::new(DisplayText::render("meeting.mp4"), Vec::new())
}

/// The scenario every form is shown in: the copy, then each stream before
/// its estimate, while it works with one, and at the end. Each step is a
/// name and the observations that lead to it.
fn scenario() -> Vec<(&'static str, Vec<JobProgress>)> {
    vec![
        ("before the job: the private copy", vec![]),
        (
            "screen changes before an estimate, speech not started",
            // 5:00 in 10 s: under the 20 s an estimate needs.
            vec![screen(300, 10)],
        ),
        (
            "screen changes with an estimate, speech before one",
            // 20:00 in 60 s: the 28:00 left take 84 s. Speech: one chunk.
            vec![screen(1_200, 60), speech(30, 12)],
        ),
        (
            "screen changes done, speech with an estimate",
            // 12:30 in 190 s: the 35:30 left take 539 s.
            vec![screen(VIDEO_S, 140), speech(750, 190)],
        ),
        ("both done", vec![speech(VIDEO_S, 700)]),
    ]
}

/// Renders every step of the scenario in place, `styling` as given.
fn in_place(styling: Styling) -> Result<String, Box<dyn std::error::Error>> {
    let mut board = board();
    let mut shown = String::new();
    for (step, observations) in scenario() {
        for progress in &observations {
            board.observe(progress);
        }
        shown.push_str("== ");
        shown.push_str(step);
        shown.push_str(" ==\n");
        shown.push_str(
            human::watch_progress(board.view(), styling)
                .map_err(|_| "too large")?
                .as_str(),
        );
        shown.push('\n');
    }
    Ok(shown)
}

/// A terminal that takes symbols and colour: bars, and the status words
/// with their symbol and colour (the escapes shown as `<ESC>`).
#[test]
fn on_a_terminal_each_stream_has_a_bar_and_its_status() -> TestResult {
    let shown = in_place(Styling::Rich)?;
    assert!(
        shown.contains("\u{1b}[32m\u{2713} done\u{1b}[0m"),
        "{shown}"
    );
    assert!(
        shown.contains("\u{1b}[2m\u{2013} waiting\u{1b}[0m"),
        "{shown}"
    );
    check_snapshot("watch-progress-terminal", &shown.replace('\u{1b}', "<ESC>"))
}

/// `NO_COLOR`: the same lines in words only, the percent in place of the
/// bar, and no escape sequence.
#[test]
fn with_no_color_the_lines_are_words_only() -> TestResult {
    let shown = in_place(Styling::Plain)?;
    assert!(!shown.contains('\u{1b}'), "{shown}");
    assert!(!shown.contains('\u{2588}') && !shown.contains('\u{2591}'));
    assert!(shown.contains("12:30 of 48:00 (26%)   about 9 min left"));
    assert!(shown.contains("working out how long"));
    check_snapshot("watch-progress-no-color", &shown)
}

/// `TERM=dumb`, or no terminal: nothing redrawn, one plain line as each
/// stream ends, each once.
#[test]
fn without_a_terminal_one_plain_line_as_each_stream_ends() -> TestResult {
    let mut board = board();
    let mut shown = String::new();
    for (_, observations) in scenario() {
        for progress in &observations {
            board.observe(progress);
            for stream in board.newly_finished() {
                shown.push_str(
                    human::watch_stream_finished(board.view(), stream)
                        .map_err(|_| "too large")?
                        .as_str(),
                );
            }
        }
    }
    // Reported again, a stream that ended is not announced again.
    board.observe(&speech(VIDEO_S, 701));
    assert_eq!(board.newly_finished(), []);
    assert!(!shown.contains('\u{1b}'));
    assert_eq!(shown, "  Screen changes   done\n  Speech           done\n");
    check_snapshot("watch-progress-dumb", &shown)
}

/// The video's name is untrusted: its hidden and control characters are
/// visible, a long one is shortened, and the caller's file names close the
/// view.
#[test]
fn the_name_is_shown_in_display_form_and_the_files_when_named() -> TestResult {
    let hostile = "meet\u{202e}ing\u{1b}[31m.mp4";
    let mut board = WatchBoard::new(
        DisplayText::render(hostile),
        vec![
            DisplayText::render("meeting.txt"),
            DisplayText::render("meeting.srt"),
        ],
    );
    let copying = human::watch_progress(board.view(), Styling::Plain).map_err(|_| "too large")?;
    assert_eq!(
        copying.as_str(),
        "Copying meet<U+202E>ing\u{fffd}[31m.mp4 into a private session\n"
    );
    board.observe(&speech(750, 190));
    let shown = human::watch_progress(board.view(), Styling::Plain).map_err(|_| "too large")?;
    assert_eq!(
        shown.as_str(),
        "VSift  meet<U+202E>ing\u{fffd}[31m.mp4  (48:00)\n\
         \n\
         \x20 Screen changes   waiting\n\
         \x20 Speech           12:30 of 48:00 (26%)   about 9 min left\n\
         \n\
         \x20 Writing as it goes: meeting.txt, meeting.srt (open them any time)\n"
    );
    let long = "a".repeat(60);
    assert_eq!(
        DisplayText::render_shortened(&long, 48),
        DisplayText::render(&format!("{}...", "a".repeat(48)))
    );
    assert_eq!(
        DisplayText::render_shortened("meeting.mp4", 48),
        DisplayText::render("meeting.mp4")
    );
    Ok(())
}

/// The visual stream gives the length; a stream that is done stays done; a
/// stage of another command, or the copy, changes nothing.
#[test]
fn the_board_follows_the_streams() {
    let mut board = board();
    assert!(!board.observe(&observed(
        ProgressStage::CopyingSource,
        10,
        100,
        media(0, 0, 0, 0)
    )));
    let mut short = speech(30, 12);
    if let Some(media) = short.update.media.as_mut() {
        media.to_us = Some(600_000_000);
    }
    assert!(board.observe(&short));
    assert_eq!(board.view().duration_us, Some(600_000_000));
    assert!(board.observe(&screen(300, 10)));
    assert_eq!(board.view().duration_us, Some(VIDEO_S * 1_000_000));
    assert!(!board.observe(&screen(300, 10)));
    assert!(board.observe(&screen(VIDEO_S, 100)));
    assert!(!board.observe(&screen(1_200, 101)));
    assert_eq!(
        board.view().stream(WatchStream::ScreenChanges),
        StreamView::Done { found: None }
    );
    let mut unmeasured = speech(60, 20);
    unmeasured.update.media = None;
    assert!(board.observe(&unmeasured));
    assert_eq!(
        board.view().stream(WatchStream::Speech),
        StreamView::Working(None)
    );
}

/// ADR 0036 section 8: symbols and colour on a terminal that takes them;
/// words only for `NO_COLOR` or a terminal without colour; plain lines for
/// `TERM=dumb`, a console without sequences, or no terminal.
#[test]
fn the_terminal_decides_the_form() {
    let terminal = TerminalFacts {
        terminal: true,
        dumb: false,
        sequences: true,
        no_color: false,
        colour_term: true,
    };
    assert_eq!(terminal.display(), WatchDisplay::InPlace(Styling::Rich));
    let no_color = TerminalFacts {
        no_color: true,
        ..terminal
    };
    assert_eq!(no_color.display(), WatchDisplay::InPlace(Styling::Plain));
    let colourless = TerminalFacts {
        colour_term: false,
        ..terminal
    };
    assert_eq!(colourless.display(), WatchDisplay::InPlace(Styling::Plain));
    let dumb = TerminalFacts {
        dumb: true,
        colour_term: false,
        ..terminal
    };
    assert_eq!(dumb.display(), WatchDisplay::Finished);
    let no_sequences = TerminalFacts {
        sequences: false,
        ..terminal
    };
    assert_eq!(no_sequences.display(), WatchDisplay::Finished);
    let pipe = TerminalFacts {
        terminal: false,
        ..terminal
    };
    assert_eq!(pipe.display(), WatchDisplay::Finished);
}

/// At most one redraw a second, on the times given: the first at once, a
/// change inside the second waits for its end, and one after it is drawn at
/// once.
#[test]
fn the_view_is_redrawn_at_most_once_a_second() {
    let start = Instant::now();
    let mut gate = RedrawGate::new(Duration::from_secs(1));
    assert_eq!(gate.when(start), Redraw::Now);
    gate.drawn(start);
    let second = start + Duration::from_secs(1);
    assert_eq!(
        gate.when(start + Duration::from_millis(1)),
        Redraw::NotBefore(second)
    );
    assert_eq!(
        gate.when(start + Duration::from_millis(999)),
        Redraw::NotBefore(second)
    );
    assert_eq!(gate.when(second), Redraw::Now);
    gate.drawn(second + Duration::from_millis(300));
    assert_eq!(
        gate.when(second + Duration::from_millis(500)),
        Redraw::NotBefore(second + Duration::from_millis(1_300))
    );
    assert_eq!(gate.when(start + Duration::from_secs(5)), Redraw::Now);
}

/// Drawn in place on standard error: the copy line at once, the next
/// change held by the throttle, and the view cleared before the result;
/// standard output stays clean.
#[tokio::test]
async fn the_view_is_drawn_on_standard_error_and_cleared() {
    let mut standard_output = Vec::new();
    let mut standard_error = Vec::new();
    let mut writer = OutputWriter::new(&mut standard_output, &mut standard_error);
    let answer = with_watch_progress(
        &mut writer,
        WatchDisplay::InPlace(Styling::Plain),
        board(),
        |observer: ProgressObserver| async move {
            observer.report(&screen(300, 10));
            tokio::task::yield_now().await;
            7
        },
    )
    .await;
    assert_eq!(answer, 7);
    assert_eq!(standard_output.len(), 0);
    assert_eq!(
        String::from_utf8_lossy(&standard_error),
        "\x1b[2KCopying meeting.mp4 into a private session\n\x1b[J\x1b[1F\x1b[J"
    );
}

/// Without a terminal: a plain line on standard error as each stream ends,
/// and nothing else.
#[tokio::test]
async fn without_a_terminal_the_ends_are_written_as_they_come() {
    let mut standard_output = Vec::new();
    let mut standard_error = Vec::new();
    let mut writer = OutputWriter::new(&mut standard_output, &mut standard_error);
    with_watch_progress(
        &mut writer,
        WatchDisplay::Finished,
        board(),
        |observer: ProgressObserver| async move {
            observer.report(&screen(300, 10));
            observer.report(&screen(VIDEO_S, 100));
            tokio::task::yield_now().await;
            observer.report(&speech(VIDEO_S, 700));
        },
    )
    .await;
    assert_eq!(standard_output.len(), 0);
    assert_eq!(
        String::from_utf8_lossy(&standard_error),
        "  Screen changes   done\n  Speech           done\n"
    );
}

/// `--json` and hosts other than the real process: nothing is written.
#[tokio::test]
async fn hidden_writes_nothing() {
    let mut standard_output = Vec::new();
    let mut standard_error = Vec::new();
    let mut writer = OutputWriter::new(&mut standard_output, &mut standard_error);
    with_watch_progress(
        &mut writer,
        WatchDisplay::Hidden,
        board(),
        |observer: ProgressObserver| async move {
            observer.report(&screen(VIDEO_S, 100));
        },
    )
    .await;
    assert_eq!((standard_output.len(), standard_error.len()), (0, 0));
}
