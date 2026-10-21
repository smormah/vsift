//! Rule 10 on the repository's real `engines.yml`, and on broken copies of
//! it: each copy changes one thing the rule holds, and must be refused.

use std::{error::Error, fs, path::Path};

use super::super::{ENGINES_WORKFLOW, check_workflow};

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
                "      group: engines-publish\n      cancel-in-progress: false",
                "      group: engines-publish\n      cancel-in-progress: true",
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

#[test]
fn every_broken_copy_is_refused() -> TestResult {
    let text = engines_text()?;
    let mut cases = release_command_copies(&text)?;
    cases.extend(trigger_and_condition_copies(&text)?);
    cases.extend(job_rule_copies(&text)?);
    assert_eq!(cases.len(), 28);
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
