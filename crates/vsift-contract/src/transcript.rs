//! Transcript evidence records and the data of `ingest`, `transcript.get` and
//! `transcript.retranscribe`.
//!
//! A transcript segment is the first published evidence record (ADR 0016
//! decision 5). Each segment carries its own identity, revision, source and
//! source-segment identities, normalized time, alignment and provenance, so a
//! consumer can store or index it without the page it arrived in.
//!
//! Imported revisions serialize exactly as before local ASR existed. Local-ASR
//! revisions and segments add only their own fields, and a revision spliced
//! from an earlier one (ADR 0017) presents each carried segment with the
//! provenance of the revision that first produced it.
//!
//! Transcript text is untrusted. The domain already rejects control characters
//! other than the line separator, and this module still passes every line
//! through [`sanitize_untrusted_text`], so the rule for placing untrusted text in
//! public output stays in one place.

use serde::Serialize;
use vsift_domain::{
    AlignmentOrigin, AsrChunkOutcome, AsrRun, MAX_CUE_TEXT_BYTES, ProviderEndTrim, SegmentOrigin,
    SessionId, SourceSegment, TimeRange, TranscriptImportError, TranscriptOffset,
    TranscriptProvenance, TranscriptRejection, TranscriptRevision, TranscriptSegment,
    TranscriptWarningKind,
};

use crate::{ConfidenceResponse, sanitize_untrusted_text};

/// Every character sanitization can replace grows from at most one byte to
/// three (U+FFFD), so this budget never truncates a domain-bounded cue.
const MAX_PRESENTED_TEXT_BYTES: usize = MAX_CUE_TEXT_BYTES * 3;

/// Summary of one transcript revision: identity, alignment and outcome.
///
/// An imported revision presents its offset and `sidecar`, exactly as before
/// local ASR existed. A local-ASR revision presents `sidecar: null` and adds
/// `local_asr` (what ran, how, over which range and with which chunk
/// outcomes), `supersedes`, `replaced_range` and `carried_segment_count`;
/// those keys never appear on an import.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranscriptRevisionData {
    revision_id: String,
    revision: u32,
    source_id: String,
    source_segments: Vec<SourceSegmentData>,
    alignment: RevisionAlignmentData,
    sidecar: Option<SidecarData>,
    language: Option<String>,
    segment_count: usize,
    warnings: Vec<TranscriptWarningData>,
    #[serde(flatten)]
    local_asr: Option<LocalAsrRevisionData>,
}

/// The fields only a local-ASR revision has, flattened into its summary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct LocalAsrRevisionData {
    local_asr: LocalAsrRunData,
    supersedes: Option<String>,
    replaced_range: Option<RangeData>,
    carried_segment_count: usize,
}

impl TranscriptRevisionData {
    /// Presents a committed revision.
    #[must_use]
    pub fn new(revision: &TranscriptRevision) -> Self {
        let (offset, sidecar, local_asr) = match revision.provenance() {
            TranscriptProvenance::Imported {
                format,
                sidecar,
                offset,
            } => (
                Some(offset.as_micros()),
                Some(SidecarData {
                    format: format.identifier(),
                    sha256: sidecar.sha256().to_owned(),
                    bytes: sidecar.bytes(),
                }),
                None,
            ),
            TranscriptProvenance::LocalAsr(run) => (
                None,
                None,
                Some(LocalAsrRevisionData {
                    local_asr: LocalAsrRunData::new(run),
                    supersedes: revision.supersedes().map(|id| id.as_str().to_owned()),
                    replaced_range: revision.replaced_range().map(RangeData::new),
                    carried_segment_count: revision
                        .segments()
                        .iter()
                        .filter(|segment| segment.carried_from().is_some())
                        .count(),
                }),
            ),
        };
        Self {
            revision_id: revision.id().as_str().to_owned(),
            revision: revision.number(),
            source_id: revision.source_id().as_str().to_owned(),
            source_segments: vec![SourceSegmentData::new(revision.source_segment())],
            alignment: RevisionAlignmentData {
                origin: revision.origin().identifier(),
                offset_us: offset,
            },
            sidecar,
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
            local_asr,
        }
    }
}

/// What a local-ASR run was: provider build, model, settings, the range it
/// covered and how many chunks it transcribed, found silent or found empty.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct LocalAsrRunData {
    provider: &'static str,
    executable_sha256: String,
    model_profile: &'static str,
    model_sha256: String,
    decoding_profile: &'static str,
    window_us: u64,
    overlap_us: u64,
    threads: u16,
    audio_stream: u32,
    covered_range: Option<RangeData>,
    chunk_count: usize,
    transcribed_chunks: usize,
    silent_chunks: usize,
    no_audio_chunks: usize,
}

impl LocalAsrRunData {
    fn new(run: &AsrRun) -> Self {
        let count = |wanted: fn(&AsrChunkOutcome) -> bool| {
            run.chunks()
                .iter()
                .filter(|record| wanted(&record.outcome()))
                .count()
        };
        Self {
            provider: run.provider().provider().identifier(),
            executable_sha256: run.provider().executable_sha256().as_str().to_owned(),
            model_profile: run.model().profile().identifier(),
            model_sha256: run.model().sha256().as_str().to_owned(),
            decoding_profile: run.decoding().identifier(),
            window_us: run.plan().window_us(),
            overlap_us: run.plan().overlap_us(),
            threads: run.threads().get(),
            audio_stream: run.audio_stream(),
            covered_range: run.covered_range().map(RangeData::new),
            chunk_count: run.chunks().len(),
            transcribed_chunks: count(|outcome| {
                matches!(outcome, AsrChunkOutcome::Transcribed { .. })
            }),
            silent_chunks: count(|outcome| matches!(outcome, AsrChunkOutcome::Silent { .. })),
            no_audio_chunks: count(|outcome| matches!(outcome, AsrChunkOutcome::NoAudio)),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    offset_us: Option<i64>,
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
    cue: Option<CueData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    carried_from: Option<CarriedFromData>,
}

impl TranscriptSegmentData {
    /// Presents one segment of `revision`.
    ///
    /// The alignment and language are those of the provenance that produced
    /// the segment: for a segment carried into a spliced revision, the
    /// revision it was first produced in. An imported segment presents its
    /// offset, cue timing and `cue` as before; a local-ASR segment presents its
    /// chunk, the provider's chunk-relative times and the recognizer, and
    /// `cue: null`. `carried_from` appears only on carried segments.
    #[must_use]
    pub fn new(revision: &TranscriptRevision, segment: &TranscriptSegment) -> Self {
        let provenance = revision.segment_provenance(segment);
        let origin = provenance.alignment_origin();
        let (alignment, cue) = match (segment.origin(), provenance) {
            (
                SegmentOrigin::ImportedCue { cue, timing },
                TranscriptProvenance::Imported { offset, .. },
            ) => (
                SegmentAlignmentData {
                    origin: origin.identifier(),
                    offset_us: Some(TranscriptOffset::as_micros(*offset)),
                    cue_start_us: Some(timing.start_micros()),
                    cue_end_us: Some(timing.end_micros()),
                    local_asr: None,
                },
                Some(CueData {
                    ordinal: cue.ordinal(),
                    line: cue.line(),
                }),
            ),
            (
                SegmentOrigin::Asr {
                    chunk,
                    provider_start,
                    provider_end,
                    trimmed,
                },
                TranscriptProvenance::LocalAsr(run),
            ) => (
                SegmentAlignmentData {
                    origin: origin.identifier(),
                    offset_us: None,
                    cue_start_us: None,
                    cue_end_us: None,
                    local_asr: AsrAlignmentData::new(
                        run,
                        chunk,
                        provider_start.as_micros(),
                        provider_end.as_micros(),
                        trimmed,
                    ),
                },
                None,
            ),
            // The domain rejects a segment whose origin does not match its
            // provenance; this arm exists only because the match is exhaustive.
            _ => (
                SegmentAlignmentData {
                    origin: origin.identifier(),
                    offset_us: None,
                    cue_start_us: None,
                    cue_end_us: None,
                    local_asr: None,
                },
                None,
            ),
        };
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
                origin: speaker_origin(origin),
            }),
            confidence: ConfidenceResponse::from(segment.confidence()),
            language: revision
                .segment_language(segment)
                .map(|tag| tag.as_str().to_owned()),
            alignment,
            cue,
            carried_from: segment.carried_from().map(|carried| CarriedFromData {
                revision_id: carried.revision().as_str().to_owned(),
                segment_id: carried.segment().as_str().to_owned(),
            }),
        }
    }
}

/// The revision and segment a carried segment's text was first produced in.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct CarriedFromData {
    revision_id: String,
    segment_id: String,
}

/// Where a local-ASR segment came from: its chunk's decoded audio, the
/// provider's times relative to it, and the recognizer that heard it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct AsrAlignmentData {
    chunk_index: u32,
    chunk_audio_start_us: u64,
    chunk_audio_end_us: u64,
    provider_start_us: u64,
    provider_end_us: u64,
    end_trimmed: bool,
    provider: &'static str,
    executable_sha256: String,
    model_profile: &'static str,
    model_sha256: String,
    decoding_profile: &'static str,
}

impl AsrAlignmentData {
    fn new(
        run: &AsrRun,
        chunk: u32,
        provider_start_us: u64,
        provider_end_us: u64,
        trimmed: ProviderEndTrim,
    ) -> Option<Self> {
        let record = run.chunks().get(usize::try_from(chunk).ok()?)?;
        // The domain guarantees an ASR segment names a transcribed chunk.
        let AsrChunkOutcome::Transcribed { audio } = record.outcome() else {
            return None;
        };
        Some(Self {
            chunk_index: chunk,
            chunk_audio_start_us: audio.start().as_micros(),
            chunk_audio_end_us: audio.end().as_micros(),
            provider_start_us,
            provider_end_us,
            end_trimmed: trimmed == ProviderEndTrim::TrimmedToAudioEnd,
            provider: run.provider().provider().identifier(),
            executable_sha256: run.provider().executable_sha256().as_str().to_owned(),
            model_profile: run.model().profile().identifier(),
            model_sha256: run.model().sha256().as_str().to_owned(),
            decoding_profile: run.decoding().identifier(),
        })
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
    #[serde(skip_serializing_if = "Option::is_none")]
    offset_us: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cue_start_us: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cue_end_us: Option<u64>,
    #[serde(flatten)]
    local_asr: Option<AsrAlignmentData>,
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

/// Data of a `transcript.retranscribe` result: the new revision.
///
/// The revision's segments are not repeated here; read them with
/// `transcript get --revision <revision_id>`, as a page or an evidence stream.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranscriptRetranscribeData {
    session_id: String,
    requested_range: Option<RangeData>,
    revision: TranscriptRevisionData,
    recognised_segment_count: usize,
}

impl TranscriptRetranscribeData {
    /// Presents a committed retranscription. `requested` is the range the
    /// caller asked for, or `None` for the whole source.
    #[must_use]
    pub fn new(
        session_id: &SessionId,
        requested: Option<TimeRange>,
        revision: &TranscriptRevision,
    ) -> Self {
        Self {
            session_id: session_id.as_str().to_owned(),
            requested_range: requested.map(RangeData::new),
            revision: TranscriptRevisionData::new(revision),
            recognised_segment_count: revision
                .segments()
                .iter()
                .filter(|segment| segment.carried_from().is_none())
                .count(),
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
            TranscriptWarningKind::ProviderSegmentsRejected => {
                "Some recognised segments had times outside their audio and were not used."
            }
            TranscriptWarningKind::ProviderEndTrimmed => {
                "Some recognised segments ended just after their audio and were cut at its end."
            }
            TranscriptWarningKind::NonSpeechMarkersRemoved => {
                "Non-speech markers such as [BLANK_AUDIO] were removed from the transcript."
            }
            TranscriptWarningKind::SeamDuplicatesRemoved => {
                "Text recognised twice where audio chunks overlap was kept once."
            }
            TranscriptWarningKind::SilentChunksSkipped => {
                "Some audio had no audible signal and was not transcribed."
            }
            TranscriptWarningKind::NoSpeechRecognised => {
                "No speech was recognised in the transcribed range; the revision records the attempt without new segments there."
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

/// Remediation when a session has no transcript to read.
pub const NO_TRANSCRIPT_REMEDIATION: &str = "This session has no transcript yet. Open the video again with ingest --transcript to import a SubRip or WebVTT file, or run transcript retranscribe with this session to transcribe its speech locally with whisper.cpp.";

fn sanitize_lines(text: &str) -> String {
    text.split('\n')
        .map(|line| sanitize_untrusted_text(line, MAX_PRESENTED_TEXT_BYTES))
        .collect::<Vec<_>>()
        .join("\n")
}

const fn speaker_origin(origin: AlignmentOrigin) -> &'static str {
    match origin {
        AlignmentOrigin::ImportedSrt => "imported_srt",
        AlignmentOrigin::ImportedWebVtt => "imported_webvtt_voice",
        // The domain rejects speaker labels on local-ASR segments; this arm
        // exists only because the match is exhaustive.
        AlignmentOrigin::LocalAsr => "local_asr",
    }
}
