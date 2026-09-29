//! Grades a phase that `run` finished: resolves the bundle, parses the
//! stream, grades and writes `harness/phase-<n>/grade.json`.
//!
//! Grading reads only the trial's records and raw logs, never the client,
//! so a trial can be graded again with a changed grader ([`GradeOptions`]):
//! the new grade is written beside the original (`--output grade-3e.json`),
//! with an amended scenario file of the same scenario, a newer checkout's
//! skill and truth, or the client home that run records from before PR 3e
//! did not keep.

use std::{
    env,
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::{
    bundle::BundleIndex,
    calls::{ReadScope, normalise_path},
    client_warnings::configuration_warnings,
    codex_rollout::ImageViews,
    error::{TrialError, read_json, write_json},
    grade::{Expected, Grade, GradeInput, grade},
    handoff::{PrivateMarkers, extract},
    layout::TrialLayout,
    run::{raw_output, read_manifest, read_run},
    scenario::Scenario,
    skill::{SkillReferences, image_code},
    trace::{self, ClientKind, Trace},
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
    if directory.is_dir() {
        // An earlier grading retained it; grading again reads the same
        // bundle instead of retaining a session that may have expired.
        deviations.push(
            "the harness retained the session to resolve the citations (an earlier grading)"
                .to_owned(),
        );
        return validate(&directory, deviations);
    }
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
    /// client's output ([`crate::leak_check`]); fails `no_canary`.
    pub sign_in_value_found: bool,
    /// The client that ran; `None` for the procedure walker.
    pub client: Option<ClientKind>,
    /// The client home, so the client's own spill files are recognised.
    pub client_home: Option<PathBuf>,
    /// Codex's image views from its session rollout.
    pub image_views: Option<ImageViews>,
}

/// How a phase is graded again; the defaults grade a fresh run.
#[derive(Clone, Debug, Default)]
pub struct GradeOptions {
    /// The grade's file name in `harness/phase-<n>/` (default `grade.json`),
    /// so a re-grade never overwrites the original.
    pub output: Option<String>,
    /// A checkout whose skill and corpus truth replace the ones the trial
    /// was prepared from.
    pub repository: Option<PathBuf>,
    /// A scenario file that replaces the one frozen at preparation; it must
    /// have the same identifier.
    pub scenario: Option<PathBuf>,
    /// The client home, for run records written before `run` kept it.
    pub client_home: Option<PathBuf>,
}

/// The default grade file of a phase.
pub const GRADE_FILE: &str = "grade.json";

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
    grade_trace_with(
        layout,
        phase,
        trace,
        raw,
        wall_time_s,
        user_names,
        findings,
        &GradeOptions::default(),
    )
}

/// [`grade_trace`] with re-grading options.
///
/// # Errors
///
/// As [`grade_trace`]; [`TrialError::Invalid`] when a replacement scenario
/// names another scenario, and [`TrialError::Refused`] when the grade file
/// name is not a plain `.json` name.
#[allow(
    clippy::too_many_arguments,
    reason = "The inputs of one grading, kept explicit as in grade_trace"
)]
pub fn grade_trace_with(
    layout: &TrialLayout,
    phase: usize,
    trace: &Trace,
    raw: &str,
    wall_time_s: Option<u64>,
    user_names: &[String],
    findings: RunFindings,
    options: &GradeOptions,
) -> Result<Grade, TrialError> {
    let file = options.output.as_deref().unwrap_or(GRADE_FILE);
    let json_name = Path::new(file)
        .extension()
        .is_some_and(|extension| extension == "json");
    if file.contains(['/', '\\']) || !json_name || file.starts_with('.') {
        return Err(TrialError::Refused(format!(
            "the grade file must be a plain .json file name, not {file:?}"
        )));
    }
    let manifest = read_manifest(layout)?;
    let scenario = Scenario::load(options.scenario.as_deref().unwrap_or(&layout.scenario()))?;
    if scenario.id != manifest.scenario_id {
        return Err(TrialError::Invalid(format!(
            "the replacement scenario is {}, the trial ran {}",
            scenario.id, manifest.scenario_id
        )));
    }
    let repository = options.repository.as_ref().unwrap_or(&manifest.repository);
    let references = SkillReferences::load(repository)?;
    let truth = CorpusTruth::load(&repository.join("fixtures").join("corpus"))?;
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
        client: findings.client.unwrap_or(ClientKind::ProcedureWalker),
        image_views: findings.image_views,
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
            client_home: findings.client_home,
        },
        canaries: &manifest.canaries,
        markers: private_markers(layout, user_names),
        bundle: bundle.as_ref(),
        image_code: &code,
        wall_time_s,
        expected,
        deviations,
        client_warnings: findings.invalid_reasons,
        sign_in_value_found: findings.sign_in_value_found,
    });
    write_json(&layout.phase(phase).join(file), &graded)?;
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
pub fn grade_phase(
    layout: &TrialLayout,
    phase: usize,
    options: &GradeOptions,
) -> Result<Grade, TrialError> {
    let record = read_run(layout, phase)?;
    let (stdout, stderr) = raw_output(&record)?;
    let trace = trace::parse(record.client, &stdout)?;
    let raw = format!("{stdout}\n{stderr}");
    let mut invalid_reasons = configuration_warnings(record.client, &stdout, &stderr);
    if record.debug_prompt {
        invalid_reasons.push(DEBUG_RUN_REASON.to_owned());
    }
    grade_trace_with(
        layout,
        phase,
        &trace,
        &raw,
        Some(record.wall_ms / 1_000),
        &environment_user_names(),
        RunFindings {
            invalid_reasons,
            sign_in_value_found: record.sign_in_leak_check.is_some_and(|scan| scan.found),
            client: Some(record.client),
            client_home: record
                .client_home
                .clone()
                .or_else(|| options.client_home.clone()),
            image_views: record.codex_image_views.clone(),
        },
        options,
    )
}
