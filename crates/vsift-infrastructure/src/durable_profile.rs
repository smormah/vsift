//! Which publication guarantee a session root may offer (ADR 0010, ADR 0020).
//!
//! ADR 0010 qualifies OS-crash-durable publication only on Ubuntu 24.04 with a
//! local ext4 filesystem, and only after an owned crash campaign has passed.
//! The store therefore claims [`PublicationGuarantee::OsCrashDurable`] only
//! when all of these hold (the decision table is [`qualifies`]):
//!
//! - the build targets Linux;
//! - `/etc/os-release` (or `/usr/lib/os-release`), read with a bound and
//!   parsed strictly by [`classify_os_release`], names Ubuntu 24.04;
//! - the root's device appears in `/proc/self/mountinfo` only as ext4 mounts
//!   that do not disable write barriers (`nobarrier` or `barrier=0`), read
//!   with a bound and parsed strictly by [`classify_mountinfo`];
//! - the campaign constant `QUALIFIED_UBUNTU_EXT4` is set, which it is since
//!   P10 PR 4's crash campaign passed (`docs/planning/p10-durable-publication.md`).
//!
//! Every other profile answers a durable request with an unsupported-guarantee
//! error before anything is changed. Anything that cannot be read or parsed
//! fails closed. Both parsers are
//! public so the fuzz harness reaches them through the same surface as the
//! store.

use std::{error::Error, fmt};

use cap_std::fs::Dir;
use vsift_application::StorageCapabilities;
use vsift_domain::PublicationGuarantee;

/// Largest `/proc/self/mountinfo` the profile check reads: 1 MiB, far above a
/// host's mount table and small enough to hold in memory.
pub const MAX_MOUNTINFO_BYTES: usize = 1024 * 1024;

/// Largest os-release file the profile check reads: 64 KiB, far above any
/// distribution's file.
pub const MAX_OS_RELEASE_BYTES: usize = 64 * 1024;

/// Set: the Ubuntu 24.04 / ext4 crash campaign passed (P10 PR 4, ADR 0020
/// section 7; evidence and run links in the P10 durable-publication record).
/// Clearing it withdraws the durable profile everywhere.
#[cfg(target_os = "linux")]
const QUALIFIED_UBUNTU_EXT4: bool = true;

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

/// What an os-release file says about the distribution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OsReleaseProfile {
    /// `ID=ubuntu` and `VERSION_ID=24.04`: the one qualified release.
    Ubuntu2404,
    /// Any other distribution or release, or a file naming none.
    Other,
}

/// Why an os-release file was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OsReleaseError {
    /// Larger than [`MAX_OS_RELEASE_BYTES`].
    TooLarge,
    /// Not UTF-8 text.
    NotUtf8,
    /// A line (numbered from 1) is not blank, a comment or a documented
    /// `KEY=value` assignment.
    Malformed {
        /// The 1-based line number.
        line: usize,
    },
}

impl fmt::Display for OsReleaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => formatter.write_str("the os-release file is too large"),
            Self::NotUtf8 => formatter.write_str("the os-release file is not UTF-8"),
            Self::Malformed { line } => write!(formatter, "os-release line {line} is malformed"),
        }
    }
}

impl Error for OsReleaseError {}

/// Classifies an `os-release(5)` file.
///
/// Every line is parsed, so a file damaged anywhere is refused rather than
/// half trusted. A line is blank, a comment starting with `#`, or
/// `KEY=value`: the key is ASCII letters, digits and underscores, not
/// starting with a digit; the value is unquoted (no whitespace, quotes,
/// backslashes, `$` or backticks), wholly single-quoted, or wholly
/// double-quoted with a backslash escaping only `"`, `\`, `$` and a
/// backtick. As in the shell the format mirrors, a later assignment of the
/// same key wins. The file names the qualified release only when `ID` is
/// exactly `ubuntu` and `VERSION_ID` exactly `24.04`; a derivative that
/// lists Ubuntu only in `ID_LIKE` is another distribution, because the
/// campaign qualified Ubuntu's own kernel and packages.
///
/// # Errors
///
/// A file over [`MAX_OS_RELEASE_BYTES`], not UTF-8, or with a malformed line.
pub fn classify_os_release(text: &[u8]) -> Result<OsReleaseProfile, OsReleaseError> {
    if text.len() > MAX_OS_RELEASE_BYTES {
        return Err(OsReleaseError::TooLarge);
    }
    let text = std::str::from_utf8(text).map_err(|_| OsReleaseError::NotUtf8)?;
    let mut id = None;
    let mut version = None;
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (key, value) =
            parse_assignment(trimmed).ok_or(OsReleaseError::Malformed { line: index + 1 })?;
        match key {
            "ID" => id = Some(value),
            "VERSION_ID" => version = Some(value),
            _ => {}
        }
    }
    Ok(
        if id.as_deref() == Some("ubuntu") && version.as_deref() == Some("24.04") {
            OsReleaseProfile::Ubuntu2404
        } else {
            OsReleaseProfile::Other
        },
    )
}

/// One `KEY=value` line of an os-release file, its value unquoted.
fn parse_assignment(line: &str) -> Option<(&str, String)> {
    let (key, raw) = line.split_once('=')?;
    let mut characters = key.chars();
    let first = characters.next()?;
    if !(first.is_ascii_alphabetic() || first == '_')
        || !characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return None;
    }
    Some((key, parse_value(raw)?))
}

/// The value of an assignment with its quoting removed.
fn parse_value(raw: &str) -> Option<String> {
    if let Some(inner) = raw.strip_prefix('\'') {
        let inner = inner.strip_suffix('\'')?;
        return (!inner.contains('\'')).then(|| inner.to_owned());
    }
    if let Some(inner) = raw.strip_prefix('"') {
        let inner = inner.strip_suffix('"')?;
        let mut value = String::with_capacity(inner.len());
        let mut characters = inner.chars();
        while let Some(character) = characters.next() {
            match character {
                '\\' => {
                    let escaped = characters.next()?;
                    if !is_shell_special(escaped) || escaped == '\'' {
                        return None;
                    }
                    value.push(escaped);
                }
                '"' | '$' | '`' => return None,
                other => value.push(other),
            }
        }
        return Some(value);
    }
    (!raw.contains(|character: char| character.is_whitespace() || is_shell_special(character)))
        .then(|| raw.to_owned())
}

/// Characters an unquoted os-release value may not contain.
const fn is_shell_special(character: char) -> bool {
    matches!(character, '"' | '\'' | '\\' | '$' | '`')
}

/// The decision table of ADR 0010 and ADR 0020: a root claims OS-crash
/// durability only when the campaign has passed, the host is Ubuntu 24.04
/// and the root's device is mounted only as ext4 with write barriers.
/// Anything that could not be read or parsed is `None` and fails closed.
#[must_use]
pub const fn qualifies(
    campaign_passed: bool,
    os: Option<OsReleaseProfile>,
    mount: Option<MountProfile>,
) -> bool {
    campaign_passed
        && matches!(os, Some(OsReleaseProfile::Ubuntu2404))
        && matches!(mount, Some(MountProfile::Ext4WithBarriers))
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
    if !QUALIFIED_UBUNTU_EXT4 {
        return false;
    }
    let os = read_bounded_file(
        &["/etc/os-release", "/usr/lib/os-release"],
        MAX_OS_RELEASE_BYTES,
    )
    .and_then(|text| classify_os_release(&text).ok());
    let mount = root.dir_metadata().ok().and_then(|metadata| {
        let device = MountDevice::from_linux_dev(cap_std::fs::MetadataExt::dev(&metadata));
        read_bounded_file(&["/proc/self/mountinfo"], MAX_MOUNTINFO_BYTES)
            .and_then(|table| classify_mountinfo(&table, device).ok())
    });
    qualifies(QUALIFIED_UBUNTU_EXT4, os, mount)
}

/// Reads the first of `paths` that opens: at most `limit` bytes and one more,
/// so an oversized file is refused by its parser rather than truncated.
#[cfg(target_os = "linux")]
fn read_bounded_file(paths: &[&str], limit: usize) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let file = paths
        .iter()
        .find_map(|path| std::fs::File::open(path).ok())?;
    let mut bytes = Vec::new();
    let limit = u64::try_from(limit).unwrap_or(u64::MAX);
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

/// No other target has a qualified durable profile (ADR 0010).
#[cfg(not(target_os = "linux"))]
fn os_crash_durable(_root: &Dir) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_MOUNTINFO_BYTES, MAX_OS_RELEASE_BYTES, MountDevice, MountInfoError, MountProfile,
        OsReleaseError, OsReleaseProfile, classify_mountinfo, classify_os_release, qualifies,
    };

    /// Ubuntu 24.04's own file, as `/usr/lib/os-release` ships it.
    const NOBLE: &str = r#"PRETTY_NAME="Ubuntu 24.04.3 LTS"
NAME="Ubuntu"
VERSION_ID="24.04"
VERSION="24.04.3 LTS (Noble Numbat)"
VERSION_CODENAME=noble
ID=ubuntu
ID_LIKE=debian
HOME_URL="https://www.ubuntu.com/"
SUPPORT_URL="https://help.ubuntu.com/"
BUG_REPORT_URL="https://bugs.launchpad.net/ubuntu/"
PRIVACY_POLICY_URL="https://www.ubuntu.com/legal/terms-and-policies/privacy-policy"
UBUNTU_CODENAME=noble
LOGO=ubuntu-logo
"#;

    #[test]
    fn only_ubuntu_24_04_itself_is_the_qualified_release() {
        assert_eq!(
            classify_os_release(NOBLE.as_bytes()),
            Ok(OsReleaseProfile::Ubuntu2404)
        );
        for (text, expected) in [
            (
                "ID=ubuntu\nVERSION_ID=24.04\n",
                OsReleaseProfile::Ubuntu2404,
            ),
            (
                "ID='ubuntu'\nVERSION_ID='24.04'",
                OsReleaseProfile::Ubuntu2404,
            ),
            (
                "# comment\n\n  ID=ubuntu  \nVERSION_ID=\"24.04\"\n",
                OsReleaseProfile::Ubuntu2404,
            ),
            // A later assignment wins, as in the shell.
            (
                "ID=debian\nID=ubuntu\nVERSION_ID=24.04\n",
                OsReleaseProfile::Ubuntu2404,
            ),
            (
                "ID=ubuntu\nVERSION_ID=24.04\nID=debian\n",
                OsReleaseProfile::Other,
            ),
            ("ID=ubuntu\nVERSION_ID=\"22.04\"\n", OsReleaseProfile::Other),
            ("ID=ubuntu\nVERSION_ID=\"24.10\"\n", OsReleaseProfile::Other),
            (
                "ID=ubuntu\nVERSION_ID=\"24.04.3\"\n",
                OsReleaseProfile::Other,
            ),
            ("ID=Ubuntu\nVERSION_ID=24.04\n", OsReleaseProfile::Other),
            (
                "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\nVERSION_ID=24.04\n",
                OsReleaseProfile::Other,
            ),
            ("ID=ubuntu\n", OsReleaseProfile::Other),
            ("VERSION_ID=24.04\n", OsReleaseProfile::Other),
            ("", OsReleaseProfile::Other),
            (
                "ID=ubuntu\nVERSION_ID=24.04\nNAME=\"a \\\"quoted\\\" \\$ \\` \\\\ name\"\n",
                OsReleaseProfile::Ubuntu2404,
            ),
        ] {
            assert_eq!(
                classify_os_release(text.as_bytes()),
                Ok(expected),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_damaged_os_release_is_refused_whole() {
        for (text, line) in [
            ("ID=ubuntu\nVERSION_ID=24.04\nnot an assignment\n", 3),
            ("ID ubuntu\n", 1),
            ("=ubuntu\n", 1),
            ("1D=ubuntu\n", 1),
            ("I-D=ubuntu\n", 1),
            ("ID=ubu ntu\n", 1),
            ("ID=\"ubuntu\n", 1),
            ("ID='ubuntu\n", 1),
            ("ID=ubuntu\"\n", 1),
            ("ID='ub'untu'\n", 1),
            ("ID=\"ub\"untu\"\n", 1),
            ("ID=\"$(reboot)\"\n", 1),
            ("ID=\"`reboot`\"\n", 1),
            ("ID=\"ubuntu\\n\"\n", 1),
            ("ID=\"ubuntu\\\"\n", 1),
            ("ID=$ubuntu\n", 1),
            ("ID=ubuntu\\\n", 1),
            ("ID=ubuntu\nVERSION_ID=24.04;reboot\nX=a b\n", 3),
        ] {
            assert_eq!(
                classify_os_release(text.as_bytes()),
                Err(OsReleaseError::Malformed { line }),
                "{text:?}"
            );
        }
        assert_eq!(
            classify_os_release(b"ID=\xff\n"),
            Err(OsReleaseError::NotUtf8)
        );
        let mut oversized = b"ID=ubuntu\nVERSION_ID=24.04\n".to_vec();
        oversized.resize(MAX_OS_RELEASE_BYTES + 1, b'\n');
        assert_eq!(
            classify_os_release(&oversized),
            Err(OsReleaseError::TooLarge)
        );
        oversized.truncate(MAX_OS_RELEASE_BYTES);
        assert_eq!(
            classify_os_release(&oversized),
            Ok(OsReleaseProfile::Ubuntu2404)
        );
    }

    /// Every combination of the three inputs: only a passed campaign on
    /// Ubuntu 24.04 over ext4 with barriers qualifies; anything unread or
    /// unparsed (`None`) fails closed.
    #[test]
    fn the_decision_table_qualifies_one_combination_only() {
        let oses = [
            None,
            Some(OsReleaseProfile::Ubuntu2404),
            Some(OsReleaseProfile::Other),
        ];
        let mounts = [
            None,
            Some(MountProfile::Ext4WithBarriers),
            Some(MountProfile::Ext4WithoutBarriers),
            Some(MountProfile::OtherFilesystem),
            Some(MountProfile::NotMounted),
        ];
        let mut qualified = 0;
        for campaign in [false, true] {
            for os in oses {
                for mount in mounts {
                    let expected = campaign
                        && os == Some(OsReleaseProfile::Ubuntu2404)
                        && mount == Some(MountProfile::Ext4WithBarriers);
                    assert_eq!(
                        qualifies(campaign, os, mount),
                        expected,
                        "{campaign} {os:?} {mount:?}"
                    );
                    qualified += usize::from(expected);
                }
            }
        }
        assert_eq!(qualified, 1);
    }

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
