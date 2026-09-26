//! Presentation of the evidence navigation commands through the v1 contract
//! (P09, ADR 0019).
//!
//! The engine validates, reuses, extracts and commits; this module maps the
//! parsed command to its engine request, the engine's typed result to the
//! `vsift-contract` result or evidence stream, and an engine failure to its
//! code with the fixed-prose remediation an agent needs to recover.

use vsift::{
    AudioClipRequest, Cancellation, CropEvidenceRequest, Engine, EngineError, EvidenceMediaError,
    EvidenceResults, FailureCode, FrameBurstRequest, FrameGetRequest, FrameNeighboursRequest,
    FrameSelection, FrameTarget,
};
use vsift_contract::{
    AUDIO_RANGE_REMEDIATION, AUDIO_RANGE_START_REMEDIATION, AudioEvidenceStream,
    BURST_RANGE_REMEDIATION, CROP_OUTSIDE_REMEDIATION, DeliveredEvidenceFile,
    EVIDENCE_BUDGET_REMEDIATION, EVIDENCE_KIND_REMEDIATION, EVIDENCE_PATH_REMEDIATION,
    EVIDENCE_TOOLS_REMEDIATION, EvidencePresentation, EvidencePresentationError,
    FrameEvidenceStream, LifecycleResponse, NO_AUDIO_CLIP_REMEDIATION, NO_FRAMES_REMEDIATION,
    OperationResponse, UNDECODABLE_EVIDENCE_REMEDIATION, UNKNOWN_CANDIDATE_REMEDIATION,
    UNKNOWN_EVIDENCE_REMEDIATION, audio_response, frame_response, frame_selection_summary,
};

use crate::{
    CommandFailure,
    command::{AudioArguments, CropArguments, FrameCommand},
    session::rfc3339,
};

/// Which kind of evidence a command extracts; some failures mean different
/// things for pictures and for sound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Medium {
    /// Frames and crops.
    Picture,
    /// Audio clips.
    Sound,
}

/// Maps an evidence failure to its code and remediation.
///
/// Evidence-specific causes are matched first: the generic mapping would
/// describe a missing media tool in terms of transcript import, and a missing
/// audio stream in terms of speech recognition.
fn evidence_failure(error: EngineError, medium: Medium) -> CommandFailure {
    let remediation = match (&error, medium) {
        (EngineError::MediaToolUnavailable(_), _) => Some(EVIDENCE_TOOLS_REMEDIATION),
        (EngineError::EvidenceBudgetExhausted, _) => Some(EVIDENCE_BUDGET_REMEDIATION),
        (EngineError::NavigationRangeTooLong, Medium::Picture) => Some(BURST_RANGE_REMEDIATION),
        (EngineError::NavigationRangeTooLong, Medium::Sound) => Some(AUDIO_RANGE_REMEDIATION),
        (EngineError::RangeOutsideSource, Medium::Sound) => Some(AUDIO_RANGE_START_REMEDIATION),
        (EngineError::NoAudioStream, Medium::Sound) => Some(NO_AUDIO_CLIP_REMEDIATION),
        (EngineError::CandidateNotFound, _) => Some(UNKNOWN_CANDIDATE_REMEDIATION),
        (EngineError::EvidenceNotFound, _) => Some(UNKNOWN_EVIDENCE_REMEDIATION),
        (EngineError::EvidenceKindMismatch, _) => Some(EVIDENCE_KIND_REMEDIATION),
        (EngineError::CropOutsideParent, _) => Some(CROP_OUTSIDE_REMEDIATION),
        (EngineError::NoVideoStream, _) => Some(NO_FRAMES_REMEDIATION),
        (
            EngineError::EvidenceMedia(
                EvidenceMediaError::Undecodable | EvidenceMediaError::NoDecodedAudio,
            ),
            _,
        ) => Some(UNDECODABLE_EVIDENCE_REMEDIATION),
        (EngineError::FrameNotSelected(selection), _) => Some(frame_selection_summary(*selection)),
        _ => None,
    };
    match remediation {
        Some(summary) => CommandFailure {
            code: error.failure_code(),
            remediation: Some(summary.to_owned()),
        },
        None => CommandFailure::from(error),
    }
}

/// A presentation failure: a path that JSON cannot name is the output's
/// fault, anything else an internal one.
fn presentation_failure(error: &EvidencePresentationError) -> CommandFailure {
    match error {
        EvidencePresentationError::NonUtf8Path => CommandFailure {
            code: FailureCode::StorageIo,
            remediation: Some(EVIDENCE_PATH_REMEDIATION.to_owned()),
        },
        EvidencePresentationError::OperationMismatch
        | EvidencePresentationError::FileMismatch
        | EvidencePresentationError::Serialization(_) => {
            CommandFailure::from(FailureCode::Internal)
        }
    }
}

/// Runs one frame command in the engine.
async fn run_frame(
    engine: &Engine,
    command: FrameCommand,
) -> Result<EvidenceResults, CommandFailure> {
    let cancellation = Cancellation::new();
    let result = match command {
        FrameCommand::Get(arguments) => {
            let target = match (arguments.at, arguments.candidate) {
                (Some(at_micros), None) => FrameTarget::At {
                    at_micros,
                    selection: arguments
                        .select
                        .map_or(FrameSelection::AtOrAfter, FrameSelection::from),
                    tolerance_micros: arguments.tolerance_us,
                },
                (None, Some(candidate)) => FrameTarget::Candidate(candidate),
                // The grammar requires exactly one of the two.
                (Some(_), Some(_)) | (None, None) => {
                    return Err(CommandFailure::from(FailureCode::InvalidArgument));
                }
            };
            engine
                .frame_get(FrameGetRequest {
                    session: arguments.session,
                    target,
                    cancellation,
                })
                .await
        }
        FrameCommand::Neighbours(arguments) => {
            engine
                .frame_neighbours(FrameNeighboursRequest {
                    session: arguments.session,
                    anchor: arguments.evidence,
                    count: arguments.count,
                    cancellation,
                })
                .await
        }
        FrameCommand::Burst(arguments) => {
            engine
                .frame_burst(FrameBurstRequest {
                    session: arguments.session,
                    from_micros: arguments.from,
                    to_micros: arguments.to,
                    max_frames: arguments.max_frames,
                    cancellation,
                })
                .await
        }
    };
    result.map_err(|error| evidence_failure(error, Medium::Picture))
}

async fn run_crop(
    engine: &Engine,
    arguments: CropArguments,
) -> Result<EvidenceResults, CommandFailure> {
    engine
        .crop(CropEvidenceRequest {
            session: arguments.session,
            parent: arguments.evidence,
            rect: arguments.rect,
            cancellation: Cancellation::new(),
        })
        .await
        .map_err(|error| evidence_failure(error, Medium::Picture))
}

async fn run_audio(
    engine: &Engine,
    arguments: AudioArguments,
) -> Result<EvidenceResults, CommandFailure> {
    engine
        .audio(AudioClipRequest {
            session: arguments.session,
            from_micros: arguments.from,
            to_micros: arguments.to,
            cancellation: Cancellation::new(),
        })
        .await
        .map_err(|error| evidence_failure(error, Medium::Sound))
}

fn lifecycle(results: &EvidenceResults) -> Result<LifecycleResponse, CommandFailure> {
    Ok(LifecycleResponse::ephemeral(rfc3339(
        results.session().lifetime().expires_at_unix_seconds(),
    )?))
}

fn delivered(results: &EvidenceResults) -> Vec<DeliveredEvidenceFile<'_>> {
    results
        .files()
        .iter()
        .map(|file| DeliveredEvidenceFile {
            evidence_id: file.evidence_id(),
            kind: file.kind(),
            path: file.path(),
        })
        .collect()
}

/// Presents a frame or crop result.
fn picture_response(
    results: &EvidenceResults,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let files = delivered(results);
    frame_response(
        &EvidencePresentation {
            record: results.record(),
            reused: results.reused(),
            files: &files,
        },
        lifecycle(results)?,
    )
    .map_err(|error| presentation_failure(&error))
}

/// Presents a frame or crop result as an evidence stream: the items, then
/// the terminal event with everything else.
fn picture_stream(results: &EvidenceResults) -> Result<FrameEvidenceStream, CommandFailure> {
    let files = delivered(results);
    FrameEvidenceStream::new(
        &EvidencePresentation {
            record: results.record(),
            reused: results.reused(),
            files: &files,
        },
        lifecycle(results)?,
    )
    .map_err(|error| presentation_failure(&error))
}

/// Runs a frame command and presents its result.
pub(crate) async fn frame(
    engine: &Engine,
    command: FrameCommand,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    picture_response(&run_frame(engine, command).await?)
}

/// Runs a frame command and presents its result as an evidence stream.
pub(crate) async fn frame_stream(
    engine: &Engine,
    command: FrameCommand,
) -> Result<FrameEvidenceStream, CommandFailure> {
    picture_stream(&run_frame(engine, command).await?)
}

/// Runs `crop` and presents its result.
pub(crate) async fn crop(
    engine: &Engine,
    arguments: CropArguments,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    picture_response(&run_crop(engine, arguments).await?)
}

/// Runs `crop` and presents its result as an evidence stream.
pub(crate) async fn crop_stream(
    engine: &Engine,
    arguments: CropArguments,
) -> Result<FrameEvidenceStream, CommandFailure> {
    picture_stream(&run_crop(engine, arguments).await?)
}

/// Runs `audio` and presents its result.
pub(crate) async fn audio(
    engine: &Engine,
    arguments: AudioArguments,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let results = run_audio(engine, arguments).await?;
    let files = delivered(&results);
    audio_response(
        &EvidencePresentation {
            record: results.record(),
            reused: results.reused(),
            files: &files,
        },
        lifecycle(&results)?,
    )
    .map_err(|error| presentation_failure(&error))
}

/// Runs `audio` and presents its result as an evidence stream: the clip,
/// then the terminal event with everything else.
pub(crate) async fn audio_stream(
    engine: &Engine,
    arguments: AudioArguments,
) -> Result<AudioEvidenceStream, CommandFailure> {
    let results = run_audio(engine, arguments).await?;
    let files = delivered(&results);
    AudioEvidenceStream::new(
        &EvidencePresentation {
            record: results.record(),
            reused: results.reused(),
            files: &files,
        },
        lifecycle(&results)?,
    )
    .map_err(|error| presentation_failure(&error))
}
