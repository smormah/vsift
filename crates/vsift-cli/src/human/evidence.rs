//! `candidates`, `frame get/neighbours/burst`, `crop` and `audio` (P13 PR
//! 2b).
//!
//! None of these results carries evidence text: candidates are times,
//! counts and hashes, and frames, crops and clips are files. What they do
//! carry is each delivered file's absolute path (`files[].path`), which
//! holds the session root the user chose. Each path is written on a line of
//! its own through [`TerminalText::push_path_line`], so it can be copied
//! whole (L-016), under the item it delivers. A path in Windows'
//! extended-length form (`\\?\`) gets one note saying how to open it, and a
//! path the builder could not write exactly (a control or hidden character,
//! or one too long for a line) gets one note pointing to `--json`.

use super::{
    push_outcome,
    text::{PathLine, RenderedText, TerminalText, TooLarge},
    time::{push_clock, push_range, push_span},
    transcript::{count, push_next_page},
    view::{
        AudioData, AudioEvidence, CandidatesPage, DeliveredFile, Envelope, FrameData,
        FrameEvidence, Range, Selection, VisualCandidate,
    },
};

/// Written once after the files when a path uses Windows' extended-length
/// form. The 125 characters are those of a session root whose artifact
/// paths stay below `MAX_PATH` (known limit L-016).
const EXTENDED_PATH_NOTE: &str = "Note: a path starting with \\\\?\\ is in Windows' \
     extended-length form. It is exact, but some programs refuse it: copy the file out with \
     PowerShell's Copy-Item -LiteralPath '<path>' <destination>, or use a --session-root of at \
     most 125 characters to get plain paths.";

/// Written once after the files when a path is not shown exactly.
const INEXACT_PATH_NOTE: &str = "Note: a path above is not shown exactly (it holds control \
     or hidden characters, shown replaced or as <U+XXXX>, or is too long for a line); read it \
     with --json.";

/// Written under an audio clip's path: who the clip is for (#340). A coding
/// agent is handed the path of a WAV file it cannot play; in the cold round
/// that found this, one agent read the file's bytes with `base64` instead.
/// The sentence says in one line, where the path is read, what `audio --help`
/// says at more length.
pub(super) const AUDIO_LISTENER_NOTE: &str = "  This clip is for a person or a speech tool to play; a coding agent cannot listen to it, so an agent reads what was said with vsift transcript get.";

/// The heading of the files, which say how long they stay valid.
const FILES_HEADING: &str =
    "Evidence (each file is valid while the session exists; retain the session to keep it):";

/// The prefix of Windows' extended-length path form.
const EXTENDED_LENGTH_PREFIX: &str = r"\\?\";

/// `candidates`: one page of visual candidates and what the index covers.
pub(super) fn candidates(envelope: &Envelope<CandidatesPage>) -> Result<RenderedText, TooLarge> {
    let page = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("Visual candidates of session ")
        .push_value(&page.session_id)
        .end_line();
    let index = &page.index;
    text.push_fixed("Index: ")
        .push_value(&index.index_id)
        .push_fixed(" (revision ")
        .push_unsigned(index.number)
        .push_fixed(", ")
        .push_value(&index.profile)
        .push_fixed(", video length ");
    push_clock(&mut text, index.duration_us);
    text.push_fixed(")").end_line();
    text.push_fixed("Range: ");
    push_range(&mut text, page.range);
    text.end_line();
    let coverage = &page.coverage;
    text.push_fixed("Searched: ");
    push_range(&mut text, coverage.searched_range);
    text.end_line();
    if coverage.analyzed.is_empty() {
        text.push_fixed("Analysed: nothing yet").end_line();
    } else {
        text.push_fixed("Analysed:").end_line();
        for range in &coverage.analyzed {
            text.push_fixed("  ");
            push_range(&mut text, *range);
            text.end_line();
        }
    }
    if !coverage.gaps.is_empty() {
        text.push_fixed("Not covered (no candidates there):")
            .end_line();
    }
    for gap in &coverage.gaps {
        text.push_fixed("  ");
        push_range(
            &mut text,
            Range {
                from_us: gap.from_us,
                to_us: gap.to_us,
            },
        );
        text.push_fixed(": ").push_value(&gap.reason);
        if gap.dropped_candidates > 0 {
            text.push_fixed(", ")
                .push_unsigned(gap.dropped_candidates)
                .push_fixed(" change candidates dropped");
        }
        text.end_line();
    }
    if coverage.ranges_truncated {
        text.push_fixed("Range lists are cut at 100 each; use --json for the full coverage.")
            .end_line();
    }
    text.push_fixed("Candidates on this page: ")
        .push_unsigned(count(page.items.len()))
        .end_line();
    for candidate in &page.items {
        push_candidate(&mut text, candidate);
    }
    if let Some(first) = page.items.first() {
        text.blank_line();
        text.push_fixed("Get a candidate's frame: vsift frame get ")
            .push_value(&page.session_id)
            .push_fixed(" --candidate ")
            .push_value(&first.candidate_id)
            .end_line();
    }
    push_next_page(&mut text, page.next_cursor.as_deref());
    push_outcome(&mut text, envelope);
    text.finish()
}

/// Writes one candidate: its identity, time and reasons, then its span and
/// what changed.
fn push_candidate(text: &mut TerminalText, candidate: &VisualCandidate) {
    text.blank_line();
    text.push_value(&candidate.candidate_id).push_fixed("  at ");
    push_clock(text, candidate.representative_us);
    text.push_fixed("  ")
        .push_value(&candidate.reasons.join(", "))
        .end_line();
    text.push_fixed("  Span ");
    push_span(text, candidate.span.from_us, candidate.span.to_us);
    let dimensions = candidate.displayed_dimensions;
    text.push_fixed(", ")
        .push_value(&candidate.stability)
        .push_fixed(", ")
        .push_unsigned(candidate.sample_count)
        .push_fixed(" samples, ")
        .push_unsigned(dimensions.width)
        .push_fixed("x")
        .push_unsigned(dimensions.height)
        .push_fixed(", visual hash ")
        .push_value(&candidate.visual_hash)
        .end_line();
    if let Some(window) = candidate.change_window {
        text.push_fixed("  Changed between ");
        push_clock(text, window.from_us);
        text.push_fixed(" and ");
        push_clock(text, window.to_us);
        if let Some(change) = &candidate.change {
            text.push_fixed(": ")
                .push_unsigned(change.changed_blocks)
                .push_fixed(" blocks changed, the largest by ")
                .push_unsigned(change.max_block_delta);
        }
        text.end_line();
    }
}

/// `frame get`, `frame neighbours`, `frame burst` and `crop`: the request,
/// how each requested time resolved, and every item with its file.
pub(super) fn frames(envelope: &Envelope<FrameData>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let request = &data.request;
    let mut text = TerminalText::result();
    if let Some(parent) = &request.parent_evidence_id {
        text.push_fixed("Crop of ")
            .push_value(parent)
            .push_fixed(" in session ");
    } else if let Some(anchor) = &request.anchor_evidence_id {
        text.push_fixed("Neighbours of ")
            .push_value(anchor)
            .push_fixed(" in session ");
    } else if data.burst.is_some() {
        text.push_fixed("Burst of frames in session ");
    } else {
        text.push_fixed("Frame of session ");
    }
    text.push_value(&data.session_id).end_line();
    text.push_fixed("Source: ")
        .push_value(&data.source_id)
        .end_line();
    push_frame_request(&mut text, data);
    push_provenance(&mut text, data.reused, &data.profile, &data.source_check);
    push_partial_reason(&mut text, data.partial_reason.as_deref());
    push_selections(&mut text, &data.selections);
    let mut paths = PathNotes::default();
    text.blank_line();
    text.push_fixed(FILES_HEADING).end_line();
    for item in &data.items {
        push_frame_item(&mut text, item);
        paths.push_files_of(&mut text, &item.evidence_id, &data.files);
    }
    paths.push_unmatched(&mut text, &data.files, |file| {
        data.items
            .iter()
            .any(|item| item.evidence_id == file.evidence_id)
    });
    paths.push_notes(&mut text);
    push_outcome(&mut text, envelope);
    text.finish()
}

/// Writes what a frame or crop call asked for.
fn push_frame_request(text: &mut TerminalText, data: &FrameData) {
    let request = &data.request;
    text.push_fixed("Requested: ");
    if let Some(rect) = request.rect {
        text.push_fixed("rectangle x ")
            .push_unsigned(rect.x)
            .push_fixed(", y ")
            .push_unsigned(rect.y)
            .push_fixed(", ")
            .push_unsigned(rect.width)
            .push_fixed("x")
            .push_unsigned(rect.height)
            .push_fixed(" of the parent image");
    } else if let Some(count) = request.count {
        text.push_fixed("up to ")
            .push_unsigned(count)
            .push_fixed(" frames on each side");
    } else if let (Some(from_us), Some(to_us)) = (request.from_us, request.to_us) {
        text.push_fixed("up to ")
            .push_unsigned(request.max_frames.unwrap_or_default())
            .push_fixed(" frames over ");
        push_range(text, Range { from_us, to_us });
    } else if let Some(candidate) = &request.candidate_id {
        text.push_fixed("the frame of candidate ")
            .push_value(candidate);
    } else if let Some(at_us) = request.at_us {
        push_clock(text, at_us);
        if let Some(policy) = &request.policy {
            text.push_fixed(", ").push_value(policy);
        }
        if let Some(tolerance) = request.tolerance_us {
            text.push_fixed(", tolerance ")
                .push_unsigned(tolerance)
                .push_fixed(" us");
        }
    }
    text.end_line();
    if let Some(stops) = &data.neighbours {
        for (label, stop) in [
            ("Before: ", &stops.before_stop),
            ("After: ", &stops.after_stop),
        ] {
            text.push_fixed(label);
            match stop {
                Some(reason) => text.push_fixed("stopped short at ").push_value(reason),
                None => text.push_fixed("every frame asked for"),
            };
            text.end_line();
        }
    }
    if let Some(burst) = &data.burst {
        text.push_fixed("Planned: ")
            .push_unsigned(burst.targets)
            .push_fixed(" targets over ");
        push_range(text, burst.planned.into());
        text.push_fixed(" (")
            .push_value(&burst.extent)
            .push_fixed("), ")
            .push_unsigned(burst.distinct)
            .push_fixed(" distinct frames")
            .end_line();
    }
}

/// Writes one frame or crop item's line and its image's facts.
fn push_frame_item(text: &mut TerminalText, item: &FrameEvidence) {
    let frame = &item.frame;
    text.blank_line();
    text.push_value(&item.evidence_id);
    if let Some(crop) = &item.crop {
        text.push_fixed("  crop ")
            .push_unsigned(crop.width)
            .push_fixed("x")
            .push_unsigned(crop.height)
            .push_fixed(" at x ")
            .push_unsigned(crop.x)
            .push_fixed(", y ")
            .push_unsigned(crop.y)
            .push_fixed(" of ")
            .push_value(&crop.parent_evidence_id)
            .end_line();
        text.push_fixed("  From the frame at ");
        push_clock(text, frame.time_us);
        text.push_fixed(" (x ")
            .push_unsigned(crop.frame_x)
            .push_fixed(", y ")
            .push_unsigned(crop.frame_y)
            .push_fixed(" there; ");
    } else {
        text.push_fixed("  frame at ");
        push_clock(text, frame.time_us);
        text.push_fixed(", ")
            .push_unsigned(frame.width)
            .push_fixed("x")
            .push_unsigned(frame.height)
            .push_fixed(" (");
    }
    text.push_fixed("stream ")
        .push_unsigned(item.stream_index)
        .push_fixed(", pts ")
        .push_signed(frame.pts)
        .push_fixed(")")
        .end_line();
    let image = &item.image;
    text.push_fixed("  Image ")
        .push_unsigned(image.width)
        .push_fixed("x")
        .push_unsigned(image.height)
        .push_fixed(", ")
        .push_unsigned(image.bytes)
        .push_fixed(" bytes, sha256 ")
        .push_value(&image.sha256)
        .end_line();
}

/// `audio`: the clip, its range and its file.
pub(super) fn audio(envelope: &Envelope<AudioData>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("Audio clip of session ")
        .push_value(&data.session_id)
        .end_line();
    text.push_fixed("Source: ")
        .push_value(&data.source_id)
        .end_line();
    text.push_fixed("Requested: ");
    push_range(&mut text, data.request);
    text.end_line();
    if data.range_clipped {
        text.push_fixed("The range ran past the end of the source; the clip ends there.")
            .end_line();
    }
    push_provenance(&mut text, data.reused, &data.profile, &data.source_check);
    push_partial_reason(&mut text, data.partial_reason.as_deref());
    push_selections(&mut text, &data.selections);
    let mut paths = PathNotes::default();
    text.blank_line();
    text.push_fixed(FILES_HEADING).end_line();
    for item in &data.items {
        push_audio_item(&mut text, item);
        paths.push_files_of(&mut text, &item.evidence_id, &data.files);
    }
    paths.push_unmatched(&mut text, &data.files, |file| {
        data.items
            .iter()
            .any(|item| item.evidence_id == file.evidence_id)
    });
    text.push_fixed(AUDIO_LISTENER_NOTE).end_line();
    paths.push_notes(&mut text);
    push_outcome(&mut text, envelope);
    text.finish()
}

/// Writes one audio item's line and its clip's facts.
fn push_audio_item(text: &mut TerminalText, item: &AudioEvidence) {
    text.blank_line();
    text.push_value(&item.evidence_id).push_fixed("  audio ");
    push_range(text, item.range.into());
    text.push_fixed(", first sample at ");
    push_clock(text, item.actual_start_us);
    text.push_fixed(" (stream ")
        .push_unsigned(item.stream_index)
        .push_fixed(")")
        .end_line();
    let audio = &item.audio;
    text.push_fixed("  WAV ")
        .push_unsigned(audio.sample_rate)
        .push_fixed(" Hz, ");
    if audio.channels == 1 {
        text.push_fixed("mono");
    } else {
        text.push_unsigned(audio.channels).push_fixed(" channels");
    }
    text.push_fixed(", ")
        .push_value(&audio.sample_format)
        .push_fixed(", ")
        .push_unsigned(audio.bytes)
        .push_fixed(" bytes, sha256 ")
        .push_value(&audio.sha256)
        .end_line();
}

/// Writes whether the record was reused, and how it was made.
fn push_provenance(text: &mut TerminalText, reused: bool, profile: &str, source_check: &str) {
    text.push_fixed(if reused {
        "Reused: an identical earlier request's record; nothing was extracted or written"
    } else {
        "Extracted now"
    })
    .push_fixed(" (profile ")
    .push_value(profile)
    .push_fixed(", source copy checked by ")
    .push_value(source_check)
    .push_fixed(")")
    .end_line();
}

/// Writes which item each requested time resolved to.
fn push_selections(text: &mut TerminalText, selections: &[Selection]) {
    if selections.is_empty() {
        return;
    }
    text.push_fixed("Selections (requested time -> actual time):")
        .end_line();
    for selection in selections {
        text.push_fixed("  ")
            .push_value(&selection.role)
            .push_fixed("  ");
        push_clock(text, selection.requested_us);
        text.push_fixed(" -> ");
        push_clock(text, selection.actual_us);
        text.push_fixed(" (");
        if selection.delta_us > 0 {
            text.push_fixed("+");
        }
        text.push_signed(selection.delta_us)
            .push_fixed(" us)  ")
            .push_value(&selection.evidence_id)
            .end_line();
    }
}

/// Says why a partial evidence call stopped early.
fn push_partial_reason(text: &mut TerminalText, reason: Option<&str>) {
    if let Some(reason) = reason {
        text.push_fixed("Stopped early: ")
            .push_value(reason)
            .push_fixed(" (the items below are committed evidence)")
            .end_line();
    }
}

/// Writes delivered paths, and remembers what the notes after them must
/// say.
#[derive(Debug, Default)]
struct PathNotes {
    extended: bool,
    inexact: bool,
}

impl PathNotes {
    /// Writes the file of the item `evidence_id`: its media type, then its
    /// path on a line of its own.
    fn push_files_of(
        &mut self,
        text: &mut TerminalText,
        evidence_id: &str,
        files: &[DeliveredFile],
    ) {
        for file in files.iter().filter(|file| file.evidence_id == evidence_id) {
            self.push_file(text, file);
        }
    }

    /// Writes every file that belongs to no item (a result never has one;
    /// it is shown rather than dropped).
    fn push_unmatched(
        &mut self,
        text: &mut TerminalText,
        files: &[DeliveredFile],
        has_item: impl Fn(&DeliveredFile) -> bool,
    ) {
        for file in files.iter().filter(|file| !has_item(file)) {
            text.push_value(&file.evidence_id).end_line();
            self.push_file(text, file);
        }
    }

    fn push_file(&mut self, text: &mut TerminalText, file: &DeliveredFile) {
        text.push_fixed("  File (")
            .push_value(&file.media_type)
            .push_fixed("):")
            .end_line();
        match text.push_path_line("    ", &file.path) {
            PathLine::Exact => {}
            PathLine::Altered | PathLine::TooLong => self.inexact = true,
        }
        self.extended |= file.path.starts_with(EXTENDED_LENGTH_PREFIX);
    }

    /// Writes each note the paths call for, once.
    fn push_notes(&self, text: &mut TerminalText) {
        if self.extended || self.inexact {
            text.blank_line();
        }
        if self.extended {
            text.push_fixed(EXTENDED_PATH_NOTE).end_line();
        }
        if self.inexact {
            text.push_fixed(INEXACT_PATH_NOTE).end_line();
        }
    }
}
