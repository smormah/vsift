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
//! - a file path, when a later renderer shows one, goes on its own line.
//!
//! Human text is unstable and not for parsing (`docs/contracts/cli-v1.md`);
//! agents use `--json`.
//!
//! **Split of the work.** This part (PR 2a) renders setup, session, ingest,
//! transcript, search and bundle results, every failure, and rejected
//! command lines. `candidates`, the frame commands, `crop`, `audio`, the
//! `job` commands and the worker hosts still print the indented JSON result
//! until PR 2b; [`result`] answers `None` for them.

mod failure;
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

/// Renders a completed result of `command` as human text, or `None` when
/// the command still prints its indented JSON result (until P13 PR 2b).
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
        // `setup check` renders from its typed report (`setup_check`), and
        // the parse and reserved setup commands only ever fail.
        CommandName::SetupCheck
        | CommandName::Parse
        | CommandName::SetupInstall
        | CommandName::SetupRepair
        | CommandName::SetupList
        | CommandName::SetupRemove
        | CommandName::SetupRollback
        // P13 PR 2b.
        | CommandName::Candidates
        | CommandName::FrameGet
        | CommandName::FrameNeighbours
        | CommandName::FrameBurst
        | CommandName::Audio
        | CommandName::Crop
        | CommandName::JobRun
        | CommandName::JobBatch
        | CommandName::JobStatus
        | CommandName::JobResume
        | CommandName::JobCancel => return Ok(None),
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
