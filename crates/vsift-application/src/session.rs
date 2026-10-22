//! Foreground source-to-session orchestration over narrow storage and source ports.

use std::{error::Error, fmt, num::NonZeroU32, path::PathBuf};

use vsift_domain::{
    DownloadLimits, DurabilityRequirement, OperationId, PublicationGuarantee, SessionId,
    SessionLifetime, SessionLifetimePolicy, SourceFetchReason, SourceId, SourceLink, SourceOrigin,
    StorageGeneration, TranscriptImportError, TranscriptRevision, TranscriptRevisionError,
};

use crate::{
    storage::{
        InitializeSessionStorage, InitializeSessionStorageRequest, InitializedSessionStorage,
        SessionStorageError, SessionStore,
    },
    transcript::{
        ImportedRevisionRequest, SourceDurationProbe, SourceProbeError, TranscriptBuildError,
        TranscriptImportRequest, build_imported_revision,
    },
};

/// A caller's request that a long stage stop at its next boundary (P10 PR 3):
/// the first Ctrl-C or `SIGTERM` of a command-line run, or a library host's
/// cancellation. Adapters check it between bounded units of work.
pub trait StageCancellation: Send + Sync {
    /// Whether the caller has asked the stage to stop.
    fn is_cancelled(&self) -> bool;
}

/// A stage nobody can cancel, for callers without a cancellation signal.
#[derive(Clone, Copy, Debug, Default)]
pub struct NeverCancelled;

impl StageCancellation for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// A held, verified source snapshot produced by an infrastructure adapter.
pub trait StagedSessionSource {
    /// Hash identity of the private bytes.
    fn source_id(&self) -> &SourceId;
    /// Bound source byte count.
    fn source_bytes(&self) -> u64;
    /// Where the bytes were fetched from, for a source fetched from a link
    /// (ADR 0028); `None` for a local file's copy.
    fn origin(&self) -> Option<&SourceOrigin> {
        None
    }
}

/// Contained source and lifecycle operations needed to open one investigation.
pub trait ForegroundSessionPort: SessionStore {
    /// Held registration preventing cleanup of a suspended opener.
    type Registration;
    /// Held source snapshot preventing cleanup during activation.
    type Snapshot: StagedSessionSource;

    /// Registers a fresh opaque ID in the bounded disposable-session index.
    ///
    /// # Errors
    ///
    /// Rejects duplicate, full, inaccessible, or corrupt index state.
    fn register(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        now_unix_seconds: u64,
    ) -> Result<Self::Registration, OpenSessionError>;

    /// The lifetime rules of sessions this root opens: the desktop policy,
    /// or a worker workspace's retention (ADR 0021 D2). The use case reports
    /// the lifetime the adapter will commit, so the two never disagree.
    fn lifetime_policy(&self) -> SessionLifetimePolicy;

    /// Copies and hashes one selected local source under the session
    /// capability, checking `cancellation` between bounded blocks.
    ///
    /// # Errors
    ///
    /// Rejects unsafe, changed, or unreadable input and storage failures;
    /// [`OpenSessionError::Cancelled`] when the caller stopped the copy.
    fn stage_source(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        source: &std::path::Path,
        cancellation: &dyn StageCancellation,
    ) -> Result<Self::Snapshot, OpenSessionError>;

    /// Fetches one video from a direct HTTPS link into the session's source
    /// slot, within `limits`, checking `cancellation` as the bytes arrive
    /// (ADR 0028). The snapshot records the fetch's origin.
    ///
    /// # Errors
    ///
    /// [`OpenSessionError::SourceFetch`] with the typed reason, a storage
    /// failure, [`OpenSessionError::SourceNoRoom`] or
    /// [`OpenSessionError::Cancelled`]; a failed fetch leaves no file.
    fn stage_link(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        link: &SourceLink,
        limits: DownloadLimits,
        cancellation: &dyn StageCancellation,
    ) -> impl Future<Output = Result<Self::Snapshot, OpenSessionError>> + Send;

    /// Publishes source binding and expiry in a fenced immutable generation.
    ///
    /// # Errors
    ///
    /// Rejects stale publication or a changed source snapshot.
    fn activate(
        &self,
        snapshot: &Self::Snapshot,
        operation_id: &OperationId,
        expected_generation: StorageGeneration,
        now_unix_seconds: u64,
    ) -> Result<StorageGeneration, OpenSessionError>;

    /// Publishes source binding, expiry and one transcript revision in a single
    /// fenced generation, so a session never becomes open without the
    /// transcript it was requested with.
    ///
    /// # Errors
    ///
    /// Rejects stale publication, a changed snapshot or an oversized record.
    fn activate_with_transcript(
        &self,
        snapshot: &Self::Snapshot,
        operation_id: &OperationId,
        expected_generation: StorageGeneration,
        now_unix_seconds: u64,
        transcript: &TranscriptRevision,
    ) -> Result<StorageGeneration, OpenSessionError>;
}

/// Where a new session's source comes from.
#[derive(Clone, Debug)]
pub enum SessionSource {
    /// A local file the caller selected, copied into the session.
    Local(PathBuf),
    /// A direct HTTPS link to a video file, fetched into the session within
    /// its limits (ADR 0028).
    Link {
        /// The link, never printed or stored.
        link: SourceLink,
        /// The fetch's size and time bounds.
        limits: DownloadLimits,
    },
}

/// Explicit source, identity, guarantee, and clock inputs for one foreground open.
#[derive(Clone, Debug)]
pub struct OpenSessionRequest {
    /// Where the source comes from: a local file or a link.
    pub source: SessionSource,
    /// Fresh opaque session identity.
    pub session_id: SessionId,
    /// Idempotency identity for initialization.
    pub initialize_operation_id: OperationId,
    /// Identity of the private source snapshot.
    pub stage_operation_id: OperationId,
    /// Identity of the activation publication.
    pub activate_operation_id: OperationId,
    /// Strongest guarantee the caller requires.
    pub durability: DurabilityRequirement,
    /// Injected wall-clock seconds for deterministic expiry decisions.
    pub now_unix_seconds: u64,
}

/// Opened session and the actual, bounded publication contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenSessionOutcome {
    /// Fresh session identity.
    pub session_id: SessionId,
    /// SHA-256 identity of the private source bytes.
    pub source_id: SourceId,
    /// Size of the source copy.
    pub source_bytes: u64,
    /// Committed activation generation.
    pub generation: StorageGeneration,
    /// Qualified publication guarantee actually supplied.
    pub publication: PublicationGuarantee,
    /// Default expiry from the initial open.
    pub lifetime: SessionLifetime,
    /// Where a fetched source came from (ADR 0028); `None` for a local file.
    pub origin: Option<SourceOrigin>,
}

/// Why the path a caller named could not be opened as a file, in the three
/// cases the operating system tells apart plainly (#369).
///
/// Every one of them is answered with the published code `STORAGE_IO`, which is
/// what 0.2.0 gave for all of them: a published failure code is not changed
/// within v1 (known limit L-127). The code says a disk failed, which is not what
/// happened, so the cause travels in the typed value and the remediation says
/// what is true. Any other failure to open a path stays an untyped I/O failure
/// ([`OpenSessionError::SourceIo`], [`SuppliedTranscriptError::Io`]): guessing a
/// cause the system did not name would be a remediation that is not true.
///
/// [`SuppliedTranscriptError::Io`]: crate::SuppliedTranscriptError::Io
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourcePathFault {
    /// No file is at the path: the name does not exist, or a folder on the way
    /// to it does not exist or is not a folder. The most ordinary first mistake
    /// (a mistyped name, a command run from another folder) lands here.
    NotFound,
    /// The path names a folder. Windows refuses to open a folder as a file, so
    /// the open fails there; Linux and macOS open it and then find it is no
    /// regular file, which is `INVALID_SOURCE` and never reaches this variant
    /// (the answer each platform has always given, so neither changes).
    IsFolder,
    /// The operating system refused to open the file for reading: the account
    /// running `VSift` is not allowed to read it.
    AccessDenied,
}

impl SourcePathFault {
    /// Every fault, in declaration order.
    pub const ALL: [Self; 3] = [Self::NotFound, Self::IsFolder, Self::AccessDenied];
}

impl fmt::Display for SourcePathFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotFound => "no file is at the selected path",
            Self::IsFolder => "the selected path is a folder",
            Self::AccessDenied => "the selected path could not be opened for reading",
        })
    }
}

/// What a selected path names when it is neither a regular file nor a folder
/// nor a link: an entry that `VSift` refuses from its directory entry, before
/// anything opens it (#264).
///
/// Opening such an entry can wait for ever (a named pipe that nothing writes
/// to) or act on a device (a tape that rewinds, a terminal that becomes the
/// process's own, a watchdog that starts), so the kind is read without
/// opening. Each kind keeps the published code it has had since 0.2.0, which
/// is why one cause has two codes: a named pipe and a device are
/// `INVALID_SOURCE`, as documented, and a socket is `STORAGE_IO`, because its
/// open always failed (known limit L-127: a published failure code does not
/// change within v1). The remediation is the same for all three.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpecialFileKind {
    /// A named pipe (FIFO).
    NamedPipe,
    /// A character or block device.
    Device,
    /// A local (UNIX-domain) socket.
    Socket,
}

impl SpecialFileKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 3] = [Self::NamedPipe, Self::Device, Self::Socket];
}

impl fmt::Display for SpecialFileKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NamedPipe => "the selected path is a named pipe, not a regular file",
            Self::Device => "the selected path is a device, not a regular file",
            Self::Socket => "the selected path is a socket, not a regular file",
        })
    }
}

/// Expected failure of a foreground session open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenSessionError {
    /// Selected source was invalid, unsupported, or changed while staged. A
    /// copy that ran out of time is [`Self::SourceCopyTooSlow`], which says
    /// nothing about the source.
    InvalidSource,
    /// Source input could not be read.
    SourceIo,
    /// The selected source path could not be opened, and the operating system
    /// said why: no file is there, it is a folder, or reading it is not
    /// allowed (#369). The published code stays `STORAGE_IO`, the one 0.2.0
    /// gave, with a remediation that says what happened (known limit L-127:
    /// changing a published failure code is not additive within v1).
    SourcePath(SourcePathFault),
    /// The selected source's own final path component is a symbolic link (or
    /// another reparse-point link). The link is not followed and the caller
    /// names the file itself (#265). The published code stays `STORAGE_IO`, the
    /// one 0.1.0 gave, with a remediation that says what happened (known limit
    /// L-127: changing a published failure code is not additive within v1).
    SourceIsLink,
    /// The selected source is a named pipe, a device or a socket, refused from
    /// its directory entry before anything opened it (#264). The published
    /// code depends on the kind ([`SpecialFileKind`]) and the remediation says
    /// what happened.
    SourceSpecialFile(SpecialFileKind),
    /// The session root's filesystem has no room for the copy of the source:
    /// found before the copy from the source's size and the free space, or
    /// when a write ran out of space (#266). Nothing was activated. The
    /// published code stays `STORAGE_IO`, what the CLI path gave, with a
    /// remediation that says what happened (known limit L-127: changing a
    /// published failure code is not additive within v1).
    SourceNoRoom,
    /// The copy of the selected source into the session took longer than the
    /// time a copy is allowed and was stopped; nothing was activated (#325).
    /// The source was not judged: a healthy video on a slow disk, a network
    /// share or a folder that downloads on first read ends here. The
    /// published code stays `INVALID_SOURCE`, what this case always gave, with
    /// a remediation that says what happened (known limit L-127: changing a
    /// published failure code is not additive within v1).
    SourceCopyTooSlow,
    /// The copy of the selected source was stopped early because, at the
    /// fastest speed measured in its first minute or more, it would have ended
    /// well after the time a copy is allowed (#325). Like
    /// [`Self::SourceCopyTooSlow`] it says nothing about the source, keeps the
    /// published code `INVALID_SOURCE` and has its own remediation; nothing was
    /// activated.
    SourceCopyWouldBeTooSlow,
    /// The clock could not represent the bounded lifetime.
    InvalidClock,
    /// Storage or coordination rejected the operation.
    Storage(SessionStorageError),
    /// The source duration needed to align a supplied transcript is unavailable.
    SourceProbe(SourceProbeError),
    /// The supplied transcript could not be aligned with the source.
    TranscriptRejected(TranscriptImportError),
    /// An assembled transcript revision violated an invariant (internal fault).
    TranscriptInvalid(TranscriptRevisionError),
    /// The caller cancelled while the source was copied; nothing was
    /// activated, and the unactivated registration is removed like that of any
    /// failed open.
    Cancelled,
    /// A link given as the source could not be fetched (ADR 0028), for the
    /// typed reason: published as `SOURCE_FETCH_FAILED`. Nothing was
    /// activated and no partial file remains; local files are unaffected.
    SourceFetch(SourceFetchReason),
}

impl fmt::Display for OpenSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSource => formatter.write_str("selected source is invalid"),
            Self::SourceIo => formatter.write_str("selected source could not be read"),
            Self::SourcePath(fault) => fault.fmt(formatter),
            Self::SourceIsLink => {
                formatter.write_str("selected source is a link, which is not followed")
            }
            Self::SourceSpecialFile(kind) => kind.fmt(formatter),
            Self::SourceNoRoom => {
                formatter.write_str("the session root has no room for a copy of the source")
            }
            Self::SourceCopyTooSlow => {
                formatter.write_str("copying the selected source took longer than the time limit")
            }
            Self::SourceCopyWouldBeTooSlow => formatter
                .write_str("copying the selected source would take longer than the time limit"),
            Self::InvalidClock => {
                formatter.write_str("session clock is outside the supported range")
            }
            Self::Storage(error) => error.fmt(formatter),
            Self::SourceProbe(error) => error.fmt(formatter),
            Self::TranscriptRejected(error) => error.fmt(formatter),
            Self::TranscriptInvalid(error) => error.fmt(formatter),
            Self::Cancelled => formatter.write_str("opening the session was cancelled"),
            Self::SourceFetch(reason) => reason.fmt(formatter),
        }
    }
}

impl Error for OpenSessionError {}

impl From<SessionStorageError> for OpenSessionError {
    fn from(value: SessionStorageError) -> Self {
        Self::Storage(value)
    }
}

/// Use case that owns the registration, staging, and commit order.
pub struct OpenSession<S> {
    port: S,
}

impl<S: ForegroundSessionPort> OpenSession<S> {
    /// Creates the use case over an explicit private-root adapter.
    pub const fn new(port: S) -> Self {
        Self { port }
    }

    /// Opens one disposable session without silently upgrading persistence.
    ///
    /// # Errors
    ///
    /// Unsupported durable requests fail before registering or mutating state.
    /// A failure after the session was registered leaves the registration, and
    /// the initial session folder if one was made, for the caller to remove: the
    /// engine does so at once with
    /// `FilesystemSessionStore::abandon_unpublished_open` (#277), and a caller
    /// that cannot leaves them for `session clean` after the idle interval.
    pub async fn execute(
        &self,
        request: OpenSessionRequest,
        cancellation: &dyn StageCancellation,
    ) -> Result<OpenSessionOutcome, OpenSessionError> {
        let prepared = self.prepare(&request, cancellation).await?;
        let generation = self.port.activate(
            &prepared.snapshot,
            &request.activate_operation_id,
            prepared.initialized.generation(),
            request.now_unix_seconds,
        )?;
        Ok(prepared.outcome(request.session_id, generation))
    }

    /// Opens one disposable session together with a supplied transcript.
    ///
    /// The source is staged and probed first; the transcript is aligned with
    /// the probed duration under the domain policy; only then are the source
    /// binding and the transcript revision published in one generation. A
    /// rejected transcript therefore never leaves an open session behind: the
    /// unactivated registration is abandoned and cleaned like any interrupted
    /// open. The imported revision is the session's first (number 1).
    ///
    /// # Errors
    ///
    /// As [`Self::execute`], plus [`OpenSessionError::SourceProbe`] when the
    /// duration is unavailable and [`OpenSessionError::TranscriptRejected`]
    /// when no supplied cue lies within the source.
    pub async fn execute_with_transcript<P>(
        &self,
        request: OpenSessionRequest,
        import: &TranscriptImportRequest,
        probe: &P,
        cancellation: &dyn StageCancellation,
    ) -> Result<(OpenSessionOutcome, TranscriptRevision), OpenSessionError>
    where
        P: SourceDurationProbe<S::Snapshot>,
    {
        let prepared = self.prepare(&request, cancellation).await?;
        let duration = probe
            .source_duration(&prepared.snapshot)
            .await
            .map_err(OpenSessionError::SourceProbe)?;
        let revision = build_imported_revision(ImportedRevisionRequest {
            session_id: &request.session_id,
            source_id: prepared.snapshot.source_id(),
            source_duration: duration,
            supplied: &import.supplied,
            offset: import.offset,
            number: NonZeroU32::MIN,
        })
        .map_err(|error| match error {
            TranscriptBuildError::Rejected(rejection) => {
                OpenSessionError::TranscriptRejected(rejection)
            }
            TranscriptBuildError::Invalid(invalid) => OpenSessionError::TranscriptInvalid(invalid),
        })?;
        let generation = self.port.activate_with_transcript(
            &prepared.snapshot,
            &request.activate_operation_id,
            prepared.initialized.generation(),
            request.now_unix_seconds,
            &revision,
        )?;
        Ok((prepared.outcome(request.session_id, generation), revision))
    }

    /// Runs every step before activation: guarantee preflight, lifetime,
    /// registration, initialization and source staging.
    async fn prepare(
        &self,
        request: &OpenSessionRequest,
        cancellation: &dyn StageCancellation,
    ) -> Result<PreparedOpen<S::Registration, S::Snapshot>, OpenSessionError> {
        let capabilities = self.port.capabilities();
        if !capabilities.supports(request.durability) {
            return Err(SessionStorageError::UnsupportedGuarantee {
                requested: request.durability,
                available: capabilities.publication(),
            }
            .into());
        }
        let lifetime =
            SessionLifetime::open_under(self.port.lifetime_policy(), request.now_unix_seconds)
                .map_err(|_| OpenSessionError::InvalidClock)?;
        let registration = self.port.register(
            &request.session_id,
            &request.initialize_operation_id,
            request.now_unix_seconds,
        )?;
        let initialized = InitializeSessionStorage::new(&self.port)
            .execute(InitializeSessionStorageRequest::new(
                request.session_id.clone(),
                request.initialize_operation_id.clone(),
                request.durability,
            ))
            .await?;
        let snapshot = match &request.source {
            SessionSource::Local(path) => self.port.stage_source(
                &request.session_id,
                &request.stage_operation_id,
                path,
                cancellation,
            )?,
            SessionSource::Link { link, limits } => {
                self.port
                    .stage_link(
                        &request.session_id,
                        &request.stage_operation_id,
                        link,
                        *limits,
                        cancellation,
                    )
                    .await?
            }
        };
        Ok(PreparedOpen {
            _registration: registration,
            initialized,
            snapshot,
            // The session's own guarantee, not the store's strongest: an
            // ephemeral session on a qualified root commits without the
            // durable protocol and must never be reported as durable.
            publication: request.durability.required_guarantee(),
            lifetime,
        })
    }
}

/// Held state between staging and activation.
///
/// The registration and snapshot holds must outlive activation, so they stay
/// together until the caller has published and built its outcome.
struct PreparedOpen<R, T> {
    _registration: R,
    initialized: InitializedSessionStorage,
    snapshot: T,
    publication: PublicationGuarantee,
    lifetime: SessionLifetime,
}

impl<R, T: StagedSessionSource> PreparedOpen<R, T> {
    fn outcome(&self, session_id: SessionId, generation: StorageGeneration) -> OpenSessionOutcome {
        OpenSessionOutcome {
            session_id,
            source_id: self.snapshot.source_id().clone(),
            source_bytes: self.snapshot.source_bytes(),
            generation,
            publication: self.publication,
            lifetime: self.lifetime,
            origin: self.snapshot.origin().cloned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        future::{Future, ready},
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use vsift_domain::{
        DurabilityRequirement, OperationId, PublicationGuarantee, SessionId, SourceId,
        StorageGeneration,
    };

    use crate::{
        AuthorizedSessionGenerationPublication, AuthorizedSessionStorageInitialization,
        SessionStorageError, SessionStore, StorageCapabilities,
    };

    use super::{
        ForegroundSessionPort, OpenSession, OpenSessionError, OpenSessionRequest,
        StagedSessionSource,
    };

    struct FakeSnapshot(SourceId);

    impl StagedSessionSource for FakeSnapshot {
        fn source_id(&self) -> &SourceId {
            &self.0
        }

        fn source_bytes(&self) -> u64 {
            1
        }
    }

    struct FakePort {
        calls: Arc<AtomicUsize>,
    }

    impl SessionStore for FakePort {
        fn capabilities(&self) -> StorageCapabilities {
            StorageCapabilities::new(PublicationGuarantee::ProcessCrashConsistent)
        }

        fn initialize(
            &self,
            _request: AuthorizedSessionStorageInitialization,
        ) -> impl Future<Output = Result<StorageGeneration, SessionStorageError>> + Send {
            self.calls.fetch_add(1, Ordering::SeqCst);
            ready(Err(SessionStorageError::Io))
        }

        fn publish(
            &self,
            _request: AuthorizedSessionGenerationPublication,
        ) -> impl Future<Output = Result<StorageGeneration, SessionStorageError>> + Send {
            self.calls.fetch_add(1, Ordering::SeqCst);
            ready(Err(SessionStorageError::Io))
        }
    }

    impl ForegroundSessionPort for FakePort {
        type Registration = ();
        type Snapshot = FakeSnapshot;

        fn lifetime_policy(&self) -> vsift_domain::SessionLifetimePolicy {
            vsift_domain::SessionLifetimePolicy::Desktop
        }

        fn register(
            &self,
            _session_id: &SessionId,
            _operation_id: &OperationId,
            _now_unix_seconds: u64,
        ) -> Result<Self::Registration, OpenSessionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(SessionStorageError::Io.into())
        }

        fn stage_source(
            &self,
            _session_id: &SessionId,
            _operation_id: &OperationId,
            _source: &std::path::Path,
            _cancellation: &dyn super::StageCancellation,
        ) -> Result<Self::Snapshot, OpenSessionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(SessionStorageError::Io.into())
        }

        fn stage_link(
            &self,
            _session_id: &SessionId,
            _operation_id: &OperationId,
            _link: &vsift_domain::SourceLink,
            _limits: vsift_domain::DownloadLimits,
            _cancellation: &dyn super::StageCancellation,
        ) -> impl Future<Output = Result<Self::Snapshot, OpenSessionError>> + Send {
            self.calls.fetch_add(1, Ordering::SeqCst);
            ready(Err(SessionStorageError::Io.into()))
        }

        fn activate(
            &self,
            _snapshot: &Self::Snapshot,
            _operation_id: &OperationId,
            _expected_generation: StorageGeneration,
            _now_unix_seconds: u64,
        ) -> Result<StorageGeneration, OpenSessionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(SessionStorageError::Io.into())
        }

        fn activate_with_transcript(
            &self,
            _snapshot: &Self::Snapshot,
            _operation_id: &OperationId,
            _expected_generation: StorageGeneration,
            _now_unix_seconds: u64,
            _transcript: &vsift_domain::TranscriptRevision,
        ) -> Result<StorageGeneration, OpenSessionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(SessionStorageError::Io.into())
        }
    }

    #[tokio::test]
    async fn durable_open_rejects_before_registration_or_storage_mutation()
    -> Result<(), Box<dyn Error>> {
        let calls = Arc::new(AtomicUsize::new(0));
        let use_case = OpenSession::new(FakePort {
            calls: Arc::clone(&calls),
        });
        let result = use_case
            .execute(
                OpenSessionRequest {
                    source: super::SessionSource::Local(PathBuf::from("unused")),
                    session_id: SessionId::parse("ses_0123456789abcdef")?,
                    initialize_operation_id: OperationId::parse("op_0123456789abcdef")?,
                    stage_operation_id: OperationId::parse("op_1111111111111111")?,
                    activate_operation_id: OperationId::parse("op_2222222222222222")?,
                    durability: DurabilityRequirement::Durable,
                    now_unix_seconds: 1_000,
                },
                &super::NeverCancelled,
            )
            .await;
        assert_eq!(
            result,
            Err(OpenSessionError::Storage(
                SessionStorageError::UnsupportedGuarantee {
                    requested: DurabilityRequirement::Durable,
                    available: PublicationGuarantee::ProcessCrashConsistent,
                }
            ))
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        Ok(())
    }
}

#[cfg(test)]
mod transcript_open_tests {
    use std::{
        error::Error,
        future::{Future, ready},
        num::NonZeroU32,
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use vsift_domain::{
        AddressCheck, CueSource, CueText, CueTiming, DirectLinkOrigin, DownloadLimits,
        DurabilityRequirement, HostName, ImportedCue, MediaTime, OperationId, ParsedTranscript,
        PublicationGuarantee, SessionId, Sha256Hex, SidecarIdentity, SourceFetchReason, SourceId,
        SourceLink, SourceOrigin, StorageGeneration, TranscriptFormat, TranscriptImportError,
        TranscriptOffset, TranscriptRejection, TranscriptRevision, TranscriptWarnings,
    };

    use crate::{
        AuthorizedSessionGenerationPublication, AuthorizedSessionStorageInitialization,
        SessionStorageError, SessionStore, SourceDurationProbe, SourceProbeError,
        StorageCapabilities, SuppliedTranscript, TranscriptImportRequest,
    };

    use super::{
        ForegroundSessionPort, OpenSession, OpenSessionError, OpenSessionRequest,
        StagedSessionSource,
    };

    type TestResult = Result<(), Box<dyn Error>>;

    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    struct Snapshot(SourceId, Option<SourceOrigin>);

    impl StagedSessionSource for Snapshot {
        fn source_id(&self) -> &SourceId {
            &self.0
        }

        fn source_bytes(&self) -> u64 {
            1_024
        }

        fn origin(&self) -> Option<&SourceOrigin> {
            self.1.as_ref()
        }
    }

    /// Records which activation ran and the revision it published.
    #[derive(Clone, Default)]
    struct RecordingPort {
        activations: Arc<Mutex<Vec<Option<TranscriptRevision>>>>,
    }

    impl RecordingPort {
        fn activations(&self) -> Vec<Option<TranscriptRevision>> {
            self.activations
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
        }

        fn record(&self, revision: Option<TranscriptRevision>) {
            if let Ok(mut guard) = self.activations.lock() {
                guard.push(revision);
            }
        }
    }

    impl SessionStore for RecordingPort {
        fn capabilities(&self) -> StorageCapabilities {
            StorageCapabilities::new(PublicationGuarantee::ProcessCrashConsistent)
        }

        fn initialize(
            &self,
            _request: AuthorizedSessionStorageInitialization,
        ) -> impl Future<Output = Result<StorageGeneration, SessionStorageError>> + Send {
            ready(Ok(StorageGeneration::INITIAL))
        }

        fn publish(
            &self,
            _request: AuthorizedSessionGenerationPublication,
        ) -> impl Future<Output = Result<StorageGeneration, SessionStorageError>> + Send {
            ready(Err(SessionStorageError::Io))
        }
    }

    impl ForegroundSessionPort for RecordingPort {
        type Registration = ();
        type Snapshot = Snapshot;

        fn lifetime_policy(&self) -> vsift_domain::SessionLifetimePolicy {
            vsift_domain::SessionLifetimePolicy::Desktop
        }

        fn register(
            &self,
            _session_id: &SessionId,
            _operation_id: &OperationId,
            _now_unix_seconds: u64,
        ) -> Result<Self::Registration, OpenSessionError> {
            Ok(())
        }

        fn stage_source(
            &self,
            _session_id: &SessionId,
            _operation_id: &OperationId,
            _source: &std::path::Path,
            _cancellation: &dyn super::StageCancellation,
        ) -> Result<Self::Snapshot, OpenSessionError> {
            Ok(Snapshot(
                SourceId::from_sha256(DIGEST).map_err(|_| OpenSessionError::InvalidSource)?,
                None,
            ))
        }

        fn stage_link(
            &self,
            _session_id: &SessionId,
            _operation_id: &OperationId,
            link: &SourceLink,
            limits: DownloadLimits,
            _cancellation: &dyn super::StageCancellation,
        ) -> impl Future<Output = Result<Self::Snapshot, OpenSessionError>> + Send {
            // A link whose host is `fails.example.test` fails offline; any
            // other is fetched at the limit's size.
            let result = if link.expose().contains("fails.example.test") {
                Err(OpenSessionError::SourceFetch(SourceFetchReason::Offline))
            } else {
                origin(limits.max_bytes()).and_then(|origin| {
                    Ok(Snapshot(
                        SourceId::from_sha256(DIGEST)
                            .map_err(|_| OpenSessionError::InvalidSource)?,
                        Some(origin),
                    ))
                })
            };
            ready(result)
        }

        fn activate(
            &self,
            _snapshot: &Self::Snapshot,
            _operation_id: &OperationId,
            _expected_generation: StorageGeneration,
            _now_unix_seconds: u64,
        ) -> Result<StorageGeneration, OpenSessionError> {
            self.record(None);
            Ok(StorageGeneration::from_value(1))
        }

        fn activate_with_transcript(
            &self,
            _snapshot: &Self::Snapshot,
            _operation_id: &OperationId,
            _expected_generation: StorageGeneration,
            _now_unix_seconds: u64,
            transcript: &TranscriptRevision,
        ) -> Result<StorageGeneration, OpenSessionError> {
            self.record(Some(transcript.clone()));
            Ok(StorageGeneration::from_value(1))
        }
    }

    struct FixedDuration(Result<MediaTime, SourceProbeError>);

    impl SourceDurationProbe<Snapshot> for FixedDuration {
        fn source_duration(
            &self,
            _source: &Snapshot,
        ) -> impl Future<Output = Result<MediaTime, SourceProbeError>> + Send {
            ready(self.0)
        }
    }

    /// The origin a stand-in fetch of `bytes` records.
    fn origin(bytes: u64) -> Result<SourceOrigin, OpenSessionError> {
        let host =
            HostName::parse("videos.example.test").map_err(|_| OpenSessionError::InvalidSource)?;
        let digest = Sha256Hex::parse(DIGEST).map_err(|_| OpenSessionError::InvalidSource)?;
        DirectLinkOrigin::new(
            digest,
            (host.clone(), host),
            0,
            AddressCheck::Direct,
            1_000,
            bytes,
        )
        .map(SourceOrigin::DirectHttps)
        .map_err(|_| OpenSessionError::InvalidSource)
    }

    fn link_request(link: &str) -> Result<OpenSessionRequest, Box<dyn Error>> {
        let mut request = request()?;
        request.source = super::SessionSource::Link {
            link: SourceLink::parse(link)?,
            limits: DownloadLimits::new(Some(2_048), None)?,
        };
        Ok(request)
    }

    /// ADR 0028: a link source is fetched by the port's `stage_link`, and the
    /// outcome carries the origin the snapshot recorded.
    #[tokio::test]
    async fn a_link_source_is_fetched_and_its_origin_reported() -> TestResult {
        let port = RecordingPort::default();
        let opened = OpenSession::new(port.clone())
            .execute(
                link_request("https://videos.example.test/a.mp4?token=t")?,
                &super::NeverCancelled,
            )
            .await?;
        let Some(SourceOrigin::DirectHttps(origin)) = opened.origin else {
            return Err("no origin".into());
        };
        assert_eq!(origin.bytes(), 2_048, "the limits reached the port");
        assert_eq!(port.activations(), [None]);
        Ok(())
    }

    /// A failed fetch never activates the session and keeps its reason.
    #[tokio::test]
    async fn a_failed_fetch_never_activates_the_session() -> TestResult {
        let port = RecordingPort::default();
        let result = OpenSession::new(port.clone())
            .execute(
                link_request("https://fails.example.test/a.mp4")?,
                &super::NeverCancelled,
            )
            .await;
        assert_eq!(
            result.map(|opened| opened.generation),
            Err(OpenSessionError::SourceFetch(SourceFetchReason::Offline))
        );
        assert_eq!(port.activations(), [] as [Option<TranscriptRevision>; 0]);
        Ok(())
    }

    fn request() -> Result<OpenSessionRequest, Box<dyn Error>> {
        Ok(OpenSessionRequest {
            source: super::SessionSource::Local(PathBuf::from("unused")),
            session_id: SessionId::parse("ses_0123456789abcdef")?,
            initialize_operation_id: OperationId::parse("op_0123456789abcdef")?,
            stage_operation_id: OperationId::parse("op_1111111111111111")?,
            activate_operation_id: OperationId::parse("op_2222222222222222")?,
            durability: DurabilityRequirement::Ephemeral,
            now_unix_seconds: 1_000,
        })
    }

    /// The F10 shape: sidecar cues 500 ms early, aligned by an explicit +500 ms.
    fn f10_import(offset_micros: i64) -> Result<TranscriptImportRequest, Box<dyn Error>> {
        let mut cues = Vec::new();
        for (index, (start, end, text)) in [
            (
                500_000,
                3_500_000,
                "This sidecar uses a 500 millisecond offset.",
            ),
            (4_500_000, 8_500_000, "Dialog R-17 is displayed now."),
        ]
        .into_iter()
        .enumerate()
        {
            let ordinal = u32::try_from(index + 1)?;
            cues.push(ImportedCue {
                source: CueSource::new(
                    NonZeroU32::new(ordinal).ok_or("zero")?,
                    NonZeroU32::new(ordinal * 4).ok_or("zero")?,
                ),
                timing: CueTiming::new(start, end)?,
                text: CueText::new(text.to_owned(), text.to_owned())?,
                speaker: None,
            });
        }
        Ok(TranscriptImportRequest {
            supplied: SuppliedTranscript {
                transcript: ParsedTranscript::new(
                    TranscriptFormat::Srt,
                    None,
                    cues,
                    TranscriptWarnings::default(),
                )?,
                sidecar: SidecarIdentity::new(DIGEST, 200)?,
            },
            offset: TranscriptOffset::from_micros(offset_micros)?,
        })
    }

    #[tokio::test]
    async fn supplied_transcript_is_published_with_activation_in_one_generation() -> TestResult {
        let port = RecordingPort::default();
        let use_case = OpenSession::new(port.clone());

        let (opened, revision) = use_case
            .execute_with_transcript(
                request()?,
                &f10_import(500_000)?,
                &FixedDuration(Ok(MediaTime::from_micros(12_000_000))),
                &super::NeverCancelled,
            )
            .await?;

        assert_eq!(opened.generation, StorageGeneration::from_value(1));
        assert_eq!(revision.number(), 1);
        let ranges: Vec<(u64, u64)> = revision
            .segments()
            .iter()
            .map(|segment| {
                (
                    segment.range().start().as_micros(),
                    segment.range().end().as_micros(),
                )
            })
            .collect();
        assert_eq!(ranges, [(1_000_000, 4_000_000), (5_000_000, 9_000_000)]);
        assert_eq!(port.activations(), [Some(revision)]);
        Ok(())
    }

    /// T-02: a wrong offset rejects the import before any activation, so no
    /// session becomes open without its transcript.
    #[tokio::test]
    async fn rejected_alignment_never_activates_the_session() -> TestResult {
        let port = RecordingPort::default();
        let use_case = OpenSession::new(port.clone());

        let result = use_case
            .execute_with_transcript(
                request()?,
                &f10_import(-20_000_000)?,
                &FixedDuration(Ok(MediaTime::from_micros(12_000_000))),
                &super::NeverCancelled,
            )
            .await;

        assert_eq!(
            result.map(|(opened, _)| opened.generation),
            Err(OpenSessionError::TranscriptRejected(
                TranscriptImportError::new(TranscriptRejection::NoCuesWithinSource)
            ))
        );
        assert_eq!(
            port.activations(),
            [] as [std::option::Option<vsift_domain::TranscriptRevision>; 0]
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_unprobeable_source_never_activates_the_session() -> TestResult {
        let port = RecordingPort::default();
        let use_case = OpenSession::new(port.clone());

        let result = use_case
            .execute_with_transcript(
                request()?,
                &f10_import(0)?,
                &FixedDuration(Err(SourceProbeError::InvalidSource)),
                &super::NeverCancelled,
            )
            .await;

        assert_eq!(
            result.map(|(opened, _)| opened.generation),
            Err(OpenSessionError::SourceProbe(
                SourceProbeError::InvalidSource
            ))
        );
        assert_eq!(
            port.activations(),
            [] as [std::option::Option<vsift_domain::TranscriptRevision>; 0]
        );
        Ok(())
    }
}
