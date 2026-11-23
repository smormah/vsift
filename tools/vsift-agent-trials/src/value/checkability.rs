//! The checkability demonstration (method section 8, hypothesis H6): shown,
//! never scored, and run with no model.
//!
//! It takes one finished V1 run and one finished R1 run of the same task and
//! adds invented statements to each report:
//!
//! - **`VSift` report:** one claim citing an evidence id that is not in the
//!   session, and one citing a real id at a time outside it. Both drafts go
//!   to `vsift handoff check --session <id> --json`, after the unchanged
//!   handoff, and the verdicts are recorded as they come.
//! - **Raw report:** one statement citing a frame file that does not exist,
//!   and one citing a real frame file at a wrong time. The only check a
//!   script can make of a raw report is whether a cited file exists, so the
//!   first can be caught and the second cannot: nothing ties a frame file to
//!   a time but its name.
//!
//! The output is `checkability.md` for the reading, with no local path in
//! it. A case that cannot be built from the runs (no handoff block, no cited
//! frame) is written down as not run, with the reason; nothing is invented to
//! fill it.

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    error::TrialError,
    handoff::extract,
    layout::TrialLayout,
    run::{raw_output, read_run},
    skill::random_hex,
    trace::parse,
    vsift_cli::{VsiftCli, arguments},
};

/// How far the "real id at a wrong time" case moves a citation's time.
const WRONG_TIME_SHIFT_US: i64 = 600_000_000;

/// The first `VSift` case.
const INVENTED_ID: &str = "a claim citing an evidence id that is not in the session";

/// The second `VSift` case.
const WRONG_TIME: &str = "a claim citing a real evidence id at a time outside it";

/// A file name a raw report may cite as its evidence.
static CITED_IMAGE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)[A-Za-z0-9_./\\-]+\.(?:png|jpe?g|webp)").ok());

/// What a check said about an invented statement.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "verdict", content = "detail")]
pub enum CheckVerdict {
    /// The check refused the draft.
    Refused(String),
    /// The check accepted the draft.
    Accepted(String),
    /// The case could not be built from the runs.
    NotRun(String),
}

impl CheckVerdict {
    fn word(&self) -> &'static str {
        match self {
            Self::Refused(_) => "refused",
            Self::Accepted(_) => "accepted",
            Self::NotRun(_) => "not run",
        }
    }

    fn detail(&self) -> &str {
        match self {
            Self::Refused(detail) | Self::Accepted(detail) | Self::NotRun(detail) => detail,
        }
    }
}

/// One invented statement and what its check said.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Case {
    /// `vsift` or `raw`.
    pub report: String,
    /// What was invented.
    pub invented: String,
    /// The check that was run.
    pub check: String,
    /// What it said.
    pub verdict: CheckVerdict,
}

/// The whole demonstration.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Demonstration {
    /// The task both runs answered.
    pub task: String,
    /// The unchanged `VSift` handoff's own verdict, the baseline.
    pub baseline: Option<CheckVerdict>,
    /// The four cases.
    pub cases: Vec<Case>,
}

fn final_message(layout: &TrialLayout) -> Result<String, TrialError> {
    let record = read_run(layout, 1)?;
    let (stdout, _) = raw_output(&record)?;
    parse(record.client, &stdout)?
        .final_message
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| TrialError::Invalid("the run gave no answer".to_owned()))
}

fn handoff_block(handoff: &Value) -> String {
    format!(
        "Report written for the checkability demonstration.\n\n```vsift-handoff\n{handoff:#}\n```\n"
    )
}

/// Runs `vsift handoff check` on a draft; the verdict is `data.valid`.
fn vsift_check(
    cli: &VsiftCli,
    folder: &Path,
    name: &str,
    session: &str,
    handoff: &Value,
) -> Result<CheckVerdict, TrialError> {
    let file = folder.join(name);
    fs::write(&file, handoff_block(handoff)).map_err(|error| TrialError::io_at(&file, error))?;
    let outcome = cli.json(&arguments(&[
        &"handoff",
        &"check",
        &"--file",
        &file,
        &"--session",
        &session,
        &"--json",
    ]))?;
    let findings = outcome.value["data"]["findings"]
        .as_array()
        .map_or(0, Vec::len);
    let detail = format!("exit {:?}, {findings} finding(s)", outcome.code);
    Ok(match outcome.value["data"]["valid"].as_bool() {
        Some(true) => CheckVerdict::Accepted(detail),
        Some(false) => CheckVerdict::Refused(detail),
        None => CheckVerdict::NotRun(format!("the check gave no verdict ({detail})")),
    })
}

/// Adds a claim citing `citation` (given the id `e9xx`) to a copy of the
/// handoff, modelled on its first claim that cites something.
fn with_invented_claim(handoff: &Value, mut citation: Value, id: &str) -> Option<Value> {
    let mut changed = handoff.clone();
    let model = changed["claims"]
        .as_array()?
        .iter()
        .find(|claim| {
            claim["citations"]
                .as_array()
                .is_some_and(|ids| !ids.is_empty())
        })?
        .clone();
    let mut claim = model;
    claim["id"] = json!(format!("c{id}"));
    claim["statement"] = json!("The recording shows the result at this moment.");
    claim["citations"] = json!([id]);
    citation["id"] = json!(id);
    changed["claims"].as_array_mut()?.push(claim);
    changed["citations"].as_array_mut()?.push(citation);
    Some(changed)
}

fn vsift_cases(
    layout: &TrialLayout,
    vsift: &Path,
) -> Result<(Option<CheckVerdict>, Vec<Case>), TrialError> {
    let check = "vsift handoff check --file <draft> --session <session> --json".to_owned();
    let case = |invented: &str, verdict: CheckVerdict| Case {
        report: "vsift".to_owned(),
        invented: invented.to_owned(),
        check: check.clone(),
        verdict,
    };
    let not_run = |why: String| -> Result<(Option<CheckVerdict>, Vec<Case>), TrialError> {
        Ok((
            None,
            vec![
                case(INVENTED_ID, CheckVerdict::NotRun(why.clone())),
                case(WRONG_TIME, CheckVerdict::NotRun(why)),
            ],
        ))
    };
    let message = final_message(layout)?;
    let handoff = match extract(&message) {
        Ok(handoff) => handoff,
        Err(why) => {
            return not_run(format!(
                "the V1 report holds no usable handoff block ({why}); a demonstration run with the handoff wording is needed"
            ));
        }
    };
    let Some(session) = handoff["session"]["session_id"].as_str().map(str::to_owned) else {
        return not_run("the V1 handoff names no session".to_owned());
    };
    let Some(model) = handoff["citations"]
        .as_array()
        .and_then(|citations| {
            citations
                .iter()
                .find(|citation| citation["evidence_id"].is_string())
        })
        .cloned()
    else {
        return not_run("the V1 handoff cites no evidence id".to_owned());
    };
    let folder = layout.harness().join("checkability");
    fs::create_dir_all(&folder).map_err(|error| TrialError::io_at(&folder, error))?;
    let cli = VsiftCli::new(vsift, layout)?;
    let baseline = vsift_check(&cli, &folder, "baseline.md", &session, &handoff)?;

    let mut invented = model.clone();
    invented["evidence_id"] = json!(format!("evd_{}", random_hex(16)?));
    let first = match with_invented_claim(&handoff, invented, "e901") {
        Some(draft) => vsift_check(&cli, &folder, "invented-id.md", &session, &draft)?,
        None => CheckVerdict::NotRun("the V1 handoff has no claim that cites something".to_owned()),
    };

    let second = match model["actual_us"].as_i64() {
        Some(actual) => {
            let mut moved = model.clone();
            moved["actual_us"] = json!(actual.saturating_add(WRONG_TIME_SHIFT_US));
            match with_invented_claim(&handoff, moved, "e902") {
                Some(draft) => vsift_check(&cli, &folder, "wrong-time.md", &session, &draft)?,
                None => CheckVerdict::NotRun(
                    "the V1 handoff has no claim that cites something".to_owned(),
                ),
            }
        }
        None => CheckVerdict::NotRun("the cited evidence carries no actual_us to move".to_owned()),
    };
    Ok((
        Some(baseline),
        vec![case(INVENTED_ID, first), case(WRONG_TIME, second)],
    ))
}

/// The image files a raw report cites, in order, once.
#[must_use]
pub fn cited_images(report: &str) -> Vec<String> {
    let Some(pattern) = CITED_IMAGE.as_ref() else {
        return Vec::new();
    };
    let mut found: Vec<String> = Vec::new();
    for hit in pattern.find_iter(report) {
        let name = hit.as_str().trim_start_matches("./").to_owned();
        if !found.contains(&name) {
            found.push(name);
        }
    }
    found
}

/// The check a script can make of a raw statement: every file it cites
/// exists in the workspace.
#[must_use]
pub fn raw_script_check(statement: &str, workspace: &Path) -> CheckVerdict {
    let cited = cited_images(statement);
    if cited.is_empty() {
        return CheckVerdict::NotRun("the statement cites no file".to_owned());
    }
    let missing: Vec<&String> = cited
        .iter()
        .filter(|name| !workspace.join(name.as_str()).is_file())
        .collect();
    if missing.is_empty() {
        CheckVerdict::Accepted("every cited file exists; nothing ties a file to a time".to_owned())
    } else {
        CheckVerdict::Refused(format!("{} cited file(s) do not exist", missing.len()))
    }
}

fn raw_cases(layout: &TrialLayout) -> Result<Vec<Case>, TrialError> {
    let check = "the cited file exists in the work folder".to_owned();
    let case = |invented: &str, verdict: CheckVerdict| Case {
        report: "raw".to_owned(),
        invented: invented.to_owned(),
        check: check.clone(),
        verdict,
    };
    let workspace = layout.workspace();
    let message = final_message(layout)?;
    let missing = format!("frames/frame-{}.png", random_hex(4)?);
    let first = raw_script_check(
        &format!("At 0:07 the row changes, as frame {missing} shows."),
        &workspace,
    );
    let real = cited_images(&message)
        .into_iter()
        .find(|name| workspace.join(name).is_file());
    let second = match real {
        Some(name) => raw_script_check(
            &format!("At 9:59 the row changes, as frame {name} shows."),
            &workspace,
        ),
        None => CheckVerdict::NotRun("the R1 report cites no frame file that exists".to_owned()),
    };
    Ok(vec![
        case("a statement citing a frame file that does not exist", first),
        case(
            "a statement citing a real frame file at a wrong time",
            second,
        ),
    ])
}

/// Runs the demonstration on a finished V1 trial and R1 trial.
///
/// # Errors
///
/// [`TrialError`] when a trial or its log cannot be read, or `vsift` cannot
/// be started.
pub fn demonstrate(
    v1_trial: &Path,
    r1_trial: &Path,
    vsift: &Path,
) -> Result<Demonstration, TrialError> {
    let v1 = TrialLayout::new(v1_trial);
    let r1 = TrialLayout::new(r1_trial);
    let task = |layout: &TrialLayout| {
        crate::run::read_manifest(layout).map(|manifest| {
            manifest
                .value
                .map_or_else(|| manifest.scenario_id.clone(), |value| value.task_id)
        })
    };
    let (v1_task, r1_task) = (task(&v1)?, task(&r1)?);
    if v1_task != r1_task {
        return Err(TrialError::Invalid(format!(
            "the two runs answered different tasks ({v1_task}, {r1_task})"
        )));
    }
    let (baseline, mut cases) = vsift_cases(&v1, vsift)?;
    cases.extend(raw_cases(&r1)?);
    Ok(Demonstration {
        task: v1_task,
        baseline,
        cases,
    })
}

/// The demonstration as `checkability.md`.
#[must_use]
pub fn markdown(demonstration: &Demonstration) -> String {
    let mut text = format!(
        "# Checkability demonstration, {}\n\nShown, not scored (method section 8). No model ran for this page.\n\n",
        demonstration.task
    );
    if let Some(baseline) = &demonstration.baseline {
        let _ = writeln!(
            text,
            "The unchanged VSift handoff: {} ({}).\n",
            baseline.word(),
            baseline.detail()
        );
    }
    text.push_str(
        "| Report | Invented | Check | Verdict | Detail |\n| --- | --- | --- | --- | --- |\n",
    );
    for case in &demonstration.cases {
        let _ = writeln!(
            text,
            "| {} | {} | `{}` | {} | {} |",
            case.report,
            case.invented,
            case.check,
            case.verdict.word(),
            case.verdict.detail()
        );
    }
    text
}

/// Runs the demonstration and writes `checkability.md` and its JSON beside it.
///
/// # Errors
///
/// As [`demonstrate`], or when an output cannot be written.
pub fn write(
    v1_trial: &Path,
    r1_trial: &Path,
    vsift: &Path,
    output: &Path,
) -> Result<Demonstration, TrialError> {
    let demonstration = demonstrate(v1_trial, r1_trial, vsift)?;
    fs::write(output, markdown(&demonstration))
        .map_err(|error| TrialError::io_at(output, error))?;
    let json_file: PathBuf = output.with_extension("json");
    crate::error::write_json(&json_file, &demonstration)?;
    Ok(demonstration)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cited_images_are_found_once_in_order() {
        let report =
            "At 0:07 (frames/f-007.png) the row turns red; see ./frames/f-007.png and shot_2.JPG.";
        assert_eq!(cited_images(report), vec!["frames/f-007.png", "shot_2.JPG"]);
    }

    #[test]
    fn a_script_catches_a_missing_file_but_not_a_wrong_time() {
        let folder =
            std::env::temp_dir().join(format!("value-checkability-{}", std::process::id()));
        let frames = folder.join("frames");
        assert!(fs::create_dir_all(&frames).is_ok());
        assert!(fs::write(frames.join("real.png"), b"png").is_ok());
        let missing = raw_script_check("At 0:07, frame frames/none.png.", &folder);
        let wrong_time = raw_script_check("At 9:59, frame frames/real.png.", &folder);
        let _ = fs::remove_dir_all(&folder);
        assert_eq!(missing.word(), "refused");
        assert_eq!(wrong_time.word(), "accepted");
        assert_eq!(
            raw_script_check("At 0:07 it changes.", Path::new(".")).word(),
            "not run"
        );
    }

    #[test]
    fn an_invented_claim_cites_only_the_invented_citation() {
        let handoff = json!({
            "claims": [{"id": "c1", "statement": "x", "citations": ["e1"]}],
            "citations": [{"id": "e1", "type": "frame", "evidence_id": "evd_0000000000000001"}]
        });
        let changed = with_invented_claim(
            &handoff,
            json!({"type": "frame", "evidence_id": "evd_ffff"}),
            "e901",
        );
        let changed = changed.unwrap_or_default();
        assert_eq!(changed["claims"][1]["citations"], json!(["e901"]));
        assert_eq!(changed["citations"][1]["id"], json!("e901"));
        assert_eq!(changed["citations"][0], handoff["citations"][0]);
    }
}
