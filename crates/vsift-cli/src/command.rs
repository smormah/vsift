//! Typed public command hierarchy.

use std::path::PathBuf;

use clap::{ArgGroup, Args, CommandFactory, Parser, Subcommand, ValueEnum};
use vsift::{
    AgentClient, ClockId, ClockReading, CropRectangle, DurabilityRequirement, EvidenceId,
    FrameSelection, GapId, IsolationProfile, JobId, ManagedComponent, ManagedVersionKey,
    ManagedVersionKeyError, OperationId, OptionalComponent, OptionalComponentSet,
    RuntimeDependency, SessionId, SetupProfile, TimeAnchorError, TranscriptRevisionId, UtcOffset,
    VisualCandidateId,
};
use vsift_contract::{CommandName, ResultView};

use crate::help_text;

/// The parser, with help laid out for where it goes.
///
/// clap's `wrap_help` wraps a terminal's help to its width (at most 100
/// columns), and with no terminal wraps at 100 and reads `COLUMNS`. Width 0 is
/// clap's source formatting: piped help stays exactly as written, whatever the
/// environment says, which is what a program or an assistant reads and what
/// the generated command reference is made from (design note E4).
pub(crate) fn parser(standard_output_is_terminal: bool) -> clap::Command {
    let command = Cli::command();
    if standard_output_is_terminal {
        command
    } else {
        command.term_width(0)
    }
}

/// Complete public R0 command parser.
#[derive(Debug, Parser)]
#[command(
    name = "vsift",
    // The package version, and in a release build its source commit
    // (build.rs, ADR 0023).
    version = env!("VSIFT_VERSION_TEXT"),
    about = help_text::ROOT_ABOUT,
    override_usage = help_text::ROOT_USAGE,
    help_template = help_text::ROOT_HELP_TEMPLATE,
    after_help = help_text::ROOT_SHORT_FOOTER,
    after_long_help = help_text::TYPICAL_INVESTIGATION_HELP
)]
pub(crate) struct Cli {
    /// Machine-readable output: one versioned JSON result.
    #[arg(long, global = true)]
    pub json: bool,

    /// Emit versioned evidence, progress and terminal records as JSON Lines.
    #[arg(long, global = true, value_enum, hide_short_help = true)]
    pub events: Option<EventFormat>,

    /// For operators who run a worker host; an agent leaves it out. Explicit
    /// private disposable-session root; defaults to the per-user cache.
    #[arg(long, global = true, hide_short_help = true)]
    pub session_root: Option<PathBuf>,

    /// For operators who run a worker host; an agent leaves it out. Isolation
    /// to run under; strict-linux is attested before any work.
    #[arg(
        long,
        global = true,
        value_enum,
        default_value_t,
        hide_short_help = true
    )]
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
    /// Check and set up what you need (run it any time).
    ///
    /// Plain `vsift setup`, in a terminal, shows what works and what is
    /// missing, offers to install what can be installed here, with each
    /// download's source, size and licence, and installs nothing until you
    /// type yes. Without a terminal it only reports. The commands below are
    /// what AI assistants and scripts use; they never ask anything.
    #[command(after_help = help_text::SETUP)]
    Setup(SetupArguments),
    /// Open a video as a private session.
    #[command(after_help = help_text::INGEST)]
    Ingest(IngestArguments),
    /// List, keep or close sessions.
    #[command(after_help = help_text::SESSION)]
    Session(SessionArguments),
    /// Read or redo a session's transcript.
    #[command(after_help = help_text::TRANSCRIPT)]
    Transcript(TranscriptArguments),
    /// Find words in what was said.
    ///
    /// Over a part of the range that was not transcribed the answer is partial
    /// and names the part: a word said there cannot be found, and that is not
    /// evidence that none was said.
    #[command(after_help = help_text::SEARCH)]
    Search(SearchArguments),
    /// List the moments the screen changed.
    #[command(after_help = help_text::CANDIDATES)]
    Candidates(CandidatesArguments),
    /// Show where the screen changed, with the frames before and after.
    ///
    /// Each change of the range is a moment the screen changed (from
    /// candidates), the exact frame before and after it (ordinary evidence,
    /// as frame get gives it) and the rectangles where those two frames
    /// differ, largest first. It says which pixels differ, never what the
    /// change means. Cite a change by its before and after evidence ids
    /// (evd_...), never by its change id (chg_...). At most 20 changes a
    /// page; a call without --cursor first analyses missing parts of the
    /// range, as candidates does.
    #[command(after_help = help_text::CHANGES)]
    Changes(ChangesArguments),
    /// Read on-screen text.
    ///
    /// With the optional OCR runner, opt-in and bounded: each call reads at most --max-frames frames of the
    /// range's plan (every visual candidate's frame and one frame every
    /// --period-us), then stops; run it again to continue. The answer counts the
    /// frames read, failed and pending, and is partial until the plan is read.
    /// ocr search then pages the text it read, each hit citing its frame, time
    /// and region, with or without a transcript (search's first page shows it
    /// beside the transcript).
    #[command(after_help = help_text::OCR)]
    Ocr(OcrArguments),
    /// Find the clocks on the screen and how they run.
    ///
    /// It works from what ocr read. A time on a screen counts as a clock only
    /// if it moves with the video.
    /// Each clock is its own source and is never converted between zones. The
    /// answer says how each moves against the video: precision to the second,
    /// gaps, freezes and what disagrees, with the frames that prove it.
    /// Bounded and resumable like ocr: run ocr first, then this, then again to
    /// continue. A gap and an observed offset change no clock time: only
    /// --accept-gap and --screen-zone do, and only with what the user stated.
    #[command(after_help = help_text::CLOCKS)]
    Clocks(ClocksArguments),
    /// Get the exact frame at a moment.
    #[command(after_help = help_text::FRAME)]
    Frame(FrameArguments),
    /// Save a short clip for a person or a speech tool to play.
    ///
    /// The clip is a WAV file of at most 30 seconds, written into the
    /// session's private folder. A person plays it, or hands it to a speech
    /// tool of their own. A coding agent cannot listen to it, and reading the
    /// file's bytes (for example with base64) tells it nothing: what was said
    /// is in the transcript, which vsift transcript get reads.
    #[command(after_help = help_text::AUDIO)]
    Audio(AudioArguments),
    /// Cut out part of a frame.
    #[command(after_help = help_text::CROP)]
    Crop(CropArguments),
    /// Check and search the recordings you kept.
    #[command(after_help = help_text::BUNDLE)]
    Bundle(BundleArguments),
    /// Run and follow long work on a session.
    #[command(after_help = help_text::JOB)]
    Job(JobArguments),
    /// Start or check a report that cites its evidence.
    #[command(after_help = help_text::HANDOFF)]
    Handoff(HandoffArguments),
    /// Add the skill to your AI assistant, or print one of its files.
    #[command(after_help = help_text::SKILL)]
    Skill(SkillArguments),
    /// Make a transcript, subtitles and screen changes beside a video.
    ///
    /// The same as  vsift <video> . Writes <name>.txt (a readable transcript), <name>.srt (subtitles most
    /// video players pick up) and <name>-vsift/ (the screen changes, with a
    /// frame for each and their list) beside the video, or into --output. It
    /// never replaces a file: a name that is taken is refused before any work.
    /// With --keep it also keeps the recording for  vsift bundle search : the
    /// bundle holds the transcript and the frames, not the video or any
    /// on-screen text.
    /// Each file names the video's SHA-256 and the ids of the run; every
    /// frame's SHA-256 is in the list. Video tools and speech recognition must
    /// be set up first (vsift setup check). Not for AI assistants: they use the
    /// session commands.
    #[command(after_help = help_text::WATCH)]
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
    /// Keep the recording for later search, in a private subfolder of this
    /// folder named after the video (an existing folder, or a new one whose
    /// parent exists). The same folder takes every recording you keep; find
    /// them with  vsift bundle search <FOLDER> . The video is not copied in,
    /// and on-screen text is not read.
    #[arg(long, value_name = "FOLDER")]
    pub keep: Option<PathBuf>,
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
    #[command(after_help = help_text::SETUP_CHECK)]
    Check(SetupCheckArguments),
    /// Create a reviewable installation plan without applying it.
    #[command(after_help = help_text::SETUP_PLAN)]
    Plan(SetupPlanArguments),
    /// Apply an unchanged, explicitly accepted installation plan.
    #[command(after_help = help_text::SETUP_INSTALL)]
    Install(SetupInstallArguments),
    /// Diagnose the managed tools and plan their repair; changes nothing.
    #[command(after_help = help_text::SETUP_REPAIR)]
    Repair,
    /// List each managed component's versions, the selected one, and whether each verifies.
    #[command(after_help = help_text::SETUP_LIST)]
    List,
    /// Remove a managed version, a whole managed component, or abandoned stages.
    #[command(after_help = help_text::SETUP_REMOVE)]
    Remove(SetupRemoveArguments),
    /// Select a component's previous managed version, or a named installed one.
    #[command(after_help = help_text::SETUP_ROLLBACK)]
    Rollback(SetupRollbackArguments),
    /// Register explicitly supplied dependency configuration.
    #[command(after_help = help_text::SETUP_CONFIGURE)]
    Configure(SetupConfigureArguments),
    /// Register an explicit user-managed local ASR model file.
    #[command(after_help = help_text::SETUP_CONFIGURE_MODEL)]
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
    #[arg(long, value_enum, hide_short_help = true)]
    pub profile: Option<ExecutionProfile>,

    /// Use this absolute `FFmpeg` executable path instead of searching `PATH`.
    #[arg(long, hide_short_help = true)]
    pub ffmpeg: Option<PathBuf>,

    /// Use this absolute `FFprobe` executable path instead of searching `PATH`.
    #[arg(long, hide_short_help = true)]
    pub ffprobe: Option<PathBuf>,

    /// Use this absolute whisper.cpp CLI path instead of searching `PATH`.
    #[arg(long, hide_short_help = true)]
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
    // The user's yt-dlp for public video pages (ADR 0028 section 4.6), and the
    // JavaScript runtime it runs (Deno only in 0.3.0). Plain comments for the
    // reason above. Spelled with a hyphen like the OCR runner; `yt_dlp` is a
    // hidden alias because `--include online_links` and the JSON results use
    // the underscore. The runtime has no alias: an alias on a stable
    // command line stays for good.
    #[value(name = "yt-dlp", alias = "yt_dlp")]
    YtDlp,
    #[value(name = "js-runtime")]
    JsRuntime,
}

impl SetupDependency {
    /// The runtime dependency this names; `None` for the optional tools
    /// (the OCR runner, yt-dlp and its runtime), which are outside the
    /// runtime dependencies.
    pub(crate) const fn runtime_dependency(self) -> Option<RuntimeDependency> {
        match self {
            Self::Ffmpeg => Some(RuntimeDependency::Ffmpeg),
            Self::Ffprobe => Some(RuntimeDependency::Ffprobe),
            Self::Whisper => Some(RuntimeDependency::Whisper),
            Self::OcrRunner | Self::YtDlp | Self::JsRuntime => None,
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

/// Foreground ingestion request: a local file, or a link to fetch.
#[derive(Args, Debug)]
#[command(group(ArgGroup::new("input").required(true).args(["source", "url"])))]
pub(crate) struct IngestArguments {
    /// Local source media path.
    pub source: Option<PathBuf>,
    /// Instead of a local file, an https:// link to a video file or page.
    ///
    /// The link names one file (MP4, MOV, MKV or WebM): it is fetched once into
    /// the session (port 443, no
    /// sign-in, public addresses only), then everything works offline as for
    /// a local file. A public video page is read through yt-dlp when one is
    /// set up (setup check says). `-` reads the link from standard input,
    /// which keeps a signed link out of the process list and the shell
    /// history. The link itself is never stored or printed.
    #[arg(long, value_name = "LINK")]
    pub url: Option<String>,
    /// Largest download for --url, in bytes (1 to 21474836480, 20 GiB);
    /// defaults to 4294967296 (4 GiB).
    #[arg(
        long,
        hide_short_help = true,
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
        hide_short_help = true,
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
        hide_short_help = true,
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
    #[arg(long, value_name = "TIME", value_parser = parse_recorded_at, hide_short_help = true)]
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
    #[command(after_help = help_text::SESSION_LIST)]
    List(SessionScanArguments),
    /// Read one session's status.
    #[command(after_help = help_text::SESSION_STATUS)]
    Status(SessionIdentityArguments),
    /// Close one session after active work settles.
    #[command(after_help = help_text::SESSION_CLOSE)]
    Close(SessionIdentityArguments),
    /// Renew one eligible ephemeral session.
    #[command(after_help = help_text::SESSION_RENEW)]
    Renew(SessionIdentityArguments),
    /// Export one session as a validated portable bundle.
    #[command(after_help = help_text::SESSION_RETAIN)]
    Retain(SessionRetainArguments),
    /// Find or remove expired owned sessions.
    #[command(after_help = help_text::SESSION_CLEAN)]
    Clean(SessionCleanArguments),
    /// Create a worker workspace at an explicit --session-root with an
    /// immutable operator policy.
    #[command(after_help = help_text::SESSION_INIT_WORKSPACE)]
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
    #[command(after_help = help_text::TRANSCRIPT_GET)]
    Get(TranscriptGetArguments),
    /// Transcribe the session's speech locally with whisper.cpp into a new
    /// revision: the whole video, or one range of it.
    #[command(after_help = help_text::TRANSCRIPT_RETRANSCRIBE)]
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
    #[arg(long, conflicts_with = "since", hide_short_help = true)]
    pub cursor: Option<String>,
    /// Revision to read (a `trv_` identity); defaults to the newest, which may
    /// be a revision still being transcribed (open, so the answer is partial).
    #[arg(long, hide_short_help = true)]
    pub revision: Option<TranscriptRevisionId>,
    /// The since token of an earlier answer (`data.since.since_token`): read
    /// only what was published after it, in start order, in the revision that
    /// answer read. Use it to follow a transcription while it runs.
    #[arg(long, value_name = "TOKEN", hide_short_help = true)]
    pub since: Option<String>,
    /// Which answer to give. `brief` keeps each segment's ids, times and
    /// `display_text` (and `carried_from` on a carried segment) and leaves out
    /// what the segments of a revision repeat; those members stay once in the
    /// revision block, which is the same in both views, so each segment is
    /// smaller. Read the full view for everything else.
    #[arg(long = "view", value_enum, default_value_t, hide_short_help = true)]
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
    #[arg(long, value_name = "OPERATION_ID", hide_short_help = true)]
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
    #[arg(long, conflicts_with = "since", hide_short_help = true)]
    pub cursor: Option<String>,
    /// Revision to search (a `trv_` identity); defaults to the newest, which
    /// may be a revision still being transcribed (open, so the answer is
    /// partial).
    #[arg(long, hide_short_help = true)]
    pub revision: Option<TranscriptRevisionId>,
    /// The since token of an earlier answer (`data.since.since_token`): search
    /// only what was published after it, in start order, in the revision that
    /// answer read.
    #[arg(long, value_name = "TOKEN", hide_short_help = true)]
    pub since: Option<String>,
    /// Which answer to give. `brief` keeps each found segment's ids, times and
    /// `display_text` (and `carried_from` on a carried segment) and leaves out
    /// what the segments of a revision repeat; those members stay once in the
    /// revision block, which is the same in both views, so each segment is
    /// smaller. Read the full view for everything else.
    #[arg(long = "view", value_enum, default_value_t, hide_short_help = true)]
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
    #[arg(long, value_parser = clap::value_parser!(u64).range(1_000_000..=600_000_000), hide_short_help = true)]
    pub period_us: Option<u64>,
    /// Most frames this call reads, 1 through 200; defaults to 40.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=200))]
    pub max_frames: Option<u16>,
    /// The OCR runner for this call only; otherwise the one setup configure
    /// ocr-runner recorded, else vsift-ocr-runner on PATH.
    #[arg(long, hide_short_help = true)]
    pub runner: Option<PathBuf>,
    /// Inference threads, 1 through 8; defaults to 2.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=8), hide_short_help = true)]
    pub threads: Option<u16>,
}

/// A bounded request to find a session's clocks (ADR 0029, stage 2).
#[derive(Args, Debug)]
pub(crate) struct ClocksArguments {
    /// Session whose clocks are found.
    pub session: SessionId,
    /// Most frames this call reads around rollovers, 0 through 200; defaults
    /// to 40. With 0 it reads nothing and needs no OCR runner.
    #[arg(long, value_parser = clap::value_parser!(u16).range(0..=200))]
    pub max_frames: Option<u16>,
    /// The OCR runner for this call only; otherwise the one setup configure
    /// ocr-runner recorded, else vsift-ocr-runner on PATH.
    #[arg(long, hide_short_help = true)]
    pub runner: Option<PathBuf>,
    /// Inference threads, 1 through 8; defaults to 2.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=8), hide_short_help = true)]
    pub threads: Option<u16>,
    /// A clock's zone as the user stated it: <clock>=<+HH:MM or -HH:MM>, for
    /// example c1=-05:00. Repeatable. Never pass an observation you were not told.
    #[arg(long, value_name = "CLOCK=OFFSET", value_parser = crate::clocks::parse_screen_zone)]
    pub screen_zone: Vec<(ClockId, UtcOffset)>,
    /// A gap the user says is real, so clock times after it move by its length.
    /// Repeatable. Never pass a gap you were not told to accept.
    #[arg(long, value_name = "GAP", value_parser = crate::clocks::parse_gap)]
    pub accept_gap: Vec<GapId>,
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
    #[command(after_help = help_text::OCR_SEARCH)]
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
    #[arg(long, hide_short_help = true)]
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
    #[arg(long, hide_short_help = true)]
    pub cursor: Option<String>,
    /// Which answer to give. `brief` keeps each candidate's id, time, span,
    /// reasons, stability, the size of the change that opened it and its hash,
    /// and leaves out what every candidate of the index repeats; those members
    /// stay once in `item_constants` and the index block, so each candidate is
    /// smaller. Read the full view for everything else.
    #[arg(long = "view", value_enum, default_value_t, hide_short_help = true)]
    pub view: ViewArgument,
    /// Which candidates to leave out of the pages. `periodic` leaves out the
    /// candidates proposed only because the screen did not change in their
    /// 10 s cell (reason `periodic_coverage`), so a shortlist of a long
    /// recording holds its changes; the answer then says how many it left out
    /// (`omitted`). Nothing is left out of the analysis or the coverage, and a
    /// cursor continues only pages of the same choice.
    #[arg(long = "omit", value_enum, default_value_t, hide_short_help = true)]
    pub omit: OmitArgument,
}

/// Screen-change page request (ADR 0034); missing windows of the range are
/// analysed first, at most 30 minutes of video per call, as for
/// `candidates`.
#[derive(Args, Debug)]
pub(crate) struct ChangesArguments {
    /// Session whose video is read.
    pub session: SessionId,
    /// Inclusive source-timeline start in microseconds; a change is listed
    /// when its after frame lies in the range.
    #[arg(long)]
    pub from: u64,
    /// Exclusive source-timeline end in microseconds; a range past the end
    /// of the video is clipped to it.
    #[arg(long)]
    pub to: u64,
    /// Changes per page, 1 through 20; defaults to 10. Each change brings
    /// up to two frames.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=20))]
    pub limit: Option<u16>,
    /// Opaque continuation token returned by the previous page of the same
    /// range and --limit; a call with a cursor never analyses anything.
    #[arg(long, hide_short_help = true)]
    pub cursor: Option<String>,
    /// Also cut each change's two largest regions out of its before and
    /// after frames, at native size: at most 16 crops a call, each a crop
    /// evidence item. Pass it again with --cursor.
    #[arg(long)]
    pub crops: bool,
}

impl ChangesArguments {
    /// The engine's crop choice this flag names: the flag is a boolean only
    /// here, at the edge of the command line.
    pub(crate) const fn crops(&self) -> vsift::ScreenChangeCrops {
        if self.crops {
            vsift::ScreenChangeCrops::LargestRegions
        } else {
            vsift::ScreenChangeCrops::NoCrops
        }
    }
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
    #[command(after_help = help_text::FRAME_GET)]
    Get(FrameGetArguments),
    /// Extract consecutive frames on each side of an earlier frame.
    #[command(after_help = help_text::FRAME_NEIGHBOURS)]
    Neighbours(NeighbourArguments),
    /// Extract distinct frames at evenly spaced times over a range.
    #[command(after_help = help_text::FRAME_BURST)]
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
    #[arg(
        long,
        value_enum,
        requires = "at",
        conflicts_with = "candidate",
        hide_short_help = true
    )]
    pub select: Option<FrameSelectArgument>,
    /// How far the frame may lie from the time, 0 through 10000000
    /// microseconds; defaults to 1000000.
    #[arg(
        long,
        hide_short_help = true,
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
    #[command(after_help = help_text::BUNDLE_VALIDATE)]
    Validate(BundleValidateArguments),
    /// Search the transcript and on-screen text of retained bundles.
    ///
    /// Each path is a bundle, or a folder of bundles read one level down: at
    /// most 64 bundles and 1 GiB are read in a call. The session root is never
    /// opened, so only what you kept and named is searched. Nothing is written
    /// and no tool runs.
    ///
    /// Every result says, for each recording, what could be searched. A hit is
    /// a lead, not a citation: to look at a moment or cite it, open the source
    /// with ingest and cite from that session.
    #[command(after_help = help_text::BUNDLE_SEARCH)]
    Search(BundleSearchArguments),
}

impl BundleCommand {
    /// Returns the public operation identifier.
    pub(crate) const fn operation_name(&self) -> CommandName {
        match self {
            Self::Validate(_) => CommandName::BundleValidate,
            Self::Search(_) => CommandName::BundleSearch,
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
    #[command(after_help = help_text::HANDOFF_CHECK)]
    Check(HandoffCheckArguments),
    /// Print a valid handoff that states nothing yet, with the session's
    /// ids filled in, to fill in instead of building the JSON from a template.
    #[command(after_help = help_text::HANDOFF_SKELETON)]
    Skeleton(HandoffSkeletonArguments),
    /// Check a draft report with the session's records and, only when it
    /// passes, retain the session to the new directory --retain names or
    /// close it (--close); without either, check only. Reads the draft from
    /// standard input unless --file names it.
    #[command(after_help = help_text::HANDOFF_FINISH)]
    Finish(HandoffFinishArguments),
}

impl HandoffCommand {
    /// Returns the public operation identifier.
    pub(crate) const fn operation_name(&self) -> CommandName {
        match self {
            Self::Check(_) => CommandName::HandoffCheck,
            Self::Skeleton(_) => CommandName::HandoffSkeleton,
            Self::Finish(_) => CommandName::HandoffFinish,
        }
    }
}

/// The session a draft is finished in, where the draft comes from, and the
/// one write asked for, if any (0.3.0, M5b).
///
/// `--retain` and `--close` are flags at this edge only: the command turns
/// them into one [`vsift_contract::HandoffFinishRequest`], and clap refuses
/// both together and `--include-source` without `--retain`.
#[derive(Args, Debug)]
pub(crate) struct HandoffFinishArguments {
    /// Session the draft reports on. Every cited segment and evidence id
    /// must resolve in its records; it is read without renewing it.
    pub session: SessionId,
    /// Only on the user's explicit request: after a passing draft that
    /// states lifecycle.action retained, retain the session to this new
    /// directory, exactly as session retain --output does.
    #[arg(long, value_name = "NEW_DIRECTORY", conflicts_with = "close")]
    pub retain: Option<PathBuf>,
    /// With --retain only, and only if the user asked for the video: also
    /// copy the private source snapshot into the bundle.
    #[arg(long, requires = "retain")]
    pub include_source: bool,
    /// After a passing draft that states lifecycle.action closed, close the
    /// session, exactly as session close does.
    #[arg(long)]
    pub close: bool,
    /// Absolute path of a file holding the whole draft report (at most
    /// 128 KiB of UTF-8); without it the draft is read from standard input.
    #[arg(long)]
    pub file: Option<PathBuf>,
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

/// Skill namespace arguments (ADR 0038).
#[derive(Args, Debug)]
pub(crate) struct SkillArguments {
    #[command(subcommand)]
    pub command: SkillCommand,
}

/// Skill operations.
#[derive(Debug, Subcommand)]
pub(crate) enum SkillCommand {
    /// Copy this release's skill into the folder each named client reads
    /// (once per folder); a folder that holds something else is left as it is
    /// unless --replace is given.
    #[command(after_help = help_text::SKILL_INSTALL)]
    Install(SkillInstallArguments),
    /// Show what is at each skill folder, read only, and the skill's files.
    #[command(after_help = help_text::SKILL_STATUS)]
    Status(SkillStatusArguments),
    /// Print one text file of this release's skill, by its name in the skill
    /// (for example SKILL.md or references/commands.md).
    #[command(after_help = help_text::SKILL_SHOW)]
    Show(SkillShowArguments),
}

impl SkillCommand {
    /// Returns the public operation identifier.
    pub(crate) const fn operation_name(&self) -> CommandName {
        match self {
            Self::Install(_) => CommandName::SkillInstall,
            Self::Status(_) => CommandName::SkillStatus,
            Self::Show(_) => CommandName::SkillShow,
        }
    }
}

/// An AI coding client whose skill folder `VSift` knows.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum ClientArgument {
    /// Claude Code (.claude/skills/).
    #[value(name = "claude-code")]
    ClaudeCode,
    /// `OpenAI` Codex (.agents/skills/).
    #[value(name = "codex")]
    Codex,
    /// GitHub Copilot in VS Code and the Copilot CLI (.agents/skills/).
    #[value(name = "copilot")]
    Copilot,
    /// Cursor (.agents/skills/).
    #[value(name = "cursor")]
    Cursor,
    /// Gemini CLI (.agents/skills/).
    #[value(name = "gemini-cli")]
    GeminiCli,
    /// `OpenCode` (.agents/skills/).
    #[value(name = "opencode")]
    OpenCode,
    /// Goose (.agents/skills/).
    #[value(name = "goose")]
    Goose,
    /// Amp (.agents/skills/).
    #[value(name = "amp")]
    Amp,
}

impl From<ClientArgument> for AgentClient {
    fn from(value: ClientArgument) -> Self {
        match value {
            ClientArgument::ClaudeCode => Self::ClaudeCode,
            ClientArgument::Codex => Self::Codex,
            ClientArgument::Copilot => Self::Copilot,
            ClientArgument::Cursor => Self::Cursor,
            ClientArgument::GeminiCli => Self::GeminiCli,
            ClientArgument::OpenCode => Self::OpenCode,
            ClientArgument::Goose => Self::Goose,
            ClientArgument::Amp => Self::Amp,
        }
    }
}

/// Which clients, and whose folders.
#[derive(Args, Debug)]
pub(crate) struct SkillInstallArguments {
    /// A client to install the skill for; repeat it for several. Clients that
    /// read the same folder have it written once.
    #[arg(long = "client", required = true, value_enum, value_name = "CLIENT")]
    pub clients: Vec<ClientArgument>,
    /// Install for one project, below this existing folder, instead of for
    /// you (below your home folder).
    #[arg(long, value_name = "FOLDER")]
    pub project: Option<PathBuf>,
    /// Put this release's skill in place of a different vsift skill or a
    /// link at a folder; anything that is not a vsift skill is never replaced.
    #[arg(long)]
    pub replace: bool,
}

/// Whose folders.
#[derive(Args, Debug)]
pub(crate) struct SkillStatusArguments {
    /// Show one project's skill folders, below this existing folder, instead
    /// of yours.
    #[arg(long, value_name = "FOLDER")]
    pub project: Option<PathBuf>,
}

/// Which file.
#[derive(Args, Debug)]
pub(crate) struct SkillShowArguments {
    /// The file's name in the skill, as skill status lists it; never a path.
    pub entry: String,
}

/// Local bundle path.
#[derive(Args, Debug)]
pub(crate) struct BundleValidateArguments {
    /// Bundle directory to validate.
    pub directory: PathBuf,
}

/// A bounded literal search of retained bundles (ADR 0035).
#[derive(Args, Debug)]
pub(crate) struct BundleSearchArguments {
    /// Bundles, or folders of bundles, at most 64.
    #[arg(required = true, value_name = "PATH")]
    pub paths: Vec<PathBuf>,
    /// Literal query text, at most 256 bytes; it is never interpreted as code
    /// or a regular expression.
    #[arg(long)]
    pub query: String,
    /// Hits per page, 1 through 100; defaults to 20. A page whose text is very
    /// long may hold fewer, and --cursor goes on from there.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub limit: Option<u16>,
    /// Opaque continuation token returned by the previous page of the same search.
    #[arg(long, hide_short_help = true)]
    pub cursor: Option<String>,
    /// Which answer to give. `brief` keeps each transcript hit's ids, times and
    /// `display_text`; on-screen hits and the coverage are the same in both.
    #[arg(long = "view", value_enum, default_value_t, hide_short_help = true)]
    pub view: ViewArgument,
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
    #[command(after_help = help_text::JOB_RUN)]
    Run(JobRequestArguments),
    /// Execute a finite JSON Lines file of job requests in a worker
    /// workspace, a bounded number at a time, each line independently.
    #[command(after_help = help_text::JOB_BATCH)]
    Batch(JobBatchArguments),
    /// Report one job: state, resumability, progress, result or failure.
    #[command(after_help = help_text::JOB_STATUS)]
    Status(JobIdentityArguments),
    /// Continue one interrupted job from its checkpoints.
    #[command(after_help = help_text::JOB_RESUME)]
    Resume(JobIdentityArguments),
    /// Cancel one job, or ask the process running it to stop.
    #[command(after_help = help_text::JOB_CANCEL)]
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
    #[command(after_help = help_text::JOB_START)]
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
    #[arg(long, value_parser = clap::value_parser!(u64).range(0..=300_000), default_value_t = 0, hide_short_help = true)]
    pub drain_timeout_ms: u64,
    /// How long a step waits (with jittered retries) for admission capacity
    /// before it answers BUSY, 0 through 60000 milliseconds; defaults to
    /// 60000.
    #[arg(long, value_parser = clap::value_parser!(u64).range(0..=60_000), default_value_t = 60_000, hide_short_help = true)]
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
    #[arg(long, value_parser = clap::value_parser!(u64).range(0..=300_000), default_value_t = 0, hide_short_help = true)]
    pub drain_timeout_ms: u64,
    /// How long a step waits (with jittered retries) for admission capacity
    /// before it answers BUSY, 0 through 60000 milliseconds; defaults to
    /// 60000.
    #[arg(long, value_parser = clap::value_parser!(u64).range(0..=60_000), default_value_t = 60_000, hide_short_help = true)]
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
    use std::{
        collections::{BTreeMap, BTreeSet},
        ffi::OsString,
        path::Path,
    };

    use clap::{CommandFactory, Parser};
    use vsift_contract::CommandName;

    use super::{Cli, parser};
    use crate::config::{DEFAULT_PROBE_TIMEOUT_SECONDS, MAX_PROBE_TIMEOUT_SECONDS};
    use crate::help_text::{BARE_HELP, ROOT_HELP_TEMPLATE, TYPICAL_INVESTIGATION_HELP};
    use crate::watch::route_bare_video;

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
    fn help_command_lines(text: &str) -> Vec<Vec<String>> {
        const SUBSTITUTIONS: [(&str, &str); 10] = [
            ("<video>", "video.mp4"),
            ("<session>", "ses_0123456789abcdef"),
            ("<us>", "1000"),
            ("<words>", "error"),
            ("<file>", "walkthrough.srt"),
            ("<microseconds>", "500000"),
            ("<name>", "bug-report"),
            ("<job>", "job_0123456789abcdef"),
            ("<evidence>", "evd_0123456789abcdef"),
            ("<digest>", "sha256"),
        ];
        text.lines()
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

    /// Every command, the root's own subcommands first, with its command path.
    fn every_command(root: &clap::Command) -> Vec<(Vec<String>, &clap::Command)> {
        let mut found: Vec<(Vec<String>, &clap::Command)> = root
            .get_subcommands()
            .map(|command| (vec![command.get_name().to_owned()], command))
            .collect();
        let mut next = 0;
        while let Some((path, command)) = found.get(next).cloned() {
            for child in command.get_subcommands() {
                let mut child_path = path.clone();
                child_path.push(child.get_name().to_owned());
                found.push((child_path, child));
            }
            next += 1;
        }
        found
    }

    /// The anchor GitHub gives a Markdown heading, as the guide tool writes it.
    fn heading_anchor(line: &str) -> Option<String> {
        let heading = line.trim_start_matches('#');
        (heading.len() < line.len()).then(|| {
            heading
                .trim()
                .to_lowercase()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '-')
                .map(|c| if c == ' ' { '-' } else { c })
                .collect()
        })
    }

    /// Design note E3: each command's help gives examples that parse and name
    /// that command, and ends with a link to a guide page that exists, at a
    /// section that exists.
    #[test]
    fn every_command_gives_examples_that_parse_and_ends_with_its_guide_page()
    -> Result<(), Box<dyn std::error::Error>> {
        const GUIDE: &str = "Guide: https://github.com/smormah/vsift/blob/main/docs/";
        let root = Cli::command();
        let commands = every_command(&root);
        assert!(commands.len() >= 51, "{} commands", commands.len());
        for (path, command) in commands {
            let help = command
                .get_after_help()
                .ok_or_else(|| format!("{path:?} has no examples"))?
                .to_string();
            let examples = help_command_lines(&help);
            assert!(!examples.is_empty(), "{path:?} has no examples");
            for example in examples {
                let routed = route_bare_video(example.iter().map(OsString::from).collect());
                let words: Vec<String> = routed
                    .iter()
                    .filter_map(|word| word.to_str().map(str::to_owned))
                    .collect();
                assert!(
                    words.windows(path.len()).any(|window| window == path),
                    "{path:?}: {words:?}"
                );
                let parsed = Cli::try_parse_from(&routed);
                assert!(parsed.is_ok(), "{words:?}: {:?}", parsed.err());
            }
            let page = help
                .lines()
                .last()
                .and_then(|line| line.strip_prefix(GUIDE))
                .ok_or_else(|| format!("{path:?} does not end with its guide page"))?;
            let (file, anchor) = page.split_once('#').unwrap_or((page, ""));
            let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
            let text = std::fs::read_to_string(docs.join(file))?;
            assert!(
                anchor.is_empty() || text.lines().filter_map(heading_anchor).any(|a| a == anchor),
                "{path:?}: {page}"
            );
        }
        Ok(())
    }

    /// The groups of the root help are written by hand because clap has none:
    /// they list every command once, with the summary its own help opens with.
    #[test]
    fn the_root_groups_are_exactly_the_commands_with_their_own_summaries() {
        let listed: BTreeMap<String, String> = ROOT_HELP_TEMPLATE
            .lines()
            .skip_while(|line| !line.starts_with("Get started"))
            .take_while(|line| !line.starts_with("Options:"))
            .filter_map(|line| {
                let (name, summary) = line.strip_prefix("  ")?.split_once("  ")?;
                Some((name.to_owned(), summary.trim().to_owned()))
            })
            .collect();
        let commands: BTreeMap<String, String> = Cli::command()
            .get_subcommands()
            .map(|command| {
                let summary = command.get_about().map(ToString::to_string);
                (command.get_name().to_owned(), summary.unwrap_or_default())
            })
            .collect();
        assert_eq!(listed, commands);
    }

    /// Piped help is printed as written, so these texts are written to fit 80
    /// columns. A link cannot be wrapped.
    #[test]
    fn the_hand_written_help_fits_eighty_columns() {
        let root = Cli::command();
        let footers: Vec<String> = every_command(&root)
            .iter()
            .filter_map(|(_, command)| command.get_after_help().map(ToString::to_string))
            .collect();
        let texts = [BARE_HELP, ROOT_HELP_TEMPLATE, TYPICAL_INVESTIGATION_HELP];
        let footers = footers.iter().map(String::as_str);
        for text in texts.into_iter().chain(footers) {
            for line in text.lines().filter(|line| !line.contains("https://")) {
                assert!(line.chars().count() <= 80, "{line}");
            }
        }
    }

    /// Design note E4: a terminal's help wraps to its width; piped help is the
    /// text as written, with nothing added or broken.
    #[test]
    fn help_wraps_to_a_terminal_and_stays_as_written_when_piped() {
        let wrapped = parser(true).term_width(40).render_long_help().to_string();
        // clap wraps the option descriptions and the guidance. The command
        // groups are template text, which it prints as it is (they fit 80
        // columns), and a link cannot be broken.
        let (_, options) = wrapped.split_once("Options:").unwrap_or_default();
        let hand_written = |line: &&str| !line.contains("https://") && !line.starts_with("Run  ");
        for line in options.lines().filter(hand_written) {
            assert!(line.chars().count() <= 40, "{line}");
        }
        assert!(!wrapped.contains(TYPICAL_INVESTIGATION_HELP), "{wrapped}");

        let piped = parser(false).render_long_help().to_string();
        assert!(piped.contains(TYPICAL_INVESTIGATION_HELP), "{piped}");
    }

    /// What plain `vsift` offers is what the parser and the routing accept.
    #[test]
    fn the_two_commands_of_the_bare_help_parse() {
        let commands: Vec<&str> = BARE_HELP
            .lines()
            .filter_map(|line| line.strip_prefix("  ")?.split_once("  "))
            .map(|(command, _)| command)
            .collect();
        assert_eq!(commands, ["vsift setup", "vsift ./meeting.mp4"]);
        for command in commands {
            let line = command.split_whitespace().map(OsString::from).collect();
            let parsed = Cli::try_parse_from(route_bare_video(line));
            assert!(parsed.is_ok(), "{command}: {:?}", parsed.err());
        }
    }

    /// The worked example in the top-level help names only commands and options
    /// the parser accepts, with the arguments it shows.
    #[test]
    fn every_command_the_typical_investigation_names_parses() {
        let lines = help_command_lines(TYPICAL_INVESTIGATION_HELP);
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

    /// The "ready-made outcomes" paragraph lists exactly the workflows this
    /// release's skill ships, each by its file name without `.md`, so a
    /// workflow added, renamed or removed fails here until the help says so
    /// (ADR 0038, section 5). The `skill show` line itself is parsed by
    /// [`every_command_the_typical_investigation_names_parses`].
    #[test]
    fn the_ready_made_outcomes_are_the_shipped_workflows() {
        let shipped: BTreeSet<&str> = vsift_contract::SKILL_FILES
            .iter()
            .filter_map(|(name, _)| name.strip_prefix("workflows/")?.strip_suffix(".md"))
            .collect();
        let listed: BTreeSet<&str> = TYPICAL_INVESTIGATION_HELP
            .lines()
            .skip_while(|line| !line.starts_with("Ready-made outcomes."))
            .take_while(|line| !line.starts_with("Each one's method"))
            .filter_map(|line| {
                let line = line.strip_prefix("  ")?;
                let (name, description) = line.split_once("  ")?;
                (!description.trim().is_empty()).then_some(name)
            })
            .collect();
        assert_eq!(listed, shipped);
        assert!(shipped.len() >= 6, "the skill lost workflows: {shipped:?}");
        assert!(TYPICAL_INVESTIGATION_HELP.contains("vsift skill show workflows/<name>.md"));
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
            "never build the JSON by search and replace",
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
        assert!(TYPICAL_INVESTIGATION_HELP.contains("--max-frames 1 to 100 (default 12)"));
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
            "--max-frames 1 to {} (default {})",
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
