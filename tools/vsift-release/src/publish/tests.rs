use std::{error::Error, fs, path::Path};

use serde_json::Value;

use super::{
    Channel, DryRunInput, DryRunReason, Enforcement, PRERELEASE_DIST_TAG, PackedPackage,
    PublishError, PublishMode, PublishPlan, ReleaseArchive, ReleaseKind, ReleaseVersion,
    RunContext, STABLE_DIST_TAG, TriggerEvent, decide_mode, enforcement, npm_tarball_name, plan,
    publication_order,
};
use crate::{
    archive::archive_file_name,
    candidate::{CandidateObservation, CandidateReport, ChangeClass, ChangedPath, CheckRecord},
    checksums::checksum_list,
    evidence::EvidenceObservation,
    guards::{GuardOutcome, Observations},
    npm::LAUNCHER_PACKAGE,
    registry::{
        PackageObservation, PackageRecord, PackageState, RegistryObservation, npm_integrity,
    },
    target::ReleaseTarget,
};

const PRE: &str = "0.2.0-rc.1";
const STABLE: &str = "0.2.0";
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

fn release_dispatch(version: &str, dry_run: DryRunInput) -> RunContext {
    context(
        TriggerEvent::WorkflowDispatch,
        &format!("refs/tags/v{version}"),
        dry_run,
    )
}

fn archives(version: &str) -> Vec<ReleaseArchive> {
    ReleaseTarget::ALL
        .into_iter()
        .map(|target| ReleaseArchive {
            target,
            file_name: archive_file_name(version, target),
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

fn tarballs(version: &str) -> Vec<PackedPackage> {
    publication_order()
        .into_iter()
        .rev()
        .map(|package| PackedPackage {
            package,
            file_name: npm_tarball_name(package, version),
            bytes: format!("tarball {package}").into_bytes(),
        })
        .collect()
}

fn version(text: &str) -> Result<ReleaseVersion, Box<dyn Error>> {
    Ok(ReleaseVersion::parse(text)?)
}

/// A plan of `text` in `context` with `observations`.
fn planned(
    text: &str,
    context: RunContext,
    observations: &Observations,
) -> Result<PublishPlan, Box<dyn Error>> {
    let archives = archives(text);
    Ok(plan(
        &version(text)?,
        context,
        &archives,
        &checksums(&archives)?,
        &tarballs(text),
        observations,
    )?)
}

/// The registry as it is just before the stable: a placeholder `latest`, the
/// candidate under `next`.
fn registry_before_the_stable() -> RegistryObservation {
    let record = PackageRecord {
        latest: Some(String::from("0.0.0")),
        next: Some(String::from(PRE)),
        versions: [
            (String::from("0.0.0"), None),
            (String::from("0.1.0"), None),
            (String::from(PRE), Some(String::from("sha512-candidate"))),
        ]
        .into_iter()
        .collect(),
    };
    RegistryObservation::Read(
        publication_order()
            .into_iter()
            .map(|package| PackageObservation {
                package,
                state: PackageState::Found(record.clone()),
            })
            .collect(),
    )
}

fn clean_candidate() -> CandidateObservation {
    CandidateObservation::Checked(Box::new(CandidateReport {
        candidate_tag: String::from("v0.2.0-rc.1"),
        candidate_version: String::from(PRE),
        candidate_commit: "a".repeat(40),
        stable_commit: String::from(COMMIT),
        ancestor: true,
        changes: vec![
            ChangedPath {
                path: String::from("Cargo.toml"),
                verdict: Ok(ChangeClass::VersionString),
            },
            ChangedPath {
                path: String::from("npm/vsift-cli/README.md"),
                verdict: Ok(ChangeClass::ShippedDocument),
            },
        ],
    }))
}

/// The evidence check's answer when the ledger is complete for `clean_candidate()`.
fn complete_evidence() -> EvidenceObservation {
    EvidenceObservation::Ran {
        succeeded: true,
        lines: vec![String::from(
            "VSift release evidence is complete for 0.2.0-rc.1 at aaaaaaaaaaaa.",
        )],
    }
}

fn healthy_stable_observations() -> Observations {
    Observations {
        registry: registry_before_the_stable(),
        candidate: clean_candidate(),
        evidence: complete_evidence(),
        check: None,
    }
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
        "99999999999999999999.0.0",
    ] {
        assert_eq!(
            ReleaseVersion::parse(bad),
            Err(PublishError::Version(bad.to_owned())),
            "{bad}"
        );
    }
}

/// The version alone decides the channel, and a stable version is any one
/// without a suffix, a 0.x version included.
#[test]
fn the_suffix_decides_the_kind_the_channel_and_the_dist_tag() -> Result<(), Box<dyn Error>> {
    for (text, kind, tag) in [
        ("0.2.0", ReleaseKind::Stable, "latest"),
        ("0.1.0", ReleaseKind::Stable, "latest"),
        ("0.0.1", ReleaseKind::Stable, "latest"),
        ("1.0.0", ReleaseKind::Stable, "latest"),
        ("10.20.30", ReleaseKind::Stable, "latest"),
        ("0.2.0-rc.1", ReleaseKind::ReleaseCandidate, "next"),
        ("0.2.0-rc.12", ReleaseKind::ReleaseCandidate, "next"),
        ("1.0.0-rc.3", ReleaseKind::ReleaseCandidate, "next"),
        ("0.2.0-rc.0", ReleaseKind::PreRelease, "next"),
        ("0.2.0-rc", ReleaseKind::PreRelease, "next"),
        ("0.2.0-rc.1.2", ReleaseKind::PreRelease, "next"),
        ("0.2.0-rc.x", ReleaseKind::PreRelease, "next"),
        ("0.2.0-RC.1", ReleaseKind::PreRelease, "next"),
        ("0.2.0-beta", ReleaseKind::PreRelease, "next"),
        ("0.2.0-next.0", ReleaseKind::PreRelease, "next"),
        ("0.2.0-0", ReleaseKind::PreRelease, "next"),
        ("2.0.0-latest", ReleaseKind::PreRelease, "next"),
        ("0.0.0-rc.1", ReleaseKind::ReleaseCandidate, "next"),
    ] {
        let parsed = version(text)?;
        assert_eq!(parsed.kind(), kind, "{text}");
        assert_eq!(parsed.kind().channel().dist_tag(), tag, "{text}");
        // A suffix means `next`; no suffix means `latest`; nothing else does.
        assert_eq!(text.contains('-'), tag == PRERELEASE_DIST_TAG, "{text}");
    }
    assert_eq!(PRERELEASE_DIST_TAG, "next");
    assert_eq!(STABLE_DIST_TAG, "latest");
    assert_eq!(Channel::Stable.output(), "stable");
    assert_eq!(Channel::PreRelease.output(), "prerelease");
    assert_eq!(Channel::Stable.untouched_dist_tag(), "next");
    assert_eq!(Channel::PreRelease.untouched_dist_tag(), "latest");
    Ok(())
}

#[test]
fn a_stable_never_publishes_under_next_and_a_pre_release_never_under_latest()
-> Result<(), Box<dyn Error>> {
    for text in [
        "0.1.0",
        "0.2.0",
        "1.0.0",
        "0.2.0-rc.1",
        "0.3.0-beta.2",
        "1.0.0-rc.1",
    ] {
        let plan = planned(
            text,
            release_dispatch(text, DryRunInput::Set),
            &Observations::none(),
        )?;
        let stable = !text.contains('-');
        for publication in &plan.npm {
            let arguments = publication.arguments(plan.channel());
            let tag = arguments
                .iter()
                .position(|argument| argument == "--tag")
                .and_then(|index| arguments.get(index + 1))
                .map(String::as_str);
            assert_eq!(tag, Some(if stable { "latest" } else { "next" }), "{text}");
            let wrong = if stable { "next" } else { "latest" };
            assert!(
                !arguments.iter().any(|argument| argument == wrong),
                "{text}"
            );
        }
        let create = plan.release.create_arguments();
        let publish = plan.release.publish_arguments();
        assert_eq!(
            create.iter().any(|argument| argument == "--prerelease"),
            !stable,
            "{text}"
        );
        assert_eq!(
            create.iter().any(|argument| argument == "--latest=false"),
            !stable,
            "{text}"
        );
        // Only the publishing edit of a stable release marks it latest.
        assert!(
            !create.iter().any(|argument| argument == "--latest"),
            "{text}"
        );
        assert_eq!(
            publish.iter().any(|argument| argument == "--latest"),
            stable,
            "{text}"
        );
        assert_eq!(
            plan.channel().output(),
            if stable { "stable" } else { "prerelease" }
        );
    }
    Ok(())
}

#[test]
fn only_a_dispatch_on_the_release_tag_with_dry_run_cleared_publishes() -> Result<(), Box<dyn Error>>
{
    let release = version(STABLE)?;
    assert_eq!(
        decide_mode(&release_dispatch(STABLE, DryRunInput::Cleared), &release),
        Ok(PublishMode::Publish)
    );
    assert_eq!(
        decide_mode(&release_dispatch(STABLE, DryRunInput::Set), &release),
        Ok(PublishMode::DryRun(DryRunReason::Requested))
    );
    for event in [TriggerEvent::PullRequest, TriggerEvent::Push] {
        for git_ref in ["refs/pull/7/merge", "refs/heads/main", "refs/tags/v0.2.0"] {
            assert_eq!(
                decide_mode(&context(event, git_ref, DryRunInput::Absent), &release),
                Ok(PublishMode::DryRun(DryRunReason::NotDispatched(event)))
            );
        }
        // A pull request or push never carries the input; one that
        // claims to is refused rather than read.
        assert!(matches!(
            decide_mode(
                &context(event, "refs/tags/v0.2.0", DryRunInput::Cleared),
                &release
            ),
            Err(PublishError::DryRunInput(_))
        ));
    }
    // Asking to publish anywhere but the tag of this version fails loudly.
    for git_ref in [
        "refs/heads/main",
        "refs/tags/v0.2.1",
        "refs/tags/0.2.0",
        "refs/tags/v0.2.0-rc.1",
        "refs/heads/v0.2.0",
    ] {
        let result = decide_mode(
            &context(
                TriggerEvent::WorkflowDispatch,
                git_ref,
                DryRunInput::Cleared,
            ),
            &release,
        );
        assert_eq!(
            result,
            Err(PublishError::NotOnReleaseTag {
                expected: String::from("refs/tags/v0.2.0"),
                actual: git_ref.to_owned()
            }),
            "{git_ref}"
        );
    }
    // The candidate's tag cannot publish the stable version, nor the stable's
    // tag the candidate.
    let candidate = version(PRE)?;
    assert!(matches!(
        decide_mode(&release_dispatch(STABLE, DryRunInput::Cleared), &candidate),
        Err(PublishError::NotOnReleaseTag { .. })
    ));
    assert!(matches!(
        decide_mode(&release_dispatch(PRE, DryRunInput::Cleared), &release),
        Err(PublishError::NotOnReleaseTag { .. })
    ));
    let mut fork = release_dispatch(STABLE, DryRunInput::Cleared);
    fork.repository = String::from("someone/vsift");
    assert_eq!(
        decide_mode(&fork, &release),
        Err(PublishError::ForeignRepository(String::from(
            "someone/vsift"
        )))
    );
    // A fork's dry run is still planned, so its pull requests are checked.
    fork.dry_run = DryRunInput::Set;
    assert_eq!(
        decide_mode(&fork, &release),
        Ok(PublishMode::DryRun(DryRunReason::Requested))
    );
    assert!(matches!(
        decide_mode(&release_dispatch(STABLE, DryRunInput::Absent), &release),
        Err(PublishError::DryRunInput(_))
    ));
    Ok(())
}

#[test]
fn a_dispatch_on_the_release_tag_is_enforced_even_as_a_dry_run() -> Result<(), Box<dyn Error>> {
    let release = version(STABLE)?;
    let enforced = |run: &RunContext, mode| enforcement(run, &release, mode);
    let publish = release_dispatch(STABLE, DryRunInput::Cleared);
    assert_eq!(
        enforced(&publish, PublishMode::Publish),
        Enforcement::Enforced
    );
    let rehearsal = release_dispatch(STABLE, DryRunInput::Set);
    assert_eq!(
        enforced(&rehearsal, PublishMode::DryRun(DryRunReason::Requested)),
        Enforcement::Enforced
    );
    // Anything else only reports.
    for run in [
        context(
            TriggerEvent::WorkflowDispatch,
            "refs/heads/main",
            DryRunInput::Set,
        ),
        context(
            TriggerEvent::WorkflowDispatch,
            "refs/tags/v0.2.1",
            DryRunInput::Set,
        ),
        context(
            TriggerEvent::PullRequest,
            "refs/pull/1/merge",
            DryRunInput::Absent,
        ),
        context(TriggerEvent::Push, "refs/tags/v0.2.0", DryRunInput::Absent),
    ] {
        assert_eq!(
            enforced(&run, PublishMode::DryRun(DryRunReason::Requested)),
            Enforcement::ReportOnly,
            "{run:?}"
        );
    }
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
fn the_placeholder_version_is_refused_in_every_mode() -> Result<(), Box<dyn Error>> {
    let archives = archives("0.0.0");
    let sums = checksums(&archives)?;
    for run in [
        release_dispatch("0.0.0", DryRunInput::Cleared),
        release_dispatch("0.0.0", DryRunInput::Set),
        context(
            TriggerEvent::PullRequest,
            "refs/pull/1/merge",
            DryRunInput::Absent,
        ),
    ] {
        assert_eq!(
            plan(
                &version("0.0.0")?,
                run,
                &archives,
                &sums,
                &tarballs("0.0.0"),
                &Observations::none()
            )
            .err(),
            Some(PublishError::PlaceholderVersion)
        );
    }
    // Only the exact placeholder: a pre-release of it is a version like any other.
    assert!(!version("0.0.0-rc.1")?.is_placeholder());
    assert!(!version("0.0.1")?.is_placeholder());
    Ok(())
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one fixture plan, read back command by command and file by file"
)]
fn the_pre_release_plan_lists_every_command_and_file_exactly() -> Result<(), Box<dyn Error>> {
    let plan = planned(
        PRE,
        release_dispatch(PRE, DryRunInput::Cleared),
        &Observations::none(),
    )?;
    assert_eq!(plan.mode, PublishMode::Publish);
    assert_eq!(
        plan.outputs(),
        "mode=publish\nversion=0.2.0-rc.1\ntag=v0.2.0-rc.1\nchannel=prerelease"
    );
    let commands: Vec<Vec<String>> = plan
        .npm
        .iter()
        .map(|publication| publication.arguments(plan.channel()))
        .collect();
    assert_eq!(
        commands.first().map(|arguments| arguments.join(" ")),
        Some(String::from(
            "publish ./npm-packages/vsift-darwin-arm64-0.2.0-rc.1.tgz --tag next --access \
             public --provenance --ignore-scripts"
        ))
    );
    assert_eq!(
        commands.last().map(|arguments| arguments.join(" ")),
        Some(String::from(
            "publish ./npm-packages/vsift-cli-0.2.0-rc.1.tgz --tag next --access public \
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
            "release create v0.2.0-rc.1 --repo smormah/vsift --verify-tag --draft --prerelease \
             --latest=false --title VSift 0.2.0-rc.1 --notes-file publish-plan/release-notes.md"
        ))
    );
    assert_eq!(
        plan.release.publish_arguments().join(" "),
        "release edit v0.2.0-rc.1 --repo smormah/vsift --draft=false"
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
            "release-assets/vsift-0.2.0-rc.1-aarch64-apple-darwin.THIRD-PARTY-NOTICES.txt",
            "release-assets/vsift-0.2.0-rc.1-aarch64-apple-darwin.cdx.json",
            "release-assets/vsift-0.2.0-rc.1-aarch64-apple-darwin.tar.gz",
            "release-assets/vsift-0.2.0-rc.1-x86_64-pc-windows-msvc.THIRD-PARTY-NOTICES.txt",
            "release-assets/vsift-0.2.0-rc.1-x86_64-pc-windows-msvc.cdx.json",
            "release-assets/vsift-0.2.0-rc.1-x86_64-pc-windows-msvc.tar.gz",
            "release-assets/vsift-0.2.0-rc.1-x86_64-unknown-linux-gnu.THIRD-PARTY-NOTICES.txt",
            "release-assets/vsift-0.2.0-rc.1-x86_64-unknown-linux-gnu.cdx.json",
            "release-assets/vsift-0.2.0-rc.1-x86_64-unknown-linux-gnu.tar.gz",
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
    assert_eq!(json["format"], "vsift-publish-plan/2");
    assert_eq!(json["mode"], "publish");
    assert_eq!(json["enforced"], true);
    assert_eq!(json["kind"], "release_candidate");
    assert_eq!(json["channel"], "prerelease");
    assert_eq!(json["dist_tag"], "next");
    assert_eq!(json["moves_latest"], false);
    assert_eq!(json["github_release"]["latest"], false);
    assert_eq!(json["github_release"]["prerelease"], true);
    assert_eq!(json["npm"].as_array().map(Vec::len), Some(4));
    assert_eq!(json["candidate"], Value::Null);
    let subjects = file("attestation-subjects.sha256");
    assert_eq!(subjects.lines().count(), 14);
    assert!(subjects.contains("  vsift-cli-0.2.0-rc.1.tgz\n"));
    let assets_list = file("release-assets.sha256");
    assert_eq!(assets_list.lines().count(), 10);
    assert!(!assets_list.contains(".tgz"));
    assert_eq!(
        file("vsift-0.2.0-rc.1-x86_64-unknown-linux-gnu.cdx.json"),
        "sbom x86_64-unknown-linux-gnu"
    );
    let summary = file("publish-plan.md");
    assert!(
        summary.contains("PUBLISH a release candidate after the `release` environment's approval")
    );
    assert!(summary.contains("`latest` is not touched"));
    assert!(!summary.contains("moves npm's `latest`"));
    let notes = file("release-notes.md");
    assert!(
        notes.starts_with("VSift 0.2.0-rc.1 is a release candidate. It is under qualification")
    );
    assert!(notes.contains("npm install --global vsift-cli@next"));
    assert!(notes.contains("--source-ref refs/tags/v0.2.0-rc.1"));
    assert!(notes.contains(COMMIT));
    Ok(())
}

#[test]
fn the_stable_plan_says_loudly_that_latest_moves_and_lists_what_moves_it()
-> Result<(), Box<dyn Error>> {
    let plan = planned(
        STABLE,
        release_dispatch(STABLE, DryRunInput::Cleared),
        &healthy_stable_observations(),
    )?;
    assert_eq!(plan.mode, PublishMode::Publish);
    assert_eq!(plan.refusal(), None);
    assert_eq!(
        plan.outputs(),
        "mode=publish\nversion=0.2.0\ntag=v0.2.0\nchannel=stable"
    );
    let commands: Vec<String> = plan
        .npm
        .iter()
        .map(|publication| publication.arguments(plan.channel()).join(" "))
        .collect();
    assert_eq!(
        commands.first().map(String::as_str),
        Some(
            "publish ./npm-packages/vsift-darwin-arm64-0.2.0.tgz --tag latest --access public \
             --provenance --ignore-scripts"
        )
    );
    assert!(
        commands
            .iter()
            .all(|command| command.contains("--tag latest "))
    );
    assert!(commands.iter().all(|command| !command.contains("next")));
    let create = plan.release.create_arguments();
    assert!(
        !create
            .iter()
            .any(|argument| argument.contains("prerelease"))
    );
    assert_eq!(
        plan.release.publish_arguments().join(" "),
        "release edit v0.2.0 --repo smormah/vsift --draft=false --latest"
    );

    let markdown = plan.markdown();
    for expected in [
        "## Publish plan: PUBLISH a STABLE release after the `release` environment's approval",
        "**This publication moves npm's `latest` dist-tag on all four packages.**",
        "Afterwards `npm install vsift-cli` installs `0.2.0`.",
        "Version `0.2.0` (stable), npm dist-tag `latest`, Git tag `v0.2.0`",
        "The plan is enforced: a failed guard refuses it",
        "| Package | Moves `latest` from | to | `next` stays |",
        "| `@vsift/darwin-arm64` | `0.0.0` | `0.2.0` | `0.2.0-rc.1` |",
        "| `@vsift/win32-x64` | `0.0.0` | `0.2.0` | `0.2.0-rc.1` |",
        "| `@vsift/linux-x64` | `0.0.0` | `0.2.0` | `0.2.0-rc.1` |",
        "| `vsift-cli` | `0.0.0` | `0.2.0` | `0.2.0-rc.1` |",
        "| Accepted candidate | passed | against `v0.2.0-rc.1`",
        "| Candidate published | passed |",
        "| `latest` moves forward | passed |",
        "| Evidence ledger | passed | VSift release evidence is complete for 0.2.0-rc.1 at aaaaaaaaaaaa. |",
        "`npm publish ./npm-packages/vsift-cli-0.2.0.tgz --tag latest",
        "as the release marked latest",
    ] {
        assert!(
            markdown.contains(expected),
            "missing {expected:?} in\n{markdown}"
        );
    }
    assert!(!markdown.contains("REFUSED"));
    assert!(!markdown.contains("`latest` is not touched"));

    let json: Value = serde_json::from_str(
        &plan
            .files()?
            .into_iter()
            .find(|(name, _)| name == "publish-plan.json")
            .map(|(_, bytes)| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default(),
    )?;
    assert_eq!(json["channel"], "stable");
    assert_eq!(json["kind"], "stable");
    assert_eq!(json["moves_latest"], true);
    assert_eq!(json["github_release"]["latest"], true);
    assert_eq!(json["github_release"]["prerelease"], false);
    assert_eq!(json["candidate"]["tag"], "v0.2.0-rc.1");
    assert_eq!(json["dist_tags"][0]["tag"], "latest");
    assert_eq!(json["dist_tags"][0]["from"], "0.0.0");
    assert_eq!(json["dist_tags"][0]["to"], "0.2.0");
    assert_eq!(json["dist_tags"][0]["untouched"], "0.2.0-rc.1");
    assert_eq!(json["guards"].as_array().map(Vec::len), Some(6));
    let notes = plan.release_notes();
    assert!(notes.starts_with("VSift 0.2.0 is published to npm under the dist-tag `latest`"));
    assert!(notes.contains("it is the latest GitHub release"));
    Ok(())
}

#[test]
fn an_enforced_stable_plan_that_passes_writes_the_release_delta_record()
-> Result<(), Box<dyn Error>> {
    let observations = Observations {
        check: Some(CheckRecord {
            run_id: 36_959_682_491,
            date: String::from("2026-10-20"),
        }),
        ..healthy_stable_observations()
    };
    let plan = planned(
        STABLE,
        release_dispatch(STABLE, DryRunInput::Cleared),
        &observations,
    )?;
    let files = plan.files()?;
    let delta = files
        .iter()
        .find(|(name, _)| name == "release-delta.json")
        .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
        .ok_or("no release-delta.json")?;
    let record: Value = serde_json::from_str(&delta)?;
    assert_eq!(record["candidate_version"], PRE);
    assert_eq!(record["candidate_commit"], "a".repeat(40));
    assert_eq!(record["stable_version"], STABLE);
    assert_eq!(record["stable_commit"], COMMIT);
    assert_eq!(record["verdict"], "allowed");
    assert_eq!(record["check"]["type"], "workflow_run");
    assert_eq!(record["check"]["workflow"], "Release");
    assert_eq!(record["check"]["run_id"], 36_959_682_491_u64);
    assert_eq!(record["date"], "2026-10-20");
    assert_eq!(record.as_object().map(serde_json::Map::len), Some(7));
    // The summary shows it too, and the plan's own JSON carries it.
    let markdown = plan.markdown();
    assert!(markdown.contains("### The evidence ledger's `release_delta` record"));
    assert!(markdown.contains("\"run_id\": 36959682491"));
    let json: Value = serde_json::from_str(
        &files
            .iter()
            .find(|(name, _)| name == "publish-plan.json")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default(),
    )?;
    assert_eq!(json["release_delta"]["verdict"], "allowed");

    // No record without the run and date; none for a plan that only reports
    // (a pull request), a pre-release or a refused plan.
    let without_check = planned(
        STABLE,
        release_dispatch(STABLE, DryRunInput::Cleared),
        &healthy_stable_observations(),
    )?;
    assert!(without_check.release_delta.is_none());
    let pull_request = planned(
        STABLE,
        context(
            TriggerEvent::PullRequest,
            "refs/pull/9/merge",
            DryRunInput::Absent,
        ),
        &observations,
    )?;
    assert!(pull_request.release_delta.is_none());
    let pre_release = planned(
        PRE,
        release_dispatch(PRE, DryRunInput::Cleared),
        &Observations {
            check: observations.check.clone(),
            ..Observations::none()
        },
    )?;
    assert!(pre_release.release_delta.is_none());
    let refused = planned(
        STABLE,
        release_dispatch(STABLE, DryRunInput::Cleared),
        &Observations {
            evidence: EvidenceObservation::NotRun,
            ..observations
        },
    )?;
    assert!(refused.release_delta.is_none());
    assert!(refused.refusal().is_some());
    Ok(())
}
#[test]
fn a_stable_plan_on_a_pull_request_reports_what_would_refuse_it_and_carries_on()
-> Result<(), Box<dyn Error>> {
    // The state of `main` today: a stable-shaped version, no candidate, and
    // npm already holding the version under `next`.
    let plan = planned(
        STABLE,
        context(
            TriggerEvent::PullRequest,
            "refs/pull/9/merge",
            DryRunInput::Absent,
        ),
        &Observations {
            registry: RegistryObservation::NotRead,
            candidate: CandidateObservation::Failed(String::from("no release candidate tag")),
            ..Observations::none()
        },
    )?;
    assert_eq!(plan.enforcement, Enforcement::ReportOnly);
    assert_eq!(plan.refusal(), None);
    assert_eq!(
        plan.mode,
        PublishMode::DryRun(DryRunReason::NotDispatched(TriggerEvent::PullRequest))
    );
    assert!(plan.outputs().starts_with("mode=dry-run\n"));
    assert!(plan.outputs().ends_with("\nchannel=stable"));
    let markdown = plan.markdown();
    assert!(markdown.starts_with("## Publish plan: dry run, nothing is published"));
    assert!(markdown.contains("a real publication of this plan would move npm's `latest`"));
    assert!(markdown.contains("A real publication of this plan would be refused, because:"));
    assert!(markdown.contains("- Accepted candidate: no release candidate tag"));
    assert!(markdown.contains("- Registry read: the registry was not read"));
    assert!(markdown.contains("report-only: a failed guard is shown and the run carries on"));
    assert!(markdown.contains("| `vsift-cli` | `not read` | `0.2.0` | `not read` |"));
    // The commands are still planned and shown.
    assert!(markdown.contains("### 2. npm, in this order"));
    Ok(())
}

#[test]
fn an_enforced_stable_plan_with_a_failed_guard_is_refused_and_shows_no_command()
-> Result<(), Box<dyn Error>> {
    for run in [
        release_dispatch(STABLE, DryRunInput::Cleared),
        release_dispatch(STABLE, DryRunInput::Set),
    ] {
        let plan = planned(
            STABLE,
            run,
            &Observations {
                registry: registry_before_the_stable(),
                candidate: CandidateObservation::Failed(String::from("no release candidate tag")),
                ..Observations::none()
            },
        )?;
        let reasons = plan.refusal().ok_or("the plan was not refused")?;
        assert_eq!(
            reasons,
            [
                "Accepted candidate: no release candidate tag",
                "Candidate published: there is no accepted candidate to look for on npm",
                "Evidence ledger: there is no accepted candidate whose evidence could be checked"
            ]
        );
        let markdown = plan.markdown();
        assert!(markdown.starts_with("## Publish plan: REFUSED, nothing is published"));
        assert!(markdown.contains("**Refused because:**"));
        assert!(!markdown.contains("### 2. npm"));
        assert!(!markdown.contains("PUBLISH a STABLE"));
        let error = PublishError::Refused(reasons).to_string();
        assert!(error.contains("nothing may be published"));
    }
    Ok(())
}

#[test]
fn a_stable_plan_is_refused_for_each_registry_and_candidate_violation() -> Result<(), Box<dyn Error>>
{
    let stable_run = || release_dispatch(STABLE, DryRunInput::Cleared);
    // latest already ahead, version already on npm under next, other bytes.
    let tarball_integrity = npm_integrity(b"tarball @vsift/darwin-arm64");
    let cases: Vec<(&str, PackageRecord, &str)> = vec![
        (
            "latest ahead",
            PackageRecord {
                latest: Some(String::from("0.3.0")),
                next: None,
                versions: [(String::from(PRE), None)].into_iter().collect(),
            },
            "not a stable version below",
        ),
        (
            "published under next",
            PackageRecord {
                latest: Some(String::from("0.0.0")),
                next: Some(String::from(STABLE)),
                versions: [
                    (String::from(PRE), None),
                    (String::from(STABLE), Some(tarball_integrity)),
                ]
                .into_iter()
                .collect(),
            },
            "another dist-tag",
        ),
        (
            "other bytes",
            PackageRecord {
                latest: Some(String::from("0.0.0")),
                next: None,
                versions: [
                    (String::from(PRE), None),
                    (String::from(STABLE), Some(String::from("sha512-other"))),
                ]
                .into_iter()
                .collect(),
            },
            "not with these bytes",
        ),
        (
            "candidate unpublished",
            PackageRecord {
                latest: Some(String::from("0.0.0")),
                next: None,
                versions: [(String::from("0.0.0"), None)].into_iter().collect(),
            },
            "is not published on",
        ),
    ];
    for (name, record, expected) in cases {
        let registry = RegistryObservation::Read(
            publication_order()
                .into_iter()
                .map(|package| PackageObservation {
                    package,
                    state: PackageState::Found(record.clone()),
                })
                .collect(),
        );
        let plan = planned(
            STABLE,
            stable_run(),
            &Observations {
                registry,
                candidate: clean_candidate(),
                ..Observations::none()
            },
        )?;
        let reasons = plan
            .refusal()
            .ok_or_else(|| format!("{name} was not refused"))?;
        assert!(
            reasons.iter().any(|reason| reason.contains(expected)),
            "{name}: {reasons:#?}"
        );
    }
    Ok(())
}

#[test]
fn a_pre_release_plan_is_enforced_only_for_what_could_make_the_publish_wrong()
-> Result<(), Box<dyn Error>> {
    // A pre-release with no registry observation publishes: the registry is
    // advisory for it.
    let plan = planned(
        PRE,
        release_dispatch(PRE, DryRunInput::Cleared),
        &Observations::none(),
    )?;
    assert_eq!(plan.refusal(), None);
    assert!(
        plan.guards
            .iter()
            .any(|guard| guard.outcome == GuardOutcome::NotEnforced)
    );
    // One that is already `latest` is refused.
    let record = PackageRecord {
        latest: Some(String::from(PRE)),
        next: None,
        versions: [(String::from(PRE), None)].into_iter().collect(),
    };
    let registry = RegistryObservation::Read(
        publication_order()
            .into_iter()
            .map(|package| PackageObservation {
                package,
                state: PackageState::Found(record.clone()),
            })
            .collect(),
    );
    let refused = planned(
        PRE,
        release_dispatch(PRE, DryRunInput::Cleared),
        &Observations {
            registry,
            candidate: CandidateObservation::NotApplicable,
            ..Observations::none()
        },
    )?;
    assert!(refused.refusal().is_some());
    Ok(())
}

#[test]
fn a_dry_run_plans_the_same_commands_and_says_it_publishes_nothing() -> Result<(), Box<dyn Error>> {
    let observations = Observations::none();
    let publish = planned(
        PRE,
        release_dispatch(PRE, DryRunInput::Cleared),
        &observations,
    )?;
    let pull_request = planned(
        PRE,
        context(
            TriggerEvent::PullRequest,
            "refs/pull/9/merge",
            DryRunInput::Absent,
        ),
        &observations,
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
    let good = archives(PRE);
    let sums = checksums(&good)?;
    let run = || release_dispatch(PRE, DryRunInput::Set);
    let release = version(PRE)?;
    let none = Observations::none();

    let mut missing = archives(PRE);
    missing.pop();
    assert!(matches!(
        plan(&release, run(), &missing, &sums, &tarballs(PRE), &none),
        Err(PublishError::Archives(_))
    ));
    let mut renamed = archives(PRE);
    if let Some(archive) = renamed.first_mut() {
        archive.file_name = String::from("vsift.tar.gz");
    }
    assert!(matches!(
        plan(&release, run(), &renamed, &sums, &tarballs(PRE), &none),
        Err(PublishError::Archives(_))
    ));
    let mut changed = archives(PRE);
    if let Some(archive) = changed.first_mut() {
        archive.bytes.push(0);
    }
    assert_eq!(
        plan(&release, run(), &changed, &sums, &tarballs(PRE), &none).err(),
        Some(PublishError::Checksums)
    );

    let mut short = tarballs(PRE);
    short.pop();
    assert!(matches!(
        plan(&release, run(), &good, &sums, &short, &none),
        Err(PublishError::Tarballs(_))
    ));
    let mut misnamed = tarballs(PRE);
    if let Some(tarball) = misnamed.first_mut() {
        tarball.file_name = String::from("vsift-cli-0.3.0.tgz");
    }
    assert!(matches!(
        plan(&release, run(), &good, &sums, &misnamed, &none),
        Err(PublishError::Tarballs(_))
    ));
    let mut doubled = tarballs(PRE);
    doubled.push(PackedPackage {
        package: LAUNCHER_PACKAGE,
        file_name: npm_tarball_name(LAUNCHER_PACKAGE, PRE),
        bytes: Vec::new(),
    });
    assert!(matches!(
        plan(&release, run(), &good, &sums, &doubled, &none),
        Err(PublishError::Tarballs(_))
    ));

    let mut short_commit = run();
    short_commit.commit = String::from("0123456");
    assert!(matches!(
        plan(&release, short_commit, &good, &sums, &tarballs(PRE), &none),
        Err(PublishError::Commit(_))
    ));
    Ok(())
}

/// The privileged jobs run the plan's commands in shell, so that no Rust
/// is compiled where an OIDC token is available. This holds the workflow's
/// commands to the ones this module builds, word for word, for both channels.
#[test]
fn the_release_workflow_runs_the_planned_commands() -> Result<(), Box<dyn Error>> {
    let workflow = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows/release.yml"),
    )?;
    let lines: Vec<&str> = workflow.lines().map(str::trim).collect();
    let order = format!("for package in {}; do", publication_order().join(" "));
    // The publication loop: once per channel's publishing step, and once each
    // in the step that records the dist-tags before and the two that read
    // them after.
    assert!(
        lines.iter().filter(|line| **line == order).count() >= 2,
        "missing: {order}"
    );
    let npm_publishes: Vec<&&str> = lines
        .iter()
        .filter(|line| line.contains("npm publish"))
        .collect();
    assert_eq!(
        npm_publishes.len(),
        2,
        "exactly one npm publish command per channel: {npm_publishes:?}"
    );
    for channel in [Channel::PreRelease, Channel::Stable] {
        let publish = format!(
            "npm publish \"./npm-packages/${{file}}\" {}",
            channel.npm_publish_flags().join(" ")
        );
        assert!(lines.contains(&publish.as_str()), "missing: {publish}");
    }
    // Each channel's GitHub release, in the one place the workflow writes it.
    for channel in [Channel::PreRelease, Channel::Stable] {
        let create = format!(
            "gh release create \"${{TAG}}\" --repo smormah/vsift {} \\",
            channel.github_create_flags().join(" ")
        );
        assert!(lines.contains(&create.as_str()), "missing: {create}");
        let publish = format!(
            "gh release edit \"${{TAG}}\" --repo smormah/vsift {}",
            channel.github_publish_flags().join(" ")
        );
        assert!(lines.contains(&publish.as_str()), "missing: {publish}");
    }
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("gh release create"))
            .count(),
        2
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| **line == "--title \"VSift ${VERSION}\" --notes-file publish-plan/release-notes.md release-assets/*")
            .count(),
        2
    );
    Ok(())
}

/// The lines of a step's `run` script, from the line after `run: |`.
fn step_script(workflow: &str, name: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let mut lines = workflow.lines();
    let header = format!("      - name: {name}");
    lines
        .by_ref()
        .find(|line| *line == header)
        .ok_or_else(|| format!("no step named {name:?}"))?;
    lines
        .by_ref()
        .find(|line| *line == "        run: |")
        .ok_or("the step has no script")?;
    Ok(lines
        .take_while(|line| line.is_empty() || line.starts_with("          "))
        .map(str::to_owned)
        .collect())
}

/// The two channels' publishing steps are written out in full, so no variable
/// ever chooses between `next` and `latest`; this holds them to be the same
/// script from the npm version check on, except for the dist-tag, and holds
/// each one's own guards.
#[test]
fn the_two_publishing_steps_differ_only_in_their_guards_and_their_dist_tag()
-> Result<(), Box<dyn Error>> {
    let workflow = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows/release.yml"),
    )?;
    let prerelease = step_script(
        &workflow,
        "Publish the platform packages, then vsift-cli, under next (pre-release)",
    )?;
    let stable = step_script(
        &workflow,
        "Publish the platform packages, then vsift-cli, under latest (stable)",
    )?;
    let shared_from = |lines: &[String]| -> Result<Vec<String>, Box<dyn Error>> {
        let start = lines
            .iter()
            .position(|line| line.trim_start().starts_with("npm_version="))
            .ok_or("no npm version check")?;
        Ok(lines.get(start..).unwrap_or_default().to_vec())
    };
    let shared_prerelease = shared_from(&prerelease)?;
    let shared_stable = shared_from(&stable)?;
    assert!(shared_stable.len() > 15, "{shared_stable:#?}");
    let as_next: Vec<String> = shared_stable
        .iter()
        .map(|line| line.replace("--tag latest", "--tag next"))
        .collect();
    assert_eq!(shared_prerelease, as_next);
    // What comes before differs: a suffix is required for `next`, none for `latest`.
    let before = |lines: &[String], text: &str| lines.iter().any(|line| line.trim() == text);
    assert!(before(
        &prerelease,
        "[[ \"${VERSION}\" =~ ^[0-9]+\\.[0-9]+\\.[0-9]+-[0-9A-Za-z.-]+$ ]]"
    ));
    assert!(before(
        &stable,
        "[[ \"${VERSION}\" =~ ^[0-9]+\\.[0-9]+\\.[0-9]+$ ]]"
    ));
    assert!(before(&stable, "test \"${VERSION}\" != \"0.0.0\""));
    assert!(before(
        &stable,
        "test \"$(printf '%s\\n%s\\n' \"${latest}\" \"${VERSION}\" | sort -V | tail -n 1)\" = \"${VERSION}\""
    ));
    // The channel conditions are the plan job's output values.
    assert!(workflow.contains("if: needs.plan.outputs.channel == 'prerelease'"));
    assert!(workflow.contains("if: needs.plan.outputs.channel == 'stable'"));
    assert_eq!(Channel::PreRelease.output(), "prerelease");
    assert_eq!(Channel::Stable.output(), "stable");
    Ok(())
}

#[test]
fn the_kind_words_and_keys_are_distinct() {
    let kinds = [
        ReleaseKind::ReleaseCandidate,
        ReleaseKind::PreRelease,
        ReleaseKind::Stable,
    ];
    for (index, kind) in kinds.iter().enumerate() {
        for other in kinds.iter().skip(index + 1) {
            assert_ne!(kind.key(), other.key());
            assert_ne!(kind.words(), other.words());
        }
    }
}
