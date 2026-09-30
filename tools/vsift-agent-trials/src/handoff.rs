//! Extracts and checks the handoff an agent wrote.
//!
//! The handoff is the final message's one fenced `vsift-handoff` block,
//! holding JSON that must follow `skills/vsift/handoff.schema.json` and the
//! rules of `references/handoff.md` a schema cannot express (every cited
//! reference exists, work cut short that can continue carries a resume
//! card ([`cut_short_reason`]), a card is at most 2 KiB, a visual claim rests on inspected pixels,
//! given budget limits are the profile's). Before any check, a closed value
//! written in another letter case is read as the schema's spelling
//! ([`HandoffSchema::normalize_case`]); another word is still refused. The
//! text checks here
//! also apply to the whole final message: no local path or home prefix, no
//! live link and no raw hidden or control character.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::{
    error::TrialError,
    policy::{BudgetProfile, Budgets},
};

/// The largest serialized resume card the skill allows.
pub const MAX_RESUME_BYTES: usize = 2 * 1024;

/// The fenced block's info string.
const FENCE_INFO: &str = "vsift-handoff";

/// Extracts the one `vsift-handoff` block of a final message and parses it.
///
/// # Errors
///
/// A description of what is wrong: no block, several blocks, an unclosed
/// block or JSON that does not parse.
pub fn extract(message: &str) -> Result<Value, String> {
    let mut blocks = Vec::new();
    let mut open: Option<(usize, Vec<&str>)> = None;
    for line in message.lines() {
        let trimmed = line.trim_start();
        let ticks = trimmed.chars().take_while(|value| *value == '`').count();
        match open.take() {
            Some((opened, body)) if ticks >= opened && trimmed[ticks..].trim().is_empty() => {
                blocks.push(body.join("\n"));
            }
            Some((opened, mut body)) => {
                body.push(line);
                open = Some((opened, body));
            }
            None if ticks >= 3 && trimmed[ticks..].trim() == FENCE_INFO => {
                open = Some((ticks, Vec::new()));
            }
            None => {}
        }
    }
    if open.is_some() {
        return Err("the vsift-handoff block is not closed".to_owned());
    }
    match blocks.as_slice() {
        [] => Err("the final message has no vsift-handoff block".to_owned()),
        [block] => serde_json::from_str(block)
            .map_err(|error| format!("the vsift-handoff block is not JSON: {error}")),
        _ => Err(format!(
            "the final message has {} vsift-handoff blocks",
            blocks.len()
        )),
    }
}

/// The closed vocabularies of the handoff schema: every member that holds
/// one, written as a path (`claims[].section`, `budget.exhausted[]`), with
/// its allowed values. Read from the schema itself, so the grader and the
/// skill's vocabulary table (checked by the CLI's `skill_contract` guard
/// against the same schema) cannot drift from it.
pub type Vocabulary = BTreeMap<String, BTreeSet<String>>;

/// Guards the schema walk against a reference cycle; the handoff schema
/// nests about eight levels deep.
const MAX_SCHEMA_DEPTH: usize = 32;

/// Reads every `enum` and string `const` of `schema` with the member path
/// that holds it, following `$ref`, `oneOf`, `anyOf`, `allOf`, `properties`
/// and `items`. Conditional subschemas (`if`, `then`, `else`, `not`) only
/// restrict members defined elsewhere, so they are not read.
#[must_use]
pub fn vocabulary(schema: &Value) -> Vocabulary {
    let mut vocabulary = Vocabulary::new();
    collect_vocabulary(schema, schema, "", 0, &mut vocabulary);
    vocabulary
}

fn collect_vocabulary(
    schema: &Value,
    node: &Value,
    path: &str,
    depth: usize,
    into: &mut Vocabulary,
) {
    if depth > MAX_SCHEMA_DEPTH {
        return;
    }
    if let Some(target) = node["$ref"]
        .as_str()
        .and_then(|reference| reference.strip_prefix('#'))
        .and_then(|pointer| schema.pointer(pointer))
    {
        collect_vocabulary(schema, target, path, depth + 1, into);
    }
    for keyword in ["oneOf", "anyOf", "allOf"] {
        for branch in node[keyword].as_array().into_iter().flatten() {
            collect_vocabulary(schema, branch, path, depth + 1, into);
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
        collect_vocabulary(schema, member, &child, depth + 1, into);
    }
    if let Some(items) = node.get("items") {
        collect_vocabulary(schema, items, &format!("{path}[]"), depth + 1, into);
    }
}

/// Rewrites a closed value written in another letter case (`"Actual"`) to
/// the schema's own spelling (`"actual"`) at every member of `path`, and
/// says where. Another word is left as it is, so the schema refuses it:
/// only the case is tolerated, never a synonym (maintainer decision,
/// 2026-09-29).
fn normalize_case_at(
    value: &mut Value,
    segments: &[&str],
    location: &str,
    allowed: &BTreeSet<String>,
    notes: &mut Vec<String>,
) {
    let Some((first, rest)) = segments.split_first() else {
        if let Value::String(text) = value
            && !allowed.contains(text.as_str())
            && let Some(canonical) = allowed
                .iter()
                .find(|candidate| candidate.eq_ignore_ascii_case(text))
        {
            notes.push(format!(
                "{location}: {text:?} read as {canonical:?} (letter case)"
            ));
            text.clone_from(canonical);
        }
        return;
    };
    let (name, each) = first
        .strip_suffix("[]")
        .map_or((*first, false), |name| (name, true));
    let child = if location.is_empty() {
        name.to_owned()
    } else {
        format!("{location}.{name}")
    };
    let Some(member) = value.get_mut(name) else {
        return;
    };
    if !each {
        normalize_case_at(member, rest, &child, allowed, notes);
        return;
    }
    if let Value::Array(items) = member {
        for (index, item) in items.iter_mut().enumerate() {
            normalize_case_at(item, rest, &format!("{child}[{index}]"), allowed, notes);
        }
    }
}

/// What the handoff checks found: `problems` fail `handoff_valid`,
/// `warnings` are recorded beside it and fail nothing.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Findings {
    /// Schema and rule violations.
    pub problems: Vec<String>,
    /// Harmless departures, such as a citation no claim or instruction uses.
    /// ([`HandoffSchema::normalize_case`] returns its own notes.)
    pub warnings: Vec<String>,
}

/// The handoff schema, compiled once, with the budget profiles a handoff's
/// optional `budget.limits` must agree with and its closed vocabularies.
pub struct HandoffSchema {
    validator: jsonschema::Validator,
    budgets: Budgets,
    vocabulary: Vocabulary,
}

impl HandoffSchema {
    /// Compiles `handoff.schema.json`; `budgets` are the profiles of
    /// `references/budgets.md`.
    ///
    /// # Errors
    ///
    /// [`TrialError::Invalid`] when the schema does not compile.
    pub fn new(schema: &Value, budgets: Budgets) -> Result<Self, TrialError> {
        let validator = jsonschema::options()
            .build(schema)
            .map_err(|error| TrialError::Invalid(format!("handoff.schema.json: {error}")))?;
        Ok(Self {
            validator,
            budgets,
            vocabulary: vocabulary(schema),
        })
    }

    /// The schema's closed vocabularies.
    #[must_use]
    pub const fn vocabulary(&self) -> &Vocabulary {
        &self.vocabulary
    }

    /// Rewrites every closed value written in another letter case to the
    /// schema's spelling, before anything else reads the handoff, and
    /// returns one note per rewritten value. The values are what an agent
    /// knows, and `"Actual"` means `"actual"`; a different word (`"image"`
    /// for a gap kind) is left for the schema to refuse.
    #[must_use]
    pub fn normalize_case(&self, handoff: &mut Value) -> Vec<String> {
        let mut notes = Vec::new();
        for (path, allowed) in &self.vocabulary {
            let segments: Vec<&str> = path.split('.').collect();
            normalize_case_at(handoff, &segments, "", allowed, &mut notes);
        }
        notes
    }

    /// Every schema and semantic problem of a handoff, and its warnings.
    #[must_use]
    pub fn check(&self, handoff: &Value) -> Findings {
        let mut findings = Findings {
            problems: self
                .validator
                .iter_errors(handoff)
                .map(|error| format!("{error} at {}", error.instance_path()))
                .collect(),
            warnings: Vec::new(),
        };
        semantic_problems(handoff, &mut findings);
        self.budget_problems(handoff, &mut findings.problems);
        findings
    }

    /// `budget.limits` is optional because the profile implies it; limits
    /// that are given must be the profile's own unless `budget.overrides`
    /// says the user changed them. The harness never grades usage from these
    /// values: it counts for itself.
    fn budget_problems(&self, handoff: &Value, problems: &mut Vec<String>) {
        let budget = &handoff["budget"];
        let limits = &budget["limits"];
        if !limits.is_object() || budget["overrides"] == true {
            return;
        }
        let profile = match budget["profile"].as_str() {
            Some("compact") => BudgetProfile::Compact,
            Some("standard") => BudgetProfile::Standard,
            _ => return,
        };
        let expected = serde_json::to_value(self.budgets.limits(profile)).unwrap_or_default();
        let differing: Vec<&str> = expected
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(name, value)| &limits[name.as_str()] != *value)
            .map(|(name, _)| name.as_str())
            .collect();
        if !differing.is_empty() {
            problems.push(format!(
                "budget.limits {differing:?} differ from the {} profile and budget.overrides is not true",
                budget["profile"].as_str().unwrap_or_default()
            ));
        }
    }
}

/// The rules of `references/handoff.md` that one schema cannot express.
///
/// A citation that no claim or instruction uses is a warning, not a
/// problem (2026-09-29, PR 3g): it names real evidence, which
/// `citations_resolve` still checks, and misleads no reader, while an
/// agent often lists everything it opened. A claim that rests on evidence
/// but cites none is refused by the schema (`minItems`), so it is not
/// reported a second time here.
fn semantic_problems(handoff: &Value, findings: &mut Findings) {
    let problems = &mut findings.problems;
    let citations = handoff["citations"].as_array().cloned().unwrap_or_default();
    let by_id: BTreeMap<&str, &Value> = citations
        .iter()
        .filter_map(|citation| citation["id"].as_str().map(|id| (id, citation)))
        .collect();
    if by_id.len() != citations.len() {
        problems.push("citation ids are not unique".to_owned());
    }
    let unavailable = handoff["capabilities"]["image_access"] == "unavailable";
    let mut used = BTreeSet::new();
    for citation in &citations {
        let visual = matches!(citation["type"].as_str(), Some("frame" | "crop"));
        if visual && unavailable && citation["pixels_inspected"] != false {
            problems.push(format!(
                "{} claims inspected pixels without image access",
                citation["id"]
            ));
        }
    }
    for claim in handoff["claims"].as_array().into_iter().flatten() {
        let mut grounded = false;
        let references: Vec<&str> = claim["citations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        for &reference in &references {
            used.insert(reference.to_owned());
            match by_id.get(reference) {
                None => problems.push(format!("claim {} cites missing {reference}", claim["id"])),
                Some(citation) => {
                    let visual = matches!(citation["type"].as_str(), Some("frame" | "crop"));
                    grounded |= !visual || citation["pixels_inspected"] == true;
                }
            }
        }
        let rests_on_evidence = matches!(
            claim["support"].as_str(),
            Some("supported" | "partially_supported" | "contradicted")
        );
        if rests_on_evidence && !references.is_empty() && !grounded {
            problems.push(format!(
                "claim {} is {} on uninspected images only",
                claim["id"], claim["support"]
            ));
        }
    }
    for item in handoff["untrusted_instructions"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if let Some(reference) = item["citation"].as_str() {
            used.insert(reference.to_owned());
            if !by_id.contains_key(reference) {
                problems.push(format!(
                    "an untrusted instruction cites missing {reference}"
                ));
            }
        }
    }
    for id in by_id.keys() {
        if !used.contains(*id) {
            findings
                .warnings
                .push(format!("citation {id} is never used"));
        }
    }
    let problems = &mut findings.problems;
    if handoff["resume"].is_null()
        && let Some(reason) = cut_short_reason(handoff)
    {
        problems.push(format!(
            "the work was cut short ({reason}) and can continue, but there is no resume card"
        ));
    }
    if !handoff["resume"].is_null() {
        let size = serde_json::to_string(&handoff["resume"]).map_or(usize::MAX, |text| text.len());
        if size > MAX_RESUME_BYTES {
            problems.push(format!("the resume card is {size} bytes"));
        }
    }
    // P12 PR 3i: a finding to verify again names the window it holds for.
    for item in handoff["resume"]["to_verify"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if item["from_us"].as_u64() > item["to_us"].as_u64() {
            problems.push(format!(
                "the resume card's finding on {} ends before it starts",
                item["id"]
            ));
        }
    }
}

/// The gap reasons that say work stopped early and another run can pick it
/// up: the agent's budget ran out (`budget_exhausted`), or a transcription
/// was cancelled or interrupted with its checkpoints kept (`cancelled`).
const RESUMABLE_GAP_REASONS: [&str; 2] = ["budget_exhausted", "cancelled"];

/// Why the handoff needs a resume card, if it does.
///
/// The card is required only when the work was cut short **and can
/// continue**: an exhausted budget limit, or a gap saying the budget ran
/// out or a job was cancelled or interrupted (reason `cancelled` or code
/// `CANCELLED`). A report that is `partial` only because a capability is
/// missing (images, speech recognition, tools) or the session expired does
/// not need one: resuming cannot fix those (supervisor's decision,
/// 2026-09-29, after Sonnet 5.5 left the card out of 3 of 3 A-05 runs). A
/// card that is given is still validated in full.
#[must_use]
pub fn cut_short_reason(handoff: &Value) -> Option<String> {
    if let Some(limits) = handoff["budget"]["exhausted"].as_array()
        && !limits.is_empty()
    {
        let names: Vec<&str> = limits.iter().filter_map(Value::as_str).collect();
        return Some(format!("budget limits exhausted: {}", names.join(", ")));
    }
    handoff["gaps"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|gap| {
            let reason = gap["reason"].as_str().unwrap_or_default();
            if RESUMABLE_GAP_REASONS.contains(&reason) {
                Some(format!("a gap with reason {reason}"))
            } else if gap["code"] == "CANCELLED" {
                Some("a gap with code CANCELLED".to_owned())
            } else {
                None
            }
        })
}

/// Characters that hide or reorder text: bidirectional controls, zero-width
/// characters, the word joiner and the byte-order mark.
#[must_use]
pub const fn is_hidden_character(character: char) -> bool {
    matches!(
        character,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
    )
}

/// Local facts the text checks look for: the trial root and the operating
/// system user name, which must never reach a report.
#[derive(Clone, Debug, Default)]
pub struct PrivateMarkers {
    /// Lower-case strings that must not appear (the trial root in both
    /// slash styles, the user name).
    pub strings: Vec<String>,
}

/// Every text-safety problem of a final message.
#[must_use]
pub fn text_problems(message: &str, markers: &PrivateMarkers) -> Vec<String> {
    let mut problems = Vec::new();
    let lowered = message.to_lowercase();
    if let Some(character) = message.chars().find(|value| is_hidden_character(*value)) {
        problems.push(format!(
            "the report contains a raw hidden character U+{:04X}",
            u32::from(character)
        ));
    }
    if message
        .chars()
        .any(|value| value.is_control() && !matches!(value, '\n' | '\r' | '\t'))
    {
        problems.push("the report contains a raw control character".to_owned());
    }
    for scheme in ["http://", "https://", "ftp://", "file://", "www."] {
        if lowered.contains(scheme) {
            problems.push(format!("the report contains a live link ({scheme})"));
        }
    }
    if has_extended_length_path(message) {
        problems.push("the report contains an extended-length path (\\\\?\\)".to_owned());
    }
    for marker in [
        "/home/",
        "/users/",
        "/root/",
        "~/",
        "~\\",
        "appdata",
        "%userprofile%",
        "$home",
    ] {
        if lowered.contains(marker) {
            problems.push(format!(
                "the report contains a home or local path ({marker})"
            ));
        }
    }
    if has_drive_path(message) {
        problems.push("the report contains an absolute drive path".to_owned());
    }
    for marker in &markers.strings {
        if !marker.is_empty() && lowered.contains(marker.as_str()) {
            problems.push("the report names the trial root or the user name".to_owned());
        }
    }
    problems
}

/// Whether the text holds a Windows extended-length path: the `\\?\`
/// prefix (or `//?/`) followed by a drive (`C:`) or the UNC form (`UNC\`).
/// The bare prefix is not a path: the skill itself tells agents to retry an
/// image without it, and a report may say so (A-09, 2026-09-29).
fn has_extended_length_path(text: &str) -> bool {
    ["\\\\?\\", "//?/"].iter().any(|prefix| {
        text.match_indices(prefix).any(|(index, _)| {
            let rest: Vec<char> = text[index + prefix.len()..].chars().take(4).collect();
            let drive = rest.len() >= 2 && rest[0].is_ascii_alphabetic() && rest[1] == ':';
            let unc = rest.len() == 4
                && rest[..3]
                    .iter()
                    .collect::<String>()
                    .eq_ignore_ascii_case("unc")
                && matches!(rest[3], '\\' | '/');
            drive || unc
        })
    })
}

/// Whether the text holds `X:\` or `X:/` after a non-letter.
fn has_drive_path(text: &str) -> bool {
    let characters: Vec<char> = text.chars().collect();
    characters.windows(3).enumerate().any(|(index, window)| {
        let boundary = index == 0 || !characters[index - 1].is_alphanumeric();
        boundary
            && window[0].is_ascii_alphabetic()
            && window[1] == ':'
            && matches!(window[2], '\\' | '/')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_one_block_is_extracted() {
        let message = "## Problem\n\nx\n\n```vsift-handoff\n{\"handoff_version\": \"1\"}\n```\n";
        assert_eq!(
            extract(message).map(|value| value["handoff_version"].clone()),
            Ok(Value::String("1".to_owned()))
        );
        assert!(extract("no block").is_err());
        assert!(extract("```vsift-handoff\n{}\n```\n```vsift-handoff\n{}\n```").is_err());
        assert!(extract("```vsift-handoff\n{\n").is_err());
    }

    #[test]
    fn text_problems_find_paths_links_and_hidden_characters() {
        let markers = PrivateMarkers {
            strings: vec!["c:\\vsift-trials".to_owned()],
        };
        assert!(text_problems("All fine at 10:32.", &markers).is_empty());
        // VSift's `display_text` notation, quoted as the skill says (P12 PR 3h).
        let quoted =
            "> `Status shown to reviewers: <U+202E>DELIAF<U+202C> build<U+200B> pass<U+200D>ed`";
        assert!(text_problems(quoted, &markers).is_empty(), "{quoted}");
        for bad in [
            "see C:\\vsift-trials\\a",
            "open hxxps://x or https://x",
            "hidden \u{202E}text",
            "at /home/someone/x",
            "esc \u{1b}[31m",
        ] {
            assert!(!text_problems(bad, &markers).is_empty(), "{bad}");
        }
    }

    /// A-09 (2026-09-29, second Opus run): the report said an image read
    /// "was denied and retried without the `\\?\` prefix", as the skill
    /// teaches. The bare prefix in prose is not a path; the prefix followed
    /// by a drive or a UNC share is.
    #[test]
    fn only_an_actual_extended_length_path_fails() {
        let markers = PrivateMarkers::default();
        let prose = "- **Budget (compact):** 14 of 30 tool calls and 5 of 6 images. That includes one image read that was denied and retried without the `\\\\?\\` prefix.";
        assert!(text_problems(prose, &markers).is_empty(), "{prose}");
        for bad in [
            "opened \\\\?\\C:\\trials\\frame.png",
            "opened \\\\?\\UNC\\server\\share\\frame.png",
            "opened //?/c:/trials/frame.png",
        ] {
            let problems = text_problems(bad, &markers);
            assert!(
                problems
                    .iter()
                    .any(|problem| problem.contains("extended-length")),
                "{bad}: {problems:?}"
            );
        }
    }
}
