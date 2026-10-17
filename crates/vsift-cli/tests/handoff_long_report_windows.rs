//! #366 on Windows: a report of 70,000 characters or more through each of the
//! agent skill's two draft forms of `vsift handoff check`, launched the way
//! Claude Code launches its two shell tools.
//!
//! - **PowerShell** (the here-string piped into the check): Claude Code hands
//!   PowerShell its script in an environment variable and runs it with
//!   `Invoke-Expression`, so the report never travels on the process command
//!   line. The whole report reaches the check, which since `0.2.2` reads up
//!   to 128 KiB, and the answer is the one `--file` gives for the same bytes.
//!   The same script put on the command line, as an older client did, cannot
//!   even start: Windows limits a command line to 32,767 characters.
//! - **Git Bash** (the quoted heredoc): Claude Code runs `bash -c` with the
//!   command inline, single-quoted for `eval`, so the report rides on the
//!   command line, and Git Bash cuts that line much sooner (at about 8,000
//!   characters, measured with Git for Windows 2.33). A whole long report
//!   cannot be checked there; the skill checks it in parts of at most about
//!   5,500 characters cut at line ends, then the block alone. This test runs
//!   that procedure and shows that the parts find every finding the whole
//!   report has, each at its own line.
//!
//! These tests start shells on purpose, because the forms under test are
//! shell text; the product never does. The shells are found at their usual
//! places (`pwsh` on `PATH`; Git Bash under `C:\Program Files\Git`, or where
//! `VSIFT_TEST_GIT_BASH` names). On a machine without one the test says so
//! and passes, except under CI, where a missing shell fails it.
#![cfg(windows)]

use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-handoff-windows-";
/// The skill's limit for one Git Bash part (`references/commands.md`,
/// "Checking the draft").
const GIT_BASH_PART: usize = 5_500;
/// The line of the long report that names a local path.
const PATH_LINE: usize = 600;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

/// A temporary directory this test created and alone removes.
struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Built<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self, child: &str) -> PathBuf {
        self.0.join(child)
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// A shell this machine lacks: a pass with a note, or under CI a failure.
fn missing(shell: &str) -> TestResult {
    if env::var_os("CI").is_some() {
        return Err(format!("{shell} is required on a Windows CI runner").into());
    }
    eprintln!("{shell} was not found; the #366 form test did not run");
    Ok(())
}

fn pwsh() -> Option<PathBuf> {
    let output = Command::new("where.exe").arg("pwsh.exe").output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    text.lines().next().map(|line| PathBuf::from(line.trim()))
}

fn git_bash() -> Option<PathBuf> {
    if let Some(named) = env::var_os("VSIFT_TEST_GIT_BASH") {
        return Some(PathBuf::from(named));
    }
    [
        r"C:\Program Files\Git\usr\bin\bash.exe",
        r"C:\Program Files\Git\bin\bash.exe",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
}

fn vsift() -> Built<String> {
    let path = env!("CARGO_BIN_EXE_vsift");
    if path.contains('\'') {
        return Err("the test binary's path holds an apostrophe".into());
    }
    Ok(path.to_owned())
}

/// A valid handoff with no session to resolve.
fn handoff() -> Value {
    json!({
        "handoff_version": "1", "status": "complete",
        "question": "What does the speaker name?",
        "capabilities": {"image_access": "unavailable"},
        "claims": [{"id": "c1", "section": "actual", "kind": "observed", "support": "supported",
            "certainty": "medium", "statement": "The speaker names dialog R-17.", "citations": ["e1"]}],
        "citations": [{"id": "e1", "type": "transcript_segment",
            "segment_id": "tsg_0123456789abcdef0123456789abcdef"}],
        "gaps": [{"kind": "image_access", "reason": "image_access_unavailable", "note": null}],
        "untrusted_instructions": [],
        "lifecycle": {"action": "left_open"}
    })
}

/// A report of at least 70,000 characters: prose with apostrophes, double
/// quotes, a backslash pair and text that is not ASCII (each a way a shell
/// form can change a report), optionally a local path at [`PATH_LINE`], then
/// the block. Returns the text above the block and the block.
fn long_report(with_path: bool) -> (String, String) {
    let mut text = String::from("## Actual\n\n");
    let mut line = 3;
    while text.chars().count() < 70_000 {
        if with_path && line == PATH_LINE {
            text.push_str("The log was copied to C:\\Users\\someone\\notes.txt by hand.\n");
        } else {
            text.push_str(&format!(
                "{line}. The dialog didn't close after Save; the user's count stayed at 7, \
                 \"as shown\" — résumé “quoted” and a\\\\b kept.\n"
            ));
        }
        line += 1;
    }
    text.push('\n');
    let block = format!("```vsift-handoff\n{}\n```\n", handoff());
    (text, block)
}

/// The check's `data`, from a result printed on standard output.
fn data(output: &Output) -> Built<Value> {
    let text = String::from_utf8(output.stdout.clone())?;
    let line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "no result; stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            )
        })?;
    let value: Value = serde_json::from_str(line)?;
    assert_eq!(value["status"], "complete", "{value}");
    Ok(value["data"].clone())
}

/// The check of `report` read with `--file`, the form no shell touches.
fn checked_from_file(root: &OwnedRoot, report: &str) -> Built<Value> {
    let file = root.path("draft.md");
    fs::write(&file, report)?;
    let output = Command::new(vsift()?)
        .args(["handoff", "check", "--json", "--file"])
        .arg(&file)
        .env("LOCALAPPDATA", root.path("user"))
        .output()?;
    data(&output)
}

/// The skill's PowerShell form: `@'` ... `'@ | vsift handoff check --json`.
fn powershell_form(report: &str) -> Built<String> {
    if report.lines().any(|line| line.starts_with("'@")) {
        return Err("the report would end the here-string early".into());
    }
    Ok(format!(
        "@'\n{report}'@ | & '{}' handoff check --json",
        vsift()?
    ))
}

/// PowerShell started as Claude Code's PowerShell tool starts it: the script
/// in an environment variable, run by `Invoke-Expression`.
fn powershell_through_the_environment(
    root: &OwnedRoot,
    pwsh: &Path,
    script: &str,
) -> Built<Output> {
    Ok(Command::new(pwsh)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$s = $env:VSIFT_TEST_SCRIPT; $env:VSIFT_TEST_SCRIPT = $null; Invoke-Expression -Command $s",
        ])
        .env("VSIFT_TEST_SCRIPT", script)
        .env("LOCALAPPDATA", root.path("user"))
        .output()?)
}

/// The skill's Git Bash form run as Claude Code's Bash tool runs a command:
/// `bash -c`, the command inline and single-quoted for `eval`.
fn git_bash_form(root: &OwnedRoot, bash: &Path, part: &str) -> std::io::Result<Output> {
    let command = format!(
        "'{}' handoff check --json <<'VSIFT_HANDOFF'\n{part}VSIFT_HANDOFF",
        env!("CARGO_BIN_EXE_vsift")
    );
    let quoted = format!("'{}'", command.replace('\'', "'\"'\"'"));
    Command::new(bash)
        .arg("-c")
        .arg(format!("eval {quoted} < /dev/null"))
        .env("LOCALAPPDATA", root.path("user"))
        .output()
}

/// The text above the block cut at line ends into parts of at most `limit`
/// characters, each with the report line it starts at.
fn parts(text: &str, limit: usize) -> Vec<(usize, String)> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut start = 1;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        if !current.is_empty() && current.chars().count() + line.chars().count() > limit {
            parts.push((start, std::mem::take(&mut current)));
            start = index + 1;
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        parts.push((start, current));
    }
    parts
}

/// The findings of a check as `(rule, line)`, the block's absence left out.
fn findings(data: &Value, first_line: usize) -> Built<Vec<(String, Option<usize>)>> {
    let mut found = Vec::new();
    for finding in data["errors"]
        .as_array()
        .ok_or("no errors")?
        .iter()
        .chain(data["warnings"].as_array().ok_or("no warnings")?)
    {
        let rule = finding["rule"].as_str().ok_or("no rule")?.to_owned();
        if rule == "handoff_block_missing" {
            continue;
        }
        let line = finding["line"]
            .as_u64()
            .map(|line| usize::try_from(line).map(|line| line + first_line - 1))
            .transpose()?;
        found.push((rule, line));
    }
    Ok(found)
}

#[test]
fn a_long_report_is_checked_whole_through_the_powershell_form() -> TestResult {
    let Some(pwsh) = pwsh() else {
        return missing("pwsh");
    };
    let root = OwnedRoot::new()?;
    for with_path in [false, true] {
        let (text, block) = long_report(with_path);
        let report = format!("{text}{block}");
        assert!(report.chars().count() >= 70_000);
        let script = powershell_form(&report)?;

        let output = powershell_through_the_environment(&root, &pwsh, &script)?;
        let through_powershell = data(&output)?;
        assert_eq!(through_powershell, checked_from_file(&root, &report)?);
        if with_path {
            assert_eq!(
                findings(&through_powershell, 1)?,
                [("local_path".to_owned(), Some(PATH_LINE))]
            );
        } else {
            assert_eq!(through_powershell["valid"], true, "{through_powershell}");
        }

        // The same script on the command line, as an older client passed it,
        // never starts: it is longer than Windows allows a command line.
        let inline = Command::new(&pwsh)
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output();
        assert!(inline.is_err(), "a 70,000-character command line started");
    }
    Ok(())
}

#[test]
fn a_long_report_is_checked_in_parts_through_the_git_bash_form() -> TestResult {
    let Some(bash) = git_bash() else {
        return missing("Git Bash");
    };
    let root = OwnedRoot::new()?;
    let (text, block) = long_report(true);
    let report = format!("{text}{block}");
    assert!(report.chars().count() >= 70_000);
    let whole = checked_from_file(&root, &report)?;
    assert_eq!(
        findings(&whole, 1)?,
        [("local_path".to_owned(), Some(PATH_LINE))]
    );

    // The whole report through the heredoc never reaches the check.
    let reached = git_bash_form(&root, &bash, &report)
        .ok()
        .and_then(|output| data(&output).ok());
    assert!(
        reached.is_none(),
        "a 70,000-character heredoc reached vsift"
    );

    // The skill's procedure: the text in parts, then the block alone.
    let mut found = Vec::new();
    let cut = parts(&text, GIT_BASH_PART);
    assert!(cut.len() >= 13, "{} parts", cut.len());
    for (first_line, part) in &cut {
        let checked = data(&git_bash_form(&root, &bash, part)?)?;
        assert_eq!(checked["errors"][0]["rule"], "handoff_block_missing");
        found.extend(findings(&checked, *first_line)?);
    }
    assert_eq!(found, findings(&whole, 1)?);
    let checked = data(&git_bash_form(&root, &bash, &block)?)?;
    assert_eq!(checked["valid"], true, "{checked}");
    Ok(())
}
