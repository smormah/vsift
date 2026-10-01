//! The governance workflow lint (R-SEC01, SEC-22; ADR 0023 section 1).
//!
//! Every workflow under `.github/workflows` is parsed into a YAML tree, not
//! scanned line by line: a flow mapping (`permissions: { id-token: write }`)
//! or an alias would otherwise hide a grant or an unpinned action from the
//! check. Comments are not part of the tree, so a rule never fires on prose.
//!
//! The rules, each fail-closed (a value of an unexpected shape is a finding):
//!
//! 1. every action and reusable workflow is pinned to a full commit SHA, a
//!    local path, or a container image digest;
//! 2. no workflow is triggered by `pull_request_target`, which runs with the
//!    base repository's token and secrets on pull-request events;
//! 3. every workflow declares top-level `permissions` that grant nothing but
//!    `read` or `none`; a job that needs a write scope names it itself, and
//!    `read-all` and `write-all` are refused everywhere;
//! 4. `id-token: write` appears only in the release workflow's `attest` and
//!    `publish` jobs;
//! 5. a `run` script interpolates only expressions whose values an outsider
//!    cannot choose (`runner`, `matrix`, `strategy`, `job` and a closed list of
//!    `github` fields); anything else reaches the script through `env`;
//! 6. the release workflow builds only the `vsift` binary of `vsift-cli`, in
//!    the release profile, with `--locked` and without any feature selection,
//!    and names no development feature, test binary or profile override
//!    anywhere, so no test-only behaviour can reach a shipped archive;
//! 7. the release workflow publishes only as the `publish` module describes
//!    (P13 PR 10): from a dispatch of the release tag with `dry_run` cleared,
//!    in the protected `release` environment, with npm provenance under
//!    `next`, and exactly the tarballs the qualification installed.

mod publish;

use std::{fs, path::Path};

use yaml_rust2::{Yaml, YamlLoader, yaml::Hash};

/// The directory whose workflows are linted, relative to the repository root.
pub(crate) const WORKFLOW_DIRECTORY: &str = ".github/workflows";

/// The release workflow, the only one allowed an OIDC token (in PR 10's
/// `attest` and `publish` jobs) and the one the release-build rule applies to.
pub(crate) const RELEASE_WORKFLOW: &str = ".github/workflows/release.yml";

/// Jobs of the release workflow that may request `id-token: write`
/// (ADR 0023: the Sigstore attestation and the npm trusted publish).
const ID_TOKEN_JOBS: [&str; 2] = ["attest", "publish"];

/// Text that must not appear in any key or value of the release workflow.
///
/// The development features weaken a build on request (ADR 0020, ADR 0023),
/// the two binaries are test and qualification tools, and a profile override
/// could turn on the debug assertions without which the features refuse to
/// compile. Comments are not searched.
const RELEASE_FORBIDDEN_TEXT: [&str; 7] = [
    "fault-injection",
    "durability-campaign",
    "install-test-hooks",
    "vsift-smoke-fixture",
    "vsift-crash-campaign",
    "CARGO_PROFILE_",
    "debug-assertions",
];

/// Arguments that select features, packages or targets beyond the `vsift`
/// binary; refused in the release workflow's cargo commands.
const RELEASE_FORBIDDEN_CARGO_ARGUMENTS: [&str; 11] = [
    "--features",
    "-F",
    "--all-features",
    "--no-default-features",
    "--workspace",
    "--all",
    "--bins",
    "--all-targets",
    "--examples",
    "--tests",
    "--profile",
];

/// `github` context fields a `run` script may interpolate: none of them can be
/// chosen by the author of a pull request or a branch name.
const SAFE_GITHUB_FIELDS: [&str; 9] = [
    "github.sha",
    "github.run_id",
    "github.run_number",
    "github.run_attempt",
    "github.repository",
    "github.repository_id",
    "github.workspace",
    "github.server_url",
    "github.event_name",
];

/// Contexts whose every field a `run` script may interpolate.
const SAFE_CONTEXT_ROOTS: [&str; 4] = ["runner", "matrix", "strategy", "job"];

/// Checks every workflow of the repository and appends one message per finding.
pub(crate) fn validate_workflows(messages: &mut Vec<String>, root: &Path) {
    let directory = root.join(WORKFLOW_DIRECTORY);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) => {
            messages.push(format!("{WORKFLOW_DIRECTORY} could not be listed: {error}"));
            return;
        }
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|extension| extension == "yml" || extension == "yaml")
        })
        .collect();
    names.sort();
    if !names.iter().any(|name| name == "release.yml") {
        messages.push(format!(
            "{RELEASE_WORKFLOW} is missing; the release build rule has nothing to hold"
        ));
    }
    for name in names {
        let relative = format!("{WORKFLOW_DIRECTORY}/{name}");
        match fs::read_to_string(root.join(&relative)) {
            Ok(text) => check_workflow(messages, &relative, &text),
            Err(error) => messages.push(format!("{relative} could not be read: {error}")),
        }
    }
}

/// Checks one workflow's text; `relative_path` names it in messages and
/// decides whether the release rules apply.
pub(crate) fn check_workflow(messages: &mut Vec<String>, relative_path: &str, text: &str) {
    let documents = match YamlLoader::load_from_str(text) {
        Ok(documents) => documents,
        Err(error) => {
            messages.push(format!("{relative_path} is not valid YAML: {error}"));
            return;
        }
    };
    let [Yaml::Hash(workflow)] = documents.as_slice() else {
        messages.push(format!(
            "{relative_path} must hold exactly one YAML document whose root is a mapping"
        ));
        return;
    };
    let mut lint = Lint {
        messages,
        path: relative_path,
        release: relative_path == RELEASE_WORKFLOW,
    };
    lint.refuse_merge_keys(workflow);
    lint.check_triggers(workflow);
    lint.check_top_level_permissions(workflow);
    lint.check_jobs(workflow);
    if lint.release {
        lint.check_release_text(workflow);
        publish::check_publishing(&mut lint, workflow);
    }
}

struct Lint<'a> {
    messages: &'a mut Vec<String>,
    path: &'a str,
    release: bool,
}

impl Lint<'_> {
    fn report(&mut self, finding: &str) {
        self.messages.push(format!("{}: {finding}", self.path));
    }

    /// YAML merge keys copy a mapping into another, which a reader of the
    /// workflow does not see where the key is used; refuse them outright.
    fn refuse_merge_keys(&mut self, workflow: &Hash) {
        let mut found = false;
        visit_hashes(workflow, &mut |hash| {
            if hash.contains_key(&Yaml::String(String::from("<<"))) {
                found = true;
            }
        });
        if found {
            self.report("uses a YAML merge key (`<<`), which the lint refuses");
        }
    }

    fn check_triggers(&mut self, workflow: &Hash) {
        let Some(triggers) = get(workflow, "on") else {
            self.report("declares no `on` triggers");
            return;
        };
        let names: Vec<&str> = match triggers {
            Yaml::String(name) => vec![name.as_str()],
            Yaml::Array(items) => items.iter().filter_map(Yaml::as_str).collect(),
            Yaml::Hash(map) => map.keys().filter_map(Yaml::as_str).collect(),
            _ => {
                self.report("has `on` triggers of an unexpected shape");
                return;
            }
        };
        if names.contains(&"pull_request_target") {
            self.report(
                "is triggered by `pull_request_target`, which runs pull-request events with \
                 the base repository's token and secrets (R-SEC01)",
            );
        }
    }

    fn check_top_level_permissions(&mut self, workflow: &Hash) {
        match get(workflow, "permissions") {
            None => self.report(
                "declares no top-level `permissions`, so its jobs would receive the \
                 repository's default token scopes; declare `permissions: {}` or read scopes",
            ),
            Some(permissions) => {
                for (scope, level) in self.permission_entries(permissions, "the workflow") {
                    if level == "write" {
                        self.report(&format!(
                            "grants `{scope}: write` to every job; grant a write scope only \
                             on the job that needs it"
                        ));
                    }
                }
            }
        }
    }

    /// Reads a `permissions` value into `(scope, level)` pairs, reporting any
    /// shorthand or unexpected value.
    fn permission_entries(&mut self, permissions: &Yaml, owner: &str) -> Vec<(String, String)> {
        match permissions {
            Yaml::Hash(map) => {
                let mut entries = Vec::new();
                for (scope, level) in map {
                    match (scope.as_str(), level.as_str()) {
                        (Some(scope), Some(level @ ("read" | "write" | "none"))) => {
                            entries.push((scope.to_owned(), level.to_owned()));
                        }
                        _ => self.report(&format!(
                            "{owner} has a `permissions` entry that is not `<scope>: \
                             read|write|none`"
                        )),
                    }
                }
                entries
            }
            Yaml::String(shorthand) => {
                self.report(&format!(
                    "{owner} uses `permissions: {shorthand}`; name each scope it needs instead"
                ));
                Vec::new()
            }
            _ => {
                self.report(&format!("{owner} has `permissions` of an unexpected shape"));
                Vec::new()
            }
        }
    }

    fn check_jobs(&mut self, workflow: &Hash) {
        let Some(Yaml::Hash(jobs)) = get(workflow, "jobs") else {
            self.report("has no `jobs` mapping");
            return;
        };
        let mut cargo_builds = 0_usize;
        for (id, job) in jobs {
            let (Some(id), Yaml::Hash(job)) = (id.as_str(), job) else {
                self.report("has a job that is not a named mapping");
                continue;
            };
            let owner = format!("job `{id}`");
            if let Some(permissions) = get(job, "permissions") {
                for (scope, level) in self.permission_entries(permissions, &owner) {
                    if scope == "id-token"
                        && level == "write"
                        && !(self.release && ID_TOKEN_JOBS.contains(&id))
                    {
                        self.report(&format!(
                            "{owner} requests `id-token: write`; only the release workflow's \
                             `attest` and `publish` jobs may (ADR 0023)"
                        ));
                    }
                }
            }
            if let Some(uses) = get(job, "uses") {
                self.check_uses(&owner, uses);
            }
            match get(job, "steps") {
                None => {}
                Some(Yaml::Array(steps)) => {
                    for (index, step) in steps.iter().enumerate() {
                        let step_owner = format!("{owner} step {}", index + 1);
                        let Yaml::Hash(step) = step else {
                            self.report(&format!("{step_owner} is not a mapping"));
                            continue;
                        };
                        if let Some(uses) = get(step, "uses") {
                            self.check_uses(&step_owner, uses);
                        }
                        match get(step, "run") {
                            None => {}
                            Some(Yaml::String(script)) => {
                                self.check_interpolation(&step_owner, script);
                                if self.release {
                                    cargo_builds += self.check_release_cargo(&step_owner, script);
                                }
                            }
                            Some(_) => {
                                self.report(&format!("{step_owner} has a `run` that is not text"));
                            }
                        }
                    }
                }
                Some(_) => self.report(&format!("{owner} has `steps` that are not a list")),
            }
        }
        if self.release && cargo_builds == 0 {
            self.report("builds nothing with `cargo build`; the release build rule cannot hold");
        }
    }

    fn check_uses(&mut self, owner: &str, uses: &Yaml) {
        match uses.as_str() {
            Some(reference) if is_pinned_reference(reference) => {}
            Some(reference) => self.report(&format!(
                "{owner} uses `{reference}`, which is not pinned to a full commit SHA \
                 (R-SEC01)"
            )),
            None => self.report(&format!("{owner} has a `uses` that is not text")),
        }
    }

    fn check_interpolation(&mut self, owner: &str, script: &str) {
        for expression in expressions(script) {
            for reference in context_references(expression) {
                if !is_safe_reference(&reference) {
                    self.report(&format!(
                        "{owner} interpolates `{reference}` into its script; pass it through \
                         `env` and quote the variable instead"
                    ));
                }
            }
        }
    }

    /// Checks the release workflow's cargo commands in one script and returns
    /// how many `cargo build` commands it holds.
    fn check_release_cargo(&mut self, owner: &str, script: &str) -> usize {
        let mut builds = 0;
        for command in logical_lines(script) {
            let words: Vec<&str> = command.split_whitespace().collect();
            let Some(position) = words.iter().position(|word| *word == "cargo") else {
                continue;
            };
            let arguments = words.get(position + 1..).unwrap_or_default();
            let subcommand = arguments.first().copied().unwrap_or_default();
            if subcommand == "install" {
                // Installs a pinned tool; it cannot change the vsift build.
                continue;
            }
            for argument in arguments {
                let name = argument.split('=').next().unwrap_or_default();
                if RELEASE_FORBIDDEN_CARGO_ARGUMENTS.contains(&name)
                    || (argument.starts_with("-F") && argument.len() > 2)
                {
                    self.report(&format!(
                        "{owner} passes `{name}` to `cargo {subcommand}`; the release build \
                         takes no feature, package or profile selection"
                    ));
                }
            }
            if subcommand == "build" || subcommand == "rustc" {
                builds += 1;
                let has_pair = |flag: &str, value: &str| {
                    arguments
                        .windows(2)
                        .any(|pair| pair.first() == Some(&flag) && pair.get(1) == Some(&value))
                        || arguments.contains(&format!("{flag}={value}").as_str())
                };
                let complete = arguments.contains(&"--release")
                    && arguments.contains(&"--locked")
                    && has_pair("--bin", "vsift")
                    && (has_pair("-p", "vsift-cli") || has_pair("--package", "vsift-cli"));
                if !complete {
                    self.report(&format!(
                        "{owner} runs `cargo {subcommand}` without all of `--release`, \
                         `--locked`, `-p vsift-cli` and `--bin vsift`"
                    ));
                }
            }
        }
        builds
    }

    fn check_release_text(&mut self, workflow: &Hash) {
        let mut found: Vec<&str> = Vec::new();
        visit_strings(workflow, &mut |text| {
            for forbidden in RELEASE_FORBIDDEN_TEXT {
                if text.contains(forbidden) && !found.contains(&forbidden) {
                    found.push(forbidden);
                }
            }
        });
        for forbidden in found {
            self.report(&format!(
                "names `{forbidden}`; a release build never involves a development feature, \
                 a test binary or a profile override (ADR 0020, ADR 0023)"
            ));
        }
    }
}

fn get<'a>(map: &'a Hash, key: &str) -> Option<&'a Yaml> {
    map.get(&Yaml::String(key.to_owned()))
}

fn visit_hashes(map: &Hash, visitor: &mut impl FnMut(&Hash)) {
    visitor(map);
    for (key, value) in map {
        visit_value_hashes(key, visitor);
        visit_value_hashes(value, visitor);
    }
}

fn visit_value_hashes(value: &Yaml, visitor: &mut impl FnMut(&Hash)) {
    match value {
        Yaml::Hash(map) => visit_hashes(map, visitor),
        Yaml::Array(items) => {
            for item in items {
                visit_value_hashes(item, visitor);
            }
        }
        _ => {}
    }
}

fn visit_strings(map: &Hash, visitor: &mut impl FnMut(&str)) {
    for (key, value) in map {
        visit_value_strings(key, visitor);
        visit_value_strings(value, visitor);
    }
}

fn visit_value_strings(value: &Yaml, visitor: &mut impl FnMut(&str)) {
    match value {
        Yaml::String(text) | Yaml::Real(text) => visitor(text),
        Yaml::Hash(map) => visit_strings(map, visitor),
        Yaml::Array(items) => {
            for item in items {
                visit_value_strings(item, visitor);
            }
        }
        _ => {}
    }
}

/// Whether a `uses` value is a local path, an image digest, or an action or
/// reusable workflow at a full 40-character commit SHA.
fn is_pinned_reference(reference: &str) -> bool {
    if reference.starts_with("./") {
        return true;
    }
    if let Some(image) = reference.strip_prefix("docker://") {
        return image
            .rsplit_once("@sha256:")
            .is_some_and(|(name, digest)| !name.is_empty() && is_lower_hex(digest, 64));
    }
    let Some((action, revision)) = reference.rsplit_once('@') else {
        return false;
    };
    let mut parts = action.split('/');
    let owner = parts.next().unwrap_or_default();
    let repository = parts.next().unwrap_or_default();
    !owner.is_empty()
        && !repository.is_empty()
        && parts.all(|part| !part.is_empty())
        && is_lower_hex(revision, 40)
}

fn is_lower_hex(text: &str, length: usize) -> bool {
    text.len() == length
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The bodies of the `${{ ... }}` expressions in a script. An unterminated
/// expression yields the rest of the script, so it is checked rather than
/// skipped.
fn expressions(script: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = script;
    while let Some(start) = rest.find("${{") {
        let body = rest.get(start + 3..).unwrap_or_default();
        let Some(end) = body.find("}}") else {
            found.push(body);
            break;
        };
        found.push(body.get(..end).unwrap_or_default());
        rest = body.get(end + 2..).unwrap_or_default();
    }
    found
}

/// The context references (`github.head_ref`, `inputs.name`, `matrix.os`) of
/// one expression body: dotted names outside string literals that are not
/// function calls or literals.
fn context_references(expression: &str) -> Vec<String> {
    let mut references = Vec::new();
    let characters: Vec<char> = expression.chars().collect();
    let mut index = 0;
    while let Some(&character) = characters.get(index) {
        if character == '\'' {
            index += 1;
            while let Some(&inner) = characters.get(index) {
                index += 1;
                if inner == '\'' {
                    if characters.get(index) == Some(&'\'') {
                        index += 1;
                    } else {
                        break;
                    }
                }
            }
            continue;
        }
        if character.is_ascii_alphabetic() || character == '_' {
            let start = index;
            while characters.get(index).is_some_and(|next| {
                next.is_ascii_alphanumeric() || matches!(next, '_' | '-' | '.' | '*')
            }) {
                index += 1;
            }
            let name: String = characters
                .get(start..index)
                .unwrap_or_default()
                .iter()
                .collect();
            let mut lookahead = index;
            while characters
                .get(lookahead)
                .is_some_and(|next| next.is_whitespace())
            {
                lookahead += 1;
            }
            let call = characters.get(lookahead) == Some(&'(');
            let literal = matches!(name.as_str(), "true" | "false" | "null");
            if !call && !literal {
                references.push(name);
            }
            continue;
        }
        if character == '[' {
            // An index such as matrix['os'] or github.event['x'] is read as a
            // reference to its container, which the path before it names.
            index += 1;
            continue;
        }
        index += 1;
    }
    references
}

fn is_safe_reference(reference: &str) -> bool {
    let root = reference.split('.').next().unwrap_or_default();
    SAFE_CONTEXT_ROOTS.contains(&root) || SAFE_GITHUB_FIELDS.contains(&reference)
}

/// Splits a script into commands, joining lines that end in a backslash.
fn logical_lines(script: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for line in script.lines() {
        let trimmed = line.trim_end();
        if let Some(continued) = trimmed.strip_suffix('\\') {
            current.push_str(continued);
            current.push(' ');
        } else {
            current.push_str(trimmed);
            lines.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::{RELEASE_WORKFLOW, check_workflow, context_references, is_pinned_reference};

    const CHECKOUT: &str = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1";

    fn findings(path: &str, text: &str) -> Vec<String> {
        let mut messages = Vec::new();
        check_workflow(&mut messages, path, text);
        messages
    }

    fn workflow(permissions: &str, jobs: &str) -> String {
        format!("name: Example\non:\n  pull_request:\n{permissions}jobs:\n{jobs}")
    }

    const GOOD_BUILD: &str =
        "cargo build --release --locked -p vsift-cli --bin vsift --target \"${TARGET}\"";

    /// The repository's release workflow with its first build step running
    /// `build` and its second one building nothing, so a test sees the
    /// release rules' findings for `build` alone: the real workflow has none
    /// (the publishing rules need its whole shape, P13 PR 10).
    fn release_workflow(build: &str) -> String {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(RELEASE_WORKFLOW),
        )
        .unwrap_or_default();
        let build_step = format!("run: {GOOD_BUILD}\n");
        text.replacen(&build_step, &format!("run: |\n          {build}\n"), 1)
            .replacen(&build_step, "run: echo rebuilt\n", 1)
    }

    #[test]
    fn a_pinned_least_privilege_workflow_passes() {
        let text = workflow(
            "permissions:\n  contents: read\n",
            &format!(
                "  test:\n    runs-on: ubuntu-latest\n    permissions:\n      contents: read\n      \
                 security-events: write\n    steps:\n      - uses: {CHECKOUT} # v7.0.1\n      \
                 - uses: ./.github/actions/local\n      - uses: docker://alpine@sha256:{}\n      \
                 - run: echo \"${{{{ runner.temp }}}}\" \"${{{{ matrix.os }}}}\" \"${{{{ github.sha }}}}\"\n",
                "a".repeat(64)
            ),
        );
        assert_eq!(
            findings(".github/workflows/ci.yml", &text),
            Vec::<String>::new()
        );
    }

    #[test]
    fn unpinned_actions_are_refused() {
        for reference in [
            "actions/checkout@v4",
            "actions/checkout@main",
            "actions/checkout@3d3c42e",
            "actions/checkout@3D3C42E5AAC5BA805825DA76410C181273BA90B1",
            "actions/checkout",
            "docker://alpine:3.20",
            "owner/repo/.github/workflows/x.yml@v1",
        ] {
            let text = workflow(
                "permissions: {}\n",
                &format!(
                    "  test:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: {reference}\n"
                ),
            );
            let messages = findings(".github/workflows/ci.yml", &text);
            assert_eq!(messages.len(), 1, "{reference}: {messages:#?}");
            assert!(
                messages
                    .iter()
                    .all(|message| message.contains("not pinned")),
                "{messages:#?}"
            );
        }
        let reusable = workflow(
            "permissions: {}\n",
            "  call:\n    uses: owner/repo/.github/workflows/x.yml@v1\n",
        );
        assert_eq!(findings(".github/workflows/ci.yml", &reusable).len(), 1);
        assert!(is_pinned_reference(
            "owner/repo/.github/workflows/x.yml@3d3c42e5aac5ba805825da76410c181273ba90b1"
        ));
    }

    #[test]
    fn pull_request_target_is_refused_in_every_trigger_form() {
        for triggers in [
            "on: pull_request_target\n",
            "on: [push, pull_request_target]\n",
            "on:\n  pull_request_target:\n    types: [opened]\n",
            "on: {pull_request_target: {}}\n",
        ] {
            let text = format!(
                "name: X\n{triggers}permissions: {{}}\njobs:\n  a:\n    runs-on: ubuntu-latest\n    \
                 steps:\n      - run: echo hi\n"
            );
            let messages = findings(".github/workflows/x.yml", &text);
            assert_eq!(messages.len(), 1, "{triggers}: {messages:#?}");
            assert!(
                messages
                    .iter()
                    .all(|message| message.contains("pull_request_target"))
            );
        }
    }

    #[test]
    fn missing_or_broad_top_level_permissions_are_refused() {
        let job = "  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n";
        for permissions in [
            "",
            "permissions: write-all\n",
            "permissions: read-all\n",
            "permissions:\n  contents: write\n",
            "permissions: {contents: read, packages: write}\n",
            "permissions:\n  contents: admin\n",
        ] {
            let messages = findings(".github/workflows/x.yml", &workflow(permissions, job));
            assert_eq!(messages.len(), 1, "{permissions}: {messages:#?}");
        }
        let job_shorthand = workflow(
            "permissions: {}\n",
            "  a:\n    runs-on: ubuntu-latest\n    permissions: write-all\n    steps:\n      \
             - run: echo hi\n",
        );
        assert_eq!(findings(".github/workflows/x.yml", &job_shorthand).len(), 1);
    }

    #[test]
    fn id_token_is_granted_only_to_the_release_attest_and_publish_jobs() {
        let grant = |job: &str, form: &str| {
            workflow(
                "permissions: {}\n",
                &format!(
                    "  {job}:\n    runs-on: ubuntu-latest\n    permissions:{form}\n    steps:\n      - run: echo hi\n"
                ),
            )
        };
        let block = "\n      id-token: write";
        let flow = " { id-token: write }";
        for form in [block, flow] {
            assert_eq!(
                findings(".github/workflows/ci.yml", &grant("attest", form)).len(),
                1
            );
            assert_eq!(
                findings(".github/workflows/ci.yml", &grant("build", form)).len(),
                1
            );
            // The release workflow's rules add a finding for having no build,
            // so only the id-token finding is counted here.
            let release_build = findings(RELEASE_WORKFLOW, &grant("build", form));
            assert_eq!(
                release_build
                    .iter()
                    .filter(|message| message.contains("id-token"))
                    .count(),
                1,
                "{release_build:#?}"
            );
            for allowed in ["attest", "publish"] {
                let messages = findings(RELEASE_WORKFLOW, &grant(allowed, form));
                assert!(
                    messages.iter().all(|message| !message.contains("id-token")),
                    "{messages:#?}"
                );
            }
        }
        let top_level = workflow(
            "permissions:\n  id-token: write\n",
            "  attest:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        );
        assert_eq!(findings(".github/workflows/ci.yml", &top_level).len(), 1);
    }

    #[test]
    fn an_alias_cannot_hide_a_grant_and_merge_keys_are_refused() {
        let aliased = "name: X\non: push\npermissions: {}\nx-grant: &grant\n  id-token: write\n\
                       jobs:\n  a:\n    runs-on: ubuntu-latest\n    permissions: *grant\n    \
                       steps:\n      - run: echo hi\n";
        let messages = findings(".github/workflows/x.yml", aliased);
        assert!(
            messages.iter().any(|message| message.contains("id-token")),
            "{messages:#?}"
        );
        let merged = "name: X\non: push\npermissions: {}\nx-grant: &grant\n  contents: read\n\
                      jobs:\n  a:\n    runs-on: ubuntu-latest\n    permissions:\n      <<: *grant\n    \
                      steps:\n      - run: echo hi\n";
        let messages = findings(".github/workflows/x.yml", merged);
        assert!(
            messages.iter().any(|message| message.contains("merge key")),
            "{messages:#?}"
        );
    }

    #[test]
    fn untrusted_expressions_in_run_scripts_are_refused() {
        for expression in [
            "github.event.pull_request.title",
            "github.head_ref",
            "inputs.seconds",
            "steps.duration.outputs.seconds",
            "env.VALUE",
            "secrets.TOKEN",
            "github.ref_name",
            "format('{0}', github.event.issue.body)",
            "toJSON(github.event)",
        ] {
            let text = workflow(
                "permissions: {}\n",
                &format!(
                    "  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo \"${{{{ {expression} }}}}\"\n"
                ),
            );
            let messages = findings(".github/workflows/x.yml", &text);
            assert_eq!(messages.len(), 1, "{expression}: {messages:#?}");
        }
        let env_passing = workflow(
            "permissions: {}\n",
            "  a:\n    runs-on: ubuntu-latest\n    steps:\n      - env:\n          \
             TITLE: ${{ github.event.pull_request.title }}\n        run: echo \"${TITLE}\"\n",
        );
        assert_eq!(
            findings(".github/workflows/x.yml", &env_passing),
            Vec::<String>::new()
        );
    }

    #[test]
    fn expression_references_skip_literals_and_function_names() {
        assert_eq!(
            context_references("matrix.mode == 'negative' && '80' || inputs.runs || 'git.hub'"),
            vec![String::from("matrix.mode"), String::from("inputs.runs")]
        );
        assert_eq!(
            context_references("contains(github.event.head_commit.message, 'it''s')"),
            vec![String::from("github.event.head_commit.message")]
        );
        assert_eq!(context_references("true && null"), Vec::<String>::new());
    }

    #[test]
    fn the_release_build_rule_accepts_only_the_plain_vsift_binary() {
        assert_eq!(
            findings(RELEASE_WORKFLOW, &release_workflow(GOOD_BUILD)),
            Vec::<String>::new()
        );
        let continued = "cargo build --release --locked \\\n            -p vsift-cli --bin vsift";
        assert_eq!(
            findings(RELEASE_WORKFLOW, &release_workflow(continued)),
            Vec::<String>::new()
        );
        for build in [
            "cargo build --release --locked -p vsift-cli --bin vsift --features x",
            "cargo build --release --locked -p vsift-cli --bin vsift --features=x",
            "cargo build --release --locked -p vsift-cli --bin vsift -Fx",
            "cargo build --release --locked -p vsift-cli --bin vsift --all-features",
            "cargo build --release --locked -p vsift-cli --bin vsift --profile campaign",
            "cargo build --release --locked --workspace --bin vsift -p vsift-cli",
            "cargo build --locked -p vsift-cli --bin vsift",
            "cargo build --release -p vsift-cli --bin vsift",
            "cargo build --release --locked -p vsift-infrastructure --bin vsift",
            "cargo build --release --locked -p vsift-cli",
            "cargo run --locked -p vsift-release --features x",
        ] {
            let messages = findings(
                RELEASE_WORKFLOW,
                &release_workflow(&format!("{GOOD_BUILD}\n          {build}")),
            );
            assert_eq!(messages.len(), 1, "{build}: {messages:#?}");
        }
        let install = format!(
            "cargo install --locked --version =0.9.2 --features cli cargo-about\n          {GOOD_BUILD}"
        );
        assert_eq!(
            findings(RELEASE_WORKFLOW, &release_workflow(&install)),
            Vec::<String>::new()
        );
        let nothing_built = release_workflow("echo no build");
        assert_eq!(findings(RELEASE_WORKFLOW, &nothing_built).len(), 1);
    }

    #[test]
    fn the_release_workflow_names_no_development_feature_or_test_binary() {
        for text in [
            "fault-injection",
            "vsift-infrastructure/durability-campaign",
            "install-test-hooks",
            "vsift-smoke-fixture",
            "vsift-crash-campaign",
            "CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS",
        ] {
            let with_env = release_workflow(GOOD_BUILD).replacen(
                "env:\n  CARGO_TERM_COLOR: never\n",
                &format!("env:\n  EXTRA: \"{text}\"\n  CARGO_TERM_COLOR: never\n"),
                1,
            );
            let messages = findings(RELEASE_WORKFLOW, &with_env);
            assert_eq!(messages.len(), 1, "{text}: {messages:#?}");
        }
        let commented = release_workflow(GOOD_BUILD).replacen(
            "\njobs:\n",
            "\n# never fault-injection or vsift-smoke-fixture\njobs:\n",
            1,
        );
        assert_eq!(findings(RELEASE_WORKFLOW, &commented), Vec::<String>::new());
        // The rule is the release workflow's: CI may build the whole workspace.
        let ci = workflow(
            "permissions: {}\n",
            &format!(
                "  build:\n    runs-on: ubuntu-24.04\n    steps:\n      - uses: {CHECKOUT}\n      \
                 - run: cargo build --workspace --release --locked --all-features\n"
            ),
        );
        assert_eq!(
            findings(".github/workflows/ci.yml", &ci),
            Vec::<String>::new()
        );
    }

    #[test]
    fn malformed_workflows_are_findings_not_crashes() {
        for text in ["", "- a\n- b\n", "a: [\n", "name: X\n---\nname: Y\n"] {
            assert!(
                !findings(".github/workflows/x.yml", text).is_empty(),
                "{text:?}"
            );
        }
    }
}
