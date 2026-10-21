//! Online sources on the wire (ADR 0028): the `origin` of a session whose
//! source was fetched from a link, and the fixed-prose remediation of
//! `SOURCE_FETCH_FAILED`.
//!
//! An origin names a digest of the link and the hosts it reached, never the
//! link: a link may carry a signed token in its query or its path. Every
//! remediation names the online step, says local files still work and never
//! echoes the link, the server's text or proxy credentials.

use serde::Serialize;
use vsift_domain::{SourceFetchReason, SourceOrigin};

use crate::envelope::utc_seconds_rfc3339;

/// The `origin` member of `ingest` and `session status` data for a source
/// fetched from a link; absent for a local file's copy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OriginData {
    kind: &'static str,
    link_sha256: String,
    requested_host: String,
    final_host: String,
    redirects: u8,
    address_check: &'static str,
    retrieved_at: String,
    bytes: u64,
}

impl OriginData {
    /// Presents an origin; `None` when its time cannot be written (after
    /// the year 9999).
    #[must_use]
    pub fn new(origin: &SourceOrigin) -> Option<Self> {
        match origin {
            SourceOrigin::DirectHttps(direct) => Some(Self {
                kind: origin.kind(),
                link_sha256: direct.link_sha256().as_str().to_owned(),
                requested_host: direct.requested_host().as_str().to_owned(),
                final_host: direct.final_host().as_str().to_owned(),
                redirects: direct.redirects(),
                address_check: direct.address_check().identifier(),
                retrieved_at: utc_seconds_rfc3339(direct.retrieved_at_unix_seconds())?,
                bytes: direct.bytes(),
            }),
        }
    }
}

/// The opening of every `SOURCE_FETCH_FAILED` remediation.
#[cfg(test)]
const ONLINE_STEP: &str =
    "Fetching the video link failed; nothing was opened and local video files are not affected.";

/// Fixed-prose remediation of a link that could not be fetched, by reason.
#[must_use]
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
        SourceFetchReason::WebPage => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The link is a web page, not a video file (reason web_page). VSift fetches direct links to an MP4, MOV, MKV or WebM file. Ask the user for the video file's own link or for the file; do not fetch the page or the video yourself."
        }
        SourceFetchReason::NotAVideo => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. What the link served is not an MP4, MOV, Matroska or WebM file (reason not_a_video). Ask the user for a link to the video file, or for the file."
        }
        SourceFetchReason::Truncated => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. The connection ended before the whole video arrived (reason truncated). Run the command again, or ask the user for the file."
        }
        SourceFetchReason::DisabledByPolicy => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. Fetching links is turned off on this computer (reason disabled_by_policy). Ask the user for the file."
        }
        SourceFetchReason::StrictIsolation => {
            "Fetching the video link failed; nothing was opened and local video files are not affected. This host runs with strict isolation, which allows no network (reason strict_isolation). Fetch the video on a host that may use the network, then give this host the file."
        }
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
