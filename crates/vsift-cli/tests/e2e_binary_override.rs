//! The binary override of the opt-in real-tool checkpoints (P14 PR 3, RQ-05).
//!
//! These tests need no media tool. They check the rules of
//! `tests/published_binary/mod.rs`: with the variable unset the Cargo-built
//! binary runs, and an override is refused, never ignored, unless it is an
//! absolute path to a file that names the version and the commit it is expected
//! to be. The binary's `--version` is answered by the test where the real one
//! cannot be (a binary that prints a commit exists only in a release build).

mod published_binary;

use std::{env, error::Error, ffi::OsString, path::Path};

use published_binary::{
    BINARY_VARIABLE, BinaryError, COMMIT_VARIABLE, Expectation, Selected, VERSION_VARIABLE,
    VersionLine, check_version_line, parse_version_output, read_override, select,
};

type TestResult = Result<(), Box<dyn Error>>;

const COMMIT: &str = "011bc4da1af6c7b0d4f3e2a1908877665544332f";
const SHOWN: &str = "011bc4da1af6";

/// An environment holding exactly `entries`.
fn environment(entries: &[(&'static str, OsString)]) -> impl Fn(&str) -> Option<OsString> + use<> {
    let entries: Vec<(&'static str, OsString)> = entries.to_vec();
    move |name| {
        entries
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, value)| value.clone())
    }
}

/// A file that exists and is not a directory: this test's own executable.
fn existing_file() -> Result<OsString, Box<dyn Error>> {
    Ok(env::current_exe()?.into_os_string())
}

/// An override with both expectations set, for the version and commit above.
fn complete(path: OsString) -> Vec<(&'static str, OsString)> {
    vec![
        (BINARY_VARIABLE, path),
        (VERSION_VARIABLE, OsString::from("0.1.0")),
        (COMMIT_VARIABLE, OsString::from(COMMIT)),
    ]
}

/// A `--version` run that must never happen.
fn never_run(_: &Path) -> Result<String, BinaryError> {
    Err(BinaryError::VersionCommandFailed(String::from(
        "the binary was run",
    )))
}

fn prints(output: &'static str) -> impl Fn(&Path) -> Result<String, BinaryError> {
    move |_| Ok(output.to_owned())
}

#[test]
fn without_the_variable_the_cargo_built_binary_runs_and_nothing_is_run() {
    assert_eq!(
        select(&environment(&[]), &never_run),
        Ok(Selected::CargoBuilt)
    );
    assert_eq!(read_override(&environment(&[])), Ok(None));
}

#[test]
fn an_empty_variable_is_refused() {
    assert_eq!(
        select(
            &environment(&[(BINARY_VARIABLE, OsString::new())]),
            &never_run
        ),
        Err(BinaryError::Empty {
            variable: BINARY_VARIABLE
        })
    );
}

#[test]
fn a_relative_path_is_refused_before_anything_runs() {
    for relative in ["vsift", "./vsift", "..\\vsift.exe", "target/release/vsift"] {
        let result = select(
            &environment(&complete(OsString::from(relative))),
            &never_run,
        );
        assert!(
            matches!(result, Err(BinaryError::RelativePath(_))),
            "{relative}: {result:?}"
        );
    }
}

#[test]
fn a_path_that_names_no_file_is_refused() {
    let missing = env::temp_dir().join("vsift-p14-no-such-binary-6f2d1c");
    let result = select(
        &environment(&complete(missing.into_os_string())),
        &never_run,
    );
    assert!(matches!(result, Err(BinaryError::Missing(_))), "{result:?}");
}

#[test]
fn a_directory_is_refused() {
    let result = select(
        &environment(&complete(env::temp_dir().into_os_string())),
        &never_run,
    );
    assert!(
        matches!(result, Err(BinaryError::NotAFile(_))),
        "{result:?}"
    );
}

#[test]
fn an_override_without_its_expectation_is_refused() -> TestResult {
    let file = existing_file()?;
    let only_path = environment(&[(BINARY_VARIABLE, file.clone())]);
    assert_eq!(
        select(&only_path, &never_run),
        Err(BinaryError::ExpectationNotSet {
            variable: VERSION_VARIABLE
        })
    );
    let no_commit = environment(&[
        (BINARY_VARIABLE, file),
        (VERSION_VARIABLE, OsString::from("0.1.0")),
    ]);
    assert_eq!(
        select(&no_commit, &never_run),
        Err(BinaryError::ExpectationNotSet {
            variable: COMMIT_VARIABLE
        })
    );
    Ok(())
}

#[test]
fn a_malformed_expectation_is_refused() -> TestResult {
    let file = existing_file()?;
    for version in ["v0.1.0", "0.1", "0.1.0 ", "latest", "0.1.0-", "1.2.3.4", ""] {
        let lookup = environment(&[
            (BINARY_VARIABLE, file.clone()),
            (VERSION_VARIABLE, OsString::from(version)),
            (COMMIT_VARIABLE, OsString::from(COMMIT)),
        ]);
        assert!(
            read_override(&lookup).is_err(),
            "version {version:?} was accepted"
        );
    }
    let too_long = format!("{COMMIT}0");
    for commit in [
        "abc",
        "011BC4DA1AF6",
        "011bc4da1af",
        "011bc4da1af6zz",
        &too_long,
    ] {
        let lookup = environment(&[
            (BINARY_VARIABLE, file.clone()),
            (VERSION_VARIABLE, OsString::from("0.1.0")),
            (COMMIT_VARIABLE, OsString::from(commit)),
        ]);
        assert!(
            read_override(&lookup).is_err(),
            "commit {commit:?} was accepted"
        );
    }
    Ok(())
}

#[test]
fn a_binary_that_names_the_expected_version_and_commit_is_accepted() -> TestResult {
    let lookup = environment(&complete(existing_file()?));
    let selected = select(&lookup, &prints("vsift 0.1.0 (011bc4da1af6)\n"))?;
    let Selected::Override {
        version_line,
        sha256,
        ..
    } = selected
    else {
        return Err("the override was not selected".into());
    };
    assert_eq!(version_line, "vsift 0.1.0 (011bc4da1af6)");
    assert_eq!(sha256.len(), 64);
    // A shorter expectation is accepted too: twelve digits is what a build shows.
    let short = environment(&[
        (BINARY_VARIABLE, existing_file()?),
        (VERSION_VARIABLE, OsString::from("0.1.0")),
        (COMMIT_VARIABLE, OsString::from(SHOWN)),
    ]);
    assert!(select(&short, &prints("vsift 0.1.0 (011bc4da1af6)\r\n")).is_ok());
    Ok(())
}

#[test]
fn another_version_is_refused() -> TestResult {
    let lookup = environment(&complete(existing_file()?));
    assert_eq!(
        select(&lookup, &prints("vsift 0.2.0 (011bc4da1af6)\n")),
        Err(BinaryError::VersionMismatch {
            expected: String::from("0.1.0"),
            found: String::from("0.2.0"),
        })
    );
    assert!(matches!(
        select(&lookup, &prints("vsift 0.1.0-rc.1 (011bc4da1af6)\n")),
        Err(BinaryError::VersionMismatch { .. })
    ));
    Ok(())
}

#[test]
fn another_commit_is_refused() -> TestResult {
    let lookup = environment(&complete(existing_file()?));
    let result = select(&lookup, &prints("vsift 0.1.0 (011bc4da1af7)\n"));
    assert!(
        matches!(result, Err(BinaryError::CommitMismatch { .. })),
        "{result:?}"
    );
    Ok(())
}

#[test]
fn a_binary_that_names_no_commit_is_refused() -> TestResult {
    let lookup = environment(&complete(existing_file()?));
    let result = select(&lookup, &prints("vsift 0.1.0\n"));
    assert!(
        matches!(result, Err(BinaryError::NoCommitInVersion(_))),
        "{result:?}"
    );
    Ok(())
}

#[test]
fn text_that_is_not_a_version_line_is_refused() -> TestResult {
    let lookup = environment(&complete(existing_file()?));
    for output in [
        "",
        "hello\n",
        "vsift\n",
        "vsift 0.1.0 (011bc4da1af6) extra\n",
        "vsift 0.1.0 (XYZ)\n",
        "vsift 0.1.0 (011bc4da1af6\n",
        "vsift 0.1.0 (011bc4da1af6)\nsecond line\n",
        "another 0.1.0 (011bc4da1af6)\n",
    ] {
        let result = select(&lookup, &prints(output));
        assert!(
            matches!(result, Err(BinaryError::VersionOutputMalformed(_))),
            "{output:?}: {result:?}"
        );
    }
    Ok(())
}

#[test]
fn a_version_run_that_fails_refuses_the_override() -> TestResult {
    let lookup = environment(&complete(existing_file()?));
    assert_eq!(
        select(&lookup, &never_run),
        Err(BinaryError::VersionCommandFailed(String::from(
            "the binary was run"
        )))
    );
    Ok(())
}

#[test]
fn both_forms_of_the_version_line_parse() -> TestResult {
    assert_eq!(
        parse_version_output("vsift 0.1.0\n")?,
        VersionLine {
            version: String::from("0.1.0"),
            commit: None
        }
    );
    assert_eq!(
        parse_version_output("vsift 0.2.0-rc.1 (011bc4da1af6)\r\n")?,
        VersionLine {
            version: String::from("0.2.0-rc.1"),
            commit: Some(String::from(SHOWN))
        }
    );
    let expectation = Expectation {
        version: String::from("0.2.0-rc.1"),
        commit: String::from(COMMIT),
    };
    assert!(check_version_line("vsift 0.2.0-rc.1 (011bc4da1af6)\n", &expectation).is_ok());
    Ok(())
}

/// The real `vsift --version`, run through the module's own runner: the
/// Cargo-built binary prints the crate version (and a commit only when
/// `VSIFT_SOURCE_COMMIT` was set for the build).
#[test]
fn the_real_runner_reads_the_cargo_built_binary() -> TestResult {
    let binary = assert_cmd::cargo::cargo_bin("vsift");
    let output = published_binary::run_version(&binary)?;
    let line = parse_version_output(&output)?;
    assert_eq!(line.version, env!("CARGO_PKG_VERSION"));
    Ok(())
}

/// A Cargo-built binary offered as an override is judged by the same rules: it
/// names no commit unless it was built with one, and an unrelated commit does
/// not match either way.
#[test]
fn the_cargo_built_binary_is_refused_as_an_override_with_an_unrelated_commit() {
    let binary = assert_cmd::cargo::cargo_bin("vsift").into_os_string();
    let lookup = environment(&[
        (BINARY_VARIABLE, binary),
        (VERSION_VARIABLE, OsString::from(env!("CARGO_PKG_VERSION"))),
        (COMMIT_VARIABLE, OsString::from("0123456789ab")),
    ]);
    let result = select(&lookup, &published_binary::run_version);
    assert!(
        matches!(
            result,
            Err(BinaryError::NoCommitInVersion(_) | BinaryError::CommitMismatch { .. })
        ),
        "{result:?}"
    );
}

/// The default path every checkpoint takes when the override is not set.
#[test]
fn the_command_builder_runs_the_cargo_built_binary_by_default() -> TestResult {
    if env::var_os(BINARY_VARIABLE).is_some() {
        println!("skipped: {BINARY_VARIABLE} is set, so the builder runs the override");
        return Ok(());
    }
    let output = published_binary::command()?.arg("--version").output()?;
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)?.starts_with("vsift "));
    assert!(!published_binary::is_override()?);
    assert_eq!(published_binary::report()?["source"], "cargo_built");
    Ok(())
}
