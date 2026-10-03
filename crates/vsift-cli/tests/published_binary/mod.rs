//! The `vsift` executable an opt-in real-tool checkpoint drives (P14 PR 3, RQ-05).
//!
//! By default a checkpoint drives the binary Cargo builds inside the test run,
//! as it always has. P14 must also show that the **published** binary completes
//! the same journeys, so one explicit, typed override selects another
//! executable, for every checkpoint through this one module:
//!
//! ```console
//! VSIFT_E2E_BINARY=<absolute path of the installed vsift executable>
//! VSIFT_E2E_EXPECTED_VERSION=0.1.0
//! VSIFT_E2E_EXPECTED_COMMIT=<the commit the version's tag points at>
//! cargo test --release -p vsift-cli --test p09_evidence_e2e -- --ignored --nocapture
//! ```
//!
//! **Why not `CARGO_BIN_EXE_vsift`.** `assert_cmd` 2.2.2 does read that
//! variable when a test runs, but `cargo test` sets it itself for every test
//! process and so replaces any value given from outside (checked on 2026-10-02:
//! a nonexistent path in the variable changes nothing). Only a variable Cargo
//! does not own can select another binary.
//!
//! **The rules.** The override is never set by default and nothing here sets
//! it. When `VSIFT_E2E_BINARY` is present it is refused, never ignored, unless:
//! the path is absolute, names a file that exists, `VSIFT_E2E_EXPECTED_VERSION`
//! and `VSIFT_E2E_EXPECTED_COMMIT` (at least twelve lowercase hexadecimal
//! digits of the commit) are both set, and `<path> --version` prints exactly
//! `vsift <version> (<commit>)` for that version and a commit the expected
//! one starts with. A binary that names no commit (a build from source without
//! `VSIFT_SOURCE_COMMIT`) is refused too, because its source cannot be checked.
//! Without the variable the Cargo-built binary runs and nothing else changes.
//!
//! The expected version and commit are explicit rather than read from the test
//! crate, because the tests may be compiled from a different commit than the
//! binary they drive (a tag that predates this module, run with today's tests).
//! The checkpoint reports record which binary ran ([`report`]).
//!
//! The module is included by `mod published_binary;` in every checkpoint, so
//! each test crate uses a part of it.

#![allow(
    dead_code,
    reason = "each checkpoint crate includes this module and uses only part of it"
)]

use std::{
    env,
    error::Error,
    ffi::OsString,
    fmt::{self, Write as _},
    fs,
    path::{Path, PathBuf},
    process::Command as Process,
    sync::OnceLock,
    time::Duration,
};

use assert_cmd::Command;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The variable that selects the executable under test.
pub(crate) const BINARY_VARIABLE: &str = "VSIFT_E2E_BINARY";
/// The version the selected executable must print.
pub(crate) const VERSION_VARIABLE: &str = "VSIFT_E2E_EXPECTED_VERSION";
/// The source commit the selected executable must name.
pub(crate) const COMMIT_VARIABLE: &str = "VSIFT_E2E_EXPECTED_COMMIT";

/// How long `--version` may take: a start, not a journey.
const VERSION_DEADLINE: Duration = Duration::from_secs(60);
/// A release build shows this many digits of its commit.
const SHORTEST_COMMIT: usize = 12;
const LONGEST_COMMIT: usize = 40;

/// Why an override was refused, or why the binary could not be started.
///
/// `Debug` prints the same sentence as `Display`: a checkpoint that returns this
/// error ends with the harness printing its `Debug` form, and the sentence is
/// what the person reading the failed run needs.
#[derive(Clone, Eq, PartialEq)]
pub(crate) enum BinaryError {
    /// A variable is set but holds nothing.
    Empty { variable: &'static str },
    /// A variable does not hold text.
    NotUnicode { variable: &'static str },
    /// The path is not absolute, so what it names depends on the working directory.
    RelativePath(String),
    /// Nothing exists at the path.
    Missing(String),
    /// The path names something other than a file.
    NotAFile(String),
    /// An override is set without the version or commit it must be checked against.
    ExpectationNotSet { variable: &'static str },
    /// The expected version or commit is not well formed.
    ExpectationMalformed {
        variable: &'static str,
        value: String,
    },
    /// `--version` could not be run, or failed.
    VersionCommandFailed(String),
    /// `--version` printed something other than `vsift <version>[ (<commit>)]`.
    VersionOutputMalformed(String),
    /// The binary does not name the commit it was built from.
    NoCommitInVersion(String),
    /// The binary is another version than expected.
    VersionMismatch { expected: String, found: String },
    /// The binary was built from another commit than expected.
    CommitMismatch { expected: String, found: String },
    /// The binary could not be read to be fingerprinted.
    Unreadable(String),
    /// The Cargo-built binary could not be found.
    CargoBuiltMissing(String),
}

impl fmt::Display for BinaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty { variable } => write!(formatter, "{variable} is set but empty"),
            Self::NotUnicode { variable } => write!(formatter, "{variable} is not text"),
            Self::RelativePath(path) => write!(
                formatter,
                "{BINARY_VARIABLE} must be an absolute path, not {path:?}"
            ),
            Self::Missing(path) => write!(formatter, "{BINARY_VARIABLE} names no file: {path:?}"),
            Self::NotAFile(path) => {
                write!(
                    formatter,
                    "{BINARY_VARIABLE} does not name a file: {path:?}"
                )
            }
            Self::ExpectationNotSet { variable } => write!(
                formatter,
                "{BINARY_VARIABLE} is set, so {variable} must be set too: the binary is \
                 refused unless it names the version and commit it is expected to be"
            ),
            Self::ExpectationMalformed { variable, value } => {
                write!(formatter, "{variable} is not well formed: {value:?}")
            }
            Self::VersionCommandFailed(reason) => {
                write!(formatter, "`--version` could not be run: {reason}")
            }
            Self::VersionOutputMalformed(output) => write!(
                formatter,
                "`--version` did not print `vsift <version> (<commit>)`: {output:?}"
            ),
            Self::NoCommitInVersion(output) => write!(
                formatter,
                "the binary names no source commit ({output:?}), so it cannot be matched to \
                 the expected one: use a release build"
            ),
            Self::VersionMismatch { expected, found } => write!(
                formatter,
                "the binary is version {found:?}, expected {expected:?}"
            ),
            Self::CommitMismatch { expected, found } => write!(
                formatter,
                "the binary names commit {found:?}, which {expected:?} does not start with"
            ),
            Self::Unreadable(reason) => write!(formatter, "the binary could not be read: {reason}"),
            Self::CargoBuiltMissing(reason) => {
                write!(formatter, "the Cargo-built vsift is missing: {reason}")
            }
        }
    }
}

impl fmt::Debug for BinaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for BinaryError {}

/// What a binary's `--version` printed: `vsift <version>[ (<commit>)]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VersionLine {
    pub(crate) version: String,
    pub(crate) commit: Option<String>,
}

/// The version and commit an override must name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Expectation {
    pub(crate) version: String,
    pub(crate) commit: String,
}

/// A request to run another binary, read from the environment but not yet checked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OverrideRequest {
    pub(crate) path: PathBuf,
    pub(crate) expectation: Expectation,
}

/// Whether `text` is a version of the form `1.2.3` or `1.2.3-rc.1`.
fn is_version(text: &str) -> bool {
    let (core, pre_release) = match text.split_once('-') {
        Some((core, pre_release)) => (core, Some(pre_release)),
        None => (text, None),
    };
    let numbers: Vec<&str> = core.split('.').collect();
    numbers.len() == 3
        && numbers
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && pre_release.is_none_or(|pre_release| {
            !pre_release.is_empty()
                && pre_release
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
        })
}

/// Whether `text` is a lowercase hexadecimal commit of 12 to 40 digits.
fn is_commit(text: &str) -> bool {
    (SHORTEST_COMMIT..=LONGEST_COMMIT).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Parses `vsift <version>` or `vsift <version> (<commit>)`, one line.
pub(crate) fn parse_version_output(output: &str) -> Result<VersionLine, BinaryError> {
    let malformed = || BinaryError::VersionOutputMalformed(output.chars().take(120).collect());
    let line = output
        .strip_suffix("\r\n")
        .or_else(|| output.strip_suffix('\n'))
        .unwrap_or(output);
    let rest = line.strip_prefix("vsift ").ok_or_else(malformed)?;
    let (version, commit) = match rest.split_once(" (") {
        Some((version, tail)) => {
            let commit = tail.strip_suffix(')').ok_or_else(malformed)?;
            (version, Some(commit))
        }
        None => (rest, None),
    };
    if !is_version(version) || commit.is_some_and(|commit| !is_commit(commit)) {
        return Err(malformed());
    }
    Ok(VersionLine {
        version: version.to_owned(),
        commit: commit.map(str::to_owned),
    })
}

/// Reads an override from the environment (`lookup`): `Ok(None)` when it is not
/// requested, an error when it is requested and not acceptable. Nothing is run
/// and the file system is only asked whether the file exists.
pub(crate) fn read_override(
    lookup: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Option<OverrideRequest>, BinaryError> {
    let Some(raw_path) = lookup(BINARY_VARIABLE) else {
        return Ok(None);
    };
    if raw_path.is_empty() {
        return Err(BinaryError::Empty {
            variable: BINARY_VARIABLE,
        });
    }
    let path = PathBuf::from(raw_path);
    if !path.is_absolute() {
        return Err(BinaryError::RelativePath(path.display().to_string()));
    }
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(BinaryError::NotAFile(path.display().to_string())),
        Err(_) => return Err(BinaryError::Missing(path.display().to_string())),
    }
    let text = |variable: &'static str| -> Result<String, BinaryError> {
        let value = lookup(variable).ok_or(BinaryError::ExpectationNotSet { variable })?;
        let value = value
            .into_string()
            .map_err(|_| BinaryError::NotUnicode { variable })?;
        if value.is_empty() {
            return Err(BinaryError::Empty { variable });
        }
        Ok(value)
    };
    let version = text(VERSION_VARIABLE)?;
    if !is_version(&version) {
        return Err(BinaryError::ExpectationMalformed {
            variable: VERSION_VARIABLE,
            value: version,
        });
    }
    let commit = text(COMMIT_VARIABLE)?;
    if !is_commit(&commit) {
        return Err(BinaryError::ExpectationMalformed {
            variable: COMMIT_VARIABLE,
            value: commit,
        });
    }
    Ok(Some(OverrideRequest {
        path,
        expectation: Expectation { version, commit },
    }))
}

/// Checks a binary's `--version` text against what is expected.
pub(crate) fn check_version_line(
    output: &str,
    expectation: &Expectation,
) -> Result<VersionLine, BinaryError> {
    let line = parse_version_output(output)?;
    let found = line
        .commit
        .as_deref()
        .ok_or_else(|| BinaryError::NoCommitInVersion(output.trim_end().to_owned()))?;
    if line.version != expectation.version {
        return Err(BinaryError::VersionMismatch {
            expected: expectation.version.clone(),
            found: line.version,
        });
    }
    // The binary shows twelve digits of its commit and the expectation may
    // hold all forty, so either may be the longer.
    if !(expectation.commit.starts_with(found) || found.starts_with(&expectation.commit)) {
        return Err(BinaryError::CommitMismatch {
            expected: expectation.commit.clone(),
            found: found.to_owned(),
        });
    }
    Ok(line)
}

/// Runs `<path> --version` and returns what it printed.
pub(crate) fn run_version(path: &Path) -> Result<String, BinaryError> {
    let output = Command::new(path)
        .arg("--version")
        .timeout(VERSION_DEADLINE)
        .output()
        .map_err(|error| BinaryError::VersionCommandFailed(error.to_string()))?;
    if !output.status.success() {
        return Err(BinaryError::VersionCommandFailed(format!(
            "exit status {:?}",
            output.status.code()
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| BinaryError::VersionOutputMalformed(String::from("not UTF-8")))
}

/// The SHA-256 of a file, lowercase hexadecimal.
fn sha256_of(path: &Path) -> Result<String, BinaryError> {
    let bytes = fs::read(path).map_err(|error| BinaryError::Unreadable(error.to_string()))?;
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(hex, "{byte:02x}").map_err(|error| BinaryError::Unreadable(error.to_string()))?;
    }
    Ok(hex)
}

/// The executable a checkpoint drives, after any override was accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Selected {
    /// The binary Cargo built for this test run.
    CargoBuilt,
    /// An installed or published binary that named the expected version and commit.
    Override {
        path: PathBuf,
        version_line: String,
        sha256: String,
    },
}

/// Accepts or refuses the override named by `lookup`, running the binary only
/// through `run_version` (a parameter so a test can answer for a binary that
/// does not exist).
pub(crate) fn select(
    lookup: &dyn Fn(&str) -> Option<OsString>,
    run_version: &dyn Fn(&Path) -> Result<String, BinaryError>,
) -> Result<Selected, BinaryError> {
    let Some(request) = read_override(lookup)? else {
        return Ok(Selected::CargoBuilt);
    };
    let output = run_version(&request.path)?;
    let line = check_version_line(&output, &request.expectation)?;
    Ok(Selected::Override {
        sha256: sha256_of(&request.path)?,
        version_line: format!(
            "vsift {} ({})",
            line.version,
            line.commit.unwrap_or_default()
        ),
        path: request.path,
    })
}

/// The selection for this process, decided once: the override is verified by
/// running `--version` once, not before every command.
fn selected() -> Result<&'static Selected, BinaryError> {
    static SELECTED: OnceLock<Result<Selected, BinaryError>> = OnceLock::new();
    SELECTED
        .get_or_init(|| select(&|name| env::var_os(name), &run_version))
        .as_ref()
        .map_err(Clone::clone)
}

/// A bounded-command builder for the binary under test.
pub(crate) fn command() -> Result<Command, BinaryError> {
    match selected()? {
        Selected::CargoBuilt => Command::cargo_bin("vsift")
            .map_err(|error| BinaryError::CargoBuiltMissing(error.to_string())),
        Selected::Override { path, .. } => Ok(Command::new(path)),
    }
}

/// A plain process builder for the binary under test, for a checkpoint that
/// spawns and signals the child itself.
pub(crate) fn process() -> Result<Process, BinaryError> {
    match selected()? {
        Selected::CargoBuilt => Ok(Process::new(assert_cmd::cargo::cargo_bin("vsift"))),
        Selected::Override { path, .. } => Ok(Process::new(path)),
    }
}

/// The executable's path, for a checkpoint that registers it as a stand-in.
pub(crate) fn path() -> Result<PathBuf, BinaryError> {
    match selected()? {
        Selected::CargoBuilt => Ok(assert_cmd::cargo::cargo_bin("vsift")),
        Selected::Override { path, .. } => Ok(path.clone()),
    }
}

/// Whether the binary under test is an accepted override.
pub(crate) fn is_override() -> Result<bool, BinaryError> {
    Ok(matches!(selected()?, Selected::Override { .. }))
}

/// A report entry that names the binary the checkpoint drove: its source, and
/// for an override its `--version` line and SHA-256, never its path.
pub(crate) fn report() -> Result<Value, BinaryError> {
    Ok(match selected()? {
        Selected::CargoBuilt => json!({
            "source": "cargo_built",
            "note": "compiled by this test run, not an installed or published binary",
        }),
        Selected::Override {
            path,
            version_line,
            sha256,
        } => json!({
            "source": "override",
            "file_name": path.file_name().and_then(|name| name.to_str()),
            "version_line": version_line,
            "sha256": sha256,
        }),
    })
}
