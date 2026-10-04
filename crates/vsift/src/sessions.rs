//! Disposable-session operations: open, inspect, renew, close, retain, clean,
//! and validate retained bundles.

use std::path::{Path, PathBuf};

use vsift_application::{
    ForegroundSessionPort, OpenSession, OpenSessionOutcome, OpenSessionRequest,
    SessionStorageError, TranscriptImportRequest,
};
use vsift_domain::{
    DurabilityRequirement, OperationId, SessionId, SessionLifetime, SessionPhase, SourceId,
    StorageGeneration, TranscriptRevision, WorkspacePolicy,
};
use vsift_infrastructure::{
    BundleSourcePolicy, BundleStatus, CleanOutcome, ContainedFile, ContainedSourceStore,
    FfprobeSourceDuration, FilesystemAdmissionPermit, FilesystemSessionStore, FreeSpaceCheck,
    SessionIndexPage, SessionRootProvisioning, SessionStatus, SourceSnapshot,
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
    /// for its retry and left the failed attempt's registration behind (one
    /// extra `initializing` session in about one batch of twenty requests at
    /// concurrency four). The store removes only a
    /// session that was never published and whose registration names **this**
    /// operation ([`FilesystemSessionStore::abandon_unpublished_open`]), through
    /// the routine `session clean` uses. A busy root is retried a few times
    /// within a bound; any other failure, or a root that cannot be reopened,
    /// leaves the registration for `session clean` exactly as before, and the
    /// caller keeps its original failure: this never reports and never masks.
    async fn abandon_failed_open(
        &self,
        root: &Path,
        session_id: &SessionId,
        operation_id: &OperationId,
    ) {
        const ATTEMPTS: u32 = 8;
        let Ok(Some(store)) = self.open_session_store(root, SessionRootProvisioning::ExistingOnly)
        else {
            return;
        };
        let Ok(now) = self.now_unix_seconds() else {
            return;
        };
        for attempt in 1..=ATTEMPTS {
            match store.abandon_unpublished_open(session_id, operation_id, now) {
                Err(SessionStorageError::Busy) if attempt < ATTEMPTS => {
                    tokio::time::sleep(std::time::Duration::from_millis(5 * u64::from(attempt)))
                        .await;
                }
                _ => return,
            }
        }
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
        let store = self.open_session_store(&root, SessionRootProvisioning::ExistingOnly)?;
        Ok((store, now))
    }
}

/// Reads a session's committed status, telling a session that is not
/// published from a failing disk (#277).
///
/// A session id with no folder in the root (never opened, still opening, its
/// opening was interrupted and left only its registration, or cleaned
/// already) used to answer `STORAGE_IO` with no remediation, the answer
/// of a disk that failed, which sends an agent to retry or to look at its
/// storage (or a bare `INVALID_ARGUMENT` when the folder existed without a
/// first generation). It is now [`EngineError::SessionNotPublished`]
/// (`INVALID_ARGUMENT`) with a remediation. A failure that is not that,
/// including a real I/O error, is unchanged.
pub(crate) fn published_status(
    store: &FilesystemSessionStore,
    session_id: &SessionId,
) -> Result<SessionStatus, EngineError> {
    match store.session_status(session_id) {
        Ok(status) => Ok(status),
        Err(error @ (SessionStorageError::Io | SessionStorageError::StateConflict)) => {
            match store.indexed_session_status(session_id) {
                Ok(None) => Err(EngineError::SessionNotPublished),
                Ok(Some(status)) => Ok(status),
                Err(_) => Err(error.into()),
            }
        }
        Err(other) => Err(other.into()),
    }
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

/// Checks a worker workspace's free-space reserve before the source is
/// copied into it: the source's size (`incoming`, read from its metadata;
/// the copy itself still refuses a file that grows) and the 1 GiB reserve
/// must be available. A desktop root is not checked, as before P11.
fn free_space_reserve(
    store: &FilesystemSessionStore,
    incoming: u64,
) -> Result<FreeSpaceReserveCheck, EngineError> {
    if store.workspace_policy().is_none() {
        return Ok(FreeSpaceReserveCheck::NotEnforced);
    }
    Ok(match store.ensure_free_space(incoming)? {
        FreeSpaceCheck::Enforced => FreeSpaceReserveCheck::Enforced,
        FreeSpaceCheck::NotEnforced => FreeSpaceReserveCheck::NotEnforced,
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
