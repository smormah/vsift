//! Parses both clients' event streams into one normalised trace.
//!
//! Claude Code (`claude -p --output-format stream-json --verbose`) writes one
//! JSON object per line: a `system`/`init` record, `assistant` messages
//! whose content holds `text` and `tool_use` blocks, `user` messages holding
//! each `tool_result`, and a final `result` record with the final text,
//! usage and `permission_denials`. Codex (`codex exec --json`) writes
//! `thread.started`, `turn.started`, `item.started`/`item.updated`/
//! `item.completed` (items of type `agent_message`, `reasoning`,
//! `command_execution`, `file_change`, `mcp_tool_call`, `web_search`,
//! `todo_list`, `error`) and `turn.completed` with usage.
//!
//! Every tool use the model *requested* is a call in the trace, whether the
//! client ran it, denied it or the run ended first: ADR 0022 decision 7
//! counts an attempted out-of-policy action as a failure even when the
//! client's sandbox stopped it. An event the parser does not recognise is
//! kept as an [`CallKind::Unrecognised`] call and fails the trial, so a
//! change in a client's format is noticed rather than silently ignored.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::TrialError;

/// Which named client produced a stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    /// Anthropic's Claude Code.
    ClaudeCode,
    /// The Codex command-line client.
    Codex,
    /// The deterministic procedure walker (`p12_skill_procedure_e2e`); not
    /// an agent.
    ProcedureWalker,
}

/// What a call asked for.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CallKind {
    /// A shell command (Claude `Bash`, Codex `command_execution`).
    Shell {
        /// The command text as the client reported it.
        command: String,
    },
    /// A file read through the client's own tool (Claude `Read`).
    ReadFile {
        /// The path as the model gave it.
        path: String,
    },
    /// A listing or search of a directory through the client's own tool
    /// (Claude `Glob`, `Grep`, `LS`). Allowed only inside the skill folders,
    /// where it reads nothing the skill does not already hand the agent; the
    /// first counted trial (2026-09-29) listed the skill's `examples/` this way.
    ListFiles {
        /// The tool name.
        tool: String,
        /// The directory the tool searched, as the model gave it; empty when
        /// the model gave none (the tool then searches the workspace).
        path: String,
        /// The file-name patterns the tool applied (Glob `pattern`, Grep
        /// `glob`), which could otherwise climb out of `path`.
        patterns: Vec<String>,
    },
    /// An image opened through a dedicated image tool.
    ViewImage {
        /// The path as the model gave it.
        path: String,
    },
    /// The client's skill loader (Claude `Skill`).
    Skill {
        /// The skill named.
        name: String,
    },
    /// Client bookkeeping that acts on nothing (a to-do list).
    Internal {
        /// The tool or item type.
        tool: String,
    },
    /// Any other tool: writing or editing files, web access, MCP servers,
    /// sub-agents. Never allowed in a trial.
    Other {
        /// The tool or item type.
        tool: String,
    },
    /// An item type this parser does not know.
    Unrecognised {
        /// The type as reported.
        tool: String,
    },
}

/// One tool use the model requested.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Position in the stream, from 0.
    pub index: usize,
    /// The model turn that requested it: calls requested together share a
    /// step (the budget's "images per step").
    pub step: usize,
    /// The client's identity for the call.
    pub id: String,
    /// What was asked for.
    pub kind: CallKind,
    /// The client's input, as reported.
    pub input: Value,
    /// Whether the client refused to run it (permission or sandbox).
    pub denied: bool,
    /// The exit code, where the client reports one.
    pub exit_code: Option<i64>,
    /// Whether the client reported the call as failed.
    pub is_error: bool,
    /// Whether the client reported a result for it at all.
    pub completed: bool,
}

/// Token usage, where the stream reports it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Input tokens read from a prompt cache.
    pub cached_input_tokens: u64,
}

/// The normalised trace of one run.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Trace {
    /// Every requested tool use, in order.
    pub calls: Vec<ToolCall>,
    /// The final message the user would read.
    pub final_message: Option<String>,
    /// The model the client reported, if it did.
    pub model: Option<String>,
    /// The client version the client reported, if it did.
    pub client_version: Option<String>,
    /// The working directory the client reported, if it did.
    pub cwd: Option<String>,
    /// Token usage summed over the run.
    pub usage: Usage,
    /// Turns, where reported.
    pub turns: Option<u64>,
    /// Wall time the client reported, in milliseconds.
    pub duration_ms: Option<u64>,
    /// How the client said the run ended (`success`, `error_max_turns`,
    /// `turn.failed`, ...).
    pub outcome: Option<String>,
    /// Lines that were not JSON or not objects.
    pub unparsed_lines: usize,
}

fn text_of(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

fn number(value: &Value) -> u64 {
    value.as_u64().unwrap_or_default()
}

/// Parses a Claude Code `stream-json` log.
#[must_use]
pub fn parse_claude(log: &str) -> Trace {
    let mut trace = Trace::default();
    let mut by_id: BTreeMap<String, usize> = BTreeMap::new();
    let mut last_text: Option<String> = None;
    let mut step = 0;
    for line in log.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            trace.unparsed_lines += 1;
            continue;
        };
        match event["type"].as_str() {
            Some("system") if event["subtype"] == "init" => {
                trace.model = text_of(&event["model"]);
                trace.client_version = text_of(&event["claude_code_version"]);
                trace.cwd = text_of(&event["cwd"]);
            }
            Some("assistant") => {
                step += 1;
                for block in event["message"]["content"].as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("text") => last_text = text_of(&block["text"]),
                        Some("tool_use") => {
                            let id = text_of(&block["id"]).unwrap_or_default();
                            let call = ToolCall {
                                index: trace.calls.len(),
                                step,
                                id: id.clone(),
                                kind: claude_kind(
                                    block["name"].as_str().unwrap_or_default(),
                                    &block["input"],
                                ),
                                input: block["input"].clone(),
                                denied: false,
                                exit_code: None,
                                is_error: false,
                                completed: false,
                            };
                            by_id.insert(id, trace.calls.len());
                            trace.calls.push(call);
                        }
                        _ => {}
                    }
                }
                let usage = &event["message"]["usage"];
                trace.usage.input_tokens += number(&usage["input_tokens"]);
                trace.usage.output_tokens += number(&usage["output_tokens"]);
                trace.usage.cached_input_tokens += number(&usage["cache_read_input_tokens"]);
            }
            Some("user") => {
                for block in event["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] != "tool_result" {
                        continue;
                    }
                    let id = block["tool_use_id"].as_str().unwrap_or_default();
                    if let Some(call) = by_id.get(id).and_then(|index| trace.calls.get_mut(*index))
                    {
                        call.completed = true;
                        call.is_error = block["is_error"] == true;
                        let content = block["content"].to_string().to_ascii_lowercase();
                        if call.is_error
                            && (content.contains("permission") || content.contains("denied"))
                        {
                            call.denied = true;
                        }
                    }
                }
            }
            Some("result") => {
                trace.final_message = text_of(&event["result"]).or_else(|| last_text.clone());
                trace.turns = event["num_turns"].as_u64();
                trace.duration_ms = event["duration_ms"].as_u64();
                trace.outcome = text_of(&event["subtype"]);
                for denial in event["permission_denials"].as_array().into_iter().flatten() {
                    let id = denial["tool_use_id"].as_str().unwrap_or_default();
                    if let Some(call) = by_id.get(id).and_then(|index| trace.calls.get_mut(*index))
                    {
                        call.denied = true;
                    }
                }
            }
            Some(_) => {}
            None => trace.unparsed_lines += 1,
        }
    }
    if trace.final_message.is_none() {
        trace.final_message = last_text;
    }
    trace
}

/// Tools that act on nothing outside the client's own state.
const CLAUDE_INTERNAL: [&str; 2] = ["TodoWrite", "TaskList"];
/// Tools this harness knows and never allows.
const CLAUDE_OTHER: [&str; 11] = [
    "Write",
    "Edit",
    "MultiEdit",
    "NotebookEdit",
    "WebFetch",
    "WebSearch",
    "Task",
    "Agent",
    "BashOutput",
    "KillShell",
    "PowerShell",
];

fn claude_kind(name: &str, input: &Value) -> CallKind {
    let string = |key: &str| input[key].as_str().unwrap_or_default().to_owned();
    match name {
        "Bash" => CallKind::Shell {
            command: string("command"),
        },
        "Read" => CallKind::ReadFile {
            path: string("file_path"),
        },
        "Glob" | "Grep" | "LS" => CallKind::ListFiles {
            tool: name.to_owned(),
            path: string("path"),
            patterns: [if name == "Glob" { "pattern" } else { "glob" }]
                .iter()
                .map(|key| string(key))
                .filter(|pattern| !pattern.is_empty())
                .collect(),
        },
        "Skill" => CallKind::Skill {
            name: input["skill"]
                .as_str()
                .or_else(|| input["command"].as_str())
                .or_else(|| input["name"].as_str())
                .unwrap_or_default()
                .to_owned(),
        },
        internal if CLAUDE_INTERNAL.contains(&internal) => CallKind::Internal {
            tool: internal.to_owned(),
        },
        other if CLAUDE_OTHER.contains(&other) || other.starts_with("mcp__") => CallKind::Other {
            tool: other.to_owned(),
        },
        unknown => CallKind::Unrecognised {
            tool: unknown.to_owned(),
        },
    }
}

/// Codex item types that are messages, not tool uses.
const CODEX_MESSAGES: [&str; 3] = ["agent_message", "reasoning", "error"];

/// Parses a Codex `exec --json` log.
#[must_use]
pub fn parse_codex(log: &str) -> Trace {
    let mut trace = Trace::default();
    let mut by_id: BTreeMap<String, usize> = BTreeMap::new();
    for line in log.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            trace.unparsed_lines += 1;
            continue;
        };
        match event["type"].as_str() {
            Some("item.started" | "item.updated" | "item.completed") => {
                let completed = event["type"] == "item.completed";
                let item = &event["item"];
                let item_type = item["type"].as_str().unwrap_or_default();
                if item_type == "agent_message" && completed {
                    trace.final_message = text_of(&item["text"]);
                }
                if CODEX_MESSAGES.contains(&item_type) {
                    continue;
                }
                let id = text_of(&item["id"]).unwrap_or_default();
                let index = *by_id.entry(id.clone()).or_insert_with(|| {
                    trace.calls.push(ToolCall {
                        index: trace.calls.len(),
                        step: trace.calls.len(),
                        id: id.clone(),
                        kind: codex_kind(item_type, item),
                        input: Value::Null,
                        denied: false,
                        exit_code: None,
                        is_error: false,
                        completed: false,
                    });
                    trace.calls.len() - 1
                });
                if let Some(call) = trace.calls.get_mut(index) {
                    call.kind = codex_kind(item_type, item);
                    call.input = codex_input(item);
                    call.exit_code = item["exit_code"].as_i64();
                    let status = item["status"].as_str().unwrap_or_default();
                    call.denied = matches!(status, "declined" | "rejected" | "denied");
                    call.is_error = call.denied
                        || status == "failed"
                        || call.exit_code.is_some_and(|code| code != 0);
                    call.completed = completed;
                }
            }
            Some("turn.completed") => {
                let usage = &event["usage"];
                trace.usage.input_tokens += number(&usage["input_tokens"]);
                trace.usage.output_tokens += number(&usage["output_tokens"]);
                trace.usage.cached_input_tokens += number(&usage["cached_input_tokens"]);
                trace.turns = Some(trace.turns.unwrap_or_default() + 1);
                trace.outcome = Some("turn.completed".to_owned());
            }
            Some("turn.failed") => trace.outcome = Some("turn.failed".to_owned()),
            Some("error") => trace.outcome = Some("error".to_owned()),
            Some(_) => {}
            None => trace.unparsed_lines += 1,
        }
    }
    trace
}

/// The item without its output, which can be long and is not an input.
fn codex_input(item: &Value) -> Value {
    let mut input = item.clone();
    if let Some(object) = input.as_object_mut() {
        object.remove("aggregated_output");
        object.remove("result");
    }
    input
}

fn codex_kind(item_type: &str, item: &Value) -> CallKind {
    let string = |key: &str| item[key].as_str().unwrap_or_default().to_owned();
    match item_type {
        "command_execution" => CallKind::Shell {
            command: string("command"),
        },
        "view_image" | "image_view" => CallKind::ViewImage {
            path: string("path"),
        },
        "todo_list" => CallKind::Internal {
            tool: item_type.to_owned(),
        },
        "file_change" | "mcp_tool_call" | "web_search" | "collab_tool_call" => CallKind::Other {
            tool: item_type.to_owned(),
        },
        unknown => CallKind::Unrecognised {
            tool: unknown.to_owned(),
        },
    }
}

/// Parses a log for `client`. The procedure walker writes its trace as
/// JSON directly.
///
/// # Errors
///
/// [`TrialError::Json`] when a procedure walker trace is not a trace.
pub fn parse(client: ClientKind, log: &str) -> Result<Trace, TrialError> {
    match client {
        ClientKind::ClaudeCode => Ok(parse_claude(log)),
        ClientKind::Codex => Ok(parse_codex(log)),
        ClientKind::ProcedureWalker => serde_json::from_str(log)
            .map_err(|error| TrialError::json("procedure walker trace", error)),
    }
}
