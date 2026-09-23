//! Disposable-session operations: open, inspect, renew, close, retain, clean,
//! and validate retained bundles.

use std::path::{Path, PathBuf};

use vsift_application::{OpenSession, OpenSessionOutcome, OpenSessionRequest};
use vsift_domain::{
    DurabilityRequirement, SessionId, SessionLifetime, SessionPhase, SourceId, StorageGeneration,
    TranscriptRevision,
};
use vsift_infrastructure::{
    BundleSourcePolicy, BundleStatus, CleanOutcome, FfprobeSourceDuration, FilesystemSessionStore,
    ProcessCancellation, SessionIndexPage, SessionRootProvisioning, SessionStatus,
};

use crate::{
    engine::{Engine, absolute_selection},
    error::{EngineError, SessionRootError},
};

/// A request to open a local video as a new disposable session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestRequest {
    /// Local source media; a relative path is resolved against the process
    /// working directory.
    pub source: PathBuf,
    /// Optional supplied `SubRip` or `WebVTT` transcript to import with it.
    pub transcript: Option<SuppliedTranscriptRequest>,
}

/// A supplied transcript file and the explicit offset that aligns it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SuppliedTranscriptRequest {
    /// Local sidecar file; a relative path is resolved against the process
    /// working directory.
    pub path: PathBuf,
    /// Signed microseconds added to every sidecar timestamp to reach source
    /// time; at most twenty-four hours either way.
    pub offset_micros: i64,
}

/// A newly opened session and, when one was supplied, its transcript revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestOutcome {
    /// The opened session.
    pub session: OpenSessionOutcome,
    /// The imported revision, committed in the same generation as the source.
    pub transcript: Option<TranscriptRevision>,
}

/// Committed facts about one session, observed at a known time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSnapshot {
    session_id: SessionId,
    source_id: SourceId,
    source_bytes: u64,
    phase: SessionPhase,
    lifetime: SessionLifetime,
    generation: StorageGeneration,
    artifact_count: usize,
    artifact_bytes: u64,
    observed_at_unix_seconds: u64,
}

impl SessionSnapshot {
    pub(crate) fn observe(status: &SessionStatus, observed_at_unix_seconds: u64) -> Self {
        Self {
            session_id: status.session_id().clone(),
            source_id: status.source_id().clone(),
            source_bytes: status.source_bytes(),
            phase: status.phase(),
            lifetime: status.lifetime(),
            generation: status.generation(),
            artifact_count: status.artifact_count(),
            artifact_bytes: status.artifact_bytes(),
            observed_at_unix_seconds,
        }
    }

    /// Session identity.
    #[must_use]
    pub const fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Hash identity of the private source copy.
    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    /// Size of the private source copy.
    #[must_use]
    pub const fn source_bytes(&self) -> u64 {
        self.source_bytes
    }

    /// Committed lifecycle phase.
    #[must_use]
    pub const fn phase(&self) -> SessionPhase {
        self.phase
    }

    /// Committed opening and expiry times.
    #[must_use]
    pub const fn lifetime(&self) -> SessionLifetime {
        self.lifetime
    }

    /// Committed storage generation.
    #[must_use]
    pub const fn generation(&self) -> StorageGeneration {
        self.generation
    }

    /// Number of committed evidence artifacts.
    #[must_use]
    pub const fn artifact_count(&self) -> usize {
        self.artifact_count
    }

    /// Total bytes of committed evidence artifacts.
    #[must_use]
    pub const fn artifact_bytes(&self) -> u64 {
        self.artifact_bytes
    }

    /// Clock reading at which the snapshot was taken.
    ///
    /// Expiry is observed rather than stored, so a presenter derives whether an
    /// open session has expired from this time and [`SessionSnapshot::lifetime`].
    #[must_use]
    pub const fn observed_at_unix_seconds(&self) -> u64 {
        self.observed_at_unix_seconds
    }
}

/// One entry of a session listing page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionListEntry {
    /// A session whose committed status was read.
    Indexed(SessionSnapshot),
    /// A session that is indexed but has not yet published its first generation.
    Initializing(SessionId),
    /// A session whose record could not be read.
    Unavailable {
        /// Identity from the index.
        session_id: SessionId,
        /// Why its record could not be read.
        error: EngineError,
    },
}

/// One bounded page of the session index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionPage {
    entries: Vec<SessionListEntry>,
    next_cursor: Option<u16>,
}

impl SessionPage {
    /// Entries on this page, in index order.
    #[must_use]
    pub fn entries(&self) -> &[SessionListEntry] {
        &self.entries
    }

    /// Consumes the page, returning its entries.
    #[must_use]
    pub fn into_entries(self) -> Vec<SessionListEntry> {
        self.entries
    }

    /// Cursor for the next page, absent on the last page.
    #[must_use]
    pub const fn next_cursor(&self) -> Option<u16> {
        self.next_cursor
    }

    /// Whether any entry on the page could not be read.
    #[must_use]
    pub fn is_partial(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| matches!(entry, SessionListEntry::Unavailable { .. }))
    }
}

/// Which sessions a cleanup request may consider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanScope {
    /// Only sessions proven closed, expired or abandoned.
    Expired,
    /// No restriction. Always rejected: cleanup never removes a session it has
    /// not proven expired, and the request must say so explicitly.
    Unrestricted,
}

/// Whether cleanup removes eligible sessions or only reports them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanMode {
    /// Report eligible sessions without removing anything.
    DryRun,
    /// Remove eligible sessions.
    Remove,
}

/// A bounded cleanup request over one page of the session index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CleanRequest {
    /// Which sessions may be considered.
    pub scope: CleanScope,
    /// Whether eligible sessions are removed.
    pub mode: CleanMode,
    /// Index bucket to continue from; `None` starts at the beginning.
    pub cursor: Option<u16>,
}

/// What cleanup decided for one examined session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanDecision {
    /// Still open and within its expiry; left alone.
    Ineligible,
    /// A dry run found it closed, expired or abandoned.
    Eligible,
    /// It was removed.
    Removed,
}

/// One entry of a cleanup page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CleanEntry {
    /// A session cleanup examined and decided on.
    Examined {
        /// Session identity.
        session_id: SessionId,
        /// The decision.
        decision: CleanDecision,
    },
    /// A session cleanup could not examine.
    Skipped {
        /// Session identity.
        session_id: SessionId,
        /// Why it was skipped.
        error: EngineError,
    },
}

/// One bounded page of cleanup results.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanPage {
    entries: Vec<CleanEntry>,
    next_cursor: Option<u16>,
    mode: CleanMode,
}

impl CleanPage {
    /// Entries on this page, in index order.
    #[must_use]
    pub fn entries(&self) -> &[CleanEntry] {
        &self.entries
    }

    /// Consumes the page, returning its entries.
    #[must_use]
    pub fn into_entries(self) -> Vec<CleanEntry> {
        self.entries
    }

    /// Cursor for the next page, absent on the last page.
    #[must_use]
    pub const fn next_cursor(&self) -> Option<u16> {
        self.next_cursor
    }

    /// Whether this page removed anything or only reported.
    #[must_use]
    pub const fn mode(&self) -> CleanMode {
        self.mode
    }

    /// Whether any entry on the page was skipped.
    #[must_use]
    pub fn is_partial(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| matches!(entry, CleanEntry::Skipped { .. }))
    }
}

/// Whether a retained bundle carries its own copy of the source media.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceRetention {
    /// Evidence metadata only; re-extraction needs the matching original.
    EvidenceOnly,
    /// A verified copy of the source goes into the bundle.
    IncludeSource,
}

impl SourceRetention {
    const fn policy(self) -> BundleSourcePolicy {
        match self {
            Self::EvidenceOnly => BundleSourcePolicy::EvidenceOnly,
            Self::IncludeSource => BundleSourcePolicy::IncludeSource,
        }
    }
}

/// A validated retained bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleSummary {
    session_id: SessionId,
    source_id: SourceId,
    source_bytes: u64,
    source_retention: SourceRetention,
    artifact_count: usize,
    artifact_bytes: u64,
}

impl BundleSummary {
    fn from_status(bundle: &BundleStatus) -> Self {
        Self {
            session_id: bundle.session_id().clone(),
            source_id: bundle.source_id().clone(),
            source_bytes: bundle.source_bytes(),
            source_retention: match bundle.source_policy() {
                BundleSourcePolicy::EvidenceOnly => SourceRetention::EvidenceOnly,
                BundleSourcePolicy::IncludeSource => SourceRetention::IncludeSource,
            },
            artifact_count: bundle.artifact_count(),
            artifact_bytes: bundle.artifact_bytes(),
        }
    }

    /// Identity of the retained session.
    #[must_use]
    pub const fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Hash identity of the source the evidence was taken from.
    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    /// Size of that source.
    #[must_use]
    pub const fn source_bytes(&self) -> u64 {
        self.source_bytes
    }

    /// Whether the bundle carries a copy of the source.
    #[must_use]
    pub const fn source_retention(&self) -> SourceRetention {
        self.source_retention
    }

    /// Number of evidence artifacts in the bundle.
    #[must_use]
    pub const fn artifact_count(&self) -> usize {
        self.artifact_count
    }

    /// Total bytes of evidence artifacts in the bundle.
    #[must_use]
    pub const fn artifact_bytes(&self) -> u64 {
        self.artifact_bytes
    }
}

impl Engine {
    /// Copies and hashes one local source into a new disposable session,
    /// optionally importing a supplied transcript with it.
    ///
    /// The session root is provisioned on first use. Without a transcript no
    /// provider runs. With one, everything that can fail without touching the
    /// session root runs first (offset bounds, reading and parsing the sidecar,
    /// locating `FFmpeg` and `FFprobe`); then the source is staged, its duration
    /// probed, the transcript aligned, and source and transcript are committed
    /// in one generation. Whisper and model weights are never needed for this.
    ///
    /// # Errors
    ///
    /// Fails with the transcript, tool, root, clock, identifier, source, probe
    /// or storage failure that stopped the open. A rejected transcript never
    /// leaves an open session.
    pub async fn ingest(&self, request: IngestRequest) -> Result<IngestOutcome, EngineError> {
        let source = absolute_selection(&request.source)?;
        let import = match &request.transcript {
            Some(transcript) => Some(self.prepare_transcript_import(transcript)?),
            None => None,
        };
        let root = self.session_root_path()?;
        let store = Self::open_session_store(&root, SessionRootProvisioning::CreateIfMissing)?
            .ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        let now = self.now_unix_seconds()?;
        let open = OpenSessionRequest {
            source,
            session_id: self.new_session_id()?,
            initialize_operation_id: self.new_operation_id()?,
            stage_operation_id: self.new_operation_id()?,
            activate_operation_id: self.new_operation_id()?,
            durability: DurabilityRequirement::Ephemeral,
            now_unix_seconds: now,
        };
        let Some((import, tools)) = import else {
            let session = OpenSession::new(store)
                .execute(open)
                .await
                .map_err(EngineError::OpenSession)?;
            return Ok(IngestOutcome {
                session,
                transcript: None,
            });
        };
        let probe_store = Self::open_session_store(&root, SessionRootProvisioning::ExistingOnly)?
            .ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        let probe = FfprobeSourceDuration::new(
            tools,
            self.config().host_isolation.into_infrastructure(),
            probe_store,
            ProcessCancellation::new(),
        );
        let (session, revision) = OpenSession::new(store)
            .execute_with_transcript(open, &import, &probe)
            .await
            .map_err(EngineError::OpenSession)?;
        Ok(IngestOutcome {
            session,
            transcript: Some(revision),
        })
    }

    /// Lists one bounded page of the session index.
    ///
    /// Empty buckets are skipped, so a page is empty only at the end of the
    /// index. A missing root lists as one empty final page and is not created.
    ///
    /// # Errors
    ///
    /// Fails when the root, clock or index cannot be read. A single unreadable
    /// session does not fail the page; it is listed as unavailable.
    pub fn list_sessions(&self, cursor: Option<u16>) -> Result<SessionPage, EngineError> {
        let (store, now) = self.existing_store()?;
        let Some(store) = store else {
            return Ok(SessionPage {
                entries: Vec::new(),
                next_cursor: None,
            });
        };
        let page = first_occupied_page(&store, cursor)?;
        let entries = page
            .session_ids()
            .iter()
            .map(
                |session_id| match store.indexed_session_status(session_id) {
                    Ok(Some(status)) => {
                        SessionListEntry::Indexed(SessionSnapshot::observe(&status, now))
                    }
                    Ok(None) => SessionListEntry::Initializing(session_id.clone()),
                    Err(error) => SessionListEntry::Unavailable {
                        session_id: session_id.clone(),
                        error: EngineError::Storage(error),
                    },
                },
            )
            .collect();
        Ok(SessionPage {
            entries,
            next_cursor: page.next_bucket(),
        })
    }

    /// Reads one session's committed status.
    ///
    /// # Errors
    ///
    /// Fails when the root is missing or unreadable, or the session cannot be read.
    pub fn session_status(&self, session_id: &SessionId) -> Result<SessionSnapshot, EngineError> {
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        let status = store.session_status(session_id)?;
        Ok(SessionSnapshot::observe(&status, now))
    }

    /// Extends one open session's idle expiry, within its maximum age.
    ///
    /// # Errors
    ///
    /// Fails when the session cannot be read or is not eligible for renewal.
    pub fn renew_session(&self, session_id: &SessionId) -> Result<SessionSnapshot, EngineError> {
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        let current = store.session_status(session_id)?;
        store.renew_session(
            session_id,
            &self.new_operation_id()?,
            current.generation(),
            now,
        )?;
        let status = store.session_status(session_id)?;
        Ok(SessionSnapshot::observe(&status, now))
    }

    /// Closes one session; it becomes eligible for cleanup.
    ///
    /// # Errors
    ///
    /// Fails when the session cannot be read or closed.
    pub fn close_session(&self, session_id: &SessionId) -> Result<SessionSnapshot, EngineError> {
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        let current = store.session_status(session_id)?;
        store.close_session(session_id, &self.new_operation_id()?, current.generation())?;
        let status = store.session_status(session_id)?;
        Ok(SessionSnapshot::observe(&status, now))
    }

    /// Exports one session to a new directory as a validated portable bundle.
    ///
    /// `output` must not exist; a relative path is resolved against the process
    /// working directory. The original source is never modified.
    ///
    /// # Errors
    ///
    /// Fails when the session cannot be read or the bundle cannot be published.
    pub fn retain_session(
        &self,
        session_id: &SessionId,
        output: &Path,
        retention: SourceRetention,
    ) -> Result<BundleSummary, EngineError> {
        let (store, _now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        let output = absolute_selection(output)?;
        let bundle = store.retain_bundle(session_id, &output, retention.policy())?;
        Ok(BundleSummary::from_status(&bundle))
    }

    /// Examines, and unless dry-running removes, sessions on one index page.
    ///
    /// Only positively identified `VSift` sessions proven closed, expired or
    /// abandoned are removed. A missing root cleans as one empty final page.
    ///
    /// # Errors
    ///
    /// Fails with [`EngineError::UnrestrictedCleanRejected`] for an unrestricted
    /// scope, after the root has been checked, and when the root, clock or index
    /// cannot be read. A single session that cannot be examined is skipped.
    pub fn clean_sessions(&self, request: CleanRequest) -> Result<CleanPage, EngineError> {
        let (store, now) = self.existing_store()?;
        if request.scope == CleanScope::Unrestricted {
            return Err(EngineError::UnrestrictedCleanRejected);
        }
        let Some(store) = store else {
            return Ok(CleanPage {
                entries: Vec::new(),
                next_cursor: None,
                mode: request.mode,
            });
        };
        let page = first_occupied_page(&store, request.cursor)?;
        let dry_run = request.mode == CleanMode::DryRun;
        let entries = page
            .session_ids()
            .iter()
            .map(
                |session_id| match store.clean_session(session_id, now, dry_run) {
                    Ok(outcome) => CleanEntry::Examined {
                        session_id: session_id.clone(),
                        decision: match outcome {
                            CleanOutcome::Ineligible => CleanDecision::Ineligible,
                            CleanOutcome::Eligible => CleanDecision::Eligible,
                            CleanOutcome::Removed => CleanDecision::Removed,
                        },
                    },
                    Err(error) => CleanEntry::Skipped {
                        session_id: session_id.clone(),
                        error: EngineError::Storage(error),
                    },
                },
            )
            .collect();
        Ok(CleanPage {
            entries,
            next_cursor: page.next_bucket(),
            mode: request.mode,
        })
    }

    /// Validates a retained bundle as bounded data, independent of any session root.
    ///
    /// A relative path is resolved against the process working directory.
    ///
    /// # Errors
    ///
    /// Fails when the bundle is unreadable, malformed or fails integrity checks.
    pub fn validate_bundle(&self, directory: &Path) -> Result<BundleSummary, EngineError> {
        let selected = absolute_selection(directory)?;
        let bundle = FilesystemSessionStore::validate_bundle(&selected)?;
        Ok(BundleSummary::from_status(&bundle))
    }

    /// Resolves the root, reads the clock and opens an existing store.
    ///
    /// The order is part of the public contract: a root failure is reported
    /// before a clock failure, which is reported before a store failure.
    pub(crate) fn existing_store(
        &self,
    ) -> Result<(Option<FilesystemSessionStore>, u64), EngineError> {
        let root = self.session_root_path()?;
        let now = self.now_unix_seconds()?;
        let store = Self::open_session_store(&root, SessionRootProvisioning::ExistingOnly)?;
        Ok((store, now))
    }
}

/// Returns the first non-empty index bucket at or after `cursor`, or the last
/// bucket when every remaining bucket is empty.
fn first_occupied_page(
    store: &FilesystemSessionStore,
    cursor: Option<u16>,
) -> Result<SessionIndexPage, EngineError> {
    let mut bucket = cursor.unwrap_or(0);
    loop {
        let page = store.scan_session_bucket(bucket)?;
        if !page.session_ids().is_empty() || page.next_bucket().is_none() {
            return Ok(page);
        }
        bucket = page
            .next_bucket()
            .ok_or(EngineError::SessionIndexInconsistent)?;
    }
}
