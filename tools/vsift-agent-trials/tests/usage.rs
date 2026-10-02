//! Usage capture: tokens and cost as the clients report them, and what a
//! tool printed, read from replayed streams. The figures are the clients',
//! never estimated by the harness (P12 recorded none).
//!
//! The streams are written from the event shapes the parsers already read
//! (a Claude Code `result` event with `usage` and `total_cost_usd`, one
//! `assistant` event per content block repeating the message's usage, and a
//! Codex `turn.completed` with `usage`); a real client's wording is what the
//! pilot runs check (known limit L-120).

use serde_json::{Value, json};
use vsift_agent_trials::{
    grade::reported_usage,
    trace::{CallKind, MAX_OUTPUT_CHARS, UsageSource, micro_usd, parse_claude, parse_codex},
};

fn lines(events: &[Value]) -> String {
    events
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Test builders take their blocks inline, by value"
)]
fn assistant(message: &str, usage: &Value, block: Value) -> Value {
    json!({"type": "assistant", "message": {"id": message, "content": [block], "usage": usage}})
}

fn text(words: &str) -> Value {
    json!({"type": "text", "text": words})
}

fn tool(id: &str) -> Value {
    json!({"type": "tool_use", "id": id, "name": "Bash", "input": {"command": "vsift --help"}})
}

#[test]
fn the_result_event_totals_the_run_and_carries_the_clients_cost() {
    let usage = json!({"input_tokens": 12, "output_tokens": 340, "cache_read_input_tokens": 9_000,
                       "cache_creation_input_tokens": 1_500});
    let stream = lines(&[
        // One model message written as two events, each repeating its usage.
        assistant("msg_1", &usage, text("first block")),
        assistant("msg_1", &usage, tool("t1")),
        json!({"type": "result", "subtype": "success", "is_error": false, "result": "done",
               "num_turns": 2, "duration_ms": 9_000, "total_cost_usd": 0.012_345_6,
               "usage": {"input_tokens": 30, "output_tokens": 700, "cache_read_input_tokens": 18_000,
                         "cache_creation_input_tokens": 1_500}}),
    ]);
    let trace = parse_claude(&stream);
    assert_eq!(trace.usage.source, UsageSource::ResultEvent);
    assert_eq!(trace.usage.input_tokens, 30);
    assert_eq!(trace.usage.output_tokens, 700);
    assert_eq!(trace.usage.cached_input_tokens, 18_000);
    assert_eq!(trace.usage.cache_creation_input_tokens, 1_500);
    assert_eq!(trace.usage.cost_micro_usd, Some(12_346));
    assert_eq!(reported_usage(&trace), Some(trace.usage));
}

#[test]
fn without_a_result_total_each_message_counts_once() {
    let first = json!({"input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": 100});
    let second = json!({"input_tokens": 20, "output_tokens": 15, "cache_creation_input_tokens": 7});
    let stream = lines(&[
        assistant("msg_1", &first, text("a")),
        assistant("msg_1", &first, tool("t1")),
        assistant("msg_2", &second, text("b")),
    ]);
    let trace = parse_claude(&stream);
    assert_eq!(trace.usage.source, UsageSource::AssistantMessages);
    assert_eq!(
        trace.usage.input_tokens, 30,
        "msg_1 counted once, not twice"
    );
    assert_eq!(trace.usage.output_tokens, 20);
    assert_eq!(trace.usage.cached_input_tokens, 100);
    assert_eq!(trace.usage.cache_creation_input_tokens, 7);
    assert_eq!(
        trace.usage.cost_micro_usd, None,
        "an estimate is never made up"
    );
}

#[test]
fn a_client_that_reports_nothing_leaves_no_figures() {
    let trace = parse_claude(&lines(&[json!({"type": "system", "subtype": "init"})]));
    assert_eq!(trace.usage.source, UsageSource::None);
    assert_eq!(reported_usage(&trace), None);
    let codex = parse_codex(&lines(&[json!({"type": "turn.started"})]));
    assert_eq!(reported_usage(&codex), None);
}

#[test]
fn codex_reports_tokens_per_turn_and_no_cost() {
    let stream = lines(&[
        json!({"type": "turn.started"}),
        json!({"type": "turn.completed",
               "usage": {"input_tokens": 24_000, "cached_input_tokens": 20_000, "output_tokens": 120,
                         "reasoning_output_tokens": 64}}),
        json!({"type": "turn.completed",
               "usage": {"input_tokens": 1_000, "cached_input_tokens": 500, "output_tokens": 30}}),
    ]);
    let trace = parse_codex(&stream);
    assert_eq!(trace.usage.source, UsageSource::TurnEvents);
    assert_eq!(trace.usage.input_tokens, 25_000);
    assert_eq!(trace.usage.cached_input_tokens, 20_500);
    assert_eq!(trace.usage.output_tokens, 150);
    assert_eq!(trace.usage.reasoning_output_tokens, 64);
    assert_eq!(trace.usage.cost_micro_usd, None);
    assert_eq!(trace.turns, Some(2));
}

#[test]
fn a_cost_becomes_millionths_of_a_dollar_or_nothing() {
    assert_eq!(micro_usd(0.0), Some(0));
    assert_eq!(micro_usd(1.234_567_4), Some(1_234_567));
    assert_eq!(micro_usd(0.000_000_5), Some(1));
    for bad in [-0.01, f64::NAN, f64::INFINITY, 1.0e12] {
        assert_eq!(micro_usd(bad), None, "{bad}");
    }
}

#[test]
fn what_a_call_printed_is_kept_bounded_for_the_gap_report() {
    let long = "x".repeat(MAX_OUTPUT_CHARS + 500);
    let stream = lines(&[
        assistant("msg_1", &json!({}), tool("t1")),
        assistant("msg_2", &json!({}), tool("t2")),
        json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": "plain text"},
            {"type": "tool_result", "tool_use_id": "t2", "content": [
                {"type": "text", "text": "first"}, {"type": "text", "text": &long}]}]}}),
    ]);
    let trace = parse_claude(&stream);
    assert_eq!(trace.calls[0].output.as_deref(), Some("plain text"));
    let second = trace.calls[1].output.as_deref().unwrap_or_default();
    assert!(
        second.starts_with("first\nxxx"),
        "{}",
        &second[..20.min(second.len())]
    );
    assert_eq!(second.chars().count(), MAX_OUTPUT_CHARS);

    let codex = parse_codex(&lines(&[
        json!({"type": "item.started", "item": {"id": "i1", "type": "command_execution", "command": "vsift --help"}}),
        json!({"type": "item.completed", "item": {"id": "i1", "type": "command_execution",
               "command": "vsift --help", "aggregated_output": "usage text", "exit_code": 0, "status": "completed"}}),
    ]));
    assert_eq!(codex.calls[0].output.as_deref(), Some("usage text"));
    assert!(matches!(codex.calls[0].kind, CallKind::Shell { .. }));
}

#[test]
fn a_trace_without_outputs_still_parses_as_before() -> Result<(), serde_json::Error> {
    // A procedure walker's trace and older serialised traces have no
    // `output`; the default is none.
    let call: vsift_agent_trials::trace::ToolCall = serde_json::from_value(json!({
        "index": 0, "step": 1, "id": "x", "kind": {"kind": "shell", "command": "vsift --help"},
        "input": null, "denied": false, "exit_code": 0, "is_error": false, "completed": true}))?;
    assert_eq!(call.output, None);
    Ok(())
}
