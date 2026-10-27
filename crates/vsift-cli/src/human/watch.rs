//! The finish screen of `vsift <video>` (ADR 0037 section 9): what was
//! written, in a person's words, with the next step last.

use super::{
    text::{RenderedText, TerminalText, TooLarge},
    view::{Envelope, Watched},
};

/// Renders a finished `watch`.
pub(super) fn finished(envelope: &Envelope<Watched>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("Done: ")
        .push_value(&readable(data.duration_us))
        .push_fixed(" of video")
        .end_line();
    text.blank_line();
    text.push_fixed("  ")
        .push_value(&data.outputs.transcript.name)
        .push_fixed("   readable transcript")
        .end_line();
    if let Some(subtitles) = &data.outputs.subtitles {
        text.push_fixed("  ")
            .push_value(&subtitles.name)
            .push_fixed("   subtitles (most video players pick them up)")
            .end_line();
    }
    let frames = u64::try_from(data.frames.len()).unwrap_or(u64::MAX);
    text.push_fixed("  ")
        .push_value(&data.outputs.folder.name)
        .push_fixed("/   ")
        .push_unsigned(data.changes_found)
        .push_fixed(" screen changes");
    if frames == data.changes_found {
        text.push_fixed(" with their frames");
    } else {
        text.push_fixed(", ")
            .push_unsigned(frames)
            .push_fixed(" frames written, spread evenly");
    }
    text.end_line();
    text.blank_line();
    for range in &data.not_transcribed {
        text.push_fixed("Not transcribed: ")
            .push_value(&readable(range.from_us))
            .push_fixed(" to ")
            .push_value(&readable(range.to_us))
            .push_fixed(" (the transcript says so in place)")
            .end_line();
    }
    if data.screen_changes_beyond_bound {
        text.push_fixed("Screen changes were found in the first four hours only.")
            .end_line();
    }
    if data.names_shortened {
        text.push_fixed(
            "The video's name was too long, so the files have a shorter one; a player will not pick the subtitles up by itself.",
        )
        .end_line();
    }
    text.push_fixed("Each file names the video's SHA-256; the frame list gives every frame's.")
        .end_line();
    text.push_fixed("More: vsift watch --help").end_line();
    text.finish()
}

/// A time as a person reads it, `HH:MM:SS`.
fn readable(micros: u64) -> String {
    vsift::readable_time(vsift::MediaTime::from_micros(micros))
}
