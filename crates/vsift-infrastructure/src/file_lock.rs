//! Advisory OS file locks that are always released explicitly.

use std::{fs, io};

/// An advisory OS lock on an open file, explicitly unlocked when released.
///
/// On Unix the standard library implements file locks with `flock`, and an
/// `flock` lock belongs to the open file description rather than to one
/// descriptor. A child process spawned by any thread in this process holds
/// duplicates of every open descriptor between `fork` and `exec`. Merely closing
/// the locked file during that window leaves the lock held until the child calls
/// `exec`, so an immediate re-acquisition by this or another process reports
/// `Busy` even though the owner has let go (issue #66). An explicit unlock
/// releases the lock through any duplicate, so every holder unlocks before its
/// file closes. On Windows the explicit unlock is equivalent to closing.
#[derive(Debug)]
pub(crate) struct HeldFileLock {
    file: Option<fs::File>,
}

impl HeldFileLock {
    /// Takes an exclusive lock without waiting.
    ///
    /// # Errors
    ///
    /// Returns `WouldBlock` when another holder has the lock, or the OS error.
    pub(crate) fn try_exclusive(file: fs::File) -> Result<Self, fs::TryLockError> {
        file.try_lock()?;
        Ok(Self { file: Some(file) })
    }

    /// Takes a shared lock without waiting.
    ///
    /// # Errors
    ///
    /// Returns `WouldBlock` when an exclusive holder has the lock, or the OS error.
    pub(crate) fn try_shared(file: fs::File) -> Result<Self, fs::TryLockError> {
        file.try_lock_shared()?;
        Ok(Self { file: Some(file) })
    }

    /// Releases the lock now and reports whether the OS accepted the unlock.
    ///
    /// Dropping the value also unlocks, but cannot report a failure; use this
    /// where a caller must know the lock is free before continuing.
    ///
    /// # Errors
    ///
    /// Returns the OS error from the unlock. The file is closed either way.
    pub(crate) fn release(mut self) -> io::Result<()> {
        self.file.take().map_or(Ok(()), |file| file.unlock())
    }

    /// Duplicates the locked descriptor, as a child between `fork` and `exec`
    /// would, so tests can prove release does not depend on closing every copy.
    #[cfg(test)]
    pub(crate) fn duplicate_descriptor(&self) -> io::Result<fs::File> {
        self.file
            .as_ref()
            .ok_or_else(|| io::Error::other("lock already released"))?
            .try_clone()
    }
}

impl Drop for HeldFileLock {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            // A drop cannot propagate the error; closing the file follows and is
            // the operating system's last-resort release of the lock.
            let _ = file.unlock();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::HeldFileLock;

    type TestResult = Result<(), Box<dyn Error>>;

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct LockFile {
        directory: PathBuf,
        path: PathBuf,
    }

    impl LockFile {
        fn new() -> Result<Self, Box<dyn Error>> {
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "vsift-file-lock-{}-{nanos}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&directory)?;
            let path = directory.join("test.lock");
            fs::write(&path, b"")?;
            Ok(Self { directory, path })
        }

        fn open(&self) -> std::io::Result<fs::File> {
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.path)
        }
    }

    impl Drop for LockFile {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn exclusive_lock_excludes_others_until_dropped() -> TestResult {
        let file = LockFile::new()?;
        let held = HeldFileLock::try_exclusive(file.open()?)?;
        assert!(matches!(
            HeldFileLock::try_shared(file.open()?),
            Err(fs::TryLockError::WouldBlock)
        ));

        drop(held);

        drop(HeldFileLock::try_exclusive(file.open()?)?);
        Ok(())
    }

    #[test]
    fn shared_locks_coexist_and_exclude_an_exclusive_holder() -> TestResult {
        let file = LockFile::new()?;
        let first = HeldFileLock::try_shared(file.open()?)?;
        let second = HeldFileLock::try_shared(file.open()?)?;
        assert!(matches!(
            HeldFileLock::try_exclusive(file.open()?),
            Err(fs::TryLockError::WouldBlock)
        ));

        drop(first);
        drop(second);

        HeldFileLock::try_exclusive(file.open()?)?.release()?;
        Ok(())
    }

    /// Models issue #66: a duplicate descriptor that outlives the holder, as a
    /// child between `fork` and `exec` would, must not keep the lock held.
    #[test]
    fn dropping_releases_the_lock_despite_a_surviving_duplicate() -> TestResult {
        let file = LockFile::new()?;
        let locked = file.open()?;
        let duplicate = locked.try_clone()?;
        let held = HeldFileLock::try_exclusive(locked)?;

        drop(held);

        drop(HeldFileLock::try_exclusive(file.open()?)?);
        drop(duplicate);
        Ok(())
    }

    #[test]
    fn explicit_release_frees_the_lock_despite_a_surviving_duplicate() -> TestResult {
        let file = LockFile::new()?;
        let locked = file.open()?;
        let duplicate = locked.try_clone()?;
        let held = HeldFileLock::try_shared(locked)?;

        held.release()?;

        drop(HeldFileLock::try_exclusive(file.open()?)?);
        drop(duplicate);
        Ok(())
    }
}
