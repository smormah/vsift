//! The v1 operation envelope and its JSON Lines terminal record.

use serde::Serialize;
use vsift_domain::{FailureCode, OperationStatus};

use crate::EventKind;

/// Public major version carried by every v1 payload.
///
/// Consumers reject unknown majors instead of guessing compatibility, so this
/// value changes only with a new, separately published schema set.
pub const CONTRACT_VERSION: &str = "1";

/// Stable error shape carried by failed JSON and JSONL terminal records.
///
/// The message is fixed prose chosen by failure code. It never interpolates
/// arguments, paths or provider output, so an error cannot leak untrusted or
/// sensitive text into agent-visible results.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ErrorResponse {
    code: &'static str,
    message: &'static str,
    retryable: bool,
    retry_after_ms: Option<u64>,
    affected_ids: Vec<String>,
    remediation: Vec<RemediationResponse>,
}

impl ErrorResponse {
    /// Creates a safe failure without interpolating untrusted evidence or arguments.
    #[must_use]
    pub fn from_code(code: FailureCode) -> Self {
        Self {
            code: code.identifier(),
            message: safe_message(code),
            retryable: code.retryable(),
            retry_after_ms: None,
            affected_ids: Vec::new(),
            remediation: Vec::new(),
        }
    }

    /// Returns the public safe message, which hosts reuse for human presentation.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }

    /// Returns the remediation summaries, which hosts reuse for human presentation.
    #[must_use]
    pub fn remediation_summaries(&self) -> Vec<&str> {
        self.remediation
            .iter()
            .map(|item| item.summary.as_str())
            .collect()
    }
}

/// Structured, non-executing remediation description.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct RemediationResponse {
    summary: String,
    required_authority: &'static str,
    command: Option<CommandResponse>,
}

/// Executable and argument array; consumers must still request required authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct CommandResponse {
    executable: String,
    arguments: Vec<String>,
}

/// Honest statement of inspected and missing result coverage.
///
/// The shape was frozen before any operation filled it. `search` (P08) is the
/// first: `truncated` says part of the searched range has no transcript,
/// `gaps` lists those parts as `<from_us>-<to_us>` and `reasons` names why,
/// with distinct identifiers. Only this crate builds it, so every host reports
/// coverage by the same rules.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CoverageResponse {
    truncated: bool,
    gaps: Vec<String>,
    reasons: Vec<String>,
}

impl CoverageResponse {
    /// Coverage with its three members, as the operation's rules decide them.
    pub(crate) const fn new(truncated: bool, gaps: Vec<String>, reasons: Vec<String>) -> Self {
        Self {
            truncated,
            gaps,
            reasons,
        }
    }

    /// Whether part of the requested result could not be covered.
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}

/// Lifecycle metadata attached to a session-backed operation.
///
/// It tells a consumer whether the evidence it just received is disposable and
/// when it disappears, so an agent never assumes persistence it was not given.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LifecycleResponse {
    mode: &'static str,
    expires_at: Option<String>,
}

impl LifecycleResponse {
    /// Disposable session with an explicit RFC 3339 expiry.
    ///
    /// The host formats the timestamp because clock access and calendar
    /// formatting are host concerns this crate deliberately does not depend on.
    #[must_use]
    pub const fn ephemeral(expires_at: String) -> Self {
        Self {
            mode: "ephemeral",
            expires_at: Some(expires_at),
        }
    }

    /// Explicitly retained bundle outside automatic session cleanup.
    #[must_use]
    pub const fn retained() -> Self {
        Self {
            mode: "retained",
            expires_at: None,
        }
    }
}

/// Complete terminal result for R0 operations.
///
/// Every field is always present, with `null` for absent values, so consumers
/// can rely on one shape regardless of outcome.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OperationResponse<T>
where
    T: Serialize,
{
    schema_version: &'static str,
    command: &'static str,
    operation_id: Option<String>,
    status: &'static str,
    data: Option<T>,
    warnings: Vec<String>,
    error: Option<ErrorResponse>,
    coverage: Option<CoverageResponse>,
    lifecycle: Option<LifecycleResponse>,
}

impl OperationResponse<serde_json::Value> {
    /// Attaches a truthful lifecycle capability to a completed result.
    #[must_use]
    pub fn with_lifecycle(mut self, lifecycle: LifecycleResponse) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    /// Creates a successful terminal response for typed serializable data.
    ///
    /// # Errors
    ///
    /// Returns the serialization error when `data` cannot be represented as JSON.
    pub fn complete<T>(command: &'static str, data: &T) -> Result<Self, serde_json::Error>
    where
        T: Serialize,
    {
        Ok(Self {
            schema_version: CONTRACT_VERSION,
            command,
            operation_id: None,
            status: OperationStatus::Complete.identifier(),
            data: Some(serde_json::to_value(data)?),
            warnings: Vec::new(),
            error: None,
            coverage: None,
            lifecycle: None,
        })
    }

    /// Creates an honest page with useful items and one or more item failures.
    ///
    /// # Errors
    ///
    /// Returns the serialization error when `data` cannot be represented as JSON.
    pub fn partial<T>(
        command: &'static str,
        data: &T,
        warning: &'static str,
    ) -> Result<Self, serde_json::Error>
    where
        T: Serialize,
    {
        let mut response = Self::complete(command, data)?;
        response.status = OperationStatus::Partial.identifier();
        response.warnings.push(warning.to_owned());
        Ok(response)
    }

    /// Attaches coverage to a completed result. Truncated coverage makes the
    /// result `partial`: useful data with a stated gap, still a success.
    #[must_use]
    pub(crate) fn with_coverage(mut self, coverage: CoverageResponse) -> Self {
        if coverage.truncated && self.error.is_none() {
            self.status = OperationStatus::Partial.identifier();
        }
        self.coverage = Some(coverage);
        self
    }

    /// Adds fixed-prose warnings to a completed result without changing its status.
    ///
    /// Used when useful work completed but something was excluded or changed;
    /// the typed detail lives in `data`.
    #[must_use]
    pub fn with_warnings(mut self, warnings: &[&'static str]) -> Self {
        self.warnings
            .extend(warnings.iter().map(|warning| (*warning).to_owned()));
        self
    }

    /// Creates a failure that also tells the caller how to fix its input.
    ///
    /// `summary` must be fixed prose chosen by a typed cause (numbers such as
    /// a line are permitted); it never carries untrusted text. No command is
    /// suggested and no authority is required.
    #[must_use]
    pub fn failure_with_remediation(
        command: &'static str,
        code: FailureCode,
        summary: String,
    ) -> Self {
        let mut response = Self::failure(command, code);
        if let Some(error) = response.error.as_mut() {
            error.remediation.push(RemediationResponse {
                summary,
                required_authority: "none",
                command: None,
            });
        }
        response
    }

    /// Returns the remediation summaries of a failure response.
    #[must_use]
    pub fn remediation_summaries(&self) -> Vec<&str> {
        self.error
            .as_ref()
            .map(ErrorResponse::remediation_summaries)
            .unwrap_or_default()
    }

    /// Creates a terminal error response when no operation was admitted.
    #[must_use]
    pub fn failure(command: &'static str, code: FailureCode) -> Self {
        let status = if code == FailureCode::Cancelled {
            OperationStatus::Cancelled
        } else {
            OperationStatus::Failed
        };
        Self {
            schema_version: CONTRACT_VERSION,
            command,
            operation_id: None,
            status: status.identifier(),
            data: None,
            warnings: Vec::new(),
            error: Some(ErrorResponse::from_code(code)),
            coverage: None,
            lifecycle: None,
        }
    }

    /// Returns the safe human message from a failure response.
    #[must_use]
    pub fn error_message(&self) -> &'static str {
        self.error
            .as_ref()
            .map_or("VSift operation failed.", ErrorResponse::message)
    }
}

/// One terminal JSON Lines event.
///
/// Evidence events ([`crate::EvidenceEventResponse`]) and future progress
/// records use the same version and identity fields, so a stream reader can
/// dispatch on `event` without knowing the command in advance. The terminal
/// event is always the last line of a stream.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TerminalEventResponse {
    schema_version: &'static str,
    event: &'static str,
    sequence: u64,
    command: &'static str,
    operation_id: Option<String>,
    result: OperationResponse<serde_json::Value>,
}

impl TerminalEventResponse {
    /// Wraps exactly one operation result as the only record of a stream.
    #[must_use]
    pub const fn new(result: OperationResponse<serde_json::Value>) -> Self {
        Self::at_sequence(result, 0)
    }

    /// Wraps the result that ends a stream after `sequence` earlier events.
    pub(crate) const fn at_sequence(
        result: OperationResponse<serde_json::Value>,
        sequence: u64,
    ) -> Self {
        Self {
            schema_version: CONTRACT_VERSION,
            event: EventKind::Terminal.identifier(),
            sequence,
            command: result.command,
            operation_id: None,
            result,
        }
    }
}

/// Returns stable safe prose for a public failure code.
const fn safe_message(code: FailureCode) -> &'static str {
    match code {
        FailureCode::Internal => "VSift encountered an unexpected internal failure.",
        FailureCode::InvalidArgument => "The command line arguments are invalid.",
        FailureCode::UnsupportedSchema => "The request schema version is unsupported.",
        FailureCode::MissingCapability => "A required local capability is unavailable.",
        FailureCode::IsolationUnavailable => {
            "The host cannot provide the required process isolation."
        }
        FailureCode::CommandNotImplemented => {
            "This reserved R0 command is not implemented in the current build."
        }
        FailureCode::InvalidSource => "The source is invalid or unsupported.",
        FailureCode::Busy => "The requested operation is temporarily busy.",
        FailureCode::DeadlineExceeded => "The operation exceeded its deadline.",
        FailureCode::ResourceLimit => "The operation exceeded a configured resource limit.",
        FailureCode::Cancelled => "The operation was cancelled.",
        FailureCode::StorageIo => "Storage or output I/O prevented completion.",
        FailureCode::IntegrityFailure => "Stored or imported data failed integrity validation.",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use vsift_domain::FailureCode;

    use super::{LifecycleResponse, OperationResponse, TerminalEventResponse};

    #[test]
    fn failure_json_has_complete_semantic_fields() -> Result<(), Box<dyn std::error::Error>> {
        let response = OperationResponse::failure("parse", FailureCode::InvalidArgument);
        let value = serde_json::to_value(response)?;

        assert_eq!(value["schema_version"], "1");
        assert_eq!(value["command"], "parse");
        assert_eq!(value["status"], "failed");
        assert_eq!(value["data"], Value::Null);
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
        assert_eq!(value["error"]["retryable"], false);
        assert!(value["warnings"].is_array());
        Ok(())
    }

    #[test]
    fn complete_and_cancelled_terminal_states_are_unambiguous()
    -> Result<(), Box<dyn std::error::Error>> {
        let complete =
            OperationResponse::complete("setup.check", &serde_json::json!({"status": "ready"}))?;
        let complete = serde_json::to_value(complete)?;
        let cancelled = serde_json::to_value(OperationResponse::failure(
            "job.cancel",
            FailureCode::Cancelled,
        ))?;

        assert_eq!(complete["status"], "complete");
        assert_eq!(complete["error"], Value::Null);
        assert_eq!(cancelled["status"], "cancelled");
        assert_eq!(cancelled["error"]["code"], "CANCELLED");
        Ok(())
    }

    #[test]
    fn partial_results_keep_data_and_explain_the_gap() -> Result<(), Box<dyn std::error::Error>> {
        let response = OperationResponse::partial(
            "session.list",
            &serde_json::json!({"items": []}),
            "Some session records could not be read.",
        )?;
        let value = serde_json::to_value(response)?;

        assert_eq!(value["status"], "partial");
        assert_eq!(value["data"]["items"], serde_json::json!([]));
        assert_eq!(
            value["warnings"],
            serde_json::json!(["Some session records could not be read."])
        );
        assert_eq!(value["error"], Value::Null);
        Ok(())
    }

    #[test]
    fn lifecycle_modes_are_explicit() -> Result<(), Box<dyn std::error::Error>> {
        let data = serde_json::json!({});
        let ephemeral = OperationResponse::complete("ingest", &data)?.with_lifecycle(
            LifecycleResponse::ephemeral(String::from("2026-09-24T00:00:00Z")),
        );
        let retained = OperationResponse::complete("session.retain", &data)?
            .with_lifecycle(LifecycleResponse::retained());

        assert_eq!(
            serde_json::to_value(ephemeral)?["lifecycle"],
            serde_json::json!({"mode": "ephemeral", "expires_at": "2026-09-24T00:00:00Z"})
        );
        assert_eq!(
            serde_json::to_value(retained)?["lifecycle"],
            serde_json::json!({"mode": "retained", "expires_at": null})
        );
        Ok(())
    }

    #[test]
    fn terminal_event_repeats_the_command_of_its_result() -> Result<(), Box<dyn std::error::Error>>
    {
        let event = TerminalEventResponse::new(OperationResponse::failure(
            "setup.install",
            FailureCode::CommandNotImplemented,
        ));
        let value = serde_json::to_value(event)?;

        assert_eq!(value["event"], "terminal");
        assert_eq!(value["sequence"], 0);
        assert_eq!(value["command"], "setup.install");
        assert_eq!(value["result"]["command"], "setup.install");
        Ok(())
    }

    #[test]
    fn failure_message_falls_back_only_without_an_error() -> Result<(), Box<dyn std::error::Error>>
    {
        let failure = OperationResponse::failure("setup.install", FailureCode::Busy);
        let complete = OperationResponse::complete("setup.check", &serde_json::json!({}))?;

        assert_eq!(
            failure.error_message(),
            "The requested operation is temporarily busy."
        );
        assert_eq!(complete.error_message(), "VSift operation failed.");
        Ok(())
    }
}
