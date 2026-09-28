//! A session store that stages one source a worker request named inside the
//! operator's input root (P11 PR 3, ADR 0021 section 9).
//!
//! The application's open-session use case asks its port to stage "the
//! selected source" by path. A worker request's source is not a path the
//! engine may reopen: it was opened once, component by component and
//! following no link, by [`crate::InputRoot::open_file`]. This adapter holds
//! that opened file and stages it when the use case asks, so nothing outside
//! the root can be read however the root changes afterwards. Every other
//! port operation is the store's own.

use std::{future::Future, path::Path, sync::Mutex};

use vsift_application::{
    AuthorizedSessionGenerationPublication, AuthorizedSessionStorageInitialization,
    ForegroundSessionPort, OpenSessionError, SessionStorageError, SessionStore, StageCancellation,
    StorageCapabilities,
};
use vsift_domain::{
    OperationId, SessionId, SessionLifetimePolicy, StorageGeneration, TranscriptRevision,
};

use crate::{
    ContainedFile, FilesystemAdmissionPermit, FilesystemSessionStore, SessionRegistration,
    SourceSnapshot, source_snapshot::open_session_error,
};

/// A [`FilesystemSessionStore`] with one contained source to stage.
pub struct ContainedSourceStore {
    store: FilesystemSessionStore,
    source: Mutex<Option<(ContainedFile, Option<FilesystemAdmissionPermit>)>>,
}

impl ContainedSourceStore {
    /// The store, and the source its next staging copies: under
    /// `admission` when the caller already holds the copy's weight, else
    /// under an admission the staging takes.
    #[must_use]
    pub const fn new(
        store: FilesystemSessionStore,
        source: ContainedFile,
        admission: Option<FilesystemAdmissionPermit>,
    ) -> Self {
        Self {
            store,
            source: Mutex::new(Some((source, admission))),
        }
    }
}

impl SessionStore for ContainedSourceStore {
    fn capabilities(&self) -> StorageCapabilities {
        self.store.capabilities()
    }

    fn initialize(
        &self,
        request: AuthorizedSessionStorageInitialization,
    ) -> impl Future<Output = Result<StorageGeneration, SessionStorageError>> + Send {
        self.store.initialize(request)
    }

    fn publish(
        &self,
        request: AuthorizedSessionGenerationPublication,
    ) -> impl Future<Output = Result<StorageGeneration, SessionStorageError>> + Send {
        self.store.publish(request)
    }
}

impl ForegroundSessionPort for ContainedSourceStore {
    type Registration = SessionRegistration;
    type Snapshot = SourceSnapshot;

    fn register(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        now_unix_seconds: u64,
    ) -> Result<Self::Registration, OpenSessionError> {
        ForegroundSessionPort::register(&self.store, session_id, operation_id, now_unix_seconds)
    }

    fn lifetime_policy(&self) -> SessionLifetimePolicy {
        self.store.lifetime_policy()
    }

    /// Stages the held source; the path the use case passes is not used.
    /// The source is staged at most once: an open stages one source.
    fn stage_source(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        _source: &Path,
        cancellation: &dyn StageCancellation,
    ) -> Result<Self::Snapshot, OpenSessionError> {
        let (source, admission) = self
            .source
            .lock()
            .map_err(|_| OpenSessionError::SourceIo)?
            .take()
            .ok_or(OpenSessionError::SourceIo)?;
        match admission {
            Some(admission) => SourceSnapshot::stage_contained_admitted(
                &self.store,
                session_id,
                operation_id,
                source,
                admission,
                cancellation,
            ),
            None => SourceSnapshot::stage_contained(
                &self.store,
                session_id,
                operation_id,
                source,
                cancellation,
            ),
        }
        .map_err(open_session_error)
    }

    fn activate(
        &self,
        snapshot: &Self::Snapshot,
        operation_id: &OperationId,
        expected_generation: StorageGeneration,
        now_unix_seconds: u64,
    ) -> Result<StorageGeneration, OpenSessionError> {
        ForegroundSessionPort::activate(
            &self.store,
            snapshot,
            operation_id,
            expected_generation,
            now_unix_seconds,
        )
    }

    fn activate_with_transcript(
        &self,
        snapshot: &Self::Snapshot,
        operation_id: &OperationId,
        expected_generation: StorageGeneration,
        now_unix_seconds: u64,
        transcript: &TranscriptRevision,
    ) -> Result<StorageGeneration, OpenSessionError> {
        ForegroundSessionPort::activate_with_transcript(
            &self.store,
            snapshot,
            operation_id,
            expected_generation,
            now_unix_seconds,
            transcript,
        )
    }
}
