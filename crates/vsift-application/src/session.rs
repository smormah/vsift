//! Foreground source-to-session orchestration over narrow storage and source ports.

use std::{error::Error, fmt, num::NonZeroU32, path::PathBuf};

use vsift_domain::{
    DurabilityRequirement, OperationId, PublicationGuarantee, SessionId, SessionLifetime, SourceId,
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

/// A held, verified source snapshot produced by an infrastructure adapter.
pub trait StagedSessionSource {
    /// Hash identity of the private bytes.
    fn source_id(&self) -> &SourceId;
    /// Bound source byte count.
    fn source_bytes(&self) -> u64;
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

    /// Copies and hashes one selected local source under the session capability.
    ///
    /// # Errors
    ///
    /// Rejects unsafe, changed, or unreadable input and storage failures.
    fn stage_source(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        source: &std::path::Path,
    ) -> Result<Self::Snapshot, OpenSessionError>;

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

/// Explicit source, identity, guarantee, and clock inputs for one foreground open.
#[derive(Clone, Debug)]
pub struct OpenSessionRequest {
    /// Caller-selected local media path.
    pub source: PathBuf,
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
}

/// Expected failure of a foreground session open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenSessionError {
    /// Selected source was invalid, unsupported, or changed while staged.
    InvalidSource,
    /// Source input could not be read.
    SourceIo,
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
}

impl fmt::Display for OpenSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSource => formatter.write_str("selected source is invalid"),
            Self::SourceIo => formatter.write_str("selected source could not be read"),
            Self::InvalidClock => {
                formatter.write_str("session clock is outside the supported range")
            }
            Self::Storage(error) => error.fmt(formatter),
            Self::SourceProbe(error) => error.fmt(formatter),
            Self::TranscriptRejected(error) => error.fmt(formatter),
            Self::TranscriptInvalid(error) => error.fmt(formatter),
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
    /// Source failure may leave a registered initial session for bounded cleanup.
    pub async fn execute(
        &self,
        request: OpenSessionRequest,
    ) -> Result<OpenSessionOutcome, OpenSessionError> {
        let prepared = self.prepare(&request).await?;
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
    ) -> Result<(OpenSessionOutcome, TranscriptRevision), OpenSessionError>
    where
        P: SourceDurationProbe<S::Snapshot>,
    {
        let prepared = self.prepare(&request).await?;
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
    ) -> Result<PreparedOpen<S::Registration, S::Snapshot>, OpenSessionError> {
        let capabilities = self.port.capabilities();
        if !capabilities.supports(request.durability) {
            return Err(SessionStorageError::UnsupportedGuarantee {
                requested: request.durability,
                available: capabilities.publication(),
            }
            .into());
        }
        let lifetime = SessionLifetime::open(request.now_unix_seconds)
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
        let snapshot = self.port.stage_source(
            &request.session_id,
            &request.stage_operation_id,
            &request.source,
        )?;
        Ok(PreparedOpen {
            _registration: registration,
            initialized,
            snapshot,
            publication: capabilities.publication(),
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
        ) -> Result<Self::Snapshot, OpenSessionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(SessionStorageError::Io.into())
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
            .execute(OpenSessionRequest {
                source: PathBuf::from("unused"),
                session_id: SessionId::parse("ses_0123456789abcdef")?,
                initialize_operation_id: OperationId::parse("op_0123456789abcdef")?,
                stage_operation_id: OperationId::parse("op_1111111111111111")?,
                activate_operation_id: OperationId::parse("op_2222222222222222")?,
                durability: DurabilityRequirement::Durable,
                now_unix_seconds: 1_000,
            })
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
        CueSource, CueText, CueTiming, DurabilityRequirement, ImportedCue, MediaTime, OperationId,
        ParsedTranscript, PublicationGuarantee, SessionId, SidecarIdentity, SourceId,
        StorageGeneration, TranscriptFormat, TranscriptImportError, TranscriptOffset,
        TranscriptRejection, TranscriptRevision, TranscriptWarnings,
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

    struct Snapshot(SourceId);

    impl StagedSessionSource for Snapshot {
        fn source_id(&self) -> &SourceId {
            &self.0
        }

        fn source_bytes(&self) -> u64 {
            1_024
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
        ) -> Result<Self::Snapshot, OpenSessionError> {
            Ok(Snapshot(
                SourceId::from_sha256(DIGEST).map_err(|_| OpenSessionError::InvalidSource)?,
            ))
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

    fn request() -> Result<OpenSessionRequest, Box<dyn Error>> {
        Ok(OpenSessionRequest {
            source: PathBuf::from("unused"),
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
            )
            .await;

        assert_eq!(
            result.map(|(opened, _)| opened.generation),
            Err(OpenSessionError::TranscriptRejected(
                TranscriptImportError::new(TranscriptRejection::NoCuesWithinSource)
            ))
        );
        assert!(port.activations().is_empty());
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
            )
            .await;

        assert_eq!(
            result.map(|(opened, _)| opened.generation),
            Err(OpenSessionError::SourceProbe(
                SourceProbeError::InvalidSource
            ))
        );
        assert!(port.activations().is_empty());
        Ok(())
    }
}
