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
use vsift_domain::{AddressCheck, DirectLinkOrigin, HostName, Sha256Hex, SourceOrigin};

/// One stored origin.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredOrigin {
    kind: String,
    link_sha256: String,
    requested_host: String,
    final_host: String,
    redirects: u8,
    address_check: String,
    retrieved_at_unix_seconds: u64,
    bytes: u64,
}

/// The stored form of `origin`.
pub(super) fn encode_origin(origin: &SourceOrigin) -> StoredOrigin {
    match origin {
        SourceOrigin::DirectHttps(direct) => StoredOrigin {
            kind: origin.kind().to_owned(),
            link_sha256: direct.link_sha256().as_str().to_owned(),
            requested_host: direct.requested_host().as_str().to_owned(),
            final_host: direct.final_host().as_str().to_owned(),
            redirects: direct.redirects(),
            address_check: direct.address_check().identifier().to_owned(),
            retrieved_at_unix_seconds: direct.retrieved_at_unix_seconds(),
            bytes: direct.bytes(),
        },
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
