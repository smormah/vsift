//! Computes the text `vsift --version` prints (P13 PR 8, ADR 0023).
//!
//! The release workflow sets `VSIFT_SOURCE_COMMIT` to the commit it builds;
//! the version line then ends with that commit, so a binary maps to its
//! protected source commit on its own. A malformed value fails the build
//! instead of shipping a binary that names no commit or a wrong one.

use std::{env, process::ExitCode};

#[path = "src/build_identity.rs"]
mod build_identity;

fn main() -> ExitCode {
    println!(
        "cargo::rerun-if-env-changed={}",
        build_identity::SOURCE_COMMIT_VARIABLE
    );
    let source_commit = env::var_os(build_identity::SOURCE_COMMIT_VARIABLE);
    let source_commit = match source_commit.as_deref().map(|value| value.to_str()) {
        None => None,
        Some(Some(commit)) => Some(commit),
        Some(None) => {
            println!("cargo::error={}", build_identity::InvalidSourceCommit);
            return ExitCode::FAILURE;
        }
    };
    match build_identity::version_text(env!("CARGO_PKG_VERSION"), source_commit) {
        Ok(text) => {
            println!("cargo::rustc-env=VSIFT_VERSION_TEXT={text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!("cargo::error={error}");
            ExitCode::FAILURE
        }
    }
}
