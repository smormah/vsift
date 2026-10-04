//! The known-limits register's review lines, as the claims check reads them.
//!
//! A statement of the claims registry may list the register entries its
//! wording leans on (`limits`). The maintainer reviews those entries once, on
//! one sheet (`docs/planning/register-review-sheet.md`), and writes the result
//! on each entry's `Review` line: `pending`, `accepted (date)`,
//! `rejected (date): reason` or `rescheduled to ... (date)`. The check reads
//! that line and nothing else of an entry; it does not judge the entry.

use std::collections::BTreeMap;

/// The register, relative to the repository root.
pub(crate) const REGISTER_PATH: &str = "docs/planning/known-limits.md";

/// What the maintainer wrote on one entry's `Review` line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Review {
    /// Not reviewed yet.
    Pending,
    /// The limit stands as described.
    Accepted,
    /// The limit must be fixed.
    Rejected,
    /// Another packet or issue owns it now.
    Rescheduled,
    /// The line says something else, or the entry has no review line.
    Unrecognised(String),
}

impl Review {
    /// Whether a public statement may lean on an entry in this state: the
    /// maintainer has decided that it stands or that someone else owns it.
    pub(crate) fn lets_a_claim_lean(&self) -> bool {
        matches!(self, Self::Accepted | Self::Rescheduled)
    }

    /// The state as the messages name it.
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Pending => String::from("pending"),
            Self::Accepted => String::from("accepted"),
            Self::Rejected => String::from("rejected (the limit must be fixed)"),
            Self::Rescheduled => String::from("rescheduled"),
            Self::Unrecognised(text) => {
                format!("not recognised ({text:?}; use pending, accepted, rejected or rescheduled)")
            }
        }
    }
}

/// The review of every entry of the register, by entry id (`L-004`).
pub(crate) fn reviews(register: &str) -> BTreeMap<String, Review> {
    let mut found = BTreeMap::new();
    let mut current: Option<(String, String)> = None;
    for line in register.lines() {
        if let Some(heading) = line.strip_prefix("### ") {
            finish(&mut found, current.take());
            let id = heading.split_whitespace().next().unwrap_or_default();
            current = is_entry_id(id).then(|| (id.to_owned(), String::new()));
        } else if line.starts_with("## ") {
            finish(&mut found, current.take());
        } else if let Some((_, body)) = current.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    finish(&mut found, current.take());
    found
}

/// Whether `text` has the shape of an entry identifier, `L-` and digits.
pub(crate) fn is_entry_id(text: &str) -> bool {
    text.strip_prefix("L-")
        .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
}

fn finish(found: &mut BTreeMap<String, Review>, entry: Option<(String, String)>) {
    if let Some((id, body)) = entry {
        found.insert(id, review_of(&body));
    }
}

/// The state written after the entry's last `**Review:**` label, up to the
/// next blank line. The text may wrap onto the next line.
fn review_of(body: &str) -> Review {
    const LABEL: &str = "**Review:**";
    let Some(start) = body.rfind(LABEL) else {
        return Review::Unrecognised(String::from("no review line"));
    };
    let after = body.get(start + LABEL.len()..).unwrap_or_default();
    let paragraph = after.split("\n\n").next().unwrap_or_default();
    let said = paragraph
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches('.')
        .to_owned();
    let lower = said.to_lowercase();
    if lower.starts_with("pending") {
        Review::Pending
    } else if lower.starts_with("accepted") {
        Review::Accepted
    } else if lower.starts_with("rejected") {
        Review::Rejected
    } else if lower.starts_with("rescheduled") {
        Review::Rescheduled
    } else {
        Review::Unrecognised(said)
    }
}

#[cfg(test)]
mod tests {
    use super::{Review, is_entry_id, reviews};

    const REGISTER: &str = "# Register\n\n## Security\n\n\
### L-001\n\n**A title.**\n\n- **Owner:** unscheduled. **Status:** accepted residual.\n  **Review:** pending.\n\n\
### L-002\n\n**A title.**\n\n- **Owner:** unscheduled. **Review:**\n  accepted (2026-10-05).\n\n\
### L-003\n\n- **Status:** deferred. **Review:** rejected (2026-10-05): fix it.\n\n\
### L-004\n\n- **Review:** rescheduled to R1 (2026-10-03).\n\n*A later note.*\n\n\
### L-005\n\n- **Owner:** nobody.\n\n\
## Next section\n\n### L-006\n\n- **Review:** Accepted (2026-10-06)\n\n\
### Not an entry\n\n- **Review:** accepted.\n";

    #[test]
    fn each_review_line_is_read_even_when_it_wraps() {
        let found = reviews(REGISTER);
        assert_eq!(found.get("L-001"), Some(&Review::Pending));
        assert_eq!(found.get("L-002"), Some(&Review::Accepted));
        assert_eq!(found.get("L-003"), Some(&Review::Rejected));
        assert_eq!(found.get("L-004"), Some(&Review::Rescheduled));
        assert_eq!(found.get("L-006"), Some(&Review::Accepted));
        assert_eq!(found.len(), 6, "{found:?}");
    }

    #[test]
    fn an_entry_without_a_review_line_is_not_taken_for_reviewed() {
        assert_eq!(
            reviews(REGISTER).get("L-005"),
            Some(&Review::Unrecognised(String::from("no review line")))
        );
    }

    #[test]
    fn only_a_decision_lets_a_claim_lean_on_an_entry() {
        assert!(Review::Accepted.lets_a_claim_lean());
        assert!(Review::Rescheduled.lets_a_claim_lean());
        assert!(!Review::Pending.lets_a_claim_lean());
        assert!(!Review::Rejected.lets_a_claim_lean());
        assert!(!Review::Unrecognised(String::from("maybe")).lets_a_claim_lean());
    }

    #[test]
    fn entry_identifiers_are_l_and_digits() {
        assert!(is_entry_id("L-004"));
        assert!(is_entry_id("L-1234"));
        for text in ["L-", "L-4x", "l-004", "L004", "RQ-01", ""] {
            assert!(!is_entry_id(text), "{text:?}");
        }
    }
}
