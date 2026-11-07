//! The grade of one value-measurement phase: the cross-arm grade of the
//! report ([`super::claims`]), the measures of method section 7, the safety
//! reading, and whether the run counts.
//!
//! A run is **excluded** only for an infrastructure failure (planner's
//! ruling 12.1): the client reported that it ignored its configuration, it
//! stopped at a usage limit, or it ended with an error before it made one
//! tool call. Everything else counts as it ended, including a run that hit
//! its budget (ruling, question 4) and a run with a safety finding.

use std::{collections::BTreeSet, fs, path::Path};

use serde::{Deserialize, Serialize};

use super::{
    claims::{ClaimGrade, grade_report},
    prepare::ValueManifest,
    task::{Arm, Task},
};
use crate::{
    calls::{ReadScope, normalise},
    client_warnings::configuration_warnings,
    cold::{Violation, safety_violations},
    error::{TrialError, read_json, write_json},
    evaluate::{DEBUG_RUN_REASON, NO_ACTION_REASON, USAGE_LIMIT_REASON},
    layout::TrialLayout,
    run::{RunRecord, raw_output, read_manifest, read_run},
    skill::SkillReferences,
    timing::{TIMES_FILE, TimeSplit, read_times, split},
    trace::{CallKind, Trace, Usage, parse},
    truth::CorpusTruth,
};

/// The file a value grade is written to, in `harness/phase-<n>/`.
pub const VALUE_GRADE_FILE: &str = "value-grade.json";

/// The calls by which the skill finishes a report (`handoff check`,
/// `handoff skeleton`): counted like any call, and shown on their own line
/// (planner's ruling, question 9).
const FINISHING: [&str; 2] = ["check", "skeleton"];

/// Image file extensions an agent may open.
const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];

/// The measures of one phase.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Measures {
    /// Model round trips: steps that asked for a tool, plus the final answer.
    pub round_trips: u64,
    /// Round trips less the steps that only finished the report (V1's
    /// `handoff check` and `skeleton`); reported, not gated.
    pub round_trips_without_finishing: u64,
    /// Tool calls, all kinds.
    pub tool_calls: u64,
    /// Calls the client refused.
    pub denied_calls: u64,
    /// Calls that failed (non-zero exit or a client error).
    pub failed_calls: u64,
    /// A call repeating the program and first argument of a call that just
    /// failed.
    pub retried_calls: u64,
    /// Calls that ran `ffmpeg`, `ffprobe` or `whisper-cli` (recorded in every
    /// arm; in V arms these are the direct calls of ruling 12.4).
    pub media_tool_calls: u64,
    /// Calls that ran `vsift`.
    pub vsift_calls: u64,
    /// `vsift handoff check` and `skeleton` calls.
    pub finishing_calls: u64,
    /// Images opened.
    pub images: u64,
    /// Their bytes, read from disk at grade time.
    pub image_bytes: u64,
    /// The client's usage figures.
    pub usage: Usage,
    /// Wall time, split.
    pub time: TimeSplit,
}

/// The grade of one value phase.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValueGrade {
    /// The trial's value manifest.
    pub value: ValueManifest,
    /// The phase, from 1.
    pub phase: usize,
    /// The model the client ran.
    pub model: String,
    /// The client's version.
    pub client_version: Option<String>,
    /// Whether the run counts; `false` only for an infrastructure failure.
    pub counted: bool,
    /// Why it does not count, in the harness's words.
    pub excluded_because: Vec<String>,
    /// Whether the run ended at its time limit or turn limit (it still
    /// counts, as it ended).
    pub hit_budget: bool,
    /// How the client said the run ended.
    pub outcome: Option<String>,
    /// The cross-arm grade of the report; `None` while the truth is pending.
    pub claims: Option<ClaimGrade>,
    /// The measures.
    pub measures: Measures,
    /// Safety findings (recorded, never an exclusion).
    pub safety: Vec<Violation>,
}

/// The first word of each simple command in a shell line, a coarse reading
/// for counting media-tool and `vsift` calls (the safety reading is the
/// cold grader's, which parses the shell properly).
fn programs(command: &str) -> Vec<String> {
    command
        .replace("&&", ";")
        .replace("||", ";")
        .split(['|', ';', '\n', '(', ')'])
        .filter_map(|part| {
            part.split_whitespace()
                .find(|word| !word.contains('='))
                .map(|word| {
                    Path::new(word.trim_matches(['"', '\'']))
                        .file_name()
                        .map_or_else(String::new, |name| name.to_string_lossy().to_lowercase())
                })
        })
        .filter(|word| !word.is_empty())
        .collect()
}

fn is_finishing(command: &str) -> bool {
    let words: Vec<&str> = command.split_whitespace().collect();
    words
        .windows(3)
        .any(|window| window[0].ends_with("vsift") && window[1] == "handoff" && FINISHING.contains(&window[2]))
}

fn image_path(call: &crate::trace::ToolCall) -> Option<&str> {
    let path = match &call.kind {
        CallKind::ViewImage { path } | CallKind::ReadFile { path } => path.as_str(),
        _ => return None,
    };
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| IMAGE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
        .map(|_| path)
}

/// The measures of a trace.
#[must_use]
pub fn measure(trace: &Trace, workspace: &Path, time: TimeSplit) -> Measures {
    let mut measures = Measures {
        usage: trace.usage,
        time,
        ..Measures::default()
    };
    let mut steps = BTreeSet::new();
    let mut steps_with_other_calls = BTreeSet::new();
    let mut previous_failure: Option<Vec<String>> = None;
    for call in &trace.calls {
        steps.insert(call.step);
        measures.tool_calls += 1;
        measures.denied_calls += u64::from(call.denied);
        let failed = call.is_error || call.exit_code.is_some_and(|code| code != 0);
        measures.failed_calls += u64::from(failed);
        let command = match &call.kind {
            CallKind::Shell { command } => Some(command.as_str()),
            _ => None,
        };
        let words: Vec<String> = command
            .map(|text| text.split_whitespace().take(2).map(str::to_owned).collect())
            .unwrap_or_default();
        if previous_failure.as_ref().is_some_and(|previous| *previous == words) && !words.is_empty() {
            measures.retried_calls += 1;
        }
        previous_failure = failed.then_some(words);
        let finishing = command.is_some_and(is_finishing);
        if finishing {
            measures.finishing_calls += 1;
        } else {
            steps_with_other_calls.insert(call.step);
        }
        if let Some(command) = command {
            let run = programs(command);
            measures.media_tool_calls += u64::from(
                run.iter()
                    .any(|program| ["ffmpeg", "ffprobe", "whisper-cli"].contains(&program.as_str())),
            );
            measures.vsift_calls += u64::from(run.iter().any(|program| program == "vsift"));
        }
        if let Some(path) = image_path(call) {
            measures.images += 1;
            let resolved = normalise(path, workspace);
            measures.image_bytes += fs::metadata(&resolved).map_or(0, |metadata| metadata.len());
        }
    }
    let answer = u64::from(trace.final_message.as_deref().is_some_and(|text| !text.trim().is_empty()));
    let count = |set: &BTreeSet<usize>| u64::try_from(set.len()).unwrap_or(u64::MAX);
    measures.round_trips = count(&steps) + answer;
    measures.round_trips_without_finishing = count(&steps_with_other_calls) + answer;
    measures
}

/// Grades phase `phase` of a value trial and writes `value-grade.json`.
///
/// # Errors
///
/// [`TrialError`] when the trial is not a value trial or a record cannot be
/// read.
pub fn grade_phase(layout: &TrialLayout, phase: usize, repository: &Path) -> Result<ValueGrade, TrialError> {
    let manifest = read_manifest(layout)?;
    let value = manifest
        .value
        .clone()
        .ok_or_else(|| TrialError::Invalid("not a value-measurement trial".to_owned()))?;
    let task = Task::load(&layout.harness().join("task.json"))?;
    let corpus = CorpusTruth::load(&repository.join("fixtures").join("corpus"))?;
    let record: RunRecord = read_run(layout, phase)?;
    let (stdout, stderr) = raw_output(&record)?;
    let trace = parse(record.client, &stdout)?;

    let mut excluded_because = configuration_warnings(record.client, &stdout, &stderr);
    if record.debug_prompt {
        excluded_because.push(DEBUG_RUN_REASON.to_owned());
    }
    if record.usage_limit.is_some() {
        excluded_because.push(USAGE_LIMIT_REASON.to_owned());
    } else if trace.calls.is_empty() && record.exit_code != Some(0) && !record.timed_out {
        excluded_because.push(NO_ACTION_REASON.to_owned());
    }
    let hit_budget = record.timed_out
        || trace
            .outcome
            .as_deref()
            .is_some_and(|outcome| outcome.contains("max_turns"));

    let phase_index = phase
        .checked_sub(1)
        .ok_or_else(|| TrialError::Invalid("phases are counted from 1".to_owned()))?;
    let items = task.truth_items(phase_index, &corpus)?;
    let claims = (!items.is_empty())
        .then(|| grade_report(trace.final_message.as_deref().unwrap_or_default(), &items));

    let times = read_times(&record.stdout.with_file_name(TIMES_FILE)).unwrap_or_default();
    let time = split(record.client, &stdout, &times, record.wall_ms);
    let measures = measure(&trace, &layout.workspace(), time);

    let references = SkillReferences::load(repository)?;
    let skill_directories = match value.arm {
        Arm::R0 | Arm::V0 => Vec::new(),
        Arm::R1 => vec![layout.workspace().join(".claude").join("skills").join(&value.guide_name)],
        Arm::V1 => vec![layout.workspace().join(".claude").join("skills").join("vsift")],
    };
    let scope = ReadScope {
        workspace: layout.workspace(),
        skill_directories,
        session_root: layout.session_root(),
        client_home: record.client_home.clone(),
    };
    let safety = safety_violations(&trace, &scope, &references.policy);

    let grade = ValueGrade {
        value,
        phase,
        model: record.model.clone(),
        client_version: record.client_version.clone(),
        counted: excluded_because.is_empty(),
        excluded_because,
        hit_budget,
        outcome: trace.outcome.clone(),
        claims,
        measures,
        safety,
    };
    write_json(&layout.phase(phase).join(VALUE_GRADE_FILE), &grade)?;
    Ok(grade)
}

/// Reads a phase's value grade.
///
/// # Errors
///
/// [`TrialError`] when it is missing or malformed.
pub fn read_grade(layout: &TrialLayout, phase: usize) -> Result<ValueGrade, TrialError> {
    serde_json::from_value(read_json(&layout.phase(phase).join(VALUE_GRADE_FILE))?)
        .map_err(|error| TrialError::json(VALUE_GRADE_FILE, error))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::trace::{ToolCall, UsageSource};

    fn call(index: usize, step: usize, command: &str, exit: i64) -> ToolCall {
        ToolCall {
            index,
            step,
            id: format!("c{index}"),
            kind: CallKind::Shell {
                command: command.to_owned(),
            },
            input: json!({}),
            denied: false,
            exit_code: Some(exit),
            is_error: false,
            completed: true,
            output: None,
        }
    }

    #[test]
    fn round_trips_count_steps_and_the_answer_and_finishing_is_apart() {
        let trace = Trace {
            calls: vec![
                call(0, 0, "ffprobe -v error recording.mp4", 0),
                call(1, 0, "ffmpeg -i recording.mp4 -vf fps=1 f%03d.png", 0),
                call(2, 1, "ffmpeg -ss 12 -i recording.mp4 x.png", 1),
                call(3, 2, "ffmpeg -ss 12 -i recording.mp4 y.png", 0),
                call(4, 3, "vsift handoff check --json <<'VSIFT_HANDOFF'", 0),
            ],
            final_message: Some("Answer".to_owned()),
            usage: Usage {
                source: UsageSource::ResultEvent,
                ..Usage::default()
            },
            ..Trace::default()
        };
        let measures = measure(&trace, Path::new("."), TimeSplit::default());
        assert_eq!(measures.round_trips, 5);
        assert_eq!(measures.round_trips_without_finishing, 4);
        assert_eq!(measures.tool_calls, 5);
        assert_eq!(measures.failed_calls, 1);
        assert_eq!(measures.retried_calls, 1);
        assert_eq!(measures.media_tool_calls, 4);
        assert_eq!(measures.vsift_calls, 1);
        assert_eq!(measures.finishing_calls, 1);
    }

    #[test]
    fn programs_are_read_through_pipes_paths_and_assignments() {
        assert_eq!(
            programs("LANG=C /usr/bin/ffmpeg -i a 2>&1 | grep showinfo"),
            vec!["ffmpeg", "grep"]
        );
    }
}
