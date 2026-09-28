//! The neutral trial root.
//!
//! `VSift` returns absolute paths in `data.files[].path`, and a client sends
//! every tool result to its model provider. A trial under the user's home
//! or profile directory, or under any path that contains the operating
//! system user name, would therefore send the user name to the provider.
//! The harness refuses such roots before it creates anything; the
//! recommended roots are `C:\vsift-trials` on Windows and
//! `/srv/vsift-trials` elsewhere.

use std::{
    env,
    path::{Path, PathBuf},
};

use crate::{calls::normalise_path, error::TrialError};

/// Environment variables that name the user's home, profile, per-user data
/// or temporary directories.
const PROFILE_VARIABLES: [&str; 8] = [
    "HOME",
    "USERPROFILE",
    "LOCALAPPDATA",
    "APPDATA",
    "TEMP",
    "TMP",
    "TMPDIR",
    "XDG_CACHE_HOME",
];

/// Environment variables that name the user.
const USER_VARIABLES: [&str; 3] = ["USERNAME", "USER", "LOGNAME"];

/// What a trial root must avoid.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RootPolicy {
    forbidden: Vec<PathBuf>,
    user_names: Vec<String>,
}

impl RootPolicy {
    /// The given directories and user names.
    #[must_use]
    pub const fn new(forbidden: Vec<PathBuf>, user_names: Vec<String>) -> Self {
        Self {
            forbidden,
            user_names,
        }
    }

    /// The current user's home, profile and temporary directories and user
    /// names, from the environment.
    #[must_use]
    pub fn from_environment() -> Self {
        let forbidden = PROFILE_VARIABLES
            .iter()
            .filter_map(env::var_os)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .collect();
        let user_names = USER_VARIABLES
            .iter()
            .filter_map(|name| env::var(name).ok())
            .filter(|name| !name.trim().is_empty())
            .collect();
        Self::new(forbidden, user_names)
    }

    /// The user names, for the report checks.
    #[must_use]
    pub fn user_names(&self) -> &[String] {
        &self.user_names
    }

    /// Refuses a root that is relative, lies under a forbidden directory,
    /// contains a user name in a path component, or holds a quote or
    /// control character (the Codex sandbox configuration quotes it).
    ///
    /// # Errors
    ///
    /// [`TrialError::Refused`] naming the rule, never the user name.
    pub fn check(&self, root: &Path) -> Result<(), TrialError> {
        if !root.is_absolute() {
            return Err(TrialError::Refused(
                "the trial root must be an absolute path".to_owned(),
            ));
        }
        let text = root.to_string_lossy();
        if text
            .chars()
            .any(|value| value.is_control() || value == '\'' || value == '"')
        {
            return Err(TrialError::Refused(
                "the trial root holds a quote or control character".to_owned(),
            ));
        }
        let normalised = normalise_path(root);
        for directory in &self.forbidden {
            let forbidden = normalise_path(directory);
            if crate::calls::is_within(&normalised, &forbidden)
                || crate::calls::is_within(&forbidden, &normalised)
            {
                return Err(TrialError::Refused(
                    "the trial root is inside (or contains) the user's home, profile or temporary directory; use a neutral root such as C:\\vsift-trials or /srv/vsift-trials".to_owned(),
                ));
            }
        }
        let components: Vec<String> = normalised.split('/').map(str::to_lowercase).collect();
        for name in &self.user_names {
            let name = name.to_lowercase();
            let found = components.iter().any(|component| {
                if name.chars().count() < 3 {
                    *component == name
                } else {
                    component.contains(&name)
                }
            });
            if found {
                return Err(TrialError::Refused(
                    "the trial root's path contains the operating system user name".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> RootPolicy {
        let home = if cfg!(windows) {
            PathBuf::from("C:\\Users\\alex")
        } else {
            PathBuf::from("/home/alex")
        };
        RootPolicy::new(vec![home], vec!["alex".to_owned()])
    }

    fn root(windows: &str, unix: &str) -> PathBuf {
        PathBuf::from(if cfg!(windows) { windows } else { unix })
    }

    #[test]
    fn neutral_roots_pass_and_profile_roots_are_refused() {
        let policy = policy();
        assert!(
            policy
                .check(&root("C:\\vsift-trials", "/srv/vsift-trials"))
                .is_ok()
        );
        for refused in [
            root("C:\\Users\\alex\\trials", "/home/alex/trials"),
            root("D:\\alex-work\\trials", "/srv/alex-work/trials"),
            root("C:\\Users", "/home"),
            PathBuf::from("relative"),
            root("C:\\vsift'trials", "/srv/vsift'trials"),
        ] {
            assert!(
                matches!(policy.check(&refused), Err(TrialError::Refused(_))),
                "{}",
                refused.display()
            );
        }
    }
}
