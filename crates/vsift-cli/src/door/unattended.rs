//! The unattended setup door, `vsift setup --unattended --install …` (ADR
//! 0036 section 6): it runs once from its parameters, asks nothing, and
//! reports pass, fail or error.
//!
//! Naming the parts is the acceptance (the maintainer's decision of
//! 2026-10-27): the run installs the plan it builds for those parts, every
//! artifact of it pinned in the reviewed catalogue and verified before use.
//! `--accept-plan` keeps the stricter form: the plan must have exactly that
//! digest. Everything that would stop the run (a part `VSift` cannot install
//! here, checks that ran out of time, a different digest) is refused before
//! anything changes.

use std::io::Write;

use vsift::{
    Cancellation, CorePart, CorePartSet, FailureCode, ManagedPlanAvailability, OptionalCapability,
    OptionalComponent, OptionalComponentSet, RuntimeDependency,
};
use vsift_contract::{
    CommandName, OperationResponse, SETUP_NEXT_AFTER_VIDEO, SETUP_NEXT_AGAIN, SETUP_NEXT_CANCELLED,
    SETUP_NEXT_CONNECTION, SETUP_NEXT_SPACE, STALE_PLAN_REMEDIATION, SetupOutcome, SetupPartName,
    SetupPartResult, SetupPartStatus, SetupResponse, setup_next_install,
};

use super::{CheckedSetup, InstallResults, SetupSteps, Stop, extra_capability, show};
use crate::{
    CommandFailure, human,
    output::{OutputMode, OutputWriter, ProcessExit},
    piece_lines::{self, PieceDisplay},
    setup, signal,
};

/// The remediation of checks that ran out of time.
const CHECKS_TIMED_OUT: &str = "Some checks ran out of time, so nothing was installed. Run \
     setup check --timeout-seconds 30 to look again, then run this command again.";

/// The remediation of a named part `VSift` cannot install on this computer.
const CANNOT_INSTALL_HERE: &str = "VSift can't install this part on this computer yet, so \
     nothing was installed. Set it up yourself as the installation guide shows \
     (https://github.com/smormah/vsift/blob/main/docs/operations/install.md), then register \
     it with setup configure or setup configure-model.";

/// The remediation of an included extra `VSift` cannot add here.
const CANNOT_ADD_HERE: &str =
    "VSift can't add web videos on this computer yet, so nothing was installed.";

/// One unattended run's parameters.
pub(crate) struct UnattendedRun<'run, S> {
    pub(crate) steps: &'run S,
    /// The core parts named with `--install`.
    pub(crate) parts: CorePartSet,
    pub(crate) include: OptionalComponentSet,
    /// `--accept-plan`: the digest the plan must have.
    pub(crate) accept_plan: Option<String>,
    pub(crate) version: &'static str,
    pub(crate) cancellation: &'run Cancellation,
    /// Readable text or `--json`.
    pub(crate) mode: OutputMode,
    /// Whether Ctrl-C is trapped (the real process) during the install.
    pub(crate) trap_interruptions: bool,
}

/// Runs the unattended door and returns its exit status: 0 on a pass, the
/// install's failure status on a fail, the failure code's on an error.
pub(crate) async fn run<S, StandardOutput, StandardError>(
    run: UnattendedRun<'_, S>,
    writer: &mut OutputWriter<StandardOutput, StandardError>,
) -> ProcessExit
where
    S: SetupSteps,
    StandardOutput: Write,
    StandardError: Write,
{
    let mode = run.mode;
    match attempt(run, writer).await {
        Ok(exit) => exit,
        Err(Stop::Failed(failure)) => {
            crate::write_command_failure(writer, mode, CommandName::Setup, failure)
        }
        Err(Stop::Output) => ProcessExit::StorageOrIo,
    }
}

async fn attempt<S, StandardOutput, StandardError>(
    run: UnattendedRun<'_, S>,
    writer: &mut OutputWriter<StandardOutput, StandardError>,
) -> Result<ProcessExit, Stop>
where
    S: SetupSteps,
    StandardOutput: Write,
    StandardError: Write,
{
    let UnattendedRun {
        steps,
        parts,
        include,
        accept_plan,
        version,
        cancellation,
        mode,
        trap_interruptions,
    } = run;
    let checked = steps.check().await?;
    refuse_what_cannot_run(&checked, parts, include)?;
    let needs_core = parts.iter().any(|part| part_missing(&checked, part));
    let needs_extra = include
        .iter()
        .any(|extra| extra_capability(&checked, extra) == Some(&OptionalCapability::NotInstalled));

    let (installed, after) = if needs_core || needs_extra {
        let plan = steps.plan(parts, include).await?;
        if let Some(digest) = &accept_plan
            && plan.plan.plan_digest.as_deref() != Some(digest.as_str())
        {
            return Err(Stop::Failed(CommandFailure::with_remediation(
                FailureCode::InvalidArgument,
                STALE_PLAN_REMEDIATION.to_owned(),
            )));
        }
        let human = mode == OutputMode::Human;
        if human {
            show(
                writer,
                human::door_unattended_installing(version, parts, include),
            )?;
        }
        let _interrupts = trap_interruptions
            .then(|| signal::listen(cancellation.clone()).ok())
            .flatten();
        let installed = if human {
            piece_lines::with_piece_display(writer, PieceDisplay::Finished, |observer| {
                steps.install(&plan, cancellation, observer)
            })
            .await?
        } else {
            steps
                .install(&plan, cancellation, vsift::ProgressObserver::none())
                .await?
        };
        let after = steps.check().await?;
        (Some(installed), after)
    } else {
        (None, checked)
    };

    let response = response(&after, parts, include, installed.as_ref());
    let exit = match (response.outcome, installed.as_ref()) {
        (SetupOutcome::Pass, _) => ProcessExit::Success,
        (
            SetupOutcome::Fail,
            Some(InstallResults {
                failure: Some(code),
                ..
            }),
        ) => ProcessExit::from(code.class()),
        (SetupOutcome::Fail, _) => ProcessExit::from(FailureCode::MissingCapability.class()),
    };
    match mode {
        OutputMode::Human => {
            let heading = installed.is_none().then_some(version);
            show(
                writer,
                human::door_unattended_result(&after, &response, heading),
            )?;
        }
        OutputMode::Json | OutputMode::JsonLines => {
            let envelope = OperationResponse::complete(CommandName::Setup.identifier(), &response)
                .map_err(|_| Stop::Failed(CommandFailure::from(FailureCode::Internal)))?;
            writer.write_json(&envelope).map_err(|_| Stop::Output)?;
        }
    }
    Ok(exit)
}

/// Refuses, before anything changes, checks that ran out of time, a named
/// part `VSift` cannot install here and an included extra it cannot add.
fn refuse_what_cannot_run(
    checked: &CheckedSetup,
    parts: CorePartSet,
    include: OptionalComponentSet,
) -> Result<(), Stop> {
    if checked.standing().timed_out() {
        return Err(Stop::Failed(CommandFailure::with_remediation(
            FailureCode::DeadlineExceeded,
            CHECKS_TIMED_OUT.to_owned(),
        )));
    }
    if parts
        .iter()
        .any(|part| part_missing(checked, part) && !can_install(checked, part))
    {
        return Err(Stop::Failed(CommandFailure::with_remediation(
            FailureCode::MissingCapability,
            CANNOT_INSTALL_HERE.to_owned(),
        )));
    }
    if include
        .iter()
        .any(|extra| extra_capability(checked, extra) == Some(&OptionalCapability::Unavailable))
    {
        return Err(Stop::Failed(CommandFailure::with_remediation(
            FailureCode::MissingCapability,
            CANNOT_ADD_HERE.to_owned(),
        )));
    }
    Ok(())
}

fn part_missing(checked: &CheckedSetup, part: CorePart) -> bool {
    let standing = checked.standing();
    match part {
        CorePart::Video => standing.video_missing(),
        CorePart::Speech => standing.speech_missing(),
    }
}

/// Whether the catalogue can install `part` on this computer.
fn can_install(checked: &CheckedSetup, part: CorePart) -> bool {
    let dependency = match part {
        CorePart::Video => RuntimeDependency::Ffmpeg,
        CorePart::Speech => RuntimeDependency::Whisper,
    };
    checked.by_dependency[setup::dependency_slot(dependency)] == ManagedPlanAvailability::Qualified
}

const fn part_name(part: CorePart) -> SetupPartName {
    match part {
        CorePart::Video => SetupPartName::Video,
        CorePart::Speech => SetupPartName::Speech,
    }
}

/// The result: each core part and each included extra, and the outcome.
fn response(
    after: &CheckedSetup,
    parts: CorePartSet,
    include: OptionalComponentSet,
    installed: Option<&InstallResults>,
) -> SetupResponse {
    let mut results = Vec::new();
    let mut outcome = SetupOutcome::Pass;
    for part in CorePart::ALL {
        let requested = parts.contains(part);
        let ready = !part_missing(after, part);
        if requested && !ready {
            outcome = SetupOutcome::Fail;
        }
        results.push(SetupPartResult {
            part: part_name(part),
            requested,
            status: if ready {
                SetupPartStatus::Ready
            } else {
                SetupPartStatus::NotSetUp
            },
            next_step: if ready {
                None
            } else if requested {
                Some(next_after_install(installed, part).to_owned())
            } else {
                Some(setup_next_install(part_name(part)))
            },
        });
    }
    for extra in include.iter() {
        let OptionalComponent::OnlineLinks = extra;
        let status = match extra_capability(after, extra) {
            Some(OptionalCapability::Installed(_)) => SetupPartStatus::Added,
            Some(OptionalCapability::Unavailable) => SetupPartStatus::NotAvailableHere,
            Some(OptionalCapability::NotInstalled) | None => SetupPartStatus::NotAdded,
        };
        results.push(SetupPartResult {
            part: SetupPartName::WebVideos,
            requested: true,
            status,
            next_step: (status != SetupPartStatus::Added).then(|| SETUP_NEXT_AGAIN.to_owned()),
        });
    }
    SetupResponse {
        outcome,
        parts: results,
        install: installed.and_then(|installed| installed.data.clone()),
    }
}

/// The next step of a named part that does not work after the install,
/// from the first failure among its pieces.
fn next_after_install(installed: Option<&InstallResults>, part: CorePart) -> &'static str {
    let Some(installed) = installed else {
        return SETUP_NEXT_AGAIN;
    };
    let reason = installed
        .components
        .iter()
        .filter(|component| {
            vsift::ManagedComponent::from_identifier(&component.component).and_then(CorePart::of)
                == Some(part)
                && component.status == "failed"
        })
        .find_map(|component| component.reason.as_deref());
    match reason {
        Some("blocked") if part == CorePart::Speech => SETUP_NEXT_AFTER_VIDEO,
        Some("cancelled") => SETUP_NEXT_CANCELLED,
        Some("offline" | "tls" | "proxy_auth" | "redirect_policy" | "http_status") => {
            SETUP_NEXT_CONNECTION
        }
        Some("storage") => SETUP_NEXT_SPACE,
        _ => SETUP_NEXT_AGAIN,
    }
}
