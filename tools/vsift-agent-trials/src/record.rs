//! Writes a bounded trial record for review and for the repository.
//!
//! A record holds what a reviewer needs to trust a result without the
//! conversation or the media: the client, version and model; the settings,
//! skill, scenario and fixture digests and the `VSift` commit; every tool
//! call with its arguments and verdict, local paths replaced by
//! root-relative tokens; exit codes, token and tool usage; the handoff;
//! both results; and the SHA-256 of the raw logs, which stay local. A
//! record is at most [`MAX_RECORD_BYTES`]; `record` refuses to write a
//! larger one rather than cut evidence silently.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{
    cold::ColdVariant,
    error::{TrialError, read_json},
    grade::Grade,
    layout::{TrialLayout, TrialManifest},
    run::{RunRecord, read_manifest, read_run},
    scenario::{Scenario, TrialMode},
    skill::CHECK_IMAGES,
};

/// The largest record, in bytes.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;

/// The longest call summary a record keeps, in characters.
const MAX_SUMMARY_CHARS: usize = 400;

/// Replacements from local strings to neutral tokens, longest first.
#[derive(Clone, Debug, Default)]
pub struct Redactions {
    pairs: Vec<(String, String)>,
}

impl Redactions {
    /// The tokens for a trial: the session root, the per-user base, the
    /// workspace, the harness and trial directories, the client home and
    /// the `vsift` executable's directory, in every slash style; the user
    /// names; the canaries.
    #[must_use]
    pub fn for_trial(
        layout: &TrialLayout,
        manifest: &TrialManifest,
        extra: &[(PathBuf, &str)],
        user_names: &[String],
    ) -> Self {
        let mut pairs = Vec::new();
        let mut add_path = |path: &Path, token: &str| {
            for variant in path_variants(&path.to_string_lossy()) {
                pairs.push((variant, token.to_owned()));
            }
        };
        add_path(&layout.session_root(), "<session-root>");
        add_path(&layout.user_base(), "<home>");
        add_path(&layout.workspace(), "<workspace>");
        add_path(&layout.harness(), "<harness>");
        add_path(layout.trial(), "<trial>");
        if let Some(directory) = manifest.vsift_executable.parent() {
            add_path(directory, "<vsift-dir>");
        }
        if let Some(prefix) = &manifest.install_prefix {
            add_path(prefix, "<install>");
        }
        for directory in &manifest.client_path_directories {
            add_path(directory, "<path-dir>");
        }
        for (path, token) in extra {
            add_path(path, token);
        }
        for name in user_names.iter().filter(|name| name.chars().count() >= 3) {
            pairs.push((name.clone(), "<user>".to_owned()));
        }
        for canary in &manifest.canaries {
            pairs.push((canary.clone(), "<canary>".to_owned()));
        }
        pairs.extend(check_code_pairs());
        pairs.sort_by_key(|pair| std::cmp::Reverse(pair.0.len()));
        Self { pairs }
    }

    /// Applies every replacement, ignoring ASCII case.
    #[must_use]
    pub fn apply(&self, text: &str) -> String {
        let mut result = text.to_owned();
        for (from, to) in &self.pairs {
            result = replace_ignoring_case(&result, from, to);
        }
        result
    }

    /// Applies every replacement to every string of a JSON value.
    #[must_use]
    pub fn apply_value(&self, value: &Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.apply(text)),
            Value::Array(items) => {
                Value::Array(items.iter().map(|item| self.apply_value(item)).collect())
            }
            Value::Object(members) => Value::Object(
                members
                    .iter()
                    .map(|(key, item)| (self.apply(key), self.apply_value(item)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

/// Every spelling of one local path that a record could hold: as the harness
/// knows it, with forward slashes, with the `\\?\` prefix, with doubled
/// backslashes (a JSON string a client printed), and in the Git Bash (MSYS)
/// form of a Windows drive path, which is how Claude Code on Windows runs its
/// commands (#283).
fn path_variants(text: &str) -> Vec<String> {
    let mut variants = vec![
        text.to_owned(),
        text.replace('\\', "/"),
        format!("\\\\?\\{text}"),
        text.replace('\\', "\\\\"),
    ];
    variants.extend(git_bash_spelling(text));
    variants.retain(|variant| !variant.is_empty());
    variants
}

/// `C:\trials\run` as Git Bash writes it, `/c/trials/run` (the drive letter in
/// lower case; the redaction ignores case, so `/C/` is covered too). A path
/// that does not start with a drive letter and a separator has no such
/// spelling: a POSIX path is already in this form.
fn git_bash_spelling(path: &str) -> Option<String> {
    let path = path.strip_prefix("\\\\?\\").unwrap_or(path);
    let mut characters = path.chars();
    let drive = characters.next().filter(char::is_ascii_alphabetic)?;
    if characters.next() != Some(':') {
        return None;
    }
    let rest = characters.as_str();
    if !rest.starts_with(['\\', '/']) {
        return None;
    }
    Some(format!(
        "/{}{}",
        drive.to_ascii_lowercase(),
        rest.replace('\\', "/")
    ))
}

/// Replacements that keep every check image's code out of a record.
///
/// A handoff reports the code the agent read from the check image, and
/// records are committed to the repository, where the code must never
/// appear as text (the CLI's `skill_contract` guard fails on it): a model
/// that found it there could pass the image check without seeing the image.
/// The code is replaced as printed and without white space, the two forms
/// the grader accepts; the record's `image_check` result still says whether
/// the reported code was right.
fn check_code_pairs() -> Vec<(String, String)> {
    CHECK_IMAGES
        .iter()
        .flat_map(|image| {
            let code = image.code();
            let joined: String = code.split_whitespace().collect();
            [code, joined]
        })
        .map(|code| (code, "<check-code>".to_owned()))
        .collect()
}

fn replace_ignoring_case(text: &str, from: &str, to: &str) -> String {
    if from.is_empty() {
        return text.to_owned();
    }
    let lowered = text.to_ascii_lowercase();
    let needle = from.to_ascii_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut position = 0;
    while let Some(found) = lowered[position..].find(&needle) {
        let start = position + found;
        result.push_str(&text[position..start]);
        result.push_str(to);
        position = start + needle.len();
    }
    result.push_str(&text[position..]);
    result
}

fn bounded(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        let mut cut: String = text.chars().take(limit).collect();
        cut.push_str("...");
        cut
    }
}

/// Builds the bounded record of one phase.
///
/// # Errors
///
/// [`TrialError::Invalid`] when even the most compact form exceeds
/// [`MAX_RECORD_BYTES`].
pub fn build_record(
    manifest: &TrialManifest,
    scenario: &Scenario,
    run: &RunRecord,
    graded: &Grade,
    redactions: &Redactions,
) -> Result<Value, TrialError> {
    let calls: Vec<Value> = graded
        .calls
        .iter()
        .map(|call| {
            json!({
                "index": call.index,
                "step": call.step,
                "summary": bounded(&redactions.apply(&call.summary), MAX_SUMMARY_CHARS),
                "denied": call.denied,
                "exit_code": call.exit_code,
                "actions": redactions.apply_value(&serde_json::to_value(&call.actions).unwrap_or(Value::Null)),
            })
        })
        .collect();
    let arguments: Vec<String> = run
        .arguments
        .iter()
        .map(|argument| bounded(&redactions.apply(argument), MAX_SUMMARY_CHARS))
        .collect();
    // The setting a cold run was under, so a result is only compared with
    // runs of the same client and setting: Claude Code ran what `prepare`
    // wrote (strict unless the operator named an isolated machine), Codex
    // always runs the realistic setting inside the container, where its
    // sandbox is the only restriction on commands.
    let cold_setting = (manifest.mode == TrialMode::Cold)
        .then(|| ColdVariant::in_effect(run.client, manifest.cold_variant.unwrap_or_default()));
    let mut record = json!({
        "record_version": 1,
        "trial_id": manifest.trial_id,
        "scenario": {"id": scenario.id, "tests": scenario.tests, "sha256": manifest.scenario_sha256},
        "phase": run.phase,
        "client": {
            "kind": run.client,
            "version": run.client_version,
            "model": run.model,
            "arguments": arguments,
            "environment_names": run.environment_names,
        },
        "settings_sha256": manifest.settings_sha256,
        "skill_sha256": manifest.skill_sha256,
        "vsift": {"commit": manifest.vsift_commit, "sha256": manifest.vsift_sha256},
        "mode": manifest.mode,
        "holdout": manifest.holdout,
        "skill_source": manifest.skill_source,
        "tools_source": manifest.tools_source,
        "install": manifest.install,
        "setup_check": manifest.setup_check,
        "freeze_sha256": manifest.freeze_sha256,
        "cold_assertions": manifest.cold_assertions,
        "cold_setting": cold_setting.map(ColdVariant::identifier),
        "reported_usage": graded.reported_usage,
        "shim_use": graded.shim_use,
        "usage_limit": run.usage_limit,
        "cold": redactions.apply_value(&serde_json::to_value(&graded.cold).unwrap_or(Value::Null)),
        "fixture_hashes": manifest.fixture_hashes,
        "run": {
            "started_unix_s": run.started_unix_s,
            "wall_ms": run.wall_ms,
            "exit_code": run.exit_code,
            "timed_out": run.timed_out,
            "raw_stdout_sha256": run.stdout_sha256,
            "raw_stderr_sha256": run.stderr_sha256,
        },
        "usage": graded.usage,
        "calls": calls,
        "handoff": redactions.apply_value(&graded.handoff.clone().unwrap_or(Value::Null)),
        "mechanical": redactions.apply_value(&serde_json::to_value(&graded.mechanical).unwrap_or(Value::Null)),
        "interpretation": redactions.apply_value(&serde_json::to_value(&graded.interpretation).unwrap_or(Value::Null)),
        "deviations": redactions.apply_value(&json!(graded.deviations)),
        "valid": graded.is_valid(),
        "invalid_reasons": redactions.apply_value(&json!(graded.invalid_reasons)),
        "client_setup": run.client_setup,
    });
    let size =
        |value: &Value| serde_json::to_vec_pretty(value).map_or(usize::MAX, |bytes| bytes.len());
    while size(&record) > MAX_RECORD_BYTES {
        let Some(calls) = record["calls"].as_array_mut() else {
            break;
        };
        if calls.len() <= 1 {
            break;
        }
        let dropped = calls.len() / 2;
        calls.truncate(calls.len() - dropped);
        let previous = record["calls_omitted"].as_u64().unwrap_or_default();
        record["calls_omitted"] = json!(previous + dropped as u64);
    }
    if size(&record) > MAX_RECORD_BYTES {
        return Err(TrialError::Invalid(format!(
            "the record exceeds {MAX_RECORD_BYTES} bytes even without most calls"
        )));
    }
    Ok(record)
}

/// Writes a phase's record to `output`, which the operator names.
///
/// # Errors
///
/// [`TrialError`] when the trial's records cannot be read, the record is
/// too large or the file cannot be written.
pub fn write_record(
    layout: &TrialLayout,
    phase: usize,
    output: &Path,
    client_home: Option<&Path>,
    user_names: &[String],
) -> Result<Value, TrialError> {
    if output
        .extension()
        .is_none_or(|extension| extension != "json")
    {
        return Err(TrialError::Refused(
            "the record file must end in .json".to_owned(),
        ));
    }
    let manifest = read_manifest(layout)?;
    let scenario = Scenario::load(&layout.scenario())?;
    let run = read_run(layout, phase)?;
    if run.debug_prompt {
        return Err(TrialError::Refused(
            "this phase was a debug run (run --debug-prompt); debug runs are never recorded"
                .to_owned(),
        ));
    }
    let graded: Grade = serde_json::from_value(read_json(&layout.phase(phase).join("grade.json"))?)
        .map_err(|error| TrialError::json("grade.json", error))?;
    let extra: Vec<(PathBuf, &str)> = client_home
        .map(|home| (home.to_path_buf(), "<client-home>"))
        .into_iter()
        .collect();
    let redactions = Redactions::for_trial(layout, &manifest, &extra, user_names);
    let record = build_record(&manifest, &scenario, &run, &graded, &redactions)?;
    crate::error::write_json(output, &record)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_ignores_case_and_prefers_the_longest_match() {
        let redactions = Redactions {
            pairs: vec![
                (
                    "C:\\trials\\t1\\workspace".to_owned(),
                    "<workspace>".to_owned(),
                ),
                ("C:\\trials\\t1".to_owned(), "<trial>".to_owned()),
            ],
        };
        assert_eq!(
            redactions.apply("read c:\\TRIALS\\t1\\workspace\\a.png and C:\\trials\\t1\\x"),
            "read <workspace>\\a.png and <trial>\\x"
        );
    }

    /// A redaction set holding every spelling of `path` under `token`.
    fn redactions_for(path: &str, token: &str) -> Redactions {
        let mut pairs: Vec<(String, String)> = path_variants(path)
            .into_iter()
            .map(|variant| (variant, token.to_owned()))
            .collect();
        // `for_trial` applies the longest spelling first.
        pairs.sort_by_key(|pair| std::cmp::Reverse(pair.0.len()));
        Redactions { pairs }
    }

    #[test]
    fn the_git_bash_spelling_of_a_drive_path_is_redacted() {
        // #283: Claude Code on Windows runs its commands in Git Bash, which
        // writes a drive path as `/c/...`; four records of the first batch
        // named the run's workspace that way.
        let redactions = redactions_for(r"C:\vsift-trials\run-9\workspace", "<workspace>");
        for command in [
            "cd /c/vsift-trials/run-9/workspace; ls; vsift --help",
            "cd /C/vsift-trials/run-9/workspace; ls; vsift --help",
            "cd /c/VSIFT-trials/run-9/workspace; ls; vsift --help",
        ] {
            assert_eq!(
                redactions.apply(command),
                "cd <workspace>; ls; vsift --help",
                "{command}"
            );
        }
        // The other four spellings are still redacted.
        for spelling in [
            r"C:\vsift-trials\run-9\workspace",
            "C:/vsift-trials/run-9/workspace",
            r"\\?\C:\vsift-trials\run-9\workspace",
            r"C:\\vsift-trials\\run-9\\workspace",
        ] {
            assert_eq!(
                redactions.apply(&format!("read {spelling}\\a.png")),
                "read <workspace>\\a.png",
                "{spelling}"
            );
        }
    }

    #[test]
    fn only_a_drive_path_has_a_git_bash_spelling() {
        assert_eq!(
            git_bash_spelling(r"D:\a\b").as_deref(),
            Some("/d/a/b"),
            "any drive letter"
        );
        assert_eq!(
            git_bash_spelling(r"\\?\C:\a").as_deref(),
            Some("/c/a"),
            "the verbatim prefix is not part of the spelling"
        );
        assert_eq!(git_bash_spelling("C:/a/b").as_deref(), Some("/c/a/b"));
        assert_eq!(git_bash_spelling("C:").as_deref(), None, "no separator");
        assert_eq!(git_bash_spelling("C:relative").as_deref(), None);
        // The Codex container's paths are POSIX already: nothing is added, and
        // nothing a POSIX path does not contain is redacted.
        assert_eq!(git_bash_spelling("/vsift-trials/run-9"), None);
        assert_eq!(git_bash_spelling("relative/path"), None);
        let posix = redactions_for("/vsift-trials/run-9/workspace", "<workspace>");
        assert_eq!(
            posix.apply("cd /vsift-trials/run-9/workspace && ls /c/other"),
            "cd <workspace> && ls /c/other"
        );
    }

    #[test]
    fn every_check_code_is_replaced_as_printed_and_joined() {
        let redactions = Redactions {
            pairs: check_code_pairs(),
        };
        for image in CHECK_IMAGES {
            let code = image.code();
            let joined: String = code.split_whitespace().collect();
            let handoff = json!({
                "capabilities": {"image_check_code": code.to_ascii_lowercase()},
                "note": format!("read {joined} from the image"),
            });
            let redacted = redactions.apply_value(&handoff);
            assert_eq!(
                redacted["capabilities"]["image_check_code"],
                json!("<check-code>")
            );
            assert_eq!(redacted["note"], json!("read <check-code> from the image"));
        }
    }
}
