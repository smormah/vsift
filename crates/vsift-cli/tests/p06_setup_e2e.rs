//! Opt-in cumulative P06 checkpoint: detect, select, verify and guide.
//!
//! Setup journeys drive the compiled `vsift` binary against an isolated
//! per-user configuration base, exactly as a headless agent would. Tool
//! verification drives the engine library, because by design (ADR 0015) no
//! CLI command exposes it yet. `ffmpeg` and `ffprobe` must be on `PATH`;
//! whisper.cpp is deliberately not required. Nothing is downloaded or
//! installed and no journey opens a network connection.
//!
//! `cargo test -p vsift-cli --locked --test p06_setup_e2e -- --ignored --nocapture`
//!
//! The run writes a bounded report to `.vsift/e2e-runs/<run-id>/report.json`.
//! A journey that cannot run here is `blocked`, never `passed`, and the test
//! fails unless every P06 journey passed.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::{Value, json};
use vsift_application::{MediaToolCheck, MediaToolVerification, MediaToolVerifier};
use vsift_infrastructure::{
    ExecutableResolver, FixtureMediaToolVerifier, HostIsolation, MediaProviderConformance,
    ProcessCancellation, ProcessRequest, ProcessSupervisor, ProcessWorkingDirectory,
    SupervisorPolicy, TerminationReason, TrustedExecutable, UserDependencyConfigStore,
    reviewed_compatibility_policy,
};

type TestResult = Result<(), Box<dyn Error>>;

/// Upper bound for one CLI invocation; a prompt or hang is killed and fails.
const CLI_DEADLINE: Duration = Duration::from_secs(60);
const OWNED_PREFIX: &str = "vsift-p06-e2e-";
const MAX_DIAGNOSTIC_CHARS: usize = 240;
const QUALIFIED_TARGET: &str = "ubuntu_24_04_x86_64";
const MISSING_MEDIA_TOOLS: &str = "FFmpeg or FFprobe is not on PATH; install or locate trusted builds, then run setup configure ffmpeg|ffprobe --executable <absolute-path>";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

/// A temporary directory this checkpoint created and alone may remove.
struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    /// Returns a fresh, not-yet-created per-user base for one journey.
    fn base(&self, journey: &str) -> PathBuf {
        self.0.join(journey)
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// Why a journey stopped before passing.
enum StageStop {
    /// The observed behaviour contradicted the expected assertion.
    Failed(String),
    /// A prerequisite outside `VSift` is absent, so the journey could not run.
    Blocked(String),
}

impl<E: Error> From<E> for StageStop {
    fn from(error: E) -> Self {
        Self::Failed(bounded(&error.to_string()))
    }
}

type StageResult = Result<Value, StageStop>;

fn bounded(text: &str) -> String {
    text.chars().take(MAX_DIAGNOSTIC_CHARS).collect()
}

fn ensure(condition: bool, expectation: &str) -> Result<(), StageStop> {
    if condition {
        Ok(())
    } else {
        Err(StageStop::Failed(bounded(expectation)))
    }
}

fn ensure_eq(observed: &Value, expected: &str, what: &str) -> Result<(), StageStop> {
    ensure(
        observed == expected,
        &format!("{what}: expected {expected}, observed {observed}"),
    )
}

fn step_mentions(value: &Value, needle: &str) -> bool {
    value.as_str().is_some_and(|step| step.contains(needle))
}

/// The two media tools found on `PATH`, when both are present.
struct MediaTools {
    ffmpeg: TrustedExecutable,
    ffprobe: TrustedExecutable,
}

impl MediaTools {
    fn discover() -> Option<Self> {
        let resolver = ExecutableResolver::from_current_path();
        Some(Self {
            ffmpeg: resolver.resolve(OsStr::new("ffmpeg")).ok()?,
            ffprobe: resolver.resolve(OsStr::new("ffprobe")).ok()?,
        })
    }
}

fn require(tools: Option<&MediaTools>) -> Result<&MediaTools, StageStop> {
    tools.ok_or_else(|| StageStop::Blocked(MISSING_MEDIA_TOOLS.to_owned()))
}

fn config_root(base: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        base.join("Library/Application Support/vsift")
    }
    #[cfg(not(target_os = "macos"))]
    {
        base.join("vsift")
    }
}

/// A bounded `vsift` invocation whose per-user state lives beneath `base`.
fn vsift(base: &Path) -> Result<Command, StageStop> {
    let mut command = Command::cargo_bin("vsift")?;
    command
        .env("LOCALAPPDATA", base)
        .env("XDG_CONFIG_HOME", base)
        .env("HOME", base)
        .timeout(CLI_DEADLINE);
    Ok(command)
}

/// Runs one JSON command and returns its exit code and single result document.
fn run_json(command: &mut Command) -> Result<(Option<i32>, Value), StageStop> {
    let output = command.output()?;
    ensure(
        output.stderr.is_empty(),
        "a JSON command wrote to standard error",
    )?;
    Ok((
        output.status.code(),
        serde_json::from_slice(&output.stdout)?,
    ))
}

fn select_media_tools(base: &Path, tools: &MediaTools) -> Result<(), StageStop> {
    for (dependency, tool) in [("ffmpeg", &tools.ffmpeg), ("ffprobe", &tools.ffprobe)] {
        let output = vsift(base)?
            .args(["setup", "configure", dependency, "--executable"])
            .arg(tool.path())
            .arg("--json")
            .output()?;
        ensure(output.status.success(), "setup configure did not succeed")?;
        ensure(
            !String::from_utf8_lossy(&output.stdout)
                .contains(tool.path().to_string_lossy().as_ref()),
            "setup configure disclosed the selected path",
        )?;
        let value: Value = serde_json::from_slice(&output.stdout)?;
        ensure_eq(
            &value["data"]["source"],
            "configured_user_path",
            "configure source",
        )?;
    }
    Ok(())
}

/// Preinstalled tools on the ambient `PATH` are found through the filtered lookup.
fn preinstalled_on_path(root: &OwnedRoot, tools: Option<&MediaTools>) -> StageResult {
    require(tools)?;
    let base = root.base("preinstalled");
    let (code, value) = run_json(vsift(&base)?.args(["setup", "check", "--json"]))?;
    ensure(
        code == Some(0),
        "setup check with media tools did not exit 0",
    )?;
    ensure(
        value["status"] == "ready" || value["status"] == "degraded",
        "media tools on PATH did not give ready or degraded",
    )?;
    for index in 0..2 {
        let dependency = &value["dependencies"][index];
        ensure_eq(&dependency["status"], "available", "media status")?;
        ensure_eq(&dependency["lookup"], "filtered_path", "media lookup")?;
        ensure_eq(
            &dependency["validation"],
            "executable_probe_only",
            "media validation",
        )?;
    }
    ensure(
        !config_root(&base).exists(),
        "setup check created per-user configuration",
    )?;
    Ok(json!({
        "readiness": value["status"],
        "whisper_status": value["dependencies"][2]["status"],
        "ffmpeg_version": value["dependencies"][0]["detail"],
        "ffprobe_version": value["dependencies"][1]["detail"],
    }))
}

/// Off-`PATH` tools are selected per call or persisted, and win over `PATH`.
fn off_path_selection(root: &OwnedRoot, tools: Option<&MediaTools>) -> StageResult {
    let tools = require(tools)?;
    let base = root.base("selected");
    select_media_tools(&base, tools)?;
    let (code, value) = run_json(
        vsift(&base)?
            .args(["setup", "check", "--json"])
            .env("PATH", ""),
    )?;
    ensure(code == Some(0), "configured media tools did not exit 0")?;
    for index in 0..2 {
        let dependency = &value["dependencies"][index];
        ensure_eq(&dependency["status"], "available", "configured status")?;
        ensure_eq(
            &dependency["lookup"],
            "configured_user_path",
            "configured lookup",
        )?;
    }

    let per_call_base = root.base("per-call");
    let (code, value) = run_json(
        vsift(&per_call_base)?
            .args(["setup", "check", "--json", "--ffmpeg"])
            .arg(tools.ffmpeg.path())
            .arg("--ffprobe")
            .arg(tools.ffprobe.path())
            .env("PATH", ""),
    )?;
    ensure(code == Some(0), "per-call media tools did not exit 0")?;
    for index in 0..2 {
        let dependency = &value["dependencies"][index];
        ensure_eq(&dependency["status"], "available", "per-call status")?;
        ensure_eq(&dependency["lookup"], "explicit_path", "per-call lookup")?;
    }
    ensure(
        !config_root(&per_call_base).exists(),
        "a per-call selection was persisted",
    )?;
    Ok(json!({
        "configured_lookup": "configured_user_path",
        "per_call_lookup": "explicit_path",
        "per_call_persisted": false,
    }))
}

/// Whisper missing leaves media usable and names both remedies.
fn partial_setup(root: &OwnedRoot, tools: Option<&MediaTools>) -> StageResult {
    let tools = require(tools)?;
    let base = root.base("partial");
    select_media_tools(&base, tools)?;
    let (code, check) = run_json(
        vsift(&base)?
            .args(["setup", "check", "--json"])
            .env("PATH", ""),
    )?;
    ensure(code == Some(0), "degraded setup check did not exit 0")?;
    ensure_eq(&check["status"], "degraded", "readiness")?;
    let whisper = &check["dependencies"][2];
    ensure_eq(&whisper["status"], "missing", "whisper status")?;
    let remediation = &whisper["remediation"];
    ensure_eq(&remediation["reason"], "missing", "remediation reason")?;
    ensure_eq(&remediation["required_authority"], "user", "authority")?;
    ensure_eq(
        &remediation["managed_install"],
        "unavailable_unqualified",
        "managed install",
    )?;
    ensure_eq(
        &remediation["explicit_path_option"],
        "--whisper",
        "path option",
    )?;
    ensure(
        step_mentions(&remediation["next_step"], "supplied transcript"),
        "whisper remediation does not offer the supplied-transcript route",
    )?;

    let (code, plan) = run_json(
        vsift(&base)?
            .args(["setup", "plan", "--profile", "desktop", "--json"])
            .env("PATH", ""),
    )?;
    ensure(code == Some(0), "setup plan did not exit 0")?;
    let data = &plan["data"];
    ensure_eq(&data["readiness"], "degraded", "plan readiness")?;
    for index in 0..2 {
        ensure_eq(
            &data["dependencies"][index]["disposition"],
            "existing_executable_probe_only",
            "media disposition",
        )?;
    }
    let whisper_plan = &data["dependencies"][2];
    let expected = if data["target"] == QUALIFIED_TARGET {
        "managed_install"
    } else {
        "manual_selection_required"
    };
    ensure_eq(
        &whisper_plan["disposition"],
        expected,
        "whisper disposition",
    )?;
    ensure(
        step_mentions(&whisper_plan["next_step"], "supplied transcript")
            || data["target"] == QUALIFIED_TARGET,
        "whisper plan step does not offer the supplied-transcript route",
    )?;
    ensure(
        step_mentions(&data["local_asr_model"]["next_step"], "supplied transcript")
            || data["target"] == QUALIFIED_TARGET,
        "model plan step does not offer the supplied-transcript route",
    )?;
    Ok(json!({
        "readiness": "degraded",
        "whisper_disposition": whisper_plan["disposition"],
        "model_disposition": data["local_asr_model"]["disposition"],
        "supplied_transcript_import": "not_implemented (P07)",
    }))
}

/// No media tools blocks setup with typed, non-executing remediation.
fn missing_media_blocked(root: &OwnedRoot) -> StageResult {
    let base = root.base("missing");
    let (code, value) = run_json(
        vsift(&base)?
            .args(["setup", "check", "--json", "--timeout-seconds", "5"])
            .env("PATH", ""),
    )?;
    ensure(code == Some(2), "missing media tools did not exit 2")?;
    ensure_eq(&value["status"], "blocked", "readiness")?;
    for (index, option) in [(0, "--ffmpeg"), (1, "--ffprobe")] {
        let dependency = &value["dependencies"][index];
        ensure_eq(&dependency["status"], "missing", "media status")?;
        let remediation = &dependency["remediation"];
        ensure_eq(&remediation["reason"], "missing", "remediation reason")?;
        ensure_eq(&remediation["required_authority"], "user", "authority")?;
        ensure_eq(
            &remediation["managed_install"],
            "unavailable_unqualified",
            "managed install",
        )?;
        ensure_eq(&remediation["explicit_path_option"], option, "path option")?;
    }

    // Headless: stdin is closed, so a prompt would read EOF or hit the deadline.
    let output = vsift(&base)?
        .args([
            "setup",
            "check",
            "--events",
            "jsonl",
            "--timeout-seconds",
            "5",
        ])
        .env("PATH", "")
        .output()?;
    ensure(
        output.status.code() == Some(2),
        "headless check did not exit 2",
    )?;
    let lines: Vec<&[u8]> = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    let [line] = lines.as_slice() else {
        return Err(StageStop::Failed(
            "headless check did not emit exactly one record".to_owned(),
        ));
    };
    let event: Value = serde_json::from_slice(line)?;
    ensure_eq(&event["event"], "terminal", "headless event")?;
    ensure(
        !config_root(&base).exists(),
        "a blocked check created per-user configuration",
    )?;
    Ok(json!({"readiness": "blocked", "headless_records": 1}))
}

/// The host target's plan gives manual guidance or reviewable, uninstallable actions.
fn managed_target_plan(root: &OwnedRoot) -> StageResult {
    let base = root.base("plan");
    let (code, plan) = run_json(
        vsift(&base)?
            .args(["setup", "plan", "--profile", "desktop", "--json"])
            .env("PATH", ""),
    )?;
    ensure(code == Some(0), "setup plan did not exit 0")?;
    let data = &plan["data"];
    let qualified = data["target"] == QUALIFIED_TARGET;
    if qualified {
        ensure_eq(
            &data["managed_install"],
            "catalogue_accepted_install_pending",
            "managed install",
        )?;
        ensure(
            data["plan_digest"].as_str().map(str::len) == Some(64),
            "qualified plan has no digest",
        )?;
    } else {
        ensure(
            step_mentions(&data["managed_install"], "unavailable_"),
            "unqualified target reported a managed install",
        )?;
        ensure(data["actions"] == json!([]), "unqualified plan has actions")?;
        ensure(
            data["plan_digest"].is_null(),
            "unqualified plan has a digest",
        )?;
        for index in 0..3 {
            let dependency = &data["dependencies"][index];
            ensure_eq(
                &dependency["disposition"],
                "manual_selection_required",
                "disposition",
            )?;
            ensure_eq(&dependency["required_authority"], "user", "authority")?;
            ensure(
                step_mentions(&dependency["next_step"], "setup configure"),
                "manual guidance does not name setup configure",
            )?;
        }
    }

    let plan_file = root.0.join("saved-plan.json");
    fs::write(&plan_file, serde_json::to_vec(&plan)?)?;
    let digest = data["plan_digest"]
        .as_str()
        .map_or_else(|| "0".repeat(64), str::to_owned);
    let (code, install) = run_json(
        vsift(&base)?
            .args(["setup", "install", "--plan"])
            .arg(&plan_file)
            .args(["--accept-plan", &digest, "--json"])
            .env("PATH", ""),
    )?;
    ensure(code == Some(2), "setup install did not exit 2")?;
    ensure_eq(
        &install["error"]["code"],
        if qualified {
            "COMMAND_NOT_IMPLEMENTED"
        } else {
            "INVALID_ARGUMENT"
        },
        "install outcome",
    )?;
    ensure(
        !config_root(&base).exists(),
        "planning or install created per-user state",
    )?;
    Ok(json!({
        "target": data["target"],
        "branch": if qualified { "qualified_install_reserved" } else { "unqualified_manual_guidance" },
        "managed_install": data["managed_install"],
        "install_outcome": install["error"]["code"],
    }))
}

#[cfg(unix)]
fn deny_record_replacement(config: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(config, fs::Permissions::from_mode(0o500))?;
    let probe = config.join("permission-probe");
    match fs::write(&probe, b"") {
        Ok(()) => {
            fs::remove_file(&probe)?;
            Ok(false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Ok(true),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn allow_record_replacement(config: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(config, fs::Permissions::from_mode(0o700))
}

#[cfg(windows)]
fn deny_record_replacement(config: &Path) -> std::io::Result<bool> {
    let record = config.join("dependencies-v1.json");
    let mut permissions = fs::metadata(&record)?.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&record, permissions)?;
    Ok(true)
}

#[cfg(windows)]
#[allow(
    clippy::permissions_set_readonly_false,
    reason = "On Windows this clears only the read-only attribute the journey set"
)]
fn allow_record_replacement(config: &Path) -> std::io::Result<()> {
    let record = config.join("dependencies-v1.json");
    let mut permissions = fs::metadata(&record)?.permissions();
    permissions.set_readonly(false);
    fs::set_permissions(&record, permissions)
}

/// A denied configuration write fails typed, once, and changes nothing.
fn denied_configuration_storage(root: &OwnedRoot) -> StageResult {
    let base = root.base("denied");
    let stand_in = Command::cargo_bin("vsift")?.get_program().to_os_string();
    let configured = vsift(&base)?
        .args(["setup", "configure", "ffmpeg", "--executable"])
        .arg(&stand_in)
        .arg("--json")
        .output()?;
    ensure(configured.status.success(), "initial configure failed")?;
    let config = config_root(&base);
    let record = config.join("dependencies-v1.json");
    let before = fs::read(&record)?;
    // Build the command first so nothing fallible sits between deny and restore.
    let mut command = vsift(&base)?;
    command
        .args(["setup", "configure", "ffprobe", "--executable"])
        .arg(&stand_in)
        .arg("--json");
    if !deny_record_replacement(&config)? {
        allow_record_replacement(&config)?;
        return Err(StageStop::Blocked(
            "permission bits do not bind the current user; run unprivileged".to_owned(),
        ));
    }
    let denied = run_json(&mut command);
    allow_record_replacement(&config)?;
    let (code, value) = denied?;
    ensure(code == Some(7), "denied storage did not exit 7")?;
    ensure_eq(&value["error"]["code"], "STORAGE_IO", "error code")?;
    ensure(
        value["error"]["retryable"] == false,
        "denied storage invited a retry",
    )?;
    ensure(
        value["error"]["remediation"] == json!([]),
        "denied storage suggested a command",
    )?;
    ensure(fs::read(&record)? == before, "the stored record changed")?;
    Ok(json!({"error_code": "STORAGE_IO", "record_unchanged": true}))
}

/// Configured tools read back from storage pass the F01 verification; a substitute fails.
async fn selected_tools_verified(root: &OwnedRoot, tools: Option<&MediaTools>) -> StageResult {
    let tools = require(tools)?;
    let base = root.base("verified");
    select_media_tools(&base, tools)?;
    let stored = UserDependencyConfigStore::at(config_root(&base))?.read()?;
    let (Some(ffmpeg), Some(ffprobe)) = (stored.ffmpeg, stored.ffprobe) else {
        return Err(StageStop::Failed(
            "configured selections were not stored".to_owned(),
        ));
    };
    let ffmpeg = TrustedExecutable::explicit(ffmpeg)?;
    let ffprobe = TrustedExecutable::explicit(ffprobe)?;
    let workspace = root.0.join("verification");
    fs::create_dir(&workspace)?;
    let verifier = |probe: TrustedExecutable| -> Result<FixtureMediaToolVerifier, StageStop> {
        Ok(FixtureMediaToolVerifier::new(
            MediaProviderConformance::r0(ffmpeg.clone(), probe),
            HostIsolation::ProcessOnly,
            workspace.clone(),
            reviewed_compatibility_policy()?,
            ProcessCancellation::new(),
        ))
    };

    let outcome = verifier(ffprobe)?.verify().await;
    ensure(
        outcome == MediaToolVerification::Verified,
        &format!("selected tools were not verified: {outcome:?}"),
    )?;
    ensure(
        fs::read_dir(&workspace)?.next().is_none(),
        "verification left a workspace behind",
    )?;

    // FFmpeg in FFprobe's place answers a version probe but cannot describe media.
    let substitute = verifier(ffmpeg.clone())?.verify().await;
    let MediaToolVerification::Failed { check, failure } = substitute else {
        return Err(StageStop::Failed(
            "a probe-incapable substitute was verified".to_owned(),
        ));
    };
    ensure(
        check == MediaToolCheck::Probe,
        "substitute failed at the wrong check",
    )?;
    ensure(
        fs::read_dir(&workspace)?.next().is_none(),
        "failed verification left a workspace behind",
    )?;
    Ok(json!({
        "selected": "verified",
        "substitute": {"check": check.identifier(), "failure": failure.identifier()},
    }))
}

fn stage(name: &str, started: Instant, result: StageResult) -> Value {
    let elapsed_ms = started.elapsed().as_millis();
    match result {
        Ok(evidence) => json!({
            "name": name, "status": "passed", "elapsed_ms": elapsed_ms, "evidence": evidence,
        }),
        Err(StageStop::Failed(diagnostic)) => json!({
            "name": name, "status": "failed", "elapsed_ms": elapsed_ms, "diagnostic": diagnostic,
        }),
        Err(StageStop::Blocked(remediation)) => json!({
            "name": name, "status": "blocked", "elapsed_ms": elapsed_ms, "remediation": remediation,
        }),
    }
}

async fn tool_line(
    executable: TrustedExecutable,
    root: &Path,
    arguments: &[&str],
) -> Option<String> {
    let cwd = ProcessWorkingDirectory::new(root).ok()?;
    let request = ProcessRequest::new(executable, cwd, Duration::from_secs(5))
        .ok()?
        .with_arguments(arguments.iter().copied());
    let limit = NonZeroUsize::new(64 * 1024)?;
    let supervisor = ProcessSupervisor::new(
        SupervisorPolicy::new(limit, Duration::from_secs(1), Duration::from_secs(1)),
        HostIsolation::ProcessOnly,
    );
    let result = supervisor
        .run(request, ProcessCancellation::new())
        .await
        .ok()?;
    if result.termination != TerminationReason::Exited
        || !result.status.success()
        || !result.stdout.complete
    {
        return None;
    }
    std::str::from_utf8(&result.stdout.bytes).ok().map(|value| {
        value
            .lines()
            .next()
            .map_or_else(String::new, |line| line.chars().take(160).collect())
    })
}

async fn repository_state(repository: &Path) -> (Option<String>, Option<bool>) {
    let Ok(git) = ExecutableResolver::from_current_path().resolve(OsStr::new("git")) else {
        return (None, None);
    };
    let revision = tool_line(git.clone(), repository, &["rev-parse", "HEAD"]).await;
    let dirty = tool_line(git, repository, &["status", "--porcelain"])
        .await
        .map(|line| !line.is_empty());
    (revision, dirty)
}

/// Runs every P06 journey in order; each one owns a fresh per-user base.
async fn run_journeys(root: &OwnedRoot, tools: Option<&MediaTools>) -> Vec<Value> {
    let mut stages = Vec::new();
    let clock = Instant::now();
    stages.push(stage(
        "p06_preinstalled_on_path",
        clock,
        preinstalled_on_path(root, tools),
    ));
    let clock = Instant::now();
    stages.push(stage(
        "p06_off_path_selection",
        clock,
        off_path_selection(root, tools),
    ));
    let clock = Instant::now();
    stages.push(stage(
        "p06_partial_setup_whisper_missing",
        clock,
        partial_setup(root, tools),
    ));
    let clock = Instant::now();
    stages.push(stage(
        "p06_missing_media_blocked",
        clock,
        missing_media_blocked(root),
    ));
    let clock = Instant::now();
    stages.push(stage(
        "p06_managed_target_plan",
        clock,
        managed_target_plan(root),
    ));
    let clock = Instant::now();
    stages.push(stage(
        "p06_denied_configuration_storage",
        clock,
        denied_configuration_storage(root),
    ));
    let clock = Instant::now();
    let verified = selected_tools_verified(root, tools).await;
    stages.push(stage("p06_selected_tools_verified", clock, verified));

    stages
}

#[tokio::test]
#[ignore = "opt-in P06 checkpoint; needs ffmpeg and ffprobe on PATH; reports to .vsift/e2e-runs"]
async fn dependency_setup_checkpoint() -> TestResult {
    let started = Instant::now();
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_dir = repository
        .join(".vsift/e2e-runs")
        .join(format!("p06-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&run_dir)?;
    let root = OwnedRoot::new()?;
    let tools = MediaTools::discover();

    let stages = run_journeys(&root, tools.as_ref()).await;

    let status_of = |wanted: &str| stages.iter().any(|entry| entry["status"] == wanted);
    let overall = if status_of("failed") {
        "failed"
    } else if status_of("blocked") {
        "blocked"
    } else {
        "passed"
    };
    let future_stages: Vec<_> = [
        "p10_recovery",
        "p11_worker_batch",
        "p12_agent_clients",
        "p13_distribution",
        "p14_release_qualification",
    ]
    .into_iter()
    .map(|name| json!({"name": name, "status": "not_implemented"}))
    .collect();
    let manifest: Value =
        serde_json::from_slice(&fs::read(repository.join("fixtures/corpus/manifest.json"))?)?;
    let versions = stages
        .first()
        .map(|entry| entry["evidence"].clone())
        .unwrap_or_default();
    let (revision, dirty) = repository_state(&repository).await;
    let report = json!({
        "schema_version": 1,
        "checkpoint": "P06",
        "revision": revision,
        "dirty": dirty,
        "fixture_manifest": {
            "schema_version": manifest["schema_version"],
            "corpus_id": manifest["corpus_id"],
        },
        "fixture": "F01.mp4 (embedded in the verifier)",
        "fixture_sha256": reviewed_compatibility_policy()?.fixture.sha256_hex(),
        "os": env::consts::OS,
        "architecture": env::consts::ARCH,
        "filesystem": env::var("VSIFT_E2E_FILESYSTEM").map_or_else(
            |_| "uninspected".to_owned(),
            |value| format!("operator reported: {}", value.chars().take(60).collect::<String>()),
        ),
        "resource_profile": "setup probes bounded by --timeout-seconds; each CLI call killed after 60 s; verification under the reviewed compatibility policy limits",
        "vsift_version": env!("CARGO_PKG_VERSION"),
        "ffmpeg_version": versions["ffmpeg_version"],
        "ffprobe_version": versions["ffprobe_version"],
        "whisper_version": null,
        "model_version": null,
        "client_versions": [],
        "authorization": "opt-in cargo test invocation; setup configure writes only to isolated temporary per-user bases; no install, download or network access",
        "prior_checkpoints": ["P04: p04_media_e2e", "P05: p05_session_e2e"],
        "stages": stages,
        "coverage_gaps": [
            "Whisper functional verification and the automatic media preflight arrive with P07",
            "The supplied-transcript import path is P07; this checkpoint proves only that missing whisper degrades rather than blocks",
            "Managed installation, managed precedence and installer fallback belong to P13",
            "Offline behaviour is inferred: no P06 command opens a network connection, but no network-denied sandbox is applied",
            "Unix denied-storage evidence is unobservable when run as root"
        ],
        "future_stages": future_stages,
        "overall": overall,
        "complete_journey": "not_implemented",
        "elapsed_ms": started.elapsed().as_millis(),
    });
    let report_path = run_dir.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("P06 checkpoint report: {}", report_path.display());
    if overall == "passed" {
        Ok(())
    } else {
        Err(format!("P06 checkpoint {overall}; see {}", report_path.display()).into())
    }
}
