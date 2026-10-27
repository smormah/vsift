use std::{error::Error, ffi::OsString, fs};

use clap::Parser as _;

use super::route_bare_video;
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
