//! Runs one named client for one trial phase.
//!
//! The client is started through the executable path the operator gives,
//! with an explicit argument list and no shell, in the trial workspace, with
//! a cleared environment (only what the client needs, the isolated per-user
//! base, the client's own sign-in directory and the canary), a wall-clock
//! timeout, and stdout and stderr written straight to the raw-log
//! directory. On Windows the client runs in a job object and on Unix in its
//! own process group, so a timeout stops everything it started.
//!
//! Claude Code: `claude -p <prompt> --output-format stream-json --verbose
//! --model <m> --max-turns <n> --setting-sources project --permission-mode
//! dontAsk --no-session-persistence --strict-mcp-config`. The trial's
//! permission rules have **one** source: the workspace's
//! `.claude/settings.json` (the committed settings `prepare` copied), read
//! as the project source. Claude Code applies a project's *allow* rules only
//! in a trusted workspace, so `run` first marks this one workspace as
//! trusted in the client home ([`crate::claude_trust`]). The harness no
//! longer also passes the file with `--settings`: in the first dry trial
//! that second copy was the only source of the allow rules (the project's
//! were ignored as untrusted), so it was unclear which rules applied. The
//! settings allow `Bash(vsift:*)`, `Read` below the workspace and the
//! `vsift` skill, and deny web, write and edit tools; `dontAsk` denies
//! anything else without prompting, except commands Claude Code itself
//! classes as read-only (such as `echo`), which it runs and the grader
//! fails. `CLAUDE_CONFIG_DIR` is the operator's signed-in trial
//! configuration, so no personal settings or memory file is loaded.
//!
//! Codex: `codex exec --json --ephemeral --ignore-user-config --ignore-rules
//! --skip-git-repo-check -m <m> --sandbox workspace-write -C <workspace>
//! -c approval_policy="never" -c sandbox_workspace_write.network_access=false
//! -c sandbox_workspace_write.writable_roots=['<session root or per-user base>']
//! -c sandbox_workspace_write.exclude_tmpdir_env_var=true
//! -c sandbox_workspace_write.exclude_slash_tmp=true [--disable view_image]
//! <prompt>`, with `CODEX_HOME` the operator's signed-in trial home. A
//! scenario without images turns Codex's image tool off with `--disable
//! view_image` (the `view_image` feature); the earlier `-c
//! tools.view_image=false` was an unknown setting Codex ignored (the Codex
//! diagnostic pass, 2026-09-29). The run stays `--ephemeral`: a debug run
//! without it (2026-09-29) showed that codex-cli 0.155's session rollout
//! records an image view only inside a code-mode `exec` tool call, not as a
//! `view_image` record a harness could count, so Codex's image budgets
//! remain unmeasured (known limit L-075). Codex trials run on
//! Linux, inside the trial container (`tools/vsift-agent-trials/containers/
//! codex`), where Codex's own Linux sandbox (its bundled bubblewrap) keeps
//! writes to the workspace and the writable roots and takes the network
//! away from every command. On Windows `run` adds
//! `-c windows.sandbox="unelevated"`: codex-cli 0.155 reads the Windows
//! sandbox mode from the user configuration, which `--ignore-user-config`
//! skips, and without a mode it rejects every command as "blocked by
//! policy" (the first dry trial). That sandbox cannot run `VSift` and does
//! not enforce the network (known limit L-076), which is why Codex trials
//! moved to the container. Codex's permission model differs from Claude
//! Code's; the grader enforces the same policy on both from the event
//! stream (ADR 0022 decision 7), so a permissive configuration cannot pass
//! a forbidden action.
//!
//! A *debug run* replaces the scenario's prompt with the operator's
//! (`--debug-prompt`), to check a client's environment with a trivial
//! request. Its run record says so, `grade` marks it invalid and `record`
//! refuses it: a debug run is never a trial.

use std::{
    env,
    ffi::OsString,
    fs::{self, File},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use process_wrap::tokio::{CommandWrap, KillOnDrop};
use serde::{Deserialize, Serialize};

use crate::{
    claude_trust::{TrustOutcome, trust_workspace},
    error::{TrialError, read_json, write_json},
    layout::{TrialLayout, TrialManifest},
    leak_check::{self, LeakCheck},
    roots::RootPolicy,
    scenario::{ImagePolicy, Scenario},
    skill::file_digest,
    trace::ClientKind,
};

/// Environment variables a client needs from the harness's environment on
/// Windows (process creation, PowerShell, temporary paths are set apart).
const WINDOWS_PASSTHROUGH: [&str; 9] = [
    "SystemRoot",
    "windir",
    "SystemDrive",
    "ComSpec",
    "PATHEXT",
    "OS",
    "PROCESSOR_ARCHITECTURE",
    "NUMBER_OF_PROCESSORS",
    "ProgramData",
];

/// The Windows sandbox mode Codex runs with: the restricted-token sandbox,
/// which needs no administrator setup (see the module documentation).
pub const CODEX_WINDOWS_SANDBOX: &str = "windows.sandbox=\"unelevated\"";

/// Codex's feature for its image tool; `codex features list` shows it as
/// stable and on by default, and `--disable` turns it off for one run.
pub const CODEX_IMAGE_FEATURE: &str = "view_image";

/// Executables that must not be reachable on the client's `PATH`: the
/// dependency picture must come from registrations alone.
const MEDIA_TOOLS: [&str; 3] = ["ffmpeg", "ffprobe", "whisper-cli"];

/// What `run` needs.
#[derive(Clone, Debug)]
pub struct RunRequest {
    /// The trial directory `prepare` made.
    pub trial: PathBuf,
    /// The phase, from 1.
    pub phase: usize,
    /// Which client.
    pub client: ClientKind,
    /// The client executable (absolute).
    pub executable: PathBuf,
    /// The model name to pass.
    pub model: String,
    /// The client's signed-in trial configuration directory
    /// (`CLAUDE_CONFIG_DIR` or `CODEX_HOME`).
    pub client_home: PathBuf,
    /// Wall-clock limit.
    pub timeout: Duration,
    /// Claude Code's `--max-turns`.
    pub max_turns: u32,
    /// Extra `PATH` directories the client needs (for example Git Bash's
    /// `usr/bin` for Claude Code on Windows).
    pub path_directories: Vec<PathBuf>,
    /// Names of harness environment variables to pass through (for example
    /// `CLAUDE_CODE_GIT_BASH_PATH` or a proxy).
    pub pass_environment: Vec<String>,
    /// The operating system directories every client needs on `PATH`
    /// ([`system_path_directories`] for real runs).
    pub system_path: Vec<PathBuf>,
    /// What the trial root must avoid.
    pub root_policy: RootPolicy,
    /// A prompt that replaces the scenario's, for a debug run (see the
    /// module documentation); `None` for a trial.
    pub debug_prompt: Option<String>,
}

/// What `run` did, written to `harness/phase-<n>/run.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunRecord {
    /// Which client.
    pub client: ClientKind,
    /// What `<client> --version` printed.
    pub client_version: Option<String>,
    /// The model passed.
    pub model: String,
    /// The phase, from 1.
    pub phase: usize,
    /// The arguments after the executable (the prompt included).
    pub arguments: Vec<String>,
    /// The names of the environment variables the client received.
    pub environment_names: Vec<String>,
    /// What the harness changed in the client home before the start (for
    /// Claude Code, the workspace's trust entry). Empty in run records
    /// written before this field existed.
    #[serde(default)]
    pub client_setup: Vec<String>,
    /// Whether the operator replaced the scenario's prompt: a debug run,
    /// never a trial. False in run records written before this field
    /// existed.
    #[serde(default)]
    pub debug_prompt: bool,
    /// Whether a value of the client home's sign-in file appeared in the
    /// raw output, scanned right after the client exited (counts only,
    /// never the values: [`crate::leak_check`]). `None` in run records
    /// written before this field existed.
    #[serde(default)]
    pub sign_in_leak_check: Option<LeakCheck>,
    /// The client home the run used, so `grade` can recognise the client's
    /// own spill files below it. `None` in run records written before this
    /// field existed (`grade --client-home` supplies it then). Local only:
    /// `record` never copies it.
    #[serde(default)]
    pub client_home: Option<PathBuf>,
    /// Start time, Unix seconds.
    pub started_unix_s: u64,
    /// Wall time.
    pub wall_ms: u64,
    /// Exit code, if the client exited by itself.
    pub exit_code: Option<i32>,
    /// Whether the harness stopped it at the timeout.
    pub timed_out: bool,
    /// The raw stdout file.
    pub stdout: PathBuf,
    /// The raw stderr file.
    pub stderr: PathBuf,
    /// SHA-256 of stdout.
    pub stdout_sha256: String,
    /// SHA-256 of stderr.
    pub stderr_sha256: String,
}

/// Reads the manifest of a trial.
///
/// # Errors
///
/// [`TrialError`] when it is missing or malformed.
pub fn read_manifest(layout: &TrialLayout) -> Result<TrialManifest, TrialError> {
    serde_json::from_value(read_json(&layout.manifest())?)
        .map_err(|error| TrialError::json("harness/trial.json", error))
}

/// The prompt of a phase, with an earlier phase's resume card filled in.
///
/// # Errors
///
/// [`TrialError::Invalid`] when the phase does not exist or needs a resume
/// card the previous phase's grade does not have.
pub fn phase_prompt(
    layout: &TrialLayout,
    manifest: &TrialManifest,
    phase: usize,
) -> Result<String, TrialError> {
    let prompt = phase
        .checked_sub(1)
        .and_then(|index| manifest.prompts.get(index))
        .ok_or_else(|| TrialError::Invalid(format!("the scenario has no phase {phase}")))?;
    if !prompt.contains("{{resume_card}}") {
        return Ok(prompt.clone());
    }
    let previous = read_json(&layout.phase(phase - 1).join("grade.json"))?;
    let card = &previous["handoff"]["resume"];
    if card.is_null() {
        return Err(TrialError::Invalid(format!(
            "phase {phase} needs phase {}'s resume card, and its handoff has none",
            phase - 1
        )));
    }
    let text =
        serde_json::to_string(card).map_err(|error| TrialError::json("resume card", error))?;
    Ok(prompt.replace("{{resume_card}}", &text))
}

/// The client's arguments.
#[must_use]
pub fn client_arguments(
    request: &RunRequest,
    layout: &TrialLayout,
    scenario: &Scenario,
    prompt: &str,
) -> Vec<String> {
    let workspace = layout.workspace().to_string_lossy().into_owned();
    match request.client {
        ClientKind::ClaudeCode | ClientKind::ProcedureWalker => vec![
            "-p".to_owned(),
            prompt.to_owned(),
            "--output-format".to_owned(),
            "stream-json".to_owned(),
            "--verbose".to_owned(),
            "--model".to_owned(),
            request.model.clone(),
            "--max-turns".to_owned(),
            request.max_turns.to_string(),
            "--setting-sources".to_owned(),
            "project".to_owned(),
            "--permission-mode".to_owned(),
            "dontAsk".to_owned(),
            "--no-session-persistence".to_owned(),
            "--strict-mcp-config".to_owned(),
        ],
        ClientKind::Codex => {
            let mut arguments = vec![
                "exec".to_owned(),
                "--json".to_owned(),
                "--ephemeral".to_owned(),
                "--ignore-user-config".to_owned(),
                "--ignore-rules".to_owned(),
                "--skip-git-repo-check".to_owned(),
                "-m".to_owned(),
                request.model.clone(),
                "--sandbox".to_owned(),
                "workspace-write".to_owned(),
                "-C".to_owned(),
                workspace,
                "-c".to_owned(),
                "approval_policy=\"never\"".to_owned(),
                "-c".to_owned(),
                "sandbox_workspace_write.network_access=false".to_owned(),
                "-c".to_owned(),
                format!(
                    "sandbox_workspace_write.writable_roots=['{}']",
                    codex_writable_root(layout).to_string_lossy()
                ),
                "-c".to_owned(),
                "sandbox_workspace_write.exclude_tmpdir_env_var=true".to_owned(),
                "-c".to_owned(),
                "sandbox_workspace_write.exclude_slash_tmp=true".to_owned(),
            ];
            if cfg!(windows) {
                arguments.extend(["-c".to_owned(), CODEX_WINDOWS_SANDBOX.to_owned()]);
            }
            if scenario.image_policy == ImagePolicy::Disabled {
                arguments.extend(["--disable".to_owned(), CODEX_IMAGE_FEATURE.to_owned()]);
            }
            arguments.push(prompt.to_owned());
            arguments
        }
    }
}

/// The operating system directories a client needs on `PATH`: the
/// Windows system and `PowerShell` folders, or `/usr/bin` and `/bin`.
#[must_use]
pub fn system_path_directories() -> Vec<PathBuf> {
    if cfg!(windows) {
        let root =
            env::var_os("SystemRoot").map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from);
        vec![
            root.join("System32"),
            root.clone(),
            root.join("System32").join("WindowsPowerShell").join("v1.0"),
        ]
    } else {
        vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")]
    }
}

/// The client's `PATH`: the directory of the trial's `vsift`, the
/// operator's extra directories and the system directories.
///
/// # Errors
///
/// [`TrialError::Refused`] when a media tool is reachable on it.
pub fn client_path(
    vsift: &Path,
    extra: &[PathBuf],
    system: &[PathBuf],
) -> Result<OsString, TrialError> {
    let mut directories: Vec<PathBuf> = vsift.parent().map(Path::to_path_buf).into_iter().collect();
    directories.extend(extra.iter().cloned());
    directories.extend(system.iter().cloned());
    for directory in &directories {
        for tool in MEDIA_TOOLS {
            let found = [tool.to_owned(), format!("{tool}.exe")]
                .iter()
                .any(|name| directory.join(name).is_file());
            if found {
                return Err(TrialError::Refused(format!(
                    "{tool} is reachable on the client's PATH; the trial must see only registered tools"
                )));
            }
        }
    }
    env::join_paths(directories)
        .map_err(|error| TrialError::Refused(format!("a PATH directory is not usable: {error}")))
}

/// The client's environment, from nothing.
///
/// # Errors
///
/// As [`client_path`].
pub fn client_environment(
    request: &RunRequest,
    layout: &TrialLayout,
    manifest: &TrialManifest,
) -> Result<Vec<(String, OsString)>, TrialError> {
    let mut environment: Vec<(String, OsString)> = vec![(
        "PATH".to_owned(),
        client_path(
            &manifest.vsift_executable,
            &request.path_directories,
            &request.system_path,
        )?,
    )];
    if cfg!(windows) {
        for name in WINDOWS_PASSTHROUGH {
            if let Some(value) = env::var_os(name) {
                environment.push((name.to_owned(), value));
            }
        }
    }
    for name in &request.pass_environment {
        if let Some(value) = env::var_os(name) {
            environment.push((name.clone(), value));
        }
    }
    for (name, value) in layout.user_environment() {
        environment.push((name, value.into_os_string()));
    }
    let temporary = layout.trial().join("tmp");
    for name in ["TEMP", "TMP", "TMPDIR"] {
        environment.push((name.to_owned(), temporary.clone().into_os_string()));
    }
    match request.client {
        ClientKind::ClaudeCode | ClientKind::ProcedureWalker => {
            environment.push((
                "CLAUDE_CONFIG_DIR".to_owned(),
                request.client_home.clone().into_os_string(),
            ));
            environment.push(("DISABLE_AUTOUPDATER".to_owned(), "1".into()));
            environment.push((
                "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".to_owned(),
                "1".into(),
            ));
        }
        ClientKind::Codex => environment.push((
            "CODEX_HOME".to_owned(),
            request.client_home.clone().into_os_string(),
        )),
    }
    if let (Some(name), Some(value)) = (&manifest.canary_variable, manifest.canaries.first()) {
        environment.push((name.clone(), value.into()));
    }
    Ok(environment)
}

/// Runs one phase.
///
/// # Errors
///
/// [`TrialError`] when the trial, root or client home is refused, or the
/// client cannot be started or stopped.
pub async fn run(request: &RunRequest) -> Result<RunRecord, TrialError> {
    let layout = TrialLayout::new(&request.trial);
    request.root_policy.check(&request.trial)?;
    request.root_policy.check(&request.client_home)?;
    if !request.executable.is_absolute() || !request.executable.is_file() {
        return Err(TrialError::Refused(
            "the client executable must be an absolute path to a file".to_owned(),
        ));
    }
    let manifest = read_manifest(&layout)?;
    let scenario = Scenario::load(&layout.scenario())?;
    let prompt = match &request.debug_prompt {
        Some(prompt) => prompt.clone(),
        None => phase_prompt(&layout, &manifest, request.phase)?,
    };
    let arguments = client_arguments(request, &layout, &scenario, &prompt);
    let environment = client_environment(request, &layout, &manifest)?;
    for directory in [
        layout.trial().join("tmp"),
        layout.raw(request.phase),
        layout.phase(request.phase),
    ] {
        fs::create_dir_all(&directory)
            .map_err(|error| TrialError::io_step("creating", &directory, error))?;
    }
    let client_setup = prepare_client(request, &layout)?;
    let client_version = client_version(&request.executable, &environment, &layout).await;
    let raw = layout.raw(request.phase);
    let stdout_path = raw.join("stdout.jsonl");
    let stderr_path = raw.join("stderr.txt");
    let stdout = File::create(&stdout_path)
        .map_err(|error| TrialError::io_step("creating the raw stdout log", &stdout_path, error))?;
    let stderr = File::create(&stderr_path)
        .map_err(|error| TrialError::io_step("creating the raw stderr log", &stderr_path, error))?;

    let mut command = tokio::process::Command::new(&request.executable);
    command
        .args(&arguments)
        .env_clear()
        .envs(environment.iter().map(|(name, value)| (name, value)))
        .current_dir(layout.workspace())
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    let mut wrapped = CommandWrap::from(command);
    contain(&mut wrapped);
    wrapped.wrap(KillOnDrop);
    let started_unix_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let started = Instant::now();
    let mut child = wrapped
        .spawn()
        .map_err(|error| TrialError::Process(format!("the client did not start: {error}")))?;
    let waited = tokio::time::timeout(request.timeout, child.wait()).await;
    let timed_out = waited.is_err();
    let exit_code = if let Ok(status) = waited {
        status
            .map_err(|error| {
                TrialError::Process(format!("waiting for the client failed: {error}"))
            })?
            .code()
    } else {
        let _ = child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(30), child.wait()).await;
        None
    };
    let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let sign_in_leak_check = check_sign_in_leak(&request.client_home, &stdout_path, &stderr_path)?;
    let record = RunRecord {
        client: request.client,
        client_version,
        model: request.model.clone(),
        phase: request.phase,
        arguments,
        environment_names: environment.iter().map(|(name, _)| name.clone()).collect(),
        client_setup,
        debug_prompt: request.debug_prompt.is_some(),
        sign_in_leak_check: Some(sign_in_leak_check),
        client_home: Some(request.client_home.clone()),
        started_unix_s,
        wall_ms,
        exit_code,
        timed_out,
        stdout_sha256: file_digest(&stdout_path)?,
        stderr_sha256: file_digest(&stderr_path)?,
        stdout: stdout_path,
        stderr: stderr_path,
    };
    write_json(&layout.phase(request.phase).join("run.json"), &record)?;
    Ok(record)
}

/// Scans the phase's raw logs for the client's sign-in values while the
/// sign-in is still present ([`crate::leak_check`]).
///
/// # Errors
///
/// [`TrialError::Io`] when a raw log cannot be read.
fn check_sign_in_leak(
    client_home: &Path,
    stdout: &Path,
    stderr: &Path,
) -> Result<LeakCheck, TrialError> {
    let read = |path: &Path| {
        fs::read(path)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .map_err(|error| TrialError::io_step("reading the raw log", path, error))
    };
    let (out, err) = (read(stdout)?, read(stderr)?);
    Ok(leak_check::scan(client_home, &[&out, &err]))
}

/// Changes the client home needs before this trial starts, described for
/// the run record (never with the home's contents).
///
/// # Errors
///
/// As [`trust_workspace`].
fn prepare_client(request: &RunRequest, layout: &TrialLayout) -> Result<Vec<String>, TrialError> {
    match request.client {
        ClientKind::ClaudeCode | ClientKind::ProcedureWalker => {
            let outcome = trust_workspace(&request.client_home, &layout.workspace())?;
            Ok(vec![
                match outcome {
                    TrustOutcome::Marked => {
                        "marked the trial workspace as trusted in the client home"
                    }
                    TrustOutcome::AlreadyTrusted => {
                        "the trial workspace was already trusted in the client home"
                    }
                }
                .to_owned(),
            ])
        }
        ClientKind::Codex => create_writable_roots(layout),
    }
}

/// The extra writable root Codex gets: a directory that holds `VSift`'s
/// session root (which is also inside the workspace, itself writable).
///
/// On Windows it is the session root itself. On Unix it is the per-user
/// base (`<workspace>/.home`): Codex's Linux sandbox bind-mounts every
/// writable root and refuses to run a command when one does not exist yet
/// (bubblewrap: "Can't bind mount ... No such file or directory", the first
/// container debug run), and the session root cannot be made in advance:
/// `VSift` creates and provisions it on its first command (an empty folder
/// made by anyone else has no ownership marker, and `VSift` rightly refuses
/// it as `INTEGRITY_FAILURE`).
#[must_use]
pub fn codex_writable_root(layout: &TrialLayout) -> PathBuf {
    if cfg!(windows) {
        layout.session_root()
    } else {
        layout.user_base()
    }
}

/// Makes sure Codex's extra writable root exists before Codex starts, on
/// Unix (see [`codex_writable_root`]): the per-user base, mode 0700 as
/// `VSift` makes its own folders. It never creates the session root.
///
/// # Errors
///
/// [`TrialError::Io`] when the directory cannot be created.
#[cfg(unix)]
fn create_writable_roots(layout: &TrialLayout) -> Result<Vec<String>, TrialError> {
    use std::os::unix::fs::DirBuilderExt;
    let root = codex_writable_root(layout);
    if root.is_dir() {
        return Ok(Vec::new());
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&root)
        .map_err(|error| TrialError::io_step("creating the per-user base", &root, error))?;
    Ok(vec![
        "created the per-user base (mode 0700) so Codex's Linux sandbox can bind it".to_owned(),
    ])
}

#[cfg(not(unix))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "One signature for every platform; only Unix can fail here"
)]
fn create_writable_roots(_layout: &TrialLayout) -> Result<Vec<String>, TrialError> {
    Ok(Vec::new())
}

#[cfg(windows)]
fn contain(command: &mut CommandWrap) {
    command.wrap(process_wrap::tokio::JobObject);
}

#[cfg(unix)]
fn contain(command: &mut CommandWrap) {
    command.wrap(process_wrap::tokio::ProcessGroup::leader());
}

#[cfg(not(any(unix, windows)))]
fn contain(_command: &mut CommandWrap) {}

/// `<client> --version`, which sends no prompt.
async fn client_version(
    executable: &Path,
    environment: &[(String, OsString)],
    layout: &TrialLayout,
) -> Option<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::new(executable)
            .arg("--version")
            .env_clear()
            .envs(environment.iter().map(|(name, value)| (name, value)))
            .current_dir(layout.workspace())
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!text.is_empty()).then(|| text.chars().take(120).collect())
}

/// Reads a phase's run record.
///
/// # Errors
///
/// [`TrialError`] when it is missing or malformed.
pub fn read_run(layout: &TrialLayout, phase: usize) -> Result<RunRecord, TrialError> {
    serde_json::from_value::<RunRecord>(read_json(&layout.phase(phase).join("run.json"))?)
        .map_err(|error| TrialError::json("run.json", error))
}

/// The raw client output of a phase (stdout, then stderr).
///
/// # Errors
///
/// [`TrialError::Io`] when a log cannot be read.
pub fn raw_output(record: &RunRecord) -> Result<(String, String), TrialError> {
    let read = |path: &Path| {
        fs::read(path)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .map_err(|error| TrialError::io_step("reading the raw log", path, error))
    };
    Ok((read(&record.stdout)?, read(&record.stderr)?))
}
