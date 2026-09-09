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
    /// Inspect local runtime dependencies without changing the machine.
    Doctor(DoctorArguments),
}

#[derive(Args, Debug)]
struct DoctorArguments {
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
        Command::Doctor(arguments) => run_doctor(arguments).await,
    }
}

async fn run_doctor(arguments: DoctorArguments) -> ExitCode {
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
    let response = DoctorResponse::from(diagnosis);
    let json = serde_json::to_string_pretty(&response)?;
    println!("{json}");
    Ok(())
}

fn print_human(diagnosis: &RuntimeDiagnosis) {
    println!("VSift runtime diagnostics");
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
        println!("Run `vsift setup --plan` to review future managed-installation options.");
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
struct DoctorResponse {
    schema_version: &'static str,
    command: &'static str,
    status: &'static str,
    dependencies: Vec<DoctorDependencyResponse>,
}

impl From<&RuntimeDiagnosis> for DoctorResponse {
    fn from(diagnosis: &RuntimeDiagnosis) -> Self {
        Self {
            schema_version: CONTRACT_VERSION,
            command: "doctor",
            status: diagnosis.readiness.identifier(),
            dependencies: diagnosis
                .dependencies
                .iter()
                .map(DoctorDependencyResponse::from)
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct DoctorDependencyResponse {
    dependency: &'static str,
    capability: &'static str,
    status: &'static str,
    detail: Option<String>,
}

impl From<&DependencyStatus> for DoctorDependencyResponse {
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
