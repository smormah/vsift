//! The release workflow's publishing rules (P13 PR 10; ADR 0023 section 1,
//! decisions B and C; R-SEC01, SEC-22).
//!
//! `release.yml` is the only workflow that can publish, so these rules apply
//! to it alone, on top of the rules every workflow meets. Each is
//! fail-closed: a missing job, condition or check is a finding.
//!
//! 1. **Triggers.** Only `pull_request`, `push` (to branches, never tags) and
//!    `workflow_dispatch`, whose one input `dry_run` is a boolean that
//!    defaults to `true`.
//! 2. **Privileged jobs.** The jobs `plan`, `attest` and `publish` exist.
//!    `attest` and `publish` run only when every one of the publish
//!    conditions holds (a dispatch with `dry_run` cleared, in `smormah/vsift`,
//!    on a `v*` tag, with the plan in `publish` mode) and nothing can bypass
//!    them (`||`, `!`, `always()`, `failure()`, `cancelled()`).
//! 3. **Write scopes.** Only `attest` (`id-token`, `attestations`) and
//!    `publish` (`id-token`, `contents`) have any.
//! 4. **Environment.** `publish`, and no other job, runs in the protected
//!    `release` environment, and is never cancelled part-way; nor is the run.
//! 5. **Commands.** `npm publish` appears only in `publish`, always with
//!    `--provenance` and `--tag next`, never `latest`; `gh release` only in
//!    `publish`, creating a pre-release on an existing tag that is not marked
//!    latest; nothing moves dist-tags, unpublishes, deletes a release or
//!    creates or pushes a tag.
//! 6. **Qualified inputs.** `npm-package` exports the tarballs' digests, and
//!    `npm-qualify`, `plan`, `attest` and `publish` each check the tarballs
//!    they use against them. `attest` and `publish` use only their few
//!    reviewed actions, download only this run's `release-dry-run`,
//!    `npm-packages` and `publish-plan` artifacts, and build, pack or install
//!    nothing, so what is published is exactly what was qualified.
//! 7. **Secrets.** The only secret the workflow names is
//!    `NPM_BOOTSTRAP_TOKEN`, and only in `publish` (the first publish of a
//!    package without a trusted publisher, release.md section 6).

use yaml_rust2::{Yaml, yaml::Hash};

use super::{Lint, context_references, expressions, get, logical_lines, visit_value_strings};

const PLAN_JOB: &str = "plan";
const ATTEST_JOB: &str = "attest";
const PUBLISH_JOB: &str = "publish";

/// The release workflow's triggers: never a tag push or a release event.
const RELEASE_TRIGGERS: [&str; 3] = ["pull_request", "push", "workflow_dispatch"];

/// Every one of these must appear in the `if` of `attest` and `publish`.
///
/// The dispatch input is compared as the string `github.event.inputs.dry_run`
/// on purpose: GitHub compares loosely, so `inputs.dry_run == false` is also
/// true when there is no input at all (null and false both become 0), which
/// would let a push or pull request through if the event check were lost.
const PUBLISH_CONDITIONS: [&str; 5] = [
    "github.event_name == 'workflow_dispatch'",
    "github.event.inputs.dry_run == 'false'",
    "github.repository == 'smormah/vsift'",
    "startsWith(github.ref, 'refs/tags/v')",
    "needs.plan.outputs.mode == 'publish'",
];

/// Operators and status functions that could make a condition true without
/// every publish condition.
const CONDITION_ESCAPES: [&str; 5] = ["||", "!", "always()", "failure()", "cancelled()"];

/// The protected environment the maintainer approves.
const RELEASE_ENVIRONMENT: &str = "release";

/// The one expression a run-level `cancel-in-progress` may be, besides
/// `false`: pull requests and pushes cancel superseded runs, a dispatch never.
const RUN_CANCELLATION: &str = "${{ github.event_name != 'workflow_dispatch' }}";

/// The artifacts of this run the privileged jobs may download.
const QUALIFIED_ARTIFACTS: [&str; 3] = ["release-dry-run", "npm-packages", "publish-plan"];

/// The job output carrying the tarballs' digests, and the check that uses it.
const TARBALL_SUMS: &str = "needs.npm-package.outputs.tarball-sums";
const DIGEST_CHECK: &str = "--check --strict";

/// Jobs that must check the tarballs against [`TARBALL_SUMS`].
const TARBALL_CHECKING_JOBS: [&str; 4] = ["npm-qualify", PLAN_JOB, ATTEST_JOB, PUBLISH_JOB];

/// The one secret the release workflow may name, and only in `publish`.
const BOOTSTRAP_SECRET: &str = "secrets.NPM_BOOTSTRAP_TOKEN";

/// Text a privileged job's scripts never contain: it neither builds, packs
/// nor installs anything, so it publishes the qualified bytes.
const PRIVILEGED_FORBIDDEN_COMMANDS: [&str; 8] = [
    "cargo",
    "vsift-release",
    "npm pack",
    "npm install",
    "npm ci",
    "npx",
    "pnpm",
    "yarn",
];

/// Commands only the maintainer runs by hand, never this workflow.
const MAINTAINER_COMMANDS: [&str; 6] = [
    "npm dist-tag",
    "npm unpublish",
    "npm deprecate",
    "gh release delete",
    "git tag",
    "git push",
];

/// Checks the publishing rules of the release workflow.
pub(super) fn check_publishing(lint: &mut Lint<'_>, workflow: &Hash) {
    check_triggers(lint, workflow);
    check_run_cancellation(lint, workflow);
    check_secrets(
        lint,
        "the workflow",
        None,
        workflow
            .iter()
            .filter(|(key, _)| key.as_str() != Some("jobs"))
            .map(|(_, value)| value),
    );
    let Some(Yaml::Hash(jobs)) = get(workflow, "jobs") else {
        return;
    };
    for required in [PLAN_JOB, ATTEST_JOB, PUBLISH_JOB] {
        if !matches!(get(jobs, required), Some(Yaml::Hash(_))) {
            lint.report(&format!(
                "has no `{required}` job; the release workflow plans, attests and publishes in \
                 those three jobs (ADR 0023)"
            ));
        }
    }
    for (id, job) in jobs {
        let (Some(id), Yaml::Hash(job)) = (id.as_str(), job) else {
            continue;
        };
        check_write_scopes(lint, id, job);
        check_environment(lint, id, job);
        check_commands(lint, id, job);
        check_secrets(lint, &format!("job `{id}`"), Some(id), job.values());
    }
    if let Some(Yaml::Hash(plan)) = get(jobs, PLAN_JOB) {
        require_needs(lint, PLAN_JOB, plan, &["npm-qualify"]);
    }
    if let Some(Yaml::Hash(attest)) = get(jobs, ATTEST_JOB) {
        check_privileged_job(
            lint,
            ATTEST_JOB,
            attest,
            &[PLAN_JOB, "npm-qualify"],
            &[
                "actions/download-artifact",
                "actions/attest-build-provenance",
            ],
        );
    }
    if let Some(Yaml::Hash(publish)) = get(jobs, PUBLISH_JOB) {
        check_privileged_job(
            lint,
            PUBLISH_JOB,
            publish,
            &[PLAN_JOB, ATTEST_JOB, "npm-qualify"],
            &["actions/download-artifact", "actions/setup-node"],
        );
        let never_cancelled = matches!(
            get(publish, "concurrency"),
            Some(Yaml::Hash(concurrency))
                if get(concurrency, "cancel-in-progress") == Some(&Yaml::Boolean(false))
        );
        if !never_cancelled {
            lint.report(
                "job `publish` must declare `concurrency` with `cancel-in-progress: false`, so a \
                 publish is never cancelled part-way",
            );
        }
    }
    check_qualified_tarballs(lint, jobs);
}

fn check_triggers(lint: &mut Lint<'_>, workflow: &Hash) {
    let Some(Yaml::Hash(triggers)) = get(workflow, "on") else {
        lint.report(
            "must declare its triggers as a mapping, so the `dry_run` input and the absence of \
             tag triggers can be checked",
        );
        return;
    };
    for trigger in triggers.keys() {
        match trigger.as_str() {
            Some(name) if RELEASE_TRIGGERS.contains(&name) => {}
            other => lint.report(&format!(
                "is triggered by `{}`; the release workflow runs only on {RELEASE_TRIGGERS:?}, \
                 and publishes only from a dispatch",
                other.unwrap_or("a non-text trigger")
            )),
        }
    }
    if let Some(Yaml::Hash(push)) = get(triggers, "push")
        && (get(push, "tags").is_some() || get(push, "tags-ignore").is_some())
    {
        lint.report(
            "is triggered by a tag push; the maintainer dispatches the release on its tag \
             instead, behind the `dry_run` input",
        );
    }
    let dry_run_ok = match get(triggers, "workflow_dispatch") {
        Some(Yaml::Hash(dispatch)) => match get(dispatch, "inputs") {
            Some(Yaml::Hash(inputs)) => {
                inputs.len() == 1
                    && matches!(
                        get(inputs, "dry_run"),
                        Some(Yaml::Hash(input))
                            if get(input, "type").and_then(Yaml::as_str) == Some("boolean")
                                && get(input, "default") == Some(&Yaml::Boolean(true))
                    )
            }
            _ => false,
        },
        _ => false,
    };
    if !dry_run_ok {
        lint.report(
            "must take exactly one `workflow_dispatch` input, `dry_run`, of type boolean with \
             default `true` (ADR 0023: every run is a dry run by default)",
        );
    }
}

fn check_run_cancellation(lint: &mut Lint<'_>, workflow: &Hash) {
    let Some(Yaml::Hash(concurrency)) = get(workflow, "concurrency") else {
        return;
    };
    let safe = match get(concurrency, "cancel-in-progress") {
        None | Some(Yaml::Boolean(false)) => true,
        Some(Yaml::String(expression)) => expression == RUN_CANCELLATION,
        Some(_) => false,
    };
    if !safe {
        lint.report(&format!(
            "cancels runs in progress in a way that could stop a publish part-way; use `false` \
             or `{RUN_CANCELLATION}`"
        ));
    }
}

/// The scopes a job may write.
fn allowed_write_scopes(job: &str) -> &'static [&'static str] {
    match job {
        ATTEST_JOB => &["id-token", "attestations"],
        PUBLISH_JOB => &["id-token", "contents"],
        _ => &[],
    }
}

fn check_write_scopes(lint: &mut Lint<'_>, id: &str, job: &Hash) {
    let Some(Yaml::Hash(permissions)) = get(job, "permissions") else {
        return;
    };
    for (scope, level) in permissions {
        if let (Some(scope), Some("write")) = (scope.as_str(), level.as_str()) {
            // `id-token` has its own rule for every workflow (the parent module).
            if scope != "id-token" && !allowed_write_scopes(id).contains(&scope) {
                lint.report(&format!(
                    "job `{id}` requests `{scope}: write`; in the release workflow only `attest` \
                     (id-token, attestations) and `publish` (id-token, contents) write anything"
                ));
            }
        }
    }
}

fn check_environment(lint: &mut Lint<'_>, id: &str, job: &Hash) {
    let environment = match get(job, "environment") {
        None => None,
        Some(Yaml::String(name)) => Some(name.as_str()),
        Some(Yaml::Hash(environment)) => Some(
            get(environment, "name")
                .and_then(Yaml::as_str)
                .unwrap_or_default(),
        ),
        Some(_) => Some(""),
    };
    match (id == PUBLISH_JOB, environment) {
        (true, Some(RELEASE_ENVIRONMENT)) | (false, None) => {}
        (true, _) => lint.report(&format!(
            "job `publish` must run in the protected `{RELEASE_ENVIRONMENT}` environment, whose \
             required reviewer is the maintainer"
        )),
        (false, Some(_)) => lint.report(&format!(
            "job `{id}` names an environment; only `publish` runs in one (it would gain the \
             environment's secrets and deployment rights)"
        )),
    }
}

/// The `run` scripts of a job's steps, with each step's `env` and `uses`.
fn steps(job: &Hash) -> Vec<&Hash> {
    match get(job, "steps") {
        Some(Yaml::Array(steps)) => steps
            .iter()
            .filter_map(|step| match step {
                Yaml::Hash(step) => Some(step),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn check_commands(lint: &mut Lint<'_>, id: &str, job: &Hash) {
    for step in steps(job) {
        let Some(Yaml::String(script)) = get(step, "run") else {
            continue;
        };
        for command in logical_lines(script) {
            check_command(lint, id, &command);
        }
    }
}

fn check_command(lint: &mut Lint<'_>, id: &str, command: &str) {
    for maintainer_only in MAINTAINER_COMMANDS {
        if command.contains(maintainer_only) {
            lint.report(&format!(
                "job `{id}` runs `{maintainer_only}`, which only the maintainer does by hand \
                 (release.md section 6)"
            ));
        }
    }
    if command.contains("npm publish") {
        if id != PUBLISH_JOB {
            lint.report(&format!(
                "job `{id}` runs `npm publish`; only the `publish` job publishes"
            ));
        }
        if !command.contains("--provenance") || !command.contains("--tag next") {
            lint.report(&format!(
                "job `{id}` runs `npm publish` without `--provenance` and `--tag next` \
                 (ADR 0023 decisions B and C)"
            ));
        }
        if command.contains("latest") {
            lint.report(&format!(
                "job `{id}` names `latest` in `npm publish`; a 0.x pre-release never moves \
                 `latest` (ADR 0023 decision B)"
            ));
        }
    }
    if command.contains("gh release") {
        if id != PUBLISH_JOB {
            lint.report(&format!(
                "job `{id}` runs `gh release`; only the `publish` job creates the release"
            ));
        }
        if command.contains("gh release create")
            && !["--verify-tag", "--prerelease", "--latest=false"]
                .iter()
                .all(|flag| command.contains(flag))
        {
            lint.report(&format!(
                "job `{id}` creates a release without `--verify-tag`, `--prerelease` and \
                 `--latest=false`: the tag must exist, and a 0.x release is a pre-release that \
                 is not marked latest"
            ));
        }
    }
}

/// Reports every `secrets` reference in `values` except the bootstrap
/// token in the `publish` job.
fn check_secrets<'a>(
    lint: &mut Lint<'_>,
    owner: &str,
    job: Option<&str>,
    values: impl IntoIterator<Item = &'a Yaml>,
) {
    let mut found: Vec<String> = Vec::new();
    for value in values {
        visit_value_strings(value, &mut |text| {
            for expression in expressions(text) {
                for reference in context_references(expression) {
                    let root = reference.split(['.', '[']).next().unwrap_or_default();
                    let allowed = job == Some(PUBLISH_JOB) && reference == BOOTSTRAP_SECRET;
                    if root == "secrets" && !allowed && !found.contains(&reference) {
                        found.push(reference);
                    }
                }
            }
        });
    }
    for reference in found {
        lint.report(&format!(
            "{owner} names `{reference}`; the release workflow's only secret is \
             `{BOOTSTRAP_SECRET}`, in the `publish` job"
        ));
    }
}

fn needs(job: &Hash) -> Vec<&str> {
    match get(job, "needs") {
        Some(Yaml::String(need)) => vec![need.as_str()],
        Some(Yaml::Array(needs)) => needs.iter().filter_map(Yaml::as_str).collect(),
        _ => Vec::new(),
    }
}

fn require_needs(lint: &mut Lint<'_>, id: &str, job: &Hash, required: &[&str]) {
    let declared = needs(job);
    for need in required {
        if !declared.contains(need) {
            lint.report(&format!(
                "job `{id}` does not need `{need}`; it must run only after it"
            ));
        }
    }
}

fn check_privileged_job(
    lint: &mut Lint<'_>,
    id: &str,
    job: &Hash,
    required_needs: &[&str],
    allowed_actions: &[&str],
) {
    require_needs(lint, id, job, required_needs);
    let condition = get(job, "if").and_then(Yaml::as_str).unwrap_or_default();
    for required in PUBLISH_CONDITIONS {
        if !condition.contains(required) {
            lint.report(&format!(
                "job `{id}` can run without the condition `{required}`; it must run only when \
                 the maintainer dispatches the release tag with `dry_run` cleared"
            ));
        }
    }
    for escape in CONDITION_ESCAPES {
        if condition.contains(escape) {
            lint.report(&format!(
                "job `{id}` has `{escape}` in its condition, which could run it without every \
                 publish condition"
            ));
        }
    }
    for step in steps(job) {
        if let Some(uses) = get(step, "uses").and_then(Yaml::as_str) {
            let action = uses.split('@').next().unwrap_or_default();
            if !allowed_actions.contains(&action) {
                lint.report(&format!(
                    "job `{id}` uses `{action}`; it may use only {allowed_actions:?}"
                ));
            }
            if action == "actions/download-artifact" {
                check_download(lint, id, step);
            }
        }
        if let Some(Yaml::String(script)) = get(step, "run") {
            for forbidden in PRIVILEGED_FORBIDDEN_COMMANDS {
                if script.contains(forbidden) {
                    lint.report(&format!(
                        "job `{id}` runs `{forbidden}`; a privileged job builds, packs and \
                         installs nothing, so it publishes exactly the qualified bytes"
                    ));
                }
            }
        }
    }
}

/// A download in a privileged job takes one named artifact of this run and
/// nothing else: no pattern, no other run, repository or token.
fn check_download(lint: &mut Lint<'_>, id: &str, step: &Hash) {
    let Some(Yaml::Hash(with)) = get(step, "with") else {
        lint.report(&format!("job `{id}` downloads without naming an artifact"));
        return;
    };
    for key in with.keys() {
        match key.as_str() {
            Some("name" | "path") => {}
            other => lint.report(&format!(
                "job `{id}` downloads with `{}`; a privileged job downloads one named artifact \
                 of this run only",
                other.unwrap_or("a non-text key")
            )),
        }
    }
    let name = get(with, "name").and_then(Yaml::as_str);
    if !name.is_some_and(|name| QUALIFIED_ARTIFACTS.contains(&name)) {
        lint.report(&format!(
            "job `{id}` downloads `{}`; it may download only {QUALIFIED_ARTIFACTS:?}",
            name.unwrap_or("an unnamed artifact")
        ));
    }
}

/// `npm-package` exports the tarballs' digests, and every job that uses the
/// tarballs checks them against that output.
fn check_qualified_tarballs(lint: &mut Lint<'_>, jobs: &Hash) {
    let exported = matches!(
        get(jobs, "npm-package"),
        Some(Yaml::Hash(job))
            if matches!(get(job, "outputs"), Some(Yaml::Hash(outputs))
                if get(outputs, "tarball-sums").is_some())
    );
    if !exported {
        lint.report(
            "job `npm-package` must export `tarball-sums`, the digests of the tarballs it packed",
        );
    }
    for id in TARBALL_CHECKING_JOBS {
        let Some(Yaml::Hash(job)) = get(jobs, id) else {
            lint.report(&format!(
                "has no `{id}` job to check the qualified tarballs"
            ));
            continue;
        };
        let checked = steps(job).into_iter().any(|step| {
            let uses_sums = matches!(get(step, "env"), Some(Yaml::Hash(env))
                if env.values().any(|value| value.as_str().is_some_and(|text| text.contains(TARBALL_SUMS))));
            let checks = matches!(get(step, "run"), Some(Yaml::String(script)) if script.contains(DIGEST_CHECK));
            uses_sums && checks
        });
        if !checked {
            lint.report(&format!(
                "job `{id}` does not check the tarballs against `{TARBALL_SUMS}` with \
                 `{DIGEST_CHECK}`; what is published must be exactly what was qualified"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs, path::Path};

    use super::super::{RELEASE_WORKFLOW, check_workflow};

    fn release_text() -> Result<String, Box<dyn Error>> {
        Ok(fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(RELEASE_WORKFLOW),
        )?)
    }

    fn findings(text: &str) -> Vec<String> {
        let mut messages = Vec::new();
        check_workflow(&mut messages, RELEASE_WORKFLOW, text);
        messages
    }

    /// Replaces the `occurrence`-th (1-based) `from` with `to`, and fails if
    /// there is no such occurrence, so no mutation is silently a no-op.
    fn mutate(
        text: &str,
        from: &str,
        to: &str,
        occurrence: usize,
    ) -> Result<String, Box<dyn Error>> {
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
    fn the_repository_release_workflow_meets_every_publishing_rule() -> Result<(), Box<dyn Error>> {
        assert_eq!(findings(&release_text()?), Vec::<String>::new());
        Ok(())
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one auditable list: each rule's violation of the real workflow and its finding"
    )]
    fn each_publishing_rule_refuses_its_violation() -> Result<(), Box<dyn Error>> {
        let text = release_text()?;
        let dispatch = "  workflow_dispatch:\n";
        let cases: Vec<(&str, String, &str)> = vec![
            (
                "a release event",
                mutate(
                    &text,
                    dispatch,
                    "  release:\n    types: [published]\n  workflow_dispatch:\n",
                    1,
                )?,
                "is triggered by `release`",
            ),
            (
                "a tag push",
                mutate(
                    &text,
                    "  push:\n    branches:\n",
                    "  push:\n    tags:\n      - 'v*'\n    branches:\n",
                    1,
                )?,
                "tag push",
            ),
            (
                "dry_run defaulting to false",
                mutate(
                    &text,
                    "        default: true\n",
                    "        default: false\n",
                    1,
                )?,
                "`dry_run`, of type boolean",
            ),
            (
                "a run that cancels a publish",
                mutate(
                    &text,
                    "cancel-in-progress: ${{ github.event_name != 'workflow_dispatch' }}",
                    "cancel-in-progress: true",
                    1,
                )?,
                "stop a publish part-way",
            ),
            (
                "publish without the release environment",
                mutate(
                    &text,
                    "      name: release\n",
                    "      name: production\n",
                    1,
                )?,
                "protected `release` environment",
            ),
            (
                "another job in an environment",
                mutate(
                    &text,
                    "  plan:\n    name:",
                    "  plan:\n    environment: release\n    name:",
                    1,
                )?,
                "job `plan` names an environment",
            ),
            (
                "attest without the dispatch condition",
                mutate(
                    &text,
                    "github.event_name == 'workflow_dispatch'\n      && ",
                    "",
                    1,
                )?,
                "job `attest` can run without the condition `github.event_name",
            ),
            (
                "publish without the tag condition",
                mutate(&text, "&& startsWith(github.ref, 'refs/tags/v')\n", "", 2)?,
                "job `publish` can run without the condition `startsWith",
            ),
            (
                "a loosely compared dry_run",
                mutate(
                    &text,
                    "github.event.inputs.dry_run == 'false'",
                    "inputs.dry_run == false",
                    2,
                )?,
                "without the condition `github.event.inputs.dry_run == 'false'`",
            ),
            (
                "a bypassable condition",
                mutate(
                    &text,
                    "&& needs.plan.outputs.mode == 'publish'\n",
                    "&& needs.plan.outputs.mode == 'publish' || always()\n",
                    2,
                )?,
                "has `||` in its condition",
            ),
            (
                "publish under latest",
                mutate(
                    &text,
                    "--tag next --access public",
                    "--tag latest --access public",
                    1,
                )?,
                "names `latest` in `npm publish`",
            ),
            (
                "publish without provenance",
                mutate(
                    &text,
                    " --provenance --ignore-scripts",
                    " --ignore-scripts",
                    1,
                )?,
                "without `--provenance` and `--tag next`",
            ),
            (
                "npm publish outside the publish job",
                mutate(
                    &text,
                    "          mkdir publish-plan\n",
                    "          mkdir publish-plan\n          npm publish x.tgz --tag next --provenance\n",
                    1,
                )?,
                "job `plan` runs `npm publish`",
            ),
            (
                "moving a dist-tag",
                mutate(
                    &text,
                    "          mkdir publish-plan\n",
                    "          mkdir publish-plan\n          npm dist-tag add vsift-cli@0.1.0 latest\n",
                    1,
                )?,
                "runs `npm dist-tag`",
            ),
            (
                "a release marked latest",
                mutate(&text, "--prerelease --latest=false", "--prerelease", 1)?,
                "without `--verify-tag`, `--prerelease` and `--latest=false`",
            ),
            (
                "a write scope in the plan",
                mutate(
                    &text,
                    "    permissions:\n      contents: read\n    outputs:\n      mode:",
                    "    permissions:\n      contents: write\n    outputs:\n      mode:",
                    1,
                )?,
                "job `plan` requests `contents: write`",
            ),
            (
                "attest writing contents",
                mutate(
                    &text,
                    "      attestations: write\n",
                    "      attestations: write\n      contents: write\n",
                    1,
                )?,
                "job `attest` requests `contents: write`",
            ),
            (
                "a download from another run",
                mutate(
                    &text,
                    "          name: npm-packages\n          path: npm-packages\n",
                    "          name: npm-packages\n          path: npm-packages\n          run-id: 1\n",
                    3,
                )?,
                "downloads with `run-id`",
            ),
            (
                "an unqualified artifact",
                mutate(
                    &text,
                    "          name: publish-plan\n          path: publish-plan\n",
                    "          name: packages-rebuilt\n          path: publish-plan\n",
                    2,
                )?,
                "downloads `packages-rebuilt`",
            ),
            (
                "a rebuild in the publish job",
                mutate(
                    &text,
                    "          export npm_config_userconfig=",
                    "          cargo run --locked -p vsift-release -- npm --out-dir x\n          export npm_config_userconfig=",
                    1,
                )?,
                "job `publish` runs `cargo`",
            ),
            (
                "another action in attest",
                mutate(
                    &text,
                    "      - name: Attest every release asset and npm tarball\n        uses: actions/attest-build-provenance@",
                    "      - name: Attest every release asset and npm tarball\n        uses: someone/attest@",
                    1,
                )?,
                "job `attest` uses `someone/attest`",
            ),
            (
                "an unchecked qualification",
                mutate(
                    &text,
                    "          TARBALL_SUMS: ${{ needs.npm-package.outputs.tarball-sums }}\n",
                    "          TARBALL_SUMS: unchecked\n",
                    1,
                )?,
                "job `npm-qualify` does not check the tarballs",
            ),
            (
                "an unchecked publish",
                mutate(
                    &text,
                    "          TARBALL_SUMS: ${{ needs.npm-package.outputs.tarball-sums }}\n",
                    "          TARBALL_SUMS: unchecked\n",
                    4,
                )?,
                "job `publish` does not check the tarballs",
            ),
            (
                "no exported digests",
                mutate(
                    &text,
                    "      tarball-sums: ${{ steps.tarballs.outputs.sums }}\n",
                    "",
                    1,
                )?,
                "must export `tarball-sums`",
            ),
            (
                "a secret in the plan",
                mutate(
                    &text,
                    "          SOURCE_COMMIT: ${{ github.sha }}\n",
                    "          SOURCE_COMMIT: ${{ github.sha }}\n          TOKEN: ${{ secrets.NPM_TOKEN }}\n",
                    1,
                )?,
                "job `plan` names `secrets.NPM_TOKEN`",
            ),
            (
                "another secret in the publish job",
                mutate(&text, "secrets.NPM_BOOTSTRAP_TOKEN", "secrets.NPM_TOKEN", 1)?,
                "job `publish` names `secrets.NPM_TOKEN`",
            ),
            (
                "a publish that can be cancelled",
                mutate(
                    &text,
                    "      group: release-publish\n      cancel-in-progress: false\n",
                    "      group: release-publish\n      cancel-in-progress: true\n",
                    1,
                )?,
                "`cancel-in-progress: false`",
            ),
            (
                "publish before the attestation",
                mutate(&text, "      - plan\n      - attest\n", "      - plan\n", 1)?,
                "job `publish` does not need `attest`",
            ),
        ];
        for (case, mutated, expected) in cases {
            let messages = findings(&mutated);
            assert!(
                messages.iter().any(|message| message.contains(expected)),
                "{case}: expected a finding containing {expected:?}, got {messages:#?}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_release_workflow_without_the_publishing_jobs_is_refused() {
        let text = "name: Release\non:\n  workflow_dispatch:\npermissions: {}\njobs:\n  build:\n    \
                    runs-on: ubuntu-24.04\n    steps:\n      - run: cargo build --release --locked \
                    -p vsift-cli --bin vsift\n";
        let messages = findings(text);
        for expected in [
            "has no `plan` job",
            "has no `attest` job",
            "has no `publish` job",
            "exactly one `workflow_dispatch` input",
            "must export `tarball-sums`",
        ] {
            assert!(
                messages.iter().any(|message| message.contains(expected)),
                "{expected}: {messages:#?}"
            );
        }
    }
}
