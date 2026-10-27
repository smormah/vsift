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
//! public output stays in one place. Each segment also carries `display_text`
//! (and a speaker `display_label`): the same text with every hidden character
//! written as `<U+XXXX>` by [`render_hidden_characters`], for a reader to quote
//! (ADR 0008, note of 2026-09-29). It is rendered here, at output, so stored
//! revisions, identities and digests never change.

use serde::Serialize;
use vsift_domain::{
    AlignmentOrigin, AsrChunkOutcome, AsrRun, JobId, MAX_CUE_TEXT_BYTES, ProviderEndTrim,
    SearchCoverage, SegmentOrigin, SessionId, SourceSegment, TimeRange, TranscriptImportError,
    TranscriptOffset, TranscriptProvenance, TranscriptRejection, TranscriptRevision,
    TranscriptSegment, TranscriptWarningKind,
};

use crate::growth::{ReadGrowth, SinceData, TRANSCRIPTION_PENDING_WARNING};
use crate::{
    CommandName, ConfidenceResponse, CoverageResponse, LifecycleResponse, OperationResponse,
    ResultView, render_hidden_characters, sanitize_untrusted_text,
    search::{TranscriptCoverageData, bounded, envelope_coverage},
    view::BRIEF_VIEW,
};

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
    /// `open` or `stopped` (ADR 0033); absent for a sealed revision, so a
    /// sealed revision is presented exactly as before 0.3.0.
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<&'static str>,
    /// For an open revision, the time before which it is final; for a stopped
    /// one, the time its run had published until. Absent when sealed.
    #[serde(skip_serializing_if = "Option::is_none")]
    published_until_us: Option<u64>,
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
                    local_asr: LocalAsrRunData::new(run, revision.unplaced()),
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
            state: match revision.state() {
                vsift_domain::RevisionState::Sealed => None,
                state => Some(state.identifier()),
            },
            published_until_us: revision
                .published_until()
                .map(vsift_domain::MediaTime::as_micros),
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
    /// Present only when not zero, so a revision with no unusable chunk is
    /// presented exactly as it was before the outcome existed (#353).
    #[serde(skip_serializing_if = "Option::is_none")]
    unusable_chunks: Option<usize>,
    /// Chunks an open revision has not published yet (ADR 0033); present only
    /// when not zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_chunks: Option<usize>,
    /// Chunks a stopped run never reached (ADR 0033); present only when not
    /// zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    not_reached_chunks: Option<usize>,
    /// The stretches where the recogniser wrote words that no segment holds
    /// (#359), all of them (a revision records at most 1,024, which is why the
    /// list is not cut as a search's is). Present only when there is any, so a
    /// revision with none is presented exactly as before. A search of the
    /// revision lists these stretches as untranscribed, unless a segment covers
    /// them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    unplaced_ranges: Vec<RangeData>,
}

impl LocalAsrRunData {
    fn new(run: &AsrRun, unplaced: &[TimeRange]) -> Self {
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
            unusable_chunks: Some(count(|outcome| {
                matches!(outcome, AsrChunkOutcome::Unusable { .. })
            }))
            .filter(|unusable| *unusable > 0),
            pending_chunks: Some(count(|outcome| matches!(outcome, AsrChunkOutcome::Pending)))
                .filter(|pending| *pending > 0),
            not_reached_chunks: Some(count(|outcome| {
                matches!(outcome, AsrChunkOutcome::NotReached)
            }))
            .filter(|not_reached| *not_reached > 0),
            unplaced_ranges: unplaced.iter().copied().map(RangeData::new).collect(),
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
///
/// `text` and `original_text` are the payload (without markup, and as
/// written); `display_text` is `text` with every hidden character made
/// visible, the form a report quotes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranscriptSegmentData {
    segment_id: String,
    revision_id: String,
    source_id: String,
    source_segment_id: String,
    start_us: u64,
    end_us: u64,
    text: String,
    display_text: String,
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
        let text = sanitize_lines(segment.text().text());
        Self {
            segment_id: segment.id().as_str().to_owned(),
            revision_id: revision.id().as_str().to_owned(),
            source_id: revision.source_id().as_str().to_owned(),
            source_segment_id: revision.source_segment().id().as_str().to_owned(),
            start_us: segment.range().start().as_micros(),
            end_us: segment.range().end().as_micros(),
            display_text: render_hidden_characters(&text),
            text,
            original_text: segment.text().original().map(sanitize_lines),
            markup: segment.text().markup().identifier(),
            speaker: segment.speaker().map(|label| {
                let label = sanitize_untrusted_text(label.as_str(), MAX_PRESENTED_TEXT_BYTES);
                SpeakerData {
                    display_label: render_hidden_characters(&label),
                    label,
                    origin: speaker_origin(origin),
                }
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

/// One transcript segment in the brief view (`--view brief`, #365): what a
/// reader needs to cite and place it, and nothing the answer states elsewhere.
///
/// It is the identity, the time, the text to quote (`display_text`, with the
/// hidden characters visible), the speaker's `display_label` with its origin
/// when there is one, and, **on a segment carried into a spliced revision,
/// `carried_from`**, which names the revision and segment its text was first
/// produced in. That one is kept because the revision block describes the
/// revision's own run, and a carried segment's recognizer, alignment and
/// language are those of the revision that produced it: without it the brief
/// view would lose where the text came from. It leaves out, because the
/// revision block of the same answer holds them once, `revision_id`,
/// `source_id` and `source_segment_id`; and, because a citation does not need
/// them, `text` and `original_text` (the full view keeps those raw), `markup`,
/// `confidence`, `language`, and the alignment and cue details. A member is the
/// full item's member with the same value: the brief item is projected from the
/// full one, never built apart from it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct BriefTranscriptSegmentData {
    segment_id: String,
    start_us: u64,
    end_us: u64,
    display_text: String,
    /// Absent, not null, when the segment has no speaker label: the brief view
    /// is the one that spends no characters on nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    speaker: Option<BriefSpeakerData>,
    /// Present only on a carried segment, as in the full item.
    #[serde(skip_serializing_if = "Option::is_none")]
    carried_from: Option<CarriedFromData>,
}

/// A speaker without its raw label: the label to quote and where it came from.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct BriefSpeakerData {
    display_label: String,
    origin: &'static str,
}

impl From<&TranscriptSegmentData> for BriefTranscriptSegmentData {
    fn from(full: &TranscriptSegmentData) -> Self {
        Self {
            segment_id: full.segment_id.clone(),
            start_us: full.start_us,
            end_us: full.end_us,
            display_text: full.display_text.clone(),
            speaker: full.speaker.as_ref().map(|speaker| BriefSpeakerData {
                display_label: speaker.display_label.clone(),
                origin: speaker.origin,
            }),
            carried_from: full.carried_from.clone(),
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
    display_label: String,
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
    /// The parts of the run's own range the recognizer's answers left without a
    /// transcript (#353), bounded like a search's. Absent when there are none,
    /// so a run without a gap is presented exactly as before.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    untranscribed_ranges: Vec<RangeData>,
    job: RetranscribeJobData,
}

/// What the recoverable job behind a retranscription did (P10, ADR 0020).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetranscribeJob {
    /// The job, derived from the request, so a retry finds it.
    pub job_id: JobId,
    /// Whether an interrupted run of the job was continued.
    pub resumed: bool,
    /// Chunks taken from the interrupted run's checkpoints.
    pub chunks_reused: u32,
    /// Whether the revision is an earlier commit returned again, without a
    /// new one (a retry with the same operation id).
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct RetranscribeJobData {
    job_id: String,
    resumed: bool,
    chunks_reused: u32,
    replayed: bool,
}

impl TranscriptRetranscribeData {
    /// Presents a committed retranscription. `requested` is the range the
    /// caller asked for, or `None` for the whole source; `job` is what the
    /// job behind it did.
    #[must_use]
    pub fn new(
        session_id: &SessionId,
        requested: Option<TimeRange>,
        revision: &TranscriptRevision,
        job: &RetranscribeJob,
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
            untranscribed_ranges: bounded(&retranscription_gaps(revision)),
            job: RetranscribeJobData {
                job_id: job.job_id.as_str().to_owned(),
                resumed: job.resumed,
                chunks_reused: job.chunks_reused,
                replayed: job.replayed,
            },
        }
    }
}

/// The parts of a retranscription's own range that this run did not
/// transcribe, in start order and merged: the windows whose recognised output
/// was unusable ([`AsrChunkOutcome::Unusable`]) less what a neighbouring
/// window, which overlaps it by five seconds, transcribed or found silent
/// ([`AsrRun::unusable_gaps`]).
///
/// This is a statement about the run, not about what the session can now
/// find: text the superseded revision had wholly inside such a part is kept as
/// it was, so a search of the new revision may still cover some of it
/// ([`SearchCoverage`](vsift_domain::SearchCoverage) counts that text), which
/// is why the two lists can differ. Listing the whole part here is the true
/// answer to "what did this run not re-transcribe". An imported revision, or a
/// run with no unusable chunk, has none.
#[must_use]
pub fn retranscription_gaps(revision: &TranscriptRevision) -> Vec<TimeRange> {
    match revision.provenance() {
        TranscriptProvenance::LocalAsr(run) => run.unusable_gaps(),
        TranscriptProvenance::Imported { .. } => Vec::new(),
    }
}

/// Completes a retranscription's response: with no unusable window it is the
/// complete result it always was, unchanged; with one it is `partial`, a
/// success that exits 0, and its envelope `coverage` lists the parts this run
/// did not transcribe as `<from_us>-<to_us>` with the reason
/// `untranscribed_range` (#353).
///
/// The reason is deliberately the published one and not a new value: the run
/// left the range untranscribed, which is what `untranscribed_range` says, and
/// a reader that already acts on it needs to learn nothing. What is specific to
/// this cause is in `data` (`untranscribed_ranges`, `local_asr.unusable_chunks`)
/// and in the warning `provider_chunks_rejected`, which also says that earlier
/// text inside those parts was kept.
#[must_use]
pub fn finish_retranscription(
    response: OperationResponse<serde_json::Value>,
    revision: &TranscriptRevision,
) -> OperationResponse<serde_json::Value> {
    match retranscription_coverage(&retranscription_gaps(revision)) {
        Some(coverage) => response.with_coverage(coverage),
        None => response,
    }
}

/// The coverage a retranscription states for the parts its run did not
/// transcribe (`gaps`, from [`retranscription_gaps`]): `None` when there are
/// none, else truncated, with the gaps as `<from_us>-<to_us>` and the reason
/// `untranscribed_range`.
///
/// It has one home so that `transcript retranscribe`, `job resume` and a
/// worker's `retranscribe` step state the same gaps in the same words (#362):
/// the direct commands put it in the envelope, a worker step in its own
/// `coverage`.
pub(crate) fn retranscription_coverage(gaps: &[TimeRange]) -> Option<CoverageResponse> {
    (!gaps.is_empty()).then(|| envelope_coverage(gaps))
}

/// Data of a `transcript.get` result: one bounded page of segments.
///
/// `transcript_coverage` (0.2.1, #373) says which parts of the requested range
/// a transcript covers, by the rule `search` uses ([`SearchCoverage::of`], run
/// by the application's transcript read and handed to this crate in the
/// [`TranscriptPresentation`], as for a search): a range that holds no segment
/// is read as "nothing was said" only where the transcript covers it. Every
/// page of one request states the coverage of the whole requested range, not of
/// the page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranscriptPageData {
    session_id: String,
    revision: TranscriptRevisionData,
    range: RangeData,
    items: Vec<TranscriptSegmentData>,
    next_cursor: Option<String>,
    transcript_coverage: TranscriptCoverageData,
    /// What to read on from (ADR 0033): present for a `--since` read, and over
    /// an open revision on the page that ends the read (no `next_cursor`).
    #[serde(skip_serializing_if = "Option::is_none")]
    since: Option<SinceData>,
}

impl TranscriptPageData {
    /// Presents one page read from a revision.
    #[must_use]
    pub fn new(page: &TranscriptPresentation<'_>) -> Self {
        Self {
            session_id: page.session_id.as_str().to_owned(),
            revision: TranscriptRevisionData::new(page.revision),
            range: RangeData::new(page.range),
            items: page
                .segments
                .iter()
                .map(|segment| TranscriptSegmentData::new(page.revision, segment))
                .collect(),
            next_cursor: page.next_cursor.map(str::to_owned),
            transcript_coverage: TranscriptCoverageData::with_growth(page.coverage, page.growth),
            since: page.growth.since.map(SinceData::new),
        }
    }
}

/// Data of a `transcript.get` result in the brief view (`--view brief`, #365):
/// [`TranscriptPageData`] with each item reduced to what a reader cites and
/// places (`BriefTranscriptSegmentData`), and a `view` member saying so.
///
/// The revision block (which holds what the full view repeats on every item),
/// the range, the cursor and the coverage are the full page's own, made by the
/// same code: a brief page states its gaps exactly as a full one does.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranscriptBriefPageData {
    session_id: String,
    view: &'static str,
    revision: TranscriptRevisionData,
    range: RangeData,
    items: Vec<BriefTranscriptSegmentData>,
    next_cursor: Option<String>,
    transcript_coverage: TranscriptCoverageData,
    #[serde(skip_serializing_if = "Option::is_none")]
    since: Option<SinceData>,
}

impl TranscriptBriefPageData {
    /// Presents one page read from a revision in the brief view.
    #[must_use]
    pub fn new(page: &TranscriptPresentation<'_>) -> Self {
        Self::from(&TranscriptPageData::new(page))
    }
}

impl From<&TranscriptPageData> for TranscriptBriefPageData {
    /// Projects the full page, so a brief member can only be a full member.
    fn from(full: &TranscriptPageData) -> Self {
        Self {
            session_id: full.session_id.clone(),
            view: BRIEF_VIEW,
            revision: full.revision.clone(),
            range: full.range,
            items: full
                .items
                .iter()
                .map(BriefTranscriptSegmentData::from)
                .collect(),
            next_cursor: full.next_cursor.clone(),
            transcript_coverage: full.transcript_coverage.clone(),
            since: full.since.clone(),
        }
    }
}

/// Everything a host presents about one page of a transcript read.
///
/// The fields borrow the engine's typed result (`Engine::transcript`), so every
/// host presents the same page identically, and the coverage of the range comes
/// from the same place as the segments: the application's transcript read, not
/// this crate. [`transcript_get_response`] makes a `partial` answer of any gap the
/// coverage names, and `Engine::transcript` gives it the coverage of the range it
/// read, so that route cannot say `complete` over a gap **when the coverage is the
/// range's own**. Nothing here checks that: the type holds a
/// [`SearchCoverage`] and a range separately, so a caller that builds the data
/// and an `OperationResponse::complete` itself, or passes another range's
/// coverage, can still say `complete` over a gap. A cross-check of
/// `coverage.searched()` against `range` was left out, because the contract has
/// no error to answer a mismatch with and a new one would change the signature
/// (a search's presentation trusts its coverage in the same way).
#[derive(Clone, Debug)]
pub struct TranscriptPresentation<'a> {
    /// Session that was read.
    pub session_id: &'a SessionId,
    /// The revision that was read.
    pub revision: &'a TranscriptRevision,
    /// The requested half-open range.
    pub range: TimeRange,
    /// The page's segments, in start order.
    pub segments: &'a [TranscriptSegment],
    /// Token for the next page, absent on the last page.
    pub next_cursor: Option<&'a str>,
    /// What the revision's transcript covers of the requested range: the whole
    /// range, not the page.
    pub coverage: &'a SearchCoverage,
    /// What a read of a growing transcript adds (ADR 0033); [`Default`] for a
    /// sealed revision read without `--since`.
    pub growth: ReadGrowth<'a>,
}

/// Fixed warning of a transcript read whose range is not fully transcribed
/// (#373): says what a gap is not (a silence), in the words of a read, and that
/// the rest of the answer is usable.
///
/// It is written to be true for every cause of a gap, and so not to tell a
/// reader to start a long job: a range nobody transcribed, a stretch the
/// recogniser could not read, words that no segment holds, and a false gap left
/// where a retranscription followed a first run with few or no segments (L-033) are
/// all gaps, and retranscribing just the range may fill any of them but may
/// also give the same stretch back (L-159), in which case only a transcript
/// file supplied with `ingest --transcript` fills it. It names no member of the
/// JSON, because the readable text prints it too. `search` has its own, older
/// warning (published, unchanged).
pub const UNTRANSCRIBED_READ_WARNING: &str = "Part of the requested range has no transcript here, so reading it shows no words there; that does not mean none were said. The parts are listed as not transcribed, and the rest of this answer is usable. If you need what was said there, transcribing just that range again (transcript retranscribe with --from and --to) may fill it; if it gives the same stretch, a transcript file you already have is the sure way: open the video with ingest --transcript.";

/// The complete `transcript.get` result for one page in `view`.
///
/// It is the complete result it always was, with the lifecycle and
/// `transcript_coverage` added, unless part of the requested range has no
/// transcript: then it is `partial`, a success that exits 0, its envelope
/// `coverage` lists the parts as `<from_us>-<to_us>` under the reason
/// `untranscribed_range` (the words and shape of a `search` over the same
/// range, because the one rule decides both) and a fixed warning says what a
/// gap is not. A range every part of which is covered keeps `coverage: null`,
/// as it always had.
///
/// # Errors
///
/// Returns the serialization error when the data cannot be represented as JSON.
pub fn transcript_get_response(
    page: &TranscriptPresentation<'_>,
    lifecycle: LifecycleResponse,
    view: ResultView,
) -> Result<OperationResponse<serde_json::Value>, serde_json::Error> {
    let command = CommandName::TranscriptGet.identifier();
    let response = match view {
        ResultView::Full => OperationResponse::complete(command, &TranscriptPageData::new(page))?,
        ResultView::Brief => {
            OperationResponse::complete(command, &TranscriptBriefPageData::new(page))?
        }
    };
    Ok(finish_transcript_read(
        response.with_lifecycle(lifecycle),
        page.coverage,
    ))
}

/// Completes a `transcript.get` response whose requested range is not fully
/// transcribed with its `partial` status, envelope coverage and warning, and
/// returns every other response unchanged (#373).
///
/// `coverage` is [`SearchCoverage::of`] for the revision and range that were
/// read, which is what `search` reports for the same revision and range: one
/// rule for both commands. A read has no use for a reason beyond the
/// published `untranscribed_range`, so that is the one it gives.
#[must_use]
pub(crate) fn finish_transcript_read(
    response: OperationResponse<serde_json::Value>,
    coverage: &SearchCoverage,
) -> OperationResponse<serde_json::Value> {
    if coverage.is_complete() {
        return response;
    }
    if coverage.pending().is_empty() {
        return response
            .with_coverage(envelope_coverage(coverage.untranscribed()))
            .with_warnings(&[UNTRANSCRIBED_READ_WARNING]);
    }
    // An open revision (ADR 0033): the read's own warning applies to what is
    // not pending, and the pending part has its reason and warning.
    let pending_only = coverage.untranscribed() == coverage.pending();
    let response = response.with_coverage(crate::search::envelope_coverage_pending(
        coverage.untranscribed(),
    ));
    if pending_only {
        response.with_warnings(&[TRANSCRIPTION_PENDING_WARNING])
    } else {
        response.with_warnings(&[UNTRANSCRIBED_READ_WARNING, TRANSCRIPTION_PENDING_WARNING])
    }
}

/// Fixed prose of the warning `provider_chunks_rejected`: some audio was given
/// to the recognizer and what it answered could not be used, so this run did
/// not transcribe it. It says what that is not (silence), exactly what
/// happened to text an earlier revision had there (the rule of
/// `EarlierTextRule`: text that reaches into those parts is kept whole as it
/// was, unless this run's own text overlaps it), and what can cover the rest,
/// which the typed warning cannot: a retranscription of just that range cuts
/// the audio at other points (a chunk's result depends on where it is cut),
/// and a transcript file the user has needs no recognition. It claims neither
/// that such a file exists nor that it covers the range.
pub const PROVIDER_CHUNKS_REJECTED_WARNING: &str = "For some audio the recogniser's answer could not be used, so this run did not transcribe it: the result lists those time ranges as untranscribed. It is not silence. Earlier text that reaches into them is kept whole as it was, unless this run's own text overlaps it; any other words said there cannot be found. Transcribing just that range again (transcript retranscribe with --from and --to) cuts the audio at other points and may cover it, and a transcript file you already have can supply it: open the video with ingest --transcript.";

/// Fixed prose of the warning `provider_segments_rejected` when some of the
/// segments it counts were words that no kept segment holds (#359): the
/// revision lists where they may lie (`local_asr.unplaced_ranges`), which
/// the count cannot. It says what a search does with those stretches (they are
/// untranscribed, so a word said there cannot be found, which is the only way
/// a reader can act on it), that the stretch is an upper bound (the words took
/// part of it), and what can cover it: the same two things as for a chunk whose
/// answer could not be used. It does not say the run is incomplete, because the
/// run's status is not changed by it.
pub const PROVIDER_SEGMENTS_UNPLACED_WARNING: &str = "Some recognised segments had times outside their audio and were not used, and for some of them the recogniser wrote words that no kept segment holds. The revision lists the stretches where those words may lie (local_asr.unplaced_ranges, an upper bound: the words took part of each), and a search counts them as untranscribed, so a word said there cannot be found. Transcribing just that range again (transcript retranscribe with --from and --to) cuts the audio at other points and may cover it, and a transcript file you already have can supply it: open the video with ingest --transcript.";

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
                if revision.unplaced().is_empty() {
                    "Some recognised segments had times outside their audio and were not used."
                } else {
                    PROVIDER_SEGMENTS_UNPLACED_WARNING
                }
            }
            TranscriptWarningKind::ProviderEndTrimmed => {
                "Some recognised segments ended after their audio and were cut at its end; a cut end is the audio's end, not evidence that speech continued there."
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
            TranscriptWarningKind::ProviderChunksRejected => PROVIDER_CHUNKS_REJECTED_WARNING,
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
pub const MEDIA_TOOLS_FOR_TRANSCRIPT_REMEDIATION: &str = "Importing a supplied transcript needs FFmpeg and FFprobe to measure the video, but not Whisper or a model. Install trusted builds or register working ones with setup configure ffmpeg|ffprobe --executable <path>, then run setup check. No need to search folders: ask the user where each one is.";

/// Remediation when a session has no transcript to read.
pub const NO_TRANSCRIPT_REMEDIATION: &str = "This session has no transcript yet. Open the video again with ingest --transcript to import a SubRip or WebVTT file, or run transcript retranscribe with this session to transcribe its speech locally with whisper.cpp.";

/// Remediation when a session has no transcript to read yet, but a
/// transcription of it is running (#517, ADR 0033).
///
/// The failure code stays the one of [`NO_TRANSCRIPT_REMEDIATION`]. Starting
/// another transcription would be refused as `BUSY` while this one runs, so the
/// remediation names the job (in `affected_ids`, after the session) and its
/// `job status` command instead. Fixed prose; it names no path, session, job
/// or time.
pub const TRANSCRIPT_RUNNING_REMEDIATION: &str = "This session has no transcript to read yet, but a transcription of it is running (the job in affected_ids). Do not start another one. Run the suggested job status command to see its progress, and read again as its parts are published or when it has finished.";

/// Remediation when a session has no transcript to read yet, but a
/// transcription of it is recorded and waiting for its turn (#517, ADR 0033).
///
/// Fixed prose; it names no path, session, job or time.
pub const TRANSCRIPT_QUEUED_REMEDIATION: &str = "This session has no transcript to read yet, but a transcription of it is waiting for its turn (the job in affected_ids). Do not start another one. Run the suggested job status command to see where it is, and read again when it has finished.";

/// Remediation when a session has no transcript to read yet, but a
/// transcription of it was interrupted and can be continued (#517, ADR 0033).
///
/// The job keeps the chunks it finished, so `job resume` continues it. Fixed
/// prose; it names no path, session, job or time.
pub const TRANSCRIPT_INTERRUPTED_REMEDIATION: &str = "This session has no transcript to read yet: a transcription of it (the job in affected_ids) was interrupted and keeps the chunks it finished. Do not start another one. Run the suggested job resume command to continue it, then read again.";

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
