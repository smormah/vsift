//! Identity of a whisper.cpp CLI build, taken from its bytes rather than its output.
//!
//! `whisper-cli` prints loader diagnostics that name absolute library paths
//! before anything else and has no stable version banner, so its output is
//! neither safe to echo nor evidence of which build ran. The executable's
//! bounded SHA-256, compared with the builds reviewed for P06, is both: it
//! never contains a path, and it is the identity a transcript's provenance
//! records.

use std::{error::Error, fmt, io, path::Path};

use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::TrustedExecutable;

/// Largest executable the build identity will hash.
///
/// The reviewed builds are under 1 MiB; a generous bound still keeps a hostile
/// or mistaken selection (a disk image renamed `whisper-cli`) from turning a
/// diagnostic into an unbounded read.
pub const MAX_WHISPER_EXECUTABLE_BYTES: u64 = 64 * 1024 * 1024;
const HASH_CHUNK_BYTES: usize = 64 * 1024;

/// The whisper.cpp v1.9.2 CLI builds reviewed in P06.
///
/// Windows: `Release/whisper-cli.exe` from the official x64 CPU archive
/// (`docs/planning/p06-windows-artifact-candidate.md`). Ubuntu: `whisper-cli`
/// from the reviewed Ubuntu 24.04 x86-64 catalogue
/// (`docs/planning/p06-ubuntu-artifact-candidate.md`).
const REVIEWED_BUILDS: [(u64, &str); 2] = [
    (
        479_232,
        "95e3c0b0e778ad9499eb0125f97c1dcf437dd9eb4ea77050b043574f93c2631d",
    ),
    (
        976_312,
        "61fa94d25ba9a4695118883011f35e8521c158145ec73bcd8805a7c11760e6d7",
    ),
];

/// Whether an executable is one of the reviewed whisper.cpp builds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WhisperBuildRecognition {
    /// Byte-identical to a reviewed whisper.cpp v1.9.2 CLI build.
    ReviewedV1_9_2,
    /// Any other build; it may still work but nothing about it was reviewed.
    Unrecognised,
}

/// Size and SHA-256 of one whisper.cpp CLI executable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WhisperBuildIdentity {
    bytes: u64,
    sha256: [u8; 32],
}

impl WhisperBuildIdentity {
    /// Executable size in bytes.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.bytes
    }

    /// SHA-256 of the executable bytes.
    #[must_use]
    pub const fn sha256(self) -> [u8; 32] {
        self.sha256
    }

    /// Lowercase hexadecimal SHA-256, the form recorded in provenance.
    #[must_use]
    pub fn sha256_hex(self) -> String {
        hex(&self.sha256)
    }

    /// Whether this is a reviewed build.
    #[must_use]
    pub fn recognition(self) -> WhisperBuildRecognition {
        let digest = self.sha256_hex();
        if REVIEWED_BUILDS
            .iter()
            .any(|(bytes, sha256)| *bytes == self.bytes && *sha256 == digest)
        {
            WhisperBuildRecognition::ReviewedV1_9_2
        } else {
            WhisperBuildRecognition::Unrecognised
        }
    }
}

/// Why a whisper.cpp build could not be identified.
#[derive(Debug)]
pub enum WhisperBuildError {
    /// The executable is larger than [`MAX_WHISPER_EXECUTABLE_BYTES`].
    TooLarge,
    /// The executable changed size while it was being read.
    Changed,
    /// The executable could not be read.
    Io(io::Error),
}

impl fmt::Display for WhisperBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "whisper executable exceeds the identity size bound",
            Self::Changed => "whisper executable changed while it was identified",
            Self::Io(_) => "whisper executable could not be read",
        })
    }
}

impl Error for WhisperBuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::TooLarge | Self::Changed => None,
        }
    }
}

/// Hashes a trusted whisper.cpp executable within [`MAX_WHISPER_EXECUTABLE_BYTES`].
///
/// # Errors
///
/// Fails for an oversized, changing or unreadable executable.
pub async fn identify_whisper_build(
    executable: &TrustedExecutable,
) -> Result<WhisperBuildIdentity, WhisperBuildError> {
    let (bytes, sha256) =
        hash_file_bounded(executable.path(), MAX_WHISPER_EXECUTABLE_BYTES).await?;
    Ok(WhisperBuildIdentity { bytes, sha256 })
}

/// Size and SHA-256 of a regular file of at most `max_bytes`, read once.
///
/// # Errors
///
/// Fails for an oversized, non-regular, changing or unreadable file.
pub(crate) async fn hash_file_bounded(
    path: &Path,
    max_bytes: u64,
) -> Result<(u64, [u8; 32]), WhisperBuildError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(WhisperBuildError::Io)?;
    let metadata = file.metadata().await.map_err(WhisperBuildError::Io)?;
    if !metadata.is_file() {
        return Err(WhisperBuildError::Changed);
    }
    let length = metadata.len();
    if length > max_bytes {
        return Err(WhisperBuildError::TooLarge);
    }
    let mut reader = file.take(length.saturating_add(1));
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; HASH_CHUNK_BYTES];
    let mut total: u64 = 0;
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(WhisperBuildError::Io)?;
        if read == 0 {
            break;
        }
        hasher.update(buffer.get(..read).ok_or(WhisperBuildError::Changed)?);
        total = total.saturating_add(u64::try_from(read).map_err(|_| WhisperBuildError::Changed)?);
    }
    if total != length {
        return Err(WhisperBuildError::Changed);
    }
    Ok((length, hasher.finalize().into()))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{WhisperBuildIdentity, WhisperBuildRecognition};

    fn digest(text: &str) -> Result<[u8; 32], Box<dyn std::error::Error>> {
        let mut bytes = [0_u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(text.get(index * 2..index * 2 + 2).ok_or("short")?, 16)?;
        }
        Ok(bytes)
    }

    #[test]
    fn only_the_exact_reviewed_bytes_are_recognised() -> Result<(), Box<dyn std::error::Error>> {
        let bytes = 479_232;
        let sha256 = digest("95e3c0b0e778ad9499eb0125f97c1dcf437dd9eb4ea77050b043574f93c2631d")?;
        let reviewed = WhisperBuildIdentity { bytes, sha256 };
        assert_eq!(
            reviewed.recognition(),
            WhisperBuildRecognition::ReviewedV1_9_2
        );
        let resized = WhisperBuildIdentity {
            bytes: bytes + 1,
            sha256,
        };
        assert_eq!(resized.recognition(), WhisperBuildRecognition::Unrecognised);
        let mut changed = sha256;
        changed[31] ^= 1;
        let rebuilt = WhisperBuildIdentity {
            bytes,
            sha256: changed,
        };
        assert_eq!(rebuilt.recognition(), WhisperBuildRecognition::Unrecognised);
        assert_eq!(
            WhisperBuildIdentity {
                bytes: 976_312,
                sha256: digest("61fa94d25ba9a4695118883011f35e8521c158145ec73bcd8805a7c11760e6d7")?,
            }
            .recognition(),
            WhisperBuildRecognition::ReviewedV1_9_2
        );
        Ok(())
    }
}
