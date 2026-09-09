//! Public command-line contract tests.

use assert_cmd::Command;
use predicates::{boolean::PredicateBooleanExt, prelude::predicate};

#[test]
fn help_identifies_the_product() -> Result<(), Box<dyn std::error::Error>> {
    let mut command = Command::cargo_bin("vsift")?;

    command
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Sift technical video"));

    Ok(())
}

#[test]
fn doctor_json_uses_the_versioned_contract() -> Result<(), Box<dyn std::error::Error>> {
    let mut command = Command::cargo_bin("vsift")?;

    command
        .args(["doctor", "--json", "--timeout-seconds", "1"])
        .assert()
        .code(predicate::eq(0).or(predicate::eq(2)))
        .stdout(predicate::str::contains("\"schema_version\": \"1\""))
        .stdout(predicate::str::contains("\"command\": \"doctor\""));

    Ok(())
}
