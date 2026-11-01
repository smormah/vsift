//! Fetching one video from a public web page through yt-dlp (ADR 0028
//! sections 4.6 to 4.8, #409).
//!
//! yt-dlp is a provider like `FFmpeg`: an explicit executable, an argument
//! array, no shell, an environment that is empty but for what is listed here,
//! and a process tree the supervisor stops. It is also a program that reads a
//! stranger's page and writes what the page tells it to, so nothing it says is
//! believed: its metadata is parsed strictly ([`crate::parse_ytdlp_info`]), its
//! standard error only chooses a typed reason
//! ([`crate::classify_ytdlp_failure`]), and the folder it wrote to must hold
//! exactly the files expected, regular, with an admitted container's magic.
//!
//! **The pair rule.** [`PageTools`] holds a yt-dlp and the runtime that goes
//! with it; the caller resolves them as a pair (a managed yt-dlp with the
//! managed Deno only, a user's with the user's runtime or none), and nothing
//! here looks one up. With no runtime yt-dlp is told `--no-js-runtimes` alone,
//! so it never finds one on its own.
//!
//! **Its connections are its own.** `VSift` checks the link before yt-dlp
//! starts, but cannot see or limit what yt-dlp connects to (known limit
//! L-209). The file sizes are counted on disk, so a false `Content-Length`
//! does not pass a cap.

use std::{
    ffi::OsString,
    io::Read,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::Duration,
};

use tokio::time::{Instant, sleep};
use vsift_application::StageCancellation;
use vsift_domain::{DownloadLimits, PageJoin, PageToolRecord, Sha256Hex, SourceFetchReason};

use crate::{
    HostIsolation, ProcessCancellation, ProcessOutcome, ProcessRequest, ProcessSupervisor,
    ProcessWorkingDirectory, SupervisorPolicy, TerminationReason, TrustedExecutable,
    link_acquisition::{MAGIC_BYTES, is_internal_name},
    source_snapshot::{SourceContainer, container_of},
    ytdlp_info::{MAX_YTDLP_INFO_BYTES, SelectedFormat, YtDlpVideo, parse_ytdlp_info},
    ytdlp_refusal::{MAX_YTDLP_STDERR_BYTES, classify_ytdlp_failure},
};

/// The longest phase 1 may take: 2 minutes.
const METADATA_DEADLINE: Duration = Duration::from_secs(120);
/// How often a running phase measures the folder and looks at cancellation.
const POLL: Duration = Duration::from_millis(200);
/// No growth for this long, once the first bytes are there, is a stall.
const STALL_DEADLINE: Duration = Duration::from_secs(30);
/// How often room is asked for again while a download grows: 64 MiB.
const ROOM_STEP: u64 = 64 * 1024 * 1024;
/// The fixed name of what yt-dlp writes.
const OUTPUT_STEM: &str = "vsift-page";
/// Phase 1's format selection: separate HTTPS video and audio to join, or one
/// HTTPS file, in the admitted containers. `--merge-output-format` is never
/// given, because `VSift` joins the streams itself.
const FORMAT_SELECTION: &str = "bv*[protocol=https][ext=mp4]+ba[protocol=https][ext=m4a]/b[protocol=https][ext=mp4]/bv*[protocol=https][ext=webm]+ba[protocol=https][ext=webm]/b[protocol=https][ext=webm]";
/// yt-dlp's own refusal of anything live; phase 1's checks are the authority.
/// `!=?` passes a site that reports no status, which is common.
const LIVE_FILTER: &str =
    "!is_live & !was_live & live_status!=?is_upcoming & live_status!=?post_live";
/// The arguments every run has (ADR 0028, note of 2026-11-01). No
/// `--max-downloads`: it stops `--dump-single-json` printing, and its exit
/// status 101 would hide a failed download. `--playlist-items 1` bounds a
/// playlist instead. `--no-progress` keeps a long download's standard output
/// within its bound, and `--fixup never` keeps yt-dlp from any `FFmpeg` it
/// might find beside itself.
const FIXED_ARGUMENTS: [&str; 28] = [
    "--ignore-config",
    "--no-plugin-dirs",
    "--no-update",
    "--no-remote-components",
    "--no-cookies",
    "--no-cookies-from-browser",
    "--no-playlist",
    "--no-batch-file",
    "--playlist-items",
    "1",
    "--abort-on-error",
    "--no-exec",
    "--restrict-filenames",
    "--no-mtime",
    "--no-part",
    "--no-continue",
    "--downloader",
    "native",
    "--no-check-formats",
    "--socket-timeout",
    "30",
    "--no-progress",
    "--fixup",
    "never",
    "--match-filters",
    LIVE_FILTER,
    "--output",
    "vsift-page.%(format_id)s.%(ext)s",
];
/// The proxy variables `VSift` itself honours, in both spellings. They pass
/// through the environment and never the command line, where other local
/// users could read them.
const PROXY_VARIABLES: [&str; 6] = [
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];

/// One tool of a page's fetch: its executable and how its origin records it.
#[derive(Clone, Debug)]
pub struct PageTool {
    /// The file to run.
    pub executable: TrustedExecutable,
    /// How the session's origin records it.
    pub record: PageToolRecord,
}

/// The proxy variables handed to yt-dlp, values included. They are secret
/// (a proxy URL can hold a password), so `Debug` prints only the names.
#[derive(Clone, Default)]
pub struct ProxyVariables(Vec<(&'static str, OsString)>);

impl ProxyVariables {
    /// The proxy variables of this process that are set.
    #[must_use]
    pub fn from_environment() -> Self {
        Self::read(|name| std::env::var_os(name))
    }

    /// The variables `variable` gives, so a test need not change the
    /// process's environment.
    pub fn read(variable: impl Fn(&str) -> Option<OsString>) -> Self {
        Self(
            PROXY_VARIABLES
                .into_iter()
                .filter_map(|name| {
                    variable(name)
                        .filter(|value| !value.is_empty())
                        .map(|value| (name, value))
                })
                .collect(),
        )
    }

    /// Whether a proxy, not only an exception, is named.
    fn names_a_proxy(&self) -> bool {
        self.0
            .iter()
            .any(|(name, _)| !name.eq_ignore_ascii_case("no_proxy"))
    }
}

impl std::fmt::Debug for ProxyVariables {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_list()
            .entries(self.0.iter().map(|(name, _)| name))
            .finish()
    }
}

/// What a page's fetch runs: a yt-dlp, the runtime that goes with it, the
/// `FFmpeg` that joins streams, and how the host runs providers.
#[derive(Clone, Debug)]
pub struct PageTools {
    /// The yt-dlp.
    pub yt_dlp: PageTool,
    /// Its runtime, resolved with it as a pair; `None` runs it with none.
    pub js_runtime: Option<PageTool>,
    /// The verified `FFmpeg`, if any; only a pair of streams needs it.
    pub ffmpeg: Option<TrustedExecutable>,
    /// How this host isolates providers.
    pub host_isolation: HostIsolation,
    /// The proxy variables to hand on.
    pub proxy: ProxyVariables,
    timing: PageTiming,
}

/// How long a fetch waits; only development builds may shorten it, so a stall
/// test does not wait 30 seconds.
#[derive(Clone, Copy, Debug)]
struct PageTiming {
    first_bytes: Duration,
    stall: Duration,
}

impl PageTools {
    /// The tools of one fetch.
    #[must_use]
    pub const fn new(
        yt_dlp: PageTool,
        js_runtime: Option<PageTool>,
        ffmpeg: Option<TrustedExecutable>,
        host_isolation: HostIsolation,
        proxy: ProxyVariables,
    ) -> Self {
        Self {
            yt_dlp,
            js_runtime,
            ffmpeg,
            host_isolation,
            proxy,
            timing: PageTiming {
                first_bytes: METADATA_DEADLINE,
                stall: STALL_DEADLINE,
            },
        }
    }

    /// The same tools with shorter waits for the first bytes and a stall
    /// (`install-test-hooks`, development builds only).
    #[cfg(any(test, feature = "install-test-hooks"))]
    #[must_use]
    pub const fn with_waits_for_tests(mut self, first_bytes: Duration, stall: Duration) -> Self {
        self.timing = PageTiming { first_bytes, stall };
        self
    }
}

/// Why a page's fetch stopped.
#[derive(Debug)]
pub(crate) enum PageFailure {
    /// The online step failed for this reason.
    Refused(SourceFetchReason),
    /// The caller cancelled.
    Cancelled,
    /// The session's filesystem has no room.
    NoRoom,
}

impl From<SourceFetchReason> for PageFailure {
    fn from(reason: SourceFetchReason) -> Self {
        Self::Refused(reason)
    }
}

/// What a finished fetch left in the session's source slot.
#[derive(Debug)]
pub(crate) struct AcquiredPage {
    pub(crate) video: YtDlpVideo,
    pub(crate) join: PageJoin,
    pub(crate) through_proxy: bool,
}

/// Where one fetch works: everything is private and removed with the session.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PageFolders<'a> {
    /// The folder yt-dlp runs in: its temporary folder and its `HOME`.
    pub(crate) home: &'a Path,
    /// The folder it writes to, and nothing else does.
    pub(crate) output: &'a Path,
    /// The session's source slot, not yet created.
    pub(crate) slot: &'a Path,
}

/// Asks whether the session's filesystem has room for this many more bytes.
pub(crate) trait PageRoom: Send + Sync {
    /// `false` when the free space was read and is short.
    fn has_room_for(&self, bytes: u64) -> bool;
}

/// A link's host may reach yt-dlp only if it is a name, and not one for an
/// internal host: yt-dlp connects with no address policy, so nothing can
/// check an address later (ADR 0028 section 4.7).
///
/// # Errors
///
/// [`SourceFetchReason::AddressRefused`] for an address or an internal name.
pub(crate) fn check_page_host(url: &reqwest::Url) -> Result<(), SourceFetchReason> {
    match url.domain() {
        Some(host) if !is_internal_name(host) => Ok(()),
        _ => Err(SourceFetchReason::AddressRefused),
    }
}

/// Fetches the page at `link` into `folders.slot`, within `limits`.
///
/// # Errors
///
/// A [`PageFailure`]. The folders may hold anything on an error; their owner
/// removes them.
pub(crate) async fn acquire_page(
    link: &reqwest::Url,
    limits: DownloadLimits,
    tools: &PageTools,
    folders: PageFolders<'_>,
    (room, cancellation): (&dyn PageRoom, &dyn StageCancellation),
) -> Result<AcquiredPage, PageFailure> {
    let deadline = Instant::now() + Duration::from_secs(u64::from(limits.max_seconds()));
    let run = Run {
        link,
        limits,
        tools,
        folders,
        deadline,
        cancellation,
    };
    let video = run.read_metadata().await?;
    let join = video.join();
    if join == PageJoin::AudioVideo && tools.ffmpeg.is_none() {
        return Err(SourceFetchReason::MediaToolsMissing.into());
    }
    run.download(&video, room).await?;
    let files = expected_files(folders.output, &video.formats, limits)?;
    match files.as_slice() {
        [file] => std::fs::rename(&file.path, folders.slot)
            .map_err(|_| PageFailure::Refused(SourceFetchReason::OnlineStepFailed))?,
        [video_file, audio_file] => run.join(video_file, audio_file, room).await?,
        _ => return Err(SourceFetchReason::UnexpectedOutput.into()),
    }
    Ok(AcquiredPage {
        video,
        join,
        through_proxy: tools.proxy.names_a_proxy(),
    })
}

/// One fetch's fixed inputs.
struct Run<'a> {
    link: &'a reqwest::Url,
    limits: DownloadLimits,
    tools: &'a PageTools,
    folders: PageFolders<'a>,
    deadline: Instant,
    cancellation: &'a dyn StageCancellation,
}

/// What stopped a supervised run early, decided by the watch beside it.
#[derive(Clone, Copy, Debug)]
enum Stop {
    Refused(SourceFetchReason),
    Cancelled,
    NoRoom,
}

impl From<Stop> for PageFailure {
    fn from(stop: Stop) -> Self {
        match stop {
            Stop::Refused(reason) => Self::Refused(reason),
            Stop::Cancelled => Self::Cancelled,
            Stop::NoRoom => Self::NoRoom,
        }
    }
}

impl Run<'_> {
    fn time_left(&self) -> Result<Duration, SourceFetchReason> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            Err(SourceFetchReason::Deadline)
        } else {
            Ok(left)
        }
    }

    /// The request for one yt-dlp run: the fixed arguments, the runtime after
    /// `--no-js-runtimes`, the output folder, then the link after `--`.
    fn ytdlp_request(
        &self,
        selection: &str,
        phase_arguments: &[&str],
        time: Duration,
    ) -> Result<ProcessRequest, SourceFetchReason> {
        let failed = |_| SourceFetchReason::OnlineStepFailed;
        let working = ProcessWorkingDirectory::new(self.folders.home).map_err(failed)?;
        let mut request = ProcessRequest::new(self.tools.yt_dlp.executable.clone(), working, time)
            .map_err(failed)?
            .with_arguments(FIXED_ARGUMENTS)
            .with_argument("--no-js-runtimes");
        if let Some(runtime) = &self.tools.js_runtime {
            let mut path = OsString::from("deno:");
            path.push(runtime.executable.path());
            request = request.with_argument("--js-runtimes").with_argument(path);
        }
        request = request
            .with_argument("--paths")
            .with_argument(self.folders.output)
            .with_argument("--format")
            .with_argument(selection)
            .with_arguments(phase_arguments.iter().copied())
            .with_argument("--")
            .with_argument(self.link.as_str());
        for name in ["TMPDIR", "TEMP", "TMP", "HOME"] {
            request = request
                .with_environment(name, self.folders.home)
                .map_err(failed)?;
        }
        request = request
            .with_environment("YTDLP_NO_PLUGINS", "1")
            .map_err(failed)?;
        // yt-dlp passes its environment on to the runtime, whose own update
        // check would be a connection nobody asked for.
        if self.tools.js_runtime.is_some() {
            request = request
                .with_environment("DENO_NO_UPDATE_CHECK", "1")
                .and_then(|request| request.with_environment("NO_COLOR", "1"))
                .map_err(failed)?;
        }
        // A one-file Python build on Windows cannot start without the system
        // folder's variable (Python's random-number start-up needs it); it
        // is a folder name, not personal data.
        #[cfg(windows)]
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            request = request
                .with_environment("SystemRoot", system_root)
                .map_err(failed)?;
        }
        for (name, value) in &self.tools.proxy.0 {
            request = request
                .with_environment(*name, value.clone())
                .map_err(failed)?;
        }
        Ok(request)
    }

    fn supervisor(&self) -> ProcessSupervisor {
        let stdout = NonZeroUsize::new(MAX_YTDLP_INFO_BYTES).unwrap_or(NonZeroUsize::MIN);
        let stderr = NonZeroUsize::new(MAX_YTDLP_STDERR_BYTES).unwrap_or(NonZeroUsize::MIN);
        ProcessSupervisor::new(
            SupervisorPolicy::default().with_stream_limits(stdout, stderr),
            self.tools.host_isolation,
        )
    }

    /// Runs `request`, calling `watch` every [`POLL`]; when it answers with a
    /// [`Stop`] the process tree is stopped and that is the answer.
    async fn supervised(
        &self,
        request: ProcessRequest,
        mut watch: impl FnMut() -> Option<Stop> + Send,
    ) -> Result<ProcessOutcome, PageFailure> {
        let cancellation = ProcessCancellation::new();
        let supervisor = self.supervisor();
        let run = supervisor.run(request, cancellation.clone());
        let mut run = std::pin::pin!(run);
        let mut stop = None;
        loop {
            tokio::select! {
                finished = run.as_mut() => {
                    if let Some(stop) = stop {
                        return Err(PageFailure::from(stop));
                    }
                    return finished.map_err(|_| {
                        PageFailure::Refused(SourceFetchReason::OnlineStepFailed)
                    });
                }
                () = sleep(POLL), if stop.is_none() => {
                    stop = if self.cancellation.is_cancelled() {
                        Some(Stop::Cancelled)
                    } else {
                        watch()
                    };
                    if stop.is_some() {
                        cancellation.cancel();
                        cancellation.escalate();
                    }
                }
            }
        }
    }

    /// The run's outcome as far as a phase can read it: a run that ended
    /// well, or the typed reason. yt-dlp's text chooses only the reason.
    /// Only exit 0 is a success: with `--no-part` a failed download can leave
    /// a good-looking file, so the folder's check cannot stand in for this.
    fn judge(outcome: &ProcessOutcome) -> Result<(), SourceFetchReason> {
        match outcome.termination {
            TerminationReason::Exited if outcome.status.success() => Ok(()),
            TerminationReason::Deadline => Err(SourceFetchReason::Deadline),
            TerminationReason::OutputLimit(_) => Err(SourceFetchReason::UnexpectedOutput),
            TerminationReason::Exited | TerminationReason::Cancelled => {
                Err(classify_ytdlp_failure(&outcome.stderr.bytes))
            }
        }
    }

    /// Phase 1: what the page holds. Nothing is written.
    async fn read_metadata(&self) -> Result<YtDlpVideo, PageFailure> {
        let time = self.time_left()?.min(METADATA_DEADLINE);
        let request = self.ytdlp_request(
            FORMAT_SELECTION,
            &["--simulate", "--dump-single-json"],
            time,
        )?;
        let outcome = self.supervised(request, || None).await?;
        Self::judge(&outcome)?;
        Ok(parse_ytdlp_info(&outcome.stdout.bytes, self.limits)?)
    }

    /// Phase 2: the download, under the caps: bytes counted on disk every
    /// 200 ms, the deadline, and a stall.
    ///
    /// yt-dlp reads the page again before its first byte, which takes as long
    /// as phase 1 did, so the wait for the first byte is phase 1's bound and
    /// the 30-second stall counts from there.
    async fn download(&self, video: &YtDlpVideo, room: &dyn PageRoom) -> Result<(), PageFailure> {
        let declared = video
            .formats
            .iter()
            .filter_map(|format| format.declared_bytes)
            .fold(0_u64, u64::saturating_add);
        if !room.has_room_for(declared.max(ROOM_STEP).min(self.limits.max_bytes())) {
            return Err(PageFailure::NoRoom);
        }
        let selection = exact_selection(&video.formats);
        let request = self.ytdlp_request(&selection, &[], self.time_left()?)?;
        let mut watch = DownloadWatch::new(self.limits, self.tools.timing, declared);
        let output = self.folders.output;
        let outcome = self
            .supervised(request, || watch.check(bytes_in(output), room))
            .await?;
        Ok(Self::judge(&outcome)?)
    }

    /// Joins the two checked streams into the slot with a stream copy: no
    /// filter, no re-encode, no other input or protocol. `FFmpeg` alone
    /// writes the slot, under the same caps.
    async fn join(
        &self,
        video: &ExpectedFile,
        audio: &ExpectedFile,
        room: &dyn PageRoom,
    ) -> Result<(), PageFailure> {
        let failed = |_| PageFailure::Refused(SourceFetchReason::OnlineStepFailed);
        let ffmpeg = self
            .tools
            .ffmpeg
            .clone()
            .ok_or(PageFailure::Refused(SourceFetchReason::MediaToolsMissing))?;
        if !room.has_room_for(video.bytes.saturating_add(audio.bytes)) {
            return Err(PageFailure::NoRoom);
        }
        let working = ProcessWorkingDirectory::new(self.folders.home).map_err(failed)?;
        let request = ProcessRequest::new(ffmpeg, working, self.time_left()?)
            .map_err(failed)?
            .with_arguments(join_arguments(video, audio, self.folders.slot));
        let slot = self.folders.slot;
        let limit = self.limits.max_bytes();
        let outcome = self
            .supervised(request, || {
                let written = std::fs::metadata(slot).map_or(0, |metadata| metadata.len());
                (written > limit).then_some(Stop::Refused(SourceFetchReason::Size))
            })
            .await?;
        match outcome.termination {
            TerminationReason::Exited if outcome.status.success() => Ok(()),
            TerminationReason::Deadline => Err(SourceFetchReason::Deadline.into()),
            _ => Err(SourceFetchReason::UnexpectedOutput.into()),
        }
    }
}

/// `FFmpeg`'s arguments for the join. Each input's options precede its own
/// `-i`, because an input option applies to the next input only; a MOV input
/// follows no external reference, as every other read does.
fn join_arguments(video: &ExpectedFile, audio: &ExpectedFile, slot: &Path) -> Vec<OsString> {
    let mut arguments: Vec<OsString> = ["-nostdin", "-hide_banner", "-loglevel", "error"]
        .into_iter()
        .map(OsString::from)
        .collect();
    for input in [video, audio] {
        arguments.extend(
            [
                "-protocol_whitelist",
                "file",
                "-f",
                input.container.demuxer(),
            ]
            .into_iter()
            .map(OsString::from),
        );
        if input.container == SourceContainer::IsoMedia {
            arguments.extend(
                ["-enable_drefs", "0", "-use_absolute_path", "0"]
                    .into_iter()
                    .map(OsString::from),
            );
        }
        arguments.push(OsString::from("-i"));
        arguments.push(input.path.clone().into_os_string());
    }
    let muxer = match video.container {
        SourceContainer::IsoMedia => "mp4",
        SourceContainer::Matroska => "matroska",
    };
    arguments.extend(
        [
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c",
            "copy",
            "-map_metadata",
            "-1",
            "-f",
            muxer,
        ]
        .into_iter()
        .map(OsString::from),
    );
    arguments.push(slot.as_os_str().to_owned());
    arguments
}

/// Follows a download's bytes on disk.
struct DownloadWatch {
    limit: u64,
    timing: PageTiming,
    started: Instant,
    last_growth: Instant,
    seen: u64,
    room_through: u64,
}

impl DownloadWatch {
    fn new(limits: DownloadLimits, timing: PageTiming, declared: u64) -> Self {
        let now = Instant::now();
        Self {
            limit: limits.max_bytes(),
            timing,
            started: now,
            last_growth: now,
            seen: 0,
            room_through: declared.max(ROOM_STEP),
        }
    }

    /// What stops the download now that `bytes` are on disk, if anything.
    fn check(&mut self, bytes: u64, room: &dyn PageRoom) -> Option<Stop> {
        if bytes > self.limit {
            return Some(Stop::Refused(SourceFetchReason::Size));
        }
        if bytes > self.seen {
            self.seen = bytes;
            self.last_growth = Instant::now();
        }
        if self.seen >= self.room_through {
            if !room.has_room_for(ROOM_STEP) {
                return Some(Stop::NoRoom);
            }
            self.room_through = self.seen.saturating_add(ROOM_STEP);
        }
        let waited = if self.seen == 0 {
            self.started.elapsed() > self.timing.first_bytes
        } else {
            self.last_growth.elapsed() > self.timing.stall
        };
        waited.then_some(Stop::Refused(SourceFetchReason::Stalled))
    }
}

/// The bytes of the entries directly in `folder`, without following a link.
/// A folder that cannot be read counts as empty: the check afterwards judges
/// what is really there.
fn bytes_in(folder: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .fold(0, u64::saturating_add)
}

/// One checked file of a finished download.
#[derive(Clone, Debug)]
struct ExpectedFile {
    path: PathBuf,
    bytes: u64,
    container: SourceContainer,
}

/// Phase 2's selection: exactly the ids phase 1 checked, each still held to
/// HTTPS on the page as yt-dlp reads it again. The comma makes a pair two
/// separate downloads, so yt-dlp never considers merging them, which with
/// `--abort-on-error` and no `FFmpeg` would stop it.
fn exact_selection(formats: &[SelectedFormat]) -> String {
    formats
        .iter()
        .map(|format| format!("{}[protocol=https]", format.format_id))
        .collect::<Vec<_>>()
        .join(",")
}

/// The name yt-dlp gives each selected format.
fn expected_names(formats: &[SelectedFormat]) -> Vec<String> {
    formats
        .iter()
        .map(|format| format!("{OUTPUT_STEM}.{}.{}", format.format_id, format.ext))
        .collect()
}

/// Checks that `output` holds exactly the files expected, in the formats'
/// order: regular files and not links, with the fixed names, an admitted
/// container's magic and, together, no more than the limit.
///
/// # Errors
///
/// [`SourceFetchReason::UnexpectedOutput`] for anything else in the folder or
/// anything missing, [`SourceFetchReason::NotAVideo`] for a wrong container,
/// [`SourceFetchReason::Size`] over the limit.
fn expected_files(
    output: &Path,
    formats: &[SelectedFormat],
    limits: DownloadLimits,
) -> Result<Vec<ExpectedFile>, SourceFetchReason> {
    let unexpected = SourceFetchReason::UnexpectedOutput;
    let names = expected_names(formats);
    let found = std::fs::read_dir(output)
        .map_err(|_| unexpected)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| unexpected)?;
    if found.len() != names.len()
        || !found.iter().all(|name| {
            name.to_str()
                .is_some_and(|name| names.iter().any(|n| n == name))
        })
    {
        return Err(unexpected);
    }
    let mut files = Vec::with_capacity(names.len());
    for name in names {
        let path = output.join(name);
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| unexpected)?;
        if !metadata.file_type().is_file() {
            return Err(unexpected);
        }
        let container = read_container(&path)?;
        files.push(ExpectedFile {
            path,
            bytes: metadata.len(),
            container,
        });
    }
    if files
        .iter()
        .any(|file| file.container != files[0].container)
    {
        return Err(unexpected);
    }
    let total = files
        .iter()
        .map(|file| file.bytes)
        .fold(0_u64, u64::saturating_add);
    if total > limits.max_bytes() {
        return Err(SourceFetchReason::Size);
    }
    Ok(files)
}

/// The admitted container a file's first bytes announce.
fn read_container(path: &Path) -> Result<SourceContainer, SourceFetchReason> {
    let mut file = std::fs::File::open(path).map_err(|_| SourceFetchReason::UnexpectedOutput)?;
    let mut header = [0_u8; MAGIC_BYTES];
    let mut filled = 0;
    while filled < MAGIC_BYTES {
        match file.read(&mut header[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(_) => return Err(SourceFetchReason::UnexpectedOutput),
        }
    }
    container_of(&header[..filled]).ok_or(SourceFetchReason::NotAVideo)
}

/// The SHA-256 of `text`, as the record's digests are written.
pub(crate) fn sha256_of(text: &str) -> Option<Sha256Hex> {
    use sha2::{Digest, Sha256};

    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut digest = String::with_capacity(64);
    for byte in Sha256::digest(text.as_bytes()) {
        digest.push(char::from(DIGITS[usize::from(byte >> 4)]));
        digest.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    Sha256Hex::parse(digest).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Room(bool);

    impl PageRoom for Room {
        fn has_room_for(&self, _bytes: u64) -> bool {
            self.0
        }
    }

    fn timing() -> PageTiming {
        PageTiming {
            first_bytes: Duration::from_secs(60),
            stall: Duration::from_secs(60),
        }
    }

    fn limits(bytes: u64) -> Result<DownloadLimits, vsift_domain::DownloadLimitsError> {
        DownloadLimits::new(Some(bytes), None)
    }

    #[test]
    fn only_a_name_that_is_not_internal_reaches_ytdlp() -> Result<(), reqwest::Error> {
        let url = |text: &str| reqwest::Url::parse(text);
        let parse = |text| url(text).map_err(|_| SourceFetchReason::NotHttps);
        assert_eq!(
            parse("https://video.example.test/page").and_then(|url| check_page_host(&url)),
            Ok(())
        );
        for refused in [
            "https://8.8.8.8/page",
            "https://[2001:4860:4860::8888]/page",
            "https://localhost/page",
            "https://intranet/page",
            "https://video.corp/page",
            "https://video.internal/page",
        ] {
            assert_eq!(
                parse(refused).and_then(|url| check_page_host(&url)),
                Err(SourceFetchReason::AddressRefused),
                "{refused}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_watch_counts_bytes_on_disk_and_asks_for_room_as_they_grow()
    -> Result<(), vsift_domain::DownloadLimitsError> {
        let mut watch = DownloadWatch::new(limits(1_000)?, timing(), 0);
        assert!(watch.check(1_000, &Room(true)).is_none());
        assert!(matches!(
            watch.check(1_001, &Room(true)),
            Some(Stop::Refused(SourceFetchReason::Size))
        ));
        let mut watch = DownloadWatch::new(DownloadLimits::DEFAULT, timing(), 0);
        assert!(watch.check(ROOM_STEP - 1, &Room(false)).is_none());
        assert!(matches!(
            watch.check(ROOM_STEP, &Room(false)),
            Some(Stop::NoRoom)
        ));
        Ok(())
    }

    #[test]
    fn a_watch_gives_the_first_byte_a_longer_wait_than_a_stall() {
        let mut watch = DownloadWatch::new(
            DownloadLimits::DEFAULT,
            PageTiming {
                first_bytes: Duration::from_secs(600),
                stall: Duration::from_millis(100),
            },
            0,
        );
        assert!(watch.check(0, &Room(true)).is_none());
        assert!(watch.check(1, &Room(true)).is_none());
        std::thread::sleep(Duration::from_millis(120));
        assert!(matches!(
            watch.check(1, &Room(true)),
            Some(Stop::Refused(SourceFetchReason::Stalled))
        ));
    }

    #[test]
    fn proxy_values_never_print_and_an_exception_alone_is_not_a_proxy() {
        let variables = ProxyVariables::read(|name| {
            (name == "HTTPS_PROXY").then(|| OsString::from("http://user:SENTINEL@proxy.test:1"))
        });
        assert!(variables.names_a_proxy());
        assert!(!format!("{variables:?}").contains("SENTINEL"));
        let exception = ProxyVariables::read(|name| {
            (name == "NO_PROXY").then(|| OsString::from("example.test"))
        });
        assert!(!exception.names_a_proxy());
    }

    #[test]
    fn a_pair_is_joined_by_a_stream_copy_with_each_input_restricted() {
        let input = |path: &str| ExpectedFile {
            path: PathBuf::from(path),
            bytes: 1,
            container: SourceContainer::IsoMedia,
        };
        let arguments: Vec<String> = join_arguments(
            &input("/w/v.mp4"),
            &input("/w/a.m4a"),
            Path::new("/s/source.media"),
        )
        .into_iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
        assert_eq!(
            arguments.join(" "),
            "-nostdin -hide_banner -loglevel error \
             -protocol_whitelist file -f mov -enable_drefs 0 -use_absolute_path 0 -i /w/v.mp4 \
             -protocol_whitelist file -f mov -enable_drefs 0 -use_absolute_path 0 -i /w/a.m4a \
             -map 0:v:0 -map 1:a:0 -c copy -map_metadata -1 -f mp4 /s/source.media"
        );
    }

    #[test]
    fn names_are_fixed() {
        let format = |id: &str, ext: &str| SelectedFormat {
            format_id: id.to_owned(),
            ext: ext.to_owned(),
            declared_bytes: None,
        };
        assert_eq!(
            expected_names(&[format("18", "mp4")]),
            ["vsift-page.18.mp4"]
        );
        let pair = [format("137", "webm"), format("140", "webm")];
        assert_eq!(
            expected_names(&pair),
            ["vsift-page.137.webm", "vsift-page.140.webm"]
        );
        assert_eq!(
            exact_selection(&pair),
            "137[protocol=https],140[protocol=https]"
        );
        assert_eq!(
            exact_selection(&[format("18", "mp4")]),
            "18[protocol=https]"
        );
    }
}
