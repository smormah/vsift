//! Bounded human, JSON, and JSONL presentation contracts.

use std::{error::Error, fmt, io, io::Write};

use serde::Serialize;
use vsift_application::RuntimeDiagnosis;
use vsift_domain::{
    Confidence, ConfidenceOrigin, DependencyState, DependencyStatus, FailureClass, FailureCode,
    FrameTiming, OperationStatus, RuntimeReadiness,
};

use crate::command::ExecutionProfile;

/// Public major version for R0 CLI payloads.
pub(crate) const CONTRACT_VERSION: &str = "1";
const MAX_RESULT_BYTES: usize = 1_048_576;
const MAX_DIAGNOSTIC_BYTES: usize = 4_096;
const MAX_PROVIDER_DETAIL_BYTES: usize = 240;

/// Selected stdout representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutputMode {
    /// Concise terminal-oriented text.
    Human,
    /// Exactly one versioned JSON result.
    Json,
    /// Versioned progress records followed by one terminal record.
    JsonLines,
}

impl OutputMode {
    /// Resolves mutually exclusive output flags.
    pub(crate) const fn resolve(json: bool, json_lines: bool) -> Result<Self, FailureCode> {
        match (json, json_lines) {
            (false, false) => Ok(Self::Human),
            (true, false) => Ok(Self::Json),
            (false, true) => Ok(Self::JsonLines),
            (true, true) => Err(FailureCode::InvalidArgument),
        }
    }
}

/// Stable operating-system process exit categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessExit {
    Success,
    Internal,
    UsageOrCapability,
    Source,
    Retryable,
    Limit,
    Cancelled,
    StorageOrIo,
}

impl ProcessExit {
    /// Returns the documented numeric process status.
    #[must_use]
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Internal => 1,
            Self::UsageOrCapability => 2,
            Self::Source => 3,
            Self::Retryable => 4,
            Self::Limit => 5,
            Self::Cancelled => 6,
            Self::StorageOrIo => 7,
        }
    }
}

impl From<FailureClass> for ProcessExit {
    fn from(value: FailureClass) -> Self {
        match value {
            FailureClass::Internal => Self::Internal,
            FailureClass::UsageOrCapability => Self::UsageOrCapability,
            FailureClass::Source => Self::Source,
            FailureClass::Retryable => Self::Retryable,
            FailureClass::Limit => Self::Limit,
            FailureClass::Cancelled => Self::Cancelled,
            FailureClass::StorageOrIo => Self::StorageOrIo,
        }
    }
}

/// Stable error shape emitted by JSON and JSONL terminal records.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ErrorResponse {
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
    pub(crate) fn from_code(code: FailureCode) -> Self {
        Self {
            code: code.identifier(),
            message: safe_message(code),
            retryable: code.retryable(),
            retry_after_ms: None,
            affected_ids: Vec::new(),
            remediation: Vec::new(),
        }
    }

    /// Returns the public safe message for human presentation.
    #[must_use]
    pub(crate) const fn message(&self) -> &'static str {
        self.message
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
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct CoverageResponse {
    truncated: bool,
    gaps: Vec<String>,
    reasons: Vec<String>,
}

/// Lifecycle metadata attached to a session-backed operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct LifecycleResponse {
    mode: &'static str,
    expires_at: Option<String>,
}

/// Complete terminal result for new R0 operations.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct OperationResponse<T>
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
    /// Creates a successful terminal response for typed serializable data.
    pub(crate) fn complete<T>(command: &'static str, data: &T) -> Result<Self, serde_json::Error>
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

    /// Creates a terminal error response when no operation was admitted.
    #[must_use]
    pub(crate) fn failure(command: &'static str, code: FailureCode) -> Self {
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
    pub(crate) fn error_message(&self) -> &'static str {
        self.error
            .as_ref()
            .map_or("VSift operation failed.", ErrorResponse::message)
    }
}

/// One terminal JSONL event. Future progress records use the same version and identity fields.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct TerminalEventResponse {
    schema_version: &'static str,
    event: &'static str,
    sequence: u64,
    command: &'static str,
    operation_id: Option<String>,
    result: OperationResponse<serde_json::Value>,
}

impl TerminalEventResponse {
    /// Wraps exactly one operation result as the terminal stream record.
    #[must_use]
    pub(crate) const fn new(result: OperationResponse<serde_json::Value>) -> Self {
        Self {
            schema_version: CONTRACT_VERSION,
            event: "terminal",
            sequence: 0,
            command: result.command,
            operation_id: None,
            result,
        }
    }
}

/// Backward-compatible v1 setup-check response.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct SetupCheckResponse {
    schema_version: &'static str,
    command: &'static str,
    profile: &'static str,
    status: &'static str,
    dependencies: Vec<SetupCheckDependencyResponse>,
}

impl SetupCheckResponse {
    /// Creates the compatible setup response for the explicitly resolved profile.
    #[must_use]
    pub(crate) fn new(diagnosis: &RuntimeDiagnosis, profile: ExecutionProfile) -> Self {
        Self {
            schema_version: CONTRACT_VERSION,
            command: "setup.check",
            profile: profile.identifier(),
            status: diagnosis.readiness.identifier(),
            dependencies: diagnosis
                .dependencies
                .iter()
                .map(SetupCheckDependencyResponse::from)
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SetupCheckDependencyResponse {
    dependency: &'static str,
    capability: &'static str,
    status: &'static str,
    detail: Option<String>,
}

impl From<&DependencyStatus> for SetupCheckDependencyResponse {
    fn from(status: &DependencyStatus) -> Self {
        let detail = match &status.state {
            DependencyState::Available { version } => {
                Some(sanitize_untrusted_text(version, MAX_PROVIDER_DETAIL_BYTES))
            }
            DependencyState::Unhealthy { .. } => Some(String::from("dependency probe failed")),
            DependencyState::Missing | DependencyState::TimedOut => None,
        };
        Self {
            dependency: status.dependency.identifier(),
            capability: status.dependency.capability().identifier(),
            status: status.state.identifier(),
            detail,
        }
    }
}

/// Public confidence representation that leaves unknown values as JSON null.
#[allow(
    dead_code,
    reason = "P01 freezes evidence metadata output before P07 and P09 produce it"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ConfidenceResponse {
    value_basis_points: Option<u16>,
    origin: &'static str,
}

impl From<Confidence> for ConfidenceResponse {
    fn from(value: Confidence) -> Self {
        let origin = match value.origin() {
            ConfidenceOrigin::Unavailable => "unavailable",
            ConfidenceOrigin::ProviderUncalibrated => "provider_uncalibrated",
            ConfidenceOrigin::ProviderCalibrated => "provider_calibrated",
        };
        Self {
            value_basis_points: value.basis_points(),
            origin,
        }
    }
}

/// Public frame timing with separate requested and provider-observed positions.
#[allow(
    dead_code,
    reason = "P01 freezes frame timing output before P09 produces it"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct FrameTimingResponse {
    #[serde(rename = "requested_time_us")]
    requested: u64,
    #[serde(rename = "actual_time_us")]
    actual: u64,
    #[serde(rename = "delta_us")]
    delta: i64,
}

impl From<FrameTiming> for FrameTimingResponse {
    fn from(value: FrameTiming) -> Self {
        Self {
            requested: value.requested().as_micros(),
            actual: value.actual().as_micros(),
            delta: value.delta_micros(),
        }
    }
}

/// Bounded writer that never uses panic-on-broken-pipe print macros.
pub(crate) struct OutputWriter<StandardOutput, StandardError> {
    standard_output: StandardOutput,
    standard_error: StandardError,
}

impl<StandardOutput, StandardError> OutputWriter<StandardOutput, StandardError>
where
    StandardOutput: Write,
    StandardError: Write,
{
    /// Creates a writer over explicit output streams.
    pub(crate) const fn new(
        standard_output: StandardOutput,
        standard_error: StandardError,
    ) -> Self {
        Self {
            standard_output,
            standard_error,
        }
    }

    /// Writes exactly one bounded JSON value followed by one newline.
    pub(crate) fn write_json<T>(&mut self, value: &T) -> Result<(), OutputError>
    where
        T: Serialize,
    {
        let mut buffer = BoundedBuffer::new(MAX_RESULT_BYTES.saturating_sub(1));
        if let Err(error) = serde_json::to_writer(&mut buffer, value) {
            return if buffer.exceeded {
                Err(OutputError::TooLarge)
            } else {
                Err(OutputError::Serialization(error))
            };
        }
        buffer.bytes.push(b'\n');
        self.standard_output
            .write_all(&buffer.bytes)
            .map_err(OutputError::Io)
    }

    /// Writes trusted application text to stdout without panic-on-I/O behavior.
    pub(crate) fn write_trusted_stdout(&mut self, value: &str) -> Result<(), OutputError> {
        if value.len() > MAX_RESULT_BYTES {
            return Err(OutputError::TooLarge);
        }
        self.standard_output
            .write_all(value.as_bytes())
            .map_err(OutputError::Io)
    }

    /// Best-effort bounded diagnostic output; stderr failure never panics.
    pub(crate) fn write_safe_diagnostic(&mut self, value: &str) {
        let mut safe = sanitize_untrusted_text(value, MAX_DIAGNOSTIC_BYTES.saturating_sub(1));
        safe.push('\n');
        let _ignored = self.standard_error.write_all(safe.as_bytes());
    }
}

struct BoundedBuffer {
    bytes: Vec<u8>,
    maximum_bytes: usize,
    exceeded: bool,
}

impl BoundedBuffer {
    fn new(maximum_bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(maximum_bytes.min(4_096)),
            maximum_bytes,
            exceeded: false,
        }
    }
}

impl Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next_length = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("serialized result length overflowed"))?;
        if next_length > self.maximum_bytes {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "serialized result exceeds byte limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A bounded output contract failure.
#[derive(Debug)]
pub(crate) enum OutputError {
    /// JSON serialization failed before output was attempted.
    Serialization(serde_json::Error),
    /// The complete result exceeded the public output budget.
    TooLarge,
    /// The selected output stream could not accept the complete result.
    Io(io::Error),
}

impl fmt::Display for OutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serialization(error) => write!(formatter, "result serialization failed: {error}"),
            Self::TooLarge => formatter.write_str("result exceeds the output byte budget"),
            Self::Io(error) => write!(formatter, "result output failed: {error}"),
        }
    }
}

impl Error for OutputError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Serialization(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::TooLarge => None,
        }
    }
}

/// Converts untrusted provider or parser text into bounded single-terminal-safe text.
#[must_use]
pub(crate) fn sanitize_untrusted_text(value: &str, maximum_bytes: usize) -> String {
    let mut result = String::with_capacity(value.len().min(maximum_bytes));
    for character in value.chars() {
        let replacement = if character.is_control() {
            '\u{fffd}'
        } else {
            character
        };
        if result.len() + replacement.len_utf8() > maximum_bytes {
            break;
        }
        result.push(replacement);
    }
    result
}

/// Returns stable safe prose for a public failure code.
const fn safe_message(code: FailureCode) -> &'static str {
    match code {
        FailureCode::Internal => "VSift encountered an unexpected internal failure.",
        FailureCode::InvalidArgument => "The command line arguments are invalid.",
        FailureCode::UnsupportedSchema => "The request schema version is unsupported.",
        FailureCode::MissingCapability => "A required local capability is unavailable.",
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

/// Converts setup readiness into its compatibility-preserving exit status.
#[must_use]
pub(crate) const fn setup_exit(readiness: RuntimeReadiness) -> ProcessExit {
    match readiness {
        RuntimeReadiness::Ready | RuntimeReadiness::Degraded => ProcessExit::Success,
        RuntimeReadiness::Blocked => ProcessExit::UsageOrCapability,
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use serde_json::Value;
    use vsift_domain::{Confidence, FailureClass, FailureCode, FrameTiming, MediaTime};

    use super::{
        ConfidenceResponse, FrameTimingResponse, OperationResponse, OutputError, OutputMode,
        OutputWriter, ProcessExit, sanitize_untrusted_text,
    };

    struct BrokenWriter;

    impl io::Write for BrokenWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn exit_categories_are_stable() {
        let cases = [
            (FailureClass::Internal, 1),
            (FailureClass::UsageOrCapability, 2),
            (FailureClass::Source, 3),
            (FailureClass::Retryable, 4),
            (FailureClass::Limit, 5),
            (FailureClass::Cancelled, 6),
            (FailureClass::StorageOrIo, 7),
        ];
        for (class, expected) in cases {
            assert_eq!(ProcessExit::from(class).code(), expected);
        }
    }

    #[test]
    fn output_modes_reject_ambiguous_selection() {
        assert_eq!(OutputMode::resolve(false, false), Ok(OutputMode::Human));
        assert_eq!(OutputMode::resolve(true, false), Ok(OutputMode::Json));
        assert_eq!(OutputMode::resolve(false, true), Ok(OutputMode::JsonLines));
        assert_eq!(
            OutputMode::resolve(true, true),
            Err(FailureCode::InvalidArgument)
        );
    }

    #[test]
    fn terminal_controls_are_replaced_before_human_display() {
        let safe = sanitize_untrusted_text("ok\u{1b}]8;;file:///secret\u{7}bad\n", 128);

        assert!(!safe.contains('\u{1b}'));
        assert!(!safe.contains('\u{7}'));
        assert!(!safe.contains('\n'));
        assert!(safe.contains('\u{fffd}'));
    }

    #[test]
    fn diagnostic_size_is_bounded_before_writing() {
        let value = "a".repeat(10_000);
        let safe = sanitize_untrusted_text(&value, 4_095);

        assert_eq!(safe.len(), 4_095);
    }

    #[test]
    fn broken_stdout_returns_typed_io_error() {
        let mut writer = OutputWriter::new(BrokenWriter, Vec::<u8>::new());
        let response = OperationResponse::failure("parse", FailureCode::InvalidArgument);

        let result = writer.write_json(&response);

        assert!(
            matches!(result, Err(OutputError::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe)
        );
    }

    #[test]
    fn oversized_json_stops_at_the_serialization_budget() {
        let mut stdout = Vec::new();
        let mut writer = OutputWriter::new(&mut stdout, Vec::<u8>::new());
        let oversized = "a".repeat(1_048_576);

        let result = writer.write_json(&oversized);

        assert!(matches!(result, Err(OutputError::TooLarge)));
        assert!(stdout.is_empty());
    }

    #[test]
    fn closed_stderr_is_ignored_without_panicking() {
        let mut writer = OutputWriter::new(Vec::<u8>::new(), BrokenWriter);

        writer.write_safe_diagnostic("safe diagnostic");
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
    fn unknown_confidence_and_frame_delta_remain_explicit() -> Result<(), Box<dyn std::error::Error>>
    {
        let confidence = serde_json::to_value(ConfidenceResponse::from(Confidence::unknown()))?;
        let timing =
            FrameTiming::new(MediaTime::from_micros(2_000), MediaTime::from_micros(1_750))?;
        let timing = serde_json::to_value(FrameTimingResponse::from(timing))?;

        assert_eq!(confidence["value_basis_points"], Value::Null);
        assert_eq!(confidence["origin"], "unavailable");
        assert_eq!(timing["requested_time_us"], 2_000);
        assert_eq!(timing["actual_time_us"], 1_750);
        assert_eq!(timing["delta_us"], -250);
        Ok(())
    }
}
