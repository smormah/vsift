//! Shared private-root checks for per-user dependency configuration and managed data.
//!
//! A directory `VSift` creates for itself is made private by `VSift`, never by
//! the directory it is created in. On Windows a new directory inherits every
//! inheritable entry of its parent's DACL, and a real per-user profile can
//! carry entries for other principals there (sandbox groups, `AppContainer`
//! capability SIDs), so an inherited DACL is not private. Each new directory
//! therefore receives its own protected DACL before anything is written into
//! it, and the post-creation validation still runs as defence in depth. An
//! existing directory is never modified: when it fails validation it is
//! rejected as [`PrivateRootError::NotPrivate`].

use std::{
    fs, io,
    path::Path,
    thread,
    time::{Duration, Instant, SystemTime},
};

use cap_fs_ext::DirExt;
use cap_std::fs::{Dir, DirBuilder};

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
/// current user. When the final directory this call created cannot be made
/// private, or fails validation, it is removed again while still empty, so a
/// later call starts from nothing instead of meeting an unusable directory.
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
            parent_dir
                .create_dir_with(name, &private_directory_builder())
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
    let parent_dir = Dir::open_ambient_dir(parent, cap_std::ambient_authority())
        .map_err(|_| PrivateRootError::Io)?;
    let root = parent_dir
        .open_dir_nofollow(name)
        .map_err(|_| PrivateRootError::UnsafeStorage)?;
    let path_metadata = fs::symlink_metadata(path).map_err(|_| PrivateRootError::Io)?;
    validate_same_directory_object(&path_metadata, &root)?;
    if created {
        // The held handle pins the new directory (it is opened without delete
        // sharing), so the path-based DACL change below reaches this object.
        let secured =
            restrict_new_directory(path).and_then(|()| validate_private_root(path, &root));
        if let Err(error) = secured {
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
/// just-created directory private. Restricting takes milliseconds.
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
/// own validation decides whether it is usable. A directory this call created
/// but could not make private is removed again.
pub(crate) fn create_private_directory(path: &Path) -> Result<(), PrivateRootError> {
    let parent = path.parent().ok_or(PrivateRootError::Unavailable)?;
    let name = Path::new(path.file_name().ok_or(PrivateRootError::Unavailable)?);
    let parent_dir = Dir::open_ambient_dir(parent, cap_std::ambient_authority())
        .map_err(|_| PrivateRootError::Io)?;
    match parent_dir.create_dir_with(name, &private_directory_builder()) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
        Err(_) => return Err(PrivateRootError::Io),
    }
    let created = parent_dir
        .open_dir_nofollow(name)
        .map_err(|_| PrivateRootError::UnsafeStorage)?;
    if let Err(error) = restrict_new_directory(path) {
        drop(created);
        let _ = parent_dir.remove_dir(name);
        return Err(error);
    }
    Ok(())
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

/// The builder for every private directory created here.
///
/// On Unix the requested mode is 0o700. The umask can only clear bits, so a
/// new directory never grants group or other access whatever the umask is.
fn private_directory_builder() -> DirBuilder {
    #[allow(unused_mut, reason = "Unix configures the creation mode")]
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use cap_std::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
}

/// Makes a directory this process has just created private to the current user.
///
/// Unix sets the owner-only mode when the directory is created, so nothing
/// remains to do here; validation still checks the mode and owner.
#[cfg(unix)]
#[allow(
    clippy::unnecessary_wraps,
    reason = "the Windows implementation can fail to replace the DACL"
)]
pub(crate) fn restrict_new_directory(_path: &Path) -> Result<(), PrivateRootError> {
    Ok(())
}

/// Makes a directory this process has just created private to the current user.
///
/// The directory's inherited DACL is replaced by a protected one: inheritance
/// from the parent is disabled, and only the current user, `LocalSystem` and
/// the local Administrators group (the principals the validator trusts, and
/// the ones a default Windows profile grants) keep full control, inherited by
/// everything later created inside. Every inherited entry and every entry for
/// another principal is removed.
///
/// `windows-acl` writes each DACL change with
/// `PROTECTED_DACL_SECURITY_INFORMATION`, so the first added entry already
/// disables inheritance; the entries it copied from the inherited DACL are
/// removed afterwards. No step grants any principal more access than the
/// inherited DACL did. The caller must hold a handle to the new directory,
/// which stops the path from being replaced while the DACL is changed.
///
/// Never call this for a directory `VSift` did not just create.
#[cfg(windows)]
pub(crate) fn restrict_new_directory(path: &Path) -> Result<(), PrivateRootError> {
    use windows_acl::{acl::ACL, helper::string_to_sid};

    /// `FILE_ALL_ACCESS`: full control of a file or directory.
    const FULL_CONTROL: u32 = 0x001F_01FF;
    /// `ACE_HEADER` flag marking an entry that came from the parent.
    const INHERITED_ACE: u8 = 0x10;

    let path = path.to_str().ok_or(PrivateRootError::UnsafeStorage)?;
    let user_sid = current_user_sid().ok_or(PrivateRootError::Io)?;
    let trusted = trusted_sids(&user_sid);
    let mut acl = ACL::from_file_path(path, false).map_err(|_| PrivateRootError::Io)?;
    for sid in trusted {
        let mut raw = string_to_sid(sid).map_err(|_| PrivateRootError::Io)?;
        acl.allow(raw.as_mut_ptr().cast(), true, FULL_CONTROL)
            .map_err(|_| PrivateRootError::Io)?;
    }
    let entries = acl.all().map_err(|_| PrivateRootError::Io)?;
    for entry in entries {
        let flags = if trusted.contains(&entry.string_sid.as_str()) {
            if entry.flags & INHERITED_ACE == 0 {
                continue;
            }
            // Only a trusted principal's copied inherited entries go; its
            // explicit full-control entry added above stays.
            Some(INHERITED_ACE)
        } else {
            None
        };
        // The SID is rebuilt from its string form: `windows-acl` keeps an
        // entry's raw SID in a vector whose length is not set.
        let mut raw = string_to_sid(&entry.string_sid).map_err(|_| PrivateRootError::Io)?;
        acl.remove_entry(raw.as_mut_ptr().cast(), None, flags)
            .map_err(|_| PrivateRootError::Io)?;
    }
    Ok(())
}

/// The string SID of the account running this process.
#[cfg(windows)]
fn current_user_sid() -> Option<String> {
    use windows_acl::helper::{current_user, name_to_sid, sid_to_string};

    let username = current_user()?;
    name_to_sid(&username, None)
        .ok()
        .and_then(|mut sid| sid_to_string(sid.as_mut_ptr().cast()).ok())
}

/// The principals a private directory may grant access to: the current user,
/// `LocalSystem` (`S-1-5-18`) and the local Administrators group (`S-1-5-32-544`).
#[cfg(windows)]
fn trusted_sids(user_sid: &str) -> [&str; 3] {
    [user_sid, "S-1-5-18", "S-1-5-32-544"]
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
    let path = path.to_str().ok_or(PrivateRootError::UnsafeStorage)?;
    let acl = ACL::from_file_path(path, false).map_err(|_| PrivateRootError::UnsafeStorage)?;
    let entries = acl.all().map_err(|_| PrivateRootError::UnsafeStorage)?;
    let trusted = trusted_sids(&user_sid);
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
        CONCURRENT_CREATION_WAIT, PrivateRootError, create_private_directory, open_private_root,
        open_private_root_with_creation, restrict_new_directory, validate_existing_private_root,
        validate_private_root,
    };

    type TestResult = Result<(), Box<dyn Error>>;

    /// `BUILTIN\Users`: a broad principal the validator does not trust.
    const USERS: &str = "S-1-5-32-545";
    /// `NT AUTHORITY\Authenticated Users`, granted on the parent afterwards.
    const AUTHENTICATED_USERS: &str = "S-1-5-11";
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

    /// Grants `sid` inheritable read access on `directory`; Windows also
    /// propagates it to every existing child whose DACL is not protected.
    fn grant_inheritable(directory: &Path, sid: &str) -> TestResult {
        let status = Command::new(icacls()?)
            .arg(directory)
            .arg("/grant")
            .arg(format!("*{sid}:(OI)(CI)(RX)"))
            .arg("/Q")
            .output()?
            .status;
        if !status.success() {
            return Err("icacls could not grant the fixture entry".into());
        }
        Ok(())
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

    fn grants(entries: &[(String, u8)], sid: &str) -> bool {
        entries.iter().any(|(entry_sid, _)| entry_sid == sid)
    }

    /// Asserts that `directory` is private, inherits nothing, and does not
    /// take on an entry added to its parent afterwards (its DACL is protected).
    fn assert_private_and_protected(parent: &Path, directory: &Path) -> TestResult {
        let held = cap_std::fs::Dir::open_ambient_dir(directory, cap_std::ambient_authority())?;
        assert_eq!(validate_private_root(directory, &held), Ok(()));
        let entries = dacl(directory)?;
        assert!(!grants(&entries, USERS), "the broad principal kept access");
        assert!(
            entries.iter().all(|(_, flags)| flags & INHERITED_ACE == 0),
            "an inherited entry remained"
        );
        grant_inheritable(parent, AUTHENTICATED_USERS)?;
        assert!(
            !grants(&dacl(directory)?, AUTHENTICATED_USERS),
            "a later parent entry propagated into the directory"
        );
        assert_eq!(validate_private_root(directory, &held), Ok(()));
        Ok(())
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

    #[test]
    fn an_opener_waits_for_a_concurrent_creator_to_make_a_new_directory_private() -> TestResult {
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
            restrict_new_directory(&creator_path)
        });
        let outcome = validate_existing_private_root(&fresh, &held, CONCURRENT_CREATION_WAIT);
        let restricted = creator.join().map_err(|_| "creator thread failed")?;
        assert_eq!(restricted, Ok(()));
        assert_eq!(outcome, Ok(()));
        Ok(())
    }
}
