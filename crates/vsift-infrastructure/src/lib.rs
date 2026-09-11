//! Infrastructure adapters for operating-system and provider boundaries.

#![forbid(unsafe_code)]

mod executable;
mod filesystem_session_store;
mod process_dependency_probe;
mod process_supervisor;

pub use executable::{
    ExecutableProvenance, ExecutableResolutionError, ExecutableResolver, TrustedExecutable,
};
pub use filesystem_session_store::{FilesystemSessionStore, SessionStoreOpenError};
pub use process_dependency_probe::ProcessDependencyProbe;
pub use process_supervisor::{
    CapturedOutput, ControlStatus, DEFAULT_STREAM_LIMIT, EffectiveControls, HardIsolation,
    HostIsolation, IsolationRequirement, OutputStream, ProcessCancellation, ProcessContainment,
    ProcessError, ProcessOutcome, ProcessRequest, ProcessRequestError, ProcessSupervisor,
    ProcessWorkingDirectory, SupervisorPolicy, TerminationReason,
};
