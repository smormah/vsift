//! Packages `VSift`'s native release archives and their `SHA256SUMS`, and
//! checks a packaged archive against its inputs (P13 PR 8, ADR 0023).
//!
//! The release workflow (`.github/workflows/release.yml`) builds the `vsift`
//! executable for each target, generates that target's notices and SBOM, and
//! then runs this tool from the repository root:
//!
//! ```console
//! vsift-release package --target <triple> --binary <file> --notices <file> \
//!     --sbom <file> --source-date-epoch <seconds> --out-dir <dir>
//! vsift-release verify --target <triple> --binary <file> --notices <file> \
//!     --sbom <file> --source-date-epoch <seconds> --archive <file>
//! vsift-release checksums --output <file> <archive>...
//! ```
//!
//! The tool writes only the files it is asked to create, never overwrites one,
//! and contacts no network.

#![forbid(unsafe_code)]

mod archive;
mod checksums;
mod target;

use std::{
    collections::BTreeMap,
    error::Error,
    fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand};

use crate::{
    archive::{
        ArchiveContents, ArchiveError, LICENCE_FILES, SKILL_DIRECTORY, archive_file_name,
        is_plain_relative_path, verify_archive, write_archive,
    },
    checksums::{ChecksumError, checksum_list},
    target::ReleaseTarget,
};

/// The version every archive is named after: the workspace version, which the
/// `vsift` binary shares (see the test `the_packaged_version_is_the_binarys`).
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The most files the skill directory may hold; it holds a dozen today.
const MAXIMUM_SKILL_FILES: usize = 256;

#[derive(Debug, Parser)]
#[command(
    name = "vsift-release",
    about = "Package and check VSift release archives"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Write `vsift-<version>-<target>.tar.gz` into the output directory.
    Package {
        #[command(flatten)]
        inputs: Inputs,
        /// The source commit's time in seconds since the Unix epoch.
        #[arg(long)]
        source_date_epoch: u64,
        /// An existing directory; the archive must not exist in it yet.
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Check that an archive holds exactly what `package` writes for the
    /// same inputs.
    Verify {
        #[command(flatten)]
        inputs: Inputs,
        /// The source commit's time the archive was packaged with.
        #[arg(long)]
        source_date_epoch: u64,
        /// The archive to read back.
        #[arg(long)]
        archive: PathBuf,
    },
    /// Write a `SHA256SUMS` file listing the given archives by file name.
    Checksums {
        /// The file to create.
        #[arg(long)]
        output: PathBuf,
        /// The archives to list.
        #[arg(required = true)]
        archives: Vec<PathBuf>,
    },
}

/// The inputs of one archive besides the repository's licences and skill.
#[derive(Debug, clap::Args)]
struct Inputs {
    /// The release target the executable was built for.
    #[arg(long, value_enum)]
    target: ReleaseTarget,
    /// The built executable; its file name must be the target's `vsift`
    /// executable name, so no other binary can be packaged.
    #[arg(long)]
    binary: PathBuf,
    /// The target's THIRD-PARTY-NOTICES from cargo-about.
    #[arg(long)]
    notices: PathBuf,
    /// The target's `CycloneDX` SBOM from cargo-cyclonedx.
    #[arg(long)]
    sbom: PathBuf,
}

/// Why the tool stopped.
#[derive(Debug)]
enum ReleaseError {
    /// The executable given is not named as the target's `vsift` executable.
    NotTheVsiftBinary {
        target: ReleaseTarget,
        path: PathBuf,
    },
    /// A file or directory could not be read or written.
    Io { path: PathBuf, source: io::Error },
    /// The skill directory holds something other than plain files and
    /// directories, or too many files.
    UnexpectedSkillEntry(PathBuf),
    /// The archive's contents or layout are wrong.
    Archive(ArchiveError),
    /// The checksum list could not be written.
    Checksums(ChecksumError),
}

impl fmt::Display for ReleaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotTheVsiftBinary { target, path } => write!(
                formatter,
                "{} is not {}; a {target} release packages only that executable",
                path.display(),
                target.executable_name()
            ),
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::UnexpectedSkillEntry(path) => write!(
                formatter,
                "{} is not a plain file or directory of the skill, or the skill has more than \
                 {MAXIMUM_SKILL_FILES} files",
                path.display()
            ),
            Self::Archive(error) => error.fmt(formatter),
            Self::Checksums(error) => error.fmt(formatter),
        }
    }
}

impl Error for ReleaseError {}

impl From<ArchiveError> for ReleaseError {
    fn from(error: ArchiveError) -> Self {
        Self::Archive(error)
    }
}

impl From<ChecksumError> for ReleaseError {
    fn from(error: ChecksumError) -> Self {
        Self::Checksums(error)
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command, Path::new(".")) {
        Ok(created) => {
            println!("{}", created.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("vsift-release: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Runs one command from `repository_root` and returns the file it created or
/// checked.
fn run(command: Command, repository_root: &Path) -> Result<PathBuf, ReleaseError> {
    match command {
        Command::Package {
            inputs,
            source_date_epoch,
            out_dir,
        } => {
            let contents = load_contents(&inputs, repository_root)?;
            let path = out_dir.join(archive_file_name(VERSION, inputs.target));
            let file = create_new(&path)?;
            let file = write_archive(VERSION, inputs.target, source_date_epoch, &contents, file)?;
            file.sync_all().map_err(|source| io_error(&path, source))?;
            Ok(path)
        }
        Command::Verify {
            inputs,
            source_date_epoch,
            archive,
        } => {
            let contents = load_contents(&inputs, repository_root)?;
            let bytes = read(&archive)?;
            verify_archive(VERSION, inputs.target, source_date_epoch, &contents, &bytes)?;
            Ok(archive)
        }
        Command::Checksums { output, archives } => {
            let mut files = Vec::new();
            for archive in &archives {
                let name = archive
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_owned();
                files.push((name, read(archive)?));
            }
            let list = checksum_list(
                files
                    .iter()
                    .map(|(name, bytes)| (name.as_str(), bytes.as_slice())),
            )?;
            let mut file = create_new(&output)?;
            file.write_all(list.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|source| io_error(&output, source))?;
            Ok(output)
        }
    }
}

fn load_contents(inputs: &Inputs, root: &Path) -> Result<ArchiveContents, ReleaseError> {
    let binary_name = inputs.binary.file_name().and_then(|name| name.to_str());
    if binary_name != Some(inputs.target.executable_name()) {
        return Err(ReleaseError::NotTheVsiftBinary {
            target: inputs.target,
            path: inputs.binary.clone(),
        });
    }
    let mut licences = BTreeMap::new();
    for name in LICENCE_FILES {
        licences.insert(name.to_owned(), read(&root.join(name))?);
    }
    Ok(ArchiveContents {
        executable: read(&inputs.binary)?,
        licences,
        notices: read(&inputs.notices)?,
        sbom: read(&inputs.sbom)?,
        skill: load_skill(root)?,
    })
}

/// Every regular file under the skill directory, keyed by its `/`-separated
/// path relative to it. A link or any other kind of entry is refused, so the
/// archive holds the skill's own bytes and nothing it points to.
fn load_skill(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, ReleaseError> {
    let mut files = BTreeMap::new();
    let mut pending = vec![(root.join(SKILL_DIRECTORY), String::new())];
    while let Some((directory, prefix)) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|source| io_error(&directory, source))?;
        for entry in entries {
            let entry = entry.map_err(|source| io_error(&directory, source))?;
            let path = entry.path();
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                return Err(ReleaseError::UnexpectedSkillEntry(path));
            };
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if !is_plain_relative_path(&relative) {
                return Err(ReleaseError::UnexpectedSkillEntry(path));
            }
            let kind = fs::symlink_metadata(&path)
                .map_err(|source| io_error(&path, source))?
                .file_type();
            if kind.is_dir() {
                pending.push((path, relative));
            } else if kind.is_file() && files.len() < MAXIMUM_SKILL_FILES {
                let bytes = read(&path)?;
                files.insert(relative, bytes);
            } else {
                return Err(ReleaseError::UnexpectedSkillEntry(path));
            }
        }
    }
    Ok(files)
}

fn read(path: &Path) -> Result<Vec<u8>, ReleaseError> {
    fs::read(path).map_err(|source| io_error(path, source))
}

fn create_new(path: &Path) -> Result<fs::File, ReleaseError> {
    fs::File::options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| io_error(path, source))
}

fn io_error(path: &Path, source: io::Error) -> ReleaseError {
    ReleaseError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs,
        path::{Path, PathBuf},
    };

    use super::{Command, Inputs, ReleaseError, VERSION, load_skill, run};
    use crate::{
        archive::tests::contents,
        target::{ReleaseTarget, tests::minimal_executable},
    };

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// A private scratch directory for one test, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Result<Self, Box<dyn Error>> {
            let path = std::env::temp_dir()
                .join(format!("vsift-release-test-{name}-{}", std::process::id()));
            if path.exists() {
                fs::remove_dir_all(&path)?;
            }
            fs::create_dir(&path)?;
            Ok(Self(path))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ignored = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_packaged_version_is_the_binarys() -> Result<(), Box<dyn Error>> {
        // Both crates inherit the workspace version, so VERSION (this tool's)
        // is the version `vsift --version` prints.
        let root = repository_root();
        for manifest in [
            "crates/vsift-cli/Cargo.toml",
            "tools/vsift-release/Cargo.toml",
        ] {
            let text = fs::read_to_string(root.join(manifest))?;
            assert!(
                text.lines()
                    .any(|line| line.trim() == "version.workspace = true"),
                "{manifest} must inherit the workspace version"
            );
        }
        assert!(!VERSION.is_empty());
        Ok(())
    }

    #[test]
    fn the_repository_skill_is_packaged_byte_for_byte() -> Result<(), Box<dyn Error>> {
        let root = repository_root();
        let skill = load_skill(&root)?;
        assert!(skill.contains_key("SKILL.md"));
        assert!(skill.contains_key("handoff.schema.json"));
        for (relative, bytes) in &skill {
            assert_eq!(
                &fs::read(root.join("skills/vsift").join(relative))?,
                bytes,
                "{relative}"
            );
        }
        Ok(())
    }

    #[test]
    fn package_verify_and_checksums_round_trip() -> Result<(), Box<dyn Error>> {
        let scratch = Scratch::new("round-trip")?;
        let target = ReleaseTarget::LinuxX64;
        let fixture = contents(target);
        let binary = scratch.0.join("vsift");
        let notices = scratch.0.join("notices.txt");
        let sbom = scratch.0.join("sbom.json");
        fs::write(&binary, &fixture.executable)?;
        fs::write(&notices, &fixture.notices)?;
        fs::write(&sbom, &fixture.sbom)?;
        let out = scratch.0.join("out");
        fs::create_dir(&out)?;
        let inputs = || Inputs {
            target,
            binary: binary.clone(),
            notices: notices.clone(),
            sbom: sbom.clone(),
        };
        let root = repository_root();

        let archive = run(
            Command::Package {
                inputs: inputs(),
                source_date_epoch: 1_790_000_000,
                out_dir: out.clone(),
            },
            &root,
        )?;
        assert_eq!(
            archive.file_name().and_then(|name| name.to_str()),
            Some(format!("vsift-{VERSION}-x86_64-unknown-linux-gnu.tar.gz").as_str())
        );
        run(
            Command::Verify {
                inputs: inputs(),
                source_date_epoch: 1_790_000_000,
                archive: archive.clone(),
            },
            &root,
        )?;
        // An existing archive is never overwritten.
        let again = run(
            Command::Package {
                inputs: inputs(),
                source_date_epoch: 1_790_000_000,
                out_dir: out.clone(),
            },
            &root,
        );
        assert!(matches!(again, Err(ReleaseError::Io { .. })), "{again:?}");

        let sums = out.join("SHA256SUMS");
        run(
            Command::Checksums {
                output: sums.clone(),
                archives: vec![archive],
            },
            &root,
        )?;
        let text = fs::read_to_string(&sums)?;
        assert!(text.ends_with(&format!(
            "  vsift-{VERSION}-x86_64-unknown-linux-gnu.tar.gz\n"
        )));
        assert_eq!(text.lines().count(), 1);
        Ok(())
    }

    #[test]
    fn only_the_vsift_executable_can_be_packaged() -> Result<(), Box<dyn Error>> {
        let scratch = Scratch::new("other-binary")?;
        let target = ReleaseTarget::LinuxX64;
        let fixture = contents(target);
        let binary = scratch.0.join("vsift-smoke-fixture");
        fs::write(&binary, minimal_executable(target))?;
        let notices = scratch.0.join("notices.txt");
        let sbom = scratch.0.join("sbom.json");
        fs::write(&notices, &fixture.notices)?;
        fs::write(&sbom, &fixture.sbom)?;
        let result = run(
            Command::Package {
                inputs: Inputs {
                    target,
                    binary,
                    notices,
                    sbom,
                },
                source_date_epoch: 0,
                out_dir: scratch.0.clone(),
            },
            &repository_root(),
        );
        assert!(
            matches!(result, Err(ReleaseError::NotTheVsiftBinary { .. })),
            "{result:?}"
        );
        assert_eq!(fs::read_dir(&scratch.0)?.count(), 3);
        Ok(())
    }
}
