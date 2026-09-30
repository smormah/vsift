//! The npm packages, assembled from the release archives (P13 PR 9, ADR 0023
//! section 2 and decisions A, H5 and H7).
//!
//! Four packages, each a directory `npm pack` turns into a tarball:
//!
//! - `vsift-cli`: the launcher `bin/vsift.cjs` and `lib/launcher.cjs`, its manifest and README from
//!   `npm/vsift-cli/` in the repository, `platform-digests.json` (the size and
//!   SHA-256 of each target's executable, computed here from the archives, so
//!   the digest the launcher checks comes from the build), the three licence
//!   files and the agent skill under `skills/vsift/`, all from the archives;
//! - `@vsift/win32-x64`, `@vsift/darwin-arm64`, `@vsift/linux-x64`: a
//!   manifest written here (`os`, `cpu`, the licence and repository, nothing
//!   else), a short README, the target's executable, its
//!   `THIRD-PARTY-NOTICES` and the licence files.
//!
//! No package has an install script, and no manifest names a person: the
//! launcher's manifest is checked field by field, the platform manifests are
//! written from a fixed template, and [`verify_tarball`] refuses a packed
//! manifest with lifecycle scripts or a `binding.gyp` (from which npm would
//! infer an install script) before comparing every packed byte.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::{self, Write},
    io::{self, Read},
};

use flate2::read::GzDecoder;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tar::{Archive, EntryType};

use crate::{
    archive::{ArchiveContents, NOTICES_NAME, SKILL_DIRECTORY},
    target::ReleaseTarget,
};

/// Where the launcher's sources live in the repository.
pub(crate) const LAUNCHER_DIRECTORY: &str = "npm/vsift-cli";
/// The launcher package's npm name (ADR 0023 decision A, amended 2026-09-30: npm
/// refused the unscoped `vsift` as too similar to existing names).
pub(crate) const LAUNCHER_PACKAGE: &str = "vsift-cli";

/// The launcher's files taken from [`LAUNCHER_DIRECTORY`] as they are.
pub(crate) const LAUNCHER_MANIFEST: &str = "package.json";
/// The launcher script, relative to [`LAUNCHER_DIRECTORY`].
pub(crate) const LAUNCHER_SCRIPT: &str = "bin/vsift.cjs";
/// The launcher library the script runs, relative to [`LAUNCHER_DIRECTORY`].
pub(crate) const LAUNCHER_LIBRARY: &str = "lib/launcher.cjs";
/// The launcher's README, relative to [`LAUNCHER_DIRECTORY`].
pub(crate) const LAUNCHER_README: &str = "README.md";

/// The digest file the launcher reads (ADR 0023 decision H5).
const DIGESTS_NAME: &str = "platform-digests.json";

/// The digest file's format identifier, which the launcher requires.
const DIGESTS_FORMAT: &str = "vsift-platform-digests/1";

const LICENCE: &str = "MIT OR Apache-2.0";
const HOMEPAGE: &str = "https://github.com/smormah/vsift";
/// The repository URL in the form npm writes it; npm provenance (P13 PR 10)
/// requires it to name the repository the workflow runs in.
const REPOSITORY_URL: &str = "git+https://github.com/smormah/vsift.git";

/// Package-manifest keys that run code when a package is installed, packed
/// or published. None may appear in any `VSift` package (ADR 0023 section 2).
const LIFECYCLE_SCRIPTS: [&str; 9] = [
    "preinstall",
    "install",
    "postinstall",
    "prepare",
    "preprepare",
    "postprepare",
    "prepublish",
    "prepack",
    "postpack",
];

/// Npm infers `"install": "node-gyp rebuild"` for a package holding this
/// file, so no package may hold it.
const NODE_GYP_FILE: &str = "binding.gyp";

/// The launcher manifest's fields, exactly; any other (`scripts`, `author`,
/// `contributors`, `maintainers`, `dependencies`, ...) is refused.
const LAUNCHER_MANIFEST_KEYS: [&str; 10] = [
    "name",
    "version",
    "description",
    "license",
    "homepage",
    "repository",
    "bin",
    "files",
    "engines",
    "optionalDependencies",
];

/// The largest packed package read back; the biggest holds one executable
/// of about 8 MiB.
const MAXIMUM_UNPACKED_BYTES: u64 = 256 * 1024 * 1024;

const EXECUTABLE_MODE: u32 = 0o755;
const FILE_MODE: u32 = 0o644;

/// The launcher's sources, read from [`LAUNCHER_DIRECTORY`].
pub(crate) struct LauncherSources {
    /// `package.json`, published byte for byte once checked.
    pub manifest: Vec<u8>,
    /// `bin/vsift.cjs`, the command.
    pub script: Vec<u8>,
    /// `lib/launcher.cjs`, what the command runs.
    pub library: Vec<u8>,
    /// `README.md`.
    pub readme: Vec<u8>,
}

/// One file of a package: its mode and bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PackageFile {
    /// 0755 for the executable and the launcher script, 0644 otherwise.
    pub mode: u32,
    /// The file's content.
    pub bytes: Vec<u8>,
}

/// One npm package, before it is packed.
#[derive(Debug)]
pub(crate) struct NpmPackage {
    /// The package name, such as `vsift` or `@vsift/linux-x64`.
    pub name: &'static str,
    /// The directory it is assembled in, which `npm pack` also names the
    /// tarball after.
    pub directory: &'static str,
    /// Every file by its `/`-separated path inside the package.
    pub files: BTreeMap<String, PackageFile>,
}

/// Why the packages could not be assembled or a tarball is not as assembled.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum NpmError {
    /// No archive, or more than one, was given for a target.
    Targets(String),
    /// The archives disagree on a file every package shares.
    SharedFilesDiffer(&'static str),
    /// The version cannot appear in a package manifest.
    UnsafeVersion(String),
    /// The launcher's manifest is not the reviewed shape.
    LauncherManifest(String),
    /// A packed tarball is not the package it should be.
    Tarball {
        /// The package, or the tarball's position when it names none.
        package: String,
        /// What is wrong.
        reason: String,
    },
}

impl fmt::Display for NpmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Targets(reason) => {
                write!(formatter, "the archives are not one per target: {reason}")
            }
            Self::SharedFilesDiffer(what) => {
                write!(formatter, "the archives do not hold the same {what}")
            }
            Self::UnsafeVersion(version) => {
                write!(
                    formatter,
                    "{version:?} cannot be written into a package manifest"
                )
            }
            Self::LauncherManifest(reason) => write!(
                formatter,
                "{LAUNCHER_DIRECTORY}/{LAUNCHER_MANIFEST} is not the reviewed launcher manifest: {reason}"
            ),
            Self::Tarball { package, reason } => {
                write!(
                    formatter,
                    "the packed {package} is not as assembled: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for NpmError {}

/// Assembles the four packages for `version` from one archive's contents per
/// target and the launcher's sources.
pub(crate) fn assemble(
    version: &str,
    archives: &[(ReleaseTarget, ArchiveContents)],
    launcher: &LauncherSources,
) -> Result<Vec<NpmPackage>, NpmError> {
    if !is_safe_version(version) {
        return Err(NpmError::UnsafeVersion(version.to_owned()));
    }
    let by_target = one_per_target(archives)?;
    check_launcher_manifest(&launcher.manifest, version)?;
    let (_, first) = by_target
        .first()
        .ok_or_else(|| NpmError::Targets(String::from("none given")))?;
    for (_, contents) in &by_target {
        if contents.licences != first.licences {
            return Err(NpmError::SharedFilesDiffer("licence files"));
        }
        if contents.skill != first.skill {
            return Err(NpmError::SharedFilesDiffer("skill"));
        }
    }

    let mut packages = Vec::new();
    let mut launcher_files = BTreeMap::new();
    launcher_files.insert(
        String::from(LAUNCHER_MANIFEST),
        file(FILE_MODE, &launcher.manifest),
    );
    launcher_files.insert(
        String::from(LAUNCHER_README),
        file(FILE_MODE, &launcher.readme),
    );
    launcher_files.insert(
        String::from(LAUNCHER_SCRIPT),
        file(EXECUTABLE_MODE, &launcher.script),
    );
    launcher_files.insert(
        String::from(LAUNCHER_LIBRARY),
        file(FILE_MODE, &launcher.library),
    );
    launcher_files.insert(
        String::from(DIGESTS_NAME),
        file(FILE_MODE, digests(version, &by_target).as_bytes()),
    );
    for (name, bytes) in &first.licences {
        launcher_files.insert(name.clone(), file(FILE_MODE, bytes));
    }
    for (relative, bytes) in &first.skill {
        launcher_files.insert(
            format!("{SKILL_DIRECTORY}/{relative}"),
            file(FILE_MODE, bytes),
        );
    }
    packages.push(NpmPackage {
        name: LAUNCHER_PACKAGE,
        directory: LAUNCHER_PACKAGE,
        files: launcher_files,
    });

    for (target, contents) in &by_target {
        let mut files = BTreeMap::new();
        files.insert(
            String::from("package.json"),
            file(FILE_MODE, platform_manifest(version, *target).as_bytes()),
        );
        files.insert(
            String::from("README.md"),
            file(FILE_MODE, platform_readme(version, *target).as_bytes()),
        );
        files.insert(
            target.executable_name().to_owned(),
            file(EXECUTABLE_MODE, &contents.executable),
        );
        files.insert(
            String::from(NOTICES_NAME),
            file(FILE_MODE, &contents.notices),
        );
        for (name, bytes) in &contents.licences {
            files.insert(name.clone(), file(FILE_MODE, bytes));
        }
        packages.push(NpmPackage {
            name: target.npm_package_name(),
            directory: target.npm_directory(),
            files,
        });
    }
    Ok(packages)
}

fn file(mode: u32, bytes: &[u8]) -> PackageFile {
    PackageFile {
        mode,
        bytes: bytes.to_vec(),
    }
}

/// The archives in [`ReleaseTarget::ALL`] order, each target exactly once.
fn one_per_target(
    archives: &[(ReleaseTarget, ArchiveContents)],
) -> Result<Vec<(ReleaseTarget, &ArchiveContents)>, NpmError> {
    let mut ordered = Vec::new();
    for target in ReleaseTarget::ALL {
        let mut matching = archives
            .iter()
            .filter(|(candidate, _)| *candidate == target);
        let Some((_, contents)) = matching.next() else {
            return Err(NpmError::Targets(format!("{target} is missing")));
        };
        if matching.next().is_some() {
            return Err(NpmError::Targets(format!("{target} is given twice")));
        }
        ordered.push((target, contents));
    }
    if archives.len() != ordered.len() {
        return Err(NpmError::Targets(String::from(
            "an archive is not a release target",
        )));
    }
    Ok(ordered)
}

/// A version made only of ASCII letters, digits, `.`, `-` and `+`, so it can
/// be written between quotes in JSON and Markdown as it is.
fn is_safe_version(version: &str) -> bool {
    !version.is_empty()
        && version.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '+')
        })
}

/// Checks the repository's launcher manifest: exactly the reviewed fields,
/// this version, and each platform package as an exact-version optional
/// dependency. Anything else (a lifecycle script, a person) is refused.
fn check_launcher_manifest(manifest: &[u8], version: &str) -> Result<(), NpmError> {
    let refuse = |reason: &str| NpmError::LauncherManifest(reason.to_owned());
    let document: Value = serde_json::from_slice(manifest).map_err(|_| refuse("it is not JSON"))?;
    let Value::Object(fields) = &document else {
        return Err(refuse("it is not a JSON object"));
    };
    let keys: BTreeSet<&str> = fields.keys().map(String::as_str).collect();
    let expected_keys: BTreeSet<&str> = LAUNCHER_MANIFEST_KEYS.into_iter().collect();
    if keys != expected_keys {
        return Err(refuse(&format!(
            "its fields must be exactly {LAUNCHER_MANIFEST_KEYS:?}"
        )));
    }
    let optional: serde_json::Map<String, Value> = ReleaseTarget::ALL
        .into_iter()
        .map(|target| (target.npm_package_name().to_owned(), json!(version)))
        .collect();
    let expectations = [
        ("name", json!(LAUNCHER_PACKAGE)),
        ("version", json!(version)),
        ("license", json!(LICENCE)),
        ("homepage", json!(HOMEPAGE)),
        ("repository", json!({"type": "git", "url": REPOSITORY_URL})),
        ("bin", json!({"vsift": LAUNCHER_SCRIPT})),
        (
            "files",
            json!([
                LAUNCHER_SCRIPT,
                LAUNCHER_LIBRARY,
                DIGESTS_NAME,
                "skills/",
                "LICENSE",
                "LICENSE-APACHE",
                "LICENSE-MIT"
            ]),
        ),
        ("engines", json!({"node": ">=22"})),
        ("optionalDependencies", Value::Object(optional)),
    ];
    for (key, expected) in expectations {
        if fields.get(key) != Some(&expected) {
            return Err(refuse(&format!("`{key}` must be {expected}")));
        }
    }
    if !fields.get("description").is_some_and(Value::is_string) {
        return Err(refuse("`description` must be text"));
    }
    Ok(())
}

/// The lowercase hexadecimal SHA-256 of `bytes`, as the launcher computes it.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _infallible = write!(hex, "{byte:02x}");
    }
    hex
}

/// `platform-digests.json`: the size and SHA-256 of every target's
/// executable, which the launcher checks before it runs one.
fn digests(version: &str, archives: &[(ReleaseTarget, &ArchiveContents)]) -> String {
    let mut packages: Vec<(&str, String)> = archives
        .iter()
        .map(|(target, contents)| {

            let hex = sha256_hex(&contents.executable);
            let entry = format!(
                "    \"{}\": {{\n      \"file\": \"{}\",\n      \"size\": {},\n      \"sha256\": \"{hex}\"\n    }}",
                target.npm_package_name(),
                target.executable_name(),
                contents.executable.len()
            );
            (target.npm_package_name(), entry)
        })
        .collect();
    packages.sort_by(|left, right| left.0.cmp(right.0));
    let entries: Vec<String> = packages.into_iter().map(|(_, entry)| entry).collect();
    format!(
        "{{\n  \"format\": \"{DIGESTS_FORMAT}\",\n  \"version\": \"{version}\",\n  \"packages\": {{\n{}\n  }}\n}}\n",
        entries.join(",\n")
    )
}

/// A platform package's manifest: identity, licence, repository, `os` and
/// `cpu`, its files, `preferUnplugged` (Yarn's Plug'n'Play keeps the
/// executable on disk, where it can run) and public access for the scope.
fn platform_manifest(version: &str, target: ReleaseTarget) -> String {
    let (os, cpu) = target.npm_os_and_cpu();
    format!(
        r#"{{
  "name": "{name}",
  "version": "{version}",
  "description": "The vsift executable for {label}. The vsift-cli package installs it and runs it as the vsift command; install vsift-cli, not this package.",
  "license": "{LICENCE}",
  "homepage": "{HOMEPAGE}",
  "repository": {{
    "type": "git",
    "url": "{REPOSITORY_URL}"
  }},
  "os": [
    "{os}"
  ],
  "cpu": [
    "{cpu}"
  ],
  "files": [
    "{executable}",
    "{NOTICES_NAME}",
    "LICENSE",
    "LICENSE-APACHE",
    "LICENSE-MIT"
  ],
  "preferUnplugged": true,
  "publishConfig": {{
    "access": "public"
  }}
}}
"#,
        name = target.npm_package_name(),
        label = target.label(),
        executable = target.executable_name(),
    )
}

fn platform_readme(version: &str, target: ReleaseTarget) -> String {
    format!(
        "# {name}\n\nThe `vsift` executable, version {version}, for {label}.\n\n\
         The [`vsift-cli`](https://www.npmjs.com/package/vsift-cli) package lists this package as an \
         optional dependency, installs it on {label} only, checks the executable's version and \
         SHA-256, and runs it as the `vsift` command. Install `vsift-cli`, not this package.\n\n\
         Licensed under MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`); \
         `{NOTICES_NAME}` lists the licences of the code compiled into the executable. \
         Source, build provenance and documentation: {HOMEPAGE}\n",
        name = target.npm_package_name(),
        label = target.label(),
    )
}

/// Checks a tarball made by `npm pack` against the packages it may be:
/// every entry under `package/`, exactly the package's files with the same
/// bytes, the executable bit exactly where the package has it, and a
/// manifest without lifecycle scripts. Returns the package's name.
pub(crate) fn verify_tarball(
    packages: &[NpmPackage],
    tarball: &[u8],
    position: usize,
) -> Result<&'static str, NpmError> {
    let refuse = |package: &str, reason: String| NpmError::Tarball {
        package: package.to_owned(),
        reason,
    };
    let unnamed = format!("tarball {position}");
    let found = read_tarball(tarball).map_err(|reason| refuse(&unnamed, reason))?;
    let manifest = found
        .get("package.json")
        .ok_or_else(|| refuse(&unnamed, String::from("it has no package.json")))?;
    let name =
        refuse_lifecycle_scripts(&manifest.bytes).map_err(|reason| refuse(&unnamed, reason))?;
    if found
        .keys()
        .any(|path| path.rsplit('/').next() == Some(NODE_GYP_FILE))
    {
        return Err(refuse(&name, format!("it holds a {NODE_GYP_FILE}")));
    }
    let package = packages
        .iter()
        .find(|package| package.name == name)
        .ok_or_else(|| {
            refuse(
                &name,
                String::from("it is not one of the assembled packages"),
            )
        })?;
    for (path, expected) in &package.files {
        let Some(actual) = found.get(path) else {
            return Err(refuse(&name, format!("{path} is missing")));
        };
        if actual.bytes != expected.bytes {
            return Err(refuse(&name, format!("{path} differs")));
        }
        let executable = |mode: u32| mode & 0o111 != 0;
        if executable(actual.mode) != executable(expected.mode) || actual.mode & 0o022 != 0 {
            return Err(refuse(
                &name,
                format!("{path} has mode {:o}, not {:o}", actual.mode, expected.mode),
            ));
        }
    }
    if let Some(extra) = found.keys().find(|path| !package.files.contains_key(*path)) {
        return Err(refuse(&name, format!("{extra} was not assembled")));
    }
    Ok(package.name)
}

/// The package name of a manifest that declares none of
/// [`LIFECYCLE_SCRIPTS`] and no `gypfile`.
fn refuse_lifecycle_scripts(manifest: &[u8]) -> Result<String, String> {
    let document: Value = serde_json::from_slice(manifest)
        .map_err(|_| String::from("its package.json is not JSON"))?;
    let name = document
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("its package.json has no name"))?
        .to_owned();
    if let Some(scripts) = document.get("scripts") {
        let lifecycle: Vec<&str> = LIFECYCLE_SCRIPTS
            .into_iter()
            .filter(|script| scripts.get(script).is_some())
            .collect();
        return Err(if lifecycle.is_empty() {
            String::from("its package.json declares scripts")
        } else {
            format!("its package.json declares lifecycle scripts {lifecycle:?}")
        });
    }
    if document.get("gypfile").is_some() {
        return Err(String::from("its package.json declares `gypfile`"));
    }
    Ok(name)
}

/// Every regular file of a packed tarball by its path under `package/`.
/// Directory entries are accepted and ignored; any other entry is refused.
fn read_tarball(tarball: &[u8]) -> Result<BTreeMap<String, PackageFile>, String> {
    let describe = |error: io::Error| format!("it cannot be read: {error}");
    let mut found = BTreeMap::new();
    let mut archive = Archive::new(GzDecoder::new(tarball).take(MAXIMUM_UNPACKED_BYTES + 1));
    let mut unpacked: u64 = 0;
    for entry in archive.entries().map_err(describe)? {
        let mut entry = entry.map_err(describe)?;
        let path = entry
            .path()
            .map_err(describe)?
            .to_string_lossy()
            .into_owned();
        let kind = entry.header().entry_type();
        if kind == EntryType::Directory {
            continue;
        }
        if kind != EntryType::Regular {
            return Err(format!("{path} is not a regular file"));
        }
        let Some(relative) = path.strip_prefix("package/") else {
            return Err(format!("{path} is not under package/"));
        };
        if relative.is_empty()
            || relative
                .split('/')
                .any(|part| part.is_empty() || part == "..")
        {
            return Err(format!("{path} is not a plain path"));
        }
        let mode = entry.header().mode().map_err(describe)?;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(describe)?;
        unpacked = unpacked.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        if unpacked > MAXIMUM_UNPACKED_BYTES {
            return Err(String::from("it is too large"));
        }
        if found
            .insert(relative.to_owned(), PackageFile { mode, bytes })
            .is_some()
        {
            return Err(format!("{path} appears twice"));
        }
    }
    Ok(found)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{collections::BTreeMap, error::Error};

    use flate2::{Compression, GzBuilder};
    use serde_json::Value;
    use tar::{Builder, EntryType, Header};

    use super::{
        LIFECYCLE_SCRIPTS, LauncherSources, NpmError, NpmPackage, PackageFile, assemble,
        sha256_hex, verify_tarball,
    };
    use crate::{archive::tests::contents, target::ReleaseTarget};

    const VERSION: &str = "0.1.0";

    /// A package's files by path.
    type Files = BTreeMap<String, PackageFile>;

    #[test]
    fn digests_are_lowercase_hexadecimal_sha256() {
        // FIPS 180-2's first SHA-256 example.
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn launcher_manifest(version: &str) -> String {
        format!(
            r#"{{
  "name": "vsift-cli",
  "version": "{version}",
  "description": "The vsift command-line tool.",
  "license": "MIT OR Apache-2.0",
  "homepage": "https://github.com/smormah/vsift",
  "repository": {{
    "type": "git",
    "url": "git+https://github.com/smormah/vsift.git"
  }},
  "bin": {{
    "vsift": "bin/vsift.cjs"
  }},
  "files": [
    "bin/vsift.cjs",
    "lib/launcher.cjs",
    "platform-digests.json",
    "skills/",
    "LICENSE",
    "LICENSE-APACHE",
    "LICENSE-MIT"
  ],
  "engines": {{
    "node": ">=22"
  }},
  "optionalDependencies": {{
    "@vsift/darwin-arm64": "{version}",
    "@vsift/linux-x64": "{version}",
    "@vsift/win32-x64": "{version}"
  }}
}}
"#
        )
    }

    fn launcher(manifest: &str) -> LauncherSources {
        LauncherSources {
            manifest: manifest.as_bytes().to_vec(),
            script: b"#!/usr/bin/env node\n'use strict';\n".to_vec(),
            library: b"'use strict';\nmodule.exports = {};\n".to_vec(),
            readme: b"# vsift\n".to_vec(),
        }
    }

    fn archives() -> Vec<(ReleaseTarget, crate::archive::ArchiveContents)> {
        ReleaseTarget::ALL
            .into_iter()
            .map(|target| (target, contents(target)))
            .collect()
    }

    /// Packs `package` the way `npm pack` does: every file under `package/`.
    pub(crate) fn pack(package: &NpmPackage) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut builder = Builder::new(GzBuilder::new().write(Vec::new(), Compression::fast()));
        for (path, file) in &package.files {
            let mut header = Header::new_ustar();
            header.set_entry_type(EntryType::Regular);
            header.set_mode(file.mode);
            header.set_size(u64::try_from(file.bytes.len())?);
            builder.append_data(
                &mut header,
                format!("package/{path}"),
                file.bytes.as_slice(),
            )?;
        }
        Ok(builder.into_inner()?.finish()?)
    }

    #[test]
    fn the_repository_launcher_manifest_is_the_reviewed_one() -> Result<(), Box<dyn Error>> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest = std::fs::read(root.join("npm/vsift-cli/package.json"))?;
        super::check_launcher_manifest(&manifest, env!("CARGO_PKG_VERSION"))?;
        Ok(())
    }

    #[test]
    fn four_packages_are_assembled_from_the_archives() -> Result<(), Box<dyn Error>> {
        let packages = assemble(VERSION, &archives(), &launcher(&launcher_manifest(VERSION)))?;
        let names: Vec<&str> = packages.iter().map(|package| package.name).collect();
        assert_eq!(
            names,
            [
                "vsift-cli",
                "@vsift/darwin-arm64",
                "@vsift/win32-x64",
                "@vsift/linux-x64"
            ]
        );
        let launcher = &packages[0];
        let files: Vec<&str> = launcher.files.keys().map(String::as_str).collect();
        assert_eq!(
            files,
            [
                "LICENSE",
                "LICENSE-APACHE",
                "LICENSE-MIT",
                "README.md",
                "bin/vsift.cjs",
                "lib/launcher.cjs",
                "package.json",
                "platform-digests.json",
                "skills/vsift/SKILL.md",
                "skills/vsift/assets/image-check.png",
                "skills/vsift/references/commands.md",
            ]
        );
        assert_eq!(launcher.files["bin/vsift.cjs"].mode, 0o755);

        let digests: Value =
            serde_json::from_slice(&launcher.files["platform-digests.json"].bytes)?;
        assert_eq!(digests["format"], "vsift-platform-digests/1");
        assert_eq!(digests["version"], VERSION);
        for target in ReleaseTarget::ALL {
            let executable = contents(target).executable;
            let entry = &digests["packages"][target.npm_package_name()];
            assert_eq!(entry["file"], target.executable_name());
            assert_eq!(entry["size"], executable.len());
            assert_eq!(entry["sha256"], sha256_hex(&executable).as_str());
        }

        for (package, target) in packages[1..].iter().zip(ReleaseTarget::ALL) {
            let files: Vec<&str> = package.files.keys().map(String::as_str).collect();
            let mut expected = vec![
                "LICENSE",
                "LICENSE-APACHE",
                "LICENSE-MIT",
                "README.md",
                "THIRD-PARTY-NOTICES",
                "package.json",
                target.executable_name(),
            ];
            expected.sort_unstable();
            assert_eq!(files, expected);
            let executable = &package.files[target.executable_name()];
            assert_eq!(executable.mode, 0o755);
            assert_eq!(executable.bytes, contents(target).executable);
            let manifest: Value = serde_json::from_slice(&package.files["package.json"].bytes)?;
            let (os, cpu) = target.npm_os_and_cpu();
            assert_eq!(manifest["name"], target.npm_package_name());
            assert_eq!(manifest["version"], VERSION);
            assert_eq!(manifest["os"], serde_json::json!([os]));
            assert_eq!(manifest["cpu"], serde_json::json!([cpu]));
            assert_eq!(manifest["preferUnplugged"], true);
            let keys: Vec<&str> = manifest
                .as_object()
                .map(|fields| fields.keys().map(String::as_str).collect())
                .unwrap_or_default();
            for forbidden in [
                "scripts",
                "author",
                "contributors",
                "maintainers",
                "gypfile",
            ] {
                assert!(
                    !keys.contains(&forbidden),
                    "{forbidden} in {}",
                    package.name
                );
            }
        }
        Ok(())
    }

    #[test]
    fn the_launcher_manifest_admits_only_the_reviewed_fields() {
        let good = launcher_manifest(VERSION);
        let bad_manifests = [
            // A lifecycle script, a person, another dependency kind.
            good.replacen(
                "\"license\"",
                "\"scripts\": {\"postinstall\": \"node install.js\"},\n  \"license\"",
                1,
            ),
            good.replacen("\"license\"", "\"author\": \"Somebody\",\n  \"license\"", 1),
            good.replacen("\"license\"", "\"dependencies\": {},\n  \"license\"", 1),
            // Another version, a range instead of an exact version, a missing target.
            launcher_manifest("0.1.1"),
            good.replacen(
                "\"@vsift/linux-x64\": \"0.1.0\"",
                "\"@vsift/linux-x64\": \"^0.1.0\"",
                1,
            ),
            good.replacen("    \"@vsift/linux-x64\": \"0.1.0\",\n", "", 1),
            // Another repository or executable name.
            good.replacen("smormah/vsift.git", "someone/vsift.git", 1),
            good.replacen(
                "\"vsift\": \"bin/vsift.cjs\"",
                "\"vs\": \"bin/vsift.cjs\"",
                1,
            ),
            String::from("[]"),
            String::from("not json"),
        ];
        for manifest in bad_manifests {
            assert!(
                matches!(
                    assemble(VERSION, &archives(), &launcher(&manifest)),
                    Err(NpmError::LauncherManifest(_))
                ),
                "accepted: {manifest}"
            );
        }
    }

    #[test]
    fn archives_must_be_one_per_target_and_agree() {
        let manifest = launcher_manifest(VERSION);
        let mut missing = archives();
        missing.pop();
        assert!(matches!(
            assemble(VERSION, &missing, &launcher(&manifest)),
            Err(NpmError::Targets(_))
        ));
        let mut doubled = archives();
        doubled.push((ReleaseTarget::LinuxX64, contents(ReleaseTarget::LinuxX64)));
        assert!(matches!(
            assemble(VERSION, &doubled, &launcher(&manifest)),
            Err(NpmError::Targets(_))
        ));
        let mut other_skill = archives();
        if let Some((_, last)) = other_skill.last_mut() {
            last.skill
                .insert(String::from("SKILL.md"), b"# Other\n".to_vec());
        }
        assert_eq!(
            assemble(VERSION, &other_skill, &launcher(&manifest)).err(),
            Some(NpmError::SharedFilesDiffer("skill"))
        );
        assert!(matches!(
            assemble("0.1.0\"", &archives(), &launcher(&manifest)),
            Err(NpmError::UnsafeVersion(_))
        ));
    }

    #[test]
    fn a_packed_package_verifies_and_every_difference_is_refused() -> Result<(), Box<dyn Error>> {
        let packages = assemble(VERSION, &archives(), &launcher(&launcher_manifest(VERSION)))?;
        for package in &packages {
            assert_eq!(verify_tarball(&packages, &pack(package)?, 1)?, package.name);
        }

        let linux = &packages[3];
        let mutate = |change: &dyn Fn(&mut Files)| {
            let mut files = linux.files.clone();
            change(&mut files);
            pack(&NpmPackage {
                name: linux.name,
                directory: linux.directory,
                files,
            })
        };
        let refused: [&dyn Fn(&mut Files); 5] = [
            &|files| {
                if let Some(file) = files.get_mut("vsift") {
                    file.bytes.push(0);
                }
            },
            &|files| {
                if let Some(file) = files.get_mut("vsift") {
                    file.mode = 0o644;
                }
            },
            &|files| {
                files.remove("THIRD-PARTY-NOTICES");
            },
            &|files| {
                files.insert(
                    String::from("vsift-smoke-fixture"),
                    PackageFile {
                        mode: 0o755,
                        bytes: vec![1],
                    },
                );
            },
            &|files| {
                files.insert(
                    String::from("binding.gyp"),
                    PackageFile {
                        mode: 0o644,
                        bytes: b"{}".to_vec(),
                    },
                );
            },
        ];
        for change in refused {
            assert!(matches!(
                verify_tarball(&packages, &mutate(change)?, 1),
                Err(NpmError::Tarball { .. })
            ));
        }
        Ok(())
    }

    #[test]
    fn any_lifecycle_script_in_a_packed_manifest_is_refused() -> Result<(), Box<dyn Error>> {
        let packages = assemble(VERSION, &archives(), &launcher(&launcher_manifest(VERSION)))?;
        let linux = &packages[3];
        for script in LIFECYCLE_SCRIPTS.into_iter().chain(["test"]) {
            let mut files = linux.files.clone();
            let manifest = String::from_utf8(files["package.json"].bytes.clone())?.replacen(
                "\"license\"",
                &format!("\"scripts\": {{\"{script}\": \"node x.js\"}},\n  \"license\""),
                1,
            );
            if let Some(file) = files.get_mut("package.json") {
                file.bytes = manifest.into_bytes();
            }
            let tarball = pack(&NpmPackage {
                name: linux.name,
                directory: linux.directory,
                files,
            })?;
            let result = verify_tarball(&packages, &tarball, 1);
            assert!(
                matches!(&result, Err(NpmError::Tarball { reason, .. }) if reason.contains("scripts")),
                "{script}: {result:?}"
            );
        }
        Ok(())
    }
}
