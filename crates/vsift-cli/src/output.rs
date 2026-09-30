//! Bounded human, JSON, and JSONL presentation.
//!
//! The v1 wire types live in `vsift-contract`; this module owns only what is
//! specific to a command-line host: output mode selection, process exit codes,
//! the output byte budget and panic-free stream writing.

use std::{error::Error, fmt, io, io::Write};

use serde::Serialize;
use vsift::{FailureClass, FailureCode, RuntimeReadiness};
use vsift_contract::{
    MAX_EVENT_LINE_BYTES, OperationResponse, TerminalEventResponse, terminal_safe_text,
};

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
        let line = encode_line(value)?;
        self.standard_output
            .write_all(&line)
            .map_err(OutputError::Io)
    }

    /// Writes a complete, already bounded JSON Lines stream.
    ///
    /// Every line was serialized before this call, so a line over the budget
    /// fails the command before any byte of the stream reaches stdout.
    pub(crate) fn write_json_lines(&mut self, lines: &JsonLines) -> Result<(), OutputError> {
        self.standard_output
            .write_all(&lines.bytes)
            .map_err(OutputError::Io)
    }

    /// Writes one encoded line and flushes it, so a reader sees it now.
    fn write_flushed(&mut self, line: &[u8]) -> Result<(), OutputError> {
        self.standard_output
            .write_all(line)
            .and_then(|()| self.standard_output.flush())
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
    ///
    /// Diagnostics can repeat text the user or a provider supplied (the
    /// parser's explanation of a rejected command line quotes the argument),
    /// so control characters are replaced and hidden characters shown as
    /// `<U+XXXX>` ([`terminal_safe_text`]).
    pub(crate) fn write_safe_diagnostic(&mut self, value: &str) {
        let mut safe = terminal_safe_text(value, MAX_DIAGNOSTIC_BYTES.saturating_sub(1));
        safe.push('\n');
        let _ignored = self.standard_error.write_all(safe.as_bytes());
    }
}

/// A JSON Lines stream assembled in memory before it is written.
///
/// Each line obeys the same per-result byte budget as a `--json` result. The
/// number of lines is bounded by the producer (a page holds at most its limit
/// of evidence events plus one terminal event), so the whole stream is bounded
/// too. Assembling first means a stream is either written complete or not at
/// all when a line is over budget.
pub(crate) struct JsonLines {
    bytes: Vec<u8>,
}

impl JsonLines {
    /// Starts an empty stream.
    pub(crate) const fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    /// Appends one bounded JSON value and its newline.
    pub(crate) fn push<T>(&mut self, value: &T) -> Result<(), OutputError>
    where
        T: Serialize,
    {
        let line = encode_line(value)?;
        self.bytes.extend_from_slice(&line);
        Ok(())
    }
}

/// Writes a JSON Lines stream one event at a time, as each is ready (P11).
///
/// Evidence streams are bounded pages and are assembled first
/// ([`JsonLines`]); a long command's progress and a worker host's lifecycle
/// and result events must reach the reader while the work runs, so this
/// writer numbers each event with the next `sequence`, bounds every line
/// but the terminal one to [`MAX_EVENT_LINE_BYTES`], writes and flushes it
/// at once, and ends the stream with the terminal event at the count of
/// events before it. Blocking on a slow reader is the caller's to avoid:
/// progress is dropped before it reaches this writer, never other events.
pub(crate) struct JsonLinesWriter<'writer, StandardOutput, StandardError> {
    output: &'writer mut OutputWriter<StandardOutput, StandardError>,
    next_sequence: u64,
}

impl<'writer, StandardOutput, StandardError> JsonLinesWriter<'writer, StandardOutput, StandardError>
where
    StandardOutput: Write,
    StandardError: Write,
{
    /// Starts a stream at sequence 0.
    pub(crate) const fn new(
        output: &'writer mut OutputWriter<StandardOutput, StandardError>,
    ) -> Self {
        Self {
            output,
            next_sequence: 0,
        }
    }

    /// Writes one non-terminal event built for the next sequence number.
    /// A line over the budget writes nothing and does not use the number.
    pub(crate) fn write_event<T, Build>(&mut self, build: Build) -> Result<(), OutputError>
    where
        T: Serialize,
        Build: FnOnce(u64) -> T,
    {
        let line = encode_line_within(&build(self.next_sequence), MAX_EVENT_LINE_BYTES)?;
        self.output.write_flushed(&line)?;
        self.next_sequence = self.next_sequence.saturating_add(1);
        Ok(())
    }

    /// Ends the stream with `result` as its terminal event.
    pub(crate) fn write_terminal(
        self,
        result: OperationResponse<serde_json::Value>,
    ) -> Result<(), OutputError> {
        let line = encode_line(&TerminalEventResponse::at_sequence(
            result,
            self.next_sequence,
        ))?;
        self.output.write_flushed(&line)
    }

    /// The writer's diagnostics channel.
    pub(crate) fn output(&mut self) -> &mut OutputWriter<StandardOutput, StandardError> {
        self.output
    }
}

/// Serializes one value as a newline-terminated line within the result budget.
fn encode_line<T>(value: &T) -> Result<Vec<u8>, OutputError>
where
    T: Serialize,
{
    encode_line_within(value, MAX_RESULT_BYTES)
}

/// Serializes one value as a newline-terminated line of at most
/// `maximum_bytes`, newline included.
fn encode_line_within<T>(value: &T, maximum_bytes: usize) -> Result<Vec<u8>, OutputError>
where
    T: Serialize,
{
    let mut buffer = BoundedBuffer::new(maximum_bytes.saturating_sub(1));
    if let Err(error) = serde_json::to_writer(&mut buffer, value) {
        return if buffer.exceeded {
            Err(OutputError::TooLarge)
        } else {
            Err(OutputError::Serialization(error))
        };
    }
    buffer.bytes.push(b'\n');
    Ok(buffer.bytes)
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

    use super::{JsonLines, JsonLinesWriter, OutputError, OutputMode, OutputWriter, ProcessExit};

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

    /// D-4 (ADR 0020): reusing an operation id for another request is a
    /// usage error, exit 2, and never retryable.
    #[test]
    fn an_idempotency_conflict_exits_as_a_usage_error() {
        let code = FailureCode::IdempotencyConflict;
        assert_eq!(ProcessExit::from(code.class()).code(), 2);
        assert!(!code.retryable());
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
    fn json_lines_are_newline_terminated_and_written_in_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut lines = JsonLines::new();
        lines.push(&serde_json::json!({"sequence": 0}))?;
        lines.push(&serde_json::json!({"sequence": 1}))?;
        let mut stdout = Vec::new();
        let mut writer = OutputWriter::new(&mut stdout, Vec::<u8>::new());

        writer.write_json_lines(&lines)?;

        assert_eq!(stdout, b"{\"sequence\":0}\n{\"sequence\":1}\n");
        Ok(())
    }

    #[test]
    fn an_oversized_line_is_rejected_before_anything_is_written()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut lines = JsonLines::new();
        lines.push(&serde_json::json!({"sequence": 0}))?;

        let oversized = lines.push(&"a".repeat(1_048_576));

        assert!(matches!(oversized, Err(OutputError::TooLarge)));
        let mut stdout = Vec::new();
        OutputWriter::new(&mut stdout, Vec::<u8>::new()).write_json_lines(&lines)?;
        assert_eq!(stdout, b"{\"sequence\":0}\n");
        Ok(())
    }

    #[test]
    fn broken_stdout_fails_a_json_lines_stream_with_a_typed_io_error()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut lines = JsonLines::new();
        lines.push(&serde_json::json!({"sequence": 0}))?;
        let mut writer = OutputWriter::new(BrokenWriter, Vec::<u8>::new());

        let result = writer.write_json_lines(&lines);

        assert!(
            matches!(result, Err(OutputError::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe)
        );
        Ok(())
    }

    /// Records every write and flush, to prove each event is flushed as it
    /// is written.
    #[derive(Default)]
    struct FlushLog {
        bytes: Vec<u8>,
        flushed_at: Vec<usize>,
    }

    impl io::Write for FlushLog {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushed_at.push(self.bytes.len());
            Ok(())
        }
    }

    #[test]
    fn events_are_numbered_flushed_and_ended_by_the_terminal_event()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut log = FlushLog::default();
        let mut output = OutputWriter::new(&mut log, Vec::<u8>::new());
        let mut stream = JsonLinesWriter::new(&mut output);
        stream.write_event(
            |sequence| serde_json::json!({"event": "progress", "sequence": sequence}),
        )?;
        stream.write_event(
            |sequence| serde_json::json!({"event": "progress", "sequence": sequence}),
        )?;
        stream.write_terminal(OperationResponse::failure(
            "job.run",
            FailureCode::Cancelled,
        ))?;

        let lines: Vec<serde_json::Value> = log
            .bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(serde_json::from_slice)
            .collect::<Result<_, _>>()?;
        assert_eq!(lines.len(), 3);
        for (index, line) in lines.iter().enumerate() {
            assert_eq!(line["sequence"], u64::try_from(index)?);
        }
        assert_eq!(lines[2]["event"], "terminal");
        assert_eq!(lines[2]["result"]["error"]["code"], "CANCELLED");
        // One flush per line, each at the end of its line.
        assert_eq!(log.flushed_at.len(), 3);
        for offset in &log.flushed_at {
            assert_eq!(log.bytes.get(offset - 1), Some(&b'\n'));
        }
        Ok(())
    }

    #[test]
    fn an_event_over_its_line_budget_writes_nothing_and_keeps_its_number()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut stdout = Vec::new();
        let mut output = OutputWriter::new(&mut stdout, Vec::<u8>::new());
        let mut stream = JsonLinesWriter::new(&mut output);
        let oversized = stream.write_event(|_| "a".repeat(super::MAX_EVENT_LINE_BYTES));
        assert!(matches!(oversized, Err(OutputError::TooLarge)));
        stream.write_event(|sequence| serde_json::json!({"sequence": sequence}))?;
        // The terminal event keeps the whole result budget.
        stream.write_terminal(OperationResponse::complete(
            "job.batch",
            &"b".repeat(super::MAX_EVENT_LINE_BYTES),
        )?)?;
        let text = String::from_utf8(stdout)?;
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("{\"sequence\":0}"));
        let terminal: serde_json::Value = serde_json::from_str(lines.next().ok_or("terminal")?)?;
        assert_eq!(terminal["sequence"], 1);
        Ok(())
    }

    #[test]
    fn a_closed_stdout_fails_an_event_with_a_typed_io_error() {
        let mut output = OutputWriter::new(BrokenWriter, Vec::<u8>::new());
        let mut stream = JsonLinesWriter::new(&mut output);
        let result = stream.write_event(|sequence| serde_json::json!({"sequence": sequence}));
        assert!(
            matches!(result, Err(OutputError::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe)
        );
    }

    #[test]
    fn closed_stderr_is_ignored_without_panicking() {
        let mut writer = OutputWriter::new(Vec::<u8>::new(), BrokenWriter);

        writer.write_safe_diagnostic("safe diagnostic");
    }
}
