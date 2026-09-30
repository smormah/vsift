//! Every failure in human mode, on stderr.
//!
//! A failure is written as its fixed message and code, then each
//! remediation's summary and suggested command, then the identifiers it
//! concerns and any retry hint: the same facts as the `--json` error, in
//! the same order, and nothing the user or the evidence supplied. The one
//! exception is a rejected command line, whose parser explanation quotes the
//! argument: it is labelled and quoted in its display form.

use serde::Deserialize;
use vsift_contract::OperationResponse;

use super::{
    text::{DisplayText, Placement, RenderedText, TerminalText},
    view::FailureEnvelope,
};
use crate::output::OutputError;

/// What a human failure adds to the published error, when there is more.
#[derive(Clone, Copy, Debug)]
pub(crate) enum HumanDetail<'detail> {
    /// The parser's own explanation of a rejected command line, which may
    /// quote an argument: quoted in its display form, labelled untrusted.
    Parser(&'detail DisplayText),
    /// A fixed-prose reason of `VSift`'s own, such as a refused
    /// configuration.
    Reason(&'static str),
}

/// Renders a failed result for stderr, with its human detail when it has
/// one.
///
/// # Errors
///
/// [`OutputError::Serialization`] when the result has no published error,
/// and [`OutputError::TooLarge`] when the text exceeds the failure budget.
pub(crate) fn failure(
    response: &OperationResponse<serde_json::Value>,
    detail: Option<HumanDetail<'_>>,
) -> Result<RenderedText, OutputError> {
    let value = serde_json::to_value(response).map_err(OutputError::Serialization)?;
    render_failure(&value, detail)
}

/// [`failure`] over the published JSON form of a failed result.
pub(super) fn render_failure(
    value: &serde_json::Value,
    detail: Option<HumanDetail<'_>>,
) -> Result<RenderedText, OutputError> {
    let envelope = FailureEnvelope::deserialize(value).map_err(OutputError::Serialization)?;
    let error = &envelope.error;
    let mut text = TerminalText::failure();
    text.push_fixed("Error: ")
        .push_value(&error.message)
        .push_fixed(" (")
        .push_value(&error.code)
        .push_fixed(")")
        .end_line();
    match detail {
        Some(HumanDetail::Parser(explanation)) => {
            text.push_fixed(
                "The parser explains (this quotes the command line, which is untrusted text):",
            )
            .push_untrusted(explanation, Placement::Quoted);
        }
        Some(HumanDetail::Reason(reason)) => {
            text.push_fixed("Reason: ").push_fixed(reason).end_line();
        }
        None => {}
    }
    for remediation in &error.remediation {
        text.push_fixed("Fix: ")
            .push_value(&remediation.summary)
            .end_line();
        if let Some(command) = &remediation.command {
            text.push_fixed("Run: ").push_value(&command.executable);
            for argument in &command.arguments {
                text.push_fixed(" ").push_value(argument);
            }
            text.end_line();
        }
    }
    if !error.affected_ids.is_empty() {
        text.push_fixed("Affected: ")
            .push_value(&error.affected_ids.join(", "))
            .end_line();
    }
    if let Some(retry_after_ms) = error.retry_after_ms {
        text.push_fixed("Retry after: ")
            .push_unsigned(retry_after_ms)
            .push_fixed(" ms")
            .end_line();
    }
    text.finish().map_err(|_| OutputError::TooLarge)
}
