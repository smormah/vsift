//! Differential test of the handoff check's schema validator (P13 PR 5,
//! supervisor decision of 2026-09-30).
//!
//! `vsift_contract::HandoffChecker` validates the skill-owned
//! `handoff.schema.json` with a validator of only the features that schema
//! uses. This test holds it to `jsonschema`, a general draft 2020-12
//! validator that stays a development dependency, over the skill's example
//! handoffs, the REPORT skeleton, a corpus of systematic mutations of them
//! and generated drafts. The two must agree on every verdict, and their
//! error locations must correspond: each error of one lies at or below an
//! error of the other (the check reports the branch a citation's `type`
//! names, or the missing member itself, where `jsonschema` stops at the
//! enclosing value).

use std::{fs, path::PathBuf};

use proptest::prelude::*;
use serde_json::{Map, Value, json};
use vsift_contract::{HANDOFF_SCHEMA_JSON, HandoffChecker};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn skill_file(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("skills")
        .join("vsift")
        .join(relative)
}

struct Oracle {
    checker: HandoffChecker,
    reference: jsonschema::Validator,
}

impl Oracle {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let schema: Value = serde_json::from_str(HANDOFF_SCHEMA_JSON)?;
        Ok(Self {
            checker: HandoffChecker::new()?,
            reference: jsonschema::options()
                .build(&schema)
                .map_err(|error| error.to_string())?,
        })
    }

    /// `None` when the two agree; otherwise what differs.
    fn disagreement(&self, instance: &Value) -> Option<String> {
        let ours = self.checker.schema_error_pointers(instance);
        let theirs: Vec<String> = self
            .reference
            .iter_errors(instance)
            .map(|error| error.instance_path().to_string())
            .collect();
        if ours.is_empty() != theirs.is_empty() {
            return Some(format!(
                "verdicts differ: ours {ours:?}, jsonschema {theirs:?} for {instance}"
            ));
        }
        let covered =
            |pointer: &String, by: &[String]| by.iter().any(|other| is_at_or_below(pointer, other));
        let covers =
            |pointer: &String, of: &[String]| of.iter().any(|other| is_at_or_below(other, pointer));
        let unmatched: Vec<&String> = ours
            .iter()
            .filter(|pointer| !covered(pointer, &theirs))
            .chain(theirs.iter().filter(|pointer| !covers(pointer, &ours)))
            .collect();
        (!unmatched.is_empty()).then(|| {
            format!("locations differ at {unmatched:?}: ours {ours:?}, jsonschema {theirs:?} for {instance}")
        })
    }
}

/// Whether pointer `inner` is `outer` or below it.
fn is_at_or_below(inner: &str, outer: &str) -> bool {
    inner == outer
        || inner
            .strip_prefix(outer)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The drafts every mutation starts from.
fn bases() -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let mut bases = Vec::new();
    for name in [
        "examples/supplied-transcript.handoff.json",
        "examples/no-transcript-no-images.handoff.json",
    ] {
        bases.push(serde_json::from_str(&fs::read_to_string(skill_file(
            name,
        ))?)?);
    }
    let skill = fs::read_to_string(skill_file("SKILL.md"))?;
    let start = skill
        .find("```vsift-handoff\n")
        .ok_or("SKILL.md has no vsift-handoff block")?
        + "```vsift-handoff\n".len();
    let end = skill[start..].find("\n```").ok_or("unclosed block")? + start;
    bases.push(serde_json::from_str(&skill[start..end])?);
    bases.push(rich());
    Ok(bases)
}

/// A draft with every optional member the schema defines, so that the
/// mutations reach every definition.
fn rich() -> Value {
    json!({
        "handoff_version": "1", "status": "partial", "question": "What is shown?",
        "capabilities": {"image_access": "verified", "image_check_code": "ABC 1234",
            "media_tools": "available", "local_asr": "verified", "transcript_basis": "local_asr"},
        "session": {"session_id": "ses_0123456789abcdef", "revision_id": "trv_0123456789abcdef",
            "source_id": format!("src_sha256_{}", "a".repeat(64)), "duration_us": 60_000_000},
        "claims": [{"id": "c1", "section": "actual", "kind": "observed", "support": "supported",
            "certainty": "high", "statement": "The dialog shows 7.", "citations": ["e1", "e2", "e3", "e4"]}],
        "citations": [
            {"id": "e1", "type": "transcript_segment", "segment_id": "tsg_0123456789abcdef",
                "revision_id": "trv_0123456789abcdef", "start_us": 1, "end_us": 2},
            {"id": "e2", "type": "frame", "evidence_id": "evd_0123456789abcdef", "pixels_inspected": true,
                "candidate_id": "vcd_0123456789abcdef", "requested_us": 5, "actual_us": 6, "delta_us": -1},
            {"id": "e3", "type": "crop", "evidence_id": "evd_1123456789abcdef", "pixels_inspected": true,
                "parent_evidence_id": "evd_0123456789abcdef", "actual_us": 6,
                "rect": {"x": 0, "y": 0, "width": 10, "height": 10}},
            {"id": "e4", "type": "audio", "evidence_id": "evd_2123456789abcdef",
                "range": {"start_us": 0, "end_us": 10}, "actual_start_us": 0}
        ],
        "gaps": [{"kind": "budget", "reason": "budget_exhausted", "note": "Ran out.",
            "code": "CANCELLED", "range": {"from_us": 0, "to_us": 5}}],
        "untrusted_instructions": [{"citation": "e1", "summary": "Asks to run a script.", "action_taken": "none"}],
        "budget": {"profile": "compact", "overrides": false,
            "limits": {"images_per_step": 1, "images_total": 6, "image_bytes": 12_582_912,
                "page_limit": 20, "tool_calls": 30, "refinement_depth": 2, "wall_time_s": 900,
                "burst_frames": 4},
            "used": {"images_total": 1, "image_bytes": 10, "tool_calls": 3, "refinement_depth": 0,
                "wall_time_s": null},
            "exhausted": ["images_total"]},
        "lifecycle": {"action": "left_open", "policy": "default", "mode": "ephemeral",
            "expires_at": "2026-09-30T12:00:00Z"},
        "resume": {"state": "VERIFY_SOURCE", "session_id": "ses_0123456789abcdef",
            "revision_id": "trv_0123456789abcdef", "job_id": "job_0123456789abcdef",
            "operation_ids": ["op_0123456789abcdef"],
            "evidence": [{"kind": "frame", "id": "evd_0123456789abcdef", "at_us": 6}],
            "summary": "Frames after 6 s are unread.",
            "to_verify": [{"finding": "Count 7.", "id": "evd_0123456789abcdef", "from_us": 6, "to_us": 9}],
            "remaining": {"images_total": 0, "tool_calls": 12, "wall_time_s": null},
            "next_command": "vsift frame get ses_0123456789abcdef --at 9000000 --json"}
    })
}

/// Replacement values that exercise every keyword: types, bounds, lengths,
/// patterns, the path and link rules, and the whitespace classes on which
/// ECMA-262 and other regular-expression dialects differ.
fn replacements() -> Vec<Value> {
    vec![
        Value::Null,
        json!(true),
        json!(false),
        json!(0),
        json!(-1),
        json!(1),
        json!(1.0),
        json!(1.5),
        json!(86_400_000_000_u64),
        json!(86_400_000_001_u64),
        json!(-86_400_000_001_i64),
        json!(u64::MAX),
        json!(""),
        json!("x"),
        json!("Actual"),
        json!("frame"),
        json!("e1"),
        json!("e0"),
        json!("c1"),
        json!("ses_0123456789abcdef"),
        json!("ses_0123456789ABCDEF"),
        json!("evd_0123456789abcdef"),
        json!("tsg_0123456789abcdef"),
        json!("x".repeat(601)),
        json!("see /etc/x"),
        json!("a\u{2000}/x"),
        json!("a\u{00A0}/x"),
        json!("a\u{2003}/x"),
        json!("a\u{FEFF}/x"),
        json!("a\u{2028}/x"),
        json!("C:\\x"),
        json!("hxxps://x"),
        json!("a https://x"),
        json!("www.x"),
        json!("~/x"),
        json!("=\\x"),
        json!("tab\there"),
        json!("zero\u{200B}width"),
        json!("vsift frame get"),
        json!("Vsift x"),
        json!([]),
        json!(["e1", "e1"]),
        json!({}),
    ]
}

/// Every pointer of `value` (objects' members and arrays' items).
fn pointers(value: &Value, at: &str, into: &mut Vec<String>) {
    match value {
        Value::Object(members) => {
            for (name, member) in members {
                let child = format!("{at}/{}", name.replace('~', "~0").replace('/', "~1"));
                into.push(child.clone());
                pointers(member, &child, into);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let child = format!("{at}/{index}");
                into.push(child.clone());
                pointers(item, &child, into);
            }
        }
        _ => {}
    }
}

/// The parent of `pointer` and its last segment.
fn split(pointer: &str) -> Option<(&str, &str)> {
    pointer.rsplit_once('/')
}

fn remove(value: &mut Value, pointer: &str) {
    let Some((parent, last)) = split(pointer) else {
        return;
    };
    match value.pointer_mut(parent) {
        Some(Value::Object(members)) => {
            members.remove(&last.replace("~1", "/").replace("~0", "~"));
        }
        Some(Value::Array(items)) => {
            if let Ok(index) = last.parse::<usize>()
                && index < items.len()
            {
                items.remove(index);
            }
        }
        _ => {}
    }
}

#[test]
fn the_examples_and_the_skeleton_agree() -> TestResult {
    let oracle = Oracle::new()?;
    for base in bases()? {
        assert_eq!(oracle.disagreement(&base), None);
        assert!(oracle.checker.schema_error_pointers(&base).is_empty());
    }
    Ok(())
}

#[test]
fn every_systematic_mutation_agrees() -> TestResult {
    let oracle = Oracle::new()?;
    let mut disagreements = Vec::new();
    let mut checked = 0_usize;
    let mut refused = 0_usize;
    for base in bases()? {
        let mut locations = Vec::new();
        pointers(&base, "", &mut locations);
        for location in &locations {
            let mut removed = base.clone();
            remove(&mut removed, location);
            let mut variants = vec![removed];
            for replacement in replacements() {
                let mut replaced = base.clone();
                if let Some(slot) = replaced.pointer_mut(location) {
                    *slot = replacement;
                }
                variants.push(replaced);
            }
            if let Some(Value::Object(_)) = base.pointer(location) {
                let mut extended = base.clone();
                if let Some(Value::Object(members)) = extended.pointer_mut(location) {
                    members.insert("unexpected".to_owned(), Value::Null);
                }
                variants.push(extended);
            }
            if let Some(Value::Array(items)) = base.pointer(location)
                && let Some(first) = items.first()
            {
                let mut doubled = base.clone();
                if let Some(Value::Array(items)) = doubled.pointer_mut(location) {
                    items.push(first.clone());
                }
                variants.push(doubled);
            }
            for variant in variants {
                checked += 1;
                if !oracle.checker.schema_error_pointers(&variant).is_empty() {
                    refused += 1;
                }
                if let Some(difference) = oracle.disagreement(&variant) {
                    disagreements.push(difference);
                }
            }
        }
    }
    assert!(checked > 2_000, "only {checked} mutations");
    // Both verdicts occur often, so agreement is not vacuous.
    assert!(
        refused > checked / 4 && checked - refused > checked / 20,
        "{refused} of {checked} refused"
    );
    assert!(
        disagreements.is_empty(),
        "{} of {checked} mutations disagree; first: {}",
        disagreements.len(),
        disagreements.first().map_or("", String::as_str)
    );
    Ok(())
}

/// A JSON value of bounded size, biased toward the schema's own words.
fn arbitrary_value() -> impl Strategy<Value = Value> {
    let words = prop::sample::select(vec![
        "1",
        "complete",
        "partial",
        "verified",
        "unavailable",
        "frame",
        "crop",
        "audio",
        "transcript_segment",
        "e1",
        "e2",
        "c1",
        "actual",
        "observed",
        "supported",
        "unsupported",
        "none",
        "left_open",
        "compact",
        "images_total",
        "VERIFY_SOURCE",
        "CANCELLED",
        "ses_0123456789abcdef",
        "evd_0123456789abcdef",
        "tsg_0123456789abcdef",
        "op_0123456789abcdef",
    ]);
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        (0_u64..100_000_000_000).prop_map(Value::from),
        (-1.0e12_f64..1.0e12).prop_map(Value::from),
        words.prop_map(Value::from),
        "[ -~\u{00A0}\u{2000}-\u{2003}\u{2028}\u{200B}\u{202E}\u{FEFF}/\\\\:~]{0,24}"
            .prop_map(Value::from),
    ];
    leaf.prop_recursive(3, 16, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::btree_map(
                prop::sample::select(vec![
                    "id",
                    "type",
                    "kind",
                    "support",
                    "citations",
                    "segment_id",
                    "evidence_id",
                    "pixels_inspected",
                    "note",
                    "reason",
                    "code",
                    "from_us",
                    "to_us",
                    "x",
                ]),
                inner,
                0..4
            )
            .prop_map(|members| {
                Value::Object(
                    members
                        .into_iter()
                        .map(|(name, value)| (name.to_owned(), value))
                        .collect::<Map<_, _>>(),
                )
            }),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, failure_persistence: None, ..ProptestConfig::default() })]

    /// A generated value placed at a generated location of a base draft.
    #[test]
    fn generated_drafts_agree(base in 0_usize..4, location in any::<prop::sample::Index>(), value in arbitrary_value()) {
        let oracle = Oracle::new().map_err(|error| TestCaseError::fail(error.to_string()))?;
        let bases = bases().map_err(|error| TestCaseError::fail(error.to_string()))?;
        let mut draft = bases[base % bases.len()].clone();
        let mut locations = Vec::new();
        pointers(&draft, "", &mut locations);
        if !locations.is_empty() {
            let at = location.get(&locations).clone();
            if let Some(slot) = draft.pointer_mut(&at) {
                *slot = value;
            }
        }
        prop_assert_eq!(oracle.disagreement(&draft), None);
    }
}
