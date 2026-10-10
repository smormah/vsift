//! Fixed-prose remediation for local speech recognition (`transcript
//! retranscribe`).
//!
//! Every summary is chosen by a typed value and names its identifiers in a
//! fixed first sentence, so an agent can act on them without parsing free
//! text. None contains a path, provider output or transcript text.

use std::borrow::Cow;

use vsift_application::{
    AsrFailure, AsrFailureReason, LocalAsrVerificationFailure, UnusableChunks,
};
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
/// request's, except where the request's prose would be false for the check:
/// the clip is a whole, reviewed one that a working tool transcribes, so a tool
/// that cannot is the problem and reinstalling it is the answer. There is no
/// range to change and no recording to blame, so it names neither a chunk's
/// position nor `--from` and `--to`, for any reason.
fn verification_prose(reason: AsrFailureReason) -> (Cow<'static, str>, &'static str) {
    let AsrFailureReason::MalformedOutput(UnusableChunks {
        unusable,
        answered,
        first,
        ..
    }) = reason
    else {
        return failure_prose(reason);
    };
    match first.error {
        ProviderOutputError::TooManyRejectedSegments => (
            Cow::Borrowed(
                "whisper.cpp ran on the clip, but most of the segments it returned did not fit the clip's audio.",
            ),
            WHISPER_REMEDY,
        ),
        ProviderOutputError::TooManySegments
        | ProviderOutputError::TooManyTokens
        | ProviderOutputError::OutOfOrderSegments
        | ProviderOutputError::InvalidTokenProbability => (
            Cow::Owned(format!(
                "The reason is {}: whisper.cpp ran on the clip, but {unusable} of the {answered} audio chunks it had answered had output VSift could not use ({}).",
                first.error.identifier(),
                refused_answer(first.error),
            )),
            WHISPER_REMEDY,
        ),
    }
}

/// The most chunks a failed run may have had answered for its prose to treat
/// the failure as one stretch of the recording that could not be read. The run
/// judges the chunks the recognizer answered, and with so few of them (three
/// chunks are at most 80 seconds of audio, and a majority of three is two) it
/// cannot tell a bad stretch from a bad recording. A different range cuts the
/// audio at other points, and a chunk's result depends on where it is cut, so
/// trying one is a reasonable next step. With more answers than this, most of
/// them unusable, the failure is about most of the speech the run covered, and
/// the prose does not suggest a range.
///
/// Why the answered chunks and not the planned ones or their share. The share
/// of a failed run is always above a half, so it tells nothing more; a plan
/// counts the quiet chunks, which say nothing about the recogniser (a long
/// recording with four stretches of speech, all unusable, plans many chunks and
/// answers four); and the answered chunks are the stretches of speech the run
/// had evidence about. What the prose says is about what the run covered
/// ("most of the speech this run covered"), which is the recording only when
/// the run was over the whole of it.
const FEW_ANSWERED_CHUNKS: u32 = 3;

/// The next step of a run whose recognizer answered for most of the chunks with
/// segments that could not be placed in their audio (#353), when there are
/// many of them. It does not tell to reinstall whisper.cpp, because
/// `VSift`'s own check of the tool passed before the run and the tool is not
/// what is wrong with this audio. What helps is a transcript the user has, which
/// needs no recognition. It does not say that a different range will not work
/// either, because with this many answers it has not been tried and is not
/// established either way.
const SUPPLIED_TRANSCRIPT_REMEDY: &str = "Nothing was committed. Use a transcript you already have instead: open the video with ingest --transcript <file> (SubRip or WebVTT), which needs no speech recognition.";

/// The next step of the same failure when only a few chunks were answered: the
/// stretch could not be read, a slightly different range cuts the audio
/// elsewhere and may work, and a transcript the user has still needs no
/// recognition.
const SHORT_STRETCH_REMEDY: &str = "Nothing was committed. Try a slightly different range with --from and --to, which cuts the audio at other points and may work, or use a transcript you already have: open the video with ingest --transcript <file> (SubRip or WebVTT), which needs no speech recognition.";

/// What it is about an answer that the rules refuse, for the chunks a failure
/// names.
const fn refused_answer(error: ProviderOutputError) -> &'static str {
    match error {
        ProviderOutputError::TooManyRejectedSegments => {
            "their segments were empty, ran backwards, started at or after the audio's end, ended beyond the 30-second window the recogniser works in, or lay outside the video"
        }
        ProviderOutputError::OutOfOrderSegments => "their segments were not in time order",
        ProviderOutputError::InvalidTokenProbability => "their scores were not numbers from 0 to 1",
        ProviderOutputError::TooManySegments => "they had more than 256 segments",
        ProviderOutputError::TooManyTokens => "a segment had more than 512 tokens",
    }
}

/// A source time as `H:MM:SS`, whole seconds rounded down.
fn clock(micros: u64) -> String {
    let seconds = micros / 1_000_000;
    format!(
        "{}:{:02}:{:02}",
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60
    )
}

/// The prose of a run that failed because most of the chunks the recognizer
/// answered were unusable (#353): which reason, how many chunks and the first
/// one, by position and by source time in the unit of `--from` and `--to`.
///
/// Numbers only: no path, no provider output and no transcript text. For the
/// rejected-segments reason it says what is true and no more: the tool works,
/// and either one stretch could not be read (a few chunks were answered, so a
/// slightly different range is worth trying, as is a transcript the user
/// supplies) or most of the speech the run covered could not be transcribed reliably (many
/// were, so it points to the supplied transcript only). For a structural fault
/// the recognizer itself is the suspect, and reinstalling it stays the next
/// step.
fn unusable_chunks_prose(details: UnusableChunks) -> (Cow<'static, str>, &'static str) {
    let UnusableChunks {
        unusable,
        answered,
        planned,
        first,
    } = details;
    let (from, to) = (
        first.window.start().as_micros(),
        first.window.end().as_micros(),
    );
    let explanation = format!(
        "The reason is {}: whisper.cpp ran, but {unusable} of the {answered} audio chunks it had answered when the run stopped had output VSift could not use ({}). The first is chunk {} of {planned}, {} to {} ({from} to {to} microseconds, the unit of --from and --to).",
        first.error.identifier(),
        refused_answer(first.error),
        first.index.saturating_add(1),
        clock(from),
        clock(to),
    );
    match first.error {
        ProviderOutputError::TooManyRejectedSegments if answered <= FEW_ANSWERED_CHUNKS => (
            Cow::Owned(format!(
                "{explanation} The tool itself works: VSift's own check of it passed before this run. This stretch of the recording could not be transcribed."
            )),
            SHORT_STRETCH_REMEDY,
        ),
        ProviderOutputError::TooManyRejectedSegments => (
            Cow::Owned(format!(
                "{explanation} The tool itself works: VSift's own check of it passed before this run. Most of the speech this run covered could not be transcribed reliably."
            )),
            SUPPLIED_TRANSCRIPT_REMEDY,
        ),
        ProviderOutputError::TooManySegments
        | ProviderOutputError::TooManyTokens
        | ProviderOutputError::OutOfOrderSegments
        | ProviderOutputError::InvalidTokenProbability => (Cow::Owned(explanation), WHISPER_REMEDY),
    }
}

fn failure_prose(reason: AsrFailureReason) -> (Cow<'static, str>, &'static str) {
    if let AsrFailureReason::MalformedOutput(details) = reason {
        return unusable_chunks_prose(details);
    }
    let (explanation, next_step) = fixed_prose(reason);
    (Cow::Borrowed(explanation), next_step)
}

const fn fixed_prose(reason: AsrFailureReason) -> (&'static str, &'static str) {
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
        // The run-level failure of most chunks' output being unusable has its
        // own prose, built from its numbers ([`unusable_chunks_prose`], #353);
        // this is the plain one a caller without them would get, and the one
        // the compiler needs for the match to be exhaustive.
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
    use vsift_application::{
        AsrFailure, AsrFailureReason, AsrStage, LocalAsrVerificationFailure, UnusableChunk,
        UnusableChunks,
    };
    use vsift_domain::{MediaTime, ProviderOutputError, TimeRange, TranscriptRevisionError};

    use super::{
        LOCAL_ASR_MODEL_REMEDIATION, LOCAL_ASR_TOOLS_REMEDIATION, NO_AUDIO_STREAM_REMEDIATION,
        UNKNOWN_REVISION_REMEDIATION, UNPINNED_MODEL_REMEDIATION, local_asr_failure_summary,
        local_asr_verification_summary,
    };

    type Built<T> = Result<T, Box<dyn std::error::Error>>;

    const SECOND: u64 = 1_000_000;

    /// The failure of a run whose first unusable chunk was chunk 66 of 83, the
    /// window 27:05 to 27:35, for `error`.
    fn most_unusable(error: ProviderOutputError) -> Built<AsrFailureReason> {
        Ok(AsrFailureReason::MalformedOutput(UnusableChunks {
            unusable: 43,
            answered: 66,
            planned: 83,
            first: UnusableChunk {
                index: 65,
                window: TimeRange::new(
                    MediaTime::from_micros(1_625 * SECOND),
                    MediaTime::from_micros(1_655 * SECOND),
                )?,
                error,
            },
        }))
    }

    /// Every reason, with the failure of most chunks unusable once for each
    /// reason it can name, and once with the largest numbers it can carry.
    fn reasons() -> Built<Vec<AsrFailureReason>> {
        let mut reasons = vec![
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
            AsrFailureReason::Workspace,
            AsrFailureReason::Io,
            AsrFailureReason::InvalidRun(TranscriptRevisionError::InvalidAsrRun),
        ];
        for error in ProviderOutputError::ALL {
            reasons.push(most_unusable(error)?);
        }
        reasons.push(AsrFailureReason::MalformedOutput(UnusableChunks {
            unusable: 1_024,
            answered: 1_024,
            planned: 1_024,
            first: UnusableChunk {
                index: 1_023,
                window: TimeRange::new(
                    MediaTime::from_micros(14_000 * SECOND),
                    MediaTime::from_micros(14_400 * SECOND),
                )?,
                error: ProviderOutputError::TooManyRejectedSegments,
            },
        }));
        Ok(reasons)
    }

    /// The same failure with the number of chunks the recognizer had answered
    /// when the run stopped: `unusable` of `answered`, the first being chunk 1
    /// of `planned`.
    fn answered_chunks(
        unusable: u32,
        answered: u32,
        planned: u32,
        error: ProviderOutputError,
    ) -> Built<AsrFailureReason> {
        Ok(AsrFailureReason::MalformedOutput(UnusableChunks {
            unusable,
            answered,
            planned,
            first: UnusableChunk {
                index: 0,
                window: TimeRange::new(
                    MediaTime::from_micros(0),
                    MediaTime::from_micros(30 * SECOND),
                )?,
                error,
            },
        }))
    }

    /// The prose of a failed run, in a form the assertions below can search.
    fn run_summary(reason: AsrFailureReason) -> String {
        local_asr_failure_summary(AsrFailure {
            stage: AsrStage::OutputValidation,
            reason,
        })
    }

    /// #353: a run in which most chunks' answers were refused for segments that
    /// could not be placed in their audio says what is true. The tool works, so
    /// it does not tell to reinstall it. With many chunks answered it says that
    /// most of the speech the run covered could not be transcribed reliably and points to a
    /// transcript the user supplies, and does not say that another range helps
    /// or that it does not: that is not established. It names the reason, the
    /// numbers and the first chunk by position and by the source time `--from`
    /// and `--to` take.
    #[test]
    fn rejected_segments_say_the_tool_works_and_point_to_a_supplied_transcript() -> Built<()> {
        let summary = local_asr_failure_summary(AsrFailure {
            stage: AsrStage::OutputValidation,
            reason: most_unusable(ProviderOutputError::TooManyRejectedSegments)?,
        });
        // The stage and reason of every failure, in the fixed first sentence.
        assert!(
            summary.starts_with(
                "Local speech recognition failed at the output_validation step (malformed_output)."
            ),
            "{summary}"
        );
        for named in [
            "too_many_rejected_segments",
            "43 of the 66 audio chunks",
            "chunk 66 of 83",
            "0:27:05 to 0:27:35",
            "1625000000 to 1655000000 microseconds",
            // Every way a segment is refused is named.
            "empty",
            "ran backwards",
            "started at or after the audio's end",
            "ended beyond the 30-second window the recogniser works in",
            "outside the video",
            // What is true, and the way through.
            "The tool itself works",
            "could not be transcribed reliably",
            "ingest --transcript <file>",
            "needs no speech recognition",
            "Nothing was committed",
        ] {
            assert!(summary.contains(named), "{named}: {summary}");
        }
        // The #274 answer is gone, and so is any claim about a range: with
        // this many answers it is not established either way.
        for old in [
            "retry with a larger range",
            "without --from and --to",
            "Reinstall the reviewed whisper.cpp",
            "setup configure whisper",
            "only if the whole video fails",
            "ends mid-speech",
            "will not change that",
            "different range",
            "larger range",
            "This stretch",
        ] {
            assert!(!summary.contains(old), "{old}: {summary}");
        }
        // It is the same prose for a run over the whole video or a range.
        assert!(!summary.contains(['/', '\\']));
        assert!(summary.len() <= 1_024, "{}", summary.len());
        Ok(())
    }

    /// #353: what the prose says depends on what is known. With three chunks
    /// answered or fewer the run cannot tell a bad stretch from a bad
    /// recording: it says the stretch could not be read, that a slightly
    /// different range cuts the audio elsewhere and may work, and that a
    /// transcript the user has needs no recognition. With four or more it says
    /// the speech could not be transcribed reliably and names only the
    /// transcript. Neither says that a range will not help, which no run has
    /// established, and neither says to reinstall the tool.
    #[test]
    fn the_next_step_depends_on_how_many_chunks_were_answered() -> Built<()> {
        let rejected = ProviderOutputError::TooManyRejectedSegments;
        for (unusable, answered, planned) in
            [(1, 1, 1), (2, 2, 3), (2, 3, 3), (3, 3, 5), (2, 3, 83)]
        {
            let summary = run_summary(answered_chunks(unusable, answered, planned, rejected)?);
            for named in [
                "This stretch of the recording could not be transcribed.",
                "Try a slightly different range with --from and --to",
                "may work",
                "ingest --transcript <file>",
                "The tool itself works",
            ] {
                assert!(
                    summary.contains(named),
                    "{unusable}/{answered}/{planned}: {named}: {summary}"
                );
            }
            for never in [
                "will not change that",
                "larger range",
                "could not be transcribed reliably",
                "Reinstall the reviewed whisper.cpp",
            ] {
                assert!(
                    !summary.contains(never),
                    "{unusable}/{answered}/{planned}: {never}: {summary}"
                );
            }
        }
        for (unusable, answered, planned) in
            [(3, 4, 4), (3, 5, 83), (41, 41, 83), (1_024, 1_024, 1_024)]
        {
            let summary = run_summary(answered_chunks(unusable, answered, planned, rejected)?);
            for named in [
                "Most of the speech this run covered could not be transcribed reliably.",
                "ingest --transcript <file>",
                "The tool itself works",
            ] {
                assert!(
                    summary.contains(named),
                    "{unusable}/{answered}/{planned}: {named}: {summary}"
                );
            }
            for never in [
                "will not change that",
                "larger range",
                "different range",
                "Try a slightly",
                "This stretch",
                "Reinstall the reviewed whisper.cpp",
            ] {
                assert!(
                    !summary.contains(never),
                    "{unusable}/{answered}/{planned}: {never}: {summary}"
                );
            }
            assert!(summary.len() <= 1_024, "{}", summary.len());
        }
        Ok(())
    }

    /// A structural fault in most chunks' answers means the recognizer itself
    /// misbehaves, so it keeps its own reason and the reinstall step, and does
    /// not claim the tool works or tell to supply a transcript.
    #[test]
    fn structural_faults_keep_the_reinstall_step() -> Built<()> {
        for error in [
            ProviderOutputError::OutOfOrderSegments,
            ProviderOutputError::InvalidTokenProbability,
            ProviderOutputError::TooManySegments,
            ProviderOutputError::TooManyTokens,
        ] {
            let broken = local_asr_failure_summary(AsrFailure {
                stage: AsrStage::OutputValidation,
                reason: most_unusable(error)?,
            });
            assert!(broken.contains(error.identifier()), "{broken}");
            assert!(broken.contains("(malformed_output)"), "{broken}");
            assert!(broken.contains("chunk 66 of 83"), "{broken}");
            assert!(
                broken.contains("Reinstall the reviewed whisper.cpp"),
                "{broken}"
            );
            assert!(!broken.contains("The tool itself works"), "{broken}");
            assert!(!broken.contains("--transcript"), "{broken}");
        }
        Ok(())
    }

    /// The setup check transcribes a whole, built-in clip: there is no range to
    /// change and no recording to blame, so every reason of a failed check says
    /// to reinstall the tool, whatever it says for a run, and none names a
    /// chunk's position, `--from` or `--to`, a range or a transcript to supply.
    #[test]
    fn the_setup_check_never_names_a_range_a_chunk_or_a_supplied_transcript() -> Built<()> {
        for error in ProviderOutputError::ALL {
            for (unusable, answered, planned) in [(1, 1, 1), (2, 3, 3), (4, 4, 6), (41, 41, 83)] {
                let summary = local_asr_verification_summary(
                    LocalAsrVerificationFailure::Transcription(AsrFailure {
                        stage: AsrStage::OutputValidation,
                        reason: answered_chunks(unusable, answered, planned, error)?,
                    }),
                );
                let named = format!("{error:?}: {summary}");
                assert!(
                    summary.contains("Reinstall the reviewed whisper.cpp"),
                    "{named}"
                );
                for never in [
                    "--from",
                    "--to",
                    "range",
                    "chunk 1 of",
                    "microseconds",
                    "--transcript",
                    "The tool itself works",
                    "This stretch",
                    "reliably",
                ] {
                    assert!(!summary.contains(never), "{never}: {named}");
                }
                assert!(summary.len() <= 1_024, "{named}");
            }
        }
        Ok(())
    }

    #[test]
    fn every_failure_names_its_stage_and_reason_within_the_schema_bound() -> Built<()> {
        for reason in reasons()? {
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
        Ok(())
    }
}
