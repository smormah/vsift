//! The GitHub release's notes, in the wording of the claims ladder (P13 PR
//! 10; P14 PR 8, ADR 0024 decision G).
//!
//! The notes are generated from this module, so they are code, and code is
//! frozen at the release candidate's cut: what the stable release says is
//! what the candidate's build of this module says for a stable version. Each
//! kind has its own opening:
//!
//! - a **release candidate** (`-rc.N`) says it is under qualification, is not
//!   announced and says nothing of support or stability;
//! - another **pre-release** says it is a pre-release;
//! - a **stable** release says what it promises (the command-line grammar,
//!   the exit codes and the v1 JSON, additively) and nothing more.
//!
//! None of them uses the word "supported": whether a platform is supported is
//! decided by the matrix of the packet's completion, not by a build, and the
//! claims check reads these notes. The paragraph about unsigned executables
//! and Windows Smart App Control is the same in all three (L-098). The notes
//! name no person.

use crate::publish::{REPOSITORY, ReleaseKind, WORKFLOW_PATH};

/// What the executables are built for; says nothing of support.
const BUILT_FOR: &str = "Windows 11 x64, macOS 15 on Apple silicon, and Linux x64 with glibc 2.35 \
                         or later and OpenSSL 3";

/// The unsigned-executable paragraph (ADR 0023 decision C, L-098).
const UNSIGNED: &str = "The executables are not code-signed or notarized, so Windows SmartScreen \
                        and macOS Gatekeeper may warn about one downloaded directly. Files \
                        installed through npm do not carry the download mark that triggers those \
                        two warnings, but Windows Smart App Control, where it is turned on, can \
                        block an unsigned program however it was installed; see the installation \
                        guide.";

/// The release notes for `version` of `kind`, built from `commit` and
/// published under `tag`.
pub(crate) fn release_notes(kind: ReleaseKind, version: &str, tag: &str, commit: &str) -> String {
    let dist_tag = kind.channel().dist_tag();
    let installed = match kind {
        ReleaseKind::Stable => String::from("vsift-cli"),
        ReleaseKind::ReleaseCandidate | ReleaseKind::PreRelease => format!("vsift-cli@{dist_tag}"),
    };
    let opening = match kind {
        ReleaseKind::ReleaseCandidate => format!(
            "VSift {version} is a release candidate under qualification. It is published to npm \
             under the dist-tag `{dist_tag}`; `latest` is not touched. It is not announced and \
             is no statement of support or stability: a later candidate or the stable release \
             may replace it. Built by the Release workflow from commit {commit} (tag `{tag}`)."
        ),
        ReleaseKind::PreRelease => format!(
            "VSift {version} is a pre-release. It is published to npm under the dist-tag \
             `{dist_tag}`; `latest` is not touched. Built by the Release workflow from commit \
             {commit} (tag `{tag}`)."
        ),
        ReleaseKind::Stable => format!(
            "VSift {version} is a stable release. It is published to npm under the dist-tag \
             `{dist_tag}`, so `npm install vsift-cli` installs it. Built by the Release \
             workflow from commit {commit} (tag `{tag}`).\n\
             \n\
             ## What a stable release promises\n\
             \n\
             The `vsift` command-line grammar, its exit codes and its v1 JSON contracts. They \
             change only by addition: new commands, fields and failure codes may appear, and \
             what exists keeps its meaning. Nothing else is promised. The accuracy figures were \
             measured on a synthetic corpus; what was tried, and what was not, is in the \
             installation guide and in the register of known limits \
             (https://github.com/{REPOSITORY}/blob/{tag}/docs/planning/known-limits.md)."
        ),
    };
    format!(
        "{opening}\n\
         \n\
         ## Install\n\
         \n\
         The npm package is `vsift-cli`; the command it installs is `vsift`. It needs Node.js 22 \
         or later, or Bun 1.2 or later.\n\
         \n\
         ```console\n\
         npm install --global {installed}\n\
         pnpm add --global {installed}\n\
         bun add --global {installed}\n\
         ```\n\
         \n\
         Yarn 4 holds back a version for a day after it is published (`npmMinimalAgeGate`). \
         Wait a day, or list `vsift-cli` and `@vsift/*` under `npmPreapprovedPackages` in the \
         project's `.yarnrc.yml`.\n\
         \n\
         The executables are built for {BUILT_FOR}.\n\
         \n\
         ## Native archives\n\
         \n\
         Each `vsift-{version}-<target>.tar.gz` holds the `vsift` executable, the licences, \
         `THIRD-PARTY-NOTICES`, a CycloneDX SBOM and the agent skill; each target's SBOM and \
         notices are also attached on their own. Check a download against `SHA256SUMS` \
         (`sha256sum --check --ignore-missing SHA256SUMS`, or `shasum -a 256 --check \
         --ignore-missing SHA256SUMS` on macOS). {UNSIGNED}\n\
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
         The npm packages also carry npm provenance: `npm audit signatures` in a project that \
         installed `{installed}` checks their registry signatures and provenance attestations.\n\
         \n\
         Installation guide: https://github.com/{REPOSITORY}/blob/{tag}/docs/operations/install.md\n"
    )
}

#[cfg(test)]
mod tests {
    use super::{BUILT_FOR, UNSIGNED, release_notes};
    use crate::publish::ReleaseKind;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn notes(kind: ReleaseKind, version: &str) -> String {
        release_notes(kind, version, &format!("v{version}"), COMMIT)
    }

    #[test]
    fn a_candidate_says_it_is_under_qualification_and_claims_nothing() {
        let text = notes(ReleaseKind::ReleaseCandidate, "0.2.0-rc.1");
        assert!(text.starts_with("VSift 0.2.0-rc.1 is a release candidate under qualification."));
        assert!(text.contains("`latest` is not touched"));
        assert!(text.contains("It is not announced"));
        assert!(text.contains("npm install --global vsift-cli@next"));
        assert!(text.contains("npm audit signatures"));
        assert!(!text.contains("is a stable release"));
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
        assert!(text.starts_with("VSift 0.2.0 is a stable release."));
        assert!(text.contains("dist-tag `latest`"));
        assert!(text.contains("npm install --global vsift-cli\n"));
        assert!(!text.contains("vsift-cli@"));
        assert!(text.contains("## What a stable release promises"));
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
            // The Smart App Control and Gatekeeper wording of P13, unchanged.
            assert!(text.contains(UNSIGNED), "{kind:?}");
            assert!(text.contains("Windows Smart App Control, where it is turned on"));
            assert!(text.contains("SmartScreen and macOS Gatekeeper may warn"));
            assert!(text.contains(BUILT_FOR));
            assert!(text.contains(&format!("--source-ref refs/tags/v{version}")));
            assert!(text.contains(&format!("(tag `v{version}`)")));
            assert!(text.contains(COMMIT));
            assert!(text.contains("Yarn 4 holds back a version for a day"));
            assert!(text.contains(&format!("blob/v{version}/docs/operations/install.md")));
            // No claim of support or readiness; the notes name the repository's
            // public owner only through its URL, and nobody else.
            let lower = text.to_ascii_lowercase();
            assert!(!lower.contains("supported"), "{kind:?}");
            assert!(!lower.contains("production"), "{kind:?}");
            assert!(!lower.contains("e-mail"), "{kind:?}");
        }
    }
}
