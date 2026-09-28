//! Grades a phase that `run` finished: resolves the bundle, parses the
//! stream, grades and writes `harness/phase-<n>/grade.json`.

use std::{env, path::Path};

use serde_json::Value;

use crate::{
    bundle::BundleIndex,
    calls::{ReadScope, normalise_path},
    client_warnings::configuration_warnings,
    error::{TrialError, read_json, write_json},
    grade::{Expected, Grade, GradeInput, grade},
    handoff::{PrivateMarkers, extract},
    layout::TrialLayout,
    run::{raw_output, read_manifest, read_run},
    scenario::Scenario,
    skill::{SkillReferences, image_code},
    trace::{self, Trace},
    truth::CorpusTruth,
    vsift_cli::{VsiftCli, arguments},
};

/// The strings a report must never contain: the trial directory in both
/// slash styles and the operating system user name.
#[must_use]
pub fn private_markers(layout: &TrialLayout, user_names: &[String]) -> PrivateMarkers {
    let trial = layout.trial().to_string_lossy().to_lowercase();
    let mut strings = vec![
        trial.clone(),
        trial.replace('\\', "/"),
        normalise_path(layout.trial()),
    ];
    strings.extend(
        user_names
            .iter()
            .filter(|name| name.chars().count() >= 3)
            .map(|name| name.to_lowercase()),
    );
    PrivateMarkers { strings }
}

/// The operating system user names from the environment.
#[must_use]
pub fn environment_user_names() -> Vec<String> {
    ["USERNAME", "USER", "LOGNAME"]
        .iter()
        .filter_map(|name| env::var(name).ok())
        .filter(|name| !name.trim().is_empty())
        .collect()
}

/// Validates the agent's bundle, or retains the session itself when the
/// agent did not, and reads the records.
fn bundle_for(
    layout: &TrialLayout,
    scenario: &Scenario,
    phase: usize,
    cli: &VsiftCli,
    handoff: Option<&Value>,
    deviations: &mut Vec<String>,
) -> Option<BundleIndex> {
    let validate = |directory: &Path, deviations: &mut Vec<String>| match cli.json(&arguments(&[
        &"bundle",
        &"validate",
        &directory,
        &"--json",
    ])) {
        Ok(outcome) if outcome.code == Some(0) => match BundleIndex::read(directory) {
            Ok(index) => Some(index),
            Err(error) => {
                deviations.push(format!("the bundle could not be read: {error}"));
                None
            }
        },
        Ok(outcome) => {
            deviations.push(format!(
                "bundle validate refused the bundle: {}",
                outcome.value["error"]["code"]
            ));
            None
        }
        Err(error) => {
            deviations.push(format!("bundle validate did not run: {error}"));
            None
        }
    };
    if let Some(name) = &scenario.retain_to {
        let directory = layout.workspace().join(format!("{name}-phase-{phase}"));
        if directory.is_dir() {
            return validate(&directory, deviations);
        }
        deviations.push("the agent did not retain the session as asked".to_owned());
    }
    let session = handoff.and_then(|value| value["session"]["session_id"].as_str())?;
    let has_citations = handoff
        .and_then(|value| value["citations"].as_array())
        .is_some_and(|citations| !citations.is_empty());
    if !has_citations {
        return None;
    }
    let directory = layout.phase(phase).join("harness-bundle");
    match cli.json(&arguments(&[
        &"session",
        &"retain",
        &session,
        &"--output",
        &directory,
        &"--json",
    ])) {
        Ok(outcome) if outcome.code == Some(0) => {
            deviations.push("the harness retained the session to resolve the citations".to_owned());
            validate(&directory, deviations)
        }
        Ok(outcome) => {
            deviations.push(format!(
                "the harness could not retain the session: {}",
                outcome.value["error"]["code"]
            ));
            None
        }
        Err(error) => {
            deviations.push(format!("the harness could not retain the session: {error}"));
            None
        }
    }
}

/// What `run` and the client's own output say about a phase, beyond its
/// trace.
#[derive(Clone, Debug, Default)]
pub struct RunFindings {
    /// The client's reports that it ignored part of its configuration
    /// ([`configuration_warnings`]), and harness reasons such as a debug
    /// run; any makes the trial invalid.
    pub invalid_reasons: Vec<String>,
    /// Whether `run` found a value of the client's sign-in file in the
    /// client's output ([`crate::secret_scan`]); fails `no_canary`.
    pub client_secret_found: bool,
}

/// Grades a trace for one phase of a prepared trial and writes the grade.
///
/// # Errors
///
/// [`TrialError`] when the trial's records, the skill or the corpus cannot
/// be read, or the grade cannot be written.
pub fn grade_trace(
    layout: &TrialLayout,
    phase: usize,
    trace: &Trace,
    raw: &str,
    wall_time_s: Option<u64>,
    user_names: &[String],
    findings: RunFindings,
) -> Result<Grade, TrialError> {
    let manifest = read_manifest(layout)?;
    let scenario = Scenario::load(&layout.scenario())?;
    let references = SkillReferences::load(&manifest.repository)?;
    let truth = CorpusTruth::load(&manifest.repository.join("fixtures").join("corpus"))?;
    let cli = VsiftCli::new(&manifest.vsift_executable, layout)?;
    let handoff = trace
        .final_message
        .as_deref()
        .and_then(|message| extract(message).ok());
    let mut deviations = Vec::new();
    let bundle = bundle_for(
        layout,
        &scenario,
        phase,
        &cli,
        handoff.as_ref(),
        &mut deviations,
    );
    let expected = if phase > 1 {
        let previous = read_json(&layout.phase(phase - 1).join("grade.json"))?;
        let session = &previous["handoff"]["session"];
        Expected {
            session_id: session["session_id"].as_str().map(str::to_owned),
            revision_id: session["revision_id"].as_str().map(str::to_owned),
            job_id: None,
            operation_id: None,
        }
    } else {
        Expected {
            session_id: manifest.prepared.session_id.clone(),
            revision_id: manifest.prepared.revision_id.clone(),
            job_id: manifest.prepared.job_id.clone(),
            operation_id: manifest.prepared.operation_id.clone(),
        }
    };
    let code = image_code();
    let graded = grade(&GradeInput {
        scenario: &scenario,
        phase: phase - 1,
        truth: &truth,
        policy: &references.policy,
        limits: references.budgets.limits(scenario.budget),
        schema: &references.schema,
        trace,
        raw_output: raw,
        scope: ReadScope {
            workspace: layout.workspace(),
            skill_directories: layout.skill_directories().to_vec(),
            session_root: layout.session_root(),
        },
        canaries: &manifest.canaries,
        markers: private_markers(layout, user_names),
        bundle: bundle.as_ref(),
        image_code: &code,
        wall_time_s,
        expected,
        deviations,
        client_warnings: findings.invalid_reasons,
        client_secret_found: findings.client_secret_found,
    });
    write_json(&layout.phase(phase).join("grade.json"), &graded)?;
    Ok(graded)
}

/// Why a debug run (`run --debug-prompt`) is never a valid trial.
pub const DEBUG_RUN_REASON: &str =
    "harness: debug run; the operator replaced the scenario's prompt, so this is not a trial";

/// Grades a phase `run` finished.
///
/// # Errors
///
/// As [`grade_trace`], and when the run record or raw logs are missing.
pub fn grade_phase(layout: &TrialLayout, phase: usize) -> Result<Grade, TrialError> {
    let record = read_run(layout, phase)?;
    let (stdout, stderr) = raw_output(&record)?;
    let trace = trace::parse(record.client, &stdout)?;
    let raw = format!("{stdout}\n{stderr}");
    let mut invalid_reasons = configuration_warnings(record.client, &stdout, &stderr);
    if record.debug_prompt {
        invalid_reasons.push(DEBUG_RUN_REASON.to_owned());
    }
    grade_trace(
        layout,
        phase,
        &trace,
        &raw,
        Some(record.wall_ms / 1_000),
        &environment_user_names(),
        RunFindings {
            invalid_reasons,
            client_secret_found: record.client_secret_scan.is_some_and(|scan| scan.found),
        },
    )
}
