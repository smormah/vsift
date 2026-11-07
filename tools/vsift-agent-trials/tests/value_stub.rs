//! The value measurement against the stand-in client: `value prepare` for
//! every arm, `run` with the task's budget and the value `PATH` rule, the
//! arrival times, the two-phase repeat's `answer-1.md`, the grade, the
//! bounded record, the committed task files and the value freeze. No prompt
//! is sent anywhere and no media tool runs: the tools are stand-in files.

mod common;

use std::{
    error::Error,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use common::{Scratch, repository};
use serde_json::{Value, json};
use vsift_agent_trials::{
    layout::TrialLayout,
    roots::RootPolicy,
    run::{RunRequest, client_environment, read_manifest, run},
    trace::ClientKind,
    truth::CorpusTruth,
    value::{
        freeze,
        grade::grade_phase,
        prepare::{ValuePrepareRequest, prepare},
        summary::write_record,
        task::{Arm, EARLIER_ANSWER, EARLIER_ANSWER_FILE, Task},
    },
};

type TestResult = Result<(), Box<dyn Error>>;

const STUB: &str = env!("CARGO_BIN_EXE_vsift-trials-stub-client");

fn tasks_folder() -> PathBuf {
    repository().join(freeze::VALUE_TASKS)
}

/// The final answer the stand-in gives.
const ANSWER: &str = "From 0:07 to 0:10 order 1017 shows queued (frame frames/f-008.png). \
                      At 0:11 order 1017 changes to failed (frame frames/f-011.png).";

/// A short Claude Code stream: one `ffprobe` call, then the answer.
fn stream(answer: &str) -> String {
    [
        json!({"type": "system", "subtype": "init", "model": "compact-model", "claude_code_version": "stub"}),
        json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Bash",
               "input": {"command": "ffprobe -v error -show_format recording.mp4"}}]}}),
        json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ok"}]}}),
        json!({"type": "result", "subtype": "success", "result": answer, "num_turns": 2,
               "duration_ms": 1_000, "duration_api_ms": 800, "permission_denials": []}),
    ]
    .iter()
    .map(Value::to_string)
    .collect::<Vec<_>>()
    .join("\n")
}

/// Everything one test's trials share: stand-in tools, a model, a guide and
/// a client home, all in one scratch folder.
struct Bench {
    scratch: Scratch,
    root: PathBuf,
}

impl Bench {
    fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let scratch = Scratch::new(label)?;
        for tool in ["ffmpeg", "ffprobe", "whisper-cli"] {
            scratch.write(&format!("bin/{tool}"), "stand-in tool")?;
        }
        scratch.write("model/ggml-base.bin", "stand-in model")?;
        scratch.write(
            "guide/SKILL.md",
            "---\nname: recording-evidence\ndescription: Answer questions about recordings.\n---\nWork in one folder.\n",
        )?;
        fs::create_dir_all(scratch.path().join("client-home"))?;
        let root = scratch.path().join("trials");
        fs::create_dir_all(&root)?;
        Ok(Self { scratch, root })
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.scratch.path().join(relative)
    }

    fn request(&self, task: &Path, arm: Arm) -> ValuePrepareRequest {
        ValuePrepareRequest {
            root: self.root.clone(),
            repository: repository(),
            task: task.to_path_buf(),
            arm,
            run_index: 1,
            pilot: false,
            vsift: arm.has_vsift().then(|| PathBuf::from(STUB)),
            install: None,
            vsift_commit: "0".repeat(40),
            ffmpeg: self.path("bin/ffmpeg"),
            ffprobe: self.path("bin/ffprobe"),
            whisper: self.path("bin/whisper-cli"),
            model: self.path("model/ggml-base.bin"),
            media: None,
            transcript: None,
            guide: self.path("guide"),
            freeze_sha256: None,
            root_policy: RootPolicy::new(Vec::new(), Vec::new()),
            cold_scan_stop: self.scratch.parent(),
        }
    }

    fn run_request(&self, layout: &TrialLayout, phase: usize) -> RunRequest {
        RunRequest {
            trial: layout.trial().to_path_buf(),
            phase,
            client: ClientKind::ClaudeCode,
            executable: PathBuf::from(STUB),
            model: "compact-model".to_owned(),
            client_home: self.path("client-home"),
            // Both are replaced by the task's budget.
            timeout: Duration::from_secs(30),
            max_turns: 7,
            path_directories: Vec::new(),
            pass_environment: Vec::new(),
            system_path: Vec::new(),
            root_policy: RootPolicy::new(Vec::new(), Vec::new()),
            debug_prompt: None,
            cold_scan_stop: self.scratch.parent(),
        }
    }

    /// The stream the stand-in replays, kept outside the workspace so that
    /// a cold arm's workspace stays cold.
    fn behave(&self, layout: &TrialLayout, answer: &str, name: &str) -> TestResult {
        let replay = self.scratch.write(&format!("replays/{name}.jsonl"), &stream(answer))?;
        let stub = layout.workspace().join(".stub");
        fs::create_dir_all(&stub)?;
        fs::write(
            stub.join("behaviour.json"),
            json!({"replay_file": replay, "exit_code": 0}).to_string(),
        )?;
        Ok(())
    }
}

fn path_of(environment: &[(String, OsString)]) -> Vec<PathBuf> {
    environment
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| std::env::split_paths(value).collect())
        .unwrap_or_default()
}

fn line_count(path: &Path) -> Result<usize, Box<dyn Error>> {
    Ok(fs::read_to_string(path)?.lines().filter(|line| !line.is_empty()).count())
}

#[tokio::test]
async fn every_arm_runs_with_the_tools_on_path_the_task_budget_and_arrival_times() -> TestResult {
    let bench = Bench::new("value-arms")?;
    let task = tasks_folder().join("T2-f04-order-1017.json");
    let stub_folder = Path::new(STUB).parent().ok_or("no stub folder")?.to_path_buf();
    for arm in Arm::ALL {
        let layout = prepare(&bench.request(&task, arm))?;
        let manifest = read_manifest(&layout)?;
        let value = manifest.value.clone().ok_or("no value manifest")?;
        assert_eq!(value.arm, arm);
        assert_eq!(value.guide_name, "recording-evidence");
        assert_eq!(value.guide_sha256.len(), 64, "every arm names the guide");
        assert_eq!(value.method_sha256.len(), 64, "every arm names the method note");
        assert!(layout.workspace().join("models").join("ggml-base.bin").is_file());
        assert!(layout.workspace().join("recording.mp4").is_file());
        assert!(layout.workspace().join("recording.srt").is_file());
        let skills = layout.workspace().join(".claude").join("skills");
        assert_eq!(skills.join("recording-evidence").is_dir(), arm == Arm::R1, "{arm:?}");
        assert_eq!(skills.join("vsift").is_dir(), arm == Arm::V1, "{arm:?}");

        bench.behave(&layout, ANSWER, arm.name())?;
        let request = bench.run_request(&layout, 1);
        let path = path_of(&client_environment(&request, &layout, &manifest)?);
        assert!(path.contains(&value.tools_directory), "{arm:?}: tools folder not on PATH");
        assert_eq!(path.contains(&stub_folder), arm.has_vsift(), "{arm:?}: vsift's folder");

        let record = run(&request).await?;
        let max_turns = record
            .arguments
            .iter()
            .position(|argument| argument == "--max-turns")
            .and_then(|index| record.arguments.get(index + 1))
            .ok_or("no --max-turns")?;
        assert_eq!(max_turns, "60", "{arm:?}: the task's budget replaces the operator's");
        let times = record.stdout.with_file_name("stdout.times");
        assert_eq!(line_count(&times)?, line_count(&record.stdout)?, "{arm:?}");

        let grade = grade_phase(&layout, 1, &repository())?;
        assert!(grade.counted, "{arm:?}: {:?}", grade.excluded_because);
        let claims = grade.claims.as_ref().ok_or("no claim grade")?;
        assert!(claims.score > 0.0, "{arm:?}: {claims:?}");
        assert_eq!(grade.measures.media_tool_calls, 1);
        assert_eq!(grade.measures.round_trips, 2);
        assert!(grade.safety.is_empty(), "{arm:?}: {:?}", grade.safety);

        // The bounded record names no local path.
        let output = bench.path(&format!("records/{}.json", arm.name()));
        let client_home = bench.path("client-home");
        write_record(&layout, 1, &output, Some(client_home.as_path()), &[])?;
        let text = fs::read_to_string(&output)?;
        for local in [bench.scratch.path().to_path_buf(), value.tools_directory.clone()] {
            let local = local.to_string_lossy().into_owned();
            assert!(
                !text.contains(&local) && !text.contains(&local.replace('\\', "\\\\")),
                "{arm:?}: the record names a local path"
            );
        }
        assert!(text.contains("vsift-value-record/1"));
    }
    Ok(())
}

#[test]
fn a_trial_without_the_guide_is_refused_in_every_arm() -> TestResult {
    let bench = Bench::new("value-no-guide")?;
    let task = tasks_folder().join("T2-f04-order-1017.json");
    for arm in Arm::ALL {
        let mut request = bench.request(&task, arm);
        request.guide = bench.path("no-guide");
        assert!(prepare(&request).is_err(), "{arm:?}");
    }
    Ok(())
}

#[tokio::test]
async fn the_second_phase_finds_the_first_answer_in_the_workspace() -> TestResult {
    let bench = Bench::new("value-repeat")?;
    let phase = json!({
        "question": "In {{video}}, what happens to order 1017, and when?",
        "truth": {"kind": "events", "events": [{"id": "F04-E03"}, {"id": "F04-E04"}]}
    });
    let task = bench.scratch.write(
        "tasks/T9-repeat.json",
        &json!({
            "id": "T9-repeat", "task": "T7", "kind": "second_question",
            "media": {"source": "corpus", "fixture": "F04", "file": "F04-speech.mp4"},
            "transcript": null,
            "phases": [phase, phase],
            "budget": {"timeout_s": 900, "max_turns": 60}
        })
        .to_string(),
    )?;
    let layout = prepare(&bench.request(&task, Arm::R0))?;
    bench.behave(&layout, ANSWER, "phase-1")?;
    run(&bench.run_request(&layout, 1)).await?;
    let earlier = layout.workspace().join(EARLIER_ANSWER_FILE);
    assert!(!earlier.exists(), "no earlier answer before phase 2");

    bench.behave(&layout, "At 0:11 order 1017 changes to failed (frame frames/f-011.png).", "phase-2")?;
    let second = run(&bench.run_request(&layout, 2)).await?;
    assert_eq!(fs::read_to_string(&earlier)?.trim_end(), ANSWER);
    let prompt = second.arguments.iter().find(|argument| argument.contains(EARLIER_ANSWER));
    assert!(prompt.is_some(), "phase 2's prompt names the earlier answer");
    let grade = grade_phase(&layout, 2, &repository())?;
    assert_eq!(grade.phase, 2);
    assert!(grade.claims.is_some());
    Ok(())
}

#[test]
fn the_committed_task_files_are_valid_and_only_the_corpus_tasks_may_be_counted() -> TestResult {
    let corpus = CorpusTruth::load(&repository().join("fixtures").join("corpus"))?;
    let mut files: Vec<PathBuf> = fs::read_dir(tasks_folder())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
        .collect();
    files.sort();
    assert_eq!(files.len(), 9);
    for file in &files {
        let task = Task::load(file)?;
        task.validate(&corpus, false)?;
        let corpus_task = ["T1", "T2", "T3"].contains(&task.task.as_str());
        assert_eq!(task.validate(&corpus, true).is_ok(), corpus_task, "{}", task.id);
        let expected = if corpus_task { (900, 60) } else { (1_800, 80) };
        assert_eq!((task.budget.timeout_s, task.budget.max_turns), expected, "{}", task.id);
        let named = file.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
        assert_eq!(named, task.id);
    }
    Ok(())
}

/// A minimal repository holding every input of the value freeze.
fn freeze_repository(scratch: &Scratch) -> TestResult {
    for (relative, text) in [
        ("tools/vsift-agent-trials/value/tasks/T1.json", "{}"),
        ("tools/vsift-agent-trials/value/raw-guide/SKILL.md", "---\nname: guide\n---\n"),
        ("tools/vsift-agent-trials/claude-value-trial-settings.json", "{}"),
        ("tools/vsift-agent-trials/src/lib.rs", "//! grader\n"),
        ("tools/vsift-agent-trials/Cargo.toml", "[package]\n"),
        ("docs/planning/value-measurement/method.md", "# Method\n"),
        ("skills/vsift/SKILL.md", "---\nname: vsift\n---\n"),
        ("fixtures/corpus/manifest.json", "{}"),
        ("fixtures/corpus/generated/speech-provenance.json", "{}"),
    ] {
        scratch.write(relative, text)?;
    }
    Ok(())
}

#[test]
fn the_value_freeze_holds_until_a_frozen_input_moves() -> TestResult {
    let scratch = Scratch::new("value-freeze")?;
    freeze_repository(&scratch)?;
    let file = scratch.path().join("value-freeze.json");
    let written = freeze::write(scratch.path(), "0123456789ab", &file)?;
    assert_eq!(written.components.len(), freeze::VALUE_COMPONENTS.len());
    assert_eq!(freeze::check(scratch.path(), &file)?, Vec::<String>::new());

    for (relative, component) in [
        ("tools/vsift-agent-trials/value/raw-guide/SKILL.md", "raw_guide"),
        ("docs/planning/value-measurement/method.md", "method"),
        ("tools/vsift-agent-trials/src/lib.rs", "grader"),
        ("tools/vsift-agent-trials/value/tasks/T1.json", "tasks"),
    ] {
        scratch.write(relative, "changed\n")?;
        let problems = freeze::check(scratch.path(), &file)?;
        assert!(
            problems.iter().any(|problem| problem.starts_with(component)),
            "{component}: {problems:?}"
        );
    }

    fs::remove_file(scratch.path().join("tools/vsift-agent-trials/value/raw-guide/SKILL.md"))?;
    assert!(freeze::compute(scratch.path(), "x").is_err(), "no freeze without the guide");
    Ok(())
}
