//! `vsift-ocr-runner`: reads text off `VSift`'s evidence frames (ADR 0030).
//!
//! The command line and its records are [`vsift_ocr_runner::cli`]'s. The
//! runtime and the models are found beside this executable, so nothing is
//! read from the environment.

#![forbid(unsafe_code)]

use std::{
    ffi::OsString,
    io::{self, Write},
    process::ExitCode,
};

use vsift_ocr_runner::{
    RunnerFailure,
    cli::{Layout, run},
    onnx::OnnxRuntime,
};

fn main() -> ExitCode {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    let Some(folder) = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(std::path::Path::to_path_buf))
    else {
        return ExitCode::from(RunnerFailure::RuntimeUnavailable.exit_status());
    };
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let status = run(
        &arguments,
        &Layout::beside(&folder),
        &OnnxRuntime,
        &mut stdout,
    );
    if stdout.flush().is_err() && status == 0 {
        return ExitCode::from(RunnerFailure::OutputFailed.exit_status());
    }
    ExitCode::from(status)
}
