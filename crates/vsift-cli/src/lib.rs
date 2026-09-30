//! Agent-friendly command parsing and bounded public presentation for `VSift`.

#![forbid(unsafe_code)]

mod batch;
mod candidates;
mod command;
mod config;
mod evidence;
mod human;
mod job;
mod json_input;
mod output;
mod parse_failure;
mod progress;
mod search;
mod session;
mod setup;
mod signal;
#[cfg(test)]
mod skill_contract;
mod worker;

use std::{ffi::OsString, io, io::Write, path::PathBuf, process::ExitCode, time::Duration};

use clap::{CommandFactory, Parser, error::ErrorKind};
use command::{
    BundleCommand, Cli, Command, EventFormat, ExecutionProfile, JobArguments, JobCommand,
    SetupCommand, TranscriptCommand,
};
use config::{ConfigLayer, EffectiveConfig, HostPolicy};
use human::HumanDetail;
use output::{JsonLines, OutputError, OutputMode, OutputWriter, ProcessExit};
use vsift::{
    Cancellation, DEFAULT_LOCAL_ASR_CHECK_BUDGET, Engine, EngineConfig, EngineError, EnginePorts,
    EvaluatedSetupPlan, ExecutableSelections, FailureCode, HostIsolation, IsolationProfile,
    ProgressObserver, SessionRootError, SessionRootLocation, SetupCheckRequest, SetupPlanRequest,
    UserConfigurationLocation, attest_host_isolation,
};
use vsift_contract::{
    ADMISSION_BUSY_REMEDIATION, ADMISSION_CAPACITY_REMEDIATION, CANDIDATE_CURSOR_REMEDIATION,
    CommandName, ConfiguredModelResponse, ConfiguredSelectionResponse,
    DURABILITY_UNAVAILABLE_REMEDIATION, EvidenceStream, IDEMPOTENCY_CONFLICT_REMEDIATION,
    ISOLATION_UNAVAILABLE_REMEDIATION, JOB_BUSY_REMEDIATION, JOB_CANCELLED_REMEDIATION,
    JOB_INTERRUPTED_REMEDIATION, JOB_NOT_RESUMABLE_REMEDIATION, JOB_SESSION_NOT_OPEN_REMEDIATION,
    LOCAL_ASR_MODEL_REMEDIATION, LOCAL_ASR_TOOLS_REMEDIATION,
    MEDIA_TOOLS_FOR_TRANSCRIPT_REMEDIATION, NO_AUDIO_STREAM_REMEDIATION, NO_TRANSCRIPT_REMEDIATION,
    NO_VIDEO_STREAM_REMEDIATION, OperationResponse, SUPERSEDED_REMEDIATION, TerminalEventResponse,
    UNKNOWN_JOB_REMEDIATION, UNKNOWN_REVISION_REMEDIATION, UNPINNED_MODEL_REMEDIATION,
    VISUAL_TOOLS_REMEDIATION, WORKSPACE_NOT_DURABLE_REMEDIATION,
    WORKSPACE_POLICY_MISMATCH_REMEDIATION, WORKSPACE_ROOT_REMEDIATION, local_asr_failure_summary,
    local_asr_verification_summary, media_tool_verification_summary, non_private_folder_summary,
    search_query_rejection_summary, transcript_rejection_summary,
};

/// Parses the process arguments, executes one command, and returns its documented exit status.
///
/// Long commands trap console interruptions (see the `signal` module): the
/// first cancels the command, the second escalates the stop of its provider
/// processes, and the process ends only after its command has returned.
pub async fn run() -> ExitCode {
    let status = execute_with(
        std::env::args_os(),
        io::stdout().lock(),
        io::stderr().lock(),
        EnginePorts::system(),
        Interruption::Trapped,
    )
    .await;
    ExitCode::from(status.code())
}

/// Whether long commands trap console interruptions. Only the process entry
/// point traps them; in-process tests keep the default, so a test runner's
/// own Ctrl-C still ends it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Interruption {
    /// `SIGINT`/`SIGTERM` or Ctrl-C/Ctrl-Break cancel the running command.
    Trapped,
    /// Nothing is trapped (in-process tests only).
    #[cfg(test)]
    Default,
}

/// Whether `command` runs long enough (copying, decoding, recognising) that
/// an interruption must cancel it rather than end the process.
const fn is_long_running(command: &Command) -> bool {
    match command {
        Command::Ingest(_)
        | Command::Candidates(_)
        | Command::Frame(_)
        | Command::Crop(_)
        | Command::Audio(_) => true,
        Command::Transcript(arguments) => {
            matches!(arguments.command, TranscriptCommand::Retranscribe(_))
        }
        Command::Job(arguments) => {
            matches!(
                arguments.command,
                JobCommand::Resume(_) | JobCommand::Run(_) | JobCommand::Batch(_)
            )
        }
        Command::Setup(_) | Command::Session(_) | Command::Search(_) | Command::Bundle(_) => false,
    }
}

/// Builds the engine a command-line invocation uses.
///
/// The CLI is a local, per-user host: sessions and dependency selections live in
/// the platform's per-user locations unless `--session-root` selects another
/// root. It claims strict worker isolation only as `isolation` says, which
/// is the result of an attestation (see [`attest_host_isolation`]), never a
/// flag taken at its word.
fn compose_engine(
    session_root: Option<PathBuf>,
    isolation: HostIsolation,
    ports: EnginePorts,
) -> Engine {
    Engine::new(
        EngineConfig {
            session_root: session_root.map_or(
                SessionRootLocation::PlatformDefault,
                SessionRootLocation::Explicit,
            ),
            user_configuration: UserConfigurationLocation::PlatformDefault,
            host_isolation: isolation,
        },
        ports,
    )
}

/// The public operation a parsed command performs; `None` for bare `setup`,
/// which only prints help.
const fn operation_name(command: &Command) -> Option<CommandName> {
    Some(match command {
        Command::Setup(arguments) => match &arguments.command {
            Some(setup) => setup.operation_name(),
            None => return None,
        },
        Command::Ingest(_) => CommandName::Ingest,
        Command::Session(arguments) => arguments.command.operation_name(),
        Command::Transcript(arguments) => arguments.command.operation_name(),
        Command::Search(_) => CommandName::Search,
        Command::Candidates(_) => CommandName::Candidates,
        Command::Frame(arguments) => arguments.command.operation_name(),
        Command::Audio(_) => CommandName::Audio,
        Command::Crop(_) => CommandName::Crop,
        Command::Bundle(arguments) => arguments.command.operation_name(),
        Command::Job(arguments) => arguments.command.operation_name(),
    })
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep top-level command composition in one exhaustive dispatch"
)]
async fn execute_with<Arguments, Argument, StandardOutput, StandardError>(
    arguments: Arguments,
    standard_output: StandardOutput,
    standard_error: StandardError,
    ports: EnginePorts,
    interruption: Interruption,
) -> ProcessExit
where
    Arguments: IntoIterator<Item = Argument>,
    Argument: Into<OsString> + Clone,
    StandardOutput: Write,
    StandardError: Write,
{
    let arguments: Vec<OsString> = arguments.into_iter().map(Into::into).collect();
    let requested_mode = detect_requested_mode(&arguments);
    let mut writer = OutputWriter::new(standard_output, standard_error);
    let cli = match Cli::try_parse_from(&arguments) {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            return write_help_or_version(&mut writer, &error.to_string());
        }
        Err(error) => {
            // Every mode carries the typed remediation, which never repeats
            // argument text (L-071). Human mode also keeps the parser's own
            // explanation, which quotes the argument: it is quoted on
            // stderr in its display form, labelled untrusted.
            let failure = parse_failure::parse_failure(&error, &arguments);
            let explanation = parse_failure::parser_explanation(&error);
            return write_failure_with_detail(
                &mut writer,
                requested_mode,
                CommandName::Parse,
                failure,
                Some(HumanDetail::Parser(&explanation)),
            );
        }
    };

    let json_lines = cli.events == Some(EventFormat::Jsonl);
    let Ok(mode) = OutputMode::resolve(cli.json, json_lines) else {
        return write_command_failure(
            &mut writer,
            OutputMode::Json,
            CommandName::Parse,
            parse_failure::output_mode_conflict(&arguments),
        );
    };

    let Some(command) = cli.command else {
        return write_root_help(&mut writer);
    };
    // A strict boundary is attested before anything else runs: a host that
    // cannot provide it stops here with `ISOLATION_UNAVAILABLE` (exit 2).
    let isolation = match operation_name(&command) {
        Some(operation) => {
            match attest_host_isolation(IsolationProfile::from(cli.host_isolation)) {
                Ok(isolation) => isolation,
                Err(error) => {
                    return write_command_failure(
                        &mut writer,
                        mode,
                        operation,
                        CommandFailure::from(error),
                    );
                }
            }
        }
        None => HostIsolation::ProcessOnly,
    };
    let engine = compose_engine(cli.session_root, isolation, ports);
    // One cancellation for the whole command; registered before any work
    // starts, so an early interruption is not missed. A handler the system
    // refuses leaves the default behaviour, which commits nothing partial.
    let cancellation = Cancellation::new();
    // `job run` and `job batch` are worker hosts: their shutdown stops
    // admission first and drains for the operator's time (ADR 0021 D4).
    let shutdown = match &command {
        Command::Job(JobArguments {
            command: JobCommand::Run(arguments),
        }) => Some(signal::Shutdown::new(Duration::from_millis(
            arguments.drain_timeout_ms,
        ))),
        Command::Job(JobArguments {
            command: JobCommand::Batch(arguments),
        }) => Some(signal::Shutdown::new(Duration::from_millis(
            arguments.drain_timeout_ms,
        ))),
        _ => None,
    };
    let _interrupts = (interruption == Interruption::Trapped && is_long_running(&command))
        .then(|| match &shutdown {
            Some(shutdown) => signal::listen_for_shutdown(shutdown.clone()).ok(),
            None => signal::listen(cancellation.clone()).ok(),
        })
        .flatten();
    match command {
        Command::Setup(arguments) => match arguments.command {
            Some(SetupCommand::Check(arguments)) => {
                let explicit = ConfigLayer {
                    profile: arguments.profile,
                    probe_timeout_seconds: arguments.timeout_seconds,
                };
                let config = match EffectiveConfig::resolve(
                    explicit,
                    ConfigLayer::default(),
                    ConfigLayer::default(),
                    &HostPolicy::local_r0(),
                ) {
                    Ok(config) => config,
                    Err(error) => {
                        return write_failure_with_detail(
                            &mut writer,
                            mode,
                            CommandName::SetupCheck,
                            CommandFailure::from(FailureCode::InvalidArgument),
                            Some(HumanDetail::Reason(error.reason())),
                        );
                    }
                };
                let request = SetupCheckRequest {
                    probe_timeout: config.probe_timeout,
                    per_call: ExecutableSelections {
                        ffmpeg: arguments.ffmpeg,
                        ffprobe: arguments.ffprobe,
                        whisper: arguments.whisper,
                    },
                    local_asr_budget: DEFAULT_LOCAL_ASR_CHECK_BUDGET,
                };
                let report = match engine.check_setup(request).await {
                    Ok(report) => report,
                    Err(error) => {
                        return write_command_failure(
                            &mut writer,
                            mode,
                            CommandName::SetupCheck,
                            CommandFailure::from(error),
                        );
                    }
                };
                setup::present_setup_check(
                    report.diagnosis(),
                    *report.local_asr(),
                    config.profile,
                    mode,
                    |dependency| report.lookup(dependency),
                    &mut writer,
                )
            }
            Some(SetupCommand::Configure(arguments)) => {
                let dependency = arguments.dependency.into();
                let result = engine
                    .configure_executable(dependency, &arguments.executable)
                    .map_err(CommandFailure::from)
                    .and_then(|()| {
                        complete(
                            CommandName::SetupConfigure,
                            &ConfiguredSelectionResponse::new(dependency),
                        )
                        .map_err(CommandFailure::from)
                    });
                write_session_result(&mut writer, mode, CommandName::SetupConfigure, result)
            }
            Some(SetupCommand::ConfigureModel(arguments)) => {
                let result = engine
                    .configure_model(&arguments.file)
                    .map_err(CommandFailure::from)
                    .and_then(|()| {
                        complete(
                            CommandName::SetupConfigureModel,
                            &ConfiguredModelResponse::new(),
                        )
                        .map_err(CommandFailure::from)
                    });
                write_session_result(&mut writer, mode, CommandName::SetupConfigureModel, result)
            }
            Some(SetupCommand::Plan(arguments)) => {
                let result = current_setup_plan(&engine, arguments.profile)
                    .await
                    .and_then(|plan| {
                        complete(CommandName::SetupPlan, plan.presentation())
                            .map_err(CommandFailure::from)
                    });
                write_session_result(&mut writer, mode, CommandName::SetupPlan, result)
            }
            Some(SetupCommand::Install(arguments)) => {
                let saved = match setup::read_saved_plan(&arguments.plan) {
                    Ok(saved) => saved,
                    Err(FailureCode::StorageIo) => {
                        return write_failure(
                            &mut writer,
                            mode,
                            CommandName::SetupInstall,
                            FailureCode::CommandNotImplemented,
                            None,
                        );
                    }
                    Err(code) => {
                        return write_failure(
                            &mut writer,
                            mode,
                            CommandName::SetupInstall,
                            code,
                            None,
                        );
                    }
                };
                let profile = match saved.profile() {
                    Ok(profile) => ExecutionProfile::from(profile),
                    Err(code) => {
                        return write_failure(
                            &mut writer,
                            mode,
                            CommandName::SetupInstall,
                            code,
                            None,
                        );
                    }
                };
                let current = match current_setup_plan(&engine, profile).await {
                    Ok(current) => current,
                    Err(failure) => {
                        return write_command_failure(
                            &mut writer,
                            mode,
                            CommandName::SetupInstall,
                            failure,
                        );
                    }
                };
                if let Err(error) = current.validate_acceptance(&saved, &arguments.accept_plan) {
                    return write_failure(
                        &mut writer,
                        mode,
                        CommandName::SetupInstall,
                        error.failure_code(),
                        None,
                    );
                }
                write_failure(
                    &mut writer,
                    mode,
                    CommandName::SetupInstall,
                    FailureCode::CommandNotImplemented,
                    None,
                )
            }
            None => write_setup_help(&mut writer),
            Some(
                reserved @ (SetupCommand::Repair(_)
                | SetupCommand::List
                | SetupCommand::Remove(_)
                | SetupCommand::Rollback(_)),
            ) => not_implemented(&mut writer, mode, reserved.operation_name()),
        },
        Command::Ingest(arguments) => {
            let result = session::ingest(&engine, arguments, &cancellation).await;
            write_session_result(&mut writer, mode, CommandName::Ingest, result)
        }
        Command::Transcript(arguments) => {
            let operation = arguments.command.operation_name();
            match arguments.command {
                TranscriptCommand::Get(arguments) if mode == OutputMode::JsonLines => {
                    let result = session::transcript_stream(&engine, arguments);
                    write_evidence_stream(&mut writer, operation, result)
                }
                TranscriptCommand::Get(arguments) => {
                    let result = session::transcript_get(&engine, arguments);
                    write_session_result(&mut writer, mode, operation, result)
                }
                TranscriptCommand::Retranscribe(arguments) if mode == OutputMode::JsonLines => {
                    progress::stream_with_progress(&mut writer, operation, |observer| {
                        session::retranscribe(&engine, arguments, &cancellation, observer)
                    })
                    .await
                }
                TranscriptCommand::Retranscribe(arguments) => {
                    let result = session::retranscribe(
                        &engine,
                        arguments,
                        &cancellation,
                        ProgressObserver::none(),
                    )
                    .await;
                    write_session_result(&mut writer, mode, operation, result)
                }
            }
        }
        Command::Session(arguments) => {
            let operation = arguments.command.operation_name();
            let result = session::execute_session(&engine, arguments.command);
            write_session_result(&mut writer, mode, operation, result)
        }
        Command::Bundle(arguments) => {
            let operation = arguments.command.operation_name();
            match arguments.command {
                BundleCommand::Validate(arguments) => {
                    let result = session::validate_bundle(&engine, &arguments.directory);
                    write_session_result(&mut writer, mode, operation, result)
                }
            }
        }
        Command::Search(arguments) if mode == OutputMode::JsonLines => {
            let result = search::search_stream(&engine, arguments);
            write_evidence_stream(&mut writer, CommandName::Search, result)
        }
        Command::Search(arguments) => {
            let result = search::search(&engine, arguments);
            write_session_result(&mut writer, mode, CommandName::Search, result)
        }
        Command::Candidates(arguments) if mode == OutputMode::JsonLines => {
            let result = candidates::candidates_stream(&engine, arguments, &cancellation).await;
            write_evidence_stream(&mut writer, CommandName::Candidates, result)
        }
        Command::Candidates(arguments) => {
            let result = candidates::candidates(&engine, arguments, &cancellation).await;
            write_session_result(&mut writer, mode, CommandName::Candidates, result)
        }
        Command::Frame(arguments) if mode == OutputMode::JsonLines => {
            let operation = arguments.command.operation_name();
            let result = evidence::frame_stream(&engine, arguments.command, &cancellation).await;
            write_evidence_stream(&mut writer, operation, result)
        }
        Command::Frame(arguments) => {
            let operation = arguments.command.operation_name();
            let result = evidence::frame(&engine, arguments.command, &cancellation).await;
            write_session_result(&mut writer, mode, operation, result)
        }
        Command::Crop(arguments) if mode == OutputMode::JsonLines => {
            let result = evidence::crop_stream(&engine, arguments, &cancellation).await;
            write_evidence_stream(&mut writer, CommandName::Crop, result)
        }
        Command::Crop(arguments) => {
            let result = evidence::crop(&engine, arguments, &cancellation).await;
            write_session_result(&mut writer, mode, CommandName::Crop, result)
        }
        Command::Audio(arguments) if mode == OutputMode::JsonLines => {
            let result = evidence::audio_stream(&engine, arguments, &cancellation).await;
            write_evidence_stream(&mut writer, CommandName::Audio, result)
        }
        Command::Audio(arguments) => {
            let result = evidence::audio(&engine, arguments, &cancellation).await;
            write_session_result(&mut writer, mode, CommandName::Audio, result)
        }
        Command::Job(arguments) => {
            let operation = arguments.command.operation_name();
            let result = match &arguments.command {
                JobCommand::Status(arguments) => job::status(&engine, arguments),
                JobCommand::Cancel(arguments) => job::cancel(&engine, arguments),
                JobCommand::Resume(arguments) if mode == OutputMode::JsonLines => {
                    return progress::stream_with_progress(&mut writer, operation, |observer| {
                        job::resume(&engine, arguments, &cancellation, observer)
                    })
                    .await;
                }
                JobCommand::Resume(arguments) => {
                    job::resume(&engine, arguments, &cancellation, ProgressObserver::none()).await
                }
                JobCommand::Run(arguments) => {
                    let shutdown = shutdown.unwrap_or_else(|| {
                        signal::Shutdown::new(Duration::from_millis(arguments.drain_timeout_ms))
                    });
                    return worker::execute(&engine, arguments, &shutdown, mode, &mut writer).await;
                }
                JobCommand::Batch(arguments) => {
                    let shutdown = shutdown.unwrap_or_else(|| {
                        signal::Shutdown::new(Duration::from_millis(arguments.drain_timeout_ms))
                    });
                    return batch::execute(engine, arguments, &shutdown, mode, &mut writer).await;
                }
            };
            write_session_result(&mut writer, mode, operation, result)
        }
    }
}

/// Answers a reserved command whose implementation packet is incomplete.
fn not_implemented<StandardOutput, StandardError>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    mode: OutputMode,
    command: CommandName,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
{
    write_failure(
        writer,
        mode,
        command,
        FailureCode::CommandNotImplemented,
        None,
    )
}

/// Resolves the effective probe deadline for `profile`, then asks the engine
/// for the current plan.
async fn current_setup_plan(
    engine: &Engine,
    profile: ExecutionProfile,
) -> Result<EvaluatedSetupPlan, CommandFailure> {
    let config = EffectiveConfig::resolve(
        ConfigLayer {
            profile: Some(profile),
            probe_timeout_seconds: None,
        },
        ConfigLayer::default(),
        ConfigLayer::default(),
        &HostPolicy::local_r0(),
    )
    .map_err(|_| FailureCode::InvalidArgument)?;
    Ok(engine
        .plan_setup(SetupPlanRequest {
            profile: config.profile.into(),
            probe_timeout: config.probe_timeout,
        })
        .await?)
}

fn complete<T: serde::Serialize>(
    command: CommandName,
    data: &T,
) -> Result<OperationResponse<serde_json::Value>, FailureCode> {
    OperationResponse::complete(command.identifier(), data).map_err(|_| FailureCode::Internal)
}

/// A failed command's public code and, when a typed cause allows them, a
/// fixed-prose remediation, a retry hint and the identifiers it concerns.
#[derive(Debug)]
pub(crate) struct CommandFailure {
    code: FailureCode,
    remediation: Option<String>,
    retry_after_ms: Option<u64>,
    affected_ids: Vec<String>,
    /// Arguments of a `vsift` command the remediation suggests, when a
    /// typed cause names one (`job resume <job>` after an interruption).
    suggested_command: Vec<String>,
}

impl CommandFailure {
    /// A failure with a fixed-prose remediation and nothing else.
    pub(crate) const fn with_remediation(code: FailureCode, summary: String) -> Self {
        Self {
            code,
            remediation: Some(summary),
            retry_after_ms: None,
            affected_ids: Vec::new(),
            suggested_command: Vec::new(),
        }
    }
}

impl CommandFailure {
    /// A failure whose fixed-prose remediation also suggests one `vsift`
    /// command, given as fixed words and validated identifiers only.
    pub(crate) const fn with_suggested_command(
        code: FailureCode,
        summary: String,
        suggested_command: Vec<String>,
    ) -> Self {
        Self {
            code,
            remediation: Some(summary),
            retry_after_ms: None,
            affected_ids: Vec::new(),
            suggested_command,
        }
    }

    /// The remediation summary, for tests of the failures built here.
    #[cfg(test)]
    pub(crate) fn summary(&self) -> Option<&str> {
        self.remediation.as_deref()
    }
}

impl From<FailureCode> for CommandFailure {
    fn from(code: FailureCode) -> Self {
        Self {
            code,
            remediation: None,
            retry_after_ms: None,
            affected_ids: Vec::new(),
            suggested_command: Vec::new(),
        }
    }
}

impl From<EngineError> for CommandFailure {
    fn from(error: EngineError) -> Self {
        let remediation = error
            .transcript_rejection()
            .map(transcript_rejection_summary)
            .or_else(|| {
                error
                    .missing_media_tool()
                    .map(|_| MEDIA_TOOLS_FOR_TRANSCRIPT_REMEDIATION.to_owned())
            })
            .or_else(|| {
                error
                    .media_tool_verification_failure()
                    .map(media_tool_verification_summary)
            })
            .or_else(|| error.non_private_folder().map(non_private_folder_summary))
            .or_else(|| {
                error
                    .search_query_rejection()
                    .map(search_query_rejection_summary)
            })
            .or_else(|| local_asr_remediation(&error));
        // The session first, then the job: the order an agent resolves them.
        let affected_ids = error
            .affected_session()
            .map(|session| session.as_str().to_owned())
            .into_iter()
            .chain(error.affected_job().map(|job| job.as_str().to_owned()))
            .collect();
        let suggested_command = match &error {
            EngineError::JobInterrupted { job, .. } => {
                vec![
                    "job".to_owned(),
                    "resume".to_owned(),
                    job.as_str().to_owned(),
                ]
            }
            _ => Vec::new(),
        };
        Self {
            code: error.failure_code(),
            remediation,
            retry_after_ms: error.retry_after_ms(),
            affected_ids,
            suggested_command,
        }
    }
}

/// Fixed-prose remediation for local speech recognition, transcript reads
/// and visual candidates; never a path, provider output or evidence text.
fn local_asr_remediation(error: &EngineError) -> Option<String> {
    match error {
        EngineError::LocalAsrToolUnavailable(_) => Some(LOCAL_ASR_TOOLS_REMEDIATION.to_owned()),
        EngineError::ModelNotSelected | EngineError::LocalAsrModelUnavailable => {
            Some(LOCAL_ASR_MODEL_REMEDIATION.to_owned())
        }
        EngineError::LocalAsrModelNotPinned => Some(UNPINNED_MODEL_REMEDIATION.to_owned()),
        EngineError::LocalAsrVerificationFailed(failure) => {
            Some(local_asr_verification_summary(*failure))
        }
        EngineError::LocalAsrFailed(failure) => Some(local_asr_failure_summary(*failure)),
        EngineError::NoAudioStream => Some(NO_AUDIO_STREAM_REMEDIATION.to_owned()),
        EngineError::TranscriptUnavailable => Some(NO_TRANSCRIPT_REMEDIATION.to_owned()),
        EngineError::TranscriptRevisionNotFound => Some(UNKNOWN_REVISION_REMEDIATION.to_owned()),
        EngineError::VisualToolUnavailable(_) => Some(VISUAL_TOOLS_REMEDIATION.to_owned()),
        EngineError::NoVideoStream => Some(NO_VIDEO_STREAM_REMEDIATION.to_owned()),
        EngineError::CandidateCursorWithoutIndex => Some(CANDIDATE_CURSOR_REMEDIATION.to_owned()),
        EngineError::JobBusy { .. } => Some(JOB_BUSY_REMEDIATION.to_owned()),
        EngineError::IdempotencyConflict { .. } => {
            Some(IDEMPOTENCY_CONFLICT_REMEDIATION.to_owned())
        }
        EngineError::RetranscriptionSuperseded { .. } => Some(SUPERSEDED_REMEDIATION.to_owned()),
        EngineError::JobInterrupted { .. } => Some(JOB_INTERRUPTED_REMEDIATION.to_owned()),
        EngineError::JobSessionNotOpen { .. } => Some(JOB_SESSION_NOT_OPEN_REMEDIATION.to_owned()),
        EngineError::JobNotFound => Some(UNKNOWN_JOB_REMEDIATION.to_owned()),
        EngineError::JobNotResumable { .. } => Some(JOB_NOT_RESUMABLE_REMEDIATION.to_owned()),
        EngineError::JobCancelled { .. } => Some(JOB_CANCELLED_REMEDIATION.to_owned()),
        other => worker_remediation(other),
    }
}

/// Fixed-prose remediation for worker workspaces, admission and isolation
/// (P11).
fn worker_remediation(error: &EngineError) -> Option<String> {
    let summary = match error {
        EngineError::SessionRoot(SessionRootError::WorkspacePolicyMismatch) => {
            WORKSPACE_POLICY_MISMATCH_REMEDIATION
        }
        EngineError::SessionRoot(SessionRootError::DurabilityUnavailable) => {
            DURABILITY_UNAVAILABLE_REMEDIATION
        }
        EngineError::SessionRoot(SessionRootError::WorkspaceRootNotExplicit) => {
            WORKSPACE_ROOT_REMEDIATION
        }
        EngineError::WorkspaceNotDurable => WORKSPACE_NOT_DURABLE_REMEDIATION,
        EngineError::AdmissionExceedsCapacity { .. } => ADMISSION_CAPACITY_REMEDIATION,
        EngineError::AdmissionBusy { .. } => ADMISSION_BUSY_REMEDIATION,
        EngineError::IsolationUnavailable(_) => ISOLATION_UNAVAILABLE_REMEDIATION,
        EngineError::Worker(failure) => worker::worker_failure_remediation(*failure),
        _ => return None,
    };
    Some(summary.to_owned())
}

fn write_session_result<StandardOutput, StandardError, Failure>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    mode: OutputMode,
    command: CommandName,
    result: Result<OperationResponse<serde_json::Value>, Failure>,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
    Failure: Into<CommandFailure>,
{
    let response = match result {
        Ok(response) => response,
        Err(failure) => return write_command_failure(writer, mode, command, failure.into()),
    };
    let write = match mode {
        OutputMode::Json => writer.write_json(&response),
        OutputMode::JsonLines => writer.write_json(&TerminalEventResponse::new(response)),
        OutputMode::Human => match human::result(command, &response) {
            Ok(Some(text)) => writer.write_rendered_stdout(&text),
            // Every command that completes has a renderer since P13 PR 2b;
            // a result without one, or without its published shape, is a
            // defect, and is never printed as raw JSON.
            Ok(None) | Err(OutputError::Serialization(_)) => {
                return write_failure(writer, mode, command, FailureCode::Internal, None);
            }
            Err(error) => Err(error),
        },
    };
    match write {
        Ok(()) => ProcessExit::Success,
        Err(error) => {
            writer.write_safe_diagnostic(&error.to_string());
            ProcessExit::StorageOrIo
        }
    }
}

/// Writes a worker host's final response in human mode (P13 PR 2b): its job
/// result or batch summary on stdout, also for a failed or cancelled
/// request, then its error on stderr as every human failure is written.
/// The host's exit status is its own; this answers only whether the text
/// was written.
///
/// # Errors
///
/// [`OutputError::Serialization`] when the response does not have its
/// published shape (a defect), [`OutputError::TooLarge`] when its text
/// exceeds the result budget, and [`OutputError::Io`] when stdout fails.
fn write_human_host<StandardOutput, StandardError>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    command: CommandName,
    response: &OperationResponse<serde_json::Value>,
) -> Result<(), OutputError>
where
    StandardOutput: Write,
    StandardError: Write,
{
    let text = human::host(command, response)?;
    if let Some(result) = &text.result {
        writer.write_rendered_stdout(result)?;
    }
    if let Some(failure) = &text.failure {
        writer.write_rendered_stderr(failure);
    }
    Ok(())
}

/// The exit of a worker host whose final response could not be written. A
/// response without its published shape is a defect: the `INTERNAL` failure
/// is written instead (never the decoder's message, which may quote the
/// response), exit 1. Anything else is an output failure, exit 7.
fn host_write_failure<StandardOutput, StandardError>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    command: CommandName,
    error: &OutputError,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
{
    match error {
        OutputError::Serialization(_) => write_failure(
            writer,
            OutputMode::Human,
            command,
            FailureCode::Internal,
            None,
        ),
        OutputError::TooLarge | OutputError::Io(_) => {
            writer.write_safe_diagnostic(&error.to_string());
            ProcessExit::StorageOrIo
        }
    }
}

/// Writes an evidence stream in `--events jsonl` mode: its evidence events in
/// order, then its terminal event. A failure before the stream exists is the
/// usual single terminal failure event.
fn write_evidence_stream<StandardOutput, StandardError, Stream>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    command: CommandName,
    result: Result<Stream, CommandFailure>,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
    Stream: EvidenceStream,
{
    let stream = match result {
        Ok(stream) => stream,
        Err(failure) => {
            return write_command_failure(writer, OutputMode::JsonLines, command, failure);
        }
    };
    let mut lines = JsonLines::new();
    let assembled = stream
        .records()
        .iter()
        .try_for_each(|record| lines.push(record))
        .and_then(|()| lines.push(stream.terminal()));
    match assembled.and_then(|()| writer.write_json_lines(&lines)) {
        Ok(()) => ProcessExit::Success,
        Err(error) => {
            writer.write_safe_diagnostic(&error.to_string());
            ProcessExit::StorageOrIo
        }
    }
}

fn detect_requested_mode(arguments: &[OsString]) -> OutputMode {
    let wants_json = arguments.iter().any(|argument| argument == "--json");
    let wants_json_lines = arguments
        .iter()
        .any(|argument| argument == "--events=jsonl")
        || arguments
            .windows(2)
            .any(|pair| pair[0] == "--events" && pair[1] == "jsonl");
    match (wants_json, wants_json_lines) {
        (true, _) => OutputMode::Json,
        (false, true) => OutputMode::JsonLines,
        (false, false) => OutputMode::Human,
    }
}

fn write_help_or_version<StandardOutput, StandardError>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    value: &str,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
{
    match writer.write_trusted_stdout(value) {
        Ok(()) => ProcessExit::Success,
        Err(error) => {
            writer.write_safe_diagnostic(&error.to_string());
            ProcessExit::StorageOrIo
        }
    }
}

fn write_root_help<StandardOutput, StandardError>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
{
    let mut help = Cli::command().render_long_help().to_string();
    help.push('\n');
    write_help_or_version(writer, &help)
}

fn write_setup_help<StandardOutput, StandardError>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
{
    let mut root = Cli::command();
    let help = root.find_subcommand_mut("setup").map_or_else(
        || String::from("VSift setup help is unavailable.\n"),
        |setup| {
            let mut rendered = setup.render_long_help().to_string();
            rendered.push('\n');
            rendered
        },
    );
    write_help_or_version(writer, &help)
}

/// Writes a failure that may carry a typed remediation.
fn write_command_failure<StandardOutput, StandardError>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    mode: OutputMode,
    command: CommandName,
    failure: CommandFailure,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
{
    write_failure_with_detail(writer, mode, command, failure, None)
}

/// Writes a failure; in human mode with `detail` after its message.
///
/// The JSON modes write the one failure result (or its terminal event);
/// human mode renders the same facts on stderr through [`human::failure`].
/// Should that text not fit its budget, the fixed message alone is written
/// instead.
fn write_failure_with_detail<StandardOutput, StandardError>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    mode: OutputMode,
    command: CommandName,
    failure: CommandFailure,
    detail: Option<HumanDetail<'_>>,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
{
    let code = failure.code;
    let response = failure_response(command, failure);
    let result = match mode {
        OutputMode::Human => {
            match human::failure(&response, detail) {
                Ok(text) => writer.write_rendered_stderr(&text),
                Err(_) => writer.write_safe_diagnostic(response.error_message()),
            }
            Ok(())
        }
        OutputMode::Json => writer.write_json(&response),
        OutputMode::JsonLines => writer.write_json(&TerminalEventResponse::new(response)),
    };
    if let Err(error) = result {
        writer.write_safe_diagnostic(&error.to_string());
        ProcessExit::StorageOrIo
    } else {
        ProcessExit::from(code.class())
    }
}

/// The failure result of `command`: its code with the typed remediation,
/// suggested command, affected identifiers and retry hint it carries.
fn failure_response(
    command: CommandName,
    failure: CommandFailure,
) -> OperationResponse<serde_json::Value> {
    let affected: Vec<&str> = failure.affected_ids.iter().map(String::as_str).collect();
    let suggested: Vec<&str> = failure
        .suggested_command
        .iter()
        .map(String::as_str)
        .collect();
    let mut response = match failure.remediation {
        Some(summary) if !suggested.is_empty() => {
            OperationResponse::failure_with_suggested_command(
                command.identifier(),
                failure.code,
                summary,
                &suggested,
            )
        }
        Some(summary) => {
            OperationResponse::failure_with_remediation(command.identifier(), failure.code, summary)
        }
        None => OperationResponse::failure(command.identifier(), failure.code),
    }
    .with_affected_ids(&affected);
    if let Some(retry_after_ms) = failure.retry_after_ms {
        response = response.with_retry_after(retry_after_ms);
    }
    response
}

/// Writes a failure that is its code alone.
fn write_failure<StandardOutput, StandardError>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    mode: OutputMode,
    command: CommandName,
    code: FailureCode,
    detail: Option<HumanDetail<'_>>,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
{
    write_failure_with_detail(writer, mode, command, CommandFailure::from(code), detail)
}

#[cfg(test)]
mod tests {
    use vsift::{EngineError, EnginePorts, JobId};
    use vsift_contract::{CommandName, IDEMPOTENCY_CONFLICT_REMEDIATION, JOB_BUSY_REMEDIATION};

    use super::{
        CommandFailure, Interruption, OutputMode, OutputWriter, ProcessExit, execute_with,
        write_command_failure,
    };

    /// P10 PR 2: a busy job's failure names the job and a retry hint; an
    /// idempotency conflict names the job, has no hint and exits 2.
    #[test]
    fn job_failures_carry_their_job_and_retry_hint() -> Result<(), Box<dyn std::error::Error>> {
        let job = JobId::parse("job_0123456789abcdef0123456789abcdef")?;
        for (error, exit, retry, remediation) in [
            (
                EngineError::JobBusy { job: job.clone() },
                ProcessExit::Retryable,
                serde_json::json!(2_000),
                JOB_BUSY_REMEDIATION,
            ),
            (
                EngineError::IdempotencyConflict { job: job.clone() },
                ProcessExit::UsageOrCapability,
                serde_json::Value::Null,
                IDEMPOTENCY_CONFLICT_REMEDIATION,
            ),
        ] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut writer = OutputWriter::new(&mut stdout, &mut stderr);
            let written = write_command_failure(
                &mut writer,
                OutputMode::Json,
                CommandName::TranscriptRetranscribe,
                CommandFailure::from(error),
            );
            assert_eq!(written, exit);
            let value: serde_json::Value = serde_json::from_slice(&stdout)?;
            assert_eq!(value["error"]["retry_after_ms"], retry);
            assert_eq!(
                value["error"]["affected_ids"],
                serde_json::json!([job.as_str()])
            );
            assert_eq!(value["error"]["remediation"][0]["summary"], remediation);
        }
        Ok(())
    }

    #[tokio::test]
    async fn root_and_setup_without_subcommands_show_help_without_side_effects()
    -> Result<(), Box<dyn std::error::Error>> {
        for arguments in [vec!["vsift"], vec!["vsift", "setup"]] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit = execute_with(
                arguments,
                &mut stdout,
                &mut stderr,
                EnginePorts::system(),
                Interruption::Default,
            )
            .await;

            assert_eq!(exit, ProcessExit::Success);
            assert!(String::from_utf8(stdout)?.contains("Usage:"));
            assert!(stderr.is_empty());
        }
        Ok(())
    }

    #[tokio::test]
    async fn explicit_json_parse_failure_is_one_machine_readable_result()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let exit = execute_with(
            ["vsift", "--json", "unknown-command"],
            &mut stdout,
            &mut stderr,
            EnginePorts::system(),
            Interruption::Default,
        )
        .await;
        let value: serde_json::Value = serde_json::from_slice(&stdout)?;

        assert_eq!(exit, ProcessExit::UsageOrCapability);
        assert_eq!(stdout.last(), Some(&b'\n'));
        assert!(!stdout[..stdout.len().saturating_sub(1)].contains(&b'\n'));
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
        assert!(stderr.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn reserved_install_remains_unavailable_for_an_unreadable_plan()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let exit = execute_with(
            [
                "vsift",
                "setup",
                "install",
                "--plan",
                "missing.json",
                "--accept-plan",
                "unknown",
                "--json",
            ],
            &mut stdout,
            &mut stderr,
            EnginePorts::system(),
            Interruption::Default,
        )
        .await;
        let value: serde_json::Value = serde_json::from_slice(&stdout)?;

        assert_eq!(exit, ProcessExit::UsageOrCapability);
        assert_eq!(value["command"], "setup.install");
        assert_eq!(value["error"]["code"], "COMMAND_NOT_IMPLEMENTED");
        assert!(stderr.is_empty());
        Ok(())
    }
}
