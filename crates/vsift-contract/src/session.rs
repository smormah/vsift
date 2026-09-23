//! Data carried by `ingest`, `session` and `bundle` results.
//!
//! Session status and bundle facts come from the host's session store, which is
//! infrastructure this crate may not depend on. The contract therefore takes
//! domain values and small typed vocabularies ([`SessionState`],
//! [`CleanItemOutcome`], [`BundleSourceInclusion`]) and owns every derived field
//! and identifier, so hosts cannot disagree on them. RFC 3339 timestamps are
//! formatted by the host, which owns the clock.

use serde::Serialize;
use vsift_application::OpenSessionOutcome;
use vsift_domain::{
    FailureCode, PublicationGuarantee, SessionId, SessionLifetime, SessionPhase, SourceId,
};

/// Publicly reported lifecycle state of one session.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Indexed, but its first generation is not yet published.
    Initializing,
    /// Open and within its expiry.
    Open,
    /// Open in storage, but past its expiry and eligible for cleanup.
    Expired,
    /// Closed by an explicit request.
    Closed,
    /// Its record could not be read; the item carries an error code.
    Unavailable,
}

impl SessionState {
    /// Derives the reported state of a committed session at `now_unix_seconds`.
    ///
    /// Expiry is observed, not stored: an open session past its expiry is
    /// reported as expired even before cleanup has run.
    #[must_use]
    pub const fn observed(
        phase: SessionPhase,
        lifetime: SessionLifetime,
        now_unix_seconds: u64,
    ) -> Self {
        match phase {
            SessionPhase::Closed => Self::Closed,
            SessionPhase::Open if lifetime.expired(now_unix_seconds) => Self::Expired,
            SessionPhase::Open => Self::Open,
        }
    }

    /// Returns the stable machine-readable state identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Initializing => "initializing",
            Self::Open => "open",
            Self::Expired => "expired",
            Self::Closed => "closed",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Data of a successful `ingest` result: the newly opened disposable session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OpenData {
    session_id: String,
    source_id: String,
    source_bytes: u64,
    generation: u64,
    publication: &'static str,
    expires_at: String,
}

impl OpenData {
    /// Presents an opened session with its host-formatted RFC 3339 expiry.
    #[must_use]
    pub fn new(opened: &OpenSessionOutcome, expires_at: String) -> Self {
        Self {
            session_id: opened.session_id.as_str().to_owned(),
            source_id: opened.source_id.as_str().to_owned(),
            source_bytes: opened.source_bytes,
            generation: opened.generation.value(),
            publication: opened.publication.identifier(),
            expires_at,
        }
    }
}

/// Committed status of one session, as returned by `session status`, `renew`
/// and `close` and embedded in `session list` items.
///
/// Fields are public because every value is a plain fact read from the host's
/// session store; there is nothing to derive except [`SessionState`], which has
/// its own constructor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StatusData {
    /// Public session identifier.
    pub session_id: String,
    /// Reported lifecycle state.
    pub state: SessionState,
    /// SHA-256 identity of the private source copy.
    pub source_id: String,
    /// Size of the private source copy.
    pub source_bytes: u64,
    /// Number of committed evidence artifacts.
    pub artifact_count: usize,
    /// Total bytes of committed evidence artifacts.
    pub artifact_bytes: u64,
    /// Committed storage generation.
    pub generation: u64,
    /// RFC 3339 expiry, formatted by the host.
    pub expires_at: String,
}

/// One page of `session list` results.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PageData {
    /// Sessions on this page, in index order.
    pub items: Vec<ListedSession>,
    /// Opaque cursor for the next page, absent on the last page.
    pub next_cursor: Option<u16>,
}

/// One `session list` item.
///
/// Constructors keep `state`, `status` and `error_code` consistent: an item has
/// status only when it was read, and an error code only when it was not.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ListedSession {
    session_id: String,
    state: SessionState,
    status: Option<StatusData>,
    error_code: Option<&'static str>,
}

impl ListedSession {
    /// A session whose committed status was read.
    #[must_use]
    pub fn indexed(session_id: &SessionId, status: StatusData) -> Self {
        Self {
            session_id: session_id.as_str().to_owned(),
            state: status.state,
            status: Some(status),
            error_code: None,
        }
    }

    /// A session that is indexed but has not yet published its first generation.
    #[must_use]
    pub fn initializing(session_id: &SessionId) -> Self {
        Self {
            session_id: session_id.as_str().to_owned(),
            state: SessionState::Initializing,
            status: None,
            error_code: None,
        }
    }

    /// A session whose record could not be read; the page becomes partial.
    #[must_use]
    pub fn unavailable(session_id: &SessionId, code: FailureCode) -> Self {
        Self {
            session_id: session_id.as_str().to_owned(),
            state: SessionState::Unavailable,
            status: None,
            error_code: Some(code.identifier()),
        }
    }
}

/// What cleanup decided for one examined session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanItemOutcome {
    /// Still open and within its expiry; left alone.
    Ineligible,
    /// A dry run found it closed, expired or abandoned.
    Eligible,
    /// It was removed.
    Removed,
}

impl CleanItemOutcome {
    /// Returns the stable machine-readable outcome identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Ineligible => "ineligible",
            Self::Eligible => "eligible",
            Self::Removed => "removed",
        }
    }
}

/// One `session clean` item.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CleanItem {
    session_id: String,
    outcome: &'static str,
    error_code: Option<&'static str>,
}

impl CleanItem {
    /// A session that cleanup examined and decided on.
    #[must_use]
    pub fn examined(session_id: &SessionId, outcome: CleanItemOutcome) -> Self {
        Self {
            session_id: session_id.as_str().to_owned(),
            outcome: outcome.identifier(),
            error_code: None,
        }
    }

    /// A session cleanup could not examine; the page becomes partial.
    #[must_use]
    pub fn skipped(session_id: &SessionId, code: FailureCode) -> Self {
        Self {
            session_id: session_id.as_str().to_owned(),
            outcome: "skipped",
            error_code: Some(code.identifier()),
        }
    }
}

/// One page of `session clean` results.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CleanData {
    /// Sessions examined on this page, in index order.
    pub items: Vec<CleanItem>,
    /// Opaque cursor for the next page, absent on the last page.
    pub next_cursor: Option<u16>,
    /// Whether this was a dry run that removed nothing.
    pub dry_run: bool,
}

/// Whether a retained bundle carries its own copy of the source media.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BundleSourceInclusion {
    /// Evidence metadata only; re-extraction needs the matching original.
    EvidenceOnly,
    /// A verified copy of the source is inside the bundle.
    SourceIncluded,
}

/// Data of `session retain` and `bundle validate` results.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BundleData {
    session_id: String,
    source_id: String,
    source_bytes: u64,
    source_included: bool,
    artifact_count: usize,
    artifact_bytes: u64,
    reextraction_requires_matching_original: bool,
    publication: &'static str,
}

impl BundleData {
    /// Presents a validated retained bundle.
    ///
    /// Bundles are published with process-crash consistency only (strict OS
    /// durability is unqualified, FS-01), and the response says so.
    #[must_use]
    pub fn new(
        session_id: &SessionId,
        source_id: &SourceId,
        source_bytes: u64,
        source: BundleSourceInclusion,
        artifact_count: usize,
        artifact_bytes: u64,
    ) -> Self {
        let source_included = source == BundleSourceInclusion::SourceIncluded;
        Self {
            session_id: session_id.as_str().to_owned(),
            source_id: source_id.as_str().to_owned(),
            source_bytes,
            source_included,
            artifact_count,
            artifact_bytes,
            reextraction_requires_matching_original: !source_included,
            publication: PublicationGuarantee::ProcessCrashConsistent.identifier(),
        }
    }
}

#[cfg(test)]
mod tests {
    use vsift_application::OpenSessionOutcome;
    use vsift_domain::{
        FailureCode, PublicationGuarantee, SessionId, SessionLifetime, SessionPhase, SourceId,
        StorageGeneration,
    };

    use super::{
        BundleData, BundleSourceInclusion, CleanItem, CleanItemOutcome, ListedSession, OpenData,
        SessionState, StatusData,
    };

    const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
    const SOURCE: &str =
        "src_sha256_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn opened_session_reports_identity_generation_and_guarantee()
    -> Result<(), Box<dyn std::error::Error>> {
        let opened = OpenSessionOutcome {
            session_id: SessionId::parse(SESSION)?,
            source_id: SourceId::parse(SOURCE)?,
            source_bytes: 10,
            generation: StorageGeneration::from_value(1),
            publication: PublicationGuarantee::ProcessCrashConsistent,
            lifetime: SessionLifetime::open(1_000)?,
        };

        let value =
            serde_json::to_value(OpenData::new(&opened, String::from("2026-09-24T00:00:00Z")))?;

        assert_eq!(
            value,
            serde_json::json!({
                "session_id": SESSION,
                "source_id": SOURCE,
                "source_bytes": 10,
                "generation": 1,
                "publication": "process_crash_consistent",
                "expires_at": "2026-09-24T00:00:00Z"
            })
        );
        Ok(())
    }

    #[test]
    fn serialized_state_matches_its_identifier() -> Result<(), Box<dyn std::error::Error>> {
        for state in [
            SessionState::Initializing,
            SessionState::Open,
            SessionState::Expired,
            SessionState::Closed,
            SessionState::Unavailable,
        ] {
            assert_eq!(serde_json::to_value(state)?, state.identifier());
        }
        Ok(())
    }

    #[test]
    fn expiry_is_observed_rather_than_stored() -> Result<(), Box<dyn std::error::Error>> {
        let lifetime = SessionLifetime::open(1_000)?;
        let expiry = lifetime.expires_at_unix_seconds();

        assert_eq!(
            SessionState::observed(SessionPhase::Open, lifetime, 1_000),
            SessionState::Open
        );
        assert_eq!(
            SessionState::observed(SessionPhase::Open, lifetime, expiry + 1),
            SessionState::Expired
        );
        assert_eq!(
            SessionState::observed(SessionPhase::Closed, lifetime, 1_000),
            SessionState::Closed
        );
        Ok(())
    }

    #[test]
    fn listed_items_carry_status_or_an_error_never_both() -> Result<(), Box<dyn std::error::Error>>
    {
        let session = SessionId::parse(SESSION)?;
        let status = StatusData {
            session_id: String::from(SESSION),
            state: SessionState::Expired,
            source_id: String::from(SOURCE),
            source_bytes: 10,
            artifact_count: 0,
            artifact_bytes: 0,
            generation: 2,
            expires_at: String::from("2026-09-24T00:00:00Z"),
        };

        let indexed = serde_json::to_value(ListedSession::indexed(&session, status))?;
        let initializing = serde_json::to_value(ListedSession::initializing(&session))?;
        let unavailable = serde_json::to_value(ListedSession::unavailable(
            &session,
            FailureCode::IntegrityFailure,
        ))?;

        assert_eq!(indexed["state"], "expired");
        assert_eq!(indexed["status"]["state"], "expired");
        assert!(indexed["error_code"].is_null());
        assert_eq!(initializing["state"], "initializing");
        assert!(initializing["status"].is_null());
        assert_eq!(unavailable["state"], "unavailable");
        assert!(unavailable["status"].is_null());
        assert_eq!(unavailable["error_code"], "INTEGRITY_FAILURE");
        Ok(())
    }

    #[test]
    fn clean_items_report_outcome_or_skip_reason() -> Result<(), Box<dyn std::error::Error>> {
        let session = SessionId::parse(SESSION)?;

        let removed =
            serde_json::to_value(CleanItem::examined(&session, CleanItemOutcome::Removed))?;
        let skipped = serde_json::to_value(CleanItem::skipped(&session, FailureCode::Busy))?;

        assert_eq!(
            removed,
            serde_json::json!({"session_id": SESSION, "outcome": "removed", "error_code": null})
        );
        assert_eq!(
            skipped,
            serde_json::json!({"session_id": SESSION, "outcome": "skipped", "error_code": "BUSY"})
        );
        Ok(())
    }

    #[test]
    fn evidence_only_bundles_require_the_matching_original()
    -> Result<(), Box<dyn std::error::Error>> {
        let session = SessionId::parse(SESSION)?;
        let source = SourceId::parse(SOURCE)?;

        let evidence_only = serde_json::to_value(BundleData::new(
            &session,
            &source,
            10,
            BundleSourceInclusion::EvidenceOnly,
            1,
            5,
        ))?;
        let with_source = serde_json::to_value(BundleData::new(
            &session,
            &source,
            10,
            BundleSourceInclusion::SourceIncluded,
            1,
            5,
        ))?;

        assert_eq!(evidence_only["source_included"], false);
        assert_eq!(
            evidence_only["reextraction_requires_matching_original"],
            true
        );
        assert_eq!(with_source["source_included"], true);
        assert_eq!(
            with_source["reextraction_requires_matching_original"],
            false
        );
        assert_eq!(with_source["publication"], "process_crash_consistent");
        Ok(())
    }
}
