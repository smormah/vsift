//! Infrastructure adapters for operating-system and provider boundaries.

#![forbid(unsafe_code)]

mod archive_inventory;
mod bounded_tar_inventory;
mod executable;
mod ffmpeg_media;
mod file_lock;
mod filesystem_session_store;
mod gzip_tar_inventory;
mod local_asr_verification;
mod managed_artifact_store;
mod managed_catalogue;
mod media_tool_verification;
mod media_tool_verification_cache;
mod private_user_root;
mod process_dependency_probe;
mod process_supervisor;
mod publisher_artifact_transfer;
mod random_identifiers;
mod session_root;
mod source_duration_probe;
mod source_snapshot;
mod speech_audio;
mod system_clock;
mod transcript_record;
mod transcript_sidecar;
mod user_dependency_config;
mod verified_artifact_transfer;
mod whisper_build;
mod whisper_cli;
mod xz_tar_inventory;

pub use archive_inventory::{
    ArchiveEntry, ArchiveEntryKind, ArchiveInventoryBounds, ArchiveInventoryError,
    MAX_ARCHIVE_ENTRIES, MAX_ARCHIVE_EXPANDED_BYTES, ReviewedArchiveAlias,
    validate_archive_inventory,
};
pub use bounded_tar_inventory::{
    MAX_TAR_STREAM_BYTES, ReviewedArchiveFile, TarInventoryError, inspect_tar_inventory,
    inspect_tar_selected_files, stage_tar_selected_files,
};
pub use executable::{
    ExecutableProvenance, ExecutableResolutionError, ExecutableResolver, TrustedExecutable,
};
pub use ffmpeg_media::{
    ExtractedAudio, ExtractedFrame, FfmpegMedia, MAX_AUDIO_BYTES, MAX_DIAGNOSTIC_BYTES,
    MAX_FRAME_BYTES, MAX_PROBE_BYTES, MAX_SPEECH_PCM_BYTES, MAX_SPEECH_PCM_MICROS, MediaError,
    MediaProviderConformance, parse_ffprobe_metadata,
};
pub use filesystem_session_store::{
    BundleSourcePolicy, BundleStatus, CleanOutcome, ExclusiveSessionLifetimeHold,
    FilesystemAdmissionPermit, FilesystemSessionStore, SessionIndexPage, SessionReadHold,
    SessionRegistration, SessionStatus, SessionStoreOpenError, SessionWorkDirectory,
};
pub use gzip_tar_inventory::{
    GzipTarInventoryError, MAX_GZIP_ARCHIVE_BYTES, inspect_gzip_tar_inventory,
    inspect_gzip_tar_selected_files, stage_gzip_tar_selected_files,
};
pub use local_asr_verification::{
    FixtureAsrVerifier, LOCAL_ASR_VERIFICATION_PROFILE, LocalAsrFiles, local_asr_fingerprint,
    local_asr_fixture_sha256,
};
pub use managed_artifact_store::{
    ManagedArtifactError, ManagedArtifactStore, ManagedInstallGuard, ManagedPayloadError,
    ManagedRuntimeIdentity, ManagedRuntimeLayoutError, ManagedRuntimePublicationError,
    ManagedVersionRemovalOutcome, PreparedManagedRuntime, PublishedManagedRuntime,
    ReviewedPayloadArchive, ReviewedRuntimeAlias, ReviewedRuntimeLayout, StagedManagedArtifact,
    StagedManagedPayload,
};
pub use managed_catalogue::{
    ManagedCatalogueError, ReviewedActionStageError, ReviewedUbuntuAction,
    accepted_ubuntu_catalogue, detect_managed_target, pinned_whisper_model,
    reviewed_compatibility_policy,
};
pub use media_tool_verification::{FixtureMediaToolVerifier, verify_model_file};
pub use media_tool_verification_cache::{
    FilesystemMediaToolVerificationCache, MAX_MEDIA_TOOL_VERIFICATION_ENTRIES,
    MAX_MEDIA_TOOL_VERIFICATION_RECORD_BYTES, MAX_REMOVED_VERIFICATION_WORKSPACES,
    MEDIA_TOOL_VERIFICATION_MAX_AGE_SECONDS, MEDIA_TOOL_VERIFICATION_PROFILE,
    MediaToolVerificationAuthority, STALE_VERIFICATION_WORKSPACE_AGE_SECONDS, StaleWorkspaceSweep,
    media_tool_fingerprint,
};
pub use process_dependency_probe::{ExplicitProbePaths, ProcessDependencyProbe};
pub use process_supervisor::{
    CapturedOutput, ControlStatus, DEFAULT_STREAM_LIMIT, EffectiveControls, HardIsolation,
    HostIsolation, IsolationRequirement, OutputStream, ProcessCancellation, ProcessContainment,
    ProcessError, ProcessOutcome, ProcessRequest, ProcessRequestError, ProcessSupervisor,
    ProcessWorkingDirectory, SupervisorPolicy, TerminationReason,
};
pub use publisher_artifact_transfer::{
    PublisherOrigin, PublisherSourceError, PublisherTransferCancellation, PublisherTransferError,
    ReviewedPublisherArtifact, download_reviewed_publisher_artifact,
};
pub use random_identifiers::RandomIdentifierSource;
pub use session_root::{
    SessionRootError, SessionRootProvisioning, open_session_root, platform_session_root,
};
pub use source_duration_probe::FfprobeSourceDuration;
pub use source_snapshot::{
    MAX_SOURCE_BYTES, MAX_SOURCE_READ_DURATION, SourceContainer, SourceError, SourceSnapshot,
};
pub use speech_audio::FfmpegSpeechAudio;
pub use system_clock::SystemClock;
pub use transcript_record::{
    MAX_TRANSCRIPT_RECORD_BYTES, decode_transcript_record, encode_transcript_record,
};
pub use transcript_sidecar::{MAX_LINE_BYTES, parse_supplied_transcript, read_supplied_transcript};
pub use user_dependency_config::{UserDependencyConfigError, UserDependencyConfigStore};
pub use verified_artifact_transfer::{ArtifactTransferError, transfer_verified};
pub use whisper_build::{
    MAX_WHISPER_EXECUTABLE_BYTES, WhisperBuildError, WhisperBuildIdentity, WhisperBuildRecognition,
    identify_whisper_build,
};
pub use whisper_cli::{
    MAX_WHISPER_MODEL_BYTES, WHISPER_CHUNK_DEADLINE, WHISPER_STDERR_LIMIT, WHISPER_STDOUT_LIMIT,
    WhisperCli, WhisperError, WhisperOutputError, WhisperOutputLimits, WhisperSpeechRecognizer,
    encode_speech_wav, parse_whisper_full_json,
};
pub use xz_tar_inventory::{
    MAX_XZ_ARCHIVE_BYTES, XzTarInventoryError, inspect_xz_tar_inventory,
    inspect_xz_tar_selected_files, stage_xz_tar_selected_files,
};
