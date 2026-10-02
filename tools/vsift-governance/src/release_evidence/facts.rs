//! The identifiers the evidence ledger may refer to, read from the documents
//! that own them.
//!
//! Nothing is copied into the checker: the item set comes from the plan's
//! table, requirements from the delivery ledger, threats from the threat
//! model, rows from `verification.md` and limits from the register. A ledger
//! entry that names an identifier those documents do not hold is refused.

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::repository::Repository;

/// The delivery ledger, which owns the requirement identifiers.
pub(crate) const DELIVERY_LEDGER: &str = "docs/planning/delivery-ledger.json";
/// The threat model, which owns the `SEC-nn` identifiers.
pub(crate) const THREAT_MODEL: &str = "docs/planning/security-threat-model.md";
/// The verification plan, which owns the test and evidence row identifiers.
pub(crate) const VERIFICATION: &str = "docs/planning/verification.md";
/// The known-limits register, which owns the `L-nnn` identifiers.
pub(crate) const KNOWN_LIMITS: &str = "docs/planning/known-limits.md";

/// The identifiers found in the owning documents.
#[derive(Debug, Default)]
pub(crate) struct Facts {
    /// The `RQ-nn` items the plan defines, in the plan's order.
    pub(crate) plan_items: Vec<String>,
    /// The delivery ledger's requirement identifiers.
    pub(crate) requirements: BTreeSet<String>,
    /// The threat model's `SEC-nn` identifiers.
    pub(crate) threats: BTreeSet<String>,
    /// Every row identifier of the verification plan.
    pub(crate) verification_rows: BTreeSet<String>,
    /// Every limit of the register.
    pub(crate) limits: BTreeSet<String>,
    /// The limits whose owner names P14.
    pub(crate) p14_limits: BTreeSet<String>,
}

impl Facts {
    /// Reads the plan at `plan_path` and the four owning documents.
    ///
    /// Returns one message per document that could not be read or held no
    /// identifier at all (a table that was renamed would otherwise make every
    /// reference look unknown, or worse, make an empty set look complete).
    pub(crate) fn read(repository: &dyn Repository, plan_path: &str) -> Result<Self, Vec<String>> {
        let mut messages = Vec::new();
        let mut read = |path: &str| match repository.read_text(path) {
            Ok(text) => Some(text),
            Err(error) => {
                messages.push(error);
                None
            }
        };
        let plan = read(plan_path);
        let delivery = read(DELIVERY_LEDGER);
        let threat_model = read(THREAT_MODEL);
        let verification = read(VERIFICATION);
        let limits = read(KNOWN_LIMITS);

        let (requirements, mapped_threats) = delivery
            .as_deref()
            .map(|text| delivery_ledger_facts(text, &mut messages))
            .unwrap_or_default();
        let defined_threats = threat_model.as_deref().map(threat_ids).unwrap_or_default();
        if threat_model.is_some() {
            for id in mapped_threats.difference(&defined_threats) {
                messages.push(format!(
                    "{DELIVERY_LEDGER} maps P14 to {id}, which {THREAT_MODEL} does not define"
                ));
            }
        }

        let facts = Self {
            plan_items: plan.as_deref().map(plan_item_ids).unwrap_or_default(),
            requirements,
            threats: mapped_threats,
            verification_rows: verification
                .as_deref()
                .map(verification_row_ids)
                .unwrap_or_default(),
            limits: limits.as_deref().map(limit_ids).unwrap_or_default(),
            p14_limits: limits.as_deref().map(p14_limit_ids).unwrap_or_default(),
        };
        for (present, empty, source) in [
            (plan.is_some(), facts.plan_items.is_empty(), plan_path),
            (
                delivery.is_some(),
                facts.requirements.is_empty() || facts.threats.is_empty(),
                DELIVERY_LEDGER,
            ),
            (
                threat_model.is_some(),
                defined_threats.is_empty(),
                THREAT_MODEL,
            ),
            (
                verification.is_some(),
                facts.verification_rows.is_empty(),
                VERIFICATION,
            ),
            (limits.is_some(), facts.limits.is_empty(), KNOWN_LIMITS),
        ] {
            if present && empty {
                messages.push(format!("{source} holds none of the identifiers it owns"));
            }
        }
        if messages.is_empty() {
            Ok(facts)
        } else {
            Err(messages)
        }
    }
}

/// The `RQ-nn` identifiers that open a table row of the plan.
pub(crate) fn plan_item_ids(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for cell in table_first_cells(text) {
        if is_numbered(cell, "RQ-", 2) && !found.iter().any(|seen| seen == cell) {
            found.push(cell.to_owned());
        }
    }
    found
}

/// The delivery ledger's requirement identifiers and the threats it maps to
/// P14 (`SEC-01..SEC-25`: the R0 threats; the threat model also lists R1's).
fn delivery_ledger_facts(
    text: &str,
    messages: &mut Vec<String>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    #[derive(Deserialize)]
    struct Ledger {
        requirements: Vec<Identified>,
        packets: Vec<Packet>,
    }
    #[derive(Deserialize)]
    struct Identified {
        id: String,
    }
    #[derive(Deserialize)]
    struct Packet {
        id: String,
        threats: Vec<String>,
    }
    let ledger = match serde_json::from_str::<Ledger>(text) {
        Ok(ledger) => ledger,
        Err(error) => {
            messages.push(format!("{DELIVERY_LEDGER} could not be read: {error}"));
            return (BTreeSet::new(), BTreeSet::new());
        }
    };
    let requirements = ledger
        .requirements
        .into_iter()
        .map(|item| item.id)
        .collect();
    let tokens: Vec<String> = ledger
        .packets
        .into_iter()
        .filter(|packet| packet.id == "P14")
        .flat_map(|packet| packet.threats)
        .collect();
    let threats = match expand_threats(&tokens) {
        Ok(threats) => threats,
        Err(error) => {
            messages.push(format!("{DELIVERY_LEDGER}: {error}"));
            BTreeSet::new()
        }
    };
    (requirements, threats)
}

/// Expands threat tokens: `SEC-07` is itself, `SEC-01..SEC-25` is every
/// identifier in the range.
fn expand_threats(tokens: &[String]) -> Result<BTreeSet<String>, String> {
    let number = |id: &str| {
        id.strip_prefix("SEC-")
            .filter(|digits| digits.len() == 2)
            .and_then(|digits| digits.parse::<u8>().ok())
    };
    let mut threats = BTreeSet::new();
    for token in tokens {
        match token.split_once("..") {
            None => {
                number(token).ok_or_else(|| format!("threat {token:?} is not SEC-nn"))?;
                threats.insert(token.clone());
            }
            Some((first, last)) => {
                let (Some(first), Some(last)) = (number(first), number(last)) else {
                    return Err(format!("threat range {token:?} is not SEC-nn..SEC-nn"));
                };
                threats.extend((first..=last).map(|index| format!("SEC-{index:02}")));
            }
        }
    }
    Ok(threats)
}

/// The `SEC-nn` identifiers that open a table row of the threat model (the
/// first cell reads `SEC-01 P0`: identifier, then priority).
fn threat_ids(text: &str) -> BTreeSet<String> {
    table_first_cells(text)
        .filter_map(|cell| cell.split_whitespace().next())
        .filter(|token| is_numbered(token, "SEC-", 2))
        .map(str::to_owned)
        .collect()
}

/// Every row identifier of the verification plan: the first cell of a table
/// row, and the identifier that opens a bullet (`- SEC-T01: ...`).
fn verification_row_ids(text: &str) -> BTreeSet<String> {
    let rows = table_first_cells(text).filter(|cell| is_row_id(cell));
    let bullets = text.lines().filter_map(|line| {
        let (token, _) = line.strip_prefix("- ")?.split_once(':')?;
        is_row_id(token).then_some(token)
    });
    rows.chain(bullets).map(str::to_owned).collect()
}

/// The `L-nnn` identifiers of the register's entry headings.
fn limit_ids(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|line| line.strip_prefix("### "))
        .map(str::trim)
        .filter(|token| is_numbered(token, "L-", 3))
        .map(str::to_owned)
        .collect()
}

/// The limits whose Owner cell of the summary table names P14.
///
/// The cells are read from the end of the row (Owner, Issue, Status), so a
/// title that contains a pipe cannot shift them.
fn p14_limit_ids(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for line in text.lines() {
        let Some(row) = line.strip_prefix("| [L-") else {
            continue;
        };
        let Some((number, _)) = row.split_once(']') else {
            continue;
        };
        let id = format!("L-{number}");
        if !is_numbered(&id, "L-", 3) {
            continue;
        }
        let cells: Vec<&str> = line.trim().trim_matches('|').split('|').collect();
        let owner = cells
            .len()
            .checked_sub(3)
            .and_then(|index| cells.get(index));
        if owner.is_some_and(|cell| {
            cell.split(|c: char| !c.is_ascii_alphanumeric())
                .any(|word| word == "P14")
        }) {
            found.insert(id);
        }
    }
    found
}

/// The first cell of every Markdown table row, trimmed.
fn table_first_cells(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter_map(|line| {
        let rest = line.trim_start().strip_prefix('|')?;
        Some(rest.split('|').next()?.trim())
    })
}

/// Whether `token` is `prefix` followed by exactly `digits` decimal digits.
pub(crate) fn is_numbered(token: &str, prefix: &str, digits: usize) -> bool {
    token.strip_prefix(prefix).is_some_and(|number| {
        number.len() == digits && number.bytes().all(|byte| byte.is_ascii_digit())
    })
}

/// Whether `token` is a test or evidence row identifier: up to three capital
/// letters, a hyphen, optional capital letters and one to three digits
/// (`C-01`, `SEC-T01`, `R-SEC03`, `RQ-01`, `A-10`).
pub(crate) fn is_row_id(token: &str) -> bool {
    let Some((prefix, rest)) = token.split_once('-') else {
        return false;
    };
    let capitals = |text: &str| text.bytes().all(|byte| byte.is_ascii_uppercase());
    let digits = rest.trim_start_matches(|character: char| character.is_ascii_uppercase());
    let letters_in_rest = rest.len() - digits.len();
    (1..=3).contains(&prefix.len())
        && capitals(prefix)
        && letters_in_rest <= 3
        && (1..=3).contains(&digits.len())
        && digits.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::{
        DELIVERY_LEDGER, Facts, KNOWN_LIMITS, THREAT_MODEL, VERIFICATION, expand_threats,
        is_numbered, is_row_id, limit_ids, p14_limit_ids, plan_item_ids, threat_ids,
        verification_row_ids,
    };
    use crate::repository::fixture::FixtureRepository;

    fn words(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|token| (*token).to_owned()).collect()
    }

    #[test]
    fn threat_ranges_and_single_threats_expand() {
        let found = expand_threats(&words(&["SEC-01..SEC-03", "SEC-07"]));
        assert_eq!(
            found.map(|set| set.into_iter().collect::<Vec<_>>()),
            Ok(words(&["SEC-01", "SEC-02", "SEC-03", "SEC-07"]))
        );
        for bad in [
            "SEC-1",
            "THREAT-01",
            "SEC-01..",
            "SEC-01..SEC-x",
            "..SEC-03",
        ] {
            assert!(expand_threats(&words(&[bad])).is_err(), "{bad}");
        }
    }

    const DELIVERY: &str = r#"{"requirements":[{"id":"R-01"},{"id":"R-02"}],
        "packets":[{"id":"P13","threats":["SEC-12"]},{"id":"P14","threats":["SEC-01..SEC-02"]}]}"#;

    fn documents() -> FixtureRepository {
        FixtureRepository::new()
            .with("plan.md", "| RQ-01 | one |\n| RQ-02 | two |\n")
            .with(DELIVERY_LEDGER, DELIVERY)
            .with(
                THREAT_MODEL,
                "| SEC-01 P0 | a |\n| SEC-02 P0 | b |\n| SEC-26 P0 | an R1 threat |\n",
            )
            .with(VERIFICATION, "| C-01 | case |\n- SEC-T01: fixture\n")
            .with(
                KNOWN_LIMITS,
                "| [L-004](#l-004) | t | security | medium | P14 | none | open |\n### L-004\n",
            )
    }

    #[test]
    fn facts_come_from_the_documents_that_own_them() -> Result<(), Vec<String>> {
        let facts = Facts::read(&documents(), "plan.md")?;
        assert_eq!(facts.plan_items, ["RQ-01", "RQ-02"]);
        assert_eq!(facts.requirements.len(), 2);
        // The R1 threat SEC-26 is in the threat model but not mapped to P14.
        let threats: Vec<&str> = facts.threats.iter().map(String::as_str).collect();
        assert_eq!(threats, ["SEC-01", "SEC-02"]);
        assert!(facts.verification_rows.contains("SEC-T01"));
        assert!(facts.limits.contains("L-004") && facts.p14_limits.contains("L-004"));
        Ok(())
    }

    #[test]
    fn an_unreadable_document_is_reported() {
        let found = Facts::read(&FixtureRepository::new(), "plan.md");
        let messages = found.err().unwrap_or_default();
        assert_eq!(messages.len(), 5, "{messages:#?}");
    }

    #[test]
    fn a_document_without_the_identifiers_it_owns_is_reported() {
        let repository = documents().with(THREAT_MODEL, "no table here\n");
        let messages = Facts::read(&repository, "plan.md")
            .err()
            .unwrap_or_default();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("holds none of the identifiers it owns")),
            "{messages:#?}"
        );
    }

    #[test]
    fn a_threat_mapped_to_p14_must_exist_in_the_threat_model() {
        let repository = documents().with(THREAT_MODEL, "| SEC-01 P0 | a |\n");
        let messages = Facts::read(&repository, "plan.md")
            .err()
            .unwrap_or_default();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("maps P14 to SEC-02")),
            "{messages:#?}"
        );
    }

    #[test]
    fn plan_items_are_the_rq_rows_of_a_table() {
        let text = "| ID | Evidence |\n| --- | --- |\n| RQ-01 | Install |\n| RQ-02 | Archive |\n\
                    | R-01 | RQ-05 mentioned in a later cell |\n| RQ-01 | duplicate row |\nRQ-09 in prose\n";
        assert_eq!(plan_item_ids(text), ["RQ-01", "RQ-02"]);
    }

    #[test]
    fn threats_are_the_sec_rows_with_their_priority() {
        let text = "| SEC-01 P0 | Shell injection | No shell | P-01 |\n| SEC-25 P0 | Leak |\n\
                    (SEC-03) in prose\n| SEC-T01 | not a threat row |\n";
        let found: Vec<String> = threat_ids(text).into_iter().collect();
        assert_eq!(found, ["SEC-01", "SEC-25"]);
    }

    #[test]
    fn verification_rows_are_table_cells_and_bullets() {
        let text = "| ID | Cases |\n| --- | --- |\n| C-01 | help |\n| A-10 | cold agent |\n\
                    | RQ-03 | offline |\n- SEC-T01: isolated fixture\n- R-SEC03: scan results\n\
                    - Not an id: text\nA-99 in prose\n";
        let found: Vec<String> = verification_row_ids(text).into_iter().collect();
        assert_eq!(found, ["A-10", "C-01", "R-SEC03", "RQ-03", "SEC-T01"]);
    }

    #[test]
    fn limits_are_the_register_headings() {
        let text = "## Summary\n### L-004\ntext\n### L-100\n### Review workflow\n### L-5\n";
        let found: Vec<String> = limit_ids(text).into_iter().collect();
        assert_eq!(found, ["L-004", "L-100"]);
    }

    #[test]
    fn p14_limits_are_read_from_the_owner_column() {
        let text = "\
| [L-004](#l-004) | Not sandboxed | security | medium | P14 | [#17](u) | deferred |
| [L-008](#l-008) | Durability | integrity | medium | P11, P14 | [#14](u), [#17](u) | accepted residual |
| [L-068](#l-068) | SEC-T01 | security | high | maintainer discussion, before P14 | [#188](u) | deferred |
| [L-001](#l-001) | A title | security | low | unscheduled | none | accepted residual |
| [L-015](#l-015) | Title with P14 in it | contract | low | unscheduled | [#172](u) | open |
| [L-110](#l-110) | A title with | a pipe | process | low | P14 | none | open |
";
        let found: Vec<String> = p14_limit_ids(text).into_iter().collect();
        assert_eq!(found, ["L-004", "L-008", "L-068", "L-110"]);
    }

    #[test]
    fn identifier_shapes() {
        assert!(is_numbered("RQ-01", "RQ-", 2));
        assert!(!is_numbered("RQ-1", "RQ-", 2));
        assert!(!is_numbered("RQ-001", "RQ-", 2));
        for good in ["C-01", "SEC-T01", "R-SEC03", "RQ-01", "A-10", "X-11"] {
            assert!(is_row_id(good), "{good}");
        }
        for bad in [
            "ID",
            "c-01",
            "C-",
            "C-0001",
            "TOOLONG-01",
            "SEC-TEXT01",
            "-01",
            "C-01a",
        ] {
            assert!(!is_row_id(bad), "{bad}");
        }
    }
}
