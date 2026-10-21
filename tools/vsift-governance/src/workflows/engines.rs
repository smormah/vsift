//! The engine release workflow's rules (ADR 0032 section E): rule 10 of the
//! workflow lint.
//!
//! `engines.yml` builds the engines `VSift` makes itself (whisper.cpp's command
//! line for macOS and Windows, an LGPL `FFmpeg` for macOS, later the OCR runner)
//! and publishes one on this repository's releases under its engine tag. It is
//! the second workflow that can publish, so it is held to the release
//! workflow's standard by rules of its own, each fail-closed:
//!
//! 1. **Triggers.** Only `pull_request` and `workflow_dispatch`, whose inputs
//!    are exactly `engine` (a choice) and `dry_run` (a boolean that defaults to
//!    `true`).
//! 2. **The guard first.** A job `guard` exists, has no `needs` and no `if`,
//!    and every other job needs it; no job may run anyway (`always()`,
//!    `failure()`, `cancelled()`, `continue-on-error`).
//! 3. **Privileged jobs.** `attest` and `publish` exist and run only when every
//!    publish condition holds (a dispatch with `dry_run` cleared, in
//!    `smormah/vsift`, on a tag, with the plan in `publish` mode), with no
//!    escape (`||`, `!`, `always()`, ...); they need `plan`, and `publish`
//!    needs `attest`.
//! 4. **Write scopes.** Only `attest` (`id-token`, `attestations`) and
//!    `publish` (`contents`) have any; `id-token` nowhere else.
//! 5. **Environment.** `publish`, and no other job, runs in the protected
//!    `release` environment, and is never cancelled part-way; nor is a
//!    dispatched run.
//! 6. **Commands.** `gh release` only in `publish`, and only `create` (with
//!    `--verify-tag`, `--draft` and `--latest=false`, never `--clobber` or a
//!    bare `--latest`), `edit` (with `--draft=false` and `--latest=false` and
//!    nothing else but `--repo`) and `view`: an engine release is never marked
//!    latest, never replaces an asset, never deletes. No job tags or pushes, and
//!    `gh api` never writes (no method, field or input).
//! 7. **What is published is what was planned.** `attest` and `publish` use
//!    only their reviewed actions, download only this run's
//!    `engine-${{ needs.guard.outputs.engine }}` artifact, build nothing and
//!    check the files against the plan's `sums` output.
//! 8. **No secret.** The workflow names no `secrets.*`; its only credential is
//!    the job token.
//!
//! The runner rule (rule 9, `runners`) keeps it off the self-hosted runner, as
//! it does the release workflow.

use yaml_rust2::{Yaml, yaml::Hash};

use super::{Lint, get, logical_lines, visit_value_strings};

const GUARD_JOB: &str = "guard";
const PLAN_JOB: &str = "plan";
const ATTEST_JOB: &str = "attest";
const PUBLISH_JOB: &str = "publish";

/// The jobs an engine release needs, in order.
const REQUIRED_JOBS: [&str; 5] = [GUARD_JOB, "build", PLAN_JOB, ATTEST_JOB, PUBLISH_JOB];

/// The engine workflow's triggers: never a push, a schedule or a release event.
const ENGINE_TRIGGERS: [&str; 2] = ["pull_request", "workflow_dispatch"];

/// Every one of these must appear in the `if` of `attest` and `publish`.
/// As in the release workflow, the dispatch input is compared as a string.
const ENGINE_PUBLISH_CONDITIONS: [&str; 5] = [
    "github.event_name == 'workflow_dispatch'",
    "github.event.inputs.dry_run == 'false'",
    "github.repository == 'smormah/vsift'",
    "startsWith(github.ref, 'refs/tags/')",
    "needs.plan.outputs.mode == 'publish'",
];

/// Operators and status functions that could make a condition true without
/// every publish condition.
const CONDITION_ESCAPES: [&str; 5] = ["||", "!", "always()", "failure()", "cancelled()"];

/// Conditions that let a job run although a job it needs failed.
const RUN_ANYWAY: [&str; 3] = ["always()", "failure()", "cancelled()"];

/// The protected environment the maintainer approves.
const RELEASE_ENVIRONMENT: &str = "release";

/// The one expression a run-level `cancel-in-progress` may be, besides
/// `false`: pull requests cancel superseded runs, a dispatch never.
const RUN_CANCELLATION: &str = "${{ github.event_name != 'workflow_dispatch' }}";

/// The artifact the privileged jobs download: this run's chosen engine.
const ENGINE_ARTIFACT: &str = "engine-${{ needs.guard.outputs.engine }}";

/// The plan's output the privileged jobs check the files against.
const PLAN_SUMS: &str = "${{ needs.plan.outputs.sums }}";

/// The `gh release` subcommands the workflow may run.
const RELEASE_SUBCOMMANDS: [&str; 3] = ["create", "edit", "view"];

/// Commands no step of the engine workflow runs.
const FORBIDDEN_COMMANDS: [&str; 6] = [
    "gh release delete",
    "gh release upload",
    "--clobber",
    "git tag",
    "git push",
    "gh workflow",
];

/// Text a privileged job's scripts never contain: it builds and installs
/// nothing, so it publishes exactly the planned bytes.
const PRIVILEGED_FORBIDDEN_TEXT: [&str; 8] = [
    "cargo",
    "cmake",
    "make ",
    "configure",
    "pip ",
    "npm ",
    "curl",
    "wget",
];

/// Applies rule 10 to the engine workflow.
pub(super) fn check_engines(lint: &mut Lint<'_>, workflow: &Hash) {
    check_triggers(lint, workflow);
    check_run_cancellation(lint, workflow);
    check_no_secret(lint, workflow);
    let Some(Yaml::Hash(jobs)) = get(workflow, "jobs") else {
        return;
    };
    for required in REQUIRED_JOBS {
        if !matches!(get(jobs, required), Some(Yaml::Hash(_))) {
            lint.report(&format!(
                "has no `{required}` job; the engine workflow guards, builds, plans, attests and \
                 publishes in the jobs {REQUIRED_JOBS:?} (ADR 0032)"
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
        check_waits_for_guard(lint, id, job);
    }
    for id in [ATTEST_JOB, PUBLISH_JOB] {
        if let Some(Yaml::Hash(job)) = get(jobs, id) {
            check_privileged_job(lint, id, job);
        }
    }
    if let Some(Yaml::Hash(publish)) = get(jobs, PUBLISH_JOB) {
        if !needs(publish).contains(&ATTEST_JOB) {
            lint.report("job `publish` must need `attest`: nothing is published unattested");
        }
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
}

fn check_triggers(lint: &mut Lint<'_>, workflow: &Hash) {
    let Some(Yaml::Hash(triggers)) = get(workflow, "on") else {
        lint.report("must declare its triggers as a mapping, so its inputs can be checked");
        return;
    };
    for trigger in triggers.keys() {
        match trigger.as_str() {
            Some(name) if ENGINE_TRIGGERS.contains(&name) => {}
            other => lint.report(&format!(
                "is triggered by `{}`; the engine workflow runs only on {ENGINE_TRIGGERS:?}, and \
                 publishes only from a dispatch",
                other.unwrap_or("a non-text trigger")
            )),
        }
    }
    let inputs_ok = match get(triggers, "workflow_dispatch") {
        Some(Yaml::Hash(dispatch)) => match get(dispatch, "inputs") {
            Some(Yaml::Hash(inputs)) => {
                inputs.len() == 2
                    && matches!(
                        get(inputs, "dry_run"),
                        Some(Yaml::Hash(input))
                            if get(input, "type").and_then(Yaml::as_str) == Some("boolean")
                                && get(input, "default") == Some(&Yaml::Boolean(true))
                    )
                    && matches!(
                        get(inputs, "engine"),
                        Some(Yaml::Hash(input))
                            if get(input, "type").and_then(Yaml::as_str) == Some("choice")
                    )
            }
            _ => false,
        },
        _ => false,
    };
    if !inputs_ok {
        lint.report(
            "must take exactly two `workflow_dispatch` inputs: `engine` (a choice) and \
             `dry_run` (a boolean with default `true`), so every run is a dry run by default",
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

fn check_no_secret(lint: &mut Lint<'_>, workflow: &Hash) {
    let mut named = false;
    for value in workflow.values() {
        visit_value_strings(value, &mut |text| {
            if text.contains("secrets.") {
                named = true;
            }
        });
    }
    if named {
        lint.report(
            "names a secret; the engine workflow's only credential is the job token (ADR 0032)",
        );
    }
}

fn needs(job: &Hash) -> Vec<&str> {
    match get(job, "needs") {
        Some(Yaml::String(single)) => vec![single.as_str()],
        Some(Yaml::Array(list)) => list.iter().filter_map(Yaml::as_str).collect(),
        _ => Vec::new(),
    }
}

/// The scopes a job may write.
fn allowed_write_scopes(job: &str) -> &'static [&'static str] {
    match job {
        ATTEST_JOB => &["id-token", "attestations"],
        PUBLISH_JOB => &["contents"],
        _ => &[],
    }
}

fn check_write_scopes(lint: &mut Lint<'_>, id: &str, job: &Hash) {
    let Some(Yaml::Hash(permissions)) = get(job, "permissions") else {
        if get(job, "permissions").is_none() {
            lint.report(&format!(
                "job `{id}` declares no `permissions`; every job of the engine workflow names its \
                 own"
            ));
        }
        return;
    };
    for (scope, level) in permissions {
        if let (Some(scope), Some("write")) = (scope.as_str(), level.as_str())
            && !allowed_write_scopes(id).contains(&scope)
        {
            lint.report(&format!(
                "job `{id}` requests `{scope}: write`; in the engine workflow only `attest` \
                 (id-token, attestations) and `publish` (contents) write anything"
            ));
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
            "job `{id}` names an environment; only `publish` runs in one"
        )),
    }
}

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
            let words: Vec<&str> = command.split_whitespace().collect();
            let normalised = words.join(" ");
            for forbidden in FORBIDDEN_COMMANDS {
                if normalised.contains(forbidden) {
                    lint.report(&format!(
                        "job `{id}` runs `{forbidden}`; an engine release never deletes, replaces, \
                         tags, pushes or starts another workflow"
                    ));
                }
            }
            if normalised.contains("gh release") {
                check_gh_release(lint, id, &normalised, &words);
            }
            if normalised.contains("gh api") {
                check_gh_api(lint, id, &words);
            }
        }
    }
}

fn check_gh_release(lint: &mut Lint<'_>, id: &str, normalised: &str, words: &[&str]) {
    if id != PUBLISH_JOB {
        lint.report(&format!(
            "job `{id}` runs `gh release`; only the `publish` job creates the release"
        ));
    }
    let subcommand = words
        .iter()
        .position(|word| *word == "release")
        .and_then(|index| words.get(index + 1))
        .copied()
        .unwrap_or_default();
    if !RELEASE_SUBCOMMANDS.contains(&subcommand) {
        lint.report(&format!(
            "job `{id}` runs `gh release {subcommand}`; the engine workflow only creates, \
             publishes and views its own release ({RELEASE_SUBCOMMANDS:?})"
        ));
    }
    if normalised.contains("gh release create") {
        if !["--verify-tag", "--draft", "--latest=false"]
            .iter()
            .all(|flag| words.contains(flag))
        {
            lint.report(&format!(
                "job `{id}` creates a release without `--verify-tag`, `--draft` and \
                 `--latest=false`: the tag must exist, the release is a draft until it is \
                 published, and an engine release is never latest"
            ));
        }
        if words.contains(&"--latest") || words.contains(&"--latest=true") {
            lint.report(&format!(
                "job `{id}` marks an engine release latest; only a VSift release is ever latest"
            ));
        }
    }
    if normalised.contains("gh release edit") {
        let start = words
            .iter()
            .position(|word| *word == "edit")
            .map_or(0, |index| index + 2);
        let mut flags_ok = true;
        let mut skip_value = false;
        for word in words.iter().skip(start) {
            if skip_value {
                skip_value = false;
            } else if *word == "--repo" {
                skip_value = true;
            } else if word.starts_with('-') && !["--draft=false", "--latest=false"].contains(word) {
                flags_ok = false;
            }
        }
        if !flags_ok || !words.contains(&"--draft=false") || !words.contains(&"--latest=false") {
            lint.report(&format!(
                "job `{id}` edits a release other than with `--repo`, `--draft=false` and \
                 `--latest=false`: it only publishes its own draft, never as latest"
            ));
        }
    }
}

/// `gh api` only reads: no method, field or input, so every call is a GET.
fn check_gh_api(lint: &mut Lint<'_>, id: &str, words: &[&str]) {
    if words.iter().any(|word| {
        matches!(
            *word,
            "-X" | "--method" | "-f" | "-F" | "--field" | "--raw-field" | "--input"
        ) || word.starts_with("--method=")
            || word.starts_with("--field=")
            || word.starts_with("--raw-field=")
    }) {
        lint.report(&format!(
            "job `{id}` runs `gh api` with a method, a field or an input; the engine workflow \
             only reads through it"
        ));
    }
}

fn check_waits_for_guard(lint: &mut Lint<'_>, id: &str, job: &Hash) {
    let condition = get(job, "if").and_then(Yaml::as_str).unwrap_or_default();
    if id == GUARD_JOB {
        if get(job, "needs").is_some() || get(job, "if").is_some() {
            lint.report(
                "job `guard` must have no `needs` and no `if`, so it runs first and cannot be \
                 skipped",
            );
        }
    } else if !needs(job).contains(&GUARD_JOB) {
        lint.report(&format!(
            "job `{id}` does not need `guard`; every job waits for the guard's refusal"
        ));
    }
    for escape in RUN_ANYWAY {
        if condition.contains(escape) {
            lint.report(&format!(
                "job `{id}` has `{escape}` in its condition, so it could run after a refusal"
            ));
        }
    }
    if get(job, "continue-on-error").is_some() {
        lint.report(&format!(
            "job `{id}` sets `continue-on-error`; no job of the engine workflow may"
        ));
    }
}

fn check_privileged_job(lint: &mut Lint<'_>, id: &str, job: &Hash) {
    if !needs(job).contains(&PLAN_JOB) {
        lint.report(&format!("job `{id}` must need `plan`"));
    }
    let condition = get(job, "if").and_then(Yaml::as_str).unwrap_or_default();
    for required in ENGINE_PUBLISH_CONDITIONS {
        if !condition.contains(required) {
            lint.report(&format!(
                "job `{id}` can run without the condition `{required}`; it must run only when \
                 the maintainer dispatches an engine tag with `dry_run` cleared"
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
    let allowed: &[&str] = if id == ATTEST_JOB {
        &[
            "actions/download-artifact",
            "actions/attest-build-provenance",
        ]
    } else {
        &["actions/download-artifact"]
    };
    let mut checks_sums = false;
    for step in steps(job) {
        if let Some(uses) = get(step, "uses").and_then(Yaml::as_str) {
            let action = uses.split('@').next().unwrap_or_default();
            if !allowed.contains(&action) {
                lint.report(&format!(
                    "job `{id}` uses `{action}`; it may use only {allowed:?}"
                ));
            }
            if action == "actions/download-artifact" {
                check_download(lint, id, step);
            }
        }
        if let Some(Yaml::String(script)) = get(step, "run") {
            for forbidden in PRIVILEGED_FORBIDDEN_TEXT {
                if script.contains(forbidden) {
                    lint.report(&format!(
                        "job `{id}` runs `{}`; a privileged job builds and installs nothing",
                        forbidden.trim_end()
                    ));
                }
            }
            let uses_sums = matches!(get(step, "env"), Some(Yaml::Hash(env))
                if env.values().any(|value| value.as_str() == Some(PLAN_SUMS)));
            if uses_sums && script.contains("cmp - dist/SHA256SUMS") {
                checks_sums = true;
            }
        }
    }
    if !checks_sums {
        lint.report(&format!(
            "job `{id}` does not compare its files' sums with `{PLAN_SUMS}` \
             (`cmp - dist/SHA256SUMS`); what is published must be exactly what was planned"
        ));
    }
}

/// A download in a privileged job takes the chosen engine's artifact of this
/// run and nothing else.
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
    if get(with, "name").and_then(Yaml::as_str) != Some(ENGINE_ARTIFACT) {
        lint.report(&format!(
            "job `{id}` downloads another artifact than `{ENGINE_ARTIFACT}`"
        ));
    }
}

#[cfg(test)]
mod tests;
