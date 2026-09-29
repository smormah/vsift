//! Counts the images Codex viewed, from its own session rollout.
//!
//! codex-cli 0.155's `exec --json` stream has no event for an image the
//! model views (known limit L-075), so the stream alone cannot show image
//! access or count images against the budget. Codex also writes every
//! session to a rollout file below its home, `$CODEX_HOME/sessions/<y>/<m>/
//! <d>/rollout-*.jsonl`, unless it runs with `--ephemeral`; its response
//! items include each `view_image` function call with the path.
//!
//! `run` reads the rollouts this run wrote right after Codex exits, while
//! the trial's Codex home still exists (in the container it is a tmpfs that
//! is emptied when the run ends), and keeps **counts only** in the run
//! record: images viewed in total, most in one model step, their bytes,
//! whether the check image was one, and the rollout's structure (record
//! types and the names of the session metadata members, never their values)
//! with a scan for the sign-in's values, so a reviewer can see that nothing
//! sensitive was kept. No path, prompt, output or value leaves the home.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    calls::{is_within, normalise, normalise_path},
    leak_check,
};

/// The name of Codex's image tool in its rollout.
pub const VIEW_IMAGE_TOOL: &str = "view_image";

/// What the rollouts of one run show about images, as counts.
#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct ImageViews {
    /// Rollout files read.
    pub rollout_files: usize,
    /// `view_image` calls in total.
    pub total: u64,
    /// Most `view_image` calls in one model step.
    pub max_per_step: u64,
    /// Bytes of the viewed files that exist (the check image and `VSift`'s
    /// images below the session root).
    pub bytes: u64,
    /// Whether one call viewed the skill's check image.
    pub check_image_viewed: bool,
    /// How many records of each type the rollouts hold (`session_meta`,
    /// `response_item:function_call`, ...), to show what a rollout holds
    /// without keeping any of it.
    pub record_types: BTreeMap<String, u64>,
    /// The member names of the session metadata record, never their values.
    pub session_meta_members: Vec<String>,
    /// Whether a value of the client's sign-in file appears in a rollout.
    pub sign_in_value_found: bool,
}

/// Where a rollout's image paths may point.
#[derive(Clone, Debug)]
pub struct ImageScope {
    /// The directory Codex ran in, for relative paths.
    pub workspace: PathBuf,
    /// The workspace's skill folders.
    pub skill_directories: Vec<PathBuf>,
    /// `VSift`'s session root.
    pub session_root: PathBuf,
}

/// The rollout files below `codex_home` written at or after `since`.
#[must_use]
pub fn rollout_files(codex_home: &Path, since: SystemTime) -> Vec<PathBuf> {
    let mut pending = vec![codex_home.join("sessions")];
    let mut found = Vec::new();
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            let named = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("rollout-")
                        && Path::new(name)
                            .extension()
                            .is_some_and(|extension| extension == "jsonl")
                });
            let recent = metadata.modified().is_ok_and(|modified| modified >= since);
            if named && recent {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Reads the rollouts this run wrote. `None` when there is none (for
/// example an `--ephemeral` run): the images are then unmeasured.
#[must_use]
pub fn image_views(codex_home: &Path, since: SystemTime, scope: &ImageScope) -> Option<ImageViews> {
    let files = rollout_files(codex_home, since);
    if files.is_empty() {
        return None;
    }
    let mut texts = Vec::new();
    for file in &files {
        if let Ok(bytes) = fs::read(file) {
            texts.push(String::from_utf8_lossy(&bytes).into_owned());
        }
    }
    let mut views = count(&texts.join("\n"), scope);
    views.rollout_files = files.len();
    let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
    views.sign_in_value_found = leak_check::scan(codex_home, &borrowed).found;
    Some(views)
}

/// Whether a rollout item is a tool call or a tool's output.
fn call_or_output(payload: &Value) -> Option<bool> {
    match payload["type"].as_str()? {
        "function_call" | "custom_tool_call" | "local_shell_call" => Some(true),
        "function_call_output" | "custom_tool_call_output" => Some(false),
        _ => None,
    }
}

/// The path a `view_image` call names.
fn viewed_path(payload: &Value) -> Option<String> {
    if payload["name"] != VIEW_IMAGE_TOOL {
        return None;
    }
    let arguments = match &payload["arguments"] {
        Value::String(text) => serde_json::from_str::<Value>(text).ok()?,
        other => other.clone(),
    };
    arguments["path"].as_str().map(str::to_owned)
}

/// Counts the image views of rollout text (one JSON record per line). A
/// model step is a run of tool calls up to the next tool output: calls the
/// model made together share a step, as in the budget's "images per step".
#[must_use]
pub fn count(text: &str, scope: &ImageScope) -> ImageViews {
    let mut views = ImageViews::default();
    let mut step_views = 0_u64;
    let mut after_output = false;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            *views.record_types.entry("unparsed".to_owned()).or_default() += 1;
            continue;
        };
        let kind = record["type"].as_str().unwrap_or("unknown").to_owned();
        let payload = &record["payload"];
        let key = match payload["type"].as_str() {
            Some(inner) => format!("{kind}:{inner}"),
            None => kind.clone(),
        };
        *views.record_types.entry(key).or_default() += 1;
        if kind == "session_meta"
            && let Some(members) = payload.as_object()
        {
            for name in members.keys() {
                if !views.session_meta_members.contains(name) {
                    views.session_meta_members.push(name.clone());
                }
            }
        }
        if kind != "response_item" {
            continue;
        }
        match call_or_output(payload) {
            Some(true) => {
                if after_output {
                    step_views = 0;
                    after_output = false;
                }
                if let Some(path) = viewed_path(payload) {
                    views.total += 1;
                    step_views += 1;
                    views.max_per_step = views.max_per_step.max(step_views);
                    record_path(&path, scope, &mut views);
                }
            }
            Some(false) => after_output = true,
            None => {}
        }
    }
    views.session_meta_members.sort();
    views
}

fn record_path(path: &str, scope: &ImageScope, views: &mut ImageViews) {
    let normalised = normalise(path, &scope.workspace);
    let in_skill = scope
        .skill_directories
        .iter()
        .any(|directory| is_within(&normalised, &normalise_path(directory)));
    if in_skill && normalised.ends_with("/assets/image-check.png") {
        views.check_image_viewed = true;
    }
    if in_skill || is_within(&normalised, &normalise_path(&scope.session_root)) {
        views.bytes += fs::metadata(path).map_or(0, |metadata| metadata.len());
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn scope() -> ImageScope {
        let workspace = PathBuf::from("/trials/t/workspace");
        ImageScope {
            skill_directories: vec![workspace.join(".agents/skills/vsift")],
            session_root: workspace.join(".home/.cache/vsift-sessions"),
            workspace,
        }
    }

    fn call(name: &str, arguments: &Value) -> String {
        json!({"timestamp": "2026-09-29T00:00:00Z", "type": "response_item",
            "payload": {"type": "function_call", "name": name, "arguments": arguments.to_string(), "call_id": "c"}})
        .to_string()
    }

    fn output() -> String {
        json!({"timestamp": "2026-09-29T00:00:00Z", "type": "response_item",
            "payload": {"type": "function_call_output", "call_id": "c", "output": "x"}})
        .to_string()
    }

    #[test]
    fn view_image_calls_are_counted_per_step_and_the_check_image_is_seen() {
        let meta = json!({"timestamp": "t", "type": "session_meta",
            "payload": {"id": "s", "cwd": "/trials/t/workspace", "cli_version": "0.155.0"}})
        .to_string();
        let lines = [
            meta,
            call(
                VIEW_IMAGE_TOOL,
                &json!({"path": ".agents/skills/vsift/assets/image-check.png"}),
            ),
            output(),
            call("exec_command", &json!({"cmd": "vsift setup check --json"})),
            output(),
            call(
                VIEW_IMAGE_TOOL,
                &json!({"path": "/trials/t/workspace/.home/.cache/vsift-sessions/a.png"}),
            ),
            call(
                VIEW_IMAGE_TOOL,
                &json!({"path": "/trials/t/workspace/.home/.cache/vsift-sessions/b.png"}),
            ),
            output(),
            output(),
        ];
        let views = count(&lines.join("\n"), &scope());
        assert_eq!(views.total, 3);
        assert_eq!(views.max_per_step, 2);
        assert!(views.check_image_viewed);
        assert_eq!(views.session_meta_members, vec!["cli_version", "cwd", "id"]);
        assert_eq!(
            views.record_types.get("response_item:function_call"),
            Some(&4)
        );
    }

    #[test]
    fn no_view_means_no_check_image() {
        let views = count(
            &[
                call("exec_command", &json!({"cmd": "vsift setup check --json"})),
                output(),
            ]
            .join("\n"),
            &scope(),
        );
        assert_eq!(views.total, 0);
        assert!(!views.check_image_viewed);
    }
}
