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
//! 10. **The guard** (#337, governance rule 15, ADR 0024 note of 2026-10-17).
//!     A job `guard` runs first and is the only job without `needs`, so every
//!     other job waits for it and a refused dispatch builds nothing. It cannot
//!     be skipped or ignored (no `if`, no `continue-on-error`), and no job that
//!     waits for it may run anyway (`always()`, `failure()`, `cancelled()` or
//!     `continue-on-error` on the job). It refuses a tag that no longer names
//!     the run's commit. It reads only
//!     (`actions: read`, `contents: read`), uses no action but the checkout,
//!     builds and installs nothing, and is handed the event, ref, repository,
//!     `dry_run` input and commit through `env`. Its script reads the paths of
//!     [`GUARD_READS`]; `gh api` there reads only [`GUARD_API_PREFIXES`].
//!     What the script decides is held by the shell check
//!     (`tools/vsift-release/tests/publish-steps.sh`).
//! 11. **The read-back window** (#403). The `publish` job takes the dist-tags
//!     of before the publish from the plan job's `tags-before` output, never
//!     from a read of its own (a re-run would read the moved or a stale tag),
//!     and the plan exports them. Each channel's step that reads them back
//!     polls in a loop bounded by rounds and by a deadline
//!     (`deadline=$((SECONDS + D))`, `for attempt in $(seq 1 N)`, `sleep S`),
//!     cuts off every `npm view` with `timeout`, waits at least
//!     [`MINIMUM_READBACK_SECONDS`] for a quick
//!     registry and, with the reads' cut-offs counted, leaves
//!     [`READBACK_MARGIN_MINUTES`] of the `publish` job's `timeout-minutes` for
//!     the rest of the job; it keeps waiting while the tag that moves has not
//!     named the version, fails at once, inside the loop, when that tag reads a
//!     value that is neither the version nor its value before the publish, or
//!     the tag that stays reads anything but its value before; and it fails
//!     when the loop ends unsettled.
//! 12. **The re-check after the approval** (the maintainer's decision of
//!     2026-10-17). The `publish` job has a step, before every step that
//!     publishes or releases and with no `if` or `continue-on-error`, that is
//!     given the guard's `env` and the planned `VERSION`, reads what the guard
//!     reads, refuses unless the ref is the planned version's tag and the tag
//!     still names the commit the run built, and carries the guard's four
//!     functions ([`GUARD_FUNCTIONS`]) character for character, so a long wait
//!     for the approval cannot publish what no longer qualifies.

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

    /// The checks the read-back of this channel makes on every answer, inside
    /// its loop, each of which must fail the step at once (#403): the tag that
    /// moves reads neither the version nor its value before the publish, or the
    /// tag that stays reads anything but its value before. An empty answer is
    /// no answer and is waited for; only these are wrong ones.
    fn wrong_answer_checks(self) -> [String; 2] {
        let (moved, stayed) = match self {
            Self::PreRelease => ("next", "latest"),
            Self::Stable => ("latest", "next"),
        };
        [
            format!(
                r#"if [ -n "${{{moved}_now}}" ] && [ "${{{moved}_now}}" != "${{VERSION}}" ] && [ "${{{moved}_now}}" != "${{{moved}}}" ]; then"#
            ),
            format!(
                r#"if [ -n "${{{stayed}_now}}" ] && [ "${{{stayed}_now}}" != "${{{stayed}}}" ]; then"#
            ),
        ]
    }

    /// The condition under which a package of this channel's read-back is
    /// still waiting: the tag that moves does not name the version yet, or
    /// the tag that stays gave no answer although it was set. It must be
    /// followed by `settled=no`, so a tag that never moved never settles.
    fn settle_condition(self) -> String {
        let (moved, stayed) = match self {
            Self::PreRelease => ("next", "latest"),
            Self::Stable => ("latest", "next"),
        };
        format!(
            r#"if [ "${{{moved}_now}}" != "${{VERSION}}" ] || {{ [ -z "${{{stayed}_now}}" ] && [ "${{{stayed}}}" != none ]; }}; then"#
        )
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

/// The first job, which refuses a wrong ref or a candidate without a passed
/// nightly before anything is built (#337, governance rule 15).
const GUARD_JOB: &str = "guard";

/// The guard's whole token: it reads workflow runs and the repository, and
/// writes nothing.
const GUARD_PERMISSIONS: [(&str, &str); 2] = [("actions", "read"), ("contents", "read")];

/// The run's facts the guard decides on, each passed through `env` exactly so.
const GUARD_ENV: [(&str, &str); 5] = [
    ("EVENT_NAME", "${{ github.event_name }}"),
    ("GIT_REF", "${{ github.ref }}"),
    ("REPOSITORY", "${{ github.repository }}"),
    ("DRY_RUN_INPUT", "${{ github.event.inputs.dry_run }}"),
    ("SOURCE_COMMIT", "${{ github.sha }}"),
];

/// What the guard's script must read: every Nightly run whose head is exactly
/// the commit (never a later or an earlier one, and in any status, so a run
/// still in progress is seen), each run's jobs, the release branch's head, the
/// comparison of the commit with that head's own commit (a tag named like the
/// branch cannot stand in), the commit the tag names now, and the rules that
/// apply to the release branch.
const GUARD_READS: [&str; 6] = [
    "repos/smormah/vsift/actions/workflows/nightly.yml/runs?head_sha=${SOURCE_COMMIT}&per_page=100",
    "repos/smormah/vsift/actions/runs/${id}/jobs",
    "repos/smormah/vsift/git/ref/heads/${release_branch}",
    "repos/smormah/vsift/compare/${SOURCE_COMMIT}...${head}",
    "repos/smormah/vsift/git/ref/tags/${tag}",
    "repos/smormah/vsift/rules/branches/${release_branch}",
];

/// The paths `gh api` may read in the guard and the re-check: GETs of this
/// repository's Nightly runs, a run's jobs, a comparison, a branch head, a tag
/// (and an annotated tag's object) and a branch's rules, nothing else.
const GUARD_API_PREFIXES: [&str; 7] = [
    "repos/smormah/vsift/actions/workflows/nightly.yml/runs?",
    "repos/smormah/vsift/actions/runs/",
    "repos/smormah/vsift/compare/",
    "repos/smormah/vsift/git/ref/heads/",
    "repos/smormah/vsift/git/ref/tags/",
    "repos/smormah/vsift/git/tags/",
    "repos/smormah/vsift/rules/branches/",
];

/// The functions the `publish` job's re-check carries as the guard has them,
/// character for character, so the two cannot drift apart.
const GUARD_FUNCTIONS: [&str; 4] = [
    "nightly_passed",
    "on_release_branch",
    "tag_commit",
    "release_branch_protected",
];

/// The guard's refusal of a tag that no longer names the commit the run built
/// (a re-run of an old run after the tag moved), which the re-check makes too.
const TAG_REFUSAL: &str = r#"[ "${tagged}" = "${SOURCE_COMMIT}" ] || refuse"#;

/// Conditions that let a job run although a job it needs failed: no job that
/// waits for the guard may carry one, or it could run after a refusal.
const RUN_ANYWAY: [&str; 3] = ["always()", "failure()", "cancelled()"];

/// The planned version, which the re-check is given besides the guard's `env`.
const RECHECK_VERSION: (&str, &str) = ("VERSION", "${{ needs.plan.outputs.version }}");

/// The re-check's refusals that the guard makes in its own way: the ref must
/// be the planned version's tag, and the tag must still name this commit.
const RECHECK_REFUSALS: [&str; 2] = [
    r#"[ "${GIT_REF}" = "refs/tags/${tag}" ] || refuse"#,
    TAG_REFUSAL,
];

/// The shortest read-back window of the dist-tags after a publish (#403): npm
/// showed one package of `0.2.1` more than five minutes late, and it read right
/// when the job was re-run about fourteen minutes after the publish; fifteen
/// minutes is the floor.
const MINIMUM_READBACK_SECONDS: u64 = 15 * 60;

/// The part of the `publish` job's `timeout-minutes` the read-back must leave
/// for publishing, the GitHub release and the checks around them.
const READBACK_MARGIN_MINUTES: u64 = 15;

/// The packages each round of the read-back reads: the four of every release.
const READBACK_PACKAGES: u64 = 4;

/// The line that ends the read-back's loop once the tags settle or its deadline
/// has passed, whatever the number of rounds left (#403).
const READBACK_DEADLINE_BREAK: &str =
    r#"if [ "${settled}" = yes ] || [ "${SECONDS}" -ge "${deadline}" ]; then"#;

/// The plan job's output the publish job takes its "before" values from, so
/// a re-run of failed jobs after a partial publish compares with the tags as
/// they were before it (#403's recovery), never with a stale read.
const PLAN_TAGS_OUTPUT: &str = "${{ needs.plan.outputs.tags-before }}";

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
    check_guard(lint, jobs);
    check_recheck(lint, jobs);
    if let Some(Yaml::Hash(publish)) = get(jobs, PUBLISH_JOB) {
        check_privileged_job(
            lint,
            PUBLISH_JOB,
            publish,
            &[PLAN_JOB, ATTEST_JOB, "npm-qualify"],
            &["actions/download-artifact", "actions/setup-node"],
        );
        check_channel_steps(lint, publish);
        check_readback(lint, publish);
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

/// `gh api` only reads: in `publish`, GitHub's latest release, to confirm a
/// stable release was marked latest; in `guard`, the Nightly runs, a run's
/// jobs and a comparison of this repository. No method, field or input (so
/// every call is a GET), no other path, and no other job.
fn check_gh_api(lint: &mut Lint<'_>, id: &str, words: &[&str]) {
    let allowed = |path: &str| match id {
        PUBLISH_JOB => {
            path == LATEST_RELEASE_PATH
                || GUARD_API_PREFIXES
                    .iter()
                    .any(|prefix| path.starts_with(prefix))
        }
        GUARD_JOB => GUARD_API_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix)),
        _ => false,
    };
    if id != PUBLISH_JOB && id != GUARD_JOB {
        lint.report(&format!(
            "job `{id}` runs `gh api`; only the `publish` job (the latest release and its \
             re-check) and the `guard` job (the Nightly runs and the release branch) read through \
             it"
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
        let bare = word.trim_matches(|character| matches!(character, '"' | '\''));
        if skip_value {
            skip_value = false;
        } else if *word == "--jq" {
            skip_value = true;
        } else if word.starts_with('-') {
            flags_ok = false;
        } else if allowed(bare) {
            paths += 1;
        } else {
            flags_ok = false;
        }
    }
    if !flags_ok || paths != 1 {
        lint.report(&format!(
            "job `{id}` runs `gh api` other than a plain read (`gh api <path> --jq <filter>`) of \
             {GUARD_API_PREFIXES:?} in `guard` and `publish`, or of `{LATEST_RELEASE_PATH}` in \
             `publish`: the workflow never writes through the API"
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

/// The lines of a stable `npm publish` step that read every package's
/// `latest` again at publish time and refuse unless it is still what the plan
/// saw or already this version: two stable runs waiting for approval could
/// otherwise both pass on their plans' values, and the lower one, published
/// second, would move `latest` backwards.
const STABLE_FRESH_LATEST: [&str; 2] = [
    r#"latest_now="$(timeout 20 npm view "${package}" dist-tags.latest --prefer-online --fetch-retries=1 2>/dev/null || true)""#,
    r#"if [ "${latest_now}" != "${latest}" ] && [ "${latest_now}" != "${VERSION}" ]; then"#,
];

/// What a step that records the dist-tags before publishing writes.
const TAGS_RECORD: &str = r#">> "${RUNNER_TEMP}/tags-before""#;

/// What each channel's verification step after the publish contains: it reads
/// both dist-tags back and waits until they settle.
const VERIFICATION_MARKS: [&str; 3] = [
    "dist-tags.latest",
    "dist-tags.next",
    r#"if [ "${settled}" != yes ]; then"#,
];

/// What a stable release step ends with: GitHub's own latest release must be
/// this tag.
const LATEST_RELEASE_CHECK: &str = r#"test "${latest_release}" = "${TAG}""#;

/// The stable `npm publish` step reads every package's `latest` again before
/// publishing and refuses unless it is still the plan's value or this version
/// ([`STABLE_FRESH_LATEST`]); `publish_at` is the line of its `npm publish`.
fn check_fresh_latest(lint: &mut Lint<'_>, script: &str, publish_at: usize) {
    let lines = trimmed_lines(script);
    let [read, refusal] = STABLE_FRESH_LATEST;
    let read_at = lines.iter().position(|line| *line == read);
    let fresh = read_at.is_some_and(|read_at| {
        fails_at(&lines, refusal)
            .is_some_and(|refused_at| read_at < refused_at && refused_at < publish_at)
    });
    if !fresh {
        lint.report(&format!(
            "the stable `npm publish` step does not read `latest` again before publishing \
             and refuse unless it is still the plan's value or this version \
             ({STABLE_FRESH_LATEST:?} followed by `exit 1`): a lower stable version \
             published after a higher one during the approval's wait would move `latest` \
             backwards"
        ));
    }
}

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
            let from_plan = matches!(
                get(step, "env"),
                Some(Yaml::Hash(env)) if env.values().any(|value| value.as_str() == Some(PLAN_TAGS_OUTPUT))
            );
            if !published && script.contains(TAGS_RECORD) {
                if from_plan && !script.contains("npm view") {
                    recorded = true;
                } else {
                    lint.report(&format!(
                        "the `publish` job records the dist-tags before publishing from \
                         something other than the plan's output `{PLAN_TAGS_OUTPUT}`: a re-run of \
                         failed jobs would compare with the tags it reads then, or with a stale \
                         copy of them (#403)"
                    ));
                }
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
            check_fresh_latest(lint, script, publish_at);
        }
    }
    if !recorded {
        lint.report(&format!(
            "job `publish` does not record the dist-tags (`{TAGS_RECORD}`, from \
             `{PLAN_TAGS_OUTPUT}`) in a step before it \
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

/// The guard (rule 10, #337, governance rule 15): the first job and the only
/// one without `needs`, which nothing can skip or ignore, with a read-only
/// token, the checkout as its one action, the run's facts in `env`, the reads
/// it decides on and its refusal of a moved tag; no job that waits for it may
/// run anyway. What it decides is the shell check's to hold.
#[expect(
    clippy::too_many_lines,
    reason = "one rule read top to bottom: the job's place, its token, its steps and its reads"
)]
fn check_guard(lint: &mut Lint<'_>, jobs: &Hash) {
    let Some(Yaml::Hash(guard)) = get(jobs, GUARD_JOB) else {
        lint.report(
            "has no `guard` job; a dispatch on a wrong ref, or a candidate from a commit without \
             a passed nightly, must be refused before anything is built (#337, governance rule \
             15)",
        );
        return;
    };
    for (id, job) in jobs {
        let (Some(id), Yaml::Hash(job)) = (id.as_str(), job) else {
            continue;
        };
        if id == GUARD_JOB {
            if get(job, "needs").is_some() {
                lint.report("job `guard` has `needs`; it runs first, before anything is built");
            }
        } else {
            if needs(job).is_empty() {
                lint.report(&format!(
                    "job `{id}` has no `needs`, so it can start before the guard has accepted \
                     the run; only `guard` starts a run, and every other job needs it or a job \
                     that does"
                ));
            }
            let condition = get(job, "if").and_then(Yaml::as_str).unwrap_or_default();
            for anyway in RUN_ANYWAY {
                if condition.contains(anyway) {
                    lint.report(&format!(
                        "job `{id}` has `{anyway}` in its condition, which would let it run after \
                         the guard refused the run"
                    ));
                }
            }
            if get(job, "continue-on-error").is_some() {
                lint.report(&format!(
                    "job `{id}` has `continue-on-error`; a job that waits for the guard may not \
                     turn a failure into a pass"
                ));
            }
        }
    }
    for key in ["if", "continue-on-error"] {
        if get(guard, key).is_some() {
            lint.report(&format!(
                "job `guard` has `{key}`; nothing may skip the guard or ignore its refusal"
            ));
        }
    }
    let read_only = matches!(
        get(guard, "permissions"),
        Some(Yaml::Hash(permissions))
            if permissions.len() == GUARD_PERMISSIONS.len()
                && GUARD_PERMISSIONS.iter().all(|(scope, level)| {
                    get(permissions, scope).and_then(Yaml::as_str) == Some(*level)
                })
    );
    if !read_only {
        lint.report(&format!(
            "job `guard` must have exactly the permissions {GUARD_PERMISSIONS:?}: it reads the \
             Nightly runs and the release branch and writes nothing"
        ));
    }
    let mut decides = false;
    for step in steps(guard) {
        let name = get(step, "name")
            .and_then(Yaml::as_str)
            .unwrap_or("an unnamed step");
        for key in ["if", "continue-on-error"] {
            if get(step, key).is_some() {
                lint.report(&format!(
                    "step `{name}` of job `guard` has `{key}`; nothing may skip the guard's \
                     checks or ignore their refusal"
                ));
            }
        }
        if let Some(uses) = get(step, "uses").and_then(Yaml::as_str) {
            let action = uses.split('@').next().unwrap_or_default();
            if action != "actions/checkout" {
                lint.report(&format!(
                    "job `guard` uses `{action}`; it may use only `actions/checkout`"
                ));
            }
        }
        if let Some(Yaml::String(script)) = get(step, "run") {
            for forbidden in PRIVILEGED_FORBIDDEN_COMMANDS {
                if script.contains(forbidden) {
                    lint.report(&format!(
                        "job `guard` runs `{forbidden}`; the guard builds and installs nothing, \
                         so it answers in seconds"
                    ));
                }
            }
            let given_the_run = matches!(
                get(step, "env"),
                Some(Yaml::Hash(env)) if GUARD_ENV
                    .iter()
                    .all(|(name, value)| get(env, name).and_then(Yaml::as_str) == Some(*value))
            );
            if given_the_run && GUARD_READS.iter().all(|read| script.contains(read)) {
                decides = true;
                if !script
                    .lines()
                    .any(|line| line.trim().starts_with(TAG_REFUSAL))
                {
                    lint.report(&format!(
                        "job `guard` does not refuse with `{TAG_REFUSAL}`: a release tag must still \
                         name the commit the run built (a re-run of an old run after the tag moved)"
                    ));
                }
            }
        }
    }
    if !decides {
        lint.report(&format!(
            "job `guard` has no step that is given {GUARD_ENV:?} through `env` and reads \
             {GUARD_READS:?}: the guard decides on the run's own ref and commit, the Nightly runs \
             of exactly that commit and the release branch"
        ));
    }
}

/// The lines of the shell function `name` in `script`, from `name() {` to the
/// `}` at the same indentation; `None` when there is no such function.
fn function_lines<'a>(script: &'a str, name: &str) -> Option<Vec<&'a str>> {
    let header = format!("{name}() {{");
    let mut lines = script.lines();
    let first = lines.by_ref().find(|line| line.trim() == header)?;
    let indent = first
        .get(..first.len().saturating_sub(first.trim_start().len()))
        .unwrap_or_default();
    let closing = format!("{indent}}}");
    let mut body = vec![first];
    for line in lines {
        body.push(line);
        if line == closing {
            return Some(body);
        }
    }
    None
}

/// The script of the step of `job` that decides as the guard does: given
/// `env` and reading `reads`, or `None`.
fn deciding_step<'a>(
    job: &'a Hash,
    env: &[(&str, &str)],
    reads: &[&str],
) -> Option<(usize, &'a Hash, &'a str)> {
    steps(job)
        .into_iter()
        .enumerate()
        .find_map(|(index, step)| {
            let Some(Yaml::String(script)) = get(step, "run") else {
                return None;
            };
            let given = matches!(
                get(step, "env"),
                Some(Yaml::Hash(given)) if env
                    .iter()
                    .all(|(name, value)| get(given, name).and_then(Yaml::as_str) == Some(*value))
            );
            (given && reads.iter().all(|read| script.contains(read))).then_some((
                index,
                step,
                script.as_str(),
            ))
        })
}

/// The re-check after the approval (rule 12): the `publish` job asks the
/// guard's questions again, with the guard's own functions, before anything
/// is published.
fn check_recheck(lint: &mut Lint<'_>, jobs: &Hash) {
    let (Some(Yaml::Hash(guard)), Some(Yaml::Hash(publish))) =
        (get(jobs, GUARD_JOB), get(jobs, PUBLISH_JOB))
    else {
        return;
    };
    let Some((_, _, guard_script)) = deciding_step(guard, &GUARD_ENV, &GUARD_READS) else {
        // The guard rule reports a guard that does not decide.
        return;
    };
    let mut env: Vec<(&str, &str)> = GUARD_ENV.to_vec();
    env.push(RECHECK_VERSION);
    let reads: Vec<&str> = GUARD_READS.to_vec();
    let Some((at, step, script)) = deciding_step(publish, &env, &reads) else {
        lint.report(&format!(
            "the `publish` job has no step that re-checks the guard's conditions after the \
             approval: given {env:?} through `env` and reading {reads:?}"
        ));
        return;
    };
    let first_release = steps(publish).into_iter().position(|candidate| {
        matches!(
            get(candidate, "run"),
            Some(Yaml::String(run)) if logical_lines(run).iter().any(|line| {
                let words = line.split_whitespace().collect::<Vec<_>>().join(" ");
                words.contains("npm publish") || words.contains("gh release")
            })
        )
    });
    if first_release.is_some_and(|first| first < at) {
        lint.report(
            "the `publish` job publishes or releases before its re-check of the guard's \
             conditions; the re-check comes first, after the approval",
        );
    }
    for key in ["if", "continue-on-error"] {
        if get(step, key).is_some() {
            lint.report(&format!(
                "the `publish` job's re-check has `{key}`; nothing may skip it or ignore its \
                 refusal"
            ));
        }
    }
    let lines = trimmed_lines(script);
    for refusal in RECHECK_REFUSALS {
        if !lines.iter().any(|line| line.starts_with(refusal)) {
            lint.report(&format!(
                "the `publish` job's re-check does not refuse with `{refusal}`: the ref must be \
                 the planned version's tag, and the tag must still name the commit this run built"
            ));
        }
    }
    for name in GUARD_FUNCTIONS {
        let ours = function_lines(script, name);
        if ours.is_none() || ours != function_lines(guard_script, name) {
            lint.report(&format!(
                "the `publish` job's re-check does not carry the guard's `{name}` function \
                 character for character, so the two could decide differently"
            ));
        }
    }
}

/// The lines of a script, trimmed.
fn trimmed_lines(script: &str) -> Vec<&str> {
    script.lines().map(str::trim).collect()
}

/// The index of `line` in `lines` when it is followed by `exit 1`
/// before its `fi`, so the step fails there; `None` otherwise.
fn fails_at(lines: &[&str], line: &str) -> Option<usize> {
    let at = lines.iter().position(|candidate| *candidate == line)?;
    lines
        .iter()
        .skip(at + 1)
        .take_while(|candidate| **candidate != "fi")
        .any(|candidate| *candidate == "exit 1")
        .then_some(at)
}

/// The read-back of the dist-tags after a publish (rule 11, #403): a loop
/// bounded by rounds and by a deadline, whose every read is cut off, whose
/// longest run (the calls' time included) fits the job's timeout, that waits
/// while a tag has not moved, fails at once on a wrong answer and fails when
/// it ends unsettled.
#[expect(
    clippy::too_many_lines,
    reason = "one rule read top to bottom: the bound, the reads, the settle and the failures"
)]
fn check_readback(lint: &mut Lint<'_>, publish: &Hash) {
    let timeout_minutes = match get(publish, "timeout-minutes") {
        Some(Yaml::Integer(minutes)) => u64::try_from(*minutes).ok(),
        _ => None,
    };
    for channel in [Channel::PreRelease, Channel::Stable] {
        let script = steps(publish).into_iter().find_map(|step| {
            let condition = get(step, "if").and_then(Yaml::as_str).unwrap_or_default();
            match get(step, "run") {
                Some(Yaml::String(script))
                    if condition.contains(channel.condition())
                        && VERIFICATION_MARKS.iter().all(|mark| script.contains(mark)) =>
                {
                    Some(script.as_str())
                }
                _ => None,
            }
        });
        // A missing read-back step is the channel-steps rule's finding.
        let Some(script) = script else {
            continue;
        };
        let lines = trimmed_lines(script);
        let name = channel.name();
        let number_after = |prefix: &str, suffix: &str| {
            lines.iter().enumerate().find_map(|(at, line)| {
                line.strip_prefix(prefix)
                    .and_then(|rest| rest.strip_suffix(suffix))
                    .and_then(|number| number.parse::<u64>().ok())
                    .map(|number| (at, number))
            })
        };
        let rounds = number_after("for attempt in $(seq 1 ", "); do");
        let pause = number_after("sleep ", "");
        let deadline = number_after("deadline=$((SECONDS + ", "))");
        let reads: Vec<&&str> = lines
            .iter()
            .filter(|line| line.contains("npm view \"${package}\""))
            .collect();
        let cut_offs: Vec<Option<u64>> = reads
            .iter()
            .map(|line| {
                line.split_once("\"$(timeout ")
                    .and_then(|(_, rest)| rest.split_once(" npm view "))
                    .and_then(|(seconds, _)| seconds.parse::<u64>().ok())
            })
            .collect();
        if reads.is_empty() || cut_offs.iter().any(Option::is_none) {
            lint.report(&format!(
                "the {name} read-back runs `npm view` without `timeout <seconds>` in front of it: \
                 one read that hangs would run the job into its timeout after the publish (#403)"
            ));
        }
        let per_round = READBACK_PACKAGES.saturating_mul(
            cut_offs
                .iter()
                .flatten()
                .fold(0_u64, |total, seconds| total.saturating_add(*seconds)),
        );
        let bounded_by_deadline = lines.contains(&READBACK_DEADLINE_BREAK);
        match (rounds, pause, deadline) {
            (Some((loop_at, rounds)), Some((sleep_at, pause)), Some((_, deadline)))
                if bounded_by_deadline =>
            {
                // The least it waits for a registry that answers at once, and the
                // most it can take when every read hangs until it is cut off.
                let shortest = rounds.saturating_mul(pause).min(deadline);
                let longest = rounds
                    .saturating_mul(pause.saturating_add(per_round))
                    .min(deadline.saturating_add(pause).saturating_add(per_round));
                if shortest < MINIMUM_READBACK_SECONDS {
                    lint.report(&format!(
                        "the {name} read-back waits {shortest} seconds ({rounds} rounds, {pause} \
                         seconds apart, a deadline of {deadline}), less than \
                         {MINIMUM_READBACK_SECONDS} seconds: npm can show a publish more than \
                         five minutes late (#403)"
                    ));
                }
                let fits = timeout_minutes.is_some_and(|timeout| {
                    longest.div_ceil(60).saturating_add(READBACK_MARGIN_MINUTES) <= timeout
                });
                if !fits {
                    lint.report(&format!(
                        "the {name} read-back can take {} minutes with its reads' cut-offs counted, \
                         which does not leave {READBACK_MARGIN_MINUTES} minutes of the `publish` \
                         job's `timeout-minutes` ({}) for the rest of the job",
                        longest.div_ceil(60),
                        timeout_minutes.map_or_else(|| String::from("not set"), |t| t.to_string())
                    ));
                }
                let inside = |at: usize| loop_at < at && at < sleep_at;
                for check in channel.wrong_answer_checks() {
                    if !fails_at(&lines, &check).is_some_and(inside) {
                        lint.report(&format!(
                            "the {name} read-back does not fail at once, inside its loop, on a \
                             wrong answer (`{check}` followed by `exit 1`): it must never fail \
                             only on time (#403)"
                        ));
                    }
                }
                let settle = channel.settle_condition();
                let waits = lines
                    .iter()
                    .position(|line| *line == settle)
                    .is_some_and(|at| {
                        inside(at)
                            && lines
                                .iter()
                                .skip(at + 1)
                                .take_while(|line| **line != "fi")
                                .any(|line| *line == "settled=no")
                    });
                if !waits {
                    lint.report(&format!(
                        "the {name} read-back does not keep waiting while the tag has not moved \
                         (`{settle}` followed by `settled=no`, inside its loop): a tag that never \
                         moved could settle"
                    ));
                }
                let timed_out = VERIFICATION_MARKS
                    .last()
                    .is_some_and(|mark| fails_at(&lines, mark).is_some_and(|at| sleep_at < at));
                if !timed_out {
                    lint.report(&format!(
                        "the {name} read-back does not fail when its window ends without the tags \
                         settling (`if [ \"${{settled}}\" != yes ]; then` after the loop, \
                         followed by `exit 1`)"
                    ));
                }
            }
            _ => lint.report(&format!(
                "the {name} read-back does not poll in a bounded loop (`deadline=$((SECONDS + \
                 D))`, `for attempt in $(seq 1 N); do`, `{READBACK_DEADLINE_BREAK}` and `sleep \
                 S`), so its window is not stated (#403)"
            )),
        }
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
    let exports_tags = matches!(
        get(plan, "outputs"),
        Some(Yaml::Hash(outputs)) if get(outputs, "tags-before").is_some()
    );
    if !exports_tags {
        lint.report(
            "job `plan` must export `tags-before`, the dist-tags it read before anything was \
             published, which the publish job compares with (#403)",
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
                    2,
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
                mutate(
                    &text,
                    "          if [ \"${settled}\" != yes ]; then\n",
                    "          if false; then\n",
                    2,
                )?,
                "has no step under `needs.plan.outputs.channel == 'stable'` that reads both \
                 dist-tags back",
            ),
            (
                "no verification of a pre-release publish",
                mutate(
                    &text,
                    "          if [ \"${settled}\" != yes ]; then\n",
                    "          if false; then\n",
                    1,
                )?,
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

    /// The guard (#337, governance rule 15) and the read-back window (#403):
    /// each way the refusal could come after the builds, be skipped, ignored
    /// or decided on the wrong commit, or the read-back could give up too soon,
    /// outlast its job or fail only on time, is a deliberately broken copy of
    /// the real workflow that the lint must name.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one auditable list: each guard and read-back rule's violation and its finding"
    )]
    fn each_guard_and_readback_rule_refuses_its_violation() -> Result<(), Box<dyn Error>> {
        let text = release_text()?;
        let guard_step = "      - name: Refuse a wrong ref, or a candidate without a passed nightly, before anything is built\n";
        let loop_line = "          for attempt in $(seq 1 40); do\n";
        let cases: Vec<(&str, String, &str)> = vec![
            (
                "no guard job",
                mutate(
                    &text,
                    "  guard:\n    name: Ref and nightly guard\n",
                    "  gate:\n    name: Ref and nightly guard\n",
                    1,
                )?,
                "has no `guard` job",
            ),
            (
                "a build that starts before the guard",
                mutate(
                    &text,
                    "    name: Build (${{ matrix.target }})\n    needs:\n      - guard\n",
                    "    name: Build (${{ matrix.target }})\n",
                    1,
                )?,
                "job `build` has no `needs`",
            ),
            (
                "an inventory that starts before the guard",
                mutate(
                    &text,
                    "    name: Notices and SBOMs\n    needs:\n      - guard\n",
                    "    name: Notices and SBOMs\n",
                    1,
                )?,
                "job `inventory` has no `needs`",
            ),
            (
                "a guard that waits for a build",
                mutate(
                    &text,
                    "    name: Ref and nightly guard\n",
                    "    name: Ref and nightly guard\n    needs:\n      - inventory\n",
                    1,
                )?,
                "job `guard` has `needs`",
            ),
            (
                "a guard that can be skipped",
                mutate(
                    &text,
                    "    name: Ref and nightly guard\n",
                    "    name: Ref and nightly guard\n    if: github.event.inputs.dry_run == 'false'\n",
                    1,
                )?,
                "job `guard` has `if`",
            ),
            (
                "a guard whose refusal is ignored",
                mutate(
                    &text,
                    "    name: Ref and nightly guard\n",
                    "    name: Ref and nightly guard\n    continue-on-error: true\n",
                    1,
                )?,
                "job `guard` has `continue-on-error`",
            ),
            (
                "a guard step that can be skipped",
                mutate(
                    &text,
                    guard_step,
                    &format!("{guard_step}        if: github.event_name == 'pull_request'\n"),
                    1,
                )?,
                "of job `guard` has `if`",
            ),
            (
                "a guard step whose failure is ignored",
                mutate(
                    &text,
                    guard_step,
                    &format!("{guard_step}        continue-on-error: true\n"),
                    1,
                )?,
                "of job `guard` has `continue-on-error`",
            ),
            (
                "a guard that may write",
                mutate(
                    &text,
                    "    permissions:\n      actions: read\n      contents: read\n",
                    "    permissions:\n      actions: write\n      contents: read\n",
                    1,
                )?,
                "job `guard` must have exactly the permissions",
            ),
            (
                "a guard with a scope it does not need",
                mutate(
                    &text,
                    "    permissions:\n      actions: read\n      contents: read\n",
                    "    permissions:\n      actions: read\n      contents: read\n      issues: read\n",
                    1,
                )?,
                "job `guard` must have exactly the permissions",
            ),
            (
                "a guard that accepts the nightly of a later commit on main",
                mutate(
                    &text,
                    "runs?head_sha=${SOURCE_COMMIT}&per_page=100",
                    "runs?branch=main&per_page=100",
                    1,
                )?,
                "job `guard` has no step that is given",
            ),
            (
                "a guard that does not look at a run's jobs",
                mutate(
                    &text,
                    "repos/smormah/vsift/actions/runs/${id}/jobs?per_page=100",
                    "repos/smormah/vsift/actions/runs/${id}?per_page=100",
                    1,
                )?,
                "job `guard` has no step that is given",
            ),
            (
                "a guard that does not check the release branch",
                mutate(
                    &text,
                    "compare/${SOURCE_COMMIT}...${head}",
                    "compare/${SOURCE_COMMIT}...main",
                    1,
                )?,
                "job `guard` has no step that is given",
            ),
            (
                "a guard that reads the ref from the pull request",
                mutate(
                    &text,
                    "          GIT_REF: ${{ github.ref }}\n",
                    "          GIT_REF: ${{ github.head_ref }}\n",
                    1,
                )?,
                "job `guard` has no step that is given",
            ),
            (
                "a guard that is not given the commit",
                mutate(
                    &text,
                    "          SOURCE_COMMIT: ${{ github.sha }}\n          GH_TOKEN:",
                    "          GH_TOKEN:",
                    1,
                )?,
                "job `guard` has no step that is given",
            ),
            (
                "a guard that writes through the API",
                mutate(
                    &text,
                    "gh api \"repos/smormah/vsift/compare/",
                    "gh api --method POST \"repos/smormah/vsift/compare/",
                    1,
                )?,
                "job `guard` runs `gh api` other than a plain read",
            ),
            (
                "a guard that reads another repository",
                mutate(
                    &text,
                    "gh api \"repos/smormah/vsift/actions/runs/${id}/jobs",
                    "gh api \"repos/someone/vsift/actions/runs/${id}/jobs",
                    1,
                )?,
                "job `guard` runs `gh api` other than a plain read",
            ),
            (
                "a guard that builds",
                mutate(
                    &text,
                    "          set -euo pipefail\n          refuse() {\n",
                    "          set -euo pipefail\n          cargo run --locked -p vsift-release -- candidate-delta\n          refuse() {\n",
                    1,
                )?,
                "job `guard` runs `cargo`",
            ),
            (
                "a guard with another action",
                mutate(
                    &text,
                    "      - name: Check out the workspace manifest\n        uses: actions/checkout@",
                    "      - name: Check out the workspace manifest\n        uses: someone/checkout@",
                    1,
                )?,
                "job `guard` uses `someone/checkout`",
            ),
            (
                "the nightly read from another job",
                mutate(
                    &text,
                    "          mkdir publish-plan\n",
                    "          mkdir publish-plan\n          gh api repos/smormah/vsift/actions/runs/1/jobs --jq .total_count\n",
                    1,
                )?,
                "job `plan` runs `gh api`; only",
            ),
            (
                "a pre-release read-back of five minutes again",
                mutate(
                    &text,
                    loop_line,
                    "          for attempt in $(seq 1 10); do\n",
                    1,
                )?,
                "the prerelease read-back waits 300 seconds",
            ),
            (
                "a stable read-back of five minutes again",
                mutate(
                    &text,
                    loop_line,
                    "          for attempt in $(seq 1 10); do\n",
                    2,
                )?,
                "the stable read-back waits 300 seconds",
            ),
            (
                "a read-back without a bound",
                mutate(&text, loop_line, "          while true; do\n", 2)?,
                "the stable read-back does not poll in a bounded loop",
            ),
            (
                "a read-back that outlasts its job",
                mutate(
                    &text,
                    "    # and the GitHub release about one.\n    timeout-minutes: 45\n",
                    "    # and the GitHub release about one.\n    timeout-minutes: 30\n",
                    1,
                )?,
                "does not leave 15 minutes of the `publish` job's `timeout-minutes` (30)",
            ),
            (
                "a pre-release read-back that waits through a wrong next",
                mutate(
                    &text,
                    "              if [ -n \"${next_now}\" ] && [ \"${next_now}\" != \"${VERSION}\" ] && [ \"${next_now}\" != \"${next}\" ]; then\n",
                    "              if false; then\n",
                    1,
                )?,
                "the prerelease read-back does not fail at once, inside its loop, on a wrong answer",
            ),
            (
                "a stable read-back that only warns about a moved next",
                mutate(
                    &text,
                    "a stable version never moves it\" >&2\n                exit 1\n",
                    "a stable version never moves it\" >&2\n                settled=no\n",
                    1,
                )?,
                "the stable read-back does not fail at once, inside its loop, on a wrong answer",
            ),
            (
                "a stable read-back that checks a wrong latest only after the window",
                mutate(
                    &text,
                    "              if [ -n \"${latest_now}\" ] && [ \"${latest_now}\" != \"${VERSION}\" ] && [ \"${latest_now}\" != \"${latest}\" ]; then\n",
                    "              if false; then\n",
                    1,
                )?,
                "the stable read-back does not fail at once, inside its loop, on a wrong answer",
            ),
            (
                "a stable read-back that passes when its window ends",
                mutate(
                    &text,
                    "the GitHub release is created then.\" >&2\n            exit 1\n",
                    "the GitHub release is created then.\" >&2\n",
                    1,
                )?,
                "the stable read-back does not fail when its window ends",
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

    /// The re-check after the approval (rule 12): each way the `publish` job
    /// could publish without asking the guard's questions again, or ask them
    /// differently, is a broken copy the lint must name.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one auditable list: each re-check rule's violation and its finding"
    )]
    fn each_recheck_rule_refuses_its_violation() -> Result<(), Box<dyn Error>> {
        let text = release_text()?;
        let recheck = "      - name: Re-check the guard's conditions after the approval, before anything is published\n";
        let cases: Vec<(&str, String, &str)> = vec![
            (
                "no re-check of the nightly",
                mutate(
                    &text,
                    "runs?head_sha=${SOURCE_COMMIT}&per_page=100",
                    "runs?branch=main&per_page=100",
                    2,
                )?,
                "has no step that re-checks the guard's conditions",
            ),
            (
                "no read of the commit the tag names",
                mutate(
                    &text,
                    "gh api \"repos/smormah/vsift/git/ref/tags/${tag}\"",
                    "gh api \"repos/smormah/vsift/compare/${tag}\"",
                    2,
                )?,
                "has no step that re-checks the guard's conditions",
            ),
            (
                "a re-check given another commit",
                mutate(
                    &text,
                    "          SOURCE_COMMIT: ${{ github.sha }}\n          VERSION:",
                    "          SOURCE_COMMIT: ${{ needs.plan.outputs.tag }}\n          VERSION:",
                    1,
                )?,
                "has no step that re-checks the guard's conditions",
            ),
            (
                "a publish before the re-check",
                mutate(
                    &text,
                    recheck,
                    &format!(
                        "      - name: Publish early\n        if: needs.plan.outputs.channel == 'prerelease'\n        env:\n          VERSION: ${{{{ needs.plan.outputs.version }}}}\n        run: |\n          [[ \"${{VERSION}}\" =~ ^[0-9]+\\.[0-9]+\\.[0-9]+-[0-9A-Za-z.-]+$ ]]\n          npm publish ./npm-packages/x.tgz --tag next --access public --provenance --ignore-scripts\n\n{recheck}"
                    ),
                    1,
                )?,
                "publishes or releases before its re-check",
            ),
            (
                "a re-check that can be skipped",
                mutate(
                    &text,
                    recheck,
                    &format!("{recheck}        if: needs.plan.outputs.channel == 'prerelease'\n"),
                    1,
                )?,
                "the `publish` job's re-check has `if`",
            ),
            (
                "a re-check whose refusal is ignored",
                mutate(
                    &text,
                    recheck,
                    &format!("{recheck}        continue-on-error: true\n"),
                    1,
                )?,
                "the `publish` job's re-check has `continue-on-error`",
            ),
            (
                "a re-check that accepts a moved tag",
                mutate(
                    &text,
                    "[ \"${tagged}\" = \"${SOURCE_COMMIT}\" ] || refuse",
                    "[ -n \"${tagged}\" ] || refuse",
                    1,
                )?,
                "does not refuse with `[ \"${tagged}\" = \"${SOURCE_COMMIT}\" ] || refuse`",
            ),
            (
                "a re-check that accepts any ref",
                mutate(
                    &text,
                    "[ \"${GIT_REF}\" = \"refs/tags/${tag}\" ] || refuse",
                    "[ -n \"${GIT_REF}\" ] || refuse",
                    1,
                )?,
                "does not refuse with `[ \"${GIT_REF}\" = \"refs/tags/${tag}\" ] || refuse`",
            ),
            (
                "a re-check whose nightly rule drifted from the guard's",
                mutate(&text, "$2 != \"Report a failure\"", "$2 !~ /Report/", 2)?,
                "does not carry the guard's `nightly_passed` function",
            ),
            (
                "a re-check that accepts a branch the guard refuses",
                mutate(
                    &text,
                    "              identical | ahead) return 0 ;;\n",
                    "              identical | ahead | behind) return 0 ;;\n",
                    2,
                )?,
                "does not carry the guard's `on_release_branch` function",
            ),
            (
                "a guard whose nightly rule drifted from the re-check's",
                mutate(&text, "$2 != \"Report a failure\"", "$2 !~ /Report/", 1)?,
                "does not carry the guard's `nightly_passed` function",
            ),
            (
                "a re-check that writes through the API",
                mutate(
                    &text,
                    "gh api \"repos/smormah/vsift/git/ref/tags/${tag}\"",
                    "gh api -X POST \"repos/smormah/vsift/git/ref/tags/${tag}\"",
                    2,
                )?,
                "job `publish` runs `gh api` other than a plain read",
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

    /// The review of 2026-10-17 (S1 to S4, N1 to N4): each hole it found is a
    /// broken copy of the real workflow that the lint must name.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one auditable list: each reviewed hole and the finding that closes it"
    )]
    fn each_reviewed_hole_is_refused() -> Result<(), Box<dyn Error>> {
        let text = release_text()?;
        let next_read = "next_now=\"$(timeout 20 npm view \"${package}\" dist-tags.next --prefer-online --fetch-retries=1 2>/dev/null || true)\"";
        let cases: Vec<(&str, String, &str)> = vec![
            (
                "S1: a guard that accepts a moved tag",
                mutate(
                    &text,
                    "[ \"${tagged}\" = \"${SOURCE_COMMIT}\" ] || refuse",
                    "[ -n \"${tagged}\" ] || refuse",
                    1,
                )?,
                "job `guard` does not refuse with `[ \"${tagged}\" = \"${SOURCE_COMMIT}\" ] || refuse`",
            ),
            (
                "S1: a re-check that does not peel an annotated tag like the guard",
                mutate(
                    &text,
                    "      tag) answer=\"$(gh api \"repos/smormah/vsift/git/tags/${sha}\" --jq \"${ref_filter}\")\" || answer=\"\" ;;\n",
                    "",
                    2,
                )?,
                "does not carry the guard's `tag_commit` function",
            ),
            (
                "S2: a read without a cut-off",
                mutate(
                    &text,
                    next_read,
                    "next_now=\"$(npm view \"${package}\" dist-tags.next --prefer-online 2>/dev/null || true)\"",
                    1,
                )?,
                "the prerelease read-back runs `npm view` without `timeout <seconds>`",
            ),
            (
                "S2: a read-back without its deadline",
                mutate(
                    &text,
                    "if [ \"${settled}\" = yes ] || [ \"${SECONDS}\" -ge \"${deadline}\" ]; then",
                    "if [ \"${settled}\" = yes ]; then",
                    2,
                )?,
                "the stable read-back does not poll in a bounded loop",
            ),
            (
                "S2: cut-offs that, counted, outlast the job",
                mutate(
                    &text,
                    "latest_now=\"$(timeout 20 npm view",
                    "latest_now=\"$(timeout 300 npm view",
                    3,
                )?,
                "the stable read-back can take",
            ),
            (
                "S2: a deadline beyond the job's timeout",
                mutate(
                    &text,
                    "deadline=$((SECONDS + 1200))",
                    "deadline=$((SECONDS + 2400))",
                    1,
                )?,
                "the prerelease read-back can take",
            ),
            (
                "S3: the dist-tags read again in the publish job",
                mutate(
                    &text,
                    "          printf '%s\\n' \"${TAGS_BEFORE}\" > \"${RUNNER_TEMP}/plan-tags\"\n",
                    "          npm view vsift-cli dist-tags --json > \"${RUNNER_TEMP}/plan-tags\"\n",
                    1,
                )?,
                "records the dist-tags before publishing from something other than the plan's output",
            ),
            (
                "S3: the record not taken from the plan",
                mutate(
                    &text,
                    "          TAGS_BEFORE: ${{ needs.plan.outputs.tags-before }}\n",
                    "          TAGS_BEFORE: ${{ needs.plan.outputs.version }}\n",
                    1,
                )?,
                "records the dist-tags before publishing from something other than the plan's output",
            ),
            (
                "S3: a plan that does not export what it read",
                mutate(
                    &text,
                    "      tags-before: ${{ steps.plan.outputs.tags-before }}\n",
                    "",
                    1,
                )?,
                "job `plan` must export `tags-before`",
            ),
            (
                "S4: a candidate read-back that settles on no answer",
                mutate(
                    &text,
                    "if [ \"${next_now}\" != \"${VERSION}\" ] || { [ -z \"${latest_now}\" ] && [ \"${latest}\" != none ]; }; then",
                    "if [ -z \"${next_now}\" ] || { [ -z \"${latest_now}\" ] && [ \"${latest}\" != none ]; }; then",
                    1,
                )?,
                "the prerelease read-back does not keep waiting while the tag has not moved",
            ),
            (
                "S4: a stable read-back that never marks itself unsettled",
                mutate(
                    &text,
                    "                settled=no\n                echo \"attempt ${attempt}: ${package} latest",
                    "                echo \"attempt ${attempt}: ${package} latest",
                    1,
                )?,
                "the stable read-back does not keep waiting while the tag has not moved",
            ),
            (
                "N1: a comparison with the branch's name, which a tag of that name would win",
                mutate(
                    &text,
                    "compare/${SOURCE_COMMIT}...${head}",
                    "compare/${SOURCE_COMMIT}...${release_branch}",
                    1,
                )?,
                "job `guard` has no step that is given",
            ),
            (
                "N2: only completed runs read, so a run in progress is not seen",
                mutate(
                    &text,
                    "runs?head_sha=${SOURCE_COMMIT}&per_page=100",
                    "runs?head_sha=${SOURCE_COMMIT}&status=completed&per_page=100",
                    1,
                )?,
                "job `guard` has no step that is given",
            ),
            (
                "N2: a re-check that ignores a run in progress, unlike the guard",
                mutate(
                    &text,
                    "    if [ -n \"${id}\" ] && [ \"${status}\" != completed ]; then\n",
                    "    if false; then\n",
                    2,
                )?,
                "does not carry the guard's `nightly_passed` function",
            ),
            (
                "N3: a job that runs whatever the guard said",
                mutate(
                    &text,
                    "    name: Notices and SBOMs\n",
                    "    name: Notices and SBOMs\n    if: always()\n",
                    1,
                )?,
                "job `inventory` has `always()` in its condition",
            ),
            (
                "N3: a job that runs unless cancelled",
                mutate(
                    &text,
                    "    name: Publish plan (a dry run publishes nothing)\n",
                    "    name: Publish plan (a dry run publishes nothing)\n    if: ${{ !cancelled() }}\n",
                    1,
                )?,
                "job `plan` has `cancelled()` in its condition",
            ),
            (
                "N3: a job that turns a failure into a pass",
                mutate(
                    &text,
                    "    name: Build (${{ matrix.target }})\n",
                    "    name: Build (${{ matrix.target }})\n    continue-on-error: true\n",
                    1,
                )?,
                "job `build` has `continue-on-error`",
            ),
            (
                "N4: a guard that does not read the release branch's rules",
                mutate(
                    &text,
                    "rules/branches/${release_branch}",
                    "branches/${release_branch}",
                    1,
                )?,
                "job `guard` has no step that is given",
            ),
            (
                "N4: a re-check that asks for fewer rules than the guard",
                mutate(
                    &text,
                    "  for rule in creation deletion non_fast_forward pull_request required_status_checks; do\n",
                    "  for rule in creation deletion; do\n",
                    2,
                )?,
                "does not carry the guard's `release_branch_protected` function",
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

    /// The re-review of 2026-10-17: the stable publish reads `latest` again
    /// after the approval, so a lower stable version published second cannot
    /// move it backwards on its plan's stale value.
    #[test]
    fn a_stable_publish_that_trusts_the_plans_latest_is_refused() -> Result<(), Box<dyn Error>> {
        let text = release_text()?;
        let cases: Vec<(&str, String)> = vec![
            (
                "no fresh read",
                mutate(
                    &text,
                    "            latest_now=\"$(timeout 20 npm view \"${package}\" dist-tags.latest --prefer-online --fetch-retries=1 2>/dev/null || true)\"\n            if [ \"${latest_now}\" != \"${latest}\" ]",
                    "            latest_now=\"${latest}\"\n            if [ \"${latest_now}\" != \"${latest}\" ]",
                    1,
                )?,
            ),
            (
                "a refusal that only warns",
                mutate(
                    &text,
                    "nor ${VERSION}: nothing is published\" >&2\n              exit 1\n",
                    "nor ${VERSION}: nothing is published\" >&2\n",
                    1,
                )?,
            ),
            (
                "a check that accepts any version",
                mutate(
                    &text,
                    "if [ \"${latest_now}\" != \"${latest}\" ] && [ \"${latest_now}\" != \"${VERSION}\" ]; then\n              echo \"${package}: latest is",
                    "if [ -z \"${latest_now}\" ]; then\n              echo \"${package}: latest is",
                    1,
                )?,
            ),
        ];
        for (case, mutated) in cases {
            let messages = findings(&mutated);
            assert!(
                messages
                    .iter()
                    .any(|message| message
                        .contains("does not read `latest` again before publishing")),
                "{case}: {messages:#?}"
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
            "has no `guard` job",
        ] {
            assert!(
                messages.iter().any(|message| message.contains(expected)),
                "{expected}: {messages:#?}"
            );
        }
    }
}
