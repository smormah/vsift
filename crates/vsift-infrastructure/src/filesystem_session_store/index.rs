//! The bounded session index: registration before initialization and bucket scans.
//!
//! ```text
//! session-index/<bucket>/<session-id>   marker: who registered the session, when
//! session-index/.registering.tmp        the staged marker of a registration in progress
//! ```

use std::{
    io::{self, Write},
    path::Path,
};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use vsift_application::SessionStorageError;
use vsift_domain::{OperationId, SessionId};

use super::map_committed_io;
use super::{
    FilesystemSessionStore, SESSION_INDEX_DIRECTORY, STORAGE_SCHEMA_VERSION, SessionIndexMarker,
    SessionIndexPage, SessionRegistration, create_private_child_directory, map_lock_error,
    map_storage_io, open_regular_file, session_bucket, stored::read_versioned_json_file,
};
use crate::{
    fault_point::{FaultPlan, FaultPoint},
    file_lock::HeldFileLock,
};

/// The staged marker of the registration in progress, in `session-index/`.
///
/// A marker is written here, flushed and renamed into its bucket, so a
/// process killed while it registers leaves no marker or a whole one. A
/// marker created in place and killed before its bytes were written was an
/// empty file, and every later scan of its bucket, and its cleanup, failed as
/// an integrity failure (issue #197). Every registration holds the root's
/// initialization lock, so at most one staged marker exists, and the next
/// registration replaces one a killed registrar left. No scan reads
/// `session-index/` itself, only its bucket directories.
const STAGED_MARKER: &str = ".registering.tmp";

impl FilesystemSessionStore {
    /// Registers a new disposable session before initialization and source I/O.
    ///
    /// A SHA-256 bucket index caps each scan at 256 markers. The held marker lock
    /// prevents expiry cleanup of a suspended or slow opener across processes.
    /// An open that was interrupted (a crash) or whose removal of its own
    /// registration failed remains registered for bounded later cleanup; a failed
    /// open that is still running removes it at once
    /// ([`Self::abandon_unpublished_open`]). The
    /// marker is staged in `session-index/`, flushed and renamed into its
    /// bucket, so an interrupted registration never leaves a torn one.
    ///
    /// # Errors
    ///
    /// Rejects duplicate IDs, malformed index entries, contention, and a full bucket.
    pub fn register_session(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        now_unix_seconds: u64,
    ) -> Result<SessionRegistration, SessionStorageError> {
        self.revalidate_root()?;
        let initialization_lock = self.try_root_initialization_lock()?;
        if !self
            .root
            .try_exists(SESSION_INDEX_DIRECTORY)
            .map_err(map_storage_io)?
        {
            create_private_child_directory(&self.root, Path::new(SESSION_INDEX_DIRECTORY))
                .map_err(map_storage_io)?;
        }
        let index = self
            .root
            .open_dir_nofollow(SESSION_INDEX_DIRECTORY)
            .map_err(map_committed_io)?;
        let bucket_name = session_bucket(session_id);
        if !index.try_exists(&bucket_name).map_err(map_storage_io)? {
            create_private_child_directory(&index, Path::new(&bucket_name))
                .map_err(map_storage_io)?;
        }
        let bucket = index
            .open_dir_nofollow(&bucket_name)
            .map_err(map_committed_io)?;
        let mut count = 0_u16;
        let mut registered = false;
        for entry in bucket.entries().map_err(map_storage_io)? {
            let entry = entry.map_err(map_storage_io)?;
            count = count
                .checked_add(1)
                .ok_or(SessionStorageError::CapacityExhausted)?;
            if count >= 256 {
                return Err(SessionStorageError::CapacityExhausted);
            }
            let name = entry.file_name();
            let name = name.to_str().ok_or(SessionStorageError::IntegrityFailure)?;
            SessionId::parse(name).map_err(|_| SessionStorageError::IntegrityFailure)?;
            registered |= name == session_id.as_str();
        }
        if registered {
            return Err(SessionStorageError::StateConflict);
        }
        let marker = SessionIndexMarker {
            schema_version: STORAGE_SCHEMA_VERSION,
            session_id: session_id.as_str().to_owned(),
            operation_id: operation_id.as_str().to_owned(),
            registered_at_unix_seconds: now_unix_seconds,
        };
        let bytes = serde_json::to_vec(&marker).map_err(|_| SessionStorageError::Io)?;
        let plan = FaultPlan::from_environment();
        stage_marker(&index, &bytes, &plan)?;
        // The name is free: this process holds the initialization lock every
        // registration takes, and it found no marker of this id above.
        index
            .rename(STAGED_MARKER, &bucket, session_id.as_str())
            .map_err(map_storage_io)?;
        plan.reach(FaultPoint::RegistrationMarkerRename);
        let marker_lock = HeldFileLock::try_shared(
            open_regular_file(&bucket, session_id.as_str(), true)
                .map_err(map_storage_io)?
                .into_std(),
        )
        .map_err(map_lock_error)?;
        // Registration returns the long-lived marker hold, so it must know the
        // short-lived root lock is free before returning rather than rely on drop.
        initialization_lock.release().map_err(map_storage_io)?;
        Ok(SessionRegistration {
            _marker_lock: marker_lock,
        })
    }

    /// Scans one fixed hash bucket without loading the entire root.
    ///
    /// Every registered session appears in exactly one bucket. At most 256 markers
    /// are inspected, and the next bucket is an explicit continuation cursor.
    ///
    /// # Errors
    ///
    /// Rejects an invalid cursor or malformed ownership marker.
    pub fn scan_session_bucket(
        &self,
        bucket: u16,
    ) -> Result<SessionIndexPage, SessionStorageError> {
        if bucket > 255 {
            return Err(SessionStorageError::StateConflict);
        }
        self.revalidate_root()?;
        let _initialization_lock = self.try_root_initialization_lock()?;
        let next_bucket = if bucket == 255 {
            None
        } else {
            Some(bucket + 1)
        };
        if !self
            .root
            .try_exists(SESSION_INDEX_DIRECTORY)
            .map_err(map_storage_io)?
        {
            return Ok(SessionIndexPage {
                session_ids: Vec::new(),
                next_bucket,
            });
        }
        let index = self
            .root
            .open_dir_nofollow(SESSION_INDEX_DIRECTORY)
            .map_err(map_committed_io)?;
        let name = format!("{bucket:02x}");
        if !index.try_exists(&name).map_err(map_storage_io)? {
            return Ok(SessionIndexPage {
                session_ids: Vec::new(),
                next_bucket,
            });
        }
        let directory = index.open_dir_nofollow(&name).map_err(map_committed_io)?;
        let mut session_ids = Vec::new();
        for entry in directory.entries().map_err(map_storage_io)? {
            let entry = entry.map_err(map_storage_io)?;
            if session_ids.len() >= 256 {
                return Err(SessionStorageError::CapacityExhausted);
            }
            let name = entry.file_name();
            let name = name.to_str().ok_or(SessionStorageError::IntegrityFailure)?;
            let session_id =
                SessionId::parse(name).map_err(|_| SessionStorageError::IntegrityFailure)?;
            if session_bucket(&session_id) != format!("{bucket:02x}") {
                return Err(SessionStorageError::IntegrityFailure);
            }
            let marker = read_versioned_json_file::<SessionIndexMarker>(&directory, name)?;
            if marker.session_id != name || OperationId::parse(&marker.operation_id).is_err() {
                return Err(SessionStorageError::IntegrityFailure);
            }
            session_ids.push(session_id);
        }
        session_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        Ok(SessionIndexPage {
            session_ids,
            next_bucket,
        })
    }
}

/// Writes `bytes` to [`STAGED_MARKER`] in `index`, created new after a
/// leftover of a killed registration is removed, and flushes it. The caller
/// holds the root's initialization lock.
fn stage_marker(index: &Dir, bytes: &[u8], plan: &FaultPlan) -> Result<(), SessionStorageError> {
    match index.remove_file(STAGED_MARKER) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(map_storage_io(error)),
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let mut file = index
        .open_with(STAGED_MARKER, &options)
        .map_err(map_storage_io)?;
    plan.reach(FaultPoint::RegistrationMarkerCreate);
    file.write_all(bytes).map_err(map_storage_io)?;
    file.sync_all().map_err(map_storage_io)
}
