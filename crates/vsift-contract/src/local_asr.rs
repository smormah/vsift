//! Fixed-prose remediation for local speech recognition (`transcript
//! retranscribe`).
//!
//! Every summary is chosen by a typed value and names its identifiers in a
//! fixed first sentence, so an agent can act on them without parsing free
//! text. None contains a path, provider output or transcript text.

use vsift_application::{AsrFailure, AsrFailureReason, LocalAsrVerificationFailure};
use vsift_domain::ProviderOutputError;

/// Remediation when `FFmpeg`, `FFprobe` or whisper.cpp is missing.
pub const LOCAL_ASR_TOOLS_REMEDIATION: &str = "Local speech recognition needs FFmpeg, FFprobe and the whisper.cpp CLI (whisper-cli). Nothing was changed. Install or locate trusted builds, register them with setup configure ffmpeg|ffprobe|whisper --executable <path>, then run setup check. A supplied SubRip or WebVTT transcript (ingest --transcript) needs no speech recognition.";

/// Remediation when no model is configured or the configured file is unusable.
pub const LOCAL_ASR_MODEL_REMEDIATION: &str = "Local speech recognition needs a registered whisper.cpp model. Nothing was changed. Register the reviewed multilingual base model (ggml-base.bin) with setup configure-model --file <path>, then retry.";

/// Remediation when the configured model is not a reviewed pinned profile.
pub const UNPINNED_MODEL_REMEDIATION: &str = "The registered model file is not a reviewed pinned model profile, so VSift will not run it: its accuracy, licence and resource use are unknown. Nothing was changed. Register a reviewed multilingual whisper.cpp base model with setup configure-model --file <path>, then retry: ggml-base.bin (147,951,465 bytes, the default) or its quantization ggml-base-q5_1.bin (59,707,625 bytes).";

/// Remediation when the video has no audio stream `VSift` can decode.
pub const NO_AUDIO_STREAM_REMEDIATION: &str = "The video has no audio stream VSift can decode, so there is no speech to transcribe. Nothing was changed. Check that the video has sound; a supplied SubRip or WebVTT transcript can be imported with ingest --transcript instead.";

/// Remediation when a requested revision is not in the session.
pub const UNKNOWN_REVISION_REMEDIATION: &str = "The session holds no transcript revision with that identity. Use a revision_id returned by ingest, transcript get or transcript retranscribe for this session.";

/// Warning `resumed_from_checkpoint`: the result continued an interrupted
/// run from its private chunk checkpoints (P10, ADR 0020).
pub const RESUMED_FROM_CHECKPOINT_WARNING: &str = "resumed_from_checkpoint: The retranscription continued an interrupted run. Chunks recognised before the interruption were taken from its private checkpoints instead of being decoded again; the revision is the one an uninterrupted run gives.";

/// Warning `checkpoint_discarded`: a stored checkpoint could not be used and
/// its chunk was recognised again.
pub const CHECKPOINT_DISCARDED_WARNING: &str = "checkpoint_discarded: A stored checkpoint of the interrupted run could not be used (damaged, incomplete or not this run's), so it was removed and its chunk recognised again. Nothing was guessed.";

/// Warning `cancellation_too_late`: the job had already committed, or was
/// committing, when cancellation was requested, so its result stands.
pub const CANCELLATION_TOO_LATE_WARNING: &str = "cancellation_too_late: The job was already committing or had committed when cancellation was requested, so its result stands and nothing was cancelled.";

/// Remediation when an operation id is reused for a different request.
pub const IDEMPOTENCY_CONFLICT_REMEDIATION: &str = "The operation id was used earlier in this session for a different request, whose result is kept. Nothing was changed. Retry the original request with that id, or send this request with a new operation id.";

/// Remediation when a job is running in another process.
pub const JOB_BUSY_REMEDIATION: &str = "The same retranscription is running in another process; affected_ids names its job. Nothing was changed. Retry after retry_after_ms: a retry of the same request continues the job or returns its result.";

/// Remediation when another revision replaced part of the range during the run.
pub const SUPERSEDED_REMEDIATION: &str = "Another revision changed the transcript around the requested range while this one ran, so the range it recognised no longer matches. Nothing was committed. Run the request again: it is widened over the newest revision.";

/// Fixed-prose warnings for how a retranscription used its checkpoints, in
/// a stable order.
#[must_use]
pub fn job_warning_messages(chunks_reused: u32, checkpoints_discarded: u32) -> Vec<&'static str> {
    let mut warnings = Vec::new();
    if chunks_reused > 0 {
        warnings.push(RESUMED_FROM_CHECKPOINT_WARNING);
    }
    if checkpoints_discarded > 0 {
        warnings.push(CHECKPOINT_DISCARDED_WARNING);
    }
    warnings
}

/// Structured remediation for a local speech-recognition run that failed.
///
/// The summary starts with `Local speech recognition failed at the <stage>
/// step (<reason>).`, then fixed prose chosen by the reason.
#[must_use]
pub fn local_asr_failure_summary(failure: AsrFailure) -> String {
    let (explanation, next_step) = failure_prose(failure.reason);
    format!(
        "Local speech recognition failed at the {} step ({}). {explanation} {next_step}",
        failure.stage.identifier(),
        failure.reason.identifier()
    )
}

/// Structured remediation for a local speech-recognition verification that
/// did not pass.
///
/// The summary starts with `VSift's local speech-recognition check failed
/// (<kind>).`, and for a failed transcription also names its stage and reason.
#[must_use]
pub fn local_asr_verification_summary(failure: LocalAsrVerificationFailure) -> String {
    let lead = "VSift's local speech-recognition check, which transcribes a short reviewed speech clip built into VSift before touching your video, failed";
    match failure {
        LocalAsrVerificationFailure::Transcription(asr) => {
            let (explanation, next_step) = verification_prose(asr.reason);
            format!(
                "{lead} (transcription) at the {} step ({}). {explanation} {next_step}",
                asr.stage.identifier(),
                asr.reason.identifier()
            )
        }
        other => {
            let (explanation, next_step) = match other {
                LocalAsrVerificationFailure::FixtureIntegrity => (
                    "The speech clip built into VSift failed its integrity check.",
                    "Nothing was changed. Reinstall VSift from a trusted release.",
                ),
                LocalAsrVerificationFailure::Workspace => (
                    "VSift could not prepare its private verification workspace in the per-user VSift directory.",
                    "Nothing was changed. Check that the per-user VSift directory is private to you, writable and has free space, then retry.",
                ),
                LocalAsrVerificationFailure::FixtureMedia => (
                    "FFprobe could not describe the built-in speech clip or found no audio in it.",
                    MEDIA_TOOL_REMEDY,
                ),
                LocalAsrVerificationFailure::UnexpectedTranscript => (
                    "whisper.cpp ran but did not recognise the clip's known words at their known times.",
                    WHISPER_REMEDY,
                ),
                LocalAsrVerificationFailure::Transcription(_) => ("", ""),
            };
            format!("{lead} ({}). {explanation} {next_step}", other.identifier())
        }
    }
}

const WHISPER_REMEDY: &str = "Nothing was committed. Reinstall the reviewed whisper.cpp v1.9.2 CLI or register a working build with setup configure whisper --executable <path>, then retry.";
const MEDIA_TOOL_REMEDY: &str = "Nothing was changed. Reinstall FFmpeg and FFprobe from a trusted build or register a working pair with setup configure, then retry.";

/// The prose for a recognition that failed during `setup check`. It is the
/// request's, except for segments that mostly did not fit their audio: the clip
/// is a whole, reviewed one with no range to widen, so a tool that cannot
/// transcribe it is the problem and reinstalling it is the answer.
const fn verification_prose(reason: AsrFailureReason) -> (&'static str, &'static str) {
    match reason {
        AsrFailureReason::MalformedOutput(ProviderOutputError::TooManyRejectedSegments) => (
            "whisper.cpp ran on the clip, but most of the segments it returned did not fit the clip's audio.",
            WHISPER_REMEDY,
        ),
        other => failure_prose(other),
    }
}

const fn failure_prose(reason: AsrFailureReason) -> (&'static str, &'static str) {
    match reason {
        AsrFailureReason::InvalidRange => (
            "The requested range lies outside the video.",
            "Nothing was committed. Choose --from and --to within the video's duration.",
        ),
        AsrFailureReason::TooManyChunks => (
            "The range needs more than 1,024 thirty-second audio chunks, the most one run processes.",
            "Nothing was committed. Retranscribe a shorter range with --from and --to.",
        ),
        AsrFailureReason::ModelChanged => (
            "The whisper.cpp executable or model differed from the one identified before the run, or changed during it, so output from two models was not mixed.",
            "Nothing was committed. Retry once the files are no longer being changed.",
        ),
        AsrFailureReason::ModelUnavailable => (
            "The registered model file could not be read.",
            "Nothing was committed. Register a readable reviewed model with setup configure-model --file <path>, then retry.",
        ),
        AsrFailureReason::UnpinnedModel => (
            "The registered model is not a reviewed pinned profile, so it was not run.",
            "Nothing was committed. Register the reviewed multilingual base model (ggml-base.bin) with setup configure-model --file <path>, then retry.",
        ),
        AsrFailureReason::Cancelled => (
            "The run was cancelled.",
            "Nothing was committed. Retry the command.",
        ),
        AsrFailureReason::Deadline => (
            "A step did not finish within its deadline; recognition allows 120 seconds per 30-second chunk.",
            "Nothing was committed. Retry when the machine is less busy, or retranscribe a shorter range.",
        ),
        AsrFailureReason::Busy => (
            "VSift's processing capacity for this session root was in use.",
            "Nothing was committed. Retry shortly.",
        ),
        AsrFailureReason::ResourceLimit => (
            "A step produced more output than VSift permits.",
            "Nothing was committed. Retranscribe a shorter range; if it persists, the audio may be unusual.",
        ),
        AsrFailureReason::AbnormalTermination => (
            "whisper.cpp ended abnormally, which usually means it ran out of memory.",
            "Nothing was committed. Close other programs and retry, or retranscribe a shorter range; a lighter model profile is planned.",
        ),
        AsrFailureReason::AudioUnavailable => (
            "The video's audio stream could not be decoded.",
            "Nothing was committed. Check that the video plays with sound in another player.",
        ),
        AsrFailureReason::ProviderFailed => ("whisper.cpp exited with an error.", WHISPER_REMEDY),
        AsrFailureReason::UnparseableOutput => (
            "whisper.cpp's output was not the documented JSON.",
            WHISPER_REMEDY,
        ),
        // A range whose recognised segments mostly did not fit the audio it was
        // given (#274): the tool ran and answered, and a short range cut
        // mid-speech is the usual cause, so the first step is a larger range.
        // It is also the only signal of a recogniser answering with garbage for
        // a whole run, so the reinstall hint stays, behind the whole-video
        // retry.
        AsrFailureReason::MalformedOutput(ProviderOutputError::TooManyRejectedSegments) => (
            "whisper.cpp ran, but most of the segments it returned did not fit the audio it was given (they were empty, ran backwards, started at or after the audio's end, or lay outside the video).",
            "Nothing was committed. A range that ends mid-speech can cause this, so retry with a larger range, or without --from and --to to recognise the whole video. Reinstall the reviewed whisper.cpp v1.9.2 CLI or register a working build with setup configure whisper --executable <path> only if the whole video fails the same way.",
        ),
        AsrFailureReason::MalformedOutput(_) => (
            "whisper.cpp's output broke VSift's rules for recognised text, such as out-of-order segments or invalid scores.",
            WHISPER_REMEDY,
        ),
        AsrFailureReason::Workspace => (
            "VSift could not use the session's private work directory.",
            "Nothing was committed. Check that the session root is private to you, writable and has free space, then retry.",
        ),
        AsrFailureReason::Io => (
            "A tool could not be started, or the session's private copy of the video could not be read.",
            "Nothing was committed. Run setup check and session status, then retry.",
        ),
        AsrFailureReason::InvalidRun(_) => (
            "VSift assembled an invalid recognition run, which is a defect.",
            "Nothing was committed. Report the command you ran.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use vsift_application::{AsrFailure, AsrFailureReason, AsrStage, LocalAsrVerificationFailure};
    use vsift_domain::{ProviderOutputError, TranscriptRevisionError};

    use super::{
        LOCAL_ASR_MODEL_REMEDIATION, LOCAL_ASR_TOOLS_REMEDIATION, NO_AUDIO_STREAM_REMEDIATION,
        UNKNOWN_REVISION_REMEDIATION, UNPINNED_MODEL_REMEDIATION, local_asr_failure_summary,
        local_asr_verification_summary,
    };

    const REASONS: [AsrFailureReason; 18] = [
        AsrFailureReason::InvalidRange,
        AsrFailureReason::TooManyChunks,
        AsrFailureReason::ModelChanged,
        AsrFailureReason::ModelUnavailable,
        AsrFailureReason::UnpinnedModel,
        AsrFailureReason::Cancelled,
        AsrFailureReason::Deadline,
        AsrFailureReason::Busy,
        AsrFailureReason::ResourceLimit,
        AsrFailureReason::AbnormalTermination,
        AsrFailureReason::AudioUnavailable,
        AsrFailureReason::ProviderFailed,
        AsrFailureReason::UnparseableOutput,
        AsrFailureReason::MalformedOutput(ProviderOutputError::TooManySegments),
        AsrFailureReason::MalformedOutput(ProviderOutputError::TooManyRejectedSegments),
        AsrFailureReason::Workspace,
        AsrFailureReason::Io,
        AsrFailureReason::InvalidRun(TranscriptRevisionError::InvalidAsrRun),
    ];

    /// #274: recognised segments that mostly did not fit their audio are, for a
    /// request, most likely a range cut mid-speech, so the answer says to widen
    /// the range first; it is also the only signal of a recogniser answering with
    /// garbage for a whole run, so the reinstall step stays, behind the
    /// whole-video retry. Any other malformed output still says to reinstall.
    #[test]
    fn segments_that_did_not_fit_their_audio_say_to_widen_the_range_before_reinstalling() {
        let stage = AsrStage::OutputValidation;
        let fit = local_asr_failure_summary(AsrFailure {
            stage,
            reason: AsrFailureReason::MalformedOutput(ProviderOutputError::TooManyRejectedSegments),
        });
        assert!(fit.contains("(malformed_output)"), "{fit}");
        assert!(fit.contains("did not fit the audio"), "{fit}");
        // Every way a segment is rejected is named.
        for kind in [
            "empty",
            "ran backwards",
            "started at or after the audio's end",
            "outside the video",
        ] {
            assert!(fit.contains(kind), "{kind}: {fit}");
        }
        // The first step is a larger range or the whole video, and the
        // reinstall step comes after it and only on a whole-video failure.
        assert!(
            matches!(
                (
                    fit.find("without --from and --to"),
                    fit.find("Reinstall the reviewed whisper.cpp")
                ),
                (Some(retry), Some(reinstall)) if retry < reinstall
            ),
            "{fit}"
        );
        assert!(
            fit.contains("only if the whole video fails the same way"),
            "{fit}"
        );
        assert!(!fit.contains("nothing needs reinstalling"), "{fit}");
        let broken = local_asr_failure_summary(AsrFailure {
            stage,
            reason: AsrFailureReason::MalformedOutput(ProviderOutputError::OutOfOrderSegments),
        });
        assert!(
            broken.contains("Reinstall the reviewed whisper.cpp"),
            "{broken}"
        );
        assert!(!broken.contains("only if"), "{broken}");
    }

    /// The setup check transcribes a whole, built-in clip: there is no range to
    /// widen, so the same reason says to reinstall the tool.
    #[test]
    fn the_setup_check_never_tells_to_widen_a_range() {
        let summary = local_asr_verification_summary(LocalAsrVerificationFailure::Transcription(
            AsrFailure {
                stage: AsrStage::OutputValidation,
                reason: AsrFailureReason::MalformedOutput(
                    ProviderOutputError::TooManyRejectedSegments,
                ),
            },
        ));
        assert!(
            summary.contains("Reinstall the reviewed whisper.cpp"),
            "{summary}"
        );
        assert!(!summary.contains("--from"), "{summary}");
        assert!(!summary.contains("larger range"), "{summary}");
    }

    #[test]
    fn every_failure_names_its_stage_and_reason_within_the_schema_bound() {
        for reason in REASONS {
            for stage in [
                AsrStage::Planning,
                AsrStage::RecognizerIdentity,
                AsrStage::AudioExtraction,
                AsrStage::Recognition,
                AsrStage::OutputValidation,
                AsrStage::Assembly,
            ] {
                let failure = AsrFailure { stage, reason };
                for summary in [
                    local_asr_failure_summary(failure),
                    local_asr_verification_summary(LocalAsrVerificationFailure::Transcription(
                        failure,
                    )),
                ] {
                    assert!(summary.contains(&format!(
                        "at the {} step ({})",
                        stage.identifier(),
                        reason.identifier()
                    )));
                    assert!(!summary.is_empty() && summary.len() <= 1024, "{summary}");
                    assert!(!summary.contains(['/', '\\']) || summary.contains("ffmpeg|"));
                }
            }
        }
        for failure in [
            LocalAsrVerificationFailure::FixtureIntegrity,
            LocalAsrVerificationFailure::Workspace,
            LocalAsrVerificationFailure::FixtureMedia,
            LocalAsrVerificationFailure::UnexpectedTranscript,
        ] {
            let summary = local_asr_verification_summary(failure);
            assert!(summary.contains(&format!("({})", failure.identifier())));
            assert!(summary.len() <= 1024);
        }
        for constant in [
            LOCAL_ASR_TOOLS_REMEDIATION,
            LOCAL_ASR_MODEL_REMEDIATION,
            UNPINNED_MODEL_REMEDIATION,
            NO_AUDIO_STREAM_REMEDIATION,
            UNKNOWN_REVISION_REMEDIATION,
        ] {
            assert!(constant.len() <= 1024);
        }
    }
}
