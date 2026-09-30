//! The closed vocabularies of the handoff schema, and reading a closed value
//! written in another letter case as the schema's spelling.
//!
//! Maintainer decision of 2026-09-29 (the grader), kept for `handoff check`
//! by the supervisor on 2026-09-30: `"Actual"` means `"actual"`, so it is
//! read that way and noted; a different word (`"image"` for a gap kind) is
//! left for the schema to refuse.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{
    rule::{HandoffFinding, HandoffRule},
    schema::{child_pointer, index_pointer},
};

/// Every member of the schema that holds a closed value, written as a path
/// (`claims[].section`, `budget.exhausted[]`), with its allowed values.
pub type HandoffVocabulary = BTreeMap<String, BTreeSet<String>>;

/// Guards the schema walk against a reference cycle.
const MAX_SCHEMA_DEPTH: usize = 32;

/// Reads every `enum` and string `const` of `schema` with the member path
/// that holds it, following `$ref`, `oneOf`, `anyOf`, `allOf`, `properties`
/// and `items`. Conditional subschemas (`if`, `then`, `not`) only restrict
/// members defined elsewhere, so they are not read.
#[must_use]
pub fn handoff_vocabulary(schema: &Value) -> HandoffVocabulary {
    let mut vocabulary = HandoffVocabulary::new();
    collect(schema, schema, "", 0, &mut vocabulary);
    vocabulary
}

fn collect(schema: &Value, node: &Value, path: &str, depth: usize, into: &mut HandoffVocabulary) {
    if depth > MAX_SCHEMA_DEPTH {
        return;
    }
    if let Some(target) = node["$ref"]
        .as_str()
        .and_then(|reference| reference.strip_prefix('#'))
        .and_then(|pointer| schema.pointer(pointer))
    {
        collect(schema, target, path, depth + 1, into);
    }
    for keyword in ["oneOf", "anyOf", "allOf"] {
        for branch in node[keyword].as_array().into_iter().flatten() {
            collect(schema, branch, path, depth + 1, into);
        }
    }
    let mut values: Vec<String> = node["enum"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    values.extend(node["const"].as_str().map(str::to_owned));
    if !values.is_empty() && !path.is_empty() {
        into.entry(path.to_owned()).or_default().extend(values);
    }
    for (name, member) in node["properties"].as_object().into_iter().flatten() {
        let child = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}.{name}")
        };
        collect(schema, member, &child, depth + 1, into);
    }
    if let Some(items) = node.get("items") {
        collect(schema, items, &format!("{path}[]"), depth + 1, into);
    }
}

/// Rewrites every closed value of `handoff` written in another letter case
/// to the schema's spelling and returns one note per rewritten value.
pub(crate) fn normalize_case(
    vocabulary: &HandoffVocabulary,
    handoff: &mut Value,
) -> Vec<HandoffFinding> {
    let mut notes = Vec::new();
    for (path, allowed) in vocabulary {
        let segments: Vec<&str> = path.split('.').collect();
        normalize_at(handoff, &segments, "", allowed, &mut notes);
    }
    notes
}

fn normalize_at(
    value: &mut Value,
    segments: &[&str],
    pointer: &str,
    allowed: &BTreeSet<String>,
    notes: &mut Vec<HandoffFinding>,
) {
    let Some((first, rest)) = segments.split_first() else {
        if let Value::String(text) = value
            && !allowed.contains(text.as_str())
            && let Some(canonical) = allowed
                .iter()
                .find(|candidate| candidate.eq_ignore_ascii_case(text))
        {
            notes.push(HandoffFinding::at(
                pointer.to_owned(),
                HandoffRule::LetterCase,
                Some(vec![canonical.clone()]),
                HandoffRule::LetterCase.message(),
            ));
            text.clone_from(canonical);
        }
        return;
    };
    let (name, each) = first
        .strip_suffix("[]")
        .map_or((*first, false), |name| (name, true));
    let child = child_pointer(pointer, name);
    let Some(member) = value.get_mut(name) else {
        return;
    };
    if !each {
        normalize_at(member, rest, &child, allowed, notes);
        return;
    }
    if let Value::Array(items) = member {
        for (index, item) in items.iter_mut().enumerate() {
            normalize_at(item, rest, &index_pointer(&child, index), allowed, notes);
        }
    }
}
