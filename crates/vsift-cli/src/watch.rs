//! `vsift <video>`, explicitly `vsift watch <video>` (ADR 0037): the
//! bare-argument rule that makes the first form the second, and the command
//! that presents [`Engine::watch`].

use std::{ffi::OsString, path::Path};

use clap::CommandFactory as _;
use vsift::{Cancellation, Engine, ProgressObserver, WatchFailure, WatchRequest};
use vsift_contract::{
    CommandName, OperationResponse, WatchCompleteness, WatchData, WatchFrameData, WatchOutputData,
    WatchOutputsData, WatchRangeData,
};

use crate::{CommandFailure, command::Cli, command::WatchArguments};

type Response = OperationResponse<serde_json::Value>;

/// The explicit form's name, inserted by the bare-argument rule.
const WATCH: &str = "watch";

/// Applies the bare-argument rule (ADR 0037 section 2) to a whole command
/// line, program name first: when the first argument that is not a global
/// option is not a command name and either names something that exists or
/// looks like a path, `watch` is inserted before it. Nothing is reordered,
/// rewritten or echoed; any other line is returned as it was.
pub(crate) fn route_bare_video(arguments: Vec<OsString>) -> Vec<OsString> {
    let Some(position) = first_operand(&arguments) else {
        return arguments;
    };
    let Some(candidate) = arguments.get(position) else {
        return arguments;
    };
    if is_command_name(candidate) || !names_a_video(candidate) {
        return arguments;
    }
    let mut routed = arguments;
    routed.insert(position, OsString::from(WATCH));
    routed
}

/// The position of the first argument that is neither the program name nor
/// a global option (with its value, in `--name value` or `--name=value`
/// form). An argument that starts with `-` and is not a global option ends
/// the search: it belongs to the parser.
fn first_operand(arguments: &[OsString]) -> Option<usize> {
    let globals = Cli::command();
    let mut position = 1;
    while let Some(argument) = arguments.get(position) {
        let text = argument.to_str()?;
        if !text.starts_with('-') {
            return Some(position);
        }
        let name = text.trim_start_matches('-');
        let (name, inline_value) = match name.split_once('=') {
            Some((name, _)) => (name, true),
            None => (name, false),
        };
        let option = globals
            .get_arguments()
            .find(|candidate| candidate.is_global_set() && candidate.get_long() == Some(name))?;
        // The action, not the value count: the count is set only once the
        // parser is built, the action is declared.
        let takes_value = option.get_action().takes_values();
        position += if takes_value && !inline_value { 2 } else { 1 };
    }
    None
}

fn is_command_name(argument: &OsString) -> bool {
    argument.to_str().is_some_and(|text| {
        Cli::command().get_subcommands().any(|command| {
            command.get_name() == text || command.get_all_aliases().any(|alias| alias == text)
        })
    })
}

/// Whether an argument that is not a command means a video: it names
/// something that exists (a file, a folder or a link, which `ingest` then
/// accepts or refuses with its own answer), or it looks like a path, with a
/// path separator or a short extension, so that a mistyped video is "not
/// found" rather than "unknown command".
fn names_a_video(argument: &OsString) -> bool {
    let path = Path::new(argument);
    if std::fs::symlink_metadata(path).is_ok() {
        return true;
    }
    let Some(text) = argument.to_str() else {
        return false;
    };
    // A path a person typed never holds a control or hidden character; text
    // that does stays the parser's to reject, inert (SEC-T02).
    if text
        .chars()
        .any(|character| character.is_control() || vsift_contract::is_hidden_character(character))
    {
        return false;
    }
    let separator = text.contains('/') || (cfg!(windows) && text.contains('\\'));
    let extension = text.rsplit_once('.').is_some_and(|(stem, extension)| {
        !stem.is_empty()
            && (1..=5).contains(&extension.len())
            && extension
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
    });
    separator || extension
}

/// Watches one video and presents what the run wrote.
pub(crate) async fn watch(
    engine: &Engine,
    arguments: &WatchArguments,
    cancellation: &Cancellation,
    progress: ProgressObserver,
) -> Result<Response, CommandFailure> {
    let outcome = engine
        .watch(WatchRequest {
            source: arguments.source.clone(),
            output: arguments.output.clone(),
            cancellation: cancellation.clone(),
            progress,
        })
        .await?;
    let output = |file: &vsift::WatchFile| {
        WatchOutputData::new(&file.path, file.sha256.clone())
            .ok_or_else(|| CommandFailure::from(vsift::FailureCode::Internal))
    };
    let frames = outcome
        .frames
        .iter()
        .map(|frame| {
            Ok(WatchFrameData {
                time_us: frame.time.as_micros(),
                candidate_id: frame.candidate_id.as_str().to_owned(),
                evidence_id: frame.evidence_id.as_str().to_owned(),
                path: output(&vsift::WatchFile {
                    path: frame.path.clone(),
                    sha256: None,
                })?
                .path,
                sha256: frame.sha256.clone(),
            })
        })
        .collect::<Result<Vec<_>, CommandFailure>>()?;
    let partial = !outcome.not_transcribed.is_empty()
        || outcome.beyond_visual_bound
        || outcome.frames_stopped_for_room;
    let data = WatchData {
        source_id: outcome.source_id.as_str().to_owned(),
        session_id: outcome.session_id.as_str().to_owned(),
        session_closed: outcome.session_closed,
        job_id: outcome.job_id.as_str().to_owned(),
        duration_us: outcome.duration.as_micros(),
        transcript_revision_id: outcome
            .transcript_revision
            .as_ref()
            .map(|revision| revision.as_str().to_owned()),
        visual_index_revision_id: outcome
            .visual_index
            .as_ref()
            .map(|index| index.as_str().to_owned()),
        outcome: if partial {
            WatchCompleteness::Partial
        } else {
            WatchCompleteness::Complete
        },
        not_transcribed: outcome
            .not_transcribed
            .iter()
            .map(|range| WatchRangeData {
                from_us: range.start().as_micros(),
                to_us: range.end().as_micros(),
            })
            .collect(),
        screen_changes_beyond_bound: outcome.beyond_visual_bound,
        frames_stopped_for_room: outcome.frames_stopped_for_room,
        names_shortened: outcome.names_shortened,
        outputs: WatchOutputsData {
            transcript: output(&outcome.transcript)?,
            subtitles: outcome.subtitles.as_ref().map(output).transpose()?,
            folder: output(&outcome.folder)?,
            screen_changes: output(&outcome.screen_changes)?,
        },
        changes_found: u64::try_from(outcome.changes_found).unwrap_or(u64::MAX),
        frames,
    };
    OperationResponse::complete(CommandName::Watch.identifier(), &data)
        .map_err(|_| CommandFailure::from(vsift::FailureCode::Internal))
}

/// Fixed-prose remediation for a `watch` failure; never a path or a name.
pub(crate) fn remediation(failure: WatchFailure) -> &'static str {
    use vsift::{OutputNameError, WatchOutputKind};
    use vsift_contract::{
        WATCH_FOLDER_EXISTS_REMEDIATION, WATCH_FRAME_MISMATCH_REMEDIATION,
        WATCH_NAME_EMPTY_REMEDIATION, WATCH_NAME_NOT_UNICODE_REMEDIATION,
        WATCH_OUTPUT_FOLDER_REMEDIATION, WATCH_OUTPUT_INSIDE_VSIFT_REMEDIATION,
        WATCH_OUTPUT_REFUSED_REMEDIATION, WATCH_SUBTITLES_EXISTS_REMEDIATION,
        WATCH_TRANSCRIPT_EXISTS_REMEDIATION,
    };
    match failure {
        WatchFailure::UnusableName(OutputNameError::NotUnicode) => {
            WATCH_NAME_NOT_UNICODE_REMEDIATION
        }
        WatchFailure::UnusableName(OutputNameError::Empty) => WATCH_NAME_EMPTY_REMEDIATION,
        WatchFailure::OutputExists(WatchOutputKind::Transcript) => {
            WATCH_TRANSCRIPT_EXISTS_REMEDIATION
        }
        WatchFailure::OutputExists(WatchOutputKind::Subtitles) => {
            WATCH_SUBTITLES_EXISTS_REMEDIATION
        }
        WatchFailure::OutputExists(WatchOutputKind::Folder | WatchOutputKind::FolderContent) => {
            WATCH_FOLDER_EXISTS_REMEDIATION
        }
        WatchFailure::OutputRefused(_) => WATCH_OUTPUT_REFUSED_REMEDIATION,
        WatchFailure::OutputInsideVsift => WATCH_OUTPUT_INSIDE_VSIFT_REMEDIATION,
        WatchFailure::OutputFolderUnusable => WATCH_OUTPUT_FOLDER_REMEDIATION,
        WatchFailure::FrameMismatch => WATCH_FRAME_MISMATCH_REMEDIATION,
    }
}

#[cfg(test)]
mod tests;
