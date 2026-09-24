//! Location and first-use provisioning of the per-user disposable-session root.
//!
//! Hosts select a session root explicitly or accept the platform's per-user
//! cache. Opening never adopts an unowned directory: an existing root must pass
//! the store's ownership and privacy validation, and a missing root is created
//! only when the caller is about to open a session.
//!
//! Concurrent first use is expected: several processes may find the root
//! missing at once. Exactly one creates it; the others wait, within a fixed
//! bound, for that creator to finish, and adopt the root only after the same
//! full validation as any other open (issue #131).

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime},
};

use crate::{
    FilesystemSessionStore, SessionStoreOpenError,
    filesystem_session_store::{RootProvisioningState, root_provisioning_state},
};

/// Longest time an open waits for a concurrent creator to finish a root.
///
/// Provisioning writes a handful of small files and normally completes within
/// milliseconds; the bound only absorbs a slow disk or a busy machine.
const PROVISIONING_WAIT: Duration = Duration::from_secs(5);
/// How recently a root must have changed to be treated as just created while
/// it holds nothing yet beyond the creator's first steps.
const PROVISIONING_RECENT: Duration = Duration::from_secs(10);
/// First pause between validation attempts; it doubles up to the maximum.
const INITIAL_BACKOFF: Duration = Duration::from_millis(2);
/// Longest pause between validation attempts.
const MAX_BACKOFF: Duration = Duration::from_millis(50);

/// Whether opening a session root may create it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionRootProvisioning {
    /// Open an existing root only; a missing root is reported as absent.
    ExistingOnly,
    /// Create a missing root, and a missing private parent directory, first.
    CreateIfMissing,
}

/// Why a session root could not be opened or provisioned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionRootError {
    /// The root path is relative.
    RootMustBeAbsolute,
    /// The root, or the parent it would be created in, has no parent directory.
    RootWithoutParent,
    /// The directory that must contain the root's parent is missing, or the
    /// parent could not be created.
    ParentUnavailable,
    /// Another process was still provisioning the root when the bounded wait
    /// ended. Retrying later is expected to succeed.
    ProvisioningInProgress,
    /// The store rejected the root while opening or provisioning it.
    Store(SessionStoreOpenError),
}

impl std::fmt::Display for SessionRootError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RootMustBeAbsolute => formatter.write_str("session root must be absolute"),
            Self::RootWithoutParent => formatter.write_str("session root has no parent directory"),
            Self::ParentUnavailable => {
                formatter.write_str("session root parent directory is unavailable")
            }
            Self::ProvisioningInProgress => {
                formatter.write_str("session root is still being created by another process")
            }
            Self::Store(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SessionRootError {}

/// Returns the platform's per-user cache location for disposable sessions.
///
/// The location is derived from the user's environment only, never from the
/// working directory or a project file. `None` means the platform variable that
/// names the per-user cache is not set.
#[must_use]
pub fn platform_session_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join("VSift-sessions"))
    }
    #[cfg(target_os = "macos")]
    {
        env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches/VSift-sessions"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .map(|base| base.join("vsift-sessions"))
    }
}

/// Opens the session store at `root`, provisioning it first when allowed.
///
/// Returns `Ok(None)` only for [`SessionRootProvisioning::ExistingOnly`] when the
/// root does not exist, so read-only operations never create state. When a
/// missing root is created, one missing parent directory is created with
/// owner-only permissions on Unix; deeper missing ancestry is refused rather
/// than silently built.
///
/// Concurrent creators are safe. The store's exclusive directory creation
/// elects one creator, which holds a provisioning lock inside the root until
/// its ownership marker is complete. Any open that finds the root present but
/// failing ownership or layout validation checks for that creator: while the
/// lock is held, or while a just-created root holds nothing beyond the
/// creator's first steps, it validates again with a short, doubling backoff for
/// at most five seconds. The root is adopted only when the full validation of
/// [`FilesystemSessionStore::open_existing`] passes. A root with no creator at
/// work (an old unmarked directory, one with foreign content, a foreign or
/// linked marker) is rejected at once with the store's own error.
///
/// Opening an existing root takes no lock, so one open never makes another
/// busy; the only wait is for a creator that is demonstrably active. Waiting
/// blocks the calling thread. Session-level writers (registration, admission)
/// keep their non-blocking locks and still report contention as busy.
///
/// # Errors
///
/// Returns [`SessionRootError`] for a relative or parentless root, an
/// unavailable parent, any store validation failure, or
/// [`SessionRootError::ProvisioningInProgress`] when a creator still holds the
/// provisioning lock after the bounded wait.
pub fn open_session_root(
    root: &Path,
    provisioning: SessionRootProvisioning,
) -> Result<Option<FilesystemSessionStore>, SessionRootError> {
    open_session_root_within(root, provisioning, PROVISIONING_WAIT)
}

/// [`open_session_root`] with an explicit wait bound, so tests can exercise
/// the bound without spending the production five seconds.
pub(crate) fn open_session_root_within(
    root: &Path,
    provisioning: SessionRootProvisioning,
    wait: Duration,
) -> Result<Option<FilesystemSessionStore>, SessionRootError> {
    if !root.is_absolute() {
        return Err(SessionRootError::RootMustBeAbsolute);
    }
    if root.exists() {
        return adopt_existing_root(root, wait).map(Some);
    }
    if provisioning == SessionRootProvisioning::ExistingOnly {
        return Ok(None);
    }
    let parent = root.parent().ok_or(SessionRootError::RootWithoutParent)?;
    if !parent.exists() {
        let grandparent = parent.parent().ok_or(SessionRootError::RootWithoutParent)?;
        if !grandparent.is_dir() {
            return Err(SessionRootError::ParentUnavailable);
        }
        match create_private_directory(parent) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(SessionRootError::ParentUnavailable),
        }
    }
    match FilesystemSessionStore::provision_default(root) {
        Ok(store) => Ok(Some(store)),
        Err(SessionStoreOpenError::RootAlreadyExists) => adopt_existing_root(root, wait).map(Some),
        Err(error) => Err(SessionRootError::Store(error)),
    }
}

/// Opens an existing root, waiting within `wait` for an active creator.
///
/// Only an ownership or layout failure can come from a root that is still
/// being provisioned; every other failure is final at once. Each attempt
/// repeats the complete validation, so nothing observed while waiting is
/// trusted on its own.
fn adopt_existing_root(
    root: &Path,
    wait: Duration,
) -> Result<FilesystemSessionStore, SessionRootError> {
    let started = Instant::now();
    let mut backoff = INITIAL_BACKOFF;
    loop {
        let rejection = match FilesystemSessionStore::open_existing(root) {
            Ok(store) => return Ok(store),
            Err(
                rejection @ (SessionStoreOpenError::InvalidOwnership
                | SessionStoreOpenError::InvalidLayout),
            ) => rejection,
            Err(error) => return Err(SessionRootError::Store(error)),
        };
        let state = root_provisioning_state(root, SystemTime::now(), PROVISIONING_RECENT);
        if state == RootProvisioningState::Settled {
            // The creator may have finished between the failed validation and
            // the probe, so a settled root is validated once more.
            return FilesystemSessionStore::open_existing(root).map_err(SessionRootError::Store);
        }
        if started.elapsed() >= wait {
            return Err(match state {
                RootProvisioningState::InProgress => SessionRootError::ProvisioningInProgress,
                RootProvisioningState::Starting | RootProvisioningState::Settled => {
                    SessionRootError::Store(rejection)
                }
            });
        }
        thread::sleep(backoff);
        backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder.create(path)
}

#[cfg(windows)]
fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{SessionRootError, SessionRootProvisioning, open_session_root};

    #[test]
    fn relative_roots_are_refused_before_any_filesystem_access() {
        for provisioning in [
            SessionRootProvisioning::ExistingOnly,
            SessionRootProvisioning::CreateIfMissing,
        ] {
            assert!(matches!(
                open_session_root(Path::new("relative-sessions"), provisioning),
                Err(SessionRootError::RootMustBeAbsolute)
            ));
        }
    }
}
