//! The summary of a `job batch`: `job-batch-data` v1 (P11, ADR 0021), and the
//! batch's outcome rule (maintainer decision D5).
//!
//! A batch streams one [`crate::WorkResult`] per request as it finishes; its
//! terminal event carries this summary: counts per status, one bounded item
//! per line it processed, the first line it did not start (after a shutdown
//! or an input error) and why it stopped.

use std::{error::Error, fmt};

use serde::Serialize;
use vsift_domain::{FailureClass, FailureCode, OperationId, OperationStatus};

use crate::{MAX_BATCH_LINES, RequestRejection};

/// What happened to one line of a batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BatchItemStatus {
    /// The request completed.
    Complete,
    /// The request completed with a stated gap.
    Partial,
    /// The request failed.
    Failed,
    /// The request was cancelled.
    Cancelled,
    /// The line was refused before it ran.
    Rejected,
}

impl BatchItemStatus {
    /// Every status, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::Complete,
        Self::Partial,
        Self::Failed,
        Self::Cancelled,
        Self::Rejected,
    ];

    /// The stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Rejected => "rejected",
        }
    }

    /// The item status of a request result.
    #[must_use]
    pub const fn of_result(status: OperationStatus) -> Self {
        match status {
            OperationStatus::Complete => Self::Complete,
            OperationStatus::Partial => Self::Partial,
            OperationStatus::Failed => Self::Failed,
            OperationStatus::Cancelled => Self::Cancelled,
        }
    }
}

/// Why a batch stopped reading.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BatchTermination {
    /// Every line was read and every admitted request finished.
    EndOfInput,
    /// A shutdown signal stopped admission.
    Shutdown,
    /// The file held more than [`MAX_BATCH_LINES`] lines.
    LineLimit,
    /// The file could not be read further.
    InputError,
}

impl BatchTermination {
    /// Every termination, in declaration order.
    pub const ALL: [Self; 4] = [
        Self::EndOfInput,
        Self::Shutdown,
        Self::LineLimit,
        Self::InputError,
    ];

    /// The stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::EndOfInput => "end_of_input",
            Self::Shutdown => "shutdown",
            Self::LineLimit => "line_limit",
            Self::InputError => "input_error",
        }
    }
}

/// The failure code a batch ends with, if not success (D5).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BatchOutcome {
    /// Every request was complete or partial: exit 0.
    Success,
    /// A shutdown stopped the batch: `CANCELLED`, exit 6.
    Stopped,
    /// The most severe failure among the lines and the input itself.
    Failed(FailureCode),
}

/// Per-status line counts.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct BatchCounts {
    complete: u32,
    partial: u32,
    failed: u32,
    cancelled: u32,
    rejected: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct BatchItem {
    line: u32,
    operation_id: Option<String>,
    status: &'static str,
    code: Option<&'static str>,
    rejection: Option<&'static str>,
    #[serde(skip)]
    state: BatchItemStatus,
    #[serde(skip)]
    failure: Option<FailureCode>,
}

/// Why a batch summary could not take another item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BatchDataError {
    /// More than [`MAX_BATCH_LINES`] items.
    TooManyItems,
    /// A line number of 0 or beyond [`MAX_BATCH_LINES`].
    InvalidLine,
}

impl fmt::Display for BatchDataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManyItems => "a batch summary holds too many items",
            Self::InvalidLine => "a batch line number is out of range",
        })
    }
}

impl Error for BatchDataError {}

/// `job-batch-data` v1.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JobBatchData {
    counts: BatchCounts,
    items: Vec<BatchItem>,
    not_started_from_line: Option<u32>,
    termination_reason: &'static str,
    #[serde(skip)]
    termination: BatchTermination,
}

impl Default for JobBatchData {
    fn default() -> Self {
        Self::new()
    }
}

impl JobBatchData {
    /// An empty summary of a batch that is still reading.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            counts: BatchCounts {
                complete: 0,
                partial: 0,
                failed: 0,
                cancelled: 0,
                rejected: 0,
            },
            items: Vec::new(),
            not_started_from_line: None,
            termination_reason: BatchTermination::EndOfInput.identifier(),
            termination: BatchTermination::EndOfInput,
        }
    }

    /// Records a request's result for its 1-based `line`.
    ///
    /// # Errors
    ///
    /// [`BatchDataError`] beyond the batch's bounds.
    pub fn record_result(
        &mut self,
        line: u32,
        operation_id: &OperationId,
        status: OperationStatus,
        failure: Option<FailureCode>,
    ) -> Result<(), BatchDataError> {
        let state = BatchItemStatus::of_result(status);
        self.push(BatchItem {
            line,
            operation_id: Some(operation_id.as_str().to_owned()),
            status: state.identifier(),
            code: failure.map(FailureCode::identifier),
            rejection: None,
            state,
            failure,
        })
    }

    /// Records a refused line, with its operation id when it had a valid one.
    ///
    /// # Errors
    ///
    /// [`BatchDataError`] beyond the batch's bounds.
    pub fn record_rejection(
        &mut self,
        line: u32,
        operation_id: Option<&OperationId>,
        rejection: RequestRejection,
    ) -> Result<(), BatchDataError> {
        self.push(BatchItem {
            line,
            operation_id: operation_id.map(|id| id.as_str().to_owned()),
            status: BatchItemStatus::Rejected.identifier(),
            code: Some(rejection.failure_code().identifier()),
            rejection: Some(rejection.identifier()),
            state: BatchItemStatus::Rejected,
            failure: Some(rejection.failure_code()),
        })
    }

    fn push(&mut self, item: BatchItem) -> Result<(), BatchDataError> {
        if item.line == 0 || usize::try_from(item.line).map_or(true, |line| line > MAX_BATCH_LINES)
        {
            return Err(BatchDataError::InvalidLine);
        }
        if self.items.len() >= MAX_BATCH_LINES {
            return Err(BatchDataError::TooManyItems);
        }
        let count = match item.state {
            BatchItemStatus::Complete => &mut self.counts.complete,
            BatchItemStatus::Partial => &mut self.counts.partial,
            BatchItemStatus::Failed => &mut self.counts.failed,
            BatchItemStatus::Cancelled => &mut self.counts.cancelled,
            BatchItemStatus::Rejected => &mut self.counts.rejected,
        };
        *count = count.saturating_add(1);
        self.items.push(item);
        Ok(())
    }

    /// Ends the summary: why the batch stopped, and the first line it did
    /// not start, if any.
    #[must_use]
    pub fn finish(
        mut self,
        termination: BatchTermination,
        not_started_from_line: Option<u32>,
    ) -> Self {
        self.termination = termination;
        self.termination_reason = termination.identifier();
        self.not_started_from_line = not_started_from_line;
        self
    }

    /// The batch's outcome (maintainer decision D5): success when every
    /// line's request was complete or partial; stopped when a shutdown
    /// stopped it; otherwise the most severe failure among the lines and
    /// the input, in the class order storage or I/O (exit 7), internal (1),
    /// limit (5), source (3), usage or capability (2), then retryable (4). A
    /// request cancelled without a shutdown (by `job cancel`) ranks between
    /// usage and retryable. An input error counts as `STORAGE_IO` and a
    /// line limit as `RESOURCE_LIMIT`.
    #[must_use]
    pub fn outcome(&self) -> BatchOutcome {
        if self.termination == BatchTermination::Shutdown {
            return BatchOutcome::Stopped;
        }
        let input = match self.termination {
            BatchTermination::InputError => Some(FailureCode::StorageIo),
            BatchTermination::LineLimit => Some(FailureCode::ResourceLimit),
            BatchTermination::EndOfInput | BatchTermination::Shutdown => None,
        };
        self.items
            .iter()
            .filter_map(|item| item.failure)
            .chain(input)
            .min_by_key(|code| severity(code.class()))
            .map_or(BatchOutcome::Success, BatchOutcome::Failed)
    }
}

/// Lower is more severe.
const fn severity(class: FailureClass) -> u8 {
    match class {
        FailureClass::StorageOrIo => 0,
        FailureClass::Internal => 1,
        FailureClass::Limit => 2,
        FailureClass::Source => 3,
        FailureClass::UsageOrCapability => 4,
        FailureClass::Cancelled => 5,
        FailureClass::Retryable => 6,
    }
}

#[cfg(test)]
mod tests {
    use vsift_domain::{FailureCode, OperationId, OperationStatus};

    use super::{BatchDataError, BatchOutcome, BatchTermination, JobBatchData};
    use crate::{MAX_BATCH_LINES, RequestRejection};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn id() -> Result<OperationId, vsift_domain::IdentifierError> {
        OperationId::parse("op_0123456789abcdef")
    }

    #[test]
    fn complete_and_partial_lines_are_success() -> TestResult {
        let mut data = JobBatchData::new();
        data.record_result(1, &id()?, OperationStatus::Complete, None)?;
        data.record_result(2, &id()?, OperationStatus::Partial, None)?;
        assert_eq!(
            data.finish(BatchTermination::EndOfInput, None).outcome(),
            BatchOutcome::Success
        );
        Ok(())
    }

    #[test]
    fn a_shutdown_wins_over_every_failure() -> TestResult {
        let mut data = JobBatchData::new();
        data.record_result(
            1,
            &id()?,
            OperationStatus::Failed,
            Some(FailureCode::StorageIo),
        )?;
        assert_eq!(
            data.finish(BatchTermination::Shutdown, Some(2)).outcome(),
            BatchOutcome::Stopped
        );
        Ok(())
    }

    /// D5's order: 7 > 1 > 5 > 3 > 2 > 4.
    #[test]
    fn the_most_severe_class_decides() -> TestResult {
        let ladder = [
            FailureCode::StorageIo,
            FailureCode::Internal,
            FailureCode::DeadlineExceeded,
            FailureCode::InvalidSource,
            FailureCode::InvalidArgument,
            FailureCode::Cancelled,
            FailureCode::Busy,
        ];
        for (index, expected) in ladder.iter().enumerate() {
            let mut data = JobBatchData::new();
            let mut line = 1;
            for code in ladder.iter().skip(index).rev() {
                data.record_result(line, &id()?, OperationStatus::Failed, Some(*code))?;
                line += 1;
            }
            data.record_result(line, &id()?, OperationStatus::Complete, None)?;
            assert_eq!(
                data.finish(BatchTermination::EndOfInput, None).outcome(),
                BatchOutcome::Failed(*expected)
            );
        }
        Ok(())
    }

    #[test]
    fn rejections_and_input_limits_count() -> TestResult {
        let mut data = JobBatchData::new();
        data.record_rejection(1, None, RequestRejection::Malformed)?;
        let rejected = data.clone().finish(BatchTermination::EndOfInput, None);
        assert_eq!(
            rejected.outcome(),
            BatchOutcome::Failed(FailureCode::InvalidArgument)
        );
        let value = serde_json::to_value(&rejected)?;
        assert_eq!(value["counts"]["rejected"], 1);
        assert_eq!(value["items"][0]["rejection"], "malformed_request");
        assert_eq!(
            data.clone()
                .finish(BatchTermination::LineLimit, Some(1_001))
                .outcome(),
            BatchOutcome::Failed(FailureCode::ResourceLimit)
        );
        assert_eq!(
            data.finish(BatchTermination::InputError, Some(2)).outcome(),
            BatchOutcome::Failed(FailureCode::StorageIo)
        );
        Ok(())
    }

    #[test]
    fn lines_and_items_are_bounded() -> TestResult {
        let mut data = JobBatchData::new();
        assert_eq!(
            data.record_result(0, &id()?, OperationStatus::Complete, None),
            Err(BatchDataError::InvalidLine)
        );
        for line in 1..=u32::try_from(MAX_BATCH_LINES)? {
            data.record_result(line, &id()?, OperationStatus::Complete, None)?;
        }
        assert_eq!(
            data.record_result(1, &id()?, OperationStatus::Complete, None),
            Err(BatchDataError::TooManyItems)
        );
        assert!(serde_json::to_vec(&data)?.len() < 1_048_576);
        Ok(())
    }
}
