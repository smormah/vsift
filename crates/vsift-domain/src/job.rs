//! Legal lifecycle transitions for a recoverable job.

use std::{error::Error, fmt};

/// Durable lifecycle state of one job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobState {
    /// Accepted but not yet executing.
    Queued,
    /// A worker currently owns execution.
    Running,
    /// Cancellation was requested and is being reconciled with commit.
    Cancelling,
    /// All requested durable outputs committed successfully.
    Succeeded,
    /// The job reached a typed terminal failure.
    Failed,
    /// Cancellation won before a successful terminal commit.
    Cancelled,
}

impl JobState {
    /// Returns whether no further transition is legal.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    /// Applies one explicit legal transition.
    ///
    /// # Errors
    ///
    /// Returns [`JobTransitionError`] when the transition is not in the lifecycle graph.
    pub fn transition_to(self, next: Self) -> Result<Self, JobTransitionError> {
        let legal = matches!(
            (self, next),
            (
                Self::Queued,
                Self::Running | Self::Cancelling | Self::Cancelled
            ) | (
                Self::Running,
                Self::Cancelling | Self::Succeeded | Self::Failed
            ) | (
                Self::Cancelling,
                Self::Cancelled | Self::Succeeded | Self::Failed
            )
        );
        if legal {
            Ok(next)
        } else {
            Err(JobTransitionError {
                current: self,
                requested: next,
            })
        }
    }
}

/// Attempt to perform an illegal or duplicate job transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobTransitionError {
    current: JobState,
    requested: JobState,
}

impl JobTransitionError {
    /// Returns the state that rejected the transition.
    #[must_use]
    pub const fn current(self) -> JobState {
        self.current
    }

    /// Returns the requested next state.
    #[must_use]
    pub const fn requested(self) -> JobState {
        self.requested
    }
}

impl fmt::Display for JobTransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "illegal job transition from {:?} to {:?}",
            self.current, self.requested
        )
    }
}

impl Error for JobTransitionError {}

#[cfg(test)]
mod tests {
    use super::JobState;

    #[test]
    fn success_requires_running_or_cancellation_reconciliation() {
        assert!(JobState::Queued.transition_to(JobState::Succeeded).is_err());
        assert_eq!(
            JobState::Running.transition_to(JobState::Succeeded),
            Ok(JobState::Succeeded)
        );
        assert_eq!(
            JobState::Cancelling.transition_to(JobState::Succeeded),
            Ok(JobState::Succeeded)
        );
    }

    #[test]
    fn every_terminal_state_rejects_later_success_or_failure() {
        for terminal in [JobState::Succeeded, JobState::Failed, JobState::Cancelled] {
            assert!(terminal.is_terminal());
            assert!(terminal.transition_to(JobState::Succeeded).is_err());
            assert!(terminal.transition_to(JobState::Failed).is_err());
            assert!(terminal.transition_to(JobState::Cancelled).is_err());
        }
    }

    #[test]
    fn cancellation_has_one_terminal_outcome() {
        assert_eq!(
            JobState::Running.transition_to(JobState::Cancelling),
            Ok(JobState::Cancelling)
        );
        assert_eq!(
            JobState::Cancelling.transition_to(JobState::Cancelled),
            Ok(JobState::Cancelled)
        );
    }
}
