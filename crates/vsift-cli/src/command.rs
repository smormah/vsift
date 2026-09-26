//! Typed public command hierarchy.

use std::path::PathBuf;

use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use vsift::{
    CropRectangle, EvidenceId, FrameSelection, JobId, RuntimeDependency, SessionId, SetupProfile,
    TranscriptRevisionId, VisualCandidateId,
};
use vsift_contract::CommandName;

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

    /// Emit versioned evidence, progress and terminal records as JSON Lines.
    #[arg(long, global = true, value_enum)]
    pub events: Option<EventFormat>,

    /// Explicit private disposable-session root; defaults to the per-user cache.
    #[arg(long, global = true)]
    pub session_root: Option<PathBuf>,

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

/// Setup namespace arguments.
#[derive(Args, Debug)]
pub(crate) struct SetupArguments {
    #[command(subcommand)]
    pub command: Option<SetupCommand>,
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
    /// Register an explicit user-managed local ASR model file.
    ConfigureModel(SetupConfigureModelArguments),
}

impl SetupCommand {
    /// Returns the public operation identifier. Bare `setup` has none: it only
    /// prints help.
    pub(crate) const fn operation_name(&self) -> CommandName {
        match self {
            Self::Check(_) => CommandName::SetupCheck,
            Self::Plan(_) => CommandName::SetupPlan,
            Self::Install(_) => CommandName::SetupInstall,
            Self::Repair(_) => CommandName::SetupRepair,
            Self::List => CommandName::SetupList,
            Self::Remove(_) => CommandName::SetupRemove,
            Self::Rollback(_) => CommandName::SetupRollback,
            Self::Configure(_) => CommandName::SetupConfigure,
            Self::ConfigureModel(_) => CommandName::SetupConfigureModel,
        }
    }
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

    /// Use this absolute `FFmpeg` executable path instead of searching `PATH`.
    #[arg(long)]
    pub ffmpeg: Option<PathBuf>,

    /// Use this absolute `FFprobe` executable path instead of searching `PATH`.
    #[arg(long)]
    pub ffprobe: Option<PathBuf>,

    /// Use this absolute whisper.cpp CLI path instead of searching `PATH`.
    #[arg(long)]
    pub whisper: Option<PathBuf>,
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
    #[arg(value_enum)]
    pub dependency: SetupDependency,
    /// Explicit executable path; project-local configuration is never auto-loaded.
    #[arg(long)]
    pub executable: PathBuf,
}

/// Explicit user-managed ASR model registration.
#[derive(Args, Debug)]
pub(crate) struct SetupConfigureModelArguments {
    /// Absolute path to an existing model file; bytes are not parsed during registration.
    #[arg(long)]
    pub file: PathBuf,
}

/// A provider whose executable may be explicitly registered by the user.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum SetupDependency {
    Ffmpeg,
    Ffprobe,
    Whisper,
}

impl From<SetupDependency> for RuntimeDependency {
    fn from(value: SetupDependency) -> Self {
        match value {
            SetupDependency::Ffmpeg => Self::Ffmpeg,
            SetupDependency::Ffprobe => Self::Ffprobe,
            SetupDependency::Whisper => Self::Whisper,
        }
    }
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

impl From<ExecutionProfile> for SetupProfile {
    fn from(value: ExecutionProfile) -> Self {
        match value {
            ExecutionProfile::Desktop => Self::Desktop,
            ExecutionProfile::Worker => Self::Worker,
        }
    }
}

impl From<SetupProfile> for ExecutionProfile {
    fn from(value: SetupProfile) -> Self {
        match value {
            SetupProfile::Desktop => Self::Desktop,
            SetupProfile::Worker => Self::Worker,
        }
    }
}

/// Foreground local ingestion request.
#[derive(Args, Debug)]
pub(crate) struct IngestArguments {
    /// Local source media path.
    pub source: PathBuf,
    /// Optional local `SubRip` (.srt) or `WebVTT` (.vtt) transcript sidecar.
    #[arg(long)]
    pub transcript: Option<PathBuf>,
    /// Signed microseconds added to every transcript timestamp to reach source
    /// time (for example 500000 or -250000); defaults to 0.
    #[arg(
        long,
        requires = "transcript",
        allow_negative_numbers = true,
        value_name = "MICROSECONDS"
    )]
    pub transcript_offset: Option<i64>,
}

/// Session namespace arguments.
#[derive(Args, Debug)]
pub(crate) struct SessionArguments {
    #[command(subcommand)]
    pub command: SessionCommand,
}

/// Session lifecycle operations.
#[derive(Debug, Subcommand)]
pub(crate) enum SessionCommand {
    /// List bounded session summaries.
    List(SessionScanArguments),
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

impl SessionCommand {
    /// Returns the public operation identifier.
    pub(crate) const fn operation_name(&self) -> CommandName {
        match self {
            Self::List(_) => CommandName::SessionList,
            Self::Status(_) => CommandName::SessionStatus,
            Self::Close(_) => CommandName::SessionClose,
            Self::Renew(_) => CommandName::SessionRenew,
            Self::Retain(_) => CommandName::SessionRetain,
            Self::Clean(_) => CommandName::SessionClean,
        }
    }
}

/// One bounded bucket from the disposable-session index.
#[derive(Args, Debug)]
pub(crate) struct SessionScanArguments {
    /// Continuation bucket returned by the previous page, 0 through 255.
    #[arg(long, value_parser = clap::value_parser!(u16).range(0..=255))]
    pub cursor: Option<u16>,
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
    /// Continuation bucket returned by the previous cleanup page.
    #[arg(long, value_parser = clap::value_parser!(u16).range(0..=255))]
    pub cursor: Option<u16>,
}

/// Transcript namespace arguments.
#[derive(Args, Debug)]
pub(crate) struct TranscriptArguments {
    #[command(subcommand)]
    pub command: TranscriptCommand,
}

/// Transcript operations.
#[derive(Debug, Subcommand)]
pub(crate) enum TranscriptCommand {
    /// Read timestamped text from a bounded range.
    Get(TranscriptGetArguments),
    /// Transcribe the session's speech locally with whisper.cpp into a new
    /// revision: the whole video, or one range of it.
    Retranscribe(TranscriptRetranscribeArguments),
}

impl TranscriptCommand {
    /// Returns the public operation identifier.
    pub(crate) const fn operation_name(&self) -> CommandName {
        match self {
            Self::Get(_) => CommandName::TranscriptGet,
            Self::Retranscribe(_) => CommandName::TranscriptRetranscribe,
        }
    }
}

/// Bounded, pageable transcript read.
#[derive(Args, Debug)]
pub(crate) struct TranscriptGetArguments {
    /// Session containing the transcript.
    pub session: SessionId,
    /// Inclusive source-timeline start in microseconds.
    #[arg(long)]
    pub from: u64,
    /// Exclusive source-timeline end in microseconds.
    #[arg(long)]
    pub to: u64,
    /// Segments per page, 1 through 100; defaults to 20.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub limit: Option<u16>,
    /// Opaque continuation token returned by the previous page of the same query.
    #[arg(long)]
    pub cursor: Option<String>,
    /// Revision to read (a `trv_` identity); defaults to the newest.
    #[arg(long)]
    pub revision: Option<TranscriptRevisionId>,
}

/// Local speech recognition of a session, whole or over one range.
#[derive(Args, Debug)]
pub(crate) struct TranscriptRetranscribeArguments {
    /// Session whose video is transcribed.
    pub session: SessionId,
    /// Inclusive source-timeline start in microseconds; requires --to. Omit
    /// both to transcribe the whole video.
    #[arg(long, requires = "to")]
    pub from: Option<u64>,
    /// Exclusive source-timeline end in microseconds; requires --from.
    #[arg(long, requires = "from")]
    pub to: Option<u64>,
}

/// Bounded literal transcript search.
#[derive(Args, Debug)]
pub(crate) struct SearchArguments {
    /// Session to search.
    pub session: SessionId,
    /// Literal query text, at most 256 bytes; it is never interpreted as code
    /// or a regular expression.
    #[arg(long)]
    pub query: String,
    /// Inclusive source-timeline start in microseconds; requires --to. Omit
    /// both to search the whole transcript.
    #[arg(long, requires = "to")]
    pub from: Option<u64>,
    /// Exclusive source-timeline end in microseconds; requires --from.
    #[arg(long, requires = "from")]
    pub to: Option<u64>,
    /// Hits per page, 1 through 100; defaults to 20.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub limit: Option<u16>,
    /// Opaque continuation token returned by the previous page of the same search.
    #[arg(long)]
    pub cursor: Option<String>,
    /// Revision to search (a `trv_` identity); defaults to the newest.
    #[arg(long)]
    pub revision: Option<TranscriptRevisionId>,
}

/// Visual-candidate page request; missing windows of the range are analysed
/// first, at most 30 minutes of video per call.
#[derive(Args, Debug)]
pub(crate) struct CandidatesArguments {
    /// Session whose video is read.
    pub session: SessionId,
    /// Inclusive source-timeline start in microseconds.
    #[arg(long)]
    pub from: u64,
    /// Exclusive source-timeline end in microseconds; a range past the end
    /// of the video is clipped to it.
    #[arg(long)]
    pub to: u64,
    /// Candidates per page, 1 through 100; defaults to 20.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub limit: Option<u16>,
    /// Opaque continuation token returned by the previous page of the same
    /// range; a call with a cursor never analyses anything.
    #[arg(long)]
    pub cursor: Option<String>,
}

/// Frame namespace arguments.
#[derive(Args, Debug)]
pub(crate) struct FrameArguments {
    #[command(subcommand)]
    pub command: FrameCommand,
}

/// Source-grounded frame operations.
#[derive(Debug, Subcommand)]
pub(crate) enum FrameCommand {
    /// Extract the frame a time or a visual candidate names.
    Get(FrameGetArguments),
    /// Extract consecutive frames on each side of an earlier frame.
    Neighbours(NeighbourArguments),
    /// Extract distinct frames at evenly spaced times over a range.
    Burst(FrameBurstArguments),
}

impl FrameCommand {
    /// Returns the public operation identifier.
    pub(crate) const fn operation_name(&self) -> CommandName {
        match self {
            Self::Get(_) => CommandName::FrameGet,
            Self::Neighbours(_) => CommandName::FrameNeighbours,
            Self::Burst(_) => CommandName::FrameBurst,
        }
    }
}

/// Which frame a time names (ADR 0019 decision 2).
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum FrameSelectArgument {
    /// The first frame at or after the time (the default), so evidence is
    /// never taken from before the moment named.
    AtOrAfter,
    /// The frame on screen at the time: the last frame at or before it.
    DisplayedAt,
}

impl From<FrameSelectArgument> for FrameSelection {
    fn from(value: FrameSelectArgument) -> Self {
        match value {
            FrameSelectArgument::AtOrAfter => Self::AtOrAfter,
            FrameSelectArgument::DisplayedAt => Self::DisplayedAt,
        }
    }
}

/// Exact frame request: a time, or a visual candidate's own frame.
#[derive(Args, Debug)]
#[command(group(ArgGroup::new("target").required(true).args(["at", "candidate"])))]
pub(crate) struct FrameGetArguments {
    /// Session containing the source.
    pub session: SessionId,
    /// Requested source-timeline time in microseconds.
    #[arg(long, value_name = "MICROSECONDS")]
    pub at: Option<u64>,
    /// A visual candidate (a `vcd_` identity from candidates) whose own frame
    /// to extract, exactly at its representative time.
    #[arg(long)]
    pub candidate: Option<VisualCandidateId>,
    /// Which frame the time names; defaults to at-or-after.
    #[arg(long, value_enum, requires = "at", conflicts_with = "candidate")]
    pub select: Option<FrameSelectArgument>,
    /// How far the frame may lie from the time, 0 through 10000000
    /// microseconds; defaults to 1000000.
    #[arg(
        long,
        requires = "at",
        conflicts_with = "candidate",
        value_name = "MICROSECONDS",
        value_parser = clap::value_parser!(u64).range(0..=10_000_000)
    )]
    pub tolerance_us: Option<u64>,
}

/// Neighbouring frames request.
#[derive(Args, Debug)]
pub(crate) struct NeighbourArguments {
    /// Session containing the source.
    pub session: SessionId,
    /// Frame evidence item (an `evd_` identity) around which to navigate.
    pub evidence: EvidenceId,
    /// Frames on each side, 1 through 20; defaults to 1.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=20))]
    pub count: Option<u8>,
}

/// Finite frame-sequence request.
#[derive(Args, Debug)]
pub(crate) struct FrameBurstArguments {
    /// Session containing the source.
    pub session: SessionId,
    /// Inclusive source-timeline start in microseconds.
    #[arg(long)]
    pub from: u64,
    /// Exclusive source-timeline end in microseconds, at most 60 s after
    /// --from; a range past the end of the video is clipped to it.
    #[arg(long)]
    pub to: u64,
    /// Evenly spaced target times, 1 through 100; defaults to 12. Targets
    /// that name the same frame return it once.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=100))]
    pub max_frames: Option<u8>,
}

/// Bounded source-audio extraction request.
#[derive(Args, Debug)]
pub(crate) struct AudioArguments {
    /// Session containing the source.
    pub session: SessionId,
    /// Inclusive source-timeline start in microseconds.
    #[arg(long)]
    pub from: u64,
    /// Exclusive source-timeline end in microseconds, at most 30 s after
    /// --from; a range past the end of the source is clipped to it.
    #[arg(long)]
    pub to: u64,
}

/// Evidence crop request.
#[derive(Args, Debug)]
pub(crate) struct CropArguments {
    /// Session containing the source.
    pub session: SessionId,
    /// Frame or crop evidence item (an `evd_` identity) to cut from.
    pub evidence: EvidenceId,
    /// Rectangle as `x,y,width,height` in the parent image's displayed
    /// pixels; containment in the parent is checked before any tool runs.
    #[arg(long, value_name = "X,Y,WIDTH,HEIGHT")]
    pub rect: CropRectangle,
}

/// Bundle namespace arguments.
#[derive(Args, Debug)]
pub(crate) struct BundleArguments {
    #[command(subcommand)]
    pub command: BundleCommand,
}

/// Portable-bundle operations.
#[derive(Debug, Subcommand)]
pub(crate) enum BundleCommand {
    /// Validate a bundle as bounded data without executing its contents.
    Validate(BundleValidateArguments),
}

impl BundleCommand {
    /// Returns the public operation identifier.
    pub(crate) const fn operation_name(&self) -> CommandName {
        match self {
            Self::Validate(_) => CommandName::BundleValidate,
        }
    }
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

impl JobCommand {
    /// Returns the public operation identifier.
    pub(crate) const fn operation_name(&self) -> CommandName {
        match self {
            Self::Run(_) => CommandName::JobRun,
            Self::Batch(_) => CommandName::JobBatch,
            Self::Status(_) => CommandName::JobStatus,
            Self::Resume(_) => CommandName::JobResume,
            Self::Cancel(_) => CommandName::JobCancel,
        }
    }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use clap::CommandFactory;
    use vsift_contract::CommandName;

    use super::Cli;

    /// Every operation identifier the parser can produce: the command path of
    /// each leaf subcommand joined with `.`, as `CommandName` documents it.
    fn parsed_operation_identifiers() -> BTreeSet<String> {
        let root = Cli::command();
        let mut identifiers = BTreeSet::new();
        for namespace in root.get_subcommands() {
            if namespace.has_subcommands() {
                for operation in namespace.get_subcommands() {
                    identifiers.insert(format!(
                        "{}.{}",
                        namespace.get_name(),
                        operation.get_name()
                    ));
                }
            } else {
                identifiers.insert(namespace.get_name().to_owned());
            }
        }
        identifiers
    }

    #[test]
    fn contract_command_names_are_exactly_the_parsed_operations_plus_parse() {
        let published: BTreeSet<String> = CommandName::ALL
            .into_iter()
            .filter(|name| *name != CommandName::Parse)
            .map(|name| name.identifier().to_owned())
            .collect();

        assert_eq!(published, parsed_operation_identifiers());
    }
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
