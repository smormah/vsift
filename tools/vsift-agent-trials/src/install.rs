//! Clean-install mode: the published package, from the real registry, into
//! a fresh prefix (P14, evidence items RQ-15 and RQ-16).
//!
//! Until P14 every trial ran a binary Cargo built from the checkout. A
//! clean-install trial runs **what a user installs**:
//! `npm install --global vsift-cli@<exact version>`, with `--prefix` a folder
//! under the neutral trial root (never the user's global folder), install
//! scripts off, and nothing of Rust, the checkout or the repository's
//! documentation on the agent's `PATH`.
//!
//! The harness proves it used the published install, not a stand-in, with
//! three independent records that [`InstallEvidence`] carries into every
//! trial record:
//!
//! 1. **Registry integrity.** The registry advertises each tarball's address
//!    and `dist.integrity` (`npm view <package>@<version> dist --json`); npm
//!    verifies every tarball against it while installing and caches what it
//!    fetched under its own content hash. The harness reads that hash back
//!    from npm's cache index (the install's own cache, in a scratch folder
//!    beside the prefix, so nothing else is in it) and requires it to equal
//!    the integrity the registry advertises, for the launcher package and for
//!    this platform's native package, and the address to be below the
//!    registry's own. A global install writes no lockfile and no `_integrity`
//!    into the packages (checked with npm 11.4.2), so the cache index is where
//!    npm records what it actually fetched.
//! 2. **The launcher's digest check.** The launcher refuses to start a
//!    native executable whose SHA-256 is not the one recorded in
//!    `platform-digests.json` when the release was built (exit 126). The
//!    harness recomputes that digest itself and also runs `vsift --version`
//!    through the launcher, which must exit 0 and name the exact version
//!    and the source commit (`vsift 0.1.0 (<12 hex>)`).
//! 3. **The exact version.** The request is an exact version, never a
//!    dist-tag or a range, and every package's version must equal it.
//!
//! What it does not prove, stated in known limit L-117: it proves the bytes
//! are the registry's, not that the registry's bytes are the maintainers'
//! (that is `npm audit signatures` and `gh attestation verify`, P14 PR 2),
//! and a machine that runs Claude Code is not a clean machine.
//!
//! Privacy: npm runs with a cleared environment, an empty user and global
//! `.npmrc` (the maintainer's own `.npmrc` may hold a publishing token, and
//! npm would send it to the registry as a header), its cache and home inside
//! a scratch folder beside the prefix, and no proxy variable unless the
//! operator passes it by name. It sends the registry only npm's own client
//! headers.

use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{TrialError, read_json, write_json},
    roots::RootPolicy,
    run::system_path_directories,
    skill::{file_digest, sha256_hex},
};

/// The public npm registry.
pub const DEFAULT_REGISTRY: &str = "https://registry.npmjs.org/";

/// The launcher package's name.
pub const LAUNCHER_PACKAGE: &str = "vsift-cli";

/// The file the launcher reads its executable digests from.
const DIGESTS_FILE: &str = "platform-digests.json";

/// The format name inside [`DIGESTS_FILE`].
const DIGESTS_FORMAT: &str = "vsift-platform-digests/1";

/// Environment variables a Windows process needs to start (the same set as
/// the client's environment, `run`).
const WINDOWS_PASSTHROUGH: [&str; 9] = [
    "SystemRoot",
    "windir",
    "SystemDrive",
    "ComSpec",
    "PATHEXT",
    "OS",
    "PROCESSOR_ARCHITECTURE",
    "NUMBER_OF_PROCESSORS",
    "ProgramData",
];

/// Where the install's bytes came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallSourceKind {
    /// The registry named in the evidence: the only source a counted trial
    /// accepts.
    Registry,
    /// Local tarballs, used by the harness's own tests; never counts.
    LocalTarballs,
}

/// The R0 target of this machine (the launcher's own table).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Target {
    /// The platform package, for example `@vsift/win32-x64`.
    pub package: &'static str,
    /// The native executable's file name inside it.
    pub executable: &'static str,
}

/// The target for an operating system and architecture as
/// `std::env::consts` names them, or `None` when R0 has none.
#[must_use]
pub fn target_for(os: &str, arch: &str) -> Option<Target> {
    match (os, arch) {
        ("windows", "x86_64") => Some(Target {
            package: "@vsift/win32-x64",
            executable: "vsift.exe",
        }),
        ("linux", "x86_64") => Some(Target {
            package: "@vsift/linux-x64",
            executable: "vsift",
        }),
        ("macos", "aarch64") => Some(Target {
            package: "@vsift/darwin-arm64",
            executable: "vsift",
        }),
        _ => None,
    }
}

/// The target of the machine the harness runs on.
///
/// # Errors
///
/// [`TrialError::Refused`] when R0 has no build for it.
pub fn host_target() -> Result<Target, TrialError> {
    target_for(env::consts::OS, env::consts::ARCH).ok_or_else(|| {
        TrialError::Refused(format!(
            "VSift has no R0 build for {} {}",
            env::consts::OS,
            env::consts::ARCH
        ))
    })
}

/// One package npm installed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstalledPackage {
    /// The package name.
    pub name: String,
    /// Its installed version.
    pub version: String,
    /// The tarball address the registry advertises (and npm fetched).
    pub resolved: String,
    /// The integrity of what npm fetched, from its cache index.
    pub integrity_installed: String,
    /// The `dist.integrity` the registry advertises for it.
    pub integrity_registry: String,
}

/// The launcher's digest check, redone by the harness.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LauncherCheck {
    /// The platform package whose executable was checked.
    pub package: String,
    /// The SHA-256 `platform-digests.json` records for it.
    pub recorded_sha256: String,
    /// The SHA-256 of the installed executable.
    pub actual_sha256: String,
    /// The size `platform-digests.json` records.
    pub recorded_size: u64,
    /// The installed executable's size.
    pub actual_size: u64,
    /// Whether size and digest both agree.
    pub matches: bool,
    /// The exit status of `vsift --version` through the launcher, which
    /// runs the same check itself and exits 126 on a mismatch.
    pub version_exit_code: Option<i32>,
    /// What `vsift --version` printed through the launcher.
    pub version_line: String,
}

/// What an install proves, free of local paths so that it may enter a
/// trial record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstallEvidence {
    /// Record version, 1.
    pub schema_version: u32,
    /// Where the bytes came from.
    pub source: InstallSourceKind,
    /// The registry asked.
    pub registry: String,
    /// The exact version requested and installed.
    pub version: String,
    /// The machine, `windows x86_64` style.
    pub platform: String,
    /// The launcher package and this platform's native package.
    pub packages: Vec<InstalledPackage>,
    /// The launcher's digest check.
    pub launcher: LauncherCheck,
    /// `node --version` of the Node.js that installed and runs it.
    pub node_version: String,
    /// `npm --version`.
    pub npm_version: String,
    /// Install scripts were switched off.
    pub ignore_scripts: bool,
}

impl InstallEvidence {
    /// Why this install does not count as the published one; empty when it
    /// does. A counted trial requires an empty list.
    #[must_use]
    pub fn not_published_because(&self) -> Vec<String> {
        let mut reasons = Vec::new();
        if self.source != InstallSourceKind::Registry {
            reasons.push("the packages did not come from a registry".to_owned());
        }
        if self.packages.len() < 2 {
            reasons.push("the launcher and a native package are not both recorded".to_owned());
        }
        for package in &self.packages {
            if package.version != self.version {
                reasons.push(format!("{} is {}", package.name, package.version));
            }
            if package.integrity_registry != package.integrity_installed {
                reasons.push(format!(
                    "{}'s installed integrity is not the registry's",
                    package.name
                ));
            }
            if !package.resolved.starts_with(&self.registry) {
                reasons.push(format!(
                    "{} was not resolved from the registry",
                    package.name
                ));
            }
        }
        if !self.launcher.matches {
            reasons.push("the native executable does not match the launcher's digest".to_owned());
        }
        if self.launcher.version_exit_code != Some(0) {
            reasons.push("vsift --version through the launcher did not exit 0".to_owned());
        }
        if !self
            .launcher
            .version_line
            .starts_with(&format!("vsift {}", self.version))
        {
            reasons.push("vsift --version does not name the installed version".to_owned());
        }
        if !self.ignore_scripts {
            reasons.push("install scripts were not switched off".to_owned());
        }
        reasons
    }
}

/// An install the harness made or verified, with the local paths the later
/// steps need. Written to the proof file; the evidence alone enters records.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstallProof {
    /// What the install proves.
    pub evidence: InstallEvidence,
    /// The npm prefix.
    pub prefix: PathBuf,
    /// The launcher package's folder.
    pub package_root: PathBuf,
    /// The native executable the launcher runs.
    pub native_executable: PathBuf,
    /// The Node.js executable that installed and runs the launcher.
    pub node: PathBuf,
    /// The directories the agent's `PATH` gets so that `vsift` resolves
    /// through npm's shim: the prefix's command folder, then Node.js's.
    pub client_path_directories: Vec<PathBuf>,
}

impl InstallProof {
    /// Reads a proof file and checks that the executable it names still has
    /// the digest it recorded.
    ///
    /// # Errors
    ///
    /// [`TrialError`] when the file is malformed, the install is gone or
    /// changed, or it does not count as the published one.
    pub fn load(path: &Path) -> Result<Self, TrialError> {
        let proof: Self = serde_json::from_value(read_json(path)?)
            .map_err(|error| TrialError::json(path.display().to_string(), error))?;
        let digest = file_digest(&proof.native_executable)?;
        if digest != proof.evidence.launcher.actual_sha256 {
            return Err(TrialError::Refused(
                "the installed vsift executable is not the one the install proof recorded"
                    .to_owned(),
            ));
        }
        let reasons = proof.evidence.not_published_because();
        if !reasons.is_empty() {
            return Err(TrialError::Refused(format!(
                "the install is not the published package: {}",
                reasons.join("; ")
            )));
        }
        Ok(proof)
    }

    /// Writes the proof file.
    ///
    /// # Errors
    ///
    /// [`TrialError`] when the file cannot be written.
    pub fn write(&self, path: &Path) -> Result<(), TrialError> {
        write_json(path, self)
    }
}

/// What the registry advertises for one package version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dist {
    /// The tarball's address.
    pub tarball: String,
    /// Its `dist.integrity`, an SRI string.
    pub integrity: String,
}

/// Answers what the registry says about a package version.
pub trait RegistryReader {
    /// The `dist` of `name@version`.
    ///
    /// # Errors
    ///
    /// [`TrialError`] when the registry cannot be asked or has no such
    /// version.
    fn dist(&self, name: &str, version: &str) -> Result<Dist, TrialError>;
}

/// Answers what npm actually fetched.
pub trait InstalledIntegrity {
    /// The integrity of the tarball npm fetched from `tarball`.
    ///
    /// # Errors
    ///
    /// [`TrialError`] when npm has no record of fetching it.
    fn integrity_of(&self, tarball: &str) -> Result<String, TrialError>;
}

/// npm's cache index, `cacache` format v5: each fetched address has a file
/// named by the SHA-256 of its key, holding one JSON line per write, the
/// last of which wins. A tarball's entry carries the integrity of the bytes
/// npm received (after verifying them against the registry's).
pub struct CacheIndex {
    cache: PathBuf,
}

impl CacheIndex {
    /// The index of the cache folder `cache` (the folder npm's
    /// `npm_config_cache` names).
    #[must_use]
    pub const fn new(cache: PathBuf) -> Self {
        Self { cache }
    }
}

impl InstalledIntegrity for CacheIndex {
    fn integrity_of(&self, tarball: &str) -> Result<String, TrialError> {
        let key = format!("make-fetch-happen:request-cache:{tarball}");
        let digest = sha256_hex(key.as_bytes());
        let file = self
            .cache
            .join("_cacache")
            .join("index-v5")
            .join(&digest[..2])
            .join(&digest[2..4])
            .join(&digest[4..]);
        let text = fs::read_to_string(&file).map_err(|_| {
            TrialError::Invalid("npm's cache holds no record of fetching a tarball".to_owned())
        })?;
        let entry = text.lines().rev().find_map(|line| {
            let (_, json) = line.split_once('\t')?;
            let entry: Value = serde_json::from_str(json).ok()?;
            (entry["key"] == key.as_str()).then_some(entry)
        });
        entry
            .and_then(|entry| entry["integrity"].as_str().map(str::to_owned))
            .filter(|integrity| integrity.starts_with("sha512-"))
            .ok_or_else(|| {
                TrialError::Invalid(
                    "npm's cache has no sha512 integrity for a tarball it fetched".to_owned(),
                )
            })
    }
}

/// Runs `vsift --version` through the installed launcher.
pub trait VersionProbe {
    /// The exit status and first stdout line of `vsift --version`.
    ///
    /// # Errors
    ///
    /// [`TrialError::Process`] when the launcher cannot be started.
    fn version(&self, package_root: &Path) -> Result<(Option<i32>, String), TrialError>;
}

/// What [`install`] needs.
#[derive(Clone, Debug)]
pub struct InstallRequest {
    /// The exact version, `0.2.0-rc.1` style; never a tag or a range.
    pub version: String,
    /// The npm prefix to create: a new folder under the neutral root.
    pub prefix: PathBuf,
    /// The Node.js executable (absolute).
    pub node: PathBuf,
    /// npm's `npm-cli.js` (absolute): the harness runs `node npm-cli.js`
    /// and never a `.cmd` or shell shim.
    pub npm_cli: PathBuf,
    /// The registry, `https` (default [`DEFAULT_REGISTRY`]).
    pub registry: String,
    /// Names of harness environment variables to pass through (a proxy).
    pub pass_environment: Vec<String>,
    /// What the prefix must avoid.
    pub root_policy: RootPolicy,
}

/// Checks that `version` is an exact semantic version `X.Y.Z` with an
/// optional pre-release part, so that it can never name a dist-tag
/// (`latest`, `next`) or a range.
///
/// # Errors
///
/// [`TrialError::Refused`] naming the rule.
pub fn validate_version(version: &str) -> Result<(), TrialError> {
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let numbers: Vec<&str> = core.split('.').collect();
    let numeric = |part: &&str| {
        !part.is_empty()
            && part.chars().all(|character| character.is_ascii_digit())
            && (part.len() == 1 || !part.starts_with('0'))
    };
    let pre_ok = pre.is_none_or(|pre| {
        !pre.is_empty()
            && pre.split('.').all(|identifier| {
                !identifier.is_empty()
                    && identifier
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric() || character == '-')
            })
    });
    if numbers.len() == 3 && numbers.iter().all(numeric) && pre_ok {
        Ok(())
    } else {
        Err(TrialError::Refused(format!(
            "{version:?} is not an exact version (X.Y.Z or X.Y.Z-pre.N); a tag or a range could install something else"
        )))
    }
}

/// Checks the registry address: `https`, or loopback `http` for a local
/// registry in a test.
///
/// # Errors
///
/// [`TrialError::Refused`] otherwise.
pub fn validate_registry(registry: &str) -> Result<(), TrialError> {
    let loopback = registry.starts_with("http://127.0.0.1")
        || registry.starts_with("http://localhost")
        || registry.starts_with("http://[::1]");
    let plain = registry
        .chars()
        .all(|character| character.is_ascii_graphic() && character != '\'' && character != '"');
    if (registry.starts_with("https://") || loopback) && registry.ends_with('/') && plain {
        Ok(())
    } else {
        Err(TrialError::Refused(
            "the registry must be an https address ending in a slash".to_owned(),
        ))
    }
}

/// The folder npm puts global packages in under `prefix`.
#[must_use]
pub fn modules_root(prefix: &Path) -> PathBuf {
    if cfg!(windows) {
        prefix.join("node_modules")
    } else {
        prefix.join("lib").join("node_modules")
    }
}

/// The folder npm puts a global package's command shims in under `prefix`.
#[must_use]
pub fn command_directory(prefix: &Path) -> PathBuf {
    if cfg!(windows) {
        prefix.to_path_buf()
    } else {
        prefix.join("bin")
    }
}

/// Where npm put the platform package: nested below the launcher package,
/// where a global install keeps a package's dependencies (what the real npm
/// does), or beside it, in a hoisted layout. Found by looking, never assumed.
fn platform_location(modules: &Path, package_root: &Path, name: &str) -> PathBuf {
    let relative: PathBuf = name.split('/').collect();
    let nested = package_root.join("node_modules").join(&relative);
    if nested.join("package.json").is_file() {
        nested
    } else {
        modules.join(relative)
    }
}

/// What [`verify_install`] needs.
pub struct VerifyRequest<'a> {
    /// The npm prefix.
    pub prefix: &'a Path,
    /// The exact version expected.
    pub version: &'a str,
    /// The registry the packages were asked of.
    pub registry: &'a str,
    /// Where the bytes came from.
    pub source: InstallSourceKind,
    /// The Node.js executable that installed and runs the launcher.
    pub node: &'a Path,
    /// `node --version`.
    pub node_version: &'a str,
    /// `npm --version`.
    pub npm_version: &'a str,
    /// The machine's R0 target.
    pub target: Target,
}

/// Reads an install back and builds its proof: the versions, what npm
/// fetched (from its cache index) against what the registry advertises, the
/// launcher's digest recomputed and `vsift --version` through the launcher.
///
/// # Errors
///
/// [`TrialError`] when a record is missing or malformed, or a package is
/// not the version asked for. A digest or integrity that disagrees is not an
/// error here: it is recorded, and [`InstallEvidence::not_published_because`]
/// names it.
pub fn verify_install(
    request: &VerifyRequest<'_>,
    registry: &dyn RegistryReader,
    installed: &dyn InstalledIntegrity,
    probe: &dyn VersionProbe,
) -> Result<InstallProof, TrialError> {
    let modules = modules_root(request.prefix);
    let package_root = modules.join(LAUNCHER_PACKAGE);
    let platform_root = platform_location(&modules, &package_root, request.target.package);
    let version_of = |root: &Path, name: &str| -> Result<(), TrialError> {
        let manifest = read_json(&root.join("package.json"))?;
        if manifest["name"] != name || manifest["version"] != request.version {
            return Err(TrialError::Invalid(format!(
                "{name} is {}, not the requested {}",
                manifest["version"], request.version
            )));
        }
        Ok(())
    };
    version_of(&package_root, LAUNCHER_PACKAGE)?;
    version_of(&platform_root, request.target.package)?;

    let digests = read_json(&package_root.join(DIGESTS_FILE))?;
    let recorded = &digests["packages"][request.target.package];
    if digests["format"] != DIGESTS_FORMAT
        || digests["version"] != request.version
        || recorded["file"] != request.target.executable
    {
        return Err(TrialError::Invalid(
            "platform-digests.json is not this version's record for this platform".to_owned(),
        ));
    }
    let recorded_sha256 = recorded["sha256"].as_str().unwrap_or_default().to_owned();
    let recorded_size = recorded["size"].as_u64().unwrap_or_default();
    let native_executable = platform_root.join(request.target.executable);
    let bytes = fs::read(&native_executable)
        .map_err(|error| TrialError::io_at(&native_executable, error))?;
    let actual_sha256 = sha256_hex(&bytes);
    let actual_size = bytes.len() as u64;

    let mut packages = Vec::new();
    for name in [LAUNCHER_PACKAGE, request.target.package] {
        let dist = registry.dist(name, request.version)?;
        packages.push(InstalledPackage {
            name: name.to_owned(),
            version: request.version.to_owned(),
            integrity_installed: installed.integrity_of(&dist.tarball)?,
            integrity_registry: dist.integrity,
            resolved: dist.tarball,
        });
    }

    let command = command_directory(request.prefix);
    let shim = if cfg!(windows) {
        command.join("vsift.cmd")
    } else {
        command.join("vsift")
    };
    if !shim.exists() {
        return Err(TrialError::Invalid(
            "npm made no vsift command in the prefix".to_owned(),
        ));
    }
    let (version_exit_code, version_line) = probe.version(&package_root)?;
    let node_directory = request
        .node
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| TrialError::Refused("the node executable has no folder".to_owned()))?;
    Ok(InstallProof {
        evidence: InstallEvidence {
            schema_version: 1,
            source: request.source,
            registry: request.registry.to_owned(),
            version: request.version.to_owned(),
            platform: format!("{} {}", env::consts::OS, env::consts::ARCH),
            packages,
            launcher: LauncherCheck {
                package: request.target.package.to_owned(),
                matches: recorded_sha256 == actual_sha256 && recorded_size == actual_size,
                recorded_sha256,
                actual_sha256,
                recorded_size,
                actual_size,
                version_exit_code,
                version_line,
            },
            node_version: request.node_version.to_owned(),
            npm_version: request.npm_version.to_owned(),
            ignore_scripts: true,
        },
        prefix: request.prefix.to_path_buf(),
        package_root,
        native_executable,
        node: request.node.to_path_buf(),
        client_path_directories: vec![command, node_directory],
    })
}

/// The cleared environment npm runs in: its own cache, home and
/// configuration files inside `support`, the registry named explicitly, no
/// update check, audit or funding call, and nothing else of the harness's.
fn npm_environment(
    support: &Path,
    node: &Path,
    registry: &str,
    pass_environment: &[String],
) -> Vec<(String, OsString)> {
    let mut directories: Vec<PathBuf> = node.parent().map(Path::to_path_buf).into_iter().collect();
    directories.extend(system_path_directories());
    let path = env::join_paths(&directories).unwrap_or_default();
    let tmp = support.join("tmp");
    let home = support.join("home");
    let mut environment: Vec<(String, OsString)> = vec![
        ("PATH".to_owned(), path),
        ("HOME".to_owned(), home.clone().into_os_string()),
        ("USERPROFILE".to_owned(), home.clone().into_os_string()),
        ("APPDATA".to_owned(), home.join("AppData").into_os_string()),
        (
            "LOCALAPPDATA".to_owned(),
            home.join("AppData").join("Local").into_os_string(),
        ),
        ("TEMP".to_owned(), tmp.clone().into_os_string()),
        ("TMP".to_owned(), tmp.clone().into_os_string()),
        ("TMPDIR".to_owned(), tmp.into_os_string()),
        (
            "npm_config_userconfig".to_owned(),
            support.join("user.npmrc").into_os_string(),
        ),
        (
            "npm_config_globalconfig".to_owned(),
            support.join("global.npmrc").into_os_string(),
        ),
        (
            "npm_config_cache".to_owned(),
            support.join("cache").into_os_string(),
        ),
        ("npm_config_registry".to_owned(), registry.into()),
        ("npm_config_update_notifier".to_owned(), "false".into()),
        ("npm_config_audit".to_owned(), "false".into()),
        ("npm_config_fund".to_owned(), "false".into()),
        ("npm_config_ignore_scripts".to_owned(), "true".into()),
        ("npm_config_progress".to_owned(), "false".into()),
        ("npm_config_color".to_owned(), "false".into()),
        ("npm_config_loglevel".to_owned(), "error".into()),
        ("npm_config_fetch_retries".to_owned(), "2".into()),
        ("npm_config_fetch_timeout".to_owned(), "120000".into()),
    ];
    if cfg!(windows) {
        for name in WINDOWS_PASSTHROUGH {
            if let Some(value) = env::var_os(name) {
                environment.push((name.to_owned(), value));
            }
        }
    }
    for name in pass_environment {
        if let Some(value) = env::var_os(name) {
            environment.push((name.clone(), value));
        }
    }
    environment
}

/// One `node <script> <arguments>` run with a cleared environment; the
/// standard output on success.
fn run_node(
    node: &Path,
    script: &Path,
    arguments: &[&str],
    environment: &[(String, OsString)],
    directory: &Path,
) -> Result<std::process::Output, TrialError> {
    Command::new(node)
        .arg(script)
        .args(arguments)
        .env_clear()
        .envs(environment.iter().map(|(name, value)| (name, value)))
        .current_dir(directory)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| TrialError::Process(format!("node did not start: {error}")))
}

/// Asks the real registry through npm itself, with the same cleared
/// environment as the install.
pub struct NpmRegistry {
    node: PathBuf,
    npm_cli: PathBuf,
    environment: Vec<(String, OsString)>,
    directory: PathBuf,
}

impl RegistryReader for NpmRegistry {
    fn dist(&self, name: &str, version: &str) -> Result<Dist, TrialError> {
        let spec = format!("{name}@{version}");
        let output = run_node(
            &self.node,
            &self.npm_cli,
            &["view", &spec, "dist", "--json"],
            &self.environment,
            &self.directory,
        )?;
        if !output.status.success() {
            return Err(TrialError::Process(format!(
                "npm view {spec} failed: {}",
                first_chars(&String::from_utf8_lossy(&output.stderr), 300)
            )));
        }
        let value: Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| TrialError::json(format!("npm view {spec}"), error))?;
        match (value["tarball"].as_str(), value["integrity"].as_str()) {
            (Some(tarball), Some(integrity)) if integrity.starts_with("sha512-") => Ok(Dist {
                tarball: tarball.to_owned(),
                integrity: integrity.to_owned(),
            }),
            _ => Err(TrialError::Invalid(format!(
                "the registry reports no tarball address and sha512 integrity for {spec}"
            ))),
        }
    }
}

/// Runs the launcher with Node.js, as the shim does.
pub struct NodeLauncher {
    node: PathBuf,
    environment: Vec<(String, OsString)>,
}

impl VersionProbe for NodeLauncher {
    fn version(&self, package_root: &Path) -> Result<(Option<i32>, String), TrialError> {
        let output = run_node(
            &self.node,
            &package_root.join("bin").join("vsift.cjs"),
            &["--version"],
            &self.environment,
            package_root,
        )?;
        let line = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        Ok((output.status.code(), line))
    }
}

fn first_chars(text: &str, count: usize) -> String {
    text.trim().chars().take(count).collect()
}

fn create(path: &Path) -> Result<(), TrialError> {
    fs::create_dir_all(path).map_err(|error| TrialError::io_at(path, error))
}

/// Installs `vsift-cli@<version>` from the registry into a new prefix and
/// returns the proof that it is the published package.
///
/// # Errors
///
/// [`TrialError`] when a request value is refused, the prefix exists, npm
/// fails or the install cannot be read back. The proof is returned even
/// when it does not count as published (see
/// [`InstallEvidence::not_published_because`]); the caller decides.
#[allow(
    clippy::too_many_lines,
    reason = "One function reads top to bottom as the order of an installation"
)]
pub fn install(request: &InstallRequest) -> Result<InstallProof, TrialError> {
    validate_version(&request.version)?;
    validate_registry(&request.registry)?;
    request.root_policy.check(&request.prefix)?;
    for (path, what) in [(&request.node, "node"), (&request.npm_cli, "npm-cli.js")] {
        if !path.is_absolute() || !path.is_file() {
            return Err(TrialError::Refused(format!(
                "--{what} must be an absolute path to a file"
            )));
        }
    }
    if request.prefix.exists() {
        return Err(TrialError::Refused(
            "the install prefix already exists; an install is made once".to_owned(),
        ));
    }
    let mut support_name = request.prefix.as_os_str().to_os_string();
    support_name.push(".support");
    let support = PathBuf::from(support_name);
    if support.exists() {
        return Err(TrialError::Refused(
            "the install's scratch folder already exists".to_owned(),
        ));
    }
    for directory in [
        &request.prefix,
        &support.join("cache"),
        &support.join("tmp"),
        &support.join("home"),
    ] {
        create(directory)?;
    }
    for file in ["user.npmrc", "global.npmrc"] {
        fs::write(support.join(file), "")
            .map_err(|error| TrialError::io_at(&support.join(file), error))?;
    }
    let environment = npm_environment(
        &support,
        &request.node,
        &request.registry,
        &request.pass_environment,
    );
    let first_line = |output: &std::process::Output| {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let node_version = first_line(
        &Command::new(&request.node)
            .arg("--version")
            .env_clear()
            .envs(environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::null())
            .output()
            .map_err(|error| TrialError::Process(format!("node did not start: {error}")))?,
    );
    let npm_version = first_line(&run_node(
        &request.node,
        &request.npm_cli,
        &["--version"],
        &environment,
        &support,
    )?);

    let spec = format!("{LAUNCHER_PACKAGE}@{}", request.version);
    let prefix = request.prefix.to_string_lossy().into_owned();
    let output = run_node(
        &request.node,
        &request.npm_cli,
        &[
            "install",
            "--global",
            "--prefix",
            &prefix,
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
            "--no-update-notifier",
            &spec,
        ],
        &environment,
        &support,
    )?;
    if !output.status.success() {
        return Err(TrialError::Process(format!(
            "npm install {spec} failed (exit {:?}): {}",
            output.status.code(),
            first_chars(&String::from_utf8_lossy(&output.stderr), 400)
        )));
    }

    let registry = NpmRegistry {
        node: request.node.clone(),
        npm_cli: request.npm_cli.clone(),
        environment: environment.clone(),
        directory: support.clone(),
    };
    let launcher = NodeLauncher {
        node: request.node.clone(),
        environment: environment
            .iter()
            .filter(|(name, _)| !name.starts_with("npm_config_"))
            .cloned()
            .collect(),
    };
    let proof = verify_install(
        &VerifyRequest {
            prefix: &request.prefix,
            version: &request.version,
            registry: &request.registry,
            source: InstallSourceKind::Registry,
            node: &request.node,
            node_version: &node_version,
            npm_version: &npm_version,
            target: host_target()?,
        },
        &registry,
        &CacheIndex::new(support.join("cache")),
        &launcher,
    )?;
    // The scratch folder holds npm's cache of the tarballs and nothing the
    // trials need; it is ours, created above, and goes with the install.
    fs::remove_dir_all(&support).map_err(|error| TrialError::io_at(&support, error))?;
    Ok(proof)
}
