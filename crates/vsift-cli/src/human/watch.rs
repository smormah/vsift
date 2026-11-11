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
        .push_unsigned(data.changes_listed);
    if data.changes_listed < data.changes_found {
        text.push_fixed(" settled screens of ")
            .push_unsigned(data.changes_found)
            .push_fixed(" changes");
    } else {
        text.push_fixed(" screen changes");
    }
    if frames == data.changes_listed {
        text.push_fixed(" with their frames");
    } else {
        text.push_fixed(", ")
            .push_unsigned(frames)
            .push_fixed(" frames written, spread evenly");
    }
    text.end_line();
    if let Some(kept) = data
        .kept
        .as_ref()
        .filter(|kept| kept.failure_code.is_none())
    {
        text.push_fixed("  ")
            .push_value(&kept.name)
            .push_fixed("/   kept for later search, in the folder you named")
            .end_line();
    }
    text.blank_line();
    for range in &data.not_transcribed {
        text.push_fixed("Not transcribed: ")
            .push_value(&readable(range.from_us))
            .push_fixed(" to ")
            .push_value(&readable(range.to_us))
            .push_fixed(" (the transcript says so in place)")
            .end_line();
    }
    push_frame_gaps(&mut text, data);
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
    if let Some(kept) = &data.kept {
        if let Some(code) = &kept.failure_code {
            text.push_fixed("Not kept for later search (")
                .push_value(code)
                .push_fixed("): VSift left the session open, with its private copy of the video, until it expires about a day unused.")
                .end_line();
            text.push_fixed("Keep it now with: vsift session retain ")
                .push_value(&data.session_id)
                .push_fixed(" --output <a new folder>")
                .end_line();
            text.push_fixed("Anything left at the kept name is incomplete and can be removed.")
                .end_line();
        } else {
            text.push_fixed("Search what you keep with: vsift bundle search <that folder>")
                .end_line();
        }
    }
    text.push_fixed("Each file names the video's SHA-256; the frame list gives every frame's.")
        .end_line();
    text.push_fixed("More: vsift watch --help").end_line();
    text.finish()
}

/// Why some listed screens have no frame: the session ran out of room, or
/// other `VSift` work held the computer (Fable review C-2).
fn push_frame_gaps(text: &mut TerminalText, data: &Watched) {
    if data.frames_stopped_for_room {
        text.push_fixed(
            "VSift's private copy ran out of room for frames, so they stop early; every settled screen is listed.",
        )
        .end_line();
    }
    if data.frames_skipped_for_capacity > 0 {
        // A finished run's files are never replaced, so the same command
        // would be refused: only a run into another folder takes them.
        let (noun, verb) = if data.frames_skipped_for_capacity == 1 {
            (" frame", " was")
        } else {
            (" frames", " were")
        };
        text.push_unsigned(data.frames_skipped_for_capacity)
            .push_fixed(noun)
            .push_fixed(verb)
            .push_fixed(
                " not saved because the computer was busy with other VSift work; their screens are listed without a frame. To get them, run the command again with --output <another folder> (it does the whole run again).",
            )
            .end_line();
    }
}

/// A time as a person reads it, `HH:MM:SS`.
fn readable(micros: u64) -> String {
    vsift::readable_time(vsift::MediaTime::from_micros(micros))
}

#[cfg(test)]
mod tests {
    use super::finished;
    use crate::human::view::{Envelope, Watched};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn screen(kept: &serde_json::Value) -> Result<String, Box<dyn std::error::Error>> {
        screen_skipping(kept, 0)
    }

    fn screen_skipping(
        kept: &serde_json::Value,
        skipped: u64,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let file = serde_json::json!({ "name": "call.txt" });
        let envelope: Envelope<Watched> = serde_json::from_value(serde_json::json!({
            "status": "complete",
            "operation_id": null,
            "data": {
                "session_id": "ses_0123456789abcdef",
                "duration_us": 60_000_000,
                "not_transcribed": [],
                "screen_changes_beyond_bound": false,
                "frames_stopped_for_room": false,
                "frames_skipped_for_capacity": skipped,
                "names_shortened": false,
                "outputs": { "transcript": file, "subtitles": null, "folder": file },
                "changes_found": 1,
                "changes_listed": 1,
                "frames": [{}],
                "kept": kept,
            },
            "warnings": [],
            "lifecycle": null,
        }))?;
        Ok(finished(&envelope)
            .map_err(|_| "too large")?
            .as_str()
            .to_owned())
    }

    /// Fable review C-2: frames skipped while the computer was busy are
    /// counted, and the next step is a run into another folder, never the
    /// same command, which a finished run's files would refuse.
    #[test]
    fn skipped_frames_are_counted_with_what_to_do() -> TestResult {
        let text = screen_skipping(&serde_json::Value::Null, 1)?;
        assert!(
            text.contains("1 frame was not saved because the computer was busy"),
            "{text}"
        );
        assert!(text.contains("--output <another folder>"), "{text}");
        let text = screen_skipping(&serde_json::Value::Null, 3)?;
        assert!(text.contains("3 frames were not saved"), "{text}");
        let text = screen(&serde_json::Value::Null)?;
        assert!(!text.contains("not saved"), "{text}");
        Ok(())
    }

    #[test]
    fn a_kept_recording_says_so_and_how_to_find_it() -> TestResult {
        let text = screen(&serde_json::json!({ "name": "call", "failure_code": null }))?;
        assert!(text.contains("call/   kept for later search"), "{text}");
        assert!(text.contains("vsift bundle search"), "{text}");
        Ok(())
    }

    #[test]
    fn no_option_says_nothing_about_keeping() -> TestResult {
        let text = screen(&serde_json::Value::Null)?;
        assert!(!text.contains("kept"), "{text}");
        assert!(!text.contains("Not kept"), "{text}");
        Ok(())
    }

    #[test]
    fn a_bundle_not_written_gives_the_command_with_the_session() -> TestResult {
        let text = screen(&serde_json::json!({
            "name": "call",
            "failure_code": "STORAGE_IO",
        }))?;
        assert!(
            text.contains("Not kept for later search (STORAGE_IO)"),
            "{text}"
        );
        assert!(text.contains("private copy of the video"), "{text}");
        assert!(
            text.contains("vsift session retain ses_0123456789abcdef --output <a new folder>"),
            "{text}"
        );
        assert!(!text.contains("call/"), "{text}");
        Ok(())
    }
}
