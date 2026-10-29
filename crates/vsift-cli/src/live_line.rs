//! The live progress line of a long command in readable-text mode (ADR 0033
//! section 5).
//!
//! When the output is readable text (neither `--json` nor `--events`) and
//! standard error is a terminal, `transcript retranscribe`, `job resume` and
//! a foreground `job start` show one line on standard error that says how far
//! the recognition has come, updated at most once a second and cleared before
//! the result is written:
//!
//! ```text
//! Transcribing 34:10 of 1:30:00 (38%), 2.1x real time, about 26 min left (estimate)
//! ```
//!
//! While the command waits for queued work to give back the root's capacity
//! (ADR 0033 section 6), the line says so until the recognition reports:
//!
//! ```text
//! Waiting for queued work to make room (at most 2 min)
//! ```
//!
//! It never appears in JSON or JSON Lines output, and never when standard
//! error is a file or a pipe. The engine calls the observer on the command's
//! own task, so the observer only keeps the newest observation; the line is
//! written from the task that owns the output writer, as the progress events
//! of `--events jsonl` are.

use std::{
    fmt::Write as _,
    io::Write,
    pin::pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use vsift::{AdmissionWaitCause, JobProgress, MediaProgress, ProgressObserver, ProgressStage};

use crate::output::OutputWriter;

/// The least time between two writes of the line.
const LINE_INTERVAL: Duration = Duration::from_secs(1);

/// Whether a command shows the live line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LiveLine {
    /// Readable text on a terminal: the line is shown.
    Shown,
    /// JSON, JSON Lines, or standard error that is not a terminal.
    Hidden,
}

/// Runs a long command with its live line, when `line` says to show it, and
/// returns the command's result once the line is cleared.
pub(crate) async fn with_live_line<StandardOutput, StandardError, Run, Work, Output>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    line: LiveLine,
    run: Run,
) -> Output
where
    StandardOutput: Write,
    StandardError: Write,
    Run: FnOnce(ProgressObserver) -> Work,
    Work: Future<Output = Output>,
{
    with_live_line_saying(writer, line, line_for, run).await
}

/// [`with_live_line`] with the sentence `say` makes of each progress
/// observation: the setup door's install says how many pieces are done.
pub(crate) async fn with_live_line_saying<StandardOutput, StandardError, Run, Work, Output>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    line: LiveLine,
    say: fn(&JobProgress) -> Option<String>,
    run: Run,
) -> Output
where
    StandardOutput: Write,
    StandardError: Write,
    Run: FnOnce(ProgressObserver) -> Work,
    Work: Future<Output = Output>,
{
    if line == LiveLine::Hidden {
        return run(ProgressObserver::none()).await;
    }
    let newest: Arc<Mutex<Option<Observed>>> = Arc::default();
    let kept = Arc::clone(&newest);
    let waits = Arc::clone(&newest);
    let observer = ProgressObserver::new(move |progress| {
        if let Ok(mut newest) = kept.lock() {
            *newest = Some(Observed::Progress(progress.clone()));
        }
    })
    .with_admission_waiting(move |waiting| {
        if let Ok(mut newest) = waits.lock() {
            *newest = Some(Observed::Waiting(waiting.cause));
        }
    });
    let mut work = pin!(run(observer));
    let mut tick = tokio::time::interval(LINE_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut written = false;
    let result = loop {
        tokio::select! {
            biased;
            result = &mut work => break result,
            _ = tick.tick() => {
                let taken = newest.lock().ok().and_then(|mut newest| newest.take());
                if let Some(sentence) = taken
                    .as_ref()
                    .and_then(|observed| line_for_observed(observed, say))
                {
                    writer.write_live_line(&sentence);
                    written = true;
                }
            }
        }
    };
    if written {
        writer.clear_live_line();
    }
    result
}

/// The newest observation the line shows.
#[derive(Clone, Debug)]
enum Observed {
    /// How far the work has come.
    Progress(JobProgress),
    /// The work waits for admission capacity.
    Waiting(AdmissionWaitCause),
}

/// The line for the newest observation, or `None` for one it does not show.
fn line_for_observed(
    observed: &Observed,
    say: fn(&JobProgress) -> Option<String>,
) -> Option<String> {
    match observed {
        Observed::Progress(progress) => say(progress),
        Observed::Waiting(cause) => Some(waiting_line(*cause)),
    }
}

/// The line while the work waits for capacity.
fn waiting_line(cause: AdmissionWaitCause) -> String {
    match cause {
        AdmissionWaitCause::QueuedWork => format!(
            "Waiting for queued work to make room (at most {} min)",
            vsift::QUEUED_YIELD_WAIT.as_secs().div_ceil(60)
        ),
        AdmissionWaitCause::CapacityInUse => "Waiting for capacity".to_owned(),
    }
}

/// The setup door's line while it installs (ADR 0036): how many of the
/// plan's pieces are done, from the transaction's own counts.
pub(crate) fn install_line(progress: &JobProgress) -> Option<String> {
    let update = progress.update;
    if update.stage != ProgressStage::InstallingComponents {
        return None;
    }
    Some(match update.total {
        Some(total) => format!("Installing: {} of {total} done", update.completed),
        None => format!("Installing: {} done", update.completed),
    })
}

/// The line for one observation, or `None` for a stage it does not show.
fn line_for(progress: &JobProgress) -> Option<String> {
    let update = progress.update;
    if update.stage != ProgressStage::RecognisingSpeech {
        return None;
    }
    Some(match update.media {
        Some(media) => format!("Transcribing {}", media_sentence(&media, true)),
        None => match update.total {
            Some(total) => format!("Transcribing: chunk {} of {total}", update.completed),
            None => format!("Transcribing: chunk {}", update.completed),
        },
    })
}

/// How far a recognition has come, in words and figures only: the media done
/// and in all, the percent, the speed and, when `estimating`, the time left.
pub(crate) fn media_sentence(media: &MediaProgress, estimating: bool) -> String {
    let mut sentence = clock(media.done_us());
    if let Some(total) = media.total_us() {
        sentence.push_str(" of ");
        sentence.push_str(&clock(total));
    }
    if let Some(percent) = media.percent() {
        let _ = write!(sentence, " ({percent}%)");
    }
    if let Some(speed) = media.speed_millis() {
        // One decimal, rounded half up: 2.149 is 2.1, 2.150 is 2.2.
        let tenths = speed.saturating_add(50) / 100;
        let _ = write!(sentence, ", {}.{}x real time", tenths / 10, tenths % 10);
    }
    if estimating && let Some(left) = media.estimate_remaining_us() {
        sentence.push_str(", ");
        sentence.push_str(&time_left(left));
        sentence.push_str(" (estimate)");
    }
    sentence
}

/// A media time as `M:SS`, or `H:MM:SS` from an hour on.
pub(crate) fn clock(micros: u64) -> String {
    let seconds = micros / 1_000_000;
    let (hours, minutes, seconds) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// The time left in round words: under a minute, minutes, or hours and
/// minutes.
pub(crate) fn time_left(micros: u64) -> String {
    let minutes = micros.div_ceil(60_000_000);
    match minutes {
        0 | 1 if micros < 60_000_000 => "under a minute left".to_owned(),
        0..60 => format!("about {minutes} min left"),
        _ => format!("about {} h {} min left", minutes / 60, minutes % 60),
    }
}

#[cfg(test)]
mod tests {
    use vsift::{
        AdmissionWaitCause, AdmissionWaiting, JobProgress, MediaProgress, ProgressObserver,
        ProgressStage, ProgressUpdate,
    };

    use super::{LiveLine, line_for, media_sentence, waiting_line, with_live_line};
    use crate::output::OutputWriter;

    /// ADR 0033 section 6: a command that waits for queued work shows that it
    /// waits, and how long it may, until the recognition reports; the line is
    /// cleared before the result.
    #[tokio::test]
    async fn a_wait_for_queued_work_shows_on_the_line() {
        let mut standard_error = Vec::new();
        let mut output = OutputWriter::new(Vec::<u8>::new(), &mut standard_error);
        with_live_line(
            &mut output,
            LiveLine::Shown,
            |observer: ProgressObserver| async move {
                observer.admission_waiting(&AdmissionWaiting {
                    job: None,
                    weight: std::num::NonZeroU16::MIN,
                    cause: AdmissionWaitCause::QueuedWork,
                });
                // The line's first tick comes on the timer's next turn, well
                // before this wait ends.
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            },
        )
        .await;
        drop(output);
        let written = String::from_utf8_lossy(&standard_error);
        assert_eq!(
            written,
            "\rWaiting for queued work to make room (at most 2 min)\x1b[K\r\x1b[K"
        );
        assert_eq!(
            waiting_line(AdmissionWaitCause::CapacityInUse),
            "Waiting for capacity"
        );
    }

    fn media(done_s: u64, total_s: Option<u64>, fresh_s: u64, wall_s: u64) -> MediaProgress {
        MediaProgress {
            from_us: 0,
            done_until_us: done_s * 1_000_000,
            to_us: total_s.map(|total| total * 1_000_000),
            elapsed_us: wall_s * 1_000_000,
            fresh_media_us: fresh_s * 1_000_000,
            fresh_elapsed_us: wall_s * 1_000_000,
            fresh_chunks: 4,
        }
    }

    #[test]
    fn the_line_says_where_how_fast_and_how_long_in_words() {
        // 34:10 of 1:30:00 at 2.1x: the 3,350 s left take about 1,595 s.
        let line = media_sentence(&media(2_050, Some(5_400), 2_050, 976), true);
        assert_eq!(
            line,
            "34:10 of 1:30:00 (37%), 2.1x real time, about 27 min left (estimate)"
        );
        // No end: no percent and no estimate.
        assert_eq!(
            media_sentence(&media(65, None, 65, 30), true),
            "1:05, 2.2x real time"
        );
        // Not estimating (a job no process runs): no estimate.
        assert_eq!(
            media_sentence(&media(100, Some(600), 100, 50), false),
            "1:40 of 10:00 (16%), 2.0x real time"
        );
        assert_eq!(
            media_sentence(&media(590, Some(600), 590, 295), true),
            "9:50 of 10:00 (98%), 2.0x real time, under a minute left (estimate)"
        );
        assert_eq!(
            media_sentence(&media(600, Some(30_000), 600, 600), true),
            "10:00 of 8:20:00 (2%), 1.0x real time, about 8 h 10 min left (estimate)"
        );
    }

    #[test]
    fn only_recognition_shows_a_line() {
        let progress = |stage, media| JobProgress {
            job: None,
            update: ProgressUpdate {
                stage,
                completed: 3,
                total: Some(12),
                media,
            },
        };
        assert_eq!(
            line_for(&progress(ProgressStage::RecognisingSpeech, None)).as_deref(),
            Some("Transcribing: chunk 3 of 12")
        );
        assert_eq!(
            line_for(&progress(ProgressStage::CopyingSource, None)),
            None
        );
        assert!(
            line_for(&progress(
                ProgressStage::RecognisingSpeech,
                Some(media(100, Some(600), 100, 50))
            ))
            .is_some_and(|line| line.starts_with("Transcribing 1:40 of 10:00"))
        );
    }
}
