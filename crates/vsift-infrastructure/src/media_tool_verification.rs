//! Proves selected media tools work by running the reviewed F01 fixture through
//! `VSift`'s real media pipeline, and identifies registered speech models.
//!
//! A version probe only shows that an executable responds. This verifier stages
//! the embedded fixture in a fresh private session, runs the same bounded P04
//! probe, frame and audio operations and the P08 visual sampling an
//! investigation uses, and compares each result with the fixture's recorded
//! truth. The fixture is embedded so the
//! check works on machines without the repository (ADR 0015).

use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use vsift_application::{
    InitializeSessionStorage, InitializeSessionStorageRequest, MediaToolCheck, MediaToolFailure,
    MediaToolVerification, MediaToolVerifier, ModelVerification, ReviewedCompatibilityPolicy,
};
use vsift_domain::{
    ArtifactIntegrity, CandidateReason, CandidateStability, DurabilityRequirement,
    MediaDescription, MediaSelection, MediaStreamKind, MediaTime, OperationId, ReviewedAsrModel,
    SessionId, TimeRange, VisualChangePolicy, VisualSample, VisualWindow, analyse_window,
};

use crate::{
    BoundSource, ExtractedAudio, ExtractedFrame, FfmpegMedia, FilesystemSessionStore,
    HostIsolation, ManagedCatalogueError, MediaError, MediaProviderConformance,
    ProcessCancellation, RawGrayFrame, SourceSnapshot, file_lock::HeldFileLock,
    reviewed_whisper_models,
};

/// The reviewed synthetic F01 fixture, identical to `fixtures/corpus/generated/F01.mp4`.
const F01: &[u8] = include_bytes!("../../../fixtures/corpus/generated/F01.mp4");
const F01_FILE_NAME: &str = "F01.mp4";
/// F01 recorded truth from `fixtures/corpus/manifest.json`.
const F01_DURATION_MICROS: u64 = 6_000_000;
const F01_WIDTH: u32 = 1280;
const F01_HEIGHT: u32 = 720;
const F01_SELECTION: MediaSelection = MediaSelection {
    video: Some(0),
    audio: Some(1),
};
/// Container duration may differ from the authored length by encoder padding.
const DURATION_TOLERANCE_MICROS: u64 = 100_000;
/// F01 is a constant 1280x720 slide, so a displayed frame exists at exactly this time.
const FRAME_AT_MICROS: u64 = 750_000;
const FRAME_TOLERANCE_MICROS: u64 = 500_000;
const AUDIO_SPAN_MICROS: u64 = 1_000_000;
/// Extracted PCM may be slightly shorter or longer than the span at codec edges.
const AUDIO_LENGTH_TOLERANCE_PERCENT: u64 = 10;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
/// F01's first visual window is the whole 6 s clip; at 20 frames per second
/// the 0.5 s sampler keeps exactly the frames at 0, 0.5, ..., 5.5 s.
const VISUAL_SAMPLE_COUNT: u64 = 12;
const VISUAL_SAMPLE_SPACING_MICROS: u64 = 500_000;
/// Block (row 4, column 7) lies inside F01's blue status panel and block 143
/// (the bottom-right corner) on its dark background. Measured with `FFmpeg`
/// 9.0 they are 72 and 26; the check only requires the panel to be clearly
/// brighter, so a build that outputs limited-range grey still passes.
const VISUAL_PANEL_BLOCK: usize = 4 * 16 + 7;
const VISUAL_BACKGROUND_BLOCK: usize = 143;
const VISUAL_MIN_CONTRAST: u8 = 20;
/// Every verification workspace is named this prefix followed by exactly
/// [`WORKSPACE_RANDOM_BYTES`] random bytes in lowercase hex, and nothing else.
const WORKSPACE_PREFIX: &str = "vsift-tool-verification-";
const WORKSPACE_RANDOM_BYTES: usize = 8;
/// File inside a workspace that its verification holds an exclusive lock on for
/// the whole run, so a sweep can tell a live workspace from a leftover.
pub(crate) const WORKSPACE_LOCK_FILE: &str = "workspace.lock";
const HASH_CHUNK_BYTES: usize = 1 << 20;

/// Verifies selected `FFmpeg`/`FFprobe` against the embedded reviewed fixture.
pub struct FixtureMediaToolVerifier {
    conformance: MediaProviderConformance,
    host_isolation: HostIsolation,
    workspace_parent: PathBuf,
    policy: ReviewedCompatibilityPolicy,
    cancellation: ProcessCancellation,
}

impl FixtureMediaToolVerifier {
    /// Creates a verifier for already trusted executables.
    ///
    /// `workspace_parent` must be an existing directory the host controls, such
    /// as the per-user session cache. Each run creates, uses and removes one
    /// fresh uniquely named child; it never adopts or deletes anything else.
    #[must_use]
    pub const fn new(
        conformance: MediaProviderConformance,
        host_isolation: HostIsolation,
        workspace_parent: PathBuf,
        policy: ReviewedCompatibilityPolicy,
        cancellation: ProcessCancellation,
    ) -> Self {
        Self {
            conformance,
            host_isolation,
            workspace_parent,
            policy,
            cancellation,
        }
    }

    async fn run(&self, workspace: &VerificationWorkspace) -> MediaToolVerification {
        match self.run_checks(workspace).await {
            Ok(()) => MediaToolVerification::Verified,
            Err((check, failure)) => MediaToolVerification::Failed { check, failure },
        }
    }

    async fn run_checks(
        &self,
        workspace: &VerificationWorkspace,
    ) -> Result<(), (MediaToolCheck, MediaToolFailure)> {
        let preparation = |failure| (MediaToolCheck::Preparation, failure);
        if !matches_integrity(F01, self.policy.fixture) {
            return Err(preparation(MediaToolFailure::FixtureIntegrity));
        }
        let fixture_path = workspace.path.join(F01_FILE_NAME);
        fs::write(&fixture_path, F01).map_err(|_| preparation(MediaToolFailure::Workspace))?;
        let store_root = workspace.path.join("store");
        let session_id =
            random_session_id().map_err(|()| preparation(MediaToolFailure::Workspace))?;
        let initialization = FilesystemSessionStore::provision_default(&store_root)
            .map_err(|_| preparation(MediaToolFailure::Workspace))?;
        InitializeSessionStorage::new(initialization)
            .execute(InitializeSessionStorageRequest::new(
                session_id.clone(),
                random_operation_id().map_err(|()| preparation(MediaToolFailure::Workspace))?,
                DurabilityRequirement::Ephemeral,
            ))
            .await
            .map_err(|_| preparation(MediaToolFailure::Workspace))?;
        let store = FilesystemSessionStore::open_existing(&store_root)
            .map_err(|_| preparation(MediaToolFailure::Workspace))?;
        let snapshot = SourceSnapshot::stage(
            &store,
            &session_id,
            &random_operation_id().map_err(|()| preparation(MediaToolFailure::Workspace))?,
            &fixture_path,
        )
        .map_err(|_| preparation(MediaToolFailure::Workspace))?;
        let media = FfmpegMedia::new(self.conformance.clone(), self.host_isolation, &store);

        let description = media
            .probe(&snapshot, self.cancellation.clone())
            .await
            .map_err(|error| (MediaToolCheck::Probe, map_media_error(&error)))?;
        check_description(&description).map_err(|failure| (MediaToolCheck::Probe, failure))?;

        let frame = media
            .frame(
                &snapshot,
                &description,
                F01_SELECTION,
                MediaTime::from_micros(FRAME_AT_MICROS),
                FRAME_TOLERANCE_MICROS,
                self.cancellation.clone(),
            )
            .await
            .map_err(|error| (MediaToolCheck::Frame, map_media_error(&error)))?;
        check_frame(&frame).map_err(|failure| (MediaToolCheck::Frame, failure))?;

        let range = TimeRange::new(
            MediaTime::from_micros(0),
            MediaTime::from_micros(AUDIO_SPAN_MICROS),
        )
        .map_err(|_| (MediaToolCheck::Audio, MediaToolFailure::FixtureIntegrity))?;
        let audio = media
            .audio(
                &snapshot,
                &description,
                F01_SELECTION,
                range,
                self.cancellation.clone(),
            )
            .await
            .map_err(|error| (MediaToolCheck::Audio, map_media_error(&error)))?;
        check_audio(&audio, &self.policy).map_err(|failure| (MediaToolCheck::Audio, failure))?;

        // Visual indexing decodes window after window from a bound copy, so
        // the check does too: the copy is hashed once more here.
        let visual = |failure| (MediaToolCheck::VisualSampling, failure);
        let bound = BoundSource::bind(snapshot).map_err(|_| visual(MediaToolFailure::Workspace))?;
        let window = VisualWindow::new(0, description.duration)
            .map_err(|_| visual(MediaToolFailure::UnexpectedResult))?;
        let frames = media
            .visual_samples(
                &bound,
                &description,
                F01_SELECTION,
                window,
                self.cancellation.clone(),
            )
            .await
            .map_err(|error| visual(map_media_error(&error)))?;
        check_visual_samples(window, &frames).map_err(visual)
    }
}

impl MediaToolVerifier for FixtureMediaToolVerifier {
    async fn verify(&self) -> MediaToolVerification {
        let Ok(workspace) = VerificationWorkspace::create(&self.workspace_parent) else {
            return MediaToolVerification::Failed {
                check: MediaToolCheck::Preparation,
                failure: MediaToolFailure::Workspace,
            };
        };
        self.run(&workspace).await
    }
}

/// Checks a registered model file against the reviewed pinned models.
///
/// A size that matches no pin is decided without reading the file, so an
/// unrelated large file is not hashed. A matching size is hashed in bounded
/// chunks, and the profile is the pin whose size and digest both match.
#[must_use]
pub fn verify_model_file(
    path: &Path,
    pinned: &[(ReviewedAsrModel, ArtifactIntegrity)],
) -> ModelVerification {
    let Ok(file) = fs::File::open(path) else {
        return ModelVerification::Unreadable;
    };
    let Ok(metadata) = file.metadata() else {
        return ModelVerification::Unreadable;
    };
    if !metadata.is_file() {
        return ModelVerification::Unreadable;
    }
    let size = metadata.len();
    if !pinned
        .iter()
        .any(|(_, integrity)| integrity.bytes() == size)
    {
        return ModelVerification::Unrecognised;
    }
    match bounded_sha256(file, size) {
        Ok(Some(digest)) => pinned
            .iter()
            .find(|(_, integrity)| integrity.bytes() == size && integrity.sha256() == digest)
            .map_or(ModelVerification::Unrecognised, |(profile, _)| {
                ModelVerification::KnownPinned(*profile)
            }),
        Ok(None) => ModelVerification::Unrecognised,
        Err(_) => ModelVerification::Unreadable,
    }
}

/// Identifies a registered model file against every reviewed pinned
/// whisper.cpp model (maintainer decisions D5 and D6).
///
/// # Errors
///
/// Fails only when the built-in reviewed model pins are malformed.
pub fn identify_whisper_model_file(
    path: &Path,
) -> Result<ModelVerification, ManagedCatalogueError> {
    let pins = reviewed_whisper_models()?.map(|model| (model.profile, model.integrity));
    Ok(verify_model_file(path, &pins))
}

/// Returns the digest when exactly `expected_bytes` were read, or `None` otherwise.
fn bounded_sha256(file: fs::File, expected_bytes: u64) -> io::Result<Option<[u8; 32]>> {
    let mut reader = file.take(expected_bytes.saturating_add(1));
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; HASH_CHUNK_BYTES];
    let mut total: u64 = 0;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let chunk = buffer
            .get(..read)
            .ok_or_else(|| io::Error::other("read overrun"))?;
        hasher.update(chunk);
        total = total.saturating_add(u64::try_from(read).map_err(io::Error::other)?);
    }
    Ok((total == expected_bytes).then(|| hasher.finalize().into()))
}

pub(crate) fn matches_integrity(bytes: &[u8], integrity: ArtifactIntegrity) -> bool {
    u64::try_from(bytes.len()).is_ok_and(|length| length == integrity.bytes())
        && <[u8; 32]>::from(Sha256::digest(bytes)) == integrity.sha256()
}

fn check_description(description: &MediaDescription) -> Result<(), MediaToolFailure> {
    let unexpected = MediaToolFailure::UnexpectedResult;
    if description
        .duration
        .as_micros()
        .abs_diff(F01_DURATION_MICROS)
        > DURATION_TOLERANCE_MICROS
    {
        return Err(unexpected);
    }
    description
        .validate_selection(F01_SELECTION)
        .map_err(|_| unexpected)?;
    let video = description
        .streams
        .iter()
        .find(|stream| Some(stream.index) == F01_SELECTION.video)
        .ok_or(unexpected)?;
    let displayed = video
        .rotation
        .displayed_dimensions(video.encoded_dimensions.ok_or(unexpected)?);
    let audio_present = description.streams.iter().any(|stream| {
        Some(stream.index) == F01_SELECTION.audio && stream.kind == MediaStreamKind::Audio
    });
    if video.kind != MediaStreamKind::Video
        || (displayed.width(), displayed.height()) != (F01_WIDTH, F01_HEIGHT)
        || !audio_present
    {
        return Err(unexpected);
    }
    Ok(())
}

fn check_frame(frame: &ExtractedFrame) -> Result<(), MediaToolFailure> {
    if frame.timing.actual().as_micros() != FRAME_AT_MICROS
        || (frame.dimensions.width(), frame.dimensions.height()) != (F01_WIDTH, F01_HEIGHT)
        || !frame.png.starts_with(PNG_SIGNATURE)
    {
        return Err(MediaToolFailure::UnexpectedResult);
    }
    Ok(())
}

/// Checks F01's visual samples: the exact time grid, a static screen (one
/// settled candidate, no change between samples) and a plausible image.
fn check_visual_samples(
    window: VisualWindow,
    frames: &[RawGrayFrame],
) -> Result<(), MediaToolFailure> {
    let unexpected = MediaToolFailure::UnexpectedResult;
    let expected_times = (0..VISUAL_SAMPLE_COUNT).map(|step| step * VISUAL_SAMPLE_SPACING_MICROS);
    if u64::try_from(frames.len()).ok() != Some(VISUAL_SAMPLE_COUNT)
        || !frames
            .iter()
            .map(|frame| frame.time.as_micros())
            .eq(expected_times)
    {
        return Err(unexpected);
    }
    let samples: Vec<VisualSample> = frames
        .iter()
        .map(|frame| VisualSample::from_gray(frame.time, &frame.pixels))
        .collect();
    let analysis =
        analyse_window(window, &samples, VisualChangePolicy::R0).map_err(|_| unexpected)?;
    let [candidate] = analysis.candidates.as_slice() else {
        return Err(unexpected);
    };
    let first = samples.first().ok_or(unexpected)?;
    let panel = first.blocks().get(VISUAL_PANEL_BLOCK).ok_or(unexpected)?;
    let background = first
        .blocks()
        .get(VISUAL_BACKGROUND_BLOCK)
        .ok_or(unexpected)?;
    if candidate.reason != CandidateReason::FirstFrame
        || candidate.stability != CandidateStability::Settled
        || u64::from(candidate.sample_count) != VISUAL_SAMPLE_COUNT
        || panel.saturating_sub(*background) < VISUAL_MIN_CONTRAST
    {
        return Err(unexpected);
    }
    Ok(())
}

fn check_audio(
    audio: &ExtractedAudio,
    policy: &ReviewedCompatibilityPolicy,
) -> Result<(), MediaToolFailure> {
    let unexpected = MediaToolFailure::UnexpectedResult;
    if audio.sample_rate != policy.audio_sample_rate_hz
        || audio.channels != policy.audio_channels
        || audio.actual_start.as_micros() != 0
    {
        return Err(unexpected);
    }
    let length = u64::try_from(audio.pcm_s16le.len()).map_err(|_| MediaToolFailure::OutputLimit)?;
    if length > policy.audio_file_limit_bytes {
        return Err(MediaToolFailure::OutputLimit);
    }
    // Signed 16-bit PCM: two bytes per sample per channel.
    let expected = u64::from(policy.audio_sample_rate_hz)
        * u64::from(policy.audio_channels)
        * 2
        * AUDIO_SPAN_MICROS
        / 1_000_000;
    let tolerance = expected * AUDIO_LENGTH_TOLERANCE_PERCENT / 100;
    if length % 2 != 0 || length.abs_diff(expected) > tolerance {
        return Err(unexpected);
    }
    Ok(())
}

/// Maps every media failure to a stable verification reason; exhaustive by design.
const fn map_media_error(error: &MediaError) -> MediaToolFailure {
    match error {
        MediaError::Request(_) | MediaError::Process(_) => MediaToolFailure::ProcessFailure,
        MediaError::ProviderRejected => MediaToolFailure::ProviderRejected,
        MediaError::Deadline => MediaToolFailure::Deadline,
        MediaError::Cancelled => MediaToolFailure::Cancelled,
        MediaError::OutputLimit => MediaToolFailure::OutputLimit,
        MediaError::Source(_) | MediaError::InvalidSourcePath | MediaError::CapacityUnavailable => {
            MediaToolFailure::Workspace
        }
        MediaError::InvalidMetadata
        | MediaError::InvalidDuration
        | MediaError::InvalidAudioRange
        | MediaError::TooManyStreams
        | MediaError::InvalidDimensions
        | MediaError::InvalidOrientation
        | MediaError::InvalidTimeline
        | MediaError::StreamUnavailable
        | MediaError::UnsupportedCodec
        | MediaError::NoFrameWithinTolerance
        | MediaError::InvalidDecodedOutput
        | MediaError::NoDecodedAudio
        | MediaError::InvalidVisualWindow => MediaToolFailure::UnexpectedResult,
    }
}

pub(crate) fn random_session_id() -> Result<SessionId, ()> {
    SessionId::parse(format!("ses_{}", random_hex()?)).map_err(|_| ())
}

pub(crate) fn random_operation_id() -> Result<OperationId, ()> {
    OperationId::parse(format!("op_{}", random_hex()?)).map_err(|_| ())
}

fn random_hex() -> Result<String, ()> {
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).map_err(|_| ())?;
    Ok(lowercase_hex(&random))
}

/// Reports whether `name` is exactly a verification workspace name.
///
/// A sweep of leftover workspaces removes only positively identified names, so
/// a look-alike (other length, uppercase or extra characters) is never touched.
pub(crate) fn is_verification_workspace_name(name: &str) -> bool {
    name.strip_prefix(WORKSPACE_PREFIX).is_some_and(|random| {
        random.len() == WORKSPACE_RANDOM_BYTES * 2
            && random
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

/// A fresh, uniquely named directory owned by one verification run.
///
/// It is created with `create_dir`, so an existing path is never adopted, and it
/// is removed on drop. Standard-library removal does not follow symbolic links.
///
/// A process killed mid-verification never runs the drop, so the workspace
/// also holds an exclusive lock on [`WORKSPACE_LOCK_FILE`] for its whole life.
/// The operating system releases that lock when the process ends, which lets a
/// later sweep tell a leftover from a workspace still in use (issue #132).
pub(crate) struct VerificationWorkspace {
    path: PathBuf,
    owner: Option<HeldFileLock>,
}

impl VerificationWorkspace {
    /// Creates a fresh workspace in `parent` and takes its lock.
    ///
    /// # Errors
    ///
    /// Fails when randomness, the directory or its lock is unavailable; any
    /// directory already created is removed again.
    pub(crate) fn create(parent: &Path) -> io::Result<Self> {
        let mut random = [0_u8; WORKSPACE_RANDOM_BYTES];
        getrandom::fill(&mut random).map_err(|_| io::Error::other("randomness unavailable"))?;
        let path = parent.join(format!("{WORKSPACE_PREFIX}{}", lowercase_hex(&random)));
        fs::create_dir(&path)?;
        // From here on, dropping the value removes the directory again.
        let mut workspace = Self { path, owner: None };
        let lock = fs::File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(workspace.path.join(WORKSPACE_LOCK_FILE))?;
        workspace.owner = Some(HeldFileLock::try_exclusive(lock).map_err(io::Error::from)?);
        Ok(workspace)
    }

    /// The workspace directory.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for VerificationWorkspace {
    fn drop(&mut self) {
        // Windows cannot remove a directory holding an open file, so the lock
        // is released and closed first. Nothing adopts an existing workspace,
        // so no other verification can start using it in between.
        if let Some(owner) = self.owner.take() {
            let _ = owner.release();
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use sha2::{Digest, Sha256};
    use vsift_application::{MediaToolFailure, ModelVerification};
    use vsift_domain::{
        ArtifactIntegrity, DisplayRotation, FrameDimensions, FrameTiming, MediaDecodeSupport,
        MediaDescription, MediaStream, MediaStreamKind, MediaTime, ReviewedAsrModel, TimeRange,
    };

    use super::{
        AUDIO_SPAN_MICROS, F01, F01_DURATION_MICROS, F01_HEIGHT, F01_WIDTH, FRAME_AT_MICROS,
        PNG_SIGNATURE, VerificationWorkspace, WORKSPACE_LOCK_FILE, check_audio, check_description,
        check_frame, check_visual_samples, is_verification_workspace_name, lowercase_hex,
        matches_integrity, verify_model_file,
    };
    use crate::{
        ExtractedAudio, ExtractedFrame, RawGrayFrame, file_lock::HeldFileLock,
        reviewed_compatibility_policy,
    };

    type TestResult = Result<(), Box<dyn Error>>;

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Result<Self, Box<dyn Error>> {
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "vsift-tool-verification-test-{}-{nanos}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path)?;
            Ok(Self(path))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn stream(
        index: u32,
        kind: MediaStreamKind,
        dimensions: Option<(u32, u32)>,
    ) -> Result<MediaStream, Box<dyn Error>> {
        Ok(MediaStream {
            index,
            kind,
            codec: String::from(if kind == MediaStreamKind::Video {
                "h264"
            } else {
                "aac"
            }),
            decode_support: MediaDecodeSupport::Supported,
            time_base_numerator: 1,
            time_base_denominator: 1000,
            start_pts: Some(0),
            encoded_dimensions: dimensions
                .map(|(width, height)| FrameDimensions::new(width, height))
                .transpose()?,
            rotation: DisplayRotation::Zero,
            language: None,
        })
    }

    fn f01_description() -> Result<MediaDescription, Box<dyn Error>> {
        Ok(MediaDescription {
            duration: MediaTime::from_micros(F01_DURATION_MICROS),
            origin_micros: 0,
            streams: vec![
                stream(0, MediaStreamKind::Video, Some((F01_WIDTH, F01_HEIGHT)))?,
                stream(1, MediaStreamKind::Audio, None)?,
            ],
        })
    }

    fn frame(actual_micros: u64, png: &[u8]) -> Result<ExtractedFrame, Box<dyn Error>> {
        Ok(ExtractedFrame {
            stream_index: 0,
            timing: FrameTiming::new(
                MediaTime::from_micros(FRAME_AT_MICROS),
                MediaTime::from_micros(actual_micros),
            )?,
            dimensions: FrameDimensions::new(F01_WIDTH, F01_HEIGHT)?,
            png: png.to_vec(),
        })
    }

    fn audio(sample_rate: u32, bytes: usize) -> Result<ExtractedAudio, Box<dyn Error>> {
        Ok(ExtractedAudio {
            stream_index: 1,
            requested: TimeRange::new(
                MediaTime::from_micros(0),
                MediaTime::from_micros(AUDIO_SPAN_MICROS),
            )?,
            actual_start: MediaTime::from_micros(0),
            sample_rate,
            channels: 1,
            pcm_s16le: vec![0; bytes],
        })
    }

    #[test]
    fn workspace_names_are_recognised_exactly() {
        assert!(is_verification_workspace_name(
            "vsift-tool-verification-0123456789abcdef"
        ));
        for name in [
            "vsift-tool-verification-",
            "vsift-tool-verification-0123456789abcde",
            "vsift-tool-verification-0123456789abcdef0",
            "vsift-tool-verification-0123456789ABCDEF",
            "vsift-tool-verification-0123456789abcdeg",
            "vsift-tool-verification-test-1-2-3",
            "vsift-tool-verification",
            "VSIFT-TOOL-VERIFICATION-0123456789abcdef",
            "x-vsift-tool-verification-0123456789abcdef",
        ] {
            assert!(!is_verification_workspace_name(name), "{name}");
        }
    }

    /// Issue #132: a workspace holds its lock for its whole life, so a sweep
    /// can tell it from a leftover, and still removes itself when dropped.
    #[test]
    fn a_workspace_holds_its_lock_until_it_removes_itself() -> TestResult {
        let parent = TempDir::new()?;
        let workspace = VerificationWorkspace::create(&parent.0)?;
        let path = workspace.path().to_path_buf();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("workspace name is not UTF-8")?;
        assert!(is_verification_workspace_name(name));
        let probe = fs::File::open(path.join(WORKSPACE_LOCK_FILE))?;
        assert!(matches!(
            HeldFileLock::try_exclusive(probe),
            Err(fs::TryLockError::WouldBlock)
        ));

        drop(workspace);

        assert!(!path.exists());
        assert_eq!(fs::read_dir(&parent.0)?.count(), 0);
        Ok(())
    }

    #[test]
    fn embedded_fixture_is_the_reviewed_policy_fixture() -> TestResult {
        let policy = reviewed_compatibility_policy()?;

        assert!(matches_integrity(F01, policy.fixture));
        Ok(())
    }

    #[test]
    fn fixture_truth_constants_match_the_corpus_manifest() -> TestResult {
        let manifest_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/manifest.json");
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(manifest_path)?)?;
        let f01 = manifest["fixtures"]
            .as_array()
            .and_then(|fixtures| fixtures.iter().find(|fixture| fixture["id"] == "F01"))
            .ok_or("F01 missing from the corpus manifest")?;

        assert_eq!(f01["duration_us"], F01_DURATION_MICROS);
        assert_eq!(f01["resolution"]["width"], F01_WIDTH);
        assert_eq!(f01["resolution"]["height"], F01_HEIGHT);
        Ok(())
    }

    #[test]
    fn description_matching_f01_truth_is_accepted() -> TestResult {
        assert_eq!(check_description(&f01_description()?), Ok(()));
        Ok(())
    }

    #[test]
    fn description_deviating_from_f01_truth_is_rejected() -> TestResult {
        let mut wrong_duration = f01_description()?;
        wrong_duration.duration = MediaTime::from_micros(F01_DURATION_MICROS + 500_000);
        let mut wrong_size = f01_description()?;
        wrong_size.streams = vec![
            stream(0, MediaStreamKind::Video, Some((640, 360)))?,
            stream(1, MediaStreamKind::Audio, None)?,
        ];
        let mut no_audio = f01_description()?;
        no_audio.streams.truncate(1);

        for description in [wrong_duration, wrong_size, no_audio] {
            assert_eq!(
                check_description(&description),
                Err(MediaToolFailure::UnexpectedResult)
            );
        }
        Ok(())
    }

    #[test]
    fn frame_checks_time_dimensions_and_png_encoding() -> TestResult {
        let mut png = PNG_SIGNATURE.to_vec();
        png.extend_from_slice(b"frame");

        assert_eq!(check_frame(&frame(FRAME_AT_MICROS, &png)?), Ok(()));
        assert_eq!(
            check_frame(&frame(FRAME_AT_MICROS + 40_000, &png)?),
            Err(MediaToolFailure::UnexpectedResult)
        );
        assert_eq!(
            check_frame(&frame(FRAME_AT_MICROS, b"not a png")?),
            Err(MediaToolFailure::UnexpectedResult)
        );
        Ok(())
    }

    #[test]
    fn audio_length_must_match_one_second_of_16khz_mono() -> TestResult {
        let policy = reviewed_compatibility_policy()?;

        assert_eq!(check_audio(&audio(16_000, 32_000)?, &policy), Ok(()));
        assert_eq!(
            check_audio(&audio(16_000, 16_000)?, &policy),
            Err(MediaToolFailure::UnexpectedResult)
        );
        assert_eq!(
            check_audio(&audio(48_000, 32_000)?, &policy),
            Err(MediaToolFailure::UnexpectedResult)
        );
        let over_limit = usize::try_from(policy.audio_file_limit_bytes)? + 2;
        assert_eq!(
            check_audio(&audio(16_000, over_limit)?, &policy),
            Err(MediaToolFailure::OutputLimit)
        );
        Ok(())
    }

    #[test]
    fn model_file_is_identified_only_by_exact_size_and_digest() -> TestResult {
        let directory = TempDir::new()?;
        let pinned_bytes = b"reviewed model bytes";
        let digest = lowercase_hex(&Sha256::digest(pinned_bytes));
        let quantized_bytes = b"quantized bytes";
        let quantized_digest = lowercase_hex(&Sha256::digest(quantized_bytes));
        let pins = [
            (
                ReviewedAsrModel::Base,
                ArtifactIntegrity::from_sha256_hex(u64::try_from(pinned_bytes.len())?, &digest)?,
            ),
            (
                ReviewedAsrModel::BaseQ5_1,
                ArtifactIntegrity::from_sha256_hex(
                    u64::try_from(quantized_bytes.len())?,
                    &quantized_digest,
                )?,
            ),
        ];
        let pinned = &pins[..];
        let known = directory.0.join("known.bin");
        fs::write(&known, pinned_bytes)?;
        let quantized = directory.0.join("quantized.bin");
        fs::write(&quantized, quantized_bytes)?;
        let same_size = directory.0.join("same-size.bin");
        fs::write(&same_size, b"reviewed model BYTES")?;
        let other_size = directory.0.join("other-size.bin");
        fs::write(&other_size, b"short")?;

        assert_eq!(
            verify_model_file(&known, pinned),
            ModelVerification::KnownPinned(ReviewedAsrModel::Base)
        );
        assert_eq!(
            verify_model_file(&quantized, pinned),
            ModelVerification::KnownPinned(ReviewedAsrModel::BaseQ5_1)
        );
        assert_eq!(
            verify_model_file(&same_size, pinned),
            ModelVerification::Unrecognised
        );
        assert_eq!(
            verify_model_file(&other_size, pinned),
            ModelVerification::Unrecognised
        );
        assert_eq!(
            verify_model_file(&directory.0.join("missing.bin"), pinned),
            ModelVerification::Unreadable
        );
        assert_eq!(
            verify_model_file(&directory.0, pinned),
            ModelVerification::Unreadable
        );
        Ok(())
    }

    /// F01-like grey frames: dark background with a brighter panel at
    /// `level`, sampled every `spacing` microseconds.
    fn gray_frames(count: u64, spacing: u64, level: impl Fn(u64) -> u8) -> Vec<RawGrayFrame> {
        (0..count)
            .map(|step| {
                let mut pixels = Box::new([26_u8; vsift_domain::VISUAL_FRAME_BYTES]);
                for row in 16..56 {
                    for column in 16..112 {
                        if let Some(pixel) = pixels.get_mut(row * 128 + column) {
                            *pixel = level(step);
                        }
                    }
                }
                RawGrayFrame {
                    time: MediaTime::from_micros(step * spacing),
                    pixels,
                }
            })
            .collect()
    }

    #[test]
    fn visual_check_requires_the_grid_a_static_screen_and_contrast() -> TestResult {
        let window =
            vsift_domain::VisualWindow::new(0, MediaTime::from_micros(F01_DURATION_MICROS))?;
        assert_eq!(
            check_visual_samples(window, &gray_frames(12, 500_000, |_| 72)),
            Ok(())
        );
        for frames in [
            gray_frames(11, 500_000, |_| 72),
            gray_frames(12, 400_000, |_| 72),
            gray_frames(12, 500_000, |_| 30),
            gray_frames(12, 500_000, |step| if step < 6 { 72 } else { 120 }),
        ] {
            assert_eq!(
                check_visual_samples(window, &frames),
                Err(MediaToolFailure::UnexpectedResult)
            );
        }
        Ok(())
    }
}
