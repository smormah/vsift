//! Location and first-use provisioning of the per-user disposable-session root.
//!
//! Hosts select a session root explicitly or accept the platform's per-user
//! cache. Opening never adopts an unowned directory: an existing root must pass
//! the store's ownership and privacy validation, and a missing root is created
//! only when the caller is about to open a session.

use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use crate::{FilesystemSessionStore, SessionStoreOpenError};

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
/// than silently built. A concurrent creator is tolerated: if another process
/// provisions the root first, the existing root is validated and opened.
///
/// # Errors
///
/// Returns [`SessionRootError`] for a relative or parentless root, an
/// unavailable parent, or any store validation failure.
pub fn open_session_root(
    root: &Path,
    provisioning: SessionRootProvisioning,
) -> Result<Option<FilesystemSessionStore>, SessionRootError> {
    if !root.is_absolute() {
        return Err(SessionRootError::RootMustBeAbsolute);
    }
    if root.exists() {
        return FilesystemSessionStore::open_existing(root)
            .map(Some)
            .map_err(SessionRootError::Store);
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
        Err(SessionStoreOpenError::RootAlreadyExists) => {
            FilesystemSessionStore::open_existing(root)
                .map(Some)
                .map_err(SessionRootError::Store)
        }
        Err(error) => Err(SessionRootError::Store(error)),
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
