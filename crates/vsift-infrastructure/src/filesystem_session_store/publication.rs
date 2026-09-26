//! Immutable generation publication: lifecycle updates, staging, installation and commit.

use std::io::{self, Write};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use vsift_application::{
    AuthorizedSessionGenerationPublication, AuthorizedSessionStorageInitialization,
    PublishSessionGenerationRequest, SessionStorageError, SessionStore, StorageCapabilities,
};
use vsift_domain::{PublicationGuarantee, SessionLifetime, StorageGeneration};

use super::{
    ATTEMPTS_DIRECTORY, COORDINATION_DIRECTORY, CURRENT_FILE, CommitPointer,
    FilesystemSessionStore, GENERATIONS_DIRECTORY, GenerationManifest, LifecycleUpdate,
    MAX_GENERATIONS_PER_SESSION, MAX_METADATA_BYTES, MAX_SESSION_ARTIFACT_BYTES,
    MAX_SESSION_ARTIFACTS, PublicationBoundary, SESSIONS_DIRECTORY, STORAGE_SCHEMA_VERSION,
    StoredArtifact, StoredArtifactKind, StoredLifecycle, StoredSessionPhase,
    chain::read_committed_manifest,
    create_regular_file, hash_bounded,
    initialization::initialize_session,
    map_lock_error, map_open_error, map_storage_io, open_regular_file, open_session_lock,
    read_bounded,
    root::{acquire_admission, validate_platform_root_permissions},
    sha256_hex,
    stored::{validate_artifact_record, validate_source_record},
};
use crate::file_lock::HeldFileLock;

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
    /// Durable requests fail before admission or filesystem mutation. Other typed
    /// failures preserve the previously committed generation.
    #[cfg(test)]
    pub(super) async fn publish_generation(
        &self,
        request: PublishSessionGenerationRequest,
    ) -> Result<StorageGeneration, SessionStorageError> {
        if request.durability() == vsift_domain::DurabilityRequirement::Durable {
            return Err(SessionStorageError::UnsupportedGuarantee {
                requested: request.durability(),
                available: PublicationGuarantee::ProcessCrashConsistent,
            });
        }
        let root = self.root.try_clone().map_err(map_storage_io)?;
        let root_path = self.root_path.clone();
        let capacity = self.admission_capacity;
        tokio::task::spawn_blocking(move || {
            validate_platform_root_permissions(&root_path, &root).map_err(map_open_error)?;
            publish_generation(&root, capacity, &request, None)
        })
        .await
        .map_err(|_| SessionStorageError::Io)?
    }
}

impl SessionStore for FilesystemSessionStore {
    fn capabilities(&self) -> StorageCapabilities {
        StorageCapabilities::new(PublicationGuarantee::ProcessCrashConsistent)
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
                initialize_session(&root, &request)
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
                publish_generation(&root, capacity, &request, None)
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
    fault: Option<PublicationBoundary>,
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

    let result = publish_generation_while_locked(root, request, LifecycleUpdate::Keep, fault);
    drop(writer);
    drop(lifetime);
    result
}

pub(super) fn publish_generation_with_update(
    root: &Dir,
    capacity: u16,
    request: &PublishSessionGenerationRequest,
    update: LifecycleUpdate,
    exclusive_lifetime: bool,
) -> Result<StorageGeneration, SessionStorageError> {
    let _admission = acquire_admission(root, capacity, 1)?;
    let coordination = root
        .open_dir_nofollow(COORDINATION_DIRECTORY)
        .map_err(map_storage_io)?;
    let lifetime_file = open_session_lock(&coordination, request.session_id(), "lifetime")?;
    let lifetime = if exclusive_lifetime {
        HeldFileLock::try_exclusive(lifetime_file)
    } else {
        HeldFileLock::try_shared(lifetime_file)
    }
    .map_err(map_lock_error)?;
    let writer = HeldFileLock::try_exclusive(open_session_lock(
        &coordination,
        request.session_id(),
        "writer",
    )?)
    .map_err(map_lock_error)?;
    let result = publish_generation_while_locked(root, request, update, None);
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

pub(super) fn publish_generation_while_locked(
    root: &Dir,
    request: &PublishSessionGenerationRequest,
    update: LifecycleUpdate,
    fault: Option<PublicationBoundary>,
) -> Result<StorageGeneration, SessionStorageError> {
    let sessions = root
        .open_dir_nofollow(SESSIONS_DIRECTORY)
        .map_err(map_storage_io)?;
    let session = sessions
        .open_dir_nofollow(request.session_id().as_str())
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let current = read_committed_manifest(&session, request.session_id())?;
    let expected = request.expected_generation().value();

    if current.manifest.generation != expected {
        if current.manifest.generation == expected.saturating_add(1)
            && current.manifest.operation_id == request.operation_id().as_str()
        {
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
    let lifecycle = update_lifecycle(current.manifest.lifecycle, update)?;
    let manifest = GenerationManifest {
        schema_version: STORAGE_SCHEMA_VERSION,
        session_id: request.session_id().as_str().to_owned(),
        operation_id: request.operation_id().as_str().to_owned(),
        generation: next.value(),
        previous_manifest_sha256: Some(current.digest),
        lifecycle,
    };
    let bytes = serde_json::to_vec(&manifest).map_err(|_| SessionStorageError::Io)?;
    // A manifest that could not be read back would strand the session.
    if u64::try_from(bytes.len()).map_or(true, |size| size > MAX_METADATA_BYTES) {
        return Err(SessionStorageError::CapacityExhausted);
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
        &attempts,
        &staged_manifest,
        &generations,
        &generation_name,
        &bytes,
        fault,
    )?;

    let pointer = CommitPointer {
        schema_version: STORAGE_SCHEMA_VERSION,
        generation: next.value(),
        manifest_sha256: sha256_hex(&bytes),
    };
    let pointer_bytes = serde_json::to_vec(&pointer).map_err(|_| SessionStorageError::Io)?;
    let staged_pointer = format!(
        "{}.{}.pointer.tmp",
        request.operation_id().as_str(),
        next.value()
    );
    prepare_staged_file(
        &attempts,
        &staged_pointer,
        &pointer_bytes,
        fault,
        PublicationBoundary::PointerWrite,
        PublicationBoundary::PointerFlush,
    )?;
    attempts
        .rename(&staged_pointer, &session, CURRENT_FILE)
        .map_err(map_storage_io)?;
    inject_fault(fault, PublicationBoundary::PointerRename)?;

    let committed = read_committed_manifest(&session, request.session_id())?;
    if committed.manifest.generation != next.value()
        || committed.manifest.operation_id != request.operation_id().as_str()
    {
        return Err(SessionStorageError::IntegrityFailure);
    }
    Ok(next)
}

pub(super) fn install_immutable_file(
    staging_directory: &Dir,
    staged_name: &str,
    destination_directory: &Dir,
    destination_name: &str,
    bytes: &[u8],
    fault: Option<PublicationBoundary>,
) -> Result<(), SessionStorageError> {
    if destination_directory
        .try_exists(destination_name)
        .map_err(map_storage_io)?
    {
        let existing = open_regular_file(destination_directory, destination_name, false)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        if read_bounded(existing).map_err(|_| SessionStorageError::IntegrityFailure)? == bytes {
            return Ok(());
        }
        return Err(SessionStorageError::StateConflict);
    }
    prepare_staged_file(
        staging_directory,
        staged_name,
        bytes,
        fault,
        PublicationBoundary::ManifestWrite,
        PublicationBoundary::ManifestFlush,
    )?;
    staging_directory
        .rename(staged_name, destination_directory, destination_name)
        .map_err(map_storage_io)?;
    inject_fault(fault, PublicationBoundary::ManifestRename)
}

pub(super) fn prepare_staged_file(
    directory: &Dir,
    name: &str,
    bytes: &[u8],
    fault: Option<PublicationBoundary>,
    write_boundary: PublicationBoundary,
    flush_boundary: PublicationBoundary,
) -> Result<(), SessionStorageError> {
    if directory.try_exists(name).map_err(map_storage_io)? {
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
            return Ok(());
        }
        directory.remove_file(name).map_err(map_storage_io)?;
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let mut file = directory
        .open_with(name, &options)
        .map_err(map_storage_io)?;
    file.write_all(bytes).map_err(map_storage_io)?;
    inject_fault(fault, write_boundary)?;
    file.sync_all().map_err(map_storage_io)?;
    inject_fault(fault, flush_boundary)
}

pub(super) fn inject_fault(
    configured: Option<PublicationBoundary>,
    reached: PublicationBoundary,
) -> Result<(), SessionStorageError> {
    #[cfg(test)]
    if std::env::var("VSIFT_P03_CRASH_BOUNDARY").ok().as_deref()
        == Some(publication_boundary_name(reached))
    {
        std::process::exit(91);
    }
    if configured == Some(reached) {
        Err(SessionStorageError::Io)
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(super) const fn publication_boundary_name(boundary: PublicationBoundary) -> &'static str {
    match boundary {
        PublicationBoundary::ManifestWrite => "manifest-write",
        PublicationBoundary::ManifestFlush => "manifest-flush",
        PublicationBoundary::ManifestRename => "manifest-rename",
        PublicationBoundary::PointerWrite => "pointer-write",
        PublicationBoundary::PointerFlush => "pointer-flush",
        PublicationBoundary::PointerRename => "pointer-rename",
    }
}

/// Installs immutable bytes under their digest name, accepting an identical
/// existing file (a retried publication) and rejecting a different one.
pub(super) fn install_content_addressed(
    directory: &Dir,
    name: &str,
    bytes: &[u8],
    digest: &str,
) -> Result<(), SessionStorageError> {
    match create_regular_file(directory, name, bytes) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let existing = open_regular_file(directory, name, false)
                .map_err(|_| SessionStorageError::IntegrityFailure)?;
            let expected =
                u64::try_from(bytes.len()).map_err(|_| SessionStorageError::CapacityExhausted)?;
            if hash_bounded(existing, expected)? == digest {
                Ok(())
            } else {
                Err(SessionStorageError::IntegrityFailure)
            }
        }
        Err(error) => Err(map_storage_io(error)),
    }
}
