//! Layer C: write errors. Assesses one round of the workload during which
//! the harness switched the device to failing every write and flush.
//!
//! The harness appends `INJECT-BEGIN <unix_ns>` just before it suspends the
//! device to swap in the failing table and `INJECT <unix_ns>` once that
//! table is live. Every failure must carry the public code `STORAGE_IO` and
//! none may happen before the swap began. No operation that started after
//! the failing table was live may be acknowledged: it could only have been
//! acknowledged by ignoring a failed write or flush. An operation that
//! started earlier may be acknowledged (its writes can have reached the
//! device before the swap); the verification after the device is restored
//! holds the store to it.

use std::collections::BTreeMap;

use crate::protocol::parse_events;

/// What one round showed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Assessment {
    /// When the swap to the failing table began, if it did.
    pub injection_began: Option<u128>,
    /// When the failing table was live, if it was.
    pub injected_at: Option<u128>,
    /// Failures after the swap began.
    pub injected_failures: usize,
    /// Of those, the ones answered `STORAGE_IO`.
    pub storage_io: usize,
    /// Failures with any other code, as `(seq, code)`.
    pub other_codes: Vec<(u64, String)>,
    /// Failures before the swap began (none should happen).
    pub failures_before_injection: usize,
    /// Operations that started once the failing table was live and were
    /// acknowledged.
    pub acknowledged_after_injection: Vec<u64>,
    /// Acknowledgements in the round.
    pub acks: usize,
}

impl Assessment {
    /// Whether the round met layer C's criteria.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.other_codes.is_empty()
            && self.failures_before_injection == 0
            && self.acknowledged_after_injection.is_empty()
    }

    /// The assessment as one report line.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "ASSESS {} injected={} injected_failures={} storage_io={} other_codes={:?} \
             failures_before_injection={} acknowledged_after_injection={:?} acks={}",
            if self.passed() { "OK" } else { "FAIL" },
            self.injected_at.is_some(),
            self.injected_failures,
            self.storage_io,
            self.other_codes,
            self.failures_before_injection,
            self.acknowledged_after_injection,
            self.acks
        )
    }
}

/// The timestamp of the first line `prefix <unix_ns>`.
fn stamp(text: &str, prefix: &str) -> Option<u128> {
    text.lines().find_map(|line| {
        line.trim_end_matches('\r')
            .strip_prefix(prefix)
            .and_then(|value| value.parse::<u128>().ok())
    })
}

/// Assesses the log of one round.
#[must_use]
pub fn assess(text: &str) -> Assessment {
    let injected_at = stamp(text, "INJECT ");
    let injection_began = stamp(text, "INJECT-BEGIN ").or(injected_at);
    let events = parse_events(text);
    let started: BTreeMap<u64, u128> = events
        .starts
        .iter()
        .map(|start| (start.seq, start.unix_ns))
        .collect();
    let mut assessment = Assessment {
        injection_began,
        injected_at,
        acks: events.acks.len(),
        ..Assessment::default()
    };
    for failure in &events.failures {
        if injection_began.is_some_and(|at| failure.unix_ns >= at) {
            assessment.injected_failures += 1;
            if failure.code == "STORAGE_IO" {
                assessment.storage_io += 1;
            }
        } else {
            assessment.failures_before_injection += 1;
        }
        if failure.code != "STORAGE_IO" {
            assessment
                .other_codes
                .push((failure.seq, failure.code.clone()));
        }
    }
    if let Some(at) = injected_at {
        assessment.acknowledged_after_injection = events
            .acks
            .iter()
            .filter(|ack| started.get(&ack.seq).is_some_and(|start| *start >= at))
            .map(|ack| ack.seq)
            .collect();
    }
    assessment
}

#[cfg(test)]
mod tests {
    use super::assess;

    const ACK: &str = "ses_0123456789abcdef0123456789abcdef 3 \
        0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef - -";

    #[test]
    fn typed_failures_after_the_injection_pass() {
        let log = format!(
            "START 1 100 renew\nACK 1 150 renew {ACK}\nSTART 2 190 renew\nINJECT 200\n\
             FAIL 2 250 renew STORAGE_IO\nSTART 3 300 evidence\nFAIL 3 350 evidence STORAGE_IO\n"
        );
        let assessment = assess(&log);
        assert!(assessment.passed(), "{assessment:?}");
        assert_eq!(assessment.injected_failures, 2);
        assert_eq!(assessment.storage_io, 2);
        assert_eq!(assessment.acks, 1);
    }

    /// The swap of tables takes time: a failure after it began is injected,
    /// and an operation that started before the failing table was live may
    /// still have been acknowledged.
    #[test]
    fn the_swap_window_counts_as_injected_but_does_not_bind() {
        let log = format!(
            "INJECT-BEGIN 180\nSTART 1 185 renew\nACK 1 187 renew {ACK}\nSTART 2 188 renew\n\
             FAIL 2 190 renew STORAGE_IO\nINJECT 200\n"
        );
        let assessment = assess(&log);
        assert!(assessment.passed(), "{assessment:?}");
        assert_eq!(assessment.injected_failures, 1);
        assert_eq!(assessment.failures_before_injection, 0);
    }

    #[test]
    fn an_acknowledgement_after_the_injection_or_another_code_fails() {
        let log = format!(
            "INJECT 200\nSTART 1 300 renew\nACK 1 350 renew {ACK}\nSTART 2 400 renew\n\
             FAIL 2 450 renew INTEGRITY_FAILURE\nFAIL 3 50 renew STORAGE_IO\n"
        );
        let assessment = assess(&log);
        assert!(!assessment.passed());
        assert_eq!(assessment.acknowledged_after_injection, vec![1]);
        assert_eq!(
            assessment.other_codes,
            vec![(2, "INTEGRITY_FAILURE".to_owned())]
        );
        assert_eq!(assessment.failures_before_injection, 1);
    }
}
