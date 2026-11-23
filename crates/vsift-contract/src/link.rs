//! Online sources on the wire (ADR 0028): the `origin` of a session whose
//! source was fetched from a link, and the fixed-prose remediation of
//! `SOURCE_FETCH_FAILED`.
//!
//! An origin names a digest of the link and the hosts it reached, never the
//! link: a link may carry a signed token in its query or its path. Every
//! remediation names the online step, says local files still work and never
//! echoes the link, the server's text or proxy credentials.

use serde::Serialize;
use vsift_domain::{PageToolRecord, PageVideoId, SourceFetchReason, SourceOrigin};

use crate::envelope::utc_seconds_rfc3339;

/// The `origin` member of `ingest` and `session status` data for a source
/// fetched from a link; absent for a local file's copy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum OriginData {
    /// A direct link to a video file.
    Direct(DirectOriginData),
    /// A public video page, fetched through yt-dlp.
    Page(Box<PageOriginData>),
}

/// The origin of a direct link.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DirectOriginData {
    kind: &'static str,
    link_sha256: String,
    requested_host: String,
    final_host: String,
    redirects: u8,
    address_check: &'static str,
    retrieved_at: String,
    bytes: u64,
}

/// The origin of a page. It names no title, uploader, description, upload
/// date, format URL or header.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PageOriginData {
    kind: &'static str,
    provider: String,
    /// The video's id, only when the site reports the video public.
    #[serde(skip_serializing_if = "Option::is_none")]
    video_id: Option<String>,
    /// The SHA-256 of the id of any other video: an unlisted video's id is
    /// a capability token.
    #[serde(skip_serializing_if = "Option::is_none")]
    video_id_sha256: Option<String>,
    link_sha256: String,
    yt_dlp: PageToolData,
    /// Absent when yt-dlp ran with no JavaScript runtime.
    #[serde(skip_serializing_if = "Option::is_none")]
    js_runtime: Option<PageToolData>,
    /// `proxy_resolved` when a proxy from the environment made yt-dlp's
    /// connections; absent otherwise, because `VSift` checks no address
    /// yt-dlp connects to (known limit L-209).
    #[serde(skip_serializing_if = "Option::is_none")]
    address_check: Option<&'static str>,
    retrieved_at: String,
    bytes: u64,
    joined: &'static str,
}

/// Which tool fetched a page: `managed` with its catalogue version, or
/// `user_managed` with its version line and digest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PageToolData {
    source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    catalogue_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
}

impl PageToolData {
    fn new(tool: &PageToolRecord) -> Self {
        match tool {
            PageToolRecord::Managed { catalogue_version } => Self {
                source: "managed",
                catalogue_version: Some(catalogue_version.as_str().to_owned()),
                version: None,
                sha256: None,
            },
            PageToolRecord::UserManaged { version, sha256 } => Self {
                source: "user_managed",
                catalogue_version: None,
                version: Some(version.as_str().to_owned()),
                sha256: Some(sha256.as_str().to_owned()),
            },
        }
    }
}

impl OriginData {
    /// Presents an origin; `None` when its time cannot be written (after
    /// the year 9999).
    #[must_use]
    pub fn new(origin: &SourceOrigin) -> Option<Self> {
        match origin {
            SourceOrigin::DirectHttps(direct) => Some(Self::Direct(DirectOriginData {
                kind: origin.kind(),
                link_sha256: direct.link_sha256().as_str().to_owned(),
                requested_host: direct.requested_host().as_str().to_owned(),
                final_host: direct.final_host().as_str().to_owned(),
                redirects: direct.redirects(),
                address_check: direct.address_check().identifier(),
                retrieved_at: utc_seconds_rfc3339(direct.retrieved_at_unix_seconds())?,
                bytes: direct.bytes(),
            })),
            SourceOrigin::PlatformPage(page) => {
                let (video_id, video_id_sha256) = match page.video_id() {
                    PageVideoId::Public(id) => (Some(id.clone()), None),
                    PageVideoId::Digest(digest) => (None, Some(digest.as_str().to_owned())),
                };
                Some(Self::Page(Box::new(PageOriginData {
                    kind: origin.kind(),
                    provider: page.provider().as_str().to_owned(),
                    video_id,
                    video_id_sha256,
                    link_sha256: page.link_sha256().as_str().to_owned(),
                    yt_dlp: PageToolData::new(page.yt_dlp()),
                    js_runtime: page.js_runtime().map(PageToolData::new),
                    address_check: page.through_proxy().then_some("proxy_resolved"),
                    retrieved_at: utc_seconds_rfc3339(page.retrieved_at_unix_seconds())?,
                    bytes: page.bytes(),
                    joined: page.joined().identifier(),
                })))
            }
        }
    }
}

/// The opening of every `SOURCE_FETCH_FAILED` remediation.
#[cfg(test)]
const ONLINE_STEP: &str =
    "Fetching the video link failed; nothing was opened and local video files are not affected.";

/// Fixed-prose remediation of a link that could not be fetched, by reason.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one arm of fixed prose for each published reason"
)]
pub const fn source_fetch_remediation(reason: SourceFetchReason) -> &'static str {
    match reason {
        SourceFetchReason::NotHttps => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. VSift fetches only https:// links to a video file (reason not_https). Ask the user for an https link to the file, or for the file itself, and open a file with: vsift ingest <video> --json."
        }
        SourceFetchReason::CredentialsInLink => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The link carries a user name or password, which VSift never sends (reason credentials_in_link). Ask the user for a public link, or to download the video and give you the file."
        }
        SourceFetchReason::PortNotAllowed => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. VSift fetches links on the standard HTTPS port only, and this link or a redirect names another port (reason port_not_allowed). Ask the user to download the video and give you the file."
        }
        SourceFetchReason::AddressRefused => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The link, or a redirect, points at a private, local or internal address, which VSift never fetches (reason address_refused). If the video is on the user's own network, ask the user to download it and give you the file; do not look for another way to reach the address."
        }
        SourceFetchReason::Tls => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The secure connection could not be established or its certificate was not trusted (reason tls). Tell the user; a network that inspects HTTPS needs its certificate trusted by the system, which is the user's or their administrator's choice."
        }
        SourceFetchReason::RedirectPolicy => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The server redirected more than three times or to a link that is not https (reason redirect_policy). Ask the user for the video file's direct https link, or for the file."
        }
        SourceFetchReason::HttpStatus => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The server did not answer with the video (reason http_status): the link may be wrong, expired or need a sign-in, which VSift never does. Ask the user to check the link, or for the file."
        }
        SourceFetchReason::ProxyAuth => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The network's proxy asked for credentials (reason proxy_auth). Tell the user; proxy credentials are set in the proxy environment variables by the user or their administrator, never by you."
        }
        SourceFetchReason::ProxyUnusable => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The proxy named by the HTTPS_PROXY or ALL_PROXY environment variable is not a proxy address VSift can use, so nothing was sent rather than going around it (reason proxy_unusable). Tell the user; the variable is set by the user or their administrator, as a proxy URL or as host:port, and never by you."
        }
        SourceFetchReason::Offline => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The server could not be reached or its name not resolved (reason offline). Check the network with the user and run the command again, or ask for the file."
        }
        SourceFetchReason::Stalled => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. No data arrived for 30 seconds (reason stalled). Run the command again later, or ask the user for the file."
        }
        SourceFetchReason::Deadline => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The download took longer than its time limit (reason deadline). Run it again with a longer --max-download-seconds (up to 14400), or ask the user for the file."
        }
        SourceFetchReason::Size => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The video is larger than the download size limit (reason size). Run it again with a larger --max-download-bytes (up to 21474836480, 20 GiB) if the user agrees to the download, or ask for the file."
        }
        SourceFetchReason::Partial => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The server sent only part of the video (reason partial). Ask the user for another link to the file, or for the file."
        }
        SourceFetchReason::Compressed => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The server sent the video compressed, which VSift does not accept (reason compressed). Ask the user for another link to the file, or for the file."
        }
        SourceFetchReason::WebPage => WEB_PAGE_REMEDIATION,
        SourceFetchReason::NotAVideo => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. What the link served is not an MP4, MOV, Matroska or WebM file (reason not_a_video). Ask the user for a link to the video file, or for the file."
        }
        SourceFetchReason::Truncated => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The connection ended before the whole video arrived (reason truncated). Run the command again, or ask the user for the file."
        }
        SourceFetchReason::StrictIsolation => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. This host runs with strict isolation, which allows no network (reason strict_isolation). Fetch the video on a host that may use the network, then give this host the file."
        }
        SourceFetchReason::LoginRequired => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The site asks for a sign-in to show this video, and VSift never signs in or uses cookies (reason login_required). Ask the user to download the video and give you the file."
        }
        SourceFetchReason::AgeRestricted => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The site shows this video only after a sign-in that confirms age, and VSift never signs in (reason age_restricted). Ask the user to download the video and give you the file."
        }
        SourceFetchReason::BotCheck => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The site asked for proof that a person is asking, which is common from shared and cloud addresses (reason bot_check). VSift does not get around such checks and neither should you. Ask the user to download the video and give you the file."
        }
        SourceFetchReason::Drm => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The video is protected against copying, and VSift does not get around that (reason drm). Ask the user for another copy they may use."
        }
        SourceFetchReason::RegionBlocked => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The site does not offer this video in this region (reason region_blocked). Ask the user for the file; do not look for a way around the restriction."
        }
        SourceFetchReason::Private => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The video is private (reason private). Ask the user to download it, if they may, and give you the file."
        }
        SourceFetchReason::Removed => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The site says the video was removed or is unavailable (reason removed). Ask the user to check the link, or for the file."
        }
        SourceFetchReason::UnsupportedSite => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. yt-dlp has no way to read this site (reason unsupported_site). Ask the user for the video file's own link, or for the file."
        }
        SourceFetchReason::ExtractorBroken => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. yt-dlp could not read this site as it is today, which happens when a site changes (reason extractor_broken). With a managed yt-dlp, run vsift setup plan again for a newer reviewed version; a yt-dlp the user supplied is theirs to update. This is not a VSift defect, so do not report it as one. Until then, ask the user for the file."
        }
        SourceFetchReason::JsRuntimeMissing => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The site needs a JavaScript runtime that yt-dlp could not use (reason js_runtime_missing). With a yt-dlp the user supplied, they register Deno with vsift setup configure js-runtime --executable <absolute-path>; a yt-dlp installed by VSift uses its own, so run vsift setup check. Or ask the user for the file."
        }
        SourceFetchReason::Network => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. yt-dlp could not reach the site or was turned away (reason network). Check the network with the user and run the command again later, or ask for the file."
        }
        SourceFetchReason::Playlist => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The link is a playlist or a channel, and VSift fetches one video (reason playlist). Ask the user for the link to the one video they mean."
        }
        SourceFetchReason::Live => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The video is a live stream, or the recording of one that is not finished, and VSift fetches finished videos only (reason live). Ask the user for a finished recording, or for the file."
        }
        SourceFetchReason::TooLong => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The video is over four hours long or the site states no length (reason too_long). Ask the user for the file or for a shorter part."
        }
        SourceFetchReason::NotHttpsFormat => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The site offers this video only as a stream or by a means other than plain HTTPS, in a form VSift does not fetch (reason not_https_format). Ask the user to download the video and give you the file."
        }
        SourceFetchReason::UnexpectedOutput => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. yt-dlp did not produce what it was asked for, so everything it wrote was removed (reason unexpected_output). Tell the user; if it is a yt-dlp they supplied, it may be out of date. Or ask for the file."
        }
        SourceFetchReason::MediaToolsMissing => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The site offers this video as separate audio and video, and joining them needs FFmpeg, which is not set up (reason media_tools_missing). Run vsift setup check, then run the command again."
        }
        SourceFetchReason::OnlineStepFailed => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. yt-dlp failed for a cause VSift cannot name (reason online_step_failed). Run the command again later, or ask the user for the file."
        }
    }
}

/// The remediation of a web page when no yt-dlp is available. On the target
/// with a managed yt-dlp it names the optional group; elsewhere the user
/// supplies the tool (ADR 0028 sections 4.1 and 4.7).
const WEB_PAGE_REMEDIATION: &str = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    "Fetching the video link failed; nothing was opened and local video files are not affected. The link is a web page, not a video file, and no usable yt-dlp is set up to read it (reason web_page). Tell the user what the optional online_links group does and that each site's terms apply, then ask before adding it: vsift setup plan --profile desktop --include online_links shows it. Or ask for the video file's own link or for the file; do not fetch the page or the video yourself."
} else {
    "Fetching the video link failed; nothing was opened and local video files are not affected. The link is a web page, not a video file, and no usable yt-dlp is set up to read it (reason web_page). On this system the user supplies yt-dlp and registers it with vsift setup configure yt-dlp --executable <absolute-path>, and each site's terms apply; ask them before doing that. Or ask for the video file's own link or for the file; do not fetch the page or the video yourself."
};

/// The `vsift` arguments a remediation offers for `reason`, and whether they
/// need the user's confirmation. Only `web_page` offers any: adding the
/// optional component is the user's decision. Elsewhere than the target with
/// a managed yt-dlp the step needs a path only the user knows, so the prose
/// carries it and no command is offered.
#[must_use]
pub const fn source_fetch_command(reason: SourceFetchReason) -> Option<&'static [&'static str]> {
    match reason {
        SourceFetchReason::WebPage if cfg!(all(target_os = "linux", target_arch = "x86_64")) => {
            Some(&[
                "setup",
                "plan",
                "--profile",
                "desktop",
                "--include",
                "online_links",
            ])
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_remediation_names_the_step_its_reason_and_local_files() {
        for reason in SourceFetchReason::ALL {
            let text = source_fetch_remediation(reason);
            assert!(text.starts_with(ONLINE_STEP), "{reason:?}");
            assert!(
                text.contains(&format!("(reason {})", reason.identifier())),
                "{reason:?}"
            );
            assert!(text.len() <= 1_024, "{reason:?}");
            assert!(!text.contains("http://"), "{reason:?}");
        }
    }
}
