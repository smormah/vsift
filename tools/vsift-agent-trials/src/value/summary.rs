//! Records and the summary of a value-measurement batch: the per-task
//! scores, the "equal" and "loss" verdicts of the planner's ruling 12.2, and
//! the thresholds of method section 2, computed and printed, never decided
//! (a loss is read by a person against the runs before any hold).

use std::{collections::BTreeMap, fs, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    grade::{ValueGrade, read_grade},
    task::Arm,
};
use crate::{
    error::{TrialError, read_json, write_json},
    layout::TrialLayout,
    record::{MAX_RECORD_BYTES, Redactions},
    run::read_manifest,
};

/// The schema name every value record carries.
pub const VALUE_RECORD_SCHEMA: &str = "vsift-value-record/1";

/// The score margin of ruling 12.2.
pub const SCORE_MARGIN: f64 = 0.10;

/// The tasks on which a correctness loss holds the launch (ruling 12.2).
pub const GATED_TASKS: [&str; 4] = ["T2", "T4", "T5", "T6"];

/// Writes the bounded record of one value phase: its grade, the hashes it
/// ran under, and nothing that names a local path.
///
/// # Errors
///
/// [`TrialError`] when the grade is missing or the record is too large.
pub fn write_record(
    layout: &TrialLayout,
    phase: usize,
    output: &Path,
    client_home: Option<&Path>,
    user_names: &[String],
) -> Result<Value, TrialError> {
    let manifest = read_manifest(layout)?;
    let grade = read_grade(layout, phase)?;
    let mut extra = vec![(grade.value.tools_directory.clone(), "<tools>")];
    if let Some(home) = client_home {
        extra.push((home.to_path_buf(), "<client-home>"));
    }
    let redactions = Redactions::for_trial(layout, &manifest, &extra, user_names);
    let record = json!({
        "schema": VALUE_RECORD_SCHEMA,
        "trial_id": manifest.trial_id,
        "vsift_commit": manifest.vsift_commit,
        "vsift_sha256": manifest.vsift_sha256,
        "skill_sha256": manifest.skill_sha256,
        "settings_sha256": manifest.settings_sha256,
        "freeze_sha256": manifest.freeze_sha256,
        "install": manifest.install.as_ref().map(|evidence| evidence.version.clone()),
        "fixture_hashes": manifest.fixture_hashes,
        "grade": grade,
    });
    let record = redactions.apply_value(&record);
    let text = serde_json::to_string_pretty(&record)
        .map_err(|error| TrialError::json("value record", error))?;
    if text.len() > MAX_RECORD_BYTES {
        return Err(TrialError::Invalid(format!(
            "the value record is {} bytes, over the {MAX_RECORD_BYTES}-byte bound",
            text.len()
        )));
    }
    write_json(output, &record)?;
    Ok(record)
}

/// One cell: a task's phase in one arm, over its counted runs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    /// The method's task number.
    pub task: String,
    /// The task file's identifier.
    pub task_id: String,
    /// The phase, from 1.
    pub phase: usize,
    /// The arm.
    pub arm: String,
    /// Counted runs, by run index.
    pub runs: Vec<u32>,
    /// Mean share of key facts matched; `None` while the truth is pending.
    pub score: Option<f64>,
    /// Wrong claims per run, by run index.
    pub wrong_by_run: BTreeMap<u32, usize>,
    /// Total wrong claims.
    pub wrong: usize,
    /// Unverifiable claims, total (reported, never gated).
    pub unverifiable: usize,
    /// Mean round trips.
    pub round_trips: f64,
    /// Mean round trips without the finishing calls.
    pub round_trips_without_finishing: f64,
    /// Mean of all tokens the client reported (input, cached, cache-write
    /// and output).
    pub tokens: f64,
    /// Mean client cost estimate, US dollars, where reported.
    pub cost_usd: Option<f64>,
    /// Mean images opened.
    pub images: f64,
    /// Mean wall, tool and model time, seconds.
    pub wall_s: f64,
    /// Mean tool time, seconds.
    pub tool_s: f64,
    /// Mean model time, seconds.
    pub model_s: f64,
    /// Runs that hit their budget.
    pub hit_budget: usize,
    /// Runs with a safety finding.
    pub unsafe_runs: usize,
    /// Runs a person should read.
    pub needs_reading: usize,
}

/// A comparison of two arms on one task phase (ruling 12.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The `VSift` arm's score is at least the raw arm's less 0.10, and it has
    /// no more wrong claims.
    EqualOrBetter,
    /// On a gated task: the score is more than 0.10 below, or more wrong
    /// claims in at least 2 of the 3 run pairs. Read by a person before any
    /// hold is declared.
    Loss,
    /// Neither: worse within the margin, or more wrong in fewer than 2 pairs.
    Neither,
    /// The truth is pending, or a cell has no counted run.
    NotGraded,
}

/// One verdict line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    /// The task number and phase.
    pub task: String,
    /// The phase.
    pub phase: usize,
    /// `v1 against r1` (gated) or `v0 against r0` (reported).
    pub pair: String,
    /// Whether this line can gate (V1 against R1 only).
    pub gated: bool,
    /// The verdict.
    pub verdict: Verdict,
    /// Run pairs in which the `VSift` arm had more wrong claims.
    pub pairs_with_more_wrong: usize,
}

/// The summary of a batch.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ValueSummary {
    /// Every cell.
    pub cells: Vec<Cell>,
    /// The verdicts.
    pub comparisons: Vec<Comparison>,
    /// The thresholds of method section 2, computed (not decided).
    pub thresholds: Vec<String>,
    /// Excluded runs and why (ruling 12.1: every one is listed).
    pub excluded: Vec<String>,
    /// Pilot runs (never counted).
    pub pilots: usize,
}

#[allow(
    clippy::cast_precision_loss,
    reason = "Means of small counts and token totals, reported to two places"
)]
fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let collected: Vec<f64> = values.collect();
    if collected.is_empty() {
        0.0
    } else {
        collected.iter().sum::<f64>() / collected.len() as f64
    }
}

#[allow(clippy::cast_precision_loss, reason = "Token and millisecond counts as means")]
fn cell(grades: &[&ValueGrade]) -> Cell {
    let first = grades.first().copied();
    let scored: Vec<f64> = grades
        .iter()
        .filter_map(|grade| grade.claims.as_ref().map(|claims| claims.score))
        .collect();
    let costs: Vec<f64> = grades
        .iter()
        .filter_map(|grade| grade.measures.usage.cost_micro_usd)
        .map(|micro| micro as f64 / 1.0e6)
        .collect();
    Cell {
        task: first.map(|grade| grade.value.task.clone()).unwrap_or_default(),
        task_id: first.map(|grade| grade.value.task_id.clone()).unwrap_or_default(),
        phase: first.map_or(1, |grade| grade.phase),
        arm: first.map(|grade| grade.value.arm.name().to_owned()).unwrap_or_default(),
        runs: grades.iter().map(|grade| grade.value.run_index).collect(),
        score: (scored.len() == grades.len() && !scored.is_empty())
            .then(|| mean(scored.iter().copied())),
        wrong_by_run: grades
            .iter()
            .map(|grade| {
                (
                    grade.value.run_index,
                    grade.claims.as_ref().map_or(0, |claims| claims.wrong.len()),
                )
            })
            .collect(),
        wrong: grades
            .iter()
            .map(|grade| grade.claims.as_ref().map_or(0, |claims| claims.wrong.len()))
            .sum(),
        unverifiable: grades
            .iter()
            .map(|grade| grade.claims.as_ref().map_or(0, |claims| claims.unverifiable.len()))
            .sum(),
        round_trips: mean(grades.iter().map(|grade| grade.measures.round_trips as f64)),
        round_trips_without_finishing: mean(
            grades
                .iter()
                .map(|grade| grade.measures.round_trips_without_finishing as f64),
        ),
        tokens: mean(grades.iter().map(|grade| {
            let usage = grade.measures.usage;
            (usage.input_tokens
                + usage.cached_input_tokens
                + usage.cache_creation_input_tokens
                + usage.output_tokens) as f64
        })),
        cost_usd: (costs.len() == grades.len() && !costs.is_empty()).then(|| mean(costs.into_iter())),
        images: mean(grades.iter().map(|grade| grade.measures.images as f64)),
        wall_s: mean(grades.iter().map(|grade| grade.measures.time.wall_ms as f64 / 1_000.0)),
        tool_s: mean(grades.iter().map(|grade| grade.measures.time.tool_ms as f64 / 1_000.0)),
        model_s: mean(grades.iter().map(|grade| grade.measures.time.model_ms as f64 / 1_000.0)),
        hit_budget: grades.iter().filter(|grade| grade.hit_budget).count(),
        unsafe_runs: grades.iter().filter(|grade| !grade.safety.is_empty()).count(),
        needs_reading: grades
            .iter()
            .filter(|grade| grade.claims.as_ref().is_some_and(|claims| claims.needs_reading))
            .count(),
    }
}

/// Compares a `VSift` cell with its raw cell by ruling 12.2.
#[must_use]
pub fn compare(vsift: &Cell, raw: &Cell, gated_task: bool) -> (Verdict, usize) {
    let pairs_with_more_wrong = vsift
        .wrong_by_run
        .iter()
        .filter(|(run, wrong)| raw.wrong_by_run.get(run).is_some_and(|other| *wrong > other))
        .count();
    let (Some(vsift_score), Some(raw_score)) = (vsift.score, raw.score) else {
        return (Verdict::NotGraded, pairs_with_more_wrong);
    };
    if vsift_score >= raw_score - SCORE_MARGIN && vsift.wrong <= raw.wrong {
        return (Verdict::EqualOrBetter, pairs_with_more_wrong);
    }
    if gated_task && (vsift_score < raw_score - SCORE_MARGIN || pairs_with_more_wrong >= 2) {
        return (Verdict::Loss, pairs_with_more_wrong);
    }
    (Verdict::Neither, pairs_with_more_wrong)
}

fn find<'a>(cells: &'a [Cell], task_id: &str, phase: usize, arm: Arm) -> Option<&'a Cell> {
    cells
        .iter()
        .find(|cell| cell.task_id == task_id && cell.phase == phase && cell.arm == arm.name())
}

fn saving(raw: f64, vsift: f64) -> Option<f64> {
    (raw > 0.0).then(|| (raw - vsift) / raw)
}

fn percent(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_owned(), |value| format!("{:.0}%", value * 100.0))
}

/// Summarises graded value phases.
#[must_use]
#[allow(clippy::too_many_lines, reason = "The summary reads top to bottom as the method's sections")]
pub fn summarize(grades: &[ValueGrade]) -> ValueSummary {
    let mut summary = ValueSummary::default();
    let mut groups: BTreeMap<(String, usize, Arm), Vec<&ValueGrade>> = BTreeMap::new();
    for grade in grades {
        if grade.value.pilot {
            summary.pilots += 1;
            continue;
        }
        if !grade.counted {
            summary.excluded.push(format!(
                "{} {} run {} phase {}: {}",
                grade.value.task_id,
                grade.value.arm.name(),
                grade.value.run_index,
                grade.phase,
                grade.excluded_because.join("; ")
            ));
            continue;
        }
        groups
            .entry((grade.value.task_id.clone(), grade.phase, grade.value.arm))
            .or_default()
            .push(grade);
    }
    for runs in groups.values_mut() {
        runs.sort_by_key(|grade| grade.value.run_index);
    }
    summary.cells = groups.values().map(|runs| cell(runs)).collect();
    let mut tasks: Vec<(String, String, usize)> = summary
        .cells
        .iter()
        .map(|cell| (cell.task_id.clone(), cell.task.clone(), cell.phase))
        .collect();
    tasks.sort();
    tasks.dedup();
    for (task_id, task, phase) in &tasks {
        for (vsift_arm, raw_arm, gated) in [(Arm::V1, Arm::R1, true), (Arm::V0, Arm::R0, false)] {
            let (Some(vsift), Some(raw)) = (
                find(&summary.cells, task_id, *phase, vsift_arm),
                find(&summary.cells, task_id, *phase, raw_arm),
            ) else {
                continue;
            };
            let gated_task = gated && GATED_TASKS.contains(&task.as_str());
            let (verdict, pairs_with_more_wrong) = compare(vsift, raw, gated_task);
            summary.comparisons.push(Comparison {
                task: format!("{task} ({task_id})"),
                phase: *phase,
                pair: format!("{} against {}", vsift_arm.name(), raw_arm.name()),
                gated,
                verdict,
                pairs_with_more_wrong,
            });
        }
    }
    // Method section 2, computed per task for V1 against R1.
    for (task_id, task, phase) in &tasks {
        let (Some(v1), Some(r1)) = (
            find(&summary.cells, task_id, *phase, Arm::V1),
            find(&summary.cells, task_id, *phase, Arm::R1),
        ) else {
            continue;
        };
        let line = match task.as_str() {
            "T4" | "T5" | "T6" => format!(
                "H1/H2 {task_id}: round trips {} fewer (needs >= 30%), tokens {} fewer (needs >= 20%), at equal correctness: see the verdict",
                percent(saving(r1.round_trips, v1.round_trips)),
                percent(saving(r1.tokens, v1.tokens))
            ),
            "T2" | "T3" => format!(
                "H3 {task_id}: V1 score {} against R1 {}, wrong {} against {} (V1 must beat R1; equal drops the claim)",
                v1.score.map_or_else(|| "n/a".to_owned(), |score| format!("{score:.2}")),
                r1.score.map_or_else(|| "n/a".to_owned(), |score| format!("{score:.2}")),
                v1.wrong,
                r1.wrong
            ),
            "T7" if *phase == 2 => format!(
                "H4 {task_id}: the second question costs {} less with V1 by the client's estimate, {} fewer tokens (needs >= 40%)",
                percent(match (r1.cost_usd, v1.cost_usd) {
                    (Some(raw), Some(vsift)) => saving(raw, vsift),
                    _ => None,
                }),
                percent(saving(r1.tokens, v1.tokens))
            ),
            "T1" => format!(
                "T1 {task_id}: V1 costs {} times R1 by the client's estimate (over 1.5 means the README says a short clip doesn't need VSift)",
                match (r1.cost_usd, v1.cost_usd) {
                    (Some(raw), Some(vsift)) if raw > 0.0 => format!("{:.2}", vsift / raw),
                    _ => "n/a".to_owned(),
                }
            ),
            _ => continue,
        };
        summary.thresholds.push(line);
    }
    summary
}

/// Reads every record in a folder and writes the summary beside it.
///
/// # Errors
///
/// [`TrialError`] when a record cannot be read or the summary written.
pub fn summarize_folder(records: &Path, output_json: &Path, output_markdown: &Path) -> Result<ValueSummary, TrialError> {
    let mut grades = Vec::new();
    let mut files: Vec<_> = fs::read_dir(records)
        .map_err(|error| TrialError::io_at(records, error))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
        .collect();
    files.sort();
    for file in files {
        let record = read_json(&file)?;
        // The folder may hold other files of the batch (a checkability
        // page's JSON); only value records are summarised.
        if record["schema"] != VALUE_RECORD_SCHEMA {
            continue;
        }
        let grade: ValueGrade = serde_json::from_value(record["grade"].clone())
            .map_err(|error| TrialError::json("a value record's grade", error))?;
        grades.push(grade);
    }
    let summary = summarize(&grades);
    write_json(output_json, &summary)?;
    fs::write(output_markdown, markdown(&summary))
        .map_err(|error| TrialError::io_at(output_markdown, error))?;
    Ok(summary)
}

/// The summary as a page a person reads.
#[must_use]
pub fn markdown(summary: &ValueSummary) -> String {
    let mut out = String::from(
        "# Value measurement: batch summary\n\nComputed by `vsift-agent-trials value summarize`. Nothing here is a public claim; a loss is read by a person against the runs before any hold is declared.\n\n## Cells\n\n| Task | Phase | Arm | Runs | Score | Wrong | Unverifiable | Round trips (without finishing) | Tokens | Cost | Images | Wall / tool / model s | Hit budget | Unsafe | Read |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for cell in &summary.cells {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {:.1} ({:.1}) | {:.0} | {} | {:.1} | {:.0} / {:.0} / {:.0} | {} | {} | {} |\n",
            cell.task_id,
            cell.phase,
            cell.arm,
            cell.runs.len(),
            cell.score.map_or_else(|| "pending".to_owned(), |score| format!("{score:.2}")),
            cell.wrong,
            cell.unverifiable,
            cell.round_trips,
            cell.round_trips_without_finishing,
            cell.tokens,
            cell.cost_usd.map_or_else(|| "n/a".to_owned(), |cost| format!("${cost:.2}")),
            cell.images,
            cell.wall_s,
            cell.tool_s,
            cell.model_s,
            cell.hit_budget,
            cell.unsafe_runs,
            cell.needs_reading
        ));
    }
    out.push_str("\n## Verdicts (ruling 12.2)\n\n| Task | Phase | Pair | Gated | Verdict | Pairs with more wrong |\n| --- | --- | --- | --- | --- | --- |\n");
    for comparison in &summary.comparisons {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {:?} | {} |\n",
            comparison.task,
            comparison.phase,
            comparison.pair,
            if comparison.gated { "yes" } else { "reported" },
            comparison.verdict,
            comparison.pairs_with_more_wrong
        ));
    }
    out.push_str("\n## Thresholds (method section 2, computed, not decided)\n\n");
    for line in &summary.thresholds {
        out.push_str(&format!("- {line}\n"));
    }
    out.push_str(&format!(
        "\n## Excluded runs (ruling 12.1)\n\n{}\n\nPilot runs (never counted): {}\n",
        if summary.excluded.is_empty() {
            "None.".to_owned()
        } else {
            summary
                .excluded
                .iter()
                .map(|line| format!("- {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        },
        summary.pilots
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell_with(score: f64, wrong: &[usize]) -> Cell {
        Cell {
            score: Some(score),
            wrong_by_run: wrong
                .iter()
                .enumerate()
                .map(|(index, count)| (u32::try_from(index + 1).unwrap_or(0), *count))
                .collect(),
            wrong: wrong.iter().sum(),
            ..Cell::default()
        }
    }

    #[test]
    fn equal_within_the_margin_with_no_more_wrong_claims() {
        let raw = cell_with(0.80, &[1, 0, 0]);
        let vsift = cell_with(0.71, &[0, 0, 1]);
        assert_eq!(compare(&vsift, &raw, true).0, Verdict::EqualOrBetter);
    }

    #[test]
    fn a_score_more_than_the_margin_below_is_a_loss_on_a_gated_task_only() {
        let raw = cell_with(0.90, &[0, 0, 0]);
        let vsift = cell_with(0.70, &[0, 0, 0]);
        assert_eq!(compare(&vsift, &raw, true).0, Verdict::Loss);
        assert_eq!(compare(&vsift, &raw, false).0, Verdict::Neither);
    }

    #[test]
    fn more_wrong_claims_in_two_of_three_pairs_is_a_loss_and_in_one_is_not() {
        let raw = cell_with(0.80, &[0, 0, 0]);
        assert_eq!(compare(&cell_with(0.80, &[1, 1, 0]), &raw, true), (Verdict::Loss, 2));
        assert_eq!(compare(&cell_with(0.80, &[3, 0, 0]), &raw, true), (Verdict::Neither, 1));
    }

    #[test]
    fn a_pending_truth_is_not_graded() {
        let raw = Cell::default();
        assert_eq!(compare(&cell_with(1.0, &[0]), &raw, true).0, Verdict::NotGraded);
    }
}
