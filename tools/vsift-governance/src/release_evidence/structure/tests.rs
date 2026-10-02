//! One test per structure rule: the baseline passes, and a single edit of it
//! makes exactly the rule under test fire.

use std::error::Error;

use serde_json::{Value, json};

use super::check_structure;
use crate::release_evidence::fixture::{
    LATER_COMMIT, RECORDED_COMMIT, baseline_value, facts, insert, ledger, repository, set,
};

type Outcome = Result<(), Box<dyn Error>>;

/// The messages the structure rules give for `value`.
fn messages(value: &Value) -> Result<Vec<String>, Box<dyn Error>> {
    Ok(check_structure(&ledger(value)?, &facts(), &repository()))
}

/// Fails unless some message contains `needle`.
fn assert_flagged(value: &Value, needle: &str) -> Outcome {
    let found = messages(value)?;
    assert!(
        found.iter().any(|message| message.contains(needle)),
        "expected a message containing {needle:?}, got {found:#?}"
    );
    Ok(())
}

/// Fails unless the baseline, after `edit`, is flagged with `needle`.
fn assert_edit_flagged(edit: impl FnOnce(&mut Value) -> Outcome, needle: &str) -> Outcome {
    let mut value = baseline_value();
    edit(&mut value)?;
    assert_flagged(&value, needle)
}

/// Fails if the structure rules give any message for `value`.
fn assert_clean(value: &Value) -> Outcome {
    let found = messages(value)?;
    assert!(found.is_empty(), "{found:#?}");
    Ok(())
}

#[test]
fn the_baseline_ledger_is_clean() -> Outcome {
    assert_clean(&baseline_value())
}

/// The release tool's example `release_delta` record passes the structure rules.
#[test]
fn the_release_tools_delta_record_is_structurally_valid() -> Outcome {
    let example = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../vsift-release/tests/release-delta.example.json"),
    )?;
    let mut value = baseline_value();
    set(
        &mut value,
        "/release_delta",
        serde_json::from_str(&example)?,
    )?;
    assert_clean(&value)
}
#[test]
fn unknown_fields_and_values_fail_the_parse() -> Outcome {
    for (path, replacement, what) in [
        (
            "/items/0/status",
            json!("done"),
            "a status outside the closed set",
        ),
        ("/items/1/applies_to/commit", json!("abc"), "a short commit"),
        ("/items/0/date", json!("2026-13-40"), "an impossible date"),
        (
            "/items/1/evidence/0/reference/type",
            json!("web_page"),
            "a link type outside the closed set",
        ),
        (
            "/items/1/applies_to/version",
            json!("0.2.0-beta"),
            "a version suffix",
        ),
    ] {
        let mut value = baseline_value();
        set(&mut value, path, replacement)?;
        assert!(ledger(&value).is_err(), "{what}");
    }
    let mut value = baseline_value();
    insert(&mut value, "/items/0", "invented", json!(1))?;
    assert!(ledger(&value).is_err(), "an unknown field");
    let mut value = baseline_value();
    insert(
        &mut value,
        "/items/1/evidence/0/reference",
        "extra",
        json!(1),
    )?;
    assert!(ledger(&value).is_err(), "an unknown field of a link");
    Ok(())
}

#[test]
fn the_header_must_be_version_one_of_p14() -> Outcome {
    assert_edit_flagged(|v| set(v, "/schema_version", json!("2")), "schema_version")?;
    assert_edit_flagged(|v| set(v, "/packet", json!("P13")), "packet must be")
}

#[test]
fn every_item_the_plan_defines_must_be_present() -> Outcome {
    assert_edit_flagged(
        |v| {
            v.pointer_mut("/items")
                .and_then(Value::as_array_mut)
                .ok_or("no items")?
                .remove(1);
            Ok(())
        },
        "RQ-02 is defined by the plan but missing",
    )
}

#[test]
fn an_item_the_plan_does_not_define_is_refused() -> Outcome {
    assert_edit_flagged(
        |v| set(v, "/items/1/id", json!("RQ-99")),
        "RQ-99 is not defined",
    )
}

#[test]
fn an_item_may_appear_only_once() -> Outcome {
    assert_edit_flagged(
        |v| set(v, "/items/1/id", json!("RQ-01")),
        "RQ-01 appears more than once",
    )
}

#[test]
fn items_keep_the_plans_order() -> Outcome {
    assert_edit_flagged(
        |v| {
            v.pointer_mut("/items")
                .and_then(Value::as_array_mut)
                .ok_or("no items")?
                .swap(0, 1);
            Ok(())
        },
        "not in the plan's order",
    )
}

#[test]
fn descriptive_text_must_not_be_empty() -> Outcome {
    assert_edit_flagged(
        |v| set(v, "/items/0/proves", json!("  ")),
        "proves must not be empty",
    )?;
    assert_edit_flagged(
        |v| set(v, "/items/0/does_not_prove", json!("")),
        "does_not_prove must not be empty",
    )?;
    assert_edit_flagged(
        |v| set(v, "/items/0/producer/description", json!("")),
        "producer.description must not be empty",
    )
}

#[test]
fn a_producer_pull_request_must_exist_in_the_plan() -> Outcome {
    assert_edit_flagged(
        |v| set(v, "/items/0/producer/p14_pr", json!(14)),
        "p14_pr 14",
    )
}

#[test]
fn supported_identifiers_must_exist_in_their_documents() -> Outcome {
    assert_edit_flagged(
        |v| set(v, "/items/0/supports/requirements", json!(["R-99"])),
        "unknown requirement R-99",
    )?;
    assert_edit_flagged(
        |v| set(v, "/items/0/supports/threats", json!(["SEC-99"])),
        "unknown threat SEC-99",
    )?;
    assert_edit_flagged(
        |v| set(v, "/items/0/supports/verification", json!(["Z-01"])),
        "unknown verification row Z-01",
    )?;
    assert_edit_flagged(
        |v| set(v, "/items/0/supports/limits", json!(["L-999"])),
        "unknown limit L-999",
    )
}

#[test]
fn a_supported_identifier_may_be_listed_once() -> Outcome {
    assert_edit_flagged(
        |v| set(v, "/items/0/supports/threats", json!(["SEC-01", "SEC-01"])),
        "threat SEC-01 is listed twice",
    )
}

#[test]
fn an_item_must_support_something_except_the_completeness_check() -> Outcome {
    assert_edit_flagged(
        |v| set(v, "/items/1/supports", json!({})),
        "RQ-02: supports no requirement",
    )?;
    let found = messages(&baseline_value())?;
    assert!(
        !found.iter().any(|message| message.starts_with("RQ-20")),
        "{found:#?}"
    );
    Ok(())
}

#[test]
fn every_item_needs_its_row_in_the_verification_plan() -> Outcome {
    let mut without_row = facts();
    assert!(without_row.verification_rows.remove("RQ-01"));
    let found = check_structure(&ledger(&baseline_value())?, &without_row, &repository());
    assert!(
        found
            .iter()
            .any(|message| message.contains("RQ-01: verification.md has no row")),
        "{found:#?}"
    );
    Ok(())
}

#[test]
fn a_scope_names_existing_paths_inside_the_repository() -> Outcome {
    assert_edit_flagged(
        |v| set(v, "/items/0/scope", json!([])),
        "scope must name at least one",
    )?;
    assert_edit_flagged(
        |v| set(v, "/items/0/scope", json!(["../outside"])),
        "not repository-relative",
    )?;
    assert_edit_flagged(
        |v| set(v, "/items/0/scope", json!(["missing/dir"])),
        "scope path missing/dir does not exist",
    )?;
    assert_edit_flagged(
        |v| set(v, "/items/0/scope", json!(["crates", "crates"])),
        "scope path crates is listed twice",
    )
}

#[test]
fn a_stable_release_cannot_need_what_the_candidate_does_not() -> Outcome {
    assert_edit_flagged(
        |v| set(v, "/items/2/gate/stable", json!("repeat")),
        "cannot be needed by the stable release",
    )
}

#[test]
fn a_counted_status_needs_a_subject_and_evidence() -> Outcome {
    assert_edit_flagged(
        |v| set(v, "/items/1/applies_to", Value::Null),
        "applies_to must be given exactly when",
    )?;
    assert_edit_flagged(
        |v| set(v, "/items/1/evidence", json!([])),
        "a passed item needs at least one evidence link",
    )?;
    assert_edit_flagged(
        |v| {
            insert(
                v,
                "/items/0",
                "applies_to",
                json!({ "version": "0.2.0-rc.1", "commit": RECORDED_COMMIT }),
            )
        },
        "applies_to must be given exactly when",
    )
}

#[test]
fn a_planned_item_counts_no_evidence() -> Outcome {
    assert_edit_flagged(
        |v| {
            let link = json!([{
                "reference": { "type": "pull_request", "number": 250 },
                "date": "2026-10-02",
                "note": "A pull request."
            }]);
            insert(v, "/items/0", "evidence", link)
        },
        "a planned item has no counted evidence",
    )
}

#[test]
fn an_issue_alone_is_not_evidence_of_a_pass() -> Outcome {
    assert_edit_flagged(
        |v| {
            set(
                v,
                "/items/1/evidence/0/reference",
                json!({ "type": "issue", "number": 17 }),
            )
        },
        "an issue alone is not evidence of a pass",
    )
}

#[test]
fn a_passed_workflow_item_links_a_run_of_the_workflow() -> Outcome {
    assert_edit_flagged(
        |v| {
            set(
                v,
                "/items/1/evidence/0/reference",
                json!({ "type": "pull_request", "number": 250 }),
            )
        },
        "a passed item produced by a workflow links at least one run",
    )?;
    // A procedure's item may rest on a record instead.
    let mut value = baseline_value();
    set(&mut value, "/items/1/producer/kind", json!("procedure"))?;
    set(
        &mut value,
        "/items/1/evidence/0/reference",
        json!({ "type": "record", "path": "docs/planning/p13-distribution.md" }),
    )?;
    assert_clean(&value)
}

#[test]
fn evidence_and_decisions_are_not_newer_than_the_item() -> Outcome {
    assert_edit_flagged(
        |v| set(v, "/items/1/date", json!("2026-10-04")),
        "evidence dated 2026-10-05 is newer than the item's date 2026-10-04",
    )?;
    assert_edit_flagged(
        |v| {
            set(v, "/items/0/status", json!("waived"))?;
            insert(
                v,
                "/items/0",
                "decision",
                json!({
                    "statement": "Ship without it.",
                    "reference": { "type": "issue", "number": 17 },
                    "date": "2026-10-03"
                }),
            )
        },
        "the decision dated 2026-10-03 is newer than the item's date 2026-10-02",
    )
}

#[test]
fn a_failed_item_names_its_issue() -> Outcome {
    let mut value = baseline_value();
    set(&mut value, "/items/1/status", json!("failed"))?;
    assert_flagged(&value, "a failed item must name the tracked issue")?;
    insert(&mut value, "/items/1", "issues", json!([251]))?;
    assert_clean(&value)
}

#[test]
fn an_issue_number_is_positive() -> Outcome {
    assert_edit_flagged(
        |v| insert(v, "/items/0", "issues", json!([0])),
        "issue number 0",
    )
}

#[test]
fn a_waiver_names_the_maintainers_decision() -> Outcome {
    let decision = json!({
        "statement": "Ship without the Smart App Control try-out, documented as untried.",
        "reference": { "type": "record", "path": "docs/planning/p14-qualification.md" },
        "date": "2026-10-02"
    });
    let mut value = baseline_value();
    set(&mut value, "/items/0/status", json!("waived"))?;
    assert_flagged(
        &value,
        "a decision is given exactly when the status is waived",
    )?;
    insert(&mut value, "/items/0", "decision", decision.clone())?;
    assert_clean(&value)?;

    assert_edit_flagged(
        |v| insert(v, "/items/0", "decision", decision),
        "a decision is given exactly when the status is waived",
    )?;
    assert_edit_flagged(
        |v| {
            set(v, "/items/0/status", json!("waived"))?;
            insert(
                v,
                "/items/0",
                "decision",
                json!({
                    "statement": " ",
                    "reference": { "type": "issue", "number": 17 },
                    "date": "2026-10-02"
                }),
            )
        },
        "the decision has no statement",
    )
}

#[test]
fn a_not_applicable_item_gives_its_reason() -> Outcome {
    assert_edit_flagged(
        |v| set(v, "/items/0/status", json!("not_applicable")),
        "needs a reason",
    )?;
    assert_edit_flagged(
        |v| {
            set(v, "/items/0/status", json!("not_applicable"))?;
            insert(v, "/items/0", "reason", json!(""))
        },
        "reason must not be empty",
    )?;
    assert_edit_flagged(
        |v| insert(v, "/items/0", "reason", json!("because")),
        "a reason belongs only to a waived or not_applicable item",
    )
}

#[test]
fn links_are_well_formed_and_their_records_exist() -> Outcome {
    assert_edit_flagged(
        |v| {
            set(
                v,
                "/items/0/prior/0/reference",
                json!({ "type": "record", "path": "docs/planning/missing.md" }),
            )
        },
        "prior record docs/planning/missing.md does not exist",
    )?;
    assert_edit_flagged(
        |v| {
            set(
                v,
                "/items/0/prior/0/reference",
                json!({ "type": "record", "path": "../secret.md" }),
            )
        },
        "is not repository-relative",
    )?;
    assert_edit_flagged(
        |v| {
            set(
                v,
                "/items/1/evidence/0/reference",
                json!({ "type": "workflow_run", "workflow": "Hosted", "run_id": 0 }),
            )
        },
        "needs a workflow name and a run id",
    )?;
    assert_edit_flagged(
        |v| {
            set(
                v,
                "/items/1/evidence/0/reference",
                json!({ "type": "pull_request", "number": 0 }),
            )
        },
        "link has number 0",
    )?;
    assert_edit_flagged(
        |v| set(v, "/items/1/evidence/0/note", json!("")),
        "link has no note",
    )
}

#[test]
fn every_requirement_threat_row_and_p14_limit_is_supported() -> Outcome {
    // R-01 is listed by two items: dropping it from one leaves it supported.
    let mut value = baseline_value();
    set(&mut value, "/items/0/supports/requirements", json!([]))?;
    assert_clean(&value)?;
    set(&mut value, "/items/1/supports/requirements", json!([]))?;
    insert(&mut value, "/items/1/supports", "limits", json!(["L-008"]))?;
    assert_flagged(&value, "requirement R-01 is supported by no evidence item")?;

    assert_edit_flagged(
        |v| set(v, "/items/0/supports/threats", json!([])),
        "threat SEC-01 is supported by no evidence item",
    )?;
    assert_edit_flagged(
        |v| set(v, "/items/0/supports/verification", json!([])),
        "verification row R-SEC03 is supported",
    )?;
    assert_edit_flagged(
        |v| set(v, "/items/0/supports/limits", json!(["L-008"])),
        "limit L-004 names P14 as its owner",
    )
}

#[test]
fn a_recorded_delta_names_a_candidate_and_the_stable_cut_from_it() -> Outcome {
    let delta = |candidate: &str, stable: &str| {
        json!({
            "candidate_version": candidate,
            "candidate_commit": RECORDED_COMMIT,
            "stable_version": stable,
            "stable_commit": LATER_COMMIT,
            "verdict": "allowed",
            "check": { "type": "record", "path": "docs/planning/p14-qualification.md" },
            "date": "2026-11-01"
        })
    };
    let mut value = baseline_value();
    set(&mut value, "/release_delta", delta("0.2.0-rc.2", "0.2.0"))?;
    assert_clean(&value)?;

    assert_edit_flagged(
        |v| set(v, "/release_delta", delta("0.2.0", "0.2.0")),
        "is not a release candidate",
    )?;
    assert_edit_flagged(
        |v| set(v, "/release_delta", delta("0.2.0-rc.1", "0.2.0-rc.2")),
        "is a release candidate, not a stable version",
    )?;
    assert_edit_flagged(
        |v| set(v, "/release_delta", delta("0.2.0-rc.1", "0.3.0")),
        "was not cut from",
    )?;
    assert_edit_flagged(
        |v| {
            let mut broken = delta("0.2.0-rc.1", "0.2.0");
            set(
                &mut broken,
                "/check/path",
                json!("docs/planning/missing.md"),
            )?;
            set(v, "/release_delta", broken)
        },
        "check record docs/planning/missing.md does not exist",
    )
}
