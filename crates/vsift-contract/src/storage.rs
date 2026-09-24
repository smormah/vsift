//! Fixed-prose remediation for an existing `VSift` folder that other accounts
//! can access.
//!
//! `VSift` makes every folder it creates private to the current user. A folder
//! that already exists is never changed: when it grants access to another
//! account, the operation fails before using it, and the remediation names the
//! folder by kind, never by path, so no absolute user path reaches the output.

/// A per-user `VSift` folder that must be private to the current user.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivateFolder {
    /// The per-user configuration folder: dependency selections and
    /// media-tool verification records.
    UserConfiguration,
    /// The disposable-session root.
    SessionRoot,
}

impl PrivateFolder {
    /// Every folder kind, in declaration order.
    pub const ALL: [Self; 2] = [Self::UserConfiguration, Self::SessionRoot];

    /// Returns the stable lowercase identifier used in the remediation summary.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::UserConfiguration => "user_configuration",
            Self::SessionRoot => "session_root",
        }
    }
}

/// Structured remediation for an existing folder that is not private.
///
/// The summary starts with the fixed sentence `The VSift <kind> folder is
/// accessible to other accounts, so VSift did not use it and changed
/// nothing.`, where `<kind>` is [`PrivateFolder::identifier`], so an agent can
/// act on it without parsing free text. Fixed prose locating the folder and
/// the next step follows. It never contains a path.
#[must_use]
pub fn non_private_folder_summary(folder: PrivateFolder) -> String {
    let (location, next_step) = match folder {
        PrivateFolder::UserConfiguration => (
            "It is the per-user VSift configuration folder: vsift under LOCALAPPDATA on Windows, under Application Support on macOS, or under the XDG configuration directory on Linux.",
            "Delete that folder and retry; VSift recreates it private to you, and dependencies are then registered again with setup configure.",
        ),
        PrivateFolder::SessionRoot => (
            "It is the disposable-session folder: the directory given with --session-root, or VSift-sessions under LOCALAPPDATA on Windows, Library/Caches on macOS, or the XDG cache directory on Linux.",
            "Sessions are disposable, so delete that folder and retry; VSift recreates it private to you.",
        ),
    };
    format!(
        "The VSift {} folder is accessible to other accounts, so VSift did not use it and changed nothing. {location} {next_step} Alternatively remove the other accounts' access yourself; on Windows, disable inherited permissions on the folder so only you, SYSTEM and Administrators keep access.",
        folder.identifier()
    )
}

#[cfg(test)]
mod tests {
    use super::{PrivateFolder, non_private_folder_summary};

    #[test]
    fn every_folder_is_named_by_kind_within_the_schema_bound() {
        for folder in PrivateFolder::ALL {
            let summary = non_private_folder_summary(folder);
            let marker = format!(
                "The VSift {} folder is accessible to other accounts,",
                folder.identifier()
            );
            assert!(summary.starts_with(&marker), "{summary}");
            assert!(summary.contains("changed nothing"), "{summary}");
            assert!(summary.len() <= 1024, "{summary}");
        }
    }
}
