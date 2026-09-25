//! Proves local speech recognition works by transcribing the reviewed F01
//! speech fixture through `VSift`'s real chain, and fingerprints a pass.
//!
//! A model file that hashes to a pinned profile and a whisper.cpp binary that
//! starts do not show that speech is recognised on this machine: a build may
//! lack a CPU feature, a model may not load, or output may be unparseable. The
//! verifier stages the embedded fixture in a fresh private session, decodes
//! its speech with the same bounded `FFmpeg` operation, runs the selected
//! recognizer with the pinned decoding profile, and applies the same parsing,
//! validation and seam merge a retranscription uses. It passes only if the
//! transcript contains the fixture's recorded words inside its recorded speech
//! window (ADR 0017). The fixture is embedded so the check works without the
//! repository.

use std::{fs, path::Path};

use sha2::{Digest, Sha256};
use vsift_application::{
    InitializeSessionStorage, InitializeSessionStorageRequest, LocalAsrVerification,
    LocalAsrVerificationFailure, LocalAsrVerifier, MediaToolFingerprint, RecognizerIdentity,
    TranscribeRangeRequest, transcribe_range, whole_file_source_segment,
};
use vsift_domain::{
    ArtifactIntegrity, ChunkPlan, DurabilityRequirement, MediaSelection, MergedSegment,
};

use crate::{
    FfmpegMedia, FfmpegSpeechAudio, FilesystemSessionStore, HostIsolation,
    MediaProviderConformance, MediaToolVerificationAuthority, ProcessCancellation,
    ProcessWorkingDirectory, SourceSnapshot, WhisperCli, WhisperSpeechRecognizer,
    media_tool_verification::{
        VerificationWorkspace, matches_integrity, random_operation_id, random_session_id,
    },
    media_tool_verification_cache::{executable_identity, field, isolation_tag, number},
};

/// The reviewed Kokoro-spoken F01 speech fixture, identical to
/// `fixtures/corpus/generated/F01-speech.mp4`.
const F01_SPEECH: &[u8] = include_bytes!("../../../fixtures/corpus/generated/F01-speech.mp4");
const F01_SPEECH_FILE_NAME: &str = "F01-speech.mp4";
/// Size and SHA-256 of the fixture, from `speech-provenance.json`.
const F01_SPEECH_BYTES: u64 = 72_990;
const F01_SPEECH_SHA256: &str = "f8222a928243160c8dbf5b1a9bc24277e49ac47c11fe5526826462877678e881";
/// The generator's speech window for F01, in source microseconds.
const F01_SPEECH_START_MICROS: u64 = 500_000;
const F01_SPEECH_END_MICROS: u64 = 4_675_000;
/// How far outside the speech window a recognised segment may reach: the
/// base model reports coarse times, but text half a second from any speech
/// was not placed at the audio that produced it.
const SPEECH_WINDOW_TOLERANCE_MICROS: u64 = 500_000;
/// Normalised phrases the transcript must contain. F01's script is "The
/// service status is healthy and the build is 2048."; these are its evidence
/// terms, which a working recognizer reproduces at any number formatting
/// (`2,048` normalises to `2048`).
const REQUIRED_PHRASES: [&str; 3] = ["service status", "healthy", "2048"];

/// Version of what a local-ASR verification checks. Bump it whenever the
/// fixture checks change meaning, so every recorded pass is re-verified.
pub const LOCAL_ASR_VERIFICATION_PROFILE: u32 = 1;
const FINGERPRINT_DOMAIN: &[u8] = b"vsift-local-asr-verification";

/// SHA-256 of the embedded speech fixture, bound into every fingerprint.
#[must_use]
pub fn local_asr_fixture_sha256() -> [u8; 32] {
    Sha256::digest(F01_SPEECH).into()
}

/// The files a local-ASR pass was produced with.
#[derive(Clone, Copy, Debug)]
pub enum LocalAsrFiles<'a> {
    /// A whisper.cpp executable and model file selected on this machine.
    Selected {
        /// whisper.cpp CLI executable.
        whisper: &'a Path,
        /// Model file.
        model: &'a Path,
    },
    /// A recognizer supplied by the embedding host, identified only by the
    /// identity it reports.
    HostSupplied,
}

/// Derives the identity a local-ASR verification pass is valid for.
///
/// It binds, in its own domain so it can never equal a media-tool
/// fingerprint: the media-tool pair's fingerprint (the fixture is decoded
/// with them), the whisper executable's canonical path and on-disk identity,
/// the model's canonical path and on-disk identity, the recognizer identity
/// (executable and model SHA-256, model profile, decoding profile and
/// threads), the R0 chunk plan, the fixture digest, host isolation, which
/// verifier produced the pass, the verification profile and the `VSift`
/// version. Returns `None` when a file's identity cannot be read; the
/// preflight then verifies without recording.
#[must_use]
pub fn local_asr_fingerprint(
    media_tools: &MediaToolFingerprint,
    files: LocalAsrFiles<'_>,
    identity: &RecognizerIdentity,
    host_isolation: HostIsolation,
    authority: MediaToolVerificationAuthority,
) -> Option<MediaToolFingerprint> {
    let mut hasher = Sha256::new();
    field(&mut hasher, FINGERPRINT_DOMAIN);
    field(&mut hasher, &LOCAL_ASR_VERIFICATION_PROFILE.to_le_bytes());
    field(&mut hasher, env!("CARGO_PKG_VERSION").as_bytes());
    field(&mut hasher, authority.tag());
    field(&mut hasher, isolation_tag(host_isolation));
    field(&mut hasher, media_tools.digest());
    field(&mut hasher, &local_asr_fixture_sha256());
    field(
        &mut hasher,
        identity.provider.provider().identifier().as_bytes(),
    );
    field(
        &mut hasher,
        identity.provider.executable_sha256().as_str().as_bytes(),
    );
    field(
        &mut hasher,
        identity.model.profile().identifier().as_bytes(),
    );
    field(&mut hasher, identity.model.sha256().as_str().as_bytes());
    field(&mut hasher, identity.decoding.identifier().as_bytes());
    number(&mut hasher, u64::from(identity.threads.get()));
    number(&mut hasher, ChunkPlan::R0.window_us());
    number(&mut hasher, ChunkPlan::R0.overlap_us());
    match files {
        LocalAsrFiles::Selected { whisper, model } => {
            field(&mut hasher, b"selected");
            executable_identity(&mut hasher, whisper)?;
            executable_identity(&mut hasher, model)?;
        }
        LocalAsrFiles::HostSupplied => field(&mut hasher, b"host_supplied"),
    }
    Some(MediaToolFingerprint::from_digest(hasher.finalize().into()))
}

/// Verifies a whisper.cpp CLI and model against the embedded speech fixture.
pub struct FixtureAsrVerifier {
    conformance: MediaProviderConformance,
    host_isolation: HostIsolation,
    workspace_parent: std::path::PathBuf,
    whisper: WhisperCli,
    expected: RecognizerIdentity,
    cancellation: ProcessCancellation,
}

impl FixtureAsrVerifier {
    /// Creates a verifier for an already identified recognizer.
    ///
    /// `expected` is the identity the caller read from `whisper`'s files; the
    /// fixture run checks it before and after, like any run. `workspace_parent`
    /// must be an existing private directory the host controls; each run
    /// creates, uses and removes one uniquely named child in it, named like a
    /// media-tool verification workspace so the same leftover sweep covers it.
    #[must_use]
    pub const fn new(
        conformance: MediaProviderConformance,
        host_isolation: HostIsolation,
        workspace_parent: std::path::PathBuf,
        whisper: WhisperCli,
        expected: RecognizerIdentity,
        cancellation: ProcessCancellation,
    ) -> Self {
        Self {
            conformance,
            host_isolation,
            workspace_parent,
            whisper,
            expected,
            cancellation,
        }
    }

    async fn run(
        &self,
        workspace: &VerificationWorkspace,
    ) -> Result<(), LocalAsrVerificationFailure> {
        let integrity = ArtifactIntegrity::from_sha256_hex(F01_SPEECH_BYTES, F01_SPEECH_SHA256)
            .map_err(|_| LocalAsrVerificationFailure::FixtureIntegrity)?;
        if !matches_integrity(F01_SPEECH, integrity) {
            return Err(LocalAsrVerificationFailure::FixtureIntegrity);
        }
        let workspace_failure = |()| LocalAsrVerificationFailure::Workspace;
        let fixture_path = workspace.path().join(F01_SPEECH_FILE_NAME);
        fs::write(&fixture_path, F01_SPEECH).map_err(|_| LocalAsrVerificationFailure::Workspace)?;
        let store_root = workspace.path().join("store");
        let session_id = random_session_id().map_err(workspace_failure)?;
        let initialization = FilesystemSessionStore::provision_default(&store_root)
            .map_err(|_| LocalAsrVerificationFailure::Workspace)?;
        InitializeSessionStorage::new(initialization)
            .execute(InitializeSessionStorageRequest::new(
                session_id.clone(),
                random_operation_id().map_err(workspace_failure)?,
                DurabilityRequirement::Ephemeral,
            ))
            .await
            .map_err(|_| LocalAsrVerificationFailure::Workspace)?;
        let store = FilesystemSessionStore::open_existing(&store_root)
            .map_err(|_| LocalAsrVerificationFailure::Workspace)?;
        let snapshot = SourceSnapshot::stage(
            &store,
            &session_id,
            &random_operation_id().map_err(workspace_failure)?,
            &fixture_path,
        )
        .map_err(|_| LocalAsrVerificationFailure::Workspace)?;
        let media = FfmpegMedia::new(self.conformance.clone(), self.host_isolation, &store);
        let description = media
            .probe(&snapshot, self.cancellation.clone())
            .await
            .map_err(|_| LocalAsrVerificationFailure::FixtureMedia)?;
        let stream = description
            .speech_audio_stream()
            .ok_or(LocalAsrVerificationFailure::FixtureMedia)?;
        let source = whole_file_source_segment(snapshot.id(), description.duration)
            .map_err(|_| LocalAsrVerificationFailure::FixtureMedia)?;
        let chunks = workspace.path().join("asr");
        fs::create_dir(&chunks).map_err(|_| LocalAsrVerificationFailure::Workspace)?;
        let chunks = ProcessWorkingDirectory::new(&chunks)
            .map_err(|_| LocalAsrVerificationFailure::Workspace)?;
        let recognizer =
            WhisperSpeechRecognizer::new(self.whisper.clone(), chunks, self.cancellation.clone());
        let audio = FfmpegSpeechAudio::new(
            &media,
            &snapshot,
            &description,
            MediaSelection {
                video: None,
                audio: Some(stream),
            },
            self.cancellation.clone(),
        );
        let transcription = transcribe_range(
            TranscribeRangeRequest {
                source_segment: &source,
                range: source.range(),
                plan: ChunkPlan::R0,
                audio_stream: stream,
                expected: &self.expected,
            },
            &audio,
            &recognizer,
            &self.cancellation,
        )
        .await
        .map_err(LocalAsrVerificationFailure::Transcription)?;
        check_transcript(&transcription.segments)
    }
}

impl LocalAsrVerifier for FixtureAsrVerifier {
    async fn verify(&self) -> LocalAsrVerification {
        if self.cancellation.is_cancelled() {
            return LocalAsrVerification::Failed(LocalAsrVerificationFailure::Transcription(
                vsift_application::AsrFailure {
                    stage: vsift_application::AsrStage::Planning,
                    reason: vsift_application::AsrFailureReason::Cancelled,
                },
            ));
        }
        let Ok(workspace) = VerificationWorkspace::create(&self.workspace_parent) else {
            return LocalAsrVerification::Failed(LocalAsrVerificationFailure::Workspace);
        };
        match self.run(&workspace).await {
            Ok(()) => LocalAsrVerification::Verified,
            Err(failure) => LocalAsrVerification::Failed(failure),
        }
    }
}

/// Whether the merged transcript says F01's evidence terms, inside F01's
/// speech window.
fn check_transcript(segments: &[MergedSegment]) -> Result<(), LocalAsrVerificationFailure> {
    let earliest = F01_SPEECH_START_MICROS.saturating_sub(SPEECH_WINDOW_TOLERANCE_MICROS);
    let latest = F01_SPEECH_END_MICROS.saturating_add(SPEECH_WINDOW_TOLERANCE_MICROS);
    let placed = segments.iter().all(|merged| {
        let range = merged.segment.range();
        range.start().as_micros() >= earliest && range.end().as_micros() <= latest
    });
    let text = normalised(
        &segments
            .iter()
            .map(|merged| merged.segment.text().text())
            .collect::<Vec<_>>()
            .join(" "),
    );
    if segments.is_empty()
        || !placed
        || !REQUIRED_PHRASES.iter().all(|phrase| text.contains(phrase))
    {
        return Err(LocalAsrVerificationFailure::UnexpectedTranscript);
    }
    Ok(())
}

/// Lowercase words with punctuation removed and single spaces between them.
fn normalised(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|character| character.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use sha2::{Digest, Sha256};
    use vsift_application::{
        LocalAsrVerificationFailure, MediaToolFingerprint, RecognizerIdentity,
    };
    use vsift_domain::{
        AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider, AsrProviderBuild, ChunkTime,
        CueText, MediaTime, MergedSegment, Sha256Hex, TimeRange,
    };

    use super::{
        F01_SPEECH, F01_SPEECH_BYTES, F01_SPEECH_SHA256, LocalAsrFiles, check_transcript,
        local_asr_fingerprint, normalised,
    };
    use crate::{HostIsolation, MediaToolVerificationAuthority};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn the_embedded_fixture_is_the_reviewed_speech_clip() {
        assert_eq!(u64::try_from(F01_SPEECH.len()).ok(), Some(F01_SPEECH_BYTES));
        let digest = Sha256::digest(F01_SPEECH);
        let expected: Vec<u8> = (0..F01_SPEECH_SHA256.len())
            .step_by(2)
            .filter_map(|index| {
                F01_SPEECH_SHA256
                    .get(index..index + 2)
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            })
            .collect();
        assert_eq!(digest.as_slice(), expected.as_slice());
    }

    /// Builds a merged segment the way the seam merge returns it. Drafts are
    /// only made by the domain's validation, so this goes through it.
    fn segments(
        spans: &[(u64, u64, &str)],
    ) -> Result<Vec<MergedSegment>, Box<dyn std::error::Error>> {
        use vsift_domain::{
            PlannedChunk, ProviderChunkOutput, ProviderSegment, SourceSegmentId,
            validate_chunk_output,
        };
        let window = TimeRange::new(MediaTime::from_micros(0), MediaTime::from_micros(6_000_000))?;
        let chunk = PlannedChunk::new(SourceSegmentId::parse("sgm_0123456789abcdef")?, 0, window);
        let mut provider = Vec::new();
        for (start, end, words) in spans {
            provider.push(ProviderSegment {
                start: ChunkTime::from_micros(*start),
                end: ChunkTime::from_micros(*end),
                text: Some(CueText::new((*words).to_owned(), (*words).to_owned())?),
                tokens: Vec::new(),
            });
        }
        let validated = validate_chunk_output(
            &chunk,
            window,
            window,
            ProviderChunkOutput {
                language: None,
                segments: provider,
            },
        )?;
        Ok(validated
            .segments()
            .iter()
            .map(|segment| MergedSegment {
                chunk: 0,
                segment: segment.clone(),
            })
            .collect())
    }

    #[test]
    fn the_transcript_must_say_the_terms_inside_the_speech_window() -> TestResult {
        let good = segments(&[(
            620_000,
            4_500_000,
            "The service status is healthy, and the build is 2,048.",
        )])?;
        assert_eq!(check_transcript(&good), Ok(()));
        let missing = segments(&[(620_000, 4_500_000, "The service status is healthy.")])?;
        let late = segments(&[(
            620_000,
            5_500_000,
            "The service status is healthy and the build is 2048.",
        )])?;
        for failing in [missing, late, Vec::new()] {
            assert_eq!(
                check_transcript(&failing),
                Err(LocalAsrVerificationFailure::UnexpectedTranscript)
            );
        }
        assert_eq!(
            normalised(" The  service-status, 2,048! "),
            "the servicestatus 2048"
        );
        Ok(())
    }

    #[test]
    fn fingerprints_bind_the_recognizer_and_differ_from_media_tool_passes() -> TestResult {
        let identity = RecognizerIdentity {
            provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(DIGEST)?),
            model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(DIGEST)?),
            decoding: AsrDecodingProfile::R0V1,
            threads: NonZeroU16::new(4).ok_or("zero")?,
        };
        let media = MediaToolFingerprint::from_digest([7; 32]);
        let fingerprint = |identity: &RecognizerIdentity, authority| {
            local_asr_fingerprint(
                &media,
                LocalAsrFiles::HostSupplied,
                identity,
                HostIsolation::ProcessOnly,
                authority,
            )
        };
        let base = fingerprint(&identity, MediaToolVerificationAuthority::HostSupplied)
            .ok_or("fingerprint")?;
        assert_ne!(base, media);
        assert_eq!(
            Some(base),
            fingerprint(&identity, MediaToolVerificationAuthority::HostSupplied)
        );
        assert_ne!(
            Some(base),
            fingerprint(&identity, MediaToolVerificationAuthority::ReviewedFixture)
        );
        let mut threads = identity.clone();
        threads.threads = NonZeroU16::MIN;
        assert_ne!(
            Some(base),
            fingerprint(&threads, MediaToolVerificationAuthority::HostSupplied)
        );
        let missing = std::env::temp_dir().join("vsift-no-such-whisper-cli");
        assert_eq!(
            local_asr_fingerprint(
                &media,
                LocalAsrFiles::Selected {
                    whisper: &missing,
                    model: &missing,
                },
                &identity,
                HostIsolation::ProcessOnly,
                MediaToolVerificationAuthority::ReviewedFixture,
            ),
            None
        );
        Ok(())
    }
}
