//! The evidence ledger's answer for the accepted release candidate (P14 PR 8;
//! ADR 0024, "What P14 delivers" item 1 and decision A).
//!
//! A stable version may be published only when the evidence ledger is
//! complete for the candidate it was cut from. The completeness check is
//! `vsift-governance release-evidence --complete-for <candidate> --commit
//! <candidate commit>` (P14 PR 1). The Release workflow's `plan` job runs it,
//! after `vsift-release candidate-delta --github-output` has named the
//! candidate, and keeps its exit status and its output in a directory:
//!
//! - `status`: the exit status, as digits;
//! - `result.txt`: everything the check printed.
//!
//! `publish-plan --evidence <directory>` reads them here. This tool does not
//! run the check itself, so it needs no dependency on the governance tool, and
//! a plan that was not given the directory says so instead of passing: a
//! stable plan whose candidate has no recorded answer is refused wherever the
//! plan is enforced.
//!
//! The text is the governance tool's own output about the ledger, which a
//! pull request can change, so it is bounded and made printable before any of
//! it reaches a plan.

use std::{fs, io::Read, path::Path};

use crate::candidate::printable;

/// The most bytes read from `result.txt`.
const MAXIMUM_RESULT_BYTES: u64 = 64 * 1024;

/// The most lines of the check's output a plan keeps.
const MAXIMUM_LINES: usize = 8;

/// What the evidence check answered, or that it did not run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EvidenceObservation {
    /// No answer: the directory was not given or holds no status. A stable
    /// plan treats this as a failure.
    NotRun,
    /// The check ran.
    Ran {
        /// Whether it exited with status 0.
        succeeded: bool,
        /// What it printed, bounded and printable, one entry per line.
        lines: Vec<String>,
    },
}

/// Reads the check's answer from `directory`. A missing directory or file is
/// "not run", never an error: the plan job skips the check for every version
/// that has no accepted candidate.
pub(crate) fn read_directory(directory: &Path) -> EvidenceObservation {
    let Some(status) = read_limited(&directory.join("status"), 16) else {
        return EvidenceObservation::NotRun;
    };
    let succeeded = match String::from_utf8_lossy(&status).trim() {
        "0" => true,
        digits if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) => false,
        _ => return EvidenceObservation::NotRun,
    };
    let result = read_limited(&directory.join("result.txt"), MAXIMUM_RESULT_BYTES)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    let lines = result
        .lines()
        .map(printable)
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .take(MAXIMUM_LINES)
        .collect();
    EvidenceObservation::Ran { succeeded, lines }
}

/// At most `limit` bytes of a file; `None` if it cannot be read or is longer.
fn read_limited(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    (u64::try_from(bytes.len()).ok()? <= limit).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fmt::Write, fs};

    use super::{EvidenceObservation, MAXIMUM_LINES, read_directory};

    fn scratch(name: &str) -> Result<std::path::PathBuf, Box<dyn Error>> {
        let path = std::env::temp_dir().join(format!(
            "vsift-release-evidence-test-{name}-{}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path)?;
        }
        fs::create_dir(&path)?;
        Ok(path)
    }

    #[test]
    fn a_missing_directory_or_status_is_not_run() -> Result<(), Box<dyn Error>> {
        let directory = scratch("missing")?;
        assert_eq!(
            read_directory(&directory.join("absent")),
            EvidenceObservation::NotRun
        );
        assert_eq!(read_directory(&directory), EvidenceObservation::NotRun);
        for status in ["", "ok", "-1", "0 ", "1x", "\u{0}"] {
            fs::write(directory.join("status"), status)?;
            let observed = read_directory(&directory);
            // Only whole digits are a status; "0 " is zero once trimmed.
            let expected = if status == "0 " {
                EvidenceObservation::Ran {
                    succeeded: true,
                    lines: Vec::new(),
                }
            } else {
                EvidenceObservation::NotRun
            };
            assert_eq!(observed, expected, "{status:?}");
        }
        fs::remove_dir_all(&directory)?;
        Ok(())
    }

    #[test]
    fn the_status_and_the_printable_bounded_output_are_read() -> Result<(), Box<dyn Error>> {
        let directory = scratch("ran")?;
        fs::write(directory.join("status"), "0\n")?;
        fs::write(
            directory.join("result.txt"),
            "VSift release evidence is complete for 0.2.0-rc.1 at 0123456789ab.\n",
        )?;
        assert_eq!(
            read_directory(&directory),
            EvidenceObservation::Ran {
                succeeded: true,
                lines: vec![String::from(
                    "VSift release evidence is complete for 0.2.0-rc.1 at 0123456789ab."
                )]
            }
        );
        // A failure: any non-zero status, the findings capped and made printable.
        fs::write(directory.join("status"), "1\n")?;
        let findings = (0..20).fold(String::new(), |mut text, index| {
            let _ = writeln!(text, "finding {index} `with\u{202e}marks`");
            text
        });
        fs::write(directory.join("result.txt"), findings)?;
        let EvidenceObservation::Ran { succeeded, lines } = read_directory(&directory) else {
            return Err("not run".into());
        };
        assert!(!succeeded);
        assert_eq!(lines.len(), MAXIMUM_LINES);
        assert!(
            lines
                .iter()
                .all(|line| !line.contains('`') && !line.contains('\u{202e}'))
        );
        // A result that is far too large keeps nothing but the status.
        fs::write(directory.join("result.txt"), vec![b'x'; 200_000])?;
        assert_eq!(
            read_directory(&directory),
            EvidenceObservation::Ran {
                succeeded: false,
                lines: Vec::new()
            }
        );
        fs::remove_dir_all(&directory)?;
        Ok(())
    }
}
