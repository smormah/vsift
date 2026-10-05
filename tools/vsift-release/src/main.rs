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
//! vsift-release npm --archive <file> --archive <file> --archive <file> \
//!     --out-dir <dir>
//! vsift-release npm-verify --archive <file> (three times) --tarball <file> \
//!     (four times)
//! vsift-release publish-plan --archive <file> (three times) --checksums <file> \
//!     --tarball <file> (four times) --event <event> --ref <ref> \
//!     --repository <owner/name> --dry-run-input <""|true|false> \
//!     --commit <sha> [--registry <dir>] --out-dir <dir>
//! vsift-release candidate-delta [--stable-commit <sha>]
//! ```
//!
//! `npm` assembles the four npm packages from the three archives (P13 PR 9,
//! see the `npm` module); `npm pack` turns each directory into a tarball, and
//! `npm-verify` checks those tarballs against a fresh assembly.
//! `publish-plan` (P13 PR 10, see the `publish` module) checks the archives,
//! `SHA256SUMS` and tarballs again, decides whether the run may publish, and
//! writes the plan the privileged `attest` and `publish` jobs carry out; it
//! prints the plan job's outputs as `name=value` lines. A version without a
//! pre-release suffix is *stable* and moves npm's `latest` (P14 PR 8): its
//! plan also reads the registry files the plan job saved (`--registry`) and
//! compares the commit with the accepted release candidate, as the `guards`,
//! `registry` and `candidate` modules describe. `candidate-delta` runs that
//! comparison alone, for the maintainer's preflight.
//!
//! The tool writes only the files it is asked to create, never overwrites one,
//! publishes nothing and contacts no network. It runs `git` (explicit
//! arguments, no shell, local objects only) for the candidate comparison.

#![forbid(unsafe_code)]

mod archive;
mod candidate;
mod checksums;
mod evidence;
mod guards;
mod notes;
mod npm;
mod publish;
mod registry;
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
        is_plain_relative_path, read_archive, verify_archive, write_archive,
    },
    candidate::{
        CandidateObservation, CheckRecord, github_output as candidate_output, head_commit, observe,
    },
    checksums::{ChecksumError, checksum_list},
    evidence::{EvidenceObservation, read_directory as read_evidence},
    guards::Observations,
    npm::{
        LAUNCHER_DIRECTORY, LAUNCHER_LIBRARY, LAUNCHER_MANIFEST, LAUNCHER_README, LAUNCHER_SCRIPT,
        LauncherSources, NpmError, NpmPackage, assemble, verify_tarball,
    },
    publish::{
        DryRunInput, PLAN_MARKDOWN, PackedPackage, PublishError, ReleaseArchive, ReleaseVersion,
        RunContext, TriggerEvent, plan, publication_order,
    },
    registry::{RegistryObservation, read_directory},
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
    /// Assemble the npm packages from the three release archives, one
    /// directory per package in the output directory.
    Npm {
        /// A release archive, named `vsift-<version>-<target>.tar.gz`; give
        /// one per target.
        #[arg(long = "archive", required = true)]
        archives: Vec<PathBuf>,
        /// An existing directory; the package directories must not exist in
        /// it yet.
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Check tarballs made by `npm pack` against a fresh assembly from the
    /// same archives.
    NpmVerify {
        /// A release archive; give one per target.
        #[arg(long = "archive", required = true)]
        archives: Vec<PathBuf>,
        /// A packed package; give one per package.
        #[arg(long = "tarball", required = true)]
        tarballs: Vec<PathBuf>,
    },
    /// Check the archives, `SHA256SUMS` and the qualified tarballs, decide
    /// whether this workflow run may publish, and write the publish plan into
    /// the output directory. Prints the plan job's outputs.
    PublishPlan(PlanArguments),
    /// Compare the accepted release candidate of this version with a commit
    /// (the stable commit): only version strings, the launcher's README, the
    /// installation guide and the work record may differ. Prints the
    /// comparison and fails if anything else does.
    CandidateDelta {
        /// The stable commit, as a full lowercase SHA; defaults to `HEAD`.
        #[arg(long)]
        stable_commit: Option<String>,
        /// Print only `candidate-version=` and `candidate-commit=` lines for
        /// `$GITHUB_OUTPUT` (nothing when there is no accepted candidate), and
        /// never fail: the plan job's evidence step runs when they are there.
        #[arg(long)]
        github_output: bool,
    },
}

/// The inputs of `publish-plan`: the release's files and the workflow run.
#[derive(Debug, clap::Args)]
struct PlanArguments {
    /// A release archive; give one per target.
    #[arg(long = "archive", required = true)]
    archives: Vec<PathBuf>,
    /// The `SHA256SUMS` file the `package` job wrote.
    #[arg(long)]
    checksums: PathBuf,
    /// A packed package the `npm-qualify` jobs installed; give one per
    /// package.
    #[arg(long = "tarball", required = true)]
    tarballs: Vec<PathBuf>,
    /// `github.event_name`.
    #[arg(long, value_enum)]
    event: TriggerEvent,
    /// `github.ref`.
    #[arg(long = "ref")]
    git_ref: String,
    /// `github.repository`.
    #[arg(long)]
    repository: String,
    /// `github.event.inputs.dry_run`: empty unless dispatched.
    #[arg(long, default_value = "")]
    dry_run_input: String,
    /// `github.sha`.
    #[arg(long)]
    commit: String,
    /// The directory the plan job saved npm's public metadata of the four
    /// packages in (`<name>.json` and `<name>.status` each). Without it the
    /// plan says the registry was not read, and a stable plan is refused
    /// wherever it is enforced.
    #[arg(long)]
    registry: Option<PathBuf>,
    /// The directory the plan job saved the evidence ledger's answer for the
    /// accepted candidate in (`status` and `result.txt`). Without it a stable
    /// plan says the check did not run, and is refused wherever it is
    /// enforced.
    #[arg(long)]
    evidence: Option<PathBuf>,
    /// `github.run_id`, which the evidence ledger's `release_delta` record
    /// names as the run that made the candidate comparison.
    #[arg(long)]
    run_id: Option<u64>,
    /// The date of the run, `YYYY-MM-DD` in UTC, for the same record.
    #[arg(long)]
    date: Option<String>,
    /// An existing directory; the plan's files must not exist in it yet.
    #[arg(long)]
    out_dir: PathBuf,
}

/// What a command produced, which `main` prints.
#[derive(Debug)]
enum Outcome {
    /// The file or directory it created or checked.
    Path(PathBuf),
    /// The plan job's outputs, one `name=value` line each.
    Outputs(String),
    /// A report, printed as it is.
    Report(String),
}

impl Outcome {
    /// The path, for the commands that produce one.
    #[cfg(test)]
    fn path(self) -> Option<PathBuf> {
        match self {
            Self::Path(path) => Some(path),
            Self::Outputs(_) | Self::Report(_) => None,
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(path) => write!(formatter, "{}", path.display()),
            Self::Outputs(text) | Self::Report(text) => formatter.write_str(text),
        }
    }
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
    /// The archive's file name does not name this version and a target.
    UnrecognisedArchive(PathBuf),
    /// The npm packages could not be assembled or a tarball is wrong.
    Npm(NpmError),
    /// Not exactly one tarball was given per package.
    Tarballs(String),
    /// The run may not publish, or its inputs are not the release.
    Publish(PublishError),
    /// The comparison with the release candidate was refused or could not be
    /// made; the text says why.
    Candidate(String),
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
            Self::UnrecognisedArchive(path) => write!(
                formatter,
                "{} is not named vsift-{VERSION}-<target>.tar.gz for a release target",
                path.display()
            ),
            Self::Npm(error) => error.fmt(formatter),
            Self::Tarballs(reason) => {
                write!(formatter, "the tarballs are not one per package: {reason}")
            }
            Self::Publish(error) => error.fmt(formatter),
            Self::Candidate(report) => formatter.write_str(report),
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

impl From<NpmError> for ReleaseError {
    fn from(error: NpmError) -> Self {
        Self::Npm(error)
    }
}

impl From<PublishError> for ReleaseError {
    fn from(error: PublishError) -> Self {
        Self::Publish(error)
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command, Path::new(".")) {
        Ok(outcome) => {
            let text = outcome.to_string();
            if !text.is_empty() {
                println!("{text}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("vsift-release: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Runs one command from `repository_root` and returns what it produced.
fn run(command: Command, repository_root: &Path) -> Result<Outcome, ReleaseError> {
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
            Ok(Outcome::Path(path))
        }
        Command::Verify {
            inputs,
            source_date_epoch,
            archive,
        } => {
            let contents = load_contents(&inputs, repository_root)?;
            let bytes = read(&archive)?;
            verify_archive(VERSION, inputs.target, source_date_epoch, &contents, &bytes)?;
            Ok(Outcome::Path(archive))
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
            write_new(&output, list.as_bytes())?;
            Ok(Outcome::Path(output))
        }
        Command::Npm { archives, out_dir } => {
            let packages = assemble_packages(&read_release_archives(&archives)?, repository_root)?;
            for package in &packages {
                write_package(&out_dir, package)?;
            }
            Ok(Outcome::Path(out_dir))
        }
        Command::NpmVerify { archives, tarballs } => {
            let packages = assemble_packages(&read_release_archives(&archives)?, repository_root)?;
            verify_tarballs(&packages, &tarballs)?;
            Ok(Outcome::Path(
                tarballs.into_iter().next().unwrap_or_default(),
            ))
        }
        Command::PublishPlan(arguments) => write_publish_plan(arguments, repository_root),
        Command::CandidateDelta {
            stable_commit,
            github_output,
        } => candidate_delta(stable_commit, github_output, repository_root),
    }
}

/// Checks the release once more, plans its publication and writes the plan's
/// files; returns the plan job's outputs.
fn write_publish_plan(
    arguments: PlanArguments,
    repository_root: &Path,
) -> Result<Outcome, ReleaseError> {
    let context = RunContext {
        event: arguments.event,
        git_ref: arguments.git_ref,
        repository: arguments.repository,
        dry_run: DryRunInput::parse(&arguments.dry_run_input)?,
        commit: arguments.commit,
    };
    let loaded = read_release_archives(&arguments.archives)?;
    let packages = assemble_packages(&loaded, repository_root)?;
    let packed = verify_tarballs(&packages, &arguments.tarballs)?;
    let release_archives: Vec<ReleaseArchive> = loaded
        .contents
        .into_iter()
        .zip(loaded.files)
        .map(|((target, contents), (file_name, bytes))| ReleaseArchive {
            target,
            file_name,
            bytes,
            sbom: contents.sbom,
            notices: contents.notices,
        })
        .collect();
    let version = ReleaseVersion::parse(VERSION)?;
    let registry = match &arguments.registry {
        Some(directory) => read_directory(directory, &publication_order()),
        None => RegistryObservation::NotRead,
    };
    let evidence = match &arguments.evidence {
        Some(directory) => read_evidence(directory),
        None => EvidenceObservation::NotRun,
    };
    let check = match (arguments.run_id, arguments.date) {
        (Some(run_id), Some(date)) if CheckRecord::is_date(&date) => {
            Some(CheckRecord { run_id, date })
        }
        (Some(_), Some(date)) => return Err(ReleaseError::Publish(PublishError::Date(date))),
        _ => None,
    };
    let observations = Observations {
        registry,
        candidate: observe(repository_root, &context.commit, &version),
        evidence,
        check,
    };
    let plan = plan(
        &version,
        context,
        &release_archives,
        &read(&arguments.checksums)?,
        &packed,
        &observations,
    )?;
    // A refused plan writes only its explanation: the job summary shows why,
    // and no attestation list, release note or command exists to be used.
    if let Some(reasons) = plan.refusal() {
        write_new(
            &arguments.out_dir.join(PLAN_MARKDOWN),
            plan.markdown().as_bytes(),
        )?;
        return Err(ReleaseError::Publish(PublishError::Refused(reasons)));
    }
    for (name, bytes) in plan.files()? {
        write_new(&arguments.out_dir.join(name), &bytes)?;
    }
    Ok(Outcome::Outputs(plan.outputs()))
}

/// Compares the accepted release candidate of this version with a commit and
/// reports whether only version strings, the launcher's README, the
/// installation guide and the work record differ.
fn candidate_delta(
    stable_commit: Option<String>,
    github_output: bool,
    repository_root: &Path,
) -> Result<Outcome, ReleaseError> {
    let version = ReleaseVersion::parse(VERSION)?;
    let commit = match stable_commit {
        Some(commit) => commit,
        None => head_commit(repository_root).map_err(ReleaseError::Candidate)?,
    };
    let observation = observe(repository_root, &commit, &version);
    if github_output {
        return Ok(Outcome::Report(candidate_output(&observation)));
    }
    match observation {
        CandidateObservation::NotApplicable => Err(ReleaseError::Candidate(format!(
            "{VERSION} has a pre-release suffix, so it has no release candidate to compare with"
        ))),
        CandidateObservation::Failed(reason) => Err(ReleaseError::Candidate(reason)),
        CandidateObservation::Checked(report) if report.violations().is_empty() => {
            Ok(Outcome::Report(report.markdown()))
        }
        CandidateObservation::Checked(report) => Err(ReleaseError::Candidate(report.markdown())),
    }
}

/// The release archives read back: each one's contents by target, and in the
/// same order its file name and bytes.
struct LoadedArchives {
    contents: Vec<(ReleaseTarget, ArchiveContents)>,
    files: Vec<(String, Vec<u8>)>,
}

/// Reads each archive back; each must be a canonical release archive of this
/// version, named for its target.
fn read_release_archives(archives: &[PathBuf]) -> Result<LoadedArchives, ReleaseError> {
    let mut loaded = LoadedArchives {
        contents: Vec::new(),
        files: Vec::new(),
    };
    for path in archives {
        let name = path.file_name().and_then(|name| name.to_str());
        let target = ReleaseTarget::ALL
            .into_iter()
            .find(|target| name == Some(archive_file_name(VERSION, *target).as_str()))
            .ok_or_else(|| ReleaseError::UnrecognisedArchive(path.clone()))?;
        let bytes = read(path)?;
        loaded
            .contents
            .push((target, read_archive(VERSION, target, &bytes)?));
        loaded
            .files
            .push((archive_file_name(VERSION, target), bytes));
    }
    Ok(loaded)
}

/// Assembles the npm packages from the archives read back and the
/// launcher's sources in the repository.
fn assemble_packages(
    archives: &LoadedArchives,
    repository_root: &Path,
) -> Result<Vec<NpmPackage>, ReleaseError> {
    let contents = &archives.contents;
    let launcher_root = repository_root.join(LAUNCHER_DIRECTORY);
    let launcher = LauncherSources {
        manifest: read(&launcher_root.join(LAUNCHER_MANIFEST))?,
        script: read(&launcher_root.join(LAUNCHER_SCRIPT))?,
        library: read(&launcher_root.join(LAUNCHER_LIBRARY))?,
        readme: read(&launcher_root.join(LAUNCHER_README))?,
    };
    Ok(assemble(VERSION, contents, &launcher)?)
}

/// Checks each tarball against the assembled packages: exactly one per
/// package, each the package it names, byte for byte.
fn verify_tarballs(
    packages: &[NpmPackage],
    tarballs: &[PathBuf],
) -> Result<Vec<PackedPackage>, ReleaseError> {
    if tarballs.len() != packages.len() {
        return Err(ReleaseError::Tarballs(format!(
            "{} given for {} packages",
            tarballs.len(),
            packages.len()
        )));
    }
    let mut verified: Vec<PackedPackage> = Vec::new();
    for (index, tarball) in tarballs.iter().enumerate() {
        let bytes = read(tarball)?;
        let name = verify_tarball(packages, &bytes, index + 1)?;
        if verified.iter().any(|packed| packed.package == name) {
            return Err(ReleaseError::Tarballs(format!("{name} is given twice")));
        }
        let file_name = tarball
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        verified.push(PackedPackage {
            package: name,
            file_name,
            bytes,
        });
    }
    Ok(verified)
}

/// Writes one package's directory under `out_dir`; the directory must not
/// exist yet. On Unix each file gets its package mode, which `npm pack`
/// records; packing therefore runs on Linux or macOS (the release workflow
/// packs on Ubuntu, and `npm-verify` refuses a tarball whose executable lost
/// its mode).
fn write_package(out_dir: &Path, package: &NpmPackage) -> Result<(), ReleaseError> {
    let root = out_dir.join(package.directory);
    fs::create_dir(&root).map_err(|source| io_error(&root, source))?;
    for (relative, file) in &package.files {
        let path = relative
            .split('/')
            .fold(root.clone(), |path, component| path.join(component));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
        }
        let mut output = create_new(&path)?;
        output
            .write_all(&file.bytes)
            .and_then(|()| output.sync_all())
            .map_err(|source| io_error(&path, source))?;
        set_mode(&path, file.mode)?;
    }
    Ok(())
}

/// Gives a written file its package mode.
#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), ReleaseError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|source| io_error(path, source))
}

/// Windows records no Unix modes; a package directory written here must be
/// packed on Linux or macOS after its modes are set there.
#[cfg(not(unix))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the Unix version can fail; this one has nothing to do"
)]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), ReleaseError> {
    Ok(())
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

/// Writes a new file; an existing one is never overwritten.
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ReleaseError> {
    let mut file = create_new(path)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| io_error(path, source))
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

    use clap::Parser;

    use super::{
        Cli, Command, Inputs, PlanArguments, ReleaseError, VERSION, assemble_packages,
        candidate_delta, load_skill, read_release_archives, run,
    };
    use crate::{
        archive::tests::contents,
        npm,
        publish::{Channel, PublishError, ReleaseVersion, TriggerEvent, npm_tarball_name},
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

    /// The plan job passes GitHub's values as they are: an empty `dry_run`
    /// input outside a dispatch, and the event's own name.
    #[test]
    fn publish_plan_reads_the_workflow_arguments_as_the_plan_job_passes_them()
    -> Result<(), Box<dyn Error>> {
        let parse = |event: &str, dry_run: &str| {
            Cli::try_parse_from([
                "vsift-release",
                "publish-plan",
                "--archive",
                "a.tar.gz",
                "--checksums",
                "SHA256SUMS",
                "--tarball",
                "t.tgz",
                "--event",
                event,
                "--ref",
                "refs/pull/1/merge",
                "--repository",
                "smormah/vsift",
                "--dry-run-input",
                dry_run,
                "--commit",
                "0123456789abcdef0123456789abcdef01234567",
                "--out-dir",
                "publish-plan",
            ])
        };
        for (event, expected) in [
            ("pull_request", TriggerEvent::PullRequest),
            ("push", TriggerEvent::Push),
            ("workflow_dispatch", TriggerEvent::WorkflowDispatch),
        ] {
            let Command::PublishPlan(arguments) = parse(event, "")?.command else {
                return Err("not publish-plan".into());
            };
            assert_eq!(arguments.event, expected);
            assert_eq!(arguments.dry_run_input, "");
        }
        let Command::PublishPlan(arguments) = parse("workflow_dispatch", "false")?.command else {
            return Err("not publish-plan".into());
        };
        assert_eq!(arguments.dry_run_input, "false");
        // An event the Release workflow does not have is refused by the parser.
        assert!(parse("schedule", "").is_err());
        assert!(parse("pull_request_target", "").is_err());
        // The registry directory is optional, and the candidate comparison
        // takes only an optional commit.
        let Command::PublishPlan(arguments) = parse("push", "")?.command else {
            return Err("not publish-plan".into());
        };
        assert_eq!(arguments.registry, None);
        let with_registry = Cli::try_parse_from([
            "vsift-release",
            "publish-plan",
            "--archive",
            "a.tar.gz",
            "--checksums",
            "SHA256SUMS",
            "--tarball",
            "t.tgz",
            "--event",
            "push",
            "--ref",
            "refs/heads/main",
            "--repository",
            "smormah/vsift",
            "--commit",
            "0123456789abcdef0123456789abcdef01234567",
            "--registry",
            "registry",
            "--out-dir",
            "publish-plan",
        ])?;
        let Command::PublishPlan(arguments) = with_registry.command else {
            return Err("not publish-plan".into());
        };
        assert_eq!(arguments.registry, Some(PathBuf::from("registry")));
        let Command::CandidateDelta {
            stable_commit,
            github_output,
        } = Cli::try_parse_from(["vsift-release", "candidate-delta"])?.command
        else {
            return Err("not candidate-delta".into());
        };
        assert_eq!(stable_commit, None);
        assert!(!github_output);
        Ok(())
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
        )?
        .path()
        .ok_or("package names no archive")?;
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

    /// Packages one fixture archive per target into `scratch/archives`.
    fn package_fixture_archives(scratch: &Scratch) -> Result<Vec<PathBuf>, Box<dyn Error>> {
        let root = repository_root();
        let archives_dir = scratch.0.join("archives");
        fs::create_dir(&archives_dir)?;
        let mut archives = Vec::new();
        for target in ReleaseTarget::ALL {
            let fixture = contents(target);
            let inputs_dir = scratch.0.join(target.triple());
            fs::create_dir(&inputs_dir)?;
            let binary = inputs_dir.join(target.executable_name());
            let notices = inputs_dir.join("notices.txt");
            let sbom = inputs_dir.join("sbom.json");
            fs::write(&binary, &fixture.executable)?;
            fs::write(&notices, &fixture.notices)?;
            fs::write(&sbom, &fixture.sbom)?;
            let archive = run(
                Command::Package {
                    inputs: Inputs {
                        target,
                        binary,
                        notices,
                        sbom,
                    },
                    source_date_epoch: 1_790_000_000,
                    out_dir: archives_dir.clone(),
                },
                &root,
            )?
            .path()
            .ok_or("package names no archive")?;
            archives.push(archive);
        }
        Ok(archives)
    }

    #[test]
    fn npm_packages_are_assembled_from_packaged_archives() -> Result<(), Box<dyn Error>> {
        let scratch = Scratch::new("npm")?;
        let root = repository_root();
        let archives = package_fixture_archives(&scratch)?;
        let archives_dir = scratch.0.join("archives");
        let out = scratch.0.join("npm");
        fs::create_dir(&out)?;
        run(
            Command::Npm {
                archives: archives.clone(),
                out_dir: out.clone(),
            },
            &root,
        )?;
        for directory in [
            "vsift-cli",
            "vsift-darwin-arm64",
            "vsift-linux-x64",
            "vsift-win32-x64",
        ] {
            assert!(
                out.join(directory).join("package.json").is_file(),
                "{directory}"
            );
        }
        assert_eq!(
            fs::read(out.join("vsift-cli/bin/vsift.cjs"))?,
            fs::read(root.join("npm/vsift-cli/bin/vsift.cjs"))?
        );
        assert_eq!(
            fs::read(out.join("vsift-cli/skills/vsift/SKILL.md"))?,
            fs::read(root.join("skills/vsift/SKILL.md"))?
        );
        assert!(out.join("vsift-win32-x64/vsift.exe").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(out.join("vsift-linux-x64/vsift"))?
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        // Existing package directories are never overwritten.
        let again = run(
            Command::Npm {
                archives: archives.clone(),
                out_dir: out,
            },
            &root,
        );
        assert!(matches!(again, Err(ReleaseError::Io { .. })), "{again:?}");

        // An archive must be named for this version and a target.
        let renamed = archives_dir.join("vsift.tar.gz");
        fs::copy(&archives[0], &renamed)?;
        let unrecognised = run(
            Command::Npm {
                archives: vec![renamed],
                out_dir: scratch.0.clone(),
            },
            &root,
        );
        assert!(
            matches!(unrecognised, Err(ReleaseError::UnrecognisedArchive(_))),
            "{unrecognised:?}"
        );
        Ok(())
    }

    /// The whole dry-run path the Release workflow's `plan` job takes on a
    /// pull request: archives, `SHA256SUMS` and packed tarballs in, the plan
    /// files and `mode=dry-run` out; a request to publish off the release tag
    /// and a tarball that is not the assembled package are refused.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one fixture release, planned and then refused three ways"
    )]
    fn publish_plan_checks_everything_and_writes_the_plan() -> Result<(), Box<dyn Error>> {
        let scratch = Scratch::new("publish-plan")?;
        let root = repository_root();
        let archives = package_fixture_archives(&scratch)?;
        let sums = scratch.0.join("SHA256SUMS");
        run(
            Command::Checksums {
                output: sums.clone(),
                archives: archives.clone(),
            },
            &root,
        )?;
        let packages = assemble_packages(&read_release_archives(&archives)?, &root)?;
        let tarball_dir = scratch.0.join("tarballs");
        fs::create_dir(&tarball_dir)?;
        let mut tarballs = Vec::new();
        for package in &packages {
            let path = tarball_dir.join(npm_tarball_name(package.name, VERSION));
            fs::write(&path, npm::tests::pack(package)?)?;
            tarballs.push(path);
        }
        let command = |event, git_ref: &str, dry_run: &str, out_dir: PathBuf, tarballs| {
            Command::PublishPlan(PlanArguments {
                archives: archives.clone(),
                checksums: sums.clone(),
                tarballs,
                event,
                git_ref: git_ref.to_owned(),
                repository: String::from("smormah/vsift"),
                dry_run_input: dry_run.to_owned(),
                commit: String::from("0123456789abcdef0123456789abcdef01234567"),
                registry: None,
                evidence: None,
                run_id: None,
                date: None,
                out_dir,
            })
        };

        let out = scratch.0.join("plan");
        fs::create_dir(&out)?;
        let outcome = run(
            command(
                TriggerEvent::PullRequest,
                "refs/pull/1/merge",
                "",
                out.clone(),
                tarballs.clone(),
            ),
            &root,
        )?;
        // The version decides the channel (a version without a suffix is stable).
        let channel = ReleaseVersion::parse(VERSION)?.kind().channel().output();
        assert_eq!(
            outcome.to_string(),
            format!("mode=dry-run\nversion={VERSION}\ntag=v{VERSION}\nchannel={channel}")
        );
        let mut written: Vec<String> = fs::read_dir(&out)?
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        written.sort();
        assert_eq!(written.len(), 11, "{written:#?}");
        for name in [
            "attestation-subjects.sha256",
            "publish-plan.json",
            "publish-plan.md",
            "release-assets.sha256",
            "release-notes.md",
        ] {
            assert!(written.iter().any(|file| file == name), "{name}");
        }
        // The SBOM asset is the archive's own SBOM, byte for byte.
        assert_eq!(
            fs::read(out.join(format!("vsift-{VERSION}-x86_64-unknown-linux-gnu.cdx.json")))?,
            contents(ReleaseTarget::LinuxX64).sbom
        );
        // A plan is never written over an existing one.
        let again = run(
            command(
                TriggerEvent::PullRequest,
                "refs/pull/1/merge",
                "",
                out,
                tarballs.clone(),
            ),
            &root,
        );
        assert!(matches!(again, Err(ReleaseError::Io { .. })), "{again:?}");

        let elsewhere = scratch.0.join("elsewhere");
        fs::create_dir(&elsewhere)?;
        let off_tag = run(
            command(
                TriggerEvent::WorkflowDispatch,
                "refs/heads/main",
                "false",
                elsewhere.clone(),
                tarballs.clone(),
            ),
            &root,
        );
        assert!(
            matches!(
                off_tag,
                Err(ReleaseError::Publish(PublishError::NotOnReleaseTag { .. }))
            ),
            "{off_tag:?}"
        );

        let mut swapped = tarballs.clone();
        swapped.swap(0, 1);
        let renamed_dir = scratch.0.join("renamed");
        fs::create_dir(&renamed_dir)?;
        let mut misnamed = Vec::new();
        for (source, target) in tarballs.iter().zip(&swapped) {
            let path = renamed_dir.join(target.file_name().ok_or("no name")?);
            fs::copy(source, &path)?;
            misnamed.push(path);
        }
        let refused = run(
            command(
                TriggerEvent::PullRequest,
                "refs/pull/1/merge",
                "",
                elsewhere,
                misnamed,
            ),
            &root,
        );
        assert!(
            matches!(
                refused,
                Err(ReleaseError::Publish(PublishError::Tarballs(_)))
            ),
            "{refused:?}"
        );
        Ok(())
    }

    /// The fixture release's archives, `SHA256SUMS` and packed tarballs.
    struct FixtureRelease {
        archives: Vec<PathBuf>,
        sums: PathBuf,
        tarballs: Vec<PathBuf>,
    }

    fn fixture_release(scratch: &Scratch) -> Result<FixtureRelease, Box<dyn Error>> {
        let root = repository_root();
        let archives = package_fixture_archives(scratch)?;
        let sums = scratch.0.join("SHA256SUMS");
        run(
            Command::Checksums {
                output: sums.clone(),
                archives: archives.clone(),
            },
            &root,
        )?;
        let packages = assemble_packages(&read_release_archives(&archives)?, &root)?;
        let tarball_dir = scratch.0.join("tarballs");
        fs::create_dir(&tarball_dir)?;
        let mut tarballs = Vec::new();
        for package in &packages {
            let path = tarball_dir.join(npm_tarball_name(package.name, VERSION));
            fs::write(&path, npm::tests::pack(package)?)?;
            tarballs.push(path);
        }
        Ok(FixtureRelease {
            archives,
            sums,
            tarballs,
        })
    }

    fn written_names(directory: &Path) -> Result<Vec<String>, Box<dyn Error>> {
        let mut names: Vec<String> = fs::read_dir(directory)?
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        names.sort();
        Ok(names)
    }

    /// The rehearsal on the release tag: a dispatch on `v<version>` with
    /// `dry_run` set is enforced, so a stable version without a registry read
    /// and a candidate is refused, writes only its explanation and fails the
    /// run; a pre-release plans as usual. (This workspace's own version decides
    /// which; the unit tests of the `publish` module hold both.)
    #[test]
    fn an_enforced_plan_that_fails_a_guard_writes_only_its_explanation()
    -> Result<(), Box<dyn Error>> {
        let scratch = Scratch::new("refused-plan")?;
        let root = repository_root();
        let FixtureRelease {
            archives,
            sums,
            tarballs,
        } = fixture_release(&scratch)?;
        let out = scratch.0.join("plan");
        fs::create_dir(&out)?;
        let outcome = run(
            Command::PublishPlan(PlanArguments {
                archives,
                checksums: sums,
                tarballs,
                event: TriggerEvent::WorkflowDispatch,
                git_ref: format!("refs/tags/v{VERSION}"),
                repository: String::from("smormah/vsift"),
                dry_run_input: String::from("true"),
                commit: String::from("0123456789abcdef0123456789abcdef01234567"),
                registry: None,
                evidence: None,
                run_id: None,
                date: None,
                out_dir: out.clone(),
            }),
            &root,
        );
        if ReleaseVersion::parse(VERSION)?.kind().channel() == Channel::Stable {
            assert!(
                matches!(
                    outcome,
                    Err(ReleaseError::Publish(PublishError::Refused(_)))
                ),
                "{outcome:?}"
            );
            assert_eq!(written_names(&out)?, ["publish-plan.md"]);
            let markdown = fs::read_to_string(out.join("publish-plan.md"))?;
            assert!(markdown.starts_with("## Publish plan: REFUSED, nothing is published"));
            assert!(markdown.contains("Registry read"));
        } else {
            outcome?;
            assert_eq!(written_names(&out)?.len(), 11);
        }
        Ok(())
    }

    /// The registry files the plan job saves are read by `--registry` and
    /// shown in the plan; nothing of the metadata but the tags and versions
    /// reaches it.
    #[test]
    fn the_registry_files_are_read_and_shown_in_the_plan() -> Result<(), Box<dyn Error>> {
        let scratch = Scratch::new("registry-plan")?;
        let root = repository_root();
        let FixtureRelease {
            archives,
            sums,
            tarballs,
        } = fixture_release(&scratch)?;
        let registry = scratch.0.join("registry");
        fs::create_dir(&registry)?;
        for stem in [
            "vsift-cli",
            "vsift-darwin-arm64",
            "vsift-linux-x64",
            "vsift-win32-x64",
        ] {
            fs::write(registry.join(format!("{stem}.status")), "200\n")?;
            fs::write(
                registry.join(format!("{stem}.json")),
                r#"{"dist-tags": {"latest": "0.0.0", "next": "0.1.0"},
                    "versions": {"0.0.0": {"maintainers": [{"name": "a-person", "email": "a-person@example.invalid"}]},
                                 "0.1.0": {}}}"#,
            )?;
        }
        let out = scratch.0.join("plan");
        fs::create_dir(&out)?;
        run(
            Command::PublishPlan(PlanArguments {
                archives,
                checksums: sums,
                tarballs,
                event: TriggerEvent::PullRequest,
                git_ref: String::from("refs/pull/1/merge"),
                repository: String::from("smormah/vsift"),
                dry_run_input: String::new(),
                commit: String::from("0123456789abcdef0123456789abcdef01234567"),
                registry: Some(registry),
                evidence: None,
                run_id: None,
                date: None,
                out_dir: out.clone(),
            }),
            &root,
        )?;
        let markdown = fs::read_to_string(out.join("publish-plan.md"))?;
        let expected = match ReleaseVersion::parse(VERSION)?.kind().channel() {
            Channel::Stable => "| `vsift-cli` | `0.0.0` | `",
            Channel::PreRelease => "| `vsift-cli` | `0.1.0` | `",
        };
        assert!(markdown.contains(expected), "{markdown}");
        for file in written_names(&out)? {
            let text = fs::read_to_string(out.join(&file)).unwrap_or_default();
            assert!(!text.contains("a-person"), "{file}");
            assert!(!text.contains("example.invalid"), "{file}");
        }
        Ok(())
    }

    #[test]
    fn the_candidate_comparison_answers_in_a_typed_way_on_this_repository() {
        // Whatever this checkout holds, the answer is a report or a typed
        // refusal, never a crash: no candidate tag, a pre-release version, or
        // (after the stable's own commit) the real comparison.
        let result = candidate_delta(None, false, &repository_root());
        assert!(
            matches!(result, Ok(_) | Err(ReleaseError::Candidate(_))),
            "{result:?}"
        );
        let absent = candidate_delta(Some(String::from("main")), false, &repository_root());
        assert!(
            matches!(absent, Err(ReleaseError::Candidate(_))),
            "{absent:?}"
        );
    }
}
