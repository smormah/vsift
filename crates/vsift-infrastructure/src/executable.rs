//! Trusted executable resolution for external provider processes.

use std::{
    env,
    error::Error,
    ffi::{OsStr, OsString},
    fmt, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::PublishedManagedRuntime;

/// Describes how an executable entered the trusted invocation boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutableProvenance {
    /// The caller supplied an explicit absolute path.
    Explicit,
    /// `VSift` selected the executable from its managed runtime registry.
    Managed,
    /// Setup discovery selected the executable from an allowlisted ambient path directory.
    AmbientPath,
}

/// A canonical absolute regular-file path approved for direct execution.
///
/// An executable of a managed version carries that version's
/// [`ManagedRuntimeHold`] (and the recognizer also its model's), so whatever
/// holds the executable keeps the version in use: it cannot be removed while
/// a job that resolved it still runs. Equality compares the path and
/// provenance only.
#[derive(Clone, Debug)]
pub struct TrustedExecutable {
    path: PathBuf,
    provenance: ExecutableProvenance,
    holds: Vec<ManagedRuntimeHold>,
}

impl PartialEq for TrustedExecutable {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && self.provenance == other.provenance
    }
}

impl Eq for TrustedExecutable {}

/// One published managed version kept in use: while any clone lives, the
/// version's shared use lock is held, so removal refuses it.
#[derive(Clone)]
pub struct ManagedRuntimeHold(Arc<PublishedManagedRuntime>);

impl ManagedRuntimeHold {
    /// Keeps `runtime` in use for as long as this hold or a clone lives.
    #[must_use]
    pub fn new(runtime: PublishedManagedRuntime) -> Self {
        Self(Arc::new(runtime))
    }

    /// The held version.
    #[must_use]
    pub fn runtime(&self) -> &PublishedManagedRuntime {
        &self.0
    }
}

impl fmt::Debug for ManagedRuntimeHold {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ManagedRuntimeHold")
            .field(self.0.identity())
            .finish()
    }
}

impl TrustedExecutable {
    /// Validates an explicit executable path without searching the ambient environment.
    ///
    /// # Errors
    ///
    /// Returns a typed validation or filesystem-inspection failure when the path is not a
    /// canonicalizable absolute regular file.
    pub fn explicit(path: impl AsRef<Path>) -> Result<Self, ExecutableResolutionError> {
        Self::from_candidate(path.as_ref(), ExecutableProvenance::Explicit)
    }

    /// Validates a path selected from the managed runtime registry.
    ///
    /// # Errors
    ///
    /// Returns a typed validation or filesystem-inspection failure when the path is not a
    /// canonicalizable absolute regular file.
    pub fn managed(path: impl AsRef<Path>) -> Result<Self, ExecutableResolutionError> {
        Self::from_candidate(path.as_ref(), ExecutableProvenance::Managed)
    }

    /// The executable `name` of the held managed version, carrying the
    /// hold so the version stays in use while the executable does.
    ///
    /// # Errors
    ///
    /// [`ExecutableResolutionError::NotFound`] when the version has no file
    /// of that name, otherwise as [`Self::managed`].
    pub fn managed_in(
        hold: &ManagedRuntimeHold,
        name: &str,
    ) -> Result<Self, ExecutableResolutionError> {
        let path = hold
            .runtime()
            .file_path(name)
            .ok_or(ExecutableResolutionError::NotFound)?;
        Ok(Self::managed(path)?.retaining(hold.clone()))
    }

    /// Also keeps `hold` in use for as long as this executable lives: a
    /// recognizer retains its managed model's version this way.
    #[must_use]
    pub fn retaining(mut self, hold: ManagedRuntimeHold) -> Self {
        self.holds.push(hold);
        self
    }

    /// Returns the canonical path used by the operating-system spawn primitive.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the source of trust for this executable selection.
    #[must_use]
    pub const fn provenance(&self) -> ExecutableProvenance {
        self.provenance
    }

    fn from_candidate(
        candidate: &Path,
        provenance: ExecutableProvenance,
    ) -> Result<Self, ExecutableResolutionError> {
        if !candidate.is_absolute() {
            return Err(ExecutableResolutionError::PathNotAbsolute);
        }

        let canonical = fs::canonicalize(candidate).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ExecutableResolutionError::NotFound
            } else {
                ExecutableResolutionError::Inspection(error)
            }
        })?;
        let metadata = fs::metadata(&canonical).map_err(ExecutableResolutionError::Inspection)?;
        if !metadata.is_file() {
            return Err(ExecutableResolutionError::NotRegularFile);
        }

        Ok(Self {
            path: canonical,
            provenance,
            holds: Vec::new(),
        })
    }
}

/// Resolves setup-time provider names through a captured, filtered path.
#[derive(Clone, Debug)]
pub struct ExecutableResolver {
    search_directories: Vec<PathBuf>,
}

impl ExecutableResolver {
    /// Captures only absolute entries from the current process `PATH`.
    ///
    /// Empty and relative entries are ignored because they would implicitly trust the current
    /// working directory. The resulting executable is always canonicalized before invocation.
    #[must_use]
    pub fn from_current_path() -> Self {
        let current_directory = env::current_dir()
            .ok()
            .and_then(|directory| fs::canonicalize(directory).ok());
        let search_directories = env::var_os("PATH")
            .map(|value| filtered_search_directories(&value, current_directory.as_deref()))
            .unwrap_or_default();
        Self { search_directories }
    }

    /// Creates a resolver from explicit absolute search directories.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutableResolutionError::SearchDirectoryNotAbsolute`] when any supplied
    /// directory is relative.
    pub fn from_directories(
        directories: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, ExecutableResolutionError> {
        let search_directories = directories
            .into_iter()
            .map(|directory| {
                if directory.is_absolute() {
                    Ok(directory)
                } else {
                    Err(ExecutableResolutionError::SearchDirectoryNotAbsolute)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { search_directories })
    }

    /// Resolves a provider filename without accepting separators or shell syntax.
    ///
    /// # Errors
    ///
    /// Returns a typed validation, not-found, or filesystem-inspection failure when no trusted
    /// regular-file candidate can be selected.
    pub fn resolve(&self, name: &OsStr) -> Result<TrustedExecutable, ExecutableResolutionError> {
        if name.is_empty() || Path::new(name).components().count() != 1 {
            return Err(ExecutableResolutionError::InvalidProviderName);
        }

        for directory in &self.search_directories {
            for filename in platform_filenames(name) {
                let candidate = directory.join(filename);
                match TrustedExecutable::from_candidate(
                    &candidate,
                    ExecutableProvenance::AmbientPath,
                ) {
                    Ok(executable) => return Ok(executable),
                    Err(ExecutableResolutionError::NotFound) => {}
                    Err(error) => return Err(error),
                }
            }
        }

        Err(ExecutableResolutionError::NotFound)
    }
}

fn filtered_search_directories(value: &OsStr, current_directory: Option<&Path>) -> Vec<PathBuf> {
    env::split_paths(value)
        .filter(|path| path.is_absolute())
        .filter(|path| {
            let canonical = fs::canonicalize(path).ok();
            canonical.as_deref() != current_directory
        })
        .collect()
}

impl Default for ExecutableResolver {
    fn default() -> Self {
        Self::from_current_path()
    }
}

#[cfg(windows)]
fn platform_filenames(name: &OsStr) -> Vec<OsString> {
    let path = Path::new(name);
    if path.extension().is_some() {
        return vec![name.to_os_string()];
    }

    ["exe", "com"]
        .into_iter()
        .map(|extension| path.with_extension(extension).into_os_string())
        .collect()
}

#[cfg(not(windows))]
fn platform_filenames(name: &OsStr) -> Vec<OsString> {
    vec![name.to_os_string()]
}

/// A typed failure raised before an external executable is trusted.
#[derive(Debug)]
pub enum ExecutableResolutionError {
    /// An executable path was relative.
    PathNotAbsolute,
    /// A resolver search directory was relative.
    SearchDirectoryNotAbsolute,
    /// A provider name was empty or contained path components.
    InvalidProviderName,
    /// No candidate existed in the approved locations.
    NotFound,
    /// The resolved filesystem object was not a regular file.
    NotRegularFile,
    /// Filesystem metadata could not be inspected safely.
    Inspection(std::io::Error),
}

impl fmt::Display for ExecutableResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PathNotAbsolute => formatter.write_str("executable path must be absolute"),
            Self::SearchDirectoryNotAbsolute => {
                formatter.write_str("executable search directories must be absolute")
            }
            Self::InvalidProviderName => formatter.write_str("provider name is invalid"),
            Self::NotFound => formatter.write_str("provider executable was not found"),
            Self::NotRegularFile => {
                formatter.write_str("provider executable is not a regular file")
            }
            Self::Inspection(_) => {
                formatter.write_str("provider executable could not be inspected")
            }
        }
    }
}

impl Error for ExecutableResolutionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Inspection(error) => Some(error),
            Self::PathNotAbsolute
            | Self::SearchDirectoryNotAbsolute
            | Self::InvalidProviderName
            | Self::NotFound
            | Self::NotRegularFile => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{env, ffi::OsStr, path::PathBuf};

    use super::{
        ExecutableResolutionError, ExecutableResolver, TrustedExecutable,
        filtered_search_directories,
    };

    #[test]
    fn rejects_relative_explicit_paths() {
        let result = TrustedExecutable::explicit(PathBuf::from("relative-provider"));

        assert!(matches!(
            result,
            Err(ExecutableResolutionError::PathNotAbsolute)
        ));
    }

    #[test]
    fn rejects_relative_search_directories() {
        let result = ExecutableResolver::from_directories([PathBuf::from("relative")]);

        assert!(matches!(
            result,
            Err(ExecutableResolutionError::SearchDirectoryNotAbsolute)
        ));
    }

    #[test]
    fn rejects_provider_names_with_path_components() {
        let resolver = ExecutableResolver::from_directories(Vec::new());
        assert!(resolver.is_ok());
        let result = resolver.and_then(|value| value.resolve(OsStr::new("../provider")));

        assert!(matches!(
            result,
            Err(ExecutableResolutionError::InvalidProviderName)
        ));
    }

    #[test]
    fn ambient_search_excludes_the_current_directory() -> Result<(), Box<dyn std::error::Error>> {
        let current = std::fs::canonicalize(env::current_dir()?)?;
        let path_value = env::join_paths([current.clone()])?;

        let filtered = filtered_search_directories(&path_value, Some(&current));

        assert!(filtered.is_empty());
        Ok(())
    }
}
