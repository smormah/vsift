//! What `VSift` reads from yt-dlp's metadata (ADR 0028 section 4.7, phase 1).
//!
//! yt-dlp's standard output is untrusted: it is a program reading a stranger's
//! web page. [`parse_ytdlp_info`] types the few fields the checks need,
//! ignores the rest and refuses anything the acquisition may not fetch. It
//! keeps no title, uploader, description, date or format URL, so none of them
//! can reach a record or a log. It has its own fuzz target, `ytdlp_info_json`.

use serde::Deserialize;
use vsift_domain::{DownloadLimits, PageJoin, PageProvider, SourceFetchReason};

/// The most standard output phase 1 may print: 8 MiB.
pub const MAX_YTDLP_INFO_BYTES: usize = 8 * 1024 * 1024;
/// Longest video a page may hold: 4 hours, in seconds.
const MAX_DURATION_SECONDS: f64 = 4.0 * 60.0 * 60.0;
/// Longest format id that may name a file.
const MAX_FORMAT_ID_BYTES: usize = 64;
/// Most selected formats a video can have: video and audio.
const PAIR: usize = 2;

/// One format yt-dlp selected, as far as `VSift` reads it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedFormat {
    /// yt-dlp's id for the format; it appears in a stream's file name.
    pub format_id: String,
    /// The file extension: `mp4`, `m4a` or `webm`.
    pub ext: String,
    /// The size the site states, if it states one.
    pub declared_bytes: Option<u64>,
}

/// What phase 1 established about the page's one video.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct YtDlpVideo {
    /// The site yt-dlp read.
    pub provider: PageProvider,
    /// The site's id for the video; it is untrusted and may be a capability
    /// token, so it is recorded only by `PageVideoId::recorded`'s rule.
    pub video_id: String,
    /// Whether the site reports the video public.
    pub public: bool,
    /// The one selected format, or the video and the audio to join.
    pub formats: Vec<SelectedFormat>,
}

impl YtDlpVideo {
    /// Whether the video comes as one file or as a pair to join.
    #[must_use]
    pub fn join(&self) -> PageJoin {
        if self.formats.len() == PAIR {
            PageJoin::AudioVideo
        } else {
            PageJoin::SingleFile
        }
    }
}

/// The fields read, each absent or `null` when the site gives none; a field
/// of the wrong type refuses the whole output.
#[derive(Debug, Default, Deserialize)]
struct RawInfo {
    #[serde(rename = "_type", default)]
    kind: Option<String>,
    #[serde(default)]
    entries: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    extractor_key: Option<String>,
    #[serde(default)]
    is_live: Option<bool>,
    #[serde(default)]
    was_live: Option<bool>,
    #[serde(default)]
    live_status: Option<String>,
    #[serde(default)]
    availability: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    format_id: Option<String>,
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default)]
    ext: Option<String>,
    #[serde(default)]
    filesize: Option<f64>,
    #[serde(default)]
    filesize_approx: Option<f64>,
    #[serde(default)]
    requested_formats: Option<Vec<RawFormat>>,
}

#[derive(Debug, Deserialize)]
struct RawFormat {
    #[serde(default)]
    format_id: Option<String>,
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default)]
    ext: Option<String>,
    #[serde(default)]
    filesize: Option<f64>,
    #[serde(default)]
    filesize_approx: Option<f64>,
}

/// Parses phase 1's standard output and applies the checks of section 4.7.
///
/// # Errors
///
/// The typed reason of the first check broken; output that is not the JSON
/// of one object is [`SourceFetchReason::UnexpectedOutput`]. A video the live
/// filter passes over is still printed, with no format selected, so the
/// status is judged before the formats.
pub fn parse_ytdlp_info(
    stdout: &[u8],
    limits: DownloadLimits,
) -> Result<YtDlpVideo, SourceFetchReason> {
    if stdout.len() > MAX_YTDLP_INFO_BYTES {
        return Err(SourceFetchReason::UnexpectedOutput);
    }
    let text = stdout.trim_ascii();
    // Serde would also read a struct from an array.
    if text.first() != Some(&b'{') {
        return Err(SourceFetchReason::UnexpectedOutput);
    }
    let raw: RawInfo =
        serde_json::from_slice(text).map_err(|_| SourceFetchReason::UnexpectedOutput)?;
    check_single_video(&raw)?;
    check_finished(&raw)?;
    check_availability(raw.availability.as_deref())?;
    check_duration(raw.duration)?;
    let formats = selected_formats(&raw)?;
    check_declared_size(&formats, limits)?;
    let provider = raw
        .extractor_key
        .as_deref()
        .and_then(|key| PageProvider::parse(key).ok())
        .ok_or(SourceFetchReason::UnexpectedOutput)?;
    let video_id = raw
        .id
        .filter(|id| !id.is_empty())
        .ok_or(SourceFetchReason::UnexpectedOutput)?;
    Ok(YtDlpVideo {
        provider,
        video_id,
        public: raw.availability.as_deref() == Some("public"),
        formats,
    })
}

fn check_single_video(raw: &RawInfo) -> Result<(), SourceFetchReason> {
    let video = raw.kind.as_deref().is_none_or(|kind| kind == "video");
    if !video || raw.entries.is_some() {
        return Err(SourceFetchReason::Playlist);
    }
    Ok(())
}

fn check_finished(raw: &RawInfo) -> Result<(), SourceFetchReason> {
    // A site may not report the status at all: only a reported one refuses.
    let live = raw.is_live == Some(true)
        || raw.was_live == Some(true)
        || raw
            .live_status
            .as_deref()
            .is_some_and(|status| status != "not_live");
    if live {
        Err(SourceFetchReason::Live)
    } else {
        Ok(())
    }
}

fn check_availability(availability: Option<&str>) -> Result<(), SourceFetchReason> {
    match availability {
        Some("private") => Err(SourceFetchReason::Private),
        Some("needs_auth" | "premium_only" | "subscriber_only") => {
            Err(SourceFetchReason::LoginRequired)
        }
        _ => Ok(()),
    }
}

fn check_duration(duration: Option<f64>) -> Result<(), SourceFetchReason> {
    match duration {
        Some(seconds) if seconds.is_finite() && seconds <= MAX_DURATION_SECONDS => Ok(()),
        _ => Err(SourceFetchReason::TooLong),
    }
}

/// Whether a format id may go into phase 2's selection and a file's name:
/// `[A-Za-z0-9][A-Za-z0-9._-]{0,63}`, so it can neither leave the folder nor
/// change the selection. yt-dlp does not check this for us.
#[must_use]
pub fn is_safe_format_id(id: &str) -> bool {
    id.len() <= MAX_FORMAT_ID_BYTES
        && id
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn check_format(
    format_id: Option<&str>,
    protocol: Option<&str>,
    ext: Option<&str>,
    sizes: (Option<f64>, Option<f64>),
) -> Result<SelectedFormat, SourceFetchReason> {
    if protocol != Some("https") {
        return Err(SourceFetchReason::NotHttpsFormat);
    }
    let ext = ext
        .filter(|ext| matches!(*ext, "mp4" | "m4a" | "webm"))
        .ok_or(SourceFetchReason::NotHttpsFormat)?;
    let format_id = format_id
        .filter(|id| is_safe_format_id(id))
        .ok_or(SourceFetchReason::UnexpectedOutput)?;
    Ok(SelectedFormat {
        format_id: format_id.to_owned(),
        ext: ext.to_owned(),
        declared_bytes: declared_bytes(sizes.0)?.or(declared_bytes(sizes.1)?),
    })
}

/// A stated size in bytes: absent, or a whole number that fits. A site's
/// fractional estimate is rounded up.
fn declared_bytes(value: Option<f64>) -> Result<Option<u64>, SourceFetchReason> {
    let Some(value) = value else {
        return Ok(None);
    };
    if !value.is_finite() || value < 0.0 {
        return Err(SourceFetchReason::UnexpectedOutput);
    }
    // Anything from 2^64 up is over every limit; the cast saturates there.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked finite and not negative; a float-to-integer cast saturates"
    )]
    Ok(Some(value.ceil() as u64))
}

fn selected_formats(raw: &RawInfo) -> Result<Vec<SelectedFormat>, SourceFetchReason> {
    match raw.requested_formats.as_deref() {
        Some([video, audio]) => [video, audio]
            .into_iter()
            .map(|format| {
                check_format(
                    format.format_id.as_deref(),
                    format.protocol.as_deref(),
                    format.ext.as_deref(),
                    (format.filesize, format.filesize_approx),
                )
            })
            .collect(),
        Some(_) => Err(SourceFetchReason::UnexpectedOutput),
        None => Ok(vec![check_format(
            raw.format_id.as_deref(),
            raw.protocol.as_deref(),
            raw.ext.as_deref(),
            (raw.filesize, raw.filesize_approx),
        )?]),
    }
}

fn check_declared_size(
    formats: &[SelectedFormat],
    limits: DownloadLimits,
) -> Result<(), SourceFetchReason> {
    let total = formats
        .iter()
        .filter_map(|format| format.declared_bytes)
        .fold(0_u64, u64::saturating_add);
    if total > limits.max_bytes() {
        Err(SourceFetchReason::Size)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn single() -> Value {
        json!({
            "_type": "video",
            "id": "abc-1",
            "extractor_key": "Generic",
            "title": "A title that must go nowhere",
            "uploader": "Someone",
            "upload_date": "20260101",
            "availability": "public",
            "live_status": "not_live",
            "is_live": false,
            "was_live": false,
            "duration": 61.5,
            "format_id": "18",
            "protocol": "https",
            "ext": "mp4",
            "url": "https://cdn.example.test/signed?token=SENTINEL",
            "filesize": 1024,
            "formats": [{"format_id": "18", "url": "https://cdn.example.test/x"}]
        })
    }

    fn pair() -> Value {
        let mut value = single();
        value["requested_formats"] = json!([
            {"format_id": "137", "protocol": "https", "ext": "mp4", "filesize": 1024},
            {"format_id": "140", "protocol": "https", "ext": "m4a", "filesize_approx": 512.5}
        ]);
        value
    }

    fn parse(value: &Value) -> Result<YtDlpVideo, SourceFetchReason> {
        let bytes = serde_json::to_vec(value).map_err(|_| SourceFetchReason::OnlineStepFailed)?;
        parse_ytdlp_info(&bytes, DownloadLimits::DEFAULT)
    }

    fn with(key: &str, member: Value) -> Value {
        let mut value = single();
        value[key] = member;
        value
    }

    #[test]
    fn one_file_and_a_pair_are_read_and_nothing_untrusted_is_kept() -> Result<(), SourceFetchReason>
    {
        let video = parse(&single())?;
        assert_eq!(video.join(), PageJoin::SingleFile);
        assert_eq!(video.provider.as_str(), "Generic");
        assert_eq!(video.video_id, "abc-1");
        assert!(video.public);
        assert!(!format!("{video:?}").contains("SENTINEL"));
        assert!(!format!("{video:?}").contains("A title"));
        let video = parse(&pair())?;
        assert_eq!(video.join(), PageJoin::AudioVideo);
        assert_eq!(video.formats[1].format_id, "140");
        assert_eq!(video.formats[1].declared_bytes, Some(513));
        Ok(())
    }

    /// A site that reports no live status is not refused for it: only a
    /// reported status refuses.
    #[test]
    fn an_unreported_status_is_not_live() {
        let mut value = single();
        for key in ["live_status", "is_live", "was_live", "availability"] {
            if let Some(object) = value.as_object_mut() {
                object.remove(key);
            }
        }
        assert!(parse(&value).is_ok_and(|video| !video.public));
    }

    #[test]
    fn what_may_not_be_fetched_is_refused_with_its_reason() {
        let cases = [
            (
                with("_type", json!("playlist")),
                SourceFetchReason::Playlist,
            ),
            (with("entries", json!([])), SourceFetchReason::Playlist),
            (with("is_live", json!(true)), SourceFetchReason::Live),
            (with("was_live", json!(true)), SourceFetchReason::Live),
            (
                with("live_status", json!("is_upcoming")),
                SourceFetchReason::Live,
            ),
            (
                with("live_status", json!("post_live")),
                SourceFetchReason::Live,
            ),
            (
                with("availability", json!("private")),
                SourceFetchReason::Private,
            ),
            (
                with("availability", json!("needs_auth")),
                SourceFetchReason::LoginRequired,
            ),
            (
                with("availability", json!("premium_only")),
                SourceFetchReason::LoginRequired,
            ),
            (
                with("availability", json!("subscriber_only")),
                SourceFetchReason::LoginRequired,
            ),
            (
                with("duration", json!(14_400.5)),
                SourceFetchReason::TooLong,
            ),
            (with("duration", Value::Null), SourceFetchReason::TooLong),
            (
                with("protocol", json!("m3u8_native")),
                SourceFetchReason::NotHttpsFormat,
            ),
            (
                with("protocol", json!("http")),
                SourceFetchReason::NotHttpsFormat,
            ),
            (with("ext", json!("flv")), SourceFetchReason::NotHttpsFormat),
            (
                with("filesize", json!(5_000_000_000_u64)),
                SourceFetchReason::Size,
            ),
            (
                with("extractor_key", json!("a b")),
                SourceFetchReason::UnexpectedOutput,
            ),
            (with("id", json!("")), SourceFetchReason::UnexpectedOutput),
            (
                with("filesize", json!(-1)),
                SourceFetchReason::UnexpectedOutput,
            ),
            (
                with("format_id", json!("../x")),
                SourceFetchReason::UnexpectedOutput,
            ),
            (
                with("format_id", json!(".hidden")),
                SourceFetchReason::UnexpectedOutput,
            ),
            (
                with("format_id", json!("-x")),
                SourceFetchReason::UnexpectedOutput,
            ),
            (
                with("format_id", json!("18,19")),
                SourceFetchReason::UnexpectedOutput,
            ),
            (
                with("format_id", json!("18[height=1]")),
                SourceFetchReason::UnexpectedOutput,
            ),
            (
                with("format_id", json!("a".repeat(65))),
                SourceFetchReason::UnexpectedOutput,
            ),
        ];
        for (value, reason) in cases {
            assert_eq!(parse(&value), Err(reason), "{value}");
        }
    }

    /// What the live filter passes over reaches `VSift` with no format.
    #[test]
    fn a_live_video_with_no_format_selected_is_live() {
        let mut value = with("is_live", json!(true));
        if let Some(object) = value.as_object_mut() {
            for key in ["format_id", "protocol", "ext"] {
                object.remove(key);
            }
        }
        assert_eq!(parse(&value), Err(SourceFetchReason::Live));
    }

    #[test]
    fn a_pair_is_held_to_the_same_rules_and_its_sizes_are_added() {
        let mut value = pair();
        value["requested_formats"][1]["protocol"] = json!("http_dash_segments");
        assert_eq!(parse(&value), Err(SourceFetchReason::NotHttpsFormat));
        let mut value = pair();
        value["requested_formats"][0]["filesize"] = json!(3_000_000_000_u64);
        value["requested_formats"][1]["filesize_approx"] = json!(3_000_000_000_u64);
        assert_eq!(parse(&value), Err(SourceFetchReason::Size));
        let mut value = pair();
        value["requested_formats"] = json!([{"format_id": "1", "protocol": "https", "ext": "mp4"}]);
        assert_eq!(parse(&value), Err(SourceFetchReason::UnexpectedOutput));
    }

    #[test]
    fn output_that_is_not_one_object_is_refused() {
        for text in [
            b"".as_slice(),
            b"not json",
            b"[]",
            b"{\"id\":",
            b"{\"id\":1}",
            b"{\"duration\":\"long\"}",
        ] {
            assert_eq!(
                parse_ytdlp_info(text, DownloadLimits::DEFAULT),
                Err(SourceFetchReason::UnexpectedOutput),
                "{text:?}"
            );
        }
        assert_eq!(
            parse_ytdlp_info(b" null\n", DownloadLimits::DEFAULT),
            Err(SourceFetchReason::UnexpectedOutput)
        );
        assert_eq!(
            parse_ytdlp_info(
                &vec![b' '; MAX_YTDLP_INFO_BYTES + 1],
                DownloadLimits::DEFAULT
            ),
            Err(SourceFetchReason::UnexpectedOutput)
        );
    }
}
