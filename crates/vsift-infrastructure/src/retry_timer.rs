//! The retry policy's side effects on a real host: operating-system
//! randomness for backoff jitter and a Tokio timer (P10, ADR 0020 section 5).
//!
//! Jitter comes from the operating system's random source so independent
//! processes that collided once do not back off in lock-step. If the source
//! fails, the largest sample is used: the retry then waits its whole
//! ceiling, which is always safe, rather than not waiting at all.

use std::{future::Future, time::Duration};

use vsift_application::RetryTimer;
use vsift_domain::Jitter;

/// Jitter from the operating system and waits on the Tokio timer.
#[derive(Clone, Copy, Debug, Default)]
pub struct TokioRetryTimer;

impl RetryTimer for TokioRetryTimer {
    fn jitter(&self) -> Jitter {
        let mut bytes = [0_u8; 4];
        match getrandom::fill(&mut bytes) {
            Ok(()) => Jitter::from_bits(u32::from_le_bytes(bytes)),
            Err(_) => Jitter::FULL,
        }
    }

    fn sleep(&self, delay: Duration) -> impl Future<Output = ()> + Send {
        tokio::time::sleep(delay)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use vsift_application::RetryTimer;

    use super::TokioRetryTimer;

    #[tokio::test]
    async fn the_timer_waits_and_draws_varied_jitter() {
        let timer = TokioRetryTimer;
        let started = Instant::now();
        timer.sleep(Duration::from_millis(20)).await;
        assert!(started.elapsed() >= Duration::from_millis(20));
        let samples: Vec<_> = (0..8).map(|_| timer.jitter()).collect();
        assert!(samples.iter().any(|sample| *sample != samples[0]));
    }
}
