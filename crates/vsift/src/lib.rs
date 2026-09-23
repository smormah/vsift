//! The embeddable `VSift` engine.
//!
//! This crate is the one library API every `VSift` host uses: the command-line
//! interface today, and later an optional MCP adapter, a worker, an indexing
//! service and a desktop application
//! ([ADR 0016](https://github.com/smormah/vsift/blob/main/docs/decisions/0016-embeddable-engine-and-evidence-contract.md)).
//! A host builds an [`Engine`] from explicit [`EngineConfig`] and
//! [`EnginePorts`], calls typed operations, and presents the typed results,
//! usually through the `vsift-contract` wire types so every host emits the same
//! JSON.
//!
//! # Stability
//!
//! **This library API is 0.x and unstable.** It may change in any release
//! until it is declared stable. The stable public surfaces are the CLI and its
//! versioned v1 JSON contract, not these Rust types.
//!
//! # Construction
//!
//! [`Engine::new`] performs no I/O and reads no global state. Locations
//! ([`SessionRootLocation`], [`UserConfigurationLocation`]) are resolved when an
//! operation first needs them, so each failure surfaces from the operation that
//! used the location. Time and identity come only from the injected [`Clock`]
//! and [`IdentifierSource`]; [`EnginePorts::system`] supplies the production
//! clock and random identifiers.
//!
//! # Operations
//!
//! - **Setup:** [`Engine::check_setup`], [`Engine::plan_setup`] with
//!   [`EvaluatedSetupPlan::validate_acceptance`] for saved-plan acceptance,
//!   [`Engine::configure_executable`] and [`Engine::configure_model`].
//! - **Sessions:** [`Engine::ingest`], [`Engine::list_sessions`],
//!   [`Engine::session_status`], [`Engine::renew_session`],
//!   [`Engine::close_session`], [`Engine::retain_session`] and
//!   [`Engine::clean_sessions`].
//! - **Bundles:** [`Engine::validate_bundle`].
//! - **Verification:** [`Engine::verify_media_tools`] and
//!   [`Engine::identify_model`]; no CLI command calls these yet.
//!
//! # Errors
//!
//! Every operation fails with a typed [`EngineError`] that keeps its cause.
//! [`EngineError::failure_code`] is the single mapping to the stable public
//! failure code.
//!
//! # Boundaries
//!
//! The domain, application and infrastructure crates are implementation detail.
//! The value types that appear in this API are re-exported here so a host
//! never depends on those crates, and infrastructure types are mirrored rather
//! than exposed.

#![forbid(unsafe_code)]

mod engine;
mod error;
mod sessions;
mod setup;
mod verification;

pub use engine::{
    Engine, EngineConfig, EnginePorts, HostIsolation, SessionRootLocation,
    UserConfigurationLocation,
};
pub use error::{EngineError, ExecutableRejection, SessionRootError, UserConfigurationError};
pub use sessions::{
    BundleSummary, CleanDecision, CleanEntry, CleanMode, CleanPage, CleanRequest, CleanScope,
    IngestRequest, SessionListEntry, SessionPage, SessionSnapshot, SourceRetention,
};
pub use setup::{
    EvaluatedSetupPlan, ExecutableSelections, SetupCheckReport, SetupCheckRequest, SetupPlanRequest,
};
pub use verification::{
    Cancellation, MediaToolSelection, MediaToolVerificationRequest, ModelSelection,
};

pub use vsift_application::{
    Clock, ClockError, IdentifierGenerationError, IdentifierSource, MediaToolCheck,
    MediaToolFailure, MediaToolVerification, ModelVerification, OpenSessionError,
    OpenSessionOutcome, PlanAcceptanceError, RuntimeDiagnosis, SessionStorageError, SetupProfile,
};
pub use vsift_domain::{
    DependencyState, DependencyStatus, DurabilityRequirement, EvidenceId, FailureClass,
    FailureCode, IdentifierError, JobId, OperationId, PublicationGuarantee, RuntimeCapability,
    RuntimeDependency, RuntimeReadiness, SessionId, SessionLifetime, SessionPhase, SourceId,
    StorageGeneration,
};
