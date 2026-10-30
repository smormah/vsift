use std::{error::Error, ffi::OsString, fs};

use clap::Parser as _;

use vsift::EngineError;
use vsift_contract::CommandName;

use super::{WatchFailure, route_bare_video};
use crate::command::{Cli, Command};

type TestResult = Result<(), Box<dyn Error>>;

fn line(arguments: &[&str]) -> Vec<OsString> {
    arguments.iter().map(OsString::from).collect()
}

fn routed(arguments: &[&str]) -> Vec<String> {
    route_bare_video(line(arguments))
        .into_iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

/// A folder of its own under the temporary folder, with a video and a
/// folder in it; removed when the test ends.
struct Place(std::path::PathBuf);

impl Place {
    fn new(name: &str) -> Result<Self, Box<dyn Error>> {
        let path =
            std::env::temp_dir().join(format!("vsift-watch-route-{name}-{}", std::process::id()));
        fs::create_dir_all(path.join("a folder"))?;
        fs::write(path.join("meeting.mp4"), b"not really a video")?;
        Ok(Self(path))
    }

    fn text(&self, child: &str) -> Result<String, Box<dyn Error>> {
        Ok(self.0.join(child).to_str().ok_or("not Unicode")?.to_owned())
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn an_existing_video_becomes_watch() -> TestResult {
    let place = Place::new("file")?;
    let video = place.text("meeting.mp4")?;
    assert_eq!(
        routed(&["vsift", &video]),
        ["vsift", "watch", video.as_str()]
    );
    let parsed = Cli::try_parse_from(route_bare_video(line(&["vsift", &video, "--output", "x"])))?;
    assert!(matches!(parsed.command, Some(Command::Watch(_))));
    Ok(())
}

#[test]
fn an_existing_folder_becomes_watch_so_ingest_refuses_it() -> TestResult {
    let place = Place::new("folder")?;
    let folder = place.text("a folder")?;
    assert_eq!(
        routed(&["vsift", &folder]),
        ["vsift", "watch", folder.as_str()]
    );
    Ok(())
}

#[test]
fn a_missing_video_that_looks_like_a_path_becomes_watch() {
    assert_eq!(
        routed(&["vsift", "meetng.mp4"]),
        ["vsift", "watch", "meetng.mp4"]
    );
    assert_eq!(routed(&["vsift", "./clip"]), ["vsift", "watch", "./clip"]);
    assert_eq!(
        routed(&["vsift", "talk.webm"]),
        ["vsift", "watch", "talk.webm"]
    );
}

#[test]
fn a_mistyped_command_stays_a_mistyped_command() {
    assert_eq!(routed(&["vsift", "serach", "x"]), ["vsift", "serach", "x"]);
    assert_eq!(routed(&["vsift", "transcrpt"]), ["vsift", "transcrpt"]);
    let hostile = "x\u{202E}/y.mp4";
    assert_eq!(routed(&["vsift", hostile]), ["vsift", hostile]);
}

#[test]
fn a_command_name_always_wins() {
    for command in ["setup", "ingest", "watch", "job"] {
        assert_eq!(routed(&["vsift", command]), ["vsift", command]);
    }
}

#[test]
fn global_options_before_the_video_are_skipped_in_both_forms() -> TestResult {
    let place = Place::new("globals")?;
    let video = place.text("meeting.mp4")?;
    assert_eq!(
        routed(&["vsift", "--json", &video]),
        ["vsift", "--json", "watch", video.as_str()]
    );
    assert_eq!(
        routed(&["vsift", "--session-root", "root", &video]),
        ["vsift", "--session-root", "root", "watch", video.as_str()]
    );
    assert_eq!(
        routed(&["vsift", "--events=jsonl", &video]),
        ["vsift", "--events=jsonl", "watch", video.as_str()]
    );
    Ok(())
}

#[test]
fn options_the_parser_owns_are_left_alone() {
    assert_eq!(routed(&["vsift", "--help"]), ["vsift", "--help"]);
    assert_eq!(routed(&["vsift", "-V"]), ["vsift", "-V"]);
    assert_eq!(routed(&["vsift"]), ["vsift"]);
    assert_eq!(
        routed(&["vsift", "--unknown", "a.mp4"]),
        ["vsift", "--unknown", "a.mp4"]
    );
}

fn failed_after_keep(failure: Option<vsift::FailureCode>) -> Result<EngineError, Box<dyn Error>> {
    Ok(EngineError::WatchFailedAfterKeep(Box::new(
        vsift::WatchFailedAfterKeep {
            error: EngineError::Watch(WatchFailure::OutputHeld(vsift::WatchOutputKind::Transcript)),
            kept: vsift::WatchKept {
                path: std::env::temp_dir().join("kept").join("meeting"),
                failure,
            },
            session: vsift::SessionId::parse("ses_0123456789abcdef")?,
            session_closed: failure.is_none(),
        },
    )))
}

#[test]
fn a_failure_after_the_seal_keeps_its_own_code_and_says_what_was_kept() -> TestResult {
    let response =
        crate::failure_response(CommandName::Watch, super::failure(failed_after_keep(None)?));
    let value = serde_json::to_value(&response)?;
    assert_eq!(value["error"]["code"], "STORAGE_IO", "{value}");
    assert_eq!(value["data"]["kept"]["state"], "kept", "{value}");
    assert_eq!(value["data"]["kept"]["name"], "meeting", "{value}");
    assert_eq!(value["data"]["session_closed"], true, "{value}");
    let words = value["error"]["remediation"][0]["summary"].to_string();
    assert!(words.contains("still kept for later search"), "{words}");
    Ok(())
}

#[test]
fn a_failure_with_no_bundle_gives_the_retain_command_with_the_real_session() -> TestResult {
    let response = crate::failure_response(
        CommandName::Watch,
        super::failure(failed_after_keep(Some(vsift::FailureCode::StorageIo))?),
    );
    let value = serde_json::to_value(&response)?;
    assert_eq!(value["data"]["kept"]["state"], "not_kept", "{value}");
    assert_eq!(
        value["data"]["kept"]["failure_code"], "STORAGE_IO",
        "{value}"
    );
    assert_eq!(value["data"]["session_closed"], false, "{value}");
    let words = value["error"]["remediation"][0]["summary"].to_string();
    assert!(
        words.contains("vsift session retain ses_0123456789abcdef --output"),
        "{words}"
    );
    assert!(words.contains("private copy of the video"), "{words}");
    Ok(())
}

/// The option parses on both forms of the command.
#[test]
fn keep_is_an_option_of_the_action() -> TestResult {
    for line in [
        ["vsift", "watch", "call.mp4", "--keep", "kept"],
        ["vsift", "call.mp4", "--keep", "kept", ""],
    ] {
        let arguments: Vec<&str> = line.into_iter().filter(|word| !word.is_empty()).collect();
        let parsed = Cli::try_parse_from(route_bare_video_for_test(&arguments))?;
        let Some(Command::Watch(arguments)) = parsed.command else {
            return Err("not the action".into());
        };
        assert_eq!(
            arguments.keep.as_deref(),
            Some(std::path::Path::new("kept"))
        );
    }
    Ok(())
}

fn route_bare_video_for_test(arguments: &[&str]) -> Vec<OsString> {
    let line = arguments.iter().map(OsString::from).collect();
    route_bare_video(line)
}
