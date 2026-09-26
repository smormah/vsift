//! Private per-session work directories and their bounded cleanup.

use std::{fs, io, path::Path};

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use vsift_application::SessionStorageError;
use vsift_domain::{SessionId, SessionPhase};

use super::{
    FilesystemSessionStore, MAX_REMOVED_WORK_DIRECTORIES, SESSIONS_DIRECTORY, SessionWorkDirectory,
    WORK_DIRECTORY, WORK_LOCK_FILE, WORK_PREFIX, WORK_RANDOM_BYTES, chain::read_committed_manifest,
    create_private_child_directory, lowercase_hex, map_lock_error, map_storage_io,
    open_regular_file,
};
use crate::file_lock::HeldFileLock;

impl FilesystemSessionStore {
    /// Creates a fresh private work directory inside an open session.
    ///
    /// Leftover work directories of the same session whose run has ended
    /// (their lock can be taken) are removed first, at most eight; one whose
    /// lock is held, or that is not exactly a work directory, is never
    /// touched, and links are never followed.
    ///
    /// # Errors
    ///
    /// Closed or expired sessions conflict, active cleanup is busy, and a work
    /// directory that cannot be created is a storage failure.
    pub fn session_work_directory(
        &self,
        session_id: &SessionId,
        now_unix_seconds: u64,
    ) -> Result<SessionWorkDirectory, SessionStorageError> {
        let hold = self.acquire_read(session_id)?;
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
        match create_private_child_directory(&session, Path::new(WORK_DIRECTORY)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(map_storage_io(error)),
        }
        let work = session
            .open_dir_nofollow(WORK_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        remove_leftover_work_directories(&work);
        let mut random = [0_u8; WORK_RANDOM_BYTES];
        getrandom::fill(&mut random).map_err(|_| SessionStorageError::Io)?;
        let name = format!("{WORK_PREFIX}{}", lowercase_hex(&random));
        create_private_child_directory(&work, Path::new(&name)).map_err(map_storage_io)?;
        let path = self
            .root_path
            .join(SESSIONS_DIRECTORY)
            .join(session_id.as_str())
            .join(WORK_DIRECTORY)
            .join(&name);
        // From here on, dropping the value removes the directory again.
        let mut directory = SessionWorkDirectory {
            path,
            owner: None,
            _hold: hold,
        };
        let lock = fs::File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(directory.path.join(WORK_LOCK_FILE))
            .map_err(map_storage_io)?;
        directory.owner = Some(HeldFileLock::try_exclusive(lock).map_err(map_lock_error)?);
        Ok(directory)
    }
}

/// Removes work directories whose run has ended: exactly named, real
/// directories whose lock file can be locked now, or that have none. A held
/// lock means a live run and the directory is kept. Failures are ignored; the
/// next run tries again.
pub(super) fn remove_leftover_work_directories(work: &Dir) {
    let Ok(entries) = work.entries() else {
        return;
    };
    let mut removed = 0;
    for entry in entries.flatten().take(256) {
        if removed == MAX_REMOVED_WORK_DIRECTORIES {
            return;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let exact = name.strip_prefix(WORK_PREFIX).is_some_and(|random| {
            random.len() == WORK_RANDOM_BYTES * 2
                && random
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        });
        if !exact || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Ok(directory) = work.open_dir_nofollow(&name) else {
            continue;
        };
        let abandoned = match open_regular_file(&directory, WORK_LOCK_FILE, true) {
            Ok(file) => HeldFileLock::try_exclusive(file.into_std())
                .is_ok_and(|held| held.release().is_ok()),
            Err(error) => error.kind() == io::ErrorKind::NotFound,
        };
        drop(directory);
        if abandoned && work.remove_dir_all(&name).is_ok() {
            removed += 1;
        }
    }
}
