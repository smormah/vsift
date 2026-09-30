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
use vsift::{FailureCode, OperationId};
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

/// The code printed in `assets/image-check.png`, kept in two parts so a
/// plain search of the repository for the joined code does not find it. An
/// agent must read it from the pixels; if any skill text contained it, a
/// model could "pass" the image check without seeing the image. Redrawn in
/// P12 PR 3i with glyphs no reader confuses (no I, l, 1, O, 0, S, 5, B or
/// 8); the trial grader keeps the same code in its table of check images.
const IMAGE_CODE_PARTS: [&str; 2] = ["HKR", "X 4739"];

/// Codes of check images the skill no longer ships, in the same two-part
/// form. None of their words may come back into the skill: an agent that
/// remembered one must not find it confirmed there.
const RETIRED_IMAGE_CODE_PARTS: [[&str; 2]; 1] = [["OK", "API 6281"]];

/// Upper bound on `SKILL.md`, which every client loads whole into its context.
const MAX_SKILL_LINES: usize = 300;

/// Upper bound on the check image, which a one-image client pays for.
const MAX_IMAGE_CHECK_BYTES: usize = 8 * 1024;

/// Upper bound on a serialized resume card (`resume` in the handoff).
const MAX_RESUME_BYTES: usize = 2 * 1024;

/// Placeholders the skill's command lines use, with a value the parser
/// accepts. An unknown placeholder fails the guard, so a new one is added here
/// deliberately.
const PLACEHOLDERS: [(&str, &str); 23] = [
    ("namespace", "session"),
    ("operation", "retain"),
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

/// The one thing a console example may add after a `vsift` command: keep only
/// the terminal event of `--events jsonl`. The trial grader accepts it as a
/// line filter in the command's own pipeline; everything else chained, piped
/// or redirected is a non-`vsift` command there. In the second dry trial
/// (2026-09-28) a strong model chained `date` before and after its commands
/// to time itself, so the examples never show any other combination.
const LINE_FILTER_SUFFIX: &str = " | tail -n 1";

/// The identifier the guard gives a help form (`vsift --help`, `vsift
/// <namespace> <operation> --help`). It is not a [`CommandName`]: it runs no
/// operation, it only prints the parser's usage text, and the skill classes
/// it `free` so an agent can recover a command's flags (added 2026-09-29,
/// after a small model guessed `session retain --directory` and piped the
/// help through `grep`). The trial grader reads the same forms from
/// `commands.md`.
const HELP_IDENTIFIER: &str = "help";

/// The flag of the help forms; clap adds it to every command.
const HELP_FLAG: &str = "--help";

/// The heredoc delimiter of the POSIX `handoff check` form.
const HANDOFF_DELIMITER: &str = "VSIFT_HANDOFF";

/// The only two forms in which the skill passes text into a command (ADR
/// 0023 decision G, P13 PR 5): the draft report, as a quoted heredoc in a
/// POSIX shell, or as a single-quoted here-string piped in PowerShell. Both
/// pass the text without expansion. `<report>` stands for the draft; the
/// fence's info string names the shell. The trial grader recognises exactly
/// these two forms too; nothing wider is accepted here or there.
const HANDOFF_CHECK_FORMS: [(&str, &str); 2] = [
    (
        "sh",
        "vsift handoff check --json <<'VSIFT_HANDOFF'\n<report>\nVSIFT_HANDOFF",
    ),
    (
        "powershell",
        "@'\n<report>\n'@ | vsift handoff check --json",
    ),
];

/// The command both forms run, as its arguments.
const HANDOFF_CHECK_COMMAND: &str = "vsift handoff check --json";

/// Fence info strings that hold shell text other than a `console` example.
const SHELL_FENCES: [&str; 6] = ["sh", "bash", "shell", "powershell", "pwsh", "ps1"];

/// Which of [`HANDOFF_CHECK_FORMS`] `text` is, exactly, in a fence of `info`.
fn handoff_check_form(info: &str, text: &str) -> Option<usize> {
    HANDOFF_CHECK_FORMS
        .iter()
        .position(|(shell, form)| *shell == info && *form == text)
}

/// Shell syntax that joins, pipes, redirects or substitutes commands.
const SHELL_OPERATORS: [&str; 8] = ["&&", "||", ";", "|", ">", "<", "$(", "`"];

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

/// Checks a help form: `vsift`, then only subcommand words, then `--help`,
/// which the parser answers with its help text. Returns whether `arguments`
/// is one; a help form the parser does not answer with help is a problem.
fn check_help_form(source: &str, arguments: &[String], problems: &mut Problems) -> bool {
    let Some((last, words)) = arguments.split_last() else {
        return false;
    };
    if last != HELP_FLAG {
        return false;
    }
    let mut command = Cli::command();
    for word in words.iter().skip(1) {
        let Some(next) = command.find_subcommand(word).cloned() else {
            problems.add(format!(
                "{source}: {arguments:?}: {word} is not a subcommand, so this is no help form"
            ));
            return true;
        };
        command = next;
    }
    match Cli::try_parse_from(arguments) {
        Err(error) if error.kind() == clap::error::ErrorKind::DisplayHelp => {}
        _ => problems.add(format!(
            "{source}: {arguments:?} does not print the parser's help"
        )),
    }
    true
}

/// Checks one `vsift` command line from a console fence and returns its
/// operation identifier ([`HELP_IDENTIFIER`] for a help form).
fn check_console_command(
    source: &str,
    line: &str,
    problems: &mut Problems,
) -> Result<Option<String>, String> {
    let (command, filtered) = match line.strip_suffix(LINE_FILTER_SUFFIX) {
        Some(command) => (command, true),
        None => (line, false),
    };
    let arguments = split_arguments(&substitute(command)?)?;
    if !filtered && check_help_form(source, &arguments, problems) {
        return Ok(Some(HELP_IDENTIFIER.to_owned()));
    }
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
    if filtered && cli.events.is_none() {
        problems.add(format!(
            "{source}: {line:?} keeps only the last line of output without --events jsonl"
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

/// `SKILL.md` without line breaks, so a rule can be found whatever its wrap.
fn flattened_skill_md() -> Result<String, String> {
    Ok(read_text(&skill_directory().join("SKILL.md"))?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" "))
}

/// The REPORT state carries a filled-in minimal handoff, not only a link,
/// and every stop ends there. In the diagnostic passes (2026-09-29) small
/// models ended without a handoff when a tool was missing ("explain the
/// remediation and stop"), wrote free-form reports or blocks with invented
/// schemas, and one wrote the report to a file. Since issue #218 the
/// skeleton holds one example claim, citation and instruction, all
/// validated here against `handoff.schema.json` and the handoff rules.
#[test]
fn skill_md_report_state_shows_a_valid_minimal_handoff() -> Result<(), String> {
    let text = read_text(&skill_directory().join("SKILL.md"))?;
    let start = text.find("### 7. REPORT").ok_or("SKILL.md lacks REPORT")?;
    let end = text[start..]
        .find("### 8. CLOSE_OR_RETAIN")
        .map(|length| start + length)
        .ok_or("SKILL.md lacks CLOSE_OR_RETAIN after REPORT")?;
    let section = parse_markdown("REPORT".to_owned(), &text[start..end]);
    let blocks: Vec<&Fence> = section
        .fences
        .iter()
        .filter(|fence| fence.info == "vsift-handoff")
        .collect();
    let mut problems = Problems::default();
    let [block] = blocks.as_slice() else {
        return Err(format!(
            "REPORT must show exactly one vsift-handoff block, found {}",
            blocks.len()
        ));
    };
    let handoff: Value = serde_json::from_str(&block.lines.join("\n"))
        .map_err(|error| format!("REPORT's vsift-handoff block is not JSON: {error}"))?;
    let schema = read_json(&skill_directory().join("handoff.schema.json"))?;
    let validator = jsonschema::options()
        .build(&schema)
        .map_err(|error| format!("handoff.schema.json is not a valid schema: {error}"))?;
    for error in validator.iter_errors(&handoff) {
        problems.add(format!(
            "REPORT's handoff: {error} at {}",
            error.instance_path()
        ));
    }
    check_handoff_semantics("REPORT's handoff", &handoff, &mut problems);
    let flat = flattened_skill_md()?;
    for needle in [
        "Every stop ends in REPORT",
        "exactly one** fenced `vsift-handoff` block",
        "Never create, edit or save a file",
        "Never `cd`",
        "run nothing else to check",
        "**Before you send**",
    ] {
        if !flat.contains(needle) {
            problems.add(format!("SKILL.md does not say {needle:?}"));
        }
    }
    if flat.contains("remediation to the user and stop") {
        problems.add("SKILL.md still stops without a handoff".to_owned());
    }
    problems.into_result()
}

/// The compact limits stand in `SKILL.md`'s rules as numbers, equal to the
/// `compact` column of `budgets.md`: a small model used `--limit 100` and
/// 43 tool calls when the rules only named the profile (2026-09-29).
#[test]
fn skill_md_states_the_compact_limits_from_budgets_md() -> Result<(), String> {
    let budgets = read_text(&skill_directory().join("references").join("budgets.md"))?;
    let compact = |row: &str| -> Result<String, String> {
        budgets
            .lines()
            .find_map(|line| {
                let cells: Vec<&str> = line.split('|').map(str::trim).collect();
                (cells.get(1) == Some(&row)).then(|| cells.get(2).map(|cell| (*cell).to_owned()))
            })
            .flatten()
            .ok_or_else(|| format!("budgets.md has no {row:?} row"))
    };
    let flat = flattened_skill_md()?;
    let mut problems = Problems::default();
    for needle in [
        format!("at most {} tool calls", compact("Tool calls")?),
        format!("{} images in total", compact("Images in total")?),
        format!("{} image per step", compact("Images per step")?),
        format!("`--limit {}`", compact("Page size")?),
        format!("`--max-frames {}`", compact("Burst frames")?),
    ] {
        if !flat.contains(&needle) {
            problems.add(format!("SKILL.md's rules do not state {needle:?}"));
        }
    }
    problems.into_result()
}

/// `handoff check` reads the profiles' limits from contract constants
/// (`vsift_contract::COMPACT_BUDGET` and `STANDARD_BUDGET`, P13 PR 5); they
/// must be the table of `budgets.md`, in the handoff's units.
#[test]
fn the_contracts_budget_profiles_are_budgets_md() -> Result<(), String> {
    let budgets = read_text(&skill_directory().join("references").join("budgets.md"))?;
    let quantity = |cell: &str| -> Option<u64> {
        let mut parts = cell.split_whitespace();
        let number: u64 = parts.next()?.parse().ok()?;
        match parts.next() {
            None => Some(number),
            Some("MiB") => number.checked_mul(1024 * 1024),
            Some("min") => number.checked_mul(60),
            Some(_) => None,
        }
    };
    let row = |name: &str| -> Result<(u64, u64), String> {
        budgets
            .lines()
            .find_map(|line| {
                let cells: Vec<&str> = line.split('|').map(str::trim).collect();
                (cells.get(1) == Some(&name))
                    .then(|| Some((quantity(cells.get(2)?)?, quantity(cells.get(3)?)?)))
            })
            .flatten()
            .ok_or_else(|| format!("budgets.md has no readable {name:?} row"))
    };
    let mut problems = Problems::default();
    for (row_name, member) in [
        ("Images per step", "images_per_step"),
        ("Images in total", "images_total"),
        ("Image bytes in total", "image_bytes"),
        ("Page size", "page_limit"),
        ("Tool calls", "tool_calls"),
        ("Refinement depth", "refinement_depth"),
        ("Wall time", "wall_time_s"),
        ("Burst frames", "burst_frames"),
    ] {
        let (compact, standard) = row(row_name)?;
        let constant = |limits: vsift_contract::HandoffBudgetLimits| {
            limits
                .members()
                .into_iter()
                .find(|(name, _)| *name == member)
                .map(|(_, value)| value)
        };
        if constant(vsift_contract::COMPACT_BUDGET) != Some(compact)
            || constant(vsift_contract::STANDARD_BUDGET) != Some(standard)
        {
            problems.add(format!("the contract's {member} differs from budgets.md"));
        }
    }
    problems.into_result()
}

/// `FIND_SPOKEN_SPANS` starts with `vsift search`: its hits carry the segment
/// identities and times a handoff cites, and A-08 requires search. In the
/// first dry trial (2026-09-28) a strong model read a short transcript whole
/// with `transcript get` and never searched, so the order is guarded here.
#[test]
fn find_spoken_spans_searches_before_it_reads_the_transcript() -> Result<(), String> {
    let text = read_text(&skill_directory().join("SKILL.md"))?;
    let start = text
        .find("### 3. FIND_SPOKEN_SPANS")
        .ok_or("SKILL.md lacks FIND_SPOKEN_SPANS")?;
    let length = text[start..]
        .find("### 4. INSPECT_CARDS")
        .ok_or("SKILL.md lacks INSPECT_CARDS after FIND_SPOKEN_SPANS")?;
    let section = parse_markdown("FIND_SPOKEN_SPANS".to_owned(), &text[start..start + length]);
    let commands: Vec<String> = section
        .fences
        .iter()
        .filter(|fence| fence.info == "console")
        .flat_map(|fence| fence.lines.iter())
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect();
    let mut problems = Problems::default();
    match commands.first() {
        Some(first) if first.starts_with("vsift search ") => {}
        other => problems.add(format!(
            "the first command of FIND_SPOKEN_SPANS must be `vsift search`, found {other:?}"
        )),
    }
    if !commands
        .iter()
        .any(|line| line.starts_with("vsift transcript get "))
    {
        problems.add("FIND_SPOKEN_SPANS no longer reads bounded transcript windows".to_owned());
    }
    if !section
        .prose
        .iter()
        .any(|line| line.contains("Always search first"))
    {
        problems.add("FIND_SPOKEN_SPANS must say to always search first".to_owned());
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
                let class = if identifier == HELP_IDENTIFIER {
                    Some(&Class::Free)
                } else {
                    policy.get(&identifier)
                };
                match (class, wanted) {
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

/// Every console example is one `vsift` command: nothing chained, piped,
/// redirected or substituted, except the documented `| tail -n 1` after
/// `--events jsonl` (checked with the parser in `check_console_command`).
#[test]
fn console_examples_are_one_vsift_command_each() -> Result<(), String> {
    let mut problems = Problems::default();
    for document in markdown_files()? {
        for fence in document
            .fences
            .iter()
            .filter(|fence| fence.info == "console")
        {
            for line in fence.lines.iter().map(|line| line.trim()) {
                if !line.starts_with("vsift ") {
                    continue;
                }
                let command = substitute(line.strip_suffix(LINE_FILTER_SUFFIX).unwrap_or(line))?;
                for operator in SHELL_OPERATORS {
                    if command.contains(operator) {
                        problems.add(format!(
                            "{}: {line:?} uses {operator:?}; run each vsift command alone",
                            document.name
                        ));
                    }
                }
            }
        }
    }
    problems.into_result()
}

/// The draft forms of `handoff check` are the skill's one input exception
/// (P13 PR 5): every shell fence of the skill is exactly one of the two
/// forms, `commands.md` shows both and `SKILL.md`'s REPORT the POSIX one,
/// and their command parses as the `free` operation `handoff.check`.
#[test]
fn the_handoff_check_forms_are_the_only_input_exception() -> Result<(), String> {
    let mut problems = Problems::default();
    let policy = policy_table(&mut problems)?;
    let arguments = split_arguments(HANDOFF_CHECK_COMMAND)?;
    match Cli::try_parse_from(&arguments) {
        Ok(cli) if cli.json => {}
        _ => problems.add(format!(
            "{HANDOFF_CHECK_COMMAND:?} does not parse with --json"
        )),
    }
    match resolve_operation(&arguments) {
        Some((identifier, _))
            if identifier == CommandName::HandoffCheck.identifier()
                && policy.get(&identifier) == Some(&Class::Free) => {}
        other => problems.add(format!(
            "the draft forms run {other:?}, not the free handoff.check",
            other = other.map(|(identifier, _)| identifier)
        )),
    }
    for (_, form) in HANDOFF_CHECK_FORMS {
        if form.matches(HANDOFF_CHECK_COMMAND).count() != 1 {
            problems.add(format!(
                "{form:?} does not run {HANDOFF_CHECK_COMMAND:?} once"
            ));
        }
    }
    let mut forms_by_file: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    for document in markdown_files()? {
        for fence in &document.fences {
            let text = fence.lines.join("\n");
            let runs_vsift = fence.lines.iter().any(|line| line.contains("vsift "));
            let shell = SHELL_FENCES.contains(&fence.info.as_str());
            // `console` examples have their own guard; JSON and Markdown
            // fences are data, whatever command a value names.
            let data =
                ["console", "vsift-handoff", "json", "markdown"].contains(&fence.info.as_str());
            if data || !(shell || runs_vsift) {
                continue;
            }
            match handoff_check_form(&fence.info, text.trim_end()) {
                Some(form) => {
                    forms_by_file
                        .entry(document.name.clone())
                        .or_default()
                        .insert(form);
                }
                None => problems.add(format!(
                    "{}: a {:?} fence is not one of the two handoff check forms: {text:?}",
                    document.name, fence.info
                )),
            }
        }
    }
    let shows = |suffix: &str, form: usize| {
        forms_by_file
            .iter()
            .any(|(name, forms)| name.ends_with(suffix) && forms.contains(&form))
    };
    if !(shows("commands.md", 0) && shows("commands.md", 1)) {
        problems.add("commands.md does not show both handoff check forms".to_owned());
    }
    if !shows("SKILL.md", 0) {
        problems.add("SKILL.md's REPORT does not show the POSIX handoff check form".to_owned());
    }
    let flat = flattened_skill_md()?;
    if !flat
        .contains("run `vsift handoff check` on your draft once, fix what it reports, then send")
    {
        problems.add("SKILL.md's REPORT does not say to check the draft once".to_owned());
    }
    problems.into_result()
}

/// Nothing wider than the two literal forms is a draft form: another
/// command piped in, an unquoted or double-quoted heredoc, a double-quoted
/// here-string, another delimiter, a file, a redirection or anything added.
#[test]
fn variants_of_the_handoff_check_forms_are_refused() {
    for (info, variant) in [
        (
            "sh",
            "vsift handoff check --json <<VSIFT_HANDOFF\n<report>\nVSIFT_HANDOFF",
        ),
        (
            "sh",
            "vsift handoff check --json <<\"VSIFT_HANDOFF\"\n<report>\nVSIFT_HANDOFF",
        ),
        (
            "sh",
            "vsift handoff check --json <<-'VSIFT_HANDOFF'\n<report>\nVSIFT_HANDOFF",
        ),
        ("sh", "vsift handoff check --json <<'EOF'\n<report>\nEOF"),
        (
            "sh",
            "vsift handoff check <<'VSIFT_HANDOFF'\n<report>\nVSIFT_HANDOFF",
        ),
        (
            "sh",
            "vsift session close <session> --json <<'VSIFT_HANDOFF'\n<report>\nVSIFT_HANDOFF",
        ),
        (
            "sh",
            "vsift handoff check --json <<'VSIFT_HANDOFF' | tail -n 1\n<report>\nVSIFT_HANDOFF",
        ),
        (
            "sh",
            "vsift handoff check --json <<'VSIFT_HANDOFF'\n<report>\nVSIFT_HANDOFF\necho done",
        ),
        ("sh", "cat draft.md | vsift handoff check --json"),
        ("sh", "vsift handoff check --json < draft.md"),
        ("sh", "echo '<report>' | vsift handoff check --json"),
        (
            "powershell",
            "@\"\n<report>\n\"@ | vsift handoff check --json",
        ),
        (
            "powershell",
            "@'\n<report>\n'@ | vsift session close <session> --json",
        ),
        (
            "powershell",
            "@'\n<report>\n'@ | vsift handoff check --json | Select-Object -Last 1",
        ),
        (
            "powershell",
            "Get-Content draft.md | vsift handoff check --json",
        ),
        (
            "powershell",
            "vsift handoff check --json <<'VSIFT_HANDOFF'\n<report>\nVSIFT_HANDOFF",
        ),
        ("sh", "@'\n<report>\n'@ | vsift handoff check --json"),
    ] {
        assert_eq!(handoff_check_form(info, variant), None, "{variant}");
    }
    for (index, (info, form)) in HANDOFF_CHECK_FORMS.iter().enumerate() {
        assert_eq!(handoff_check_form(info, form), Some(index));
    }
}

/// The words of `text` that start with `op_` or `op-`, cut at the first
/// character that is neither alphanumeric nor `_` or `-`, so a malformed id
/// such as the dry trial's `op-asr-walkthrough-1` is seen whole.
fn operation_id_tokens(text: &str) -> Vec<String> {
    let characters: Vec<char> = text.chars().collect();
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let mut tokens = Vec::new();
    let mut index = 0;
    while index + 3 <= characters.len() {
        let starts = characters[index] == 'o'
            && characters[index + 1] == 'p'
            && matches!(characters[index + 2], '_' | '-')
            && (index == 0 || !is_word(characters[index - 1]));
        if starts {
            let token: String = characters[index..]
                .iter()
                .take_while(|c| is_word(**c))
                .collect();
            index += token.len();
            tokens.push(token);
        } else {
            index += 1;
        }
    }
    tokens
}

/// Every operation id the skill shows follows the published grammar (cli-v1
/// D-1: `op_` and 16 to 64 lowercase letters or digits), checked with the
/// parser's own type, and `SKILL.md` shows a valid example before its first
/// command that takes one. In the second dry trial (2026-09-28) a strong model
/// first sent `--operation-id op-asr-walkthrough-1`, a parse failure.
#[test]
fn operation_ids_follow_the_published_grammar() -> Result<(), String> {
    let mut problems = Problems::default();
    for path in skill_text_files()? {
        for token in operation_id_tokens(&read_text(&path)?) {
            // The bare prefix is how the prose names the grammar.
            if token != "op_" && OperationId::parse(token.clone()).is_err() {
                problems.add(format!(
                    "{}: {token:?} is not a valid operation id",
                    path.display()
                ));
            }
        }
    }
    let text = read_text(&skill_directory().join("SKILL.md"))?;
    let first_use = parse_markdown("SKILL.md".to_owned(), &text)
        .fences
        .iter()
        .filter(|fence| fence.info == "console")
        .flat_map(|fence| fence.lines.iter())
        .find(|line| line.contains("--operation-id"))
        .and_then(|line| text.find(line.as_str()))
        .ok_or("SKILL.md shows no command with --operation-id")?;
    let state_start = text[..first_use].rfind("\n### ").unwrap_or_default();
    let examples = operation_id_tokens(&text[state_start..first_use]);
    if !examples
        .iter()
        .any(|token| OperationId::parse(token.clone()).is_ok())
    {
        problems.add(
            "SKILL.md shows no valid example operation id before its first --operation-id command"
                .to_owned(),
        );
    }
    for (valid, token) in [
        (false, "op-asr-walkthrough-1"),
        (false, "op_asr-walkthrough-1"),
        (true, "op_retx0123456789abcdef0123456701"),
    ] {
        let found = operation_id_tokens(&format!("--operation-id {token} --json"));
        let parsed = found
            .first()
            .is_some_and(|found| OperationId::parse(found.clone()).is_ok());
        if parsed != valid {
            problems.add(format!(
                "the operation id scan reads {token:?} as {found:?}"
            ));
        }
    }
    problems.into_result()
}

/// The rules that always apply forbid every other program, including the
/// "harmless" clock read of the second dry trial, and the budget says who
/// keeps the wall time.
#[test]
fn the_skill_forbids_other_programs_and_self_timing() -> Result<(), String> {
    let mut problems = Problems::default();
    let skill = read_text(&skill_directory().join("SKILL.md"))?;
    let rules_start = skill
        .find("## Rules that always apply")
        .ok_or("SKILL.md lacks its rules")?;
    let rules_end = skill[rules_start..]
        .find("## The procedure")
        .map_or(skill.len(), |end| rules_start + end);
    let rules = &skill[rules_start..rules_end];
    for needle in ["Nothing but `vsift`", "`date`", "`&&`", "`| tail -n 1`"] {
        if !rules.contains(needle) {
            problems.add(format!("SKILL.md's rules do not mention {needle}"));
        }
    }
    let budgets = read_text(&skill_directory().join("references").join("budgets.md"))?;
    for needle in [
        "measured and enforced by the host",
        "`date`",
        "`budget.used.wall_time_s`",
    ] {
        if !budgets.contains(needle) {
            problems.add(format!("budgets.md does not say {needle}"));
        }
    }
    problems.into_result()
}

#[test]
fn inline_commands_and_flags_exist() -> Result<(), String> {
    let mut problems = Problems::default();
    let mut flags = every_long_flag();
    flags.insert(HELP_FLAG.to_owned());
    let globals = long_flags(&Cli::command());
    for document in markdown_files()? {
        for span in inline_spans(&document, &mut problems) {
            if span.starts_with("vsift ") {
                let arguments = split_arguments(&substitute(&span)?)?;
                if check_help_form(&document.name, &arguments, &mut problems) {
                    continue;
                }
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
    allowed.insert(HANDOFF_DELIMITER.to_owned());
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

/// Whether the work was cut short and can continue, which is when a resume
/// card is required (supervisor's decision, 2026-09-29): a budget limit
/// exhausted, or a gap with reason `budget_exhausted` or `cancelled`, or
/// code `CANCELLED`. A report that is `partial` only because a capability
/// is missing or the session expired needs no card; resuming cannot fix it.
/// The trial grader applies the same rule (`cut_short_reason`).
fn cut_short(handoff: &Value) -> bool {
    let exhausted = handoff["budget"]["exhausted"]
        .as_array()
        .is_some_and(|limits| !limits.is_empty());
    exhausted
        || handoff["gaps"].as_array().into_iter().flatten().any(|gap| {
            matches!(
                gap["reason"].as_str(),
                Some("budget_exhausted" | "cancelled")
            ) || gap["code"] == "CANCELLED"
        })
}

/// A resume card's findings to verify again (P12 PR 3i) each name a window
/// that does not end before it starts; the trial grader applies the same
/// rule and also checks the window against `VSift`'s records.
fn check_resume_windows(name: &str, handoff: &Value, problems: &mut Problems) {
    for item in handoff["resume"]["to_verify"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if item["from_us"].as_u64() > item["to_us"].as_u64() {
            problems.add(format!(
                "{name}: the resume card's finding on {} ends before it starts",
                item["id"]
            ));
        }
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
        // A claim that rests on evidence and cites none is the schema's
        // `minItems` failure; this rule is about what the citations show.
        if rests_on_evidence && !references.is_empty() && !grounded {
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
    if cut_short(handoff) && handoff["resume"].is_null() {
        problems.add(format!(
            "{name}: work cut short that can continue needs a resume card"
        ));
    }
    if !handoff["resume"].is_null() {
        let size = serde_json::to_string(&handoff["resume"]).map_or(usize::MAX, |text| text.len());
        if size > MAX_RESUME_BYTES {
            problems.add(format!("{name}: resume card is {size} bytes"));
        }
    }
    check_resume_windows(name, handoff, problems);
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

/// The `required` list of a schema object, sorted.
fn required_members(schema: &Value) -> Vec<String> {
    let mut members: Vec<String> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    members.sort();
    members
}

fn sorted(members: &[&str]) -> Vec<String> {
    let mut members: Vec<String> = members.iter().map(|member| (*member).to_owned()).collect();
    members.sort();
    members
}

/// Maintainer decision 2 of 2026-09-29: the handoff requires only what the
/// agent alone knows (its findings, the evidence identities, whether it
/// looked at each image, gaps, instructions it saw and the session's fate);
/// everything `VSift` already recorded is optional. Handoff v1 is
/// unreleased and was revised in place.
#[test]
fn handoff_schema_requires_only_what_the_agent_alone_knows() -> Result<(), String> {
    let schema = read_json(&skill_directory().join("handoff.schema.json"))?;
    let definitions = &schema["$defs"];
    let mut problems = Problems::default();
    for (name, object, expected) in [
        (
            "the handoff",
            &schema,
            sorted(&[
                "handoff_version",
                "status",
                "question",
                "capabilities",
                "claims",
                "citations",
                "gaps",
                "untrusted_instructions",
                "lifecycle",
            ]),
        ),
        (
            "capabilities",
            &schema["properties"]["capabilities"],
            sorted(&["image_access"]),
        ),
        (
            "a claim",
            &definitions["claim"],
            sorted(&[
                "id",
                "section",
                "kind",
                "support",
                "certainty",
                "statement",
                "citations",
            ]),
        ),
        (
            "a segment citation",
            &definitions["transcript_segment_citation"],
            sorted(&["id", "type", "segment_id"]),
        ),
        (
            "a frame citation",
            &definitions["frame_citation"],
            sorted(&["id", "type", "evidence_id", "pixels_inspected"]),
        ),
        (
            "a crop citation",
            &definitions["crop_citation"],
            sorted(&["id", "type", "evidence_id", "pixels_inspected"]),
        ),
        (
            "an audio citation",
            &definitions["audio_citation"],
            sorted(&["id", "type", "evidence_id"]),
        ),
        (
            "a gap",
            &definitions["gap"],
            sorted(&["kind", "reason", "note"]),
        ),
        (
            "the lifecycle",
            &definitions["lifecycle"],
            sorted(&["action"]),
        ),
        ("a budget", &definitions["budget"], sorted(&["profile"])),
    ] {
        let found = required_members(object);
        if found != expected {
            problems.add(format!("{name} requires {found:?}, not {expected:?}"));
        }
    }
    let validator = jsonschema::options()
        .build(&schema)
        .map_err(|error| format!("handoff.schema.json is not a valid schema: {error}"))?;
    let verified_without_code = serde_json::json!({
        "handoff_version": "1", "status": "complete", "question": "What happens?",
        "capabilities": {"image_access": "verified"}, "claims": [], "citations": [],
        "gaps": [], "untrusted_instructions": [], "lifecycle": {"action": "left_open"}
    });
    if validator.is_valid(&verified_without_code) {
        problems.add("a verified image access without its code validates".to_owned());
    }
    problems.into_result()
}

/// The REPORT state's skeleton shows the required members only, so a small
/// model copies the slim shape; the optional ones are named beside it.
#[test]
fn report_skeleton_holds_only_the_required_members() -> Result<(), String> {
    let text = read_text(&skill_directory().join("SKILL.md"))?;
    let start = text.find("### 7. REPORT").ok_or("SKILL.md lacks REPORT")?;
    let section = parse_markdown("REPORT".to_owned(), &text[start..]);
    let block = section
        .fences
        .iter()
        .find(|fence| fence.info == "vsift-handoff")
        .ok_or("REPORT shows no vsift-handoff block")?;
    let handoff: Value = serde_json::from_str(&block.lines.join("\n"))
        .map_err(|error| format!("REPORT's vsift-handoff block is not JSON: {error}"))?;
    let schema = read_json(&skill_directory().join("handoff.schema.json"))?;
    let keys = |value: &Value| -> Vec<String> {
        let mut keys: Vec<String> = value
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, _)| key.clone())
            .collect();
        keys.sort();
        keys
    };
    let mut problems = Problems::default();
    if keys(&handoff) != required_members(&schema) {
        problems.add(format!(
            "the skeleton's members {:?} are not the required {:?}",
            keys(&handoff),
            required_members(&schema)
        ));
    }
    if keys(&handoff["capabilities"]) != sorted(&["image_access", "image_check_code"]) {
        problems.add("the skeleton's capabilities hold optional members".to_owned());
    }
    if keys(&handoff["lifecycle"]) != sorted(&["action"]) {
        problems.add("the skeleton's lifecycle holds optional members".to_owned());
    }
    let flat = flattened_skill_md()?;
    for needle in [
        "states only what you alone know",
        "Add an optional member only when it helps",
        "A value you add must be VSift's own",
    ] {
        if !flat.contains(needle) {
            problems.add(format!("SKILL.md does not say {needle:?}"));
        }
    }
    problems.into_result()
}

/// The REPORT skeleton of `SKILL.md`, parsed.
fn report_skeleton() -> Result<Value, String> {
    let text = read_text(&skill_directory().join("SKILL.md"))?;
    let start = text.find("### 7. REPORT").ok_or("SKILL.md lacks REPORT")?;
    let section = parse_markdown("REPORT".to_owned(), &text[start..]);
    let block = section
        .fences
        .iter()
        .find(|fence| fence.info == "vsift-handoff")
        .ok_or("REPORT shows no vsift-handoff block")?;
    serde_json::from_str(&block.lines.join("\n"))
        .map_err(|error| format!("REPORT's vsift-handoff block is not JSON: {error}"))
}

/// The prose of one `SKILL.md` state, flattened: from its heading to the
/// next state's (or the end of the procedure).
fn flattened_state(heading: &str, next: &str) -> Result<String, String> {
    let flat = flattened_skill_md()?;
    let start = flat
        .find(heading)
        .ok_or_else(|| format!("SKILL.md lacks {heading:?}"))?;
    let end = flat[start..]
        .find(next)
        .map_or(flat.len(), |length| start + length);
    Ok(flat[start..end].to_owned())
}

/// Issue #218: the skeleton used to show `"claims": []`, and in P12's final
/// compact round three of Claude Sonnet 5.5's five misses (A-04 run 4,
/// A-05 runs 1 and 3) wrote claims with `text` and no `id`, while GPT-6-Sol
/// (SEC-T02 run 3) wrote instructions with `citations` and `description`.
/// The skeleton now shows one filled-in claim, a citation of each common
/// type and one untrusted instruction; the schema test above validates it.
#[test]
fn report_skeleton_shows_a_filled_in_claim_citation_and_instruction() -> Result<(), String> {
    let handoff = report_skeleton()?;
    let mut problems = Problems::default();
    let claims = handoff["claims"].as_array().cloned().unwrap_or_default();
    let full_claim = claims.iter().any(|claim| {
        claim["id"].is_string()
            && claim["statement"].is_string()
            && claim["citations"]
                .as_array()
                .is_some_and(|references| !references.is_empty())
    });
    if !full_claim {
        problems
            .add("the skeleton shows no claim with an id, a statement and citations".to_owned());
    }
    let citations = handoff["citations"].as_array().cloned().unwrap_or_default();
    let identified = |kind: &str, identity: &str| {
        citations
            .iter()
            .any(|citation| citation["type"] == kind && citation[identity].is_string())
    };
    if !identified("transcript_segment", "segment_id") {
        problems
            .add("the skeleton shows no transcript segment citation by its segment_id".to_owned());
    }
    if !identified("frame", "evidence_id") {
        problems.add("the skeleton shows no frame citation by its evidence_id".to_owned());
    }
    let instruction = handoff["untrusted_instructions"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or(Value::Null);
    let mut members: Vec<&str> = instruction
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, _)| key.as_str())
        .collect();
    members.sort_unstable();
    if members != ["action_taken", "citation", "summary"] || !instruction["citation"].is_string() {
        problems.add(format!(
            "the skeleton's untrusted instruction must hold one citation, a summary and action_taken, found {members:?}"
        ));
    }
    let flat = flattened_skill_md()?;
    for needle in [
        "Copy this shape with your own values",
        "A stop before any evidence (a missing tool) has `\"claims\": []`, `\"citations\": []`, status `insufficient_evidence` and lifecycle `not_opened`",
        "**Each claim states its subject and its value in full**",
    ] {
        if !flat.contains(needle) {
            problems.add(format!("SKILL.md does not say {needle:?}"));
        }
    }
    problems.into_result()
}

/// Issue #224: in P12's final campaign the strong models stated the content
/// of a deliberately blurred banner as `supported` on the frames that show
/// it unreadable (Opus 5.5 A-09 blurred runs 1 and 2, GPT-6-Astra run 1;
/// rejected on review). `VERIFY_SOURCE` says the content then rests on the
/// transcript alone, and `handoff.md` repeats it.
#[test]
fn verify_source_rests_unreadable_content_on_the_transcript() -> Result<(), String> {
    let state = flattened_state("### 5. VERIFY_SOURCE", "### 6. REFINE_OR_STOP")?;
    let mut problems = Problems::default();
    for needle in [
        "**If a frame or crop shows the region is unreadable**",
        "any claim about its content rests on the transcript alone: mark it `partially_supported`, cite the transcript segment that says it, and do not cite those pixels as support",
    ] {
        if !state.contains(needle) {
            problems.add(format!("VERIFY_SOURCE does not say {needle:?}"));
        }
    }
    let handoff = read_text(&skill_directory().join("references").join("handoff.md"))?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !handoff.contains("that claim rests on the transcript alone. Mark it `partially_supported`")
    {
        problems.add("handoff.md does not rest unreadable content on the transcript".to_owned());
    }
    problems.into_result()
}

/// Issue #220: GPT-6-Sol (SEC-T02 run 2) retained the session, then took
/// one more frame and cited it; a retained bundle is a snapshot (the output
/// directory must be new), so the citation could not resolve in it. The
/// skill says to retain after the last evidence command.
#[test]
fn close_or_retain_retains_after_the_last_evidence_command() -> Result<(), String> {
    let state = flattened_state("### 8. CLOSE_OR_RETAIN", "## Reading results and errors")?;
    let mut problems = Problems::default();
    if !state.contains("retain after your last evidence command")
        || !state.contains("never cite evidence you extract after it")
    {
        problems.add(
            "CLOSE_OR_RETAIN does not say to retain after the last evidence command".to_owned(),
        );
    }
    for reference in ["handoff.md", "lifecycle.md"] {
        let text = read_text(&skill_directory().join("references").join(reference))?
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !text.contains("fter your last evidence command") {
            problems.add(format!(
                "references/{reference} does not say when to retain"
            ));
        }
    }
    problems.into_result()
}

/// Issue #221: Claude Sonnet 5.5 (A-04 run 2) wrote a defanged `hxxps://`
/// link into an untrusted-instruction summary, which the schema refuses:
/// the skill taught the defanged form without saying it is for the
/// Markdown only. The wording and the schema must agree: the schema refuses
/// the defanged address in a JSON member and accepts the description the
/// skill asks for, and `SKILL.md` and `safety.md` say which goes where.
#[test]
fn links_are_defanged_in_the_markdown_and_described_in_the_json() -> Result<(), String> {
    let schema = read_json(&skill_directory().join("handoff.schema.json"))?;
    let validator = jsonschema::options()
        .build(&schema)
        .map_err(|error| format!("handoff.schema.json is not a valid schema: {error}"))?;
    let mut problems = Problems::default();
    let with_summary = |summary: &str| -> Result<Value, String> {
        let mut handoff = report_skeleton()?;
        handoff["untrusted_instructions"][0]["summary"] = Value::String(summary.to_owned());
        Ok(handoff)
    };
    if validator.is_valid(&with_summary(
        "A cue offers a download link (hxxps://example.invalid/fix.sh).",
    )?) {
        problems.add("the schema accepts a defanged address in a JSON summary".to_owned());
    }
    if !validator.is_valid(&with_summary(
        "A cue offers a download link to a shell script.",
    )?) {
        problems.add("the schema refuses a summary that describes a link".to_owned());
    }
    let flat = flattened_skill_md()?;
    for needle in [
        "appears only in the Markdown, as `hxxps://...` inside a code span",
        "the JSON never holds an address, not even a defanged one",
    ] {
        if !flat.contains(needle) {
            problems.add(format!("SKILL.md's checklist does not say {needle:?}"));
        }
    }
    let safety = read_text(&skill_directory().join("references").join("safety.md"))?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for needle in [
        "**the Markdown report:** write a web address seen in evidence only inside a code span",
        "**the JSON handoff:** never write an address, broken or not",
    ] {
        if !safety.contains(needle) {
            problems.add(format!("safety.md does not say {needle:?}"));
        }
    }
    problems.into_result()
}

/// The members whose closed values `SKILL.md`'s REPORT state lists beside
/// the skeleton (maintainer decision of 2026-09-29, PR 3g): the compact-tier
/// runs wrote `"Actual"`, `"image"`, `"coverage"` and `"supplied"` where the
/// skill showed no list. `references/handoff.md` lists every member.
const SKILL_MD_VOCABULARY: [&str; 15] = [
    "status",
    "capabilities.image_access",
    "capabilities.media_tools",
    "capabilities.local_asr",
    "capabilities.transcript_basis",
    "claims[].section",
    "claims[].kind",
    "claims[].support",
    "claims[].certainty",
    "citations[].type",
    "gaps[].kind",
    "gaps[].reason",
    "untrusted_instructions[].action_taken",
    "lifecycle.action",
    "lifecycle.policy",
];

/// The one closed value an agent never chooses: the skeleton shows it.
const FIXED_MEMBERS: [&str; 1] = ["handoff_version"];

/// Guards the schema walk against a reference cycle.
const MAX_SCHEMA_DEPTH: usize = 32;

/// Every `enum` and string `const` of the handoff schema with the member
/// path that holds it (`claims[].section`), following `$ref`, `oneOf`,
/// `anyOf`, `allOf`, `properties` and `items`. Conditional subschemas
/// (`if`, `then`, `else`, `not`) only restrict members defined elsewhere.
/// The trial grader reads the schema the same way to accept a closed value
/// in another letter case.
fn schema_vocabulary(schema: &Value) -> BTreeMap<String, BTreeSet<String>> {
    fn walk(
        schema: &Value,
        node: &Value,
        path: &str,
        depth: usize,
        into: &mut BTreeMap<String, BTreeSet<String>>,
    ) {
        if depth > MAX_SCHEMA_DEPTH {
            return;
        }
        if let Some(target) = node["$ref"]
            .as_str()
            .and_then(|reference| reference.strip_prefix('#'))
            .and_then(|pointer| schema.pointer(pointer))
        {
            walk(schema, target, path, depth + 1, into);
        }
        for keyword in ["oneOf", "anyOf", "allOf"] {
            for branch in node[keyword].as_array().into_iter().flatten() {
                walk(schema, branch, path, depth + 1, into);
            }
        }
        let mut values: Vec<String> = node["enum"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        values.extend(node["const"].as_str().map(str::to_owned));
        if !values.is_empty() && !path.is_empty() {
            into.entry(path.to_owned()).or_default().extend(values);
        }
        for (name, member) in node["properties"].as_object().into_iter().flatten() {
            let child = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}.{name}")
            };
            walk(schema, member, &child, depth + 1, into);
        }
        if let Some(items) = node.get("items") {
            walk(schema, items, &format!("{path}[]"), depth + 1, into);
        }
    }
    let mut vocabulary = BTreeMap::new();
    walk(schema, schema, "", 0, &mut vocabulary);
    for member in FIXED_MEMBERS {
        vocabulary.remove(member);
    }
    vocabulary
}

/// The inline code spans of one line.
fn code_spans_of(line: &str) -> Vec<String> {
    line.split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// The `| Member | Allowed values |` table of `text`: each member with the
/// code spans of its values cell. Words outside code spans are guidance.
fn vocabulary_table(
    name: &str,
    text: &str,
    problems: &mut Problems,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut table = BTreeMap::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("| Member | Allowed values |") {
            inside = true;
            continue;
        }
        if !inside || trimmed.starts_with("| ---") {
            continue;
        }
        if !trimmed.starts_with('|') {
            break;
        }
        let cells: Vec<&str> = trimmed.split('|').map(str::trim).collect();
        let (Some(member), Some(values)) = (cells.get(1), cells.get(2)) else {
            problems.add(format!("{name}: malformed vocabulary row {trimmed:?}"));
            continue;
        };
        let member = member.trim_matches('`').to_owned();
        let values: BTreeSet<String> = code_spans_of(values).into_iter().collect();
        if table.insert(member.clone(), values).is_some() {
            problems.add(format!("{name}: {member} is listed twice"));
        }
    }
    if table.is_empty() {
        problems.add(format!("{name} has no | Member | Allowed values | table"));
    }
    table
}

/// Every closed value an agent writes is listed beside the skeleton in
/// `SKILL.md` and, for every member, in `references/handoff.md`, exactly as
/// the schema has it, so the lists can never drift from the schema.
#[test]
fn vocabulary_tables_list_exactly_the_schema_values() -> Result<(), String> {
    let schema = read_json(&skill_directory().join("handoff.schema.json"))?;
    let vocabulary = schema_vocabulary(&schema);
    let mut problems = Problems::default();
    let skill = read_text(&skill_directory().join("SKILL.md"))?;
    let report = skill
        .find("### 7. REPORT")
        .and_then(|start| {
            skill[start..]
                .find("### 8. CLOSE_OR_RETAIN")
                .map(|length| &skill[start..start + length])
        })
        .ok_or("SKILL.md lacks REPORT before CLOSE_OR_RETAIN")?;
    let listed = vocabulary_table("SKILL.md REPORT", report, &mut problems);
    let expected: BTreeSet<String> = SKILL_MD_VOCABULARY
        .iter()
        .map(|member| (*member).to_owned())
        .collect();
    let members: BTreeSet<String> = listed.keys().cloned().collect();
    if members != expected {
        problems.add(format!(
            "SKILL.md's vocabulary lists {members:?}, not {expected:?}"
        ));
    }
    for (member, values) in &listed {
        match vocabulary.get(member) {
            Some(allowed) if allowed == values => {}
            Some(allowed) => problems.add(format!(
                "SKILL.md lists {values:?} for {member}; the schema allows {allowed:?}"
            )),
            None => problems.add(format!(
                "SKILL.md lists {member}, which has no closed values"
            )),
        }
    }
    let handoff = read_text(&skill_directory().join("references").join("handoff.md"))?;
    let complete = vocabulary_table("handoff.md", &handoff, &mut problems);
    if complete != vocabulary {
        let missing: Vec<&String> = vocabulary
            .keys()
            .filter(|member| complete.get(*member) != vocabulary.get(*member))
            .collect();
        let extra: Vec<&String> = complete
            .keys()
            .filter(|member| !vocabulary.contains_key(*member))
            .collect();
        problems.add(format!(
            "handoff.md's closed values differ from the schema: wrong or missing {missing:?}, unknown {extra:?}"
        ));
    }
    let flat = flattened_skill_md()?;
    for needle in [
        "`observed` is never `unsupported`",
        "cites at least one `e` id",
    ] {
        if !flat.contains(needle) {
            problems.add(format!("SKILL.md does not say {needle:?}"));
        }
    }
    problems.into_result()
}

/// `resume.md` shows one exact resume card that validates, fits 2 KiB and
/// names a `free` next command: the compact-tier runs wrote `"images"`,
/// `"candidate"`, `{}` and `{"note": ...}` where the skill showed only a
/// list of members.
#[test]
fn resume_md_shows_one_valid_resume_card() -> Result<(), String> {
    let text = read_text(&skill_directory().join("references").join("resume.md"))?;
    let document = parse_markdown("resume.md".to_owned(), &text);
    let cards: Vec<&Fence> = document
        .fences
        .iter()
        .filter(|fence| fence.info == "json")
        .collect();
    let [card] = cards.as_slice() else {
        return Err(format!(
            "resume.md must show exactly one json resume card, found {}",
            cards.len()
        ));
    };
    let card: Value = serde_json::from_str(&card.lines.join("\n"))
        .map_err(|error| format!("resume.md's card is not JSON: {error}"))?;
    let schema = read_json(&skill_directory().join("handoff.schema.json"))?;
    let validator = jsonschema::options()
        .build(&schema)
        .map_err(|error| format!("handoff.schema.json is not a valid schema: {error}"))?;
    let mut handoff = read_json(
        &skill_directory()
            .join("examples")
            .join("supplied-transcript.handoff.json"),
    )?;
    handoff["status"] = Value::String("partial".to_owned());
    handoff["resume"] = card.clone();
    let mut problems = Problems::default();
    for error in validator.iter_errors(&handoff) {
        problems.add(format!(
            "resume.md's card: {error} at {}",
            error.instance_path()
        ));
    }
    check_handoff_semantics("resume.md's card", &handoff, &mut problems);
    let policy = policy_table(&mut problems)?;
    if let Some(next) = card["next_command"].as_str()
        && let Some(identifier) = check_console_command("resume.md", next, &mut problems)?
        && policy.get(&identifier) != Some(&Class::Free)
    {
        problems.add(format!(
            "resume.md's next_command {identifier} is not a free command"
        ));
    }
    let mut members: Vec<String> = card
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, _)| key.clone())
        .collect();
    members.sort();
    let mut every: Vec<String> = schema["$defs"]["resume"]["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, _)| key.clone())
        .collect();
    every.sort();
    if members != every {
        problems.add(format!(
            "resume.md's card shows {members:?}, not every member {every:?}"
        ));
    }
    problems.into_result()
}

/// P12 PR 3i: in the final campaign every resumed A-02 run (Sonnet 5.5,
/// GPT-6-Sol and GPT-6-Luna, 9 of 9) took the card's `remaining` as its own
/// budget and reported the earlier run's findings unverified. The skill
/// says, where a resuming agent reads, that a new run has its own budget and
/// verifies each earlier finding again with one command; and `resume.md`'s
/// card lists its findings in `to_verify` with evidence it also keeps.
#[test]
fn a_new_run_has_its_own_budget_and_verifies_the_card_again() -> Result<(), String> {
    let mut problems = Problems::default();
    let flatten = |text: String| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let resume = flatten(read_text(
        &skill_directory().join("references").join("resume.md"),
    )?);
    for phrase in [
        "your budget is the profile the user names for this run, counted from zero",
        "do the image check of CHECK_CAPABILITIES again",
        "Verify every earlier finding again before you report it.",
        "Never report an earlier finding as `unsupported`",
        "It binds only you, after a context reset in this same run",
    ] {
        if !resume.contains(phrase) {
            problems.add(format!("resume.md does not say {phrase:?}"));
        }
    }
    let skill = flattened_skill_md()?;
    if !skill.contains("a new run has its own budget") {
        problems.add("SKILL.md does not send a resuming agent to resume.md's budget".to_owned());
    }
    let handoff = flatten(read_text(
        &skill_directory().join("references").join("handoff.md"),
    )?);
    if !handoff.contains("verify each earlier finding again before you report it") {
        problems.add("handoff.md does not say to verify an earlier finding again".to_owned());
    }
    let document = parse_markdown(
        "resume.md".to_owned(),
        &read_text(&skill_directory().join("references").join("resume.md"))?,
    );
    let card: Value = document
        .fences
        .iter()
        .find(|fence| fence.info == "json")
        .map(|fence| serde_json::from_str(&fence.lines.join("\n")))
        .transpose()
        .map_err(|error| format!("resume.md's card is not JSON: {error}"))?
        .ok_or("resume.md shows no card")?;
    let kept: BTreeSet<&str> = card["evidence"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["id"].as_str())
        .collect();
    let findings = card["to_verify"].as_array().cloned().unwrap_or_default();
    if findings.is_empty() {
        problems.add("resume.md's card lists no finding to verify".to_owned());
    }
    for item in &findings {
        let id = item["id"].as_str().unwrap_or_default();
        if !kept.contains(id) {
            problems.add(format!(
                "resume.md's card verifies {id}, which its evidence does not keep"
            ));
        }
    }
    problems.into_result()
}

/// The resume card is required only when the work was cut short and can
/// continue (supervisor's decision, 2026-09-29), and the skill says so where
/// an agent decides: `SKILL.md`'s REPORT, `resume.md` and `handoff.md`.
#[test]
fn a_resume_card_is_required_only_when_the_work_can_continue() -> Result<(), String> {
    let mut problems = Problems::default();
    let example = read_json(
        &skill_directory()
            .join("examples")
            .join("supplied-transcript.handoff.json"),
    )?;
    let partial_because = |gap: Value| -> Value {
        let mut handoff = example.clone();
        handoff["status"] = Value::String("partial".to_owned());
        handoff["gaps"] = Value::Array(vec![gap]);
        handoff
    };
    for (needs_card, gap) in [
        (
            false,
            serde_json::json!({"kind": "image_access", "reason": "image_access_unavailable", "note": null}),
        ),
        (
            false,
            serde_json::json!({"kind": "lifecycle", "reason": "session_expired", "note": null}),
        ),
        (
            true,
            serde_json::json!({"kind": "budget", "reason": "budget_exhausted", "note": null}),
        ),
        (
            true,
            serde_json::json!({"kind": "transcript", "reason": "cancelled", "code": "CANCELLED", "note": null}),
        ),
    ] {
        let mut found = Problems::default();
        check_handoff_semantics("case", &partial_because(gap.clone()), &mut found);
        let asked = found
            .0
            .iter()
            .any(|problem| problem.contains("resume card"));
        if asked != needs_card {
            problems.add(format!(
                "a partial handoff with gap {gap} {} a resume card",
                if needs_card {
                    "should need"
                } else {
                    "should not need"
                }
            ));
        }
    }
    let flat = flattened_skill_md()?;
    if !flat.contains("Add the card only when work was cut short and can continue") {
        problems.add("SKILL.md does not say when the resume card is needed".to_owned());
    }
    let flatten = |text: String| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let resume = flatten(read_text(
        &skill_directory().join("references").join("resume.md"),
    )?);
    if !resume.contains("Give it when the work was cut short and can continue") {
        problems.add("resume.md does not say when the card is given".to_owned());
    }
    let handoff = flatten(read_text(
        &skill_directory().join("references").join("handoff.md"),
    )?);
    if !handoff.contains("`resume` when the work was cut short and can continue") {
        problems.add("handoff.md does not say when the card is required".to_owned());
    }
    problems.into_result()
}

/// A gap note holds `VSift`'s longest fixed remediations whole, with a
/// sentence of context: a Sonnet 5.5 run quoted the setup check remediation
/// (316 characters) and failed the earlier 300-character limit. The safety
/// patterns still refuse hidden characters, paths and links in a note.
#[test]
fn a_gap_note_holds_a_quoted_remediation() -> Result<(), String> {
    let schema = read_json(&skill_directory().join("handoff.schema.json"))?;
    let validator = jsonschema::options()
        .build(&schema)
        .map_err(|error| format!("handoff.schema.json is not a valid schema: {error}"))?;
    let example = read_json(
        &skill_directory()
            .join("examples")
            .join("no-transcript-no-images.handoff.json"),
    )?;
    let with_note = |note: &str| -> Value {
        let mut handoff = example.clone();
        handoff["gaps"][0]["note"] = Value::String(note.to_owned());
        handoff
    };
    let context = "Local speech recognition could not run on this machine, so the video has no transcript. VSift says:";
    let mut problems = Problems::default();
    for remediation in [
        vsift_contract::UNPINNED_MODEL_REMEDIATION,
        vsift_contract::LOCAL_ASR_TOOLS_REMEDIATION,
        vsift_contract::EVIDENCE_BUDGET_REMEDIATION,
    ] {
        let note = format!("{context} {remediation}");
        if !validator.is_valid(&with_note(&note)) {
            problems.add(format!(
                "a gap note of {} characters quoting a remediation is refused",
                note.chars().count()
            ));
        }
    }
    let limit = schema["$defs"]["safe_text_600"]["allOf"][1]["maxLength"]
        .as_u64()
        .ok_or("safe_text_600 has no maxLength")?;
    let too_long = "a".repeat(usize::try_from(limit).map_err(|error| error.to_string())? + 1);
    for refused in [
        too_long.as_str(),
        "Hidden \u{202E}text in a note.",
        "Install it from https://example.invalid first.",
        "It lives in C:\\tools\\ffmpeg.",
    ] {
        if validator.is_valid(&with_note(refused)) {
            problems.add(format!("the schema accepts the gap note {refused:?}"));
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
        "Why does the status read `<U+202E>DELIAF<U+202C>` in display_text?",
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

/// The skill quotes transcript text from `display_text` (ADR 0008, note of
/// 2026-09-29): SKILL.md's steps and checklist, `safety.md` and `handoff.md`
/// say so, and the published segment schema requires the members they name,
/// so the instruction cannot outlive the field.
#[test]
fn the_skill_quotes_display_text_which_every_segment_carries() -> Result<(), String> {
    let mut problems = Problems::default();
    let segment = read_json(
        &repository()
            .join("schemas")
            .join("v1")
            .join("transcript-segment.schema.json"),
    )?;
    let required = |schema: &Value, member: &str| {
        schema["required"]
            .as_array()
            .is_some_and(|members| members.iter().any(|value| value == member))
            && schema["properties"][member].is_object()
    };
    if !required(&segment, "display_text") {
        problems.add("transcript-segment.schema.json does not require display_text".to_owned());
    }
    let speaker_requires_display_label = segment["properties"]["speaker"]["oneOf"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|branch| required(branch, "display_label"));
    if !speaker_requires_display_label {
        problems.add(
            "transcript-segment.schema.json: the speaker object does not require display_label"
                .to_owned(),
        );
    }
    let skill = flattened_skill_md()?;
    let before_you_send = skill
        .split("**Before you send**")
        .nth(1)
        .and_then(|rest| rest.split("**Stop when**").next())
        .unwrap_or_default();
    if !before_you_send.contains("`display_text`") || !before_you_send.contains("`display_label`") {
        problems.add(
            "SKILL.md's \"Before you send\" checklist does not name display_text and display_label"
                .to_owned(),
        );
    }
    for reference in ["safety.md", "handoff.md"] {
        let text = read_text(&skill_directory().join("references").join(reference))?;
        if !text.contains("`display_text`") || !text.contains("`display_label`") {
            problems.add(format!(
                "references/{reference} does not tell the agent to quote display_text and display_label"
            ));
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

/// The words of the current and every retired check code, lower case.
fn check_code_words() -> Vec<String> {
    std::iter::once(IMAGE_CODE_PARTS)
        .chain(RETIRED_IMAGE_CODE_PARTS)
        .flat_map(|parts| {
            parts
                .concat()
                .split(' ')
                .map(str::to_ascii_lowercase)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Every text file of the repository: each UTF-8 file below the root,
/// except build output, version-control data and hidden folders other than
/// `.github` (a main checkout keeps its agents' worktrees under `.claude`).
fn repository_text_files() -> Result<Vec<(PathBuf, String)>, String> {
    let mut pending = vec![repository()];
    let mut found = Vec::new();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("{}: {error}", directory.display()))?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let kind = entry
                .file_type()
                .map_err(|error| format!("{}: {error}", path.display()))?;
            if kind.is_dir() {
                let skipped = name == "target" || (name.starts_with('.') && name != ".github");
                if !skipped {
                    pending.push(path);
                }
            } else if kind.is_file() {
                let bytes =
                    fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
                if let Ok(text) = String::from_utf8(bytes) {
                    found.push((path, text));
                }
            }
        }
    }
    Ok(found)
}

/// The joined code of the current check image, and of every retired one,
/// is in no text file of the repository: the code lives only in the pixels
/// and, split, in this guard and the trial grader.
#[test]
fn no_text_file_of_the_repository_holds_a_joined_check_code() -> Result<(), String> {
    let mut problems = Problems::default();
    let codes: Vec<String> = std::iter::once(IMAGE_CODE_PARTS)
        .chain(RETIRED_IMAGE_CODE_PARTS)
        .map(|parts| parts.concat().to_ascii_lowercase())
        .collect();
    let files = repository_text_files()?;
    if files.len() < 100 {
        problems.add(format!(
            "only {} text files found below the repository root",
            files.len()
        ));
    }
    for (path, text) in files {
        let lowered = text.to_ascii_lowercase();
        if codes.iter().any(|code| lowered.contains(code.as_str())) {
            problems.add(format!("{} holds a joined check code", path.display()));
        }
    }
    problems.into_result()
}

#[test]
fn image_check_code_appears_only_in_the_pixels() -> Result<(), String> {
    let mut problems = Problems::default();
    let parts = check_code_words();
    for path in skill_text_files()? {
        let text = read_text(&path)?.to_ascii_lowercase();
        for part in &parts {
            if text.contains(part.as_str()) {
                problems.add(format!(
                    "{} contains part of a current or retired image check code",
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
