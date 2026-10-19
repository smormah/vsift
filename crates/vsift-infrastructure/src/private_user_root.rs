//! Shared private-root checks for per-user dependency configuration and managed data.
//!
//! A directory `VSift` creates for itself is made private by `VSift`, never by
//! the directory it is created in, and it is private **from its first
//! instant**. On Unix the creation mode (0o700) does that. On Windows a new
//! directory would inherit every inheritable entry of its parent's DACL, and a
//! real per-user profile can carry entries for other principals there (sandbox
//! groups, `AppContainer` capability SIDs), so each one is created with its own
//! security descriptor (`vsift_windows_security`, ADR 0026): a protected DACL
//! granting only the current user, `LocalSystem` and Administrators. Nothing
//! writes a new directory's permissions afterwards. Until ADR 0026 the DACL was
//! replaced by path-based writes just after creation, and each such write on a
//! parent made Windows rewrite the DACLs of the folders inside it while their
//! own creators were changing them, which left a session root with no entry
//! for its own user (#206, #443).
//!
//! The post-creation validation still runs as defence in depth. An existing
//! directory is never modified: when it fails validation it is rejected as
//! [`PrivateRootError::NotPrivate`].

use std::{
    fs, io,
    path::Path,
    thread,
    time::{Duration, Instant, SystemTime},
};

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;

/// A failure to obtain a private, non-link per-user directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateRootError {
    /// The path has no parent or final component to work with.
    Unavailable,
    /// The path is a link or another object, or its permissions cannot be read.
    UnsafeStorage,
    /// A real directory exists but other accounts can access it, or on Unix
    /// another user owns it. It is left exactly as found.
    NotPrivate,
    /// A concurrent creator won the race for the final directory.
    Busy,
    /// The operating system could not complete a storage operation.
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
///
/// Missing ancestors and the final directory are each created private to the
/// current user. When the final directory this call created fails validation,
/// it is removed again while still empty, so a later call starts from nothing
/// instead of meeting an unusable directory.
pub(crate) fn open_private_root_with_creation(
    path: &Path,
    create: bool,
) -> Result<Option<(Dir, bool)>, PrivateRootError> {
    let parent = path.parent().ok_or(PrivateRootError::Unavailable)?;
    let name = Path::new(path.file_name().ok_or(PrivateRootError::Unavailable)?);
    let mut created = false;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(PrivateRootError::UnsafeStorage),
        Err(error) if error.kind() == io::ErrorKind::NotFound && !create => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_missing_private_ancestors(parent)?;
            let parent_dir = Dir::open_ambient_dir(parent, cap_std::ambient_authority())
                .map_err(|_| PrivateRootError::Io)?;
            create_private_child(&parent_dir, parent, name).map_err(|error| {
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
    let parent_dir = Dir::open_ambient_dir(parent, cap_std::ambient_authority())
        .map_err(|_| PrivateRootError::Io)?;
    let root = parent_dir
        .open_dir_nofollow(name)
        .map_err(|_| PrivateRootError::UnsafeStorage)?;
    let path_metadata = fs::symlink_metadata(path).map_err(|_| PrivateRootError::Io)?;
    validate_same_directory_object(&path_metadata, &root)?;
    if created {
        if let Err(error) = validate_private_root(path, &root) {
            drop(root);
            // Removes only an empty directory; nothing was written into it.
            let _ = parent_dir.remove_dir(name);
            return Err(error);
        }
    } else {
        validate_existing_private_root(path, &root, CONCURRENT_CREATION_WAIT)?;
    }
    Ok(Some((root, created)))
}

/// Longest time an opener waits for a concurrent creator to make a
/// just-created directory private. Since ADR 0026 a directory `VSift` creates
/// is private from its first instant; the wait remains for a creator of an
/// earlier version, which restricted the directory after creating it.
const CONCURRENT_CREATION_WAIT: Duration = Duration::from_secs(2);
/// How recently an empty directory must have been created to be treated as a
/// concurrent creator's directory that is not yet private.
const CONCURRENT_CREATION_RECENT: Duration = Duration::from_secs(10);

/// Validates an existing root, giving a concurrent creator a moment to finish.
///
/// Another process may have created the directory an instant ago and not yet
/// replaced its inherited DACL. Only a directory that is still empty and was
/// created within the last few seconds is waited for; each attempt repeats
/// the full validation, and a directory with content or an older one is
/// judged at once. The directory is never changed.
fn validate_existing_private_root(
    path: &Path,
    root: &Dir,
    wait: Duration,
) -> Result<(), PrivateRootError> {
    let started = Instant::now();
    let mut backoff = Duration::from_millis(2);
    loop {
        match validate_private_root(path, root) {
            Err(PrivateRootError::NotPrivate)
                if started.elapsed() < wait && is_recent_and_empty(root) =>
            {
                thread::sleep(backoff);
                backoff = backoff.saturating_mul(2).min(Duration::from_millis(50));
            }
            outcome => return outcome,
        }
    }
}

fn is_recent_and_empty(root: &Dir) -> bool {
    let empty = root
        .entries()
        .is_ok_and(|mut entries| entries.next().is_none());
    let recent = root
        .dir_metadata()
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| {
            SystemTime::now()
                .duration_since(modified.into_std())
                .map_or(true, |age| age <= CONCURRENT_CREATION_RECENT)
        });
    empty && recent
}

/// Creates one directory private to the current user unless the path exists.
///
/// An existing entry is left untouched and reported as success; the caller's
/// own validation decides whether it is usable.
pub(crate) fn create_private_directory(path: &Path) -> Result<(), PrivateRootError> {
    let parent = path.parent().ok_or(PrivateRootError::Unavailable)?;
    let name = Path::new(path.file_name().ok_or(PrivateRootError::Unavailable)?);
    let parent_dir = Dir::open_ambient_dir(parent, cap_std::ambient_authority())
        .map_err(|_| PrivateRootError::Io)?;
    match create_private_child(&parent_dir, parent, name) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(_) => Err(PrivateRootError::Io),
    }
}

/// Creates each missing ancestor of a private root, top down, privately.
///
/// Existing ancestors, including linked ones, are used as they are, as
/// `create_dir_all` would; only directories created here are made private.
fn create_missing_private_ancestors(directory: &Path) -> Result<(), PrivateRootError> {
    let mut missing = Vec::new();
    let mut cursor = directory;
    while !cursor.is_dir() {
        missing.push(cursor);
        cursor = cursor.parent().ok_or(PrivateRootError::Unavailable)?;
    }
    for ancestor in missing.into_iter().rev() {
        create_private_directory(ancestor)?;
    }
    Ok(())
}

/// Creates `name` inside `parent`, the held directory at `parent_path`,
/// private to the current user from its first instant.
///
/// On Unix the directory is created relative to the held handle with mode
/// 0o700 (a umask can only clear bits). On Windows it is created with its own
/// security descriptor (ADR 0026): owned by the current user, with a protected
/// DACL of full control, inherited by everything later created inside, for the
/// current user, `LocalSystem` and Administrators, and nothing else; the path is
/// the canonical (verbatim) form of `parent_path` joined with `name`. Nothing
/// writes the directory's permissions afterwards.
///
/// # Errors
///
/// The operating system's error, `AlreadyExists` when the name exists.
pub(crate) fn create_private_child(
    parent: &Dir,
    parent_path: &Path,
    name: &Path,
) -> io::Result<()> {
    create_private_child_observed(parent, parent_path, name, |_| {})
}

/// [`create_private_child`], calling `observe` with the new directory's path
/// the moment it exists, before the call returns: tests use it to act at that
/// instant, as a concurrent process could.
#[cfg(unix)]
pub(crate) fn create_private_child_observed(
    parent: &Dir,
    parent_path: &Path,
    name: &Path,
    observe: impl FnOnce(&Path),
) -> io::Result<()> {
    use cap_std::fs::{DirBuilder, DirBuilderExt as _};
    let mut builder = DirBuilder::new();
    builder.mode(0o700);
    parent.create_dir_with(name, &builder)?;
    observe(&parent_path.join(name));
    Ok(())
}

/// [`create_private_child`], calling `observe` with the new directory's path
/// the moment it exists, before the call returns: tests use it to act at that
/// instant, as a concurrent process could.
#[cfg(windows)]
pub(crate) fn create_private_child_observed(
    _parent: &Dir,
    parent_path: &Path,
    name: &Path,
    observe: impl FnOnce(&Path),
) -> io::Result<()> {
    let user_sid = current_user_sid()
        .ok_or_else(|| io::Error::other("the current user's SID is unavailable"))?;
    let descriptor = vsift_windows_security::PrivateDirectoryDescriptor::for_owner(&user_sid)
        .map_err(io::Error::other)?;
    let path = fs::canonicalize(parent_path)?.join(name);
    vsift_windows_security::create_directory(&path, &descriptor)?;
    observe(&path);
    Ok(())
}

/// The principals a private directory may grant access to: the current user,
/// `LocalSystem` and the local Administrators group. The one list, used by
/// every validator.
#[cfg(windows)]
pub(crate) fn trusted_principals(user_sid: &str) -> [&str; 3] {
    [
        user_sid,
        vsift_windows_security::SYSTEM_SID,
        vsift_windows_security::ADMINISTRATORS_SID,
    ]
}

/// Whether an entry grants access, in any of the four forms of an allow entry
/// (as opposed to denying it, or being a kind this code does not know).
#[cfg(windows)]
pub(crate) fn is_allow_entry(kind: windows_acl::acl::AceType) -> bool {
    use windows_acl::acl::AceType;

    matches!(
        kind,
        AceType::AccessAllow
            | AceType::AccessAllowCallback
            | AceType::AccessAllowObject
            | AceType::AccessAllowCallbackObject
    )
}

/// The string SID of the account running this process.
#[cfg(windows)]
pub(crate) fn current_user_sid() -> Option<String> {
    use windows_acl::helper::{current_user, name_to_sid, sid_to_string};

    let username = current_user()?;
    name_to_sid(&username, None)
        .ok()
        .and_then(|mut sid| sid_to_string(sid.as_mut_ptr().cast()).ok())
}

/// What a directory's DACL says about the current user, read without opening
/// the directory itself.
#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnAccess {
    /// The DACL grants the current user access and no one the validator does
    /// not trust.
    Private,
    /// The DACL grants someone the validator does not trust.
    Shared,
    /// The DACL grants only trusted principals, and the current user is not
    /// among them: its own entry is gone, the state #443's race left a session
    /// root in. Only its owner (or an administrator) can give it back.
    OwnEntryMissing,
    /// The DACL could not be read.
    Unreadable,
}

/// The path a DACL is read by: the canonical, extended-length form (`\?\C:\...`).
///
/// `GetNamedSecurityInfoW`, which `windows-acl` calls, refuses a plain path of
/// `MAX_PATH` (260) characters or more, so a private root under a long
/// per-user folder failed its own validation (#469); the extended-length form
/// is read at any length. Every caller has already checked that the last
/// component is the directory it holds, not a link, so the canonical form
/// names the same directory. A path that cannot be canonicalized (one whose
/// owner lost its own entry, say) is read as it is given, as before.
#[cfg(windows)]
pub(crate) fn security_read_path(path: &Path) -> Option<String> {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .into_os_string()
        .into_string()
        .ok()
}

/// Reads the DACL at `path` and classifies it. The owner of a directory can
/// read its DACL even without an entry of its own, so a root that lost the
/// user's entry is still told apart from one that cannot be read at all.
#[cfg(windows)]
pub(crate) fn own_access(path: &Path) -> OwnAccess {
    use windows_acl::acl::{ACL, AceType};

    let (Some(user_sid), Some(path)) = (current_user_sid(), security_read_path(path)) else {
        return OwnAccess::Unreadable;
    };
    let Ok(entries) = ACL::from_file_path(&path, false).and_then(|acl| acl.all()) else {
        return OwnAccess::Unreadable;
    };
    let trusted = trusted_principals(&user_sid);
    let shared = entries.iter().any(|entry| {
        entry.entry_type == AceType::Unknown
            || (is_allow_entry(entry.entry_type) && !trusted.contains(&entry.string_sid.as_str()))
    });
    let user_allowed = entries
        .iter()
        .any(|entry| is_allow_entry(entry.entry_type) && entry.string_sid == user_sid);
    match (shared, user_allowed) {
        (true, _) => OwnAccess::Shared,
        (false, true) => OwnAccess::Private,
        (false, false) => OwnAccess::OwnEntryMissing,
    }
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
        return Err(PrivateRootError::NotPrivate);
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn validate_private_root(path: &Path, _root: &Dir) -> Result<(), PrivateRootError> {
    use windows_acl::acl::{ACL, AceType};

    let user_sid = current_user_sid().ok_or(PrivateRootError::UnsafeStorage)?;
    let path = security_read_path(path).ok_or(PrivateRootError::UnsafeStorage)?;
    let acl = ACL::from_file_path(&path, false).map_err(|_| PrivateRootError::UnsafeStorage)?;
    let entries = acl.all().map_err(|_| PrivateRootError::UnsafeStorage)?;
    let trusted = trusted_principals(&user_sid);
    let current_user_allowed = entries
        .iter()
        .any(|entry| is_allow_entry(entry.entry_type) && entry.string_sid == user_sid);
    let unsafe_entry = entries.iter().any(|entry| {
        entry.entry_type == AceType::Unknown
            || (is_allow_entry(entry.entry_type) && !trusted.contains(&entry.string_sid.as_str()))
    });
    if !current_user_allowed || unsafe_entry {
        return Err(PrivateRootError::NotPrivate);
    }
    Ok(())
}

/// Reproduces a real Windows profile whose `%LOCALAPPDATA%` passes inheritable
/// entries for other principals to every new child, which made `setup
/// configure` fail with `STORAGE_IO`. The fixture's extra parent entry is
/// added with the system `icacls.exe`, independently of the code under test.
#[cfg(all(test, windows))]
mod windows_tests {
    use std::{
        env,
        error::Error,
        fmt::Write as _,
        fs,
        path::{Path, PathBuf},
        process::Command,
        thread,
        time::{Duration, Instant},
    };

    use windows_acl::acl::ACL;

    use super::{
        CONCURRENT_CREATION_WAIT, OwnAccess, PrivateRootError, create_private_child_observed,
        create_private_directory, current_user_sid, open_private_root,
        open_private_root_with_creation, own_access, validate_existing_private_root,
        validate_private_root,
    };

    type TestResult = Result<(), Box<dyn Error>>;

    /// `BUILTIN\Users`: a broad principal the validator does not trust.
    const USERS: &str = "S-1-5-32-545";
    /// `NT AUTHORITY\Authenticated Users`, granted on the parent afterwards.
    const AUTHENTICATED_USERS: &str = "S-1-5-11";
    const SYSTEM: &str = "S-1-5-18";
    const ADMINISTRATORS: &str = "S-1-5-32-544";
    const INHERITED_ACE: u8 = 0x10;

    /// A temporary parent whose DACL passes read access for `BUILTIN\Users`
    /// to every new child, as the maintainer's `%LOCALAPPDATA%` passes its
    /// sandbox-group and capability entries.
    struct HostileParent(PathBuf);

    impl HostileParent {
        fn new() -> Result<Self, Box<dyn Error>> {
            let mut random = [0_u8; 16];
            getrandom::fill(&mut random).map_err(|_| std::io::Error::other("random failed"))?;
            let mut suffix = String::with_capacity(random.len() * 2);
            for byte in random {
                write!(suffix, "{byte:02x}")?;
            }
            let path = env::temp_dir().join(format!("vsift-private-root-acl-{suffix}"));
            fs::create_dir(&path)?;
            let parent = Self(path);
            grant_inheritable(&parent.0, USERS)?;
            Ok(parent)
        }
    }

    impl Drop for HostileParent {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn icacls() -> Result<PathBuf, Box<dyn Error>> {
        let system_root = env::var_os("SystemRoot").ok_or("SystemRoot is not set")?;
        Ok(PathBuf::from(system_root)
            .join("System32")
            .join("icacls.exe"))
    }

    /// Runs `icacls.exe` with explicit arguments (never a shell).
    fn run_icacls(arguments: &[&std::ffi::OsStr]) -> TestResult {
        let status = Command::new(icacls()?).args(arguments).output()?.status;
        if !status.success() {
            return Err(format!("icacls {arguments:?} failed").into());
        }
        Ok(())
    }

    /// Grants `sid` inheritable read access on `directory`; Windows also
    /// propagates it to every existing child whose DACL is not protected.
    fn grant_inheritable(directory: &Path, sid: &str) -> TestResult {
        let grant = format!("*{sid}:(OI)(CI)(RX)");
        run_icacls(&[
            directory.as_os_str(),
            "/grant".as_ref(),
            grant.as_ref(),
            "/Q".as_ref(),
        ])
    }

    /// Each DACL entry as its string SID and header flags.
    fn dacl(directory: &Path) -> Result<Vec<(String, u8)>, Box<dyn Error>> {
        let path = directory.to_str().ok_or("fixture path is not UTF-8")?;
        let acl = ACL::from_file_path(path, false).map_err(|_| "DACL is unreadable")?;
        Ok(acl
            .all()
            .map_err(|_| "DACL is unreadable")?
            .into_iter()
            .map(|entry| (entry.string_sid, entry.flags))
            .collect())
    }

    fn sorted_dacl(directory: &Path) -> Result<Vec<(String, u8)>, Box<dyn Error>> {
        let mut entries = dacl(directory)?;
        entries.sort();
        Ok(entries)
    }

    /// The three explicit, inheritable entries a private directory holds.
    fn private_dacl(user: &str) -> Vec<(String, u8)> {
        let mut expected = vec![
            (user.to_owned(), 0x03_u8),
            (SYSTEM.to_owned(), 0x03),
            (ADMINISTRATORS.to_owned(), 0x03),
        ];
        expected.sort();
        expected
    }

    fn grants(entries: &[(String, u8)], sid: &str) -> bool {
        entries.iter().any(|(entry_sid, _)| entry_sid == sid)
    }

    /// Asserts that `directory` holds exactly the private DACL and does not
    /// take on an entry added to its parent afterwards (its DACL is protected).
    fn assert_private_and_protected(parent: &Path, directory: &Path) -> TestResult {
        let user = current_user_sid().ok_or("no current user")?;
        let held = cap_std::fs::Dir::open_ambient_dir(directory, cap_std::ambient_authority())?;
        assert_eq!(validate_private_root(directory, &held), Ok(()));
        assert_eq!(
            sorted_dacl(directory)?,
            private_dacl(&user),
            "{}",
            directory.display()
        );
        grant_inheritable(parent, AUTHENTICATED_USERS)?;
        assert!(
            !grants(&dacl(directory)?, AUTHENTICATED_USERS),
            "a later parent entry propagated into the directory"
        );
        assert_eq!(validate_private_root(directory, &held), Ok(()));
        Ok(())
    }

    /// Gives the current user its full-control entry back when dropped, so a
    /// fixture the user was locked out of can still be removed.
    struct RestoreAccess(PathBuf);

    impl Drop for RestoreAccess {
        fn drop(&mut self) {
            if let Some(user) = current_user_sid() {
                let grant = format!("*{user}:(OI)(CI)F");
                let _ = run_icacls(&[self.0.as_os_str(), "/grant".as_ref(), grant.as_ref()]);
            }
        }
    }

    #[test]
    fn the_fixture_reproduces_an_inherited_broad_entry() -> TestResult {
        let parent = HostileParent::new()?;
        let plain = parent.0.join("plain");
        fs::create_dir(&plain)?;
        assert!(grants(&dacl(&plain)?, USERS));
        let held = cap_std::fs::Dir::open_ambient_dir(&plain, cap_std::ambient_authority())?;
        assert_eq!(
            validate_private_root(&plain, &held),
            Err(PrivateRootError::NotPrivate)
        );
        Ok(())
    }

    #[test]
    fn a_created_root_and_its_created_ancestors_are_private_and_protected() -> TestResult {
        let parent = HostileParent::new()?;
        let ancestor = parent.0.join("vsift");
        let root = ancestor.join("managed-v1");
        let opened = open_private_root_with_creation(&root, true);
        assert!(matches!(opened, Ok(Some((_, true)))));
        drop(opened);
        assert_private_and_protected(&parent.0, &ancestor)?;
        assert_private_and_protected(&ancestor, &root)?;
        // Reopening applies only the read-only validation and still passes.
        assert!(matches!(
            open_private_root_with_creation(&root, true),
            Ok(Some((_, false)))
        ));
        Ok(())
    }

    #[test]
    fn a_single_created_directory_is_private_and_protected() -> TestResult {
        let parent = HostileParent::new()?;
        let directory = parent.0.join("sessions-parent");
        assert_eq!(create_private_directory(&directory), Ok(()));
        assert_private_and_protected(&parent.0, &directory)
    }

    /// #443's mechanism, made on demand. A path-based DACL write on a parent
    /// makes Windows rewrite the DACL of each folder inside it from what it
    /// read of that folder a moment before. With several first users at once
    /// (the instrumented runs of #443), such a write from the parent's creator
    /// landed on a new root after its own creator's last check, carrying the
    /// root's DACL from an earlier moment, and the root kept it.
    ///
    /// Here that late write carries the folder's DACL as it was the instant
    /// the folder came into being, as Windows' propagation computes it: a
    /// folder that inherited from its parent then is given its parent's
    /// inheritable entries again (`icacls /reset` writes exactly that), and a
    /// folder whose DACL was already protected keeps it (nothing is written).
    ///
    /// A folder created with its security descriptor (ADR 0026) is protected
    /// and private from that instant, so the late write changes nothing. A
    /// folder created and then restricted, as before, inherited from the
    /// parent at that instant (here `BUILTIN\Users`): the late write gives
    /// those entries back and the folder is no longer private, with its creator
    /// already done.
    #[test]
    fn a_late_parent_write_cannot_take_a_new_folder_back_to_an_earlier_state() -> TestResult {
        let user = current_user_sid().ok_or("no current user")?;
        let parent = HostileParent::new()?;
        let held = cap_std::fs::Dir::open_ambient_dir(&parent.0, cap_std::ambient_authority())?;
        let mut first_instant = None;
        create_private_child_observed(&held, &parent.0, Path::new("root"), |created| {
            first_instant = Some(dacl(created).map_err(|error| error.to_string()));
        })?;
        let first_instant = first_instant.ok_or("the creation was not observed")??;
        let root = parent.0.join("root");
        let _restore = RestoreAccess(root.clone());
        if first_instant
            .iter()
            .any(|(_, flags)| flags & INHERITED_ACE != 0)
        {
            run_icacls(&[root.as_os_str(), "/reset".as_ref(), "/Q".as_ref()])?;
        }

        assert_eq!(sorted_dacl(&root)?, private_dacl(&user));
        assert!(!grants(&dacl(&root)?, USERS), "the stale write reached it");
        let opened = cap_std::fs::Dir::open_ambient_dir(&root, cap_std::ambient_authority())?;
        assert_eq!(validate_private_root(&root, &opened), Ok(()));
        Ok(())
    }

    /// A directory that has lost the current user's entry, with only
    /// `LocalSystem` and Administrators left (#443's broken root), is told
    /// apart from one that grants someone else and from a private one, by its
    /// DACL alone: its owner can read it without an entry of its own.
    #[test]
    fn a_folder_without_the_users_own_entry_is_recognised() -> TestResult {
        let user = current_user_sid().ok_or("no current user")?;
        let parent = HostileParent::new()?;
        let root = parent.0.join("root");
        assert_eq!(create_private_directory(&root), Ok(()));
        assert_eq!(own_access(&root), OwnAccess::Private);

        let _restore = RestoreAccess(root.clone());
        let remove = format!("*{user}");
        run_icacls(&[root.as_os_str(), "/remove:g".as_ref(), remove.as_ref()])?;
        assert_eq!(sorted_dacl(&root)?.len(), 2);
        assert_eq!(own_access(&root), OwnAccess::OwnEntryMissing);

        grant_inheritable(&root, USERS)?;
        assert_eq!(own_access(&root), OwnAccess::Shared);
        assert_eq!(own_access(&parent.0.join("absent")), OwnAccess::Unreadable);
        Ok(())
    }

    #[test]
    fn an_existing_directory_is_rejected_as_not_private_and_left_untouched() -> TestResult {
        let parent = HostileParent::new()?;
        let existing = parent.0.join("vsift");
        fs::create_dir(&existing)?;
        // Content means no creator can still be at work, so it is judged at once.
        fs::write(existing.join("dependencies-v1.json"), b"{}")?;
        let before = dacl(&existing)?;
        let started = Instant::now();
        assert!(matches!(
            open_private_root(&existing, true),
            Err(PrivateRootError::NotPrivate)
        ));
        assert!(matches!(
            open_private_root(&existing, false),
            Err(PrivateRootError::NotPrivate)
        ));
        assert!(started.elapsed() < CONCURRENT_CREATION_WAIT);
        // Creating over an existing directory is a no-op that never touches it.
        assert_eq!(create_private_directory(&existing), Ok(()));
        assert!(existing.is_dir());
        assert_eq!(dacl(&existing)?, before);
        assert_eq!(fs::read_dir(&existing)?.count(), 1);
        Ok(())
    }

    /// An earlier version's creator restricted a directory after creating it;
    /// an opener still gives such a creator a moment to finish.
    #[test]
    fn an_opener_waits_for_a_concurrent_creator_to_make_a_new_directory_private() -> TestResult {
        let user = current_user_sid().ok_or("no current user")?;
        let parent = HostileParent::new()?;
        let fresh = parent.0.join("vsift");
        fs::create_dir(&fresh)?;
        let held = cap_std::fs::Dir::open_ambient_dir(&fresh, cap_std::ambient_authority())?;

        // Nobody finishes it: fresh and empty, it is waited on, then refused.
        let started = Instant::now();
        let wait = Duration::from_millis(100);
        assert_eq!(
            validate_existing_private_root(&fresh, &held, wait),
            Err(PrivateRootError::NotPrivate)
        );
        assert!(started.elapsed() >= wait);

        // A creator that restricts it during the wait lets the opener use it.
        let creator_path = fresh.clone();
        let creator = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            let grants = [
                format!("*{user}:(OI)(CI)F"),
                format!("*{SYSTEM}:(OI)(CI)F"),
                format!("*{ADMINISTRATORS}:(OI)(CI)F"),
            ];
            run_icacls(&[
                creator_path.as_os_str(),
                "/inheritance:r".as_ref(),
                "/grant:r".as_ref(),
                grants[0].as_ref(),
                grants[1].as_ref(),
                grants[2].as_ref(),
            ])
            .map_err(|error| error.to_string())
        });
        let outcome = validate_existing_private_root(&fresh, &held, CONCURRENT_CREATION_WAIT);
        let restricted = creator.join().map_err(|_| "creator thread failed")?;
        assert_eq!(restricted, Ok(()));
        assert_eq!(outcome, Ok(()));
        Ok(())
    }

    /// #469: a private root whose plain path is longer than `MAX_PATH`, as
    /// `%LOCALAPPDATA%\vsift` is under a long per-user folder, is created,
    /// validated and reopened; its DACL was read by the plain path, which
    /// Windows refuses at that length, so it failed as unsafe storage.
    #[test]
    fn a_root_deeper_than_max_path_is_created_and_validated() -> TestResult {
        let parent = HostileParent::new()?;
        let mut deep = parent.0.clone();
        let mut index = 0_u32;
        while deep.as_os_str().len() < 300 {
            deep.push(format!(
                "a-made-up-folder-name-that-only-makes-the-path-long-{index}"
            ));
            index += 1;
        }
        let root = deep.join("vsift");
        assert!(!root.as_os_str().to_string_lossy().starts_with(r"\\?\"));

        let created = open_private_root_with_creation(&root, true);

        assert!(matches!(created, Ok(Some((_, true)))), "{created:?}");
        drop(created);
        assert!(matches!(
            open_private_root_with_creation(&root, true),
            Ok(Some((_, false)))
        ));
        assert_eq!(own_access(&root), OwnAccess::Private);
        Ok(())
    }

    /// Nothing the creation leaves is inherited, on any level it creates.
    #[test]
    fn no_created_level_holds_an_inherited_entry() -> TestResult {
        let parent = HostileParent::new()?;
        let root = parent.0.join("a").join("b").join("c");
        assert!(matches!(
            open_private_root_with_creation(&root, true),
            Ok(Some((_, true)))
        ));
        for level in [parent.0.join("a"), parent.0.join("a").join("b"), root] {
            assert!(
                dacl(&level)?
                    .iter()
                    .all(|(_, flags)| flags & INHERITED_ACE == 0),
                "{}",
                level.display()
            );
        }
        Ok(())
    }
}

/// The Unix creation mode, which the umask can only narrow.
#[cfg(all(test, unix))]
mod unix_tests {
    use std::{error::Error, fs, os::unix::fs::PermissionsExt as _, path::Path};

    use super::create_private_child_observed;
    use crate::test_folder::TestFolder;

    #[test]
    fn a_created_child_is_owner_only_from_its_first_instant() -> Result<(), Box<dyn Error>> {
        let folder = TestFolder::new("vsift-private-child")?;
        let held = cap_std::fs::Dir::open_ambient_dir(&*folder, cap_std::ambient_authority())?;
        let mut mode_then = None;
        create_private_child_observed(&held, &folder, Path::new("root"), |created| {
            mode_then = fs::metadata(created)
                .ok()
                .map(|metadata| metadata.permissions().mode() & 0o777);
        })?;
        assert_eq!(mode_then.map(|mode| mode & 0o077), Some(0));
        Ok(())
    }
}
