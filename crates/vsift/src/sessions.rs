//! Disposable-session operations: open, inspect, renew, close, retain, clean,
//! and validate retained bundles.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use vsift_application::{
    ForegroundSessionPort, OpenSession, OpenSessionError, OpenSessionOutcome, OpenSessionRequest,
    SessionStorageError, TranscriptImportRequest,
};
use vsift_domain::{
    DurabilityRequirement, OperationId, SessionId, SessionLifetime, SessionPhase, SourceId,
    StorageGeneration, TranscriptRevision, WorkspacePolicy,
};
use vsift_infrastructure::{
    BundleSourcePolicy, BundleStatus, CleanOutcome, ContainedFile, ContainedSourceStore,
    FfprobeSourceDuration, FilesystemAdmissionPermit, FilesystemSessionStore, FreeSpaceCheck,
    MAX_SOURCE_BYTES, SessionIndexPage, SessionRootProvisioning, SessionStatus, SourceSnapshot,
};

use crate::{
    engine::{Engine, absolute_selection},
    error::{EngineError, SessionRootError},
    verification::Cancellation,
};

/// A request to open a local video as a new disposable session.
#[derive(Clone, Debug)]
pub struct IngestRequest {
    /// Local source media; a relative path is resolved against the process
    /// working directory.
    pub source: PathBuf,
    /// Optional supplied `SubRip` or `WebVTT` transcript to import with it.
    pub transcript: Option<SuppliedTranscriptRequest>,
    /// Signal that stops the copy of the source between 64 KiB blocks and
    /// the transcript's duration probe; a cancelled ingest opens no session.
    pub cancellation: Cancellation,
    /// How the session publishes, fixed for its whole life (ADR 0020).
    ///
    /// [`DurabilityRequirement::Ephemeral`] is the desktop default:
    /// consistent across a process crash. [`DurabilityRequirement::Durable`]
    /// asks that every acknowledged generation survive an OS crash or power
    /// loss; only a root on the qualified profile (Ubuntu 24.04, local ext4
    /// with write barriers, ADR 0010) can honour it, and anywhere else the
    /// ingest fails with `MISSING_CAPABILITY` before any session is
    /// registered, never downgrading the request.
    ///
    /// It is the least the caller requires. In a worker workspace the
    /// workspace's policy decides (ADR 0021 section 3): a durable workspace
    /// makes the session durable whatever was asked, which is how the
    /// command line's plain `ingest --session-root <workspace>` becomes its
    /// durable mode (ADR 0020 D-3), and a durable requirement in an
    /// ephemeral workspace fails with [`EngineError::WorkspaceNotDurable`].
    pub durability: DurabilityRequirement,
}

/// The source an ingest copies: a path the caller selected, or a file a
/// worker request named inside the operator's input root, already opened
/// there following no link (P11, ADR 0021 section 9).
pub(crate) enum IngestSource {
    /// An absolute path, opened by the staging itself.
    Path(PathBuf),
    /// A file opened inside the input root, and the copy's admission when
    /// the caller already holds it.
    Contained {
        /// The opened source.
        file: ContainedFile,
        /// The copy's weight-1 admission, taken before the session is
        /// registered.
        admission: Option<FilesystemAdmissionPermit>,
    },
}

/// The supplied transcript an ingest imports, as for [`IngestSource`].
pub(crate) enum IngestTranscript {
    /// A caller-selected sidecar.
    Path(SuppliedTranscriptRequest),
    /// A sidecar opened inside the input root, and its offset.
    Contained {
        /// The opened sidecar.
        file: ContainedFile,
        /// Signed microseconds from sidecar time to source time.
        offset_micros: i64,
    },
}

/// Everything one ingest needs, resolved: the public [`IngestRequest`], or
/// a worker request's step, which also fixes the session id so the id is
/// recorded before the copy starts (ADR 0021 section 2).
pub(crate) struct PreparedIngest {
    pub(crate) source: IngestSource,
    pub(crate) transcript: Option<IngestTranscript>,
    pub(crate) cancellation: Cancellation,
    pub(crate) durability: DurabilityRequirement,
    pub(crate) session_id: Option<SessionId>,
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
    /// Whether the workspace's free-space reserve was checked before the
    /// copy: only a worker workspace on Unix checks it (P11 PR 2).
    pub free_space: FreeSpaceReserveCheck,
}

/// Whether an ingest checked the free-space reserve before its copy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FreeSpaceReserveCheck {
    /// A workspace on Unix: the filesystem had the source's size and the
    /// 1 GiB reserve free before the copy.
    Enforced,
    /// A desktop root, or a workspace on Windows: nothing was checked.
    NotEnforced,
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
    manifest_sha256: String,
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
            manifest_sha256: bundle.manifest_sha256().to_owned(),
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

    /// SHA-256 of the bundle's validated manifest (`bundle.json`), 64
    /// lowercase hexadecimal digits. The manifest lists every file with its
    /// digest, so this identifies the whole bundle; a worker request records
    /// it and checks it again when it meets the bundle later (P11).
    #[must_use]
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }
}

impl Engine {
    /// Copies and hashes one local source into a new disposable session,
    /// optionally importing a supplied transcript with it.
    ///
    /// The session root is provisioned on first use. Without a transcript no
    /// provider runs. With one, everything that can fail without touching the
    /// session root runs first (offset bounds, reading and parsing the sidecar,
    /// locating `FFmpeg` and `FFprobe`, and the automatic media-tool preflight,
    /// which verifies them once per tool identity); then the source is staged,
    /// its duration probed, the transcript aligned, and source and transcript
    /// are committed in one generation. Whisper and model weights are never
    /// needed for this.
    ///
    /// # Errors
    ///
    /// Fails with the transcript, tool, verification, root, clock, identifier,
    /// source, probe or storage failure that stopped the open. A rejected
    /// transcript or a failed verification never leaves an open session, and a
    /// failed verification happens before the session root is touched.
    pub async fn ingest(&self, request: IngestRequest) -> Result<IngestOutcome, EngineError> {
        let source = absolute_selection(&request.source)?;
        self.ingest_prepared(PreparedIngest {
            source: IngestSource::Path(source),
            transcript: request.transcript.map(IngestTranscript::Path),
            cancellation: request.cancellation,
            durability: request.durability,
            session_id: None,
        })
        .await
    }

    /// [`Self::ingest`] of a resolved request: a worker request's step
    /// passes its contained source and its recorded session id here.
    pub(crate) async fn ingest_prepared(
        &self,
        request: PreparedIngest,
    ) -> Result<IngestOutcome, EngineError> {
        let import = match request.transcript {
            Some(IngestTranscript::Path(transcript)) => {
                Some(self.prepare_transcript_import(&transcript)?)
            }
            Some(IngestTranscript::Contained {
                file,
                offset_micros,
            }) => Some(self.prepare_contained_transcript_import(file, offset_micros)?),
            None => None,
        };
        // Only the transcript path runs FFprobe on the source, so only it
        // needs verified tools; a plain ingest runs no provider.
        if let Some((_, tools)) = &import {
            self.ensure_media_tools_verified(tools).await?;
        }
        let root = self.session_root_path()?;
        let store = self
            .open_session_store(&root, SessionRootProvisioning::CreateIfMissing)?
            .ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        let durability = session_durability(request.durability, store.workspace_policy())?;
        let incoming = match &request.source {
            IngestSource::Path(source) => {
                std::fs::metadata(source).map_or(0, |metadata| metadata.len())
            }
            IngestSource::Contained { file, .. } => file.len(),
        };
        let free_space = free_space_reserve(&store, incoming)?;
        let now = self.now_unix_seconds()?;
        let session_id = match request.session_id {
            Some(session_id) => session_id,
            None => self.new_session_id()?,
        };
        let (source_path, contained) = match request.source {
            IngestSource::Path(path) => (path, None),
            // The contained adapter stages the opened file; the use case's
            // path is not read.
            IngestSource::Contained { file, admission } => {
                (PathBuf::new(), Some((file, admission)))
            }
        };
        let open = OpenSessionRequest {
            source: source_path,
            session_id,
            initialize_operation_id: self.new_operation_id()?,
            stage_operation_id: self.new_operation_id()?,
            activate_operation_id: self.new_operation_id()?,
            durability,
            now_unix_seconds: now,
        };
        let import = match import {
            Some((import, tools)) => {
                let probe_store = self
                    .open_session_store(&root, SessionRootProvisioning::ExistingOnly)?
                    .ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
                let probe = FfprobeSourceDuration::new(
                    tools,
                    self.config().host_isolation.into_infrastructure(),
                    probe_store,
                    request.cancellation.0.clone(),
                );
                Some((import, probe))
            }
            None => None,
        };
        let registered = (
            open.session_id.clone(),
            open.initialize_operation_id.clone(),
        );
        let opened = match contained {
            None => open_session(store, open, import.as_ref(), &request.cancellation).await,
            Some((file, admission)) => {
                open_session(
                    ContainedSourceStore::new(store, file, admission),
                    open,
                    import.as_ref(),
                    &request.cancellation,
                )
                .await
            }
        };
        let (session, transcript) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                // The failed open's registration, and the session folder it
                // began, are removed now rather than left for a day (#277).
                self.abandon_failed_open(&root, &registered.0, &registered.1)
                    .await;
                return Err(error);
            }
        };
        Ok(IngestOutcome {
            session,
            transcript,
            free_space,
        })
    }

    /// Removes the registration, and the session folder it began, that one open
    /// that failed before publishing its first generation created (#277).
    ///
    /// A failed open used to leave both for `session clean` to collect a day
    /// later. Under load that is routine: a worker request that finds the
    /// root's initialization lock busy after it registered opens a new session
    /// for its retry and left the failed attempt's registration behind (9 of 40
    /// rounds of twenty requests at concurrency four on a hosted runner listed
    /// one extra `initializing` session). The store removes only a session that
    /// was never published and whose registration names **this** operation
    /// ([`FilesystemSessionStore::abandon_unpublished_open`]), through the
    /// routine `session clean` uses.
    ///
    /// **Nothing is tried when no registration exists**: a failure that came
    /// before it was made (the root's initialization lock was busy when the
    /// registration asked for it, a guarantee the root cannot give, a clock) has
    /// nothing to remove, and asking a busy root again would only wait.
    ///
    /// **A busy root is waited for, for a bounded time**, not given up on after a
    /// few tries: the root's lock is held for tens of milliseconds by every
    /// other opener (measured: five consecutive refusals were common in a batch
    /// of two requests at a time on Windows, and a fixed eight tries then gave up
    /// on a slow runner). The wait is five seconds
    /// ([`crate::EnginePorts::with_failed_open_removal_wait`]) and applies only to this
    /// failure path, so a request that did not fail never waits for it, and the
    /// engine is asynchronous, so no thread is blocked. If the removal still
    /// fails, or the root cannot be reopened, the registration is left for
    /// `session clean` exactly as before, and the caller keeps its original
    /// failure: this never reports and never masks.
    async fn abandon_failed_open(
        &self,
        root: &Path,
        session_id: &SessionId,
        operation_id: &OperationId,
    ) {
        let Ok(Some(store)) = self.open_session_store(root, SessionRootProvisioning::ExistingOnly)
        else {
            return;
        };
        if !matches!(store.is_registered(session_id), Ok(true)) {
            return;
        }
        let Ok(now) = self.now_unix_seconds() else {
            return;
        };
        let _outcome = retry_while_transient(self.failed_open_removal_wait(), || {
            store.abandon_unpublished_open(session_id, operation_id, now)
        })
        .await;
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
        let status = published_status(&store, session_id)?;
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
        let current = published_status(&store, session_id)?;
        store.renew_session(
            session_id,
            &self.new_operation_id()?,
            current.generation(),
            now,
        )?;
        let status = published_status(&store, session_id)?;
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
        let current = published_status(&store, session_id)?;
        store.close_session(session_id, &self.new_operation_id()?, current.generation())?;
        let status = published_status(&store, session_id)?;
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
        // A registration that was removed after the page listed it (another
        // cleanup, or a failed open removing its own) is not an entry: there is
        // nothing left to report and nothing went wrong.
        let entries = page
            .session_ids()
            .iter()
            .filter_map(
                |session_id| match store.clean_session(session_id, now, dry_run) {
                    Ok(CleanOutcome::Gone) => None,
                    Ok(outcome) => Some(CleanEntry::Examined {
                        session_id: session_id.clone(),
                        decision: match outcome {
                            CleanOutcome::Eligible => CleanDecision::Eligible,
                            CleanOutcome::Removed => CleanDecision::Removed,
                            CleanOutcome::Ineligible | CleanOutcome::Gone => {
                                CleanDecision::Ineligible
                            }
                        },
                    }),
                    Err(error) => Some(CleanEntry::Skipped {
                        session_id: session_id.clone(),
                        error: EngineError::Storage(error),
                    }),
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
        let store = self.open_session_store(&root, SessionRootProvisioning::ExistingOnly)?;
        Ok((store, now))
    }
}

/// Reads a session's committed status, naming a session that is not
/// published as that and not as a failing disk (#277).
///
/// A session id with no published session (never opened, still opening, its
/// opening was interrupted and left only its registration, or cleaned
/// already) answered `STORAGE_IO`, `INVALID_ARGUMENT` or `INTEGRITY_FAILURE`
/// with no remediation, depending on what the lookup met first: the first
/// sends an agent to retry or to look at its storage, the last tells it stored
/// data is damaged. It is now [`EngineError::SessionNotPublished`], which
/// **keeps the code the lookup gave** (changing a published failure code is not
/// additive within v1, known limit L-127) and carries a remediation that says
/// it is a missing session and not a diagnosis of the storage. A failure that
/// is not that, including a real I/O error or damage to a session folder that
/// exists, is unchanged.
pub(crate) fn published_status(
    store: &FilesystemSessionStore,
    session_id: &SessionId,
) -> Result<SessionStatus, EngineError> {
    match store.session_status(session_id) {
        Ok(status) => Ok(status),
        // A cleaned session leaves its lock files, so the read finds the
        // folder missing as damage, not as absence.
        Err(
            error @ (SessionStorageError::Io
            | SessionStorageError::StateConflict
            | SessionStorageError::IntegrityFailure),
        ) => match store.indexed_session_status(session_id) {
            Ok(None) => Err(EngineError::SessionNotPublished(error)),
            Ok(Some(status)) => Ok(status),
            Err(_) => Err(error.into()),
        },
        Err(other) => Err(other.into()),
    }
}

/// Runs `attempt` again while it reports a refusal that goes away by itself,
/// for at most `limit` (the first attempt always runs), pausing 5 ms and then
/// twice as long after each, up to 100 ms; any other outcome, success included,
/// ends it at once.
///
/// `Busy` is such a refusal everywhere. On Windows so is `AccessDenied`: a file
/// that another handle (a scanner, a child process, a remover) still holds open
/// refuses to be deleted until it lets go.
async fn retry_while_transient<T, F>(
    limit: Duration,
    mut attempt: F,
) -> Result<T, SessionStorageError>
where
    F: FnMut() -> Result<T, SessionStorageError>,
{
    let started = Instant::now();
    let mut pause = Duration::from_millis(5);
    loop {
        match attempt() {
            Err(error) if is_transient(error) && started.elapsed() < limit => {
                tokio::time::sleep(pause).await;
                pause = (pause * 2).min(Duration::from_millis(100));
            }
            outcome => return outcome,
        }
    }
}

/// Whether a refusal goes away by itself (see [`retry_while_transient`]).
const fn is_transient(error: SessionStorageError) -> bool {
    matches!(error, SessionStorageError::Busy)
        || (cfg!(windows) && matches!(error, SessionStorageError::AccessDenied))
}

/// Registers, stages and activates one session through `port`, importing
/// the supplied transcript in the same generation when there is one.
async fn open_session<Port>(
    port: Port,
    open: OpenSessionRequest,
    import: Option<&(TranscriptImportRequest, FfprobeSourceDuration)>,
    cancellation: &Cancellation,
) -> Result<(OpenSessionOutcome, Option<TranscriptRevision>), EngineError>
where
    Port: ForegroundSessionPort<Snapshot = SourceSnapshot>,
{
    let use_case = OpenSession::new(port);
    match import {
        None => use_case
            .execute(open, &cancellation.0)
            .await
            .map(|session| (session, None))
            .map_err(EngineError::OpenSession),
        Some((import, probe)) => use_case
            .execute_with_transcript(open, import, probe, &cancellation.0)
            .await
            .map(|(session, revision)| (session, Some(revision)))
            .map_err(EngineError::OpenSession),
    }
}

/// Checks the room for the source's copy before it starts, so a source that
/// cannot fit is refused at once and not after it has filled the disk
/// (#266): the source's size (`incoming`, read from its metadata; the copy
/// itself still refuses a file that grows) and, in a worker workspace, the
/// 1 GiB reserve must be available.
///
/// A worker workspace keeps its answer exactly as before (`RESOURCE_LIMIT`).
/// A desktop root, the CLI path, keeps only a small margin for the session's
/// own records and refuses with [`OpenSessionError::SourceNoRoom`], which has
/// its own remediation and the CLI path's published code, `STORAGE_IO` (known
/// limit L-127). The desktop check is best effort (known limit L-061), and it
/// comes after the source-size limit ([`ensure_room_for_desktop_copy`]).
fn free_space_reserve(
    store: &FilesystemSessionStore,
    incoming: u64,
) -> Result<FreeSpaceReserveCheck, EngineError> {
    if store.workspace_policy().is_none() {
        ensure_room_for_desktop_copy(incoming, |bytes| store.ensure_room_for_copy(bytes))?;
        return Ok(FreeSpaceReserveCheck::NotEnforced);
    }
    Ok(match store.ensure_free_space(incoming)? {
        FreeSpaceCheck::Enforced => FreeSpaceReserveCheck::Enforced,
        FreeSpaceCheck::NotEnforced => FreeSpaceReserveCheck::NotEnforced,
    })
}

/// A desktop root's room check for a source of `incoming` bytes, **after** the
/// source-size limit: a source over [`MAX_SOURCE_BYTES`] is never copied (staging
/// refuses it before it writes anything, as `INVALID_SOURCE`), so it needs no
/// room and is not asked for any.
///
/// Why this order matters (#310): `INVALID_SOURCE` is what 0.1.0 answered for
/// such a source, whatever the disk. The room check added for #266 ran first
/// and answered `STORAGE_IO` when the source was also larger than the free
/// space, which changed a published failure code (not additive within v1, known
/// limits L-126 and L-127). Stepping aside here, rather than checking the limit
/// a second time, keeps the limit in one place (staging) and its answer exactly
/// what it was. A source within the limit that does not fit keeps the #266
/// answer. A worker workspace is not affected: its reserve was checked first
/// in 0.1.0 and its answer is unchanged ([`free_space_reserve`]).
///
/// `ensure_room` is the store's check, passed in so that the order is tested
/// with any amount of free space on every platform.
fn ensure_room_for_desktop_copy(
    incoming: u64,
    ensure_room: impl FnOnce(u64) -> Result<FreeSpaceCheck, SessionStorageError>,
) -> Result<(), EngineError> {
    if incoming > MAX_SOURCE_BYTES {
        return Ok(());
    }
    ensure_room(incoming)
        .map(|_checked| ())
        .map_err(|error| match error {
            SessionStorageError::CapacityExhausted => {
                EngineError::OpenSession(OpenSessionError::SourceNoRoom)
            }
            other => EngineError::Storage(other),
        })
}

/// The durability a new session publishes with: at least what the caller
/// requires, and in a worker workspace exactly the workspace's policy, which
/// the caller cannot lower (ADR 0020 D-3, ADR 0021 section 3).
///
/// A durable workspace makes every session durable, so the command line's
/// plain `ingest --session-root <workspace>` is its durable mode. A durable
/// requirement in an ephemeral workspace is refused rather than silently
/// weakened.
fn session_durability(
    required: DurabilityRequirement,
    workspace: Option<WorkspacePolicy>,
) -> Result<DurabilityRequirement, EngineError> {
    match (workspace.map(WorkspacePolicy::durability), required) {
        (None, required) => Ok(required),
        (Some(DurabilityRequirement::Durable), _) => Ok(DurabilityRequirement::Durable),
        (Some(DurabilityRequirement::Ephemeral), DurabilityRequirement::Ephemeral) => {
            Ok(DurabilityRequirement::Ephemeral)
        }
        (Some(DurabilityRequirement::Ephemeral), DurabilityRequirement::Durable) => {
            Err(EngineError::WorkspaceNotDurable)
        }
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

#[cfg(test)]
mod tests {
    use std::{cell::Cell, time::Duration};

    use vsift_application::{OpenSessionError, SessionStorageError};
    use vsift_infrastructure::{FreeSpaceCheck, MAX_SOURCE_BYTES};

    use super::{ensure_room_for_desktop_copy, retry_while_transient};
    use crate::error::EngineError;

    const LONG: Duration = Duration::from_secs(30);
    const TIB: u64 = 1024 * 1024 * 1024 * 1024;

    /// #310: a source over the size limit is never copied, so it is not asked
    /// for room, however little there is: staging then refuses it with the
    /// limit's answer, `INVALID_SOURCE`, which is what 0.1.0 gave whatever the
    /// disk. Checking the room first answered `STORAGE_IO` whenever the source
    /// was also larger than the free space. The check is a closure that always
    /// says "no room", so the result does not depend on this machine's disk.
    #[test]
    fn a_source_over_the_size_limit_is_not_asked_for_room() {
        for incoming in [MAX_SOURCE_BYTES + 1, 4 * TIB, u64::MAX] {
            let asked = Cell::new(0_u32);

            let outcome = ensure_room_for_desktop_copy(incoming, |_| {
                asked.set(asked.get() + 1);
                Err(SessionStorageError::CapacityExhausted)
            });

            assert_eq!(outcome, Ok(()), "{incoming} bytes");
            assert_eq!(asked.get(), 0, "{incoming} bytes: the room was asked for");
        }
    }

    /// #266, unchanged: a source the limit accepts, up to and including the
    /// limit itself, that does not fit the root is refused before the copy as
    /// "no room" (`STORAGE_IO` with its own remediation).
    #[test]
    fn a_source_within_the_size_limit_that_does_not_fit_is_refused_as_no_room() {
        for incoming in [1, MAX_SOURCE_BYTES / 2, MAX_SOURCE_BYTES] {
            let asked = Cell::new(None);

            let outcome = ensure_room_for_desktop_copy(incoming, |bytes| {
                asked.set(Some(bytes));
                Err(SessionStorageError::CapacityExhausted)
            });

            assert_eq!(
                outcome,
                Err(EngineError::OpenSession(OpenSessionError::SourceNoRoom)),
                "{incoming} bytes"
            );
            assert_eq!(asked.get(), Some(incoming), "{incoming} bytes");
        }
    }

    /// A source that fits is let through, and so is one whose room could not be
    /// measured (the check is best effort, known limit L-061): the copy's own
    /// write failure stays the backstop.
    #[test]
    fn a_source_that_fits_or_cannot_be_measured_is_let_through() {
        for checked in [FreeSpaceCheck::Enforced, FreeSpaceCheck::NotEnforced] {
            assert_eq!(
                ensure_room_for_desktop_copy(MAX_SOURCE_BYTES, |_| Ok(checked)),
                Ok(()),
                "{checked:?}"
            );
        }
    }

    /// Only "no room" is a source that does not fit; any other failure of the
    /// check is the storage failure it was.
    #[test]
    fn a_failed_check_other_than_no_room_stays_a_storage_failure() {
        for failure in [
            SessionStorageError::Io,
            SessionStorageError::IntegrityFailure,
            SessionStorageError::Busy,
        ] {
            assert_eq!(
                ensure_room_for_desktop_copy(1, |_| Err(failure)),
                Err(EngineError::Storage(failure)),
                "{failure:?}"
            );
        }
    }

    /// The busy path: a removal that meets a busy root is tried again and
    /// succeeds when the root frees up.
    #[tokio::test]
    async fn a_busy_attempt_is_retried_until_it_succeeds() {
        let calls = Cell::new(0_u32);

        let outcome = retry_while_transient(LONG, || {
            calls.set(calls.get() + 1);
            if calls.get() < 4 {
                Err(SessionStorageError::Busy)
            } else {
                Ok("removed")
            }
        })
        .await;

        assert_eq!(outcome, Ok("removed"));
        assert_eq!(calls.get(), 4);
    }

    /// The bound is a time: a root that stays busy is asked until the limit and
    /// then the last answer is returned for the caller to ignore. The first
    /// attempt always runs, even with no time at all.
    #[tokio::test]
    async fn a_root_that_stays_busy_is_asked_until_the_limit_and_no_longer() {
        let calls = Cell::new(0_u32);
        let started = std::time::Instant::now();

        let outcome: Result<(), SessionStorageError> =
            retry_while_transient(Duration::from_millis(60), || {
                calls.set(calls.get() + 1);
                Err(SessionStorageError::Busy)
            })
            .await;

        assert_eq!(outcome, Err(SessionStorageError::Busy));
        assert!(calls.get() >= 2, "{} attempts", calls.get());
        assert!(started.elapsed() >= Duration::from_millis(60));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );

        let once: Cell<u32> = Cell::new(0);
        let none: Result<(), SessionStorageError> = retry_while_transient(Duration::ZERO, || {
            once.set(once.get() + 1);
            Err(SessionStorageError::Busy)
        })
        .await;
        assert_eq!(none, Err(SessionStorageError::Busy));
        assert_eq!(once.get(), 1);
    }

    /// Any other failure, like success, is final at once: no retry of an
    /// integrity failure or an I/O error.
    #[tokio::test]
    async fn any_other_outcome_is_final_at_once() {
        for failure in [
            SessionStorageError::Io,
            SessionStorageError::IntegrityFailure,
            SessionStorageError::StateConflict,
        ] {
            let calls = Cell::new(0_u32);

            let outcome: Result<(), SessionStorageError> = retry_while_transient(LONG, || {
                calls.set(calls.get() + 1);
                Err(failure)
            })
            .await;

            assert_eq!(outcome, Err(failure));
            assert_eq!(calls.get(), 1, "{failure:?}");
        }
    }

    /// A file another handle still holds open refuses to be deleted on Windows
    /// until it lets go, so `AccessDenied` is tried again there and only there.
    #[tokio::test]
    async fn access_denied_is_transient_on_windows_only() {
        let calls = Cell::new(0_u32);

        let outcome: Result<(), SessionStorageError> =
            retry_while_transient(Duration::from_millis(40), || {
                calls.set(calls.get() + 1);
                Err(SessionStorageError::AccessDenied)
            })
            .await;

        assert_eq!(outcome, Err(SessionStorageError::AccessDenied));
        assert_eq!(calls.get() > 1, cfg!(windows), "{} attempts", calls.get());
    }
}
