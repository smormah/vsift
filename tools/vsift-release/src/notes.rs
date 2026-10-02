//! The GitHub release's notes, in the wording of the claims ladder (P13 PR
//! 10; P14 PR 8, ADR 0024 decision G, known limit L-102).
//!
//! The notes are rendered from Markdown templates in `tools/vsift-release/
//! notes/`, so the public-claims check (`docs/planning/public-claims.json`)
//! reads them like any public document: a controlled word, a banned phrase or
//! a statement above the current rung in a template fails the Governance job.
//! Each kind of version has its own opening, followed by one shared body:
//!
//! - a **release candidate** (`-rc.N`) says it is under qualification, is not
//!   announced and says nothing of support or stability (`candidate.md`);
//! - another **pre-release** says it is a pre-release (`pre-release.md`);
//! - a **stable** release says what it promises (the command-line grammar, the
//!   exit codes and the v1 JSON, additively) and nothing more (`stable.md`).
//!
//! None of the templates uses a controlled word ("supported", "stable",
//! "qualified" and kin): whether a platform is supported is decided by the
//! matrix of the packet's completion, not by a build, so the body names the
//! three machines as the R0 targets. The templates are code, and code is
//! frozen at the release candidate's cut: what the stable release says is what
//! the candidate's build of this module says for a stable version. The
//! paragraph about unsigned executables and Windows Smart App Control is the
//! same in all three (L-098). The notes name no person.

use crate::publish::{REPOSITORY, ReleaseKind, WORKFLOW_PATH};

const CANDIDATE_OPENING: &str = include_str!("../notes/candidate.md");
const PRE_RELEASE_OPENING: &str = include_str!("../notes/pre-release.md");
const STABLE_OPENING: &str = include_str!("../notes/stable.md");
const BODY: &str = include_str!("../notes/body.md");

/// The release notes for `version` of `kind`, built from `commit` and
/// published under `tag`.
pub(crate) fn release_notes(kind: ReleaseKind, version: &str, tag: &str, commit: &str) -> String {
    let dist_tag = kind.channel().dist_tag();
    let installed = match kind {
        ReleaseKind::Stable => String::from("vsift-cli"),
        ReleaseKind::ReleaseCandidate | ReleaseKind::PreRelease => format!("vsift-cli@{dist_tag}"),
    };
    let opening = match kind {
        ReleaseKind::ReleaseCandidate => CANDIDATE_OPENING,
        ReleaseKind::PreRelease => PRE_RELEASE_OPENING,
        ReleaseKind::Stable => STABLE_OPENING,
    };
    let template = format!("{}\n\n{}", opening.trim_end(), BODY.trim_end());
    let values = [
        ("{version}", version),
        ("{tag}", tag),
        ("{commit}", commit),
        ("{dist_tag}", dist_tag),
        ("{installed}", installed.as_str()),
        ("{repository}", REPOSITORY),
        ("{workflow}", WORKFLOW_PATH),
    ];
    let mut text = values
        .iter()
        .fold(template, |text, (name, value)| text.replace(name, value));
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::{BODY, CANDIDATE_OPENING, PRE_RELEASE_OPENING, STABLE_OPENING, release_notes};
    use crate::publish::ReleaseKind;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    /// The Smart App Control, `SmartScreen` and Gatekeeper paragraph of P13 (ADR
    /// 0023 decision C, L-098), word for word: any change to it is deliberate.
    const UNSIGNED: &str = "The executables are not code-signed or notarized, so Windows \
                            SmartScreen and macOS Gatekeeper may warn about one downloaded \
                            directly. Files installed through npm do not carry the download mark \
                            that triggers those two warnings, but Windows Smart App Control, \
                            where it is turned on, can block an unsigned program however it was \
                            installed; see the installation guide.";

    fn notes(kind: ReleaseKind, version: &str) -> String {
        release_notes(kind, version, &format!("v{version}"), COMMIT)
    }

    #[test]
    fn a_candidate_says_it_is_under_qualification_and_claims_nothing() {
        let text = notes(ReleaseKind::ReleaseCandidate, "0.2.0-rc.1");
        assert!(
            text.starts_with("VSift 0.2.0-rc.1 is a release candidate. It is under qualification")
        );
        assert!(text.contains("it is not announced"));
        assert!(text.contains("no statement of support or stability"));
        assert!(text.contains("`latest` is not touched"));
        assert!(text.contains("npm install --global vsift-cli@next"));
        assert!(text.contains("npm audit signatures"));
        assert!(!text.contains("What this release promises"));
    }

    #[test]
    fn a_pre_release_keeps_its_plain_wording() {
        let text = notes(ReleaseKind::PreRelease, "0.3.0-beta.1");
        assert!(text.starts_with("VSift 0.3.0-beta.1 is a pre-release."));
        assert!(text.contains("npm install --global vsift-cli@next"));
        assert!(text.contains("`latest` is not touched"));
    }

    #[test]
    fn a_stable_release_promises_the_contracts_and_installs_from_latest() {
        let text = notes(ReleaseKind::Stable, "0.2.0");
        assert!(text.starts_with("VSift 0.2.0 is published to npm under the dist-tag `latest`"));
        assert!(text.contains("it is the latest GitHub release"));
        assert!(text.contains("npm install --global vsift-cli\n"));
        assert!(!text.contains("vsift-cli@"));
        assert!(text.contains("## What this release promises"));
        assert!(text.contains("change only by addition"));
        assert!(text.contains("synthetic corpus"));
        assert!(text.contains("blob/v0.2.0/docs/planning/known-limits.md"));
        assert!(!text.contains("release candidate"));
    }

    #[test]
    fn every_kind_keeps_the_unsigned_wording_and_the_verification_and_names_nobody() {
        for (kind, version) in [
            (ReleaseKind::ReleaseCandidate, "0.2.0-rc.1"),
            (ReleaseKind::PreRelease, "0.3.0-beta.1"),
            (ReleaseKind::Stable, "0.2.0"),
        ] {
            let text = notes(kind, version);
            assert!(text.contains(UNSIGNED), "{kind:?}");
            assert!(text.contains("Windows Smart App Control, where it is turned on"));
            assert!(text.contains("SmartScreen and macOS Gatekeeper may warn"));
            assert!(text.contains(
                "The executables are built for the three R0 targets: Windows 11 x64, macOS 15 on \
                 Apple silicon, and Linux x64 with glibc 2.35 or later and OpenSSL 3."
            ));
            assert!(text.contains(&format!("--source-ref refs/tags/v{version}")));
            assert!(text.contains(&format!("(tag `v{version}`)")));
            assert!(text.contains(COMMIT));
            assert!(text.contains("Yarn 4 holds back a version for a day"));
            assert!(text.contains(&format!("blob/v{version}/docs/operations/install.md")));
            assert!(text.ends_with("install.md\n"), "{kind:?}");
            // No claim of support or readiness, and nobody named.
            let lower = text.to_ascii_lowercase();
            assert!(!lower.contains("supported"), "{kind:?}");
            assert!(!lower.contains("production"), "{kind:?}");
            assert!(!lower.contains("e-mail"), "{kind:?}");
        }
    }

    /// The templates are read by the claims check as Markdown documents, so they
    /// use no controlled word, whatever the kind; and every placeholder in them
    /// is one this module fills in.
    #[test]
    fn the_templates_use_no_controlled_word_and_no_unfilled_placeholder() {
        for (name, template) in [
            ("candidate.md", CANDIDATE_OPENING),
            ("pre-release.md", PRE_RELEASE_OPENING),
            ("stable.md", STABLE_OPENING),
            ("body.md", BODY),
        ] {
            let lower = template.to_ascii_lowercase();
            for word in [
                "supported",
                "stable",
                "qualified",
                "qualify",
                "qualifies",
                "certified",
                "guaranteed",
            ] {
                let found = lower
                    .split(|character: char| !character.is_alphanumeric() && character != '_')
                    .any(|candidate| candidate == word);
                assert!(!found, "{name} uses the controlled word {word:?}");
            }
        }
        for kind in [
            ReleaseKind::ReleaseCandidate,
            ReleaseKind::PreRelease,
            ReleaseKind::Stable,
        ] {
            let text = notes(kind, "1.2.3");
            for placeholder in [
                "{version}",
                "{tag}",
                "{commit}",
                "{dist_tag}",
                "{installed}",
                "{repository}",
                "{workflow}",
            ] {
                assert!(!text.contains(placeholder), "{kind:?}: {placeholder}");
            }
            assert!(!text.contains("{{") && !text.contains("}}"), "{kind:?}");
        }
    }
}
