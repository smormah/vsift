//! Telling why yt-dlp failed (ADR 0028 section 4.8).
//!
//! yt-dlp reports most causes only as text on standard error, and its exit
//! status does not tell them apart. [`classify_ytdlp_failure`] reads that text
//! only to choose a typed reason: it is never echoed, stored or logged. The
//! phrases are case-sensitive substrings of the pinned yt-dlp's own messages,
//! and the hosted phrase check (`tools/online_video_component_review.py`) finds
//! each in the pinned tag's source, so a re-pin that changes one fails there.

use vsift_domain::SourceFetchReason;

/// The most of yt-dlp's standard error that is read: 64 KiB.
pub const MAX_YTDLP_STDERR_BYTES: usize = 64 * 1024;

/// Each reason's phrases, in the order they are tried: `age_restricted`
/// comes before `login_required`, because yt-dlp's age message asks to sign in.
pub const YTDLP_PHRASES: [(SourceFetchReason, &[&str]); 12] = [
    (
        SourceFetchReason::AgeRestricted,
        &["Sign in to confirm your age", "age-restricted"],
    ),
    (
        SourceFetchReason::LoginRequired,
        &[
            "This video requires login",
            "--cookies-from-browser or --cookies for the authentication",
            "Join this channel to get access",
            "members-only content",
        ],
    ),
    (
        SourceFetchReason::BotCheck,
        &[
            "Sign in to confirm you\u{2019}re not a bot",
            "confirm you're not a bot",
        ],
    ),
    (
        SourceFetchReason::Drm,
        &["This video is DRM protected", "DRM protected"],
    ),
    (
        SourceFetchReason::RegionBlocked,
        &[
            "not available in your country",
            "geo restriction",
            "The uploader has not made this video available",
        ],
    ),
    (
        SourceFetchReason::Private,
        &["Private video", "This video is private"],
    ),
    (
        SourceFetchReason::Removed,
        &[
            "Video unavailable",
            "This video has been removed",
            "account associated with this video has been terminated",
        ],
    ),
    (SourceFetchReason::UnsupportedSite, &["Unsupported URL"]),
    (
        SourceFetchReason::ExtractorBroken,
        &[
            "Unable to extract",
            "please report this issue on",
            "Requested format is not available",
        ],
    ),
    (
        SourceFetchReason::JsRuntimeMissing,
        &[
            "No supported JavaScript runtime could be found",
            "n challenge solving failed",
            "JS runtime",
        ],
    ),
    (
        SourceFetchReason::ProxyAuth,
        &["407 Proxy Authentication Required"],
    ),
    (
        SourceFetchReason::Network,
        &[
            "Unable to download webpage",
            "Failed to resolve",
            "Connection refused",
            "timed out",
            "HTTP Error 403",
        ],
    ),
];

/// The typed reason yt-dlp's standard error names, or
/// [`SourceFetchReason::OnlineStepFailed`] when it names none. Text that is
/// not valid UTF-8 is read as far as it is.
#[must_use]
pub fn classify_ytdlp_failure(stderr: &[u8]) -> SourceFetchReason {
    let bounded = stderr.get(..MAX_YTDLP_STDERR_BYTES).unwrap_or(stderr);
    let text = String::from_utf8_lossy(bounded);
    YTDLP_PHRASES
        .iter()
        .find(|(_, phrases)| phrases.iter().any(|phrase| text.contains(phrase)))
        .map_or(SourceFetchReason::OnlineStepFailed, |(reason, _)| *reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_phrase_chooses_its_reason() {
        for (reason, phrases) in YTDLP_PHRASES {
            for phrase in phrases {
                let text = format!("ERROR: [site] abc: {phrase} and more\n");
                assert_eq!(classify_ytdlp_failure(text.as_bytes()), reason, "{phrase}");
            }
        }
    }

    #[test]
    fn age_comes_before_login() {
        let text = "ERROR: Sign in to confirm your age. Use --cookies-from-browser or --cookies for the authentication.";
        assert_eq!(
            classify_ytdlp_failure(text.as_bytes()),
            SourceFetchReason::AgeRestricted
        );
    }

    #[test]
    fn anything_else_is_a_failure_with_no_name() {
        assert_eq!(
            classify_ytdlp_failure(b"ERROR: something new\n"),
            SourceFetchReason::OnlineStepFailed
        );
        assert_eq!(
            classify_ytdlp_failure(b"\xff\xfe"),
            SourceFetchReason::OnlineStepFailed
        );
        assert_eq!(
            classify_ytdlp_failure(b""),
            SourceFetchReason::OnlineStepFailed
        );
    }

    #[test]
    fn phrases_are_matched_as_written() {
        assert_eq!(
            classify_ytdlp_failure(b"private video"),
            SourceFetchReason::OnlineStepFailed
        );
    }

    #[test]
    fn nothing_past_the_bound_is_read() {
        let mut text = vec![b' '; MAX_YTDLP_STDERR_BYTES];
        text.extend_from_slice(b"Private video");
        assert_eq!(
            classify_ytdlp_failure(&text),
            SourceFetchReason::OnlineStepFailed
        );
    }
}
