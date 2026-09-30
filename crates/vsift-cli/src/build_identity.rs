//! The text `vsift --version` prints after the executable's name.
//!
//! Shared by the build script, which computes it once at compile time, and by
//! this crate's unit tests. A release build names the source commit it was
//! built from (ADR 0023: "`vsift --version` gains the source commit as a
//! suffix"), so a binary can be matched to its protected commit without its
//! archive. The release workflow sets [`SOURCE_COMMIT_VARIABLE`] to the full
//! commit SHA; a build from source without it prints the crate version alone.

use std::fmt;

/// The environment variable that carries the full source commit SHA.
pub(crate) const SOURCE_COMMIT_VARIABLE: &str = "VSIFT_SOURCE_COMMIT";

/// How many hexadecimal digits of the commit the version line shows: enough
/// to be unambiguous in this repository for its lifetime, short enough to read.
const SHOWN_COMMIT_DIGITS: usize = 12;

/// A source commit value that is not a full 40-digit lowercase commit SHA.
///
/// Refused rather than shown or dropped: a release that silently lost its
/// commit, or printed an arbitrary label as one, would break the mapping from
/// binary to protected commit that the suffix exists for.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct InvalidSourceCommit;

impl fmt::Display for InvalidSourceCommit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{SOURCE_COMMIT_VARIABLE} must be a full 40-character lowercase hexadecimal commit SHA"
        )
    }
}

impl std::error::Error for InvalidSourceCommit {}

/// The version text for `package_version` and an optional source commit.
///
/// An absent or empty commit gives the package version alone, as a build from
/// source has always printed; a valid commit adds its first twelve digits in
/// parentheses (`0.1.0 (1a9d027c3b5e)`), the form `rustc --version` uses.
pub(crate) fn version_text(
    package_version: &str,
    source_commit: Option<&str>,
) -> Result<String, InvalidSourceCommit> {
    match source_commit {
        None | Some("") => Ok(package_version.to_owned()),
        Some(commit) => {
            let valid = commit.len() == 40
                && commit
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
            match commit.get(..SHOWN_COMMIT_DIGITS) {
                Some(shown) if valid => Ok(format!("{package_version} ({shown})")),
                _ => Err(InvalidSourceCommit),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InvalidSourceCommit, version_text};

    const COMMIT: &str = "1a9d027c3b5e8f40a1b2c3d4e5f60718293a4b5c";

    #[test]
    fn a_build_without_a_commit_prints_the_package_version() {
        assert_eq!(version_text("0.1.0", None), Ok(String::from("0.1.0")));
        assert_eq!(version_text("0.1.0", Some("")), Ok(String::from("0.1.0")));
    }

    #[test]
    fn a_release_build_adds_the_first_twelve_digits_of_its_commit() {
        assert_eq!(
            version_text("0.1.0", Some(COMMIT)),
            Ok(String::from("0.1.0 (1a9d027c3b5e)"))
        );
    }

    #[test]
    fn anything_but_a_full_lowercase_commit_is_refused() {
        for commit in [
            "1a9d027",
            "1A9D027C3B5E8F40A1B2C3D4E5F60718293A4B5C",
            "1a9d027c3b5e8f40a1b2c3d4e5f60718293a4b5c0",
            "1a9d027c3b5e8f40a1b2c3d4e5f60718293a4b5g",
            "refs/heads/main-with-forty-characters-xx",
            "1a9d027c3b5e8f40a1b2c3d4e5f60718293a4b5 ",
        ] {
            assert_eq!(
                version_text("0.1.0", Some(commit)),
                Err(InvalidSourceCommit),
                "{commit}"
            );
        }
    }
}
