//! What `setup check` and `setup plan` say about a tool whose probe timed
//! out (issue #391).
//!
//! `setup check` reported an installed, healthy `FFmpeg` as `timed_out` on the
//! first run after a restart and told its owner, in `remediation.next_step`,
//! to "Install or locate a trusted `FFmpeg` executable". The published `reason`
//! (`timed_out`) was right and stays; the advice was the advice for a missing
//! tool. These tests hold the contract crate to the answer for every
//! dependency and both kinds of timeout: the tool was found, which of the two
//! things happened, under which deadline, and what helps, within the 240
//! characters the published schema allows; and never to install or locate it.
//! A missing tool, and one that was found and failed, keep their published
//! advice word for word.

use std::{fs, io, path::PathBuf, time::Duration};

use serde_json::Value;
use vsift_application::{
    LocalAsrCheckOutcome, LocalAsrModelStatus, LocalAsrNotRunReason, LocalAsrSetupStatus,
    ManagedPlanAvailability, ManagedPlanObservation, ManagedSetupPlan, RuntimeDiagnosis,
    SetupProfile, SetupSelectionState, plan_managed_setup,
};
use vsift_contract::{
    DependencyLookup, MAX_PROBE_TIMEOUT, OperationResponse, ProbeTimeHeadroom, SavedSetupPlan,
    SetupCheckResponse, SetupPlanResponse, probe_deadline_text,
};
use vsift_domain::{
    DependencyState, DependencyStatus, FailureCode, ManagedTarget, ProbeTimeout, ProbeTimeoutKind,
    RuntimeDependency, RuntimeReadiness,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Words that tell a reader to get a tool, or where to point `VSift` at one.
/// None of them is true of a tool that was found.
const INSTALL_ADVICE: [&str; 9] = [
    "Install or locate",
    "install or locate",
    "setup install",
    "setup configure",
    "reviewed build",
    "Managed installation",
    "absolute path",
    "absolute-path",
    "missing",
];

/// The tools were found, so the local-ASR part got as far as the model.
const NO_MODEL: LocalAsrSetupStatus = LocalAsrSetupStatus {
    model: LocalAsrModelStatus::NotSelected,
    verification: LocalAsrCheckOutcome::NotRun(LocalAsrNotRunReason::ModelNotSelected),
};

const KINDS: [ProbeTimeoutKind; 2] = [ProbeTimeoutKind::NoAnswer, ProbeTimeoutKind::NotStarted];

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn load(relative_path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(
        schema_root().join(relative_path),
    )?)?)
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    let schema = load(schema_path)?;
    jsonschema::validator_for(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

fn timed_out(kind: ProbeTimeoutKind, shared_deadline: Duration) -> DependencyState {
    DependencyState::TimedOut(ProbeTimeout {
        kind,
        shared_deadline,
    })
}

fn available() -> DependencyState {
    DependencyState::Available {
        version: String::from("fixture 1"),
    }
}

/// A diagnosis with these states, in the stable dependency order.
fn diagnosis(states: [DependencyState; 3], readiness: RuntimeReadiness) -> RuntimeDiagnosis {
    RuntimeDiagnosis {
        readiness,
        dependencies: RuntimeDependency::ALL
            .into_iter()
            .zip(states)
            .map(|(dependency, state)| DependencyStatus { dependency, state })
            .collect(),
    }
}

/// The reported shape: the first tool does not answer within the deadline
/// and the two after it are never started.
fn reported_shape(shared_deadline: Duration) -> RuntimeDiagnosis {
    diagnosis(
        [
            timed_out(ProbeTimeoutKind::NoAnswer, shared_deadline),
            timed_out(ProbeTimeoutKind::NotStarted, shared_deadline),
            timed_out(ProbeTimeoutKind::NotStarted, shared_deadline),
        ],
        RuntimeReadiness::Blocked,
    )
}

fn check(
    diagnosis: &RuntimeDiagnosis,
    managed_install: ManagedPlanAvailability,
) -> Result<Value, serde_json::Error> {
    serde_json::to_value(SetupCheckResponse::new(
        diagnosis,
        SetupProfile::Desktop,
        |_| DependencyLookup::FilteredPath,
        managed_install,
        &NO_MODEL,
    ))
}

/// The one dependency of a check answer that is at `index`.
fn dependency(response: &Value, index: usize) -> Result<&Value, io::Error> {
    response["dependencies"]
        .get(index)
        .ok_or_else(|| io::Error::other(format!("no dependency {index}")))
}

fn next_step(dependency: &Value) -> Result<&str, io::Error> {
    dependency["remediation"]["next_step"]
        .as_str()
        .ok_or_else(|| io::Error::other(format!("no next_step in {dependency}")))
}

fn assert_no_install_advice(text: &str, context: &str) {
    for phrase in INSTALL_ADVICE {
        assert!(!text.contains(phrase), "{context}: says {phrase:?}: {text}");
    }
}

/// The whole answer for the reported shape under the default deadline, where
/// managed installation is available: the status stays `blocked` and each
/// reason stays `timed_out`, and nothing in it proposes an install.
#[test]
fn a_timed_out_setup_check_matches_the_frozen_example() -> TestResult {
    let response = check(
        &reported_shape(Duration::from_secs(30)),
        ManagedPlanAvailability::Qualified,
    )?;

    validate("setup-check-response.schema.json", &response)?;
    assert_eq!(response, load("examples/setup-check.timed-out.json")?);
    assert_eq!(response["status"], "blocked");
    Ok(())
}

/// The regression test of the finding, for each dependency and each kind of
/// timeout, over the deadlines the command line can apply and the extremes a
/// library host could pass.
#[test]
fn no_tool_whose_probe_timed_out_is_told_to_install_or_locate_it() -> TestResult {
    let deadlines = [
        Duration::ZERO,
        Duration::from_millis(1),
        Duration::from_secs(1),
        Duration::from_millis(1_500),
        Duration::from_secs(5),
        Duration::from_secs(30),
        Duration::from_secs(60),
        Duration::from_secs(3_600),
        Duration::from_hours(24),
        Duration::MAX,
    ];
    for (index, tool) in RuntimeDependency::ALL.into_iter().enumerate() {
        for kind in KINDS {
            for deadline in deadlines {
                for managed_install in [
                    ManagedPlanAvailability::Qualified,
                    ManagedPlanAvailability::TargetUnavailable,
                ] {
                    let context = format!("{tool:?} {kind:?} {deadline:?} {managed_install:?}");
                    let mut states = [available(), available(), available()];
                    states[index] = timed_out(kind, deadline);
                    let response = check(
                        &diagnosis(states, RuntimeReadiness::Blocked),
                        managed_install,
                    )?;
                    validate("setup-check-response.schema.json", &response)?;

                    let item = dependency(&response, index)?;
                    assert_eq!(item["status"], "timed_out", "{context}");
                    assert_eq!(item["detail"], Value::Null, "{context}");
                    assert_eq!(item["validation"], "not_validated", "{context}");
                    assert_eq!(item["remediation"]["reason"], "timed_out", "{context}");
                    assert_eq!(
                        item["remediation"]["required_authority"], "user",
                        "{context}"
                    );
                    let step = next_step(item)?;
                    assert_no_install_advice(step, &context);
                    assert!(step.chars().count() <= 240, "{context}: {step}");
                    assert!(step.contains(" was found "), "{context}: {step}");
                    assert!(
                        step.contains(&format!("({})", probe_deadline_text(deadline))),
                        "{context}: {step}"
                    );
                    // More time is offered only while the option has more
                    // to give; after its longest deadline the step says so.
                    match ProbeTimeHeadroom::of(deadline) {
                        ProbeTimeHeadroom::MoreTimeAllowed => assert!(
                            step.ends_with(
                                "A first run after an install or restart can be slow. Run setup check again, or allow more time with --timeout-seconds (up to 60)."
                            ),
                            "{context}: {step}"
                        ),
                        ProbeTimeHeadroom::AtTheMaximum => {
                            assert!(!step.contains("allow more time"), "{context}: {step}");
                            assert!(!step.contains("up to 60"), "{context}: {step}");
                            assert!(
                                step.contains(
                                    "; --timeout-seconds cannot allow more. Run setup check again. If "
                                ),
                                "{context}: {step}"
                            );
                            assert!(
                                step.ends_with("hangs: run it yourself to see."),
                                "{context}: {step}"
                            );
                        }
                    }
                    // A tool that responded has no remediation at all.
                    for other in (0..3).filter(|other| *other != index) {
                        assert_eq!(
                            dependency(&response, other)?["remediation"],
                            Value::Null,
                            "{context}"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

/// A tool that was never started is not said to have given no answer, and a
/// tool that gave no answer is not said to have been left unstarted.
#[test]
fn the_two_kinds_of_timeout_are_told_apart_in_words_and_share_one_reason() -> TestResult {
    let response = check(
        &reported_shape(Duration::from_secs(5)),
        ManagedPlanAvailability::TargetUnavailable,
    )?;
    let no_answer = next_step(dependency(&response, 0)?)?;
    assert!(
        no_answer.starts_with(
            "FFmpeg was found and started but did not answer before the deadline all tool checks share (5 seconds)."
        ),
        "{no_answer}"
    );
    assert!(!no_answer.contains("not started"), "{no_answer}");
    for (index, name) in [(1, "FFprobe"), (2, "whisper.cpp")] {
        let not_started = next_step(dependency(&response, index)?)?;
        assert!(
            not_started.starts_with(&format!(
                "{name} was found but not started: the deadline all tool checks share (5 seconds) had already passed."
            )),
            "{not_started}"
        );
        assert!(!not_started.contains("did not answer"), "{not_started}");
    }
    for index in 0..3 {
        let item = dependency(&response, index)?;
        assert_eq!(item["status"], "timed_out");
        assert_eq!(item["remediation"]["reason"], "timed_out");
    }
    Ok(())
}

/// "Allow more time (up to 60)" is not advice for a caller who already gave
/// the probes sixty seconds. One second less still offers it; at the
/// option's maximum, and beyond it for a library host, the step says the
/// option cannot allow more, to run the check again, and what a tool that
/// never answers means. The figure in the advice is the option's maximum.
#[test]
fn after_the_longest_deadline_more_time_is_not_offered() -> TestResult {
    assert_eq!(MAX_PROBE_TIMEOUT, Duration::from_secs(60));
    assert_eq!(
        ProbeTimeHeadroom::of(Duration::from_secs(59)),
        ProbeTimeHeadroom::MoreTimeAllowed
    );
    for at_most in [MAX_PROBE_TIMEOUT, Duration::from_secs(61), Duration::MAX] {
        assert_eq!(
            ProbeTimeHeadroom::of(at_most),
            ProbeTimeHeadroom::AtTheMaximum
        );
    }

    let below = check(
        &reported_shape(Duration::from_secs(59)),
        ManagedPlanAvailability::Qualified,
    )?;
    assert!(
        next_step(dependency(&below, 0)?)?.ends_with(&format!(
            "or allow more time with --timeout-seconds (up to {}).",
            MAX_PROBE_TIMEOUT.as_secs()
        )),
        "{below}"
    );

    let at_the_maximum = check(
        &reported_shape(MAX_PROBE_TIMEOUT),
        ManagedPlanAvailability::Qualified,
    )?;
    validate("setup-check-response.schema.json", &at_the_maximum)?;
    assert_eq!(
        next_step(dependency(&at_the_maximum, 0)?)?,
        "FFmpeg was found and started but did not answer before the deadline all tool checks share (60 seconds); --timeout-seconds cannot allow more. Run setup check again. If it never answers, the tool itself hangs: run it yourself to see."
    );
    assert_eq!(
        next_step(dependency(&at_the_maximum, 1)?)?,
        "FFprobe was found but not started: the deadline all tool checks share (60 seconds) had already passed; --timeout-seconds cannot allow more. Run setup check again. If a tool never answers, that tool hangs: run it yourself to see."
    );
    assert_eq!(
        next_step(dependency(&at_the_maximum, 2)?)?,
        "whisper.cpp was found but not started: the deadline all tool checks share (60 seconds) had already passed; --timeout-seconds cannot allow more. Run setup check again. If a tool never answers, that tool hangs: run it yourself to see."
    );
    for index in 0..3 {
        let item = dependency(&at_the_maximum, index)?;
        assert_eq!(item["remediation"]["reason"], "timed_out");
        assert_no_install_advice(next_step(item)?, "at the maximum");
    }
    Ok(())
}

/// The advice for a missing tool, and for one that was found and failed its
/// probe, is the published text, unchanged: for those two it is true.
#[test]
fn a_missing_or_failing_tool_keeps_its_published_advice() -> TestResult {
    let published = [
        "Install or locate a trusted FFmpeg executable, then rerun setup check.",
        "Install or locate a trusted FFprobe executable, then rerun setup check.",
        "Install or locate a trusted whisper.cpp CLI executable for local ASR, then rerun setup check. A supplied transcript can skip local ASR.",
    ];
    for (state, reason) in [
        (DependencyState::Missing, "missing"),
        (
            DependencyState::Unhealthy {
                message: String::from("process exited with exit code: 3"),
            },
            "unhealthy",
        ),
    ] {
        let response = check(
            &diagnosis(
                [state.clone(), state.clone(), state],
                RuntimeReadiness::Blocked,
            ),
            ManagedPlanAvailability::Qualified,
        )?;
        validate("setup-check-response.schema.json", &response)?;
        for (index, text) in published.into_iter().enumerate() {
            let item = dependency(&response, index)?;
            assert_eq!(item["remediation"]["reason"], reason);
            assert_eq!(next_step(item)?, text, "{reason}");
        }
    }
    Ok(())
}

/// The deadline is named as the command line applies it, and stays short
/// and true for the durations only a library host could pass.
#[test]
fn a_deadline_is_named_in_whole_seconds_or_milliseconds_and_bounded() {
    for (deadline, text) in [
        (Duration::ZERO, "0 seconds"),
        (Duration::from_secs(1), "1 second"),
        (Duration::from_secs(2), "2 seconds"),
        (Duration::from_secs(30), "30 seconds"),
        (Duration::from_secs(60), "60 seconds"),
        (Duration::from_secs(3_600), "3600 seconds"),
        (Duration::from_millis(1_500), "1500 ms"),
        (Duration::from_nanos(1), "1 ms"),
        (Duration::from_micros(2_000_001), "2001 ms"),
        (Duration::from_millis(3_600_001), "over an hour"),
        (Duration::MAX, "over an hour"),
    ] {
        assert_eq!(probe_deadline_text(deadline), text, "{deadline:?}");
    }
}

/// A plan as `setup plan` presents it on a machine with no managed tier.
fn present(plan: &ManagedSetupPlan) -> SetupPlanResponse {
    SetupPlanResponse::new(plan, &ManagedPlanObservation::without_managed_tier(plan))
}

fn plan_where_nothing_can_be_installed(states: [DependencyState; 3]) -> ManagedSetupPlan {
    plan_managed_setup(
        SetupProfile::Desktop,
        diagnosis(states, RuntimeReadiness::Blocked),
        ManagedTarget::WindowsX86_64,
        SetupSelectionState::default(),
        1_800_000_000,
        None,
    )
}

/// `setup plan` after the same slow start: the two tools that were found are
/// not described as missing, the plan says it decided nothing for them and
/// to check and plan again, and a tool that really is missing keeps its
/// published step.
#[test]
fn a_plan_made_while_probes_timed_out_matches_the_frozen_example() -> TestResult {
    let plan = plan_where_nothing_can_be_installed([
        timed_out(ProbeTimeoutKind::NoAnswer, Duration::from_secs(30)),
        timed_out(ProbeTimeoutKind::NotStarted, Duration::from_secs(30)),
        DependencyState::Missing,
    ]);
    let response =
        serde_json::to_value(OperationResponse::complete("setup.plan", &present(&plan))?)?;

    validate("setup-plan.schema.json", &response)?;
    assert_eq!(response, load("examples/setup-plan.timed-out.json")?);
    Ok(())
}

/// The plan's step for a timed-out probe, for each dependency and both kinds:
/// one text for both, with no install advice and no deadline in it, within
/// the 512 characters the published schema allows a step.
#[test]
fn a_plan_never_tells_the_owner_of_a_timed_out_tool_to_install_it() -> TestResult {
    for (index, (tool, name)) in RuntimeDependency::ALL
        .into_iter()
        .zip(["FFmpeg", "FFprobe", "whisper.cpp"])
        .enumerate()
    {
        let mut steps = Vec::new();
        for kind in KINDS {
            let mut states = [available(), available(), available()];
            states[index] = timed_out(kind, Duration::from_secs(30));
            let plan = plan_where_nothing_can_be_installed(states);
            let response =
                serde_json::to_value(OperationResponse::complete("setup.plan", &present(&plan))?)?;
            validate("setup-plan.schema.json", &response)?;
            let item = &response["data"]["dependencies"][index];
            assert_eq!(item["status"], "timed_out", "{tool:?}");
            assert_eq!(item["disposition"], "manual_selection_required");
            assert_eq!(item["required_authority"], "user");
            let step = item["next_step"]
                .as_str()
                .ok_or("the plan has no next step")?
                .to_owned();
            assert_no_install_advice(&step, &format!("{tool:?} {kind:?}"));
            assert!(
                step.starts_with(&format!("{name} was found, but its check did not finish")),
                "{step}"
            );
            assert!(step.contains("proposes no install because of it"), "{step}");
            assert!(step.ends_with("then run setup plan again."), "{step}");
            assert!(!step.contains("30 seconds"), "{step}");
            assert!(step.chars().count() <= 512, "{step}");
            steps.push(step);
        }
        assert_eq!(steps[0], steps[1], "one text for both kinds: {tool:?}");
    }
    Ok(())
}

/// The next step is part of what acceptance compares, so it must follow the
/// plan's own probe and never what the machine shows later: a plan saved
/// while a probe timed out is still the same plan when it is rebuilt from
/// the same observation, and no longer the same once the tool has answered.
#[test]
fn a_saved_plan_with_a_timed_out_probe_is_accepted_only_while_the_probe_still_times_out()
-> TestResult {
    let timed_out_states = || {
        [
            timed_out(ProbeTimeoutKind::NoAnswer, Duration::from_secs(30)),
            timed_out(ProbeTimeoutKind::NotStarted, Duration::from_secs(30)),
            DependencyState::Missing,
        ]
    };
    let original = plan_where_nothing_can_be_installed(timed_out_states());
    let bytes = serde_json::to_vec(&OperationResponse::complete(
        "setup.plan",
        &present(&original),
    )?)?;
    let saved: SavedSetupPlan = serde_json::from_slice(&bytes)?;
    assert_eq!(saved.validate_envelope(), Ok(()));

    let again = plan_where_nothing_can_be_installed(timed_out_states());
    assert_eq!(saved.require_same_plan(&present(&again)), Ok(()));

    // The same observation with the other kind of timeout and another
    // deadline is the same plan: neither is part of its intent.
    let other_kind = plan_where_nothing_can_be_installed([
        timed_out(ProbeTimeoutKind::NotStarted, Duration::from_secs(5)),
        timed_out(ProbeTimeoutKind::NoAnswer, Duration::from_secs(5)),
        DependencyState::Missing,
    ]);
    assert_eq!(saved.require_same_plan(&present(&other_kind)), Ok(()));

    // What the machine shows now is not compared: the same plan observed
    // with the tools answering (a managed version, say) is still accepted,
    // and its step still describes what the plan's own probe saw.
    let observed_later = SetupPlanResponse::new(
        &again,
        &ManagedPlanObservation {
            readiness: RuntimeReadiness::Degraded,
            dependencies: diagnosis(
                [available(), available(), DependencyState::Missing],
                RuntimeReadiness::Degraded,
            )
            .dependencies,
            ..ManagedPlanObservation::without_managed_tier(&again)
        },
    );
    assert_eq!(saved.require_same_plan(&observed_later), Ok(()));
    let later = serde_json::to_value(&observed_later)?;
    assert_eq!(later["dependencies"][0]["status"], "available");
    assert!(
        later["dependencies"][0]["next_step"]
            .as_str()
            .is_some_and(|step| step.starts_with("FFmpeg was found, but its check did not finish")),
        "{later}"
    );

    // Once the plan's own probe sees the tool answer, it is another plan.
    let answered =
        plan_where_nothing_can_be_installed([available(), available(), DependencyState::Missing]);
    assert_eq!(
        saved.require_same_plan(&present(&answered)),
        Err(FailureCode::InvalidArgument)
    );
    Ok(())
}
