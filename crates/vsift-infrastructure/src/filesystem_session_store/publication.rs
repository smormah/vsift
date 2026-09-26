//! Immutable generation publication: lifecycle updates, staging, installation and commit.
//!
//! The ordering of every step, and what a durable session adds to it, is
//! described in [`super::commit`].

use std::io::{self, Write};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use vsift_application::{
    AuthorizedSessionGenerationPublication, AuthorizedSessionStorageInitialization,
    PublishSessionGenerationRequest, SessionStorageError, SessionStore, StorageCapabilities,
};
use vsift_domain::{DurabilityRequirement, OperationId, SessionLifetime, StorageGeneration};

use super::{
    ARTIFACTS_DIRECTORY, ATTEMPTS_DIRECTORY, CHAIN_CHECKPOINT_FILE, COORDINATION_DIRECTORY,
    CURRENT_FILE, ChainCheck, ChainCheckpoint, CommitPointer, CommittedManifest,
    FilesystemSessionStore, GENERATIONS_DIRECTORY, GenerationManifest, LifecycleUpdate,
    MAX_GENERATIONS_PER_SESSION, MAX_METADATA_BYTES, MAX_SESSION_ARTIFACT_BYTES,
    MAX_SESSION_ARTIFACTS, SESSIONS_DIRECTORY, STORAGE_SCHEMA_VERSION, StoredArtifact,
    StoredArtifactKind, StoredLifecycle, StoredSessionPhase, VerifiedHeadCache,
    chain::read_committed_manifest,
    commit::{Commit, CommitHooks, DirRole, FsOp},
    hash_bounded,
    initialization::initialize_session,
    map_lock_error, map_open_error, map_storage_io, open_regular_file, open_session_lock,
    read_bounded,
    root::{acquire_admission, validate_platform_root_permissions},
    sha256_hex,
    stored::{validate_artifact_record, validate_source_record},
};
use crate::{fault_point::FaultPoint, file_lock::HeldFileLock};

impl FilesystemSessionStore {
    /// Publishes the next immutable manifest with optimistic generation fencing.
    ///
    /// Lock ordering is admission, shared lifetime, then metadata writer. No method
    /// waits while holding the writer lock. A repeated compatible operation returns
    /// its already committed generation; a stale token or conflicting operation is
    /// rejected.
    ///
    /// # Errors
    ///
    /// Requests the store's capabilities cannot honour fail before admission or
    /// filesystem mutation. Other typed failures preserve the previously
    /// committed generation.
    #[cfg(test)]
    pub(super) async fn publish_generation(
        &self,
        request: PublishSessionGenerationRequest,
    ) -> Result<StorageGeneration, SessionStorageError> {
        if !self.capabilities.supports(request.durability()) {
            return Err(SessionStorageError::UnsupportedGuarantee {
                requested: request.durability(),
                available: self.capabilities.publication(),
            });
        }
        let root = self.root.try_clone().map_err(map_storage_io)?;
        let root_path = self.root_path.clone();
        let capacity = self.admission_capacity;
        tokio::task::spawn_blocking(move || {
            validate_platform_root_permissions(&root_path, &root).map_err(map_open_error)?;
            publish_generation(&root, capacity, &request, &CommitHooks::new())
        })
        .await
        .map_err(|_| SessionStorageError::Io)?
    }
}

impl SessionStore for FilesystemSessionStore {
    fn capabilities(&self) -> StorageCapabilities {
        self.capabilities
    }

    fn initialize(
        &self,
        request: AuthorizedSessionStorageInitialization,
    ) -> impl Future<Output = Result<StorageGeneration, SessionStorageError>> + Send {
        let root = self.root.try_clone().map_err(map_storage_io);
        let root_path = self.root_path.clone();
        async move {
            let root = root?;
            tokio::task::spawn_blocking(move || {
                validate_platform_root_permissions(&root_path, &root).map_err(map_open_error)?;
                initialize_session(&root, &request, &CommitHooks::new())
            })
            .await
            .map_err(|_| SessionStorageError::Io)?
        }
    }

    fn publish(
        &self,
        request: AuthorizedSessionGenerationPublication,
    ) -> impl Future<Output = Result<StorageGeneration, SessionStorageError>> + Send {
        let root = self.root.try_clone().map_err(map_storage_io);
        let root_path = self.root_path.clone();
        let capacity = self.admission_capacity;
        let request = PublishSessionGenerationRequest::new(
            request.session_id().clone(),
            request.operation_id().clone(),
            request.expected_generation(),
            request.durability(),
        );
        async move {
            let root = root?;
            tokio::task::spawn_blocking(move || {
                validate_platform_root_permissions(&root_path, &root).map_err(map_open_error)?;
                publish_generation(&root, capacity, &request, &CommitHooks::new())
            })
            .await
            .map_err(|_| SessionStorageError::Io)?
        }
    }
}

pub(super) fn publish_generation(
    root: &Dir,
    capacity: u16,
    request: &PublishSessionGenerationRequest,
    hooks: &CommitHooks<'_>,
) -> Result<StorageGeneration, SessionStorageError> {
    let _admission = acquire_admission(root, capacity, 1)?;
    let coordination = root
        .open_dir_nofollow(COORDINATION_DIRECTORY)
        .map_err(map_storage_io)?;
    let lifetime = HeldFileLock::try_shared(open_session_lock(
        &coordination,
        request.session_id(),
        "lifetime",
    )?)
    .map_err(map_lock_error)?;
    let writer = HeldFileLock::try_exclusive(open_session_lock(
        &coordination,
        request.session_id(),
        "writer",
    )?)
    .map_err(map_lock_error)?;

    let result = publish_generation_while_locked(root, request, LifecycleUpdate::Keep, hooks, None);
    drop(writer);
    drop(lifetime);
    result
}

/// The lifetime hold a lifecycle generation takes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LifetimeHold {
    /// Shared with readers: activation.
    Shared,
    /// Exclusive of every reader and writer: renewal and close.
    Exclusive,
}

pub(super) fn publish_generation_with_update(
    root: &Dir,
    capacity: u16,
    request: &PublishSessionGenerationRequest,
    update: LifecycleUpdate,
    lifetime_hold: LifetimeHold,
    hooks: &CommitHooks<'_>,
    cache: Option<&VerifiedHeadCache>,
) -> Result<StorageGeneration, SessionStorageError> {
    let _admission = acquire_admission(root, capacity, 1)?;
    let coordination = root
        .open_dir_nofollow(COORDINATION_DIRECTORY)
        .map_err(map_storage_io)?;
    let lifetime_file = open_session_lock(&coordination, request.session_id(), "lifetime")?;
    let lifetime = match lifetime_hold {
        LifetimeHold::Exclusive => HeldFileLock::try_exclusive(lifetime_file),
        LifetimeHold::Shared => HeldFileLock::try_shared(lifetime_file),
    }
    .map_err(map_lock_error)?;
    let writer = HeldFileLock::try_exclusive(open_session_lock(
        &coordination,
        request.session_id(),
        "writer",
    )?)
    .map_err(map_lock_error)?;
    let result = publish_generation_while_locked(root, request, update, hooks, cache);
    drop(writer);
    drop(lifetime);
    result
}

pub(super) fn update_lifecycle(
    current: Option<StoredLifecycle>,
    update: LifecycleUpdate,
) -> Result<Option<StoredLifecycle>, SessionStorageError> {
    match update {
        LifecycleUpdate::Keep => Ok(current),
        LifecycleUpdate::Activate {
            source_id,
            source_name,
            source_bytes,
            now,
            artifacts,
        } => {
            if current.is_some() || source_bytes == 0 || source_bytes > crate::MAX_SOURCE_BYTES {
                return Err(SessionStorageError::StateConflict);
            }
            validate_source_record(&source_id, &source_name, source_bytes)?;
            if artifacts.len() > 256 {
                return Err(SessionStorageError::CapacityExhausted);
            }
            for (index, artifact) in artifacts.iter().enumerate() {
                validate_artifact_record(artifact)?;
                if artifacts
                    .iter()
                    .skip(index + 1)
                    .any(|later| later.name == artifact.name)
                {
                    return Err(SessionStorageError::CapacityExhausted);
                }
            }
            let lifetime =
                SessionLifetime::open(now).map_err(|_| SessionStorageError::StateConflict)?;
            Ok(Some(StoredLifecycle {
                phase: StoredSessionPhase::Open,
                opened_at_unix_seconds: lifetime.opened_at_unix_seconds(),
                expires_at_unix_seconds: lifetime.expires_at_unix_seconds(),
                source_id,
                source_name,
                source_bytes,
                artifacts,
                verified_source_identity: None,
            }))
        }
        LifecycleUpdate::Renew { now } => {
            let mut record = current.ok_or(SessionStorageError::StateConflict)?;
            if !matches!(record.phase, StoredSessionPhase::Open) {
                return Err(SessionStorageError::StateConflict);
            }
            let lifetime = record.validated_lifetime()?;
            let renewed = lifetime
                .renew(now)
                .map_err(|_| SessionStorageError::StateConflict)?;
            record.expires_at_unix_seconds = renewed.expires_at_unix_seconds();
            Ok(Some(record))
        }
        LifecycleUpdate::Close => {
            let mut record = current.ok_or(SessionStorageError::StateConflict)?;
            record.validated_lifetime()?;
            if !matches!(record.phase, StoredSessionPhase::Open) {
                return Err(SessionStorageError::StateConflict);
            }
            record.phase = StoredSessionPhase::Closed;
            Ok(Some(record))
        }
        LifecycleUpdate::AddArtifact { artifact, now } => {
            let mut record = current.ok_or(SessionStorageError::StateConflict)?;
            let lifetime = record.validated_lifetime()?;
            if !matches!(record.phase, StoredSessionPhase::Open) || lifetime.expired(now) {
                return Err(SessionStorageError::StateConflict);
            }
            validate_artifact_record(&artifact)?;
            if record.artifacts.len() >= 256
                || record
                    .artifacts
                    .iter()
                    .any(|existing| existing.name == artifact.name)
            {
                return Err(SessionStorageError::CapacityExhausted);
            }
            if sub_budget_full(&record.artifacts, artifact.kind) {
                return Err(SessionStorageError::CapacityExhausted);
            }
            let total = record
                .artifacts
                .iter()
                .try_fold(artifact.bytes, |sum, item| sum.checked_add(item.bytes))
                .ok_or(SessionStorageError::CapacityExhausted)?;
            if total > MAX_SESSION_ARTIFACT_BYTES {
                return Err(SessionStorageError::CapacityExhausted);
            }
            record.artifacts.push(artifact);
            Ok(Some(record))
        }
        LifecycleUpdate::AddEvidence {
            artifacts,
            verified_identity,
            now,
        } => add_evidence(current, artifacts, verified_identity, now).map(Some),
    }
}

/// Adds one evidence call's artifacts and verified source identity.
pub(super) fn add_evidence(
    current: Option<StoredLifecycle>,
    artifacts: Vec<StoredArtifact>,
    verified_identity: Option<String>,
    now: u64,
) -> Result<StoredLifecycle, SessionStorageError> {
    let mut record = current.ok_or(SessionStorageError::StateConflict)?;
    let lifetime = record.validated_lifetime()?;
    if !matches!(record.phase, StoredSessionPhase::Open) || lifetime.expired(now) {
        return Err(SessionStorageError::StateConflict);
    }
    for artifact in artifacts {
        validate_artifact_record(&artifact)?;
        if !artifact.kind.is_evidence() {
            return Err(SessionStorageError::IntegrityFailure);
        }
        // Evidence media is content-addressed, so a file an earlier call
        // committed is kept rather than rejected; the same name with another
        // kind or size is a corrupt manifest.
        if let Some(existing) = record
            .artifacts
            .iter()
            .find(|existing| existing.name == artifact.name)
        {
            if existing.kind != artifact.kind
                || existing.sha256 != artifact.sha256
                || existing.bytes != artifact.bytes
            {
                return Err(SessionStorageError::IntegrityFailure);
            }
            continue;
        }
        record.artifacts.push(artifact);
    }
    if record.artifacts.len() > MAX_SESSION_ARTIFACTS
        || evidence_artifact_count(&record.artifacts) > crate::MAX_EVIDENCE_ARTIFACTS
    {
        return Err(SessionStorageError::CapacityExhausted);
    }
    let total = record
        .artifacts
        .iter()
        .try_fold(0_u64, |sum, item| sum.checked_add(item.bytes))
        .ok_or(SessionStorageError::CapacityExhausted)?;
    if total > MAX_SESSION_ARTIFACT_BYTES {
        return Err(SessionStorageError::CapacityExhausted);
    }
    if let Some(identity) = verified_identity {
        record.verified_source_identity = Some(identity);
    }
    Ok(record)
}

/// Whether the sub-budget an artifact of `kind` counts against is full.
///
/// Visual-index revisions and evidence (ADR 0019 D4) are bounded separately
/// so a runaway caller cannot fill the session's 256 artifact slots with
/// them.
pub(super) fn sub_budget_full(artifacts: &[StoredArtifact], kind: StoredArtifactKind) -> bool {
    if kind == StoredArtifactKind::VisualIndexRecord {
        artifacts
            .iter()
            .filter(|existing| existing.kind == StoredArtifactKind::VisualIndexRecord)
            .count()
            >= crate::MAX_VISUAL_INDEX_RECORDS
    } else {
        kind.is_evidence() && evidence_artifact_count(artifacts) >= crate::MAX_EVIDENCE_ARTIFACTS
    }
}

/// How many of a session's artifacts count against the evidence sub-budget.
pub(super) fn evidence_artifact_count(artifacts: &[StoredArtifact]) -> usize {
    artifacts
        .iter()
        .filter(|artifact| artifact.kind.is_evidence())
        .count()
}

/// Commits the next generation of a session whose writer lock is held.
///
/// The session's recorded durability, not the request's, chooses the
/// protocol ([`super::commit`]): an ephemeral session cannot be upgraded by a
/// durable request (a conflict), and a durable session commits durably even
/// for an ephemeral request. After the commit the chain checkpoint advances to
/// the new head (#164); failing to write it never fails the commit, because
/// the checkpoint only shortens later reads.
pub(super) fn publish_generation_while_locked(
    root: &Dir,
    request: &PublishSessionGenerationRequest,
    update: LifecycleUpdate,
    hooks: &CommitHooks<'_>,
    cache: Option<&VerifiedHeadCache>,
) -> Result<StorageGeneration, SessionStorageError> {
    let sessions = root
        .open_dir_nofollow(SESSIONS_DIRECTORY)
        .map_err(map_storage_io)?;
    let session = sessions
        .open_dir_nofollow(request.session_id().as_str())
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let current = read_committed_manifest(
        &session,
        request.session_id(),
        ChainCheck::Incremental(cache),
    )?;
    let commit = hooks.commit(current.manifest.durability);
    if request.durability() == DurabilityRequirement::Durable && !commit.durable() {
        return Err(SessionStorageError::StateConflict);
    }
    let expected = request.expected_generation().value();

    if current.manifest.generation != expected {
        if current.manifest.generation == expected.saturating_add(1)
            && current.manifest.operation_id == request.operation_id().as_str()
        {
            acknowledge_retry(commit, &session)?;
            return Ok(StorageGeneration::from_value(current.manifest.generation));
        }
        return Err(SessionStorageError::StateConflict);
    }
    if current.manifest.operation_id == request.operation_id().as_str() {
        return Err(SessionStorageError::StateConflict);
    }

    let next = request
        .expected_generation()
        .successor()
        .map_err(|_| SessionStorageError::CapacityExhausted)?;
    if next.value() >= MAX_GENERATIONS_PER_SESSION {
        return Err(SessionStorageError::CapacityExhausted);
    }
    let activates = matches!(update, LifecycleUpdate::Activate { .. });
    let bytes = next_manifest(current, request, next, update)?;
    if activates && commit.durable() {
        // The source copy staged for activation must be reachable before
        // the generation that names it.
        let artifacts = session
            .open_dir_nofollow(ARTIFACTS_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        commit.sync_directory(&artifacts, DirRole::Artifacts)?;
    }
    let generations = session
        .open_dir_nofollow(GENERATIONS_DIRECTORY)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let attempts = session
        .open_dir_nofollow(ATTEMPTS_DIRECTORY)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let generation_name = format!("{}.json", next.value());
    let staged_manifest = format!(
        "{}.{}.manifest.tmp",
        request.operation_id().as_str(),
        next.value()
    );
    install_immutable_file(
        commit,
        &attempts,
        &staged_manifest,
        &generations,
        &generation_name,
        &bytes,
    )?;
    commit.sync_directory(&generations, DirRole::Generations)?;
    commit.reach(FaultPoint::ManifestDirectorySync)?;
    let pointer = CommitPointer {
        schema_version: STORAGE_SCHEMA_VERSION,
        generation: next.value(),
        manifest_sha256: sha256_hex(&bytes),
    };
    install_pointer(
        commit,
        &session,
        &attempts,
        request.operation_id(),
        &pointer,
    )?;

    let committed = read_committed_manifest(
        &session,
        request.session_id(),
        ChainCheck::Incremental(cache),
    )?;
    if committed.manifest.generation != next.value()
        || committed.manifest.operation_id != request.operation_id().as_str()
    {
        return Err(SessionStorageError::IntegrityFailure);
    }
    commit.record(|| FsOp::Committed);
    // Best effort by design: see the function's documentation.
    let _ = write_chain_checkpoint(
        commit,
        &session,
        &attempts,
        request.operation_id(),
        next.value(),
        &pointer.manifest_sha256,
    );
    Ok(next)
}

/// Encodes generation `next`: the current head's lifecycle with `update`
/// applied, linked to the head's digest and carrying its durability.
fn next_manifest(
    current: CommittedManifest,
    request: &PublishSessionGenerationRequest,
    next: StorageGeneration,
    update: LifecycleUpdate,
) -> Result<Vec<u8>, SessionStorageError> {
    let lifecycle = update_lifecycle(current.manifest.lifecycle, update)?;
    let manifest = GenerationManifest {
        schema_version: STORAGE_SCHEMA_VERSION,
        session_id: request.session_id().as_str().to_owned(),
        operation_id: request.operation_id().as_str().to_owned(),
        generation: next.value(),
        previous_manifest_sha256: Some(current.digest),
        durability: current.manifest.durability,
        lifecycle,
    };
    let bytes = serde_json::to_vec(&manifest).map_err(|_| SessionStorageError::Io)?;
    // A manifest that could not be read back would strand the session.
    if u64::try_from(bytes.len()).map_or(true, |size| size > MAX_METADATA_BYTES) {
        return Err(SessionStorageError::CapacityExhausted);
    }
    Ok(bytes)
}

/// Acknowledges a retry of an operation whose generation is already the head
/// (its first acknowledgement was lost).
///
/// A durable attempt may have failed after its renames and before its
/// directory syncs, so they are repeated before the generation is
/// acknowledged again.
fn acknowledge_retry(commit: Commit<'_>, session: &Dir) -> Result<(), SessionStorageError> {
    if commit.durable() {
        let generations = session
            .open_dir_nofollow(GENERATIONS_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        commit.sync_directory(&generations, DirRole::Generations)?;
        commit.sync_directory(session, DirRole::Session)?;
    }
    commit.record(|| FsOp::Committed);
    Ok(())
}

/// Stages the commit pointer, renames it to `current.json` and, for a durable
/// session, synchronises the session directory: the commit point.
fn install_pointer(
    commit: Commit<'_>,
    session: &Dir,
    attempts: &Dir,
    operation: &OperationId,
    pointer: &CommitPointer,
) -> Result<(), SessionStorageError> {
    let bytes = serde_json::to_vec(pointer).map_err(|_| SessionStorageError::Io)?;
    let staged = format!("{}.{}.pointer.tmp", operation.as_str(), pointer.generation);
    prepare_staged_file(
        commit,
        attempts,
        &staged,
        &bytes,
        Some(FaultPoint::PointerWrite),
        Some(FaultPoint::PointerFlush),
    )?;
    attempts
        .rename(&staged, session, CURRENT_FILE)
        .map_err(map_storage_io)?;
    commit.record(|| {
        FsOp::Rename(
            DirRole::Attempts,
            staged.clone(),
            DirRole::Session,
            CURRENT_FILE.to_owned(),
        )
    });
    commit.reach(FaultPoint::PointerRename)?;
    commit.sync_directory(session, DirRole::Session)?;
    commit.reach(FaultPoint::PointerDirectorySync)
}

/// Advances `chain-verified.json` to a head the writer has just committed.
///
/// The writer verified the chain from the previous head down to the previous
/// anchor before committing, and the new head links to the previous one, so
/// the new head's chain is verified in full. The file is staged, flushed and
/// renamed over the old checkpoint, so a reader sees the old or the new one.
fn write_chain_checkpoint(
    commit: Commit<'_>,
    session: &Dir,
    attempts: &Dir,
    operation: &OperationId,
    generation: u64,
    manifest_sha256: &str,
) -> Result<(), SessionStorageError> {
    let checkpoint = ChainCheckpoint {
        schema_version: STORAGE_SCHEMA_VERSION,
        generation,
        manifest_sha256: manifest_sha256.to_owned(),
    };
    let bytes = serde_json::to_vec(&checkpoint).map_err(|_| SessionStorageError::Io)?;
    let staged = format!("{}.{generation}.chain.tmp", operation.as_str());
    prepare_staged_file(
        commit,
        attempts,
        &staged,
        &bytes,
        Some(FaultPoint::ChainCheckpointWrite),
        None,
    )?;
    attempts
        .rename(&staged, session, CHAIN_CHECKPOINT_FILE)
        .map_err(map_storage_io)?;
    commit.record(|| {
        FsOp::Rename(
            DirRole::Attempts,
            staged.clone(),
            DirRole::Session,
            CHAIN_CHECKPOINT_FILE.to_owned(),
        )
    });
    Ok(())
}

/// Installs a generation manifest from `attempts/` into `generations/`.
///
/// An existing manifest with other bytes is a conflict. An identical one is a
/// retried publication: an ephemeral commit keeps it, while a durable commit
/// never trusts a file an earlier, possibly failed, attempt flushed and stages
/// the bytes again over it.
pub(super) fn install_immutable_file(
    commit: Commit<'_>,
    staging_directory: &Dir,
    staged_name: &str,
    destination_directory: &Dir,
    destination_name: &str,
    bytes: &[u8],
) -> Result<(), SessionStorageError> {
    if destination_directory
        .try_exists(destination_name)
        .map_err(map_storage_io)?
    {
        let existing = open_regular_file(destination_directory, destination_name, false)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        if read_bounded(existing).map_err(|_| SessionStorageError::IntegrityFailure)? != bytes {
            return Err(SessionStorageError::StateConflict);
        }
        if !commit.durable() {
            commit.record(|| FsOp::Accept(DirRole::Generations, destination_name.to_owned()));
            return Ok(());
        }
    }
    prepare_staged_file(
        commit,
        staging_directory,
        staged_name,
        bytes,
        Some(FaultPoint::ManifestWrite),
        Some(FaultPoint::ManifestFlush),
    )?;
    staging_directory
        .rename(staged_name, destination_directory, destination_name)
        .map_err(map_storage_io)?;
    commit.record(|| {
        FsOp::Rename(
            DirRole::Attempts,
            staged_name.to_owned(),
            DirRole::Generations,
            destination_name.to_owned(),
        )
    });
    commit.reach(FaultPoint::ManifestRename)
}

/// Stages `bytes` as `name` in `attempts/`: created new, written, flushed.
///
/// A leftover staged file from an earlier attempt is reused by an ephemeral
/// commit when it already holds the bytes (it is flushed again). A durable
/// commit always deletes it and writes it again, because a flush that follows
/// a failed one can report success for data that never reached the disk.
pub(super) fn prepare_staged_file(
    commit: Commit<'_>,
    directory: &Dir,
    name: &str,
    bytes: &[u8],
    write_point: Option<FaultPoint>,
    flush_point: Option<FaultPoint>,
) -> Result<(), SessionStorageError> {
    if directory.try_exists(name).map_err(map_storage_io)? {
        if !commit.durable() {
            let existing_file = open_regular_file(directory, name, true);
            let existing = existing_file
                .as_ref()
                .ok()
                .and_then(|file| file.try_clone().ok())
                .and_then(|file| read_bounded(file).ok())
                .unwrap_or_default();
            if existing == bytes {
                existing_file
                    .map_err(map_storage_io)?
                    .sync_all()
                    .map_err(map_storage_io)?;
                commit.record(|| FsOp::Accept(DirRole::Attempts, name.to_owned()));
                return Ok(());
            }
        }
        directory.remove_file(name).map_err(map_storage_io)?;
        commit.record(|| FsOp::Remove(DirRole::Attempts, name.to_owned()));
    }
    let mut file = create_new(directory, name).map_err(map_storage_io)?;
    commit.record(|| FsOp::Create(DirRole::Attempts, name.to_owned()));
    file.write_all(bytes).map_err(map_storage_io)?;
    commit.record(|| FsOp::Write(DirRole::Attempts, name.to_owned()));
    if let Some(point) = write_point {
        commit.reach(point)?;
    }
    file.sync_all().map_err(map_storage_io)?;
    commit.record(|| FsOp::SyncFile(DirRole::Attempts, name.to_owned()));
    if let Some(point) = flush_point {
        commit.reach(point)?;
    }
    Ok(())
}

/// Creates `name` in `directory` new, writes `bytes` and flushes it.
///
/// # Errors
///
/// The I/O error, so a caller can tell an existing file (`AlreadyExists`).
pub(super) fn create_committed_file(
    commit: Commit<'_>,
    directory: &Dir,
    role: DirRole,
    name: &str,
    bytes: &[u8],
) -> io::Result<()> {
    let mut file = create_new(directory, name)?;
    commit.record(|| FsOp::Create(role, name.to_owned()));
    file.write_all(bytes)?;
    commit.record(|| FsOp::Write(role, name.to_owned()));
    file.sync_all()?;
    commit.record(|| FsOp::SyncFile(role, name.to_owned()));
    Ok(())
}

/// Creates `name` in `directory`, failing if anything exists there, without
/// following a link.
fn create_new(directory: &Dir, name: &str) -> io::Result<cap_std::fs::File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    directory.open_with(name, &options)
}

/// Installs the content-addressed files of one commit in a session's
/// `artifacts/` directory, before the generation that names them.
pub(super) struct ArtifactInstaller<'a> {
    /// The session's commit.
    pub(super) commit: Commit<'a>,
    /// `artifacts/`.
    pub(super) artifacts: &'a Dir,
    /// `attempts/`, where a durable commit re-stages a file it cannot trust.
    pub(super) attempts: &'a Dir,
    /// The operation installing, which names re-staged files.
    pub(super) operation: &'a OperationId,
    /// The artifacts the committed head lists.
    pub(super) listed: &'a [StoredArtifact],
}

impl ArtifactInstaller<'_> {
    /// Installs `bytes` as `name`, whose SHA-256 is `digest`.
    ///
    /// A new file is created, written and flushed. An existing file is a
    /// retried or shared installation: an ephemeral commit accepts it when
    /// its bytes hash to `digest`, as does a durable commit when the
    /// committed head already lists it. Otherwise a durable commit stages the
    /// bytes again and renames them over the file (fsyncgate, ADR 0020).
    ///
    /// # Errors
    ///
    /// An existing file with other bytes is an integrity failure; any write,
    /// flush or rename error is a storage failure.
    pub(super) fn install(
        &self,
        name: &str,
        bytes: &[u8],
        digest: &str,
    ) -> Result<(), SessionStorageError> {
        let commit = self.commit;
        match create_committed_file(commit, self.artifacts, DirRole::Artifacts, name, bytes) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let listed = self
                    .listed
                    .iter()
                    .any(|artifact| artifact.name == name && artifact.sha256 == digest);
                if commit.durable() && !listed {
                    let staged = format!("{}.{digest}.artifact.tmp", self.operation.as_str());
                    prepare_staged_file(commit, self.attempts, &staged, bytes, None, None)?;
                    self.attempts
                        .rename(&staged, self.artifacts, name)
                        .map_err(map_storage_io)?;
                    commit.record(|| {
                        FsOp::Rename(
                            DirRole::Attempts,
                            staged.clone(),
                            DirRole::Artifacts,
                            name.to_owned(),
                        )
                    });
                } else {
                    let existing = open_regular_file(self.artifacts, name, false)
                        .map_err(|_| SessionStorageError::IntegrityFailure)?;
                    let expected = u64::try_from(bytes.len())
                        .map_err(|_| SessionStorageError::CapacityExhausted)?;
                    if hash_bounded(existing, expected)? != digest {
                        return Err(SessionStorageError::IntegrityFailure);
                    }
                    commit.record(|| FsOp::Accept(DirRole::Artifacts, name.to_owned()));
                }
            }
            Err(error) => return Err(map_storage_io(error)),
        }
        commit.reach(FaultPoint::ArtifactInstall)
    }

    /// Makes every installed file reachable: synchronises `artifacts/` once
    /// for a durable commit.
    ///
    /// # Errors
    ///
    /// A failed synchronisation is a storage failure.
    pub(super) fn finish(&self) -> Result<(), SessionStorageError> {
        self.commit
            .sync_directory(self.artifacts, DirRole::Artifacts)?;
        self.commit.reach(FaultPoint::ArtifactDirectorySync)
    }
}
