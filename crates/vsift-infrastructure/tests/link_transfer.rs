//! ADR 0028 (#407): fetching a direct HTTPS link into a session, against a
//! local HTTPS server signed by a test root, through the development-only link
//! test route (`install-test-hooks`). No test reaches the network: names are
//! answered by the route's test resolver and every connection goes to
//! `127.0.0.1`.
//!
//! Every refusal leaves no file in the session's artifact folder, and no
//! answer, error or stored record holds the link's token.

use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{IpAddr, Ipv4Addr, Shutdown, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use native_tls::{Identity, TlsAcceptor};
use vsift_application::{
    InitializeSessionStorage, InitializeSessionStorageRequest, NeverCancelled, StageCancellation,
};
use vsift_domain::{
    AddressCheck, DownloadLimits, DurabilityRequirement, OperationId, SessionId, SourceFetchReason,
    SourceLink, SourceOrigin, StorageGeneration,
};
use vsift_infrastructure::{
    FilesystemSessionStore, LinkTestRoute, SourceError, SourceSnapshot, TestAnswers,
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const PREFIX: &str = "vsift-link-transfer-test-";
/// The token every link carries; it must never appear anywhere `VSift` writes.
const SENTINEL: &str = "SENTINEL-TOKEN-7f3a9c";
const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tls")
        .join(name)
}

/// A small MP4-shaped body: an `ftyp` box, then filler.
fn video_body(length: usize) -> Vec<u8> {
    let mut body = b"\0\0\0\x18ftypisom\0\0\x02\0isomiso2".to_vec();
    body.resize(length.max(body.len()), 0x5a);
    body
}

/// How the test server answers one path.
#[derive(Clone)]
enum Reply {
    /// `200 OK` with this body and content type, with or without a length.
    Body {
        body: Vec<u8>,
        content_type: &'static str,
        with_length: bool,
        extra: &'static str,
    },
    /// A status line and headers with no body.
    Status(&'static str, String),
    /// Announces `announced` bytes, sends `sent`, then closes.
    CutShort { announced: usize, sent: usize },
    /// Sends the headers, then one byte every `pause`.
    Drip { pause: Duration, bytes: usize },
}

#[derive(Default)]
struct ServerState {
    routes: BTreeMap<String, Reply>,
    requests: Vec<String>,
}

struct HttpsServer {
    port: u16,
    state: Arc<Mutex<ServerState>>,
}

impl HttpsServer {
    fn start(certificate: &str, key: &str) -> TestResult<Self> {
        let identity =
            Identity::from_pkcs8(&fs::read(fixture(certificate))?, &fs::read(fixture(key))?)?;
        let acceptor = Arc::new(TlsAcceptor::new(identity)?);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        let state = Arc::new(Mutex::new(ServerState::default()));
        let shared = Arc::clone(&state);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let acceptor = Arc::clone(&acceptor);
                let state = Arc::clone(&shared);
                thread::spawn(move || {
                    if let Ok(tls) = acceptor.accept(stream) {
                        let _ = serve(tls, &state);
                    }
                });
            }
        });
        Ok(Self { port, state })
    }

    fn trusted() -> TestResult<Self> {
        Self::start("link-test-server.cert.pem", "link-test-server.key.pem")
    }

    fn route(&self, path: &str, reply: Reply) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .routes
            .insert(path.to_owned(), reply);
    }

    fn requests(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .clone()
    }

    fn link(&self, host: &str, path: &str) -> TestResult<SourceLink> {
        Ok(SourceLink::parse(&format!(
            "https://{host}:{}{path}?token={SENTINEL}#t=1",
            self.port
        ))?)
    }
}

fn serve(
    mut stream: native_tls::TlsStream<TcpStream>,
    state: &Mutex<ServerState>,
) -> std::io::Result<()> {
    let mut head = Vec::new();
    {
        let mut reader = BufReader::new(&mut stream);
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 || line == "\r\n" {
                break;
            }
            head.push(line);
        }
    }
    let request = head.concat();
    let path = head
        .first()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|target| target.split('?').next())
        .unwrap_or_default()
        .to_owned();
    let reply = {
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        state.requests.push(request);
        state.routes.get(&path).cloned()
    };
    match reply {
        Some(Reply::Body {
            body,
            content_type,
            with_length,
            extra,
        }) => {
            let length = if with_length {
                format!("Content-Length: {}\r\n", body.len())
            } else {
                String::new()
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n{length}{extra}Connection: close\r\n\r\n"
            )?;
            stream.write_all(&body)?;
        }
        Some(Reply::Status(status, headers)) => {
            write!(
                stream,
                "HTTP/1.1 {status}\r\n{headers}Content-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
        }
        Some(Reply::CutShort { announced, sent }) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {announced}\r\nConnection: close\r\n\r\n"
            )?;
            stream.write_all(&video_body(sent)[..sent])?;
        }
        Some(Reply::Drip { pause, bytes }) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {bytes}\r\nConnection: close\r\n\r\n"
            )?;
            for byte in video_body(bytes) {
                stream.write_all(&[byte])?;
                stream.flush()?;
                thread::sleep(pause);
            }
        }
        None => {
            write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
        }
    }
    stream.flush()?;
    let _ = stream.shutdown();
    Ok(())
}

/// A plain-HTTP proxy on `127.0.0.1` that tunnels every `CONNECT` to the
/// test server, or answers it with `407`.
fn start_proxy(target_port: u16, demand_credentials: bool) -> TestResult<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    thread::spawn(move || {
        for client in listener.incoming().flatten() {
            thread::spawn(move || {
                let _ = tunnel(client, target_port, demand_credentials);
            });
        }
    });
    Ok(port)
}

fn tunnel(
    mut client: TcpStream,
    target_port: u16,
    demand_credentials: bool,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(client.try_clone()?);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" {
            break;
        }
    }
    if demand_credentials {
        client.write_all(
            b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"test\"\r\nContent-Length: 0\r\n\r\n",
        )?;
        return Ok(());
    }
    let mut server = TcpStream::connect((Ipv4Addr::LOCALHOST, target_port))?;
    client.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")?;
    let mut client_read = client.try_clone()?;
    let mut server_write = server.try_clone()?;
    let upstream = thread::spawn(move || {
        let _ = std::io::copy(&mut client_read, &mut server_write);
        let _ = server_write.shutdown(Shutdown::Write);
    });
    let _ = std::io::copy(&mut server, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    let _ = upstream.join();
    Ok(())
}

struct OwnedRoot(PathBuf);

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

impl OwnedRoot {
    fn new() -> TestResult<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path =
            env::temp_dir().join(format!("{PREFIX}{}-{stamp}-{sequence}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
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

const SESSION: &str = "ses_0123456789abcdef";

/// An initialised, unactivated session in a fresh root.
async fn session(root: &OwnedRoot) -> TestResult<FilesystemSessionStore> {
    let workspace = root.0.join("workspace");
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

fn route(server: &HttpsServer, answers: &Arc<TestAnswers>) -> TestResult<LinkTestRoute> {
    Ok(LinkTestRoute::new(
        server.port,
        fs::read(fixture("link-test-ca.cert.pem"))?,
        Arc::clone(answers),
    ))
}

fn answers() -> Arc<TestAnswers> {
    let answers = Arc::new(TestAnswers::default());
    answers.answer("video.example.test", vec![vec![LOOPBACK]]);
    answers.answer("cdn.example.test", vec![vec![LOOPBACK]]);
    answers
}

async fn fetch(
    store: &FilesystemSessionStore,
    link: &SourceLink,
    limits: DownloadLimits,
    route: LinkTestRoute,
) -> Result<SourceSnapshot, SourceError> {
    fetch_cancellable(store, link, limits, route, &NeverCancelled).await
}

async fn fetch_cancellable(
    store: &FilesystemSessionStore,
    link: &SourceLink,
    limits: DownloadLimits,
    route: LinkTestRoute,
    cancellation: &dyn StageCancellation,
) -> Result<SourceSnapshot, SourceError> {
    let session_id = SessionId::parse(SESSION).map_err(|_| SourceError::IdentityFailure)?;
    let operation =
        OperationId::parse("op_1111111111111111").map_err(|_| SourceError::IdentityFailure)?;
    SourceSnapshot::stage_link_for_tests(
        store,
        &session_id,
        &operation,
        (link, limits),
        route,
        cancellation,
    )
    .await
}

/// Every file under `directory`, recursively.
fn files_under(directory: &Path) -> TestResult<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else {
                found.push(entry.path());
            }
        }
    }
    Ok(found)
}

fn source_files(root: &OwnedRoot) -> TestResult<Vec<PathBuf>> {
    Ok(files_under(&root.0.join("workspace"))?
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("source-"))
        })
        .collect())
}

/// Fetches `path` from the trusted server and expects `reason`, no source
/// file and no token in the error.
async fn refused(
    server: &HttpsServer,
    host: &str,
    path: &str,
    limits: DownloadLimits,
    answers: &Arc<TestAnswers>,
    reason: SourceFetchReason,
) -> TestResult {
    let root = OwnedRoot::new()?;
    let store = session(&root).await?;
    let result = fetch(
        &store,
        &server.link(host, path)?,
        limits,
        route(server, answers)?,
    )
    .await;
    let Err(error) = result else {
        return Err(format!("{path} was accepted").into());
    };
    assert!(
        matches!(error, SourceError::Fetch(found) if found == reason),
        "{path}: {error:?}, expected {reason:?}"
    );
    assert!(!format!("{error} {error:?}").contains(SENTINEL));
    assert_eq!(
        source_files(&root)?,
        Vec::<PathBuf>::new(),
        "{path} left a file"
    );
    Ok(())
}

fn body(body: Vec<u8>, with_length: bool) -> Reply {
    Reply::Body {
        body,
        content_type: "video/mp4",
        with_length,
        extra: "",
    }
}

#[tokio::test]
async fn a_video_is_fetched_with_or_without_a_length_and_its_origin_recorded() -> TestResult {
    let server = HttpsServer::trusted()?;
    server.route("/video.mp4", body(video_body(200_000), true));
    server.route("/streamed.mp4", body(video_body(150_000), false));
    let answers = answers();
    for (path, bytes) in [("/video.mp4", 200_000_u64), ("/streamed.mp4", 150_000)] {
        let root = OwnedRoot::new()?;
        let store = session(&root).await?;
        let snapshot = fetch(
            &store,
            &server.link("video.example.test", path)?,
            DownloadLimits::DEFAULT,
            route(&server, &answers)?,
        )
        .await?;
        assert_eq!(snapshot.bytes(), bytes);
        let Some(SourceOrigin::DirectHttps(origin)) = snapshot.origin() else {
            return Err("no origin".into());
        };
        assert_eq!(origin.requested_host().as_str(), "video.example.test");
        assert_eq!(origin.final_host().as_str(), "video.example.test");
        assert_eq!(origin.redirects(), 0);
        assert_eq!(origin.address_check(), AddressCheck::Direct);
        assert_eq!(origin.bytes(), bytes);
        // The session binds the fetched bytes: activate it and read it back.
        let generation = store.activate_source(
            &snapshot,
            &OperationId::parse("op_2222222222222222")?,
            StorageGeneration::INITIAL,
            1_000,
        )?;
        drop(snapshot);
        let status = store.session_status(&SessionId::parse(SESSION)?)?;
        assert_eq!(status.generation(), generation);
        assert_eq!(status.source_bytes(), bytes);
        let Some(SourceOrigin::DirectHttps(stored)) = status.origin() else {
            return Err("the origin was not stored".into());
        };
        assert_eq!(stored.bytes(), bytes);
        // Nothing VSift wrote holds the link's token, its path or its query.
        for file in files_under(&root.0)? {
            let text = String::from_utf8_lossy(&fs::read(&file)?).into_owned();
            assert!(
                !text.contains(SENTINEL),
                "{} holds the token",
                file.display()
            );
            assert!(
                !text.contains(path.trim_start_matches('/'))
                    || file.to_string_lossy().contains("source-"),
                "{} holds the link's path",
                file.display()
            );
        }
    }
    let requests = server.requests();
    assert!(
        requests
            .iter()
            .all(|request| request.contains("Accept-Encoding: identity")
                || request.contains("accept-encoding: identity"))
    );
    for request in &requests {
        let lower = request.to_ascii_lowercase();
        assert!(!lower.contains("cookie:"), "{request}");
        assert!(!lower.contains("authorization:"), "{request}");
        assert!(!lower.contains("referer:"), "{request}");
        assert!(lower.contains("user-agent: vsift/"), "{request}");
    }
    Ok(())
}

#[tokio::test]
async fn redirects_are_followed_three_times_on_https_only() -> TestResult {
    let server = HttpsServer::trusted()?;
    let port = server.port;
    server.route("/video.mp4", body(video_body(64_000), true));
    server.route(
        "/moved",
        Reply::Status(
            "302 Found",
            format!("Location: https://cdn.example.test:{port}/video.mp4\r\n"),
        ),
    );
    server.route(
        "/loop",
        Reply::Status(
            "302 Found",
            format!("Location: https://cdn.example.test:{port}/loop\r\n"),
        ),
    );
    server.route(
        "/to-http",
        Reply::Status(
            "302 Found",
            format!("Location: http://cdn.example.test:{port}/video.mp4\r\n"),
        ),
    );
    server.route(
        "/to-private",
        Reply::Status(
            "302 Found",
            format!("Location: https://private.example.test:{port}/video.mp4\r\n"),
        ),
    );
    server.route(
        "/to-metadata",
        Reply::Status(
            "302 Found",
            "Location: https://169.254.169.254/latest/meta-data/\r\n".to_owned(),
        ),
    );
    server.route(
        "/to-other-port",
        Reply::Status(
            "302 Found",
            "Location: https://cdn.example.test:8443/video.mp4\r\n".to_owned(),
        ),
    );
    let answers = answers();
    answers.answer(
        "private.example.test",
        vec![vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))]],
    );

    let root = OwnedRoot::new()?;
    let store = session(&root).await?;
    let snapshot = fetch(
        &store,
        &server.link("video.example.test", "/moved")?,
        DownloadLimits::DEFAULT,
        route(&server, &answers)?,
    )
    .await?;
    let Some(SourceOrigin::DirectHttps(origin)) = snapshot.origin() else {
        return Err("no origin".into());
    };
    assert_eq!(origin.final_host().as_str(), "cdn.example.test");
    assert_eq!(origin.redirects(), 1);

    for (path, reason) in [
        ("/loop", SourceFetchReason::RedirectPolicy),
        ("/to-http", SourceFetchReason::RedirectPolicy),
        ("/to-private", SourceFetchReason::AddressRefused),
        ("/to-metadata", SourceFetchReason::AddressRefused),
        ("/to-other-port", SourceFetchReason::PortNotAllowed),
    ] {
        refused(
            &server,
            "video.example.test",
            path,
            DownloadLimits::DEFAULT,
            &answers,
            reason,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test]
async fn names_that_resolve_to_internal_addresses_are_refused_where_the_connection_is_made()
-> TestResult {
    let server = HttpsServer::trusted()?;
    server.route("/video.mp4", body(video_body(64_000), true));
    let table = answers();
    let private = IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3));
    for (name, addresses) in [
        ("private.example.test", vec![private]),
        ("cdn.example.test", vec![LOOPBACK, private]),
        ("video.example.test", vec!["169.254.169.254".parse()?]),
        ("video.example.test", vec!["::ffff:127.0.0.1".parse()?]),
        ("video.example.test", vec!["fd00:ec2::254".parse()?]),
    ] {
        table.answer(name, vec![addresses]);
        refused(
            &server,
            name,
            "/video.mp4",
            DownloadLimits::DEFAULT,
            &table,
            SourceFetchReason::AddressRefused,
        )
        .await?;
    }
    // Rebinding: the name answers the allowed address first and a private one
    // next; every connection is checked, so the second fetch is refused.
    let rebinding = answers();
    rebinding.answer("video.example.test", vec![vec![LOOPBACK], vec![private]]);
    let root = OwnedRoot::new()?;
    let store = session(&root).await?;
    fetch(
        &store,
        &server.link("video.example.test", "/video.mp4")?,
        DownloadLimits::DEFAULT,
        route(&server, &rebinding)?,
    )
    .await?;
    refused(
        &server,
        "video.example.test",
        "/video.mp4",
        DownloadLimits::DEFAULT,
        &rebinding,
        SourceFetchReason::AddressRefused,
    )
    .await?;
    Ok(())
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one table of every refused body, each with its route"
)]
async fn bodies_that_are_not_one_whole_identity_video_are_refused() -> TestResult {
    let server = HttpsServer::trusted()?;
    let answers = answers();
    server.route(
        "/partial",
        Reply::Status(
            "206 Partial Content",
            "Content-Range: bytes 0-0/10\r\n".to_owned(),
        ),
    );
    server.route("/missing", Reply::Status("404 Not Found", String::new()));
    server.route(
        "/gzip",
        Reply::Body {
            body: video_body(1_000),
            content_type: "video/mp4",
            with_length: true,
            extra: "Content-Encoding: gzip\r\n",
        },
    );
    server.route(
        "/page",
        Reply::Body {
            body: b"<!DOCTYPE html><html></html>".to_vec(),
            content_type: "text/html; charset=utf-8",
            with_length: true,
            extra: "",
        },
    );
    server.route(
        "/page-as-octets",
        Reply::Body {
            body: b"<!DOCTYPE html><html></html>".to_vec(),
            content_type: "application/octet-stream",
            with_length: true,
            extra: "",
        },
    );
    let mut avi = b"RIFF\0\0\0\0AVI LIST".to_vec();
    avi.resize(4_096, 0);
    server.route("/avi", body(avi, true));
    server.route("/tiny", body(b"\0\0\0\x18ftyp".to_vec(), true));
    server.route(
        "/cut",
        Reply::CutShort {
            announced: 100_000,
            sent: 1_000,
        },
    );
    server.route("/big", body(video_body(9_000), true));
    server.route("/big-unannounced", body(video_body(9_000), false));
    let small = DownloadLimits::new(Some(8_192), None)?;
    for (path, limits, reason) in [
        (
            "/partial",
            DownloadLimits::DEFAULT,
            SourceFetchReason::Partial,
        ),
        (
            "/missing",
            DownloadLimits::DEFAULT,
            SourceFetchReason::HttpStatus,
        ),
        (
            "/gzip",
            DownloadLimits::DEFAULT,
            SourceFetchReason::Compressed,
        ),
        ("/page", DownloadLimits::DEFAULT, SourceFetchReason::WebPage),
        (
            "/page-as-octets",
            DownloadLimits::DEFAULT,
            SourceFetchReason::WebPage,
        ),
        (
            "/avi",
            DownloadLimits::DEFAULT,
            SourceFetchReason::NotAVideo,
        ),
        (
            "/tiny",
            DownloadLimits::DEFAULT,
            SourceFetchReason::NotAVideo,
        ),
        (
            "/cut",
            DownloadLimits::DEFAULT,
            SourceFetchReason::Truncated,
        ),
        ("/big", small, SourceFetchReason::Size),
        ("/big-unannounced", small, SourceFetchReason::Size),
    ] {
        refused(
            &server,
            "video.example.test",
            path,
            limits,
            &answers,
            reason,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test]
async fn a_slow_transfer_is_stopped_by_its_deadline_or_a_stall() -> TestResult {
    let server = HttpsServer::trusted()?;
    let answers = answers();
    server.route(
        "/drip",
        Reply::Drip {
            pause: Duration::from_millis(100),
            bytes: 400,
        },
    );
    server.route(
        "/stall",
        Reply::Drip {
            pause: Duration::from_secs(3),
            bytes: 400,
        },
    );
    refused(
        &server,
        "video.example.test",
        "/drip",
        DownloadLimits::new(None, Some(1))?,
        &answers,
        SourceFetchReason::Deadline,
    )
    .await?;
    let root = OwnedRoot::new()?;
    let store = session(&root).await?;
    let result = fetch(
        &store,
        &server.link("video.example.test", "/stall")?,
        DownloadLimits::DEFAULT,
        route(&server, &answers)?.with_stall(Duration::from_millis(500)),
    )
    .await;
    assert!(
        matches!(result, Err(SourceError::Fetch(SourceFetchReason::Stalled))),
        "{:?}",
        result.err()
    );
    assert_eq!(source_files(&root)?, Vec::<PathBuf>::new());
    Ok(())
}

struct CancelledNow;

impl StageCancellation for CancelledNow {
    fn is_cancelled(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn a_cancelled_fetch_leaves_nothing() -> TestResult {
    let server = HttpsServer::trusted()?;
    server.route("/video.mp4", body(video_body(64_000), true));
    let root = OwnedRoot::new()?;
    let store = session(&root).await?;
    let result = fetch_cancellable(
        &store,
        &server.link("video.example.test", "/video.mp4")?,
        DownloadLimits::DEFAULT,
        route(&server, &answers())?,
        &CancelledNow,
    )
    .await;
    assert!(
        matches!(result, Err(SourceError::Cancelled)),
        "{:?}",
        result.err()
    );
    assert_eq!(source_files(&root)?, Vec::<PathBuf>::new());
    Ok(())
}

#[tokio::test]
async fn an_untrusted_certificate_is_a_tls_failure() -> TestResult {
    let server = HttpsServer::start("untrusted-loopback.cert.pem", "untrusted-loopback.key.pem")?;
    server.route("/video.mp4", body(video_body(64_000), true));
    refused(
        &server,
        "video.example.test",
        "/video.mp4",
        DownloadLimits::DEFAULT,
        &answers(),
        SourceFetchReason::Tls,
    )
    .await
}

#[tokio::test]
async fn through_a_proxy_the_origin_says_the_proxy_resolved_the_name() -> TestResult {
    let server = HttpsServer::trusted()?;
    server.route("/video.mp4", body(video_body(64_000), true));
    let answers = answers();
    let tunnel = start_proxy(server.port, false)?;
    let root = OwnedRoot::new()?;
    let store = session(&root).await?;
    let snapshot = fetch(
        &store,
        &server.link("video.example.test", "/video.mp4")?,
        DownloadLimits::DEFAULT,
        route(&server, &answers)?.through_proxy(&format!("http://127.0.0.1:{tunnel}")),
    )
    .await?;
    let Some(SourceOrigin::DirectHttps(origin)) = snapshot.origin() else {
        return Err("no origin".into());
    };
    assert_eq!(origin.address_check(), AddressCheck::ProxyResolved);
    drop(snapshot);

    let demanding = start_proxy(server.port, true)?;
    let root = OwnedRoot::new()?;
    let store = session(&root).await?;
    let result = fetch(
        &store,
        &server.link("video.example.test", "/video.mp4")?,
        DownloadLimits::DEFAULT,
        route(&server, &answers)?.through_proxy(&format!("http://127.0.0.1:{demanding}")),
    )
    .await;
    assert!(
        matches!(
            result,
            Err(SourceError::Fetch(SourceFetchReason::ProxyAuth))
        ),
        "{:?}",
        result.err()
    );

    // Through a proxy VSift cannot see the address, so a name for an
    // internal host is refused before anything is sent.
    let link = SourceLink::parse(&format!("https://files.internal:{}/video.mp4", server.port))?;
    let root = OwnedRoot::new()?;
    let store = session(&root).await?;
    let result = fetch(
        &store,
        &link,
        DownloadLimits::DEFAULT,
        route(&server, &answers)?.through_proxy(&format!("http://127.0.0.1:{tunnel}")),
    )
    .await;
    assert!(
        matches!(
            result,
            Err(SourceError::Fetch(SourceFetchReason::AddressRefused))
        ),
        "{:?}",
        result.err()
    );
    Ok(())
}

#[tokio::test]
async fn links_with_credentials_another_port_or_an_internal_address_are_refused_before_any_request()
-> TestResult {
    let server = HttpsServer::trusted()?;
    let answers = answers();
    for (link, reason) in [
        (
            format!(
                "https://user:{SENTINEL}@video.example.test:{}/a.mp4",
                server.port
            ),
            SourceFetchReason::CredentialsInLink,
        ),
        (
            "https://video.example.test:8443/a.mp4".to_owned(),
            SourceFetchReason::PortNotAllowed,
        ),
        (
            "https://10.0.0.1/a.mp4".to_owned(),
            SourceFetchReason::AddressRefused,
        ),
        (
            "https://[fe80::1]/a.mp4".to_owned(),
            SourceFetchReason::AddressRefused,
        ),
    ] {
        let root = OwnedRoot::new()?;
        let store = session(&root).await?;
        let result = fetch(
            &store,
            &SourceLink::parse(&link)?,
            DownloadLimits::DEFAULT,
            route(&server, &answers)?,
        )
        .await;
        // The case is named by its expected reason, never by the link: one
        // link carries a credential, and a test log is a log too.
        assert!(
            matches!(result, Err(SourceError::Fetch(found)) if found == reason),
            "expected {reason:?}"
        );
    }
    assert_eq!(
        server.requests(),
        Vec::<String>::new(),
        "a refused link was sent"
    );
    Ok(())
}

#[test]
fn the_server_reads_its_fixtures() -> TestResult {
    let mut text = String::new();
    fs::File::open(fixture("link-test-ca.cert.pem"))?.read_to_string(&mut text)?;
    assert!(text.starts_with("-----BEGIN CERTIFICATE-----"));
    Ok(())
}
