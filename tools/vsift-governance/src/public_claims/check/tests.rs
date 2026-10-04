//! One test per rule of the claims check. The baseline registry and its one
//! document pass; each test changes one thing.

use std::error::Error;

use serde_json::{Value, json};

use super::check_registry;
use crate::{
    public_claims::schema::ClaimsRegistry,
    release_evidence::fixture::{baseline_value, ledger},
    repository::fixture::FixtureRepository,
};

type Outcome = Result<(), Box<dyn Error>>;

const README: &str = "# VSift\n\nVSift 0.1.0 is not a stable or supported release. The managed\n\
                      install is **qualified** on Ubuntu 24.04 only.\n";

fn registry_value() -> Value {
    json!({
        "schema_version": "1",
        "current_rung": "now",
        "documents": ["README.md"],
        "unscanned_documents": [
            { "path": "docs/other.md", "owner": "P14 PR 9", "reason": "not yet worded" }
        ],
        "controlled_words": ["supported", "stable", "qualified"],
        "banned_phrases": [
            { "id": "BAN-01", "phrases": ["production ready"], "reason": "never claimed", "lifted_by": [] },
            { "id": "BAN-02", "phrases": ["strict worker"], "reason": "until its evidence", "lifted_by": ["RQ-02"] }
        ],
        "statements": [
            { "kind": "non_claim", "id": "NC-001", "documents": ["README.md"],
              "text": "not a stable or supported release", "note": "A negation." },
            { "kind": "claim", "id": "CL-001", "rung": "now", "documents": ["README.md"],
              "text": "qualified on Ubuntu 24.04 only", "basis": ["docs/planning/p13-distribution.md"],
              "note": "The P13 record." },
            { "kind": "claim", "id": "CL-002", "rung": "candidate", "documents": ["README.md"],
              "text": "is a release candidate under qualification", "requires": ["RQ-02"],
              "note": "Only once the candidate is verified." },
            { "kind": "claim", "id": "CL-003", "rung": "after_p14", "documents": ["README.md"],
              "text": "Windows 11 is supported", "requires": ["RQ-01"],
              "note": "The matrix cell." }
        ]
    })
}

fn repository(readme: &str) -> FixtureRepository {
    FixtureRepository::new()
        .with("README.md", readme)
        .with("docs/other.md", "unscanned")
        .with("docs/planning/p13-distribution.md", "record")
}

fn messages(registry: &Value, readme: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let registry: ClaimsRegistry = serde_json::from_value(registry.clone())?;
    Ok(check_registry(
        &registry,
        &ledger(&baseline_value())?,
        &repository(readme),
    ))
}

fn assert_clean(registry: &Value, readme: &str) -> Outcome {
    let found = messages(registry, readme)?;
    assert!(found.is_empty(), "{found:#?}");
    Ok(())
}

fn assert_flagged(registry: &Value, readme: &str, needle: &str) -> Outcome {
    let found = messages(registry, readme)?;
    assert!(
        found.iter().any(|message| message.contains(needle)),
        "expected a message containing {needle:?}, got {found:#?}"
    );
    Ok(())
}

/// Replaces the value at `path` of the baseline registry and expects `needle`.
fn assert_edit_flagged(path: &str, replacement: Value, needle: &str) -> Outcome {
    let mut registry = registry_value();
    *registry
        .pointer_mut(path)
        .ok_or_else(|| format!("no value at {path}"))? = replacement;
    assert_flagged(&registry, README, needle)
}

#[test]
fn the_baseline_registry_and_document_are_clean() -> Outcome {
    assert_clean(&registry_value(), README)
}

#[test]
fn unknown_fields_and_kinds_fail_the_parse() {
    let mut registry = registry_value();
    registry["statements"][0]["kind"] = json!("opinion");
    assert!(serde_json::from_value::<ClaimsRegistry>(registry).is_err());
    let mut registry = registry_value();
    registry["invented"] = json!(1);
    assert!(serde_json::from_value::<ClaimsRegistry>(registry).is_err());
    let mut registry = registry_value();
    registry["current_rung"] = json!("launched");
    assert!(serde_json::from_value::<ClaimsRegistry>(registry).is_err());
}

#[test]
fn the_schema_version_must_be_one() -> Outcome {
    assert_edit_flagged("/schema_version", json!("2"), "schema_version")
}

#[test]
fn scanned_documents_must_exist_and_be_listed_once() -> Outcome {
    assert_edit_flagged("/documents", json!([]), "documents must name at least one")?;
    assert_edit_flagged(
        "/documents",
        json!(["README.md", "MISSING.md"]),
        "scanned document MISSING.md does not exist",
    )?;
    assert_edit_flagged(
        "/documents",
        json!(["README.md", "README.md"]),
        "scanned document README.md is listed twice",
    )
}

#[test]
fn unscanned_documents_exist_and_are_not_also_scanned() -> Outcome {
    assert_edit_flagged(
        "/unscanned_documents/0/path",
        json!("docs/missing.md"),
        "unscanned document docs/missing.md does not exist",
    )?;
    assert_edit_flagged(
        "/unscanned_documents/0/path",
        json!("README.md"),
        "README.md is both scanned and listed as unscanned",
    )?;
    assert_edit_flagged(
        "/unscanned_documents/0/owner",
        json!(" "),
        "needs an owner and a reason",
    )
}

#[test]
fn a_controlled_word_outside_a_registered_statement_is_flagged() -> Outcome {
    assert_flagged(
        &registry_value(),
        &format!("{README}The command line is stable.\n"),
        "\"stable\" is used outside a registered statement",
    )?;
    assert_flagged(
        &registry_value(),
        &format!("{README}Windows is supported.\n"),
        "\"supported\" is used outside a registered statement",
    )
}

#[test]
fn a_controlled_word_inside_a_wrapped_marked_up_statement_is_covered() -> Outcome {
    let readme = "VSift is **not a\n> stable** or [supported](#x) release. The managed install is \
                  qualified\non Ubuntu 24.04-only.\n";
    assert_clean(&registry_value(), readme)
}

#[test]
fn the_controlled_words_are_single_words() -> Outcome {
    assert_edit_flagged(
        "/controlled_words",
        json!([]),
        "controlled_words must not be empty",
    )?;
    assert_edit_flagged(
        "/controlled_words",
        json!(["supported", "two words"]),
        "controlled word \"two words\" must be a single word",
    )
}

#[test]
fn a_banned_phrase_is_flagged_unless_a_non_claim_excuses_it() -> Outcome {
    let flagged = format!("{README}VSift is production-ready.\n");
    assert_flagged(
        &registry_value(),
        &flagged,
        "banned phrase \"production ready\" (BAN-01)",
    )?;

    let mut registry = registry_value();
    registry["statements"]
        .as_array_mut()
        .ok_or("no statements")?
        .push(json!({
            "kind": "non_claim", "id": "NC-002", "documents": ["README.md"],
            "text": "is not production ready", "note": "A negation."
        }));
    assert_clean(
        &registry,
        &format!("{README}VSift is not production-ready.\n"),
    )
}

#[test]
fn a_claim_cannot_excuse_a_banned_phrase() -> Outcome {
    let mut registry = registry_value();
    registry["statements"]
        .as_array_mut()
        .ok_or("no statements")?
        .push(json!({
            "kind": "claim", "id": "CL-004", "rung": "now", "documents": ["README.md"],
            "text": "is production ready", "basis": ["docs/planning/p13-distribution.md"],
            "note": "A claim that tries to excuse a banned phrase."
        }));
    assert_flagged(
        &registry,
        &format!("{README}VSift is production ready.\n"),
        "banned phrase \"production ready\"",
    )
}

#[test]
fn a_ban_is_lifted_only_by_passed_evidence() -> Outcome {
    // BAN-02 is lifted by RQ-02, which is passed in the baseline ledger.
    assert_clean(
        &registry_value(),
        &format!("{README}The strict worker is on.\n"),
    )?;
    // RQ-01 is planned, so a ban it lifts stays.
    let mut registry = registry_value();
    registry["banned_phrases"][1]["lifted_by"] = json!(["RQ-01"]);
    assert_flagged(
        &registry,
        &format!("{README}The strict worker is on.\n"),
        "banned phrase \"strict worker\" (BAN-02)",
    )?;
    // A ban nothing lifts is never lifted.
    registry["banned_phrases"][1]["lifted_by"] = json!([]);
    assert_flagged(
        &registry,
        &format!("{README}The strict worker is on.\n"),
        "banned phrase \"strict worker\" (BAN-02)",
    )
}

#[test]
fn banned_phrase_groups_are_well_formed() -> Outcome {
    assert_edit_flagged(
        "/banned_phrases/0/lifted_by",
        json!(["RQ-99"]),
        "BAN-01 is lifted by RQ-99, which is not an evidence item",
    )?;
    assert_edit_flagged(
        "/banned_phrases/0/phrases",
        json!([]),
        "BAN-01 needs non-empty phrases",
    )?;
    assert_edit_flagged(
        "/banned_phrases/0/reason",
        json!(""),
        "BAN-01 needs a reason",
    )?;
    assert_edit_flagged(
        "/banned_phrases/1/id",
        json!("BAN-01"),
        "BAN-01 appears twice",
    )
}

#[test]
fn statements_are_well_formed() -> Outcome {
    assert_edit_flagged(
        "/statements/1/id",
        json!("NC-001"),
        "statement NC-001 appears twice",
    )?;
    assert_edit_flagged(
        "/statements/0/text",
        json!("supported"),
        "a fragment of at least two words",
    )?;
    assert_edit_flagged(
        "/statements/0/documents",
        json!([]),
        "NC-001: documents must not be empty",
    )?;
    assert_edit_flagged(
        "/statements/0/documents",
        json!(["docs/other.md"]),
        "NC-001: docs/other.md is not a scanned document",
    )
}

#[test]
fn a_claim_names_its_evidence() -> Outcome {
    assert_edit_flagged(
        "/statements/2/requires",
        json!(["RQ-99"]),
        "CL-002: requires RQ-99, which is not an evidence item",
    )?;
    assert_edit_flagged(
        "/statements/1/basis",
        json!(["docs/planning/missing.md"]),
        "CL-001: basis docs/planning/missing.md does not exist",
    )?;
    assert_edit_flagged(
        "/statements/1/basis",
        json!([]),
        "CL-001: a claim names the evidence items it requires or the record it rests on",
    )?;
    let mut registry = registry_value();
    registry["statements"][2]["requires"] = json!([]);
    registry["statements"][2]["basis"] = json!(["docs/planning/p13-distribution.md"]);
    assert_flagged(
        &registry,
        README,
        "CL-002: a claim above the now rung requires at least one evidence item",
    )
}

#[test]
fn every_statement_carries_a_note() -> Outcome {
    assert_edit_flagged(
        "/statements/1/note",
        json!("  "),
        "CL-001: a statement needs a note",
    )?;
    assert_edit_flagged(
        "/statements/0/note",
        json!(""),
        "NC-001: a statement needs a note",
    )
}

#[test]
fn a_non_claim_must_excuse_something() -> Outcome {
    assert_edit_flagged(
        "/statements/0/text",
        json!("a statement with no controlled word"),
        "NC-001: a non-claim excuses a controlled word or banned phrase",
    )
}

#[test]
fn a_statement_may_appear_only_in_the_documents_it_is_registered_for() -> Outcome {
    let mut registry = registry_value();
    registry["documents"] = json!(["README.md", "SECURITY.md"]);
    let repository = repository(README).with(
        "SECURITY.md",
        "VSift is not a stable or supported release.\n",
    );
    let registry: ClaimsRegistry = serde_json::from_value(registry)?;
    let found = check_registry(&registry, &ledger(&baseline_value())?, &repository);
    assert!(
        found.iter().any(|message| message.contains(
            "SECURITY.md: statement NC-001 is used here but registered only for README.md"
        )),
        "{found:#?}"
    );
    Ok(())
}

#[test]
fn a_claim_above_the_current_rung_may_not_be_used() -> Outcome {
    let readme = format!("{README}VSift 0.2.0-rc.1 is a release candidate under qualification.\n");
    assert_flagged(
        &registry_value(),
        &readme,
        "CL-002 is a candidate claim in use while the current rung is now",
    )?;
    let mut registry = registry_value();
    registry["current_rung"] = json!("candidate");
    assert_clean(&registry, &readme)
}

#[test]
fn a_claim_in_use_needs_its_evidence_passed() -> Outcome {
    // Every claim at or below the current rung is in use, or it would be stale.
    let readme = format!(
        "{README}VSift is a release candidate under qualification. Windows 11 is supported.\n"
    );
    let mut registry = registry_value();
    registry["current_rung"] = json!("after_p14");
    // CL-003 requires RQ-01, which is planned in the baseline ledger.
    assert_flagged(
        &registry,
        &readme,
        "CL-003 is in use but requires RQ-01 to be passed; it is planned",
    )?;
    registry["statements"][3]["requires"] = json!(["RQ-02"]);
    assert_clean(&registry, &readme)
}

#[test]
fn a_registered_statement_that_no_document_uses_is_stale() -> Outcome {
    // NC-001 is gone from the document, and CL-001 (rung now) with it.
    let found = messages(&registry_value(), "# VSift\n\nNothing to see.\n")?;
    assert!(
        found
            .iter()
            .any(|message| message.starts_with("NC-001 is registered but appears in none")),
        "{found:#?}"
    );
    assert!(
        found
            .iter()
            .any(|message| message.starts_with("CL-001 is registered but appears in none")),
        "{found:#?}"
    );
    assert!(
        !found
            .iter()
            .any(|message| message.starts_with("CL-002") || message.starts_with("CL-003")),
        "claims above the current rung may wait: {found:#?}"
    );
    Ok(())
}

#[test]
fn the_words_of_a_graphic_are_held_to_the_same_rules_as_a_document() -> Outcome {
    let mut registry = registry_value();
    registry["documents"] = json!(["README.md", "docs/assets/roadmap.svg"]);
    let registry: ClaimsRegistry = serde_json::from_value(registry)?;
    let graphic = |body: &str| {
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><style>.stable{{fill:red}}</style>\
             <rect fill=\"supported\"/><text x=\"1\">{body}</text></svg>"
        )
    };
    let found = |body: &str| -> Result<Vec<String>, Box<dyn Error>> {
        let repository = repository(README).with("docs/assets/roadmap.svg", &graphic(body));
        Ok(check_registry(
            &registry,
            &ledger(&baseline_value())?,
            &repository,
        ))
    };
    let flagged = found("Windows 11 is supported")?;
    assert!(
        flagged.iter().any(|message| message.contains(
            "docs/assets/roadmap.svg: \"supported\" is used outside a registered statement"
        )),
        "{flagged:#?}"
    );
    let banned = found("VSift is production ready")?;
    assert!(
        banned.iter().any(|message| message
            .contains("docs/assets/roadmap.svg: banned phrase \"production ready\"")),
        "{banned:#?}"
    );
    // A style rule, an attribute and the roadmap's own plain words pass.
    let clean = found("Done, now and next")?;
    assert!(clean.is_empty(), "{clean:#?}");
    Ok(())
}

const LIMITS_REGISTER: &str = "# Known limits\n\n## Security\n\n\
### L-004\n\n- **Owner:** P14. **Review:** pending.\n\n\
### L-007\n\n- **Owner:** x. **Status:** accepted residual.\n  **Review:**\n  accepted (2026-10-05).\n\n\
### L-008\n\n- **Review:** rejected (2026-10-05): fix it.\n\n\
### L-009\n\n- **Review:** rescheduled to R1 (2026-10-05).\n";

/// The messages for a registry whose repository also holds the known-limits register.
fn found_with_register(registry: &Value, readme: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let registry: ClaimsRegistry = serde_json::from_value(registry.clone())?;
    let repository = repository(readme).with("docs/planning/known-limits.md", LIMITS_REGISTER);
    Ok(check_registry(
        &registry,
        &ledger(&baseline_value())?,
        &repository,
    ))
}

/// The baseline registry at the last rung, with CL-003 (the matrix cell) leaning on `limits`.
fn leaning_registry(limits: &Value) -> Value {
    let mut registry = registry_value();
    registry["current_rung"] = json!("after_p14");
    // RQ-02 is passed in the baseline ledger, so the evidence is not what fails.
    registry["statements"][3]["requires"] = json!(["RQ-02"]);
    registry["statements"][3]["limits"] = limits.clone();
    registry
}

const IN_USE: &str = "VSift is not a stable or supported release. The managed install is \
                      **qualified** on Ubuntu 24.04 only. VSift is a release candidate under \
                      qualification. Windows 11 is supported.\n";

#[test]
fn a_claim_above_now_in_use_waits_for_the_review_of_the_limits_it_leans_on() -> Outcome {
    let pending = found_with_register(&leaning_registry(&json!(["L-004"])), IN_USE)?;
    assert!(
        pending.iter().any(|message| message
            .contains("CL-003 is in use but leans on L-004, whose review is pending")),
        "{pending:#?}"
    );
    let rejected = found_with_register(&leaning_registry(&json!(["L-008"])), IN_USE)?;
    assert!(
        rejected.iter().any(|message| message.contains(
            "CL-003 is in use but leans on L-008, whose review is rejected (the limit must be fixed)"
        )),
        "{rejected:#?}"
    );
    // One pending limit among reviewed ones is still named alone.
    let mixed = found_with_register(
        &leaning_registry(&json!(["L-007", "L-004", "L-009"])),
        IN_USE,
    )?;
    assert_eq!(
        mixed
            .iter()
            .filter(|message| message.contains("leans on"))
            .count(),
        1,
        "{mixed:#?}"
    );
    Ok(())
}

#[test]
fn an_accepted_or_rescheduled_review_lets_the_claim_lean_even_when_it_wraps() -> Outcome {
    // L-007's review wraps onto the next line; L-009 is rescheduled.
    let found = found_with_register(&leaning_registry(&json!(["L-007", "L-009"])), IN_USE)?;
    assert!(found.is_empty(), "{found:#?}");
    Ok(())
}

#[test]
fn a_claim_that_is_not_in_use_may_lean_on_a_pending_limit() -> Outcome {
    // CL-003 is above the current rung (now) and not used: it waits, whatever it leans on.
    let mut registry = registry_value();
    registry["statements"][3]["limits"] = json!(["L-004"]);
    let found = found_with_register(&registry, README)?;
    assert!(found.is_empty(), "{found:#?}");
    Ok(())
}

#[test]
fn the_pre_release_wording_of_the_now_rung_is_not_held_by_the_review() -> Outcome {
    // CL-001 is a `now` claim in use; the review sheet came after it was published.
    let mut registry = registry_value();
    registry["statements"][1]["limits"] = json!(["L-004", "L-008"]);
    let found = found_with_register(&registry, README)?;
    assert!(found.is_empty(), "{found:#?}");
    Ok(())
}

#[test]
fn the_limits_a_claim_lists_are_real_entries_listed_once() -> Outcome {
    let missing = found_with_register(&leaning_registry(&json!(["L-999"])), IN_USE)?;
    assert!(
        missing.iter().any(|message| message
            .contains("CL-003: leans on L-999, which the known-limits register does not hold")),
        "{missing:#?}"
    );
    let malformed = found_with_register(&leaning_registry(&json!(["L-4x"])), IN_USE)?;
    assert!(
        malformed
            .iter()
            .any(|message| message
                .contains("CL-003: limit \"L-4x\" is not a register entry identifier")),
        "{malformed:#?}"
    );
    let repeated = found_with_register(&leaning_registry(&json!(["L-007", "L-007"])), IN_USE)?;
    assert!(
        repeated
            .iter()
            .any(|message| message.contains("CL-003: limit L-007 is listed twice")),
        "{repeated:#?}"
    );
    // Without the register file the check says so instead of passing quietly.
    let found = messages(&leaning_registry(&json!(["L-007"])), IN_USE)?;
    assert!(
        found.iter().any(|message| message.contains(
            "statements list the limits they lean on, but the register could not be read"
        )),
        "{found:#?}"
    );
    Ok(())
}
