//! The bounded session index: registration before initialization and bucket scans.

use std::{io, path::Path};

use cap_fs_ext::DirExt;
use vsift_application::SessionStorageError;
use vsift_domain::{OperationId, SessionId};

use super::map_committed_io;
use super::{
    FilesystemSessionStore, SESSION_INDEX_DIRECTORY, STORAGE_SCHEMA_VERSION, SessionIndexMarker,
    SessionIndexPage, SessionRegistration, create_private_child_directory, create_regular_file,
    map_lock_error, map_storage_io, open_regular_file, session_bucket,
    stored::read_versioned_json_file,
};
use crate::file_lock::HeldFileLock;

impl FilesystemSessionStore {
    /// Registers a new disposable session before initialization and source I/O.
    ///
    /// A SHA-256 bucket index caps each scan at 256 markers. The held marker lock
    /// prevents expiry cleanup of a suspended or slow opener across processes.
    /// An interrupted open remains registered for bounded later cleanup.
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
        }
        let marker = SessionIndexMarker {
            schema_version: STORAGE_SCHEMA_VERSION,
            session_id: session_id.as_str().to_owned(),
            operation_id: operation_id.as_str().to_owned(),
            registered_at_unix_seconds: now_unix_seconds,
        };
        let bytes = serde_json::to_vec(&marker).map_err(|_| SessionStorageError::Io)?;
        create_regular_file(&bucket, session_id.as_str(), &bytes).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                SessionStorageError::StateConflict
            } else {
                map_storage_io(error)
            }
        })?;
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
