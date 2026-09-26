//! Infrastructure adapters for operating-system and provider boundaries.

#![forbid(unsafe_code)]

mod archive_inventory;
mod bounded_tar_inventory;
mod evidence_media;
mod evidence_record;
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
mod source_binding;
mod source_duration_probe;
mod source_snapshot;
mod speech_audio;
mod system_clock;
mod transcript_record;
mod transcript_sidecar;
mod user_dependency_config;
mod verified_artifact_transfer;
mod visual_index_record;
mod visual_sampler;
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
pub use evidence_media::{FfmpegAudioExtractor, FfmpegFrameExtractor};
pub use evidence_record::{
    MAX_AUDIO_WAV_BYTES, MAX_EVIDENCE_ARTIFACTS, MAX_EVIDENCE_RECORD_BYTES, decode_evidence_record,
    encode_evidence_record,
};
pub use executable::{
    ExecutableProvenance, ExecutableResolutionError, ExecutableResolver, TrustedExecutable,
};
pub use ffmpeg_media::{
    ExtractedAudio, ExtractedFrame, ExtractedImage, ExtractedWav, FfmpegMedia, FrameListingWindow,
    ImageRegion, MAX_AUDIO_BYTES, MAX_DIAGNOSTIC_BYTES, MAX_FRAME_BYTES, MAX_IMAGES_PER_RUN,
    MAX_LISTING_DIAGNOSTIC_BYTES, MAX_LISTING_RANGE_MICROS, MAX_PROBE_BYTES, MAX_SPEECH_PCM_BYTES,
    MAX_SPEECH_PCM_MICROS, MAX_VISUAL_DIAGNOSTIC_BYTES, MAX_VISUAL_SAMPLE_BYTES,
    MAX_WAV_CLIP_MICROS, MediaError, MediaProviderConformance, ObservedFrameTime, RawGrayFrame,
    VisualSamplingWindow, WAV_HEADER_BYTES, WAV_SAMPLE_RATE, max_frames_per_run,
    parse_ashowinfo_start, parse_ffprobe_metadata, parse_frame_listing, parse_frame_showinfo,
    parse_png_sequence, parse_visual_samples, wav_from_pcm_s16le_mono,
};
pub use filesystem_session_store::{
    BundleSourcePolicy, BundleStatus, CleanOutcome, EvidenceInventory, EvidenceMediaFile,
    ExclusiveSessionLifetimeHold, FilesystemAdmissionPermit, FilesystemSessionStore,
    SessionIndexPage, SessionReadHold, SessionRegistration, SessionStatus, SessionStoreOpenError,
    SessionWorkDirectory,
};
pub use gzip_tar_inventory::{
    GzipTarInventoryError, MAX_GZIP_ARCHIVE_BYTES, inspect_gzip_tar_inventory,
    inspect_gzip_tar_selected_files, stage_gzip_tar_selected_files,
};
pub use local_asr_verification::{
    FixtureAsrVerifier, LOCAL_ASR_VERIFICATION_PROFILE, LocalAsrFiles, local_asr_fingerprint,
    local_asr_fixture_sha256, run_within_budget,
};
pub use managed_artifact_store::{
    ManagedArtifactError, ManagedArtifactStore, ManagedInstallGuard, ManagedPayloadError,
    ManagedRuntimeIdentity, ManagedRuntimeLayoutError, ManagedRuntimePublicationError,
    ManagedVersionRemovalOutcome, PreparedManagedRuntime, PublishedManagedRuntime,
    ReviewedPayloadArchive, ReviewedRuntimeAlias, ReviewedRuntimeLayout, StagedManagedArtifact,
    StagedManagedPayload,
};
pub use managed_catalogue::{
    ManagedCatalogueError, ReviewedActionStageError, ReviewedUbuntuAction, ReviewedWhisperModel,
    accepted_ubuntu_catalogue, detect_managed_target, pinned_whisper_model,
    reviewed_compatibility_policy, reviewed_whisper_models, whisper_model_profile,
};
pub use media_tool_verification::{
    FixtureMediaToolVerifier, identify_whisper_model_file, verify_model_file,
};
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
pub use source_binding::{BoundSource, EvidenceSourceCheck, SourceBinding, VerifiedSourceIdentity};
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
pub use visual_index_record::{
    MAX_VISUAL_INDEX_RECORD_BYTES, MAX_VISUAL_INDEX_RECORDS, decode_visual_index_record,
    encode_visual_index_record,
};
pub use visual_sampler::FfmpegVisualSampler;
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
