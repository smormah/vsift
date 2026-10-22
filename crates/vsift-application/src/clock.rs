//! Wall-clock time as an injected port.
//!
//! Session expiry, renewal and cleanup eligibility all depend on "now". Reading the
//! system clock inside a use case would make those decisions untestable and would
//! hide a process-global input, so every host supplies a [`Clock`] explicitly.

use std::{error::Error, fmt};

/// Why the current time could not be read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockError {
    /// The clock reported a time before the Unix epoch, which no session
    /// lifetime can represent.
    BeforeUnixEpoch,
}

impl fmt::Display for ClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BeforeUnixEpoch => "clock reports a time before the Unix epoch",
        })
    }
}

impl Error for ClockError {}

/// Reads a monotonic clock: how long the process has run. A run reads it for
/// the interval between two published parts and for its speed (ADR 0033).
/// Injected, like every clock a use case reads.
pub trait Stopwatch: Send + Sync {
    /// Time since an origin fixed for the process.
    fn elapsed(&self) -> std::time::Duration;
}

/// A stopwatch that never advances: for hosts that neither publish as they
/// go nor show a speed. A run timed on it reports no speed and no estimate.
#[derive(Clone, Copy, Debug, Default)]
pub struct StoppedStopwatch;

impl Stopwatch for StoppedStopwatch {
    fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}

/// Port that reports the current wall-clock time.
///
/// Resolution is whole seconds because every lifecycle rule (idle expiry, the
/// maximum session age and cleanup eligibility) is defined in seconds.
pub trait Clock: Send + Sync {
    /// Returns the current time as whole seconds since the Unix epoch.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError`] when the time cannot be represented.
    fn now_unix_seconds(&self) -> Result<u64, ClockError>;
}
