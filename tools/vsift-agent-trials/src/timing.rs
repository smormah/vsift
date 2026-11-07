//! When each line of a client's event stream arrived, and the split of a
//! run's wall time into tool time and model time (the value measurement's
//! method, section 7).
//!
//! Neither client stamps its events with a time, so `run` reads the
//! client's stdout itself: it writes each line to `stdout.jsonl` byte for
//! byte (the raw log and its digest are what they were before) and one
//! arrival time per line, in milliseconds from the start, to `stdout.times`.
//! Both clients print an event as it happens, so a tool request's line
//! arrives when the model asked and its result's line when the tool
//! finished. **Tool time** is the union of the intervals from each request
//! to its result (calls made together overlap once); **model time** is the
//! wall time less tool time and the client's start (the time to its first
//! line). Buffering in the client could shift a line by a little, which is
//! why Claude Code's own `duration_api_ms` is kept beside the split as a
//! check.

use std::{collections::BTreeMap, fs, path::Path, time::Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};

use crate::trace::ClientKind;

/// The file of arrival times beside `stdout.jsonl`.
pub const TIMES_FILE: &str = "stdout.times";

/// Copies a client's stdout to `out` unchanged and writes one arrival time
/// per line (milliseconds since `started`) to `times`.
///
/// # Errors
///
/// The first read or write error.
pub async fn copy_with_times<R, W, T>(
    reader: R,
    mut out: W,
    mut times: T,
    started: Instant,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
    T: tokio::io::AsyncWrite + Unpin,
{
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line).await?;
        if read == 0 {
            break;
        }
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        out.write_all(&line).await?;
        times.write_all(format!("{elapsed}\n").as_bytes()).await?;
        // Flushed per line, so a copy that `run` stops 30 s after the client
        // exits has lost nothing it had read.
        out.flush().await?;
        times.flush().await?;
    }
    out.flush().await?;
    times.flush().await
}

/// Reads an arrival-time file, or `None` when it is missing or malformed
/// (a run recorded before it existed).
#[must_use]
pub fn read_times(path: &Path) -> Option<Vec<u64>> {
    let text = fs::read_to_string(path).ok()?;
    text.lines().map(|line| line.trim().parse().ok()).collect()
}

/// A run's wall time, split.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimeSplit {
    /// The harness's wall time.
    pub wall_ms: u64,
    /// The time to the client's first line.
    pub startup_ms: u64,
    /// The union of the intervals in which a tool ran.
    pub tool_ms: u64,
    /// Wall time less tool time and start-up.
    pub model_ms: u64,
    /// Tool requests that never got a result line (cut by a timeout).
    pub unfinished_calls: usize,
    /// Claude Code's own API time, when its result event gives it.
    pub client_api_ms: Option<u64>,
}

/// For each call identity, the line that asked for it and the line that
/// answered it.
fn call_lines(client: ClientKind, lines: &[&str]) -> BTreeMap<String, (usize, Option<usize>)> {
    let mut calls: BTreeMap<String, (usize, Option<usize>)> = BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match client {
            ClientKind::ClaudeCode | ClientKind::ProcedureWalker => {
                let Some(content) = event["message"]["content"].as_array() else {
                    continue;
                };
                for block in content {
                    match block["type"].as_str() {
                        Some("tool_use") => {
                            if let Some(id) = block["id"].as_str() {
                                calls.entry(id.to_owned()).or_insert((index, None));
                            }
                        }
                        Some("tool_result") => {
                            if let Some(entry) = block["tool_use_id"]
                                .as_str()
                                .and_then(|id| calls.get_mut(id))
                            {
                                entry.1.get_or_insert(index);
                            }
                        }
                        _ => {}
                    }
                }
            }
            ClientKind::Codex => {
                let Some(id) = event["item"]["id"].as_str() else {
                    continue;
                };
                match event["type"].as_str() {
                    Some("item.started") => {
                        calls.entry(id.to_owned()).or_insert((index, None));
                    }
                    Some("item.completed") => {
                        // An item that completed without starting (a message)
                        // is not a tool call.
                        if let Some(entry) = calls.get_mut(id) {
                            entry.1.get_or_insert(index);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    calls
}

/// Splits a run's wall time from its raw log and arrival times.
#[must_use]
pub fn split(client: ClientKind, log: &str, times: &[u64], wall_ms: u64) -> TimeSplit {
    let lines: Vec<&str> = log.lines().collect();
    let at = |index: usize| times.get(index).copied();
    let mut intervals: Vec<(u64, u64)> = Vec::new();
    let mut unfinished_calls = 0;
    for (asked, answered) in call_lines(client, &lines).values() {
        match (at(*asked), answered.and_then(at)) {
            (Some(start), Some(end)) if end >= start => intervals.push((start, end)),
            (Some(start), None) => {
                unfinished_calls += 1;
                intervals.push((start, wall_ms.max(start)));
            }
            _ => {}
        }
    }
    intervals.sort_unstable();
    let mut tool_ms = 0;
    let mut current: Option<(u64, u64)> = None;
    for (start, end) in intervals {
        current = match current {
            Some((from, to)) if start <= to => Some((from, to.max(end))),
            Some((from, to)) => {
                tool_ms += to - from;
                Some((start, end))
            }
            None => Some((start, end)),
        };
    }
    if let Some((from, to)) = current {
        tool_ms += to - from;
    }
    let startup_ms = times.first().copied().unwrap_or(0).min(wall_ms);
    let client_api_ms = lines
        .iter()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|event| event["type"] == "result")
        .and_then(|event| event["duration_api_ms"].as_u64());
    TimeSplit {
        wall_ms,
        startup_ms,
        tool_ms,
        model_ms: wall_ms.saturating_sub(tool_ms).saturating_sub(startup_ms),
        unfinished_calls,
        client_api_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude_log() -> String {
        [
            r#"{"type":"system","subtype":"init"}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"a","name":"Bash","input":{}},{"type":"tool_use","id":"b","name":"Bash","input":{}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"a","content":"x"}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"b","content":"y"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"c","name":"Read","input":{}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"c","content":"z"}]}}"#,
            r#"{"type":"result","duration_api_ms":4200}"#,
        ]
        .join("\n")
    }

    /// Two calls made together overlap once; a later call adds its own
    /// interval; start-up is the time to the first line.
    #[test]
    fn tool_time_is_the_union_of_call_intervals() {
        let times = [500, 1_000, 3_000, 4_000, 6_000, 6_500, 9_000];
        let split = split(ClientKind::ClaudeCode, &claude_log(), &times, 10_000);
        assert_eq!(split.startup_ms, 500);
        assert_eq!(split.tool_ms, 3_000 + 500);
        assert_eq!(split.model_ms, 10_000 - 3_500 - 500);
        assert_eq!(split.unfinished_calls, 0);
        assert_eq!(split.client_api_ms, Some(4_200));
    }

    #[test]
    fn a_call_cut_by_the_timeout_runs_to_the_end() {
        let log = claude_log().lines().take(2).collect::<Vec<_>>().join("\n");
        let split = split(ClientKind::ClaudeCode, &log, &[100, 1_000], 5_000);
        assert_eq!(split.tool_ms, 4_000);
        assert_eq!(split.unfinished_calls, 2);
    }

    #[test]
    fn codex_items_are_timed_from_start_to_completion() {
        let log = [
            r#"{"type":"item.started","item":{"id":"i1","type":"command_execution"}}"#,
            r#"{"type":"item.completed","item":{"id":"i0","type":"agent_message"}}"#,
            r#"{"type":"item.completed","item":{"id":"i1","type":"command_execution"}}"#,
        ]
        .join("\n");
        let split = split(ClientKind::Codex, &log, &[200, 900, 2_200], 3_000);
        assert_eq!(split.tool_ms, 2_000);
        assert_eq!(split.client_api_ms, None);
    }

    #[tokio::test]
    async fn the_copy_keeps_every_byte_and_one_time_per_line() {
        let input = b"{\"a\":1}\n{\"b\":2}\nlast line without newline".to_vec();
        let mut out = Vec::new();
        let mut times = Vec::new();
        let copied = copy_with_times(&input[..], &mut out, &mut times, Instant::now()).await;
        assert!(copied.is_ok());
        assert_eq!(out, input);
        assert_eq!(String::from_utf8_lossy(&times).lines().count(), 3);
    }
}
