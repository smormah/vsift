//! Typed public command hierarchy.

use std::path::PathBuf;

use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use vsift::{
    ClockReading, CropRectangle, DurabilityRequirement, EvidenceId, FrameSelection,
    IsolationProfile, JobId, ManagedComponent, ManagedVersionKey, ManagedVersionKeyError,
    OperationId, OptionalComponent, OptionalComponentSet, RuntimeDependency, SessionId,
    SetupProfile, TimeAnchorError, TranscriptRevisionId, VisualCandidateId,
};
use vsift_contract::{CommandName, ResultView};

/// The top-level help's worked example: what a first investigation runs, in
/// order, for a reader (often an agent) that has only this help (P14 PR 7).
///
/// Help text only, not a JSON contract. Every line that starts with `vsift`
/// or a step number followed by `vsift` is parsed by a test, so a command,
/// option or argument renamed in the parser fails that test until this text is
/// updated. clap does not wrap it (the `wrap_help` feature is off), so the
/// lines are written at the width they print.
pub(crate) const TYPICAL_INVESTIGATION_HELP: &str = "\
A typical investigation (every command takes --json):
  1. vsift setup check --json
       What is installed. Report anything missing to the user; never install it yourself.
  2. vsift ingest <video> --json
       Copies the video into a private session and prints its id (ses_...). The original
       file is never changed. If the user gave you a transcript file (.srt or .vtt), add
         --transcript <file>   (and --transcript-offset <microseconds> if its times are
                                shifted from the video's)
       For a direct https:// link to a video file, put --url <link> in place of <video>
       (--url - reads the link from standard input). VSift fetches it once, with limits,
       and nothing after that uses the network. A web page is not a video file.
       Without one, speech is found only when local speech recognition is set up (setup
       check says); then run:
         vsift job start <session> --json
       A recording under 10 minutes is transcribed before it answers. A longer one is
       transcribed in the background: follow it with vsift job status <job> --json, and
       search and read the part already done meanwhile (answers say partial). If
       recognition is not set up there is no speech evidence: say so; do not install
       anything.
  3. vsift search <session> --query <words> --limit 20 --json
     vsift transcript get <session> --from <us> --to <us> --limit 20 --json
       What was said. Every time is microseconds from the start of the video.
  4. vsift candidates <session> --from <us> --to <us> --limit 20 --json
       Moments where the picture changes.
  5. vsift frame get <session> --at <us> --json
     vsift frame burst <session> --from <us> --to <us> --max-frames 4 --json
       Look at specific moments; each frame is an image file to open.
  6. vsift handoff skeleton <session> --question <words> --json
     vsift handoff check --json
       Only when you were asked for a vsift-handoff report. skeleton prints a valid one
       with the session's ids filled in (data.handoff), to add your findings to; never
       build the JSON by search and replace. check reads the draft from standard input and
       lists what to fix before you send it. A plain-text report needs no check (a draft
       without a vsift-handoff block is reported as missing one).
  7. vsift session close <session> --json
       When you are done.

Ask for small pages. --limit takes 1 to 100 (default 20) and --max-frames 1 to 100
(default 12), more than an investigation needs: reading 20 results at a time and looking
at a handful of frames keeps it cheap and inside any budget. Ask for more only when the
user does.

--session-root and --host-isolation are for operators who run VSift as a service for other
people (a worker host). They are not part of an investigation: leave them out. The default
private per-user location is the right one; an agent never needs to name, search for or
create another folder. If the default location cannot be used (for example the drive has no
room for the video), the error says why: report it to the user, who frees the space or asks
an operator, instead of choosing a folder.

When a command fails, read its code and remediation (the \"Fix:\" and \"Run:\" lines, or
error.remediation with --json) and do what they say. A missing tool is the user's to install;
an installation plan is theirs to accept.";

/// Complete public R0 command parser.
#[derive(Debug, Parser)]
#[command(
    name = "vsift",
    // The package version, and in a release build its source commit
    // (build.rs, ADR 0023).
    version = env!("VSIFT_VERSION_TEXT"),
    about = "Sift technical video into agent-ready evidence",
    after_help = TYPICAL_INVESTIGATION_HELP,
    disable_help_subcommand = true
)]
pub(crate) struct Cli {
    /// Emit one versioned JSON terminal result.
    #[arg(long, global = true)]
    pub json: bool,

    /// Emit versioned evidence, progress and terminal records as JSON Lines.
    #[arg(long, global = true, value_enum)]
    pub events: Option<EventFormat>,

    /// For operators who run a worker host; an agent leaves it out. Explicit
    /// private disposable-session root; defaults to the per-user cache.
    #[arg(long, global = true)]
    pub session_root: Option<PathBuf>,

    /// For operators who run a worker host; an agent leaves it out. Isolation
    /// to run under; strict-linux is attested before any work.
    #[arg(long, global = true, value_enum, default_value_t)]
    pub host_isolation: HostIsolationArgument,

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
    /// Check what is installed and set up what is missing (run it any time).
    ///
    /// Plain `vsift setup`, in a terminal, shows what works and what is
    /// missing, offers to install what can be installed here, with each
    /// download's source, size and licence, and installs nothing until you
    /// type yes. Without a terminal it only reports. The commands below are
    /// what AI assistants and scripts use; they never ask anything.
    Setup(SetupArguments),
    /// Prepare a local video investigation session.
    Ingest(IngestArguments),
    /// Inspect or change a session lifecycle.
    Session(SessionArguments),
    /// Read or revise timestamped transcript evidence.
    Transcript(TranscriptArguments),
    /// Search timestamped transcript evidence.
    ///
    /// Over a part of the range that was not transcribed the answer is partial
    /// and names the part: a word said there cannot be found, and that is not
    /// evidence that none was said.
    Search(SearchArguments),
    /// Page through visual evidence candidates.
    Candidates(CandidatesArguments),
    /// Read on-screen text off a session's frames with the optional OCR runner.
    ///
    /// Opt-in and bounded: each call reads at most --max-frames frames of the
    /// range's plan (every visual candidate's frame and one frame every
    /// --period-us), then stops; run it again to continue. The answer counts the
    /// frames read, failed and pending, and is partial until the plan is read.
    /// ocr search then pages the text it read, each hit citing its frame, time
    /// and region, with or without a transcript (search's first page shows it
    /// beside the transcript).
    Ocr(OcrArguments),
    /// Extract or navigate source-grounded frames.
    Frame(FrameArguments),
    /// Extract a bounded audio clip for a person or a speech tool to play; a
    /// coding agent cannot listen to it and reads speech with transcript get.
    ///
    /// The clip is a WAV file of at most 30 seconds, written into the
    /// session's private folder. A person plays it, or hands it to a speech
    /// tool of their own. A coding agent cannot listen to it, and reading the
    /// file's bytes (for example with base64) tells it nothing: what was said
    /// is in the transcript, which vsift transcript get reads.
    Audio(AudioArguments),
    /// Crop an orientation-correct evidence image.
    Crop(CropArguments),
    /// Validate portable evidence bundles.
    Bundle(BundleArguments),
    /// Execute or inspect recoverable worker jobs.
    Job(JobArguments),
    /// Check an agent's draft handoff report before it is sent.
    Handoff(HandoffArguments),
    /// Make a transcript, subtitles and the screen changes of a video, beside
    /// it (the same as  vsift <video>).
    ///
    /// Writes <name>.txt (a readable transcript), <name>.srt (subtitles most
    /// video players pick up) and <name>-vsift/ (the screen changes, with a
    /// frame for each and their list) beside the video, or into --output. It
    /// never replaces a file: a name that is taken is refused before any work.
    /// Each file names the video's SHA-256 and the ids of the run; every
    /// frame's SHA-256 is in the list. Video tools and speech recognition must
    /// be set up first (vsift setup check). Not for AI assistants: they use the
    /// session commands.
    Watch(WatchArguments),
}

/// `watch` arguments (ADR 0037).
#[derive(Args, Debug)]
pub(crate) struct WatchArguments {
    /// The video.
    pub source: PathBuf,
    /// The folder to write into, instead of beside the video: an existing
    /// folder, or a new one whose parent exists.
    #[arg(long, value_name = "FOLDER")]
    pub output: Option<PathBuf>,
}

/// Setup namespace arguments: plain `vsift setup` is the setup door (ADR
/// 0036), and each subcommand is one step of it for assistants and scripts.
#[derive(Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct SetupArguments {
    #[command(subcommand)]
    pub command: Option<SetupCommand>,
    /// Plain `vsift setup` only: also offer an optional extra (repeatable).
    /// `web-videos` fetches videos from public web pages (Ubuntu 24.04
    /// x86-64 only for now).
    #[arg(long = "include", value_enum, value_name = "EXTRA")]
    pub include: Vec<DoorExtra>,
    /// Plain `vsift setup` only: nobody is there to answer. Installs the
    /// parts named with --install without asking, and reports pass, fail or
    /// error; --json gives the `setup` result.
    #[arg(long = "unattended", requires = "install")]
    pub unattended: bool,
    /// With --unattended only: the core parts to install, `video`, `speech`
    /// or both (`video,speech`). Naming them accepts the plan for them;
    /// every download stays pinned and checked.
    #[arg(
        long = "install",
        value_enum,
        value_delimiter = ',',
        value_name = "PART",
        requires = "unattended"
    )]
    pub install: Vec<DoorPart>,
    /// With --unattended only: install only when the plan built for the
    /// named parts has exactly this digest (`plan_digest`).
    #[arg(long = "accept-plan", value_name = "DIGEST", requires = "unattended")]
    pub accept_plan: Option<String>,
}

/// A core part the unattended setup door installs, by the name a person
/// reads in its table (ADR 0036 section 6).
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum DoorPart {
    /// `FFmpeg` with `FFprobe`: reading videos and taking frames.
    Video,
    /// whisper.cpp and its model: turning speech into text.
    Speech,
}

/// An optional extra the setup door adds, by the name a person reads in its
/// table (ADR 0036 section 7). Each maps to an optional component group of
/// the managed catalogue; more join as their groups land.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum DoorExtra {
    /// Videos from public web pages: `online_links` in the JSON results.
    #[value(name = "web-videos")]
    WebVideos,
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
    /// Diagnose the managed tools and plan their repair; changes nothing.
    Repair,
    /// List each managed component's versions, the selected one, and whether each verifies.
    List,
    /// Remove a managed version, a whole managed component, or abandoned stages.
    Remove(SetupRemoveArguments),
    /// Select a component's previous managed version, or a named installed one.
    Rollback(SetupRollbackArguments),
    /// Register explicitly supplied dependency configuration.
    Configure(SetupConfigureArguments),
    /// Register an explicit user-managed local ASR model file.
    ConfigureModel(SetupConfigureModelArguments),
}

impl SetupCommand {
    /// Returns the public operation identifier of a setup subcommand; plain
    /// `setup` is [`CommandName::Setup`].
    pub(crate) const fn operation_name(&self) -> CommandName {
        match self {
            Self::Check(_) => CommandName::SetupCheck,
            Self::Plan(_) => CommandName::SetupPlan,
            Self::Install(_) => CommandName::SetupInstall,
            Self::Repair => CommandName::SetupRepair,
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
    /// Seconds allowed for all the dependency probes together, 1 to 60 (default 30).
    ///
    /// The probes run one after another and share this one deadline. A tool
    /// that has not answered when it passes, or was not reached before it, is
    /// reported as a timeout, not as missing: run the check again, or allow
    /// more time here. A first run after an install or a restart can be slow.
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
    /// Include an optional component (repeatable). `online_links` is what
    /// fetching a video from a public web page needs: yt-dlp and the
    /// JavaScript runtime it runs (Ubuntu 24.04 x86-64 only). Ask the
    /// operator first. The plan lists what it leaves out.
    #[arg(long = "include", value_enum, value_name = "NAME")]
    pub include: Vec<OptionalComponentArgument>,
}

/// An optional component `setup plan --include` adds, named as the JSON
/// results name it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum OptionalComponentArgument {
    /// Online links: yt-dlp and the JavaScript runtime it needs.
    #[value(name = "online_links")]
    OnlineLinks,
}

impl OptionalComponentArgument {
    /// The set of optional components `arguments` include.
    pub(crate) fn set(arguments: &[Self]) -> OptionalComponentSet {
        arguments.iter().fold(
            OptionalComponentSet::EMPTY,
            |set, argument| match argument {
                Self::OnlineLinks => set.with(OptionalComponent::OnlineLinks),
            },
        )
    }
}

/// Explicit installation-plan acceptance.
#[derive(Args, Debug)]
pub(crate) struct SetupInstallArguments {
    /// Path to the saved `setup plan --json` result to apply.
    #[arg(long)]
    pub plan: PathBuf,
    /// The saved plan's `plan_digest`, accepting exactly that plan.
    #[arg(long)]
    pub accept_plan: String,
    /// Install offline from this absolute folder, which holds each
    /// artifact of the plan under the file name its source URL ends with;
    /// the bytes are verified exactly as a download is.
    #[arg(long)]
    pub artifact_dir: Option<PathBuf>,
}

/// What `setup rollback` selects.
#[derive(Args, Debug)]
pub(crate) struct SetupRollbackArguments {
    /// The managed component.
    #[arg(value_enum)]
    pub component: ManagedComponentArgument,
    /// Select this installed version instead of the one selected before
    /// (see `setup list`).
    #[arg(long, value_parser = parse_managed_version)]
    pub version: Option<ManagedVersionKey>,
}

/// What `setup remove` removes: one version, a whole component, or
/// abandoned stages.
#[derive(Args, Debug)]
#[command(group(
    ArgGroup::new("target")
        .required(true)
        .args(["component", "stale_stages"])
))]
pub(crate) struct SetupRemoveArguments {
    /// The managed component; without --version, its selection and every
    /// version not in use by a running job.
    #[arg(value_enum)]
    pub component: Option<ManagedComponentArgument>,
    /// Remove only this unselected version of the component.
    #[arg(long, requires = "component", value_parser = parse_managed_version)]
    pub version: Option<ManagedVersionKey>,
    /// Remove stages abandoned by interrupted or failed installations and
    /// half-written selection pointers.
    #[arg(long, conflicts_with_all = ["component", "version"])]
    pub stale_stages: bool,
}

/// A managed component, named as the JSON results name it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum ManagedComponentArgument {
    /// `FFmpeg` and `FFprobe`.
    #[value(name = "ffmpeg_ffprobe")]
    MediaTools,
    /// The whisper.cpp command-line runtime.
    #[value(name = "whisper_cli")]
    WhisperCli,
    /// The speech-recognition model.
    #[value(name = "whisper_model")]
    WhisperModel,
    /// The optional yt-dlp's JavaScript runtime.
    #[value(name = "js_runtime_deno")]
    JsRuntimeDeno,
    /// The optional yt-dlp.
    #[value(name = "yt_dlp")]
    YtDlp,
}

impl From<ManagedComponentArgument> for ManagedComponent {
    fn from(value: ManagedComponentArgument) -> Self {
        match value {
            ManagedComponentArgument::MediaTools => Self::MediaTools,
            ManagedComponentArgument::WhisperCli => Self::WhisperCli,
            ManagedComponentArgument::WhisperModel => Self::WhisperModel,
            ManagedComponentArgument::JsRuntimeDeno => Self::JsRuntimeDeno,
            ManagedComponentArgument::YtDlp => Self::YtDlp,
        }
    }
}

/// A canonical managed version key; anything else is rejected before it
/// reaches the store or any output.
fn parse_managed_version(value: &str) -> Result<ManagedVersionKey, ManagedVersionKeyError> {
    ManagedVersionKey::parse(value)
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
    // The optional OCR runner, `vsift-ocr-runner` (ADR 0030), recorded apart
    // from the other dependencies. A plain comment: a doc comment would become
    // this value's help and change how every value of the list is shown.
    OcrRunner,
}

impl SetupDependency {
    /// The runtime dependency this names; `None` for the OCR runner, which is
    /// an optional engine outside the runtime dependencies.
    pub(crate) const fn runtime_dependency(self) -> Option<RuntimeDependency> {
        match self {
            Self::Ffmpeg => Some(RuntimeDependency::Ffmpeg),
            Self::Ffprobe => Some(RuntimeDependency::Ffprobe),
            Self::Whisper => Some(RuntimeDependency::Whisper),
            Self::OcrRunner => None,
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

/// Foreground ingestion request: a local file, or a direct link to fetch.
#[derive(Args, Debug)]
#[command(group(ArgGroup::new("input").required(true).args(["source", "url"])))]
pub(crate) struct IngestArguments {
    /// Local source media path.
    pub source: Option<PathBuf>,
    /// Instead of a local file, a direct https:// link to a video file (MP4,
    /// MOV, MKV or WebM): it is fetched once into the session (port 443, no
    /// sign-in, public addresses only), then everything works offline as for
    /// a local file. `-` reads the link from standard input, which keeps a
    /// signed link out of the process list and the shell history. The link
    /// itself is never stored or printed.
    #[arg(long, value_name = "LINK")]
    pub url: Option<String>,
    /// Largest download for --url, in bytes (1 to 21474836480, 20 GiB);
    /// defaults to 4294967296 (4 GiB).
    #[arg(
        long,
        requires = "url",
        conflicts_with = "source",
        value_name = "BYTES",
        value_parser = clap::value_parser!(u64).range(1..=21_474_836_480)
    )]
    pub max_download_bytes: Option<u64>,
    /// Longest download for --url, in seconds (1 to 14400, 4 hours);
    /// defaults to 1800 (30 minutes).
    #[arg(
        long,
        requires = "url",
        conflicts_with = "source",
        value_name = "SECONDS",
        value_parser = clap::value_parser!(u32).range(1..=14_400)
    )]
    pub max_download_seconds: Option<u32>,
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
    /// When the recording started, for clock times on every result (time
    /// anchoring): RFC 3339, for example 2026-10-21T14:03:12+01:00 (local time
    /// with its UTC offset), 2026-10-21T13:03:12Z (UTC), or
    /// 2026-10-21T14:03:12 (local time, zone unknown). It is taken as exact
    /// and preferred over the time the video file itself records.
    #[arg(long, value_name = "TIME", value_parser = parse_recorded_at)]
    pub recorded_at: Option<ClockReading>,
}

fn parse_recorded_at(value: &str) -> Result<ClockReading, TimeAnchorError> {
    ClockReading::parse(value)
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
    /// Create a worker workspace at an explicit --session-root with an
    /// immutable operator policy.
    InitWorkspace(SessionInitWorkspaceArguments),
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
            Self::InitWorkspace(_) => CommandName::SessionInitWorkspace,
        }
    }
}

/// How a worker workspace's sessions publish.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum DurabilityArgument {
    /// Every acknowledged result survives an OS crash or power loss; only
    /// on Ubuntu 24.04 with local ext4.
    Durable,
    /// Consistent across a process crash, on every platform (development
    /// and CI).
    Ephemeral,
}

impl From<DurabilityArgument> for DurabilityRequirement {
    fn from(value: DurabilityArgument) -> Self {
        match value {
            DurabilityArgument::Durable => Self::Durable,
            DurabilityArgument::Ephemeral => Self::Ephemeral,
        }
    }
}

/// A worker workspace's operator policy (ADR 0021 D1, D2).
#[derive(Args, Debug)]
pub(crate) struct SessionInitWorkspaceArguments {
    /// How the workspace's sessions publish.
    #[arg(long, value_enum)]
    pub durability: DurabilityArgument,
    /// Admission capacity in weight units, 1 through 64: the most work the
    /// workspace runs at once (a whisper.cpp run weighs its threads, a
    /// visual window 2, a copy or an evidence extraction 1).
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=64))]
    pub admission_slots: u16,
    /// How long a session lives after it opens or is renewed, 1 through 720
    /// hours; defaults to 168.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=720))]
    pub retention_hours: Option<u64>,
}

/// The isolation a host asks for.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum HostIsolationArgument {
    /// Per-process containment only; no strict boundary is claimed.
    #[default]
    ProcessOnly,
    /// The strict Linux worker boundary: accepted only when the kernel
    /// attests a cgroup v2 with CPU, memory and process limits, a read-only
    /// root and no network but loopback; otherwise `ISOLATION_UNAVAILABLE`
    /// before any work.
    StrictLinux,
}

impl From<HostIsolationArgument> for IsolationProfile {
    fn from(value: HostIsolationArgument) -> Self {
        match value {
            HostIsolationArgument::ProcessOnly => Self::ProcessOnly,
            HostIsolationArgument::StrictLinux => Self::StrictLinux,
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
    ///
    /// Over a part of the range that was not transcribed the answer is partial
    /// and names the part: an empty page there is a gap, not a silence.
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

/// How much of each item a read answers with (#365).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum ViewArgument {
    /// Every field of every item: the answer every release gave (the default).
    #[default]
    Full,
    /// Each item without what the answer already states once: the revision's
    /// hashes, source and model details (or the index's source and size). It
    /// shapes the --json answer (readable text is the same for both), and
    /// --events jsonl refuses it, because a stream's records are complete
    /// evidence.
    Brief,
}

impl ViewArgument {
    /// The contract's view this option names.
    pub(crate) const fn result_view(self) -> ResultView {
        match self {
            Self::Full => ResultView::Full,
            Self::Brief => ResultView::Brief,
        }
    }
}

/// Bounded, pageable transcript read.
#[derive(Args, Debug)]
pub(crate) struct TranscriptGetArguments {
    /// Session containing the transcript.
    pub session: SessionId,
    /// Inclusive source-timeline start in microseconds; with --since it may be
    /// left out (from the start).
    #[arg(long, required_unless_present = "since")]
    pub from: Option<u64>,
    /// Exclusive source-timeline end in microseconds; with --since it may be
    /// left out (to the end).
    #[arg(long, required_unless_present = "since")]
    pub to: Option<u64>,
    /// Segments per page, 1 through 100; defaults to 20. The maximum stays 100;
    /// full segments are large, and --view brief makes a long page smaller.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub limit: Option<u16>,
    /// Opaque continuation token returned by the previous page of the same query.
    #[arg(long, conflicts_with = "since")]
    pub cursor: Option<String>,
    /// Revision to read (a `trv_` identity); defaults to the newest, which may
    /// be a revision still being transcribed (open, so the answer is partial).
    #[arg(long)]
    pub revision: Option<TranscriptRevisionId>,
    /// The since token of an earlier answer (`data.since.since_token`): read
    /// only what was published after it, in start order, in the revision that
    /// answer read. Use it to follow a transcription while it runs.
    #[arg(long, value_name = "TOKEN")]
    pub since: Option<String>,
    /// Which answer to give. `brief` keeps each segment's ids, times and
    /// `display_text` (and `carried_from` on a carried segment) and leaves out
    /// what the segments of a revision repeat; those members stay once in the
    /// revision block, which is the same in both views, so each segment is
    /// smaller. Read the full view for everything else.
    #[arg(long = "view", value_enum, default_value_t)]
    pub view: ViewArgument,
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
    /// Caller-chosen operation id (`op_` and 16 to 64 lowercase letters or
    /// digits): repeating the request with it returns the committed result
    /// without a new revision; the same id with another request is
    /// `IDEMPOTENCY_CONFLICT`. Checked before anything is read.
    #[arg(long, value_name = "OPERATION_ID")]
    pub operation_id: Option<OperationId>,
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
    /// Hits per page, 1 through 100; defaults to 20. The maximum stays 100;
    /// full items are large, and --view brief makes a long page smaller.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub limit: Option<u16>,
    /// Opaque continuation token returned by the previous page of the same search.
    #[arg(long, conflicts_with = "since")]
    pub cursor: Option<String>,
    /// Revision to search (a `trv_` identity); defaults to the newest, which
    /// may be a revision still being transcribed (open, so the answer is
    /// partial).
    #[arg(long)]
    pub revision: Option<TranscriptRevisionId>,
    /// The since token of an earlier answer (`data.since.since_token`): search
    /// only what was published after it, in start order, in the revision that
    /// answer read.
    #[arg(long, value_name = "TOKEN")]
    pub since: Option<String>,
    /// Which answer to give. `brief` keeps each found segment's ids, times and
    /// `display_text` (and `carried_from` on a carried segment) and leaves out
    /// what the segments of a revision repeat; those members stay once in the
    /// revision block, which is the same in both views, so each segment is
    /// smaller. Read the full view for everything else.
    #[arg(long = "view", value_enum, default_value_t)]
    pub view: ViewArgument,
}

/// A bounded request to read on-screen text off a session's frames (ADR 0030),
/// or, with `search`, a page of what was read.
#[derive(Args, Debug)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub(crate) struct OcrArguments {
    /// Search the on-screen text ocr has read instead of reading more.
    #[command(subcommand)]
    pub command: Option<OcrCommand>,
    /// Session whose video is read.
    #[arg(required = true)]
    pub session: Option<SessionId>,
    /// Inclusive source-timeline start in microseconds; requires --to. Omit
    /// both to read the whole video.
    #[arg(long, requires = "to")]
    pub from: Option<u64>,
    /// Exclusive source-timeline end in microseconds; requires --from. A range
    /// past the end of the video is clipped to it.
    #[arg(long, requires = "from")]
    pub to: Option<u64>,
    /// The period of the frames read between candidates, in microseconds, from
    /// 1000000 (one a second) to 600000000; defaults to 10000000 (ten seconds).
    #[arg(long, value_parser = clap::value_parser!(u64).range(1_000_000..=600_000_000))]
    pub period_us: Option<u64>,
    /// Most frames this call reads, 1 through 200; defaults to 40.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=200))]
    pub max_frames: Option<u16>,
    /// The OCR runner for this call only; otherwise the one setup configure
    /// ocr-runner recorded, else vsift-ocr-runner on PATH.
    #[arg(long)]
    pub runner: Option<PathBuf>,
    /// Inference threads, 1 through 8; defaults to 2.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=8))]
    pub threads: Option<u16>,
}

/// What `ocr` does instead of reading frames.
#[derive(Debug, Subcommand)]
pub(crate) enum OcrCommand {
    /// Page the on-screen text ocr has read for a literal query.
    ///
    /// Works with or without a transcript. Each hit cites the frame it was read
    /// on (time, timestamp, size), its region and how long it stayed; every hit
    /// is paged with --cursor. The answer is partial, and says how much of the
    /// range was read, until ocr has read every planned frame: text on a frame
    /// not read cannot be found. Runs no tool and writes nothing.
    Search(OcrSearchArguments),
}

/// A bounded literal search of a session's on-screen text (ADR 0030).
#[derive(Args, Debug)]
pub(crate) struct OcrSearchArguments {
    /// Session to search.
    pub session: SessionId,
    /// Literal query text, at most 256 bytes; it is never interpreted as code
    /// or a regular expression.
    #[arg(long)]
    pub query: String,
    /// Inclusive source-timeline start in microseconds; requires --to. Omit
    /// both to search the whole video.
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
    /// Candidates per page, 1 through 100; defaults to 20. The maximum stays
    /// 100; full items are large, and --view brief makes a long page smaller.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub limit: Option<u16>,
    /// Opaque continuation token returned by the previous page of the same
    /// range; a call with a cursor never analyses anything.
    #[arg(long)]
    pub cursor: Option<String>,
    /// Which answer to give. `brief` keeps each candidate's id, time, span,
    /// reasons, stability, the size of the change that opened it and its hash,
    /// and leaves out what every candidate of the index repeats; those members
    /// stay once in `item_constants` and the index block, so each candidate is
    /// smaller. Read the full view for everything else.
    #[arg(long = "view", value_enum, default_value_t)]
    pub view: ViewArgument,
    /// Which candidates to leave out of the pages. `periodic` leaves out the
    /// candidates proposed only because the screen did not change in their
    /// 10 s cell (reason `periodic_coverage`), so a shortlist of a long
    /// recording holds its changes; the answer then says how many it left out
    /// (`omitted`). Nothing is left out of the analysis or the coverage, and a
    /// cursor continues only pages of the same choice.
    #[arg(long = "omit", value_enum, default_value_t)]
    pub omit: OmitArgument,
}

/// Which candidates `candidates --omit` leaves out (#367).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum OmitArgument {
    /// Every candidate: the answer every release gave (the default).
    #[default]
    None,
    /// Candidates whose only reason is `periodic_coverage`.
    Periodic,
}

impl OmitArgument {
    /// The engine's omission this option names.
    pub(crate) const fn omission(self) -> vsift::CandidateOmission {
        match self {
            Self::None => vsift::CandidateOmission::Nothing,
            Self::Periodic => vsift::CandidateOmission::PeriodicCoverage,
        }
    }
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

/// Handoff namespace arguments (P13 PR 5).
#[derive(Args, Debug)]
pub(crate) struct HandoffArguments {
    #[command(subcommand)]
    pub command: HandoffCommand,
}

/// Handoff operations.
#[derive(Debug, Subcommand)]
pub(crate) enum HandoffCommand {
    /// Check a draft report and its vsift-handoff block against the skill's
    /// handoff schema and rules; reads the draft from standard input unless
    /// --file names it.
    Check(HandoffCheckArguments),
    /// Print a valid handoff that states nothing yet, with the session's
    /// ids filled in, to fill in instead of building the JSON from a template.
    Skeleton(HandoffSkeletonArguments),
}

impl HandoffCommand {
    /// Returns the public operation identifier.
    pub(crate) const fn operation_name(&self) -> CommandName {
        match self {
            Self::Check(_) => CommandName::HandoffCheck,
            Self::Skeleton(_) => CommandName::HandoffSkeleton,
        }
    }
}

/// Where the draft comes from, and the session to resolve it in.
#[derive(Args, Debug)]
pub(crate) struct HandoffCheckArguments {
    /// Absolute path of a file holding the whole draft report (at most
    /// 128 KiB of UTF-8); without it the draft is read from standard input.
    #[arg(long)]
    pub file: Option<PathBuf>,
    /// Open session whose records every cited segment and evidence id must
    /// resolve in; read only, never renewed.
    #[arg(long)]
    pub session: Option<SessionId>,
}

/// The session and question a handoff skeleton is written for (ADR 0038).
#[derive(Args, Debug)]
pub(crate) struct HandoffSkeletonArguments {
    /// Open session the handoff reports on; read only, never renewed.
    pub session: SessionId,
    /// The user's question in your own words (1 to 500 characters, one line),
    /// written into the handoff's question member.
    #[arg(long)]
    pub question: String,
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
    /// Execute one versioned job request in a worker workspace: ingest,
    /// retranscribe, candidates, retain and close, each once per operation
    /// id.
    Run(JobRequestArguments),
    /// Execute a finite JSON Lines file of job requests in a worker
    /// workspace, a bounded number at a time, each line independently.
    Batch(JobBatchArguments),
    /// Report one job: state, resumability, progress, result or failure.
    Status(JobIdentityArguments),
    /// Continue one interrupted job from its checkpoints.
    Resume(JobIdentityArguments),
    /// Cancel one job, or ask the process running it to stop.
    Cancel(JobIdentityArguments),
    /// Prepare a session as one job and follow it with job status: index its
    /// video for candidates, then transcribe it; in the background for a
    /// recording of ten minutes or more, in the foreground for a shorter one.
    ///
    /// A background job publishes the transcript as it goes: search and
    /// transcript get answer over the finished part, marked partial, while it
    /// runs. job status reports how far each step has come in seconds of the
    /// recording, its speed and an estimate of the time left. With --first,
    /// the range named is transcribed before everything else. job cancel or
    /// session close stops it, and what it published stays.
    Start(JobStartArguments),
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
            Self::Start(_) => CommandName::JobStart,
        }
    }
}

/// One request document and the operator's roots and controls (P11 PR 3,
/// ADR 0021).
#[derive(Args, Debug)]
pub(crate) struct JobRequestArguments {
    /// Versioned JSON request file (job-request v1, at most 64 KiB).
    #[arg(long)]
    pub request: PathBuf,
    /// Absolute directory the request's source and transcript paths are
    /// relative to; nothing outside it is read, and no link is followed.
    #[arg(long)]
    pub input_root: PathBuf,
    /// Absolute, existing directory a retain step writes its bundle below.
    #[arg(long)]
    pub bundle_root: Option<PathBuf>,
    /// After a shutdown signal, how long the running step may finish before
    /// it is cancelled, 0 through 300000 milliseconds; defaults to 0 (stop
    /// at once).
    #[arg(long, value_parser = clap::value_parser!(u64).range(0..=300_000), default_value_t = 0)]
    pub drain_timeout_ms: u64,
    /// How long a step waits (with jittered retries) for admission capacity
    /// before it answers BUSY, 0 through 60000 milliseconds; defaults to
    /// 60000.
    #[arg(long, value_parser = clap::value_parser!(u64).range(0..=60_000), default_value_t = 60_000)]
    pub admission_wait_ms: u64,
}

/// A finite JSON Lines request file and the operator's roots and controls
/// (P11 PR 4, ADR 0021 section 5).
#[derive(Args, Debug)]
pub(crate) struct JobBatchArguments {
    /// Regular file of at most 1,000 lines, each one job-request v1 object
    /// of at most 64 KiB, or blank.
    #[arg(long)]
    pub requests: PathBuf,
    /// Absolute directory every request's source and transcript paths are
    /// relative to; nothing outside it is read, and no link is followed.
    #[arg(long)]
    pub input_root: PathBuf,
    /// Absolute, existing directory retain steps write their bundles below.
    #[arg(long)]
    pub bundle_root: Option<PathBuf>,
    /// Requests run at once, 1 through 16 and at most the workspace's
    /// admission capacity; the next line is read only when one ends.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=16), default_value_t = 1)]
    pub concurrency: u16,
    /// After a shutdown signal, how long running steps may finish before
    /// they are cancelled, 0 through 300000 milliseconds; defaults to 0.
    #[arg(long, value_parser = clap::value_parser!(u64).range(0..=300_000), default_value_t = 0)]
    pub drain_timeout_ms: u64,
    /// How long a step waits (with jittered retries) for admission capacity
    /// before it answers BUSY, 0 through 60000 milliseconds; defaults to
    /// 60000.
    #[arg(long, value_parser = clap::value_parser!(u64).range(0..=60_000), default_value_t = 60_000)]
    pub admission_wait_ms: u64,
}

/// The session to prepare and, optionally, how (ADR 0033).
#[derive(Args, Debug)]
pub(crate) struct JobStartArguments {
    /// Session to prepare (a `ses_` identity).
    pub session: SessionId,
    /// quick runs in the foreground and answers when done; watched runs in
    /// the background and answers at once; queued waits its turn in the
    /// root's queue in the background. Defaults by the recording's length:
    /// quick under ten minutes, watched from ten minutes on.
    #[arg(long, value_enum)]
    pub mode: Option<JobModeArgument>,
    /// The window to transcribe first: its start and end in microseconds of
    /// the recording joined by a colon (60000000:70000000), at most 30 minutes
    /// long. When the user named a moment, it is ready within about one
    /// window's recognition, before the rest.
    #[arg(long, value_name = "FROM_US:TO_US", value_parser = parse_first_window)]
    pub first: Option<FirstWindow>,
}

/// A first window given as `<from_us>:<to_us>`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FirstWindow {
    /// Inclusive start, in microseconds.
    pub(crate) from_us: u64,
    /// Exclusive end, in microseconds.
    pub(crate) to_us: u64,
}

/// Parses `<from_us>:<to_us>`: two whole numbers of microseconds, the first
/// smaller, at most 30 minutes apart.
fn parse_first_window(text: &str) -> Result<FirstWindow, String> {
    let (from, to) = text
        .split_once(':')
        .ok_or_else(|| "expected <from_us>:<to_us>".to_owned())?;
    let number = |part: &str| {
        part.parse::<u64>()
            .map_err(|_| "each side is a whole number of microseconds".to_owned())
    };
    let (from_us, to_us) = (number(from)?, number(to)?);
    if to_us <= from_us {
        return Err("the window must end after it starts".to_owned());
    }
    if to_us - from_us > vsift::MAX_FIRST_WINDOW_MICROS {
        return Err("the window is at most 30 minutes (1800000000 microseconds)".to_owned());
    }
    Ok(FirstWindow { from_us, to_us })
}

/// How a started job runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum JobModeArgument {
    /// In the foreground: the command answers when the job is done.
    Quick,
    /// In the background: the command answers once a runner holds the job.
    Watched,
    /// In the root's queue, after the jobs queued before it, one at a time at
    /// the whole capacity, giving way at every chunk to anyone who waits: for
    /// several recordings, or when there is no rush.
    Queued,
}

impl From<JobModeArgument> for vsift::JobMode {
    fn from(value: JobModeArgument) -> Self {
        match value {
            JobModeArgument::Quick => Self::Quick,
            JobModeArgument::Watched => Self::Watched,
            JobModeArgument::Queued => Self::Queued,
        }
    }
}

/// One validated job identifier; the job's session is found through the
/// session root's job index, so no session is given.
#[derive(Args, Debug)]
pub(crate) struct JobIdentityArguments {
    /// Job to inspect or change (a `job_` identity).
    pub job: JobId,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use clap::{CommandFactory, Parser};
    use vsift_contract::CommandName;

    use super::{Cli, TYPICAL_INVESTIGATION_HELP};
    use crate::config::{DEFAULT_PROBE_TIMEOUT_SECONDS, MAX_PROBE_TIMEOUT_SECONDS};

    /// Every operation identifier the parser can produce: the command path of
    /// each leaf subcommand joined with `.`, as `CommandName` documents it.
    fn parsed_operation_identifiers() -> BTreeSet<String> {
        let root = Cli::command();
        let mut identifiers = BTreeSet::new();
        for namespace in root.get_subcommands() {
            if namespace.has_subcommands() {
                // `ocr` both runs itself and has `ocr search`; plain `setup`
                // is the setup door beside its subcommands (ADR 0036).
                if namespace.is_args_conflicts_with_subcommands_set() {
                    identifiers.insert(namespace.get_name().to_owned());
                }
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

    /// The command lines of [`TYPICAL_INVESTIGATION_HELP`], with each
    /// placeholder replaced by a value of the right kind.
    fn help_command_lines() -> Vec<Vec<String>> {
        const SUBSTITUTIONS: [(&str, &str); 6] = [
            ("<video>", "video.mp4"),
            ("<session>", "ses_0123456789abcdef"),
            ("<us>", "1000"),
            ("<words>", "error"),
            ("<file>", "walkthrough.srt"),
            ("<microseconds>", "500000"),
        ];
        TYPICAL_INVESTIGATION_HELP
            .lines()
            .map(str::trim)
            .filter_map(|line| {
                // A step line is "N. vsift ...", a second command of a step is
                // "vsift ..."; an option offered inside prose is not a command.
                let command = match line.split_once(". vsift ") {
                    Some((step, rest)) if step.chars().all(|c| c.is_ascii_digit()) => {
                        format!("vsift {rest}")
                    }
                    _ => line
                        .strip_prefix("vsift ")
                        .map(|rest| format!("vsift {rest}"))?,
                };
                Some(
                    command
                        .split_whitespace()
                        .map(|word| {
                            SUBSTITUTIONS
                                .iter()
                                .fold(word.to_owned(), |word, (from, to)| word.replace(from, to))
                        })
                        .collect(),
                )
            })
            .collect()
    }

    /// The worked example in the top-level help names only commands and options
    /// the parser accepts, with the arguments it shows.
    #[test]
    fn every_command_the_typical_investigation_names_parses() {
        let lines = help_command_lines();
        assert!(
            lines.len() >= 8,
            "the example lost commands: {} lines found",
            lines.len()
        );
        for line in lines {
            let parsed = Cli::try_parse_from(&line);
            assert!(parsed.is_ok(), "{line:?}: {:?}", parsed.err());
        }
    }

    /// The example also names the optional transcript options, the job
    /// commands of a transcription and the operator-only options; each is real.
    #[test]
    fn the_options_the_example_mentions_in_prose_are_real_options() {
        for line in [
            [
                "vsift",
                "ingest",
                "video.mp4",
                "--transcript",
                "walkthrough.srt",
                "--transcript-offset",
                "500000",
            ]
            .as_slice(),
            ["vsift", "job", "start", "ses_0123456789abcdef"].as_slice(),
            ["vsift", "job", "status", "job_0123456789abcdef"].as_slice(),
            [
                "vsift",
                "--session-root",
                "/x",
                "--host-isolation",
                "process-only",
                "session",
                "list",
            ]
            .as_slice(),
        ] {
            let parsed = Cli::try_parse_from(line);
            assert!(parsed.is_ok(), "{line:?}: {:?}", parsed.err());
        }
        for needle in [
            "--transcript <file>",
            "--transcript-offset <microseconds>",
            "vsift job start <session> --json",
            "vsift job status <job> --json",
            "never
       build the JSON by search and replace",
            "--session-root and --host-isolation are for operators",
        ] {
            assert!(TYPICAL_INVESTIGATION_HELP.contains(needle), "{needle}");
        }
    }

    /// Issue #391: the help of `setup check --timeout-seconds` said "for each
    /// dependency probe", which it never was. It is one deadline for all the
    /// probes, and the help states its range and its default, taken from the
    /// figures the configuration applies. The parser's range is unchanged, and
    /// `setup plan` still has no such option (it would have to be given to
    /// `setup install` too, which probes again to compare the plan).
    #[test]
    fn the_setup_check_deadline_option_states_its_default_and_keeps_its_range()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut root = Cli::command();
        let check = root
            .find_subcommand_mut("setup")
            .and_then(|setup| setup.find_subcommand_mut("check"))
            .ok_or("setup check is not a command")?;
        let help = check
            .render_long_help()
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let stated = format!(
            "Seconds allowed for all the dependency probes together, 1 to {MAX_PROBE_TIMEOUT_SECONDS} (default {DEFAULT_PROBE_TIMEOUT_SECONDS})."
        );
        assert!(help.contains(&stated), "{help}");
        assert!(!help.contains("for each dependency probe"), "{help}");

        let accepts = |arguments: &[&str]| Cli::try_parse_from(arguments).is_ok();
        for seconds in ["1", "30", "60"] {
            assert!(
                accepts(&["vsift", "setup", "check", "--timeout-seconds", seconds]),
                "{seconds}"
            );
        }
        for refused in ["0", "61", "-1", "soon"] {
            assert!(
                !accepts(&["vsift", "setup", "check", "--timeout-seconds", refused]),
                "{refused}"
            );
        }
        assert!(accepts(&["vsift", "setup", "plan", "--profile", "desktop"]));
        assert!(!accepts(&[
            "vsift",
            "setup",
            "plan",
            "--profile",
            "desktop",
            "--timeout-seconds",
            "30"
        ]));
        Ok(())
    }

    /// The numbers the example states are the parser's own.
    #[test]
    fn the_ranges_the_example_states_are_the_parsers_ranges() {
        let accepts = |arguments: &[&str]| Cli::try_parse_from(arguments).is_ok();
        let session = "ses_0123456789abcdef";
        for limit in ["1", "100"] {
            assert!(accepts(&[
                "vsift", "search", session, "--query", "x", "--limit", limit
            ]));
            assert!(accepts(&[
                "vsift",
                "transcript",
                "get",
                session,
                "--from",
                "0",
                "--to",
                "1",
                "--limit",
                limit
            ]));
            assert!(accepts(&[
                "vsift",
                "candidates",
                session,
                "--from",
                "0",
                "--to",
                "1",
                "--limit",
                limit
            ]));
            assert!(accepts(&[
                "vsift",
                "frame",
                "burst",
                session,
                "--from",
                "0",
                "--to",
                "1",
                "--max-frames",
                limit
            ]));
        }
        for limit in ["0", "101"] {
            assert!(!accepts(&[
                "vsift", "search", session, "--query", "x", "--limit", limit
            ]));
            assert!(!accepts(&[
                "vsift",
                "transcript",
                "get",
                session,
                "--from",
                "0",
                "--to",
                "1",
                "--limit",
                limit
            ]));
            assert!(!accepts(&[
                "vsift",
                "candidates",
                session,
                "--from",
                "0",
                "--to",
                "1",
                "--limit",
                limit
            ]));
            assert!(!accepts(&[
                "vsift",
                "frame",
                "burst",
                session,
                "--from",
                "0",
                "--to",
                "1",
                "--max-frames",
                limit
            ]));
        }
        assert!(TYPICAL_INVESTIGATION_HELP.contains("--limit takes 1 to 100 (default 20)"));
        assert!(TYPICAL_INVESTIGATION_HELP.contains(
            "--max-frames 1 to 100
(default 12)"
        ));
    }

    /// The defaults and ceilings the example prints (20 and 100 for a page, 12
    /// and 100 for a burst) are the engine's own constants, not numbers copied
    /// into the text: a change to either fails this until the text follows.
    #[test]
    fn the_defaults_and_ceilings_the_example_prints_are_the_engines() {
        use vsift_domain::PageLimit;

        assert_eq!(PageLimit::DEFAULT.get(), 20);
        assert_eq!(PageLimit::MAX, 100);
        assert_eq!(vsift::DEFAULT_BURST_FRAMES, 12);
        assert_eq!(vsift_application::MAX_FRAMES_PER_CALL, 100);
        let page = format!(
            "--limit takes 1 to {} (default {})",
            PageLimit::MAX,
            PageLimit::DEFAULT.get()
        );
        let burst = format!(
            "--max-frames 1 to {}\n(default {})",
            vsift_application::MAX_FRAMES_PER_CALL,
            vsift::DEFAULT_BURST_FRAMES
        );
        assert!(TYPICAL_INVESTIGATION_HELP.contains(&page), "{page}");
        assert!(TYPICAL_INVESTIGATION_HELP.contains(&burst), "{burst}");
        // The suggestion the text makes (20 results, a handful of frames) is
        // the default page, and a burst command line it shows asks for fewer
        // frames than the default.
        assert!(TYPICAL_INVESTIGATION_HELP.contains("--limit 20"));
        assert!(TYPICAL_INVESTIGATION_HELP.contains("--max-frames 4"));
        const { assert!(4 < vsift::DEFAULT_BURST_FRAMES) };
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
            Some(Command::Ingest(arguments)) => {
                assert_eq!(
                    arguments.source.as_deref().map(std::path::Path::as_os_str),
                    Some(source.as_os_str())
                );
            }
            _ => return Err("ingest command did not preserve the source path".into()),
        }
        Ok(())
    }
}
