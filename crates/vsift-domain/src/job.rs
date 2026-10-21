//! Legal lifecycle transitions for a recoverable job (P10, ADR 0020).
//!
//! A job is the durable identity of one long operation, such as a
//! retranscription, so an interrupted run can be found and continued. The
//! state graph below is the only authority on which change is legal; storage
//! adapters apply a change only through [`JobState::transition_to`].
//!
//! ```text
//! Queued -> Running | Cancelling | Cancelled
//! Running -> Committing | Interrupted | Cancelling | Failed
//! Committing -> Succeeded | Failed | Interrupted
//! Interrupted -> Running | Cancelled
//! Cancelling -> Cancelled | Failed
//! ```
//!
//! Cancellation during `Committing` is too late: the commit transition is
//! serialized with cancellation, so a job is never both cancelled and
//! succeeded. `Committing -> Interrupted` is taken only after recovery found
//! that the commit did not land (the manifest chain is the source of truth),
//! so the work can be resumed rather than lost.

use std::{error::Error, fmt};

/// Durable lifecycle state of one job.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JobState {
    /// Accepted but not yet executing.
    Queued,
    /// A worker currently owns execution.
    Running,
    /// The job's result is being published; cancellation can no longer win.
    Committing,
    /// The last attempt stopped before committing; the job can be resumed.
    ///
    /// A job whose record still says `Running` or `Committing` but whose
    /// owner lock can be taken is interrupted too: its process ended without
    /// recording it (ADR 0020 section 4).
    Interrupted,
    /// Cancellation was requested and is being reconciled with commit.
    Cancelling,
    /// All requested durable outputs committed successfully.
    Succeeded,
    /// The job reached a typed terminal failure and cannot be resumed.
    Failed,
    /// Cancellation won before a successful terminal commit.
    Cancelled,
}

impl JobState {
    /// Every state, in lifecycle order.
    pub const ALL: [Self; 8] = [
        Self::Queued,
        Self::Running,
        Self::Committing,
        Self::Interrupted,
        Self::Cancelling,
        Self::Succeeded,
        Self::Failed,
        Self::Cancelled,
    ];

    /// Stable lowercase identifier used in stored records and contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Committing => "committing",
            Self::Interrupted => "interrupted",
            Self::Cancelling => "cancelling",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Parses a stable identifier.
    #[must_use]
    pub fn parse(identifier: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|state| state.identifier() == identifier)
    }

    /// Returns whether no further transition is legal.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    /// Whether a record in this state names a process that may still own the
    /// job, so only its owner lock can say whether the job is live.
    #[must_use]
    pub const fn claims_an_owner(self) -> bool {
        matches!(self, Self::Running | Self::Committing | Self::Cancelling)
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
                Self::Committing | Self::Interrupted | Self::Cancelling | Self::Failed
            ) | (
                Self::Committing,
                Self::Succeeded | Self::Failed | Self::Interrupted
            ) | (Self::Interrupted, Self::Running | Self::Cancelled)
                | (Self::Cancelling, Self::Cancelled | Self::Failed)
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

/// What kind of operation a job runs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JobKind {
    /// `transcript retranscribe`: local speech recognition of a session range.
    Retranscribe,
    /// `job start` (ADR 0033): the preparation of a session, step by step (the
    /// transcript, and first a window the caller named), run in the
    /// foreground or by a runner in the background. Its transcript is a
    /// retranscribe job of its own, which keeps its checkpoints.
    Prepare,
}

impl JobKind {
    /// Every kind.
    pub const ALL: [Self; 2] = [Self::Retranscribe, Self::Prepare];

    /// Stable lowercase identifier used in stored records and contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Retranscribe => "retranscribe",
            Self::Prepare => "prepare",
        }
    }

    /// Parses a stable identifier.
    #[must_use]
    pub fn parse(identifier: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.identifier() == identifier)
    }
}

/// How a job runs, chosen by who is waiting (ADR 0033 section 6). The modes
/// are parameters of one path, never separate code paths.
///
/// The third mode the record names, queued, comes with the root's queue.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JobMode {
    /// Someone waits for a short recording: the call runs in the foreground
    /// and answers when done; the revision is sealed in one generation.
    Quick,
    /// Someone waits for a long one: a runner works in the background,
    /// publishes the revision as it goes, reports its progress and leaves one
    /// unit of the root's admission capacity free for evidence.
    Watched,
}

impl JobMode {
    /// Every mode.
    pub const ALL: [Self; 2] = [Self::Quick, Self::Watched];

    /// The recordings shorter than this are quick by default; longer ones are
    /// watched (ADR 0033 section 6): ten minutes of media.
    ///
    /// Why ten minutes: a quick run answers in one call, and on the reference
    /// machines a ten-minute recording is recognised in about one to four
    /// minutes, which is as long as a caller should be kept waiting without
    /// a status. The figure is checked on the hosted real-media workflow.
    pub const QUICK_LIMIT_MICROS: u64 = 600_000_000;

    /// Stable lowercase identifier used in stored records and contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Watched => "watched",
        }
    }

    /// Parses a stable identifier.
    #[must_use]
    pub fn parse(identifier: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.identifier() == identifier)
    }

    /// The mode a request that names none runs in, for a recording of
    /// `duration_micros` (`None` when its length is not known yet): quick
    /// under [`Self::QUICK_LIMIT_MICROS`], watched from it on and when the
    /// length is unknown, so a long recording never keeps a caller waiting.
    #[must_use]
    pub const fn for_duration(duration_micros: Option<u64>) -> Self {
        match duration_micros {
            Some(duration) if duration < Self::QUICK_LIMIT_MICROS => Self::Quick,
            _ => Self::Watched,
        }
    }
}

/// One stream of a `prepare` job (ADR 0033 section 5): a piece of the
/// preparation, run in the order of [`PrepareStream::ALL`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PrepareStream {
    /// The transcript of the window the caller named first (`--first`, D5):
    /// a range retranscription on the whole run's chunk grid, sealed at once,
    /// whose chunks the whole transcript then reuses.
    FirstWindow,
    /// The visual-candidate index of the whole source (D6), published in
    /// batches of windows.
    Visual,
    /// The transcript: a whole-source retranscription.
    Transcript,
}

impl PrepareStream {
    /// Every stream, in the order a `prepare` job runs them.
    pub const ALL: [Self; 3] = [Self::FirstWindow, Self::Visual, Self::Transcript];

    /// Stable lowercase identifier used in stored records and contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::FirstWindow => "first",
            Self::Visual => "visual",
            Self::Transcript => "transcript",
        }
    }

    /// Parses a stable identifier.
    #[must_use]
    pub fn parse(identifier: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|stream| stream.identifier() == identifier)
    }
}

/// The longest window a `prepare` job transcribes first (`--first`): 30
/// minutes of media, so that what is held for the whole run to reuse stays
/// bounded (at most 72 R0 chunks).
pub const MAX_FIRST_WINDOW_MICROS: u64 = 1_800_000_000;

/// Attempt to perform an illegal or duplicate job transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobTransitionError {
    current: JobState,
    requested: JobState,
}

impl JobTransitionError {
    /// Describes a change a caller refused itself, for a change the graph
    /// has no edge for at all, such as restarting a job in a new epoch.
    #[must_use]
    pub const fn new(current: JobState, requested: JobState) -> Self {
        Self { current, requested }
    }

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
            "illegal job transition from {} to {}",
            self.current.identifier(),
            self.requested.identifier()
        )
    }
}

impl Error for JobTransitionError {}

#[cfg(test)]
mod tests {
    use super::{JobKind, JobMode, JobState, PrepareStream};

    /// ADR 0033 section 6: under ten minutes is quick, from ten minutes on and
    /// with no known length is watched; identifiers round-trip.
    #[test]
    fn the_mode_follows_the_recordings_length() {
        let limit = JobMode::QUICK_LIMIT_MICROS;
        assert_eq!(JobMode::for_duration(Some(0)), JobMode::Quick);
        assert_eq!(JobMode::for_duration(Some(limit - 1)), JobMode::Quick);
        assert_eq!(JobMode::for_duration(Some(limit)), JobMode::Watched);
        assert_eq!(
            JobMode::for_duration(Some(4 * 3_600_000_000)),
            JobMode::Watched
        );
        assert_eq!(JobMode::for_duration(None), JobMode::Watched);
        for mode in JobMode::ALL {
            assert_eq!(JobMode::parse(mode.identifier()), Some(mode));
        }
        for stream in PrepareStream::ALL {
            assert_eq!(PrepareStream::parse(stream.identifier()), Some(stream));
        }
        assert_eq!(JobKind::parse("prepare"), Some(JobKind::Prepare));
        assert_eq!(JobMode::parse("queued"), None);
    }

    /// C-09: the whole graph, edge by edge. Anything not listed is illegal.
    #[test]
    fn exactly_the_documented_transitions_are_legal() {
        let legal = [
            (JobState::Queued, JobState::Running),
            (JobState::Queued, JobState::Cancelling),
            (JobState::Queued, JobState::Cancelled),
            (JobState::Running, JobState::Committing),
            (JobState::Running, JobState::Interrupted),
            (JobState::Running, JobState::Cancelling),
            (JobState::Running, JobState::Failed),
            (JobState::Committing, JobState::Succeeded),
            (JobState::Committing, JobState::Failed),
            (JobState::Committing, JobState::Interrupted),
            (JobState::Interrupted, JobState::Running),
            (JobState::Interrupted, JobState::Cancelled),
            (JobState::Cancelling, JobState::Cancelled),
            (JobState::Cancelling, JobState::Failed),
        ];
        for from in JobState::ALL {
            for to in JobState::ALL {
                let expected = legal.contains(&(from, to));
                assert_eq!(
                    from.transition_to(to).is_ok(),
                    expected,
                    "{} -> {}",
                    from.identifier(),
                    to.identifier()
                );
            }
        }
    }

    /// C-09: success needs a commit; neither a running nor a cancelling job
    /// can jump to it.
    #[test]
    fn success_requires_the_committing_state() {
        assert!(JobState::Queued.transition_to(JobState::Succeeded).is_err());
        assert!(
            JobState::Running
                .transition_to(JobState::Succeeded)
                .is_err()
        );
        assert!(
            JobState::Cancelling
                .transition_to(JobState::Succeeded)
                .is_err()
        );
        assert_eq!(
            JobState::Running
                .transition_to(JobState::Committing)
                .and_then(|state| state.transition_to(JobState::Succeeded)),
            Ok(JobState::Succeeded)
        );
    }

    #[test]
    fn every_terminal_state_rejects_every_later_transition() {
        for terminal in [JobState::Succeeded, JobState::Failed, JobState::Cancelled] {
            assert!(terminal.is_terminal());
            assert!(!terminal.claims_an_owner());
            for next in JobState::ALL {
                assert!(terminal.transition_to(next).is_err());
            }
        }
    }

    /// C-09: cancellation has one terminal outcome, and a committing job
    /// cannot be cancelled (it is too late).
    #[test]
    fn cancellation_has_one_terminal_outcome_and_loses_to_a_commit() {
        assert_eq!(
            JobState::Running.transition_to(JobState::Cancelling),
            Ok(JobState::Cancelling)
        );
        assert_eq!(
            JobState::Cancelling.transition_to(JobState::Cancelled),
            Ok(JobState::Cancelled)
        );
        assert!(
            JobState::Committing
                .transition_to(JobState::Cancelling)
                .is_err()
        );
        assert!(
            JobState::Committing
                .transition_to(JobState::Cancelled)
                .is_err()
        );
    }

    /// An interrupted job resumes or is cancelled; it never jumps to a commit.
    #[test]
    fn an_interrupted_job_resumes_through_running() {
        assert_eq!(
            JobState::Running.transition_to(JobState::Interrupted),
            Ok(JobState::Interrupted)
        );
        assert_eq!(
            JobState::Interrupted.transition_to(JobState::Running),
            Ok(JobState::Running)
        );
        assert!(
            JobState::Interrupted
                .transition_to(JobState::Committing)
                .is_err()
        );
        assert!(
            JobState::Interrupted
                .transition_to(JobState::Succeeded)
                .is_err()
        );
    }

    #[test]
    fn identifiers_are_distinct_and_parse_back() {
        for (index, state) in JobState::ALL.into_iter().enumerate() {
            assert_eq!(JobState::parse(state.identifier()), Some(state));
            assert!(
                JobState::ALL[index + 1..]
                    .iter()
                    .all(|other| other.identifier() != state.identifier())
            );
        }
        assert_eq!(JobState::parse("RUNNING"), None);
        for kind in JobKind::ALL {
            assert_eq!(JobKind::parse(kind.identifier()), Some(kind));
        }
        assert_eq!(JobKind::parse("evidence"), None);
    }

    #[test]
    fn only_states_that_name_a_live_process_claim_an_owner() {
        for state in JobState::ALL {
            assert_eq!(
                state.claims_an_owner(),
                matches!(
                    state,
                    JobState::Running | JobState::Committing | JobState::Cancelling
                ),
                "{}",
                state.identifier()
            );
        }
    }
}
