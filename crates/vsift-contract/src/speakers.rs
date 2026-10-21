//! The data of speaker labels (ADR 0031): the `speakers` block of `transcript get`,
//! `search`, `transcript speakers` and `transcript name-speaker`, and the per-segment
//! `speaker_label`.
//!
//! Every presentation says what a label is and is not, because an agent quotes it:
//! `S2, by voice` groups segments the engine heard in one voice; it is not who the
//! person is, two similar voices can share one label, and a segment in which two
//! people speak is not split (the labels have segment precision). A name appears only
//! as the user's own statement (`verified_name`, "named by the user"). This block is
//! distinct from the segment's `speaker` member, which is a voice tag of a supplied
//! `WebVTT` file.

use serde::Serialize;
use vsift_domain::{
    NoConfidentSpeaker, SegmentAttribution, SegmentSpeaker, SessionId, SpeakerRecord, SpeakerTag,
    TranscriptRevision,
};

use crate::{
    CommandName, LifecycleResponse, OperationResponse, search::bounded, transcript::RangeData,
};

/// The precision of every speaker label: a whole transcript segment.
const PRECISION: &str = "segment";

/// What a label is, in the words every answer that shows labels carries.
pub const SPEAKER_LABELS_STATEMENT: &str = "Speaker labels are provisional: S1, S2 and so on group transcript segments the diarization engine heard in one voice, numbered by when each voice is first heard. A label is not who the person is, two similar voices can share one label, and a segment in which two people speak is not split: the labels have segment precision. A verified name is the user's own statement, never inferred from a voice or the screen.";

/// The fixed warning of an answer that shows labels when some segments have none.
pub const UNLABELLED_SEGMENTS_WARNING: &str = "Some segments have no confident speaker (the engine heard little speech there, or more than one voice), and some may lie outside the audio the engine heard; speakers.coverage lists them. A search filtered by speaker does not find those segments.";

/// One label of a record, with the user's name if they recorded one.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SpeakerLabelData {
    label: String,
    display: String,
    verified_name: Option<String>,
    first_heard_us: u64,
    speech_us: u64,
    labelled_segments: u32,
}

/// The engine and models that made a record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SpeakerEngineData {
    name: &'static str,
    executable_sha256: String,
    segmentation_model_sha256: String,
    embedding_model_sha256: String,
}

/// What a record covers of its revision's segments.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SpeakerCoverageData {
    labelled_segments: u32,
    no_confident_speaker_segments: u32,
    not_diarized_segments: u32,
    labelled_ranges: Vec<RangeData>,
    no_confident_speaker_ranges: Vec<RangeData>,
    not_diarized_ranges: Vec<RangeData>,
    ranges_truncated: bool,
}

/// The `speakers` block: one speaker record of the revision an answer read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SpeakersData {
    record_id: String,
    record_number: u32,
    revision_id: String,
    labelling: &'static str,
    precision: &'static str,
    given_speaker_count: Option<u8>,
    diarized_range: RangeData,
    engine: SpeakerEngineData,
    labels: Vec<SpeakerLabelData>,
    coverage: SpeakerCoverageData,
    statement: &'static str,
}

impl SpeakersData {
    /// Presents a record.
    #[must_use]
    pub fn new(record: &SpeakerRecord) -> Self {
        let parts = record.parts();
        let coverage = record.coverage();
        let lists = [
            coverage.labelled(),
            coverage.no_confident_speaker(),
            coverage.not_diarized(),
        ];
        Self {
            record_id: record.id().as_str().to_owned(),
            record_number: parts.number.get(),
            revision_id: parts.revision_id.as_str().to_owned(),
            labelling: parts.profile.identifier(),
            precision: PRECISION,
            given_speaker_count: parts.count.given_count(),
            diarized_range: RangeData::new(parts.diarized_range),
            engine: SpeakerEngineData {
                name: parts.provenance.engine().identifier(),
                executable_sha256: parts.provenance.executable_sha256().as_str().to_owned(),
                segmentation_model_sha256: parts
                    .provenance
                    .segmentation_model_sha256()
                    .as_str()
                    .to_owned(),
                embedding_model_sha256: parts
                    .provenance
                    .embedding_model_sha256()
                    .as_str()
                    .to_owned(),
            },
            labels: record
                .speakers()
                .into_iter()
                .map(|summary| {
                    let name = record
                        .name(summary.tag())
                        .map(|name| name.as_str().to_owned());
                    SpeakerLabelData {
                        label: summary.tag().to_string(),
                        display: display(summary.tag(), name.as_deref()),
                        verified_name: name,
                        first_heard_us: summary.first_heard().as_micros(),
                        speech_us: summary.speech_micros(),
                        labelled_segments: summary.labelled_segments(),
                    }
                })
                .collect(),
            coverage: SpeakerCoverageData {
                labelled_segments: coverage.labelled_segments(),
                no_confident_speaker_segments: coverage.unlabelled_segments(),
                not_diarized_segments: coverage.not_diarized_segments(),
                labelled_ranges: bounded(coverage.labelled()),
                no_confident_speaker_ranges: bounded(coverage.no_confident_speaker()),
                not_diarized_ranges: bounded(coverage.not_diarized()),
                ranges_truncated: lists
                    .iter()
                    .any(|list| list.len() > crate::MAX_COVERAGE_RANGES),
            },
            statement: SPEAKER_LABELS_STATEMENT,
        }
    }
}

/// How a label reads: `S2, by voice`, or `Alice (S2, named by the user)`.
fn display(tag: SpeakerTag, name: Option<&str>) -> String {
    match name {
        Some(name) => format!(
            "{} ({tag}, named by the user)",
            crate::sanitize_untrusted_text(name, 512)
        ),
        None => format!("{tag}, by voice"),
    }
}

/// One segment's speaker under the record an answer read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct SegmentSpeakerLabelData {
    record_id: String,
    /// `null` when no label clearly holds the segment.
    label: Option<String>,
    display: String,
    verified_name: Option<String>,
    /// `null` with a label; `little_speech` or `mixed` without one.
    no_confident_speaker: Option<&'static str>,
    dominant_share_basis_points: Option<u16>,
    speech_share_basis_points: Option<u16>,
}

impl SegmentSpeakerLabelData {
    pub(crate) fn new(record: &SpeakerRecord, attribution: &SegmentAttribution) -> Self {
        let record_id = record.id().as_str().to_owned();
        match attribution.speaker() {
            SegmentSpeaker::Labelled {
                tag,
                dominant_basis_points,
                speech_basis_points,
            } => {
                let name = record.name(tag).map(|name| name.as_str().to_owned());
                Self {
                    record_id,
                    label: Some(tag.to_string()),
                    display: display(tag, name.as_deref()),
                    verified_name: name,
                    no_confident_speaker: None,
                    dominant_share_basis_points: Some(dominant_basis_points),
                    speech_share_basis_points: Some(speech_basis_points),
                }
            }
            SegmentSpeaker::NoConfidentSpeaker(reason) => Self {
                record_id,
                label: None,
                display: match reason {
                    NoConfidentSpeaker::LittleSpeech => {
                        "no confident speaker (little speech heard)".to_owned()
                    }
                    NoConfidentSpeaker::Mixed => {
                        "no confident speaker (more than one voice)".to_owned()
                    }
                },
                verified_name: None,
                no_confident_speaker: Some(reason.identifier()),
                dominant_share_basis_points: None,
                speech_share_basis_points: None,
            },
        }
    }

    /// The brief view's form: the label and how it reads.
    pub(crate) fn brief(&self) -> BriefSpeakerLabelData {
        BriefSpeakerLabelData {
            label: self.label.clone(),
            display: self.display.clone(),
        }
    }
}

/// A segment's speaker in the brief view.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct BriefSpeakerLabelData {
    label: Option<String>,
    display: String,
}

/// The per-segment label of `segment` under `record`, if the record labels it.
pub(crate) fn segment_label(
    record: Option<&SpeakerRecord>,
    segment: &vsift_domain::TranscriptSegment,
) -> Option<SegmentSpeakerLabelData> {
    let record = record?;
    record
        .attribution(segment.id())
        .map(|attribution| SegmentSpeakerLabelData::new(record, attribution))
}

/// Data of a `transcript.speakers` or `transcript.name-speaker` result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SpeakersResultData {
    session_id: String,
    speakers: SpeakersData,
    records_in_session: usize,
}

/// Everything a host presents about a speaker record it committed or read.
#[derive(Clone, Debug)]
pub struct SpeakersPresentation<'a> {
    /// The session.
    pub session_id: &'a SessionId,
    /// The revision the record labels.
    pub revision: &'a TranscriptRevision,
    /// The record.
    pub record: &'a SpeakerRecord,
    /// How many speaker records the session holds.
    pub records: usize,
}

/// The complete result of `transcript speakers` (or `transcript name-speaker`, by
/// `command`): `partial` with [`UNLABELLED_SEGMENTS_WARNING`] when some segment has no
/// label, a success that exits 0.
///
/// # Errors
///
/// Returns the serialization error when the data cannot be represented as JSON.
pub fn speakers_response(
    command: CommandName,
    page: &SpeakersPresentation<'_>,
    lifecycle: LifecycleResponse,
) -> Result<OperationResponse<serde_json::Value>, serde_json::Error> {
    let data = SpeakersResultData {
        session_id: page.session_id.as_str().to_owned(),
        speakers: SpeakersData::new(page.record),
        records_in_session: page.records,
    };
    let response = if page.record.coverage().is_complete() {
        OperationResponse::complete(command.identifier(), &data)?
    } else {
        OperationResponse::partial(command.identifier(), &data, UNLABELLED_SEGMENTS_WARNING)?
    };
    Ok(response.with_lifecycle(lifecycle))
}

/// Adds [`UNLABELLED_SEGMENTS_WARNING`] when `record` leaves a segment unlabelled.
pub(crate) fn with_unlabelled_warning(
    response: OperationResponse<serde_json::Value>,
    record: Option<&SpeakerRecord>,
) -> OperationResponse<serde_json::Value> {
    match record {
        Some(record) if !record.coverage().is_complete() => {
            response.with_warnings(&[UNLABELLED_SEGMENTS_WARNING])
        }
        _ => response,
    }
}

/// Fixed remediation when the diarization tool or a model is missing.
pub const DIARIZER_REMEDIATION: &str = "Speaker labels need the speaker-diarization tool (sherpa-onnx-offline-speaker-diarization) and its two models: the pyannote segmentation 3.0 model and the NeMo TitaNet small embedding model. Install them with setup install, or give their paths with --diarizer, --segmentation-model and --embedding-model. No need to search folders: ask the user where they are. Transcripts and search work without them.";

/// Fixed remediation when the range is longer than one diarization may hear.
pub const DIARIZATION_RANGE_REMEDIATION: &str = "One diarization hears at most two hours. Label a part of the recording with --from and --to (microseconds), at most two hours long; labels of different parts are not linked, so S1 in one part need not be S1 in another.";

/// Fixed remediation when a revision has no speaker record to name a label in.
pub const NO_SPEAKER_RECORD_REMEDIATION: &str = "This transcript revision has no speaker labels yet. Run transcript speakers with this session first, then name a label with transcript name-speaker.";
