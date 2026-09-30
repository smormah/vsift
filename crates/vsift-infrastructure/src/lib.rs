//! Infrastructure adapters for operating-system and provider boundaries.

#![forbid(unsafe_code)]

// Fault points stop the process on request (ADR 0020). They exist for tests
// and crash campaigns and must never reach a release build.
#[cfg(all(feature = "fault-injection", not(debug_assertions)))]
compile_error!(
    "the fault-injection feature stops the process on request and must never be \
     enabled in a release build"
);

// The crash campaign's negative control can remove a directory
// synchronisation on request (ADR 0020). It must never reach a release build.
#[cfg(all(feature = "durability-campaign", not(debug_assertions)))]
compile_error!(
    "the durability-campaign feature can weaken durable commits on request and must \
     never be enabled in a release build"
);

// The managed-install test hooks can route a download to a local server and
// fail a stage write on request (P13). They must never reach a release build.
#[cfg(all(feature = "install-test-hooks", not(debug_assertions)))]
compile_error!(
    "the install-test-hooks feature bypasses the reviewed publisher routes and must      never be enabled in a release build"
);

mod archive_inventory;
mod batch_file;
mod bounded_tar_inventory;
mod contained_source_store;
mod durable_profile;
mod evidence_media;
mod evidence_record;
mod executable;
mod fault_point;
mod ffmpeg_media;
mod file_lock;
mod filesystem_session_store;
mod gzip_tar_inventory;
mod host_attestation;
mod input_root;
mod local_asr_verification;
mod managed_artifact_store;
mod managed_catalogue;
mod managed_installer;
mod managed_smoke;
mod media_tool_verification;
mod media_tool_verification_cache;
mod offline_artifact_import;
mod private_user_root;
mod process_dependency_probe;
mod process_supervisor;
mod publisher_artifact_transfer;
mod random_identifiers;
mod request_timing;
mod retry_timer;
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
pub use batch_file::{BatchFile, BatchFileError, BatchLine, BatchLines};
pub use bounded_tar_inventory::{
    MAX_TAR_STREAM_BYTES, ReviewedArchiveFile, TarInventoryError, inspect_tar_inventory,
    inspect_tar_selected_files, stage_tar_selected_files,
};
pub use contained_source_store::ContainedSourceStore;
pub use durable_profile::{
    MAX_MOUNTINFO_BYTES, MAX_OS_RELEASE_BYTES, MountDevice, MountInfoError, MountProfile,
    OsReleaseError, OsReleaseProfile, RootMountAccess, classify_mountinfo, classify_os_release,
    classify_root_mount, directory_offers_os_crash_durability, qualifies,
};
pub use evidence_media::{FfmpegAudioExtractor, FfmpegFrameExtractor};
pub use evidence_record::{
    MAX_AUDIO_WAV_BYTES, MAX_EVIDENCE_ARTIFACTS, MAX_EVIDENCE_RECORD_BYTES, decode_evidence_record,
    encode_evidence_record,
};
pub use executable::{
    ExecutableProvenance, ExecutableResolutionError, ExecutableResolver, ManagedRuntimeHold,
    TrustedExecutable,
};
#[cfg(feature = "fault-injection")]
pub use fault_point::{FAULT_EXIT_CODE, FAULT_MARKER, FAULT_POINT_VARIABLE, FaultPoint};
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
    BundleSourcePolicy, BundleStatus, CleanOutcome, DEFAULT_ADMISSION_CAPACITY, EvidenceInventory,
    EvidenceMediaFile, ExclusiveSessionLifetimeHold, FREE_SPACE_RESERVE_BYTES,
    FilesystemAdmissionPermit, FilesystemJobOwner, FilesystemSessionStore, FreeSpaceCheck,
    JOB_CANCEL_POLL, MAX_RECORDED_DOCUMENT_BYTES, MAX_RECORDED_STEPS, MAX_REQUEST_RECORD_BYTES,
    MAX_REQUEST_RECORDS, RecordedRequestResult, RequestRecordWrite, SessionIndexPage,
    SessionReadHold, SessionRegistration, SessionStatus, SessionStoreOpenError,
    SessionWorkDirectory, WorkerRequestClaim, WorkerRequestOwner, WorkerRequestRecord,
    decode_chunk_checkpoint, decode_job_record, decode_request_record, encode_chunk_checkpoint,
    encode_job_record, encode_request_record,
};
pub use gzip_tar_inventory::{
    GzipTarInventoryError, MAX_GZIP_ARCHIVE_BYTES, inspect_gzip_tar_inventory,
    inspect_gzip_tar_selected_files, stage_gzip_tar_selected_files,
};
pub use host_attestation::{
    AttestationGap, AttestationParseError, CgroupLimit, CgroupMembership, HostObservation,
    MAX_CGROUP_DEPTH, MAX_CGROUP_FILE_BYTES, MAX_NET_DEV_BYTES, MAX_PROC_CGROUP_BYTES,
    NetworkInterfaces, StrictLinuxAttestation, attest_strict_linux_host, decide_strict_linux,
    parse_cgroup_limit, parse_cpu_max, parse_net_dev, parse_proc_cgroup,
};
pub use input_root::{
    ContainedFile, ContainedPathError, InputRoot, InputRootError, MAX_INPUT_PATH_BYTES,
    MAX_INPUT_PATH_COMPONENTS,
};
pub use local_asr_verification::{
    FixtureAsrVerifier, LOCAL_ASR_VERIFICATION_PROFILE, LocalAsrFiles, local_asr_fingerprint,
    local_asr_fixture_sha256, run_within_budget,
};
pub use managed_artifact_store::{
    ManagedArtifactError, ManagedArtifactStore, ManagedCandidateError, ManagedCandidateFailure,
    ManagedInstallGuard, ManagedPayloadError, ManagedRuntimeIdentity, ManagedRuntimeLayoutError,
    ManagedRuntimePublicationError, ManagedRuntimeRole, ManagedVersionRemovalOutcome,
    PreparedManagedRuntime, PublishedManagedRuntime, ReviewedPayloadArchive, ReviewedRuntimeAlias,
    ReviewedRuntimeLayout, StagedManagedArtifact, StagedManagedCandidate, StagedManagedPayload,
};
pub use managed_catalogue::{
    ManagedCatalogueError, ReviewedActionStageError, ReviewedUbuntuAction, ReviewedWhisperModel,
    accepted_ubuntu_catalogue, detect_managed_target, managed_executable_name,
    pinned_whisper_model, reviewed_compatibility_policy, reviewed_whisper_models,
    whisper_model_profile,
};
pub use managed_installer::{
    ActionAuthority, ManagedArtifactSource, ManagedInstallerConfig, ReviewedManagedInstaller,
    SmokeCompanionSource,
};
pub use managed_smoke::{
    MediaSmokeRequest, ReviewedFixtureVerifiers, SmokeCompanions, SmokeFixtureVerifiers,
    SpeechSmokeRequest, StagedCompatibilitySmoke,
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
pub use offline_artifact_import::{ArtifactImportError, import_reviewed_artifact};
pub use process_dependency_probe::{ExplicitProbePaths, ProcessDependencyProbe};
pub use process_supervisor::{
    CapturedOutput, ControlStatus, DEFAULT_STREAM_LIMIT, EffectiveControls, HardIsolation,
    HostIsolation, IsolationRequirement, OutputStream, ProcessCancellation, ProcessContainment,
    ProcessError, ProcessOutcome, ProcessRequest, ProcessRequestError, ProcessSupervisor,
    ProcessWorkingDirectory, SupervisorPolicy, TerminationReason, completed_termination,
};
pub use publisher_artifact_transfer::{
    PublisherOrigin, PublisherSourceError, PublisherTransferError, ReviewedPublisherArtifact,
    download_reviewed_publisher_artifact,
};
pub use random_identifiers::RandomIdentifierSource;
pub use request_timing::{DeadlineOutcome, run_until_deadline, sleep_unless_cancelled};
pub use retry_timer::TokioRetryTimer;
pub use session_root::{
    PROVISIONING_WAIT, SessionRootError, SessionRootProvisioning, open_session_root,
    open_session_root_within, platform_session_root,
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
pub use transcript_sidecar::{
    MAX_LINE_BYTES, parse_supplied_transcript, read_supplied_transcript,
    read_supplied_transcript_contained,
};
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
