//! An operator's input root: the only place a worker request's source and
//! transcript paths may name (P11, ADR 0021 section 9; S-01, S-02, SEC-05).
//!
//! The root is an explicit absolute directory, canonicalised once and held
//! open as a capability. A request path is never joined to it as a string:
//! it is checked against a narrow relative grammar (the same rules as the
//! contract's `RelativeInputPath`: `/`-separated components, none empty,
//! `.` or `..`, no `\`, `:`, control character, trailing dot or space or
//! Windows device name, at most 32 components and 1,024 bytes), then opened
//! one component at a time relative to the held directory without following
//! any link. A link anywhere on the path is refused, whether it points in or
//! out of the root, so no spelling, link or junction can reach a file outside
//! it, and what is opened is the object the name named when it was opened.
//! The result must be a regular file with a single link, so a hard link to a
//! file outside the root is refused too.

use std::{
    error::Error,
    fmt,
    path::{Component, Path},
};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, File, Metadata, OpenOptions};

/// Longest request path, in bytes.
pub const MAX_INPUT_PATH_BYTES: usize = 1_024;
/// Most components of a request path.
pub const MAX_INPUT_PATH_COMPONENTS: usize = 32;

/// Why an input root could not be opened.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputRootError {
    /// The root must be an absolute, local path.
    NotAbsolute,
    /// The root does not exist or cannot be opened as a directory.
    Unavailable,
}

impl fmt::Display for InputRootError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotAbsolute => "the input root must be an absolute local path",
            Self::Unavailable => "the input root cannot be opened as a directory",
        })
    }
}

impl Error for InputRootError {}

/// Why a request path could not be opened inside the input root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContainedPathError {
    /// The path breaks the relative grammar: absolute, `..`, a drive or
    /// stream (`:`), `\`, a device name, a control character, too long.
    Invalid,
    /// A component is a symbolic link, junction or other reparse point: a
    /// way out of the root is never followed (`path_outside_input_root`).
    Link,
    /// Nothing exists at the path.
    NotFound,
    /// The path names a directory or special file, or a file with several
    /// hard links.
    NotRegularFile,
    /// The file could not be opened or inspected.
    Io,
}

impl fmt::Display for ContainedPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "the input path is not a valid relative path",
            Self::Link => "the input path goes through a link",
            Self::NotFound => "nothing exists at the input path",
            Self::NotRegularFile => "the input path is not a single-link regular file",
            Self::Io => "the input path could not be opened",
        })
    }
}

impl Error for ContainedPathError {}

/// An explicit, canonicalised, held input directory.
pub struct InputRoot {
    directory: Dir,
}

/// A regular file opened inside an [`InputRoot`], with the metadata read
/// from the opened handle.
pub struct ContainedFile {
    pub(crate) file: File,
    pub(crate) metadata: Metadata,
}

impl ContainedFile {
    /// The file's size in bytes, as read from the opened handle.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.metadata.len()
    }

    /// Whether the file is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.metadata.len() == 0
    }
}

impl InputRoot {
    /// Canonicalises `path` once and holds it open as the root every request
    /// path is resolved in.
    ///
    /// # Errors
    ///
    /// [`InputRootError`] for a relative or non-local path, or one that is
    /// not an openable directory.
    pub fn open(path: &Path) -> Result<Self, InputRootError> {
        if !path.is_absolute() || !crate::source_snapshot::is_local_path(path) {
            return Err(InputRootError::NotAbsolute);
        }
        let canonical = std::fs::canonicalize(path).map_err(|_| InputRootError::Unavailable)?;
        if !crate::source_snapshot::is_local_path(&canonical) {
            return Err(InputRootError::NotAbsolute);
        }
        let directory = Dir::open_ambient_dir(&canonical, cap_std::ambient_authority())
            .map_err(|_| InputRootError::Unavailable)?;
        Ok(Self { directory })
    }

    /// Opens the regular file `relative` names inside the root, one
    /// component at a time, following no link.
    ///
    /// # Errors
    ///
    /// [`ContainedPathError`]; nothing outside the root is ever opened.
    pub fn open_file(&self, relative: &str) -> Result<ContainedFile, ContainedPathError> {
        let components = relative_components(relative)?;
        let Some((name, parents)) = components.split_last() else {
            return Err(ContainedPathError::Invalid);
        };
        let mut directory = self
            .directory
            .try_clone()
            .map_err(|_| ContainedPathError::Io)?;
        for parent in parents {
            if !refuse_link(&directory, parent)?.is_dir() {
                return Err(ContainedPathError::NotFound);
            }
            directory = directory
                .open_dir_nofollow(parent)
                .map_err(|error| open_error(&error))?;
        }
        if !refuse_link(&directory, name)?.is_file() {
            return Err(ContainedPathError::NotRegularFile);
        }
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = directory
            .open_with(name, &options)
            .map_err(|error| open_error(&error))?;
        let metadata = file.metadata().map_err(|_| ContainedPathError::Io)?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(ContainedPathError::NotRegularFile);
        }
        Ok(ContainedFile { file, metadata })
    }
}

/// The components of a request path, checked against the relative grammar.
fn relative_components(relative: &str) -> Result<Vec<&str>, ContainedPathError> {
    if relative.is_empty() || relative.len() > MAX_INPUT_PATH_BYTES {
        return Err(ContainedPathError::Invalid);
    }
    let components: Vec<&str> = relative.split('/').collect();
    if components.len() > MAX_INPUT_PATH_COMPONENTS
        || components
            .iter()
            .any(|component| !valid_component(component))
    {
        return Err(ContainedPathError::Invalid);
    }
    // The platform must read every component as one plain name too.
    if !Path::new(relative)
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(ContainedPathError::Invalid);
    }
    Ok(components)
}

fn valid_component(component: &str) -> bool {
    !component.is_empty()
        && component != "."
        && component != ".."
        && !component.ends_with(['.', ' '])
        && !component.chars().any(|character| {
            character.is_control()
                || matches!(character, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        && !crate::source_snapshot::is_reserved_device_name(component)
}

/// Refuses a component that is a link or other reparse point, without
/// following it, and returns what it is otherwise. The open that follows
/// refuses a link too, so a component swapped for one in between is still
/// never followed.
fn refuse_link(directory: &Dir, name: &str) -> Result<Metadata, ContainedPathError> {
    let metadata = directory
        .symlink_metadata(name)
        .map_err(|error| open_error(&error))?;
    if metadata.file_type().is_symlink() {
        return Err(ContainedPathError::Link);
    }
    Ok(metadata)
}

fn open_error(error: &std::io::Error) -> ContainedPathError {
    match error.kind() {
        std::io::ErrorKind::NotFound => ContainedPathError::NotFound,
        std::io::ErrorKind::NotADirectory => ContainedPathError::NotRegularFile,
        _ => ContainedPathError::Io,
    }
}
