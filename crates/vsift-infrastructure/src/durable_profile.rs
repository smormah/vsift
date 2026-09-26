//! Which publication guarantee a session root may offer (ADR 0010, ADR 0020).
//!
//! ADR 0010 qualifies OS-crash-durable publication only on Ubuntu 24.04 with a
//! local ext4 filesystem, and only after an owned crash campaign has passed.
//! The store therefore claims [`PublicationGuarantee::OsCrashDurable`] only
//! when all of these hold:
//!
//! - the build targets Linux;
//! - the root's device appears in `/proc/self/mountinfo` only as ext4 mounts
//!   that do not disable write barriers (`nobarrier` or `barrier=0`), read
//!   with a bound and parsed strictly by [`classify_mountinfo`];
//! - the campaign constant `QUALIFIED_UBUNTU_EXT4` is set. It is `false`
//!   until P10's campaign evidence is recorded, so every profile still fails
//!   durable requests closed with an unsupported-guarantee error.
//!
//! The mountinfo parser is public so the fuzz harness reaches it through the
//! same surface as the store.

use std::{error::Error, fmt};

use cap_std::fs::Dir;
use vsift_application::StorageCapabilities;
use vsift_domain::PublicationGuarantee;

/// Largest `/proc/self/mountinfo` the profile check reads: 1 MiB, far above a
/// host's mount table and small enough to hold in memory.
pub const MAX_MOUNTINFO_BYTES: usize = 1024 * 1024;

/// Set only when the Ubuntu 24.04 / ext4 crash campaign has passed (P10 PR 4).
#[cfg(target_os = "linux")]
const QUALIFIED_UBUNTU_EXT4: bool = false;

/// A Linux device number split into its major and minor parts, as
/// `/proc/self/mountinfo` prints it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MountDevice {
    major: u32,
    minor: u32,
}

impl MountDevice {
    /// A device from its major and minor numbers.
    #[must_use]
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Splits a Linux `st_dev` the way glibc's `major()` and `minor()` do.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "each masked part fits in 32 bits by construction"
    )]
    pub const fn from_linux_dev(device: u64) -> Self {
        let major = ((device >> 32) & 0xffff_f000) | ((device >> 8) & 0x0000_0fff);
        let minor = ((device >> 12) & 0xffff_ff00) | (device & 0x0000_00ff);
        Self {
            major: major as u32,
            minor: minor as u32,
        }
    }

    /// The major number.
    #[must_use]
    pub const fn major(self) -> u32 {
        self.major
    }

    /// The minor number.
    #[must_use]
    pub const fn minor(self) -> u32 {
        self.minor
    }
}

/// What the mount table says about one device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MountProfile {
    /// Every mount of the device is ext4 with write barriers left on.
    Ext4WithBarriers,
    /// Every mount is ext4, but at least one disables write barriers.
    Ext4WithoutBarriers,
    /// At least one mount of the device is another filesystem.
    OtherFilesystem,
    /// No mount of the device is listed.
    NotMounted,
}

/// Why a mount table was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MountInfoError {
    /// Larger than [`MAX_MOUNTINFO_BYTES`].
    TooLarge,
    /// Not UTF-8 text.
    NotUtf8,
    /// A line (numbered from 1) does not have the documented shape.
    Malformed {
        /// The 1-based line number.
        line: usize,
    },
}

impl fmt::Display for MountInfoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => formatter.write_str("the mount table is too large"),
            Self::NotUtf8 => formatter.write_str("the mount table is not UTF-8"),
            Self::Malformed { line } => write!(formatter, "mount table line {line} is malformed"),
        }
    }
}

impl Error for MountInfoError {}

/// Classifies `device` from a `/proc/<pid>/mountinfo` table.
///
/// Every line is parsed, not only the device's, so a table that is damaged
/// anywhere is refused rather than half trusted. A line holds the mount id,
/// the parent id, `major:minor`, the root, the mount point, the mount options
/// and any optional fields, then a lone `-`, then the filesystem type, the
/// source and the superblock options, separated by single spaces (the kernel
/// escapes spaces inside fields). Write barriers count as disabled when
/// either option list holds `nobarrier` or `barrier=0`.
///
/// # Errors
///
/// A table over [`MAX_MOUNTINFO_BYTES`], not UTF-8, or with a malformed line.
pub fn classify_mountinfo(
    mountinfo: &[u8],
    device: MountDevice,
) -> Result<MountProfile, MountInfoError> {
    if mountinfo.len() > MAX_MOUNTINFO_BYTES {
        return Err(MountInfoError::TooLarge);
    }
    let text = std::str::from_utf8(mountinfo).map_err(|_| MountInfoError::NotUtf8)?;
    let mut seen = false;
    let mut other = false;
    let mut without_barriers = false;
    for (index, line) in text.lines().enumerate() {
        let entry = parse_line(line).ok_or(MountInfoError::Malformed { line: index + 1 })?;
        if entry.device != device {
            continue;
        }
        seen = true;
        if entry.filesystem != "ext4" {
            other = true;
        } else if disables_barriers(entry.mount_options) || disables_barriers(entry.super_options) {
            without_barriers = true;
        }
    }
    Ok(if !seen {
        MountProfile::NotMounted
    } else if other {
        MountProfile::OtherFilesystem
    } else if without_barriers {
        MountProfile::Ext4WithoutBarriers
    } else {
        MountProfile::Ext4WithBarriers
    })
}

/// The fields of one mount line the profile needs.
struct MountEntry<'a> {
    device: MountDevice,
    mount_options: &'a str,
    filesystem: &'a str,
    super_options: &'a str,
}

fn parse_line(line: &str) -> Option<MountEntry<'_>> {
    let (before, after) = line.split_once(" - ")?;
    let mut fields = before.split(' ');
    fields.next()?.parse::<u32>().ok()?;
    fields.next()?.parse::<u32>().ok()?;
    let (major, minor) = fields.next()?.split_once(':')?;
    let device = MountDevice::new(major.parse().ok()?, minor.parse().ok()?);
    let root = fields.next()?;
    let mount_point = fields.next()?;
    let mount_options = fields.next()?;
    if [root, mount_point, mount_options]
        .iter()
        .any(|field| field.is_empty())
        || fields.any(str::is_empty)
    {
        return None;
    }
    let mut tail = after.split(' ');
    let filesystem = tail.next()?;
    let source = tail.next()?;
    let super_options = tail.next()?;
    if tail.next().is_some() || filesystem.is_empty() || source.is_empty() {
        return None;
    }
    Some(MountEntry {
        device,
        mount_options,
        filesystem,
        super_options,
    })
}

fn disables_barriers(options: &str) -> bool {
    options
        .split(',')
        .any(|option| option == "nobarrier" || option == "barrier=0")
}

/// The capabilities a store over `root` may claim on this host.
pub(crate) fn storage_capabilities(root: &Dir) -> StorageCapabilities {
    StorageCapabilities::new(if os_crash_durable(root) {
        PublicationGuarantee::OsCrashDurable
    } else {
        PublicationGuarantee::ProcessCrashConsistent
    })
}

/// Whether the root sits on the qualified Ubuntu/ext4 profile.
#[cfg(target_os = "linux")]
fn os_crash_durable(root: &Dir) -> bool {
    use std::io::Read as _;
    if !QUALIFIED_UBUNTU_EXT4 {
        return false;
    }
    let Ok(metadata) = root.dir_metadata() else {
        return false;
    };
    let device = MountDevice::from_linux_dev(cap_std::fs::MetadataExt::dev(&metadata));
    let Ok(file) = std::fs::File::open("/proc/self/mountinfo") else {
        return false;
    };
    let mut table = Vec::new();
    let limit = u64::try_from(MAX_MOUNTINFO_BYTES).unwrap_or(u64::MAX);
    if file.take(limit + 1).read_to_end(&mut table).is_err() {
        return false;
    }
    classify_mountinfo(&table, device) == Ok(MountProfile::Ext4WithBarriers)
}

/// No other target has a qualified durable profile (ADR 0010).
#[cfg(not(target_os = "linux"))]
fn os_crash_durable(_root: &Dir) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_MOUNTINFO_BYTES, MountDevice, MountInfoError, MountProfile, classify_mountinfo,
    };

    const TABLE: &str = "\
22 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw,errors=remount-ro
23 22 0:21 / /proc rw,nosuid,nodev,noexec,relatime shared:12 - proc proc rw
24 22 8:2 / /data rw,relatime - ext4 /dev/sda2 rw,nobarrier
25 22 8:3 / /xfs rw,relatime - xfs /dev/sda3 rw,attr2
26 22 8:4 / /old rw,relatime - ext4 /dev/sda4 rw,barrier=0
27 22 8:1 /srv /mnt/bind\\040dir rw,relatime shared:1 - ext4 /dev/sda1 rw,errors=remount-ro
28 22 8:5 / /mixed rw - ext4 /dev/sda5 rw
29 22 8:5 / /mixed2 rw - btrfs /dev/sda5 rw
";

    #[test]
    fn devices_are_classified_across_every_mount_of_them() {
        let classify =
            |major, minor| classify_mountinfo(TABLE.as_bytes(), MountDevice::new(major, minor));
        assert_eq!(classify(8, 1), Ok(MountProfile::Ext4WithBarriers));
        assert_eq!(classify(8, 2), Ok(MountProfile::Ext4WithoutBarriers));
        assert_eq!(classify(8, 3), Ok(MountProfile::OtherFilesystem));
        assert_eq!(classify(8, 4), Ok(MountProfile::Ext4WithoutBarriers));
        assert_eq!(classify(8, 5), Ok(MountProfile::OtherFilesystem));
        assert_eq!(classify(0, 21), Ok(MountProfile::OtherFilesystem));
        assert_eq!(classify(9, 9), Ok(MountProfile::NotMounted));
    }

    #[test]
    fn a_damaged_table_is_refused_whole() {
        for (table, line) in [
            ("22 1 8:1 / / rw - ext4 /dev/sda1 rw\nnot a mount line\n", 2),
            ("22 1 8:1 / / rw ext4 /dev/sda1 rw\n", 1),
            ("22 1 8-1 / / rw - ext4 /dev/sda1 rw\n", 1),
            ("x 1 8:1 / / rw - ext4 /dev/sda1 rw\n", 1),
            ("22 1 8:1 / / rw - ext4 /dev/sda1\n", 1),
            ("22 1 8:1 / / rw - ext4 /dev/sda1 rw extra\n", 1),
            ("22 1 8:1 /  / rw - ext4 /dev/sda1 rw\n", 1),
            ("22 1 8:1 / / - ext4 /dev/sda1 rw\n", 1),
        ] {
            assert_eq!(
                classify_mountinfo(table.as_bytes(), MountDevice::new(8, 1)),
                Err(MountInfoError::Malformed { line }),
                "{table:?}"
            );
        }
        assert_eq!(
            classify_mountinfo(b"\xff", MountDevice::new(8, 1)),
            Err(MountInfoError::NotUtf8)
        );
        assert_eq!(
            classify_mountinfo(&vec![b'a'; MAX_MOUNTINFO_BYTES + 1], MountDevice::new(8, 1)),
            Err(MountInfoError::TooLarge)
        );
        assert_eq!(
            classify_mountinfo(b"", MountDevice::new(8, 1)),
            Ok(MountProfile::NotMounted)
        );
    }

    #[test]
    fn linux_device_numbers_split_like_glibc() {
        assert_eq!(MountDevice::from_linux_dev(0x0801), MountDevice::new(8, 1));
        // Minor bits above the low eight sit above the twelve major bits.
        assert_eq!(
            MountDevice::from_linux_dev(0x0103_f00a),
            MountDevice::new(0x3f0, 0x100a)
        );
        let device = MountDevice::from_linux_dev((0x1234_5000_u64 << 32) | (0xabc << 8) | 0x7f);
        assert_eq!(device.major(), 0x1234_5abc);
        assert_eq!(device.minor(), 0x7f);
    }
}
