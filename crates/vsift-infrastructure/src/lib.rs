//! Infrastructure adapters for operating-system and provider boundaries.

#![forbid(unsafe_code)]

mod executable;
mod ffmpeg_media;
mod filesystem_session_store;
mod process_dependency_probe;
mod process_supervisor;
mod source_snapshot;

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
pub use process_dependency_probe::ProcessDependencyProbe;
pub use process_supervisor::{
    CapturedOutput, ControlStatus, DEFAULT_STREAM_LIMIT, EffectiveControls, HardIsolation,
    HostIsolation, IsolationRequirement, OutputStream, ProcessCancellation, ProcessContainment,
    ProcessError, ProcessOutcome, ProcessRequest, ProcessRequestError, ProcessSupervisor,
    ProcessWorkingDirectory, SupervisorPolicy, TerminationReason,
};
pub use source_snapshot::{
    MAX_SOURCE_BYTES, MAX_SOURCE_READ_DURATION, SourceContainer, SourceError, SourceSnapshot,
};
