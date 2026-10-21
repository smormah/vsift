//! Speaker labels for a transcript revision: the diarization port and its use cases
//! (ADR 0031).
//!
//! [`label_speakers`] checks the request against the revision and the duration cap,
//! asks a [`SpeakerDiarizer`] for turns, and returns the new [`SpeakerRecord`] for the
//! caller to commit; every per-segment label is derived by the domain. [`name_speaker`]
//! makes the next record of the same turns with a name the user verified. Neither use
//! case sees audio or a voice embedding: the adapter decodes the audio, the engine keeps
//! its embeddings in its own process, and only times and cluster numbers come back.

use std::{error::Error, fmt, future::Future, num::NonZeroU32};

use vsift_domain::{
    DiarizationError, DiarizationProvenance, DiarizedTurn, MediaTime, SessionId, SourceId,
    SpeakerCount, SpeakerLabellingProfile, SpeakerRecord, SpeakerRecordId, SpeakerRecordParts,
    SpeakerTag, TimeRange, TranscriptRevision, TranscriptRevisionId, VerifiedName,
};

use crate::transcript::derived_identity;

#[cfg(test)]
mod tests;

/// The longest stretch of audio one diarization may hear, in microseconds.
///
/// The engine holds the whole stretch, and the embeddings it computes from it, in
/// memory until it clusters (L-205). The maintainer's decision of 2026-10-21 aims at
/// about two hours if an 8 GB machine allows; the evaluation's hosted measurement sets
/// the figure (`docs/planning/speaker-labels-evaluation.md`, section 6.4).
pub const MAX_DIARIZATION_MICROS: u64 = 2 * 60 * 60 * 1_000_000;

/// The shortest stretch worth diarizing: one segmentation window of the engine.
pub const MIN_DIARIZATION_MICROS: u64 = 1_000_000;

/// What the engine is asked to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiarizationRequest {
    /// The source time to hear.
    pub range: TimeRange,
    /// How many speakers to find.
    pub count: SpeakerCount,
}

/// Why diarization could not produce turns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiarizationRunError {
    /// The engine or a model file is missing or cannot be identified.
    EngineUnavailable,
    /// The engine or its audio decode exceeded the deadline.
    Deadline,
    /// The caller cancelled.
    Cancelled,
    /// Output or work exceeded a bound.
    ResourceLimit,
    /// The engine exited unsuccessfully.
    EngineFailed,
    /// The engine ended abnormally (a signal, or an operating-system crash status);
    /// exhausted memory is the usual cause.
    AbnormalTermination,
    /// The engine's output is not its documented format.
    UnparseableOutput,
    /// The source has no audio stream that can be decoded.
    NoAudio,
    /// The source copy changed or could not be read.
    Source,
    /// The private work folder could not be used.
    Workspace,
    /// Admission capacity was unavailable.
    Busy,
}

impl fmt::Display for DiarizationRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EngineUnavailable => "the diarization engine or a model is unavailable",
            Self::Deadline => "diarization exceeded its deadline",
            Self::Cancelled => "diarization was cancelled",
            Self::ResourceLimit => "diarization output or work exceeded its bound",
            Self::EngineFailed => "the diarization engine failed",
            Self::AbnormalTermination => "the diarization engine terminated abnormally",
            Self::UnparseableOutput => "the diarization engine's output is unparseable",
            Self::NoAudio => "the source has no audio to diarize",
            Self::Source => "the source copy could not be read",
            Self::Workspace => "the diarization work folder could not be used",
            Self::Busy => "diarization admission is busy",
        })
    }
}

impl Error for DiarizationRunError {}

/// Port that hears a source's audio and reports who spoke when.
pub trait SpeakerDiarizer: Send + Sync {
    /// Identifies the engine and models as they are now, from their bytes.
    fn identity(
        &self,
    ) -> impl Future<Output = Result<DiarizationProvenance, DiarizationRunError>> + Send;

    /// Diarizes `request.range` of the bound source.
    ///
    /// Returns the engine's turns on the source timeline. They are bounded and
    /// parsed, not yet checked against the range; the use case and the domain do that.
    fn diarize(
        &self,
        request: DiarizationRequest,
    ) -> impl Future<Output = Result<Vec<DiarizedTurn>, DiarizationRunError>> + Send;
}

/// Why speaker labels could not be made or named.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeakerLabelError {
    /// The requested range lies outside the source.
    RangeOutsideSource,
    /// The range is shorter than [`MIN_DIARIZATION_MICROS`].
    RangeTooShort,
    /// The range is longer than [`MAX_DIARIZATION_MICROS`]; the remediation is a range
    /// of at most that.
    RangeTooLong {
        /// The cap, in microseconds.
        cap_micros: u64,
        /// The range asked for, in microseconds.
        requested_micros: u64,
    },
    /// The engine could not produce turns.
    Diarization(DiarizationRunError),
    /// The engine's identity changed during the run (a swapped file).
    EngineChanged,
    /// The turns or the record were refused by the domain.
    Record(DiarizationError),
    /// A record identity could not be formed.
    Identity,
}

impl fmt::Display for SpeakerLabelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RangeOutsideSource => formatter.write_str("the range lies outside the source"),
            Self::RangeTooShort => formatter.write_str("the range is too short to diarize"),
            Self::RangeTooLong { cap_micros, .. } => write!(
                formatter,
                "one diarization hears at most {} minutes",
                cap_micros / 60_000_000
            ),
            Self::Diarization(error) => error.fmt(formatter),
            Self::EngineChanged => {
                formatter.write_str("the diarization engine or a model changed during the run")
            }
            Self::Record(error) => error.fmt(formatter),
            Self::Identity => formatter.write_str("a speaker record identity could not be formed"),
        }
    }
}

impl Error for SpeakerLabelError {}

/// What to label, and where the new record goes among the session's records.
#[derive(Clone, Debug)]
pub struct LabelSpeakersRequest<'a> {
    /// The session.
    pub session_id: &'a SessionId,
    /// The revision to label.
    pub revision: &'a TranscriptRevision,
    /// The source's duration on the normalized timeline.
    pub source_duration: MediaTime,
    /// The range to hear; `None` is the whole source.
    pub range: Option<TimeRange>,
    /// How many speakers to find.
    pub count: SpeakerCount,
    /// The number the new record takes (the session's newest plus one).
    pub number: NonZeroU32,
}

/// The identity of the session's `number`th speaker record, for `revision_id` of `source_id`.
///
/// # Errors
///
/// Returns [`SpeakerLabelError::Identity`] when the derived identity does not parse.
pub fn speaker_record_id(
    session_id: &SessionId,
    source_id: &SourceId,
    revision_id: &TranscriptRevisionId,
    number: NonZeroU32,
) -> Result<SpeakerRecordId, SpeakerLabelError> {
    SpeakerRecordId::parse(derived_identity(
        "spk_",
        "vsift.speaker-record.v1",
        &[
            session_id.as_str(),
            source_id.as_str(),
            revision_id.as_str(),
            &number.get().to_string(),
        ],
    ))
    .map_err(|_| SpeakerLabelError::Identity)
}

/// The range one diarization hears: the request's, or the whole source, checked
/// against the source and the cap.
///
/// # Errors
///
/// Returns the range errors of [`SpeakerLabelError`].
pub fn diarization_range(
    requested: Option<TimeRange>,
    source_duration: MediaTime,
) -> Result<TimeRange, SpeakerLabelError> {
    let range = match requested {
        Some(range) => range,
        None => TimeRange::new(MediaTime::from_micros(0), source_duration)
            .map_err(|_| SpeakerLabelError::RangeOutsideSource)?,
    };
    if range.end() > source_duration {
        return Err(SpeakerLabelError::RangeOutsideSource);
    }
    let requested_micros = range.duration_micros();
    if requested_micros < MIN_DIARIZATION_MICROS {
        return Err(SpeakerLabelError::RangeTooShort);
    }
    if requested_micros > MAX_DIARIZATION_MICROS {
        return Err(SpeakerLabelError::RangeTooLong {
            cap_micros: MAX_DIARIZATION_MICROS,
            requested_micros,
        });
    }
    Ok(range)
}

/// Diarizes the revision's source and returns its new speaker record, uncommitted.
///
/// The engine is identified before and after the run, so turns from a swapped
/// executable or model are never kept. Turns that end past the range by less than one
/// 10 ms frame (the engine's own rounding) are trimmed to it; any other turn outside it
/// is refused by the domain.
///
/// # Errors
///
/// Returns a typed [`SpeakerLabelError`]; nothing is committed by this function.
pub async fn label_speakers(
    request: LabelSpeakersRequest<'_>,
    diarizer: &impl SpeakerDiarizer,
) -> Result<SpeakerRecord, SpeakerLabelError> {
    let range = diarization_range(request.range, request.source_duration)?;
    let id = speaker_record_id(
        request.session_id,
        request.revision.source_id(),
        request.revision.id(),
        request.number,
    )?;
    let before = diarizer
        .identity()
        .await
        .map_err(SpeakerLabelError::Diarization)?;
    let turns = diarizer
        .diarize(DiarizationRequest {
            range,
            count: request.count,
        })
        .await
        .map_err(SpeakerLabelError::Diarization)?;
    let after = diarizer
        .identity()
        .await
        .map_err(SpeakerLabelError::Diarization)?;
    if before != after {
        return Err(SpeakerLabelError::EngineChanged);
    }
    let turns = trim_to_range(turns, range);
    SpeakerRecord::derive(
        SpeakerRecordParts {
            id,
            number: request.number,
            source_id: request.revision.source_id().clone(),
            revision_id: request.revision.id().clone(),
            diarized_range: range,
            count: request.count,
            profile: SpeakerLabellingProfile::V1,
            provenance: after,
            turns,
            names: std::collections::BTreeMap::new(),
        },
        request.revision,
    )
    .map_err(SpeakerLabelError::Record)
}

/// The engine's 10 ms output frame: a turn may end this far past the audio it heard.
const ENGINE_ROUNDING_MICROS: u64 = 10_000;

fn trim_to_range(turns: Vec<DiarizedTurn>, range: TimeRange) -> Vec<DiarizedTurn> {
    turns
        .into_iter()
        .filter_map(|turn| {
            let end = turn.range().end();
            if end > range.end()
                && end.as_micros() <= range.end().as_micros() + ENGINE_ROUNDING_MICROS
            {
                TimeRange::new(turn.range().start(), range.end())
                    .ok()
                    .map(|trimmed| DiarizedTurn::new(trimmed, turn.speaker()))
            } else {
                Some(turn)
            }
        })
        .collect()
}

/// The next speaker record: the same turns as `record`, with `tag` named `name` (or
/// its name removed when `name` is `None`).
///
/// # Errors
///
/// Returns [`SpeakerLabelError::Record`] when the record has no such label or belongs
/// to another revision, and [`SpeakerLabelError::Identity`] when the new identity does
/// not form.
pub fn name_speaker(
    session_id: &SessionId,
    record: &SpeakerRecord,
    revision: &TranscriptRevision,
    tag: SpeakerTag,
    name: Option<VerifiedName>,
    number: NonZeroU32,
) -> Result<SpeakerRecord, SpeakerLabelError> {
    let mut parts = record
        .with_name(tag, name)
        .map_err(SpeakerLabelError::Record)?;
    parts.id = speaker_record_id(session_id, revision.source_id(), revision.id(), number)?;
    parts.number = number;
    SpeakerRecord::derive(parts, revision).map_err(SpeakerLabelError::Record)
}
