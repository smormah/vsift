//! Scripted terminal tests of the setup door (ADR 0036 section 11): a
//! scripted person at a fake console, and setup steps that answer from the
//! published v1 examples, so the whole conversation runs without a
//! terminal, a tool on the machine or a network.
//!
//! The snapshots in `crates/vsift-cli/tests/human_output/setup-door-*.txt`
//! are what a person reads; `VSIFT_UPDATE_HUMAN_SNAPSHOTS=1` rewrites them.
//! They hold standard output only: on a real terminal the typed answer and
//! its Enter, echoed by the terminal, sit between the question and what
//! follows it.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    future::{Future, ready},
};

use serde_json::{Value, json};
use vsift::{
    Cancellation, CorePart, CorePartSet, DependencyState, DependencyStatus, FailureCode,
    LocalAsrCheckOutcome, LocalAsrModelStatus, LocalAsrNotRunReason, LocalAsrSetupStatus,
    ManagedComponent, ManagedPlanAvailability, OptionalCapability, OptionalComponent,
    OptionalComponentSet, PieceProgress, PieceStage, ProbeTimeout, ProbeTimeoutKind,
    ProgressObserver, ReviewedAsrModel, RuntimeDependency, RuntimeDiagnosis, RuntimeReadiness,
};
use vsift_contract::DependencyLookup;

use super::{
    Answer, CheckedSetup, Console, DoorOptions, DoorRun, InstallResults, QuestionLine, Reply,
    ReviewedPlan, SetupSteps, run,
};
use crate::{
    CommandFailure,
    command::DoorExtra,
    human::{
        self, Styling,
        tests::{check_snapshot, example},
    },
    live_line::LiveLine,
    output::{OutputMode, OutputWriter, ProcessExit},
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The version in the heading of every snapshot.
const VERSION: &str = "0.0.0-test";

/// A person at the fake terminal, or nobody.
struct ScriptedConsole {
    present: bool,
    styling: Styling,
    reply: Reply,
    /// Presses Ctrl-C while the question waits: the cancellation fires,
    /// whatever was typed.
    interrupts: bool,
    asked: Cell<u32>,
}

impl ScriptedConsole {
    /// A person who types `answer` and presses Enter.
    fn person(answer: Answer) -> Self {
        Self {
            present: true,
            styling: Styling::Plain,
            reply: Reply {
                answer,
                line: QuestionLine::Ended,
            },
            interrupts: false,
            asked: Cell::new(0),
        }
    }

    fn nobody() -> Self {
        Self {
            present: false,
            ..Self::person(Answer::Yes)
        }
    }
}

impl Console for ScriptedConsole {
    fn person_present(&self) -> bool {
        self.present
    }

    fn styling(&self) -> Styling {
        self.styling
    }

    fn live_line(&self) -> LiveLine {
        LiveLine::Hidden
    }

    fn answer(&mut self, cancellation: &Cancellation) -> impl Future<Output = Reply> {
        self.asked.set(self.asked.get() + 1);
        if self.interrupts {
            cancellation.cancel();
        }
        ready(self.reply)
    }
}

/// Setup steps that answer from a script and count what was asked of them.
struct ScriptedSteps {
    checks: RefCell<VecDeque<CheckedSetup>>,
    plan: Option<Value>,
    install: RefCell<Option<Result<InstallResults, CommandFailure>>>,
    planned: Cell<u32>,
    planned_parts: Cell<Option<CorePartSet>>,
    installed: Cell<u32>,
    /// What the install reports of its pieces, in order.
    pieces: Vec<PieceProgress>,
}

impl ScriptedSteps {
    fn new(checks: Vec<CheckedSetup>) -> Self {
        Self {
            checks: RefCell::new(checks.into()),
            plan: None,
            install: RefCell::new(None),
            planned: Cell::new(0),
            planned_parts: Cell::new(None),
            installed: Cell::new(0),
            pieces: Vec::new(),
        }
    }

    fn with_pieces(mut self, pieces: Vec<PieceProgress>) -> Self {
        self.pieces = pieces;
        self
    }

    fn with_plan(mut self, plan: Value) -> Self {
        self.plan = Some(plan);
        self
    }

    fn with_install(self, install: Result<InstallResults, CommandFailure>) -> Self {
        *self.install.borrow_mut() = Some(install);
        self
    }
}

impl SetupSteps for ScriptedSteps {
    fn check(&self) -> impl Future<Output = Result<CheckedSetup, CommandFailure>> {
        ready(
            self.checks
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| CommandFailure::from(FailureCode::Internal)),
        )
    }

    fn plan(
        &self,
        parts: CorePartSet,
        _include: OptionalComponentSet,
    ) -> impl Future<Output = Result<ReviewedPlan, CommandFailure>> {
        self.planned.set(self.planned.get() + 1);
        self.planned_parts.set(Some(parts));
        ready(
            self.plan
                .clone()
                .ok_or_else(|| CommandFailure::from(FailureCode::Internal))
                .and_then(|envelope| ReviewedPlan::from_envelope(envelope, parts)),
        )
    }

    fn install(
        &self,
        _plan: &ReviewedPlan,
        _cancellation: &Cancellation,
        progress: ProgressObserver,
    ) -> impl Future<Output = Result<InstallResults, CommandFailure>> {
        self.installed.set(self.installed.get() + 1);
        for piece in &self.pieces {
            progress.install_piece(piece);
        }
        ready(
            self.install
                .borrow_mut()
                .take()
                .unwrap_or_else(|| Err(CommandFailure::from(FailureCode::Internal))),
        )
    }
}

fn found() -> DependencyState {
    DependencyState::Available {
        version: String::from("fixture 1"),
    }
}

const SPEECH_WORKS: LocalAsrSetupStatus = LocalAsrSetupStatus {
    model: LocalAsrModelStatus::KnownPinned(ReviewedAsrModel::BaseQ5_1),
    verification: LocalAsrCheckOutcome::Verified(vsift::LocalAsrVerificationSource::Recorded),
};

const NO_SPEECH: LocalAsrSetupStatus = LocalAsrSetupStatus {
    model: LocalAsrModelStatus::NotSelected,
    verification: LocalAsrCheckOutcome::NotRun(LocalAsrNotRunReason::WhisperUnavailable),
};

const NOTHING_YET: LocalAsrSetupStatus = LocalAsrSetupStatus {
    model: LocalAsrModelStatus::NotSelected,
    verification: LocalAsrCheckOutcome::NotRun(LocalAsrNotRunReason::MediaToolsUnavailable),
};

/// A check that found `states`, where managed installs are `managed` and
/// web videos stand as `web_videos`.
fn checked(
    states: [DependencyState; 3],
    local_asr: LocalAsrSetupStatus,
    managed: ManagedPlanAvailability,
    web_videos: OptionalCapability,
) -> CheckedSetup {
    let media_missing = states[..2]
        .iter()
        .any(|state| !matches!(state, DependencyState::Available { .. }));
    let complete = !media_missing && local_asr == SPEECH_WORKS;
    CheckedSetup {
        diagnosis: RuntimeDiagnosis {
            readiness: if media_missing {
                RuntimeReadiness::Blocked
            } else if complete {
                RuntimeReadiness::Ready
            } else {
                RuntimeReadiness::Degraded
            },
            dependencies: RuntimeDependency::ALL
                .into_iter()
                .zip(states)
                .map(|(dependency, state)| DependencyStatus { dependency, state })
                .collect(),
        },
        local_asr,
        availability: managed,
        by_dependency: [managed; 3],
        updates: Vec::new(),
        optional: vec![(OptionalComponent::OnlineLinks, web_videos)],
        lookups: [DependencyLookup::FilteredPath; 3],
    }
}

fn video_ready_speech_missing(managed: ManagedPlanAvailability) -> CheckedSetup {
    checked(
        [found(), found(), DependencyState::Missing],
        NO_SPEECH,
        managed,
        OptionalCapability::NotInstalled,
    )
}

fn all_ready() -> CheckedSetup {
    checked(
        [found(), found(), found()],
        SPEECH_WORKS,
        ManagedPlanAvailability::Qualified,
        OptionalCapability::NotInstalled,
    )
}

/// One reviewed action of a plan, as `setup plan --json` publishes it.
fn action(component: &str, version: &str, publisher: &str, bytes: u64) -> Value {
    json!({
        "id": format!("install-{component}"),
        "state": "pending",
        "component": component,
        "version": version,
        "publisher": publisher,
        "source_url": format!("https://example.invalid/{component}"),
        "bytes": bytes,
        "sha256": "b".repeat(64),
        "format": "raw_file",
        "archive_limits": null,
        "selected_files": [],
        "archive_links": [],
        "runtime_copies": [],
        "licence": "MIT",
        "notice_url": "https://example.invalid/LICENSE",
        "source_code_url": "https://example.invalid/source",
        "trust_limit": "Publisher bytes pinned by digest.",
        "licence_scope": "disclosure_not_legal_clearance",
        "destination": "private_per_user_managed_runtime",
        "permissions": "private_user_only",
        "change": "planned_download_verify_extract_smoke_activate",
        "required_authority": "user",
        "files": [{"name": component, "bytes": bytes, "sha256": "b".repeat(64), "mode": "owner_read_write"}]
    })
}

/// A plan of the reviewed catalogue that installs `actions`.
fn plan(actions: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let mut plan = example("setup-plan.unavailable.json")?;
    plan["data"]["target"] = json!("ubuntu_24_04_x86_64");
    plan["data"]["managed_install"] = json!("catalogue_accepted");
    plan["data"]["catalogue_revision"] = json!("2026-09-20.1");
    plan["data"]["plan_digest"] = json!("a".repeat(64));
    plan["data"]["install_needed"] = json!(true);
    plan["data"]["actions"] = Value::Array(actions);
    Ok(plan)
}

fn speech_plan() -> Result<Value, Box<dyn std::error::Error>> {
    plan(vec![
        action("whisper_cli", "whisper.cpp-v1.9.2", "ggml-org", 9_412_000),
        action("whisper_model", "base-q5_1", "ggml-org", 59_707_625),
    ])
}

/// The `data` of a `setup.install` example, with its components replaced.
fn install_results(
    components: Value,
    failure: Option<FailureCode>,
) -> Result<InstallResults, Box<dyn std::error::Error>> {
    let mut data = example("setup-install.json")?["data"].take();
    data["components"] = components;
    InstallResults::from_data(data, failure).map_err(|_| "the install result does not read".into())
}

fn component(name: &str, status: &str, step: Option<&str>, reason: Option<&str>) -> Value {
    json!({
        "component": name,
        "version": "fixture",
        "status": status,
        "step": step,
        "reason": reason,
        "failure_code": null,
        "smoke_check": null,
        "stage": null,
        "retention_reason": null
    })
}

/// Runs the door and returns its exit status and what it wrote.
async fn converse(
    console: &mut ScriptedConsole,
    steps: &ScriptedSteps,
    include: Vec<DoorExtra>,
) -> Result<(ProcessExit, String, String), Box<dyn std::error::Error>> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let cancellation = Cancellation::new();
    let exit = {
        let mut writer = OutputWriter::new(&mut stdout, &mut stderr);
        run(
            DoorRun {
                console,
                steps,
                options: DoorOptions { include },
                version: VERSION,
                cancellation: &cancellation,
                trap_interruptions: false,
            },
            &mut writer,
        )
        .await
    };
    Ok((exit, String::from_utf8(stdout)?, String::from_utf8(stderr)?))
}

fn assert_plain(text: &str) {
    assert!(
        text.chars()
            .all(|character| character == '\n' || !character.is_control()),
        "{text:?}"
    );
    assert!(
        !text.contains('\u{2713}') && !text.contains('\u{2717}'),
        "{text}"
    );
}

/// Video works, speech is missing, `VSift` can install it: one offer naming
/// each piece's publisher, size and licence, one question, and on yes the
/// install, the results and the table again, ending with what to try.
#[tokio::test]
async fn yes_installs_what_was_offered_and_ends_complete() -> TestResult {
    let steps = ScriptedSteps::new(vec![
        video_ready_speech_missing(ManagedPlanAvailability::Qualified),
        all_ready(),
    ])
    .with_plan(speech_plan()?)
    .with_install(Ok(install_results(
        json!([
            component("whisper_cli", "activated", None, None),
            component("whisper_model", "activated", None, None),
        ]),
        None,
    )?));
    let mut console = ScriptedConsole::person(Answer::Yes);

    let (exit, stdout, stderr) = converse(&mut console, &steps, Vec::new()).await?;

    check_snapshot("setup-door-yes", &stdout)?;
    assert_plain(&stdout);
    assert_eq!(exit, ProcessExit::Success);
    assert_eq!((console.asked.get(), steps.installed.get()), (1, 1));
    assert!(stdout.contains("Install speech now? [y/N] "), "{stdout}");
    assert!(
        stdout.ends_with("Try it: vsift ingest ./your-video.mp4\n"),
        "{stdout}"
    );
    assert!(stderr.is_empty(), "{stderr}");
    Ok(())
}

/// No (Enter, `n`, a closed input) installs nothing, and the last line
/// says how to come back.
#[tokio::test]
async fn no_installs_nothing_and_says_how_to_come_back() -> TestResult {
    let steps = ScriptedSteps::new(vec![video_ready_speech_missing(
        ManagedPlanAvailability::Qualified,
    )])
    .with_plan(speech_plan()?);
    let mut console = ScriptedConsole::person(Answer::No);

    let (exit, stdout, _) = converse(&mut console, &steps, Vec::new()).await?;

    check_snapshot("setup-door-no", &stdout)?;
    assert_eq!(steps.installed.get(), 0);
    assert_eq!(exit, ProcessExit::Success);
    assert!(
        stdout.ends_with("Nothing was installed. Run vsift setup again when you're ready.\n"),
        "{stdout}"
    );
    Ok(())
}

/// A typed answer's Enter is the only break after the question; with no
/// typed answer (the time limit, Ctrl-C, a closed input) the door breaks the
/// line itself, so the close never sits on the question's line.
#[tokio::test]
async fn the_close_starts_on_the_line_after_the_question() -> TestResult {
    const QUESTION: &str = "Install speech now? [y/N] ";
    const CLOSE: &str = "Nothing was installed.";
    for (reply, between) in [
        (
            Reply {
                answer: Answer::No,
                line: QuestionLine::Ended,
            },
            "",
        ),
        (Reply::UNANSWERED, "\n"),
    ] {
        let steps = ScriptedSteps::new(vec![video_ready_speech_missing(
            ManagedPlanAvailability::Qualified,
        )])
        .with_plan(speech_plan()?);
        let mut console = ScriptedConsole {
            reply,
            ..ScriptedConsole::person(Answer::No)
        };

        let (_, stdout, _) = converse(&mut console, &steps, Vec::new()).await?;

        assert!(
            stdout.contains(&format!("{QUESTION}{between}{CLOSE}")),
            "{reply:?}: {stdout:?}"
        );
    }
    Ok(())
}

/// Ctrl-C at the question is a no, even when something was typed.
#[tokio::test]
async fn ctrl_c_at_the_question_installs_nothing() -> TestResult {
    let steps = ScriptedSteps::new(vec![video_ready_speech_missing(
        ManagedPlanAvailability::Qualified,
    )])
    .with_plan(speech_plan()?);
    let mut console = ScriptedConsole {
        interrupts: true,
        ..ScriptedConsole::person(Answer::Yes)
    };

    let (_, stdout, _) = converse(&mut console, &steps, Vec::new()).await?;

    assert_eq!(steps.installed.get(), 0);
    assert!(stdout.ends_with("Nothing was installed. Run vsift setup again when you're ready.\n"));
    Ok(())
}

/// Only `y` and `yes` are yes.
#[test]
fn only_y_and_yes_are_yes() {
    for line in ["y", "Y\n", "yes", " YES \r\n", "Yes"] {
        assert_eq!(Answer::of(line), Answer::Yes, "{line:?}");
    }
    for line in [
        "",
        "\n",
        "n",
        "no",
        "yeah",
        "y es",
        "yes please",
        "\u{1b}[A",
    ] {
        assert_eq!(Answer::of(line), Answer::No, "{line:?}");
    }
}

/// A piece that fails: video is installed, speech's program fails to
/// download, its model is not tried. What finished stays, the table says
/// where the person stands, and the last line says what to do.
#[tokio::test]
async fn a_failed_piece_keeps_what_finished_and_says_what_to_do() -> TestResult {
    let nothing = checked(
        [
            DependencyState::Missing,
            DependencyState::Missing,
            DependencyState::Missing,
        ],
        NOTHING_YET,
        ManagedPlanAvailability::Qualified,
        OptionalCapability::NotInstalled,
    );
    let steps = ScriptedSteps::new(vec![
        nothing,
        video_ready_speech_missing(ManagedPlanAvailability::Qualified),
    ])
    .with_plan(plan(vec![
        action("ffmpeg_ffprobe", "n9.0.1", "BtbN", 112_640_000),
        action("whisper_cli", "whisper.cpp-v1.9.2", "ggml-org", 9_412_000),
        action("whisper_model", "base-q5_1", "ggml-org", 59_707_625),
    ])?)
    .with_install(Ok(install_results(
        json!([
            component("ffmpeg_ffprobe", "activated", None, None),
            component("whisper_cli", "failed", Some("download"), Some("offline")),
            component("whisper_model", "failed", None, Some("blocked")),
        ]),
        Some(FailureCode::DownloadFailed),
    )?));
    let mut console = ScriptedConsole::person(Answer::Yes);

    let (exit, stdout, _) = converse(&mut console, &steps, Vec::new()).await?;

    check_snapshot("setup-door-failed", &stdout)?;
    assert_eq!(exit, ProcessExit::from(FailureCode::DownloadFailed.class()));
    assert!(
        stdout.contains("Install video and speech now? [y/N] "),
        "{stdout}"
    );
    assert!(
        stdout.contains("whisper.cpp   the download stopped: no connection"),
        "{stdout}"
    );
    assert!(
        stdout.ends_with("Next: check your connection, then run  vsift setup\n"),
        "{stdout}"
    );
    Ok(())
}

/// When video fails first, speech is waiting for video, not failed.
#[tokio::test]
async fn speech_after_a_failed_video_is_waiting_for_video() -> TestResult {
    let nothing = checked(
        [
            DependencyState::Missing,
            DependencyState::Missing,
            DependencyState::Missing,
        ],
        NOTHING_YET,
        ManagedPlanAvailability::Qualified,
        OptionalCapability::NotInstalled,
    );
    let steps = ScriptedSteps::new(vec![nothing.clone(), nothing])
        .with_plan(plan(vec![
            action("ffmpeg_ffprobe", "n9.0.1", "BtbN", 112_640_000),
            action("whisper_cli", "whisper.cpp-v1.9.2", "ggml-org", 9_412_000),
        ])?)
        .with_install(Ok(install_results(
            json!([
                component(
                    "ffmpeg_ffprobe",
                    "failed",
                    Some("smoke"),
                    Some("provider_failed")
                ),
                component("whisper_cli", "failed", None, Some("blocked")),
            ]),
            Some(FailureCode::MissingCapability),
        )?));
    let mut console = ScriptedConsole::person(Answer::Yes);

    let (_, stdout, _) = converse(&mut console, &steps, Vec::new()).await?;

    assert!(stdout.contains("waiting for video"), "{stdout}");
    assert!(stdout.ends_with("Next: vsift setup\n"), "{stdout}");
    assert!(stdout.contains("troubleshooting guide"), "{stdout}");
    Ok(())
}

/// Where `VSift` cannot install a missing part, nothing is offered for it:
/// one line says so, the guide is linked, and the last line is the one
/// command that registers a tool the person installs themselves.
#[tokio::test]
async fn a_part_vsift_cannot_install_here_gets_the_do_it_yourself_step() -> TestResult {
    let steps = ScriptedSteps::new(vec![video_ready_speech_missing(
        ManagedPlanAvailability::TargetUnavailable,
    )]);
    let mut console = ScriptedConsole::person(Answer::Yes);

    let (exit, stdout, _) = converse(&mut console, &steps, Vec::new()).await?;

    check_snapshot("setup-door-cannot-install", &stdout)?;
    assert_eq!((console.asked.get(), steps.planned.get()), (0, 0));
    assert_eq!(exit, ProcessExit::Success);
    assert!(
        stdout.contains("VSift can't install speech on this computer yet.\n"),
        "{stdout}"
    );
    assert!(
        stdout.ends_with(
            "Next: vsift setup configure whisper --executable <full-path-to-whisper-cli>\n"
        ),
        "{stdout}"
    );
    Ok(())
}

/// Issue #396 at the door: for each tool, missing or found and failing,
/// where `VSift` cannot install it, the last line is the one command that
/// registers it, never advice to look for it.
#[tokio::test]
async fn each_tool_vsift_cannot_install_ends_with_its_registration_command() -> TestResult {
    let commands = [
        "Next: vsift setup configure ffmpeg --executable <full-path-to-ffmpeg>\n",
        "Next: vsift setup configure ffprobe --executable <full-path-to-ffprobe>\n",
        "Next: vsift setup configure whisper --executable <full-path-to-whisper-cli>\n",
    ];
    for (index, command) in commands.into_iter().enumerate() {
        for state in [
            DependencyState::Missing,
            DependencyState::Unhealthy {
                message: String::from("process exited with exit code: 3"),
            },
        ] {
            let mut states = [found(), found(), found()];
            states[index] = state;
            let steps = ScriptedSteps::new(vec![checked(
                states,
                SPEECH_WORKS,
                ManagedPlanAvailability::TargetUnavailable,
                OptionalCapability::Unavailable,
            )]);
            let mut console = ScriptedConsole::person(Answer::Yes);

            let (_, stdout, _) = converse(&mut console, &steps, Vec::new()).await?;

            assert!(stdout.ends_with(command), "{index}: {stdout}");
            assert!(
                !stdout.contains("locate") && !stdout.contains("search"),
                "{stdout}"
            );
            assert_eq!(console.asked.get(), 0);
        }
    }
    Ok(())
}

/// Nobody at a terminal: the read-only table, no question and no plan,
/// and the exit status `setup check` gives.
#[tokio::test]
async fn without_a_terminal_it_reports_and_never_asks() -> TestResult {
    let steps = ScriptedSteps::new(vec![checked(
        [
            DependencyState::Missing,
            DependencyState::Missing,
            DependencyState::Missing,
        ],
        NOTHING_YET,
        ManagedPlanAvailability::Qualified,
        OptionalCapability::NotInstalled,
    )])
    .with_plan(speech_plan()?);
    let mut console = ScriptedConsole::nobody();

    let (exit, stdout, _) = converse(&mut console, &steps, Vec::new()).await?;

    check_snapshot("setup-door-no-terminal", &stdout)?;
    assert_plain(&stdout);
    assert_eq!(exit, ProcessExit::UsageOrCapability);
    assert_eq!(
        (
            console.asked.get(),
            steps.planned.get(),
            steps.installed.get()
        ),
        (0, 0, 0)
    );
    assert!(!stdout.contains("[y/N]"), "{stdout}");
    Ok(())
}

/// Already complete: the table, the extras line, and what to try.
#[tokio::test]
async fn a_complete_setup_says_so_and_names_the_extras() -> TestResult {
    let steps = ScriptedSteps::new(vec![all_ready()]);
    let mut console = ScriptedConsole::person(Answer::Yes);

    let (exit, stdout, _) = converse(&mut console, &steps, Vec::new()).await?;

    check_snapshot("setup-door-complete", &stdout)?;
    assert_eq!(exit, ProcessExit::Success);
    assert_eq!((console.asked.get(), steps.planned.get()), (0, 0));
    assert!(
        stdout.contains(
            "Optional extras: web videos. Add one with: vsift setup --include web-videos\n"
        ),
        "{stdout}"
    );
    Ok(())
}

/// Checks that ran out of time are not an offer: nothing is planned, and
/// the step is to check again, where more time can be allowed.
#[tokio::test]
async fn timed_out_checks_offer_nothing() -> TestResult {
    let timed_out = DependencyState::TimedOut(ProbeTimeout {
        kind: ProbeTimeoutKind::NoAnswer,
        shared_deadline: std::time::Duration::from_secs(30),
    });
    let steps = ScriptedSteps::new(vec![checked(
        [found(), found(), timed_out],
        NO_SPEECH,
        ManagedPlanAvailability::Qualified,
        OptionalCapability::NotInstalled,
    )])
    .with_plan(speech_plan()?);
    let mut console = ScriptedConsole::person(Answer::Yes);

    let (_, stdout, _) = converse(&mut console, &steps, Vec::new()).await?;

    assert_eq!((console.asked.get(), steps.planned.get()), (0, 0));
    assert!(stdout.ends_with("Next: vsift setup check\n"), "{stdout}");
    Ok(())
}

/// An extra the person asked for, on a complete setup: the offer covers
/// the extra alone, with its own words, and the question says "Add".
#[tokio::test]
async fn an_included_extra_is_offered_on_its_own() -> TestResult {
    let mut plan = plan(Vec::new())?;
    plan["data"]["optional_components"] = json!([{
        "name": "online_links",
        "status": "included",
        "summary": "yt-dlp (GPL-3.0-or-later, with the MIT-licensed Deno runtime) fetches videos from web pages when you give VSift a link. Downloading is subject to each site's terms.",
        "add_command": null,
        "actions": [
            action("js_runtime_deno", "2.5.0", "denoland", 41_000_000),
            action("yt_dlp", "2026.09.01", "yt-dlp", 3_100_000),
        ],
        "next_step": "Ask the operator before accepting."
    }]);
    let steps = ScriptedSteps::new(vec![all_ready()]).with_plan(plan);
    let mut console = ScriptedConsole::person(Answer::No);

    let (_, stdout, _) = converse(&mut console, &steps, vec![DoorExtra::WebVideos]).await?;

    assert_eq!(steps.planned.get(), 1);
    assert!(
        stdout.contains("VSift can add web videos for you:"),
        "{stdout}"
    );
    assert!(stdout.contains("subject to each site's terms"), "{stdout}");
    assert!(stdout.contains("Add web videos now? [y/N] "), "{stdout}");
    Ok(())
}

/// The colour fallback: on a terminal that takes colour, each status has a
/// symbol and a colour; plain, the same words and nothing else.
#[tokio::test]
async fn symbols_and_colour_only_when_the_terminal_takes_them() -> TestResult {
    let mut rich = ScriptedConsole {
        styling: Styling::Rich,
        ..ScriptedConsole::person(Answer::No)
    };
    let steps = ScriptedSteps::new(vec![video_ready_speech_missing(
        ManagedPlanAvailability::TargetUnavailable,
    )]);
    let (_, styled, _) = converse(&mut rich, &steps, Vec::new()).await?;
    let steps = ScriptedSteps::new(vec![video_ready_speech_missing(
        ManagedPlanAvailability::TargetUnavailable,
    )]);
    let mut plain = ScriptedConsole::person(Answer::No);
    let (_, unstyled, _) = converse(&mut plain, &steps, Vec::new()).await?;

    assert!(
        styled.contains("\u{1b}[32m\u{2713} ready\u{1b}[0m"),
        "{styled:?}"
    );
    assert!(
        styled.contains("\u{1b}[31m\u{2717} not set up\u{1b}[0m"),
        "{styled:?}"
    );
    assert_plain(&unstyled);
    let stripped = styled
        .replace("\u{1b}[32m\u{2713} ", "")
        .replace("\u{1b}[31m\u{2717} ", "")
        .replace("\u{1b}[2m\u{2013} ", "")
        .replace("\u{1b}[0m", "");
    // Without the symbols the words are the same; only the padding after a
    // status differs, by the symbol's two columns.
    let squeeze = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(squeeze(&stripped), squeeze(&unstyled));
    Ok(())
}

/// Runs the unattended door and returns its exit status and what it wrote.
async fn unattended(
    steps: &ScriptedSteps,
    parts: CorePartSet,
    accept_plan: Option<&str>,
    mode: OutputMode,
) -> Result<(ProcessExit, String, String), Box<dyn std::error::Error>> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let cancellation = Cancellation::new();
    let exit = {
        let mut writer = OutputWriter::new(&mut stdout, &mut stderr);
        super::unattended::run(
            super::unattended::UnattendedRun {
                steps,
                parts,
                include: OptionalComponentSet::EMPTY,
                accept_plan: accept_plan.map(str::to_owned),
                version: VERSION,
                cancellation: &cancellation,
                mode,
                trap_interruptions: false,
            },
            &mut writer,
        )
        .await
    };
    Ok((exit, String::from_utf8(stdout)?, String::from_utf8(stderr)?))
}

const SPEECH: CorePartSet = CorePartSet::EMPTY.with(CorePart::Speech);

fn piece(component: ManagedComponent, stage: PieceStage) -> PieceProgress {
    PieceProgress {
        component,
        stage,
        received: 10,
        total: 10,
    }
}

/// ADR 0036 section 6: naming the parts installs the plan built for them,
/// with no question; one plain line per piece as it finishes, then the
/// table and `Result: pass`, exit 0.
#[tokio::test]
async fn unattended_installs_the_named_parts_and_passes() -> TestResult {
    let steps = ScriptedSteps::new(vec![
        video_ready_speech_missing(ManagedPlanAvailability::Qualified),
        all_ready(),
    ])
    .with_plan(speech_plan()?)
    .with_pieces(vec![
        piece(ManagedComponent::WhisperCli, PieceStage::Waiting),
        piece(ManagedComponent::WhisperCli, PieceStage::Ready),
        piece(ManagedComponent::WhisperModel, PieceStage::Ready),
    ])
    .with_install(Ok(install_results(
        json!([
            component("whisper_cli", "activated", None, None),
            component("whisper_model", "activated", None, None),
        ]),
        None,
    )?));

    let (exit, stdout, stderr) = unattended(&steps, SPEECH, None, OutputMode::Human).await?;

    check_snapshot("setup-door-unattended", &stdout)?;
    assert_plain(&stdout);
    assert_eq!(exit, ProcessExit::Success);
    assert_eq!(steps.planned_parts.get(), Some(SPEECH));
    assert_eq!(steps.installed.get(), 1);
    assert!(stdout.ends_with("Result: pass\n"), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
    Ok(())
}

/// With `--json` the result is the `setup` command's, and a speech part
/// that waits for video is a fail with its next step; the exit status is
/// the install's.
#[tokio::test]
async fn unattended_json_reports_a_fail_with_each_parts_next_step() -> TestResult {
    let nothing = checked(
        [
            DependencyState::Missing,
            DependencyState::Missing,
            DependencyState::Missing,
        ],
        NOTHING_YET,
        ManagedPlanAvailability::Qualified,
        OptionalCapability::NotInstalled,
    );
    let steps = ScriptedSteps::new(vec![nothing.clone(), nothing])
        .with_plan(plan(vec![
            action("ffmpeg_ffprobe", "n9.0.1", "BtbN", 112_640_000),
            action("whisper_cli", "whisper.cpp-v1.9.2", "ggml-org", 9_412_000),
        ])?)
        .with_install(Ok(install_results(
            json!([
                component(
                    "ffmpeg_ffprobe",
                    "failed",
                    Some("download"),
                    Some("offline")
                ),
                component("whisper_cli", "failed", None, Some("blocked")),
            ]),
            Some(FailureCode::DownloadFailed),
        )?));

    let (exit, stdout, _) = unattended(&steps, CorePartSet::ALL, None, OutputMode::Json).await?;

    let result: Value = serde_json::from_str(&stdout)?;
    assert_eq!(result["command"], "setup");
    assert_eq!(result["data"]["outcome"], "fail");
    assert_eq!(result["data"]["parts"][0]["status"], "not_set_up");
    assert_eq!(
        result["data"]["parts"][0]["next_step"],
        vsift_contract::SETUP_NEXT_CONNECTION
    );
    assert_eq!(
        result["data"]["parts"][1]["next_step"],
        vsift_contract::SETUP_NEXT_AFTER_VIDEO
    );
    assert_eq!(
        result["data"]["install"]["components"][0]["reason"],
        "offline"
    );
    assert_eq!(exit, ProcessExit::from(FailureCode::DownloadFailed.class()));
    Ok(())
}

/// Nothing to do is a pass, with no plan built and `install` null.
#[tokio::test]
async fn unattended_with_nothing_to_do_passes_without_a_plan() -> TestResult {
    let steps = ScriptedSteps::new(vec![all_ready()]);

    let (exit, stdout, _) = unattended(&steps, CorePartSet::ALL, None, OutputMode::Json).await?;

    let result: Value = serde_json::from_str(&stdout)?;
    assert_eq!(result["data"]["outcome"], "pass");
    assert_eq!(result["data"]["install"], Value::Null);
    assert_eq!(exit, ProcessExit::Success);
    assert_eq!((steps.planned.get(), steps.installed.get()), (0, 0));
    Ok(())
}

/// A named part `VSift` cannot install here, timed-out checks, and a plan
/// whose digest is not the one given are errors, refused before anything
/// is installed.
#[tokio::test]
async fn unattended_errors_refuse_before_anything_is_installed() -> TestResult {
    let unavailable = ScriptedSteps::new(vec![video_ready_speech_missing(
        ManagedPlanAvailability::TargetUnavailable,
    )]);
    let (exit, stdout, _) = unattended(&unavailable, SPEECH, None, OutputMode::Json).await?;
    let failure: Value = serde_json::from_str(&stdout)?;
    assert_eq!(failure["command"], "setup");
    assert_eq!(failure["error"]["code"], "MISSING_CAPABILITY");
    assert_eq!(
        exit,
        ProcessExit::from(FailureCode::MissingCapability.class())
    );
    assert_eq!(unavailable.planned.get(), 0);

    let mismatch = ScriptedSteps::new(vec![video_ready_speech_missing(
        ManagedPlanAvailability::Qualified,
    )])
    .with_plan(speech_plan()?);
    let (_, stdout, _) =
        unattended(&mismatch, SPEECH, Some(&"b".repeat(64)), OutputMode::Json).await?;
    let failure: Value = serde_json::from_str(&stdout)?;
    assert_eq!(failure["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(mismatch.installed.get(), 0);

    let matching = ScriptedSteps::new(vec![
        video_ready_speech_missing(ManagedPlanAvailability::Qualified),
        all_ready(),
    ])
    .with_plan(speech_plan()?)
    .with_install(Ok(install_results(json!([]), None)?));
    let (exit, _, _) =
        unattended(&matching, SPEECH, Some(&"a".repeat(64)), OutputMode::Json).await?;
    assert_eq!(exit, ProcessExit::Success);
    assert_eq!(matching.installed.get(), 1);
    Ok(())
}

/// The bars name each piece, fill with its bytes and end with its stage.
#[test]
fn each_piece_has_a_bar_its_megabytes_and_its_stage() {
    let lines = human::piece_bars(&[
        PieceProgress {
            component: ManagedComponent::MediaTools,
            stage: PieceStage::Downloading,
            received: 56_000_000,
            total: 112_000_000,
        },
        PieceProgress {
            component: ManagedComponent::WhisperModel,
            stage: PieceStage::WaitingForVideo,
            received: 60_000_000,
            total: 60_000_000,
        },
    ]);
    assert_eq!(
        lines,
        [
            format!(
                "  FFmpeg        {}{}    56 of 112 MB  downloading",
                "\u{2588}".repeat(10),
                "\u{2591}".repeat(10)
            ),
            format!(
                "  speech model  {}    60 of 60 MB  waiting for video",
                "\u{2588}".repeat(20)
            ),
        ]
    );
}
