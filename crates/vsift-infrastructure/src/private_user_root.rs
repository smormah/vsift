//! Shared private-root checks for per-user dependency configuration and managed data.

use std::{fs, io, path::Path};

use cap_fs_ext::DirExt;
use cap_std::fs::{Dir, DirBuilder};

/// A failure to obtain a private, non-link per-user directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateRootError {
    Unavailable,
    UnsafeStorage,
    Busy,
    Io,
}

/// Opens a private root, creating missing per-user parent directories when authorized.
pub(crate) fn open_private_root(
    path: &Path,
    create: bool,
) -> Result<Option<Dir>, PrivateRootError> {
    open_private_root_with_creation(path, create).map(|root| root.map(|(directory, _)| directory))
}

/// Returns whether this call created the final private directory.
pub(crate) fn open_private_root_with_creation(
    path: &Path,
    create: bool,
) -> Result<Option<(Dir, bool)>, PrivateRootError> {
    let mut created = false;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(PrivateRootError::UnsafeStorage),
        Err(error) if error.kind() == io::ErrorKind::NotFound && !create => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or(PrivateRootError::Unavailable)?;
            fs::create_dir_all(parent).map_err(|_| PrivateRootError::Io)?;
            #[allow(unused_mut, reason = "Unix configures the creation mode")]
            let mut builder = DirBuilder::new();
            #[cfg(unix)]
            {
                use cap_std::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            let parent_dir = Dir::open_ambient_dir(parent, cap_std::ambient_authority())
                .map_err(|_| PrivateRootError::Io)?;
            parent_dir
                .create_dir_with(
                    Path::new(path.file_name().ok_or(PrivateRootError::Unavailable)?),
                    &builder,
                )
                .map_err(|error| {
                    if error.kind() == io::ErrorKind::AlreadyExists {
                        PrivateRootError::Busy
                    } else {
                        PrivateRootError::Io
                    }
                })?;
            created = true;
        }
        Err(_) => return Err(PrivateRootError::Io),
    }
    let parent = path.parent().ok_or(PrivateRootError::Unavailable)?;
    let parent_dir = Dir::open_ambient_dir(parent, cap_std::ambient_authority())
        .map_err(|_| PrivateRootError::Io)?;
    let root = parent_dir
        .open_dir_nofollow(path.file_name().ok_or(PrivateRootError::Unavailable)?)
        .map_err(|_| PrivateRootError::UnsafeStorage)?;
    let path_metadata = fs::symlink_metadata(path).map_err(|_| PrivateRootError::Io)?;
    validate_same_directory_object(&path_metadata, &root)?;
    validate_private_root(path, &root)?;
    Ok(Some((root, created)))
}

#[cfg(unix)]
pub(crate) fn validate_same_directory_object(
    path_metadata: &fs::Metadata,
    held: &Dir,
) -> Result<(), PrivateRootError> {
    use cap_std::fs::MetadataExt as _;
    use std::os::unix::fs::MetadataExt;
    let held_metadata = held.dir_metadata().map_err(|_| PrivateRootError::Io)?;
    if !path_metadata.is_dir()
        || path_metadata.dev() != held_metadata.dev()
        || path_metadata.ino() != held_metadata.ino()
    {
        return Err(PrivateRootError::UnsafeStorage);
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn validate_same_directory_object(
    path_metadata: &fs::Metadata,
    held: &Dir,
) -> Result<(), PrivateRootError> {
    let held_metadata = held.dir_metadata().map_err(|_| PrivateRootError::Io)?;
    // A held Windows directory handle prevents replacement; stable metadata
    // does not expose its file index for an additional identity comparison.
    if !path_metadata.is_dir() || !held_metadata.is_dir() {
        return Err(PrivateRootError::UnsafeStorage);
    }
    Ok(())
}

/// Checks that a name still opens the same held directory before cleanup.
pub(crate) fn validate_same_held_directory(
    at_name: &Dir,
    held: &Dir,
) -> Result<(), PrivateRootError> {
    let named = at_name.dir_metadata().map_err(|_| PrivateRootError::Io)?;
    let original = held.dir_metadata().map_err(|_| PrivateRootError::Io)?;
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt as _;
        if !named.is_dir()
            || !original.is_dir()
            || named.dev() != original.dev()
            || named.ino() != original.ino()
        {
            return Err(PrivateRootError::UnsafeStorage);
        }
    }
    #[cfg(windows)]
    {
        // Both no-follow handles stay open until cleanup on the tested NTFS
        // profile, which refuses replacement of an open directory.
        if !named.is_dir() || !original.is_dir() {
            return Err(PrivateRootError::UnsafeStorage);
        }
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn validate_private_root(_path: &Path, root: &Dir) -> Result<(), PrivateRootError> {
    use cap_std::fs::{MetadataExt as _, PermissionsExt};
    let metadata = root.dir_metadata().map_err(|_| PrivateRootError::Io)?;
    if metadata.permissions().mode() & 0o077 != 0
        || metadata.uid() != rustix::process::getuid().as_raw()
    {
        return Err(PrivateRootError::UnsafeStorage);
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn validate_private_root(path: &Path, _root: &Dir) -> Result<(), PrivateRootError> {
    use windows_acl::{
        acl::{ACL, AceType},
        helper::{current_user, name_to_sid, sid_to_string},
    };
    let username = current_user().ok_or(PrivateRootError::UnsafeStorage)?;
    let user_sid = name_to_sid(&username, None)
        .ok()
        .and_then(|mut sid| sid_to_string(sid.as_mut_ptr().cast()).ok())
        .ok_or(PrivateRootError::UnsafeStorage)?;
    let path = path.to_str().ok_or(PrivateRootError::UnsafeStorage)?;
    let acl = ACL::from_file_path(path, false).map_err(|_| PrivateRootError::UnsafeStorage)?;
    let entries = acl.all().map_err(|_| PrivateRootError::UnsafeStorage)?;
    let trusted = [user_sid.as_str(), "S-1-5-18", "S-1-5-32-544"];
    let current_user_allowed = entries.iter().any(|entry| {
        matches!(
            entry.entry_type,
            AceType::AccessAllow
                | AceType::AccessAllowCallback
                | AceType::AccessAllowObject
                | AceType::AccessAllowCallbackObject
        ) && entry.string_sid == user_sid
    });
    let unsafe_entry = entries.iter().any(|entry| {
        entry.entry_type == AceType::Unknown
            || (matches!(
                entry.entry_type,
                AceType::AccessAllow
                    | AceType::AccessAllowCallback
                    | AceType::AccessAllowObject
                    | AceType::AccessAllowCallbackObject
            ) && !trusted.contains(&entry.string_sid.as_str()))
    });
    if !current_user_allowed || unsafe_entry {
        return Err(PrivateRootError::UnsafeStorage);
    }
    Ok(())
}
