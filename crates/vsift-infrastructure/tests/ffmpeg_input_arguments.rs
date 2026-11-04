//! Every `FFmpeg` and `FFprobe` command the media adapter builds for a source
//! forces its input: `-protocol_whitelist file`, then `-f` with the source's
//! own demuxer (`mov` or `matroska`), before `-i`.
//!
//! Why it matters: `FFmpeg` picks a demuxer by probing the file when `-f` is
//! absent, and the reviewed build links demuxers `VSift` never reviewed (the
//! `vapoursynth` one, added with the `0.2.4` re-pin, loads a script library at
//! run time; L-227 and the re-pin's component review rest on the same
//! arguments). A hostile file must not be able to choose its own demuxer.
//!
//! Why it reads the source. Four of the five commands are assembled inline in
//! the async methods that also run the provider (`probe`, `frame`,
//! `visual_samples`, `decode_pcm` in `ffmpeg_media.rs`; `evidence_request` in
//! `ffmpeg_media/evidence.rs`), so there is no builder to call without running
//! `FFmpeg`. The test reads the adapter's production code (each file up to its
//! `#[cfg(test)]` module) and holds every `"-i"` to the rule; it also counts
//! them, so a new command fails here until this test names it.

use vsift_infrastructure::SourceContainer;

const MEDIA: &str = include_str!("../src/ffmpeg_media.rs");
const EVIDENCE: &str = include_str!("../src/ffmpeg_media/evidence.rs");

/// The production part of a source file, with every run of whitespace made one space.
fn production(source: &str) -> Result<String, String> {
    let end = source
        .find("\n#[cfg(test)]")
        .ok_or("the file has no test module marker")?;
    Ok(source[..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" "))
}

/// For each `"-i"`, the text of its command from the request that starts it.
fn commands(code: &str) -> Result<Vec<&str>, String> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = code[from..].find("\"-i\"") {
        let input = from + offset;
        let start = code[..input]
            .rfind("Self::request(")
            .ok_or("an input with no request before it")?;
        found.push(&code[start..input]);
        from = input + 4;
    }
    Ok(found)
}

/// The rule for one command; `Err` names what is wrong.
fn forces_its_input(command: &str) -> Result<(), String> {
    const WHITELIST: &str = "\"-protocol_whitelist\", \"file\"";
    const DEMUXER: &str = "\"-f\", source.container().demuxer()";
    let whitelist = command
        .find(WHITELIST)
        .ok_or("no -protocol_whitelist file")?;
    let first_format = command.find("\"-f\"").ok_or("no -f before -i")?;
    if !command[first_format..].starts_with(DEMUXER) {
        return Err("the first -f is not the source's demuxer".into());
    }
    if whitelist > first_format {
        return Err("-protocol_whitelist comes after -f".into());
    }
    if command[first_format + 1..].contains("\"-f\"") {
        return Err("another -f between the demuxer and -i".into());
    }
    Ok(())
}

#[test]
fn every_source_input_is_forced_to_file_and_its_own_demuxer() -> Result<(), String> {
    let media = production(MEDIA)?;
    let evidence = production(EVIDENCE)?;
    let media_commands = commands(&media)?;
    let evidence_commands = commands(&evidence)?;
    // probe, frame, visual_samples and decode_pcm; evidence_request.
    assert_eq!(media_commands.len(), 4, "the inputs of ffmpeg_media.rs");
    assert_eq!(evidence_commands.len(), 1, "the inputs of evidence.rs");
    for command in media_commands.iter().chain(&evidence_commands) {
        forces_its_input(command).map_err(|problem| format!("{problem}: {command}"))?;
    }
    Ok(())
}

#[test]
fn the_rule_refuses_a_command_without_the_forced_input() {
    let good = "Self::request(x)? .with_arguments([\"-protocol_whitelist\", \"file\"]) \
                .with_arguments([\"-f\", source.container().demuxer(), ";
    assert_eq!(forces_its_input(good), Ok(()));
    let no_format = "Self::request(x)? .with_arguments([\"-protocol_whitelist\", \"file\"]) ";
    assert!(forces_its_input(no_format).is_err());
    let no_whitelist = "Self::request(x)? .with_arguments([\"-f\", source.container().demuxer(), ";
    assert!(forces_its_input(no_whitelist).is_err());
    let other_format = "Self::request(x)? .with_arguments([\"-protocol_whitelist\", \"file\"]) \
                        .with_arguments([\"-f\", \"vapoursynth\", ";
    assert!(forces_its_input(other_format).is_err());
    let overridden = "Self::request(x)? .with_arguments([\"-protocol_whitelist\", \"file\"]) \
                      .with_arguments([\"-f\", source.container().demuxer(), \"-f\", \"lavfi\", ";
    assert!(forces_its_input(overridden).is_err());
}

#[test]
fn the_source_demuxers_are_mov_and_matroska_only() {
    // Exhaustive: a new container fails to compile here until it is reviewed.
    for container in [SourceContainer::IsoMedia, SourceContainer::Matroska] {
        let expected = match container {
            SourceContainer::IsoMedia => "mov",
            SourceContainer::Matroska => "matroska",
        };
        assert_eq!(container.demuxer(), expected);
    }
}
