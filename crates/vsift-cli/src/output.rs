//! Bounded human, JSON, and JSONL presentation.
//!
//! The v1 wire types live in `vsift-contract`; this module owns only what is
//! specific to a command-line host: output mode selection, process exit codes,
//! the output byte budget and panic-free stream writing.

use std::{error::Error, fmt, io, io::Write};

use serde::Serialize;
use vsift::{FailureClass, FailureCode, RuntimeReadiness};
use vsift_contract::sanitize_untrusted_text;

const MAX_RESULT_BYTES: usize = 1_048_576;
const MAX_DIAGNOSTIC_BYTES: usize = 4_096;

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

    use vsift::{FailureClass, FailureCode};
    use vsift_contract::OperationResponse;

    use super::{OutputError, OutputMode, OutputWriter, ProcessExit};

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
}
