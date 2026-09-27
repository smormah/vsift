//! Initialization of a session's private storage at generation 0.
//!
//! The session is built under a temporary attempt name and renamed into
//! `sessions/` whole. A durable session (ADR 0020) also flushes each file and
//! synchronises `generations/`, the attempt directory, `sessions/` and the
//! session's index bucket before generation 0 is acknowledged, and never adopts
//! an attempt an earlier, possibly failed, run left behind.

use std::io;

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use vsift_application::{AuthorizedSessionStorageInitialization, SessionStorageError};
use vsift_domain::{OperationId, SessionId, StorageGeneration};

use super::map_committed_io;
use super::{
    ARTIFACTS_DIRECTORY, ATTEMPTS_DIRECTORY, COORDINATION_DIRECTORY, CURRENT_FILE, ChainCheck,
    CommitPointer, GENERATIONS_DIRECTORY, GenerationManifest, INITIAL_GENERATION_FILE,
    INITIALIZATION_LOCK, RECORDS_DIRECTORY, SESSION_INDEX_DIRECTORY, SESSIONS_DIRECTORY,
    STORAGE_SCHEMA_VERSION, StoredDurability,
    chain::read_committed_manifest,
    commit::{Commit, CommitHooks, DirRole, FsOp},
    create_regular_file, initialization_attempt_name, map_lock_error, map_storage_io,
    open_regular_file,
    publication::create_committed_file,
    session_bucket, sha256_hex,
};
use crate::file_lock::HeldFileLock;

/// Initializes a session the application authorized (its durability passed
/// the store's capability preflight).
pub(super) fn initialize_session(
    root: &Dir,
    request: &AuthorizedSessionStorageInitialization,
    hooks: &CommitHooks<'_>,
) -> Result<StorageGeneration, SessionStorageError> {
    initialize(
        root,
        &InitialGeneration {
            session_id: request.session_id(),
            operation_id: request.operation_id(),
            durability: StoredDurability::from_requirement(request.durability()),
        },
        hooks,
    )
}

/// What generation 0 of a session records.
pub(super) struct InitialGeneration<'a> {
    /// The new session.
    pub(super) session_id: &'a SessionId,
    /// The initializing operation.
    pub(super) operation_id: &'a OperationId,
    /// The session's publication mode for its whole life.
    pub(super) durability: StoredDurability,
}

/// Creates generation 0 under the root's initialization lock.
pub(super) fn initialize(
    root: &Dir,
    request: &InitialGeneration<'_>,
    hooks: &CommitHooks<'_>,
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

    let commit = hooks.commit(request.durability);
    let result = initialize_session_while_locked(root, &coordination, request, commit);
    drop(initialization_lock);
    result
}

pub(super) fn initialize_session_while_locked(
    root: &Dir,
    coordination: &Dir,
    request: &InitialGeneration<'_>,
    commit: Commit<'_>,
) -> Result<StorageGeneration, SessionStorageError> {
    ensure_session_lock_anchor(coordination, request.session_id, "lifetime")?;
    ensure_session_lock_anchor(coordination, request.session_id, "writer")?;

    let sessions = root
        .open_dir_nofollow(SESSIONS_DIRECTORY)
        .map_err(map_storage_io)?;
    if sessions
        .try_exists(request.session_id.as_str())
        .map_err(map_storage_io)?
    {
        let generation = validate_initial_session(&sessions, request, commit.durability())?;
        // A durable retry repeats the synchronisations an earlier attempt may
        // have failed before it acknowledges.
        make_session_reachable(root, &sessions, request.session_id, commit)?;
        return Ok(generation);
    }

    let attempt_name = initialization_attempt_name(request.session_id, request.operation_id);
    if sessions.try_exists(&attempt_name).map_err(map_storage_io)? {
        if commit.durable() {
            // Its files may have been flushed by a failed attempt: rebuild.
            discard_incomplete_initial_attempt(&sessions, &attempt_name)?;
        } else {
            match validate_attempt(&sessions, &attempt_name, request, commit.durability()) {
                Ok(generation) => {
                    sessions
                        .rename(&attempt_name, &sessions, request.session_id.as_str())
                        .map_err(map_storage_io)?;
                    return Ok(generation);
                }
                Err(SessionStorageError::IntegrityFailure) => {
                    discard_incomplete_initial_attempt(&sessions, &attempt_name)?;
                }
                Err(error) => return Err(error),
            }
        }
    }

    create_initial_attempt(&sessions, &attempt_name, request, commit)?;
    sessions
        .rename(&attempt_name, &sessions, request.session_id.as_str())
        .map_err(map_storage_io)?;
    commit.record(|| {
        FsOp::RenameDirectory(
            DirRole::Sessions,
            attempt_name.clone(),
            DirRole::Sessions,
            request.session_id.as_str().to_owned(),
        )
    });
    make_session_reachable(root, &sessions, request.session_id, commit)?;
    let generation = validate_initial_session(&sessions, request, commit.durability())?;
    commit.record(|| FsOp::Committed);
    Ok(generation)
}

/// Synchronises `sessions/` and the session's registration in the index, so a
/// durable session and its registration survive an OS crash once generation 0
/// is acknowledged.
fn make_session_reachable(
    root: &Dir,
    sessions: &Dir,
    session_id: &SessionId,
    commit: Commit<'_>,
) -> Result<(), SessionStorageError> {
    if !commit.durable() {
        return Ok(());
    }
    commit.sync_directory(sessions, DirRole::Sessions)?;
    if !root
        .try_exists(SESSION_INDEX_DIRECTORY)
        .map_err(map_storage_io)?
    {
        return Ok(());
    }
    let index = root
        .open_dir_nofollow(SESSION_INDEX_DIRECTORY)
        .map_err(map_committed_io)?;
    let bucket_name = session_bucket(session_id);
    if !index.try_exists(&bucket_name).map_err(map_storage_io)? {
        return Ok(());
    }
    let bucket = index
        .open_dir_nofollow(&bucket_name)
        .map_err(map_committed_io)?;
    commit.sync_directory(&bucket, DirRole::IndexBucket)?;
    commit.sync_directory(&index, DirRole::SessionIndex)
}

pub(super) fn discard_incomplete_initial_attempt(
    sessions: &Dir,
    attempt_name: &str,
) -> Result<(), SessionStorageError> {
    let attempt = sessions
        .open_dir_nofollow(attempt_name)
        .map_err(map_committed_io)?;
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
                .map_err(map_committed_io)?;
        }
    }
    drop(attempt);
    sessions.remove_dir(attempt_name).map_err(map_committed_io)
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
    request: &InitialGeneration<'_>,
    commit: Commit<'_>,
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
        session_id: request.session_id.as_str().to_owned(),
        operation_id: request.operation_id.as_str().to_owned(),
        generation: StorageGeneration::INITIAL.value(),
        previous_manifest_sha256: None,
        durability: commit.durability(),
        lifecycle: None,
    };
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|_| SessionStorageError::Io)?;
    let generations = attempt
        .open_dir_nofollow(GENERATIONS_DIRECTORY)
        .map_err(map_storage_io)?;
    create_committed_file(
        commit,
        &generations,
        DirRole::Generations,
        INITIAL_GENERATION_FILE,
        &manifest_bytes,
    )
    .map_err(map_storage_io)?;
    commit.sync_directory(&generations, DirRole::Generations)?;

    let pointer = CommitPointer {
        schema_version: STORAGE_SCHEMA_VERSION,
        generation: StorageGeneration::INITIAL.value(),
        manifest_sha256: sha256_hex(&manifest_bytes),
    };
    let pointer_bytes = serde_json::to_vec(&pointer).map_err(|_| SessionStorageError::Io)?;
    create_committed_file(
        commit,
        &attempt,
        DirRole::Session,
        CURRENT_FILE,
        &pointer_bytes,
    )
    .map_err(map_storage_io)?;
    commit.sync_directory(&attempt, DirRole::Session)
}

pub(super) fn validate_attempt(
    sessions: &Dir,
    attempt_name: &str,
    request: &InitialGeneration<'_>,
    durability: StoredDurability,
) -> Result<StorageGeneration, SessionStorageError> {
    validate_session_directory(
        sessions,
        attempt_name,
        request.session_id,
        request.operation_id,
        durability,
    )
}

pub(super) fn validate_initial_session(
    sessions: &Dir,
    request: &InitialGeneration<'_>,
    durability: StoredDurability,
) -> Result<StorageGeneration, SessionStorageError> {
    validate_session_directory(
        sessions,
        request.session_id.as_str(),
        request.session_id,
        request.operation_id,
        durability,
    )
}

/// Checks an initialized session directory: its layout, a generation-0 head
/// from `operation_id`, and the requested durability (a session's mode never
/// changes, so another one is a conflict).
pub(super) fn validate_session_directory(
    sessions: &Dir,
    directory_name: &str,
    session_id: &SessionId,
    operation_id: &OperationId,
    durability: StoredDurability,
) -> Result<StorageGeneration, SessionStorageError> {
    let session = sessions
        .open_dir_nofollow(directory_name)
        .map_err(map_committed_io)?;
    for directory in [
        GENERATIONS_DIRECTORY,
        RECORDS_DIRECTORY,
        ARTIFACTS_DIRECTORY,
        ATTEMPTS_DIRECTORY,
    ] {
        session
            .open_dir_nofollow(directory)
            .map_err(map_committed_io)?;
    }

    let committed = read_committed_manifest(&session, session_id, ChainCheck::Full)?;
    let manifest = committed.manifest;
    if manifest.generation != StorageGeneration::INITIAL.value() {
        return Err(SessionStorageError::IntegrityFailure);
    }
    if manifest.operation_id != operation_id.as_str() || manifest.durability != durability {
        return Err(SessionStorageError::StateConflict);
    }
    Ok(StorageGeneration::INITIAL)
}
