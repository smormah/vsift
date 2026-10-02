//! A small valid ledger and the repository it is checked against, for the
//! rule tests. Each test changes one field of the baseline and expects one
//! rule to fire.

use std::error::Error;

use serde_json::{Value, json};

use super::{facts::Facts, schema::EvidenceLedger};
use crate::repository::fixture::FixtureRepository;

/// Commit the baseline's passed item is recorded at.
pub(crate) const RECORDED_COMMIT: &str = "1111111111111111111111111111111111111111";
/// Another commit, for targets and deltas.
pub(crate) const LATER_COMMIT: &str = "2222222222222222222222222222222222222222";

/// A valid ledger: one planned item with earlier material, one passed item
/// carried by scope, and the completeness item.
pub(crate) fn baseline_value() -> Value {
    json!({
        "schema_version": "1",
        "packet": "P14",
        "plan": "docs/planning/p14-qualification.md",
        "release_delta": null,
        "items": [
            {
                "id": "RQ-01",
                "title": "Clean install",
                "supports": {
                    "requirements": ["R-01"],
                    "threats": ["SEC-01"],
                    "verification": ["R-SEC03"],
                    "limits": ["L-004"]
                },
                "proves": "The packages install.",
                "does_not_prove": "A clean machine.",
                "producer": { "kind": "workflow", "p14_pr": 2, "description": "A hosted workflow." },
                "scope": ["crates", "npm"],
                "gate": { "candidate": "required", "stable": "repeat" },
                "status": "planned",
                "prior": [
                    {
                        "reference": { "type": "record", "path": "docs/planning/p13-distribution.md" },
                        "date": "2026-10-01",
                        "note": "One install on one machine."
                    }
                ],
                "date": "2026-10-02"
            },
            {
                "id": "RQ-02",
                "title": "Archive",
                "supports": { "requirements": ["R-01"] },
                "proves": "The archive runs.",
                "does_not_prove": "Browser prompts.",
                "producer": { "kind": "workflow", "p14_pr": 2, "description": "A hosted workflow." },
                "scope": ["crates"],
                "gate": { "candidate": "required", "stable": "carry" },
                "status": "passed",
                "applies_to": { "version": "0.2.0-rc.1", "commit": RECORDED_COMMIT },
                "evidence": [
                    {
                        "reference": { "type": "workflow_run", "workflow": "Hosted", "run_id": 36_931_487_439_u64 },
                        "date": "2026-10-05",
                        "note": "All jobs green."
                    }
                ],
                "date": "2026-10-05"
            },
            {
                "id": "RQ-20",
                "title": "Completeness",
                "supports": {},
                "proves": "Nothing was forgotten.",
                "does_not_prove": "That each entry is correct.",
                "producer": { "kind": "governance_check", "p14_pr": 1, "description": "This check." },
                "scope": ["."],
                "gate": { "candidate": "not_required", "stable": "not_required" },
                "status": "planned",
                "date": "2026-10-02"
            }
        ]
    })
}

/// The identifiers the baseline refers to.
pub(crate) fn facts() -> Facts {
    let set = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect();
    Facts {
        plan_items: ["RQ-01", "RQ-02", "RQ-20"]
            .iter()
            .map(|id| (*id).to_owned())
            .collect(),
        requirements: set(&["R-01"]),
        threats: set(&["SEC-01"]),
        verification_rows: set(&["RQ-01", "RQ-02", "RQ-20", "R-SEC03", "A-10"]),
        limits: set(&["L-004", "L-008"]),
        p14_limits: set(&["L-004"]),
    }
}

/// The files the baseline's paths name.
pub(crate) fn repository() -> FixtureRepository {
    FixtureRepository::new()
        .with("crates/vsift/src/lib.rs", "")
        .with("npm/vsift-cli/package.json", "{}")
        .with("docs/planning/p13-distribution.md", "record")
        .with("docs/planning/p14-qualification.md", "plan")
}

/// Parses a ledger value.
pub(crate) fn ledger(value: &Value) -> Result<EvidenceLedger, Box<dyn Error>> {
    Ok(serde_json::from_value(value.clone())?)
}

/// Replaces the value at the JSON pointer `path`.
pub(crate) fn set(value: &mut Value, path: &str, replacement: Value) -> Result<(), Box<dyn Error>> {
    let slot = value
        .pointer_mut(path)
        .ok_or_else(|| format!("the baseline has nothing at {path}"))?;
    *slot = replacement;
    Ok(())
}

/// Inserts `replacement` under `key` of the object at `path`.
pub(crate) fn insert(
    value: &mut Value,
    path: &str,
    key: &str,
    replacement: Value,
) -> Result<(), Box<dyn Error>> {
    let object = value
        .pointer_mut(path)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| format!("the baseline has no object at {path}"))?;
    object.insert(key.to_owned(), replacement);
    Ok(())
}
