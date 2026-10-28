//! The setup door, plain `vsift setup` (ADR 0036): for a person at a
//! terminal, it checks, shows where they stand, offers to install what is
//! missing and installs only on their yes.
//!
//! The door is a front over the commands assistants use, never beside them:
//! it checks with `setup check`'s engine call and renderer, builds the plan
//! `setup plan` builds, and installs through the transaction of `setup
//! install`, accepting the plan it showed by that plan's own digest. Two
//! ports keep the conversation testable without a terminal or a network:
//!
//! - [`Console`]: whether a person is there, whether their terminal takes
//!   colour, and their answer to the one question.
//! - [`SetupSteps`]: check, plan and install. Production runs them on the
//!   engine ([`EngineSteps`]); tests script them.
//!
//! What the door never does: install without a typed yes, ask for
//! elevation, search folders, or ask anything when nobody is there. Every
//! other command keeps never prompting.

use std::{
    io::{self, BufRead, IsTerminal, Read, Write},
    time::Duration,
};

use vsift::{
    Cancellation, CorePartSet, DEFAULT_LOCAL_ASR_CHECK_BUDGET, Engine, ExecutableSelections,
    FailureCode, LocalAsrSetupStatus, ManagedPlanAvailability, ManagedUpdate, OptionalCapability,
    OptionalComponent, OptionalComponentSet, ProgressObserver, RuntimeDependency, RuntimeDiagnosis,
    SetupCheckRequest, SetupInstallRequest,
};
use vsift_contract::{CommandName, DependencyLookup, OperationResponse, SavedSetupPlan};

use crate::{
    CommandFailure,
    command::{DoorExtra, ExecutionProfile},
    config::{ConfigLayer, EffectiveConfig, HostPolicy},
    human::{self, InstallComponent, SetupInstall, SetupPlan, Standing, Styling},
    live_line::{self, LiveLine},
    output::{OutputMode, OutputWriter, ProcessExit, setup_exit},
    piece_lines::{self, PieceDisplay},
    setup::{self, ManagedSetupFacts},
    signal,
};

/// How long the door waits for an answer before taking it as no.
const ANSWER_WAIT: Duration = Duration::from_secs(600);

/// Most bytes of an answer read: `yes` and a line break fit many times over.
const MAX_ANSWER_BYTES: u64 = 256;

/// The person's answer to the one question.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Answer {
    /// `y` or `yes`, in any case, with spaces around it or not.
    Yes,
    /// Anything else: Enter, another word, a closed input, Ctrl-C, or no
    /// answer in time.
    No,
}

/// Whether the question's line was ended by the person's Enter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuestionLine {
    /// A typed answer: the terminal moved to the next line with the Enter,
    /// so what follows starts there.
    Ended,
    /// Nothing was typed (Ctrl-C, a closed input, no answer in time): the
    /// cursor still sits after the question, so what follows breaks the
    /// line first.
    Open,
}

/// What the person did at the question: the answer, and where it left the
/// cursor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Reply {
    pub(crate) answer: Answer,
    pub(crate) line: QuestionLine,
}

impl Reply {
    /// No typed line: a no that leaves the cursor after the question.
    pub(crate) const UNANSWERED: Self = Self {
        answer: Answer::No,
        line: QuestionLine::Open,
    };

    /// A line the person typed and ended with Enter.
    pub(crate) fn typed(line: &str) -> Self {
        Self {
            answer: Answer::of(line),
            line: QuestionLine::Ended,
        }
    }
}

impl Answer {
    /// Reads a typed line: only `y` and `yes` are yes.
    pub(crate) fn of(line: &str) -> Self {
        let word = line.trim();
        if word.eq_ignore_ascii_case("y") || word.eq_ignore_ascii_case("yes") {
            Self::Yes
        } else {
            Self::No
        }
    }
}

/// The person at the terminal, as the door sees them.
pub(crate) trait Console {
    /// Standard input and standard output are both terminals.
    fn person_present(&self) -> bool;

    /// Whether the terminal takes symbols and colour (ADR 0036 section 8).
    fn styling(&self) -> Styling;

    /// Whether the install's live line may be drawn on standard error.
    fn live_line(&self) -> LiveLine;

    /// Waits for one answer; a cancellation (Ctrl-C) is a no.
    async fn answer(&mut self, cancellation: &Cancellation) -> Reply;
}

/// The real terminal of the process.
pub(crate) struct SystemConsole;

impl Console for SystemConsole {
    fn person_present(&self) -> bool {
        io::stdin().is_terminal() && io::stdout().is_terminal()
    }

    fn styling(&self) -> Styling {
        // `NO_COLOR` set to anything, or a dumb terminal, keeps the words
        // plain; a Windows console takes colour only once it accepts
        // virtual terminal sequences.
        let colour = io::stdout().is_terminal()
            && !anstyle_query::no_color()
            && anstyle_query::term_supports_color()
            && windows_console_takes_colour();
        if colour {
            Styling::Rich
        } else {
            Styling::Plain
        }
    }

    fn live_line(&self) -> LiveLine {
        if io::stderr().is_terminal() {
            LiveLine::Shown
        } else {
            LiveLine::Hidden
        }
    }

    async fn answer(&mut self, cancellation: &Cancellation) -> Reply {
        // Reading a terminal blocks, so one thread reads the line and the
        // door waits for it, the cancellation or the time limit, whichever
        // comes first. A thread still waiting when the door moves on ends
        // with the process.
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let reader = std::thread::Builder::new()
            .name("vsift-setup-answer".to_owned())
            .spawn(move || {
                let mut line = String::new();
                let read = io::stdin()
                    .lock()
                    .take(MAX_ANSWER_BYTES)
                    .read_line(&mut line);
                let _ignored = sender.send(read.map(|_| line));
            });
        if reader.is_err() {
            return Reply::UNANSWERED;
        }
        tokio::select! {
            read = receiver => match read {
                // A line ended by Enter, or one so long the read stopped
                // before its end; a closed input (Ctrl-D, even after a few
                // characters) never moved the cursor off the question.
                Ok(Ok(line)) if line.ends_with('\n') || line.len() as u64 == MAX_ANSWER_BYTES => {
                    Reply::typed(&line)
                }
                _ => Reply::UNANSWERED,
            },
            () = cancellation.cancelled() => Reply::UNANSWERED,
            () = tokio::time::sleep(ANSWER_WAIT) => Reply::UNANSWERED,
        }
    }
}

/// A console nobody sits at: what in-process tests of the whole command
/// line get, so they never read the test runner's terminal.
#[cfg(test)]
pub(crate) struct Nobody;

#[cfg(test)]
impl Console for Nobody {
    fn person_present(&self) -> bool {
        false
    }

    fn styling(&self) -> Styling {
        Styling::Plain
    }

    fn live_line(&self) -> LiveLine {
        LiveLine::Hidden
    }

    fn answer(&mut self, _cancellation: &Cancellation) -> impl Future<Output = Reply> {
        std::future::ready(Reply::UNANSWERED)
    }
}

#[cfg(windows)]
fn windows_console_takes_colour() -> bool {
    anstyle_query::windows::enable_ansi_colors().unwrap_or(false)
}

#[cfg(not(windows))]
const fn windows_console_takes_colour() -> bool {
    true
}

/// What one check found, owned, so the door can show it after the engine
/// call has returned.
#[derive(Clone, Debug)]
pub(crate) struct CheckedSetup {
    pub(crate) diagnosis: RuntimeDiagnosis,
    pub(crate) local_asr: LocalAsrSetupStatus,
    pub(crate) availability: ManagedPlanAvailability,
    pub(crate) by_dependency: [ManagedPlanAvailability; 3],
    pub(crate) updates: Vec<ManagedUpdate>,
    pub(crate) optional: Vec<(OptionalComponent, OptionalCapability)>,
    /// Where each probed executable came from, in `RuntimeDependency::ALL`
    /// order.
    pub(crate) lookups: [DependencyLookup; 3],
}

impl CheckedSetup {
    /// The facts the shared renderer reads.
    pub(crate) fn managed(&self) -> ManagedSetupFacts<'_> {
        ManagedSetupFacts {
            availability: self.availability,
            by_dependency: self.by_dependency,
            updates: &self.updates,
            optional: &self.optional,
        }
    }

    /// Where the executable probed for `dependency` came from.
    pub(crate) const fn lookup(&self, dependency: RuntimeDependency) -> DependencyLookup {
        self.lookups[setup::dependency_slot(dependency)]
    }

    /// Video and speech as a person meets them.
    pub(crate) fn standing(&self) -> Standing<'_> {
        Standing::of(&self.diagnosis, self.local_asr)
    }

    /// Whether this computer has a reviewed catalogue for anything.
    fn can_install_anything(&self) -> bool {
        self.availability == ManagedPlanAvailability::Qualified
            || self
                .by_dependency
                .contains(&ManagedPlanAvailability::Qualified)
    }
}

/// The plan the door offers: its `setup plan --json` result, exactly as a
/// person would save it, its published data, and the core parts it was
/// built for.
#[derive(Debug)]
pub(crate) struct ReviewedPlan {
    /// The whole `setup.plan` result, accepted as the saved plan.
    pub(crate) envelope: serde_json::Value,
    /// Its `data`, read as the readable views read it.
    pub(crate) plan: SetupPlan,
    /// The core parts the plan may install; the engine rebuilds it for
    /// these to accept it.
    pub(crate) parts: CorePartSet,
}

impl ReviewedPlan {
    /// Reads an evaluated plan's published result.
    ///
    /// # Errors
    ///
    /// `INTERNAL` when the result does not read back as a plan.
    pub(crate) fn from_envelope(
        envelope: serde_json::Value,
        parts: CorePartSet,
    ) -> Result<Self, CommandFailure> {
        let plan = envelope
            .get("data")
            .cloned()
            .ok_or_else(|| CommandFailure::from(FailureCode::Internal))
            .and_then(|data| {
                serde_json::from_value(data)
                    .map_err(|_| CommandFailure::from(FailureCode::Internal))
            })?;
        Ok(Self {
            envelope,
            plan,
            parts,
        })
    }
}

/// What one install did to each piece, in plan order, the failure code the
/// transaction ended with, when it did not install everything core, and its
/// `setup.install` data, which the unattended result carries.
#[derive(Debug, Default)]
pub(crate) struct InstallResults {
    pub(crate) components: Vec<InstallComponent>,
    pub(crate) failure: Option<FailureCode>,
    pub(crate) data: Option<serde_json::Value>,
}

impl InstallResults {
    /// Reads a `setup.install` result's data.
    fn from_data(
        data: serde_json::Value,
        failure: Option<FailureCode>,
    ) -> Result<Self, CommandFailure> {
        let install: SetupInstall = serde_json::from_value(data.clone())
            .map_err(|_| CommandFailure::from(FailureCode::Internal))?;
        let mut components = install.components;
        components.extend(install.optional_components);
        Ok(Self {
            components,
            failure,
            data: Some(data),
        })
    }
}

/// The three steps of the door, as the commands assistants use run them.
pub(crate) trait SetupSteps {
    /// `setup check` with the desktop profile and the default deadline.
    async fn check(&self) -> Result<CheckedSetup, CommandFailure>;

    /// `setup plan --profile desktop` for the core `parts`, with the
    /// included extras.
    async fn plan(
        &self,
        parts: CorePartSet,
        include: OptionalComponentSet,
    ) -> Result<ReviewedPlan, CommandFailure>;

    /// `setup install`, accepting `plan` by its own digest. A component
    /// that fails is a result; a refusal before anything is installed (a
    /// busy install, a plan the machine no longer gives) is an error.
    async fn install(
        &self,
        plan: &ReviewedPlan,
        cancellation: &Cancellation,
        progress: ProgressObserver,
    ) -> Result<InstallResults, CommandFailure>;
}

/// The steps, run on the engine.
pub(crate) struct EngineSteps<'engine> {
    pub(crate) engine: &'engine Engine,
}

impl EngineSteps<'_> {
    /// The door's configuration: the desktop profile and the default probe
    /// deadline, which a person never has to choose.
    fn config() -> Result<EffectiveConfig, CommandFailure> {
        EffectiveConfig::resolve(
            ConfigLayer {
                profile: Some(ExecutionProfile::Desktop),
                probe_timeout_seconds: None,
            },
            ConfigLayer::default(),
            ConfigLayer::default(),
            &HostPolicy::local_r0(),
        )
        .map_err(|_| CommandFailure::from(FailureCode::InvalidArgument))
    }
}

impl SetupSteps for EngineSteps<'_> {
    async fn check(&self) -> Result<CheckedSetup, CommandFailure> {
        let config = Self::config()?;
        let report = self
            .engine
            .check_setup(SetupCheckRequest {
                probe_timeout: config.probe_timeout,
                per_call: ExecutableSelections {
                    ffmpeg: None,
                    ffprobe: None,
                    whisper: None,
                },
                local_asr_budget: DEFAULT_LOCAL_ASR_CHECK_BUDGET,
            })
            .await
            .map_err(CommandFailure::from)?;
        Ok(CheckedSetup {
            diagnosis: report.diagnosis().clone(),
            local_asr: *report.local_asr(),
            availability: report.managed_install(),
            by_dependency: RuntimeDependency::ALL
                .map(|dependency| report.managed_install_for(dependency)),
            updates: report.managed_updates().to_vec(),
            optional: report.optional_components().to_vec(),
            lookups: RuntimeDependency::ALL.map(|dependency| report.lookup(dependency)),
        })
    }

    async fn plan(
        &self,
        parts: CorePartSet,
        include: OptionalComponentSet,
    ) -> Result<ReviewedPlan, CommandFailure> {
        let plan =
            crate::current_setup_plan(self.engine, ExecutionProfile::Desktop, parts, include)
                .await?;
        let envelope =
            OperationResponse::complete(CommandName::SetupPlan.identifier(), plan.presentation())
                .map_err(|_| CommandFailure::from(FailureCode::Internal))
                .and_then(|response| {
                    serde_json::to_value(&response)
                        .map_err(|_| CommandFailure::from(FailureCode::Internal))
                })?;
        ReviewedPlan::from_envelope(envelope, parts)
    }

    async fn install(
        &self,
        plan: &ReviewedPlan,
        cancellation: &Cancellation,
        progress: ProgressObserver,
    ) -> Result<InstallResults, CommandFailure> {
        // The plan goes to the engine exactly as `setup install --plan`
        // reads a saved one: through the strict saved-plan reader, then the
        // engine's own acceptance, which rebuilds the plan and requires it
        // and the digest to match.
        let saved: SavedSetupPlan = serde_json::from_value(plan.envelope.clone())
            .map_err(|_| CommandFailure::from(FailureCode::Internal))?;
        saved.validate_envelope().map_err(CommandFailure::from)?;
        let digest = plan
            .plan
            .plan_digest
            .clone()
            .ok_or_else(|| CommandFailure::from(FailureCode::InvalidArgument))?;
        let config = Self::config()?;
        match setup::install_saved(
            self.engine,
            SetupInstallRequest {
                saved_plan: saved,
                accepted_digest: digest,
                artifact_directory: None,
                probe_timeout: config.probe_timeout,
                parts: plan.parts,
            },
            cancellation,
            progress,
        )
        .await
        {
            Ok(response) => {
                let data = serde_json::to_value(&response)
                    .ok()
                    .and_then(|mut value| value.get_mut("data").map(serde_json::Value::take))
                    .ok_or_else(|| CommandFailure::from(FailureCode::Internal))?;
                InstallResults::from_data(data, None)
            }
            Err(failure) => match failure.data {
                Some(data) => InstallResults::from_data(*data, Some(failure.code)),
                None => Err(failure),
            },
        }
    }
}

/// The door's parsed options.
#[derive(Clone, Debug, Default)]
pub(crate) struct DoorOptions {
    /// The extras to add, by their person-facing names.
    pub(crate) include: Vec<DoorExtra>,
}

impl DoorOptions {
    /// The included extras as the plan request takes them.
    pub(crate) fn include_set(&self) -> OptionalComponentSet {
        self.include
            .iter()
            .fold(OptionalComponentSet::EMPTY, |set, extra| match extra {
                DoorExtra::WebVideos => set.with(OptionalComponent::OnlineLinks),
            })
    }
}

/// Everything the door needs from the process around it.
pub(crate) struct DoorRun<'run, C, S> {
    pub(crate) console: &'run mut C,
    pub(crate) steps: &'run S,
    pub(crate) options: DoorOptions,
    /// The version in the heading, `VSift 0.3.0 setup`.
    pub(crate) version: &'static str,
    pub(crate) cancellation: &'run Cancellation,
    /// Whether Ctrl-C is trapped (the real process) once the question is
    /// asked; in-process tests leave the default.
    pub(crate) trap_interruptions: bool,
}

/// Runs the door and returns its exit status: the status `setup check`
/// gives for where the setup ends, or that of an install failure.
pub(crate) async fn run<C, S, StandardOutput, StandardError>(
    door: DoorRun<'_, C, S>,
    writer: &mut OutputWriter<StandardOutput, StandardError>,
) -> ProcessExit
where
    C: Console,
    S: SetupSteps,
    StandardOutput: Write,
    StandardError: Write,
{
    match converse(door, writer).await {
        Ok(exit) => exit,
        Err(Stop::Failed(failure)) => {
            crate::write_command_failure(writer, OutputMode::Human, CommandName::Setup, failure)
        }
        Err(Stop::Output) => ProcessExit::StorageOrIo,
    }
}

/// Why the conversation stopped early.
enum Stop {
    /// A step failed before anything was installed.
    Failed(CommandFailure),
    /// Standard output could not be written.
    Output,
}

impl From<CommandFailure> for Stop {
    fn from(failure: CommandFailure) -> Self {
        Self::Failed(failure)
    }
}

/// Writes one rendered piece of the conversation.
fn show<StandardOutput, StandardError>(
    writer: &mut OutputWriter<StandardOutput, StandardError>,
    text: Result<human::RenderedText, human::TooLarge>,
) -> Result<(), Stop>
where
    StandardOutput: Write,
    StandardError: Write,
{
    let text = text.map_err(|_| Stop::Output)?;
    writer
        .write_rendered_stdout(&text)
        .and_then(|()| writer.flush_stdout())
        .map_err(|_| Stop::Output)
}

async fn converse<C, S, StandardOutput, StandardError>(
    door: DoorRun<'_, C, S>,
    writer: &mut OutputWriter<StandardOutput, StandardError>,
) -> Result<ProcessExit, Stop>
where
    C: Console,
    S: SetupSteps,
    StandardOutput: Write,
    StandardError: Write,
{
    let DoorRun {
        console,
        steps,
        options,
        version,
        cancellation,
        trap_interruptions,
    } = door;
    let checked = steps.check().await?;
    let check_exit = setup_exit(checked.diagnosis.readiness);
    if !console.person_present() {
        show(writer, human::door_no_terminal(&checked, version))?;
        return Ok(check_exit);
    }
    let styling = console.styling();
    show(writer, human::door_opening(&checked, version, styling))?;

    let standing = checked.standing();
    let include = options.include_set();
    let wants_core = standing.video_missing() || standing.speech_missing();
    let wants_extras = include
        .iter()
        .any(|extra| extra_capability(&checked, extra) == Some(&OptionalCapability::NotInstalled));
    if standing.timed_out() || !(wants_core || wants_extras) || !checked.can_install_anything() {
        show(writer, human::door_nothing_to_offer(&checked, include))?;
        return Ok(check_exit);
    }

    let plan = steps.plan(CorePartSet::ALL, include).await?;
    let offer = human::DoorOffer::of(&plan.plan, standing);
    if offer.is_empty() {
        show(writer, human::door_nothing_to_offer(&checked, include))?;
        return Ok(check_exit);
    }
    show(writer, human::door_offer(&offer, include, &checked))?;
    show(writer, human::door_question(&offer))?;

    // From the question on, Ctrl-C is an answer (no) and, during the
    // install, a cancellation; before it, it ends the process as for any
    // short command.
    let _interrupts = trap_interruptions
        .then(|| signal::listen(cancellation.clone()).ok())
        .flatten();
    let reply = console.answer(cancellation).await;
    if reply.answer == Answer::No || cancellation.is_cancelled() {
        show(writer, human::door_declined(reply.line))?;
        return Ok(check_exit);
    }

    show(writer, human::door_installing(&offer, reply.line))?;
    // Bars need a terminal that takes symbols and colour (ADR 0036 section
    // 8); without one, the count of pieces done is the live line on
    // standard error, as in the first release of the door.
    let installed = if styling == Styling::Rich {
        piece_lines::with_piece_display(writer, PieceDisplay::Bars, |observer| {
            steps.install(&plan, cancellation, observer)
        })
        .await?
    } else {
        live_line::with_live_line_saying(
            writer,
            console.live_line(),
            live_line::install_line,
            |observer| steps.install(&plan, cancellation, observer),
        )
        .await?
    };
    show(writer, human::door_install_results(&installed, styling))?;

    let after = steps.check().await?;
    show(
        writer,
        human::door_after_install(&after, &installed, include, styling),
    )?;
    Ok(installed.failure.map_or_else(
        || setup_exit(after.diagnosis.readiness),
        |code| ProcessExit::from(code.class()),
    ))
}

/// Where an extra stands on this computer, when the check reports it.
pub(crate) fn extra_capability(
    checked: &CheckedSetup,
    extra: OptionalComponent,
) -> Option<&OptionalCapability> {
    checked
        .optional
        .iter()
        .find(|(component, _)| *component == extra)
        .map(|(_, capability)| capability)
}

pub(crate) mod unattended;

#[cfg(test)]
mod tests;
