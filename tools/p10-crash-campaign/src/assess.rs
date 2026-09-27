//! Layer C: write errors. Assesses one round of the workload during which
//! the harness switched the device to failing every write and flush.
//!
//! The harness appends `INJECT <unix_ns>` to the round's log once the
//! failing table is live. Every failure must carry the public code
//! `STORAGE_IO`, none may happen before the injection, and no operation that
//! started after the injection may be acknowledged: it could only have been
//! acknowledged by ignoring a failed write or flush. Operations that started
//! before and were acknowledged after are allowed here; the verification
//! after the device is restored holds the store to them.

use std::collections::BTreeMap;

use crate::protocol::parse_events;

/// What one round showed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Assessment {
    /// When the failing table went live, if it did.
    pub injected_at: Option<u128>,
    /// Failures after the injection.
    pub injected_failures: usize,
    /// Of those, the ones answered `STORAGE_IO`.
    pub storage_io: usize,
    /// Failures with any other code, as `(seq, code)`.
    pub other_codes: Vec<(u64, String)>,
    /// Failures before the injection (none should happen).
    pub failures_before_injection: usize,
    /// Operations that started after the injection and were acknowledged.
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

/// Assesses the log of one round.
#[must_use]
pub fn assess(text: &str) -> Assessment {
    let injected_at = text.lines().find_map(|line| {
        line.trim_end_matches('\r')
            .strip_prefix("INJECT ")
            .and_then(|value| value.parse::<u128>().ok())
    });
    let events = parse_events(text);
    let started: BTreeMap<u64, u128> = events
        .starts
        .iter()
        .map(|start| (start.seq, start.unix_ns))
        .collect();
    let mut assessment = Assessment {
        injected_at,
        acks: events.acks.len(),
        ..Assessment::default()
    };
    for failure in &events.failures {
        if injected_at.is_some_and(|at| failure.unix_ns >= at) {
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
