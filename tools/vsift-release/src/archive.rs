//! The layout, writer and reader of one release archive.
//!
//! An archive is `vsift-<version>-<target>.tar.gz` holding one top directory
//! of the same name with exactly: the `vsift` executable, the three licence
//! files, `THIRD-PARTY-NOTICES`, the target's `CycloneDX` SBOM
//! `vsift.cdx.json`, and the agent skill under `skills/vsift/`, byte-identical
//! to the repository's (ADR 0023: section 1 and decision H7).
//!
//! The bytes are a function of the inputs alone: entries in path order, every
//! timestamp the source commit's time, owner and group 0 without names,
//! modes 0755 for directories and the executable and 0644 otherwise, and a
//! gzip header without a name or time. Two runs over the same commit produce
//! the same archive, whichever runner packages it.

use std::{
    collections::BTreeMap,
    fmt,
    io::{self, Read, Write},
};

use flate2::{Compression, GzBuilder, read::GzDecoder};
use tar::{Archive, Builder, EntryType, Header};

use crate::target::ReleaseTarget;

/// The licence files every archive carries, from the repository root.
pub(crate) const LICENCE_FILES: [&str; 3] = ["LICENSE", "LICENSE-APACHE", "LICENSE-MIT"];

/// The notices file's name inside the archive (ADR 0023).
pub(crate) const NOTICES_NAME: &str = "THIRD-PARTY-NOTICES";

/// The SBOM's name inside the archive.
pub(crate) const SBOM_NAME: &str = "vsift.cdx.json";

/// Where the skill sits in the repository and in the archive.
pub(crate) const SKILL_DIRECTORY: &str = "skills/vsift";

/// The first line `notices.hbs` writes; a notices file without it was not
/// produced by the reviewed template.
const NOTICES_FIRST_LINE: &str = "THIRD-PARTY NOTICES";

/// The upper bound on an archive's uncompressed content when it is read
/// back, far above today's size (about 8 MiB) and far below a decompression
/// bomb's.
const MAXIMUM_UNPACKED_BYTES: u64 = 256 * 1024 * 1024;

const EXECUTABLE_MODE: u32 = 0o755;
const DIRECTORY_MODE: u32 = 0o755;
const FILE_MODE: u32 = 0o644;

/// The contents of one archive, before it is written.
pub(crate) struct ArchiveContents {
    /// The `vsift` executable built for the target.
    pub executable: Vec<u8>,
    /// Each licence file, by its name in [`LICENCE_FILES`].
    pub licences: BTreeMap<String, Vec<u8>>,
    /// The cargo-about output for the target.
    pub notices: Vec<u8>,
    /// The cargo-cyclonedx output for the target.
    pub sbom: Vec<u8>,
    /// Every skill file, by its path relative to [`SKILL_DIRECTORY`] with
    /// `/` separators.
    pub skill: BTreeMap<String, Vec<u8>>,
}

/// Why an archive could not be written or does not hold what it must.
#[derive(Debug)]
pub(crate) enum ArchiveError {
    /// The executable is not of the target's format and architecture.
    ForeignExecutable(ReleaseTarget),
    /// A licence file listed in [`LICENCE_FILES`] is missing or empty.
    MissingLicence(&'static str),
    /// The notices file does not start as the reviewed template writes it.
    UnexpectedNotices,
    /// The SBOM is not a `CycloneDX` JSON document describing `vsift-cli`.
    UnexpectedSbom(&'static str),
    /// The skill has no `SKILL.md`, or a path that is not plain and relative.
    UnexpectedSkill(String),
    /// Writing or reading the archive failed.
    Io(io::Error),
    /// The archive read back differs from what it must hold.
    Mismatch(String),
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ForeignExecutable(target) => write!(
                formatter,
                "the executable is not a {target} executable of the expected architecture"
            ),
            Self::MissingLicence(name) => {
                write!(formatter, "licence file {name} is missing or empty")
            }
            Self::UnexpectedNotices => write!(
                formatter,
                "the notices file does not start with {NOTICES_FIRST_LINE:?}"
            ),
            Self::UnexpectedSbom(reason) => write!(formatter, "the SBOM is not usable: {reason}"),
            Self::UnexpectedSkill(reason) => write!(formatter, "the skill is not usable: {reason}"),
            Self::Io(error) => write!(formatter, "archive input or output failed: {error}"),
            Self::Mismatch(reason) => write!(formatter, "the archive is not as packaged: {reason}"),
        }
    }
}

impl std::error::Error for ArchiveError {}

impl From<io::Error> for ArchiveError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The archive's base name without extension, which is also its top
/// directory: `vsift-<version>-<target>`.
pub(crate) fn archive_stem(version: &str, target: ReleaseTarget) -> String {
    format!("vsift-{version}-{}", target.triple())
}

/// The archive's file name: `vsift-<version>-<target>.tar.gz`.
pub(crate) fn archive_file_name(version: &str, target: ReleaseTarget) -> String {
    format!("{}.tar.gz", archive_stem(version, target))
}

/// One entry of an archive: a directory or a regular file with its mode.
#[derive(Debug, Eq, PartialEq)]
enum Entry {
    Directory,
    File { mode: u32, bytes: Vec<u8> },
}

/// Checks the contents and lays them out as the archive's entries, keyed by
/// their path inside the archive (directories without a trailing slash).
fn entries(
    version: &str,
    target: ReleaseTarget,
    contents: &ArchiveContents,
) -> Result<BTreeMap<String, Entry>, ArchiveError> {
    if !target.matches_executable(&contents.executable) {
        return Err(ArchiveError::ForeignExecutable(target));
    }
    for name in LICENCE_FILES {
        if contents.licences.get(name).is_none_or(Vec::is_empty) {
            return Err(ArchiveError::MissingLicence(name));
        }
    }
    if contents.licences.len() != LICENCE_FILES.len() {
        return Err(ArchiveError::Mismatch(String::from(
            "only the three licence files may be packaged",
        )));
    }
    check_notices(&contents.notices)?;
    check_sbom(&contents.sbom)?;
    if !contents.skill.contains_key("SKILL.md") {
        return Err(ArchiveError::UnexpectedSkill(String::from(
            "SKILL.md is missing",
        )));
    }

    let top = archive_stem(version, target);
    let mut entries = BTreeMap::new();
    entries.insert(top.clone(), Entry::Directory);
    let mut add_file = |relative: &str, mode: u32, bytes: &[u8]| {
        entries.insert(
            format!("{top}/{relative}"),
            Entry::File {
                mode,
                bytes: bytes.to_vec(),
            },
        );
    };
    add_file(
        target.executable_name(),
        EXECUTABLE_MODE,
        &contents.executable,
    );
    for (name, bytes) in &contents.licences {
        add_file(name, FILE_MODE, bytes);
    }
    add_file(NOTICES_NAME, FILE_MODE, &contents.notices);
    add_file(SBOM_NAME, FILE_MODE, &contents.sbom);
    for (relative, bytes) in &contents.skill {
        if !is_plain_relative_path(relative) {
            return Err(ArchiveError::UnexpectedSkill(String::from(
                "a skill path is not a plain relative path",
            )));
        }
        add_file(&format!("{SKILL_DIRECTORY}/{relative}"), FILE_MODE, bytes);
    }
    let directories: Vec<String> = entries
        .keys()
        .flat_map(|path| {
            let mut parents = Vec::new();
            let mut current = path.as_str();
            while let Some((parent, _)) = current.rsplit_once('/') {
                parents.push(parent.to_owned());
                current = parent;
            }
            parents
        })
        .collect();
    for directory in directories {
        entries.entry(directory).or_insert(Entry::Directory);
    }
    Ok(entries)
}

fn check_notices(notices: &[u8]) -> Result<(), ArchiveError> {
    if notices.starts_with(NOTICES_FIRST_LINE.as_bytes()) {
        Ok(())
    } else {
        Err(ArchiveError::UnexpectedNotices)
    }
}

fn check_sbom(sbom: &[u8]) -> Result<(), ArchiveError> {
    let document: serde_json::Value =
        serde_json::from_slice(sbom).map_err(|_| ArchiveError::UnexpectedSbom("it is not JSON"))?;
    if document
        .get("bomFormat")
        .and_then(serde_json::Value::as_str)
        != Some("CycloneDX")
    {
        return Err(ArchiveError::UnexpectedSbom(
            "its bomFormat is not CycloneDX",
        ));
    }
    let component = document
        .get("metadata")
        .and_then(|metadata| metadata.get("component"))
        .and_then(|component| component.get("name"))
        .and_then(serde_json::Value::as_str);
    if component != Some("vsift-cli") {
        return Err(ArchiveError::UnexpectedSbom(
            "it does not describe the vsift-cli crate",
        ));
    }
    Ok(())
}

/// A path of `/`-separated normal components: no root, no `.` or `..`, no
/// empty component and no backslash.
pub(crate) fn is_plain_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && path
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

/// Writes the archive for `contents` to `output` and returns `output`.
///
/// `source_date_epoch` is the source commit's time in seconds, which every
/// entry carries as its modification time.
pub(crate) fn write_archive<W: Write>(
    version: &str,
    target: ReleaseTarget,
    source_date_epoch: u64,
    contents: &ArchiveContents,
    output: W,
) -> Result<W, ArchiveError> {
    let entries = entries(version, target, contents)?;
    // No file name and a zero time in the gzip header; the operating-system
    // byte is "unknown" on every host.
    let encoder = GzBuilder::new()
        .operating_system(255)
        .mtime(0)
        .write(output, Compression::best());
    let mut builder = Builder::new(encoder);
    for (path, entry) in &entries {
        let mut header = Header::new_ustar();
        header.set_mtime(source_date_epoch);
        header.set_uid(0);
        header.set_gid(0);
        match entry {
            Entry::Directory => {
                header.set_entry_type(EntryType::Directory);
                header.set_mode(DIRECTORY_MODE);
                header.set_size(0);
                builder.append_data(&mut header, path, io::empty())?;
            }
            Entry::File { mode, bytes } => {
                header.set_entry_type(EntryType::Regular);
                header.set_mode(*mode);
                header.set_size(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                builder.append_data(&mut header, path, bytes.as_slice())?;
            }
        }
    }
    let encoder = builder.into_inner()?;
    Ok(encoder.finish()?)
}

/// Reads an archive back and checks that it holds exactly what
/// [`write_archive`] writes for `contents` and `source_date_epoch`.
///
/// Two checks, so a fault in one path cannot hide in the other: the entries
/// read back through a tar reader must be the expected paths, kinds, modes
/// and bytes and nothing else; and the archive's bytes must equal a fresh
/// packaging, which also fixes every header field the first check does not
/// name (owners, times) and the compression.
pub(crate) fn verify_archive(
    version: &str,
    target: ReleaseTarget,
    source_date_epoch: u64,
    contents: &ArchiveContents,
    archive: &[u8],
) -> Result<(), ArchiveError> {
    let expected = entries(version, target, contents)?;
    let mut found = BTreeMap::new();
    let mut reader = Archive::new(GzDecoder::new(archive).take(MAXIMUM_UNPACKED_BYTES + 1));
    let mut unpacked: u64 = 0;
    for entry in reader.entries()? {
        let mut entry = entry?;
        let path = entry
            .path()?
            .to_string_lossy()
            .trim_end_matches('/')
            .to_owned();
        let header = entry.header();
        let mode = header.mode()?;
        let kind = header.entry_type();
        let actual = if kind == EntryType::Directory {
            if mode != DIRECTORY_MODE {
                return Err(ArchiveError::Mismatch(format!("{path} has mode {mode:o}")));
            }
            Entry::Directory
        } else if kind == EntryType::Regular {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            unpacked = unpacked.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
            if unpacked > MAXIMUM_UNPACKED_BYTES {
                return Err(ArchiveError::Mismatch(String::from(
                    "the archive is too large",
                )));
            }
            Entry::File { mode, bytes }
        } else {
            return Err(ArchiveError::Mismatch(format!(
                "{path} is neither a directory nor a regular file"
            )));
        };
        if found.insert(path.clone(), actual).is_some() {
            return Err(ArchiveError::Mismatch(format!("{path} appears twice")));
        }
    }
    for (path, entry) in &expected {
        match found.remove(path) {
            None => return Err(ArchiveError::Mismatch(format!("{path} is missing"))),
            Some(actual) if actual != *entry => {
                return Err(ArchiveError::Mismatch(format!("{path} differs")));
            }
            Some(_) => {}
        }
    }
    if let Some(extra) = found.keys().next() {
        return Err(ArchiveError::Mismatch(format!("{extra} was not packaged")));
    }
    let repackaged = write_archive(version, target, source_date_epoch, contents, Vec::new())?;
    if repackaged != archive {
        return Err(ArchiveError::Mismatch(String::from(
            "its bytes differ from a fresh packaging (header fields or compression)",
        )));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{collections::BTreeMap, error::Error, io::Read};

    use flate2::{Compression, GzBuilder, read::GzDecoder};
    use tar::{Archive, Builder, EntryType, Header};

    use super::{ArchiveContents, ArchiveError, archive_file_name, verify_archive, write_archive};
    use crate::target::{ReleaseTarget, tests::minimal_executable};

    const EPOCH: u64 = 1_790_000_000;

    pub(crate) fn contents(target: ReleaseTarget) -> ArchiveContents {
        let licences = ["LICENSE", "LICENSE-APACHE", "LICENSE-MIT"]
            .into_iter()
            .map(|name| (name.to_owned(), format!("{name} text\n").into_bytes()))
            .collect();
        let skill = [
            ("SKILL.md", "# Skill\n"),
            ("references/commands.md", "commands\n"),
            ("assets/image-check.png", "\u{89}PNG"),
        ]
        .into_iter()
        .map(|(path, text)| (path.to_owned(), text.as_bytes().to_vec()))
        .collect();
        ArchiveContents {
            executable: minimal_executable(target),
            licences,
            notices: b"THIRD-PARTY NOTICES\n\nMIT\n".to_vec(),
            sbom: br#"{"bomFormat":"CycloneDX","specVersion":"1.5","metadata":{"component":{"name":"vsift-cli"}}}"#.to_vec(),
            skill,
        }
    }

    /// Path, kind, mode, modification time and whether both numeric owner
    /// fields of the ustar header (bytes 108..124) are zero.
    type Listing = Vec<(String, EntryType, u32, u64, bool)>;

    fn listing(archive: &[u8]) -> Result<Listing, Box<dyn Error>> {
        let mut result = Vec::new();
        let mut archive = Archive::new(GzDecoder::new(archive));
        for entry in archive.entries()? {
            let entry = entry?;
            let header = entry.header();
            result.push((
                entry.path()?.to_string_lossy().into_owned(),
                header.entry_type(),
                header.mode()?,
                header.mtime()?,
                header.as_bytes().get(108..124).is_some_and(|fields| {
                    fields.iter().all(|byte| matches!(byte, b'0' | b' ' | 0))
                }),
            ));
        }
        Ok(result)
    }

    #[test]
    fn archives_are_named_by_version_and_target() {
        assert_eq!(
            archive_file_name("0.1.0", ReleaseTarget::LinuxX64),
            "vsift-0.1.0-x86_64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(
            archive_file_name("0.1.0", ReleaseTarget::WindowsX64),
            "vsift-0.1.0-x86_64-pc-windows-msvc.tar.gz"
        );
    }

    #[test]
    fn an_archive_holds_exactly_the_release_layout() -> Result<(), Box<dyn Error>> {
        for target in ReleaseTarget::ALL {
            let archive = write_archive("0.1.0", target, EPOCH, &contents(target), Vec::new())?;
            let entries = listing(&archive)?;
            let top = format!("vsift-0.1.0-{target}");
            let paths: Vec<String> = entries
                .iter()
                .map(|(path, ..)| path.trim_end_matches('/').to_owned())
                .collect();
            let executable = target.executable_name();
            let mut expected: Vec<String> = [
                "",
                "/LICENSE",
                "/LICENSE-APACHE",
                "/LICENSE-MIT",
                "/THIRD-PARTY-NOTICES",
                &format!("/{executable}"),
                "/skills",
                "/skills/vsift",
                "/skills/vsift/SKILL.md",
                "/skills/vsift/assets",
                "/skills/vsift/assets/image-check.png",
                "/skills/vsift/references",
                "/skills/vsift/references/commands.md",
                "/vsift.cdx.json",
            ]
            .iter()
            .map(|suffix| format!("{top}{suffix}"))
            .collect();
            expected.sort();
            assert_eq!(paths, expected, "{target}");
            for (path, kind, mode, mtime, owned_by_zero) in &entries {
                assert_eq!((*mtime, *owned_by_zero), (EPOCH, true), "{path}");
                let expected_mode = if *kind == EntryType::Directory || path.ends_with(executable) {
                    0o755
                } else {
                    0o644
                };
                assert_eq!(*mode, expected_mode, "{path}");
            }
        }
        Ok(())
    }

    #[test]
    fn packaging_is_deterministic() -> Result<(), Box<dyn Error>> {
        let target = ReleaseTarget::LinuxX64;
        let first = write_archive("0.1.0", target, EPOCH, &contents(target), Vec::new())?;
        let second = write_archive("0.1.0", target, EPOCH, &contents(target), Vec::new())?;
        assert_eq!(first, second);
        // The gzip header carries no name and no time (RFC 1952: bytes 3..8).
        assert_eq!(first.get(3..8), Some([0_u8, 0, 0, 0, 0].as_slice()));
        let later = write_archive("0.1.0", target, EPOCH + 1, &contents(target), Vec::new())?;
        assert_ne!(first, later);
        Ok(())
    }

    #[test]
    fn a_written_archive_verifies_and_any_change_is_found() -> Result<(), Box<dyn Error>> {
        let target = ReleaseTarget::MacosArm64;
        let archive = write_archive("0.1.0", target, EPOCH, &contents(target), Vec::new())?;
        verify_archive(
            "0.1.0",
            target,
            EPOCH,
            &contents(target),
            archive.as_slice(),
        )?;

        let mut changed_skill = contents(target);
        changed_skill
            .skill
            .insert(String::from("SKILL.md"), b"# Changed\n".to_vec());
        assert!(matches!(
            verify_archive("0.1.0", target, EPOCH, &changed_skill, archive.as_slice()),
            Err(ArchiveError::Mismatch(_))
        ));

        let mut other_binary = contents(target);
        other_binary.executable.push(0);
        assert!(matches!(
            verify_archive("0.1.0", target, EPOCH, &other_binary, archive.as_slice()),
            Err(ArchiveError::Mismatch(_))
        ));
        Ok(())
    }

    #[test]
    fn an_extra_binary_in_an_archive_is_refused() -> Result<(), Box<dyn Error>> {
        let target = ReleaseTarget::LinuxX64;
        let good = write_archive("0.1.0", target, EPOCH, &contents(target), Vec::new())?;
        // Rebuild the same archive with a test binary added beside `vsift`.
        let mut builder = Builder::new(GzBuilder::new().write(Vec::new(), Compression::best()));
        let mut source = Archive::new(GzDecoder::new(good.as_slice()));
        for entry in source.entries()? {
            let mut entry = entry?;
            let mut header = entry.header().clone();
            let path = entry.path()?.into_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            builder.append_data(&mut header, path, bytes.as_slice())?;
        }
        let mut header = Header::new_ustar();
        header.set_entry_type(EntryType::Regular);
        header.set_mode(0o755);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(EPOCH);
        header.set_size(4);
        builder.append_data(
            &mut header,
            "vsift-0.1.0-x86_64-unknown-linux-gnu/vsift-smoke-fixture",
            b"\x7fELF".as_slice(),
        )?;
        let tampered = builder.into_inner()?.finish()?;
        let result = verify_archive(
            "0.1.0",
            target,
            EPOCH,
            &contents(target),
            tampered.as_slice(),
        );
        assert!(
            matches!(&result, Err(ArchiveError::Mismatch(reason)) if reason.contains("vsift-smoke-fixture")),
            "{result:?}"
        );
        Ok(())
    }

    #[test]
    fn the_same_entries_packaged_differently_are_refused() -> Result<(), Box<dyn Error>> {
        let target = ReleaseTarget::LinuxX64;
        let good = write_archive("0.1.0", target, EPOCH, &contents(target), Vec::new())?;
        // Every entry unchanged, only the compression level differs.
        let mut builder = Builder::new(GzBuilder::new().write(Vec::new(), Compression::fast()));
        let mut source = Archive::new(GzDecoder::new(good.as_slice()));
        for entry in source.entries()? {
            let mut entry = entry?;
            let mut header = entry.header().clone();
            let path = entry.path()?.into_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            builder.append_data(&mut header, path, bytes.as_slice())?;
        }
        let recompressed = builder.into_inner()?.finish()?;
        assert_ne!(recompressed, good);
        let result = verify_archive("0.1.0", target, EPOCH, &contents(target), &recompressed);
        assert!(
            matches!(&result, Err(ArchiveError::Mismatch(reason)) if reason.contains("fresh packaging")),
            "{result:?}"
        );
        // A different commit time is a different archive too.
        let result = verify_archive("0.1.0", target, EPOCH + 1, &contents(target), &good);
        assert!(
            matches!(result, Err(ArchiveError::Mismatch(_))),
            "{result:?}"
        );
        Ok(())
    }

    #[test]
    fn contents_that_are_not_a_release_are_refused() {
        let target = ReleaseTarget::WindowsX64;
        let mut wrong_target = contents(target);
        wrong_target.executable = minimal_executable(ReleaseTarget::LinuxX64);
        let mut no_licence = contents(target);
        no_licence.licences.remove("LICENSE-MIT");
        let mut extra_licence = contents(target);
        extra_licence
            .licences
            .insert(String::from("COPYING"), b"x".to_vec());
        let mut notices = contents(target);
        notices.notices = b"Some other text".to_vec();
        let mut sbom = contents(target);
        sbom.sbom = br#"{"bomFormat":"SPDX"}"#.to_vec();
        let mut other_component = contents(target);
        other_component.sbom =
            br#"{"bomFormat":"CycloneDX","metadata":{"component":{"name":"vsift-infrastructure"}}}"#
                .to_vec();
        let mut no_skill = contents(target);
        no_skill.skill.remove("SKILL.md");
        let mut escaping_skill = contents(target);
        escaping_skill
            .skill
            .insert(String::from("../outside.md"), b"x".to_vec());
        for (name, candidate) in [
            ("wrong target", wrong_target),
            ("no licence", no_licence),
            ("extra licence", extra_licence),
            ("notices", notices),
            ("sbom", sbom),
            ("sbom component", other_component),
            ("no skill", no_skill),
            ("escaping skill", escaping_skill),
        ] {
            assert!(
                write_archive("0.1.0", target, EPOCH, &candidate, Vec::new()).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn skill_paths_must_be_plain_and_relative() {
        let map: BTreeMap<&str, bool> = [
            ("SKILL.md", true),
            ("references/commands.md", true),
            ("", false),
            ("/etc/passwd", false),
            ("a//b", false),
            ("./a", false),
            ("a/../b", false),
            ("a\\b", false),
        ]
        .into_iter()
        .collect();
        for (path, plain) in map {
            assert_eq!(super::is_plain_relative_path(path), plain, "{path:?}");
        }
    }
}
