//! Stable failure classification shared by public hosts.

/// Exit-level failure category for one public operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureClass {
    /// An unexpected invariant or implementation failure.
    Internal,
    /// Invalid usage, configuration, schema, or required capability.
    UsageOrCapability,
    /// Invalid, unsupported, or changed source input.
    Source,
    /// A condition that can be retried under caller policy.
    Retryable,
    /// A deadline or resource budget ended the operation.
    Limit,
    /// Caller or host cancellation won the terminal transition.
    Cancelled,
    /// Storage, integrity, or output I/O prevented success.
    StorageOrIo,
}

/// Stable machine-readable code for an expected or translated public failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureCode {
    /// Unexpected internal failure.
    Internal,
    /// CLI option, value, combination, or configuration was invalid.
    InvalidArgument,
    /// A request used an unsupported schema major.
    UnsupportedSchema,
    /// A required runtime capability is missing or incompatible.
    MissingCapability,
    /// The host cannot provide a required process or worker isolation boundary.
    IsolationUnavailable,
    /// The command is reserved by R0 but its implementation packet is incomplete.
    CommandNotImplemented,
    /// Source bytes or media structure are invalid or unsupported.
    InvalidSource,
    /// A provider or admission controller reported retryable pressure.
    Busy,
    /// The operation exceeded its deadline.
    DeadlineExceeded,
    /// A byte, count, memory, disk, or process budget was exceeded.
    ResourceLimit,
    /// Cancellation completed before a success commit.
    Cancelled,
    /// Storage or output I/O prevented the promised result.
    StorageIo,
    /// Committed or imported data failed integrity validation.
    IntegrityFailure,
}

impl FailureCode {
    /// Returns the stable uppercase JSON identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Internal => "INTERNAL",
            Self::InvalidArgument => "INVALID_ARGUMENT",
            Self::UnsupportedSchema => "UNSUPPORTED_SCHEMA",
            Self::MissingCapability => "MISSING_CAPABILITY",
            Self::IsolationUnavailable => "ISOLATION_UNAVAILABLE",
            Self::CommandNotImplemented => "COMMAND_NOT_IMPLEMENTED",
            Self::InvalidSource => "INVALID_SOURCE",
            Self::Busy => "BUSY",
            Self::DeadlineExceeded => "DEADLINE_EXCEEDED",
            Self::ResourceLimit => "RESOURCE_LIMIT",
            Self::Cancelled => "CANCELLED",
            Self::StorageIo => "STORAGE_IO",
            Self::IntegrityFailure => "INTEGRITY_FAILURE",
        }
    }

    /// Returns the process-level failure category.
    #[must_use]
    pub const fn class(self) -> FailureClass {
        match self {
            Self::Internal => FailureClass::Internal,
            Self::InvalidArgument
            | Self::UnsupportedSchema
            | Self::MissingCapability
            | Self::IsolationUnavailable
            | Self::CommandNotImplemented => FailureClass::UsageOrCapability,
            Self::InvalidSource => FailureClass::Source,
            Self::Busy => FailureClass::Retryable,
            Self::DeadlineExceeded | Self::ResourceLimit => FailureClass::Limit,
            Self::Cancelled => FailureClass::Cancelled,
            Self::StorageIo | Self::IntegrityFailure => FailureClass::StorageOrIo,
        }
    }

    /// Returns whether retry can be useful without changing the request.
    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(self, Self::Busy)
    }
}

/// Terminal status represented by public JSON and JSONL contracts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationStatus {
    /// All requested work completed; warnings may still be present.
    Complete,
    /// Useful output committed but one or more requested optional stages failed.
    Partial,
    /// Requested work failed without a committed successful terminal result.
    Failed,
    /// Cancellation won the terminal transition.
    Cancelled,
}

impl OperationStatus {
    /// Returns the stable lowercase JSON identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FailureClass, FailureCode, OperationStatus};

    #[test]
    fn public_failure_codes_map_to_documented_exit_classes() {
        assert_eq!(
            FailureCode::InvalidArgument.class(),
            FailureClass::UsageOrCapability
        );
        assert_eq!(
            FailureCode::IsolationUnavailable.identifier(),
            "ISOLATION_UNAVAILABLE"
        );
        assert_eq!(FailureCode::InvalidSource.class(), FailureClass::Source);
        assert_eq!(FailureCode::Busy.class(), FailureClass::Retryable);
        assert_eq!(FailureCode::DeadlineExceeded.class(), FailureClass::Limit);
        assert_eq!(FailureCode::Cancelled.class(), FailureClass::Cancelled);
        assert_eq!(FailureCode::StorageIo.class(), FailureClass::StorageOrIo);
        assert_eq!(FailureCode::Internal.class(), FailureClass::Internal);
    }

    #[test]
    fn only_retryable_codes_claim_automatic_retry_semantics() {
        assert!(FailureCode::Busy.retryable());
        for code in [
            FailureCode::Internal,
            FailureCode::InvalidArgument,
            FailureCode::UnsupportedSchema,
            FailureCode::MissingCapability,
            FailureCode::IsolationUnavailable,
            FailureCode::CommandNotImplemented,
            FailureCode::InvalidSource,
            FailureCode::DeadlineExceeded,
            FailureCode::ResourceLimit,
            FailureCode::Cancelled,
            FailureCode::StorageIo,
            FailureCode::IntegrityFailure,
        ] {
            assert!(!code.retryable());
        }
    }

    #[test]
    fn operation_terminal_status_identifiers_are_stable() {
        for (status, identifier) in [
            (OperationStatus::Complete, "complete"),
            (OperationStatus::Partial, "partial"),
            (OperationStatus::Failed, "failed"),
            (OperationStatus::Cancelled, "cancelled"),
        ] {
            assert_eq!(status.identifier(), identifier);
        }
    }
}
