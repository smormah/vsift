//! The self-hosted runner policy (ADR 0025): rule 9 of the workflow lint.
//!
//! The repository is public and belongs to a person, not an organisation, so
//! GitHub's runner groups cannot restrict which workflows reach a self-hosted
//! runner: any job of any workflow that names its labels is sent to it. This
//! rule is the restriction, checked when a change is merged. It is not a
//! run-time guard: a pull request's own copy of a workflow runs before anyone
//! merges it, which is why the runner also has a job-started hook of its own
//! (`tools/loki/job-guard.cjs`) and the repository requires approval for
//! outside collaborators' workflow runs (ADR 0025, residual risks).
//!
//! The rules, each fail-closed:
//!
//! 1. **Every runner a job names is visible and known.** A `runs-on` label is
//!    a GitHub-hosted image (`ubuntu-24.04`, `windows-latest`, `macos-15`,
//!    ...) or one of the labels of the one approved self-hosted runner,
//!    LOKI (`loki`, and its default labels `self-hosted`, `windows`, `x64`).
//!    A label may come from a literal matrix (`${{ matrix.os }}`); any other
//!    expression is a finding, except the jobs of [`COMPUTED_RUNNER_JOBS`],
//!    whose runners a reviewed script chooses from hosted labels. Runner
//!    groups (`runs-on: { group: ... }`) are refused: a personal repository
//!    has none.
//! 2. **A job that reaches LOKI says so.** A job whose labels can reach the
//!    self-hosted runner names `loki` among them; `self-hosted`, `windows` or
//!    `x64` alone would reach it silently.
//! 3. **Only scheduled or manually started workflows.** A workflow with a
//!    LOKI job has no trigger but `schedule` and `workflow_dispatch`: never
//!    `pull_request`, `pull_request_target`, `push`, `workflow_run`,
//!    `issue_comment`, `workflow_call` or any other event.
//! 4. **Never the release path.** The release workflow and the engine workflow
//!    (ADR 0032) have no LOKI job, and a
//!    workflow with one grants no write scope but a hosted job's
//!    `issues: write` (its failure report), so it cannot attest (`id-token`,
//!    `attestations`), sign with an OIDC identity, push or release; it runs
//!    no job in a deployment `environment`. Publishing is already refused
//!    outside the release workflow (rule 8).
//! 5. **Least privilege.** A LOKI job declares `permissions` of exactly
//!    `contents: read` (any other scope only as `none`).
//! 6. **No secret.** A workflow with a LOKI job names no secret but
//!    `secrets.GITHUB_TOKEN` and passes none to a reusable workflow; the LOKI
//!    job itself names neither a secret nor `github.token`.
//! 7. **No credential left on the machine.** Every `actions/checkout` step of
//!    a LOKI job sets `persist-credentials: false`, so the job token is not
//!    written into the checkout's Git configuration on a persistent machine.
//! 8. **Bounded.** A LOKI job sets a literal `timeout-minutes` of at most
//!    [`MAXIMUM_TIMEOUT_MINUTES`]: a self-hosted job otherwise may run for
//!    days.

use yaml_rust2::{Yaml, yaml::Hash};

use super::{Lint, context_references, expressions, get, visit_strings};

/// The label that names the one approved self-hosted runner (ADR 0025).
pub(crate) const SELF_HOSTED_LABEL: &str = "loki";

/// Labels that reach LOKI: its own label and the default labels the runner
/// application gives a Windows x64 machine. No GitHub-hosted image answers to
/// any of them.
const SELF_HOSTED_LABELS: [&str; 4] = [SELF_HOSTED_LABEL, "self-hosted", "windows", "x64"];

/// The only events that may start a workflow with a LOKI job.
const ALLOWED_TRIGGERS: [&str; 2] = ["schedule", "workflow_dispatch"];

/// The longest `timeout-minutes` a LOKI job may set (12 hours: a night).
const MAXIMUM_TIMEOUT_MINUTES: i64 = 720;

/// Jobs whose runner is computed, with the reason the lint accepts it.
///
/// `ci.yml`'s `quality` job reads its systems from `tools/ci_lanes.py`
/// (`SYSTEMS_UBUNTU`, `SYSTEMS_ALL`), which names GitHub-hosted labels only;
/// the job runs on pull requests, so LOKI's job-started hook would refuse it
/// even if the script were changed to name `loki`.
const COMPUTED_RUNNER_JOBS: [(&str, &str); 1] = [(".github/workflows/ci.yml", "quality")];

/// The families of GitHub-hosted image labels.
const HOSTED_FAMILIES: [&str; 3] = ["ubuntu-", "windows-", "macos-"];

/// The suffixes a GitHub-hosted image label may carry after its version.
const HOSTED_SUFFIXES: [&str; 4] = ["-arm64", "-arm", "-xlarge", "-large"];

/// The write scope a hosted job of a LOKI workflow may hold.
const REPORT_WRITE_SCOPE: &str = "issues";

/// What the lint knows about one job's runner.
struct Runner {
    labels: Vec<String>,
    unseen: Vec<String>,
}

/// Applies rule 9 to one workflow.
pub(super) fn check_runners(lint: &mut Lint<'_>, workflow: &Hash) {
    let Some(Yaml::Hash(jobs)) = get(workflow, "jobs") else {
        return;
    };
    let mut self_hosted_jobs: Vec<&str> = Vec::new();
    for (id, job) in jobs {
        let (Some(id), Yaml::Hash(job)) = (id.as_str(), job) else {
            continue;
        };
        if check_job_runner(lint, id, job) {
            self_hosted_jobs.push(id);
        }
    }
    if self_hosted_jobs.is_empty() {
        return;
    }
    let job_list = self_hosted_jobs
        .iter()
        .map(|id| format!("`{id}`"))
        .collect::<Vec<_>>()
        .join(", ");
    if lint.release || lint.engines {
        lint.report(&format!(
            "runs {job_list} on the self-hosted runner; the release workflow runs only on \
             GitHub-hosted runners, and so does the engine workflow (ADR 0025, ADR 0032); \
             their attestations are verified with `--deny-self-hosted-runners`"
        ));
    }
    check_triggers(lint, workflow, &job_list);
    check_secrets(lint, workflow, &job_list);
    for (id, job) in jobs {
        let (Some(id), Yaml::Hash(job)) = (id.as_str(), job) else {
            continue;
        };
        let self_hosted = self_hosted_jobs.contains(&id);
        check_write_scopes(lint, id, job, self_hosted);
        if get(job, "environment").is_some() {
            lint.report(&format!(
                "job `{id}` runs in a deployment environment in a workflow that runs on the \
                 self-hosted runner; environments guard the release path (ADR 0025)"
            ));
        }
        if self_hosted {
            check_self_hosted_job(lint, id, job);
        }
    }
}

/// Checks the labels of one job and returns whether it can reach LOKI.
fn check_job_runner(lint: &mut Lint<'_>, id: &str, job: &Hash) -> bool {
    let Some(runs_on) = get(job, "runs-on") else {
        return false;
    };
    let runner = runner_of(lint, id, job, runs_on);
    let computed = COMPUTED_RUNNER_JOBS.contains(&(lint.path, id));
    if !computed {
        for unseen in &runner.unseen {
            lint.report(&format!(
                "job `{id}` names its runner through `{unseen}`, which the lint cannot read; \
                 name it literally or in a literal matrix, so no job reaches the self-hosted \
                 runner unseen (ADR 0025)"
            ));
        }
    }
    let mut self_hosted = false;
    for label in &runner.labels {
        let lower = label.to_ascii_lowercase();
        if SELF_HOSTED_LABELS.contains(&lower.as_str()) {
            self_hosted = true;
        } else if !is_hosted_label(&lower) {
            lint.report(&format!(
                "job `{id}` names the runner label `{label}`, which is neither a GitHub-hosted \
                 image nor the `{SELF_HOSTED_LABEL}` runner (ADR 0025)"
            ));
        }
    }
    if self_hosted
        && !runner
            .labels
            .iter()
            .any(|label| label.eq_ignore_ascii_case(SELF_HOSTED_LABEL))
    {
        lint.report(&format!(
            "job `{id}` can reach the self-hosted runner without naming `{SELF_HOSTED_LABEL}`; \
             a job that runs there names it (ADR 0025)"
        ));
    }
    self_hosted
}

/// Reads a `runs-on` value into the labels it names and the expressions the
/// lint cannot resolve.
fn runner_of(lint: &mut Lint<'_>, id: &str, job: &Hash, runs_on: &Yaml) -> Runner {
    let mut runner = Runner {
        labels: Vec::new(),
        unseen: Vec::new(),
    };
    match runs_on {
        Yaml::String(label) => add_label(&mut runner, job, label),
        Yaml::Array(labels) => {
            for label in labels {
                match label.as_str() {
                    Some(label) => add_label(&mut runner, job, label),
                    None => runner.unseen.push(String::from("a label that is not text")),
                }
            }
        }
        Yaml::Hash(map) => {
            if get(map, "group").is_some() {
                lint.report(&format!(
                    "job `{id}` names a runner group; a personal repository has none, and the \
                     policy names its one self-hosted runner by label (ADR 0025)"
                ));
            }
            match get(map, "labels") {
                Some(Yaml::String(label)) => add_label(&mut runner, job, label),
                Some(Yaml::Array(labels)) => {
                    for label in labels {
                        match label.as_str() {
                            Some(label) => add_label(&mut runner, job, label),
                            None => runner.unseen.push(String::from("a label that is not text")),
                        }
                    }
                }
                Some(_) => runner
                    .unseen
                    .push(String::from("`labels` of an unexpected shape")),
                None => {}
            }
        }
        _ => runner
            .unseen
            .push(String::from("a `runs-on` of an unexpected shape")),
    }
    runner
}

/// Adds one `runs-on` entry: a literal label, or the values of the literal
/// matrix entry `${{ matrix.<key> }}` names.
fn add_label(runner: &mut Runner, job: &Hash, label: &str) {
    if !label.contains("${{") {
        runner.labels.push(label.to_owned());
        return;
    }
    let key = label
        .trim()
        .strip_prefix("${{")
        .and_then(|rest| rest.strip_suffix("}}"))
        .map(str::trim)
        .and_then(|reference| reference.strip_prefix("matrix."))
        .filter(|key| {
            !key.is_empty()
                && key.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                })
        });
    let Some(key) = key else {
        runner.unseen.push(label.trim().to_owned());
        return;
    };
    let matrix = match get(job, "strategy") {
        Some(Yaml::Hash(strategy)) => get(strategy, "matrix"),
        _ => None,
    };
    let Some(Yaml::Hash(matrix)) = matrix else {
        runner.unseen.push(label.trim().to_owned());
        return;
    };
    let mut found = false;
    match get(matrix, key) {
        None => {}
        Some(Yaml::Array(values)) => {
            for value in values {
                found = true;
                match value.as_str() {
                    Some(value) if !value.contains("${{") => runner.labels.push(value.to_owned()),
                    _ => runner.unseen.push(format!("matrix.{key}")),
                }
            }
        }
        Some(Yaml::String(value)) if !value.contains("${{") => {
            found = true;
            runner.labels.push(value.clone());
        }
        Some(_) => {
            found = true;
            runner.unseen.push(format!("matrix.{key}"));
        }
    }
    match get(matrix, "include") {
        None => {}
        Some(Yaml::Array(entries)) => {
            for entry in entries {
                let Yaml::Hash(entry) = entry else {
                    runner.unseen.push(String::from("matrix.include"));
                    continue;
                };
                match get(entry, key) {
                    None => {}
                    Some(Yaml::String(value)) if !value.contains("${{") => {
                        found = true;
                        runner.labels.push(value.clone());
                    }
                    Some(_) => {
                        found = true;
                        runner.unseen.push(format!("matrix.include.{key}"));
                    }
                }
            }
        }
        Some(_) => runner.unseen.push(String::from("matrix.include")),
    }
    if !found {
        runner.unseen.push(label.trim().to_owned());
    }
}

/// Whether a lower-case label is a GitHub-hosted image: `ubuntu-`, `windows-`
/// or `macos-`, then `latest` or a version of digits and dots, then
/// optionally an architecture or size suffix.
fn is_hosted_label(label: &str) -> bool {
    let Some(rest) = HOSTED_FAMILIES
        .iter()
        .find_map(|family| label.strip_prefix(family))
    else {
        return false;
    };
    let version = HOSTED_SUFFIXES
        .iter()
        .find_map(|suffix| rest.strip_suffix(suffix))
        .unwrap_or(rest);
    version == "latest"
        || (version.starts_with(|character: char| character.is_ascii_digit())
            && version
                .chars()
                .all(|character| character.is_ascii_digit() || character == '.')
            && !version.ends_with('.'))
}

/// Rule 9.3: only `schedule` and `workflow_dispatch`.
fn check_triggers(lint: &mut Lint<'_>, workflow: &Hash, job_list: &str) {
    let names: Vec<String> = match get(workflow, "on") {
        Some(Yaml::String(name)) => vec![name.clone()],
        Some(Yaml::Array(items)) => items
            .iter()
            .map(|item| item.as_str().unwrap_or("<not text>").to_owned())
            .collect(),
        Some(Yaml::Hash(map)) => map
            .keys()
            .map(|key| key.as_str().unwrap_or("<not text>").to_owned())
            .collect(),
        // The general trigger rule already reports a missing or odd `on`.
        _ => return,
    };
    let refused: Vec<String> = names
        .iter()
        .filter(|name| !ALLOWED_TRIGGERS.contains(&name.as_str()))
        .map(|name| format!("`{name}`"))
        .collect();
    if !refused.is_empty() {
        lint.report(&format!(
            "runs {job_list} on the self-hosted runner but is triggered by {}; a workflow with a \
             self-hosted job is started only by `schedule` or `workflow_dispatch` (ADR 0025)",
            refused.join(", ")
        ));
    }
}

/// Rule 9.6, for the whole workflow: no secret but `GITHUB_TOKEN`, and none
/// handed to a reusable workflow.
fn check_secrets(lint: &mut Lint<'_>, workflow: &Hash, job_list: &str) {
    let mut found: Vec<String> = Vec::new();
    visit_strings(workflow, &mut |text| {
        for reference in secret_references(text) {
            if reference != "secrets.GITHUB_TOKEN" && !found.contains(&reference) {
                found.push(reference);
            }
        }
    });
    for reference in found {
        lint.report(&format!(
            "names `{reference}` in a workflow that runs {job_list} on the self-hosted runner; such \
             a workflow uses no secret but `GITHUB_TOKEN` (ADR 0025)"
        ));
    }
    if let Some(Yaml::Hash(jobs)) = get(workflow, "jobs") {
        for (id, job) in jobs {
            if let (Some(id), Yaml::Hash(job)) = (id.as_str(), job)
                && get(job, "secrets").is_some()
            {
                lint.report(&format!(
                    "job `{id}` passes secrets to a reusable workflow in a workflow that runs on \
                     the self-hosted runner (ADR 0025)"
                ));
            }
        }
    }
}

/// The `secrets` references of the expressions in one text.
fn secret_references(text: &str) -> Vec<String> {
    expressions(text)
        .into_iter()
        .flat_map(context_references)
        .filter(|reference| reference == "secrets" || reference.starts_with("secrets."))
        .collect()
}

/// Rule 9.4: the only write scope of a LOKI workflow is a hosted job's
/// `issues: write`.
fn check_write_scopes(lint: &mut Lint<'_>, id: &str, job: &Hash, self_hosted: bool) {
    let Some(Yaml::Hash(permissions)) = get(job, "permissions") else {
        // A shorthand is already refused by rule 3.
        return;
    };
    for (scope, level) in permissions {
        let (Some(scope), Some("write")) = (scope.as_str(), level.as_str()) else {
            continue;
        };
        if self_hosted || scope != REPORT_WRITE_SCOPE {
            lint.report(&format!(
                "job `{id}` holds `{scope}: write` in a workflow that runs on the self-hosted \
                 runner; the only write scope there is a GitHub-hosted job's \
                 `{REPORT_WRITE_SCOPE}: write`, so the workflow cannot attest, sign, push or \
                 release (ADR 0025)"
            ));
        }
    }
}

/// Rules 9.5 to 9.8 for one job that runs on LOKI.
fn check_self_hosted_job(lint: &mut Lint<'_>, id: &str, job: &Hash) {
    let least_privilege = match get(job, "permissions") {
        Some(Yaml::Hash(permissions)) => {
            let mut contents_read = false;
            let mut other = false;
            for (scope, level) in permissions {
                match (scope.as_str(), level.as_str()) {
                    (Some("contents"), Some("read")) => contents_read = true,
                    (Some(_), Some("none")) => {}
                    _ => other = true,
                }
            }
            contents_read && !other
        }
        _ => false,
    };
    if !least_privilege {
        lint.report(&format!(
            "job `{id}` runs on the self-hosted runner and must declare `permissions` of exactly \
             `contents: read` (ADR 0025)"
        ));
    }

    let mut token_references: Vec<String> = Vec::new();
    visit_strings(job, &mut |text| {
        for expression in expressions(text) {
            for reference in context_references(expression) {
                let token = reference == "github.token"
                    || reference == "secrets"
                    || reference.starts_with("secrets.");
                if token && !token_references.contains(&reference) {
                    token_references.push(reference);
                }
            }
        }
    });
    for reference in token_references {
        lint.report(&format!(
            "job `{id}` runs on the self-hosted runner and names `{reference}`; it is handed no \
             token or secret (ADR 0025)"
        ));
    }

    if let Some(Yaml::Array(steps)) = get(job, "steps") {
        for (index, step) in steps.iter().enumerate() {
            let Yaml::Hash(step) = step else {
                continue;
            };
            let checkout = get(step, "uses")
                .and_then(Yaml::as_str)
                .is_some_and(|uses| uses.starts_with("actions/checkout@"));
            if !checkout {
                continue;
            }
            let persisted = match get(step, "with") {
                Some(Yaml::Hash(with)) => get(with, "persist-credentials"),
                _ => None,
            };
            let off = match persisted {
                Some(Yaml::Boolean(value)) => !value,
                Some(Yaml::String(value)) => value == "false",
                _ => false,
            };
            if !off {
                lint.report(&format!(
                    "job `{id}` step {} checks out on the self-hosted runner without \
                     `persist-credentials: false`, which would leave the job token in the \
                     checkout (ADR 0025)",
                    index + 1
                ));
            }
        }
    }

    match get(job, "timeout-minutes") {
        Some(Yaml::Integer(minutes)) if (1..=MAXIMUM_TIMEOUT_MINUTES).contains(minutes) => {}
        _ => lint.report(&format!(
            "job `{id}` runs on the self-hosted runner without a literal `timeout-minutes` of 1 \
             to {MAXIMUM_TIMEOUT_MINUTES} (ADR 0025)"
        )),
    }
}

#[cfg(test)]
mod tests;
