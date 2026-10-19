//! Rule 9's tests: the repository's overnight workflow passes, and a broken
//! copy of it fails for each rule it breaks.

use std::{error::Error, fs, path::Path};

use super::super::{RELEASE_WORKFLOW, check_workflow};
use super::is_hosted_label;

const OVERNIGHT: &str = ".github/workflows/loki-overnight.yml";

fn overnight_text() -> Result<String, Box<dyn Error>> {
    Ok(fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(OVERNIGHT),
    )?)
}

fn findings(path: &str, text: &str) -> Vec<String> {
    let mut messages = Vec::new();
    check_workflow(&mut messages, path, text);
    messages
}

/// Replaces the one occurrence of `from` with `to`, and fails unless `from`
/// occurs exactly once, so no mutation is silently a no-op or lands elsewhere.
fn mutate(text: &str, from: &str, to: &str) -> Result<String, Box<dyn Error>> {
    let count = text.matches(from).count();
    if count != 1 {
        return Err(format!("{from:?} occurs {count} times, not once").into());
    }
    Ok(text.replacen(from, to, 1))
}

#[test]
fn the_repository_overnight_workflow_meets_the_runner_policy() -> Result<(), Box<dyn Error>> {
    let text = overnight_text()?;
    assert_eq!(findings(OVERNIGHT, &text), Vec::<String>::new());
    assert!(text.contains("runs-on: [self-hosted, loki]"));
    Ok(())
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one auditable list: each rule's violation of the real workflow and its finding"
)]
fn each_runner_rule_refuses_its_violation_of_the_overnight_workflow() -> Result<(), Box<dyn Error>>
{
    let text = overnight_text()?;
    let dispatch = "  workflow_dispatch:\n    inputs:\n";
    let job_permissions = "    permissions:\n      contents: read\n    defaults:\n";
    let checkout = "          persist-credentials: false\n          clean: true\n";
    let runs_on = "    runs-on: [self-hosted, loki]\n";
    let timeout = "    timeout-minutes: 330\n";
    let plan_env = "        env:\n          PLAN: ${{ needs.plan.outputs.plan }}\n";
    let report_env = "          GH_TOKEN: ${{ github.token }}\n          RESULTS:";
    let mut cases: Vec<(&str, String, &str)> = Vec::new();
    for trigger in [
        "pull_request",
        "pull_request_target",
        "push",
        "workflow_run",
        "issue_comment",
        "workflow_call",
        "merge_group",
    ] {
        cases.push((
            trigger,
            mutate(&text, dispatch, &format!("  {trigger}:\n{dispatch}"))?,
            "is started only by `schedule` or `workflow_dispatch`",
        ));
    }
    cases.extend([
        (
            "triggers as a list",
            mutate(
                &text,
                "on:\n  schedule:\n",
                "on: [schedule, pull_request]\nx-on:\n  schedule:\n",
            )?,
            "triggered by `pull_request`",
        ),
        (
            "a LOKI job that may write contents",
            mutate(
                &text,
                job_permissions,
                "    permissions:\n      contents: write\n    defaults:\n",
            )?,
            "holds `contents: write`",
        ),
        (
            "a LOKI job with a second read scope",
            mutate(
                &text,
                job_permissions,
                "    permissions:\n      contents: read\n      actions: read\n    defaults:\n",
            )?,
            "must declare `permissions` of exactly `contents: read`",
        ),
        (
            "a LOKI job without its own permissions",
            mutate(&text, job_permissions, "    defaults:\n")?,
            "must declare `permissions` of exactly `contents: read`",
        ),
        (
            "a report job that may write pull requests",
            mutate(
                &text,
                "      issues: write\n",
                "      issues: write\n      pull-requests: write\n",
            )?,
            "holds `pull-requests: write`",
        ),
        (
            "a planning job that may attest",
            mutate(
                &text,
                "    timeout-minutes: 10\n    permissions:\n      contents: read\n    outputs:",
                "    timeout-minutes: 10\n    permissions:\n      contents: read\n      attestations: write\n    outputs:",
            )?,
            "holds `attestations: write`",
        ),
        (
            "a job in a deployment environment",
            mutate(
                &text,
                "    name: Report a failure\n",
                "    name: Report a failure\n    environment: release\n",
            )?,
            "runs in a deployment environment",
        ),
        (
            "a secret in the report job",
            mutate(
                &text,
                report_env,
                "          GH_TOKEN: ${{ github.token }}\n          NPM: ${{ secrets.NPM_TOKEN }}\n          RESULTS:",
            )?,
            "names `secrets.NPM_TOKEN`",
        ),
        (
            "the job token handed to the LOKI job",
            mutate(
                &text,
                plan_env,
                "        env:\n          GH_TOKEN: ${{ github.token }}\n          PLAN: ${{ needs.plan.outputs.plan }}\n",
            )?,
            "names `github.token`",
        ),
        (
            "GITHUB_TOKEN handed to the LOKI job",
            mutate(
                &text,
                plan_env,
                "        env:\n          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}\n          PLAN: ${{ needs.plan.outputs.plan }}\n",
            )?,
            "names `secrets.GITHUB_TOKEN`",
        ),
        (
            "a checkout that keeps its credentials",
            mutate(
                &text,
                checkout,
                "          persist-credentials: true\n          clean: true\n",
            )?,
            "without `persist-credentials: false`",
        ),
        (
            "a checkout with the default credentials",
            mutate(&text, checkout, "          clean: true\n")?,
            "without `persist-credentials: false`",
        ),
        (
            "a LOKI job without a timeout",
            mutate(&text, timeout, "")?,
            "without a literal `timeout-minutes`",
        ),
        (
            "a LOKI job with a timeout of days",
            mutate(&text, timeout, "    timeout-minutes: 4320\n")?,
            "without a literal `timeout-minutes`",
        ),
        (
            "a LOKI job whose timeout an input sets",
            mutate(&text, timeout, "    timeout-minutes: ${{ inputs.minutes }}\n")?,
            "without a literal `timeout-minutes`",
        ),
        (
            "self-hosted without loki",
            mutate(&text, runs_on, "    runs-on: [self-hosted, windows]\n")?,
            "without naming `loki`",
        ),
        (
            "an unknown runner label",
            mutate(&text, runs_on, "    runs-on: [self-hosted, loki, gpu]\n")?,
            "names the runner label `gpu`",
        ),
        (
            "a runner group",
            mutate(
                &text,
                runs_on,
                "    runs-on:\n      group: overnight\n      labels: [loki]\n",
            )?,
            "names a runner group",
        ),
        (
            "a runner an input chooses",
            mutate(&text, runs_on, "    runs-on: ${{ inputs.runner }}\n")?,
            "names its runner through `${{ inputs.runner }}`",
        ),
    ]);
    for (name, mutated, expected) in &cases {
        let messages = findings(OVERNIGHT, mutated);
        assert!(
            messages.iter().any(|message| message.contains(expected)),
            "{name}: expected a finding containing {expected:?}, got {messages:#?}"
        );
    }
    Ok(())
}

#[test]
fn the_release_workflow_never_runs_on_the_self_hosted_runner() -> Result<(), Box<dyn Error>> {
    let messages = findings(RELEASE_WORKFLOW, &overnight_text()?);
    assert!(
        messages
            .iter()
            .any(|message| message
                .contains("the release workflow runs only on GitHub-hosted runners")),
        "{messages:#?}"
    );
    let release = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(RELEASE_WORKFLOW),
    )?;
    let moved = mutate(
        &release,
        "  guard:\n    name: Ref and nightly guard\n    runs-on: ubuntu-24.04\n",
        "  guard:\n    name: Ref and nightly guard\n    runs-on: [self-hosted, loki]\n",
    )?;
    let messages = findings(RELEASE_WORKFLOW, &moved);
    assert!(
        messages
            .iter()
            .any(|message| message.contains("runs only on GitHub-hosted runners")),
        "{messages:#?}"
    );
    Ok(())
}

fn workflow(triggers: &str, job: &str) -> String {
    format!("name: X\non:\n{triggers}permissions:\n  contents: read\njobs:\n{job}")
}

const LOKI_JOB: &str = "  a:\n    runs-on: RUNNER\n    timeout-minutes: 30\n    permissions:\n      \
                        contents: read\n    strategy:\n      matrix:\n        os: [ubuntu-latest, loki]\n    \
                        steps:\n      - run: echo hi\n";

#[test]
fn a_matrix_cell_that_reaches_loki_makes_the_job_a_loki_job() {
    let job = LOKI_JOB.replace("RUNNER", "${{ matrix.os }}");
    let messages = findings(
        ".github/workflows/x.yml",
        &workflow("  pull_request:\n", &job),
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("triggered by `pull_request`")),
        "{messages:#?}"
    );
    assert_eq!(
        findings(
            ".github/workflows/x.yml",
            &workflow("  schedule:\n    - cron: \"1 1 * * *\"\n", &job)
        ),
        Vec::<String>::new()
    );
    let included = "  a:\n    runs-on: ${{ matrix.runner }}\n    strategy:\n      matrix:\n        \
                    include:\n          - runner: ubuntu-24.04\n          - runner: x64\n    steps:\n      \
                    - run: echo hi\n";
    let messages = findings(".github/workflows/x.yml", &workflow("  push:\n", included));
    assert!(
        messages
            .iter()
            .any(|message| message.contains("without naming `loki`")),
        "{messages:#?}"
    );
}

#[test]
fn only_the_listed_job_may_compute_its_runner() {
    let job = "  quality:\n    runs-on: ${{ matrix.os }}\n    strategy:\n      matrix:\n        \
               os: ${{ fromJSON(needs.changes.outputs.systems) }}\n    steps:\n      - run: echo hi\n";
    let text = workflow("  pull_request:\n", job);
    assert_eq!(
        findings(".github/workflows/ci.yml", &text),
        Vec::<String>::new()
    );
    let messages = findings(".github/workflows/other.yml", &text);
    assert_eq!(messages.len(), 1, "{messages:#?}");
    assert!(
        messages
            .iter()
            .all(|message| message.contains("cannot read"))
    );
    let renamed = text.replacen("  quality:", "  lint:", 1);
    assert_eq!(findings(".github/workflows/ci.yml", &renamed).len(), 1);
}

#[test]
fn a_loki_workflow_passes_no_secret_to_a_reusable_workflow() {
    let jobs = "  a:\n    runs-on: loki\n    timeout-minutes: 30\n    permissions:\n      contents: read\n    \
                steps:\n      - run: echo hi\n  b:\n    uses: ./.github/workflows/guide.yml\n    secrets: inherit\n";
    let messages = findings(
        ".github/workflows/x.yml",
        &workflow("  workflow_dispatch:\n", jobs),
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("passes secrets to a reusable workflow")),
        "{messages:#?}"
    );
}

#[test]
fn hosted_image_labels_are_told_from_others() {
    for hosted in [
        "ubuntu-latest",
        "ubuntu-24.04",
        "ubuntu-22.04",
        "ubuntu-24.04-arm",
        "windows-latest",
        "windows-2025",
        "windows-11-arm",
        "macos-latest",
        "macos-15",
        "macos-15-xlarge",
        "macos-14-large",
    ] {
        assert!(is_hosted_label(hosted), "{hosted}");
    }
    for other in [
        "loki",
        "self-hosted",
        "windows",
        "ubuntu",
        "ubuntu-",
        "ubuntu-24.",
        "ubuntu-next",
        "my-ubuntu-24.04",
        "linux-24.04",
        "windows-2025-gpu",
    ] {
        assert!(!is_hosted_label(other), "{other}");
    }
}
