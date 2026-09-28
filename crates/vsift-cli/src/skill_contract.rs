//! Contract guard for the agent skill in `skills/vsift` (P12, ADR 0022).
//!
//! The skill is prose and JSON that orchestrate the published CLI. Nothing
//! compiles it, so without this guard a renamed flag, a new command or a new
//! failure code would silently leave agents following stale instructions.
//! These tests read the skill's files and hold them to the real parser
//! ([`Cli`]), the published command and failure-code lists, the v1 schemas
//! and the skill's own handoff schema.
//!
//! The module lives inside the crate, rather than in `tests/`, because the
//! parser is crate-private; the same reason the contract-names test sits in
//! `command.rs`.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use clap::{CommandFactory, Parser};
use serde_json::Value;
use vsift::FailureCode;
use vsift_contract::CommandName;

use crate::command::{Cli, HostIsolationArgument};

/// The eight states of the investigation procedure, in order. They are the
/// only upper-case snake identifiers the skill may use besides failure codes.
const STATES: [&str; 8] = [
    "CHECK_CAPABILITIES",
    "PREPARE",
    "FIND_SPOKEN_SPANS",
    "INSPECT_CARDS",
    "VERIFY_SOURCE",
    "REFINE_OR_STOP",
    "REPORT",
    "CLOSE_OR_RETAIN",
];

/// The code word printed in `assets/image-check.png`, kept in two parts so a
/// plain search of the repository for the joined word does not find it. An
/// agent must read it from the pixels; if any skill text contained it, a
/// model could "pass" the image check without seeing the image.
const IMAGE_CODE_PARTS: [&str; 2] = ["OK", "API 6281"];

/// Upper bound on `SKILL.md`, which every client loads whole into its context.
const MAX_SKILL_LINES: usize = 300;

/// Upper bound on the check image, which a one-image client pays for.
const MAX_IMAGE_CHECK_BYTES: usize = 8 * 1024;

/// Upper bound on a serialized resume card (`resume` in the handoff).
const MAX_RESUME_BYTES: usize = 2 * 1024;

/// Placeholders the skill's command lines use, with a value the parser
/// accepts. An unknown placeholder fails the guard, so a new one is added here
/// deliberately.
const PLACEHOLDERS: [(&str, &str); 21] = [
    ("video", "recording.mp4"),
    ("transcript", "recording.srt"),
    ("offset-us", "500000"),
    ("session", "ses_0123456789abcdef"),
    ("text", "R-17"),
    ("n", "5"),
    ("from-us", "0"),
    ("to-us", "10000000"),
    ("us", "5000000"),
    ("candidate", "vcd_0123456789abcdef"),
    ("evidence", "evd_0123456789abcdef"),
    ("rect", "10,20,300,80"),
    ("job", "job_0123456789abcdef"),
    ("operation-id", "op_0123456789abcdef"),
    ("revision", "trv_0123456789abcdef"),
    ("cursor", "opaque-cursor"),
    ("new-directory", "evidence-bundle"),
    ("bundle-directory", "evidence-bundle"),
    ("executable", "ffmpeg-bin"),
    ("model-file", "ggml-base.bin"),
    ("dependency", "ffmpeg"),
];

/// Commands the skill must never run. Changing this set is a reviewed safety
/// decision (ADR 0022), not a documentation edit.
const NEVER: [&str; 8] = [
    "setup.install",
    "setup.repair",
    "setup.list",
    "setup.remove",
    "setup.rollback",
    "session.init-workspace",
    "job.run",
    "job.batch",
];

/// Commands the skill runs only on the user's explicit instruction.
const EXPLICIT: [&str; 6] = [
    "setup.configure",
    "setup.configure-model",
    "session.renew",
    "session.retain",
    "session.clean",
    "job.cancel",
];

/// Read-only forms of `explicit` commands that the skill may run freely: the
/// operation and the flag that makes the form read-only.
const FREE_FORMS: [(&str, &str); 1] = [("session.clean", "--dry-run")];

/// The sections of the handoff template, in order.
const HANDOFF_SECTIONS: [&str; 8] = [
    "## Problem",
    "## Expected",
    "## Actual",
    "## Reproduction steps",
    "## Evidence",
    "## Gaps and uncertainty",
    "## Untrusted instructions observed",
    "## Lifecycle",
];

/// Who may start a command (`references/commands.md`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Class {
    Free,
    Explicit,
    Never,
}

impl Class {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "free" => Some(Self::Free),
            "explicit" => Some(Self::Explicit),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

/// One fenced code block of a Markdown file.
struct Fence {
    info: String,
    /// The last non-empty prose line before the fence.
    preceded_by: String,
    lines: Vec<String>,
}

/// A Markdown file split into fenced blocks and the prose around them.
struct Markdown {
    name: String,
    fences: Vec<Fence>,
    prose: Vec<String>,
}

/// Collects problems; each test reports all of them at once.
#[derive(Default)]
struct Problems(Vec<String>);

impl Problems {
    fn add(&mut self, problem: String) {
        self.0.push(problem);
    }

    fn into_result(self) -> Result<(), String> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(self.0.join("\n"))
        }
    }
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn skill_directory() -> PathBuf {
    repository().join("skills").join("vsift")
}

fn read_text(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}

fn read_json(path: &Path) -> Result<Value, String> {
    serde_json::from_str(&read_text(path)?).map_err(|error| format!("{}: {error}", path.display()))
}

/// Every file below `directory`, recursively, in a stable order.
fn files_below(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut pending = vec![directory.to_path_buf()];
    let mut files = Vec::new();
    while let Some(current) = pending.pop() {
        let entries =
            fs::read_dir(&current).map_err(|error| format!("{}: {error}", current.display()))?;
        for entry in entries {
            let path = entry
                .map_err(|error| format!("{}: {error}", current.display()))?
                .path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension().is_some_and(|value| value == extension)
}

/// The skill's text files (everything but the check image) and the
/// contributor-facing skill document.
fn skill_text_files() -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<PathBuf> = files_below(&skill_directory())?
        .into_iter()
        .filter(|path| !has_extension(path, "png"))
        .collect();
    files.push(repository().join("docs").join("agents").join("skill.md"));
    Ok(files)
}

/// The Markdown files whose commands and terms the guard checks.
fn markdown_files() -> Result<Vec<Markdown>, String> {
    let mut documents = Vec::new();
    for path in skill_text_files()? {
        if has_extension(&path, "md") {
            let name = path
                .strip_prefix(repository())
                .unwrap_or(&path)
                .display()
                .to_string();
            documents.push(parse_markdown(name, &read_text(&path)?));
        }
    }
    Ok(documents)
}

/// Splits Markdown into fenced blocks and prose, following the `CommonMark` rule
/// that a fence closes only with at least as many backticks as opened it.
fn parse_markdown(name: String, text: &str) -> Markdown {
    let mut fences = Vec::new();
    let mut prose = Vec::new();
    let mut open: Option<(usize, Fence)> = None;
    let mut last_prose = String::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let ticks = trimmed.chars().take_while(|c| *c == '`').count();
        match open.take() {
            Some((opened, fence)) if ticks >= opened && trimmed[ticks..].trim().is_empty() => {
                fences.push(fence);
            }
            Some((opened, mut fence)) => {
                fence.lines.push(line.to_owned());
                open = Some((opened, fence));
            }
            None if ticks >= 3 => {
                open = Some((
                    ticks,
                    Fence {
                        info: trimmed[ticks..].trim().to_owned(),
                        preceded_by: last_prose.clone(),
                        lines: Vec::new(),
                    },
                ));
            }
            None => {
                if !line.trim().is_empty() {
                    line.trim().clone_into(&mut last_prose);
                }
                prose.push(line.to_owned());
            }
        }
    }
    if let Some((_, fence)) = open {
        fences.push(fence);
        prose.push(format!("{name}: unterminated fence"));
    }
    Markdown {
        name,
        fences,
        prose,
    }
}

/// The inline code spans of a document's prose. A span may wrap onto the next
/// line but never across a blank line; an unmatched backtick is a problem.
fn inline_spans(document: &Markdown, problems: &mut Problems) -> Vec<String> {
    let mut spans = Vec::new();
    for paragraph in document.prose.join("\n").split("\n\n") {
        let characters: Vec<char> = paragraph.chars().collect();
        let mut index = 0;
        while index < characters.len() {
            if characters[index] != '`' {
                index += 1;
                continue;
            }
            let run = characters[index..]
                .iter()
                .take_while(|c| **c == '`')
                .count();
            let start = index + run;
            let mut cursor = start;
            let mut closed = None;
            while cursor < characters.len() {
                if characters[cursor] == '`' {
                    let closing = characters[cursor..]
                        .iter()
                        .take_while(|c| **c == '`')
                        .count();
                    if closing == run {
                        closed = Some(cursor);
                        break;
                    }
                    cursor += closing;
                } else {
                    cursor += 1;
                }
            }
            if let Some(end) = closed {
                let span: String = characters[start..end].iter().collect();
                spans.push(span.replace('\n', " ").trim().to_owned());
                index = end + run;
            } else {
                problems.add(format!(
                    "{}: unmatched backtick in paragraph starting {:?}",
                    document.name,
                    paragraph.lines().next().unwrap_or_default()
                ));
                break;
            }
        }
    }
    spans
}

/// Replaces `<name>` placeholders with parser-valid values.
fn substitute(line: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut rest = line;
    while let Some(open) = rest.find('<') {
        output.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after
            .find('>')
            .ok_or_else(|| format!("unclosed placeholder in {line:?}"))?;
        let name = &after[..close];
        let value = PLACEHOLDERS
            .iter()
            .find(|(placeholder, _)| *placeholder == name)
            .map(|(_, value)| *value)
            .ok_or_else(|| format!("unknown placeholder <{name}> in {line:?}"))?;
        output.push_str(value);
        rest = &after[close + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

/// Splits a command line into arguments: whitespace separates, and single or
/// double quotes group without escapes, as the skill's examples are written.
fn split_arguments(line: &str) -> Result<Vec<String>, String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for character in line.chars() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => current.push(character),
            None if character == '"' || character == '\'' => {
                quote = Some(character);
                started = true;
            }
            None if character.is_whitespace() => {
                if started {
                    arguments.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            None => {
                current.push(character);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err(format!("unterminated quote in {line:?}"));
    }
    if started {
        arguments.push(current);
    }
    Ok(arguments)
}

/// Resolves the leading subcommand words of `vsift ...` arguments to the
/// public operation identifier and its leaf command.
fn resolve_operation(arguments: &[String]) -> Option<(String, clap::Command)> {
    let mut command = Cli::command();
    let mut path = Vec::new();
    for word in arguments.iter().skip(1) {
        let Some(next) = command.find_subcommand(word).cloned() else {
            break;
        };
        path.push(word.clone());
        command = next;
    }
    (!path.is_empty() && !command.has_subcommands()).then(|| (path.join("."), command))
}

fn long_flags(command: &clap::Command) -> BTreeSet<String> {
    command
        .get_arguments()
        .filter_map(clap::Arg::get_long)
        .map(|long| format!("--{long}"))
        .collect()
}

/// Every long flag of the parser, globals included.
fn every_long_flag() -> BTreeSet<String> {
    let mut flags = BTreeSet::new();
    let mut pending = vec![Cli::command()];
    while let Some(command) = pending.pop() {
        flags.extend(long_flags(&command));
        pending.extend(command.get_subcommands().cloned());
    }
    flags
}

/// The class table of `references/commands.md`: operation identifier to
/// class, with any table problem reported.
fn policy_table(problems: &mut Problems) -> Result<BTreeMap<String, Class>, String> {
    let text = read_text(&skill_directory().join("references").join("commands.md"))?;
    let mut table = BTreeMap::new();
    for line in text.lines() {
        let Some(row) = line.strip_prefix("| `vsift ") else {
            continue;
        };
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        let (Some(command), Some(class)) = (cells.first(), cells.get(1)) else {
            problems.add(format!("commands.md: malformed row {line:?}"));
            continue;
        };
        let identifier = command.trim_end_matches('`').replace(' ', ".");
        match Class::parse(class) {
            Some(class) => {
                if table.insert(identifier.clone(), class).is_some() {
                    problems.add(format!("commands.md classifies {identifier} twice"));
                }
            }
            None => problems.add(format!(
                "commands.md: unknown class {class:?} for {identifier}"
            )),
        }
    }
    Ok(table)
}

/// Checks one `vsift` command line from a console fence and returns its
/// operation identifier.
fn check_console_command(
    source: &str,
    line: &str,
    problems: &mut Problems,
) -> Result<Option<String>, String> {
    let arguments = split_arguments(&substitute(line)?)?;
    let cli = match Cli::try_parse_from(&arguments) {
        Ok(cli) => cli,
        Err(error) => {
            problems.add(format!(
                "{source}: {line:?} does not parse: {}",
                error.kind()
            ));
            return Ok(None);
        }
    };
    if cli.command.is_none() {
        problems.add(format!("{source}: {line:?} names no operation"));
    }
    if !cli.json && cli.events.is_none() {
        problems.add(format!(
            "{source}: {line:?} has neither --json nor --events"
        ));
    }
    if cli.session_root.is_some() || cli.host_isolation != HostIsolationArgument::ProcessOnly {
        problems.add(format!(
            "{source}: {line:?} uses an operator-only global option"
        ));
    }
    Ok(resolve_operation(&arguments).map(|(identifier, _)| identifier))
}

#[test]
fn skill_md_has_front_matter_states_and_a_size_bound() -> Result<(), String> {
    let text = read_text(&skill_directory().join("SKILL.md"))?;
    let mut problems = Problems::default();
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() > MAX_SKILL_LINES {
        problems.add(format!(
            "SKILL.md has {} lines, above {MAX_SKILL_LINES}",
            lines.len()
        ));
    }
    let front: Vec<&str> = if let Some((&"---", rest)) = lines.split_first() {
        rest.iter()
            .take_while(|line| **line != "---")
            .copied()
            .collect()
    } else {
        problems.add("SKILL.md does not start with YAML front matter".to_owned());
        Vec::new()
    };
    let keys: Vec<&str> = front
        .iter()
        .filter_map(|line| line.split_once(':').map(|(key, _)| key.trim()))
        .collect();
    if keys != ["name", "description"] {
        problems.add(format!(
            "front matter must hold exactly name and description (no tool grants), found {keys:?}"
        ));
    }
    if !front.contains(&"name: vsift") {
        problems.add("front matter name is not vsift".to_owned());
    }
    let description = front
        .iter()
        .find_map(|line| line.strip_prefix("description: "))
        .unwrap_or_default();
    if description.is_empty() || description.len() > 1024 || description.contains(['<', '>']) {
        problems.add("description must be 1 to 1024 characters without angle brackets".to_owned());
    }
    let mut position = 0;
    for (number, state) in STATES.iter().enumerate() {
        let heading = format!("### {}. {state}", number + 1);
        match text[position..].find(&heading) {
            Some(found) => position += found + heading.len(),
            None => problems.add(format!("SKILL.md lacks {heading:?} in order")),
        }
    }
    for reference in files_below(&skill_directory().join("references"))? {
        let name = reference
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !text.contains(&format!("(references/{name})")) {
            problems.add(format!("SKILL.md does not link references/{name}"));
        }
    }
    problems.into_result()
}

#[test]
fn console_commands_parse_and_respect_their_class() -> Result<(), String> {
    let mut problems = Problems::default();
    let policy = policy_table(&mut problems)?;
    for document in markdown_files()? {
        for fence in document
            .fences
            .iter()
            .filter(|fence| fence.info == "console")
        {
            let wanted = if document.name.ends_with("SKILL.md") {
                Some(Class::Free)
            } else if document.name.ends_with("commands.md") {
                Some(
                    if fence.preceded_by == "Only on the user's explicit instruction:" {
                        Class::Explicit
                    } else {
                        Class::Free
                    },
                )
            } else {
                None
            };
            for line in fence.lines.iter().map(|line| line.trim()) {
                if line.is_empty() {
                    continue;
                }
                if !line.starts_with("vsift ") {
                    problems.add(format!(
                        "{}: console line {line:?} is not a vsift command",
                        document.name
                    ));
                    continue;
                }
                let Some(identifier) = check_console_command(&document.name, line, &mut problems)?
                else {
                    continue;
                };
                match (policy.get(&identifier), wanted) {
                    (None, _) => problems.add(format!(
                        "{}: {identifier} has no class in commands.md",
                        document.name
                    )),
                    (Some(Class::Never), _) => problems.add(format!(
                        "{}: shows the never-class command {identifier}",
                        document.name
                    )),
                    (Some(Class::Explicit), Some(Class::Free))
                        if FREE_FORMS.iter().any(|(operation, flag)| {
                            *operation == identifier
                                && line.split_whitespace().any(|word| word == *flag)
                        }) => {}
                    (Some(class), Some(wanted)) if *class != wanted => problems.add(format!(
                        "{}: {identifier} is {class:?} but appears where {wanted:?} commands are listed",
                        document.name
                    )),
                    _ => {}
                }
            }
        }
    }
    problems.into_result()
}

#[test]
fn inline_commands_and_flags_exist() -> Result<(), String> {
    let mut problems = Problems::default();
    let flags = every_long_flag();
    let globals = long_flags(&Cli::command());
    for document in markdown_files()? {
        for span in inline_spans(&document, &mut problems) {
            if span.starts_with("vsift ") {
                let arguments = split_arguments(&substitute(&span)?)?;
                let Some((identifier, leaf)) = resolve_operation(&arguments) else {
                    problems.add(format!(
                        "{}: `{span}` does not name a vsift operation",
                        document.name
                    ));
                    continue;
                };
                let known = long_flags(&leaf);
                for argument in arguments
                    .iter()
                    .filter(|argument| argument.starts_with("--"))
                {
                    let flag = argument.split('=').next().unwrap_or_default();
                    if !known.contains(flag) && !globals.contains(flag) {
                        problems.add(format!(
                            "{}: `{span}`: {flag} is not a flag of {identifier}",
                            document.name
                        ));
                    }
                }
            } else if span.starts_with("--") {
                let flag = span.split(['=', ' ']).next().unwrap_or_default();
                if !flags.contains(flag) {
                    problems.add(format!("{}: `{span}` is not a vsift flag", document.name));
                }
            }
        }
    }
    problems.into_result()
}

#[test]
fn policy_table_classifies_every_command_exactly_once() -> Result<(), String> {
    let mut problems = Problems::default();
    let table = policy_table(&mut problems)?;
    // `parse` is the identifier of a command line that did not parse; there
    // is nothing to run, so it has no class.
    let published: BTreeSet<String> = CommandName::ALL
        .into_iter()
        .filter(|name| *name != CommandName::Parse)
        .map(|name| name.identifier().to_owned())
        .collect();
    let classified: BTreeSet<String> = table.keys().cloned().collect();
    for missing in published.difference(&classified) {
        problems.add(format!("commands.md does not classify {missing}"));
    }
    for unknown in classified.difference(&published) {
        problems.add(format!("commands.md classifies unknown command {unknown}"));
    }
    let with = |wanted: Class| -> BTreeSet<&str> {
        table
            .iter()
            .filter(|(_, class)| **class == wanted)
            .map(|(identifier, _)| identifier.as_str())
            .collect()
    };
    if with(Class::Never) != NEVER.into_iter().collect() {
        problems.add(format!(
            "never-class commands changed: {:?} (expected {NEVER:?})",
            with(Class::Never)
        ));
    }
    if with(Class::Explicit) != EXPLICIT.into_iter().collect() {
        problems.add(format!(
            "explicit-class commands changed: {:?} (expected {EXPLICIT:?})",
            with(Class::Explicit)
        ));
    }
    problems.into_result()
}

/// Upper-case snake identifiers (`INVALID_ARGUMENT`, `VERIFY_SOURCE`) in text.
fn upper_snake_words(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| {
            word.contains('_')
                && word.starts_with(|c: char| c.is_ascii_uppercase())
                && word
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        })
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_failure_code_named_is_published() -> Result<(), String> {
    let mut problems = Problems::default();
    let mut allowed: BTreeSet<String> = FailureCode::ALL
        .into_iter()
        .map(|code| code.identifier().to_owned())
        .collect();
    allowed.extend(STATES.iter().map(|state| (*state).to_owned()));
    for path in skill_text_files()? {
        for word in upper_snake_words(&read_text(&path)?) {
            if !allowed.contains(&word) {
                problems.add(format!(
                    "{}: {word} is neither a failure code nor a state",
                    path.display()
                ));
            }
        }
    }
    let schema = read_json(&skill_directory().join("handoff.schema.json"))?;
    let codes: BTreeSet<String> = schema["$defs"]["gap"]["properties"]["code"]["oneOf"][1]["enum"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let published: BTreeSet<String> = FailureCode::ALL
        .into_iter()
        .map(|code| code.identifier().to_owned())
        .collect();
    if codes != published {
        problems.add(format!(
            "handoff gap codes {codes:?} differ from FailureCode::ALL"
        ));
    }
    let states: Vec<&str> = schema["$defs"]["resume"]["properties"]["state"]["enum"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if states != STATES {
        problems.add(format!(
            "resume states {states:?} differ from the procedure"
        ));
    }
    problems.into_result()
}

/// The published v1 contract, to resolve names against: the schemas and
/// their examples, where a name appears quoted, and the contract document,
/// where it appears as inline code. Some published names live only in the
/// document, because the schema leaves the member a bounded string (the
/// envelope's `coverage.reasons`, such as `untranscribed_range`).
struct PublishedContract {
    schemas: String,
    document: String,
}

impl PublishedContract {
    fn read() -> Result<Self, String> {
        let mut schemas = String::new();
        for path in files_below(&repository().join("schemas").join("v1"))? {
            if has_extension(&path, "json") || has_extension(&path, "jsonl") {
                schemas.push_str(&read_text(&path)?);
            }
        }
        let document = read_text(
            &repository()
                .join("docs")
                .join("contracts")
                .join("cli-v1.md"),
        )?;
        Ok(Self { schemas, document })
    }

    fn publishes(&self, name: &str) -> bool {
        self.schemas.contains(&format!("\"{name}\""))
            || self.document.contains(&format!("`{name}`"))
    }
}

fn is_snake_word(word: &str) -> bool {
    word.starts_with(|c: char| c.is_ascii_lowercase())
        && word
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The names a span asks the reader to find in a result: a snake-case
/// identifier, or each member of a dotted field path. Spans that are
/// identities, commands, flags or file names are not names.
fn schema_names(span: &str) -> Vec<String> {
    const ID_PREFIXES: [&str; 10] = [
        "ses_", "job_", "op_", "trv_", "tsg_", "sgm_", "vix_", "vcd_", "evd_", "src_",
    ];
    const FILE_SUFFIXES: [&str; 5] = [".md", ".json", ".png", ".yaml", ".jsonl"];
    if ID_PREFIXES.iter().any(|prefix| span.starts_with(prefix))
        || FILE_SUFFIXES.iter().any(|suffix| span.ends_with(suffix))
    {
        return Vec::new();
    }
    if span.contains('.') {
        let members: Vec<String> = span
            .split('.')
            .map(|member| member.replace("[]", ""))
            .collect();
        if members.iter().all(|member| is_snake_word(member)) {
            return members;
        }
        return Vec::new();
    }
    if span.contains('_') && is_snake_word(span) {
        return vec![span.to_owned()];
    }
    Vec::new()
}

#[test]
fn field_and_reason_names_resolve_in_the_schemas() -> Result<(), String> {
    let mut problems = Problems::default();
    let published = PublishedContract::read()?;
    let handoff = read_text(&skill_directory().join("handoff.schema.json"))?;
    for document in markdown_files()? {
        for span in inline_spans(&document, &mut problems) {
            for name in schema_names(&span) {
                if !published.publishes(&name) && !handoff.contains(&format!("\"{name}\"")) {
                    problems.add(format!(
                        "{}: `{span}`: {name} is in neither the v1 contract nor the handoff schema",
                        document.name
                    ));
                }
            }
        }
    }
    let schema = read_json(&skill_directory().join("handoff.schema.json"))?;
    for reason in schema["$defs"]["cli_gap_reason"]["enum"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !published.publishes(reason) {
            problems.add(format!(
                "CLI gap reason {reason} is not published in the v1 contract"
            ));
        }
    }
    problems.into_result()
}

/// Every string in a JSON value, depth first.
fn strings(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::String(text) => into.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| strings(item, into)),
        Value::Object(members) => members.values().for_each(|item| strings(item, into)),
        _ => {}
    }
}

/// The rules of `references/handoff.md` that one schema cannot express.
fn check_handoff_semantics(name: &str, handoff: &Value, problems: &mut Problems) {
    let citations = handoff["citations"].as_array().cloned().unwrap_or_default();
    let by_id: BTreeMap<&str, &Value> = citations
        .iter()
        .filter_map(|citation| citation["id"].as_str().map(|id| (id, citation)))
        .collect();
    if by_id.len() != citations.len() {
        problems.add(format!("{name}: citation ids are not unique"));
    }
    let mut used = BTreeSet::new();
    let image_access = handoff["capabilities"]["image_access"].as_str();
    for citation in &citations {
        let visual = matches!(citation["type"].as_str(), Some("frame" | "crop"));
        if visual
            && image_access == Some("unavailable")
            && citation["pixels_inspected"] != Value::Bool(false)
        {
            problems.add(format!(
                "{name}: {} claims inspected pixels without image access",
                citation["id"]
            ));
        }
    }
    for claim in handoff["claims"].as_array().into_iter().flatten() {
        let references: Vec<&str> = claim["citations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let mut grounded = false;
        for reference in &references {
            used.insert((*reference).to_owned());
            match by_id.get(reference) {
                None => problems.add(format!(
                    "{name}: claim {} cites missing {reference}",
                    claim["id"]
                )),
                Some(citation) => {
                    let visual = matches!(citation["type"].as_str(), Some("frame" | "crop"));
                    grounded |= !visual || citation["pixels_inspected"] == Value::Bool(true);
                }
            }
        }
        let rests_on_evidence = matches!(
            claim["support"].as_str(),
            Some("supported" | "partially_supported" | "contradicted")
        );
        if rests_on_evidence && !grounded {
            problems.add(format!(
                "{name}: claim {} is {} on uninspected images only",
                claim["id"], claim["support"]
            ));
        }
    }
    for item in handoff["untrusted_instructions"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if let Some(reference) = item["citation"].as_str() {
            used.insert(reference.to_owned());
        }
    }
    for id in by_id.keys() {
        if !used.contains(*id) {
            problems.add(format!("{name}: citation {id} is never used"));
        }
    }
    let partial = matches!(handoff["status"].as_str(), Some("partial"));
    if partial && handoff["resume"].is_null() {
        problems.add(format!("{name}: a partial handoff needs a resume card"));
    }
    if !handoff["resume"].is_null() {
        let size = serde_json::to_string(&handoff["resume"]).map_or(usize::MAX, |text| text.len());
        if size > MAX_RESUME_BYTES {
            problems.add(format!("{name}: resume card is {size} bytes"));
        }
    }
    let mut texts = Vec::new();
    strings(handoff, &mut texts);
    for text in texts {
        let lowered = text.to_ascii_lowercase();
        let path_like = ["\\\\?\\", ":\\", "/users/", "/home/", "~/", "appdata"]
            .iter()
            .any(|marker| lowered.contains(marker));
        if path_like {
            problems.add(format!("{name}: {text:?} looks like a local path"));
        }
    }
}

#[test]
fn example_handoffs_validate_against_the_handoff_schema() -> Result<(), String> {
    let mut problems = Problems::default();
    let schema = read_json(&skill_directory().join("handoff.schema.json"))?;
    let validator = jsonschema::options()
        .build(&schema)
        .map_err(|error| format!("handoff.schema.json is not a valid schema: {error}"))?;
    let policy = policy_table(&mut problems)?;
    let examples: Vec<PathBuf> = files_below(&skill_directory().join("examples"))?
        .into_iter()
        .filter(|path| path.to_string_lossy().ends_with(".handoff.json"))
        .collect();
    if examples.len() < 2 {
        problems.add("expected at least two example handoffs".to_owned());
    }
    for path in examples {
        let name = path.display().to_string();
        let handoff = read_json(&path)?;
        for error in validator.iter_errors(&handoff) {
            problems.add(format!("{name}: {error} at {}", error.instance_path()));
        }
        check_handoff_semantics(&name, &handoff, &mut problems);
        let next = handoff["resume"]["next_command"].as_str();
        let identifier = match next {
            Some(next) => check_console_command(&name, next, &mut problems)?,
            None => None,
        };
        if let Some(identifier) = identifier
            && policy.get(&identifier) != Some(&Class::Free)
        {
            problems.add(format!(
                "{name}: resume next_command {identifier} is not a free command"
            ));
        }
    }
    problems.into_result()
}

#[test]
fn handoff_schema_refuses_paths_links_and_hidden_characters() -> Result<(), String> {
    let schema = read_json(&skill_directory().join("handoff.schema.json"))?;
    let validator = jsonschema::options()
        .build(&schema)
        .map_err(|error| format!("handoff.schema.json is not a valid schema: {error}"))?;
    let example = read_json(
        &skill_directory()
            .join("examples")
            .join("supplied-transcript.handoff.json"),
    )?;
    let mut problems = Problems::default();
    let refused = [
        "Open C:\\Users\\someone\\video.mp4 first.",
        "See /home/someone/video.mp4.",
        "Saved under ~/recordings.",
        "Found at \\\\server\\share.",
        "Visit https://example.invalid now.",
        "Text with a hidden \u{202E}override.",
        "Zero\u{200B}width.",
        "Two\nlines.",
    ];
    for text in refused {
        let mut handoff = example.clone();
        handoff["question"] = Value::String(text.to_owned());
        if validator.is_valid(&handoff) {
            problems.add(format!("the schema accepts {text:?}"));
        }
    }
    let accepted = [
        "Does dialog R-17 appear at 00:05.000, and/or later?",
        "Is the ratio 3/4 shown (about ~5 s in)?",
    ];
    for text in accepted {
        let mut handoff = example.clone();
        handoff["question"] = Value::String(text.to_owned());
        if !validator.is_valid(&handoff) {
            problems.add(format!("the schema refuses {text:?}"));
        }
    }
    problems.into_result()
}

#[test]
fn handoff_template_has_every_section_in_order() -> Result<(), String> {
    let text =
        read_text(&skill_directory().join("references").join("handoff.md"))?.replace("\r\n", "\n");
    let mut problems = Problems::default();
    let mut position = 0;
    for section in HANDOFF_SECTIONS {
        match text[position..].find(&format!("\n{section}\n")) {
            Some(found) => position += found + section.len(),
            None => problems.add(format!("handoff.md template lacks {section:?} in order")),
        }
    }
    if !text[position..].contains("```vsift-handoff") {
        problems
            .add("handoff.md template lacks the vsift-handoff block after Lifecycle".to_owned());
    }
    problems.into_result()
}

#[test]
fn image_check_code_appears_only_in_the_pixels() -> Result<(), String> {
    let mut problems = Problems::default();
    let code = IMAGE_CODE_PARTS.concat();
    let parts: Vec<String> = code.split(' ').map(str::to_ascii_lowercase).collect();
    for path in skill_text_files()? {
        let text = read_text(&path)?.to_ascii_lowercase();
        for part in &parts {
            if text.contains(part.as_str()) {
                problems.add(format!(
                    "{} contains part of the image check code",
                    path.display()
                ));
            }
        }
    }
    let image_path = skill_directory().join("assets").join("image-check.png");
    let image =
        fs::read(&image_path).map_err(|error| format!("{}: {error}", image_path.display()))?;
    if !image.starts_with(b"\x89PNG\r\n\x1a\n") {
        problems.add("assets/image-check.png is not a PNG".to_owned());
    }
    if image.len() > MAX_IMAGE_CHECK_BYTES {
        problems.add(format!(
            "assets/image-check.png is {} bytes, above {MAX_IMAGE_CHECK_BYTES}",
            image.len()
        ));
    }
    let lowered = image.to_ascii_lowercase();
    for part in &parts {
        if lowered
            .windows(part.len())
            .any(|window| window == part.as_bytes())
        {
            problems.add("the check image carries its code as text, not only as pixels".to_owned());
        }
    }
    problems.into_result()
}

#[test]
fn relative_links_resolve_and_no_tools_are_granted() -> Result<(), String> {
    let mut problems = Problems::default();
    for path in skill_text_files()? {
        if !has_extension(&path, "md") {
            continue;
        }
        let text = read_text(&path)?;
        let directory = path.parent().map(Path::to_path_buf).unwrap_or_default();
        for piece in text.split("](").skip(1) {
            let target = piece.split(')').next().unwrap_or_default();
            let target = target.split('#').next().unwrap_or_default();
            if target.is_empty() || target.contains("://") {
                continue;
            }
            if !directory.join(target).exists() {
                problems.add(format!(
                    "{}: link {target} does not resolve",
                    path.display()
                ));
            }
        }
    }
    let codex = read_text(&skill_directory().join("agents").join("openai.yaml"))?;
    if codex
        .lines()
        .any(|line| line.trim_start().starts_with("dependencies:"))
    {
        problems.add("agents/openai.yaml must not declare tool dependencies".to_owned());
    }
    problems.into_result()
}
