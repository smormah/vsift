//! What npm's registry says about the four packages (P14 PR 8, ADR 0024
//! decisions A and B).
//!
//! The Release workflow's unprivileged `plan` job reads each package's public
//! metadata with an anonymous `GET` (no credential, no write) into one file
//! per package, plus the HTTP status it got. `publish-plan` reads those files
//! here; this tool contacts no network itself. What it learns is what a
//! stable plan must state (which dist-tags move from what to what) and what it
//! must check before `latest` moves (the candidate was published, nothing is
//! ahead of this version, this version is not already on npm under another
//! tag).
//!
//! The metadata is untrusted input, and it names the package's maintainers:
//! only dist-tags and per-version integrity strings are taken from it, the
//! rest is dropped here and never printed, so no plan, log or artifact
//! repeats it. A file that is too large, is not JSON or has another shape is
//! "unreadable", never guessed at.

use std::{collections::BTreeMap, fs, io::Read, path::Path};

use serde_json::Value;
use sha2::{Digest, Sha512};

/// The largest metadata file read. The real ones are a few kilobytes; a file
/// this large is not npm's answer for one of these packages.
const MAXIMUM_METADATA_BYTES: u64 = 8 * 1024 * 1024;

/// The most versions one package's metadata may list.
const MAXIMUM_VERSIONS: usize = 10_000;

/// What the plan job found, for all four packages, or that it did not look.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RegistryObservation {
    /// No registry directory was given (a local run, or a test).
    NotRead,
    /// One observation per package, in publication order.
    Read(Vec<PackageObservation>),
}

/// What was found for one package.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PackageObservation {
    /// The package's npm name.
    pub package: &'static str,
    /// What the registry answered.
    pub state: PackageState,
}

/// The registry's answer for one package.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PackageState {
    /// The status or the metadata could not be read; the reason names no
    /// content of the metadata.
    Unreadable(String),
    /// The registry answered 404: no such package.
    NotFound,
    /// The package exists.
    Found(PackageRecord),
}

/// The part of a package's metadata the plan uses.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct PackageRecord {
    /// The version the `latest` dist-tag names.
    pub latest: Option<String>,
    /// The version the `next` dist-tag names.
    pub next: Option<String>,
    /// Every published version and its `dist.integrity` (`sha512-<base64>`),
    /// when the registry gave one.
    pub versions: BTreeMap<String, Option<String>>,
}

impl PackageRecord {
    /// Whether `version` has been published.
    pub(crate) fn has_version(&self, version: &str) -> bool {
        self.versions.contains_key(version)
    }

    /// The integrity string npm holds for `version`, if it is published and
    /// the registry gave one.
    pub(crate) fn integrity_of(&self, version: &str) -> Option<&str> {
        self.versions.get(version).and_then(Option::as_deref)
    }
}

/// The file name stem the plan job uses for `package`: the name without its
/// `@`, with `/` replaced by `-` (the stem `npm pack` uses for tarballs).
pub(crate) fn file_stem(package: &str) -> String {
    package.trim_start_matches('@').replace('/', "-")
}

/// Reads the plan job's files for `packages` from `directory`: for each, the
/// metadata `<stem>.json` and the HTTP status `<stem>.status`.
pub(crate) fn read_directory(directory: &Path, packages: &[&'static str]) -> RegistryObservation {
    RegistryObservation::Read(
        packages
            .iter()
            .map(|package| PackageObservation {
                package,
                state: read_package(directory, package),
            })
            .collect(),
    )
}

fn read_package(directory: &Path, package: &str) -> PackageState {
    let stem = file_stem(package);
    let status = match read_limited(&directory.join(format!("{stem}.status")), 16) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).trim().to_owned(),
        Err(reason) => {
            return PackageState::Unreadable(format!("no HTTP status recorded: {reason}"));
        }
    };
    match status.as_str() {
        "404" => PackageState::NotFound,
        "200" => match read_limited(
            &directory.join(format!("{stem}.json")),
            MAXIMUM_METADATA_BYTES,
        ) {
            Ok(bytes) => {
                parse_metadata(&bytes).map_or_else(PackageState::Unreadable, PackageState::Found)
            }
            Err(reason) => {
                PackageState::Unreadable(format!("the metadata cannot be read: {reason}"))
            }
        },
        other => PackageState::Unreadable(format!(
            "the registry answered HTTP {}",
            if other.len() == 3 && other.bytes().all(|byte| byte.is_ascii_digit()) {
                other
            } else {
                "with no usable status"
            }
        )),
    }
}

/// Reads at most `limit` bytes of a file; a longer file is refused.
fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|error| error.kind().to_string())?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| error.kind().to_string())?;
    if u64::try_from(bytes.len()).map_or(true, |length| length > limit) {
        return Err(String::from("it is larger than expected"));
    }
    Ok(bytes)
}

/// Extracts the dist-tags and the version integrities from a package's
/// metadata, and nothing else.
pub(crate) fn parse_metadata(bytes: &[u8]) -> Result<PackageRecord, String> {
    let unreadable = |what: &str| format!("the registry's metadata is not usable: {what}");
    let value: Value = serde_json::from_slice(bytes).map_err(|_| unreadable("it is not JSON"))?;
    let Value::Object(object) = value else {
        return Err(unreadable("it is not an object"));
    };
    let tag = |name: &str| -> Result<Option<String>, String> {
        match object.get("dist-tags") {
            Some(Value::Object(tags)) => match tags.get(name) {
                None => Ok(None),
                Some(Value::String(version)) => Ok(Some(version.clone())),
                Some(_) => Err(unreadable("a dist-tag is not text")),
            },
            _ => Err(unreadable("it has no dist-tags")),
        }
    };
    let latest = tag("latest")?;
    let next = tag("next")?;
    let mut versions = BTreeMap::new();
    match object.get("versions") {
        Some(Value::Object(listed)) => {
            if listed.len() > MAXIMUM_VERSIONS {
                return Err(unreadable("it lists too many versions"));
            }
            for (version, entry) in listed {
                let integrity = entry
                    .get("dist")
                    .and_then(|dist| dist.get("integrity"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                versions.insert(version.clone(), integrity);
            }
        }
        _ => return Err(unreadable("it has no versions")),
    }
    Ok(PackageRecord {
        latest,
        next,
        versions,
    })
}

/// The `dist.integrity` npm records for a tarball: `sha512-` and the
/// standard base64 of its SHA-512 (what `openssl dgst -sha512 -binary |
/// base64` gives in the publish job).
pub(crate) fn npm_integrity(bytes: &[u8]) -> String {
    format!("sha512-{}", base64(&Sha512::digest(bytes)))
}

/// Standard base64 with padding (RFC 4648 section 4).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk.first().copied().unwrap_or_default();
        let second = chunk.get(1).copied();
        let third = chunk.get(2).copied();
        let group = (u32::from(first) << 16)
            | (u32::from(second.unwrap_or_default()) << 8)
            | u32::from(third.unwrap_or_default());
        let symbol = |shift: u32| {
            ALPHABET
                .get(usize::try_from((group >> shift) & 0x3f).unwrap_or_default())
                .copied()
                .map_or('=', char::from)
        };
        text.push(symbol(18));
        text.push(symbol(12));
        text.push(if second.is_some() { symbol(6) } else { '=' });
        text.push(if third.is_some() { symbol(0) } else { '=' });
    }
    text
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs};

    use super::{
        PackageRecord, PackageState, RegistryObservation, base64, file_stem, npm_integrity,
        parse_metadata, read_directory,
    };

    /// Metadata shaped like the registry's, with a maintainer that must never
    /// reach a plan: every name here is made up.
    const METADATA: &str = r#"{
        "_id": "vsift-cli", "name": "vsift-cli",
        "dist-tags": {"latest": "0.0.0", "next": "0.1.0"},
        "versions": {
            "0.0.0": {"maintainers": [{"name": "someone", "email": "someone@example.invalid"}],
                      "dist": {"integrity": "sha512-AAAA", "shasum": "ff"}},
            "0.1.0": {"dist": {"integrity": "sha512-BBBB"}},
            "0.2.0-rc.1": {"dist": {}}
        }
    }"#;

    #[test]
    fn base64_matches_the_rfc_4648_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected, "{input:?}");
        }
    }

    #[test]
    fn the_integrity_is_what_npm_records() {
        // `printf abc | openssl dgst -sha512 -binary | base64`
        assert_eq!(
            npm_integrity(b"abc"),
            "sha512-3a81oZNherrMQXNJriBBMRLm+k6JqX6iCp7u5ktV05ohkpkqJ0/BqDa6PCOj/uu9RU1EI2Q86A4qmslPpUyknw=="
        );
    }

    #[test]
    fn only_the_tags_and_integrities_are_taken_from_the_metadata() -> Result<(), Box<dyn Error>> {
        let record = parse_metadata(METADATA.as_bytes())?;
        assert_eq!(record.latest.as_deref(), Some("0.0.0"));
        assert_eq!(record.next.as_deref(), Some("0.1.0"));
        assert_eq!(record.integrity_of("0.1.0"), Some("sha512-BBBB"));
        assert!(record.has_version("0.2.0-rc.1"));
        assert_eq!(record.integrity_of("0.2.0-rc.1"), None);
        assert!(!record.has_version("0.3.0"));
        // Nothing of the maintainers survives into the record, whether they are
        // named per version or for the whole package, and a tag that is not set
        // stays unset instead of taking another field's text.
        assert!(!format!("{record:?}").contains("example.invalid"));
        let bare = parse_metadata(
            br#"{"maintainers": [{"name": "someone", "email": "someone@example.invalid"}],
                 "_npmUser": {"name": "someone", "email": "someone@example.invalid"},
                 "dist-tags": {}, "versions": {}}"#,
        )?;
        assert_eq!(bare, PackageRecord::default());
        assert!(!format!("{bare:?}").contains("example.invalid"));
        Ok(())
    }

    #[test]
    fn metadata_of_another_shape_is_unreadable_and_says_nothing_of_its_content() {
        for text in [
            "",
            "[]",
            "not json",
            r#"{"versions": {}}"#,
            r#"{"dist-tags": {}}"#,
            r#"{"dist-tags": {"latest": 1}, "versions": {}}"#,
            r#"{"dist-tags": [], "versions": {}}"#,
            r#"{"dist-tags": {}, "versions": []}"#,
        ] {
            let result = parse_metadata(text.as_bytes());
            assert!(
                matches!(&result, Err(reason) if reason.starts_with("the registry's metadata is not usable")),
                "{text:?}: {result:?}"
            );
        }
    }

    #[test]
    fn the_directory_is_read_per_package_with_its_status() -> Result<(), Box<dyn Error>> {
        let directory = std::env::temp_dir().join(format!(
            "vsift-release-registry-test-{}",
            std::process::id()
        ));
        if directory.exists() {
            fs::remove_dir_all(&directory)?;
        }
        fs::create_dir(&directory)?;
        fs::write(directory.join("vsift-cli.status"), "200\n")?;
        fs::write(directory.join("vsift-cli.json"), METADATA)?;
        fs::write(directory.join("vsift-win32-x64.status"), "404\n")?;
        fs::write(directory.join("vsift-linux-x64.status"), "503\n")?;
        fs::write(directory.join("vsift-darwin-arm64.status"), "200\n")?;
        // No metadata file for the last: unreadable, not a crash.
        let observation = read_directory(
            &directory,
            &[
                "vsift-cli",
                "@vsift/win32-x64",
                "@vsift/linux-x64",
                "@vsift/darwin-arm64",
            ],
        );
        let RegistryObservation::Read(packages) = observation else {
            return Err("the directory was not read".into());
        };
        let states: Vec<&PackageState> = packages.iter().map(|package| &package.state).collect();
        assert!(matches!(states.first(), Some(PackageState::Found(_))));
        assert_eq!(states.get(1), Some(&&PackageState::NotFound));
        assert!(
            matches!(states.get(2), Some(PackageState::Unreadable(reason)) if reason == "the registry answered HTTP 503")
        );
        assert!(matches!(states.get(3), Some(PackageState::Unreadable(_))));

        // A status that is not three digits is never repeated back.
        fs::write(directory.join("vsift-cli.status"), "<script>\n")?;
        let RegistryObservation::Read(again) = read_directory(&directory, &["vsift-cli"]) else {
            return Err("the directory was not read".into());
        };
        assert_eq!(
            again.first().map(|package| &package.state),
            Some(&PackageState::Unreadable(String::from(
                "the registry answered HTTP with no usable status"
            )))
        );
        // A missing status file is unreadable too.
        fs::remove_file(directory.join("vsift-cli.status"))?;
        let RegistryObservation::Read(missing) = read_directory(&directory, &["vsift-cli"]) else {
            return Err("the directory was not read".into());
        };
        assert!(matches!(
            missing.first().map(|package| &package.state),
            Some(PackageState::Unreadable(_))
        ));
        fs::remove_dir_all(&directory)?;
        Ok(())
    }

    #[test]
    fn file_stems_are_the_tarball_stems() {
        assert_eq!(file_stem("vsift-cli"), "vsift-cli");
        assert_eq!(file_stem("@vsift/win32-x64"), "vsift-win32-x64");
    }
}
