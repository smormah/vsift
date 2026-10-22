//! Online sources (ADR 0028): a link to a video, the bounds of fetching it,
//! why a fetch failed, and the record of where a fetched source came from.
//!
//! A link is untrusted and may carry a signed token, in its query or in its
//! path, so [`SourceLink`] never prints itself: its `Debug` form is redacted,
//! and the session records only a digest of it and the hosts it reached
//! ([`DirectLinkOrigin`]). Fetching is the infrastructure's; these values are
//! what the use case and the public contract say about it.

use std::{error::Error, fmt};

use crate::Sha256Hex;

/// Longest link `ingest --url` accepts.
pub const MAX_SOURCE_LINK_BYTES: usize = 2_048;
/// Default largest download: 4 GiB.
pub const DEFAULT_DOWNLOAD_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Largest download a caller may ask for: the 20 GiB source maximum.
pub const MAX_DOWNLOAD_BYTES: u64 = 20 * 1024 * 1024 * 1024;
/// Default whole-transfer deadline: 30 minutes.
pub const DEFAULT_DOWNLOAD_SECONDS: u32 = 30 * 60;
/// Longest whole-transfer deadline a caller may ask for: 4 hours.
pub const MAX_DOWNLOAD_SECONDS: u32 = 4 * 60 * 60;
/// Most redirects a fetch follows.
pub const MAX_SOURCE_REDIRECTS: u8 = 3;
/// Longest host name a record keeps (DNS's limit).
pub const MAX_HOST_NAME_BYTES: usize = 253;

/// Why a link could not be fetched into a session (`SOURCE_FETCH_FAILED`'s
/// typed reason). Every reason names the online step; local files are
/// unaffected by any of them.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SourceFetchReason {
    /// The link is not an `https://` link (or is not a link at all).
    NotHttps,
    /// The link carries a user name or password, which is never sent.
    CredentialsInLink,
    /// The link or a redirect names a port other than 443.
    PortNotAllowed,
    /// The link's host, or a redirect's, is or resolves to an address that is
    /// not public (loopback, private, link-local, metadata and others), or is
    /// a name for an internal host.
    AddressRefused,
    /// The TLS connection could not be established or verified.
    Tls,
    /// A redirect left HTTPS, or there were more than three.
    RedirectPolicy,
    /// The server answered with a status other than `200 OK`.
    HttpStatus,
    /// A proxy asked for credentials.
    ProxyAuth,
    /// The proxy the environment names (`HTTPS_PROXY` or `ALL_PROXY`) is not
    /// a proxy address `VSift` can use. It is refused rather than ignored, so a
    /// fetch never goes direct past a proxy the user set.
    ProxyUnusable,
    /// The name could not be resolved or the server not reached.
    Offline,
    /// No byte arrived for 30 seconds.
    Stalled,
    /// The whole transfer took longer than its deadline.
    Deadline,
    /// The body is, or says it is, larger than the download limit.
    Size,
    /// The server sent part of a resource (`206` or a range).
    Partial,
    /// The server sent a compressed body.
    Compressed,
    /// The link answered with a web page, not a video file.
    WebPage,
    /// The body does not start as an MP4/MOV or Matroska/WebM file.
    NotAVideo,
    /// The body ended before the length the server announced.
    Truncated,
    /// The host runs with strict isolation, which allows no network.
    StrictIsolation,
}

impl SourceFetchReason {
    /// Every reason, in declaration order.
    pub const ALL: [Self; 19] = [
        Self::NotHttps,
        Self::CredentialsInLink,
        Self::PortNotAllowed,
        Self::AddressRefused,
        Self::Tls,
        Self::RedirectPolicy,
        Self::HttpStatus,
        Self::ProxyAuth,
        Self::ProxyUnusable,
        Self::Offline,
        Self::Stalled,
        Self::Deadline,
        Self::Size,
        Self::Partial,
        Self::Compressed,
        Self::WebPage,
        Self::NotAVideo,
        Self::Truncated,
        Self::StrictIsolation,
    ];

    /// Stable public identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::NotHttps => "not_https",
            Self::CredentialsInLink => "credentials_in_link",
            Self::PortNotAllowed => "port_not_allowed",
            Self::AddressRefused => "address_refused",
            Self::Tls => "tls",
            Self::RedirectPolicy => "redirect_policy",
            Self::HttpStatus => "http_status",
            Self::ProxyAuth => "proxy_auth",
            Self::ProxyUnusable => "proxy_unusable",
            Self::Offline => "offline",
            Self::Stalled => "stalled",
            Self::Deadline => "deadline",
            Self::Size => "size",
            Self::Partial => "partial",
            Self::Compressed => "compressed",
            Self::WebPage => "web_page",
            Self::NotAVideo => "not_a_video",
            Self::Truncated => "truncated",
            Self::StrictIsolation => "strict_isolation",
        }
    }
}

impl fmt::Display for SourceFetchReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the video link could not be fetched ({})",
            self.identifier()
        )
    }
}

impl Error for SourceFetchReason {}

/// A link to a video, as the caller gave it, checked only for its shape: at
/// most [`MAX_SOURCE_LINK_BYTES`] of printable ASCII with no space, starting
/// `https://` (in any case). The infrastructure parses and judges it fully.
///
/// It may carry a signed token, so it never prints itself: `Debug` is
/// redacted, there is no `Display`, and [`Self::expose`] is the one way to
/// read it, for the transport alone.
#[derive(Clone, Eq, PartialEq)]
pub struct SourceLink(String);

impl SourceLink {
    /// Checks a link's shape.
    ///
    /// # Errors
    ///
    /// [`SourceFetchReason::NotHttps`] for an empty, oversized, non-ASCII or
    /// spaced text, or one that does not start `https://`.
    pub fn parse(text: &str) -> Result<Self, SourceFetchReason> {
        let printable = text.bytes().all(|byte| byte.is_ascii_graphic());
        let https = text
            .get(..8)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"));
        if text.len() > MAX_SOURCE_LINK_BYTES || !printable || !https || text.len() == 8 {
            return Err(SourceFetchReason::NotHttps);
        }
        Ok(Self(text.to_owned()))
    }

    /// The link's text, for the transport that fetches it. Never print, log
    /// or store it: it may carry a signed token.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SourceLink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SourceLink(<redacted>)")
    }
}

/// The bounds of one fetch: its largest size and its whole-transfer deadline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DownloadLimits {
    max_bytes: u64,
    max_seconds: u32,
}

/// A download limit outside its allowed range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DownloadLimitsError {
    /// The size is zero or above [`MAX_DOWNLOAD_BYTES`].
    Bytes,
    /// The deadline is zero or above [`MAX_DOWNLOAD_SECONDS`].
    Seconds,
}

impl fmt::Display for DownloadLimitsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Bytes => "the download size limit must be from 1 byte to 20 GiB",
            Self::Seconds => "the download time limit must be from 1 second to 4 hours",
        })
    }
}

impl Error for DownloadLimitsError {}

impl DownloadLimits {
    /// The defaults: 4 GiB and 30 minutes.
    pub const DEFAULT: Self = Self {
        max_bytes: DEFAULT_DOWNLOAD_BYTES,
        max_seconds: DEFAULT_DOWNLOAD_SECONDS,
    };

    /// Limits a caller chose, each defaulting when absent.
    ///
    /// # Errors
    ///
    /// [`DownloadLimitsError`] for a value outside its range.
    pub fn new(
        max_bytes: Option<u64>,
        max_seconds: Option<u32>,
    ) -> Result<Self, DownloadLimitsError> {
        let max_bytes = max_bytes.unwrap_or(DEFAULT_DOWNLOAD_BYTES);
        let max_seconds = max_seconds.unwrap_or(DEFAULT_DOWNLOAD_SECONDS);
        if max_bytes == 0 || max_bytes > MAX_DOWNLOAD_BYTES {
            return Err(DownloadLimitsError::Bytes);
        }
        if max_seconds == 0 || max_seconds > MAX_DOWNLOAD_SECONDS {
            return Err(DownloadLimitsError::Seconds);
        }
        Ok(Self {
            max_bytes,
            max_seconds,
        })
    }

    /// Largest body accepted.
    #[must_use]
    pub const fn max_bytes(self) -> u64 {
        self.max_bytes
    }

    /// Whole-transfer deadline, in seconds.
    #[must_use]
    pub const fn max_seconds(self) -> u32 {
        self.max_seconds
    }
}

/// A host name or address as a fetch reached it: lower-case ASCII letters,
/// digits, `.`, `-`, `:`, `[` and `]`, at most [`MAX_HOST_NAME_BYTES`]. Only
/// the host is ever recorded, never a path or query.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct HostName(String);

/// A host name that is empty, too long or has a character a host cannot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostNameError;

impl fmt::Display for HostNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the host name is not a lower-case host")
    }
}

impl Error for HostNameError {}

impl HostName {
    /// Accepts a lower-case host.
    ///
    /// # Errors
    ///
    /// [`HostNameError`] for anything else.
    pub fn parse(text: &str) -> Result<Self, HostNameError> {
        if text.is_empty()
            || text.len() > MAX_HOST_NAME_BYTES
            || !text.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'-' | b':' | b'[' | b']')
            })
        {
            return Err(HostNameError);
        }
        Ok(Self(text.to_owned()))
    }

    /// The host's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How the address a fetch connected to was checked.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AddressCheck {
    /// `VSift` resolved the name and connected only to the public addresses
    /// it checked.
    Direct,
    /// A proxy from the environment resolved the name; `VSift` checked only
    /// that the host was not an address or name for an internal host
    /// (known limit L-208).
    ProxyResolved,
}

impl AddressCheck {
    /// Stable public and stored identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::ProxyResolved => "proxy_resolved",
        }
    }

    /// The check named by `identifier`, if any.
    #[must_use]
    pub fn from_identifier(identifier: &str) -> Option<Self> {
        [Self::Direct, Self::ProxyResolved]
            .into_iter()
            .find(|check| check.identifier() == identifier)
    }
}

/// Where a session's source came from when it was fetched (ADR 0028). A
/// source copied from a local file has no origin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceOrigin {
    /// A direct HTTPS link to a video file (#407).
    DirectHttps(DirectLinkOrigin),
}

impl SourceOrigin {
    /// Stable public and stored identifier of the kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::DirectHttps(_) => "direct_https",
        }
    }
}

/// The record of one direct-link fetch. It holds a digest of the link, never
/// the link: a link may carry a signed token in its query or its path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectLinkOrigin {
    link_sha256: Sha256Hex,
    requested_host: HostName,
    final_host: HostName,
    redirects: u8,
    address_check: AddressCheck,
    retrieved_at_unix_seconds: u64,
    bytes: u64,
}

/// An origin record whose values cannot belong together.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceOriginError;

impl fmt::Display for SourceOriginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the source origin record is not valid")
    }
}

impl Error for SourceOriginError {}

impl DirectLinkOrigin {
    /// Records one fetch.
    ///
    /// # Errors
    ///
    /// [`SourceOriginError`] for more than [`MAX_SOURCE_REDIRECTS`] redirects,
    /// no bytes or more than [`MAX_DOWNLOAD_BYTES`].
    pub fn new(
        link_sha256: Sha256Hex,
        (requested_host, final_host): (HostName, HostName),
        redirects: u8,
        address_check: AddressCheck,
        retrieved_at_unix_seconds: u64,
        bytes: u64,
    ) -> Result<Self, SourceOriginError> {
        if redirects > MAX_SOURCE_REDIRECTS || bytes == 0 || bytes > MAX_DOWNLOAD_BYTES {
            return Err(SourceOriginError);
        }
        Ok(Self {
            link_sha256,
            requested_host,
            final_host,
            redirects,
            address_check,
            retrieved_at_unix_seconds,
            bytes,
        })
    }

    /// SHA-256 of the link as given, after parsing and normalising it.
    #[must_use]
    pub const fn link_sha256(&self) -> &Sha256Hex {
        &self.link_sha256
    }

    /// The host the link named.
    #[must_use]
    pub const fn requested_host(&self) -> &HostName {
        &self.requested_host
    }

    /// The host the body came from, after redirects.
    #[must_use]
    pub const fn final_host(&self) -> &HostName {
        &self.final_host
    }

    /// Redirects followed.
    #[must_use]
    pub const fn redirects(&self) -> u8 {
        self.redirects
    }

    /// How the connected address was checked.
    #[must_use]
    pub const fn address_check(&self) -> AddressCheck {
        self.address_check
    }

    /// When the body was complete, in Unix seconds.
    #[must_use]
    pub const fn retrieved_at_unix_seconds(&self) -> u64 {
        self.retrieved_at_unix_seconds
    }

    /// Bytes received, the size of the session's source.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_is_checked_for_its_shape_and_never_printed() -> Result<(), SourceFetchReason> {
        let link = SourceLink::parse("HTTPS://videos.example.test/a.mp4?token=SECRET-1")?;
        assert_eq!(format!("{link:?}"), "SourceLink(<redacted>)");
        assert!(link.expose().ends_with("SECRET-1"));
        let long = format!("https://example.test/{}", "a".repeat(MAX_SOURCE_LINK_BYTES));
        for refused in [
            "",
            "https://",
            "http://example.test/a.mp4",
            "ftp://example.test/a.mp4",
            "https://example.test/a b.mp4",
            "https://example.test/\u{00e9}.mp4",
            "/home/example/video.mp4",
            long.as_str(),
        ] {
            assert_eq!(
                SourceLink::parse(refused),
                Err(SourceFetchReason::NotHttps),
                "{refused}"
            );
        }
        Ok(())
    }

    #[test]
    fn download_limits_default_and_stay_in_range() {
        assert_eq!(DownloadLimits::new(None, None), Ok(DownloadLimits::DEFAULT));
        assert_eq!(DownloadLimits::DEFAULT.max_bytes(), 4 * 1024 * 1024 * 1024);
        assert_eq!(DownloadLimits::DEFAULT.max_seconds(), 1_800);
        assert!(DownloadLimits::new(Some(MAX_DOWNLOAD_BYTES), Some(MAX_DOWNLOAD_SECONDS)).is_ok());
        assert_eq!(
            DownloadLimits::new(Some(MAX_DOWNLOAD_BYTES + 1), None),
            Err(DownloadLimitsError::Bytes)
        );
        assert_eq!(
            DownloadLimits::new(Some(0), None),
            Err(DownloadLimitsError::Bytes)
        );
        assert_eq!(
            DownloadLimits::new(None, Some(MAX_DOWNLOAD_SECONDS + 1)),
            Err(DownloadLimitsError::Seconds)
        );
        assert_eq!(
            DownloadLimits::new(None, Some(0)),
            Err(DownloadLimitsError::Seconds)
        );
    }

    #[test]
    fn reasons_have_unique_identifiers() {
        let mut identifiers: Vec<&str> = SourceFetchReason::ALL
            .iter()
            .map(|reason| reason.identifier())
            .collect();
        identifiers.sort_unstable();
        identifiers.dedup();
        assert_eq!(identifiers.len(), SourceFetchReason::ALL.len());
    }

    #[test]
    fn an_origin_holds_only_hosts_and_a_digest() -> Result<(), Box<dyn Error>> {
        let digest = Sha256Hex::parse("0".repeat(64)).map_err(|_| "digest")?;
        let origin = DirectLinkOrigin::new(
            digest.clone(),
            (
                HostName::parse("videos.example.test")?,
                HostName::parse("cdn.example.test")?,
            ),
            2,
            AddressCheck::Direct,
            1_792_587_792,
            1_024,
        )?;
        assert_eq!(origin.final_host().as_str(), "cdn.example.test");
        assert_eq!(SourceOrigin::DirectHttps(origin).kind(), "direct_https");
        assert!(
            DirectLinkOrigin::new(
                digest,
                (HostName::parse("a.test")?, HostName::parse("a.test")?),
                4,
                AddressCheck::Direct,
                0,
                1,
            )
            .is_err()
        );
        assert!(HostName::parse("Upper.Example").is_err());
        assert!(HostName::parse("a.test/path").is_err());
        assert!(HostName::parse("[::1]").is_ok());
        for check in [AddressCheck::Direct, AddressCheck::ProxyResolved] {
            assert_eq!(
                AddressCheck::from_identifier(check.identifier()),
                Some(check)
            );
        }
        Ok(())
    }
}
