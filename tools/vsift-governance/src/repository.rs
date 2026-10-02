//! The read-only view of the repository that the release-evidence and
//! public-claims checks work from.
//!
//! The two checks read documents and tables, never write, and never reach the
//! network. Putting the reads behind [`Repository`] lets every rule be tested
//! against a small in-memory fixture instead of the real tree.

use std::{fs, path::Path};

/// Reads repository files by their repository-relative path.
pub(crate) trait Repository {
    /// The text of the file at `path`, or the reason it could not be read.
    fn read_text(&self, path: &str) -> Result<String, String>;

    /// Whether `path` names an existing regular file.
    fn is_file(&self, path: &str) -> bool;

    /// Whether `path` names an existing file or directory.
    fn exists(&self, path: &str) -> bool;
}

/// The repository checked out on disk below `root`.
pub(crate) struct DiskRepository<'a> {
    root: &'a Path,
}

impl<'a> DiskRepository<'a> {
    /// A view of the repository whose top directory is `root`.
    pub(crate) fn new(root: &'a Path) -> Self {
        Self { root }
    }
}

impl Repository for DiskRepository<'_> {
    fn read_text(&self, path: &str) -> Result<String, String> {
        if !is_repository_relative(path) {
            return Err(format!("{path} is not a repository-relative path"));
        }
        fs::read_to_string(self.root.join(path)).map_err(|error| format!("{path}: {error}"))
    }

    fn is_file(&self, path: &str) -> bool {
        is_repository_relative(path) && self.root.join(path).is_file()
    }

    fn exists(&self, path: &str) -> bool {
        is_repository_relative(path) && self.root.join(path).exists()
    }
}

/// Whether `path` stays inside the repository by its spelling alone: not
/// empty, not absolute, no drive letter, no backslash and no `..` or empty
/// segment. A ledger or registry that names a path is data a contributor
/// edits, so it never gets to point the check at a file elsewhere.
pub(crate) fn is_repository_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "..")
}

#[cfg(test)]
pub(crate) mod fixture {
    //! An in-memory repository for rule tests.

    use std::collections::BTreeMap;

    use super::{Repository, is_repository_relative};

    /// A repository made of the files a test adds.
    #[derive(Default)]
    pub(crate) struct FixtureRepository {
        files: BTreeMap<String, String>,
    }

    impl FixtureRepository {
        /// An empty repository.
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// Adds (or replaces) a file.
        pub(crate) fn with(mut self, path: &str, text: &str) -> Self {
            self.files.insert(path.to_owned(), text.to_owned());
            self
        }
    }

    impl Repository for FixtureRepository {
        fn read_text(&self, path: &str) -> Result<String, String> {
            if !is_repository_relative(path) {
                return Err(format!("{path} is not a repository-relative path"));
            }
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| format!("{path}: not found"))
        }

        fn is_file(&self, path: &str) -> bool {
            self.files.contains_key(path)
        }

        fn exists(&self, path: &str) -> bool {
            let directory = format!("{}/", path.trim_end_matches('/'));
            path == "."
                || self.files.contains_key(path)
                || self.files.keys().any(|name| name.starts_with(&directory))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_repository_relative;

    #[test]
    fn ordinary_repository_paths_are_relative() {
        for path in [
            "README.md",
            "docs/planning/known-limits.md",
            "crates",
            ".github/workflows/release.yml",
            ".",
        ] {
            assert!(is_repository_relative(path), "{path}");
        }
    }

    #[test]
    fn paths_that_leave_the_repository_are_refused() {
        for path in [
            "",
            "/etc/passwd",
            "../outside",
            "docs/../../outside",
            "docs//planning",
            "docs\\planning",
            "C:/Windows",
            "docs/",
            "a\0b",
        ] {
            assert!(!is_repository_relative(path), "{path:?}");
        }
    }
}
