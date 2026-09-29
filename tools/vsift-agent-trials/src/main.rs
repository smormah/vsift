//! `vsift-agent-trials`: prepare, run, grade and record named-client agent
//! trials of the `VSift` skill. See `docs/agents/trials.md`.

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use clap::{Parser, Subcommand, ValueEnum};
use vsift_agent_trials::{
    TrialError,
    evaluate::{GradeOptions, environment_user_names, grade_phase},
    layout::TrialLayout,
    policy::CommandPolicy,
    prepare::{PrepareRequest, prepare},
    record::write_record,
    roots::RootPolicy,
    run::{RunRequest, run, system_path_directories},
    scenario::Scenario,
    skill::SkillReferences,
    trace::ClientKind,
    truth::CorpusTruth,
};

#[derive(Parser)]
#[command(
    name = "vsift-agent-trials",
    about = "Named-client agent trials of the VSift skill (P12)"
)]
struct Arguments {
    #[command(subcommand)]
    command: Step,
}

#[derive(Clone, Copy, ValueEnum)]
enum Client {
    Claude,
    Codex,
}

#[derive(Subcommand)]
enum Step {
    /// Builds a scenario's trial workspace under a neutral root.
    Prepare {
        /// The neutral trial root (for example C:\vsift-trials).
        #[arg(long)]
        root: PathBuf,
        /// The scenario file.
        #[arg(long)]
        scenario: PathBuf,
        /// The vsift executable (absolute).
        #[arg(long)]
        vsift: PathBuf,
        /// The `VSift` commit under test (git rev-parse HEAD).
        #[arg(long)]
        vsift_commit: String,
        /// The ffmpeg executable (absolute).
        #[arg(long)]
        ffmpeg: Option<PathBuf>,
        /// The ffprobe executable (absolute).
        #[arg(long)]
        ffprobe: Option<PathBuf>,
        /// whisper-cli (absolute).
        #[arg(long)]
        whisper: Option<PathBuf>,
        /// The ggml model (absolute).
        #[arg(long)]
        model: Option<PathBuf>,
        /// The repository checkout (default: this crate's).
        #[arg(long)]
        repository: Option<PathBuf>,
    },
    /// Runs one client for one phase of a prepared trial.
    Run {
        /// The trial directory prepare printed.
        #[arg(long)]
        trial: PathBuf,
        /// The phase, from 1.
        #[arg(long, default_value_t = 1)]
        phase: usize,
        /// Which client.
        #[arg(long, value_enum)]
        client: Client,
        /// The client executable (absolute).
        #[arg(long)]
        executable: PathBuf,
        /// The model to pass.
        #[arg(long)]
        model: String,
        /// The signed-in trial configuration directory (`CLAUDE_CONFIG_DIR` or `CODEX_HOME`).
        #[arg(long)]
        client_home: PathBuf,
        /// Wall-clock limit in seconds.
        #[arg(long, default_value_t = 1_500)]
        timeout_s: u64,
        /// Claude Code's --max-turns.
        #[arg(long, default_value_t = 60)]
        max_turns: u32,
        /// Extra PATH directories for the client.
        #[arg(long = "path-dir")]
        path_directories: Vec<PathBuf>,
        /// Harness environment variables to pass through by name.
        #[arg(long = "pass-env")]
        pass_environment: Vec<String>,
        /// Replaces the scenario's prompt for a debug run, which is never a
        /// trial: grade marks it invalid and record refuses it.
        #[arg(long)]
        debug_prompt: Option<String>,
    },
    /// Grades a phase that run finished (again, with the options below).
    Grade {
        /// The trial directory.
        #[arg(long)]
        trial: PathBuf,
        /// The phase, from 1.
        #[arg(long, default_value_t = 1)]
        phase: usize,
        /// The grade's file name in harness/phase-<n>/ (default grade.json);
        /// a re-grade writes beside the original, for example grade-3e.json.
        #[arg(long)]
        output: Option<String>,
        /// A checkout whose skill and corpus truth replace the trial's.
        #[arg(long)]
        repository: Option<PathBuf>,
        /// An amended scenario file of the same scenario.
        #[arg(long)]
        scenario: Option<PathBuf>,
        /// The client home, for run records that did not keep it.
        #[arg(long)]
        client_home: Option<PathBuf>,
    },
    /// Writes a bounded record of a graded phase.
    Record {
        /// The trial directory.
        #[arg(long)]
        trial: PathBuf,
        /// The phase, from 1.
        #[arg(long, default_value_t = 1)]
        phase: usize,
        /// The record file (docs/planning/p12-agent-trials/<name>.json).
        #[arg(long)]
        output: PathBuf,
        /// The client home to redact from the record.
        #[arg(long)]
        client_home: Option<PathBuf>,
    },
    /// Checks every scenario file against the corpus truth and the skill.
    CheckScenarios {
        /// The repository checkout (default: this crate's).
        #[arg(long)]
        repository: Option<PathBuf>,
    },
}

fn default_repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn check_scenarios(repository: &Path) -> Result<usize, TrialError> {
    let truth = CorpusTruth::load(&repository.join("fixtures").join("corpus"))?;
    let references = SkillReferences::load(repository)?;
    let policy: &CommandPolicy = &references.policy;
    let directory = repository
        .join("tools")
        .join("vsift-agent-trials")
        .join("scenarios");
    let mut count = 0;
    for (_, path) in vsift_agent_trials::skill::files_below(&directory)? {
        Scenario::load(&path)?.validate(&truth, policy)?;
        count += 1;
    }
    Ok(count)
}

#[allow(
    clippy::too_many_lines,
    reason = "One match over the subcommands keeps each step next to its flags"
)]
async fn execute(step: Step) -> Result<String, TrialError> {
    match step {
        Step::Prepare {
            root,
            scenario,
            vsift,
            vsift_commit,
            ffmpeg,
            ffprobe,
            whisper,
            model,
            repository,
        } => {
            let layout = prepare(&PrepareRequest {
                root,
                repository: repository.unwrap_or_else(default_repository),
                scenario,
                vsift,
                vsift_commit,
                ffmpeg,
                ffprobe,
                whisper,
                model,
                root_policy: RootPolicy::from_environment(),
            })
            .await?;
            Ok(format!("prepared {}", layout.trial().display()))
        }
        Step::Run {
            trial,
            phase,
            client,
            executable,
            model,
            client_home,
            timeout_s,
            max_turns,
            path_directories,
            pass_environment,
            debug_prompt,
        } => {
            let record = run(&RunRequest {
                trial,
                phase,
                client: match client {
                    Client::Claude => ClientKind::ClaudeCode,
                    Client::Codex => ClientKind::Codex,
                },
                executable,
                model,
                client_home,
                timeout: Duration::from_secs(timeout_s),
                max_turns,
                path_directories,
                pass_environment,
                system_path: system_path_directories(),
                root_policy: RootPolicy::from_environment(),
                debug_prompt,
            })
            .await?;
            Ok(format!(
                "phase {} ran: exit {:?}, timed out {}, {} ms",
                record.phase, record.exit_code, record.timed_out, record.wall_ms
            ))
        }
        Step::Grade {
            trial,
            phase,
            output,
            repository,
            scenario,
            client_home,
        } => {
            let graded = grade_phase(
                &TrialLayout::new(trial),
                phase,
                &GradeOptions {
                    output,
                    repository,
                    scenario,
                    client_home,
                },
            )?;
            let failed: Vec<&str> = graded
                .mechanical
                .checks
                .iter()
                .filter(|check| !check.passed)
                .map(|check| check.name.as_str())
                .collect();
            Ok(format!(
                "{}mechanical: {} {:?}; interpretation: {}; tool calls {}; images {}",
                if graded.is_valid() {
                    ""
                } else {
                    "INVALID TRIAL (see invalid_reasons in grade.json; re-run, do not count); "
                },
                if graded.mechanical.passed {
                    "passed"
                } else {
                    "failed"
                },
                failed,
                if graded.interpretation.passed {
                    "passed"
                } else {
                    "failed"
                },
                graded.usage.tool_calls,
                graded.usage.images_total
            ))
        }
        Step::Record {
            trial,
            phase,
            output,
            client_home,
        } => {
            write_record(
                &TrialLayout::new(trial),
                phase,
                &output,
                client_home.as_deref(),
                &environment_user_names(),
            )?;
            Ok(format!("recorded {}", output.display()))
        }
        Step::CheckScenarios { repository } => {
            let count = check_scenarios(&repository.unwrap_or_else(default_repository))?;
            Ok(format!(
                "{count} scenarios are consistent with the truth and the skill"
            ))
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let arguments = Arguments::parse();
    match execute(arguments.command).await {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("vsift-agent-trials: {error}");
            ExitCode::FAILURE
        }
    }
}
