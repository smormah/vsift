//! Transcript evidence records and the data of `ingest` and `transcript.get`.
//!
//! A transcript segment is the first published evidence record (ADR 0016
//! decision 5). Each segment carries its own identity, revision, source and
//! source-segment identities, normalized time, alignment and provenance, so a
//! consumer can store or index it without the page it arrived in.
//!
//! Transcript text is untrusted. The domain already rejects control characters
//! other than the line separator, and this module still passes every line
//! through [`sanitize_untrusted_text`], so the rule for placing untrusted text in
//! public output stays in one place.

use serde::Serialize;
use vsift_domain::{
    AlignmentOrigin, MAX_CUE_TEXT_BYTES, SessionId, SourceSegment, TimeRange,
    TranscriptImportError, TranscriptRejection, TranscriptRevision, TranscriptSegment,
    TranscriptWarningKind,
};

use crate::{ConfidenceResponse, sanitize_untrusted_text};

/// Every character sanitization can replace grows from at most one byte to
/// three (U+FFFD), so this budget never truncates a domain-bounded cue.
const MAX_PRESENTED_TEXT_BYTES: usize = MAX_CUE_TEXT_BYTES * 3;

/// Summary of one transcript revision: identity, alignment and import outcome.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranscriptRevisionData {
    revision_id: String,
    revision: u32,
    source_id: String,
    source_segments: Vec<SourceSegmentData>,
    alignment: RevisionAlignmentData,
    sidecar: SidecarData,
    language: Option<String>,
    segment_count: usize,
    warnings: Vec<TranscriptWarningData>,
}

impl TranscriptRevisionData {
    /// Presents a committed revision.
    #[must_use]
    pub fn new(revision: &TranscriptRevision) -> Self {
        Self {
            revision_id: revision.id().as_str().to_owned(),
            revision: revision.number(),
            source_id: revision.source_id().as_str().to_owned(),
            source_segments: vec![SourceSegmentData::new(revision.source_segment())],
            alignment: RevisionAlignmentData {
                origin: revision.origin().identifier(),
                offset_us: revision.offset().as_micros(),
            },
            sidecar: SidecarData {
                format: sidecar_format(revision.origin()),
                sha256: revision.sidecar().sha256().to_owned(),
                bytes: revision.sidecar().bytes(),
            },
            language: revision.language().map(|tag| tag.as_str().to_owned()),
            segment_count: revision.segments().len(),
            warnings: revision
                .warnings()
                .as_slice()
                .iter()
                .map(|warning| TranscriptWarningData {
                    code: warning.kind().identifier(),
                    count: warning.count(),
                    first_cue: warning.first_cue(),
                    excluded_cues: warning.kind().excludes_cues(),
                })
                .collect(),
        }
    }
}

/// One ordered source segment a revision covers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SourceSegmentData {
    source_segment_id: String,
    index: u32,
    start_us: u64,
    end_us: u64,
    state: &'static str,
}

impl SourceSegmentData {
    fn new(segment: &SourceSegment) -> Self {
        Self {
            source_segment_id: segment.id().as_str().to_owned(),
            index: segment.index(),
            start_us: segment.range().start().as_micros(),
            end_us: segment.range().end().as_micros(),
            state: segment.state().identifier(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct RevisionAlignmentData {
    origin: &'static str,
    offset_us: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SidecarData {
    format: &'static str,
    sha256: String,
    bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct TranscriptWarningData {
    code: &'static str,
    count: u32,
    first_cue: u32,
    excluded_cues: bool,
}

/// One timestamped transcript segment: the published transcript evidence record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranscriptSegmentData {
    segment_id: String,
    revision_id: String,
    source_id: String,
    source_segment_id: String,
    start_us: u64,
    end_us: u64,
    text: String,
    original_text: Option<String>,
    markup: &'static str,
    speaker: Option<SpeakerData>,
    confidence: ConfidenceResponse,
    language: Option<String>,
    alignment: SegmentAlignmentData,
    cue: CueData,
}

impl TranscriptSegmentData {
    /// Presents one segment of `revision`.
    #[must_use]
    pub fn new(revision: &TranscriptRevision, segment: &TranscriptSegment) -> Self {
        Self {
            segment_id: segment.id().as_str().to_owned(),
            revision_id: revision.id().as_str().to_owned(),
            source_id: revision.source_id().as_str().to_owned(),
            source_segment_id: revision.source_segment().id().as_str().to_owned(),
            start_us: segment.range().start().as_micros(),
            end_us: segment.range().end().as_micros(),
            text: sanitize_lines(segment.text().text()),
            original_text: segment.text().original().map(sanitize_lines),
            markup: segment.text().markup().identifier(),
            speaker: segment.speaker().map(|label| SpeakerData {
                label: sanitize_untrusted_text(label.as_str(), MAX_PRESENTED_TEXT_BYTES),
                origin: speaker_origin(revision.origin()),
            }),
            confidence: ConfidenceResponse::from(segment.confidence()),
            language: revision.language().map(|tag| tag.as_str().to_owned()),
            alignment: SegmentAlignmentData {
                origin: revision.origin().identifier(),
                offset_us: revision.offset().as_micros(),
                cue_start_us: segment.cue_timing().start_micros(),
                cue_end_us: segment.cue_timing().end_micros(),
            },
            cue: CueData {
                ordinal: segment.cue().ordinal(),
                line: segment.cue().line(),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SpeakerData {
    label: String,
    origin: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SegmentAlignmentData {
    origin: &'static str,
    offset_us: i64,
    cue_start_us: u64,
    cue_end_us: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct CueData {
    ordinal: u32,
    line: u32,
}

/// The requested half-open source range of a page.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct RangeData {
    from_us: u64,
    to_us: u64,
}

impl RangeData {
    pub(crate) const fn new(range: TimeRange) -> Self {
        Self {
            from_us: range.start().as_micros(),
            to_us: range.end().as_micros(),
        }
    }
}

/// Data of a `transcript.get` result: one bounded page of segments.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranscriptPageData {
    session_id: String,
    revision: TranscriptRevisionData,
    range: RangeData,
    items: Vec<TranscriptSegmentData>,
    next_cursor: Option<String>,
}

impl TranscriptPageData {
    /// Presents one page read from `revision`.
    #[must_use]
    pub fn new(
        session_id: &SessionId,
        revision: &TranscriptRevision,
        range: TimeRange,
        segments: &[TranscriptSegment],
        next_cursor: Option<&str>,
    ) -> Self {
        Self {
            session_id: session_id.as_str().to_owned(),
            revision: TranscriptRevisionData::new(revision),
            range: RangeData::new(range),
            items: segments
                .iter()
                .map(|segment| TranscriptSegmentData::new(revision, segment))
                .collect(),
            next_cursor: next_cursor.map(str::to_owned),
        }
    }
}

/// Fixed prose for the envelope `warnings` of an import, one per warning kind.
///
/// The typed warnings, with counts and the first affected cue, are in the
/// revision data; this prose only tells a reader that they exist.
#[must_use]
pub fn transcript_warning_messages(revision: &TranscriptRevision) -> Vec<&'static str> {
    revision
        .warnings()
        .as_slice()
        .iter()
        .map(|warning| match warning.kind() {
            TranscriptWarningKind::EmptyCuesSkipped => {
                "Some supplied transcript cues had no text and were not imported."
            }
            TranscriptWarningKind::MarkupRemoved => {
                "Markup was removed from some transcript cue text; the original text is kept."
            }
            TranscriptWarningKind::SpeakerLabelDiscarded => {
                "Some transcript voice labels were invalid or ambiguous and were not attached."
            }
            TranscriptWarningKind::OverlappingCues => {
                "Some supplied transcript cues overlap in time; each keeps its own timing."
            }
            TranscriptWarningKind::CuesOutsideSource => {
                "Some supplied transcript cues lie outside the source timeline and were not imported."
            }
            TranscriptWarningKind::CuesCrossingSourceBoundary => {
                "Some supplied transcript cues cross the start or end of the source and were not imported."
            }
        })
        .collect()
}

/// Structured remediation for a rejected transcript import.
///
/// The summary is fixed prose chosen by the typed rejection, plus the line
/// number, never text from the file, so it cannot carry untrusted content.
#[must_use]
pub fn transcript_rejection_summary(error: TranscriptImportError) -> String {
    let rejection = error.rejection();
    let advice = match rejection {
        TranscriptRejection::TooLarge => "Supply a transcript of at most 8 MiB.",
        TranscriptRejection::UnsupportedEncoding => "Convert the transcript to UTF-8.",
        TranscriptRejection::InvalidUtf8 => "Convert the transcript to valid UTF-8.",
        TranscriptRejection::ControlCharacter => "Remove control characters from the transcript.",
        TranscriptRejection::LineTooLong => "Split lines longer than 4096 bytes.",
        TranscriptRejection::TooManyCues => "Supply at most 20000 timed cues.",
        TranscriptRejection::CueTextTooLong => "Shorten cue text to at most 4096 bytes.",
        TranscriptRejection::InvalidHeader => {
            "Start a WebVTT file with a valid WEBVTT signature and separate the header from cues with a blank line."
        }
        TranscriptRejection::InvalidCueNumber => "Start each SubRip cue with its cue number.",
        TranscriptRejection::InvalidTimingLine => {
            "Write cue timing as start --> end with nothing after the end time in SubRip."
        }
        TranscriptRejection::InvalidTimestamp => {
            "Correct the timestamp: SubRip uses H:MM:SS,mmm and WebVTT uses [HH:]MM:SS.mmm."
        }
        TranscriptRejection::NonPositiveDuration => {
            "Give every cue an end time after its start time."
        }
        TranscriptRejection::OutOfOrder => "Order cues by start time.",
        TranscriptRejection::UntimedText => {
            "Untimed text cannot support timestamp citations; supply timed SubRip or WebVTT cues."
        }
        TranscriptRejection::NoCues => "Supply at least one timed cue with text.",
        TranscriptRejection::OffsetOutOfRange => {
            "Choose a transcript offset within 24 hours either way."
        }
        TranscriptRejection::NoCuesWithinSource => {
            "No cue lies inside the video after the offset; check the transcript offset and that the transcript belongs to this video."
        }
    };
    match error.line() {
        Some(line) => format!(
            "The supplied transcript was rejected ({}) at line {line}. {advice}",
            rejection.identifier()
        ),
        None => format!(
            "The supplied transcript was rejected ({}). {advice}",
            rejection.identifier()
        ),
    }
}

/// Remediation when `FFmpeg` or `FFprobe` is missing for transcript alignment.
pub const MEDIA_TOOLS_FOR_TRANSCRIPT_REMEDIATION: &str = "Importing a supplied transcript needs FFmpeg and FFprobe to measure the video, but not Whisper or a model. Install or locate trusted builds, register them with setup configure, then run setup check.";

fn sanitize_lines(text: &str) -> String {
    text.split('\n')
        .map(|line| sanitize_untrusted_text(line, MAX_PRESENTED_TEXT_BYTES))
        .collect::<Vec<_>>()
        .join("\n")
}

const fn sidecar_format(origin: AlignmentOrigin) -> &'static str {
    match origin {
        AlignmentOrigin::ImportedSrt => "srt",
        AlignmentOrigin::ImportedWebVtt => "webvtt",
    }
}

const fn speaker_origin(origin: AlignmentOrigin) -> &'static str {
    match origin {
        AlignmentOrigin::ImportedSrt => "imported_srt",
        AlignmentOrigin::ImportedWebVtt => "imported_webvtt_voice",
    }
}
