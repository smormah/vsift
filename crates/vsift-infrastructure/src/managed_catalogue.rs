//! Reviewed managed-artifact catalogue and exact host-target assessment.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::fs;
use std::{collections::HashSet, error::Error, fmt};

use vsift_application::{
    AcceptedManagedArtifact, AcceptedManagedCatalogue, ManagedSetupAction, ReviewedArchiveLimits,
    ReviewedArchiveLink, ReviewedArchiveSelection, ReviewedCompatibilityPolicy,
    ReviewedManagedFile, ReviewedRuntimeCopy, SupersededManagedVersion, SupersessionReason,
};
use vsift_domain::{
    ArtifactIntegrity, ArtifactIntegrityError, AsrModelProfile, ManagedArtifactFormat,
    ManagedComponent, ManagedTarget, ReviewedAsrModel,
};

use crate::{
    ArchiveInventoryBounds, ManagedCandidateError, ManagedCandidateFailure, ManagedPayloadError,
    ManagedRuntimeLayoutError, ManagedRuntimeRole, PreparedManagedRuntime, PublisherOrigin,
    PublisherSourceError, ReviewedArchiveAlias, ReviewedArchiveFile, ReviewedPayloadArchive,
    ReviewedPublisherArtifact, ReviewedRuntimeAlias, ReviewedRuntimeLayout, StagedManagedArtifact,
    StagedManagedCandidate, StagedManagedPayload,
};

const CATALOGUE_REVISION: &str = "ubuntu-24.04-x86_64-2026-10-18-r3";
const STOP_NEW_PLANS_AT: u64 = 1_848_700_800;
const STOP_NEW_PLANS_DATE: &str = "2028-08-01T00:00:00Z";
const FFMPEG_URL: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-08-31-13-27/ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0.tar.xz";
const WHISPER_URL: &str =
    "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-ubuntu-x64.tar.gz";
const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/80da2d8bfee42b0e836fc3a9890373e5defc00a6/ggml-base.bin";
const MODEL_BYTES: u64 = 147_951_465;
const MODEL_SHA256: &str = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";
/// The `q5_1` quantization of the multilingual base model (maintainer decision
/// D6). Revision `80da2d8` predates the repository's quantized files, so this
/// pins the repository's newest revision, `5359861` (2024-10-29), at which
/// `ggml-base.bin` is byte-identical to the base pin above (same Git LFS
/// SHA-256): both reviewed models exist together at one immutable revision.
/// The maintainer accepted this pin revision on 2026-09-25.
const MODEL_Q5_1_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base-q5_1.bin";
const MODEL_Q5_1_BYTES: u64 = 59_707_625;
const MODEL_Q5_1_SHA256: &str = "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898";
const COMPATIBILITY_FIXTURE_BYTES: u64 = 76_500;
const COMPATIBILITY_FIXTURE_SHA256: &str =
    "65cec002d7dd8747e8ceb76f25270d35f38bfe354292e3c07bfa6169e2445070";
const EXPECTED_FFMPEG_VERSION: &str = "ffmpeg version n9.0.1-11-ge47273f4d9-20260831";
const EXPECTED_FFPROBE_VERSION: &str = "ffprobe version n9.0.1-11-ge47273f4d9-20260831";

/// Reviewed source data is internally inconsistent and must not produce a plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedCatalogueError {
    /// A pinned size or digest is invalid.
    InvalidIntegrity,
    /// A direct-origin URL does not match its reviewed publisher route.
    InvalidPublisherSource,
    /// Selected paths, runtime copies or archive limits disagree.
    InvalidLayout,
    /// An action differs from the exact currently accepted source entry.
    UnacceptedAction,
}

impl fmt::Display for ManagedCatalogueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIntegrity => "managed catalogue integrity metadata is invalid",
            Self::InvalidPublisherSource => "managed catalogue publisher source is invalid",
            Self::InvalidLayout => "managed catalogue runtime inventory is invalid",
            Self::UnacceptedAction => "managed action differs from the accepted catalogue",
        })
    }
}

impl Error for ManagedCatalogueError {}

impl From<ArtifactIntegrityError> for ManagedCatalogueError {
    fn from(_: ArtifactIntegrityError) -> Self {
        Self::InvalidIntegrity
    }
}

impl From<PublisherSourceError> for ManagedCatalogueError {
    fn from(_: PublisherSourceError) -> Self {
        Self::InvalidPublisherSource
    }
}

/// Detects only host profiles that have explicit R0 setup semantics.
///
/// Linux is accepted only when `/etc/os-release` identifies Ubuntu 24.04 on
/// x86-64. Unknown derivatives and versions fail closed as unsupported.
#[must_use]
pub fn detect_managed_target() -> ManagedTarget {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return ManagedTarget::WindowsX86_64;

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return ManagedTarget::MacOsArm64;

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let Ok(release) = fs::read_to_string("/etc/os-release") else {
            return ManagedTarget::Unsupported;
        };
        let mut identifier = None;
        let mut version = None;
        for line in release.lines() {
            if let Some(value) = line.strip_prefix("ID=") {
                identifier = Some(trim_os_release_value(value));
            } else if let Some(value) = line.strip_prefix("VERSION_ID=") {
                version = Some(trim_os_release_value(value));
            }
        }
        if identifier == Some("ubuntu") && version == Some("24.04") {
            return ManagedTarget::Ubuntu2404X86_64;
        }
        ManagedTarget::Unsupported
    }

    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64")
    )))]
    ManagedTarget::Unsupported
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn trim_os_release_value(value: &str) -> &str {
    value.trim_matches('"')
}

/// Returns the complete accepted catalogue for the only qualified R0 target.
///
/// # Errors
///
/// Fails closed if any literal reviewed digest or publisher URL becomes invalid.
pub fn accepted_ubuntu_catalogue() -> Result<AcceptedManagedCatalogue, ManagedCatalogueError> {
    let artifacts = vec![ffmpeg_artifact()?, whisper_artifact()?, model_artifact()?];
    for artifact in &artifacts {
        validate_layout(artifact)?;
    }
    Ok(AcceptedManagedCatalogue {
        revision: String::from(CATALOGUE_REVISION),
        target: ManagedTarget::Ubuntu2404X86_64,
        stop_new_plans_at: STOP_NEW_PLANS_AT,
        stop_new_plans_date: String::from(STOP_NEW_PLANS_DATE),
        artifacts,
        compatibility: compatibility_policy()?,
        superseded: superseded_versions(),
    })
}

/// The earlier reviewed versions the current pins replace, and why.
///
/// `setup check` names one of these when the managed store still selects it
/// (a machine that installed with an earlier release). A re-pin adds the
/// version it replaces here, with `SecurityFixes` when the new pin carries
/// fixes for security defects of the old one, and keeps the earlier entries:
/// a machine can skip releases. whisper.cpp v1.9.2 lacks v1.9.3 to v1.9.5's
/// memory-safety fixes and the fix of CVE-2026-110810 (L-137). `FFmpeg` and
/// the model have not been re-pinned yet, so they have no entry.
fn superseded_versions() -> Vec<SupersededManagedVersion> {
    vec![SupersededManagedVersion {
        component: ManagedComponent::WhisperCli,
        version: String::from("whisper.cpp-v1.9.2-ubuntu-x64"),
        reason: SupersessionReason::SecurityFixes,
    }]
}

/// Returns the reviewed compatibility policy that any selected provider set must meet.
///
/// The policy is target-independent: it fixes the fixture, output limits,
/// deadlines and audio contract. It is published with the Ubuntu catalogue
/// revision, but bring-your-own verification uses it on every target.
///
/// # Errors
///
/// Fails if the reviewed fixture integrity literal is malformed.
pub fn reviewed_compatibility_policy() -> Result<ReviewedCompatibilityPolicy, ManagedCatalogueError>
{
    compatibility_policy()
}

/// Returns the exact size and SHA-256 of the reviewed pinned whisper.cpp model.
///
/// # Errors
///
/// Fails if the reviewed model literals are malformed.
pub fn pinned_whisper_model() -> Result<ArtifactIntegrity, ManagedCatalogueError> {
    Ok(model_artifact()?.integrity)
}

/// One reviewed pinned whisper.cpp model: its profile, exact identity and the
/// immutable publisher URL it was reviewed at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReviewedWhisperModel {
    /// Profile a file with this identity runs as.
    pub profile: ReviewedAsrModel,
    /// Exact size and SHA-256.
    pub integrity: ArtifactIntegrity,
    /// HTTPS URL at a pinned Hugging Face revision.
    pub source_url: &'static str,
    /// File name at that revision.
    pub file_name: &'static str,
}

/// Returns every reviewed pinned whisper.cpp model, default first.
///
/// A registered model runs only when its size and SHA-256 equal one of these
/// (maintainer decisions D5 and D6); which one it equals is its profile, so no
/// configuration field selects a profile. Only the default is part of the
/// managed installation plan.
///
/// # Errors
///
/// Fails if any reviewed model literal or publisher URL is malformed.
pub fn reviewed_whisper_models() -> Result<[ReviewedWhisperModel; 2], ManagedCatalogueError> {
    let reviewed =
        |profile: ReviewedAsrModel| -> Result<ReviewedWhisperModel, ManagedCatalogueError> {
            let (bytes, sha256, source_url, file_name) = match profile {
                ReviewedAsrModel::Base => (MODEL_BYTES, MODEL_SHA256, MODEL_URL, "ggml-base.bin"),
                ReviewedAsrModel::BaseQ5_1 => (
                    MODEL_Q5_1_BYTES,
                    MODEL_Q5_1_SHA256,
                    MODEL_Q5_1_URL,
                    "ggml-base-q5_1.bin",
                ),
            };
            let integrity = integrity(bytes, sha256)?;
            validate_source(source_url, PublisherOrigin::HuggingFaceModel, integrity)?;
            Ok(ReviewedWhisperModel {
                profile,
                integrity,
                source_url,
                file_name,
            })
        };
    Ok([
        reviewed(ReviewedAsrModel::Base)?,
        reviewed(ReviewedAsrModel::BaseQ5_1)?,
    ])
}

/// The profile of a model file with this exact size and SHA-256:
/// [`AsrModelProfile::Unreviewed`] unless it equals a reviewed pin.
#[must_use]
pub fn whisper_model_profile(bytes: u64, sha256: [u8; 32]) -> AsrModelProfile {
    reviewed_whisper_models()
        .ok()
        .and_then(|models| {
            models.into_iter().find(|model| {
                model.integrity.bytes() == bytes && model.integrity.sha256() == sha256
            })
        })
        .map_or(AsrModelProfile::Unreviewed, |model| model.profile.profile())
}

fn compatibility_policy() -> Result<ReviewedCompatibilityPolicy, ManagedCatalogueError> {
    Ok(ReviewedCompatibilityPolicy {
        fixture: ArtifactIntegrity::from_sha256_hex(
            COMPATIBILITY_FIXTURE_BYTES,
            COMPATIBILITY_FIXTURE_SHA256,
        )?,
        expected_ffmpeg_version: String::from(EXPECTED_FFMPEG_VERSION),
        expected_ffprobe_version: String::from(EXPECTED_FFPROBE_VERSION),
        stream_limit_bytes: 64 * 1024,
        audio_file_limit_bytes: 256 * 1024,
        transcript_file_limit_bytes: 64 * 1024,
        media_deadline_seconds: 60,
        inference_deadline_seconds: 180,
        audio_sample_rate_hz: 16_000,
        audio_channels: 1,
    })
}

/// Exact production-source binding for one action from the accepted Ubuntu plan.
///
/// Constructing this value rechecks every field against the reviewed literals;
/// a serialized plan or caller-edited action cannot supply transport authority.
#[derive(Clone, Debug)]
pub struct ReviewedUbuntuAction {
    artifact: AcceptedManagedArtifact,
    publisher: ReviewedPublisherArtifact,
    bounds: Option<ArchiveInventoryBounds>,
}

/// A reviewed action cannot enter its owned payload staging path.
#[derive(Debug)]
pub enum ReviewedActionStageError {
    /// Staged bytes were verified against a different reviewed artifact.
    IntegrityMismatch,
    /// Reviewed raw-file or archive payload staging failed.
    Payload(ManagedPayloadError),
}

impl fmt::Display for ReviewedActionStageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::IntegrityMismatch => "staged artifact differs from the accepted action",
            Self::Payload(_) => "reviewed managed payload could not be staged",
        })
    }
}

impl Error for ReviewedActionStageError {}

impl ReviewedUbuntuAction {
    /// Rebinds a planned action to the exact current accepted source entry.
    ///
    /// # Errors
    ///
    /// Rejects any changed action inventory, URL, digest, disclosure or version.
    pub fn from_accepted_action(
        action: &ManagedSetupAction,
    ) -> Result<Self, ManagedCatalogueError> {
        let (expected, url, origin) = match action.artifact.component {
            ManagedComponent::MediaTools => (
                ffmpeg_artifact()?,
                FFMPEG_URL,
                PublisherOrigin::GitHubRelease,
            ),
            ManagedComponent::WhisperCli => (
                whisper_artifact()?,
                WHISPER_URL,
                PublisherOrigin::GitHubRelease,
            ),
            ManagedComponent::WhisperModel => (
                model_artifact()?,
                MODEL_URL,
                PublisherOrigin::HuggingFaceModel,
            ),
        };
        if action.id != format!("install-{}", expected.component.identifier())
            || action.artifact != expected
        {
            return Err(ManagedCatalogueError::UnacceptedAction);
        }
        validate_layout(&expected)?;
        let bounds = expected
            .archive_limits
            .map(|limits| ArchiveInventoryBounds::new(limits.entries, limits.expanded_bytes))
            .transpose()
            .map_err(|_| ManagedCatalogueError::InvalidLayout)?;
        let publisher =
            ReviewedPublisherArtifact::from_reviewed_source(url, origin, expected.integrity)?;
        Ok(Self {
            artifact: expected,
            publisher,
            bounds,
        })
    }

    /// Binds a test catalogue's action to a local test server: the artifact
    /// is fetched from `<base_url>/<file name of its source URL>`.
    ///
    /// Development builds only (`install-test-hooks`, refused in a release
    /// build and by the governance check): unlike
    /// [`Self::from_accepted_action`] it does not require the compiled
    /// literals, so the transaction tests can install small fixture
    /// artifacts. The action's size, SHA-256 and layout review still apply.
    ///
    /// # Errors
    ///
    /// Rejects an invalid layout or a URL that is not a canonical loopback
    /// route.
    #[cfg(any(test, feature = "install-test-hooks"))]
    pub fn loopback_for_tests(
        action: &ManagedSetupAction,
        base_url: &str,
        proxy: Option<&str>,
    ) -> Result<Self, ManagedCatalogueError> {
        validate_layout(&action.artifact)?;
        let file_name = action
            .artifact
            .source_url
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .ok_or(ManagedCatalogueError::InvalidPublisherSource)?;
        let bounds = action
            .artifact
            .archive_limits
            .map(|limits| ArchiveInventoryBounds::new(limits.entries, limits.expanded_bytes))
            .transpose()
            .map_err(|_| ManagedCatalogueError::InvalidLayout)?;
        let publisher = ReviewedPublisherArtifact::loopback_for_tests(
            &format!("{base_url}/{file_name}"),
            action.artifact.integrity,
            proxy,
        )?;
        Ok(Self {
            artifact: action.artifact.clone(),
            publisher,
            bounds,
        })
    }

    /// Immutable publisher source and exact artifact integrity for bounded transfer.
    #[must_use]
    pub const fn publisher_source(&self) -> &ReviewedPublisherArtifact {
        &self.publisher
    }

    /// Exact accepted component and version metadata for later publication.
    #[must_use]
    pub const fn artifact(&self) -> &AcceptedManagedArtifact {
        &self.artifact
    }

    /// Applies the complete reviewed raw-file or archive policy to staged bytes.
    ///
    /// The returned payload is unactivated and still requires runtime assembly
    /// and provider compatibility smoke before any publication.
    ///
    /// # Errors
    ///
    /// Refuses a different whole-artifact identity and propagates typed raw
    /// copy or bounded archive-extraction failures.
    pub fn stage_payload<'a>(
        &self,
        staged: &'a StagedManagedArtifact,
    ) -> Result<StagedManagedPayload<'a>, ReviewedActionStageError> {
        if staged.integrity() != self.artifact.integrity {
            return Err(ReviewedActionStageError::IntegrityMismatch);
        }
        if self.artifact.format == ManagedArtifactFormat::RawFile {
            let Some(file) = self.artifact.files.first() else {
                return Err(ReviewedActionStageError::Payload(
                    ManagedPayloadError::InvalidReview,
                ));
            };
            return staged
                .stage_reviewed_raw_file(&file.name, file.integrity)
                .map_err(ReviewedActionStageError::Payload);
        }
        let Some(limits) = self.artifact.archive_limits else {
            return Err(ReviewedActionStageError::Payload(
                ManagedPayloadError::InvalidReview,
            ));
        };
        let archive = match self.artifact.format {
            ManagedArtifactFormat::TarXz => ReviewedPayloadArchive::XzTar {
                max_compressed_bytes: self.artifact.integrity.bytes(),
                max_tar_bytes: limits.max_stream_bytes,
            },
            ManagedArtifactFormat::TarGz => ReviewedPayloadArchive::GzipTar {
                max_compressed_bytes: self.artifact.integrity.bytes(),
                max_tar_bytes: limits.max_stream_bytes,
            },
            ManagedArtifactFormat::RawFile => {
                return Err(ReviewedActionStageError::Payload(
                    ManagedPayloadError::InvalidReview,
                ));
            }
        };
        let Some(bounds) = self.bounds else {
            return Err(ReviewedActionStageError::Payload(
                ManagedPayloadError::InvalidReview,
            ));
        };
        let links = self
            .artifact
            .archive_links
            .iter()
            .map(|link| ReviewedArchiveAlias {
                path: &link.archive_path,
                target: &link.target,
            })
            .collect::<Vec<_>>();
        let files = self
            .artifact
            .selected_files
            .iter()
            .map(|file| ReviewedArchiveFile {
                path: &file.archive_path,
                integrity: file.integrity,
            })
            .collect::<Vec<_>>();
        staged
            .stage_reviewed_payload(archive, bounds, &links, &files)
            .map_err(ReviewedActionStageError::Payload)
    }

    /// Copies the exact selected files and reviewed regular aliases into an
    /// unactivated private runtime; no archive link is materialized as a link.
    ///
    /// # Errors
    ///
    /// Rejects changed payload bytes, invalid layout or unsafe private storage.
    pub fn prepare_runtime<'payload>(
        &self,
        payload: &'payload StagedManagedPayload<'_>,
    ) -> Result<PreparedManagedRuntime<'payload, 'payload>, ManagedRuntimeLayoutError> {
        let selected = match self.artifact.format {
            ManagedArtifactFormat::RawFile => self
                .artifact
                .files
                .iter()
                .map(|file| (file.name.as_str(), file.integrity))
                .collect::<Vec<_>>(),
            ManagedArtifactFormat::TarXz | ManagedArtifactFormat::TarGz => self
                .artifact
                .selected_files
                .iter()
                .map(|file| (file.runtime_name.as_str(), file.integrity))
                .collect::<Vec<_>>(),
        };
        if payload.selected_names().len() != selected.len()
            || selected
                .iter()
                .any(|(name, integrity)| payload.selected_integrity(name) != Some(*integrity))
        {
            return Err(ManagedRuntimeLayoutError::InvalidReview);
        }
        let max_bytes = self
            .artifact
            .files
            .iter()
            .try_fold(0_u64, |total, file| {
                total.checked_add(file.integrity.bytes())
            })
            .ok_or(ManagedRuntimeLayoutError::InvalidReview)?;
        let aliases = self
            .artifact
            .runtime_copies
            .iter()
            .map(|copy| ReviewedRuntimeAlias {
                name: &copy.name,
                source_selected: &copy.source_selected,
            })
            .collect::<Vec<_>>();
        let executables = self
            .artifact
            .files
            .iter()
            .filter(|file| file.executable)
            .map(|file| file.name.as_str())
            .collect::<Vec<_>>();
        payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
            max_bytes,
            aliases: &aliases,
            executables: &executables,
        })
    }

    /// Applies the reviewed payload and runtime policy to verified staged
    /// bytes and returns one owned, unactivated smoke candidate.
    ///
    /// The candidate names the reviewed smoke roles of its component: the
    /// media-tools archive supplies `ffmpeg` and `ffprobe`, the whisper.cpp
    /// archive `whisper-cli`, and the model artifact its one model file.
    ///
    /// # Errors
    ///
    /// Returns the typed failure with what cleanup did with the stage.
    pub fn stage_candidate(
        &self,
        staged: StagedManagedArtifact,
    ) -> Result<StagedManagedCandidate, ManagedCandidateError> {
        let component = self.artifact.component;
        let model = self.artifact.files.first().map(|file| file.name.clone());
        let named = |role| {
            managed_executable_name(role)
                .map(|name| (role, name))
                .into_iter()
        };
        let owned: Vec<(ManagedRuntimeRole, String)> = match component {
            ManagedComponent::MediaTools => named(ManagedRuntimeRole::Ffmpeg)
                .chain(named(ManagedRuntimeRole::Ffprobe))
                .collect(),
            ManagedComponent::WhisperCli => named(ManagedRuntimeRole::WhisperCli).collect(),
            ManagedComponent::WhisperModel => model
                .map(|name| vec![(ManagedRuntimeRole::SpeechModel, name)])
                .unwrap_or_default(),
        };
        let roles: Vec<(ManagedRuntimeRole, &str)> = owned
            .iter()
            .map(|(role, name)| (*role, name.as_str()))
            .collect();
        StagedManagedCandidate::assemble(staged, component, &roles, |artifact| {
            let payload = self.stage_payload(artifact).map_err(|error| match error {
                ReviewedActionStageError::IntegrityMismatch => {
                    ManagedCandidateFailure::IntegrityMismatch
                }
                ReviewedActionStageError::Payload(payload) => {
                    ManagedCandidateFailure::Payload(payload)
                }
            })?;
            match self.prepare_runtime(&payload) {
                Ok(runtime) => {
                    let runtime = runtime.into_parts();
                    Ok((payload.into_parts(), runtime))
                }
                Err(error) => {
                    // A payload that cannot be cleaned stays for the stage
                    // disposal to report.
                    let _ = payload.discard();
                    Err(ManagedCandidateFailure::Runtime(error))
                }
            }
        })
    }
}

/// The file name a managed runtime gives an executable role on this host:
/// the reviewed name (`ffmpeg`, `ffprobe`, `whisper-cli`) with the
/// platform's executable suffix, which is empty on the one managed target.
/// `None` for the model, whose name is its catalogue file's.
#[must_use]
pub fn managed_executable_name(role: ManagedRuntimeRole) -> Option<String> {
    let base = match role {
        ManagedRuntimeRole::Ffmpeg => "ffmpeg",
        ManagedRuntimeRole::Ffprobe => "ffprobe",
        ManagedRuntimeRole::WhisperCli => "whisper-cli",
        ManagedRuntimeRole::SpeechModel => return None,
    };
    Some(format!("{base}{}", std::env::consts::EXE_SUFFIX))
}

fn validate_layout(artifact: &AcceptedManagedArtifact) -> Result<(), ManagedCatalogueError> {
    let mut names = HashSet::new();
    for file in &artifact.files {
        if !flat_name(&file.name) || !names.insert(file.name.to_ascii_lowercase()) {
            return Err(ManagedCatalogueError::InvalidLayout);
        }
    }
    if artifact.format == ManagedArtifactFormat::RawFile {
        return if artifact.archive_limits.is_none()
            && artifact.selected_files.is_empty()
            && artifact.archive_links.is_empty()
            && artifact.runtime_copies.is_empty()
            && artifact.files.len() == 1
            && artifact.files[0].integrity == artifact.integrity
        {
            Ok(())
        } else {
            Err(ManagedCatalogueError::InvalidLayout)
        };
    }

    let Some(limits) = artifact.archive_limits else {
        return Err(ManagedCatalogueError::InvalidLayout);
    };
    if limits.max_stream_bytes < limits.expanded_bytes
        || limits.entries == 0
        || limits.expanded_bytes == 0
        || artifact.selected_files.is_empty()
    {
        return Err(ManagedCatalogueError::InvalidLayout);
    }
    let mut selected_paths = HashSet::new();
    let mut selected_names = HashSet::new();
    for selection in &artifact.selected_files {
        if !archive_path(&selection.archive_path)
            || !flat_name(&selection.runtime_name)
            || selection.archive_path.rsplit('/').next() != Some(selection.runtime_name.as_str())
            || !selected_paths.insert(selection.archive_path.to_ascii_lowercase())
            || !selected_names.insert(selection.runtime_name.to_ascii_lowercase())
            || !artifact.files.iter().any(|file| {
                file.name == selection.runtime_name && file.integrity == selection.integrity
            })
        {
            return Err(ManagedCatalogueError::InvalidLayout);
        }
    }
    let mut link_paths = HashSet::new();
    for link in &artifact.archive_links {
        if !archive_path(&link.archive_path)
            || !flat_name(&link.target)
            || !link_paths.insert(link.archive_path.to_ascii_lowercase())
            || selected_paths.contains(&link.archive_path.to_ascii_lowercase())
        {
            return Err(ManagedCatalogueError::InvalidLayout);
        }
    }
    let mut copy_names = HashSet::new();
    for copy in &artifact.runtime_copies {
        let Some(source) = artifact
            .files
            .iter()
            .find(|file| file.name == copy.source_selected)
        else {
            return Err(ManagedCatalogueError::InvalidLayout);
        };
        if !flat_name(&copy.name)
            || selected_names.contains(&copy.name.to_ascii_lowercase())
            || !selected_names.contains(&copy.source_selected.to_ascii_lowercase())
            || !copy_names.insert(copy.name.to_ascii_lowercase())
            || !artifact
                .files
                .iter()
                .any(|file| file.name == copy.name && file.integrity == source.integrity)
        {
            return Err(ManagedCatalogueError::InvalidLayout);
        }
    }
    if artifact.files.iter().all(|file| {
        selected_names.contains(&file.name.to_ascii_lowercase())
            || copy_names.contains(&file.name.to_ascii_lowercase())
    }) {
        Ok(())
    } else {
        Err(ManagedCatalogueError::InvalidLayout)
    }
}

fn flat_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn archive_path(path: &str) -> bool {
    !path.is_empty() && path.len() <= 256 && path.split('/').all(flat_name)
}

fn ffmpeg_artifact() -> Result<AcceptedManagedArtifact, ManagedCatalogueError> {
    let integrity = integrity(
        113_372_924,
        "204fc02692b11249c3e688ad18538ce2939129a1fc6abc32a6b2638a024496cf",
    )?;
    validate_source(FFMPEG_URL, PublisherOrigin::GitHubRelease, integrity)?;
    Ok(AcceptedManagedArtifact {
        component: ManagedComponent::MediaTools,
        version: String::from("n9.0.1-11-ge47273f4d9-20260831"),
        publisher: String::from("BtbN FFmpeg Builds"),
        source_url: String::from(FFMPEG_URL),
        integrity,
        format: ManagedArtifactFormat::TarXz,
        archive_limits: Some(ReviewedArchiveLimits {
            max_stream_bytes: 400_000_000,
            entries: 73,
            expanded_bytes: 370_667_773,
        }),
        selected_files: vec![
            selection(
                "ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0/LICENSE.txt",
                "LICENSE.txt",
                7_651,
                "da7eabb7bafdf7d3ae5e9f223aa5bdc1eece45ac569dc21b3b037520b4464768",
            )?,
            selection(
                "ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0/bin/ffmpeg",
                "ffmpeg",
                116_038_416,
                "ed57193f048a65bfb0aa3c360639d7f7109ca014405201e3ea478c9ca4ea20fc",
            )?,
            selection(
                "ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0/bin/ffprobe",
                "ffprobe",
                115_829_520,
                "0e3357bef1737ec02ae600e7f6e4e409966d8d0647521ca523c622be574137b7",
            )?,
        ],
        archive_links: Vec::new(),
        runtime_copies: Vec::new(),
        licence: String::from("LGPL version 3 archive notice (publisher-labelled static build)"),
        notice_url: String::from("https://github.com/FFmpeg/FFmpeg/blob/n9.0.1/COPYING.LGPLv3"),
        source_code_url: String::from(
            "https://github.com/BtbN/FFmpeg-Builds/tree/8267213e26c1031621e6e1210fe3aa4867214f6a",
        ),
        trust_limit: String::from(
            "Third-party static build: the archive LGPLv3 notice and hosted F01 media operations were checked; a complete compiled-component source/licence inventory and legal clearance are not claimed.",
        ),
        files: vec![
            file(
                "LICENSE.txt",
                7_651,
                "da7eabb7bafdf7d3ae5e9f223aa5bdc1eece45ac569dc21b3b037520b4464768",
                false,
            )?,
            file(
                "ffmpeg",
                116_038_416,
                "ed57193f048a65bfb0aa3c360639d7f7109ca014405201e3ea478c9ca4ea20fc",
                true,
            )?,
            file(
                "ffprobe",
                115_829_520,
                "0e3357bef1737ec02ae600e7f6e4e409966d8d0647521ca523c622be574137b7",
                true,
            )?,
        ],
    })
}

/// The reviewed regular files selected from the whisper.cpp v1.9.5 Ubuntu x64
/// archive: name, bytes, SHA-256 and whether it is executable.
///
/// Since v1.9.3 upstream publishes a release's binaries only under the build
/// release of the same commit, and the v1.9.5 release names it: `b5454`, built
/// from `d1be6fde11ac` (the tag's commit) by upstream's release workflow with
/// `WHISPER_BUILD_IS_DEV=OFF`. The sizes and digests were read from the archive
/// on a hosted runner (`tools/p06_archive_inventory.py`).
///
/// The first six are the CLI, its libraries and notice; the alias table in
/// [`whisper_artifact`] refers to them by position. The rest are ggml's
/// optimised CPU backends, which it selects at load time for the host CPU;
/// without them only the generic `libggml-cpu-x64.so` could load, about 11x
/// slower on hosted runners (issue #153).
const WHISPER_FILES: [(&str, u64, &str, bool); 19] = [
    (
        "LICENSE",
        1_078,
        "94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d",
        false,
    ),
    (
        "whisper-cli",
        976_312,
        "110a858e128d1fbc5a75899b42a1de8a7a4c10856bfe45dad1adc278f6459d14",
        true,
    ),
    (
        "libggml.so.0.26.0",
        55_184,
        "25406a47ddc7a6b45adb4995ad5efbc6a7ef6efbc43e64a506d978d11dd6fec8",
        false,
    ),
    (
        "libggml-base.so.0.26.0",
        931_232,
        "8cd57970d5ba71529442758c0b42cb820ed319e03be0691aead74ef298d3f51f",
        false,
    ),
    (
        "libggml-cpu-x64.so",
        942_168,
        "c19a80e868a34bdb197e2901c8f785a77640d6e56055134514b5fa2807171af5",
        false,
    ),
    (
        "libwhisper.so.1.9.5",
        615_800,
        "1517f17d5188f7ef70cbb66a2b8ce258c510d20c25018d18e2801351de13cd55",
        false,
    ),
    (
        "libggml-cpu-alderlake.so",
        1_280_320,
        "d33aa9c10d716534dff77655da446f09516ee4583b695e8cc1aa447e57848dac",
        false,
    ),
    (
        "libggml-cpu-cannonlake.so",
        1_362_352,
        "0cb6e59a34c9798a512398e820942ee18ce99c1e0b308ec593e78655759eaafe",
        false,
    ),
    (
        "libggml-cpu-cascadelake.so",
        1_374_344,
        "a0286798b10092eabaf5f81ce9ae664aa78d3c609ed2138fa8a7ca22da73ef7d",
        false,
    ),
    (
        "libggml-cpu-cooperlake.so",
        1_374_344,
        "4169f4b9c975e46ddc386b695f64fb3a38a3dacd40bbb15465a2a5dbcabb9a1f",
        false,
    ),
    (
        "libggml-cpu-haswell.so",
        1_284_416,
        "a63377026e388671d7925e729fced52d9dd85d1bca780dce97bc2c74817053d4",
        false,
    ),
    (
        "libggml-cpu-icelake.so",
        1_374_344,
        "3f28f5a44dd3640cf179d4f678ea9bdacc536ad69b6b5fb5efd155b6b91c49c8",
        false,
    ),
    (
        "libggml-cpu-ivybridge.so",
        1_213_104,
        "8b22efecdde26d41d3d864f2d5f6e29c31e0951d6453465d1f4fbda50b9eaa4b",
        false,
    ),
    (
        "libggml-cpu-piledriver.so",
        1_209_008,
        "e04b72d641d4ae129c05a2ca167dae4843eb0ecae5d40a31a5d98ae480641b11",
        false,
    ),
    (
        "libggml-cpu-sandybridge.so",
        1_217_200,
        "806218b034614ad31dbe4fbae48bcc0282c966bbe9f0085380d1c49b6b0f7d0b",
        false,
    ),
    (
        "libggml-cpu-sapphirerapids.so",
        1_636_576,
        "d190abfe2ad900b24da89c883f633dc621f69ba12b2f5664659826374c1f4cfb",
        false,
    ),
    (
        "libggml-cpu-skylakex.so",
        1_362_352,
        "66d8e2a22ef39225eb2d80e3a1dd9c8a7dd2be41966760c27f22e79623611dd5",
        false,
    ),
    (
        "libggml-cpu-sse42.so",
        950_184,
        "58e6f62fa6bc2732c5a8b6323377ae62ba6cc1ebad9dfd298b6548819bd94375",
        false,
    ),
    (
        "libggml-cpu-zen4.so",
        1_374_344,
        "42bb25dff93816eb12327a1b82c04ae436dd17eebf30da556143ec9acffd9582",
        false,
    ),
];

fn whisper_artifact() -> Result<AcceptedManagedArtifact, ManagedCatalogueError> {
    let integrity = integrity(
        10_364_195,
        "a72becf15d7917f990f6313867a52638b82b7f9ef237fb0c980dac56a135781c",
    )?;
    validate_source(WHISPER_URL, PublisherOrigin::GitHubRelease, integrity)?;
    let base_files = WHISPER_FILES;
    let mut files = base_files
        .into_iter()
        .map(|(name, bytes, sha256, executable)| file(name, bytes, sha256, executable))
        .collect::<Result<Vec<_>, _>>()?;
    for (name, source) in [
        ("libggml.so.0", 2_usize),
        ("libggml.so", 2),
        ("libggml-base.so.0", 3),
        ("libggml-base.so", 3),
        ("libwhisper.so.1", 5),
        ("libwhisper.so", 5),
    ] {
        let source_integrity = files[source].integrity;
        files.push(ReviewedManagedFile {
            name: String::from(name),
            integrity: source_integrity,
            executable: false,
        });
    }
    Ok(AcceptedManagedArtifact {
        component: ManagedComponent::WhisperCli,
        version: String::from("whisper.cpp-v1.9.5-ubuntu-x64"),
        publisher: String::from("ggml-org whisper.cpp"),
        source_url: String::from(WHISPER_URL),
        integrity,
        format: ManagedArtifactFormat::TarGz,
        archive_limits: Some(ReviewedArchiveLimits {
            max_stream_bytes: 30_000_000,
            entries: 47,
            expanded_bytes: 28_128_870,
        }),
        selected_files: base_files
            .into_iter()
            .map(|(name, bytes, sha256, _)| {
                selection(
                    &format!("whisper-bin-ubuntu-x64/{name}"),
                    name,
                    bytes,
                    sha256,
                )
            })
            .collect::<Result<Vec<_>, _>>()?,
        archive_links: whisper_archive_links(),
        runtime_copies: whisper_runtime_copies(),
        licence: String::from("MIT"),
        notice_url: String::from("https://github.com/ggml-org/whisper.cpp/blob/v1.9.5/LICENSE"),
        source_code_url: String::from("https://github.com/ggml-org/whisper.cpp/tree/v1.9.5"),
        trust_limit: String::from(
            "Hosted Ubuntu 24.04 x86-64 tone-audio model-backed inference passed; real-speech accuracy, arbitrary Ubuntu hosts and the production installer remain unqualified.",
        ),
        files,
    })
}

fn whisper_archive_links() -> Vec<ReviewedArchiveLink> {
    [
        ("libwhisper.so", "libwhisper.so.1"),
        ("libwhisper.so.1", "libwhisper.so.1.9.5"),
        ("libparakeet.so", "libparakeet.so.1"),
        ("libparakeet.so.1", "libparakeet.so.1.9.5"),
        ("libggml.so", "libggml.so.0"),
        ("libggml.so.0", "libggml.so.0.26.0"),
        ("libggml-base.so", "libggml-base.so.0"),
        ("libggml-base.so.0", "libggml-base.so.0.26.0"),
    ]
    .into_iter()
    .map(|(name, target)| ReviewedArchiveLink {
        archive_path: format!("whisper-bin-ubuntu-x64/{name}"),
        target: String::from(target),
    })
    .collect()
}

fn whisper_runtime_copies() -> Vec<ReviewedRuntimeCopy> {
    [
        ("libggml.so.0", "libggml.so.0.26.0"),
        ("libggml.so", "libggml.so.0.26.0"),
        ("libggml-base.so.0", "libggml-base.so.0.26.0"),
        ("libggml-base.so", "libggml-base.so.0.26.0"),
        ("libwhisper.so.1", "libwhisper.so.1.9.5"),
        ("libwhisper.so", "libwhisper.so.1.9.5"),
    ]
    .into_iter()
    .map(|(name, source_selected)| ReviewedRuntimeCopy {
        name: String::from(name),
        source_selected: String::from(source_selected),
    })
    .collect()
}

fn model_artifact() -> Result<AcceptedManagedArtifact, ManagedCatalogueError> {
    let integrity = integrity(MODEL_BYTES, MODEL_SHA256)?;
    validate_source(MODEL_URL, PublisherOrigin::HuggingFaceModel, integrity)?;
    Ok(AcceptedManagedArtifact {
        component: ManagedComponent::WhisperModel,
        version: String::from("whisper-base-multilingual-80da2d8"),
        publisher: String::from("ggerganov whisper.cpp model repository"),
        source_url: String::from(MODEL_URL),
        integrity,
        format: ManagedArtifactFormat::RawFile,
        archive_limits: None,
        selected_files: Vec::new(),
        archive_links: Vec::new(),
        runtime_copies: Vec::new(),
        licence: String::from("MIT (repository model-card disclosure)"),
        notice_url: String::from(
            "https://huggingface.co/ggerganov/whisper.cpp/blob/80da2d8bfee42b0e836fc3a9890373e5defc00a6/README.md",
        ),
        source_code_url: String::from(
            "https://huggingface.co/ggerganov/whisper.cpp/tree/80da2d8bfee42b0e836fc3a9890373e5defc00a6",
        ),
        trust_limit: String::from(
            "Pinned model bytes passed hosted tone-audio inference; speech accuracy and model suitability remain P07 qualification work. The repository model card labels MIT.",
        ),
        files: vec![ReviewedManagedFile {
            name: String::from("ggml-base.bin"),
            integrity,
            executable: false,
        }],
    })
}

fn file(
    name: &str,
    bytes: u64,
    sha256: &str,
    executable: bool,
) -> Result<ReviewedManagedFile, ManagedCatalogueError> {
    Ok(ReviewedManagedFile {
        name: String::from(name),
        integrity: integrity(bytes, sha256)?,
        executable,
    })
}

fn selection(
    archive_path: &str,
    runtime_name: &str,
    bytes: u64,
    sha256: &str,
) -> Result<ReviewedArchiveSelection, ManagedCatalogueError> {
    Ok(ReviewedArchiveSelection {
        archive_path: String::from(archive_path),
        runtime_name: String::from(runtime_name),
        integrity: integrity(bytes, sha256)?,
    })
}

fn integrity(bytes: u64, sha256: &str) -> Result<ArtifactIntegrity, ManagedCatalogueError> {
    ArtifactIntegrity::from_sha256_hex(bytes, sha256).map_err(Into::into)
}

fn validate_source(
    url: &'static str,
    origin: PublisherOrigin,
    integrity: ArtifactIntegrity,
) -> Result<(), ManagedCatalogueError> {
    ReviewedPublisherArtifact::from_reviewed_source(url, origin, integrity)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use vsift_application::{ManagedSetupAction, SupersessionReason};
    use vsift_domain::{AsrModelProfile, ManagedComponent, ManagedTarget, ReviewedAsrModel};

    use super::{
        ManagedCatalogueError, ReviewedUbuntuAction, accepted_ubuntu_catalogue,
        detect_managed_target, pinned_whisper_model, reviewed_whisper_models, validate_layout,
        whisper_model_profile,
    };

    /// D6: the quantized profile sits beside the default, in the same
    /// repository, and a file is its profile only by exact size and digest.
    #[test]
    fn reviewed_models_are_distinct_pins_identified_only_by_exact_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let [base, quantized] = reviewed_whisper_models()?;
        assert_eq!(base.profile, ReviewedAsrModel::Base);
        assert_eq!(base.integrity, pinned_whisper_model()?);
        assert_eq!(quantized.profile, ReviewedAsrModel::BaseQ5_1);
        assert_eq!(quantized.integrity.bytes(), 59_707_625);
        assert_ne!(base.integrity.sha256(), quantized.integrity.sha256());
        for model in [base, quantized] {
            assert!(
                model
                    .source_url
                    .starts_with("https://huggingface.co/ggerganov/whisper.cpp/resolve/")
            );
            assert!(model.source_url.ends_with(&format!("/{}", model.file_name)));
            assert_eq!(
                whisper_model_profile(model.integrity.bytes(), model.integrity.sha256()),
                model.profile.profile()
            );
        }
        assert_eq!(
            whisper_model_profile(base.integrity.bytes(), quantized.integrity.sha256()),
            AsrModelProfile::Unreviewed
        );
        assert_eq!(
            whisper_model_profile(
                quantized.integrity.bytes() + 1,
                quantized.integrity.sha256()
            ),
            AsrModelProfile::Unreviewed
        );
        Ok(())
    }

    /// The v1.9.2 managed install of `0.2.2` and earlier is named, with its
    /// security reason, and nothing the catalogue pins now is superseded.
    #[test]
    fn the_earlier_whisper_pin_is_superseded_for_its_security_fixes()
    -> Result<(), Box<dyn std::error::Error>> {
        let catalogue = accepted_ubuntu_catalogue()?;
        let update = catalogue
            .update_for(
                ManagedComponent::WhisperCli,
                "whisper.cpp-v1.9.2-ubuntu-x64",
            )
            .ok_or("v1.9.2 is not reported as superseded")?;
        assert_eq!(update.reviewed_version, "whisper.cpp-v1.9.5-ubuntu-x64");
        assert_eq!(update.reason, SupersessionReason::SecurityFixes);
        let mut seen = HashSet::new();
        for entry in &catalogue.superseded {
            assert!(seen.insert((entry.component.identifier(), entry.version.clone())));
            assert!(
                catalogue
                    .artifacts
                    .iter()
                    .all(|artifact| artifact.version != entry.version),
                "a pinned version is listed as superseded"
            );
        }
        for artifact in &catalogue.artifacts {
            assert_eq!(
                catalogue.update_for(artifact.component, &artifact.version),
                None
            );
        }
        Ok(())
    }

    #[test]
    fn accepted_catalogue_is_complete_unique_and_time_bounded()
    -> Result<(), Box<dyn std::error::Error>> {
        let catalogue = accepted_ubuntu_catalogue()?;
        assert_eq!(catalogue.target, ManagedTarget::Ubuntu2404X86_64);
        assert_eq!(catalogue.artifacts.len(), 3);
        assert_eq!(
            catalogue.artifacts[0].component,
            ManagedComponent::MediaTools
        );
        assert_eq!(
            catalogue.artifacts[1].component,
            ManagedComponent::WhisperCli
        );
        assert_eq!(
            catalogue.artifacts[2].component,
            ManagedComponent::WhisperModel
        );
        assert_eq!(catalogue.stop_new_plans_date, "2028-08-01T00:00:00Z");
        assert_eq!(catalogue.compatibility.fixture.bytes(), 76_500);
        assert_eq!(
            catalogue.compatibility.fixture.sha256_hex(),
            "65cec002d7dd8747e8ceb76f25270d35f38bfe354292e3c07bfa6169e2445070"
        );
        assert_eq!(catalogue.compatibility.audio_sample_rate_hz, 16_000);
        assert_eq!(catalogue.compatibility.audio_channels, 1);
        assert_eq!(catalogue.compatibility.stream_limit_bytes, 64 * 1024);
        assert_eq!(catalogue.compatibility.audio_file_limit_bytes, 256 * 1024);
        assert_eq!(
            catalogue.compatibility.transcript_file_limit_bytes,
            64 * 1024
        );
        assert_eq!(catalogue.compatibility.media_deadline_seconds, 60);
        assert_eq!(catalogue.compatibility.inference_deadline_seconds, 180);
        for artifact in catalogue.artifacts {
            assert!(artifact.source_url.starts_with("https://"));
            assert!(!artifact.files.is_empty());
            let unique = artifact
                .files
                .iter()
                .map(|file| file.name.to_ascii_lowercase())
                .collect::<HashSet<_>>();
            assert_eq!(unique.len(), artifact.files.len());
        }
        Ok(())
    }

    #[test]
    fn host_detection_returns_a_named_or_unsupported_target() {
        assert!(matches!(
            detect_managed_target(),
            ManagedTarget::Ubuntu2404X86_64
                | ManagedTarget::WindowsX86_64
                | ManagedTarget::MacOsArm64
                | ManagedTarget::Unsupported
        ));
    }

    #[test]
    fn altered_selected_or_alias_inventory_fails_closed() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut catalogue = accepted_ubuntu_catalogue()?;
        catalogue.artifacts[1].runtime_copies[0].source_selected = String::from("not-selected");
        assert_eq!(
            validate_layout(&catalogue.artifacts[1]),
            Err(ManagedCatalogueError::InvalidLayout)
        );
        let mut catalogue = accepted_ubuntu_catalogue()?;
        catalogue.artifacts[0].selected_files[0].archive_path = String::from("../outside");
        assert_eq!(
            validate_layout(&catalogue.artifacts[0]),
            Err(ManagedCatalogueError::InvalidLayout)
        );
        let mut catalogue = accepted_ubuntu_catalogue()?;
        catalogue.artifacts[1].selected_files[0].runtime_name = String::from("elsewhere");
        assert_eq!(
            validate_layout(&catalogue.artifacts[1]),
            Err(ManagedCatalogueError::InvalidLayout)
        );
        Ok(())
    }

    #[test]
    fn only_exact_accepted_actions_receive_publisher_authority()
    -> Result<(), Box<dyn std::error::Error>> {
        let catalogue = accepted_ubuntu_catalogue()?;
        for artifact in catalogue.artifacts {
            let action = ManagedSetupAction {
                id: format!("install-{}", artifact.component.identifier()),
                artifact,
            };
            let reviewed = ReviewedUbuntuAction::from_accepted_action(&action)?;
            assert_eq!(
                reviewed.publisher_source().integrity(),
                action.artifact.integrity
            );
            assert_eq!(reviewed.artifact(), &action.artifact);

            let mut altered = action.clone();
            altered.artifact.version.push_str("-changed");
            assert!(matches!(
                ReviewedUbuntuAction::from_accepted_action(&altered),
                Err(ManagedCatalogueError::UnacceptedAction)
            ));
            let mut altered = action.clone();
            altered.id.push_str("-changed");
            assert!(matches!(
                ReviewedUbuntuAction::from_accepted_action(&altered),
                Err(ManagedCatalogueError::UnacceptedAction)
            ));
        }
        Ok(())
    }
}
