//! Direct HTTPS transfer of reviewed publisher bytes into unactivated owned staging.
//!
//! One `GET` per attempt, with no `Range` header: there is deliberately no
//! resume (ADR 0007). An interrupted transfer discards its stage and the next
//! attempt restarts at byte zero; only a complete `200 OK` body is accepted,
//! so a partial `206` is refused. Every byte is checked against the reviewed
//! size and SHA-256 as it arrives, and a failure carries a typed reason
//! (ADR 0023 decision H3), never the server's text, a signed CDN URL or proxy
//! credentials.

use std::{error::Error, fmt, time::Duration};

use reqwest::{
    Client, StatusCode, Url,
    header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_RANGE},
    redirect::Policy,
};
use tokio::time::{Instant, timeout_at};
use vsift_application::{DownloadFailureReason, ProgressSink};
use vsift_domain::{ArtifactIntegrity, ProgressStage, ProgressUpdate};

use crate::{
    ManagedArtifactError, ManagedArtifactStore, ProcessCancellation, StagedManagedArtifact,
};

const MAX_REDIRECTS: usize = 3;
const MAX_RESPONSE_CHUNK_BYTES: usize = 8 * 1024 * 1024;
const CONNECT_DEADLINE: Duration = Duration::from_secs(15);
const STALL_DEADLINE: Duration = Duration::from_secs(30);
const TRANSFER_DEADLINE: Duration = Duration::from_secs(600);
/// Bytes between two progress observations of one transfer.
pub(crate) const PROGRESS_STEP_BYTES: u64 = 1024 * 1024;
/// The neutral client identity sent to publishers: the product and purpose
/// only, never a user, host or contact detail.
const USER_AGENT: &str = "VSift/0.1 managed setup";
/// How hyper-util's private `TunnelError::ProxyAuthRequired` ends its text
/// (`tunnel error: proxy authorization required`) when a proxy answers a
/// `CONNECT` with `407`. The type is not exported, so its text is the only
/// way to tell proxy authentication from other tunnel failures; the D-07
/// proxy test fails if a dependency update changes it.
const PROXY_AUTH_REQUIRED: &str = "proxy authorization required";

/// The variable that lets a development build reach real publishers; its
/// only accepted value is [`DEVELOPMENT_NETWORK_ALLOWED`].
#[cfg(debug_assertions)]
const DEVELOPMENT_NETWORK_VARIABLE: &str = "VSIFT_DEV_PUBLISHER_NETWORK";
/// The value of [`DEVELOPMENT_NETWORK_VARIABLE`] that allows the network.
#[cfg(debug_assertions)]
const DEVELOPMENT_NETWORK_ALLOWED: &str = "allow";

/// Development builds resolve no host name (P13 PR 4 network guard).
///
/// Every test that is not `--release` runs a development build, and a test
/// that reaches a real publisher by accident (an accepted real plan digest,
/// say) would download from the internet on a contributor's machine or a CI
/// runner. Every reviewed publisher route names its host, and the loopback
/// test routes and test proxies are `127.0.0.1` literals that are never
/// resolved, so refusing every name keeps a development build on the
/// machine: the download fails as `offline` before any connection. The
/// opt-in real-tool tests run `--release`; a developer who wants a debug
/// build to download sets [`DEVELOPMENT_NETWORK_VARIABLE`] to
/// [`DEVELOPMENT_NETWORK_ALLOWED`]. Release builds never compile this.
#[cfg(debug_assertions)]
struct DevelopmentBuildResolver;

#[cfg(debug_assertions)]
impl reqwest::dns::Resolve for DevelopmentBuildResolver {
    fn resolve(&self, _name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(std::future::ready(Err(
            Box::new(DevelopmentNetworkRefused) as Box<dyn Error + Send + Sync>
        )))
    }
}

/// A development build refused to resolve a publisher's host.
#[cfg(debug_assertions)]
#[derive(Debug)]
struct DevelopmentNetworkRefused;

#[cfg(debug_assertions)]
impl fmt::Display for DevelopmentNetworkRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a development build does not resolve publisher hosts")
    }
}

#[cfg(debug_assertions)]
impl Error for DevelopmentNetworkRefused {}

/// Whether this development build was explicitly allowed to download.
#[cfg(debug_assertions)]
fn development_network_allowed() -> bool {
    std::env::var_os(DEVELOPMENT_NETWORK_VARIABLE)
        .is_some_and(|value| value == DEVELOPMENT_NETWORK_ALLOWED)
}

/// A publisher's reviewed release-service route, separate from the artifact digest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublisherOrigin {
    /// A GitHub release asset which may redirect only to its release-assets host.
    GitHubRelease,
    /// A pinned Hugging Face revision which may redirect only to its observed CDN.
    HuggingFaceModel,
    /// A local test server on `127.0.0.1`, which may redirect only to itself.
    /// Development builds only (`install-test-hooks`): it lets the D-03 and
    /// D-07 tests drive the real transport without the network.
    #[cfg(any(test, feature = "install-test-hooks"))]
    Loopback,
}

/// Exact publisher URL and integrity supplied by reviewed `VSift` source.
#[derive(Clone, Debug)]
pub struct ReviewedPublisherArtifact {
    url: Url,
    origin: PublisherOrigin,
    integrity: ArtifactIntegrity,
    /// An explicit proxy for a loopback test route; production transfers
    /// use the system proxy settings.
    #[cfg(any(test, feature = "install-test-hooks"))]
    test_proxy: Option<Url>,
}

/// A URL in reviewed source is not an immutable publisher artifact route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublisherSourceError {
    /// The URL is malformed, non-HTTPS, credential-bearing or has a query/fragment.
    InvalidUrl,
    /// The URL is outside the selected publisher's immutable release route.
    UnreviewedRoute,
}

impl fmt::Display for PublisherSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidUrl => "reviewed publisher URL is not canonical HTTPS",
            Self::UnreviewedRoute => "publisher URL is outside the reviewed route",
        })
    }
}

impl Error for PublisherSourceError {}

impl ReviewedPublisherArtifact {
    /// Validates a literal immutable source entry before it can enter transport.
    ///
    /// This constructor is for the reviewed source catalogue in `VSift` code. A
    /// user-entered URL or digest cannot become a managed setup trust anchor.
    ///
    /// # Errors
    ///
    /// Rejects noncanonical URLs and floating model revisions.
    pub fn from_reviewed_source(
        url: &'static str,
        origin: PublisherOrigin,
        integrity: ArtifactIntegrity,
    ) -> Result<Self, PublisherSourceError> {
        let parsed = Url::parse(url).map_err(|_| PublisherSourceError::InvalidUrl)?;
        if !canonical_https(&parsed) || parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(PublisherSourceError::InvalidUrl);
        }
        let route_is_reviewed = match origin {
            PublisherOrigin::GitHubRelease => {
                parsed.host_str() == Some("github.com") && pinned_release_route(parsed.path())
            }
            PublisherOrigin::HuggingFaceModel => {
                parsed.host_str() == Some("huggingface.co") && pinned_model_revision(parsed.path())
            }
            #[cfg(any(test, feature = "install-test-hooks"))]
            PublisherOrigin::Loopback => false,
        };
        if !route_is_reviewed {
            return Err(PublisherSourceError::UnreviewedRoute);
        }
        Ok(Self {
            url: parsed,
            origin,
            integrity,
            #[cfg(any(test, feature = "install-test-hooks"))]
            test_proxy: None,
        })
    }

    /// A route to a local test server, for the transfer and transaction
    /// tests only. `url` must be `http` or `https` on `127.0.0.1` with an
    /// explicit port and no credentials, query or fragment; `proxy`, when
    /// given, replaces the system proxy settings.
    ///
    /// Development builds only (`install-test-hooks`, refused in a release
    /// build and by the governance check): it bypasses the reviewed
    /// catalogue's trust anchor, which is why no production path can reach
    /// it.
    ///
    /// # Errors
    ///
    /// Rejects anything but a canonical loopback URL.
    #[cfg(any(test, feature = "install-test-hooks"))]
    pub fn loopback_for_tests(
        url: &str,
        integrity: ArtifactIntegrity,
        proxy: Option<&str>,
    ) -> Result<Self, PublisherSourceError> {
        let parsed = Url::parse(url).map_err(|_| PublisherSourceError::InvalidUrl)?;
        if !canonical_loopback(&parsed) || parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(PublisherSourceError::InvalidUrl);
        }
        let test_proxy = proxy
            .map(Url::parse)
            .transpose()
            .map_err(|_| PublisherSourceError::InvalidUrl)?;
        Ok(Self {
            url: parsed,
            origin: PublisherOrigin::Loopback,
            integrity,
            test_proxy,
        })
    }

    /// Whole-artifact bytes and SHA-256 which the transport must verify.
    #[must_use]
    pub const fn integrity(&self) -> ArtifactIntegrity {
        self.integrity
    }

    /// The artifact's file name: the last segment of its reviewed URL, the
    /// name an offline `--artifact-dir` import looks for.
    #[must_use]
    pub fn file_name(&self) -> Option<&str> {
        self.url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .filter(|name| !name.is_empty())
    }
}

/// A bounded reason no publisher bytes can be staged or activated.
#[derive(Debug)]
pub enum PublisherTransferError {
    /// The download did not complete, for the typed reason.
    Failed(DownloadFailureReason),
    /// The caller cancelled before completion.
    Cancelled,
    /// Owned staging or complete size/SHA-256 validation failed.
    Staging(ManagedArtifactError),
}

impl fmt::Display for PublisherTransferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(reason) => {
                write!(
                    formatter,
                    "publisher download failed ({})",
                    reason.identifier()
                )
            }
            Self::Cancelled => formatter.write_str("publisher transfer was cancelled"),
            Self::Staging(_) => {
                formatter.write_str("publisher bytes could not be verified in private staging")
            }
        }
    }
}

impl Error for PublisherTransferError {}

/// Downloads exact pinned publisher bytes to a private unactivated stage.
///
/// Redirects are allowed only to the reviewed publisher CDN for this source;
/// signed CDN URLs and proxy credentials are never included in errors. There
/// is deliberately no resume: interrupted transfers are discarded and a
/// later attempt restarts from byte zero. `progress` receives a
/// `fetching_artifact` observation about every mebibyte.
///
/// # Errors
///
/// Returns typed download, cancellation or staging failures.
pub async fn download_reviewed_publisher_artifact(
    store: &ManagedArtifactStore,
    artifact: &ReviewedPublisherArtifact,
    cancellation: &ProcessCancellation,
    progress: &dyn ProgressSink,
) -> Result<StagedManagedArtifact, PublisherTransferError> {
    if cancellation.is_cancelled() {
        return Err(PublisherTransferError::Cancelled);
    }
    let deadline = Instant::now() + TRANSFER_DEADLINE;
    let client = transfer_client(artifact)?;
    let request = client
        .get(artifact.url.clone())
        .header(ACCEPT_ENCODING, "identity")
        .send();
    let mut response = tokio::select! {
        () = cancellation.wait_cancelled() => return Err(PublisherTransferError::Cancelled),
        result = timeout_at(deadline, request) => {
            result
                .map_err(|_| PublisherTransferError::Failed(DownloadFailureReason::Offline))?
                .map_err(|error| PublisherTransferError::Failed(classify(&error)))?
        }
    };
    validate_response(&response, artifact).map_err(PublisherTransferError::Failed)?;
    let total = artifact.integrity.bytes();
    let mut received = 0_u64;
    let mut reported = 0_u64;
    report_fetch(progress, 0, total);
    let mut staging = store
        .begin_stream(artifact.integrity)
        .map_err(PublisherTransferError::Staging)?;
    loop {
        let chunk = tokio::select! {
            () = cancellation.wait_cancelled() => Err(PublisherTransferError::Cancelled),
            result = timeout_at(deadline, response.chunk()) => match result {
                Ok(chunk) => {
                    chunk.map_err(|error| PublisherTransferError::Failed(classify(&error)))
                }
                Err(_) => Err(PublisherTransferError::Failed(DownloadFailureReason::Offline)),
            },
        };
        let chunk = match chunk {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(error) => {
                staging.abort().map_err(PublisherTransferError::Staging)?;
                return Err(error);
            }
        };
        if chunk.len() > MAX_RESPONSE_CHUNK_BYTES {
            staging.abort().map_err(PublisherTransferError::Staging)?;
            return Err(PublisherTransferError::Failed(DownloadFailureReason::Size));
        }
        let write = tokio::select! {
            () = cancellation.wait_cancelled() => Err(PublisherTransferError::Cancelled),
            result = timeout_at(deadline, staging.append(&chunk)) => match result {
                Ok(written) => written.map_err(PublisherTransferError::Staging),
                Err(_) => Err(PublisherTransferError::Failed(DownloadFailureReason::Offline)),
            },
        };
        if let Err(error) = write {
            staging.abort().map_err(PublisherTransferError::Staging)?;
            return Err(error);
        }
        received = received.saturating_add(chunk.len() as u64);
        if received.saturating_sub(reported) >= PROGRESS_STEP_BYTES {
            reported = received;
            report_fetch(progress, received, total);
        }
    }
    if cancellation.is_cancelled() {
        staging.abort().map_err(PublisherTransferError::Staging)?;
        return Err(PublisherTransferError::Cancelled);
    }
    let finished = tokio::select! {
        () = cancellation.wait_cancelled() => Err(PublisherTransferError::Cancelled),
        result = timeout_at(deadline, staging.finish()) => match result {
            Ok(finished) => finished.map_err(PublisherTransferError::Staging),
            Err(_) => Err(PublisherTransferError::Failed(DownloadFailureReason::Offline)),
        },
    };
    if let Err(error) = finished {
        staging.abort().map_err(PublisherTransferError::Staging)?;
        return Err(error);
    }
    if cancellation.is_cancelled() {
        staging.abort().map_err(PublisherTransferError::Staging)?;
        return Err(PublisherTransferError::Cancelled);
    }
    report_fetch(progress, received, total);
    Ok(staging.complete())
}

/// The client for one transfer: system TLS (at least 1.2), HTTPS only, no
/// compression, no referer, the reviewed redirect policy, bounded
/// deadlines and the neutral user agent.
fn transfer_client(artifact: &ReviewedPublisherArtifact) -> Result<Client, PublisherTransferError> {
    let origin = artifact.origin;
    let start = artifact.url.clone();
    let redirect = Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS
            || !reviewed_redirect(attempt.url(), origin, &start)
        {
            attempt.error("redirect outside reviewed publisher route")
        } else {
            attempt.follow()
        }
    });
    let builder = Client::builder()
        .tls_backend_native()
        .https_only(true)
        .tls_version_min(reqwest::tls::Version::TLS_1_2)
        .redirect(redirect)
        .referer(false)
        .no_gzip()
        .no_brotli()
        .no_zstd()
        .no_deflate()
        .connect_timeout(CONNECT_DEADLINE)
        .read_timeout(STALL_DEADLINE)
        .timeout(TRANSFER_DEADLINE)
        .user_agent(USER_AGENT);
    #[cfg(debug_assertions)]
    let builder = if development_network_allowed() {
        builder
    } else {
        builder.dns_resolver(DevelopmentBuildResolver)
    };
    #[cfg(any(test, feature = "install-test-hooks"))]
    let builder = if origin == PublisherOrigin::Loopback {
        let builder = builder.https_only(false);
        match &artifact.test_proxy {
            Some(proxy) => builder.proxy(
                reqwest::Proxy::all(proxy.clone())
                    .map_err(|_| PublisherTransferError::Failed(DownloadFailureReason::Offline))?,
            ),
            None => builder.no_proxy(),
        }
    } else {
        builder
    };
    builder
        .build()
        .map_err(|_| PublisherTransferError::Failed(DownloadFailureReason::Tls))
}

fn report_fetch(progress: &dyn ProgressSink, completed: u64, total: u64) {
    progress.report(ProgressUpdate {
        stage: ProgressStage::FetchingArtifact,
        completed: completed.min(total),
        total: Some(total),
    });
}

/// Accepts only one complete `200 OK` identity body of the reviewed size
/// from the reviewed route.
fn validate_response(
    response: &reqwest::Response,
    artifact: &ReviewedPublisherArtifact,
) -> Result<(), DownloadFailureReason> {
    if response.status() == StatusCode::PROXY_AUTHENTICATION_REQUIRED {
        return Err(DownloadFailureReason::ProxyAuth);
    }
    if response.status() != StatusCode::OK {
        return Err(DownloadFailureReason::HttpStatus);
    }
    if response.url() != &artifact.url
        && !reviewed_redirect(response.url(), artifact.origin, &artifact.url)
    {
        return Err(DownloadFailureReason::RedirectPolicy);
    }
    if response.headers().contains_key(CONTENT_RANGE)
        || response
            .headers()
            .get(CONTENT_ENCODING)
            .is_some_and(|encoding| encoding.as_bytes() != b"identity")
    {
        return Err(DownloadFailureReason::HttpStatus);
    }
    if response
        .content_length()
        .is_some_and(|bytes| bytes != artifact.integrity.bytes())
    {
        return Err(DownloadFailureReason::Size);
    }
    Ok(())
}

/// The typed reason of a transport failure. Only the error's kind and the
/// types in its source chain are read, never its text, except for the one
/// proxy-authentication marker documented at [`PROXY_AUTH_REQUIRED`].
fn classify(error: &reqwest::Error) -> DownloadFailureReason {
    if error.is_redirect() {
        return DownloadFailureReason::RedirectPolicy;
    }
    let mut source: Option<&(dyn Error + 'static)> = Some(error);
    while let Some(current) = source {
        if current.is::<native_tls::Error>() {
            return DownloadFailureReason::Tls;
        }
        if current.source().is_none() && current.to_string().ends_with(PROXY_AUTH_REQUIRED) {
            return DownloadFailureReason::ProxyAuth;
        }
        source = current.source();
    }
    DownloadFailureReason::Offline
}

fn canonical_https(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some()
}

#[cfg(any(test, feature = "install-test-hooks"))]
fn canonical_loopback(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host_str() == Some("127.0.0.1")
        && url.port().is_some()
        && url.username().is_empty()
        && url.password().is_none()
}

#[cfg_attr(
    not(any(test, feature = "install-test-hooks")),
    allow(
        unused_variables,
        reason = "only the loopback route compares with its start"
    )
)]
fn reviewed_redirect(url: &Url, origin: PublisherOrigin, start: &Url) -> bool {
    match origin {
        PublisherOrigin::GitHubRelease => {
            canonical_https(url)
                && url.fragment().is_none()
                && url.host_str() == Some("release-assets.githubusercontent.com")
                && url.path().starts_with("/github-production-release-asset/")
        }
        PublisherOrigin::HuggingFaceModel => {
            canonical_https(url)
                && url.fragment().is_none()
                && url.host_str() == Some("us.aws.cdn.hf.co")
                && url.path().starts_with("/xet-bridge-us/")
        }
        #[cfg(any(test, feature = "install-test-hooks"))]
        PublisherOrigin::Loopback => {
            canonical_loopback(url)
                && url.fragment().is_none()
                && url.scheme() == start.scheme()
                && url.port() == start.port()
        }
    }
}

fn pinned_model_revision(path: &str) -> bool {
    let segments = path.split('/').collect::<Vec<_>>();
    let ["", owner, repo, "resolve", revision, filename] = segments.as_slice() else {
        return false;
    };
    if owner.is_empty() || repo.is_empty() || filename.is_empty() {
        return false;
    }
    revision.len() == 40
        && revision
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn pinned_release_route(path: &str) -> bool {
    let segments = path.split('/').collect::<Vec<_>>();
    let ["", owner, repo, "releases", "download", tag, filename] = segments.as_slice() else {
        return false;
    };
    !owner.is_empty()
        && !repo.is_empty()
        && !filename.is_empty()
        && !tag.is_empty()
        && !["latest", "main", "master"].contains(&tag.to_ascii_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use std::{fmt::Write as _, fs, path::PathBuf};

    use vsift_application::NoProgress;
    use vsift_domain::ArtifactIntegrity;

    use crate::{ManagedArtifactStore, ProcessCancellation};

    use super::{
        PublisherOrigin, PublisherSourceError, PublisherTransferError, ReviewedPublisherArtifact,
        download_reviewed_publisher_artifact, reviewed_redirect,
    };

    fn integrity() -> Result<ArtifactIntegrity, Box<dyn std::error::Error>> {
        Ok(ArtifactIntegrity::from_sha256_hex(
            3,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )?)
    }

    #[test]
    fn accepts_only_immutable_canonical_publisher_routes() -> Result<(), Box<dyn std::error::Error>>
    {
        let pinned = ReviewedPublisherArtifact::from_reviewed_source(
            "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.2/whisper-bin-ubuntu-x64.tar.gz",
            PublisherOrigin::GitHubRelease,
            integrity()?,
        )?;
        assert_eq!(pinned.file_name(), Some("whisper-bin-ubuntu-x64.tar.gz"));
        assert!(matches!(
            ReviewedPublisherArtifact::from_reviewed_source(
                "http://github.com/ggml-org/whisper.cpp/releases/download/v1.9.2/whisper-bin-ubuntu-x64.tar.gz",
                PublisherOrigin::GitHubRelease,
                integrity()?
            ),
            Err(PublisherSourceError::InvalidUrl)
        ));
        assert!(matches!(
            ReviewedPublisherArtifact::from_reviewed_source(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
                PublisherOrigin::HuggingFaceModel,
                integrity()?
            ),
            Err(PublisherSourceError::UnreviewedRoute)
        ));
        assert!(matches!(
            ReviewedPublisherArtifact::from_reviewed_source(
                "https://github.com/ggml-org/whisper.cpp/releases/download/latest/whisper-bin-ubuntu-x64.tar.gz",
                PublisherOrigin::GitHubRelease,
                integrity()?
            ),
            Err(PublisherSourceError::UnreviewedRoute)
        ));
        assert!(matches!(
            ReviewedPublisherArtifact::from_reviewed_source(
                "http://127.0.0.1:8080/artifact.bin",
                PublisherOrigin::Loopback,
                integrity()?
            ),
            Err(PublisherSourceError::InvalidUrl)
        ));
        assert!(ReviewedPublisherArtifact::from_reviewed_source(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/80da2d8bfee42b0e836fc3a9890373e5defc00a6/ggml-base.bin",
            PublisherOrigin::HuggingFaceModel,
            integrity()?
        ).is_ok());
        Ok(())
    }

    #[test]
    fn loopback_routes_accept_only_canonical_local_urls() -> Result<(), Box<dyn std::error::Error>>
    {
        assert!(
            ReviewedPublisherArtifact::loopback_for_tests(
                "http://127.0.0.1:8080/a.bin",
                integrity()?,
                None
            )
            .is_ok()
        );
        for url in [
            "http://localhost:8080/a.bin",
            "http://127.0.0.1/a.bin",
            "http://user:secret@127.0.0.1:8080/a.bin",
            "http://127.0.0.1:8080/a.bin?x=1",
            "ftp://127.0.0.1:8080/a.bin",
        ] {
            assert!(
                ReviewedPublisherArtifact::loopback_for_tests(url, integrity()?, None).is_err(),
                "{url}"
            );
        }
        Ok(())
    }

    #[test]
    fn rejects_host_spoofing_downgrade_userinfo_and_unreviewed_cdns()
    -> Result<(), Box<dyn std::error::Error>> {
        let start = reqwest::Url::parse(
            "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.2/whisper-bin-ubuntu-x64.tar.gz",
        )?;
        let good = reqwest::Url::parse(
            "https://release-assets.githubusercontent.com/github-production-release-asset/541269386/file?sig=private",
        )?;
        assert!(reviewed_redirect(
            &good,
            PublisherOrigin::GitHubRelease,
            &start
        ));
        for url in [
            "https://release-assets.githubusercontent.com.evil.test/github-production-release-asset/file",
            "http://release-assets.githubusercontent.com/github-production-release-asset/file",
            "https://user:secret@release-assets.githubusercontent.com/github-production-release-asset/file",
            "https://release-assets.githubusercontent.com:444/github-production-release-asset/file",
            "https://objects.githubusercontent.com/github-production-release-asset/file",
        ] {
            assert!(!reviewed_redirect(
                &reqwest::Url::parse(url)?,
                PublisherOrigin::GitHubRelease,
                &start
            ));
        }
        let model =
            reqwest::Url::parse("https://us.aws.cdn.hf.co/xet-bridge-us/object?Signature=private")?;
        assert!(reviewed_redirect(
            &model,
            PublisherOrigin::HuggingFaceModel,
            &start
        ));
        assert!(!reviewed_redirect(
            &reqwest::Url::parse("https://eu.aws.cdn.hf.co/xet-bridge-us/object")?,
            PublisherOrigin::HuggingFaceModel,
            &start
        ));
        let loopback = reqwest::Url::parse("http://127.0.0.1:8080/a.bin")?;
        assert!(reviewed_redirect(
            &reqwest::Url::parse("http://127.0.0.1:8080/b.bin")?,
            PublisherOrigin::Loopback,
            &loopback
        ));
        for url in [
            "http://localhost:8080/b.bin",
            "http://127.0.0.1:8081/b.bin",
            "https://127.0.0.1:8080/b.bin",
            "http://user:secret@127.0.0.1:8080/b.bin",
        ] {
            assert!(
                !reviewed_redirect(
                    &reqwest::Url::parse(url)?,
                    PublisherOrigin::Loopback,
                    &loopback
                ),
                "{url}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn pre_cancelled_transfer_does_not_create_storage()
    -> Result<(), Box<dyn std::error::Error>> {
        let parent = unique_parent()?;
        let result = async {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            let artifact = ReviewedPublisherArtifact::from_reviewed_source(
                "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.2/whisper-bin-ubuntu-x64.tar.gz",
                PublisherOrigin::GitHubRelease,
                integrity()?,
            )?;
            let cancellation = ProcessCancellation::new();
            cancellation.cancel();
            assert!(matches!(
                download_reviewed_publisher_artifact(&store, &artifact, &cancellation, &NoProgress).await,
                Err(PublisherTransferError::Cancelled)
            ));
            assert_eq!(fs::read_dir(&parent)?.count(), 0);
            Ok::<(), Box<dyn std::error::Error>>(())
        }.await;
        fs::remove_dir_all(parent)?;
        result
    }

    /// The P13 PR 4 network guard: a development build (every test run
    /// without `--release`) fails a reviewed publisher download as `offline`
    /// before any connection and creates no stage.
    #[cfg(debug_assertions)]
    #[tokio::test]
    async fn a_development_build_does_not_reach_a_publisher()
    -> Result<(), Box<dyn std::error::Error>> {
        if super::development_network_allowed() {
            println!("skipped: this run allows the development network");
            return Ok(());
        }
        let parent = unique_parent()?;
        let result = async {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            for (url, origin) in [
                (
                    "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.2/whisper-bin-ubuntu-x64.tar.gz",
                    PublisherOrigin::GitHubRelease,
                ),
                (
                    "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base-q5_1.bin",
                    PublisherOrigin::HuggingFaceModel,
                ),
            ] {
                let artifact =
                    ReviewedPublisherArtifact::from_reviewed_source(url, origin, integrity()?)?;
                assert!(matches!(
                    download_reviewed_publisher_artifact(
                        &store,
                        &artifact,
                        &ProcessCancellation::new(),
                        &NoProgress
                    )
                    .await,
                    Err(PublisherTransferError::Failed(
                        super::DownloadFailureReason::Offline
                    ))
                ));
            }
            assert_eq!(fs::read_dir(&parent)?.count(), 0);
            Ok::<(), Box<dyn std::error::Error>>(())
        }
        .await;
        fs::remove_dir_all(parent)?;
        result
    }

    #[tokio::test]
    #[ignore = "opt-in pinned publisher download; set VSIFT_P06_DIRECT_TRANSFER=1 and run --release \
                (or set VSIFT_DEV_PUBLISHER_NETWORK=allow)"]
    async fn pinned_whisper_asset_downloads_to_owned_stage_without_execution()
    -> Result<(), Box<dyn std::error::Error>> {
        if std::env::var_os("VSIFT_P06_DIRECT_TRANSFER").is_none() {
            return Err("opt-in P06 direct transfer flag missing".into());
        }
        let parent = unique_parent()?;
        let result = async {
            let store = ManagedArtifactStore::at(parent.join("managed"))?;
            let pinned = ArtifactIntegrity::from_sha256_hex(
                9_497_583,
                "46811a3ecf584307480a220b9ef5ff81b7b22dc41577cbc274ce3afc61f753b1",
            )?;
            let artifact = ReviewedPublisherArtifact::from_reviewed_source(
                "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.2/whisper-bin-ubuntu-x64.tar.gz",
                PublisherOrigin::GitHubRelease,
                pinned,
            )?;
            let cancellation = ProcessCancellation::new();
            let staged =
                download_reviewed_publisher_artifact(&store, &artifact, &cancellation, &NoProgress)
                    .await?;
            assert_eq!(staged.open_artifact()?.metadata()?.len(), pinned.bytes());
            staged.discard()?;
            Ok::<(), Box<dyn std::error::Error>>(())
        }.await;
        fs::remove_dir_all(parent)?;
        result
    }

    fn unique_parent() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| std::io::Error::other("random source failed"))?;
        let mut name = String::with_capacity(32);
        for byte in random {
            write!(&mut name, "{byte:02x}")?;
        }
        let path = std::env::temp_dir().join(format!("vsift-direct-transfer-test-{name}"));
        fs::create_dir(&path)?;
        Ok(path)
    }
}
