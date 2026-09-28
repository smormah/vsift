//! Reading one bounded JSON document from a file the caller names.
//!
//! Decoding is the contract's strict bounded decoder
//! ([`vsift_contract::decode_strict_json`]), shared by every host since P11;
//! this module only reads the file without first allocating for an
//! untrusted size.

use std::{
    error::Error,
    fmt,
    fs::File,
    io::{Read, Take},
    path::Path,
};

use serde::de::DeserializeOwned;
use vsift::FailureCode;
use vsift_contract::{JsonLimits, StrictJsonError, decode_strict_json};

/// Reads and decodes one document of at most `limits.max_bytes`.
pub(crate) fn read_json_file<T>(path: &Path, limits: JsonLimits) -> Result<T, JsonInputError>
where
    T: DeserializeOwned,
{
    let file = File::open(path).map_err(JsonInputError::Io)?;
    let budget = u64::try_from(limits.max_bytes).map_or(u64::MAX, |bytes| bytes.saturating_add(1));
    let mut bounded: Take<File> = file.take(budget);
    let mut bytes = Vec::with_capacity(limits.max_bytes.min(64 * 1024));
    bounded
        .read_to_end(&mut bytes)
        .map_err(JsonInputError::Io)?;
    decode_strict_json(&bytes, limits).map_err(JsonInputError::Decode)
}

/// Why a JSON document file was refused before it was used.
#[derive(Debug)]
pub(crate) enum JsonInputError {
    /// The file could not be opened or read.
    Io(std::io::Error),
    /// The bytes were refused by the strict decoder.
    Decode(StrictJsonError),
}

impl JsonInputError {
    /// Returns the stable public failure code.
    #[must_use]
    pub(crate) const fn code(&self) -> FailureCode {
        match self {
            Self::Io(_) => FailureCode::StorageIo,
            Self::Decode(error) => error.code(),
        }
    }
}

impl fmt::Display for JsonInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => formatter.write_str("JSON document could not be read"),
            Self::Decode(error) => error.fmt(formatter),
        }
    }
}

impl Error for JsonInputError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Decode(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use vsift_contract::{JsonLimits, StrictJsonError};

    use super::{JsonInputError, read_json_file};

    #[test]
    fn file_reader_stops_after_the_byte_budget() -> Result<(), Box<dyn std::error::Error>> {
        let mut random = [0_u8; 12];
        getrandom::fill(&mut random).map_err(|_| std::io::Error::other("random source failed"))?;
        let path = std::env::temp_dir().join(format!("vsift-plan-{}.json", hex(&random)));
        std::fs::write(&path, vec![b' '; JsonLimits::DOCUMENT.max_bytes + 1])?;

        let result = read_json_file::<serde_json::Value>(&path, JsonLimits::DOCUMENT);
        std::fs::remove_file(&path)?;

        assert!(matches!(
            result,
            Err(JsonInputError::Decode(StrictJsonError::TooLarge))
        ));
        Ok(())
    }

    #[test]
    fn a_missing_file_is_a_storage_failure() {
        let result = read_json_file::<serde_json::Value>(
            std::path::Path::new("vsift-no-such-plan.json"),
            JsonLimits::DOCUMENT,
        );
        assert!(matches!(result, Err(error) if error.code() == vsift::FailureCode::StorageIo));
    }

    fn hex(bytes: &[u8]) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut encoded = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
            encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
        }
        encoded
    }
}
