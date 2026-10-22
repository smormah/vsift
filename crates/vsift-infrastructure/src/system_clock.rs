//! The operating-system wall clock behind the application [`Clock`] port.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use vsift_application::{Clock, ClockError, Stopwatch};

/// Reads the host's real-time clock.
///
/// Hosts compose this adapter into the engine in production; tests inject a
/// fixed clock instead so expiry decisions are deterministic.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_seconds(&self) -> Result<u64, ClockError> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .map_err(|_| ClockError::BeforeUnixEpoch)
    }
}

/// The host's monotonic clock, from the moment it was made: the interval
/// between two parts of a revision published as it goes (ADR 0033).
#[derive(Clone, Copy, Debug)]
pub struct MonotonicStopwatch(Instant);

impl MonotonicStopwatch {
    /// A stopwatch started now.
    #[must_use]
    pub fn started() -> Self {
        Self(Instant::now())
    }
}

impl Stopwatch for MonotonicStopwatch {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use vsift_application::Clock;

    use super::SystemClock;

    #[test]
    fn system_clock_reports_a_time_after_the_epoch() -> Result<(), Box<dyn std::error::Error>> {
        // 2020-09-13: any correctly set clock running this test is later.
        assert!(SystemClock.now_unix_seconds()? > 1_600_000_000);
        Ok(())
    }
}
