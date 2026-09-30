//! Human-readable terminal output, the default without `--json` or
//! `--events` (ADR 0008; ADR 0023 decision H2, P13 PRs 2a and 2b).
//!
//! One renderer per command turns a result into readable text through the
//! one [`TerminalText`] builder, which replaces control characters, shows
//! hidden characters as `<U+XXXX>` and bounds the output. The rules every
//! renderer follows:
//!
//! - no ANSI colour and no OSC-8 link (R0 has neither), and no TTY
//!   detection: the text is the same when piped;
//! - identifiers are never cut, and each is written whole;
//! - evidence text is labelled untrusted and quoted only from `display_text`
//!   or `display_label` (the views in `view` cannot hold raw text), each
//!   line after the quote prefix `  | `;
//! - a failure is its fixed message and code, then each remediation's
//!   summary and suggested command (fixed words and validated identifiers),
//!   on stderr;
//! - a delivered file path goes on a line of its own, through the builder's
//!   path entry, never cut (L-016);
//! - a worker host (`job run`, `job batch`) renders its final result; its
//!   event stream is `--events jsonl` only.
//!
//! Human text is unstable and not for parsing (`docs/contracts/cli-v1.md`);
//! agents use `--json`.
//!
//! PR 2a rendered setup, session, ingest, transcript, search and bundle
//! results, every failure and rejected command lines; PR 2b renders
//! `candidates`, the frame commands, `crop`, `audio`, the `job` commands and
//! the worker hosts. Every command that completes has a renderer: [`result`]
//! answers `None` only for `setup check` (rendered from its typed report by
//! [`setup_check`]) and the commands that only ever fail.

mod evidence;
mod failure;
mod handoff;
mod job;
mod session;
mod setup;
mod text;
mod time;
mod transcript;
mod view;

use serde::Deserialize;
use vsift_contract::{CommandName, OperationResponse};

pub(crate) use failure::{HumanDetail, failure};
pub(crate) use setup::setup_check;
use text::TerminalText;
pub(crate) use text::{DisplayText, RenderedText};

use crate::output::OutputError;

/// The human text of a worker host's final response.
#[derive(Debug)]
pub(crate) struct HostText {
    /// The job result or batch summary, for stdout; a failed or cancelled
    /// request has one too.
    pub(crate) result: Option<RenderedText>,
    /// The error, for stderr, when the response has one.
    pub(crate) failure: Option<RenderedText>,
}

/// Renders a worker host's final response (`job run`, `job batch`): its
/// data as a result, also when the request failed or was cancelled, and its
/// error as a failure. Either may be absent (a refusal before the request
/// ran has no data).
///
/// # Errors
///
/// As [`result`] and [`failure`].
pub(crate) fn host(
    command: CommandName,
    response: &OperationResponse<serde_json::Value>,
) -> Result<HostText, OutputError> {
    let value = serde_json::to_value(response).map_err(OutputError::Serialization)?;
    render_host(command, &value)
}

/// [`host`] over the published JSON form of a response.
fn render_host(command: CommandName, value: &serde_json::Value) -> Result<HostText, OutputError> {
    let present = |member: &str| value.get(member).is_some_and(|inner| !inner.is_null());
    let result = if present("data") {
        render_value(command, value)?
    } else {
        None
    };
    let failure = if present("error") {
        Some(failure::render_failure(value, None)?)
    } else {
        None
    };
    Ok(HostText { result, failure })
}

/// Renders a completed result of `command` as human text, or `None` for
/// `setup check` (see [`setup_check`]) and the commands that only fail.
///
/// # Errors
///
/// [`OutputError::Serialization`] when the result does not have its
/// published shape, and [`OutputError::TooLarge`] when its text exceeds the
/// result budget.
pub(crate) fn result(
    command: CommandName,
    response: &OperationResponse<serde_json::Value>,
) -> Result<Option<RenderedText>, OutputError> {
    let value = serde_json::to_value(response).map_err(OutputError::Serialization)?;
    render_value(command, &value)
}

/// [`result`] over the published JSON form of a result.
fn render_value(
    command: CommandName,
    value: &serde_json::Value,
) -> Result<Option<RenderedText>, OutputError> {
    let rendered = match command {
        CommandName::SetupPlan => setup::plan(&envelope(value)?),
        CommandName::SetupInstall => setup::install(&envelope(value)?),
        CommandName::SetupConfigure => setup::configure(&envelope(value)?),
        CommandName::SetupConfigureModel => setup::configure_model(&envelope(value)?),
        CommandName::Ingest => session::ingest(&envelope(value)?),
        CommandName::SessionList => session::list(&envelope(value)?),
        CommandName::SessionStatus | CommandName::SessionRenew | CommandName::SessionClose => {
            session::status(command, &envelope(value)?)
        }
        CommandName::SessionRetain | CommandName::BundleValidate => {
            session::bundle(command, &envelope(value)?)
        }
        CommandName::SessionClean => session::clean(&envelope(value)?),
        CommandName::SessionInitWorkspace => session::workspace(&envelope(value)?),
        CommandName::TranscriptGet => transcript::page(&envelope(value)?),
        CommandName::TranscriptRetranscribe => transcript::retranscription(&envelope(value)?),
        CommandName::Search => transcript::search(&envelope(value)?),
        CommandName::Candidates => evidence::candidates(&envelope(value)?),
        CommandName::FrameGet
        | CommandName::FrameNeighbours
        | CommandName::FrameBurst
        | CommandName::Crop => evidence::frames(&envelope(value)?),
        CommandName::Audio => evidence::audio(&envelope(value)?),
        CommandName::JobStatus => job::status("Job ", &envelope(value)?),
        CommandName::JobCancel => job::status("Cancel requested for job ", &envelope(value)?),
        CommandName::JobResume => job::resume(&envelope(value)?),
        CommandName::JobRun => job::run(&envelope(value)?),
        CommandName::JobBatch => job::batch(&envelope(value)?),
        CommandName::HandoffCheck => handoff::check(&envelope(value)?),
        // `setup check` renders from its typed report (`setup_check`), and
        // the parse and reserved setup commands only ever fail.
        CommandName::SetupCheck
        | CommandName::Parse
        | CommandName::SetupRepair
        | CommandName::SetupList
        | CommandName::SetupRemove
        | CommandName::SetupRollback => return Ok(None),
    };
    rendered.map(Some).map_err(|_| OutputError::TooLarge)
}

/// Writes what every result ends with: a partial status, the operation it
/// is recorded under, its lifecycle and its warnings.
fn push_outcome<D>(text: &mut TerminalText, envelope: &view::Envelope<D>) {
    let has_outcome = envelope.status != view::Status::Complete
        || envelope.operation_id.is_some()
        || envelope.lifecycle.is_some()
        || !envelope.warnings.is_empty();
    if has_outcome {
        text.blank_line();
    }
    match envelope.status {
        view::Status::Complete => {}
        view::Status::Partial => {
            text.push_fixed("Status: partial (the warnings below say what is missing)")
                .end_line();
        }
        view::Status::Failed => {
            text.push_fixed("Status: failed").end_line();
        }
        view::Status::Cancelled => {
            text.push_fixed("Status: cancelled").end_line();
        }
    }
    if let Some(operation_id) = &envelope.operation_id {
        text.push_fixed("Operation: ")
            .push_value(operation_id)
            .end_line();
    }
    if let Some(lifecycle) = &envelope.lifecycle {
        text.push_fixed("Lifecycle: ").push_value(&lifecycle.mode);
        match &lifecycle.expires_at {
            Some(expires_at) => text.push_fixed(", expires ").push_value(expires_at),
            None => text.push_fixed(", never cleaned up automatically"),
        };
        text.end_line();
    }
    for warning in &envelope.warnings {
        text.push_fixed("Warning: ").push_value(warning).end_line();
    }
}

/// Reads a result's published form as the view `D`.
fn envelope<'value, D>(value: &'value serde_json::Value) -> Result<view::Envelope<D>, OutputError>
where
    D: Deserialize<'value>,
{
    view::Envelope::deserialize(value).map_err(OutputError::Serialization)
}

#[cfg(test)]
mod tests;
