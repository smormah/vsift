//! The publish plan (P13 PR 10, ADR 0023 section 1 and decisions B and C).
//!
//! The Release workflow's unprivileged `plan` job runs `vsift-release
//! publish-plan` on every run. It checks the release archives, their
//! `SHA256SUMS` and the npm tarballs the `npm-qualify` jobs installed, decides
//! whether this run may publish at all, and writes down exactly what the
//! privileged `attest` and `publish` jobs would do: the files to attest, the
//! `npm publish` commands in order, and the GitHub pre-release with its
//! assets. A dry run (every run that is not the maintainer's dispatch of the
//! release tag with `dry_run` cleared) stops there, so the plan is what a dry
//! run shows; the privileged jobs only execute it after the `release`
//! environment's approval.
//!
//! Only this module decides the mode, the dist-tag and the arguments. The
//! workflow's two privileged jobs repeat the same commands in a few lines of
//! shell, so that no Rust is compiled where an OIDC token is available; a test
//! here holds the workflow's commands to the ones this module builds.

use std::fmt::{self, Write};

use clap::ValueEnum;
use serde_json::{Value, json};

use crate::{
    archive::archive_file_name, checksums::checksum_list, npm::LAUNCHER_PACKAGE, npm::sha256_hex,
    target::ReleaseTarget,
};

/// The only repository a release may be published from.
pub(crate) const REPOSITORY: &str = "smormah/vsift";

/// The workflow npm's trusted publishers and the attestations name.
pub(crate) const WORKFLOW_PATH: &str = ".github/workflows/release.yml";

/// The dist-tag every R0 publication uses (ADR 0023 decision B): a 0.x
/// pre-release goes under `next`, and `latest` stays the `vsift-cli@0.0.0`
/// placeholder until a stable release, which is P14's to plan.
pub(crate) const DIST_TAG: &str = "next";

/// The plan's format identifier.
const PLAN_FORMAT: &str = "vsift-publish-plan/1";

/// Where the privileged jobs download the tarballs, the plan and the release
/// assets (the artifact download paths in `release.yml`).
pub(crate) const TARBALL_DIRECTORY: &str = "npm-packages";
/// The directory the plan's files are downloaded to.
pub(crate) const PLAN_DIRECTORY: &str = "publish-plan";
/// The directory the GitHub release's assets are gathered in.
pub(crate) const ASSET_DIRECTORY: &str = "release-assets";

/// The flags of every `npm publish`: the `next` dist-tag, public access (a
/// scope's first publish is otherwise restricted), npm provenance (automatic
/// with trusted publishing, and required with the bootstrap token), and no
/// lifecycle scripts (a tarball has none; this makes sure none runs).
pub(crate) const NPM_PUBLISH_FLAGS: [&str; 6] = [
    "--tag",
    DIST_TAG,
    "--access",
    "public",
    "--provenance",
    "--ignore-scripts",
];

/// The flags of `gh release create`: the tag must already exist (the
/// maintainer creates it under the tag ruleset; the workflow never creates a
/// tag), the release starts as a draft so a half-uploaded release is never
/// public, and it is a pre-release that GitHub does not mark as latest.
pub(crate) const GITHUB_RELEASE_FLAGS: [&str; 4] =
    ["--verify-tag", "--draft", "--prerelease", "--latest=false"];

/// The plan's file names inside [`PLAN_DIRECTORY`].
pub(crate) const PLAN_JSON: &str = "publish-plan.json";
/// The plan in words, which the workflow adds to the job summary.
pub(crate) const PLAN_MARKDOWN: &str = "publish-plan.md";
/// The GitHub release's notes.
pub(crate) const RELEASE_NOTES: &str = "release-notes.md";
/// Every file the `attest` job attests, in `sha256sum` format.
pub(crate) const SUBJECTS_LIST: &str = "attestation-subjects.sha256";
/// Every asset of the GitHub release, in `sha256sum` format.
pub(crate) const ASSETS_LIST: &str = "release-assets.sha256";

/// The event that started the workflow run, as `github.event_name` names it.
/// The Release workflow has exactly these three triggers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum TriggerEvent {
    /// A pull request to `main`.
    #[value(name = "pull_request")]
    PullRequest,
    /// A push to `main`.
    #[value(name = "push")]
    Push,
    /// A manual dispatch, the only event that can publish.
    #[value(name = "workflow_dispatch")]
    WorkflowDispatch,
}

impl TriggerEvent {
    const fn name(self) -> &'static str {
        match self {
            Self::PullRequest => "pull_request",
            Self::Push => "push",
            Self::WorkflowDispatch => "workflow_dispatch",
        }
    }
}

/// The dispatch's `dry_run` input as `github.event.inputs.dry_run` gives it:
/// empty for any other event, otherwise `true` or `false`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DryRunInput {
    /// No input: the run was not dispatched.
    Absent,
    /// `dry_run` left set (its default).
    Set,
    /// `dry_run` cleared: the maintainer asks to publish.
    Cleared,
}

impl DryRunInput {
    /// Reads the input's text; anything but empty, `true` or `false` is
    /// refused rather than guessed.
    pub(crate) fn parse(text: &str) -> Result<Self, PublishError> {
        match text {
            "" => Ok(Self::Absent),
            "true" => Ok(Self::Set),
            "false" => Ok(Self::Cleared),
            other => Err(PublishError::DryRunInput(other.to_owned())),
        }
    }
}

/// What the workflow run is: its event, ref, repository, `dry_run` input and
/// commit, as GitHub reports them.
#[derive(Clone, Debug)]
pub(crate) struct RunContext {
    /// `github.event_name`.
    pub event: TriggerEvent,
    /// `github.ref`, such as `refs/tags/v0.1.0` or `refs/pull/7/merge`.
    pub git_ref: String,
    /// `github.repository`.
    pub repository: String,
    /// `github.event.inputs.dry_run`.
    pub dry_run: DryRunInput,
    /// `github.sha`, the commit the run built.
    pub commit: String,
}

/// Why a run does not publish.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DryRunReason {
    /// Pull requests and pushes never publish.
    NotDispatched(TriggerEvent),
    /// The dispatch left `dry_run` set.
    Requested,
}

/// Whether this run may publish.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PublishMode {
    /// Build, check and plan; publish nothing.
    DryRun(DryRunReason),
    /// Attest and publish after the `release` environment's approval.
    Publish,
}

impl PublishMode {
    /// The value of the plan job's `mode` output, which the privileged jobs'
    /// conditions compare with `publish`.
    pub(crate) const fn output(self) -> &'static str {
        match self {
            Self::DryRun(_) => "dry-run",
            Self::Publish => "publish",
        }
    }
}

/// A release version: `MAJOR.MINOR.PATCH` with an optional pre-release,
/// numbers without leading zeros, as semantic versioning and npm read it.
/// Build metadata (`+...`) is refused: npm drops it, so two versions that
/// differ only there would publish as one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReleaseVersion {
    text: String,
    major: u64,
    prerelease: bool,
}

impl ReleaseVersion {
    /// Parses a version, refusing anything semantic versioning would not
    /// accept or npm would rewrite.
    pub(crate) fn parse(text: &str) -> Result<Self, PublishError> {
        let refuse = || PublishError::Version(text.to_owned());
        let (core, prerelease) = match text.split_once('-') {
            Some((core, prerelease)) => (core, Some(prerelease)),
            None => (text, None),
        };
        let numbers: Vec<&str> = core.split('.').collect();
        let [major, minor, patch] = numbers.as_slice() else {
            return Err(refuse());
        };
        for number in [major, minor, patch] {
            if !is_numeric_identifier(number) {
                return Err(refuse());
            }
        }
        if let Some(prerelease) = prerelease {
            let valid = prerelease.split('.').all(|identifier| {
                !identifier.is_empty()
                    && identifier
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric() || character == '-')
                    && (!identifier
                        .chars()
                        .all(|character| character.is_ascii_digit())
                        || is_numeric_identifier(identifier))
            });
            if !valid {
                return Err(refuse());
            }
        }
        let major = major.parse::<u64>().map_err(|_| refuse())?;
        Ok(Self {
            text: text.to_owned(),
            major,
            prerelease: prerelease.is_some(),
        })
    }

    /// The dist-tag this version is published under: `next` for a 0.x
    /// version or a pre-release. A stable version is refused: its dist-tag
    /// (`latest`) and its release are P14's decision, and this workflow never
    /// moves `latest`.
    pub(crate) fn dist_tag(&self) -> Result<&'static str, PublishError> {
        if self.major == 0 || self.prerelease {
            Ok(DIST_TAG)
        } else {
            Err(PublishError::StableVersion(self.text.clone()))
        }
    }

    /// The Git tag a publication of this version must be dispatched on.
    pub(crate) fn git_tag(&self) -> String {
        format!("v{}", self.text)
    }

    /// The version's text.
    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }
}

/// A semantic-versioning numeric identifier: digits, and no leading zero
/// unless it is `0` itself.
fn is_numeric_identifier(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|character| character.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'))
}

/// Why a publish plan could not be made. Each refusal stops the run before
/// anything privileged starts.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum PublishError {
    /// The workspace version is not a version npm would publish unchanged.
    Version(String),
    /// A stable version: P14 decides its dist-tag and release.
    StableVersion(String),
    /// The `dry_run` input is neither empty, `true` nor `false`.
    DryRunInput(String),
    /// A publish was asked for on a ref that is not this version's tag.
    NotOnReleaseTag {
        /// The ref a publish must run on.
        expected: String,
        /// The ref the run is on.
        actual: String,
    },
    /// A publish was asked for in another repository (a fork).
    ForeignRepository(String),
    /// The commit is not a full lowercase SHA-1.
    Commit(String),
    /// The archives are not exactly one canonical archive per target.
    Archives(String),
    /// `SHA256SUMS` does not list exactly the archives.
    Checksums,
    /// The tarballs are not exactly one per package, named as `npm pack`
    /// names them.
    Tarballs(String),
    /// The plan could not be written as JSON.
    Json(String),
}

impl fmt::Display for PublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Version(version) => write!(
                formatter,
                "{version:?} is not a MAJOR.MINOR.PATCH[-PRERELEASE] version npm publishes as it is"
            ),
            Self::StableVersion(version) => write!(
                formatter,
                "{version} is a stable version; this workflow publishes only 0.x versions and \
                 pre-releases under `{DIST_TAG}`, and a stable release is P14's to plan"
            ),
            Self::DryRunInput(text) => write!(
                formatter,
                "the dry_run input {text:?} is not empty, `true` or `false`"
            ),
            Self::NotOnReleaseTag { expected, actual } => write!(
                formatter,
                "a publish runs only when the workflow is dispatched on {expected}; this run is \
                 on {actual}. Dispatch it on the tag, or leave dry_run set"
            ),
            Self::ForeignRepository(repository) => write!(
                formatter,
                "a publish runs only in {REPOSITORY}, not in {repository}"
            ),
            Self::Commit(commit) => {
                write!(formatter, "{commit:?} is not a full lowercase commit SHA")
            }
            Self::Archives(reason) => write!(formatter, "the release archives are wrong: {reason}"),
            Self::Checksums => write!(
                formatter,
                "SHA256SUMS does not list exactly the release archives with their digests"
            ),
            Self::Tarballs(reason) => write!(formatter, "the npm tarballs are wrong: {reason}"),
            Self::Json(reason) => write!(formatter, "the plan cannot be written: {reason}"),
        }
    }
}

impl std::error::Error for PublishError {}

/// Decides whether this run may publish `version`.
///
/// Only a manual dispatch with `dry_run` cleared publishes, and only on the
/// tag `v<version>` of this repository. A cleared `dry_run` anywhere else is
/// an error, never a silent dry run, so a maintainer who asked to publish
/// learns at once why nothing was.
pub(crate) fn decide_mode(
    context: &RunContext,
    version: &ReleaseVersion,
) -> Result<PublishMode, PublishError> {
    match (context.event, context.dry_run) {
        (TriggerEvent::WorkflowDispatch, DryRunInput::Set) => {
            Ok(PublishMode::DryRun(DryRunReason::Requested))
        }
        (TriggerEvent::WorkflowDispatch, DryRunInput::Cleared) => {
            if context.repository != REPOSITORY {
                return Err(PublishError::ForeignRepository(context.repository.clone()));
            }
            let expected = format!("refs/tags/{}", version.git_tag());
            if context.git_ref != expected {
                return Err(PublishError::NotOnReleaseTag {
                    expected,
                    actual: context.git_ref.clone(),
                });
            }
            Ok(PublishMode::Publish)
        }
        (TriggerEvent::WorkflowDispatch, DryRunInput::Absent) => {
            Err(PublishError::DryRunInput(String::new()))
        }
        (event, DryRunInput::Absent) => Ok(PublishMode::DryRun(DryRunReason::NotDispatched(event))),
        (_, DryRunInput::Set | DryRunInput::Cleared) => Err(PublishError::DryRunInput(
            String::from("an input on an event that is not a dispatch"),
        )),
    }
}

/// One checked release archive and the files the GitHub release also
/// carries separately.
pub(crate) struct ReleaseArchive {
    /// Its target.
    pub target: ReleaseTarget,
    /// Its file name.
    pub file_name: String,
    /// Its bytes.
    pub bytes: Vec<u8>,
    /// The SBOM inside it.
    pub sbom: Vec<u8>,
    /// The notices inside it.
    pub notices: Vec<u8>,
}

/// One tarball that `npm-verify` found to be the package it names.
pub(crate) struct PackedPackage {
    /// The package's npm name.
    pub package: &'static str,
    /// The tarball's file name.
    pub file_name: String,
    /// Its bytes.
    pub bytes: Vec<u8>,
}

/// A file by name and SHA-256.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Digested {
    /// The file name.
    pub name: String,
    /// Its lowercase hexadecimal SHA-256.
    pub sha256: String,
}

/// One `npm publish`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NpmPublication {
    /// The package it publishes.
    pub package: &'static str,
    /// The tarball, by name and digest.
    pub tarball: Digested,
}

impl NpmPublication {
    /// The arguments of `npm` for this publication, run from the job's
    /// working directory.
    pub(crate) fn arguments(&self) -> Vec<String> {
        let mut arguments = vec![
            String::from("publish"),
            format!("./{TARBALL_DIRECTORY}/{}", self.tarball.name),
        ];
        arguments.extend(NPM_PUBLISH_FLAGS.iter().map(|flag| (*flag).to_owned()));
        arguments
    }
}

/// The GitHub pre-release.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GithubRelease {
    /// The existing tag it is created on.
    pub tag: String,
    /// Its title.
    pub title: String,
    /// Its assets in name order.
    pub assets: Vec<Digested>,
}

impl GithubRelease {
    /// The arguments of `gh` that create the release as a draft with its
    /// assets; the workflow then publishes the draft.
    pub(crate) fn create_arguments(&self) -> Vec<String> {
        let mut arguments = vec![
            String::from("release"),
            String::from("create"),
            self.tag.clone(),
            String::from("--repo"),
            String::from(REPOSITORY),
        ];
        arguments.extend(GITHUB_RELEASE_FLAGS.iter().map(|flag| (*flag).to_owned()));
        arguments.extend([
            String::from("--title"),
            self.title.clone(),
            String::from("--notes-file"),
            format!("{PLAN_DIRECTORY}/{RELEASE_NOTES}"),
        ]);
        arguments.extend(
            self.assets
                .iter()
                .map(|asset| format!("{ASSET_DIRECTORY}/{}", asset.name)),
        );
        arguments
    }
}

/// Everything a run would publish, and whether it may.
#[derive(Clone, Debug)]
pub(crate) struct PublishPlan {
    /// The version.
    pub version: ReleaseVersion,
    /// The dist-tag, always [`DIST_TAG`].
    pub dist_tag: &'static str,
    /// Whether this run publishes.
    pub mode: PublishMode,
    /// The run it was made in.
    pub context: RunContext,
    /// The `npm publish` commands, platform packages first.
    pub npm: Vec<NpmPublication>,
    /// The GitHub pre-release.
    pub release: GithubRelease,
    /// Every file the `attest` job attests.
    pub subjects: Vec<Digested>,
    /// The SBOM and notices files the release carries beside the archives.
    pub extracted: Vec<(String, Vec<u8>)>,
}

/// The file `npm pack` writes for `package` at `version`: the name without
/// its `@`, `/` replaced by `-`, then the version.
pub(crate) fn npm_tarball_name(package: &str, version: &str) -> String {
    let stem = package.trim_start_matches('@').replace('/', "-");
    format!("{stem}-{version}.tgz")
}

/// The name the release gives a target's SBOM.
pub(crate) fn sbom_asset_name(version: &str, target: ReleaseTarget) -> String {
    format!("vsift-{version}-{}.cdx.json", target.triple())
}

/// The name the release gives a target's notices.
pub(crate) fn notices_asset_name(version: &str, target: ReleaseTarget) -> String {
    format!(
        "vsift-{version}-{}.THIRD-PARTY-NOTICES.txt",
        target.triple()
    )
}

/// The order packages are published in: every platform package first, so
/// the launcher never names a platform version that is not there yet.
pub(crate) fn publication_order() -> Vec<&'static str> {
    ReleaseTarget::ALL
        .into_iter()
        .map(ReleaseTarget::npm_package_name)
        .chain([LAUNCHER_PACKAGE])
        .collect()
}

/// Makes the plan from checked inputs: the workspace version, the run, one
/// canonical archive per target, the `SHA256SUMS` file as the `package` job
/// wrote it, and one verified tarball per package.
pub(crate) fn plan(
    version: &str,
    context: RunContext,
    archives: &[ReleaseArchive],
    checksums: &[u8],
    tarballs: &[PackedPackage],
) -> Result<PublishPlan, PublishError> {
    let version = ReleaseVersion::parse(version)?;
    let dist_tag = version.dist_tag()?;
    if !is_full_commit(&context.commit) {
        return Err(PublishError::Commit(context.commit.clone()));
    }
    let mode = decide_mode(&context, &version)?;

    for target in ReleaseTarget::ALL {
        let expected = archive_file_name(version.as_str(), target);
        let count = archives
            .iter()
            .filter(|archive| archive.target == target && archive.file_name == expected)
            .count();
        if count != 1 {
            return Err(PublishError::Archives(format!(
                "expected exactly one {expected}"
            )));
        }
    }
    if archives.len() != ReleaseTarget::ALL.len() {
        return Err(PublishError::Archives(String::from(
            "an archive is not a release target",
        )));
    }
    let listed = checksum_list(
        archives
            .iter()
            .map(|archive| (archive.file_name.as_str(), archive.bytes.as_slice())),
    )
    .map_err(|_| PublishError::Checksums)?;
    if listed.as_bytes() != checksums {
        return Err(PublishError::Checksums);
    }

    let mut npm = Vec::new();
    for package in publication_order() {
        let expected = npm_tarball_name(package, version.as_str());
        let mut matching = tarballs.iter().filter(|tarball| tarball.package == package);
        let (Some(tarball), None) = (matching.next(), matching.next()) else {
            return Err(PublishError::Tarballs(format!(
                "expected exactly one tarball of {package}"
            )));
        };
        if tarball.file_name != expected {
            return Err(PublishError::Tarballs(format!(
                "{package} is in {}, not {expected}",
                tarball.file_name
            )));
        }
        npm.push(NpmPublication {
            package,
            tarball: digested(&tarball.file_name, &tarball.bytes),
        });
    }
    if tarballs.len() != npm.len() {
        return Err(PublishError::Tarballs(String::from(
            "a tarball is not one of the packages",
        )));
    }

    let mut extracted = Vec::new();
    let mut assets = vec![digested("SHA256SUMS", checksums)];
    for archive in archives {
        assets.push(digested(&archive.file_name, &archive.bytes));
        let sbom = sbom_asset_name(version.as_str(), archive.target);
        let notices = notices_asset_name(version.as_str(), archive.target);
        assets.push(digested(&sbom, &archive.sbom));
        assets.push(digested(&notices, &archive.notices));
        extracted.push((sbom, archive.sbom.clone()));
        extracted.push((notices, archive.notices.clone()));
    }
    assets.sort_by(|left, right| left.name.cmp(&right.name));
    let mut subjects = assets.clone();
    subjects.extend(npm.iter().map(|publication| publication.tarball.clone()));
    subjects.sort_by(|left, right| left.name.cmp(&right.name));

    let release = GithubRelease {
        tag: version.git_tag(),
        title: format!("VSift {}", version.as_str()),
        assets,
    };
    Ok(PublishPlan {
        version,
        dist_tag,
        mode,
        context,
        npm,
        release,
        subjects,
        extracted,
    })
}

fn digested(name: &str, bytes: &[u8]) -> Digested {
    Digested {
        name: name.to_owned(),
        sha256: sha256_hex(bytes),
    }
}

fn is_full_commit(commit: &str) -> bool {
    commit.len() == 40
        && commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl PublishPlan {
    /// Every file of the plan by name: the JSON plan, the plan in words, the
    /// release notes, the two digest lists and the extracted SBOMs and
    /// notices. The workflow uploads them as the `publish-plan` artifact.
    pub(crate) fn files(&self) -> Result<Vec<(String, Vec<u8>)>, PublishError> {
        let json = serde_json::to_string_pretty(&self.json())
            .map_err(|error| PublishError::Json(error.to_string()))?;
        let mut files = vec![
            (String::from(PLAN_JSON), format!("{json}\n").into_bytes()),
            (String::from(PLAN_MARKDOWN), self.markdown().into_bytes()),
            (
                String::from(RELEASE_NOTES),
                self.release_notes().into_bytes(),
            ),
            (
                String::from(SUBJECTS_LIST),
                sha256sum_lines(&self.subjects).into_bytes(),
            ),
            (
                String::from(ASSETS_LIST),
                sha256sum_lines(&self.release.assets).into_bytes(),
            ),
        ];
        files.extend(self.extracted.iter().cloned());
        Ok(files)
    }

    /// The plan job's outputs, one `name=value` line each, for
    /// `$GITHUB_OUTPUT`.
    pub(crate) fn outputs(&self) -> String {
        format!(
            "mode={}\nversion={}\ntag={}",
            self.mode.output(),
            self.version.as_str(),
            self.version.git_tag()
        )
    }

    fn json(&self) -> Value {
        let digests = |files: &[Digested]| -> Vec<Value> {
            files
                .iter()
                .map(|file| json!({"name": file.name, "sha256": file.sha256}))
                .collect()
        };
        let npm: Vec<Value> = self
            .npm
            .iter()
            .map(|publication| {
                json!({
                    "package": publication.package,
                    "tarball": publication.tarball.name,
                    "sha256": publication.tarball.sha256,
                    "arguments": publication.arguments(),
                })
            })
            .collect();
        json!({
            "format": PLAN_FORMAT,
            "mode": self.mode.output(),
            "version": self.version.as_str(),
            "dist_tag": self.dist_tag,
            "git_tag": self.version.git_tag(),
            "commit": self.context.commit,
            "run": {
                "event": self.context.event.name(),
                "ref": self.context.git_ref,
                "repository": self.context.repository,
            },
            "attestation_subjects": digests(&self.subjects),
            "npm": npm,
            "github_release": {
                "tag": self.release.tag,
                "title": self.release.title,
                "prerelease": true,
                "latest": false,
                "arguments": self.release.create_arguments(),
                "assets": digests(&self.release.assets),
            },
        })
    }

    /// The plan in words, for the job summary: what would be attested,
    /// published and released, and whether this run does it.
    pub(crate) fn markdown(&self) -> String {
        let version = self.version.as_str();
        let mut text = String::new();
        let _ = match self.mode {
            PublishMode::DryRun(reason) => writeln!(
                text,
                "## Publish plan: dry run, nothing is published\n\n{}",
                match reason {
                    DryRunReason::NotDispatched(event) => format!(
                        "This run was started by `{}`, which never publishes.",
                        event.name()
                    ),
                    DryRunReason::Requested =>
                        String::from("This run was dispatched with `dry_run` set."),
                }
            ),
            PublishMode::Publish => writeln!(
                text,
                "## Publish plan: PUBLISH after the `release` environment's approval\n\n\
                 The `attest` job attests the files below, then the `publish` job waits for the \
                 maintainer's approval of the `release` environment before it publishes anything."
            ),
        };
        let _ = writeln!(
            text,
            "\n- Version `{version}`, npm dist-tag `{}` (`latest` is not touched), Git tag `{}`\n\
             - Commit `{}`, event `{}` on `{}` in `{}`",
            self.dist_tag,
            self.version.git_tag(),
            self.context.commit,
            self.context.event.name(),
            self.context.git_ref,
            self.context.repository,
        );
        let _ = writeln!(
            text,
            "\n### 1. Sigstore build provenance (job `attest`)\n\n| File | SHA-256 |\n| --- | --- |"
        );
        for subject in &self.subjects {
            let _ = writeln!(text, "| `{}` | `{}` |", subject.name, subject.sha256);
        }
        let _ = writeln!(
            text,
            "\n### 2. npm, in this order (job `publish`, environment `release`)\n\n\
             | # | Package | Tarball | SHA-256 | Command |\n| --- | --- | --- | --- | --- |"
        );
        for (index, publication) in self.npm.iter().enumerate() {
            let _ = writeln!(
                text,
                "| {} | `{}@{version}` | `{}` | `{}` | `npm {}` |",
                index + 1,
                publication.package,
                publication.tarball.name,
                publication.tarball.sha256,
                shell_words(&publication.arguments()),
            );
        }
        let _ = writeln!(
            text,
            "\n### 3. GitHub pre-release (job `publish`)\n\n`gh {}`, then the draft is published \
             as a pre-release that is not marked latest.\n\n| Asset | SHA-256 |\n| --- | --- |",
            shell_words(&self.release.create_arguments()),
        );
        for asset in &self.release.assets {
            let _ = writeln!(text, "| `{}` | `{}` |", asset.name, asset.sha256);
        }
        text
    }

    /// The GitHub release's notes. They name no person.
    pub(crate) fn release_notes(&self) -> String {
        let version = self.version.as_str();
        let tag = self.version.git_tag();
        format!(
            "VSift {version} is a pre-release. It is published to npm under the dist-tag \
             `{dist_tag}`; `latest` stays the `vsift-cli@0.0.0` placeholder until a stable \
             release. Built by the Release workflow from commit {commit} (tag `{tag}`).\n\
             \n\
             ## Install\n\
             \n\
             The npm package is `vsift-cli`; the command it installs is `vsift`. It needs \
             Node.js 22 or later, or Bun 1.2 or later.\n\
             \n\
             ```console\n\
             npm install --global vsift-cli@{dist_tag}\n\
             pnpm add --global vsift-cli@{dist_tag}\n\
             bun add --global vsift-cli@{dist_tag}\n\
             ```\n\
             \n\
             Yarn 4 holds back a version for a day after it is published \
             (`npmMinimalAgeGate`). Wait a day, or list `vsift-cli` and `@vsift/*` under \
             `npmPreapprovedPackages` in the project's `.yarnrc.yml`.\n\
             \n\
             Supported machines: Windows 11 x64, macOS 15 on Apple silicon, and Linux x64 with \
             glibc 2.35 or later and OpenSSL 3.\n\
             \n\
             ## Native archives\n\
             \n\
             Each `vsift-{version}-<target>.tar.gz` holds the `vsift` executable, the licences, \
             `THIRD-PARTY-NOTICES`, a CycloneDX SBOM and the agent skill; each target's SBOM and \
             notices are also attached on their own. Check a download against `SHA256SUMS` \
             (`sha256sum --check --ignore-missing SHA256SUMS`, or `shasum -a 256 --check \
             --ignore-missing SHA256SUMS` on macOS). The executables are not code-signed or \
             notarized, so Windows SmartScreen and macOS Gatekeeper may warn about one \
             downloaded directly; installing through npm avoids that.\n\
             \n\
             ## Verify the provenance\n\
             \n\
             Every archive, `SHA256SUMS`, SBOM, notices file and npm tarball has a Sigstore \
             build-provenance attestation from the Release workflow:\n\
             \n\
             ```console\n\
             gh attestation verify <file> --repo {REPOSITORY} \\\n  \
             --signer-workflow {REPOSITORY}/{WORKFLOW_PATH} \\\n  \
             --source-ref refs/tags/{tag} --deny-self-hosted-runners\n\
             ```\n\
             \n\
             The npm packages also carry npm provenance: `npm audit signatures` in a project \
             that installed `vsift-cli@{dist_tag}` checks their registry signatures and \
             provenance attestations.\n\
             \n\
             Installation guide: https://github.com/{REPOSITORY}/blob/{tag}/docs/operations/install.md\n",
            dist_tag = self.dist_tag,
            commit = self.context.commit,
        )
    }
}

/// `sha256sum` lines for `files`, in the order given.
fn sha256sum_lines(files: &[Digested]) -> String {
    files.iter().fold(String::new(), |mut text, file| {
        let _ = writeln!(text, "{}  {}", file.sha256, file.name);
        text
    })
}

/// Arguments as a reader would type them: plain words as they are, anything
/// else in single quotes. For display only; nothing runs this text.
fn shell_words(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| {
            let plain = !argument.is_empty()
                && argument.chars().all(|character| {
                    character.is_ascii_alphanumeric()
                        || matches!(character, '-' | '_' | '.' | '/' | '=' | '@' | ':')
                });
            if plain {
                argument.clone()
            } else {
                format!("'{}'", argument.replace('\'', r"'\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs, path::Path};

    use serde_json::Value;

    use super::{
        DIST_TAG, DryRunInput, DryRunReason, GITHUB_RELEASE_FLAGS, NPM_PUBLISH_FLAGS,
        PackedPackage, PublishError, PublishMode, ReleaseArchive, ReleaseVersion, RunContext,
        TriggerEvent, decide_mode, npm_tarball_name, plan, publication_order,
    };
    use crate::{
        archive::archive_file_name, checksums::checksum_list, npm::LAUNCHER_PACKAGE,
        target::ReleaseTarget,
    };

    const VERSION: &str = "0.1.0";
    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn context(event: TriggerEvent, git_ref: &str, dry_run: DryRunInput) -> RunContext {
        RunContext {
            event,
            git_ref: git_ref.to_owned(),
            repository: String::from("smormah/vsift"),
            dry_run,
            commit: String::from(COMMIT),
        }
    }

    fn release_dispatch(dry_run: DryRunInput) -> RunContext {
        context(TriggerEvent::WorkflowDispatch, "refs/tags/v0.1.0", dry_run)
    }

    fn archives() -> Vec<ReleaseArchive> {
        ReleaseTarget::ALL
            .into_iter()
            .map(|target| ReleaseArchive {
                target,
                file_name: archive_file_name(VERSION, target),
                bytes: format!("archive {target}").into_bytes(),
                sbom: format!("sbom {target}").into_bytes(),
                notices: format!("notices {target}").into_bytes(),
            })
            .collect()
    }

    fn checksums(archives: &[ReleaseArchive]) -> Result<Vec<u8>, Box<dyn Error>> {
        Ok(checksum_list(
            archives
                .iter()
                .map(|archive| (archive.file_name.as_str(), archive.bytes.as_slice())),
        )?
        .into_bytes())
    }

    fn tarballs() -> Vec<PackedPackage> {
        publication_order()
            .into_iter()
            .rev()
            .map(|package| PackedPackage {
                package,
                file_name: npm_tarball_name(package, VERSION),
                bytes: format!("tarball {package}").into_bytes(),
            })
            .collect()
    }

    #[test]
    fn versions_parse_as_semantic_versioning_without_build_metadata() {
        for good in [
            "0.1.0",
            "0.0.1",
            "1.2.3",
            "0.1.0-rc.1",
            "0.1.0-next.0",
            "10.20.30-x-y",
        ] {
            assert!(ReleaseVersion::parse(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "0.1",
            "0.1.0.0",
            "01.1.0",
            "0.01.0",
            "0.1.0-",
            "0.1.0-rc..1",
            "0.1.0-01",
            "0.1.0+build",
            "v0.1.0",
            "0.1.0-rc_1",
            "0.1.0 ",
            "a.b.c",
        ] {
            assert_eq!(
                ReleaseVersion::parse(bad),
                Err(PublishError::Version(bad.to_owned())),
                "{bad}"
            );
        }
    }

    #[test]
    fn only_zero_x_versions_and_pre_releases_are_published_and_always_under_next()
    -> Result<(), Box<dyn Error>> {
        for version in ["0.1.0", "0.9.9", "1.0.0-rc.1", "2.0.0-beta"] {
            assert_eq!(ReleaseVersion::parse(version)?.dist_tag()?, DIST_TAG);
        }
        assert_eq!(DIST_TAG, "next");
        for stable in ["1.0.0", "2.3.4"] {
            assert_eq!(
                ReleaseVersion::parse(stable)?.dist_tag(),
                Err(PublishError::StableVersion(stable.to_owned()))
            );
        }
        Ok(())
    }

    #[test]
    fn only_a_dispatch_on_the_release_tag_with_dry_run_cleared_publishes()
    -> Result<(), Box<dyn Error>> {
        let version = ReleaseVersion::parse(VERSION)?;
        assert_eq!(
            decide_mode(&release_dispatch(DryRunInput::Cleared), &version),
            Ok(PublishMode::Publish)
        );
        assert_eq!(
            decide_mode(&release_dispatch(DryRunInput::Set), &version),
            Ok(PublishMode::DryRun(DryRunReason::Requested))
        );
        for event in [TriggerEvent::PullRequest, TriggerEvent::Push] {
            for git_ref in ["refs/pull/7/merge", "refs/heads/main", "refs/tags/v0.1.0"] {
                assert_eq!(
                    decide_mode(&context(event, git_ref, DryRunInput::Absent), &version),
                    Ok(PublishMode::DryRun(DryRunReason::NotDispatched(event)))
                );
            }
            // A pull request or push never carries the input; one that
            // claims to is refused rather than read.
            assert!(matches!(
                decide_mode(
                    &context(event, "refs/tags/v0.1.0", DryRunInput::Cleared),
                    &version
                ),
                Err(PublishError::DryRunInput(_))
            ));
        }
        // Asking to publish anywhere but the tag of this version fails loudly.
        for git_ref in [
            "refs/heads/main",
            "refs/tags/v0.1.1",
            "refs/tags/0.1.0",
            "refs/tags/v0.1.0-rc.1",
            "refs/heads/v0.1.0",
        ] {
            let result = decide_mode(
                &context(
                    TriggerEvent::WorkflowDispatch,
                    git_ref,
                    DryRunInput::Cleared,
                ),
                &version,
            );
            assert_eq!(
                result,
                Err(PublishError::NotOnReleaseTag {
                    expected: String::from("refs/tags/v0.1.0"),
                    actual: git_ref.to_owned()
                }),
                "{git_ref}"
            );
        }
        let mut fork = release_dispatch(DryRunInput::Cleared);
        fork.repository = String::from("someone/vsift");
        assert_eq!(
            decide_mode(&fork, &version),
            Err(PublishError::ForeignRepository(String::from(
                "someone/vsift"
            )))
        );
        // A fork's dry run is still planned, so its pull requests are checked.
        fork.dry_run = DryRunInput::Set;
        assert_eq!(
            decide_mode(&fork, &version),
            Ok(PublishMode::DryRun(DryRunReason::Requested))
        );
        assert!(matches!(
            decide_mode(&release_dispatch(DryRunInput::Absent), &version),
            Err(PublishError::DryRunInput(_))
        ));
        Ok(())
    }

    #[test]
    fn the_dry_run_input_is_read_exactly() {
        assert_eq!(DryRunInput::parse(""), Ok(DryRunInput::Absent));
        assert_eq!(DryRunInput::parse("true"), Ok(DryRunInput::Set));
        assert_eq!(DryRunInput::parse("false"), Ok(DryRunInput::Cleared));
        for other in ["False", "0", "no", " false", "false\n"] {
            assert_eq!(
                DryRunInput::parse(other),
                Err(PublishError::DryRunInput(other.to_owned()))
            );
        }
    }

    #[test]
    fn tarball_names_are_the_ones_npm_pack_writes() {
        assert_eq!(
            npm_tarball_name("vsift-cli", "0.1.0"),
            "vsift-cli-0.1.0.tgz"
        );
        assert_eq!(
            npm_tarball_name("@vsift/win32-x64", "0.1.0-rc.1"),
            "vsift-win32-x64-0.1.0-rc.1.tgz"
        );
    }

    #[test]
    fn platform_packages_are_published_before_the_launcher() {
        assert_eq!(
            publication_order(),
            [
                "@vsift/darwin-arm64",
                "@vsift/win32-x64",
                "@vsift/linux-x64",
                LAUNCHER_PACKAGE
            ]
        );
    }

    #[test]
    fn the_plan_lists_every_command_and_file_exactly() -> Result<(), Box<dyn Error>> {
        let archives = archives();
        let plan = plan(
            VERSION,
            release_dispatch(DryRunInput::Cleared),
            &archives,
            &checksums(&archives)?,
            &tarballs(),
        )?;
        assert_eq!(plan.mode, PublishMode::Publish);
        assert_eq!(plan.outputs(), "mode=publish\nversion=0.1.0\ntag=v0.1.0");
        let commands: Vec<Vec<String>> = plan
            .npm
            .iter()
            .map(super::NpmPublication::arguments)
            .collect();
        assert_eq!(
            commands.first().map(|arguments| arguments.join(" ")),
            Some(String::from(
                "publish ./npm-packages/vsift-darwin-arm64-0.1.0.tgz --tag next --access public \
                 --provenance --ignore-scripts"
            ))
        );
        assert_eq!(
            commands.last().map(|arguments| arguments.join(" ")),
            Some(String::from(
                "publish ./npm-packages/vsift-cli-0.1.0.tgz --tag next --access public \
                 --provenance --ignore-scripts"
            ))
        );
        for arguments in &commands {
            assert!(!arguments.iter().any(|argument| argument.contains("latest")));
        }
        let release = plan.release.create_arguments();
        assert_eq!(
            release.get(..13).map(|words| words.join(" ")),
            Some(String::from(
                "release create v0.1.0 --repo smormah/vsift --verify-tag --draft --prerelease \
                 --latest=false --title VSift 0.1.0 --notes-file publish-plan/release-notes.md"
            ))
        );
        let assets: Vec<&str> = release
            .get(13..)
            .unwrap_or_default()
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(
            assets,
            [
                "release-assets/SHA256SUMS",
                "release-assets/vsift-0.1.0-aarch64-apple-darwin.THIRD-PARTY-NOTICES.txt",
                "release-assets/vsift-0.1.0-aarch64-apple-darwin.cdx.json",
                "release-assets/vsift-0.1.0-aarch64-apple-darwin.tar.gz",
                "release-assets/vsift-0.1.0-x86_64-pc-windows-msvc.THIRD-PARTY-NOTICES.txt",
                "release-assets/vsift-0.1.0-x86_64-pc-windows-msvc.cdx.json",
                "release-assets/vsift-0.1.0-x86_64-pc-windows-msvc.tar.gz",
                "release-assets/vsift-0.1.0-x86_64-unknown-linux-gnu.THIRD-PARTY-NOTICES.txt",
                "release-assets/vsift-0.1.0-x86_64-unknown-linux-gnu.cdx.json",
                "release-assets/vsift-0.1.0-x86_64-unknown-linux-gnu.tar.gz",
            ]
        );
        // Every release asset and every tarball is attested.
        assert_eq!(plan.subjects.len(), 10 + 4);

        let files = plan.files()?;
        let file = |name: &str| {
            files
                .iter()
                .find(|(candidate, _)| candidate == name)
                .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
                .unwrap_or_default()
        };
        let json: Value = serde_json::from_str(&file("publish-plan.json"))?;
        assert_eq!(json["format"], "vsift-publish-plan/1");
        assert_eq!(json["mode"], "publish");
        assert_eq!(json["dist_tag"], "next");
        assert_eq!(json["github_release"]["latest"], false);
        assert_eq!(json["npm"].as_array().map(Vec::len), Some(4));
        let subjects = file("attestation-subjects.sha256");
        assert_eq!(subjects.lines().count(), 14);
        assert!(subjects.contains("  vsift-cli-0.1.0.tgz\n"));
        let assets_list = file("release-assets.sha256");
        assert_eq!(assets_list.lines().count(), 10);
        assert!(!assets_list.contains(".tgz"));
        assert_eq!(
            file("vsift-0.1.0-x86_64-unknown-linux-gnu.cdx.json"),
            "sbom x86_64-unknown-linux-gnu"
        );
        let summary = file("publish-plan.md");
        assert!(summary.contains("PUBLISH after the `release` environment's approval"));
        assert!(summary.contains("`latest` is not touched"));
        let notes = file("release-notes.md");
        assert!(notes.contains("npm install --global vsift-cli@next"));
        assert!(notes.contains("--source-ref refs/tags/v0.1.0"));
        assert!(notes.contains(COMMIT));
        Ok(())
    }

    #[test]
    fn a_dry_run_plans_the_same_commands_and_says_it_publishes_nothing()
    -> Result<(), Box<dyn Error>> {
        let archives = archives();
        let sums = checksums(&archives)?;
        let publish = plan(
            VERSION,
            release_dispatch(DryRunInput::Cleared),
            &archives,
            &sums,
            &tarballs(),
        )?;
        let pull_request = plan(
            VERSION,
            context(
                TriggerEvent::PullRequest,
                "refs/pull/9/merge",
                DryRunInput::Absent,
            ),
            &archives,
            &sums,
            &tarballs(),
        )?;
        assert_eq!(
            pull_request.mode,
            PublishMode::DryRun(DryRunReason::NotDispatched(TriggerEvent::PullRequest))
        );
        assert_eq!(pull_request.npm, publish.npm);
        assert_eq!(pull_request.release, publish.release);
        assert_eq!(pull_request.subjects, publish.subjects);
        assert!(pull_request.outputs().starts_with("mode=dry-run\n"));
        assert!(
            pull_request
                .markdown()
                .starts_with("## Publish plan: dry run, nothing is published")
        );
        Ok(())
    }

    #[test]
    fn inputs_that_are_not_exactly_the_release_are_refused() -> Result<(), Box<dyn Error>> {
        let good = archives();
        let sums = checksums(&good)?;
        let run = || release_dispatch(DryRunInput::Set);

        let mut missing = archives();
        missing.pop();
        assert!(matches!(
            plan(VERSION, run(), &missing, &sums, &tarballs()),
            Err(PublishError::Archives(_))
        ));
        let mut renamed = archives();
        if let Some(archive) = renamed.first_mut() {
            archive.file_name = String::from("vsift.tar.gz");
        }
        assert!(matches!(
            plan(VERSION, run(), &renamed, &sums, &tarballs()),
            Err(PublishError::Archives(_))
        ));
        let mut changed = archives();
        if let Some(archive) = changed.first_mut() {
            archive.bytes.push(0);
        }
        assert_eq!(
            plan(VERSION, run(), &changed, &sums, &tarballs()).err(),
            Some(PublishError::Checksums)
        );

        let mut short = tarballs();
        short.pop();
        assert!(matches!(
            plan(VERSION, run(), &good, &sums, &short),
            Err(PublishError::Tarballs(_))
        ));
        let mut misnamed = tarballs();
        if let Some(tarball) = misnamed.first_mut() {
            tarball.file_name = String::from("vsift-cli-0.2.0.tgz");
        }
        assert!(matches!(
            plan(VERSION, run(), &good, &sums, &misnamed),
            Err(PublishError::Tarballs(_))
        ));
        let mut doubled = tarballs();
        doubled.push(PackedPackage {
            package: LAUNCHER_PACKAGE,
            file_name: npm_tarball_name(LAUNCHER_PACKAGE, VERSION),
            bytes: Vec::new(),
        });
        assert!(matches!(
            plan(VERSION, run(), &good, &sums, &doubled),
            Err(PublishError::Tarballs(_))
        ));

        let mut short_commit = run();
        short_commit.commit = String::from("0123456");
        assert!(matches!(
            plan(VERSION, short_commit, &good, &sums, &tarballs()),
            Err(PublishError::Commit(_))
        ));
        assert!(matches!(
            plan("1.0.0", run(), &good, &sums, &tarballs()),
            Err(PublishError::StableVersion(_))
        ));
        Ok(())
    }

    /// The privileged jobs run the plan's commands in shell, so that no Rust
    /// is compiled where an OIDC token is available. This holds the workflow's
    /// commands to the ones this module builds, word for word.
    #[test]
    fn the_release_workflow_runs_the_planned_commands() -> Result<(), Box<dyn Error>> {
        let workflow = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows/release.yml"),
        )?;
        let lines: Vec<&str> = workflow.lines().map(str::trim).collect();
        let order = format!("for package in {}; do", publication_order().join(" "));
        assert!(lines.contains(&order.as_str()), "missing: {order}");
        let publish = format!(
            "npm publish \"./npm-packages/${{file}}\" {}",
            NPM_PUBLISH_FLAGS.join(" ")
        );
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains("npm publish"))
                .count(),
            1,
            "exactly one npm publish command"
        );
        assert!(lines.contains(&publish.as_str()), "missing: {publish}");
        let release = format!(
            "gh release create \"${{TAG}}\" --repo smormah/vsift {} \\",
            GITHUB_RELEASE_FLAGS.join(" ")
        );
        assert!(lines.contains(&release.as_str()), "missing: {release}");
        assert!(lines.contains(
            &"--title \"VSift ${VERSION}\" --notes-file publish-plan/release-notes.md release-assets/*"
        ));
        Ok(())
    }
}
