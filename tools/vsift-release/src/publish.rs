//! The publish plan (P13 PR 10, ADR 0023 section 1 and decisions B and C;
//! P14 PR 8, ADR 0024 decisions A and B).
//!
//! The Release workflow's unprivileged `plan` job runs `vsift-release
//! publish-plan` on every run. It checks the release archives, their
//! `SHA256SUMS` and the npm tarballs the `npm-qualify` jobs installed, decides
//! whether this run may publish at all, and writes down exactly what the
//! privileged `attest` and `publish` jobs would do: the files to attest, the
//! `npm publish` commands in order, and the GitHub release with its assets. A
//! dry run (every run that is not the maintainer's dispatch of the release tag
//! with `dry_run` cleared) stops there, so the plan is what a dry run shows;
//! the privileged jobs only execute it after the `release` environment's
//! approval.
//!
//! There are two channels, and the version alone decides which:
//!
//! - a version **with a pre-release suffix** (`0.2.0-rc.1`, `0.3.0-beta.2`) is
//!   published under the dist-tag `next`, as a GitHub pre-release that is not
//!   marked latest, and never touches `latest`;
//! - a version **without one** (`0.2.0`, and by the same rule `1.0.0`) is
//!   *stable*: it is published under `latest`, moving it on all four
//!   packages, as the GitHub release marked latest. Every stable plan carries
//!   the [guards](crate::guards) that stand between a bad run and `latest`.
//!
//! The version `0.0.0` is the placeholder every package already holds and is
//! refused in every mode. The workflow still never runs `npm dist-tag`: a
//! stable version reaches `latest` only by being published with `--tag
//! latest`.
//!
//! Only this module decides the mode, the dist-tag and the arguments. The
//! workflow's two privileged jobs repeat the same commands in a few lines of
//! shell, so that no Rust is compiled where an OIDC token is available; a test
//! here holds the workflow's commands to the ones this module builds.

use std::fmt::{self, Write};

use clap::ValueEnum;
use serde_json::{Value, json};

use crate::{
    archive::archive_file_name,
    candidate::{CandidateObservation, CandidateReport, release_delta},
    checksums::checksum_list,
    guards::{DistTagMove, Evaluation, Guard, GuardOutcome, Observations, evaluate},
    notes::release_notes,
    npm::{LAUNCHER_PACKAGE, sha256_hex},
    registry::npm_integrity,
    target::ReleaseTarget,
};

/// The only repository a release may be published from.
pub(crate) const REPOSITORY: &str = "smormah/vsift";

/// The workflow npm's trusted publishers and the attestations name.
pub(crate) const WORKFLOW_PATH: &str = ".github/workflows/release.yml";

/// The dist-tag of a pre-release (ADR 0023 decision B).
pub(crate) const PRERELEASE_DIST_TAG: &str = "next";

/// The dist-tag of a stable version (ADR 0024 decision A): the only tag whose
/// move the workflow cannot take back, so only a stable version reaches it.
pub(crate) const STABLE_DIST_TAG: &str = "latest";

/// The plan's format identifier. Version 2 (P14 PR 8) adds the channel, the
/// dist-tag moves, the guards and the candidate.
const PLAN_FORMAT: &str = "vsift-publish-plan/2";

/// Where the privileged jobs download the tarballs, the plan and the release
/// assets (the artifact download paths in `release.yml`).
pub(crate) const TARBALL_DIRECTORY: &str = "npm-packages";
/// The directory the plan's files are downloaded to.
pub(crate) const PLAN_DIRECTORY: &str = "publish-plan";
/// The directory the GitHub release's assets are gathered in.
pub(crate) const ASSET_DIRECTORY: &str = "release-assets";

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
/// The record of the candidate comparison in the evidence ledger's `release_delta`
/// shape, written for an enforced stable plan that passed every guard.
pub(crate) const RELEASE_DELTA: &str = "release-delta.json";

/// Which of the two publications a version gets. The pairing of a channel
/// with its dist-tag and its GitHub flags is made here and nowhere else; the
/// workflow's two publishing paths are each held to one channel's commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Channel {
    /// A version with a pre-release suffix: `next`, a GitHub pre-release.
    PreRelease,
    /// A version without one: `latest`, the GitHub release marked latest.
    Stable,
}

impl Channel {
    /// The value of the plan job's `channel` output, which the privileged
    /// jobs' steps compare.
    pub(crate) const fn output(self) -> &'static str {
        match self {
            Self::PreRelease => "prerelease",
            Self::Stable => "stable",
        }
    }

    /// The dist-tag every `npm publish` of this channel sets.
    pub(crate) const fn dist_tag(self) -> &'static str {
        match self {
            Self::PreRelease => PRERELEASE_DIST_TAG,
            Self::Stable => STABLE_DIST_TAG,
        }
    }

    /// The dist-tag this channel leaves alone.
    pub(crate) const fn untouched_dist_tag(self) -> &'static str {
        match self {
            Self::PreRelease => STABLE_DIST_TAG,
            Self::Stable => PRERELEASE_DIST_TAG,
        }
    }

    /// The flags of every `npm publish`: the channel's dist-tag, public
    /// access (a scope's first publish is otherwise restricted), npm
    /// provenance (automatic with trusted publishing, and required with the
    /// bootstrap token), and no lifecycle scripts (a tarball has none; this
    /// makes sure none runs). The dist-tag is always explicit, never npm's
    /// default.
    pub(crate) const fn npm_publish_flags(self) -> [&'static str; 6] {
        [
            "--tag",
            self.dist_tag(),
            "--access",
            "public",
            "--provenance",
            "--ignore-scripts",
        ]
    }

    /// The flags of `gh release create`: the tag must already exist (the
    /// maintainer creates it under the tag ruleset; the workflow never
    /// creates a tag), and the release starts as a draft so a half-uploaded
    /// release is never public. A pre-release is marked as one and is never
    /// latest; a stable release is not marked latest until it is published
    /// (see [`Self::github_publish_flags`]).
    pub(crate) const fn github_create_flags(self) -> &'static [&'static str] {
        match self {
            Self::PreRelease => &["--verify-tag", "--draft", "--prerelease", "--latest=false"],
            Self::Stable => &["--verify-tag", "--draft"],
        }
    }

    /// The flags of the `gh release edit` that publishes the draft: a stable
    /// release is marked latest by this edit, once, explicitly.
    pub(crate) const fn github_publish_flags(self) -> &'static [&'static str] {
        match self {
            Self::PreRelease => &["--draft=false"],
            Self::Stable => &["--draft=false", "--latest"],
        }
    }
}

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

/// Whether a failed guard stops the run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Enforcement {
    /// A publish, or a dispatch on the version's own tag (the rehearsal of
    /// one, even with `dry_run` set): a failed guard refuses the plan, so the
    /// dry run on the tag fails whenever the real run would.
    Enforced,
    /// Any other run (a pull request, a push, a dispatch elsewhere): the
    /// findings are shown and the run carries on, so a working tree whose
    /// version has no candidate yet does not fail every pull request.
    ReportOnly,
}

/// The kinds of version, which differ in their channel and in the wording of
/// their release notes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReleaseKind {
    /// `X.Y.Z-rc.N` with a positive `N`: a release candidate, under `next`.
    ReleaseCandidate,
    /// Any other pre-release suffix: under `next`.
    PreRelease,
    /// No pre-release suffix, a 0.x version included: under `latest`.
    Stable,
}

impl ReleaseKind {
    /// The channel this kind is published in.
    pub(crate) const fn channel(self) -> Channel {
        match self {
            Self::ReleaseCandidate | Self::PreRelease => Channel::PreRelease,
            Self::Stable => Channel::Stable,
        }
    }

    const fn key(self) -> &'static str {
        match self {
            Self::ReleaseCandidate => "release_candidate",
            Self::PreRelease => "pre_release",
            Self::Stable => "stable",
        }
    }

    const fn words(self) -> &'static str {
        match self {
            Self::ReleaseCandidate => "release candidate",
            Self::PreRelease => "pre-release",
            Self::Stable => "stable",
        }
    }
}

/// A release version: `MAJOR.MINOR.PATCH` with an optional pre-release,
/// numbers without leading zeros, as semantic versioning and npm read it.
/// Build metadata (`+...`) is refused: npm drops it, so two versions that
/// differ only there would publish as one.
///
/// Whether a version is stable depends on its suffix alone: `0.2.0` is
/// stable exactly as `1.0.0` is. (`0.1.0` is stable by this rule and was
/// published before it existed, as a pre-release under `next`; a plan for it
/// is refused by the registry guards, since npm holds it under another tag.)
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReleaseVersion {
    text: String,
    core: (u64, u64, u64),
    prerelease: Option<String>,
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
        let number = |text: &str| text.parse::<u64>().map_err(|_| refuse());
        Ok(Self {
            text: text.to_owned(),
            core: (number(major)?, number(minor)?, number(patch)?),
            prerelease: prerelease.map(str::to_owned),
        })
    }

    /// The kind of version: a release candidate, another pre-release, or
    /// stable.
    pub(crate) fn kind(&self) -> ReleaseKind {
        match self.prerelease.as_deref() {
            None => ReleaseKind::Stable,
            Some(suffix) if is_candidate_suffix(suffix) => ReleaseKind::ReleaseCandidate,
            Some(_) => ReleaseKind::PreRelease,
        }
    }

    /// Whether this is the `0.0.0` that every package already holds as its
    /// placeholder: never a release.
    pub(crate) fn is_placeholder(&self) -> bool {
        self.core == (0, 0, 0) && self.prerelease.is_none()
    }

    /// Whether this version's `MAJOR.MINOR.PATCH` is below `other`'s. Only
    /// the numbers are compared, which is exact for stable versions.
    pub(crate) fn precedes(&self, other: &Self) -> bool {
        self.core < other.core
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

/// `rc.N` with a positive `N`: the suffix of a release candidate.
fn is_candidate_suffix(suffix: &str) -> bool {
    suffix
        .strip_prefix("rc.")
        .is_some_and(|number| is_numeric_identifier(number) && number != "0")
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
    /// The version is `0.0.0`, the placeholder every package already holds.
    PlaceholderVersion,
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
    /// The date given for the delta record is not YYYY-MM-DD.
    Date(String),
    /// An enforced plan failed at least one guard.
    Refused(Vec<String>),
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
            Self::PlaceholderVersion => write!(
                formatter,
                "0.0.0 is the placeholder every package already holds; it is never a release"
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
            Self::Date(date) => write!(formatter, "{date:?} is not a date written YYYY-MM-DD"),
            Self::Refused(reasons) => write!(
                formatter,
                "the plan is refused, nothing may be published: {}",
                reasons.join("; ")
            ),
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

/// Whether a failed guard stops this run: always when it publishes, and for
/// a dispatch on the version's own tag even with `dry_run` set.
pub(crate) fn enforcement(
    context: &RunContext,
    version: &ReleaseVersion,
    mode: PublishMode,
) -> Enforcement {
    let on_release_tag = context.event == TriggerEvent::WorkflowDispatch
        && context.git_ref == format!("refs/tags/{}", version.git_tag());
    if mode == PublishMode::Publish || on_release_tag {
        Enforcement::Enforced
    } else {
        Enforcement::ReportOnly
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
    /// The `dist.integrity` npm will record for the tarball, which the
    /// registry guards compare with what npm already holds.
    pub integrity: String,
}

impl NpmPublication {
    /// The arguments of `npm` for this publication in `channel`, run from
    /// the job's working directory.
    pub(crate) fn arguments(&self, channel: Channel) -> Vec<String> {
        let mut arguments = vec![
            String::from("publish"),
            format!("./{TARBALL_DIRECTORY}/{}", self.tarball.name),
        ];
        arguments.extend(
            channel
                .npm_publish_flags()
                .iter()
                .map(|flag| (*flag).to_owned()),
        );
        arguments
    }
}

/// The GitHub release.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GithubRelease {
    /// The channel, which decides its flags.
    pub channel: Channel,
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
        arguments.extend(
            self.channel
                .github_create_flags()
                .iter()
                .map(|flag| (*flag).to_owned()),
        );
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

    /// The arguments of `gh` that publish the draft.
    pub(crate) fn publish_arguments(&self) -> Vec<String> {
        let mut arguments = vec![
            String::from("release"),
            String::from("edit"),
            self.tag.clone(),
            String::from("--repo"),
            String::from(REPOSITORY),
        ];
        arguments.extend(
            self.channel
                .github_publish_flags()
                .iter()
                .map(|flag| (*flag).to_owned()),
        );
        arguments
    }
}

/// Everything a run would publish, and whether it may.
#[derive(Clone, Debug)]
pub(crate) struct PublishPlan {
    /// The version.
    pub version: ReleaseVersion,
    /// Whether this run publishes.
    pub mode: PublishMode,
    /// Whether a failed guard stops it.
    pub enforcement: Enforcement,
    /// The run it was made in.
    pub context: RunContext,
    /// The `npm publish` commands, platform packages first.
    pub npm: Vec<NpmPublication>,
    /// The GitHub release.
    pub release: GithubRelease,
    /// Every file the `attest` job attests.
    pub subjects: Vec<Digested>,
    /// The SBOM and notices files the release carries beside the archives.
    pub extracted: Vec<(String, Vec<u8>)>,
    /// What each guard found.
    pub guards: Vec<Guard>,
    /// What the publication does to each package's dist-tags.
    pub moves: Vec<DistTagMove>,
    /// The comparison with the accepted candidate, for a stable version.
    pub candidate: Option<Box<CandidateReport>>,
    /// The evidence ledger's `release_delta` record, for an enforced stable plan
    /// whose guards all passed and that was given the run and date to name.
    pub release_delta: Option<serde_json::Value>,
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

/// Requires exactly one archive per target, named for the version, and the
/// `SHA256SUMS` file to be the one those archives produce.
fn check_archives(
    version: &ReleaseVersion,
    archives: &[ReleaseArchive],
    checksums: &[u8],
) -> Result<(), PublishError> {
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
    Ok(())
}

/// One publication per package, in publication order, from exactly one
/// correctly named tarball each.
fn npm_publications(
    version: &ReleaseVersion,
    tarballs: &[PackedPackage],
) -> Result<Vec<NpmPublication>, PublishError> {
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
            integrity: npm_integrity(&tarball.bytes),
        });
    }
    if tarballs.len() != npm.len() {
        return Err(PublishError::Tarballs(String::from(
            "a tarball is not one of the packages",
        )));
    }
    Ok(npm)
}

/// Makes the plan from checked inputs: the version, the run, one canonical
/// archive per target, the `SHA256SUMS` file as the `package` job wrote it,
/// one verified tarball per package, and what the plan job observed outside
/// them (the registry and the accepted candidate).
pub(crate) fn plan(
    version: &ReleaseVersion,
    context: RunContext,
    archives: &[ReleaseArchive],
    checksums: &[u8],
    tarballs: &[PackedPackage],
    observations: &Observations,
) -> Result<PublishPlan, PublishError> {
    if version.is_placeholder() {
        return Err(PublishError::PlaceholderVersion);
    }
    if !is_full_commit(&context.commit) {
        return Err(PublishError::Commit(context.commit.clone()));
    }
    let mode = decide_mode(&context, version)?;
    let enforcement = enforcement(&context, version, mode);
    let channel = version.kind().channel();

    check_archives(version, archives, checksums)?;
    let npm = npm_publications(version, tarballs)?;

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

    let Evaluation { guards, moves } = evaluate(version, &npm, observations);
    let candidate = match &observations.candidate {
        CandidateObservation::Checked(report) if channel == Channel::Stable => Some(report.clone()),
        _ => None,
    };
    let release_delta = match (&candidate, &observations.check) {
        (Some(report), Some(check))
            if enforcement == Enforcement::Enforced
                && guards
                    .iter()
                    .all(|guard| guard.outcome != GuardOutcome::Failed) =>
        {
            Some(release_delta(report, version.as_str(), check))
        }
        _ => None,
    };
    let release = GithubRelease {
        channel,
        tag: version.git_tag(),
        title: format!("VSift {}", version.as_str()),
        assets,
    };
    Ok(PublishPlan {
        version: version.clone(),
        mode,
        enforcement,
        context,
        npm,
        release,
        subjects,
        extracted,
        guards,
        moves,
        candidate,
        release_delta,
    })
}

fn digested(name: &str, bytes: &[u8]) -> Digested {
    Digested {
        name: name.to_owned(),
        sha256: sha256_hex(bytes),
    }
}

/// Whether `commit` is a full lowercase SHA-1.
pub(crate) fn is_full_commit(commit: &str) -> bool {
    commit.len() == 40
        && commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A table cell: no pipe and no line break, which would end it.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\n', '\r'], " ")
}

impl PublishPlan {
    /// The channel of this plan's version.
    pub(crate) fn channel(&self) -> Channel {
        self.version.kind().channel()
    }

    /// The failed guards of an enforced plan, as the reasons it is refused;
    /// `None` when the plan stands. A report-only plan is never refused.
    pub(crate) fn refusal(&self) -> Option<Vec<String>> {
        if self.enforcement != Enforcement::Enforced {
            return None;
        }
        let reasons = self.failed_guards();
        if reasons.is_empty() {
            None
        } else {
            Some(reasons)
        }
    }

    fn failed_guards(&self) -> Vec<String> {
        self.guards
            .iter()
            .filter(|guard| guard.outcome == GuardOutcome::Failed)
            .map(|guard| format!("{}: {}", guard.name, guard.detail))
            .collect()
    }

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
        if let Some(delta) = &self.release_delta {
            let text = serde_json::to_string_pretty(delta)
                .map_err(|error| PublishError::Json(error.to_string()))?;
            files.push((
                String::from(RELEASE_DELTA),
                format!("{text}\n").into_bytes(),
            ));
        }
        files.extend(self.extracted.iter().cloned());
        Ok(files)
    }

    /// The plan job's outputs, one `name=value` line each, for
    /// `$GITHUB_OUTPUT`.
    pub(crate) fn outputs(&self) -> String {
        format!(
            "mode={}\nversion={}\ntag={}\nchannel={}",
            self.mode.output(),
            self.version.as_str(),
            self.version.git_tag(),
            self.channel().output()
        )
    }

    fn json(&self) -> Value {
        let digests = |files: &[Digested]| -> Vec<Value> {
            files
                .iter()
                .map(|file| json!({"name": file.name, "sha256": file.sha256}))
                .collect()
        };
        let channel = self.channel();
        let npm: Vec<Value> = self
            .npm
            .iter()
            .map(|publication| {
                json!({
                    "package": publication.package,
                    "tarball": publication.tarball.name,
                    "sha256": publication.tarball.sha256,
                    "integrity": publication.integrity,
                    "arguments": publication.arguments(channel),
                })
            })
            .collect();
        let dist_tags: Vec<Value> = self
            .moves
            .iter()
            .map(|moved| {
                json!({
                    "package": moved.package,
                    "tag": moved.tag,
                    "from": moved.from,
                    "to": moved.to,
                    "untouched_tag": moved.untouched_tag,
                    "untouched": moved.untouched,
                })
            })
            .collect();
        let guards: Vec<Value> = self
            .guards
            .iter()
            .map(|guard| {
                json!({
                    "name": guard.name,
                    "outcome": guard.outcome.key(),
                    "detail": guard.detail,
                })
            })
            .collect();
        let candidate = self.candidate.as_ref().map(|report| {
            json!({
                "tag": report.candidate_tag,
                "version": report.candidate_version,
                "commit": report.candidate_commit,
                "changed_paths": report.changes.iter().map(|change| change.path.as_str()).collect::<Vec<_>>(),
            })
        });
        json!({
            "format": PLAN_FORMAT,
            "mode": self.mode.output(),
            "enforced": self.enforcement == Enforcement::Enforced,
            "version": self.version.as_str(),
            "kind": self.version.kind().key(),
            "channel": channel.output(),
            "dist_tag": channel.dist_tag(),
            "moves_latest": channel == Channel::Stable,
            "git_tag": self.version.git_tag(),
            "commit": self.context.commit,
            "run": {
                "event": self.context.event.name(),
                "ref": self.context.git_ref,
                "repository": self.context.repository,
            },
            "dist_tags": dist_tags,
            "guards": guards,
            "candidate": candidate,
            "release_delta": self.release_delta,
            "attestation_subjects": digests(&self.subjects),
            "npm": npm,
            "github_release": {
                "tag": self.release.tag,
                "title": self.release.title,
                "prerelease": channel == Channel::PreRelease,
                "latest": channel == Channel::Stable,
                "arguments": self.release.create_arguments(),
                "publish_arguments": self.release.publish_arguments(),
                "assets": digests(&self.release.assets),
            },
        })
    }

    /// The first lines of the plan: what it is, and above all whether it
    /// moves `latest`.
    fn banner(&self) -> String {
        let version = self.version.as_str();
        let kind = self.version.kind();
        let stable = kind == ReleaseKind::Stable;
        let refused = self.refusal().is_some();
        let mut text = String::new();
        let _ = match (self.mode, refused) {
            (_, true) => writeln!(
                text,
                "## Publish plan: REFUSED, nothing is published\n\n\
                 This run asked to publish or rehearse `{version}` on its tag, and a guard failed."
            ),
            (PublishMode::DryRun(reason), false) => writeln!(
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
            (PublishMode::Publish, false) if stable => writeln!(
                text,
                "## Publish plan: PUBLISH a STABLE release after the `release` environment's \
                 approval\n\n\
                 The `attest` job attests the files below, then the `publish` job waits for the \
                 maintainer's approval of the `release` environment before it publishes anything."
            ),
            (PublishMode::Publish, false) => writeln!(
                text,
                "## Publish plan: PUBLISH a {} after the `release` environment's approval\n\n\
                 The `attest` job attests the files below, then the `publish` job waits for the \
                 maintainer's approval of the `release` environment before it publishes anything.",
                kind.words()
            ),
        };
        if stable {
            let _ = match self.mode {
                PublishMode::Publish => writeln!(
                    text,
                    "\n> **This publication moves npm's `latest` dist-tag on all four packages.** \
                     Afterwards `npm install vsift-cli` installs `{version}`. A published version \
                     cannot be unpublished: a bad one is deprecated and replaced by a new version \
                     (release.md section 6.5)."
                ),
                PublishMode::DryRun(_) => writeln!(
                    text,
                    "\n> **This version is stable: a real publication of this plan would move \
                     npm's `latest` dist-tag on all four packages.**"
                ),
            };
        } else {
            let _ = writeln!(
                text,
                "\n> **`latest` is not touched.** This {} is published under `{}` only.",
                kind.words(),
                self.channel().dist_tag()
            );
        }
        let failed = self.failed_guards();
        if !failed.is_empty() {
            let _ = writeln!(
                text,
                "\n**{}**\n",
                if refused {
                    "Refused because:"
                } else {
                    "A real publication of this plan would be refused, because:"
                }
            );
            for reason in failed {
                let _ = writeln!(text, "- {reason}");
            }
        }
        text
    }

    /// The plan in words, for the job summary: what would be attested,
    /// published and released, what moves which dist-tag, which guards held,
    /// and whether this run does it.
    pub(crate) fn markdown(&self) -> String {
        let mut text = self.banner();
        self.write_summary(&mut text);
        if self.refusal().is_some() {
            return text;
        }
        self.write_commands(&mut text);
        text
    }

    /// The run, what happens to each dist-tag, the guards and, when the plan
    /// passed an enforced run, the record for the evidence ledger.
    fn write_summary(&self, text: &mut String) {
        let version = self.version.as_str();
        let channel = self.channel();
        let _ = writeln!(
            text,
            "\n### This run\n\n- Version `{version}` ({}), npm dist-tag `{}`, Git tag `{}`\n\
             - Commit `{}`, event `{}` on `{}` in `{}`\n\
             - The plan is {}",
            self.version.kind().words(),
            channel.dist_tag(),
            self.version.git_tag(),
            self.context.commit,
            self.context.event.name(),
            self.context.git_ref,
            self.context.repository,
            if self.enforcement == Enforcement::Enforced {
                "enforced: a failed guard refuses it"
            } else {
                "report-only: a failed guard is shown and the run carries on"
            },
        );
        let _ = writeln!(
            text,
            "\n### What this publication does to the dist-tags\n\n\
             | Package | Moves `{tag}` from | to | `{other}` stays |\n| --- | --- | --- | --- |",
            tag = channel.dist_tag(),
            other = channel.untouched_dist_tag()
        );
        for moved in &self.moves {
            let _ = writeln!(
                text,
                "| `{}` | `{}` | `{}` | `{}` |",
                moved.package,
                cell(&moved.from),
                cell(&moved.to),
                cell(&moved.untouched)
            );
        }
        let _ = writeln!(
            text,
            "\n### Guards\n\n| Guard | Result | What was seen |\n| --- | --- | --- |"
        );
        for guard in &self.guards {
            let _ = writeln!(
                text,
                "| {} | {} | {} |",
                cell(guard.name),
                guard.outcome.label(),
                cell(&guard.detail)
            );
        }
        if let Some(delta) = &self.release_delta {
            let rendered = serde_json::to_string_pretty(delta).unwrap_or_default();
            let _ = writeln!(
                text,
                "\n### The evidence ledger's `release_delta` record\n\n\
                 Copy this into `docs/planning/p14-evidence-ledger.json` after the publish \
                 (release.md section 6.7); the plan's artifact `{RELEASE_DELTA}` holds it for 7 \
                 days.\n\n```json\n{rendered}\n```"
            );
        }
    }

    /// What `attest` and `publish` would do: the files to attest, the four
    /// `npm publish` commands and the GitHub release commands.
    fn write_commands(&self, text: &mut String) {
        let version = self.version.as_str();
        let channel = self.channel();
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
                shell_words(&publication.arguments(channel)),
            );
        }
        let _ = writeln!(
            text,
            "\n### 3. GitHub release (job `publish`)\n\n`gh {}`, then `gh {}` publishes the draft \
             {}.\n\n| Asset | SHA-256 |\n| --- | --- |",
            shell_words(&self.release.create_arguments()),
            shell_words(&self.release.publish_arguments()),
            match channel {
                Channel::PreRelease => "as a pre-release that is not marked latest",
                Channel::Stable => "as the release marked latest",
            },
        );
        for asset in &self.release.assets {
            let _ = writeln!(text, "| `{}` | `{}` |", asset.name, asset.sha256);
        }
    }

    /// The GitHub release's notes. They name no person.
    pub(crate) fn release_notes(&self) -> String {
        release_notes(
            self.version.kind(),
            self.version.as_str(),
            &self.version.git_tag(),
            &self.context.commit,
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
mod tests;
