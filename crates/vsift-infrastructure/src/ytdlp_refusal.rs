//! Telling why yt-dlp failed (ADR 0028 section 4.8).
//!
//! yt-dlp reports most causes only as text on standard error, and its exit
//! status does not tell them apart. [`classify_ytdlp_failure`] reads that text
//! only to choose a typed reason: it is never echoed, stored or logged. A
//! phrase only chooses a reason and its remediation; it never accepts anything.
//!
//! [`YTDLP_PHRASES`] are case-sensitive substrings of the pinned yt-dlp's own
//! messages: the hosted phrase check (`tools/online_video_component_review.py`)
//! finds each in the pinned tag's source, so a re-pin that changes one fails
//! there. [`SITE_PHRASES`] are text a site or a proxy writes, which yt-dlp only
//! passes on, so no source holds them and no check notices when one changes:
//! a site that rewords one falls to a broader reason. The list is closed: an
//! addition needs an observed failure on record (ADR 0028's note of
//! 2026-11-01), and the check fails if one turns up in the source.

use vsift_domain::SourceFetchReason;

/// The most of yt-dlp's standard error that is read: 64 KiB.
pub const MAX_YTDLP_STDERR_BYTES: usize = 64 * 1024;

/// Each reason's checked phrases, in the order the reasons are tried:
/// `age_restricted` and `bot_check` come before `login_required`, because
/// yt-dlp adds its sign-in hint to any site message that says "sign in".
/// Within a reason these come before its [`SITE_PHRASES`].
pub const YTDLP_PHRASES: [(SourceFetchReason, &[&str]); 12] = [
    (
        SourceFetchReason::AgeRestricted,
        &["confirm your age", "age-restricted"],
    ),
    (SourceFetchReason::BotCheck, &[]),
    (
        SourceFetchReason::LoginRequired,
        &[
            "This video requires login",
            "--cookies-from-browser or --cookies for the authentication",
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
    (SourceFetchReason::ProxyAuth, &[]),
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

/// Text from a site or a proxy that yt-dlp passes on, by reason. Exactly
/// these six: see the module documentation. Line numbers are the pinned tag's.
pub const SITE_PHRASES: [(SourceFetchReason, &[&str]); 4] = [
    (
        SourceFetchReason::LoginRequired,
        // YouTube's `reason`, passed on at extractor/youtube/_video.py:4053-4076.
        &["Join this channel to get access", "members-only content"],
    ),
    (
        SourceFetchReason::BotCheck,
        // YouTube's `reason`, passed on at extractor/youtube/_video.py:4053-4076.
        &[
            "Sign in to confirm you\u{2019}re not a bot",
            "confirm you're not a bot",
        ],
    ),
    (
        SourceFetchReason::Removed,
        // YouTube's `reason` or `subreason`, as above.
        &["account associated with this video has been terminated"],
    ),
    (
        SourceFetchReason::ProxyAuth,
        // The proxy's status line, in the HTTP library's error, which
        // networking/_requests.py:344-345 and _urllib.py:434 wrap as the cause.
        &["407 Proxy Authentication Required"],
    ),
];

fn site_phrases(reason: SourceFetchReason) -> &'static [&'static str] {
    SITE_PHRASES
        .iter()
        .find(|(listed, _)| *listed == reason)
        .map_or(&[], |(_, phrases)| *phrases)
}

/// The typed reason yt-dlp's standard error names, or
/// [`SourceFetchReason::OnlineStepFailed`] when it names none. Text that is
/// not valid UTF-8 is read as far as it is.
#[must_use]
pub fn classify_ytdlp_failure(stderr: &[u8]) -> SourceFetchReason {
    let bounded = stderr.get(..MAX_YTDLP_STDERR_BYTES).unwrap_or(stderr);
    let text = String::from_utf8_lossy(bounded);
    YTDLP_PHRASES
        .iter()
        .find(|(reason, phrases)| {
            phrases
                .iter()
                .chain(site_phrases(*reason))
                .any(|phrase| text.contains(phrase))
        })
        .map_or(SourceFetchReason::OnlineStepFailed, |(reason, _)| *reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_phrase_chooses_its_reason() {
        for (reason, phrases) in YTDLP_PHRASES.into_iter().chain(SITE_PHRASES) {
            for phrase in phrases {
                let text = format!("ERROR: [site] abc: {phrase} and more\n");
                assert_eq!(classify_ytdlp_failure(text.as_bytes()), reason, "{phrase}");
            }
        }
    }

    #[test]
    fn the_site_list_is_closed_and_shares_nothing_with_the_checked_list() {
        let site: Vec<&str> = SITE_PHRASES
            .iter()
            .flat_map(|(_, phrases)| phrases.iter().copied())
            .collect();
        assert_eq!(site.len(), 6);
        for (_, phrases) in YTDLP_PHRASES {
            assert!(!phrases.iter().any(|phrase| site.contains(phrase)));
        }
        for (reason, _) in SITE_PHRASES {
            assert!(YTDLP_PHRASES.iter().any(|(listed, _)| *listed == reason));
        }
    }

    /// yt-dlp adds its sign-in hint to a site's message, so the hint alone
    /// must not decide when the message names something more exact.
    #[test]
    fn age_and_a_bot_check_come_before_login() {
        let hint = "Use --cookies-from-browser or --cookies for the authentication.";
        for (message, reason) in [
            (
                "Sign in to confirm your age",
                SourceFetchReason::AgeRestricted,
            ),
            (
                "Sign in to confirm you\u{2019}re not a bot",
                SourceFetchReason::BotCheck,
            ),
            ("Sign in to view this", SourceFetchReason::LoginRequired),
        ] {
            let text = format!("ERROR: [youtube] abc: {message}. {hint}");
            assert_eq!(classify_ytdlp_failure(text.as_bytes()), reason, "{message}");
        }
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
