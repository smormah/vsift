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

/// Remediation for an existing session root that holds no `VSift` ownership
/// marker (#261).
///
/// `VSift` writes the marker last when it creates a root, so a folder without
/// one was made by someone else and is never adopted: it is left exactly as it
/// was. The failure code stays `INTEGRITY_FAILURE` (the v1 answer since 0.1.0,
/// and v1 is additive only); this text is what tells the person the folder is
/// theirs, not damaged `VSift` data. The first sentence is stable so an agent
/// can act on it without parsing free text. It never contains a path.
pub const UNOWNED_SESSION_ROOT_REMEDIATION: &str = "The VSift session_root folder holds no VSift ownership marker, so VSift did not create it and did not use it. Nothing was changed: VSift never adopts a folder it did not create. It is the disposable-session folder: the directory given with --session-root, or VSift-sessions under LOCALAPPDATA on Windows, Library/Caches on macOS, or the XDG cache directory on Linux. Name a --session-root path that does not exist yet and VSift creates it, private to you; or, if that folder holds nothing you need, delete it yourself and retry.";

/// Remediation when a source (or a supplied transcript) is itself a symbolic
/// link (#265).
///
/// The code stays `STORAGE_IO`, the one 0.1.0 gave and, within v1, the only one
/// it may give (changing a published failure code is not additive; known limit
/// L-127), so this text says what happened: the link is not followed, and no
/// storage failed. The first sentence is stable and the text names no path.
pub const SOURCE_IS_LINK_REMEDIATION: &str = "The path you gave is a link, and VSift does not follow links. Nothing was read or copied, and no storage failed (the code is the closest published one). Name the file the link points to and run the command again with that path.";

/// Remediation when the session root's filesystem has no room for a copy of
/// the source (#266), found before the copy from the source's size and the
/// free space, or when a write ran out of space.
///
/// The code stays `STORAGE_IO`, what the CLI path gave and, within v1, the only
/// one it may give (changing a published failure code is not additive; known
/// limit L-127), so this text says what happened: nothing is damaged, the
/// video is fine, and it is the user's drive (or an operator's choice of
/// folder) that has to change, not the request. The first sentence is stable.
/// It never contains a path or a size.
pub const SOURCE_NO_ROOM_REMEDIATION: &str = "The folder that holds VSift's sessions does not have room for a copy of this video. Nothing was committed, nothing is damaged and the video itself is fine (the code is the closest published one). Report this to the user: freeing space on that drive, or an operator naming a folder on a drive with more room with --session-root, fixes it; do not choose a folder yourself. Then run the command again. The folder is the --session-root directory, or VSift-sessions under LOCALAPPDATA on Windows, Library/Caches on macOS, or the XDG cache directory on Linux.";

#[cfg(test)]
mod tests {
    use super::{
        PrivateFolder, SOURCE_IS_LINK_REMEDIATION, SOURCE_NO_ROOM_REMEDIATION,
        UNOWNED_SESSION_ROOT_REMEDIATION, non_private_folder_summary,
    };

    #[test]
    fn the_link_remediation_is_stable_bounded_and_pathless() {
        let text = SOURCE_IS_LINK_REMEDIATION;
        assert!(
            text.starts_with("The path you gave is a link, and VSift does not follow links."),
            "{text}"
        );
        assert!(text.contains("Name the file the link points to"));
        assert!(text.contains("no storage failed"), "{text}");
        assert!(!text.contains("corrupt"), "{text}");
        assert!(text.len() <= 1024, "{} bytes", text.len());
        assert!(!text.contains('\\') && !text.contains(":/"), "{text}");
    }

    #[test]
    fn the_no_room_remediation_is_stable_bounded_and_pathless() {
        let text = SOURCE_NO_ROOM_REMEDIATION;
        assert!(
            text.starts_with(
                "The folder that holds VSift's sessions does not have room for a copy"
            ),
            "{text}"
        );
        assert!(text.contains("--session-root"));
        assert!(text.contains("Report this to the user"));
        assert!(text.contains("nothing is damaged"), "{text}");
        assert!(
            !text.contains("corrupt") && !text.contains("integrity"),
            "{text}"
        );
        assert!(text.len() <= 1024, "{} bytes", text.len());
        assert!(!text.contains('\\') && !text.contains(":/"), "{text}");
    }

    #[test]
    fn the_unowned_root_remediation_is_stable_bounded_and_pathless() {
        let text = UNOWNED_SESSION_ROOT_REMEDIATION;
        assert!(
            text.starts_with(
                "The VSift session_root folder holds no VSift ownership marker, so VSift did not create it"
            ),
            "{text}"
        );
        assert!(text.contains("never adopts a folder it did not create"));
        assert!(text.contains("does not exist yet"));
        assert!(text.contains("--session-root"));
        assert!(text.len() <= 1024, "{} bytes", text.len());
        assert!(!text.contains('\\') && !text.contains(":/"), "{text}");
    }

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
