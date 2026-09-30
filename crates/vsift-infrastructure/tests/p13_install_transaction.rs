//! P13 PR 4: the managed-install transaction on real private storage, the
//! real HTTP transport and the real compatibility smoke (D-02, D-03, D-06,
//! D-07), on every CI operating system and with no network.
//!
//! A test catalogue of small fixture artifacts (the `vsift-smoke-fixture`
//! executable under the reviewed tool names, packed as gzip/tar, and a raw
//! model file) is served by a local HTTP server on `127.0.0.1` through the
//! development-only loopback route (`install-test-hooks`). The server
//! records every request, so the tests can prove that no attempt ever sends
//! a `Range` header, and can drop, truncate, alter, redirect or refuse a
//! response. A second local server acts as a proxy that demands
//! authentication, and a third presents a certificate no trust store
//! accepts. The smoke's fixture verifiers are fixed verdicts; everything
//! else is production code: download or import, exact size and SHA-256,
//! bounded extraction, runtime layout, smoke, publication and selection.

use std::{
    collections::{BTreeMap, VecDeque},
    env, fs,
    future::Future,
    io::{BufRead, BufReader, Cursor, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};
use tar::{Builder, Header};
use vsift_application::{
    AcceptedManagedArtifact, AcceptedManagedCatalogue, CompatibilitySmokeCheck,
    CompatibilitySmokeFailure, CompatibilitySmokeFailureReason, ComponentInstallFailure,
    ComponentInstallOutcome, DownloadFailureReason, InstallFailureReason, InstallStep,
    LocalAsrVerification, LocalAsrVerifier, ManagedInstallReport, ManagedSetupAction,
    MediaToolCheck, MediaToolFailure, MediaToolVerification, MediaToolVerifier, NoProgress,
    ProgressSink, ReviewedArchiveLimits, ReviewedArchiveSelection, ReviewedCompatibilityPolicy,
    ReviewedManagedFile, RuntimeDiagnosis, SetupProfile, SetupSelectionState, StageDisposal,
    install_managed_components, plan_managed_setup,
};
use vsift_domain::{
    ArtifactIntegrity, DependencyState, DependencyStatus, FailureCode, ManagedArtifactFormat,
    ManagedComponent, ManagedTarget, ProgressStage, ProgressUpdate, RuntimeDependency,
    RuntimeReadiness,
};
use vsift_infrastructure::{
    ActionAuthority, HostIsolation, ManagedArtifactSource, ManagedArtifactStore,
    ManagedInstallerConfig, ManagedRuntimeHold, ManagedRuntimeRole, MediaProviderConformance,
    MediaSmokeRequest, ProcessCancellation, ReviewedManagedInstaller, SmokeCompanionSource,
    SmokeCompanions, SmokeFixtureVerifiers, SpeechSmokeRequest, TrustedExecutable,
    managed_executable_name, reviewed_compatibility_policy,
};

// P13 PR 7: the kill tests of the managed store reuse this file's fixture
// catalogue, local publisher and smoke doubles, and need the development
// fault points as well (a workspace test run enables both features).
#[cfg(feature = "fault-injection")]
#[path = "p13_install_transaction/kill.rs"]
mod kill;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const FIXTURE_TOOL: &str = env!("CARGO_BIN_EXE_vsift-smoke-fixture");
const FIXTURE_BUILD: &str = "vsift-smoke-fixture";
const MODEL_BYTES: &[u8] = b"fixture speech model bytes";
const SENTINEL: &str = "vsift-sentinel-proxy-secret-7c1f";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

const MEDIA: &str = "ffmpeg_ffprobe";
const WHISPER: &str = "whisper_cli";
const MODEL: &str = "whisper_model";

// ---------------------------------------------------------------------------
// A local HTTP server with scripted replies.

/// What the server answers for one request to a path.
#[derive(Clone, Debug)]
enum Reply {
    /// `200 OK` with the body and its exact length.
    Body(Vec<u8>),
    /// `200 OK` declaring the whole length, then only the first `sent`
    /// bytes before the connection is closed.
    DropAfter { body: Vec<u8>, sent: usize },
    /// `200 OK` declaring the whole length, then the first `sent` bytes and
    /// a stall long past any test.
    StallAfter { body: Vec<u8>, sent: usize },
    /// `206 Partial Content` with a `Content-Range` for the whole body.
    Partial(Vec<u8>),
    /// `302 Found` to the location.
    Redirect(String),
    /// A status line and no body.
    Status(u16, &'static str),
    /// `407 Proxy Authentication Required`, for a proxy.
    ProxyAuth,
}

/// One request the server read.
#[derive(Clone, Debug)]
struct Recorded {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
}

impl Recorded {
    fn has_header(&self, name: &str) -> bool {
        self.headers.iter().any(|(header, _)| header == name)
    }
}

#[derive(Default)]
struct ServerState {
    routes: BTreeMap<String, VecDeque<Reply>>,
    fallback: Option<Reply>,
    requests: Vec<Recorded>,
}

struct TestServer {
    port: u16,
    state: Arc<Mutex<ServerState>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl TestServer {
    fn start() -> TestResult<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let state = Arc::new(Mutex::new(ServerState::default()));
        let shared = Arc::clone(&state);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = Arc::clone(&shared);
                thread::spawn(move || {
                    let _ = serve(stream, &state);
                });
            }
        });
        Ok(Self { port, state })
    }

    /// A server that answers every request with `reply`.
    fn answering(reply: Reply) -> TestResult<Self> {
        let server = Self::start()?;
        lock(&server.state).fallback = Some(reply);
        Ok(server)
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Answers `path` with each reply in turn, the last one from then on.
    fn route(&self, path: &str, replies: Vec<Reply>) {
        lock(&self.state)
            .routes
            .insert(path.to_owned(), replies.into());
    }

    fn requests(&self) -> Vec<Recorded> {
        lock(&self.state).requests.clone()
    }
}

fn serve(stream: TcpStream, state: &Mutex<ServerState>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let path = target
        .find("://")
        .and_then(|scheme| {
            target[scheme + 3..]
                .find('/')
                .map(|slash| &target[scheme + 3 + slash..])
        })
        .unwrap_or(&target)
        .to_owned();
    let reply = {
        let mut state = lock(state);
        state.requests.push(Recorded {
            method,
            target,
            headers,
        });
        let routed = state.routes.get_mut(&path).and_then(|replies| {
            if replies.len() > 1 {
                replies.pop_front()
            } else {
                replies.front().cloned()
            }
        });
        routed.or_else(|| state.fallback.clone())
    };
    write_reply(stream, reply)
}

fn write_reply(mut stream: TcpStream, reply: Option<Reply>) -> std::io::Result<()> {
    match reply {
        Some(Reply::Body(body)) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(&body)?;
        }
        Some(Reply::DropAfter { body, sent }) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(&body[..sent.min(body.len())])?;
            stream.flush()?;
        }
        Some(Reply::StallAfter { body, sent }) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(&body[..sent.min(body.len())])?;
            stream.flush()?;
            thread::sleep(Duration::from_secs(20));
        }
        Some(Reply::Partial(body)) => {
            write!(
                stream,
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len().saturating_sub(1),
                body.len(),
                body.len()
            )?;
            stream.write_all(&body)?;
        }
        Some(Reply::Redirect(location)) => {
            write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
        }
        Some(Reply::Status(code, reason)) => {
            write!(
                stream,
                "HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
        }
        Some(Reply::ProxyAuth) => {
            write!(
                stream,
                "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"vsift-test\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
        }
        None => {
            write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
        }
    }
    stream.flush()?;
    stream.shutdown(Shutdown::Both)
}

/// A TLS server whose certificate no trust store accepts.
fn start_untrusted_tls_server() -> TestResult<u16> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tls");
    let certificate = fs::read(directory.join("untrusted-loopback.cert.pem"))?;
    let key = fs::read(directory.join("untrusted-loopback.key.pem"))?;
    let identity = native_tls::Identity::from_pkcs8(&certificate, &key)?;
    let acceptor = native_tls::TlsAcceptor::new(identity)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let acceptor = acceptor.clone();
            thread::spawn(move || {
                if let Ok(mut tls) = acceptor.accept(stream) {
                    let mut buffer = [0_u8; 1024];
                    let _ = tls.read(&mut buffer);
                    let _ = tls.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
            });
        }
    });
    Ok(port)
}

// ---------------------------------------------------------------------------
// The fixture catalogue.

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(DIGITS[usize::from(byte >> 4)]));
        result.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    result
}

fn integrity_of(bytes: &[u8]) -> TestResult<ArtifactIntegrity> {
    Ok(ArtifactIntegrity::from_sha256_hex(
        u64::try_from(bytes.len())?,
        &hex(&Sha256::digest(bytes)),
    )?)
}

fn role_name(role: ManagedRuntimeRole) -> TestResult<String> {
    managed_executable_name(role).ok_or_else(|| "the role has no executable name".into())
}

/// One file of a fixture archive.
struct ArchiveFile {
    name: String,
    bytes: Vec<u8>,
    executable: bool,
}

/// A gzip/tar artifact of `files` under `folder/`, reviewed like the real
/// catalogue's archives.
fn archive_artifact(
    component: ManagedComponent,
    version: &str,
    file_name: &str,
    folder: &str,
    files: &[ArchiveFile],
) -> TestResult<(AcceptedManagedArtifact, Vec<u8>)> {
    let mut tar = Builder::new(Vec::new());
    let mut expanded = 0_u64;
    for file in files {
        let mut header = Header::new_gnu();
        header.set_path(format!("{folder}/{}", file.name))?;
        header.set_size(u64::try_from(file.bytes.len())?);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append(&header, Cursor::new(&file.bytes))?;
        expanded += u64::try_from(file.bytes.len())?;
    }
    let tar = tar.into_inner()?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&tar)?;
    let bytes = encoder.finish()?;
    let artifact = AcceptedManagedArtifact {
        component,
        version: version.to_owned(),
        publisher: String::from("fixture publisher"),
        source_url: format!("https://publisher.invalid/fixture/{file_name}"),
        integrity: integrity_of(&bytes)?,
        format: ManagedArtifactFormat::TarGz,
        archive_limits: Some(ReviewedArchiveLimits {
            max_stream_bytes: u64::try_from(tar.len())?,
            entries: files.len(),
            expanded_bytes: expanded,
        }),
        selected_files: files
            .iter()
            .map(|file| {
                Ok(ReviewedArchiveSelection {
                    archive_path: format!("{folder}/{}", file.name),
                    runtime_name: file.name.clone(),
                    integrity: integrity_of(&file.bytes)?,
                })
            })
            .collect::<TestResult<Vec<_>>>()?,
        archive_links: Vec::new(),
        runtime_copies: Vec::new(),
        licence: String::from("MIT"),
        notice_url: String::from("https://publisher.invalid/notice"),
        source_code_url: String::from("https://publisher.invalid/source"),
        trust_limit: String::from("fixture evidence only"),
        files: files
            .iter()
            .map(|file| {
                Ok(ReviewedManagedFile {
                    name: file.name.clone(),
                    integrity: integrity_of(&file.bytes)?,
                    executable: file.executable,
                })
            })
            .collect::<TestResult<Vec<_>>>()?,
    };
    Ok((artifact, bytes))
}

/// The fixture catalogue and the bytes of each artifact by file name.
struct Fixture {
    catalogue: AcceptedManagedCatalogue,
    files: BTreeMap<String, Vec<u8>>,
}

const MEDIA_FILE: &str = "fixture-media.tar.gz";
const WHISPER_FILE: &str = "fixture-whisper.tar.gz";
const MODEL_FILE: &str = "fixture-model.bin";

impl Fixture {
    /// The fixture catalogue; `media_version` names the media tools'
    /// version, whose notice file differs with it so its bytes do too.
    fn new(media_version: &str) -> TestResult<Self> {
        let tool = fs::read(FIXTURE_TOOL)?;
        let (media, media_bytes) = archive_artifact(
            ManagedComponent::MediaTools,
            media_version,
            MEDIA_FILE,
            "media",
            &[
                ArchiveFile {
                    name: role_name(ManagedRuntimeRole::Ffmpeg)?,
                    bytes: tool.clone(),
                    executable: true,
                },
                ArchiveFile {
                    name: role_name(ManagedRuntimeRole::Ffprobe)?,
                    bytes: tool.clone(),
                    executable: true,
                },
                ArchiveFile {
                    name: String::from("NOTICE.txt"),
                    bytes: format!("fixture media tools {media_version}\n").into_bytes(),
                    executable: false,
                },
            ],
        )?;
        let (whisper, whisper_bytes) = archive_artifact(
            ManagedComponent::WhisperCli,
            "fixture-whisper-1",
            WHISPER_FILE,
            "whisper",
            &[ArchiveFile {
                name: role_name(ManagedRuntimeRole::WhisperCli)?,
                bytes: tool,
                executable: true,
            }],
        )?;
        let model = AcceptedManagedArtifact {
            component: ManagedComponent::WhisperModel,
            version: String::from("fixture-model-1"),
            publisher: String::from("fixture publisher"),
            source_url: format!("https://publisher.invalid/fixture/{MODEL_FILE}"),
            integrity: integrity_of(MODEL_BYTES)?,
            format: ManagedArtifactFormat::RawFile,
            archive_limits: None,
            selected_files: Vec::new(),
            archive_links: Vec::new(),
            runtime_copies: Vec::new(),
            licence: String::from("MIT"),
            notice_url: String::from("https://publisher.invalid/notice"),
            source_code_url: String::from("https://publisher.invalid/source"),
            trust_limit: String::from("fixture evidence only"),
            files: vec![ReviewedManagedFile {
                name: MODEL_FILE.to_owned(),
                integrity: integrity_of(MODEL_BYTES)?,
                executable: false,
            }],
        };
        let catalogue = AcceptedManagedCatalogue {
            revision: String::from("p13-install-fixture-r1"),
            target: ManagedTarget::Ubuntu2404X86_64,
            stop_new_plans_at: 2_000,
            stop_new_plans_date: String::from("fixture-date"),
            artifacts: vec![media, whisper, model],
            compatibility: policy()?,
        };
        let files = BTreeMap::from([
            (MEDIA_FILE.to_owned(), media_bytes),
            (WHISPER_FILE.to_owned(), whisper_bytes),
            (MODEL_FILE.to_owned(), MODEL_BYTES.to_vec()),
        ]);
        Ok(Self { catalogue, files })
    }

    fn bytes(&self, file: &str) -> TestResult<Vec<u8>> {
        self.files
            .get(file)
            .cloned()
            .ok_or_else(|| format!("no fixture file {file}").into())
    }

    /// The accepted plan's actions for a host with none of the tools.
    fn actions(&self) -> TestResult<Vec<ManagedSetupAction>> {
        let plan = plan_managed_setup(
            SetupProfile::Desktop,
            RuntimeDiagnosis {
                readiness: RuntimeReadiness::Blocked,
                dependencies: RuntimeDependency::ALL
                    .into_iter()
                    .map(|dependency| DependencyStatus {
                        dependency,
                        state: DependencyState::Missing,
                    })
                    .collect(),
            },
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_000,
            Some(self.catalogue.clone()),
        );
        let digest = plan
            .digest
            .clone()
            .ok_or("the fixture plan has no digest")?;
        plan.validate_acceptance(&digest)
            .map_err(|error| format!("{error:?}"))?;
        if plan.actions.len() != 3 {
            return Err("the fixture plan does not install three components".into());
        }
        Ok(plan.actions)
    }

    /// Serves every artifact from `server`.
    fn serve_all(&self, server: &TestServer) {
        for (file, bytes) in &self.files {
            server.route(&format!("/{file}"), vec![Reply::Body(bytes.clone())]);
        }
    }
}

fn policy() -> TestResult<ReviewedCompatibilityPolicy> {
    Ok(ReviewedCompatibilityPolicy {
        expected_ffmpeg_version: format!("ffmpeg version {FIXTURE_BUILD}"),
        expected_ffprobe_version: format!("ffprobe version {FIXTURE_BUILD}"),
        media_deadline_seconds: 30,
        inference_deadline_seconds: 30,
        ..reviewed_compatibility_policy()?
    })
}

// ---------------------------------------------------------------------------
// Smoke verifiers and companions.

#[derive(Clone, Copy)]
struct FixedVerifiers {
    media: MediaToolVerification,
    speech: LocalAsrVerification,
}

impl FixedVerifiers {
    const fn passing() -> Self {
        Self {
            media: MediaToolVerification::Verified,
            speech: LocalAsrVerification::Verified,
        }
    }
}

struct Fixed<T>(T);

impl MediaToolVerifier for Fixed<MediaToolVerification> {
    fn verify(&self) -> impl Future<Output = MediaToolVerification> + Send {
        std::future::ready(self.0)
    }
}

impl LocalAsrVerifier for Fixed<LocalAsrVerification> {
    fn verify(&self) -> impl Future<Output = LocalAsrVerification> + Send {
        std::future::ready(self.0)
    }
}

impl SmokeFixtureVerifiers for FixedVerifiers {
    type Media = Fixed<MediaToolVerification>;
    type Speech = Fixed<LocalAsrVerification>;

    fn media(&self, _request: MediaSmokeRequest) -> Self::Media {
        Fixed(self.media)
    }

    fn speech(&self, _request: SpeechSmokeRequest) -> Self::Speech {
        Fixed(self.speech)
    }
}

/// Companions from the managed store alone: what it has selected now.
struct StoreCompanions {
    store: ManagedArtifactStore,
}

impl StoreCompanions {
    fn hold(&self, component: &str) -> Option<ManagedRuntimeHold> {
        self.store
            .open_selected_runtime(component)
            .ok()
            .flatten()
            .map(ManagedRuntimeHold::new)
    }

    fn executable(&self, component: &str, role: ManagedRuntimeRole) -> Option<TrustedExecutable> {
        let hold = self.hold(component)?;
        TrustedExecutable::managed_in(&hold, &managed_executable_name(role)?).ok()
    }
}

impl SmokeCompanionSource for StoreCompanions {
    fn companions(&self) -> SmokeCompanions {
        let media = self
            .executable(MEDIA, ManagedRuntimeRole::Ffmpeg)
            .zip(self.executable(MEDIA, ManagedRuntimeRole::Ffprobe))
            .map(|(ffmpeg, ffprobe)| MediaProviderConformance::r0(ffmpeg, ffprobe));
        let model = self.hold(MODEL).and_then(|hold| {
            let name = hold.runtime().reviewed_names().first()?.to_string();
            hold.runtime().file_path(&name)
        });
        SmokeCompanions {
            media,
            whisper: self.executable(WHISPER, ManagedRuntimeRole::WhisperCli),
            model,
        }
    }
}

// ---------------------------------------------------------------------------
// The managed root and one transaction.

/// A fresh temporary parent for one managed root, removed on drop.
struct TestRoot {
    parent: PathBuf,
}

impl TestRoot {
    fn new() -> TestResult<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let parent = env::temp_dir().join(format!(
            "vsift-p13-install-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&parent)?;
        Ok(Self { parent })
    }

    fn root(&self) -> PathBuf {
        self.parent.join("managed")
    }

    fn store(&self) -> TestResult<ManagedArtifactStore> {
        Ok(ManagedArtifactStore::at(self.root())?)
    }

    /// Every `stage-*` directory in the managed root.
    fn stages(&self) -> TestResult<Vec<PathBuf>> {
        let mut stages = Vec::new();
        if !self.root().exists() {
            return Ok(stages);
        }
        for entry in fs::read_dir(self.root())? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with("stage-") {
                stages.push(entry.path());
            }
        }
        Ok(stages)
    }

    /// The version selected for `component`, if any.
    fn selected(&self, component: &str) -> TestResult<Option<String>> {
        Ok(self
            .store()?
            .open_selected_runtime(component)?
            .map(|runtime| runtime.identity().version().to_owned()))
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.parent);
    }
}

/// How one transaction fetches its artifacts.
enum Via {
    Server { base: String, proxy: Option<String> },
    Directory(PathBuf),
}

struct Run<'a> {
    store: ManagedArtifactStore,
    via: Via,
    verifiers: FixedVerifiers,
    cancellation: ProcessCancellation,
    progress: &'a dyn ProgressSink,
}

impl Run<'_> {
    fn new(store: ManagedArtifactStore, via: Via) -> Self {
        Self {
            store,
            via,
            verifiers: FixedVerifiers::passing(),
            cancellation: ProcessCancellation::new(),
            progress: &NoProgress,
        }
    }

    async fn install(self, actions: &[ManagedSetupAction]) -> TestResult<ManagedInstallReport> {
        let (source, authority) = match self.via {
            Via::Server { base, proxy } => (
                ManagedArtifactSource::Publisher,
                ActionAuthority::Loopback {
                    base_url: base,
                    proxy,
                },
            ),
            Via::Directory(directory) => (
                ManagedArtifactSource::Directory(directory),
                ActionAuthority::Loopback {
                    base_url: String::from("http://127.0.0.1:9"),
                    proxy: None,
                },
            ),
        };
        let guard = self.store.try_install_guard()?;
        let installer = ReviewedManagedInstaller::new(
            ManagedInstallerConfig {
                store: self.store.clone(),
                source,
                authority,
                policy: policy()?,
                host_isolation: HostIsolation::ProcessOnly,
                verifiers: self.verifiers,
                companions: StoreCompanions {
                    store: self.store.clone(),
                },
                cancellation: self.cancellation,
            },
            &guard,
            self.progress,
        );
        Ok(install_managed_components(&installer, actions, &NoProgress).await)
    }
}

fn statuses(report: &ManagedInstallReport) -> Vec<&'static str> {
    report
        .components
        .iter()
        .map(|component| component.outcome.identifier())
        .collect()
}

fn failure(report: &ManagedInstallReport, index: usize) -> TestResult<ComponentInstallFailure> {
    match report
        .components
        .get(index)
        .map(|component| component.outcome)
    {
        Some(ComponentInstallOutcome::Failed(failure)) => Ok(failure),
        other => Err(format!("component {index} did not fail: {other:?}").into()),
    }
}

fn download_failure(reason: DownloadFailureReason) -> ComponentInstallFailure {
    ComponentInstallFailure::at(
        InstallStep::Download,
        InstallFailureReason::Download(reason),
    )
}

// ---------------------------------------------------------------------------
// The tests.

/// A fresh install downloads, stages, smokes, publishes and selects all
/// three components; nothing of their stages remains, each selected version
/// opens, and a rerun reports every component `already_current` without
/// another request.
#[tokio::test]
async fn a_fresh_install_activates_every_component_and_a_rerun_fetches_nothing() -> TestResult {
    let fixture = Fixture::new("fixture-media-1")?;
    let server = TestServer::start()?;
    fixture.serve_all(&server);
    let root = TestRoot::new()?;
    let actions = fixture.actions()?;
    let progress = RecordedProgress::default();

    let mut run = Run::new(
        root.store()?,
        Via::Server {
            base: server.base(),
            proxy: None,
        },
    );
    run.progress = &progress;
    let report = run.install(&actions).await?;

    assert_eq!(statuses(&report), ["activated"; 3], "{report:?}");
    assert_eq!(report.failure_code(), None);
    assert!(
        report
            .components
            .iter()
            .all(|component| component.stage == Some(StageDisposal::Discarded))
    );
    assert!(root.stages()?.is_empty());
    assert_eq!(root.selected(MEDIA)?.as_deref(), Some("fixture-media-1"));
    assert_eq!(
        root.selected(WHISPER)?.as_deref(),
        Some("fixture-whisper-1")
    );
    assert_eq!(root.selected(MODEL)?.as_deref(), Some("fixture-model-1"));
    let media = root
        .store()?
        .open_selected_runtime(MEDIA)?
        .ok_or("no media runtime")?;
    let ffmpeg = role_name(ManagedRuntimeRole::Ffmpeg)?;
    let path = media.file_path(&ffmpeg).ok_or("no ffmpeg path")?;
    assert!(path.is_file());
    assert_eq!(media.manifest_sha256().len(), 64);
    drop(media);

    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().all(|request| {
        request.method == "GET"
            && !request.has_header("range")
            && request
                .headers
                .iter()
                .any(|(name, value)| name == "user-agent" && value == "VSift/0.1 managed setup")
    }));
    let fetched: Vec<ProgressUpdate> = lock(&progress.updates)
        .iter()
        .filter(|update| update.stage == ProgressStage::FetchingArtifact)
        .copied()
        .collect();
    assert!(
        fetched
            .iter()
            .any(|update| Some(update.completed) == update.total)
    );

    let rerun = Run::new(
        root.store()?,
        Via::Server {
            base: server.base(),
            proxy: None,
        },
    )
    .install(&actions)
    .await?;
    assert_eq!(statuses(&rerun), ["already_current"; 3]);
    assert_eq!(server.requests().len(), 3, "a rerun fetched again");
    Ok(())
}

#[derive(Default)]
struct RecordedProgress {
    updates: Mutex<Vec<ProgressUpdate>>,
}

impl ProgressSink for RecordedProgress {
    fn report(&self, update: ProgressUpdate) {
        lock(&self.updates).push(update);
    }
}

/// D-03: a connection dropped mid-body discards the stage; the rerun
/// restarts at byte zero (a second full `GET` with no `Range`) and
/// continues from the component that failed.
#[tokio::test]
async fn a_dropped_download_is_discarded_and_a_rerun_restarts_at_zero() -> TestResult {
    let fixture = Fixture::new("fixture-media-1")?;
    let server = TestServer::start()?;
    fixture.serve_all(&server);
    let whisper = fixture.bytes(WHISPER_FILE)?;
    server.route(
        &format!("/{WHISPER_FILE}"),
        vec![
            Reply::DropAfter {
                body: whisper.clone(),
                sent: whisper.len() / 2,
            },
            Reply::Body(whisper.clone()),
        ],
    );
    let root = TestRoot::new()?;
    let actions = fixture.actions()?;
    let via = || Via::Server {
        base: server.base(),
        proxy: None,
    };

    let first = Run::new(root.store()?, via()).install(&actions).await?;
    assert_eq!(statuses(&first), ["activated", "failed", "failed"]);
    assert_eq!(
        failure(&first, 1)?,
        download_failure(DownloadFailureReason::Offline)
    );
    assert_eq!(
        failure(&first, 2)?.reason,
        InstallFailureReason::Blocked,
        "the model is not fetched after the recognizer failed"
    );
    assert_eq!(first.failure_code(), Some(FailureCode::DownloadFailed));
    assert!(root.stages()?.is_empty(), "the partial stage was kept");
    assert_eq!(root.selected(WHISPER)?, None);

    let second = Run::new(root.store()?, via()).install(&actions).await?;
    assert_eq!(
        statuses(&second),
        ["already_current", "activated", "activated"]
    );
    let whisper_requests: Vec<Recorded> = server
        .requests()
        .into_iter()
        .filter(|request| request.target.ends_with(WHISPER_FILE))
        .collect();
    assert_eq!(whisper_requests.len(), 2);
    assert!(
        whisper_requests
            .iter()
            .all(|request| !request.has_header("range"))
    );
    assert!(root.stages()?.is_empty());
    Ok(())
}

/// D-03: a partial `206`, other statuses, altered bytes and a wrong length
/// are refused with their typed reason; nothing is staged or published.
#[tokio::test]
async fn partial_altered_and_wrongly_sized_responses_are_refused() -> TestResult {
    let fixture = Fixture::new("fixture-media-1")?;
    let media = fixture.bytes(MEDIA_FILE)?;
    let mut altered = media.clone();
    if let Some(byte) = altered.last_mut() {
        *byte ^= 0xff;
    }
    let mut longer = media.clone();
    longer.push(0);
    let shorter = media[..media.len() - 1].to_vec();
    for (reply, expected) in [
        (
            Reply::Partial(media.clone()),
            download_failure(DownloadFailureReason::HttpStatus),
        ),
        (
            Reply::Status(404, "Not Found"),
            download_failure(DownloadFailureReason::HttpStatus),
        ),
        (
            Reply::Status(503, "Service Unavailable"),
            download_failure(DownloadFailureReason::HttpStatus),
        ),
        (
            Reply::Body(altered),
            ComponentInstallFailure::at(
                InstallStep::Download,
                InstallFailureReason::DigestMismatch,
            ),
        ),
        (
            Reply::Body(longer),
            download_failure(DownloadFailureReason::Size),
        ),
        (
            Reply::Body(shorter),
            download_failure(DownloadFailureReason::Size),
        ),
    ] {
        let server = TestServer::start()?;
        fixture.serve_all(&server);
        server.route(&format!("/{MEDIA_FILE}"), vec![reply.clone()]);
        let root = TestRoot::new()?;
        let report = Run::new(
            root.store()?,
            Via::Server {
                base: server.base(),
                proxy: None,
            },
        )
        .install(&fixture.actions()?)
        .await?;
        assert_eq!(failure(&report, 0)?, expected, "{reply:?}");
        assert_eq!(statuses(&report), ["failed"; 3]);
        assert!(root.stages()?.is_empty(), "{reply:?}");
        assert!(!root.root().join("versions-v1").exists(), "{reply:?}");
        assert!(
            server
                .requests()
                .iter()
                .all(|request| !request.target.ends_with(WHISPER_FILE))
        );
    }
    Ok(())
}

/// D-05: a newer install selects a new version and leaves the one a job
/// still holds untouched; an executable resolved from a version keeps it
/// from removal for as long as the executable lives.
#[tokio::test]
async fn an_update_never_removes_a_version_an_executable_holds() -> TestResult {
    let first = Fixture::new("fixture-media-1")?;
    let server = TestServer::start()?;
    first.serve_all(&server);
    let root = TestRoot::new()?;
    let via = || Via::Server {
        base: server.base(),
        proxy: None,
    };
    Run::new(root.store()?, via())
        .install(&first.actions()?)
        .await?;
    let store = root.store()?;
    let old_identity = vsift_infrastructure::ManagedRuntimeIdentity::new(MEDIA, "fixture-media-1")?;
    let held = ManagedRuntimeHold::new(store.open_published_runtime(&old_identity)?);
    let executable = TrustedExecutable::managed_in(&held, &role_name(ManagedRuntimeRole::Ffmpeg)?)?;
    drop(held);

    let second = Fixture::new("fixture-media-2")?;
    second.serve_all(&server);
    let report = Run::new(root.store()?, via())
        .install(&second.actions()?)
        .await?;
    assert_eq!(
        statuses(&report),
        ["activated", "already_current", "already_current"]
    );
    assert_eq!(root.selected(MEDIA)?.as_deref(), Some("fixture-media-2"));
    assert!(executable.path().is_file());

    let guard = store.try_install_guard()?;
    assert_eq!(
        store.remove_published_runtime(&guard, &old_identity)?,
        vsift_infrastructure::ManagedVersionRemovalOutcome::InUse
    );
    drop(executable);
    assert_eq!(
        store.remove_published_runtime(&guard, &old_identity)?,
        vsift_infrastructure::ManagedVersionRemovalOutcome::Removed
    );
    Ok(())
}

/// D-03: a disk that fills while a newer version is staged fails with a
/// storage reason; the version selected before stays selected and opens.
#[tokio::test]
async fn a_full_disk_keeps_the_previous_version_selected_and_usable() -> TestResult {
    let first = Fixture::new("fixture-media-1")?;
    let server = TestServer::start()?;
    first.serve_all(&server);
    let root = TestRoot::new()?;
    let base = || Via::Server {
        base: server.base(),
        proxy: None,
    };
    let installed = Run::new(root.store()?, base())
        .install(&first.actions()?)
        .await?;
    assert_eq!(statuses(&installed), ["activated"; 3]);

    let second = Fixture::new("fixture-media-2")?;
    second.serve_all(&server);
    let full = root.store()?.with_stage_write_failure_after(1024);
    let report = Run::new(full, base()).install(&second.actions()?).await?;
    assert_eq!(
        failure(&report, 0)?,
        ComponentInstallFailure::at(InstallStep::Download, InstallFailureReason::Storage)
    );
    assert_eq!(report.failure_code(), Some(FailureCode::StorageIo));
    assert_eq!(
        statuses(&report),
        ["failed", "already_current", "already_current"]
    );
    assert!(root.stages()?.is_empty());
    let media = root
        .store()?
        .open_selected_runtime(MEDIA)?
        .ok_or("the previous version is no longer selected")?;
    assert_eq!(media.identity().version(), "fixture-media-1");
    media.open_reviewed_file(&role_name(ManagedRuntimeRole::Ffmpeg)?)?;
    Ok(())
}

/// D-03: cancelling during a download stops it, discards the stage and
/// reports the component cancelled and the rest blocked.
#[tokio::test]
async fn cancellation_during_a_download_discards_the_stage() -> TestResult {
    let fixture = Fixture::new("fixture-media-1")?;
    let server = TestServer::start()?;
    fixture.serve_all(&server);
    let media = fixture.bytes(MEDIA_FILE)?;
    server.route(
        &format!("/{MEDIA_FILE}"),
        vec![Reply::StallAfter {
            body: media.clone(),
            sent: media.len() / 2,
        }],
    );
    let root = TestRoot::new()?;
    let cancellation = ProcessCancellation::new();
    let canceller = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        canceller.cancel();
    });
    let mut run = Run::new(
        root.store()?,
        Via::Server {
            base: server.base(),
            proxy: None,
        },
    );
    run.cancellation = cancellation;
    let report = run.install(&fixture.actions()?).await?;
    assert_eq!(
        failure(&report, 0)?,
        ComponentInstallFailure::at(InstallStep::Download, InstallFailureReason::Cancelled)
    );
    assert_eq!(report.failure_code(), Some(FailureCode::Cancelled));
    assert_eq!(failure(&report, 1)?.reason, InstallFailureReason::Blocked);
    assert!(root.stages()?.is_empty());
    assert!(!root.root().join("versions-v1").exists());
    Ok(())
}

/// D-06: a smoke that fails keeps the component unpublished and its stage
/// removed, with the smoke's step and reason in the report.
#[tokio::test]
async fn a_failed_smoke_publishes_nothing_and_removes_the_stage() -> TestResult {
    let fixture = Fixture::new("fixture-media-1")?;
    let server = TestServer::start()?;
    fixture.serve_all(&server);
    let root = TestRoot::new()?;
    let mut run = Run::new(
        root.store()?,
        Via::Server {
            base: server.base(),
            proxy: None,
        },
    );
    run.verifiers.media = MediaToolVerification::Failed {
        check: MediaToolCheck::Probe,
        failure: MediaToolFailure::UnexpectedResult,
    };
    let report = run.install(&fixture.actions()?).await?;
    assert_eq!(
        failure(&report, 0)?,
        ComponentInstallFailure::at(
            InstallStep::Smoke,
            InstallFailureReason::Smoke(CompatibilitySmokeFailure {
                check: CompatibilitySmokeCheck::MediaFixture,
                reason: CompatibilitySmokeFailureReason::FixtureMismatch,
            })
        )
    );
    assert_eq!(report.failure_code(), Some(FailureCode::MissingCapability));
    assert_eq!(report.components[0].stage, Some(StageDisposal::Discarded));
    assert!(root.stages()?.is_empty());
    assert!(!root.root().join("versions-v1").exists());
    Ok(())
}

/// D-07: a certificate no trust store accepts fails with the typed `tls`
/// reason before any byte is staged.
#[tokio::test]
async fn an_untrusted_certificate_fails_with_the_tls_reason() -> TestResult {
    let fixture = Fixture::new("fixture-media-1")?;
    let port = start_untrusted_tls_server()?;
    let root = TestRoot::new()?;
    let report = Run::new(
        root.store()?,
        Via::Server {
            base: format!("https://127.0.0.1:{port}"),
            proxy: None,
        },
    )
    .install(&fixture.actions()?)
    .await?;
    assert_eq!(
        failure(&report, 0)?,
        download_failure(DownloadFailureReason::Tls)
    );
    assert_eq!(report.failure_code(), Some(FailureCode::DownloadFailed));
    assert!(root.stages()?.is_empty());
    Ok(())
}

/// D-07: a proxy that answers `407` fails with `proxy_auth`, for a plain
/// request and for an HTTPS tunnel, and the proxy credentials appear
/// nowhere in the report.
#[tokio::test]
async fn a_proxy_demanding_authentication_fails_without_leaking_credentials() -> TestResult {
    let fixture = Fixture::new("fixture-media-1")?;
    let proxy = TestServer::answering(Reply::ProxyAuth)?;
    let proxy_url = format!("http://vsift-user:{SENTINEL}@127.0.0.1:{}", proxy.port);
    for base in ["http://127.0.0.1:9", "https://127.0.0.1:9"] {
        let root = TestRoot::new()?;
        let report = Run::new(
            root.store()?,
            Via::Server {
                base: base.to_owned(),
                proxy: Some(proxy_url.clone()),
            },
        )
        .install(&fixture.actions()?)
        .await?;
        assert_eq!(
            failure(&report, 0)?,
            download_failure(DownloadFailureReason::ProxyAuth),
            "{base}"
        );
        assert!(!format!("{report:?}").contains(SENTINEL));
        assert!(root.stages()?.is_empty());
    }
    let requests = proxy.requests();
    assert!(requests.iter().any(|request| request.method == "CONNECT"));
    assert!(requests.iter().any(|request| request.method == "GET"));
    assert!(
        requests
            .iter()
            .all(|request| request.has_header("proxy-authorization"))
    );
    Ok(())
}

/// D-07: a redirect to another host or carrying credentials is refused
/// with `redirect_policy`; a redirect within the reviewed route is followed.
#[tokio::test]
async fn redirects_leave_only_the_reviewed_route() -> TestResult {
    let fixture = Fixture::new("fixture-media-1")?;
    for (target, expected) in [
        (
            String::from("http://localhost:{port}/moved"),
            Some(download_failure(DownloadFailureReason::RedirectPolicy)),
        ),
        (
            String::from("http://vsift-user:secret@127.0.0.1:{port}/moved"),
            Some(download_failure(DownloadFailureReason::RedirectPolicy)),
        ),
        (
            String::from("https://127.0.0.1:{port}/moved"),
            Some(download_failure(DownloadFailureReason::RedirectPolicy)),
        ),
        (String::from("http://127.0.0.1:{port}/moved"), None),
    ] {
        let server = TestServer::start()?;
        fixture.serve_all(&server);
        let location = target.replace("{port}", &server.port.to_string());
        server.route(&format!("/{MEDIA_FILE}"), vec![Reply::Redirect(location)]);
        server.route("/moved", vec![Reply::Body(fixture.bytes(MEDIA_FILE)?)]);
        let root = TestRoot::new()?;
        let report = Run::new(
            root.store()?,
            Via::Server {
                base: server.base(),
                proxy: None,
            },
        )
        .install(&fixture.actions()?)
        .await?;
        match expected {
            Some(expected) => {
                assert_eq!(failure(&report, 0)?, expected, "{target}");
                assert!(root.stages()?.is_empty());
            }
            None => assert_eq!(statuses(&report), ["activated"; 3], "{target}"),
        }
    }
    Ok(())
}

/// D-07: an offline `--artifact-dir` import applies the same verification
/// with no request at all: the good files install, a missing, altered or
/// resized file is refused with its reason.
#[tokio::test]
async fn an_offline_import_is_verified_like_a_download() -> TestResult {
    let fixture = Fixture::new("fixture-media-1")?;
    let folder = TestRoot::new()?;
    let write_all = |replace: Option<(&str, Option<Vec<u8>>)>| -> TestResult {
        for (name, bytes) in &fixture.files {
            let path = folder.parent.join(name);
            let _ = fs::remove_file(&path);
            match &replace {
                Some((replaced, None)) if replaced == name => {}
                Some((replaced, Some(other))) if replaced == name => fs::write(path, other)?,
                _ => fs::write(path, bytes)?,
            }
        }
        Ok(())
    };
    let media = fixture.bytes(MEDIA_FILE)?;
    let mut altered = media.clone();
    if let Some(byte) = altered.first_mut() {
        *byte ^= 0xff;
    }
    let mut resized = media.clone();
    resized.push(0);
    for (replace, expected) in [
        (
            Some((MEDIA_FILE, None)),
            Some(InstallFailureReason::ArtifactMissing),
        ),
        (
            Some((MEDIA_FILE, Some(altered))),
            Some(InstallFailureReason::DigestMismatch),
        ),
        (
            Some((MEDIA_FILE, Some(resized))),
            Some(InstallFailureReason::SizeMismatch),
        ),
        (None, None),
    ] {
        write_all(replace)?;
        let root = TestRoot::new()?;
        let report = Run::new(root.store()?, Via::Directory(folder.parent.clone()))
            .install(&fixture.actions()?)
            .await?;
        if let Some(reason) = expected {
            assert_eq!(
                failure(&report, 0)?,
                ComponentInstallFailure::at(InstallStep::Import, reason)
            );
            assert!(root.stages()?.is_empty());
        } else {
            assert_eq!(statuses(&report), ["activated"; 3], "{report:?}");
            assert_eq!(root.selected(MODEL)?.as_deref(), Some("fixture-model-1"));
        }
    }
    Ok(())
}

/// D-07: an import follows no link, even to the reviewed bytes.
#[cfg(unix)]
#[tokio::test]
async fn an_offline_import_refuses_a_link() -> TestResult {
    let fixture = Fixture::new("fixture-media-1")?;
    let folder = TestRoot::new()?;
    let elsewhere = TestRoot::new()?;
    for (name, bytes) in &fixture.files {
        fs::write(elsewhere.parent.join(name), bytes)?;
        std::os::unix::fs::symlink(elsewhere.parent.join(name), folder.parent.join(name))?;
    }
    let root = TestRoot::new()?;
    let report = Run::new(root.store()?, Via::Directory(folder.parent.clone()))
        .install(&fixture.actions()?)
        .await?;
    assert_eq!(
        failure(&report, 0)?,
        ComponentInstallFailure::at(
            InstallStep::Import,
            InstallFailureReason::ArtifactNotRegularFile
        )
    );
    Ok(())
}
