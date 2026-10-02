//! Opt-in P12 checkpoint: the skill's documented procedure, walked
//! deterministically against real tools and graded by the trial grader.
//!
//! **This is not an agent trial.** No model runs: a fixed program follows
//! the command sequence `skills/vsift/SKILL.md` documents for the A-08
//! (local speech recognition) and A-09 (supplied transcript) journeys,
//! exactly as an agent would type each command, builds a handoff from the
//! results, and hands the resulting trace to the same grader the
//! named-client trials use. It proves that the documented procedure,
//! the harness's preparation and the grader agree with the real CLI; the
//! named-client trials (P12 PR 3) remain the qualification evidence.
//!
//! The walker cannot see images, so it records `image_access`
//! `unavailable` and cites frames with `pixels_inspected` false, as the
//! skill requires of a client without image access.
//!
//! ```console
//! VSIFT_P12_TRIAL_ROOT=C:\vsift-trials
//! VSIFT_TEST_VSIFT_BIN=<absolute path to a release vsift executable>
//! VSIFT_TEST_WHISPER_CLI=<absolute whisper-cli path>
//! VSIFT_TEST_WHISPER_MODEL=<absolute ggml-base.bin path>
//! cargo test -p vsift-agent-trials --locked --test p12_skill_procedure_e2e -- --ignored --nocapture
//! ```
//!
//! `FFmpeg` and `FFprobe` must be on `PATH` (they are registered into each
//! trial's isolated per-user base, never used from `PATH` by `vsift`). The
//! trial root must be neutral: outside the user's profile and without the
//! user name in its path. It writes `.vsift/e2e-runs/p12-<run-id>/report.json`
//! and prints `p12_skill_procedure: passed` when every stage passed; a stage
//! that cannot run is `blocked`.

use std::{
    env,
    error::Error,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use vsift_agent_trials::{
    evaluate::{RunFindings, environment_user_names, grade_trace},
    layout::{ToolSource, TrialLayout},
    prepare::{PrepareRequest, prepare},
    roots::RootPolicy,
    run::read_manifest,
    trace::{CallKind, ToolCall, Trace},
    vsift_cli::VsiftCli,
};

type TestResult = Result<(), Box<dyn Error>>;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn absolute_file(variable: &str) -> Option<PathBuf> {
    env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path).find_map(|directory| {
        [name.to_owned(), format!("{name}.exe")]
            .into_iter()
            .map(|file| directory.join(file))
            .find(|candidate| candidate.is_file())
    })
}

/// Everything the checkpoint needs from the machine.
struct Machine {
    root: PathBuf,
    vsift: PathBuf,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    whisper: PathBuf,
    model: PathBuf,
}

impl Machine {
    fn discover() -> Result<Self, String> {
        let root = env::var_os("VSIFT_P12_TRIAL_ROOT")
            .map(PathBuf::from)
            .ok_or("set VSIFT_P12_TRIAL_ROOT to a neutral trial root such as C:\\vsift-trials")?;
        let missing = |what: &str| format!("{what} is missing");
        Ok(Self {
            root,
            vsift: absolute_file("VSIFT_TEST_VSIFT_BIN")
                .ok_or_else(|| missing("VSIFT_TEST_VSIFT_BIN"))?,
            ffmpeg: on_path("ffmpeg").ok_or_else(|| missing("ffmpeg on PATH"))?,
            ffprobe: on_path("ffprobe").ok_or_else(|| missing("ffprobe on PATH"))?,
            whisper: absolute_file("VSIFT_TEST_WHISPER_CLI")
                .ok_or_else(|| missing("VSIFT_TEST_WHISPER_CLI"))?,
            model: absolute_file("VSIFT_TEST_WHISPER_MODEL")
                .ok_or_else(|| missing("VSIFT_TEST_WHISPER_MODEL"))?,
        })
    }
}

/// The walker: runs each documented command, records it as the command
/// text an agent would type, and keeps the results.
struct Walker {
    cli: VsiftCli,
    trace: Trace,
}

fn quote(argument: &str) -> String {
    if argument.is_empty()
        || argument
            .chars()
            .any(|value| value.is_whitespace() || "\"'$`|&;<>(){}".contains(value))
    {
        format!("\"{argument}\"")
    } else {
        argument.to_owned()
    }
}

impl Walker {
    fn record(&mut self, arguments: &[String]) {
        let text = std::iter::once("vsift".to_owned())
            .chain(arguments.iter().map(|argument| quote(argument)))
            .collect::<Vec<_>>()
            .join(" ");
        let index = self.trace.calls.len();
        self.trace.calls.push(ToolCall {
            index,
            step: index,
            id: format!("walk_{index}"),
            kind: CallKind::Shell { command: text },
            input: Value::Null,
            denied: false,
            exit_code: Some(0),
            is_error: false,
            completed: true,
            output: None,
        });
    }

    /// One `--json` command that must succeed.
    fn json(&mut self, arguments: &[String]) -> Result<Value, Box<dyn Error>> {
        self.record(arguments);
        let os: Vec<OsString> = arguments.iter().map(OsString::from).collect();
        Ok(self.cli.ok(&os)?)
    }

    /// One `--events jsonl` command; its terminal event's result.
    fn terminal(&mut self, arguments: &[String]) -> Result<Value, Box<dyn Error>> {
        self.record(arguments);
        let os: Vec<OsString> = arguments.iter().map(OsString::from).collect();
        let (code, stdout) = self.cli.stdout(&os)?;
        let last = stdout.lines().last().ok_or("no terminal event")?;
        let event: Value = serde_json::from_str(last)?;
        if code != Some(0) || event["event"] != "terminal" {
            return Err(
                format!("{arguments:?} failed: {}", event["result"]["error"]["code"]).into(),
            );
        }
        Ok(event["result"].clone())
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn text(value: &Value) -> Result<String, Box<dyn Error>> {
    Ok(value.as_str().ok_or("expected text")?.to_owned())
}

fn number(value: &Value) -> Result<u64, Box<dyn Error>> {
    Ok(value.as_u64().ok_or("expected a number")?)
}

/// The skill's A-08 or A-09 journey over F05: capabilities, prepare,
/// spoken spans, cards, the source frame, report, retain.
#[allow(
    clippy::too_many_lines,
    reason = "The procedure's states read best in order, in one place"
)]
fn walk(layout: &TrialLayout, local_asr: bool) -> Result<Value, Box<dyn Error>> {
    let manifest = read_manifest(layout)?;
    let mut walker = Walker {
        cli: VsiftCli::new(&manifest.vsift_executable, layout)?,
        trace: Trace::default(),
    };
    let started = Instant::now();

    // CHECK_CAPABILITIES: the walker cannot see images.
    let check = walker.json(&strings(&["setup", "check", "--json"]))?;
    let asr_status = text(&check["local_asr"]["verification"]["status"])
        .unwrap_or_else(|_| "not_checked".to_owned());

    // PREPARE.
    let mut ingest = strings(&["ingest", &manifest.video]);
    if let Some(transcript) = &manifest.transcript {
        ingest.extend(strings(&[
            "--transcript",
            transcript,
            "--transcript-offset",
            "0",
        ]));
    }
    ingest.push("--json".to_owned());
    let opened = walker.json(&ingest)?;
    let session = text(&opened["data"]["session_id"])?;
    let source = text(&opened["data"]["source_id"])?;
    let (revision, duration) = if local_asr {
        let digits: String = session
            .trim_start_matches("ses_")
            .chars()
            .take(24)
            .collect();
        let result = walker.terminal(&strings(&[
            "transcript",
            "retranscribe",
            &session,
            "--operation-id",
            &format!("op_retx{digits}01"),
            "--events",
            "jsonl",
        ]))?;
        let revision = &result["data"]["revision"];
        (
            text(&revision["revision_id"])?,
            revision["source_segments"][0]["end_us"].as_u64(),
        )
    } else {
        let transcript = &opened["data"]["transcript"];
        (
            text(&transcript["revision_id"])?,
            transcript["source_segments"][0]["end_us"].as_u64(),
        )
    };

    // FIND_SPOKEN_SPANS: the error code, then its context.
    let found = walker.json(&strings(&[
        "search", &session, "--query", "E-409", "--limit", "5", "--json",
    ]))?;
    let hit = found["data"]["items"][0].clone();
    let (start, end) = (number(&hit["start_us"])?, number(&hit["end_us"])?);
    let context = walker.json(&strings(&[
        "transcript",
        "get",
        &session,
        "--from",
        &start.saturating_sub(15_000_000).to_string(),
        "--to",
        &(end + 15_000_000).to_string(),
        "--limit",
        "20",
        "--json",
    ]))?;
    let expectation = context["data"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|item| {
            item["text"]
                .as_str()
                .is_some_and(|value| value.to_lowercase().contains("banner"))
        })
        .cloned()
        .unwrap_or_else(|| hit.clone());

    // INSPECT_CARDS: lead and lag around the span; the first visual change
    // at or after the span's start.
    let cards = walker.json(&strings(&[
        "candidates",
        &session,
        "--from",
        &start.saturating_sub(5_000_000).to_string(),
        "--to",
        &(end + 10_000_000).to_string(),
        "--limit",
        "20",
        "--json",
    ]))?;
    let items = cards["data"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let changed = |item: &&Value| {
        item["reasons"].as_array().is_some_and(|reasons| {
            reasons
                .iter()
                .any(|reason| reason == "visual_change" || reason == "settled_after_motion")
        })
    };
    let card = items
        .iter()
        .filter(|item| {
            item["representative_us"]
                .as_u64()
                .is_some_and(|time| time >= start)
        })
        .find(changed)
        .or_else(|| items.first())
        .ok_or("no candidate around the span")?
        .clone();
    let candidate = text(&card["candidate_id"])?;

    // VERIFY_SOURCE: the candidate's own frame (not viewed: no image access).
    let frame = walker.json(&strings(&[
        "frame",
        "get",
        &session,
        "--candidate",
        &candidate,
        "--json",
    ]))?;
    let selection = frame["data"]["selections"][0].clone();

    // CLOSE_OR_RETAIN, as the scenario's user asked.
    let retained = walker.json(&strings(&[
        "session",
        "retain",
        &session,
        "--output",
        "evidence-bundle-phase-1",
        "--json",
    ]))?;
    let elapsed = started.elapsed().as_secs();

    let segment_citation = |id: &str, item: &Value| {
        json!({"id": id, "type": "transcript_segment", "revision_id": revision, "segment_id": item["segment_id"],
               "start_us": item["start_us"], "end_us": item["end_us"]})
    };
    let mut citations = vec![segment_citation("e1", &hit)];
    let mut expected_refs = vec!["e1"];
    if expectation["segment_id"] != hit["segment_id"] {
        citations.push(segment_citation("e3", &expectation));
        expected_refs = vec!["e3"];
    }
    citations.push(json!({"id": "e2", "type": "frame", "evidence_id": selection["evidence_id"],
        "candidate_id": candidate, "requested_us": selection["requested_us"], "actual_us": selection["actual_us"],
        "delta_us": selection["delta_us"], "pixels_inspected": false}));
    let calls = u64::try_from(walker.trace.calls.len())?;
    let handoff = json!({
        "handoff_version": "1",
        "status": "complete",
        "question": "What goes wrong when the invoice is submitted?",
        "capabilities": {
            "image_access": "unavailable",
            "image_check_code": null,
            "media_tools": "available",
            "local_asr": match asr_status.as_str() {
                "verified" | "failed" | "not_run" => asr_status.as_str(),
                _ => "not_checked",
            },
            "transcript_basis": if local_asr { "local_asr" } else { "supplied_transcript" }
        },
        "session": {"session_id": session, "source_id": source, "revision_id": revision, "duration_us": duration},
        "claims": [
            {"id": "c1", "section": "actual", "kind": "observed", "support": "supported", "certainty": "medium",
             "statement": "The speaker says the page shows error E-409 and leaves Submit enabled.", "citations": ["e1"]},
            {"id": "c2", "section": "expected", "kind": "observed", "support": "supported", "certainty": "medium",
             "statement": "The speaker says a success banner was expected.", "citations": expected_refs},
            {"id": "c3", "section": "actual", "kind": "inferred", "support": "unsupported", "certainty": "low",
             "statement": "What the screen shows after the submission was not viewed, because image access is unavailable.",
             "citations": ["e2"]}
        ],
        "citations": citations,
        "gaps": [{"kind": "image_access", "range": null, "reason": "image_access_unavailable", "code": null,
                  "note": "The procedure walker cannot open images, so no frame was viewed."}],
        "untrusted_instructions": [],
        "budget": {"profile": "compact", "overrides": false,
                   "limits": {"images_per_step": 1, "images_total": 6, "image_bytes": 12_582_912, "page_limit": 20,
                              "tool_calls": 30, "refinement_depth": 2, "wall_time_s": 900, "burst_frames": 4},
                   "used": {"images_total": 0, "image_bytes": 0, "tool_calls": calls, "refinement_depth": 0,
                            "wall_time_s": elapsed},
                   "exhausted": []},
        "lifecycle": {"policy": "user_stated", "action": "retained", "mode": retained["lifecycle"]["mode"],
                      "expires_at": retained["lifecycle"]["expires_at"]},
        "resume": null
    });
    walker.trace.final_message = Some(format!(
        "## Problem\n\nSubmitting the invoice shows an error [c1].\n\n## Expected\n\nA success banner [c2].\n\n\
         ## Actual\n\nAn error code is announced [c1]; the screen was not viewed [c3].\n\n\
         ## Reproduction steps\n\n1. Submit the invoice [c1].\n\n## Evidence\n\nSee the citations.\n\n\
         ## Gaps and uncertainty\n\nNo image access.\n\n## Untrusted instructions observed\n\nNone observed.\n\n\
         ## Lifecycle\n\nRetained.\n\n```vsift-handoff\n{}\n```\n",
        serde_json::to_string_pretty(&handoff)?
    ));
    let graded = grade_trace(
        layout,
        1,
        &walker.trace,
        "",
        Some(elapsed),
        &environment_user_names(),
        RunFindings::default(),
    )?;
    let failed: Vec<Value> = graded
        .mechanical
        .checks
        .iter()
        .filter(|check| !check.passed)
        .map(|check| json!({"name": check.name, "details": check.details}))
        .collect();
    let evidence = json!({
        "mechanical": graded.mechanical.passed,
        "failed_checks": failed,
        "interpretation": graded.interpretation.passed,
        "key_facts": graded.interpretation.key_facts,
        "tool_calls": graded.usage.tool_calls,
        "deviations": graded.deviations,
        "search_hit_us": [start, end],
        "frame_actual_us": selection["actual_us"],
        "elapsed_s": elapsed,
    });
    if graded.mechanical.passed && graded.interpretation.passed {
        Ok(evidence)
    } else {
        Err(format!("the grader failed the walked procedure: {evidence}").into())
    }
}

async fn stage(
    machine: &Machine,
    scenario: &str,
    local_asr: bool,
) -> Result<Value, Box<dyn Error>> {
    let layout = prepare(&PrepareRequest {
        root: machine.root.clone(),
        repository: repository(),
        scenario: repository()
            .join("tools/vsift-agent-trials/scenarios")
            .join(format!("{scenario}.json")),
        vsift: machine.vsift.clone(),
        vsift_commit: env::var("VSIFT_TEST_COMMIT").unwrap_or_else(|_| "unrecorded".to_owned()),
        ffmpeg: Some(machine.ffmpeg.clone()),
        ffprobe: Some(machine.ffprobe.clone()),
        whisper: Some(machine.whisper.clone()),
        model: Some(machine.model.clone()),
        root_policy: RootPolicy::from_environment(),
        install: None,
        tools: ToolSource::Registered,
        freeze_sha256: None,
        cold_scan_stop: None,
    })
    .await?;
    let result = walk(&layout, local_asr);
    let _ = fs::remove_dir_all(layout.trial());
    result
}

#[tokio::test]
#[ignore = "opt-in P12 procedure checkpoint; needs VSIFT_P12_TRIAL_ROOT, VSIFT_TEST_VSIFT_BIN, whisper and FFmpeg; not an agent trial"]
async fn skill_procedure_checkpoint() -> TestResult {
    let started = Instant::now();
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_dir = repository()
        .canonicalize()?
        .join(".vsift/e2e-runs")
        .join(format!("p12-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&run_dir)?;
    let mut stages = Vec::new();
    match Machine::discover() {
        Ok(machine) => {
            for (name, scenario, local_asr) in [
                ("p12_procedure_a08_local_asr", "A-08-f05-local-asr", true),
                ("p12_procedure_a09_supplied", "A-09-f05-supplied", false),
            ] {
                let clock = Instant::now();
                let outcome = stage(&machine, scenario, local_asr).await;
                let elapsed_ms = clock.elapsed().as_millis();
                stages.push(match outcome {
                    Ok(evidence) => json!({"name": name, "status": "passed", "elapsed_ms": elapsed_ms, "evidence": evidence}),
                    Err(error) => json!({"name": name, "status": "failed", "elapsed_ms": elapsed_ms,
                                         "diagnostic": error.to_string().chars().take(4_000).collect::<String>()}),
                });
            }
        }
        Err(remediation) => {
            for name in ["p12_procedure_a08_local_asr", "p12_procedure_a09_supplied"] {
                stages.push(json!({"name": name, "status": "blocked", "remediation": remediation}));
            }
        }
    }
    let status = |wanted: &str| stages.iter().any(|entry| entry["status"] == wanted);
    let overall = if status("failed") {
        "failed"
    } else if status("blocked") {
        "blocked"
    } else {
        "passed"
    };
    let report = json!({
        "schema_version": 1,
        "checkpoint": "P12 skill procedure (deterministic walker, not an agent trial)",
        "os": env::consts::OS,
        "architecture": env::consts::ARCH,
        "stages": stages,
        "overall": overall,
        "agent_trials": "not_run: named-client trials are P12 PR 3",
        "elapsed_ms": started.elapsed().as_millis(),
    });
    let report_path = run_dir.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("P12 skill procedure report: {}", report_path.display());
    println!("p12_skill_procedure: {overall}");
    if overall == "passed" {
        Ok(())
    } else {
        Err(format!(
            "P12 skill procedure checkpoint {overall}; see {}",
            report_path.display()
        )
        .into())
    }
}
