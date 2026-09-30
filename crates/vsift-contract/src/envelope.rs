//! The v1 operation envelope and its JSON Lines terminal record.

use serde::Serialize;
use vsift_domain::{
    FailureCode, OperationId, OperationStatus, SessionLifetime, SessionLifetimePolicy,
};

use crate::EventKind;

/// Most identifiers a failure names in `affected_ids`.
pub const MAX_AFFECTED_IDS: usize = 100;
/// Longest retry hint: one day, the published schema's bound.
const MAX_RETRY_AFTER_MS: u64 = 86_400_000;

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

    /// A session of an explicitly initialised worker workspace (P11): it
    /// lives as long as the workspace policy says, with an RFC 3339 expiry.
    #[must_use]
    pub const fn durable_worker(expires_at: String) -> Self {
        Self {
            mode: "durable_worker",
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

    /// The lifecycle of a session with `lifetime`: `ephemeral` for a
    /// desktop session, `durable_worker` for a session of a worker
    /// workspace, with its expiry in RFC 3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`).
    ///
    /// A worker host records its results, so the contract formats the
    /// expiry itself rather than each host (P11 PR 3). `None` for an expiry
    /// after the year 9999, which no clock that passes the session rules
    /// reaches.
    #[must_use]
    pub fn of_session(lifetime: SessionLifetime) -> Option<Self> {
        let expires_at = utc_seconds_rfc3339(lifetime.expires_at_unix_seconds())?;
        Some(match lifetime.policy() {
            SessionLifetimePolicy::Desktop => Self::ephemeral(expires_at),
            SessionLifetimePolicy::Workspace(_) => Self::durable_worker(expires_at),
        })
    }
}

/// Formats Unix seconds as RFC 3339 UTC with a `Z` offset, the form the
/// command line's time formatter writes; `None` after 9999-12-31.
fn utc_seconds_rfc3339(seconds: u64) -> Option<String> {
    const SECONDS_PER_DAY: u64 = 86_400;
    let days = seconds / SECONDS_PER_DAY;
    let of_day = seconds % SECONDS_PER_DAY;
    // Howard Hinnant's civil-from-days, for days since 1970-01-01.
    let shifted = days.checked_add(719_468)?;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    if year > 9_999 {
        return None;
    }
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3_600,
        of_day % 3_600 / 60,
        of_day % 60
    ))
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

    /// Creates a failure whose remediation also suggests one `vsift`
    /// command, as the executable `vsift` and an argument array a caller
    /// runs without a shell (for example `job resume <job>`).
    ///
    /// `summary` follows [`OperationResponse::failure_with_remediation`];
    /// `arguments` must be fixed words and validated identifiers only, never
    /// a path or untrusted text. The command changes nothing the caller did
    /// not already ask for, so it needs no authority.
    #[must_use]
    pub fn failure_with_suggested_command(
        command: &'static str,
        code: FailureCode,
        summary: String,
        arguments: &[&str],
    ) -> Self {
        let mut response = Self::failure(command, code);
        if let Some(error) = response.error.as_mut() {
            error.remediation.push(RemediationResponse {
                summary,
                required_authority: "none",
                command: Some(CommandResponse {
                    executable: "vsift".to_owned(),
                    arguments: arguments
                        .iter()
                        .map(|argument| (*argument).to_owned())
                        .collect(),
                }),
            });
        }
        response
    }

    /// Attaches `data` to a failed or cancelled result (P11 `job run`: the
    /// job result of a request that ended with a failure), so a supervisor
    /// sees what each step did and which one ended the request. The error
    /// stays the result's authority; the data only reports.
    ///
    /// # Errors
    ///
    /// Returns the serialization error when `data` cannot be represented as JSON.
    pub fn with_failure_data<T>(mut self, data: &T) -> Result<Self, serde_json::Error>
    where
        T: Serialize,
    {
        self.data = Some(serde_json::to_value(data)?);
        Ok(self)
    }

    /// [`Self::with_failure_data`] for data already in its published JSON
    /// form, which cannot fail to serialize (`setup install`: every
    /// component of the plan beside the error that stopped it).
    #[must_use]
    pub fn with_failure_value(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }

    /// Names the operation the result is recorded under, so a caller can
    /// retry with it and receive the same result (P10, ADR 0020).
    #[must_use]
    pub fn with_operation_id(mut self, operation_id: &OperationId) -> Self {
        self.operation_id = Some(operation_id.as_str().to_owned());
        self
    }

    /// Tells the caller of a retryable failure how long to wait before
    /// retrying; ignored on a success or a non-retryable failure.
    #[must_use]
    pub fn with_retry_after(mut self, retry_after_ms: u64) -> Self {
        if let Some(error) = self.error.as_mut().filter(|error| error.retryable) {
            error.retry_after_ms = Some(retry_after_ms.clamp(1, MAX_RETRY_AFTER_MS));
        }
        self
    }

    /// Names the identifiers a failure concerns, such as the job that is
    /// busy; ignored on a success. At most [`MAX_AFFECTED_IDS`] are kept.
    #[must_use]
    pub fn with_affected_ids(mut self, ids: &[&str]) -> Self {
        if let Some(error) = self.error.as_mut() {
            error.affected_ids = ids
                .iter()
                .take(MAX_AFFECTED_IDS)
                .map(|id| (*id).to_owned())
                .collect();
        }
        self
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
    pub fn new(result: OperationResponse<serde_json::Value>) -> Self {
        Self::at_sequence(result, 0)
    }

    /// Wraps the result that ends a stream after `sequence` earlier events;
    /// the event repeats the result's operation id.
    #[must_use]
    pub fn at_sequence(result: OperationResponse<serde_json::Value>, sequence: u64) -> Self {
        Self {
            schema_version: CONTRACT_VERSION,
            event: EventKind::Terminal.identifier(),
            sequence,
            command: result.command,
            operation_id: result.operation_id.clone(),
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
        FailureCode::IdempotencyConflict => {
            "The operation id was already used for a different request."
        }
        FailureCode::DownloadFailed => {
            "A managed download from the reviewed publisher did not complete."
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use vsift_domain::FailureCode;

    use super::{LifecycleResponse, OperationResponse, TerminalEventResponse, utc_seconds_rfc3339};

    /// The contract's own expiry formatter writes what the command line's
    /// RFC 3339 formatter writes, leap days and centuries included.
    #[test]
    fn utc_expiries_are_rfc3339() {
        for (seconds, expected) in [
            (0, Some("1970-01-01T00:00:00Z")),
            (951_782_400, Some("2000-02-29T00:00:00Z")),
            (1_791_201_600, Some("2026-10-05T12:00:00Z")),
            (4_107_628_799, Some("2100-03-01T23:59:59Z")),
            (253_402_300_799, Some("9999-12-31T23:59:59Z")),
            (253_402_300_800, None),
            (u64::MAX, None),
        ] {
            assert_eq!(
                utc_seconds_rfc3339(seconds).as_deref(),
                expected,
                "{seconds}"
            );
        }
    }

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

    /// D-4 (ADR 0020): the idempotency conflict is published with fixed
    /// prose and is not retryable.
    #[test]
    fn an_idempotency_conflict_is_a_fixed_prose_non_retryable_failure()
    -> Result<(), Box<dyn std::error::Error>> {
        let value = serde_json::to_value(OperationResponse::failure(
            "transcript.retranscribe",
            FailureCode::IdempotencyConflict,
        ))?;
        assert_eq!(value["status"], "failed");
        assert_eq!(value["error"]["code"], "IDEMPOTENCY_CONFLICT");
        assert_eq!(value["error"]["retryable"], false);
        assert_eq!(
            value["error"]["message"],
            "The operation id was already used for a different request."
        );
        Ok(())
    }

    /// P10: a busy job names itself and a retry hint; a non-retryable
    /// failure never carries a hint; the terminal event repeats the
    /// operation id of its result.
    #[test]
    fn retry_hints_affected_ids_and_operation_ids_are_carried()
    -> Result<(), Box<dyn std::error::Error>> {
        let busy = OperationResponse::failure("transcript.retranscribe", FailureCode::Busy)
            .with_retry_after(2_000)
            .with_affected_ids(&["job_0123456789abcdef"]);
        let value = serde_json::to_value(&busy)?;
        assert_eq!(value["error"]["retry_after_ms"], 2_000);
        assert_eq!(
            value["error"]["affected_ids"],
            serde_json::json!(["job_0123456789abcdef"])
        );
        let conflict =
            OperationResponse::failure("transcript.retranscribe", FailureCode::IdempotencyConflict)
                .with_retry_after(2_000);
        assert_eq!(
            serde_json::to_value(conflict)?["error"]["retry_after_ms"],
            Value::Null
        );
        let operation = vsift_domain::OperationId::parse("op_0123456789abcdef")?;
        let complete =
            OperationResponse::complete("transcript.retranscribe", &serde_json::json!({}))?
                .with_operation_id(&operation);
        let event = serde_json::to_value(TerminalEventResponse::new(complete))?;
        assert_eq!(event["operation_id"], "op_0123456789abcdef");
        assert_eq!(event["result"]["operation_id"], "op_0123456789abcdef");
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
