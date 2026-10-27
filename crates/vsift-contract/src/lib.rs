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
//!   [`SavedSetupPlan`] input, and the configured-selection responses; the managed
//!   lifecycle data [`SetupListResponse`], [`SetupRollbackResponse`],
//!   [`SetupRemoveResponse`] and [`SetupRepairResponse`] (P13 PR 6).
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
//!   [`TranscriptPageData`] built from a [`TranscriptPresentation`] by
//!   [`transcript_get_response`] (which also fills the envelope coverage and
//!   the `partial` status when the range has untranscribed parts, #373), the
//!   `transcript.retranscribe` result [`TranscriptRetranscribeData`], and
//!   fixed-prose warnings and remediation.
//! - **Views:** a [`ResultView`] asks `transcript.get`, `search` and
//!   `candidates` for the full answer (the default, unchanged) or the brief
//!   one ([`TranscriptBriefPageData`], [`SearchBriefData`],
//!   [`CandidatesBriefData`]), which states once what every item of a
//!   revision or index repeats. Each brief type is projected from its full
//!   type, and a stream is never brief ([`BRIEF_VIEW_STREAM_REMEDIATION`]).
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
//!   for an existing [`PrivateFolder`] that other accounts can access, and
//!   [`UNOWNED_SESSION_ROOT_REMEDIATION`] for a session root `VSift` did not
//!   create.
//! - **Text:** [`sanitize_untrusted_text`], the one rule for placing untrusted
//!   provider text in public output; [`render_hidden_characters`], which
//!   writes every [`is_hidden_character`] as `<U+XXXX>` for the `display_text`
//!   of transcript segments; and [`terminal_safe_text`], both rules at once
//!   for a host's terminal diagnostics.
//! - **Handoff check (P13 PR 5):** [`HandoffChecker`] checks an agent's
//!   draft report against the skill-owned `handoff.schema.json` (embedded
//!   as [`HANDOFF_SCHEMA_JSON`] and validated by a subset validator that
//!   refuses any schema feature it does not implement), the handoff rules
//!   and the report-text rules; [`HandoffCheckData`] is the `handoff.check`
//!   data, whose findings never repeat the draft. The trial grader uses the
//!   same checker, so the command and the grader cannot disagree.
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
mod delivered_path;
mod envelope;
mod events;
mod evidence;
mod growth;
mod handoff;
mod input;
mod install;
mod job;
mod lifecycle;
mod local_asr;
mod navigation;
mod optional_component;
mod request;
mod screen_text;
mod search;
mod session;
mod setup;
mod storage;
mod stream;
mod text;
mod transcript;
mod verification;
mod view;
mod watch;
mod work;
mod workspace;

pub use batch::{BatchDataError, BatchItemStatus, BatchOutcome, BatchTermination, JobBatchData};
pub use events::{
    LifecycleEventResponse, LifecycleKind, LifecycleReason, MAX_EVENT_LINE_BYTES,
    MAX_PROGRESS_EVENTS, PROGRESS_INTERVAL_MS, ProgressEventResponse, ProgressReport, Readiness,
    RequestEnd, RequestRef, ResultEventResponse,
};
pub use growth::{
    PendingCover, ReadGrowth, SinceRead, TRANSCRIPTION_PENDING_REASON,
    TRANSCRIPTION_PENDING_WARNING,
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
    ADMISSION_BUSY_REMEDIATION, ADMISSION_CAPACITY_REMEDIATION, BATCH_CONCURRENCY_REMEDIATION,
    BATCH_FILE_REMEDIATION, BATCH_LINE_LIMIT_REMEDIATION, BATCH_LINES_FAILED_REMEDIATION,
    BATCH_STOPPED_REMEDIATION, BUNDLE_MISMATCH_REMEDIATION, BUNDLE_ROOT_REQUIRED_REMEDIATION,
    DEFAULT_SESSION_RETENTION_SECONDS, DURABILITY_UNAVAILABLE_REMEDIATION,
    INPUT_NOT_FOUND_REMEDIATION, INPUT_NOT_REGULAR_FILE_REMEDIATION, INPUT_ROOT_REMEDIATION,
    INPUT_UNREADABLE_REMEDIATION, ISOLATION_UNAVAILABLE_REMEDIATION, MAX_ADMISSION_CAPACITY,
    MAX_SESSION_RETENTION_SECONDS, MIN_SESSION_RETENTION_SECONDS, PARTIAL_BATCH_WARNING,
    REQUEST_BUSY_REMEDIATION, REQUEST_CONFLICT_REMEDIATION, REQUEST_DEADLINE_REMEDIATION,
    REQUEST_FILE_REMEDIATION, REQUEST_SESSION_REMEDIATION, REQUEST_STOPPED_REMEDIATION,
    REQUEST_UNRECORDED_REMEDIATION, WORKER_WORKSPACE_REQUIRED_REMEDIATION,
    WORKSPACE_NOT_DURABLE_REMEDIATION, WORKSPACE_POLICY_MISMATCH_REMEDIATION,
    WORKSPACE_ROOT_REMEDIATION, WorkspaceData, WorkspaceInitOutcome, WorkspacePolicyError,
};

pub use candidates::{
    CANDIDATE_CURSOR_REMEDIATION, CandidatesBriefData, CandidatesData, CandidatesEvidenceStream,
    CandidatesPresentation, CandidatesStreamData, NO_VIDEO_STREAM_REMEDIATION,
    VISUAL_COVERAGE_WARNING, VISUAL_TOOLS_REMEDIATION, VisualCandidateData, candidates_response,
    candidates_response_with_view,
};
pub use command::CommandName;
pub use envelope::{
    CONTRACT_VERSION, CoverageResponse, ErrorResponse, LifecycleResponse, MAX_AFFECTED_IDS,
    OperationResponse, TerminalEventResponse, utc_seconds_rfc3339,
};
pub use evidence::{ConfidenceResponse, FrameTimingResponse};
pub use handoff::{
    COMPACT_BUDGET, HANDOFF_DRAFT_NOT_UTF8_REMEDIATION, HANDOFF_DRAFT_TOO_LARGE_REMEDIATION,
    HANDOFF_FILE_REMEDIATION, HANDOFF_SCHEMA_JSON, HANDOFF_VERSION, HandoffBudgetLimits,
    HandoffCheck, HandoffCheckData, HandoffChecker, HandoffEvidenceKind, HandoffFinding,
    HandoffRule, HandoffRuleScope, HandoffSchemaError, HandoffSessionCheck, HandoffSessionGap,
    HandoffSessionRecords, HandoffVocabulary, MAX_HANDOFF_FINDINGS, MAX_HANDOFF_REPORT_BYTES,
    MAX_RESUME_CARD_BYTES, STANDARD_BUDGET, budget_profile, extract_handoff_block,
    handoff_is_cut_short, handoff_pattern_has_message, handoff_report_text_findings,
    handoff_vocabulary,
};
pub use install::{
    ARTIFACT_DIRECTORY_REMEDIATION, InstallSource, MANAGED_INSTALL_BUSY_REMEDIATION,
    MANAGED_STORAGE_REMEDIATION, MANAGED_UNAVAILABLE_REMEDIATION,
    OPTIONAL_COMPONENT_NOT_INSTALLED_WARNING, STALE_PLAN_REMEDIATION, SetupInstallResponse,
    optional_install_failure_summary, setup_install_failure_summary,
};
pub use job::{
    ESTIMATE_BASIS, JOB_CANCELLED_REMEDIATION, JOB_INTERRUPTED_REMEDIATION,
    JOB_NOT_RESUMABLE_REMEDIATION, JOB_SESSION_NOT_OPEN_REMEDIATION, JobData, JobPresentation,
    JobResumeData, JobRunner, JobStartData, MediaProgressData, RUNNER_NOT_STARTED_REMEDIATION,
    STREAM_NOT_STARTED, STREAM_SKIPPED, SessionJobData, SessionStatusData, StreamPresentation,
    UNKNOWN_JOB_REMEDIATION,
};
pub use lifecycle::{
    InstallCleanupResponse, SetupListResponse, SetupRemoveResponse, SetupRepairResponse,
    SetupRollbackResponse, SuggestedCommandResponse, managed_lifecycle_remediation,
    repair_fix_command, setup_remove_failure_summary,
};
pub use local_asr::{
    CANCELLATION_TOO_LATE_WARNING, CHECKPOINT_DISCARDED_WARNING, IDEMPOTENCY_CONFLICT_REMEDIATION,
    JOB_BUSY_REMEDIATION, LOCAL_ASR_MODEL_REMEDIATION, LOCAL_ASR_TOOLS_REMEDIATION,
    NO_AUDIO_STREAM_REMEDIATION, RESUMED_FROM_CHECKPOINT_WARNING, REVISION_LEFT_OPEN_WARNING,
    SUPERSEDED_REMEDIATION, UNKNOWN_REVISION_REMEDIATION, UNPINNED_MODEL_REMEDIATION,
    job_warning_messages, local_asr_failure_summary, local_asr_verification_summary,
};
pub use navigation::{
    AUDIO_RANGE_REMEDIATION, AUDIO_RANGE_START_REMEDIATION, AUDIO_RANGE_TOO_SHORT_REMEDIATION,
    AudioData, AudioEvidenceData, AudioEvidenceStream, AudioStreamData, BURST_RANGE_REMEDIATION,
    CROP_OUTSIDE_REMEDIATION, DeliveredEvidenceFile, EVIDENCE_BUDGET_REMEDIATION,
    EVIDENCE_KIND_REMEDIATION, EVIDENCE_PATH_REMEDIATION, EVIDENCE_TOOLS_REMEDIATION,
    EvidencePresentation, EvidencePresentationError, FrameData, FrameEvidenceData,
    FrameEvidenceStream, FrameStreamData, MEDIA_BUSY_REMEDIATION, NO_AUDIO_CLIP_REMEDIATION,
    NO_FRAMES_REMEDIATION, REVISION_LEFT_OPEN_REMEDIATION, TRANSCRIPTION_RUNNING_REMEDIATION,
    UNDECODABLE_EVIDENCE_REMEDIATION, UNKNOWN_CANDIDATE_REMEDIATION, UNKNOWN_EVIDENCE_REMEDIATION,
    audio_response, frame_response, frame_selection_summary, partial_evidence_warning,
};
pub use optional_component::{
    OptionalComponentText, SITE_TERMS_NOTICE, include_command_arguments, optional_check_step,
    optional_component_text,
};
pub use screen_text::{
    CANDIDATES_INCOMPLETE_REASON, FRAMES_AWAITING_ANALYSIS_REASON, FRAMES_FAILED_REASON,
    FRAMES_PENDING_REASON, NOT_RUN_REASON, OCR_FAILED_REMEDIATION, OCR_INDEX_FULL_REMEDIATION,
    OCR_RUNNER_REMEDIATION, OCR_SEARCH_CURSOR_REMEDIATION, OCR_SEARCH_PARTIAL_WARNING,
    SCREEN_TEXT_PARTIAL_WARNING, SCREEN_TEXT_SCOPE, ScreenTextData, ScreenTextHitPresentation,
    ScreenTextPresentation, ScreenTextSearchData, ScreenTextSearchPageData,
    ScreenTextSearchPagePresentation, ScreenTextSearchPresentation, ScreenTextUnavailable,
    UnreadableIndex, UnreadableReason, screen_text_response, screen_text_search_response,
};
pub use search::{
    MAX_COVERAGE_RANGES, SearchBriefData, SearchData, SearchEvidenceStream, SearchPresentation,
    SearchStreamData, UNPLACED_WORDS_SEARCH_WARNING, UNTRANSCRIBED_SEARCH_WARNING,
    search_query_rejection_summary, search_response, search_response_with_view,
};
pub use session::{
    BundleData, BundleSourceInclusion, CleanData, CleanItem, CleanItemOutcome, ListedSession,
    OpenData, PageData, SessionState, StatusData,
};
pub use setup::{
    ConfiguredModelResponse, ConfiguredSelectionResponse, DependencyLookup, MAX_PROBE_TIMEOUT,
    NO_FOLDER_SEARCH_ADVICE, OPERATOR_DECISION_INSTRUCTION, OptionalComponentView,
    ProbeTimeHeadroom, SavedSetupPlan, SetupCheckResponse, SetupPlanResponse, explicit_path_option,
    managed_update_next_step, probe_deadline_text,
};
pub use storage::{
    PrivateFolder, SESSION_NOT_PUBLISHED_REMEDIATION, SESSION_ROOT_NAME_REMEDIATION,
    SESSION_ROOT_OWN_ACCESS_REMEDIATION, SOURCE_ACCESS_DENIED_REMEDIATION,
    SOURCE_COPY_TOO_SLOW_REMEDIATION, SOURCE_COPY_WOULD_BE_TOO_SLOW_REMEDIATION,
    SOURCE_IS_FOLDER_REMEDIATION, SOURCE_IS_LINK_REMEDIATION, SOURCE_NO_ROOM_REMEDIATION,
    SOURCE_NOT_FOUND_REMEDIATION, SOURCE_SPECIAL_FILE_REMEDIATION,
    UNOWNED_SESSION_ROOT_REMEDIATION, non_private_folder_summary,
};
pub use stream::{
    EventKind, EvidenceEventResponse, EvidenceRecordType, EvidenceStream, TranscriptEvidenceStream,
    TranscriptStreamData,
};
pub use text::{
    HIDDEN_CHARACTER_GROWTH, HIDDEN_CHARACTER_UNICODE_VERSION, MAX_PROVIDER_DETAIL_BYTES,
    is_hidden_character, render_hidden_characters, sanitize_untrusted_text, terminal_safe_text,
};
pub use transcript::{
    MEDIA_TOOLS_FOR_TRANSCRIPT_REMEDIATION, NO_TRANSCRIPT_REMEDIATION,
    PROVIDER_CHUNKS_REJECTED_WARNING, PROVIDER_SEGMENTS_UNPLACED_WARNING, RetranscribeJob,
    SourceSegmentData, TRANSCRIPT_INTERRUPTED_REMEDIATION, TRANSCRIPT_QUEUED_REMEDIATION,
    TRANSCRIPT_RUNNING_REMEDIATION, TranscriptBriefPageData, TranscriptPageData,
    TranscriptPresentation, TranscriptRetranscribeData, TranscriptRevisionData,
    TranscriptSegmentData, UNTRANSCRIBED_READ_WARNING, finish_retranscription,
    retranscription_gaps, transcript_get_response, transcript_rejection_summary,
    transcript_warning_messages,
};
pub use verification::media_tool_verification_summary;
pub use view::{BRIEF_VIEW_STREAM_REMEDIATION, ResultView};
pub use watch::{
    WATCH_FOLDER_EXISTS_REMEDIATION, WATCH_FRAME_MISMATCH_REMEDIATION,
    WATCH_NAME_EMPTY_REMEDIATION, WATCH_NAME_NOT_UNICODE_REMEDIATION,
    WATCH_OUTPUT_FOLDER_REMEDIATION, WATCH_OUTPUT_INSIDE_VSIFT_REMEDIATION,
    WATCH_OUTPUT_REFUSED_REMEDIATION, WATCH_SUBTITLES_EXISTS_REMEDIATION,
    WATCH_TRANSCRIPT_EXISTS_REMEDIATION, WatchCompleteness, WatchData, WatchFrameData,
    WatchOutputData, WatchOutputsData, WatchRangeData,
};
