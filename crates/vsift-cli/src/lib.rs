//! Agent-friendly command parsing and bounded public presentation for `VSift`.

#![forbid(unsafe_code)]

mod command;
mod config;
mod json_input;
mod output;
mod session;
mod setup;

use std::{ffi::OsString, io, io::Write, process::ExitCode};

use clap::{CommandFactory, Parser, error::ErrorKind};
use command::{BundleCommand, Cli, Command, EventFormat, SetupCommand};
use config::{ConfigLayer, EffectiveConfig, HostPolicy};
use output::{OperationResponse, OutputMode, OutputWriter, ProcessExit, TerminalEventResponse};
use vsift_domain::FailureCode;
use vsift_infrastructure::ProcessDependencyProbe;

/// Parses the process arguments, executes one command, and returns its documented exit status.
pub async fn run() -> ExitCode {
    let status = execute_with(
        std::env::args_os(),
        io::stdout().lock(),
        io::stderr().lock(),
    )
    .await;
    ExitCode::from(status.code())
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep top-level command composition in one exhaustive dispatch"
)]
async fn execute_with<Arguments, Argument, StandardOutput, StandardError>(
    arguments: Arguments,
    standard_output: StandardOutput,
    standard_error: StandardError,
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
    let cli = match Cli::try_parse_from(arguments) {
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
            let detail = error.to_string();
            return write_failure(
                &mut writer,
                requested_mode,
                "parse",
                FailureCode::InvalidArgument,
                Some(&detail),
            );
        }
    };

    let json_lines = cli.events == Some(EventFormat::Jsonl);
    let mode = match OutputMode::resolve(cli.json, json_lines) {
        Ok(mode) => mode,
        Err(code) => {
            return write_failure(&mut writer, OutputMode::Json, "parse", code, None);
        }
    };

    let Some(command) = cli.command else {
        return write_root_help(&mut writer);
    };
    let explicit_session_root = cli.session_root;
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
                        let detail = error.to_string();
                        return write_failure(
                            &mut writer,
                            mode,
                            "setup.check",
                            FailureCode::InvalidArgument,
                            Some(&detail),
                        );
                    }
                };
                let probe = ProcessDependencyProbe::new(config.probe_timeout);
                setup::run_setup_check(probe, config.profile, mode, &mut writer).await
            }
            None => write_setup_help(&mut writer),
            Some(unimplemented) => {
                let operation = command::SetupArguments {
                    command: Some(unimplemented),
                }
                .operation_name();
                write_failure(
                    &mut writer,
                    mode,
                    operation,
                    FailureCode::CommandNotImplemented,
                    None,
                )
            }
        },
        Command::Ingest(arguments) => {
            let result = session::ingest(arguments, explicit_session_root.as_deref()).await;
            write_session_result(&mut writer, mode, "ingest", result)
        }
        Command::Session(arguments) => {
            let operation = arguments.operation_name();
            let result =
                session::execute_session(arguments.command, explicit_session_root.as_deref());
            write_session_result(&mut writer, mode, operation, result)
        }
        Command::Bundle(arguments) => match arguments.command {
            BundleCommand::Validate(arguments) => {
                let result = session::validate_bundle(&arguments.directory);
                write_session_result(&mut writer, mode, "bundle.validate", result)
            }
        },
        command => write_failure(
            &mut writer,
            mode,
            command.operation_name(),
            FailureCode::CommandNotImplemented,
            None,
        ),
    }
}

fn write_session_result<StandardOutput, StandardError>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    mode: OutputMode,
    command: &'static str,
    result: Result<OperationResponse<serde_json::Value>, FailureCode>,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
{
    let response = match result {
        Ok(response) => response,
        Err(code) => return write_failure(writer, mode, command, code, None),
    };
    let write = match mode {
        OutputMode::Json => writer.write_json(&response),
        OutputMode::JsonLines => writer.write_json(&TerminalEventResponse::new(response)),
        OutputMode::Human => match serde_json::to_string_pretty(&response) {
            Ok(mut text) => {
                text.push('\n');
                writer.write_trusted_stdout(&text)
            }
            Err(_) => return write_failure(writer, mode, command, FailureCode::Internal, None),
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

fn write_failure<StandardOutput, StandardError>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    mode: OutputMode,
    command: &'static str,
    code: FailureCode,
    human_detail: Option<&str>,
) -> ProcessExit
where
    StandardOutput: Write,
    StandardError: Write,
{
    let response = OperationResponse::failure(command, code);
    let result = match mode {
        OutputMode::Human => {
            let message = match human_detail {
                Some(detail) => detail,
                None => response.error_message(),
            };
            writer.write_safe_diagnostic(message);
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

#[cfg(test)]
mod tests {
    use super::{ProcessExit, execute_with};

    #[tokio::test]
    async fn root_and_setup_without_subcommands_show_help_without_side_effects()
    -> Result<(), Box<dyn std::error::Error>> {
        for arguments in [vec!["vsift"], vec!["vsift", "setup"]] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit = execute_with(arguments, &mut stdout, &mut stderr).await;

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
    async fn reserved_command_fails_explicitly_without_running_future_work()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let exit = execute_with(
            ["vsift", "setup", "plan", "--profile", "desktop", "--json"],
            &mut stdout,
            &mut stderr,
        )
        .await;
        let value: serde_json::Value = serde_json::from_slice(&stdout)?;

        assert_eq!(exit, ProcessExit::UsageOrCapability);
        assert_eq!(value["command"], "setup.plan");
        assert_eq!(value["error"]["code"], "COMMAND_NOT_IMPLEMENTED");
        assert!(stderr.is_empty());
        Ok(())
    }
}
