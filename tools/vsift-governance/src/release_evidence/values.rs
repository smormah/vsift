//! Validated scalar values of the evidence ledger.
//!
//! Dates, versions and commits arrive as JSON strings. They are parsed into
//! these types when the ledger is read, so no rule ever compares raw strings
//! and a malformed value cannot reach a status or completeness decision.

use std::fmt;

use serde::Deserialize;

/// Why a scalar value of the ledger was refused.
#[derive(Debug)]
pub(crate) struct ValueError(String);

impl fmt::Display for ValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ValueError {}

/// A calendar date written `YYYY-MM-DD` (UTC, as every date in the plan).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct IsoDate {
    year: u16,
    month: u8,
    day: u8,
}

impl TryFrom<String> for IsoDate {
    type Error = ValueError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let refuse = || ValueError(format!("{text:?} is not a date written YYYY-MM-DD"));
        let mut parts = text.split('-');
        let (Some(year), Some(month), Some(day), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(refuse());
        };
        let shaped = year.len() == 4
            && month.len() == 2
            && day.len() == 2
            && [year, month, day]
                .iter()
                .all(|part| part.bytes().all(|byte| byte.is_ascii_digit()));
        if !shaped {
            return Err(refuse());
        }
        let (Ok(year), Ok(month), Ok(day)) =
            (year.parse::<u16>(), month.parse::<u8>(), day.parse::<u8>())
        else {
            return Err(refuse());
        };
        if !(2000..=2200).contains(&year) || day == 0 || day > days_in_month(year, month) {
            return Err(refuse());
        }
        Ok(Self { year, month, day })
    }
}

impl fmt::Display for IsoDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:04}-{:02}-{:02}",
            self.year, self.month, self.day
        )
    }
}

/// The number of days in `month` of `year`, or 0 for a month that does not
/// exist (so every day of it is refused).
fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => 0,
    }
}

/// A full 40-digit lowercase hexadecimal Git commit identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct CommitSha(String);

impl CommitSha {
    /// The 40 hexadecimal digits.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CommitSha {
    type Error = ValueError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let valid = text.len() == 40
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if valid {
            Ok(Self(text))
        } else {
            Err(ValueError(format!(
                "{text:?} is not a full 40-digit lowercase commit"
            )))
        }
    }
}

impl fmt::Display for CommitSha {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A release version: `MAJOR.MINOR.PATCH`, with an optional `-rc.N` suffix.
///
/// A version with the suffix is a release candidate (ADR 0024 decision B); one
/// without it is stable (decision A). No other suffix exists in the plan, so
/// no other is accepted.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct ReleaseVersion {
    major: u32,
    minor: u32,
    patch: u32,
    candidate: Option<u32>,
}

impl ReleaseVersion {
    /// Parses a version given on a command line or in a file.
    pub(crate) fn parse(text: &str) -> Result<Self, ValueError> {
        let refuse = || {
            ValueError(format!(
                "{text:?} is not a release version (MAJOR.MINOR.PATCH, optionally -rc.N)"
            ))
        };
        let (core, candidate) = match text.split_once("-rc.") {
            Some((core, number)) => (core, Some(number)),
            None => (text, None),
        };
        let mut numbers = core.split('.');
        let (Some(major), Some(minor), Some(patch), None) = (
            numbers.next(),
            numbers.next(),
            numbers.next(),
            numbers.next(),
        ) else {
            return Err(refuse());
        };
        let major = parse_number(major).ok_or_else(refuse)?;
        let minor = parse_number(minor).ok_or_else(refuse)?;
        let patch = parse_number(patch).ok_or_else(refuse)?;
        let candidate = match candidate {
            None => None,
            Some(number) => Some(parse_number(number).filter(|n| *n > 0).ok_or_else(refuse)?),
        };
        Ok(Self {
            major,
            minor,
            patch,
            candidate,
        })
    }

    /// Whether this is a release candidate (`-rc.N`).
    pub(crate) fn is_candidate(self) -> bool {
        self.candidate.is_some()
    }

    /// The stable version of the same release: this version without `-rc.N`.
    pub(crate) fn without_candidate(self) -> Self {
        Self {
            candidate: None,
            ..self
        }
    }
}

/// A decimal number without a sign, a leading zero or any other character.
fn parse_number(text: &str) -> Option<u32> {
    let plain = !text.is_empty()
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    if plain { text.parse().ok() } else { None }
}

impl TryFrom<String> for ReleaseVersion {
    type Error = ValueError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl fmt::Display for ReleaseVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)?;
        match self.candidate {
            Some(number) => write!(formatter, "-rc.{number}"),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CommitSha, IsoDate, ReleaseVersion};

    #[test]
    fn dates_are_calendar_dates() {
        for text in ["2026-10-02", "2028-02-29", "2026-12-31"] {
            assert!(IsoDate::try_from(text.to_owned()).is_ok(), "{text}");
        }
        for text in [
            "2026-10-2",
            "26-10-02",
            "2026/10/02",
            "2026-13-01",
            "2026-02-29",
            "2026-04-31",
            "2026-00-10",
            "2026-10-00",
            "2026-10-02T00:00",
            "2026-+1-02",
            "",
        ] {
            assert!(IsoDate::try_from(text.to_owned()).is_err(), "{text}");
        }
    }

    #[test]
    fn a_date_prints_as_it_was_written() -> Result<(), super::ValueError> {
        assert_eq!(
            IsoDate::try_from(String::from("2026-10-02"))?.to_string(),
            "2026-10-02"
        );
        Ok(())
    }

    #[test]
    fn commits_are_full_lowercase_hex() {
        let good = "011bc4da1af62a837d7ac5f319fc0ee55c9cb2aa";
        assert!(CommitSha::try_from(good.to_owned()).is_ok());
        for text in [
            "011bc4da1af6",
            "011BC4DA1AF62A837D7AC5F319FC0EE55C9CB2AA",
            "011bc4da1af62a837d7ac5f319fc0ee55c9cb2a",
            "011bc4da1af62a837d7ac5f319fc0ee55c9cb2aag",
            "",
        ] {
            assert!(CommitSha::try_from(text.to_owned()).is_err(), "{text}");
        }
    }

    #[test]
    fn versions_are_stable_or_release_candidates() -> Result<(), super::ValueError> {
        assert!(!ReleaseVersion::parse("0.2.0")?.is_candidate());
        assert!(ReleaseVersion::parse("0.2.0-rc.1")?.is_candidate());
        assert_eq!(
            ReleaseVersion::parse("0.2.0-rc.2")?.to_string(),
            "0.2.0-rc.2"
        );
        for text in [
            "0.2",
            "0.2.0.1",
            "v0.2.0",
            "0.2.0-rc",
            "0.2.0-rc.0",
            "0.2.0-beta.1",
            "0.2.0-rc.1-extra",
            "01.2.0",
            "0.2.-1",
            "",
        ] {
            assert!(ReleaseVersion::parse(text).is_err(), "{text}");
        }
        Ok(())
    }
}
