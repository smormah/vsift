//! Weighted admission to a session root and how long a caller waits for it
//! (P11, ADR 0021 section 5a; verification X-07).
//!
//! A root has an immutable capacity in weight units (4 for a desktop root,
//! 1 to 64 for a worker workspace). Every stage that occupies the machine
//! reserves the weight of what it runs, not one unit per process: a whisper.cpp
//! recognition takes its recognizer threads, a visual `FFmpeg` pass the two
//! threads it is given, and a source copy or one evidence extraction one
//! unit. The policy here is pure: the reservation itself is an OS-lock
//! adapter, and waiting is the caller's side effect.

use std::{
    num::{NonZeroU16, NonZeroUsize},
    time::Duration,
};

use crate::Jitter;

/// Weight of copying a source into a session or of one evidence extraction
/// (a frame, a crop, an audio clip, a probe): one unit.
pub const SINGLE_STAGE_WEIGHT: NonZeroU16 = NonZeroU16::MIN;

/// Weight of one visual-candidate window: its `FFmpeg` pass runs two
/// decoder threads (`-threads 2`), so it reserves two units.
pub const VISUAL_WINDOW_WEIGHT: NonZeroU16 = match NonZeroU16::new(2) {
    Some(weight) => weight,
    None => NonZeroU16::MIN,
};

/// Most recognizer threads a whisper.cpp run asks for. More gives little on
/// the pinned base model and would starve the rest of the machine.
pub const MAX_RECOGNIZER_THREADS: NonZeroU16 = match NonZeroU16::new(8) {
    Some(threads) => threads,
    None => NonZeroU16::MIN,
};

/// Recognizer threads assumed when the machine's parallelism is unknown.
const UNKNOWN_PARALLELISM_THREADS: NonZeroU16 = match NonZeroU16::new(4) {
    Some(threads) => threads,
    None => NonZeroU16::MIN,
};

/// The recognizer threads, and so the admission weight, of one whisper.cpp
/// run on a root of `capacity`: the machine's parallelism (four when it is
/// unknown), at most [`MAX_RECOGNIZER_THREADS`] and at most the root's
/// capacity, so a recognition always fits the root it runs in.
///
/// The count is recorded in the run's provenance, so it takes part in the
/// revision's identity: the same audio recognised on a root of smaller
/// capacity is a different run (known limit L-023).
#[must_use]
pub fn recognizer_threads(parallelism: Option<NonZeroUsize>, capacity: NonZeroU16) -> NonZeroU16 {
    let machine = parallelism
        .and_then(|threads| u16::try_from(threads.get()).ok())
        .and_then(NonZeroU16::new)
        .unwrap_or(match parallelism {
            // More than 65,535 threads: the cap below applies anyway.
            Some(_) => MAX_RECOGNIZER_THREADS,
            None => UNKNOWN_PARALLELISM_THREADS,
        });
    machine.min(MAX_RECOGNIZER_THREADS).min(capacity)
}

/// The longest admission wait a caller may ask for: 60 s.
pub const MAX_ADMISSION_WAIT: Duration = Duration::from_secs(60);
/// First polling ceiling of a bounded wait.
const POLL_BASE: Duration = Duration::from_millis(50);
/// Largest polling ceiling of a bounded wait.
const POLL_CAP: Duration = Duration::from_secs(1);
/// Shortest pause between polls, so a small jitter sample never spins.
const POLL_FLOOR: Duration = Duration::from_millis(10);
/// The retry hint a caller receives when admission stays busy: the same two
/// seconds as every other `BUSY` hint.
pub const ADMISSION_RETRY_AFTER: Duration = Duration::from_secs(2);

/// How long a bounded admission wait may last: more than zero and at most
/// [`MAX_ADMISSION_WAIT`].
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AdmissionBudget(Duration);

impl AdmissionBudget {
    /// The longest budget.
    pub const MAX: Self = Self(MAX_ADMISSION_WAIT);

    /// A budget of `wait`.
    ///
    /// # Errors
    ///
    /// [`AdmissionBudgetError`] for zero or more than 60 s.
    pub fn new(wait: Duration) -> Result<Self, AdmissionBudgetError> {
        if wait.is_zero() || wait > MAX_ADMISSION_WAIT {
            return Err(AdmissionBudgetError);
        }
        Ok(Self(wait))
    }

    /// The budget.
    #[must_use]
    pub const fn duration(self) -> Duration {
        self.0
    }
}

/// An admission budget of zero or more than 60 s.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionBudgetError;

impl std::fmt::Display for AdmissionBudgetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the admission wait must be more than zero and at most 60 s")
    }
}

impl std::error::Error for AdmissionBudgetError {}

/// What a caller does when the root's capacity is taken.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AdmissionWait {
    /// Do not wait: contention is reported at once, and the operation's own
    /// bounded retries (at most two, P10) apply. Interactive commands keep
    /// this, so an agent is never left waiting silently.
    #[default]
    Immediate,
    /// Poll with a full-jitter backoff until capacity frees or the budget
    /// is spent, then report `BUSY` with a retry hint (job commands, P11).
    Bounded(AdmissionBudget),
}

/// What follows one busy admission attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionDecision {
    /// Sleep this long, then try again.
    PollAfter(Duration),
    /// Stop: report `BUSY` and suggest retrying after this long.
    Busy {
        /// The retry hint.
        retry_after: Duration,
    },
}

impl AdmissionWait {
    /// Decides what follows the busy attempt number `polls` (0 for the
    /// first) after `waited` has already been spent waiting.
    ///
    /// A bounded wait sleeps a full-jitter delay, uniform below
    /// `min(1 s, 50 ms * 2^polls)` and at least 10 ms, and never past its
    /// budget: the last poll lands on the budget's edge, and once the
    /// budget is spent the answer is `Busy`. The total wait is therefore at
    /// most the budget whatever the jitter.
    #[must_use]
    pub fn decide(self, polls: u32, waited: Duration, jitter: Jitter) -> AdmissionDecision {
        let busy = AdmissionDecision::Busy {
            retry_after: ADMISSION_RETRY_AFTER,
        };
        let Self::Bounded(budget) = self else {
            return busy;
        };
        let Some(remaining) = budget
            .duration()
            .checked_sub(waited)
            .filter(|remaining| !remaining.is_zero())
        else {
            return busy;
        };
        let factor = 1_u32.checked_shl(polls).unwrap_or(u32::MAX);
        let ceiling = POLL_BASE
            .checked_mul(factor)
            .map_or(POLL_CAP, |ceiling| ceiling.min(POLL_CAP));
        let delay = jitter.scale(ceiling).max(POLL_FLOOR).min(remaining);
        AdmissionDecision::PollAfter(delay)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        num::{NonZeroU16, NonZeroUsize},
        time::Duration,
    };

    use super::{
        ADMISSION_RETRY_AFTER, AdmissionBudget, AdmissionBudgetError, AdmissionDecision,
        AdmissionWait, MAX_ADMISSION_WAIT, MAX_RECOGNIZER_THREADS, recognizer_threads,
    };
    use crate::Jitter;

    #[test]
    fn recognizer_threads_are_capped_by_capacity() -> Result<(), Box<dyn std::error::Error>> {
        let capacity = |value| NonZeroU16::new(value).ok_or("zero");
        let cores = |value| NonZeroUsize::new(value).ok_or("zero");
        for (parallelism, root, expected) in [
            (Some(cores(32)?), capacity(64)?, 8),
            (Some(cores(32)?), capacity(4)?, 4),
            (Some(cores(2)?), capacity(4)?, 2),
            (Some(cores(6)?), capacity(64)?, 6),
            (Some(cores(16)?), capacity(1)?, 1),
            (None, capacity(64)?, 4),
            (None, capacity(3)?, 3),
            (Some(cores(1_000_000)?), capacity(64)?, 8),
        ] {
            assert_eq!(
                recognizer_threads(parallelism, root).get(),
                expected,
                "{parallelism:?} {root}"
            );
            assert!(recognizer_threads(parallelism, root) <= root);
            assert!(recognizer_threads(parallelism, root) <= MAX_RECOGNIZER_THREADS);
        }
        Ok(())
    }

    #[test]
    fn a_budget_is_more_than_zero_and_at_most_a_minute() {
        assert_eq!(
            AdmissionBudget::new(Duration::ZERO),
            Err(AdmissionBudgetError)
        );
        assert_eq!(
            AdmissionBudget::new(MAX_ADMISSION_WAIT + Duration::from_millis(1)),
            Err(AdmissionBudgetError)
        );
        assert_eq!(
            AdmissionBudget::new(MAX_ADMISSION_WAIT),
            Ok(AdmissionBudget::MAX)
        );
    }

    #[test]
    fn an_immediate_wait_is_busy_at_once() {
        assert_eq!(
            AdmissionWait::Immediate.decide(0, Duration::ZERO, Jitter::FULL),
            AdmissionDecision::Busy {
                retry_after: ADMISSION_RETRY_AFTER
            }
        );
    }

    /// Whatever the jitter, a bounded wait sums to at most its budget, its
    /// last poll lands on the budget's edge, and it then answers busy.
    #[test]
    fn admission_wait_is_bounded_then_busy() -> Result<(), Box<dyn std::error::Error>> {
        for budget_ms in [1, 15, 250, 3_000, 60_000] {
            let budget = AdmissionBudget::new(Duration::from_millis(budget_ms))?;
            for jitter in [
                Jitter::NONE,
                Jitter::from_bits(1 << 31),
                Jitter::FULL,
                Jitter::from_bits(0x1234_5678),
            ] {
                let wait = AdmissionWait::Bounded(budget);
                let mut waited = Duration::ZERO;
                let mut polls = 0_u32;
                loop {
                    match wait.decide(polls, waited, jitter) {
                        AdmissionDecision::PollAfter(delay) => {
                            assert!(!delay.is_zero());
                            assert!(delay <= Duration::from_secs(1));
                            waited += delay;
                            polls += 1;
                            assert!(polls < 10_000, "the wait never ended");
                        }
                        AdmissionDecision::Busy { retry_after } => {
                            assert_eq!(retry_after, ADMISSION_RETRY_AFTER);
                            break;
                        }
                    }
                }
                assert_eq!(waited, budget.duration(), "{budget_ms} ms {jitter:?}");
            }
        }
        Ok(())
    }

    #[test]
    fn the_polling_ceiling_doubles_to_a_second() -> Result<(), Box<dyn std::error::Error>> {
        let wait = AdmissionWait::Bounded(AdmissionBudget::MAX);
        let delays: Vec<Duration> = (0..8)
            .map(
                |polls| match wait.decide(polls, Duration::ZERO, Jitter::FULL) {
                    AdmissionDecision::PollAfter(delay) => Ok(delay),
                    AdmissionDecision::Busy { .. } => Err("busy before the budget"),
                },
            )
            .collect::<Result<_, _>>()?;
        assert_eq!(
            delays,
            [49, 99, 199, 399, 799, 999, 999, 999]
                .map(Duration::from_millis)
                .to_vec()
        );
        assert_eq!(
            wait.decide(0, Duration::ZERO, Jitter::NONE),
            AdmissionDecision::PollAfter(Duration::from_millis(10))
        );
        Ok(())
    }
}
