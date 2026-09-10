//! Validated opaque identifiers used at public boundaries.

use std::{error::Error, fmt, str::FromStr};

const MIN_OPAQUE_SUFFIX_LENGTH: usize = 16;
const MAX_OPAQUE_SUFFIX_LENGTH: usize = 64;
const SHA256_HEX_LENGTH: usize = 64;

/// Why a public identifier was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentifierError {
    /// The identifier does not use the prefix required by its type.
    InvalidPrefix,
    /// The opaque component is outside the public size bounds.
    InvalidLength,
    /// The identifier contains characters outside its canonical alphabet.
    InvalidCharacter,
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidPrefix => "identifier has the wrong type prefix",
            Self::InvalidLength => "identifier length is outside the allowed range",
            Self::InvalidCharacter => "identifier is not in canonical form",
        };
        formatter.write_str(message)
    }
}

impl Error for IdentifierError {}

macro_rules! opaque_identifier {
    ($name:ident, $prefix:literal, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Parses and validates the canonical public representation.
            ///
            /// # Errors
            ///
            /// Returns [`IdentifierError`] when prefix, length, or characters are invalid.
            pub fn parse(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                validate_opaque_identifier(&value, $prefix)?;
                Ok(Self(value))
            }

            /// Returns the canonical public representation.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = IdentifierError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

opaque_identifier!(
    SessionId,
    "ses_",
    "Opaque identity for one disposable or retained investigation session."
);
opaque_identifier!(JobId, "job_", "Opaque identity for one worker job.");
opaque_identifier!(
    OperationId,
    "op_",
    "Opaque identity used to recover the result of one public operation."
);
opaque_identifier!(
    ArtifactId,
    "art_",
    "Opaque identity for one committed derived artifact."
);
opaque_identifier!(
    EvidenceId,
    "evd_",
    "Opaque identity for one source-grounded evidence item."
);

/// Cryptographic identity of immutable source bytes.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceId(String);

impl SourceId {
    const PREFIX: &'static str = "src_sha256_";

    /// Creates a source identity from a canonical lowercase SHA-256 digest.
    ///
    /// # Errors
    ///
    /// Returns [`IdentifierError`] when the digest is not 64 lowercase hexadecimal bytes.
    pub fn from_sha256(hex_digest: &str) -> Result<Self, IdentifierError> {
        validate_digest(hex_digest)?;
        Ok(Self(format!("{}{hex_digest}", Self::PREFIX)))
    }

    /// Parses the canonical public representation.
    ///
    /// # Errors
    ///
    /// Returns [`IdentifierError`] when prefix or digest is invalid.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        let digest = value
            .strip_prefix(Self::PREFIX)
            .ok_or(IdentifierError::InvalidPrefix)?;
        validate_digest(digest)?;
        Ok(Self(value))
    }

    /// Returns the canonical public representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for SourceId {
    type Err = IdentifierError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Digest of canonical operation inputs and compatibility-affecting policy.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OperationKey(String);

impl OperationKey {
    const PREFIX: &'static str = "opk_sha256_";

    /// Creates an operation key from a canonical lowercase SHA-256 digest.
    ///
    /// # Errors
    ///
    /// Returns [`IdentifierError`] when the digest is not 64 lowercase hexadecimal bytes.
    pub fn from_sha256(hex_digest: &str) -> Result<Self, IdentifierError> {
        validate_digest(hex_digest)?;
        Ok(Self(format!("{}{hex_digest}", Self::PREFIX)))
    }

    /// Parses the canonical public representation.
    ///
    /// # Errors
    ///
    /// Returns [`IdentifierError`] when prefix or digest is invalid.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        let digest = value
            .strip_prefix(Self::PREFIX)
            .ok_or(IdentifierError::InvalidPrefix)?;
        validate_digest(digest)?;
        Ok(Self(value))
    }

    /// Returns the canonical public representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for OperationKey {
    type Err = IdentifierError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl fmt::Display for OperationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

fn validate_opaque_identifier(value: &str, prefix: &str) -> Result<(), IdentifierError> {
    let suffix = value
        .strip_prefix(prefix)
        .ok_or(IdentifierError::InvalidPrefix)?;
    if !(MIN_OPAQUE_SUFFIX_LENGTH..=MAX_OPAQUE_SUFFIX_LENGTH).contains(&suffix.len()) {
        return Err(IdentifierError::InvalidLength);
    }
    if !suffix
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return Err(IdentifierError::InvalidCharacter);
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<(), IdentifierError> {
    if value.len() != SHA256_HEX_LENGTH {
        return Err(IdentifierError::InvalidLength);
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(IdentifierError::InvalidCharacter);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{IdentifierError, OperationKey, SessionId, SourceId};

    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn opaque_ids_reject_paths_options_and_control_characters() {
        for value in [
            "ses_../../outside0000",
            "ses_--providerflag00",
            "ses_shell&payload000",
            "ses_spaces are data",
            "ses_unicodeépayload",
            "ses_Uppercaseletters",
            "ses_newline0000000\n",
            "job_0123456789abcdef",
        ] {
            assert!(SessionId::parse(value).is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn opaque_ids_require_a_bounded_suffix() {
        assert_eq!(
            SessionId::parse("ses_short").err(),
            Some(IdentifierError::InvalidLength)
        );
        assert!(SessionId::parse("ses_0123456789abcdef").is_ok());
        assert!(SessionId::parse(format!("ses_{}", "a".repeat(64))).is_ok());
        assert_eq!(
            SessionId::parse(format!("ses_{}", "a".repeat(65))).err(),
            Some(IdentifierError::InvalidLength)
        );
    }

    #[test]
    fn digest_ids_require_canonical_lowercase_sha256() {
        assert!(SourceId::from_sha256(DIGEST).is_ok());
        assert!(OperationKey::from_sha256(DIGEST).is_ok());
        assert!(SourceId::from_sha256(&DIGEST.to_uppercase()).is_err());
        assert!(OperationKey::from_sha256("abc").is_err());
    }
}
