//! Typed public command hierarchy.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use vsift_domain::{EvidenceId, JobId, SessionId};

/// Complete public R0 command parser.
#[derive(Debug, Parser)]
#[command(
    name = "vsift",
    version,
    about = "Sift technical video into agent-ready evidence",
    disable_help_subcommand = true
)]
pub(crate) struct Cli {
    /// Emit one versioned JSON terminal result.
    #[arg(long, global = true)]
    pub json: bool,

    /// Emit versioned progress and terminal records as JSON Lines.
    #[arg(long, global = true, value_enum)]
    pub events: Option<EventFormat>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Supported streaming event representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum EventFormat {
    /// One bounded JSON object per line.
    Jsonl,
}

/// Stable top-level R0 namespace.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Inspect and manage the local `VSift` setup.
    Setup(SetupArguments),
    /// Prepare a local video investigation session.
    Ingest(IngestArguments),
    /// Inspect or change a session lifecycle.
    Session(SessionArguments),
    /// Read or revise timestamped transcript evidence.
    Transcript(TranscriptArguments),
    /// Search timestamped transcript evidence.
    Search(SearchArguments),
    /// Page through visual evidence candidates.
    Candidates(CandidatesArguments),
    /// Extract or navigate source-grounded frames.
    Frame(FrameArguments),
    /// Extract a bounded source audio range.
    Audio(AudioArguments),
    /// Crop an orientation-correct evidence image.
    Crop(CropArguments),
    /// Validate portable evidence bundles.
    Bundle(BundleArguments),
    /// Execute or inspect recoverable worker jobs.
    Job(JobArguments),
}

impl Command {
    /// Returns the stable operation identifier used in public responses.
    #[must_use]
    pub(crate) const fn operation_name(&self) -> &'static str {
        match self {
            Self::Setup(arguments) => arguments.operation_name(),
            Self::Ingest(_) => "ingest",
            Self::Session(arguments) => arguments.operation_name(),
            Self::Transcript(arguments) => arguments.operation_name(),
            Self::Search(_) => "search",
            Self::Candidates(_) => "candidates",
            Self::Frame(arguments) => arguments.operation_name(),
            Self::Audio(_) => "audio",
            Self::Crop(_) => "crop",
            Self::Bundle(arguments) => arguments.operation_name(),
            Self::Job(arguments) => arguments.operation_name(),
        }
    }
}

/// Setup namespace arguments.
#[derive(Args, Debug)]
pub(crate) struct SetupArguments {
    #[command(subcommand)]
    pub command: Option<SetupCommand>,
}

impl SetupArguments {
    pub(crate) const fn operation_name(&self) -> &'static str {
        match self.command {
            Some(SetupCommand::Check(_)) => "setup.check",
            Some(SetupCommand::Plan(_)) => "setup.plan",
            Some(SetupCommand::Install(_)) => "setup.install",
            Some(SetupCommand::Repair(_)) => "setup.repair",
            Some(SetupCommand::List) => "setup.list",
            Some(SetupCommand::Remove(_)) => "setup.remove",
            Some(SetupCommand::Rollback(_)) => "setup.rollback",
            Some(SetupCommand::Configure(_)) => "setup.configure",
            None => "setup",
        }
    }
}

/// Setup lifecycle operations.
#[derive(Debug, Subcommand)]
pub(crate) enum SetupCommand {
    /// Inspect local dependencies without changing the machine.
    Check(SetupCheckArguments),
    /// Create a reviewable installation plan without applying it.
    Plan(SetupPlanArguments),
    /// Apply an unchanged, explicitly accepted installation plan.
    Install(SetupInstallArguments),
    /// Create or apply a bounded repair plan.
    Repair(SetupRepairArguments),
    /// List configured and managed dependency versions.
    List,
    /// Remove an unused managed dependency version.
    Remove(SetupVersionArguments),
    /// Activate an earlier validated managed version.
    Rollback(SetupVersionArguments),
    /// Register explicitly supplied dependency configuration.
    Configure(SetupConfigureArguments),
}

/// Read-only setup-check options.
#[derive(Args, Debug)]
pub(crate) struct SetupCheckArguments {
    /// Maximum seconds to wait for each dependency probe.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=60))]
    pub timeout_seconds: Option<u64>,

    /// Capability profile to diagnose.
    #[arg(long, value_enum)]
    pub profile: Option<ExecutionProfile>,
}

/// Setup plan selection.
#[derive(Args, Debug)]
pub(crate) struct SetupPlanArguments {
    /// Capability profile whose dependencies should be planned.
    #[arg(long, value_enum)]
    pub profile: ExecutionProfile,
}

/// Explicit installation-plan acceptance.
#[derive(Args, Debug)]
pub(crate) struct SetupInstallArguments {
    /// Path to the previously generated plan.
    #[arg(long)]
    pub plan: PathBuf,
    /// Digest printed by the unchanged plan.
    #[arg(long)]
    pub accept_plan: String,
}

/// Repair plan input; application remains a later packet.
#[derive(Args, Debug)]
pub(crate) struct SetupRepairArguments {
    /// Capability profile to inspect for repair.
    #[arg(long, value_enum)]
    pub profile: ExecutionProfile,
}

/// Managed dependency and version selector.
#[derive(Args, Debug)]
pub(crate) struct SetupVersionArguments {
    /// Stable dependency identifier.
    pub dependency: String,
    /// Immutable version identifier.
    pub version: String,
}

/// Explicit external dependency registration.
#[derive(Args, Debug)]
pub(crate) struct SetupConfigureArguments {
    /// Stable dependency identifier.
    pub dependency: String,
    /// Explicit executable path; project-local configuration is never auto-loaded.
    #[arg(long)]
    pub executable: PathBuf,
}

/// Resource and lifecycle profile selected for an operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum ExecutionProfile {
    /// Interactive local investigation with disposable state.
    Desktop,
    /// Noninteractive bounded single-host worker execution.
    Worker,
}

impl ExecutionProfile {
    /// Returns the stable machine-readable profile name.
    #[must_use]
    pub(crate) const fn identifier(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Worker => "worker",
        }
    }
}

/// Foreground local ingestion request.
#[derive(Args, Debug)]
pub(crate) struct IngestArguments {
    /// Local source media path.
    pub source: PathBuf,
    /// Optional local transcript sidecar.
    #[arg(long)]
    pub transcript: Option<PathBuf>,
}

/// Session namespace arguments.
#[derive(Args, Debug)]
pub(crate) struct SessionArguments {
    #[command(subcommand)]
    pub command: SessionCommand,
}

impl SessionArguments {
    const fn operation_name(&self) -> &'static str {
        match self.command {
            SessionCommand::List => "session.list",
            SessionCommand::Status(_) => "session.status",
            SessionCommand::Close(_) => "session.close",
            SessionCommand::Renew(_) => "session.renew",
            SessionCommand::Retain(_) => "session.retain",
            SessionCommand::Clean(_) => "session.clean",
        }
    }
}

/// Session lifecycle operations.
#[derive(Debug, Subcommand)]
pub(crate) enum SessionCommand {
    /// List bounded session summaries.
    List,
    /// Read one session's status.
    Status(SessionIdentityArguments),
    /// Close one session after active work settles.
    Close(SessionIdentityArguments),
    /// Renew one eligible ephemeral session.
    Renew(SessionIdentityArguments),
    /// Export one session as a validated portable bundle.
    Retain(SessionRetainArguments),
    /// Find or remove expired owned sessions.
    Clean(SessionCleanArguments),
}

/// One validated session identifier.
#[derive(Args, Debug)]
pub(crate) struct SessionIdentityArguments {
    /// Session to inspect or change.
    pub session: SessionId,
}

/// Explicit session export request.
#[derive(Args, Debug)]
pub(crate) struct SessionRetainArguments {
    /// Session to retain.
    pub session: SessionId,
    /// New destination directory.
    #[arg(long)]
    pub output: PathBuf,
    /// Include bound source bytes in the bundle.
    #[arg(long)]
    pub include_source: bool,
}

/// Bounded expired-session cleanup request.
#[derive(Args, Debug)]
pub(crate) struct SessionCleanArguments {
    /// Restrict the operation to sessions proven expired.
    #[arg(long)]
    pub expired: bool,
    /// Report eligible sessions without deleting them.
    #[arg(long)]
    pub dry_run: bool,
}

/// Transcript namespace arguments.
#[derive(Args, Debug)]
pub(crate) struct TranscriptArguments {
    #[command(subcommand)]
    pub command: TranscriptCommand,
}

impl TranscriptArguments {
    const fn operation_name(&self) -> &'static str {
        match self.command {
            TranscriptCommand::Get(_) => "transcript.get",
            TranscriptCommand::Retranscribe(_) => "transcript.retranscribe",
        }
    }
}

/// Transcript operations.
#[derive(Debug, Subcommand)]
pub(crate) enum TranscriptCommand {
    /// Read timestamped text from a bounded range.
    Get(SessionRangeArguments),
    /// Produce a new transcription revision for a bounded range.
    Retranscribe(SessionRangeArguments),
}

/// Session and normalized microsecond range.
#[derive(Args, Debug)]
pub(crate) struct SessionRangeArguments {
    /// Session containing the source.
    pub session: SessionId,
    /// Inclusive source-timeline start in microseconds.
    #[arg(long)]
    pub from: u64,
    /// Exclusive source-timeline end in microseconds.
    #[arg(long)]
    pub to: u64,
}

/// Bounded literal transcript search.
#[derive(Args, Debug)]
pub(crate) struct SearchArguments {
    /// Session to search.
    pub session: SessionId,
    /// Literal query text; it is never interpreted as code or regular expression.
    #[arg(long)]
    pub query: String,
}

/// Visual-candidate page request.
#[derive(Args, Debug)]
pub(crate) struct CandidatesArguments {
    /// Session containing visual evidence.
    pub session: SessionId,
    /// Inclusive source-timeline start in microseconds.
    #[arg(long)]
    pub from: u64,
    /// Exclusive source-timeline end in microseconds.
    #[arg(long)]
    pub to: u64,
    /// Positive page size, capped by the application contract.
    #[arg(long)]
    pub limit: Option<u16>,
    /// Opaque continuation token returned by the previous page.
    #[arg(long)]
    pub cursor: Option<String>,
}

/// Frame namespace arguments.
#[derive(Args, Debug)]
pub(crate) struct FrameArguments {
    #[command(subcommand)]
    pub command: FrameCommand,
}

impl FrameArguments {
    const fn operation_name(&self) -> &'static str {
        match self.command {
            FrameCommand::Get(_) => "frame.get",
            FrameCommand::Neighbours(_) => "frame.neighbours",
            FrameCommand::Burst(_) => "frame.burst",
        }
    }
}

/// Source-grounded frame operations.
#[derive(Debug, Subcommand)]
pub(crate) enum FrameCommand {
    /// Extract the first displayed frame at or after a requested time.
    Get(FrameGetArguments),
    /// Retrieve bounded adjacent evidence states.
    Neighbours(NeighbourArguments),
    /// Extract a finite sequence over a bounded range.
    Burst(FrameBurstArguments),
}

/// Exact frame request.
#[derive(Args, Debug)]
pub(crate) struct FrameGetArguments {
    /// Session containing the source.
    pub session: SessionId,
    /// Requested source-timeline time in microseconds.
    #[arg(long)]
    pub at: u64,
}

/// Neighbouring evidence request.
#[derive(Args, Debug)]
pub(crate) struct NeighbourArguments {
    /// Evidence item around which to navigate.
    pub evidence: EvidenceId,
    /// Maximum states on each side.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=20))]
    pub count: Option<u16>,
}

/// Finite frame-sequence request.
#[derive(Args, Debug)]
pub(crate) struct FrameBurstArguments {
    /// Session containing the source.
    pub session: SessionId,
    /// Inclusive source-timeline start in microseconds.
    #[arg(long)]
    pub from: u64,
    /// Exclusive source-timeline end in microseconds.
    #[arg(long)]
    pub to: u64,
    /// Hard frame-count budget.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub max_frames: u16,
}

/// Bounded source-audio extraction request.
#[derive(Args, Debug)]
pub(crate) struct AudioArguments {
    /// Session containing the source.
    pub session: SessionId,
    /// Inclusive source-timeline start in microseconds.
    #[arg(long)]
    pub from: u64,
    /// Exclusive source-timeline end in microseconds.
    #[arg(long)]
    pub to: u64,
}

/// Evidence crop request.
#[derive(Args, Debug)]
pub(crate) struct CropArguments {
    /// Source evidence image.
    pub evidence: EvidenceId,
    /// Rectangle as `x,y,width,height`; frame containment is checked before I/O.
    #[arg(long)]
    pub rect: String,
}

/// Bundle namespace arguments.
#[derive(Args, Debug)]
pub(crate) struct BundleArguments {
    #[command(subcommand)]
    pub command: BundleCommand,
}

impl BundleArguments {
    const fn operation_name(&self) -> &'static str {
        match self.command {
            BundleCommand::Validate(_) => "bundle.validate",
        }
    }
}

/// Portable-bundle operations.
#[derive(Debug, Subcommand)]
pub(crate) enum BundleCommand {
    /// Validate a bundle as bounded data without executing its contents.
    Validate(BundleValidateArguments),
}

/// Local bundle path.
#[derive(Args, Debug)]
pub(crate) struct BundleValidateArguments {
    /// Bundle directory to validate.
    pub directory: PathBuf,
}

/// Worker-job namespace arguments.
#[derive(Args, Debug)]
pub(crate) struct JobArguments {
    #[command(subcommand)]
    pub command: JobCommand,
}

impl JobArguments {
    const fn operation_name(&self) -> &'static str {
        match self.command {
            JobCommand::Run(_) => "job.run",
            JobCommand::Batch(_) => "job.batch",
            JobCommand::Status(_) => "job.status",
            JobCommand::Resume(_) => "job.resume",
            JobCommand::Cancel(_) => "job.cancel",
        }
    }
}

/// Recoverable worker-job operations.
#[derive(Debug, Subcommand)]
pub(crate) enum JobCommand {
    /// Execute one versioned request.
    Run(JobRequestArguments),
    /// Execute a finite JSONL request stream.
    Batch(JobBatchArguments),
    /// Read one job's durable status.
    Status(JobIdentityArguments),
    /// Resume one compatible interrupted job.
    Resume(JobIdentityArguments),
    /// Request cancellation of one job.
    Cancel(JobIdentityArguments),
}

/// One request document.
#[derive(Args, Debug)]
pub(crate) struct JobRequestArguments {
    /// Versioned JSON request file.
    #[arg(long)]
    pub request: PathBuf,
}

/// Finite request stream.
#[derive(Args, Debug)]
pub(crate) struct JobBatchArguments {
    /// Versioned JSONL request file.
    #[arg(long)]
    pub requests: PathBuf,
}

/// One validated job identifier.
#[derive(Args, Debug)]
pub(crate) struct JobIdentityArguments {
    /// Job to inspect or change.
    pub job: JobId,
}

#[cfg(all(test, unix))]
mod unix_tests {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    use clap::Parser;

    use super::{Cli, Command};

    #[test]
    fn non_utf8_source_path_remains_os_data() -> Result<(), Box<dyn std::error::Error>> {
        let source = OsString::from_vec(vec![b'v', b'i', b'd', 0xff, b'.', b'm', b'p', b'4']);
        let parsed = Cli::try_parse_from([
            OsString::from("vsift"),
            OsString::from("ingest"),
            source.clone(),
        ])?;

        match parsed.command {
            Some(Command::Ingest(arguments)) => assert_eq!(arguments.source.as_os_str(), source),
            _ => return Err("ingest command did not preserve the source path".into()),
        }
        Ok(())
    }
}
