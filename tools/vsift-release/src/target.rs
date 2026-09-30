//! The R0 release targets (ADR 0023 decision D) and the executable format
//! each one must carry.

use std::fmt;

use clap::ValueEnum;

/// One of the three R0 release targets, named by its Rust target triple.
///
/// A closed set: a fourth target joins only with its qualification evidence
/// and an ADR change, never by passing a new triple on the command line.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum ReleaseTarget {
    /// Windows 11 x64, MSVC toolchain with a static C runtime.
    #[value(name = "x86_64-pc-windows-msvc")]
    WindowsX64,
    /// macOS 15 on Apple silicon.
    #[value(name = "aarch64-apple-darwin")]
    MacosArm64,
    /// x64 Linux with glibc, built on Ubuntu 22.04.
    #[value(name = "x86_64-unknown-linux-gnu")]
    LinuxX64,
}

impl ReleaseTarget {
    /// Every target, in the order archives and checksums list them.
    pub(crate) const ALL: [Self; 3] = [Self::MacosArm64, Self::WindowsX64, Self::LinuxX64];

    /// The Rust target triple, which also names the archive.
    pub(crate) const fn triple(self) -> &'static str {
        match self {
            Self::WindowsX64 => "x86_64-pc-windows-msvc",
            Self::MacosArm64 => "aarch64-apple-darwin",
            Self::LinuxX64 => "x86_64-unknown-linux-gnu",
        }
    }

    /// The executable's file name inside the archive: always `vsift`, the
    /// only binary a release ships.
    pub(crate) const fn executable_name(self) -> &'static str {
        match self {
            Self::WindowsX64 => "vsift.exe",
            Self::MacosArm64 | Self::LinuxX64 => "vsift",
        }
    }

    /// The npm package that carries this target's executable (ADR 0023
    /// decision A and its amendment: the `@vsift` scope).
    pub(crate) const fn npm_package_name(self) -> &'static str {
        match self {
            Self::WindowsX64 => "@vsift/win32-x64",
            Self::MacosArm64 => "@vsift/darwin-arm64",
            Self::LinuxX64 => "@vsift/linux-x64",
        }
    }

    /// The directory the npm package is assembled in, which is also the stem
    /// of the file `npm pack` names after the package.
    pub(crate) const fn npm_directory(self) -> &'static str {
        match self {
            Self::WindowsX64 => "vsift-win32-x64",
            Self::MacosArm64 => "vsift-darwin-arm64",
            Self::LinuxX64 => "vsift-linux-x64",
        }
    }

    /// The `os` and `cpu` values the npm package declares: Node.js's
    /// `process.platform` and `process.arch` of the machines it runs on.
    pub(crate) const fn npm_os_and_cpu(self) -> (&'static str, &'static str) {
        match self {
            Self::WindowsX64 => ("win32", "x64"),
            Self::MacosArm64 => ("darwin", "arm64"),
            Self::LinuxX64 => ("linux", "x64"),
        }
    }

    /// The machines the target runs on, in words.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::WindowsX64 => "Windows x64",
            Self::MacosArm64 => "macOS on Apple silicon",
            Self::LinuxX64 => "Linux x64 with glibc",
        }
    }

    /// Whether `bytes` start as an executable of this target's format and
    /// architecture: PE32+ for x86-64, 64-bit Mach-O for arm64, or 64-bit
    /// little-endian ELF for x86-64. Packaging refuses anything else, so a
    /// binary built for another target cannot enter an archive by mistake.
    pub(crate) fn matches_executable(self, bytes: &[u8]) -> bool {
        match self {
            Self::WindowsX64 => is_pe_x86_64(bytes),
            Self::MacosArm64 => is_macho_arm64(bytes),
            Self::LinuxX64 => is_elf_x86_64(bytes),
        }
    }
}

impl fmt::Display for ReleaseTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.triple())
    }
}

fn little_endian_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes(slice.try_into().ok()?))
}

fn little_endian_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes(slice.try_into().ok()?))
}

/// `MZ`, then at the offset stored at 0x3C the `PE\0\0` signature and the
/// machine type `IMAGE_FILE_MACHINE_AMD64` (0x8664).
fn is_pe_x86_64(bytes: &[u8]) -> bool {
    if bytes.get(..2) != Some(b"MZ") {
        return false;
    }
    let Some(header) =
        little_endian_u32(bytes, 0x3C).and_then(|offset| usize::try_from(offset).ok())
    else {
        return false;
    };
    let signature = header.checked_add(4).and_then(|end| bytes.get(header..end));
    signature == Some(b"PE\0\0")
        && header
            .checked_add(4)
            .and_then(|machine| little_endian_u16(bytes, machine))
            == Some(0x8664)
}

/// The 64-bit Mach-O magic `MH_MAGIC_64` (little-endian) and the CPU type
/// `CPU_TYPE_ARM64` (0x0100000C).
fn is_macho_arm64(bytes: &[u8]) -> bool {
    little_endian_u32(bytes, 0) == Some(0xFEED_FACF)
        && little_endian_u32(bytes, 4) == Some(0x0100_000C)
}

/// `\x7fELF`, class 64-bit, little-endian data and machine `EM_X86_64` (62).
fn is_elf_x86_64(bytes: &[u8]) -> bool {
    bytes.get(..4) == Some(b"\x7fELF")
        && bytes.get(4) == Some(&2)
        && bytes.get(5) == Some(&1)
        && little_endian_u16(bytes, 18) == Some(62)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::ReleaseTarget;

    /// The smallest header each target's check accepts, for tests of the
    /// packager that need a binary of the right format.
    pub(crate) fn minimal_executable(target: ReleaseTarget) -> Vec<u8> {
        match target {
            ReleaseTarget::WindowsX64 => {
                let mut bytes = vec![0_u8; 0x48];
                bytes[..2].copy_from_slice(b"MZ");
                bytes[0x3C..0x40].copy_from_slice(&0x40_u32.to_le_bytes());
                bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
                bytes[0x44..0x46].copy_from_slice(&0x8664_u16.to_le_bytes());
                bytes
            }
            ReleaseTarget::MacosArm64 => {
                let mut bytes = Vec::new();
                bytes.extend_from_slice(&0xFEED_FACF_u32.to_le_bytes());
                bytes.extend_from_slice(&0x0100_000C_u32.to_le_bytes());
                bytes.extend_from_slice(&[0; 24]);
                bytes
            }
            ReleaseTarget::LinuxX64 => {
                let mut bytes = vec![0_u8; 64];
                bytes[..4].copy_from_slice(b"\x7fELF");
                bytes[4] = 2;
                bytes[5] = 1;
                bytes[18..20].copy_from_slice(&62_u16.to_le_bytes());
                bytes
            }
        }
    }

    #[test]
    fn each_target_accepts_only_its_own_executable_format() {
        for target in ReleaseTarget::ALL {
            for candidate in ReleaseTarget::ALL {
                assert_eq!(
                    target.matches_executable(&minimal_executable(candidate)),
                    target == candidate,
                    "{target} given a {candidate} executable"
                );
            }
            assert!(!target.matches_executable(b""));
            assert!(!target.matches_executable(b"#!/bin/sh\necho vsift\n"));
        }
    }

    #[test]
    fn a_truncated_or_foreign_architecture_header_is_refused() {
        let mut arm_linux = minimal_executable(ReleaseTarget::LinuxX64);
        arm_linux[18..20].copy_from_slice(&183_u16.to_le_bytes());
        assert!(!ReleaseTarget::LinuxX64.matches_executable(&arm_linux));

        let mut x86_mac = minimal_executable(ReleaseTarget::MacosArm64);
        x86_mac[4..8].copy_from_slice(&0x0100_0007_u32.to_le_bytes());
        assert!(!ReleaseTarget::MacosArm64.matches_executable(&x86_mac));

        let mut arm_windows = minimal_executable(ReleaseTarget::WindowsX64);
        arm_windows[0x44..0x46].copy_from_slice(&0xAA64_u16.to_le_bytes());
        assert!(!ReleaseTarget::WindowsX64.matches_executable(&arm_windows));

        let mut pointing_outside = minimal_executable(ReleaseTarget::WindowsX64);
        pointing_outside[0x3C..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(!ReleaseTarget::WindowsX64.matches_executable(&pointing_outside));
    }
}
