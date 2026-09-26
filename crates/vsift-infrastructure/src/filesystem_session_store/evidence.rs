//! Committing and reading evidence (P09): artifacts, evidence records and verified media paths.

use std::{collections::BTreeSet, path::PathBuf};

use cap_fs_ext::DirExt;
use vsift_application::{PublishSessionGenerationRequest, SessionStorageError};
use vsift_domain::{
    EvidenceMediaKind, OperationId, SessionArtifactKind, SessionId, SessionPhase, StorageGeneration,
};

use super::{
    ARTIFACTS_DIRECTORY, ATTEMPTS_DIRECTORY, COORDINATION_DIRECTORY, EvidenceInventory,
    EvidenceMediaFile, FilesystemSessionStore, LifecycleUpdate, SESSIONS_DIRECTORY, StoredArtifact,
    StoredArtifactKind,
    chain::read_committed_manifest,
    commit::CommitHooks,
    hash_bounded, map_lock_error, map_storage_io, open_regular_file, open_session_lock,
    publication::{ArtifactInstaller, evidence_artifact_count, publish_generation_while_locked},
    reads::read_evidence_artifact,
    root::acquire_admission,
    sha256_hex,
};
use crate::{VerifiedSourceIdentity, file_lock::HeldFileLock};

/// The inputs of one evidence commit besides its session and generation.
#[derive(Clone, Copy)]
pub(super) struct EvidenceFiles<'a> {
    /// The call's new media files.
    pub(super) media: &'a [EvidenceMediaFile<'a>],
    /// The call's encoded evidence record.
    pub(super) record: &'a [u8],
    /// The source identity the call verified with a full hash, if it did.
    pub(super) verified_identity: Option<&'a VerifiedSourceIdentity>,
    /// The commit time.
    pub(super) now_unix_seconds: u64,
}

impl FilesystemSessionStore {
    /// Publishes one bounded P04 media result as immutable session evidence.
    ///
    /// The artifact is installed by digest before its manifest generation commits.
    /// A failed publication can leave an unreferenced owned file, which is never
    /// reported as evidence and is removed with the disposable session.
    ///
    /// # Errors
    ///
    /// Rejects closed/expired sessions, stale generations, invalid byte budgets,
    /// contention, or a changed content-addressed artifact.
    pub fn publish_artifact(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        expected_generation: StorageGeneration,
        kind: SessionArtifactKind,
        bytes: &[u8],
        now_unix_seconds: u64,
    ) -> Result<StorageGeneration, SessionStorageError> {
        let max = StoredArtifactKind::from_domain(kind).max_bytes();
        if bytes.is_empty() || bytes.len() > max {
            return Err(SessionStorageError::CapacityExhausted);
        }
        self.revalidate_root()?;
        let _admission = acquire_admission(&self.root, self.admission_capacity, 1)?;
        let coordination = self
            .root
            .open_dir_nofollow(COORDINATION_DIRECTORY)
            .map_err(map_storage_io)?;
        let _lifetime =
            HeldFileLock::try_shared(open_session_lock(&coordination, session_id, "lifetime")?)
                .map_err(map_lock_error)?;
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?;
        let session = sessions
            .open_dir_nofollow(session_id.as_str())
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let committed = read_committed_manifest(&session, session_id, self.chain_check())?;
        let record = committed
            .manifest
            .lifecycle
            .ok_or(SessionStorageError::StateConflict)?;
        let status = record.to_status(session_id.clone(), committed.manifest.generation)?;
        if status.phase() != SessionPhase::Open || status.lifetime().expired(now_unix_seconds) {
            return Err(SessionStorageError::StateConflict);
        }
        let digest = sha256_hex(bytes);
        let name = format!("artifact-{digest}.{}", kind.extension());
        let artifacts = session
            .open_dir_nofollow(ARTIFACTS_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let attempts = session
            .open_dir_nofollow(ATTEMPTS_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let hooks = CommitHooks::new();
        let durability = committed.manifest.durability;
        let installer = ArtifactInstaller {
            commit: hooks.commit(durability),
            artifacts: &artifacts,
            attempts: &attempts,
            operation: operation_id,
            listed: &record.artifacts,
        };
        installer.install(&name, bytes, &digest)?;
        installer.finish()?;
        let _writer =
            HeldFileLock::try_exclusive(open_session_lock(&coordination, session_id, "writer")?)
                .map_err(map_lock_error)?;
        // The session's own durability: its commit protocol was fixed when
        // it was initialized (ADR 0020).
        let request = PublishSessionGenerationRequest::new(
            session_id.clone(),
            operation_id.clone(),
            expected_generation,
            durability.requirement(),
        );
        publish_generation_while_locked(
            &self.root,
            &request,
            LifecycleUpdate::AddArtifact {
                artifact: StoredArtifact {
                    kind: StoredArtifactKind::from_domain(kind),
                    name,
                    sha256: digest,
                    bytes: u64::try_from(bytes.len())
                        .map_err(|_| SessionStorageError::CapacityExhausted)?,
                },
                now: now_unix_seconds,
            },
            &hooks,
            Some(&self.verified_heads),
        )
    }

    /// Commits one evidence call (P09, ADR 0019): its new media files and its
    /// evidence record in one generation, and the source identity the call
    /// verified with a full hash, if it did (D1).
    ///
    /// Every file is installed by digest before the generation commits. A
    /// file the session already holds under the same name, kind, size and
    /// digest is kept rather than rejected, so two calls that extracted the
    /// same frame share one file; the same name with another kind or size is
    /// an integrity failure. The generation is checked against the evidence
    /// sub-budget ([`crate::MAX_EVIDENCE_ARTIFACTS`]), the 256 artifact slots
    /// and the 10 GiB bound.
    ///
    /// # Errors
    ///
    /// Rejects closed/expired sessions and stale generations
    /// ([`SessionStorageError::StateConflict`]), a file over its kind's bound
    /// or a session over its budgets
    /// ([`SessionStorageError::CapacityExhausted`]), contention, and a
    /// conflicting existing file ([`SessionStorageError::IntegrityFailure`]).
    #[allow(
        clippy::too_many_arguments,
        reason = "One generation's inputs; the store's other commits take the same"
    )]
    pub fn publish_evidence(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        expected_generation: StorageGeneration,
        media: &[EvidenceMediaFile<'_>],
        record: &[u8],
        verified_identity: Option<&VerifiedSourceIdentity>,
        now_unix_seconds: u64,
    ) -> Result<StorageGeneration, SessionStorageError> {
        self.commit_evidence(
            session_id,
            operation_id,
            expected_generation,
            &EvidenceFiles {
                media,
                record,
                verified_identity,
                now_unix_seconds,
            },
            &CommitHooks::new(),
        )
    }

    /// [`Self::publish_evidence`] under explicit commit hooks, so the unit
    /// tests can trace or interrupt it.
    pub(super) fn commit_evidence(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        expected_generation: StorageGeneration,
        files: &EvidenceFiles<'_>,
        hooks: &CommitHooks<'_>,
    ) -> Result<StorageGeneration, SessionStorageError> {
        let EvidenceFiles {
            media,
            record,
            verified_identity,
            now_unix_seconds,
        } = *files;
        let mut files: Vec<(StoredArtifactKind, &[u8])> = media
            .iter()
            .map(|file| (StoredArtifactKind::from_media(file.kind), file.bytes))
            .collect();
        files.push((StoredArtifactKind::EvidenceRecord, record));
        let mut artifacts = Vec::with_capacity(files.len());
        for (kind, bytes) in &files {
            if bytes.is_empty() || bytes.len() > kind.max_bytes() {
                return Err(SessionStorageError::CapacityExhausted);
            }
            let digest = sha256_hex(bytes);
            artifacts.push(StoredArtifact {
                kind: *kind,
                name: format!("artifact-{digest}.{}", kind.extension()),
                sha256: digest,
                bytes: u64::try_from(bytes.len())
                    .map_err(|_| SessionStorageError::CapacityExhausted)?,
            });
        }
        self.revalidate_root()?;
        let _admission = acquire_admission(&self.root, self.admission_capacity, 1)?;
        let coordination = self
            .root
            .open_dir_nofollow(COORDINATION_DIRECTORY)
            .map_err(map_storage_io)?;
        let _lifetime =
            HeldFileLock::try_shared(open_session_lock(&coordination, session_id, "lifetime")?)
                .map_err(map_lock_error)?;
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?;
        let session = sessions
            .open_dir_nofollow(session_id.as_str())
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let committed = read_committed_manifest(&session, session_id, self.chain_check())?;
        let record_state = committed
            .manifest
            .lifecycle
            .ok_or(SessionStorageError::StateConflict)?;
        let status = record_state.to_status(session_id.clone(), committed.manifest.generation)?;
        if status.phase() != SessionPhase::Open || status.lifetime().expired(now_unix_seconds) {
            return Err(SessionStorageError::StateConflict);
        }
        let directory = session
            .open_dir_nofollow(ARTIFACTS_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let attempts = session
            .open_dir_nofollow(ATTEMPTS_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let durability = committed.manifest.durability;
        let installer = ArtifactInstaller {
            commit: hooks.commit(durability),
            artifacts: &directory,
            attempts: &attempts,
            operation: operation_id,
            listed: &record_state.artifacts,
        };
        for ((_, bytes), artifact) in files.iter().zip(&artifacts) {
            installer.install(&artifact.name, bytes, &artifact.sha256)?;
        }
        installer.finish()?;
        let _writer =
            HeldFileLock::try_exclusive(open_session_lock(&coordination, session_id, "writer")?)
                .map_err(map_lock_error)?;
        // The session's own durability (ADR 0020).
        let request = PublishSessionGenerationRequest::new(
            session_id.clone(),
            operation_id.clone(),
            expected_generation,
            durability.requirement(),
        );
        publish_generation_while_locked(
            &self.root,
            &request,
            LifecycleUpdate::AddEvidence {
                artifacts,
                verified_identity: verified_identity.map(|identity| identity.as_str().to_owned()),
                now: now_unix_seconds,
            },
            hooks,
            Some(&self.verified_heads),
        )
    }

    /// Reads every committed evidence record of an open session, with what
    /// the session may still take.
    ///
    /// Each record is read under a shared lifetime hold, its size and SHA-256
    /// are checked against the committed manifest, and it is decoded strictly
    /// (structure, request key and item identities re-derived) and must
    /// describe the session's source. The media files the records name are
    /// not read here; a caller that returns them verifies each one with
    /// [`Self::verified_artifact_path`].
    ///
    /// # Errors
    ///
    /// Closed or expired sessions conflict; a changed, oversized or invalid
    /// record is an integrity failure, and a newer record version is
    /// unsupported.
    pub fn read_evidence_records(
        &self,
        session_id: &SessionId,
        now_unix_seconds: u64,
    ) -> Result<EvidenceInventory, SessionStorageError> {
        let _hold = self.acquire_read(session_id)?;
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?;
        let session = sessions
            .open_dir_nofollow(session_id.as_str())
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let committed = read_committed_manifest(&session, session_id, self.chain_check())?;
        let lifecycle = committed
            .manifest
            .lifecycle
            .ok_or(SessionStorageError::StateConflict)?;
        let status = lifecycle.to_status(session_id.clone(), committed.manifest.generation)?;
        if status.phase() != SessionPhase::Open || status.lifetime().expired(now_unix_seconds) {
            return Err(SessionStorageError::StateConflict);
        }
        let directory = session
            .open_dir_nofollow(ARTIFACTS_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let mut records = Vec::new();
        let mut known_media = BTreeSet::new();
        for artifact in &lifecycle.artifacts {
            match artifact.kind {
                StoredArtifactKind::EvidenceRecord => records.push(read_evidence_artifact(
                    &directory,
                    artifact,
                    session_id,
                    status.source_id(),
                )?),
                StoredArtifactKind::FramePng | StoredArtifactKind::AudioWav => {
                    known_media.insert(artifact.sha256.clone());
                }
                StoredArtifactKind::AudioPcm
                | StoredArtifactKind::TranscriptRecord
                | StoredArtifactKind::VisualIndexRecord => {}
            }
        }
        Ok(EvidenceInventory {
            records,
            evidence_artifacts: evidence_artifact_count(&lifecycle.artifacts),
            status,
            known_media,
        })
    }

    /// Verifies one committed evidence media file of an open session and
    /// returns its absolute path (ADR 0019 D2).
    ///
    /// The file must be listed in the committed manifest with exactly this
    /// kind, size and SHA-256, and its bytes are hashed again before the path
    /// is returned (INV-02). The path stays valid while the session exists;
    /// it is for the result that delivers the file, never for a record.
    ///
    /// # Errors
    ///
    /// Closed or expired sessions conflict; a file the manifest does not
    /// list, or whose bytes differ, is an integrity failure.
    pub fn verified_artifact_path(
        &self,
        session_id: &SessionId,
        kind: EvidenceMediaKind,
        sha256: &str,
        bytes: u64,
        now_unix_seconds: u64,
    ) -> Result<PathBuf, SessionStorageError> {
        let _hold = self.acquire_read(session_id)?;
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?;
        let session = sessions
            .open_dir_nofollow(session_id.as_str())
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let committed = read_committed_manifest(&session, session_id, self.chain_check())?;
        let lifecycle = committed
            .manifest
            .lifecycle
            .ok_or(SessionStorageError::StateConflict)?;
        let status = lifecycle.to_status(session_id.clone(), committed.manifest.generation)?;
        if status.phase() != SessionPhase::Open || status.lifetime().expired(now_unix_seconds) {
            return Err(SessionStorageError::StateConflict);
        }
        let kind = StoredArtifactKind::from_media(kind);
        let artifact = lifecycle
            .artifacts
            .iter()
            .find(|artifact| {
                artifact.kind == kind && artifact.sha256 == sha256 && artifact.bytes == bytes
            })
            .ok_or(SessionStorageError::IntegrityFailure)?;
        let directory = session
            .open_dir_nofollow(ARTIFACTS_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let file = open_regular_file(&directory, &artifact.name, false)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        if hash_bounded(file, artifact.bytes)? != artifact.sha256 {
            return Err(SessionStorageError::IntegrityFailure);
        }
        Ok(self
            .root_path
            .join(SESSIONS_DIRECTORY)
            .join(session_id.as_str())
            .join(ARTIFACTS_DIRECTORY)
            .join(&artifact.name))
    }
}
