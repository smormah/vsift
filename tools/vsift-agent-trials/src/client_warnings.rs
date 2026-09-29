//! Finds a client's own reports that it did not apply the trial
//! configuration.
//!
//! A trial is only meaningful under the configuration the harness gave the
//! client. The first dry trial (2026-09-28) showed that a client can drop
//! part of it and still run: Claude Code printed "Ignoring 3
//! permissions.allow entries from .claude/settings.json: this workspace has
//! not been trusted" on stderr and carried on. Any such report makes the
//! trial **invalid**: the grader fails its `client_configuration` check and
//! records the report, so a misconfigured run can never pass silently.
//!
//! The search covers the client's whole stderr and, in the event stream,
//! only the client's own notices (Claude Code `system` records, Codex
//! `error` events and items), never tool output: a transcript or skill text
//! that happens to contain the same words must not invalidate a trial.

use serde_json::Value;

use crate::trace::ClientKind;

/// Lower-case phrases that show the client ignored or could not apply part
/// of its configuration: settings, permission rules, the sandbox or the
/// skill.
const CONFIGURATION_PHRASES: [&str; 20] = [
    "has not been trusted",
    // Codex reports a setting it does not know as a stream item of type
    // `error` ("Codex is ignoring 1 unrecognized configuration setting ...
    // `tools.view_image` is ignored", the Codex diagnostic pass of
    // 2026-09-29), not on stderr.
    "unrecognized configuration",
    "is ignoring",
    "is ignored",
    "deprecated setting",
    "entries from .claude/settings",
    "invalid settings",
    "settings error",
    "failed to parse settings",
    "failed to load config",
    "error loading config",
    "sandbox setup required",
    "refusing to run unsandboxed",
    "windows sandbox setup",
    "failed to load skill",
    // Codex's Linux sandbox (bubblewrap) could not be set up: commands
    // would fail or, worse, the client might fall back to no sandbox.
    "could not find bubblewrap",
    "needs access to create user namespaces",
    "no permissions to create a new namespace",
    "error building bubblewrap command",
    "sandbox failed a command",
];

/// Longest report kept, in characters.
const MAX_REPORT_CHARS: usize = 240;

/// The client's reports that it ignored part of the trial configuration,
/// each bounded and prefixed with where it was found.
#[must_use]
pub fn configuration_warnings(client: ClientKind, stdout: &str, stderr: &str) -> Vec<String> {
    let mut warnings: Vec<String> = stderr
        .lines()
        .filter(|line| names_configuration(line))
        .map(|line| report("stderr", line))
        .collect();
    for line in stdout.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        for notice in client_notices(client, &event) {
            if names_configuration(&notice.text)
                || (notice.error && names_configuration_topic(&notice.text))
            {
                warnings.push(report("stream", &notice.text));
            }
        }
    }
    warnings.dedup();
    warnings
}

fn names_configuration(text: &str) -> bool {
    let lower = text.to_lowercase();
    CONFIGURATION_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
}

/// Whether a client's own error notice is about its configuration, a
/// setting or its sandbox, whatever the wording: an error of that kind
/// always means the trial did not run as configured.
fn names_configuration_topic(text: &str) -> bool {
    let lower = text.to_lowercase();
    ["config", "setting", "sandbox"]
        .iter()
        .any(|topic| lower.contains(topic))
}

/// One piece of text in which the client itself speaks.
struct Notice {
    text: String,
    /// Whether the client reported it as an error (Codex `error` events and
    /// items), which counts on its topic alone.
    error: bool,
}

fn report(source: &str, text: &str) -> String {
    let trimmed: String = text.trim().chars().take(MAX_REPORT_CHARS).collect();
    format!("{source}: {trimmed}")
}

/// The text of a stream event that is the client speaking, not a tool.
fn client_notices(client: ClientKind, event: &Value) -> Vec<Notice> {
    let strings = |value: &Value, error: bool| -> Vec<Notice> {
        ["message", "text", "content", "warning", "error"]
            .iter()
            .filter_map(|field| value[*field].as_str())
            .map(|text| Notice {
                text: text.to_owned(),
                error,
            })
            .collect()
    };
    match client {
        ClientKind::ClaudeCode | ClientKind::ProcedureWalker => {
            if event["type"] == "system" {
                strings(event, false)
            } else {
                Vec::new()
            }
        }
        ClientKind::Codex => {
            if event["type"] == "error" {
                strings(event, true)
            } else if event["item"]["type"] == "error" {
                strings(&event["item"], true)
            } else if event["item"]["type"] == "command_execution" {
                // Codex's Linux sandbox reports its own failure as the
                // command's output, before the command runs ("bwrap: Can't
                // bind mount ...", the first container debug run). Only
                // output that *starts* with bubblewrap's prefix counts, so
                // a transcript that merely contains the words cannot.
                event["item"]["aggregated_output"]
                    .as_str()
                    .filter(|output| output.trim_start().starts_with("bwrap: "))
                    .map(|output| {
                        vec![Notice {
                            text: format!("sandbox failed a command: {output}"),
                            error: false,
                        }]
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dry_run_trust_warning_invalidates_a_claude_trial() {
        let stderr = "Ignoring 3 permissions.allow entries from .claude/settings.json: this workspace has not been trusted. Run Claude Code interactively here once and accept the trust dialog, or set projects[\"C:/vsift-trials/t/workspace\"].hasTrustDialogAccepted: true in C:\\vsift-trials\\.clients\\claude\\.claude.json.\n";
        let warnings = configuration_warnings(ClientKind::ClaudeCode, "", stderr);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].starts_with("stderr: Ignoring 3 permissions.allow"));
        assert!(warnings[0].chars().count() <= MAX_REPORT_CHARS + "stderr: ".len());
    }

    #[test]
    fn codex_sandbox_and_skill_failures_are_reported_and_benign_lines_are_not() {
        let stderr = "Reading additional input from stdin...\n\
            2026-09-28T19:54:28Z ERROR codex_core::session::session: failed to load skill C:\\t\\.agents\\skills\\vsift\\SKILL.md: missing YAML frontmatter\n\
            2026-09-28T19:54:29Z WARN sandbox setup required: sandbox users missing\n";
        let stdout = "{\"type\":\"item.completed\",\"item\":{\"type\":\"error\",\"message\":\"windows unelevated restricted-token sandbox cannot enforce deny-read restrictions directly; refusing to run unsandboxed\"}}\n";
        let warnings = configuration_warnings(ClientKind::Codex, stdout, stderr);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(
            warnings
                .iter()
                .any(|warning| warning.starts_with("stream: "))
        );
    }

    #[test]
    fn a_linux_sandbox_that_cannot_start_invalidates_a_codex_trial() {
        // What bubblewrap and codex-cli 0.155 print when the container
        // refuses user namespaces (Docker's builtin seccomp profile).
        let stderr = "bwrap: No permissions to create a new namespace, likely because the kernel does not allow non-privileged user namespaces.\n\
            error building bubblewrap command: Read-only file system (os error 30)\n";
        let warnings = configuration_warnings(ClientKind::Codex, "", stderr);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        // The first container debug run: the sandbox failed every command
        // before it ran, reported as the command's output.
        let failed = "{\"type\":\"item.completed\",\"item\":{\"type\":\"command_execution\",\"exit_code\":1,\"aggregated_output\":\"bwrap: Can't bind mount /bindfile on /newroot/t/workspace/.home/.cache/vsift-sessions: Unable to mount source on destination: No such file or directory\\n\"}}";
        assert_eq!(
            configuration_warnings(ClientKind::Codex, failed, "").len(),
            1
        );
        let quoted = "{\"type\":\"item.completed\",\"item\":{\"type\":\"command_execution\",\"aggregated_output\":\"{\\\"text\\\":\\\"bwrap: Can't bind mount\\\"}\"}}";
        assert!(configuration_warnings(ClientKind::Codex, quoted, "").is_empty());
    }

    #[test]
    fn tool_output_that_quotes_the_words_does_not_invalidate_a_trial() {
        let stdout = [
            "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"content\":\"Ignoring 3 permissions.allow entries from .claude/settings.json: this workspace has not been trusted\"}]}}",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"command_execution\",\"aggregated_output\":\"sandbox setup required\"}}",
            "not json at all: has not been trusted",
        ]
        .join("\n");
        assert!(configuration_warnings(ClientKind::ClaudeCode, &stdout, "").is_empty());
        assert!(configuration_warnings(ClientKind::Codex, &stdout, "").is_empty());
        let notice = "{\"type\":\"system\",\"subtype\":\"warning\",\"message\":\"Invalid settings in .claude/settings.json\"}";
        assert_eq!(
            configuration_warnings(ClientKind::ClaudeCode, notice, "").len(),
            1
        );
    }
}
