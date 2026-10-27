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
//! - **The last line is the one next step.** Until the setup door ships, it
//!   names the concrete command; [`Next`] is the single place that changes
//!   when `vsift setup` takes over.
//!
//! The words and the table use ASCII only: the symbols and colour of the
//! design note belong to the door, which knows whether a terminal is there.

use vsift::{
    DependencyState, LocalAsrCheckOutcome, LocalAsrModelStatus, LocalAsrNotRunReason,
    LocalAsrSetupStatus, ManagedPlanAvailability, ManagedUpdate, OptionalCapability,
    OptionalComponent, ProbeTimeoutKind, RuntimeDependency, RuntimeDiagnosis, SupersessionReason,
};
use vsift_contract::{
    DependencyLookup, ProbeTimeHeadroom, managed_update_next_step, probe_deadline_text,
};

use super::text::{RenderedText, TerminalText, TooLarge};
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

/// The one next step, until `vsift setup` takes the walk-through over.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Next {
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
    fn of(
        states: [&DependencyState; 3],
        speech: SpeechState,
        complete: bool,
        managed: ManagedSetupFacts<'_>,
    ) -> Self {
        if complete {
            return Self::TryAVideo;
        }
        if states
            .iter()
            .any(|state| matches!(state, DependencyState::TimedOut(_)))
        {
            return Self::CheckAgain;
        }
        let needs_a_tool = RuntimeDependency::ALL
            .into_iter()
            .zip(states)
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
        match speech {
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
    let states = RuntimeDependency::ALL.map(|dependency| {
        diagnosis
            .dependencies
            .iter()
            .find(|status| status.dependency == dependency)
            .map_or(&NOT_PROBED, |status| &status.state)
    });
    let [ffmpeg, ffprobe, whisper] = states;
    let video = VideoState::of(ffmpeg, ffprobe);
    let speech = SpeechState::of(whisper, local_asr);
    let complete = video == VideoState::Ready && speech == SpeechState::Ready;

    let mut text = TerminalText::result();
    text.push_fixed("VSift setup check").end_line().blank_line();
    push_table(&mut text, video, speech, managed.optional);
    text.blank_line();
    for update in managed.updates {
        push_update(&mut text, update);
    }
    text.blank_line();
    push_verdict(&mut text, video, speech);
    push_details(&mut text, states, speech, lookup);
    if complete {
        push_extras_hint(&mut text, managed.optional, profile);
    } else if video == VideoState::Ready {
        text.push_fixed("Video works, so a transcript you already have can still be used.")
            .end_line();
    }
    let next = Next::of(states, speech, complete, managed);
    push_guide(&mut text, next);
    push_next(&mut text, next);
    text.finish()
}

/// One row of the table. Every cell is `VSift`'s own fixed word.
struct Row {
    part: &'static str,
    status: &'static str,
    uses: &'static str,
    purpose: &'static str,
}

/// The table: video, speech, then each optional extra, always shown.
fn push_table(
    text: &mut TerminalText,
    video: VideoState,
    speech: SpeechState,
    optional: &[(OptionalComponent, OptionalCapability)],
) {
    let mut rows = vec![
        Row {
            part: "Video",
            status: video.status(),
            uses: "FFmpeg",
            purpose: "reading videos, taking frames",
        },
        Row {
            part: "Speech",
            status: speech.status(),
            uses: "whisper.cpp",
            purpose: "turning speech into text",
        },
    ];
    for (component, capability) in optional {
        let OptionalComponent::OnlineLinks = component;
        rows.push(Row {
            part: "Web videos",
            status: match capability {
                OptionalCapability::Installed(_) => "added",
                OptionalCapability::NotInstalled => "none added",
                OptionalCapability::Unavailable => "not available",
            },
            uses: "yt-dlp",
            purpose: "web page videos (optional)",
        });
    }
    let header = Row {
        part: "Part",
        status: "Status",
        uses: "Uses",
        purpose: "What it's for",
    };
    let width = |cell: fn(&Row) -> &'static str| {
        rows.iter()
            .chain(std::iter::once(&header))
            .map(|row| cell(row).len())
            .max()
            .unwrap_or(0)
    };
    let widths = [
        width(|row| row.part),
        width(|row| row.status),
        width(|row| row.uses),
    ];
    for row in std::iter::once(&header).chain(rows.iter()) {
        text.push_fixed("  ");
        for (cell, width) in [row.part, row.status, row.uses].into_iter().zip(widths) {
            text.push_fixed(cell)
                .push_value(&" ".repeat(width - cell.len() + 2));
        }
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

/// On a complete setup, the optional extras that are not added yet.
fn push_extras_hint(
    text: &mut TerminalText,
    optional: &[(OptionalComponent, OptionalCapability)],
    profile: ExecutionProfile,
) {
    if profile == ExecutionProfile::Worker {
        return;
    }
    for (component, capability) in optional {
        if *capability == OptionalCapability::NotInstalled {
            let OptionalComponent::OnlineLinks = component;
            text.push_fixed(
                "Optional extra: web videos. To see what it would add: vsift setup plan --include \
                 online_links",
            )
            .end_line();
        }
    }
}

/// Where to read more, on its own line before the step.
fn push_guide(text: &mut TerminalText, next: Next) {
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

/// The last line: the one step to take.
fn push_next(text: &mut TerminalText, next: Next) {
    text.push_fixed("Next: ");
    match next {
        Next::TryAVideo => text.push_fixed("vsift ingest ./your-video.mp4"),
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
