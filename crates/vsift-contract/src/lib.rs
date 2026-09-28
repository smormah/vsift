//! Versioned v1 JSON wire contract shared by every `VSift` host.
//!
//! The CLI is the first host, but ADR 0016 commits `VSift` to further hosts (an
//! optional MCP adapter, a worker, a desktop application). Each of them must emit
//! byte-for-byte the same JSON for the same outcome, so the wire types and the
//! mapping from domain and application values into them live here once, instead
//! of being re-implemented in every host.
//!
//! The crate is organised by concern:
//!
//! - **Envelope:** [`OperationResponse`], its [`ErrorResponse`],
//!   [`CoverageResponse`] and [`LifecycleResponse`] parts, the JSON Lines
//!   [`TerminalEventResponse`], and the [`CommandName`] identifiers they carry.
//! - **Setup:** [`SetupCheckResponse`], [`SetupPlanResponse`], the strict
//!   [`SavedSetupPlan`] input, and the configured-selection responses.
//! - **Session:** [`OpenData`], [`StatusData`], [`PageData`], [`CleanData`],
//!   [`BundleData`] and their item types; `session status` adds its newest
//!   jobs through [`SessionStatusData`].
//! - **Jobs:** [`JobData`] for `job status` and `job cancel`, built from a
//!   host's [`JobPresentation`], [`JobResumeData`] for `job resume`, and the
//!   fixed-prose remediation for interrupted, unknown, ended and session-less
//!   jobs (P10 PR 3).
//! - **Evidence:** [`ConfidenceResponse`] and [`FrameTimingResponse`], frozen
//!   before the packets that produce them.
//! - **Transcript:** [`TranscriptSegmentData`] (the published evidence
//!   record), [`TranscriptRevisionData`], the `transcript.get` page
//!   [`TranscriptPageData`], the `transcript.retranscribe` result
//!   [`TranscriptRetranscribeData`], and fixed-prose warnings and remediation.
//! - **Local ASR:** [`local_asr_failure_summary`] and
//!   [`local_asr_verification_summary`], the fixed-prose remediation for a
//!   failed retranscription or its automatic verification, and the fixed
//!   remediation for missing tools, models and audio.
//! - **Evidence stream:** the JSON Lines form of a page, a
//!   [`TranscriptEvidenceStream`] of [`EvidenceEventResponse`] records ended by
//!   one terminal event whose data is [`TranscriptStreamData`], with the
//!   published [`EventKind`] and [`EvidenceRecordType`] identifiers.
//! - **Search:** the `search` page [`SearchData`] and its stream
//!   [`SearchEvidenceStream`] (the matching segments as `transcript_segment`
//!   evidence events, then [`SearchStreamData`]), built from a
//!   [`SearchPresentation`] by [`search_response`], which also fills the
//!   envelope [`CoverageResponse`] and the `partial` status, and
//!   [`search_query_rejection_summary`] for a rejected query. Every stream is
//!   written through the [`EvidenceStream`] view.
//! - **Candidates:** the `candidates` page [`CandidatesData`], its published
//!   evidence record [`VisualCandidateData`] and its stream
//!   [`CandidatesEvidenceStream`] (`visual_candidate` evidence events, then
//!   [`CandidatesStreamData`]), built from a [`CandidatesPresentation`] by
//!   [`candidates_response`], which fills the envelope coverage and the
//!   `partial` status when the range has gaps (P08).
//! - **Evidence navigation:** the `frame get`, `frame neighbours`,
//!   `frame burst` and `crop` result [`FrameData`], its published evidence
//!   record [`FrameEvidenceData`] and its stream [`FrameEvidenceStream`]
//!   (`frame_evidence` evidence events, then [`FrameStreamData`]), built by
//!   [`frame_response`]; the `audio` result [`AudioData`], its record
//!   [`AudioEvidenceData`] and stream [`AudioEvidenceStream`] (then
//!   [`AudioStreamData`]), built by [`audio_response`]. Both present an
//!   [`EvidencePresentation`] with the files delivered as absolute session
//!   artifact paths (ADR 0019 D2), the `partial` status and
//!   [`partial_evidence_warning`] when a call stopped short, and fixed-prose
//!   remediation such as [`EVIDENCE_BUDGET_REMEDIATION`] and
//!   [`frame_selection_summary`] (P09).
//! - **Verification:** [`media_tool_verification_summary`], the fixed-prose
//!   remediation for a failed automatic media-tool preflight.
//! - **Storage:** [`non_private_folder_summary`], the fixed-prose remediation
//!   for an existing [`PrivateFolder`] that other accounts can access.
//! - **Text:** [`sanitize_untrusted_text`], the one rule for placing untrusted
//!   provider text in public output.
//! - **Worker requests (P11):** the strict bounded document decoder
//!   [`decode_strict_json`]; the versioned [`WorkRequest`] decoded by
//!   [`decode_work_request`] (and [`decode_batch_line`] for a `job batch`
//!   line) with its canonical [`WorkRequestDigest`] and typed
//!   [`RequestRejection`]; the [`WorkResult`] that answers it; the batch
//!   summary [`JobBatchData`] with its outcome rule; and [`WorkspaceData`],
//!   the result of initialising a worker workspace.
//!
//! The published JSON Schemas under `schemas/v1` are authoritative. This crate's
//! tests validate its serialized values against them. The Rust API itself is 0.x
//! and unstable (ADR 0016 decision 3); only the JSON it produces is stable.
//!
//! Dependencies point inward: this crate depends on `vsift-domain` and
//! `vsift-application` only, never on infrastructure or a host. Host concerns such
//! as exit codes, human text, output budgets and clock formatting stay in the host.

#![forbid(unsafe_code)]

mod batch;
mod candidates;
mod command;
mod envelope;
mod events;
mod evidence;
mod input;
mod job;
mod local_asr;
mod navigation;
mod request;
mod search;
mod session;
mod setup;
mod storage;
mod stream;
mod text;
mod transcript;
mod verification;
mod work;
mod workspace;

pub use batch::{BatchDataError, BatchItemStatus, BatchOutcome, BatchTermination, JobBatchData};
pub use events::{
    LifecycleEventResponse, LifecycleKind, LifecycleReason, MAX_EVENT_LINE_BYTES,
    MAX_PROGRESS_EVENTS, PROGRESS_INTERVAL_MS, ProgressEventResponse, ProgressReport, Readiness,
    RequestEnd, RequestRef, ResultEventResponse,
};
pub use input::{JsonLimits, StrictJsonError, decode_strict_json};
pub use request::{
    BatchLine, BundleName, InputPathError, MAX_BATCH_LINES, MAX_BUNDLE_NAME_BYTES,
    MAX_INPUT_PATH_BYTES, MAX_INPUT_PATH_COMPONENTS, MAX_REQUEST_DEADLINE_MS, MAX_REQUEST_STEPS,
    RelativeInputPath, RequestDeadline, RequestDurability, RequestRejection,
    SuppliedTranscriptInput, WORK_REQUEST_LIMITS, WorkRequest, WorkRequestDigest, WorkStep,
    WorkStepKind, WorkTarget, decode_batch_line, decode_work_request, validate_steps,
};
pub use work::{
    FreeSpaceReserve, MAX_RESULT_STEPS, MAX_WORK_RESULT_BYTES, PARTIAL_REQUEST_WARNING,
    RecordedResultError, RequestFailure, ResourceLimits, ResultOrigin, StepOutputs, StepResult,
    StepStatus, StepTiming, WorkControls, WorkFailure, WorkResult, WorkResultError,
    WorkResultParts, WorkerIsolation,
};
pub use workspace::{
    ADMISSION_BUSY_REMEDIATION, ADMISSION_CAPACITY_REMEDIATION, BUNDLE_MISMATCH_REMEDIATION,
    BUNDLE_ROOT_REQUIRED_REMEDIATION, DEFAULT_SESSION_RETENTION_SECONDS,
    DURABILITY_UNAVAILABLE_REMEDIATION, INPUT_NOT_FOUND_REMEDIATION,
    INPUT_NOT_REGULAR_FILE_REMEDIATION, INPUT_ROOT_REMEDIATION, INPUT_UNREADABLE_REMEDIATION,
    ISOLATION_UNAVAILABLE_REMEDIATION, MAX_ADMISSION_CAPACITY, MAX_SESSION_RETENTION_SECONDS,
    MIN_SESSION_RETENTION_SECONDS, REQUEST_BUSY_REMEDIATION, REQUEST_CONFLICT_REMEDIATION,
    REQUEST_DEADLINE_REMEDIATION, REQUEST_FILE_REMEDIATION, REQUEST_SESSION_REMEDIATION,
    REQUEST_STOPPED_REMEDIATION, WORKER_WORKSPACE_REQUIRED_REMEDIATION,
    WORKSPACE_NOT_DURABLE_REMEDIATION, WORKSPACE_POLICY_MISMATCH_REMEDIATION,
    WORKSPACE_ROOT_REMEDIATION, WorkspaceData, WorkspaceInitOutcome, WorkspacePolicyError,
};

pub use candidates::{
    CANDIDATE_CURSOR_REMEDIATION, CandidatesData, CandidatesEvidenceStream, CandidatesPresentation,
    CandidatesStreamData, NO_VIDEO_STREAM_REMEDIATION, VISUAL_COVERAGE_WARNING,
    VISUAL_TOOLS_REMEDIATION, VisualCandidateData, candidates_response,
};
pub use command::CommandName;
pub use envelope::{
    CONTRACT_VERSION, CoverageResponse, ErrorResponse, LifecycleResponse, MAX_AFFECTED_IDS,
    OperationResponse, TerminalEventResponse,
};
pub use evidence::{ConfidenceResponse, FrameTimingResponse};
pub use job::{
    JOB_CANCELLED_REMEDIATION, JOB_INTERRUPTED_REMEDIATION, JOB_NOT_RESUMABLE_REMEDIATION,
    JOB_SESSION_NOT_OPEN_REMEDIATION, JobData, JobPresentation, JobResumeData, SessionJobData,
    SessionStatusData, UNKNOWN_JOB_REMEDIATION,
};
pub use local_asr::{
    CANCELLATION_TOO_LATE_WARNING, CHECKPOINT_DISCARDED_WARNING, IDEMPOTENCY_CONFLICT_REMEDIATION,
    JOB_BUSY_REMEDIATION, LOCAL_ASR_MODEL_REMEDIATION, LOCAL_ASR_TOOLS_REMEDIATION,
    NO_AUDIO_STREAM_REMEDIATION, RESUMED_FROM_CHECKPOINT_WARNING, SUPERSEDED_REMEDIATION,
    UNKNOWN_REVISION_REMEDIATION, UNPINNED_MODEL_REMEDIATION, job_warning_messages,
    local_asr_failure_summary, local_asr_verification_summary,
};
pub use navigation::{
    AUDIO_RANGE_REMEDIATION, AUDIO_RANGE_START_REMEDIATION, AudioData, AudioEvidenceData,
    AudioEvidenceStream, AudioStreamData, BURST_RANGE_REMEDIATION, CROP_OUTSIDE_REMEDIATION,
    DeliveredEvidenceFile, EVIDENCE_BUDGET_REMEDIATION, EVIDENCE_KIND_REMEDIATION,
    EVIDENCE_PATH_REMEDIATION, EVIDENCE_TOOLS_REMEDIATION, EvidencePresentation,
    EvidencePresentationError, FrameData, FrameEvidenceData, FrameEvidenceStream, FrameStreamData,
    NO_AUDIO_CLIP_REMEDIATION, NO_FRAMES_REMEDIATION, UNDECODABLE_EVIDENCE_REMEDIATION,
    UNKNOWN_CANDIDATE_REMEDIATION, UNKNOWN_EVIDENCE_REMEDIATION, audio_response, frame_response,
    frame_selection_summary, partial_evidence_warning,
};
pub use search::{
    MAX_COVERAGE_RANGES, SearchData, SearchEvidenceStream, SearchPresentation, SearchStreamData,
    UNTRANSCRIBED_SEARCH_WARNING, search_query_rejection_summary, search_response,
};
pub use session::{
    BundleData, BundleSourceInclusion, CleanData, CleanItem, CleanItemOutcome, ListedSession,
    OpenData, PageData, SessionState, StatusData,
};
pub use setup::{
    ConfiguredModelResponse, ConfiguredSelectionResponse, DependencyLookup, SavedSetupPlan,
    SetupCheckResponse, SetupPlanResponse, explicit_path_option,
};
pub use storage::{PrivateFolder, non_private_folder_summary};
pub use stream::{
    EventKind, EvidenceEventResponse, EvidenceRecordType, EvidenceStream, TranscriptEvidenceStream,
    TranscriptStreamData,
};
pub use text::{MAX_PROVIDER_DETAIL_BYTES, sanitize_untrusted_text};
pub use transcript::{
    MEDIA_TOOLS_FOR_TRANSCRIPT_REMEDIATION, NO_TRANSCRIPT_REMEDIATION, RetranscribeJob,
    SourceSegmentData, TranscriptPageData, TranscriptRetranscribeData, TranscriptRevisionData,
    TranscriptSegmentData, transcript_rejection_summary, transcript_warning_messages,
};
pub use verification::media_tool_verification_summary;
