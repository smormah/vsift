//! The stored form of a fetched source's origin (ADR 0028).
//!
//! Written once, at activation, as an optional member of the session's
//! lifecycle record and copied unchanged into every later generation; a
//! session whose source is a local file's copy writes nothing, so its record
//! is what earlier releases wrote. A release before `0.3.0` reads a session
//! that has the member as damaged (the L-044 trade-off). It holds a digest of
//! the link and the hosts, never the link. Decoding rebuilds it through the
//! domain constructors, so a modified record is an integrity failure.

use serde::{Deserialize, Serialize};
use vsift_application::SessionStorageError;
use vsift_domain::{
    AddressCheck, DirectLinkOrigin, HostName, PageJoin, PageOrigin, PageProvider, PageToolRecord,
    PageToolVersion, PageVideoId, Sha256Hex, SourceOrigin,
};

/// One stored origin. Each form names its `kind` and refuses a member it does
/// not know, so the two cannot be mistaken for each other.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub(super) enum StoredOrigin {
    /// A direct link (#407).
    Direct(StoredDirectOrigin),
    /// A page fetched through yt-dlp (#409).
    Page(StoredPageOrigin),
}

/// The stored form of a direct link's origin.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredDirectOrigin {
    kind: String,
    link_sha256: String,
    requested_host: String,
    final_host: String,
    redirects: u8,
    address_check: String,
    retrieved_at_unix_seconds: u64,
    bytes: u64,
}

/// The stored form of a page's origin.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredPageOrigin {
    kind: String,
    provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    video_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    video_id_sha256: Option<String>,
    link_sha256: String,
    yt_dlp: StoredPageTool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    js_runtime: Option<StoredPageTool>,
    through_proxy: bool,
    retrieved_at_unix_seconds: u64,
    bytes: u64,
    joined: String,
}

/// The stored form of a page's tool: a catalogue version, or a user's file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredPageTool {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    catalogue_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
}

fn encode_page_tool(tool: &PageToolRecord) -> StoredPageTool {
    match tool {
        PageToolRecord::Managed { catalogue_version } => StoredPageTool {
            catalogue_version: Some(catalogue_version.as_str().to_owned()),
            version: None,
            sha256: None,
        },
        PageToolRecord::UserManaged { version, sha256 } => StoredPageTool {
            catalogue_version: None,
            version: Some(version.as_str().to_owned()),
            sha256: Some(sha256.as_str().to_owned()),
        },
    }
}

fn decode_page_tool(stored: &StoredPageTool) -> Result<PageToolRecord, SessionStorageError> {
    let integrity = |_| SessionStorageError::IntegrityFailure;
    match (&stored.catalogue_version, &stored.version, &stored.sha256) {
        (Some(catalogue_version), None, None) => Ok(PageToolRecord::Managed {
            catalogue_version: PageToolVersion::parse(catalogue_version).map_err(integrity)?,
        }),
        (None, Some(version), Some(sha256)) => Ok(PageToolRecord::UserManaged {
            version: PageToolVersion::parse(version).map_err(integrity)?,
            sha256: Sha256Hex::parse(sha256.clone())
                .map_err(|_| SessionStorageError::IntegrityFailure)?,
        }),
        _ => Err(SessionStorageError::IntegrityFailure),
    }
}

fn decode_page_origin(
    stored: &StoredPageOrigin,
    source_bytes: u64,
) -> Result<SourceOrigin, SessionStorageError> {
    let integrity = |_| SessionStorageError::IntegrityFailure;
    if stored.kind != "platform_page" || stored.bytes != source_bytes {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let video_id = match (&stored.video_id, &stored.video_id_sha256) {
        (Some(id), None) if PageVideoId::is_id_shaped(id) => PageVideoId::Public(id.clone()),
        (None, Some(digest)) => PageVideoId::Digest(
            Sha256Hex::parse(digest.clone()).map_err(|_| SessionStorageError::IntegrityFailure)?,
        ),
        _ => return Err(SessionStorageError::IntegrityFailure),
    };
    let joined =
        PageJoin::from_identifier(&stored.joined).ok_or(SessionStorageError::IntegrityFailure)?;
    let page = PageOrigin::new(
        (
            PageProvider::parse(&stored.provider).map_err(integrity)?,
            video_id,
        ),
        Sha256Hex::parse(stored.link_sha256.clone())
            .map_err(|_| SessionStorageError::IntegrityFailure)?,
        (
            decode_page_tool(&stored.yt_dlp)?,
            stored.js_runtime.as_ref().map(decode_page_tool).transpose()?,
        ),
        stored.through_proxy,
        stored.retrieved_at_unix_seconds,
        (stored.bytes, joined),
    )
    .map_err(integrity)?;
    Ok(SourceOrigin::PlatformPage(page))
}

/// The stored form of `origin`.
pub(super) fn encode_origin(origin: &SourceOrigin) -> StoredOrigin {
    match origin {
        SourceOrigin::DirectHttps(direct) => StoredOrigin::Direct(StoredDirectOrigin {
            kind: origin.kind().to_owned(),
            link_sha256: direct.link_sha256().as_str().to_owned(),
            requested_host: direct.requested_host().as_str().to_owned(),
            final_host: direct.final_host().as_str().to_owned(),
            redirects: direct.redirects(),
            address_check: direct.address_check().identifier().to_owned(),
            retrieved_at_unix_seconds: direct.retrieved_at_unix_seconds(),
            bytes: direct.bytes(),
        }),
        SourceOrigin::PlatformPage(page) => {
            let (video_id, video_id_sha256) = match page.video_id() {
                PageVideoId::Public(id) => (Some(id.clone()), None),
                PageVideoId::Digest(digest) => (None, Some(digest.as_str().to_owned())),
            };
            StoredOrigin::Page(StoredPageOrigin {
                kind: origin.kind().to_owned(),
                provider: page.provider().as_str().to_owned(),
                video_id,
                video_id_sha256,
                link_sha256: page.link_sha256().as_str().to_owned(),
                yt_dlp: encode_page_tool(page.yt_dlp()),
                js_runtime: page.js_runtime().map(encode_page_tool),
                through_proxy: page.through_proxy(),
                retrieved_at_unix_seconds: page.retrieved_at_unix_seconds(),
                bytes: page.bytes(),
                joined: page.joined().identifier().to_owned(),
            })
        }
    }
}

/// The origin a lifecycle record stores, validated, for a source of
/// `source_bytes`.
///
/// # Errors
///
/// [`SessionStorageError::IntegrityFailure`] for an unknown kind or check,
/// a value the domain refuses, or a size that is not the source's.
pub(super) fn decode_origin(
    stored: &StoredOrigin,
    source_bytes: u64,
) -> Result<SourceOrigin, SessionStorageError> {
    let stored = match stored {
        StoredOrigin::Direct(stored) => stored,
        StoredOrigin::Page(stored) => return decode_page_origin(stored, source_bytes),
    };
    let integrity = |_| SessionStorageError::IntegrityFailure;
    if stored.kind != "direct_https" || stored.bytes != source_bytes {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let address_check = AddressCheck::from_identifier(&stored.address_check)
        .ok_or(SessionStorageError::IntegrityFailure)?;
    let direct = DirectLinkOrigin::new(
        Sha256Hex::parse(stored.link_sha256.clone())
            .map_err(|_| SessionStorageError::IntegrityFailure)?,
        (
            HostName::parse(&stored.requested_host).map_err(integrity)?,
            HostName::parse(&stored.final_host).map_err(integrity)?,
        ),
        stored.redirects,
        address_check,
        stored.retrieved_at_unix_seconds,
        stored.bytes,
    )
    .map_err(|_| SessionStorageError::IntegrityFailure)?;
    Ok(SourceOrigin::DirectHttps(direct))
}

/// Refuses an activation whose origin, if any, does not decode.
///
/// # Errors
///
/// As [`decode_origin`].
pub(super) fn validate_origin(
    stored: Option<&StoredOrigin>,
    source_bytes: u64,
) -> Result<(), SessionStorageError> {
    stored.map_or(Ok(()), |stored| {
        decode_origin(stored, source_bytes).map(|_| ())
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    fn origin() -> Result<SourceOrigin, Box<dyn Error>> {
        Ok(SourceOrigin::DirectHttps(DirectLinkOrigin::new(
            Sha256Hex::parse("a".repeat(64)).map_err(|_| "digest")?,
            (
                HostName::parse("videos.example.test")?,
                HostName::parse("cdn.example.test")?,
            ),
            1,
            AddressCheck::ProxyResolved,
            1_792_587_792,
            4_096,
        )?))
    }

    #[test]
    fn an_origin_round_trips_and_names_no_link() -> Result<(), Box<dyn Error>> {
        let origin = origin()?;
        let json = serde_json::to_string(&encode_origin(&origin))?;
        assert_eq!(
            json,
            format!(
                r#"{{"kind":"direct_https","link_sha256":"{}","requested_host":"videos.example.test","final_host":"cdn.example.test","redirects":1,"address_check":"proxy_resolved","retrieved_at_unix_seconds":1792587792,"bytes":4096}}"#,
                "a".repeat(64)
            )
        );
        let read: StoredOrigin = serde_json::from_str(&json)?;
        assert_eq!(decode_origin(&read, 4_096)?, origin);
        Ok(())
    }

    fn page_origin(public: bool) -> Result<SourceOrigin, Box<dyn Error>> {
        let digest = Sha256Hex::parse("b".repeat(64)).map_err(|_| "digest")?;
        Ok(SourceOrigin::PlatformPage(PageOrigin::new(
            (
                PageProvider::parse("Generic")?,
                PageVideoId::recorded("abc-1", digest.clone(), public),
            ),
            digest.clone(),
            (
                PageToolRecord::Managed {
                    catalogue_version: PageToolVersion::parse("yt-dlp-2026.08.19-linux-x86_64")?,
                },
                Some(PageToolRecord::UserManaged {
                    version: PageToolVersion::parse("deno 2.9.7")?,
                    sha256: digest,
                }),
            ),
            true,
            1_792_587_792,
            (4_096, PageJoin::AudioVideo),
        )?))
    }

    #[test]
    fn a_page_origin_round_trips_and_its_id_is_kept_as_it_was_recorded()
    -> Result<(), Box<dyn Error>> {
        for public in [true, false] {
            let origin = page_origin(public)?;
            let json = serde_json::to_string(&encode_origin(&origin))?;
            assert_eq!(json.contains("abc-1"), public, "{json}");
            let read: StoredOrigin = serde_json::from_str(&json)?;
            assert_eq!(decode_origin(&read, 4_096)?, origin);
        }
        Ok(())
    }

    #[test]
    fn a_modified_page_origin_is_an_integrity_failure() -> Result<(), Box<dyn Error>> {
        let json = serde_json::to_string(&encode_origin(&page_origin(true)?))?;
        for (from, to) in [
            (r#""kind":"platform_page""#, r#""kind":"direct_https""#),
            (r#""provider":"Generic""#, r#""provider":"a b""#),
            (r#""video_id":"abc-1""#, r#""video_id":"a b""#),
            (r#""joined":"audio_video""#, r#""joined":"other""#),
            (
                r#""catalogue_version":"yt-dlp-2026.08.19-linux-x86_64""#,
                r#""version":"2026.08.19""#,
            ),
        ] {
            assert_ne!(json, json.replace(from, to), "{from}");
            let read: StoredOrigin = serde_json::from_str(&json.replace(from, to))?;
            assert_eq!(
                decode_origin(&read, 4_096),
                Err(SessionStorageError::IntegrityFailure),
                "{to}"
            );
        }
        let read: StoredOrigin = serde_json::from_str(&json)?;
        assert_eq!(
            decode_origin(&read, 1),
            Err(SessionStorageError::IntegrityFailure),
            "the size must be the source's"
        );
        assert!(
            serde_json::from_str::<StoredOrigin>(
                &json.replace(r#""bytes":4096"#, r#""bytes":4096,"title":"a title""#)
            )
            .is_err(),
            "a stored title is refused"
        );
        Ok(())
    }

    #[test]
    fn a_modified_origin_is_an_integrity_failure() -> Result<(), Box<dyn Error>> {
        let json = serde_json::to_string(&encode_origin(&origin()?))?;
        for (from, to) in [
            (r#""kind":"direct_https""#, r#""kind":"platform_page""#),
            (
                r#""address_check":"proxy_resolved""#,
                r#""address_check":"none""#,
            ),
            (r#""redirects":1"#, r#""redirects":9"#),
            (
                r#""final_host":"cdn.example.test""#,
                r#""final_host":"cdn.example.test/a.mp4""#,
            ),
        ] {
            let read: StoredOrigin = serde_json::from_str(&json.replace(from, to))?;
            assert_eq!(
                decode_origin(&read, 4_096),
                Err(SessionStorageError::IntegrityFailure),
                "{to}"
            );
        }
        let read: StoredOrigin = serde_json::from_str(&json)?;
        assert_eq!(
            decode_origin(&read, 4_097),
            Err(SessionStorageError::IntegrityFailure),
            "the size must be the source's"
        );
        assert!(
            serde_json::from_str::<StoredOrigin>(&json.replace(
                r#""bytes":4096"#,
                r#""bytes":4096,"link":"https://videos.example.test/a.mp4""#
            ))
            .is_err(),
            "a stored link is refused"
        );
        Ok(())
    }
}
