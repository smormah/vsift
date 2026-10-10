//! What a transcript search could and could not see.
//!
//! A search can only find words that a transcript holds. An agent that is not
//! told which parts of the video were never transcribed would read "no hit" as
//! "not said", so every search reports, for the range it searched, which parts
//! a transcript covers, which it does not, and where local recognition found no
//! speech at all. Search looks at transcript text only; on-screen text is out
//! of its scope.
//!
//! Coverage is derived from the revision's own provenance:
//!
//! - **Supplied transcript:** `VSift` cannot know which parts of the video a
//!   supplied file was meant to cover, so the file is taken to cover the whole
//!   source and the basis says its completeness is not verified.
//! - **Local ASR:** every chunk window of the run was examined. A window whose
//!   audio was transcribed, or found silent, or had no audio at all, is
//!   covered; a part of the video no run examined is not. Silent and audio-less
//!   windows that no transcribed window overlaps are reported as no speech. A
//!   window whose recognised output was unusable
//!   ([`AsrChunkOutcome::Unusable`]) is examined but **not covered**, and is not
//!   no speech either: the recognizer may have been given speech it could not
//!   place, so a word said there cannot be found (#353). The parts of it that a
//!   neighbouring window, which overlaps it by five seconds, transcribed are
//!   covered by that window.
//! - **Spliced revision:** the revision's own run covers the range it replaced;
//!   outside that range, each revision whose segments it carries contributes
//!   its own coverage by the same rules. A run whose segments were all replaced
//!   is no longer recorded in the revision, so what only it examined counts as
//!   not covered: the report can understate coverage, never overstate it. That
//!   holds for an unusable window of the revision's own run too: an earlier
//!   revision's windows, which its replaced segments no longer back, are not
//!   counted over it.
//!
//! No-speech ranges never contain a segment of the revision.

use crate::{
    AsrChunkOutcome, AsrRun, MediaTime, TimeRange, TranscriptProvenance, TranscriptRevision,
};

/// Where the words of a searched revision came from, which says how far its
/// coverage can be trusted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverageBasis {
    /// A supplied `SubRip` or `WebVTT` file. It is taken to cover the whole
    /// source, but `VSift` has not verified that it is complete.
    SuppliedTranscript,
    /// Local speech recognition only; coverage is what the runs examined.
    LocalAsr,
    /// Local speech recognition of part of the source, spliced into a revision
    /// that still carries supplied text elsewhere, whose completeness is not
    /// verified.
    Mixed,
}

impl CoverageBasis {
    /// Every basis, in declaration order.
    pub const ALL: [Self; 3] = [Self::SuppliedTranscript, Self::LocalAsr, Self::Mixed];

    /// Stable identifier written to the coverage `basis` field.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::SuppliedTranscript => "supplied_transcript",
            Self::LocalAsr => "local_asr",
            Self::Mixed => "mixed",
        }
    }
}

/// The coverage of one search: the searched range split into transcribed and
/// untranscribed parts, with the no-speech parts of the transcribed ones.
///
/// Every list is in start order, merged (no two ranges overlap or touch) and
/// clipped to the searched range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchCoverage {
    basis: CoverageBasis,
    searched: Option<TimeRange>,
    transcribed: Vec<TimeRange>,
    untranscribed: Vec<TimeRange>,
    no_speech: Vec<TimeRange>,
}

impl SearchCoverage {
    /// The coverage of `revision` within `window`, or within its whole source
    /// segment without a window.
    ///
    /// The searched range is the window clipped to the source segment; a
    /// window wholly outside the source searches nothing and has no ranges.
    #[must_use]
    pub fn of(revision: &TranscriptRevision, window: Option<TimeRange>) -> Self {
        let source = span(revision.source_segment().range());
        let searched = match window {
            None => Some(source),
            Some(window) => intersection(span(window), source),
        };
        let (basis, transcribed, no_speech) = revision_coverage(revision, source);
        let Some(searched_span) = searched else {
            return Self {
                basis,
                searched: None,
                transcribed: Vec::new(),
                untranscribed: Vec::new(),
                no_speech: Vec::new(),
            };
        };
        let transcribed = clip(&transcribed, searched_span);
        let untranscribed = subtract(&[searched_span], &transcribed);
        let no_speech = clip(&no_speech, searched_span);
        Self {
            basis,
            searched: to_range(searched_span),
            transcribed: to_ranges(&transcribed),
            untranscribed: to_ranges(&untranscribed),
            no_speech: to_ranges(&no_speech),
        }
    }

    /// Where the revision's words came from.
    #[must_use]
    pub const fn basis(&self) -> CoverageBasis {
        self.basis
    }

    /// The part of the source that was searched, `None` when the requested
    /// range lies wholly outside the source.
    #[must_use]
    pub const fn searched(&self) -> Option<TimeRange> {
        self.searched
    }

    /// Parts of the searched range a transcript covers.
    #[must_use]
    pub fn transcribed(&self) -> &[TimeRange] {
        &self.transcribed
    }

    /// Parts of the searched range no transcript covers; a word said there
    /// cannot be found.
    #[must_use]
    pub fn untranscribed(&self) -> &[TimeRange] {
        &self.untranscribed
    }

    /// Transcribed parts where local recognition found no audible signal or
    /// no audio at all.
    #[must_use]
    pub fn no_speech(&self) -> &[TimeRange] {
        &self.no_speech
    }

    /// Whether every part of the searched range is transcribed.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.untranscribed.is_empty()
    }
}

/// A half-open span in microseconds; always `start < end` where built here.
type Span = (u64, u64);

fn span(range: TimeRange) -> Span {
    (range.start().as_micros(), range.end().as_micros())
}

fn to_range((start, end): Span) -> Option<TimeRange> {
    TimeRange::new(MediaTime::from_micros(start), MediaTime::from_micros(end)).ok()
}

fn to_ranges(spans: &[Span]) -> Vec<TimeRange> {
    spans.iter().copied().filter_map(to_range).collect()
}

fn intersection((start, end): Span, (other_start, other_end): Span) -> Option<Span> {
    let (start, end) = (start.max(other_start), end.min(other_end));
    (start < end).then_some((start, end))
}

/// Sorts `spans` and merges every overlapping or touching pair.
fn merged(mut spans: Vec<Span>) -> Vec<Span> {
    spans.retain(|(start, end)| start < end);
    spans.sort_unstable();
    let mut result: Vec<Span> = Vec::with_capacity(spans.len());
    for (start, end) in spans {
        match result.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => result.push((start, end)),
        }
    }
    result
}

/// The merged spans of `spans` minus every span of `removed` (both merged).
fn subtract(spans: &[Span], removed: &[Span]) -> Vec<Span> {
    let mut result = Vec::new();
    for &(start, end) in spans {
        let mut cursor = start;
        for &(removed_start, removed_end) in removed {
            if removed_end <= cursor {
                continue;
            }
            if removed_start >= end {
                break;
            }
            if removed_start > cursor {
                result.push((cursor, removed_start));
            }
            cursor = cursor.max(removed_end);
            if cursor >= end {
                break;
            }
        }
        if cursor < end {
            result.push((cursor, end));
        }
    }
    result
}

fn clip(spans: &[Span], bounds: Span) -> Vec<Span> {
    spans
        .iter()
        .filter_map(|&candidate| intersection(candidate, bounds))
        .collect()
}

/// The windows of `run`'s chunks, split into those it transcribed, those it
/// found silent or without audio, and those whose recognised output was
/// unusable.
struct RunWindows {
    transcribed: Vec<Span>,
    quiet: Vec<Span>,
    unusable: Vec<Span>,
}

impl RunWindows {
    /// Every window the run examined, whatever came of it. An earlier
    /// revision's coverage is never counted inside these: the run replaced
    /// what that revision said there.
    fn examined(&self) -> Vec<Span> {
        merged(
            self.transcribed
                .iter()
                .chain(&self.quiet)
                .chain(&self.unusable)
                .copied()
                .collect(),
        )
    }

    /// The windows the run covers: what it transcribed or found quiet.
    fn covered(&self) -> Vec<Span> {
        merged(
            self.transcribed
                .iter()
                .chain(&self.quiet)
                .copied()
                .collect(),
        )
    }
}

fn run_windows(run: &AsrRun) -> RunWindows {
    let mut windows = RunWindows {
        transcribed: Vec::new(),
        quiet: Vec::new(),
        unusable: Vec::new(),
    };
    for record in run.chunks() {
        let window = span(record.chunk().window());
        match record.outcome() {
            AsrChunkOutcome::Transcribed { .. } => windows.transcribed.push(window),
            AsrChunkOutcome::Silent { .. } | AsrChunkOutcome::NoAudio => {
                windows.quiet.push(window);
            }
            AsrChunkOutcome::Unusable { .. } => windows.unusable.push(window),
        }
    }
    windows.transcribed = merged(windows.transcribed);
    windows.quiet = merged(windows.quiet);
    windows.unusable = merged(windows.unusable);
    windows
}

/// The basis, transcribed spans and no-speech spans of a whole revision,
/// before clipping to a searched range.
fn revision_coverage(
    revision: &TranscriptRevision,
    source: Span,
) -> (CoverageBasis, Vec<Span>, Vec<Span>) {
    let run = match revision.provenance() {
        TranscriptProvenance::Imported { .. } => {
            return (CoverageBasis::SuppliedTranscript, vec![source], Vec::new());
        }
        TranscriptProvenance::LocalAsr(run) => run,
    };
    let own = run_windows(run);
    let mut covered = own.covered();
    // What an earlier revision covers is counted only outside the windows this
    // run examined, an unusable one included: the run replaced what the
    // earlier revision said there, so its windows no longer back anything
    // inside them.
    let own_examined = own.examined();
    let mut heard = own.transcribed;
    let mut quiet = own.quiet;
    let mut supplied = false;
    for inherited in revision.inherited() {
        match inherited.provenance() {
            TranscriptProvenance::Imported { .. } => {
                supplied = true;
                covered.extend(subtract(&[source], &own_examined));
            }
            TranscriptProvenance::LocalAsr(inherited_run) => {
                let earlier = run_windows(inherited_run);
                covered.extend(subtract(&earlier.transcribed, &own_examined));
                covered.extend(subtract(&earlier.quiet, &own_examined));
                heard.extend(subtract(&earlier.transcribed, &own_examined));
                quiet.extend(subtract(&earlier.quiet, &own_examined));
            }
        }
    }
    let segments: Vec<Span> = revision
        .segments()
        .iter()
        .map(|segment| span(segment.range()))
        .collect();
    let speech = merged(heard.into_iter().chain(segments).collect());
    let no_speech = subtract(&merged(quiet), &speech);
    let basis = if supplied {
        CoverageBasis::Mixed
    } else {
        CoverageBasis::LocalAsr
    };
    (basis, merged(covered), no_speech)
}
