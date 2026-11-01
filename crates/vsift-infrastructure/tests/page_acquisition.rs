//! ADR 0028 sections 4.6 to 4.9 (#409): fetching a public page through yt-dlp,
//! with a stand-in for yt-dlp and `FFmpeg` (`testbin/yt_dlp_stand_in.rs`). No
//! test reaches the network: the stand-in opens no socket, and the routing
//! test's one connection is to a local HTTPS server through the
//! development-only link test route (`install-test-hooks`).
//!
//! Each test stages the stand-in under the name it needs, tells it what to do
//! through a file beside it, and reads what it was given from another, which
//! goes when the test's folder is removed.

use std::{
    env,
    error::Error,
    ffi::OsString,
    fs,
    io::{BufRead, BufReader, Write},
    net::{IpAddr, Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use native_tls::{Identity, TlsAcceptor};
use vsift_application::{
    ForegroundSessionPort, InitializeSessionStorage, InitializeSessionStorageRequest,
    NeverCancelled, OpenSessionError, StageCancellation, StagedSessionSource,
};
use vsift_domain::{
    DownloadLimits, DurabilityRequirement, OperationId, PageJoin, PageOrigin, PageToolRecord,
    PageToolVersion, PageVideoId, SessionId, Sha256Hex, SourceFetchReason, SourceLink,
    SourceOrigin,
};
use vsift_infrastructure::{
    FilesystemSessionStore, HostIsolation, LinkTestRoute, PageSourceStore, PageTool,
    PageToolSource, PageTools, ProxyVariables, SITE_PHRASES, SourceError, SourceSnapshot,
    TestAnswers, TrustedExecutable, YTDLP_PHRASES,
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const STAND_IN: &str = env!("CARGO_BIN_EXE_vsift-yt-dlp-stand-in");
const PREFIX: &str = "vsift-page-acquisition-test-";
const SESSION: &str = "ses_0123456789abcdef";
/// The token every link carries; only the stand-in's arguments may hold it.
const SENTINEL: &str = "SENTINEL-TOKEN-5e2b71";
/// A credential in the proxy variable; only the environment may hold it.
const PROXY_SENTINEL: &str = "SENTINEL-PROXY-90ac13";
const MP4: &str = "0000001866747970";
const MKV: &str = "1a45dfa3";
const NOT_MEDIA: &str = "3c68746d6c3e";
/// Phase 1's format selection, in full.
const PHASE_ONE_SELECTION: &str = "bv*[protocol=https][ext=mp4]+ba[protocol=https][ext=m4a]/b[protocol=https][ext=mp4]/bv*[protocol=https][ext=webm]+ba[protocol=https][ext=webm]/b[protocol=https][ext=webm]";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> TestResult<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path =
            env::temp_dir().join(format!("{PREFIX}{}-{stamp}-{sequence}", std::process::id()));
        fs::create_dir(&path)?;
        fs::create_dir(path.join("tools"))?;
        Ok(Self(path))
    }

    fn tools(&self) -> PathBuf {
        self.0.join("tools")
    }

    /// The stand-in, staged under `name`.
    fn stage(&self, name: &str) -> TestResult<TrustedExecutable> {
        let path = self
            .tools()
            .join(format!("{name}{}", env::consts::EXE_SUFFIX));
        fs::copy(STAND_IN, &path)?;
        Ok(TrustedExecutable::explicit(path)?)
    }

    /// Tells the stand-in what to do, with `info` as phase 1's output.
    fn scenario(&self, info: &str, lines: &[String]) -> TestResult {
        fs::write(self.tools().join("info.json"), info)?;
        let mut text = String::from("info info.json\n");
        for line in lines {
            text.push_str(line);
            text.push('\n');
        }
        fs::write(self.tools().join("scenario.txt"), text)?;
        Ok(())
    }

    /// Each call the stand-in took: its arguments and its environment.
    fn calls(&self) -> TestResult<Vec<Call>> {
        let path = self.tools().join("calls.log");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let mut calls = Vec::new();
        for line in fs::read_to_string(path)?.lines() {
            if line == "call" {
                calls.push(Call::default());
            } else if let (Some(call), Some(argument)) =
                (calls.last_mut(), line.strip_prefix("arg "))
            {
                call.arguments.push(argument.to_owned());
            } else if let (Some(call), Some(variable)) =
                (calls.last_mut(), line.strip_prefix("env "))
                && let Some((name, value)) = variable.split_once('=')
            {
                call.environment.push((name.to_owned(), value.to_owned()));
            }
        }
        Ok(calls)
    }

    async fn store(&self) -> TestResult<FilesystemSessionStore> {
        let workspace = self.0.join("workspace");
        let store = FilesystemSessionStore::provision_default(&workspace)?;
        let session_id = SessionId::parse(SESSION)?;
        let _registration = store.register_session(
            &session_id,
            &OperationId::parse("op_0123456789abcdef")?,
            1_000,
        )?;
        InitializeSessionStorage::new(store)
            .execute(InitializeSessionStorageRequest::new(
                session_id,
                OperationId::parse("op_0123456789abcdef")?,
                DurabilityRequirement::Ephemeral,
            ))
            .await?;
        Ok(FilesystemSessionStore::open_existing(&workspace)?)
    }

    /// Every file left in the session's work and artifact folders.
    fn session_files(&self) -> TestResult<Vec<PathBuf>> {
        let session = self.0.join("workspace").join("sessions").join(SESSION);
        let mut files = Vec::new();
        for folder in ["work", "artifacts"] {
            collect_files(&session.join(folder), &mut files)?;
        }
        Ok(files)
    }

    /// Every byte of every file in the session folder, as text.
    fn session_bytes(&self) -> TestResult<String> {
        let mut files = Vec::new();
        collect_files(
            &self.0.join("workspace").join("sessions").join(SESSION),
            &mut files,
        )?;
        let mut text = String::new();
        for file in files {
            text.push_str(&String::from_utf8_lossy(&fs::read(file)?));
        }
        Ok(text)
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn collect_files(folder: &Path, files: &mut Vec<PathBuf>) -> TestResult {
    if !folder.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            collect_files(&entry.path(), files)?;
        } else {
            files.push(entry.path());
        }
    }
    Ok(())
}

#[derive(Debug, Default)]
struct Call {
    arguments: Vec<String>,
    environment: Vec<(String, String)>,
}

impl Call {
    fn variable(&self, name: &str) -> Option<&str> {
        self.environment
            .iter()
            .find(|(found, _)| found == name)
            .map(|(_, value)| value.as_str())
    }

    fn after(&self, flag: &str) -> Option<&str> {
        self.arguments
            .iter()
            .position(|argument| argument == flag)
            .and_then(|index| self.arguments.get(index + 1))
            .map(String::as_str)
    }
}

fn single_info() -> String {
    r#"{"_type":"video","id":"abc-1","extractor_key":"Generic","title":"TITLE-SENTINEL","uploader":"UPLOADER-SENTINEL",
       "availability":"public","live_status":"not_live","duration":61.0,"format_id":"18","protocol":"https","ext":"mp4",
       "url":"https://cdn.example.test/signed?sig=URL-SENTINEL"}"#
        .to_owned()
}

fn pair_info(video_ext: &str, audio_ext: &str) -> String {
    format!(
        r#"{{"_type":"video","id":"abc-2","extractor_key":"Generic","availability":"unlisted","duration":61.0,
             "requested_formats":[
               {{"format_id":"137","protocol":"https","ext":"{video_ext}"}},
               {{"format_id":"140","protocol":"https","ext":"{audio_ext}"}}]}}"#
    )
}

fn link() -> TestResult<SourceLink> {
    Ok(SourceLink::parse(&format!(
        "https://video.example.test/watch?v=abc&token={SENTINEL}"
    ))?)
}

fn digest(character: char) -> TestResult<Sha256Hex> {
    Sha256Hex::parse(character.to_string().repeat(64)).map_err(|_| "digest".into())
}

fn managed_tool(executable: TrustedExecutable, version: &str) -> TestResult<PageTool> {
    Ok(PageTool {
        executable,
        record: PageToolRecord::Managed {
            catalogue_version: PageToolVersion::parse(version)?,
        },
    })
}

fn user_tool(executable: TrustedExecutable, character: char) -> TestResult<PageTool> {
    Ok(PageTool {
        executable,
        record: PageToolRecord::UserManaged {
            version: PageToolVersion::parse("2026.08.19")?,
            sha256: digest(character)?,
        },
    })
}

/// A managed yt-dlp with a stand-in runtime and `FFmpeg`.
fn managed_tools(root: &OwnedRoot, proxy: ProxyVariables) -> TestResult<PageTools> {
    Ok(PageTools::new(
        managed_tool(root.stage("yt-dlp")?, "yt-dlp-2026.08.19-linux-x86_64")?,
        Some(managed_tool(
            root.stage("deno")?,
            "deno-2.9.7-x86_64-linux-gnu",
        )?),
        Some(root.stage("ffmpeg")?),
        HostIsolation::ProcessOnly,
        proxy,
    ))
}

/// A user's yt-dlp with no runtime.
fn user_tools(root: &OwnedRoot) -> TestResult<PageTools> {
    Ok(PageTools::new(
        user_tool(root.stage("yt-dlp")?, 'c')?,
        None,
        Some(root.stage("ffmpeg")?),
        HostIsolation::ProcessOnly,
        ProxyVariables::default(),
    ))
}

async fn stage(
    store: &FilesystemSessionStore,
    limits: DownloadLimits,
    tools: &PageTools,
    cancellation: &dyn StageCancellation,
) -> Result<SourceSnapshot, SourceError> {
    let session_id = SessionId::parse(SESSION).map_err(|_| SourceError::IdentityFailure)?;
    let operation =
        OperationId::parse("op_1111111111111111").map_err(|_| SourceError::IdentityFailure)?;
    let link = link().map_err(|_| SourceError::IdentityFailure)?;
    SourceSnapshot::stage_page(
        store,
        &session_id,
        &operation,
        (&link, limits),
        tools,
        cancellation,
    )
    .await
}

fn page_origin(snapshot: &SourceSnapshot) -> TestResult<&PageOrigin> {
    match snapshot.origin() {
        Some(SourceOrigin::PlatformPage(page)) => Ok(page),
        _ => Err("not a page's origin".into()),
    }
}

fn refused(result: Result<SourceSnapshot, SourceError>) -> TestResult<SourceFetchReason> {
    match result {
        Err(SourceError::Fetch(reason)) => Ok(reason),
        Err(other) => Err(format!("another failure: {other:?}").into()),
        Ok(_) => Err("it was not refused".into()),
    }
}

/// The arguments every yt-dlp run has, in order, as ADR 0028's note of
/// 2026-11-01 fixes them.
fn fixed_arguments() -> Vec<String> {
    [
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
        "!is_live & !was_live & live_status!=?is_upcoming & live_status!=?post_live",
        "--output",
        "vsift-page.%(format_id)s.%(ext)s",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// The names `VSift` may set on yt-dlp, whatever the platform adds.
fn allowed_variable(name: &str) -> bool {
    matches!(
        name,
        "TMPDIR"
            | "TEMP"
            | "TMP"
            | "HOME"
            | "YTDLP_NO_PLUGINS"
            | "DENO_NO_UPDATE_CHECK"
            | "NO_COLOR"
            | "HTTPS_PROXY"
            | "https_proxy"
            | "ALL_PROXY"
            | "all_proxy"
            | "NO_PROXY"
            | "no_proxy"
    ) || (cfg!(windows) && name == "SystemRoot")
}

/// One file: both phases get the exact arguments and environment, the file
/// becomes the session's source, the record holds no untrusted text, and
/// nothing else stays behind.
#[tokio::test]
async fn one_file_is_fetched_with_the_exact_invocation_and_recorded() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store().await?;
    let tools = managed_tools(
        &root,
        ProxyVariables::read(|name| {
            (name == "HTTPS_PROXY")
                .then(|| OsString::from(format!("http://user:{PROXY_SENTINEL}@proxy.test:3128")))
        }),
    )?;
    root.scenario(
        &single_info(),
        &[format!("write vsift-page.18.mp4 {MP4} 4096")],
    )?;
    let snapshot = stage(&store, DownloadLimits::DEFAULT, &tools, &NeverCancelled).await?;
    assert_eq!(snapshot.bytes(), 4_096);

    let calls = root.calls()?;
    let [metadata, download] = calls.as_slice() else {
        return Err(format!("two calls were expected: {calls:?}").into());
    };
    let deno = tools
        .js_runtime
        .as_ref()
        .ok_or("no runtime")?
        .executable
        .path();
    let output = metadata.after("--paths").ok_or("no --paths")?.to_owned();
    let tail = |phase: &[&str]| -> Vec<String> {
        let mut expected = fixed_arguments();
        expected.extend([
            String::from("--no-js-runtimes"),
            String::from("--js-runtimes"),
            format!("deno:{}", deno.display()),
            String::from("--paths"),
            output.clone(),
        ]);
        expected.extend(phase.iter().map(|argument| (*argument).to_owned()));
        expected.push(String::from("--"));
        expected.push(format!(
            "https://video.example.test/watch?v=abc&token={SENTINEL}"
        ));
        expected
    };
    assert_eq!(
        metadata.arguments,
        tail(&[
            "--format",
            PHASE_ONE_SELECTION,
            "--simulate",
            "--dump-single-json"
        ])
    );
    assert_eq!(
        download.arguments,
        tail(&["--format", "18[protocol=https]"])
    );

    for call in [metadata, download] {
        for (name, _) in &call.environment {
            assert!(allowed_variable(name), "{name} reached yt-dlp");
        }
        assert_eq!(call.variable("YTDLP_NO_PLUGINS"), Some("1"));
        assert_eq!(call.variable("DENO_NO_UPDATE_CHECK"), Some("1"));
        let home = call.variable("HOME").ok_or("no HOME")?;
        for name in ["TMPDIR", "TEMP", "TMP"] {
            assert_eq!(call.variable(name), Some(home), "{name}");
        }
        assert_ne!(home, output, "the output folder holds only the output");
        assert!(
            Path::new(home)
                .components()
                .any(|part| part.as_os_str() == SESSION),
            "the temporary folder is the session's"
        );
        assert!(
            call.variable("HTTPS_PROXY")
                .is_some_and(|value| value.contains(PROXY_SENTINEL))
        );
        assert!(
            call.arguments
                .iter()
                .all(|argument| !argument.contains(PROXY_SENTINEL)),
            "a proxy's credential is never on the command line"
        );
    }

    let origin = page_origin(&snapshot)?;
    assert_eq!(origin.provider().as_str(), "Generic");
    assert_eq!(
        origin.video_id(),
        &PageVideoId::Public(String::from("abc-1"))
    );
    assert_eq!(origin.joined(), PageJoin::SingleFile);
    assert!(origin.through_proxy());
    assert_eq!(origin.bytes(), 4_096);
    assert!(matches!(
        origin.js_runtime(),
        Some(PageToolRecord::Managed { .. })
    ));
    let printed = format!("{origin:?}");
    for sentinel in [
        SENTINEL,
        PROXY_SENTINEL,
        "TITLE-SENTINEL",
        "UPLOADER-SENTINEL",
        "URL-SENTINEL",
    ] {
        assert!(!printed.contains(sentinel), "{sentinel}");
        assert!(!root.session_bytes()?.contains(sentinel), "{sentinel}");
    }
    let left = root.session_files()?;
    assert_eq!(left.len(), 1, "only the source stays: {left:?}");
    Ok(())
}

/// A user's yt-dlp with no runtime is told `--no-js-runtimes` alone, gets no
/// runtime's variables, and the record says it is the user's. A runtime on
/// the machine is never found: nothing here looks one up.
#[tokio::test]
async fn a_users_ytdlp_runs_with_no_runtime_when_it_has_none() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store().await?;
    let tools = user_tools(&root)?;
    root.scenario(
        &single_info(),
        &[format!("write vsift-page.18.mp4 {MP4} 4096")],
    )?;
    let snapshot = stage(&store, DownloadLimits::DEFAULT, &tools, &NeverCancelled).await?;
    for call in root.calls()? {
        assert!(call.arguments.iter().any(|a| a == "--no-js-runtimes"));
        assert!(!call.arguments.iter().any(|a| a == "--js-runtimes"));
        assert_eq!(call.variable("DENO_NO_UPDATE_CHECK"), None);
    }
    let origin = page_origin(&snapshot)?;
    assert!(matches!(
        origin.yt_dlp(),
        PageToolRecord::UserManaged { .. }
    ));
    assert_eq!(origin.js_runtime(), None);
    assert!(!origin.through_proxy());
    Ok(())
}

/// Separate audio and video are joined by `FFmpeg`'s stream copy into the
/// slot, which alone stays, and an unlisted video's id is kept only as a digest.
#[tokio::test]
async fn separate_streams_are_joined_and_an_unlisted_id_is_not_kept() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store().await?;
    let tools = managed_tools(&root, ProxyVariables::default())?;
    root.scenario(
        &pair_info("mp4", "m4a"),
        &[
            format!("write vsift-page.137.mp4 {MP4} 3000"),
            format!("write vsift-page.140.m4a {MP4} 1000"),
        ],
    )?;
    let snapshot = stage(&store, DownloadLimits::DEFAULT, &tools, &NeverCancelled).await?;
    assert_eq!(snapshot.bytes(), 4_000);
    let calls = root.calls()?;
    let [_, download, join] = calls.as_slice() else {
        return Err(format!("three calls were expected: {calls:?}").into());
    };
    assert_eq!(
        download.after("--format"),
        Some("137[protocol=https],140[protocol=https]"),
        "exactly the ids phase 1 checked, as two separate downloads"
    );
    assert_eq!(join.after("-c"), Some("copy"));
    assert_eq!(join.after("-map_metadata"), Some("-1"));
    assert_eq!(
        join.arguments
            .iter()
            .filter(|a| *a == "-protocol_whitelist")
            .count(),
        2,
        "every input is restricted"
    );
    assert_eq!(
        join.environment.iter().filter(|(n, _)| n == "HOME").count(),
        0
    );
    let origin = page_origin(&snapshot)?;
    assert_eq!(origin.joined(), PageJoin::AudioVideo);
    assert!(matches!(origin.video_id(), PageVideoId::Digest(_)));
    assert!(!root.session_bytes()?.contains("abc-2"));
    assert_eq!(root.session_files()?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn webm_streams_are_joined_as_matroska() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store().await?;
    let tools = user_tools(&root)?;
    root.scenario(
        &pair_info("webm", "webm"),
        &[
            format!("write vsift-page.137.webm {MKV} 3000"),
            format!("write vsift-page.140.webm {MKV} 1000"),
        ],
    )?;
    stage(&store, DownloadLimits::DEFAULT, &tools, &NeverCancelled).await?;
    let calls = root.calls()?;
    let join = calls.last().ok_or("no call")?;
    assert_eq!(join.after("-f"), Some("matroska"));
    assert!(!join.arguments.iter().any(|a| a == "-enable_drefs"));
    assert_eq!(
        join.arguments.iter().rev().nth(1).map(String::as_str),
        Some("matroska"),
        "the output's format comes last, before the slot"
    );
    Ok(())
}

/// A page that needs a join with no `FFmpeg` set up stops before downloading.
#[tokio::test]
async fn a_pair_without_ffmpeg_is_refused_before_the_download() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store().await?;
    let mut tools = user_tools(&root)?;
    tools.ffmpeg = None;
    root.scenario(&pair_info("mp4", "m4a"), &[])?;
    let reason = refused(stage(&store, DownloadLimits::DEFAULT, &tools, &NeverCancelled).await)?;
    assert_eq!(reason, SourceFetchReason::MediaToolsMissing);
    assert_eq!(root.calls()?.len(), 1, "phase 2 never started");
    assert_eq!(root.session_files()?, Vec::<PathBuf>::new());
    Ok(())
}

/// Whatever yt-dlp leaves, only exactly the expected regular files pass, and
/// any other outcome leaves nothing.
#[tokio::test]
async fn anything_but_the_expected_files_is_refused() -> TestResult {
    let mp4 = format!("write vsift-page.18.mp4 {MP4} 4096");
    let mut cases = vec![
        ("nothing", Vec::new(), SourceFetchReason::UnexpectedOutput),
        (
            "two files for one",
            vec![mp4.clone(), format!("write vsift-page.19.mp4 {MP4} 16")],
            SourceFetchReason::UnexpectedOutput,
        ),
        (
            "the stem alone",
            vec![format!("write vsift-page.mp4 {MP4} 16")],
            SourceFetchReason::UnexpectedOutput,
        ),
        (
            "a wrong container",
            vec![format!("write vsift-page.18.mp4 {NOT_MEDIA} 64")],
            SourceFetchReason::NotAVideo,
        ),
        (
            "too short to hold magic",
            vec![String::from("write vsift-page.18.mp4 00 4")],
            SourceFetchReason::NotAVideo,
        ),
    ];
    if cfg!(unix) {
        cases.push((
            "a link",
            vec![String::from("link vsift-page.18.mp4 /etc/hostname")],
            SourceFetchReason::UnexpectedOutput,
        ));
    }
    for (name, lines, expected) in cases {
        let root = OwnedRoot::new()?;
        let store = root.store().await?;
        let tools = user_tools(&root)?;
        root.scenario(&single_info(), &lines)?;
        let reason =
            refused(stage(&store, DownloadLimits::DEFAULT, &tools, &NeverCancelled).await)?;
        assert_eq!(reason, expected, "{name}");
        assert_eq!(root.session_files()?, Vec::<PathBuf>::new(), "{name}");
    }
    Ok(())
}

/// Bytes are counted on disk: a file over the limit is stopped whatever the
/// page declared, and yt-dlp's text never decides it.
#[tokio::test]
async fn the_size_limit_holds_on_disk() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store().await?;
    let tools = user_tools(&root)?;
    root.scenario(
        &single_info(),
        &[format!("write vsift-page.18.mp4 {MP4} 4096")],
    )?;
    let limits = DownloadLimits::new(Some(1_024), None)?;
    assert_eq!(
        refused(stage(&store, limits, &tools, &NeverCancelled).await)?,
        SourceFetchReason::Size
    );
    assert_eq!(root.session_files()?, Vec::<PathBuf>::new());
    Ok(())
}

#[tokio::test]
async fn a_hung_tool_and_a_stalled_download_are_stopped() -> TestResult {
    let waits = |tools: PageTools| {
        tools.with_waits_for_tests(Duration::from_millis(400), Duration::from_millis(400))
    };
    let growing = format!("grow vsift-page.18.mp4 {MP4} 1024");
    for (name, lines, limits, expected) in [
        (
            "metadata hangs",
            vec![String::from("hang-phase 1")],
            DownloadLimits::new(None, Some(1))?,
            SourceFetchReason::Deadline,
        ),
        (
            "no first byte",
            vec![String::from("hang-phase 2")],
            DownloadLimits::DEFAULT,
            SourceFetchReason::Stalled,
        ),
        (
            "bytes stop arriving",
            vec![growing],
            DownloadLimits::DEFAULT,
            SourceFetchReason::Stalled,
        ),
    ] {
        let root = OwnedRoot::new()?;
        let store = root.store().await?;
        let tools = waits(user_tools(&root)?);
        root.scenario(&single_info(), &lines)?;
        assert_eq!(
            refused(stage(&store, limits, &tools, &NeverCancelled).await)?,
            expected,
            "{name}"
        );
        assert_eq!(root.session_files()?, Vec::<PathBuf>::new(), "{name}");
    }
    Ok(())
}

/// One scenario per phrase of section 4.8, on either phase's failure, and an
/// unrecognised one. The text chooses the reason and goes nowhere else.
#[tokio::test]
async fn every_phrase_gives_its_typed_reason_and_is_not_echoed() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store().await?;
    let tools = user_tools(&root)?;
    let mut cases: Vec<(String, SourceFetchReason)> = vec![(
        String::from("something new went wrong"),
        SourceFetchReason::OnlineStepFailed,
    )];
    for (reason, phrases) in YTDLP_PHRASES.into_iter().chain(SITE_PHRASES) {
        cases.extend(phrases.iter().map(|phrase| ((*phrase).to_owned(), reason)));
    }
    for (phrase, expected) in cases {
        root.scenario(
            "",
            &[format!("stderr ERROR: {phrase}"), String::from("exit1 1")],
        )?;
        let result = stage(&store, DownloadLimits::DEFAULT, &tools, &NeverCancelled).await;
        let printed = format!("{:?}", result.as_ref().err());
        assert_eq!(refused(result)?, expected, "{phrase}");
        assert!(!printed.contains(&phrase), "{phrase}");
    }
    Ok(())
}

/// With `--no-part` a failed download leaves a file with good magic under
/// its final name, so only the exit status tells: anything but 0 is a failure,
/// whatever the folder holds, 101 included.
#[tokio::test]
async fn a_good_looking_file_from_a_failed_run_is_refused() -> TestResult {
    for (name, info, lines) in [
        (
            "a first stream that failed, its partial file left",
            single_info(),
            vec![
                format!("write vsift-page.18.mp4 {MP4} 2048"),
                String::from("exit2 1"),
            ],
        ),
        (
            "a pair whose second stream failed, the first file left",
            pair_info("mp4", "m4a"),
            vec![
                format!("write vsift-page.137.mp4 {MP4} 2048"),
                String::from("exit2 1"),
            ],
        ),
        (
            "both streams there and exit 101",
            pair_info("mp4", "m4a"),
            vec![
                format!("write vsift-page.137.mp4 {MP4} 2048"),
                format!("write vsift-page.140.m4a {MP4} 2048"),
                String::from("exit2 101"),
            ],
        ),
        (
            "one file there and exit 101",
            single_info(),
            vec![
                format!("write vsift-page.18.mp4 {MP4} 2048"),
                String::from("exit2 101"),
            ],
        ),
    ] {
        let root = OwnedRoot::new()?;
        let store = root.store().await?;
        let tools = user_tools(&root)?;
        root.scenario(&info, &lines)?;
        assert_eq!(
            refused(stage(&store, DownloadLimits::DEFAULT, &tools, &NeverCancelled).await)?,
            SourceFetchReason::OnlineStepFailed,
            "{name}"
        );
        assert_eq!(root.session_files()?, Vec::<PathBuf>::new(), "{name}");
    }
    Ok(())
}

/// A phrase chooses a reason and nothing else. A run that exited 0 is judged
/// by the folder, whatever its text says: with nothing there it fails that
/// check, and with the expected file it passes.
#[tokio::test]
async fn a_site_phrase_only_chooses_a_reason() -> TestResult {
    let wall = "stderr ERROR: Sign in to confirm you\u{2019}re not a bot";
    let root = OwnedRoot::new()?;
    let store = root.store().await?;
    let tools = user_tools(&root)?;
    root.scenario(&single_info(), &[String::from(wall)])?;
    assert_eq!(
        refused(stage(&store, DownloadLimits::DEFAULT, &tools, &NeverCancelled).await)?,
        SourceFetchReason::UnexpectedOutput
    );

    let root = OwnedRoot::new()?;
    let store = root.store().await?;
    let tools = user_tools(&root)?;
    root.scenario(
        &single_info(),
        &[
            String::from(wall),
            format!("write vsift-page.18.mp4 {MP4} 4096"),
        ],
    )?;
    let snapshot = stage(&store, DownloadLimits::DEFAULT, &tools, &NeverCancelled).await?;
    assert_eq!(snapshot.bytes(), 4_096);
    Ok(())
}

/// A metadata run that stops at exit 101 is a failure too.
#[tokio::test]
async fn exit_101_is_not_a_success_in_phase_one() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store().await?;
    let tools = user_tools(&root)?;
    root.scenario(&single_info(), &[String::from("exit1 101")])?;
    assert_eq!(
        refused(stage(&store, DownloadLimits::DEFAULT, &tools, &NeverCancelled).await)?,
        SourceFetchReason::OnlineStepFailed
    );
    assert_eq!(root.calls()?.len(), 1);
    Ok(())
}

/// An id outside the pattern never reaches a selection or a file's name.
#[tokio::test]
async fn a_format_id_outside_the_pattern_is_refused_before_the_download() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store().await?;
    let tools = user_tools(&root)?;
    root.scenario(
        &single_info().replace(r#""format_id":"18""#, r#""format_id":"18,../x""#),
        &[],
    )?;
    assert_eq!(
        refused(stage(&store, DownloadLimits::DEFAULT, &tools, &NeverCancelled).await)?,
        SourceFetchReason::UnexpectedOutput
    );
    assert_eq!(root.calls()?.len(), 1, "phase 2 never started");
    Ok(())
}

#[tokio::test]
async fn a_failed_download_is_read_the_same_way() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store().await?;
    let tools = user_tools(&root)?;
    root.scenario(
        &single_info(),
        &[
            String::from("stderr ERROR: HTTP Error 403: Forbidden"),
            String::from("exit2 1"),
        ],
    )?;
    assert_eq!(
        refused(stage(&store, DownloadLimits::DEFAULT, &tools, &NeverCancelled).await)?,
        SourceFetchReason::Network
    );
    assert_eq!(root.session_files()?, Vec::<PathBuf>::new());
    Ok(())
}

struct CancelledNow;

impl StageCancellation for CancelledNow {
    fn is_cancelled(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn a_cancelled_fetch_stops_the_tool_and_leaves_nothing() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store().await?;
    let tools = user_tools(&root)?;
    root.scenario(&single_info(), &[String::from("hang-phase 1")])?;
    let result = stage(&store, DownloadLimits::DEFAULT, &tools, &CancelledNow).await;
    assert!(matches!(result, Err(SourceError::Cancelled)));
    assert_eq!(root.session_files()?, Vec::<PathBuf>::new());
    Ok(())
}

/// Section 3's rules hold before yt-dlp starts, and a host written as an
/// address or an internal name never reaches it: it connects with no policy.
#[tokio::test]
async fn a_link_that_may_not_reach_ytdlp_never_starts_it() -> TestResult {
    let root = OwnedRoot::new()?;
    let store = root.store().await?;
    let tools = user_tools(&root)?;
    root.scenario(&single_info(), &[])?;
    let session_id = SessionId::parse(SESSION)?;
    let operation = OperationId::parse("op_1111111111111111")?;
    for (text, expected) in [
        ("https://8.8.8.8/page", SourceFetchReason::AddressRefused),
        ("https://127.0.0.1/page", SourceFetchReason::AddressRefused),
        ("https://video.corp/page", SourceFetchReason::AddressRefused),
        ("https://intranet/page", SourceFetchReason::AddressRefused),
        (
            "https://video.example.test:8443/page",
            SourceFetchReason::PortNotAllowed,
        ),
        (
            "https://user:pass@video.example.test/page",
            SourceFetchReason::CredentialsInLink,
        ),
    ] {
        let link = SourceLink::parse(text)?;
        let result = SourceSnapshot::stage_page(
            &store,
            &session_id,
            &operation,
            (&link, DownloadLimits::DEFAULT),
            &tools,
            &NeverCancelled,
        )
        .await;
        assert_eq!(refused(result)?, expected, "{text}");
    }
    assert_eq!(root.calls()?.len(), 0);
    Ok(())
}

/// A local HTTPS server answering every request with `reply`, on the committed
/// test certificates.
fn start_server(reply: &'static [u8]) -> TestResult<u16> {
    let fixture = |name: &str| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/tls")
            .join(name)
    };
    let identity = Identity::from_pkcs8(
        &fs::read(fixture("link-test-server.cert.pem"))?,
        &fs::read(fixture("link-test-server.key.pem"))?,
    )?;
    let acceptor = Arc::new(TlsAcceptor::new(identity)?);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let acceptor = Arc::clone(&acceptor);
            thread::spawn(move || {
                if let Ok(mut tls) = acceptor.accept(stream) {
                    let mut lines = BufReader::new(&mut tls);
                    let mut line = String::new();
                    while lines.read_line(&mut line).is_ok_and(|read| read > 2) {
                        line.clear();
                    }
                    let _ = tls.write_all(reply);
                }
            });
        }
    });
    Ok(port)
}

const HTML_REPLY: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 6\r\nConnection: close\r\n\r\n<html>";
const VIDEO_REPLY: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 24\r\nConnection: close\r\n\r\n\0\0\0\x18ftypisom\0\0\x02\0isomiso2";

struct Fixed(Option<PageTools>);

impl PageToolSource for Fixed {
    async fn page_tools(&self) -> Option<PageTools> {
        self.0.clone()
    }
}

async fn stage_routed(
    root: &OwnedRoot,
    reply: &'static [u8],
    tools: Option<PageTools>,
) -> TestResult<Result<vsift_infrastructure::SourceSnapshot, OpenSessionError>> {
    let store = root.store().await?;
    let port = start_server(reply)?;
    let answers = Arc::new(TestAnswers::default());
    answers.answer(
        "video.example.test",
        vec![vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]],
    );
    let route = LinkTestRoute::new(
        port,
        fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/tls/link-test-ca.cert.pem"),
        )?,
        answers,
    );
    let routed = PageSourceStore::new(store, Fixed(tools)).through_test_route(route);
    let link = SourceLink::parse(&format!("https://video.example.test:{port}/watch"))?;
    Ok(routed
        .stage_link(
            &SessionId::parse(SESSION)?,
            &OperationId::parse("op_1111111111111111")?,
            &link,
            DownloadLimits::DEFAULT,
            &NeverCancelled,
        )
        .await)
}

/// The routing: a link that answers with a web page goes to yt-dlp once the
/// direct fetch has refused it, and the same slot is used.
#[tokio::test]
async fn a_web_page_goes_to_ytdlp_when_one_is_resolved() -> TestResult {
    let root = OwnedRoot::new()?;
    let tools = user_tools(&root)?;
    root.scenario(
        &single_info(),
        &[format!("write vsift-page.18.mp4 {MP4} 4096")],
    )?;
    let snapshot = stage_routed(&root, HTML_REPLY, Some(tools)).await??;
    assert_eq!(snapshot.source_bytes(), 4_096);
    assert!(matches!(
        StagedSessionSource::origin(&snapshot),
        Some(SourceOrigin::PlatformPage(_))
    ));
    assert_eq!(root.calls()?.len(), 2);
    Ok(())
}

#[tokio::test]
async fn a_web_page_stays_refused_when_no_ytdlp_is_resolved() -> TestResult {
    let root = OwnedRoot::new()?;
    let result = stage_routed(&root, HTML_REPLY, None).await?;
    assert!(matches!(
        result,
        Err(OpenSessionError::SourceFetch(SourceFetchReason::WebPage))
    ));
    assert_eq!(root.session_files()?, Vec::<PathBuf>::new());
    Ok(())
}

/// A direct link never starts yt-dlp, even with one set up.
#[tokio::test]
async fn a_direct_video_link_never_starts_ytdlp() -> TestResult {
    let root = OwnedRoot::new()?;
    let tools = user_tools(&root)?;
    root.scenario(&single_info(), &[])?;
    let snapshot = stage_routed(&root, VIDEO_REPLY, Some(tools)).await??;
    assert!(matches!(
        StagedSessionSource::origin(&snapshot),
        Some(SourceOrigin::DirectHttps(_))
    ));
    assert_eq!(root.calls()?.len(), 0);
    Ok(())
}
