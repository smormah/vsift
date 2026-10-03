//! `vsift-agent-trials`: install, prepare, run, grade and record named-client
//! agent trials of the `VSift` skill and of a cold agent. See
//! `docs/agents/trials.md`.

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use clap::{Parser, Subcommand, ValueEnum};
use vsift_agent_trials::{
    TrialError,
    campaign::{CampaignState, ClientName, Outcome, Tier},
    cold::{CLAUDE_COLD_SETTINGS, ColdVariant},
    evaluate::{GradeOptions, environment_user_names, grade_phase},
    freeze, holdout,
    install::{DEFAULT_REGISTRY, InstallProof, InstallRequest, install},
    layout::{ToolSource, TrialLayout},
    policy::CommandPolicy,
    prepare::{PrepareRequest, prepare},
    record::write_record,
    roots::RootPolicy,
    run::{RunRequest, run, system_path_directories},
    scenario::Scenario,
    skill::SkillReferences,
    summary::summarize,
    trace::ClientKind,
    truth::CorpusTruth,
};

/// The exit code of `run` when the client stopped at its usage limit: the
/// campaign scripts wait and run the phase again (`EX_TEMPFAIL`).
const EXIT_USAGE_LIMIT: u8 = 75;

/// The exit code of `campaign next` when nothing is left to run.
const EXIT_NOTHING_LEFT: u8 = 3;

/// The exit code of `campaign next` when a blocked run stops the campaign.
const EXIT_BLOCKED: u8 = 4;

#[derive(Parser)]
#[command(
    name = "vsift-agent-trials",
    about = "Named-client agent trials of the VSift skill and of a cold agent (P12, P14)"
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

impl From<Client> for ClientName {
    fn from(client: Client) -> Self {
        match client {
            Client::Claude => Self::Claude,
            Client::Codex => Self::Codex,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Tools {
    /// Executables registered by absolute path with `setup configure`.
    Registered,
    /// `setup plan` and `setup install` by the harness (Ubuntu 24.04 x64).
    Managed,
}

/// Which Claude Code cold settings `prepare` writes (a cold scenario only).
#[derive(Clone, Copy, ValueEnum)]
enum ColdSettings {
    /// `Bash(vsift:*)` only: Claude Code on the maintainer's machine.
    Strict,
    /// Also the read-only helpers: only where the machine is isolated.
    Realistic,
}

#[derive(Clone, Copy, ValueEnum)]
enum TierArgument {
    Review,
    Compact,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutcomeArgument {
    Counted,
    Invalid,
    UsageLimited,
    HarnessError,
}

#[derive(Subcommand)]
enum CampaignStep {
    /// Writes a client's state file for a batch from the plan.
    Init {
        /// The batch, 1 to 3.
        #[arg(long)]
        batch: u8,
        /// The client.
        #[arg(long, value_enum)]
        client: Client,
        /// The published version under test (`0.1.0`, `0.2.0-rc.1`).
        #[arg(long)]
        version: String,
        /// The state file to create.
        #[arg(long)]
        state: PathBuf,
    },
    /// Prints the next run as one JSON line, or says why there is none:
    /// exit 3 when nothing is left, exit 4 when a blocked run stops the
    /// campaign.
    Next {
        /// The state file.
        #[arg(long)]
        state: PathBuf,
    },
    /// Records one attempt at a run.
    Mark {
        /// The state file.
        #[arg(long)]
        state: PathBuf,
        /// The run's identifier.
        #[arg(long)]
        run: String,
        /// The trial's identifier, when one was made.
        #[arg(long)]
        trial: Option<String>,
        /// How the attempt ended.
        #[arg(long, value_enum)]
        outcome: OutcomeArgument,
        /// A short note (never a path or a prompt).
        #[arg(long)]
        note: Option<String>,
    },
    /// Adds a reserve run, under the rule the operator stated before the
    /// batch.
    AddReserve {
        /// The state file.
        #[arg(long)]
        state: PathBuf,
        /// The scenario.
        #[arg(long)]
        scenario: String,
        /// The model tier.
        #[arg(long, value_enum)]
        tier: TierArgument,
    },
    /// Prints where a state file stands.
    Status {
        /// The state file.
        #[arg(long)]
        state: PathBuf,
    },
}

#[derive(Subcommand)]
enum FreezeStep {
    /// Records the digests of the skill, grader, scenarios, settings and
    /// truth at the commit under test.
    Write {
        /// The repository checkout (default: this crate's).
        #[arg(long)]
        repository: Option<PathBuf>,
        /// The commit under test (git rev-parse HEAD).
        #[arg(long)]
        commit: String,
        /// The freeze file to write.
        #[arg(long)]
        output: PathBuf,
    },
    /// Fails if anything frozen has changed.
    Check {
        /// The repository checkout (default: this crate's).
        #[arg(long)]
        repository: Option<PathBuf>,
        /// The freeze file.
        #[arg(long)]
        file: PathBuf,
        /// Only these components (comma separated), for example
        /// `grader,cold,settings,truth` between the cold rounds.
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
    },
}

#[derive(Subcommand)]
enum Step {
    /// Installs the published package from the registry into a fresh prefix
    /// (clean-install mode) and writes the proof that it is the published
    /// one.
    Install {
        /// The exact version (`0.1.0`, `0.2.0-rc.1`); never a tag.
        #[arg(long)]
        version: String,
        /// The npm prefix to create: a new folder under the neutral root.
        #[arg(long)]
        prefix: PathBuf,
        /// Node.js (absolute).
        #[arg(long)]
        node: PathBuf,
        /// npm's npm-cli.js (absolute).
        #[arg(long)]
        npm_cli: PathBuf,
        /// The registry (https, ending in a slash).
        #[arg(long, default_value = DEFAULT_REGISTRY)]
        registry: String,
        /// Where to write the proof.
        #[arg(long)]
        proof: PathBuf,
        /// Harness environment variables to pass npm by name (a proxy).
        #[arg(long = "pass-env")]
        pass_environment: Vec<String>,
    },
    /// Checks an install proof against the install on disk and prints what
    /// it proves.
    VerifyInstall {
        /// The proof file `install` wrote.
        #[arg(long)]
        proof: PathBuf,
    },
    /// Builds a scenario's trial workspace under a neutral root.
    Prepare {
        /// The neutral trial root (for example C:\vsift-trials).
        #[arg(long)]
        root: PathBuf,
        /// The scenario file.
        #[arg(long)]
        scenario: PathBuf,
        /// The vsift executable (absolute). Not used with --install-proof.
        #[arg(long)]
        vsift: Option<PathBuf>,
        /// An install proof (clean-install mode): the published package's
        /// native executable is used, the skill copy comes from its package,
        /// and the agent reaches vsift only through npm's command shim.
        #[arg(long)]
        install_proof: Option<PathBuf>,
        /// How the tools are provided.
        #[arg(long, value_enum, default_value = "registered")]
        tools: Tools,
        /// The Claude Code settings of a cold trial: `strict` (vsift only,
        /// the default) or `realistic` (also read-only helpers), which gives
        /// a cold agent the run of the user's files and so belongs only on
        /// an isolated machine. Ignored for a skill trial.
        #[arg(long, value_enum, default_value = "strict")]
        cold_settings: ColdSettings,
        /// The freeze file the trial runs under.
        #[arg(long)]
        freeze: Option<PathBuf>,
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
    /// Runs one client for one phase of a prepared trial. Exits 75 when the
    /// client stopped at its usage limit.
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
        /// The record file (docs/planning/p14-agent-trials/<batch>/records/<name>.json).
        #[arg(long)]
        output: PathBuf,
        /// The client home to redact from the record.
        #[arg(long)]
        client_home: Option<PathBuf>,
    },
    /// Checks every scenario file (tuning, cold and hold-out) against the
    /// corpus truth and the skill, and the hold-out set's separation.
    CheckScenarios {
        /// The repository checkout (default: this crate's).
        #[arg(long)]
        repository: Option<PathBuf>,
    },
    /// Records or checks what may not change during a batch.
    Freeze {
        #[command(subcommand)]
        step: FreezeStep,
    },
    /// Plans a batch and keeps its state: what to run next, and what
    /// counted.
    Campaign {
        #[command(subcommand)]
        step: CampaignStep,
    },
    /// Summarises a batch: the gates of the plan, results and usage.
    Summarize {
        /// A campaign state file (repeat for each client).
        #[arg(long = "state", required = true)]
        states: Vec<PathBuf>,
        /// The folder of bounded records.
        #[arg(long)]
        records: PathBuf,
        /// Where to write summary.json.
        #[arg(long)]
        output_json: PathBuf,
        /// Where to write SUMMARY.md.
        #[arg(long)]
        output_markdown: PathBuf,
    },
}

fn default_repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn scenario_directories(repository: &Path) -> [PathBuf; 3] {
    let root = repository.join("tools").join("vsift-agent-trials");
    [
        root.join("scenarios"),
        root.join("cold"),
        root.join("holdout"),
    ]
}

fn check_scenarios(repository: &Path) -> Result<usize, TrialError> {
    let truth = CorpusTruth::load(&repository.join("fixtures").join("corpus"))?;
    let references = SkillReferences::load(repository)?;
    let policy: &CommandPolicy = &references.policy;
    let mut count = 0;
    for directory in scenario_directories(repository) {
        for (relative, path) in vsift_agent_trials::skill::files_below(&directory)? {
            if relative == "INDEX.json" {
                continue;
            }
            Scenario::load(&path)?.validate(&truth, policy)?;
            count += 1;
        }
    }
    let problems = holdout::check(repository)?;
    if !problems.is_empty() {
        return Err(TrialError::Invalid(format!(
            "the hold-out set: {}",
            problems.join("; ")
        )));
    }
    Ok(count)
}

/// What a step reports and the exit code it ends with.
struct Done {
    message: String,
    code: u8,
}

impl Done {
    fn ok(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 0,
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "One match over the subcommands keeps each step next to its flags"
)]
async fn execute(step: Step) -> Result<Done, TrialError> {
    match step {
        Step::Install {
            version,
            prefix,
            node,
            npm_cli,
            registry,
            proof,
            pass_environment,
        } => {
            let installed = install(&InstallRequest {
                version,
                prefix,
                node,
                npm_cli,
                registry,
                pass_environment,
                root_policy: RootPolicy::from_environment(),
            })?;
            installed.write(&proof)?;
            let reasons = installed.evidence.not_published_because();
            if !reasons.is_empty() {
                return Err(TrialError::Refused(format!(
                    "the install is not the published package: {}",
                    reasons.join("; ")
                )));
            }
            Ok(Done::ok(format!(
                "installed {} ({})",
                installed.evidence.version, installed.evidence.launcher.version_line
            )))
        }
        Step::VerifyInstall { proof } => {
            let proof = InstallProof::load(&proof)?;
            Ok(Done::ok(format!(
                "published install of vsift-cli {}: {}; integrity of {} package(s) equals the registry's; the native executable matches the launcher's digest ({})",
                proof.evidence.version,
                proof.evidence.launcher.version_line,
                proof.evidence.packages.len(),
                &proof.evidence.launcher.actual_sha256
                    [..12.min(proof.evidence.launcher.actual_sha256.len())]
            )))
        }
        Step::Prepare {
            root,
            scenario,
            vsift,
            install_proof,
            tools,
            cold_settings,
            freeze,
            vsift_commit,
            ffmpeg,
            ffprobe,
            whisper,
            model,
            repository,
        } => {
            let repository = repository.unwrap_or_else(default_repository);
            let proof = install_proof
                .as_deref()
                .map(InstallProof::load)
                .transpose()?;
            let binary = match (&proof, vsift) {
                (Some(_), None) => PathBuf::new(),
                (None, Some(path)) => path,
                (Some(_), Some(_)) => {
                    return Err(TrialError::Refused(
                        "give --install-proof or --vsift, not both".to_owned(),
                    ));
                }
                (None, None) => {
                    return Err(TrialError::Refused(
                        "give --install-proof (clean install) or --vsift (a binary built from the checkout)"
                            .to_owned(),
                    ));
                }
            };
            let freeze_sha256 = match freeze {
                Some(file) => {
                    let recorded = freeze::load(&file)?;
                    let problems = freeze::check(&repository, &file, None)?;
                    if !problems.is_empty() {
                        return Err(TrialError::Refused(format!(
                            "the freeze no longer holds: {}",
                            problems.join("; ")
                        )));
                    }
                    Some(recorded.freeze_sha256)
                }
                None => None,
            };
            let layout = prepare(&PrepareRequest {
                root,
                repository,
                scenario,
                vsift: binary,
                vsift_commit,
                ffmpeg,
                ffprobe,
                whisper,
                model,
                root_policy: RootPolicy::from_environment(),
                install: proof,
                tools: match tools {
                    Tools::Registered => ToolSource::Registered,
                    Tools::Managed => ToolSource::Managed,
                },
                freeze_sha256,
                cold_scan_stop: None,
                cold_variant: match cold_settings {
                    ColdSettings::Strict => ColdVariant::Strict,
                    ColdSettings::Realistic => ColdVariant::Realistic,
                },
            })
            .await?;
            Ok(Done::ok(format!("prepared {}", layout.trial().display())))
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
                cold_scan_stop: None,
            })
            .await?;
            if let Some(limit) = &record.usage_limit {
                return Ok(Done {
                    message: format!(
                        "usage-limit reset={}",
                        limit
                            .reset_unix_s
                            .map_or_else(|| "unknown".to_owned(), |value| value.to_string())
                    ),
                    code: EXIT_USAGE_LIMIT,
                });
            }
            Ok(Done::ok(format!(
                "phase {} ran: exit {:?}, timed out {}, {} ms",
                record.phase, record.exit_code, record.timed_out, record.wall_ms
            )))
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
            let cold = graded.cold.as_ref().map_or_else(String::new, |report| {
                format!(
                    "; cold safety {} ({} violation(s)), usefulness {}",
                    if report.safety.passed {
                        "passed"
                    } else {
                        "FAILED"
                    },
                    report.safety.violations.len(),
                    if report.usefulness.passed {
                        "passed"
                    } else {
                        "failed"
                    }
                )
            });
            Ok(Done::ok(format!(
                "{}mechanical: {} {:?}; interpretation: {}; tool calls {}; images {}{cold}",
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
            )))
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
            Ok(Done::ok(format!("recorded {}", output.display())))
        }
        Step::CheckScenarios { repository } => {
            let count = check_scenarios(&repository.unwrap_or_else(default_repository))?;
            Ok(Done::ok(format!(
                "{count} scenarios are consistent with the truth and the skill (cold settings: {CLAUDE_COLD_SETTINGS})"
            )))
        }
        Step::Freeze { step } => match step {
            FreezeStep::Write {
                repository,
                commit,
                output,
            } => {
                let written = freeze::write(
                    &repository.unwrap_or_else(default_repository),
                    &commit,
                    &output,
                )?;
                Ok(Done::ok(format!("froze {}", written.freeze_sha256)))
            }
            FreezeStep::Check {
                repository,
                file,
                only,
            } => {
                let problems = freeze::check(
                    &repository.unwrap_or_else(default_repository),
                    &file,
                    (!only.is_empty()).then_some(only.as_slice()),
                )?;
                if problems.is_empty() {
                    Ok(Done::ok("nothing frozen has changed"))
                } else {
                    Err(TrialError::Refused(problems.join("; ")))
                }
            }
        },
        Step::Campaign { step } => campaign(step),
        Step::Summarize {
            states,
            records,
            output_json,
            output_markdown,
        } => {
            let states: Vec<CampaignState> = states
                .iter()
                .map(|path| CampaignState::load(path))
                .collect::<Result<_, _>>()?;
            let mut values = Vec::new();
            for (_, path) in vsift_agent_trials::skill::files_below(&records)? {
                if path
                    .extension()
                    .is_some_and(|extension| extension == "json")
                {
                    values.push(vsift_agent_trials::error::read_json(&path)?);
                }
            }
            let summary = summarize(&states, &values);
            vsift_agent_trials::error::write_json(&output_json, &summary)?;
            std::fs::write(&output_markdown, summary.to_markdown())
                .map_err(|error| TrialError::io_at(&output_markdown, error))?;
            Ok(Done::ok(format!(
                "summarised {} counted run(s) against {} gate(s)",
                summary.runs.len(),
                summary.gates.len()
            )))
        }
    }
}

fn campaign(step: CampaignStep) -> Result<Done, TrialError> {
    match step {
        CampaignStep::Init {
            batch,
            client,
            version,
            state,
        } => {
            if state.exists() {
                return Err(TrialError::Refused(
                    "the state file exists; a campaign is planned once".to_owned(),
                ));
            }
            let fresh = CampaignState::new(batch, client.into(), &version)?;
            fresh.save(&state)?;
            Ok(Done::ok(fresh.describe()))
        }
        CampaignStep::Next { state } => {
            let loaded = CampaignState::load(&state)?;
            match loaded.next() {
                Ok(Some(next)) => Ok(Done::ok(
                    serde_json::to_string(&next.run)
                        .map_err(|error| TrialError::json("the next run", error))?,
                )),
                Ok(None) => Ok(Done {
                    message: format!("nothing left: {}", loaded.describe()),
                    code: EXIT_NOTHING_LEFT,
                }),
                Err(blocked) => Ok(Done {
                    message: format!(
                        "blocked: {blocked} failed {} attempts to count; the plan's counts are never reduced silently, so the operator decides",
                        vsift_agent_trials::campaign::MAX_FAILED_ATTEMPTS
                    ),
                    code: EXIT_BLOCKED,
                }),
            }
        }
        CampaignStep::Mark {
            state,
            run,
            trial,
            outcome,
            note,
        } => {
            let mut loaded = CampaignState::load(&state)?;
            let status = loaded.mark(
                &run,
                trial,
                match outcome {
                    OutcomeArgument::Counted => Outcome::Counted,
                    OutcomeArgument::Invalid => Outcome::Invalid,
                    OutcomeArgument::UsageLimited => Outcome::UsageLimited,
                    OutcomeArgument::HarnessError => Outcome::HarnessError,
                },
                note,
            )?;
            loaded.save(&state)?;
            Ok(Done::ok(format!(
                "{run}: {status:?}; {}",
                loaded.describe()
            )))
        }
        CampaignStep::AddReserve {
            state,
            scenario,
            tier,
        } => {
            let mut loaded = CampaignState::load(&state)?;
            let id = loaded.add_reserve(
                &scenario,
                match tier {
                    TierArgument::Review => Tier::Review,
                    TierArgument::Compact => Tier::Compact,
                },
            )?;
            loaded.save(&state)?;
            Ok(Done::ok(format!("added {id}")))
        }
        CampaignStep::Status { state } => Ok(Done::ok(CampaignState::load(&state)?.describe())),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let arguments = Arguments::parse();
    match execute(arguments.command).await {
        Ok(done) => {
            println!("{}", done.message);
            ExitCode::from(done.code)
        }
        Err(error) => {
            eprintln!("vsift-agent-trials: {error}");
            ExitCode::FAILURE
        }
    }
}
