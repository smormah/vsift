//! Session lifecycle generations: activation, status, renewal and close.

use cap_fs_ext::DirExt;
use vsift_application::{PublishSessionGenerationRequest, SessionStorageError};
use vsift_domain::{OperationId, SessionArtifactKind, SessionId, StorageGeneration};

use super::{
    FilesystemSessionStore, LifecycleUpdate, SESSIONS_DIRECTORY, SessionStatus, StoredArtifact,
    StoredArtifactKind,
    chain::read_committed_manifest,
    map_storage_io,
    publication::{install_content_addressed, publish_generation_with_update},
    sha256_hex,
};
use crate::SourceSnapshot;

impl FilesystemSessionStore {
    /// Binds a verified private source to an initialized ephemeral session.
    ///
    /// The snapshot's shared hold protects it from cleanup until the generation
    /// commits. Explicit durable publication still requires the application
    /// preflight and is not available through this desktop-only operation.
    ///
    /// # Errors
    ///
    /// Returns a typed source, conflict, capacity, or storage failure.
    pub fn activate_source(
        &self,
        snapshot: &SourceSnapshot,
        operation_id: &OperationId,
        expected_generation: StorageGeneration,
        now_unix_seconds: u64,
    ) -> Result<StorageGeneration, SessionStorageError> {
        snapshot
            .verify()
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        self.revalidate_root()?;
        let request = PublishSessionGenerationRequest::new(
            snapshot.session_id().clone(),
            operation_id.clone(),
            expected_generation,
            vsift_domain::DurabilityRequirement::Ephemeral,
        );
        publish_generation_with_update(
            &self.root,
            self.admission_capacity,
            &request,
            LifecycleUpdate::Activate {
                source_id: snapshot.id().as_str().to_owned(),
                source_name: snapshot.file_name().to_owned(),
                source_bytes: snapshot.bytes(),
                now: now_unix_seconds,
                artifacts: Vec::new(),
            },
            false,
        )
    }

    /// Binds a verified source and publishes one evidence artifact in the same
    /// generation.
    ///
    /// The artifact is installed by digest in the session's artifact directory
    /// first, under the snapshot's shared lifetime hold; the generation that
    /// opens the session then references it. If publication fails, the
    /// unreferenced file is never reported as evidence and is removed with the
    /// abandoned session.
    ///
    /// # Errors
    ///
    /// Returns a typed source, conflict, capacity, or storage failure.
    pub fn activate_source_with_artifact(
        &self,
        snapshot: &SourceSnapshot,
        operation_id: &OperationId,
        expected_generation: StorageGeneration,
        now_unix_seconds: u64,
        kind: SessionArtifactKind,
        bytes: &[u8],
    ) -> Result<StorageGeneration, SessionStorageError> {
        let kind = StoredArtifactKind::from_domain(kind);
        if bytes.is_empty() || bytes.len() > kind.max_bytes() {
            return Err(SessionStorageError::CapacityExhausted);
        }
        snapshot
            .verify()
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        self.revalidate_root()?;
        let digest = sha256_hex(bytes);
        let name = format!("artifact-{digest}.{}", kind.extension());
        install_content_addressed(snapshot.artifact_directory(), &name, bytes, &digest)?;
        let request = PublishSessionGenerationRequest::new(
            snapshot.session_id().clone(),
            operation_id.clone(),
            expected_generation,
            vsift_domain::DurabilityRequirement::Ephemeral,
        );
        publish_generation_with_update(
            &self.root,
            self.admission_capacity,
            &request,
            LifecycleUpdate::Activate {
                source_id: snapshot.id().as_str().to_owned(),
                source_name: snapshot.file_name().to_owned(),
                source_bytes: snapshot.bytes(),
                now: now_unix_seconds,
                artifacts: vec![StoredArtifact {
                    kind,
                    name,
                    sha256: digest,
                    bytes: u64::try_from(bytes.len())
                        .map_err(|_| SessionStorageError::CapacityExhausted)?,
                }],
            },
            false,
        )
    }
}

impl FilesystemSessionStore {
    /// Reads one committed lifecycle after verifying the root and manifest chain.
    ///
    /// # Errors
    ///
    /// Orphaned initializations and corrupt lifecycle metadata fail closed.
    pub fn session_status(
        &self,
        session_id: &SessionId,
    ) -> Result<SessionStatus, SessionStorageError> {
        let _hold = self.acquire_read(session_id)?;
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?;
        let session = sessions
            .open_dir_nofollow(session_id.as_str())
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let committed = read_committed_manifest(&session, session_id)?;
        let lifecycle = committed
            .manifest
            .lifecycle
            .ok_or(SessionStorageError::StateConflict)?;
        lifecycle.to_status(session_id.clone(), committed.manifest.generation)
    }

    /// Reads a listed registration without hiding committed integrity failures.
    ///
    /// A marker without an initialized/activated session is reported as pending;
    /// corrupt committed metadata remains a typed failure for the listing caller.
    ///
    /// # Errors
    ///
    /// Returns a typed permission, contention, version, or integrity failure.
    pub fn indexed_session_status(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionStatus>, SessionStorageError> {
        self.revalidate_root()?;
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?;
        if !sessions
            .try_exists(session_id.as_str())
            .map_err(map_storage_io)?
        {
            return Ok(None);
        }
        match self.session_status(session_id) {
            Ok(status) => Ok(Some(status)),
            Err(SessionStorageError::StateConflict) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Extends an unexpired open session within its original seven-day cap.
    ///
    /// # Errors
    ///
    /// Active holds return busy; expired, closed, or stale generations conflict.
    pub fn renew_session(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        expected_generation: StorageGeneration,
        now_unix_seconds: u64,
    ) -> Result<StorageGeneration, SessionStorageError> {
        self.revalidate_root()?;
        let request = PublishSessionGenerationRequest::new(
            session_id.clone(),
            operation_id.clone(),
            expected_generation,
            vsift_domain::DurabilityRequirement::Ephemeral,
        );
        publish_generation_with_update(
            &self.root,
            self.admission_capacity,
            &request,
            LifecycleUpdate::Renew {
                now: now_unix_seconds,
            },
            true,
        )
    }

    /// Closes a session after claiming its exclusive lifetime lock.
    ///
    /// # Errors
    ///
    /// Active work returns busy; stale or already closed sessions conflict.
    pub fn close_session(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        expected_generation: StorageGeneration,
    ) -> Result<StorageGeneration, SessionStorageError> {
        self.revalidate_root()?;
        let request = PublishSessionGenerationRequest::new(
            session_id.clone(),
            operation_id.clone(),
            expected_generation,
            vsift_domain::DurabilityRequirement::Ephemeral,
        );
        publish_generation_with_update(
            &self.root,
            self.admission_capacity,
            &request,
            LifecycleUpdate::Close,
            true,
        )
    }
}
