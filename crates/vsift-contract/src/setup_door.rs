//! The `setup` result: one unattended run of the setup door, `vsift setup
//! --unattended --install … --json` (ADR 0036 section 6).
//!
//! It says whether every part the caller named works after the run, where
//! each core part and each included extra stands, and what the install did
//! when one ran. An error (a part `VSift` cannot install here, checks that
//! ran out of time, a busy install, a plan whose digest is not the one
//! given) is the ordinary failure envelope, never this result.
//!
//! A pass is a `complete` envelope (exit 0). A fail is a `failed` envelope
//! whose `error.code` has the class the process exits with, the install's own
//! failure code or `MISSING_CAPABILITY`, and whose `data` is still this
//! result (the planner's ruling on the Fable 0.3.0 review, P-2, amending
//! ADR 0036): every non-zero exit carries an error of its class.

use serde::{Deserialize, Serialize};
use vsift_domain::FailureCode;

use crate::{CommandName, OperationResponse};

/// Whether every named part works after the run.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupOutcome {
    /// Every named part works, also when nothing needed installing (exit 0).
    Pass,
    /// The install ran and a named part does not work; its entry says what
    /// to do next.
    Fail,
}

/// A part of the setup, as the result names it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupPartName {
    /// `FFmpeg` with `FFprobe`.
    Video,
    /// whisper.cpp and its model.
    Speech,
    /// The optional extra that fetches videos from public web pages
    /// (`online_links` in `setup check` and `setup plan`).
    WebVideos,
}

/// Where one part stands after the run.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupPartStatus {
    /// A core part that works.
    Ready,
    /// A core part that does not work.
    NotSetUp,
    /// An extra that is installed.
    Added,
    /// An extra that is not installed.
    NotAdded,
    /// An extra `VSift` cannot add on this computer.
    NotAvailableHere,
}

/// One core part or included extra after the run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupPartResult {
    /// The part.
    pub part: SetupPartName,
    /// Whether the caller named it, with `--install` or `--include`.
    pub requested: bool,
    /// Where it stands now.
    pub status: SetupPartStatus,
    /// The one thing to do about it, in fixed prose; `null` when nothing is
    /// needed.
    pub next_step: Option<String>,
}

/// The data of a `setup` result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupResponse {
    /// Whether every named part works.
    pub outcome: SetupOutcome,
    /// Video, speech, then each included extra.
    pub parts: Vec<SetupPartResult>,
    /// The `setup.install` data of the install the run made, exactly as
    /// `setup install` reports it; `null` when nothing needed installing.
    pub install: Option<serde_json::Value>,
}

/// The next step of a part that a connection problem stopped.
pub const SETUP_NEXT_CONNECTION: &str =
    "Check the network connection and any proxy, then run the same command again.";

/// The next step of a part that could not be saved.
pub const SETUP_NEXT_SPACE: &str =
    "Make room in the user's data folder, then run the same command again.";

/// The next step of a part that failed for another reason.
pub const SETUP_NEXT_AGAIN: &str = "Run the same command again; if it fails again, see the \
     troubleshooting guide: https://github.com/smormah/vsift/blob/main/docs/guide/troubleshooting.md";

/// The next step of speech when video did not install: its verified
/// download is kept for the next run.
pub const SETUP_NEXT_AFTER_VIDEO: &str = "Speech waits for video: once video installs, run the \
     same command again. The checked speech download is kept, so it is not downloaded again.";

/// The next step of a run that was cancelled.
pub const SETUP_NEXT_CANCELLED: &str = "The run was stopped; run the same command again.";

/// The remediation of a fail: the install ran and a named part does not
/// work; each part's entry in `data` says what to do next.
pub const SETUP_FAILED_REMEDIATION: &str = "The install ran, but a part you named does not work yet. data.parts says which part, with its next step; data.install says what the install did.";

/// The failure code of a run with this outcome: `None` for a pass; for a
/// fail, the install's own failure code when it had one, otherwise
/// `MISSING_CAPABILITY` (a named part still does not work). The process
/// exits with this code's class, so the envelope and the exit agree.
#[must_use]
pub fn setup_failure_code(
    outcome: SetupOutcome,
    install_failure: Option<FailureCode>,
) -> Option<FailureCode> {
    match outcome {
        SetupOutcome::Pass => None,
        SetupOutcome::Fail => Some(install_failure.unwrap_or(FailureCode::MissingCapability)),
    }
}

/// The `setup` envelope: `complete` for a pass; for a fail, a `failed`
/// envelope with [`setup_failure_code`]'s code and
/// [`SETUP_FAILED_REMEDIATION`], whose `data` is the same result.
///
/// # Errors
///
/// Returns the serialization error when the result cannot be represented as
/// JSON.
pub fn setup_door_response(
    result: &SetupResponse,
    install_failure: Option<FailureCode>,
) -> Result<OperationResponse<serde_json::Value>, serde_json::Error> {
    let command = CommandName::Setup.identifier();
    match setup_failure_code(result.outcome, install_failure) {
        None => OperationResponse::complete(command, result),
        Some(code) => OperationResponse::failure_with_remediation(
            command,
            code,
            SETUP_FAILED_REMEDIATION.to_owned(),
        )
        .with_failure_data(result),
    }
}

/// The next step of a core part that was not named and does not work.
#[must_use]
pub fn setup_next_install(part: SetupPartName) -> String {
    let name = match part {
        SetupPartName::Video => "video",
        SetupPartName::Speech => "speech",
        SetupPartName::WebVideos => return String::from("vsift setup --include web-videos"),
    };
    format!("vsift setup --unattended --install {name}")
}
