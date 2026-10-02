//! The command line of `vsift-governance`.

use std::{error::Error, fmt, path::PathBuf};

use crate::release_evidence::{CommitSha, CompletenessRequest, ReleaseVersion};

/// What a mistaken command line is answered with.
pub(crate) const USAGE: &str = "usage:\n  \
    cargo run -p vsift-governance -- check [ledger-path]\n  \
    cargo run -p vsift-governance -- release-evidence [--complete-for <version> [--commit <sha>]]\n  \
    cargo run -p vsift-governance -- public-claims";

/// A command line that does not parse: shown as the reason and the usage,
/// not as a failed check.
#[derive(Debug)]
pub(crate) struct UsageError(pub(crate) String);

impl fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for UsageError {}

/// The command a command line asks for.
#[derive(Debug)]
pub(crate) enum Command {
    /// Validates the delivery ledger and every control that runs on each pull
    /// request (the release-evidence structure and the public claims too).
    Check {
        /// The delivery ledger file.
        ledger: PathBuf,
    },
    /// Validates the release evidence ledger, and with `--complete-for` its
    /// completeness for one release.
    ReleaseEvidence {
        /// The release to check completeness for, if asked.
        completeness: Option<CompletenessRequest>,
    },
    /// Validates the public-claims registry against the evidence ledger.
    PublicClaims,
}

/// Parses the arguments after the program name.
pub(crate) fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut arguments = arguments.into_iter();
    let command = arguments.next();
    let rest: Vec<String> = arguments.collect();
    match (command.as_deref(), rest.as_slice()) {
        (Some("check"), []) => Ok(Command::Check {
            ledger: PathBuf::from(crate::DEFAULT_LEDGER),
        }),
        (Some("check"), [ledger]) => Ok(Command::Check {
            ledger: PathBuf::from(ledger),
        }),
        (Some("public-claims"), []) => Ok(Command::PublicClaims),
        (Some("release-evidence"), options) => parse_release_evidence(options),
        _ => Err(String::from(USAGE)),
    }
}

/// `release-evidence [--complete-for <version> [--commit <sha>]]`.
fn parse_release_evidence(options: &[String]) -> Result<Command, String> {
    let mut version = None;
    let mut commit = None;
    let mut options = options.iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "--complete-for" => {
                let text = next_value(&mut options, "--complete-for")?;
                version = Some(ReleaseVersion::parse(text).map_err(|error| error.to_string())?);
            }
            "--commit" => {
                let text = next_value(&mut options, "--commit")?;
                commit =
                    Some(CommitSha::try_from(text.clone()).map_err(|error| error.to_string())?);
            }
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    match (version, commit) {
        (Some(version), commit) => Ok(Command::ReleaseEvidence {
            completeness: Some(CompletenessRequest { version, commit }),
        }),
        (None, None) => Ok(Command::ReleaseEvidence { completeness: None }),
        (None, Some(_)) => Err(format!("--commit only goes with --complete-for\n{USAGE}")),
    }
}

/// The value that follows an option.
fn next_value<'a>(
    options: &mut std::slice::Iter<'a, String>,
    name: &str,
) -> Result<&'a String, String> {
    options
        .next()
        .ok_or_else(|| format!("{name} needs a value\n{USAGE}"))
}

#[cfg(test)]
mod tests {
    use super::{Command, parse};

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn check_takes_an_optional_ledger_path() {
        assert!(matches!(
            parse(words("check")),
            Ok(Command::Check { ledger }) if ledger.ends_with("delivery-ledger.json")
        ));
        assert!(matches!(
            parse(words("check other.json")),
            Ok(Command::Check { ledger }) if ledger.ends_with("other.json")
        ));
        assert!(parse(words("check a b")).is_err());
    }

    #[test]
    fn public_claims_takes_no_arguments() {
        assert!(matches!(
            parse(words("public-claims")),
            Ok(Command::PublicClaims)
        ));
        assert!(parse(words("public-claims now")).is_err());
    }

    #[test]
    fn release_evidence_checks_structure_alone_or_completeness() {
        assert!(matches!(
            parse(words("release-evidence")),
            Ok(Command::ReleaseEvidence { completeness: None })
        ));
        assert!(matches!(
            parse(words("release-evidence --complete-for 0.2.0-rc.1")),
            Ok(Command::ReleaseEvidence { completeness: Some(request) })
                if request.commit.is_none() && request.version.is_candidate()
        ));
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert!(matches!(
            parse(words(&format!("release-evidence --complete-for 0.2.0 --commit {sha}"))),
            Ok(Command::ReleaseEvidence { completeness: Some(request) })
                if request.commit.is_some() && !request.version.is_candidate()
        ));
    }

    #[test]
    fn release_evidence_options_are_checked() {
        for line in [
            "release-evidence --complete-for",
            "release-evidence --complete-for v0.2.0",
            "release-evidence --commit 0123456789abcdef0123456789abcdef01234567",
            "release-evidence --complete-for 0.2.0 --commit abc",
            "release-evidence --everything",
            "release-evidence --complete-for 0.2.0 extra",
        ] {
            assert!(parse(words(line)).is_err(), "{line}");
        }
    }

    #[test]
    fn anything_else_is_a_usage_error() {
        for line in ["", "validate", "--help", "CHECK"] {
            assert!(parse(words(line)).is_err(), "{line:?}");
        }
    }
}
