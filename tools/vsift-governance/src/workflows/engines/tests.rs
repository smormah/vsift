//! Rule 10 on the repository's real `engines.yml`, and on broken copies of
//! it: each copy changes one thing the rule holds, and must be refused.

use std::{error::Error, fs, path::Path};

use super::{
    super::{ENGINES_WORKFLOW, check_workflow},
    check_matrix_against_recipes,
};

type TestResult = Result<(), Box<dyn Error>>;

fn engines_text() -> Result<String, Box<dyn Error>> {
    Ok(fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(ENGINES_WORKFLOW),
    )?)
}

fn findings(text: &str) -> Vec<String> {
    let mut messages = Vec::new();
    check_workflow(&mut messages, ENGINES_WORKFLOW, text);
    check_matrix_against_recipes(
        &mut messages,
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        text,
    );
    messages
}

/// Replaces the `occurrence`-th (1-based) `from` with `to`, and fails if
/// there is no such occurrence, so no mutation is silently a no-op.
fn mutate(text: &str, from: &str, to: &str, occurrence: usize) -> Result<String, Box<dyn Error>> {
    let position = text
        .match_indices(from)
        .nth(occurrence.saturating_sub(1))
        .map(|(position, _)| position)
        .ok_or_else(|| format!("{from:?} occurs fewer than {occurrence} times"))?;
    let mut mutated = String::with_capacity(text.len());
    mutated.push_str(text.get(..position).unwrap_or_default());
    mutated.push_str(to);
    mutated.push_str(text.get(position + from.len()..).unwrap_or_default());
    Ok(mutated)
}

#[test]
fn the_repository_engine_workflow_meets_every_rule() -> TestResult {
    assert_eq!(findings(&engines_text()?), Vec::<String>::new());
    Ok(())
}

type BrokenCopy = (&'static str, String, &'static str);

/// Broken copies of the real workflow, with a word each one's finding
/// must contain.
fn release_command_copies(text: &str) -> Result<Vec<BrokenCopy>, Box<dyn Error>> {
    let create = "gh release create \"${ENGINE_TAG}\" --repo smormah/vsift --verify-tag --draft --latest=false";
    let edit =
        "gh release edit \"${ENGINE_TAG}\" --repo smormah/vsift --draft=false --latest=false";
    Ok(vec![
        (
            "a create that may become latest",
            mutate(text, create, &create.replace(" --latest=false", ""), 1)?,
            "--latest=false",
        ),
        (
            "a create marked latest",
            mutate(text, create, &format!("{create} --latest"), 1)?,
            "latest",
        ),
        (
            "a create without --verify-tag",
            mutate(text, create, &create.replace(" --verify-tag", ""), 1)?,
            "--verify-tag",
        ),
        (
            "a create that replaces assets",
            mutate(text, create, &format!("{create} --clobber"), 1)?,
            "--clobber",
        ),
        (
            "an edit that may become latest",
            mutate(text, edit, &edit.replace(" --latest=false", ""), 1)?,
            "--latest=false",
        ),
        (
            "an edit that sets another flag",
            mutate(text, edit, &format!("{edit} --prerelease"), 1)?,
            "edits a release",
        ),
        (
            "an upload to the release",
            mutate(
                text,
                "gh release view \"${ENGINE_TAG}\"",
                "gh release upload \"${ENGINE_TAG}\" dist/SHA256SUMS",
                1,
            )?,
            "gh release upload",
        ),
        (
            "a deleting API call",
            mutate(
                text,
                "gh api \"repos/smormah/vsift/releases/tags/${ENGINE_TAG}\" \\",
                "gh api -X DELETE \"repos/smormah/vsift/releases/tags/${ENGINE_TAG}\" \\",
                1,
            )?,
            "method",
        ),
        (
            "a tag pushed by the workflow",
            mutate(
                text,
                "set -euo pipefail\n",
                "set -euo pipefail\n          git push origin HEAD\n",
                1,
            )?,
            "git push",
        ),
    ])
}

/// Broken copies of the real workflow, with a word each one's finding
/// must contain.
fn trigger_and_condition_copies(text: &str) -> Result<Vec<BrokenCopy>, Box<dyn Error>> {
    Ok(vec![
        (
            "a dry run off by default",
            mutate(
                text,
                "        type: boolean\n        default: true",
                "        type: boolean\n        default: false",
                1,
            )?,
            "dry_run",
        ),
        (
            "a push trigger",
            mutate(
                text,
                "on:\n  pull_request:",
                "on:\n  push:\n    branches:\n      - main\n  pull_request:",
                1,
            )?,
            "push",
        ),
        (
            "an attest without the dry_run condition",
            mutate(
                text,
                "      && github.event.inputs.dry_run == 'false'\n",
                "",
                1,
            )?,
            "dry_run == 'false'",
        ),
        (
            "a publish without the dry_run condition",
            mutate(
                text,
                "      && github.event.inputs.dry_run == 'false'\n",
                "",
                2,
            )?,
            "dry_run == 'false'",
        ),
        (
            "a publish off a tag",
            mutate(
                text,
                "      && startsWith(github.ref, 'refs/tags/')\n",
                "",
                2,
            )?,
            "refs/tags/",
        ),
        (
            "a publish condition with an escape",
            mutate(
                text,
                "      && needs.plan.outputs.mode == 'publish'\n    runs-on: ubuntu-24.04\n    timeout-minutes: 20",
                "      && needs.plan.outputs.mode == 'publish' || always()\n    runs-on: ubuntu-24.04\n    timeout-minutes: 20",
                1,
            )?,
            "always()",
        ),
        (
            "a publish in another environment",
            mutate(text, "      name: release\n", "      name: staging\n", 1)?,
            "environment",
        ),
        (
            "a build that writes",
            mutate(
                text,
                "    timeout-minutes: 150\n    permissions:\n      contents: read",
                "    timeout-minutes: 150\n    permissions:\n      contents: write",
                1,
            )?,
            "contents: write",
        ),
        (
            "a publish with an OIDC token",
            mutate(
                text,
                "    permissions:\n      contents: write",
                "    permissions:\n      contents: write\n      id-token: write",
                1,
            )?,
            "id-token",
        ),
        (
            "a guard on the self-hosted runner",
            mutate(
                text,
                "    runs-on: ubuntu-24.04\n    timeout-minutes: 5",
                "    runs-on: [self-hosted, loki]\n    timeout-minutes: 5",
                1,
            )?,
            "self-hosted",
        ),
    ])
}

/// Broken copies of the real workflow, with a word each one's finding
/// must contain.
fn job_rule_copies(text: &str) -> Result<Vec<BrokenCopy>, Box<dyn Error>> {
    Ok(vec![
        (
            "a build leg on the self-hosted runner",
            mutate(
                text,
                "            os: windows-2025",
                "            os: loki",
                1,
            )?,
            "self-hosted",
        ),
        (
            "a publish that does not compare the planned sums",
            mutate(
                text,
                "printf '%s\\n' \"${PLAN_SUMS}\" | cmp - dist/SHA256SUMS",
                "true",
                2,
            )?,
            "sums",
        ),
        (
            "a publish that downloads another artifact",
            mutate(
                text,
                "name: planned-${{ needs.guard.outputs.engine }}\n          path: dist",
                "name: engine-whisper-cli-macos-macos-arm64\n          path: dist",
                3,
            )?,
            "artifact",
        ),
        (
            "a publish that can be cancelled",
            mutate(
                text,
                "      group: engines-publish-${{ needs.guard.outputs.tag }}\n      cancel-in-progress: false",
                "      group: engines-publish-${{ needs.guard.outputs.tag }}\n      cancel-in-progress: true",
                1,
            )?,
            "cancel-in-progress",
        ),
        (
            "a plan that runs after a refusal",
            mutate(
                text,
                "    runs-on: ubuntu-24.04\n    timeout-minutes: 10",
                "    if: always()\n    runs-on: ubuntu-24.04\n    timeout-minutes: 10",
                1,
            )?,
            "always()",
        ),
        (
            "a guard that can be skipped",
            mutate(
                text,
                "    name: Engine, ref and tag guard\n",
                "    name: Engine, ref and tag guard\n    if: github.event_name == 'workflow_dispatch'\n",
                1,
            )?,
            "guard",
        ),
        (
            "a secret",
            mutate(
                text,
                "          GH_TOKEN: ${{ github.token }}\n",
                "          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}\n",
                1,
            )?,
            "secret",
        ),
        (
            "an attest with another action",
            mutate(
                text,
                "uses: actions/attest-build-provenance@",
                "uses: actions/attest@",
                1,
            )?,
            "actions/attest",
        ),
        (
            "a privileged job that builds",
            mutate(
                text,
                "          (cd dist && sha256sum --check --strict SHA256SUMS)\n          test",
                "          (cd dist && sha256sum --check --strict SHA256SUMS)\n          cargo build\n          test",
                2,
            )?,
            "builds and installs nothing",
        ),
    ])
}

/// Broken copies that remove or weaken one check of the tag guard (RV-3
/// finding 16) or of the draft check (finding 10), with a text each one's
/// finding must contain.
fn tag_guard_copies(text: &str) -> Result<Vec<BrokenCopy>, Box<dyn Error>> {
    let ref_check = "[ \"${GIT_REF}\" = \"refs/tags/${tag}\" ] || refuse";
    let commit_check = "[ \"${tagged}\" = \"${SOURCE_COMMIT}\" ] || refuse";
    Ok(vec![
        (
            "a guard that no longer checks the ref is the engine's tag",
            mutate(text, ref_check, "true || refuse", 1)?,
            "`guard` no longer checks that the ref is the engine's tag",
        ),
        (
            "a guard that accepts a v* tag",
            mutate(
                text,
                ref_check,
                "[ \"${GIT_REF}\" = \"refs/tags/${tag}\" ] || [[ \"${GIT_REF}\" == refs/tags/v* ]] || refuse",
                1,
            )?,
            "`guard` no longer checks that the ref is the engine's tag",
        ),
        (
            "a guard that names a v* ref",
            mutate(
                text,
                "          [ \"${REPOSITORY}\" = smormah/vsift ]",
                "          [[ \"${GIT_REF}\" == refs/tags/v* ]]\n          [ \"${REPOSITORY}\" = smormah/vsift ]",
                1,
            )?,
            "`refs/tags/v`",
        ),
        (
            "an engine named like a version",
            mutate(text, "| ocr-runner) ;;", "| ocr-runner | v0) ;;", 1)?,
            "named like a version",
        ),
        (
            "a guard that does not compare the tag's commit",
            mutate(text, commit_check, "true || refuse", 1)?,
            "`guard` no longer checks that the tag still names",
        ),
        (
            "a guard without the on-main check",
            mutate(text, "on_main || refuse", "true || refuse", 1)?,
            "`guard` no longer checks that the commit is on `main`",
        ),
        (
            "a guard whose main comparison accepts a commit behind main",
            mutate(
                text,
                "identical | ahead) return 0 ;;",
                "identical | ahead | behind) return 0 ;;",
                1,
            )?,
            "only `identical` and `ahead` pass",
        ),
        (
            "a guard without the no-release check",
            mutate(text, "no_release || refuse", "true || refuse", 1)?,
            "`guard` no longer checks that the tag has no release",
        ),
        (
            "a guard without the draft check",
            mutate(text, "[ -z \"${drafts}\" ] || refuse", "true || refuse", 1)?,
            "`guard` no longer checks that the tag has no draft release",
        ),
        (
            "a guard that sets publish before it checks the ref",
            mutate(
                text,
                ref_check,
                "echo \"mode=publish\" >> \"${GITHUB_OUTPUT}\"\n          [ \"${GIT_REF}\" = \"refs/tags/${tag}\" ] || refuse",
                1,
            )?,
            "only after it has set `mode=publish`",
        ),
    ])
}

/// Broken copies that remove or weaken the re-check `publish` makes after the
/// approval, or the binding of the tag it releases (finding 16).
fn publish_recheck_copies(text: &str) -> Result<Vec<BrokenCopy>, Box<dyn Error>> {
    let tag_binding = "ENGINE_TAG: ${{ needs.guard.outputs.tag }}";
    Ok(vec![
        (
            "a publish that no longer re-checks the ref",
            mutate(
                text,
                "          [ \"${GIT_REF}\" = \"refs/tags/${ENGINE_TAG}\" ]\n",
                "",
                1,
            )?,
            "`publish` no longer re-checks that the ref is still",
        ),
        (
            "a publish that no longer re-checks the tag's commit",
            mutate(
                text,
                "[ \"${tagged}\" = \"${SOURCE_COMMIT}\" ]\n          drafts=",
                "true\n          drafts=",
                1,
            )?,
            "`publish` no longer re-checks that the tag still names",
        ),
        (
            "a publish that no longer re-checks for a draft",
            mutate(text, "[ -z \"${drafts}\" ]\n", "true\n", 1)?,
            "`publish` no longer re-checks that the tag has no draft",
        ),
        (
            "a publish that rechecks the pushed ref name",
            mutate(text, tag_binding, "ENGINE_TAG: ${{ github.ref_name }}", 2)?,
            "binds `ENGINE_TAG`",
        ),
        (
            "a release created for the pushed ref name",
            mutate(text, tag_binding, "ENGINE_TAG: ${{ github.ref_name }}", 3)?,
            "binds `ENGINE_TAG`",
        ),
    ])
}

/// Broken copies for the plan's target count and the build matrix (finding 12).
fn plan_and_matrix_copies(text: &str) -> Result<Vec<BrokenCopy>, Box<dyn Error>> {
    let leg = "          - engine: ocr-runner\n            target: macos-arm64\n            os: macos-15\n";
    Ok(vec![
        (
            "a plan that does not compare the recipe's targets",
            mutate(
                text,
                "if [ \"${expected}\" != \"${present}\" ]; then",
                "if false; then",
                1,
            )?,
            "recipe's targets",
        ),
        (
            "a plan that is not given the recipe's targets",
            mutate(
                text,
                "          ENGINE_TARGETS: ${{ needs.guard.outputs.targets }}\n",
                "",
                1,
            )?,
            "recipe's targets",
        ),
        (
            "a build matrix that drops a leg",
            mutate(text, leg, "", 1)?,
            "no leg for ocr-runner on macos-arm64",
        ),
        (
            "a build matrix with a leg no recipe names",
            mutate(
                text,
                leg,
                &format!(
                    "{leg}          - engine: ocr-runner\n            target: linux-arm64\n            os: ubuntu-24.04-arm\n"
                ),
                1,
            )?,
            "no recipe names",
        ),
    ])
}

/// Broken copies for the spellings of `gh`, of `secrets` and of the publish group.
fn spelling_copies(text: &str) -> Result<Vec<BrokenCopy>, Box<dyn Error>> {
    let releases_api = "gh api \"repos/smormah/vsift/releases/tags/${ENGINE_TAG}\" \\";
    Ok(vec![
        (
            "gh with a global flag before release",
            mutate(
                text,
                "gh release view \"${ENGINE_TAG}\" --repo smormah/vsift",
                "gh -R smormah/vsift release delete \"${ENGINE_TAG}\"",
                1,
            )?,
            "gh release delete",
        ),
        (
            "an API method written without a space",
            mutate(
                text,
                releases_api,
                "gh api -XDELETE \"repos/smormah/vsift/releases/tags/${ENGINE_TAG}\" \\",
                1,
            )?,
            "method",
        ),
        (
            "an API field written without a space",
            mutate(
                text,
                releases_api,
                "gh api -fname=x \"repos/smormah/vsift/releases/tags/${ENGINE_TAG}\" \\",
                1,
            )?,
            "method",
        ),
        (
            "an API typed field written without a space",
            mutate(
                text,
                releases_api,
                "gh api -Fdraft=false \"repos/smormah/vsift/releases/tags/${ENGINE_TAG}\" \\",
                1,
            )?,
            "method",
        ),
        (
            "a secret named by index",
            mutate(
                text,
                "          GH_TOKEN: ${{ github.token }}\n",
                "          GH_TOKEN: ${{ secrets['GITHUB_TOKEN'] }}\n",
                1,
            )?,
            "names a secret",
        ),
        (
            "every secret passed on as JSON",
            mutate(
                text,
                "          GH_TOKEN: ${{ github.token }}\n",
                "          GH_TOKEN: ${{ toJSON(secrets) }}\n",
                1,
            )?,
            "names a secret",
        ),
        (
            "a job that inherits every secret",
            mutate(
                text,
                "    permissions: {}\n    outputs:\n      mode: ${{ steps.plan.outputs.mode }}",
                "    permissions: {}\n    secrets: inherit\n    outputs:\n      mode: ${{ steps.plan.outputs.mode }}",
                1,
            )?,
            "names a secret",
        ),
        (
            "a publish group shared by every engine",
            mutate(
                text,
                "group: engines-publish-${{ needs.guard.outputs.tag }}",
                "group: engines-publish",
                1,
            )?,
            "concurrency group",
        ),
    ])
}

#[test]
fn every_broken_copy_is_refused() -> TestResult {
    let text = engines_text()?;
    let mut cases = release_command_copies(&text)?;
    cases.extend(trigger_and_condition_copies(&text)?);
    cases.extend(job_rule_copies(&text)?);
    cases.extend(tag_guard_copies(&text)?);
    cases.extend(publish_recheck_copies(&text)?);
    cases.extend(plan_and_matrix_copies(&text)?);
    cases.extend(spelling_copies(&text)?);
    assert_eq!(cases.len(), 55);
    for (name, copy, expected) in cases {
        let messages = findings(&copy);
        assert!(
            messages.iter().any(|message| message.contains(expected)),
            "{name}: no finding mentions {expected:?}: {messages:?}"
        );
    }
    Ok(())
}

/// Another workflow may not attest: `id-token` is the engine workflow's
/// `attest` job's and the release workflow's alone.
#[test]
fn the_id_token_allowance_is_the_engine_workflows_attest_job_alone() -> TestResult {
    let text = engines_text()?;
    let mut messages = Vec::new();
    check_workflow(&mut messages, ".github/workflows/other.yml", &text);
    assert!(
        messages.iter().any(|message| message.contains("id-token")),
        "{messages:?}"
    );
    Ok(())
}
