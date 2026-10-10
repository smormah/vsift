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
    FrameSelection, FrameTarget, SessionStorageError, SourceProbeError,
};
use vsift_contract::{
    AUDIO_RANGE_REMEDIATION, AUDIO_RANGE_START_REMEDIATION, AUDIO_RANGE_TOO_SHORT_REMEDIATION,
    AudioEvidenceStream, BURST_RANGE_REMEDIATION, CROP_OUTSIDE_REMEDIATION, DeliveredEvidenceFile,
    EVIDENCE_BUDGET_REMEDIATION, EVIDENCE_KIND_REMEDIATION, EVIDENCE_PATH_REMEDIATION,
    EVIDENCE_TOOLS_REMEDIATION, EvidencePresentation, EvidencePresentationError,
    FrameEvidenceStream, LifecycleResponse, MEDIA_BUSY_REMEDIATION, NO_AUDIO_CLIP_REMEDIATION,
    NO_FRAMES_REMEDIATION, OperationResponse, UNDECODABLE_EVIDENCE_REMEDIATION,
    UNKNOWN_CANDIDATE_REMEDIATION, UNKNOWN_EVIDENCE_REMEDIATION, audio_response, frame_response,
    frame_selection_summary,
};

use crate::{
    CommandFailure,
    command::{AudioArguments, CropArguments, FrameCommand},
    session::session_lifecycle,
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

/// Whether the failure is another request holding what an evidence command
/// needs (#342).
///
/// Three typed causes, one answer. The media adapter never waits for the
/// root's capacity, so its probe and its extraction each report contention
/// at once ([`SourceProbeError::Busy`], [`EvidenceMediaError::Busy`]); and
/// the session store reports a lock another request holds (the session's, or
/// its writer lock at the commit) or the capacity the commit itself reserves
/// as [`SessionStorageError::Busy`]. All three have the published code
/// `BUSY`, and the caller does the same for each: wait, then run the command
/// again. Nothing is committed in any of them, because evidence is published
/// in one generation.
const fn is_contention(error: &EngineError) -> bool {
    matches!(
        error,
        EngineError::EvidenceMedia(EvidenceMediaError::Busy)
            | EngineError::SourceProbe(SourceProbeError::Busy)
            | EngineError::Storage(SessionStorageError::Busy)
    )
}

/// Maps an evidence failure to its code and remediation.
///
/// Evidence-specific causes are matched first: the generic mapping would
/// describe a missing media tool in terms of transcript import, and a missing
/// audio stream in terms of speech recognition.
fn evidence_failure(error: EngineError, medium: Medium) -> CommandFailure {
    if is_contention(&error) {
        return CommandFailure::contention(error.failure_code(), MEDIA_BUSY_REMEDIATION.to_owned());
    }
    let remediation = match (&error, medium) {
        (EngineError::MediaToolUnavailable(_), _) => Some(EVIDENCE_TOOLS_REMEDIATION),
        (EngineError::EvidenceBudgetExhausted, _) => Some(EVIDENCE_BUDGET_REMEDIATION),
        (EngineError::NavigationRangeTooLong, Medium::Picture) => Some(BURST_RANGE_REMEDIATION),
        (EngineError::NavigationRangeTooLong, Medium::Sound) => Some(AUDIO_RANGE_REMEDIATION),
        (EngineError::RangeOutsideSource, Medium::Sound) => Some(AUDIO_RANGE_START_REMEDIATION),
        (EngineError::AudioRangeTooShort, _) => Some(AUDIO_RANGE_TOO_SHORT_REMEDIATION),
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
        Some(summary) => CommandFailure::with_remediation(error.failure_code(), summary.to_owned()),
        None => CommandFailure::from(error),
    }
}

/// A presentation failure: a path that JSON cannot name is the output's
/// fault, anything else an internal one.
fn presentation_failure(error: &EvidencePresentationError) -> CommandFailure {
    match error {
        EvidencePresentationError::NonUtf8Path => CommandFailure::with_remediation(
            FailureCode::StorageIo,
            EVIDENCE_PATH_REMEDIATION.to_owned(),
        ),
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
    cancellation: &Cancellation,
) -> Result<EvidenceResults, CommandFailure> {
    let cancellation = cancellation.clone();
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
    cancellation: &Cancellation,
) -> Result<EvidenceResults, CommandFailure> {
    engine
        .crop(CropEvidenceRequest {
            session: arguments.session,
            parent: arguments.evidence,
            rect: arguments.rect,
            cancellation: cancellation.clone(),
        })
        .await
        .map_err(|error| evidence_failure(error, Medium::Picture))
}

async fn run_audio(
    engine: &Engine,
    arguments: AudioArguments,
    cancellation: &Cancellation,
) -> Result<EvidenceResults, CommandFailure> {
    engine
        .audio(AudioClipRequest {
            session: arguments.session,
            from_micros: arguments.from,
            to_micros: arguments.to,
            cancellation: cancellation.clone(),
        })
        .await
        .map_err(|error| evidence_failure(error, Medium::Sound))
}

fn lifecycle(results: &EvidenceResults) -> Result<LifecycleResponse, CommandFailure> {
    Ok(session_lifecycle(results.session().lifetime())?)
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
    cancellation: &Cancellation,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    picture_response(&run_frame(engine, command, cancellation).await?)
}

/// Runs a frame command and presents its result as an evidence stream.
pub(crate) async fn frame_stream(
    engine: &Engine,
    command: FrameCommand,
    cancellation: &Cancellation,
) -> Result<FrameEvidenceStream, CommandFailure> {
    picture_stream(&run_frame(engine, command, cancellation).await?)
}

/// Runs `crop` and presents its result.
pub(crate) async fn crop(
    engine: &Engine,
    arguments: CropArguments,
    cancellation: &Cancellation,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    picture_response(&run_crop(engine, arguments, cancellation).await?)
}

/// Runs `crop` and presents its result as an evidence stream.
pub(crate) async fn crop_stream(
    engine: &Engine,
    arguments: CropArguments,
    cancellation: &Cancellation,
) -> Result<FrameEvidenceStream, CommandFailure> {
    picture_stream(&run_crop(engine, arguments, cancellation).await?)
}

/// Runs `audio` and presents its result.
pub(crate) async fn audio(
    engine: &Engine,
    arguments: AudioArguments,
    cancellation: &Cancellation,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let results = run_audio(engine, arguments, cancellation).await?;
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
    cancellation: &Cancellation,
) -> Result<AudioEvidenceStream, CommandFailure> {
    let results = run_audio(engine, arguments, cancellation).await?;
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

#[cfg(test)]
mod tests {
    use vsift::{
        ADMISSION_RETRY_AFTER, EngineError, EvidenceMediaError, FailureCode, SessionStorageError,
        SourceProbeError,
    };
    use vsift_contract::{
        AUDIO_RANGE_START_REMEDIATION, AUDIO_RANGE_TOO_SHORT_REMEDIATION, CommandName,
        MEDIA_BUSY_REMEDIATION,
    };

    use super::{Medium, evidence_failure};
    use crate::{OutputMode, OutputWriter, ProcessExit, write_command_failure};

    /// #342: every typed cause of contention an evidence command can meet is
    /// `BUSY` (the code it always had) with the remediation and the admission
    /// retry hint, for a picture and for a clip alike; other storage failures
    /// and other media failures keep what they had.
    #[test]
    fn contention_on_an_evidence_command_is_busy_with_a_remediation_and_a_retry_hint()
    -> Result<(), Box<dyn std::error::Error>> {
        let hint = u64::try_from(ADMISSION_RETRY_AFTER.as_millis())?;
        assert_eq!(hint, 2_000);
        for error in [
            EngineError::EvidenceMedia(EvidenceMediaError::Busy),
            EngineError::SourceProbe(SourceProbeError::Busy),
            EngineError::Storage(SessionStorageError::Busy),
        ] {
            for medium in [Medium::Picture, Medium::Sound] {
                let failure = evidence_failure(error.clone(), medium);
                assert_eq!(failure.code, FailureCode::Busy, "{error}");
                assert!(failure.code.retryable(), "{error}");
                assert_eq!(failure.summary(), Some(MEDIA_BUSY_REMEDIATION), "{error}");
                assert_eq!(failure.retry_after_ms, Some(hint), "{error}");
            }
        }

        for error in [
            EngineError::Storage(SessionStorageError::Io),
            EngineError::Storage(SessionStorageError::IntegrityFailure),
            EngineError::SourceProbe(SourceProbeError::Deadline),
            EngineError::EvidenceMedia(EvidenceMediaError::Deadline),
            EngineError::EvidenceMedia(EvidenceMediaError::Cancelled),
        ] {
            let failure = evidence_failure(error.clone(), Medium::Picture);
            assert_ne!(failure.summary(), Some(MEDIA_BUSY_REMEDIATION), "{error}");
            assert_eq!(failure.retry_after_ms, None, "{error}");
        }
        Ok(())
    }

    /// #342: what `frame get` prints for a refused request, as `--json` and
    /// for a person, is the published example; the exit status is the
    /// retryable class (4), as it was.
    #[test]
    fn a_busy_frame_get_answers_the_published_example_and_exits_with_the_retryable_class()
    -> Result<(), Box<dyn std::error::Error>> {
        let example: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../schemas/v1/examples/frame-get.busy.json"),
        )?)?;
        let busy = || {
            evidence_failure(
                EngineError::Storage(SessionStorageError::Busy),
                Medium::Picture,
            )
        };

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut writer = OutputWriter::new(&mut stdout, &mut stderr);
        let written =
            write_command_failure(&mut writer, OutputMode::Json, CommandName::FrameGet, busy());
        assert_eq!(written, ProcessExit::Retryable);
        assert_eq!(written.code(), 4);
        assert!(stderr.is_empty());
        let value: serde_json::Value = serde_json::from_slice(&stdout)?;
        assert_eq!(value, example);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut writer = OutputWriter::new(&mut stdout, &mut stderr);
        let written = write_command_failure(
            &mut writer,
            OutputMode::Human,
            CommandName::FrameGet,
            busy(),
        );
        assert_eq!(written, ProcessExit::Retryable);
        assert!(stdout.is_empty());
        assert_eq!(
            String::from_utf8(stderr)?,
            format!(
                "Error: The requested operation is temporarily busy. (BUSY)\nFix: {MEDIA_BUSY_REMEDIATION}\nRetry after: 2000 ms\n"
            )
        );
        Ok(())
    }

    /// #332: an audio range whose length rounds to no sample is the caller's
    /// range to change, `INVALID_ARGUMENT`, with a remediation that says how
    /// short is too short; it is not the answer of a range past the end.
    #[test]
    fn an_audio_range_that_rounds_to_no_sample_is_an_invalid_argument_with_its_own_remediation() {
        let failure = evidence_failure(EngineError::AudioRangeTooShort, Medium::Sound);
        assert_eq!(failure.code, FailureCode::InvalidArgument);
        assert_eq!(failure.summary(), Some(AUDIO_RANGE_TOO_SHORT_REMEDIATION));
        assert!(AUDIO_RANGE_TOO_SHORT_REMEDIATION.contains("31 microseconds or less"));
        assert!(AUDIO_RANGE_TOO_SHORT_REMEDIATION.len() <= 1024);

        let past_the_end = evidence_failure(EngineError::RangeOutsideSource, Medium::Sound);
        assert_eq!(past_the_end.code, FailureCode::InvalidArgument);
        assert_eq!(past_the_end.summary(), Some(AUDIO_RANGE_START_REMEDIATION));
    }
}
