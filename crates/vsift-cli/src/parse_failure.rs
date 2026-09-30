//! Typed remediation for a command line the parser rejects (L-071, P13 PR 1).
//!
//! In `--json` and `--events jsonl` modes a rejected command line answers
//! `INVALID_ARGUMENT` with `command` `parse`. Its remediation says what kind
//! of mistake it is ([`ParseRejection`]), names the defined argument when
//! the parser identifies one, and suggests the help of the deepest command
//! the line reached, such as `vsift crop --help`.
//!
//! **Nothing the user typed is repeated.** Argument text can come from
//! evidence (a transcript line an agent copied), so it may carry hidden or
//! control characters, or instructions. The remediation is fixed prose; the
//! only names in it are the grammar's own: a subcommand of [`Cli`] or a
//! defined argument (`--rect`, `<SESSION>`), taken from the grammar after the
//! parser's report is resolved against it. A name that does not resolve is
//! left out, so an unknown flag or an invalid value is never echoed. The
//! parser's own explanation, which does quote the text, is shown only in
//! human mode, on stderr, through [`crate::output::OutputWriter::write_safe_diagnostic`].

use std::ffi::OsString;

use clap::{
    Arg, CommandFactory,
    error::{ContextKind, ContextValue, ErrorKind},
};
use vsift::FailureCode;

use crate::{CommandFailure, command::Cli};

/// Why the parser rejected a command line: a closed set, carried in the
/// remediation summary as `(<identifier>)`, the form of the search query's
/// rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParseRejection {
    /// An argument the command does not define, or one argument too many.
    UnknownArgument,
    /// A required argument is missing.
    MissingRequired,
    /// An argument's value is missing or not in the form it takes.
    InvalidValue,
    /// A value was given to an argument that takes none (or fewer).
    UnexpectedValue,
    /// Two arguments that exclude each other, or one argument given twice.
    ArgumentConflict,
    /// A namespace such as `session` was given without its operation.
    MissingSubcommand,
    /// A word where a command or operation was expected is not one.
    UnknownSubcommand,
    /// An argument is not valid Unicode text.
    InvalidUtf8,
    /// A rejection the parser reports in a way `VSift` does not classify;
    /// kept so a parser upgrade cannot produce an unlisted value.
    Unclassified,
}

impl ParseRejection {
    /// Every rejection, in documentation order (tests check the contract
    /// document against it).
    #[cfg(test)]
    pub(crate) const ALL: [Self; 9] = [
        Self::UnknownArgument,
        Self::MissingRequired,
        Self::InvalidValue,
        Self::UnexpectedValue,
        Self::ArgumentConflict,
        Self::MissingSubcommand,
        Self::UnknownSubcommand,
        Self::InvalidUtf8,
        Self::Unclassified,
    ];

    /// The stable identifier the remediation carries.
    pub(crate) const fn identifier(self) -> &'static str {
        match self {
            Self::UnknownArgument => "unknown_argument",
            Self::MissingRequired => "missing_required",
            Self::InvalidValue => "invalid_value",
            Self::UnexpectedValue => "unexpected_value",
            Self::ArgumentConflict => "argument_conflict",
            Self::MissingSubcommand => "missing_subcommand",
            Self::UnknownSubcommand => "unknown_subcommand",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::Unclassified => "unclassified",
        }
    }

    /// The rejection for a parser error kind. `at_namespace` says whether
    /// the deepest command reached has operations of its own, which decides
    /// what a bare namespace (`vsift session`) is missing.
    const fn from_kind(kind: ErrorKind, at_namespace: bool) -> Self {
        match kind {
            ErrorKind::UnknownArgument => Self::UnknownArgument,
            ErrorKind::InvalidValue
            | ErrorKind::ValueValidation
            | ErrorKind::NoEquals
            | ErrorKind::TooFewValues
            | ErrorKind::WrongNumberOfValues => Self::InvalidValue,
            ErrorKind::TooManyValues => Self::UnexpectedValue,
            ErrorKind::ArgumentConflict => Self::ArgumentConflict,
            ErrorKind::MissingSubcommand => Self::MissingSubcommand,
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand if at_namespace => {
                Self::MissingSubcommand
            }
            ErrorKind::MissingRequiredArgument
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => Self::MissingRequired,
            ErrorKind::InvalidSubcommand => Self::UnknownSubcommand,
            ErrorKind::InvalidUtf8 => Self::InvalidUtf8,
            _ => Self::Unclassified,
        }
    }
}

/// Where in the grammar a rejection happened, in the grammar's own names.
#[derive(Debug, Eq, PartialEq)]
struct GrammarLocation {
    /// Subcommand names from the root to the deepest command reached.
    command_path: Vec<String>,
    /// The defined argument the parser blamed, when it resolves.
    argument: Option<String>,
    /// The other argument of a conflict, when it resolves.
    other_argument: Option<String>,
    /// An argument of the deepest command that takes comma-separated
    /// values (`--rect`), which PowerShell splits unless it is quoted.
    comma_separated: Option<String>,
}

impl GrammarLocation {
    /// The command as a reader writes it: `crop`, `transcript get`, or
    /// `vsift` for the root.
    fn command_label(&self) -> String {
        if self.command_path.is_empty() {
            String::from("vsift")
        } else {
            self.command_path.join(" ")
        }
    }

    /// The help form of the deepest command: its path, then `--help`.
    fn help_arguments(&self) -> Vec<String> {
        let mut arguments = self.command_path.clone();
        arguments.push(String::from("--help"));
        arguments
    }
}

/// The typed failure for a command line the parser rejected with `error`.
///
/// `arguments` is the full command line, program name first; it is read only
/// to find the deepest subcommand, by exact comparison with the grammar's
/// names.
pub(crate) fn parse_failure(error: &clap::Error, arguments: &[OsString]) -> CommandFailure {
    let mut grammar = Cli::command();
    grammar.build();
    let command = deepest_command(&grammar, arguments);
    let location = GrammarLocation {
        command_path: command.path,
        argument: blamed_argument(error, ContextKind::InvalidArg, command.definition),
        other_argument: blamed_argument(error, ContextKind::PriorArg, command.definition),
        comma_separated: comma_separated_argument(command.definition),
    };
    let reason = ParseRejection::from_kind(error.kind(), command.definition.has_subcommands());
    remediation(reason, &location)
}

/// The typed failure for `--json` given together with `--events`: the two
/// output modes exclude each other.
pub(crate) fn output_mode_conflict(arguments: &[OsString]) -> CommandFailure {
    let mut grammar = Cli::command();
    grammar.build();
    let command = deepest_command(&grammar, arguments);
    let location = GrammarLocation {
        command_path: command.path,
        argument: defined_long(command.definition, "json"),
        other_argument: defined_long(command.definition, "events"),
        comma_separated: comma_separated_argument(command.definition),
    };
    remediation(ParseRejection::ArgumentConflict, &location)
}

/// Builds the one remediation: fixed prose around grammar names, and the
/// help of the deepest command as its suggested command.
fn remediation(reason: ParseRejection, location: &GrammarLocation) -> CommandFailure {
    let summary = summary(reason, location);
    CommandFailure::with_suggested_command(
        FailureCode::InvalidArgument,
        summary,
        location.help_arguments(),
    )
}

/// The remediation summary: `The command line was rejected (<reason>).`,
/// what is wrong in grammar names, and a pointer to the help; with a
/// PowerShell quoting note when the command takes comma-separated values.
fn summary(reason: ParseRejection, location: &GrammarLocation) -> String {
    let command = location.command_label();
    let argument = location.argument.as_deref();
    let other = location.other_argument.as_deref();
    let problem = match (reason, argument, other) {
        (ParseRejection::UnknownArgument, _, _) => {
            format!("`{command}` does not take one of the arguments given")
        }
        (ParseRejection::MissingRequired, Some(argument), _) => {
            format!("`{command}` needs `{argument}`")
        }
        (ParseRejection::MissingRequired, None, _) => {
            format!("`{command}` is missing a required argument")
        }
        (ParseRejection::InvalidValue, Some(argument), _) => {
            format!("The value of `{argument}` is missing or not valid for `{command}`")
        }
        (ParseRejection::InvalidValue, None, _) => {
            format!("A value given to `{command}` is missing or not valid")
        }
        (ParseRejection::UnexpectedValue, Some(argument), _) => {
            format!("`{argument}` was given a value it does not take in `{command}`")
        }
        (ParseRejection::UnexpectedValue, None, _) => {
            format!("An argument of `{command}` was given a value it does not take")
        }
        (ParseRejection::ArgumentConflict, Some(argument), Some(other)) if argument != other => {
            format!("`{argument}` cannot be used with `{other}` in `{command}`")
        }
        (ParseRejection::ArgumentConflict, Some(argument), _) => format!(
            "`{argument}` was given more than once, or with an argument it excludes, in `{command}`"
        ),
        (ParseRejection::ArgumentConflict, None, _) => format!(
            "An argument of `{command}` was given more than once, or with an argument it excludes"
        ),
        (ParseRejection::MissingSubcommand, _, _) => {
            format!("`{command}` needs one of its commands")
        }
        (ParseRejection::UnknownSubcommand, _, _) => {
            format!("`{command}` has no command by the name given")
        }
        (ParseRejection::InvalidUtf8, _, _) => {
            format!("An argument given to `{command}` is not valid Unicode text")
        }
        (ParseRejection::Unclassified, _, _) => {
            format!("`{command}` did not accept the command line")
        }
    };
    let note = location
        .comma_separated
        .as_deref()
        .map(|comma_separated| {
            format!(
                " On PowerShell, quote the value of `{comma_separated}` (for example \
                 '10,20,300,80'): unquoted, PowerShell splits it at the commas into several \
                 arguments."
            )
        })
        .unwrap_or_default();
    format!(
        "The command line was rejected ({}). {problem}; read its help.{note}",
        reason.identifier()
    )
}

/// A command of the grammar and the subcommand names that lead to it.
struct ReachedCommand<'grammar> {
    path: Vec<String>,
    definition: &'grammar clap::Command,
}

/// The deepest subcommand the command line names, found by comparing each
/// argument exactly with the grammar's subcommand names.
///
/// Values of options that take one (`--session-root <path>`) are skipped, so
/// a path that happens to read `crop` is not taken for the command. The
/// walk stops at `--`, at the first word that is neither an option nor a
/// subcommand of a namespace, and never looks at a leaf's own positionals.
fn deepest_command<'grammar>(
    root: &'grammar clap::Command,
    arguments: &[OsString],
) -> ReachedCommand<'grammar> {
    let mut path = Vec::new();
    let mut current = root;
    let mut remaining = arguments.iter().skip(1);
    while let Some(argument) = remaining.next() {
        let Some(text) = argument.to_str() else {
            continue;
        };
        if text == "--" {
            break;
        }
        if let Some(long) = text.strip_prefix("--") {
            let takes_value = !long.contains('=')
                && current
                    .get_arguments()
                    .find(|defined| defined.get_long() == Some(long))
                    .is_some_and(|defined| defined.get_action().takes_values());
            if takes_value {
                remaining.next();
            }
            continue;
        }
        if text.starts_with('-') {
            continue;
        }
        if !current.has_subcommands() {
            continue;
        }
        match current.find_subcommand(text) {
            Some(subcommand) => {
                path.push(subcommand.get_name().to_owned());
                current = subcommand;
            }
            None => break,
        }
    }
    ReachedCommand {
        path,
        definition: current,
    }
}

/// The grammar name of the argument the parser's report blames under
/// `kind`, when the report resolves to an argument `command` defines.
///
/// The parser renders a defined argument as `--rect <X,Y,WIDTH,HEIGHT>` or
/// `<SESSION>`; only the leading name is compared, and the name returned is
/// rebuilt from the grammar, never copied from the report.
fn blamed_argument(
    error: &clap::Error,
    kind: ContextKind,
    command: &clap::Command,
) -> Option<String> {
    let rendered = match error.get(kind)? {
        ContextValue::String(value) => value.as_str(),
        ContextValue::Strings(values) => values.first()?.as_str(),
        _ => return None,
    };
    let name = rendered.split([' ', '=']).next()?;
    if let Some(long) = name.strip_prefix("--") {
        return defined_long(command, long);
    }
    let value_name = name
        .strip_prefix('<')
        .and_then(|inner| inner.strip_suffix('>'))
        .or_else(|| {
            name.strip_prefix('[')
                .and_then(|inner| inner.strip_suffix(']'))
        })?;
    command
        .get_positionals()
        .find(|defined| positional_name(defined) == value_name)
        .map(|defined| format!("<{}>", positional_name(defined)))
}

/// `--<long>` when `command` defines that long option.
fn defined_long(command: &clap::Command, long: &str) -> Option<String> {
    command
        .get_arguments()
        .find_map(|defined| defined.get_long().filter(|defined| *defined == long))
        .map(|defined| format!("--{defined}"))
}

/// A positional's name as help shows it: its value name, or its id in
/// upper case.
fn positional_name(argument: &Arg) -> String {
    argument
        .get_value_names()
        .and_then(<[clap::builder::Str]>::first)
        .map_or_else(
            || argument.get_id().as_str().to_uppercase(),
            ToString::to_string,
        )
}

/// The first option of `command` whose value is written as a
/// comma-separated list (its value name has a comma, like
/// `X,Y,WIDTH,HEIGHT`).
fn comma_separated_argument(command: &clap::Command) -> Option<String> {
    command.get_arguments().find_map(|defined| {
        let long = defined.get_long()?;
        defined
            .get_value_names()?
            .iter()
            .any(|name| name.as_str().contains(','))
            .then(|| format!("--{long}"))
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use clap::{CommandFactory, Parser, error::ErrorKind};

    use super::{ParseRejection, deepest_command, parse_failure};
    use crate::command::Cli;

    fn arguments(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    fn rejection(words: &[&str]) -> Result<(clap::Error, Vec<OsString>), String> {
        let line = arguments(words);
        match Cli::try_parse_from(&line) {
            Ok(_) => Err(format!("{words:?} parsed")),
            Err(error) => Ok((error, line)),
        }
    }

    #[test]
    fn identifiers_are_distinct_snake_case_words() {
        let mut seen = std::collections::BTreeSet::new();
        for reason in ParseRejection::ALL {
            let identifier = reason.identifier();
            assert!(seen.insert(identifier), "{identifier} repeats");
            assert!(
                identifier
                    .chars()
                    .all(|character| character.is_ascii_lowercase()
                        || character.is_ascii_digit()
                        || character == '_'),
                "{identifier}"
            );
        }
    }

    /// Kinds no command line of this grammar produces still map to a listed
    /// rejection; an unknown one is `unclassified`, never a new value.
    #[test]
    fn every_parser_kind_maps_to_a_listed_rejection() {
        for (kind, at_namespace, expected) in [
            (ErrorKind::MissingSubcommand, false, "missing_subcommand"),
            (
                ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand,
                true,
                "missing_subcommand",
            ),
            (
                ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand,
                false,
                "missing_required",
            ),
            (ErrorKind::NoEquals, false, "invalid_value"),
            (ErrorKind::TooFewValues, false, "invalid_value"),
            (ErrorKind::WrongNumberOfValues, false, "invalid_value"),
            (ErrorKind::Io, false, "unclassified"),
            (ErrorKind::Format, false, "unclassified"),
        ] {
            assert_eq!(
                ParseRejection::from_kind(kind, at_namespace).identifier(),
                expected,
                "{kind:?}"
            );
        }
    }

    /// The contract document lists every rejection the CLI can report.
    #[test]
    fn the_contract_document_lists_every_rejection() -> Result<(), Box<dyn std::error::Error>> {
        let contract = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/contracts/cli-v1.md"),
        )?;
        for reason in ParseRejection::ALL {
            let listed = format!("`{}`", reason.identifier());
            assert!(
                contract.contains(&listed),
                "cli-v1.md does not list {listed}"
            );
        }
        Ok(())
    }

    #[test]
    fn the_walk_follows_subcommands_and_skips_option_values() {
        let mut grammar = Cli::command();
        grammar.build();
        for (words, expected) in [
            (vec!["vsift"], vec![]),
            (vec!["vsift", "crop", "session"], vec!["crop"]),
            (
                vec!["vsift", "--session-root", "crop", "session", "status"],
                vec!["session", "status"],
            ),
            (
                vec!["vsift", "--json", "transcript", "get", "get"],
                vec!["transcript", "get"],
            ),
            (vec!["vsift", "session", "frob", "status"], vec!["session"]),
            (vec!["vsift", "--", "crop"], vec![]),
        ] {
            let reached = deepest_command(&grammar, &arguments(&words));
            assert_eq!(reached.path, expected, "{words:?}");
        }
    }

    #[test]
    fn a_blamed_name_that_is_not_in_the_grammar_is_left_out()
    -> Result<(), Box<dyn std::error::Error>> {
        let (error, line) = rejection(&["vsift", "crop", "--rectangle", "1,2,3,4"])?;
        let failure = parse_failure(&error, &line);
        let summary = failure.summary().ok_or("summary")?;
        assert!(!summary.contains("--rectangle"), "{summary}");
        assert!(summary.contains("(unknown_argument)"), "{summary}");
        Ok(())
    }
}
