//! C-01: the typed remediation of a rejected command line (L-071, P13 PR 1).
//!
//! In `--json` and `--events jsonl` modes a command line the parser rejects
//! answers `INVALID_ARGUMENT` with `command` `parse` and one remediation: a
//! fixed-prose summary carrying the rejection's identifier, and the help of
//! the deepest command reached as its suggested `command`. The summary names
//! only the grammar's own subcommands and arguments; the text the user
//! supplied is never repeated in a machine result. Human mode keeps the
//! parser's explanation on stderr, with control characters replaced and
//! hidden characters shown as `<U+XXXX>`.
//!
//! A parse failure happens before the engine is composed, so none of these
//! commands reads configuration or touches a session root.

use std::{error::Error, ffi::OsString, process::Output};

use assert_cmd::Command;
use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;

const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const EVIDENCE: &str = "evd_0123456789abcdef0123456789abcdef";
const CANDIDATE: &str = "vcd_0123456789abcdef0123456789abcdef";

/// The PowerShell note every remediation of `crop` ends with.
const POWERSHELL_NOTE: &str = " On PowerShell, quote the value of `--rect` (for example \
     '10,20,300,80'): unquoted, PowerShell splits it at the commas into several arguments.";

/// A unique marker in argument text that must never reach a machine result:
/// a right-to-left override, a zero-width space, an escape sequence and a
/// line break around ASCII words that appear nowhere in `VSift`'s own text.
const SENTINEL: &str = "QXSENTINEL\u{202E}ZWREVERSED\u{200B}JOINED\u{1b}[31mESCAPED\nINJECTED";

/// The ASCII words of [`SENTINEL`]; any of them in stdout is an echo.
const SENTINEL_WORDS: [&str; 5] = ["QXSENTINEL", "ZWREVERSED", "JOINED", "ESCAPED", "INJECTED"];

fn run<Argument>(arguments: &[Argument]) -> Result<Output, Box<dyn Error>>
where
    Argument: AsRef<std::ffi::OsStr>,
{
    Ok(Command::cargo_bin("vsift")?.args(arguments).output()?)
}

/// The single failure result of a `--json` run, checked for the parse
/// envelope: exit 2, `parse`, `INVALID_ARGUMENT`, one line, silent stderr.
fn parse_result(output: &Output) -> Result<Value, Box<dyn Error>> {
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty(), "stderr: {:?}", output.stderr);
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    assert!(!output.stdout[..output.stdout.len() - 1].contains(&b'\n'));
    let value: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(value["command"], "parse");
    assert_eq!(value["status"], "failed");
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(
        value["error"]["message"],
        "The command line arguments are invalid."
    );
    Ok(value)
}

/// The one remediation of a parse result: its summary and suggested
/// arguments, after checking it needs no authority and names `vsift`.
fn remediation(value: &Value) -> Result<(String, Vec<String>), Box<dyn Error>> {
    let items = value["error"]["remediation"]
        .as_array()
        .ok_or("remediation is not an array")?;
    assert_eq!(items.len(), 1, "{items:?}");
    let item = &items[0];
    assert_eq!(item["required_authority"], "none");
    assert_eq!(item["command"]["executable"], "vsift");
    let summary = item["summary"].as_str().ok_or("summary")?.to_owned();
    let arguments = item["command"]["arguments"]
        .as_array()
        .ok_or("arguments")?
        .iter()
        .map(|argument| argument.as_str().map(str::to_owned).ok_or("argument"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((summary, arguments))
}

fn expected_summary(reason: &str, problem: &str, powershell: bool) -> String {
    let note = if powershell { POWERSHELL_NOTE } else { "" };
    format!("The command line was rejected ({reason}). {problem}; read its help.{note}")
}

/// A rejected command line (after `--json`), its reason, the problem the
/// summary states, whether the PowerShell note follows, and the help.
type Case = (
    &'static [&'static str],
    &'static str,
    &'static str,
    bool,
    &'static [&'static str],
);

/// One command line per rejection (more where the summary has several
/// forms), with the exact summary and help. `invalid_utf8` is tested apart,
/// because its argument is not a `str`.
const CASES: [Case; 12] = [
    (
        &["crop", SESSION, EVIDENCE, "--rect", "10", "20", "300", "80"],
        "unknown_argument",
        "`crop` does not take one of the arguments given",
        true,
        &["crop", "--help"],
    ),
    (
        &["crop", SESSION, EVIDENCE],
        "missing_required",
        "`crop` needs `--rect`",
        true,
        &["crop", "--help"],
    ),
    (
        &["frame", "get", SESSION],
        "missing_required",
        "`frame get` is missing a required argument",
        false,
        &["frame", "get", "--help"],
    ),
    (
        &["crop", SESSION, EVIDENCE, "--rect", "10"],
        "invalid_value",
        "The value of `--rect` is missing or not valid for `crop`",
        true,
        &["crop", "--help"],
    ),
    (
        &["session", "status", "ses_short"],
        "invalid_value",
        "The value of `<SESSION>` is missing or not valid for `session status`",
        false,
        &["session", "status", "--help"],
    ),
    (
        &["session", "clean", "--dry-run=yes"],
        "unexpected_value",
        "`--dry-run` was given a value it does not take in `session clean`",
        false,
        &["session", "clean", "--help"],
    ),
    (
        &[
            "frame",
            "get",
            SESSION,
            "--at",
            "1",
            "--candidate",
            CANDIDATE,
        ],
        "argument_conflict",
        "`--at` cannot be used with `--candidate` in `frame get`",
        false,
        &["frame", "get", "--help"],
    ),
    (
        &[
            "transcript",
            "get",
            SESSION,
            "--from",
            "1",
            "--from",
            "2",
            "--to",
            "3",
        ],
        "argument_conflict",
        "`--from` was given more than once, or with an argument it excludes, in `transcript get`",
        false,
        &["transcript", "get", "--help"],
    ),
    (
        &["--events", "jsonl", "session", "list"],
        "argument_conflict",
        "`--json` cannot be used with `--events` in `session list`",
        false,
        &["session", "list", "--help"],
    ),
    (
        &["session"],
        "missing_subcommand",
        "`session` needs one of its commands",
        false,
        &["session", "--help"],
    ),
    (
        &["session", "frob"],
        "unknown_subcommand",
        "`session` has no command by the name given",
        false,
        &["session", "--help"],
    ),
    (
        &["frob"],
        "unknown_subcommand",
        "`vsift` has no command by the name given",
        false,
        &["--help"],
    ),
];

#[test]
fn every_rejection_has_its_fixed_remediation_and_help() -> TestResult {
    for (words, reason, problem, powershell, help) in CASES {
        let mut line = vec!["--json"];
        line.extend_from_slice(words);
        let value = parse_result(&run(&line)?)?;
        let (summary, arguments) = remediation(&value)?;
        assert_eq!(
            summary,
            expected_summary(reason, problem, powershell),
            "{words:?}"
        );
        assert_eq!(arguments, help, "{words:?}");
    }
    Ok(())
}

/// The same remediation arrives as the terminal event of `--events jsonl`.
#[test]
fn json_lines_mode_ends_with_the_same_remediation() -> TestResult {
    let output = run(&[
        "--events", "jsonl", "crop", SESSION, EVIDENCE, "--rect", "10",
    ])?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout)?;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let event: Value = serde_json::from_str(lines[0])?;
    assert_eq!(event["event"], "terminal");
    assert_eq!(event["sequence"], 0);
    let result = &event["result"];
    assert_eq!(result["command"], "parse");
    let (summary, arguments) = remediation(result)?;
    assert_eq!(
        summary,
        expected_summary(
            "invalid_value",
            "The value of `--rect` is missing or not valid for `crop`",
            true
        )
    );
    assert_eq!(arguments, ["crop", "--help"]);
    Ok(())
}

/// Text that is not valid Unicode where a command takes text (`--query`).
#[test]
fn an_argument_that_is_not_unicode_is_invalid_utf8() -> TestResult {
    let mut line: Vec<OsString> = ["--json", "search", SESSION, "--query"]
        .into_iter()
        .map(OsString::from)
        .collect();
    line.push(not_unicode());
    let value = parse_result(&run(&line)?)?;
    let (summary, arguments) = remediation(&value)?;
    assert_eq!(
        summary,
        expected_summary(
            "invalid_utf8",
            "An argument given to `search` is not valid Unicode text",
            false
        )
    );
    assert_eq!(arguments, ["search", "--help"]);
    Ok(())
}

#[cfg(windows)]
fn not_unicode() -> OsString {
    use std::os::windows::ffi::OsStringExt;
    // An unpaired surrogate: valid on a Windows command line, not UTF-8.
    OsString::from_wide(&[0x0071, 0xD800, 0x0071])
}

#[cfg(unix)]
fn not_unicode() -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(vec![b'q', 0xFF, b'q'])
}

/// Every place argument text can sit in a rejected command line.
fn hostile_lines() -> Vec<Vec<String>> {
    let sentinel = SENTINEL.to_owned();
    let flag = format!("--{SENTINEL}");
    let words = |items: &[&str]| items.iter().map(|item| (*item).to_owned()).collect();
    vec![
        // An unknown flag.
        words(&["crop", SESSION, EVIDENCE, "--rect", "1,2,3,4", &flag]),
        // An unknown flag given a value.
        words(&[
            "crop",
            SESSION,
            EVIDENCE,
            "--rect",
            "1,2,3,4",
            &format!("{flag}={SENTINEL}"),
        ]),
        // An invalid value of a defined argument.
        words(&["crop", SESSION, EVIDENCE, "--rect", &sentinel]),
        // An invalid identity.
        words(&["session", "status", &sentinel]),
        // A value given to a flag that takes none.
        words(&["session", "clean", &format!("--dry-run={SENTINEL}")]),
        // An unknown command and an unknown operation.
        words(&[&sentinel]),
        words(&["session", &sentinel]),
        // One argument too many.
        words(&["session", "status", SESSION, &sentinel]),
    ]
}

fn assert_no_echo(stdout: &[u8], context: &str) -> TestResult {
    let text = String::from_utf8(stdout.to_vec())?;
    for word in SENTINEL_WORDS {
        assert!(!text.contains(word), "{context}: {word} echoed in {text}");
    }
    for hidden in ['\u{202E}', '\u{200B}', '\u{1b}'] {
        assert!(!text.contains(hidden), "{context}: hidden character echoed");
    }
    assert!(!text.contains("\\u001b"), "{context}: escape echoed");
    Ok(())
}

/// SEC-T02 for the parser: hostile argument text never reaches stdout in
/// the machine modes, wherever it sits in the command line.
#[test]
fn hostile_argument_text_never_reaches_a_machine_result() -> TestResult {
    for words in hostile_lines() {
        for mode in [vec!["--json"], vec!["--events", "jsonl"]] {
            let mut line: Vec<String> = mode.iter().map(|item| (*item).to_owned()).collect();
            line.extend(words.iter().cloned());
            let output = run(&line)?;
            let context = format!("{:?} {mode:?}", words.first());
            assert_eq!(output.status.code(), Some(2), "{context}");
            assert!(output.stderr.is_empty(), "{context}");
            assert_no_echo(&output.stdout, &context)?;
            let text = String::from_utf8(output.stdout)?;
            let value: Value = serde_json::from_str(text.trim_end())?;
            let result = if mode.len() == 1 {
                value
            } else {
                value["result"].clone()
            };
            assert_eq!(result["command"], "parse", "{context}");
            let (summary, _) = remediation(&result)?;
            assert!(
                summary.starts_with("The command line was rejected ("),
                "{context}: {summary}"
            );
        }
    }
    Ok(())
}

/// Human mode keeps the parser's explanation on stderr, which quotes the
/// argument; it is one line with controls replaced and hidden characters
/// shown as notation, and stdout stays empty.
#[test]
fn human_mode_shows_the_parser_detail_in_terminal_safe_form() -> TestResult {
    for words in hostile_lines() {
        let output = run(&words)?;
        let context = format!("{:?}", words.first());
        assert_eq!(output.status.code(), Some(2), "{context}");
        assert!(output.stdout.is_empty(), "{context}");
        let stderr = String::from_utf8(output.stderr)?;
        for raw in ['\u{202E}', '\u{200B}', '\u{1b}'] {
            assert!(!stderr.contains(raw), "{context}: raw character in stderr");
        }
        assert_eq!(stderr.matches('\n').count(), 1, "{context}: {stderr}");
        assert!(stderr.ends_with('\n'), "{context}");
        // Where the detail quotes the text, the hidden characters are
        // notation and the line break is U+FFFD. (The parser itself drops
        // the escape sequence; either way no escape reaches the terminal.)
        if stderr.contains("ZWREVERSED") {
            assert!(
                stderr.contains("QXSENTINEL<U+202E>ZWREVERSED<U+200B>JOINED"),
                "{context}: {stderr}"
            );
            assert!(
                stderr.contains("ESCAPED\u{fffd}INJECTED"),
                "{context}: {stderr}"
            );
        }
    }
    // The detail of an unknown flag names it, in terminal-safe form.
    let unknown = run(&[
        "crop",
        SESSION,
        EVIDENCE,
        "--rect",
        "1,2,3,4",
        &format!("--{SENTINEL}"),
    ])?;
    let stderr = String::from_utf8(unknown.stderr)?;
    assert!(
        stderr.contains("--QXSENTINEL<U+202E>ZWREVERSED<U+200B>JOINED"),
        "{stderr}"
    );
    Ok(())
}
