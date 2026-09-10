//! Bounded JSON decoding for future noninteractive request commands.

#![allow(
    dead_code,
    reason = "P01 freezes the bounded decoder before P11 admits job request files"
)]

use std::{error::Error, fmt};

use serde::de::DeserializeOwned;
use vsift_domain::FailureCode;

/// Maximum accepted bytes for one R0 JSON request document.
pub(crate) const MAX_JSON_INPUT_BYTES: usize = 1_048_576;
/// Maximum object/array nesting before deserialization begins.
pub(crate) const MAX_JSON_NESTING: usize = 64;

/// A supported public contract major.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SchemaVersion {
    /// R0 schema major.
    V1,
}

impl SchemaVersion {
    /// Rejects unknown majors instead of guessing compatibility.
    pub(crate) fn parse(value: &str) -> Result<Self, JsonInputError> {
        match value {
            "1" => Ok(Self::V1),
            _ => Err(JsonInputError::UnsupportedVersion),
        }
    }
}

/// Decodes one bounded strict request. Individual request structs must deny unknown fields.
pub(crate) fn decode_json<T>(bytes: &[u8]) -> Result<T, JsonInputError>
where
    T: DeserializeOwned,
{
    if bytes.len() > MAX_JSON_INPUT_BYTES {
        return Err(JsonInputError::TooLarge);
    }
    validate_nesting(bytes)?;
    serde_json::from_slice(bytes).map_err(JsonInputError::Malformed)
}

/// Why a noninteractive JSON request was rejected before execution.
#[derive(Debug)]
pub(crate) enum JsonInputError {
    /// The request exceeded the input byte budget.
    TooLarge,
    /// Object/array nesting exceeded the parser budget.
    TooDeep,
    /// The bytes were not a valid instance of the strict request type.
    Malformed(serde_json::Error),
    /// The request used an unknown schema major.
    UnsupportedVersion,
}

impl JsonInputError {
    /// Returns the stable public failure code.
    #[must_use]
    pub(crate) const fn code(&self) -> FailureCode {
        match self {
            Self::UnsupportedVersion => FailureCode::UnsupportedSchema,
            Self::TooLarge | Self::TooDeep | Self::Malformed(_) => FailureCode::InvalidArgument,
        }
    }
}

impl fmt::Display for JsonInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::TooLarge => "JSON request exceeds the byte limit",
            Self::TooDeep => "JSON request exceeds the nesting limit",
            Self::Malformed(_) => "JSON request does not match its strict schema",
            Self::UnsupportedVersion => "JSON request uses an unsupported schema major",
        };
        formatter.write_str(message)
    }
}

impl Error for JsonInputError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Malformed(error) => Some(error),
            Self::TooLarge | Self::TooDeep | Self::UnsupportedVersion => None,
        }
    }
}

fn validate_nesting(bytes: &[u8]) -> Result<(), JsonInputError> {
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;

    for byte in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }

        match *byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth = depth.checked_add(1).ok_or(JsonInputError::TooDeep)?;
                if depth > MAX_JSON_NESTING {
                    return Err(JsonInputError::TooDeep);
                }
            }
            b'}' | b']' => {
                depth = depth.checked_sub(1).ok_or_else(malformed_nesting_error)?;
            }
            _ => {}
        }
    }

    Ok(())
}

fn malformed_nesting_error() -> JsonInputError {
    JsonInputError::Malformed(serde_json::Error::io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "unbalanced JSON container",
    )))
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use vsift_domain::SessionId;

    use super::{
        JsonInputError, MAX_JSON_INPUT_BYTES, MAX_JSON_NESTING, SchemaVersion, decode_json,
    };

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct TestRequest {
        schema_version: String,
        action: TestAction,
        session_id: String,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum TestAction {
        Inspect,
    }

    #[test]
    fn strict_decoder_accepts_a_valid_request() -> Result<(), Box<dyn std::error::Error>> {
        let request: TestRequest = decode_json(
            br#"{"schema_version":"1","action":"inspect","session_id":"ses_0123456789abcdef"}"#,
        )?;

        assert!(matches!(
            SchemaVersion::parse(&request.schema_version),
            Ok(SchemaVersion::V1)
        ));
        assert!(matches!(request.action, TestAction::Inspect));
        assert!(SessionId::parse(request.session_id).is_ok());
        Ok(())
    }

    #[test]
    fn strict_decoder_rejects_missing_unknown_and_invalid_enum_fields() {
        for bytes in [
            br#"{"schema_version":"1","action":"inspect"}"#.as_slice(),
            br#"{"schema_version":"1","action":"inspect","session_id":"ses_0123456789abcdef","extra":true}"#.as_slice(),
            br#"{"schema_version":"1","action":"execute","session_id":"ses_0123456789abcdef"}"#.as_slice(),
        ] {
            assert!(decode_json::<TestRequest>(bytes).is_err());
        }
    }

    #[test]
    fn schema_major_and_forged_identifier_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
        assert!(matches!(
            SchemaVersion::parse("2"),
            Err(JsonInputError::UnsupportedVersion)
        ));
        let request: TestRequest = decode_json(
            br#"{"schema_version":"1","action":"inspect","session_id":"../../outside"}"#,
        )?;
        assert!(SessionId::parse(request.session_id).is_err());
        Ok(())
    }

    #[test]
    fn byte_and_nesting_budgets_fail_before_deserialization() {
        let oversized = vec![b' '; MAX_JSON_INPUT_BYTES + 1];
        assert!(matches!(
            decode_json::<TestRequest>(&oversized),
            Err(JsonInputError::TooLarge)
        ));

        let too_deep = format!(
            "{}0{}",
            "[".repeat(MAX_JSON_NESTING + 1),
            "]".repeat(MAX_JSON_NESTING + 1)
        );
        assert!(matches!(
            decode_json::<serde_json::Value>(too_deep.as_bytes()),
            Err(JsonInputError::TooDeep)
        ));
    }

    #[test]
    fn braces_inside_strings_do_not_consume_nesting_budget()
    -> Result<(), Box<dyn std::error::Error>> {
        let value: serde_json::Value = decode_json(br#"{"value":"[[[{{{\\\""}"#)?;
        assert!(value.is_object());
        Ok(())
    }
}
