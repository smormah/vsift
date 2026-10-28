//! The setup door's readable text (ADR 0036): the opening table, the offer
//! and its one question, the install's results, and the closing lines.
//!
//! The table, the verdict and the details are `setup check`'s own
//! ([`push_overview`]); the door adds symbols and colour when the terminal
//! takes them, and ends, as every view does, with where the person stands
//! or the one thing to do next. Third-party detail appears at three moments
//! only: each piece's publisher, size and licence in the offer, one line per
//! piece after the install, and the table again. Exact addresses and
//! fingerprints stay in `setup plan`.

use vsift::{
    CorePartSet, InstallFailureReason, OptionalCapability, OptionalComponent, OptionalComponentSet,
    PieceProgress, PieceStage,
};
use vsift_contract::{SetupOutcome, SetupPartName, SetupResponse};

use super::{
    setup_check::{Door, Next, Standing, push_extras_line, push_guide, push_next, push_overview},
    text::{RenderedText, Styling, TerminalText, Tone, TooLarge},
    view::{InstallComponent, PlanAction, SetupPlan},
};
use crate::{
    command::ExecutionProfile,
    door::{CheckedSetup, InstallResults, QuestionLine, extra_capability},
};

/// Where a person sets up a part `VSift` cannot install here.
const DO_IT_YOURSELF_GUIDE: &str =
    "https://github.com/smormah/vsift/blob/main/docs/operations/install.md";

/// Where a person looks when an install keeps failing.
const TROUBLESHOOTING_GUIDE: &str =
    "https://github.com/smormah/vsift/blob/main/docs/guide/troubleshooting.md";

/// The first command to try once setup is complete; the verdict line above
/// it already says the setup is complete.
const TRY_IT: &str = "Try it: vsift ./your-video.mp4";

/// The columns of the longest piece name, `speech model`.
const PIECE_NAME_COLUMNS: usize = 12;

/// The cells of a progress bar.
const BAR_CELLS: u64 = 20;

/// A part of the setup, as a person meets it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Part {
    /// `FFmpeg`, with `FFprobe`.
    Video,
    /// whisper.cpp and its model.
    Speech,
    /// The optional web-video extra.
    WebVideos,
}

impl Part {
    const fn name(self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Speech => "Speech",
            Self::WebVideos => "Web videos",
        }
    }

    const fn in_a_sentence(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Speech => "speech",
            Self::WebVideos => "web videos",
        }
    }

    const fn is_core(self) -> bool {
        matches!(self, Self::Video | Self::Speech)
    }
}

/// A managed component by the name a person knows it, and the part it
/// belongs to; `None` for a component this build does not know, which the
/// offer still counts and installs but names by its identifier.
fn piece(component: &str) -> Option<(Part, &'static str)> {
    Some(match component {
        "ffmpeg_ffprobe" => (Part::Video, "FFmpeg"),
        "whisper_cli" => (Part::Speech, "whisper.cpp"),
        "whisper_model" => (Part::Speech, "speech model"),
        "yt_dlp" => (Part::WebVideos, "yt-dlp"),
        "js_runtime_deno" => (Part::WebVideos, "Deno"),
        _ => return None,
    })
}

/// One piece the door offers to download.
#[derive(Clone, Debug)]
struct OfferedPiece {
    part: Part,
    component: String,
    publisher: String,
    licence: String,
    bytes: u64,
}

/// What the door offers: every piece of the plan still to install, the
/// parts they make up, and the missing core parts `VSift` cannot install on
/// this computer.
#[derive(Clone, Debug)]
pub(crate) struct DoorOffer {
    pieces: Vec<OfferedPiece>,
    parts: Vec<Part>,
    unavailable: Vec<Part>,
    /// What each included extra adds, its size and its terms, in the plan's
    /// own words.
    extra_summaries: Vec<String>,
}

impl DoorOffer {
    /// The offer a plan makes for a setup that stands as `standing` does.
    pub(crate) fn of(plan: &SetupPlan, standing: Standing<'_>) -> Self {
        let pending = |action: &&PlanAction| action.state.as_deref() != Some("current");
        let mut pieces: Vec<OfferedPiece> = plan
            .actions
            .iter()
            .filter(pending)
            .map(|action| offered(action, Part::Video))
            .collect();
        let mut extra_summaries = Vec::new();
        for optional in &plan.optional_components {
            let before = pieces.len();
            pieces.extend(
                optional
                    .actions
                    .iter()
                    .filter(pending)
                    .map(|action| offered(action, Part::WebVideos)),
            );
            if pieces.len() > before {
                extra_summaries.push(optional.summary.clone());
            }
        }
        let mut parts: Vec<Part> = Vec::new();
        for piece in &pieces {
            if !parts.contains(&piece.part) {
                parts.push(piece.part);
            }
        }
        let missing = [
            (Part::Video, standing.video_missing()),
            (Part::Speech, standing.speech_missing()),
        ];
        let unavailable = missing
            .into_iter()
            .filter(|(part, missing)| *missing && !parts.contains(part))
            .map(|(part, _)| part)
            .collect();
        Self {
            pieces,
            parts,
            unavailable,
            extra_summaries,
        }
    }

    /// Nothing to download.
    pub(crate) fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    fn total_bytes(&self) -> u64 {
        self.pieces
            .iter()
            .fold(0_u64, |total, piece| total.saturating_add(piece.bytes))
    }
}

/// An action of the plan as a piece of the offer; `fallback` is the part
/// of a component this build does not know.
fn offered(action: &PlanAction, fallback: Part) -> OfferedPiece {
    OfferedPiece {
        part: piece(&action.component).map_or(fallback, |(part, _)| part),
        component: action.component.clone(),
        publisher: action.publisher.clone(),
        licence: action.licence.clone(),
        bytes: action.bytes,
    }
}

/// Writes a piece's name: the one a person knows, or the identifier.
fn push_piece_name(text: &mut TerminalText, component: &str) -> usize {
    if let Some((_, name)) = piece(component) {
        text.push_fixed(name);
        name.chars().count()
    } else {
        text.push_value(component);
        component.chars().count()
    }
}

/// A size in round megabytes, as a person reads a download: `about 60 MB`,
/// or `under 1 MB`.
fn size_words(bytes: u64) -> String {
    let megabytes = bytes.saturating_add(500_000) / 1_000_000;
    if megabytes == 0 {
        "under 1 MB".to_owned()
    } else {
        format!("about {megabytes} MB")
    }
}

fn push_size(text: &mut TerminalText, bytes: u64) {
    text.push_value(&size_words(bytes));
}

/// Parts in a sentence: `speech`, `video and speech`, `video, speech and
/// web videos`.
fn push_parts(text: &mut TerminalText, parts: &[Part]) {
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            text.push_fixed(if index + 1 == parts.len() {
                " and "
            } else {
                ", "
            });
        }
        text.push_fixed(part.in_a_sentence());
    }
}

/// Spaces after a cell of `used` columns, to `width` and two more.
fn pad(text: &mut TerminalText, used: usize, width: usize) {
    text.push_value(&" ".repeat(width.saturating_sub(used) + 2));
}

/// The first screen: the heading with the version, then `setup check`'s
/// table, verdict and details.
pub(crate) fn door_opening(
    checked: &CheckedSetup,
    version: &str,
    styling: Styling,
) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    push_heading(&mut text, version);
    push_overview(
        &mut text,
        checked.standing(),
        checked.managed(),
        styling,
        &|dependency| checked.lookup(dependency),
    );
    text.finish()
}

fn push_heading(text: &mut TerminalText, version: &str) {
    text.push_fixed("VSift ")
        .push_value(version)
        .push_fixed(" setup")
        .end_line()
        .blank_line();
}

/// Without a terminal: the read-only view, and why nothing was asked.
pub(crate) fn door_no_terminal(
    checked: &CheckedSetup,
    version: &str,
) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    push_heading(&mut text, version);
    push_overview(
        &mut text,
        checked.standing(),
        checked.managed(),
        Styling::Plain,
        &|dependency| checked.lookup(dependency),
    );
    text.push_fixed("Nobody is here to answer, so nothing was installed.")
        .end_line();
    let standing = checked.standing();
    let missing = [
        (Part::Video, standing.video_missing()),
        (Part::Speech, standing.speech_missing()),
    ];
    let missing: Vec<Part> = missing
        .into_iter()
        .filter(|(_, missing)| *missing)
        .map(|(part, _)| part)
        .collect();
    if !missing.is_empty() && !standing.timed_out() {
        text.push_fixed("To install without questions: vsift setup --unattended --install ");
        for (index, part) in missing.iter().enumerate() {
            if index > 0 {
                text.push_fixed(",");
            }
            text.push_fixed(part.in_a_sentence());
        }
        text.end_line();
    }
    text.finish()
}

/// The close when there is nothing to download: the setup is complete, its
/// checks ran out of time, or what is missing cannot be installed here.
pub(crate) fn door_nothing_to_offer(
    checked: &CheckedSetup,
    include: OptionalComponentSet,
) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    push_close(&mut text, checked, include);
    text.finish()
}

/// Where the person stands when nothing more can be installed now, and the
/// one thing to do next.
fn push_close(text: &mut TerminalText, checked: &CheckedSetup, include: OptionalComponentSet) {
    let standing = checked.standing();
    let _noted = push_extras_not_here(text, checked, include);
    if standing.complete() {
        push_extras_line(text, &checked.optional, ExecutionProfile::Desktop);
        text.push_fixed(TRY_IT).end_line();
        return;
    }
    if !standing.timed_out() {
        let missing = [
            (Part::Video, standing.video_missing()),
            (Part::Speech, standing.speech_missing()),
        ];
        for (part, _) in missing.into_iter().filter(|(_, missing)| *missing) {
            push_cannot_install(text, part);
        }
    }
    let next = Next::of(standing, checked.managed());
    push_guide(text, next);
    push_next(text, next, Door::Present);
}

/// Included extras that cannot be added on this computer; whether it
/// wrote anything.
fn push_extras_not_here(
    text: &mut TerminalText,
    checked: &CheckedSetup,
    include: OptionalComponentSet,
) -> bool {
    let mut wrote = false;
    for extra in include.iter() {
        if extra_capability(checked, extra) == Some(&OptionalCapability::Unavailable) {
            let OptionalComponent::OnlineLinks = extra;
            text.push_fixed("VSift can't add web videos on this computer yet.")
                .end_line();
            wrote = true;
        }
    }
    wrote
}

fn push_cannot_install(text: &mut TerminalText, part: Part) {
    text.push_fixed("VSift can't install ")
        .push_fixed(part.in_a_sentence())
        .push_fixed(" on this computer yet.")
        .end_line();
}

/// The offer: what cannot be installed here, then each piece with its
/// publisher, size and licence, and what the downloads come to.
pub(crate) fn door_offer(
    offer: &DoorOffer,
    include: OptionalComponentSet,
    checked: &CheckedSetup,
) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    text.end_line();
    let mut noted = push_extras_not_here(&mut text, checked, include);
    for part in &offer.unavailable {
        push_cannot_install(&mut text, *part);
    }
    if !offer.unavailable.is_empty() {
        text.push_fixed("The installation guide shows how to set it up yourself: ")
            .push_fixed(DO_IT_YOURSELF_GUIDE)
            .end_line();
        noted = true;
    }
    if noted {
        text.blank_line();
    }
    text.push_fixed("VSift can ")
        .push_fixed(if offer.parts.iter().any(|part| part.is_core()) {
            "install "
        } else {
            "add "
        });
    push_parts(&mut text, &offer.parts);
    text.push_fixed(" for you:").end_line().blank_line();

    let names: Vec<usize> = offer
        .pieces
        .iter()
        .map(|piece| name_columns(&piece.component))
        .collect();
    let name_width = names.iter().copied().max().unwrap_or(0);
    let publisher_width = offer
        .pieces
        .iter()
        .map(|piece| piece.publisher.chars().count() + "from ".len())
        .max()
        .unwrap_or(0);
    let size_width = offer
        .pieces
        .iter()
        .map(|piece| size_words(piece.bytes).len())
        .max()
        .unwrap_or(0);
    for piece in &offer.pieces {
        text.push_fixed("  ");
        let used = push_piece_name(&mut text, &piece.component);
        pad(&mut text, used, name_width);
        text.push_fixed("from ").push_value(&piece.publisher);
        pad(
            &mut text,
            piece.publisher.chars().count() + "from ".len(),
            publisher_width,
        );
        let size = size_words(piece.bytes);
        text.push_value(&size);
        pad(&mut text, size.len(), size_width);
        text.push_value(&piece.licence).end_line();
    }
    for summary in &offer.extra_summaries {
        text.blank_line().push_value(summary).end_line();
    }
    text.blank_line();
    text.push_fixed("That is ")
        .push_value(&size_words(offer.total_bytes()))
        .push_fixed(" to download, which can take a few minutes.")
        .end_line();
    text.push_fixed("Each download is checked against a fixed fingerprint before it is used,")
        .end_line()
        .push_fixed("and it is installed for your user only.")
        .end_line();
    text.push_fixed("To see the exact addresses and fingerprints first: vsift setup plan")
        .end_line();
    text.finish()
}

/// The columns a piece's name takes.
fn name_columns(component: &str) -> usize {
    piece(component).map_or_else(
        || component.chars().count(),
        |(_, name)| name.chars().count(),
    )
}

/// The one question, with the cursor left after it.
pub(crate) fn door_question(offer: &DoorOffer) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    text.end_line();
    text.push_fixed(if offer.parts.iter().any(|part| part.is_core()) {
        "Install "
    } else {
        "Add "
    });
    push_parts(&mut text, &offer.parts);
    text.push_fixed(" now? [y/N] ");
    text.finish_question()
}

/// Moves off the question's line when the person's Enter has not; a typed
/// answer's own line break is the only break after the question.
fn leave_question(text: &mut TerminalText, line: QuestionLine) {
    if line == QuestionLine::Open {
        text.end_line();
    }
}

/// The close after a no.
pub(crate) fn door_declined(line: QuestionLine) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    leave_question(&mut text, line);
    text.push_fixed("Nothing was installed. Run vsift setup again when you're ready.")
        .end_line();
    text.finish()
}

/// The heading of the install.
pub(crate) fn door_installing(
    offer: &DoorOffer,
    line: QuestionLine,
) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    leave_question(&mut text, line);
    text.push_fixed("Installing ");
    push_parts(&mut text, &offer.parts);
    text.push_fixed(": ")
        .push_unsigned(offer.pieces.len() as u64);
    text.push_fixed(if offer.pieces.len() == 1 {
        " download, "
    } else {
        " downloads, "
    });
    push_size(&mut text, offer.total_bytes());
    text.end_line();
    text.push_fixed("Ctrl-C stops it and keeps what has finished.")
        .end_line();
    text.finish()
}

/// One line per piece, with the part named on its first piece.
pub(crate) fn door_install_results(
    installed: &InstallResults,
    styling: Styling,
) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    text.end_line();
    let video_failed = installed.components.iter().any(|component| {
        piece(&component.component).map(|(part, _)| part) == Some(Part::Video)
            && component.status == "failed"
    });
    let part_width = installed
        .components
        .iter()
        .filter_map(|component| piece(&component.component))
        .map(|(part, _)| part.name().chars().count())
        .max()
        .unwrap_or(0);
    let name_width = installed
        .components
        .iter()
        .map(|component| name_columns(&component.component))
        .max()
        .unwrap_or(0);
    let mut last_part = None;
    for component in &installed.components {
        let part = piece(&component.component).map(|(part, _)| part);
        text.push_fixed("  ");
        let shown = match part {
            Some(part) if last_part != Some(part) => {
                text.push_fixed(part.name());
                part.name().chars().count()
            }
            _ => 0,
        };
        last_part = part;
        pad(&mut text, shown, part_width);
        let used = push_piece_name(&mut text, &component.component);
        pad(&mut text, used, name_width);
        let (words, tone) = piece_outcome(component, part, video_failed);
        text.push_status(words, tone, styling);
        text.end_line();
    }
    text.finish()
}

/// What a piece's stage says, in plain words.
fn stage_words(stage: PieceStage) -> &'static str {
    match stage {
        PieceStage::Waiting => "waiting",
        PieceStage::Downloading => "downloading",
        PieceStage::Checking => "checking",
        PieceStage::Testing => "testing",
        PieceStage::Ready => "ready",
        PieceStage::AlreadyCurrent => "already there",
        PieceStage::WaitingForVideo => "waiting for video",
        PieceStage::Failed(failure) => match failure.reason {
            InstallFailureReason::Blocked => "not tried",
            InstallFailureReason::Cancelled => "stopped",
            reason => failure_words(reason.identifier()),
        },
    }
}

/// Whether a piece has reached the end of this run.
pub(crate) const fn piece_finished(stage: PieceStage) -> bool {
    matches!(
        stage,
        PieceStage::Ready
            | PieceStage::AlreadyCurrent
            | PieceStage::WaitingForVideo
            | PieceStage::Failed(_)
    )
}

/// Megabytes, rounded, as a progress line counts them.
fn megabytes(bytes: u64) -> u64 {
    bytes.saturating_add(500_000) / 1_000_000
}

/// One live progress line per piece while the door installs (ADR 0036
/// section 5): the name, a bar, the megabytes so far of the reviewed size,
/// and the stage. Built from fixed words and figures only.
pub(crate) fn piece_bars(pieces: &[PieceProgress]) -> Vec<String> {
    pieces
        .iter()
        .map(|piece| {
            let filled = piece
                .received
                .min(piece.total)
                .saturating_mul(BAR_CELLS)
                .checked_div(piece.total)
                .unwrap_or(BAR_CELLS);
            let bar: String = (0..BAR_CELLS)
                .map(|cell| {
                    if cell < filled {
                        '\u{2588}'
                    } else {
                        '\u{2591}'
                    }
                })
                .collect();
            format!(
                "  {:<width$}  {bar}  {:>4} of {} MB  {}",
                piece_name(piece),
                megabytes(piece.received),
                megabytes(piece.total),
                stage_words(piece.stage),
                width = PIECE_NAME_COLUMNS,
            )
        })
        .collect()
}

fn piece_name(progress: &PieceProgress) -> &'static str {
    piece(progress.component.identifier())
        .map_or_else(|| progress.component.identifier(), |(_, name)| name)
}

/// The plain line of a piece that finished, written when there are no bars
/// (the unattended form).
pub(crate) fn piece_finished_line(progress: &PieceProgress) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    text.push_fixed("  ");
    let name = piece_name(progress);
    text.push_fixed(name);
    pad(&mut text, name.chars().count(), PIECE_NAME_COLUMNS);
    text.push_fixed(stage_words(progress.stage)).end_line();
    text.finish()
}

/// The heading of an unattended run that installs.
pub(crate) fn door_unattended_installing(
    version: &str,
    parts: CorePartSet,
    include: OptionalComponentSet,
) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    push_heading(&mut text, version);
    let mut named: Vec<Part> = parts
        .iter()
        .map(|part| match part {
            vsift::CorePart::Video => Part::Video,
            vsift::CorePart::Speech => Part::Speech,
        })
        .collect();
    if include.contains(OptionalComponent::OnlineLinks) {
        named.push(Part::WebVideos);
    }
    text.push_fixed("Installing ");
    push_parts(&mut text, &named);
    text.push_fixed(" without questions:").end_line();
    text.finish()
}

/// The close of an unattended run: the table, the outcome, and each
/// part's next step.
pub(crate) fn door_unattended_result(
    after: &CheckedSetup,
    response: &SetupResponse,
    heading: Option<&str>,
) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    match heading {
        Some(version) => push_heading(&mut text, version),
        None => {
            text.end_line();
        }
    }
    push_overview(
        &mut text,
        after.standing(),
        after.managed(),
        Styling::Plain,
        &|dependency| after.lookup(dependency),
    );
    text.push_fixed(match response.outcome {
        SetupOutcome::Pass => "Result: pass",
        SetupOutcome::Fail => "Result: fail",
    })
    .end_line();
    for part in &response.parts {
        if let Some(next) = &part.next_step {
            text.push_fixed("Next for ")
                .push_fixed(match part.part {
                    SetupPartName::Video => "video",
                    SetupPartName::Speech => "speech",
                    SetupPartName::WebVideos => "web videos",
                })
                .push_fixed(": ")
                .push_value(next)
                .end_line();
        }
    }
    text.finish()
}

/// A piece's result in plain words.
fn piece_outcome(
    component: &InstallComponent,
    part: Option<Part>,
    video_failed: bool,
) -> (&'static str, Tone) {
    match component.status.as_str() {
        "activated" => ("ready", Tone::Good),
        "already_current" => ("already there", Tone::Good),
        _ => match component.reason.as_deref() {
            Some("blocked") if part == Some(Part::Speech) && video_failed => {
                ("waiting for video", Tone::Quiet)
            }
            Some("blocked") => ("not tried", Tone::Quiet),
            Some("cancelled") => ("stopped", Tone::Quiet),
            Some(reason) => (failure_words(reason), Tone::Bad),
            None => ("did not install", Tone::Bad),
        },
    }
}

/// What went wrong, for a reason identifier of the `setup install` result.
fn failure_words(reason: &str) -> &'static str {
    match reason {
        "offline" => "the download stopped: no connection",
        "tls" => "the download stopped: the secure connection failed",
        "proxy_auth" => "the download stopped: the proxy asked for a password",
        "redirect_policy" | "http_status" => {
            "the download stopped: the server answered unexpectedly"
        }
        "size" | "size_mismatch" | "digest_mismatch" | "review_mismatch" => {
            "the download did not match its fingerprint"
        }
        "storage" => "it could not be saved in your user folder",
        _ => "it did not pass its test on this computer",
    }
}

/// What a person does about the first failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Advice {
    Connection,
    Space,
    Again,
    WhenReady,
}

fn advice(installed: &InstallResults) -> Option<Advice> {
    installed
        .components
        .iter()
        .filter(|component| component.status == "failed")
        .find_map(|component| match component.reason.as_deref() {
            Some("blocked") => None,
            Some("cancelled") => Some(Advice::WhenReady),
            Some("offline" | "tls" | "proxy_auth" | "redirect_policy" | "http_status") => {
                Some(Advice::Connection)
            }
            Some("storage") => Some(Advice::Space),
            _ => Some(Advice::Again),
        })
}

/// After the install: the table again, then where the person stands and
/// the one thing to do next.
pub(crate) fn door_after_install(
    after: &CheckedSetup,
    installed: &InstallResults,
    include: OptionalComponentSet,
    styling: Styling,
) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    text.end_line();
    let standing = after.standing();
    push_overview(
        &mut text,
        standing,
        after.managed(),
        styling,
        &|dependency| after.lookup(dependency),
    );
    match advice(installed) {
        Some(Advice::Connection) => {
            text.push_fixed("Next: check your connection, then run  vsift setup")
                .end_line();
        }
        Some(Advice::Space) => {
            text.push_fixed("Next: make some room in your user folder, then run  vsift setup")
                .end_line();
        }
        Some(Advice::WhenReady) => {
            text.push_fixed("Next: run  vsift setup  when you're ready")
                .end_line();
        }
        Some(Advice::Again) => {
            text.push_fixed("If it fails again, the troubleshooting guide helps: ")
                .push_fixed(TROUBLESHOOTING_GUIDE)
                .end_line()
                .push_fixed("Next: vsift setup")
                .end_line();
        }
        None => push_close(&mut text, after, include),
    }
    text.finish()
}
