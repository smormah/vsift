//! Direct HTTPS transfer of reviewed publisher bytes into unactivated owned staging.

use std::{error::Error, fmt, time::Duration};

use reqwest::{
    Client, Url,
    header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_RANGE},
    redirect::Policy,
};
use tokio::{
    sync::watch,
    time::{Instant, timeout_at},
};
use vsift_domain::ArtifactIntegrity;

use crate::{ManagedArtifactError, ManagedArtifactStore, StagedManagedArtifact};

const MAX_REDIRECTS: usize = 3;
const MAX_RESPONSE_CHUNK_BYTES: usize = 8 * 1024 * 1024;
const CONNECT_DEADLINE: Duration = Duration::from_secs(15);
const STALL_DEADLINE: Duration = Duration::from_secs(30);
const TRANSFER_DEADLINE: Duration = Duration::from_secs(600);

/// A publisher's reviewed release-service route, separate from the artifact digest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublisherOrigin {
    /// A GitHub release asset which may redirect only to its release-assets host.
    GitHubRelease,
    /// A pinned Hugging Face revision which may redirect only to its observed CDN.
    HuggingFaceModel,
}

/// Exact publisher URL and integrity supplied by reviewed `VSift` source.
#[derive(Clone, Debug)]
pub struct ReviewedPublisherArtifact {
    url: Url,
    origin: PublisherOrigin,
    integrity: ArtifactIntegrity,
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
        };
        if !route_is_reviewed {
            return Err(PublisherSourceError::UnreviewedRoute);
        }
        Ok(Self {
            url: parsed,
            origin,
            integrity,
        })
    }

    /// Whole-artifact bytes and SHA-256 which the transport must verify.
    #[must_use]
    pub const fn integrity(&self) -> ArtifactIntegrity {
        self.integrity
    }
}

/// Caller cancellation for one bounded publisher transfer.
#[derive(Clone, Debug)]
pub struct PublisherTransferCancellation {
    sender: watch::Sender<bool>,
}

impl PublisherTransferCancellation {
    /// Creates an uncancelled transfer signal.
    #[must_use]
    pub fn new() -> Self {
        let (sender, _) = watch::channel(false);
        Self { sender }
    }

    /// Requests cancellation; repeated requests are idempotent.
    pub fn cancel(&self) {
        self.sender.send_replace(true);
    }

    /// Reports whether cancellation was already requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.sender.borrow()
    }

    async fn cancelled(&self) {
        let mut receiver = self.sender.subscribe();
        loop {
            if *receiver.borrow_and_update() {
                return;
            }
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Default for PublisherTransferCancellation {
    fn default() -> Self {
        Self::new()
    }
}

/// A bounded reason no publisher bytes can be staged or activated.
#[derive(Debug)]
pub enum PublisherTransferError {
    /// TLS, proxy, DNS, connection or response-body transport failed.
    Network,
    /// A response or read deadline expired.
    TimedOut,
    /// The publisher attempted a forbidden, unsafe or excessive redirect.
    RedirectRejected,
    /// The server returned something other than an ordinary complete response.
    UnexpectedResponse,
    /// Response metadata conflicts with the reviewed artifact size or encoding.
    InvalidResponseMetadata,
    /// The response body exceeded the bounded per-chunk memory policy.
    ResponseChunkTooLarge,
    /// The caller cancelled before completion.
    Cancelled,
    /// Owned staging or complete size/SHA-256 validation failed.
    Staging(ManagedArtifactError),
}

impl fmt::Display for PublisherTransferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Network => "publisher HTTPS transfer failed",
            Self::TimedOut => "publisher transfer exceeded its deadline",
            Self::RedirectRejected => "publisher redirect is outside the reviewed policy",
            Self::UnexpectedResponse => "publisher did not return a complete artifact",
            Self::InvalidResponseMetadata => {
                "publisher response metadata differs from reviewed bytes"
            }
            Self::ResponseChunkTooLarge => "publisher response chunk exceeded the memory limit",
            Self::Cancelled => "publisher transfer was cancelled",
            Self::Staging(_) => "publisher bytes could not be verified in private staging",
        })
    }
}

impl Error for PublisherTransferError {}

/// Downloads exact pinned publisher bytes to a private unactivated stage.
///
/// Redirects are allowed only to the reviewed publisher CDN for this source;
/// signed CDN URLs and proxy credentials are never included in errors. There
/// is deliberately no resume in this first transport policy: interrupted
/// transfers are discarded and a later attempt restarts from byte zero.
///
/// # Errors
///
/// Returns typed source, response, timeout, cancellation or staging failures.
pub async fn download_reviewed_publisher_artifact(
    store: &ManagedArtifactStore,
    artifact: &ReviewedPublisherArtifact,
    cancellation: &PublisherTransferCancellation,
) -> Result<StagedManagedArtifact, PublisherTransferError> {
    if cancellation.is_cancelled() {
        return Err(PublisherTransferError::Cancelled);
    }
    let deadline = Instant::now() + TRANSFER_DEADLINE;
    let origin = artifact.origin;
    let redirect = Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS || !reviewed_redirect(attempt.url(), origin) {
            attempt.error("redirect outside reviewed publisher route")
        } else {
            attempt.follow()
        }
    });
    let client = Client::builder()
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
        .user_agent("VSift/0.1 managed setup")
        .build()
        .map_err(|_| PublisherTransferError::Network)?;
    let request = client
        .get(artifact.url.clone())
        .header(ACCEPT_ENCODING, "identity")
        .send();
    let mut response = tokio::select! {
        () = cancellation.cancelled() => return Err(PublisherTransferError::Cancelled),
        result = timeout_at(deadline, request) => {
            result.map_err(|_| PublisherTransferError::TimedOut)?
                .map_err(|error| map_network_error(&error))?
        }
    };
    validate_response(&response, artifact)?;
    let mut staging = store
        .begin_stream(artifact.integrity)
        .map_err(PublisherTransferError::Staging)?;
    loop {
        let chunk = tokio::select! {
            () = cancellation.cancelled() => Err(PublisherTransferError::Cancelled),
            result = timeout_at(deadline, response.chunk()) => {
                result.map_err(|_| PublisherTransferError::TimedOut)?
                    .map_err(|error| map_network_error(&error))
            }
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
            return Err(PublisherTransferError::ResponseChunkTooLarge);
        }
        let write = tokio::select! {
            () = cancellation.cancelled() => Err(PublisherTransferError::Cancelled),
            result = timeout_at(deadline, staging.append(&chunk)) => {
                result.map_err(|_| PublisherTransferError::TimedOut)?
                    .map_err(PublisherTransferError::Staging)
            }
        };
        if let Err(error) = write {
            staging.abort().map_err(PublisherTransferError::Staging)?;
            return Err(error);
        }
    }
    if cancellation.is_cancelled() {
        staging.abort().map_err(PublisherTransferError::Staging)?;
        return Err(PublisherTransferError::Cancelled);
    }
    let finished = tokio::select! {
        () = cancellation.cancelled() => Err(PublisherTransferError::Cancelled),
        result = timeout_at(deadline, staging.finish()) => {
            result.map_err(|_| PublisherTransferError::TimedOut)?
                .map_err(PublisherTransferError::Staging)
        }
    };
    if let Err(error) = finished {
        staging.abort().map_err(PublisherTransferError::Staging)?;
        return Err(error);
    }
    if cancellation.is_cancelled() {
        staging.abort().map_err(PublisherTransferError::Staging)?;
        return Err(PublisherTransferError::Cancelled);
    }
    Ok(staging.complete())
}

fn validate_response(
    response: &reqwest::Response,
    artifact: &ReviewedPublisherArtifact,
) -> Result<(), PublisherTransferError> {
    if response.status() != reqwest::StatusCode::OK
        || (response.url() != &artifact.url && !reviewed_redirect(response.url(), artifact.origin))
    {
        return Err(PublisherTransferError::UnexpectedResponse);
    }
    if response.headers().contains_key(CONTENT_RANGE)
        || response
            .headers()
            .get(CONTENT_ENCODING)
            .is_some_and(|encoding| encoding.as_bytes() != b"identity")
        || response
            .content_length()
            .is_some_and(|bytes| bytes != artifact.integrity.bytes())
    {
        return Err(PublisherTransferError::InvalidResponseMetadata);
    }
    Ok(())
}

fn canonical_https(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some()
}

fn reviewed_redirect(url: &Url, origin: PublisherOrigin) -> bool {
    if !canonical_https(url) || url.fragment().is_some() {
        return false;
    }
    match origin {
        PublisherOrigin::GitHubRelease => {
            url.host_str() == Some("release-assets.githubusercontent.com")
                && url.path().starts_with("/github-production-release-asset/")
        }
        PublisherOrigin::HuggingFaceModel => {
            url.host_str() == Some("us.aws.cdn.hf.co") && url.path().starts_with("/xet-bridge-us/")
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

fn map_network_error(error: &reqwest::Error) -> PublisherTransferError {
    if error.is_timeout() {
        PublisherTransferError::TimedOut
    } else if error.is_redirect() {
        PublisherTransferError::RedirectRejected
    } else {
        PublisherTransferError::Network
    }
}

#[cfg(test)]
mod tests {
    use std::{fmt::Write as _, fs, path::PathBuf};

    use vsift_domain::ArtifactIntegrity;

    use crate::ManagedArtifactStore;

    use super::{
        PublisherOrigin, PublisherSourceError, PublisherTransferCancellation,
        PublisherTransferError, ReviewedPublisherArtifact, download_reviewed_publisher_artifact,
        reviewed_redirect,
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
        );
        assert!(pinned.is_ok());
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
        assert!(ReviewedPublisherArtifact::from_reviewed_source(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/80da2d8bfee42b0e836fc3a9890373e5defc00a6/ggml-base.bin",
            PublisherOrigin::HuggingFaceModel,
            integrity()?
        ).is_ok());
        Ok(())
    }

    #[test]
    fn rejects_host_spoofing_downgrade_userinfo_and_unreviewed_cdns()
    -> Result<(), Box<dyn std::error::Error>> {
        let good = reqwest::Url::parse(
            "https://release-assets.githubusercontent.com/github-production-release-asset/541269386/file?sig=private",
        )?;
        assert!(reviewed_redirect(&good, PublisherOrigin::GitHubRelease));
        for url in [
            "https://release-assets.githubusercontent.com.evil.test/github-production-release-asset/file",
            "http://release-assets.githubusercontent.com/github-production-release-asset/file",
            "https://user:secret@release-assets.githubusercontent.com/github-production-release-asset/file",
            "https://release-assets.githubusercontent.com:444/github-production-release-asset/file",
            "https://objects.githubusercontent.com/github-production-release-asset/file",
        ] {
            assert!(!reviewed_redirect(
                &reqwest::Url::parse(url)?,
                PublisherOrigin::GitHubRelease
            ));
        }
        let model =
            reqwest::Url::parse("https://us.aws.cdn.hf.co/xet-bridge-us/object?Signature=private")?;
        assert!(reviewed_redirect(&model, PublisherOrigin::HuggingFaceModel));
        assert!(!reviewed_redirect(
            &reqwest::Url::parse("https://eu.aws.cdn.hf.co/xet-bridge-us/object")?,
            PublisherOrigin::HuggingFaceModel
        ));
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
            let cancellation = PublisherTransferCancellation::new();
            cancellation.cancel();
            assert!(matches!(
                download_reviewed_publisher_artifact(&store, &artifact, &cancellation).await,
                Err(PublisherTransferError::Cancelled)
            ));
            assert_eq!(fs::read_dir(&parent)?.count(), 0);
            Ok::<(), Box<dyn std::error::Error>>(())
        }.await;
        fs::remove_dir_all(parent)?;
        result
    }

    #[tokio::test]
    #[ignore = "opt-in pinned publisher download; set VSIFT_P06_DIRECT_TRANSFER=1"]
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
            let cancellation = PublisherTransferCancellation::new();
            let staged = download_reviewed_publisher_artifact(&store, &artifact, &cancellation).await?;
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
