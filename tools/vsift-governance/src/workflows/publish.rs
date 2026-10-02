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
//!    `--provenance` and an explicit `--tag next` or `--tag latest`; `gh
//!    release` only in `publish`, creating a draft on an existing tag; `gh
//!    api` only to read the latest release; nothing deprecates, unpublishes,
//!    deletes a release or creates or pushes a tag, and no workflow moves a
//!    dist-tag (the general rule 8 of the parent module).
//! 6. **Qualified inputs.** `npm-package` exports the tarballs' digests, and
//!    `npm-qualify`, `plan`, `attest` and `publish` each check the tarballs
//!    they use against them. `attest` and `publish` use only their few
//!    reviewed actions, download only this run's `release-dry-run`,
//!    `npm-packages` and `publish-plan` artifacts, and build, pack or install
//!    nothing, so what is published is exactly what was qualified.
//! 7. **Secrets.** The only secret the workflow names is
//!    `NPM_BOOTSTRAP_TOKEN`, and only in `publish` (the first publish of a
//!    package without a trusted publisher, release.md section 6).
//! 8. **Channels** (P14 PR 8, ADR 0024 decisions A and B). The `plan` job
//!    exports the version's `channel` (`prerelease` or `stable`). Every step of
//!    the release workflow that publishes or releases serves exactly one
//!    channel and runs only when the plan says so: `--tag next`, `gh release
//!    create ... --prerelease --latest=false` and the pre-release's `gh release
//!    edit` only in a step whose `if` is `needs.plan.outputs.channel ==
//!    'prerelease'`; `--tag latest`, a `gh release create` without
//!    `--prerelease` and a `gh release edit ... --latest` only in a step whose
//!    `if` is `needs.plan.outputs.channel == 'stable'`. Each `npm publish`
//!    step checks the version's own shape in shell first (a suffix for `next`,
//!    none for `latest`), so a pre-release can never reach `latest` and a
//!    stable version never `next`, by two independent paths.
//! 9. **The stable path's inputs.** The `plan` job checks out the full
//!    history (the candidate comparison needs the candidate's tag), reads the
//!    registry with plain GETs of `https://registry.npmjs.org/` only, and
//!    passes the result to `publish-plan` with `--registry`. It also names the
//!    accepted release candidate (`candidate-delta --github-output`), checks the
//!    evidence ledger for it (`vsift-governance release-evidence --complete-for
//!    ... --commit ...`, RQ-20) and hands the answer, the run id and the date to
//!    `publish-plan` (`--evidence`, `--run-id`, `--date`). A `curl` anywhere in
//!    the workflow takes only the reviewed read-only flags.

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
const PRIVILEGED_FORBIDDEN_COMMANDS: [&str; 10] = [
    "cargo",
    "vsift-release",
    "npm pack",
    "npm install",
    "npm ci",
    "npx",
    "pnpm",
    "yarn",
    "curl",
    "wget",
];

/// Commands only the maintainer runs by hand, never this workflow. (`npm
/// dist-tag` is the parent module's rule for every workflow: `latest` moves
/// only by publishing a stable version.)
const MAINTAINER_COMMANDS: [&str; 5] = [
    "npm unpublish",
    "npm deprecate",
    "gh release delete",
    "git tag",
    "git push",
];

/// The two channels of a release (P14 PR 8). The plan job's `channel` output
/// names one; each step that publishes or releases serves one and says so in
/// its `if`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Channel {
    /// A version with a pre-release suffix: `--tag next`, a GitHub
    /// pre-release.
    PreRelease,
    /// A version without one: `--tag latest`, the release marked latest.
    Stable,
}

impl Channel {
    const fn name(self) -> &'static str {
        match self {
            Self::PreRelease => "prerelease",
            Self::Stable => "stable",
        }
    }

    /// The condition a step of this channel runs under.
    const fn condition(self) -> &'static str {
        match self {
            Self::PreRelease => "needs.plan.outputs.channel == 'prerelease'",
            Self::Stable => "needs.plan.outputs.channel == 'stable'",
        }
    }

    const fn other(self) -> Self {
        match self {
            Self::PreRelease => Self::Stable,
            Self::Stable => Self::PreRelease,
        }
    }

    /// The line an `npm publish` step of this channel opens with, so the
    /// version's own shape is checked in the privileged job as well as in the
    /// plan: a suffix for `next`, none for `latest`.
    const fn shape_check(self) -> &'static str {
        match self {
            Self::PreRelease => r#"[[ "${VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+-[0-9A-Za-z.-]+$ ]]"#,
            Self::Stable => r#"[[ "${VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]"#,
        }
    }
}

/// The only `curl` flags the release workflow uses: a read-only GET with a
/// bounded time and no header, body, credential, configuration or redirect.
const CURL_FLAGS: [&str; 7] = [
    "--silent",
    "--show-error",
    "--proto",
    "--max-time",
    "--retry",
    "--output",
    "--write-out",
];

/// Where `curl` may go: npm's public registry, nowhere else.
const REGISTRY_URL: &str = "https://registry.npmjs.org/";

/// The one thing `gh api` may read in the release workflow.
const LATEST_RELEASE_PATH: &str = "repos/smormah/vsift/releases/latest";

/// The flags a `gh release edit` may carry besides its tag.
const RELEASE_EDIT_FLAGS: [&str; 3] = ["--repo", "--draft=false", "--latest"];

/// The `gh release` subcommands the workflow may run: it never uploads to a
/// published release or changes one but to publish its own draft.
const RELEASE_SUBCOMMANDS: [&str; 3] = ["create", "edit", "view"];

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
        check_plan_job(lint, plan);
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
        check_channel_steps(lint, publish);
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
        let mut implied: Vec<Channel> = Vec::new();
        for command in logical_lines(script) {
            if let Some(channel) = check_command(lint, id, &command)
                && !implied.contains(&channel)
            {
                implied.push(channel);
            }
        }
        check_step_channel(lint, id, step, script, &implied);
    }
}

/// Checks one command of a `run` script and returns the channel it serves,
/// if it publishes or releases.
fn check_command(lint: &mut Lint<'_>, id: &str, command: &str) -> Option<Channel> {
    let words: Vec<&str> = command.split_whitespace().collect();
    // Single spaces, so `npm  dist-tag` or a tab cannot slip past a match.
    let normalised = words.join(" ");
    for maintainer_only in MAINTAINER_COMMANDS {
        if normalised.contains(maintainer_only) {
            lint.report(&format!(
                "job `{id}` runs `{maintainer_only}`, which only the maintainer does by hand \
                 (release.md section 6)"
            ));
        }
    }
    let mut channel = None;
    if normalised.contains("npm publish") {
        channel = check_npm_publish(lint, id, &words);
    }
    if normalised.contains("gh release") {
        channel = check_gh_release(lint, id, &normalised, &words).or(channel);
    }
    if normalised.contains("gh api") {
        check_gh_api(lint, id, &words);
    }
    if words
        .iter()
        .any(|word| *word == "curl" || word.ends_with("(curl"))
    {
        check_curl(lint, id, &words);
    }
    channel
}

fn check_npm_publish(lint: &mut Lint<'_>, id: &str, words: &[&str]) -> Option<Channel> {
    if id != PUBLISH_JOB {
        lint.report(&format!(
            "job `{id}` runs `npm publish`; only the `publish` job publishes"
        ));
    }
    if !words.contains(&"--provenance") {
        lint.report(&format!(
            "job `{id}` runs `npm publish` without `--provenance` (ADR 0023 decision C)"
        ));
    }
    if words.iter().any(|word| word.starts_with("--tag=")) {
        lint.report(&format!(
            "job `{id}` runs `npm publish` with `--tag=...`; write the dist-tag as `--tag next` \
             or `--tag latest` so the lint can read it"
        ));
    }
    let tag = words
        .iter()
        .position(|word| *word == "--tag")
        .and_then(|index| words.get(index + 1))
        .copied();
    match tag {
        Some("next") => Some(Channel::PreRelease),
        Some("latest") => Some(Channel::Stable),
        _ => {
            lint.report(&format!(
                "job `{id}` runs `npm publish` without an explicit `--tag next` or `--tag \
                 latest`: the dist-tag is never npm's default and never a variable (ADR 0024 \
                 decision A)"
            ));
            None
        }
    }
}

fn check_gh_release(
    lint: &mut Lint<'_>,
    id: &str,
    normalised: &str,
    words: &[&str],
) -> Option<Channel> {
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
            "job `{id}` runs `gh release {subcommand}`; the workflow only creates, publishes and \
             views its own release ({RELEASE_SUBCOMMANDS:?})"
        ));
    }
    if normalised.contains("gh release create") {
        if !["--verify-tag", "--draft"]
            .iter()
            .all(|flag| words.contains(flag))
        {
            lint.report(&format!(
                "job `{id}` creates a release without `--verify-tag` and `--draft`: the tag must \
                 exist, and a release is a draft until it is published"
            ));
        }
        if words.contains(&"--prerelease") {
            if !words.contains(&"--latest=false") {
                lint.report(&format!(
                    "job `{id}` creates a pre-release without `--latest=false`: a pre-release is \
                     never marked latest"
                ));
            }
            return Some(Channel::PreRelease);
        }
        if words.iter().any(|word| word.starts_with("--latest")) {
            lint.report(&format!(
                "job `{id}` marks a release latest when it creates the draft; a stable release \
                 is marked latest once, by the edit that publishes it"
            ));
        }
        return Some(Channel::Stable);
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
            } else if word.starts_with('-') && !RELEASE_EDIT_FLAGS.contains(word) {
                flags_ok = false;
            }
        }
        if !flags_ok || !words.contains(&"--draft=false") {
            lint.report(&format!(
                "job `{id}` edits a release with flags other than `--repo`, `--draft=false` and \
                 `--latest`, or without `--draft=false`: it only publishes its own draft"
            ));
        }
        return Some(if words.contains(&"--latest") {
            Channel::Stable
        } else {
            Channel::PreRelease
        });
    }
    None
}

/// `gh api` only reads GitHub's latest release, to confirm a stable release
/// was marked latest: no method, field or input, no other path.
fn check_gh_api(lint: &mut Lint<'_>, id: &str, words: &[&str]) {
    if id != PUBLISH_JOB {
        lint.report(&format!(
            "job `{id}` runs `gh api`; only the `publish` job reads the latest release"
        ));
    }
    let Some(start) = words.iter().position(|word| *word == "api") else {
        return;
    };
    let mut paths = 0_usize;
    let mut skip_value = false;
    let mut flags_ok = true;
    for word in words.iter().skip(start + 1) {
        if matches!(*word, "||" | "&&" | ";" | "|") {
            break;
        }
        if skip_value {
            skip_value = false;
        } else if *word == "--jq" {
            skip_value = true;
        } else if word.starts_with('-') {
            flags_ok = false;
        } else if *word == LATEST_RELEASE_PATH {
            paths += 1;
        } else {
            flags_ok = false;
        }
    }
    if !flags_ok || paths != 1 {
        lint.report(&format!(
            "job `{id}` runs `gh api` other than `gh api {LATEST_RELEASE_PATH} --jq <filter>`: \
             the workflow only reads the latest release, never writes through the API"
        ));
    }
}

/// A `curl` in the release workflow is a read-only GET of npm's public
/// registry: only the reviewed flags, and no URL but the registry's.
fn check_curl(lint: &mut Lint<'_>, id: &str, words: &[&str]) {
    let Some(start) = words
        .iter()
        .position(|word| *word == "curl" || word.ends_with("(curl"))
    else {
        return;
    };
    let mut registry_urls = 0_usize;
    let mut other = Vec::new();
    for word in words.iter().skip(start + 1) {
        if matches!(*word, "||" | "&&" | ";" | "|") {
            break;
        }
        let bare = word.trim_matches(|character| matches!(character, '"' | '\''));
        if word.starts_with('-') {
            if !CURL_FLAGS.contains(word) {
                other.push((*word).to_owned());
            }
        } else if bare.contains("://") {
            if bare.starts_with(REGISTRY_URL) {
                registry_urls += 1;
            } else {
                other.push((*word).to_owned());
            }
        }
    }
    if !other.is_empty() || registry_urls != 1 {
        lint.report(&format!(
            "job `{id}` runs `curl` other than a plain GET of `{REGISTRY_URL}` with only \
             {CURL_FLAGS:?}{}",
            if other.is_empty() {
                String::new()
            } else {
                format!(" (found {})", other.join(", "))
            }
        ));
    }
}

/// A step that publishes or releases serves one channel, runs only when the
/// plan says so, and (for `npm publish`) checks the version's shape itself.
fn check_step_channel(
    lint: &mut Lint<'_>,
    id: &str,
    step: &Hash,
    script: &str,
    implied: &[Channel],
) {
    let name = get(step, "name")
        .and_then(Yaml::as_str)
        .unwrap_or("an unnamed step");
    let [channel] = implied else {
        if implied.len() > 1 {
            lint.report(&format!(
                "step `{name}` of job `{id}` publishes or releases for both channels; each step \
                 serves exactly one (ADR 0024 decision A)"
            ));
        }
        return;
    };
    let condition = get(step, "if").and_then(Yaml::as_str).unwrap_or_default();
    if !condition.contains(channel.condition()) || condition.contains(channel.other().condition()) {
        lint.report(&format!(
            "step `{name}` of job `{id}` serves the {} channel but does not run only under \
             `{}`",
            channel.name(),
            channel.condition()
        ));
    }
    for escape in CONDITION_ESCAPES {
        if condition.contains(escape) {
            lint.report(&format!(
                "step `{name}` of job `{id}` has `{escape}` in its condition, which could run it \
                 for the wrong channel"
            ));
        }
    }
    let publishes = logical_lines(script).iter().any(|line| {
        line.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .contains("npm publish")
    });
    if publishes
        && !script
            .lines()
            .any(|line| line.trim() == channel.shape_check())
    {
        lint.report(&format!(
            "step `{name}` of job `{id}` publishes for the {} channel without checking the \
             version's own shape with `{}`",
            channel.name(),
            channel.shape_check()
        ));
    }
}

/// The line of a stable `npm publish` step that requires every package's
/// `latest` (read into `tags-before` just before) to be a version at or below
/// the one being published, so `latest` can only move forward.
const STABLE_FORWARD_CHECK: &str =
    r#"test "$(printf '%s\n%s\n' "${latest}" "${VERSION}" | sort -V | tail -n 1)" = "${VERSION}""#;

/// What a step that records the dist-tags before publishing writes.
const TAGS_RECORD: &str = r#">> "${RUNNER_TEMP}/tags-before""#;

/// What each channel's verification step after the publish contains: it reads
/// both dist-tags back and waits until they settle.
const VERIFICATION_MARKS: [&str; 3] = [
    "dist-tags.latest",
    "dist-tags.next",
    r#"test "${settled}" = yes"#,
];

/// What a stable release step ends with: GitHub's own latest release must be
/// this tag.
const LATEST_RELEASE_CHECK: &str = r#"test "${latest_release}" = "${TAG}""#;

/// The publish job's dist-tag bookkeeping (P14 PR 8): the tags are recorded
/// before the first publish, a stable publish checks `latest` only moves
/// forward, each channel's tags are verified after, and a stable release is
/// confirmed to be GitHub's latest.
fn check_channel_steps(lint: &mut Lint<'_>, job: &Hash) {
    let mut recorded = false;
    let mut published = false;
    for step in steps(job) {
        let Some(Yaml::String(script)) = get(step, "run") else {
            continue;
        };
        let publish_at = script.lines().position(|line| {
            line.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .contains("npm publish")
        });
        let Some(publish_at) = publish_at else {
            if !published && script.contains(TAGS_RECORD) && script.contains("dist-tags.latest") {
                recorded = true;
            }
            continue;
        };
        published = true;
        let condition = get(step, "if").and_then(Yaml::as_str).unwrap_or_default();
        if condition.contains(Channel::Stable.condition()) {
            let check_at = script
                .lines()
                .position(|line| line.trim() == STABLE_FORWARD_CHECK);
            if check_at.is_none_or(|check_at| check_at >= publish_at) {
                lint.report(&format!(
                    "the stable `npm publish` step does not first require `latest` to be at or \
                     below the version being published (`{STABLE_FORWARD_CHECK}`): `latest` \
                     must only move forward"
                ));
            }
        }
    }
    if !recorded {
        lint.report(&format!(
            "job `publish` does not record the dist-tags (`{TAGS_RECORD}`) in a step before it \
             publishes, so it cannot check afterwards that only one tag moved"
        ));
    }
    for channel in [Channel::PreRelease, Channel::Stable] {
        let verified = steps(job).into_iter().any(|step| {
            get(step, "if")
                .and_then(Yaml::as_str)
                .is_some_and(|condition| condition.contains(channel.condition()))
                && matches!(
                    get(step, "run"),
                    Some(Yaml::String(script)) if VERIFICATION_MARKS.iter().all(|mark| script.contains(mark))
                )
        });
        if !verified {
            lint.report(&format!(
                "job `publish` has no step under `{}` that reads both dist-tags back after \
                 publishing and waits for them to settle ({VERIFICATION_MARKS:?})",
                channel.condition()
            ));
        }
    }
    let confirms_latest_release = steps(job).into_iter().any(|step| {
        get(step, "if")
            .and_then(Yaml::as_str)
            .is_some_and(|condition| condition.contains(Channel::Stable.condition()))
            && matches!(
                get(step, "run"),
                Some(Yaml::String(script)) if script.contains("gh release create")
                    && script.lines().any(|line| line.trim() == LATEST_RELEASE_CHECK)
            )
    });
    if !confirms_latest_release {
        lint.report(&format!(
            "the stable release step does not confirm that GitHub's latest release is the new \
             tag (`{LATEST_RELEASE_CHECK}`)"
        ));
    }
}

/// The arguments every `vsift-release publish-plan` of the `plan` job carries (P14 PR 8):
/// the saved registry metadata, the evidence check's answer, and the run and date a delta
/// record names.
const PUBLISH_PLAN_ARGUMENTS: [&str; 4] = ["--registry", "--evidence", "--run-id", "--date"];

/// The `plan` job's inputs for the stable path (P14 PR 8): it exports the
/// channel, fetches the full history so the candidate's tag is there, and
/// hands `publish-plan` the registry files it saved.
fn check_plan_job(lint: &mut Lint<'_>, plan: &Hash) {
    let exports_channel = matches!(
        get(plan, "outputs"),
        Some(Yaml::Hash(outputs)) if get(outputs, "channel").is_some()
    );
    if !exports_channel {
        lint.report(
            "job `plan` must export `channel`, which the publish steps' conditions compare \
             (ADR 0024 decision A)",
        );
    }
    let full_history = steps(plan).into_iter().any(|step| {
        get(step, "uses")
            .and_then(Yaml::as_str)
            .is_some_and(|uses| uses.starts_with("actions/checkout@"))
            && matches!(
                get(step, "with"),
                Some(Yaml::Hash(with)) if get(with, "fetch-depth") == Some(&Yaml::Integer(0))
            )
    });
    if !full_history {
        lint.report(
            "job `plan` must check out with `fetch-depth: 0`: a stable plan compares the commit \
             with its accepted release candidate, whose tag a shallow clone does not have",
        );
    }
    let mut plans = 0_usize;
    let mut complete_plans = 0_usize;
    let mut finds_candidate = false;
    let mut checks_evidence = false;
    for step in steps(plan) {
        if let Some(Yaml::String(script)) = get(step, "run") {
            for command in logical_lines(script) {
                let words: Vec<&str> = command.split_whitespace().collect();
                if words.contains(&"vsift-release") && words.contains(&"publish-plan") {
                    plans += 1;
                    if PUBLISH_PLAN_ARGUMENTS
                        .iter()
                        .all(|argument| words.contains(argument))
                    {
                        complete_plans += 1;
                    }
                }
                if words.contains(&"vsift-release")
                    && words.contains(&"candidate-delta")
                    && words.contains(&"--github-output")
                {
                    finds_candidate = true;
                }
                if words.contains(&"vsift-governance")
                    && words.contains(&"release-evidence")
                    && words.contains(&"--complete-for")
                    && words.contains(&"--commit")
                {
                    checks_evidence = true;
                }
            }
        }
    }
    if plans == 0 || complete_plans != plans {
        lint.report(&format!(
            "job `plan` must run `vsift-release publish-plan` with {PUBLISH_PLAN_ARGUMENTS:?}, so \
             a stable plan states and checks the registry and the evidence ledger before `latest` \
             may move, and names the run in the delta record"
        ));
    }
    if !finds_candidate {
        lint.report(
            "job `plan` must run `vsift-release candidate-delta --github-output`, which names the \
             accepted release candidate the evidence check is run for",
        );
    }
    if !checks_evidence {
        lint.report(
            "job `plan` must run `vsift-governance release-evidence --complete-for <candidate> \
             --commit <commit>`: a stable version is published only when the evidence ledger is \
             complete for its accepted candidate (RQ-20)",
        );
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
                "a pre-release publish under latest",
                mutate(
                    &text,
                    "--tag next --access public",
                    "--tag latest --access public",
                    1,
                )?,
                "serves the stable channel but does not run only under \
                 `needs.plan.outputs.channel == 'stable'`",
            ),
            (
                "publish without provenance",
                mutate(
                    &text,
                    " --provenance --ignore-scripts",
                    " --ignore-scripts",
                    1,
                )?,
                "without `--provenance`",
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
                "moves a dist-tag",
            ),
            (
                "a pre-release marked latest",
                mutate(&text, "--prerelease --latest=false", "--prerelease", 1)?,
                "creates a pre-release without `--latest=false`",
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

    /// The channel rules of P14 PR 8: each way a pre-release could reach
    /// `latest`, a stable version `next`, `latest` move backwards or by
    /// another path, or the stable path lose one of its inputs, is a
    /// deliberately broken copy of the real workflow that the lint must name.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one auditable list: each channel rule's violation of the real workflow and its finding"
    )]
    fn each_channel_rule_refuses_its_violation() -> Result<(), Box<dyn Error>> {
        let text = release_text()?;
        let stable_if = "        if: needs.plan.outputs.channel == 'stable'\n";
        let prerelease_if = "        if: needs.plan.outputs.channel == 'prerelease'\n";
        let after_mkdir = "          mkdir publish-plan\n";
        let stable_shape = "          [[ \"${VERSION}\" =~ ^[0-9]+\\.[0-9]+\\.[0-9]+$ ]]\n";
        let forward_check = "            test \"$(printf '%s\\n%s\\n' \"${latest}\" \"${VERSION}\" | sort -V | tail -n 1)\" = \"${VERSION}\"\n";
        let after_plan_step = |extra: &str| {
            mutate(
                &text,
                after_mkdir,
                &format!("{after_mkdir}          {extra}\n"),
                1,
            )
        };
        let cases: Vec<(&str, String, &str)> = vec![
            (
                "a stable publish under next",
                mutate(
                    &text,
                    "--tag latest --access public",
                    "--tag next --access public",
                    1,
                )?,
                "serves the prerelease channel but does not run only under \
                 `needs.plan.outputs.channel == 'prerelease'`",
            ),
            (
                "a stable publish step without its channel condition",
                mutate(&text, stable_if, "", 1)?,
                "serves the stable channel but does not run only under",
            ),
            (
                "a stable publish step gated on the pre-release channel",
                mutate(&text, stable_if, prerelease_if, 1)?,
                "serves the stable channel but does not run only under",
            ),
            (
                "a pre-release publish step that also runs for stable",
                mutate(
                    &text,
                    prerelease_if,
                    "        if: needs.plan.outputs.channel == 'prerelease' || needs.plan.outputs.channel == 'stable'\n",
                    1,
                )?,
                "has `||` in its condition",
            ),
            (
                "a bypassable channel condition",
                mutate(
                    &text,
                    stable_if,
                    "        if: needs.plan.outputs.channel == 'stable' || always()\n",
                    1,
                )?,
                "has `||` in its condition",
            ),
            (
                "a stable publish without its shape check",
                mutate(&text, stable_shape, "", 1)?,
                "without checking the version's own shape",
            ),
            (
                "a pre-release shape check that also accepts a stable version",
                mutate(
                    &text,
                    "^[0-9]+\\.[0-9]+\\.[0-9]+-[0-9A-Za-z.-]+$ ]]",
                    "^[0-9]+\\.[0-9]+\\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]",
                    1,
                )?,
                "without checking the version's own shape",
            ),
            (
                "a publish tag read from a variable",
                mutate(
                    &text,
                    "--tag latest --access public",
                    "--tag \"${DIST_TAG}\" --access public",
                    1,
                )?,
                "without an explicit `--tag next` or `--tag latest`",
            ),
            (
                "a publish tag in the --tag=latest form",
                mutate(
                    &text,
                    "--tag latest --access public",
                    "--tag=latest --access public",
                    1,
                )?,
                "with `--tag=...`",
            ),
            (
                "a stable publish that does not check that latest only moves forward",
                mutate(&text, forward_check, "", 1)?,
                "does not first require `latest` to be at or below",
            ),
            (
                "no record of the dist-tags before publishing",
                mutate(
                    &text,
                    " >> \"${RUNNER_TEMP}/tags-before\"",
                    " > /dev/null",
                    1,
                )?,
                "does not record the dist-tags",
            ),
            (
                "no verification of a stable publish",
                mutate(&text, "          test \"${settled}\" = yes\n", "", 2)?,
                "has no step under `needs.plan.outputs.channel == 'stable'` that reads both \
                 dist-tags back",
            ),
            (
                "no verification of a pre-release publish",
                mutate(&text, "          test \"${settled}\" = yes\n", "", 1)?,
                "has no step under `needs.plan.outputs.channel == 'prerelease'` that reads both \
                 dist-tags back",
            ),
            (
                "a stable release that does not confirm it is GitHub's latest",
                mutate(
                    &text,
                    "          test \"${latest_release}\" = \"${TAG}\"\n",
                    "",
                    1,
                )?,
                "does not confirm that GitHub's latest release",
            ),
            (
                "a stable release marked latest when it is created",
                mutate(
                    &text,
                    "--verify-tag --draft \\\n",
                    "--verify-tag --draft --latest \\\n",
                    1,
                )?,
                "marks a release latest when it creates the draft",
            ),
            (
                "a stable release created as a pre-release",
                mutate(
                    &text,
                    "--verify-tag --draft \\\n",
                    "--verify-tag --draft --prerelease --latest=false \\\n",
                    1,
                )?,
                "for both channels",
            ),
            (
                "a release edited with other flags",
                mutate(
                    &text,
                    "--draft=false --latest\n",
                    "--draft=false --latest --notes changed\n",
                    1,
                )?,
                "edits a release with flags other than",
            ),
            (
                "an upload to the release",
                mutate(
                    &text,
                    "          gh release view \"${TAG}\" --repo smormah/vsift\n",
                    "          gh release view \"${TAG}\" --repo smormah/vsift\n          gh release upload \"${TAG}\" extra.txt\n",
                    1,
                )?,
                "runs `gh release upload`",
            ),
            (
                "a mutating API call",
                mutate(
                    &text,
                    "gh api repos/smormah/vsift/releases/latest",
                    "gh api -X DELETE repos/smormah/vsift/releases/latest",
                    1,
                )?,
                "runs `gh api` other than",
            ),
            (
                "an API read of something else",
                mutate(
                    &text,
                    "gh api repos/smormah/vsift/releases/latest",
                    "gh api repos/smormah/vsift/releases",
                    1,
                )?,
                "runs `gh api` other than",
            ),
            (
                "a curl with a header",
                mutate(
                    &text,
                    "curl --silent --show-error",
                    "curl -H 'Authorization: x' --silent --show-error",
                    1,
                )?,
                "runs `curl` other than a plain GET",
            ),
            (
                "a curl that follows redirects",
                mutate(
                    &text,
                    "curl --silent --show-error",
                    "curl --silent --location --show-error",
                    1,
                )?,
                "runs `curl` other than a plain GET",
            ),
            (
                "a curl to another host",
                mutate(
                    &text,
                    "https://registry.npmjs.org/${package//",
                    "https://example.com/${package//",
                    1,
                )?,
                "runs `curl` other than a plain GET",
            ),
            (
                "a download in a privileged job",
                mutate(
                    &text,
                    "      - name: Fetch the plan\n",
                    "      - name: Download\n        run: curl --silent https://registry.npmjs.org/x\n\n      - name: Fetch the plan\n",
                    1,
                )?,
                "job `attest` runs `curl`",
            ),
            (
                "a plan job with a shallow checkout",
                mutate(&text, "          fetch-depth: 0\n", "", 1)?,
                "must check out with `fetch-depth: 0`",
            ),
            (
                "a plan job that does not export the channel",
                mutate(
                    &text,
                    "      channel: ${{ steps.plan.outputs.channel }}\n",
                    "",
                    1,
                )?,
                "job `plan` must export `channel`",
            ),
            (
                "a plan that is not given the registry",
                mutate(
                    &text,
                    "--registry \"${RUNNER_TEMP}/registry\" --evidence",
                    "--evidence",
                    1,
                )?,
                "must run `vsift-release publish-plan` with",
            ),
            (
                "a plan that ignores the evidence ledger's answer",
                mutate(
                    &text,
                    " --evidence \"${RUNNER_TEMP}/evidence\" \\\n",
                    " \\\n",
                    1,
                )?,
                "must run `vsift-release publish-plan` with",
            ),
            (
                "a plan that does not name the run in its delta record",
                mutate(
                    &text,
                    "            --run-id \"${RUN_ID}\" --date \"$(date -u +%F)\" \\\n",
                    "",
                    1,
                )?,
                "must run `vsift-release publish-plan` with",
            ),
            (
                "a plan job that does not find the accepted candidate",
                mutate(
                    &text,
                    "candidate-delta --github-output --stable-commit",
                    "candidate-delta --stable-commit",
                    1,
                )?,
                "must run `vsift-release candidate-delta --github-output`",
            ),
            (
                "a plan job that does not check the evidence ledger",
                mutate(
                    &text,
                    "release-evidence --complete-for \"${CANDIDATE_VERSION}\" --commit \"${CANDIDATE_COMMIT}\"",
                    "release-evidence",
                    1,
                )?,
                "must run `vsift-governance release-evidence --complete-for",
            ),
            (
                "a dist-tag move with extra spaces",
                after_plan_step("npm   dist-tag  add vsift-cli@0.2.0 latest")?,
                "moves a dist-tag",
            ),
            (
                "a dist-tag move with a quoted subcommand",
                after_plan_step("npm \"dist-tag\" add vsift-cli@0.2.0 latest")?,
                "moves a dist-tag",
            ),
            (
                "a dist-tag move with flags before the subcommand",
                after_plan_step(
                    "npm --registry https://registry.npmjs.org/ dist-tag add vsift-cli@0.2.0 latest",
                )?,
                "moves a dist-tag",
            ),
            (
                "a dist-tag move through pnpm",
                after_plan_step("pnpm dist-tag add vsift-cli@0.2.0 latest")?,
                "moves a dist-tag",
            ),
            (
                "a dist-tag move through Yarn",
                after_plan_step("yarn npm tag add vsift-cli@0.2.0 latest")?,
                "moves a dist-tag",
            ),
            (
                "a dist-tag move through the registry API",
                after_plan_step(
                    "echo PUT https://registry.npmjs.org/-/package/vsift-cli/dist-tags/latest",
                )?,
                "moves a dist-tag",
            ),
        ];
        for (case, mutated, expected) in cases {
            let messages = findings(&mutated);
            assert!(
                messages.iter().any(|message| message.contains(expected)),
                "{case}: expected a finding containing {expected:?}, got {messages:#?}"
            );
        }
        // Reading the dist-tags is not moving them.
        let read = after_plan_step("npm view vsift-cli dist-tags --json")?;
        assert_eq!(findings(&read), Vec::<String>::new());
        let read_one = after_plan_step("npm view \"@vsift/win32-x64\" dist-tags.latest")?;
        assert_eq!(findings(&read_one), Vec::<String>::new());
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
