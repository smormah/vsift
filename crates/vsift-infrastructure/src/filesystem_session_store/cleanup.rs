//! Claiming and removing closed, expired or abandoned sessions.

use std::path::Path;

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use vsift_application::SessionStorageError;
use vsift_domain::{OperationId, SessionId, SessionLifetime, SessionPhase};

use super::map_committed_io;
use super::{
    COORDINATION_DIRECTORY, ChainCheck, CleanOutcome, CommittedManifest, FilesystemSessionStore,
    GENERATIONS_DIRECTORY, INITIAL_GENERATION_FILE, INITIALIZATION_LOCK, SESSION_INDEX_DIRECTORY,
    SESSIONS_DIRECTORY, SessionIndexMarker,
    chain::read_committed_manifest,
    index::{MarkerRead, is_held_by_a_remover, read_marker},
    map_lock_error, map_storage_io, open_regular_file, session_bucket,
};
use crate::file_lock::HeldFileLock;

impl FilesystemSessionStore {
    /// Claims and cleans one session only after verifying its ownership and state.
    ///
    /// An initial generation without a source binding is abandoned only after
    /// the same idle interval. A held lifetime lock always wins over timestamps.
    ///
    /// A registration that is gone when it is claimed is [`CleanOutcome::Gone`],
    /// and a removal an earlier call left half done (a quarantine) is finished.
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
        self.clean_session_scoped(session_id, now_unix_seconds, dry_run, None)
    }

    /// Removes the registration, and the initialized session folder if there is
    /// one, that one failed open created, **at once**, without waiting for the
    /// idle interval (#277).
    ///
    /// This is [`Self::clean_session`] under a narrower rule, not another
    /// deleter: the same claim, the same exclusive lifetime lock, the same
    /// bounded and owned-tree-checked quarantine removal. It removes only what
    /// `operation_id`, the operation that registered the session, made: the
    /// registration marker must name that operation, so a marker another open
    /// wrote is [`CleanOutcome::Ineligible`]. It removes only a session that
    /// was **never published**: a session whose first generation was activated
    /// is always [`CleanOutcome::Ineligible`]. A busy session, an unreadable
    /// marker or any failure is an error and removes nothing; the caller keeps
    /// its original failure and the registration is left for `session clean`
    /// after the idle interval, as before.
    ///
    /// # Errors
    ///
    /// Busy, corrupt, future-version, or inaccessible sessions are never removed.
    pub fn abandon_unpublished_open(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        now_unix_seconds: u64,
    ) -> Result<CleanOutcome, SessionStorageError> {
        self.clean_session_scoped(session_id, now_unix_seconds, false, Some(operation_id))
    }

    /// [`Self::clean_session`], or with `abandon` the rule of
    /// [`Self::abandon_unpublished_open`].
    fn clean_session_scoped(
        &self,
        session_id: &SessionId,
        now_unix_seconds: u64,
        dry_run: bool,
        abandon: Option<&OperationId>,
    ) -> Result<CleanOutcome, SessionStorageError> {
        let index_claim = self.claim_registration(session_id)?;
        if let Some(operation) = abandon {
            // Only what this very open registered: its marker names it.
            match &index_claim {
                Some((_, _, marker)) if marker.operation_id == operation.as_str() => {}
                _ => return Ok(CleanOutcome::Ineligible),
            }
        }
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
            // No folder, no quarantine and no marker: the registration was removed
            // after it was listed (another cleanup, or a failed open removing its
            // own). There is nothing left to examine, which is not damage.
            let Some((bucket, _marker_lock, marker)) = index_claim else {
                return Ok(CleanOutcome::Gone);
            };
            if abandon.is_none()
                && now_unix_seconds.saturating_sub(marker.registered_at_unix_seconds)
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
            .map_err(map_committed_io)?;
        // A quarantine is a removal that already started and was cut short (a
        // process killed, a scanner or a child holding a file open on Windows):
        // it may have deleted the manifest pointer, so it is finished without
        // reading the manifest, which would fail it for ever.
        let eligible = if existing_quarantine {
            true
        } else {
            let committed = read_committed_manifest(&session, session_id, ChainCheck::Full)?;
            session_is_eligible(
                &session,
                session_id,
                &committed,
                now_unix_seconds,
                abandon.is_some(),
            )?
        };
        if !eligible {
            return Ok(CleanOutcome::Ineligible);
        }
        if dry_run {
            return Ok(CleanOutcome::Eligible);
        }
        let mut budget = CleanupBudget::default();
        validate_owned_tree(&session, 0, &mut budget)?;
        // The root job index must not outlive the jobs it names.
        self.unindex_session_jobs(&session);
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
            .map_err(map_committed_io)?;
        let bucket_name = session_bucket(session_id);
        if !index.try_exists(&bucket_name).map_err(map_storage_io)? {
            return Ok(None);
        }
        let bucket = index
            .open_dir_nofollow(&bucket_name)
            .map_err(map_committed_io)?;
        if !bucket
            .try_exists(session_id.as_str())
            .map_err(map_storage_io)?
        {
            return Ok(None);
        }
        // Another remover may be deleting this marker right now: it is claimed
        // (Windows lock, sharing or access errors: `Busy`, tried again by the
        // caller), or already gone (no registration left to claim).
        let marker_lock = match open_regular_file(&bucket, session_id.as_str(), true) {
            Ok(file) => file.into_std(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) if is_held_by_a_remover(&error) => return Err(SessionStorageError::Busy),
            Err(error) => return Err(map_committed_io(error)),
        };
        let marker = match read_marker(&bucket, session_id.as_str())? {
            MarkerRead::Present(marker) => marker,
            MarkerRead::Gone => return Ok(None),
            MarkerRead::Held => return Err(SessionStorageError::Busy),
        };
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

/// Whether a session folder that is not already quarantined may be cleaned
/// now: a closed or expired published session, or a first generation that was
/// never published and is either idle for the full interval or, for
/// `abandon`, the failed open's own (#277). A published session is never
/// eligible under `abandon`.
fn session_is_eligible(
    session: &Dir,
    session_id: &SessionId,
    committed: &CommittedManifest,
    now_unix_seconds: u64,
    abandon: bool,
) -> Result<bool, SessionStorageError> {
    if let Some(record) = &committed.manifest.lifecycle {
        // A published session is never an abandoned open.
        if abandon {
            return Ok(false);
        }
        let status = record
            .clone()
            .to_status(session_id.clone(), committed.manifest.generation)?;
        return Ok(
            status.phase() == SessionPhase::Closed || status.lifetime().expired(now_unix_seconds)
        );
    }
    if committed.manifest.generation != 0 {
        return Err(SessionStorageError::IntegrityFailure);
    }
    if abandon {
        // Never published: the open that made it has failed.
        return Ok(true);
    }
    let generations = session
        .open_dir_nofollow(GENERATIONS_DIRECTORY)
        .map_err(map_committed_io)?;
    let initial = open_regular_file(&generations, INITIAL_GENERATION_FILE, false)
        .map_err(map_committed_io)?;
    let modified = initial
        .metadata()
        .map_err(map_storage_io)?
        .modified()
        .map_err(map_storage_io)?
        .into_std()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| SessionStorageError::IntegrityFailure)?
        .as_secs();
    Ok(now_unix_seconds.saturating_sub(modified) >= SessionLifetime::IDLE_SECONDS)
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
                .map_err(map_committed_io)?;
            validate_owned_tree(&child, depth + 1, budget)?;
        } else if kind.is_file() {
            let name = name.to_str().ok_or(SessionStorageError::IntegrityFailure)?;
            let file = open_regular_file(directory, name, false).map_err(map_committed_io)?;
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
