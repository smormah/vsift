//! Reviewed managed-artifact catalogue and exact host-target assessment.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::fs;
use std::{collections::HashSet, error::Error, fmt};

use vsift_application::{
    AcceptedManagedArtifact, AcceptedManagedCatalogue, ManagedSetupAction, ReviewedArchiveLimits,
    ReviewedArchiveLink, ReviewedArchiveSelection, ReviewedManagedFile, ReviewedRuntimeCopy,
};
use vsift_domain::{
    ArtifactIntegrity, ArtifactIntegrityError, ManagedArtifactFormat, ManagedComponent,
    ManagedTarget,
};

use crate::{
    ArchiveInventoryBounds, ManagedPayloadError, ManagedRuntimeLayoutError, PreparedManagedRuntime,
    PublisherOrigin, PublisherSourceError, ReviewedArchiveAlias, ReviewedArchiveFile,
    ReviewedPayloadArchive, ReviewedPublisherArtifact, ReviewedRuntimeAlias, ReviewedRuntimeLayout,
    StagedManagedArtifact, StagedManagedPayload,
};

const CATALOGUE_REVISION: &str = "ubuntu-24.04-x86_64-2026-09-21-r1";
const STOP_NEW_PLANS_AT: u64 = 1_848_700_800;
const STOP_NEW_PLANS_DATE: &str = "2028-08-01T00:00:00Z";
const FFMPEG_URL: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-08-31-13-27/ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0.tar.xz";
const WHISPER_URL: &str = "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.2/whisper-bin-ubuntu-x64.tar.gz";
const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/80da2d8bfee42b0e836fc3a9890373e5defc00a6/ggml-base.bin";

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

/// A reviewed action cannot enter the archive staging path.
#[derive(Debug)]
pub enum ReviewedActionStageError {
    /// The selected artifact is a raw file, not an archive.
    RawFile,
    /// Staged bytes were verified against a different reviewed artifact.
    IntegrityMismatch,
    /// Archive inventory, extraction or private staging failed.
    Payload(ManagedPayloadError),
}

impl fmt::Display for ReviewedActionStageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::RawFile => "raw managed artifact requires a file staging path",
            Self::IntegrityMismatch => "staged artifact differs from the accepted action",
            Self::Payload(_) => "reviewed archive could not be staged",
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

    /// Applies the complete reviewed archive inventory to verified staged bytes.
    ///
    /// The raw model requires a separate regular-file staging path. The returned
    /// payload is unactivated and still requires runtime assembly and smoke.
    ///
    /// # Errors
    ///
    /// Refuses raw files and propagates typed bounded-extraction failures.
    pub fn stage_archive<'a>(
        &self,
        staged: &'a StagedManagedArtifact,
    ) -> Result<StagedManagedPayload<'a>, ReviewedActionStageError> {
        if staged.integrity() != self.artifact.integrity {
            return Err(ReviewedActionStageError::IntegrityMismatch);
        }
        let Some(limits) = self.artifact.archive_limits else {
            return Err(ReviewedActionStageError::RawFile);
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
            ManagedArtifactFormat::RawFile => return Err(ReviewedActionStageError::RawFile),
        };
        let Some(bounds) = self.bounds else {
            return Err(ReviewedActionStageError::RawFile);
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
        if self.artifact.format == ManagedArtifactFormat::RawFile
            || payload.selected_names().len() != self.artifact.selected_files.len()
            || self
                .artifact
                .selected_files
                .iter()
                .any(|file| payload.selected_integrity(&file.runtime_name) != Some(file.integrity))
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

fn whisper_artifact() -> Result<AcceptedManagedArtifact, ManagedCatalogueError> {
    let integrity = integrity(
        9_497_583,
        "46811a3ecf584307480a220b9ef5ff81b7b22dc41577cbc274ce3afc61f753b1",
    )?;
    validate_source(WHISPER_URL, PublisherOrigin::GitHubRelease, integrity)?;
    let base_files = [
        (
            "LICENSE",
            1_078,
            "94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d",
            false,
        ),
        (
            "whisper-cli",
            976_312,
            "61fa94d25ba9a4695118883011f35e8521c158145ec73bcd8805a7c11760e6d7",
            true,
        ),
        (
            "libggml.so.0.18.1",
            54_936,
            "1985fa3dc169a16715a0998da0a075b29be8f68ea2501e3c043be53be7f11857",
            false,
        ),
        (
            "libggml-base.so.0.18.1",
            910_680,
            "bc41368cecccc3db8b4f52ad168b51413ee6c005a772b1d3e4f4b3bb47777553",
            false,
        ),
        (
            "libggml-cpu-x64.so",
            878_024,
            "b7c084e19dc63a83acf9d6dac8d2cba089026996bf805659e10d650d5a51c216",
            false,
        ),
        (
            "libwhisper.so.1.9.2",
            611_280,
            "afd9560fa2dd20a7c0f9aa682f9c4f339b2d223f2ad6fa200fc229bc3b1606d6",
            false,
        ),
    ];
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
        version: String::from("whisper.cpp-v1.9.2-ubuntu-x64"),
        publisher: String::from("ggml-org whisper.cpp"),
        source_url: String::from(WHISPER_URL),
        integrity,
        format: ManagedArtifactFormat::TarGz,
        archive_limits: Some(ReviewedArchiveLimits {
            max_stream_bytes: 30_000_000,
            entries: 44,
            expanded_bytes: 24_519_182,
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
        notice_url: String::from("https://github.com/ggml-org/whisper.cpp/blob/v1.9.2/LICENSE"),
        source_code_url: String::from("https://github.com/ggml-org/whisper.cpp/tree/v1.9.2"),
        trust_limit: String::from(
            "Hosted Ubuntu 24.04 x86-64 tone-audio model-backed inference passed; real-speech accuracy, arbitrary Ubuntu hosts and the production installer remain unqualified.",
        ),
        files,
    })
}

fn whisper_archive_links() -> Vec<ReviewedArchiveLink> {
    [
        ("libwhisper.so", "libwhisper.so.1"),
        ("libwhisper.so.1", "libwhisper.so.1.9.2"),
        ("libparakeet.so", "libparakeet.so.1"),
        ("libparakeet.so.1", "libparakeet.so.1.9.2"),
        ("libggml.so", "libggml.so.0"),
        ("libggml.so.0", "libggml.so.0.18.1"),
        ("libggml-base.so", "libggml-base.so.0"),
        ("libggml-base.so.0", "libggml-base.so.0.18.1"),
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
        ("libggml.so.0", "libggml.so.0.18.1"),
        ("libggml.so", "libggml.so.0.18.1"),
        ("libggml-base.so.0", "libggml-base.so.0.18.1"),
        ("libggml-base.so", "libggml-base.so.0.18.1"),
        ("libwhisper.so.1", "libwhisper.so.1.9.2"),
        ("libwhisper.so", "libwhisper.so.1.9.2"),
    ]
    .into_iter()
    .map(|(name, source_selected)| ReviewedRuntimeCopy {
        name: String::from(name),
        source_selected: String::from(source_selected),
    })
    .collect()
}

fn model_artifact() -> Result<AcceptedManagedArtifact, ManagedCatalogueError> {
    let integrity = integrity(
        147_951_465,
        "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    )?;
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

    use vsift_application::ManagedSetupAction;
    use vsift_domain::{ManagedComponent, ManagedTarget};

    use super::{
        ManagedCatalogueError, ReviewedUbuntuAction, accepted_ubuntu_catalogue,
        detect_managed_target, validate_layout,
    };

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
