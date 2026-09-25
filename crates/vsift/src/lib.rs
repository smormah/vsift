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
//! - **Transcripts:** [`Engine::ingest`] with a [`SuppliedTranscriptRequest`]
//!   imports a `SubRip` or `WebVTT` sidecar; [`Engine::retranscribe`]
//!   transcribes the session's speech, or one range of it, locally with
//!   whisper.cpp into a new revision (the only operation that runs local ASR);
//!   [`Engine::transcript`] pages the newest revision, or any earlier one by
//!   identity.
//! - **Bundles:** [`Engine::validate_bundle`].
//! - **Verification:** [`Engine::verify_media_tools`] and
//!   [`Engine::identify_model`]; no CLI command calls these. Operations that
//!   run `FFmpeg`/`FFprobe` on user media (today [`Engine::ingest`] with a
//!   transcript) first run an automatic preflight that verifies the resolved
//!   tools once per tool identity and records the pass in private per-user
//!   state; a failure is [`EngineError::MediaToolVerificationFailed`] and
//!   nothing is written. [`EnginePorts::with_media_tool_verifier`] replaces the
//!   reviewed fixture verifier for tests and hosts with their own authority.
//!   [`Engine::retranscribe`] also runs a local-ASR preflight that transcribes
//!   a reviewed speech clip once per recognizer identity;
//!   [`EnginePorts::with_local_asr_verifier`] and
//!   [`EnginePorts::with_speech_recognizer`] replace it or the recognizer.
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

mod asr;
mod engine;
mod error;
mod local_asr_check;
mod sessions;
mod setup;
mod transcripts;
mod verification;

pub use asr::{RetranscribeOutcome, RetranscribeRange, RetranscribeRequest};
pub use engine::{
    Engine, EngineConfig, EnginePorts, HostIsolation, SessionRootLocation,
    UserConfigurationLocation,
};
pub use error::{
    EngineError, ExecutableRejection, SessionRootError, TranscriptSourceError,
    UserConfigurationError,
};
pub use local_asr_check::DEFAULT_LOCAL_ASR_CHECK_BUDGET;
pub use sessions::{
    BundleSummary, CleanDecision, CleanEntry, CleanMode, CleanPage, CleanRequest, CleanScope,
    IngestOutcome, IngestRequest, SessionListEntry, SessionPage, SessionSnapshot, SourceRetention,
    SuppliedTranscriptRequest,
};
pub use setup::{
    EvaluatedSetupPlan, ExecutableSelections, SetupCheckReport, SetupCheckRequest, SetupPlanRequest,
};
pub use transcripts::{TranscriptExcerpt, TranscriptQuery};
pub use verification::{
    Cancellation, MediaToolSelection, MediaToolVerificationRequest, ModelSelection,
};

pub use vsift_application::{
    AsrFailure, AsrFailureReason, AsrStage, Clock, ClockError, IdentifierGenerationError,
    IdentifierSource, LocalAsrCheckFailure, LocalAsrCheckOutcome, LocalAsrModelStatus,
    LocalAsrNotRunReason, LocalAsrSetupStatus, LocalAsrVerification, LocalAsrVerificationFailure,
    LocalAsrVerificationSource, LocalAsrVerifier, MediaToolCheck, MediaToolFailure,
    MediaToolPreflightFailure, MediaToolVerification, MediaToolVerifier, ModelVerification,
    OpenSessionError, OpenSessionOutcome, PlanAcceptanceError, RecognizerIdentity,
    RuntimeDiagnosis, SessionStorageError, SetupProfile, SourceProbeError, SpeechPcm,
    SpeechRecognitionError, SpeechRecognizer, TranscriptBuildError, TranscriptQueryError,
};
/// Transcript evidence values that appear in this API.
pub use vsift_domain::{
    AlignmentOrigin, CarriedFrom, Confidence, ConfidenceOrigin, CueMarkup, CueSource, CueText,
    CueTiming, CursorError, InheritedRevision, LanguageTag, MediaTime, ProviderEndTrim,
    SegmentOrigin, SidecarIdentity, SourceSegment, SourceSegmentId, SourceSegmentState,
    SpeakerLabel, TimeRange, TranscriptFormat, TranscriptImportError, TranscriptOffset,
    TranscriptProvenance, TranscriptRejection, TranscriptRevision, TranscriptRevisionId,
    TranscriptSegment, TranscriptSegmentId, TranscriptWarning, TranscriptWarningKind,
    TranscriptWarnings,
};
/// Local-ASR provenance values reachable from [`TranscriptProvenance`], and
/// the values a host-supplied [`SpeechRecognizer`] works with.
pub use vsift_domain::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, ChunkPlan, ChunkTime, PlannedChunk, ProviderChunkOutput,
    ProviderOutputError, ProviderSegment, ProviderToken, ProviderTokenKind, ReviewedAsrModel,
    Sha256Hex,
};
pub use vsift_domain::{
    DependencyState, DependencyStatus, DurabilityRequirement, EvidenceId, FailureClass,
    FailureCode, IdentifierError, JobId, OperationId, PublicationGuarantee, RuntimeCapability,
    RuntimeDependency, RuntimeReadiness, SessionId, SessionLifetime, SessionPhase, SourceId,
    StorageGeneration,
};
