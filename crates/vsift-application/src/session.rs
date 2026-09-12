//! Foreground source-to-session orchestration over narrow storage and source ports.

use std::{error::Error, fmt, path::PathBuf};

use vsift_domain::{
    DurabilityRequirement, OperationId, PublicationGuarantee, SessionId, SessionLifetime, SourceId,
    StorageGeneration,
};

use crate::storage::{
    InitializeSessionStorage, InitializeSessionStorageRequest, SessionStorageError, SessionStore,
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
        let _registration = self.port.register(
            &request.session_id,
            &request.initialize_operation_id,
            request.now_unix_seconds,
        )?;
        let initialized = InitializeSessionStorage::new(&self.port)
            .execute(InitializeSessionStorageRequest::new(
                request.session_id.clone(),
                request.initialize_operation_id,
                request.durability,
            ))
            .await?;
        let snapshot = self.port.stage_source(
            &request.session_id,
            &request.stage_operation_id,
            &request.source,
        )?;
        let generation = self.port.activate(
            &snapshot,
            &request.activate_operation_id,
            initialized.generation(),
            request.now_unix_seconds,
        )?;
        Ok(OpenSessionOutcome {
            session_id: request.session_id,
            source_id: snapshot.source_id().clone(),
            source_bytes: snapshot.source_bytes(),
            generation,
            publication: capabilities.publication(),
            lifetime,
        })
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
