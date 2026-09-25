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
//!   [`BundleData`] and their item types.
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
//! - **Verification:** [`media_tool_verification_summary`], the fixed-prose
//!   remediation for a failed automatic media-tool preflight.
//! - **Storage:** [`non_private_folder_summary`], the fixed-prose remediation
//!   for an existing [`PrivateFolder`] that other accounts can access.
//! - **Text:** [`sanitize_untrusted_text`], the one rule for placing untrusted
//!   provider text in public output.
//!
//! The published JSON Schemas under `schemas/v1` are authoritative. This crate's
//! tests validate its serialized values against them. The Rust API itself is 0.x
//! and unstable (ADR 0016 decision 3); only the JSON it produces is stable.
//!
//! Dependencies point inward: this crate depends on `vsift-domain` and
//! `vsift-application` only, never on infrastructure or a host. Host concerns such
//! as exit codes, human text, output budgets and clock formatting stay in the host.

#![forbid(unsafe_code)]

mod command;
mod envelope;
mod evidence;
mod local_asr;
mod session;
mod setup;
mod storage;
mod stream;
mod text;
mod transcript;
mod verification;

pub use command::CommandName;
pub use envelope::{
    CONTRACT_VERSION, CoverageResponse, ErrorResponse, LifecycleResponse, OperationResponse,
    TerminalEventResponse,
};
pub use evidence::{ConfidenceResponse, FrameTimingResponse};
pub use local_asr::{
    LOCAL_ASR_MODEL_REMEDIATION, LOCAL_ASR_TOOLS_REMEDIATION, NO_AUDIO_STREAM_REMEDIATION,
    UNKNOWN_REVISION_REMEDIATION, UNPINNED_MODEL_REMEDIATION, local_asr_failure_summary,
    local_asr_verification_summary,
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
    EventKind, EvidenceEventResponse, EvidenceRecordType, TranscriptEvidenceStream,
    TranscriptStreamData,
};
pub use text::{MAX_PROVIDER_DETAIL_BYTES, sanitize_untrusted_text};
pub use transcript::{
    MEDIA_TOOLS_FOR_TRANSCRIPT_REMEDIATION, NO_TRANSCRIPT_REMEDIATION, SourceSegmentData,
    TranscriptPageData, TranscriptRetranscribeData, TranscriptRevisionData, TranscriptSegmentData,
    transcript_rejection_summary, transcript_warning_messages,
};
pub use verification::media_tool_verification_summary;
