//! Immutable integrity requirements for reviewed managed artifacts.

use std::{error::Error, fmt};

/// Maximum compressed bytes accepted by the initial managed-transfer policy.
pub const MAX_MANAGED_ARTIFACT_BYTES: u64 = 1_073_741_824;

/// Host profiles for which managed runtime compatibility may be reviewed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedTarget {
    /// Ubuntu 24.04 on the x86-64 architecture.
    Ubuntu2404X86_64,
    /// A Windows x86-64 host without an accepted managed catalogue entry.
    WindowsX86_64,
    /// An Apple macOS ARM64 host without an accepted managed catalogue entry.
    MacOsArm64,
    /// A host outside the explicitly named R0 managed profiles.
    Unsupported,
}

impl ManagedTarget {
    /// Stable identifier used by setup plans and acceptance digests.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Ubuntu2404X86_64 => "ubuntu_24_04_x86_64",
            Self::WindowsX86_64 => "windows_x86_64",
            Self::MacOsArm64 => "macos_arm64",
            Self::Unsupported => "unsupported",
        }
    }
}

/// A separately reviewed managed-install component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedComponent {
    /// One archive supplying the paired `FFmpeg` and `FFprobe` executables.
    MediaTools,
    /// The whisper.cpp command-line runtime and its required libraries.
    WhisperCli,
    /// Multilingual Whisper model weights selected for local ASR.
    WhisperModel,
}

impl ManagedComponent {
    /// Stable identifier used by the catalogue and public plans.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::MediaTools => "ffmpeg_ffprobe",
            Self::WhisperCli => "whisper_cli",
            Self::WhisperModel => "whisper_model",
        }
    }
}

/// Packaging format accepted by the reviewed extraction path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedArtifactFormat {
    /// An XZ-compressed tar archive.
    TarXz,
    /// A gzip-compressed tar archive.
    TarGz,
    /// One unarchived regular file.
    RawFile,
}

impl ManagedArtifactFormat {
    /// Stable identifier used in reviewable plans.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::TarXz => "tar_xz",
            Self::TarGz => "tar_gz",
            Self::RawFile => "raw_file",
        }
    }
}

/// Invalid reviewed size or canonical SHA-256 metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactIntegrityError {
    /// A zero-byte artifact cannot be installed.
    Empty,
    /// The artifact exceeds the bounded transfer policy.
    TooLarge,
    /// The digest is not exactly 64 lowercase hexadecimal characters.
    InvalidSha256,
}

impl fmt::Display for ArtifactIntegrityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "managed artifact size must be positive",
            Self::TooLarge => "managed artifact exceeds the transfer limit",
            Self::InvalidSha256 => "managed artifact SHA-256 is not canonical",
        })
    }
}

impl Error for ArtifactIntegrityError {}

/// Exact bytes and SHA-256 pinned by a reviewed source catalogue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactIntegrity {
    bytes: u64,
    sha256: [u8; 32],
}

impl ArtifactIntegrity {
    /// Validates the pinned size and canonical SHA-256 digest.
    ///
    /// # Errors
    ///
    /// Rejects zero/oversized artifacts and malformed or noncanonical digests.
    pub fn from_sha256_hex(bytes: u64, sha256: &str) -> Result<Self, ArtifactIntegrityError> {
        if bytes == 0 {
            return Err(ArtifactIntegrityError::Empty);
        }
        if bytes > MAX_MANAGED_ARTIFACT_BYTES {
            return Err(ArtifactIntegrityError::TooLarge);
        }
        let encoded = sha256.as_bytes();
        if encoded.len() != 64
            || !encoded
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(ArtifactIntegrityError::InvalidSha256);
        }
        let mut digest = [0_u8; 32];
        for (index, pair) in encoded.as_chunks::<2>().0.iter().enumerate() {
            let upper = hex_nibble(pair[0]).ok_or(ArtifactIntegrityError::InvalidSha256)?;
            let lower = hex_nibble(pair[1]).ok_or(ArtifactIntegrityError::InvalidSha256)?;
            digest[index] = (upper << 4) | lower;
        }
        Ok(Self {
            bytes,
            sha256: digest,
        })
    }

    /// Expected exact compressed artifact size.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.bytes
    }

    /// Expected whole-artifact SHA-256 bytes.
    #[must_use]
    pub const fn sha256(self) -> [u8; 32] {
        self.sha256
    }

    /// Returns the canonical lowercase hexadecimal SHA-256 representation.
    #[must_use]
    pub fn sha256_hex(self) -> String {
        use fmt::Write as _;

        let mut encoded = String::with_capacity(64);
        for byte in self.sha256 {
            let _ = write!(&mut encoded, "{byte:02x}");
        }
        encoded
    }
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{ArtifactIntegrity, ArtifactIntegrityError, MAX_MANAGED_ARTIFACT_BYTES};

    #[test]
    fn requires_bounded_size_and_canonical_digest() -> Result<(), ArtifactIntegrityError> {
        let digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let valid = ArtifactIntegrity::from_sha256_hex(3, digest);
        assert!(valid.is_ok());
        assert_eq!(
            ArtifactIntegrity::from_sha256_hex(0, digest),
            Err(ArtifactIntegrityError::Empty)
        );
        assert_eq!(
            ArtifactIntegrity::from_sha256_hex(MAX_MANAGED_ARTIFACT_BYTES + 1, digest),
            Err(ArtifactIntegrityError::TooLarge)
        );
        assert_eq!(
            ArtifactIntegrity::from_sha256_hex(3, &digest.to_ascii_uppercase()),
            Err(ArtifactIntegrityError::InvalidSha256)
        );
        assert_eq!(
            ArtifactIntegrity::from_sha256_hex(3, "not a digest"),
            Err(ArtifactIntegrityError::InvalidSha256)
        );
        assert_eq!(valid?.sha256_hex(), digest);
        Ok(())
    }
}
