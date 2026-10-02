//! Recognises a client that stopped at its usage limit (P14).
//!
//! A batch spends the maintainer's Claude and Codex allowances, and both
//! clients refuse to continue once one is used up. A phase that ended that
//! way says nothing about `VSift` or the agent: it is invalid, never counted,
//! and the campaign scripts wait and run it again.
//!
//! The reading is deliberately narrow: it looks only where a client reports
//! its own failures (Claude Code's error `result` event and its `error`
//! events, Codex's `error` and `turn.failed` events, and the client's
//! standard error), and only when the run produced no successful final
//! message, so an agent's own text that mentions a "rate limit" (a fixture
//! about retry limits) can never be taken for one. The messages these
//! clients print are not a published contract: the patterns below are the
//! ones their documentation and public reports show, and a pilot run is
//! where a new wording is found (known limit L-120).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::trace::ClientKind;

/// What a usage-limited phase reported.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UsageLimit {
    /// When the client says the limit lifts, in Unix seconds, if it said.
    pub reset_unix_s: Option<u64>,
    /// The kind of message that matched (`limit`, `rate`, `overloaded`);
    /// never the message itself.
    pub matched: String,
}

/// Fragments (lower case) of a message that says the allowance or the rate
/// is used up, with the kind they name.
const PATTERNS: [(&str, &str); 11] = [
    ("usage limit", "limit"),
    ("usage_limit", "limit"),
    ("limit reached", "limit"),
    ("hit your limit", "limit"),
    ("hit your usage", "limit"),
    ("credit balance is too low", "limit"),
    ("quota", "limit"),
    ("rate limit", "rate"),
    ("rate_limit", "rate"),
    ("too many requests", "rate"),
    ("overloaded", "overloaded"),
];

fn matched_kind(text: &str) -> Option<&'static str> {
    let lowered = text.to_ascii_lowercase();
    PATTERNS
        .iter()
        .find(|(fragment, _)| lowered.contains(fragment))
        .map(|(_, kind)| *kind)
}

/// A reset time written as `|<unix seconds>` after the message, which
/// Claude Code's headless mode has used.
fn reset_in(text: &str) -> Option<u64> {
    let (_, tail) = text.split_once('|')?;
    let digits: String = tail
        .trim()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    (9..=11)
        .contains(&digits.len())
        .then(|| digits.parse().ok())
        .flatten()
}

/// The strings of one stream event that report a failure.
fn failure_texts(event: &Value) -> Vec<String> {
    let mut texts = Vec::new();
    let mut push = |value: &Value| {
        if let Some(text) = value.as_str() {
            texts.push(text.to_owned());
        }
    };
    if event["type"] == "result" && event["is_error"] == true {
        push(&event["result"]);
    }
    if event["type"] == "error" || event["type"] == "turn.failed" {
        push(&event["message"]);
        push(&event["error"]["message"]);
        push(&event["error"]);
    }
    if event["error"].is_string() {
        push(&event["error"]);
        for block in event["message"]["content"].as_array().into_iter().flatten() {
            push(&block["text"]);
        }
    }
    texts
}

/// Whether the stream holds a successful final message: Claude Code's
/// `result` event that is not an error, or a Codex turn that completed.
fn finished_normally(client: ClientKind, events: &[Value]) -> bool {
    events.iter().any(|event| match client {
        ClientKind::ClaudeCode | ClientKind::ProcedureWalker => {
            event["type"] == "result" && event["is_error"] != true
        }
        ClientKind::Codex => event["type"] == "turn.completed",
    })
}

/// Reads a finished phase's output for a usage limit.
#[must_use]
pub fn detect(client: ClientKind, stdout: &str, stderr: &str) -> Option<UsageLimit> {
    let events: Vec<Value> = stdout
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    if finished_normally(client, &events) {
        return None;
    }
    let mut candidates: Vec<String> = events.iter().flat_map(failure_texts).collect();
    candidates.extend(stderr.lines().map(str::to_owned));
    candidates.iter().find_map(|text| {
        matched_kind(text).map(|kind| UsageLimit {
            reset_unix_s: reset_in(text),
            matched: kind.to_owned(),
        })
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn lines(events: &[Value]) -> String {
        events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_claude_code_limit_message_is_read_with_its_reset_time() {
        let stdout = lines(&[
            json!({"type": "system", "subtype": "init"}),
            json!({"type": "result", "subtype": "success", "is_error": true,
                   "result": "Claude AI usage limit reached|1790000000"}),
        ]);
        let limit = detect(ClientKind::ClaudeCode, &stdout, "");
        assert_eq!(
            limit,
            Some(UsageLimit {
                reset_unix_s: Some(1_790_000_000),
                matched: "limit".to_owned()
            })
        );
    }

    #[test]
    fn a_rate_limit_error_on_an_assistant_event_counts() {
        let stdout = lines(&[json!({
            "type": "assistant", "error": "rate_limit",
            "message": {"content": [{"type": "text", "text": "You've hit your limit"}]}})]);
        assert_eq!(
            detect(ClientKind::ClaudeCode, &stdout, "").map(|limit| limit.matched),
            Some("rate".to_owned())
        );
    }

    #[test]
    fn codex_failure_events_and_stderr_count() {
        let stdout = lines(&[
            json!({"type": "turn.started"}),
            json!({"type": "turn.failed", "error": {"message": "You've hit your usage limit. Try again later."}}),
        ]);
        assert!(detect(ClientKind::Codex, &stdout, "").is_some());
        assert!(detect(ClientKind::Codex, "", "Error: usage_limit_reached").is_some());
    }

    #[test]
    fn a_run_that_finished_is_never_a_usage_limit() {
        // The agent's own words about a "rate limit" are not the client's.
        let claude = lines(&[json!({
            "type": "result", "is_error": false,
            "result": "The retry limit and the rate limit are three."})]);
        assert_eq!(detect(ClientKind::ClaudeCode, &claude, ""), None);
        let codex = lines(&[
            json!({"type": "item.completed", "item": {"type": "agent_message", "text": "usage limit"}}),
            json!({"type": "turn.completed", "usage": {}}),
        ]);
        assert_eq!(
            detect(ClientKind::Codex, &codex, "rate limit in stderr"),
            None
        );
    }

    #[test]
    fn other_failures_are_not_usage_limits() {
        let stdout = lines(&[json!({"type": "result", "is_error": true, "result": "max turns"})]);
        assert_eq!(detect(ClientKind::ClaudeCode, &stdout, "boom"), None);
        assert_eq!(detect(ClientKind::Codex, "", ""), None);
    }
}
