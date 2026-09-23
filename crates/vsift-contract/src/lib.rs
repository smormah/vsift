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
//!   [`CoverageResponse`] and [`LifecycleResponse`] parts, and the JSON Lines
//!   [`TerminalEventResponse`].
//! - **Setup:** [`SetupCheckResponse`], [`SetupPlanResponse`], the strict
//!   [`SavedSetupPlan`] input, and the configured-selection responses.
//! - **Session:** [`OpenData`], [`StatusData`], [`PageData`], [`CleanData`],
//!   [`BundleData`] and their item types.
//! - **Evidence:** [`ConfidenceResponse`] and [`FrameTimingResponse`], frozen
//!   before the packets that produce them.
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

mod envelope;
mod evidence;
mod session;
mod setup;
mod text;

pub use envelope::{
    CONTRACT_VERSION, CoverageResponse, ErrorResponse, LifecycleResponse, OperationResponse,
    TerminalEventResponse,
};
pub use evidence::{ConfidenceResponse, FrameTimingResponse};
pub use session::{
    BundleData, BundleSourceInclusion, CleanData, CleanItem, CleanItemOutcome, ListedSession,
    OpenData, PageData, SessionState, StatusData,
};
pub use setup::{
    ConfiguredModelResponse, ConfiguredSelectionResponse, DependencyLookup, SavedSetupPlan,
    SetupCheckResponse, SetupPlanResponse, explicit_path_option,
};
pub use text::{MAX_PROVIDER_DETAIL_BYTES, sanitize_untrusted_text};
