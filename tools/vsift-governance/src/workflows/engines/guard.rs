//! The tag guard of the engine release workflow (ADR 0032 section E), held by
//! the lint line by line.
//!
//! A publish happens only on an engine's own tag, never a `v*` tag, on a
//! commit of `main`, when the tag still names the commit the run builds and
//! when no release of it exists, not even a draft. Those checks are shell in
//! the `guard` job (and the tag and draft checks again in `publish`, after the
//! maintainer's approval), so deleting one would leave every other rule
//! green. This module requires each check to stand in the script as one whole
//! command, in front of the line that sets `mode=publish`, and requires every
//! credential-like input of the checks to be bound to the one context
//! expression that cannot be chosen by whoever dispatches the run.

use yaml_rust2::{Yaml, yaml::Hash};

use super::super::{Lint, get, logical_lines};
use super::{GUARD_JOB, PUBLISH_JOB, steps};

/// A command of the guard's script the lint requires, as the script's words
/// joined by single spaces. `Prefix` ends in `|| refuse `, so a second
/// alternative added before the refusal does not match.
enum Required {
    /// The whole command is exactly this.
    Exactly(&'static str),
    /// The command starts with this.
    Prefix(&'static str),
}

/// What each required command checks, and the command itself.
const GUARD_CHECKS: [(&str, Required); 8] = [
    (
        "that the tag is `<engine>-v<version>` of the chosen engine",
        Required::Exactly("tag=\"${ENGINE_INPUT}-v${version}\""),
    ),
    (
        "that the ref is the engine's tag (so never a `v*` tag)",
        Required::Prefix("[ \"${GIT_REF}\" = \"refs/tags/${tag}\" ] || refuse "),
    ),
    (
        "that the tag is read from GitHub",
        Required::Exactly("tag_commit || tagged=\"\""),
    ),
    (
        "that the tag still names this run's commit",
        Required::Prefix("[ \"${tagged}\" = \"${SOURCE_COMMIT}\" ] || refuse "),
    ),
    (
        "that the commit is on `main`",
        Required::Prefix("on_main || refuse "),
    ),
    (
        "that `main` is compared with this run's commit, and only `identical` and `ahead` pass",
        Required::Exactly("identical | ahead) return 0 ;;"),
    ),
    (
        "that the tag has no release",
        Required::Prefix("no_release || refuse "),
    ),
    (
        "that the tag has no draft release either",
        Required::Prefix("[ -z \"${drafts}\" ] || refuse "),
    ),
];

/// Commands of `publish`'s re-check, after the approval, each exactly.
const PUBLISH_CHECKS: [(&str, &str); 3] = [
    (
        "that the ref is still the engine's tag",
        "[ \"${GIT_REF}\" = \"refs/tags/${ENGINE_TAG}\" ]",
    ),
    (
        "that the tag still names this run's commit",
        "[ \"${tagged}\" = \"${SOURCE_COMMIT}\" ]",
    ),
    ("that the tag has no draft release", "[ -z \"${drafts}\" ]"),
];

/// The one expression each input of the checks may be bound to.
const BOUND_INPUTS: [(&str, &str); 4] = [
    ("ENGINE_TAG", "${{ needs.guard.outputs.tag }}"),
    ("GIT_REF", "${{ github.ref }}"),
    ("SOURCE_COMMIT", "${{ github.sha }}"),
    ("REPOSITORY", "${{ github.repository }}"),
];

/// The line of the guard's script that lets the run publish.
const MODE_PUBLISH: &str = "echo \"mode=publish\"";

pub(super) fn check_guard(lint: &mut Lint<'_>, workflow: &Hash, jobs: &Hash) {
    if let Some(Yaml::Hash(guard)) = get(jobs, GUARD_JOB) {
        check_guard_script(lint, workflow, guard);
    }
    if let Some(Yaml::Hash(publish)) = get(jobs, PUBLISH_JOB) {
        check_publish_script(lint, publish);
    }
    for (id, job) in jobs {
        if let (Some(id), Yaml::Hash(job)) = (id.as_str(), job) {
            check_bound_inputs(lint, id, job);
        }
    }
}

/// The words of every command of every script of a job, each command's words
/// joined by single spaces.
fn commands(job: &Hash) -> Vec<String> {
    let mut commands = Vec::new();
    for step in steps(job) {
        if let Some(Yaml::String(script)) = get(step, "run") {
            for line in logical_lines(script) {
                commands.push(line.split_whitespace().collect::<Vec<_>>().join(" "));
            }
        }
    }
    commands
}

fn check_guard_script(lint: &mut Lint<'_>, workflow: &Hash, guard: &Hash) {
    let commands = commands(guard);
    let position = |required: &Required| {
        commands.iter().position(|command| match required {
            Required::Exactly(text) => command == text,
            Required::Prefix(text) => command.starts_with(text),
        })
    };
    let publishes = commands
        .iter()
        .position(|command| command.starts_with(MODE_PUBLISH));
    if publishes.is_none() {
        lint.report(
            "job `guard` never sets `mode=publish`; a publish is decided by the guard alone",
        );
    }
    for (meaning, required) in &GUARD_CHECKS {
        match (position(required), publishes) {
            (None, _) => lint.report(&format!(
                "job `guard` no longer checks {meaning}; the check must stand as one whole \
                 command ending in a refusal (ADR 0032 section E)"
            )),
            (Some(at), Some(publish_at)) if at > publish_at => lint.report(&format!(
                "job `guard` checks {meaning} only after it has set `mode=publish`"
            )),
            _ => {}
        }
    }
    if commands
        .iter()
        .any(|command| command.contains("refs/tags/v"))
    {
        lint.report(
            "job `guard` names a `refs/tags/v` ref; the engine workflow never accepts a `v*` tag, \
             which is a VSift release",
        );
    }
    check_engine_list(lint, workflow, &commands);
}

/// The engines the guard accepts are exactly the dispatch input's options, and
/// none is named like a version, so no tag `v*` can be an engine's tag.
fn check_engine_list(lint: &mut Lint<'_>, workflow: &Hash, commands: &[String]) {
    let accepted: Vec<&str> = commands
        .iter()
        .position(|command| command == "case \"${ENGINE_INPUT}\" in")
        .and_then(|at| commands.get(at + 1))
        .and_then(|list| list.strip_suffix(") ;;"))
        .map(|list| list.split(" | ").collect())
        .unwrap_or_default();
    let options: Vec<&str> = dispatch_engine_options(workflow);
    let mut sorted_accepted = accepted.clone();
    sorted_accepted.sort_unstable();
    let mut sorted_options = options;
    sorted_options.sort_unstable();
    if accepted.is_empty() || sorted_accepted != sorted_options {
        lint.report(
            "job `guard` must accept exactly the engines the `engine` input offers, in one `case` \
             list: any other name could build a tag that is not an engine's",
        );
    }
    if accepted.iter().any(|name| name.starts_with('v')) {
        lint.report("job `guard` accepts an engine named like a version (`v...`)");
    }
}

/// The `engine` choice input's options.
fn dispatch_engine_options(workflow: &Hash) -> Vec<&str> {
    let mut node = Some(workflow);
    for key in ["on", "workflow_dispatch", "inputs", "engine"] {
        node = match node.and_then(|map| get(map, key)) {
            Some(Yaml::Hash(next)) => Some(next),
            _ => None,
        };
    }
    match node.and_then(|engine| get(engine, "options")) {
        Some(Yaml::Array(options)) => options.iter().filter_map(Yaml::as_str).collect(),
        _ => Vec::new(),
    }
}

fn check_publish_script(lint: &mut Lint<'_>, publish: &Hash) {
    let commands = commands(publish);
    for (meaning, required) in PUBLISH_CHECKS {
        if !commands.iter().any(|command| command == required) {
            lint.report(&format!(
                "job `publish` no longer re-checks {meaning} after the approval; the check \
                 `{required}` must stand as one whole command"
            ));
        }
    }
    if !commands.iter().any(|command| {
        command.starts_with("tagged=\"$(gh api \"repos/smormah/vsift/git/ref/tags/${ENGINE_TAG}\"")
    }) {
        lint.report("job `publish` no longer reads the tag's commit from GitHub for its re-check");
    }
    if commands
        .iter()
        .any(|command| command.contains("refs/tags/v"))
    {
        lint.report("job `publish` names a `refs/tags/v` ref; it never accepts a `v*` tag");
    }
}

/// Every step that uses one of the guard's inputs binds it to the one
/// expression for it: the tag from the guard's output, never `github.ref_name`
/// or an input.
fn check_bound_inputs(lint: &mut Lint<'_>, id: &str, job: &Hash) {
    let job_env = match get(job, "env") {
        Some(Yaml::Hash(env)) => Some(env),
        _ => None,
    };
    for step in steps(job) {
        let step_env = match get(step, "env") {
            Some(Yaml::Hash(env)) => Some(env),
            _ => None,
        };
        let script = get(step, "run").and_then(Yaml::as_str).unwrap_or_default();
        for (name, expression) in BOUND_INPUTS {
            let bound = step_env
                .and_then(|env| get(env, name))
                .or_else(|| job_env.and_then(|env| get(env, name)));
            match bound {
                Some(value) if value.as_str() != Some(expression) => lint.report(&format!(
                    "job `{id}` binds `{name}` to something other than `{expression}`"
                )),
                None if script.contains(&format!("${{{name}}}")) => lint.report(&format!(
                    "job `{id}` uses `{name}` without binding it to `{expression}`"
                )),
                _ => {}
            }
        }
    }
}
