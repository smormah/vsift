//! Infrastructure adapters for operating-system and provider boundaries.

#![forbid(unsafe_code)]

mod archive_inventory;
mod bounded_tar_inventory;
mod executable;
mod ffmpeg_media;
mod filesystem_session_store;
mod gzip_tar_inventory;
mod process_dependency_probe;
mod process_supervisor;
mod source_snapshot;
mod user_dependency_config;
mod verified_artifact_transfer;

pub use archive_inventory::{
    ArchiveEntry, ArchiveEntryKind, ArchiveInventoryBounds, ArchiveInventoryError,
    MAX_ARCHIVE_ENTRIES, MAX_ARCHIVE_EXPANDED_BYTES, ReviewedArchiveAlias,
    validate_archive_inventory,
};
pub use bounded_tar_inventory::{MAX_TAR_STREAM_BYTES, TarInventoryError, inspect_tar_inventory};
pub use executable::{
    ExecutableProvenance, ExecutableResolutionError, ExecutableResolver, TrustedExecutable,
};
pub use ffmpeg_media::{
    ExtractedAudio, ExtractedFrame, FfmpegMedia, MAX_AUDIO_BYTES, MAX_DIAGNOSTIC_BYTES,
    MAX_FRAME_BYTES, MAX_PROBE_BYTES, MediaError, MediaProviderConformance,
};
pub use filesystem_session_store::{
    BundleSourcePolicy, BundleStatus, CleanOutcome, ExclusiveSessionLifetimeHold,
    FilesystemAdmissionPermit, FilesystemSessionStore, SessionIndexPage, SessionReadHold,
    SessionRegistration, SessionStatus, SessionStoreOpenError,
};
pub use gzip_tar_inventory::{
    GzipTarInventoryError, MAX_GZIP_ARCHIVE_BYTES, inspect_gzip_tar_inventory,
};
pub use process_dependency_probe::{ExplicitProbePaths, ProcessDependencyProbe};
pub use process_supervisor::{
    CapturedOutput, ControlStatus, DEFAULT_STREAM_LIMIT, EffectiveControls, HardIsolation,
    HostIsolation, IsolationRequirement, OutputStream, ProcessCancellation, ProcessContainment,
    ProcessError, ProcessOutcome, ProcessRequest, ProcessRequestError, ProcessSupervisor,
    ProcessWorkingDirectory, SupervisorPolicy, TerminationReason,
};
pub use source_snapshot::{
    MAX_SOURCE_BYTES, MAX_SOURCE_READ_DURATION, SourceContainer, SourceError, SourceSnapshot,
};
pub use user_dependency_config::{UserDependencyConfigError, UserDependencyConfigStore};
pub use verified_artifact_transfer::{ArtifactTransferError, transfer_verified};
