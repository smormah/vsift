//! The error remediations for a missing media tool or speech recogniser
//! (issue #396).
//!
//! `ingest --transcript`, `candidates`, the evidence commands and
//! `transcript retranscribe` answer `MISSING_CAPABILITY` when `FFmpeg`, `FFprobe`
//! or whisper.cpp is not available. Their remediations said "Install or locate
//! trusted builds", and a cold agent read "locate" as leave to search the
//! machine for them. They now name the command that registers each tool by
//! the path the user gives, and say that no folder search is needed. The
//! code, the authority and the shape of the remediation are unchanged.

use vsift_contract::{
    EVIDENCE_TOOLS_REMEDIATION, LOCAL_ASR_TOOLS_REMEDIATION,
    MEDIA_TOOLS_FOR_TRANSCRIPT_REMEDIATION, VISUAL_TOOLS_REMEDIATION,
};

/// The remediations with the `setup configure` dependencies each one names.
const REMEDIATIONS: [(&str, &str); 4] = [
    (VISUAL_TOOLS_REMEDIATION, "ffmpeg|ffprobe"),
    (EVIDENCE_TOOLS_REMEDIATION, "ffmpeg|ffprobe"),
    (MEDIA_TOOLS_FOR_TRANSCRIPT_REMEDIATION, "ffmpeg|ffprobe"),
    (LOCAL_ASR_TOOLS_REMEDIATION, "ffmpeg|ffprobe|whisper"),
];

#[test]
fn a_missing_tool_remediation_names_the_registration_and_needs_no_search() {
    for (remediation, dependencies) in REMEDIATIONS {
        assert!(
            remediation.contains(&format!(
                "Install trusted builds or register working ones with setup configure {dependencies} --executable <path>, then run setup check. No need to search folders: ask the user where each one is."
            )),
            "{remediation}"
        );
        assert!(
            !remediation.to_lowercase().contains("locate"),
            "{remediation}"
        );
        assert!(remediation.chars().count() <= 1024, "{remediation}");
    }
}
