//! Which of npm's command shims the client's `vsift` calls went through.
//!
//! On Windows `npm install --global` writes three files beside each other:
//! `vsift` (a POSIX `sh` script, which Git Bash runs), `vsift.ps1`
//! (PowerShell) and `vsift.cmd` (`cmd.exe`). P14 PR 2 found that the `.cmd`
//! shim lets `cmd.exe` read a command line a second time: `%NAME%` is
//! expanded, quotes are dropped and an unquoted redirection runs as a command
//! (issue #257, known limit L-109). The other two shims passed every hostile
//! case.
//!
//! Three things keep the agent trials on the safe two. The harness never runs
//! a shim itself: it runs the package's native executable or `node` with the
//! launcher, as explicit programs and arguments. Claude Code's trial
//! settings allow only `Bash(vsift:*)`, which on Windows is Git Bash, and deny
//! every other tool under `dontAsk`. And every grade counts, from the
//! commands the client reported, how many `vsift` calls ran in which shell,
//! so a trial record says which shim the client used and a call through
//! `cmd.exe` is visible rather than assumed away.

use serde::{Deserialize, Serialize};

use crate::{
    shell::{Dialect, parse_script},
    trace::{CallKind, ToolCall},
};

/// How many `vsift` calls ran through each of npm's shims, as read from the
/// commands the client reported.
///
/// A call the client was refused is not counted: it never ran.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShimUse {
    /// Calls in a POSIX shell: on Windows the extensionless `sh` shim that
    /// Git Bash runs, on Linux npm's plain link to the launcher (no shim).
    pub posix: u32,
    /// Calls in PowerShell, which resolves `vsift` to the `vsift.ps1` shim.
    pub powershell: u32,
    /// Calls through `cmd.exe`, which resolves `vsift` to the `vsift.cmd`
    /// shim that re-reads arguments (L-109, #257).
    pub cmd: u32,
}

impl ShimUse {
    /// Counts the `vsift` calls among `calls` by the shell they ran in.
    #[must_use]
    pub fn from_calls(calls: &[ToolCall]) -> Self {
        let mut counted = Self::default();
        for call in calls.iter().filter(|call| !call.denied) {
            let CallKind::Shell { command } = &call.kind else {
                continue;
            };
            for shell in parse_script(command, Dialect::Posix).vsift_shells {
                let slot = match shell {
                    Dialect::Posix => &mut counted.posix,
                    Dialect::PowerShell => &mut counted.powershell,
                    Dialect::Cmd => &mut counted.cmd,
                };
                *slot = slot.saturating_add(1);
            }
        }
        counted
    }

    /// Whether no `vsift` call ran at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The note a grade carries when a call went through `cmd.exe`, or `None`.
    ///
    /// It is a note and not a failure: the permission rules already refuse
    /// that shell for Claude Code, and on Windows nothing else runs a client,
    /// so seeing it means a rule or the harness changed.
    #[must_use]
    pub fn cmd_note(&self) -> Option<String> {
        (self.cmd > 0).then(|| {
            format!(
                "{} vsift call(s) ran through cmd.exe's vsift.cmd shim, which re-reads arguments (L-109, issue #257); the harness expects Git Bash or PowerShell",
                self.cmd
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn shell_call(command: &str, denied: bool) -> ToolCall {
        ToolCall {
            index: 0,
            step: 0,
            id: "call".to_owned(),
            kind: CallKind::Shell {
                command: command.to_owned(),
            },
            input: json!({}),
            denied,
            exit_code: Some(0),
            is_error: false,
            completed: true,
            output: None,
        }
    }

    #[test]
    fn a_plain_call_is_a_posix_shim_call() {
        let used = ShimUse::from_calls(&[shell_call("vsift setup check --json", false)]);
        assert_eq!(
            used,
            ShimUse {
                posix: 1,
                powershell: 0,
                cmd: 0
            }
        );
        assert!(used.cmd_note().is_none());
    }

    #[test]
    fn a_wrapper_names_the_shell_that_ran_the_call() {
        let used = ShimUse::from_calls(&[
            shell_call(r#"powershell -NoProfile -Command "vsift --version""#, false),
            shell_call("cmd /c vsift --version", false),
            shell_call("bash -lc 'vsift --version'", false),
        ]);
        assert_eq!(
            used,
            ShimUse {
                posix: 1,
                powershell: 1,
                cmd: 1
            }
        );
        assert!(used.cmd_note().is_some());
    }

    #[test]
    fn an_explicit_extension_names_its_shim_in_any_shell() {
        let used = ShimUse::from_calls(&[
            shell_call("vsift.cmd --version", false),
            shell_call("vsift.ps1 --version", false),
        ]);
        assert_eq!((used.posix, used.powershell, used.cmd), (0, 1, 1));
    }

    #[test]
    fn a_refused_call_and_other_programs_are_not_counted() {
        let used = ShimUse::from_calls(&[
            shell_call("cmd /c vsift --version", true),
            shell_call("ls -la", false),
        ]);
        assert!(used.is_empty());
    }

    #[test]
    fn every_vsift_command_of_a_pipeline_counts() {
        let used = ShimUse::from_calls(&[shell_call(
            "vsift setup check --json && vsift ingest clip.mp4 --json",
            false,
        )]);
        assert_eq!(used.posix, 2);
    }
}
