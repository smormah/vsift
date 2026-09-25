//! Fixed-prose remediation for local speech recognition (`transcript
//! retranscribe`).
//!
//! Every summary is chosen by a typed value and names its identifiers in a
//! fixed first sentence, so an agent can act on them without parsing free
//! text. None contains a path, provider output or transcript text.

use vsift_application::{AsrFailure, AsrFailureReason, LocalAsrVerificationFailure};

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
            let (explanation, next_step) = failure_prose(asr.reason);
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
        AsrFailureReason::MalformedOutput(_) => (
            "whisper.cpp's output broke VSift's rules for recognised text, such as segment times outside their audio or invalid scores.",
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

    const REASONS: [AsrFailureReason; 17] = [
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
        AsrFailureReason::Workspace,
        AsrFailureReason::Io,
        AsrFailureReason::InvalidRun(TranscriptRevisionError::InvalidAsrRun),
    ];

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
