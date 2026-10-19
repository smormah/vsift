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

/// Remediation when the session root's own final folder name is one `VSift`
/// does not use (#232).
///
/// The code stays `STORAGE_IO`, what the case answered before it had a cause
/// (changing a published failure code is not additive within v1; known limit
/// L-127), so this text says what happened and states the rule. Control
/// characters are named for Windows only: Linux and macOS allow them in a
/// name, and so does `VSift`. The first sentence is stable and the text names
/// no path.
pub const SESSION_ROOT_NAME_REMEDIATION: &str = "The last folder name of the session root you gave is one VSift does not use, so nothing was created or changed, and no storage failed (the code is the closest published one). The name must not be empty, hold a colon, end with a space or a dot, or be a name Windows keeps for a device (CON, PRN, AUX, NUL, COM1 to COM9, LPT1 to LPT9, with or without an extension); on Windows it must also hold no control character and none of < > \" | ? *. Give --session-root a folder whose last name avoids these, and run the command again.";

/// Remediation for a session root whose permissions no longer hold an entry
/// for the current user (Windows, #443).
///
/// Before ADR 0026 several commands that used a new session root at the same
/// moment could leave it with only `SYSTEM` and Administrators. Such a root
/// refuses every open; `VSift` never gives the entry back itself, because it
/// cannot tell who removed it. The code stays `STORAGE_IO`, what the case
/// answered before it had a cause (L-127). The first sentence is stable, and
/// the text names no path: `<folder>` stands for it.
pub const SESSION_ROOT_OWN_ACCESS_REMEDIATION: &str = "The VSift session_root folder's permissions no longer include your own account, so VSift cannot open it and changed nothing. Only SYSTEM and Administrators keep access: earlier versions of VSift could leave a new session folder like this on Windows when several commands used it for the first time at once. It is the disposable-session folder: the directory given with --session-root, or VSift-sessions under LOCALAPPDATA. To use it again, give your account its access back from a Command Prompt, with that folder's path: icacls \"<folder>\" /grant \"%USERDOMAIN%\\%USERNAME%:(OI)(CI)F\". Or, as sessions are disposable, delete the folder (if Windows refuses, run the same icacls command first) and retry; VSift creates it again, private to you. VSift does not change the folder's permissions itself.";

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

/// Remediation when a source (or a supplied transcript) names a named pipe, a
/// device or a socket (#264), refused from its directory entry before anything
/// opened it.
///
/// A named pipe and a device keep `INVALID_SOURCE`, the documented answer, and
/// a socket keeps `STORAGE_IO`, the answer its failed open always gave (within
/// v1 a published failure code does not change; known limit L-127), so the
/// text says for the socket that no storage failed. It says the thing was not
/// opened, because opening one can wait for ever or act on a device. The first
/// sentence is stable and the text names no path.
pub const SOURCE_SPECIAL_FILE_REMEDIATION: &str = "The path you gave is not a regular file: it is a named pipe, a device or a socket, and VSift does not open those. Nothing was read or copied, and no storage failed (for a socket the code is the closest published one). VSift reads only regular files, such as a video saved on a disk. Name the video file itself and run the command again. If you do not know which file the user means, ask the user.";

/// Remediation when the path given as a source (or a supplied transcript) names
/// no file (#369): the name is mistyped, or the command was run from another
/// folder than the one that holds the file.
///
/// The code stays `STORAGE_IO`, the one 0.2.0 gave and, within v1, the only one
/// it may give (changing a published failure code is not additive; known limit
/// L-127), so this text says what happened: no file is there, nothing was read
/// and no storage failed, and a relative path is read from the folder the
/// command ran in, which is the usual cause. It says to ask the user and not to
/// guess, because an agent that cannot find a file is better off asking than
/// searching a person's disk. The first sentence is stable and the text names
/// no path.
pub const SOURCE_NOT_FOUND_REMEDIATION: &str = "No file is at the path you gave, so nothing was read or copied. No storage failed (the code is the closest published one). A relative path is read from the folder the command was run in, which may not be the folder that holds the file; a folder on the path that does not exist gives the same answer. Check the spelling of the name and of the folders, or give the file's full path, and run the command again. If you do not know where the file is, ask the user for its full path instead of guessing.";

/// Remediation when the path given as a source (or a supplied transcript) is a
/// folder (#369).
///
/// Only Windows answers a folder with `STORAGE_IO`: it refuses to open a folder
/// as a file. Linux and macOS open it, find it is no regular file and answer
/// `INVALID_SOURCE`, as they always have, so neither answer changes (known limit
/// L-127). This text is for the platform that has the published code
/// `STORAGE_IO`; the first sentence is stable and the text names no path.
pub const SOURCE_IS_FOLDER_REMEDIATION: &str = "The path you gave is a folder, not a file. Nothing was read or copied, and no storage failed (the code is the closest published one). VSift reads the one file you name and does not look inside folders. Name the file itself, with its full path or its path from the folder the command was run in, and run the command again. If you do not know which file the user means, ask the user.";

/// Remediation when the operating system refused to open the path given as a
/// source (or a supplied transcript) for reading (#369).
///
/// The code stays `STORAGE_IO`, the one 0.2.0 gave and the closest published
/// one: nothing is damaged, but access was refused, which is a failure of
/// access to storage. The text says it is the user's to change and not the
/// agent's: giving permission on someone's files is not something to do on a
/// guess. The first sentence is stable and the text names no path.
pub const SOURCE_ACCESS_DENIED_REMEDIATION: &str = "The operating system did not let VSift open the file at the path you gave, or a folder on the way to it, so nothing was read or copied. The account VSift runs as is not allowed to read it (the code is the closest published one). Report this to the user: they can give that account permission to read it, or copy the file to a folder that account can read and name the copy. Do not change permissions yourself. Then run the command again.";

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

/// Remediation when the copy of the source into the session took longer than
/// the ten-minute limit and was stopped (#325).
///
/// The code stays `INVALID_SOURCE`, what this case always gave and, within v1,
/// the only one it may give (changing a published failure code is not
/// additive; known limit L-127). That code says the video is at fault, which
/// was never looked at, so this text says what happened: the copy was slow,
/// the video was not judged, and a copy on a local disk is what to try. It
/// says "usually", because the copy has two ends: when the session root is the
/// slow one, moving the video does not help, and the text says so. It is
/// for the person, not for an agent to act on alone: copying gigabytes of
/// someone's video to another place is their decision. The first sentence is
/// stable. It names no path, size or speed; the limit it names is the fixed
/// one (`MAX_SOURCE_READ_DURATION` in `vsift-infrastructure`, known limit
/// L-140), and a test of the CLI, which sees both, holds the two together.
pub const SOURCE_COPY_TOO_SLOW_REMEDIATION: &str = "Copying this video into VSift's session took longer than the ten-minute limit, so the copy was stopped. The video itself was not judged and may be fine (the code is the closest published one); nothing was committed. This happens with a large video on a slow disk, a network share, a removable drive or a cloud-synced folder that downloads on first read. Report this to the user: copying the video to a local disk and running the same command on that copy usually fixes it (if the folder that holds VSift's sessions is on a slow or network disk, that can be the slow side instead). Running the same command again on the same file starts the copy from the beginning.";

/// Remediation when the copy of the source into the session was stopped early
/// because, at the fastest speed measured, it would have taken longer than the
/// ten-minute limit (#325).
///
/// The code stays `INVALID_SOURCE`, as for the copy that ran out of time
/// ([`SOURCE_COPY_TOO_SLOW_REMEDIATION`]), and the advice is the same; the
/// first sentence differs, because the copy did not run for ten minutes, and
/// is stable. It names no path, size or speed.
pub const SOURCE_COPY_WOULD_BE_TOO_SLOW_REMEDIATION: &str = "Copying this video into VSift's session would take longer than the ten-minute limit: VSift measured how fast the copy went and stopped it after the first minute or so instead of at ten minutes. The video itself was not judged and may be fine (the code is the closest published one); nothing was committed. This happens with a large video on a slow disk, a network share, a removable drive or a cloud-synced folder that downloads on first read. Report this to the user: copying the video to a local disk and running the same command on that copy usually fixes it (if the folder that holds VSift's sessions is on a slow or network disk, that can be the slow side instead). Running the same command again on the same file measures the copy again from the beginning.";

/// Remediation when no published session has the given id (#277): it was never
/// opened, is still being opened by another command, its opening was
/// interrupted (a crash) and left only a registration, it was cleaned already,
/// or its folder is gone.
///
/// The code is **the one the lookup always gave** (`STORAGE_IO`, or
/// `INVALID_ARGUMENT`, or `INTEGRITY_FAILURE` for a cleaned session whose lock
/// files remain): changing a published failure code is not additive within v1
/// (known limit L-127), so this text is what says it is a missing session and
/// not a diagnosis of the storage. It does not claim the storage is sound: a
/// published session whose folder was lost to real damage looks the same, which
/// is why it says when it would be damage. The first sentence is stable and the
/// text names neither a path nor an id.
pub const SESSION_NOT_PUBLISHED_REMEDIATION: &str = "No published session has this id, or its folder is gone. The id may be mistyped, the session may have expired or been cleaned, or its opening may not have finished: a command still running, or one that was interrupted, leaves a registration with no session behind it (session list shows it as initializing), which session clean removes once it is a day old. This is a missing session, not a diagnosis of the storage: the code (storage or integrity) is the closest published one. Treat it as damage only if session list still shows the session as indexed, or you know the session was open and you did not delete or move its folder. Check the id with session list, or open a new session with ingest.";

#[cfg(test)]
mod tests {
    use super::{
        PrivateFolder, SESSION_NOT_PUBLISHED_REMEDIATION, SOURCE_ACCESS_DENIED_REMEDIATION,
        SOURCE_COPY_TOO_SLOW_REMEDIATION, SOURCE_IS_FOLDER_REMEDIATION, SOURCE_IS_LINK_REMEDIATION,
        SOURCE_NO_ROOM_REMEDIATION, SOURCE_NOT_FOUND_REMEDIATION, UNOWNED_SESSION_ROOT_REMEDIATION,
        non_private_folder_summary,
    };

    /// #369: the three remediations of a path that could not be opened say what
    /// is true, are bounded and pathless, and are told apart by their first
    /// sentence, so an agent can act on one without parsing free text.
    #[test]
    fn the_source_path_remediations_are_stable_bounded_pathless_and_distinct() {
        for (text, first_sentence) in [
            (
                SOURCE_NOT_FOUND_REMEDIATION,
                "No file is at the path you gave,",
            ),
            (
                SOURCE_IS_FOLDER_REMEDIATION,
                "The path you gave is a folder, not a file.",
            ),
            (
                SOURCE_ACCESS_DENIED_REMEDIATION,
                "The operating system did not let VSift open the file at the path you gave",
            ),
        ] {
            assert!(text.starts_with(first_sentence), "{text}");
            assert!(
                text.contains("nothing was read or copied")
                    || text.contains("Nothing was read or copied"),
                "{text}"
            );
            assert!(
                text.contains("the code is the closest published one"),
                "{text}"
            );
            // None of them says the video is damaged or unsupported, and none
            // gives a command or an option to run.
            for wrong in ["corrupt", "damaged", "invalid", "unsupported", "--"] {
                assert!(!text.contains(wrong), "{wrong}: {text}");
            }
            assert!(text.len() <= 1024, "{} bytes", text.len());
            assert!(!text.contains('\\') && !text.contains(":/"), "{text}");
        }
        // Only a path that names no file explains where a relative path is read
        // from; only a permission refusal sends the person to the user.
        assert!(SOURCE_NOT_FOUND_REMEDIATION.contains("folder the command was run in"));
        assert!(SOURCE_NOT_FOUND_REMEDIATION.contains("full path"));
        assert!(SOURCE_NOT_FOUND_REMEDIATION.contains("ask the user"));
        assert!(SOURCE_NOT_FOUND_REMEDIATION.contains("No storage failed"));
        assert!(SOURCE_IS_FOLDER_REMEDIATION.contains("does not look inside folders"));
        assert!(SOURCE_ACCESS_DENIED_REMEDIATION.contains("Report this to the user"));
        assert!(SOURCE_ACCESS_DENIED_REMEDIATION.contains("Do not change permissions yourself"));
        assert!(!SOURCE_ACCESS_DENIED_REMEDIATION.contains("folder the command was run in"));
    }

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
    fn the_slow_copy_remediation_is_stable_bounded_and_does_not_blame_the_video() {
        let text = SOURCE_COPY_TOO_SLOW_REMEDIATION;
        assert!(
            text.starts_with(
                "Copying this video into VSift's session took longer than the ten-minute limit"
            ),
            "{text}"
        );
        assert!(text.contains("was not judged"), "{text}");
        assert!(text.contains("closest published one"), "{text}");
        assert!(text.contains("Report this to the user"), "{text}");
        assert!(text.contains("local disk"), "{text}");
        // A local copy is the usual way out, not a promise: the session root
        // can be the slow end of the copy.
        assert!(text.contains("usually fixes it"), "{text}");
        assert!(!text.contains("copy fixes it"), "{text}");
        assert!(
            text.contains("folder that holds VSift's sessions"),
            "{text}"
        );
        // It is not the answer of a bad file, and it promises no way to go on:
        // no option raises the limit and a rerun starts again (#325).
        for wrong in [
            "invalid",
            "unsupported",
            "corrupt",
            "damaged",
            "--",
            "resume",
        ] {
            assert!(!text.contains(wrong), "{wrong}: {text}");
        }
        assert!(
            text.contains("starts the copy from the beginning"),
            "{text}"
        );
        assert!(text.len() <= 1024, "{} bytes", text.len());
        assert!(!text.contains('\\') && !text.contains(":/"), "{text}");
    }

    #[test]
    fn the_not_published_remediation_is_stable_bounded_and_does_not_claim_the_storage_is_sound() {
        let text = SESSION_NOT_PUBLISHED_REMEDIATION;
        assert!(
            text.starts_with("No published session has this id, or its folder is gone."),
            "{text}"
        );
        assert!(text.contains("not a diagnosis of the storage"), "{text}");
        assert!(text.contains("closest published one"), "{text}");
        assert!(
            !text.contains("Nothing is wrong with the storage"),
            "it must not claim the storage is sound: {text}"
        );
        assert!(text.contains("session list") && text.contains("session clean"));
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
