//! The rules a worker host applies to one request (P11 PR 3, ADR 0021
//! sections 2, 4 and 6; verification X-09).
//!
//! Everything here is pure: the host supplies what its request record says,
//! whether another process holds the request, the failure a step ended with,
//! how long it has waited and how much of the request's deadline is left, and
//! a jitter sample; the rules answer. Sleeping, randomness and storage stay
//! with the caller, so every decision is reproducible in tests.

use std::time::Duration;

use crate::{AdmissionDecision, AdmissionWait, FailureCode, Jitter, RetryClass};

/// A step never starts with less of its request's deadline left than this:
/// it could not finish, and starting it would only leave more to resume.
pub const MIN_STEP_BUDGET: Duration = Duration::from_secs(1);

/// What a request record says about the operation id a request arrives with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordedRequest {
    /// Whether the record's request digest is the arriving request's.
    pub same_digest: bool,
    /// Whether the recorded request has ended (see [`ends_request`]).
    pub ended: bool,
}

/// What a host does with an arriving request, decided from its record and
/// from whether another process holds the request now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestAdmission {
    /// No record: a first attempt.
    Start,
    /// An ended record of the same request: return its result, run nothing.
    Replay,
    /// An unfinished record of the same request whose owner is gone:
    /// continue from its first unfinished step.
    Continue,
    /// Another process holds the same request: answer `BUSY`.
    Busy,
    /// The operation id is bound to another request: answer
    /// `IDEMPOTENCY_CONFLICT` and change nothing.
    Conflict,
}

/// Decides what an arriving request does (ADR 0021 section 4).
///
/// A different digest is a conflict whoever holds the record, so a
/// supervisor learns of a reused key at once. A request held elsewhere is
/// busy even before its record exists (the holder is writing it).
#[must_use]
pub const fn admit_request(
    record: Option<RecordedRequest>,
    held_elsewhere: bool,
) -> RequestAdmission {
    match (record, held_elsewhere) {
        (
            Some(RecordedRequest {
                same_digest: false, ..
            }),
            _,
        ) => RequestAdmission::Conflict,
        (_, true) => RequestAdmission::Busy,
        (None, false) => RequestAdmission::Start,
        (Some(RecordedRequest { ended: true, .. }), false) => RequestAdmission::Replay,
        (Some(RecordedRequest { ended: false, .. }), false) => RequestAdmission::Continue,
    }
}

/// Whether a request that finished its run with `failure` has ended, so its
/// result is recorded and replayed from now on.
///
/// A request that completed (or completed with a stated gap) has ended, and
/// so has one whose failure cannot change without a changed request or
/// setup. A failure the retry policy classes as transient or resumable
/// (`BUSY`, `DEADLINE_EXCEEDED`, `STORAGE_IO`, `CANCELLED`) leaves it
/// interrupted: the same request, delivered again, continues it.
#[must_use]
pub const fn ends_request(failure: Option<FailureCode>) -> bool {
    match failure {
        None => true,
        Some(code) => matches!(RetryClass::of(code), RetryClass::Permanent),
    }
}

/// What follows a step's failure inside one request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepRetry {
    /// Wait this long, then run the step again.
    After(Duration),
    /// Report the failure as it is.
    Report,
    /// The request's deadline cannot hold another attempt: report
    /// `DEADLINE_EXCEEDED`.
    DeadlineExceeded,
}

/// Decides what follows a step that failed with `code` after `polls` earlier
/// retries and `waited` spent waiting, with `remaining` of the request's
/// deadline left.
///
/// Only contention (`BUSY`: a taken admission slot, a busy session or writer,
/// the same job running elsewhere) is retried, with the host's bounded,
/// full-jitter [`AdmissionWait`], so many workers that collided once do not
/// collide again in lock-step; the wait is the admission wait of the whole
/// step. A retry that would leave less than [`MIN_STEP_BUDGET`] of the
/// deadline is not attempted, and the request is out of time. Everything
/// else is reported at once: permanent failures are never retried, and
/// resumable ones (`DEADLINE_EXCEEDED`, `STORAGE_IO`, `CANCELLED`) are the
/// caller's to redeliver.
#[must_use]
pub fn step_retry(
    code: FailureCode,
    polls: u32,
    waited: Duration,
    remaining: Duration,
    wait: AdmissionWait,
    jitter: Jitter,
) -> StepRetry {
    if !matches!(RetryClass::of(code), RetryClass::Transient) {
        return StepRetry::Report;
    }
    match wait.decide(polls, waited, jitter) {
        AdmissionDecision::Busy { .. } => StepRetry::Report,
        AdmissionDecision::PollAfter(delay) => {
            if remaining.saturating_sub(delay) < MIN_STEP_BUDGET {
                StepRetry::DeadlineExceeded
            } else {
                StepRetry::After(delay)
            }
        }
    }
}

/// Whether a step may start with `remaining` of its request's deadline left.
#[must_use]
pub fn step_may_start(remaining: Duration) -> bool {
    remaining >= MIN_STEP_BUDGET
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        MIN_STEP_BUDGET, RecordedRequest, RequestAdmission, StepRetry, admit_request, ends_request,
        step_may_start, step_retry,
    };
    use crate::{AdmissionBudget, AdmissionWait, FailureCode, Jitter};

    const fn recorded(same_digest: bool, ended: bool) -> RecordedRequest {
        RecordedRequest { same_digest, ended }
    }

    /// ADR 0021 section 4's table, and the two rows it adds for a request
    /// held before its record exists.
    #[test]
    fn the_record_table_decides_every_arrival() {
        for (record, held, expected) in [
            (None, false, RequestAdmission::Start),
            (None, true, RequestAdmission::Busy),
            (Some(recorded(true, true)), false, RequestAdmission::Replay),
            (
                Some(recorded(true, false)),
                false,
                RequestAdmission::Continue,
            ),
            (Some(recorded(true, false)), true, RequestAdmission::Busy),
            (Some(recorded(true, true)), true, RequestAdmission::Busy),
            (
                Some(recorded(false, true)),
                false,
                RequestAdmission::Conflict,
            ),
            (
                Some(recorded(false, false)),
                false,
                RequestAdmission::Conflict,
            ),
            (
                Some(recorded(false, false)),
                true,
                RequestAdmission::Conflict,
            ),
        ] {
            assert_eq!(admit_request(record, held), expected, "{record:?} {held}");
        }
    }

    #[test]
    fn only_permanent_failures_and_success_end_a_request() {
        assert!(ends_request(None));
        for code in FailureCode::ALL {
            let resumable = matches!(
                code,
                FailureCode::Busy
                    | FailureCode::DeadlineExceeded
                    | FailureCode::StorageIo
                    | FailureCode::Cancelled
            );
            assert_eq!(ends_request(Some(code)), !resumable, "{code:?}");
        }
    }

    fn bounded(millis: u64) -> Result<AdmissionWait, crate::AdmissionBudgetError> {
        AdmissionBudget::new(Duration::from_millis(millis)).map(AdmissionWait::Bounded)
    }

    /// X-09: contention is retried with jitter within the admission wait and
    /// the deadline; permanent and resumable failures are not retried.
    #[test]
    fn busy_is_retried_within_the_wait_and_nothing_else_is()
    -> Result<(), Box<dyn std::error::Error>> {
        let wait = bounded(5_000)?;
        let plenty = Duration::from_secs(600);
        assert!(matches!(
            step_retry(FailureCode::Busy, 0, Duration::ZERO, plenty, wait, Jitter::FULL),
            StepRetry::After(delay) if !delay.is_zero() && delay <= Duration::from_millis(50)
        ));
        for code in FailureCode::ALL {
            if code == FailureCode::Busy {
                continue;
            }
            assert_eq!(
                step_retry(code, 0, Duration::ZERO, plenty, wait, Jitter::FULL),
                StepRetry::Report,
                "{code:?}"
            );
        }
        // The admission wait is spent: report BUSY.
        assert_eq!(
            step_retry(
                FailureCode::Busy,
                9,
                Duration::from_millis(5_000),
                plenty,
                wait,
                Jitter::FULL
            ),
            StepRetry::Report
        );
        // An interactive wait never retries.
        assert_eq!(
            step_retry(
                FailureCode::Busy,
                0,
                Duration::ZERO,
                plenty,
                AdmissionWait::Immediate,
                Jitter::FULL
            ),
            StepRetry::Report
        );
        Ok(())
    }

    /// X-09: with the deadline nearly spent, a retry that would leave less
    /// than a step's minimum budget is not attempted.
    #[test]
    fn a_deadline_near_exhaustion_is_deadline_exceeded() -> Result<(), Box<dyn std::error::Error>> {
        let wait = bounded(60_000)?;
        assert_eq!(
            step_retry(
                FailureCode::Busy,
                5,
                Duration::from_secs(2),
                MIN_STEP_BUDGET,
                wait,
                Jitter::FULL
            ),
            StepRetry::DeadlineExceeded
        );
        assert!(!step_may_start(
            MIN_STEP_BUDGET.saturating_sub(Duration::from_millis(1))
        ));
        assert!(step_may_start(MIN_STEP_BUDGET));
        Ok(())
    }
}
