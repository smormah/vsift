//! Initialization of a session's private storage at generation 0.

use std::io;

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use vsift_application::{AuthorizedSessionStorageInitialization, SessionStorageError};
use vsift_domain::{OperationId, SessionId, StorageGeneration};

use super::{
    ARTIFACTS_DIRECTORY, ATTEMPTS_DIRECTORY, COORDINATION_DIRECTORY, CURRENT_FILE, CommitPointer,
    GENERATIONS_DIRECTORY, GenerationManifest, INITIAL_GENERATION_FILE, INITIALIZATION_LOCK,
    RECORDS_DIRECTORY, SESSIONS_DIRECTORY, STORAGE_SCHEMA_VERSION, chain::read_committed_manifest,
    create_regular_file, initialization_attempt_name, map_lock_error, map_storage_io,
    open_regular_file, sha256_hex,
};
use crate::file_lock::HeldFileLock;

pub(super) fn initialize_session(
    root: &Dir,
    request: &AuthorizedSessionStorageInitialization,
) -> Result<StorageGeneration, SessionStorageError> {
    let coordination = root
        .open_dir_nofollow(COORDINATION_DIRECTORY)
        .map_err(map_storage_io)?;
    let initialization_lock = HeldFileLock::try_exclusive(
        open_regular_file(&coordination, INITIALIZATION_LOCK, true)
            .map_err(map_storage_io)?
            .into_std(),
    )
    .map_err(map_lock_error)?;

    let result = initialize_session_while_locked(root, &coordination, request);
    drop(initialization_lock);
    result
}

pub(super) fn initialize_session_while_locked(
    root: &Dir,
    coordination: &Dir,
    request: &AuthorizedSessionStorageInitialization,
) -> Result<StorageGeneration, SessionStorageError> {
    ensure_session_lock_anchor(coordination, request.session_id(), "lifetime")?;
    ensure_session_lock_anchor(coordination, request.session_id(), "writer")?;

    let sessions = root
        .open_dir_nofollow(SESSIONS_DIRECTORY)
        .map_err(map_storage_io)?;
    if sessions
        .try_exists(request.session_id().as_str())
        .map_err(map_storage_io)?
    {
        return validate_initial_session(&sessions, request.session_id(), request.operation_id());
    }

    let attempt_name = initialization_attempt_name(request.session_id(), request.operation_id());
    if sessions.try_exists(&attempt_name).map_err(map_storage_io)? {
        match validate_attempt(&sessions, &attempt_name, request) {
            Ok(generation) => {
                sessions
                    .rename(&attempt_name, &sessions, request.session_id().as_str())
                    .map_err(map_storage_io)?;
                return Ok(generation);
            }
            Err(SessionStorageError::IntegrityFailure) => {
                discard_incomplete_initial_attempt(&sessions, &attempt_name)?;
            }
            Err(error) => return Err(error),
        }
    }

    create_initial_attempt(&sessions, &attempt_name, request)?;
    sessions
        .rename(&attempt_name, &sessions, request.session_id().as_str())
        .map_err(map_storage_io)?;
    validate_initial_session(&sessions, request.session_id(), request.operation_id())
}

pub(super) fn discard_incomplete_initial_attempt(
    sessions: &Dir,
    attempt_name: &str,
) -> Result<(), SessionStorageError> {
    let attempt = sessions
        .open_dir_nofollow(attempt_name)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let _ = attempt.remove_file(CURRENT_FILE);
    for directory_name in [
        GENERATIONS_DIRECTORY,
        RECORDS_DIRECTORY,
        ARTIFACTS_DIRECTORY,
        ATTEMPTS_DIRECTORY,
    ] {
        if let Ok(directory) = attempt.open_dir_nofollow(directory_name) {
            if directory_name == GENERATIONS_DIRECTORY {
                let _ = directory.remove_file(INITIAL_GENERATION_FILE);
            }
            drop(directory);
            attempt
                .remove_dir(directory_name)
                .map_err(|_| SessionStorageError::IntegrityFailure)?;
        }
    }
    drop(attempt);
    sessions
        .remove_dir(attempt_name)
        .map_err(|_| SessionStorageError::IntegrityFailure)
}

pub(super) fn ensure_session_lock_anchor(
    coordination: &Dir,
    session_id: &SessionId,
    role: &str,
) -> Result<(), SessionStorageError> {
    let name = format!("{}.{}.lock", session_id.as_str(), role);
    match create_regular_file(coordination, &name, b"vsift stable lock anchor\n") {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            open_regular_file(coordination, &name, true)
                .map(drop)
                .map_err(map_storage_io)
        }
        Err(error) => Err(map_storage_io(error)),
    }
}

pub(super) fn create_initial_attempt(
    sessions: &Dir,
    attempt_name: &str,
    request: &AuthorizedSessionStorageInitialization,
) -> Result<(), SessionStorageError> {
    sessions.create_dir(attempt_name).map_err(map_storage_io)?;
    let attempt = sessions
        .open_dir_nofollow(attempt_name)
        .map_err(map_storage_io)?;
    for directory in [
        GENERATIONS_DIRECTORY,
        RECORDS_DIRECTORY,
        ARTIFACTS_DIRECTORY,
        ATTEMPTS_DIRECTORY,
    ] {
        attempt.create_dir(directory).map_err(map_storage_io)?;
    }

    let manifest = GenerationManifest {
        schema_version: STORAGE_SCHEMA_VERSION,
        session_id: request.session_id().as_str().to_owned(),
        operation_id: request.operation_id().as_str().to_owned(),
        generation: StorageGeneration::INITIAL.value(),
        previous_manifest_sha256: None,
        lifecycle: None,
    };
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|_| SessionStorageError::Io)?;
    let generations = attempt
        .open_dir_nofollow(GENERATIONS_DIRECTORY)
        .map_err(map_storage_io)?;
    create_regular_file(&generations, INITIAL_GENERATION_FILE, &manifest_bytes)
        .map_err(map_storage_io)?;

    let pointer = CommitPointer {
        schema_version: STORAGE_SCHEMA_VERSION,
        generation: StorageGeneration::INITIAL.value(),
        manifest_sha256: sha256_hex(&manifest_bytes),
    };
    let pointer_bytes = serde_json::to_vec(&pointer).map_err(|_| SessionStorageError::Io)?;
    create_regular_file(&attempt, CURRENT_FILE, &pointer_bytes).map_err(map_storage_io)
}

pub(super) fn validate_attempt(
    sessions: &Dir,
    attempt_name: &str,
    request: &AuthorizedSessionStorageInitialization,
) -> Result<StorageGeneration, SessionStorageError> {
    validate_session_directory(
        sessions,
        attempt_name,
        request.session_id(),
        request.operation_id(),
    )
}

pub(super) fn validate_initial_session(
    sessions: &Dir,
    session_id: &SessionId,
    operation_id: &OperationId,
) -> Result<StorageGeneration, SessionStorageError> {
    validate_session_directory(sessions, session_id.as_str(), session_id, operation_id)
}

pub(super) fn validate_session_directory(
    sessions: &Dir,
    directory_name: &str,
    session_id: &SessionId,
    operation_id: &OperationId,
) -> Result<StorageGeneration, SessionStorageError> {
    let session = sessions
        .open_dir_nofollow(directory_name)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    for directory in [
        GENERATIONS_DIRECTORY,
        RECORDS_DIRECTORY,
        ARTIFACTS_DIRECTORY,
        ATTEMPTS_DIRECTORY,
    ] {
        session
            .open_dir_nofollow(directory)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
    }

    let committed = read_committed_manifest(&session, session_id)?;
    let manifest = committed.manifest;
    if manifest.generation != StorageGeneration::INITIAL.value() {
        return Err(SessionStorageError::IntegrityFailure);
    }
    if manifest.operation_id != operation_id.as_str() {
        return Err(SessionStorageError::StateConflict);
    }
    Ok(StorageGeneration::INITIAL)
}
