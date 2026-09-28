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
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use vsift_agent_trials::{
    TrialError,
    claude_trust::{CLAUDE_STATE_FILE, project_key},
    error::write_json,
    evaluate::grade_phase,
    layout::{PreparedState, TrialLayout, TrialManifest},
    record::{MAX_RECORD_BYTES, write_record},
    roots::RootPolicy,
    run::{CODEX_WINDOWS_SANDBOX, RunRequest, client_arguments, phase_prompt, run},
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

/// A trial directory below the system temporary directory, removed on drop.
struct Trial {
    root: PathBuf,
    layout: TrialLayout,
}

impl Trial {
    fn new(scenario: &str) -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root =
            std::env::temp_dir().join(format!("vsift-trials-stub-{}-{stamp}", std::process::id()));
        let layout = TrialLayout::new(root.join("trial"));
        fs::create_dir_all(layout.workspace().join(".stub"))?;
        fs::create_dir_all(root.join("client-home"))?;
        fs::create_dir_all(layout.harness())?;
        fs::copy(scenario_file(scenario), layout.scenario())?;
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
        fs::write(stub.join("behaviour.json"), behaviour.to_string())?;
        fs::write(stub.join("replay.jsonl"), replay)?;
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
        }
    }

    fn invocation(&self) -> Result<Value, Box<dyn Error>> {
        Ok(serde_json::from_str(&fs::read_to_string(
            self.layout
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
    assert_eq!(fs::read_to_string(&record.stdout)?, replay);
    assert!(fs::read_to_string(&record.stderr)?.contains("stub client stderr line"));
    assert!(record.stdout.starts_with(trial.layout.raw(1)));

    let invocation = trial.invocation()?;
    let scenario = Scenario::load(&trial.layout.scenario())?;
    let prompt = phase_prompt(
        &trial.layout,
        &serde_json::from_value(serde_json::from_str::<Value>(&fs::read_to_string(
            trial.layout.manifest(),
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

    let graded = grade_phase(&trial.layout, 1)?;
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

#[tokio::test]
async fn claude_code_gets_one_settings_source_in_a_trusted_workspace() -> TestResult {
    let trial = Trial::new("A-01-f01-missing-tools")?;
    let state_file = trial.root.join("client-home").join(CLAUDE_STATE_FILE);
    fs::write(
        &state_file,
        "{\n  \"userID\": \"kept\",\n  \"projects\": {\n    \"elsewhere\": {\"hasTrustDialogAccepted\": false}\n  }\n}\n",
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
    let state: Value = serde_json::from_str(&fs::read_to_string(&state_file)?)?;
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
    fs::remove_dir_all(trial.root.join("client-home"))?;
    fs::remove_file(
        trial
            .layout
            .workspace()
            .join(".stub")
            .join("invocation.json"),
    )?;
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
        "tools.view_image=false",
    ] {
        assert!(arguments.iter().any(|argument| argument == flag), "{flag}");
    }
    // Without a Windows sandbox mode codex-cli 0.155 rejects every command
    // ("blocked by policy", the first dry trial).
    assert_eq!(
        arguments
            .iter()
            .any(|argument| argument == CODEX_WINDOWS_SANDBOX),
        cfg!(windows)
    );
    assert!(record.client_setup.is_empty());
    assert!(
        !trial
            .root
            .join("client-home")
            .join(CLAUDE_STATE_FILE)
            .exists(),
        "Codex runs never touch a Claude Code state file"
    );
    let session_root = trial.layout.session_root().to_string_lossy().into_owned();
    assert!(arguments.iter().any(|argument| {
        argument.starts_with("sandbox_workspace_write.writable_roots=")
            && argument.contains(&session_root)
    }));
    let names: Vec<String> = serde_json::from_value(invocation["environment_names"].clone())?;
    assert!(names.iter().any(|name| name == "CODEX_HOME"));
    assert!(!names.iter().any(|name| name == "CLAUDE_CONFIG_DIR"));
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
    let manifest: TrialManifest =
        serde_json::from_str(&fs::read_to_string(trial.layout.manifest())?)?;
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
