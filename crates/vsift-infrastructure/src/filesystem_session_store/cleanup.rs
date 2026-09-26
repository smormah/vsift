//! Claiming and removing closed, expired or abandoned sessions.

use std::path::Path;

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use vsift_application::SessionStorageError;
use vsift_domain::{OperationId, SessionId, SessionLifetime, SessionPhase};

use super::{
    COORDINATION_DIRECTORY, CleanOutcome, FilesystemSessionStore, GENERATIONS_DIRECTORY,
    INITIAL_GENERATION_FILE, INITIALIZATION_LOCK, SESSION_INDEX_DIRECTORY, SESSIONS_DIRECTORY,
    SessionIndexMarker, chain::read_committed_manifest, map_lock_error, map_storage_io,
    open_regular_file, session_bucket, stored::read_versioned_json_file,
};
use crate::file_lock::HeldFileLock;

impl FilesystemSessionStore {
    /// Claims and cleans one session only after verifying its ownership and state.
    ///
    /// An initial generation without a source binding is abandoned only after
    /// the same idle interval. A held lifetime lock always wins over timestamps.
    ///
    /// # Errors
    ///
    /// Busy, corrupt, future-version, or inaccessible sessions are never removed.
    pub fn clean_session(
        &self,
        session_id: &SessionId,
        now_unix_seconds: u64,
        dry_run: bool,
    ) -> Result<CleanOutcome, SessionStorageError> {
        let index_claim = self.claim_registration(session_id)?;
        self.revalidate_root()?;
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?;
        let quarantine_name = format!(".quarantine-{}", session_id.as_str());
        let existing_quarantine = sessions
            .try_exists(&quarantine_name)
            .map_err(map_storage_io)?;
        if !existing_quarantine
            && !sessions
                .try_exists(session_id.as_str())
                .map_err(map_storage_io)?
        {
            let Some((bucket, _marker_lock, marker)) = index_claim else {
                return Err(SessionStorageError::IntegrityFailure);
            };
            if now_unix_seconds.saturating_sub(marker.registered_at_unix_seconds)
                < SessionLifetime::IDLE_SECONDS
            {
                return Ok(CleanOutcome::Ineligible);
            }
            if dry_run {
                return Ok(CleanOutcome::Eligible);
            }
            bucket
                .remove_file(session_id.as_str())
                .map_err(map_storage_io)?;
            return Ok(CleanOutcome::Removed);
        }
        let _hold = self.try_acquire_exclusive_lifetime(session_id)?;
        let selected_name = if existing_quarantine {
            quarantine_name.as_str()
        } else {
            session_id.as_str()
        };
        let session = sessions
            .open_dir_nofollow(selected_name)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let committed = read_committed_manifest(&session, session_id)?;
        let eligible = if existing_quarantine {
            true
        } else if let Some(record) = committed.manifest.lifecycle {
            let status = record.to_status(session_id.clone(), committed.manifest.generation)?;
            status.phase() == SessionPhase::Closed || status.lifetime().expired(now_unix_seconds)
        } else {
            if committed.manifest.generation != 0 {
                return Err(SessionStorageError::IntegrityFailure);
            }
            let generations = session
                .open_dir_nofollow(GENERATIONS_DIRECTORY)
                .map_err(|_| SessionStorageError::IntegrityFailure)?;
            let initial = open_regular_file(&generations, INITIAL_GENERATION_FILE, false)
                .map_err(|_| SessionStorageError::IntegrityFailure)?;
            let modified = initial
                .metadata()
                .map_err(map_storage_io)?
                .modified()
                .map_err(map_storage_io)?
                .into_std()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| SessionStorageError::IntegrityFailure)?
                .as_secs();
            now_unix_seconds.saturating_sub(modified) >= SessionLifetime::IDLE_SECONDS
        };
        if !eligible {
            return Ok(CleanOutcome::Ineligible);
        }
        if dry_run {
            return Ok(CleanOutcome::Eligible);
        }
        let mut budget = CleanupBudget::default();
        validate_owned_tree(&session, 0, &mut budget)?;
        drop(session);
        if !existing_quarantine {
            if sessions
                .try_exists(&quarantine_name)
                .map_err(map_storage_io)?
            {
                return Err(SessionStorageError::StateConflict);
            }
            sessions
                .rename(session_id.as_str(), &sessions, &quarantine_name)
                .map_err(map_storage_io)?;
        }
        sessions
            .remove_dir_all(&quarantine_name)
            .map_err(map_storage_io)?;
        if let Some((bucket, _marker_lock, _marker)) = index_claim {
            bucket
                .remove_file(session_id.as_str())
                .map_err(map_storage_io)?;
        }
        Ok(CleanOutcome::Removed)
    }

    pub(super) fn claim_registration(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<(Dir, HeldFileLock, SessionIndexMarker)>, SessionStorageError> {
        let _initialization_lock = self.try_root_initialization_lock()?;
        if !self
            .root
            .try_exists(SESSION_INDEX_DIRECTORY)
            .map_err(map_storage_io)?
        {
            return Ok(None);
        }
        let index = self
            .root
            .open_dir_nofollow(SESSION_INDEX_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let bucket_name = session_bucket(session_id);
        if !index.try_exists(&bucket_name).map_err(map_storage_io)? {
            return Ok(None);
        }
        let bucket = index
            .open_dir_nofollow(&bucket_name)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        if !bucket
            .try_exists(session_id.as_str())
            .map_err(map_storage_io)?
        {
            return Ok(None);
        }
        let marker_lock = open_regular_file(&bucket, session_id.as_str(), true)
            .map_err(|_| SessionStorageError::IntegrityFailure)?
            .into_std();
        let marker = read_versioned_json_file::<SessionIndexMarker>(&bucket, session_id.as_str())?;
        let marker_lock = HeldFileLock::try_exclusive(marker_lock).map_err(map_lock_error)?;
        if marker.session_id != session_id.as_str()
            || OperationId::parse(&marker.operation_id).is_err()
        {
            return Err(SessionStorageError::IntegrityFailure);
        }
        Ok(Some((bucket, marker_lock, marker)))
    }

    pub(super) fn try_root_initialization_lock(&self) -> Result<HeldFileLock, SessionStorageError> {
        let coordination = self
            .root
            .open_dir_nofollow(COORDINATION_DIRECTORY)
            .map_err(map_storage_io)?;
        HeldFileLock::try_exclusive(
            open_regular_file(&coordination, INITIALIZATION_LOCK, true)
                .map_err(map_storage_io)?
                .into_std(),
        )
        .map_err(map_lock_error)
    }
}

#[derive(Default)]
pub(super) struct CleanupBudget {
    entries: u32,
    bytes: u64,
}

pub(super) fn validate_owned_tree(
    directory: &Dir,
    depth: u8,
    budget: &mut CleanupBudget,
) -> Result<(), SessionStorageError> {
    if depth > 5 {
        return Err(SessionStorageError::CapacityExhausted);
    }
    for entry in directory.entries().map_err(map_storage_io)? {
        let entry = entry.map_err(map_storage_io)?;
        budget.entries = budget
            .entries
            .checked_add(1)
            .ok_or(SessionStorageError::CapacityExhausted)?;
        if budget.entries > 100_000 {
            return Err(SessionStorageError::CapacityExhausted);
        }
        let name = entry.file_name();
        let kind = entry.file_type().map_err(map_storage_io)?;
        if kind.is_dir() {
            let child = directory
                .open_dir_nofollow(Path::new(&name))
                .map_err(|_| SessionStorageError::IntegrityFailure)?;
            validate_owned_tree(&child, depth + 1, budget)?;
        } else if kind.is_file() {
            let name = name.to_str().ok_or(SessionStorageError::IntegrityFailure)?;
            let file = open_regular_file(directory, name, false)
                .map_err(|_| SessionStorageError::IntegrityFailure)?;
            let metadata = file.metadata().map_err(map_storage_io)?;
            budget.bytes = budget
                .bytes
                .checked_add(metadata.len())
                .ok_or(SessionStorageError::CapacityExhausted)?;
            if budget.bytes > 30 * 1024 * 1024 * 1024 {
                return Err(SessionStorageError::CapacityExhausted);
            }
        } else {
            return Err(SessionStorageError::IntegrityFailure);
        }
    }
    Ok(())
}
