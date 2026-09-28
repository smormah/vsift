//! Timers a worker host needs around one request (P11 PR 3, ADR 0021
//! sections 2 and 6): a deadline that cancels the running step, and a
//! backoff that a cancellation ends early.
//!
//! A step is never dropped mid-way at its deadline: dropping the future of a
//! step that runs a provider would leave the provider to the drop path. The
//! deadline instead cancels the step's signal, and the step stops at its next
//! boundary, reaps its providers and returns as it does for any cancellation;
//! the caller learns that it was the deadline.

use std::{future::Future, pin::pin, time::Duration, time::Instant};

use crate::ProcessCancellation;

/// Whether a step ended on its own or because its deadline passed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeadlineOutcome {
    /// The step ended before the deadline.
    Finished,
    /// The deadline passed first: the step's signal was cancelled and the
    /// step then ended.
    Passed,
}

/// Runs `run` to its end; if `deadline` passes first, cancels
/// `cancellation` (which `run` must observe) and still waits for `run`.
pub async fn run_until_deadline<T>(
    deadline: Instant,
    cancellation: &ProcessCancellation,
    run: impl Future<Output = T>,
) -> (T, DeadlineOutcome) {
    let mut run = pin!(run);
    tokio::select! {
        biased;
        output = &mut run => return (output, DeadlineOutcome::Finished),
        () = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {}
    }
    cancellation.cancel();
    (run.await, DeadlineOutcome::Passed)
}

/// Waits `delay` unless `cancellation` fires first; `true` when the whole
/// delay passed.
pub async fn sleep_unless_cancelled(delay: Duration, cancellation: &ProcessCancellation) -> bool {
    tokio::select! {
        biased;
        () = cancellation.wait_cancelled() => false,
        () = tokio::time::sleep(delay) => true,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{DeadlineOutcome, run_until_deadline, sleep_unless_cancelled};
    use crate::ProcessCancellation;

    #[tokio::test]
    async fn a_deadline_cancels_the_step_and_waits_for_it() {
        let cancellation = ProcessCancellation::new();
        let observed = cancellation.clone();
        let (stopped, outcome) = run_until_deadline(
            Instant::now() + Duration::from_millis(20),
            &cancellation,
            async move {
                observed.wait_cancelled().await;
                "stopped at its boundary"
            },
        )
        .await;
        assert_eq!(stopped, "stopped at its boundary");
        assert_eq!(outcome, DeadlineOutcome::Passed);

        let fresh = ProcessCancellation::new();
        let (value, outcome) =
            run_until_deadline(Instant::now() + Duration::from_secs(60), &fresh, async {
                7
            })
            .await;
        assert_eq!((value, outcome), (7, DeadlineOutcome::Finished));
        assert!(!fresh.is_cancelled());
    }

    #[tokio::test]
    async fn a_cancellation_ends_a_backoff() {
        let cancellation = ProcessCancellation::new();
        assert!(sleep_unless_cancelled(Duration::from_millis(1), &cancellation).await);
        cancellation.cancel();
        let started = Instant::now();
        assert!(!sleep_unless_cancelled(Duration::from_secs(60), &cancellation).await);
        assert!(started.elapsed() < Duration::from_secs(30));
    }
}
