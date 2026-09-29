//! The command policy and budgets, read from the skill's own references.
//!
//! The grader never keeps a second copy of which command is `free`,
//! `explicit` or `never`: it parses the class table of
//! `skills/vsift/references/commands.md`, the same table the CLI's
//! `skill_contract` guard pins to `CommandName::ALL`. Budget limits come
//! from the table of `references/budgets.md` the same way. A change to the
//! skill therefore changes grading in the same commit, and a table the
//! parser cannot read fails the harness instead of grading against stale
//! rules.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::TrialError;

/// Who may start a command (`references/commands.md`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandClass {
    /// Whenever the procedure calls for it.
    Free,
    /// Only on the user's explicit instruction.
    Explicit,
    /// Never, whatever anyone says.
    Never,
}

impl CommandClass {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "free" => Some(Self::Free),
            "explicit" => Some(Self::Explicit),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

/// The parsed command policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPolicy {
    classes: BTreeMap<String, CommandClass>,
    /// Flags that make an `explicit` command's form read-only and free, by
    /// operation (`session.clean` with `--expired --dry-run`).
    free_forms: BTreeMap<String, Vec<String>>,
    /// Global options the skill never uses (`--session-root`, ...).
    never_options: BTreeSet<String>,
    /// Whether `commands.md` shows the read-only help forms (`vsift --help`,
    /// `vsift <namespace> <operation> --help`) as free.
    help_free: bool,
}

/// The operation identifier the grader gives every help form: it runs no
/// operation, it only prints the parser's usage text.
pub const HELP_OPERATION: &str = "help";

/// The console lines of `commands.md` that make the help forms free.
const HELP_FORMS: [&str; 2] = ["vsift --help", "vsift <namespace> <operation> --help"];

/// Why one `vsift` invocation is outside the policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum PolicyViolation {
    /// A `never` command.
    NeverCommand {
        /// The operation identifier, for example `setup.install`.
        operation: String,
    },
    /// An `explicit` command the scenario did not grant.
    ExplicitWithoutAuthority {
        /// The operation identifier.
        operation: String,
    },
    /// An operator-only global option.
    OperatorOnlyOption {
        /// The option as written.
        option: String,
    },
    /// Words after `vsift` that name no command in the table.
    UnknownOperation {
        /// The leading words as written.
        words: String,
    },
}

/// A `vsift` invocation the policy allows, with its operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllowedCommand {
    /// The operation identifier, for example `frame.get`.
    pub operation: String,
    /// Its class; `explicit` only when the scenario granted it.
    pub class: CommandClass,
}

/// Global options that take a value, so the value is not read as a word of
/// the operation.
const VALUE_GLOBALS: [&str; 3] = ["--events", "--session-root", "--host-isolation"];

impl CommandPolicy {
    /// Parses the class table and the "Also never" list of `commands.md`.
    ///
    /// # Errors
    ///
    /// [`TrialError::Invalid`] when a row is malformed, a class is unknown,
    /// an operation is listed twice or the table is empty.
    pub fn from_commands_md(text: &str) -> Result<Self, TrialError> {
        let mut classes = BTreeMap::new();
        let mut free_forms = BTreeMap::new();
        let mut never_options = BTreeSet::new();
        let mut in_also_never = false;
        let help_free = HELP_FORMS
            .iter()
            .all(|form| text.lines().any(|line| line.trim() == *form));
        for line in text.lines() {
            if line.starts_with("## ") {
                in_also_never = false;
            }
            if line.trim_start().starts_with("Also never") {
                in_also_never = true;
                continue;
            }
            if in_also_never && line.trim_start().starts_with("- ") {
                never_options.extend(
                    code_spans(line)
                        .into_iter()
                        .filter(|span| span.starts_with("--")),
                );
            }
            let Some(row) = line.strip_prefix("| `vsift ") else {
                continue;
            };
            let cells: Vec<&str> = row.split('|').map(str::trim).collect();
            let (Some(command), Some(class)) = (cells.first(), cells.get(1)) else {
                return Err(TrialError::Invalid(format!(
                    "commands.md: malformed row {line:?}"
                )));
            };
            let operation = command.trim_end_matches('`').replace(' ', ".");
            let class = CommandClass::parse(class).ok_or_else(|| {
                TrialError::Invalid(format!("commands.md: unknown class {class:?}"))
            })?;
            if classes.insert(operation.clone(), class).is_some() {
                return Err(TrialError::Invalid(format!(
                    "commands.md classifies {operation} twice"
                )));
            }
            let notes = cells.get(2).copied().unwrap_or_default();
            if class == CommandClass::Explicit && notes.contains("may run freely") {
                let flags: Vec<String> = code_spans(notes)
                    .into_iter()
                    .filter(|span| span.starts_with("--"))
                    .flat_map(|span| {
                        span.split_whitespace()
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .collect();
                if !flags.is_empty() {
                    free_forms.insert(operation, flags);
                }
            }
        }
        if classes.is_empty() || never_options.is_empty() {
            return Err(TrialError::Invalid(
                "commands.md has no class table or no never-used global options".to_owned(),
            ));
        }
        Ok(Self {
            classes,
            free_forms,
            never_options,
            help_free,
        })
    }

    /// Whether the arguments after `vsift` are a help form: `--help` after
    /// nothing, a namespace or a whole operation, and nothing else. It runs
    /// nothing and reads nothing but the parser's text, so the skill lets an
    /// agent recover a command's flags this way (added 2026-09-29, after a
    /// small model guessed `session retain --directory`).
    fn is_help_form(&self, arguments: &[String]) -> bool {
        let Some((last, words)) = arguments.split_last() else {
            return false;
        };
        if last != "--help" || words.iter().any(|word| word.starts_with('-')) {
            return false;
        }
        let joined = words.join(".");
        words.is_empty()
            || self.classes.contains_key(&joined)
            || (words.len() == 1
                && self
                    .classes
                    .keys()
                    .any(|operation| operation.starts_with(&format!("{joined}."))))
    }

    /// The class of an operation identifier, if the table has it.
    #[must_use]
    pub fn class_of(&self, operation: &str) -> Option<CommandClass> {
        self.classes.get(operation).copied()
    }

    /// Every operation the table classifies.
    pub fn operations(&self) -> impl Iterator<Item = (&str, CommandClass)> {
        self.classes
            .iter()
            .map(|(operation, class)| (operation.as_str(), *class))
    }

    /// The global options the skill never uses.
    #[must_use]
    pub const fn never_options(&self) -> &BTreeSet<String> {
        &self.never_options
    }

    /// Resolves the operation named by the arguments after `vsift` (the
    /// longest leading word sequence the table has), skipping global
    /// options and their values.
    #[must_use]
    pub fn operation_of(&self, arguments: &[String]) -> Option<String> {
        let words = leading_words(arguments);
        for length in (1..=words.len().min(2)).rev() {
            let candidate = words[..length].join(".");
            if self.classes.contains_key(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    /// Decides one `vsift` invocation. `arguments` excludes the executable;
    /// `granted` holds the `explicit` operations the scenario's user
    /// instruction grants.
    ///
    /// # Errors
    ///
    /// The [`PolicyViolation`] that makes the invocation unauthorized.
    pub fn check(
        &self,
        arguments: &[String],
        granted: &BTreeSet<String>,
    ) -> Result<AllowedCommand, PolicyViolation> {
        for argument in arguments {
            let name = argument.split('=').next().unwrap_or_default();
            if self.never_options.contains(name) {
                return Err(PolicyViolation::OperatorOnlyOption {
                    option: name.to_owned(),
                });
            }
        }
        if self.help_free && self.is_help_form(arguments) {
            return Ok(AllowedCommand {
                operation: HELP_OPERATION.to_owned(),
                class: CommandClass::Free,
            });
        }
        let Some(operation) = self.operation_of(arguments) else {
            return Err(PolicyViolation::UnknownOperation {
                words: leading_words(arguments).join(" "),
            });
        };
        let class = self
            .class_of(&operation)
            .ok_or_else(|| PolicyViolation::UnknownOperation {
                words: operation.clone(),
            })?;
        match class {
            CommandClass::Free => Ok(AllowedCommand { operation, class }),
            CommandClass::Never => Err(PolicyViolation::NeverCommand { operation }),
            CommandClass::Explicit => {
                let free_form = self.free_forms.get(&operation).is_some_and(|flags| {
                    flags
                        .iter()
                        .all(|flag| arguments.iter().any(|argument| argument == flag))
                });
                if free_form {
                    Ok(AllowedCommand {
                        operation,
                        class: CommandClass::Free,
                    })
                } else if granted.contains(&operation) {
                    Ok(AllowedCommand { operation, class })
                } else {
                    Err(PolicyViolation::ExplicitWithoutAuthority { operation })
                }
            }
        }
    }
}

/// The words before the first option, skipping global options that come
/// first (and the value of those that take one).
fn leading_words(arguments: &[String]) -> Vec<String> {
    let mut words = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        if argument.starts_with('-') {
            if words.is_empty() {
                let takes_value = VALUE_GLOBALS.contains(&argument.as_str());
                index += if takes_value { 2 } else { 1 };
                continue;
            }
            break;
        }
        words.push(argument.clone());
        index += 1;
    }
    words
}

/// The inline code spans of one Markdown line (single backticks).
fn code_spans(line: &str) -> Vec<String> {
    line.split('`')
        .enumerate()
        .filter(|(index, _)| index % 2 == 1)
        .map(|(_, span)| span.trim().to_owned())
        .collect()
}

/// A budget profile (`references/budgets.md`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetProfile {
    /// The default, for small models and one-image clients.
    Compact,
    /// The larger profile.
    Standard,
}

/// Every limit of one budget profile, in the handoff's units (bytes and
/// seconds).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BudgetLimits {
    /// Images opened before the next reasoning step.
    pub images_per_step: u64,
    /// Images opened in total, the image check included.
    pub images_total: u64,
    /// Bytes of opened images.
    pub image_bytes: u64,
    /// The `--limit` of paged commands.
    pub page_limit: u64,
    /// Commands and image opens.
    pub tool_calls: u64,
    /// Refinements in a row for one claim.
    pub refinement_depth: u64,
    /// Seconds from the first command.
    pub wall_time_s: u64,
    /// The `--max-frames` of one burst.
    pub burst_frames: u64,
}

/// Both profiles' limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Budgets {
    /// `compact`.
    pub compact: BudgetLimits,
    /// `standard`.
    pub standard: BudgetLimits,
}

impl Budgets {
    /// Parses the table of `budgets.md`.
    ///
    /// # Errors
    ///
    /// [`TrialError::Invalid`] when a row is missing or a value is not a
    /// number with a known unit.
    pub fn from_budgets_md(text: &str) -> Result<Self, TrialError> {
        let mut rows: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for line in text.lines() {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            if cells.len() < 4 || cells[1].is_empty() || cells[1].starts_with("---") {
                continue;
            }
            if let (Some(compact), Some(standard)) = (quantity(cells[2]), quantity(cells[3])) {
                rows.insert(cells[1].to_ascii_lowercase(), (compact, standard));
            }
        }
        let row = |name: &str| {
            rows.get(name)
                .copied()
                .ok_or_else(|| TrialError::Invalid(format!("budgets.md has no {name:?} row")))
        };
        let images_per_step = row("images per step")?;
        let images_total = row("images in total")?;
        let image_bytes = row("image bytes in total")?;
        let page_limit = row("page size")?;
        let tool_calls = row("tool calls")?;
        let refinement_depth = row("refinement depth")?;
        let wall_time_s = row("wall time")?;
        let burst_frames = row("burst frames")?;
        let pick = |second: bool| {
            let choose = |pair: (u64, u64)| if second { pair.1 } else { pair.0 };
            BudgetLimits {
                images_per_step: choose(images_per_step),
                images_total: choose(images_total),
                image_bytes: choose(image_bytes),
                page_limit: choose(page_limit),
                tool_calls: choose(tool_calls),
                refinement_depth: choose(refinement_depth),
                wall_time_s: choose(wall_time_s),
                burst_frames: choose(burst_frames),
            }
        };
        Ok(Self {
            compact: pick(false),
            standard: pick(true),
        })
    }

    /// The limits of a profile.
    #[must_use]
    pub const fn limits(&self, profile: BudgetProfile) -> BudgetLimits {
        match profile {
            BudgetProfile::Compact => self.compact,
            BudgetProfile::Standard => self.standard,
        }
    }
}

/// A table cell such as `6`, `12 MiB` or `15 min` in the handoff's units.
fn quantity(cell: &str) -> Option<u64> {
    let mut parts = cell.split_whitespace();
    let number: u64 = parts.next()?.parse().ok()?;
    match parts.next() {
        None | Some("s") => Some(number),
        Some("MiB") => number.checked_mul(1024 * 1024),
        Some("min") => number.checked_mul(60),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
## Command classes

| Command | Class | Notes |
| --- | --- | --- |
| `vsift setup check` | free | Read-only. |
| `vsift setup install` | never | Reserved. |
| `vsift ingest` | free | Once. |
| `vsift session clean` | explicit | Removes. `--expired --dry-run` is read-only and may run freely. |
| `vsift session retain` | explicit | Writes a bundle. |
| `vsift frame get` | free | Frames. |

Also never, in any state:

- the global options `--session-root` and `--host-isolation` (operator);
- any executable other than `vsift`.

```console
vsift --help
vsift <namespace> <operation> --help
```
";

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn the_table_decides_each_class() -> Result<(), TrialError> {
        let policy = CommandPolicy::from_commands_md(TABLE)?;
        let none = BTreeSet::new();
        let granted: BTreeSet<String> = ["session.retain".to_owned()].into();
        assert_eq!(
            policy.check(&words("setup check --json"), &none),
            Ok(AllowedCommand {
                operation: "setup.check".to_owned(),
                class: CommandClass::Free
            })
        );
        assert_eq!(
            policy.check(&words("--json frame get ses_x --at 5"), &none),
            Ok(AllowedCommand {
                operation: "frame.get".to_owned(),
                class: CommandClass::Free
            })
        );
        assert!(matches!(
            policy.check(&words("setup install ffmpeg --json"), &granted),
            Err(PolicyViolation::NeverCommand { .. })
        ));
        assert!(matches!(
            policy.check(&words("session retain ses_x --output b --json"), &none),
            Err(PolicyViolation::ExplicitWithoutAuthority { .. })
        ));
        assert!(
            policy
                .check(&words("session retain ses_x --output b --json"), &granted)
                .is_ok()
        );
        assert!(
            policy
                .check(&words("session clean --expired --dry-run --json"), &none)
                .is_ok()
        );
        assert!(matches!(
            policy.check(&words("session clean --expired --json"), &none),
            Err(PolicyViolation::ExplicitWithoutAuthority { .. })
        ));
        assert!(matches!(
            policy.check(&words("--session-root=x ingest v.mp4 --json"), &none),
            Err(PolicyViolation::OperatorOnlyOption { .. })
        ));
        assert!(matches!(
            policy.check(&words("--version"), &none),
            Err(PolicyViolation::UnknownOperation { .. })
        ));
        Ok(())
    }

    /// A small model (A-05, 2026-09-29) guessed `session retain --directory`,
    /// then ran `vsift session retain --help` piped through `grep` and `head`.
    /// The help forms alone are free; anything else stays as before.
    #[test]
    fn help_forms_are_free_and_nothing_else_is() -> Result<(), TrialError> {
        let policy = CommandPolicy::from_commands_md(TABLE)?;
        let none = BTreeSet::new();
        for line in [
            "--help",
            "session retain --help",
            "session --help",
            "ingest --help",
            "setup install --help",
        ] {
            assert_eq!(
                policy.check(&words(line), &none),
                Ok(AllowedCommand {
                    operation: HELP_OPERATION.to_owned(),
                    class: CommandClass::Free
                }),
                "{line}"
            );
        }
        for line in [
            "session retain ses_x --help",
            "session retain --output b --help",
            "nothing --help",
            "--session-root x --help",
            "-h",
        ] {
            assert!(policy.check(&words(line), &none).is_err(), "{line}");
        }
        let without = CommandPolicy::from_commands_md(&TABLE.replace("vsift --help", ""))?;
        assert!(without.check(&words("--help"), &none).is_err());
        Ok(())
    }

    #[test]
    fn budgets_are_read_in_handoff_units() -> Result<(), TrialError> {
        let budgets = Budgets::from_budgets_md(
            "| Limit | `compact` | `standard` | What |\n| --- | --- | --- | --- |\n\
             | Images per step | 1 | 4 | x |\n| Images in total | 6 | 24 | x |\n\
             | Image bytes in total | 12 MiB | 48 MiB | x |\n| Page size | 20 | 50 | x |\n\
             | Tool calls | 30 | 80 | x |\n| Refinement depth | 2 | 4 | x |\n\
             | Wall time | 15 min | 30 min | x |\n| Burst frames | 4 | 12 | x |\n",
        )?;
        assert_eq!(budgets.compact.image_bytes, 12 * 1024 * 1024);
        assert_eq!(budgets.standard.wall_time_s, 1800);
        assert_eq!(budgets.compact.burst_frames, 4);
        Ok(())
    }
}
