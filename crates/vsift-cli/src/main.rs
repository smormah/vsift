//! `VSift` command-line composition root.

#![forbid(unsafe_code)]

use std::{process::ExitCode, time::Duration};

use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use vsift_application::{DiagnoseRuntime, RuntimeDiagnosis};
use vsift_domain::{DependencyState, DependencyStatus, RuntimeReadiness};
use vsift_infrastructure::ProcessDependencyProbe;

const CONTRACT_VERSION: &str = "1";
const BLOCKED_EXIT_CODE: u8 = 2;

#[derive(Debug, Parser)]
#[command(
    name = "vsift",
    version,
    about = "Sift technical video into agent-ready evidence"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect and manage the local `VSift` setup.
    Setup(SetupArguments),
}

#[derive(Args, Debug)]
struct SetupArguments {
    #[command(subcommand)]
    command: SetupCommand,
}

#[derive(Debug, Subcommand)]
enum SetupCommand {
    /// Inspect local runtime dependencies without changing the machine.
    Check(SetupCheckArguments),
}

#[derive(Args, Debug)]
struct SetupCheckArguments {
    /// Emit the versioned machine-readable response contract.
    #[arg(long)]
    json: bool,

    /// Maximum seconds to wait for each dependency probe.
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=60))]
    timeout_seconds: u64,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Command::Setup(arguments) => match arguments.command {
            SetupCommand::Check(arguments) => run_setup_check(arguments).await,
        },
    }
}

async fn run_setup_check(arguments: SetupCheckArguments) -> ExitCode {
    let probe = ProcessDependencyProbe::new(Duration::from_secs(arguments.timeout_seconds));
    let diagnosis = DiagnoseRuntime::new(probe).execute().await;

    if arguments.json {
        if let Err(error) = print_json(&diagnosis) {
            eprintln!("VSift could not serialize the diagnostic result: {error}");
            return ExitCode::FAILURE;
        }
    } else {
        print_human(&diagnosis);
    }

    if diagnosis.readiness == RuntimeReadiness::Blocked {
        ExitCode::from(BLOCKED_EXIT_CODE)
    } else {
        ExitCode::SUCCESS
    }
}

fn print_json(diagnosis: &RuntimeDiagnosis) -> Result<(), serde_json::Error> {
    let response = SetupCheckResponse::from(diagnosis);
    let json = serde_json::to_string_pretty(&response)?;
    println!("{json}");
    Ok(())
}

fn print_human(diagnosis: &RuntimeDiagnosis) {
    println!("VSift setup check");
    println!("Status: {}", diagnosis.readiness.identifier());

    for status in &diagnosis.dependencies {
        let (marker, detail) = human_state(&status.state);
        println!(
            "[{marker}] {} ({}): {detail}",
            status.dependency.display_name(),
            status.dependency.capability().identifier()
        );
    }

    if diagnosis.readiness == RuntimeReadiness::Blocked {
        println!(
            "Installation assistance is not available yet; install the missing media dependencies and run this check again."
        );
    }
}

fn human_state(state: &DependencyState) -> (&'static str, &str) {
    match state {
        DependencyState::Available { version } => ("ok", version),
        DependencyState::Missing => ("missing", "not found on PATH"),
        DependencyState::Unhealthy { message } => ("unhealthy", message),
        DependencyState::TimedOut => ("timeout", "probe exceeded its deadline"),
    }
}

#[derive(Serialize)]
struct SetupCheckResponse {
    schema_version: &'static str,
    command: &'static str,
    status: &'static str,
    dependencies: Vec<SetupCheckDependencyResponse>,
}

impl From<&RuntimeDiagnosis> for SetupCheckResponse {
    fn from(diagnosis: &RuntimeDiagnosis) -> Self {
        Self {
            schema_version: CONTRACT_VERSION,
            command: "setup.check",
            status: diagnosis.readiness.identifier(),
            dependencies: diagnosis
                .dependencies
                .iter()
                .map(SetupCheckDependencyResponse::from)
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct SetupCheckDependencyResponse {
    dependency: &'static str,
    capability: &'static str,
    status: &'static str,
    detail: Option<String>,
}

impl From<&DependencyStatus> for SetupCheckDependencyResponse {
    fn from(status: &DependencyStatus) -> Self {
        let detail = match &status.state {
            DependencyState::Available { version } => Some(version.clone()),
            DependencyState::Unhealthy { message } => Some(message.clone()),
            DependencyState::Missing | DependencyState::TimedOut => None,
        };

        Self {
            dependency: status.dependency.identifier(),
            capability: status.dependency.capability().identifier(),
            status: status.state.identifier(),
            detail,
        }
    }
}
