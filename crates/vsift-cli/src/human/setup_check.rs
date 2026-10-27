//! `setup check`'s readable view: for a person opening `VSift` for the first
//! time, a table of what works, one plain sentence on where they stand and
//! the one next step (the first-run design note, parts C3 and C4).
//!
//! The view is read-only and written for a person. Wording meant for an AI
//! assistant (who to ask, how a path was found, which profile ran) stays in
//! the JSON remediation, which is unchanged. Two rules shape it:
//!
//! - **"Ready" means the work can be done.** Video is ready when `FFmpeg` and
//!   `FFprobe` respond. Speech is ready only when whisper.cpp, a model it can
//!   run and the local-ASR check all pass, so a tool that responds without a
//!   usable model is "model missing", never "ready". The setup is complete
//!   only when video and speech are both ready.
//! - **The last line is the one next step.** For a person that is the setup
//!   door, `vsift setup` (ADR 0036), whenever the setup is not complete; a
//!   worker host's operator gets the concrete command ([`Next`]), which is
//!   also the door's own last line when it has nothing to offer.
//!
//! The table, the verdict and the details are one renderer,
//! [`push_overview`], shared with the door, so the two cannot drift. Here
//! the words use ASCII only; the door adds symbols and colour when a
//! terminal takes them.

use vsift::{
    DependencyState, LocalAsrCheckOutcome, LocalAsrModelStatus, LocalAsrNotRunReason,
    LocalAsrSetupStatus, ManagedPlanAvailability, ManagedUpdate, OptionalCapability,
    OptionalComponent, ProbeTimeoutKind, RuntimeDependency, RuntimeDiagnosis, SupersessionReason,
};
use vsift_contract::{
    DependencyLookup, ProbeTimeHeadroom, managed_update_next_step, probe_deadline_text,
};

use super::text::{RenderedText, Styling, TerminalText, Tone, TooLarge};
use crate::{command::ExecutionProfile, setup::ManagedSetupFacts};

/// Where a person finds the tools to install themselves.
const INSTALL_GUIDE: &str = "https://github.com/smormah/vsift/blob/main/docs/operations/install.md";

/// Where a person looks when a check keeps failing.
const TROUBLESHOOTING_GUIDE: &str =
    "https://github.com/smormah/vsift/blob/main/docs/guide/troubleshooting.md";

/// A dependency that was never probed counts as not found.
static NOT_PROBED: DependencyState = DependencyState::Missing;

/// How video stands: `FFmpeg` and `FFprobe` together.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VideoState {
    Ready,
    Missing,
    NotWorking,
    NoAnswer,
}

impl VideoState {
    fn of(ffmpeg: &DependencyState, ffprobe: &DependencyState) -> Self {
        let tools = [ffmpeg, ffprobe];
        if tools
            .iter()
            .any(|state| matches!(state, DependencyState::Missing))
        {
            Self::Missing
        } else if tools
            .iter()
            .any(|state| matches!(state, DependencyState::Unhealthy { .. }))
        {
            Self::NotWorking
        } else if tools
            .iter()
            .any(|state| matches!(state, DependencyState::TimedOut(_)))
        {
            Self::NoAnswer
        } else {
            Self::Ready
        }
    }

    const fn tone(self) -> Tone {
        match self {
            Self::Ready => Tone::Good,
            Self::Missing | Self::NotWorking | Self::NoAnswer => Tone::Bad,
        }
    }

    const fn status(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Missing => "not set up",
            Self::NotWorking => "not working",
            Self::NoAnswer => "no answer",
        }
    }
}

/// How speech stands: whisper.cpp, its model and the local-ASR check
/// together.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpeechState {
    Ready,
    /// whisper.cpp was not found; `with_model` when no model is registered
    /// either.
    Missing {
        with_model: bool,
    },
    NotWorking,
    NoAnswer,
    ModelMissing,
    ModelUnreadable,
    ModelUnknown,
    TestFailed,
    WaitingForVideo,
}

impl SpeechState {
    fn of(whisper: &DependencyState, local_asr: LocalAsrSetupStatus) -> Self {
        match whisper {
            DependencyState::TimedOut(_) => Self::NoAnswer,
            DependencyState::Missing => Self::Missing {
                with_model: local_asr.model == LocalAsrModelStatus::NotSelected,
            },
            DependencyState::Unhealthy { .. } => Self::NotWorking,
            DependencyState::Available { .. } => match local_asr.model {
                LocalAsrModelStatus::NotSelected => Self::ModelMissing,
                LocalAsrModelStatus::Unreadable => Self::ModelUnreadable,
                LocalAsrModelStatus::Unrecognised => Self::ModelUnknown,
                LocalAsrModelStatus::KnownPinned(_) => match local_asr.verification {
                    LocalAsrCheckOutcome::Verified(_) => Self::Ready,
                    LocalAsrCheckOutcome::Failed(_) => Self::TestFailed,
                    LocalAsrCheckOutcome::NotRun(reason) => match reason {
                        LocalAsrNotRunReason::MediaToolsUnavailable => Self::WaitingForVideo,
                        LocalAsrNotRunReason::WhisperUnavailable => Self::NotWorking,
                        LocalAsrNotRunReason::ModelNotSelected => Self::ModelMissing,
                        LocalAsrNotRunReason::ModelNotPinned => Self::ModelUnknown,
                    },
                },
            },
        }
    }

    const fn tone(self) -> Tone {
        match self {
            Self::Ready => Tone::Good,
            Self::WaitingForVideo => Tone::Quiet,
            _ => Tone::Bad,
        }
    }

    const fn status(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Missing { .. } => "not set up",
            Self::NotWorking => "not working",
            Self::NoAnswer => "no answer",
            Self::ModelMissing => "model missing",
            Self::ModelUnreadable => "model unreadable",
            Self::ModelUnknown => "unknown model",
            Self::TestFailed => "test failed",
            Self::WaitingForVideo => "waiting for video",
        }
    }
}

/// The one concrete next step: the door's last line when it has nothing to
/// offer, and a worker host's in `setup check`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Next {
    /// Everything works.
    TryAVideo,
    /// The checks ran out of time: run them again.
    CheckAgain,
    /// `VSift` can install the missing part from its reviewed catalogue.
    ReviewThePlan,
    /// Register a tool the person installs themselves.
    RegisterTool(RuntimeDependency),
    /// Register a speech model the person downloads themselves.
    RegisterModel,
    /// Speech failed its test; nothing more is known than to look again.
    LookAgain,
}

impl Next {
    pub(super) fn of(standing: Standing<'_>, managed: ManagedSetupFacts<'_>) -> Self {
        if standing.complete() {
            return Self::TryAVideo;
        }
        if standing.timed_out() {
            return Self::CheckAgain;
        }
        let needs_a_tool = RuntimeDependency::ALL
            .into_iter()
            .zip(standing.states)
            .find(|(_, state)| {
                matches!(
                    state,
                    DependencyState::Missing | DependencyState::Unhealthy { .. }
                )
            });
        if let Some((dependency, _)) = needs_a_tool {
            return if managed.by_dependency[crate::setup::dependency_slot(dependency)]
                == ManagedPlanAvailability::Qualified
            {
                Self::ReviewThePlan
            } else {
                Self::RegisterTool(dependency)
            };
        }
        match standing.speech {
            SpeechState::ModelMissing
                if managed.availability == ManagedPlanAvailability::Qualified =>
            {
                Self::ReviewThePlan
            }
            SpeechState::ModelMissing
            | SpeechState::ModelUnreadable
            | SpeechState::ModelUnknown => Self::RegisterModel,
            SpeechState::TestFailed => Self::LookAgain,
            _ => Self::CheckAgain,
        }
    }
}

/// Where a setup stands: each tool's state, and video and speech as a
/// person meets them. Worked out once, for `setup check` and the door.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Standing<'report> {
    states: [&'report DependencyState; 3],
    video: VideoState,
    speech: SpeechState,
}

impl<'report> Standing<'report> {
    /// The standing a check reports.
    pub(crate) fn of(diagnosis: &'report RuntimeDiagnosis, local_asr: LocalAsrSetupStatus) -> Self {
        let states = RuntimeDependency::ALL.map(|dependency| {
            diagnosis
                .dependencies
                .iter()
                .find(|status| status.dependency == dependency)
                .map_or(&NOT_PROBED, |status| &status.state)
        });
        let [ffmpeg, ffprobe, whisper] = states;
        Self {
            states,
            video: VideoState::of(ffmpeg, ffprobe),
            speech: SpeechState::of(whisper, local_asr),
        }
    }

    /// Video and speech both work.
    pub(crate) fn complete(&self) -> bool {
        self.video == VideoState::Ready && self.speech == SpeechState::Ready
    }

    /// Video needs something done.
    pub(crate) fn video_missing(&self) -> bool {
        self.video != VideoState::Ready
    }

    /// Speech needs something done of its own: speech that is only waiting
    /// for video is not missing.
    pub(crate) fn speech_missing(&self) -> bool {
        !matches!(
            self.speech,
            SpeechState::Ready | SpeechState::WaitingForVideo
        )
    }

    /// A tool check ran out of time, so what is missing is not known.
    pub(crate) fn timed_out(&self) -> bool {
        self.states
            .iter()
            .any(|state| matches!(state, DependencyState::TimedOut(_)))
    }
}

/// `setup check`, rendered for a person from its typed report.
///
/// Nothing in the view is provider text, so no untrusted value reaches the
/// terminal; the managed versions of an update are catalogue keys.
/// `lookup` is used for one sentence only: a path the person gave that was
/// not found.
pub(crate) fn setup_check<L>(
    diagnosis: &RuntimeDiagnosis,
    local_asr: LocalAsrSetupStatus,
    managed: ManagedSetupFacts<'_>,
    profile: ExecutionProfile,
    lookup: &L,
) -> Result<RenderedText, TooLarge>
where
    L: Fn(RuntimeDependency) -> DependencyLookup,
{
    let standing = Standing::of(diagnosis, local_asr);
    let mut text = TerminalText::result();
    text.push_fixed("VSift setup check").end_line().blank_line();
    push_overview(&mut text, standing, managed, Styling::Plain, lookup);
    if standing.complete() {
        push_extras_line(&mut text, managed.optional, profile);
    }
    let next = Next::of(standing, managed);
    push_guide(&mut text, next);
    // A person's way on is the setup door (ADR 0036); a worker host is set
    // up by its operator, who gets the concrete step.
    if profile == ExecutionProfile::Desktop && !standing.complete() && next != Next::CheckAgain {
        text.push_fixed("Next: vsift setup").end_line();
    } else {
        push_next(&mut text, next, Door::Absent);
    }
    text.finish()
}

/// The table, any updates, the line on where the person stands, what
/// happened to a tool that did not simply go missing and, when only speech
/// is missing, that video still works: everything `setup check` and the
/// door share.
pub(super) fn push_overview<L>(
    text: &mut TerminalText,
    standing: Standing<'_>,
    managed: ManagedSetupFacts<'_>,
    styling: Styling,
    lookup: &L,
) where
    L: Fn(RuntimeDependency) -> DependencyLookup,
{
    push_table(
        text,
        standing.video,
        standing.speech,
        managed.optional,
        styling,
    );
    text.blank_line();
    for update in managed.updates {
        push_update(text, update);
    }
    text.blank_line();
    push_verdict(text, standing.video, standing.speech);
    push_details(text, standing.states, standing.speech, lookup);
    if !standing.complete() && standing.video == VideoState::Ready {
        text.push_fixed("Video works, so a transcript you already have can still be used.")
            .end_line();
    }
}

/// One row of the table. Every cell is `VSift`'s own fixed word.
struct Row {
    part: &'static str,
    status: (&'static str, Tone),
    uses: &'static str,
    purpose: &'static str,
}

/// The table: video, speech, then each optional extra, always shown. With
/// [`Styling::Rich`] each status has its symbol and colour; the words are
/// the same either way, so the plain table is the styled one without them.
fn push_table(
    text: &mut TerminalText,
    video: VideoState,
    speech: SpeechState,
    optional: &[(OptionalComponent, OptionalCapability)],
    styling: Styling,
) {
    let mut rows = vec![
        Row {
            part: "Video",
            status: (video.status(), video.tone()),
            uses: "FFmpeg",
            purpose: "reading videos, taking frames",
        },
        Row {
            part: "Speech",
            status: (speech.status(), speech.tone()),
            uses: "whisper.cpp",
            purpose: "turning speech into text",
        },
    ];
    for (component, capability) in optional {
        let OptionalComponent::OnlineLinks = component;
        rows.push(Row {
            part: "Web videos",
            status: match capability {
                OptionalCapability::Installed(_) => ("added", Tone::Good),
                OptionalCapability::NotInstalled => ("none added", Tone::Quiet),
                OptionalCapability::Unavailable => ("not available", Tone::Quiet),
            },
            uses: "yt-dlp",
            purpose: "web page videos (optional)",
        });
    }
    // A styled status is two columns wider: its symbol and a space.
    let symbol_columns = match styling {
        Styling::Plain => 0,
        Styling::Rich => 2,
    };
    let width = |cell: fn(&Row) -> &'static str| {
        rows.iter()
            .map(|row| cell(row).chars().count())
            .max()
            .unwrap_or(0)
    };
    let part_width = width(|row| row.part).max("Part".len());
    let status_width = (width(|row| row.status.0) + symbol_columns).max("Status".len());
    let uses_width = width(|row| row.uses).max("Uses".len());
    let pad = |text: &mut TerminalText, used: usize, width: usize| {
        text.push_value(&" ".repeat(width.saturating_sub(used) + 2));
    };
    text.push_fixed("  Part");
    pad(text, "Part".len(), part_width);
    text.push_fixed("Status");
    pad(text, "Status".len(), status_width);
    text.push_fixed("Uses");
    pad(text, "Uses".len(), uses_width);
    text.push_fixed("What it's for").end_line();
    for row in &rows {
        text.push_fixed("  ").push_fixed(row.part);
        pad(text, row.part.chars().count(), part_width);
        let (words, tone) = row.status;
        let used = text.push_status(words, tone, styling);
        pad(text, used, status_width);
        text.push_fixed(row.uses);
        pad(text, row.uses.chars().count(), uses_width);
        text.push_fixed(row.purpose).end_line();
    }
}

/// A managed version the catalogue supersedes, with the step in the JSON's
/// words.
fn push_update(text: &mut TerminalText, update: &ManagedUpdate) {
    text.push_fixed(match update.reason {
        SupersessionReason::SecurityFixes => "Update available (security fixes): ",
        SupersessionReason::Maintenance => "Update available: ",
    })
    .push_value(update.component.identifier())
    .push_fixed(" is on ")
    .push_value(&update.selected_version)
    .end_line();
    text.push_fixed("  ")
        .push_value(&managed_update_next_step(update))
        .end_line();
}

/// The one plain line on where the person stands. "Complete" only ever
/// means video and speech both work.
fn push_verdict(text: &mut TerminalText, video: VideoState, speech: SpeechState) {
    if video == VideoState::Ready && speech == SpeechState::Ready {
        text.push_fixed("Setup complete: video and speech both work.")
            .end_line();
        return;
    }
    let video_clause = match video {
        VideoState::Ready => None,
        VideoState::Missing => Some("video is missing"),
        VideoState::NotWorking => Some("video is not working"),
        VideoState::NoAnswer => Some("video did not answer in time"),
    };
    let speech_clause = match speech {
        SpeechState::Ready => None,
        SpeechState::Missing { .. } => Some("speech is missing"),
        SpeechState::NotWorking => Some("speech is not working"),
        SpeechState::NoAnswer => Some("speech did not answer in time"),
        SpeechState::ModelMissing => Some("the speech model is missing"),
        SpeechState::ModelUnreadable => Some("the speech model cannot be read"),
        SpeechState::ModelUnknown => Some("the speech model is not one VSift knows"),
        SpeechState::TestFailed => Some("speech failed its test"),
        // Said once, by the video clause.
        SpeechState::WaitingForVideo => {
            video_clause.map_or(Some("speech cannot be tested yet"), |_| None)
        }
    };
    let merged = match (video, speech) {
        (VideoState::Missing, SpeechState::Missing { .. }) => Some("video and speech are missing"),
        (VideoState::NotWorking, SpeechState::NotWorking) => {
            Some("video and speech are not working")
        }
        (VideoState::NoAnswer, SpeechState::NoAnswer) => {
            Some("video and speech did not answer in time")
        }
        _ => None,
    };
    text.push_fixed("Your setup isn't complete: ");
    if let Some(both) = merged {
        text.push_fixed(both);
    } else {
        if let Some(clause) = video_clause {
            text.push_fixed(clause);
        }
        if let (Some(_), Some(_)) = (video_clause, speech_clause) {
            text.push_fixed(" and ");
        }
        if let Some(clause) = speech_clause {
            text.push_fixed(clause);
        }
    }
    text.push_fixed(".").end_line();
}

/// The name a person knows a dependency by. `FFprobe` is named only where a
/// step needs it, because it comes with `FFmpeg`.
const fn tool_name(dependency: RuntimeDependency) -> &'static str {
    match dependency {
        RuntimeDependency::Ffmpeg => "FFmpeg",
        RuntimeDependency::Ffprobe => "FFprobe",
        RuntimeDependency::Whisper => "whisper.cpp",
    }
}

/// Names in a sentence: `A`, `A and B`, `A, B and C`.
fn push_names(text: &mut TerminalText, names: &[&'static str]) {
    for (index, name) in names.iter().enumerate() {
        if index > 0 {
            text.push_fixed(if index + 1 == names.len() {
                " and "
            } else {
                ", "
            });
        }
        text.push_fixed(name);
    }
}

/// What happened to the tools that are not simply missing, and what a
/// missing speech part is made of.
fn push_details<L>(
    text: &mut TerminalText,
    states: [&DependencyState; 3],
    speech: SpeechState,
    lookup: &L,
) where
    L: Fn(RuntimeDependency) -> DependencyLookup,
{
    let mut broken = Vec::new();
    let mut unanswered = Vec::new();
    let mut not_tried = Vec::new();
    let mut deadline = None;
    for (dependency, state) in RuntimeDependency::ALL.into_iter().zip(states) {
        match state {
            DependencyState::Available { .. } => {}
            DependencyState::Missing => {
                if lookup(dependency) != DependencyLookup::FilteredPath {
                    text.push_fixed("The path given for ")
                        .push_fixed(tool_name(dependency))
                        .push_fixed(" was not found.")
                        .end_line();
                }
            }
            DependencyState::Unhealthy { .. } => broken.push(tool_name(dependency)),
            DependencyState::TimedOut(timeout) => {
                deadline.get_or_insert(timeout.shared_deadline);
                match timeout.kind {
                    ProbeTimeoutKind::NoAnswer => unanswered.push(tool_name(dependency)),
                    ProbeTimeoutKind::NotStarted => not_tried.push(tool_name(dependency)),
                }
            }
        }
    }
    if !broken.is_empty() {
        push_names(text, &broken);
        text.push_fixed(if broken.len() == 1 { " was" } else { " were" })
            .push_fixed(" found but did not run properly.")
            .end_line();
    }
    if let Some(deadline) = deadline {
        if !unanswered.is_empty() {
            push_names(text, &unanswered);
            text.push_fixed(if unanswered.len() == 1 {
                " was"
            } else {
                " were"
            })
            .push_fixed(" found but did not answer within ")
            .push_value(&probe_deadline_text(deadline))
            .push_fixed(", the time all three tool checks share.")
            .end_line();
        }
        if !not_tried.is_empty() {
            push_names(text, &not_tried);
            text.push_fixed(if not_tried.len() == 1 {
                " was"
            } else {
                " were"
            })
            .push_fixed(" found but not tried: the time ran out first.")
            .end_line();
        }
        text.push_fixed(
            "A first run after installing a tool or restarting the machine can be slow, for \
             example while a virus scanner reads a large program for the first time.",
        )
        .end_line();
        text.push_fixed(match ProbeTimeHeadroom::of(deadline) {
            ProbeTimeHeadroom::MoreTimeAllowed => {
                "To allow more time, add --timeout-seconds (up to 60)."
            }
            ProbeTimeHeadroom::AtTheMaximum => {
                "That was the longest wait allowed. If a tool never answers, the tool itself \
                 hangs: run it yourself to see what it does."
            }
        })
        .end_line();
    }
    match speech {
        SpeechState::Missing { with_model: true } => {
            text.push_fixed(
                "Speech needs two things, whisper.cpp and its model, and neither is set up yet.",
            )
            .end_line();
        }
        SpeechState::TestFailed => {
            text.push_fixed(
                "whisper.cpp and the model were found, but the short speech test did not pass.",
            )
            .end_line();
        }
        _ => {}
    }
}

/// On a complete setup, the optional extras that could be added here and
/// are not yet, with the door's command for adding one. A worker host is
/// set up by its operator, so it gets no such line.
pub(super) fn push_extras_line(
    text: &mut TerminalText,
    optional: &[(OptionalComponent, OptionalCapability)],
    profile: ExecutionProfile,
) {
    if profile == ExecutionProfile::Worker {
        return;
    }
    let addable = optional
        .iter()
        .any(|(_, capability)| *capability == OptionalCapability::NotInstalled);
    if addable {
        // Web videos are the only extra a catalogue has so far; the others
        // join this line as their groups land (ADR 0036 section 7).
        text.push_fixed(
            "Optional extras: web videos. Add one with: vsift setup --include web-videos",
        )
        .end_line();
    }
}

/// Where to read more, on its own line before the step.
pub(super) fn push_guide(text: &mut TerminalText, next: Next) {
    match next {
        Next::RegisterTool(_) | Next::RegisterModel => {
            text.push_fixed("The installation guide shows where to get it: ")
                .push_fixed(INSTALL_GUIDE)
                .end_line();
        }
        Next::LookAgain => {
            text.push_fixed("If it keeps failing, the troubleshooting guide helps: ")
                .push_fixed(TROUBLESHOOTING_GUIDE)
                .end_line();
        }
        Next::TryAVideo | Next::CheckAgain | Next::ReviewThePlan => {}
    }
}

/// Whether the step is written for the setup door, which checks again
/// itself, or for `setup check`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Door {
    /// The door: looking again means running it again.
    Present,
    /// `setup check` for a worker host.
    Absent,
}

/// The last line: the one step to take.
pub(super) fn push_next(text: &mut TerminalText, next: Next, door: Door) {
    text.push_fixed("Next: ");
    match next {
        Next::TryAVideo => text.push_fixed("vsift ingest ./your-video.mp4"),
        // A check that ran out of time is run again as `setup check`, where
        // `--timeout-seconds` allows more time.
        Next::LookAgain | Next::ReviewThePlan if door == Door::Present => {
            text.push_fixed("vsift setup")
        }
        Next::CheckAgain | Next::LookAgain => text.push_fixed("vsift setup check"),
        Next::ReviewThePlan => text.push_fixed(
            "vsift setup plan   (shows what VSift can install for you, and changes nothing)",
        ),
        Next::RegisterTool(dependency) => {
            // A found-but-broken tool and a missing one are fixed the same way.
            text.push_fixed(match dependency {
                RuntimeDependency::Ffmpeg => {
                    "vsift setup configure ffmpeg --executable <full-path-to-ffmpeg>"
                }
                RuntimeDependency::Ffprobe => {
                    "vsift setup configure ffprobe --executable <full-path-to-ffprobe>"
                }
                RuntimeDependency::Whisper => {
                    "vsift setup configure whisper --executable <full-path-to-whisper-cli>"
                }
            })
        }
        Next::RegisterModel => {
            text.push_fixed("vsift setup configure-model --file <full-path-to-model>")
        }
    };
    text.end_line();
}
