//! The candidate-to-stable delta check (P14 PR 8, ADR 0024 decisions A and
//! B).
//!
//! A stable version is published from a tag, by the same workflow as a
//! release candidate, but only after a candidate of it (`v<X.Y.Z>-rc.<N>`)
//! was cut, published and qualified. The stable commit must then be the
//! candidate plus nothing but the stable's own version strings and the
//! documents that ship inside the artifacts, so the qualification of the
//! candidate is the qualification of the stable's code. This module is the
//! mechanical form of that rule (the "candidate rule" of decision B): it
//! compares the two commits path by path and refuses every other difference.
//!
//! - The **accepted candidate** is the highest-numbered `v<X.Y.Z>-rc.<N>` tag
//!   of the stable's own version. Only the maintainer can create a `v*` tag
//!   (the tag ruleset); taking the highest number means a stable can never be
//!   built on an older candidate than the last one cut.
//! - Every file that differs must be one of two kinds, both named below and
//!   nothing else: a **version-string file**, whose stable content must equal
//!   the candidate's with the candidate's version text replaced by the
//!   stable's (so a manifest or lockfile may change that and nothing else);
//!   or a **shipped document**, which may change freely because it is text
//!   that travels inside the packages and nothing else reads it as code.
//! - The edit must be an ordinary file edit (`M`, mode `100644` before and
//!   after): an added, deleted, renamed, re-moded or linked path is refused.
//! - The candidate must be an ancestor of the stable commit.
//!
//! The lists are deliberately short. The skill (`skills/vsift/`) is *not* a
//! shipped document here: its bytes are what the named-client trials
//! qualified (their digest is frozen in every trial record), so changing it
//! after the candidate would ship an unqualified skill. The release notes
//! and the platform packages' README are generated from this repository's
//! code, which is frozen at the candidate cut too. Adding a path is a
//! reviewed change to the constants below, in the same pull request as a
//! mutation test and the documentation.
//!
//! Git is run with explicit arguments, never through a shell, against the
//! repository in the working directory, and never touches the network: the
//! workflow's checkout fetches the full history and the tags first.

use std::{
    fmt::Write as _,
    path::Path,
    process::{Command, Stdio},
};

use crate::publish::{ReleaseKind, ReleaseVersion, is_full_commit};

/// Files whose stable content must equal the candidate's with the candidate's
/// version text replaced by the stable's, byte for byte: the workspace and
/// fuzz manifests and lockfiles, the launcher's manifest (its version and the
/// three exact optional dependencies) and the changelog's headings.
pub(crate) const VERSION_STRING_FILES: [&str; 6] = [
    "Cargo.toml",
    "Cargo.lock",
    "fuzz/Cargo.toml",
    "fuzz/Cargo.lock",
    "npm/vsift-cli/package.json",
    "CHANGELOG.md",
];

/// Documents that ship inside the artifacts and may change freely between the
/// candidate and the stable: the launcher package's README, whose install
/// instructions differ between a release candidate and a stable release.
pub(crate) const SHIPPED_DOCUMENT_FILES: [&str; 1] = ["npm/vsift-cli/README.md"];

/// The largest file read to compare a version-string file.
const MAXIMUM_BLOB_BYTES: usize = 16 * 1024 * 1024;

/// The longest path printed in a report.
const MAXIMUM_PRINTED_PATH: usize = 200;

/// A path that changed between the two commits, as `git diff-tree --raw -z`
/// reports it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Change {
    /// The path, as git wrote it.
    pub path: String,
    /// The mode before, such as `100644`.
    pub old_mode: String,
    /// The mode after.
    pub new_mode: String,
    /// The status letter: `M` modified, `A` added, `D` deleted, `T` type
    /// changed, and so on.
    pub status: char,
    /// The object before.
    pub old_object: String,
    /// The object after.
    pub new_object: String,
}

/// Why a changed path is allowed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChangeClass {
    /// Only the version text differs.
    VersionString,
    /// A document that ships inside the artifacts.
    ShippedDocument,
}

/// One changed path and the verdict on it: why it is allowed, or why it is
/// not.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChangedPath {
    /// The path, made printable.
    pub path: String,
    /// `Ok` if the difference is allowed.
    pub verdict: Result<ChangeClass, String>,
}

/// The comparison of the accepted candidate with the stable commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CandidateReport {
    /// The candidate's tag, such as `v0.2.0-rc.1`.
    pub candidate_tag: String,
    /// The candidate's version, such as `0.2.0-rc.1`.
    pub candidate_version: String,
    /// The commit of the candidate tag.
    pub candidate_commit: String,
    /// The stable commit.
    pub stable_commit: String,
    /// Whether the candidate commit is an ancestor of the stable commit.
    pub ancestor: bool,
    /// Every path that differs, in git's order.
    pub changes: Vec<ChangedPath>,
}

impl CandidateReport {
    /// Everything that stops the stable: a candidate that is not an ancestor,
    /// the same commit, and every path that differs without being allowed.
    pub(crate) fn violations(&self) -> Vec<String> {
        let mut violations = Vec::new();
        if self.candidate_commit == self.stable_commit {
            violations.push(String::from(
                "the stable commit is the candidate's commit: a stable version needs its own \
                 commit, with its own version strings",
            ));
        }
        if !self.ancestor {
            violations.push(format!(
                "the candidate {} is not an ancestor of this commit",
                self.candidate_tag
            ));
        }
        for change in &self.changes {
            if let Err(reason) = &change.verdict {
                violations.push(format!("`{}` {reason}", change.path));
            }
        }
        violations
    }

    /// One line for the plan's guard table.
    pub(crate) fn summary(&self) -> String {
        let count = |class: ChangeClass| {
            self.changes
                .iter()
                .filter(|change| change.verdict == Ok(class))
                .count()
        };
        format!(
            "against `{}` (commit `{}`): {} version-string and {} shipped-document files differ, \
             nothing else",
            self.candidate_tag,
            short(&self.candidate_commit),
            count(ChangeClass::VersionString),
            count(ChangeClass::ShippedDocument),
        )
    }

    /// The report in words, for `vsift-release candidate-delta`.
    pub(crate) fn markdown(&self) -> String {
        let mut text = String::new();
        let _ = writeln!(
            text,
            "Candidate `{}` (version {}, commit `{}`) against stable commit `{}`:\n",
            self.candidate_tag, self.candidate_version, self.candidate_commit, self.stable_commit
        );
        for change in &self.changes {
            let _ = match &change.verdict {
                Ok(ChangeClass::VersionString) => {
                    writeln!(text, "- `{}`: version string only", change.path)
                }
                Ok(ChangeClass::ShippedDocument) => {
                    writeln!(text, "- `{}`: shipped document", change.path)
                }
                Err(reason) => writeln!(text, "- `{}` {reason}: REFUSED", change.path),
            };
        }
        if self.changes.is_empty() {
            let _ = writeln!(text, "- no path differs");
        }
        let violations = self.violations();
        if violations.is_empty() {
            let _ = writeln!(
                text,
                "\nThe differences are limited to version strings and the launcher's README."
            );
        } else {
            let _ = writeln!(text, "\nNot allowed:");
            for violation in violations {
                let _ = writeln!(text, "- {violation}");
            }
        }
        text
    }
}

/// What the candidate check found for one plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CandidateObservation {
    /// Not a stable version: there is no candidate to compare with.
    NotApplicable,
    /// The comparison ran; the report may hold violations.
    Checked(Box<CandidateReport>),
    /// The comparison could not be made (no candidate tag, git failed, the
    /// full history is not there); the reason names no file content.
    Failed(String),
}

pub(crate) fn short(commit: &str) -> &str {
    commit.get(..12).unwrap_or(commit)
}

/// The lines `vsift-release candidate-delta --github-output` prints for
/// `$GITHUB_OUTPUT`: the accepted candidate's version and commit when one was
/// found, nothing otherwise. The plan job's evidence step runs only when they
/// are there.
pub(crate) fn github_output(observation: &CandidateObservation) -> String {
    match observation {
        CandidateObservation::Checked(report) => format!(
            "candidate-version={}\ncandidate-commit={}",
            report.candidate_version, report.candidate_commit
        ),
        CandidateObservation::NotApplicable | CandidateObservation::Failed(_) => String::new(),
    }
}

/// When and in which run a candidate comparison was made: what the evidence
/// ledger's `release_delta` records beside the comparison.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CheckRecord {
    /// The workflow run (`github.run_id`).
    pub run_id: u64,
    /// The date, `YYYY-MM-DD`, in UTC.
    pub date: String,
}

impl CheckRecord {
    /// Whether `text` is shaped like `YYYY-MM-DD`. The evidence ledger's own
    /// check holds the calendar rules.
    pub(crate) fn is_date(text: &str) -> bool {
        let parts: Vec<&str> = text.split('-').collect();
        matches!(parts.as_slice(), [year, month, day]
            if year.len() == 4 && month.len() == 2 && day.len() == 2
                && [year, month, day].iter().all(|part| part.bytes().all(|byte| byte.is_ascii_digit())))
    }
}

/// The `release_delta` record of the evidence ledger
/// (`docs/planning/p14-evidence-ledger.json`) for a comparison: the accepted
/// candidate, the stable version and commit, the verdict and the run that made
/// it. The maintainer copies it into the ledger after the stable is published;
/// the ledger's completeness check for the stable release refuses to carry
/// candidate evidence without it. The shape is held by a shared example that
/// the governance tool parses (`tools/vsift-release/tests/release-delta.example.json`).
pub(crate) fn release_delta(
    report: &CandidateReport,
    stable_version: &str,
    record: &CheckRecord,
) -> serde_json::Value {
    let verdict = if report.violations().is_empty() {
        "allowed"
    } else {
        "rejected"
    };
    serde_json::json!({
        "candidate_version": report.candidate_version,
        "candidate_commit": report.candidate_commit,
        "stable_version": stable_version,
        "stable_commit": report.stable_commit,
        "verdict": verdict,
        "check": {
            "type": "workflow_run",
            "workflow": "Release",
            "run_id": record.run_id,
        },
        "date": record.date,
    })
}

/// A path as it may be printed in a report: ASCII graphic characters and
/// spaces only (anything else becomes `?`), without a backtick, at most
/// [`MAXIMUM_PRINTED_PATH`] characters. Paths come from the checked-out
/// commit, which a fork's pull request controls.
pub(crate) fn printable(path: &str) -> String {
    path.chars()
        .take(MAXIMUM_PRINTED_PATH)
        .map(|character| {
            if (character.is_ascii_graphic() || character == ' ') && character != '`' {
                character
            } else {
                '?'
            }
        })
        .collect()
}

/// The candidate tag names for `stable`, highest number first: only
/// `v<X.Y.Z>-rc.<N>` for the stable's own `X.Y.Z` and a positive `N` without
/// a leading zero. Any other tag is ignored, never read as a candidate.
pub(crate) fn candidate_tags(stable: &ReleaseVersion, listed: &[String]) -> Vec<(u64, String)> {
    let prefix = format!("v{}-rc.", stable.as_str());
    let mut found: Vec<(u64, String)> = listed
        .iter()
        .filter_map(|tag| {
            let number = tag.strip_prefix(&prefix)?;
            let valid = !number.is_empty()
                && number.bytes().all(|byte| byte.is_ascii_digit())
                && !number.starts_with('0');
            if !valid {
                return None;
            }
            Some((number.parse::<u64>().ok()?, tag.clone()))
        })
        .collect();
    found.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
    found
}

/// `haystack` with every non-overlapping `from` replaced by `to`.
pub(crate) fn replace_all(haystack: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    if from.is_empty() {
        return haystack.to_vec();
    }
    let mut result = Vec::with_capacity(haystack.len());
    let mut index = 0;
    while let Some(rest) = haystack.get(index..) {
        if rest.starts_with(from) {
            result.extend_from_slice(to);
            index += from.len();
        } else if let Some(&byte) = rest.first() {
            result.push(byte);
            index += 1;
        } else {
            break;
        }
    }
    result
}

/// Judges every change. `read_object` returns the bytes of a git object; it
/// is called only for version-string files.
pub(crate) fn evaluate(
    changes: &[Change],
    candidate_version: &str,
    stable_version: &str,
    read_object: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
) -> Vec<ChangedPath> {
    changes
        .iter()
        .map(|change| ChangedPath {
            path: printable(&change.path),
            verdict: classify(change, candidate_version, stable_version, read_object),
        })
        .collect()
}

fn classify(
    change: &Change,
    candidate_version: &str,
    stable_version: &str,
    read_object: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<ChangeClass, String> {
    let ordinary_edit =
        change.status == 'M' && change.old_mode == "100644" && change.new_mode == "100644";
    let path = change.path.as_str();
    let version_file = VERSION_STRING_FILES.contains(&path);
    let document = SHIPPED_DOCUMENT_FILES.contains(&path);
    if !version_file && !document {
        return Err(String::from(
            "may not differ between the candidate and the stable release: only version strings \
             and the launcher's README may",
        ));
    }
    if !ordinary_edit {
        return Err(format!(
            "is {} ({} to {}); only an edit of an ordinary file is allowed",
            describe_status(change.status),
            change.old_mode,
            change.new_mode
        ));
    }
    if document {
        return Ok(ChangeClass::ShippedDocument);
    }
    let before = read_object(&change.old_object)?;
    let after = read_object(&change.new_object)?;
    let expected = replace_all(
        &before,
        candidate_version.as_bytes(),
        stable_version.as_bytes(),
    );
    if expected == after {
        Ok(ChangeClass::VersionString)
    } else {
        Err(format!(
            "changes more than the version text `{candidate_version}` to `{stable_version}`"
        ))
    }
}

fn describe_status(status: char) -> &'static str {
    match status {
        'A' => "added",
        'D' => "deleted",
        'T' => "changed in type",
        'M' => "modified",
        _ => "changed in an unexpected way",
    }
}

/// Parses `git diff-tree -r --raw -z --no-abbrev --no-renames <a> <b>`:
/// `:<old mode> <new mode> <old object> <new object> <status>` and the path,
/// each ended by NUL.
pub(crate) fn parse_raw_diff(output: &[u8]) -> Result<Vec<Change>, String> {
    let text = std::str::from_utf8(output)
        .map_err(|_| String::from("git's list of changed paths is not UTF-8 text"))?;
    let mut fields = text.split('\0');
    let mut changes = Vec::new();
    while let Some(meta) = fields.next() {
        if meta.is_empty() {
            break;
        }
        let path = fields
            .next()
            .filter(|path| !path.is_empty())
            .ok_or_else(|| String::from("git's list of changed paths ends inside an entry"))?;
        let words: Vec<&str> = meta.trim_start_matches(':').split_whitespace().collect();
        let [old_mode, new_mode, old_object, new_object, status] = words.as_slice() else {
            return Err(String::from(
                "git's list of changed paths has an unexpected entry",
            ));
        };
        let mut letters = status.chars();
        let (Some(status), None) = (letters.next(), letters.next()) else {
            return Err(String::from(
                "git's list of changed paths has an unexpected status",
            ));
        };
        changes.push(Change {
            path: path.to_owned(),
            old_mode: (*old_mode).to_owned(),
            new_mode: (*new_mode).to_owned(),
            status,
            old_object: (*old_object).to_owned(),
            new_object: (*new_object).to_owned(),
        });
    }
    Ok(changes)
}

/// Whether `text` is a git object name: 40 or 64 lowercase hexadecimal digits.
fn is_object_name(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Runs git in `root` with explicit arguments: no shell, no prompt, no
/// pager, no stdin.
fn git(root: &Path, arguments: &[&str]) -> Result<std::process::Output, String> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("git could not be run: {}", error.kind()))
}

/// Git's output on success, or a message that names the command and no
/// repository content.
fn git_output(root: &Path, arguments: &[&str]) -> Result<Vec<u8>, String> {
    let output = git(root, arguments)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(format!(
            "`git {}` failed",
            arguments.first().copied().unwrap_or_default()
        ))
    }
}

fn commit_of(root: &Path, revision: &str) -> Result<Option<String>, String> {
    let output = git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{revision}^{{commit}}"),
        ],
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_owned(),
    ))
}

/// The full SHA of `HEAD` in the repository at `root`.
pub(crate) fn head_commit(root: &Path) -> Result<String, String> {
    commit_of(root, "HEAD")?
        .filter(|commit| is_full_commit(commit))
        .ok_or_else(|| String::from("HEAD is not a commit of this repository"))
}

/// Compares the accepted candidate of `stable` with `stable_commit` in the
/// repository at `root`.
pub(crate) fn observe(
    root: &Path,
    stable_commit: &str,
    stable: &ReleaseVersion,
) -> CandidateObservation {
    if stable.kind() != ReleaseKind::Stable {
        return CandidateObservation::NotApplicable;
    }
    match compare(root, stable_commit, stable) {
        Ok(report) => CandidateObservation::Checked(Box::new(report)),
        Err(reason) => CandidateObservation::Failed(reason),
    }
}

fn compare(
    root: &Path,
    stable_commit: &str,
    stable: &ReleaseVersion,
) -> Result<CandidateReport, String> {
    if !is_full_commit(stable_commit) {
        return Err(String::from("the stable commit is not a full commit SHA"));
    }
    if commit_of(root, stable_commit)?.as_deref() != Some(stable_commit) {
        return Err(String::from(
            "this commit is not in the clone: the checkout must fetch the full history and the tags",
        ));
    }
    let pattern = format!("v{}-rc.*", stable.as_str());
    let listing = git_output(root, &["tag", "--list", &pattern])?;
    let tags: Vec<String> = String::from_utf8_lossy(&listing)
        .lines()
        .map(str::to_owned)
        .collect();
    let Some((_, candidate_tag)) = candidate_tags(stable, &tags).into_iter().next() else {
        return Err(format!(
            "no release candidate tag `v{}-rc.<N>` exists: a stable version is published only \
             after a candidate of it",
            stable.as_str()
        ));
    };
    let candidate_version = candidate_tag.trim_start_matches('v').to_owned();
    let candidate_commit = commit_of(root, &format!("refs/tags/{candidate_tag}"))?
        .filter(|commit| is_full_commit(commit))
        .ok_or_else(|| format!("the tag {candidate_tag} does not name a commit"))?;
    let ancestor = {
        let output = git(
            root,
            &[
                "merge-base",
                "--is-ancestor",
                &candidate_commit,
                stable_commit,
            ],
        )?;
        match output.status.code() {
            Some(0) => true,
            Some(1) => false,
            _ => return Err(String::from("`git merge-base` failed")),
        }
    };
    let raw = git_output(
        root,
        &[
            "diff-tree",
            "-r",
            "--raw",
            "-z",
            "--no-abbrev",
            "--no-renames",
            &candidate_commit,
            stable_commit,
        ],
    )?;
    let changes = parse_raw_diff(&raw)?;
    let mut read_object = |object: &str| -> Result<Vec<u8>, String> {
        if !is_object_name(object) {
            return Err(String::from("git named an object in an unexpected form"));
        }
        let bytes = git_output(root, &["cat-file", "blob", object])?;
        if bytes.len() > MAXIMUM_BLOB_BYTES {
            return Err(String::from("is larger than a version-string file can be"));
        }
        Ok(bytes)
    };
    let changes = evaluate(
        &changes,
        &candidate_version,
        stable.as_str(),
        &mut read_object,
    );
    Ok(CandidateReport {
        candidate_tag,
        candidate_version,
        candidate_commit,
        stable_commit: stable_commit.to_owned(),
        ancestor,
        changes,
    })
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs, path::Path, path::PathBuf, process::Command};

    use super::{
        CandidateObservation, CandidateReport, Change, ChangeClass, ChangedPath, CheckRecord,
        SHIPPED_DOCUMENT_FILES, VERSION_STRING_FILES, candidate_tags, evaluate, github_output,
        observe, parse_raw_diff, printable, release_delta, replace_all,
    };
    use crate::publish::ReleaseVersion;

    const CANDIDATE: &str = "0.2.0-rc.1";
    const STABLE: &str = "0.2.0";

    fn change(path: &str, status: char, old_mode: &str, new_mode: &str) -> Change {
        Change {
            path: path.to_owned(),
            old_mode: old_mode.to_owned(),
            new_mode: new_mode.to_owned(),
            status,
            old_object: format!("{:a<40}", ""),
            new_object: format!("{:b<40}", ""),
        }
    }

    fn modified(path: &str) -> Change {
        change(path, 'M', "100644", "100644")
    }

    /// The two blobs of every change in these tests: `a...` is the
    /// candidate's file and `b...` the stable's.
    fn reader(
        before: &'static str,
        after: &'static str,
    ) -> impl FnMut(&str) -> Result<Vec<u8>, String> {
        move |object: &str| {
            Ok(if object.starts_with('a') {
                before.as_bytes().to_vec()
            } else {
                after.as_bytes().to_vec()
            })
        }
    }

    fn verdicts(
        changes: &[Change],
        before: &'static str,
        after: &'static str,
    ) -> Vec<Result<ChangeClass, String>> {
        evaluate(changes, CANDIDATE, STABLE, &mut reader(before, after))
            .into_iter()
            .map(|changed| changed.verdict)
            .collect()
    }

    #[test]
    fn the_allowed_files_are_exactly_these() {
        // Changing a list is a reviewed decision (release.md section 6.8):
        // this test makes the edit visible in the diff of the test too.
        assert_eq!(
            VERSION_STRING_FILES,
            [
                "Cargo.toml",
                "Cargo.lock",
                "fuzz/Cargo.toml",
                "fuzz/Cargo.lock",
                "npm/vsift-cli/package.json",
                "CHANGELOG.md",
            ]
        );
        assert_eq!(SHIPPED_DOCUMENT_FILES, ["npm/vsift-cli/README.md"]);
        // The skill's bytes are what the agent trials qualified.
        for path in VERSION_STRING_FILES.iter().chain(&SHIPPED_DOCUMENT_FILES) {
            assert!(!path.starts_with("skills/"), "{path}");
            assert!(!path.starts_with("crates/"), "{path}");
            assert!(!path.starts_with("tools/"), "{path}");
            assert!(!path.starts_with(".github/"), "{path}");
        }
    }

    #[test]
    fn a_version_string_edit_is_allowed_and_nothing_more_is() {
        let before = "version = \"0.2.0-rc.1\"\nname = \"vsift\"\n";
        let after = "version = \"0.2.0\"\nname = \"vsift\"\n";
        for path in VERSION_STRING_FILES {
            assert_eq!(
                verdicts(&[modified(path)], before, after),
                [Ok(ChangeClass::VersionString)],
                "{path}"
            );
        }
        // Something else changed in the same file.
        let extra = "version = \"0.2.0\"\nname = \"vsift\"\nextra = 1\n";
        let result = verdicts(&[modified("Cargo.toml")], before, extra);
        assert!(
            matches!(&result[..], [Err(reason)] if reason.contains("changes more than the version text")),
            "{result:?}"
        );
        // The version was not changed at all.
        let unchanged = verdicts(&[modified("Cargo.lock")], before, before);
        assert!(matches!(&unchanged[..], [Err(_)]), "{unchanged:?}");
        // A different new version.
        let wrong = "version = \"0.2.1\"\nname = \"vsift\"\n";
        assert!(matches!(
            &verdicts(&[modified("Cargo.toml")], before, wrong)[..],
            [Err(_)]
        ));
    }

    #[test]
    fn a_shipped_document_may_change_freely() {
        assert_eq!(
            verdicts(
                &[modified("npm/vsift-cli/README.md")],
                "old\n",
                "entirely new\n"
            ),
            [Ok(ChangeClass::ShippedDocument)]
        );
    }

    #[test]
    fn every_other_path_is_refused_with_its_name() {
        for path in [
            "crates/vsift-cli/src/main.rs",
            "skills/vsift/SKILL.md",
            ".github/workflows/release.yml",
            "tools/vsift-release/src/publish.rs",
            "npm/vsift-cli/lib/launcher.cjs",
            "npm/vsift-cli/bin/vsift.cjs",
            "docs/operations/install.md",
            "README.md",
            "memory/TODO.md",
            "LICENSE",
            "deny.toml",
            "rust-toolchain.toml",
        ] {
            let changed = evaluate(&[modified(path)], CANDIDATE, STABLE, &mut reader("a", "b"));
            assert_eq!(changed.len(), 1);
            assert!(
                matches!(&changed[0].verdict, Err(reason) if reason.contains("may not differ")),
                "{path}: {changed:?}"
            );
            assert_eq!(changed[0].path, path);
        }
    }

    #[test]
    fn an_allowed_path_must_be_an_ordinary_edit() {
        for (status, old_mode, new_mode, word) in [
            ('A', "000000", "100644", "added"),
            ('D', "100644", "000000", "deleted"),
            ('T', "100644", "120000", "changed in type"),
            ('M', "100644", "100755", "modified"),
            ('M', "100644", "120000", "modified"),
            ('M', "160000", "160000", "modified"),
        ] {
            for path in ["Cargo.toml", "npm/vsift-cli/README.md"] {
                let result = verdicts(
                    &[change(path, status, old_mode, new_mode)],
                    "0.2.0-rc.1",
                    "0.2.0",
                );
                assert!(
                    matches!(&result[..], [Err(reason)] if reason.contains(word)),
                    "{path} {status} {old_mode} {new_mode}: {result:?}"
                );
            }
        }
    }

    #[test]
    fn an_unreadable_object_refuses_the_path() {
        let mut failing = |_: &str| -> Result<Vec<u8>, String> { Err(String::from("not found")) };
        let changed = evaluate(&[modified("Cargo.toml")], CANDIDATE, STABLE, &mut failing);
        assert_eq!(changed[0].verdict, Err(String::from("not found")));
    }

    #[test]
    fn paths_are_made_printable() {
        assert_eq!(printable("a/b.rs"), "a/b.rs");
        assert_eq!(printable("a`b\nc\u{202e}d"), "a?b?c?d");
        assert_eq!(printable(&"x".repeat(500)).len(), 200);
    }

    #[test]
    fn replacement_is_exact_and_does_not_overlap() {
        assert_eq!(
            replace_all(b"a-0.2.0-rc.1-b-0.2.0-rc.1", b"0.2.0-rc.1", b"0.2.0"),
            b"a-0.2.0-b-0.2.0"
        );
        assert_eq!(replace_all(b"aaa", b"aa", b"b"), b"ba");
        assert_eq!(replace_all(b"abc", b"", b"x"), b"abc");
        assert_eq!(replace_all(b"", b"a", b"b"), b"");
        assert_eq!(
            replace_all(b"0.2.0-rc.10", b"0.2.0-rc.1", b"0.2.0"),
            b"0.2.00"
        );
    }

    #[test]
    fn candidate_tags_are_the_stables_own_and_the_highest_comes_first() -> Result<(), Box<dyn Error>>
    {
        let stable = ReleaseVersion::parse(STABLE)?;
        let listed: Vec<String> = [
            "v0.2.0-rc.1",
            "v0.2.0-rc.10",
            "v0.2.0-rc.9",
            "v0.2.0-rc.0",
            "v0.2.0-rc.01",
            "v0.2.0-rc.",
            "v0.2.0-rc.1-extra",
            "v0.2.0-rc.1.2",
            "v0.2.0-beta.1",
            "v0.2.1-rc.1",
            "v0.2.00-rc.1",
            "v0.2.0",
            "0.2.0-rc.2",
            "v0.2.0-rc.99999999999999999999999999",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let found: Vec<String> = candidate_tags(&stable, &listed)
            .into_iter()
            .map(|(_, tag)| tag)
            .collect();
        assert_eq!(found, ["v0.2.0-rc.10", "v0.2.0-rc.9", "v0.2.0-rc.1"]);
        Ok(())
    }

    #[test]
    fn git_s_raw_listing_is_parsed() -> Result<(), Box<dyn Error>> {
        let old = "a".repeat(40);
        let new = "b".repeat(40);
        let output = format!(
            ":100644 100644 {old} {new} M\0Cargo.toml\0:000000 100644 {old} {new} A\0dir/new file.rs\0"
        );
        let changes = parse_raw_diff(output.as_bytes())?;
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].path, "Cargo.toml");
        assert_eq!(changes[0].status, 'M');
        assert_eq!(changes[1].path, "dir/new file.rs");
        assert_eq!(changes[1].old_mode, "000000");
        assert_eq!(parse_raw_diff(b"")?, Vec::<Change>::new());
        for bad in [
            ":100644 100644 a b M\0".to_owned(),
            ":100644 100644 a b MM\0Cargo.toml\0".to_owned(),
            ":100644 100644 a b\0Cargo.toml\0".to_owned(),
        ] {
            assert!(parse_raw_diff(bad.as_bytes()).is_err(), "{bad:?}");
        }
        assert!(parse_raw_diff(&[0xff, 0xfe]).is_err());
        Ok(())
    }

    /// The record the plan writes is exactly the shape the evidence ledger's
    /// `release_delta` reads: this example is parsed by the governance tool's
    /// own tests too (`completeness` and `structure`), so neither side can
    /// change the shape alone.
    #[test]
    fn the_release_delta_record_is_the_shared_example() -> Result<(), Box<dyn Error>> {
        let report = CandidateReport {
            candidate_tag: String::from("v0.2.0-rc.1"),
            candidate_version: String::from(CANDIDATE),
            candidate_commit: "1".repeat(40),
            stable_commit: "2".repeat(40),
            ancestor: true,
            changes: vec![ChangedPath {
                path: String::from("Cargo.toml"),
                verdict: Ok(ChangeClass::VersionString),
            }],
        };
        let record = CheckRecord {
            run_id: 36_959_682_491,
            date: String::from("2026-11-01"),
        };
        let written = release_delta(&report, STABLE, &record);
        let example: serde_json::Value = serde_json::from_str(&fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/release-delta.example.json"),
        )?)?;
        assert_eq!(written, example);
        // A comparison that found something else is a rejected delta.
        let mut rejected = report.clone();
        rejected.changes.push(ChangedPath {
            path: String::from("crates/x.rs"),
            verdict: Err(String::from("may not differ")),
        });
        assert_eq!(
            release_delta(&rejected, STABLE, &record)["verdict"],
            "rejected"
        );
        assert!(CheckRecord::is_date("2026-11-01"));
        for bad in [
            "",
            "2026-1-01",
            "26-11-01",
            "2026/11/01",
            "2026-11-01x",
            "2026-11",
        ] {
            assert!(!CheckRecord::is_date(bad), "{bad:?}");
        }
        Ok(())
    }

    #[test]
    fn the_github_output_names_the_accepted_candidate_or_nothing() {
        let report = CandidateReport {
            candidate_tag: String::from("v0.2.0-rc.3"),
            candidate_version: String::from("0.2.0-rc.3"),
            candidate_commit: "a".repeat(40),
            stable_commit: "b".repeat(40),
            ancestor: true,
            changes: Vec::new(),
        };
        assert_eq!(
            github_output(&CandidateObservation::Checked(Box::new(report))),
            format!(
                "candidate-version=0.2.0-rc.3\ncandidate-commit={}",
                "a".repeat(40)
            )
        );
        assert_eq!(github_output(&CandidateObservation::NotApplicable), "");
        assert_eq!(
            github_output(&CandidateObservation::Failed(String::from("no tag"))),
            ""
        );
    }

    // The tests below run git on throwaway repositories. Their identity is
    // made up and no configuration of the machine is read or written.

    struct Repository {
        path: PathBuf,
    }

    impl Repository {
        fn new(name: &str) -> Result<Self, Box<dyn Error>> {
            let path = std::env::temp_dir().join(format!(
                "vsift-release-candidate-{name}-{}",
                std::process::id()
            ));
            if path.exists() {
                fs::remove_dir_all(&path)?;
            }
            fs::create_dir(&path)?;
            let repository = Self { path };
            repository.git(&["init", "--quiet", "--initial-branch=main"])?;
            Ok(repository)
        }

        fn git(&self, arguments: &[&str]) -> Result<String, Box<dyn Error>> {
            let output = Command::new("git")
                .arg("-C")
                .arg(&self.path)
                .args([
                    "-c",
                    "user.name=Release Test",
                    "-c",
                    "user.email=release-test@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "tag.gpgsign=false",
                    "-c",
                    "core.autocrlf=false",
                    "-c",
                    "core.safecrlf=false",
                ])
                .args(arguments)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.path.join("no-such-global-config"))
                .output()?;
            if !output.status.success() {
                return Err(format!(
                    "git {arguments:?} failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )
                .into());
            }
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        }

        fn write(&self, path: &str, text: &str) -> Result<(), Box<dyn Error>> {
            let file = self.path.join(path);
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(file, text)?;
            Ok(())
        }

        fn commit(&self, message: &str) -> Result<String, Box<dyn Error>> {
            self.git(&["add", "--all"])?;
            self.git(&["commit", "--quiet", "--message", message])?;
            self.git(&["rev-parse", "HEAD"])
        }

        fn base(&self, version: &str) -> Result<(), Box<dyn Error>> {
            self.write(
                "Cargo.toml",
                &format!("[workspace.package]\nversion = \"{version}\"\n"),
            )?;
            self.write(
                "npm/vsift-cli/package.json",
                &format!("{{\"version\": \"{version}\"}}\n"),
            )?;
            self.write("npm/vsift-cli/README.md", "install vsift-cli@next\n")?;
            self.write("crates/vsift-cli/src/main.rs", "fn main() {}\n")?;
            Ok(())
        }

        fn bump(&self, version: &str) -> Result<(), Box<dyn Error>> {
            self.write(
                "Cargo.toml",
                &format!("[workspace.package]\nversion = \"{version}\"\n"),
            )?;
            self.write(
                "npm/vsift-cli/package.json",
                &format!("{{\"version\": \"{version}\"}}\n"),
            )
        }
    }

    impl Drop for Repository {
        fn drop(&mut self) {
            let _ignored = fs::remove_dir_all(&self.path);
        }
    }

    fn checked(
        observation: CandidateObservation,
    ) -> Result<Box<super::CandidateReport>, Box<dyn Error>> {
        match observation {
            CandidateObservation::Checked(report) => Ok(report),
            other => Err(format!("not checked: {other:?}").into()),
        }
    }

    #[test]
    fn a_stable_that_adds_only_versions_and_the_readme_to_the_candidate_passes()
    -> Result<(), Box<dyn Error>> {
        let repository = Repository::new("clean")?;
        repository.base(CANDIDATE)?;
        let candidate = repository.commit("candidate")?;
        repository.git(&["tag", "v0.2.0-rc.1"])?;
        repository.bump(STABLE)?;
        repository.write("npm/vsift-cli/README.md", "install vsift-cli\n")?;
        let stable = repository.commit("stable")?;

        let version = ReleaseVersion::parse(STABLE)?;
        let report = checked(observe(&repository.path, &stable, &version))?;
        assert_eq!(report.violations(), Vec::<String>::new(), "{report:#?}");
        assert_eq!(report.candidate_tag, "v0.2.0-rc.1");
        assert_eq!(report.candidate_commit, candidate);
        assert!(report.ancestor);
        let paths: Vec<&str> = report
            .changes
            .iter()
            .map(|changed| changed.path.as_str())
            .collect();
        assert_eq!(
            paths,
            [
                "Cargo.toml",
                "npm/vsift-cli/README.md",
                "npm/vsift-cli/package.json"
            ]
        );
        assert!(
            report
                .summary()
                .contains("2 version-string and 1 shipped-document")
        );
        assert!(report.markdown().contains("limited to version strings"));
        Ok(())
    }

    #[test]
    fn a_stable_with_any_other_difference_is_refused() -> Result<(), Box<dyn Error>> {
        let version = ReleaseVersion::parse(STABLE)?;
        // A source file, a new file, a deleted allowed file and an edit beyond the
        // version in a manifest, each on its own commit.
        for (name, edit) in [
            ("source", 0),
            ("added", 1),
            ("deleted", 2),
            ("manifest", 3),
            ("workflow", 4),
        ] {
            let repository = Repository::new(&format!("refused-{name}"))?;
            repository.base(CANDIDATE)?;
            repository.commit("candidate")?;
            repository.git(&["tag", "v0.2.0-rc.1"])?;
            repository.bump(STABLE)?;
            match edit {
                0 => repository.write("crates/vsift-cli/src/main.rs", "fn main() { fix(); }\n")?,
                1 => repository.write("docs/new.md", "x\n")?,
                2 => fs::remove_file(repository.path.join("npm/vsift-cli/README.md"))?,
                3 => repository.write(
                    "Cargo.toml",
                    "[workspace.package]\nversion = \"0.2.0\"\nedition = \"2024\"\n",
                )?,
                _ => repository.write(".github/workflows/release.yml", "name: Release\n")?,
            }
            let stable = repository.commit("stable")?;
            let report = checked(observe(&repository.path, &stable, &version))?;
            assert!(!report.violations().is_empty(), "{name}: {report:#?}");
        }
        Ok(())
    }

    #[test]
    fn the_highest_candidate_is_the_accepted_one() -> Result<(), Box<dyn Error>> {
        let repository = Repository::new("two-candidates")?;
        repository.base("0.2.0-rc.1")?;
        repository.commit("candidate one")?;
        repository.git(&["tag", "v0.2.0-rc.1"])?;
        repository.write("crates/vsift-cli/src/main.rs", "fn main() { fix(); }\n")?;
        repository.base("0.2.0-rc.2")?;
        repository.write("crates/vsift-cli/src/main.rs", "fn main() { fix(); }\n")?;
        repository.commit("candidate two")?;
        repository.git(&["tag", "v0.2.0-rc.2"])?;
        repository.bump(STABLE)?;
        let stable = repository.commit("stable")?;
        let version = ReleaseVersion::parse(STABLE)?;
        let report = checked(observe(&repository.path, &stable, &version))?;
        assert_eq!(report.candidate_tag, "v0.2.0-rc.2");
        assert_eq!(report.violations(), Vec::<String>::new(), "{report:#?}");
        Ok(())
    }

    #[test]
    fn a_candidate_that_is_not_an_ancestor_is_refused() -> Result<(), Box<dyn Error>> {
        let repository = Repository::new("not-ancestor")?;
        repository.base("0.1.0")?;
        repository.commit("base")?;
        repository.git(&["switch", "--quiet", "--create", "other"])?;
        repository.bump(CANDIDATE)?;
        repository.commit("candidate on another branch")?;
        repository.git(&["tag", "v0.2.0-rc.1"])?;
        repository.git(&["switch", "--quiet", "main"])?;
        repository.bump(STABLE)?;
        let stable = repository.commit("stable on main")?;
        let version = ReleaseVersion::parse(STABLE)?;
        let report = checked(observe(&repository.path, &stable, &version))?;
        assert!(!report.ancestor);
        assert!(
            report
                .violations()
                .iter()
                .any(|violation| violation.contains("not an ancestor")),
            "{report:#?}"
        );
        Ok(())
    }

    #[test]
    fn the_stable_commit_being_the_candidate_is_refused() -> Result<(), Box<dyn Error>> {
        let repository = Repository::new("same-commit")?;
        repository.base(CANDIDATE)?;
        let candidate = repository.commit("candidate")?;
        repository.git(&["tag", "v0.2.0-rc.1"])?;
        let version = ReleaseVersion::parse(STABLE)?;
        let report = checked(observe(&repository.path, &candidate, &version))?;
        assert!(
            report
                .violations()
                .iter()
                .any(|violation| violation.contains("is the candidate's commit")),
            "{report:#?}"
        );
        Ok(())
    }

    #[test]
    fn no_candidate_a_missing_commit_and_a_pre_release_are_each_answered()
    -> Result<(), Box<dyn Error>> {
        let repository = Repository::new("none")?;
        repository.base(STABLE)?;
        let commit = repository.commit("a stable with no candidate")?;
        let stable = ReleaseVersion::parse(STABLE)?;
        assert!(matches!(
            observe(&repository.path, &commit, &stable),
            CandidateObservation::Failed(reason) if reason.contains("no release candidate tag")
        ));
        // A tag that is not this version's candidate does not count.
        repository.git(&["tag", "v0.2.0-beta.1"])?;
        repository.git(&["tag", "v0.2.1-rc.1"])?;
        assert!(matches!(
            observe(&repository.path, &commit, &stable),
            CandidateObservation::Failed(_)
        ));
        // A commit this clone does not have (a shallow checkout).
        let absent = "1".repeat(40);
        assert!(matches!(
            observe(&repository.path, &absent, &stable),
            CandidateObservation::Failed(reason) if reason.contains("full history")
        ));
        // Not a commit SHA at all.
        assert!(matches!(
            observe(&repository.path, "main", &stable),
            CandidateObservation::Failed(_)
        ));
        // A pre-release has no candidate to compare with.
        let pre = ReleaseVersion::parse(CANDIDATE)?;
        assert_eq!(
            observe(&repository.path, &commit, &pre),
            CandidateObservation::NotApplicable
        );
        // A directory that is not a repository is a failure, not a crash.
        let elsewhere = std::env::temp_dir().join(format!(
            "vsift-release-not-a-repository-{}",
            std::process::id()
        ));
        fs::create_dir_all(&elsewhere)?;
        let result = observe(Path::new(&elsewhere), &commit, &stable);
        fs::remove_dir_all(&elsewhere)?;
        assert!(
            matches!(result, CandidateObservation::Failed(_)),
            "{result:?}"
        );
        Ok(())
    }
}
