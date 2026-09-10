//! `VSift` executable entry point.

#![forbid(unsafe_code)]

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    vsift_cli::run().await
}
