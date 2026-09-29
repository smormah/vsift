//! `run` and `grade` against the stand-in client.
//!
//! The stand-in (`vsift-trials-stub-client`) replays a recorded Claude Code
//! stream and records the arguments, working directory and environment it
//! was given, so these tests prove what `run` passes to a client (an
//! explicit argument list, no shell, a cleared environment, the canary)
//! and that its output reaches the raw-log directory and the grader. No
//! prompt is sent anywhere.

use std::{
    collections::BTreeMap,
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use vsift_agent_trials::{
    TrialError,
    claude_trust::{CLAUDE_STATE_FILE, project_key},
    error::write_json,
    evaluate::{GradeOptions, grade_phase},
    layout::{PreparedState, TrialLayout, TrialManifest},
    record::{MAX_RECORD_BYTES, write_record},
    roots::RootPolicy,
    run::{
        CODEX_WINDOWS_SANDBOX, RunRequest, client_arguments, codex_writable_root, phase_prompt, run,
    },
    scenario::Scenario,
    trace::ClientKind,
};

type TestResult = Result<(), Box<dyn Error>>;

const CANARY: &str = "VSIFT-CANARY-00000000000000000000beef";
const STUB: &str = env!("CARGO_BIN_EXE_vsift-trials-stub-client");

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn scenario_file(id: &str) -> PathBuf {
    repository()
        .join("tools")
        .join("vsift-agent-trials")
        .join("scenarios")
        .join(format!("{id}.json"))
}

/// Distinguishes the trials one test process creates. The clock alone did
/// not: macOS reports wall time in microseconds, so two tests starting in
/// parallel got the same root, and the first to finish deleted the other's
/// files (`NotFound` on hosted macOS, PR #203).
static NEXT_TRIAL: AtomicU64 = AtomicU64::new(0);

/// Names the step and path of a failed file operation.
fn at<T>(step: &str, path: &Path, result: io::Result<T>) -> Result<T, Box<dyn Error>> {
    result.map_err(|error| format!("{step} {}: {error}", path.display()).into())
}

/// Reads a text file, naming it on failure.
fn read_text(path: &Path) -> Result<String, Box<dyn Error>> {
    at("reading", path, fs::read_to_string(path))
}

/// A trial directory below the system temporary directory, removed on drop.
struct Trial {
    root: PathBuf,
    layout: TrialLayout,
}

impl Trial {
    fn new(scenario: &str) -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_TRIAL.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "vsift-trials-stub-{}-{sequence}-{stamp}",
            std::process::id()
        ));
        // `create_dir`, not `create_dir_all`: a root that already exists
        // belongs to someone else and must fail loudly, never be shared.
        at("creating the trial root", &root, fs::create_dir(&root))?;
        let layout = TrialLayout::new(root.join("trial"));
        let stub = layout.workspace().join(".stub");
        at("creating", &stub, fs::create_dir_all(&stub))?;
        let client_home = root.join("client-home");
        at("creating", &client_home, fs::create_dir_all(&client_home))?;
        at(
            "creating",
            &layout.harness(),
            fs::create_dir_all(layout.harness()),
        )?;
        at(
            "copying the scenario to",
            &layout.scenario(),
            fs::copy(scenario_file(scenario), layout.scenario()),
        )?;
        let parsed = Scenario::load(&layout.scenario())?;
        let manifest = TrialManifest {
            schema_version: 1,
            trial_id: "trial".to_owned(),
            scenario_id: parsed.id.clone(),
            scenario_sha256: "0".repeat(64),
            repository: repository(),
            video: "walkthrough.mp4".to_owned(),
            transcript: None,
            canary_variable: Some("VSIFT_TRIAL_SERVICE_TOKEN".to_owned()),
            canaries: vec![CANARY.to_owned()],
            prepared: PreparedState::default(),
            fixture_hashes: BTreeMap::new(),
            skill_sha256: "0".repeat(64),
            settings_sha256: "0".repeat(64),
            vsift_commit: "0".repeat(40),
            vsift_sha256: "0".repeat(64),
            vsift_executable: PathBuf::from(STUB),
            prompts: parsed
                .phases
                .iter()
                .map(|phase| phase.prompt.replace("{{video}}", "walkthrough.mp4"))
                .collect(),
        };
        write_json(&layout.manifest(), &manifest)?;
        Ok(Self { root, layout })
    }

    fn behave(&self, behaviour: &Value, replay: &str) -> TestResult {
        let stub = self.layout.workspace().join(".stub");
        let behaviour_file = stub.join("behaviour.json");
        at(
            "writing",
            &behaviour_file,
            fs::write(&behaviour_file, behaviour.to_string()),
        )?;
        let replay_file = stub.join("replay.jsonl");
        at("writing", &replay_file, fs::write(&replay_file, replay))?;
        Ok(())
    }

    fn request(&self, client: ClientKind, timeout: Duration) -> RunRequest {
        RunRequest {
            trial: self.layout.trial().to_path_buf(),
            phase: 1,
            client,
            executable: PathBuf::from(STUB),
            model: "compact-model".to_owned(),
            client_home: self.root.join("client-home"),
            timeout,
            max_turns: 40,
            path_directories: Vec::new(),
            pass_environment: Vec::new(),
            system_path: Vec::new(),
            root_policy: RootPolicy::new(Vec::new(), Vec::new()),
            debug_prompt: None,
        }
    }

    fn invocation(&self) -> Result<Value, Box<dyn Error>> {
        Ok(serde_json::from_str(&read_text(
            &self
                .layout
                .workspace()
                .join(".stub")
                .join("invocation.json"),
        )?)?)
    }
}

impl Drop for Trial {
    fn drop(&mut self) {
        if self
            .root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("vsift-trials-stub-"))
        {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

/// A recorded A-01 run: the skill loaded, one setup check, an honest
/// handoff that names the missing tools.
fn a01_stream() -> String {
    let handoff = json!({
        "handoff_version": "1",
        "status": "insufficient_evidence",
        "question": "What status and build number does the presenter report?",
        "capabilities": {"image_access": "unavailable", "image_check_code": null, "media_tools": "missing",
                         "local_asr": "not_run", "transcript_basis": "none"},
        "session": {"session_id": null, "source_id": null, "revision_id": null, "duration_us": null},
        "claims": [],
        "citations": [],
        "gaps": [{"kind": "dependency", "range": null, "reason": "needs_user_authority",
                  "code": "MISSING_CAPABILITY", "note": "FFmpeg and FFprobe are neither installed nor registered."}],
        "untrusted_instructions": [],
        "budget": {"profile": "compact", "overrides": false,
                   "limits": {"images_per_step": 1, "images_total": 6, "image_bytes": 12_582_912, "page_limit": 20,
                              "tool_calls": 30, "refinement_depth": 2, "wall_time_s": 900, "burst_frames": 4},
                   "used": {"images_total": 0, "image_bytes": 0, "tool_calls": 1, "refinement_depth": 0, "wall_time_s": 20},
                   "exhausted": []},
        "lifecycle": {"policy": "default", "action": "not_opened", "mode": null, "expires_at": null},
        "resume": null
    });
    let message = format!(
        "FFmpeg and FFprobe are missing. Install trusted builds and register them with setup configure.\n\n\
         ```vsift-handoff\n{handoff}\n```\n"
    );
    [
        json!({"type": "system", "subtype": "init", "model": "compact-model", "claude_code_version": "stub"}),
        json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t0", "name": "Skill", "input": {"skill": "vsift"}}]}}),
        json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t0", "content": "loaded"}]}}),
        json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "vsift setup check --json"}}]}}),
        json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "content": "{}"}]}}),
        json!({"type": "result", "subtype": "success", "result": message, "num_turns": 3, "duration_ms": 20_000,
               "permission_denials": []}),
    ]
    .iter()
    .map(Value::to_string)
    .collect::<Vec<_>>()
    .join("\n")
}

#[tokio::test]
async fn run_passes_an_explicit_argument_list_and_a_cleared_environment() -> TestResult {
    let trial = Trial::new("A-01-f01-missing-tools")?;
    let replay = a01_stream();
    trial.behave(&json!({"replay": "replay.jsonl", "exit_code": 0}), &replay)?;
    let request = trial.request(ClientKind::ClaudeCode, Duration::from_secs(60));
    let record = run(&request).await?;
    assert_eq!(record.exit_code, Some(0));
    assert!(!record.timed_out);
    assert_eq!(read_text(&record.stdout)?, replay);
    assert!(read_text(&record.stderr)?.contains("stub client stderr line"));
    assert!(record.stdout.starts_with(trial.layout.raw(1)));

    let invocation = trial.invocation()?;
    let scenario = Scenario::load(&trial.layout.scenario())?;
    let prompt = phase_prompt(
        &trial.layout,
        &serde_json::from_value(serde_json::from_str::<Value>(&read_text(
            &trial.layout.manifest(),
        )?)?)?,
        1,
    )?;
    let expected = client_arguments(&request, &trial.layout, &scenario, &prompt);
    assert_eq!(invocation["arguments"], json!(expected));
    assert_eq!(expected[0], "-p");
    for flag in [
        "--permission-mode",
        "dontAsk",
        "--output-format",
        "stream-json",
        "--max-turns",
    ] {
        assert!(expected.iter().any(|argument| argument == flag), "{flag}");
    }
    let names: Vec<String> = serde_json::from_value(invocation["environment_names"].clone())?;
    for present in [
        "CLAUDE_CONFIG_DIR",
        "VSIFT_TRIAL_SERVICE_TOKEN",
        "LOCALAPPDATA",
        "XDG_CONFIG_HOME",
        "PATH",
    ] {
        assert!(
            names.iter().any(|name| name.eq_ignore_ascii_case(present)),
            "{present} missing"
        );
    }
    for absent in [
        "USERNAME",
        "USER",
        "CODEX_HOME",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
    ] {
        assert!(
            !names.iter().any(|name| name.eq_ignore_ascii_case(absent)),
            "{absent} leaked"
        );
    }
    let cwd = PathBuf::from(invocation["cwd"].as_str().ok_or("no cwd")?);
    assert_eq!(
        fs::canonicalize(cwd)?,
        fs::canonicalize(trial.layout.workspace())?
    );

    let graded = grade_phase(&trial.layout, 1, &GradeOptions::default())?;
    let failed: Vec<&str> = graded
        .mechanical
        .checks
        .iter()
        .filter(|check| !check.passed)
        .map(|check| check.name.as_str())
        .collect();
    assert!(failed.is_empty(), "{failed:?}");
    assert!(graded.interpretation.passed, "{:?}", graded.interpretation);
    assert!(trial.layout.phase(1).join("grade.json").is_file());
    a_regrade_writes_beside_the_original(&trial.layout, graded.mechanical.passed)?;

    // The bounded record: under 64 KiB, local paths and the canary
    // replaced by tokens, the raw log identified by its digest only.
    let output = trial.root.join("records").join("a-01-stub.json");
    let client_home = trial.root.join("client-home");
    let written = write_record(&trial.layout, 1, &output, Some(&client_home), &[])?;
    let bytes = fs::read(&output)?;
    assert!(bytes.len() <= MAX_RECORD_BYTES);
    let text = String::from_utf8(bytes)?;
    let workspace = trial.layout.workspace().to_string_lossy().into_owned();
    assert!(!text.contains(&workspace) && !text.contains(&workspace.replace('\\', "\\\\")));
    assert!(!text.contains(CANARY));
    assert_eq!(written["valid"], true);
    assert_eq!(
        written["run"]["raw_stdout_sha256"].as_str().map(str::len),
        Some(64)
    );
    assert_eq!(written["mechanical"]["passed"], true);
    assert!(written["interpretation"]["human_review"].is_null());
    assert!(matches!(
        write_record(&trial.layout, 1, &trial.root.join("record.txt"), None, &[]),
        Err(TrialError::Refused(_))
    ));
    Ok(())
}

/// Regression for PR #203 on hosted macOS: trials created at the same
/// instant by parallel tests must get separate roots, or one test's cleanup
/// deletes another's files.
#[test]
fn trials_created_in_parallel_never_share_a_root() -> TestResult {
    let trials: Vec<Trial> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..32)
            .map(|_| {
                scope.spawn(|| Trial::new("A-01-f01-missing-tools").map_err(|e| e.to_string()))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| "a trial thread failed".to_owned())
                    .and_then(|trial| trial)
            })
            .collect::<Result<Vec<_>, String>>()
    })?;
    let roots: std::collections::BTreeSet<&PathBuf> =
        trials.iter().map(|trial| &trial.root).collect();
    assert_eq!(roots.len(), trials.len());
    for trial in &trials {
        assert!(
            trial.layout.scenario().is_file(),
            "{}",
            trial.root.display()
        );
    }
    Ok(())
}

#[tokio::test]
async fn claude_code_gets_one_settings_source_in_a_trusted_workspace() -> TestResult {
    let trial = Trial::new("A-01-f01-missing-tools")?;
    let state_file = trial.root.join("client-home").join(CLAUDE_STATE_FILE);
    at(
        "writing",
        &state_file,
        fs::write(
            &state_file,
            "{\n  \"userID\": \"kept\",\n  \"projects\": {\n    \"elsewhere\": {\"hasTrustDialogAccepted\": false}\n  }\n}\n",
        ),
    )?;
    trial.behave(&json!({"exit_code": 0}), "")?;
    let request = trial.request(ClientKind::ClaudeCode, Duration::from_secs(60));
    let record = run(&request).await?;
    // One source of permission rules: the trusted workspace's project
    // settings, never a second copy through `--settings`.
    let arguments: Vec<String> = serde_json::from_value(trial.invocation()?["arguments"].clone())?;
    assert!(!arguments.iter().any(|argument| argument == "--settings"));
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair[0] == "--setting-sources" && pair[1] == "project")
    );
    let state: Value = serde_json::from_str(&read_text(&state_file)?)?;
    assert_eq!(
        state["projects"][project_key(&trial.layout.workspace())]["hasTrustDialogAccepted"],
        true
    );
    assert_eq!(
        state["projects"]["elsewhere"]["hasTrustDialogAccepted"],
        false
    );
    assert_eq!(state["userID"], "kept");
    assert_eq!(record.client_setup.len(), 1);
    // A missing client home is refused before the client starts.
    let client_home = trial.root.join("client-home");
    at("removing", &client_home, fs::remove_dir_all(&client_home))?;
    let invocation = trial
        .layout
        .workspace()
        .join(".stub")
        .join("invocation.json");
    at("removing", &invocation, fs::remove_file(&invocation))?;
    assert!(matches!(run(&request).await, Err(TrialError::Refused(_))));
    assert!(
        !trial
            .layout
            .workspace()
            .join(".stub")
            .join("invocation.json")
            .exists()
    );
    Ok(())
}

/// A re-grade (PR 3e) writes beside the original grade and never over it;
/// a file name that is not a plain `.json` name is refused.
fn a_regrade_writes_beside_the_original(layout: &TrialLayout, passed: bool) -> TestResult {
    let original = fs::read(layout.phase(1).join("grade.json"))?;
    let again = GradeOptions {
        output: Some("grade-3e.json".to_owned()),
        ..GradeOptions::default()
    };
    let regraded = grade_phase(layout, 1, &again)?;
    assert_eq!(regraded.mechanical.passed, passed);
    assert!(layout.phase(1).join("grade-3e.json").is_file());
    assert_eq!(fs::read(layout.phase(1).join("grade.json"))?, original);
    for refused in ["../grade.json", "grade.txt", ".json"] {
        let options = GradeOptions {
            output: Some(refused.to_owned()),
            ..GradeOptions::default()
        };
        assert!(
            matches!(
                grade_phase(layout, 1, &options),
                Err(TrialError::Refused(_))
            ),
            "{refused}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn codex_gets_its_sandbox_and_the_session_root_as_writable() -> TestResult {
    let trial = Trial::new("A-05-f07-images-disabled")?;
    trial.behave(&json!({"exit_code": 3}), "")?;
    let record = run(&trial.request(ClientKind::Codex, Duration::from_secs(60))).await?;
    assert_eq!(record.exit_code, Some(3));
    let invocation = trial.invocation()?;
    let arguments: Vec<String> = serde_json::from_value(invocation["arguments"].clone())?;
    assert_eq!(&arguments[..3], ["exec", "--json", "--ephemeral"]);
    for flag in [
        "--ignore-user-config",
        "--sandbox",
        "workspace-write",
        "sandbox_workspace_write.network_access=false",
        "sandbox_workspace_write.exclude_tmpdir_env_var=true",
        "sandbox_workspace_write.exclude_slash_tmp=true",
    ] {
        assert!(arguments.iter().any(|argument| argument == flag), "{flag}");
    }
    // The Codex diagnostic pass (2026-09-29): codex-cli 0.155 reported
    // "`tools.view_image` is ignored" and let the agent view images. The
    // images-disabled scenario turns the `view_image` feature off instead.
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair[0] == "--disable" && pair[1] == "view_image"),
        "{arguments:?}"
    );
    assert!(
        !arguments
            .iter()
            .any(|argument| argument.contains("tools.view_image"))
    );
    assert_eq!(record.client_home, Some(trial.root.join("client-home")));
    // Without a Windows sandbox mode codex-cli 0.155 rejects every command
    // ("blocked by policy", the first dry trial).
    assert_eq!(
        arguments
            .iter()
            .any(|argument| argument == CODEX_WINDOWS_SANDBOX),
        cfg!(windows)
    );
    // On Unix the per-user base exists before the client starts (Codex's
    // Linux sandbox binds every writable root), private as VSift makes it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let base = trial.layout.user_base();
        assert_eq!(fs::metadata(&base)?.permissions().mode() & 0o777, 0o700);
    }
    #[cfg(not(unix))]
    assert!(record.client_setup.is_empty());
    assert!(
        !trial
            .root
            .join("client-home")
            .join(CLAUDE_STATE_FILE)
            .exists(),
        "Codex runs never touch a Claude Code state file"
    );
    // The extra writable root holds the session root: the root itself on
    // Windows, the per-user base on Unix (which must exist before Codex's
    // Linux sandbox can bind it; the session root is VSift's to create).
    let writable = codex_writable_root(&trial.layout);
    assert!(trial.layout.session_root().starts_with(&writable));
    let writable_text = writable.to_string_lossy().into_owned();
    assert!(arguments.iter().any(|argument| {
        argument == &format!("sandbox_workspace_write.writable_roots=['{writable_text}']")
    }));
    assert!(!trial.layout.session_root().exists());
    let names: Vec<String> = serde_json::from_value(invocation["environment_names"].clone())?;
    assert!(names.iter().any(|name| name == "CODEX_HOME"));
    assert!(!names.iter().any(|name| name == "CLAUDE_CONFIG_DIR"));
    Ok(())
}

#[tokio::test]
async fn a_debug_run_uses_the_operators_prompt_and_is_never_a_trial() -> TestResult {
    let trial = Trial::new("A-01-f01-missing-tools")?;
    let replay = a01_stream();
    trial.behave(&json!({"replay": "replay.jsonl", "exit_code": 0}), &replay)?;
    let mut request = trial.request(ClientKind::Codex, Duration::from_secs(60));
    request.debug_prompt = Some("Run vsift --version and report the result.".to_owned());
    let record = run(&request).await?;
    assert!(record.debug_prompt);
    let arguments: Vec<String> = serde_json::from_value(trial.invocation()?["arguments"].clone())?;
    assert_eq!(
        arguments.last().map(String::as_str),
        Some("Run vsift --version and report the result.")
    );
    // The scenario's own prompt is not passed.
    let scenario_prompt = phase_prompt(
        &trial.layout,
        &serde_json::from_value(serde_json::from_str::<Value>(&read_text(
            &trial.layout.manifest(),
        )?)?)?,
        1,
    )?;
    assert!(!arguments.contains(&scenario_prompt));
    // grade marks it invalid; record refuses it.
    let graded = grade_phase(&trial.layout, 1, &GradeOptions::default())?;
    assert!(!graded.is_valid());
    assert!(
        graded
            .invalid_reasons
            .iter()
            .any(|reason| reason.contains("debug run"))
    );
    let output = trial.root.join("debug-record.json");
    assert!(matches!(
        write_record(&trial.layout, 1, &output, None, &[]),
        Err(TrialError::Refused(_))
    ));
    assert!(!output.exists());
    Ok(())
}

#[tokio::test]
async fn a_sign_in_value_in_the_output_fails_no_canary_and_is_never_logged() -> TestResult {
    const SIGN_IN_PART: &str = "c2lnbi1pbi12YWx1ZS1mb3ItdGhlLXNjYW4tdGVzdA";
    let trial = Trial::new("A-01-f01-missing-tools")?;
    let client_home = trial.root.join("client-home");
    let auth = client_home.join("auth.json");
    at(
        "writing",
        &auth,
        fs::write(
            &auth,
            json!({"tokens": {"access_token": format!("eyJhbGciOiJub25lIn0x.{SIGN_IN_PART}.sig")}})
                .to_string(),
        ),
    )?;
    // The agent printed one part of the token.
    let replay = format!(
        "{}\n{}",
        a01_stream(),
        json!({"type": "system", "subtype": "note", "text": SIGN_IN_PART})
    );
    trial.behave(&json!({"replay": "replay.jsonl", "exit_code": 0}), &replay)?;
    let record = run(&trial.request(ClientKind::ClaudeCode, Duration::from_secs(60))).await?;
    let scan = record.sign_in_leak_check.ok_or("no scan")?;
    assert_eq!(scan.files_checked, 1);
    assert!(scan.found);
    let run_json = read_text(&trial.layout.phase(1).join("run.json"))?;
    assert!(
        !run_json.contains(SIGN_IN_PART),
        "the value is never logged"
    );
    let graded = grade_phase(&trial.layout, 1, &GradeOptions::default())?;
    let no_canary = graded
        .mechanical
        .checks
        .iter()
        .find(|check| check.name == "no_canary")
        .ok_or("no no_canary check")?;
    assert!(!no_canary.passed);
    assert!(!serde_json::to_string(&graded)?.contains(SIGN_IN_PART));
    Ok(())
}

#[tokio::test]
async fn a_client_that_overruns_its_time_is_stopped() -> TestResult {
    let trial = Trial::new("A-01-f01-missing-tools")?;
    trial.behave(&json!({"sleep_ms": 30_000}), "")?;
    let record = run(&trial.request(ClientKind::ClaudeCode, Duration::from_secs(2))).await?;
    assert!(record.timed_out);
    assert_eq!(record.exit_code, None);
    assert!(record.wall_ms < 20_000, "{} ms", record.wall_ms);
    Ok(())
}

#[tokio::test]
async fn a_root_in_the_profile_is_refused_before_anything_runs() -> TestResult {
    let trial = Trial::new("A-01-f01-missing-tools")?;
    let mut request = trial.request(ClientKind::ClaudeCode, Duration::from_secs(5));
    request.root_policy = RootPolicy::new(vec![trial.root.clone()], Vec::new());
    assert!(matches!(run(&request).await, Err(TrialError::Refused(_))));
    assert!(
        !trial
            .layout
            .workspace()
            .join(".stub")
            .join("invocation.json")
            .exists()
    );
    Ok(())
}

#[test]
fn a_later_phase_gets_the_earlier_resume_card() -> TestResult {
    let trial = Trial::new("A-02-f02-compact-resume")?;
    let manifest: TrialManifest = serde_json::from_str(&read_text(&trial.layout.manifest())?)?;
    assert!(
        phase_prompt(&trial.layout, &manifest, 2).is_err(),
        "no phase 1 grade yet"
    );
    write_json(
        &trial.layout.phase(1).join("grade.json"),
        &json!({"handoff": {"resume": {"state": "VERIFY_SOURCE", "session_id": "ses_0123456789abcdef"}}}),
    )?;
    let prompt = phase_prompt(&trial.layout, &manifest, 2)?;
    assert!(prompt.contains("\"session_id\":\"ses_0123456789abcdef\""));
    assert!(!prompt.contains("{{resume_card}}"));
    Ok(())
}
