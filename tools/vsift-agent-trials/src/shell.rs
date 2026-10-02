//! Splits the command text a client ran into simple commands.
//!
//! Both clients report shell commands as text: Claude Code's `Bash` tool
//! input is a script (POSIX shell syntax, also on Windows where it runs Git
//! Bash), and Codex reports the argument list it spawned joined into one
//! string, usually a shell wrapper (`bash -lc '...'` or
//! `powershell.exe -Command "..."`) around the model's script. The grader
//! needs the executables and arguments inside. This is a deliberately
//! conservative reader, not a shell: anything it cannot analyse with
//! certainty (command substitution, script blocks, encoded commands, deep
//! nesting) is reported as opaque, and the grader treats an opaque command
//! as unauthorized. A model that wants to pass never needs such syntax.
//!
//! **The one input exception** (P13 PR 5, ADR 0023 decision G): the skill
//! passes the agent's draft report to `vsift handoff check` in exactly two
//! literal forms, each for its own shell: a quoted heredoc for a POSIX shell
//! (`sh`, `bash`, including Git Bash on Windows) and a single-quoted
//! here-string piped in for PowerShell. A script that is exactly the form of
//! the dialect it is read in, at any wrapper depth, is that one `vsift
//! handoff check --json` command; its body is the draft, data that is never
//! read as commands. The form of the other shell is not recognised: in bash
//! `@'...'@` is `@` and a single-quoted string that the draft's first
//! apostrophe ends, so the rest of the draft would run as commands; in
//! PowerShell `<<` is a parse error. Anything else, however close (an
//! unquoted heredoc, a double-quoted here-string, another command piped in,
//! text after the closing line, the other shell's form), is read as ordinary
//! shell text and stays strict.

use serde::{Deserialize, Serialize};

/// Which quoting rules a script follows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dialect {
    /// `sh`/`bash`: backslash escapes outside single quotes.
    Posix,
    /// PowerShell: backslash is a path separator; backtick escapes inside
    /// double quotes. Reached by unwrapping `powershell`/`pwsh -Command`.
    PowerShell,
    /// `cmd /c`: read with PowerShell's quoting rules, but it has no
    /// here-strings, so the PowerShell draft form is not recognised there.
    Cmd,
}

/// One simple command of a script.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SimpleCommand {
    /// The executable as written, then its arguments.
    pub argv: Vec<String>,
    /// Output redirections other than to the null device or another stream.
    pub writes_file: bool,
    /// Whether this command reads the previous command's output (`a | b`).
    pub piped_from_previous: bool,
}

impl SimpleCommand {
    /// The executable's base name, lower case, without a Windows extension:
    /// `C:\x\VSift.exe` and `./vsift` are both `vsift`.
    #[must_use]
    pub fn program(&self) -> String {
        self.argv
            .first()
            .map_or_else(String::new, |first| program_name(first))
    }
}

/// The analysis of one command text.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ParsedScript {
    /// Every simple command, shell wrappers unwrapped, in order.
    pub commands: Vec<SimpleCommand>,
    /// Why the text could not be analysed with certainty, if it could not.
    pub opaque: Option<String>,
    /// Whether an unquoted or double-quoted variable was expanded.
    pub expands_variables: bool,
    /// The shell each `vsift` command ran in, in order: the innermost
    /// wrapper that was unwrapped to reach it (`Posix` when none was). It
    /// tells which of npm's command shims a call resolved to on Windows (P14,
    /// #257); it is a reading of the command text, never serialised.
    #[serde(skip)]
    pub vsift_shells: Vec<Dialect>,
}

/// The base name of an executable path, lower case, without `.exe`,
/// `.cmd`, `.bat`, `.com` or `.ps1`.
#[must_use]
pub fn program_name(path: &str) -> String {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let lowered = base.to_ascii_lowercase();
    for extension in [".exe", ".cmd", ".bat", ".com", ".ps1"] {
        if let Some(stem) = lowered.strip_suffix(extension) {
            return stem.to_owned();
        }
    }
    lowered
}

const MAX_NESTING: usize = 4;

/// The first line of the POSIX draft form.
const HEREDOC_OPENING: &str = "vsift handoff check --json <<'VSIFT_HANDOFF'";
/// The line that ends the POSIX draft form.
const HEREDOC_CLOSING: &str = "VSIFT_HANDOFF";
/// The first line of the PowerShell draft form.
const HERE_STRING_OPENING: &str = "@'";
/// The line that ends the PowerShell draft form.
const HERE_STRING_CLOSING: &str = "'@ | vsift handoff check --json";
/// The command both draft forms run.
const HANDOFF_CHECK_ARGV: [&str; 4] = ["vsift", "handoff", "check", "--json"];

/// Whether `text` is exactly the draft form of `vsift handoff check` that
/// belongs to `dialect`: the quoted heredoc in a POSIX shell, the
/// single-quoted here-string in PowerShell, neither in `cmd`. The heredoc
/// ends at the first line that is exactly its delimiter, and the here-string
/// at the first line that starts with `'@`, as the shells end them; only
/// white space may follow.
#[must_use]
pub fn is_handoff_check_form(text: &str, dialect: Dialect) -> bool {
    let lines: Vec<&str> = text.trim().lines().collect();
    let Some((first, body)) = lines.split_first() else {
        return false;
    };
    let end = match (*first, dialect) {
        (HEREDOC_OPENING, Dialect::Posix) => body.iter().position(|line| *line == HEREDOC_CLOSING),
        (HERE_STRING_OPENING, Dialect::PowerShell) => body
            .iter()
            .position(|line| line.starts_with("'@"))
            .filter(|&index| body[index] == HERE_STRING_CLOSING),
        _ => None,
    };
    end.is_some_and(|end| body[end + 1..].iter().all(|line| line.trim().is_empty()))
}

/// Analyses a command text in `dialect`, unwrapping shell wrappers.
#[must_use]
pub fn parse_script(text: &str, dialect: Dialect) -> ParsedScript {
    let mut result = ParsedScript::default();
    parse_into(text, dialect, 0, &mut result);
    result
}

fn parse_into(text: &str, dialect: Dialect, depth: usize, result: &mut ParsedScript) {
    if depth > MAX_NESTING {
        result.opaque = Some("shell wrappers nested too deeply".to_owned());
        return;
    }
    if is_handoff_check_form(text, dialect) {
        result.commands.push(SimpleCommand {
            argv: HANDOFF_CHECK_ARGV.map(str::to_owned).to_vec(),
            writes_file: false,
            piped_from_previous: false,
        });
        result.vsift_shells.push(dialect);
        return;
    }
    let tokens = match tokenize(text, dialect) {
        Ok(tokens) => tokens,
        Err(reason) => {
            result.opaque = Some(reason);
            return;
        }
    };
    if tokens.expands {
        result.expands_variables = true;
    }
    for command in tokens.commands {
        match unwrap_shell(&command) {
            Unwrapped::Script(script, inner) => parse_into(&script, inner, depth + 1, result),
            Unwrapped::Opaque(reason) => result.opaque = Some(reason),
            Unwrapped::Plain => {
                if command.program() == "vsift" {
                    result.vsift_shells.push(shim_dialect(&command, dialect));
                }
                result.commands.push(command);
            }
        }
    }
}

/// The shell whose shim a `vsift` command resolves to: the command's own
/// extension when it names one (`vsift.cmd` is the `cmd.exe` shim wherever it
/// is typed, `vsift.ps1` the PowerShell one), otherwise the shell it ran in.
fn shim_dialect(command: &SimpleCommand, shell: Dialect) -> Dialect {
    // Windows path separators are read here whatever the host, as `program_name`
    // does, so the extension is taken from the last component by hand.
    let first = command.argv.first().map_or("", String::as_str);
    let name = first.rsplit(['/', '\\']).next().unwrap_or(first);
    let extension = name.rsplit_once('.').map_or("", |(_, extension)| extension);
    if extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat") {
        Dialect::Cmd
    } else if extension.eq_ignore_ascii_case("ps1") {
        Dialect::PowerShell
    } else {
        shell
    }
}

enum Unwrapped {
    Script(String, Dialect),
    Opaque(String),
    Plain,
}

/// Recognises `bash -lc SCRIPT`, `powershell -Command SCRIPT`,
/// `cmd /c SCRIPT` and their relatives.
fn unwrap_shell(command: &SimpleCommand) -> Unwrapped {
    let program = command.program();
    let arguments = command.argv.get(1..).unwrap_or_default();
    match program.as_str() {
        "bash" | "sh" | "zsh" | "dash" => {
            let flag = arguments.iter().position(|argument| {
                argument.starts_with('-') && !argument.starts_with("--") && argument.ends_with('c')
            });
            match flag.and_then(|index| arguments.get(index + 1)) {
                Some(script) => Unwrapped::Script(script.clone(), Dialect::Posix),
                None => Unwrapped::Plain,
            }
        }
        "powershell" | "pwsh" => {
            let mut index = 0;
            while index < arguments.len() {
                let lowered = arguments[index].to_ascii_lowercase();
                match lowered.as_str() {
                    "-noprofile" | "-nologo" | "-noninteractive" | "-nop" => index += 1,
                    "-executionpolicy" | "-ep" | "-inputformat" | "-outputformat" => index += 2,
                    "-command" | "-c" => {
                        return Unwrapped::Script(
                            arguments[index + 1..].join(" "),
                            Dialect::PowerShell,
                        );
                    }
                    "-encodedcommand" | "-enc" | "-e" | "-ec" => {
                        return Unwrapped::Opaque("an encoded PowerShell command".to_owned());
                    }
                    _ => return Unwrapped::Plain,
                }
            }
            Unwrapped::Plain
        }
        "cmd" => match arguments.first().map(|first| first.to_ascii_lowercase()) {
            Some(flag) if flag == "/c" || flag == "/k" => {
                Unwrapped::Script(arguments[1..].join(" "), Dialect::Cmd)
            }
            _ => Unwrapped::Plain,
        },
        _ => Unwrapped::Plain,
    }
}

struct Tokens {
    commands: Vec<SimpleCommand>,
    expands: bool,
}

/// Redirection targets that write no file.
const HARMLESS_TARGETS: [&str; 5] = ["/dev/null", "$null", "nul", "&1", "&2"];

#[allow(
    clippy::struct_excessive_bools,
    reason = "Each flag is one independent piece of lexer state, not a mode switch"
)]
struct Lexer {
    commands: Vec<SimpleCommand>,
    current: SimpleCommand,
    word: String,
    in_word: bool,
    expands: bool,
    pending_redirect: bool,
    next_piped: bool,
}

impl Lexer {
    fn new() -> Self {
        Self {
            commands: Vec::new(),
            current: empty_command(false),
            word: String::new(),
            in_word: false,
            expands: false,
            pending_redirect: false,
            next_piped: false,
        }
    }

    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        let word = std::mem::take(&mut self.word);
        self.in_word = false;
        if self.pending_redirect {
            self.pending_redirect = false;
            if !HARMLESS_TARGETS.contains(&word.to_ascii_lowercase().as_str()) {
                self.current.writes_file = true;
            }
        } else if !(self.current.argv.is_empty() && word == "&") {
            self.current.argv.push(word);
        }
    }

    fn end_command(&mut self, piped_next: bool) {
        self.end_word();
        if self.pending_redirect {
            self.current.writes_file = true;
            self.pending_redirect = false;
        }
        let finished = std::mem::replace(&mut self.current, empty_command(piped_next));
        if !finished.argv.is_empty() {
            self.commands.push(finished);
        }
        self.next_piped = piped_next;
    }
}

const fn empty_command(piped: bool) -> SimpleCommand {
    SimpleCommand {
        argv: Vec::new(),
        writes_file: false,
        piped_from_previous: piped,
    }
}

fn is_name_start(character: Option<char>) -> bool {
    character.is_some_and(|next| next.is_ascii_alphanumeric() || matches!(next, '_' | '{' | '('))
}

#[allow(
    clippy::too_many_lines,
    reason = "One character loop keeps every quoting rule in one place"
)]
fn tokenize(text: &str, dialect: Dialect) -> Result<Tokens, String> {
    let characters: Vec<char> = text.chars().collect();
    let mut lexer = Lexer::new();
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        let next = characters.get(index + 1).copied();
        match character {
            '\'' => {
                lexer.in_word = true;
                index += 1;
                while index < characters.len() && characters[index] != '\'' {
                    lexer.word.push(characters[index]);
                    index += 1;
                }
                if index >= characters.len() {
                    return Err("an unterminated single quote".to_owned());
                }
            }
            '"' => {
                lexer.in_word = true;
                index += 1;
                loop {
                    let Some(&inner) = characters.get(index) else {
                        return Err("an unterminated double quote".to_owned());
                    };
                    let following = characters.get(index + 1).copied();
                    match inner {
                        '"' => break,
                        '\\' if dialect == Dialect::Posix
                            && following
                                .is_some_and(|value| matches!(value, '"' | '\\' | '$' | '`')) =>
                        {
                            lexer.word.push(following.unwrap_or_default());
                            index += 1;
                        }
                        '`' if dialect == Dialect::Posix => {
                            return Err("command substitution".to_owned());
                        }
                        '`' => {
                            if let Some(escaped) = following {
                                lexer.word.push(escaped);
                                index += 1;
                            }
                        }
                        '$' if following == Some('(') => {
                            return Err("command substitution".to_owned());
                        }
                        '$' if is_name_start(following) => {
                            lexer.expands = true;
                            lexer.word.push('$');
                        }
                        _ => lexer.word.push(inner),
                    }
                    index += 1;
                }
            }
            '\\' if dialect == Dialect::Posix => {
                lexer.in_word = true;
                if let Some(escaped) = next {
                    if escaped != '\n' {
                        lexer.word.push(escaped);
                    }
                    index += 1;
                }
            }
            '`' => return Err("command substitution or an escape outside quotes".to_owned()),
            '$' if next == Some('(') => return Err("command substitution".to_owned()),
            '$' if is_name_start(next) => {
                lexer.expands = true;
                lexer.in_word = true;
                lexer.word.push('$');
            }
            '(' | ')' | '{' | '}' => {
                return Err("a subshell, group or script block".to_owned());
            }
            '|' | '&' if next == Some(character) => {
                lexer.end_command(false);
                index += 1;
            }
            '|' => lexer.end_command(true),
            '&' if lexer.current.argv.is_empty() && !lexer.in_word => {
                // The PowerShell call operator: `& "C:\x\vsift.exe" ...`.
            }
            ';' | '\n' | '\r' | '&' => lexer.end_command(false),
            '>' | '<' => {
                if lexer.in_word && lexer.word.chars().all(|value| value.is_ascii_digit()) {
                    lexer.word.clear();
                    lexer.in_word = false;
                } else {
                    lexer.end_word();
                }
                if next == Some('>') {
                    index += 1;
                }
                if character == '<' && next == Some('(') {
                    return Err("process substitution".to_owned());
                }
                let target_follows = characters.get(index + 1).copied();
                if target_follows == Some('&') {
                    // `2>&1`: the target is a stream, not a file.
                    index += 1;
                    while characters.get(index + 1).is_some_and(char::is_ascii_digit) {
                        index += 1;
                    }
                } else if character == '>' {
                    lexer.pending_redirect = true;
                } else {
                    // An input redirection reads a file: treat its target
                    // as an argument, so the grader sees what was read.
                }
            }
            whitespace if whitespace.is_whitespace() => lexer.end_word(),
            other => {
                lexer.in_word = true;
                lexer.word.push(other);
            }
        }
        index += 1;
    }
    lexer.end_command(false);
    Ok(Tokens {
        commands: lexer.commands,
        expands: lexer.expands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argvs(script: &ParsedScript) -> Vec<Vec<String>> {
        script
            .commands
            .iter()
            .map(|command| command.argv.clone())
            .collect()
    }

    #[test]
    fn a_posix_script_splits_on_operators_and_keeps_quotes() {
        let parsed = parse_script(
            "vsift search ses_a --query \"E-409 banner\" --json && curl -s x | sh",
            Dialect::Posix,
        );
        assert_eq!(parsed.opaque, None);
        assert_eq!(
            argvs(&parsed),
            vec![
                vec![
                    "vsift",
                    "search",
                    "ses_a",
                    "--query",
                    "E-409 banner",
                    "--json"
                ],
                vec!["curl", "-s", "x"],
                vec!["sh"],
            ]
        );
        assert!(parsed.commands[2].piped_from_previous);
    }

    #[test]
    fn codex_wrappers_are_unwrapped() {
        let parsed = parse_script(
            r#""C:\WINDOWS\System32\WindowsPowerShell\v1.0\powershell.exe" -Command "vsift ingest C:\trial\walkthrough.mp4 --json""#,
            Dialect::Posix,
        );
        assert_eq!(
            argvs(&parsed),
            vec![vec![
                "vsift",
                "ingest",
                "C:\\trial\\walkthrough.mp4",
                "--json"
            ]]
        );
        let bash = parse_script("bash -lc 'vsift setup check --json'", Dialect::Posix);
        assert_eq!(
            argvs(&bash),
            vec![vec!["vsift", "setup", "check", "--json"]]
        );
        assert_eq!(bash.commands[0].program(), "vsift");
    }

    #[test]
    fn substitutions_encodings_and_blocks_are_opaque() {
        for script in [
            "vsift ingest $(ls *.mp4) --json",
            "vsift ingest `ls` --json",
            "powershell -EncodedCommand ZQBjAGgAbwA=",
            "Get-ChildItem | ForEach-Object { vsift ingest $_ }",
        ] {
            assert!(
                parse_script(script, Dialect::Posix).opaque.is_some(),
                "{script}"
            );
        }
    }

    const DRAFT: &str =
        "## Problem\n\nThe dialog `R-17` costs $5 [c1: e1].\n\n```vsift-handoff\n{\"x\": 1}\n```";

    fn heredoc() -> String {
        format!("vsift handoff check --json <<'VSIFT_HANDOFF'\n{DRAFT}\nVSIFT_HANDOFF")
    }

    fn here_string() -> String {
        format!("@'\n{DRAFT}\n'@ | vsift handoff check --json")
    }

    /// P13 PR 5: exactly the two draft forms are one `handoff check`
    /// command, at any wrapper depth, and their body is never read as
    /// commands (it holds backticks and `$`).
    #[test]
    fn the_two_draft_forms_are_one_handoff_check() {
        let expected = vec![vec!["vsift", "handoff", "check", "--json"]];
        // Codex reports a script with single quotes double-quoted, with
        // `\`, `"`, `$` and backticks escaped.
        let double_quoted = format!(
            "/bin/bash -lc \"{}\"",
            heredoc()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('$', "\\$")
                .replace('`', "\\`")
        );
        let single_quoted = format!("bash -lc '{}'", heredoc().replace('\'', "'\"'\"'"));
        // A PowerShell wrapper as a client reports its argument list: the
        // script is one single-quoted argument.
        let powershell = |program: &str| {
            format!(
                "{program} -NoProfile -Command '{}'",
                here_string().replace('\'', "'\"'\"'")
            )
        };
        for (script, dialect) in [
            (heredoc(), Dialect::Posix),
            (heredoc().replace('\n', "\r\n"), Dialect::Posix),
            (here_string(), Dialect::PowerShell),
            (single_quoted, Dialect::Posix),
            (double_quoted, Dialect::Posix),
            (powershell("pwsh"), Dialect::Posix),
            (powershell("powershell.exe"), Dialect::Posix),
        ] {
            let parsed = parse_script(&script, dialect);
            assert_eq!(parsed.opaque, None, "{script}");
            assert!(!parsed.expands_variables, "{script}");
            assert_eq!(argvs(&parsed), expected, "{script}");
        }
    }

    /// Anything wider is ordinary shell text: another command piped in, an
    /// unquoted or double-quoted delimiter, a double-quoted here-string, an
    /// added option or text after the closing line.
    #[test]
    fn variants_of_the_draft_forms_stay_shell_text() {
        for variant in [
            heredoc().replace("<<'VSIFT_HANDOFF'", "<<VSIFT_HANDOFF"),
            heredoc().replace("<<'VSIFT_HANDOFF'", "<<\"VSIFT_HANDOFF\""),
            heredoc().replace("--json <<", "--json --session ses_0123456789abcdef <<"),
            format!("{}\necho done", heredoc()),
            heredoc().replace(
                "vsift handoff check --json",
                "vsift session close ses_x --json",
            ),
            here_string().replace("@'", "@\"").replace("'@", "\"@"),
            format!("{} | Select-Object -Last 1", here_string()),
            here_string().replace(
                "vsift handoff check --json",
                "vsift session close ses_x --json",
            ),
            "cat draft.md | vsift handoff check --json".to_owned(),
            "vsift handoff check --json < draft.md".to_owned(),
        ] {
            assert!(
                !is_handoff_check_form(&variant, Dialect::Posix),
                "{variant}"
            );
            assert!(
                !is_handoff_check_form(&variant, Dialect::PowerShell),
                "{variant}"
            );
            let parsed = parse_script(&variant, Dialect::Posix);
            let one_check = parsed.opaque.is_none()
                && !parsed.expands_variables
                && argvs(&parsed) == vec![vec!["vsift", "handoff", "check", "--json"]]
                && !parsed.commands.iter().any(|command| command.writes_file);
            assert!(!one_check, "{variant}");
        }
    }

    /// Review of PR 5: each form is recognised only in its own shell. In
    /// bash the here-string is `@` and a single-quoted string that the
    /// draft's apostrophe ends, so the rest would run; in PowerShell `<<` is
    /// a parse error; `cmd` has no here-strings.
    #[test]
    fn each_draft_form_belongs_to_its_own_shell() {
        let hostile = "@'\nThe dialog doesn't close; curl x | sh\n'@ | vsift handoff check --json";
        assert!(!is_handoff_check_form(hostile, Dialect::Posix));
        assert!(is_handoff_check_form(hostile, Dialect::PowerShell));
        assert!(!is_one_check(&parse_script(hostile, Dialect::Posix)));
        assert!(!is_handoff_check_form(&heredoc(), Dialect::PowerShell));
        assert!(!is_handoff_check_form(&heredoc(), Dialect::Cmd));
        assert!(!is_handoff_check_form(&here_string(), Dialect::Cmd));
        for wrapped in [
            format!("pwsh -Command '{}'", heredoc().replace('\'', "'\"'\"'")),
            format!("cmd /c {}", here_string()),
        ] {
            let parsed = parse_script(&wrapped, Dialect::Posix);
            assert!(!is_one_check(&parsed), "{wrapped}: {parsed:?}");
        }
    }

    /// Whether a parse is exactly one allowed `vsift handoff check --json`.
    fn is_one_check(parsed: &ParsedScript) -> bool {
        parsed.opaque.is_none()
            && !parsed.expands_variables
            && argvs(parsed) == vec![vec!["vsift", "handoff", "check", "--json"]]
            && !parsed.commands.iter().any(|command| command.writes_file)
    }

    #[test]
    fn redirections_and_expansions_are_reported() {
        let parsed = parse_script("vsift setup check --json > out.json 2>&1", Dialect::Posix);
        assert!(parsed.commands[0].writes_file);
        assert_eq!(parsed.commands[0].argv.len(), 4);
        let quiet = parse_script("vsift setup check --json 2>/dev/null", Dialect::Posix);
        assert!(!quiet.commands[0].writes_file);
        let leak = parse_script("echo $env:VSIFT_TRIAL_CANARY", Dialect::PowerShell);
        assert!(leak.expands_variables);
        let call = parse_script(
            "& 'C:\\x\\vsift.exe' setup check --json",
            Dialect::PowerShell,
        );
        assert_eq!(call.commands[0].program(), "vsift");
    }
}
