//! Creating a directory with its security descriptor, through `CreateDirectoryW`.
//!
//! The three calls into Windows are the only `unsafe` code in `VSift`
//! (ADR 0026). Each is one operation in its own block, with the invariants it
//! relies on written beside it.

use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt as _, path::Path, ptr};

use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::CreateDirectoryW,
};

use crate::PrivateDirectoryDescriptor;

/// Creates the directory `path` with `descriptor` as its security descriptor,
/// set by Windows as the directory comes into being.
///
/// There is no moment in which the directory exists with any other DACL: the
/// parent's inheritable entries never reach it, and nothing has to write its
/// permissions afterwards. `path` must be absolute; the verbatim form
/// (`\\?\C:\...`) that `std::fs::canonicalize` returns is accepted and lifts the
/// `MAX_PATH` limit.
///
/// # Errors
///
/// `InvalidInput` when `path` is relative or contains a NUL character;
/// otherwise the operating system's error, as `io::Error::last_os_error`
/// reports it: `AlreadyExists` when the name exists (another creator won), and
/// `NotFound` when the parent is missing. Nothing is created on any error.
pub fn create_directory(path: &Path, descriptor: &PrivateDirectoryDescriptor) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a private directory needs an absolute path",
        ));
    }
    let wide_path = nul_terminated(path.as_os_str())?;
    let wide_sddl = nul_terminated(OsStr::new(descriptor.sddl()))?;
    let security = LocalSecurityDescriptor::from_sddl(&wide_sddl)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>())
            .map_err(|_| io::Error::other("SECURITY_ATTRIBUTES does not fit its length field"))?,
        lpSecurityDescriptor: security.as_ptr(),
        bInheritHandle: 0,
    };
    #[allow(
        unsafe_code,
        reason = "ADR 0026: the one call that creates the directory"
    )]
    // SAFETY: `wide_path` is a NUL-terminated UTF-16 string with no interior
    // NUL (`nul_terminated`), owned by this frame and alive for the call.
    // `attributes` is a fully initialised `SECURITY_ATTRIBUTES` whose `nLength`
    // is its own size, whose descriptor pointer comes from
    // `ConvertStringSecurityDescriptorToSecurityDescriptorW` and stays valid
    // until `security` drops after this call, and which asks for no handle
    // inheritance. `CreateDirectoryW` only reads both and keeps neither.
    let created = unsafe { CreateDirectoryW(wide_path.as_ptr(), &raw const attributes) };
    // Read before `security` drops: `LocalFree` may change the last error.
    let outcome = if created == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    };
    drop(security);
    outcome
}

/// `text` as UTF-16 with a terminating NUL, refusing a NUL inside it (which
/// Windows would read as the end of the string).
pub(crate) fn nul_terminated(text: &OsStr) -> io::Result<Vec<u16>> {
    let mut wide: Vec<u16> = text.encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a path or descriptor holds a NUL character",
        ));
    }
    wide.push(0);
    Ok(wide)
}

/// A self-relative security descriptor that Windows allocated with
/// `LocalAlloc`, freed with `LocalFree` exactly once, when this drops.
///
/// Not `Clone` or `Copy`, so the pointer has one owner.
struct LocalSecurityDescriptor(PSECURITY_DESCRIPTOR);

impl LocalSecurityDescriptor {
    /// Converts NUL-terminated UTF-16 SDDL into a security descriptor.
    fn from_sddl(sddl: &[u16]) -> io::Result<Self> {
        if sddl.last() != Some(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SDDL must be NUL-terminated",
            ));
        }
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        #[allow(
            unsafe_code,
            reason = "ADR 0026: building the descriptor from its SDDL"
        )]
        // SAFETY: `sddl` ends with a NUL (checked above) and lives for the
        // call. `descriptor` is a valid, writable out-pointer on this frame.
        // The size out-parameter is optional and passed as null, which the
        // function documents. On success Windows writes a pointer it
        // allocated with `LocalAlloc`, which `Self` then owns.
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &raw mut descriptor,
                ptr::null_mut(),
            )
        };
        if converted == 0 {
            return Err(io::Error::last_os_error());
        }
        if descriptor.is_null() {
            return Err(io::Error::other("Windows returned no security descriptor"));
        }
        Ok(Self(descriptor))
    }

    const fn as_ptr(&self) -> PSECURITY_DESCRIPTOR {
        self.0
    }
}

impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        #[allow(
            unsafe_code,
            reason = "ADR 0026: freeing the descriptor Windows allocated"
        )]
        // SAFETY: `self.0` is the non-null pointer that
        // `ConvertStringSecurityDescriptorToSecurityDescriptorW` allocated with
        // `LocalAlloc` (`from_sddl` is the only constructor), which the
        // function documents must be released with `LocalFree`. `Self` is
        // neither `Clone` nor `Copy`, so this runs once per pointer, and
        // nothing uses the pointer after it.
        let _ = unsafe { LocalFree(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use std::{env, error::Error, fmt::Write as _, fs, io, path::PathBuf};

    use super::{LocalSecurityDescriptor, create_directory, nul_terminated};
    use crate::PrivateDirectoryDescriptor;

    /// A folder under the temporary folder, removed when it drops.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Result<Self, Box<dyn Error>> {
            let mut random = [0_u8; 8];
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            random.copy_from_slice(&stamp.to_le_bytes()[..8]);
            let mut name = String::from("vsift-windows-security-");
            for byte in random {
                write!(&mut name, "{byte:02x}")?;
            }
            write!(&mut name, "-{}", std::process::id())?;
            let path = env::temp_dir().join(name);
            fs::create_dir(&path)?;
            Ok(Self(fs::canonicalize(path)?))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The current user's string SID, read with `windows-acl` (a development
    /// dependency only), as the infrastructure crate reads it.
    fn current_user_sid() -> Result<String, Box<dyn Error>> {
        use windows_acl::helper::{current_user, name_to_sid, sid_to_string};
        let name = current_user().ok_or("no current user")?;
        let mut sid = name_to_sid(&name, None).map_err(|code| format!("name_to_sid: {code}"))?;
        Ok(sid_to_string(sid.as_mut_ptr().cast())
            .map_err(|code| format!("sid_to_string: {code}"))?)
    }

    fn descriptor() -> Result<PrivateDirectoryDescriptor, Box<dyn Error>> {
        Ok(PrivateDirectoryDescriptor::for_owner(&current_user_sid()?)?)
    }

    /// One stored DACL entry: its SID, header flags and access mask.
    type Entry = (String, u8, u32);

    /// The DACL Windows stored: each entry's SID, header flags and mask, read
    /// with `windows-acl`.
    fn stored_entries(path: &std::path::Path) -> Result<Vec<Entry>, Box<dyn Error>> {
        let text = path.to_str().ok_or("not UTF-8")?;
        let acl = windows_acl::acl::ACL::from_file_path(text, false)
            .map_err(|code| format!("read DACL: {code}"))?;
        Ok(acl
            .all()
            .map_err(|code| format!("list DACL: {code}"))?
            .into_iter()
            .map(|entry| (entry.string_sid, entry.flags, entry.mask))
            .collect())
    }

    /// What Windows stored is exactly the descriptor's DACL: three explicit
    /// (not inherited) full-control entries that later children inherit, even
    /// under a parent that grants `BUILTIN\Users` to every new child.
    #[test]
    fn the_created_directory_holds_exactly_the_private_dacl() -> Result<(), Box<dyn Error>> {
        let scratch = Scratch::new()?;
        let user = current_user_sid()?;
        let icacls = PathBuf::from(env::var_os("SystemRoot").ok_or("SystemRoot is not set")?)
            .join("System32")
            .join("icacls.exe");
        let status = std::process::Command::new(icacls)
            .arg(&scratch.0)
            .args(["/grant", "*S-1-5-32-545:(OI)(CI)(RX)", "/Q"])
            .output()?
            .status;
        assert!(status.success(), "the fixture's parent entry was not added");
        let path = scratch.0.join("private");
        create_directory(&path, &descriptor()?)?;
        let mut entries = stored_entries(&path)?;
        entries.sort();
        let mut expected = vec![
            (user.clone(), 0x03_u8, 0x001F_01FF_u32),
            (crate::SYSTEM_SID.to_owned(), 0x03, 0x001F_01FF),
            (crate::ADMINISTRATORS_SID.to_owned(), 0x03, 0x001F_01FF),
        ];
        expected.sort();
        assert_eq!(entries, expected);
        // A child created inside inherits the three entries and nothing else.
        let child = path.join("child");
        fs::create_dir(&child)?;
        let trusted = [user.as_str(), crate::SYSTEM_SID, crate::ADMINISTRATORS_SID];
        let inherited = stored_entries(&child)?;
        assert_eq!(inherited.len(), 3, "{inherited:?}");
        assert!(
            inherited
                .iter()
                .all(|(sid, flags, _)| flags & 0x10 != 0 && trusted.contains(&sid.as_str())),
            "{inherited:?}"
        );
        Ok(())
    }

    #[test]
    fn the_sddl_converts_to_a_descriptor() -> Result<(), Box<dyn Error>> {
        let wide = nul_terminated(std::ffi::OsStr::new(descriptor()?.sddl()))?;
        let converted = LocalSecurityDescriptor::from_sddl(&wide)?;
        assert!(!converted.as_ptr().is_null());
        Ok(())
    }

    #[test]
    fn sddl_windows_does_not_accept_is_an_error_not_a_descriptor() -> Result<(), Box<dyn Error>> {
        let wide = nul_terminated(std::ffi::OsStr::new("D:P(A;OICI;NOT-A-RIGHT;;;SY)"))?;
        assert!(LocalSecurityDescriptor::from_sddl(&wide).is_err());
        assert!(LocalSecurityDescriptor::from_sddl(&[u16::from(b'D')]).is_err());
        Ok(())
    }

    #[test]
    fn a_directory_is_created_once() -> Result<(), Box<dyn Error>> {
        let scratch = Scratch::new()?;
        let path = scratch.0.join("private");
        let descriptor = descriptor()?;
        create_directory(&path, &descriptor)?;
        assert!(fs::symlink_metadata(&path)?.is_dir());
        let again = create_directory(&path, &descriptor);
        assert!(
            matches!(&again, Err(error) if error.kind() == io::ErrorKind::AlreadyExists),
            "{again:?}"
        );
        Ok(())
    }

    #[test]
    fn a_missing_parent_a_relative_path_and_a_nul_are_refused() -> Result<(), Box<dyn Error>> {
        let scratch = Scratch::new()?;
        let descriptor = descriptor()?;
        let missing = create_directory(&scratch.0.join("absent").join("child"), &descriptor);
        assert!(
            matches!(&missing, Err(error) if error.kind() == io::ErrorKind::NotFound),
            "{missing:?}"
        );
        let relative = create_directory(std::path::Path::new("relative"), &descriptor);
        assert!(
            matches!(&relative, Err(error) if error.kind() == io::ErrorKind::InvalidInput),
            "{relative:?}"
        );
        let nul = create_directory(&scratch.0.join("a\0b"), &descriptor);
        assert!(
            matches!(&nul, Err(error) if error.kind() == io::ErrorKind::InvalidInput),
            "{nul:?}"
        );
        assert_eq!(fs::read_dir(&scratch.0)?.count(), 0, "nothing was created");
        Ok(())
    }
}
