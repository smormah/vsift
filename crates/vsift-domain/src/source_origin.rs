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
    /// The page's video needs a sign-in, which `VSift` never does.
    LoginRequired,
    /// The page's video is age-restricted, which needs a sign-in.
    AgeRestricted,
    /// The site asked for proof that the caller is a person.
    BotCheck,
    /// The page's video is protected by digital rights management.
    Drm,
    /// The page's video is not offered in this region.
    RegionBlocked,
    /// The page's video is private.
    Private,
    /// The page's video was removed or is unavailable.
    Removed,
    /// yt-dlp has no way to read this site.
    UnsupportedSite,
    /// yt-dlp could not read this site as it is today.
    ExtractorBroken,
    /// The site needs a JavaScript runtime that is not available.
    JsRuntimeMissing,
    /// yt-dlp could not reach the site.
    Network,
    /// The link is a playlist or a channel, not one video.
    Playlist,
    /// The page's video is live, or a recording of a stream still being made.
    Live,
    /// The page's video is over four hours long, or states no length.
    TooLong,
    /// The page offers the video only by a means other than plain HTTPS.
    NotHttpsFormat,
    /// yt-dlp left something other than the files expected of it, or printed
    /// something other than what was asked.
    UnexpectedOutput,
    /// The online step failed for a cause `VSift` cannot name.
    OnlineStepFailed,
    /// The page's video comes as separate audio and video, and no verified
    /// `FFmpeg` is set up to join them.
    MediaToolsMissing,
}

impl SourceFetchReason {
    /// Every reason, in declaration order.
    pub const ALL: [Self; 37] = [
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
        Self::LoginRequired,
        Self::AgeRestricted,
        Self::BotCheck,
        Self::Drm,
        Self::RegionBlocked,
        Self::Private,
        Self::Removed,
        Self::UnsupportedSite,
        Self::ExtractorBroken,
        Self::JsRuntimeMissing,
        Self::Network,
        Self::Playlist,
        Self::Live,
        Self::TooLong,
        Self::NotHttpsFormat,
        Self::UnexpectedOutput,
        Self::OnlineStepFailed,
        Self::MediaToolsMissing,
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
            Self::LoginRequired => "login_required",
            Self::AgeRestricted => "age_restricted",
            Self::BotCheck => "bot_check",
            Self::Drm => "drm",
            Self::RegionBlocked => "region_blocked",
            Self::Private => "private",
            Self::Removed => "removed",
            Self::UnsupportedSite => "unsupported_site",
            Self::ExtractorBroken => "extractor_broken",
            Self::JsRuntimeMissing => "js_runtime_missing",
            Self::Network => "network",
            Self::Playlist => "playlist",
            Self::Live => "live",
            Self::TooLong => "too_long",
            Self::NotHttpsFormat => "not_https_format",
            Self::UnexpectedOutput => "unexpected_output",
            Self::OnlineStepFailed => "online_step_failed",
            Self::MediaToolsMissing => "media_tools_missing",
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
    /// A public video page, fetched through yt-dlp (#409).
    PlatformPage(PageOrigin),
}

impl SourceOrigin {
    /// Stable public and stored identifier of the kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::DirectHttps(_) => "direct_https",
            Self::PlatformPage(_) => "platform_page",
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

/// Longest provider name a page's record keeps.
pub const MAX_PAGE_PROVIDER_BYTES: usize = 64;
/// Longest video id a page's record keeps in clear.
pub const MAX_PAGE_VIDEO_ID_BYTES: usize = 128;
/// Longest tool version a page's record keeps.
pub const MAX_PAGE_TOOL_VERSION_BYTES: usize = 64;

/// The name yt-dlp gives the site it read (its `extractor_key`): 1 to
/// [`MAX_PAGE_PROVIDER_BYTES`] ASCII letters and digits.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PageProvider(String);

impl PageProvider {
    /// Accepts a provider name.
    ///
    /// # Errors
    ///
    /// [`SourceOriginError`] for any other text: it came from yt-dlp, so it
    /// is untrusted.
    pub fn parse(text: &str) -> Result<Self, SourceOriginError> {
        if text.is_empty()
            || text.len() > MAX_PAGE_PROVIDER_BYTES
            || !text.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(SourceOriginError);
        }
        Ok(Self(text.to_owned()))
    }

    /// The name's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How a page's video is named in its record. An unlisted video's id is a
/// capability token, so the id is kept in clear only when the site reports
/// the video public; otherwise only its SHA-256 is kept.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum PageVideoId {
    /// The site reports the video public.
    Public(String),
    /// Anything else: only the digest of the id.
    Digest(Sha256Hex),
}

impl PageVideoId {
    /// The record of a video id: in clear only when `public` and it has the
    /// shape of an id, otherwise `digest`, the SHA-256 of `id`.
    #[must_use]
    pub fn recorded(id: &str, digest: Sha256Hex, public: bool) -> Self {
        if public && Self::is_id_shaped(id) {
            Self::Public(id.to_owned())
        } else {
            Self::Digest(digest)
        }
    }

    /// Whether `id` may be kept in clear: 1 to
    /// [`MAX_PAGE_VIDEO_ID_BYTES`] ASCII letters, digits, `-`, `_` and `.`.
    #[must_use]
    pub fn is_id_shaped(id: &str) -> bool {
        !id.is_empty()
            && id.len() <= MAX_PAGE_VIDEO_ID_BYTES
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    }
}

/// A tool's version as a page's record keeps it: 1 to
/// [`MAX_PAGE_TOOL_VERSION_BYTES`] printable ASCII characters.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PageToolVersion(String);

impl PageToolVersion {
    /// Accepts a version.
    ///
    /// # Errors
    ///
    /// [`SourceOriginError`] for any other text.
    pub fn parse(text: &str) -> Result<Self, SourceOriginError> {
        if text.is_empty()
            || text.len() > MAX_PAGE_TOOL_VERSION_BYTES
            || !text.bytes().all(|byte| (0x20..0x7f).contains(&byte))
        {
            return Err(SourceOriginError);
        }
        Ok(Self(text.to_owned()))
    }

    /// The version's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Which yt-dlp or JavaScript runtime fetched a page.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum PageToolRecord {
    /// A version `VSift` installed from its reviewed catalogue.
    Managed {
        /// The catalogue's version key.
        catalogue_version: PageToolVersion,
    },
    /// A file the user supplied: nobody here reviewed it.
    UserManaged {
        /// The first line of its version output.
        version: PageToolVersion,
        /// The SHA-256 of the file.
        sha256: Sha256Hex,
    },
}

/// Whether a page's video was one file or a joined pair of streams.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PageJoin {
    /// One file, moved into the session's source slot.
    SingleFile,
    /// Separate audio and video, joined by a stream copy.
    AudioVideo,
}

impl PageJoin {
    /// Stable public and stored identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::SingleFile => "single_file",
            Self::AudioVideo => "audio_video",
        }
    }

    /// The value named by `identifier`, if any.
    #[must_use]
    pub fn from_identifier(identifier: &str) -> Option<Self> {
        [Self::SingleFile, Self::AudioVideo]
            .into_iter()
            .find(|join| join.identifier() == identifier)
    }
}

/// The record of one page fetched through yt-dlp. It holds a digest of the
/// link and never a title, uploader, description, upload date, format URL or
/// header: those are untrusted text and other people's personal data, and an
/// upload date is not a recording time (ADR 0028 section 5).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageOrigin {
    provider: PageProvider,
    video_id: PageVideoId,
    link_sha256: Sha256Hex,
    yt_dlp: PageToolRecord,
    js_runtime: Option<PageToolRecord>,
    through_proxy: bool,
    retrieved_at_unix_seconds: u64,
    bytes: u64,
    joined: PageJoin,
}

impl PageOrigin {
    /// Records one fetched page. `tools` are the yt-dlp and the runtime it
    /// ran with, if any; `through_proxy` says whether a proxy from the
    /// environment was handed to it.
    ///
    /// # Errors
    ///
    /// [`SourceOriginError`] for no bytes or more than [`MAX_DOWNLOAD_BYTES`].
    pub fn new(
        (provider, video_id): (PageProvider, PageVideoId),
        link_sha256: Sha256Hex,
        tools: (PageToolRecord, Option<PageToolRecord>),
        through_proxy: bool,
        retrieved_at_unix_seconds: u64,
        (bytes, joined): (u64, PageJoin),
    ) -> Result<Self, SourceOriginError> {
        if bytes == 0 || bytes > MAX_DOWNLOAD_BYTES {
            return Err(SourceOriginError);
        }
        let (yt_dlp, js_runtime) = tools;
        Ok(Self {
            provider,
            video_id,
            link_sha256,
            yt_dlp,
            js_runtime,
            through_proxy,
            retrieved_at_unix_seconds,
            bytes,
            joined,
        })
    }

    /// The site yt-dlp read.
    #[must_use]
    pub const fn provider(&self) -> &PageProvider {
        &self.provider
    }

    /// The video's id, or its digest.
    #[must_use]
    pub const fn video_id(&self) -> &PageVideoId {
        &self.video_id
    }

    /// SHA-256 of the link as given, after parsing and normalising it.
    #[must_use]
    pub const fn link_sha256(&self) -> &Sha256Hex {
        &self.link_sha256
    }

    /// The yt-dlp that fetched the page.
    #[must_use]
    pub const fn yt_dlp(&self) -> &PageToolRecord {
        &self.yt_dlp
    }

    /// The JavaScript runtime it ran with, if any.
    #[must_use]
    pub const fn js_runtime(&self) -> Option<&PageToolRecord> {
        self.js_runtime.as_ref()
    }

    /// Whether a proxy from the environment made yt-dlp's connections, so
    /// the proxy resolved the names (known limits L-208 and L-209).
    #[must_use]
    pub const fn through_proxy(&self) -> bool {
        self.through_proxy
    }

    /// When the source was complete, in Unix seconds.
    #[must_use]
    pub const fn retrieved_at_unix_seconds(&self) -> u64 {
        self.retrieved_at_unix_seconds
    }

    /// The size of the session's source.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Whether the source was one file or a joined pair.
    #[must_use]
    pub const fn joined(&self) -> PageJoin {
        self.joined
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

    #[test]
    fn a_page_record_keeps_an_id_in_clear_only_when_public_and_id_shaped()
    -> Result<(), Box<dyn Error>> {
        let digest = Sha256Hex::parse("1".repeat(64)).map_err(|_| "digest")?;
        assert_eq!(
            PageVideoId::recorded("abc_DEF-1.2", digest.clone(), true),
            PageVideoId::Public(String::from("abc_DEF-1.2"))
        );
        for (id, public) in [
            ("abc", false),
            ("", true),
            ("a b", true),
            ("a/b", true),
            ("<script>", true),
        ] {
            assert_eq!(
                PageVideoId::recorded(id, digest.clone(), public),
                PageVideoId::Digest(digest.clone()),
                "{id}"
            );
        }
        let long = "a".repeat(MAX_PAGE_VIDEO_ID_BYTES + 1);
        assert!(!PageVideoId::is_id_shaped(&long));
        Ok(())
    }

    #[test]
    fn untrusted_provider_and_version_text_is_refused() {
        assert!(PageProvider::parse("Youtube1").is_ok());
        for refused in ["", "You Tube", "generic:html", "\u{00e9}"] {
            assert!(PageProvider::parse(refused).is_err(), "{refused}");
        }
        assert!(PageProvider::parse(&"a".repeat(MAX_PAGE_PROVIDER_BYTES + 1)).is_err());
        assert!(PageToolVersion::parse("deno 2.9.7 (stable)").is_ok());
        assert!(PageToolVersion::parse("").is_err());
        assert!(PageToolVersion::parse("2026\n").is_err());
        assert!(PageToolVersion::parse(&"1".repeat(MAX_PAGE_TOOL_VERSION_BYTES + 1)).is_err());
    }

    #[test]
    fn a_page_origin_is_bounded_like_a_download() -> Result<(), Box<dyn Error>> {
        let digest = Sha256Hex::parse("2".repeat(64)).map_err(|_| "digest")?;
        let tool = PageToolRecord::Managed {
            catalogue_version: PageToolVersion::parse("yt-dlp-2026.08.19-linux-x86_64")?,
        };
        let build = |bytes| {
            PageOrigin::new(
                (
                    PageProvider::parse("Generic")?,
                    PageVideoId::Digest(digest.clone()),
                ),
                digest.clone(),
                (tool.clone(), None),
                false,
                1_792_587_792,
                (bytes, PageJoin::AudioVideo),
            )
        };
        let origin = build(1_024)?;
        assert_eq!(origin.joined().identifier(), "audio_video");
        assert_eq!(SourceOrigin::PlatformPage(origin).kind(), "platform_page");
        assert!(build(0).is_err());
        assert!(build(MAX_DOWNLOAD_BYTES + 1).is_err());
        for join in [PageJoin::SingleFile, PageJoin::AudioVideo] {
            assert_eq!(PageJoin::from_identifier(join.identifier()), Some(join));
        }
        Ok(())
    }
}
