//! Storage coordination ports and guarantee-gated use cases.

use std::{error::Error, fmt, future::Future};

use vsift_domain::{
    DurabilityRequirement, OperationId, PublicationGuarantee, SessionId, StorageGeneration,
};

/// Qualified behavior exposed by one configured session store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageCapabilities {
    publication: PublicationGuarantee,
}

impl StorageCapabilities {
    /// Describes an adapter after its host profile has been qualified.
    #[must_use]
    pub const fn new(publication: PublicationGuarantee) -> Self {
        Self { publication }
    }

    /// Returns the strongest publication behavior the adapter may acknowledge.
    #[must_use]
    pub const fn publication(self) -> PublicationGuarantee {
        self.publication
    }

    /// Reports whether the adapter may accept the requested durability.
    #[must_use]
    pub const fn supports(self, requirement: DurabilityRequirement) -> bool {
        self.publication.satisfies(requirement)
    }
}

/// Request to allocate the storage structures for a future investigation session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitializeSessionStorageRequest {
    session_id: SessionId,
    operation_id: OperationId,
    durability: DurabilityRequirement,
}

impl InitializeSessionStorageRequest {
    /// Creates a typed session-storage initialization request.
    #[must_use]
    pub const fn new(
        session_id: SessionId,
        operation_id: OperationId,
        durability: DurabilityRequirement,
    ) -> Self {
        Self {
            session_id,
            operation_id,
            durability,
        }
    }

    /// Returns the session whose private storage should be initialized.
    #[must_use]
    pub const fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Returns the operation responsible for this initialization attempt.
    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    /// Returns the requested durability.
    #[must_use]
    pub const fn durability(&self) -> DurabilityRequirement {
        self.durability
    }
}

/// Preflighted request that a [`SessionStore`] may use to mutate its private root.
///
/// Only [`InitializeSessionStorage`] can create this value. That keeps the
/// unsupported-guarantee check ahead of the first adapter mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizedSessionStorageInitialization {
    session_id: SessionId,
    operation_id: OperationId,
    durability: DurabilityRequirement,
}

impl AuthorizedSessionStorageInitialization {
    /// Returns the session whose private storage is being initialized.
    #[must_use]
    pub const fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Returns the operation responsible for this initialization attempt.
    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    /// Returns the preflighted durability requirement.
    #[must_use]
    pub const fn durability(&self) -> DurabilityRequirement {
        self.durability
    }
}

/// Storage allocation committed for a future investigation session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitializedSessionStorage {
    session_id: SessionId,
    operation_id: OperationId,
    generation: StorageGeneration,
    publication: PublicationGuarantee,
}

impl InitializedSessionStorage {
    /// Returns the initialized session.
    #[must_use]
    pub const fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Returns the operation that initialized the storage.
    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    /// Returns the first immutable generation published for the session.
    #[must_use]
    pub const fn generation(&self) -> StorageGeneration {
        self.generation
    }

    /// Returns the qualified publication behavior used for the commit.
    #[must_use]
    pub const fn publication(&self) -> PublicationGuarantee {
        self.publication
    }
}

/// Expected failure from session storage or its coordination boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionStorageError {
    /// The configured adapter cannot honor the requested persistence behavior.
    UnsupportedGuarantee {
        /// Durability required by the caller.
        requested: DurabilityRequirement,
        /// Strongest behavior qualified for the adapter and host profile.
        available: PublicationGuarantee,
    },
    /// A live owner currently excludes the requested operation.
    Busy,
    /// The requested generation is stale or otherwise conflicts with committed state.
    StateConflict,
    /// Stored bytes or metadata failed integrity validation.
    IntegrityFailure,
    /// Filesystem permissions denied the contained operation.
    AccessDenied,
    /// A configured or host storage capacity boundary prevented the operation.
    CapacityExhausted,
    /// Another storage I/O failure prevented the promised result.
    Io,
}

impl fmt::Display for SessionStorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedGuarantee {
                requested,
                available,
            } => write!(
                formatter,
                "requested {} storage but the qualified adapter provides only {} publication",
                requested.identifier(),
                available.identifier()
            ),
            Self::Busy => formatter.write_str("session storage is owned by another live operation"),
            Self::StateConflict => formatter.write_str("session storage generation conflicts"),
            Self::IntegrityFailure => {
                formatter.write_str("session storage failed integrity validation")
            }
            Self::AccessDenied => formatter.write_str("session storage access was denied"),
            Self::CapacityExhausted => formatter.write_str("session storage capacity is exhausted"),
            Self::Io => formatter.write_str("session storage I/O failed"),
        }
    }
}

impl Error for SessionStorageError {}

/// Port that owns contained session metadata and immutable generation publication.
pub trait SessionStore: Send + Sync {
    /// Returns the behavior qualified for this adapter and host profile.
    fn capabilities(&self) -> StorageCapabilities;

    /// Initializes private storage after the application has authorized its guarantee.
    fn initialize(
        &self,
        request: AuthorizedSessionStorageInitialization,
    ) -> impl Future<Output = Result<StorageGeneration, SessionStorageError>> + Send;
}

/// Application boundary that rejects unsupported durability before storage mutation.
pub struct InitializeSessionStorage<S> {
    store: S,
}

impl<S> InitializeSessionStorage<S>
where
    S: SessionStore,
{
    /// Creates the use case with its session-store dependency.
    pub const fn new(store: S) -> Self {
        Self { store }
    }

    /// Initializes storage without allowing the adapter to downgrade durability.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStorageError::UnsupportedGuarantee`] before calling the
    /// mutating port, or preserves the adapter's typed storage failure.
    pub async fn execute(
        &self,
        request: InitializeSessionStorageRequest,
    ) -> Result<InitializedSessionStorage, SessionStorageError> {
        let capabilities = self.store.capabilities();
        if !capabilities.supports(request.durability) {
            return Err(SessionStorageError::UnsupportedGuarantee {
                requested: request.durability,
                available: capabilities.publication(),
            });
        }

        let authorized = AuthorizedSessionStorageInitialization {
            session_id: request.session_id.clone(),
            operation_id: request.operation_id.clone(),
            durability: request.durability,
        };
        let generation = self.store.initialize(authorized).await?;

        Ok(InitializedSessionStorage {
            session_id: request.session_id,
            operation_id: request.operation_id,
            generation,
            publication: request.durability.required_guarantee(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        future::{Future, ready},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use super::{
        AuthorizedSessionStorageInitialization, InitializeSessionStorage,
        InitializeSessionStorageRequest, SessionStorageError, SessionStore, StorageCapabilities,
    };
    use vsift_domain::{
        DurabilityRequirement, OperationId, PublicationGuarantee, SessionId, StorageGeneration,
    };

    type TestResult = Result<(), Box<dyn Error>>;

    struct FakeSessionStore {
        capabilities: StorageCapabilities,
        initialize_calls: Arc<AtomicUsize>,
        result: Result<StorageGeneration, SessionStorageError>,
    }

    impl SessionStore for FakeSessionStore {
        fn capabilities(&self) -> StorageCapabilities {
            self.capabilities
        }

        fn initialize(
            &self,
            _request: AuthorizedSessionStorageInitialization,
        ) -> impl Future<Output = Result<StorageGeneration, SessionStorageError>> + Send {
            self.initialize_calls.fetch_add(1, Ordering::SeqCst);
            ready(self.result)
        }
    }

    fn request(
        durability: DurabilityRequirement,
    ) -> Result<InitializeSessionStorageRequest, vsift_domain::IdentifierError> {
        Ok(InitializeSessionStorageRequest::new(
            SessionId::parse("ses_0123456789abcdef")?,
            OperationId::parse("op_0123456789abcdef")?,
            durability,
        ))
    }

    #[tokio::test]
    async fn durable_request_fails_before_the_mutating_port_is_called() -> TestResult {
        let initialize_calls = Arc::new(AtomicUsize::new(0));
        let use_case = InitializeSessionStorage::new(FakeSessionStore {
            capabilities: StorageCapabilities::new(PublicationGuarantee::ProcessCrashConsistent),
            initialize_calls: Arc::clone(&initialize_calls),
            result: Ok(StorageGeneration::INITIAL),
        });

        let result = use_case
            .execute(request(DurabilityRequirement::Durable)?)
            .await;

        assert_eq!(
            result,
            Err(SessionStorageError::UnsupportedGuarantee {
                requested: DurabilityRequirement::Durable,
                available: PublicationGuarantee::ProcessCrashConsistent,
            })
        );
        assert_eq!(initialize_calls.load(Ordering::SeqCst), 0);
        Ok(())
    }

    #[tokio::test]
    async fn ephemeral_request_reaches_the_store_and_reports_effective_guarantee() -> TestResult {
        let initialize_calls = Arc::new(AtomicUsize::new(0));
        let use_case = InitializeSessionStorage::new(FakeSessionStore {
            capabilities: StorageCapabilities::new(PublicationGuarantee::ProcessCrashConsistent),
            initialize_calls: Arc::clone(&initialize_calls),
            result: Ok(StorageGeneration::INITIAL),
        });

        let initialized = use_case
            .execute(request(DurabilityRequirement::Ephemeral)?)
            .await?;

        assert_eq!(initialize_calls.load(Ordering::SeqCst), 1);
        assert_eq!(initialized.generation(), StorageGeneration::INITIAL);
        assert_eq!(
            initialized.publication(),
            PublicationGuarantee::ProcessCrashConsistent
        );
        Ok(())
    }

    #[tokio::test]
    async fn typed_adapter_failure_is_preserved() -> TestResult {
        let use_case = InitializeSessionStorage::new(FakeSessionStore {
            capabilities: StorageCapabilities::new(PublicationGuarantee::ProcessCrashConsistent),
            initialize_calls: Arc::new(AtomicUsize::new(0)),
            result: Err(SessionStorageError::AccessDenied),
        });

        let result = use_case
            .execute(request(DurabilityRequirement::Ephemeral)?)
            .await;

        assert_eq!(result, Err(SessionStorageError::AccessDenied));
        Ok(())
    }
}
