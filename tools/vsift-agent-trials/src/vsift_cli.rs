//! Runs the `vsift` binary for the harness itself (registering tools,
//! preparing sessions, validating and retaining bundles).
//!
//! Every invocation names the executable by absolute path, passes an
//! explicit argument list (never a shell), points `VSift` at the trial's
//! isolated per-user base and empties `PATH`, as the P06-P11 checkpoints
//! do, so a tool is found only where it was registered.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

use serde_json::Value;

use crate::{error::TrialError, layout::TrialLayout};

/// The `vsift` executable bound to one trial's per-user base.
#[derive(Clone, Debug)]
pub struct VsiftCli {
    executable: PathBuf,
    environment: Vec<(String, PathBuf)>,
    working_directory: PathBuf,
}

/// A finished `--json` invocation.
#[derive(Clone, Debug)]
pub struct JsonOutcome {
    /// The exit code.
    pub code: Option<i32>,
    /// The parsed result.
    pub value: Value,
}

impl VsiftCli {
    /// Binds `executable` (absolute) to the trial's per-user base.
    ///
    /// # Errors
    ///
    /// [`TrialError::Refused`] when the executable path is not absolute or
    /// not a file.
    pub fn new(executable: &Path, layout: &TrialLayout) -> Result<Self, TrialError> {
        if !executable.is_absolute() || !executable.is_file() {
            return Err(TrialError::Refused(
                "the vsift executable must be an absolute path to a file".to_owned(),
            ));
        }
        Ok(Self {
            executable: executable.to_path_buf(),
            environment: layout.user_environment(),
            working_directory: layout.workspace(),
        })
    }

    /// The executable.
    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    fn command(&self, arguments: &[OsString]) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .args(arguments)
            .current_dir(&self.working_directory)
            .env("PATH", "")
            .stdin(Stdio::null());
        for (name, value) in &self.environment {
            command.env(name, value);
        }
        command
    }

    /// Runs one `--json` command and parses its result.
    ///
    /// # Errors
    ///
    /// [`TrialError::Process`] when it cannot start or prints no JSON.
    pub fn json(&self, arguments: &[OsString]) -> Result<JsonOutcome, TrialError> {
        let output = self
            .command(arguments)
            .output()
            .map_err(|error| TrialError::Process(format!("vsift did not start: {error}")))?;
        let value = serde_json::from_slice(&output.stdout).map_err(|error| {
            TrialError::Process(format!(
                "vsift {:?} printed no JSON result: {error}",
                arguments.first()
            ))
        })?;
        Ok(JsonOutcome {
            code: output.status.code(),
            value,
        })
    }

    /// Runs one command and returns its exit code and standard output (for
    /// `--events jsonl`).
    ///
    /// # Errors
    ///
    /// [`TrialError::Process`] when it cannot start or its output is not
    /// UTF-8.
    pub fn stdout(&self, arguments: &[OsString]) -> Result<(Option<i32>, String), TrialError> {
        let output = self
            .command(arguments)
            .output()
            .map_err(|error| TrialError::Process(format!("vsift did not start: {error}")))?;
        let text = String::from_utf8(output.stdout)
            .map_err(|_| TrialError::Process("vsift printed invalid UTF-8".to_owned()))?;
        Ok((output.status.code(), text))
    }

    /// Runs one `--json` command that must succeed.
    ///
    /// # Errors
    ///
    /// [`TrialError::Process`] naming the failure code when it fails.
    pub fn ok(&self, arguments: &[OsString]) -> Result<Value, TrialError> {
        let outcome = self.json(arguments)?;
        if outcome.code == Some(0) {
            Ok(outcome.value)
        } else {
            Err(TrialError::Process(format!(
                "vsift {} failed: {} {}",
                arguments
                    .iter()
                    .take(2)
                    .map(|argument| argument.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(" "),
                outcome.value["error"]["code"],
                outcome.value["error"]["remediation"][0]["summary"]
            )))
        }
    }

    /// Starts a command in the background with its output discarded.
    ///
    /// # Errors
    ///
    /// [`TrialError::Process`] when it cannot start.
    pub fn spawn(&self, arguments: &[OsString]) -> Result<Child, TrialError> {
        self.command(arguments)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| TrialError::Process(format!("vsift did not start: {error}")))
    }
}

/// Builds an argument list from string and path pieces.
#[must_use]
pub fn arguments(pieces: &[&dyn AsRef<std::ffi::OsStr>]) -> Vec<OsString> {
    pieces
        .iter()
        .map(|piece| piece.as_ref().to_os_string())
        .collect()
}
