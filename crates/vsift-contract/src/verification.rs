//! Fixed-prose remediation for a failed automatic media-tool preflight.

use vsift_application::{MediaToolFailure, MediaToolPreflightFailure};

/// Structured remediation for a media-tool preflight that did not pass.
///
/// The summary starts with a fixed sentence carrying the typed check and reason
/// identifiers, `... at the <check> step (<reason>).`, so an agent can act on
/// them without parsing free text, followed by fixed prose chosen by the reason.
/// It never contains paths or tool output.
#[must_use]
pub fn media_tool_verification_summary(failure: MediaToolPreflightFailure) -> String {
    let (explanation, next_step) = match failure.failure {
        MediaToolFailure::ProcessFailure => (
            "A tool could not be started or exited abnormally.",
            TOOL_REMEDY,
        ),
        MediaToolFailure::ProviderRejected => (
            "A tool rejected the request or could not decode the reviewed test video built into VSift, for example because the selected FFprobe is another program.",
            TOOL_REMEDY,
        ),
        MediaToolFailure::OutputLimit => (
            "A tool produced more output than VSift permits.",
            TOOL_REMEDY,
        ),
        MediaToolFailure::UnexpectedResult => (
            "The tools ran but their results differ from the built-in test video's known answers, for example because the selected FFprobe is another program.",
            TOOL_REMEDY,
        ),
        MediaToolFailure::Deadline => (
            "A tool did not finish the small built-in test video within its deadline.",
            "Nothing was changed. Retry when the machine is less busy; if it keeps failing, register a different FFmpeg and FFprobe with setup configure.",
        ),
        MediaToolFailure::Workspace => (
            "VSift could not prepare its private verification workspace in the per-user VSift directory.",
            "Nothing was changed. Check that the per-user VSift directory is private to you, writable and has free space, then retry.",
        ),
        MediaToolFailure::Cancelled => (
            "Verification was cancelled.",
            "Nothing was changed. Retry the operation.",
        ),
        MediaToolFailure::FixtureIntegrity => (
            "The test video built into VSift failed its integrity check.",
            "Nothing was changed. Reinstall VSift from a trusted release.",
        ),
    };
    format!(
        "The selected FFmpeg and FFprobe failed VSift's media-tool check at the {} step ({}). {explanation} {next_step}",
        failure.check.identifier(),
        failure.failure.identifier()
    )
}

const TOOL_REMEDY: &str = "Nothing was changed. Reinstall FFmpeg and FFprobe from a trusted build, or register a working pair with setup configure ffmpeg --executable <path> and setup configure ffprobe --executable <path>, then retry; the check runs again automatically.";

#[cfg(test)]
mod tests {
    use vsift_application::{MediaToolCheck, MediaToolFailure, MediaToolPreflightFailure};

    use super::media_tool_verification_summary;

    #[test]
    fn every_reason_names_its_check_and_stays_within_the_schema_bound() {
        for failure in [
            MediaToolFailure::FixtureIntegrity,
            MediaToolFailure::Workspace,
            MediaToolFailure::ProcessFailure,
            MediaToolFailure::ProviderRejected,
            MediaToolFailure::Deadline,
            MediaToolFailure::OutputLimit,
            MediaToolFailure::UnexpectedResult,
            MediaToolFailure::Cancelled,
        ] {
            for check in [
                MediaToolCheck::Preparation,
                MediaToolCheck::Probe,
                MediaToolCheck::Frame,
                MediaToolCheck::Audio,
                MediaToolCheck::VisualSampling,
            ] {
                let summary =
                    media_tool_verification_summary(MediaToolPreflightFailure { check, failure });
                let marker = format!(
                    "at the {} step ({}).",
                    check.identifier(),
                    failure.identifier()
                );
                assert!(summary.contains(&marker), "{summary}");
                assert!(summary.contains("Nothing was changed."), "{summary}");
                assert!(summary.len() <= 1024, "{summary}");
            }
        }
    }
}
