//! Which failures a job retries by itself, when, and which make it
//! non-resumable (P10, ADR 0020 section 5; verification X-09).
//!
//! The policy is pure: it is given the failure's public code, how many
//! automatic retries the operation has already used, how much of its deadline
//! is left and a jitter sample, and answers with a decision. Randomness and
//! sleeping belong to the caller, so every decision is reproducible in tests.
//!
//! - `BUSY` from contention that clears in seconds (admission slots, the
//!   session writer lock, a generation moved by a renewal) is retried at most
//!   twice, after a full-jitter exponential backoff (base 200 ms, cap 2 s): the
//!   delay is uniform in `[0, min(cap, base * 2^retry))`, so many callers that
//!   collided once do not collide again in lock-step. A retry that would not
//!   fit in the remaining deadline is not attempted.
//! - `DEADLINE_EXCEEDED`, `STORAGE_IO` and `CANCELLED` go back to the caller
//!   with the job left resumable: its finished chunks are kept.
//! - Everything else is never retried automatically: invalid input, a missing
//!   capability, integrity, unsupported schemas, resource limits and
//!   idempotency conflicts cannot change without a changed request or setup.
//! - A chunk that fails three times with the same code poisons the job: it
//!   stops being resumable, so a deterministic failure cannot loop forever.

use std::time::Duration;

use crate::FailureCode;

/// Most automatic retries of one operation.
pub const MAX_AUTOMATIC_RETRIES: u32 = 2;
/// Backoff before the first automatic retry, before jitter.
pub const RETRY_BASE_DELAY: Duration = Duration::from_millis(200);
/// Longest backoff before any automatic retry, before jitter.
pub const RETRY_MAX_DELAY: Duration = Duration::from_secs(2);
/// Identical failures at one chunk that make a job non-resumable.
pub const POISON_THRESHOLD: usize = 3;
/// Most attempts one job may start before it fails for good.
pub const MAX_JOB_ATTEMPTS: u32 = 16;

/// A uniform sample in `[0, 1)`, as the fraction `bits / 2^32`.
///
/// Integer arithmetic keeps the backoff exact and identical on every
/// platform; the caller draws the bits from its random source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Jitter(u32);

impl Jitter {
    /// No delay at all: the smallest sample.
    pub const NONE: Self = Self(0);
    /// The largest sample, just under the whole backoff ceiling.
    pub const FULL: Self = Self(u32::MAX);

    /// A sample from 32 random bits.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// Scales `ceiling` by this sample, rounding down to a whole millisecond.
    #[must_use]
    pub fn scale(self, ceiling: Duration) -> Duration {
        let millis = u64::try_from(ceiling.as_millis()).unwrap_or(u64::MAX);
        let scaled = (u128::from(millis) * u128::from(self.0)) >> 32;
        Duration::from_millis(u64::try_from(scaled).unwrap_or(u64::MAX))
    }
}

/// How the policy classifies one failure code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryClass {
    /// Contention that clears quickly: retried automatically within budget.
    Transient,
    /// Stops the attempt, but the job stays resumable for the caller.
    Resumable,
    /// Never retried without a changed request or setup.
    Permanent,
}

impl RetryClass {
    /// The class of `code`.
    #[must_use]
    pub const fn of(code: FailureCode) -> Self {
        match code {
            FailureCode::Busy => Self::Transient,
            FailureCode::DeadlineExceeded | FailureCode::StorageIo | FailureCode::Cancelled => {
                Self::Resumable
            }
            FailureCode::Internal
            | FailureCode::InvalidArgument
            | FailureCode::UnsupportedSchema
            | FailureCode::MissingCapability
            | FailureCode::IsolationUnavailable
            | FailureCode::CommandNotImplemented
            | FailureCode::InvalidSource
            | FailureCode::ResourceLimit
            | FailureCode::IntegrityFailure
            | FailureCode::IdempotencyConflict
            // Only `setup install` downloads, and it is not a job: the
            // user reruns the command, nothing retries it automatically.
            | FailureCode::DownloadFailed => Self::Permanent,
        }
    }
}

/// What to do after one failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryDecision {
    /// Wait this long, then retry automatically.
    RetryAfter(Duration),
    /// Report the failure; the job stays resumable and the caller decides.
    LeaveToCaller,
    /// Report the failure; retrying the same request cannot succeed.
    Never,
}

/// The automatic retry policy: how often and how long to back off.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    max_automatic: u32,
    base: Duration,
    cap: Duration,
}

impl RetryPolicy {
    /// R0: at most two retries, full jitter over 200 ms doubling to 2 s.
    pub const R0: Self = Self {
        max_automatic: MAX_AUTOMATIC_RETRIES,
        base: RETRY_BASE_DELAY,
        cap: RETRY_MAX_DELAY,
    };

    /// The backoff ceiling before retry number `retry` (0 for the first):
    /// `min(cap, base * 2^retry)`.
    #[must_use]
    pub fn ceiling(self, retry: u32) -> Duration {
        let factor = 1_u32.checked_shl(retry).unwrap_or(u32::MAX);
        self.base
            .checked_mul(factor)
            .map_or(self.cap, |delay| delay.min(self.cap))
    }

    /// Decides what follows a failure with `code`, when `retries_used`
    /// automatic retries have already run and `remaining` is what is left of
    /// the operation's deadline (`None` without one).
    #[must_use]
    pub fn decide(
        self,
        code: FailureCode,
        retries_used: u32,
        remaining: Option<Duration>,
        jitter: Jitter,
    ) -> RetryDecision {
        match RetryClass::of(code) {
            RetryClass::Permanent => RetryDecision::Never,
            RetryClass::Resumable => RetryDecision::LeaveToCaller,
            RetryClass::Transient => {
                if retries_used >= self.max_automatic {
                    return RetryDecision::LeaveToCaller;
                }
                let delay = jitter.scale(self.ceiling(retries_used));
                if remaining.is_some_and(|remaining| remaining < delay) {
                    RetryDecision::LeaveToCaller
                } else {
                    RetryDecision::RetryAfter(delay)
                }
            }
        }
    }
}

/// One failed attempt of a job: the chunk it stopped at, if it was working
/// on one, and the failure's public code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptFailure {
    /// Zero-based index of the chunk the attempt failed at.
    pub chunk: Option<u32>,
    /// Public code of the failure.
    pub code: FailureCode,
}

/// The chunk that has failed [`POISON_THRESHOLD`] times with the same code,
/// if any: the job is then not resumable.
///
/// Cancellations never count, because the caller chose to stop; nor do
/// failures outside a chunk (admission, commit), which say nothing about
/// the chunk's audio.
#[must_use]
pub fn poisoned_chunk(failures: &[AttemptFailure]) -> Option<u32> {
    failures.iter().find_map(|failure| {
        let chunk = failure.chunk?;
        if failure.code == FailureCode::Cancelled {
            return None;
        }
        let same = failures
            .iter()
            .filter(|other| other.chunk == Some(chunk) && other.code == failure.code)
            .count();
        (same >= POISON_THRESHOLD).then_some(chunk)
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        AttemptFailure, Jitter, MAX_AUTOMATIC_RETRIES, RetryClass, RetryDecision, RetryPolicy,
        poisoned_chunk,
    };
    use crate::FailureCode;

    const HALF: Jitter = Jitter::from_bits(1 << 31);

    /// X-09: every public code has exactly one class, and the table is the
    /// one ADR 0020 records.
    #[test]
    fn every_failure_code_is_classified_as_documented() {
        for code in FailureCode::ALL {
            let expected = match code {
                FailureCode::Busy => RetryClass::Transient,
                FailureCode::DeadlineExceeded | FailureCode::StorageIo | FailureCode::Cancelled => {
                    RetryClass::Resumable
                }
                _ => RetryClass::Permanent,
            };
            assert_eq!(RetryClass::of(code), expected, "{}", code.identifier());
        }
        for never in [
            FailureCode::InvalidArgument,
            FailureCode::InvalidSource,
            FailureCode::MissingCapability,
            FailureCode::IntegrityFailure,
            FailureCode::UnsupportedSchema,
            FailureCode::ResourceLimit,
            FailureCode::IdempotencyConflict,
        ] {
            assert_eq!(
                RetryPolicy::R0.decide(never, 0, None, HALF),
                RetryDecision::Never,
                "{}",
                never.identifier()
            );
        }
        for resumable in [
            FailureCode::DeadlineExceeded,
            FailureCode::StorageIo,
            FailureCode::Cancelled,
        ] {
            assert_eq!(
                RetryPolicy::R0.decide(resumable, 0, None, HALF),
                RetryDecision::LeaveToCaller,
                "{}",
                resumable.identifier()
            );
        }
    }

    /// X-09: busy is retried at most twice, with full-jitter exponential
    /// backoff capped at two seconds.
    #[test]
    fn busy_is_retried_twice_with_full_jitter_backoff() {
        let policy = RetryPolicy::R0;
        assert_eq!(policy.ceiling(0), Duration::from_millis(200));
        assert_eq!(policy.ceiling(1), Duration::from_millis(400));
        assert_eq!(policy.ceiling(3), Duration::from_millis(1_600));
        assert_eq!(policy.ceiling(4), Duration::from_secs(2));
        assert_eq!(policy.ceiling(40), Duration::from_secs(2));
        for (retries, jitter, expected) in [
            (0, Jitter::NONE, RetryDecision::RetryAfter(Duration::ZERO)),
            (
                0,
                HALF,
                RetryDecision::RetryAfter(Duration::from_millis(100)),
            ),
            (
                1,
                HALF,
                RetryDecision::RetryAfter(Duration::from_millis(200)),
            ),
            (
                1,
                Jitter::FULL,
                RetryDecision::RetryAfter(Duration::from_millis(399)),
            ),
            (
                MAX_AUTOMATIC_RETRIES,
                Jitter::NONE,
                RetryDecision::LeaveToCaller,
            ),
            (7, HALF, RetryDecision::LeaveToCaller),
        ] {
            assert_eq!(
                policy.decide(FailureCode::Busy, retries, None, jitter),
                expected,
                "retry {retries}"
            );
        }
    }

    /// X-09: a backoff that does not fit the remaining deadline is skipped.
    #[test]
    fn a_retry_that_would_outlive_the_deadline_is_not_attempted() {
        let policy = RetryPolicy::R0;
        assert_eq!(
            policy.decide(FailureCode::Busy, 1, Some(Duration::from_millis(199)), HALF),
            RetryDecision::LeaveToCaller
        );
        assert_eq!(
            policy.decide(FailureCode::Busy, 1, Some(Duration::from_millis(200)), HALF),
            RetryDecision::RetryAfter(Duration::from_millis(200))
        );
        assert_eq!(
            policy.decide(FailureCode::Busy, 0, Some(Duration::ZERO), Jitter::NONE),
            RetryDecision::RetryAfter(Duration::ZERO)
        );
    }

    /// X-09: different samples spread retries; no two callers that drew
    /// different bits wait the same time (no synchronised amplification).
    #[test]
    fn jitter_spreads_retries_across_the_whole_window() {
        let delays: Vec<Duration> = (0_u32..16)
            .map(|step| Jitter::from_bits(step << 28).scale(Duration::from_millis(1_600)))
            .collect();
        for (index, delay) in delays.iter().enumerate() {
            assert!(*delay < Duration::from_millis(1_600));
            assert!(delays[index + 1..].iter().all(|other| other != delay));
        }
        assert_eq!(delays[0], Duration::ZERO);
        assert_eq!(delays[8], Duration::from_millis(800));
    }

    /// Three identical failures at one chunk poison the job; different
    /// codes, different chunks, failures outside a chunk and cancellations
    /// do not.
    #[test]
    fn three_identical_failures_at_one_chunk_poison_the_job() {
        let at = |chunk: Option<u32>, code: FailureCode| AttemptFailure { chunk, code };
        let deadline = FailureCode::DeadlineExceeded;
        assert_eq!(poisoned_chunk(&[]), None);
        assert_eq!(
            poisoned_chunk(&[at(Some(4), deadline), at(Some(4), deadline)]),
            None
        );
        assert_eq!(
            poisoned_chunk(&[
                at(Some(4), deadline),
                at(Some(5), deadline),
                at(Some(4), FailureCode::StorageIo),
                at(Some(4), deadline),
            ]),
            None
        );
        assert_eq!(
            poisoned_chunk(&[
                at(Some(4), deadline),
                at(None, FailureCode::Busy),
                at(Some(4), deadline),
                at(Some(4), deadline),
            ]),
            Some(4)
        );
        assert_eq!(
            poisoned_chunk(&[at(None, deadline), at(None, deadline), at(None, deadline)]),
            None
        );
        let cancelled = at(Some(2), FailureCode::Cancelled);
        assert_eq!(poisoned_chunk(&[cancelled, cancelled, cancelled]), None);
    }
}
