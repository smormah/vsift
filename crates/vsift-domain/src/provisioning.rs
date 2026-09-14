//! Immutable integrity requirements for reviewed managed artifacts.

use std::{error::Error, fmt};

/// Maximum compressed bytes accepted by the initial managed-transfer policy.
pub const MAX_MANAGED_ARTIFACT_BYTES: u64 = 1_073_741_824;

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
    fn requires_bounded_size_and_canonical_digest() {
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
    }
}
