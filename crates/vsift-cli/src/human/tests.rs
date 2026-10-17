//! Renderer tests over the frozen v1 examples, and the golden snapshots
//! that let a reviewer read what each command prints.
//!
//! The snapshots in `crates/vsift-cli/tests/human_output/` are for
//! readability review only; human text is not a contract. Set
//! `VSIFT_UPDATE_HUMAN_SNAPSHOTS=1` to rewrite them after a deliberate
//! change, then review the diff.

use std::{env, fs, path::PathBuf, time::Duration};

use serde_json::{Value, json};
use vsift::{
    DependencyState, DependencyStatus, LocalAsrCheckOutcome, LocalAsrModelStatus,
    LocalAsrNotRunReason, LocalAsrSetupStatus, ManagedPlanAvailability, ProbeTimeout,
    ProbeTimeoutKind, RuntimeDependency, RuntimeDiagnosis, RuntimeReadiness,
};
use vsift_contract::{
    CommandName, ConfiguredModelResponse, ConfiguredSelectionResponse, DependencyLookup,
    MEDIA_BUSY_REMEDIATION, OperationResponse, is_hidden_character,
};

use super::{
    HumanDetail, evidence, failure::render_failure, render_host, render_value, result, setup_check,
    text::DisplayText,
};
use crate::command::ExecutionProfile;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn example(name: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(repository(
        &format!("schemas/v1/examples/{name}"),
    ))?)?)
}

/// Compares `text` with the snapshot `name`, or rewrites it on request.
pub(crate) fn check_snapshot(name: &str, text: &str) -> TestResult {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/human_output")
        .join(format!("{name}.txt"));
    if env::var_os("VSIFT_UPDATE_HUMAN_SNAPSHOTS").is_some() {
        fs::write(&path, text)?;
        return Ok(());
    }
    let expected = fs::read_to_string(&path)
        .map_err(|error| {
            format!("snapshot {name} unreadable ({error}); set VSIFT_UPDATE_HUMAN_SNAPSHOTS=1")
        })?
        .replace("\r\n", "\n");
    assert_eq!(text, expected, "snapshot {name} differs");
    Ok(())
}

/// Asserts the terminal rules on `text`: no control character but line
/// breaks, no hidden character, and no line over the diagnostic budget.
fn assert_terminal_safe(text: &str) {
    for character in text.chars() {
        assert!(
            character == '\n' || !character.is_control(),
            "control U+{:04X}",
            u32::from(character)
        );
        assert!(
            !is_hidden_character(character),
            "hidden U+{:04X}",
            u32::from(character)
        );
    }
    for line in text.lines() {
        assert!(line.len() < 4_096, "a line of {} bytes", line.len());
    }
}

fn render(command: CommandName, value: &Value) -> Result<String, Box<dyn std::error::Error>> {
    let rendered = render_value(command, value)?.ok_or("no renderer")?;
    let text = rendered.as_str().to_owned();
    assert_terminal_safe(&text);
    Ok(text)
}

/// #353: the list of what a run did not transcribe says that earlier text is
/// kept only when there was an earlier revision to keep it from. A first run has
/// nothing to keep; a run over an earlier revision says what the rule is, and
/// that there may be none, because the result does not count what was kept.
#[test]
fn the_untranscribed_label_speaks_of_earlier_text_only_after_an_earlier_revision() -> TestResult {
    let first = example("transcript-retranscribe.partial.json")?;
    assert!(first["data"]["revision"]["supersedes"].is_null());
    let text = render(CommandName::TranscriptRetranscribe, &first)?;
    assert!(
        text.contains("Not transcribed by this run (the answer for these parts could not be used; words said there cannot be found):\n"),
        "{text}"
    );
    assert!(!text.contains("earlier text"), "{text}");

    let mut spliced = first;
    spliced["data"]["revision"]["supersedes"] = Value::from("trv_2222222222222222");
    let text = render(CommandName::TranscriptRetranscribe, &spliced)?;
    assert!(
        text.contains("earlier text that reaches into them, if any, is kept"),
        "{text}"
    );
    // The label ends in the colon, with no space before the line break.
    assert!(!text.contains(": \n"), "{text}");
    Ok(())
}

/// #359: a revision can list up to 1,024 stretches where words that no segment
/// holds may lie, and the text prints at most 100 of them, as a search prints at
/// most 100 of each kind of range, then says how many more there are and where
/// to read them. The label says in plain words what a stretch is: the longest
/// the missing words could cover.
#[test]
fn many_unplaced_stretches_are_cut_where_a_search_cuts_its_ranges() -> TestResult {
    let mut value = example("transcript-retranscribe.unplaced.json")?;
    let stretches: Vec<Value> = (0..130_u64)
        .map(|index| {
            serde_json::json!({
                "from_us": 1_000_000 + index * 10_000_000,
                "to_us": 3_000_000 + index * 10_000_000
            })
        })
        .collect();
    value["data"]["revision"]["local_asr"]["unplaced_ranges"] = Value::Array(stretches);
    let text = render(CommandName::TranscriptRetranscribe, &value)?;
    assert_eq!(text.matches("(--from ").count(), 100, "{text}");
    assert!(
        text.contains("  and 30 more (use --json for all of them)\n"),
        "{text}"
    );
    assert!(
        text.contains("Each stretch is the longest they could cover (they probably took less)"),
        "{text}"
    );
    // The label is for a reader who is not a developer.
    for jargon in ["recogniser", "segment", "upper bound"] {
        let label = text
            .lines()
            .find(|line| line.starts_with("Speech was heard"));
        assert!(
            label.is_some_and(|line| !line.contains(jargon)),
            "{jargon}: {text}"
        );
    }
    check_snapshot("transcript-retranscribe-unplaced-many", &text)?;
    // One short of the cut prints them all and says nothing of more.
    value["data"]["revision"]["local_asr"]["unplaced_ranges"] = Value::Array(
        (0..100_u64)
            .map(|index| {
                serde_json::json!({
                    "from_us": 1_000_000 + index * 10_000_000,
                    "to_us": 3_000_000 + index * 10_000_000
                })
            })
            .collect(),
    );
    let text = render(CommandName::TranscriptRetranscribe, &value)?;
    assert_eq!(text.matches("(--from ").count(), 100);
    assert!(!text.contains(" more (use --json"), "{text}");
    Ok(())
}

/// Every frozen example of a PR 2a command renders; its snapshot is the
/// text a person reads.
#[test]
fn frozen_examples_render_as_readable_text() -> TestResult {
    for (command, file, snapshot) in [
        (CommandName::Ingest, "ingest.transcript.json", "ingest"),
        (
            CommandName::TranscriptGet,
            "transcript-get.json",
            "transcript-get",
        ),
        (
            CommandName::TranscriptGet,
            "transcript-get.asr.json",
            "transcript-get-asr",
        ),
        (
            CommandName::TranscriptGet,
            "transcript-get.partial.json",
            "transcript-get-partial",
        ),
        (
            CommandName::TranscriptRetranscribe,
            "transcript-retranscribe.json",
            "transcript-retranscribe",
        ),
        (
            CommandName::TranscriptRetranscribe,
            "transcript-retranscribe.partial.json",
            "transcript-retranscribe-partial",
        ),
        (
            CommandName::TranscriptRetranscribe,
            "transcript-retranscribe.unplaced.json",
            "transcript-retranscribe-unplaced",
        ),
        (CommandName::Search, "search.json", "search"),
        (
            CommandName::SetupPlan,
            "setup-plan.unqualified.json",
            "setup-plan-unqualified",
        ),
        (
            CommandName::SetupPlan,
            "setup-plan.unavailable.json",
            "setup-plan-unavailable",
        ),
        (
            CommandName::SetupPlan,
            "setup-plan.timed-out.json",
            "setup-plan-timeout",
        ),
        (
            CommandName::SessionInitWorkspace,
            "workspace-init.json",
            "session-init-workspace",
        ),
    ] {
        let text = render(command, &example(file)?)?;
        check_snapshot(snapshot, &text)?;
    }
    Ok(())
}

/// What `setup check` prints for these probe results, with no model
/// registered, where managed installation is or is not available.
fn setup_check_text(
    states: [DependencyState; 3],
    readiness: RuntimeReadiness,
    managed_install: ManagedPlanAvailability,
) -> Result<String, Box<dyn std::error::Error>> {
    let diagnosis = RuntimeDiagnosis {
        readiness,
        dependencies: RuntimeDependency::ALL
            .into_iter()
            .zip(states)
            .map(|(dependency, state)| DependencyStatus { dependency, state })
            .collect(),
    };
    let rendered = setup_check(
        &diagnosis,
        LocalAsrSetupStatus {
            model: LocalAsrModelStatus::NotSelected,
            verification: LocalAsrCheckOutcome::NotRun(LocalAsrNotRunReason::ModelNotSelected),
        },
        managed_install,
        ExecutionProfile::Desktop,
        &|_| DependencyLookup::FilteredPath,
    )
    .map_err(|_| "the setup check is too large to render")?;
    let text = rendered.as_str().to_owned();
    assert_terminal_safe(&text);
    Ok(text)
}

fn timed_out(kind: ProbeTimeoutKind, seconds: u64) -> DependencyState {
    DependencyState::TimedOut(ProbeTimeout {
        kind,
        shared_deadline: Duration::from_secs(seconds),
    })
}

/// Words that tell a reader to get a tool or to point `VSift` at one. None
/// is true of a tool that was found. The advice for a missing tool was
/// "Install or locate" before issue #396 and is "Install a trusted build" and
/// `setup configure` since; both are listed, so a timed-out tool gets neither.
const INSTALL_ADVICE: [&str; 15] = [
    "Install or locate",
    "install or locate",
    "Install a trusted",
    "register a working one",
    "setup configure ",
    "<absolute-path>",
    "No need to search folders",
    "setup install",
    "setup plan",
    "reviewed build",
    "Managed installation",
    "absolute path",
    "--ffmpeg",
    "--ffprobe",
    "--whisper",
];

/// Issue #391: the first `setup check` after a restart reported an installed,
/// healthy `FFmpeg` and `FFprobe` as `[timeout]` and printed, under each, the
/// advice for a missing tool. The reported shape (the first tool does not
/// answer, the two after it are never started) now says what happened to
/// each, under which deadline, and what helps; it is still `blocked`, and
/// nothing in it says to install or locate a tool or offers to install one,
/// whether or not this machine has managed installation.
#[test]
fn a_tool_whose_probe_timed_out_is_not_told_to_install_or_locate_it() -> TestResult {
    let reported = || {
        [
            timed_out(ProbeTimeoutKind::NoAnswer, 30),
            timed_out(ProbeTimeoutKind::NotStarted, 30),
            timed_out(ProbeTimeoutKind::NotStarted, 30),
        ]
    };
    let text = setup_check_text(
        reported(),
        RuntimeReadiness::Blocked,
        ManagedPlanAvailability::TargetUnavailable,
    )?;
    check_snapshot("setup-check-timeout", &text)?;
    assert_eq!(
        text,
        setup_check_text(
            reported(),
            RuntimeReadiness::Blocked,
            ManagedPlanAvailability::Qualified,
        )?,
        "managed installation changes nothing for a tool that was found"
    );

    for phrase in INSTALL_ADVICE {
        assert!(!text.contains(phrase), "says {phrase:?}: {text}");
    }
    assert!(!text.contains("[missing]") && !text.contains("not found"));
    assert!(text.contains("Status: blocked\n"), "{text}");
    assert_eq!(text.matches("[timeout] ").count(), 3, "{text}");
    assert_eq!(
        text.matches(
            "Run setup check again, or allow more time with --timeout-seconds (up to 60).\n"
        )
        .count(),
        3,
        "{text}"
    );
    assert!(
        text.contains(
            "[timeout] FFmpeg (media_processing): found, but it did not answer in time [filtered PATH]\n  It was started and had not answered when the deadline passed: 30 seconds, which the three tool checks share."
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "[timeout] FFprobe (media_processing): found, but not started before the deadline passed [filtered PATH]\n  It was not started, because the deadline had already passed: 30 seconds, which the three tool checks share. Nothing is known about it yet."
        ),
        "{text}"
    );
    Ok(())
}

/// The same for each tool on its own and each kind of timeout, with the
/// deadline the command line applied named as it was given; and a tool that
/// is missing, or was found and failed, keeps the advice it had.
#[test]
fn each_tool_and_each_kind_of_timeout_gets_the_timeout_advice_only() -> TestResult {
    let available = || DependencyState::Available {
        version: String::from("fixture 1"),
    };
    for index in 0..3 {
        for (kind, said) in [
            (
                ProbeTimeoutKind::NoAnswer,
                "found, but it did not answer in time",
            ),
            (
                ProbeTimeoutKind::NotStarted,
                "found, but not started before the deadline passed",
            ),
        ] {
            let mut states = [available(), available(), available()];
            states[index] = timed_out(kind, 1);
            let text = setup_check_text(
                states,
                RuntimeReadiness::Degraded,
                ManagedPlanAvailability::Qualified,
            )?;
            for phrase in INSTALL_ADVICE {
                assert!(
                    !text.contains(phrase),
                    "{index} {kind:?}: {phrase:?}: {text}"
                );
            }
            assert_eq!(text.matches("[timeout] ").count(), 1, "{text}");
            assert!(text.contains(said), "{text}");
            assert!(
                text.contains(": 1 second, which the three tool checks share."),
                "{text}"
            );
        }
        for state in [
            DependencyState::Missing,
            DependencyState::Unhealthy {
                message: String::from("process exited with exit code: 3"),
            },
        ] {
            let mut states = [available(), available(), available()];
            states[index] = state;
            let text = setup_check_text(
                states,
                RuntimeReadiness::Degraded,
                ManagedPlanAvailability::Qualified,
            )?;
            assert!(
                text.contains(
                    "  Install a trusted build or register a working one with setup configure "
                ),
                "{text}"
            );
            assert!(
                text.contains(
                    "
  No need to search folders: ask the user where it is.
"
                ),
                "{text}"
            );
            assert!(!text.contains("--timeout-seconds"), "{text}");
        }
    }
    Ok(())
}

/// Issue #396: under a missing tool `setup check` said "Install or locate
/// this trusted tool", and a cold agent read "locate" as leave to search the
/// machine. For each tool, missing or found and failing, with and without
/// managed installation, the advice now names the commands that make the tool
/// known to `VSift` (`setup configure` with an absolute path, the per-call
/// option, and `setup plan` with `setup install` only where they are offered)
/// and says on a line of its own that no folder search is needed.
#[test]
fn a_missing_tool_is_told_how_to_register_it_and_that_no_search_is_needed() -> TestResult {
    let available = || DependencyState::Available {
        version: String::from("fixture 1"),
    };
    for (index, dependency) in RuntimeDependency::ALL.into_iter().enumerate() {
        let identifier = dependency.identifier();
        for state in [
            DependencyState::Missing,
            DependencyState::Unhealthy {
                message: String::from("process exited with exit code: 3"),
            },
        ] {
            for managed_install in [
                ManagedPlanAvailability::Qualified,
                ManagedPlanAvailability::TargetUnavailable,
            ] {
                let mut states = [available(), available(), available()];
                states[index] = state.clone();
                let text = setup_check_text(states, RuntimeReadiness::Degraded, managed_install)?;
                let managed = if managed_install == ManagedPlanAvailability::Qualified {
                    "Or let VSift install the reviewed build: review setup plan, then accept it with setup install."
                } else {
                    "Managed installation is not available for this target."
                };
                let expected = format!(
                    "\n  Install a trusted build or register a working one with setup configure {identifier} --executable <absolute-path>, then rerun setup check (or try a path once with setup check --{identifier} <absolute-path>). {managed}\n  No need to search folders: ask the user where it is.\n"
                );
                assert!(
                    text.contains(&expected),
                    "{identifier} {managed_install:?}: {text}"
                );
                assert_eq!(
                    text.matches("No need to search folders").count(),
                    1,
                    "{text}"
                );
                assert_eq!(
                    text.contains("setup plan"),
                    managed_install == ManagedPlanAvailability::Qualified,
                    "{text}"
                );
                assert!(!text.to_lowercase().contains("locate"), "{text}");
            }
        }
    }
    Ok(())
}

/// "Allow more time with --timeout-seconds (up to 60)" is no advice for
/// someone who ran `setup check --timeout-seconds 60`. After the longest
/// deadline the text says the option cannot extend it, to run the check
/// again, and what a tool that never answers means; it still never says to
/// install or locate the tool. One second less still offers more time.
#[test]
fn after_the_longest_deadline_the_readable_advice_does_not_offer_more_time() -> TestResult {
    let text = setup_check_text(
        [
            timed_out(ProbeTimeoutKind::NoAnswer, 60),
            timed_out(ProbeTimeoutKind::NotStarted, 60),
            timed_out(ProbeTimeoutKind::NotStarted, 60),
        ],
        RuntimeReadiness::Blocked,
        ManagedPlanAvailability::Qualified,
    )?;
    assert!(
        text.contains(
            "[timeout] FFmpeg (media_processing): found, but it did not answer in time [filtered PATH]\n  It was started and had not answered when the deadline passed: 60 seconds, which the three tool checks share and --timeout-seconds cannot extend. Run setup check again. If it never answers, the tool itself hangs: run it yourself to see what it does.\n"
        ),
        "{text}"
    );
    assert_eq!(
        text.matches(
            "  It was not started, because the deadline had already passed: 60 seconds, which the three tool checks share and --timeout-seconds cannot extend. Nothing is known about it yet. Run setup check again. If a tool never answers, that tool hangs: run it yourself to see what it does.\n"
        )
        .count(),
        2,
        "{text}"
    );
    assert!(!text.contains("allow more time"), "{text}");
    assert!(!text.contains("up to 60"), "{text}");
    for phrase in INSTALL_ADVICE {
        assert!(!text.contains(phrase), "says {phrase:?}: {text}");
    }

    let one_second_less = setup_check_text(
        [
            timed_out(ProbeTimeoutKind::NoAnswer, 59),
            timed_out(ProbeTimeoutKind::NotStarted, 59),
            timed_out(ProbeTimeoutKind::NotStarted, 59),
        ],
        RuntimeReadiness::Blocked,
        ManagedPlanAvailability::Qualified,
    )?;
    assert_eq!(
        one_second_less
            .matches("or allow more time with --timeout-seconds (up to 60).\n")
            .count(),
        3,
        "{one_second_less}"
    );
    Ok(())
}

/// A plan with a reviewed action names its download, digest, licence,
/// notices, trust limit and files, each URL on its own line.
#[test]
fn a_plan_with_an_action_names_everything_the_user_accepts() -> TestResult {
    let mut plan = example("setup-plan.unavailable.json")?;
    plan["data"]["target"] = json!("ubuntu_24_04_x86_64");
    plan["data"]["managed_install"] = json!("catalogue_accepted");
    plan["data"]["catalogue_revision"] = json!("2026-09-20.1");
    plan["data"]["stop_new_plans_at"] = json!("2026-12-31T00:00:00Z");
    plan["data"]["plan_digest"] = json!("a".repeat(64));
    plan["data"]["install_needed"] = json!(true);
    plan["data"]["actions"] = json!([{
        "id": "install-whisper_model",
        "state": "pending",
        "component": "whisper_model",
        "version": "base-q5_1",
        "publisher": "ggml-org",
        "source_url": "https://example.invalid/ggml-base-q5_1.bin",
        "bytes": 59_707_625,
        "sha256": "b".repeat(64),
        "format": "raw_file",
        "archive_limits": null,
        "selected_files": [],
        "archive_links": [],
        "runtime_copies": [],
        "licence": "MIT",
        "notice_url": "https://example.invalid/LICENSE",
        "source_code_url": "https://example.invalid/source",
        "trust_limit": "Publisher bytes pinned by digest; model quality is not verified.",
        "licence_scope": "disclosure_not_legal_clearance",
        "destination": "private_per_user_managed_runtime",
        "permissions": "private_user_only",
        "change": "planned_download_verify_extract_smoke_activate",
        "required_authority": "user",
        "files": [{"name": "ggml-base-q5_1.bin", "bytes": 59_707_625, "sha256": "b".repeat(64), "mode": "owner_read_write"}]
    }]);
    let text = render(CommandName::SetupPlan, &plan)?;
    check_snapshot("setup-plan-action", &text)?;
    assert!(text.contains(&"a".repeat(64)), "the digest is cut");
    assert!(
        text.lines()
            .any(|line| line.trim() == "https://example.invalid/ggml-base-q5_1.bin")
    );
    assert!(text.contains("from ggml-org [pending]"));
    assert!(!text.contains("nothing to install"));

    // P13 PR 4: after the install the same plan says each action is current
    // and that nothing is left to install.
    plan["data"]["install_needed"] = json!(false);
    plan["data"]["actions"][0]["state"] = json!("current");
    plan["data"]["local_asr_model"]["status"] = json!("managed_current");
    plan["data"]["readiness"] = json!("ready");
    let installed = render(CommandName::SetupPlan, &plan)?;
    assert!(installed.contains("from ggml-org [current]"));
    assert!(installed.contains("nothing to install"));
    assert!(installed.contains("Readiness: ready"));
    Ok(())
}

#[test]
fn setup_registrations_say_what_was_registered_and_what_is_next() -> TestResult {
    let selection = OperationResponse::complete(
        CommandName::SetupConfigure.identifier(),
        &ConfiguredSelectionResponse::new(RuntimeDependency::Ffprobe),
    )?;
    let model = OperationResponse::complete(
        CommandName::SetupConfigureModel.identifier(),
        &ConfiguredModelResponse::new(),
    )?;
    let selection = result(CommandName::SetupConfigure, &selection)?.ok_or("no renderer")?;
    let model = result(CommandName::SetupConfigureModel, &model)?.ok_or("no renderer")?;
    check_snapshot("setup-configure", selection.as_str())?;
    check_snapshot("setup-configure-model", model.as_str())?;
    Ok(())
}

/// P13: `setup install` lists each component with its status, the step and
/// typed reason of a failure and the stage's disposal; a failure also
/// renders its error, with the remediation, for stderr.
#[test]
fn setup_install_lists_each_component_and_its_outcome() -> TestResult {
    let complete = render_value(CommandName::SetupInstall, &example("setup-install.json")?)?
        .ok_or("no renderer")?;
    assert_terminal_safe(complete.as_str());
    check_snapshot("setup-install", complete.as_str())?;
    assert!(
        complete
            .as_str()
            .contains("[activated] ffmpeg_ffprobe n9.0.1-11-ge47273f4d9-20260831")
    );

    let failed_value = example("setup-install.failed.json")?;
    let failed = render_value(CommandName::SetupInstall, &failed_value)?.ok_or("no renderer")?;
    assert_terminal_safe(failed.as_str());
    check_snapshot("setup-install-failed", failed.as_str())?;
    assert!(failed.as_str().contains(
        "[failed] whisper_cli whisper.cpp-v1.9.2-ubuntu-x64: download offline, DOWNLOAD_FAILED"
    ));
    assert!(failed.as_str().contains("Status: failed"));
    let error = render_failure(&failed_value, None)?;
    assert_terminal_safe(error.as_str());
    check_snapshot("failure-setup-install-download", error.as_str())?;
    assert!(error.as_str().contains("(DOWNLOAD_FAILED)"));

    // #256: a tool the loader could not start names the missing library, in
    // the component line and in the remediation, so a person sees what to install.
    let missing_value = example("setup-install.missing-library.json")?;
    let missing = render_value(CommandName::SetupInstall, &missing_value)?.ok_or("no renderer")?;
    assert_terminal_safe(missing.as_str());
    check_snapshot("setup-install-missing-library", missing.as_str())?;
    assert!(missing.as_str().contains(
        "[failed] whisper_cli whisper.cpp-v1.9.2-ubuntu-x64: smoke (banner check) provider_failed (missing shared library libgomp.so.1), MISSING_CAPABILITY; stage discarded"
    ));
    let missing_error = render_failure(&missing_value, None)?;
    assert_terminal_safe(missing_error.as_str());
    check_snapshot(
        "failure-setup-install-missing-library",
        missing_error.as_str(),
    )?;
    assert!(
        missing_error
            .as_str()
            .contains("sudo apt-get install libgomp1")
    );
    assert!(missing_error.as_str().contains("(MISSING_CAPABILITY)"));
    Ok(())
}

/// P13 PR 6: `setup list`, `setup rollback`, `setup remove` and `setup
/// repair` render every component, version, outcome and finding, and a
/// repair finding's command as a `Run:` line of fixed words and keys.
#[test]
fn setup_lifecycle_results_render() -> TestResult {
    for (command, file, snapshot, expected) in [
        (
            CommandName::SetupList,
            "setup-list.json",
            "setup-list",
            "  [verified] n9.0.1-11-ge47273f4d9-20260831, selected",
        ),
        (
            CommandName::SetupRollback,
            "setup-rollback.json",
            "setup-rollback",
            "ffmpeg_ffprobe: rolled_back, selected n8.1-2-g0123456789-20260601",
        ),
        (
            CommandName::SetupRemove,
            "setup-remove.json",
            "setup-remove",
            "Abandoned stages: removed 2, kept 0",
        ),
        (
            CommandName::SetupRepair,
            "setup-repair.json",
            "setup-repair",
            "  Run: vsift setup rollback ffmpeg_ffprobe",
        ),
        (
            CommandName::SetupRemove,
            "setup-remove.failed.json",
            "setup-remove-failed",
            "[in_use] whisper_cli whisper.cpp-v1.9.2-ubuntu-x64",
        ),
    ] {
        let text = render(command, &example(file)?)?;
        check_snapshot(snapshot, &text)?;
        assert!(text.contains(expected), "{snapshot}: {text}");
    }
    let failed = render_failure(&example("setup-remove.failed.json")?, None)?;
    assert_terminal_safe(failed.as_str());
    assert!(failed.as_str().contains("(BUSY)"));
    assert!(failed.as_str().contains("Retry after: 30000 ms"));
    let install = render(CommandName::SetupInstall, &example("setup-install.json")?)?;
    assert!(install.contains("Abandoned stages: removed 1, kept 0"));
    assert!(install.contains("Cleanup: [removed] ffmpeg_ffprobe n8.1-2-g0123456789-20260601"));
    Ok(())
}

/// Every failure example renders as message, remediation and suggested
/// command, on stderr.
#[test]
fn frozen_failures_render_as_message_remediation_and_command() -> TestResult {
    for (file, snapshot) in [
        ("parse-failure.json", "failure-parse"),
        ("operation-error.json", "failure-not-implemented"),
        ("transcript-rejected.json", "failure-transcript-rejected"),
        (
            "retranscribe-cancelled.json",
            "failure-retranscribe-cancelled",
        ),
        (
            "retranscribe-unusable-output.json",
            "failure-retranscribe-unusable-output",
        ),
        (
            "retranscribe-unusable-output.short-range.json",
            "failure-retranscribe-unusable-output-short-range",
        ),
        ("storage-not-private.json", "failure-storage-not-private"),
        ("session-root-unowned.json", "failure-session-root-unowned"),
        ("ingest-copy-too-slow.json", "failure-ingest-copy-too-slow"),
        (
            "ingest-source-not-found.json",
            "failure-ingest-source-not-found",
        ),
        (
            "ingest-source-is-folder.json",
            "failure-ingest-source-is-folder",
        ),
        (
            "ingest-source-access-denied.json",
            "failure-ingest-source-access-denied",
        ),
        ("frame-get.busy.json", "failure-frame-get-busy"),
        (
            "frame-get.busy-transcription.json",
            "failure-frame-get-busy-transcription",
        ),
        (
            "media-tool-verification-failed.json",
            "failure-media-tool-verification",
        ),
    ] {
        let rendered = render_failure(&example(file)?, None)?;
        assert_terminal_safe(rendered.as_str());
        assert!(rendered.as_str().starts_with("Error: "));
        check_snapshot(snapshot, rendered.as_str())?;
    }
    let cancelled = render_failure(&example("retranscribe-cancelled.json")?, None)?;
    assert!(
        cancelled
            .as_str()
            .lines()
            .any(|line| line.starts_with("Run: vsift job resume job_")),
        "{}",
        cancelled.as_str()
    );
    Ok(())
}

/// #342: a media command refused because another request holds its session
/// or the root's capacity tells a person what to do and when: the message and
/// code, the fix line (wait, run it again, one command at a time) and the
/// retry hint, in that order.
#[test]
fn a_busy_media_command_prints_its_fix_and_its_retry_hint() -> TestResult {
    let rendered = render_failure(&example("frame-get.busy.json")?, None)?;
    let lines: Vec<&str> = rendered.as_str().lines().collect();
    assert_eq!(
        lines,
        [
            "Error: The requested operation is temporarily busy. (BUSY)",
            &format!("Fix: {MEDIA_BUSY_REMEDIATION}"),
            "Retry after: 2000 ms",
        ]
    );
    Ok(())
}

/// #340: the human text of an audio result says, on the line under the clip's
/// path, who the clip is for and how an agent reads speech. A frame and a
/// crop are images an agent can open, so they say nothing of the kind.
#[test]
fn an_audio_clip_says_who_it_is_for_under_its_path() -> TestResult {
    let text = render(CommandName::Audio, &example("audio.json")?)?;
    let lines: Vec<&str> = text.lines().collect();
    let label = lines
        .iter()
        .position(|line| *line == "  File (audio/wav):")
        .ok_or("the clip's file is missing")?;
    assert!(
        lines[label + 1].starts_with("    /vsift-session-root/"),
        "{text}"
    );
    assert_eq!(lines[label + 2], evidence::AUDIO_LISTENER_NOTE);
    assert_eq!(text.matches("cannot listen").count(), 1, "{text}");
    for needle in [
        "for a person or a speech tool to play",
        "a coding agent cannot listen to it",
        "vsift transcript get",
    ] {
        assert!(evidence::AUDIO_LISTENER_NOTE.contains(needle), "{needle}");
    }

    for (command, file) in [
        (CommandName::FrameGet, "frame-get.json"),
        (CommandName::Crop, "crop.json"),
    ] {
        let images = render(command, &example(file)?)?;
        assert!(!images.contains("cannot listen"), "{file}");
    }
    Ok(())
}

/// A parser explanation is quoted under its untrusted label; a fixed reason
/// follows the message.
#[test]
fn a_failure_shows_its_human_detail() -> TestResult {
    let explanation = DisplayText::render_lines(
        "error: invalid value 'a\u{202e}b\u{1b}[31m' for '--rect'\n  tip",
    );
    let parse = render_failure(
        &example("parse-failure.json")?,
        Some(HumanDetail::Parser(&explanation)),
    )?;
    assert_terminal_safe(parse.as_str());
    let lines: Vec<&str> = parse.as_str().lines().collect();
    assert_eq!(
        lines[1],
        "The parser explains (this quotes the command line, which is untrusted text):"
    );
    assert_eq!(
        lines[2],
        "  | error: invalid value 'a<U+202E>b\u{fffd}[31m' for '--rect'"
    );
    assert_eq!(lines[3], "  |   tip");
    assert!(lines[4].starts_with("Fix: The command line was rejected (invalid_value)."));
    assert_eq!(lines[5], "Run: vsift crop --help");

    let refused = render_failure(
        &example("operation-error.json")?,
        Some(HumanDetail::Reason(
            "execution profile is denied by host policy",
        )),
    )?;
    assert!(
        refused
            .as_str()
            .contains("\nReason: execution profile is denied by host policy\n")
    );
    Ok(())
}

/// Defence in depth: a result whose display fields broke their contract
/// (raw escapes, an OSC-8 link, bidirectional and separator characters) and
/// whose identifiers carry controls still reaches the terminal inert.
#[test]
fn hostile_display_fields_and_identifiers_stay_inert() -> TestResult {
    let mut page = example("transcript-get.json")?;
    page["data"]["session_id"] = json!("ses_0123\u{1b}[2J\u{9b}31m");
    page["data"]["items"][0]["display_text"] = json!(
        "\u{1b}]8;;https://example.invalid\u{7}click\u{1b}]8;;\u{7} \u{202e}DELIAF\u{202c}\nnext\u{2028}line\u{85}"
    );
    page["data"]["items"][0]["speaker"] = json!({
        "label": "ignored",
        "display_label": "Adm\u{200b}in\u{1b}[8m\nForged: line",
        "origin": "imported_webvtt_voice"
    });
    let text = render(CommandName::TranscriptGet, &page)?;
    assert!(!text.contains("\u{1b}]8;"));
    assert!(text.contains("  | \u{fffd}]8;;https://example.invalid\u{fffd}click"));
    assert!(text.contains("<U+202E>DELIAF<U+202C>"));
    assert!(text.contains("\n  | next<U+2028>line\u{fffd}\n"));
    assert!(text.contains("): Adm<U+200B>in\u{fffd}[8m\u{fffd}Forged: line\n"));
    // Raw `text` is never read: the example's plain text is not printed
    // where the display text was replaced.
    assert!(!text.contains("This synthetic sidecar is aligned with"));
    Ok(())
}

/// Every frozen example of a PR 2b command renders; its snapshot is the
/// text a person reads.
#[test]
fn part_two_examples_render_as_readable_text() -> TestResult {
    for (command, file, snapshot) in [
        (CommandName::Candidates, "candidates.json", "candidates"),
        (
            CommandName::Candidates,
            "candidates.partial.json",
            "candidates-partial",
        ),
        (CommandName::FrameGet, "frame-get.json", "frame-get"),
        (
            CommandName::FrameNeighbours,
            "frame-neighbours.json",
            "frame-neighbours",
        ),
        (
            CommandName::FrameBurst,
            "frame-burst.partial.json",
            "frame-burst-partial",
        ),
        (CommandName::Crop, "crop.json", "crop"),
        (CommandName::Audio, "audio.json", "audio"),
        (CommandName::JobStatus, "job-status.json", "job-status"),
        (CommandName::JobResume, "job-resume.json", "job-resume"),
        (CommandName::JobCancel, "job-cancel.json", "job-cancel"),
        (CommandName::JobRun, "job-run.json", "job-run"),
        (
            CommandName::JobRun,
            "job-run.partial.json",
            "job-run-partial",
        ),
        (
            CommandName::JobRun,
            "job-run.replayed.json",
            "job-run-replayed",
        ),
        (CommandName::JobBatch, "job-batch.json", "job-batch"),
    ] {
        let text = render(command, &example(file)?)?;
        check_snapshot(snapshot, &text)?;
    }
    Ok(())
}

/// Every command that completes has a renderer: `result` answers `None`
/// only for `setup check`, rendered from its typed report, and for the
/// commands that only ever fail.
#[test]
fn only_setup_check_and_rejected_command_lines_have_no_result_renderer() -> TestResult {
    for command in [CommandName::SetupCheck, CommandName::Parse] {
        assert!(render_value(command, &json!({}))?.is_none(), "{command:?}");
    }
    for command in [
        CommandName::SetupInstall,
        CommandName::SetupList,
        CommandName::SetupRollback,
        CommandName::SetupRemove,
        CommandName::SetupRepair,
        CommandName::Candidates,
        CommandName::FrameGet,
        CommandName::Crop,
        CommandName::Audio,
        CommandName::JobStatus,
        CommandName::JobRun,
        CommandName::JobBatch,
    ] {
        assert!(render_value(command, &json!({})).is_err(), "{command:?}");
    }
    Ok(())
}

/// L-016 and SEC-T02 (P13 PR 2b): a delivered path stands alone on its
/// line under its file's label. An exact path is the line itself; a path
/// with controls or hidden characters (the session root is the user's
/// choice) reaches the terminal inert and is flagged; the extended-length
/// form gets its note once.
#[test]
fn delivered_paths_stand_alone_and_stay_inert() -> TestResult {
    let hostile = "/root/a\u{202e}gnp.exe\u{200b}/\u{1b}]8;;https://example.invalid\u{7}x\u{1b}\\\n\
                   Forged: line\u{2028}\u{85}/artifact.png";
    let extended = r"\\?\C:\Users\someone\AppData\Local\vsift\sessions\ses_0123456789abcdef0123456789abcdef\artifacts\artifact-4ab8.png";
    for (command, file) in [
        (CommandName::FrameGet, "frame-get.json"),
        (CommandName::FrameNeighbours, "frame-neighbours.json"),
        (CommandName::FrameBurst, "frame-burst.partial.json"),
        (CommandName::Crop, "crop.json"),
        (CommandName::Audio, "audio.json"),
    ] {
        let mut value = example(file)?;
        value["data"]["files"][0]["path"] = json!(hostile);
        if value["data"]["files"][1].is_object() {
            value["data"]["files"][1]["path"] = json!(extended);
        }
        let text = render(command, &value)?;
        assert!(!text.contains("\u{1b}]8;") && !text.contains('\u{2028}'));
        let lines: Vec<&str> = text.lines().collect();
        let shown = lines
            .iter()
            .position(|line| line.contains("<U+202E>gnp.exe<U+200B>"))
            .ok_or("the hostile path is missing")?;
        assert!(lines[shown].starts_with("    /root/a<U+202E>"), "{text}");
        assert!(
            lines[shown].ends_with("\u{fffd}Forged: line<U+2028>\u{fffd}/artifact.png"),
            "{text}"
        );
        assert!(lines[shown - 1].starts_with("  File ("), "{text}");
        assert!(!lines.iter().any(|line| line.starts_with("Forged")));
        assert_eq!(text.matches("is not shown exactly").count(), 1, "{text}");
        if value["data"]["files"][1].is_object() {
            assert!(
                lines.contains(&format!("    {extended}").as_str()),
                "{text}"
            );
            assert_eq!(text.matches("extended-length form").count(), 1, "{text}");
        } else {
            assert!(!text.contains("extended-length form"), "{text}");
        }
    }
    Ok(())
}

/// A path longer than a line is replaced by a statement, never cut; an
/// exact extended-length path is its own line, as `--json` gives it.
#[test]
fn a_long_or_extended_path_is_never_cut() -> TestResult {
    let long = format!("/{}", "p".repeat(4_200));
    let mut value = example("frame-get.json")?;
    value["data"]["files"][0]["path"] = json!(long);
    let text = render(CommandName::FrameGet, &value)?;
    assert!(!text.contains(&"p".repeat(100)), "a path was cut");
    assert!(text.contains("\n    (a path longer than 4000 bytes; read it with --json)\n"));
    assert!(text.contains("is not shown exactly"));

    let extended = format!(r"\\?\C:\{}\artifact.png", "d".repeat(300));
    let mut value = example("audio.json")?;
    value["data"]["files"][0]["path"] = json!(extended);
    let text = render(CommandName::Audio, &value)?;
    check_snapshot("audio-extended-path", &text)?;
    assert!(text.lines().any(|line| line == format!("    {extended}")));
    Ok(())
}

/// A failed worker request renders its job result for stdout and its error
/// for stderr; a refusal before the request ran has an error only.
#[test]
fn a_failed_worker_request_renders_its_result_and_its_error() -> TestResult {
    let mut failed = example("job-run.json")?;
    failed["status"] = json!("failed");
    failed["data"]["status"] = json!("failed");
    failed["data"]["steps"][1]["status"] = json!("failed");
    failed["data"]["steps"][1]["outputs"] = json!(null);
    failed["data"]["steps"][1]["failure"] =
        json!({"code": "BUSY", "retryable": true, "retry_after_ms": 2000});
    failed["data"]["steps"][2]["status"] = json!("not_started");
    failed["data"]["steps"][2]["outputs"] = json!(null);
    failed["data"]["steps"][2]["coverage"] = json!(null);
    failed["data"]["steps"][3]["status"] = json!("not_started");
    failed["data"]["steps"][3]["outputs"] = json!(null);
    failed["data"]["failure"] = json!({
        "code": "BUSY", "retryable": true, "retry_after_ms": 2000, "step": 1, "rejection": null
    });
    failed["error"] = json!({
        "code": "BUSY",
        "message": "The resource is busy.",
        "retryable": true,
        "retry_after_ms": 2000,
        "affected_ids": ["ses_0123456789abcdef0123456789abcdef"],
        "remediation": [{"summary": "Deliver the same request again after the retry hint.", "command": null, "required_authority": "none"}]
    });
    let text = render_host(CommandName::JobRun, &failed)?;
    let result = text.result.ok_or("no result")?;
    let error = text.failure.ok_or("no error")?;
    assert_terminal_safe(result.as_str());
    check_snapshot("job-run-failed", result.as_str())?;
    check_snapshot("job-run-failed-stderr", error.as_str())?;

    let text = render_host(CommandName::JobRun, &example("operation-error.json")?)?;
    assert!(text.result.is_none());
    assert!(
        text.failure
            .ok_or("no error")?
            .as_str()
            .starts_with("Error: ")
    );
    Ok(())
}

/// A result that does not have its published shape fails to render
/// instead of printing half of it.
#[test]
fn a_result_without_its_published_shape_is_refused() {
    let broken = json!({"status": "complete", "data": {"session_id": 3}});
    assert!(render_value(CommandName::Ingest, &broken).is_err());
}

/// #371: a session page that is not the last still says how to read the next
/// one. Since a page holds up to 256 sessions the readable snapshots, made
/// with a handful of sessions, end on their last page, so the line is pinned
/// here for both commands.
#[test]
fn a_session_page_that_is_not_the_last_names_its_next_cursor() -> TestResult {
    let item = json!({
        "error_code": null,
        "session_id": "ses_c574fee114f5a357daedeac75e93addc",
        "state": "open",
        "status": {
            "artifact_bytes": 0,
            "artifact_count": 0,
            "expires_at": "2026-10-18T03:38:15Z",
            "generation": 1,
            "session_id": "ses_c574fee114f5a357daedeac75e93addc",
            "source_bytes": 13,
            "source_id": "src_sha256_1a55b52ee45dfa2fb9466588a56dfa010ef5d26fbcd5aeb9a8676706323d1bec",
            "state": "open"
        }
    });
    let list = json!({
        "schema_version": "1", "command": "session.list", "operation_id": null,
        "status": "complete", "data": {"items": [item], "next_cursor": 200},
        "warnings": [], "error": null, "coverage": null, "lifecycle": null
    });
    let clean = json!({
        "schema_version": "1", "command": "session.clean", "operation_id": null,
        "status": "complete",
        "data": {"dry_run": true, "items": [{
            "error_code": null, "outcome": "ineligible",
            "session_id": "ses_c574fee114f5a357daedeac75e93addc"
        }], "next_cursor": 200},
        "warnings": [], "error": null, "coverage": null, "lifecycle": null
    });
    for (command, value) in [
        (CommandName::SessionList, list),
        (CommandName::SessionClean, clean),
    ] {
        let text = render(command, &value)?;
        assert!(
            text.contains("More on the next page: repeat the command with --cursor 200\n"),
            "{text}"
        );
    }
    Ok(())
}
